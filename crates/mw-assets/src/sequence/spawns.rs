//! The sequence items that put cars, cops, traffic and objects into a cutscene, and the gameplay switches
//! (docs/decomp.md "Sequence spawn items"). Each is read here with the fields its handler reads (the item
//! data's offsets are the `Genesys.Gen.<type>` field offsets, `mwtool genesys-types`).
//!
//! | type (factory) | handler methods (vtable) | what it does |
//! |------|------|------|
//! | `AddRaceCarSequenceItem` (0x00860a60) | lead `0x0085edd0` (2 s), prepare `FUN_00855730`, enter `FUN_00850d80`, stop `FUN_00853da0` | a car with behaviours (clips) for the item's span |
//! | `SpawnAICopCar` (0x00860c40) | enter `0x00850380`, update `FUN_00852e40`, stop `FUN_00852de0` | an AI cop on a trigger, rolling, with a command |
//! | `GameplaySequenceItem` (0x00860a00) | enter `FUN_00854990`, stop `FUN_00851b20` | a gameplay switch by `TriggerType` |
//! | `SpawnAnimatedTrafficSequenceItem` (0x00860e80) | update `FUN_008532c0`, stop `FUN_008503a0` | a traffic car on a clip |
//! | `OverrideAvailableTrafficSequenceItem` (0x00860f40) | enter `FUN_00851f00`, stop `FUN_00850050` | traffic picks only these vehicles |
//! | `SpawnTrafficSequenceItem` (0x00860ee0) | enter `FUN_00853660` | traffic cars on triggers (not in the event sequences) |
//! | `SpawnDynamicEntitySequenceItem` (0x00860d00) | lead `0x0085eec0`, prepare `FUN_008534f0`, enter `FUN_00855520`, stop `FUN_008503e0` | a world object with behaviours |
//! | `SetPlayerDamageSequenceItem` (0x00860dc0) | enter `FUN_00855110` | the player's car's damage per zone |

use mw_bundle::ResourceId;
use mw_formats::genesys::{Object, Value};
use mw_formats::{Trigger, TriggerShape};

use super::{AnimatedCar, RacerRef, SequenceContext, bone_keys, boolean, float, handle, short, string, uint, vec3};

