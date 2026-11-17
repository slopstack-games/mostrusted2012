//! Content packs and entitlements as the install describes them (decomp.md "DLC and entitlements"):
//!
//! - `GAMELOGIC/GAMEPLAY.BNDL`: the `GamePack` list (the game-pack manager's +0x6c list: `GameChangerID`,
//!   `72c1dd5c` entitlement, `Release`), the `Entitlement` objects (`Name`, `Description`, `Purchasable`) and the
//!   `GameUnlockList`, whose `GameUnlock` records name a source object (`6060c3fa`, a handle: an `Entitlement`
//!   for the DLC grants) and the asset they unlock (`d2cded7d`, with `UnlockAssetType`: 4 = vehicle, 9 = game
//!   pack, −1 = event).
//! - `GLOBALCONFIG.BNDL` `GamePackConfig` (`GlobalConfig` +0x2c): +0x10 `3e261246` is the pack the airport needs
//!   (`FUN_00643c90`).
//!
//! The per-car pack is the vehicle list's +0x2c ([`crate::VehicleInfo::game_pack`]), the per-event pack the
//! event's `GamePack` ([`crate::events::Event::game_pack`]).

use mw_formats::genesys::{Object, Value, field_hash};

use crate::{AssetStore, Result};

/// The base game's pack (`GameChangerID` 17127, release 0, no entitlement).
pub const BASE_GAME_PACK: u32 = 17127;

/// One `GamePack`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GamePackInfo {
    /// +0x1c `GameChangerID`.
    pub id: u32,
    /// +0x24 `72c1dd5c`: the entitlement that grants it (0 = none).
    pub entitlement: u32,
    /// +0x28 `Release`: 0 = base game, 1..4 = DLC.
    pub release: u16,
    /// `DisplayName`: language string id.
    pub display_name: u32,
}

/// One `Entitlement` object.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntitlementInfo {
    /// `GameChangerID` (the id `FUN_006401b0` compares).
    pub id: u32,
    pub name: String,
    pub description: String,
    pub purchasable: bool,
}

/// A `GameUnlock` record whose source is an entitlement: the asset is unlocked only while it is owned
/// (`FUN_00686010`, `FUN_00688fa0`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EntitlementGrant {
    pub entitlement: u32,
    /// `d2cded7d`: the unlocked asset (a `VehicleList` id, a pack's `GameChangerID`, an event id).
    pub asset: u32,
    /// `UnlockAssetType`: 4 vehicle, 9 game pack, −1 event.
    pub asset_type: i32,
}

/// Everything the install says about packs and entitlements.
#[derive(Debug, Clone, Default)]
pub struct PackData {
    pub game_packs: Vec<GamePackInfo>,
    pub entitlements: Vec<EntitlementInfo>,
    pub grants: Vec<EntitlementGrant>,
    /// `GamePackConfig` +0x10: the airport's pack.
    pub airport_pack: u32,
}

const ID: u32 = 0x655d_3f5f;

fn uint(v: Option<&Value>) -> u32 {
    match v {
        Some(Value::UInt(u)) => *u as u32,
        Some(Value::Int(i)) => *i as u32,
        _ => 0,
    }
}

fn int(v: Option<&Value>) -> i32 {
    match v {
        Some(Value::UInt(u)) => *u as i32,
        Some(Value::Int(i)) => *i as i32,
        _ => 0,
    }
}

fn text(v: Option<&Value>) -> String {
    if let Some(Value::Str(s)) = v { s.trim().to_string() } else { String::new() }
}

fn is(o: &Object, ty: &str) -> bool {
    o.type_name.strip_prefix("Genesys.Gen.").unwrap_or(&o.type_name) == ty
}

impl AssetStore {
    /// The packs, entitlements and entitlement grants of the install.
    pub fn pack_data(&self) -> Result<PackData> {
        let mut out = PackData::default();
        for o in crate::events::items(self, "GAMELOGIC/GAMEPLAY.BNDL")? {
            if is(&o, "GamePack") {
                out.game_packs.push(GamePackInfo {
                    id: uint(o.get(ID)),
                    entitlement: uint(o.get(0x72c1_dd5c)),
                    release: uint(o.get(field_hash("Release"))) as u16,
                    display_name: uint(o.get(field_hash("DisplayName"))),
                });
            } else if is(&o, "Entitlement") {
                out.entitlements.push(EntitlementInfo {
                    id: uint(o.get(ID)),
                    name: text(o.get(field_hash("Name"))),
                    description: text(o.get(field_hash("Description"))),
                    purchasable: matches!(o.get(field_hash("Purchasable")), Some(Value::Bool(true))),
                });
            } else if is(&o, "GameUnlock")
                && let Some(Value::Handle(Some(src))) = o.get(0x6060_c3fa)
            {
                // A GeneSys object's resource id is 0x0100000000000000 | its `GameChangerID`.
                out.grants.push(EntitlementGrant {
                    entitlement: src.0 as u32,
                    asset: uint(o.get(0xd2cd_ed7d)),
                    asset_type: int(o.get(field_hash("UnlockAssetType"))),
                });
            }
        }
        out.grants.retain(|g| out.entitlements.iter().any(|e| e.id == g.entitlement));
        out.airport_pack = crate::events::items(self, "GLOBALCONFIG.BNDL")?
            .iter()
            .find(|o| is(o, "GamePackConfig"))
            .map_or(0, |o| uint(o.get(0x3e26_1246)));
        Ok(out)
    }
}
