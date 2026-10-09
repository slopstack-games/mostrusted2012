//! `AIData` (0x200): the AI's road graph. One instance in the install, `HAWAII/AI/AIDATA.BNDL` (599 168 bytes, no
//! imports). Road centre-line nodes with a drivable width, linked both ways and sorted into a 125 m grid; a
//! junction-level navigation graph over them (searched with A* for routes); jump links. No racing lines and no
//! speed values: the game builds the AI's path at runtime as a spline through node centres with each node's width
//! as a corridor (docs/decomp.md "AI racers"). Names follow burnout.wiki's symbol-derived "AI Data (NFS Most
//! Wanted)" page; every offset was decoded and checked against the data and the readers in the decomp.
//!
//! Header (pointers are offsets into chunk 0):
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | i32  | version (16) |
//! | 0x10 | 0x30 | node graph: `ptr nodes`, `ptr cells`, `i32 nodes`, `i32 cells x`, `i32 cells z`, `f32 min x`, `f32 min z`, `f32 cell size x`, `f32 cell size z` (read by `FUN_004375b0`) |
//! | 0x40 | ptr  | navigation nodes (junctions), 0x30 each |
//! | 0x44 | ptr  | node mappings, 0x10 each (one per node) |
//! | 0x48 | i32  | junction count |
//! | 0x4C | i32  | mapping count (= node count) |
//! | 0x50 | ptr  | jump links, 0x40 each (`FUN_00520280`) |
//! | 0x54 | i32  | jump link count |
//!
//! Cell (4): `i16` node count, `i16` first node; nodes are stored sorted by cell (index `cz × cells_x + cx`).
//!
//! Node (0x28): `i16[6]` linked nodes (first `count` used; links are symmetric), `f32[3]` position at 0x0C,
//! `f32[3]` side vector at 0x18 (across the road, y = 0, not normalised), `u8` fast width at 0x24 and drivable
//! width at 0x25 (both in 0.25 m; the path element's half-widths are drivable × 0.125 m, `FUN_004f2310`), `u8`
//! link count at 0x26 (3+ = junction), `u8` flags at 0x27 (0x1 shortcut / secondary road, avoided by
//! `FUN_00436c00`'s first pass; 0x8 no reset, 0x10 one-way start, 0x20 one-way end around jumps).
//!
//! Junction (0x30): `u16[6]` exit cost (edge length / 200 m × 511), `i16[6]` target junction at 0x0C, `u32` road
//! section id at 0x18 (0, filled at runtime), `u32` junction id at 0x1C, `i16` node at 0x20, `u8[6]` exit angle
//! (× 2 = degrees) at 0x22, `i8` exit count at 0x28, `i8[6]` exit flags at 0x29 (0x1 shortcut, 0x4 jump).
//!
//! Mapping (0x10): `f32[2]` chain distance / 200 to each end junction, `i16[2]` the junctions, `u8` count (2 on a
//! chain, 1 at a junction).
//!
//! Jump link (0x40): two points of `f32[3]` position, `f32[3]` half-width vector, `u32` zone id (filled at load),
//! `u8` traversable.
use mw_bundle::{Reader, Resource};

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct AiData {
    pub version: i32,
    pub nodes: Vec<AiNode>,
    /// Grid: origin (min x, min z), cell size (x, z), cell counts (x, z); per cell (first node, node count).
    pub grid_min: [f32; 2],
    pub cell_size: [f32; 2],
    pub cells_x: usize,
    pub cells_z: usize,
    pub cells: Vec<(u16, u16)>,
    pub junctions: Vec<AiJunction>,
    pub jumps: Vec<[AiJumpPoint; 2]>,
}

#[derive(Debug, Clone)]
pub struct AiNode {
    pub links: Vec<u16>,
    pub position: [f32; 3],
    /// Across the road (y = 0), not normalised; may be zero.
    pub side: [f32; 3],
    /// Widths in metres: the "fast" (inner) width and the drivable width.
    pub fast_width: f32,
    pub width: f32,
    pub flags: u8,
}

impl AiNode {
    pub const SHORTCUT: u8 = 0x1;
    pub fn junction(&self) -> bool {
        self.links.len() >= 3
    }
}

#[derive(Debug, Clone)]
pub struct AiJunction {
    pub node: u16,
    pub id: u32,
    /// Per exit: target junction, cost (edge length in metres), angle (degrees), flags.
    pub exits: Vec<(u16, f32, f32, u8)>,
}

