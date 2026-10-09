//! Races: one offline event (`mw_assets::events`) run from the start to the result, by the rules of the
//! original's `CHECKPTS` behaviour and `DistanceToFinishManager` (docs/decomp.md "Races").
//!
//! `--race <event id>` (ids from `mwtool events`) loads the event, puts the player's car on its `SpawnPosition`
//! (with the event's first allowed car unless `--drive` names one), and runs
//! [`RacePhase::Waiting`] → [`RacePhase::Intro`] → [`RacePhase::Racing`] → [`RacePhase::Finished`].
//!
//! # Rules (sources in decomp.md "Races")
//!
//! - **Intro and countdown**: the intro lasts the game mode's `15f07dbf` (10 s for races, 0 for speed runs;
//!   `FUN_0070caa0`); the 3-2-1 shows while `IntroTime` = ⌊remaining + 1⌋ ≤ 3 (`FUN_006e2670`, the HSM's
//!   "Wait For Countdown Start" condition). The cars are frozen meanwhile: handbrake = max(input, 1)
//!   (`FREEZE_CAR`, `FUN_006c1a80`), so the throttle still revs the engine. At 0 the race clock starts.
//! - **Route entries** (`FUN_00645290`): entry 0 is the start, then every checkpoint of every lap; a gate's
//!   entry stands at its box centre lowered by half its height (on the road). Cumulative distances along the
//!   entries give the distance to the finish.
//! - **Checkpoints** (`FUN_0075da20`): only the racer's target gate is tested, every frame, as an overlap of the
//!   car's box with the trigger box (the trigger hit list, `FUN_00537190` / `FUN_00536f70`): held overlap,
//!   in order, not one-way (every checkpoint box has flags 0). After a hit the lap counter (from 1) goes up when
//!   the gate was the lap's last; the hit on the last entry is the finish.
//! - **Position** (`FUN_00749bb0`): 1 + the racers ahead, where ahead = finished, or a strictly smaller
//!   distance to the finish (`FUN_00661b40`: total − cumulative distance of the target + what is left of the
//!   target's segment). Finished racers keep their finishing place.
//! - **Wrong way / missed checkpoint** (`FUN_006585b0` state machine, per racer here; the original runs it for
//!   the local player): 1 s / 2 s / √10 m / 1 s thresholds as traced. No penalty in the code.
//! - **Finish** (`FUN_006b9d00`): the player's place and race time are stored; the event is won in the top
//!   three, or only in first place when the event's `c8f26580` is set. The race ends for the player then.
//! - **Respawn** (`FUN_006c9fb0`): a reset during a race faces the target gate (`RespawnFacing`, read by
//!   `crash.rs`).
//!
//! Model decisions (no source; decomp.md "Races"): the road-network route segments between entries are straight
//! lines here (the original asks the AI road network for each, `FUN_006423e0`); the car's overlap box is the
//! body's half extents centred that high above the car's origin; AI finishing times are recorded (the original
//! stores none); the race clock runs in fixed ticks.
//!
//! # API for the AI, HUD and cutscene code
//!
//! - [`RaceState`] (resource, present while a race is loaded): the event and its game mode, the phase, the
//!   intro and race clocks ([`RaceState::intro_left`], [`RaceState::countdown_digit`], [`RaceState::clock`]),
//!   the gates ([`RaceCheckpoint`]: transform, size, normal, ground point), the route entries
//!   ([`RaceState::entries`], start + every gate of every lap, with cumulative distances) and one lap as a
//!   polyline ([`RaceState::route`]), the player's start ([`RaceState::player_start`]) and the opponents' grid
//!   ([`RaceState::grid`]: one transform per `AIPlayerInformation` slot in data order, with its `AIPlayerType`),
//!   the live standings ([`RaceState::standings`], best first) and the results.
//! - [`Racer`] (component): put it on **any** entity with a `Transform` to make it race. The player's car gets
//!   one automatically; an AI car spawned on `RaceState::grid[i]` should get `Racer::ai(name, i)` (and its own
//!   half extents). [`RacerProgress`] is inserted by the race: lap, target gate, distance to the finish, lap
//!   times, finish time, position, wrong way / missed checkpoint.
//! - [`RaceState::hold`]: while true the race stays in `Waiting` (an intro cutscene, the AI being spawned).
//! - Messages: [`RacePhaseChanged`], [`CheckpointPassed`], [`LapCompleted`], [`RacerFinished`]. After an AI
//!   racer's `RacerFinished` the original sets its AI goal 3 (stop racing, `FUN_00843030`); when the player
//!   finishes every remaining AI racer gets that goal too (`CHECKPTS` exit).
//!
//! Minimal text (countdown, lap, position, time) is drawn with `game_text` until the HUD wave replaces it.
//! `--test race` carries the player's car through every gate kinematically (no driving involved), prints
//! `TEST RESULT race ...` and exits: it checks the race logic, not the handling.

