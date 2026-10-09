//! Traffic at runtime: civilian cars that are spawned, follow the lanes and are removed the way the original's
//! traffic system does it (docs/decomp.md "Traffic and AI vehicles", "Traffic rules", "Traffic runtime"). Data:
//! `mw_assets::traffic` (lanes, flows, vehicle types, `TrafficSystemConfig` / `TrafficParamConfig` /
//! `TrafficFuzzyLogicConfig`).
//!
//! Per tick, in the order of `FUN_0060eab0`:
//! 1. **Active area** (`FUN_005f0270`, `FUN_0060c9f0`): the hull cell the player is in plus its neighbour list.
//!    Newly active hulls are filled with cars along their lanes (`FUN_0060db90` → `FUN_0060ce20` →
//!    `FUN_0060afb0`); cars whose hull left the area are removed.
//! 2. **Density multiplier** (`FUN_005f0780`): `sys[0x80]` (4.78) × the player-speed ramp (×1.2 at ≤ 20 mph).
//! 3. **Decisions** (`FUN_005fc220` → `FUN_005ea6a0` / `FUN_00617d00`): the held stop line ahead
//!    (`FUN_005f0d90`), the car ahead (`FUN_005f1380`), the nearest obstacle (the player, crashed traffic) in a
//!    45° cone up to 50 m (`FUN_00511540`), scored with the fuzzy envelopes; the best option sets a mode, a
//!    target speed and a distance to reach it in.
//! 4. **Emitters** (`FUN_0061c780`, `FUN_0060e5e0`, `FUN_005fb120`): lane starts at the area's edge spawn a car
//!    every `60 / (m · Density)` s while the load is below 200, unless the last car on the lane is closer than
//!    2 s at lane speed; no car is placed within 50 m of the player (`FUN_00608720`).
//! 5. **Motion** (`FUN_00614c20`, `FUN_0061a190`, `FUN_0061a450`, `FUN_0061a550`): the speed controller, the
//!    advance along the rungs, the route through junctions (least loaded span, `FUN_00618470`, planned 25 m
//!    ahead by `FUN_005fc020`); a car leaving the area or the network is removed.
//! 6. **Traffic lights** (`FUN_00605d30`): each junction cycles its phases; a phase's mask holds stop lines.
//! 7. **Load budget** (`FUN_005fd6a0`): cars within 60 m count 30 (crashed) / 20 (moving) / 5 (slower than
//!    6 m/s); at 200 or more the farthest is removed every 0.5 s.
//!
//! Lane followers are kinematic bodies (moved by velocity to the lane pose). A dynamic body touching one (the
//! player's car, a crashed traffic car) turns it into a dynamic rigid body with the car's mass and its lane
//! velocity ([`TrafficBody`], the contact hook in `props.rs`); from then on it is "crashing traffic".
//!
//! Model decisions (not from the code; docs/decomp.md "Traffic runtime"): see the `MODEL:` comments.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use avian3d::prelude::*;
use bevy::prelude::*;
use mw_assets::traffic::{MPH, TrafficData};
use mw_bundle::ResourceId;

use crate::loader::{CarRequest, LoaderRequests, TrafficCarSpec, TrafficModelRequest, WheelSpec};
use crate::materials::GameMaterial;

/// Lane-follower slots (`FUN_006024f0(first, 0x100)`; the world's vehicle table keeps slots 0–255 for traffic).
const MAX_CARS: usize = 256;
/// Spawning stops at this load (`DAT_00fb1ab0`).
const LOAD_BUDGET: f32 = 200.0;
/// The load counts cars within this distance: `sqrt(DAT_0110f590)`, 3600 read live (research log).
const LOAD_RADIUS: f32 = 60.0;
/// Look-ahead factor of the leader scan (`DAT_00fb1bb0`).
const LOOK_FACTOR: f32 = 1.5;
/// Corner check: the lane direction `3 v + 10` m ahead (`DAT_00fb2114`, `DAT_00fb1aa4`) against the car's
/// heading; at or below cos = 0.85 (`DAT_00fb2888`) the cruise speed is capped at 5 m/s (`DAT_00fb278c`).
const CORNER_LOOK_TIME: f32 = 3.0;
const CORNER_LOOK_BASE: f32 = 10.0;
const CORNER_COS: f32 = 0.85;
const CORNER_SPEED: f32 = 5.0;
/// The cruise controller's target cap (`DAT_00fab690`).
const CRUISE_CAP: f32 = 500.0;
/// Brake lights: on below −0.6 m/s² (`DAT_00fa3010`) or when slower than 2.5 m/s (`DAT_00fa3020`) without
/// accelerating (`FUN_0061a450`).
const BRAKE_ACCEL: f32 = -0.6;
const BRAKE_SPEED: f32 = 2.5;
/// Waiting time only grows below this speed (`DAT_00fb278c`, `FUN_00614c20`).
const WAIT_SPEED: f32 = 5.0;
/// Obstacle cone (`FUN_005fc220` → `FUN_00511540`): cos 45° (`DAT_00fb2410`), up to 50 m (`DAT_00fb23a0`)
/// ahead, heights scaled by 2.5 (`DAT_00fb2420`); the local player weighs 0.5, crashed traffic 0.5.
// The game's constant is 0.7071 (0x3f350481), not the float nearest 1/√2.
#[allow(clippy::approx_constant)]
const CONE_COS: f32 = 0.7071;
const CONE_RANGE: f32 = 50.0;
const CONE_HEIGHT_SCALE: f32 = 2.5;
const PLAYER_WEIGHT: f32 = 0.5;
const CRASHED_WEIGHT: f32 = 0.5;
/// Heading smoothing per 60 Hz tick (`FUN_006132d0`, `DAT_010c4d20` = 0.4).
const HEADING_BLEND: f32 = 0.4;
/// Hysteresis of the option choice (`DAT_00fb2390`).
const SCORE_HYSTERESIS: f32 = 0.05;
/// A trailer lengthens the back extent (`DAT_00fb23c0`).
const TRAILER_LENGTH: f32 = 10.0;
/// The runtime cruise-speed scale `world+0xa2c00` (1.0 read live in free roam).
const SPEED_SCALE: f32 = 1.0;
/// MODEL (performance): lane followers get their entity (models, collider) within this distance of the player
/// and lose it beyond `DRAW_RANGE + 50 m`; the simulation runs for every car in the active area.
const DRAW_RANGE: f32 = 250.0;
/// `PlayerVsTraffic` contact response (`CollisionResponses_RaceCar`): friction 0.2, restitution −0.1 (taken as 0).
const CRASH_FRICTION: f32 = 0.2;

pub struct TrafficPlugin {
    pub enabled: bool,
    /// Multiplies the density (`--traffic-density`; 1 = the game's).
    pub density: f32,
}

impl Plugin for TrafficPlugin {
    fn build(&self, app: &mut App) {
        if !self.enabled {
            return;
        }
        app.insert_resource(TrafficSettings { density: self.density })
            .init_resource::<TrafficModels>()
            .add_systems(Update, (start_loading, receive_data, spawn_entities, count_log))
            .add_systems(FixedUpdate, (crash_hit_cars, traffic_tick, drive_bodies).chain());
    }
}

#[derive(Resource)]
struct TrafficSettings {
    density: f32,
}

/// The data, loaded on a background thread.
#[derive(Resource)]
struct DataJob(std::sync::Mutex<Receiver<Option<Arc<TrafficData>>>>);

/// Marks a lane follower's collider: `slot` is its index in [`TrafficSystem::cars`], `serial` tells reused
/// slots apart. The contact hook (`props::PropHooks`) adds [`TrafficHit`] when a dynamic body touches it.
#[derive(Component, Clone, Copy)]
pub struct TrafficBody {
    pub slot: usize,
    pub serial: u32,
}

/// Inserted by the contact hook on a lane follower's collider: turn the car into a crashing (physical) car.
#[derive(Component)]
pub struct TrafficHit;

/// The root entity of a traffic car (lane follower or crashed).
#[derive(Component)]
pub struct TrafficCar {
    /// Index into `TrafficData::vehicles` (for other systems: sounds, HUD).
    #[allow(dead_code)]
    pub vehicle: usize,
}

/// A wheel spinner of a traffic car: radius (m) for the spin.
#[derive(Component)]
struct TrafficWheel {
    radius: f32,
    left: bool,
}

/// Loaded traffic models by (vehicle type, paint).
#[derive(Resource, Default)]
pub struct TrafficModels {
    models: HashMap<(usize, usize), Arc<TrafficModel>>,
    requested: HashSet<(usize, usize)>,
}

struct TrafficModel {
    body: crate::ModelParts,
    wheels: Vec<(WheelSpec, Vec<(crate::ModelParts, bool)>)>,
}

impl TrafficModels {
    /// Called by the loader-message pump (`Msg::TrafficCar`).
    pub fn arrived(&mut self, spec: &TrafficCarSpec, parts: &HashMap<ResourceId, crate::ModelParts>) {
        let get = |m: &ResourceId| parts.get(m).cloned().unwrap_or_default();
        let model = TrafficModel {
            body: spec.body.iter().flat_map(get).collect(),
            wheels: spec
                .wheels
                .iter()
                .map(|w| (w.clone(), w.parts.iter().map(|(m, spins)| (get(m), *spins)).collect()))
                .collect(),
        };
        self.models.insert((spec.vehicle, spec.paint), Arc::new(model));
    }
}

