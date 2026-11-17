//! `mwtool surfaces --near x,z,r --flat`: the longest straight, level stretches of ground around a point, from
//! the collision mesh (a test strip for captures of the original: docs/probe.md "Capture helpers").
//!
//! The upward-facing collision triangles (normal y > 0.95) are rasterised into a 1 m height grid (the highest
//! one per cell, with its surface). Along straight lines in every direction (0.5° apart, 2 m apart across) a
//! run is a stretch of consecutive cells with ground where neighbouring heights differ by at most 2 cm and the
//! whole run stays within `tol` of height (and on the given surface, if any). The longest runs are listed.
//! `--flat-kerb` instead starts from every low steep face (a kerb) and measures the level run-up along its normal
//! and the level ground behind it, so the kerb is met square. `ground_at` serves `mwtool tp --hops`.

use anyhow::Result;
use mw_assets::AssetStore;
use rayon::prelude::*;

const CELL: f32 = 1.0;
const STEP_MAX: f32 = 0.02;

struct Grid {
    x0: f32,
    z0: f32,
    n: usize,
    h: Vec<f32>,
    surface: Vec<u8>,
    /// Low steep faces (kerbs): centre, unit normal, height.
    kerbs: Vec<([f32; 3], [f32; 3], f32)>,
}

impl Grid {
    fn at(&self, x: f32, z: f32) -> Option<(f32, u8)> {
        let (i, j) = (((x - self.x0) / CELL).floor(), ((z - self.z0) / CELL).floor());
        if i < 0.0 || j < 0.0 || i as usize >= self.n || j as usize >= self.n {
            return None;
        }
        let k = j as usize * self.n + i as usize;
        self.h[k].is_finite().then(|| (self.h[k], self.surface[k]))
    }
}

fn build_grid(store: &AssetStore, [x, z, r]: [f32; 3]) -> Result<Grid> {
    use mw_bundle::ResourceType;
    use mw_formats::CollisionTag;
    let n = (2.0 * r / CELL).ceil() as usize;
    let (x0, z0) = (x - r, z - r);
    let mut g = Grid { x0, z0, n, h: vec![f32::NEG_INFINITY; n * n], surface: vec![0; n * n], kerbs: Vec::new() };
    for p in store.bundle_paths().to_vec() {
        if !p.file_name().is_some_and(|f| f.to_string_lossy().starts_with("TRK_UNIT")) {
            continue;
        }
        let Some(b) = store.bundle(&p) else { continue };
        for e in b.entries_of_type(ResourceType::PolygonSoupList) {
            let soups = mw_formats::PolygonSoupList::parse(&b.load(e)?)?;
            if soups.max[0] < x - r || soups.min[0] > x + r || soups.max[2] < z - r || soups.min[2] > z + r {
                continue;
            }
            let (verts, tris) = soups.triangles();
            for (t, tag) in tris.iter().zip(soups.triangle_tags()) {
                let [a, b, c] = t.map(|i| verts[i as usize]);
                let (u, v) = ([0, 1, 2].map(|k| b[k] - a[k]), [0, 1, 2].map(|k| c[k] - a[k]));
                let nrm = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                let len = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt();
                if len <= 0.0 {
                    continue;
                }
                let (ylo, yhi) = (a[1].min(b[1]).min(c[1]), a[1].max(b[1]).max(c[1]));
                let centre = [0, 1, 2].map(|k| (a[k] + b[k] + c[k]) / 3.0);
                if (nrm[1] / len).abs() < 0.3
                    && (0.05..0.3).contains(&(yhi - ylo))
                    && (centre[0] - x).abs() < r
                    && (centre[2] - z).abs() < r
                {
                    let hn = (nrm[0] * nrm[0] + nrm[2] * nrm[2]).sqrt();
                    g.kerbs.push((centre, [nrm[0] / hn, 0.0, nrm[2] / hn], yhi - ylo));
                }
                if (nrm[1] / len).abs() < 0.95 {
                    continue;
                }
                let surface = CollisionTag::from_u32(tag).surface;
                // Barycentric rasterisation of the triangle's xz footprint at the cell centres.
                let lo = |k: usize| a[k].min(b[k]).min(c[k]);
                let hi = |k: usize| a[k].max(b[k]).max(c[k]);
                let i0 = (((lo(0) - x0) / CELL).floor().max(0.0)) as usize;
                let i1 = (((hi(0) - x0) / CELL).ceil().max(0.0) as usize).min(n);
                let j0 = (((lo(2) - z0) / CELL).floor().max(0.0)) as usize;
                let j1 = (((hi(2) - z0) / CELL).ceil().max(0.0) as usize).min(n);
                let det = (b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2]);
                if det.abs() < 1e-9 {
                    continue;
                }
                for j in j0..j1 {
                    for i in i0..i1 {
                        let (px, pz) = (x0 + (i as f32 + 0.5) * CELL, z0 + (j as f32 + 0.5) * CELL);
                        let l1 = ((px - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (pz - a[2])) / det;
                        let l2 = ((b[0] - a[0]) * (pz - a[2]) - (px - a[0]) * (b[2] - a[2])) / det;
                        if l1 < -1e-4 || l2 < -1e-4 || l1 + l2 > 1.0 + 1e-4 {
                            continue;
                        }
                        let h = a[1] + l1 * (b[1] - a[1]) + l2 * (c[1] - a[1]);
                        let k = j * n + i;
                        if h > g.h[k] {
                            g.h[k] = h;
                            g.surface[k] = surface;
                        }
                    }
                }
            }
        }
        store.evict_all();
    }
    Ok(g)
}

