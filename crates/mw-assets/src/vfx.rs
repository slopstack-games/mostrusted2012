//! Which particle effects the game plays where: GeneSys objects that point at `BearEffect`s.
//!
//! - `HAWAII/SURFACELIST.BNDL`: `Surfaces.Surfaces` lists the 65 `SurfaceType`s (index = surface id,
//!   0 = `Default_surface`, the tarmac). Each surface has two weather conditions (dry first, then wet;
//!   field names not cracked), each with a `Surface_vfx` → `SurfaceVfx`: `WheelVfx` (kick-up from a
//!   rolling wheel), `PlayerWheelEffects` / `AIWheelEffects` → `VfxWheelEffects` (`DefaultTyre` = tyre
//!   smoke when sliding, `OnRims` = rim sparks, two unnamed), impact effects by direction
//!   (`FrontImpactEffect`, `FrontLeftImpactEffect`, ..., `TopImpactEffect`, `BottomImpactEffect`) and
//!   tyre mark materials.
//! - `GLOBALEFFECTS.BNDL`: `VfxGlobals` (`VehicleImpacts`, `VehicleScrapes` → `VfxVehicleCollisions`, 36
//!   unnamed effect handles each), `VfxMisc` (`BurstTyreWheelEffects`, `SpikeStripSparks`, ...).
//!
//! Field names: `genesys_names.txt` (cracked 2026-10-04, research log "particles").

use std::collections::HashMap;

use mw_bundle::{ResourceId, ResourceType};
use mw_formats::BearEffect;
use mw_formats::genesys::{Object, Value, field_hash};

use crate::{AssetStore, Error, Result};

/// `Surfaces` object field holding the surface list (`Surfaces`), and the two weather conditions of a
/// `SurfaceType` (unnamed; dry first).
const SURFACE_DRY: u32 = 0xf395b019;
const SURFACE_WET: u32 = 0x7348be25;

/// Effects for a car's wheels on one surface (`VfxWheelEffects`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WheelEffects {
    /// Tyre smoke / dust while the tyre slides or spins.
    pub default_tyre: Option<ResourceId>,
    /// Sparks while driving on the rims.
    pub on_rims: Option<ResourceId>,
    /// Fields `61ac1863` (a smoke variant, set on dirt and for the burst tyre) and `e7a7150c`.
    pub other: [Option<ResourceId>; 2],
}

/// The effects of one surface in one weather condition (`SurfaceVfx`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SurfaceEffects {
    /// Kick-up behind a rolling wheel (`WheelVfx`).
    pub wheel: Option<ResourceId>,
    pub player_wheels: WheelEffects,
    pub ai_wheels: WheelEffects,
    /// Impact effects by direction: front, front left, front right, rear, rear left, rear right,
    /// left, right, top, bottom.
    pub impacts: [Option<ResourceId>; 10],
    /// `TyreMarksMaterials` (`Vfx_TyreMarksNew` materials) by tyre: field `eff16a09`, `Offroad`, `Road`,
    /// `Sport`, `Track` (field order).
    pub tyre_marks: [Option<ResourceId>; 5],
}

pub const TYRE_MARKS_ROAD: usize = 2;

/// A `Vfx_TyreMarksNew` material: its three diffuse layers and material constants.
#[derive(Debug, Clone, PartialEq)]
pub struct TyreMarkMaterial {
    pub id: ResourceId,
    /// `RunDiffuseSampler`, `SlipDiffuseSampler`, `SpinDiffuseSampler`.
    pub textures: [Option<ResourceId>; 3],
    /// `WidthScale`, `Animation`, `Specular` (first component of each; `None` when unset).
    pub width_scale: Option<f32>,
    pub animation: Option<[f32; 4]>,
    pub specular: Option<f32>,
}

pub const IMPACT_FRONT: usize = 0;
pub const IMPACT_REAR: usize = 3;
pub const IMPACT_LEFT: usize = 6;
pub const IMPACT_RIGHT: usize = 7;
pub const IMPACT_TOP: usize = 8;
pub const IMPACT_BOTTOM: usize = 9;

const IMPACT_FIELDS: [&str; 10] = [
    "FrontImpactEffect",
    "FrontLeftImpactEffect",
    "FrontRightImpactEffect",
    "RearImpactEffect",
    "RearLeftImpactEffect",
    "RearRightImpactEffect",
    "LeftImpactEffect",
    "RightImpactEffect",
    "TopImpactEffect",
    "BottomImpactEffect",
];

