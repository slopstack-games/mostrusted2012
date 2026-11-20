//! Race cutscenes: the event's intro sequence before the start and its outro after the player's finish, played
//! from the game's own `Sequence` data (`mw_assets::sequence`; docs/decomp.md "Cutscenes and sequences").
//!
//! - **Which sequences** (data): an `OfflineEvent`'s `EventIntro` (in its `IntroMessageGroup`'s feedback group)
//!   plays before the race, its `EventOutro` (in the game mode's feedback group, 604180 for races) after the
//!   finish. The offline game flow has no countdown: the race mode's `CountdownSequence` is 0 and the HSM's
//!   "Play countdown" state sits under the online flow only; the intro ends with the cars teleported onto the
//!   grid already rolling (`TeleportPlayerSequenceItem.Speed_MPH`: 70 for the player, 55 for the opponents).
//! - **Clock**: real seconds (as the crash camera; the slow-motion items scale the simulation, not the
//!   sequence), run with the timeline controllers of the sequence and of its conducted sequences, each on its
//!   own clock (`mw_assets::timeline`, ported from the code; `seq_clock.rs` feeds them the logic's bindings and
//!   the input). A `JumpOnButtonPress` controller is the skip: SKIP_CUTSCENE (Esc / pad Back) while the clock is
//!   between its `TriggerTime` and its destination jumps there (the intro of 462174: 1 s → 19.5 s, just after
//!   the teleports). The race outros wait in their conducted sequence (a `JumpMonitorTimelineController` makes
//!   the outro follow it), which loops while the award widget is up and skips what the event did not earn.
//! - **Items played**: animated cameras (a clip of a two-bone camera skeleton, world space or relative to the
//!   player's car; horizontal field of view `2 atan(18 / FocalLength)`, a 36 mm frame, `FUN_005c6c10`), ICE
//!   camera takes (`mw_formats::camera_take`), the dynamic camera's blend to the
//!   gameplay camera (lerp of transform and FOV by the item's curves), teleports, the AI driving the player
//!   (`ai.rs`'s driver on the player's car, at most `MaxSpeedMph`), animated race cars (the clip's `car` bone),
//!   hidden race cars, hidden HUD, the letterbox, slow motion and the event trigger.
//! - **Race hand-over**: while the intro plays `RaceState::cutscene` keeps the race in `Waiting` (`hold` stays
//!   the AI's); the intro starts once the opponents are placed. A teleport sets `RaceState::rolling_start` (no
//!   freeze); the intro's `EventTriggerSequenceItem` (at 24 s in 462174) starts the race with no countdown
//!   (`RaceState::intro` = 0).
//!
//! Camera conventions (settled 2026-10-09 by captures of the original, docs/decomp.md "Cutscenes and
//! sequences"): the camera bone's key is the camera matrix as it is (rows left, up, forward like the game camera
//! `FUN_00495b70` fills): it looks along +Z with +Y up, roll included, so 462174's intro is upside down from its
//! second shot on, as in the original; the skeleton's bind pose (180° about Y on bone 1) is not applied. Every
//! FOV is horizontal (`camera::projection_fov`). The outro's take samples the finishing car's transform once
//! (`0bd6ff75` flags, `FUN_0049a910`): the camera stays where the car finished and the car drives out of shot, as
//! the original does.
//!
//! Model decisions (no source yet): without the logic (`--no-hsm`) the race starts on the intro's event trigger
//! (with it, go is the logic's `StartGameModeTimer` after the intro, `logic_race.rs`; the trigger only ends the
//! cutscene's hold); `GetTransform` bindings of animated cameras follow the car
//! live; the animated car is placed on the clip's car bone every tick with the clip's velocity (the original
//! drives it with its `controls_*` bones, not read); audio, post-processing and UI layouts other than the
//! letterbox are not played; the controllers' bindings resolve only through the logic (`--hsm`), else they do
//! not hold (the outro then plays through its earned-award branch to the end).
//!
//! `--no-cutscenes` turns them off (the race then keeps the game mode's 10 s frozen intro); `--cutscene outro`
//! plays the outro at once (for screenshots). `MW_CUTSCENE_TRACE=<file>` logs the camera every frame.
//! `MW_CUTSCENE_SHOT=[intro:|outro:]<s>` saves `--screenshot` when the playing sequence's clock reaches `s` and exits (give a
//! large `--screenshot-delay` so the usual capture doesn't fire first); `MW_CUTSCENE_START=<s>` starts the clock
//! there (both for checking shots headless).

