//! World placement: `InstanceList` (0x50).
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | ptr  | instances |
//! | 0x04 | u32  | array size |
//! | 0x08 | u32  | always-loaded instance count (the rest are backdrop) |
//! | 0x0C | u32  | version (3) |
//!
//! Instance (0x60 bytes):
//!
//! | off  | type     | field |
//! |------|----------|-------|
//! | 0x00 | ptr      | model (import) |
//! | 0x04 | f32      | unknown (draw distance / LOD bias?) |
//! | 0x08 | u32      | unknown |
//! | 0x10 | ptr      | per-instance material overrides ([`InstanceOverrides`]), or 0 |
//! | 0x20 | f32×4×4  | transform, row-major, rows = x/y/z axes then translation |
//!
//! The transform's w column is not 0/0/0/1: the world vertex shaders pass it to the pixel shader, which
//! multiplies the albedo by its xyz (instance tint) and the AO map by its w (instance AO scale).
//!
//! Override block (0x20 bytes, data follows it in the same chunk):
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | u8   | 1 |
//! | 0x01 | u8   | constant count |
//! | 0x02 | u8   | sampler count |
//! | 0x04 | ptr  | constant names (array of string pointers) |
//! | 0x08 | ptr  | constant slots (u8 each, 0xff) |
//! | 0x0C | ptr  | constant values (f32×4 each) |
//! | 0x10 | ptr  | sampler names (array of string pointers) |
//! | 0x14 | ptr  | sampler slots (u8 each, 0xff) |
//! | 0x18 | ptr  | SamplerState imports (u32 each) |
//! | 0x1C | ptr  | Texture imports (u32 each) |

use mw_bundle::{Reader, Resource, ResourceId};

use crate::Result;

#[derive(Debug, Clone)]
pub struct Instance {
    pub model: Option<ResourceId>,
    /// Row-major; row 3 is the translation (D3D convention, `v * M`).
    pub transform: [[f32; 4]; 4],
    pub always_loaded: bool,
    pub unknown_04: f32,
    pub unknown_08: u32,
    pub overrides: InstanceOverrides,
}

/// Material constants and textures that one placed instance overrides, by shader parameter / sampler
/// name (`diffuseTintColour`, `AOMapTextureSampler`, `LightMapTextureSampler`, ...).
#[derive(Debug, Clone, Default)]
pub struct InstanceOverrides {
    pub constants: Vec<(String, [f32; 4])>,
    pub textures: Vec<(String, ResourceId)>,
}

impl InstanceOverrides {
    fn parse(res: &Resource, o: usize) -> Result<Self> {
        let r = Reader(res.header());
        let mut out = Self::default();
        if o == 0 {
            return Ok(out);
        }
        let n_const = r.u8(o + 1)? as usize;
        let n_samp = r.u8(o + 2)? as usize;
        let name = |table: usize, i: usize| -> Result<String> { cstr(r, r.u32(table + i * 4)? as usize) };
        let [cn, _, cv, sn, _, _, tx] = r.u32_array::<7>(o + 4)?.map(|v| v as usize);
        for i in 0..n_const {
            out.constants.push((name(cn, i)?, r.f32_array(cv + i * 16)?));
        }
        for i in 0..n_samp {
            if let Some(tex) = res.import_at((tx + i * 4) as u32) {
                out.textures.push((name(sn, i)?, tex));
            }
        }
        Ok(out)
    }

    pub fn constant(&self, name: &str) -> Option<[f32; 4]> {
        self.constants.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| *v)
    }

    pub fn texture(&self, name: &str) -> Option<ResourceId> {
        self.textures.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, t)| *t)
    }
}

/// A NUL-terminated string (at most 256 bytes).
fn cstr(r: Reader, o: usize) -> Result<String> {
    let s = r.bytes(o, 256.min(r.0.len().saturating_sub(o)))?;
    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    Ok(String::from_utf8_lossy(&s[..end]).into_owned())
}

impl Instance {
    /// Transform a point (`v * M`).
    pub fn transform_point(&self, p: [f32; 3]) -> [f32; 3] {
        let m = &self.transform;
        std::array::from_fn(|c| p[0] * m[0][c] + p[1] * m[1][c] + p[2] * m[2][c] + m[3][c])
    }
}

#[derive(Debug, Clone)]
pub struct InstanceList {
    pub instances: Vec<Instance>,
}

