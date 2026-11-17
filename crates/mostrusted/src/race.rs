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
//!   (`FREEZE_CAR`, `FUN_006c1a80`), so the throttle still revs the engine. At 0 the race clock starts. With the
//!   event's intro sequence playing (`cutscene.rs`, the default) there is neither: the offline flow has no
//!   countdown, the intro hands over a rolling start (`RaceState::rolling_start`) and sets the intro to 0.
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
//! - **After the race**: done racers are removed by distance (`retire_racers`, `FUN_00521f00`); the event ends
//!   after the outro and the results (`end_event`) and free roam resumes.
//!
//! Model decisions (no source; decomp.md "Races"): the route segments are road-graph paths cut at the entries
//! (the original asks the AI road network for each, `FUN_006423e0`); the car's overlap box is the
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
//!   ([`RaceState::grid`]: one slot per racing `AIPlayerInformation` in data order, cops left out, with its
//!   `AIPlayerType`, car and colours; `ai.rs` drops the +0xee slot),
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
//! The race HUD, countdown and results are `hud.rs` (the game's layouts); `MW_RACE_TEXT=1` adds a one-line
//! debug text (countdown, lap, position, time).
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
    /// `--test race` for a race started at runtime (`--hsm-event`): the scripted run once it starts.
    pub test_runtime: bool,
}

/// Under `--hsm` the race runs "driven" (logic.rs): started on the logic's respawn at the event start, ended when
/// the logic's mode machine has gone back to free drive. Present only then.
#[derive(Resource, Default)]
pub struct RaceDriven {
    /// The logic went back to free drive: [`end_event`] ends the event now.
    pub end: bool,
}

impl Plugin for RacePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<RacePhaseChanged>()
            .add_message::<CheckpointPassed>()
            .add_message::<LapCompleted>()
            .add_message::<RacerFinished>()
            .add_message::<StartRace>()
            .add_message::<RaceEnded>()
            .init_resource::<RespawnFacing>();
        // The race runs while `RaceState` exists: from `--race` (inserted here) or a runtime [`StartRace`]
        // (Easydrive's RACES tier).
        let racing = resource_exists::<RaceState>;
        app.add_plugins(crate::chevron::ChevronPlugin)
            .add_systems(Update, (start_race, place_at_start).chain())
            .add_systems(Update, (route_segments, spawn_route_markers).run_if(resource_added::<RaceState>))
            .add_systems(Update, (attach_player, log_messages, end_event).run_if(racing))
            .add_systems(Update, retire_racers)
            .add_systems(
                FixedUpdate,
                (hold_at_start.before(crate::drive::car_physics), run_race.after(crate::drive::car_physics))
                    .run_if(racing),
            );
        // The race HUD is `hud.rs` (the game's race layouts); `MW_RACE_TEXT=1` adds the old one-line debug text.
        if std::env::var_os("MW_RACE_TEXT").is_some() {
            app.add_systems(Update, (spawn_text, update_text).run_if(racing));
        }
        if self.test_runtime {
            app.insert_resource(RaceTest { runtime: true, ..default() })
                .add_systems(FixedUpdate, run_test.before(crate::drive::car_physics).before(hold_at_start));
        }
        let Some(setup) = &self.setup else { return };
        app.insert_resource(RaceState::new(setup));
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
    /// Route distance from the start: the sum of the segment polylines' lengths (`FUN_006676b0`).
    pub distance: f32,
}

/// The route from one entry to the next as a polyline (the original's per-segment path through the AI road
/// network, `FUN_006423e0` → `FUN_006676b0`; DTF+0x2778: x, y, z and each piece's length).
#[derive(Debug, Clone, Default)]
pub struct RouteSegment {
    pub points: Vec<Vec3>,
    /// Distance along the polyline at each point.
    cum: Vec<f32>,
}

impl RouteSegment {
    pub fn new(points: Vec<Vec3>) -> Self {
        let mut cum = Vec::with_capacity(points.len());
        let mut d = 0.0;
        for (i, p) in points.iter().enumerate() {
            if i > 0 {
                d += p.distance(points[i - 1]);
            }
            cum.push(d);
        }
        Self { points, cum }
    }

