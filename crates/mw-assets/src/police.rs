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
    pub const FORCED_NUMBER: u32 = 0xa53c_2545; // +0x113
    pub const LEVEL: u32 = 0xb226_6b6e; // +0x116
    pub const REMOVE_CHASE: u32 = 0xb315_1902; // +0x7c
    pub const SPAWN_CHASE: u32 = 0xb408_d33d; // +0x80
    pub const REMOVE_COOLDOWN: u32 = 0x4fa2_2cdc; // +0x84
    pub const SPAWN_COOLDOWN: u32 = 0xbf89_6b50; // +0x88
    pub const NEAR_RANGE_SLOW: u32 = 0x6aa1_36b6; // +0x90
    pub const NEAR_RANGE: u32 = 0x12e5_7a71; // +0x94
    pub const CONE_ANGLE_ALERT: u32 = 0x29f1_5999; // +0x98
    pub const CONE_ANGLE: u32 = 0x12f3_9403; // +0x9c
    pub const RANGE_ALERT: u32 = 0x66f5_e3f4; // +0xa0
    pub const RANGE_CHASING: u32 = 0x527e_8830; // +0xa4
    pub const RANGE_IDLE: u32 = 0xee00_24df; // +0xa8
    pub const MEMORY: u32 = 0xac12_feb2; // +0xbc
    pub const REMOVE_FREE: u32 = 0x415b_8b74; // +0xd0
    pub const SPAWN_FREE: u32 = 0x7aa0_635a; // +0xd4
    pub const SPEEDING: u32 = 0x78f4_72fa; // +0xd8
    pub const SEARCH_SPEED: u32 = 0xe6da_b8d2; // +0x104
    pub const SPAWN_SPEED: u32 = field_hash("SpawnSpeed"); // AIPlayerType +0x6c
    pub const SPAWN_ALLOWED: u32 = 0xf3d7_aeeb; // PursuitBehaviour +0x22
    pub const SURPLUS_REMOVE: u32 = 0x1076_9f53; // HeatLevel +0xac
    pub const CRASHED_REMOVE: u32 = 0x534e_9625; // HeatLevel +0xb8
    pub const GOTO_TIMEOUT: u32 = 0xbe25_ae81; // HeatLevel +0xc0
    // Gameplay.CopType +0x14 .. +0x19 (spawn kinds, `FUN_005261d0`)
    pub const COP_FLAGS: [u32; 6] = [0x1dda_314a, 0x5834_848c, 0x7b83_78db, 0x08b7_85d5, 0xe191_fc4c, 0x9c3e_6a66];
    // GameMode +0x5a / +0x60 / +0x62 / +0x64: spawn weights side road, ahead, behind, waiting
    pub const SPAWN_WEIGHTS: [u32; 4] = [0x3c0d_b900, 0xe616_3878, 0x85bf_10ab, 0x6022_63d2];
    // HeatLevel.BehaviourCoordination
    pub const FALLBACK: u32 = 0xe8cb_6438; // +0x0c
    pub const MAIN: u32 = 0xdd65_94c8; // +0x30
    pub const LOW_SPEED: u32 = 0x7a65_b100; // +0x54
    // HeatLevel.BehaviourCoordination.PursuitBehaviour
    pub const ROADBLOCK: u32 = field_hash("Roadblock");
    pub const COP_BEHAVIOURS: u32 = field_hash("CopBehaviours");
    pub const COP_COUNT: u32 = 0x5ec3_75f3;
    pub const ROADBLOCK_REPEAT: u32 = 0xcbbe_b3e0; // +0x10
    pub const ROADBLOCK_DELAY: u32 = 0xae2d_8f7d; // +0x14
    pub const DURATION: u32 = 0x4fea_2957; // +0x18
    pub const FALLBACK_AFTER: u32 = 0xf58d_1d54; // +0x23
    // HeatLevel.BehaviourCoordination +0x58 / +0x5c: the cycle behaviours
    pub const CYCLE: u32 = 0xb7dd_bf1a;
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
    /// `4fea2957` (+0x18): seconds the sequencer runs this behaviour (`FUN_00514e40`).
    pub duration: f32,
    /// `ae2d8f7d` (+0x14): seconds from the behaviour's start to its first roadblock; `cbbeb3e0` (+0x10): seconds
    /// from a roadblock to the next (`FUN_00526150`, `FUN_005246f0`).
    pub roadblock_delay: f32,
    pub roadblock_repeat: f32,
    /// `f58d1d54` (+0x23): the sequencer's event count after which the fallback behaviour runs (0 = never).
    pub fallback_after: u8,
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
    /// `a53c2545` (+0x113): when a sequence forced this level (`GameplaySequenceItem` type 1, G+0x2f414 +0xa8),
    /// `Player.HeatLevelInt` shows its number (`FUN_0066a7b0`); without it the number stays the heat's.
    pub forced_number: bool,
    /// `a504e53b` (+0x6c): the scoring action posted when the level is first reached (`FUN_0065c5e0`).
    pub level_up_action: u32,
    /// `Helicopter` (+0x74): 0 on every level in the install.
    pub helicopter: u32,
    /// `Coordination.dd6594c8` (+0x30 of the coordination block): the behaviour the coordinator's sequencer
    /// starts with, and `.e8cb6438` (+0x0c): the one it falls back to (`FUN_0051f010`).
    pub main: Behaviour,
    pub fallback: Behaviour,
    /// `Coordination.b7ddbf1a` (+0x58, count +0x5c): the behaviours the sequencer cycles through after the main one.
    pub cycle: Vec<Behaviour>,
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
    /// `66f5e3f4` (+0xa0): the range of an alerted cop that is not in a chasing state (`FUN_00516400` holds);
    /// `6aa136b6` (+0x90): the no-cone range for a target at 5 mph or less (+0x94 above, `.rdata` 0x00fa8538);
    /// `29f15999` (+0x98): the cone's half angle of an alerted cop (+0x9c otherwise; `FUN_006c05b0`).
    pub sight_alert: f32,
    pub sight_near_slow: f32,
    pub sight_cone_alert_deg: f32,
    /// `ac12feb2` (+0xbc): seconds a cop remembers a player it no longer sees (`FUN_006c6280`).
    pub memory: f32,
    /// `78f472fa` (+0xd8): a player seen driving faster than this (mph) is wanted (`FUN_005192b0`).
    pub speeding_mph: f32,
    /// `Coordination.7a65b100` (+0x54 of the coordination block): below this player speed (mph) nearby cops
    /// switch to the low-speed chase (`FUN_005240a0`).
    pub low_speed_mph: f32,
    /// `e6dab8d2` (+0x104): a cop's speed limit while it drives to a point or searches (mph, `FUN_004f76c0`).
    pub search_speed_mph: f32,
    /// `10769f53` (+0xac, 100 m): a cop the behaviour no longer wants is removed beyond this (off screen,
    /// `FUN_00522c40`).
    pub surplus_remove_distance: f32,
    /// `534e9625` (+0xb8, 125 m): a crashed cop off screen is removed this far away (`FUN_00522f40`).
    pub crashed_remove_distance: f32,
    /// `be25ae81` (+0xc0, 10 s): a cop sent to the last-seen point searches after this long (`FUN_0051a4e0`).
    pub goto_timeout: f32,
    /// `dd335ee5` (+0xb0) and `092b3408` (+0xb4): a race car that has finished its race is removed beyond the
    /// first distance when it is off screen, beyond the second in any case (m, horizontal; `FUN_00521f00`).
    pub finished_racer_remove: [f32; 2],
}

