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
    /// Surface / material tags (meaning not decoded yet).
    pub collision_tag: [u16; 2],
}

impl PolygonSoupList {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let [x0, y0, z0, _] = r.f32_array::<4>(0x00)?;
        let [x1, y1, z1, _] = r.f32_array::<4>(0x10)?;
        let table = r.u32(0x20)? as usize;
        let count = r.u32(0x28)? as usize;
        let mut soups = Vec::with_capacity(count);
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
}

impl SoupPolygon {
    /// The polygon's triangles as vertex indices (quads split strip-wise).
    pub fn triangles(&self) -> impl Iterator<Item = [u8; 3]> {
        let [a, b, c, d] = self.indices;
        std::iter::once([a, b, c]).chain(self.is_quad.then_some([b, d, c]))
    }
}