// Parts of the API (grid slots, results, entries) are for the AI / HUD / cutscene code of the next wave.
#![allow(dead_code)]

use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;
use mw_assets::events::{Event, GameMode};

use crate::drive::{Car, CarInput};
use crate::game_text::{GameText, TITLE_FONT};

/// What `--race` hands to the plugin.
pub struct RaceSetup {
    pub event: Event,
    pub game_mode: Option<GameMode>,
    /// Display name (the event's `Name` text).
    pub name: String,
    /// `--test race`: the scripted run through the gates.
    pub test: bool,
}

pub struct RacePlugin {
    pub setup: Option<Arc<RaceSetup>>,
}

impl Plugin for RacePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<RacePhaseChanged>()
            .add_message::<CheckpointPassed>()
            .add_message::<LapCompleted>()
            .add_message::<RacerFinished>()
            .init_resource::<RespawnFacing>();
        let Some(setup) = &self.setup else { return };
        app.add_plugins(crate::chevron::ChevronPlugin)
            .add_systems(Startup, spawn_route_markers)
            .insert_resource(RaceState::new(setup))
            .add_systems(Update, (attach_player, spawn_text, update_text, log_messages))
            .add_systems(
                FixedUpdate,
                (hold_at_start.before(crate::drive::car_physics), run_race.after(crate::drive::car_physics)),
            );
        if setup.test {
            app.insert_resource(RaceTest::default())
                .add_systems(FixedUpdate, run_test.before(crate::drive::car_physics).before(hold_at_start))
                .add_systems(Startup, |mut t: ResMut<Time<Virtual>>| t.set_relative_speed(4.0));
        }
    }
}

/// The intro when the event's game mode is missing: the race mode's `15f07dbf` (10 s).
const DEFAULT_INTRO_SECS: f32 = 10.0;
/// Wrong-way / missed-checkpoint thresholds (`FUN_006585b0`; `.bss` 0x01177580, 0x01177388, 0x01177378 read
/// from the snapshot in ticks of 1/3e6 s, `DAT_00fb77b8` = 10 compared with the squared distance).
const MISSED_AFTER: f32 = 1.0;
const WRONG_WAY_AFTER: f32 = 2.0;
const WRONG_WAY_MOVED_SQ: f32 = 10.0;
const SETTLE: f32 = 1.0;
/// `FUN_00661b40`: a car more than this far behind its segment's start adds the straight distance to it.
const BEHIND_START: f32 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RacePhase {
    /// Loaded; waiting for the world, the player's car and anything holding the start ([`RaceState::hold`]).
    Waiting,
    /// The game mode's intro (cutscene, then the 3-2-1 in its last 3 s); cars frozen.
    Intro,
    /// Go: the race clock runs.
    Racing,
    /// The player has finished.
    Finished,
}

/// A checkpoint gate (a trigger box of the world's `TriggerData`).
#[derive(Debug, Clone, Copy)]
pub struct RaceCheckpoint {
    /// Trigger id.
    pub id: u32,
    /// Centre and orientation (local Z = the gate's normal, the driving direction through it).
    pub transform: Transform,
    /// Full extents: across the road, height, depth.
    pub size: Vec3,
}

impl RaceCheckpoint {
    pub fn normal(&self) -> Vec3 {
        self.transform.rotation * Vec3::Z
    }

    /// The route entry point: the box centre lowered by half its height (`FUN_00645290`), on the road.
    pub fn ground(&self) -> Vec3 {
        self.transform.translation - Vec3::Y * (self.size.y * 0.5)
    }

