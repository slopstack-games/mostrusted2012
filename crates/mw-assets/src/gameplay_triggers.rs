//! `GameplayTrigger`s (all 200 in the list object 457533 of `GAMELOGIC/GAMEPLAY.BNDL`; decomp.md "Gameplay
//! triggers"): what an event's trigger boxes and the scoring actions set off. Layout (`mwtool genesys-types`):
//!
//! | type | offset | field | meaning |
//! |------|--------|-------|---------|
//! | `GameplayTrigger` (48 B) | 0x0c | `Predicate` | a binding that must hold when it fires (`FUN_0063e470`) |
//! | | 0x14 | `655d3f5f` | UniqueId (what an event's `a9d6149c` list names) |
//! | | 0x18 | `3c1e3e9d` | re-arm time, s (re-arm mode 2) |
//! | | 0x1c / 0x26 | `Input`[] | `Trigger` (+0xc): a world trigger whose hit queues it |
//! | | 0x20 / 0x28 | `Output`[] (44 B each) | what it does |
//! | | 0x24 | `4d96984c` | re-arm: 1 once its sequence ended, 2 after +0x18 s, else never in the event |
//! | | 0x2b / 0x2c | `14ababa7` / `6a0708fd` | queued / fired while the player is crashed (+0x2c: waits) |
//! | `Output` | 0x0c | `Predicate` | a binding this output needs |
//! | | 0x14 / 0x29 | `8239ac66`[] | speech events: the first whose conditions hold (`FUN_006e0a90`) |
//! | | 0x18 / 0x24 | `1795ca2a`[] | AI players spawned at a trigger (`FUN_006ce030`, the queue `FUN_006e8ae0`) |
//! | | 0x1c / 0x26 | `Sequence`[] `SequenceOutput` | the first: `Sequence` (+0xc), `SequenceType` (+0x10), `a011d5ae` (+0x12) |
//! | | 0x20 / 0x28 | `RoadblockInstance`[] | roadblocks placed (`FUN_0083f030`, after `FUN_008456d0`) |

use mw_formats::genesys::{Object, Value, field_hash};
use mw_formats::ui::object_id;

use crate::scoring::GAMEPLAY_TRIGGERS;
use crate::{AssetStore, Result};

/// One output's sequence (`GameplayTrigger.Output.SequenceOutput`).
#[derive(Debug, Clone, PartialEq)]
pub struct SequenceOutput {
    pub sequence: u32,
    /// `SequenceType` (+0x10): the sequence player (0 / 2 UIM+0x12568, 3 / 4 a free feedback player
    /// `FUN_007b2620`, 5 / 7 UIM+0x126dc, 6 the sim-timer player UIM+0x47748, else UIM+0x1245c).
    pub kind: u16,
    /// `a011d5ae` (+0x12): the queued parameters are handed to the sequence.
    pub with_params: bool,
}

/// One `GameplayTrigger.Output`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Output {
    pub predicate: String,
    pub speech: Vec<u32>,
    /// The `AIPlayerInformation` count (spawns; not read further yet).
    pub ai_players: usize,
    /// The first `SequenceOutput` (the only one the code plays).
    pub sequence: Option<SequenceOutput>,
    pub roadblocks: usize,
}

/// One `GameplayTrigger`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GameplayTrigger {
    pub id: u32,
    pub predicate: String,
    pub inputs: Vec<u32>,
    pub outputs: Vec<Output>,
    /// `4d96984c` (+0x24).
    pub rearm: u16,
    /// `3c1e3e9d` (+0x18), s.
    pub rearm_time: f32,
    /// `14ababa7` (+0x2b): fires while the player is crashed.
    pub while_crashed: bool,
    /// `6a0708fd` (+0x2c): queued while crashed, waits to fire until the player is not.
    pub after_crash: bool,
}

fn num(v: Option<&Value>) -> f64 {
    match v {
        Some(Value::UInt(n)) => *n as f64,
        Some(Value::Int(n)) => *n as f64,
        Some(Value::Float(f)) => f64::from(*f),
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        _ => 0.0,
    }
}

fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn structs(v: Option<&Value>) -> impl Iterator<Item = &Object> {
    let a: &[Value] = match v {
        Some(Value::Array(a)) => a,
        _ => &[],
    };
    a.iter().filter_map(|x| if let Value::Struct(o) = x { Some(o) } else { None })
}

fn ids(v: Option<&Value>) -> Vec<u32> {
    match v {
        Some(Value::Array(a)) => a.iter().map(|x| num(Some(x)) as u32).collect(),
        _ => Vec::new(),
    }
}

impl GameplayTrigger {
    pub fn read(o: &Object) -> Self {
        let predicate = field_hash("Predicate");
        GameplayTrigger {
            id: num(o.get(0x655d_3f5f)) as u32,
            predicate: text(o.get(predicate)),
            inputs: structs(o.get(field_hash("Input"))).map(|i| num(i.get(field_hash("Trigger"))) as u32).collect(),
            outputs: structs(o.get(field_hash("Output")))
                .map(|out| Output {
                    predicate: text(out.get(predicate)),
                    speech: ids(out.get(0x8239_ac66)),
                    ai_players: structs(out.get(0x1795_ca2a)).count(),
                    sequence: structs(out.get(field_hash("Sequence"))).next().map(|s| SequenceOutput {
                        sequence: num(s.get(field_hash("Sequence"))) as u32,
                        kind: num(s.get(field_hash("SequenceType"))) as u16,
                        with_params: num(s.get(0xa011_d5ae)) != 0.0,
                    }),
                    roadblocks: structs(out.get(field_hash("RoadblockInstance"))).count(),
                })
                .collect(),
            rearm: num(o.get(0x4d96_984c)) as u16,
            rearm_time: num(o.get(0x3c1e_3e9d)) as f32,
            while_crashed: num(o.get(0x14ab_aba7)) != 0.0,
            after_crash: num(o.get(0x6a07_08fd)) != 0.0,
        }
    }
}

impl AssetStore {
    /// Every `GameplayTrigger` of the list object 457533.
    pub fn gameplay_triggers(&self) -> Result<Vec<GameplayTrigger>> {
        let list = self.genesys_object(object_id(GAMEPLAY_TRIGGERS))?;
        let mut out = Vec::new();
        for (_, v) in &list.fields {
            if let Value::Array(a) = v {
                for o in a.iter().filter_map(|x| if let Value::Struct(o) = x { Some(o) } else { None }) {
                    if o.type_name.ends_with(".GameplayTrigger") || o.type_name == "GameplayTrigger" {
                        out.push(GameplayTrigger::read(o));
                    }
                }
            }
        }
        Ok(out)
    }
}
