//! Free-roam collectables and Speed Points (docs/decomp.md "Free-roam rules", "Trigger hits", "Speed Points").
//!
//! Speed cameras, security gates and billboards are `TriggerData` boxes of kinds '4', '0' and '2'
//! (`FUN_006e4920`). What the game does, and what is ported here:
//!
//! - **Overlap** (`FUN_00536a10`, `FUN_00536f70`): the car's body box against the trigger's box (full extents =
//!   the record's size, `FUN_00539e50`) or sphere. A box with flags bit 0 only counts while the car's forward
//!   axis points along the box's local Z (`dot >= 0`).
//! - **Firing** (`FUN_00707650`): on entry only (a trigger in last frame's list is skipped); triggers already
//!   found are dropped, except speed cameras; the rest go into a queue that hands one hit to the handler every
//!   4 s (`DAT_00fcb050`): the timer rests at 4 s while the queue is empty, so a lone hit is handled at once.
//! - **Handler** (`FUN_007059b0`): a speed camera reads the car's speed when its hit is handled and counts at
//!   or above `CollectablesConfig.d89eeda4` (50 mph), recorded as `(int)(100 × mph)` (`FUN_006ed0a0`); a
//!   security gate and a billboard count once. A billboard is also handled, without the queue, when its prop
//!   is broken by the player's car (game-logic message 0x14 from `FUN_005b5340`; the compound's instance id is
//!   the trigger's id).
//! - **Speed Points**: the handler sends scoring events naming `ScoringConfig` actions; `FUN_006d08a0` adds the
//!   action's Speed Points to the total (`mw_assets::scoring`): `SpeedCamDiscovered` and `BillboardDiscovered`
//!   500, `SmashGateDiscovered` 250, each on the first find only. The gate's events need the free-roam flag
//!   (game mode +0x2e90): none during an event.
//! - **HUD**: an action with a `Sequence` and a `Title` shows the title ("Billboard +500") with that sequence;
//!   an action's `GameplayTrigger` plays its output sequences (the speed camera's plate with the speed, the
//!   gate's speedwall panel with the found count). `hud.rs` draws them from [`FreeroamHud`].
//!
//! Not from the code (model decisions, marked below): the overlap's narrow phase, the car box's centre, what
//! "the prop breaks" is in our prop model, the collectable totals, when a `GameplayTrigger` fires.
//!
//! The security gate gives **no nitrous**: the +10 of `FUN_0074cf40` belongs to the Smash-and-Grab game mode's
//! scoring behaviour (all four callers are its methods), not to free-roam gates (docs/decomp.md).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use mw_assets::scoring::{Scoring, ScoringAction, action};
use mw_formats::{Collectable, Trigger, TriggerShape};

use crate::Game;
use crate::drive::{Car, Player};
use crate::props::{BillboardFaces, Compound, CompoundPiece, PropBody, PropKnocked};

/// Seconds the Speed Points total shows after it changed, and at the start (the HSM's "SP Visibility" states
/// "Show SP Points", `TIME` = 8: `mwtool hsm 1695370`).
const SP_SHOW_SECONDS: f64 = 8.0;

/// Seconds between two queued hits (`DAT_00fcb050`).
const HIT_INTERVAL: f32 = 4.0;
/// mph per m/s.
const MPH: f32 = 2.236_936;

pub struct FreeroamPlugin {
    /// The profile file the progress is kept in (`None`: not saved).
    pub profile: Option<PathBuf>,
    /// `--new-game`: start without the saved progress.
    pub new_game: bool,
    /// `--test freeroam`.
    pub test: Option<FreeroamTest>,
}

impl Plugin for FreeroamPlugin {
    fn build(&self, app: &mut App) {
        let mut state = Collectables { profile: self.profile.clone(), ..default() };
        if let (Some(path), false) = (&self.profile, self.new_game) {
            state.progress = Progress::load(path);
        }
        app.insert_resource(state).add_message::<ScoreAction>().add_systems(
            Update,
            (load, overlaps, billboards, handle_hits, score_actions, publish, save, billboard_faces).chain(),
        );
        if let Some(test) = &self.test {
            app.insert_resource(test.clone()).add_systems(Update, run_test.after(save));
        }
    }
}

/// What is kept in the profile.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Progress {
    pub speed_points: u64,
    /// Best speed per speed camera, hundredths of a mph (as the game records it).
    pub cameras: BTreeMap<u32, i32>,
    pub billboards: BTreeSet<u32>,
    pub gates: BTreeSet<u32>,
}

/// The profile keys this module owns (`key=value` lines; other lines are kept as they are).
const KEYS: [&str; 4] = ["speed_points", "cameras", "billboards", "gates"];

impl Progress {
    pub fn parse(text: &str) -> Self {
        let mut p = Self::default();
        let ids = |v: &str| v.split(',').filter_map(|x| x.trim().parse::<u32>().ok()).collect();
        for line in text.lines() {
            match line.split_once('=') {
                Some(("speed_points", v)) => p.speed_points = v.trim().parse().unwrap_or(0),
                Some(("cameras", v)) => {
                    p.cameras = v
                        .split(',')
                        .filter_map(|x| {
                            let (id, best) = x.trim().split_once(':')?;
                            Some((id.parse().ok()?, best.parse().ok()?))
                        })
                        .collect()
                }
                Some(("billboards", v)) => p.billboards = ids(v),
                Some(("gates", v)) => p.gates = ids(v),
                _ => {}
            }
        }
        p
    }

