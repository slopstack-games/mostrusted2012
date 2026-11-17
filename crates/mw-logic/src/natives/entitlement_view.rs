//! The `EntitlementManager` binding root (`FUN_00649eb0`: type "EntitlementManager", size 0xd20, root registered by
//! `FUN_00a4d2a0` on the entitlement manager EM = G+0x29280): 10 getters (`FUN_00a4e350`) and 1 field
//! (`FUN_00a4e2f0`). Answers from [`super::entitlements::Entitlements`] (offline: base game owned, no DLC pack:
//! the user's decision documented there).
//!
//! | member | source | answer |
//! |--------|--------|--------|
//! | `HasOnlinePass` | 0x654000: EM+0xce0 clear → true, else `FUN_006401b0(0x01176f28)` | [`Entitlements::has_online_pass`] |
//! | `HasPdlc1..4` | 0x654030 / 50 / 70 / 90: `FUN_006401b0` on the pack's global id | [`Entitlements::has_pdlc`] |
//! | `IsPdlc3Purchasable`, `IsPdlc4Purchasable` | 0x654880 / 0x6548a0: `FUN_00647680(3 / 4)` | [`Entitlements::pdlc_purchasable`] |
//! | `ThankYouCount` | 0x6514c0: EM+0x9d4 | [`Entitlements::thank_you_count`] |
//! | `ShowDirectDownloadInfo` | field EM+0xae6 (bool) | [`Entitlements::direct_download`] |
//!
//! `ThankYouItem[i]` (0x6530c0, a `ThankYouItemViewModel` list) does not resolve: no item is queued offline.
//! `DlcMessageIndicator` (0x651480: the profile value 0x11, property type 1) does not resolve yet, so the HSM's
//! "Is DLC Messaging Required?" (2241062) keeps taking its `ONFAILEXIT` TRUE exits as before: its
//! `HSB_SEL_EX_B` reads `DlcMessageIndicator(GTEQ:n)` through the string visitor, whose int32 slot (0xa49bf0,
//! slot 12 of 0x00fd8368) prints `%d` without the comparison, and which slot property type 1 reaches
//! (`FUN_00a4eb00`) and the profile's value are not traced (research log 2026-11-17).
//!
//! [`Entitlements::has_online_pass`]: super::entitlements::Entitlements::has_online_pass
//! [`Entitlements::has_pdlc`]: super::entitlements::Entitlements::has_pdlc
//! [`Entitlements::pdlc_purchasable`]: super::entitlements::Entitlements::pdlc_purchasable
//! [`Entitlements::thank_you_count`]: super::entitlements::Entitlements::thank_you_count
//! [`Entitlements::direct_download`]: super::entitlements::Entitlements::direct_download

use crate::binding::{Lookup, Segment, Value};

use super::LogicState;

/// `EntitlementManager.<member>`.
pub fn lookup(s: &LogicState, members: &[Segment]) -> Lookup {
    let Some((first, _)) = members.split_first() else { return Lookup::Found(Value::Object(true)) };
    let Some(name) = MEMBERS.iter().find(|n| crate::binding::hash(n) == first.hash) else { return Lookup::Null };
    let e = &s.entitlements;
    let b = |v: bool| Lookup::Found(Value::Bool(v));
    match *name {
        "HasOnlinePass" => b(e.has_online_pass()),
        "HasPdlc1" => b(e.has_pdlc(1)),
        "HasPdlc2" => b(e.has_pdlc(2)),
        "HasPdlc3" => b(e.has_pdlc(3)),
        "HasPdlc4" => b(e.has_pdlc(4)),
        "IsPdlc3Purchasable" => b(e.pdlc_purchasable(3)),
        "IsPdlc4Purchasable" => b(e.pdlc_purchasable(4)),
        "ShowDirectDownloadInfo" => b(e.direct_download),
        "ThankYouCount" => Lookup::Found(Value::Int(e.thank_you_count)),
        _ => Lookup::Null,
    }
}

