//! Police and pursuit data (docs/decomp.md "Police and pursuits").
//!
//! - `GameLogicConfig` (`GLOBALCONFIG.BNDL` 0100000000045e6f, `GlobalConfig` +0x24): the bust rule's distance,
//!   speed and time, the cop radar's range, the list of free-roam heat levels (+0xa0, 7 ids) and the ids of the
//!   lists in `GAMELOGIC/GAMEPLAY.BNDL` that hold every `HeatLevel` (+0xf8) and `CopType` (+0xf0).
//! - `HeatLevel` (0x118 bytes, 40 of them): one row per heat level of a game mode; the seven free-roam ones are
//!   levels 0–6 with heat thresholds 0 / 2 / 8 / 16 / 26 / 38 / 52.
//! - `Gameplay.CopType` → `AIPlayerType` → `Rollout` → `Vehicle`: which car a cop of a heat level drives.
//! - `ScoringAction.1a88cb4c` (+0x24): the heat a scoring action adds.
//!
//! Field offsets are instance offsets as the game's code reads them (`mwtool genesys-types`).

use std::collections::HashMap;

use mw_bundle::ResourceId;
use mw_formats::genesys::{Object, Value, field_hash};

use crate::events::{GAMEPLAY, items};
use crate::{AssetStore, Result};

/// `GameLogicConfig` in `GLOBALCONFIG.BNDL`.
const GAME_LOGIC_CONFIG: ResourceId = ResourceId(0x0100_0000_0004_5e6f);
/// `ScoringConfig` in `GLOBALCONFIG.BNDL` (`GlobalConfig` +0x44).
const SCORING_CONFIG: ResourceId = ResourceId(0x0100_0000_0005_f734);

mod h {
    use super::field_hash;
    pub const ID: u32 = 0x655d_3f5f;
    // GameLogicConfig
    pub const RADAR_RANGE: u32 = 0xf0e2_06de; // +0x218
    pub const BUST_DISTANCE: u32 = 0xd38d_aa84; // +0x288
    pub const BUST_SPEED: u32 = 0x35d8_9591; // +0x28c
    pub const BUST_TIME: u32 = 0x3297_c582; // +0x290
    pub const FREE_ROAM_LEVELS: u32 = 0xab4e_53fd; // +0xa0
    // ScoringConfig
    pub const CHASE_STARTED: u32 = 0xd2f3_74fe; // +0x50
    pub const CHASE_RESUMED: u32 = 0x88d9_920e; // +0x58
    // HeatLevel
    pub const COORDINATION: u32 = field_hash("Coordination"); // +0x0c
    pub const LEVEL_UP_ACTION: u32 = 0xa504_e53b; // +0x6c
    pub const HELICOPTER: u32 = field_hash("Helicopter"); // +0x74
    pub const COOLDOWN_TIME: u32 = 0xc422_193a; // +0x8c
    pub const LOSE_RADIUS: u32 = 0xed9f_5c6a; // +0xdc
    pub const HEAT_TIME: u32 = 0x2d16_3b85; // +0x100
    pub const THRESHOLD: u32 = field_hash("Threshold"); // +0x10e
    pub const COOLDOWN_ALLOWED: u32 = 0x249e_d427; // +0x110
    pub const LEVEL: u32 = 0xb226_6b6e; // +0x116
    pub const REMOVE_CHASE: u32 = 0xb315_1902; // +0x7c
    pub const SPAWN_CHASE: u32 = 0xb408_d33d; // +0x80
    pub const REMOVE_COOLDOWN: u32 = 0x4fa2_2cdc; // +0x84
    pub const SPAWN_COOLDOWN: u32 = 0xbf89_6b50; // +0x88
    pub const NEAR_RANGE: u32 = 0x12e5_7a71; // +0x94
    pub const CONE_ANGLE: u32 = 0x12f3_9403; // +0x9c
    pub const RANGE_CHASING: u32 = 0x527e_8830; // +0xa4
    pub const RANGE_IDLE: u32 = 0xee00_24df; // +0xa8
    pub const MEMORY: u32 = 0xac12_feb2; // +0xbc
    pub const REMOVE_FREE: u32 = 0x415b_8b74; // +0xd0
    pub const SPAWN_FREE: u32 = 0x7aa0_635a; // +0xd4
    pub const SPEEDING: u32 = 0x78f4_72fa; // +0xd8
    pub const SEARCH_SPEED: u32 = 0xe6da_b8d2; // +0x104
    pub const SPAWN_SPEED: u32 = field_hash("SpawnSpeed"); // AIPlayerType +0x6c
    pub const SPAWN_ALLOWED: u32 = 0xf3d7_aeeb; // PursuitBehaviour +0x22
    // HeatLevel.BehaviourCoordination
    pub const FALLBACK: u32 = 0xe8cb_6438; // +0x0c
    pub const MAIN: u32 = 0xdd65_94c8; // +0x30
    pub const LOW_SPEED: u32 = 0x7a65_b100; // +0x54
    // HeatLevel.BehaviourCoordination.PursuitBehaviour
    pub const ROADBLOCK: u32 = field_hash("Roadblock");
    pub const COP_BEHAVIOURS: u32 = field_hash("CopBehaviours");
    pub const COP_COUNT: u32 = 0x5ec3_75f3;
    // ...PursuitBehaviour.CopBehaviour
    pub const COP_TYPE: u32 = 0x10a8_0aa8;
    pub const COP_SPEED: u32 = 0xa762_0186;
    pub const COP_BEHAVIOUR: u32 = field_hash("CopBehaviour");
    pub const POSITIONING: u32 = field_hash("Positioning");
    // Gameplay.CopType / AIPlayerType / Rollout / ScoringAction
    pub const AI_PLAYER_TYPE: u32 = field_hash("AIPlayerType");
    pub const ROLLOUT: u32 = field_hash("Rollout");
    pub const VEHICLE: u32 = field_hash("Vehicle");
    pub const TAKEDOWN_ACTION: u32 = 0x23f2_735d;
    pub const ACTION_HEAT: u32 = 0x1a88_cb4c; // +0x24
}

