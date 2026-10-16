//! Coronas: the glow sprites at lamps (docs/decomp.md "Coronas"). Data: `Genesys.Gen.Corona` definitions with
//! their `Glow` / `Flare` / `Beam` lists, each carrying a two-segment curve block, and the cars'
//! `VehicleVfxBehaviour.Coronas` entries that place them at locators.
//!
//! **The curve block** (144 bytes, eight `Vector4`s; `FUN_004ab450`): for a 4-channel input x, per channel,
//! below the threshold `c2cad262` (+0x50) a = `c08c80f4` (+0x30) · x + `a2a9209e` (+0x10), else a =
//! `7c96a82d` (+0x40) · x + `1eb30847` (+0x20); a = min(a, 1); out = a · (`0b494d31` (+0x60) below,
//! `b75365e8` (+0x80) above) + `d95ae391` (+0x70), times the caller's scale.

use std::collections::HashMap;
use std::sync::Arc;

use mw_bundle::{ResourceId, ResourceType};
use mw_formats::genesys::{Object, Value, field_hash};

use crate::AssetStore;
use crate::police_lights::{FlashAnimation, Locator};

/// A two-segment curve block (`FUN_004ab450`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Curve {
    pub offset_low: [f32; 4],
    pub offset_high: [f32; 4],
    pub slope_low: [f32; 4],
    pub slope_high: [f32; 4],
    pub threshold: [f32; 4],
    pub scale_low: [f32; 4],
    pub base: [f32; 4],
    pub scale_high: [f32; 4],
}

/// `FUN_004ab450`'s clamp (`.rdata` 0x00fa26c0 = 1).
const CURVE_CLAMP: f32 = 1.0;

impl Curve {
    fn from_object(o: Option<&Object>) -> Self {
        let Some(o) = o else { return Self::default() };
        let v = |h: u32| match o.get(h) {
            Some(Value::Vector(v)) => *v,
            _ => [0.0; 4],
        };
        Self {
            offset_low: v(0xa2a9_209e),
            offset_high: v(0x1eb3_0847),
            slope_low: v(0xc08c_80f4),
            slope_high: v(0x7c96_a82d),
            threshold: v(0xc2ca_d262),
            scale_low: v(0x0b49_4d31),
            base: v(0xd95a_e391),
            scale_high: v(0xb753_65e8),
        }
    }

    /// The curve at `x`, channel by channel, times `scale`.
    pub fn eval(&self, x: [f32; 4], scale: [f32; 4]) -> [f32; 4] {
        std::array::from_fn(|i| {
            let low = x[i] < self.threshold[i];
            let a = if low {
                self.slope_low[i] * x[i] + self.offset_low[i]
            } else {
                self.slope_high[i] * x[i] + self.offset_high[i]
            }
            .min(CURVE_CLAMP);
            (a * if low { self.scale_low[i] } else { self.scale_high[i] } + self.base[i]) * scale[i]
        })
    }
}

/// One sprite of a corona definition (`Corona.Glow` / `.Flare` / `.Beam`, 224 bytes each).
#[derive(Debug, Clone)]
pub struct Sprite {
    pub curve: Curve,
    /// `Colour` (+0xa0).
    pub colour: [f32; 4],
    /// +0xb0: `2d6e41d4` (glow), `2b9cc12c` (flare), `034c050f` (beam).
    pub vector: [f32; 4],
    /// `Material` (+0xc0).
    pub material: Option<ResourceId>,
    /// +0xc8 / +0xcc: glow `a9531774` / `RotationOffset`, flare `Position` / `RotationOffset`, beam `a2974d7e` /
    /// `e9b65ffe`.
    pub value_a: f32,
    pub value_b: f32,
    /// `be0fbea3` (+0xd0).
    pub flag: u16,
}