/// Field hashes (the dump's names where `mwtool genesys-crack` found one).
mod f {
    // AddRaceCarSequenceItem (60 B)
    pub const CAR_BINDING: u32 = 0x3cc8_4bc1; // +0x24 StringBase
    pub const VEHICLE_COMPONENT: u32 = 0x711e_1a79; // +0x2c (+0x40 traffic, +0x38 SpawnTraffic)
    pub const VEHICLE_PAINT: u32 = 0xee5f_a173; // +0x30
    pub const BEHAVIOURS: u32 = 0xb48b_c50d; // +0x34 handles, count fd413ea0 (+0x3b)
    pub const ADD_RACE_CAR_38: u32 = 0x96fe_0093; // +0x38 bool, not read by the handler's methods
    pub const SUSPEND: u32 = 0x824d_41fe; // +0x39 bool
    pub const EXISTING: u32 = 0x8f98_df6b; // +0x3a bool
    // SpawnAICopCar (128 B)
    pub const DIRECTION_BINDING: u32 = 0xa787_8fc3; // +0x50
    pub const POSITION_BINDING: u32 = 0xa4d5_18db; // +0x58 (+0x30 SpawnTraffic, an array there)
    pub const NAME: u32 = 0x7006_926e; // +0x60
    pub const AI_TYPE: u32 = 0x5abe_8d6b; // +0x68
    pub const SPAWN_TRIGGER: u32 = 0x5bff_13c3; // +0x6c
    pub const SPEED_MPH: u32 = 0x77be_ef06; // +0x70
    pub const INSTANCE: u32 = 0xedfe_446a; // +0x74 AIPlayerInstance*
    pub const COMMAND: u32 = 0xc169_955f; // +0x78 u16
    pub const RELEASE: u32 = 0xaa2a_f3d8; // +0x7a bool
    pub const INSTANCE_ALT_COLOUR: u32 = 0x9b4e_35f7; // AIPlayerInstance +0x0c
    pub const INSTANCE_COLOUR: u32 = 0x2968_e77a; // +0x14
    pub const INSTANCE_TYPE: u32 = 0x5479_7065; // +0x18 "Type"
    // GameplaySequenceItem (44 B)
    pub const DATA: u32 = 0x4461_7461; // +0x24 "Data"
    pub const TRIGGER_TYPE: u32 = 0x24da_9f30; // +0x28 u16
    // SpawnAnimatedTrafficSequenceItem (72 B)
    pub const ANIMATION: u32 = 0x1769_3e5e; // +0x30
    pub const SKELETON: u32 = 0x351c_850f; // +0x38
    pub const TRAFFIC_FLAG: u32 = 0xd19e_b48d; // +0x44 bool
    // OverrideAvailableTrafficSequenceItem (44 B)
    pub const VEHICLES: u32 = 0x933a_1ec6; // +0x24, count d2834b37 (+0x28)
    // SpawnTrafficSequenceItem (72 B)
    pub const TRIGGERS: u32 = 0x1c15_d5cf; // +0x34
    // SpawnDynamicEntitySequenceItem (96 B)
    pub const INITIAL_POSITION: u32 = 0x50d0_fa53; // +0x30
    pub const INITIAL_ROTATION: u32 = 0xfd0d_4148; // +0x40
    pub const WORLD_COMPONENT: u32 = 0x8fa2_eba8; // +0x50
    // SetPlayerDamageSequenceItem (76 B)
    pub const DAMAGE_TARGET: u32 = 0xbeae_015c; // +0x24
    pub const DAMAGE: [u32; 8] = [
        0x5e7c_d650, // +0x28 FrontDamage
        0x6704_9b36, // +0x2c
        0x131a_d6e1, // +0x30
        0xa112_6074, // +0x34
        0x8cb2_fbea, // +0x38 RearDamage
        0x64f6_10c7, // +0x3c
        0xecd2_0138, // +0x40
        0x42ed_49ca, // +0x44
    ];
    pub const DAMAGE_48: u32 = 0x244b_3c18; // +0x48 bool
    pub const DAMAGE_49: u32 = 0xa59f_a767; // +0x49 bool
}

/// `AddRaceCarSequenceItem`: a car for the item's span, driven by its behaviours.
///
/// Prepare (`FUN_00855730`, from 2 s before the start: lead time `.rdata` 0x00f8a808): the vehicle and colour are
/// `VehicleComponent` / `VehiclePaint`, or, when the `3cc84bc1` binding (`Players.LocalPlayer.RaceCarIndex`,
/// `Players.Racer[n].RaceCarIndex`) is set, the bound car's vehicle and colour (`FUN_00850df0`: `FUN_00573670`,
/// `FUN_0058f1d0`). With `8f98df6b` the car comes from the sequence queue's message 4 (`FUN_00853ef0`), else a
/// new car is spawned (`FUN_00853f40` → `FUN_0066f8c0`, type 0xb, at the origin) and owned by the item. The
/// item waits until the car is in the world (`FUN_005733b0`); with `824d41fe` it posts message 0x36 (flag 0).
/// Enter (`FUN_00850d80`): every behaviour is added to the car (`FUN_00844c60`). Stop (`FUN_00853da0`): the
/// behaviours are removed (`FUN_00844cb0`); an owned car is released (`FUN_005a0ed0` / `FUN_0066f950`, message
/// 1), a borrowed one gets message 5 and, with `824d41fe`, message 0x36 (flag 1).
#[derive(Debug, Clone)]
pub struct AddRaceCar {
    pub car_binding: String,
    /// The bound racer when `car_binding` names one.
    pub racer: RacerRef,
    /// `VEHICLES/VEH_<id>_HI.BNDL` (0 when the binding gives the car).
    pub vehicle: u32,
    pub paint: u32,
    /// `8f98df6b`: the car comes from the queue (message 4), not a new spawn.
    pub existing: bool,
    /// `824d41fe`: message 0x36 around the item (flag 0 at the start, 1 at the end; meaning not traced).
    pub suspend: bool,
    /// `96fe0093` (not read by the item's methods).
    pub flag_38: bool,
    /// The `AnimatedRaceCarBehaviour`s (clips on the race-car skeleton) among `Behaviours`.
    pub clips: Vec<AnimatedCar>,
    /// Behaviours of other types (type names).
    pub other_behaviours: Vec<String>,
}

