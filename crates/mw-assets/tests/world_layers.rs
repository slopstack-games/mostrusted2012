//! Whole-install checks for the world layers beyond `InstanceList`. Skipped
//! when no game install is found.

use mw_assets::AssetStore;
use mw_bundle::ResourceType;
use mw_formats as fmt;
use mw_gamedir::GameDir;

fn store() -> Option<AssetStore> {
    match GameDir::locate(None) {
        Ok(game) => Some(AssetStore::new(game).expect("index the install")),
        Err(e) => {
            eprintln!("skipped: no game install ({e})");
            None
        }
    }
}

/// Every world-layer list and object definition in the install parses, every
/// listed object resolves to a definition, and its model exists.
#[test]
fn all_world_layers_parse_and_resolve() {
    let Some(store) = store() else { return };
    type ParseDef = fn(&mw_bundle::Resource) -> fmt::Result<fmt::ObjectDef>;
    let kinds: [(ResourceType, ResourceType, ParseDef); 3] = [
        (ResourceType::PropInstanceList, ResourceType::PropObject, fmt::ObjectDef::parse_prop),
        (ResourceType::CompoundInstanceList, ResourceType::CompoundObject, fmt::ObjectDef::parse_compound),
        (ResourceType::DynamicInstanceList, ResourceType::WorldObject, fmt::ObjectDef::parse_world_object),
    ];
    let (mut lists, mut instances, mut defs, mut lights) = (0, 0, 0, 0);
    let mut failures = Vec::new();
    for rel in store.bundle_paths().iter().map(|p| p.display().to_string()).collect::<Vec<_>>() {
        let Some(b) = store.bundle(&rel) else { continue };
        for (list_ty, def_ty, parse_def) in kinds {
            for e in b.entries_of_type(def_ty) {
                defs += 1;
                match parse_def(&b.load(e).unwrap()) {
                    Ok(d) if d.model().is_some() => {}
                    Ok(d) => failures.push(format!("{rel}: {def_ty:?} {} has no model", d.id)),
                    Err(err) => failures.push(format!("{rel}: {def_ty:?} {}: {err}", e.id)),
                }
            }
            for e in b.entries_of_type(list_ty) {
                lists += 1;
                let list = match fmt::ObjectInstanceList::parse(&b.load(e).unwrap()) {
                    Ok(l) => l,
                    Err(err) => {
                        failures.push(format!("{rel}: {list_ty:?} {}: {err}", e.id));
                        continue;
                    }
                };
                for inst in &list.instances {
                    instances += 1;
                    let ok = inst.object.is_some_and(|o| {
                        store.load(o).ok().and_then(|r| parse_def(&r).ok()).and_then(|d| d.model()).is_some()
                    });
                    if !ok {
                        failures.push(format!("{rel}: {list_ty:?} {}: unresolved {:?}", e.id, inst.object));
                    }
                    if !inst.transform.iter().flatten().all(|v| v.is_finite()) {
                        failures.push(format!("{rel}: {list_ty:?} {}: non-finite transform", e.id));
                    }
                }
            }
        }
        for e in b.entries_of_type(ResourceType::LightInstanceList) {
            match fmt::ObjectInstanceList::parse(&b.load(e).unwrap()) {
                Ok(l) => lights += l.instances.len(),
                Err(err) => failures.push(format!("{rel}: LightInstanceList {}: {err}", e.id)),
            }
        }
        store.evict_all();
    }
    eprintln!("{lists} lists, {instances} instances, {defs} object definitions, {lights} lights");
    for f in failures.iter().take(20) {
        eprintln!("{f}");
    }
    assert!(failures.is_empty(), "{} failures", failures.len());
    assert!(instances > 0 && defs > 0);
}

