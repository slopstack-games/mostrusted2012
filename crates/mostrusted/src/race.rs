//! Races: one offline event (`mw_assets::events`) run from the start to the result.
//!
//! `--race <event id>` (ids from `mwtool events`) loads the event, puts the player's car on its `SpawnPosition`
//! (with the event's first allowed car unless `--drive` names one), and runs
//! [`RacePhase::Waiting`] → [`RacePhase::Countdown`] → [`RacePhase::Racing`] → [`RacePhase::Finished`].
//! What the original does is in docs/decomp.md ("Races"); the rules below name their source, and the ones
//! without one are listed as model decisions there and marked `Model decision` here.
//!
//! # API for the AI, HUD and cutscene code
//!
//! - [`RaceState`] (resource, present while a race is loaded): the event, the phase, the countdown and race
//!   clocks, the checkpoint gates ([`RaceCheckpoint`]: transform, size, normal), the route as a polyline
//!   ([`RaceState::route`]: start, then every gate centre in driving order; one lap for circuits), the
//!   player's start ([`RaceState::player_start`]) and the opponents' grid ([`RaceState::grid`]: one transform
//!   per `AIPlayerInformation` slot, in data order, with the slot's `AIPlayerType` id), the live standings
//!   ([`RaceState::standings`], racer entities best first) and the final results.
//! - [`Racer`] (component): put it on **any** entity with a `Transform` to make it race. The player's car gets
//!   one automatically; an AI car spawned on `RaceState::grid[i]` should get `Racer::ai(name, i)`. Progress is
//!   tracked in [`RacerProgress`] (inserted by the race): lap, next checkpoint, checkpoints passed, distance to
//!   the next gate, lap times, finish time and position.
//! - Messages: [`RacePhaseChanged`] (every phase change: the intro cutscene can hold [`RaceState::hold`] to delay
//!   the countdown), [`CheckpointPassed`], [`LapCompleted`], [`RacerFinished`].
//! - [`RaceState::hold`]: while true the race stays in `Waiting` (for an intro sequence or the AI's spawn).
//!
//! Minimal text (countdown, lap, position, time) is drawn with `game_text` until the HUD wave replaces it.
//! `--test race` drives the player's car through every gate kinematically (no driving skill involved), prints
//! `TEST RESULT race ...` and exits: it checks the race logic, not the handling.

// Parts of the API (grid slots, results, the route) are for the AI / HUD / cutscene code of the next wave.
#![allow(dead_code)]

use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;
use mw_assets::events::{Event, EventKind};

use crate::drive::{Car, CarInput};
use crate::game_text::{GameText, TITLE_FONT};