/// One cop of a pursuit behaviour (`HeatLevel.BehaviourCoordination.PursuitBehaviour.CopBehaviour`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CopSlot {
    /// `10a80aa8`: the `Gameplay.CopType` id.
    pub cop_type: u32,
    /// `a7620186`: the cop's speed limit (mph; 110–130 on the free-roam levels).
    pub speed_mph: f32,
    /// `CopBehaviour` enum (0, 2, 6 on the free-roam levels).
    pub behaviour: i32,
    /// `Positioning` (−2 … 1).
    pub positioning: i32,
}

/// A `PursuitBehaviour`: the cops a heat level fields and their roadblock.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Behaviour {
    /// `Roadblock`: the roadblock definition's id (0 = none).
    pub roadblock: u32,
    /// `5ec375f3`: the number of cops (the game spawns one per `CopBehaviours` entry, `FUN_00526e30`).
    pub cop_count: u32,
    /// `f3d7aeeb` (+0x22): missing cops are spawned while this behaviour runs.
    pub spawn_allowed: bool,
    pub cops: Vec<CopSlot>,
}

/// A `HeatLevel` row.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeatLevel {
    pub id: u32,
    /// `b2266b6e` (+0x116): the level number the HUD shows (0–6).
    pub level: u8,
    /// `Threshold` (+0x10e): the heat value at which this level ends (its upper bound).
    pub threshold: f32,
    /// `2d163b85` (+0x100): seconds; while chased the heat rises by `threshold / heat_time` per second
    /// (`FUN_00657e60`), not at all when ≤ 0.
    pub heat_time: f32,
    /// `c422193a` (+0x8c): seconds the cooldown takes to cool through this level's heat band (`FUN_006615c0`).
    pub cooldown_time: f32,
    /// `ed9f5c6a` (+0xdc): metres from where the cops last saw the player beyond which the chase turns into the
    /// cooldown (`FUN_0065bca0`); twice it is the pursuit circle on the map (`FUN_0065be70`).
    pub lose_radius: f32,
    /// `249ed427` (+0x110): the level has a cooldown (`FUN_00656140`).
    pub cooldown_allowed: bool,
    /// `a504e53b` (+0x6c): the scoring action posted when the level is first reached (`FUN_0065c5e0`).
    pub level_up_action: u32,
    /// `Helicopter` (+0x74): 0 on every level in the install.
    pub helicopter: u32,
    /// `Coordination.dd6594c8` (+0x30 of the coordination block): the behaviour the coordinator's sequencer
    /// starts with, and `.e8cb6438` (+0x0c): the one it falls back to (`FUN_0051f010`).
    pub main: Behaviour,
    pub fallback: Behaviour,
    /// Cops are spawned this far from the player (m; `FUN_00513e80`): `7aa0635a` (+0xd4) with no pursuit,
    /// `b408d33d` (+0x80) in a chase, `bf896b50` (+0x88) in the cooldown.
    pub spawn_distance: [f32; 3],
    /// Cops farther than this are removed (m; `FUN_00522c40` / `FUN_00522dc0`): `415b8b74` (+0xd0),
    /// `b3151902` (+0x7c), `4fa22cdc` (+0x84), in the same order.
    pub remove_distance: [f32; 3],
    /// A cop's sight (`FUN_006c5810`): range of a chasing cop (`527e8830`, +0xa4) and of any other
    /// (`ee0024df`, +0xa8), the range within which the cone is not needed (`12e57a71`, +0x94) and the cone's
    /// half angle in degrees (`12f39403`, +0x9c).
    pub sight_chasing: f32,
    pub sight_idle: f32,
    pub sight_near: f32,
    pub sight_cone_deg: f32,
    /// `ac12feb2` (+0xbc): seconds a cop remembers a player it no longer sees (`FUN_006c6280`).
    pub memory: f32,
    /// `78f472fa` (+0xd8): a player seen driving faster than this (mph) is wanted (`FUN_005192b0`).
    pub speeding_mph: f32,
    /// `Coordination.7a65b100` (+0x54 of the coordination block): below this player speed (mph) nearby cops
    /// switch to the low-speed chase (`FUN_005240a0`).
    pub low_speed_mph: f32,
    /// `e6dab8d2` (+0x104): a cop's speed limit while it drives to a point or searches (mph, `FUN_004f76c0`).
    pub search_speed_mph: f32,
}

