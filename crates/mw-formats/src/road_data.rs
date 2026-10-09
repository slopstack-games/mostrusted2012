//! `RoadData` (0x203): the gameplay road network. One instance in the install, `00000000d5045485` in
//! `HAWAII/GAMELOGIC/ROADDATA.BNDL` (73 584 bytes, chunk 0 only, no imports): 147 roads, 342 sections, 1494
//! points. The game keeps a handle to it at game +0x153a8 and locates every player on it (`FUN_00666200`, one
//! player slot per game-logic tick); the nitrous refill reads it for driving in an oncoming lane
//! (`FUN_0065eec0`) and for shortcuts (docs/decomp.md "Nitrous refill"). Field names are ours.
//!
//! Header (pointers are offsets into chunk 0):
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | u32  | version (9) |
//! | 0x04 | ptr  | roads, 0x34 each |
//! | 0x08 | ptr  | a second array, 0x20 each (not decoded) |
//! | 0x0C | u32  | road count |
//! | 0x10 | u32  | second-array count |
//!
//! Road (0x34): `f32[4]` box (min x, min z, max x, max z) at 0x00; two zero words; four `u32` ids at 0x18
//! (text ids of the road's name when flag bit 0 is set, unverified); `ptr` sections at 0x28; `ptr` the first
//! section's points at 0x2C; `i16` section count at 0x30; `u8` flags at 0x33 (bit 0 named road, bit 1 shortcut:
//! set on no road of the install).
//!
//! Section (0x20): `f32[4]` box at 0x00; `u32` id at 0x10; `ptr` points at 0x14; `u32` point count at 0x18;
//! `f32` at 0x1C (0, 2, 4, 5 or 6; unknown).
//!
//! Point (0x20): `f32[3]` position at 0x00; `f32` capture radius (m) at 0x0C; `f32[3]` tangent at 0x10 (not unit:
//! the Hermite tangent of the cubic to the next point); `i8[4]` oncoming-lane bounds at 0x1C, whole metres of
//! lateral offset along `normalize(tangent × up)`: (0x1C, 0x1D) when the car heads against the tangent, (0x1E,
//! 0x1F) when it heads along it. (0, 9, −9, 0) is a two-way road whose left half is oncoming.
use mw_bundle::{Reader, Resource};

use crate::{Error, Result};

#[derive(Debug, Clone, Default)]
pub struct RoadData {
    pub version: u32,
    pub roads: Vec<Road>,
}

#[derive(Debug, Clone)]
pub struct Road {
    /// min x, min z, max x, max z.
    pub bounds: [f32; 4],
    pub flags: u8,
    pub sections: Vec<RoadSection>,
}

impl Road {
    /// Flag bit 1: the road is a shortcut (`FUN_00667c10`, the TakingShortcut refill).
    pub const SHORTCUT: u8 = 0x2;
}

#[derive(Debug, Clone)]
pub struct RoadSection {
    pub bounds: [f32; 4],
    pub id: u32,
    pub points: Vec<RoadPoint>,
}

#[derive(Debug, Clone, Copy)]
pub struct RoadPoint {
    pub position: [f32; 3],
    pub radius: f32,
    pub tangent: [f32; 3],
    /// Oncoming lateral bounds (m): `[lo, hi]` heading against the tangent, then heading along it.
    pub oncoming: [[i8; 2]; 2],
}

/// Where a car is on the road network: road, section and the first point of the segment it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoadPosition {
    pub road: usize,
    pub section: usize,
    pub point: usize,
}

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn inside(b: &[f32; 4], p: V3) -> bool {
    b[0] < p[0] && p[0] < b[2] && b[1] < p[2] && p[2] < b[3]
}

/// The game's Hermite segment (`FUN_00a3ba80`; the basis matrices at `.bss` 0x0118a640 / 0x0118a6c0, read live):
/// position and tangent at `t` from the two points and their tangents.
pub fn hermite(p0: V3, p1: V3, m0: V3, m1: V3, t: f32) -> (V3, V3) {
    let (t2, t3) = (t * t, t * t * t);
    let h = [2.0 * t3 - 3.0 * t2 + 1.0, -2.0 * t3 + 3.0 * t2, t3 - 2.0 * t2 + t, t3 - t2];
    let d = [6.0 * t2 - 6.0 * t, -6.0 * t2 + 6.0 * t, 3.0 * t2 - 4.0 * t + 1.0, 3.0 * t2 - 2.0 * t];
    let f = |w: [f32; 4]| -> V3 { std::array::from_fn(|i| w[0] * p0[i] + w[1] * p1[i] + w[2] * m0[i] + w[3] * m1[i]) };
    (f(h), f(d))
}