/// Every groundcover collection parses, its groups add up to the layer totals
/// and its blobs resolve and decode.
#[test]
fn all_groundcover_parses() {
    let Some(store) = store() else { return };
    let mut seen = std::collections::HashSet::new();
    let (mut collections, mut instances, mut layers_used) = (0, 0usize, 0);
    let mut failures = Vec::new();
    for rel in store.bundle_paths().iter().map(|p| p.display().to_string()).collect::<Vec<_>>() {
        let Some(b) = store.bundle(&rel) else { continue };
        for e in b.entries_of_type(ResourceType::GroundcoverCollection) {
            if !seen.insert(e.id) {
                continue;
            }
            collections += 1;
            let col = match fmt::GroundcoverCollection::parse(&b.load(e).unwrap()) {
                Ok(c) => c,
                Err(err) => {
                    failures.push(format!("{rel}: collection {}: {err}", e.id));
                    continue;
                }
            };
            for (l, layer) in col.layers.iter().enumerate() {
                let sum: u32 = layer.groups.iter().map(|g| g.count).sum();
                if sum != layer.total {
                    failures.push(format!("{}: layer {l} groups sum {sum} != total {}", e.id, layer.total));
                }
                if layer.groups.is_empty() {
                    continue;
                }
                layers_used += 1;
                let blobs: Option<Vec<_>> = col
                    .blobs
                    .iter()
                    .map(|&id| store.load(id).ok().and_then(|r| fmt::GroundcoverInstances::parse(&r).ok()))
                    .collect();
                match blobs.map(|b| col.instances(l, &b)) {
                    Some(Ok(v)) => {
                        instances += v.len();
                        if v.iter().any(|i| !i.position.iter().all(|p| p.is_finite())) {
                            failures.push(format!("{}: layer {l} non-finite position", e.id));
                        }
                    }
                    Some(Err(err)) => failures.push(format!("{}: layer {l}: {err}", e.id)),
                    None => failures.push(format!("{}: layer {l}: blob missing or unparsable", e.id)),
                }
                if layer.groups.iter().any(|g| col.materials.get(g.material).and_then(|m| m.material).is_none()) {
                    failures.push(format!("{}: layer {l} has a group without material", e.id));
                }
            }
        }
        store.evict_all();
    }
    eprintln!("{collections} collections, {layers_used} layers, {instances} instances");
    for f in failures.iter().take(20) {
        eprintln!("{f}");
    }
    assert!(failures.is_empty(), "{} failures", failures.len());
    assert!(instances > 0);
}

#[test]
#[ignore]
fn diag_gc_ground() {
    let Some(store) = store() else { return };
    let b = store.bundle("HAWAII/TRK_UNIT1.BNDL").unwrap();
    let mut tris: Vec<[[f32; 3]; 3]> = Vec::new();
    for rel in ["HAWAII/TRK_UNIT1.BNDL", "HAWAII/TRK_UNIT50.BNDL", "HAWAII/TRK_UNIT10.BNDL"] {
        let bb = store.bundle(rel).unwrap();
        for e in bb.entries_of_type(ResourceType::PolygonSoupList) {
            let list = fmt::PolygonSoupList::parse(&bb.load(e).unwrap()).unwrap();
            let (v, t) = list.triangles();
            tris.extend(t.iter().map(|t| [v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]]));
        }
    }
    eprintln!("{} triangles", tris.len());
    let mut dys = Vec::new();
    let mut good = Vec::new();
    for e in b.entries_of_type(ResourceType::GroundcoverCollection) {
        let g = store.groundcover(e.id).unwrap();
        for i in g.instances.iter().step_by(7) {
            let p = i.position;
            if !(-1220.0..-980.0).contains(&p[0]) || !(1520.0..2000.0).contains(&p[2]) {
                continue;
            }
            let mut best: Option<f32> = None;
            for t in &tris {
                let d = |a: [f32; 3], b: [f32; 3]| (b[0] - a[0]) * (p[2] - a[2]) - (b[2] - a[2]) * (p[0] - a[0]);
                let (d0, d1, d2) = (d(t[0], t[1]), d(t[1], t[2]), d(t[2], t[0]));
                if (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0) || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0) {
                    let y = (t[0][1] + t[1][1] + t[2][1]) / 3.0;
                    if best.is_none_or(|b| (y - p[1]).abs() < (b - p[1]).abs()) {
                        best = Some(y);
                    }
                }
            }
            if let Some(y) = best {
                dys.push(p[1] - y);
                if (p[1] - y).abs() < 0.05 {
                    good.push((p, i.size, i.quads));
                }
            }
        }
    }
    dys.sort_by(|a, b| a.total_cmp(b));
    let n = dys.len();
    eprintln!(
        "{n} instances over collision; dy percentiles 5/25/50/75/95: {:?}",
        [n / 20, n / 4, n / 2, n * 3 / 4, n * 19 / 20].map(|i| dys[i])
    );
    let mut scored: Vec<_> = good
        .iter()
        .map(|(p, sz, q)| {
            let n = good.iter().filter(|(o, _, _)| (o[0] - p[0]).abs() < 8.0 && (o[2] - p[2]).abs() < 8.0).count();
            (n, *p, *sz, *q)
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    for s in scored.iter().step_by(40).take(8) {
        eprintln!("cluster {} at {:.1?} size {:?} quads {}", s.0, s.1, s.2, s.3);
    }
}