    pub fn length(&self) -> f32 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// The point's distance along the polyline through its nearest piece (`FUN_00516af0`: the first piece with
    /// the smallest 3D distance to the point clamped onto it), unclamped (`FUN_0051e850` / `FUN_0051e7d0` project
    /// onto the piece's direction without clamping), and the clamped projection.
    /// The point at distance `s` along the polyline (extended straight past either end) and the direction there.
    pub fn at(&self, s: f32) -> (Vec3, Vec3) {
        let n = self.points.len();
        if n < 2 {
            return (self.points.first().copied().unwrap_or_default(), Vec3::Z);
        }
        let i = self.cum.partition_point(|&c| c <= s).clamp(1, n - 1) - 1;
        let dir = (self.points[i + 1] - self.points[i]).normalize_or(Vec3::Z);
        (self.points[i] + dir * (s - self.cum[i]), dir)
    }

    fn project(&self, pos: Vec3) -> (f32, Vec3) {
        let mut best = (f32::MAX, 0.0, self.points.first().copied().unwrap_or(pos));
        for i in 0..self.points.len().saturating_sub(1) {
            let (a, b) = (self.points[i], self.points[i + 1]);
            let len = self.cum[i + 1] - self.cum[i];
            if len <= 1e-6 {
                continue;
            }
            let dir = (b - a) / len;
            let t = (pos - a).dot(dir);
            let q = a + dir * t.clamp(0.0, len);
            let d2 = q.distance_squared(pos);
            if d2 < best.0 {
                best = (d2, self.cum[i] + t, q);
            }
        }
        (best.1, best.2)
    }
}

/// An opponent's start slot.
#[derive(Debug, Clone, Copy)]
pub struct GridSlot {
    /// On the trigger point, facing its direction (`FUN_00686f30` / `FUN_0042f250`); spawn with zero velocity.
    pub transform: Transform,
    /// `AIPlayerType` id (GAMEPLAY.BNDL): the driver profile.
    pub ai_type: u32,
    /// The car: `AIPlayerType.Rollout` → `Rollout.Vehicle`, a `VehicleList` id (`FUN_006cc870`).
    pub vehicle: Option<u32>,
    /// The paint: `AIPlayerInformation.2968e77a` (a `ColourId`; 0 = the rollout's `Colour`), and `9b4e35f7`, used
    /// instead when the first is the player's car's colour (`FUN_006ce030`).
    pub colour: u32,
    pub alt_colour: u32,
    /// The event's Most Wanted rival (`AIPlayerType` +0xb4).
    pub most_wanted: bool,
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
    /// `segments[k]`: the route from entry k − 1 to entry k (`segments[0]` is empty). Straight lines until
    /// `route_segments` replaces them with road-graph paths.
    pub segments: Vec<RouteSegment>,
    pub player_start: Transform,
    pub grid: Vec<GridSlot>,
    /// Racers, best first (updated every tick while racing).
    pub standings: Vec<Entity>,
    pub results: Vec<RaceResult>,
    /// The player won (`FUN_006b9d00`'s rule), once finished.
    pub won: Option<bool>,
    /// Hold the start in `Waiting` (intro sequence, AI not spawned yet).
    pub hold: bool,
    /// A runtime start ([`StartRace`]) is placing the player's car: held at the start until the start area is
    /// resident (`FUN_00647e00`, [`place_at_start`]).
    pub placing: bool,
    /// An intro / outro sequence is playing (`cutscene.rs`): the start stays in `Waiting` while it is set, and
    /// `--test race` waits for the outro before it exits. Separate from `hold` (the AI's).
    pub cutscene: bool,
    /// The intro's teleports put the cars on the grid rolling (`TeleportPlayerSequenceItem.Speed_MPH`): the
    /// offline flow has no freeze and no countdown (decomp.md "Cutscenes and sequences").
    pub rolling_start: bool,
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
        // The racers: `FUN_006ce030` files cop types (+0xb5 / +0xb3) in the cop list, not with the opponents.
        let grid = e
            .opponents
            .iter()
            .filter(|o| !o.cop)
            .filter_map(|o| {
                let s = o.start?;
                Some(GridSlot {
                    transform: Transform::from_translation(Vec3::from_array(s.position))
                        .with_rotation(heading_rotation(s.direction)),
                    ai_type: o.ai_type,
                    vehicle: o.rollout.map(|r| r.vehicle).filter(|&v| v != 0),
                    colour: if o.colour != 0 { o.colour } else { o.rollout.map_or(0, |r| r.colour) },
                    alt_colour: o.alt_colour,
                    most_wanted: o.most_wanted,
                })
            })
            .collect();
        let segments = std::iter::once(RouteSegment::default())
            .chain(entries.windows(2).map(|w| RouteSegment::new(vec![w[0].position, w[1].position])))
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
            segments,
            player_start,
            grid,
            standings: Vec::new(),
            results: Vec::new(),
            won: None,
            hold: false,
            placing: false,
            cutscene: false,
            rolling_start: false,
        }
    }

    /// Seconds left in the intro.
    pub fn intro_left(&self) -> f32 {
        (self.intro - self.intro_elapsed).max(0.0)
    }

    /// The original's `IntroTime` (`FUN_006e2670`): whole seconds left plus one; the countdown shows it while
    /// it is 3 or less. `None` outside the intro.
    pub fn countdown_digit(&self) -> Option<u32> {
        (self.phase == RacePhase::Intro && self.intro > 0.0)
            .then(|| (self.intro_left() + 1.0).floor() as u32)
            .filter(|&d| d <= 3)
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

    /// Distance to the finish through route entry `target` (`FUN_00661b40`) and the distance from the car to its
    /// projection on that entry's segment (the term `FUN_0066b3f0` adds for the wrong-way test).
    ///
    /// `FUN_00661b40`: total − cumdist[target] + what is left of the target's segment after the car's projection
    /// (`FUN_0051e850`, clamped to [0, length]). While less than 0.5 m is left (the car is at or past the target)
    /// the distance along the following segment is added (`FUN_0051e7d0`, clamped), and the next one's while the
    /// car is within 0.5 m of that one's end: driving on past a gate makes the distance grow. On the last segment
    /// looked at, a car less than 0.01 m along it and more than 20 m from its start adds the straight distance to
    /// that start.
    pub fn distance_to_finish(&self, target: usize, pos: Vec3) -> (f32, f32) {
        let last = self.entries.len().saturating_sub(1);
        if target == 0 || target > last {
            return (0.0, 0.0);
        }
        let seg = &self.segments[target];
        let (along, projection) = seg.project(pos);
        let mut rem = (seg.length() - along).clamp(0.0, seg.length());
        let mut k = target;
        let mut stop = rem >= REACHED;
        while !stop && k < last {
            k += 1;
            let next = &self.segments[k];
            let along = next.project(pos).0.clamp(0.0, next.length());
            rem += along;
            stop = along < next.length() - REACHED;
        }
        let seg = &self.segments[k];
        if let Some(&first) = seg.points.first()
            && seg.length() - rem < AT_START
            && pos.distance(first) > BEHIND_START
        {
            rem += pos.distance(first);
        }
        (self.total_distance() - self.entries[target].distance + rem, pos.distance(projection))
    }
}

