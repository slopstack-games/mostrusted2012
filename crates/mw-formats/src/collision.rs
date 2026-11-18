//! `PolygonSoupList` (0x60): static collision geometry.
//!
//! | off  | type  | field |
//! |------|-------|-------|
//! | 0x00 | f32×4 | AABB min |
//! | 0x10 | f32×4 | AABB max |
//! | 0x20 | ptr   | soup pointers (ptr[n]) |
//! | 0x24 | ptr   | soup AABBs, SoA in groups of 4 (0x70 bytes per group) |
//! | 0x28 | i32   | soup count |
//! | 0x2C | i32   | data size |
//!
//! Soup (0x10-byte header):
//!
//! | off | type  | field |
//! |-----|-------|-------|
//! | 0x0 | ptr   | polygons (0xC bytes each, quads first) |
//! | 0x4 | ptr   | vertices (u16×3 each) |
//! | 0x8 | u16   | data size |
//! | 0xA | i8×3  | vertex offset multipliers (× 500 m) |
//! | 0xD | u8    | quad count |
//! | 0xE | u8    | polygon count |
//! | 0xF | u8    | vertex count |
//!
//! A vertex is `u16 * 500/32768 + multiplier * 500` (metres, world space).
//! Polygon: `u16 collision tag[2]`, `u8 vertex indices[4]` (4th = 0xFF for
//! triangles), `u8 edge cosines[4]`. Quads are in triangle-strip order: the
//! fourth vertex is opposite the first, so they split into (a, b, c) and
//! (b, d, c). (A fan split makes every quad a bow-tie with a hole in it;
//! checked on every quad in the install.)
//!
//! The collision tag, read as one little-endian `u32` (`tag[0] | tag[1] << 16`, burnout.wiki's
//! `muCollisionTag`), packs flags (bits 0-9), 3 unused bits, an index into the global surface list
//! (bits 13-19; `Genesys.Gen.Surfaces` in `HAWAII/SURFACELIST.BNDL`, 65 entries) and the track unit
//! (bits 20-31). The wiki lists the same fields from bit 0 = most significant (PS3 bitfield
//! order). See [`CollisionTag`].

use mw_bundle::{Reader, Resource};

use crate::Result;

/// Metres per quantised vertex unit.
pub const SOUP_GRANULARITY: f32 = 500.0 / 32768.0;
/// Metres per vertex-offset multiplier step.
pub const SOUP_OFFSET_UNIT: f32 = 500.0;

#[derive(Debug, Clone)]
pub struct PolygonSoupList {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub soups: Vec<PolygonSoup>,
}

#[derive(Debug, Clone)]
pub struct PolygonSoup {
    pub vertices: Vec<[f32; 3]>,
    pub polygons: Vec<SoupPolygon>,
}

#[derive(Debug, Clone, Copy)]
pub struct SoupPolygon {
    /// Indices into the soup's vertices; triangles have 3.
    pub indices: [u8; 4],
    pub is_quad: bool,
    /// Collision tag halves; [`SoupPolygon::tag`] decodes them.
    pub collision_tag: [u16; 2],
}

/// A decoded polygon collision tag (fields and flag names from burnout.wiki, "Polygon Soup
/// List/Need for Speed"; bit positions checked on the PC data: the unit field holds the track unit's
/// own number on every triangle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CollisionTag {
    /// Track unit the polygon belongs to (bits 20-31).
    pub unit: u16,
    /// Index into the `Surfaces` list (bits 13-19).
    pub surface: u8,
    /// `CollisionTag::DRIVABLE` etc. (bits 0-9).
    pub flags: u16,
}

impl CollisionTag {
    pub const DRIVABLE: u16 = 0x1;
    pub const ROAD: u16 = 0x2;
    pub const KERB: u16 = 0x4;
    pub const DECAL: u16 = 0x8;
    pub const SOLID_DECAL: u16 = 0x10;
    pub const REMOVABLE: u16 = 0x20;
    pub const FATAL: u16 = 0x40;
    pub const SUPER_FATAL: u16 = 0x80;
    pub const FATAL_FIELD: u16 = 0x100;
    pub const LOCAL_PLAYER_ONLY: u16 = 0x200;

