//! World placement: `InstanceList` (0x50).
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | ptr  | instances |
//! | 0x04 | u32  | array size |
//! | 0x08 | u32  | always-loaded instance count (the rest are backdrop) |
//! | 0x0C | u32  | version (3) |
//!
//! Instance (0x60 bytes):
//!
//! | off  | type     | field |
//! |------|----------|-------|
//! | 0x00 | ptr      | model (import) |
//! | 0x04 | f32      | unknown (draw distance / LOD bias?) |
//! | 0x08 | u32      | unknown |
//! | 0x10 | ptr      | tint data |
//! | 0x20 | f32×4×4  | transform, row-major, rows = x/y/z axes then translation |

use mw_bundle::{Reader, Resource, ResourceId};

use crate::Result;

#[derive(Debug, Clone)]
pub struct Instance {
    pub model: Option<ResourceId>,
    /// Row-major; row 3 is the translation (D3D convention, `v * M`).
    pub transform: [[f32; 4]; 4],
    pub always_loaded: bool,
    pub unknown_04: f32,
    pub unknown_08: u32,
}

impl Instance {
    /// Transform a point (`v * M`).
    pub fn transform_point(&self, p: [f32; 3]) -> [f32; 3] {
        let m = &self.transform;
        std::array::from_fn(|c| p[0] * m[0][c] + p[1] * m[1][c] + p[2] * m[2][c] + m[3][c])
    }
}

#[derive(Debug, Clone)]
pub struct InstanceList {
    pub instances: Vec<Instance>,
}

impl InstanceList {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let base = r.u32(0x00)?;
        let count = r.u32(0x04)?;
        let always = r.u32(0x08)?;
        let mut instances = Vec::with_capacity(count as usize);
        for i in 0..count {
            let o = base + i * 0x60;
            let mut transform = [[0.0; 4]; 4];
            for (row, t) in transform.iter_mut().enumerate() {
                *t = r.f32_array(o as usize + 0x20 + row * 16)?;
            }
            instances.push(Instance {
                model: res.import_at(o),
                transform,
                always_loaded: i < always,
                unknown_04: r.f32(o as usize + 4)?,
                unknown_08: r.u32(o as usize + 8)?,
            });
        }
        Ok(Self { instances })
    }
}
