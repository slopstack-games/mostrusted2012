//! Traffic: the lane network (`TrafficLaneData`) plus the GeneSys data the game's traffic code reads, in
//! the units that code uses. What the code does with it is in docs/decomp.md ("Traffic and AI vehicles").
//!
//! | source | what |
//! |--------|------|
//! | `HAWAII/TRAFFIC.BNDL` `TrafficLaneData` | hulls, lane sections, rungs, spans, junctions, stop lines |
//! | `TrafficFlow` (one per section flow index) | `Speed` (0x24, mph), `Density` (0x20, cars per minute per lane at multiplier 1) |
//! | `TrafficFlowType` (section byte 0x25) | `FlowType[]`: `TrafficVehicle` handle (0x0c), `Colour` (0x14), `Proportion` (0x1c) |
//! | `TrafficVehicle` | `VehicleComponent` id: the car is `VEHICLES/VEH_<id>_TR.BNDL`; `Trailer` handle (0x20) |
//! | `VehicleComponent` (`VEHICLES/TRAFFICATTRIBS.BNDL`) | `Colour_Palette` → `VehicleColourPalette` paint handles (`16671e74`) |
//! | `RaceCarPhysicalDefinition` (id `0x01000002_<component>`) | box centre `62dbd683`, half extents `98e902ed`, body mass |
//! | `GLOBALCONFIG.BNDL` `TrafficSystemConfig` / `TrafficParamConfig` / `TrafficFuzzyLogicConfig` | the rules' constants ([`SystemConfig`], [`ParamConfig`], [`FuzzyConfig`]) |

use std::sync::Arc;

use mw_bundle::ResourceId;
use mw_formats as fmt;
use mw_formats::genesys::{Object, Value, field_hash};

use crate::paint::VehiclePaint;
use crate::{AssetStore, Error, Result};

/// mph → m/s (`DAT_010c4d24`, `.data`).
pub const MPH: f32 = 0.447_04;

const GLOBAL_CONFIG: &str = "GLOBALCONFIG.BNDL";

/// One `TrafficFlow`: the cruise speed and spawn density of the sections that name it.
#[derive(Debug, Clone, Copy, Default)]
pub struct TrafficFlow {
    pub speed_mph: f32,
    /// Cars per minute per lane at density multiplier 1 (`FUN_005fb120`).
    pub density: f32,
}

/// One entry of a `TrafficFlowType`.
#[derive(Debug, Clone, Copy)]
pub struct FlowEntry {
    /// Index into [`TrafficData::vehicles`].
    pub vehicle: usize,
    pub proportion: f32,
    /// A paint to use instead of the round-robin (`FUN_00608720` matches it against the palette); 0 = none.
    pub colour: u32,
}

/// A traffic car model with its paints and physical box.
#[derive(Debug, Clone)]
pub struct TrafficVehicleType {
    /// The `TrafficVehicle` object.
    pub vehicle: ResourceId,
    /// `VehicleComponent` id (decimal in the bundle name).
    pub component: u32,
    pub bundle: String,
    /// The palette's paints in order, with their ids (`FUN_00608720` cycles through them per type).
    pub paints: Vec<(ResourceId, VehiclePaint)>,
    /// `RaceCarPhysicalDefinition.PhysicalDefinition`: box centre and half extents (car space, +Z forward).
    /// `FUN_005f4d10` takes the car's front / back extents along the lane from their z.
    pub box_centre: [f32; 3],
    pub half_extents: [f32; 3],
    /// The chassis body's mass (kg).
    pub mass: f32,
    /// `TrafficVehicle.Trailer` is set: the back extent grows by 10 m (`DAT_00fb23c0`). None in the PC data.
    pub trailer: bool,
}

