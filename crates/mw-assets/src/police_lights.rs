//! A car's flashing lights: the `VehicleVfxBehaviour` coronas and light sources of effect group 3 (the cop
//! lights, docs/decomp.md "Police sirens, light bars and radio") with their `FlashAnimation`s, and whether the
//! car has a siren (`VehicleComponent.54eb210e`, +0x8e).
//!
//! - `FlashAnimation` (+0x10 / +0x14 `Animations`): segments `FlashAnimation.Animation` (0x24 bytes: +0x0c
//!   `Pattern` handle, +0x18 `Duration`, +0x1c `Frequency`, +0x20 `Phase`); `FlashPattern` (+0x10 / +0x14
//!   `Pattern`): float samples. Evaluated by `FUN_0049ef00` ([`FlashAnimation::value`]).
//! - The car's flash clock is the game clock × a per-car rate `frac(slot × 0.39 + 0.5) × 0.2 + 0.9`
//!   (entity +0x3e4, `FUN_00593ac0`), so cops flash out of step.

use std::collections::HashMap;
use std::sync::Arc;

use mw_bundle::{ResourceId, ResourceType};
use mw_formats::genesys::{Object, Value, field_hash};

use crate::AssetStore;
use crate::lights::LightDef;

/// Effect group of the cop lights (`FUN_0057a5e0`: bit 3 while the entity's +0x1c27 is set).
pub const COP_GROUP: u8 = 3;

/// `VehicleComponent.54eb210e` (+0x8e): the car has a siren; its horn cycles the siren instead (`FUN_00587720`).
const HAS_SIREN: u32 = 0x54eb_210e;
/// `VehicleVfxBehaviour.*.da6ac384`: 0 always, 1 by day (fading out with the night level), 2 at night.
const GATE: u32 = 0xda6a_c384;
/// `VehicleVfxBehaviour.Corona` size fields (+0x38 / +0x3c; width and height, inferred).
const SIZE_X: u32 = 0xf65c_1f07;
const SIZE_Y: u32 = 0xf65c_1f06;
/// `VehicleVfxBehaviour.Light.a3de907e` (+0x34): the light's intensity scale (`FUN_005818f0`).
const LIGHT_SCALE: u32 = 0xa3de_907e;

/// One segment of a [`FlashAnimation`].
#[derive(Debug, Clone, PartialEq)]
pub struct FlashSegment {
    pub pattern: Vec<f32>,
    pub duration: f32,
    pub frequency: f32,
    pub phase: f32,
}

/// `Genesys.Gen.FlashAnimation`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FlashAnimation {
    pub segments: Vec<FlashSegment>,
}

impl FlashAnimation {
    /// The animation's value at flash time `t` (`FUN_0049ef00`): the segment is picked by `frac(t / T)` with
    /// `T` = the sum of the durations (each takes its share in order; the last if none matches), then the
    /// pattern is read at `f = (Frequency · t + Phase) · n` (the absolute time, not the time in the segment;
    /// n = the sample count), linearly between samples `trunc(f) mod n` and the next.
    pub fn value(&self, t: f32) -> f32 {
        let total: f32 = self.segments.iter().map(|s| s.duration).sum();
        let Some(last) = self.segments.last() else { return 0.0 };
        let mut seg = last;
        if total > 0.0 {
            let mut x = (t / total).fract();
            if x < 0.0 {
                x += 1.0;
            }
            for s in &self.segments {
                x -= s.duration / total;
                if x <= 0.0 {
                    seg = s;
                    break;
                }
            }
        }
        let n = seg.pattern.len();
        if n == 0 {
            return 0.0;
        }
        let f = (seg.frequency * t + seg.phase) * n as f32;
        let i = f.trunc();
        let a = seg.pattern[(i as i64).rem_euclid(n as i64) as usize];
        let b = seg.pattern[(i as i64 + 1).rem_euclid(n as i64) as usize];
        a + (b - a) * (f - f.floor())
    }
}

/// A placed point (car space): rotation quaternion (x, y, z, w) and position.
pub type Locator = mw_formats::Locator;

