//! CPU decoding of texture mips to RGBA8 (previews, exports, fallbacks).

use crate::{DxgiFormat, Texture, f16_to_f32};

impl Texture {
    /// Decode one mip of the first array slice to tightly packed RGBA8.
    pub fn decode_rgba8(&self, level: u32) -> Option<(u32, u32, Vec<u8>)> {
        let w = (self.width >> level).max(1);
        let h = (self.height >> level).max(1);
        let start: usize = (0..level).map(|m| self.mip_size(m)).sum();
        let src = self.data.get(start..start + self.mip_size(level))?;
        let mut out = vec![0u8; (w * h * 4) as usize];
        match self.format {
            DxgiFormat::Bc1Unorm | DxgiFormat::Bc2Unorm | DxgiFormat::Bc3Unorm => {
                let bb = self.format.block_bytes()?;
                let bw = w.div_ceil(4) as usize;
                for (bi, block) in src.chunks_exact(bb).enumerate() {
                    let (bx, by) = ((bi % bw) * 4, (bi / bw) * 4);
                    let px = match self.format {
                        DxgiFormat::Bc1Unorm => decode_bc1(block, true),
                        DxgiFormat::Bc2Unorm => decode_bc2(block),
                        _ => decode_bc3(block),
                    };
                    for (i, p) in px.iter().enumerate() {
                        let (x, y) = (bx + i % 4, by + i / 4);
                        if x < w as usize && y < h as usize {
                            let o = (y * w as usize + x) * 4;
                            out[o..o + 4].copy_from_slice(p);
                        }
                    }
                }
            }
            DxgiFormat::R8G8B8A8Unorm | DxgiFormat::R8G8B8A8UnormSrgb => {
                let n = out.len();
                out.copy_from_slice(src.get(..n)?)
            }
            DxgiFormat::B8G8R8A8Unorm => {
                for (o, s) in out.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<4>().0) {
                    *o = [s[2], s[1], s[0], s[3]];
                }
            }
            DxgiFormat::R8Unorm => {
                for (o, &s) in out.as_chunks_mut::<4>().0.iter_mut().zip(src) {
                    *o = [s, s, s, 255];
                }
            }
            DxgiFormat::R16G16B16A16Float => {
                for (o, s) in out.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<8>().0) {
                    for c in 0..4 {
                        let v = f16_to_f32(u16::from_le_bytes([s[c * 2], s[c * 2 + 1]]));
                        o[c] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    }
                }
            }
            _ => return None,
        }
        Some((w, h, out))
    }
}

fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 31) as u32;
    let g = ((c >> 5) & 63) as u32;
    let b = (c & 31) as u32;
    [((r * 527 + 23) >> 6) as u8, ((g * 259 + 33) >> 6) as u8, ((b * 527 + 23) >> 6) as u8]
}

fn decode_bc1(b: &[u8], allow_alpha: bool) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let (p0, p1) = (rgb565(c0), rgb565(c1));
    let mix = |a: u8, b: u8, wa: u32, wb: u32| ((a as u32 * wa + b as u32 * wb) / (wa + wb)) as u8;
    let mut pal = [[0u8; 4]; 4];
    pal[0] = [p0[0], p0[1], p0[2], 255];
    pal[1] = [p1[0], p1[1], p1[2], 255];
    if c0 > c1 || !allow_alpha {
        pal[2] = [mix(p0[0], p1[0], 2, 1), mix(p0[1], p1[1], 2, 1), mix(p0[2], p1[2], 2, 1), 255];
        pal[3] = [mix(p0[0], p1[0], 1, 2), mix(p0[1], p1[1], 1, 2), mix(p0[2], p1[2], 1, 2), 255];
    } else {
        pal[2] = [mix(p0[0], p1[0], 1, 1), mix(p0[1], p1[1], 1, 1), mix(p0[2], p1[2], 1, 1), 255];
        pal[3] = [0, 0, 0, 0];
    }
    let bits = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    std::array::from_fn(|i| pal[((bits >> (i * 2)) & 3) as usize])
}

fn decode_bc2(b: &[u8]) -> [[u8; 4]; 16] {
    let mut px = decode_bc1(&b[8..16], false);
    let alpha = u64::from_le_bytes(b[0..8].try_into().unwrap());
    for (i, p) in px.iter_mut().enumerate() {
        p[3] = (((alpha >> (i * 4)) & 0xF) * 17) as u8;
    }
    px
}

fn decode_bc3(b: &[u8]) -> [[u8; 4]; 16] {
    let mut px = decode_bc1(&b[8..16], false);
    let (a0, a1) = (b[0] as u32, b[1] as u32);
    let mut pal = [0u8; 8];
    pal[0] = a0 as u8;
    pal[1] = a1 as u8;
    if a0 > a1 {
        for i in 1..7 {
            pal[i + 1] = (((7 - i as u32) * a0 + i as u32 * a1) / 7) as u8;
        }
    } else {
        for i in 1..5 {
            pal[i + 1] = (((5 - i as u32) * a0 + i as u32 * a1) / 5) as u8;
        }
        pal[6] = 0;
        pal[7] = 255;
    }
    let mut bits = 0u64;
    for (i, &v) in b[2..8].iter().enumerate() {
        bits |= (v as u64) << (8 * i);
    }
    for (i, p) in px.iter_mut().enumerate() {
        p[3] = pal[((bits >> (i * 3)) & 7) as usize];
    }
    px
}