const MEMBERS: &[&str] = &[
    "HasOnlinePass",
    "HasPdlc1",
    "HasPdlc2",
    "HasPdlc3",
    "HasPdlc4",
    "IsPdlc3Purchasable",
    "IsPdlc4Purchasable",
    "ShowDirectDownloadInfo",
    "ThankYouCount",
];

/// `Mode.CurrentVehicleGamepack` (0x717010 → `FUN_006e7210`): the pack of the local player's car (`FUN_0042e1b0`
/// on its vehicle id, +0x2c `GamePackID`, then `FUN_00643b40`), a `GamePack` object whose members are its fields
/// (`Release`, `GameChangerID`); no car or no pack gives the null object.
pub fn current_vehicle_gamepack(s: &LogicState, rest: &[Segment]) -> Lookup {
    let pack = s
        .players
        .local_player()
        .map(|p| s.entitlements.vehicle_pack(p.vehicle_id))
        .and_then(|id| s.entitlements.game_pack(id));
    let Some(pack) = pack else { return Lookup::Null };
    let Some(f) = rest.first() else { return Lookup::Found(Value::Object(true)) };
    let h = crate::binding::hash;
    if f.hash == h("Release") {
        Lookup::Found(Value::Int(i32::from(pack.release)))
    } else if f.hash == h("GameChangerID") {
        Lookup::Found(Value::UInt(pack.id))
    } else {
        Lookup::Null
    }
}

#[cfg(test)]
mod tests {
    use super::super::LogicState;
    use super::super::entitlements::{GamePack, ids};

    fn state() -> LogicState {
        let mut s = LogicState::default();
        s.entitlements.game_packs = vec![
            GamePack { id: 17127, entitlement: 0, release: 0 },
            GamePack { id: 2196330, entitlement: ids::PDLC2, release: 2 },
            GamePack { id: 2196660, entitlement: ids::PDLC3, release: 3 },
            GamePack { id: 2196661, entitlement: ids::PDLC4, release: 4 },
        ];
        s
    }

    #[test]
    fn entitlement_manager_root() {
        let mut s = state();
        assert_eq!(s.resolve_bool("EntitlementManager.HasOnlinePass"), Some(true));
        for n in 1..=4 {
            assert_eq!(s.resolve_bool(&format!("EntitlementManager.HasPdlc{n}")), Some(false));
        }
        assert_eq!(s.resolve_bool("EntitlementManager.HasPdlc2(NOT)"), Some(true));
        assert_eq!(s.resolve_bool("EntitlementManager.IsPdlc3Purchasable"), Some(true));
        assert_eq!(s.resolve_bool("EntitlementManager.ShowDirectDownloadInfo(NOT)"), Some(true));
        assert_eq!(s.resolve_bool("EntitlementManager.ThankYouCount(GTEQ:1)"), Some(false));
        assert_eq!(s.resolve_hsm_string("EntitlementManager.DlcMessageIndicator(GTEQ:1)"), None, "not resolved yet");
        s.entitlements.add(ids::PDLC2);
        assert_eq!(s.resolve_bool("EntitlementManager.HasPdlc2"), Some(true));
        assert_eq!(s.resolve_bool("EntitlementManager.HasPdlc1"), Some(false));
    }

    #[test]
    fn current_vehicle_gamepack() {
        let mut s = state();
        s.entitlements.vehicle_packs.insert(2076462, 2196661);
        s.entitlements.vehicle_packs.insert(52, 17127);
        s.players.local = Some(0);
        s.players.slots[0] = Some(Default::default());
        assert_eq!(s.resolve_bool("Mode.CurrentVehicleGamepack.Release(EQUALS:0)"), Some(false), "no car: null");
        s.players.local_player_mut().unwrap().vehicle_id = 52;
        assert_eq!(s.resolve_bool("Mode.CurrentVehicleGamepack.Release(EQUALS:0)"), Some(true));
        s.players.local_player_mut().unwrap().vehicle_id = 2076462;
        assert_eq!(s.resolve_bool("Mode.CurrentVehicleGamepack.Release(EQUALS:4)"), Some(true));
        assert_eq!(s.resolve_bool("Mode.CurrentVehicleGamepack.Release(EQUALS:0)"), Some(false));
    }
}