#[derive(Debug, Clone, Copy)]
pub struct AiJumpPoint {
    pub position: [f32; 3],
    pub half_width: [f32; 3],
    pub traversable: bool,
}

fn bad(what: String) -> Error {
    Error::Malformed(what)
}

impl AiData {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let version = r.i32(0)?;
        let g = 0x10;
        let nodes_at = r.u32(g)? as usize;
        let cells_at = r.u32(g + 4)? as usize;
        let n_nodes = r.i32(g + 8)?;
        let cells_x = r.i32(g + 0xC)?;
        let cells_z = r.i32(g + 0x10)?;
        if !(0..=65535).contains(&n_nodes) || !(1..=4096).contains(&cells_x) || !(1..=4096).contains(&cells_z) {
            return Err(bad(format!("graph of {n_nodes} nodes, {cells_x} x {cells_z} cells")));
        }
        let (n_nodes, cells_x, cells_z) = (n_nodes as usize, cells_x as usize, cells_z as usize);
        let grid_min = r.f32_array::<2>(g + 0x14)?;
        let cell_size = r.f32_array::<2>(g + 0x1C)?;
        let mut nodes = Vec::with_capacity(n_nodes);
        for i in 0..n_nodes {
            let o = nodes_at + 0x28 * i;
            let count = (r.u8(o + 0x26)? as usize).min(6);
            let links = (0..count)
                .map(|k| {
                    let l = r.i16(o + 2 * k)?;
                    if l < 0 || l as usize >= n_nodes {
                        return Err(bad(format!("node {i} links to {l}")));
                    }
                    Ok(l as u16)
                })
                .collect::<Result<Vec<_>>>()?;
            nodes.push(AiNode {
                links,
                position: r.f32_array::<3>(o + 0x0C)?,
                side: r.f32_array::<3>(o + 0x18)?,
                fast_width: r.u8(o + 0x24)? as f32 * 0.25,
                width: r.u8(o + 0x25)? as f32 * 0.25,
                flags: r.u8(o + 0x27)?,
            });
        }
        let cells = (0..cells_x * cells_z)
            .map(|c| Ok((r.i16(cells_at + 4 * c + 2)? as u16, r.i16(cells_at + 4 * c)? as u16)))
            .collect::<Result<Vec<_>>>()?;
        let junctions_at = r.u32(0x40)? as usize;
        let n_junctions = r.i32(0x48)?.clamp(0, 65535) as usize;
        let mut junctions = Vec::with_capacity(n_junctions);
        for j in 0..n_junctions {
            let o = junctions_at + 0x30 * j;
            let n = (r.u8(o + 0x28)? as usize).min(6);
            let exits = (0..n)
                .map(|k| {
                    Ok((
                        r.i16(o + 0x0C + 2 * k)? as u16,
                        r.u16(o + 2 * k)? as f32 / 511.0 * 200.0,
                        r.u8(o + 0x22 + k)? as f32 * 2.0,
                        r.u8(o + 0x29 + k)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            junctions.push(AiJunction { node: r.i16(o + 0x20)? as u16, id: r.u32(o + 0x1C)?, exits });
        }
        let jumps_at = r.u32(0x50)? as usize;
        let n_jumps = r.i32(0x54)?.clamp(0, 65535) as usize;
        let point = |o: usize| -> Result<AiJumpPoint> {
            Ok(AiJumpPoint {
                position: r.f32_array::<3>(o)?,
                half_width: r.f32_array::<3>(o + 0x0C)?,
                traversable: r.u8(o + 0x1C)? != 0,
            })
        };
        let jumps = (0..n_jumps)
            .map(|k| Ok([point(jumps_at + 0x40 * k)?, point(jumps_at + 0x40 * k + 0x20)?]))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { version, nodes, grid_min, cell_size, cells_x, cells_z, cells, junctions, jumps })
    }

    fn cell_of(&self, x: f32, z: f32) -> (usize, usize) {
        let c = |v: f32, min: f32, size: f32, n: usize| (((v - min) / size).floor().max(0.0) as usize).min(n - 1);
        (
            c(x, self.grid_min[0], self.cell_size[0], self.cells_x),
            c(z, self.grid_min[1], self.cell_size[1], self.cells_z),
        )
    }

    /// The node nearest `p` over the 3×3 cells around it, measuring to each node and its link segments with the
    /// height difference weighted by 5 (`FUN_004375b0`, `.rdata` 0x00f9c614); `non_junction` skips junction
    /// nodes. Returns the node index.
    pub fn nearest_node(&self, p: [f32; 3], non_junction: bool) -> Option<usize> {
        let (cx, cz) = self.cell_of(p[0], p[2]);
        let d2 = |a: [f32; 3]| (a[0] - p[0]).powi(2) + (5.0 * (a[1] - p[1])).powi(2) + (a[2] - p[2]).powi(2);
        let mut best: Option<(f32, usize)> = None;
        for z in cz.saturating_sub(1)..=(cz + 1).min(self.cells_z - 1) {
            for x in cx.saturating_sub(1)..=(cx + 1).min(self.cells_x - 1) {
                let (first, count) = self.cells[z * self.cells_x + x];
                for i in first as usize..(first as usize + count as usize).min(self.nodes.len()) {
                    let n = &self.nodes[i];
                    if non_junction && n.junction() {
                        continue;
                    }
                    let mut d = d2(n.position);
                    for &l in &n.links {
                        let b = self.nodes[l as usize].position;
                        let a = n.position;
                        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                        let len2 = ab.iter().map(|v| v * v).sum::<f32>().max(1e-6);
                        let t = (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1] + (p[2] - a[2]) * ab[2]) / len2)
                            .clamp(0.0, 1.0);
                        d = d.min(d2([a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t]));
                    }
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, i));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    /// The shortest node path from `from` to `to` over the links (A* with 3D link lengths and the straight-line
    /// distance as the heuristic). The game searches the junction graph (`FUN_00527300`: exit cost = length,
    /// plus a turn penalty and optional ×5 / ×100 factors on shortcut / jump exits, not applied here) and expands
    /// the junction edges back into nodes; over plain link lengths both give the same path.
    pub fn path(&self, from: usize, to: usize) -> Option<Vec<usize>> {
        use std::cmp::Ordering;
        use std::collections::BinaryHeap;
        #[derive(PartialEq)]
        struct Open(f32, usize);
        impl Eq for Open {}
        impl PartialOrd for Open {
            fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
                Some(self.cmp(o))
            }
        }
        impl Ord for Open {
            fn cmp(&self, o: &Self) -> Ordering {
                o.0.total_cmp(&self.0)
            }
        }
        let dist = |a: usize, b: usize| {
            let (p, q) = (self.nodes[a].position, self.nodes[b].position);
            ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
        };
        if from >= self.nodes.len() || to >= self.nodes.len() {
            return None;
        }
        let mut cost = vec![f32::INFINITY; self.nodes.len()];
        let mut came = vec![usize::MAX; self.nodes.len()];
        let mut open = BinaryHeap::new();
        cost[from] = 0.0;
        open.push(Open(dist(from, to), from));
        while let Some(Open(_, n)) = open.pop() {
            if n == to {
                let mut out = vec![to];
                let mut c = to;
                while c != from {
                    c = came[c];
                    out.push(c);
                }
                out.reverse();
                return Some(out);
            }
            for &l in &self.nodes[n].links {
                let l = l as usize;
                let g = cost[n] + dist(n, l);
                if g < cost[l] {
                    cost[l] = g;
                    came[l] = n;
                    open.push(Open(g + dist(l, to), l));
                }
            }
        }
        None
    }

    /// The neighbour of `node` best aligned with the ground-plane direction `dir` (`FUN_00436c00`): a first pass
    /// over neighbours without the shortcut flag, then any. `exclude` is skipped (the node we came from).
    pub fn next_node(&self, node: usize, dir: [f32; 2], exclude: Option<usize>) -> Option<usize> {
        let n = &self.nodes[node];
        for allow_shortcut in [false, true] {
            let mut best: Option<(f32, usize)> = None;
            for &l in &n.links {
                let l = l as usize;
                let m = &self.nodes[l];
                if Some(l) == exclude || (!allow_shortcut && m.flags & AiNode::SHORTCUT != 0) {
                    continue;
                }
                let d = [m.position[0] - n.position[0], m.position[2] - n.position[2]];
                let len = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-6);
                let dot = (d[0] * dir[0] + d[1] * dir[1]) / len;
                if best.is_none_or(|b| dot > b.0) {
                    best = Some((dot, l));
                }
            }
            if let Some(b) = best {
                return Some(b.1);
            }
        }
        None
    }
}
