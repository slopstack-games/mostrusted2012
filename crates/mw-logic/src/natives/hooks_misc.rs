//! The seventh twenty HSM hooks (research-log 2026-11-17 "seventh 20 HSM hooks"): the blacklist description, the
//! bool binding, the saved vehicle and rollout, the award progress strings, the settings steppers, the widget
//! group exit, the deferred view models and the small online ones (offline: what they do to the logic's state).
//! Handlers read from the snapshot's disassembly; what only the engine can do is a [`Request`].

use super::hook_args::{self as arg, strtol10};
use super::hooks_flow::exit_if;
use super::lua_widgets::exit_group_via;
use super::{LogicState, Registry, Request};
use crate::hsm::{EXIT, Key};
use mw_formats::json::Json;
use std::collections::BTreeSet;

type Args<'a> = Option<&'a Json>;

/// `Path` (`DAT_01178040`, `ResolveBoolBinding`): a name of four bytes hashes to its bytes.
pub const PATH: u32 = 0x5061_7468;
/// `lpcViewModelName` (`DAT_0117ee04`: 0x1a5058d7, the two deferred view-model hooks).
pub const LPC_VIEW_MODEL_NAME: u32 = 0x1a50_58d7;
/// The setting's index of `IncrementSetting` / `DecrementSetting` (`DAT_0117ec1c`; the category is `lpcValue`,
/// `DAT_0117ec30`). The member's name is not known (hash read from the snapshot's `.bss`, not found among the
/// names tried).
pub const SETTING_INDEX: u32 = 0x2f34_8b52;
/// The text of `ExitWidgetGroup` (`DAT_0117ed40`; name not known, see above).
pub const EXIT_GROUP_TEXT: u32 = 0x8395_9c02;
/// The binding text of `SetJumpToStoreFlag` (`DAT_01177d18`; name not known, see above).
pub const JUMP_BINDING: u32 = 0x9e94_3e15;

/// Fields the hooks of this file touch.
#[derive(Debug, Default)]
pub struct HookMiscFields {
    /// (G+0x2f3fc)+0x20 +0x2c48 / +0x2c49 / +0x2c4a: which description `FUN_006f9380` chose (game pack lock,
    /// vehicle not owned, not yet raced); cleared by `CalculateBlacklistDescription`, set by the runtime's answer.
    pub blacklist_flags: [bool; 3],
    /// The entitlement manager's (G+0x2f3e0 +0x4ce0) +0xdbc: the entitlement the store should open on
    /// (`FUN_0086d610`; `SetJumpToStoreFlag` sets it from its binding, `ClearEntitlementJumpID` to 0).
    pub entitlement_jump_id: i32,
    /// G+0x2a9e4: the visibility of the hosted game (public 4, joinable 3, invite only 2).
    pub game_visibility: i32,
    /// The names the screen manager's page has registered as deferred view models (`FUN_007f7510` looks them up in
    /// each widget group's list); the runtime fills it from the loaded pages.
    pub registered_deferred_view_models: BTreeSet<String>,
    /// `FUN_007f7510` answers false while the page is changing (SM+0x280 +0x1b4, or its +0x21c object's +0x1904, is
    /// not 0); the runtime sets it.
    pub screen_changing: bool,
}

pub fn register(r: &mut Registry) {
    r.set_hook("CalculateBlacklistDescription", calculate_blacklist_description);
    r.set_hook("ResolveBoolBinding", resolve_bool_binding);
    r.set_hook("SetVehicleAndRolloutFromSaveGameByVehicleID", set_vehicle_and_rollout_by_id);
    r.set_hook("SetVehicleAndRolloutFromSaveGameByVehicleIDString", set_vehicle_and_rollout_by_id_string);
    r.set_hook("UpdateAwardProgressBindings", update_award_progress_bindings);
    r.set_hook("IncrementSetting", increment_setting);
    r.set_hook("DecrementSetting", decrement_setting);
    r.set_hook("ExitWidgetGroup", exit_widget_group);
    r.set_hook("UpdateDeferredViewModelSilent", update_deferred_view_model_silent);
    r.set_hook("HasScreenRegisterdDeferredViewModel", has_screen_registerd_deferred_view_model);
    r.set_hook("SetJumpToStoreFlag", set_jump_to_store_flag);
    r.set_hook("ClearEntitlementJumpID", clear_entitlement_jump_id);
    r.set_hook("ClearNetworkedTimer", clear_networked_timer);
    r.set_hook("ClearOnlineEasyDrive", clear_online_easy_drive);
    r.set_hook("RefreshOnlineEasyDrive", refresh_online_easy_drive);
    r.set_hook("SetGameVisibilityInviteOnly", set_game_visibility_invite_only);
    r.set_hook("SetGameVisibilityJoinable", set_game_visibility_joinable);
    r.set_hook("SetGameVisibilityPublic", set_game_visibility_public);
    r.set_hook("SetOnlineRaceCarFrozen", set_online_race_car_frozen);
    r.set_hook("SetOnlineRaceCarUnfrozen", set_online_race_car_unfrozen);
}