/// A `Genesys.Gen.Corona` (224 bytes).
#[derive(Debug, Clone)]
pub struct CoronaDef {
    pub id: ResourceId,
    /// `330b6dee` (+0x10): the corona's own curve block.
    pub curve: Curve,
    /// `793b2691` / `88b68533` / `c4e3e3a7` (+0xa4 / +0xa8 / +0xac).
    pub values: [f32; 3],
    pub beams: Vec<Sprite>,
    pub flares: Vec<Sprite>,
    /// The four glow lists (+0xb8 `60c1d15f`, +0xbc `Glows`, +0xc0 `2392d7f2`, +0xc4 `9ee381ab`).
    pub glows: [Vec<Sprite>; 4],
    /// `f3961afd` (+0xc8, `Flags`), `b83f8dd0`, `2ad78577`, `625bd9a3`, `eea8dc53`, `0af387ad`, `d9777b79`.
    pub flags: [u16; 7],
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

fn sub(o: &Object, hash: u32) -> Option<&Object> {
    match o.get(hash)? {
        Value::Struct(s) => Some(s),
        _ => None,
    }
}

fn num(o: &Object, hash: u32) -> f32 {
    match o.get(hash) {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f32,
        Some(Value::UInt(u)) => *u as f32,
        Some(Value::Bool(b)) => f32::from(u8::from(*b)),
        _ => 0.0,
    }
}

fn vector(o: &Object, hash: u32) -> [f32; 4] {
    match o.get(hash) {
        Some(Value::Vector(v)) => *v,
        _ => [0.0; 4],
    }
}

fn handle(o: &Object, hash: u32) -> Option<ResourceId> {
    match o.get(hash)? {
        Value::Handle(h) => *h,
        Value::Ref(r) => Some(*r),
        _ => None,
    }
}

impl Sprite {
    fn from_object(o: &Object, curve: u32, vec: u32, a: u32, b: u32) -> Self {
        Self {
            curve: Curve::from_object(sub(o, curve)),
            colour: vector(o, 0x3460_a645),
            vector: vector(o, vec),
            material: handle(o, 0x01e5_cd8e),
            value_a: num(o, a),
            value_b: num(o, b),
            flag: num(o, 0xbe0f_bea3) as u16,
        }
    }
}

impl CoronaDef {
    pub fn from_object(id: ResourceId, o: &Object) -> Option<Self> {
        if o.type_name != "Genesys.Gen.Corona" {
            return None;
        }
        let glow = |h: u32| -> Vec<Sprite> {
            structs(o, h)
                .into_iter()
                .map(|s| Sprite::from_object(s, 0x4bc3_52e5, 0x2d6e_41d4, 0xa953_1774, 0x1f11_152d))
                .collect()
        };
        Some(Self {
            id,
            curve: Curve::from_object(sub(o, 0x330b_6dee)),
            values: [num(o, 0x793b_2691), num(o, 0x88b6_8533), num(o, 0xc4e3_e3a7)],
            beams: structs(o, 0xe992_6621)
                .into_iter()
                .map(|s| Sprite::from_object(s, 0x4bc3_52e5, 0x034c_050f, 0xa297_4d7e, 0xe9b6_5ffe))
                .collect(),
            flares: structs(o, 0xa303_70e8)
                .into_iter()
                .map(|s| Sprite::from_object(s, 0xf69b_d3da, 0x2b9c_c12c, 0xa156_caa3, 0x1f11_152d))
                .collect(),
            glows: [glow(0x60c1_d15f), glow(0xf759_174a), glow(0x2392_d7f2), glow(0x9ee3_81ab)],
            flags: [0xf396_1afd, 0xb83f_8dd0, 0x2ad7_8577, 0x625b_d9a3, 0xeea8_dc53, 0x0af3_87ad, 0xd977_7b79]
                .map(|h| num(o, h) as u16),
        })
    }
}

/// A `VehicleVfxBehaviour.Corona` entry with its definition and locators.
#[derive(Debug, Clone)]
pub struct CarCorona {
    pub group: u8,
    /// `da6ac384`: 0 always, 1 by day, 2 at night (`FUN_0089b400`).
    pub gate: u8,
    pub colour: [f32; 4],
    pub brightness: f32,
    /// `f65c1f07`, `f65c1f06`.
    pub size: [f32; 2],
    pub mirror: bool,
    pub def: Option<Arc<CoronaDef>>,
    pub animation: Option<Arc<FlashAnimation>>,
    pub locators: Vec<Locator>,
}

impl AssetStore {
    /// A corona definition.
    pub fn corona_def(&self, id: ResourceId) -> Option<CoronaDef> {
        CoronaDef::from_object(id, &self.genesys_object(id).ok()?)
    }