use std::io::Write;
use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;
use mw_assets::sequence::{AnimatedCamera, ItemKind, RacerRef, Sequence};
use mw_formats::camera_take::{Bindings, Frame};

use crate::drive::{Car, Player};
use crate::race::{RacePhase, RaceState, Racer};

/// m/s per mph.
const MPH: f32 = 0.447_04;

/// What `main` hands in.
pub struct CutsceneSetup {
    pub intro: Option<Arc<Sequence>>,
    pub outro: Option<Arc<Sequence>>,
    /// `--test race`: skip the intro at its skip point (exercises the skip; the test then drives the race).
    pub auto_skip: bool,
    /// `--cutscene outro`: play the outro as soon as the player's car is placed.
    pub force_outro: bool,
}

pub struct CutscenePlugin {
    pub setup: Option<Arc<CutsceneSetup>>,
}

impl Plugin for CutscenePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::cutscene_spawns::SpawnsPlugin);
        app.init_resource::<CutsceneState>()
            .init_resource::<LetterboxBars>()
            .add_systems(Startup, spawn_letterbox)
            .add_systems(Update, draw_letterbox.after(crate::camera::GameplayCamera).after(crate::crash::run_sequence));
        // The systems run while a setup exists: from `--race` (inserted here) or a race started at runtime
        // ([`run_event_cutscenes`], the map's event card).
        let running = resource_exists::<Setup>;
        app.add_systems(
            Update,
            (start_cutscenes, run_cutscene)
                .chain()
                .run_if(running)
                .after(crate::camera::GameplayCamera)
                .before(draw_letterbox),
        )
        .add_systems(FixedUpdate, apply_cars.run_if(running).before(crate::drive::car_physics));
        let Some(setup) = &self.setup else { return };
        app.insert_resource(Setup(setup.clone())).add_systems(Startup, hold_for_intro);
    }
}

#[derive(Resource)]
struct Setup(Arc<CutsceneSetup>);

/// The intro and outro of a race started at runtime (`event_card.rs`): the cutscene systems run from now on.
pub fn run_event_cutscenes(commands: &mut Commands, intro: Option<Arc<Sequence>>, outro: Option<Arc<Sequence>>) {
    commands.insert_resource(Setup(Arc::new(CutsceneSetup { intro, outro, auto_skip: false, force_outro: false })));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Intro,
    Outro,
}

/// A sequence being played.
struct Playing {
    which: Which,
    seq: Arc<Sequence>,
    /// Sequence time (real seconds × the run's speed-up, `base_speed`): the sequence's own clock.
    t: f32,
    /// The sequence's clocks with their timeline controllers (`seq.clocks`).
    clock: mw_assets::timeline::SequenceClock,
    /// Per item: has it started (one-shot actions fired).
    started: Vec<bool>,
    /// `Time<Virtual>` speed before the sequence (tests run faster than real time).
    base_speed: f32,
    /// The last camera pose a cinematic item set (the dynamic camera blends from it).
    last_view: Option<(Transform, f32)>,
    /// The race has been released (event trigger).
    released: bool,
    /// The player's car is driven by the AI driver (`DriveLocalPlayerWithAISequenceItem`).
    driving: Option<Entity>,
    /// Racers hidden by `HideRacecarSequenceItem`.
    hidden: Vec<Entity>,
    /// Per camera item: its bindings as sampled when it started.
    frozen: std::collections::HashMap<usize, Vec<Frame>>,
}

/// One-shot work for the next fixed tick.
enum Pending {
    Teleport { entity: Entity, position: Vec3, direction: Vec3, speed: f32 },
}

#[derive(Resource, Default)]
pub struct CutsceneState {
    /// Don't start the intro yet (the event card's "TRANSITION" has not been set).
    pub hold_intro: bool,
    /// Under `--hsm` (logic.rs): the intro and the outro start when the logic plays them (`RUNGAMELOGIC` of
    /// `Event.Data.EventIntro` / `EventOutro` → `PlaySequence`), not by themselves.
    pub logic_driven: bool,
    pub logic_intro: bool,
    pub logic_outro: bool,
    playing: Option<Playing>,
    intro_started: bool,
    outro_started: bool,
    pending: Vec<Pending>,
    /// The animated race car's pose this tick: entity, position, rotation, velocity.
    car_pose: Option<(Entity, Vec3, Quat, Vec3)>,
    trace: Option<std::fs::File>,
    trace_opened: bool,
    /// `MW_CUTSCENE_SHOT`: frames since the capture (exit 30 frames later).
    shot_frames: Option<u32>,
}