/// `CalculateBlacklistDescription` (0x676a70): `strtol(lpcValue)` - 1 → `FUN_006f9380` on (G+0x2f3fc)+0x20: the
/// three flags (+0x2c48 word, +0x2c4a) are cleared first, the rest (which member's text, the HUD message) is the
/// runtime's.
fn calculate_blacklist_description(s: &mut LogicState, args: Args) -> Key {
    let rank = strtol10(arg::string(args, arg::LPC_VALUE)).wrapping_sub(1);
    s.hook_misc.blacklist_flags = [false; 3];
    s.requests.push(Request::CalculateBlacklistDescription { rank });
    EXIT
}

/// `ResolveBoolBinding` (0x680160): the `Path` member through `FUN_0067f4e0` (the Lua native's code): TRUE when the
/// binding resolves to true, FALSE when it does not parse, resolve or is false.
fn resolve_bool_binding(s: &mut LogicState, args: Args) -> Key {
    let b = arg::string(args, PATH);
    exit_if(s.resolve_bool(b).unwrap_or(false))
}

/// `SetVehicleAndRolloutFromSaveGameByVehicleID` (0x680680): `liId` → `FUN_006800a0`.
fn set_vehicle_and_rollout_by_id(s: &mut LogicState, args: Args) -> Key {
    set_vehicle_and_rollout(s, arg::int(args, arg::LI_ID))
}

/// `...ByVehicleIDString` (0x687a30): `strtol(lpcValue)` → `FUN_006800a0`.
fn set_vehicle_and_rollout_by_id_string(s: &mut LogicState, args: Args) -> Key {
    set_vehicle_and_rollout(s, strtol10(arg::string(args, arg::LPC_VALUE)))
}

/// `FUN_006800a0`: only a vehicle id above 0 does anything.
fn set_vehicle_and_rollout(s: &mut LogicState, vehicle: i32) -> Key {
    if vehicle > 0 {
        s.requests.push(Request::SetVehicleAndRolloutFromSaveGame { vehicle });
    }
    EXIT
}

/// `UpdateAwardProgressBindings` (0x7eb4e0): `FUN_00724680` and `FUN_00724690` on (G+0x2f3f8), the calls
/// `UpdateProgressAwardsStrings` makes.
fn update_award_progress_bindings(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::UpdateProgressAwardsStrings);
    EXIT
}

/// The category (`lpcValue`) and the index of the setting, both `strtol`ed (`FUN_007e8900` / `FUN_007e8970`).
fn setting_of(args: Args) -> (i32, i32) {
    let category = strtol10(arg::string(args, arg::LPC_VALUE));
    let index = strtol10(arg::string(args, SETTING_INDEX));
    (category, index)
}

/// `IncrementSetting` (0x7eb610): `FUN_007e8900` raises the setting by one below its maximum; TRUE when it did,
/// else FALSE.
fn increment_setting(s: &mut LogicState, args: Args) -> Key {
    let (c, i) = setting_of(args);
    exit_if(s.settings.increment(c, i))
}

/// `DecrementSetting` (0x7eb680): `FUN_007e8970` lowers it by one above its minimum.
fn decrement_setting(s: &mut LogicState, args: Args) -> Key {
    let (c, i) = setting_of(args);
    exit_if(s.settings.decrement(c, i))
}