/// What other systems can ask about traffic ([`TrafficSystem::cars_near`]).
#[derive(Debug, Clone, Copy)]
pub struct TrafficCarInfo {
    pub entity: Option<Entity>,
    pub position: Vec3,
    pub velocity: Vec3,
    /// Crashed (a dynamic rigid body) rather than following its lane.
    pub crashed: bool,
    /// Index into `TrafficData::vehicles`.
    pub vehicle: usize,
}

/// A traffic car as the engine-sound pool sees it ([`TrafficSystem::sound_cars`]).
#[derive(Debug, Clone, Copy)]
pub struct TrafficSoundCar {
    pub key: u32,
    pub position: Vec3,
    pub velocity: Vec3,
    pub forward: Vec3,
    pub vehicle: usize,
    pub random: f32,
}

/// The game's random source (`world+0x36190`): a 64-bit LCG whose high word feeds an 8-slot ring of floats
/// in [1, 2); each draw returns the slot written 8 draws earlier, minus 1.
struct GameRng {
    state: u64,
    ring: [f32; 8],
    index: usize,
}

impl GameRng {
    /// MODEL: the seed and the ring's start values are not known; any fixed seed keeps runs reproducible.
    fn new(seed: u64) -> Self {
        let mut r = Self { state: seed, ring: [1.0; 8], index: 0 };
        for _ in 0..8 {
            r.next();
        }
        r
    }

    fn next(&mut self) -> f32 {
        let out = self.ring[self.index];
        let high = (self.state >> 32) as u32;
        self.state = self.state.wrapping_mul(0x5851_f42d_4c95_7f2d).wrapping_add(1);
        self.ring[self.index] = f32::from_bits(high >> 9 | 0x3f80_0000);
        self.index = (self.index + 1) & 7;
        out - 1.0
    }
}

/// One lane follower (the 0x80-byte record at `world+(slot+0x37f)*0x80`).
#[derive(Clone)]
struct LaneCar {
    serial: u32,
    hull: u16,
    section: u8,
    /// Distance along the section (m); the game keeps a rung position, which maps to it linearly per rung.
    dist: f32,
    speed: f32,
    accel: f32,
    /// Speed before this tick's update (+0x44).
    prev_speed: f32,
    /// The car's random personality in [0, 1) (+0x3c, drawn at placement).
    personality: f32,
    vehicle: usize,
    paint: usize,
    /// Front and back extents along the lane: box centre z ± half length (+0x4c, +0x50).
    front: f32,
    back: f32,
    /// Behaviour mode (+0x22): 0 stop for an obstacle, 2 match an obstacle, 3 stop at a stop line,
    /// 4 follow the car ahead, 5 cruise.
    mode: u8,
    target_speed: f32,
    target_dist: f32,
    /// Seconds spent waiting behind an obstacle (+0x34).
    wait: f32,
    /// The planned route: up to 5 (hull, section) after the current one (+0x1bf88).
    path: VecDeque<(u16, u8)>,
    braking: bool,
    position: Vec3,
    forward: Vec3,
    up: Vec3,
    entity: Option<Entity>,
    leader: Option<usize>,
}

/// A crashed traffic car (a dynamic rigid body), still counted by the load budget.
struct Crashed {
    entity: Entity,
    vehicle: usize,
    position: Vec3,
    velocity: Vec3,
    right: Vec3,
}

#[derive(Default, Clone)]
struct JunctionState {
    timer: f32,
    phase: usize,
}

/// Per active hull (the 0x498-byte record at `world+0x37f08`).
#[derive(Default)]
struct HullState {
    junctions: Vec<JunctionState>,
    /// Stop lines held by their junction (+0x250 + index).
    held: Vec<bool>,
}

struct Emitter {
    hull: u16,
    section: u8,
    timer: f32,
}

/// An obstacle for the cone scan: position, velocity, right axis, weight.
#[derive(Clone, Copy)]
struct Obstacle {
    position: Vec3,
    velocity: Vec3,
    right: Vec3,
    weight: f32,
    /// The crashed car's entity (a crashed car does not avoid itself).
    entity: Option<Entity>,
}

#[derive(Resource)]
pub struct TrafficSystem {
    data: Arc<TrafficData>,
    rng: GameRng,
    cars: Vec<Option<LaneCar>>,
    serial: u32,
    crashed: Vec<Crashed>,
    active: Vec<u16>,
    active_set: Vec<bool>,
    last_hull: Option<usize>,
    hulls: HashMap<u16, HullState>,
    emitters: Vec<Emitter>,
    emitter_phase: f32,
    /// Density multiplier (`world+0xa2b7c`).
    multiplier: f32,
    load: f32,
    cull_timer: f32,
    /// Round-robin paint per vehicle type (`world+0xa0390`).
    paint_cursor: Vec<usize>,
    /// Cars per (hull, section), sorted by distance along the section; rebuilt every tick.
    by_section: HashMap<(u16, u8), Vec<usize>>,
    density_scale: f32,
    pub spawned: usize,
    pub removed: usize,
    /// Time spent in the tick (s) and ticks, for `MW_TRAFFIC_LOG`.
    tick_time: f64,
    ticks: u32,
}

impl TrafficSystem {
    fn new(data: Arc<TrafficData>, density_scale: f32) -> Self {
        let n_hulls = data.lanes.hulls.len();
        let n_vehicles = data.vehicles.len();
        Self {
            data,
            rng: GameRng::new(0x2545_f491_4f6c_dd1d),
            cars: vec![None; MAX_CARS],
            serial: 0,
            crashed: Vec::new(),
            active: Vec::new(),
            active_set: vec![false; n_hulls],
            last_hull: None,
            hulls: HashMap::new(),
            emitters: Vec::new(),
            emitter_phase: 0.0,
            multiplier: 0.0,
            load: 0.0,
            cull_timer: 0.0,
            paint_cursor: vec![0; n_vehicles],
            by_section: HashMap::new(),
            density_scale,
            spawned: 0,
            removed: 0,
            tick_time: 0.0,
            ticks: 0,
        }
    }

    /// Traffic cars within `radius` of `point` (lane followers and crashed ones).
    pub fn cars_near(&self, point: Vec3, radius: f32) -> Vec<TrafficCarInfo> {
        let r2 = radius * radius;
        let lane = self.cars.iter().flatten().map(|c| TrafficCarInfo {
            entity: c.entity,
            position: c.position,
            velocity: c.forward * c.speed,
            crashed: false,
            vehicle: c.vehicle,
        });
        let crashed = self.crashed.iter().map(|c| TrafficCarInfo {
            entity: Some(c.entity),
            position: c.position,
            velocity: c.velocity,
            crashed: true,
            vehicle: c.vehicle,
        });
        lane.chain(crashed).filter(|c| c.position.distance_squared(point) <= r2).collect()
    }

    /// Traffic cars within `radius` of `point` for the engine-sound pool (`car_audio.rs`): a key that tells
    /// cars apart (lane followers by spawn serial, crashed ones by entity, as [`Self::near_miss_query`]), the
    /// position, velocity and forward axis, the vehicle type and the car's random value (its personality).
    pub fn sound_cars(&self, point: Vec3, radius: f32) -> Vec<TrafficSoundCar> {
        let r2 = radius * radius;
        let lane = self.cars.iter().flatten().map(|c| TrafficSoundCar {
            key: c.serial,
            position: c.position,
            velocity: c.forward * c.speed,
            forward: c.forward,
            vehicle: c.vehicle,
            random: c.personality,
        });
        let crashed = self.crashed.iter().map(|c| TrafficSoundCar {
            key: 0x8000_0000 | c.entity.to_bits() as u32,
            position: c.position,
            velocity: c.velocity,
            forward: c.right.cross(Vec3::Y).normalize_or_zero(),
            vehicle: c.vehicle,
            // MODEL: a crashed car keeps no personality here; any fixed value.
            random: 0.5,
        });
        lane.chain(crashed).filter(|c| c.position.distance_squared(point) <= r2).collect()
    }

    /// The traffic vehicle types (for their sounds).
    pub fn vehicles(&self) -> &[mw_assets::traffic::TrafficVehicleType] {
        &self.data.vehicles
    }

    /// The nitrous near-miss query (`FUN_00665f30` → the world's sphere query `FUN_00aa0440` on broadphase
    /// category 8, traffic only): cars whose bounding sphere meets the sphere (`point`, `radius`), with an id
    /// (lane followers by spawn serial, crashed ones by entity) and their forward axis. The bounding sphere is
    /// taken as the physical box's (centre, |half extents|) (model decision: the broadphase's own spheres measured
    /// 2.32–2.58 m in a snapshot).
    pub fn near_miss_query(&self, point: Vec3, radius: f32) -> Vec<(u32, Vec3)> {
        let sphere = |vehicle: usize, origin: Vec3, forward: Vec3, up: Vec3| {
            let v = &self.data.vehicles[vehicle];
            let centre = origin + forward * v.box_centre[2] + up * v.box_centre[1];
            (centre, Vec3::from(v.half_extents).length())
        };
        let meets = |(c, r): (Vec3, f32)| c.distance_squared(point) <= (r + radius) * (r + radius);
        let lane = self
            .cars
            .iter()
            .flatten()
            .filter(|c| meets(sphere(c.vehicle, c.position, c.forward, c.up)))
            .map(|c| (c.serial, c.forward));
        let crashed = self.crashed.iter().filter_map(|c| {
            // right = rotation × X, so forward = right × up (up taken as world up for a crashing car).
            let forward = c.right.cross(Vec3::Y).normalize_or_zero();
            meets(sphere(c.vehicle, c.position, forward, Vec3::Y))
                .then_some((0x8000_0000 | c.entity.to_bits() as u32, forward))
        });
        lane.chain(crashed).collect()
    }

