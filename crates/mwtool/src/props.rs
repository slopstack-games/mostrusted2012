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
    // Where each object definition is first placed (the sequence objects' lines name it).
    let mut first_at: HashMap<ResourceId, (String, [f32; 3])> = HashMap::new();
    for name in &names {
        let Some(b) = store.bundle(name) else { continue };
        let layers = store.bundle_layers(&b)?;
        for p in layers.props.iter().chain(&layers.compounds).chain(&layers.dynamics) {
            let Some(obj) = p.object else { continue };
            uses.entry(obj).or_insert((0, p.model)).0 += 1;
            first_at
                .entry(obj.0)
                .or_insert_with(|| (name.clone(), [p.transform[3][0], p.transform[3][1], p.transform[3][2]]));
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
                if let Ok(d) = store.object_def(*id, *kind) {
                    for q in &store.object_sequences(&d).sequences {
                        println!(
                            "   sequence behaviour {:?} {}: sequence {:?}, at start {}, on by contact {}, animated {}; first placed {:?}",
                            q.label,
                            q.object,
                            q.sequence,
                            q.at_start,
                            q.on_by_contact,
                            q.animated,
                            first_at.get(id)
                        );
                    }
                }
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
            if let Some(e) = phys.impact_effect {
                println!("   impact effect {e}");
            }
            print_vfx("   ", &phys.vfx);
            print_sounds("   ", &phys.sounds);
            if let Ok(d) = store.object_def(*id, *kind) {
                let seqs = store.object_sequences(&d);
                for q in &seqs.sequences {
                    println!(
                        "   sequence behaviour {:?} {} part {:?}: sequence {:?}, at start {}, on by contact {}, off by contact {}, animated {}; first placed {:?}",
                        q.label,
                        q.object,
                        q.part,
                        q.sequence,
                        q.at_start,
                        q.on_by_contact,
                        q.off_by_contact,
                        q.animated,
                        first_at.get(id)
                    );
                }
                for c in &seqs.collisions {
                    println!(
                        "   collision sequences {} part {:?}: vehicle hit {:?}, other {:?}",
                        c.object, c.part, c.vehicle_hit, c.other
                    );
                }
            }
            if let Some(c) = &phys.compound {
                println!("   compound: enabled at start {}, joints {:?}", c.enabled_at_start, c.joints);
                // The model's triangles per skeleton bone (a triangle belongs to the bone of its first vertex's
                // heaviest weight): each part draws the geometry skinned to its bone.
                let mut per_bone: BTreeMap<u16, usize> = BTreeMap::new();
                let mut unskinned = 0;
                if let Ok((meshes, _)) = store.model_meshes(*model, 0) {
                    for m in meshes.iter().filter(|m| !mw_assets::is_hidden_shader(&m.shader_name)) {
                        let (Some(j), Some(w)) = (&m.mesh.joints, &m.mesh.weights) else {
                            unskinned += m.mesh.indices.len() / 3;
                            continue;
                        };
                        for t in m.mesh.indices.as_chunks::<3>().0 {
                            let v = t[0] as usize;
                            let k = (0..4).max_by(|&a, &b| w[v][a].total_cmp(&w[v][b])).unwrap_or(0);
                            *per_bone.entry(j[v][k]).or_default() += 1;
                        }
                    }
                }
                println!("   model triangles per bone {per_bone:?}, unskinned {unskinned}");
                for (i, p) in c.parts.iter().enumerate() {
                    let d = p.definition.as_ref();
                    let info = d.and_then(|d| d.info);
                    println!(
                        "   part {i}: bone {} at ({:.2} {:.2} {:.2}) mass {:.0} vols {} replaces {:?} next {:?} replaced by {:?} \
                         links {:?} smash proxy {:?} pinned {:?} knock {:?} sound {}",
                        p.bone_index,
                        p.bone[3][0],
                        p.bone[3][1],
                        p.bone[3][2],
                        d.map_or(0.0, |d| d.bodies.iter().map(|b| b.mass).sum::<f32>()),
                        d.map_or(0, |d| d.bodies.iter().map(|b| b.volumes.len()).sum::<usize>()),
                        p.replaces,
                        p.next,
                        p.first_replacement,
                        p.links,
                        p.smash_force,
                        info.map(|i| i.starts_pinned),
                        info.map(|i| i.force_67f0233c),
                        p.impact_sound.is_some(),
                    );
                    print_vfx("      ", &p.vfx);
                    print_sounds("      ", &p.sounds);
                }
            }
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

/// The low steep faces (kerbs, steps) within `r` of (`x`, `z`): triangles steeper than 45° whose vertical extent
/// is under 0.5 m, nearest first (at most 40), with the height they span, their surface and horizontal facing.
#[allow(clippy::type_complexity)]
fn steps_near(store: &AssetStore, [x, z, r]: [f32; 3]) -> Result<()> {
    use mw_bundle::ResourceType;
    use mw_formats::CollisionTag;
    let mut found: Vec<(f32, [f32; 3], f32, f32, [f32; 3], u8)> = Vec::new();
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
                let n = n.map(|k| k / len);
                let (lo, hi) = (a[1].min(b[1]).min(c[1]), a[1].max(b[1]).max(c[1]));
                if n[1].abs() > std::f32::consts::FRAC_1_SQRT_2 || hi - lo > 0.5 || hi - lo < 0.01 {
                    continue;
                }
                found.push((d, centre, lo, hi, n, CollisionTag::from_u32(tag).surface));
            }
        }
        store.evict_all();
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    println!("dist  centre (x y z)              y from..to       height  surface  normal (x y z)");
    for (d, c, lo, hi, n, s) in found.into_iter().take(40) {
        println!(
            "{d:>5.1}  ({:>8.2} {:>6.2} {:>8.2})  {lo:>6.2}..{hi:>6.2}  {:>5.3}  {s:>7}  ({:>5.2} {:>5.2} {:>5.2})",
            c[0],
            c[1],
            c[2],
            hi - lo,
            n[0],
            n[1],
            n[2]
        );
    }
    Ok(())
}

