//! `TrafficLaneData` (0x701): the traffic lane network. One instance in the install, in `HAWAII/TRAFFIC.BNDL`
//! (2.3 MB, 57 imports). Like Burnout Paradise's `TrafficData` it splits the world into hulls, each holding
//! lane sections made of rungs, but the hull grid, the record sizes and the flow tables differ.
//!
//! Header (offsets in chunk 0, pointers are offsets into the same chunk):
//!
//! | off  | type      | field |
//! |------|-----------|-------|
//! | 0x00 | u16       | version (3) |
//! | 0x02 | u16       | hull count (294) |
//! | 0x04 | u32       | offset of the bundle import table (end of data) |
//! | 0x08 | u32       | 0x50 (cell record size?) |
//! | 0x0C | ptr       | `u32[hulls]`: pointer to each hull's lane data (empty hulls share one zeroed record) |
//! | 0x10 | ptr, u32  | import slots of the `TrafficFlow` GeneSys objects (34): indexed by section `+0x24` |
//! | 0x18 | ptr, u32  | import slots of the `TrafficFlowType` objects (23) |
//! | 0x20 | u32       | 0xFFFFFFFF |
//! | 0x24 | u16, u16  | vehicle type count (11), the same again |
//! | 0x2C..0x34 | ptr | three more pointers to the flow import slots (counts unknown, 0?) |
//! | 0x38 | ptr       | vehicle types, 8 bytes each: `u16 0xFFFF, u16 0, u32 index` |
//! | 0x3C | ptr       | vehicle traits, 20 bytes each: `f32 0.65, 3 x u32 0, f32 1200` |
//! | 0x40, 0x44 | ptr | vehicle ids, `u32` each: low half of a `VehicleComponent` id in `VEHICLES/TRAFFICATTRIBS.BNDL` |
//! | 0x50 | u32       | hull count again |
//! | 0x54 | ptr       | hull cells (0x60), 0x50 each |
//!
//! Hull cell (0x50): `ptr points` (`[x, z, 0, 0]` f32 each), `u16[10]` neighbour cells at 0x04, `u32`
//! neighbour count at 0x18, AABB min at 0x20 / max at 0x30 (`[x, ±FLT_MAX, z, 0]`), `u32` index at 0x40,
//! `u32` point count at 0x48. The cells are ~400 m quads; 99.6 % of a hull's rungs lie inside its cell.
//!
//! Hull lane data (0x40 header):
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | u8   | sections |
//! | 0x01 | u8   | section spans |
//! | 0x02 | u8   | junctions (0x150 bytes each, [`TrafficJunction`]) |
//! | 0x03 | u8   | stop lines (`u16` each: a rung position in 8.8 fixed point, `FUN_00439b60`) |
//! | 0x04 | u8   | neighbours (4 bytes each) |
//! | 0x05 | u8   | static (parked) vehicles (0x50 bytes each, [`TrafficStaticVehicle`]) |
//! | 0x06 | u16  | rungs |
//! | 0x08 | u16, u16 | unknown, nearly always equal (light range?) |
//! | 0x0E | u16  | entries of the u8 list at 0x30 (0, 1, 2, ...) |
//! | 0x10 | ptr x 8 | sections, rungs, cumulative rung lengths, neighbours, spans, static vehicles, junctions, stop lines |
//!
//! Section (0x30): `u32` first rung, `u8` rung count, `u8` first stop line (0xFF none), `u8` stop line count,
//! `u8` span (index into the hull's spans; its count of cars steers the route choice, `FUN_00618470`); `u16[3]` forward hulls and `u16[3]` backward hulls (0xFFFF none) at 0x08 / 0x0E, `u8[3]`
//! forward and backward sections at 0x14 / 0x17, `u8` first neighbour at 0x1A, `u8` left / right neighbour
//! counts at 0x1C / 0x1D, `f32` length at 0x20 (= the last cumulative rung length), `u8` flow at 0x24
//! (index into the `TrafficFlow` list; adjacent lanes of one road use different flows), `u8` at 0x25 (0..21:
//! probably a `TrafficFlowType` index, unverified).
//!
//! Rung (0x20): two `[x, y, z, 0]` points across the lane (≈4.5 m apart). Cumulative rung lengths restart
//! at 0 in every section. Neighbour (4): `u8` section, `u8` last rung (rung count − 1), `u16` 0.
//! Span (12): `f32` max vehicles, `f32` its reciprocal, `u16` hull and `u8` span of the span it continues
//! into (0xFFFF none; `FUN_00618470` sums the load along that chain).
//!
//! Junction (0x150, read in `FUN_00605d30`, the traffic-light cycle): `u16[16]` phase durations at 0x04
//! (tenths of a second, `DAT_00fb1a94` = 0.1), `u8[16]` phase masks at 0x24 (bit k set: light group k
//! holds its stop lines), `u8` phase count at 0x34, `u8` group count at 0x35, groups of 0x20 bytes from
//! 0x40: `u16` light object ids at 0x00, `u8` stop line indices at 0x04, `u16` their hulls at 0x0A,
//! `u8` stop line count at 0x16, `u8` light count at 0x17; light k's object handle (the lamp's placed instance id)
//! is the `u32` at group − 8 + 4k (`FUN_005ef950`).
use mw_bundle::{Reader, Resource, ResourceId};

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct TrafficLaneData {
    pub version: u16,
    pub hulls: Vec<TrafficHull>,
    /// `TrafficFlow` GeneSys objects (speed, density, flow type).
    pub flows: Vec<Option<ResourceId>>,
    /// `TrafficFlowType` GeneSys objects (vehicle lists with proportions).
    pub flow_types: Vec<Option<ResourceId>>,
    /// Traffic vehicle types: low 32 bits of their `VehicleComponent` id.
    pub vehicle_ids: Vec<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct TrafficHull {
    /// Cell polygon, `[x, z]` per point.
    pub points: Vec<[f32; 2]>,
    /// Neighbouring cells (hull indices).
    pub cell_neighbours: Vec<u16>,
    pub sections: Vec<TrafficSection>,
    pub rungs: Vec<[[f32; 3]; 2]>,
    pub cumulative_lengths: Vec<f32>,
    /// `(section, last rung)` per entry.
    pub neighbours: Vec<(u8, u8)>,
    /// `(max vehicles, a, index)` per span.
    pub spans: Vec<(f32, u16, u16)>,
    pub junctions: Vec<TrafficJunction>,
    /// Stop line positions: rung index in the high byte, fraction (1/256) in the low byte.
    pub stop_lines: Vec<u16>,
    /// Parked cars ([`TrafficStaticVehicle`], 0x50 bytes each).
    pub static_vehicles: Vec<TrafficStaticVehicle>,
}

/// A parked car's record (0x50 bytes; read by `FUN_0060db90`, which places the hull's parked cars when the hull
/// becomes active, and `FUN_0060b2f0`, which spawns them at the matrix).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TrafficStaticVehicle {
    /// World matrix, D3D row-major (rows 0..2 the axes, row 3 the position).
    pub matrix: [f32; 16],
    /// `u16` at 0x40: the `TrafficFlowType` the vehicle is drawn from (`FUN_00608c80`).
    pub flow_type: u16,
    /// `u8` at 0x42: the chance (percent) that the car is there (`LCG % 100 + 1 <= chance`).
    pub chance: u8,
    /// `u8` at 0x43: bit 0 = not during a pursuit.
    pub flags: u8,
    /// 0x44..0x50, not read by the placement code.
    pub rest: [u32; 3],
}

