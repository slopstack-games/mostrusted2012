//! Whole-install checks for the zone list and the backdrop composition. Skipped when no game install is
//! found.

use std::collections::BTreeSet;

use mw_assets::AssetStore;
use mw_assets::backdrop::{BackdropSelector, track_unit_number};
use mw_bundle::ResourceId;
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

fn track_units(store: &AssetStore) -> Vec<(u32, String)> {
    store
        .bundle_paths()
        .iter()
        .filter_map(|p| {
            let name = p.to_string_lossy().into_owned();
            Some((track_unit_number(&name)?, name))
        })
        .collect()
}

/// One zone per track unit, with sane polygons and neighbour links.
#[test]
fn zone_list_matches_track_units() {
    let Some(store) = store() else { return };
    let zones = store.zone_list().expect("parse the zone list");
    let ids: BTreeSet<u32> = zones.zones.iter().map(|z| z.id).collect();
    let units: BTreeSet<u32> = track_units(&store).into_iter().map(|(n, _)| n).collect();
    assert_eq!(ids.len(), zones.zones.len(), "zone ids are unique");
    assert_eq!(ids, units, "zone ids are the track unit numbers");
    assert_eq!(zones.zones.iter().map(|z| z.points.len()).sum::<usize>(), 941);
    for z in &zones.zones {
        assert!(z.points.len() >= 3, "zone {} has {} points", z.id, z.points.len());
        assert!(z.neighbours.iter().all(|&(n, f)| n < zones.zones.len() && (f == 1 || f == 3)));
        // The polygon lies within the stored bounds, and zones do not overlap (sampled on a grid).
        let [x0, z0, x1, z1] = z.bounds;
        assert!(
            z.points.iter().all(|p| p[0] >= x0 - 0.01 && p[0] <= x1 + 0.01 && p[1] >= z0 - 0.01 && p[1] <= z1 + 0.01)
        );
        let mut inside = 0;
        for i in 0..16 {
            for j in 0..16 {
                let (x, y) = (x0 + (x1 - x0) * (i as f32 + 0.5) / 16.0, z0 + (z1 - z0) * (j as f32 + 0.5) / 16.0);
                if z.contains(x, y) {
                    inside += 1;
                    let owners: Vec<u32> = zones.zones.iter().filter(|o| o.contains(x, y)).map(|o| o.id).collect();
                    assert_eq!(owners, [z.id], "zones overlap at ({x}, {y})");
                }
            }
        }
        assert!(inside > 0, "zone {} has no area", z.id);
    }
}

fn selector(store: &AssetStore) -> BackdropSelector {
    let mut listings = Vec::new();
    for (unit, name) in track_units(store) {
        let b = store.bundle(&name).unwrap();
        for p in store.bundle_placements(&b).unwrap() {
            if p.backdrop {
                listings.push((unit, p.model, p.transform));
            }
        }
        store.evict_all();
    }
    store.backdrop_selector(listings.iter().map(|(u, m, t)| (*u, *m, t))).expect("build the selector")
}

fn id(hex: &str) -> ResourceId {
    ResourceId::parse(hex).unwrap()
}

/// Known LOD tiers of the same backdrop tile: the composition keeps the tier the local zone uses.
#[test]
fn composition_keeps_one_tier() {
    let Some(store) = store() else { return };
    let sel = selector(&store);
    let centre = |unit: u32| {
        let z = sel.zones().zones.iter().find(|z| z.id == unit).unwrap();
        let n = z.points.len() as f32;
        (z.points.iter().map(|p| p[0]).sum::<f32>() / n, z.points.iter().map(|p| p[1]).sum::<f32>() / n)
    };
    // Hillside south-west of downtown: 22,747 triangles in the backdrop of units 49-53 and 137, 2,342 in
    // that of 26 units farther away.
    let (fine, coarse) = (id("00000000b2fe2fa7"), id("0000000028a73b07"));
    for unit in [49, 52, 53] {
        let (x, z) = centre(unit);
        assert!(sel.keeps(fine, 0, x, z), "fine tile at unit {unit}");
        assert!(!sel.keeps(coarse, 0, x, z), "coarse tile at unit {unit}");
    }
    // Landmark tower at (853, 1508) inside unit 23: unit 23 lists the full model, far units a twin with
    // LOD2 meshes.
    assert!(sel.keeps(id("0000000037852c3c"), 0, 853.0, 1508.0));
    assert!(!sel.keeps(id("00000000462b7b99"), 0, 853.0, 1508.0));
}
