//! `TriggerData` (0x202): the world's trigger volumes, one resource in `HAWAII/TRIGGERS.BNDL`.
//!
//! Loaded by `FUN_0053c2f0`, which walks the three arrays below as one list (boxes, then spheres, then
//! points) and maps each record's id to its index in that list; the collectable hit handler
//! `FUN_007059b0` reads a record's id (+0xc) and kind (+0x10). See docs/formats.md "TriggerData".
//!
//! Header (offsets are relative to the resource start, pointers are offsets):
//!
//! | off  | type      | meaning                                                         |
//! |------|-----------|-----------------------------------------------------------------|
//! | 0x00 | u32       | version (8)                                                     |
//! | 0x04 | u32 ptr   | boxes (0x30 bytes each)                                         |
//! | 0x08 | u32       | box count                                                       |
//! | 0x0c | u32 ptr   | spheres (0x18 bytes each)                                       |
//! | 0x10 | u32       | sphere count                                                    |
//! | 0x14 | u32 ptr   | points (0x20 bytes each)                                        |
//! | 0x18 | u32       | point count                                                     |
//! | 0x1c | u32 ptr   | spatial index: (u32 ptr, u32 count) pairs (structure open)      |
//! | 0x20 | u32       | grid columns (45; inferred)                                     |
//! | 0x24 | u32       | grid rows (39; inferred)                                        |
//! | 0x28 | f32 x2    | grid origin x, z (inferred: -4794.6, -4303.2)                   |
//! | 0x30 | f32 x2    | grid cell size x, z (inferred: 250, 250)                        |
//!
//! Every record starts with the same 0x14 bytes: position `f32[3]` (+0x00), id `u32` (+0x0c), kind `u8`
//! (+0x10), shape `u8` (+0x11: 0 box, 1 sphere, 2 point), flags `u8` (+0x12), pad `u8` (+0x13). Then:
//!
//! - box: rotation quaternion `f32[4]` x y z w (+0x14), size `f32[3]` (+0x24; full or half extents, open)
//! - sphere: radius `f32` (+0x14)
//! - point: direction `f32[3]` (+0x14), unit length
//!
//! The kind byte is the game's collectable type for the free-roam objects (`FUN_006e4920` names them for
//! telemetry): see [`Collectable`].

use mw_bundle::{Reader, Resource};

use crate::Result;

/// Free-roam collectable kinds, from the kind byte as `FUN_006e4920` / `FUN_007059b0` test it. Base-game
/// codes are '0' gate, '2' billboard, '4' speed camera, '<' jack spot, '=' jack spot car, 6 body shop; the
/// letters 'A'..'O' are the same kinds in other content packs (none are in the base TRIGGERS.BNDL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Collectable {
    SecurityGate,
    Billboard,
    SpeedCamera,
    JackspotLocation,
    JackspotCar,
    BodyShop,
}

impl Collectable {
    pub fn from_kind(kind: u8) -> Option<Self> {
        Some(match kind {
            b'0' | b'H' => Self::SecurityGate,
            b'2' | b'A' | b'D' | b'J' | b'M' => Self::Billboard,
            b'4' | b'G' => Self::SpeedCamera,
            b'<' | b'B' | b'E' | b'K' | b'N' => Self::JackspotLocation,
            b'=' | b'C' | b'F' | b'L' | b'O' => Self::JackspotCar,
            6 => Self::BodyShop,
            _ => return None,
        })
    }

