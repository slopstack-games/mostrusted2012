//! Streaming zones: `ZoneList` (0x90, the only resource in `HAWAII/PVS.BNDL`).
//!
//! One zone per track unit: a 2-D polygon (x, z) on the ground plane, its neighbours, and the district and
//! event arena it belongs to. The zone ID is the track unit number (`HAWAII/TRK_UNIT<id>.BNDL`).
//!
//! Header (0x1C bytes):
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | ptr  | all zone points (16 bytes each) |
//! | 0x04 | ptr  | zones (0x50 bytes each) |
//! | 0x08 | ptr  | per-zone index of its first point (u32 each) |
//! | 0x0C | ptr  | per-zone point count (u16 each) |
//! | 0x10 | u32  | zone count (169) |
//! | 0x14 | u32  | total point count (941) |
//! | 0x18 | u32  | 3 (version?) |
//!
//! Zone (0x50 bytes):
//!
//! | off  | type  | field |
//! |------|-------|-------|
//! | 0x00 | ptr   | points: `f32 x, f32 z, 0, 0` each, in polygon order |
//! | 0x10 | f32×4 | bounds min (x, -FLT_MAX, z, 0) |
//! | 0x20 | f32×4 | bounds max (x, +FLT_MAX, z, 0) |
//! | 0x30 | ptr   | neighbours: `ptr zone, u32 flags` each |
//! | 0x34 | u32   | zone ID = track unit number |
//! | 0x38 | u32   | district ID (`HAWAII/DISTRICT_<id>.BNDL`) |
//! | 0x3C | u32   | event arena ID (`HAWAII/ARENA_<id>.BNDL`) or 0 |
//! | 0x40 | u8    | flag (0 or 1, meaning unknown) |
//! | 0x42 | u8    | point count |
//! | 0x43 | u8    | neighbour count |
//!
//! Neighbour flags are 3 (470×, polygons share an edge) or 1 (626×, nearby, e.g. touching at a corner).

use mw_bundle::{Reader, Resource};

use crate::Result;

const ZONE_SIZE: usize = 0x50;

#[derive(Debug, Clone)]
pub struct Zone {
    /// Track unit number.
    pub id: u32,
    pub district: u32,
    pub arena: u32,
    pub flag: u8,
    /// Polygon on the ground plane, `[x, z]` per point.
    pub points: Vec<[f32; 2]>,
    /// `[min_x, min_z, max_x, max_z]`.
    pub bounds: [f32; 4],
    /// Neighbouring zones as (index into [`ZoneList::zones`], flags).
    pub neighbours: Vec<(usize, u32)>,
}

impl Zone {
    /// Whether `(x, z)` lies inside the polygon (even-odd rule).
    pub fn contains(&self, x: f32, z: f32) -> bool {
        let [x0, z0, x1, z1] = self.bounds;
        if x < x0 || x > x1 || z < z0 || z > z1 {
            return false;
        }
        let mut inside = false;
        let n = self.points.len();
        for i in 0..n {
            let [ax, az] = self.points[i];
            let [bx, bz] = self.points[(i + 1) % n];
            if (az > z) != (bz > z) && x < (bx - ax) * (z - az) / (bz - az) + ax {
                inside = !inside;
            }
        }
        inside
    }

    /// Distance from `(x, z)` to the polygon (0 inside).
    pub fn distance(&self, x: f32, z: f32) -> f32 {
        if self.contains(x, z) {
            return 0.0;
        }
        let n = self.points.len();
        (0..n)
            .map(|i| {
                let [ax, az] = self.points[i];
                let [bx, bz] = self.points[(i + 1) % n];
                let (dx, dz) = (bx - ax, bz - az);
                let len2 = dx * dx + dz * dz;
                let t = if len2 > 0.0 { (((x - ax) * dx + (z - az) * dz) / len2).clamp(0.0, 1.0) } else { 0.0 };
                let (px, pz) = (ax + t * dx - x, az + t * dz - z);
                (px * px + pz * pz).sqrt()
            })
            .fold(f32::INFINITY, f32::min)
    }
}

#[derive(Debug, Clone)]
pub struct ZoneList {
    pub zones: Vec<Zone>,
}

impl ZoneList {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let base = r.u32(0x04)? as usize;
        let count = r.u32(0x10)? as usize;
        if count > 4096 {
            return Err(crate::Error::Malformed(format!("zone count {count}")));
        }
        let record = |i: usize| base + i * ZONE_SIZE;
        let mut zones = Vec::with_capacity(count);
        for i in 0..count {
            let o = record(i);
            let points_at = r.u32(o)? as usize;
            let neighbours_at = r.u32(o + 0x30)? as usize;
            let n_points = r.u8(o + 0x42)? as usize;
            let n_neighbours = r.u8(o + 0x43)? as usize;
            let [min_x, _, min_z, _] = r.f32_array::<4>(o + 0x10)?;
            let [max_x, _, max_z, _] = r.f32_array::<4>(o + 0x20)?;
            let points =
                (0..n_points).map(|k| r.f32_array::<2>(points_at + k * 16)).collect::<mw_bundle::Result<Vec<_>>>()?;
            let mut neighbours = Vec::with_capacity(n_neighbours);
            for k in 0..n_neighbours {
                let [ptr, flags] = r.u32_array::<2>(neighbours_at + k * 8)?;
                let ptr = ptr as usize;
                if ptr < base || !(ptr - base).is_multiple_of(ZONE_SIZE) || (ptr - base) / ZONE_SIZE >= count {
                    return Err(crate::Error::Malformed(format!("zone {i} neighbour pointer {ptr:#x}")));
                }
                neighbours.push(((ptr - base) / ZONE_SIZE, flags));
            }
            zones.push(Zone {
                id: r.u32(o + 0x34)?,
                district: r.u32(o + 0x38)?,
                arena: r.u32(o + 0x3C)?,
                flag: r.u8(o + 0x40)?,
                points,
                bounds: [min_x, min_z, max_x, max_z],
                neighbours,
            });
        }
        Ok(Self { zones })
    }

    /// The zone whose polygon contains `(x, z)`.
    pub fn zone_at(&self, x: f32, z: f32) -> Option<&Zone> {
        self.zones.iter().find(|zone| zone.contains(x, z))
    }

    /// The zone closest to `(x, z)` (the containing one if any).
    pub fn nearest(&self, x: f32, z: f32) -> Option<&Zone> {
        self.zone_at(x, z).or_else(|| self.zones.iter().min_by(|a, b| a.distance(x, z).total_cmp(&b.distance(x, z))))
    }
}

#[cfg(test)]
mod tests {
    use super::Zone;

    fn square() -> Zone {
        Zone {
            id: 1,
            district: 0,
            arena: 0,
            flag: 0,
            points: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            bounds: [0.0, 0.0, 10.0, 10.0],
            neighbours: vec![],
        }
    }

    #[test]
    fn contains_and_distance() {
        let z = square();
        assert!(z.contains(5.0, 5.0));
        assert!(!z.contains(15.0, 5.0));
        assert_eq!(z.distance(5.0, 5.0), 0.0);
        assert!((z.distance(13.0, 5.0) - 3.0).abs() < 1e-5);
        assert!((z.distance(13.0, 14.0) - 5.0).abs() < 1e-5);
    }
}