/// `FUN_00661b40`'s thresholds (`.rdata` 0x00fb8290 = 0.5, 0x00fb82f0 = 0.01; 0x00fb8300 = 20 is `BEHIND_START`).
const REACHED: f32 = 0.5;
const AT_START: f32 = 0.01;

/// Replace the straight segments with paths along the AI road graph (`FUN_006423e0` asks the road network for
/// each segment, `FUN_00843240`; `FUN_006676b0` stores the path and adds its length to the cumulative distance):
/// the shortest node path between the nodes nearest the two entries (`AiData::path`), as the Catmull-Rom centre
/// line the AI drives (`ai::spline_through`). Model decision: the path is cut at its points nearest the two
/// entries and ends on the entries themselves (how the original joins its path to the gates is not traced); a
/// segment without a path stays straight.
fn route_segments(game: Res<crate::Game>, mut race: ResMut<RaceState>) {
    let graph = match game.store.ai_data() {
        Ok(g) => g,
        Err(e) => {
            warn!("race: no road graph ({e}); straight route segments");
            return;
        }
    };
    let race = &mut *race;
    let mut cache: std::collections::HashMap<(Option<usize>, Option<usize>), RouteSegment> = Default::default();
    let mut straight = 0;
    for k in 1..race.entries.len() {
        let (a, b) = (race.entries[k - 1], race.entries[k]);
        let seg = cache
            .entry((a.gate, b.gate))
            .or_insert_with(|| {
                road_segment(&graph, a.position, b.position).unwrap_or_else(|| {
                    straight += 1;
                    RouteSegment::new(vec![a.position, b.position])
                })
            })
            .clone();
        race.entries[k].distance = race.entries[k - 1].distance + seg.length();
        race.segments[k] = seg;
    }
    info!(
        "race {}: route {:.0} m along the road graph ({} segments, {straight} straight)",
        race.event.id,
        race.total_distance(),
        race.entries.len() - 1
    );
}