/// `Gameplay.CopType` +0x14 .. +0x19: which spawn kinds a cop type may use in a pursuit (`FUN_005261d0`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CopSpawnFlags {
    /// +0x14 `1dda314a`: ahead, on the player's route and facing the same way (mode 0x10); also enables "ahead".
    pub ahead_along: bool,
    /// +0x15 `5834848c`: behind the player.
    pub behind: bool,
    /// +0x16 `7b8378db`: ahead, oncoming.
    pub ahead: bool,
    /// +0x17 `08b785d5`: from a side road.
    pub side: bool,
    /// +0x18 `e191fc4c`: an extra waiting spawn first.
    pub extra_wait: bool,
    /// +0x19 `9c3e6a66`: a waiting (parked) cop.
    pub wait: bool,
}

/// The free-roam game mode ("Freedrive") in `GAMELOGIC/GAMEMODES.BNDL`.
const FREE_ROAM_GAME_MODE: u32 = 390145;
/// The scoring actions "SLAM" (posted in a chase or cooldown) and "TrafficHit".
const SLAM_ACTION: u32 = 955544;
const TRAFFIC_HIT_ACTION: u32 = 390556;

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
    /// Every `HeatLevel` by id (an event's own list, `OfflineEvent.ab4e53fd`, names rows from here).
    pub levels_by_id: HashMap<u32, HeatLevel>,
    /// The running offline event's own heat levels (`OfflineEvent.ab4e53fd` through `levels_by_id`; GMM+0x2ce0's
    /// event data, `FUN_006e66a0`): when not empty the heat's level is looked up here first (`FUN_00669640`,
    /// `FUN_00669860`). Set by the runtime while an event runs; empty in free roam.
    pub event_levels: Vec<HeatLevel>,
    /// The level a sequence forced (G+0x2f414 +0xa8: `GameplaySequenceItem` type 1, `FUN_006675e0`): it stands for
    /// the heat's level wherever the level row is read (`FUN_0066aa30`, `FUN_00669860`) until a later item sets
    /// another (an id with no row clears it). Never cleared otherwise (the setter has no other caller).
    pub forced_level: Option<HeatLevel>,
    /// `Gameplay.CopType` id → `VehicleList` id of the car (`AIPlayerType.Rollout` → `Rollout.Vehicle`).
    pub cop_vehicles: HashMap<u32, u32>,
    /// `AIPlayerType` id → the first `Gameplay.CopType` (data order) whose `AIPlayerType` it is: the cop type of an AI
    /// cop spawned by its AI type (`SpawnAICopCar`).
    pub cop_type_of_ai: HashMap<u32, u32>,
    /// `Gameplay.CopType` id → the speed a cop of that type is spawned with (mph; `AIPlayerType.SpawnSpeed`,
    /// +0x6c, `FUN_005220b0`).
    pub cop_spawn_speed: HashMap<u32, f32>,
    /// `Gameplay.CopType` id → the heat its takedown's scoring action adds (`AIPlayerType.23f2735d`).
    pub takedown_heat: HashMap<u32, f32>,
    /// Heat of the "CHASE INITIATED" scoring action (`ScoringConfig` +0x50; the variants at +0x48, +0x4c and
    /// +0x54 hold the same 0.5) and of +0x58, posted when a cooldown turns back into a chase.
    pub chase_started_heat: f32,
    pub chase_resumed_heat: f32,
    /// `Gameplay.CopType` id → its spawn kinds.
    pub cop_flags: HashMap<u32, CopSpawnFlags>,
    /// The free-roam `GameMode` (390145, "Freedrive") +0x5a / +0x60 / +0x62 / +0x64: weights of the side-road,
    /// ahead, behind and waiting spawns in a pursuit (`FUN_005261d0`; 10 / 10 / 10 / 0).
    pub spawn_weights: [f32; 4],
    /// Heat of the "SLAM" scoring action (955544, 0.2) and of "TrafficHit" (390556, 0.1).
    pub slam_heat: f32,
    pub traffic_hit_heat: f32,
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
        duration: float(o, h::DURATION),
        roadblock_delay: float(o, h::ROADBLOCK_DELAY),
        roadblock_repeat: float(o, h::ROADBLOCK_REPEAT),
        fallback_after: uint(o, h::FALLBACK_AFTER) as u8,
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
        forced_number: boolean(o, h::FORCED_NUMBER),
        level_up_action: uint(o, h::LEVEL_UP_ACTION),
        helicopter: uint(o, h::HELICOPTER),
        main: behaviour(sub(h::MAIN)),
        fallback: behaviour(sub(h::FALLBACK)),
        cycle: coordination
            .map(|c| structs(c.get(h::CYCLE)).into_iter().map(|b| behaviour(Some(b))).collect())
            .unwrap_or_default(),
        spawn_distance: [float(o, h::SPAWN_FREE), float(o, h::SPAWN_CHASE), float(o, h::SPAWN_COOLDOWN)],
        remove_distance: [float(o, h::REMOVE_FREE), float(o, h::REMOVE_CHASE), float(o, h::REMOVE_COOLDOWN)],
        sight_chasing: float(o, h::RANGE_CHASING),
        sight_idle: float(o, h::RANGE_IDLE),
        sight_near: float(o, h::NEAR_RANGE),
        sight_cone_deg: float(o, h::CONE_ANGLE),
        sight_alert: float(o, h::RANGE_ALERT),
        sight_near_slow: float(o, h::NEAR_RANGE_SLOW),
        sight_cone_alert_deg: float(o, h::CONE_ANGLE_ALERT),
        memory: float(o, h::MEMORY),
        speeding_mph: float(o, h::SPEEDING),
        search_speed_mph: float(o, h::SEARCH_SPEED),
        low_speed_mph: coordination.map_or(0.0, |c| float(c, h::LOW_SPEED)),
        surplus_remove_distance: float(o, h::SURPLUS_REMOVE),
        crashed_remove_distance: float(o, h::CRASHED_REMOVE),
        goto_timeout: float(o, h::GOTO_TIMEOUT),
        finished_racer_remove: [float(o, 0xdd33_5ee5), float(o, 0x092b_3408)],
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
                "Genesys.Gen.Gameplay.CopType" => {
                    let f = |k: usize| boolean(&o, h::COP_FLAGS[k]);
                    let flags = CopSpawnFlags {
                        ahead_along: f(0),
                        behind: f(1),
                        ahead: f(2),
                        side: f(3),
                        extra_wait: f(4),
                        wait: f(5),
                    };
                    cfg.cop_flags.insert(uint(&o, h::ID), flags);
                    cop_types.push((uint(&o, h::ID), uint(&o, h::AI_PLAYER_TYPE)))
                }
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
        cfg.levels_by_id = levels;
        for &(id, player_type) in &cop_types {
            cfg.cop_type_of_ai.entry(player_type).or_insert(id);
        }
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
        cfg.slam_heat = action_heat.get(&SLAM_ACTION).copied().unwrap_or(0.0);
        cfg.traffic_hit_heat = action_heat.get(&TRAFFIC_HIT_ACTION).copied().unwrap_or(0.0);
        if let Ok(modes) = items(self, "GAMELOGIC/GAMEMODES.BNDL")
            && let Some(m) = modes.iter().find(|o| uint(o, h::ID) == FREE_ROAM_GAME_MODE)
        {
            cfg.spawn_weights = h::SPAWN_WEIGHTS.map(|k| float(m, k));
        }
        if let Ok(scoring) = self.genesys_object(SCORING_CONFIG) {
            let heat = |hash: u32| action_heat.get(&uint(&scoring, hash)).copied().unwrap_or(0.0);
            cfg.chase_started_heat = heat(h::CHASE_STARTED);
            cfg.chase_resumed_heat = heat(h::CHASE_RESUMED);
        }
        Ok(cfg)
    }
}