#[derive(Debug, Clone, Default)]
pub struct Surface {
    pub dry: SurfaceEffects,
    pub wet: SurfaceEffects,
}

/// Everything the runtime needs to pick effects.
#[derive(Debug, Clone, Default)]
pub struct VfxLibrary {
    /// By surface id (`Surfaces.Surfaces` order; 0 = default tarmac).
    pub surfaces: Vec<Surface>,
    /// `VfxGlobals.VehicleImpacts` / `VehicleScrapes` (`VfxVehicleCollisions`, declaration order).
    pub vehicle_impacts: Vec<Option<ResourceId>>,
    pub vehicle_scrapes: Vec<Option<ResourceId>>,
    pub burst_tyre: WheelEffects,
}

/// A particle material: the `VfxParticles_*` shader and its diffuse texture / atlas settings.
#[derive(Debug, Clone, PartialEq)]
pub struct ParticleMaterial {
    pub id: ResourceId,
    pub shader: String,
    pub texture: Option<ResourceId>,
    /// `NumTiles` (atlas columns, rows) for the `_SubUV` shaders, else 1 × 1.
    pub tiles: [u32; 2],
    /// `EnableLooping`.
    pub looping: bool,
    /// `Overbright` (`VfxParticles_MotionBlurSpriteUntex`), else 1.
    pub overbright: f32,
}

/// One `VehicleVfxBehaviour.SpotEffect`: an effect played at each locator of a group while the car's
/// effect group bit is set (`FUN_005b28d0`; research log 2026-10-09 "exhaust effects").
#[derive(Debug, Clone)]
pub struct VehicleSpotEffect {
    pub effect: ResourceId,
    /// Bit in the car's effect mask (`FUN_0057a5e0`): 8 nitrous, 9 / 20 the nitrous upgrade's other modes,
    /// 12 wrecked, 14 backfire, 19 engine running.
    pub group: u8,
    /// `da6ac384`: 0 always, 1 only while the world flag +0x1b934 is clear, 2 only while it is set.
    pub condition: u8,
    /// Restarted whenever it ends while the group is on (otherwise started on the group's rising edge).
    pub looping: bool,
    /// Also played mirrored across the car's centre line.
    pub mirror: bool,
    pub locator_group: u32,
    pub locators: Vec<mw_formats::Locator>,
}

/// Effect group bits (`VehicleSpotEffect::group`).
pub mod spot_group {
    pub const NITROUS: u8 = 8;
    pub const WRECKED: u8 = 12;
    pub const BACKFIRE: u8 = 14;
    pub const ENGINE_RUNNING: u8 = 19;
}

fn handle(o: &Object, name: &str) -> Option<ResourceId> {
    handle_hash(o, field_hash(name))
}

fn handle_hash(o: &Object, hash: u32) -> Option<ResourceId> {
    match o.get(hash)? {
        Value::Handle(h) => *h,
        Value::Ref(r) => Some(*r),
        _ => None,
    }
}

fn handles(o: &Object) -> Vec<Option<ResourceId>> {
    o.fields
        .iter()
        .filter_map(|(_, v)| match v {
            Value::Handle(h) => Some(*h),
            _ => None,
        })
        .collect()
}

impl AssetStore {
    pub fn bear_effect(&self, id: ResourceId) -> Result<BearEffect> {
        let res = self.load(id)?;
        if res.resource_type() != ResourceType::BearEffect {
            return Err(Error::WrongType { id, expected: ResourceType::BearEffect, actual: res.resource_type() });
        }
        Ok(BearEffect::parse(&res)?)
    }

    pub fn particle_material(&self, id: ResourceId) -> Result<ParticleMaterial> {
        let material = self.material(id)?;
        let shader = self.shader(material.shader.ok_or(Error::Missing(id))?)?;
        let param = |name: &str| {
            let hash = shader.parameters.iter().find(|p| p.name == name).map(|p| p.hash)?;
            material
                .parameters
                .iter()
                .find(|p| p.name_hash == hash)
                .and_then(|p| p.value)
                .or_else(|| shader.parameters.iter().find(|p| p.hash == hash).and_then(|p| p.default))
        };
        let texture = shader
            .sampler_named("DiffuseSampler")
            .and_then(|s| material.samplers.iter().find(|m| m.key == s.key))
            .and_then(|m| m.texture);
        let tiles = param("NumTiles").map(|v| [v[0].max(1.0) as u32, v[1].max(1.0) as u32]).unwrap_or([1, 1]);
        Ok(ParticleMaterial {
            id,
            texture,
            tiles,
            looping: param("EnableLooping").is_some_and(|v| v[0] > 0.5),
            overbright: param("Overbright").map(|v| v[0]).unwrap_or(1.0),
            shader: shader.name,
        })
    }

