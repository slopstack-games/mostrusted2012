//! `mwtool placement-check`: drops every placement (instances, props, compounds, dynamics, groundcover)
//! onto the track units' collision and reports per layer and per model how far it stands off the ground.
//!
//! Ground = the collision surface under the placement's origin (vertical line through x, z) nearest in y.
//! Collision does not cover far terrain, so judge systematic patterns (one model, one unit, one flag), not
//! single misses.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use mw_assets::AssetStore;
use mw_bundle::{ResourceId, ResourceType};
use mw_formats as fmt;

const CELL: f32 = 8.0;

/// Collision triangles of every loaded unit in a 2D grid over x, z.
#[derive(Default)]
struct Ground {
    tris: Vec<([[f32; 3]; 3], u32)>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

impl Ground {
    fn add(&mut self, list: &fmt::PolygonSoupList) {
        let (verts, tris) = list.triangles();
        let tags = list.triangle_tags();
        for (t, tag) in tris.iter().zip(tags) {
            let p = t.map(|i| verts[i as usize]);
            let idx = self.tris.len() as u32;
            self.tris.push((p, tag));
            let (x0, x1) =
                (p.iter().map(|v| v[0]).fold(f32::MAX, f32::min), p.iter().map(|v| v[0]).fold(f32::MIN, f32::max));
            let (z0, z1) =
                (p.iter().map(|v| v[2]).fold(f32::MAX, f32::min), p.iter().map(|v| v[2]).fold(f32::MIN, f32::max));
            let c = |v: f32| (v / CELL).floor() as i32;
            if c(x1) - c(x0) > 64 || c(z1) - c(z0) > 64 {
                continue;
            }
            for cx in c(x0)..=c(x1) {
                for cz in c(z0)..=c(z1) {
                    self.grid.entry((cx, cz)).or_default().push(idx);
                }
            }
        }
    }

    /// The surface under (x, z) nearest to `y`: (ground y, collision tag).
    fn under(&self, p: [f32; 3]) -> Option<(f32, u32)> {
        let key = ((p[0] / CELL).floor() as i32, (p[2] / CELL).floor() as i32);
        let mut best: Option<(f32, u32)> = None;
        for &i in self.grid.get(&key)? {
            let (t, tag) = &self.tris[i as usize];
            let Some(y) = height_at(t, p[0], p[2]) else { continue };
            if best.is_none_or(|(b, _)| (y - p[1]).abs() < (b - p[1]).abs()) {
                best = Some((y, *tag));
            }
        }
        best
    }
}

fn height_at(t: &[[f32; 3]; 3], x: f32, z: f32) -> Option<f32> {
    let [a, b, c] = t;
    let d = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if d.abs() < 1e-9 {
        return None;
    }
    let l0 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / d;
    let l1 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / d;
    let l2 = 1.0 - l0 - l1;
    const E: f32 = -1e-4;
    (l0 >= E && l1 >= E && l2 >= E).then(|| l0 * a[1] + l1 * b[1] + l2 * c[1])
}

/// Offset statistics of one group of placements.
#[derive(Default)]
struct Stats {
    dy: Vec<f32>,
    none: usize,
    drivable: usize,
}

impl Stats {
    fn add(&mut self, ground: &Ground, p: [f32; 3]) -> Option<f32> {
        match ground.under(p) {
            Some((y, tag)) => {
                let dy = p[1] - y;
                self.dy.push(dy);
                if dy.abs() < 0.5 && fmt::CollisionTag::from_u32(tag).has(fmt::CollisionTag::DRIVABLE) {
                    self.drivable += 1;
                }
                Some(dy)
            }
            None => {
                self.none += 1;
                None
            }
        }
    }

    fn total(&self) -> usize {
        self.dy.len() + self.none
    }

