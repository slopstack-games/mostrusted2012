//! Whole-install checks for the content packs (`mw_assets::packs`, decomp.md "DLC and entitlements"). Skipped
//! when no game install is found.

use mw_assets::AssetStore;
use mw_assets::packs::BASE_GAME_PACK;
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

/// The five packs with their releases and entitlements, the airport's pack, and the DLC content they gate.
#[test]
fn packs_read() {
    let Some(store) = store() else { return };
    let packs = store.pack_data().expect("pack data");
    let p: Vec<(u32, u32, u16)> = packs.game_packs.iter().map(|p| (p.id, p.entitlement, p.release)).collect();
    assert_eq!(
        p,
        [
            (BASE_GAME_PACK, 0, 0),
            (2115003, 2115001, 1),
            (2196330, 2201004, 2),
            (2196660, 2201005, 3),
            (2196661, 2201006, 4)
        ]
    );
    assert_eq!(packs.airport_pack, 2196330);
    // Each DLC entitlement grants its pack (type 9), five cars (type 4) and one event (type -1).
    for e in [2115001, 2201004, 2201005, 2201006] {
        let kinds: Vec<i32> = packs.grants.iter().filter(|g| g.entitlement == e).map(|g| g.asset_type).collect();
        assert_eq!(kinds.iter().filter(|&&k| k == 9).count(), 1, "{e}");
        assert_eq!(kinds.iter().filter(|&&k| k == 4).count(), 5, "{e}");
        assert_eq!(kinds.iter().filter(|&&k| k == -1).count(), 1, "{e}");
    }
    // Cars and events of the packs (vehicle list +0x2c, event `GamePack`).
    let lang = store.language(mw_assets::DEFAULT_LANGUAGE).expect("language");
    let cars = store.vehicles(&lang).expect("vehicles");
    let dlc_cars: std::collections::BTreeSet<u32> =
        cars.iter().filter(|v| v.game_pack != BASE_GAME_PACK).map(|v| v.id).collect();
    assert_eq!(dlc_cars.len(), 20);
    assert!(cars.iter().any(|v| v.id == 2196000 && v.game_pack == 2196330), "Porsche 918 Spyder: Terminal Velocity");
    let events = store.events().expect("events");
    assert_eq!(events.events.iter().filter(|e| e.game_pack != BASE_GAME_PACK).count(), 46);
}

/// The airport barrier (`CheckPointsConfig` `e74de7e2`): one chevron across the airport road.
#[test]
fn airport_barrier_chevron() {
    let Ok(game) = mw_gamedir::GameDir::locate(None) else { return };
    let store = mw_assets::AssetStore::new(game).unwrap();
    let m = store.route_markers().unwrap();
    let ids: Vec<u32> = m.barriers.iter().map(|c| c.id).collect();
    assert_eq!(ids, [2_204_089], "{:?}", m.barriers);
    let p = &m.barriers[0].points;
    assert!(
        p.len() >= 2 && p.iter().all(|q| (130.0..140.0).contains(&q[0]) && (-2035.0..-1985.0).contains(&q[2])),
        "{p:?}"
    );
}
