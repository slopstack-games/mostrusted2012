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
//! | 0x04 | f32      | LOD distance scale ([`Instance::lod_scale`]; 1.0 for most, up to 8) |
//! | 0x08 | u32      | unknown (unique per instance) |
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
    /// Multiplies the camera distance limits of the model's LODs for this placement: the draw path
    /// (`FUN_008a02f0`, called from `FUN_00550020` with this field) divides the distance by it before
    /// comparing with [`crate::Model::lod_distances`] (docs/decomp.md "Model LOD and draw distance").
    pub lod_scale: f32,
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
    pub(crate) fn parse(res: &Resource, o: usize) -> Result<Self> {
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
            if let Some(tex) = u32::try_from(tx + i * 4).ok().and_then(|o| res.import_at(o)) {
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
        let mut instances = Vec::with_capacity((count as usize).min(4096));
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
                lod_scale: r.f32(o as usize + 4)?,
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
/// | 0x48/0x4C | u32 | instance id (prop/dynamic at 0x48, compound/light at 0x4C) |
/// | 0x4C/0x50 | f32 | LOD distance scale after the id ([`ObjectInstance::lod_scale`]; 1.0 for most, 0.9–10) |
/// | 0x50 | ptr     | extra per-instance data in the second half of the chunk (props: a `InstanceAdditiveAndAO` block); not decoded |
/// | 0x54 | ptr     | compounds: a material-override block ([`ObjectInstance::overrides`]), 0 for most |
///
/// List header (0x10 bytes): `u32 version` (prop 2, compound 2, dynamic 3, light 1), `ptr` to the
/// instances (0x10), `u32 count`, `u32 0`. Empty lists have a 0x10-byte chunk with a null pointer.
#[derive(Debug, Clone)]
pub struct ObjectInstance {
    pub object: Option<ResourceId>,
    /// Row-major with column 3 forced to (0, 0, 0, 1); row 3 is the translation.
    pub transform: [[f32; 4]; 4],
    /// The words at +0x48 and +0x4c: the instance id (props / dynamics in the first, compounds / lights in the
    /// second). On a billboard's or a security gate's compound it is the id of the object's `TriggerData`
    /// record (checked on every billboard and gate, `mwtool freeroam --objects`): the "linked trigger" the game
    /// reads at the smashable object's +0x90 (`FUN_005b5340`).
    pub ids: [u32; 2],
    /// The float after the instance id: the props' LOD distance scale (`FUN_00550020` passes record +0x4C of
    /// the prop list to the model draw `FUN_008a02f0`, like `InstanceList` +0x04).
    pub lod_scale: f32,
    /// Compound lists only: the material overrides of the block at +0x54 (laid out like an `InstanceList`
    /// instance's, see the module docs). The 216 billboards are the only compounds with one: `DiffuseSampler` =
    /// texture `0100000000141784` (a silhouette on a torn poster), the picture slot of the found billboard's
    /// `World_SmashedBillboard_InstanceDiffuse` material. The game binds the block's constants and textures
    /// per object before the model draw (`FUN_00550020`: `FUN_00a621a0` / `FUN_00a62820` per entry).
    pub overrides: InstanceOverrides,
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
        // The id sits at +0x48 in prop / dynamic lists and at +0x4C in compound / light lists; the scale follows.
        let scale_at = match res.resource_type() {
            mw_bundle::ResourceType::CompoundInstanceList | mw_bundle::ResourceType::LightInstanceList => 0x50,
            _ => 0x4C,
        };
        let compound = res.resource_type() == mw_bundle::ResourceType::CompoundInstanceList;
        let mut instances = Vec::with_capacity(count);
        for i in 0..count {
            let o = base + i * OBJECT_INSTANCE_SIZE;
            let mut transform = [[0.0; 4]; 4];
            for (row, t) in transform.iter_mut().enumerate() {
                let [x, y, z, _] = r.f32_array(o + row * 16)?;
                *t = [x, y, z, if row == 3 { 1.0 } else { 0.0 }];
            }
            instances.push(ObjectInstance {
                object: res.import_at(o as u32 + 0x40),
                transform,
                ids: [r.u32(o + 0x48)?, r.u32(o + 0x4c)?],
                lod_scale: r.f32(o + scale_at)?,
                // Compound lists: the material-override block after the scale (unreadable blocks: none).
                overrides: match (compound, r.u32(o + 0x54)) {
                    (true, Ok(block)) => InstanceOverrides::parse(res, block as usize).unwrap_or_default(),
                    _ => InstanceOverrides::default(),
                },
            });
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
/// | 0x14 | handle | `mImpactEffectHandle`: a `BearEffect` (`GLOBALEFFECTS.BNDL`) |
/// | 0x1C | handle | `mImpactSoundHandle`: a `WCPlaySoundBehaviour` (GeneSys) |
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
/// 0x01, model at 0x04, `mpSkeleton` at 0x08 (the model is skinned to it: one bone per part), behaviours at
/// 0x0C, locators at 0x10, physical definition at 0x1C), plus the model it stands as at 0x24 (unskinned,
/// same bbox as 0x04, which is drawn only once broken) and on billboards 0x28 (the found billboard), a pointer to 8-byte joint handles at 0x2C (count at 0x3C), and at
/// 0x30 a pointer to `u32` part count at 0x38 parts; 0x34 points to one u32 hash per part (the joints name
/// their two parts by these). A part is 0x70 bytes: an embedded `WorldObject` (no model of its own,
/// behaviours at +0x0C with their count at +0x01, its own physical definition at +0x1C, locators),
/// `(1,1,1,1)` at +0x4C, its skeleton bone at +0x60 (`FUN_005341a0` → part +0x134, posed by `FUN_00532b00`),
/// three u8 part indices at +0x64 / +0x65 / +0x66 (0xFF = none): the part this one replaces when that smashes
/// (0xFF = present from the start, `FUN_005547f0`), the next part of the same replacement chain, and the
/// first part that replaces this one (message 3 in `FUN_0055a680`); and at +0x68 a pointer to +0x6C u8
/// part indices this part is linked to (a pinned part comes loose once all of them are loose,
/// `FUN_0054e750`). Behaviour entries are 0x14 bytes: GeneSys import at +0x00 (its id again at +0x08),
/// type id at +0x0C (0x5A1F2 `EnableCompoundBehaviour`, 0x6909C `SmashProxyBehaviour`, 0x5D79D
/// `MakePhysicalBehaviour`, 0x2AD8 `WCPlaySoundBehaviour`, 0x2AE0 `WCVfxBehaviour`), a hash at +0x10.
#[derive(Debug, Clone)]
pub struct ObjectDef {
    pub id: ResourceId,
    /// Model imports in slot order (see the layout above); `None` for an empty slot.
    pub models: Vec<Option<ResourceId>>,
    /// The `PhysicalDefinition` GeneSys object (collision volumes, mass).
    pub physics: Option<ResourceId>,
    /// Compound objects: the physical definition of each part.
    pub parts: Vec<Option<ResourceId>>,
    /// Compound objects: everything else about each part (same order as [`Self::parts`]).
    pub compound_parts: Vec<CompoundPart>,
    /// Compound objects: the skeleton the model is skinned to (0x08).
    pub skeleton: Option<ResourceId>,
    /// Compound objects: joint definitions between parts (GeneSys imports, 0x2C / 0x3C).
    pub joints: Vec<Option<ResourceId>>,
    /// Compound and world objects: behaviours (0x0C, count at 0x01).
    pub behaviours: Vec<Behaviour>,
    /// Props: `mfResetDistanceSquared` (m²).
    pub reset_distance_sq: Option<f32>,
    /// Props: `mbRemoveInsteadOfReset`.
    pub remove_instead_of_reset: bool,
    /// Props: the impact effect (+0x14, a `BearEffect` in `GLOBALEFFECTS.BNDL`; 81% of props have one).
    pub impact_effect: Option<ResourceId>,
    /// Props: the impact sound (+0x1c, a `WCPlaySoundBehaviour` GeneSys object with `Collision_Waves`).
    pub impact_sound: Option<ResourceId>,
    /// Compound and world objects: `mLocators` (0x10: u32 count, ptr ids, ptr `{u32 n, ptr locators, u32}`;
    /// a locator is a quaternion and a position in object space, the layout of a car's locator groups):
    /// where the object's `WCVfxBehaviour` lights, coronas and spot effects sit (a double street lamp has one
    /// group with a locator at each head, pointing down).
    pub locator_groups: Vec<crate::vehicle::LocatorGroup>,
    /// Compound objects: the same per part (same order as [`Self::parts`]).
    pub part_locator_groups: Vec<Vec<crate::vehicle::LocatorGroup>>,
}

/// `WCVfxBehaviour` type id.
pub const WC_VFX_BEHAVIOUR: u32 = 0x2ae0;

/// The `mLocators` block of the (embedded) `WorldObject` at `base`. Unreadable blocks give no groups.
fn locator_groups(res: &Resource, base: usize) -> Vec<crate::vehicle::LocatorGroup> {
    use crate::vehicle::{Locator, LocatorGroup};
    let r = Reader(res.header());
    let read = || -> Result<Vec<LocatorGroup>> {
        let (count, ids, groups) =
            (r.u32(base + 0x10)? as usize, r.u32(base + 0x14)? as usize, r.u32(base + 0x18)? as usize);
        if count == 0 || count > 256 || ids == 0 || groups == 0 {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(count);
        for g in 0..count {
            let n = (r.u32(groups + g * 12)? as usize).min(64);
            let at = r.u32(groups + g * 12 + 4)? as usize;
            let locators = (0..n)
                .map(|k| {
                    let [x, y, z, _] = r.f32_array::<4>(at + k * 32 + 16)?;
                    Ok(Locator { rotation: r.f32_array(at + k * 32)?, position: [x, y, z] })
                })
                .collect::<Result<_>>()?;
            out.push(LocatorGroup { id: r.u32(ids + g * 4)?, locators });
        }
        Ok(out)
    };
    read().unwrap_or_default()
}

/// A behaviour of a world object or compound part (`maBehaviourSpecs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Behaviour {
    /// The behaviour's GeneSys object.
    pub object: Option<ResourceId>,
    /// GeneSys type id (low bits of the type's resource id).
    pub type_id: u32,
    pub hash: u32,
}

/// `EnableCompoundBehaviour` type id.
pub const ENABLE_COMPOUND_BEHAVIOUR: u32 = 0x5a1f2;
/// `SmashProxyBehaviour` type id.
pub const SMASH_PROXY_BEHAVIOUR: u32 = 0x6909c;

/// One part of a `CompoundObject` (see [`ObjectDef`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompoundPart {
    /// The part's hash (0x34 list).
    pub hash: u32,
    /// Skeleton bone the part's geometry is skinned to and whose bind pose places it (+0x60).
    pub bone: u32,
    /// The part index this one replaces when that part smashes (+0x64); `None` = present from the start.
    pub replaces: Option<u8>,
    /// The next part in the same replacement chain (+0x65).
    pub next: Option<u8>,
    /// The first part that replaces this one when it smashes (+0x66).
    pub first_replacement: Option<u8>,
    /// Part indices this part is linked to (+0x68 / +0x6C).
    pub links: Vec<u8>,
    pub behaviours: Vec<Behaviour>,
}

/// Size of one `CompoundObject` part record.
const COMPOUND_PART_SIZE: usize = 0x70;

fn behaviours(res: &Resource, base: usize) -> Result<Vec<Behaviour>> {
    let r = Reader(res.header());
    let (count, ptr) = (r.u8(base + 1)? as usize, r.u32(base + 0x0C)? as usize);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let o = ptr + i * 0x14;
        out.push(Behaviour { object: res.import_at(o as u32), type_id: r.u32(o + 0x0C)?, hash: r.u32(o + 0x10)? });
    }
    Ok(out)
}

fn part_index(b: u8) -> Option<u8> {
    (b != 0xff).then_some(b)
}

impl ObjectDef {
    fn parse_slots(res: &Resource, slots: &[u32], physics: u32) -> Result<Self> {
        // The header must at least reach the first slot.
        Reader(res.header()).u32(slots[0] as usize)?;
        Ok(Self {
            id: res.id,
            models: slots.iter().map(|&o| res.import_at(o)).collect(),
            physics: res.import_at(physics),
            parts: Vec::new(),
            compound_parts: Vec::new(),
            skeleton: None,
            joints: Vec::new(),
            behaviours: Vec::new(),
            reset_distance_sq: None,
            remove_instead_of_reset: false,
            impact_effect: None,
            impact_sound: None,
            locator_groups: Vec::new(),
            part_locator_groups: Vec::new(),
        })
    }

    pub fn parse_prop(res: &Resource) -> Result<Self> {
        let mut def = Self::parse_slots(res, &[0x04, 0x08], 0x0C)?;
        let r = Reader(res.header());
        def.reset_distance_sq = r.f32(0x24).ok();
        def.remove_instead_of_reset = r.u8(0x28).is_ok_and(|b| b != 0);
        def.impact_effect = res.import_at(0x14);
        def.impact_sound = res.import_at(0x1c);
        Ok(def)
    }
    pub fn parse_compound(res: &Resource) -> Result<Self> {
        let mut def = Self::parse_slots(res, &[0x04, 0x24, 0x28], 0x1C)?;
        let r = Reader(res.header());
        let (base, count) = (r.u32(0x30)? as usize, r.u32(0x38)? as usize);
        if count > 256 {
            return Err(crate::Error::Malformed(format!("compound part count {count}")));
        }
        def.skeleton = res.import_at(0x08);
        def.behaviours = behaviours(res, 0)?;
        def.locator_groups = locator_groups(res, 0);
        let hashes = r.u32(0x34)? as usize;
        for i in 0..count {
            let o = base + i * COMPOUND_PART_SIZE;
            r.u32(o + COMPOUND_PART_SIZE - 4)?;
            def.parts.push(res.import_at((o + 0x1C) as u32));
            def.part_locator_groups.push(locator_groups(res, o));
            let (links_ptr, links) = (r.u32(o + 0x68)? as usize, r.u8(o + 0x6c)? as usize);
            def.compound_parts.push(CompoundPart {
                hash: if hashes == 0 { 0 } else { r.u32(hashes + 4 * i)? },
                bone: r.u32(o + 0x60)?,
                replaces: part_index(r.u8(o + 0x64)?),
                next: part_index(r.u8(o + 0x65)?),
                first_replacement: part_index(r.u8(o + 0x66)?),
                links: if links == 0 { Vec::new() } else { r.bytes(links_ptr, links)?.to_vec() },
                behaviours: behaviours(res, o)?,
            });
        }
        let (joints_ptr, joints) = (r.u32(0x2c)? as usize, r.u32(0x3c)? as usize);
        if joints > 256 {
            return Err(crate::Error::Malformed(format!("compound joint count {joints}")));
        }
        def.joints =
            (0..joints).map(|i| u32::try_from(joints_ptr + 8 * i).ok().and_then(|o| res.import_at(o))).collect();
        Ok(def)
    }
    pub fn parse_world_object(res: &Resource) -> Result<Self> {
        let mut def = Self::parse_slots(res, &[0x04], 0x1C)?;
        def.behaviours = behaviours(res, 0).unwrap_or_default();
        def.locator_groups = locator_groups(res, 0);
        Ok(def)
    }

    /// The model to draw: the first populated slot.
    pub fn model(&self) -> Option<ResourceId> {
        self.models.iter().flatten().next().copied()
    }
}