/// What `--race` hands to the plugin.
pub struct RaceSetup {
    pub event: Event,
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
            .add_message::<RacerFinished>();
        let Some(setup) = &self.setup else { return };
        app.insert_resource(RaceState::new(setup))
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

/// Model decision (until the countdown's length is traced): 3 s, the "3, 2, 1" of the original's start.
pub const COUNTDOWN_SECS: f32 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RacePhase {
    /// Loaded, waiting for the world, the player's car and anything holding the start ([`RaceState::hold`]).
    Waiting,
    /// The countdown runs ([`RaceState::countdown`] seconds left); cars are held.
    Countdown,
    /// Go: the race clock runs.
    Racing,
    /// The player has finished (or every racer has).
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

    /// Whether the segment `a` → `b` touches the gate box (slab test in the box's frame), so a fast car
    /// cannot step over the 2 m deep box between two ticks.
    pub fn crossed(&self, a: Vec3, b: Vec3) -> bool {
        let inv = self.transform.rotation.inverse();
        let (la, lb) = (inv * (a - self.transform.translation), inv * (b - self.transform.translation));
        let half = self.size * 0.5;
        let d = lb - la;
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for i in 0..3 {
            if d[i].abs() < 1e-6 {
                if la[i].abs() > half[i] {
                    return false;
                }
                continue;
            }
            let (mut ta, mut tb) = ((-half[i] - la[i]) / d[i], (half[i] - la[i]) / d[i]);
            if ta > tb {
                std::mem::swap(&mut ta, &mut tb);
            }
            t0 = t0.max(ta);
            t1 = t1.min(tb);
            if t0 > t1 {
                return false;
            }
        }
        true
    }
}

/// An opponent's start slot.
#[derive(Debug, Clone, Copy)]
pub struct GridSlot {
    /// On the trigger point (its height is the road's, give or take a few cm: snap to the ground).
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
    pub position: usize,
    /// Race time (s) at the finish.
    pub time: f32,
    pub lap_times: Vec<f32>,
    pub best_lap: Option<f32>,
}

#[derive(Resource)]
pub struct RaceState {
    pub event: Event,
    pub name: String,
    pub phase: RacePhase,
    /// Seconds left in the countdown.
    pub countdown: f32,
    /// Race time since go (s).
    pub clock: f32,
    /// Laps to drive (1 for sprints).
    pub laps: u8,
    pub checkpoints: Vec<RaceCheckpoint>,
    pub player_start: Transform,
    pub grid: Vec<GridSlot>,
    /// Racers, best first (updated every tick while racing).
    pub standings: Vec<Entity>,
    pub results: Vec<RaceResult>,
    /// Hold the start in `Waiting` (intro sequence, AI not spawned yet).
    pub hold: bool,
}

fn heading_rotation(direction: [f32; 3]) -> Quat {
    Quat::from_rotation_y(direction[0].atan2(direction[2]))
}

impl RaceState {
    fn new(setup: &RaceSetup) -> Self {
        let e = &setup.event;
        let checkpoints = e
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
            name: setup.name.clone(),
            phase: RacePhase::Waiting,
            countdown: COUNTDOWN_SECS,
            clock: 0.0,
            laps: e.laps.max(1),
            checkpoints,
            player_start,
            grid,
            standings: Vec::new(),
            results: Vec::new(),
            hold: false,
        }
    }

    /// The route: the start, then every gate centre in driving order (one lap).
    pub fn route(&self) -> Vec<Vec3> {
        std::iter::once(self.player_start.translation)
            .chain(self.checkpoints.iter().map(|c| c.transform.translation))
            .collect()
    }

    /// Gates per lap. For a circuit the last gate is the start / finish line; for a sprint the finish.
    pub fn gates_per_lap(&self) -> usize {
        self.checkpoints.len()
    }

    /// Gates to pass to finish.
    pub fn total_gates(&self) -> usize {
        self.checkpoints.len() * self.laps as usize
    }

    pub fn is_circuit(&self) -> bool {
        self.event.kind == EventKind::Circuit || self.laps > 1
    }
}

/// Marks an entity as a racer. Any entity with a `Transform` can race.
#[derive(Component, Debug, Clone)]
pub struct Racer {
    pub name: String,
    pub player: bool,
    /// Index into [`RaceState::grid`] for opponents.
    pub grid_slot: Option<usize>,
}

impl Racer {
    pub fn player() -> Self {
        Self { name: "Player".into(), player: true, grid_slot: None }
    }

    pub fn ai(name: impl Into<String>, grid_slot: usize) -> Self {
        Self { name: name.into(), player: false, grid_slot: Some(grid_slot) }
    }
}

/// A racer's progress (inserted by the race on every [`Racer`]).
#[derive(Component, Debug, Clone, Default)]
pub struct RacerProgress {
    /// Current lap, 1-based (0 before the start).
    pub lap: u8,
    /// Index of the next gate within the lap.
    pub next: usize,
    /// Gates passed since the start (all laps).
    pub passed: usize,
    /// Straight-line distance to the next gate's centre (m).
    pub to_next: f32,
    /// Race time when the current lap started, and the finished laps' times.
    pub lap_start: f32,
    pub lap_times: Vec<f32>,
    /// Race time at the finish.
    pub finished: Option<f32>,
    /// 1-based position in the standings.
    pub position: usize,
    /// Moving away from the next gate (model decision; see `run_race`).
    pub wrong_way: bool,
    last: Option<Vec3>,
}

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