/// `TrafficSystemConfig` (`GLOBALCONFIG.BNDL` 0100000000069446); offsets are instance offsets as decomp.md's
/// `sys[x]` names them.
#[derive(Debug, Clone)]
pub struct SystemConfig {
    /// 0x0c: 24 hourly values (0.05 at night, 0.5 at 08:00). No reader found (decomp.md).
    pub hours: Vec<f32>,
    /// 0x80: the base density multiplier (4.78).
    pub density_base: f32,
    /// 0x84: the jitter of pre-placed cars, as a fraction of their spacing (0.3, `FUN_0060afb0`).
    pub prepopulate_jitter: f32,
    /// 0x88 / 0x90: the multiplier's factor at or below `slow_mph` / at or above `fast_mph` (1.2 / 1).
    pub slow_factor: f32,
    pub fast_factor: f32,
    /// 0x8c / 0x94: the player speeds of that ramp (60 / 20 mph).
    pub fast_mph: f32,
    pub slow_mph: f32,
    /// 0x98: the emitters' timer phase step (0.7).
    pub emitter_phase: f32,
    /// 0x9c / 0xa0 / 0xa4: load per physical / moving / slow car (30 / 20 / 5, `FUN_005fd6a0`).
    pub load_physical: f32,
    pub load_moving: f32,
    pub load_slow: f32,
    /// 0xa8: seconds between removals while over the load budget (0.5).
    pub cull_cooldown: f32,
    /// 0xb4: below this speed a car counts as slow (6).
    pub slow_speed: f32,
    /// 0xc8 / 0xb8: the spawn interval's random factor runs from the first to the second (1 / 1, `FUN_005fb120`).
    pub interval_min: f32,
    pub interval_max: f32,
    /// 0xd0: the density floor (0).
    pub min_density: f32,
    /// 0xd4: no car is placed within this distance of a player (50 m, `FUN_00608720`).
    pub spawn_clearance: f32,
    /// 0x10c `PursuitScale` (0.5).
    pub pursuit_scale: f32,
}

/// `TrafficParamConfig` (0100000000069444; `DAT_0110f940`, confirmed by a live read); `prm[x]` in decomp.md.
#[derive(Debug, Clone)]
pub struct ParamConfig {
    /// 0x10: standstill gap behind the car ahead (4 m).
    pub standstill_gap: f32,
    /// 0x34: the planned route reaches at least this far ahead (25 m, `FUN_005fc020`).
    pub plan_distance: f32,
    /// 0x3c: following speed per metre of gap (0.649 /s, `FUN_00617d00` branches 2 and 4).
    pub gap_speed: f32,
    /// 0x44: the acceleration cap of the braking law (8 m/s², `FUN_0061a190`).
    pub max_accel_brake_law: f32,
    /// 0x4c / 0x64: the cruise controller's acceleration limits (4 / −8 m/s²).
    pub max_accel: f32,
    pub min_accel_cruise: f32,
    /// 0x50 `MinAcceleration` (−20 m/s²).
    pub min_accel: f32,
    /// 0x5c: merging cars are looked for within this distance of the section end (14 m, `FUN_005f1380`).
    pub merge_distance: f32,
    /// 0x60 / 0x88: look-ahead `max(time × speed, distance)` (25 m, 2 s), × 1.5 (`DAT_00fb1bb0`).
    pub look_distance: f32,
    pub look_time: f32,
    /// 0x6c: spawn headway behind the last car on a section (2 s, `FUN_0060e5e0`).
    pub spawn_headway: f32,
    /// 0x70: a decelerating car below this speed stops (0.05 m/s, `FUN_0061a450`).
    pub stop_speed: f32,
    /// 0x74: a merging car stopped at a stop line slower than this is not a leader (5 m/s).
    pub merge_stopped_speed: f32,
    /// 0x78: below this distance to the target point the car brakes at `min_accel` (0.05 m).
    pub min_brake_distance: f32,
    /// 0x80: added to the manoeuvre score while in mode 1 (0.15).
    pub mode1_bonus: f32,
    /// 0x84: impatience from the personality (0.5 s per unit).
    pub impatience_personality: f32,
    /// 0x9c: stopping margin behind an obstacle (10 m).
    pub obstacle_margin: f32,
    /// 0xac: extra gap per unit of personality (3 m).
    pub personality_gap: f32,
    /// 0xb0: stop lines are seen within this distance (100.277 m, `FUN_005f0d90`).
    pub stop_line_range: f32,
    /// 0xb4: stop-line margin per unit of personality (4 m).
    pub personality_stop_margin: f32,
}

