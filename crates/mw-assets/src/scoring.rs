//! Scoring: what the game awards for (`ScoringAction`s) and how many Speed Points each is worth
//! (docs/decomp.md "Speed Points").
//!
//! - **`ScoringConfig`** (`GLOBALCONFIG.BNDL`, UniqueId 390964; the config at global +0x44 in the code): about
//!   190 UniqueId fields, one per thing game code can score (`BillboardDiscovered`, `SpeedCamDiscovered`,
//!   `SmashGateDiscovered`, `CarSwapDiscovered`, `Takedown` …). Code names an action by the field's offset.
//! - **`ScoringAction`** (223 of them in one list object of `GAMELOGIC/GAMEPLAY.BNDL`, UniqueId 390077, the
//!   list a player object holds at +0x3ac / +0x3b0): `Predicate` (a binding that must hold), `GameplayTrigger`
//!   (sounds / effects), `Sequence` + `Title` (the HUD message: a `Sequence` whose layout shows the title
//!   text), a heat amount (`1a88cb4c`), `Score` (the pursuit / event score, indexed by heat level offline),
//!   **`8c1feb6d` = the Speed Points** (its count field is `SPCount`), `273e156e` (a bonus by the number of
//!   players, online), `Scope`, `Queue`, `Priority`.
//!
//! The offline award (`FUN_006d08a0`): Speed Points = the scoring event's own value if it carries one, else
//! the action's first `8c1feb6d` entry; added to the profile's total by `FUN_00734400` when above 0.

use std::collections::HashMap;

use mw_formats::genesys::{Object, Value, field_hash};
use mw_formats::ui::object_id;

use crate::{AssetStore, Result};

/// The `ScoringConfig` object (`GLOBALCONFIG.BNDL`).
pub const SCORING_CONFIG: u32 = 390_964;
/// The list of every `ScoringAction` (`GAMELOGIC/GAMEPLAY.BNDL`).
pub const SCORING_ACTIONS: u32 = 390_077;
/// The list of every `GameplayTrigger` (`GAMELOGIC/GAMEPLAY.BNDL`).
pub const GAMEPLAY_TRIGGERS: u32 = 457_533;
/// The `CollectablesConfig` object (`GLOBALCONFIG.BNDL`).
pub const COLLECTABLES_CONFIG: u32 = 908_780;

/// `ScoringAction.8c1feb6d`: the Speed Points array (name not recovered; its count field hashes as `SPCount`).
pub const SP_FIELD: u32 = 0x8c1f_eb6d;
const ID: u32 = 0x655d_3f5f;

/// `ScoringConfig` fields the free-roam collectables use, by the offsets `FUN_007059b0` / `FUN_006ed0a0` /
/// `FUN_006ed330` read (names cracked from the hashes; each sorts between its neighbours).
pub mod action {
    use mw_formats::genesys::field_hash;
    /// +0x18: a billboard smashed for the first time.
    pub const BILLBOARD_DISCOVERED: u32 = field_hash("BillboardDiscovered");
    /// +0x1c: sent with it (0 points).
    pub const BILLBOARD_FOUND: u32 = field_hash("BillboardFound");
    /// +0xfc: every billboard smash offline (`FUN_006ed330`; 0 points): billboards are jumps.
    pub const HIT_JUMP: u32 = field_hash("HitJump");
    /// +0x288: a security gate driven through for the first time.
    pub const SMASH_GATE_DISCOVERED: u32 = field_hash("SmashGateDiscovered");
    /// +0x260: sent with it (0 points; its `GameplayTrigger` plays a sequence).
    pub const SECURITY_GATE_HIT: u32 = field_hash("SecurityGateHit");
    /// +0x28c: a speed camera triggered for the first time.
    pub const SPEED_CAM_DISCOVERED: u32 = field_hash("SpeedCamDiscovered");
    /// +0x294: every counted speed camera pass (0 points).
    pub const SPEED_CAMERA_TRIGGERED: u32 = field_hash("SpeedCameraTriggered");
    /// +0x38: a jack spot car found for the first time; +0x3c with the location.
    pub const CAR_SWAP_DISCOVERED: u32 = field_hash("CarSwapDiscovered");
    pub const CAR_SWAP_FOUND: u32 = field_hash("CarSwapFound");
}