    /// Remove every traffic car (lane followers and crashed ones) within `radius` of `point`
    /// (`ClearTrafficSequenceItem`, `cutscene.rs`).
    pub fn clear_near(&mut self, point: Vec3, radius: f32, commands: &mut Commands) -> usize {
        let r2 = radius * radius;
        let mut n = 0;
        for slot in 0..MAX_CARS {
            if self.cars[slot].as_ref().is_some_and(|c| c.position.distance_squared(point) <= r2) {
                remove_car(self, slot, commands);
                n += 1;
            }
        }
        let before = self.crashed.len();
        self.crashed.retain(|c| {
            let near = c.position.distance_squared(point) <= r2;
            if near {
                commands.entity(c.entity).despawn();
            }
            !near
        });
        n += before - self.crashed.len();
        self.removed += before - self.crashed.len();
        n
    }

    /// Number of lane followers and crashed cars.
    pub fn counts(&self) -> (usize, usize) {
        (self.cars.iter().flatten().count(), self.crashed.len())
    }

    fn section(&self, hull: u16, section: u8) -> Option<&mw_formats::TrafficSection> {
        self.data.lanes.hulls.get(hull as usize)?.sections.get(section as usize)
    }

    fn section_length(&self, hull: u16, section: u8) -> f32 {
        self.section(hull, section).map_or(0.0, |s| s.length)
    }

    fn flow(&self, hull: u16, section: u8) -> mw_assets::traffic::TrafficFlow {
        self.section(hull, section).and_then(|s| self.data.flows.get(s.flow as usize).copied()).unwrap_or_default()
    }

    /// Cruise speed of a section: `TrafficFlow.Speed` × mph × the runtime scale (`FUN_00617d00`).
    fn cruise(&self, hull: u16, section: u8) -> f32 {
        self.flow(hull, section).speed_mph * MPH * SPEED_SCALE
    }

    /// Distance along a section of a rung position (`rung` + fraction), from the cumulative rung lengths.
    fn rung_dist(&self, hull: u16, section: u8, rung_pos: f32) -> f32 {
        let Some(h) = self.data.lanes.hulls.get(hull as usize) else { return 0.0 };
        let Some(s) = h.sections.get(section as usize) else { return 0.0 };
        let last = (s.rung_count as usize).saturating_sub(1);
        let r = (rung_pos.floor().max(0.0) as usize).min(last);
        let cum = |k: usize| h.cumulative_lengths.get(s.first_rung as usize + k).copied().unwrap_or(0.0);
        if r >= last {
            return cum(last);
        }
        cum(r) + (rung_pos - r as f32).clamp(0.0, 1.0) * (cum(r + 1) - cum(r))
    }

    /// Lane point and direction at a distance along a section: the rung midpoints interpolated, the direction
    /// of the rung segment, and the rungs' cross direction (`FUN_005e7f20`).
    fn lane_pose(&self, hull: u16, section: u8, dist: f32) -> Option<(Vec3, Vec3, Vec3)> {
        let h = self.data.lanes.hulls.get(hull as usize)?;
        let s = h.sections.get(section as usize)?;
        let n = s.rung_count as usize;
        if n < 2 {
            return None;
        }
        let first = s.first_rung as usize;
        let cum = |k: usize| h.cumulative_lengths.get(first + k).copied().unwrap_or(0.0);
        let mut r = 0;
        while r + 2 < n && cum(r + 1) <= dist {
            r += 1;
        }
        let (a, b) = (h.rungs.get(first + r)?, h.rungs.get(first + r + 1)?);
        let mid = |x: &[[f32; 3]; 2]| (Vec3::from(x[0]) + Vec3::from(x[1])) * 0.5;
        let across = |x: &[[f32; 3]; 2]| Vec3::from(x[1]) - Vec3::from(x[0]);
        let seg = (cum(r + 1) - cum(r)).max(1e-3);
        let f = ((dist - cum(r)) / seg).clamp(0.0, 1.0);
        let (pa, pb) = (mid(a), mid(b));
        let p = pa.lerp(pb, f);
        let dir = (pb - pa).normalize_or(Vec3::Z);
        let right = across(a).lerp(across(b), f).normalize_or(Vec3::X);
        Some((p, dir, right))
    }

    fn hull_state(&mut self, hull: u16) -> &mut HullState {
        let data = self.data.clone();
        self.hulls.entry(hull).or_insert_with(|| {
            let h = &data.lanes.hulls[hull as usize];
            HullState {
                // MODEL: the record starts zeroed (timer 0, phase 0): the first tick moves every junction to
                // its phase 1 (the record's initialiser was not read).
                junctions: vec![JunctionState::default(); h.junctions.len()],
                held: vec![false; h.stop_lines.len()],
            }
        })
    }

    fn held(&self, hull: u16, stop_line: usize) -> bool {
        self.hulls.get(&hull).and_then(|h| h.held.get(stop_line).copied()).unwrap_or(false)
    }

    /// Density in cars per minute for a flow density (`FUN_005fb120`).
    fn density(&self, density: f32) -> f32 {
        if density > 0.0 && self.multiplier > 0.0 {
            let floor = density.min(self.data.system.min_density);
            (self.multiplier * density).max(floor)
        } else {
            0.0
        }
    }

    /// The spawn interval of an emitter on this section (`FUN_005fb120`): `60 / d × lerp(0xc8, 0xb8, u)`.
    fn interval(&mut self, hull: u16, section: u8) -> f32 {
        let d = self.density(self.flow(hull, section).density).max(1e-15);
        let u = self.rng.next();
        let s = &self.data.system;
        60.0 / d * (s.interval_min + (s.interval_max - s.interval_min) * u)
    }

    /// Pick a vehicle type from a flow type by `Proportion` (`FUN_00608c80`), and its paint: round-robin
    /// over the type's palette (`FUN_00608720`, `world+0xa0390`), or the entry's `Colour` when set.
    fn pick_vehicle(&mut self, flow_type: u8) -> Option<(usize, usize)> {
        let entries = self.data.flow_types.get(flow_type as usize)?;
        let total: f32 = entries.iter().map(|e| e.proportion).sum();
        if entries.is_empty() || total <= 0.0 {
            return None;
        }
        let r = self.rng.next() * total;
        let mut sum = 0.0;
        let mut pick = entries[entries.len() - 1];
        for e in entries {
            sum += e.proportion;
            if sum >= r {
                pick = *e;
                break;
            }
        }
        let paints = &self.data.vehicles[pick.vehicle].paints;
        let paint = match paints.iter().position(|(id, _)| pick.colour != 0 && id.0 as u32 == pick.colour) {
            Some(i) => i,
            None if paints.is_empty() => 0,
            None => {
                let c = &mut self.paint_cursor[pick.vehicle];
                *c = (*c + 1) % paints.len();
                *c
            }
        };
        Some((pick.vehicle, paint))
    }

    /// Place a car on a lane (`FUN_00608720` → `FUN_005f4d10`): not within the clearance of the player.
    fn place(&mut self, hull: u16, section: u8, dist: f32, pick: (usize, usize), player: Option<Vec3>) -> bool {
        let Some((position, dir, right)) = self.lane_pose(hull, section, dist) else { return false };
        if let Some(p) = player
            && p.distance(position) < self.data.system.spawn_clearance
        {
            return false;
        }
        let Some(slot) = self.cars.iter().position(Option::is_none) else { return false };
        let personality = self.rng.next();
        let v = &self.data.vehicles[pick.0];
        let front = v.box_centre[2] + v.half_extents[2];
        let mut back = v.box_centre[2] - v.half_extents[2];
        if v.trailer {
            back -= TRAILER_LENGTH;
        }
        let speed = self.cruise(hull, section);
        self.serial = self.serial.wrapping_add(1);
        let up = up_from(dir, right);
        self.cars[slot] = Some(LaneCar {
            serial: self.serial,
            hull,
            section,
            dist,
            speed,
            accel: 0.0,
            prev_speed: speed,
            personality,
            vehicle: pick.0,
            paint: pick.1,
            front,
            back,
            mode: 5,
            target_speed: speed,
            target_dist: f32::MAX,
            wait: 0.0,
            path: VecDeque::new(),
            braking: false,
            position,
            forward: dir,
            up,
            entity: None,
            leader: None,
        });
        self.spawned += 1;
        self.plan(slot);
        self.index_sections();
        true
    }

