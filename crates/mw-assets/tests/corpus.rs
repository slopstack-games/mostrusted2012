//! Checks against the whole install. Skipped (with a message) when no game
//! install is found.

use mw_assets::AssetStore;
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

/// Collision quads are in triangle-strip order: splitting them as (a, b, c) +
/// (b, d, c) gives two triangles facing the same way. (The fan split left a
/// hole in every quad.)
#[test]
fn collision_quads_split_without_bowties() {
    let Some(store) = store() else { return };
    let (mut quads, mut bowties) = (0usize, 0usize);
    for rel in store.bundle_paths().to_vec() {
        let Some(b) = store.bundle(&rel) else { continue };
        for e in b.entries_of_type(ResourceType::PolygonSoupList) {
            let list = mw_formats::PolygonSoupList::parse(&b.load(e).unwrap()).unwrap();
            for soup in &list.soups {
                for p in soup.polygons.iter().filter(|p| p.is_quad) {
                    let v = |i: u8| soup.vertices.get(i as usize).copied();
                    let tris: Vec<_> = p.triangles().filter_map(|t| Some([v(t[0])?, v(t[1])?, v(t[2])?])).collect();
                    let [t0, t1] = tris[..] else { continue };
                    let n = |t: [[f32; 3]; 3]| {
                        let (a, b) = (sub(t[1], t[0]), sub(t[2], t[0]));
                        [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
                    };
                    let (n0, n1) = (n(t0), n(t1));
                    quads += 1;
                    if n0[0] * n1[0] + n0[1] * n1[1] + n0[2] * n1[2] < 0.0 {
                        bowties += 1;
                    }
                }
            }
        }
        store.evict_all();
    }
    eprintln!("{quads} collision quads, {bowties} bow-ties");
    assert!(quads > 0);
    assert!(bowties * 1000 < quads, "{bowties} of {quads} quads split into bow-ties");
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
