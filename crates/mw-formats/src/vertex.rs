//! `VertexDescriptor` (0x3): a D3D11 input layout.
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | u32  | unknown (1) |
//! | 0x04 | u32  | bitmask of present semantics |
//! | 0x08 | u32  | 0 |
//! | 0x0C | u32  | attribute count |
//! | 0x10 | 0x14×n | attributes |
//!
//! Attribute (0x14 bytes): `u8 semantic, u8 semantic_index, u8 input_slot,
//! u8 class, u32 DXGI_FORMAT, u32 offset, u32 instance_step_rate, u32 stride`.

use mw_bundle::{Reader, Resource};

use crate::{DxgiFormat, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Semantic {
    Position,
    PositionT,
    Normal,
    Color,
    /// TEXCOORD1..=8 in the game's numbering (stored 0-based here).
    TexCoord(u8),
    BlendIndices,
    BlendWeight,
    Tangent,
    Binormal,
    Color2,
    PSize,
    Unknown(u8),
}

impl Semantic {
    fn from_raw(v: u8) -> Self {
        match v {
            1 => Self::Position,
            2 => Self::PositionT,
            3 => Self::Normal,
            4 => Self::Color,
            5..=12 => Self::TexCoord(v - 5),
            13 => Self::BlendIndices,
            14 => Self::BlendWeight,
            15 => Self::Tangent,
            16 => Self::Binormal,
            17 => Self::Color2,
            18 => Self::PSize,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VertexAttribute {
    pub semantic: Semantic,
    pub semantic_index: u8,
    pub input_slot: u8,
    pub format: DxgiFormat,
    pub offset: u32,
}

#[derive(Debug, Clone)]
pub struct VertexDescriptor {
    pub attributes: Vec<VertexAttribute>,
    pub stride: u32,
}

impl VertexDescriptor {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let count = r.u32(0x0C)? as usize;
        let mut attributes = Vec::with_capacity(count);
        let mut stride = 0;
        for i in 0..count {
            let o = 0x10 + i * 0x14;
            attributes.push(VertexAttribute {
                semantic: Semantic::from_raw(r.u8(o)?),
                semantic_index: r.u8(o + 1)?,
                input_slot: r.u8(o + 2)?,
                format: DxgiFormat::from_raw(r.u32(o + 4)?),
                offset: r.u32(o + 8)?,
            });
            stride = stride.max(r.u32(o + 0x10)?);
        }
        Ok(Self { attributes, stride })
    }

    pub fn find(&self, semantic: Semantic) -> Option<&VertexAttribute> {
        self.attributes.iter().find(|a| a.semantic == semantic)
    }

    /// Decode one attribute for every vertex of a buffer.
    pub fn read_attribute(&self, semantic: Semantic, vertices: &[u8]) -> Option<Vec<[f32; 4]>> {
        let attr = self.find(semantic)?;
        let stride = self.stride as usize;
        if stride == 0 {
            return None;
        }
        (0..vertices.len() / stride)
            .map(|v| attr.format.decode(vertices.get(v * stride + attr.offset as usize..)?))
            .collect()
    }
}