    /// Whether a box (centre, rotation, half extents) overlaps the gate box (separating-axis test).
    pub fn overlaps(&self, centre: Vec3, rotation: Quat, half: Vec3) -> bool {
        let a =
            [self.transform.rotation * Vec3::X, self.transform.rotation * Vec3::Y, self.transform.rotation * Vec3::Z];
        let b = [rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z];
        let ha = self.size * 0.5;
        let d = centre - self.transform.translation;
        let mut axes: Vec<Vec3> = a.iter().chain(b.iter()).copied().collect();
        for x in &a {
            for y in &b {
                let c = x.cross(*y);
                if c.length_squared() > 1e-8 {
                    axes.push(c.normalize());
                }
            }
        }
        axes.iter().all(|l| {
            let ra = ha.x * a[0].dot(*l).abs() + ha.y * a[1].dot(*l).abs() + ha.z * a[2].dot(*l).abs();
            let rb = half.x * b[0].dot(*l).abs() + half.y * b[1].dot(*l).abs() + half.z * b[2].dot(*l).abs();
            d.dot(*l).abs() <= ra + rb
        })
    }
}

/// One route entry: the start or a gate of a lap.
#[derive(Debug, Clone, Copy)]
pub struct RouteEntry {
    pub position: Vec3,
    /// Gate index into [`RaceState::checkpoints`] (`None` for the start).
    pub gate: Option<usize>,
    /// Route distance from the start (straight segments; model decision).
    pub distance: f32,
}

/// An opponent's start slot.
#[derive(Debug, Clone, Copy)]
pub struct GridSlot {
    /// On the trigger point, facing its direction (`FUN_00686f30` / `FUN_0042f250`); spawn with zero velocity.
    pub transform: Transform,
    /// `AIPlayerType` id (GAMEPLAY.BNDL): the driver profile.
    pub ai_type: u32,
    /// `AIPlayerInformation` `9b4e35f7` / `2968e77a` (open; the latter matches paint `ColourId`s).
    pub unknown: u32,
    pub colour: u32,
}

/// A racer's final result.
#[derive(Debug, Clone)]
pub struct RaceResult {
    pub racer: Entity,
    pub name: String,
    pub player: bool,
    pub position: usize,
    /// Race time (s) at the finish.
    pub time: f32,
    pub lap_times: Vec<f32>,
    pub best_lap: Option<f32>,
}

#[derive(Resource)]
pub struct RaceState {
    pub event: Event,
    pub game_mode: Option<GameMode>,
    pub name: String,
    pub phase: RacePhase,
    /// The intro's length and the time spent in it (s).
    pub intro: f32,
    pub intro_elapsed: f32,
    /// Race time since go (s).
    pub clock: f32,
    /// Laps to drive (1 for sprints).
    pub laps: u8,
    pub checkpoints: Vec<RaceCheckpoint>,
    /// Start, then every gate of every lap.
    pub entries: Vec<RouteEntry>,
    pub player_start: Transform,
    pub grid: Vec<GridSlot>,
    /// Racers, best first (updated every tick while racing).
    pub standings: Vec<Entity>,
    pub results: Vec<RaceResult>,
    /// The player won (`FUN_006b9d00`'s rule), once finished.
    pub won: Option<bool>,
    /// Hold the start in `Waiting` (intro sequence, AI not spawned yet).
    pub hold: bool,
}

fn heading_rotation(direction: [f32; 3]) -> Quat {
    Quat::from_rotation_y(direction[0].atan2(direction[2]))
}