/// `ExitWidgetGroup` (0x7f24f0): the text member → `FUN_007eef40`, the Lua native's code (255 bytes at most).
fn exit_widget_group(s: &mut LogicState, args: Args) -> Key {
    let text = arg::string(args, EXIT_GROUP_TEXT);
    exit_group_via(s, super::lua_args::fit(text, 256));
    EXIT
}

/// `UpdateDeferredViewModelSilent` (0x7f9940): `FUN_007f72f0`(page, `lpcViewModelName`, silent = 1, &out): the load
/// kind 2; the id it leaves in the out value is not used.
fn update_deferred_view_model_silent(s: &mut LogicState, args: Args) -> Key {
    let name = arg::string(args, LPC_VIEW_MODEL_NAME).to_string();
    s.requests.push(Request::UpdateDeferredViewModelSilent { name });
    EXIT
}

/// `HasScreenRegisterdDeferredViewModel` (0x7f9990): `FUN_007f7510`: false while the page changes, else TRUE when
/// one of the widget groups' deferred view-model lists names it (byte comparison).
fn has_screen_registerd_deferred_view_model(s: &mut LogicState, args: Args) -> Key {
    let name = arg::string(args, LPC_VIEW_MODEL_NAME);
    let f = &s.hook_misc;
    exit_if(!f.screen_changing && f.registered_deferred_view_models.contains(name))
}

/// `SetJumpToStoreFlag` (0x6770d0) → `FUN_00675580`: (G+0x2f3fc)+0x2f27 = 1; the binding text read as an int (0
/// when it does not resolve) is the entitlement manager's jump id (`FUN_0086d610`).
fn set_jump_to_store_flag(s: &mut LogicState, args: Args) -> Key {
    s.game_modes.jump_to_store = true;
    let id = match crate::binding::Binding::parse(arg::string(args, JUMP_BINDING)) {
        Ok(b) => {
            let r = b.eval_int(s);
            if r.ok { r.value } else { 0 }
        }
        Err(_) => 0,
    };
    s.hook_misc.entitlement_jump_id = id;
    EXIT
}

/// `ClearEntitlementJumpID` (0x6770a0): the jump id (`FUN_0086d610`) = 0.
fn clear_entitlement_jump_id(s: &mut LogicState, _: Args) -> Key {
    s.hook_misc.entitlement_jump_id = 0;
    EXIT
}

/// `ClearNetworkedTimer` (0x68ae20): message 6 into the networked-timer queue.
fn clear_networked_timer(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::ClearNetworkedTimer);
    EXIT
}

/// `ClearOnlineEasyDrive` (0x676d50): `FUN_006f44e0` on G+0x14c08.
fn clear_online_easy_drive(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::ClearOnlineEasyDrive);
    EXIT
}

/// `RefreshOnlineEasyDrive` (0x676d30): `FUN_00706ec0` on G+0x14c08.
fn refresh_online_easy_drive(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::RefreshOnlineEasyDrive);
    EXIT
}

/// `SetGameVisibilityInviteOnly` (0x676310): G+0x2a9e4 = 2.
fn set_game_visibility_invite_only(s: &mut LogicState, _: Args) -> Key {
    s.hook_misc.game_visibility = 2;
    EXIT
}

/// `SetGameVisibilityJoinable` (0x6762f0): G+0x2a9e4 = 3.
fn set_game_visibility_joinable(s: &mut LogicState, _: Args) -> Key {
    s.hook_misc.game_visibility = 3;
    EXIT
}

/// `SetGameVisibilityPublic` (0x6762d0): G+0x2a9e4 = 4.
fn set_game_visibility_public(s: &mut LogicState, _: Args) -> Key {
    s.hook_misc.game_visibility = 4;
    EXIT
}

/// `SetOnlineRaceCarFrozen` (0x68aea0).
fn set_online_race_car_frozen(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::SetRaceCarFrozen { frozen: true });
    EXIT
}

/// `SetOnlineRaceCarUnfrozen` (0x68af10).
fn set_online_race_car_unfrozen(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::SetRaceCarFrozen { frozen: false });
    EXIT
}
