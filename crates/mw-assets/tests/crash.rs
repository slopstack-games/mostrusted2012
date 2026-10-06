//! Whole-install checks for the crash rules and the crash camera (`mw_assets::crash`). Skipped when no
//! game install is found.

use mw_assets::AssetStore;
use mw_assets::crash::CrashingRules;
use mw_bundle::ResourceType;
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

/// The offline player's world rule, the crash sequence's timeline and the reset speeds, as listed in
/// docs/decomp.md ("Crashes").
#[test]
fn crash_data_reads() {
    let Some(store) = store() else { return };
    let data = store.crash_data().expect("crash data");
    println!("{data:#?}");

    let world = data.rules.world.expect("world rule");
    assert_eq!(world.damage, Some(30.0));
    assert_eq!(world.closing_mph, Some(70.0));
    assert!(data.rules.landing.is_none(), "the offline player has no landing rule");
    assert!(world.is_crash(None, 70.5) && !world.is_crash(None, 70.0) && world.is_crash(Some(31.0), 0.0));

    let cam = &data.camera;
    assert_eq!(cam.shots.len(), 2);
    assert_eq!((cam.shots[0].start, cam.shots[0].end), (0.5, 2.0));
    assert_eq!((cam.shots[1].start, cam.shots[1].end), (2.0, 4.5));
    let freeway = &cam.shots[0].camera;
    assert!(freeway.camera_binding.contains("Freeway"), "{}", freeway.camera_binding);
    assert_eq!(freeway.analyser.expect("Freeway analyser").translation, [-5.0, -8.0, 10.0]);
    assert_eq!(freeway.offset, [0.0, 3.0, 0.0]);
    let car_shot = &cam.shots[1].camera;
    assert_eq!(car_shot.offset, [-1.5, 1.0, 3.0]);
    assert_eq!((car_shot.zoom.min_fov, car_shot.zoom.max_fov), (10.0, 90.0));
    assert_eq!(cam.hide_hud, Some((0.0, 4.5)));
    let lb = cam.letterbox.expect("letterbox");
    assert_eq!((lb.start, lb.end, lb.bars, lb.transition_out), (0.0, 4.75, [94.0, 100.0], 0.25));
    // Slow motion: 1 → 0.1 over the first half second, then 0.14 and 0.22, then real time.
    assert!((cam.time_scale(0.0) - 1.0).abs() < 1e-4);
    assert!((cam.time_scale(0.499) - 0.1006).abs() < 1e-2);
    assert_eq!(cam.time_scale(1.0), 0.14);
    assert_eq!(cam.time_scale(3.0), 0.22);
    assert_eq!(cam.time_scale(4.6), 1.0);
    assert_eq!(cam.cameras_end(), 4.5);

    assert_eq!((data.reset.min_mph, data.reset.scale, data.reset.crash_max_mph), (30.0, 0.7, 50.0));
    assert_eq!(data.reset.after_crash(20.0), 30.0);
    assert_eq!(data.reset.after_crash(60.0), 42.0);
    assert_eq!(data.reset.after_crash(100.0), 50.0);
}

/// Every `Physics.CrashingRules` in the install reads, and every present threshold is positive.
#[test]
fn all_crashing_rules_read() {
    let Some(store) = store() else { return };
    let mut n = 0;
    for rel in ["GAMELOGIC/GAMEPLAY.BNDL", "PHYSICS/PHYSICSCONFIG.BNDL"] {
        let b = store.bundle(rel).expect(rel);
        for e in b.entries_of_type(ResourceType::GenesysObject) {
            let o = store.decode_genesys(&b.load(e).unwrap()).unwrap();
            if o.type_name != "Genesys.Gen.Physics.CrashingRules" {
                continue;
            }
            n += 1;
            let r = CrashingRules::from_object(e.id, &o);
            let rules = [r.world, r.traffic, r.landing, r.player, r.roadblock]
                .into_iter()
                .flatten()
                .chain(r.other.iter().map(|(_, r)| *r));
            for rule in rules {
                assert!(rule.damage.is_none_or(|v| v > 0.0) && rule.closing_mph.is_none_or(|v| v > 0.0), "{rule:?}");
                assert!(rule.third.is_none(), "d6c0e908 is empty everywhere");
            }
        }
    }
    assert_eq!(n, 14);
}
