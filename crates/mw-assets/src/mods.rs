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

/// What a profile index changes, from the call sites of `FUN_00574870` (docs/decomp.md "Nitrous and upgrades").
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    /// `FUN_009cd8f0` folds index 0 into a scale on 1.0; `FUN_009d7110` multiplies three `Engine.Power_curve`
    /// values (`8c227e4d`, `91c58956`, `12ad185d`) by it. Our model scales the torque curve.
    TorqueScale,
    /// `Gearbox.Differential`, `Gear_change_time`, every gear's `Gear_down_RPM` / `Gear_up_speed`
    /// (`FUN_009d7110`, `FUN_009d2740`).
    FinalDrive,
    GearChangeTime,
    GearDownRpm,
    GearUpSpeed,
    /// `SteeringBehaviour` fields (`FUN_009cf6a0`): (quantity, column) of `Steering`; quantity 0 range, 1 rate,
    /// 2 counter rate, 3 centre rate, 4 reverse speed.
    Steer(u8, usize),
    /// `HandlingBehaviour` fields by `VehicleHandling::param` path.
    Params(&'static [&'static str]),
    /// Identified, but not read by our model.
    NotModelled,
}

/// Profile index → what it changes. Indices not listed (4–19, 22–25, 29, 30, 33, 41, 48–54) have call sites
/// whose field was not identified.
pub const PARAMS: &[(u32, &str, Target)] = &[
    (0, "Engine torque scale", Target::TorqueScale),
    (1, "drift yaw torque (FUN_009d0860)", Target::NotModelled),
    (2, "Steering.High_speed_counter_steer_rate", Target::Steer(2, 1)),
    (3, "Steering.Reverse_speed", Target::Steer(4, 0)),
    (20, "Gearbox.Gear_change_time", Target::GearChangeTime),
    (21, "Steering.High_speed_center_steer_rate", Target::Steer(3, 1)),
    (
        26,
        "Drift.Yaw_torque.179fe51e+4cad24de",
        Target::Params(&["Drift.Yaw_torque.179fe51e", "Drift.Yaw_torque.4cad24de"]),
    ),
    (27, "Drift.Yaw_torque.e324fa82", Target::Params(&["Drift.Yaw_torque.e324fa82"])),
    (28, "Drift.Other.55930ac1+de67c45d", Target::Params(&["Drift.Other.55930ac1", "Drift.Other.de67c45d"])),
    (31, "Steering.High_speed_steer_rate", Target::Steer(1, 1)),
    (32, "Steering.High_speed_steering_range", Target::Steer(0, 1)),
    (34, "Drift.Drift_trigger.8e4032c0", Target::Params(&["Drift.Drift_trigger.8e4032c0"])),
    (35, "Drift.Side_force.09832a93", Target::Params(&["Drift.Side_force.09832a93"])),
    (36, "Steering.Reverse_steering_range", Target::Steer(0, 2)),
    (37, "Steering.16fd6022", Target::NotModelled),
    (38, "Gearbox.Differential", Target::FinalDrive),
    (39, "Gear.Gear_down_RPM", Target::GearDownRpm),
    (40, "Gear.Gear_up_speed", Target::GearUpSpeed),
    (42, "Drift.Side_force.ff5dcf85", Target::Params(&["Drift.Side_force.ff5dcf85"])),
    (43, "Drift.Side_force.f6c50d59", Target::Params(&["Drift.Side_force.f6c50d59"])),
    (44, "Drift.Side_force.df1c4fa8", Target::Params(&["Drift.Side_force.df1c4fa8"])),
    (45, "Drift.Other.30580747", Target::Params(&["Drift.Other.30580747"])),
    (46, "Drift.Drift_trigger.8f151151", Target::Params(&["Drift.Drift_trigger.8f151151"])),
    (47, "Drift.Drift_trigger.0dce38f7", Target::Params(&["Drift.Drift_trigger.0dce38f7"])),
];

/// The field a profile index changes (`p<index>` when not identified).
pub fn param_name(index: u32) -> String {
    PARAMS.iter().find(|p| p.0 == index).map_or_else(|| format!("p{index}"), |p| p.1.to_string())
}

/// Route `h`'s parameters through the chosen mods as `FUN_00574870` does at each call site.
pub fn apply_to(h: &mut crate::handling::VehicleHandling, mods: &[Modification]) {
    for &(index, _, target) in PARAMS {
        if !mods.iter().any(|m| m.param == index) {
            continue;
        }
        let f = |v: f32| apply(mods, index, v);
        let gearboxes = |h: &mut crate::handling::VehicleHandling, g: &dyn Fn(&mut crate::handling::Gearbox)| {
            h.gearbox.iter_mut().chain(h.cruising_gearbox.iter_mut()).for_each(g)
        };
        match target {
            Target::TorqueScale => {
                let s = f(1.0);
                for p in &mut h.torque_curve {
                    p.torque *= s;
                    p.tangent_in[1] *= s;
                    p.tangent_out[1] *= s;
                }
            }
            Target::FinalDrive => gearboxes(h, &|g| g.final_drive = f(g.final_drive)),
            Target::GearDownRpm => gearboxes(h, &|g| g.downshift_rpm.iter_mut().for_each(|v| *v = f(*v))),
            Target::GearUpSpeed => gearboxes(h, &|g| g.upshift_mph.iter_mut().for_each(|v| *v = f(*v))),
            Target::GearChangeTime => h.gear_change_time = h.gear_change_time.map(f),
            Target::Steer(q, c) => {
                if let Some(s) = h.steering.as_mut() {
                    let v = match q {
                        0 => &mut s.range[c],
                        1 => &mut s.rate[c],
                        2 => &mut s.counter_rate[c],
                        3 => &mut s.center_rate[c],
                        _ => &mut s.reverse_speed_mph,
                    };
                    *v = f(*v);
                }
            }
            Target::Params(paths) => {
                for p in paths {
                    if let Some(v) = h.param_mut(p) {
                        *v = f(*v);
                    }
                }
            }
            Target::NotModelled => {}
        }
    }
}

/// `Nitrous.053c5a6a` (tank burned per second) of a `NitrousUpgrade`'s `NitrousParameters`.
pub fn nitrous_burn(store: &AssetStore, id: &str) -> Option<f32> {
    let id = mw_bundle::ResourceId(u64::from_str_radix(id, 16).ok()?);
    let obj = store.decode_genesys(&store.load(id).ok()?).ok()?;
    // The `Nitrous` field (type `NitrousParameters.NitrousUsage`).
    let Value::Struct(u) = obj.get(field_hash("Nitrous"))? else { return None };
    match u.get(0x053c5a6a)? {
        Value::Float(f) => Some(*f),
        _ => None,
    }
}
