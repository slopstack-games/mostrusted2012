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

/// Groundcover sprites in front of a camera (`--view eye,target`): every collection of every track unit,
/// filtered to the groups the runtime draws there (box within 125·r) and a 30° cone around the view axis.
/// Prints per size exponent how many lie beyond their own range and how many float above the collision.
pub fn view(store: &AssetStore, pose: &str, top: usize) -> Result<()> {
    let v: Vec<f32> = pose.split(',').map(|s| s.trim().parse()).collect::<Result<_, _>>()?;
    anyhow::ensure!(v.len() == 6, "--view wants eye x,y,z,target x,y,z");
    let eye = [v[0], v[1], v[2]];
    let axis = {
        let d = [v[3] - v[0], v[4] - v[1], v[5] - v[2]];
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        d.map(|c| c / l)
    };
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
    let mut done = std::collections::HashSet::new();
    // shift -> [seen, beyond own range, > 2 m above collision, no collision under]
    let mut by_e: BTreeMap<u32, [usize; 4]> = BTreeMap::new();
    let mut rows = Vec::new();
    for name in &units {
        let Some(b) = store.bundle(name) else { continue };
        let ids: Vec<ResourceId> = b.entries_of_type(ResourceType::GroundcoverCollection).map(|e| e.id).collect();
        for id in ids {
            if !done.insert(id) {
                continue;
            }
            let Ok(gc) = store.groundcover(id) else { continue };
            for (g, range) in &gc.groups {
                let near: [f32; 3] = std::array::from_fn(|k| eye[k].clamp(g.min[k], g.max[k]));
                let gd = (0..3).map(|k| (near[k] - eye[k]).powi(2)).sum::<f32>().sqrt();
                if gd > g.radius * fmt::groundcover::DRAW_DISTANCE_SCALE {
                    continue;
                }
                for i in &gc.instances[range.clone()] {
                    let d = [i.position[0] - eye[0], i.position[1] - eye[1], i.position[2] - eye[2]];
                    let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                    let cos = (d[0] * axis[0] + d[1] * axis[1] + d[2] * axis[2]) / dist.max(1e-3);
                    if cos < 30f32.to_radians().cos() {
                        continue;
                    }
                    let c = by_e.entry(i.shift).or_default();
                    c[0] += 1;
                    let beyond = dist > i.draw_range();
                    c[1] += beyond as usize;
                    let dy = ground.under(i.position).map(|(y, _)| i.position[1] - y);
                    match dy {
                        Some(dy) if dy > 2.0 => c[2] += 1,
                        None => c[3] += 1,
                        _ => {}
                    }
                    rows.push((dist, gc.id, *i, dy, beyond, g.radius));
                }
            }
        }
        store.evict_all();
    }
    println!("e: seen / beyond own range / >2 m above collision / no collision under");
    for (e, c) in &by_e {
        println!("e {e}: {} / {} / {} / {}", c[0], c[1], c[2], c[3]);
    }
    // Highest above the horizon first (sprites against the sky), or farthest first.
    let elevation = |r: &(f32, ResourceId, fmt::GroundcoverInstance, Option<f32>, bool, f32)| {
        ((r.2.position[1] - eye[1]) / r.0).asin()
    };
    if std::env::var_os("MW_VIEW_HIGH").is_some() {
        rows.sort_by(|a, b| elevation(b).total_cmp(&elevation(a)));
        println!("highest above the horizon {top}:");
    } else {
        rows.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!("farthest {top}:");
    }
    for (dist, id, i, dy, beyond, r) in rows.iter().take(top) {
        let (w, h) = i.size_m();
        println!(
            "{id} d {dist:7.1} at {:7.1} {:6.1} {:7.1} e {} w/h byte {}/{} ({w:.2} x {h:.2} m) own range {:6.1} beyond {beyond} group r {r:.2} dy {}",
            i.position[0],
            i.position[1],
            i.position[2],
            i.shift,
            i.width,
            i.height,
            i.draw_range(),
            dy.map_or("-".into(), |d| format!("{d:+.1}"))
        );
    }
    Ok(())
}