/// `SpawnAICopCar`'s command (+0x78) → the AI behaviour command it sets on the spawned car (`FUN_00852e40` →
/// `FUN_006b8a00`, the car's +0x580; names from the table at `.data` 0x010c4360).
pub fn cop_command(value: u16) -> (u8, &'static str) {
    match value {
        1 => (9, "Chasing"),
        2 => (0x16, "LowSpeedChasing"),
        3 => (3, "FreeRoam"),
        4 => (0x10, "WaitingInterceptor"),
        _ => (8, "IdleLights"),
    }
}

/// `SpawnAICopCar` (`FUN_00852e40`, once, on the first update in range): the AI type is the instance's `Type`
/// (with its colours) or else +0x68 (`FUN_006e6140`: the AI-type list by id), else the player manager's default;
/// the place is the `SpawnTrigger`'s point and direction (`FUN_0053c020`, `FUN_0042f250`) or, without one, the
/// `PositionBinding` / direction binding (or `Position` / `Direction`; a zero position is no spawn); the car
/// spawns (`FUN_006ce030`, the racers' spawn) with velocity `Speed` mph along the direction, gets the name hash of
/// +0x60 (`FUN_006b8a20`) and the command [`cop_command`]. Stop (`FUN_00852de0`): with `aa2af3d8` set the car is
/// released (`FUN_006ce180`(slot, 3)) once `FUN_006b8840` allows it.
#[derive(Debug, Clone)]
pub struct SpawnCop {
    pub ai_type: u32,
    /// The `AIPlayerType`'s `Rollout.Vehicle`, when it resolves.
    pub vehicle: Option<u32>,
    /// The instance's colour and the alternative used when it equals the player's (`FUN_006ce030`).
    pub colour: u32,
    pub alt_colour: u32,
    pub trigger: u32,
    /// The spawn point and unit direction: the trigger's, else `Position` / `Direction`.
    pub position: [f32; 3],
    pub direction: [f32; 3],
    /// The trigger resolved (else `position` is the item's own, maybe zero).
    pub resolved: bool,
    pub position_binding: String,
    pub direction_binding: String,
    pub name: String,
    pub speed_mph: f32,
    /// +0x78 (see [`cop_command`]).
    pub command: u16,
    /// `aa2af3d8`: release the car when the item stops.
    pub release: bool,
}

/// `SpawnAnimatedTrafficSequenceItem` (`FUN_008532c0`, every update): the clip is sampled at the item's local
/// time (clamped to the clip) and the skeleton's bone 1 placed as a traffic car of `VehicleComponent`
/// (`FUN_00845ff0`, first update) or moved (`FUN_008460c0`, with bone 2 when the skeleton has more than two bones);
/// stop (`FUN_008503a0` → `FUN_00846080`) removes it.
#[derive(Debug, Clone)]
pub struct AnimatedTraffic {
    /// Traffic `VehicleComponent` id (`VEHICLES/VEH_<id>_TR.BNDL`).
    pub vehicle: u32,
    pub animation: Option<ResourceId>,
    pub skeleton: Option<ResourceId>,
    pub payload: u64,
    /// `d19eb48d` (+0x44), passed to the traffic system with the pose (not traced).
    pub flag: bool,
    /// Bone 1 per frame, world space.
    pub keys: Vec<super::CameraKey>,
    pub frame_rate: f32,
}

impl AnimatedTraffic {
    pub fn key_at(&self, t: f32) -> Option<super::CameraKey> {
        clip_key(&self.keys, self.frame_rate, t)
    }
}

/// `SpawnTrafficSequenceItem` (`FUN_00853660`; only in free-roam sequences).
#[derive(Debug, Clone)]
pub struct SpawnTraffic {
    pub vehicle: u32,
    pub triggers: Vec<u32>,
    /// The triggers' points (position, direction) that resolve.
    pub points: Vec<([f32; 3], [f32; 3])>,
    pub payload: u64,
}