/// A fuzzy membership `(a, b, c, d)`: 0 below a, rising to 1 at b, 1 to c, falling to 0 at d.
#[derive(Debug, Clone, Copy, Default)]
pub struct Envelope(pub [f32; 4]);

impl Envelope {
    /// `FUN_005ea6a0`: `clamp(min((x − a)/(b − a), (d − x)/(d − c)), 0, 1)`.
    pub fn of(&self, x: f32) -> f32 {
        let [a, b, c, d] = self.0;
        let rise = (x - a) / (b - a).max(1e-6);
        let fall = (d - x) / (d - c).max(1e-6);
        rise.min(fall).clamp(0.0, 1.0)
    }
}

/// The envelopes `FUN_005ea6a0` reads from its SIMD block (`world+0x69810`), matched to the
/// `TrafficFuzzyLogicConfig` fields by value against a live dump of the block (research log 2026-10-09).
#[derive(Debug, Clone, Default)]
pub struct FuzzyConfig {
    /// Obstacle distance ahead: `07b6e570`, `DistanceNear`, `249c401d`, `3a5dd3ed`.
    pub obstacle_distance: [Envelope; 4],
    /// Obstacle height: `HeightNear`.
    pub obstacle_height: Envelope,
    /// Closing speed (obstacle and leader): `13ca8b67`, `01d18569`, `5483011b`, `0072dac3`.
    pub closing: [Envelope; 4],
    /// Obstacle lateral offset: `6eec31b8`, `87c8bc7e`.
    pub lateral: [Envelope; 2],
    /// Distance to a held stop line: `87f03da4`.
    pub stop_line: Envelope,
    /// Gap to the car ahead: `050a5e6e`.
    pub leader_gap: Envelope,
    /// Impatience: `3eddeb08`.
    pub impatience: Envelope,
    /// The cruise option's fixed score (block +0x510; 0.2 read live, its writer not found).
    pub cruise_score: f32,
}

/// Everything the traffic runtime needs.
#[derive(Debug, Clone)]
pub struct TrafficData {
    pub lanes: fmt::TrafficLaneData,
    /// By section flow index.
    pub flows: Vec<TrafficFlow>,
    /// By section flow-type index.
    pub flow_types: Vec<Vec<FlowEntry>>,
    pub vehicles: Vec<TrafficVehicleType>,
    pub system: SystemConfig,
    pub params: ParamConfig,
    pub fuzzy: FuzzyConfig,
}

fn num(v: &Value) -> Option<f32> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f32),
        Value::UInt(u) => Some(*u as f32),
        _ => None,
    }
}

fn float(o: &Object, hash: u32) -> f32 {
    o.get(hash).and_then(num).unwrap_or(0.0)
}

fn handle(v: Option<&Value>) -> Option<ResourceId> {
    match v? {
        Value::Handle(h) => *h,
        Value::Ref(r) => Some(*r),
        _ => None,
    }
}

