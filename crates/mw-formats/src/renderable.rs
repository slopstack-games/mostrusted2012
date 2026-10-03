//! `Renderable` (0x5): a set of meshes sharing one body chunk that holds all
//! index and vertex buffers.
//!
//! Header (chunk 0, version 12):
//!
//! | off  | type  | field |
//! |------|-------|-------|
//! | 0x00 | f32×4 | bounding sphere (centre xyz, radius) |
//! | 0x10 | u16   | version (12) |
//! | 0x12 | u16   | mesh count |
//! | 0x14 | u32   | pointer to mesh pointer table |
//! | 0x18 | u8×2  | flags |
//!
//! Mesh (0x60 bytes):
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | 0x10 | compressed oriented bounding box |
//! | 0x10 | u32  | `D3D11_PRIMITIVE_TOPOLOGY` (4 list, 5 strip) |
//! | 0x14 | i32  | base vertex |
//! | 0x18 | u32  | start index |
//! | 0x1C | u32  | index count |
//! | 0x20 | ptr  | material (import) |
//! | 0x24 | u8×4 | sub-part code, ?, ?, ? |
//! | 0x28 | ptr  | index buffer descriptor |
//! | 0x2C | ptr  | vertex buffer descriptor |
//!
//! Buffer descriptors are `{u32 0, u32 1, u32 bind, u32 body_offset,
//! u32 byte_size, u32 index_stride}`.

use mw_bundle::{Reader, Resource, ResourceId};

use crate::{Error, Result, VertexDescriptor};

/// Scale applied to `R16G16B16A16_SNORM` vertex positions (vehicles, characters).
/// Verified against real-world car dimensions.
pub const SNORM_POSITION_SCALE: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topology {
    TriangleList,
    TriangleStrip,
    Other(u32),
}

#[derive(Debug, Clone)]
pub struct Mesh {
    pub topology: Topology,
    pub base_vertex: i32,
    pub start_index: u32,
    pub index_count: u32,
    pub material: Option<ResourceId>,
    pub sub_part: u8,
    pub index_offset: u32,
    pub index_size: u32,
    pub index_stride: u32,
    pub vertex_offset: u32,
    pub vertex_size: u32,
}

#[derive(Debug, Clone)]
pub struct Renderable {
    pub id: ResourceId,
    pub bounding_sphere: [f32; 4],
    pub version: u16,
    pub meshes: Vec<Mesh>,
    body: Vec<u8>,
}