/// `SpawnDynamicEntitySequenceItem`: a world object (`WorldComponent`) at `InitialPosition` / `InitialRotation`
/// with its behaviours. Lead 2 s (`0x0085eec0` → `.rdata` 0x00f8a808); prepare `FUN_008534f0` posts world message 0
/// (`FUN_00843d30` → `FUN_0084c380`: the component's id (+0x50), the matrix of the rotation (+0x40) and position
/// (+0x30), zero velocity) and is ready once the request's status (+0x18) is 3, keeping the entity's handle (+0x20
/// → +0x28); enter `FUN_00855520` posts message 8 {item, behaviour id, handle} per `Behaviours` entry (+0x58, count
/// +0x5c); stop `FUN_008503e0` posts message 3 (`FUN_00843e60`: the entity goes).
#[derive(Debug, Clone)]
pub struct DynamicEntity {
    pub position: [f32; 3],
    /// Quaternion (x, y, z, w).
    pub rotation: [f32; 4],
    pub world_component: Option<ResourceId>,
    pub behaviours: Vec<ResourceId>,
    /// The first `AnimatedDynamicEntityBehaviour`'s clip (`Animation` on `Skeleton`): its first animated bone per
    /// frame, world space, and the clip's frame rate.
    pub keys: Vec<super::CameraKey>,
    pub frame_rate: f32,
    /// The behaviours' other types (not carried out).
    pub other_behaviours: Vec<String>,
}

impl DynamicEntity {
    pub fn key_at(&self, t: f32) -> Option<super::CameraKey> {
        clip_key(&self.keys, self.frame_rate, t)
    }
}

/// `SetPlayerDamageSequenceItem` (`FUN_00855110`): the eight values (percent, × 0.01 `.rdata` 0x00f9d474) are posted
/// with sequence message 0x23 in the order +0x28, +0x38, +0x34, +0x44, +0x2c, +0x3c, +0x30, +0x40, after the
/// overall level max(all) × the car's +3000.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerDamage {
    /// `beae015c` (+0x24).
    pub target: u32,
    /// The eight values in field order (+0x28 `FrontDamage` … +0x38 `RearDamage` … +0x44), percent.
    pub fields: [f32; 8],
    pub flags: [bool; 2],
}

impl PlayerDamage {
    /// The values in the order the handler posts them (message 0x23), 0..1.
    pub fn posted(&self) -> [f32; 8] {
        [0, 4, 3, 7, 1, 5, 2, 6].map(|k| self.fields[k] * 0.01)
    }
}

fn clip_key(keys: &[super::CameraKey], frame_rate: f32, t: f32) -> Option<super::CameraKey> {
    let car =
        AnimatedCar { racer: RacerRef::Other, behaviour: None, label: String::new(), keys: keys.to_vec(), frame_rate };
    car.key_at(t)
}

/// A trigger's point and direction (`FUN_0053c020` record, `FUN_0042f250`: a point's vector, a box's local Z,
/// else (1, 0, 0)).
pub(super) fn trigger_point(t: &Trigger) -> ([f32; 3], [f32; 3]) {
    let dir = match t.shape {
        TriggerShape::Point { direction } => direction,
        TriggerShape::Box { rotation: [x, y, z, w], .. } => {
            [2.0 * (x * z + w * y), 2.0 * (y * z - w * x), 1.0 - 2.0 * (x * x + y * y)]
        }
        TriggerShape::Sphere { .. } => [1.0, 0.0, 0.0],
    };
    (t.position, dir)
}