/// Every placed render mesh of every track unit (main and backdrop `InstanceList` entries) whose triangles
/// cover the point `x,z`, with the surface heights there.
pub fn column(store: &AssetStore, at: &str) -> Result<()> {
    let v: Vec<f32> = at.split(',').map(|s| s.trim().parse()).collect::<Result<_, _>>()?;
    anyhow::ensure!(v.len() == 2, "--column wants x,z");
    let (x, z) = (v[0], v[1]);
    if let Ok(zones) = store.zone_list() {
        println!(
            "zone at the point: {:?}, nearest zone: {:?}",
            zones.zone_at(x, z).map(|z| z.id),
            zones.nearest(x, z).map(|z| z.id)
        );
        if let Ok(list) = std::env::var("MW_COLUMN_ZONES") {
            for id in list.split(',').filter_map(|s| s.parse::<u32>().ok()) {
                if let Some(zone) = zones.zones.iter().find(|z| z.id == id) {
                    let nb: Vec<u32> = zone.neighbours.iter().map(|&(i, f)| zones.zones[i].id * 10 + f).collect();
                    println!(
                        "zone {id}: bounds {:?} district {} arena {} flag {} neighbours (id*10+flag) {nb:?}",
                        zone.bounds, zone.district, zone.arena, zone.flag
                    );
                }
            }
        }
    }
    let units: Vec<String> = store
        .bundle_paths()
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    for name in &units {
        let Some(b) = store.bundle(name) else { continue };
        let gcs: Vec<String> =
            b.entries_of_type(ResourceType::GroundcoverCollection).map(|e| e.id.to_string()).collect();
        if !gcs.is_empty() && std::env::var_os("MW_COLUMN_GC").is_some() {
            println!("{name} groundcover {}", gcs.join(" "));
        }
        for e in b.entries_of_type(ResourceType::InstanceList) {
            let Ok(list) = fmt::InstanceList::parse(&b.load(e)?) else {
                println!("{name}: instance list {} failed", e.id);
                continue;
            };
            let missing = list.instances.iter().filter(|i| i.model.is_none()).count();
            if missing > 0 {
                println!("{name}: {missing} instances without a model");
            }
        }
        let Ok(placements) = store.bundle_placements(&b) else {
            println!("{name}: placements failed");
            continue;
        };
        for p in placements {
            let (meshes, errors) = match store.model_meshes(p.model, 0) {
                Ok(m) => m,
                Err(e) => {
                    println!("{name} model {} backdrop {}: {e}", p.model, p.backdrop);
                    continue;
                }
            };
            for e in &errors {
                println!("{name} model {} backdrop {}: {e}", p.model, p.backdrop);
            }
            let m = p.transform;
            let world = |q: [f32; 4]| -> [f32; 3] {
                std::array::from_fn(|k| q[0] * m[0][k] + q[1] * m[1][k] + q[2] * m[2][k] + m[3][k])
            };
            for sm in &meshes {
                let pos: Vec<[f32; 3]> = sm.mesh.positions.iter().map(|&q| world(q)).collect();
                let mut hits = Vec::new();
                for t in sm.mesh.indices.as_chunks::<3>().0 {
                    let (Some(&a), Some(&b), Some(&c)) =
                        (pos.get(t[0] as usize), pos.get(t[1] as usize), pos.get(t[2] as usize))
                    else {
                        continue;
                    };
                    if let Some(y) = height_at(&[a, b, c], x, z) {
                        hits.push(y);
                    }
                }
                if !hits.is_empty() {
                    hits.sort_by(f32::total_cmp);
                    let mask = store.model(p.model).map_or(0, |m| m.pass_mask);
                    let tris = sm.mesh.indices.len() / 3;
                    let edge = sm
                        .mesh
                        .indices
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .filter_map(|t| {
                            Some(
                                pos.get(t[0] as usize)?
                                    .iter()
                                    .zip(pos.get(t[1] as usize)?)
                                    .map(|(a, b)| (a - b) * (a - b))
                                    .sum::<f32>()
                                    .sqrt(),
                            )
                        })
                        .sum::<f32>()
                        / tris.max(1) as f32;
                    println!(
                        "{name} model {} backdrop {} pass mask {mask:#04x} mesh {} ({tris} tris, edge {edge:.1} m) {} y {:?}",
                        p.model,
                        p.backdrop,
                        sm.mesh_index,
                        sm.shader_name,
                        hits.iter().map(|y| format!("{y:.1}")).collect::<Vec<_>>()
                    );
                }
            }
        }
        store.evict_all();
    }
    Ok(())
}