/// One `ScoringAction`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScoringAction {
    pub id: u32,
    /// A binding that must be true for the action to count (`FUN_0063e470`), e.g.
    /// `Players.LocalPlayer.IsInChaseOrCooldown`. Empty = always.
    pub predicate: String,
    pub gameplay_trigger: u32,
    /// The HUD message's `Sequence` (UniqueId) and its title text (language id); 0 = no message.
    pub sequence: u32,
    pub title: u32,
    /// `1a88cb4c`: added to the pursuit heat (`FUN_006bdac0`).
    pub heat: f32,
    /// `Score`: the pursuit / event score (offline: entry heat level − 1, else the first).
    pub score: Vec<u32>,
    /// `8c1feb6d`: Speed Points (offline: the first entry; online: by finishing position).
    pub speed_points: Vec<u32>,
    pub scope: u32,
    pub queue: u32,
    pub priority: u32,
}

impl ScoringAction {
    /// The Speed Points an offline scoring event of this action awards (`FUN_006d08a0`: the event's own value
    /// when it has one, else the first entry).
    pub fn sp(&self) -> u32 {
        self.speed_points.first().copied().unwrap_or(0)
    }
}

/// The scoring data: every action, and which action each `ScoringConfig` field names.
#[derive(Debug, Clone, Default)]
pub struct Scoring {
    pub actions: HashMap<u32, ScoringAction>,
    /// (`ScoringConfig` field hash, action UniqueId) in declaration order.
    pub config: Vec<(u32, u32)>,
    /// `GameplayTrigger` UniqueId → the `Sequence`s its outputs name (`Output[].Sequence[].Sequence`): the
    /// speed camera's flash and plate, the security gate's speedwall panel. What else a trigger does (its
    /// inputs, `8239ac66`, roadblocks) is not read.
    pub trigger_sequences: HashMap<u32, Vec<u32>>,
}

impl Scoring {
    /// The action a `ScoringConfig` field names (a field hash, e.g. [`action::BILLBOARD_DISCOVERED`]).
    pub fn by_field(&self, field: u32) -> Option<&ScoringAction> {
        let id = self.config.iter().find(|(h, _)| *h == field)?.1;
        self.actions.get(&id)
    }
}

/// `CollectablesConfig` values the collectable code reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CollectablesConfig {
    /// `d89eeda4` (+0x64): the least speed (mph) a speed camera counts (`FUN_006ed0a0`); 50.
    pub speed_camera_min_mph: f32,
}

fn uint(v: Option<&Value>) -> u32 {
    match v {
        Some(Value::UInt(u)) => *u as u32,
        Some(Value::Int(i)) => *i as u32,
        _ => 0,
    }
}

fn uints(v: Option<&Value>) -> Vec<u32> {
    match v {
        Some(Value::Array(a)) => a.iter().map(|v| uint(Some(v))).collect(),
        _ => Vec::new(),
    }
}

/// Every struct under `o` whose type name ends with `suffix` (not looking inside a match).
fn collect<'a>(o: &'a Object, suffix: &str, out: &mut Vec<&'a Object>) {
    fn walk<'a>(v: &'a Value, suffix: &str, out: &mut Vec<&'a Object>) {
        match v {
            Value::Struct(o) => collect(o, suffix, out),
            Value::Array(a) => a.iter().for_each(|v| walk(v, suffix, out)),
            _ => {}
        }
    }
    if o.type_name.ends_with(suffix) {
        out.push(o);
        return;
    }
    for (_, v) in &o.fields {
        walk(v, suffix, out);
    }
}