impl RaceState {
    fn new(setup: &RaceSetup) -> Self {
        let e = &setup.event;
        let checkpoints: Vec<RaceCheckpoint> = e
            .checkpoints
            .iter()
            .map(|g| RaceCheckpoint {
                id: g.id,
                transform: Transform::from_translation(Vec3::from_array(g.position))
                    .with_rotation(Quat::from_array(g.rotation).normalize()),
                size: Vec3::from_array(g.size),
            })
            .collect();
        let player_start = e.spawn.map_or(Transform::IDENTITY, |s| {
            Transform::from_translation(Vec3::from_array(s.position)).with_rotation(heading_rotation(s.direction))
        });
        let laps = e.laps.max(1);
        let mut entries = vec![RouteEntry { position: player_start.translation, gate: None, distance: 0.0 }];
        for _ in 0..laps {
            for (i, g) in checkpoints.iter().enumerate() {
                let prev = entries.last().expect("start entry");
                let p = g.ground();
                entries.push(RouteEntry {
                    position: p,
                    gate: Some(i),
                    distance: prev.distance + prev.position.distance(p),
                });
            }
        }
        let grid = e
            .opponents
            .iter()
            .filter_map(|o| {
                let s = o.start?;
                Some(GridSlot {
                    transform: Transform::from_translation(Vec3::from_array(s.position))
                        .with_rotation(heading_rotation(s.direction)),
                    ai_type: o.ai_type,
                    unknown: o.unknown_9b4e,
                    colour: o.colour,
                })
            })
            .collect();
        Self {
            event: e.clone(),
            game_mode: setup.game_mode,
            name: setup.name.clone(),
            phase: RacePhase::Waiting,
            intro: setup.game_mode.map_or(DEFAULT_INTRO_SECS, |m| m.intro_time.max(0.0)),
            intro_elapsed: 0.0,
            clock: 0.0,
            laps,
            checkpoints,
            entries,
            player_start,
            grid,
            standings: Vec::new(),
            results: Vec::new(),
            won: None,
            hold: false,
        }
    }

    /// Seconds left in the intro.
    pub fn intro_left(&self) -> f32 {
        (self.intro - self.intro_elapsed).max(0.0)
    }

    /// The original's `IntroTime` (`FUN_006e2670`): whole seconds left plus one; the countdown shows it while
    /// it is 3 or less. `None` outside the intro.
    pub fn countdown_digit(&self) -> Option<u32> {
        (self.phase == RacePhase::Intro).then(|| (self.intro_left() + 1.0).floor() as u32).filter(|&d| d <= 3)
    }

    /// One lap as a polyline: the start, then every gate's ground point in order.
    pub fn route(&self) -> Vec<Vec3> {
        std::iter::once(self.player_start.translation).chain(self.checkpoints.iter().map(|c| c.ground())).collect()
    }

    pub fn gates_per_lap(&self) -> usize {
        self.checkpoints.len()
    }

    /// Gates to pass to finish (entries after the start).
    pub fn total_gates(&self) -> usize {
        self.entries.len() - 1
    }

    /// Route length to the finish.
    pub fn total_distance(&self) -> f32 {
        self.entries.last().map_or(0.0, |e| e.distance)
    }

    /// Distance to the finish through route entry `target` (`FUN_00661b40`, with straight segments) and the
    /// distance from the car to its projection on that segment.
    pub fn distance_to_finish(&self, target: usize, pos: Vec3) -> (f32, f32) {
        let Some(t) = self.entries.get(target) else { return (0.0, 0.0) };
        let start = self.entries[target.saturating_sub(1)].position;
        let seg = t.position - start;
        let len = seg.length();
        let along = if len > 1e-3 { (pos - start).dot(seg) / len } else { 0.0 };
        let clamped = along.clamp(0.0, len);
        let projection = start + if len > 1e-3 { seg / len * clamped } else { Vec3::ZERO };
        let mut d = self.total_distance() - t.distance + (len - clamped);
        if along < 0.0 && pos.distance(start) > BEHIND_START {
            d += pos.distance(start);
        }
        (d, pos.distance(projection))
    }
}

/// Marks an entity as a racer. Any entity with a `Transform` can race.
#[derive(Component, Debug, Clone)]
pub struct Racer {
    pub name: String,
    pub player: bool,
    /// Index into [`RaceState::grid`] for opponents.
    pub grid_slot: Option<usize>,
    /// The car's box for the gate overlap (half extents, m), centred this high above the origin.
    pub half_extents: Vec3,
}

impl Racer {
    pub fn player(half_extents: Vec3) -> Self {
        Self { name: "Player".into(), player: true, grid_slot: None, half_extents }
    }

    pub fn ai(name: impl Into<String>, grid_slot: usize, half_extents: Vec3) -> Self {
        Self { name: name.into(), player: false, grid_slot: Some(grid_slot), half_extents }
    }
}