impl RoadSection {
    /// `FUN_006592b0`: is `pos` on the cubic from point `i` to `i + 1`? Bisects t from [0, 1] towards the nearer
    /// end (at most 10 halvings, until the two ends' squared distances differ by less than 1 m²); the car is on it
    /// once a midpoint is within (max of the two radii + 1 m) and less than 5 m (`.rdata` 0x00fb82b0) in height.
    pub fn on_segment(&self, i: usize, pos: V3) -> bool {
        let (Some(a), Some(b)) = (self.points.get(i), self.points.get(i + 1)) else { return false };
        let reach = (a.radius.max(b.radius) + 1.0).powi(2);
        let at = |t: f32| hermite(a.position, b.position, a.tangent, b.tangent, t).0;
        let d2 = |p: V3| dot(sub(pos, p), sub(pos, p));
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        let (mut d_lo, mut d_hi) = (d2(at(lo)), d2(at(hi)));
        for _ in 0..10 {
            let m = 0.5 * (lo + hi);
            let p = at(m);
            let dm = d2(p);
            if dm < reach && (pos[1] - p[1]).abs() < 5.0 {
                return true;
            }
            let diff = if d_hi <= d_lo {
                let d = (d_lo - dm).abs();
                (lo, d_lo) = (m, dm);
                d
            } else {
                let d = (d_hi - dm).abs();
                (hi, d_hi) = (m, dm);
                d
            };
            if diff < 1.0 {
                break;
            }
        }
        false
    }

    /// `FUN_0065f410`: the segment `pos` is on, from the nearest point (3D): before the first point or past the last
    /// there is none; otherwise the segment ending at the nearest point, then the one starting there.
    pub fn segment_at(&self, pos: V3) -> Option<usize> {
        let n = self.points.len();
        if n < 2 {
            return None;
        }
        let d2 = |p: &RoadPoint| dot(sub(pos, p.position), sub(pos, p.position));
        let mut k = 0;
        let mut best = f32::MAX;
        for (i, p) in self.points.iter().enumerate() {
            let d = d2(p);
            if d < best {
                (k, best) = (i, d);
            }
        }
        let along = |i: usize| dot(sub(pos, self.points[i].position), self.points[i].tangent);
        if k == 0 {
            (along(0) >= 0.0 && self.on_segment(0, pos)).then_some(0)
        } else if k == n - 1 {
            (along(k) <= 0.0 && self.on_segment(n - 2, pos)).then_some(n - 2)
        } else if self.on_segment(k - 1, pos) {
            Some(k - 1)
        } else {
            self.on_segment(k, pos).then_some(k)
        }
    }
}

