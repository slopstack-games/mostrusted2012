//! Whole-install checks for the vehicle cameras (`mw_assets::camera`). Skipped when no game install is
//! found.

use mw_assets::AssetStore;
use mw_assets::camera::VehicleCameras;
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

/// Every car bundle's `VehicleCameraContainer` reads, with the value ranges the camera code relies on
/// (min ≤ max pairs, eye behind the look-at point, the pitch / roll thresholds ordered), and the
/// Porsche 911's matches the values in docs/handling.md.
#[test]
fn all_vehicle_cameras_read() {
    let Some(store) = store() else { return };
    let mut n = 0;
    let mut problems = Vec::new();
    for rel in store.bundle_paths().iter().map(|p| p.display().to_string()).collect::<Vec<_>>() {
        if !rel.starts_with("VEHICLES/VEH_") || !rel.ends_with("_HI.BNDL") {
            continue;
        }
        let Some(b) = store.bundle(&rel) else { continue };
        for e in b.entries_of_type(ResourceType::GenesysObject) {
            let Ok(obj) = store.decode_genesys(&b.load(e).unwrap()) else { continue };
            if obj.type_name != "Genesys.Gen.VehicleCameraContainer" {
                continue;
            }
            n += 1;
            let Some(c) = VehicleCameras::from_container(&obj) else {
                problems.push(format!("{rel}: container does not read"));
                continue;
            };
            let x = &c.external;
            let ok = x.fov[0] < x.fov[1]
                && x.eye[2] < x.look_at[2]
                && x.pitch.deceleration[0] < x.pitch.deceleration[1]
                && x.roll.lateral[0] < x.roll.lateral[1]
                && x.roll.angular[0] < x.roll.angular[1]
                && x.shake.speed[0] < x.shake.speed[1]
                && x.yaw_spring.speed[0] < x.yaw_spring.speed[1]
                && x.acceleration.back < 0.0
                && x.acceleration.forward > 0.0
                && x.impact.rotation[0].limits[0] < x.impact.rotation[0].limits[1]
                && c.bonnet.is_some_and(|b| b.fov[0] <= b.fov[1])
                && c.bumper.is_some_and(|b| b.fov[0] <= b.fov[1]);
            if !ok {
                problems.push(format!("{rel}: {c:?}"));
            }
            if rel.contains("VEH_1085007_HI") {
                assert_eq!(x.eye, [0.0, 1.6, -4.6]);
                assert_eq!(x.fov, [70.0, 95.0]);
                assert_eq!(x.roll.max_angle, 4.7);
                assert_eq!(x.pitch.max_change, 4.5);
                assert_eq!(x.pitch.deceleration, [2.0, 12.0]);
                assert_eq!(x.shake.speed, [0.0, 180.0]);
                assert_eq!(x.impact.rotation[0].spring, 450.0);
                assert_eq!(c.bumper.unwrap().eye, [0.0, 0.425, 1.5]);
            }
        }
        store.evict_all();
    }
    assert!(n >= 70, "only {n} camera containers found");
    assert!(problems.is_empty(), "{} of {n}:\n{}", problems.len(), problems.join("\n"));
}
