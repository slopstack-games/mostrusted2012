//! `mwtool terrain-check`: finds drawn ground (terrain / plot meshes of the track units, their lower LODs,
//! and the composed backdrop) that stands above a road mesh at the same spot, i.e. terrain that would
//! show through the road.
//!
//! Roads are the `DriveableSurface*` meshes of the main (non-backdrop) instances. Ground is every
//! `TerrainPBR*` / `PlotPBR*` mesh of the main instances at LOD0 and at each lower LOD the model has, plus
//! every solid backdrop mesh after the zone composition (`mw_assets::backdrop`, all units loaded). Both are
//! rasterised on a grid; at a grid point a ground surface "pokes" when the highest road surface within
//! ±3 m of it lies more than `--eps` below it.

use std::collections::HashMap;

use anyhow::Result;
use mw_assets::AssetStore;
use mw_assets::backdrop::track_unit_number;
use mw_bundle::ResourceId;

const TILE: f32 = 64.0;
const BAND: f32 = 3.0;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
enum Class {
    /// Main instance terrain / plot mesh at this LOD.
    Main(u8),
    /// Composed backdrop.
    Backdrop,
}

struct Tri {
    p: [[f32; 3]; 3],
    /// Index into `sources`.
    src: u32,
}

struct Source {
    class: Class,
    unit: u32,
    model: ResourceId,
    shader: String,
}

fn world(m: &[[f32; 4]; 4], q: [f32; 4]) -> [f32; 3] {
    std::array::from_fn(|k| q[0] * m[0][k] + q[1] * m[1][k] + q[2] * m[2][k] + m[3][k])
}

