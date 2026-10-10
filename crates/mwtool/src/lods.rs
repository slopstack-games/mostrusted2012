//! `mwtool lods`: the world's model LOD tables and placement LOD scales over the track units (the game's
//! rule: docs/decomp.md "Model LOD and draw distance").
//!
//! Per layer: the placements' LOD scale values. Per model placed in the world: LOD count, the last LOD
//! distance (= its draw distance at scale 1), which LOD states point at a renderable of their own, and the
//! shadow distance. `--at x,y,z` adds what the game would draw from that camera position at a given view
//! scale (`--scale`, default 1.5: the "LodScaling" setting 1 at a 73.7° horizontal FOV).

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use mw_assets::AssetStore;
use mw_bundle::ResourceId;
use mw_formats as fmt;

struct ModelInfo {
    model: fmt::Model,
    /// The LOD0 renderable's bounding sphere (object space).
    sphere: Option<[f32; 4]>,
}

fn bucket(d: f32) -> String {
    if !d.is_finite() || d > 1e6 {
        return "inf".into();
    }
    let edges = [10.0, 25.0, 50.0, 75.0, 100.0, 150.0, 200.0, 300.0, 400.0, 600.0, 800.0, 1000.0, 2000.0, 5000.0];
    match edges.iter().find(|&&e| d <= e) {
        Some(e) => format!("<= {e:>5}"),
        None => "> 5000".into(),
    }
}

pub fn run(store: &AssetStore, at: Option<[f32; 3]>, scale: f32) -> Result<()> {
    let units: Vec<String> = store
        .bundle_paths()
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let mut models: HashMap<ResourceId, Option<ModelInfo>> = HashMap::new();
    let mut scales: BTreeMap<(&str, String), usize> = BTreeMap::new();
    // layer -> (drawn per LOD, culled, no table)
    let mut seen: BTreeMap<&str, ([usize; 4], usize, usize)> = BTreeMap::new();
    let mut uses: HashMap<ResourceId, usize> = HashMap::new();
    for name in &units {
        let Some(b) = store.bundle(name) else { continue };
        let layers = store.bundle_layers(&b)?;
        let groups: [(&str, &Vec<mw_assets::Placement>); 4] = [
            ("instance", &layers.instances),
            ("prop", &layers.props),
            ("compound", &layers.compounds),
            ("dynamic", &layers.dynamics),
        ];
        for (layer, list) in groups {
            for p in list {
                let layer = if p.backdrop { "backdrop" } else { layer };
                *scales.entry((layer, format!("{:.3}", p.lod_scale))).or_default() += 1;
                *uses.entry(p.model).or_default() += 1;
                let info = models.entry(p.model).or_insert_with(|| {
                    let model = store.model(p.model).ok()?;
                    let sphere =
                        model.renderable_for_state(0).and_then(|r| store.renderable(r).ok()).map(|r| r.bounding_sphere);
                    Some(ModelInfo { model, sphere })
                });
                let (Some(eye), Some(info)) = (at, info.as_ref()) else { continue };
                let entry = seen.entry(layer).or_default();
                let Some([x, y, z, r]) = info.sphere else {
                    entry.2 += 1;
                    continue;
                };
                let m = &p.transform;
                let c: [f32; 3] = std::array::from_fn(|k| x * m[0][k] + y * m[1][k] + z * m[2][k] + m[3][k]);
                let dist = ((c[0] - eye[0]).powi(2) + (c[1] - eye[1]).powi(2) + (c[2] - eye[2]).powi(2)).sqrt();
                let d = (dist - r).max(0.0) / (p.lod_scale * scale);
                match info.model.lod_for_distance(d) {
                    Some(l) => entry.0[l.min(3)] += 1,
                    None if info.model.lod_distances.is_empty() => entry.2 += 1,
                    None => entry.1 += 1,
                }
            }
        }
        store.evict_all();
    }

    println!("placement LOD scales (layer, value: placements)");
    for ((layer, v), n) in &scales {
        println!("  {layer:<9} {v:>8}: {n}");
    }

    let mut counts: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    let mut last: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut shadow: BTreeMap<String, usize> = BTreeMap::new();
    let mut own = [0usize; 6];
    let mut missing_state = 0;
    let mut unordered = 0;
    let mut distances: BTreeMap<String, usize> = BTreeMap::new();
    let mut no_model = 0;
    let mut shadow_state = (0, 0);
    for (id, info) in &models {
        let Some(info) = info else {
            no_model += 1;
            continue;
        };
        let m = &info.model;
        let n = m.lod_distances.len();
        if let Some(r) = m.renderable_for_state(fmt::Model::STATE_SHADOW) {
            shadow_state.0 += 1;
            shadow_state.1 += (Some(r) != m.renderable_for_state(0)) as usize;
        }
        let e = counts.entry(n).or_default();
        e.0 += 1;
        e.1 += uses[id];
        if let Some(&l) = m.lod_distances.last() {
            let e = last.entry(bucket(l)).or_default();
            e.0 += 1;
            e.1 += uses[id];
        }
        *shadow.entry(bucket(m.shadow_distance)).or_default() += 1;
        *distances
            .entry(m.lod_distances.iter().map(|d| format!("{d:.0}")).collect::<Vec<_>>().join("/"))
            .or_default() += 1;
        if m.lod_distances.windows(2).any(|w| w[1] < w[0]) {
            unordered += 1;
        }
        for (s, slot) in own.iter_mut().enumerate().take(n.min(6)) {
            match m.renderable_for_state(s) {
                Some(r) if s == 0 || Some(r) != m.renderable_for_state(s - 1) => *slot += 1,
                Some(_) => {}
                None => missing_state += 1,
            }
        }
    }
    println!("\nmodels placed in the world: {} ({no_model} unreadable)", models.len());
    println!("LOD count: models / placements");
    for (n, (m, p)) in &counts {
        println!("  {n}: {m} / {p}");
    }
    println!("last LOD distance (draw distance at scale 1): models / placements");
    for (b, (m, p)) in &last {
        println!("  {b}: {m} / {p}");
    }
    println!("shadow distance: models");
    for (b, m) in &shadow {
        println!("  {b}: {m}");
    }
    println!("LOD states with a renderable different from the previous state's: {own:?}");
    println!("models with a SHADOW state (6) renderable: {} ({} of them not LOD0's)", shadow_state.0, shadow_state.1);
    println!("LOD states (< LOD count) without a renderable: {missing_state}; tables not ascending: {unordered}");
    let mut common: Vec<_> = distances.into_iter().collect();
    common.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!("most common LOD tables (metres): {:?}", &common[..common.len().min(20)]);
    if let Some(eye) = at {
        println!("\nfrom {eye:?} at view scale {scale}: layer: drawn at LOD0..3, culled, without a LOD table");
        for (layer, (lods, culled, none)) in &seen {
            println!("  {layer:<9} {lods:?} culled {culled}, no table {none}");
        }
    }
    Ok(())
}
