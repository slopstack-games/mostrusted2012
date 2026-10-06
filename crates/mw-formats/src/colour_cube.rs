//! `ColourCube` (0x52): the colour grade applied after the tonemap.
//!
//! | offset | type        | meaning |
//! |--------|-------------|---------|
//! | 0x00   | u32         | edge length N (16 on every instance) |
//! | 0x04   | u32         | N again |
//! | 0x08   | u32 × 2     | 0 |
//! | 0x10   | u8 × 3 × N³ | a D3D volume texture without its alpha byte: input red fastest, then green, then blue; each entry stored B, G, R. The output for input (r, g, b) is at 3 × (r + N g + N² b) |
//!
//! Input and output are gamma-encoded colour; cube `…3e93` is the exact identity (it reads the same in
//! either axis order, so it does not settle the order). The game blends up to seven cubes with weights on
//! the CPU into a 16³ four-byte volume ("TintBlend", `FUN_008e0930` starts the job, `FUN_008e0ac0` copies
//! the result row by row into the texture, no reordering) and the final post pass built into `NFS13.exe`
//! samples it at `rgb × ColourCubeScalesOffsets.xyz + .w` with x = red. The entries are therefore the
//! texture's own byte order (x fastest, B8G8R8 + alpha). The earlier reading of the same bytes (blue
//! fastest, stored R, G, B) swapped red and blue in every output, and turned the noon cube's cooling of
//! the mid-tones into a yellow cast (research log 2026-10-06).

use mw_bundle::{Reader, Resource};

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct ColourCube {
    pub size: usize,
    /// The stored entries, `size³ × 3` bytes: red fastest, each entry B, G, R (see the module docs).
    pub bgr: Vec<u8>,
}

impl ColourCube {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let size = r.u32(0)? as usize;
        if size == 0 || size > 64 || r.u32(4)? as usize != size {
            return Err(Error::Malformed(format!("colour cube size {size}")));
        }
        let bgr = r.bytes(0x10, size * size * size * 3)?.to_vec();
        Ok(Self { size, bgr })
    }

    /// The graded colour (R, G, B) for the input at integer coordinates (r, g, b).
    pub fn at(&self, r: usize, g: usize, b: usize) -> [u8; 3] {
        let i = 3 * (r + self.size * (g + self.size * b));
        [self.bgr[i + 2], self.bgr[i + 1], self.bgr[i]]
    }
}