impl CutsceneState {
    /// A sequence is playing (the menu leaves Esc / Back to SKIP_CUTSCENE meanwhile).
    pub fn playing(&self) -> bool {
        self.playing.is_some()
    }

    /// The playing sequence and its clock (s), for the spawn items (`cutscene_spawns.rs`).
    pub fn clock(&self) -> Option<(Which, &Arc<Sequence>, f32)> {
        self.playing.as_ref().map(|p| (p.which, &p.seq, p.t))
    }

    /// The intro / outro has been started (or there was none to start) and is no longer playing.
    pub fn finished(&self, which: Which) -> bool {
        let started = match which {
            Which::Intro => self.intro_started,
            Which::Outro => self.outro_started,
        };
        started && self.playing.as_ref().is_none_or(|p| p.which != which)
    }

    /// A new runtime race: its cutscenes are still to come.
    pub fn reset(&mut self) {
        self.intro_started = false;
        self.outro_started = false;
        self.logic_intro = false;
        self.logic_outro = false;
    }

    fn trace(&mut self, line: std::fmt::Arguments) {
        if !self.trace_opened {
            self.trace_opened = true;
            if let Some(path) = std::env::var_os("MW_CUTSCENE_TRACE") {
                self.trace = std::fs::File::create(path).ok();
                if let Some(f) = &mut self.trace {
                    let _ = writeln!(f, "# <which> <t> <time_scale> <camera> <eye x y z> <look x y z> <fov_deg>");
                }
            }
        }
        if let Some(f) = &mut self.trace {
            let _ = f.write_fmt(line);
            let _ = f.write_all(b"\n");
        }
    }
}

/// The letterbox bars this frame (`UILayout` 0x19c66: black, 1280×720 reference frame); the crash camera and
/// the cutscenes ask for them, [`draw_letterbox`] draws them and clears the request.
#[derive(Resource, Default)]
pub struct LetterboxBars {
    request: Option<([f32; 2], f32)>,
}

impl LetterboxBars {
    /// Show bars of these heights (reference pixels of 720: top, bottom), slid out by `out` (0..1).
    pub fn show(&mut self, bars: [f32; 2], out: f32) {
        self.request = Some((bars, out));
    }
}

#[derive(Component)]
struct LetterboxBar {
    top: bool,
}

fn spawn_letterbox(mut commands: Commands) {
    for top in [true, false] {
        let mut node = Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), ..default() };
        if top {
            node.top = Val::Px(0.0);
        } else {
            node.bottom = Val::Px(0.0);
        }
        commands.spawn((
            Name::new(if top { "letterbox top" } else { "letterbox bottom" }),
            node,
            BackgroundColor(Color::BLACK),
            GlobalZIndex(10),
            Visibility::Hidden,
            LetterboxBar { top },
        ));
    }
}

fn draw_letterbox(mut req: ResMut<LetterboxBars>, mut bars: Query<(&LetterboxBar, &mut Node, &mut Visibility)>) {
    let request = req.request.take();
    for (bar, mut node, mut vis) in &mut bars {
        let Some((heights, out)) = request else {
            vis.set_if_neq(Visibility::Hidden);
            continue;
        };
        vis.set_if_neq(Visibility::Inherited);
        // Visible height of 720, and the slide-out (`TransitionOutBehaviour`: 150 px over its duration).
        let h = if bar.top { heights[0] } else { heights[1] };
        node.height = Val::Vh(h / 7.2);
        let slide = Val::Vh(-out * 150.0 / 7.2);
        if bar.top {
            node.top = slide;
        } else {
            node.bottom = slide;
        }
    }
}

/// With an intro to play, the race waits for it from the start.
fn hold_for_intro(setup: Res<Setup>, race: Option<ResMut<RaceState>>) {
    if let Some(mut race) = race
        && (setup.0.intro.is_some() || setup.0.force_outro)
    {
        race.cutscene = true;
    }
}

