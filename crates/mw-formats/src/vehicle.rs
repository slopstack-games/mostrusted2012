//! `VehicleGraphicsSpec` (0x106): how a car's models are assembled.
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | ptr  | part models (imports, ptr[part count]) |
//! | 0x04 | ptr  | skeleton (import) |
//! | 0x08 | ptr  | collision PolygonSoupList (import) |
//! | 0x0C | ptr  | wheels (0x90 bytes each) |
//! | 0x10 | u16  | part count (1 in every shipped car: the body) |
//! | 0x12 | u8   | unknown (11 everywhere) |
//! | 0x13 | u8   | wheel count (4) |
//! | 0x14 | u32  | effect locator count? |
//! | 0x18 | ptr  | effect locators? (0x14-byte entries with in-place handles) |
//! | 0x1C | u32  | count (≈ 0x16–0x1A) |
//! | 0x20 | ptr  | u32[count] ids |
//! | 0x24 | ptr  | `{u32 n, ptr, u32}`[count] |
//!
//! Wheel (0x90 bytes):
//!
//! | off  | type  | field |
//! |------|-------|-------|
//! | 0x00 | f32×4 | position (x, y, z, 0); left wheels at +X |
//! | 0x10 | f32×4 | rotation quaternion (x, y, z, w) |
//! | 0x20 | f32×4 | scale (x, y, z, 0) |
//! | 0x30 | ptr×18| imports to GeneSys data (shared between wheels) |
//! | 0x78 | ptr   | wheel part models (imports, ptr[part count]) |
//! | 0x7C | u32   | bitmask: which parts spin (7 = tyre, rim, disc; caliper stays) |
//! | 0x80 | u16   | part count (2–4) |
//! | 0x82 | char[14] | name: `FrontLeft`, `FrontRight`, `RearLeft`, `RearRight` |

use mw_bundle::{Reader, Resource, ResourceId};

use crate::Result;

#[derive(Debug, Clone)]
pub struct VehicleGraphicsSpec {
    pub parts: Vec<Option<ResourceId>>,
    pub skeleton: Option<ResourceId>,
    pub collision: Option<ResourceId>,
    pub wheels: Vec<Wheel>,
}

#[derive(Debug, Clone)]
pub struct Wheel {
    pub name: String,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub parts: Vec<WheelPart>,
}

#[derive(Debug, Clone, Copy)]
pub struct WheelPart {
    pub model: Option<ResourceId>,
    pub spins: bool,
}

impl Wheel {
    /// Wheels on the -X side are the right-hand wheels.
    pub fn is_right(&self) -> bool {
        self.name.contains("Right")
    }

    pub fn is_front(&self) -> bool {
        self.name.starts_with("Front")
    }
}

impl VehicleGraphicsSpec {
    pub fn parse(res: &Resource) -> Result<Self> {
        let data = res.header();
        let r = Reader(data);
        let models = r.u32(0x00)?;
        let n_parts = r.u16(0x10)? as u32;
        let wheels_ptr = r.u32(0x0C)? as usize;
        let n_wheels = r.u8(0x13)? as usize;
        let mut wheels = Vec::with_capacity(n_wheels);
        for w in 0..n_wheels {
            let b = wheels_ptr + w * 0x90;
            let [px, py, pz, _] = r.f32_array::<4>(b)?;
            let [sx, sy, sz, _] = r.f32_array::<4>(b + 0x20)?;
            let parts_ptr = r.u32(b + 0x78)?;
            let spin_mask = r.u32(b + 0x7C)?;
            let n = r.u16(b + 0x80)? as u32;
            let name = r.bytes(b + 0x82, 14)?;
            let name_len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            wheels.push(Wheel {
                name: String::from_utf8_lossy(&name[..name_len]).into_owned(),
                position: [px, py, pz],
                rotation: r.f32_array(b + 0x10)?,
                scale: [sx, sy, sz],
                parts: (0..n)
                    .map(|i| WheelPart { model: res.import_at(parts_ptr + i * 4), spins: spin_mask & (1 << i) != 0 })
                    .collect(),
            });
        }
        Ok(Self {
            parts: (0..n_parts).map(|i| res.import_at(models + i * 4)).collect(),
            skeleton: res.import_at(0x04),
            collision: res.import_at(0x08),
            wheels,
        })
    }
}