    /// Every `VehicleVfxBehaviour.Coronas` entry of a car's bundle.
    pub fn car_coronas(&self, bundle: &mw_bundle::Bundle, spec: &mw_formats::VehicleGraphicsSpec) -> Vec<CarCorona> {
        let mut defs: HashMap<ResourceId, Option<Arc<CoronaDef>>> = HashMap::new();
        let mut anims: HashMap<ResourceId, Option<Arc<FlashAnimation>>> = HashMap::new();
        let mut out = Vec::new();
        for e in bundle.entries_of_type(ResourceType::GenesysObject) {
            let Ok(res) = bundle.load(e) else { continue };
            let Ok(o) = self.decode_genesys(&res) else { continue };
            if o.type_name != "Genesys.Gen.VehicleVfxBehaviour" {
                continue;
            }
            for c in structs(&o, field_hash("Coronas")) {
                let def = handle(c, field_hash("Corona_definition"))
                    .and_then(|id| defs.entry(id).or_insert_with(|| self.corona_def(id).map(Arc::new)).clone());
                let animation = handle(c, field_hash("Animation"))
                    .and_then(|id| anims.entry(id).or_insert_with(|| self.flash_animation(id).map(Arc::new)).clone());
                let group = num(c, field_hash("Group")) as u8;
                out.push(CarCorona {
                    group,
                    gate: num(c, 0xda6a_c384) as u8,
                    colour: match c.get(field_hash("Colour")) {
                        Some(Value::Vector(v)) => *v,
                        _ => [1.0; 4],
                    },
                    brightness: num(c, field_hash("Brightness")),
                    size: [num(c, 0xf65c_1f07), num(c, 0xf65c_1f06)],
                    mirror: matches!(c.get(field_hash("Mirror")), Some(Value::Bool(true))),
                    def,
                    animation,
                    locators: spec
                        .locator_group(num(c, field_hash("Locator_group")) as u32)
                        .map(|g| g.locators.clone())
                        .unwrap_or_default(),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cop_corona_curve() {
        // The Interceptor's blue bar corona `…251ad`.
        let c = Curve {
            offset_low: [1.0; 4],
            offset_high: [-1.0 / 3.0, -1.0, -1.0, -1.0],
            slope_low: [-0.63661975, -0.004, -0.004, -0.004],
            slope_high: [0.21220659, 0.004, 0.004, 0.004],
            threshold: [std::f32::consts::FRAC_PI_2, 250.0, 250.0, 250.0],
            scale_low: [0.5, -1.2, -0.2, -0.2],
            base: [0.5, 1.65, 0.3, 0.3],
            scale_high: [0.5, 1.35, 0.2, 0.2],
        };
        let at0 = c.eval([0.0, 0.0, 0.0, 0.0], [1.0; 4]);
        assert!((at0[0] - 1.0).abs() < 1e-5 && (at0[1] - 0.45).abs() < 1e-5);
        let far = c.eval([std::f32::consts::FRAC_PI_2, 250.0, 250.0, 250.0], [1.0; 4]);
        assert!((far[1] - (0.0 * 1.35 + 1.65)).abs() < 1e-4);
    }
}
