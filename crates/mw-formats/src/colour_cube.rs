//! `ColourCube` (0x52): the colour grade a keyframe applies after the tonemap.
//!
//! | offset | type        | meaning |
//! |--------|-------------|---------|
//! | 0x00   | u32         | edge length N (16 on every instance) |
//! | 0x04   | u32         | N again |
//! | 0x08   | u32 × 2     | 0 |
//! | 0x10   | u8 × 3 × N³ | RGB8 entries, blue fastest: entry (r, g, b) at 3 × (b + N g + N² r) |
//!
//! Input and output are gamma-encoded colour; cube `…3e93` is the exact identity. The game looks it up
//! as `cube(saturate(rgb × ColourTintAndBloomHeadroom) × scale + offset)` (`UIRearViewMirrorShader`).

use mw_bundle::{Reader, Resource};

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct ColourCube {
    pub size: usize,
    /// RGB8 entries, `size³ × 3` bytes, blue fastest.
    pub rgb: Vec<u8>,
}

impl ColourCube {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let size = r.u32(0)? as usize;
        if size == 0 || size > 64 || r.u32(4)? as usize != size {
            return Err(Error::Malformed(format!("colour cube size {size}")));
        }
        let rgb = r.bytes(0x10, size * size * size * 3)?.to_vec();
        Ok(Self { size, rgb })
    }

    /// The entry at integer coordinates.
    pub fn at(&self, r: usize, g: usize, b: usize) -> [u8; 3] {
        let i = 3 * (b + self.size * (g + self.size * r));
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }
}
