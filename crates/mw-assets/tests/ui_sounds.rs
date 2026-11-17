//! Whole-install check of the UI sounds the logic plays by field (`mw_assets::ui_sounds`). Skipped when no game
//! install is found.

use mw_assets::AssetStore;
use mw_assets::sequence::ItemKind;
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

/// The eight `MiscSound.UI` ids by field, each a UI sequence with wave items.
#[test]
fn ui_sounds_by_field() {
    let Some(store) = store() else { return };
    let got: Vec<Option<u32>> = (0x4c..=0x68).step_by(4).map(|f| store.ui_sound(f)).collect();
    let want = [1836407, 1836401, 1836405, 1836335, 1836058, 1836333, 1836331, 1836060];
    assert_eq!(got, want.map(Some));
    assert_eq!(store.ui_sound(0x48), None);
    for id in want {
        let seq = store.ui_sequence(id).expect("UI sound sequence");
        assert!(seq.items.iter().any(|i| matches!(i.kind, ItemKind::Wave(_))), "{id}: no wave item");
    }
}