/// Start the intro once the world, the player's car and the opponents are in place; the outro at the finish.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
fn start_cutscenes(
    setup: Res<Setup>,
    loaded: Res<crate::Loaded>,
    mut state: ResMut<CutsceneState>,
    race: Option<ResMut<RaceState>>,
    virtual_time: Res<Time<Virtual>>,
    player: Query<(Entity, Option<&crate::race::RacerProgress>), (With<Car>, With<Player>)>,
    prewarm: Option<Res<crate::prewarm::Prewarm>>,
) {
    let Some(mut race) = race else { return };
    // During the race the flag says an outro is still to come (`--test race` waits for it).
    if race.phase == RacePhase::Racing && setup.0.outro.is_some() && !state.outro_started {
        race.cutscene = true;
    }
    if state.playing.is_some() {
        return;
    }
    let Ok((_, progress)) = player.single() else { return };
    let start = |which, seq: &Arc<Sequence>, base_speed| Playing {
        which,
        seq: seq.clone(),
        t: env_seconds("MW_CUTSCENE_START").unwrap_or(0.0),
        clock: mw_assets::timeline::SequenceClock::new(&seq.clocks, env_seconds("MW_CUTSCENE_START").unwrap_or(0.0)),
        started: vec![false; seq.items.len()],
        base_speed,
        last_view: None,
        released: false,
        driving: None,
        hidden: Vec::new(),
        frozen: Default::default(),
    };
    let finished = race.phase == RacePhase::Finished || progress.is_some_and(|p| p.finished.is_some());
    // Driven: the outro waits for the logic's `PlaySequence` of it.
    let finished = finished && (!state.logic_driven || state.logic_outro);
    if !state.outro_started && (finished || (setup.0.force_outro && loaded.done && !race.hold)) {
        state.outro_started = true;
        state.intro_started = true;
        if let Some(seq) = &setup.0.outro {
            info!("cutscene: outro {} ({:.1} s, {} items)", seq.unique_id, seq.length, seq.items.len());
            race.cutscene = true;
            if setup.0.force_outro {
                race.rolling_start = true;
            }
            state.playing = Some(start(Which::Outro, seq, virtual_time.relative_speed()));
        }
        return;
    }
    // The pipeline pre-warm (`prewarm.rs`) runs before the intro, while the world is still the loading view.
    if state.intro_started
        || race.phase != RacePhase::Waiting
        || !loaded.done
        || race.hold
        || prewarm.is_some_and(|p| p.pending())
        || state.hold_intro
        || (state.logic_driven && !state.logic_intro && setup.0.intro.is_some())
    {
        return;
    }
    state.intro_started = true;
    match &setup.0.intro {
        Some(seq) => {
            info!(
                "cutscene: intro {} ({:.1} s, {} items: {:?}; skip {:?})",
                seq.unique_id,
                seq.length,
                seq.items.len(),
                seq.summary(),
                seq.skip()
            );
            state.playing = Some(start(Which::Intro, seq, virtual_time.relative_speed()));
        }
        None => race.cutscene = false,
    }
}

