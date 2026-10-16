//! The game-world items of a HUD sequence (a scoring action's `GameplayTrigger` output, `mw_assets::scoring`):
//! what the speed camera's sequence 1107082 does besides its layouts (docs/decomp.md "Speed camera flash").
//!
//! | Item | Fields | What the game does |
//! |------|--------|--------------------|
//! | `SequenceItems.PostFxOverrideSequenceItem` (44 B) | `Automated_Values` [{`ModulatingValue`, `Automated_Property` (u16)}], `a258bf17` | per frame (`FUN_008635a0`) the curve at the item's normalised time → override slot 0 / 1 / 2 (`FUN_0083b7a0`) |
//! | `GameLogic.ModifyBehavioursInAreaSequenceItem` (96 B) | `TranslationOffset` (+0x30), `Label` (+0x40), `TransformBinding` (+0x48), `Radius` (+0x50), `ActionType` (+0x54) | start (`FUN_00856370`): objects within `Radius` of the binding's position, behaviours named `Label` switched on (`ActionType` ≠ 1) or off; end (`0x00851e20`): the reverse |
//! | `WaveSequenceItem` | `Waves`, `Gain`, `Mixer_Channel` | a one-shot at the item's start |

use mw_bundle::ResourceId;
use mw_formats::genesys::{Object, Value, field_hash};

use crate::AssetStore;
use crate::sequence::Modulated;

#[derive(Debug, Clone)]
pub enum FxKind {
    /// `Automated_Property` → the curve. Property 0 is the final pass's brightness, 1 its contrast, 2 a pair of
    /// PostFX state values (`FUN_008ecf80`).
    PostFx(Vec<(u16, Modulated)>),
    ModifyBehaviours {
        label: String,
        binding: String,
        offset: [f32; 3],
        radius: f32,
        /// `ActionType`: 1 switches off at the start and on at the end; anything else the other way round.
        action: u16,
    },
    Wave {
        wave: ResourceId,
        /// `Gain.Value` × the item's `Mixer_Channel` `Gain`.
        gain: f32,
    },
}

#[derive(Debug, Clone)]
pub struct FxItem {
    pub start: f32,
    pub end: f32,
    /// `EnabledBinding` (empty = always).
    pub enabled: String,
    pub kind: FxKind,
}

impl FxItem {
    /// The item's normalised time at sequence time `t`, while it runs.
    pub fn phase(&self, t: f32) -> Option<f32> {
        (t >= self.start && t < self.end).then(|| (t - self.start) / (self.end - self.start).max(1e-6))
    }
}

fn num(o: &Object, name: &str) -> f32 {
    match o.get(field_hash(name)) {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f32,
        Some(Value::UInt(u)) => *u as f32,
        _ => 0.0,
    }
}

fn text(o: &Object, name: &str) -> String {
    match o.get(field_hash(name)) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn handle(v: Option<&Value>) -> Option<ResourceId> {
    match v? {
        Value::Handle(h) => *h,
        Value::Ref(r) => Some(*r),
        _ => None,
    }
}

fn structs<'a>(o: &'a Object, name: &str) -> impl Iterator<Item = &'a Object> {
    let list = match o.get(field_hash(name)) {
        Some(Value::Array(a)) => a.as_slice(),
        _ => &[],
    };
    list.iter().filter_map(|v| match v {
        Value::Struct(s) => Some(s),
        _ => None,
    })
}

impl AssetStore {
    /// The world items of the sequence with UniqueId `unique_id` (empty when it has none or is missing).
    pub fn sequence_fx(&self, unique_id: u32) -> Vec<FxItem> {
        let Ok(o) = self.genesys_object(ResourceId(0x0100_0000_0000_0000 | unique_id as u64)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for it in structs(&o, "SequenceItems") {
            let ty = it.type_name.as_str();
            let kind = if ty.ends_with("PostFxOverrideSequenceItem") {
                let values = structs(it, "Automated_Values")
                    .filter_map(|v| {
                        let m = match v.get(field_hash("ModulatingValue")) {
                            Some(Value::Struct(m)) => Modulated::read(m),
                            _ => return None,
                        };
                        Some((num(v, "Automated_Property") as u16, m))
                    })
                    .collect();
                FxKind::PostFx(values)
            } else if ty.ends_with("ModifyBehavioursInAreaSequenceItem") {
                let offset = match it.get(field_hash("TranslationOffset")) {
                    Some(Value::Vector(v)) => [v[0], v[1], v[2]],
                    _ => [0.0; 3],
                };
                FxKind::ModifyBehaviours {
                    label: text(it, "Label"),
                    binding: text(it, "TransformBinding"),
                    offset,
                    radius: num(it, "Radius"),
                    action: num(it, "ActionType") as u16,
                }
            } else if ty.ends_with(".WaveSequenceItem") {
                let wave = match it.get(field_hash("Waves")) {
                    Some(Value::Array(a)) => a.first().and_then(|v| handle(Some(v))),
                    _ => None,
                };
                let Some(wave) = wave else { continue };
                let gain = match it.get(field_hash("Gain")) {
                    Some(Value::Struct(g)) => Modulated::read(g).value,
                    _ => 1.0,
                };
                let channel = handle(it.get(field_hash("Mixer_Channel")))
                    .and_then(|c| self.genesys_object(c).ok())
                    .map_or(1.0, |c| match c.get(field_hash("Gain")) {
                        Some(Value::Float(g)) => *g,
                        _ => 1.0,
                    });
                FxKind::Wave { wave, gain: gain * channel }
            } else {
                continue;
            };
            out.push(FxItem {
                start: num(it, "StartTime"),
                end: num(it, "EndTime"),
                enabled: text(it, "EnabledBinding"),
                kind,
            });
        }
        out
    }
}