/// A traffic-light junction: a cycle of phases, each holding some light groups' stop lines.
#[derive(Debug, Clone, Default)]
pub struct TrafficJunction {
    /// Phase durations in seconds.
    pub durations: Vec<f32>,
    /// Per phase: bit k set = light group k is red (its stop lines hold traffic).
    pub masks: Vec<u8>,
    pub groups: Vec<TrafficLightGroup>,
}

/// One light group of a junction: the stop lines (hull, index into the hull's stop lines) it controls.
#[derive(Debug, Clone, Default)]
pub struct TrafficLightGroup {
    pub stop_lines: Vec<(u16, u8)>,
    pub lights: Vec<u16>,
    /// Per light: the u32 at group − 8 + 4k (`FUN_005ef950` copies it into the light's record, the object the
    /// light-cycle messages name).
    pub light_objects: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct TrafficSection {
    pub first_rung: u32,
    pub rung_count: u8,
    /// First stop line (index into the hull's `stop_lines`) and how many follow.
    pub stop_line: Option<u8>,
    pub stop_line_count: u8,
    pub span: u8,
    pub forward: Vec<(u16, u8)>,
    pub backward: Vec<(u16, u8)>,
    pub first_neighbour: u8,
    pub left_neighbours: u8,
    pub right_neighbours: u8,
    pub length: f32,
    pub flow: u8,
    pub flow_type: u8,
}

fn bad(what: String) -> Error {
    Error::Malformed(what)
}

impl TrafficLaneData {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let version = r.u16(0)?;
        let n_hulls = r.u16(2)? as usize;
        let imports = |ptr: usize, n: usize| -> Result<Vec<Option<ResourceId>>> {
            if n > 4096 {
                return Err(bad(format!("import list of {n}")));
            }
            Ok((0..n).map(|k| u32::try_from(ptr + 8 * k).ok().and_then(|o| res.import_at(o))).collect())
        };
        let flows = imports(r.u32(0x10)? as usize, r.u32(0x14)? as usize)?;
        let flow_types = imports(r.u32(0x18)? as usize, r.u32(0x1C)? as usize)?;
        let n_veh = r.u16(0x24)? as usize;
        let ids_at = r.u32(0x40)? as usize;
        let vehicle_ids = (0..n_veh).map(|k| r.u32(ids_at + 4 * k)).collect::<mw_bundle::Result<Vec<_>>>()?;
        let lanes_at = r.u32(0x0C)? as usize;
        let cells_at = r.u32(0x54)? as usize;
        let mut hulls = Vec::with_capacity(n_hulls);
        for i in 0..n_hulls {
            let mut hull = TrafficHull::default();
            let c = cells_at + i * 0x50;
            let pts = r.u32(c)? as usize;
            hull.points = (0..r.u32(c + 0x48)?.min(64) as usize)
                .map(|k| r.f32_array::<2>(pts + 16 * k))
                .collect::<mw_bundle::Result<_>>()?;
            hull.cell_neighbours = (0..r.u32(c + 0x18)?.min(10) as usize)
                .map(|k| r.u16(c + 4 + 2 * k))
                .collect::<mw_bundle::Result<_>>()?;
            let h = r.u32(lanes_at + 4 * i)? as usize;
            Self::parse_lanes(r, h, n_hulls, &mut hull).map_err(|e| bad(format!("hull {i}: {e}")))?;
            hulls.push(hull);
        }
        Ok(Self { version, hulls, flows, flow_types, vehicle_ids })
    }