/// Advance the playing sequence: skip, items, camera, HUD, letterbox, slow motion, the race hand-over.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run_cutscene(
    mut commands: Commands,
    setup: Res<Setup>,
    game: Res<crate::Game>,
    real: Res<Time<Real>>,
    mut virtual_time: ResMut<Time<Virtual>>,
    mut state: ResMut<CutsceneState>,
    mut race: Option<ResMut<RaceState>>,
    (mut hud, mut letterbox, mut snap, mut picture): (
        ResMut<crate::hud::HudSettings>,
        ResMut<LetterboxBars>,
        ResMut<crate::camera::SnapCamera>,
        ResMut<crate::sequence_picture::SequencePicture>,
    ),
    (keys, pads, menu, view): (
        Res<ButtonInput<KeyCode>>,
        Query<&Gamepad>,
        Option<Res<crate::input::MenuInput>>,
        Option<NonSend<crate::ui_sequence::LogicView>>,
    ),
    (ai_race, mut traffic): (Option<Res<crate::ai::RaceSetup>>, Option<ResMut<crate::traffic::TrafficSystem>>),
    racers: Query<(Entity, &Racer, &Transform), Without<Camera3d>>,
    player: Query<(Entity, &Transform), (With<Car>, With<Player>, Without<Camera3d>)>,
    mut visibility: Query<&mut Visibility, With<Car>>,
    mut camera: Query<(&mut Transform, &mut Projection), crate::camera::DrivingCamera>,
    mut exit: MessageWriter<AppExit>,
) {
    // `MW_CUTSCENE_SHOT`: exit after the capture.
    if let Some(n) = state.shot_frames.as_mut() {
        *n += 1;
        if *n == 30 {
            exit.write(AppExit::Success);
        }
    }
    let Some(mut p) = state.playing.take() else { return };
    let player = player.single().ok();
    if let (Some((which, at)), Some(path), None) = (shot_at(), &game.screenshot, state.shot_frames)
        && which.is_none_or(|w| w == p.which)
        && p.t >= at
    {
        info!("cutscene: screenshot at {:.2} s of the {:?}", p.t, p.which);
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(path.clone()));
        state.shot_frames = Some(0);
    }
    // The clocks: real time, sped up with the run (tests run the simulation faster than real time: the sequence
    // keeps pace with it, so the intro hands over where it does at 1x), moved by the timeline controllers. The
    // skip is the intro's `JumpOnButtonPress` (SKIP_CUTSCENE).
    let seq = p.seq.clone();
    {
        let pad_list: Vec<&Gamepad> = pads.iter().collect();
        let dev = crate::input::Devices { keys: &keys, pads: &pad_list };
        let skip = if setup.0.auto_skip && p.which == Which::Intro { 2 } else { crate::seq_clock::skip_state(&dev) };
        let view = view.as_deref();
        let mut holds = |b: &str| {
            crate::frontend::binding_true(b, &mut |path, arg| crate::ui_sequence::logic_lookup(view, path, arg))
        };
        let mut env = crate::seq_clock::GameEnv { holds: &mut holds, menu: menu.as_deref(), skip };
        let jumps = p.clock.tick(&seq.clocks, real.delta_secs() * p.base_speed, &mut env);
        crate::seq_clock::log_jumps("cutscene", &seq, &jumps);
        if jumps.iter().any(|j| j.node == 0 && j.kind == "JumpOnButtonPress") {
            snap.0 = true;
        }
    }
    p.t = p.clock.time();
    let t = p.t;
    // Picture and environment items (PostFX states, time of day, ...): `sequence_picture.rs`.
    picture.play(if p.which == Which::Intro { "cutscene intro" } else { "cutscene outro" }, &seq, t);

    // Slow motion: the simulation's clock (the sequence runs on real time).
    let scale = seq.time_scale_at(&|i| p.clock.frame_time(&seq.clocks, i.clock)).max(0.0);
    virtual_time.set_relative_speed(p.base_speed * scale);

    // Racer lookup: the player, or an opponent by `Players.Racer[n]` (grid slot n − 1).
    let racer_entity = |r: RacerRef| -> Option<Entity> {
        match r {
            RacerRef::LocalPlayer => player.map(|(e, _)| e),
            RacerRef::Racer(n) => racers
                .iter()
                .find(|(_, rc, _)| !rc.player && (n as usize).checked_sub(1).is_some_and(|k| rc.grid_slot == Some(k)))
                .map(|(e, ..)| e),
            RacerRef::Other => None,
        }
    };

    let mut hide_hud = false;
    let mut drive: Option<f32> = None;
    let mut hidden: Vec<Entity> = Vec::new();
    let mut view: Option<(Transform, f32, &'static str)> = None;
    let mut blend: Option<(f32, f32)> = None;
    // An ICE take's GAME_CAM_BLEND (1 = the gameplay camera).
    let mut ice_blend = 0.0f32;
    state.car_pose = None;
    for (index, item) in seq.items.iter().enumerate() {
        // The item's own clock (its conducted sequence's); out of range or not conducted: it stops, and starts
        // again when a jump brings it back (`FUN_00a42b90`).
        let Some(t) = p.clock.frame_time(&seq.clocks, item.clock).filter(|t| *t >= item.start && *t < item.end) else {
            p.started[index] = false;
            continue;
        };
        let first = !p.started[index];
        p.started[index] = true;
        let local = t - item.start;
        let u = local / (item.end - item.start).max(1e-6);
        match &item.kind {
            ItemKind::HideHud => hide_hud = true,
            ItemKind::Letterbox(l) => {
                if let Some(out) = l.state(t) {
                    letterbox.show(l.bars, out);
                }
            }
            ItemKind::EventTrigger if first && p.which == Which::Intro => {
                if let Some(race) = race.as_mut() {
                    release(race, &mut p);
                }
            }
            ItemKind::Teleport(tp) if first => {
                let Some(entity) = racer_entity(tp.racer) else { continue };
                // Model decision: an opponent whose destination is the player's own (462174's Racer[8] and the
                // player both go to (-626, 52, 2392)) is not moved. The original seats fewer opponents there
                // (slots driving the player's car are skipped, `c75c9d90`), our opponents' cars are a model.
                let player_spot = seq.items.iter().find_map(|i| match &i.kind {
                    ItemKind::Teleport(p) if p.racer == RacerRef::LocalPlayer => Some(Vec3::from(p.position)),
                    _ => None,
                });
                if tp.racer != RacerRef::LocalPlayer
                    && player_spot.is_some_and(|p| p.distance(Vec3::from(tp.position)) < 3.0)
                {
                    info!("cutscene: {:?} not teleported onto the player's start", tp.racer);
                    continue;
                }
                let dir = Vec3::from(tp.direction).with_y(0.0).normalize_or(Vec3::Z);
                state.pending.push(Pending::Teleport {
                    entity,
                    position: Vec3::from(tp.position),
                    direction: dir,
                    speed: tp.speed_mph * MPH,
                });
                if let Some(race) = race.as_mut() {
                    race.rolling_start = true;
                }
                if tp.racer == RacerRef::LocalPlayer {
                    snap.0 = true;
                }
            }
            ItemKind::DriveWithAi { max_speed_mph, .. } => drive = Some(*max_speed_mph),
            ItemKind::ClearTraffic(c) => {
                if let Some(traffic) = traffic.as_mut() {
                    let mut points: Vec<Vec3> = c.points.iter().map(|p| Vec3::from(*p)).collect();
                    if c.binding.contains("LocalPlayer") {
                        points.extend(player.map(|(_, t)| t.translation));
                    }
                    for p in points {
                        traffic.clear_near(p, c.radius, &mut commands);
                    }
                }
            }
            ItemKind::HideRacecar(r) => hidden.extend(racer_entity(*r)),
            ItemKind::AnimatedCar(c) => {
                if let (Some(entity), Some(k0)) = (racer_entity(c.racer), c.key_at(local)) {
                    let k1 = c.key_at(local + 1.0 / c.frame_rate.max(1.0)).unwrap_or(k0);
                    let (a, b) = (Vec3::from(k0.translation), Vec3::from(k1.translation));
                    let mut velocity = (b - a) * c.frame_rate.max(1.0);
                    // A cut in the clip (the car jumps between shots) is no speed.
                    if velocity.length() > 120.0 {
                        velocity = Vec3::ZERO;
                    }
                    state.car_pose = Some((entity, a, Quat::from_array(k0.rotation).normalize(), velocity));
                }
            }
            ItemKind::AnimatedCamera(c) => {
                let relative = (!c.relative_to.is_empty())
                    .then(|| racer_transform(&c.relative_to, player.map(|(_, t)| *t), &racers))
                    .flatten();
                if let Some(v) = animated_camera_view(c, local, u, relative, !c.relative_to.is_empty()) {
                    view = Some((v.0, v.1, "animated"));
                }
            }
            ItemKind::DynamicCamera(d) => blend = Some(d.factors(u)),
            ItemKind::Camera(shots) => {
                let Some((shot, take)) = shots.iter().find_map(|s| Some((s, s.take_data.as_ref()?))) else { continue };
                // The bound transforms, sampled when the take starts (flags set) or every frame.
                let live: Vec<Frame> = shot
                    .bindings
                    .iter()
                    .map(|b| racer_transform(b, player.map(|(_, t)| *t), &racers).map_or(Frame::IDENTITY, frame_of))
                    .collect();
                let frozen = p.frozen.entry(index).or_insert_with(|| live.clone());
                let avatars: Vec<Frame> = live
                    .iter()
                    .enumerate()
                    .map(|(i, l)| if shot.binding_flags.get(i).copied().unwrap_or(false) { frozen[i] } else { *l })
                    .collect();
                let subject = player.map_or(Frame::IDENTITY, |(_, t)| frame_of(*t));
                let cam = take.camera(local, &Bindings { subject, avatars: &avatars });
                ice_blend = cam.game_camera_blend.clamp(0.0, 1.0);
                // The take's frame: x left, y up, z forward (game axes); Bevy's camera: x right, y up, looks −Z.
                let (x, y, z) = (Vec3::from(cam.frame.x), Vec3::from(cam.frame.y), Vec3::from(cam.frame.z));
                let rotation = Quat::from_mat3(&Mat3::from_cols(-x, y, -z)).normalize();
                view = Some((
                    Transform { translation: Vec3::from(cam.eye), rotation, scale: Vec3::ONE },
                    cam.fov_degrees,
                    "ice",
                ));
            }
            _ => {}
        }
    }

    // The AI drives the player's car while a `DriveLocalPlayerWithAISequenceItem` runs.
    match (drive, p.driving, player) {
        (Some(max_mph), None, Some((entity, car_t))) => {
            let route = match p.which {
                Which::Intro => ai_race.as_ref().map(|r| r.route.clone()),
                // After the finish the race route ends: a route along the road graph ahead of the car.
                Which::Outro => game.store.ai_data().ok().and_then(|g| {
                    let fwd = car_t.rotation * Vec3::Z;
                    crate::ai::route_from_ai_graph(&g, car_t.translation, fwd.x.atan2(fwd.z), 3000.0).map(Arc::new)
                }),
            };
            if let Some(route) = route {
                let mut driver = crate::ai::AiDriver::new(route);
                driver.schedule_speed = max_mph * MPH;
                commands.entity(entity).insert((driver, crate::drive::ExternalControls));
                p.driving = Some(entity);
                info!("cutscene: the AI drives the player's car (at most {max_mph} mph)");
            }
        }
        (None, Some(entity), _) => {
            stop_driving(&mut commands, entity);
            p.driving = None;
        }
        _ => {}
    }

    // Hidden racers.
    for e in &p.hidden {
        if !hidden.contains(e)
            && let Ok(mut v) = visibility.get_mut(*e)
        {
            v.set_if_neq(Visibility::Inherited);
        }
    }
    for e in &hidden {
        if let Ok(mut v) = visibility.get_mut(*e) {
            v.set_if_neq(Visibility::Hidden);
        }
    }
    p.hidden = hidden;
    hud.hidden_by_sequence = hide_hud;

    // The camera: a cinematic view overrides what the gameplay camera placed this frame; the dynamic camera
    // blends from the last cinematic view to the gameplay one.
    if let Ok((mut cam, mut projection)) = camera.single_mut() {
        let gameplay_fov = match &*projection {
            // In the game's (horizontal) degrees, as the cinematic FOVs.
            Projection::Perspective(pp) => crate::camera::game_fov(pp),
            _ => 60.0,
        };
        let out = match (view, blend, p.last_view) {
            (Some((v, fov, kind)), _, _) if ice_blend > 0.0 => {
                p.last_view = Some((v, fov));
                let mixed = Transform {
                    translation: v.translation.lerp(cam.translation, ice_blend),
                    rotation: v.rotation.slerp(cam.rotation, ice_blend),
                    scale: Vec3::ONE,
                };
                Some((mixed, fov + (gameplay_fov - fov) * ice_blend, kind))
            }
            (Some((v, fov, kind)), _, _) => {
                p.last_view = Some((v, fov));
                Some((v, fov, kind))
            }
            (None, Some((ft, ff)), Some((from, from_fov))) => {
                let v = Transform {
                    translation: from.translation.lerp(cam.translation, ft),
                    rotation: from.rotation.slerp(cam.rotation, ft),
                    scale: Vec3::ONE,
                };
                Some((v, from_fov + (gameplay_fov - from_fov) * ff, "blend"))
            }
            _ => None,
        };
        if let Some((v, fov, _)) = out {
            *cam = v;
            if let Projection::Perspective(pp) = &mut *projection {
                pp.fov = crate::camera::projection_fov(fov, pp.aspect_ratio);
            }
        }
        let (kind, fov) = out.map_or(("gameplay", gameplay_fov), |o| (o.2, o.1));
        let (e, l) = (cam.translation, cam.translation + cam.forward() * 10.0);
        state.trace(format_args!(
            "{:?} {t:.3} {scale:.3} {kind} {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} {fov:.1}",
            p.which, e.x, e.y, e.z, l.x, l.y, l.z
        ));
    }

    if t >= seq.length || p.clock.quit() {
        // The end: time, HUD and the car back to normal.
        virtual_time.set_relative_speed(p.base_speed);
        hud.hidden_by_sequence = false;
        if let Some(entity) = p.driving.take() {
            stop_driving(&mut commands, entity);
        }
        for e in &p.hidden {
            if let Ok(mut v) = visibility.get_mut(*e) {
                v.set_if_neq(Visibility::Inherited);
            }
        }
        if let Some(race) = race.as_mut() {
            if p.which == Which::Intro && !p.released {
                release(race, &mut p);
            }
            race.cutscene = false;
        }
        info!("cutscene: {:?} over after {t:.2} s", p.which);
        return;
    }
    state.playing = Some(p);
}