    /// Rebuild the per-section car lists, sorted by distance along the section.
    fn index_sections(&mut self) {
        self.by_section.clear();
        for (i, c) in self.cars.iter().enumerate() {
            if let Some(c) = c {
                self.by_section.entry((c.hull, c.section)).or_default().push(i);
            }
        }
        let cars = &self.cars;
        for list in self.by_section.values_mut() {
            list.sort_by(|a, b| {
                let (a, b) = (cars[*a].as_ref().unwrap(), cars[*b].as_ref().unwrap());
                a.dist.total_cmp(&b.dist)
            });
        }
    }

    /// The rearmost car on a section.
    fn last_on(&self, hull: u16, section: u8) -> Option<usize> {
        self.by_section.get(&(hull, section)).and_then(|l| l.first().copied())
    }

    /// The frontmost car on a section.
    fn first_on(&self, hull: u16, section: u8) -> Option<usize> {
        self.by_section.get(&(hull, section)).and_then(|l| l.last().copied())
    }

    fn car(&self, i: usize) -> &LaneCar {
        self.cars[i].as_ref().expect("live car")
    }

    /// Choose the next section after `(hull, section)`: the forward link whose span chain is least loaded
    /// (`FUN_00618470`: cars in the span × its reciprocal capacity, summed along the span links; 1 for a
    /// hull outside the area); ties go to the first link.
    fn choose_next(&self, hull: u16, section: u8, counts: &HashMap<(u16, u8), u16>) -> Option<(u16, u8)> {
        let s = self.section(hull, section)?;
        let mut best: Option<((u16, u8), f32)> = None;
        for &(h, sec) in &s.forward {
            let load = if self.active_set.get(h as usize) == Some(&true) {
                let lanes = &self.data.lanes.hulls;
                let span_of = |h: u16, i: u8| lanes.get(h as usize).and_then(|x| x.spans.get(i as usize)).copied();
                let Some(first_span) = self.section(h, sec).map(|x| x.span) else { continue };
                let mut load = 0.0;
                // The chain walk (with the game's tortoise-and-hare loop check); counts are read from the
                // first hull's record with the chained span index, as the code does.
                let (mut slow, mut fast) = ((h, first_span), (h, first_span));
                let mut toggle = false;
                let mut steps = 0;
                while let Some((max, next_hull, next)) = span_of(fast.0, fast.1) {
                    let recip = if max > 0.0 { 1.0 / max } else { 0.0 };
                    load += *counts.get(&(h, fast.1)).unwrap_or(&0) as f32 * recip;
                    if next_hull == 0xffff || steps > 64 {
                        break;
                    }
                    fast = (next_hull, next as u8);
                    toggle = !toggle;
                    if !toggle {
                        if let Some((_, sh, sn)) = span_of(slow.0, slow.1) {
                            slow = (sh, sn as u8);
                        }
                        if slow.1 == fast.1 {
                            break;
                        }
                    }
                    steps += 1;
                }
                load
            } else {
                1.0
            };
            if best.is_none_or(|(_, b)| load < b) {
                best = Some(((h, sec), load));
            }
        }
        best.map(|(n, _)| n)
    }

    /// Extend a car's planned route until it reaches `prm[0x34]` (25 m) past the car, 5 sections at most
    /// (`FUN_005fc020`).
    fn plan(&mut self, slot: usize) {
        let counts = self.span_counts();
        let Some(car) = self.cars[slot].as_ref() else { return };
        let mut ahead = self.section_length(car.hull, car.section) - car.dist;
        let mut last = (car.hull, car.section);
        let mut path = car.path.clone();
        for &(h, s) in &path {
            ahead += self.section_length(h, s);
            last = (h, s);
        }
        while ahead < self.data.params.plan_distance && path.len() < 5 {
            let Some(next) = self.choose_next(last.0, last.1, &counts) else { break };
            path.push_back(next);
            ahead += self.section_length(next.0, next.1);
            last = next;
        }
        if let Some(car) = self.cars[slot].as_mut() {
            car.path = path;
        }
    }

    /// Cars per (hull, span) (`+0x290` of the hull record).
    fn span_counts(&self) -> HashMap<(u16, u8), u16> {
        let mut out = HashMap::new();
        for c in self.cars.iter().flatten() {
            if let Some(s) = self.section(c.hull, c.section) {
                *out.entry((c.hull, s.span)).or_default() += 1;
            }
        }
        out
    }

    /// The distance to the next held stop line on the car's route, within `prm[0xb0]` (`FUN_005f0d90`).
    fn stop_line_ahead(&self, car: &LaneCar) -> Option<f32> {
        let lanes = &self.data.lanes;
        let range = self.data.params.stop_line_range;
        let line_dist = |hull: u16, section: u8, after: f32| -> Option<(usize, f32)> {
            let h = lanes.hulls.get(hull as usize)?;
            let s = h.sections.get(section as usize)?;
            let first = s.stop_line? as usize;
            (first..first + s.stop_line_count as usize).find_map(|i| {
                let v = *h.stop_lines.get(i)?;
                let d = self.rung_dist(hull, section, (v >> 8) as f32 + (v & 0xff) as f32 / 256.0);
                (d > after).then_some((i, d))
            })
        };
        if let Some((i, d)) = line_dist(car.hull, car.section, car.dist) {
            return (self.held(car.hull, i) && d - car.dist < range).then_some(d - car.dist);
        }
        let mut acc = self.section_length(car.hull, car.section) - car.dist;
        for &(h, s) in car.path.iter().take(5) {
            if let Some((i, d)) = line_dist(h, s, f32::MIN) {
                if !self.held(h, i) {
                    return None;
                }
                return (acc + d < range).then_some(acc + d);
            }
            acc += self.section_length(h, s);
        }
        None
    }

    /// The car ahead within `limit` and the distance to its back (`FUN_005f1380`): the next car on the same
    /// section, the rearmost car on the forward links (two levels), and cars about to merge into the same
    /// successor when the section end is within `prm[0x5c]`.
    fn leader(&self, slot: usize, limit: f32) -> Option<(usize, f32)> {
        let car = self.car(slot);
        let mut best: Option<(usize, f32)> = None;
        let mut consider = |i: usize, d: f32| {
            if i != slot && d < limit && best.is_none_or(|(_, b)| d < b) {
                best = Some((i, d));
            }
        };
        if let Some(list) = self.by_section.get(&(car.hull, car.section)) {
            let me = list.iter().position(|&i| i == slot);
            if let Some(&next) = me.and_then(|k| list.get(k + 1)) {
                let o = self.car(next);
                consider(next, (o.dist - car.dist).max(0.0) + o.back);
            }
        }
        let rem = self.section_length(car.hull, car.section) - car.dist;
        if rem < limit
            && let Some(s) = self.section(car.hull, car.section)
        {
            for &(h, sec) in &s.forward {
                if let Some(i) = self.last_on(h, sec) {
                    let o = self.car(i);
                    consider(i, o.dist + rem + o.back);
                }
                let len = self.section_length(h, sec);
                if rem + len < limit
                    && let Some(s2) = self.section(h, sec)
                {
                    for &(h2, sec2) in &s2.forward {
                        if let Some(i) = self.last_on(h2, sec2) {
                            let o = self.car(i);
                            consider(i, o.dist + rem + len + o.back);
                        }
                    }
                }
                // Merging: other sections feeding the same successor.
                if rem < self.data.params.merge_distance
                    && let Some(succ) = self.section(h, sec)
                {
                    for &(bh, bs) in &succ.backward {
                        if (bh, bs) == (car.hull, car.section) {
                            continue;
                        }
                        if let Some(i) = self.first_on(bh, bs) {
                            let o = self.car(i);
                            let their_rem = self.section_length(bh, bs) - o.dist;
                            if their_rem < rem && (o.mode != 3 || o.speed >= self.data.params.merge_stopped_speed) {
                                consider(i, rem - their_rem + o.back);
                            }
                        }
                    }
                }
            }
        }
        best
    }

    /// The lane direction `distance` metres ahead along the route (`FUN_005f11a0`).
    fn direction_ahead(&self, car: &LaneCar, distance: f32) -> Option<Vec3> {
        let mut d = car.dist + distance;
        let (mut h, mut s) = (car.hull, car.section);
        let mut path = car.path.iter();
        loop {
            let len = self.section_length(h, s);
            if d <= len {
                break;
            }
            match path.next() {
                Some(&(nh, ns)) => {
                    d -= len;
                    (h, s) = (nh, ns);
                }
                None => {
                    d = len;
                    break;
                }
            }
        }
        self.lane_pose(h, s, d).map(|(_, dir, _)| dir)
    }
}

/// The car's up axis from its forward direction and the rung's cross direction.
fn up_from(forward: Vec3, across: Vec3) -> Vec3 {
    let up = forward.cross(across).normalize_or(Vec3::Y);
    if up.y < 0.0 { -up } else { up }
}