    pub fn load(path: &Path) -> Self {
        crate::platform::read_text(path).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    fn lines(&self) -> String {
        let join = |ids: &BTreeSet<u32>| ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let cameras = self.cameras.iter().map(|(id, best)| format!("{id}:{best}")).collect::<Vec<_>>().join(",");
        format!(
            "speed_points={}\ncameras={cameras}\nbillboards={}\ngates={}\n",
            self.speed_points,
            join(&self.billboards),
            join(&self.gates)
        )
    }

    /// Write our lines into the profile, keeping every other line (the car and its position, other modules').
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut text = String::new();
        if let Some(old) = crate::platform::read_text(path) {
            for line in old.lines() {
                if !line.split_once('=').is_some_and(|(k, _)| KEYS.contains(&k)) {
                    text.push_str(line);
                    text.push('\n');
                }
            }
        }
        text.push_str(&self.lines());
        crate::platform::write_text(path, &text)
    }
}

/// A handled hit, for logs and the test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub kind: Collectable,
    pub id: u32,
    /// Speed cameras: the recorded speed (hundredths of a mph), 0 below the minimum (not counted).
    pub centi_mph: i32,
    pub first: bool,
    /// Speed Points awarded.
    pub sp: u32,
    /// Handled because the billboard's prop broke (message 0x14), not through the overlap queue.
    pub via_prop: bool,
}

#[derive(Resource, Default)]
pub struct Collectables {
    loaded: bool,
    /// The collectable triggers that are overlap-tested: speed cameras, security gates, billboards.
    triggers: Vec<Trigger>,
    scoring: Scoring,
    /// Language strings of the actions' titles.
    titles: BTreeMap<u32, String>,
    min_mph: f32,
    /// Triggers the car overlapped on the last frame (the game's per-vehicle hit list).
    inside: BTreeSet<u32>,
    queue: VecDeque<u32>,
    timer: f32,
    /// Billboards broken by the player's car since the last frame.
    smashed: Vec<u32>,
    /// Billboards whose prop is broken (until it stands again): one break is one message.
    broken: BTreeSet<u32>,
    pub progress: Progress,
    /// Hits handled, oldest first.
    pub hits: Vec<Hit>,
    profile: Option<PathBuf>,
    dirty: bool,
    since_save: f32,
}

impl Collectables {
    /// A new game (`FUN_0072bf20` through the logic's `NewGame`, `logic_profile.rs`): the finds and the Speed
    /// Points bucket emptied, saved with the next save.
    pub fn reset_progress(&mut self) {
        self.progress = Progress::default();
        self.dirty = true;
    }

    pub fn total(&self, kind: Collectable) -> usize {
        self.triggers.iter().filter(|t| t.collectable() == Some(kind)).count()
    }

    pub fn found(&self, kind: Collectable) -> usize {
        match kind {
            Collectable::SpeedCamera => self.progress.cameras.len(),
            Collectable::Billboard => self.progress.billboards.len(),
            Collectable::SecurityGate => self.progress.gates.len(),
            _ => 0,
        }
    }

    fn is_found(&self, kind: Collectable, id: u32) -> bool {
        match kind {
            Collectable::SpeedCamera => self.progress.cameras.contains_key(&id),
            Collectable::Billboard => self.progress.billboards.contains(&id),
            Collectable::SecurityGate => self.progress.gates.contains(&id),
            _ => false,
        }
    }

    pub fn ready(&self) -> bool {
        self.loaded && !self.triggers.is_empty()
    }

    /// The nearest trigger of a kind (the test picks its targets with it).
    pub fn nearest(&self, kind: Collectable, p: Vec3) -> Option<Trigger> {
        self.triggers
            .iter()
            .filter(|t| t.collectable() == Some(kind))
            .min_by(|a, b| {
                Vec3::from_array(a.position).distance(p).total_cmp(&Vec3::from_array(b.position).distance(p))
            })
            .copied()
    }
}

/// A sequence the HUD plays (`hud.rs`).
#[derive(Debug, Clone)]
pub struct PlayingSequence {
    pub sequence: u32,
    pub clock: f32,
    /// `UISequences.Params[#].Text1` when the scoring action supplies it (its title), else the item's own text.
    pub text1: Option<String>,
}