fn uints(o: &Object, hash: u32) -> Vec<u32> {
    match o.get(hash) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| match v {
                Value::UInt(u) => Some(*u as u32),
                Value::Int(i) => Some(*i as u32),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn handles(o: &Object, hash: u32) -> Vec<ResourceId> {
    match o.get(hash) {
        Some(Value::Array(a)) => a.iter().filter_map(|v| handle(Some(v))).collect(),
        v => handle(v).into_iter().collect(),
    }
}

fn raw_u64(o: &Object, hash: u32) -> u64 {
    match o.get(hash) {
        Some(Value::Raw(b)) if b.len() >= 8 => u64::from_le_bytes(b[..8].try_into().unwrap_or_default()),
        Some(Value::UInt(u)) => *u,
        _ => 0,
    }
}

fn quat(v: Option<&Value>) -> Option<[f32; 4]> {
    match v? {
        Value::Vector(v) => Some(*v),
        _ => None,
    }
}

impl SequenceContext<'_> {
    /// `AIPlayerType` id → `Rollout.Vehicle` (GAMEPLAY.BNDL), read once.
    fn ai_type_vehicle(&self, ai_type: u32) -> Option<u32> {
        let map = self.ai_vehicles.get_or_init(|| {
            let mut types = std::collections::HashMap::new();
            let mut rollouts = std::collections::HashMap::new();
            for o in crate::events::items(self.store, crate::events::GAMEPLAY).unwrap_or_default() {
                match short(&o) {
                    "AIPlayerType" => {
                        types.insert(uint(&o, super::h::ID), uint(&o, mw_formats::genesys::field_hash("Rollout")));
                    }
                    "Rollout" => {
                        rollouts.insert(uint(&o, super::h::ID), uint(&o, mw_formats::genesys::field_hash("Vehicle")));
                    }
                    _ => {}
                }
            }
            types.into_iter().filter_map(|(t, r)| Some((t, *rollouts.get(&r)?))).collect()
        });
        map.get(&ai_type).copied()
    }

    /// An `AnimatedRaceCarBehaviour` handle as a clip.
    pub(super) fn animated_behaviour(&self, behaviour: ResourceId, racer: RacerRef) -> Result<AnimatedCar, String> {
        let b = self.store.genesys_object(behaviour).map_err(|e| e.to_string())?;
        if short(&b) != "AnimatedRaceCarBehaviour" {
            return Err(short(&b).to_string());
        }
        let animation = self.animation(handle(b.get(super::h::ANIMATION)));
        Ok(AnimatedCar {
            racer,
            behaviour: Some(behaviour),
            label: string(&b, mw_formats::genesys::field_hash("Label")),
            keys: animation.as_ref().map(bone_keys).unwrap_or_default(),
            frame_rate: animation.as_ref().map_or(30.0, |a| a.frame_rate),
        })
    }

    /// The spawn items, by short type name (`None` for other types).
    pub(super) fn spawn_item(&self, kind: &str, item: &Object) -> Option<super::ItemKind> {
        use super::ItemKind;
        Some(match kind {
            "AddRaceCarSequenceItem" => {
                let car_binding = string(item, f::CAR_BINDING);
                let (mut clips, mut other_behaviours) = (Vec::new(), Vec::new());
                let racer = RacerRef::parse(&car_binding);
                for b in handles(item, f::BEHAVIOURS) {
                    match self.animated_behaviour(b, racer) {
                        Ok(c) => clips.push(c),
                        Err(t) => other_behaviours.push(t),
                    }
                }
                ItemKind::AddRaceCar(Box::new(AddRaceCar {
                    racer: if car_binding.is_empty() { RacerRef::Other } else { racer },
                    car_binding,
                    vehicle: uint(item, f::VEHICLE_COMPONENT),
                    paint: uint(item, f::VEHICLE_PAINT),
                    existing: boolean(item, f::EXISTING).unwrap_or(false),
                    suspend: boolean(item, f::SUSPEND).unwrap_or(false),
                    flag_38: boolean(item, f::ADD_RACE_CAR_38).unwrap_or(false),
                    clips,
                    other_behaviours,
                }))
            }
            "SpawnAICopCar" => {
                let instance = match item.get(f::INSTANCE) {
                    Some(Value::Struct(s)) => Some(s),
                    _ => None,
                };
                let (ai_type, colour, alt_colour) = match instance {
                    Some(i) if uint(i, f::INSTANCE_TYPE) != 0 => {
                        (uint(i, f::INSTANCE_TYPE), uint(i, f::INSTANCE_COLOUR), uint(i, f::INSTANCE_ALT_COLOUR))
                    }
                    _ => (uint(item, f::AI_TYPE), 0, 0),
                };
                let trigger = uint(item, f::SPAWN_TRIGGER);
                let (position, direction, resolved) = match self.triggers.get(&trigger).filter(|_| trigger != 0) {
                    Some(t) => {
                        let (p, d) = trigger_point(t);
                        (p, d, true)
                    }
                    None => (
                        vec3(item.get(super::h::POSITION)).unwrap_or_default(),
                        vec3(item.get(super::h::DIRECTION)).unwrap_or([0.0, 0.0, 1.0]),
                        false,
                    ),
                };
                ItemKind::SpawnCop(Box::new(SpawnCop {
                    ai_type,
                    vehicle: self.ai_type_vehicle(ai_type),
                    colour,
                    alt_colour,
                    trigger,
                    position,
                    direction,
                    resolved,
                    position_binding: string(item, f::POSITION_BINDING),
                    direction_binding: string(item, f::DIRECTION_BINDING),
                    name: string(item, f::NAME),
                    speed_mph: float(item, f::SPEED_MPH).unwrap_or(0.0),
                    command: uint(item, f::COMMAND) as u16,
                    release: boolean(item, f::RELEASE).unwrap_or(false),
                }))
            }
            "GameplaySequenceItem" => {
                ItemKind::Gameplay { data: uint(item, f::DATA), trigger_type: uint(item, f::TRIGGER_TYPE) as u16 }
            }
            "SpawnAnimatedTrafficSequenceItem" => {
                let animation_id = handle(item.get(f::ANIMATION));
                let animation = self.animation(animation_id);
                ItemKind::AnimatedTraffic(Box::new(AnimatedTraffic {
                    vehicle: uint(item, f::VEHICLE_COMPONENT),
                    animation: animation_id,
                    skeleton: handle(item.get(f::SKELETON)),
                    payload: raw_u64(item, 0x3ed4_7c75),
                    flag: boolean(item, f::TRAFFIC_FLAG).unwrap_or(false),
                    keys: animation.as_ref().map(bone_keys).unwrap_or_default(),
                    frame_rate: animation.as_ref().map_or(30.0, |a| a.frame_rate),
                }))
            }
            "OverrideAvailableTrafficSequenceItem" => ItemKind::OverrideTraffic(uints(item, f::VEHICLES)),
            "SpawnTrafficSequenceItem" => {
                let triggers = uints(item, f::TRIGGERS);
                let points = triggers.iter().filter_map(|t| self.triggers.get(t)).map(trigger_point).collect();
                ItemKind::SpawnTraffic(Box::new(SpawnTraffic {
                    vehicle: uint(item, f::VEHICLE_COMPONENT),
                    triggers,
                    points,
                    payload: raw_u64(item, 0x3ed4_7c75),
                }))
            }
            "SpawnDynamicEntitySequenceItem" => {
                let behaviours = handles(item, f::BEHAVIOURS);
                let (mut animation, mut other_behaviours) = (None, Vec::new());
                for &b in &behaviours {
                    match self.store.genesys_object(b) {
                        Ok(o) if short(&o) == "AnimatedDynamicEntityBehaviour" => {
                            if animation.is_none() {
                                animation = self.animation(handle(o.get(super::h::ANIMATION)));
                            }
                        }
                        Ok(o) => other_behaviours.push(short(&o).to_string()),
                        Err(e) => other_behaviours.push(e.to_string()),
                    }
                }
                ItemKind::DynamicEntity(Box::new(DynamicEntity {
                    position: vec3(item.get(f::INITIAL_POSITION)).unwrap_or_default(),
                    rotation: quat(item.get(f::INITIAL_ROTATION)).unwrap_or([0.0, 0.0, 0.0, 1.0]),
                    world_component: handle(item.get(f::WORLD_COMPONENT)),
                    behaviours,
                    keys: animation.as_ref().map(bone_keys).unwrap_or_default(),
                    frame_rate: animation.as_ref().map_or(30.0, |a| a.frame_rate),
                    other_behaviours,
                }))
            }
            "SetPlayerDamageSequenceItem" => ItemKind::PlayerDamage(PlayerDamage {
                target: uint(item, f::DAMAGE_TARGET),
                fields: f::DAMAGE.map(|k| float(item, k).unwrap_or(0.0)),
                flags: [boolean(item, f::DAMAGE_48).unwrap_or(false), boolean(item, f::DAMAGE_49).unwrap_or(false)],
            }),
            _ => return None,
        })
    }
}
