//! Groundcover (grass, bushes, small plants): `GroundcoverCollection` (0x20F) and
//! `GroundcoverInstances` (0x214).
//!
//! A collection covers a region (a few hundred metres) and has three layers (sprite classes), each
//! with its own material and a bounding-volume tree over the instances. The instances live in
//! separate `GroundcoverInstances` blobs. The game's `Groundcover` shader draws every instance as a
//! camera-facing sprite with wind sway; there is no mesh.
//!
//! Collection header (chunk 0):
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | u32  | version (4) |
//! | 0x04 + 0x0C·L | ptr, u32, u32 | layer L (0..3): node array, node count, total instance count |
//! | 0x28 | u32  | 3 |
//! | 0x2C | u32  | probably a GPU buffer size (not used) |
//! | 0x38 | u32  | blob count |
//! | 0x3C | ptr  | array of blob imports (`GroundcoverInstances`) |
//! | 0x30 | ptr  | material table (0x40) |
//! | 0x34 | u32  | material count (1..5) |
//! | 0x40 + 0x110·M | | material M: material import (handle) at +0x00, then 16 texture regions at +0x10 (vec4: offset.xy, scale.xy) |
//!
//! The material is a `Groundcover` shader material
//! (one `DiffuseSampler` texture with an alpha mask; wind parameters `amount` and `speed`).
//!
//! Nodes (0x60 bytes) form a tree in depth-first order. Inner nodes have `u32 4 == 2` at +0x10 and a
//! bounding box at +0x40. Leaf nodes have: `+0x10: 0`, `+0x14: ptr` to group records, `+0x18: group
//! count`, `+0x1C: instance count`. Group record (0x40 bytes, at the leaf's pointer):
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | f32×8 | box: min xyz, -r, max xyz, +r (r = largest sprite size) |
//! | 0x20 | u32×4 | 1, material index, 0, 0 |
//! | 0x30 | u32  | triangles per sprite: 2, 4, 6 or 8 (1 to 4 crossed quads; the shader's vertex count) |
//! | 0x34 | u32  | instance count |
//! | 0x38 | u32  | `first instance index << 16 | blob index` (index within that blob) |
//! | 0x3C | f32  | r |
//!
//! Blobs hold at most about 8190 instances each; big collections have many (up to ~9), and layers
//! continue where the previous one stopped, so a layer's groups can span several blobs.
//!
//! Instance (16 bytes, `GroundcoverInstances` chunk 0 = `u32 0x10, u32 count, 0, 0` then records):
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | u16×3 | x, y, z as fractions of the group box (÷65535) |
//! | 0x06 | u8   | yaw (÷256 of a turn; equals the size byte in some groups, where it is probably unused) |
//! | 0x07 | u8   | sprite size in 1/32 m (the box margin `r` is the largest size) |
//! | 0x08 | i8×3 | ground normal (÷127) |
//! | 0x0B | u8   | unused (0) |
//! | 0x0C | u8×3 | tint (the shader outputs sqrt(2·tint)) |
//! | 0x0F | u8   | texture region index |

use mw_bundle::{Reader, Resource, ResourceId};

use crate::{Error, Result};

const LAYERS: usize = 3;
const REGIONS: usize = 16;
const NODE_SIZE: usize = 0x60;
const GROUP_SIZE: usize = 0x40;
const INSTANCE_SIZE: usize = 16;
const MAX_NODES: usize = 1 << 16;
const MAX_MATERIALS: usize = 16;
const MAX_BLOBS: usize = 256;

/// A box of instances sharing one quantisation range.
#[derive(Debug, Clone)]
pub struct Group {
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// Index into [`GroundcoverCollection::materials`].
    pub material: usize,
    /// Crossed quads per sprite (1..=4).
    pub quads: u32,
    /// Index into [`GroundcoverCollection::blobs`].
    pub blob: u32,
    pub start: u32,
    pub count: u32,
}

#[derive(Debug, Clone, Default)]
pub struct GroundcoverLayer {
    pub groups: Vec<Group>,
    pub total: u32,
}

#[derive(Debug, Clone, Default)]
pub struct GroundcoverMaterial {
    pub material: Option<ResourceId>,
    /// Texture regions as `[offset.x, offset.y, scale.x, scale.y]`.
    pub regions: Vec<[f32; 4]>,
}

#[derive(Debug, Clone)]
pub struct GroundcoverCollection {
    pub id: ResourceId,
    pub materials: Vec<GroundcoverMaterial>,
    pub layers: Vec<GroundcoverLayer>,
    pub blobs: Vec<ResourceId>,
}

/// One decoded instance.
#[derive(Debug, Clone, Copy)]
pub struct GroundcoverInstance {
    pub position: [f32; 3],
    /// Index into the collection's materials.
    pub material: usize,
    /// Crossed quads to draw.
    pub quads: u32,
    /// Sprite size in metres (the height; width follows the texture region's aspect).
    pub size: f32,
    /// Yaw in radians.
    pub yaw: f32,
    pub normal: [f32; 3],
    pub tint: [u8; 3],
    pub region: u8,
}