impl AssetStore {
    /// The scoring actions and the `ScoringConfig` naming them.
    pub fn scoring(&self) -> Result<Scoring> {
        let list = self.genesys_object(object_id(SCORING_ACTIONS))?;
        let mut found = Vec::new();
        collect(&list, ".ScoringAction", &mut found);
        let actions = found
            .into_iter()
            .map(|o| {
                let a = ScoringAction {
                    id: uint(o.get(ID)),
                    predicate: match o.get(field_hash("Predicate")) {
                        Some(Value::Str(s)) => s.clone(),
                        _ => String::new(),
                    },
                    gameplay_trigger: uint(o.get(field_hash("GameplayTrigger"))),
                    sequence: uint(o.get(field_hash("Sequence"))),
                    title: uint(o.get(field_hash("Title"))),
                    heat: match o.get(0x1a88_cb4c) {
                        Some(Value::Float(f)) => *f,
                        _ => 0.0,
                    },
                    score: uints(o.get(field_hash("Score"))),
                    speed_points: uints(o.get(SP_FIELD)),
                    scope: uint(o.get(field_hash("Scope"))),
                    queue: uint(o.get(field_hash("Queue"))),
                    priority: uint(o.get(field_hash("Priority"))),
                };
                (a.id, a)
            })
            .collect();
        let config = self.genesys_object(object_id(SCORING_CONFIG))?;
        let config = config
            .fields
            .iter()
            .filter(|(h, _)| *h != ID)
            .filter_map(|(h, v)| match v {
                Value::UInt(_) | Value::Int(_) => Some((*h, uint(Some(v)))),
                _ => None,
            })
            .collect();
        let mut trigger_sequences = HashMap::new();
        if let Ok(list) = self.genesys_object(object_id(GAMEPLAY_TRIGGERS)) {
            let mut triggers = Vec::new();
            collect(&list, ".GameplayTrigger", &mut triggers);
            for t in triggers {
                let mut outputs = Vec::new();
                for (_, v) in &t.fields {
                    if let Value::Array(a) = v {
                        for o in a.iter().filter_map(|v| if let Value::Struct(o) = v { Some(o) } else { None }) {
                            collect(o, ".SequenceOutput", &mut outputs);
                        }
                    }
                }
                let sequences: Vec<u32> =
                    outputs.iter().map(|o| uint(o.get(field_hash("Sequence")))).filter(|&s| s != 0).collect();
                if !sequences.is_empty() {
                    trigger_sequences.insert(uint(t.get(ID)), sequences);
                }
            }
        }
        Ok(Scoring { actions, config, trigger_sequences })
    }

    pub fn collectables_config(&self) -> Result<CollectablesConfig> {
        let o = self.genesys_object(object_id(COLLECTABLES_CONFIG))?;
        let speed_camera_min_mph = match o.get(0xd89e_eda4) {
            Some(Value::Float(f)) => *f,
            _ => return Err(crate::Error::Missing(object_id(COLLECTABLES_CONFIG))),
        };
        Ok(CollectablesConfig { speed_camera_min_mph })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mw_formats::genesys::count_hash;

    /// The Speed Points array's count field is `SPCount` (found by `mwtool genesys-crack`); the array itself is
    /// field 0x8c1feb6d.
    #[test]
    fn sp_count_field() {
        assert_eq!(field_hash("SPCount"), 0x26f4_5143);
        assert_eq!(field_hash("ScoreCount"), 0x7d7e_35b8);
        assert_eq!(count_hash(field_hash("Score")), 0x7d7e_35b8);
    }

    #[test]
    fn config_field_hashes() {
        assert_eq!(action::BILLBOARD_DISCOVERED, 0xc6dc_e6d1);
        assert_eq!(action::BILLBOARD_FOUND, 0x5366_3045);
        assert_eq!(action::HIT_JUMP, 0xb004_19a8);
        assert_eq!(action::SMASH_GATE_DISCOVERED, 0x5599_9934);
        assert_eq!(action::SECURITY_GATE_HIT, 0x0817_e28c);
        assert_eq!(action::SPEED_CAM_DISCOVERED, 0x589c_1f76);
        assert_eq!(action::SPEED_CAMERA_TRIGGERED, 0xd344_0241);
        assert_eq!(action::CAR_SWAP_DISCOVERED, 0xa0b3_8b1f);
        assert_eq!(action::CAR_SWAP_FOUND, 0x1d90_8317);
    }
}