/// Groundcover groups by who lists their collection: the zone owning the group's place (the containing
/// zone, else the nearest), one of its neighbours, or only zones elsewhere; with the share of instances
/// standing on collision in each class and the distance to the nearest listing zone.
pub fn owners(store: &AssetStore) -> Result<()> {
    let zones = store.zone_list()?;
    let mut ground = Ground::default();
    let units: Vec<String> = store
        .bundle_paths()
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let mut listed: HashMap<ResourceId, Vec<u32>> = HashMap::new();
    for name in &units {
        let Some(b) = store.bundle(name) else { continue };
        let unit = mw_assets::backdrop::track_unit_number(name);
        for e in b.entries_of_type(ResourceType::PolygonSoupList) {
            if let Ok(list) = fmt::PolygonSoupList::parse(&b.load(e)?) {
                ground.add(&list);
            }
        }
        for e in b.entries_of_type(ResourceType::GroundcoverCollection) {
            listed.entry(e.id).or_default().extend(unit);
        }
        store.evict_all();
    }
    let mut per_unit: BTreeMap<u32, usize> = BTreeMap::new();
    for by in listed.values() {
        for u in by {
            *per_unit.entry(*u).or_default() += 1;
        }
    }
    let counts: Vec<usize> = per_unit.values().copied().collect();
    println!(
        "{} collections, listed by {} units: {}..{} per unit, mean {:.1}; listings per collection {}..{}",
        listed.len(),
        per_unit.len(),
        counts.iter().min().unwrap_or(&0),
        counts.iter().max().unwrap_or(&0),
        counts.iter().sum::<usize>() as f32 / counts.len().max(1) as f32,
        listed.values().map(|v| v.len()).min().unwrap_or(0),
        listed.values().map(|v| v.len()).max().unwrap_or(0),
    );
    // Distinct collections a zone plus its neighbours list (the game's draw list has 64 slots).
    let mut by_unit: HashMap<u32, Vec<ResourceId>> = HashMap::new();
    for (id, by) in &listed {
        for u in by {
            by_unit.entry(*u).or_default().push(*id);
        }
    }
    for flags in [3u32, 1] {
        let mut sizes: Vec<(usize, u32)> = zones
            .zones
            .iter()
            .map(|z| {
                let mut set: std::collections::HashSet<ResourceId> = std::collections::HashSet::new();
                set.extend(by_unit.get(&z.id).into_iter().flatten());
                for &(i, f) in &z.neighbours {
                    if f & flags == flags {
                        set.extend(by_unit.get(&zones.zones[i].id).into_iter().flatten());
                    }
                }
                (set.len(), z.id)
            })
            .collect();
        sizes.sort();
        println!(
            "zone + neighbours with flags {flags}: distinct collections min {:?} median {:?} max {:?}; over 64: {}",
            sizes.first(),
            sizes.get(sizes.len() / 2),
            sizes.last(),
            sizes.iter().filter(|s| s.0 > 64).count()
        );
    }
    // class -> [groups, instances, on ground (|dy| < 0.5), no ground, far (> 1 km from every listing zone)]
    let mut classes: BTreeMap<&str, [usize; 5]> = BTreeMap::new();
    let mut ids: Vec<_> = listed.keys().copied().collect();
    ids.sort();
    for id in ids {
        let by = &listed[&id];
        let Ok(gc) = store.groundcover(id) else { continue };
        for (g, range) in &gc.groups {
            let (cx, cz) = ((g.min[0] + g.max[0]) / 2.0, (g.min[2] + g.max[2]) / 2.0);
            let Some(owner) = zones.nearest(cx, cz) else { continue };
            let inside = zones.zone_at(cx, cz).is_some();
            let class = if by.contains(&owner.id) {
                "owner lists"
            } else if owner.neighbours.iter().any(|&(i, _)| by.contains(&zones.zones[i].id)) {
                "a neighbour lists"
            } else {
                "only others list"
            };
            let class = match (class, inside) {
                ("owner lists", true) => "in zone: owner lists",
                ("owner lists", false) => "outside: nearest lists",
                ("a neighbour lists", true) => "in zone: a neighbour lists",
                ("a neighbour lists", false) => "outside: a neighbour of nearest lists",
                (_, true) => "in zone: only others list",
                _ => "outside: only others list",
            };
            let near = by
                .iter()
                .filter_map(|u| zones.zones.iter().find(|z| z.id == *u))
                .map(|z| z.distance(cx, cz))
                .fold(f32::MAX, f32::min);
            let bucket = match near {
                d if d <= 0.0 => "dist: inside a listing zone",
                d if d < 100.0 => "dist: < 100 m",
                d if d < 300.0 => "dist: 100-300 m",
                d if d < 1000.0 => "dist: 300-1000 m",
                _ => "dist: > 1 km",
            };
            for class in [class, bucket] {
                let c = classes.entry(class).or_default();
                c[0] += 1;
                c[4] += (near > 1000.0) as usize;
                for i in &gc.instances[range.clone()] {
                    c[1] += 1;
                    match ground.under(i.position) {
                        Some((y, _)) if (i.position[1] - y).abs() < 0.5 => c[2] += 1,
                        None => c[3] += 1,
                        _ => {}
                    }
                }
            }
        }
        store.evict_all();
    }
    println!(
        "class: groups / instances / on collision (<0.5 m) / no collision / groups > 1 km from every listing zone"
    );
    for (k, c) in &classes {
        println!(
            "{k:<38} {:>7} {:>9} {:>5.1}% {:>5.1}% {:>7}",
            c[0],
            c[1],
            100.0 * c[2] as f32 / c[1].max(1) as f32,
            100.0 * c[3] as f32 / c[1].max(1) as f32,
            c[4]
        );
    }
    Ok(())
}