/// What the pursuit rules read.
#[derive(Debug, Clone, Default)]
pub struct PoliceConfig {
    /// `GameLogicConfig.f0e206de` (+0x218, 200): range of the cop radar (m; `FUN_0065d470`).
    pub radar_range: f32,
    /// `GameLogicConfig.d38daa84` (+0x288, 20): a cop this close can bust (m; `FUN_0065cda0`).
    pub bust_distance: f32,
    /// `GameLogicConfig.35d89591` (+0x28c, 25): the bust rule's speed (mph).
    pub bust_speed_mph: f32,
    /// `GameLogicConfig.3297c582` (+0x290, 4): seconds the bust meter takes to fill.
    pub bust_time: f32,
    /// The free-roam heat levels in order (`GameLogicConfig.ab4e53fd`, +0xa0).
    pub levels: Vec<HeatLevel>,
    /// `Gameplay.CopType` id → `VehicleList` id of the car (`AIPlayerType.Rollout` → `Rollout.Vehicle`).
    pub cop_vehicles: HashMap<u32, u32>,
    /// `Gameplay.CopType` id → the speed a cop of that type is spawned with (mph; `AIPlayerType.SpawnSpeed`,
    /// +0x6c, `FUN_005220b0`).
    pub cop_spawn_speed: HashMap<u32, f32>,
    /// `Gameplay.CopType` id → the heat its takedown's scoring action adds (`AIPlayerType.23f2735d`).
    pub takedown_heat: HashMap<u32, f32>,
    /// Heat of the "CHASE INITIATED" scoring action (`ScoringConfig` +0x50; the variants at +0x48, +0x4c and
    /// +0x54 hold the same 0.5) and of +0x58, posted when a cooldown turns back into a chase.
    pub chase_started_heat: f32,
    pub chase_resumed_heat: f32,
}

fn uint(o: &Object, hash: u32) -> u32 {
    match o.get(hash) {
        Some(Value::UInt(u)) => *u as u32,
        Some(Value::Int(i)) => *i as u32,
        Some(Value::Float(f)) => *f as u32,
        _ => 0,
    }
}

fn int(o: &Object, hash: u32) -> i32 {
    match o.get(hash) {
        Some(Value::UInt(u)) => *u as i32,
        Some(Value::Int(i)) => *i as i32,
        Some(Value::Float(f)) => *f as i32,
        _ => 0,
    }
}

fn float(o: &Object, hash: u32) -> f32 {
    match o.get(hash) {
        Some(Value::Float(f)) => *f,
        Some(Value::UInt(u)) => *u as f32,
        Some(Value::Int(i)) => *i as f32,
        _ => 0.0,
    }
}

fn boolean(o: &Object, hash: u32) -> bool {
    matches!(o.get(hash), Some(Value::Bool(true)))
}

fn structs(v: Option<&Value>) -> Vec<&Object> {
    match v {
        Some(Value::Array(a)) => {
            a.iter().filter_map(|v| if let Value::Struct(o) = v { Some(o) } else { None }).collect()
        }
        Some(Value::Struct(o)) => vec![o],
        _ => Vec::new(),
    }
}

fn behaviour(o: Option<&Object>) -> Behaviour {
    let Some(o) = o else { return Behaviour::default() };
    Behaviour {
        roadblock: uint(o, h::ROADBLOCK),
        cop_count: uint(o, h::COP_COUNT),
        spawn_allowed: boolean(o, h::SPAWN_ALLOWED),
        cops: structs(o.get(h::COP_BEHAVIOURS))
            .into_iter()
            .map(|c| CopSlot {
                cop_type: uint(c, h::COP_TYPE),
                speed_mph: float(c, h::COP_SPEED),
                behaviour: int(c, h::COP_BEHAVIOUR),
                positioning: int(c, h::POSITIONING),
            })
            .collect(),
    }
}

