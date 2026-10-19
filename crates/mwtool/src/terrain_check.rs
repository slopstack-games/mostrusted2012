//! `mwtool terrain-check`: finds drawn ground (terrain / plot meshes of the track units, their lower LODs,
//! and the composed backdrop) that stands above a road mesh at the same spot, i.e. terrain that would
//! show through the road.
//!
//! Roads are the `DriveableSurface*` meshes of the main (non-backdrop) instances, at every LOD. Ground is
//! every `TerrainPBR*` / `PlotPBR*` mesh of the main instances at LOD0, every other mesh of their lower
//! LODs (class `Lod`, split into terrain / plot shaders and the rest), plus every solid backdrop mesh after
//! the zone composition (`mw_assets::backdrop`, all units loaded). Both are rasterised on a grid; at a grid
//! point a ground surface "pokes" when the highest road surface within ±3 m of it lies more than `--eps`
//! below it.
//!
//! Only roads that are drawn **at the same time** as the ground count: each main mesh carries the LOD
//! states it is drawn at, and under the game's LOD rule (`FUN_008a02f0`, `crates/mostrusted/src/lod.rs`) a
//! placement is at LOD l while the camera's distance to its placed LOD0 bounding-sphere centre lies in a
//! shell `r + d[l-1]·s·V .. r + d[l]·s·V` (r = sphere radius, d = the model's LOD distances, s = the
//! placement's LOD scale, V = the view's LOD scale `--scale`). Two meshes are drawn together when some
//! camera position lies in both shells. The same shells give a lower bound for the distance from which a
//! poke can be seen (`near`).
//!
//! Each poke sample is also classified by the collision under it: `driven` when a collision surface lies
//! within 0.3 m of the road mesh (the car drives on that road, so the ground standing out of it is in plain
//! view), `buried` when the collision follows the ground instead (the road mesh runs on under authored
//! ground: the player drives on the ground), `other` otherwise.

use std::collections::HashMap;

use anyhow::Result;
use mw_assets::AssetStore;
use mw_assets::backdrop::track_unit_number;
use mw_bundle::{ResourceId, ResourceType};
use mw_formats as fmt;

const TILE: f32 = 64.0;
const BAND: f32 = 3.0;
/// Collision within this of the road mesh: the road is driven on.
const DRIVEN: f32 = 0.3;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
enum Class {
    /// Main instance terrain / plot mesh at LOD0.
    Main,
    /// Lower LOD (first LOD state) of a main instance whose LOD0 holds ground or road: terrain / plot shaders.
    LodGround(u8),
    /// Lower LOD, any other shader (buildings, bridge decks, foliage cards of the LOD).
    LodOther(u8),
    /// Composed backdrop.
    Backdrop,
    /// The SHADOW-state meshes (`WorldShadow_*`) of models the main view skips: shadow casters only.
    StandInShadow,
}

struct Tri {
    p: [[f32; 3]; 3],
    /// Index into `sources`.
    src: u32,
    /// Index into `shells` (`u32::MAX` = drawn at every distance).
    shell: u32,
}

struct Source {
    class: Class,
    unit: u32,
    model: ResourceId,
    shader: String,
}

/// Where the camera may stand while a placement's mesh is drawn: a shell around its placed LOD0
/// bounding-sphere centre.
#[derive(Clone, Copy)]
struct Shell {
    centre: [f32; 3],
    lo: f32,
    hi: f32,
}

impl Shell {
    const ALWAYS: Shell = Shell { centre: [0.0; 3], lo: 0.0, hi: f32::INFINITY };

    /// A placement drawn at LOD states `l0..=l1`.
    fn new(table: &Lod, scale: f32, view: f32, l0: usize, l1: usize) -> Self {
        let k = scale * view;
        let lo = if l0 == 0 { 0.0 } else { table.radius + table.distances[l0 - 1] * k };
        let hi = table.distances.get(l1).map_or(f32::INFINITY, |d| table.radius + d * k);
        Self { centre: table.centre, lo, hi }
    }