/// What the HUD's bindings read (`hud::HudState::freeroam`).
#[derive(Default)]
pub struct FreeroamHud {
    /// `ScoringViewModel.CurrentSpeedPoints`.
    pub speed_points: u64,
    /// Game time (s) the total last changed (the label's backing is triggered by it, `hud.rs`).
    pub sp_changed: Option<f64>,
    /// `SPVisible.IsVisible`: the total shows until this game time. The game-logic state machine "SP
    /// Visibility" (HSM node 1695370) shows it for 8 s at the start and for 8 s whenever
    /// `ScoringViewModel.CurrentSpeedPoints` changes (and through a pursuit, which we do not have).
    pub sp_visible_until: Option<f64>,
    /// `Collectables.LastCameraSpeed` (mph).
    pub last_camera_mph: f32,
    /// `Collectables.SpeedwallColFound` / `SpeedwallColAmt`: security gates found and their number (the only
    /// speedwall panel shown, text 1470065 "SECURITY GATES {found} / {amount}").
    pub wall_found: u32,
    pub wall_total: u32,
    /// `Collectables.SpeedCamsFoundFromSpeedwall` / `SpeedCamAmountFromSpeedwall`.
    pub cameras_found: u32,
    pub cameras_total: u32,
    pub playing: Vec<PlayingSequence>,
}

