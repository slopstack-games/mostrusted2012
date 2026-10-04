//! `WorldPaintMap` (0x91): a value painted over the world map, stored as one quadtree per 1 km cell.
//!
//! Three instances: `HAWAII/ENVIRONMENT.BNDL` `…2823d` paints the 25 `EnvironmentTimeline`s (which
//! timeline lights each place), `…285cb` the 19 `LayerFog` objects, and `GLOBALEFFECTS.BNDL` `…285cc`
//! one `ParticleLighting` object. `EnvironmentSettings` names the first two by the low bits of their IDs
//! (fields `409f546f` / `ae067627`), `VfxGlobals` the third (handle `5e608031`).
//!
//! Header (0x30 bytes):
//!
//! | off  | type   | field |
//! |------|--------|-------|
//! | 0x00 | u32    | version (1) |
//! | 0x04 | u32    | entry count (index 0 = nothing painted) |
//! | 0x08 | u32    | offset of the entry list (u32 each: low bits of a resource ID, 0 for index 0) |
//! | 0x0C | u32    | grid width (cells along x) |
//! | 0x10 | u32    | grid height (cells along z) |
//! | 0x14 | i32    | x of the grid origin, in cells (signed: −7 on the effects map) |
//! | 0x18 | i32    | z of the grid origin, in cells |
//! | 0x1C | f32    | cell size (1000 m) |
//! | 0x20 | f32    | 1 / cell size |
//! | 0x24 | u32    | unknown (4878 / 1926 / 94) |
//! | 0x28 | u32    | offset of the cell table |
//!
//! Cell table: `width × height` entries of `u32 offset, u32 size` (row-major, x fastest); size 0 = an
//! unpainted cell. Each cell's block is a quadtree of 8-byte nodes, root first: four child bytes, then four
//! offset bytes. A child byte below 0x80 is a leaf (an entry index); otherwise the child is another node at
//! block offset `(byte & 0x7F) << 8 | offset byte`. Children are ordered (x0 z0, x1 z0, x0 z1, x1 z1).
//!
//! World position → cell: `cx = floor(x / cell) + origin_x`, `cz = floor(z / cell) + origin_z`
//! (checked against the track units' zone polygons, see `docs/formats.md`).

use mw_bundle::{Reader, Resource};

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct WorldPaintMap {
    /// Low bits of the painted resources' IDs (`0x0100_0000_0000_0000 | id`); index 0 is "none".
    pub entries: Vec<u32>,
    pub width: u32,
    pub height: u32,
    pub origin: [i32; 2],
    pub cell_size: f32,
    /// Per cell: the quadtree block (empty = unpainted).
    cells: Vec<Vec<u8>>,
}

impl WorldPaintMap {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let version = r.u32(0)?;
        if version != 1 {
            return Err(Error::Unsupported(format!("WorldPaintMap version {version}")));
        }
        let count = r.u32(4)? as usize;
        let list = r.u32(8)? as usize;
        let (width, height) = (r.u32(0x0C)?, r.u32(0x10)?);
        let origin = [r.i32(0x14)?, r.i32(0x18)?];
        let cell_size = r.f32(0x1C)?;
        let table = r.u32(0x28)? as usize;
        if count > 256 || width > 1024 || height > 1024 || cell_size.is_nan() || cell_size <= 0.0 {
            return Err(Error::Malformed(format!("paint map {count} entries, {width}×{height}, cell {cell_size}")));
        }
        let entries = (0..count).map(|i| r.u32(list + 4 * i)).collect::<std::result::Result<_, _>>()?;
        let mut cells = Vec::with_capacity((width * height) as usize);
        for i in 0..(width * height) as usize {
            let (off, size) = (r.u32(table + 8 * i)? as usize, r.u32(table + 8 * i + 4)? as usize);
            cells.push(if size == 0 { Vec::new() } else { r.bytes(off, size)?.to_vec() });
        }
        Ok(Self { entries, width, height, origin, cell_size, cells })
    }

    /// The entry index painted at a world position (`None` outside the grid or in an unpainted cell).
    pub fn index_at(&self, x: f32, z: f32) -> Option<u8> {
        let fx = x / self.cell_size + self.origin[0] as f32;
        let fz = z / self.cell_size + self.origin[1] as f32;
        if !(fx >= 0.0 && fz >= 0.0 && fx < self.width as f32 && fz < self.height as f32) {
            return None;
        }
        let (cx, cz) = (fx as usize, fz as usize);
        let block = &self.cells[cz * self.width as usize + cx];
        if block.is_empty() {
            return None;
        }
        // Descend: u, v are the position inside the current node, 0..1.
        let (mut u, mut v) = (fx - cx as f32, fz - cz as f32);
        let mut node = 0usize;
        // A malformed tree could loop; real ones are at most ~12 levels deep.
        for _ in 0..24 {
            let q = (u >= 0.5) as usize | ((v >= 0.5) as usize) << 1;
            u = (u * 2.0).fract();
            v = (v * 2.0).fract();
            let child = *block.get(node + q)?;
            if child < 0x80 {
                return Some(child);
            }
            node = ((child as usize & 0x7F) << 8) | *block.get(node + 4 + q)? as usize;
        }
        None
    }

    /// The low bits of the resource painted at a world position (`None` where nothing is painted).
    pub fn entry_at(&self, x: f32, z: f32) -> Option<u32> {
        self.index_at(x, z).and_then(|i| self.entries.get(i as usize).copied()).filter(|&id| id != 0)
    }
}