impl InstanceList {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let base = r.u32(0x00)?;
        let count = r.u32(0x04)?;
        let always = r.u32(0x08)?;
        let mut instances = Vec::with_capacity(count as usize);
        for i in 0..count {
            let o = base + i * 0x60;
            let mut transform = [[0.0; 4]; 4];
            for (row, t) in transform.iter_mut().enumerate() {
                *t = r.f32_array(o as usize + 0x20 + row * 16)?;
            }
            instances.push(Instance {
                model: res.import_at(o),
                transform,
                always_loaded: i < always,
                unknown_04: r.f32(o as usize + 4)?,
                unknown_08: r.u32(o as usize + 8)?,
                overrides: InstanceOverrides::parse(res, r.u32(o as usize + 0x10)? as usize)?,
            });
        }
        Ok(Self { instances })
    }
}

/// One placed object in a `PropInstanceList`, `CompoundInstanceList`,
/// `DynamicInstanceList` or `LightInstanceList`. All four share one layout:
///
/// | off  | type    | field |
/// |------|---------|-------|
/// | 0x00 | f32×4×4 | transform: x/y/z axis rows then translation (same convention as `InstanceList`); column 3 is not 0/0/0/1 and is ignored (translation-row w is often 0.94, probably a packed tint/AO byte) |
/// | 0x40 | ptr     | object (import): `PropObject`, `CompoundObject`, `WorldObject`, or (lights) a GeneSys handle |
/// | 0x48/0x4C | u32 | instance id (prop/dynamic at 0x48, compound/light at 0x4C), then f32 1.0 or 0 |
/// | 0x50 | ptr     | extra per-instance data in the second half of the chunk (props: a `InstanceAdditiveAndAO` block); not decoded |
///
/// List header (0x10 bytes): `u32 version` (prop 2, compound 2, dynamic 3, light 1), `ptr` to the
/// instances (0x10), `u32 count`, `u32 0`. Empty lists have a 0x10-byte chunk with a null pointer.
#[derive(Debug, Clone)]
pub struct ObjectInstance {
    pub object: Option<ResourceId>,
    /// Row-major with column 3 forced to (0, 0, 0, 1); row 3 is the translation.
    pub transform: [[f32; 4]; 4],
}

#[derive(Debug, Clone)]
pub struct ObjectInstanceList {
    pub version: u32,
    pub instances: Vec<ObjectInstance>,
}

/// Size of one [`ObjectInstance`] record.
const OBJECT_INSTANCE_SIZE: usize = 0x60;
/// Sanity limit so a corrupt count cannot trigger a huge allocation.
const MAX_OBJECT_INSTANCES: usize = 1 << 20;

impl ObjectInstanceList {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let version = r.u32(0x00)?;
        let base = r.u32(0x04)? as usize;
        let count = r.u32(0x08)? as usize;
        if count > MAX_OBJECT_INSTANCES {
            return Err(crate::Error::Malformed(format!("object instance count {count}")));
        }
        let mut instances = Vec::with_capacity(count);
        for i in 0..count {
            let o = base + i * OBJECT_INSTANCE_SIZE;
            let mut transform = [[0.0; 4]; 4];
            for (row, t) in transform.iter_mut().enumerate() {
                let [x, y, z, _] = r.f32_array(o + row * 16)?;
                *t = [x, y, z, if row == 3 { 1.0 } else { 0.0 }];
            }
            instances.push(ObjectInstance { object: res.import_at(o as u32 + 0x40), transform });
        }
        Ok(Self { version, instances })
    }
}

