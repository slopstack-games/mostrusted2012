//! The global surface list: what the surface index in a collision polygon's tag refers to.
//!
//! `HAWAII/SURFACELIST.BNDL` holds one `Genesys.Gen.Surfaces` object whose `Surfaces` array (65
//! handles) is the list that `mw_formats::CollisionTag::surface` indexes. Each entry is a
//! `SurfaceType`: a debug `Colour`, two `Surface_weather_conditions` blocks (`f395b019`, `7348be25`;
//! probably dry and wet) with per-contact-type (`Normal`, `Flats`, `Rims`, `Offroad`, `Track`)
//! `Surface_Physics` / `Surface_Rumble` / `Surface_Sounds` handles and a `Surface_vfx` handle, plus
//! `Collision_Sounds` and `Collision_Rumble`. `SurfacePhysics` gives `Surface_Grip` per tyre class
//! (`RoadOverrides` and four unnamed ones), `Surface_speed` and `Surface_roughness`. Surface names
//! are not in the data.

use mw_bundle::{ResourceId, ResourceType};
use mw_formats::genesys::{Object, Value, field_hash};

use crate::{AssetStore, Error, Result};

/// One entry of the surface list.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceInfo {
    /// The `SurfaceType` object.
    pub id: ResourceId,
    /// Debug colour (`Colour`, linear RGB).
    pub colour: [f32; 3],
    /// `Surface_Grip` of the normal contact's `RoadOverrides` in the first weather block (1.0 = full
    /// grip; 0.735 on rims).
    pub grip: Option<f32>,
    /// `Surface_speed` of the same `SurfacePhysics` (a small enum, 1 on most surfaces).
    pub speed: Option<u32>,
    /// `Surface_roughness` (two unnamed floats).
    pub roughness: Option<[f32; 2]>,
    /// The normal contact's `Surface_Sounds` / `Surface_vfx` handles (first weather block), for audio
    /// and particle systems.
    pub sounds: Option<ResourceId>,
    pub vfx: Option<ResourceId>,
    pub collision_sounds: Option<ResourceId>,
}

const DRY: u32 = 0xf395b019;

fn get<'a>(o: &'a Object, name: &str) -> Option<&'a Value> {
    o.get(field_hash(name))
}

fn sub(o: &Object, hash: u32) -> Option<&Object> {
    match o.get(hash)? {
        Value::Struct(s) => Some(s),
        _ => None,
    }
}

fn handle(v: Option<&Value>) -> Option<ResourceId> {
    match v? {
        Value::Handle(h) => *h,
        Value::Ref(id) => Some(*id),
        _ => None,
    }
}

fn float(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

impl AssetStore {
    /// The surface list, in collision-tag index order.
    pub fn surfaces(&self) -> Result<Vec<SurfaceInfo>> {
        let b = self.bundle("HAWAII/SURFACELIST.BNDL").ok_or(Error::Missing(ResourceId::NULL))?;
        let mut list = None;
        for e in b.entries_of_type(ResourceType::GenesysObject) {
            let o = self.decode_genesys(&b.load(e)?)?;
            if o.type_name == "Genesys.Gen.Surfaces" {
                list = Some(o);
                break;
            }
        }
        let list = list.ok_or(Error::Missing(ResourceId::NULL))?;
        let handles: Vec<Option<ResourceId>> = match get(&list, "Surfaces") {
            Some(Value::Array(a)) => a.iter().map(|v| handle(Some(v))).collect(),
            _ => Vec::new(),
        };
        let mut out = Vec::with_capacity(handles.len());
        for h in handles {
            let Some(id) = h else { continue };
            let s = self.genesys_object(id)?;
            let colour = match get(&s, "Colour") {
                Some(Value::Vector(c)) => [c[0], c[1], c[2]],
                _ => [0.0; 3],
            };
            let dry = sub(&s, DRY);
            let normal = dry.and_then(|d| sub(d, field_hash("Normal")));
            let physics =
                handle(normal.and_then(|n| get(n, "Surface_Physics"))).and_then(|p| self.genesys_object(p).ok());
            let road = physics.as_ref().and_then(|p| sub(p, field_hash("RoadOverrides")));
            let roughness = physics.as_ref().and_then(|p| sub(p, field_hash("Surface_roughness"))).map(|r| {
                let f = r.floats();
                [f.first().copied().unwrap_or(0.0), f.get(1).copied().unwrap_or(0.0)]
            });
            out.push(SurfaceInfo {
                id,
                colour,
                grip: float(road.and_then(|r| get(r, "Surface_Grip"))),
                speed: physics.as_ref().and_then(|p| match get(p, "Surface_speed")? {
                    Value::UInt(v) => u32::try_from(*v).ok(),
                    Value::Int(v) => u32::try_from(*v).ok(),
                    _ => None,
                }),
                roughness,
                sounds: handle(normal.and_then(|n| get(n, "Surface_Sounds"))),
                vfx: handle(dry.and_then(|d| get(d, "Surface_vfx"))),
                collision_sounds: handle(get(&s, "Collision_Sounds")),
            });
        }
        Ok(out)
    }
}