/// The fuzzy scores of the six options (`FUN_005ea6a0`; layout and terms read from its disassembly).
/// `obstacle` = (distance ahead, height, closing speed, lateral offset), `lead` = (stop line distance, gap,
/// closing speed, obstacle speed along the heading), `own` = (impatience, waiting flag, mode-1 bonus).
fn scores(f: &mw_assets::traffic::FuzzyConfig, obstacle: [f32; 4], lead: [f32; 4], own: [f32; 3]) -> [f32; 6] {
    let m0 = f.obstacle_distance.map(|e| e.of(obstacle[0]));
    let m1 = f.obstacle_height.of(obstacle[1]);
    let m2 = f.closing.map(|e| e.of(obstacle[2]));
    let m3 = f.lateral[0].of(obstacle[3]);
    let m4 = f.stop_line.of(lead[0]);
    let m5 = f.leader_gap.of(lead[1]);
    let m6 = f.closing[0].of(lead[2]);
    let m7 = f.impatience.of(own[0]);
    let s1 = m1.min(m3.min(m0[1].min(m2[2]).max(m0[0].min(1.0 - m2[0]))));
    let s2 = m1.min(m0[1].max(m0[2]).min(m3.min(1.0 - m2[2])));
    let s0 = if s1.max(s2) > 0.0 && own[2] > 0.0 { 1.0 } else { m7.min(own[1].max(s1.max(s2))) + own[2] };
    [s0, s1, s2, m4, m5.min(1.0 - m6), f.cruise_score]
}

/// The option the arbiter takes (`FUN_00617d00`): the first score that beats the best so far by the
/// hysteresis.
fn choose(scores: [f32; 6]) -> usize {
    let (mut best, mut pick) = (0.0, 0);
    for (i, s) in scores.into_iter().enumerate() {
        if best < s {
            best = s + SCORE_HYSTERESIS;
            pick = i;
        }
    }
    pick
}

struct Decision {
    mode: u8,
    target_speed: f32,
    target_dist: f32,
    leader: Option<usize>,
}

/// Start loading the traffic data once the world request is known.
fn start_loading(
    mut commands: Commands,
    game: Option<Res<crate::Game>>,
    job: Option<Res<DataJob>>,
    mut done: Local<bool>,
) {
    if *done || job.is_some() {
        return;
    }
    let Some(game) = game else { return };
    *done = true;
    let store = game.store.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("mw-traffic".into())
        .spawn(move || {
            let data = match store.traffic_data() {
                Ok(d) => Some(d),
                Err(e) => {
                    warn!("traffic: no data ({e})");
                    None
                }
            };
            let _ = tx.send(data);
        })
        .expect("spawn traffic thread");
    commands.insert_resource(DataJob(std::sync::Mutex::new(rx)));
}

fn receive_data(
    mut commands: Commands,
    job: Option<Res<DataJob>>,
    settings: Res<TrafficSettings>,
    system: Option<Res<TrafficSystem>>,
) {
    if system.is_some() {
        return;
    }
    let Some(job) = job else { return };
    let Ok(Some(data)) = job.0.lock().unwrap().try_recv() else { return };
    info!(
        "traffic: {} hulls, {} vehicle types, density ×{}",
        data.lanes.hulls.len(),
        data.vehicles.len(),
        data.system.density_base * settings.density
    );
    commands.insert_resource(TrafficSystem::new(data, settings.density));
}