    pub fn tyre_mark_material(&self, id: ResourceId) -> Result<TyreMarkMaterial> {
        let material = self.material(id)?;
        let shader = self.shader(material.shader.ok_or(Error::Missing(id))?)?;
        let param = |name: &str| {
            let hash = shader.parameters.iter().find(|p| p.name == name).map(|p| p.hash)?;
            material
                .parameters
                .iter()
                .find(|p| p.name_hash == hash)
                .and_then(|p| p.value)
                .or_else(|| shader.parameters.iter().find(|p| p.hash == hash).and_then(|p| p.default))
        };
        let texture = |name: &str| {
            shader
                .sampler_named(name)
                .and_then(|s| material.samplers.iter().find(|m| m.key == s.key))
                .and_then(|m| m.texture)
        };
        Ok(TyreMarkMaterial {
            id,
            textures: ["RunDiffuseSampler", "SlipDiffuseSampler", "SpinDiffuseSampler"].map(texture),
            width_scale: param("WidthScale").map(|v| v[0]),
            animation: param("Animation"),
            specular: param("Specular").map(|v| v[0]),
        })
    }

    /// Resolve the surface and global effect tables.
    pub fn vfx_library(&self) -> Result<VfxLibrary> {
        let mut lib = VfxLibrary::default();
        // Surfaces.
        let surfaces = self.genesys_objects_of("HAWAII/SURFACELIST.BNDL", "Genesys.Gen.Surfaces")?;
        let mut cache: HashMap<ResourceId, Object> = HashMap::new();
        let mut object = |id: ResourceId| -> Option<Object> {
            if let Some(o) = cache.get(&id) {
                return Some(o.clone());
            }
            let o = self.genesys_object(id).ok()?;
            cache.insert(id, o.clone());
            Some(o)
        };
        if let Some(list) = surfaces.first().and_then(|s| match s.get(field_hash("Surfaces")) {
            Some(Value::Array(a)) => Some(a.clone()),
            _ => None,
        }) {
            for v in list {
                let Value::Handle(Some(st)) = v else {
                    lib.surfaces.push(Surface::default());
                    continue;
                };
                let st = object(st);
                let effects = |cond: u32, object: &mut dyn FnMut(ResourceId) -> Option<Object>| -> SurfaceEffects {
                    let Some(Value::Struct(c)) = st.as_ref().and_then(|s| s.get(cond)) else {
                        return Default::default();
                    };
                    let Some(vfx) = handle(c, "Surface_vfx").and_then(&mut *object) else { return Default::default() };
                    let mut wheels = |name: &str| -> WheelEffects {
                        let Some(w) = handle(&vfx, name).and_then(&mut *object) else { return Default::default() };
                        WheelEffects {
                            default_tyre: handle(&w, "DefaultTyre"),
                            on_rims: handle(&w, "OnRims"),
                            other: [handle_hash(&w, 0x61ac1863), handle_hash(&w, 0xe7a7150c)],
                        }
                    };
                    let player_wheels = wheels("PlayerWheelEffects");
                    let ai_wheels = wheels("AIWheelEffects");
                    SurfaceEffects {
                        wheel: handle(&vfx, "WheelVfx"),
                        player_wheels,
                        ai_wheels,
                        impacts: IMPACT_FIELDS.map(|n| handle(&vfx, n)),
                        tyre_marks: match vfx.get(0x75cb5de9) {
                            Some(Value::Struct(t)) => [
                                0xeff16a09,
                                field_hash("Offroad"),
                                field_hash("Road"),
                                field_hash("Sport"),
                                field_hash("Track"),
                            ]
                            .map(|h| handle_hash(t, h)),
                            _ => [None; 5],
                        },
                    }
                };
                let dry = effects(SURFACE_DRY, &mut object);
                let wet = effects(SURFACE_WET, &mut object);
                lib.surfaces.push(Surface { dry, wet });
            }
        }
        // Globals.
        if let Some(g) = self.genesys_objects_of("GLOBALEFFECTS.BNDL", "Genesys.Gen.VfxGlobals")?.first() {
            let list = |name: &str, object: &mut dyn FnMut(ResourceId) -> Option<Object>| {
                handle(g, name).and_then(object).map(|o| handles(&o)).unwrap_or_default()
            };
            lib.vehicle_impacts = list("VehicleImpacts", &mut object);
            lib.vehicle_scrapes = list("VehicleScrapes", &mut object);
            if let Some(misc) = handle(g, "Misc").and_then(&mut object)
                && let Some(w) = handle(&misc, "BurstTyreWheelEffects").and_then(&mut object)
            {
                lib.burst_tyre = WheelEffects {
                    default_tyre: handle(&w, "DefaultTyre"),
                    on_rims: handle(&w, "OnRims"),
                    other: [handle_hash(&w, 0x61ac1863), handle_hash(&w, 0xe7a7150c)],
                };
            }
        }
        Ok(lib)
    }

