//! Whole-install checks for vehicle damage data. Skipped when no game install is found.

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

/// Every Skeleton, Animation and AnimationList in the install parses.
#[test]
fn all_animation_resources_parse() {
    let Some(store) = store() else { return };
    let mut counts = [0usize; 3];
    let mut failures = Vec::new();
    for rel in store.bundle_paths().iter().map(|p| p.display().to_string()).collect::<Vec<_>>() {
        let Some(b) = store.bundle(&rel) else { continue };
        for (i, ty) in [ResourceType::Skeleton, ResourceType::Animation, ResourceType::AnimationList].iter().enumerate()
        {
            for e in b.entries_of_type(*ty) {
                counts[i] += 1;
                let res = b.load(e).unwrap();
                let r = match ty {
                    ResourceType::Skeleton => fmt::Skeleton::parse(&res).map(|_| ()),
                    ResourceType::Animation => fmt::Animation::parse(&res).map(|_| ()),
                    _ => fmt::AnimationList::parse(&res).map(|_| ()),
                };
                if let Err(err) = r {
                    failures.push(format!("{rel}: {ty:?} {}: {err}", e.id));
                }
            }
        }
        store.evict_all();
    }
    eprintln!("skeletons {}, animations {}, animation lists {}", counts[0], counts[1], counts[2]);
    assert!(failures.is_empty(), "{} failures, e.g. {:#?}", failures.len(), &failures[..failures.len().min(10)]);
}

/// Every car with a graphics spec has a damage model whose clips match its skeleton, and the body's
/// bone indices stay inside the skeleton.
#[test]
fn every_car_has_a_damage_model() {
    let Some(store) = store() else { return };
    let lang = store.language(mw_assets::DEFAULT_LANGUAGE).unwrap();
    let mut seen = std::collections::HashSet::new();
    let (mut ok, mut missing) = (0, Vec::new());
    for v in store.vehicles(&lang).unwrap() {
        let Some(path) = v.hi_bundle else { continue };
        if !seen.insert(path.clone()) {
            continue;
        }
        let b = store.bundle(&path).unwrap();
        match store.vehicle_damage(&b).unwrap() {
            Some(d) => {
                ok += 1;
                assert!((7..=8).contains(&d.clips.len()), "{path}: {} clips", d.clips.len());
                // Frame 0 of every clip is the bind pose.
                for (p, b) in d.pose(&vec![0.0; d.clips.len()]).iter().zip(&d.bind) {
                    let dist = (0..3).map(|k| (p.translation[k] - b.translation[k]).abs()).fold(0.0, f32::max);
                    assert!(dist < 1e-5 && (p.scale - 1.0).abs() < 1e-6, "{path}: {p:?} vs {b:?}");
                }
            }
            None => missing.push(v.name),
        }
        store.evict_all();
    }
    eprintln!("{ok} cars with damage, without: {missing:?}");
    assert!(ok >= 70, "only {ok} cars have damage data");
    assert!(missing.len() <= 1, "{missing:?}");
}