#[derive(Debug, Clone, Copy)]
struct Run {
    len: f32,
    heading: f32,
    start: [f32; 3],
    end: [f32; 3],
    spread: f32,
}

pub fn find(
    store: &AssetStore,
    at: [f32; 3],
    tol: f32,
    surface: Option<u8>,
    heading: Option<f32>,
    kerb: bool,
) -> Result<()> {
    let g = build_grid(store, at)?;
    let filled = g.h.iter().filter(|h| h.is_finite()).count();
    println!("height grid: {}×{} cells of {CELL} m, {filled} with ground", g.n, g.n);
    let r = at[2];
    if kerb {
        return kerb_runs(&g, tol, surface);
    }
    let near_heading = |deg: f32| heading.is_none_or(|h| ((deg - h + 90.0).rem_euclid(180.0) - 90.0).abs() <= 3.0);
    let angles: Vec<f32> = (0..360).map(|k| k as f32 * 0.5).filter(|&d| near_heading(d)).collect();
    let mut runs: Vec<Run> = angles
        .par_iter()
        .flat_map_iter(|&deg| {
            let th = deg.to_radians();
            let (d, p) = ([th.sin(), th.cos()], [th.cos(), -th.sin()]);
            let mut best: Vec<Run> = Vec::new();
            let mut v = -r;
            while v <= r {
                let mut samples: Vec<Option<(f32, u8)>> = Vec::new();
                let mut u = -r;
                while u <= r {
                    let (x, z) = (at[0] + u * d[0] + v * p[0], at[1] + u * d[1] + v * p[1]);
                    samples.push(g.at(x, z).filter(|s| surface.is_none_or(|want| s.1 == want)));
                    u += CELL;
                }
                // Two pointers with monotonic deques for the window's max and min.
                let (mut lo, mut maxq, mut minq) =
                    (0usize, std::collections::VecDeque::<usize>::new(), std::collections::VecDeque::<usize>::new());
                let mut line_best: Option<(usize, usize, f32)> = None;
                for (i, s) in samples.iter().enumerate() {
                    let Some((h, _)) = *s else {
                        lo = i + 1;
                        maxq.clear();
                        minq.clear();
                        continue;
                    };
                    if i > lo && samples[i - 1].is_some_and(|(hp, _)| (h - hp).abs() > STEP_MAX) {
                        lo = i;
                        maxq.clear();
                        minq.clear();
                    }
                    while maxq.back().is_some_and(|&k: &usize| samples[k].unwrap().0 <= h) {
                        maxq.pop_back();
                    }
                    maxq.push_back(i);
                    while minq.back().is_some_and(|&k: &usize| samples[k].unwrap().0 >= h) {
                        minq.pop_back();
                    }
                    minq.push_back(i);
                    while samples[maxq[0]].unwrap().0 - samples[minq[0]].unwrap().0 > tol {
                        lo += 1;
                        while maxq.front().is_some_and(|&k| k < lo) {
                            maxq.pop_front();
                        }
                        while minq.front().is_some_and(|&k| k < lo) {
                            minq.pop_front();
                        }
                    }
                    let spread = samples[maxq[0]].unwrap().0 - samples[minq[0]].unwrap().0;
                    if line_best.is_none_or(|b| i - lo > b.1 - b.0) {
                        line_best = Some((lo, i, spread));
                    }
                }
                if let Some((a, b, spread)) = line_best {
                    let len = (b - a) as f32 * CELL;
                    if len > 30.0 {
                        let pt = |k: usize| {
                            let u = -r + k as f32 * CELL;
                            let (x, z) = (at[0] + u * d[0] + v * p[0], at[1] + u * d[1] + v * p[1]);
                            [x, samples[k].unwrap().0, z]
                        };
                        best.push(Run { len, heading: deg, start: pt(a), end: pt(b), spread });
                    }
                }
                v += 2.0;
            }
            best
        })
        .collect();
    runs.sort_by(|a, b| b.len.total_cmp(&a.len));
    let mut shown: Vec<Run> = Vec::new();
    println!("length  heading  start (x y z)                end (x y z)                  height spread");
    for run in runs {
        let mid = |r: &Run| [(r.start[0] + r.end[0]) / 2.0, (r.start[2] + r.end[2]) / 2.0];
        let m = mid(&run);
        let dup = |o: &Run| {
            let mo = mid(o);
            let dh = ((run.heading - o.heading + 90.0).rem_euclid(180.0) - 90.0).abs();
            (m[0] - mo[0]).hypot(m[1] - mo[1]) < 60.0 && dh < 8.0
        };
        if shown.iter().any(dup) {
            continue;
        }
        println!(
            "{:>6.0}  {:>7.1}  ({:>8.1} {:>6.2} {:>8.1})  ({:>8.1} {:>6.2} {:>8.1})  {:.3}",
            run.len,
            run.heading,
            run.start[0],
            run.start[1],
            run.start[2],
            run.end[0],
            run.end[1],
            run.end[2],
            run.spread
        );
        shown.push(run);
        if shown.len() >= 30 {
            break;
        }
    }
    println!("(heading in degrees, 0 = +z, 90 = +x: `mwtool tp x,y,z,heading` and `mostrusted --spawn`)");
    Ok(())
}

