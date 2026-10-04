//! World object physics and collision surface tags, checked against the whole install. Skipped (with a
//! message) when no game install is found.

use std::collections::{BTreeMap, HashMap};

use mw_assets::AssetStore;
use mw_assets::backdrop::track_unit_number;
use mw_assets::props::Volume;
use mw_bundle::ResourceType;
use mw_formats::CollisionTag;
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

fn track_units(store: &AssetStore) -> Vec<String> {
    store
        .bundle_paths()
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")))
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// Every placed prop / compound / dynamic object's physical definition decodes, with positive
/// volumes inside the definition's bounds, and every prop and compound has one.
#[test]
fn world_object_physics_decodes() {
    let Some(store) = store() else { return };
    let mut seen = HashMap::new();
    for name in track_units(&store) {
        let Some(b) = store.bundle(&name) else { continue };
        let layers = store.bundle_layers(&b).unwrap();
        for p in layers.props.iter().chain(&layers.compounds).chain(&layers.dynamics) {
            seen.entry(p.object.unwrap()).or_insert(0usize);
        }
        store.evict_all();
    }
    let (mut with, mut volumes) = (0, 0);
    for &(id, kind) in seen.keys() {
        let phys = store.object_physics(id, kind).unwrap_or_else(|e| panic!("{kind:?} {id}: {e}"));
        let Some(def) = &phys.definition else {
            assert_eq!(kind, mw_assets::props::ObjectKind::Dynamic, "{kind:?} {id} has no physical definition");
            continue;
        };
        with += 1;
        assert!(!def.bodies.is_empty(), "{id}: no rigid body");
        let (c, h) = def.bounds;
        for body in &def.bodies {
            assert!(body.mass >= 0.0);
            for v in &body.volumes {
                volumes += 1;
                // The volume centre in object space lies within the bounds (with a little slack).
                let (t, size) = match v {
                    Volume::Box { transform, half_extents } => {
                        (transform, half_extents.iter().fold(0.0, |a: f32, b| a.max(*b)))
                    }
                    Volume::Sphere { transform, radius } => (transform, *radius),
                    Volume::Cylinder { transform, radius, half_length }
                    | Volume::Capsule { transform, radius, half_length } => (transform, radius.max(*half_length)),
                    Volume::Hull { transform, .. } => (transform, 0.0),
                };
                assert!(size > 0.0 || matches!(v, Volume::Hull { .. }), "{id}: empty volume {v:?}");
                let centre: [f32; 3] = std::array::from_fn(|i| {
                    t[3][0] * body.transform[0][i]
                        + t[3][1] * body.transform[1][i]
                        + t[3][2] * body.transform[2][i]
                        + body.transform[3][i]
                });
                for i in 0..3 {
                    assert!(
                        (centre[i] - c[i]).abs() <= h[i] + 0.05,
                        "{id}: volume centre {centre:?} outside bounds {c:?} ± {h:?}"
                    );
                }
            }
        }
    }
    eprintln!("{} object definitions, {with} with physics, {volumes} volumes", seen.len());
    assert!(with >= 200);
}

/// Collision tags: the unit field is the track unit's own number and every surface index is inside the
/// 65-entry surface list.
#[test]
fn collision_tags_decode() {
    let Some(store) = store() else { return };
    let surfaces = store.surfaces().unwrap();
    assert_eq!(surfaces.len(), 65);
    let (mut tris, mut own_unit) = (0usize, 0usize);
    let mut by_surface: BTreeMap<u8, usize> = BTreeMap::new();
    let mut by_flags: BTreeMap<u16, usize> = BTreeMap::new();
    for name in track_units(&store) {
        let Some(b) = store.bundle(&name) else { continue };
        let unit = track_unit_number(&name).unwrap();
        for e in b.entries_of_type(ResourceType::PolygonSoupList) {
            let list = mw_formats::PolygonSoupList::parse(&b.load(e).unwrap()).unwrap();
            let tags = list.triangle_tags();
            assert_eq!(tags.len(), list.triangles().1.len());
            for t in tags.into_iter().map(CollisionTag::from_u32) {
                tris += 1;
                own_unit += (t.unit as u32 == unit) as usize;
                assert!((t.surface as usize) < surfaces.len(), "{name}: surface {}", t.surface);
                *by_surface.entry(t.surface).or_default() += 1;
                *by_flags.entry(t.flags).or_default() += 1;
            }
        }
        store.evict_all();
    }
    eprintln!("{tris} triangles, {own_unit} tagged with their own unit");
    eprintln!("by surface: {by_surface:?}");
    eprintln!("by flags: {by_flags:?}");
    assert!(own_unit * 10 > tris * 9, "only {own_unit} of {tris} triangles carry their unit number");
}

#[test]
#[ignore]
fn diag_raw_tags() {
    let Some(store) = store() else { return };
    for name in ["HAWAII/TRK_UNIT1.BNDL", "HAWAII/TRK_UNIT30.BNDL"] {
        let b = store.bundle(name).unwrap();
        let mut h: BTreeMap<u32, usize> = BTreeMap::new();
        for e in b.entries_of_type(ResourceType::PolygonSoupList) {
            let list = mw_formats::PolygonSoupList::parse(&b.load(e).unwrap()).unwrap();
            for s in &list.soups {
                for p in &s.polygons {
                    *h.entry(p.collision_tag[0] as u32 | (p.collision_tag[1] as u32) << 16).or_default() += 1;
                }
            }
        }
        for (t, n) in &h {
            eprintln!("{name} {t:08x} {t:032b} {n}");
        }
    }
}