impl GroundcoverCollection {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let nmat = r.u32(0x34)? as usize;
        if nmat > MAX_MATERIALS {
            return Err(Error::Malformed(format!("groundcover material count {nmat}")));
        }
        let mut materials = Vec::with_capacity(nmat);
        for m in 0..nmat {
            let base = 0x40 + 0x110 * m;
            let regions =
                (0..REGIONS).map(|i| r.f32_array(base + 0x10 + 0x10 * i)).collect::<mw_bundle::Result<_>>()?;
            materials.push(GroundcoverMaterial { material: res.import_at(base as u32), regions });
        }
        let mut layers = Vec::with_capacity(LAYERS);
        for l in 0..LAYERS {
            let (ptr, count, total) =
                (r.u32(0x04 + 0x0C * l)?, r.u32(0x08 + 0x0C * l)? as usize, r.u32(0x0C + 0x0C * l)?);
            let mut layer = GroundcoverLayer { total, ..Default::default() };
            if count > MAX_NODES {
                return Err(Error::Malformed(format!("groundcover node count {count}")));
            }
            if count > 0 {
                for n in 0..count {
                    let o = ptr as usize + n * NODE_SIZE;
                    // Leaf: no children flag at +0x10.
                    if r.u32(o + 0x10)? != 0 {
                        continue;
                    }
                    let (gptr, gcount) = (r.u32(o + 0x14)? as usize, r.u32(o + 0x18)? as usize);
                    if gcount > MAX_NODES {
                        return Err(Error::Malformed(format!("groundcover group count {gcount}")));
                    }
                    for g in 0..gcount {
                        let go = gptr + g * GROUP_SIZE;
                        let b: [f32; 8] = r.f32_array(go)?;
                        layer.groups.push(Group {
                            min: [b[0], b[1], b[2]],
                            max: [b[4], b[5], b[6]],
                            material: r.u32(go + 0x24)? as usize,
                            quads: (r.u32(go + 0x30)? / 2).clamp(1, 4),
                            count: r.u32(go + 0x34)?,
                            blob: r.u32(go + 0x38)? & 0xFFFF,
                            start: r.u32(go + 0x38)? >> 16,
                        });
                    }
                }
            }
            layers.push(layer);
        }
        let nblobs = r.u32(0x38)? as usize;
        let blobs = if nblobs == 0 || nblobs > MAX_BLOBS {
            Vec::new()
        } else {
            let ptr = r.u32(0x3C)?;
            (0..nblobs as u32).filter_map(|i| res.import_at(ptr + 4 * i)).collect()
        };
        Ok(Self { id: res.id, materials, layers, blobs })
    }

    /// Decode a layer's instances. `blobs` are the collection's blobs in order, parsed.
    pub fn instances(&self, layer: usize, blobs: &[GroundcoverInstances]) -> Result<Vec<GroundcoverInstance>> {
        let mut out = Vec::new();
        let Some(layer) = self.layers.get(layer) else { return Ok(out) };
        for g in &layer.groups {
            let end = g.start as usize + g.count as usize;
            let blob =
                blobs.get(g.blob as usize).ok_or_else(|| Error::Malformed(format!("groundcover blob {}", g.blob)))?;
            let records = blob.records.get(g.start as usize..end).ok_or_else(|| {
                Error::Malformed(format!("groundcover group {}..{end} past {} instances", g.start, blob.records.len()))
            })?;
            out.extend(records.iter().map(|rec| decode(rec, g)));
        }
        Ok(out)
    }
}

fn decode(rec: &[u8; INSTANCE_SIZE], g: &Group) -> GroundcoverInstance {
    let q = |i: usize| u16::from_le_bytes([rec[i], rec[i + 1]]) as f32 / 65535.0;
    let position = std::array::from_fn(|k| g.min[k] + q(2 * k) * (g.max[k] - g.min[k]));
    let n = |i: usize| rec[i] as i8 as f32 / 127.0;
    GroundcoverInstance {
        position,
        material: g.material,
        quads: g.quads,
        size: rec[7] as f32 / 32.0,
        yaw: rec[6] as f32 / 256.0 * std::f32::consts::TAU,
        normal: [n(8), n(9), n(10)],
        tint: [rec[12], rec[13], rec[14]],
        region: rec[15],
    }
}

/// Raw instance records of a `GroundcoverInstances` resource.
#[derive(Debug, Clone)]
pub struct GroundcoverInstances {
    pub records: Vec<[u8; INSTANCE_SIZE]>,
}

impl GroundcoverInstances {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let base = r.u32(0x00)? as usize;
        let count = r.u32(0x04)? as usize;
        if count > 1 << 22 {
            return Err(Error::Malformed(format!("groundcover instance count {count}")));
        }
        let mut records = Vec::with_capacity(count);
        for i in 0..count {
            let b = r.bytes(base + i * INSTANCE_SIZE, INSTANCE_SIZE)?;
            records.push(b.try_into().map_err(|_| Error::Malformed("short groundcover record".into()))?);
        }
        Ok(Self { records })
    }
}