fn vec3(o: &Object, hash: u32) -> Option<[f32; 3]> {
    match o.get(hash)? {
        Value::Vector(v) => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

fn envelope(o: &Object, hash: u32) -> Envelope {
    match o.get(hash) {
        Some(Value::Struct(e)) => {
            Envelope([float(e, 0x51b8_fc78), float(e, 0x93c8_1fa3), float(e, 0x2acd_4610), float(e, 0x6811_992f)])
        }
        _ => Envelope::default(),
    }
}

impl SystemConfig {
    fn read(o: &Object) -> Self {
        let f = |h| float(o, h);
        Self {
            hours: match o.get(0x9783_c32e) {
                Some(Value::Array(a)) => a.iter().filter_map(num).collect(),
                _ => Vec::new(),
            },
            density_base: f(0x809d_7518),
            prepopulate_jitter: f(0xa3a3_c338),
            slow_factor: f(0x56eb_b29d),
            fast_mph: f(0x5f85_2f64),
            fast_factor: f(0x56eb_ba8b),
            slow_mph: f(0xb7f9_0631),
            emitter_phase: f(0x52bb_ee0f),
            load_physical: f(0x50b1_aca8),
            load_moving: f(0x83dd_1214),
            load_slow: f(0x87a7_e037),
            cull_cooldown: f(0xc323_b819),
            slow_speed: f(0x80ea_ec72),
            interval_max: f(0x96d7_30c2),
            interval_min: f(0x6c7b_728e),
            min_density: f(0x0c46_4bf8),
            spawn_clearance: f(0x67cf_04e5),
            pursuit_scale: f(field_hash("PursuitScale")),
        }
    }
}

impl ParamConfig {
    fn read(o: &Object) -> Self {
        let f = |h| float(o, h);
        Self {
            standstill_gap: f(0xa5cd_60b8),
            plan_distance: f(0x29a8_0792),
            gap_speed: f(0x0ea8_45e0),
            max_accel_brake_law: f(0xb9be_788e),
            max_accel: f(0xfc10_89c3),
            min_accel_cruise: f(0xa3fd_8015),
            min_accel: f(field_hash("MinAcceleration")),
            merge_distance: f(0xfcee_e32b),
            look_distance: f(0xd5e5_e614),
            look_time: f(0xf90e_991e),
            spawn_headway: f(0xcafb_be9b),
            stop_speed: f(0x45f4_41ef),
            merge_stopped_speed: f(0x0ffd_a036),
            min_brake_distance: f(0x0a9f_19ec),
            mode1_bonus: f(0xac74_8164),
            impatience_personality: f(0x7abb_2945),
            obstacle_margin: f(0x8b58_f9e3),
            personality_gap: f(0xe055_1946),
            stop_line_range: f(0x4fb3_dd91),
            personality_stop_margin: f(0x1395_1366),
        }
    }
}

impl FuzzyConfig {
    fn read(o: &Object) -> Self {
        let e = |h| envelope(o, h);
        Self {
            obstacle_distance: [e(0x07b6_e570), e(field_hash("DistanceNear")), e(0x249c_401d), e(0x3a5d_d3ed)],
            obstacle_height: e(field_hash("HeightNear")),
            closing: [e(0x13ca_8b67), e(0x01d1_8569), e(0x5483_011b), e(0x0072_dac3)],
            lateral: [e(0x6eec_31b8), e(0x87c8_bc7e)],
            stop_line: e(0x87f0_3da4),
            leader_gap: e(0x050a_5e6e),
            impatience: e(0x3edd_eb08),
            cruise_score: 0.2,
        }
    }
}

impl AssetStore {
    fn global_config(&self, type_name: &str) -> Result<Object> {
        let b = self.bundle(GLOBAL_CONFIG).ok_or(Error::Missing(ResourceId::NULL))?;
        for e in b.entries_of_type(mw_bundle::ResourceType::GenesysObject) {
            if let Ok(o) = self.decode_genesys(&b.load(e)?)
                && o.type_name == type_name
            {
                return Ok(o);
            }
        }
        Err(Error::Missing(ResourceId::NULL))
    }

    /// The lane network and every traffic rule constant, decoded once (a few hundred ms).
    pub fn traffic_data(&self) -> Result<Arc<TrafficData>> {
        let lanes = self.traffic_lanes()?;
        let flows = lanes
            .flows
            .iter()
            .map(|id| {
                let Some(o) = id.and_then(|id| self.genesys_object(id).ok()) else { return TrafficFlow::default() };
                TrafficFlow { speed_mph: float(&o, field_hash("Speed")), density: float(&o, field_hash("Density")) }
            })
            .collect();
        let mut vehicles: Vec<TrafficVehicleType> = Vec::new();
        let mut flow_types = Vec::new();
        for id in &lanes.flow_types {
            let mut entries = Vec::new();
            if let Some(o) = id.and_then(|id| self.genesys_object(id).ok())
                && let Some(Value::Array(items)) = o.get(field_hash("FlowType"))
            {
                for item in items {
                    let Value::Struct(s) = item else { continue };
                    let Some(vid) = handle(s.get(0xa5f0_54ae)) else { continue };
                    let index = match vehicles.iter().position(|v| v.vehicle == vid) {
                        Some(i) => i,
                        None => match self.traffic_vehicle(vid) {
                            Ok(v) => {
                                vehicles.push(v);
                                vehicles.len() - 1
                            }
                            Err(_) => continue,
                        },
                    };
                    let colour = match s.get(0x3460_a645) {
                        Some(Value::Int(i)) => *i as u32,
                        Some(Value::UInt(u)) => *u as u32,
                        _ => 0,
                    };
                    entries.push(FlowEntry { vehicle: index, proportion: float(s, field_hash("Proportion")), colour });
                }
            }
            flow_types.push(entries);
        }
        Ok(Arc::new(TrafficData {
            lanes,
            flows,
            flow_types,
            vehicles,
            system: SystemConfig::read(&self.global_config("Genesys.Gen.TrafficSystemConfig")?),
            params: ParamConfig::read(&self.global_config("Genesys.Gen.TrafficParamConfig")?),
            fuzzy: FuzzyConfig::read(&self.global_config("Genesys.Gen.TrafficFuzzyLogicConfig")?),
        }))
    }

    fn traffic_vehicle(&self, id: ResourceId) -> Result<TrafficVehicleType> {
        let o = self.genesys_object(id)?;
        let component = float(&o, field_hash("VehicleComponent")) as u32;
        let bundle = format!("VEHICLES/VEH_{component}_TR.BNDL");
        if self.bundle(&bundle).is_none() {
            return Err(Error::Missing(id));
        }
        // The palette: VehicleComponent (low id = the component) → Colour_Palette → paint handles.
        let mut paints = Vec::new();
        if let Ok(vc) = self.genesys_object(ResourceId(0x0100_0000_0000_0000 | component as u64))
            && let Some(pal) = handle(vc.get(field_hash("Colour_Palette")))
            && let Ok(pal) = self.genesys_object(pal)
            && let Some(Value::Array(items)) = pal.get(0x1667_1e74)
        {
            for item in items {
                if let Some(pid) = handle(Some(item))
                    && let Ok(Some(p)) = self.vehicle_paint(pid)
                {
                    paints.push((pid, p));
                }
            }
        }
        // Box and mass from the physical definition (its id carries the component in the low bits).
        let (mut box_centre, mut half_extents, mut mass) = ([0.0, 0.6, 0.0], [0.9, 0.6, 2.2], 1200.0);
        if let Ok(rcpd) = self.genesys_object(ResourceId(0x0100_0002_0000_0000 | component as u64))
            && let Some(Value::Struct(def)) = rcpd.get(field_hash("PhysicalDefinition"))
        {
            box_centre = vec3(def, 0x62db_d683).unwrap_or(box_centre);
            half_extents = vec3(def, 0x98e9_02ed).unwrap_or(half_extents);
            if let Some(Value::Array(bodies)) = def.get(field_hash("RigidBodies"))
                && let Some(Value::Struct(b)) = bodies.first()
            {
                mass = b.get(field_hash("Mass")).and_then(num).unwrap_or(mass);
            }
        }
        Ok(TrafficVehicleType {
            vehicle: id,
            component,
            bundle,
            paints,
            box_centre,
            half_extents,
            mass,
            trailer: handle(o.get(field_hash("Trailer"))).is_some(),
        })
    }
}