fn road_segment(graph: &mw_formats::AiData, a: Vec3, b: Vec3) -> Option<RouteSegment> {
    let na = graph.nearest_node(a.to_array(), false)?;
    let nb = graph.nearest_node(b.to_array(), false)?;
    let nodes = graph.path(na, nb)?;
    let spline = crate::ai::spline_through(graph, &nodes)?;
    let pts = &spline.points;
    let nearest = |p: Vec3, from: usize| {
        (from..pts.len()).min_by(|&i, &j| pts[i].distance_squared(p).total_cmp(&pts[j].distance_squared(p)))
    };
    let i0 = nearest(a, 0)?;
    let i1 = nearest(b, i0)?;
    let mut points = vec![a];
    points.extend(pts.iter().take(i1).skip(i0 + 1).copied());
    points.push(b);
    Some(RouteSegment::new(points))
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
    /// The event's Most Wanted rival (`AIPlayerType` +0xb4): left alone by the finished-racer removal.
    pub most_wanted: bool,
}

impl Racer {
    pub fn player(half_extents: Vec3) -> Self {
        Self { name: "Player".into(), player: true, grid_slot: None, half_extents, most_wanted: false }
    }

    pub fn ai(name: impl Into<String>, grid_slot: usize, half_extents: Vec3) -> Self {
        Self { name: name.into(), player: false, grid_slot: Some(grid_slot), half_extents, most_wanted: false }
    }