/// A transform as the game's row-major frame (rows: the local x, y, z axes, then the position).
pub(crate) fn frame_of(t: Transform) -> Frame {
    Frame {
        x: (t.rotation * Vec3::X).to_array(),
        y: (t.rotation * Vec3::Y).to_array(),
        z: (t.rotation * Vec3::Z).to_array(),
        pos: t.translation.to_array(),
    }
}

/// `MW_CUTSCENE_SHOT`: `[intro:|outro:]<seconds>`.
fn shot_at() -> Option<(Option<Which>, f32)> {
    let v = std::env::var("MW_CUTSCENE_SHOT").ok()?;
    let (which, t) = match v.split_once(':') {
        Some(("intro", t)) => (Some(Which::Intro), t),
        Some(("outro", t)) => (Some(Which::Outro), t),
        _ => (None, v.as_str()),
    };
    Some((which, t.parse().ok()?))
}

fn env_seconds(name: &str) -> Option<f32> {
    std::env::var(name).ok()?.parse().ok()
}

/// Hand the start to the race: no freeze, no countdown (the offline flow has none).
fn release(race: &mut RaceState, p: &mut Playing) {
    p.released = true;
    race.intro = 0.0;
    race.cutscene = false;
    info!("cutscene: the race starts at {:.2} s of the intro (event trigger)", p.t);
}