    fn median(&self) -> f32 {
        let mut v = self.dy.clone();
        v.sort_by(f32::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(f32::NAN)
    }

    fn line(&self) -> String {
        let n = self.dy.len().max(1) as f32;
        let frac = |f: &dyn Fn(f32) -> bool| 100.0 * self.dy.iter().filter(|d| f(**d)).count() as f32 / n;
        format!(
            "n={:<8} noground={:<7} median={:+7.2}  |dy|<0.1:{:5.1}%  <0.5:{:5.1}%  >2 up:{:5.1}%  >2 down:{:5.1}%  on-drivable:{}",
            self.total(),
            self.none,
            self.median(),
            frac(&|d| d.abs() < 0.1),
            frac(&|d| d.abs() < 0.5),
            frac(&|d| d > 2.0),
            frac(&|d| d < -2.0),
            self.drivable
        )
    }
}

pub fn run(store: &AssetStore, bundles: &[String], top: usize) -> Result<()> {
    let units: Vec<String> = store
        .bundle_paths()
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    // Collision of every unit (placements near a unit edge may stand on the neighbour's).
    let mut ground = Ground::default();
    for name in &units {
        let Some(b) = store.bundle(name) else { continue };
        for e in b.entries_of_type(ResourceType::PolygonSoupList) {
            if let Ok(list) = fmt::PolygonSoupList::parse(&b.load(e)?) {
                ground.add(&list);
            }
        }
        store.evict_all();
    }
    eprintln!("{} collision triangles", ground.tris.len());

    let checked = if bundles.is_empty() { units.clone() } else { bundles.to_vec() };
    let mut layers: BTreeMap<String, Stats> = BTreeMap::new();
    let mut models: HashMap<(String, ResourceId), Stats> = HashMap::new();
    let mut names: HashMap<ResourceId, String> = HashMap::new();
    let mut units_stats: BTreeMap<String, Stats> = BTreeMap::new();
    let mut worst: Vec<(f32, String, [f32; 3])> = Vec::new();
    let mut is_tree: HashMap<ResourceId, (bool, f32)> = HashMap::new();
    for name in &checked {
        let Some(b) = store.bundle(name) else { continue };
        let wl = store.bundle_layers(&b)?;
        let sets = [
            ("instance", wl.instances.iter().filter(|p| !p.backdrop).collect::<Vec<_>>()),
            ("backdrop", wl.instances.iter().filter(|p| p.backdrop).collect()),
            ("prop", wl.props.iter().collect()),
            ("compound", wl.compounds.iter().collect()),
            ("dynamic", wl.dynamics.iter().collect()),
        ];
        for (layer, list) in sets {
            for p in list {
                // Trees (models drawn with a `Tree_*` shader) are reported as their own layer.
                // Their lowest vertex (model y, scaled by the matrix's y row) is checked, not the origin.
                let (tree, min_y) = *is_tree.entry(p.model).or_insert_with(|| {
                    let Ok((meshes, _)) = store.model_meshes(p.model, 0) else { return (false, 0.0) };
                    let tree = meshes.iter().any(|m| m.shader_name.starts_with("Tree_"));
                    let min_y = meshes.iter().flat_map(|m| &m.mesh.positions).map(|v| v[1]).fold(f32::MAX, f32::min);
                    (tree, if min_y.is_finite() && min_y < f32::MAX { min_y } else { 0.0 })
                });
                let layer = if tree { format!("{layer} tree") } else { layer.to_string() };
                let layer = layer.as_str();
                let lift = if tree { min_y * p.transform[1][1] } else { 0.0 };
                let pos = [p.transform[3][0], p.transform[3][1] + lift, p.transform[3][2]];
                if let Some(n) = b.debug_name(p.model) {
                    names.entry(p.model).or_insert_with(|| n.to_string());
                }
                let dy = models.entry((layer.into(), p.model)).or_default().add(&ground, pos);
                layers.entry(layer.into()).or_default().add(&ground, pos);
                if let Some(dy) = dy.filter(|d| d.abs() > 2.0 && !matches!(layer, "instance" | "backdrop")) {
                    worst.push((dy, format!("{layer} {} {name}", p.model), pos));
                }
            }
        }
        for e in b.entries_of_type(ResourceType::GroundcoverCollection) {
            let Ok(gc) = store.groundcover(e.id) else { continue };
            for i in &gc.instances {
                let layer = format!("groundcover e{}", i.shift);
                let dy = layers.entry(layer.clone()).or_default().add(&ground, i.position);
                units_stats.entry(name.clone()).or_default().add(&ground, i.position);
                if let Some(dy) = dy.filter(|d| d.abs() > 2.0) {
                    worst.push((dy, format!("groundcover {} {name}", gc.id), i.position));
                }
            }
        }
        store.evict_all();
    }

    println!("== per layer (origin y - ground y)");
    for (layer, s) in &layers {
        println!("{layer:<16} {}", s.line());
    }
    println!("== groundcover per unit, worst {top} by share within 0.5 m (n >= 200)");
    let mut u: Vec<_> = units_stats.iter().filter(|(_, s)| s.dy.len() >= 200).collect();
    let near = |s: &Stats| s.dy.iter().filter(|d| d.abs() < 0.5).count() as f32 / s.dy.len().max(1) as f32;
    u.sort_by(|a, b| near(a.1).total_cmp(&near(b.1)));
    for (name, s) in u.iter().take(top) {
        println!("{name:<28} {}", s.line());
    }
    println!("== models (props / compounds / dynamics) with n >= 5, worst {top} by |median|");
    let mut m: Vec<_> =
        models.iter().filter(|((l, _), s)| s.dy.len() >= 5 && !matches!(l.as_str(), "instance" | "backdrop")).collect();
    m.sort_by(|a, b| b.1.median().abs().total_cmp(&a.1.median().abs()));
    for ((layer, id), s) in m.iter().take(top) {
        println!("{layer:<9} {id} {:<32} {}", names.get(id).map_or("", |s| s.as_str()), s.line());
    }
    println!("== largest offsets (|dy| > 2 m, not InstanceList)");
    worst.sort_by(|a, b| b.0.abs().total_cmp(&a.0.abs()));
    for (dy, what, p) in worst.iter().take(top) {
        println!("{dy:+8.2}  {what}  at {:.1},{:.1},{:.1}", p[0], p[1], p[2]);
    }
    Ok(())
}

/// Groundcover per group: share of groups whose instances stand on the ground, by layer / blob / start.
pub fn groups(store: &AssetStore, bundles: &[String]) -> Result<()> {
    let mut ground = Ground::default();
    let units: Vec<String> = store
        .bundle_paths()
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    for name in &units {
        let Some(b) = store.bundle(name) else { continue };
        for e in b.entries_of_type(ResourceType::PolygonSoupList) {
            if let Ok(list) = fmt::PolygonSoupList::parse(&b.load(e)?) {
                ground.add(&list);
            }
        }
        store.evict_all();
    }
    let checked = if bundles.is_empty() { units.clone() } else { bundles.to_vec() };
    let verbose = std::env::var_os("MW_GC_VERBOSE").is_some();
    let mut done = std::collections::HashSet::new();
    let mut classes: BTreeMap<String, [usize; 3]> = BTreeMap::new();
    for name in &checked {
        let Some(b) = store.bundle(name) else { continue };
        for e in b.entries_of_type(ResourceType::GroundcoverCollection) {
            if !done.insert(e.id) {
                continue;
            }
            let col = fmt::GroundcoverCollection::parse(&b.load(e)?)?;
            let blobs: Vec<_> = col
                .blobs
                .iter()
                .filter_map(|&id| fmt::GroundcoverInstances::parse(&store.load(id).ok()?).ok())
                .collect();
            for (l, layer) in col.layers.iter().enumerate() {
                for (gi, g) in layer.groups.iter().enumerate() {
                    let mut one = col.clone();
                    one.layers = vec![fmt::GroundcoverLayer { groups: vec![g.clone()], total: g.count }];
                    let Ok(inst) = one.instances(0, &blobs) else { continue };
                    let mut s = Stats::default();
                    for i in &inst {
                        s.add(&ground, i.position);
                    }
                    if s.dy.len() < 5 {
                        continue;
                    }
                    // Alternative decodes of the raw u16 fractions (diagnostic; MW_GC_VARIANTS=1).
                    if std::env::var_os("MW_GC_VARIANTS").is_some()
                        && let Some(recs) = blobs
                            .get(g.blob as usize)
                            .and_then(|b| b.records.get(g.start as usize..(g.start + g.count) as usize))
                    {
                        type Variant = fn([f32; 3]) -> [f32; 3];
                        let variants: [(&str, Variant); 6] = [
                            ("xyz", |q| q),
                            ("zyx", |q| [q[2], q[1], q[0]]),
                            ("flip x", |q| [1.0 - q[0], q[1], q[2]]),
                            ("flip z", |q| [q[0], q[1], 1.0 - q[2]]),
                            ("flip y", |q| [q[0], 1.0 - q[1], q[2]]),
                            ("flip xz", |q| [1.0 - q[0], q[1], 1.0 - q[2]]),
                        ];
                        for (vn, f) in variants {
                            let mut vs = Stats::default();
                            for r in recs {
                                let q = std::array::from_fn(|k| {
                                    u16::from_le_bytes([r[2 * k], r[2 * k + 1]]) as f32 / 65535.0
                                });
                                let q = f(q);
                                let p = std::array::from_fn(|k| g.min[k] + q[k] * (g.max[k] - g.min[k]));
                                vs.add(&ground, p);
                            }
                            let c = classes.entry(format!("variant {vn}")).or_default();
                            c[0] += 1;
                            c[1] += (vs.dy.iter().filter(|d| d.abs() < 0.1).count() * 2 > vs.dy.len()) as usize;
                            c[2] += vs.dy.iter().filter(|d| d.abs() < 0.1).count();
                        }
                    }
                    let good = s.dy.iter().filter(|d| d.abs() < 0.1).count() * 2 > s.dy.len();
                    if verbose && gi < 40 {
                        println!(
                            "{} L{l} g{gi} blob {} start {} n {} e {} r {:.3} max w/h byte {}/{} box y {:.1}..{:.1} median {:+.2} good {good}",
                            col.id,
                            g.blob,
                            g.start,
                            g.count,
                            g.shift,
                            g.radius,
                            inst.iter().map(|i| i.width).max().unwrap_or(0),
                            inst.iter().map(|i| i.height).max().unwrap_or(0),
                            g.min[1],
                            g.max[1],
                            s.median()
                        );
                    }
                    let keys = [
                        format!("layer {l}"),
                        format!("blob {}", g.blob.min(3)),
                        format!("e {}", g.shift),
                        format!("start0 {}", g.start == 0),
                        format!("group# {}", gi.min(4)),
                    ];
                    for key in keys {
                        let c = classes.entry(key).or_default();
                        c[0] += 1;
                        c[1] += good as usize;
                        c[2] += s.dy.len();
                    }
                }
            }
        }
        store.evict_all();
    }
    for (k, [n, good, inst]) in &classes {
        println!("{k:<14} groups {n:<7} good {good:<7} ({:5.1}%)  instances {inst}", 100.0 * *good as f32 / *n as f32);
    }
    Ok(())
}
