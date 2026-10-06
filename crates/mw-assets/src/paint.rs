//! Car paints: the `Genesys.Gen.VehiclePaint` objects in a vehicle bundle.
//!
//! Field names were cracked with the paint shader's constant names as vocabulary (research log
//! 2026-10-04). The paint pixel shader (`VehicleNFS13_BodyPaint`) reads them through its object
//! constants: albedo = `PbrPaintDiffuseColour.rgb`, F0 = `PbrPaintSpecularColour.rgb`, and a clear coat
//! whose reflectance rises from its F0 at normal incidence towards 1 at grazing angles by its Fresnel
//! factor, with the base layer scaled by (1 - coat).

use mw_bundle::{Bundle, ResourceType};
use mw_formats::genesys::{Object, Value, field_hash};

use crate::{AssetStore, Result};

/// One paint option of a car. Colours are linear RGB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VehiclePaint {
    pub diffuse: [f32; 3],
    pub specular: [f32; 3],
    pub specular_roughness: f32,
    /// Clear coat reflectance at normal incidence (field `a1d177bd`, 0.019–0.05; unnamed, read as the
    /// coat F0 by its range and its place between `PbrClearcoatFresnel` and `PbrClearcoatRoughness`).
    pub clearcoat_specular: f32,
    pub clearcoat_fresnel: f32,
    pub clearcoat_roughness: f32,
    pub clearcoat: bool,
    pub pearlescent: [f32; 3],
    /// `DirtColour` / `DustColour`: the paint shaders' `GenericVector4_B` / `_C` colours of the dirt and
    /// dust layers (their `.w` is not read by the paint shaders).
    pub dirt_colour: [f32; 3],
    pub dust_colour: [f32; 3],
    /// `PaintColour`: the colour family index the menus group paints by.
    pub colour_index: i64,
}

fn vec3(o: &Object, name: &str) -> Option<[f32; 3]> {
    match o.get(field_hash(name))? {
        Value::Vector(v) => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

fn float(o: &Object, hash: u32) -> Option<f32> {
    match o.get(hash)? {
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

impl VehiclePaint {
    fn from_object(o: &Object) -> Option<Self> {
        Some(Self {
            diffuse: vec3(o, "PbrDiffuseColour")?,
            specular: vec3(o, "PbrSpecularColour")?,
            specular_roughness: float(o, field_hash("PbrSpecularRoughness")).unwrap_or(0.0),
            clearcoat_specular: float(o, 0xa1d177bd).unwrap_or(0.04),
            clearcoat_fresnel: float(o, field_hash("PbrClearcoatFresnel")).unwrap_or(0.5),
            clearcoat_roughness: float(o, field_hash("PbrClearcoatRoughness")).unwrap_or(0.0),
            clearcoat: !matches!(o.get(field_hash("PbrClearcoatEnabled")), Some(Value::Bool(false))),
            pearlescent: vec3(o, "PbrPearlescentColour").unwrap_or([0.0; 3]),
            dirt_colour: vec3(o, "DirtColour").unwrap_or([0.0; 3]),
            dust_colour: vec3(o, "DustColour").unwrap_or([0.0; 3]),
            colour_index: match o.get(field_hash("PaintColour")) {
                Some(Value::Int(i)) => *i,
                Some(Value::UInt(u)) => *u as i64,
                _ => -1,
            },
        })
    }
}

impl AssetStore {
    /// The paint options stored in a vehicle bundle, in resource ID order.
    pub fn vehicle_paints(&self, bundle: &Bundle) -> Result<Vec<VehiclePaint>> {
        let mut entries: Vec<_> = bundle.entries_of_type(ResourceType::GenesysObject).collect();
        entries.sort_by_key(|e| e.id.0);
        let mut out = Vec::new();
        for e in entries {
            let Ok(obj) = self.decode_genesys(&bundle.load(e)?) else { continue };
            if obj.type_name == "Genesys.Gen.VehiclePaint"
                && let Some(p) = VehiclePaint::from_object(&obj)
            {
                out.push(p);
            }
        }
        Ok(out)
    }
}