/// Square approaches to kerbs: for every low steep face, the level run-up straight towards it along its normal
/// (the face's normal points at the car that meets it), and the level ground after it.
#[allow(clippy::type_complexity)]
fn kerb_runs(g: &Grid, tol: f32, surface: Option<u8>) -> Result<()> {
    let ok = |s: Option<(f32, u8)>| s.filter(|s| surface.is_none_or(|w| s.1 == w));
    let mut out: Vec<(f32, f32, [f32; 3], [f32; 3], f32, f32)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for &(c, n, _height) in &g.kerbs {
        if !seen.insert(((c[0] * 2.0).round() as i64, (c[2] * 2.0).round() as i64)) {
            continue;
        }
        // Travel against the normal.
        let d = [-n[0], -n[2]];
        let at = |u: f32| g.at(c[0] + u * d[0], c[2] + u * d[1]);
        let Some((h0, _)) = ok(at(-1.0)) else { continue };
        let mut len = 0.0;
        let mut prev = h0;
        let mut u = -1.0;
        loop {
            u -= 0.5;
            match ok(at(u)) {
                Some((h, _)) if (h - prev).abs() <= STEP_MAX && (h - h0).abs() <= tol && len < 1500.0 => {
                    prev = h;
                    len += 0.5;
                }
                _ => break,
            }
        }
        // The level ground after the kerb: as far as it stays within `tol` of its first height, with no step.
        let Some((h1, _)) = ok(at(1.0)) else { continue };
        let (mut after, mut prev, mut u) = (0.0f32, h1, 1.0f32);
        loop {
            u += 0.5;
            match ok(at(u)) {
                Some((h, _)) if (h - prev).abs() <= STEP_MAX && (h - h1).abs() <= tol && after < 1500.0 => {
                    prev = h;
                    after += 0.5;
                }
                _ => break,
            }
        }
        if len < 30.0 || after < 10.0 {
            continue;
        }
        let step = h1 - h0;
        if !(0.05..0.3).contains(&step.abs()) {
            continue;
        }
        let heading = d[0].atan2(d[1]).to_degrees().rem_euclid(360.0);
        let start = [c[0] - (len + 1.0) * d[0], h0, c[2] - (len + 1.0) * d[1]];
        out.push((len, heading, start, c, step, after));
    }
    out.sort_by(|a, b| b.0.min(b.5).total_cmp(&a.0.min(a.5)));
    println!("run-up  heading  start (x ground z)           kerb face (x y z)            step   level after");
    let mut shown: Vec<[f32; 3]> = Vec::new();
    for (len, heading, start, c, step, after) in out {
        if shown.iter().any(|k| (k[0] - c[0]).hypot(k[2] - c[2]) < 10.0) {
            continue;
        }
        shown.push(c);
        println!(
            "{len:>6.0}  {heading:>7.1}  ({:>8.2} {:>6.2} {:>8.2})  ({:>8.2} {:>6.2} {:>8.2})  {step:+.3}  {after:.0} m",
            start[0], start[1], start[2], c[0], c[1], c[2]
        );
        if shown.len() >= 60 {
            break;
        }
    }
    println!("(drive from the start along the heading: the kerb is met square; `mwtool tp x,ground,z,heading`)");
    Ok(())
}