/// A racer's progress (inserted by the race on every [`Racer`]).
#[derive(Component, Debug, Clone, Default)]
pub struct RacerProgress {
    /// Current lap, 1-based (0 before go).
    pub lap: u8,
    /// Gates passed (0-based checkpoint counter `cp`); the target is route entry `cp + 1`.
    pub passed: usize,
    /// Distance to the finish (m).
    pub to_finish: f32,
    /// Race time when the current lap started, and the finished laps' times.
    pub lap_start: f32,
    pub lap_times: Vec<f32>,
    /// Race time at the finish.
    pub finished: Option<f32>,
    /// 1-based position.
    pub position: usize,
    pub wrong_way: bool,
    pub missed_checkpoint: bool,
    /// Distance driven while neither going the wrong way nor past a missed checkpoint (m): the speed run's
    /// average speed is this over the race time (`FUN_006e0580`; the accumulation itself is ours).
    pub counted_distance: f32,
    ww: WrongWay,
    last_pos: Option<Vec3>,
}

impl RacerProgress {
    /// The target gate's route entry.
    pub fn target(&self) -> usize {
        self.passed + 1
    }

    /// Average speed over the counted distance (mph).
    pub fn average_mph(&self, clock: f32) -> f32 {
        if clock > 0.0 { self.counted_distance / clock / 0.447_04 } else { 0.0 }
    }
}

/// `FUN_006585b0`'s state.
#[derive(Debug, Clone, Copy, Default)]
struct WrongWay {
    state: u8,
    timer: f32,
    stored: Vec3,
    missed: bool,
    last_d: Option<f32>,
    last_next: Option<f32>,
}

/// Where a respawn during a race should face (the player's target gate; `FUN_006c9fb0`); read by `crash.rs`.
#[derive(Resource, Default)]
pub struct RespawnFacing(pub Option<Vec3>);

#[derive(Message, Debug, Clone, Copy)]
pub struct RacePhaseChanged {
    pub phase: RacePhase,
}

#[derive(Message, Debug, Clone, Copy)]
pub struct CheckpointPassed {
    pub racer: Entity,
    /// Gate index within the lap, the lap (1-based) and the race time.
    pub index: usize,
    pub lap: u8,
    pub time: f32,
}

#[derive(Message, Debug, Clone, Copy)]
pub struct LapCompleted {
    pub racer: Entity,
    /// The lap just completed (1-based), its time and the race time.
    pub lap: u8,
    pub lap_time: f32,
    pub time: f32,
}

#[derive(Message, Debug, Clone, Copy)]
pub struct RacerFinished {
    pub racer: Entity,
    pub position: usize,
    pub time: f32,
}

fn spawn_route_markers(
    mut commands: Commands,
    game: Res<crate::Game>,
    race: Res<RaceState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<crate::chevron::ChevronMaterial>>,
) {
    crate::chevron::spawn_markers(
        &mut commands,
        &game.store,
        &race.event.chevrons,
        &mut meshes,
        &mut images,
        &mut materials,
    );
}

/// Give the player's car (each new one after a swap too) its `Racer`.
fn attach_player(mut commands: Commands, cars: Query<(Entity, &Car), Without<Racer>>) {
    for (e, car) in &cars {
        commands.entity(e).insert((Racer::player(car.params.body_half_extents), RacerProgress::default()));
    }
}

/// `FREEZE_CAR`: before go the handbrake is held (`FUN_006c1a80`: handbrake = max(input, 1)); the throttle
/// still works.
fn hold_at_start(race: Res<RaceState>, mut input: ResMut<CarInput>) {
    if matches!(race.phase, RacePhase::Waiting | RacePhase::Intro) {
        input.handbrake = true;
    }
}

