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
//! | 0x02 | u8   | junctions (0x150 bytes each, layout open) |
//! | 0x03 | u8   | stop lines (2 bytes each) |
//! | 0x04 | u8   | neighbours (4 bytes each) |
//! | 0x05 | u8   | static (parked) vehicles (0x50 bytes each) |
//! | 0x06 | u16  | rungs |
//! | 0x08 | u16, u16 | unknown, nearly always equal (light range?) |
//! | 0x0E | u16  | entries of the u8 list at 0x30 (0, 1, 2, ...) |
//! | 0x10 | ptr x 8 | sections, rungs, cumulative rung lengths, neighbours, spans, static vehicles, junctions, stop lines |
//!
//! Section (0x30): `u32` first rung, `u8` rung count, `u8` stop line (0xFF none), `u8` stop line count,
//! `u8` span; `u16[3]` forward hulls and `u16[3]` backward hulls (0xFFFF none) at 0x08 / 0x0E, `u8[3]`
//! forward and backward sections at 0x14 / 0x17, `u8` first neighbour at 0x1A, `u8` left / right neighbour
//! counts at 0x1C / 0x1D, `f32` length at 0x20 (= the last cumulative rung length), `u8` flow at 0x24
//! (index into the `TrafficFlow` list; adjacent lanes of one road use different flows), `u8` at 0x25 (0..21:
//! probably a `TrafficFlowType` index, unverified).
//!
//! Rung (0x20): two `[x, y, z, 0]` points across the lane (≈4.5 m apart). Cumulative rung lengths restart
//! at 0 in every section. Neighbour (4): `u8` section, `u8` last rung (rung count − 1), `u16` 0.
//! Span (12): `f32` max vehicles, `f32` its reciprocal, `u16` (a hull index?), `u16` span index.
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
    pub junctions: usize,
    pub stop_lines: usize,
    /// Raw 0x50-byte records (a world matrix plus 0x10 bytes, layout open).
    pub static_vehicles: Vec<[f32; 20]>,
}

#[derive(Debug, Clone)]
pub struct TrafficSection {
    pub first_rung: u32,
    pub rung_count: u8,
    pub stop_line: Option<u8>,
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
            Ok((0..n).map(|k| res.import_at((ptr + 8 * k) as u32)).collect())
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
        hull.junctions = r.u8(h + 2)? as usize;
        hull.stop_lines = r.u8(h + 3)? as usize;
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
            hull.static_vehicles.push(r.f32_array::<20>(p[5] + 0x50 * k)?);
        }
        r.bytes(p[6], 0x150 * hull.junctions)?;
        r.bytes(p[7], 2 * hull.stop_lines)?;
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
}