    pub fn from_u32(v: u32) -> Self {
        Self { unit: (v >> 20) as u16, surface: ((v >> 13) & 0x7F) as u8, flags: (v & 0x3FF) as u16 }
    }

    pub fn to_u32(self) -> u32 {
        (self.unit as u32 & 0xFFF) << 20 | (self.surface as u32 & 0x7F) << 13 | self.flags as u32 & 0x3FF
    }

    pub fn has(self, flag: u16) -> bool {
        self.flags & flag != 0
    }
}

impl PolygonSoupList {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let [x0, y0, z0, _] = r.f32_array::<4>(0x00)?;
        let [x1, y1, z1, _] = r.f32_array::<4>(0x10)?;
        let table = r.u32(0x20)? as usize;
        let count = r.u32(0x28)? as usize;
        let mut soups = Vec::with_capacity(count.min(4096));
        for i in 0..count {
            let s = r.u32(table + i * 4)? as usize;
            let (polys, verts) = (r.u32(s)? as usize, r.u32(s + 4)? as usize);
            let mult = [r.u8(s + 0xA)? as i8, r.u8(s + 0xB)? as i8, r.u8(s + 0xC)? as i8];
            let (n_quads, n_polys, n_verts) =
                (r.u8(s + 0xD)? as usize, r.u8(s + 0xE)? as usize, r.u8(s + 0xF)? as usize);
            let vertices = (0..n_verts)
                .map(|v| {
                    let o = verts + v * 6;
                    Ok([
                        r.u16(o)? as f32 * SOUP_GRANULARITY + mult[0] as f32 * SOUP_OFFSET_UNIT,
                        r.u16(o + 2)? as f32 * SOUP_GRANULARITY + mult[1] as f32 * SOUP_OFFSET_UNIT,
                        r.u16(o + 4)? as f32 * SOUP_GRANULARITY + mult[2] as f32 * SOUP_OFFSET_UNIT,
                    ])
                })
                .collect::<Result<_>>()?;
            let polygons = (0..n_polys)
                .map(|p| {
                    let o = polys + p * 0xC;
                    let idx = r.bytes(o + 4, 4)?;
                    Ok(SoupPolygon {
                        indices: [idx[0], idx[1], idx[2], idx[3]],
                        is_quad: p < n_quads,
                        collision_tag: [r.u16(o)?, r.u16(o + 2)?],
                    })
                })
                .collect::<Result<_>>()?;
            soups.push(PolygonSoup { vertices, polygons });
        }
        Ok(Self { min: [x0, y0, z0], max: [x1, y1, z1], soups })
    }

    /// Flatten into one indexed triangle mesh (quads split in two).
    pub fn triangles(&self) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        let mut verts = Vec::new();
        let mut tris = Vec::new();
        for soup in &self.soups {
            let base = verts.len() as u32;
            verts.extend_from_slice(&soup.vertices);
            let n = soup.vertices.len() as u8;
            for p in &soup.polygons {
                for t in p.triangles().filter(|t| t.iter().all(|&i| i < n)) {
                    tris.push(t.map(|i| base + i as u32));
                }
            }
        }
        (verts, tris)
    }

    /// The raw collision tag of every triangle [`PolygonSoupList::triangles`] returns, in the same
    /// order (decode with [`CollisionTag::from_u32`]).
    pub fn triangle_tags(&self) -> Vec<u32> {
        let mut tags = Vec::new();
        for soup in &self.soups {
            let n = soup.vertices.len() as u8;
            for p in &soup.polygons {
                let tag = p.tag().to_u32();
                tags.extend(p.triangles().filter(|t| t.iter().all(|&i| i < n)).map(|_| tag));
            }
        }
        tags
    }
}

impl SoupPolygon {
    pub fn tag(&self) -> CollisionTag {
        CollisionTag::from_u32(self.collision_tag[0] as u32 | (self.collision_tag[1] as u32) << 16)
    }

    /// The polygon's triangles as vertex indices (quads split strip-wise).
    pub fn triangles(&self) -> impl Iterator<Item = [u8; 3]> {
        let [a, b, c, d] = self.indices;
        std::iter::once([a, b, c]).chain(self.is_quad.then_some([b, d, c]))
    }
}