/// Every collision triangle with a vertex within `r` (horizontally) of (`x`, `z`): track unit, surface, collision
/// flags, normal and vertices.
fn tris_near(store: &AssetStore, [x, z, r]: [f32; 3]) -> Result<()> {
    use mw_bundle::ResourceType;
    use mw_formats::CollisionTag;
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
                let v = t.map(|i| verts[i as usize]);
                if !v.iter().any(|p| (p[0] - x).hypot(p[2] - z) <= r) {
                    continue;
                }
                let (u, w) = ([0, 1, 2].map(|k| v[1][k] - v[0][k]), [0, 1, 2].map(|k| v[2][k] - v[0][k]));
                let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
                let tag = CollisionTag::from_u32(tag);
                println!(
                    "{} s{:<3} f{:#05x} n({:5.2} {:5.2} {:5.2}) {}",
                    p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()),
                    tag.surface,
                    tag.flags,
                    n[0] / len,
                    n[1] / len,
                    n[2] / len,
                    v.iter().map(|p| format!("({:.2} {:.3} {:.2})", p[0], p[1], p[2])).collect::<Vec<_>>().join(" ")
                );
            }
        }
        store.evict_all();
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

/// `mwtool surfaces --classes`: per surface the tag flags of its `SurfaceType`, its tyre-class overrides and its
/// dry wheel effects.
pub fn surface_classes(store: &AssetStore) -> Result<()> {
    let list = store.surfaces()?;
    let lib = store.vfx_library()?;
    let id = |o: Option<mw_bundle::ResourceId>| o.map_or("-".to_string(), |i| format!("{:06x}", i.0 & 0xff_ffff));
    println!(
        "index  physics  flags  tyre classes grip/rolling/drag: offroad(0) 08b66463(1) road(2) sport(3) track(4)    \
         kick-up / ai, smoke, marks (road)"
    );
    for (i, s) in list.iter().enumerate() {
        let classes: Vec<String> = s
            .overrides
            .iter()
            .map(|o| o.map_or("-".into(), |o| format!("{:.3}/{:.3}/{:.2}", o.grip, o.rolling, o.slide_drag)))
            .map(|t| format!("{t:<17}"))
            .collect();
        let fx = lib.surfaces.get(i).map(|f| &f.dry);
        println!(
            "{i:>5}  {:>7}  {:#05x}  {}  {} / {}, {}, {}",
            s.physics_uid.map_or("-".into(), |u| u.to_string()),
            s.tag_flags,
            classes.join(" "),
            id(fx.and_then(|f| f.wheel)),
            id(fx.and_then(|f| f.ai_wheel)),
            id(fx.and_then(|f| f.player_wheels.default_tyre)),
            id(fx.and_then(|f| f.tyre_marks[mw_assets::vfx::TYRE_MARKS_ROAD])),
        );
    }
    Ok(())
}