/// Roadblock data (docs/decomp.md "Roadblocks"): the `RoadBlockDefinition`s a `PursuitBehaviour.Roadblock` names,
/// their `RoadBlockLayer`s (lists 604097 and 604098 in `GAMELOGIC/GAMEPLAY.BNDL`) and what each layer item is.
#[derive(Debug, Clone, Default)]
pub struct RoadblockData {
    pub definitions: HashMap<u32, RoadblockDefinition>,
    pub layers: HashMap<u32, RoadblockLayer>,
    /// What a layer item's id names (`FUN_00846c60`: a weapon-list entry → a world object, else an AI player).
    pub items: HashMap<u32, RoadblockItemKind>,
    /// `GameLogicConfig` +0x280 (`c1f9bb3f`, 500 m): a roadblock goes once the player is farther (`FUN_008414f0`;
    /// × 0.8 outside a pursuit).
    pub removal_distance: f32,
}

/// `Genesys.Gen.RoadBlockDefinition` (48 bytes; read by `FUN_00844ff0`).
#[derive(Debug, Clone, Default)]
pub struct RoadblockDefinition {
    pub id: u32,
    /// +0x1c (`0b27d7f6`): the layer at the roadblock's point.
    pub centre: u32,
    /// +0x10 (`c0308014`, count +0x2a): layers behind it (towards the player), one per `layer_distance`.
    pub back: Vec<u32>,
    /// +0x0c (`2ac6cc0c`, count +0x28): layers beyond it (empty in every definition).
    pub front: Vec<u32>,
    /// +0x24 `LayerDistance` (m).
    pub layer_distance: f32,
    /// +0x18 (`ba625679`, count +0x2c): character ids (not read by the layout).
    pub characters: Vec<u32>,
    /// +0x20 `Sequence`.
    pub sequence: u32,
}