fn heat_level(o: &Object) -> HeatLevel {
    let coordination = structs(o.get(h::COORDINATION)).into_iter().next();
    let sub = |hash: u32| coordination.and_then(|c| structs(c.get(hash)).into_iter().next());
    HeatLevel {
        id: uint(o, h::ID),
        level: uint(o, h::LEVEL) as u8,
        threshold: float(o, h::THRESHOLD),
        heat_time: float(o, h::HEAT_TIME),
        cooldown_time: float(o, h::COOLDOWN_TIME),
        lose_radius: float(o, h::LOSE_RADIUS),
        cooldown_allowed: boolean(o, h::COOLDOWN_ALLOWED),
        level_up_action: uint(o, h::LEVEL_UP_ACTION),
        helicopter: uint(o, h::HELICOPTER),
        main: behaviour(sub(h::MAIN)),
        fallback: behaviour(sub(h::FALLBACK)),
        spawn_distance: [float(o, h::SPAWN_FREE), float(o, h::SPAWN_CHASE), float(o, h::SPAWN_COOLDOWN)],
        remove_distance: [float(o, h::REMOVE_FREE), float(o, h::REMOVE_CHASE), float(o, h::REMOVE_COOLDOWN)],
        sight_chasing: float(o, h::RANGE_CHASING),
        sight_idle: float(o, h::RANGE_IDLE),
        sight_near: float(o, h::NEAR_RANGE),
        sight_cone_deg: float(o, h::CONE_ANGLE),
        memory: float(o, h::MEMORY),
        speeding_mph: float(o, h::SPEEDING),
        search_speed_mph: float(o, h::SEARCH_SPEED),
        low_speed_mph: coordination.map_or(0.0, |c| float(c, h::LOW_SPEED)),
    }
}

impl AssetStore {
    /// The pursuit rules' data: `GameLogicConfig`, the free-roam heat levels and the cop types' cars.
    pub fn police_config(&self) -> Result<PoliceConfig> {
        let logic = self.genesys_object(GAME_LOGIC_CONFIG)?;
        let mut cfg = PoliceConfig {
            radar_range: float(&logic, h::RADAR_RANGE),
            bust_distance: float(&logic, h::BUST_DISTANCE),
            bust_speed_mph: float(&logic, h::BUST_SPEED),
            bust_time: float(&logic, h::BUST_TIME),
            ..Default::default()
        };
        let level_ids: Vec<u32> = match logic.get(h::FREE_ROAM_LEVELS) {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| match v {
                    Value::UInt(u) => Some(*u as u32),
                    Value::Int(i) => Some(*i as u32),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let mut levels = HashMap::new();
        let (mut cop_types, mut player_types, mut rollouts, mut action_heat) =
            (Vec::new(), HashMap::new(), HashMap::new(), HashMap::new());
        for o in items(self, GAMEPLAY)? {
            match o.type_name.as_str() {
                "Genesys.Gen.HeatLevel" => {
                    levels.insert(uint(&o, h::ID), heat_level(&o));
                }
                "Genesys.Gen.Gameplay.CopType" => cop_types.push((uint(&o, h::ID), uint(&o, h::AI_PLAYER_TYPE))),
                "Genesys.Gen.AIPlayerType" => {
                    player_types.insert(
                        uint(&o, h::ID),
                        (uint(&o, h::ROLLOUT), uint(&o, h::TAKEDOWN_ACTION), float(&o, h::SPAWN_SPEED)),
                    );
                }
                "Genesys.Gen.Rollout" => {
                    rollouts.insert(uint(&o, h::ID), uint(&o, h::VEHICLE));
                }
                "Genesys.Gen.ScoringAction" => {
                    action_heat.insert(uint(&o, h::ID), float(&o, h::ACTION_HEAT));
                }
                _ => {}
            }
        }
        cfg.levels = level_ids.iter().filter_map(|id| levels.get(id).cloned()).collect();
        for (id, player_type) in cop_types {
            let Some(&(rollout, takedown, spawn_speed)) = player_types.get(&player_type) else { continue };
            cfg.cop_spawn_speed.insert(id, spawn_speed);
            if let Some(&vehicle) = rollouts.get(&rollout) {
                cfg.cop_vehicles.insert(id, vehicle);
            }
            if let Some(&heat) = action_heat.get(&takedown) {
                cfg.takedown_heat.insert(id, heat);
            }
        }
        if let Ok(scoring) = self.genesys_object(SCORING_CONFIG) {
            let heat = |hash: u32| action_heat.get(&uint(&scoring, hash)).copied().unwrap_or(0.0);
            cfg.chase_started_heat = heat(h::CHASE_STARTED);
            cfg.chase_resumed_heat = heat(h::CHASE_RESUMED);
        }
        Ok(cfg)
    }
}