#[allow(clippy::too_many_arguments)]
fn run_race(
    time: Res<Time>,
    loaded: Res<crate::Loaded>,
    mut race: ResMut<RaceState>,
    mut facing: ResMut<RespawnFacing>,
    mut racers: Query<(Entity, &Racer, &mut RacerProgress, &Transform)>,
    players: Query<(), With<Car>>,
    mut phase_msg: MessageWriter<RacePhaseChanged>,
    mut cp_msg: MessageWriter<CheckpointPassed>,
    mut lap_msg: MessageWriter<LapCompleted>,
    mut fin_msg: MessageWriter<RacerFinished>,
) {
    let dt = time.delta_secs();
    let race = &mut *race;
    let set_phase = |race: &mut RaceState, phase: RacePhase, w: &mut MessageWriter<RacePhaseChanged>| {
        race.phase = phase;
        w.write(RacePhaseChanged { phase });
    };
    match race.phase {
        RacePhase::Waiting => {
            if loaded.done && !players.is_empty() && !race.hold {
                race.intro_elapsed = 0.0;
                set_phase(race, RacePhase::Intro, &mut phase_msg);
            }
            return;
        }
        RacePhase::Intro => {
            race.intro_elapsed += dt;
            if race.intro_elapsed < race.intro {
                return;
            }
            race.clock = 0.0;
            for (_, _, mut p, _) in &mut racers {
                *p = RacerProgress { lap: 1, ..default() };
            }
            set_phase(race, RacePhase::Racing, &mut phase_msg);
            return;
        }
        RacePhase::Finished => {
            race.clock += dt;
            return;
        }
        RacePhase::Racing => race.clock += dt,
    }
    let n = race.gates_per_lap();
    let last = race.total_gates();
    if n == 0 {
        return;
    }
    let now = race.clock;
    for (entity, racer, mut p, t) in &mut racers {
        if p.lap == 0 {
            // Joined after go (a swapped car): from the start of the race.
            *p = RacerProgress { lap: 1, ..default() };
        }
        if p.finished.is_some() {
            continue;
        }
        let pos = t.translation;
        if let Some(prev) = p.last_pos.replace(pos)
            && !p.wrong_way
            && !p.missed_checkpoint
        {
            p.counted_distance += prev.distance(pos);
        }
        let target = p.target();
        let gate = race.entries[target].gate.expect("entries after the start are gates");
        let centre = pos + t.rotation * Vec3::Y * racer.half_extents.y;
        if race.checkpoints[gate].overlaps(centre, t.rotation, racer.half_extents) {
            cp_msg.write(CheckpointPassed { racer: entity, index: gate, lap: p.lap, time: now });
            p.passed += 1;
            if target == last {
                let lap_time = now - p.lap_start;
                p.lap_times.push(lap_time);
                lap_msg.write(LapCompleted { racer: entity, lap: p.lap, lap_time, time: now });
                p.finished = Some(now);
                let position = race.results.len() + 1;
                p.position = position;
                p.to_finish = 0.0;
                p.wrong_way = false;
                p.missed_checkpoint = false;
                race.results.push(RaceResult {
                    racer: entity,
                    name: racer.name.clone(),
                    player: racer.player,
                    position,
                    time: now,
                    lap_times: p.lap_times.clone(),
                    best_lap: p.lap_times.iter().copied().reduce(f32::min),
                });
                if racer.player {
                    // FUN_006b9d00 / FUN_006f46e0: top three (first only with `c8f26580`); a speed run also
                    // needs an average speed of at least the last (lowest) TargetSpeed, mph.
                    let mut won = position <= if race.event.first_place_only { 1 } else { 3 };
                    if let Some(&least) = race.event.target_speed.last() {
                        won &= p.average_mph(now) >= least;
                    }
                    if let Some(&limit) = race.event.target_time.first() {
                        won &= now <= limit;
                    }
                    race.won = Some(won);
                }
                fin_msg.write(RacerFinished { racer: entity, position, time: now });
                continue;
            }
            if p.passed.is_multiple_of(n) {
                let lap_time = now - p.lap_start;
                p.lap_times.push(lap_time);
                lap_msg.write(LapCompleted { racer: entity, lap: p.lap, lap_time, time: now });
                p.lap_start = now;
                p.lap += 1;
            }
        }
        let target = p.target();
        let (d_finish, off) = race.distance_to_finish(target, pos);
        p.to_finish = d_finish;
        // FUN_006585b0: d through the target (+ the distance off the route), d' through the next one.
        let d = d_finish + off;
        let d_next = (target < last).then(|| race.distance_to_finish(target + 1, pos).0);
        let ww = &mut p.ww;
        let rose = ww.last_d.is_some_and(|l| d > l);
        let fell = ww.last_d.is_some_and(|l| d < l);
        let next_fell = matches!((d_next, ww.last_next), (Some(a), Some(b)) if a < b);
        ww.timer += dt;
        match ww.state {
            0 => {
                ww.timer = 0.0;
                ww.stored = pos;
                if rose {
                    ww.state = 1;
                }
            }
            1 => {
                if ww.timer > MISSED_AFTER && rose && next_fell {
                    ww.missed = true;
                    ww.state = 2;
                } else if ww.timer > WRONG_WAY_AFTER && pos.distance_squared(ww.stored) > WRONG_WAY_MOVED_SQ {
                    ww.missed = false;
                    ww.state = 2;
                } else if fell {
                    ww.state = 0;
                }
            }
            2 => {
                if !rose {
                    ww.state = 3;
                    ww.timer = 0.0;
                }
            }
            _ => {
                if rose {
                    ww.state = 2;
                } else if ww.timer > SETTLE {
                    ww.state = 0;
                }
            }
        }
        ww.last_d = Some(d);
        ww.last_next = d_next;
        let flagged = ww.state >= 2;
        let missed = ww.missed;
        p.wrong_way = flagged && !missed;
        // HasMissedCheckpoint only for checkpoints 1..count−2 (`FUN_006584d0`).
        p.missed_checkpoint = flagged && missed && (1..last).contains(&p.passed);
        if racer.player {
            facing.0 = Some(race.entries[target].position);
        }
    }
    // Positions (FUN_00749bb0): 1 + racers ahead (finished, or strictly closer to the finish).
    let all: Vec<(Entity, Option<usize>, f32)> =
        racers.iter().map(|(e, _, p, _)| (e, p.finished.map(|_| p.position), p.to_finish)).collect();
    let mut order = Vec::with_capacity(all.len());
    for (e, place, d) in &all {
        let position = match place {
            Some(place) => *place,
            None => 1 + all.iter().filter(|(o, op, od)| o != e && (op.is_some() || od < d)).count(),
        };
        if let Ok((_, _, mut p, _)) = racers.get_mut(*e) {
            p.position = position;
        }
        order.push((position, *e));
    }
    order.sort_by_key(|o| o.0);
    race.standings = order.into_iter().map(|o| o.1).collect();
    if race.won.is_some() {
        facing.0 = None;
        set_phase(race, RacePhase::Finished, &mut phase_msg);
    }
}

