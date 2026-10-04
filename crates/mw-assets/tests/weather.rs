//! Environment and weather data: the timeline paint map, Alternate keyframes, road wetness inputs.
//! Skipped when no game install is found.

use mw_assets::AssetStore;
use mw_assets::environment::{self, PUDDLE_MAP, TIMELINE_PAINT_MAP};
use mw_bundle::{ResourceId, ResourceType};
use mw_gamedir::GameDir;

fn store() -> Option<AssetStore> {
    let Some(game) = GameDir::locate(None).ok() else {
        eprintln!("skipped: no game install");
        return None;
    };
    Some(AssetStore::new(game).unwrap())
}

/// The timeline paint map lists the 25 timelines, covers every track unit's zone, and gives the expected
/// timelines at two reference places.
#[test]
fn timeline_paint_map() {
    let Some(store) = store() else { return };
    let map = mw_formats::WorldPaintMap::parse(&store.load(TIMELINE_PAINT_MAP).unwrap()).unwrap();
    assert_eq!(map.entries.len(), 26 - 1);
    assert_eq!(map.entries[0], 0);
    for e in &map.entries[1..] {
        let o = store.decode_genesys(&store.load(ResourceId(0x0100_0000_0000_0000 | *e as u64)).unwrap()).unwrap();
        assert_eq!(o.type_name, "Genesys.Gen.EnvironmentTimeline");
    }
    let zones = store.zone_list().unwrap();
    for z in &zones.zones {
        let [x0, z0, x1, z1] = z.bounds;
        let (cx, cz) = ((x0 + x1) / 2.0, (z0 + z1) / 2.0);
        // Sample the zone's own points' centroid when the box centre is outside the polygon.
        let (cx, cz) = if z.contains(cx, cz) {
            (cx, cz)
        } else {
            let n = z.points.len() as f32;
            (z.points.iter().map(|p| p[0]).sum::<f32>() / n, z.points.iter().map(|p| p[1]).sum::<f32>() / n)
        };
        assert!(map.entry_at(cx, cz).is_some(), "zone {} at {cx:.0},{cz:.0} is unpainted", z.id);
    }
    // The drive spawn west of downtown and a downtown plaza.
    assert_eq!(store.environment_timeline_at(-1158.0, 1700.0).unwrap(), ResourceId(0x0100_0000_0012_45c8));
    assert_eq!(store.environment_timeline_at(815.0, 1760.0).unwrap(), ResourceId(0x0100_0000_0012_45c6));
}

/// Every timeline keyframe at the noon / night bands has an Alternate twin with a nearer fog.
#[test]
fn alternate_keyframes() {
    let Some(store) = store() else { return };
    for hour in [0, 8, 13, 17] {
        let k = store.environment_keyframe_at(environment::DEFAULT_TIMELINE, hour).unwrap();
        let alt = k.alternate.expect("default timeline keyframe without an Alternate");
        let a = store.environment_keyframe(alt).unwrap();
        assert!(a.alternate.is_none(), "alternates do not chain");
        assert!(a.fog.far < k.fog.far, "hour {hour}: alternate fog {} vs {}", a.fog.far, k.fog.far);
        assert!(a.weather.wind_speed_max > 0.0);
    }
}

/// Road meshes carry a per-vertex wetness in TEXCOORD5.x (0..1) and the puddle map is a 4-channel
/// texture.
#[test]
fn road_wetness_inputs() {
    let Some(store) = store() else { return };
    let tex = store.texture(PUDDLE_MAP).unwrap();
    assert_eq!((tex.width, tex.height), (256, 256));
    let b = store.bundle("HAWAII/TRK_UNIT1.BNDL").unwrap();
    let (mut n, mut wet, mut max) = (0usize, 0usize, 0f32);
    let mut hist = [0usize; 11];
    for e in b.entries_of_type(ResourceType::Renderable) {
        let Ok((meshes, _)) = store.renderable_meshes(e.id) else { continue };
        for sm in meshes.iter().filter(|m| m.shader_name.starts_with("DriveableSurface")) {
            let Some(t5) = sm.mesh.texcoord(5) else { continue };
            for t in t5 {
                n += 1;
                wet += (t[0] > 0.01) as usize;
                max = max.max(t[0]);
                hist[((t[0].clamp(0.0, 1.0) * 10.0) as usize).min(10)] += 1;
            }
        }
    }
    eprintln!("road vertices {n}, wet (TEXCOORD5.x > 0.01) {wet}, max {max}, tenths {hist:?}");
    assert!(n > 1000 && wet > 0);
}

/// Fractional hours blend the two timeline entries around them, wrapping past midnight.
#[test]
fn timeline_blend() {
    let Some(store) = store() else { return };
    let tl = environment::DEFAULT_TIMELINE;
    let p = store.timeline_position(tl, 13.5).unwrap();
    assert_eq!((p.from, p.to), (ResourceId(0x0100_0000_0012_4619), ResourceId(0x0100_0000_0012_4618)));
    assert!((p.t - 0.5).abs() < 1e-6);
    // The last entry (23:00) blends into the first (00:00) over one hour.
    let p = store.timeline_position(tl, 23.25).unwrap();
    assert_eq!(p.to, ResourceId(0x0100_0000_0013_6cc6));
    assert!((p.t - 0.25).abs() < 1e-6);
    let (a, b) = (store.environment_keyframe(p.from).unwrap(), store.environment_keyframe(p.to).unwrap());
    let mid = store.environment_at(tl, 23.25, false).unwrap();
    for i in 0..3 {
        let want = a.key_light[i] + (b.key_light[i] - a.key_light[i]) * 0.25;
        assert!((mid.key_light[i] - want).abs() < 1e-5);
    }
}
