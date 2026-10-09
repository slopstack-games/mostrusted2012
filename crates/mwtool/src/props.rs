//! `mwtool props`: physics of every prop / compound / dynamic object definition placed in the world.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use mw_assets::AssetStore;
use mw_assets::props::{ObjectKind, ObjectPhysics, Volume};
use mw_bundle::ResourceId;

pub fn run(store: &AssetStore, bundles: &[String], verbose: bool) -> Result<()> {
    let names: Vec<String> = if bundles.is_empty() {
        store
            .bundle_paths()
            .iter()
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")))
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    } else {
        bundles.to_vec()
    };
    // Placements per object definition, and a debug name for its model where a bundle has one.
    let mut uses: HashMap<(ResourceId, ObjectKind), (usize, ResourceId)> = HashMap::new();
    let mut model_names: HashMap<ResourceId, String> = HashMap::new();
    for name in &names {
        let Some(b) = store.bundle(name) else { continue };
        let layers = store.bundle_layers(&b)?;
        for p in layers.props.iter().chain(&layers.compounds).chain(&layers.dynamics) {
            let Some(obj) = p.object else { continue };
            uses.entry(obj).or_insert((0, p.model)).0 += 1;
            if let Some(n) = b.debug_name(p.model) {
                model_names.entry(p.model).or_insert_with(|| n.to_string());
            }
        }
        store.evict_all();
    }
    let mut objects: Vec<_> = uses.into_iter().collect();
    objects.sort_by_key(|((id, kind), _)| (format!("{kind:?}"), *id));

    // Aggregates: per kind, placements with / without physics, volume kinds, mass and threshold histograms.
    #[derive(Default)]
    struct Stats {
        defs: usize,
        placements: usize,
        no_physics: usize,
        volumes: BTreeMap<&'static str, usize>,
        masses: BTreeMap<u32, usize>,
        bodies: BTreeMap<usize, usize>,
        force_to_damage: BTreeMap<u32, usize>,
        force_67f0233c: BTreeMap<u32, usize>,
        force_to_smash: BTreeMap<u32, usize>,
        parts: BTreeMap<usize, usize>,
        errors: usize,
    }
    let mut stats: BTreeMap<String, Stats> = BTreeMap::new();
    for ((id, kind), (count, model)) in &objects {
        let s = stats.entry(format!("{kind:?}")).or_default();
        s.defs += 1;
        s.placements += count;
        let phys: ObjectPhysics = match store.object_physics(*id, *kind) {
            Ok(p) => p,
            Err(e) => {
                s.errors += 1;
                if verbose {
                    println!("{kind:?} {id}: {e}");
                }
                continue;
            }
        };
        *s.parts.entry(phys.parts.len()).or_default() += 1;
        let Some(def) = &phys.definition else {
            s.no_physics += count;
            if verbose {
                println!("{kind:?} {id} x{count} model {model}: no physical definition");
            }
            continue;
        };
        *s.bodies.entry(def.bodies.len()).or_default() += 1;
        *s.masses.entry(phys.mass().round() as u32).or_default() += count;
        if let Some(info) = def.info {
            *s.force_to_damage.entry(info.force_to_damage as u32).or_default() += count;
            *s.force_67f0233c.entry(info.force_67f0233c as u32).or_default() += count;
            if let Some(f) = info.force_to_smash {
                *s.force_to_smash.entry(f as u32).or_default() += count;
            }
        }
        for v in def.bodies.iter().flat_map(|b| &b.volumes) {
            let k = match v {
                Volume::Box { .. } => "box",
                Volume::Sphere { .. } => "sphere",
                Volume::Cylinder { .. } => "cylinder",
                Volume::Capsule { .. } => "capsule",
                Volume::Hull { .. } => "hull",
            };
            *s.volumes.entry(k).or_default() += 1;
        }
        if verbose {
            let b = &def.bodies.first();
            let (c, h) = def.bounds;
            println!(
                "{kind:?} {id} x{count} {} mass {:.0} bounds c({:.2} {:.2} {:.2}) h({:.2} {:.2} {:.2}) vols {} friction {:.2} rest {:.2} drag {:.1}/{:.1} info {:?} parts {} reset {:?}",
                model_names.get(model).map_or_else(|| model.to_string(), |n| n.clone()),
                phys.mass(),
                c[0],
                c[1],
                c[2],
                h[0],
                h[1],
                h[2],
                def.bodies.iter().map(|b| b.volumes.len()).sum::<usize>(),
                b.map_or(0.0, |b| b.friction),
                b.map_or(0.0, |b| b.restitution),
                b.map_or(0.0, |b| b.linear_drag),
                b.map_or(0.0, |b| b.angular_drag),
                def.info,
                phys.parts.len(),
                phys.reset_distance,
            );
        }
    }
    for (kind, s) in &stats {
        println!(
            "== {kind}: {} definitions, {} placements ({} without physics, {} errors)",
            s.defs, s.placements, s.no_physics, s.errors
        );
        println!("   volumes {:?}", s.volumes);
        println!("   bodies per definition {:?}, parts per object {:?}", s.bodies, s.parts);
        println!("   mass kg (placements) {:?}", s.masses);
        println!("   ForceToDamage {:?}", s.force_to_damage);
        println!("   67f0233c {:?}", s.force_67f0233c);
        if !s.force_to_smash.is_empty() {
            println!("   ForceToSmash {:?}", s.force_to_smash);
        }
    }
    Ok(())
}