pub fn surfaces(store: &AssetStore, near: Option<[f32; 3]>, find: Option<u8>, steps: bool, tris: bool) -> Result<()> {
    use mw_bundle::ResourceType;
    use mw_formats::CollisionTag;
    let list = store.surfaces()?;
    if let Some(at) = near
        && steps
    {
        return steps_near(store, at);
    }
    if let Some(at) = near
        && tris
    {
        return tris_near(store, at);
    }
    if let Some(at) = near {
        return surfaces_near(store, &list, at, find);
    }
    let mut tris: BTreeMap<u8, (usize, BTreeMap<u16, usize>)> = BTreeMap::new();
    let mut ground: BTreeMap<u8, f64> = BTreeMap::new();
    let mut places: Vec<[f32; 3]> = Vec::new();
    for p in store.bundle_paths().to_vec() {
        if !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")) {
            continue;
        }
        let Some(b) = store.bundle(&p) else { continue };
        for e in b.entries_of_type(ResourceType::PolygonSoupList) {
            let soups = mw_formats::PolygonSoupList::parse(&b.load(e)?)?;
            if let Some(want) = find {
                let (verts, idx) = soups.triangles();
                for (tri, tag) in idx.iter().zip(soups.triangle_tags()) {
                    if CollisionTag::from_u32(tag).surface != want {
                        continue;
                    }
                    let [a, b, c] = tri.map(|i| verts[i as usize]);
                    let sub = |p: [f32; 3], q: [f32; 3]| [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
                    let (u, v) = (sub(b, a), sub(c, a));
                    let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    let centre = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0, (a[2] + b[2] + c[2]) / 3.0];
                    let far = |p: &[f32; 3]| {
                        let d = sub(*p, centre);
                        d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > 150.0 * 150.0
                    };
                    if len > 0.0 && (n[1] / len).abs() > 0.95 && places.iter().all(far) {
                        places.push(centre);
                    }
                }
            }
            let (verts, idx) = soups.triangles();
            for (tri, t) in idx.iter().zip(soups.triangle_tags().into_iter().map(CollisionTag::from_u32)) {
                let s = tris.entry(t.surface).or_default();
                s.0 += 1;
                *s.1.entry(t.flags).or_default() += 1;
                // Ground area: the triangles facing up (normal within ~45° of +Y).
                let [a, b, c] = tri.map(|i| verts[i as usize]);
                let (u, v) = ([0, 1, 2].map(|k| b[k] - a[k]), [0, 1, 2].map(|k| c[k] - a[k]));
                let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if len > 0.0 && n[1] / len > 0.7 {
                    *ground.entry(t.surface).or_default() += len as f64 / 2.0;
                }
            }
        }
        store.evict_all();
    }
    println!(
        "index  id                colour (debug)         grip   rolling  drag   physics  speed  roughness      triangles  ground m²  dirt/dust dry | wet  flags (count)"
    );
    let fx = |e: &Option<mw_assets::surfaces::VehicleEffects>| match e {
        Some(e) => {
            format!("{:+.3}/{} {:+.3}/{} {}-{}", e.dirt.0, e.dirt.1, e.dust.0, e.dust.1, e.speed_start, e.speed_full)
        }
        None => "-".into(),
    };
    for (i, s) in list.iter().enumerate() {
        let (n, flags) = tris.get(&(i as u8)).cloned().unwrap_or_default();
        println!(
            "{i:>5}  {}  ({:.2} {:.2} {:.2})   {:>5}  {:>7}  {:>5}  {:>7}  {:>5}  {:<13}  {n:>9}  {:>9.0}  {} | {}  {:?}",
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
            ground.get(&(i as u8)).copied().unwrap_or(0.0),
            fx(&s.vehicle_effects[0]),
            fx(&s.vehicle_effects[1]),
            flags
        );
    }
    if let Some(want) = find {
        println!("surface {want}: {} places", places.len());
        for p in places.iter().take(40) {
            println!("  {:.0},{:.0},{:.0}", p[0], p[1], p[2]);
        }
    }
    Ok(())
}

/// One line per `WCVfxBehaviour`: label, what contacts do to it, its effects and what it switches.
/// The `WCPlaySoundBehaviour`s (`mw_assets::world_sfx::WcSound`): label, flags, channel and list sizes.
fn print_sounds(indent: &str, sounds: &[mw_assets::world_sfx::WcSound]) {
    for s in sounds {
        println!(
            "{indent}sound {:?} {}: on contact {}, off {}, offset {:?}, channel {}, car hit {} (many {}), world {}, \
             slide {}, damage {}",
            s.label,
            s.object,
            s.on_collision,
            s.off_on_collision,
            s.offset,
            s.mixer_channel.map_or_else(|| "-".to_string(), |h| h.to_string()),
            s.collision_waves.len(),
            s.many_waves.len(),
            s.world_waves.len(),
            s.slide_waves.len(),
            s.damage_waves.len(),
        );
    }
}

fn print_vfx(indent: &str, vfx: &[mw_assets::props::WcVfx]) {
    for v in vfx {
        let id = |h: Option<ResourceId>| h.map_or_else(|| "-".to_string(), |h| h.to_string());
        println!(
            "{indent}vfx {:?} {}: on at start {}, on by contact {}, off by contact {}, impact effect {}, \
             contact effect {} above {} m/s, lights {} coronas {} spot effects {}",
            v.label,
            v.object,
            v.active_at_start,
            v.activate_on_collision,
            v.deactivate_on_collision,
            id(v.impact_effect),
            id(v.contact_effect),
            v.contact_speed,
            v.lights,
            v.coronas,
            v.spot_effects.len(),
        );
        for e in &v.spot_effects {
            let at: Vec<String> = e
                .locators
                .iter()
                .map(|(q, p)| {
                    format!("({:.2} {:.2} {:.2} | q {:.2} {:.2} {:.2} {:.2})", p[0], p[1], p[2], q[0], q[1], q[2], q[3])
                })
                .collect();
            println!("{indent}   spot effect {} group {}: {}", id(e.effect), e.group, at.join(" "));
        }
    }
}