/// What an object definition (`PropObject`, `CompoundObject`, `WorldObject`) draws and how it behaves
/// physically. Field names in backticks are from burnout.wiki (2012-09-14 Hawaii debug symbols).
///
/// `PropObject` (`Game::World::PropObject`, 0x70-0x80 bytes):
///
/// | off  | type   | field |
/// |------|--------|-------|
/// | 0x00 | u32    | `muVersionNumber` (1) |
/// | 0x04 | ptr    | `mpModel` (import) |
/// | 0x08 | ptr    | `mpDamagedModel` (import; the same model in every instance seen) |
/// | 0x0C | handle | `mPhysicalDefinitionHandle`: GeneSys `PhysicalDefinition` |
/// | 0x14 | handle | `mImpactEffectHandle` (GeneSys, VFX) |
/// | 0x1C | handle | `mImpactSoundHandle` (GeneSys, audio) |
/// | 0x24 | f32    | `mfResetDistanceSquared` (10000 = 100 m) |
/// | 0x28 | bool   | `mbRemoveInsteadOfReset` |
///
/// `WorldObject` (`Game::World::WorldObject`, 0x40-0x4A0 bytes): u8 version (8) at 0x00, u8
/// `muNumBehaviours` at 0x01, bool `mbIsStack` at 0x02, `mpModel` at 0x04, `mpSkeleton` at 0x08,
/// `maBehaviourSpecs` ptr at 0x0C (0x14-byte entries: GeneSys behaviour handle, u32 type, u32 hash),
/// `mLocators` at 0x10 (u32 count, ptr ids, ptr locator lists), `mPhysicalDefinitionHandle` at 0x1C
/// (absent on trigger-like objects without collision).
///
/// `CompoundObject` (0x240-0x1150 bytes) starts like a `WorldObject` (version 8, behaviour count at
/// 0x01, model at 0x04, physical definition at 0x1C, locators at 0x10), plus a GeneSys handle at 0x08,
/// a second model at 0x24 (lower-poly twin of 0x04, same bbox) and sometimes 0x28, and at 0x30 a
/// pointer to `u32` part count at 0x38 parts. A part is 0x70 bytes: an embedded `WorldObject` (no model
/// of its own, its own physical definition at +0x1C, behaviours, locators), `(1,1,1,1)` at +0x4C, a u32
/// part number at +0x60 and four u8 links at +0x64 (other part numbers, 0xFF = none; probably what each
/// part hangs from). 0x34 points to one u32 hash per part.
#[derive(Debug, Clone)]
pub struct ObjectDef {
    pub id: ResourceId,
    /// Model imports in slot order (see the layout above); `None` for an empty slot.
    pub models: Vec<Option<ResourceId>>,
    /// The `PhysicalDefinition` GeneSys object (collision volumes, mass).
    pub physics: Option<ResourceId>,
    /// Compound objects: the physical definition of each part.
    pub parts: Vec<Option<ResourceId>>,
    /// Props: `mfResetDistanceSquared` (m²).
    pub reset_distance_sq: Option<f32>,
    /// Props: `mbRemoveInsteadOfReset`.
    pub remove_instead_of_reset: bool,
}

/// Size of one `CompoundObject` part record.
const COMPOUND_PART_SIZE: usize = 0x70;

impl ObjectDef {
    fn parse_slots(res: &Resource, slots: &[u32], physics: u32) -> Result<Self> {
        // The header must at least reach the first slot.
        Reader(res.header()).u32(slots[0] as usize)?;
        Ok(Self {
            id: res.id,
            models: slots.iter().map(|&o| res.import_at(o)).collect(),
            physics: res.import_at(physics),
            parts: Vec::new(),
            reset_distance_sq: None,
            remove_instead_of_reset: false,
        })
    }

    pub fn parse_prop(res: &Resource) -> Result<Self> {
        let mut def = Self::parse_slots(res, &[0x04, 0x08], 0x0C)?;
        let r = Reader(res.header());
        def.reset_distance_sq = r.f32(0x24).ok();
        def.remove_instead_of_reset = r.u8(0x28).is_ok_and(|b| b != 0);
        Ok(def)
    }
    pub fn parse_compound(res: &Resource) -> Result<Self> {
        let mut def = Self::parse_slots(res, &[0x04, 0x24, 0x28], 0x1C)?;
        let r = Reader(res.header());
        let (base, count) = (r.u32(0x30)? as usize, r.u32(0x38)? as usize);
        if count > 256 {
            return Err(crate::Error::Malformed(format!("compound part count {count}")));
        }
        for i in 0..count {
            let o = base + i * COMPOUND_PART_SIZE;
            r.u32(o + COMPOUND_PART_SIZE - 4)?;
            def.parts.push(res.import_at((o + 0x1C) as u32));
        }
        Ok(def)
    }
    pub fn parse_world_object(res: &Resource) -> Result<Self> {
        Self::parse_slots(res, &[0x04], 0x1C)
    }

    /// The model to draw: the first populated slot.
    pub fn model(&self) -> Option<ResourceId> {
        self.models.iter().flatten().next().copied()
    }
}