/// A `VehicleVfxBehaviour.Corona` of a flashing group.
#[derive(Debug, Clone)]
pub struct FlashCorona {
    pub group: u8,
    pub gate: u8,
    pub colour: [f32; 4],
    pub brightness: f32,
    pub size: [f32; 2],
    pub mirror: bool,
    pub definition: Option<ResourceId>,
    pub animation: Option<Arc<FlashAnimation>>,
    pub locators: Vec<Locator>,
}

/// A `VehicleVfxBehaviour.Light` of a flashing group (the bar's `Light_Custom`s).
#[derive(Debug, Clone)]
pub struct FlashLight {
    pub group: u8,
    pub gate: u8,
    pub def: LightDef,
    /// `a3de907e`.
    pub scale: f32,
    pub animation: Option<Arc<FlashAnimation>>,
    pub locators: Vec<Locator>,
}

/// A car's cop lights.
#[derive(Debug, Clone, Default)]
pub struct CarFlashLights {
    pub has_siren: bool,
    pub coronas: Vec<FlashCorona>,
    pub lights: Vec<FlashLight>,
}

impl CarFlashLights {
    pub fn is_empty(&self) -> bool {
        !self.has_siren && self.coronas.is_empty() && self.lights.is_empty()
    }
}

fn int(o: &Object, hash: u32) -> u64 {
    match o.get(hash) {
        Some(Value::Int(i)) => *i as u64,
        Some(Value::UInt(u)) => *u,
        Some(Value::Float(f)) => *f as u64,
        _ => 0,
    }
}

fn float(o: &Object, hash: u32, default: f32) -> f32 {
    match o.get(hash) {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f32,
        Some(Value::UInt(u)) => *u as f32,
        _ => default,
    }
}

fn handle(o: &Object, hash: u32) -> Option<ResourceId> {
    match o.get(hash)? {
        Value::Handle(h) => *h,
        Value::Ref(r) => Some(*r),
        _ => None,
    }
}