fn log_messages(
    race: Res<RaceState>,
    mut phases: MessageReader<RacePhaseChanged>,
    mut cps: MessageReader<CheckpointPassed>,
    mut laps: MessageReader<LapCompleted>,
    mut fins: MessageReader<RacerFinished>,
    racers: Query<&Racer>,
) {
    let name = |e: Entity| racers.get(e).map_or("?".to_string(), |r| r.name.clone());
    for m in phases.read() {
        info!("race {}: {:?} (intro {:.1} s, clock {:.2} s)", race.event.id, m.phase, race.intro, race.clock);
    }
    for m in cps.read() {
        info!("race: {} passed checkpoint {} of lap {} at {:.2} s", name(m.racer), m.index, m.lap, m.time);
    }
    for m in laps.read() {
        info!("race: {} completed lap {} in {:.2} s (race {:.2} s)", name(m.racer), m.lap, m.lap_time, m.time);
    }
    for m in fins.read() {
        info!("race: {} finished P{} at {:.2} s (won: {:?})", name(m.racer), m.position, m.time, race.won);
    }
}

#[derive(Component)]
struct RaceText;

fn spawn_text(mut commands: Commands, mut done: Local<bool>) {
    if std::mem::replace(&mut *done, true) {
        return;
    }
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            top: Val::Px(60.0),
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|c| {
            c.spawn((RaceText, GameText::new("", 30.0, Color::WHITE).with_font(TITLE_FONT)));
        });
}

fn update_text(
    race: Res<RaceState>,
    mut text: Query<&mut GameText, With<RaceText>>,
    racers: Query<(&Racer, &RacerProgress)>,
) {
    let Ok(mut t) = text.single_mut() else { return };
    let player = racers.iter().find(|(r, _)| r.player).map(|(_, p)| p);
    let clock = |s: f32| format!("{}:{:05.2}", (s / 60.0) as u32, s % 60.0);
    let n = racers.iter().count().max(1);
    let s = match race.phase {
        RacePhase::Waiting => race.name.clone(),
        RacePhase::Intro => race.countdown_digit().map_or_else(|| race.name.clone(), |d| d.to_string()),
        RacePhase::Racing => {
            let Some(p) = player else { return };
            let warn = if p.missed_checkpoint {
                "   MISSED CHECKPOINT"
            } else if p.wrong_way {
                "   WRONG WAY"
            } else {
                ""
            };
            format!(
                "LAP {}/{}   POS {}/{}   CP {}/{}   {}{warn}",
                p.lap.min(race.laps),
                race.laps,
                p.position,
                n,
                p.passed,
                race.total_gates(),
                clock(race.clock)
            )
        }
        RacePhase::Finished => {
            let Some(p) = player else { return };
            let verdict = if race.won == Some(true) { "WON" } else { "LOST" };
            format!("FINISHED P{} {verdict}   {}", p.position, clock(p.finished.unwrap_or(race.clock)))
        }
    };
    if t.text != s {
        t.text = s;
    }
}