    pub fn rival(mut self, most_wanted: bool) -> Self {
        self.most_wanted = most_wanted;
        self
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
    /// Done racing (player +0x5d7): set when an AI racer reaches its last checkpoint and, on `CHECKPTS` exit
    /// (the player's finish, 0x75d960), on every AI racer still racing; both also get AI goal 3. Done racers are
    /// removed by distance ([`retire_racers`], `FUN_00521f00`).
    pub done: bool,
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

/// The event is over and free roam has resumed ([`end_event`]): `RaceState` is gone from the next frame on.
#[derive(Message, Debug, Clone, Copy)]
pub struct RaceEnded {
    pub event: u32,
    pub won: Option<bool>,
}

/// A race has been run and ended (a second race is refused: the AI and cutscene set-up run once per run).
#[derive(Resource)]
struct RaceOver;

#[derive(Message, Debug, Clone, Copy)]
pub struct RacerFinished {
    pub racer: Entity,
    pub position: usize,
    pub time: f32,
}

/// Start a race while driving (Easydrive's "START RACE"): the player's car is put on the event's start and the
/// race begins as with `--race` (`Waiting` → intro → go). One race per run: the AI sets up its opponents once
/// (`ai.rs`), so a second start is refused while or after a race.
#[derive(Message, Clone)]
pub struct StartRace(pub Arc<RaceSetup>);

#[allow(clippy::type_complexity)]
fn start_race(
    mut commands: Commands,
    mut starts: MessageReader<StartRace>,
    race: Option<Res<RaceState>>,
    over: Option<Res<RaceOver>>,
    mut cars: Query<
        (&mut Transform, Option<&mut LinearVelocity>, Option<&mut AngularVelocity>),
        (With<Car>, With<crate::drive::Player>),
    >,
) {
    let Some(StartRace(setup)) = starts.read().last() else { return };
    if race.is_some() || over.is_some() {
        warn!("race {}: a race was already run; restart the game to run another", setup.event.id);
        return;
    }
    let mut state = RaceState::new(setup);
    state.placing = true;
    info!("race {}: {} started from Easydrive", setup.event.id, setup.name);
    for (mut t, lin, ang) in &mut cars {
        // A little above the start point (the car settles onto the road).
        *t = state.player_start.with_translation(state.player_start.translation + Vec3::Y * 0.5);
        if let Some(mut lin) = lin {
            lin.0 = Vec3::ZERO;
        }
        if let Some(mut ang) = ang {
            ang.0 = Vec3::ZERO;
        }
    }
    commands.insert_resource(state);
}

/// A runtime start puts the player's car at the start the way the game's placement does (`FUN_00647e00`,
/// decomp.md "Start position"): while the start is not resident within 200 m (`frontend::PLACE_RADIUS`; the car is
/// at rest) the car is held there, the streaming focus moves to it (message 4; `streaming::PlacementFocus`) and the
/// loading screen stays up (the event card holds while `placing`); once resident, with the ground under it in, the
/// car is put down and the focus goes back to the camera (message 5).
#[allow(clippy::type_complexity)]
fn place_at_start(
    race: Option<ResMut<RaceState>>,
    residency: Res<crate::frontend::Residency>,
    mut focus: ResMut<crate::streaming::PlacementFocus>,
    spatial: SpatialQuery,
    mut cars: Query<
        (&mut Transform, Option<&mut LinearVelocity>, Option<&mut AngularVelocity>),
        (With<Car>, With<crate::drive::Player>),
    >,
) {
    let Some(mut race) = race else { return };
    if !race.placing {
        return;
    }
    let target = race.player_start.translation;
    let ground = SpatialQueryFilter::default().with_mask(crate::props::GameLayer::World);
    let resident = residency.resident(target, crate::frontend::PLACE_RADIUS)
        && spatial.cast_ray(target + Vec3::Y * 5.0, Dir3::NEG_Y, 40.0, true, &ground).is_some();
    if resident {
        race.placing = false;
        focus.0 = None;
        info!("race {}: start area resident, car placed", race.event.id);
        return;
    }
    focus.0 = Some(target);
    for (mut t, lin, ang) in &mut cars {
        *t = race.player_start.with_translation(target + Vec3::Y * 0.5);
        if let Some(mut lin) = lin {
            lin.0 = Vec3::ZERO;
        }
        if let Some(mut ang) = ang {
            ang.0 = Vec3::ZERO;
        }
    }
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

/// Give the player's car (each new one after a swap too) its `Racer`. A body shop's respray keeps the car
/// entity (`paint_wear::Respray`, as the game's message 0xc paints the same vehicle), so its progress stays.
fn attach_player(mut commands: Commands, cars: Query<(Entity, &Car), (Without<Racer>, With<crate::drive::Player>)>) {
    for (e, car) in &cars {
        commands.entity(e).insert((Racer::player(car.params.body_half_extents), RacerProgress::default()));
    }
}

/// `FREEZE_CAR`: before go the handbrake is held (`FUN_006c1a80`: handbrake = max(input, 1)); the throttle
/// still works.
fn hold_at_start(race: Res<RaceState>, mut input: ResMut<CarInput>) {
    if matches!(race.phase, RacePhase::Waiting | RacePhase::Intro) && !race.rolling_start {
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
            if loaded.done && !players.is_empty() && !race.hold && !race.placing && !race.cutscene {
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
                // `CHECKPTS` (0x75xxxx, decomp.md "Races"): AI goal 3 and +0x5d7 = 1 at the last checkpoint.
                p.done = true;
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
        // FUN_0066b3f0 → FUN_006585b0: d through the target (+ the distance off its segment), d' through the next
        // one; with the finish as the target d' is the plain distance to the finish.
        let d = d_finish + off;
        let d_next = if target < last { race.distance_to_finish(target + 1, pos).0 } else { d_finish };
        let ww = &mut p.ww;
        let rose = ww.last_d.is_some_and(|l| d > l);
        let fell = ww.last_d.is_some_and(|l| d < l);
        let next_fell = ww.last_next.is_some_and(|l| d_next < l);
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
                    if racer.player {
                        info!("race: missed checkpoint (target entry {target}) at {now:.2} s");
                    }
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
        ww.last_next = Some(d_next);
        if racer.player && std::env::var_os("MW_RACE_WW_TRACE").is_some() {
            info!(
                "WW t={now:.2} target={target} d={d:.1} dn={d_next:.1} off={off:.1} state={} timer={:.2} missed={}",
                ww.state, ww.timer, ww.missed
            );
        }
        let flagged = ww.state >= 2;
        let missed = ww.missed;
        p.wrong_way = flagged && !missed;
        // HasMissedCheckpoint only while the target entry is 1..count − 2, i.e. not the finish (`FUN_006584d0`
        // reads DTF+0xd90, the target entry).
        p.missed_checkpoint = flagged && missed && (1..last).contains(&target);
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
        // `CHECKPTS` exit (0x75d960): every AI racer still racing (not a cop, with a car) gets +0x5d7 = 1 and
        // AI goal 3, like one that finished.
        for (_, racer, mut p, _) in &mut racers {
            if !racer.player {
                p.done = true;
            }
        }
        set_phase(race, RacePhase::Finished, &mut phase_msg);
    }
}

/// The finished-racer removal distances when the heat level is unknown: the free-roam rows' `dd335ee5` /
/// `092b3408` (100 / 300 m on all seven; `mwtool police`).
const FINISHED_RACER_REMOVE: [f32; 2] = [100.0, 300.0];
/// The blacklist shutdown behaviour releases the Most Wanted rival beyond 300 m (`FUN_00754290` state 6 and
/// `FUN_007428b0`: squared distance against `.rdata` 0x00fd2108 = 90000).
const RIVAL_RELEASE: f32 = 300.0;

/// `FUN_00521f00` (run by the AI coordinator `FUN_00526c30` every frame while the game mode's timer runs: in a
/// race from go to the player's finish, and in free roam, whose "Freedrive Update" starts the timer): a race car
/// that is done racing (+0x5d7), has its car, is not being reset (`FUN_006b8840`) and is not the Most Wanted rival
/// is removed (message 0xe → `FUN_006ce180(slot, 3)`) when its horizontal distance from the player is beyond the
/// heat level's `dd335ee5` (+0xb0) and it is off screen (`FUN_006bf380`: a 4 m sphere against the camera's
/// frustum), or beyond `092b3408` (+0xb4) in any case. The rival is left to the blacklist behaviour, which
/// releases it beyond 300 m (its takedown phase is not modelled).
#[allow(clippy::type_complexity)]
fn retire_racers(
    mut commands: Commands,
    race: Option<Res<RaceState>>,
    police: Option<Res<crate::police::Police>>,
    frustum: Query<&bevy::camera::primitives::Frustum, crate::MainCamera>,
    player: Query<&Transform, (With<Car>, With<crate::drive::Player>)>,
    racers: Query<
        (Entity, &Racer, &RacerProgress, &Transform),
        (Without<crate::drive::Player>, Without<crate::car_impacts::AiCrash>),
    >,
) {
    if race.as_ref().is_some_and(|r| r.phase != RacePhase::Racing) {
        return;
    }
    let Ok(me) = player.single() else { return };
    let [off_screen, always] =
        police.as_ref().and_then(|p| p.heat_level()).map_or(FINISHED_RACER_REMOVE, |l| l.finished_racer_remove);
    let frustum = frustum.iter().next();
    let flat = |v: Vec3| Vec2::new(v.x, v.z);
    for (e, racer, p, t) in &racers {
        if racer.player || !p.done {
            continue;
        }
        let d = flat(t.translation).distance(flat(me.translation));
        let remove = if racer.most_wanted {
            d > RIVAL_RELEASE
        } else {
            let seen = frustum.is_some_and(|f| {
                f.intersects_sphere(
                    &bevy::camera::primitives::Sphere { center: t.translation.into(), radius: 4.0 },
                    true,
                )
            });
            d > always || (d > off_screen && !seen)
        };
        if remove {
            info!(
                "race: {} removed {d:.0} m from the player{}",
                racer.name,
                if d > always { "" } else { ", off screen" }
            );
            commands.entity(e).despawn();
        }
    }
}

/// The results' length (s): the results sequence's items run 0.25–8.25 s (`hud::RESULTS_SEQUENCE`).
pub const RESULTS_SECS: f32 = 8.25;

/// The event's end: once the outro and the results are over (the HSM's "Run PageRunner2 Context - Outros" →
/// "Race Game Mode" exit → "On Event End"), the race is torn down and free roam resumes ("Freedrive Update"): the
/// race state goes (so free-roam rules such as the jack spots, collectables and the free-roam HUD apply again), the
/// player's car is no longer a racer and the route markers go. The opponents stay until [`retire_racers`] removes
/// them. Model decision: the end is the results sequence's last item (the offline results are the front end's
/// `ArcResults` screen, which the player closes). Driven ([`RaceDriven`], `--hsm`): when the logic says so.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn end_event(
    mut commands: Commands,
    race: Res<RaceState>,
    driven: Option<ResMut<RaceDriven>>,
    hud: Option<Res<crate::hud::HudState>>,
    mut facing: ResMut<RespawnFacing>,
    players: Query<Entity, (With<Racer>, With<crate::drive::Player>)>,
    markers: Query<Entity, With<crate::chevron::RouteMarker>>,
    mut ended: MessageWriter<RaceEnded>,
) {
    match driven {
        Some(mut d) => {
            if !std::mem::take(&mut d.end) {
                return;
            }
        }
        None => {
            if race.phase != RacePhase::Finished || race.cutscene {
                return;
            }
            if hud.as_ref().is_some_and(|h| h.results_clock.is_none_or(|t| t < RESULTS_SECS)) {
                return;
            }
        }
    }
    info!("race {}: event over (won: {:?}), back to free roam", race.event.id, race.won);
    commands.remove_resource::<RaceState>();
    commands.insert_resource(RaceOver);
    facing.0 = None;
    for e in &players {
        commands.entity(e).remove::<(Racer, RacerProgress)>();
    }
    for e in &markers {
        commands.entity(e).despawn();
    }
    ended.write(RaceEnded { event: race.event.id, won: race.won });
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

/// `--test race`: once racing, the player's car first backs away from its target for 4 s (the wrong-way check),
/// then is carried along the route's segments (kinematically, at 40 m/s) through every gate; then the results
/// are printed.
///
/// - `MW_RACE_TEST_MISS=<k>`: on the way to the k-th gate (route entry k, 1-based) the car skips it and drives
///   through the next one first (from 60 m before it), then comes back for the skipped one: `missed_at_skip`
///   says whether the HUD's missed-checkpoint flag came up on that leg, `missed_seen` covers the rest.
/// - `MW_RACE_TEST_AFTER=<s>`: after the results, wait for the event's end, hold the car still for `s` seconds and
///   print `TEST RESULT race after ... racers_left=N` (the opponents still in the world).
/// - `MW_RACE_TEST_JACK=1`: then the jack-spot test (`jackspots.rs`) swaps cars at the nearest spot and exits.
/// - `MW_RACE_TEST_HOLD=<s>`: keep running this long after the result (to screenshot the results).
#[derive(Resource, Default)]
struct RaceTest {
    /// Target entry being driven through and the distance along its approach.
    leg: Option<(usize, f32)>,
    done: bool,
    ticks: u32,
    /// The wrong-way check: drive the first leg backwards first.
    reversed: bool,
    saw_wrong_way: bool,
    /// The missed-checkpoint flag outside the deliberate miss.
    saw_missed: bool,
    /// `MW_RACE_TEST_MISS`: the entry to skip, the leg through the next one (distance along it), and whether the
    /// flag came up meanwhile.
    miss: Option<usize>,
    skip_leg: Option<f32>,
    skipped: Option<usize>,
    /// Where the forward run starts on the target's segment (the car's projection after the wrong-way check).
    from: f32,
    /// After the skip leg, until the flag clears: still the miss's.
    skip_tail: bool,
    missed_at_skip: bool,
    /// `MW_RACE_TEST_AFTER`: seconds to wait after the event's end, and the time waited so far.
    after: Option<f32>,
    waited: Option<f32>,
    held_at: Option<Vec3>,
    exit_at: Option<u32>,
    /// The race is started at runtime (`--hsm-event`): the 4× speed-up begins with it, and the result waits for
    /// the event's end (the logic's way back to free roam).
    runtime: bool,
    sped_up: bool,
}

const TEST_SPEED: f32 = 40.0;
const TEST_AFTER: f32 = 10.0;
/// The skipped gate's leg starts this far before the next gate: more than a second at `TEST_SPEED` (the
/// missed-checkpoint test needs d rising for over `MISSED_AFTER`).
const TEST_SKIP_BEFORE: f32 = 60.0;

fn env_f32(name: &str) -> Option<f32> {
    std::env::var(name).ok().and_then(|s| s.parse().ok())
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn run_test(
    time: Res<Time>,
    mut speed: ResMut<Time<Virtual>>,
    race: Option<Res<RaceState>>,
    mut test: ResMut<RaceTest>,
    mut input: ResMut<CarInput>,
    mut cars: Query<
        (&mut Transform, &mut LinearVelocity, &mut AngularVelocity, Option<&RacerProgress>),
        (With<Car>, With<crate::drive::Player>),
    >,
    opponents: Query<&Racer, Without<crate::drive::Player>>,
    mut exit: MessageWriter<AppExit>,
) {
    if test.runtime && !test.done && test.waited.is_none() {
        // A race started at runtime: nothing to do until it exists; then as `--race --test race` (4× speed).
        if race.is_none() {
            return;
        }
        if !std::mem::replace(&mut test.sped_up, true) {
            speed.set_relative_speed(4.0);
        }
    }
    test.ticks += 1;
    if test.done {
        if test.exit_at.is_some_and(|t| test.ticks >= t) {
            test.exit_at = None;
            exit.write(AppExit::Success);
        }
        return;
    }
    if test.ticks == 1 {
        test.miss = env_f32("MW_RACE_TEST_MISS").map(|k| k as usize);
        test.after = env_f32("MW_RACE_TEST_AFTER");
        if test.runtime {
            // The result waits for the event's end (the way back to free roam).
            test.after.get_or_insert(1.0);
        }
    }
    let Ok((mut t, mut lin, mut ang, p)) = cars.single_mut() else { return };
    // After the results: the event's end, then the opponents left.
    if let Some(waited) = test.waited {
        if race.is_some() {
            return;
        }
        let held = *test.held_at.get_or_insert(t.translation);
        t.translation = held;
        lin.0 = Vec3::ZERO;
        ang.0 = Vec3::ZERO;
        let waited = waited + time.delta_secs();
        test.waited = Some(waited);
        if waited >= test.after.unwrap_or(0.0) {
            info!(
                "TEST RESULT race after event_over=true waited={waited:.1}s racers_left={}",
                opponents.iter().count()
            );
            test.done = true;
            input.scripted = None;
            if std::env::var_os("MW_RACE_TEST_JACK").is_none() {
                exit.write(AppExit::Success);
            }
        }
        return;
    }
    let Some(race) = race else { return };
    // Give up if the race never starts or finishes (5 min of game time).
    if test.ticks > 60 * 300 {
        error!("TEST RESULT race FAILED: phase {:?} after {} ticks", race.phase, test.ticks);
        test.done = true;
        exit.write(AppExit::error());
        return;
    }
    let Some(p) = p else { return };
    match race.phase {
        RacePhase::Waiting | RacePhase::Intro => return,
        // The outro sequence plays to its end first (`cutscene.rs`).
        RacePhase::Finished if race.cutscene => {}
        RacePhase::Finished => {
            let laps: Vec<String> = p.lap_times.iter().map(|l| format!("{l:.2}")).collect();
            let skip = test
                .skipped
                .map_or(String::new(), |k| format!(" skipped_entry={k} missed_at_skip={}", test.missed_at_skip));
            info!(
                "TEST RESULT race event={} kind={} intro={:.1}s laps={} gates={} passed={} finished={:.2}s position={} won={:?} wrong_way_seen={} missed_seen={}{skip} average={:.1}mph lap_times=[{}]",
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
                test.saw_missed,
                p.average_mph(p.finished.unwrap_or(race.clock)),
                laps.join(","),
            );
            input.scripted = Some([0.0; 4]);
            if test.after.is_some() {
                test.waited = Some(0.0);
                return;
            }
            test.done = true;
            match env_f32("MW_RACE_TEST_HOLD") {
                Some(hold) => test.exit_at = Some(test.ticks + (hold * 60.0) as u32),
                None => {
                    exit.write(AppExit::Success);
                }
            }
            return;
        }
        RacePhase::Racing => {}
    }
    input.scripted = Some([0.0; 4]);
    test.saw_wrong_way |= p.wrong_way;
    if test.skip_leg.is_some() || test.skip_tail {
        test.missed_at_skip |= p.missed_checkpoint;
        test.skip_tail &= p.missed_checkpoint || test.skip_leg.is_some();
    } else {
        test.saw_missed |= p.missed_checkpoint;
    }
    // First 4 s: drive away from the first gate (a wrong-way check), then the gates.
    if !test.reversed {
        if race.clock < 4.0 {
            // Away from the racer's next target from where the car is: after an intro's rolling start the car
            // is no longer on `player_start`.
            let target = race.entries.get(p.target()).map_or(race.entries[1].position, |e| e.position);
            let back = -(target - t.translation).with_y(0.0).normalize_or(Vec3::Z);
            t.translation += back * 15.0 * time.delta_secs();
            t.rotation = Quat::from_rotation_y(back.x.atan2(back.z));
            lin.0 = Vec3::ZERO;
            ang.0 = Vec3::ZERO;
            return;
        }
        test.reversed = true;
    }
    let step = TEST_SPEED * time.delta_secs();
    let last = race.total_gates();
    if test.leg.is_none() {
        let seg = &race.segments[p.target()];
        test.from = seg.project(t.translation).0.clamp(0.0, seg.length());
    }
    let mut place = |(point, dir): (Vec3, Vec3)| {
        *t =
            Transform::from_translation(point + Vec3::Y * 0.5).with_rotation(Quat::from_rotation_y(dir.x.atan2(dir.z)));
        lin.0 = Vec3::ZERO;
        ang.0 = Vec3::ZERO;
    };
    // The deliberate miss: the next segment from `TEST_SKIP_BEFORE` before its end (the gate after the skipped
    // one) to `TEST_AFTER` past it, then back to the start of the skipped gate's segment.
    if test.miss == Some(p.target()) && p.target() < last {
        let seg = &race.segments[p.target() + 1];
        let s = test.skip_leg.unwrap_or(seg.length() - TEST_SKIP_BEFORE) + step;
        place(seg.at(s));
        if s < seg.length() + TEST_AFTER {
            test.skip_leg = Some(s);
        } else {
            test.skip_leg = None;
            test.skip_tail = true;
            test.skipped = test.miss.take();
            test.leg = Some((p.target(), 0.0));
        }
        return;
    }
    // Along the route: the target's segment from where the car is, then the next ones.
    let (mut k, mut s) = test.leg.unwrap_or((p.target(), test.from));
    s += step;
    while s > race.segments[k].length() && k < last {
        s -= race.segments[k].length();
        k += 1;
    }
    place(race.segments[k].at(s));
    test.leg = Some((k, s));
}