/// Point-in-polygon on a hull cell's `[x, z]` outline.
fn in_cell(points: &[[f32; 2]], x: f32, z: f32) -> bool {
    let mut inside = false;
    let n = points.len();
    for i in 0..n {
        let (a, b) = (points[i], points[(i + n - 1) % n]);
        if (a[1] > z) != (b[1] > z) && x < (b[0] - a[0]) * (z - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
    }
    inside
}

/// The player's position and velocity, else the camera's position.
type PlayerQuery<'w, 's> =
    Query<'w, 's, (&'static Transform, &'static LinearVelocity), (With<crate::drive::Player>, Without<TrafficCar>)>;

#[allow(clippy::too_many_arguments)]
fn traffic_tick(
    mut commands: Commands,
    system: Option<ResMut<TrafficSystem>>,
    time: Res<Time>,
    player: PlayerQuery,
    camera: Query<&GlobalTransform, crate::MainCamera>,
    crashed_q: Query<(&Transform, &LinearVelocity), With<TrafficCar>>,
) {
    let Some(mut sys) = system else { return };
    let sys = &mut *sys;
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let started = std::time::Instant::now();
    tick(sys, &mut commands, dt, &player, &camera, &crashed_q);
    sys.tick_time += started.elapsed().as_secs_f64();
    sys.ticks += 1;
}

fn tick(
    sys: &mut TrafficSystem,
    commands: &mut Commands,
    dt: f32,
    player: &PlayerQuery,
    camera: &Query<&GlobalTransform, crate::MainCamera>,
    crashed_q: &Query<(&Transform, &LinearVelocity), With<TrafficCar>>,
) {
    // MODEL: traffic waits for the player's car (in a drive session it spawns once the ground under it has
    // streamed in; the camera stands elsewhere until then). Without a car (the free camera) it follows the
    // camera.
    let (player_pos, player_vel, player_right) = match player.iter().next() {
        Some((t, v)) => (Some(t.translation), v.0, t.rotation * Vec3::X),
        None if sys.last_hull.is_some() => (camera.iter().next().map(|t| t.translation()), Vec3::ZERO, Vec3::X),
        None => return,
    };
    let Some(focus) = player_pos else { return };

    // Crashed cars: refresh their state from the physics.
    sys.crashed.retain_mut(|c| match crashed_q.get(c.entity) {
        Ok((t, v)) => {
            c.position = t.translation;
            c.velocity = v.0;
            c.right = t.rotation * Vec3::X;
            true
        }
        Err(_) => false,
    });

    // The density multiplier (`FUN_005f0780`, which the frame function runs before the tick).
    let s = sys.data.system.clone();
    let speed_mph = player_vel.length() / MPH;
    let t = ((speed_mph - s.fast_mph) / (s.slow_mph - s.fast_mph)).clamp(0.0, 1.0);
    sys.multiplier = s.density_base * (s.fast_factor + (s.slow_factor - s.fast_factor) * t) * sys.density_scale;

    // The active area.
    update_area(sys, commands, focus);
    // Crashed cars go with the area too (inferred, as lane followers do).
    let data = sys.data.clone();
    let active = sys.active.clone();
    sys.crashed.retain(|c| {
        let inside = active.iter().any(|&h| in_cell(&data.lanes.hulls[h as usize].points, c.position.x, c.position.z));
        if !inside {
            commands.entity(c.entity).despawn();
        }
        inside
    });

    sys.index_sections();

    // 3. Decisions.
    let mut obstacles: Vec<Obstacle> = Vec::new();
    if player.iter().next().is_some() {
        obstacles.push(Obstacle {
            position: focus,
            velocity: player_vel,
            right: player_right,
            weight: PLAYER_WEIGHT,
            entity: None,
        });
    }
    for c in &sys.crashed {
        obstacles.push(Obstacle {
            position: c.position,
            velocity: c.velocity,
            right: c.right,
            weight: CRASHED_WEIGHT,
            entity: Some(c.entity),
        });
    }
    let decisions: Vec<Option<Decision>> =
        (0..MAX_CARS).map(|i| sys.cars[i].as_ref().map(|_| decide(sys, i, &obstacles))).collect();
    for (i, d) in decisions.into_iter().enumerate() {
        if let (Some(d), Some(car)) = (d, sys.cars[i].as_mut()) {
            car.mode = d.mode;
            car.target_speed = d.target_speed;
            car.target_dist = d.target_dist;
            car.leader = d.leader;
        }
    }

    // 4. Emitters, while the load allows (`FUN_0060e5e0`).
    if sys.load < LOAD_BUDGET {
        emit(sys, dt, player_pos);
    }

    // 5. Motion.
    for i in 0..MAX_CARS {
        if sys.cars[i].is_some() {
            move_car(sys, i, dt, commands);
        }
    }

    // 6. Traffic lights.
    cycle_lights(sys, dt);

    // 7. The load budget.
    budget(sys, focus, dt, commands);
}

/// Active hull set, hull population and removal of cars that left it (`FUN_005f0270`, `FUN_0060c9f0`).
fn update_area(sys: &mut TrafficSystem, commands: &mut Commands, focus: Vec3) {
    let data = sys.data.clone();
    let lanes = &data.lanes;
    let inside = |i: usize| lanes.hulls.get(i).is_some_and(|h| in_cell(&h.points, focus.x, focus.z));
    let cell = match sys.last_hull {
        Some(h) if inside(h) => Some(h),
        _ => (0..lanes.hulls.len()).find(|&i| inside(i)).or(sys.last_hull),
    };
    let Some(cell) = cell else { return };
    let changed = sys.last_hull != Some(cell);
    sys.last_hull = Some(cell);
    if !changed && !sys.active.is_empty() {
        return;
    }
    let mut active: Vec<u16> = vec![cell as u16];
    active.extend(lanes.hulls[cell].cell_neighbours.iter().copied().filter(|&h| (h as usize) < lanes.hulls.len()));
    active.dedup();
    let old: HashSet<u16> = sys.active.iter().copied().collect();
    let new: HashSet<u16> = active.iter().copied().collect();
    sys.active_set.iter_mut().for_each(|a| *a = false);
    for &h in &active {
        sys.active_set[h as usize] = true;
    }
    // Hulls that left: their state goes, and (inferred) the cars on them.
    for h in old.difference(&new) {
        sys.hulls.remove(h);
    }
    for i in 0..MAX_CARS {
        if let Some(c) = &sys.cars[i]
            && !new.contains(&c.hull)
        {
            remove_car(sys, i, commands);
        }
    }
    sys.active = active.clone();
    // Emitters (`FUN_0061c780`): sections with a density whose first backward link is missing or outside
    // the area; persisting emitters keep their timers (inferred).
    let mut emitters = Vec::new();
    for &h in &active {
        let n = sys.data.lanes.hulls[h as usize].sections.len();
        for si in 0..n {
            let s = &sys.data.lanes.hulls[h as usize].sections[si];
            if sys.data.flows.get(s.flow as usize).is_none_or(|f| f.density == 0.0) {
                continue;
            }
            let edge = s.backward.first().is_none_or(|&(bh, _)| !new.contains(&bh));
            if !edge {
                continue;
            }
            let si = si as u8;
            let timer = match sys.emitters.iter().find(|e| e.hull == h && e.section == si) {
                Some(e) => e.timer,
                None => {
                    let interval = sys.interval(h, si);
                    let t = interval * 0.5 + sys.emitter_phase;
                    sys.emitter_phase = (sys.emitter_phase + sys.data.system.emitter_phase).fract();
                    t
                }
            };
            emitters.push(Emitter { hull: h, section: si, timer });
        }
    }
    sys.emitters = emitters;
    // Newly active hulls are filled along their lanes (`FUN_0060db90`).
    sys.index_sections();
    for &h in active.iter().filter(|h| !old.contains(h)) {
        sys.hull_state(h);
        populate_hull(sys, h, Some(focus));
    }
}

/// Fill a newly active hull (`FUN_0060db90` → `FUN_0060ce20`): from every section entered from another hull,
/// follow the first forward link within the hull, then the other two links of the starting section.
fn populate_hull(sys: &mut TrafficSystem, hull: u16, player: Option<Vec3>) {
    let n = sys.data.lanes.hulls[hull as usize].sections.len();
    let mut carry = sys.rng.next() * 0.99;
    let mut visited = vec![false; n];
    for si in 0..n {
        let entered = sys.data.lanes.hulls[hull as usize].sections[si].backward.first().is_none_or(|&(h, _)| h != hull);
        if entered {
            carry = populate_chain(sys, hull, si, carry, &mut visited, player);
        }
    }
}

fn populate_chain(
    sys: &mut TrafficSystem,
    hull: u16,
    start: usize,
    mut carry: f32,
    visited: &mut [bool],
    player: Option<Vec3>,
) -> f32 {
    if visited[start] {
        return carry;
    }
    visited[start] = true;
    carry = populate_section(sys, hull, start as u8, carry, player);
    let mut cur = start;
    while let Some(&(h, next)) = sys.data.lanes.hulls[hull as usize].sections[cur].forward.first() {
        if h != hull || visited.get(next as usize) != Some(&false) {
            break;
        }
        visited[next as usize] = true;
        carry = populate_section(sys, hull, next, carry, player);
        cur = next as usize;
    }
    let others: Vec<(u16, u8)> =
        sys.data.lanes.hulls[hull as usize].sections[start].forward.iter().skip(1).copied().collect();
    for (h, s) in others {
        if h == hull {
            carry = populate_chain(sys, hull, s as usize, carry, visited, player);
        }
    }
    carry
}

/// Cars on one section at the flow's density (`FUN_0060afb0`): `length / speed / (60 / d) + carry` cars,
/// evenly spaced with a random offset of up to `sys[0x84]` (0.3) of the spacing. Returns the new carry.
fn populate_section(sys: &mut TrafficSystem, hull: u16, section: u8, carry: f32, player: Option<Vec3>) -> f32 {
    let flow = sys.flow(hull, section);
    if flow.density == 0.0 {
        return carry;
    }
    let d = sys.density(flow.density);
    let Some(s) = sys.section(hull, section) else { return carry };
    let (length, flow_type) = (s.length, s.flow_type);
    let speed = (flow.speed_mph * MPH).max(1e-3);
    let total = if d > 0.0 { length / speed / (60.0 / d) + carry } else { carry };
    let n = total.floor();
    if n >= 1.0 {
        let spacing = length / n;
        let mut base = spacing * (total - n);
        for _ in 0..n as usize {
            let jitter = sys.rng.next() * sys.data.system.prepopulate_jitter * spacing;
            if let Some(pick) = sys.pick_vehicle(flow_type) {
                let dist = (base + jitter).min(length);
                sys.place(hull, section, dist, pick, player);
            }
            base += spacing;
        }
    }
    total - n
}

/// One car's decision (`FUN_005fc220` → `FUN_00617d00`).
fn decide(sys: &TrafficSystem, slot: usize, obstacles: &[Obstacle]) -> Decision {
    let car = sys.car(slot);
    let p = &sys.data.params;
    let v = car.speed;
    let cruise = sys.cruise(car.hull, car.section);
    // The car ahead.
    let limit = LOOK_FACTOR * (p.look_time * v).max(p.look_distance);
    let leader = sys.leader(slot, limit);
    let (gap, closing) = match leader {
        Some((i, d)) => {
            let gap = (d - car.front - p.standstill_gap - p.personality_gap * car.personality).max(0.0);
            (gap, v - sys.car(i).speed)
        }
        None => (f32::MAX, f32::MAX),
    };
    let stop = sys.stop_line_ahead(car).unwrap_or(f32::MAX);
    // The obstacle scan: nearest by |d| × weight × (1 − cos) in the cone.
    let mut obstacle = [f32::MAX; 4];
    let mut obstacle_speed = f32::MAX;
    let right = car.up.cross(car.forward).normalize_or(Vec3::X);
    let mut best = f32::MAX;
    for o in obstacles {
        if o.entity.is_some() && o.entity == car.entity {
            continue;
        }
        let diff = o.position - car.position;
        let scaled = Vec3::new(diff.x, diff.y * CONE_HEIGHT_SCALE, diff.z);
        let along = car.forward.dot(scaled);
        if !(0.0..=CONE_RANGE).contains(&along) {
            continue;
        }
        let len = scaled.length();
        let cos = if len > 0.0 { car.forward.dot(scaled / len) } else { 1.0 };
        if cos < CONE_COS {
            continue;
        }
        let metric = len * o.weight * (1.0 - cos);
        if metric < best {
            best = metric;
            let dir = diff.normalize_or(car.forward);
            let lateral = right.dot(diff) * (o.right.dot(right).abs() * 0.5 + 0.5);
            obstacle = [car.forward.dot(diff), car.up.dot(diff), dir.dot(car.forward * v - o.velocity), lateral];
            obstacle_speed = o.velocity.dot(car.forward);
        }
    }
    let leader_mode = leader.map(|(i, _)| sys.car(i).mode);
    let waiting = matches!(car.mode, 0 | 2) || (car.mode == 4 && matches!(leader_mode, Some(0 | 2)));
    let own = [
        p.impatience_personality * car.personality + car.wait,
        if waiting { 1.0 } else { 0.0 },
        if car.mode == 1 { p.mode1_bonus } else { 0.0 },
    ];
    let s = scores(&sys.data.fuzzy, obstacle, [stop, gap, closing, obstacle_speed], own);
    let margin = (obstacle[0] - p.obstacle_margin).max(0.0);
    let follow = |other: f32, g: f32| cruise.min(v.min(other).max(p.gap_speed * g));
    let (mode, target_speed, target_dist) = match choose(s) {
        // MODEL: option 0 starts a manoeuvre (mode 5 and a request in the 5-entry queue at world+0x32bac);
        // the manoeuvres are not ported, so a car that would start one keeps stopping for its obstacle.
        0 if obstacle[0] < f32::MAX => (0, 0.0, margin),
        0 => (5, cruise, f32::MAX),
        1 => (0, 0.0, margin),
        2 => (2, follow(obstacle_speed, gap), margin),
        3 => (3, 0.0, stop - p.personality_stop_margin * car.personality),
        4 => (4, follow(leader.map_or(f32::MAX, |(i, _)| sys.car(i).speed), gap), gap),
        _ => (5, cruise, f32::MAX),
    };
    Decision { mode, target_speed, target_dist, leader: leader.map(|(i, _)| i) }
}

/// Spawning at the emitters (`FUN_0060e5e0`).
fn emit(sys: &mut TrafficSystem, dt: f32, player: Option<Vec3>) {
    for k in 0..sys.emitters.len() {
        sys.emitters[k].timer -= dt;
        if sys.emitters[k].timer > 0.0 {
            continue;
        }
        let (hull, section, timer) = (sys.emitters[k].hull, sys.emitters[k].section, sys.emitters[k].timer);
        let Some(s) = sys.section(hull, section) else { continue };
        let (flow_type, length) = (s.flow_type, s.length);
        let pick = sys.pick_vehicle(flow_type);
        let speed = sys.cruise(hull, section);
        // Where the car would be had it spawned on time.
        let dist = (speed * -timer).min(length);
        let clear = match sys.last_on(hull, section) {
            None => true,
            // The code subtracts the last car's back extent here (`FUN_0060e5e0`).
            Some(i) => {
                let o = sys.car(i);
                o.dist - o.back - dist >= speed * sys.data.params.spawn_headway
            }
        };
        if clear && let Some(pick) = pick {
            sys.place(hull, section, dist, pick, player);
        }
        let next = sys.interval(hull, section);
        sys.emitters[k].timer += next;
    }
}

/// Speed and position update of one car (`FUN_00614c20`, `FUN_0061a190`, `FUN_0061a450`, `FUN_0061a550`).
fn move_car(sys: &mut TrafficSystem, slot: usize, dt: f32, commands: &mut Commands) {
    let p = sys.data.params.clone();
    let car = sys.car(slot).clone();
    // Waiting time (`FUN_00614c20`): grows while stopped for an obstacle with no moving traffic directly ahead.
    let mut wait = car.wait;
    if matches!(car.mode, 5 | 1 | 3) || car.speed >= WAIT_SPEED {
        wait = 0.0;
    } else {
        let ahead_is_traffic = sys
            .by_section
            .get(&(car.hull, car.section))
            .and_then(|l| l.iter().position(|&i| i == slot).and_then(|k| l.get(k + 1)))
            .is_some_and(|&i| matches!(sys.car(i).mode, 0 | 2 | 4));
        if ahead_is_traffic {
            wait = 0.0;
        } else if matches!(car.mode, 0 | 2) {
            wait += dt;
        }
    }
    // The controller (`FUN_0061a190`).
    let mut cruise = sys.cruise(car.hull, car.section);
    if let Some(dir) = sys.direction_ahead(&car, car.speed * CORNER_LOOK_TIME + CORNER_LOOK_BASE)
        && car.forward.dot(dir) <= CORNER_COS
    {
        cruise = cruise.clamp(0.0, CORNER_SPEED);
    }
    let accel = match car.mode {
        5 => (cruise.clamp(0.0, CRUISE_CAP) - car.speed).clamp(p.min_accel_cruise, p.max_accel),
        0..=4 => {
            if car.target_dist < p.min_brake_distance {
                p.min_accel
            } else {
                let target = car.target_speed.max(0.0).min(cruise);
                ((target * target - car.speed * car.speed) / (car.target_dist * 2.0))
                    .clamp(p.min_accel, p.max_accel_brake_law)
            }
        }
        _ => 0.0,
    };
    // Integration (`FUN_0061a450`).
    let mut speed = (car.speed + dt * accel).max(0.0);
    if accel <= 0.0 && speed < p.stop_speed {
        speed = 0.0;
    }
    let braking = !(accel >= BRAKE_ACCEL && (speed >= BRAKE_SPEED || accel > 0.0));
    // The advance (`FUN_0061a550`): distance covered this tick, or the stopping distance when shorter.
    let mut travel = accel * 0.5 * dt * dt + dt * car.speed;
    if accel < 0.0 {
        let stopping = -(car.speed * car.speed) / (accel * 2.0);
        if stopping >= 0.0 && stopping < travel {
            travel = stopping;
        }
    }
    let travel = travel.max(0.0);
    let mut c = car;
    c.wait = wait;
    c.prev_speed = c.speed;
    c.speed = speed;
    c.accel = accel;
    c.braking = braking;
    c.dist += travel;
    let mut replan = false;
    loop {
        let len = sys.section_length(c.hull, c.section);
        if c.dist < len {
            break;
        }
        let next = match c.path.pop_front() {
            Some(n) => Some(n),
            None => {
                let counts = sys.span_counts();
                sys.choose_next(c.hull, c.section, &counts)
            }
        };
        let Some((h, s)) = next else {
            remove_car(sys, slot, commands);
            return;
        };
        if sys.active_set.get(h as usize) != Some(&true) {
            remove_car(sys, slot, commands);
            return;
        }
        c.dist -= len;
        c.hull = h;
        c.section = s;
        replan = true;
    }
    // The pose: lane point, heading smoothed towards the rung segment (`FUN_006132d0`).
    if let Some((pos, dir, across)) = sys.lane_pose(c.hull, c.section, c.dist) {
        // MODEL: the 0.4 blend is per 60 Hz tick in the game; scaled here for the fixed step.
        let k = 1.0 - (1.0 - HEADING_BLEND).powf(dt * 60.0);
        c.forward = c.forward.lerp(dir, k).normalize_or(dir);
        c.up = up_from(c.forward, across);
        c.position = pos;
    }
    sys.cars[slot] = Some(c);
    if replan {
        sys.plan(slot);
    }
}

/// Junction light cycles (`FUN_00605d30`): the phase advances when its timer runs out; each light group's
/// stop lines are held while the phase's mask has the group's bit.
fn cycle_lights(sys: &mut TrafficSystem, dt: f32) {
    let data = sys.data.clone();
    let active = sys.active.clone();
    let mut held: Vec<(u16, u8, bool)> = Vec::new();
    for h in active {
        let hull = &data.lanes.hulls[h as usize];
        let state = sys.hull_state(h);
        for (j, junction) in hull.junctions.iter().enumerate() {
            let Some(js) = state.junctions.get_mut(j) else { continue };
            js.timer -= dt;
            if js.timer > 0.0 || junction.durations.is_empty() {
                continue;
            }
            js.phase = (js.phase + 1) % junction.durations.len();
            js.timer += junction.durations[js.phase];
            let mask = junction.masks.get(js.phase).copied().unwrap_or(0);
            for (g, group) in junction.groups.iter().enumerate() {
                let red = mask & (1 << g) != 0;
                for &(sh, sl) in &group.stop_lines {
                    held.push((sh, sl, red));
                }
            }
        }
    }
    for (h, line, red) in held {
        if let Some(s) = sys.hulls.get_mut(&h)
            && let Some(x) = s.held.get_mut(line as usize)
        {
            *x = red;
        }
    }
}

/// The load budget (`FUN_005fd6a0`).
fn budget(sys: &mut TrafficSystem, focus: Vec3, dt: f32, commands: &mut Commands) {
    let s = &sys.data.system;
    let mut load = 0.0;
    let mut farthest: Option<(f32, Option<usize>, Option<Entity>)> = None;
    for (i, c) in sys.cars.iter().enumerate() {
        let Some(c) = c else { continue };
        let d = c.position.distance(focus);
        if d > LOAD_RADIUS {
            continue;
        }
        load += if c.speed < s.slow_speed { s.load_slow } else { s.load_moving };
        if farthest.is_none_or(|(f, _, _)| d > f) {
            farthest = Some((d, Some(i), None));
        }
    }
    for c in &sys.crashed {
        let d = c.position.distance(focus);
        if d > LOAD_RADIUS {
            continue;
        }
        load += s.load_physical;
        if farthest.is_none_or(|(f, _, _)| d > f) {
            farthest = Some((d, None, Some(c.entity)));
        }
    }
    sys.load = load;
    if load < LOAD_BUDGET {
        return;
    }
    sys.cull_timer -= dt;
    if sys.cull_timer > 0.0 {
        return;
    }
    sys.cull_timer = s.cull_cooldown;
    match farthest {
        Some((_, Some(i), _)) => remove_car(sys, i, commands),
        Some((_, None, Some(e))) => {
            sys.crashed.retain(|c| c.entity != e);
            commands.entity(e).despawn();
            sys.removed += 1;
        }
        _ => {}
    }
}

fn remove_car(sys: &mut TrafficSystem, slot: usize, commands: &mut Commands) {
    if let Some(c) = sys.cars[slot].take() {
        if let Some(e) = c.entity {
            commands.entity(e).despawn();
        }
        sys.removed += 1;
    }
}

/// Request models for cars that need one and build the entities of cars whose model has arrived.
fn spawn_entities(
    mut commands: Commands,
    system: Option<ResMut<TrafficSystem>>,
    mut models: ResMut<TrafficModels>,
    requests: Option<Res<LoaderRequests>>,
    player: PlayerQuery,
    camera: Query<&GlobalTransform, crate::MainCamera>,
) {
    let Some(mut sys) = system else { return };
    let sys = &mut *sys;
    let focus = match player.iter().next() {
        Some((t, _)) => t.translation,
        None => match camera.iter().next() {
            Some(t) => t.translation(),
            None => return,
        },
    };
    for slot in 0..MAX_CARS {
        let Some(car) = sys.cars[slot].as_ref() else { continue };
        let d = car.position.distance(focus);
        if let Some(e) = car.entity {
            if d > DRAW_RANGE + 50.0 {
                commands.entity(e).despawn();
                sys.cars[slot].as_mut().expect("live car").entity = None;
            }
            continue;
        }
        if d > DRAW_RANGE {
            continue;
        }
        let key = (car.vehicle, car.paint);
        let Some(model) = models.models.get(&key).cloned() else {
            if models.requested.insert(key)
                && let Some(req) = &requests
            {
                let v = &sys.data.vehicles[car.vehicle];
                let _ = req.0.send(CarRequest {
                    bundle: v.bundle.clone(),
                    paint_index: 0,
                    traffic: Some(TrafficModelRequest {
                        vehicle: car.vehicle,
                        paint: car.paint,
                        colour: v.paints.get(car.paint).map(|(_, p)| *p),
                    }),
                    opponent: None,
                });
            }
            continue;
        };
        let v = &sys.data.vehicles[car.vehicle];
        let rotation = Quat::from_mat3(&Mat3::from_cols(car.up.cross(car.forward), car.up, car.forward));
        let transform = Transform::from_translation(car.position).with_rotation(rotation);
        let body = commands
            .spawn((
                Name::new("traffic car"),
                TrafficCar { vehicle: car.vehicle },
                transform,
                Visibility::default(),
                RigidBody::Kinematic,
                TransformInterpolation,
            ))
            .id();
        // MODEL: the physical box (`RaceCarPhysicalDefinition`) extended down to the ground plane of the car
        // (its wheels): a crashed car has no suspension here (the game's `FUN_005f7cf0` wheel poses are not
        // ported), so the box stands in for the wheels.
        let top = v.box_centre[1] + v.half_extents[1];
        let size = Vec3::new(v.half_extents[0] * 2.0, top, v.half_extents[2] * 2.0);
        commands.spawn((
            Collider::cuboid(size.x, size.y, size.z),
            Transform::from_xyz(v.box_centre[0], top * 0.5, v.box_centre[2]),
            traffic_layers(false),
            // The collider carries the car's mass, so a crashed car gets its box's inertia.
            ColliderDensity(v.mass / (size.x * size.y * size.z).max(0.1)),
            TrafficBody { slot, serial: car.serial },
            ActiveCollisionHooks::MODIFY_CONTACTS,
            Friction::new(CRASH_FRICTION),
            Restitution::new(0.0),
            ChildOf(body),
        ));
        for (mesh, mat) in &model.body {
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d::<GameMaterial>(mat.clone()),
                crate::shadow_cull::ShadowClass::Car,
                ChildOf(body),
            ));
        }
        for (w, parts) in &model.wheels {
            let pivot =
                commands.spawn((Transform::from_translation(w.position), Visibility::default(), ChildOf(body))).id();
            let mirror = if w.mirror { Vec3::new(-1.0, 1.0, 1.0) } else { Vec3::ONE };
            // MODEL: wheels spin at speed / radius with the radius taken as the wheel's height (the
            // kinematic wheel pose `FUN_005f7cf0` was not read).
            let spinner = commands
                .spawn((
                    Transform::from_rotation(w.rotation).with_scale(mirror),
                    Visibility::default(),
                    TrafficWheel { radius: w.position.y.max(0.2), left: w.mirror },
                    ChildOf(pivot),
                ))
                .id();
            let fixed = commands
                .spawn((Transform::from_rotation(w.rotation).with_scale(mirror), Visibility::default(), ChildOf(pivot)))
                .id();
            for (parts, spins) in parts {
                for (mesh, mat) in parts {
                    commands.spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d::<GameMaterial>(mat.clone()),
                        crate::shadow_cull::ShadowClass::Car,
                        ChildOf(if *spins { spinner } else { fixed }),
                    ));
                }
            }
        }
        if let Some(c) = sys.cars[slot].as_mut() {
            c.entity = Some(body);
        }
    }
}