    fn parse_lanes(r: Reader, h: usize, n_hulls: usize, hull: &mut TrafficHull) -> Result<()> {
        let n_sec = r.u8(h)? as usize;
        let n_span = r.u8(h + 1)? as usize;
        let n_junctions = r.u8(h + 2)? as usize;
        let n_stop_lines = r.u8(h + 3)? as usize;
        let n_nb = r.u8(h + 4)? as usize;
        let n_static = r.u8(h + 5)? as usize;
        let n_rungs = r.u16(h + 6)? as usize;
        let p = r.u32_array::<8>(h + 0x10)?.map(|x| x as usize);
        for k in 0..n_rungs {
            let [ax, ay, az, _, bx, by, bz, _] = r.f32_array::<8>(p[1] + 0x20 * k)?;
            hull.rungs.push([[ax, ay, az], [bx, by, bz]]);
            hull.cumulative_lengths.push(r.f32(p[2] + 4 * k)?);
        }
        for k in 0..n_nb {
            hull.neighbours.push((r.u8(p[3] + 4 * k)?, r.u8(p[3] + 4 * k + 1)?));
        }
        for k in 0..n_span {
            let o = p[4] + 12 * k;
            hull.spans.push((r.f32(o)?, r.u16(o + 8)?, r.u16(o + 10)?));
        }
        for k in 0..n_static {
            let o = p[5] + 0x50 * k;
            hull.static_vehicles.push(TrafficStaticVehicle {
                matrix: r.f32_array::<16>(o)?,
                flow_type: r.u16(o + 0x40)?,
                chance: r.u8(o + 0x42)?,
                flags: r.u8(o + 0x43)?,
                rest: r.u32_array::<3>(o + 0x44)?,
            });
        }
        for k in 0..n_junctions {
            hull.junctions.push(Self::parse_junction(r, p[6] + 0x150 * k, n_hulls)?);
        }
        for k in 0..n_stop_lines {
            hull.stop_lines.push(r.u16(p[7] + 2 * k)?);
        }
        for s in 0..n_sec {
            let o = p[0] + 0x30 * s;
            let first_rung = r.u32(o)?;
            let rung_count = r.u8(o + 4)?;
            if first_rung as usize + rung_count as usize > n_rungs {
                return Err(bad(format!("section {s} rungs {first_rung}+{rung_count} of {n_rungs}")));
            }
            let link = |hulls_at: usize, secs_at: usize| -> Result<Vec<(u16, u8)>> {
                let mut v = Vec::new();
                for j in 0..3 {
                    let hi = r.u16(hulls_at + 2 * j)?;
                    if hi == 0xFFFF {
                        continue;
                    }
                    if hi as usize >= n_hulls {
                        return Err(bad(format!("section {s} links to hull {hi}")));
                    }
                    v.push((hi, r.u8(secs_at + j)?));
                }
                Ok(v)
            };
            let stop = r.u8(o + 5)?;
            let first_neighbour = r.u8(o + 0x1A)?;
            let (left, right) = (r.u8(o + 0x1C)?, r.u8(o + 0x1D)?);
            if first_neighbour as usize + left as usize + right as usize > n_nb {
                return Err(bad(format!("section {s} neighbours past {n_nb}")));
            }
            hull.sections.push(TrafficSection {
                first_rung,
                rung_count,
                stop_line: (stop != 0xFF).then_some(stop),
                stop_line_count: r.u8(o + 6)?,
                span: r.u8(o + 7)?,
                forward: link(o + 0x08, o + 0x14)?,
                backward: link(o + 0x0E, o + 0x17)?,
                first_neighbour,
                left_neighbours: left,
                right_neighbours: right,
                length: r.f32(o + 0x20)?,
                flow: r.u8(o + 0x24)?,
                flow_type: r.u8(o + 0x25)?,
            });
        }
        Ok(())
    }