fn structs(o: &Object, hash: u32) -> Vec<&Object> {
    match o.get(hash) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| match v {
                Value::Struct(s) => Some(s),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

impl AssetStore {
    /// A `FlashAnimation` and its patterns.
    pub fn flash_animation(&self, id: ResourceId) -> Option<FlashAnimation> {
        let o = self.genesys_object(id).ok()?;
        if o.type_name != "Genesys.Gen.FlashAnimation" {
            return None;
        }
        let segments = structs(&o, field_hash("Animations"))
            .into_iter()
            .map(|a| {
                let pattern = handle(a, field_hash("Pattern"))
                    .and_then(|p| self.genesys_object(p).ok())
                    .map(|p| match p.get(field_hash("Pattern")) {
                        Some(Value::Array(v)) => v
                            .iter()
                            .filter_map(|x| match x {
                                Value::Float(f) => Some(*f),
                                Value::Int(i) => Some(*i as f32),
                                Value::UInt(u) => Some(*u as f32),
                                _ => None,
                            })
                            .collect(),
                        _ => Vec::new(),
                    })
                    .unwrap_or_default();
                FlashSegment {
                    pattern,
                    duration: float(a, field_hash("Duration"), 0.0),
                    frequency: float(a, field_hash("Frequency"), 0.0),
                    phase: float(a, field_hash("Phase"), 0.0),
                }
            })
            .collect();
        Some(FlashAnimation { segments })
    }

    /// The car's cop lights: coronas and lights of [`COP_GROUP`] with their locators, and the siren flag.
    pub fn car_flash_lights(
        &self,
        bundle: &mw_bundle::Bundle,
        spec: &mw_formats::VehicleGraphicsSpec,
    ) -> CarFlashLights {
        let mut out = CarFlashLights::default();
        let mut anims: HashMap<ResourceId, Option<Arc<FlashAnimation>>> = HashMap::new();
        let mut anim = |id: Option<ResourceId>| -> Option<Arc<FlashAnimation>> {
            let id = id?;
            anims.entry(id).or_insert_with(|| self.flash_animation(id).map(Arc::new)).clone()
        };
        let locators = |id: u64| spec.locator_group(id as u32).map(|g| g.locators.clone()).unwrap_or_default();
        for e in bundle.entries_of_type(ResourceType::GenesysObject) {
            let Ok(res) = bundle.load(e) else { continue };
            let Ok(o) = self.decode_genesys(&res) else { continue };
            match o.type_name.as_str() {
                "Genesys.Gen.VehicleComponent" => {
                    out.has_siren |= matches!(o.get(HAS_SIREN), Some(Value::Bool(true)));
                }
                "Genesys.Gen.VehicleVfxBehaviour" => {
                    for c in structs(&o, field_hash("Coronas")) {
                        let group = int(c, field_hash("Group")) as u8;
                        if group != COP_GROUP {
                            continue;
                        }
                        let colour = match c.get(field_hash("Colour")) {
                            Some(Value::Vector(v)) => *v,
                            _ => [1.0; 4],
                        };
                        out.coronas.push(FlashCorona {
                            group,
                            gate: int(c, GATE) as u8,
                            colour,
                            brightness: float(c, field_hash("Brightness"), 1.0),
                            size: [float(c, SIZE_X, 1.0), float(c, SIZE_Y, 1.0)],
                            mirror: matches!(c.get(field_hash("Mirror")), Some(Value::Bool(true))),
                            definition: handle(c, field_hash("Corona_definition")),
                            animation: anim(handle(c, field_hash("Animation"))),
                            locators: locators(int(c, field_hash("Locator_group"))),
                        });
                    }
                    for l in structs(&o, field_hash("Lights")) {
                        let group = int(l, field_hash("Group")) as u8;
                        if group != COP_GROUP {
                            continue;
                        }
                        let Some(def) = handle(l, field_hash("LightDefinition"))
                            .and_then(|id| Some((id, self.genesys_object(id).ok()?)))
                            .and_then(|(id, o)| LightDef::from_object(id, &o))
                        else {
                            continue;
                        };
                        out.lights.push(FlashLight {
                            group,
                            gate: int(l, GATE) as u8,
                            def,
                            scale: float(l, LIGHT_SCALE, 1.0),
                            animation: anim(handle(l, field_hash("Animation"))),
                            locators: locators(int(l, field_hash("LocatorGroup"))),
                        });
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// The per-car flash rate (entity +0x3e4, `FUN_00593ac0`): `frac(slot × 0.39 + 0.5) × 0.2 + 0.9`.
pub fn flash_rate(slot: usize) -> f32 {
    (slot as f32 * 0.39 + 0.5).fract() * 0.2 + 0.9
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anim() -> FlashAnimation {
        let mut a = vec![0.0; 20];
        a[4] = 1.0;
        a[8] = 1.0;
        a[12] = 1.0;
        let mut b = vec![0.0; 16];
        b[4] = 1.0;
        b[12] = 1.0;
        FlashAnimation {
            segments: vec![
                FlashSegment { pattern: a, duration: 8.0, frequency: 0.4, phase: 0.0 },
                FlashSegment { pattern: b, duration: 8.0, frequency: 0.5, phase: 0.0 },
            ],
        }
    }

    #[test]
    fn flash_reads_the_pattern_at_absolute_time() {
        let a = anim();
        // First segment: 8 samples a second; sample 4 at t = 0.5 s.
        assert!((a.value(0.5) - 1.0).abs() < 1e-5);
        // Halfway between sample 4 (1) and 5 (0).
        assert!((a.value(0.5625) - 0.5).abs() < 1e-4);
        assert_eq!(a.value(0.25), 0.0);
        // Second segment (8..16 s): f = 0.5 · t · 16; at t = 8.5 s, f = 68 → sample 4.
        assert!((a.value(8.5) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn rates_spread_between_0_9_and_1_1() {
        for s in 0..20 {
            let r = flash_rate(s);
            assert!((0.9..=1.1).contains(&r));
        }
        assert!((flash_rate(0) - 1.0).abs() < 1e-6);
    }
}