/// Collision layers of a traffic collider: lane followers meet only dynamic things (the player, props,
/// crashed cars); crashed cars also meet the world.
fn traffic_layers(crashed: bool) -> CollisionLayers {
    use crate::props::GameLayer;
    if crashed {
        CollisionLayers::new(
            GameLayer::Traffic,
            [
                GameLayer::World,
                GameLayer::Car,
                GameLayer::CarBumper,
                GameLayer::Prop,
                GameLayer::Traffic,
                GameLayer::LooseProp,
            ],
        )
    } else {
        CollisionLayers::new(
            GameLayer::Traffic,
            [GameLayer::Car, GameLayer::Prop, GameLayer::Traffic, GameLayer::LooseProp],
        )
    }
}

/// Move the lane followers' kinematic bodies to their lane pose by velocity (so contacts see the motion),
/// and spin the wheels.
fn drive_bodies(
    system: Option<Res<TrafficSystem>>,
    time: Res<Time>,
    mut bodies: Query<(&Position, &Rotation, &mut LinearVelocity, &mut AngularVelocity, &RigidBody), With<TrafficCar>>,
    mut wheels: Query<(&mut Transform, &TrafficWheel, &ChildOf)>,
    pivots: Query<&ChildOf, Without<TrafficWheel>>,
) {
    let Some(sys) = system else { return };
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let mut speeds: HashMap<Entity, f32> = HashMap::new();
    for c in sys.cars.iter().flatten() {
        let Some(e) = c.entity else { continue };
        let Ok((pos, rot, mut lin, mut ang, rb)) = bodies.get_mut(e) else { continue };
        if !rb.is_kinematic() {
            continue;
        }
        let target = Quat::from_mat3(&Mat3::from_cols(c.up.cross(c.forward), c.up, c.forward));
        lin.0 = (c.position - pos.0) / dt;
        let delta = target * rot.0.inverse();
        let (axis, mut angle) = delta.to_axis_angle();
        if angle > std::f32::consts::PI {
            angle -= std::f32::consts::TAU;
        }
        ang.0 = if axis.is_finite() { axis * angle / dt } else { Vec3::ZERO };
        speeds.insert(e, c.speed);
    }
    for (mut t, w, parent) in &mut wheels {
        let Ok(pivot_parent) = pivots.get(parent.parent()) else { continue };
        let Some(&v) = speeds.get(&pivot_parent.parent()) else { continue };
        let spin = v * dt / w.radius * if w.left { 1.0 } else { -1.0 };
        t.rotate_local_x(spin);
    }
}