    /// A car's spot effects (`VehicleVfxBehaviour.SpotEffects` in its bundle), each with the locators of its
    /// `LocatorGroup` from the car's `VehicleGraphicsSpec`.
    pub fn vehicle_spot_effects(
        &self,
        bundle: &mw_bundle::Bundle,
        spec: &mw_formats::VehicleGraphicsSpec,
    ) -> Result<Vec<VehicleSpotEffect>> {
        let mut out = Vec::new();
        for e in bundle.entries_of_type(ResourceType::GenesysObject) {
            let Ok(o) = self.decode_genesys(&bundle.load(e)?) else { continue };
            if o.type_name != "Genesys.Gen.VehicleVfxBehaviour" {
                continue;
            }
            let Some(Value::Array(list)) = o.get(field_hash("SpotEffects")) else { continue };
            for v in list {
                let Value::Struct(s) = v else { continue };
                let Some(effect) = handle(s, "SpotEffect") else { continue };
                let int = |name: &str| match s.get(field_hash(name)) {
                    Some(Value::Int(i)) => *i as u64,
                    Some(Value::UInt(u)) => *u,
                    _ => 0,
                };
                let flag = |name: &str| matches!(s.get(field_hash(name)), Some(Value::Bool(true)));
                let group_id = int("LocatorGroup") as u32;
                out.push(VehicleSpotEffect {
                    effect,
                    group: int("Group") as u8,
                    condition: match s.get(0xda6a_c384) {
                        Some(Value::Int(i)) => *i as u8,
                        Some(Value::UInt(u)) => *u as u8,
                        _ => 0,
                    },
                    looping: flag("Looping"),
                    mirror: flag("Mirror"),
                    locator_group: group_id,
                    locators: spec.locator_group(group_id).map(|g| g.locators.clone()).unwrap_or_default(),
                });
            }
        }
        Ok(out)
    }

    /// Decoded GeneSys objects of one type in a bundle.
    fn genesys_objects_of(&self, bundle: &str, type_name: &str) -> Result<Vec<Object>> {
        let b = self.bundle(bundle).ok_or(Error::Missing(ResourceId::NULL))?;
        let mut out = Vec::new();
        for e in b.entries_of_type(ResourceType::GenesysObject) {
            if let Ok(o) = self.decode_genesys(&b.load(e)?)
                && o.type_name == type_name
            {
                out.push(o);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mw_gamedir::GameDir;

    #[test]
    fn library_resolves_against_the_install() {
        let Ok(game) = GameDir::locate(None) else {
            eprintln!("no game install found; skipping");
            return;
        };
        let store = AssetStore::new(game).unwrap();
        let lib = store.vfx_library().unwrap();
        assert_eq!(lib.surfaces.len(), 65);
        let tarmac = &lib.surfaces[0].dry;
        let smoke = tarmac.player_wheels.default_tyre.expect("tarmac tyre smoke");
        let fx = store.bear_effect(smoke).unwrap();
        assert!(fx.param("WheelParams").is_some());
        assert!(fx.layers.iter().all(|l| l.renderer().is_some()));
        assert_eq!(lib.vehicle_impacts.len(), 36);
        // Every effect the tables name parses, and every renderer's material resolves.
        let mut ids: Vec<ResourceId> = lib
            .surfaces
            .iter()
            .flat_map(|s| [&s.dry, &s.wet])
            .flat_map(|e| [e.wheel, e.player_wheels.default_tyre, e.player_wheels.on_rims].into_iter().chain(e.impacts))
            .chain(lib.vehicle_impacts.iter().copied())
            .chain(lib.vehicle_scrapes.iter().copied())
            .flatten()
            .collect();
        ids.sort();
        ids.dedup();
        for id in ids {
            let fx = store.bear_effect(id).unwrap();
            for l in &fx.layers {
                if let Some(m) = l.renderer().and_then(|r| r.material)
                    && store.type_of(m) == Some(ResourceType::Material)
                {
                    store.particle_material(m).unwrap();
                }
            }
        }
    }
}