    /// The name the game's telemetry uses (`FUN_006e4920`).
    pub fn name(self) -> &'static str {
        match self {
            Self::SecurityGate => "SecGate",
            Self::Billboard => "Billboard",
            Self::SpeedCamera => "SpeedCam",
            Self::JackspotLocation => "JackspotLocation",
            Self::JackspotCar => "JackspotCar",
            Self::BodyShop => "BodyShop",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TriggerShape {
    Box { rotation: [f32; 4], size: [f32; 3] },
    Sphere { radius: f32 },
    Point { direction: [f32; 3] },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trigger {
    pub position: [f32; 3],
    pub id: u32,
    pub kind: u8,
    pub flags: u8,
    pub shape: TriggerShape,
}

impl Trigger {
    pub fn collectable(&self) -> Option<Collectable> {
        Collectable::from_kind(self.kind)
    }

    /// Whether the point lies inside the volume (points never). Boxes take `size` as the full size (an
    /// assumption: it fits the objects, e.g. 10.5 m billboards and road-wide speed camera boxes).
    pub fn contains(&self, p: [f32; 3]) -> bool {
        let d = [p[0] - self.position[0], p[1] - self.position[1], p[2] - self.position[2]];
        match self.shape {
            TriggerShape::Sphere { radius } => d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= radius * radius,
            TriggerShape::Box { rotation: [x, y, z, w], size } => {
                // Rotate d by the conjugate quaternion into the box frame.
                let (qx, qy, qz) = (-x, -y, -z);
                let t = [2.0 * (qy * d[2] - qz * d[1]), 2.0 * (qz * d[0] - qx * d[2]), 2.0 * (qx * d[1] - qy * d[0])];
                let l = [
                    d[0] + w * t[0] + (qy * t[2] - qz * t[1]),
                    d[1] + w * t[1] + (qz * t[0] - qx * t[2]),
                    d[2] + w * t[2] + (qx * t[1] - qy * t[0]),
                ];
                (0..3).all(|i| l[i].abs() <= 0.5 * size[i])
            }
            TriggerShape::Point { .. } => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TriggerData {
    pub version: u32,
    /// Boxes, then spheres, then points: the order `FUN_0053c2f0` indexes them in.
    pub triggers: Vec<Trigger>,
    /// Header +0x20 / +0x24 (inferred grid columns and rows).
    pub grid_dims: [u32; 2],
    /// Header +0x28..0x38 (inferred grid origin x, z and cell size x, z).
    pub grid: [f32; 4],
}

impl TriggerData {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let version = r.u32(0)?;
        if version != 8 {
            return Err(crate::Error::Unsupported(format!("TriggerData version {version}")));
        }
        let mut triggers = Vec::new();
        for (ptr_off, stride, shape) in [(0x04usize, 0x30usize, 0u8), (0x0c, 0x18, 1), (0x14, 0x20, 2)] {
            let base = r.u32(ptr_off)? as usize;
            let count = r.u32(ptr_off + 4)? as usize;
            if count.saturating_mul(stride) > r.0.len() {
                return Err(crate::Error::Malformed(format!("trigger count {count} at {ptr_off:#x}")));
            }
            for i in 0..count {
                let o = base + i * stride;
                let id = r.u32(o + 0xc)?;
                let [kind, shape_code, flags, _] = r.u32(o + 0x10)?.to_le_bytes();
                if shape_code != shape {
                    return Err(crate::Error::Malformed(format!("trigger {id}: shape {shape_code} in array {shape}")));
                }
                let shape = match shape {
                    0 => TriggerShape::Box { rotation: r.f32_array(o + 0x14)?, size: r.f32_array(o + 0x24)? },
                    1 => TriggerShape::Sphere { radius: r.f32(o + 0x14)? },
                    _ => TriggerShape::Point { direction: r.f32_array(o + 0x14)? },
                };
                triggers.push(Trigger { position: r.f32_array(o)?, id, kind, flags, shape });
            }
        }
        Ok(Self { version, triggers, grid_dims: [r.u32(0x20)?, r.u32(0x24)?], grid: r.f32_array(0x28)? })
    }

    pub fn collectables(&self, kind: Collectable) -> impl Iterator<Item = &Trigger> {
        self.triggers.iter().filter(move |t| t.collectable() == Some(kind))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(Collectable::from_kind(b'4'), Some(Collectable::SpeedCamera));
        assert_eq!(Collectable::from_kind(b'0'), Some(Collectable::SecurityGate));
        assert_eq!(Collectable::from_kind(7), None);
    }

    #[test]
    fn box_contains() {
        // 90 degrees about Y: local x runs along world -z.
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let t = Trigger {
            position: [10.0, 0.0, 0.0],
            id: 1,
            kind: b'4',
            flags: 0,
            shape: TriggerShape::Box { rotation: [0.0, s, 0.0, s], size: [10.0, 2.0, 2.0] },
        };
        assert!(t.contains([10.0, 0.0, 4.0]));
        assert!(!t.contains([14.0, 0.0, 0.0]));
    }
}