fn stop_driving(commands: &mut Commands, entity: Entity) {
    if let Ok(mut e) = commands.get_entity(entity) {
        e.remove::<(crate::ai::AiDriver, crate::drive::ExternalControls)>();
    }
}

/// The transform a `...GetTransform` binding names.
fn racer_transform(
    binding: &str,
    player: Option<Transform>,
    racers: &Query<(Entity, &Racer, &Transform), Without<Camera3d>>,
) -> Option<Transform> {
    match RacerRef::parse(binding) {
        RacerRef::LocalPlayer => player,
        RacerRef::Racer(n) => racers
            .iter()
            .find(|(_, rc, _)| !rc.player && (n as usize).checked_sub(1).is_some_and(|k| rc.grid_slot == Some(k)))
            .map(|(_, _, t)| *t),
        RacerRef::Other => None,
    }
}

/// The camera an `AnimatedCameraSequenceItem` gives at `local` seconds (`u` through the item): the clip's camera
/// bone (in world space, or in the bound transform's frame) and the FOV of its focal length.
fn animated_camera_view(
    c: &AnimatedCamera,
    local: f32,
    u: f32,
    relative: Option<Transform>,
    wants_relative: bool,
) -> Option<(Transform, f32)> {
    let k = c.key_at(local)?;
    if wants_relative && relative.is_none() {
        return None;
    }
    let mut position = Vec3::from(k.translation);
    let mut rotation = Quat::from_array(k.rotation).normalize();
    if let Some(r) = relative {
        position = r.transform_point(position);
        rotation = r.rotation * rotation;
    }
    // The bone looks along its +Z with its +Y up, its full orientation used (roll included: 462174's intro is
    // upside down in the original, capture 2026-10-09); Bevy's camera looks along −Z with +Y up.
    let fix = Quat::from_rotation_y(std::f32::consts::PI);
    let fov = AnimatedCamera::fov_degrees(c.focal_length.at(u));
    Some((Transform { translation: position, rotation: rotation * fix, scale: Vec3::ONE }, fov))
}

