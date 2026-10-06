//! A car's performance mods: `Genesys.Gen.Vehicles.PerformanceUpgrades` in its vehicle bundle (docs/decomp.md
//! "Nitrous and upgrades").
//!
//! Two categories, each with up to 18 slot fields; a slot holds a `ModifierUpgrade` {Icon, Name (Language ids),
//! `Modifier` = `PerformanceModifier` {`Modifications`: [{`Value`, `566a7ed1` parameter index,
//! `ModificationType` 0 multiply / 1 add / 2 replace}]}}, a `NitrousUpgrade` (a handle to a `NitrousParameters`)
//! or a `TyreUpgrade` (`a316f8e3`, a tyre index). The game folds the chosen mods into a 50-entry profile that
//! `FUN_00574870(index, value)` applies: value × Π(multiply) + Σ(add), or the replace operand.

use mw_bundle::{Bundle, ResourceType};
use mw_formats::genesys::{Object, Value, field_hash};

use crate::{AssetStore, Result};

/// One change to a profiled handling parameter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Modification {
    /// The index `FUN_00574870` is called with (`566a7ed1`).
    pub param: u32,
    /// 0 multiply, 1 add, 2 replace (`ModificationType`).
    pub op: u32,
    pub value: f32,
}

/// One mod choice in a slot.
#[derive(Debug, Clone, PartialEq)]
pub struct ModChoice {
    /// The slot field's hash (its name when known).
    pub slot: u32,
    /// Language string ids.
    pub name: Option<u64>,
    pub icon: Option<u64>,
    pub kind: String,
    pub modifications: Vec<Modification>,
    /// `NitrousUpgrade`: the `NitrousParameters` it selects; `TyreUpgrade`: the tyre index.
    pub nitrous: Option<String>,
    pub tyre: Option<i64>,
}

/// `FUN_00574870`: apply the modifications for `param` to `value`.
pub fn apply(mods: &[Modification], param: u32, value: f32) -> f32 {
    let (mut mul, mut add) = (value, 0.0);
    for m in mods.iter().filter(|m| m.param == param) {
        match m.op {
            0 => mul *= m.value,
            1 => add += m.value,
            2 => return m.value,
            _ => {}
        }
    }
    mul + add
}

fn int(o: &Object, hash: u32) -> Option<i64> {
    match o.get(hash)? {
        Value::Int(i) => Some(*i),
        Value::UInt(u) => Some(*u as i64),
        _ => None,
    }
}

fn choice(slot: u32, o: &Object) -> ModChoice {
    let mut modifications = Vec::new();
    if let Some(Value::Struct(m)) = o.get(field_hash("Modifier"))
        && let Some(Value::Array(items)) = m.get(field_hash("Modifications"))
    {
        for it in items {
            let Value::Struct(it) = it else { continue };
            let value = match it.get(field_hash("Value")) {
                Some(Value::Float(f)) => *f,
                _ => continue,
            };
            modifications.push(Modification {
                param: int(it, 0x566a7ed1).unwrap_or(-1) as u32,
                op: int(it, field_hash("ModificationType")).unwrap_or(-1) as u32,
                value,
            });
        }
    }
    let nitrous = match o.get(0x5b7038f6) {
        Some(Value::Handle(Some(id))) | Some(Value::Ref(id)) => Some(format!("{:016x}", id.0)),
        _ => None,
    };
    ModChoice {
        slot,
        name: int(o, field_hash("Name")).map(|v| v as u64),
        icon: int(o, field_hash("Icon")).map(|v| v as u64),
        kind: o.type_name.trim_start_matches("Genesys.Gen.Vehicles.").to_string(),
        modifications,
        nitrous,
        tyre: int(o, 0xa316f8e3),
    }
}

/// Every mod of a vehicle bundle, per category (the two `Category` fields, in field order).
pub fn performance_upgrades(store: &AssetStore, bundle: &Bundle) -> Result<Vec<Vec<ModChoice>>> {
    let mut out = Vec::new();
    for e in bundle.entries_of_type(ResourceType::GenesysObject) {
        let Ok(obj) = store.decode_genesys(&bundle.load(e)?) else { continue };
        if obj.type_name != "Genesys.Gen.Vehicles.PerformanceUpgrades" {
            continue;
        }
        for (_, v) in &obj.fields {
            let Value::Struct(cat) = v else { continue };
            let slots = cat
                .fields
                .iter()
                .filter_map(|(h, v)| match v {
                    Value::Struct(o) => Some(choice(*h, o)),
                    _ => None,
                })
                .collect();
            out.push(slots);
        }
        break;
    }
    Ok(out)
}
