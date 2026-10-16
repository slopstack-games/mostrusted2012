//! The light sources cars carry: `VehicleVfxBehaviour.Lights` (docs/decomp.md "Car lights on the world"). Each
//! entry (0x40 bytes) names a light definition (`Light_Spot` / `Light_Cone` / `Light_Point` / `Light_Custom` /
//! `Light_ProjectedTexture`), the car's locator group it sits at, its effect `Group` (a bit of the car's effect
//! mask), the day / night gate `da6ac384`, a colour (+0x10) and an intensity scale `a3de907e` (+0x34).
//!
//! **`Light_ProjectedTexture`** (0x285c, 160 bytes; offsets as the code reads them, `FUN_005818f0`):
//! +0x48 `AttenuationThreshold`, +0x4c `Intensity`, +0x60 `ed20dbe4` (Vector2: the field-of-view scale),
//! +0x70 `8745e6dd` (Vector3: the attenuation polynomial a, b, c of 1 / (a d² + b d + c)), +0x80 `664ab465`
//! (Vector4: the window of the light's clip space that is lit, left / bottom / right / top), +0x90 `Material`
//! (its texture is the light's `ProjectiveTextureSampler` cookie). The two Vector fields are named from their
//! use (`FovScale`, `Attenuation` are this module's names, not cracked hashes).

use std::collections::HashMap;
use std::sync::Arc;

use mw_bundle::{ResourceId, ResourceType};
use mw_formats::genesys::{Object, Value, field_hash};

use crate::AssetStore;
use crate::lights::LightDef;
use crate::police_lights::{FlashAnimation, Locator};

/// GeneSys type id of `Light_ProjectedTexture` (`FUN_005818f0` / `FUN_0054ade0` compare against it).
pub const PROJECTED_TEXTURE: u32 = 0x285c;

/// The projector part of a `Light_ProjectedTexture`.
#[derive(Debug, Clone, PartialEq)]
pub struct Projector {
    /// `ed20dbe4` (+0x60): scales the half-angles of the light's frustum (x, y).
    pub fov_scale: [f32; 2],
    /// `8745e6dd` (+0x70): a, b, c of the attenuation 1 / (a d² + b d + c) (`deferred_attenuation`).
    pub attenuation: [f32; 3],
    /// `664ab465` (+0x80): the lit window of the light's clip space (left, bottom, right, top); the game draws the
    /// light volume over this window only (`FUN_008b1860` → `FUN_008a1c10`).
    pub window: [f32; 4],
    /// `Material` (+0x90).
    pub material: Option<ResourceId>,
    /// The material's texture (the cookie bound to `ProjectiveTextureSampler`).
    pub texture: Option<ResourceId>,
}

/// One `VehicleVfxBehaviour.Light` with its definition and locators.
#[derive(Debug, Clone)]
pub struct CarLightSource {
    pub group: u8,
    /// `da6ac384`: 0 always, 1 by day, 2 at night (through `FUN_0089b400`).
    pub gate: u8,
    /// +0x10, multiplied into the definition's `Colour`.
    pub colour: [f32; 4],
    /// `a3de907e` (+0x34).
    pub scale: f32,
    pub animation: Option<Arc<FlashAnimation>>,
    pub def: Arc<LightDef>,
    /// Set for a `Light_ProjectedTexture`.
    pub projector: Option<Arc<Projector>>,
    pub locators: Vec<Locator>,
}

fn num(o: &Object, hash: u32) -> f32 {
    match o.get(hash) {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f32,
        Some(Value::UInt(u)) => *u as f32,
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

impl AssetStore {
    /// The projector of a `Light_ProjectedTexture` definition object.
    pub fn projector(&self, o: &Object) -> Option<Projector> {
        if o.type_name != "Genesys.Gen.Light_ProjectedTexture" {
            return None;
        }
        let fov = vector(o, 0xed20_dbe4);
        let att = vector(o, 0x8745_e6dd);
        let material = handle(o, 0x01e5_cd8e);
        let texture = material.and_then(|m| self.projector_texture(m));
        Some(Projector {
            fov_scale: [fov[0], fov[1]],
            attenuation: [att[0], att[1], att[2]],
            window: vector(o, 0x664a_b465),
            material,
            texture,
        })
    }

    /// The texture a projected light's material binds to `ProjectiveTextureSampler` (else its first texture).
    pub fn projector_texture(&self, material: ResourceId) -> Option<ResourceId> {
        let m = self.material(material).ok()?;
        let named = m.shader.and_then(|s| self.shader(s).ok()).and_then(|s| {
            let key = s.sampler_named("ProjectiveTextureSampler")?.key;
            m.samplers.iter().find(|x| x.key == key)?.texture
        });
        named.or_else(|| m.samplers.iter().find_map(|x| x.texture))
    }

    /// Every `VehicleVfxBehaviour.Lights` entry of a car's bundle.
    pub fn car_light_sources(
        &self,
        bundle: &mw_bundle::Bundle,
        spec: &mw_formats::VehicleGraphicsSpec,
    ) -> Vec<CarLightSource> {
        type Def = Option<(Arc<LightDef>, Option<Arc<Projector>>)>;
        let mut defs: HashMap<ResourceId, Def> = HashMap::new();
        let mut anims: HashMap<ResourceId, Option<Arc<FlashAnimation>>> = HashMap::new();
        let mut out = Vec::new();
        for e in bundle.entries_of_type(ResourceType::GenesysObject) {
            let Ok(res) = bundle.load(e) else { continue };
            let Ok(o) = self.decode_genesys(&res) else { continue };
            if o.type_name != "Genesys.Gen.VehicleVfxBehaviour" {
                continue;
            }
            let Some(Value::Array(lights)) = o.get(field_hash("Lights")) else { continue };
            for l in lights {
                let Value::Struct(l) = l else { continue };
                let Some(def_id) = handle(l, field_hash("LightDefinition")) else { continue };
                let Some((def, projector)) = defs
                    .entry(def_id)
                    .or_insert_with(|| {
                        let o = self.genesys_object(def_id).ok()?;
                        let def = LightDef::from_object(def_id, &o)?;
                        Some((Arc::new(def), self.projector(&o).map(Arc::new)))
                    })
                    .clone()
                else {
                    continue;
                };
                let animation = handle(l, field_hash("Animation"))
                    .and_then(|id| anims.entry(id).or_insert_with(|| self.flash_animation(id).map(Arc::new)).clone());
                out.push(CarLightSource {
                    group: num(l, field_hash("Group")) as u8,
                    gate: num(l, 0xda6a_c384) as u8,
                    colour: match l.get(field_hash("Colour")) {
                        Some(Value::Vector(v)) => *v,
                        _ => [1.0; 4],
                    },
                    scale: match l.get(0xa3de_907e) {
                        Some(_) => num(l, 0xa3de_907e),
                        None => 1.0,
                    },
                    animation,
                    def,
                    projector,
                    locators: spec
                        .locator_group(num(l, field_hash("LocatorGroup")) as u32)
                        .map(|g| g.locators.clone())
                        .unwrap_or_default(),
                });
            }
        }
        out
    }
}
