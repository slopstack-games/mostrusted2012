//! The entitlement manager **EM** (G+0x29280, `FUN_006401b0`'s `this`) and the game-pack manager **GPM**
//! (G+0x2ab00, EM+0xadc → G): which content packs the player owns and how the logic gates on them (decomp.md
//! "DLC and entitlements").
//!
//! - **Owned set**: EM+0x22c, up to 64 entitlement ids, count at +0x32c (0 from the constructor, the stores at
//!   0x64adfb..0x64ae15). Filled from the platform's owned product tags through the `NucleusGrantMappingsList`
//!   (`FUN_00643810`: tag → `Entitlement` objects) and from the save's "entitlements" cache (`FUN_00647400` via
//!   `FUN_0072f9b0`); `FUN_00640030` writes new ones back and queues their thank-you items (+0x9d4).
//! - **The check** `FUN_006401b0(id)`: the online pass (`0x01176f28`) counts as owned while the pass is not required
//!   (+0xce0 clear) or +0x1a4 / +0xae5 are set; any other id is owned when it is in the set.
//! - **Game packs**: GPM's list (+0x6c / +0x70, `GAMEPLAY.BNDL` `GamePack` items: `GameChangerID` +0x1c,
//!   `72c1dd5c` entitlement +0x24, `Release` +0x28). The base pack (17127) has release 0 and no entitlement;
//!   the four DLC packs have releases 1..4 and the `PDLC*` entitlements.
//!
//! **Offline default (user decision, 2026-11-17, not a model value)**: the Steam install ships every DLC pack in
//! one depot, so the files cannot show what the player owns. Until a real entitlement source exists the owned set
//! is empty: the base game counts as owned (its pack needs no entitlement) and every DLC pack as not owned.

use std::collections::HashMap;

/// Entitlement ids (`Genesys.Gen.Entitlement` objects in `GAMEPLAY.BNDL`, their `GameChangerID`; the globals are
/// set by static initialisers in `.text`: 0xf56530.., 0xf5bf10..).
pub mod ids {
    /// "OnlinePass" (`DAT_01176f28`, initialiser 0xf56530).
    pub const ONLINE_PASS: u32 = 94740;
    /// "LimitedEdition" (`DAT_01176f24` / `DAT_01179b40`; set, never read by the getters).
    pub const LIMITED_EDITION: u32 = 94746;
    /// DLC1 "Ultimate Speed Pack" (`DAT_01176c70` / `DAT_011797a8`, 0x2045b9).
    pub const PDLC1: u32 = 2115001;
    /// DLC2 "Terminal Velocity Pack" (`DAT_01176f68` / `DAT_01179b78`, 0x2195ac): the airport.
    pub const PDLC2: u32 = 2201004;
    /// DLC3 "Movie Legends Pack" (`DAT_01177100` / `DAT_01179c7c`, 0x2195ad).
    pub const PDLC3: u32 = 2201005;
    /// DLC4 "NFS Heroes Pack" (`DAT_01176e04` / `DAT_011799f4`, 0x2195ae).
    pub const PDLC4: u32 = 2201006;
    /// The four DLC packs, by release - 1 (`HasPdlc1..4`).
    pub const PDLC: [u32; 4] = [PDLC1, PDLC2, PDLC3, PDLC4];
}

/// EM+0x22c holds at most 64 ids (`local_100[64]` in `FUN_00647400`, the save table's 0x40).
pub const MAX_OWNED: usize = 64;

/// One `GamePack` of GPM's list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GamePack {
    /// +0x1c `GameChangerID` (17127 = the base game).
    pub id: u32,
    /// +0x24 `72c1dd5c`: the entitlement that grants it (0 = none needed).
    pub entitlement: u32,
    /// +0x28 `Release`: 0 = the base game, 1..4 = the DLC packs.
    pub release: u16,
}

/// What the logic knows about entitlements and packs (EM and GPM).
#[derive(Debug, Clone, Default)]
pub struct Entitlements {
    /// EM+0x22c (count +0x32c): the owned entitlement ids. Empty offline (the user decision above).
    pub owned: Vec<u32>,
    /// EM+0xce0: the online pass is required (0 from the constructor; no other writer in the dump).
    pub online_pass_required: bool,
    /// EM+0x1a4: the online pass check is lifted (0 from the constructor).
    pub flag_1a4: bool,
    /// EM+0xae5: `FUN_004128c0()` (returns 0 on PC).
    pub flag_ae5: bool,
    /// EM+0xae6 `ShowDirectDownloadInfo` ("Is this the Direct Download Sony version?"): 0 from the constructor.
    pub direct_download: bool,
    /// EM+0x9d4 `ThankYouCount`: thank-you messages queued by `FUN_00640030` (0 while nothing new is owned).
    pub thank_you_count: i32,
    /// `DlcMessageIndicator` (0x651480: profile value 0x11 through `FUN_0069fce0` / `FUN_00732650`;
    /// `SetDlcMessageIndicator` writes it). Kept, not yet answered by the root (`entitlement_view`).
    pub dlc_message_indicator: i32,
    /// The save's "entitle_vis" table (`FUN_0072fcc0`): entitlement ids marked visible.
    pub visible: Vec<u32>,
    /// GPM+0x6c / +0x70: the game packs (filled from the install; empty = not loaded).
    pub game_packs: Vec<GamePack>,
    /// `VehicleList` id → `GamePackID` (+0x2c of the vehicle list entry, `FUN_0042e1b0`), filled from the install.
    pub vehicle_packs: HashMap<u32, u32>,
    /// `GamePackConfig` +0x10 (`GlobalConfig` +0x2c): the pack the airport needs (`FUN_00643c90`; 2196330).
    pub airport_pack: u32,
}