/// `Genesys.Gen.RoadBlockLayer` (60 bytes; laid out by `FUN_00841710`): a row of items across the road.
#[derive(Debug, Clone, Default)]
pub struct RoadblockLayer {
    pub id: u32,
    /// +0x0c (`e74d0880`): the item at the row's centre.
    pub centre: RoadblockItem,
    /// +0x30 (`5b0d9447`, count +0x38): items towards −side, placed last to first.
    pub left: Vec<RoadblockItem>,
    /// +0x34 (`aa63128e`, count +0x3a): items towards +side, in order.
    pub right: Vec<RoadblockItem>,
    /// +0x28 `Distance` / +0x2c `FirstDistance` (m): the spacing, and the first item's offset from the centre.
    pub distance: f32,
    pub first_distance: f32,
}

/// `RoadBlockLayer.Item` (24 bytes): +0x10 (`23406ec7`) what to place (0 = a gap), +0x14 `Angle` (degrees).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RoadblockItem {
    pub object: u32,
    pub angle: f32,
}

/// What a roadblock item places.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoadblockItemKind {
    /// An `AIPlayerType` (a cop car, `FUN_00845270`): its `Rollout`'s vehicle id.
    Car { vehicle: u32 },
    /// A `PhysicalDefinition` placed as a dynamic world object (`FUN_00841390` → `FUN_006cb430`): the
    /// `WorldObject` of `HAWAII/GLOBALRESOURCES.BNDL` with that physics.
    Object { world_object: ResourceId },
    /// A `SpikeStripWeapon` (`FUN_006cb430` kind 2): not placed by the runtime.
    SpikeStrip,
}