/// `--test race`: once racing, the player's car is carried through its target gate along the gate's normal
/// (kinematically: from 20 m before the gate to 10 m past it at 40 m/s), gate after gate, then the results
/// are printed.
#[derive(Resource, Default)]
struct RaceTest {
    /// Target entry being driven through and the distance along its approach.
    leg: Option<(usize, f32)>,
    done: bool,
    ticks: u32,
    /// The wrong-way check: drive the first leg backwards first.
    reversed: bool,
    saw_wrong_way: bool,
}

const TEST_SPEED: f32 = 40.0;
const TEST_BEFORE: f32 = 20.0;
const TEST_AFTER: f32 = 10.0;

fn run_test(
    time: Res<Time>,
    race: Res<RaceState>,
    mut test: ResMut<RaceTest>,
    mut input: ResMut<CarInput>,
    mut cars: Query<(&mut Transform, &mut LinearVelocity, &mut AngularVelocity, &RacerProgress), With<Car>>,
    mut exit: MessageWriter<AppExit>,
) {
    if test.done {
        return;
    }
    test.ticks += 1;
    let Ok((mut t, mut lin, mut ang, p)) = cars.single_mut() else { return };
    // Give up if the race never starts or finishes (5 min of game time).
    if test.ticks > 60 * 300 {
        error!("TEST RESULT race FAILED: phase {:?} after {} ticks", race.phase, test.ticks);
        test.done = true;
        exit.write(AppExit::error());
        return;
    }
    match race.phase {
        RacePhase::Waiting | RacePhase::Intro => return,
        RacePhase::Finished => {
            test.done = true;
            let laps: Vec<String> = p.lap_times.iter().map(|l| format!("{l:.2}")).collect();
            info!(
                "TEST RESULT race event={} kind={} intro={:.1}s laps={} gates={} passed={} finished={:.2}s position={} won={:?} wrong_way_seen={} average={:.1}mph lap_times=[{}]",
                race.event.id,
                race.event.kind.label(),
                race.intro,
                race.laps,
                race.total_gates(),
                p.passed,
                p.finished.unwrap_or(f32::NAN),
                p.position,
                race.won,
                test.saw_wrong_way,
                p.average_mph(p.finished.unwrap_or(race.clock)),
                laps.join(","),
            );
            exit.write(AppExit::Success);
            return;
        }
        RacePhase::Racing => {}
    }
    input.scripted = Some([0.0; 4]);
    test.saw_wrong_way |= p.wrong_way;
    // First 4 s: drive away from the first gate (a wrong-way check), then the gates.
    if !test.reversed {
        if race.clock < 4.0 {
            let back = -(race.entries[1].position - race.player_start.translation).with_y(0.0).normalize_or(Vec3::Z);
            t.translation += back * 15.0 * time.delta_secs();
            t.rotation = Quat::from_rotation_y(back.x.atan2(back.z));
            lin.0 = Vec3::ZERO;
            ang.0 = Vec3::ZERO;
            return;
        }
        test.reversed = true;
    }
    let (target, s) = match test.leg {
        Some((g, s)) if s < TEST_BEFORE + TEST_AFTER => (g, s),
        _ => (p.target(), 0.0),
    };
    let Some(gate) = race.entries.get(target).and_then(|e| e.gate) else { return };
    let cp = race.checkpoints[gate];
    let normal = cp.normal().with_y(0.0).normalize_or(Vec3::Z);
    let s = s + TEST_SPEED * time.delta_secs();
    t.translation = cp.ground() + Vec3::Y * 0.5 + normal * (s - TEST_BEFORE);
    t.rotation = Quat::from_rotation_y(normal.x.atan2(normal.z));
    lin.0 = Vec3::ZERO;
    ang.0 = Vec3::ZERO;
    test.leg = Some((target, s));
}