impl Entitlements {
    /// `FUN_006401b0`: the entitlement is owned.
    pub fn has(&self, id: u32) -> bool {
        if id == ids::ONLINE_PASS && (!self.online_pass_required || self.flag_1a4 || self.flag_ae5) {
            return true;
        }
        self.owned.contains(&id)
    }

    /// `FUN_00647400` / `FUN_00643810`'s add: skipped when already owned, the online pass only while it is required.
    pub fn add(&mut self, id: u32) {
        if self.owned.contains(&id) || self.owned.len() >= MAX_OWNED {
            return;
        }
        if id != ids::ONLINE_PASS || self.online_pass_required {
            self.owned.push(id);
        }
    }

    /// `HasOnlinePass` (0x654000): true without the requirement, else `has(ONLINE_PASS)`.
    pub fn has_online_pass(&self) -> bool {
        !self.online_pass_required || self.has(ids::ONLINE_PASS)
    }

    /// `HasPdlc1..4` (0x654030 / 0x654050 / 0x654070 / 0x654090; `FUN_006402d0` / `2e0` / `2f0` / `300`).
    pub fn has_pdlc(&self, n: usize) -> bool {
        (1..=4).contains(&n) && self.has(ids::PDLC[n - 1])
    }

    /// `FUN_00643b40`: the pack with this id.
    pub fn game_pack(&self, id: u32) -> Option<&GamePack> {
        self.game_packs.iter().find(|p| p.id == id)
    }

    /// `FUN_00643ae0`: the pack with this release.
    pub fn game_pack_by_release(&self, release: u16) -> Option<&GamePack> {
        self.game_packs.iter().find(|p| p.release == release)
    }

    /// `FUN_00643c10`: a DLC pack (release > 0).
    pub fn is_dlc_pack(&self, id: u32) -> bool {
        self.game_pack(id).is_some_and(|p| p.release > 0)
    }

    /// `FUN_00643c50`: the pack's release is `release`.
    pub fn pack_release_is(&self, id: u32, release: u16) -> bool {
        self.game_pack(id).is_some_and(|p| p.release == release)
    }

    /// `FUN_00647780`: the pack is owned: it exists and needs no entitlement or an owned one.
    pub fn pack_owned(&self, id: u32) -> bool {
        self.game_pack(id).is_some_and(|p| p.entitlement == 0 || self.has(p.entitlement))
    }

    /// `FUN_0072fcc0`: the entitlement is shown (in lists, the store): the four DLC packs always, others when the
    /// save's "entitle_vis" table marks them.
    pub fn entitlement_visible(&self, id: u32) -> bool {
        id != 0 && (ids::PDLC.contains(&id) || self.visible.contains(&id))
    }

    /// `FUN_00647700`: the pack is shown (its content listed): it exists and its entitlement is visible or owned.
    pub fn pack_visible(&self, id: u32) -> bool {
        self.game_pack(id).is_some_and(|p| self.entitlement_visible(p.entitlement) || self.has(p.entitlement))
    }

    /// `IsPdlc3Purchasable` / `IsPdlc4Purchasable` (`FUN_00647680(3 / 4)`): the pack of that release exists and its
    /// entitlement is visible or owned.
    pub fn pdlc_purchasable(&self, release: u16) -> bool {
        self.game_pack_by_release(release)
            .is_some_and(|p| self.entitlement_visible(p.entitlement) || self.has(p.entitlement))
    }

    /// `FUN_00686010`: an unlock record whose source (`GameUnlock` `6060c3fa`, an `Entitlement`) is not owned is
    /// blocked; one without a source is not.
    pub fn unlock_blocked(&self, source_entitlement: Option<u32>) -> bool {
        source_entitlement.is_some_and(|e| !self.has(e))
    }

    /// `FUN_00640210`: any of the listed entitlements is owned (`GameLogicConfig` +0xac: LimitedEdition and the
    /// four DLC packs; the double-SP bonus `FUN_006bdee0` / `FUN_006c2140`).
    pub fn owns_any(&self, list: &[u32]) -> bool {
        list.iter().any(|&e| self.has(e))
    }

