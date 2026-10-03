//! `Texture` (0x1): a D3D11 texture header in chunk 0 and the raw mip chain in
//! chunk 1.
//!
//! PC header (0x30 bytes):
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | u32  | runtime pointer (0) |
//! | 0x04 | u32  | usage (always 1) |
//! | 0x08 | u32  | dimension: 6 = 1D, 7 = 2D, 8 = 3D, 9 = cube |
//! | 0x0C | u32×4| runtime pointers (0) |
//! | 0x1C | u32  | `DXGI_FORMAT` |
//! | 0x20 | u32  | flags (0x10 / 0x20 / 0x30 observed) |
//! | 0x24 | u16  | width |
//! | 0x26 | u16  | height |
//! | 0x28 | u16  | depth |
//! | 0x2A | u16  | array size (6 for cubes) |
//! | 0x2C | u8   | most detailed mip used at runtime (streaming bias) |
//! | 0x2D | u8   | mip count |

use mw_bundle::{Reader, Resource};

use crate::{DxgiFormat, Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    D1,
    D2,
    D3,
    Cube,
}

#[derive(Debug, Clone)]
pub struct Texture {
    pub format: DxgiFormat,
    pub dimension: Dimension,
    pub flags: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub array_size: u32,
    pub most_detailed_mip: u8,
    pub mip_count: u32,
    /// All mips of all array slices, slice-major (D3D subresource order).
    pub data: Vec<u8>,
}

impl Texture {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let dimension = match r.u32(0x08)? {
            6 => Dimension::D1,
            7 => Dimension::D2,
            8 => Dimension::D3,
            9 => Dimension::Cube,
            d => return Err(Error::Unsupported(format!("texture dimension {d}"))),
        };
        Ok(Self {
            format: DxgiFormat::from_raw(r.u32(0x1C)?),
            dimension,
            flags: r.u32(0x20)?,
            width: r.u16(0x24)? as u32,
            height: r.u16(0x26)? as u32,
            depth: (r.u16(0x28)? as u32).max(1),
            array_size: (r.u16(0x2A)? as u32).max(1),
            most_detailed_mip: r.u8(0x2C)?,
            mip_count: (r.u8(0x2D)? as u32).max(1),
            data: res.body().to_vec(),
        })
    }

    /// Byte size of one mip level of one slice.
    pub fn mip_size(&self, level: u32) -> usize {
        let w = (self.width >> level).max(1) as usize;
        let h = (self.height >> level).max(1) as usize;
        let d = (self.depth >> level).max(1) as usize;
        match self.format.block_bytes() {
            Some(bb) => w.div_ceil(4) * h.div_ceil(4) * bb * d,
            None => w * h * d * self.format.element_size().unwrap_or(4),
        }
    }

    /// Bytes of the full mip chain for one array slice.
    pub fn slice_size(&self) -> usize {
        (0..self.mip_count).map(|m| self.mip_size(m)).sum()
    }

    /// Payload without the 0x80 alignment padding at the end.
    pub fn pixel_data(&self) -> &[u8] {
        let n = (self.slice_size() * self.array_size as usize).min(self.data.len());
        &self.data[..n]
    }

    /// Encode as a `.dds` file (legacy header where possible, DX10 otherwise).
    pub fn to_dds(&self) -> Vec<u8> {
        const DDSD_CAPS: u32 = 0x1;
        const DDSD_HEIGHT: u32 = 0x2;
        const DDSD_WIDTH: u32 = 0x4;
        const DDSD_PIXELFORMAT: u32 = 0x1000;
        const DDSD_MIPMAPCOUNT: u32 = 0x20000;
        const DDSD_LINEARSIZE: u32 = 0x80000;
        const DDSD_DEPTH: u32 = 0x800000;
        const DDPF_FOURCC: u32 = 0x4;
        const DDSCAPS_COMPLEX: u32 = 0x8;
        const DDSCAPS_TEXTURE: u32 = 0x1000;
        const DDSCAPS_MIPMAP: u32 = 0x400000;
        const DDSCAPS2_CUBEMAP_ALL: u32 = 0xFE00;
        const DDSCAPS2_VOLUME: u32 = 0x200000;

        let fourcc: &[u8; 4] = match self.format {
            DxgiFormat::Bc1Unorm => b"DXT1",
            DxgiFormat::Bc2Unorm => b"DXT3",
            DxgiFormat::Bc3Unorm => b"DXT5",
            _ => b"DX10",
        };
        let mut flags = DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT | DDSD_MIPMAPCOUNT | DDSD_LINEARSIZE;
        if self.dimension == Dimension::D3 {
            flags |= DDSD_DEPTH;
        }
        let mut caps = DDSCAPS_TEXTURE;
        if self.mip_count > 1 {
            caps |= DDSCAPS_MIPMAP | DDSCAPS_COMPLEX;
        }
        let caps2 = match self.dimension {
            Dimension::Cube => DDSCAPS2_CUBEMAP_ALL,
            Dimension::D3 => DDSCAPS2_VOLUME,
            _ => 0,
        };

        let mut out = Vec::with_capacity(148 + self.data.len());
        let mut w = |v: u32| out.extend_from_slice(&v.to_le_bytes());
        w(u32::from_le_bytes(*b"DDS "));
        w(124);
        w(flags);
        w(self.height);
        w(self.width);
        w(self.mip_size(0) as u32);
        w(if self.dimension == Dimension::D3 { self.depth } else { 0 });
        w(self.mip_count);
        for _ in 0..11 {
            w(0);
        }
        // DDS_PIXELFORMAT
        w(32);
        w(DDPF_FOURCC);
        w(u32::from_le_bytes(*fourcc));
        for _ in 0..5 {
            w(0);
        }
        w(caps);
        w(caps2);
        w(0);
        w(0);
        w(0);
        if fourcc == b"DX10" {
            let (dim, misc, array) = match self.dimension {
                Dimension::D1 => (2, 0, self.array_size),
                Dimension::D2 => (3, 0, self.array_size),
                Dimension::D3 => (4, 0, 1),
                Dimension::Cube => (3, 0x4, (self.array_size / 6).max(1)),
            };
            w(self.format.raw());
            w(dim);
            w(misc);
            w(array);
            w(0);
        }
        out.extend_from_slice(self.pixel_data());
        out
    }
}