/// The ground height at (x, z): the highest upward-facing drivable collision triangle there below `below` (to skip
/// bridges and roofs above the place meant).
pub fn ground_at(store: &AssetStore, x: f32, z: f32, below: f32) -> Result<Option<f32>> {
    use mw_bundle::ResourceType;
    let mut best: Option<f32> = None;
    for p in store.bundle_paths().to_vec() {
        if !p.file_name().is_some_and(|f| f.to_string_lossy().starts_with("TRK_UNIT")) {
            continue;
        }
        let Some(b) = store.bundle(&p) else { continue };
        for e in b.entries_of_type(ResourceType::PolygonSoupList) {
            let soups = mw_formats::PolygonSoupList::parse(&b.load(e)?)?;
            if soups.max[0] < x || soups.min[0] > x || soups.max[2] < z || soups.min[2] > z {
                continue;
            }
            let (verts, tris) = soups.triangles();
            for (t, tag) in tris.iter().zip(soups.triangle_tags()) {
                if mw_formats::CollisionTag::from_u32(tag).flags & mw_formats::CollisionTag::DRIVABLE == 0 {
                    continue;
                }
                let [a, b, c] = t.map(|i| verts[i as usize]);
                let det = (b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2]);
                if det.abs() < 1e-9 {
                    continue;
                }
                let l1 = ((x - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (z - a[2])) / det;
                let l2 = ((b[0] - a[0]) * (z - a[2]) - (x - a[0]) * (b[2] - a[2])) / det;
                if l1 < 0.0 || l2 < 0.0 || l1 + l2 > 1.0 {
                    continue;
                }
                let (u, v) = ([0, 1, 2].map(|k| b[k] - a[k]), [0, 1, 2].map(|k| c[k] - a[k]));
                let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if len <= 0.0 || (n[1] / len).abs() < 0.7 {
                    continue;
                }
                let h = a[1] + l1 * (b[1] - a[1]) + l2 * (c[1] - a[1]);
                if h < below && best.is_none_or(|g| h > g) {
                    best = Some(h);
                }
            }
        }
        store.evict_all();
    }
    Ok(best)
}