    /// The pack of a car (`FUN_0042e1b0(id)` +0x2c), 0 when unknown.
    pub fn vehicle_pack(&self, vehicle: u32) -> u32 {
        self.vehicle_packs.get(&vehicle).copied().unwrap_or(0)
    }

    /// The content check of the event filters (`FUN_006f3830`, `FUN_0074a600`): a DLC pack's content is
    /// available only when its entitlement is owned.
    pub fn pack_content_available(&self, pack: u32) -> bool {
        match self.game_pack(pack) {
            Some(p) if p.release > 0 => self.has(p.entitlement),
            _ => true,
        }
    }

    /// `FUN_00678e60` / `FUN_00688c90`: mod slots 11 (DRIFT TIRES) and 13 (JUMP NITROUS) need the Terminal
    /// Velocity pack (`HasPdlc2`).
    pub fn mod_slot_available(&self, slot: i32) -> bool {
        !(slot == 0xb || slot == 0xd) || self.has_pdlc(2)
    }

    /// `FUN_00643c90` on the local player: the airport's pack (`GamePackConfig` +0x10) is in the player's GPM bits.
    /// `FUN_006478e0` sets the local player's bit of each pack from the unlock manager (`FUN_0068f190(pack id)`);
    /// the packs' only `GameUnlock` records are sourced by their entitlements, so the bit is [`Self::pack_owned`].
    pub fn airport_pack_owned(&self) -> bool {
        self.pack_owned(self.airport_pack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The install's packs (`GAMEPLAY.BNDL` resource 0x33ba).
    fn packs() -> Entitlements {
        let p = |id, entitlement, release| GamePack { id, entitlement, release };
        Entitlements {
            game_packs: vec![
                p(17127, 0, 0),
                p(2115003, ids::PDLC1, 1),
                p(2196330, ids::PDLC2, 2),
                p(2196660, ids::PDLC3, 3),
                p(2196661, ids::PDLC4, 4),
            ],
            airport_pack: 2196330,
            ..Default::default()
        }
    }

    #[test]
    fn offline_default_owns_the_base_game_only() {
        let e = packs();
        assert!(e.has_online_pass());
        assert!(e.has(ids::ONLINE_PASS));
        assert!((1..=4).all(|n| !e.has_pdlc(n)));
        assert!(e.pack_owned(17127));
        assert!(!e.pack_owned(2115003) && !e.pack_owned(2196661));
        assert!(!e.pack_owned(1), "an unknown pack is not owned");
        assert!(!e.airport_pack_owned());
        assert!(e.pack_content_available(17127) && !e.pack_content_available(2196330));
        assert!(!e.mod_slot_available(0xb) && !e.mod_slot_available(0xd) && e.mod_slot_available(0xc));
        assert!(e.unlock_blocked(Some(ids::PDLC3)) && !e.unlock_blocked(None));
        // The DLC packs stay listed (visible) without being owned.
        assert!(e.pack_visible(2196660) && e.is_dlc_pack(2196660) && !e.is_dlc_pack(17127));
        assert!(!e.pack_visible(17127), "the base pack has no entitlement to show");
        assert!(e.pdlc_purchasable(3) && e.pdlc_purchasable(4));
    }

    #[test]
    fn owning_a_pack() {
        let mut e = packs();
        e.add(ids::PDLC2);
        e.add(ids::PDLC2);
        e.add(ids::ONLINE_PASS);
        assert_eq!(e.owned, vec![ids::PDLC2], "duplicates and the unrequired online pass are skipped");
        assert!(e.has_pdlc(2) && !e.has_pdlc(1));
        assert!(e.airport_pack_owned() && e.pack_owned(2196330) && e.mod_slot_available(0xd));
        assert!(e.owns_any(&[ids::LIMITED_EDITION, ids::PDLC2]));
        e.online_pass_required = true;
        assert!(!e.has_online_pass());
        e.add(ids::ONLINE_PASS);
        assert!(e.has_online_pass());
    }
}

/// The packs and each car's pack read from the install, nothing owned (the offline rule above): what the logic
/// host and the runtime's DLC lock start from.
#[cfg(feature = "assets")]
pub fn load(store: &mw_assets::AssetStore) -> mw_assets::Result<Entitlements> {
    let packs = store.pack_data()?;
    let lang = store.language(mw_assets::DEFAULT_LANGUAGE)?;
    Ok(Entitlements {
        game_packs: packs
            .game_packs
            .iter()
            .map(|p| GamePack { id: p.id, entitlement: p.entitlement, release: p.release })
            .collect(),
        airport_pack: packs.airport_pack,
        vehicle_packs: store.vehicles(&lang)?.iter().map(|v| (v.id, v.game_pack)).collect(),
        ..Default::default()
    })
}