    fn parse_junction(r: Reader, o: usize, n_hulls: usize) -> Result<TrafficJunction> {
        let phases = (r.u8(o + 0x34)? as usize).min(16);
        let n_groups = (r.u8(o + 0x35)? as usize).min(8);
        let mut j = TrafficJunction::default();
        for k in 0..phases {
            j.durations.push(r.u16(o + 4 + 2 * k)? as f32 * 0.1);
            j.masks.push(r.u8(o + 0x24 + k)?);
        }
        for g in 0..n_groups {
            let b = o + 0x40 + 0x20 * g;
            let mut group = TrafficLightGroup::default();
            for k in 0..(r.u8(b + 0x16)? as usize).min(6) {
                let hull = r.u16(b + 0x0A + 2 * k)?;
                if hull as usize >= n_hulls {
                    return Err(bad(format!("junction stop line in hull {hull}")));
                }
                group.stop_lines.push((hull, r.u8(b + 4 + k)?));
            }
            for k in 0..(r.u8(b + 0x17)? as usize).min(2) {
                group.lights.push(r.u16(b + 2 * k)?);
                // `FUN_005ef950` copies the u32 at group − 8 + 4k into the light's record (its object).
                group.light_objects.push(r.u32(b - 8 + 4 * k)?);
            }
            j.groups.push(group);
        }
        Ok(j)
    }
}

/// One lane section as a centre-line polyline (what a traffic spawner needs).
#[derive(Debug, Clone)]
pub struct LanePolyline {
    pub hull: usize,
    pub section: usize,
    /// Rung midpoints, `[x, y, z]`.
    pub points: Vec<[f32; 3]>,
    /// Index into [`TrafficLaneData::flows`] (its `TrafficFlow` holds the speed in mph and the density).
    pub flow: u8,
    pub length: f32,
}

impl TrafficLaneData {
    /// Lane sections with at least one rung midpoint within `radius` of `(x, z)` (ground plane).
    pub fn lanes_near(&self, x: f32, z: f32, radius: f32) -> Vec<LanePolyline> {
        let r2 = radius * radius;
        let mut out = Vec::new();
        for (hi, hull) in self.hulls.iter().enumerate() {
            for (si, s) in hull.sections.iter().enumerate() {
                let Some(rungs) = hull.rungs.get(s.first_rung as usize..s.first_rung as usize + s.rung_count as usize)
                else {
                    continue;
                };
                let points: Vec<[f32; 3]> = rungs
                    .iter()
                    .map(|[a, b]| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5, (a[2] + b[2]) * 0.5])
                    .collect();
                if points.iter().any(|p| (p[0] - x).powi(2) + (p[2] - z).powi(2) <= r2) {
                    out.push(LanePolyline { hull: hi, section: si, points, flow: s.flow, length: s.length });
                }
            }
        }
        out
    }
}