/// The sequences the collectables can start, for the HUD to load: the scoring actions' own (`Sequence`) and
/// their `GameplayTrigger`s' outputs.
pub fn hud_sequences(scoring: &Scoring) -> Vec<u32> {
    let mut out = Vec::new();
    for field in ACTIONS {
        let Some(a) = scoring.by_field(field) else { continue };
        let own = (a.sequence != 0 && a.title != 0).then_some(a.sequence);
        let triggered = scoring.trigger_sequences.get(&a.gameplay_trigger).into_iter().flatten().copied();
        for s in own.into_iter().chain(triggered) {
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// The `ScoringConfig` actions the handler sends for the three kinds.
const ACTIONS: [u32; 7] = [
    action::SPEED_CAMERA_TRIGGERED,
    action::SPEED_CAM_DISCOVERED,
    action::SECURITY_GATE_HIT,
    action::SMASH_GATE_DISCOVERED,
    action::HIT_JUMP,
    action::BILLBOARD_FOUND,
    action::BILLBOARD_DISCOVERED,
];

fn load(game: Option<Res<Game>>, mut state: ResMut<Collectables>, mut hud: ResMut<crate::hud::HudState>) {
    let Some(game) = game else { return };
    if state.loaded {
        return;
    }
    state.loaded = true;
    match game.store.trigger_data() {
        Ok(data) => {
            state.triggers = data
                .triggers
                .iter()
                .filter(|t| {
                    matches!(
                        t.collectable(),
                        Some(Collectable::SpeedCamera | Collectable::SecurityGate | Collectable::Billboard)
                    )
                })
                .copied()
                .collect()
        }
        Err(e) => warn!("collectables: TriggerData not loaded: {e}"),
    }
    match (game.store.scoring(), game.store.collectables_config()) {
        (Ok(scoring), Ok(cfg)) => {
            state.scoring = scoring;
            state.min_mph = cfg.speed_camera_min_mph;
        }
        (s, c) => {
            warn!("collectables: scoring data not loaded ({:?} / {:?}): nothing is counted", s.err(), c.err());
            state.triggers.clear();
        }
    }
    if let Ok(lang) = game.store.language(mw_assets::DEFAULT_LANGUAGE) {
        let titles: Vec<u32> = ACTIONS.iter().filter_map(|&f| state.scoring.by_field(f)).map(|a| a.title).collect();
        for id in titles {
            if let Some(text) = lang.get(id) {
                state.titles.insert(id, text.to_owned());
            }
        }
    }
    hud.freeroam.speed_points = state.progress.speed_points;
    info!(
        "collectables: {} speed cameras (min {} mph, {} found), {} gates ({} found), {} billboards ({} found), {} SP",
        state.total(Collectable::SpeedCamera),
        state.min_mph,
        state.found(Collectable::SpeedCamera),
        state.total(Collectable::SecurityGate),
        state.found(Collectable::SecurityGate),
        state.total(Collectable::Billboard),
        state.found(Collectable::Billboard),
        state.progress.speed_points
    );
}

/// An oriented box: centre, axes (columns of the rotation) and half extents.
#[derive(Clone, Copy)]
struct Obb {
    centre: Vec3,
    axes: [Vec3; 3],
    half: Vec3,
}

impl Obb {
    fn new(centre: Vec3, rotation: Quat, half: Vec3) -> Self {
        Self { centre, axes: [rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z], half }
    }

    /// Separating-axis test of two boxes.
    fn overlaps(&self, other: &Obb) -> bool {
        let d = other.centre - self.centre;
        let radius = |b: &Obb, axis: Vec3| (0..3).map(|i| b.half[i] * b.axes[i].dot(axis).abs()).sum::<f32>();
        let separated = |axis: Vec3| {
            let len = axis.length();
            len > 1e-5 && d.dot(axis).abs() > radius(self, axis) + radius(other, axis)
        };
        for i in 0..3 {
            if separated(self.axes[i]) || separated(other.axes[i]) {
                return false;
            }
        }
        for a in self.axes {
            for b in other.axes {
                if separated(a.cross(b)) {
                    return false;
                }
            }
        }
        true
    }

    fn distance_to(&self, p: Vec3) -> f32 {
        let d = p - self.centre;
        let mut closest = self.centre;
        for i in 0..3 {
            closest += self.axes[i] * d.dot(self.axes[i]).clamp(-self.half[i], self.half[i]);
        }
        closest.distance(p)
    }
}

/// Whether the car's box is in the trigger's volume (points never).
/// Model decision: the game's overlap job (`FUN_00536a10`) queries with the body's half extents at the vehicle's
/// pose; its narrow phase was not read. Here: box against box by separating axes, the car's box centred on its
/// origin (the body box's own centre offset was not traced).
fn car_in_trigger(car: &Obb, t: &Trigger) -> bool {
    let centre = Vec3::from_array(t.position);
    match t.shape {
        TriggerShape::Box { rotation: [x, y, z, w], size } => {
            // Half extents = size × 0.5 (`FUN_00a102b0`).
            car.overlaps(&Obb::new(centre, Quat::from_xyzw(x, y, z, w).normalize(), Vec3::from_array(size) * 0.5))
        }
        TriggerShape::Sphere { radius } => car.distance_to(centre) <= radius,
        TriggerShape::Point { .. } => false,
    }
}

/// `--hsm`: the world's triggers enabled by the logic (`ENABLETRIGS` "EnableTriggers" in "Show World States":
/// `FUN_00846590`(1) on enter, (0) on exit, logic.rs). Model decision: the message's receiver (queue
/// `DAT_010edb48`) was not traced; it is read as switching these trigger hits. Absent (no `--hsm`): always on.
#[derive(Resource, Default)]
pub struct TriggersEnabled(pub bool);

/// The per-frame hit list and the firing rule (`FUN_00536f70`, `FUN_00707650`).
fn overlaps(
    mut state: ResMut<Collectables>,
    cars: Query<(&Car, &Transform), With<Player>>,
    enabled: Option<Res<TriggersEnabled>>,
) {
    let on = enabled.is_none_or(|e| e.0);
    let (true, Ok((car, xf))) = (on, cars.single()) else {
        state.inside.clear();
        return;
    };
    let state = &mut *state;
    let body = Obb::new(xf.translation, xf.rotation, car.params.body_half_extents);
    // Vehicle +0x100: the matrix's third row, the car's forward axis (local +Z).
    let forward = xf.rotation * Vec3::Z;
    let reach = car.params.body_half_extents.length();
    let mut inside = BTreeSet::new();
    for t in &state.triggers {
        let centre = Vec3::from_array(t.position);
        let extent = match t.shape {
            TriggerShape::Box { size, .. } => Vec3::from_array(size).length() * 0.5,
            TriggerShape::Sphere { radius } => radius,
            TriggerShape::Point { .. } => continue,
        };
        if centre.distance_squared(xf.translation) > (extent + reach).powi(2) || !car_in_trigger(&body, t) {
            continue;
        }
        // One-way boxes (flags bit 0): only while the car faces along the box's local Z.
        if let (true, TriggerShape::Box { rotation: [x, y, z, w], .. }) = (t.flags & 1 != 0, t.shape)
            && forward.dot(Quat::from_xyzw(x, y, z, w).normalize() * Vec3::Z) < 0.0
        {
            continue;
        }
        inside.insert(t.id);
    }
    for &id in &inside {
        if state.inside.contains(&id) {
            continue;
        }
        let Some(kind) = state.triggers.iter().find(|t| t.id == id).and_then(|t| t.collectable()) else { continue };
        // Found triggers are dropped, speed cameras are repeatable (`FUN_006fa530`).
        if kind != Collectable::SpeedCamera && state.is_found(kind, id) {
            continue;
        }
        state.queue.push_back(id);
    }
    state.inside = inside;
}

/// Billboards: the prop broken by the player's car (message 0x14). The game's object carries its trigger's id
/// (instance +0x4c = the trigger id on all 216 billboards, at the trigger's position: `mwtool freeroam
/// --objects`); here the compound is matched by that position.
/// Model decision: "the prop breaks" is our prop coming loose (`props.rs`: the hit passed its break threshold);
/// the game's callback fires on a state of its smashable object that was not traced further.
fn billboards(
    mut state: ResMut<Collectables>,
    mut knocked: MessageReader<PropKnocked>,
    props: Query<(&PropBody, Has<CompoundPiece>)>,
) {
    let state = &mut *state;
    let board_at = |triggers: &[Trigger], p: Vec3| {
        triggers
            .iter()
            .find(|t| t.collectable() == Some(Collectable::Billboard) && Vec3::from_array(t.position).distance(p) < 0.5)
            .map(|t| t.id)
    };
    // A billboard standing again (put back by `props.rs`) can be broken again.
    if !state.broken.is_empty() {
        for (prop, piece) in &props {
            if !piece
                && prop.pinned
                && let Some(id) = board_at(&state.triggers, prop.home.translation)
            {
                state.broken.remove(&id);
            }
        }
    }
    for k in knocked.read() {
        // The compound itself carries the trigger's id, not its parts.
        let Ok((prop, false)) = props.get(k.prop) else { continue };
        let Some(id) = board_at(&state.triggers, prop.home.translation) else { continue };
        if state.broken.insert(id) && k.by_player {
            state.smashed.push(id);
        }
    }
}

/// Which face a billboard compound stands with (`props::BillboardFaces`): the advert until its trigger is in
/// the found list, then the "WANTED" poster with its picture (`loader::Loader::compound_standing`: the game
/// picks the model when it creates the object at stream-in, from the save's found list). Offline the picture
/// is the instance's own texture (a silhouette): the game's `BillboardImages` downloads are online content.
/// Model decision: the game does not stream our compounds out and in again, so the face is picked when the
/// found list is first known and again while the compound is broken (it is hidden then: the new face shows
/// when it is put back), not while it stands.
fn billboard_faces(
    state: Res<Collectables>,
    mut faces: Query<(&PropBody, &mut BillboardFaces, Option<&Compound>)>,
    mut vis: Query<&mut Visibility, Without<PropBody>>,
) {
    if !state.ready() {
        return;
    }
    for (prop, mut f, compound) in &mut faces {
        if f.shown.is_some() && !compound.is_some_and(Compound::is_smashed) {
            continue;
        }
        let at = prop.home.translation;
        let found = state
            .triggers
            .iter()
            .find(|t| {
                t.collectable() == Some(Collectable::Billboard) && Vec3::from_array(t.position).distance(at) < 0.5
            })
            .is_some_and(|t| state.progress.billboards.contains(&t.id));
        if f.shown == Some(found) {
            continue;
        }
        for (e, show) in [(f.advert, !found), (f.found, found)] {
            if let Ok(mut v) = vis.get_mut(e) {
                *v = if show { Visibility::Inherited } else { Visibility::Hidden };
            }
        }
        f.shown = Some(found);
    }
}

/// The queue's timer and the hit handler (`FUN_00707650`'s tail, `FUN_007059b0`).
fn handle_hits(
    mut state: ResMut<Collectables>,
    mut hud: ResMut<crate::hud::HudState>,
    time: Res<Time>,
    cars: Query<&Car, With<Player>>,
    race: Option<Res<crate::race::RaceState>>,
) {
    let state = &mut *state;
    let mph = cars.single().ok().map(|c| c.speed.abs() * MPH);
    // The gate's scoring events need the free-roam flag (game mode +0x2e90).
    let free_roam = race.is_none();
    let now = time.elapsed_secs_f64();
    // Billboards broken by the car: handled at once (message 0x14 calls the handler directly).
    for id in std::mem::take(&mut state.smashed) {
        handle(state, &mut hud.freeroam, Collectable::Billboard, id, mph, free_roam, now, true);
    }
    if state.queue.is_empty() {
        state.timer = HIT_INTERVAL;
        return;
    }
    state.timer += time.delta_secs();
    if state.timer > HIT_INTERVAL {
        state.timer = 0.0;
        if let Some(id) = state.queue.pop_front()
            && let Some(kind) = state.triggers.iter().find(|t| t.id == id).and_then(|t| t.collectable())
        {
            handle(state, &mut hud.freeroam, kind, id, mph, free_roam, now, false);
        }
    }
}

/// A scoring event posted by another module (the game's `FUN_006a8050` with a `ScoringConfig` field): the
/// field's name hash, `mw_assets::scoring::action`. Its Speed Points, message and sequences run as for the
/// collectables' own events.
#[derive(Message, Clone, Copy, Debug)]
pub struct ScoreAction(pub u32);

fn score_actions(
    mut state: ResMut<Collectables>,
    mut hud: ResMut<crate::hud::HudState>,
    time: Res<Time>,
    mut actions: MessageReader<ScoreAction>,
) {
    let state = &mut *state;
    for ScoreAction(field) in actions.read().copied() {
        let Some(a) = state.scoring.by_field(field).cloned() else { continue };
        let sp = score(state, &mut hud.freeroam, &a, time.elapsed_secs_f64());
        state.dirty = true;
        info!("scoring action {field:08x}: +{sp} SP (total {})", state.progress.speed_points);
    }
}

/// `FUN_007059b0` for one trigger.
#[allow(clippy::too_many_arguments)]
fn handle(
    state: &mut Collectables,
    hud: &mut FreeroamHud,
    kind: Collectable,
    id: u32,
    mph: Option<f32>,
    free_roam: bool,
    now: f64,
    via_prop: bool,
) {
    let first = !state.is_found(kind, id);
    let mut hit = Hit { kind, id, centi_mph: 0, first, sp: 0, via_prop };
    let mut actions: Vec<u32> = Vec::new();
    match kind {
        Collectable::SpeedCamera => {
            // `FUN_006ed0a0`: |speed| in mph against the minimum; below it nothing happens.
            let Some(mph) = mph.filter(|&m| m >= state.min_mph) else {
                info!("speed camera {id}: {:.1} mph, under {} mph", mph.unwrap_or(0.0), state.min_mph);
                hit.first = false;
                state.hits.push(hit);
                return;
            };
            hit.centi_mph = (100.0 * mph) as i32;
            actions.push(action::SPEED_CAMERA_TRIGGERED);
            if first {
                actions.push(action::SPEED_CAM_DISCOVERED);
            }
            // Offline the record is the local best (`FUN_008749e0` → `FUN_00874830`).
            let best = state.progress.cameras.entry(id).or_insert(0);
            *best = (*best).max(hit.centi_mph);
            hud.last_camera_mph = hit.centi_mph as f32 / 100.0;
        }
        Collectable::SecurityGate => {
            if first && free_roam {
                actions.extend([action::SECURITY_GATE_HIT, action::SMASH_GATE_DISCOVERED]);
            }
            state.progress.gates.insert(id);
        }
        Collectable::Billboard => {
            // `FUN_006ed330`: every hit offline; then the first find's pair.
            actions.push(action::HIT_JUMP);
            if first {
                actions.extend([action::BILLBOARD_FOUND, action::BILLBOARD_DISCOVERED]);
            }
            state.progress.billboards.insert(id);
        }
        _ => return,
    }
    state.dirty = true;
    hud.wall_found = state.found(Collectable::SecurityGate) as u32;
    hud.wall_total = state.total(Collectable::SecurityGate) as u32;
    hud.cameras_found = state.found(Collectable::SpeedCamera) as u32;
    hud.cameras_total = state.total(Collectable::SpeedCamera) as u32;
    for field in actions {
        let Some(a) = state.scoring.by_field(field).cloned() else { continue };
        hit.sp += score(state, hud, &a, now);
    }
    info!(
        "collectable {} {id}: {}{}+{} SP (total {}), found {}/{}",
        kind.name(),
        if kind == Collectable::SpeedCamera {
            format!("{:.2} mph, ", hit.centi_mph as f32 / 100.0)
        } else {
            String::new()
        },
        if first { "first find, " } else { "" },
        hit.sp,
        state.progress.speed_points,
        state.found(kind),
        state.total(kind)
    );
    state.hits.push(hit);
}

/// One scoring event offline (`FUN_006d08a0`): the action's Speed Points go to the total when above 0
/// (`FUN_00735ca0` → `FUN_00734400`), its message and its trigger's sequences start.
/// Model decision: an action's `GameplayTrigger` is taken to fire its output sequences whenever the action
/// scores (the trigger's inputs and conditions were not traced).
fn score(state: &mut Collectables, hud: &mut FreeroamHud, a: &ScoringAction, now: f64) -> u32 {
    let sp = a.sp();
    if sp > 0 {
        state.progress.speed_points += sp as u64;
        hud.speed_points = state.progress.speed_points;
        hud.sp_changed = Some(now);
        hud.sp_visible_until = Some(now + SP_SHOW_SECONDS);
    }
    if a.sequence != 0
        && let Some(title) = state.titles.get(&a.title)
    {
        // The title's `{sp}` is the award ("Billboard +500").
        let text = mw_formats::ui::substitute(title, |expr| (expr == "sp").then(|| sp.to_string()));
        hud.playing.push(PlayingSequence { sequence: a.sequence, clock: 0.0, text1: Some(text) });
    }
    for &s in state.scoring.trigger_sequences.get(&a.gameplay_trigger).into_iter().flatten() {
        hud.playing.retain(|p| p.sequence != s);
        hud.playing.push(PlayingSequence { sequence: s, clock: 0.0, text1: None });
    }
    sp
}

/// Advance the HUD sequences' clocks; the HUD draws each item in its time range. They are dropped after 30 s
/// (longer than any of them: the speed camera's runs 6.5 s).
fn publish(mut hud: ResMut<crate::hud::HudState>, time: Res<Time>, cars: Query<(), (With<Car>, With<Player>)>) {
    // The first "Show SP Points" state: 8 s from the start of play.
    if hud.freeroam.sp_visible_until.is_none() && !cars.is_empty() {
        hud.freeroam.sp_visible_until = Some(time.elapsed_secs_f64() + SP_SHOW_SECONDS);
        // The profile's total arriving counts as a change of `MostWantedListManager.LocalPlayerSpeedPoints`: the
        // counter's strip (sub-image 1470275, triggered by it) shows with the label, as in the original's
        // `ref/ui-2026-10-19/hud_1680x720.png`. Model decision (the trigger's evaluation was not traced).
        hud.freeroam.sp_changed = Some(time.elapsed_secs_f64());
    }
    if hud.freeroam.playing.is_empty() {
        return;
    }
    let dt = time.delta_secs();
    for p in &mut hud.freeroam.playing {
        p.clock += dt;
    }
    hud.freeroam.playing.retain(|p| p.clock < 30.0);
}

/// Write the progress a moment after it changed.
fn save(mut state: ResMut<Collectables>, time: Res<Time<Real>>) {
    if !state.dirty {
        return;
    }
    state.since_save += time.delta_secs();
    if state.since_save < 1.0 {
        return;
    }
    state.dirty = false;
    state.since_save = 0.0;
    if let Some(path) = &state.profile
        && let Err(e) = state.progress.save(path)
    {
        warn!("could not save {}: {e}", path.display());
    }
}

/// `--test freeroam`: pass the nearest speed camera too slowly, then fast, then again; drive through the nearest
/// security gate and fly through the nearest billboard; check the profile. Prints `TEST RESULT freeroam ...`.
#[derive(Resource, Clone, Default)]
pub struct FreeroamTest {
    pub screenshot: Option<PathBuf>,
    step: usize,
    timer: f32,
    started: bool,
    hits_before: usize,
    sp_before: u64,
    /// When the pass's first hit was seen (the result is printed a moment later: a billboard is handled by
    /// its volume and by its prop).
    hit_at: Option<f32>,
    shot_at: Option<f32>,
    /// Where the car started: the targets are the collectables nearest to it.
    anchor: Option<Vec3>,
}

impl FreeroamTest {
    pub fn new(screenshot: Option<PathBuf>) -> Self {
        Self { screenshot, ..default() }
    }
}

/// Where `--test freeroam` starts when `--spawn` is not given: a speed camera (1025449), a security gate
/// (1274961) and a billboard (1496888) within 25 m of each other (`mwtool freeroam --list`).
pub const TEST_SPAWN: [f32; 3] = [2227.0, 56.0, 4240.0];

/// The test's passes: what, the speed (m/s) and the start distance before the trigger along its local Z.
const TEST_STEPS: [(&str, Collectable, f32, f32); 5] = [
    ("camera-slow", Collectable::SpeedCamera, 15.0, 20.0),
    ("camera", Collectable::SpeedCamera, 40.0, 30.0),
    ("camera-again", Collectable::SpeedCamera, 45.0, 30.0),
    // The billboard hangs 12 m behind the gate: it goes first, from between the two.
    ("billboard", Collectable::Billboard, 35.0, 8.0),
    ("gate", Collectable::SecurityGate, 25.0, 15.0),
];

#[allow(clippy::too_many_arguments)]
fn run_test(
    mut commands: Commands,
    mut test: ResMut<FreeroamTest>,
    mut state: ResMut<Collectables>,
    hud: Res<crate::hud::HudState>,
    time: Res<Time>,
    spatial: avian3d::prelude::SpatialQuery,
    props: Query<&PropBody>,
    mut input: ResMut<crate::drive::CarInput>,
    mut player: Query<
        (&mut Transform, &mut avian3d::prelude::LinearVelocity, &mut avian3d::prelude::AngularVelocity, &Car),
        With<Player>,
    >,
    mut exit: MessageWriter<AppExit>,
) {
    use avian3d::prelude::*;
    let Ok((mut xf, mut v, mut w, car)) = player.single_mut() else { return };
    if !state.ready() {
        return;
    }
    test.timer += time.delta_secs();
    if let Some(at) = test.shot_at
        && test.timer >= at
    {
        test.shot_at = None;
        if let Some(out) = &test.screenshot {
            let name = TEST_STEPS.get(test.step).map_or("end", |s| s.0);
            let stem = out.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let path = out.with_file_name(format!("{stem}-{name}.png"));
            info!("TEST freeroam: screenshot {}", path.display());
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(path));
        }
    }
    let ground = |p: Vec3| {
        let filter = SpatialQueryFilter::from_mask(crate::props::GameLayer::World);
        let from = p + Vec3::Y * 6.0;
        spatial
            .ray_hits(from, Dir3::NEG_Y, 30.0, 16, false, &filter)
            .into_iter()
            .map(|h| from.y - h.distance)
            .min_by(|a, b| (a - p.y).abs().total_cmp(&(b - p.y).abs()))
    };
    let Some(&(name, kind, speed, before)) = TEST_STEPS.get(test.step) else {
        // The profile: what was written reads back as the state.
        input.scripted = Some([0.0, 1.0, 0.0, 0.0]);
        if test.started {
            if test.timer > 1.5 {
                exit.write(AppExit::Success);
            }
            return;
        }
        test.started = true;
        test.timer = 0.0;
        let p = &state.progress;
        match &state.profile {
            Some(path) => {
                let saved = p.save(path).is_ok();
                let back = Progress::load(path);
                println!(
                    "TEST RESULT freeroam profile saved={saved} reloaded_equal={} speed_points={} cameras={} gates={} billboards={}",
                    back == *p,
                    back.speed_points,
                    back.cameras.len(),
                    back.gates.len(),
                    back.billboards.len()
                );
            }
            None => println!("TEST RESULT freeroam profile none (pass --profile <file>)"),
        }
        println!(
            "TEST RESULT freeroam total speed_points={} hud_speed_points={} cameras={}/{} gates={}/{} billboards={}/{}",
            p.speed_points,
            hud.freeroam.speed_points,
            state.found(Collectable::SpeedCamera),
            state.total(Collectable::SpeedCamera),
            state.found(Collectable::SecurityGate),
            state.total(Collectable::SecurityGate),
            state.found(Collectable::Billboard),
            state.total(Collectable::Billboard)
        );
        return;
    };
    let anchor = *test.anchor.get_or_insert(xf.translation);
    let Some(target) = state.nearest(kind, anchor) else {
        println!("TEST RESULT freeroam {name} no trigger");
        exit.write(AppExit::Success);
        return;
    };
    let centre = Vec3::from_array(target.position);
    let TriggerShape::Box { rotation: [qx, qy, qz, qw], .. } = target.shape else { return };
    let along = (Quat::from_xyzw(qx, qy, qz, qw).normalize() * Vec3::Z).with_y(0.0).normalize_or(Vec3::Z);
    if !test.started {
        input.scripted = Some([0.0, 0.0, 0.0, 1.0]);
        // The world under the start and, for the billboard, its prop (with its collider: props near a car
        // carry one) must be there. The car waits at the start, so the props around it stream in.
        let start = centre - along * before;
        let on_ground = ground(start);
        let prop_there = kind != Collectable::Billboard
            || props.iter().any(|p| p.active && p.home.translation.distance(centre) < 0.5);
        let ready = on_ground.is_some() && prop_there;
        if let Some(g) = on_ground {
            xf.translation = Vec3::new(start.x, g + 0.6, start.z);
            xf.rotation = Quat::from_rotation_y(along.x.atan2(along.z));
            v.0 = Vec3::ZERO;
            w.0 = Vec3::ZERO;
        }
        // The queue must be idle so that the pass is handled when it happens.
        if !ready || !state.queue.is_empty() || test.timer < 2.0 {
            if test.timer > 120.0 {
                println!(
                    "TEST RESULT freeroam {name} id={} not ready (ground {on_ground:?}, prop {prop_there})",
                    target.id
                );
                exit.write(AppExit::Success);
            }
            return;
        }
        // Billboards hang over a drop: fly through at the board's height. The others: on the ground.
        let y = match (kind, on_ground) {
            (Collectable::Billboard, Some(g)) => centre.y.max(g + 0.6),
            (_, Some(g)) => g + 0.6,
            _ => centre.y,
        };
        xf.translation = Vec3::new(start.x, y, start.z);
        xf.rotation = Quat::from_rotation_y(along.x.atan2(along.z));
        v.0 = along * speed;
        w.0 = Vec3::ZERO;
        state.inside.clear();
        test.started = true;
        test.timer = 0.0;
        test.hits_before = state.hits.len();
        test.hit_at = None;
        test.sp_before = state.progress.speed_points;
        info!(
            "TEST freeroam {name}: {} {} from {:?} at {speed} m/s along {along:?}",
            kind.name(),
            target.id,
            xf.translation
        );
        return;
    }
    // Hold the speed through the pass.
    input.scripted = Some([if car.speed < speed { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0]);
    let hits: Vec<Hit> = state.hits[test.hits_before..].iter().filter(|h| h.id == target.id).copied().collect();
    let hit = hits.first().copied();
    if hit.is_some() && test.hit_at.is_none() {
        test.hit_at = Some(test.timer);
    }
    if test.hit_at.map_or(test.timer < 4.0, |at| test.timer < at + 0.4) {
        return;
    }
    let via: Vec<&str> = hits.iter().map(|h| if h.via_prop { "prop" } else { "volume" }).collect();
    let gained = state.progress.speed_points - test.sp_before;
    match hit {
        Some(h) => println!(
            "TEST RESULT freeroam {name} id={} counted={} speed_mph={:.2} best_mph={:.2} first={} sp={} gained={gained} total={} found={}/{} after={:.2}s handled_by={} messages={}",
            h.id,
            h.kind != Collectable::SpeedCamera || h.centi_mph > 0,
            h.centi_mph as f32 / 100.0,
            state.progress.cameras.get(&h.id).copied().unwrap_or(0) as f32 / 100.0,
            h.first,
            h.sp,
            state.progress.speed_points,
            state.found(kind),
            state.total(kind),
            test.hit_at.unwrap_or(0.0),
            via.join("+"),
            hud.freeroam.playing.len()
        ),
        None => println!("TEST RESULT freeroam {name} id={} no hit in 4 s (car at {:?})", target.id, xf.translation),
    }
    test.shot_at = Some(0.3);
    test.started = false;
    test.timer = 0.0;
    test.step += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_round_trip() {
        let mut p = Progress { speed_points: 1250, ..default() };
        p.cameras.insert(1025449, 8947);
        p.gates.insert(1274961);
        p.billboards.extend([1496888, 998884]);
        let text = format!("bundle=VEHICLES/VEH_1085007_HI.BNDL\nposition=1.00,2.00,3.00\n{}", p.lines());
        assert_eq!(Progress::parse(&text), p);
        // An old profile (no progress lines) is an empty progress.
        assert_eq!(Progress::parse("bundle=X\nposition=1,2,3\n"), Progress::default());
    }

    #[test]
    fn boxes() {
        let car = Obb::new(Vec3::new(0.0, 0.5, 0.0), Quat::IDENTITY, Vec3::new(0.9, 0.55, 2.2));
        let t = |x: f32, flags: u8| Trigger {
            position: [x, 1.0, 0.0],
            id: 1,
            kind: b'4',
            flags,
            shape: TriggerShape::Box { rotation: [0.0, 0.0, 0.0, 1.0], size: [4.0, 4.0, 2.0] },
        };
        // The car's side reaches into the box before its origin does.
        assert!(car_in_trigger(&car, &t(2.5, 0)));
        assert!(!t(2.5, 0).contains([0.0, 0.5, 0.0]));
        assert!(!car_in_trigger(&car, &t(3.5, 0)));
    }
}