    fn dist(&self, p: [f32; 3]) -> f32 {
        (0..3).map(|i| (p[i] - self.centre[i]).powi(2)).sum::<f32>().sqrt()
    }

    /// Whether one camera position lies in both shells.
    fn meets(&self, o: &Shell) -> bool {
        if self.hi.is_infinite() && self.lo == 0.0 || o.hi.is_infinite() && o.lo == 0.0 {
            return true;
        }
        let k = self.dist(o.centre);
        // ρb in [o.lo, o.hi] with ρb >= self.lo - k and |ρb - k| <= self.hi.
        let lo = o.lo.max(self.lo - k).max(k - self.hi);
        let hi = o.hi.min(k + self.hi);
        lo < hi
    }

    /// Lower bound of the camera's distance to `p` while drawn.
    fn near(&self, p: [f32; 3]) -> f32 {
        if self.lo == 0.0 { 0.0 } else { (self.lo - self.dist(p)).max(0.0) }
    }
}

/// A model's LOD table in world space.
struct Lod {
    centre: [f32; 3],
    radius: f32,
    distances: Vec<f32>,
}

fn world(m: &[[f32; 4]; 4], q: [f32; 4]) -> [f32; 3] {
    std::array::from_fn(|k| q[0] * m[0][k] + q[1] * m[1][k] + q[2] * m[2][k] + m[3][k])
}

fn push_mesh(out: &mut Vec<Tri>, mesh: &fmt::DecodedMesh, m: &[[f32; 4]; 4], src: u32, shell: u32) {
    let pos: Vec<[f32; 3]> = mesh.positions.iter().map(|&q| world(m, q)).collect();
    for t in mesh.indices.as_chunks::<3>().0 {
        let (Some(&a), Some(&b), Some(&c)) = (pos.get(t[0] as usize), pos.get(t[1] as usize), pos.get(t[2] as usize))
        else {
            continue;
        };
        out.push(Tri { p: [a, b, c], src, shell });
    }
}

/// Barycentric height of a triangle at (x, z), None outside.
fn height_at(t: &[[f32; 3]; 3], x: f32, z: f32) -> Option<f32> {
    let [a, b, c] = t;
    let d = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if d.abs() < 1e-9 {
        return None;
    }
    let l0 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / d;
    let l1 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / d;
    let l2 = 1.0 - l0 - l1;
    (l0 >= 0.0 && l1 >= 0.0 && l2 >= 0.0).then(|| l0 * a[1] + l1 * b[1] + l2 * c[1])
}

fn bounds(t: &[[f32; 3]; 3]) -> [f32; 4] {
    let xs = t.map(|v| v[0]);
    let zs = t.map(|v| v[2]);
    [
        xs.iter().copied().fold(f32::MAX, f32::min),
        zs.iter().copied().fold(f32::MAX, f32::min),
        xs.iter().copied().fold(f32::MIN, f32::max),
        zs.iter().copied().fold(f32::MIN, f32::max),
    ]
}

fn tiles_of(b: [f32; 4]) -> impl Iterator<Item = (i32, i32)> {
    let t = |v: f32| (v / TILE).floor() as i32;
    let (i0, j0, i1, j1) = (t(b[0]), t(b[1]), t(b[2]), t(b[3]));
    (j0..=j1).flat_map(move |j| (i0..=i1).map(move |i| (i, j)))
}

/// Collision triangles binned on the tile grid.
#[derive(Default)]
struct Collision {
    tris: Vec<[[f32; 3]; 3]>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

impl Collision {
    fn add(&mut self, list: &fmt::PolygonSoupList) {
        let (verts, tris) = list.triangles();
        for t in &tris {
            let p = t.map(|i| verts[i as usize]);
            let b = bounds(&p);
            if b[2] - b[0] > 2000.0 || b[3] - b[1] > 2000.0 {
                continue;
            }
            let idx = self.tris.len() as u32;
            self.tris.push(p);
            for k in tiles_of(b) {
                self.grid.entry(k).or_default().push(idx);
            }
        }
    }