/// Per surface, the triangles whose centre lies within `r` (horizontally) of (`x`, `z`): count, the nearest
/// one's centre and its normal's up component.
fn surfaces_near(
    store: &AssetStore,
    list: &[mw_assets::surfaces::SurfaceInfo],
    [x, z, r]: [f32; 3],
    find: Option<u8>,
) -> Result<()> {
    // With `find`: (cell x, cell z) -> (triangle count of that surface, area, height sum)
    let mut cells: BTreeMap<(i32, i32), (usize, f32, f32)> = BTreeMap::new();
    use mw_bundle::ResourceType;
    use mw_formats::CollisionTag;
    // surface -> (count, nearest distance, centre, normal y, flags)
    let mut near: BTreeMap<u8, (usize, f32, [f32; 3], f32, u16)> = BTreeMap::new();
    for p in store.bundle_paths().to_vec() {
        if !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")) {
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
                let centre = [0, 1, 2].map(|k| (a[k] + b[k] + c[k]) / 3.0);
                let d = ((centre[0] - x).powi(2) + (centre[2] - z).powi(2)).sqrt();
                if d > r {
                    continue;
                }
                let (u, v) = ([0, 1, 2].map(|k| b[k] - a[k]), [0, 1, 2].map(|k| c[k] - a[k]));
                let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
                let tag = CollisionTag::from_u32(tag);
                if find == Some(tag.surface) && n[1] / len > 0.9 {
                    let c = cells
                        .entry(((centre[0] / 20.0).floor() as i32, (centre[2] / 20.0).floor() as i32))
                        .or_default();
                    *c = (c.0 + 1, c.1 + len / 2.0, c.2 + centre[1]);
                }
                let e = near.entry(tag.surface).or_insert((0, f32::MAX, [0.0; 3], 0.0, 0));
                e.0 += 1;
                if d < e.1 {
                    *e = (e.0, d, centre, (n[1] / len).abs(), tag.flags);
                }
            }
        }
        store.evict_all();
    }
    if find.is_some() {
        let mut v: Vec<_> = cells.into_iter().collect();
        v.sort_by(|a, b| b.1.1.total_cmp(&a.1.1));
        println!("cell centre (x y z)          triangles  flat area m²");
        for ((cx, cz), (n, area, ys)) in v.into_iter().take(15) {
            println!(
                "({:>7.0} {:>6.1} {:>7.0})  {n:>9}  {area:>8.0}",
                cx as f32 * 20.0 + 10.0,
                ys / n as f32,
                cz as f32 * 20.0 + 10.0
            );
        }
        return Ok(());
    }
    println!("surface  grip   triangles  nearest (x y z)            dist  up    flags");
    for (s, (n, d, c, up, flags)) in near {
        let grip = list.get(s as usize).and_then(|i| i.grip).map_or("-".into(), |g| format!("{g:.3}"));
        println!(
            "{s:>7}  {grip:>5}  {n:>9}  ({:>8.1} {:>6.1} {:>8.1})  {d:>5.1}  {up:.2}  {flags:#05x}",
            c[0], c[1], c[2]
        );
    }
    Ok(())
}

pub fn surfaces(store: &AssetStore, near: Option<[f32; 3]>, find: Option<u8>) -> Result<()> {
    use mw_bundle::ResourceType;
    use mw_formats::CollisionTag;
    let list = store.surfaces()?;
    if let Some(at) = near {
        return surfaces_near(store, &list, at, find);
    }
    let mut tris: BTreeMap<u8, (usize, BTreeMap<u16, usize>)> = BTreeMap::new();
    for p in store.bundle_paths().to_vec() {
        if !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")) {
            continue;
        }
        let Some(b) = store.bundle(&p) else { continue };
        for e in b.entries_of_type(ResourceType::PolygonSoupList) {
            let soups = mw_formats::PolygonSoupList::parse(&b.load(e)?)?;
            for t in soups.triangle_tags().into_iter().map(CollisionTag::from_u32) {
                let s = tris.entry(t.surface).or_default();
                s.0 += 1;
                *s.1.entry(t.flags).or_default() += 1;
            }
        }
        store.evict_all();
    }
    println!(
        "index  id                colour (debug)         grip   rolling  drag   physics  speed  roughness      triangles  flags (count)"
    );
    for (i, s) in list.iter().enumerate() {
        let (n, flags) = tris.get(&(i as u8)).cloned().unwrap_or_default();
        println!(
            "{i:>5}  {}  ({:.2} {:.2} {:.2})   {:>5}  {:>7}  {:>5}  {:>7}  {:>5}  {:<13}  {n:>9}  {:?}",
            s.id,
            s.colour[0],
            s.colour[1],
            s.colour[2],
            s.grip.map_or("-".into(), |g| format!("{g:.3}")),
            s.rolling.map_or("-".into(), |g| format!("{g:.3}")),
            s.slide_drag.map_or("-".into(), |g| format!("{g:.2}")),
            s.physics_uid.map_or("-".into(), |g| g.to_string()),
            s.speed.map_or("-".into(), |g| g.to_string()),
            s.roughness.map_or("-".into(), |r| format!("{:.2} {:.2}", r[0], r[1])),
            flags
        );
    }
    Ok(())
}