/// Teleports and the animated car, before the physics step.
#[allow(clippy::type_complexity)]
fn apply_cars(
    mut state: ResMut<CutsceneState>,
    spatial: SpatialQuery,
    mut cars: Query<(&mut Car, &mut Transform, &mut LinearVelocity, &mut AngularVelocity)>,
) {
    for pending in std::mem::take(&mut state.pending) {
        match pending {
            Pending::Teleport { entity, position, direction, speed } => {
                let Ok((mut car, mut t, mut lin, mut ang)) = cars.get_mut(entity) else { continue };
                // On the road under the trigger point (as `ai.rs` places the grid).
                let from = position + Vec3::Y * 5.0;
                let filter = SpatialQueryFilter::from_excluded_entities([entity, car.body_collider])
                    .with_mask(crate::props::GameLayer::World);
                let ground = spatial.cast_ray(from, Dir3::NEG_Y, 40.0, true, &filter).map(|h| from.y - h.distance);
                // On the ground, as the game places it (`FUN_00647e00`: y is the ground; the car origin is at road level): a
                // higher placement dropped the car and played a landing at every teleport.
                t.translation = position.with_y(ground.unwrap_or(position.y) + 0.02);
                t.rotation = Quat::from_rotation_y(direction.x.atan2(direction.z));
                lin.0 = direction * speed;
                ang.0 = Vec3::ZERO;
                // The game's placement of a moving car (`TeleportPlayerSequenceItem` → `FUN_00647e00` →
                // `FUN_008f8190` with the moving flag): the wheels roll at road speed and the gearbox is placed for
                // it. Without it the wheels kept their old spin and slid until they caught up: a full-intensity
                // wheelspin skid at every race start (user report 2026-11-17).
                car.place_moving(speed);
                info!("cutscene: teleport to {:.1?} at {:.0} mph", t.translation, speed / MPH);
            }
        }
    }
    if let Some((entity, position, rotation, velocity)) = state.car_pose
        && let Ok((_, mut t, mut lin, mut ang)) = cars.get_mut(entity)
    {
        t.translation = position;
        t.rotation = rotation;
        lin.0 = velocity;
        ang.0 = Vec3::ZERO;
    }
}