impl RoadData {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let version = r.u32(0)?;
        let roads_at = r.u32(0x04)? as usize;
        let n_roads = r.u32(0x0C)? as usize;
        if n_roads > 65535 {
            return Err(Error::Malformed(format!("{n_roads} roads")));
        }
        let i8_at = |o: usize| -> Result<i8> { Ok(r.u8(o)? as i8) };
        let mut roads = Vec::with_capacity(n_roads);
        for i in 0..n_roads {
            let o = roads_at + 0x34 * i;
            let sections_at = r.u32(o + 0x28)? as usize;
            let n_sections = r.i16(o + 0x30)?.max(0) as usize;
            let mut sections = Vec::with_capacity(n_sections);
            for s in 0..n_sections {
                let so = sections_at + 0x20 * s;
                let points_at = r.u32(so + 0x14)? as usize;
                let n_points = r.u32(so + 0x18)? as usize;
                if n_points > 4096 {
                    return Err(Error::Malformed(format!("road {i} section {s}: {n_points} points")));
                }
                let points = (0..n_points)
                    .map(|p| {
                        let po = points_at + 0x20 * p;
                        Ok(RoadPoint {
                            position: r.f32_array::<3>(po)?,
                            radius: r.f32(po + 0x0C)?,
                            tangent: r.f32_array::<3>(po + 0x10)?,
                            oncoming: [[i8_at(po + 0x1C)?, i8_at(po + 0x1D)?], [i8_at(po + 0x1E)?, i8_at(po + 0x1F)?]],
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                sections.push(RoadSection { bounds: r.f32_array::<4>(so)?, id: r.u32(so + 0x10)?, points });
            }
            roads.push(Road { bounds: r.f32_array::<4>(o)?, flags: r.u8(o + 0x33)?, sections });
        }
        Ok(Self { version, roads })
    }

    /// `FUN_006638f0`: the first road (file order) and section whose boxes strictly contain `pos` (x, z) and
    /// whose cubic segments `pos` is on.
    pub fn locate(&self, pos: V3) -> Option<RoadPosition> {
        for (ri, road) in self.roads.iter().enumerate() {
            if !inside(&road.bounds, pos) {
                continue;
            }
            for (si, s) in road.sections.iter().enumerate() {
                if inside(&s.bounds, pos)
                    && let Some(point) = s.segment_at(pos)
                {
                    return Some(RoadPosition { road: ri, section: si, point });
                }
            }
        }
        None
    }

    pub fn road(&self, at: RoadPosition) -> Option<&Road> {
        self.roads.get(at.road)
    }

    /// `FUN_0065eec0`: is a car at `pos` heading along `forward` (its body's forward axis, vehicle +0x100) in an
    /// oncoming lane at the stored road position? The segment is [k − 1, k] when the car is not ahead of point k
    /// along its tangent, else [k, k + 1] (clamped to the section); the car is projected on the straight chord
    /// (t clamped to 0..1) and the cubic's point and tangent there give the lateral offset s along
    /// `normalize(tangent × up)`. The bounds come from the segment's first point: (0x1C, 0x1D) when
    /// forward · tangent ≤ 0 (`.rdata` 0x00fb82a0 = 0), else (0x1E, 0x1F); oncoming when lo < s < hi.
    pub fn oncoming(&self, at: RoadPosition, pos: V3, forward: V3) -> bool {
        let Some(s) = self.roads.get(at.road).and_then(|r| r.sections.get(at.section)) else { return false };
        let Some(pk) = s.points.get(at.point) else { return false };
        let n = s.points.len();
        let heading_against = dot(pk.tangent, forward) <= 0.0;
        let (a, b) = if dot(sub(pos, pk.position), pk.tangent) <= 0.0 {
            (at.point.saturating_sub(1), at.point)
        } else {
            (at.point, (at.point + 1).min(n - 1))
        };
        let (pa, pb) = (&s.points[a], &s.points[b]);
        let (curve, tangent) = if a == b {
            (pa.position, pa.tangent)
        } else {
            let chord = sub(pb.position, pa.position);
            let t = (dot(sub(pos, pa.position), chord) / dot(chord, chord)).clamp(0.0, 1.0);
            hermite(pa.position, pb.position, pa.tangent, pb.tangent, t)
        };
        // tangent × up, up = (0, 1, 0) (`.rdata` 0x00fb8240).
        let side = [-tangent[2], 0.0, tangent[0]];
        let len = (side[0] * side[0] + side[2] * side[2]).sqrt();
        if len <= 0.0 {
            return false;
        }
        let lateral = dot(side, sub(pos, curve)) / len;
        let [lo, hi] = pa.oncoming[usize::from(!heading_against)];
        (lo as f32) < lateral && lateral < hi as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hermite_ends() {
        let (p0, p1, m) = ([0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 0.0, 0.0]);
        assert_eq!(hermite(p0, p1, m, m, 0.0).0, p0);
        assert_eq!(hermite(p0, p1, m, m, 1.0).0, p1);
        let (mid, d) = hermite(p0, p1, m, m, 0.5);
        assert!((mid[0] - 5.0).abs() < 1e-5 && (d[0] - 10.0).abs() < 1e-4);
    }

    fn straight() -> RoadData {
        // A two-way road along +x, points every 50 m, radius 10.
        let points = (0..3)
            .map(|i| RoadPoint {
                position: [50.0 * i as f32, 0.0, 0.0],
                radius: 10.0,
                tangent: [50.0, 0.0, 0.0],
                oncoming: [[0, 9], [-9, 0]],
            })
            .collect();
        let bounds = [-20.0, -20.0, 120.0, 20.0];
        RoadData {
            version: 9,
            roads: vec![Road { bounds, flags: 0, sections: vec![RoadSection { bounds, id: 0, points }] }],
        }
    }

    #[test]
    fn locate_and_oncoming() {
        let r = straight();
        let at = r.locate([60.0, 0.0, 3.0]).unwrap();
        assert_eq!((at.road, at.section), (0, 0));
        assert!(at.point <= 1);
        assert!(r.locate([200.0, 0.0, 0.0]).is_none());
        // tangent × up = (−tz, 0, tx) = (0, 0, 50): +z is the positive side.
        // Heading along +x, the bounds are (−9, 0): z in (−9, 0) is oncoming.
        assert!(r.oncoming(at, [60.0, 0.0, -3.0], [1.0, 0.0, 0.0]));
        assert!(!r.oncoming(at, [60.0, 0.0, 3.0], [1.0, 0.0, 0.0]));
        // Heading against it, (0, 9): z in (0, 9).
        assert!(r.oncoming(at, [60.0, 0.0, 3.0], [-1.0, 0.0, 0.0]));
    }
}
