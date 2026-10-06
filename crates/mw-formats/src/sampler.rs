//! `SamplerState` (0x7): one texture sampler's filter, address modes and LOD range, bound per material
//! slot next to the texture (`Material::samplers[].sampler_state`).
//!
//! The game turns it into a `D3D11_SAMPLER_DESC` field by field (the function that builds the descriptor
//! and calls `CreateSamplerState`; docs/formats.md "SamplerState"):
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | u32  | AddressU (D3D11 numbering: 1 wrap, 2 mirror, 3 clamp, 4 border, 5 mirror once) |
//! | 0x04 | u32  | AddressV |
//! | 0x08 | u32  | AddressW |
//! | 0x0C | u32  | magnification filter (0 point, 1 linear, 2 anisotropic) |
//! | 0x10 | u32  | minification filter (same) |
//! | 0x14 | u32  | mip filter (same) |
//! | 0x18 | f32  | MinLOD (-FLT_MAX in every instance) |
//! | 0x1C | f32  | MaxLOD (FLT_MAX in every instance) |
//! | 0x20 | u32  | MaxAnisotropy |
//! | 0x24 | f32  | MipLODBias |
//! | 0x28 | i32  | comparison function, -1 = none (then D3D11 ALWAYS and a non-comparison filter) |
//! | 0x2C | u8   | border colour: 0 = transparent black, else opaque white |
//! | 0x34 | ptr  | runtime: the created `ID3D11SamplerState` (0 on disk) |
//!
//! Any of the three filters at 2 makes the whole sampler `D3D11_FILTER_ANISOTROPIC`; otherwise the D3D11
//! filter is `min << 4 | mag << 2 | mip`.

use mw_bundle::{Reader, Resource, ResourceId};

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplerState {
    pub id: ResourceId,
    /// U, V, W address modes in D3D11 numbering.
    pub address: [u32; 3],
    pub mag_filter: u32,
    pub min_filter: u32,
    pub mip_filter: u32,
    pub min_lod: f32,
    pub max_lod: f32,
    pub max_anisotropy: u32,
    pub mip_lod_bias: f32,
    pub comparison: i32,
    pub border_white: bool,
}

impl SamplerState {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        Ok(Self {
            id: res.id,
            address: [r.u32(0x00)?, r.u32(0x04)?, r.u32(0x08)?],
            mag_filter: r.u32(0x0C)?,
            min_filter: r.u32(0x10)?,
            mip_filter: r.u32(0x14)?,
            min_lod: r.f32(0x18)?,
            max_lod: r.f32(0x1C)?,
            max_anisotropy: r.u32(0x20)?,
            mip_lod_bias: r.f32(0x24)?,
            comparison: r.u32(0x28)? as i32,
            border_white: r.u8(0x2C)? != 0,
        })
    }

    /// Whether D3D11 samples it anisotropically (any filter field at 2).
    pub fn anisotropic(&self) -> bool {
        self.mag_filter == 2 || self.min_filter == 2 || self.mip_filter == 2
    }

    /// Short description for listings: `wrap/wrap aniso8 bias-0.90`.
    pub fn summary(&self) -> String {
        let a = |m: u32| match m {
            1 => "wrap",
            2 => "mirror",
            3 => "clamp",
            4 => "border",
            5 => "mirror1",
            _ => "?",
        };
        let f = |m: u32| match m {
            0 => "P",
            1 => "L",
            _ => "A",
        };
        let filter = if self.anisotropic() {
            format!("aniso{}", self.max_anisotropy)
        } else {
            format!("min{}mag{}mip{}", f(self.min_filter), f(self.mag_filter), f(self.mip_filter))
        };
        format!("{}/{} {} bias{:+.2}", a(self.address[0]), a(self.address[1]), filter, self.mip_lod_bias)
    }
}