/// Lane followers that a dynamic body touched become crashed cars (dynamic rigid bodies with the car's mass,
/// keeping their lane velocity).
fn crash_hit_cars(
    mut commands: Commands,
    system: Option<ResMut<TrafficSystem>>,
    hits: Query<(Entity, &TrafficBody, &ChildOf), With<TrafficHit>>,
) {
    let Some(mut sys) = system else {
        return;
    };
    for (collider, body, parent) in &hits {
        commands.entity(collider).remove::<(TrafficHit, TrafficBody, ActiveCollisionHooks)>();
        let Some(car) = sys.cars.get(body.slot).and_then(|c| c.as_ref()) else { continue };
        if car.serial != body.serial || car.entity != Some(parent.parent()) {
            continue;
        }
        let car = sys.cars[body.slot].take().expect("checked above");
        let entity = parent.parent();
        let v = &sys.data.vehicles[car.vehicle];
        commands.entity(entity).insert((
            RigidBody::Dynamic,
            LinearVelocity(car.forward * car.speed),
            AngularVelocity(Vec3::ZERO),
        ));
        commands.entity(collider).insert(traffic_layers(true));
        info!("traffic: car {} ({}) hit at {:.0} km/h, now crashing", body.slot, v.bundle, car.speed * 3.6);
        sys.crashed.push(Crashed {
            entity,
            vehicle: car.vehicle,
            position: car.position,
            velocity: car.forward * car.speed,
            right: car.up.cross(car.forward),
        });
    }
}

/// A count line every 5 s and the cars near the player every second (`MW_TRAFFIC_LOG=1`), for headless checks.
fn count_log(
    system: Option<ResMut<TrafficSystem>>,
    time: Res<Time>,
    player: PlayerQuery,
    mut next: Local<f32>,
    mut next_near: Local<f32>,
) {
    let Some(sys) = system else { return };
    if std::env::var_os("MW_TRAFFIC_LOG").is_none() {
        return;
    }
    if time.elapsed_secs() >= *next_near
        && let Some((t, _)) = player.iter().next()
    {
        *next_near = time.elapsed_secs() + 1.0;
        let mut near = sys.cars_near(t.translation, 40.0);
        near.sort_by(|a, b| {
            a.position.distance_squared(t.translation).total_cmp(&b.position.distance_squared(t.translation))
        });
        let line: Vec<String> = near
            .iter()
            .take(4)
            .map(|c| {
                let mode = if c.crashed {
                    "crashed".to_string()
                } else {
                    let mode = sys.cars.iter().flatten().find(|l| l.entity.is_some() && l.entity == c.entity);
                    mode.map_or("-".to_string(), |l| format!("mode {}", l.mode))
                };
                format!(
                    "type {} at {:.1} m, ahead {:.1} m, {:.1} m/s, {mode}",
                    c.vehicle,
                    c.position.distance(t.translation),
                    (c.position - t.translation).dot(t.rotation * Vec3::Z),
                    c.velocity.length()
                )
            })
            .collect();
        if !line.is_empty() {
            info!("traffic near: {}", line.join("; "));
        }
    }
    if time.elapsed_secs() < *next {
        return;
    }
    *next = time.elapsed_secs() + 5.0;
    let tick_ms = 1000.0 * sys.tick_time / sys.ticks.max(1) as f64;
    let mut sys = sys;
    sys.tick_time = 0.0;
    sys.ticks = 0;
    let (lane, crashed) = sys.counts();
    let modes = sys.cars.iter().flatten().fold([0usize; 6], |mut m, c| {
        m[c.mode.min(5) as usize] += 1;
        m
    });
    info!(
        "traffic: {lane} cars ({crashed} crashed), tick {tick_ms:.2} ms, {} hulls active, {} emitters, load {:.0}, spawned {} removed {}, modes stop {} match {} stopline {} follow {} cruise {}",
        sys.active.len(),
        sys.emitters.len(),
        sys.load,
        sys.spawned,
        sys.removed,
        modes[0],
        modes[2],
        modes[3],
        modes[4],
        modes[5]
    );
}