/// Give the player's car (each new one after a swap too) its `Racer`.
fn attach_player(mut commands: Commands, cars: Query<Entity, (With<Car>, Without<Racer>)>) {
    for e in &cars {
        commands.entity(e).insert((Racer::player(), RacerProgress::default()));
    }
}

/// Before go: the cars stay on the line. Model decision (until the start hold is traced): brakes full on,
/// throttle passed through so the engine can be revved.
fn hold_at_start(race: Res<RaceState>, mut input: ResMut<CarInput>) {
    if matches!(race.phase, RacePhase::Waiting | RacePhase::Countdown) {
        input.brake = 1.0;
        input.handbrake = true;
    }
}

#[allow(clippy::too_many_arguments)]
fn run_race(
    time: Res<Time>,
    loaded: Res<crate::Loaded>,
    mut race: ResMut<RaceState>,
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
                race.countdown = COUNTDOWN_SECS;
                set_phase(race, RacePhase::Countdown, &mut phase_msg);
            }
            return;
        }
        RacePhase::Countdown => {
            race.countdown -= dt;
            if race.countdown > 0.0 {
                return;
            }
            race.countdown = 0.0;
            race.clock = 0.0;
            for (_, _, mut p, t) in &mut racers {
                *p = RacerProgress { lap: 1, last: Some(t.translation), ..default() };
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
    if n == 0 {
        return;
    }
    let now = race.clock;
    for (entity, racer, mut p, t) in &mut racers {
        if p.lap == 0 {
            // Joined after the start (a swapped car): continue from its position.
            *p = RacerProgress { lap: 1, last: Some(t.translation), ..default() };
        }
        let pos = t.translation;
        let last = p.last.replace(pos).unwrap_or(pos);
        if p.finished.is_some() {
            continue;
        }
        // Only the next gate counts, in order (model decision until the checkpoint rule is traced).
        let gate = race.checkpoints[p.next];
        if gate.crossed(last, pos) {
            let index = p.next;
            p.passed += 1;
            cp_msg.write(CheckpointPassed { racer: entity, index, lap: p.lap, time: now });
            p.next = (p.next + 1) % n;
            if index + 1 == n {
                let lap_time = now - p.lap_start;
                p.lap_times.push(lap_time);
                lap_msg.write(LapCompleted { racer: entity, lap: p.lap, lap_time, time: now });
                p.lap_start = now;
                if p.lap >= race.laps {
                    p.finished = Some(now);
                    let position = race.results.len() + 1;
                    p.position = position;
                    let best_lap = p.lap_times.iter().copied().reduce(f32::min);
                    race.results.push(RaceResult {
                        racer: entity,
                        name: racer.name.clone(),
                        position,
                        time: now,
                        lap_times: p.lap_times.clone(),
                        best_lap,
                    });
                    fin_msg.write(RacerFinished { racer: entity, position, time: now });
                } else {
                    p.lap += 1;
                }
            }
        }
        let next = race.checkpoints[p.next].transform.translation;
        let to_next = pos.distance(next);
        // Model decision: wrong way = the distance to the next gate grows while moving (> 1 m/s).
        let rate = (to_next - p.to_next) / dt.max(1e-6);
        p.wrong_way = p.to_next > 0.0 && rate > 1.0;
        p.to_next = to_next;
    }
    // Standings: finished racers by finish time, then gates passed, then the distance to the next gate
    // (model decision until the game's ranking is traced).
    let mut order: Vec<(Entity, Option<f32>, usize, f32)> =
        racers.iter().map(|(e, _, p, _)| (e, p.finished, p.passed, p.to_next)).collect();
    order.sort_by(|a, b| match (a.1, b.1) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => b.2.cmp(&a.2).then(a.3.total_cmp(&b.3)),
    });
    race.standings = order.iter().map(|o| o.0).collect();
    for (i, e) in race.standings.iter().enumerate() {
        if let Ok((_, _, mut p, _)) = racers.get_mut(*e)
            && p.finished.is_none()
        {
            p.position = i + 1;
        }
    }
    // The race ends for the player when the player finishes (model decision until the finish is traced).
    let player_done = racers.iter().any(|(_, r, p, _)| r.player && p.finished.is_some());
    if player_done {
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
        info!("race {}: {:?} (clock {:.2} s)", race.event.id, m.phase, race.clock);
    }
    for m in cps.read() {
        info!("race: {} passed checkpoint {} of lap {} at {:.2} s", name(m.racer), m.index, m.lap, m.time);
    }
    for m in laps.read() {
        info!("race: {} completed lap {} in {:.2} s (race {:.2} s)", name(m.racer), m.lap, m.lap_time, m.time);
    }
    for m in fins.read() {
        info!("race: {} finished P{} at {:.2} s", name(m.racer), m.position, m.time);
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
    let n = race.standings.len().max(1);
    let s = match race.phase {
        RacePhase::Waiting => race.name.clone(),
        RacePhase::Countdown => format!("{}", race.countdown.ceil() as u32),
        RacePhase::Racing => {
            let Some(p) = player else { return };
            let wrong = if p.wrong_way { "   WRONG WAY" } else { "" };
            format!(
                "LAP {}/{}   POS {}/{}   CP {}/{}   {}{wrong}",
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
            format!("FINISHED P{}   {}", p.position, clock(p.finished.unwrap_or(race.clock)))
        }
    };
    if t.text != s {
        t.text = s;
    }
}

/// `--test race`: once racing, the player's car is carried through every gate in turn along its normal
/// (kinematically: 20 m before the gate to 10 m past it at 40 m/s), then the results are printed.
#[derive(Resource, Default)]
struct RaceTest {
    /// Gate being driven through and the distance along its approach.
    leg: Option<(usize, f32)>,
    gates: usize,
    done: bool,
    ticks: u32,
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
        RacePhase::Waiting | RacePhase::Countdown => return,
        RacePhase::Finished => {
            test.done = true;
            let r = race.results.iter().find(|r| r.racer == race.standings.first().copied().unwrap_or(r.racer));
            let laps: Vec<String> = p.lap_times.iter().map(|l| format!("{l:.2}")).collect();
            info!(
                "TEST RESULT race event={} kind={} laps={} gates={} passed={} finished={:.2}s position={} lap_times=[{}] results={}",
                race.event.id,
                race.event.kind.label(),
                race.laps,
                race.total_gates(),
                p.passed,
                p.finished.unwrap_or(f32::NAN),
                p.position,
                laps.join(","),
                r.map_or(0, |_| race.results.len()),
            );
            exit.write(AppExit::Success);
            return;
        }
        RacePhase::Racing => {}
    }
    input.scripted = Some([0.0; 4]);
    let n = race.gates_per_lap();
    let (gate, s) = match test.leg {
        Some((g, s)) if s < TEST_BEFORE + TEST_AFTER => (g, s),
        _ => {
            test.gates += 1;
            (p.next, 0.0)
        }
    };
    let cp = race.checkpoints[gate % n];
    let normal = Vec3::new(cp.normal().x, 0.0, cp.normal().z).normalize_or(Vec3::Z);
    // Road height: the gate box stands on the road (its centre is half its height up).
    let base = cp.transform.translation - Vec3::Y * (cp.size.y * 0.5 - 0.5);
    let s = s + TEST_SPEED * time.delta_secs();
    t.translation = base + normal * (s - TEST_BEFORE);
    t.rotation = Quat::from_rotation_y(normal.x.atan2(normal.z));
    lin.0 = Vec3::ZERO;
    ang.0 = Vec3::ZERO;
    test.leg = Some((gate, s));
}