    /// Whether a collision surface lies within `tol` of `y` at (x, z).
    fn near(&self, x: f32, z: f32, y: f32, tol: f32) -> bool {
        let key = ((x / TILE).floor() as i32, (z / TILE).floor() as i32);
        self.grid.get(&key).is_some_and(|list| {
            list.iter().any(|&i| height_at(&self.tris[i as usize], x, z).is_some_and(|h| (h - y).abs() <= tol))
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
enum Ground {
    Driven,
    Buried,
    Other,
}

/// One reported spot (an 8 m cell).
#[derive(Default)]
struct Spot {
    samples: usize,
    driven: usize,
    max_dh: f32,
    near: f32,
    at: [f32; 3],
}

/// One road sample: height, model, shell.
type RoadSample = (f32, ResourceId, u32);

pub struct Options<'a> {
    pub step: f32,
    pub eps: f32,
    pub top: usize,
    pub only: Option<&'a str>,
    pub all_passes: bool,
    /// The view's LOD scale (`FUN_008b5fe0`; 1.5 = "LodScaling" 1 at a 4:3-ish FOV, `mwtool lods`).
    pub scale: f32,
    /// Count only pokes that can be seen from closer than this (metres).
    pub within: f32,
    /// Count only pokes over a driven road (collision at the road mesh).
    pub driven: bool,
}

pub fn run(store: &AssetStore, o: Options) -> Result<()> {
    let Options { step, eps, top, only, all_passes, scale: view, within, driven: driven_only } = o;
    let units: Vec<String> = store
        .bundle_paths()
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let mut sources: Vec<Source> = Vec::new();
    let mut src_index: HashMap<(Class, u32, ResourceId, String), u32> = HashMap::new();
    let mut source = |class, unit, model, shader: &str| -> u32 {
        *src_index.entry((class, unit, model, shader.to_string())).or_insert_with(|| {
            sources.push(Source { class, unit, model, shader: shader.to_string() });
            sources.len() as u32 - 1
        })
    };
    let mut shells: Vec<Shell> = Vec::new();
    let mut roads: Vec<Tri> = Vec::new();
    let mut ground: Vec<Tri> = Vec::new();
    let mut collision = Collision::default();
    let mut listings: Vec<(u32, ResourceId, [[f32; 4]; 4])> = Vec::new();
    // Per model: (first LOD state, last LOD state, meshes) of each distinct lower-LOD renderable.
    type Levels = Vec<(usize, usize, Vec<mw_assets::SceneMesh>)>;
    let mut lod_models: HashMap<ResourceId, Levels> = HashMap::new();
    let is_ground = |s: &str| s.starts_with("TerrainPBR") || s.starts_with("PlotPBR");
    for name in &units {
        let Some(b) = store.bundle(name) else { continue };
        let unit = track_unit_number(name).unwrap_or(0);
        for e in b.entries_of_type(ResourceType::PolygonSoupList) {
            if let Ok(list) = fmt::PolygonSoupList::parse(&b.load(e)?) {
                collision.add(&list);
            }
        }
        let Ok(layers) = store.bundle_layers(&b) else { continue };
        for p in &layers.instances {
            if p.backdrop {
                listings.push((unit, p.model, p.transform));
                continue;
            }
            let Ok(model) = store.model(p.model) else { continue };
            // The main view skips models whose pass mask has its bit (`Model::in_main_view`).
            if !all_passes && !model.in_main_view() {
                // Their SHADOW state is drawn in the shadow cascades: ground in the light's view.
                let shadow = model.renderable_for_state(fmt::Model::STATE_SHADOW);
                if let Some(meshes) = shadow.and_then(|r| store.renderable_meshes(r).ok()).map(|m| m.0) {
                    for sm in &meshes {
                        let src = source(Class::StandInShadow, unit, p.model, &sm.shader_name);
                        push_mesh(&mut ground, &sm.mesh, &p.transform, src, u32::MAX);
                    }
                }
                continue;
            }
            let Ok((meshes, _)) = store.model_meshes(p.model, 0) else { continue };
            if !meshes.iter().any(|sm| is_ground(&sm.shader_name) || sm.shader_name.starts_with("DriveableSurface")) {
                continue;
            }
            // The placement's LOD table (as `lod.rs`): LOD0 sphere centre placed, radius as is.
            let lod0 = model.renderable_for_state(0);
            let n = model.lod_distances.len().min(4);
            let table = lod0.and_then(|r| store.renderable(r).ok()).filter(|_| n > 0).map(|r| {
                let s = r.bounding_sphere;
                Lod {
                    centre: world(&p.transform, [s[0], s[1], s[2], 1.0]),
                    radius: s[3],
                    distances: model.lod_distances[..n].to_vec(),
                }
            });
            let mut shell = |l0: usize, l1: usize| -> u32 {
                shells.push(table.as_ref().map_or(Shell::ALWAYS, |t| Shell::new(t, p.lod_scale, view, l0, l1)));
                shells.len() as u32 - 1
            };
            let levels = lod_models.entry(p.model).or_insert_with(|| {
                let mut out: Levels = Vec::new();
                let mut prev = lod0;
                for l in 1..n {
                    let r = model.renderable_for_state(l);
                    if r == prev {
                        if let Some(last) = out.last_mut() {
                            last.1 = l;
                        }
                        continue;
                    }
                    prev = r;
                    let meshes = r.and_then(|r| store.renderable_meshes(r).ok()).map(|m| m.0).unwrap_or_default();
                    out.push((l, l, meshes));
                }
                out
            });
            let lod0_last = levels.first().map_or(n.max(1) - 1, |l| l.0 - 1);
            let sh = shell(0, lod0_last);
            for sm in &meshes {
                let s = sm.shader_name.as_str();
                if mw_assets::is_hidden_shader(s) {
                    continue;
                }
                if s.starts_with("DriveableSurface") {
                    let src = source(Class::Main, unit, p.model, s);
                    push_mesh(&mut roads, &sm.mesh, &p.transform, src, sh);
                } else if is_ground(s) {
                    let src = source(Class::Main, unit, p.model, s);
                    push_mesh(&mut ground, &sm.mesh, &p.transform, src, sh);
                }
            }
            for (l0, l1, meshes) in levels.iter() {
                let sh = shell(*l0, *l1);
                for sm in meshes {
                    let s = sm.shader_name.as_str();
                    if mw_assets::is_hidden_shader(s) {
                        continue;
                    }
                    if s.starts_with("DriveableSurface") {
                        let src = source(Class::Main, unit, p.model, s);
                        push_mesh(&mut roads, &sm.mesh, &p.transform, src, sh);
                        continue;
                    }
                    let class = if is_ground(s) { Class::LodGround(*l0 as u8) } else { Class::LodOther(*l0 as u8) };
                    let src = source(class, unit, p.model, s);
                    push_mesh(&mut ground, &sm.mesh, &p.transform, src, sh);
                }
            }
        }
        store.evict_all();
    }
    eprintln!(
        "{} road triangles, {} main ground triangles, {} collision triangles",
        roads.len(),
        ground.len(),
        collision.tris.len()
    );

    // The composed backdrop (every unit loaded), sent once per (model, transform) as the loader does.
    let sel = store.backdrop_selector(listings.iter().map(|(u, m, t)| (*u, *m, t)))?;
    let mut sent = std::collections::HashSet::new();
    for (u, model, t) in &listings {
        let bits: [u32; 16] = std::array::from_fn(|i| t[i / 4][i % 4].to_bits());
        if !sent.insert((*model, bits)) {
            continue;
        }
        let Ok((meshes, _)) = store.model_meshes(*model, 0) else { continue };
        for sm in &meshes {
            let s = sm.shader_name.as_str();
            if mw_assets::is_hidden_shader(s) || mw_assets::backdrop::is_water_shader(s) {
                continue;
            }
            let Some(kept) = sel.filter_mesh(*model, sm.mesh_index, &sm.mesh, t) else { continue };
            let src = source(Class::Backdrop, *u, *model, s);
            push_mesh(&mut ground, &kept, t, src, u32::MAX);
        }
    }
    store.evict_all();
    eprintln!("{} ground triangles with the backdrop", ground.len());
    if let Some(only) = only {
        ground.retain(|t| {
            let s = &sources[t.src as usize];
            match only {
                "backdrop" => s.class == Class::Backdrop,
                "shadow" => s.class == Class::StandInShadow,
                "main" => s.class == Class::Main,
                "lod" => matches!(s.class, Class::LodGround(_) | Class::LodOther(_)),
                "lod-ground" => matches!(s.class, Class::LodGround(_)),
                _ => true,
            }
        });
    }
    let shell_of = |i: u32| if i == u32::MAX { &Shell::ALWAYS } else { &shells[i as usize] };

    // Bin into tiles.
    let mut road_tiles: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (i, t) in roads.iter().enumerate() {
        let b = bounds(&t.p);
        if b[2] - b[0] > 2000.0 || b[3] - b[1] > 2000.0 {
            continue;
        }
        for k in tiles_of(b) {
            road_tiles.entry(k).or_default().push(i as u32);
        }
    }
    let mut ground_tiles: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (i, t) in ground.iter().enumerate() {
        let b = bounds(&t.p);
        for k in tiles_of(b).filter(|k| road_tiles.contains_key(k)) {
            ground_tiles.entry(k).or_default().push(i as u32);
        }
    }

    let n = (TILE / step).round() as usize;
    let mut spots: HashMap<(i32, i32, u32), Spot> = HashMap::new();
    let mut area: HashMap<(Class, Ground), usize> = HashMap::new();
    let mut road_samples = 0usize;
    let mut keys: Vec<_> = road_tiles.keys().copied().collect();
    keys.sort();
    for key in keys {
        let Some(gt) = ground_tiles.get(&key) else { continue };
        let ox = key.0 as f32 * TILE;
        let oz = key.1 as f32 * TILE;
        let mut cells: Vec<Vec<RoadSample>> = vec![Vec::new(); n * n];
        let range = |lo: f32, hi: f32, o: f32| {
            let a = (((lo - o) / step - 0.5).ceil()).max(0.0) as usize;
            let b = (((hi - o) / step - 0.5).floor()).min(n as f32 - 1.0);
            if b < 0.0 { (1, 0) } else { (a, b as usize) }
        };
        for &ri in &road_tiles[&key] {
            let r = &roads[ri as usize];
            let b = bounds(&r.p);
            let (i0, i1) = range(b[0], b[2], ox);
            let (j0, j1) = range(b[1], b[3], oz);
            for j in j0..=j1.min(n - 1) {
                for i in i0..=i1.min(n - 1) {
                    let (x, z) = (ox + (i as f32 + 0.5) * step, oz + (j as f32 + 0.5) * step);
                    if let Some(y) = height_at(&r.p, x, z) {
                        cells[j * n + i].push((y, sources[r.src as usize].model, r.shell));
                    }
                }
            }
        }
        road_samples += cells.iter().filter(|c| !c.is_empty()).count();
        for &gi in gt {
            let g = &ground[gi as usize];
            let gsh = shell_of(g.shell);
            let b = bounds(&g.p);
            let (i0, i1) = range(b[0], b[2], ox);
            let (j0, j1) = range(b[1], b[3], oz);
            for j in j0..=j1.min(n - 1) {
                for i in i0..=i1.min(n - 1) {
                    let ys = &cells[j * n + i];
                    if ys.is_empty() {
                        continue;
                    }
                    let (x, z) = (ox + (i as f32 + 0.5) * step, oz + (j as f32 + 0.5) * step);
                    let Some(h) = height_at(&g.p, x, z) else { continue };
                    // The highest road drawn together with this ground (same LOD state for the same
                    // placement, overlapping camera shells for others).
                    let mut top = f32::MIN;
                    let mut near = 0.0f32;
                    for &(y, _, rs) in ys {
                        if (y - h).abs() > BAND || y <= top {
                            continue;
                        }
                        let rsh = shell_of(rs);
                        if rs != g.shell && !gsh.meets(rsh) {
                            continue;
                        }
                        top = y;
                        let p = [x, y, z];
                        near = gsh.near(p).max(rsh.near(p));
                    }
                    if top == f32::MIN || h - top <= eps || near > within {
                        continue;
                    }
                    let kind = if collision.near(x, z, top, DRIVEN) {
                        Ground::Driven
                    } else if collision.near(x, z, h, DRIVEN) {
                        Ground::Buried
                    } else {
                        Ground::Other
                    };
                    if driven_only && kind != Ground::Driven {
                        continue;
                    }
                    let dh = h - top;
                    let class = sources[g.src as usize].class;
                    *area.entry((class, kind)).or_default() += 1;
                    let spot = spots.entry(((x / 8.0).floor() as i32, (z / 8.0).floor() as i32, g.src)).or_default();
                    if spot.samples == 0 {
                        spot.near = f32::MAX;
                    }
                    spot.samples += 1;
                    spot.driven += usize::from(kind == Ground::Driven);
                    spot.near = spot.near.min(near);
                    if dh > spot.max_dh {
                        spot.max_dh = dh;
                        spot.at = [x, top, z];
                    }
                }
            }
        }
    }
    let cell_area = step * step;
    println!(
        "road area sampled: {:.0} m² (grid {step} m, eps {eps} m, view LOD scale {view}, seen from < {within} m)",
        road_samples as f32 * cell_area
    );
    println!("ground above a road drawn at the same time (m²; driven = collision at the road mesh):");
    let mut classes: Vec<Class> = area.keys().map(|k| k.0).collect();
    classes.sort();
    classes.dedup();
    for c in &classes {
        let a = |g| *area.get(&(*c, g)).unwrap_or(&0) as f32 * cell_area;
        println!(
            "  {:16} driven {:7.0}  buried {:7.0}  other {:7.0}",
            format!("{c:?}"),
            a(Ground::Driven),
            a(Ground::Buried),
            a(Ground::Other)
        );
    }
    // Merge 8 m cells into clusters per source (flood over neighbouring cells).
    type Cells<'a> = Vec<((i32, i32), &'a Spot)>;
    let mut by_src: HashMap<u32, Cells> = HashMap::new();
    for ((cx, cz, src), s) in &spots {
        by_src.entry(*src).or_default().push(((*cx, *cz), s));
    }
    struct Cluster {
        src: u32,
        samples: usize,
        driven: usize,
        max_dh: f32,
        near: f32,
        at: [f32; 3],
    }
    let mut clusters: Vec<Cluster> = Vec::new();
    for (src, list) in by_src {
        let map: HashMap<(i32, i32), &Spot> = list.into_iter().collect();
        let mut seen = std::collections::HashSet::new();
        for &start in map.keys() {
            if !seen.insert(start) {
                continue;
            }
            let mut c = Cluster { src, samples: 0, driven: 0, max_dh: 0.0, near: f32::MAX, at: [0.0; 3] };
            let mut stack = vec![start];
            while let Some(k) = stack.pop() {
                let s = map[&k];
                c.samples += s.samples;
                c.driven += s.driven;
                c.near = c.near.min(s.near);
                if s.max_dh > c.max_dh {
                    c.max_dh = s.max_dh;
                    c.at = s.at;
                }
                for d in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)] {
                    let n = (k.0 + d.0, k.1 + d.1);
                    if map.contains_key(&n) && seen.insert(n) {
                        stack.push(n);
                    }
                }
            }
            clusters.push(c);
        }
    }
    // Rank by visible area weighted by how far it stands out (capped: a wall of terrain counts like 1 m).
    clusters.sort_by(|a, b| {
        let w = |c: &Cluster| c.samples as f32 * c.max_dh.min(1.0);
        w(b).total_cmp(&w(a))
    });
    let zones = store.zone_list().ok();
    println!(
        "top {top} clusters (area m², of it driven, max height above the road, seen from ≥ m, where, zone, source):"
    );
    for c in clusters.iter().take(top) {
        let s = &sources[c.src as usize];
        let zone = zones.as_ref().and_then(|z| z.zone_at(c.at[0], c.at[2])).map(|z| z.id);
        // A camera standing on a driven road 12-25 m away, 2 m up, looking at the spot.
        let p = c.at;
        let mut best: Option<([f32; 3], f32)> = None;
        let key = ((p[0] / TILE).floor() as i32, (p[2] / TILE).floor() as i32);
        for k in (-1..=1).flat_map(|a| (-1..=1).map(move |b| (key.0 + a, key.1 + b))) {
            for &ri in road_tiles.get(&k).map_or(&[][..], |v| &v[..]) {
                let t = &roads[ri as usize].p;
                let m: [f32; 3] = std::array::from_fn(|i| (t[0][i] + t[1][i] + t[2][i]) / 3.0);
                let d = ((m[0] - p[0]).powi(2) + (m[2] - p[2]).powi(2)).sqrt();
                if !(12.0..25.0).contains(&d) || (m[1] - p[1]).abs() > 4.0 || !collision.near(m[0], m[2], m[1], DRIVEN)
                {
                    continue;
                }
                let score = (d - 18.0).abs();
                if best.is_none_or(|b| score < b.1) {
                    best = Some((m, score));
                }
            }
        }
        let camera = best.map_or(String::from("-"), |(m, _)| {
            format!("--camera={:.1},{:.1},{:.1},{:.1},{:.1},{:.1}", m[0], m[1] + 2.0, m[2], p[0], p[1] + 0.5, p[2])
        });
        println!(
            "{:6.0} m² {:6.0} {:5.2} m {:5.0} m at {:.1},{:.1},{:.1} zone {} {:?} unit {} model {} {} {camera}",
            c.samples as f32 * cell_area,
            c.driven as f32 * cell_area,
            c.max_dh,
            c.near,
            c.at[0],
            c.at[1],
            c.at[2],
            zone.map_or("-".into(), |z| z.to_string()),
            s.class,
            s.unit,
            s.model,
            s.shader
        );
    }
    Ok(())
}

/// `--masks`: the `Model` +0x19 pass mask over every main placement (instances, props, compounds, dynamics), per
/// mask and the family of the LOD0 renderable's first visible shader, with placements, models, and whether
/// the model has a SHADOW-state renderable of its own.
pub fn masks(store: &AssetStore) -> Result<()> {
    let units: Vec<String> = store
        .bundle_paths()
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    // (layer, mask, family) -> (placements, models, models with a SHADOW renderable)
    let mut table: std::collections::BTreeMap<
        (&str, u8, String),
        (usize, std::collections::HashSet<ResourceId>, usize),
    > = Default::default();
    let mut family: HashMap<ResourceId, Option<(u8, String, bool)>> = HashMap::new();
    let mut at: HashMap<ResourceId, Vec<[f32; 3]>> = HashMap::new();
    for name in &units {
        let Some(b) = store.bundle(name) else { continue };
        let Ok(layers) = store.bundle_layers(&b) else { continue };
        let lists: [(&str, &Vec<mw_assets::Placement>); 4] = [
            ("instance", &layers.instances),
            ("prop", &layers.props),
            ("compound", &layers.compounds),
            ("dynamic", &layers.dynamics),
        ];
        for (layer, list) in lists {
            for p in list.iter().filter(|p| !p.backdrop) {
                let f = family.entry(p.model).or_insert_with(|| {
                    let m = store.model(p.model).ok()?;
                    let shader = store
                        .model_meshes(p.model, 0)
                        .ok()
                        .and_then(|(ms, _)| {
                            ms.iter().map(|s| s.shader_name.clone()).find(|s| !mw_assets::is_hidden_shader(s))
                        })
                        .unwrap_or_default();
                    let fam = shader.split('_').next().unwrap_or("").to_string();
                    let own_shadow = m
                        .renderable_for_state(fmt::Model::STATE_SHADOW)
                        .is_some_and(|r| Some(r) != m.renderable_for_state(0));
                    Some((m.pass_mask, fam, own_shadow))
                });
                let Some((mask, fam, own_shadow)) = f.clone() else { continue };
                if mask & 0x0f == 0x0f {
                    at.entry(p.model).or_default().push([p.transform[3][0], p.transform[3][1], p.transform[3][2]]);
                }
                let e = table.entry((layer, mask, fam)).or_default();
                e.0 += 1;
                if e.1.insert(p.model) && own_shadow {
                    e.2 += 1;
                }
            }
        }
        store.evict_all();
    }
    // Models no view draws (all five bits) or only the shadow / far views: their meshes.
    let mut odd: Vec<_> =
        family.iter().filter_map(|(m, f)| f.as_ref().filter(|f| f.0 & 0x0f == 0x0f).map(|f| (*m, f.0))).collect();
    odd.sort();
    for (m, mask) in odd {
        let Ok((meshes, _)) = store.model_meshes(m, 0) else { continue };
        let places = at.get(&m).map_or(&[][..], |v| &v[..]);
        let first: Vec<String> =
            places.iter().take(3).map(|p| format!("({:.0}, {:.0}, {:.0})", p[0], p[1], p[2])).collect();
        println!("model {m}: {} placements, e.g. {}", places.len(), first.join(" "));
        for sm in &meshes {
            let p = &sm.mesh.positions;
            let lo: [f32; 3] = std::array::from_fn(|k| p.iter().map(|v| v[k]).fold(f32::MAX, f32::min));
            let hi: [f32; 3] = std::array::from_fn(|k| p.iter().map(|v| v[k]).fold(f32::MIN, f32::max));
            println!(
                "model {m} mask 0x{mask:02x}: {} verts {} tris, extent {:.2} x {:.2} x {:.2} m, {} diffuse {:?}",
                p.len(),
                sm.mesh.indices.len() / 3,
                hi[0] - lo[0],
                hi[1] - lo[1],
                hi[2] - lo[2],
                sm.shader_name,
                sm.diffuse
            );
        }
    }
    println!("layer     mask  family                       placements  models  with SHADOW renderable");
    for ((layer, mask, fam), (n, models, shadow)) in &table {
        println!("{layer:9} 0x{mask:02x}  {fam:28} {n:10}  {:6}  {shadow}", models.len());
    }
    Ok(())
}

/// `--model <id>`: a model's pass mask, LOD and shadow distances, and per state (LOD0..5, SHADOW, OCCLUSION) the
/// renderable it maps to with its meshes' shaders, triangle counts and bounds.
pub fn model(store: &AssetStore, id: ResourceId) -> Result<()> {
    let m = store.model(id)?;
    println!(
        "model {id}: pass mask 0x{:02x}, LOD distances {:?}, shadow distance {:.0} m, states {:?}",
        m.pass_mask, m.lod_distances, m.shadow_distance, m.state_renderables
    );
    for state in 0..m.state_renderables.len() {
        let Some(r) = m.renderable_for_state(state) else { continue };
        let Ok((meshes, _)) = store.renderable_meshes(r) else { continue };
        for sm in &meshes {
            let p = &sm.mesh.positions;
            let lo: [f32; 3] = std::array::from_fn(|k| p.iter().map(|v| v[k]).fold(f32::MAX, f32::min));
            let hi: [f32; 3] = std::array::from_fn(|k| p.iter().map(|v| v[k]).fold(f32::MIN, f32::max));
            println!(
                "  state {state} renderable {r}: {} tris, {} [{:.1} {:.1} {:.1}]..[{:.1} {:.1} {:.1}]",
                sm.mesh.indices.len() / 3,
                sm.shader_name,
                lo[0],
                lo[1],
                lo[2],
                hi[0],
                hi[1],
                hi[2]
            );
        }
    }
    Ok(())
}