mod rb {
    use super::field_hash;
    pub const CENTRE: u32 = 0x0b27_d7f6;
    pub const BACK: u32 = 0xc030_8014;
    pub const FRONT: u32 = 0x2ac6_cc0c;
    pub const CHARACTERS: u32 = 0xba62_5679;
    pub const LAYER_DISTANCE: u32 = field_hash("LayerDistance");
    pub const SEQUENCE: u32 = field_hash("Sequence");
    pub const LAYER_CENTRE: u32 = 0xe74d_0880;
    pub const LAYER_LEFT: u32 = 0x5b0d_9447;
    pub const LAYER_RIGHT: u32 = 0xaa63_128e;
    pub const DISTANCE: u32 = field_hash("Distance");
    pub const FIRST_DISTANCE: u32 = field_hash("FirstDistance");
    pub const ITEM_OBJECT: u32 = 0x2340_6ec7;
    pub const ANGLE: u32 = field_hash("Angle");
    /// `GameLogicConfig` +0x280.
    pub const REMOVAL: u32 = 0xc1f9_bb3f;
}

fn ids(o: &Object, hash: u32) -> Vec<u32> {
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

fn roadblock_item(o: &Object) -> RoadblockItem {
    RoadblockItem { object: uint(o, rb::ITEM_OBJECT), angle: float(o, rb::ANGLE) }
}

impl AssetStore {
    /// The roadblock definitions, layers and item kinds.
    pub fn roadblock_data(&self) -> Result<RoadblockData> {
        let removal = self.genesys_object(GAME_LOGIC_CONFIG).map(|o| float(&o, rb::REMOVAL)).unwrap_or(500.0);
        let mut data = RoadblockData { removal_distance: removal, ..Default::default() };
        let (mut player_types, mut rollouts) = (HashMap::new(), HashMap::new());
        for o in items(self, GAMEPLAY)? {
            match o.type_name.as_str() {
                "Genesys.Gen.RoadBlockDefinition" => {
                    let id = uint(&o, h::ID);
                    let def = RoadblockDefinition {
                        id,
                        centre: uint(&o, rb::CENTRE),
                        back: ids(&o, rb::BACK),
                        front: ids(&o, rb::FRONT),
                        layer_distance: float(&o, rb::LAYER_DISTANCE),
                        characters: ids(&o, rb::CHARACTERS),
                        sequence: uint(&o, rb::SEQUENCE),
                    };
                    data.definitions.insert(id, def);
                }
                "Genesys.Gen.RoadBlockLayer" => {
                    let id = uint(&o, h::ID);
                    let list = |hash| structs(o.get(hash)).into_iter().map(roadblock_item).collect();
                    let centre = structs(o.get(rb::LAYER_CENTRE)).first().map(|i| roadblock_item(i));
                    let layer = RoadblockLayer {
                        id,
                        centre: centre.unwrap_or_default(),
                        left: list(rb::LAYER_LEFT),
                        right: list(rb::LAYER_RIGHT),
                        distance: float(&o, rb::DISTANCE),
                        first_distance: float(&o, rb::FIRST_DISTANCE),
                    };
                    data.layers.insert(id, layer);
                }
                "Genesys.Gen.AIPlayerType" => {
                    player_types.insert(uint(&o, h::ID), uint(&o, h::ROLLOUT));
                }
                "Genesys.Gen.Rollout" => {
                    rollouts.insert(uint(&o, h::ID), uint(&o, h::VEHICLE));
                }
                _ => {}
            }
        }
        let mut wanted: Vec<u32> = data
            .layers
            .values()
            .flat_map(|l| std::iter::once(l.centre).chain(l.left.iter().copied()).chain(l.right.iter().copied()))
            .map(|i| i.object)
            .filter(|&o| o != 0)
            .collect();
        wanted.sort_unstable();
        wanted.dedup();
        let mut physics_ids = Vec::new();
        for id in wanted {
            if let Some(rollout) = player_types.get(&id) {
                if let Some(&vehicle) = rollouts.get(rollout) {
                    data.items.insert(id, RoadblockItemKind::Car { vehicle });
                }
                continue;
            }
            // A top-level object: its resource id is the UniqueId tagged 0x01000000 (an `AIPlayerType`, a
            // weapon) or 0x01000001 (a `PhysicalDefinition`).
            let Ok(o) = self.genesys_object(ResourceId(0x0100_0000_0000_0000 | id as u64)) else {
                physics_ids.push(id);
                continue;
            };
            match o.type_name.as_str() {
                "Genesys.Gen.AIPlayerType" => {
                    let rollout = uint(&o, h::ROLLOUT);
                    let vehicle = rollouts.get(&rollout).copied().or_else(|| {
                        let r = self.genesys_object(ResourceId(0x0100_0000_0000_0000 | rollout as u64)).ok()?;
                        Some(uint(&r, h::VEHICLE))
                    });
                    if let Some(vehicle) = vehicle.filter(|&v| v != 0) {
                        data.items.insert(id, RoadblockItemKind::Car { vehicle });
                    }
                }
                "Genesys.Gen.SpikeStripWeapon" => {
                    data.items.insert(id, RoadblockItemKind::SpikeStrip);
                }
                _ => {}
            }
        }
        if !physics_ids.is_empty()
            && let Some(b) = self.bundle("HAWAII/GLOBALRESOURCES.BNDL")
        {
            for e in b.entries_of_type(mw_bundle::ResourceType::WorldObject) {
                let Ok(def) = self.object_def(e.id, crate::props::ObjectKind::Dynamic) else { continue };
                let Some(physics) = def.physics else { continue };
                let unique = (physics.0 & 0xffff_ffff) as u32;
                if physics.0 >> 32 == 0x0100_0001 && physics_ids.contains(&unique) {
                    data.items.insert(unique, RoadblockItemKind::Object { world_object: e.id });
                }
            }
        }
        Ok(data)
    }
}