impl Renderable {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let version = r.u16(0x10)?;
        if version != 12 {
            return Err(Error::Unsupported(format!("renderable version {version}")));
        }
        let count = r.u16(0x12)? as usize;
        let table = r.u32(0x14)? as usize;
        let mut meshes = Vec::with_capacity(count);
        for i in 0..count {
            let m = r.u32(table + i * 4)? as usize;
            let ib = r.u32(m + 0x28)? as usize;
            let vb = r.u32(m + 0x2C)? as usize;
            meshes.push(Mesh {
                topology: match r.u32(m + 0x10)? {
                    4 => Topology::TriangleList,
                    5 => Topology::TriangleStrip,
                    t => Topology::Other(t),
                },
                base_vertex: r.i32(m + 0x14)?,
                start_index: r.u32(m + 0x18)?,
                index_count: r.u32(m + 0x1C)?,
                material: res.import_at((m + 0x20) as u32),
                sub_part: r.u8(m + 0x24)?,
                index_offset: r.u32(ib + 0x0C)?,
                index_size: r.u32(ib + 0x10)?,
                index_stride: r.u32(ib + 0x14)?,
                vertex_offset: r.u32(vb + 0x0C)?,
                vertex_size: r.u32(vb + 0x10)?,
            });
        }
        Ok(Self { id: res.id, bounding_sphere: r.f32_array(0)?, version, meshes, body: res.body().to_vec() })
    }

    pub fn vertex_bytes(&self, mesh: &Mesh) -> Result<&[u8]> {
        Ok(Reader(&self.body).bytes(mesh.vertex_offset as usize, mesh.vertex_size as usize)?)
    }

    /// Raw indices as stored (strip or list), honouring start index.
    pub fn raw_indices(&self, mesh: &Mesh) -> Result<Vec<u32>> {
        let bytes = Reader(&self.body).bytes(mesh.index_offset as usize, mesh.index_size as usize)?;
        let stride = mesh.index_stride.max(2) as usize;
        let all: Vec<u32> = bytes
            .chunks_exact(stride)
            .map(|c| {
                if stride == 4 {
                    u32::from_le_bytes(c.try_into().unwrap())
                } else {
                    u16::from_le_bytes([c[0], c[1]]) as u32
                }
            })
            .collect();
        let start = mesh.start_index as usize;
        let end = (start + mesh.index_count as usize).min(all.len());
        Ok(all.get(start..end).unwrap_or_default().to_vec())
    }

    /// Indices as a triangle list. Strips are unrolled with primitive restart
    /// (0xFFFF / 0xFFFFFFFF) and degenerate triangles removed.
    pub fn triangle_list(&self, mesh: &Mesh) -> Result<Vec<u32>> {
        let raw = self.raw_indices(mesh)?;
        let restart = if mesh.index_stride == 4 { u32::MAX } else { 0xFFFF };
        let base = |i: u32| (i as i64 + mesh.base_vertex as i64) as u32;
        Ok(match mesh.topology {
            Topology::TriangleList => raw.into_iter().map(base).collect(),
            Topology::TriangleStrip => {
                let mut out = Vec::with_capacity(raw.len() * 3);
                for run in raw.split(|&i| i == restart) {
                    for (k, w) in run.windows(3).enumerate() {
                        let (a, b, c) = (w[0], w[1], w[2]);
                        if a == b || b == c || a == c {
                            continue;
                        }
                        if k % 2 == 0 {
                            out.extend([base(a), base(b), base(c)]);
                        } else {
                            out.extend([base(b), base(a), base(c)]);
                        }
                    }
                }
                out
            }
            Topology::Other(t) => return Err(Error::Unsupported(format!("primitive topology {t}"))),
        })
    }

    /// Convenience: decode positions/normals/uvs for a mesh with its layout.
    pub fn decode_mesh(&self, mesh: &Mesh, layout: &VertexDescriptor) -> Result<DecodedMesh> {
        use crate::Semantic;
        let vb = self.vertex_bytes(mesh)?;
        let mut positions = layout
            .read_attribute(Semantic::Position, vb)
            .ok_or_else(|| Error::Malformed("vertex layout has no POSITION".into()))?;
        // Compressed (SNORM16) positions span ±10 m; the vertex shader rescales them.
        if layout.find(Semantic::Position).is_some_and(|a| a.format == crate::DxgiFormat::R16G16B16A16Snorm) {
            for p in &mut positions {
                for c in &mut p[..3] {
                    *c *= SNORM_POSITION_SCALE;
                }
                p[3] = 1.0;
            }
        }
        let normals = layout.read_attribute(Semantic::Normal, vb);
        let tangents = layout.read_attribute(Semantic::Tangent, vb);
        let binormals = layout.read_attribute(Semantic::Binormal, vb);
        let texcoords = (0..8).map(|i| layout.read_attribute(Semantic::TexCoord(i), vb)).collect();
        let n = positions.len() as u32;
        // Drop triangles that reference vertices outside the buffer.
        let indices: Vec<u32> = self
            .triangle_list(mesh)?
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|t| t.iter().all(|&i| i < n))
            .flatten()
            .copied()
            .collect();
        Ok(DecodedMesh { positions, normals, tangents, binormals, texcoords, indices })
    }
}

#[derive(Debug, Clone)]
pub struct DecodedMesh {
    pub positions: Vec<[f32; 4]>,
    pub normals: Option<Vec<[f32; 4]>>,
    pub tangents: Option<Vec<[f32; 4]>>,
    pub binormals: Option<Vec<[f32; 4]>>,
    /// `TEXCOORD0..7` (HLSL numbering; the game's semantic 5 is TEXCOORD0).
    pub texcoords: Vec<Option<Vec<[f32; 4]>>>,
    pub indices: Vec<u32>,
}

impl DecodedMesh {
    pub fn texcoord(&self, index: usize) -> Option<&[[f32; 4]]> {
        self.texcoords.get(index)?.as_deref()
    }
}