fn push_mesh(out: &mut Vec<Tri>, mesh: &mw_formats::DecodedMesh, m: &[[f32; 4]; 4], src: u32) {
    let pos: Vec<[f32; 3]> = mesh.positions.iter().map(|&q| world(m, q)).collect();
    for t in mesh.indices.as_chunks::<3>().0 {
        let (Some(&a), Some(&b), Some(&c)) = (pos.get(t[0] as usize), pos.get(t[1] as usize), pos.get(t[2] as usize))
        else {
            continue;
        };
        out.push(Tri { p: [a, b, c], src });
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

/// One reported spot (an 8 m cell).
#[derive(Default)]
struct Spot {
    samples: usize,
    max_dh: f32,
    at: [f32; 3],
    src: u32,
}

pub fn run(store: &AssetStore, step: f32, eps: f32, top: usize, only: Option<&str>, all_passes: bool) -> Result<()> {
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
    let mut roads: Vec<Tri> = Vec::new();
    let mut ground: Vec<Tri> = Vec::new();
    let mut listings: Vec<(u32, ResourceId, [[f32; 4]; 4])> = Vec::new();
    let mut lod_models: HashMap<ResourceId, Vec<(usize, Vec<mw_assets::SceneMesh>)>> = HashMap::new();
    for name in &units {
        let Some(b) = store.bundle(name) else { continue };
        let unit = track_unit_number(name).unwrap_or(0);
        let Ok(layers) = store.bundle_layers(&b) else { continue };
        for p in &layers.instances {
            if p.backdrop {
                listings.push((unit, p.model, p.transform));
                continue;
            }
            // The main view skips models whose pass mask has its bit (`Model::in_main_view`).
            if !all_passes && store.model(p.model).is_ok_and(|m| !m.in_main_view()) {
                continue;
            }
            let Ok((meshes, _)) = store.model_meshes(p.model, 0) else { continue };
            for sm in &meshes {
                let s = sm.shader_name.as_str();
                if mw_assets::is_hidden_shader(s) {
                    continue;
                }
                if s.starts_with("DriveableSurface") {
                    let src = source(Class::Main(0), unit, p.model, s);
                    push_mesh(&mut roads, &sm.mesh, &p.transform, src);
                } else if s.starts_with("TerrainPBR") || s.starts_with("PlotPBR") {
                    let src = source(Class::Main(0), unit, p.model, s);
                    push_mesh(&mut ground, &sm.mesh, &p.transform, src);
                }
            }
            // Lower LODs of ground models (their own renderables).
            if !meshes
                .iter()
                .any(|sm| sm.shader_name.starts_with("TerrainPBR") || sm.shader_name.starts_with("PlotPBR"))
            {
                continue;
            }
            let lods = lod_models.entry(p.model).or_insert_with(|| {
                let Ok(model) = store.model(p.model) else { return Vec::new() };
                let lod0 = model.renderable_for_state(0);
                (1..model.lod_distances.len().min(4))
                    .filter_map(|l| {
                        let r = model.renderable_for_state(l)?;
                        if Some(r) == lod0 {
                            return None;
                        }
                        Some((l, store.renderable_meshes(r).ok()?.0))
                    })
                    .collect()
            });
            for (l, meshes) in lods.iter() {
                for sm in meshes {
                    let s = sm.shader_name.as_str();
                    if mw_assets::is_hidden_shader(s) || s.starts_with("DriveableSurface") {
                        continue;
                    }
                    let src = source(Class::Main(*l as u8), unit, p.model, s);
                    push_mesh(&mut ground, &sm.mesh, &p.transform, src);
                }
            }
        }
        store.evict_all();
    }
    eprintln!("{} road triangles, {} main ground triangles", roads.len(), ground.len());

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
            push_mesh(&mut ground, &kept, t, src);
        }
    }
    store.evict_all();
    eprintln!("{} ground triangles with the backdrop", ground.len());
    if let Some(only) = only {
        ground.retain(|t| {
            let s = &sources[t.src as usize];
            match only {
                "backdrop" => s.class == Class::Backdrop,
                "main" => s.class == Class::Main(0),
                "lod" => matches!(s.class, Class::Main(l) if l > 0),
                _ => true,
            }
        });
    }

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
    let mut area: HashMap<Class, usize> = HashMap::new();
    let mut road_samples = 0usize;
    let mut keys: Vec<_> = road_tiles.keys().copied().collect();
    keys.sort();
    for key in keys {
        let Some(gt) = ground_tiles.get(&key) else { continue };
        let ox = key.0 as f32 * TILE;
        let oz = key.1 as f32 * TILE;
        let mut cells: Vec<Vec<(f32, ResourceId)>> = vec![Vec::new(); n * n];
        let range = |lo: f32, hi: f32, o: f32| {
            let a = (((lo - o) / step - 0.5).ceil()).max(0.0) as usize;
            let b = (((hi - o) / step - 0.5).floor()).min(n as f32 - 1.0);
            if b < 0.0 { (1, 0) } else { (a, b as usize) }
        };
        for &ri in &road_tiles[&key] {
            let t = &roads[ri as usize].p;
            let b = bounds(t);
            let (i0, i1) = range(b[0], b[2], ox);
            let (j0, j1) = range(b[1], b[3], oz);
            for j in j0..=j1.min(n - 1) {
                for i in i0..=i1.min(n - 1) {
                    let (x, z) = (ox + (i as f32 + 0.5) * step, oz + (j as f32 + 0.5) * step);
                    if let Some(y) = height_at(t, x, z) {
                        cells[j * n + i].push((y, sources[roads[ri as usize].src as usize].model));
                    }
                }
            }
        }
        road_samples += cells.iter().filter(|c| !c.is_empty()).count();
        for &gi in gt {
            let g = &ground[gi as usize];
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
                    // A lower LOD of a model replaces the model's own road too: compare it with other roads.
                    let gs = &sources[g.src as usize];
                    let top = ys
                        .iter()
                        .filter(|(y, m)| (y - h).abs() <= BAND && (gs.class == Class::Main(0) || *m != gs.model))
                        .map(|(y, _)| *y)
                        .fold(f32::MIN, f32::max);
                    if top == f32::MIN || h - top <= eps {
                        continue;
                    }
                    let dh = h - top;
                    let class = gs.class;
                    *area.entry(class).or_default() += 1;
                    let spot = spots.entry(((x / 8.0).floor() as i32, (z / 8.0).floor() as i32, g.src)).or_default();
                    spot.samples += 1;
                    if dh > spot.max_dh {
                        spot.max_dh = dh;
                        spot.at = [x, top, z];
                        spot.src = g.src;
                    }
                }
            }
        }
    }
    let cell_area = step * step;
    println!("road area sampled: {:.0} m² (grid {step} m, eps {eps} m)", road_samples as f32 * cell_area);
    let mut classes: Vec<_> = area.into_iter().collect();
    classes.sort();
    for (c, k) in &classes {
        println!("  ground above road: {c:?}: {:.0} m²", *k as f32 * cell_area);
    }
    // Merge 8 m cells into clusters per source (flood over neighbouring cells).
    let mut by_src: HashMap<u32, Vec<((i32, i32), &Spot)>> = HashMap::new();
    for ((cx, cz, src), s) in &spots {
        by_src.entry(*src).or_default().push(((*cx, *cz), s));
    }
    struct Cluster {
        src: u32,
        samples: usize,
        max_dh: f32,
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
            let mut c = Cluster { src, samples: 0, max_dh: 0.0, at: [0.0; 3] };
            let mut stack = vec![start];
            while let Some(k) = stack.pop() {
                let s = map[&k];
                c.samples += s.samples;
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
    println!("top {top} clusters (area m², max height above the road, where, zone, source):");
    for c in clusters.iter().take(top) {
        let s = &sources[c.src as usize];
        let zone = zones.as_ref().and_then(|z| z.zone_at(c.at[0], c.at[2])).map(|z| z.id);
        println!(
            "{:7.0} m² {:6.2} m at {:.1},{:.1},{:.1} zone {:?} {:?} unit {} model {} {}",
            c.samples as f32 * cell_area,
            c.max_dh,
            c.at[0],
            c.at[1],
            c.at[2],
            zone,
            s.class,
            s.unit,
            s.model,
            s.shader
        );
    }
    Ok(())
}
