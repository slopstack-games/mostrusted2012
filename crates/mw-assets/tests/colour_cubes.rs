//! Every `ColourCube` in the install parses, and cube `…3e93` is the identity (it is symmetric, so it does
//! not settle the axis order: see `mw_formats::ColourCube`).
//! Skipped when no game install is found.

use mw_assets::AssetStore;
use mw_bundle::{ResourceId, ResourceType};
use mw_gamedir::GameDir;

#[test]
fn colour_cubes_parse() {
    let Some(game) = GameDir::locate(None).ok() else {
        eprintln!("skipped: no game install");
        return;
    };
    let store = AssetStore::new(game).unwrap();
    let mut n = 0;
    for bundle in ["HAWAII/ENVIRONMENT.BNDL", "POSTFX.BNDL"] {
        let b = store.bundle(bundle).unwrap();
        for e in b.entries_of_type(ResourceType::ColourCube) {
            let cube = mw_formats::ColourCube::parse(&b.load(e).unwrap()).unwrap();
            assert_eq!(cube.size, 16, "{}", e.id);
            n += 1;
        }
    }
    assert!(n >= 21, "{n} cubes");
    let identity = mw_formats::ColourCube::parse(&store.load(ResourceId(0x0100_0000_0000_3e93)).unwrap()).unwrap();
    for (r, g, b) in [(15, 0, 0), (0, 15, 0), (0, 0, 15), (3, 7, 11)] {
        assert_eq!(identity.at(r, g, b), [r as u8 * 17, g as u8 * 17, b as u8 * 17]);
    }
}
