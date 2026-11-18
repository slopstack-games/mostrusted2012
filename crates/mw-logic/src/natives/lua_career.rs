//! Lua natives of the single-player career that only store or read a value of the game's managers (the ones
//! that rebuild lists, start events or change the profile are not ported yet: research log 2026-11-17 "Lua
//! natives"). G = `*0x01177868`; GMM = G+0x2f3fc (game-mode manager); UIM = G+0x2f480.

use mw_lua::{Lua, Value};

use super::hook_args::strtol10;
use super::hooks_online::select_dom;
use super::hooks_playlist::strtoll10;
use super::lua_args::{int, push_int, string_or_invalid, toboolean, tonumber};
use super::state::Request;
use super::{LogicState, Registry};

type R = mw_lua::Result<Vec<Value>>;

/// UIM (G+0x2f480), the fields the Lua natives write.
#[derive(Debug, Default)]
pub struct UiManagerFields {
    /// +0x2c8e4.
    pub target_offline_event: i32,
    /// +0x47698 (byte; the Easy Guide object UIM+0x47640, +0x58): pages being viewed.
    pub viewing_easy_drive_pages: bool,
    /// +0x47730 (Easy Guide +0xf0).
    pub requested_easy_guide_content: i32,
}

/// G+0x2f418 (its name is not known): the challenge vehicle set and the driving-test list group.
#[derive(Debug, Default)]
pub struct ChallengeFields {
    /// +0x334.
    pub challenge_vehicle_set: i32,
    /// +0x33c.
    pub driving_test_list_group: i32,
}

/// `SetOfflineEventToPlay(id)` (0x006799d0 → `FUN_0063e380(id, false)` on the object at G+0x2f4c8: +0x230 id, +0x234 −1, +0x238 flag).
fn set_offline_event_to_play(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let g = &mut state.game;
    g.pending_event = int(&args, 1) as u32;
    g.pending_event_aux = -1;
    g.pending_event_flag = false;
    Ok(vec![])
}

/// `SetTargetOfflineEvent(id)` (0x00679a10): UIM+0x2c8e4.
fn set_target_offline_event(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.ui_manager.target_offline_event = int(&args, 1);
    Ok(vec![])
}

/// `SetStoryMode(on)` (0x00679c90): GMM+0x2e7a.
fn set_story_mode(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.game_modes.story_mode = toboolean(&args, 1);
    Ok(vec![])
}

/// `GetChallengeVehicleSet()` (0x00675940): (G+0x2f418)+0x334 as a signed int.
fn get_challenge_vehicle_set(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![push_int(state.challenges.challenge_vehicle_set)])
}

/// `SelectDrivingTestListGroup(group)` (0x00679d20): (G+0x2f418)+0x33c.
fn select_driving_test_list_group(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.challenges.driving_test_list_group = int(&args, 1);
    Ok(vec![])
}

/// `SetRequestedEasyGuideContent(content)` (0x0067a2a0): UIM+0x47730.
fn set_requested_easy_guide_content(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.ui_manager.requested_easy_guide_content = int(&args, 1);
    Ok(vec![])
}

/// `StopViewingEasyDrivePages()` (0x006758e0): UIM+0x47698 = 0, UIM+0x47730 = 0.
fn stop_viewing_easy_drive_pages(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.ui_manager.viewing_easy_drive_pages = false;
    state.ui_manager.requested_easy_guide_content = 0;
    Ok(vec![])
}

/// `GetCurrentSafeHouse()` (0x00675770): always 0.
fn get_current_safe_house(_: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Number(0.0)])
}

/// `SetSelectedVehicleId` (0x00679f00), `SetSafeHouse` (0x00679ee0), `SetTargetRouteFromSafehouse` (0x00679f70):
/// `lua_tonumber(L, 1)`, the result dropped (`fstp st(0)`); nothing else.
fn read_one_number(_: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let _ = tonumber(&args, 1);
    Ok(vec![])
}

/// `ChangePerkOnCustomRollout` (0x00679c30): reads arguments 1 and 2 as numbers, nothing else.
fn change_perk_on_custom_rollout(_: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let _ = (tonumber(&args, 1), tonumber(&args, 2));
    Ok(vec![])
}

pub fn register(r: &mut Registry) {
    r.set_lua("SetOfflineEventToPlay", set_offline_event_to_play);
    r.set_lua("SetTargetOfflineEvent", set_target_offline_event);
    r.set_lua("SetStoryMode", set_story_mode);
    r.set_lua("GetChallengeVehicleSet", get_challenge_vehicle_set);
    r.set_lua("SelectDrivingTestListGroup", select_driving_test_list_group);
    r.set_lua("SetRequestedEasyGuideContent", set_requested_easy_guide_content);
    r.set_lua("StopViewingEasyDrivePages", stop_viewing_easy_drive_pages);
    r.set_lua("GetCurrentSafeHouse", get_current_safe_house);
    r.set_lua("SetSelectedVehicleId", read_one_number);
    r.set_lua("SetSafeHouse", read_one_number);
    r.set_lua("SetTargetRouteFromSafehouse", read_one_number);
    r.set_lua("ChangePerkOnCustomRollout", change_perk_on_custom_rollout);
    register_more(r);
}
/// The rollout manager's records (G+0x2f404 + slot x 0xd8 + 0x827c) and the online object's fields.
#[derive(Debug, Default)]
pub struct CareerFields {
    /// Record +0x30 / +0x34 of each of the 12 slots: the selected vehicle id and colour id (the runtime fills them
    /// when the rollout manager applies a rollout).
    pub selected_vehicle: [i32; 12],
    pub selected_colour: [i32; 12],
    /// Rollout manager +0x8f90 (`SelectDefaultRollout`; +0x8f8c is cleared).
    pub default_rollout: i32,
    pub default_rollout_flag: i32,
    /// Online object (+0x2020 of `*0x0117e9f4`) +0x2f6d0 (64 bit): the recommendation's time to beat.
    pub recommendation_time_to_beat: u64,
}

/// `SetEventFromAvailableVehicles()` (0x006756b0): `FUN_00673d70`.
fn set_event_from_available_vehicles(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::SetEventFromAvailableVehicles);
    Ok(vec![])
}

/// `SetTargetRecommendedEvent(str)` (0x00679a50): `FUN_00725130`(strtol(str), 0, 0); returns whether the
/// recommendations exist (as the hook of the same name).
fn set_target_recommended_event(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let index = strtol10(&string_or_invalid(&args, 1));
    let f = &mut state.hook_online;
    if !f.has_recommendations {
        return Ok(vec![Value::Bool(false)]);
    }
    f.target_index = index;
    f.target_param = 0;
    state.requests.push(Request::SetRecommendedEvent { index, param: 0 });
    Ok(vec![Value::Bool(true)])
}

/// `SetCurrentRecommendedEvent()` (0x00675680): `FUN_00726c80` (as the hook), no result.
fn set_current_recommended_event(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let f = &mut state.hook_online;
    f.current_index = f.target_index;
    if f.has_recommendations {
        let (index, param) = (f.target_index, f.target_param);
        state.requests.push(Request::SetRecommendedEvent { index, param });
    }
    Ok(vec![])
}

/// `SetCarListForRecommendedEvent(str)` (0x00679ac0): strtol above 0 and `FUN_00723300` (as the hook); boolean.
fn set_car_list_for_recommended_event(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let n = strtol10(&string_or_invalid(&args, 1));
    let f = &mut state.hook_online;
    if n <= 0 || !f.has_recommendations {
        return Ok(vec![Value::Bool(false)]);
    }
    let event_id = f.event_ids.get(n as usize).copied().unwrap_or(0);
    f.car_list_event = event_id;
    state.requests.push(Request::SetCarListForEvent { event_id });
    Ok(vec![Value::Bool(true)])
}

/// `SelectRecommendationDom(index)` (0x00679b70): `FUN_00724ef0`(index, 2) on (G+0x2f454)+0x5a3c.
fn select_recommendation_dom(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    select_dom(state, int(&args, 1), 2);
    Ok(vec![])
}

/// `SetFriendInviteId(str)` (0x0067a250): `strtoull(str or "<invalid>", 0, 10)` into G+0x2f498 (64 bit).
fn set_friend_invite_id(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.hook_playlist.friend_invite_id = strtoll10(&string_or_invalid(&args, 1));
    Ok(vec![])
}

/// `SetJoinPlayerId(str)` (0x007e99d0): the id into +0x2f4b0 and +0x2f4b8, +0x2f4ac = 1.
fn set_join_player_id(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = strtoll10(&string_or_invalid(&args, 1));
    let f = &mut state.hook_playlist;
    f.join_friend_id = id;
    f.join_friend_id_copy = id;
    f.join_friend_set = true;
    Ok(vec![])
}

/// `SetFriendCarouselViewModelActive(bool)` (0x007e9880): (G+0x2f3e0) bytes +0x14 and +0x15 = the flag (only
/// +0x14 is modelled, as the hooks do).
fn set_friend_carousel_view_model_active(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.hook_playlist.friend_carousel_active = toboolean(&args, 1);
    Ok(vec![])
}

/// `SetRecommendationTimeToBeat(n)` (0x007ee900): `n` (sign-extended) x 3000 (`__allmul`) into +0x2f6d0 (64 bit).
fn set_recommendation_time_to_beat(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.career.recommendation_time_to_beat = (i64::from(int(&args, 1)) * 3000) as u64;
    Ok(vec![])
}

/// `ClearEntitlementJumpID()` (0x006759f0): `FUN_0086d610`(0) on (G+0x2f3e0)+0x4ce0.
fn clear_entitlement_jump_id(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.hook_misc.entitlement_jump_id = 0;
    Ok(vec![])
}

/// `HasEasyDriveOptionBeenSeen(id)` (0x0067a380): `FUN_0072eb90`; mirror of the rows as the hook.
fn has_easy_drive_option_been_seen(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let seen = state.select.easydrive_options_seen.get(&int(&args, 1)).copied().unwrap_or(false);
    Ok(vec![Value::Bool(seen)])
}

/// `SetEasyDriveOptionSeen(id, seen)` (0x0067a3d0): `FUN_00674d20`(id, seen) (as the hook).
fn set_easy_drive_option_seen(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let option = int(&args, 1);
    let seen = toboolean(&args, 2);
    state.select.easydrive_options_seen.insert(option, seen);
    state.requests.push(Request::SetEasyDriveOptionSeen { option, seen });
    Ok(vec![])
}

/// `StoreTotalSP(n)` (0x007ea700).
fn store_total_sp(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::StoreTotalSp { sp: int(&args, 1) });
    Ok(vec![])
}

/// `MarkStoreItemAsSeen(n)` (0x007ea740).
fn mark_store_item_as_seen(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::MarkStoreItemAsSeen { item: int(&args, 1) });
    Ok(vec![])
}

/// `UpdateCachedMostWantedList(str)` (0x007e9a50): the string is read, `FUN_0071dfe0()` takes no argument.
fn update_cached_most_wanted_list(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::UpdateCachedMostWantedList);
    Ok(vec![])
}

/// `GrantCloudCompeteAward(name, amount)` (0x007ea1c0).
fn grant_cloud_compete_award(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let name = string_or_invalid(&args, 1);
    state.requests.push(Request::GrantCloudCompeteAward { name, amount: int(&args, 2) });
    Ok(vec![])
}

/// `GrantAwardForSpeedwallMovement(a, b)` (0x006801e0): both arguments are checked as numbers and dropped.
fn grant_award_for_speedwall_movement(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::GrantAwardForSpeedwallMovement);
    Ok(vec![])
}

/// `TryToGiveAwardForBeingFirstInSpeedwall(a)` (0x00680250).
fn try_to_give_award_first_in_speedwall(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::TryToGiveAwardFirstInSpeedwall);
    Ok(vec![])
}

/// `LoadOnlineSeenUnlocks(n)` (0x0067a5b0).
fn load_online_seen_unlocks(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::LoadOnlineSeenUnlocks { arg: int(&args, 1) });
    Ok(vec![])
}

/// `SaveOnlineSeenUnlocks(n)` (0x0067a5f0).
fn save_online_seen_unlocks(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::SaveOnlineSeenUnlocks { arg: int(&args, 1) });
    Ok(vec![])
}

/// `RemoveOnlineSeenUnlocksMenu(x)` (0x0067a630): the argument is checked and dropped.
fn remove_online_seen_unlocks_menu(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::RemoveOnlineSeenUnlocksMenu);
    Ok(vec![])
}

/// `SetOnlineUnlockToSeen(unlock, value)` (0x0067a660).
fn set_online_unlock_to_seen(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::SetOnlineUnlockToSeen { unlock: int(&args, 1), value: int(&args, 2) });
    Ok(vec![])
}

pub fn register_more(r: &mut Registry) {
    r.set_lua("SetEventFromAvailableVehicles", set_event_from_available_vehicles);
    r.set_lua("SetTargetRecommendedEvent", set_target_recommended_event);
    r.set_lua("SetCurrentRecommendedEvent", set_current_recommended_event);
    r.set_lua("SetCarListForRecommendedEvent", set_car_list_for_recommended_event);
    r.set_lua("SelectRecommendationDom", select_recommendation_dom);
    r.set_lua("SetFriendInviteId", set_friend_invite_id);
    r.set_lua("SetJoinPlayerId", set_join_player_id);
    r.set_lua("SetFriendCarouselViewModelActive", set_friend_carousel_view_model_active);
    r.set_lua("SetRecommendationTimeToBeat", set_recommendation_time_to_beat);
    r.set_lua("ClearEntitlementJumpID", clear_entitlement_jump_id);
    r.set_lua("HasEasyDriveOptionBeenSeen", has_easy_drive_option_been_seen);
    r.set_lua("SetEasyDriveOptionSeen", set_easy_drive_option_seen);
    r.set_lua("StoreTotalSP", store_total_sp);
    r.set_lua("MarkStoreItemAsSeen", mark_store_item_as_seen);
    r.set_lua("UpdateCachedMostWantedList", update_cached_most_wanted_list);
    r.set_lua("GrantCloudCompeteAward", grant_cloud_compete_award);
    r.set_lua("GrantAwardForSpeedwallMovement", grant_award_for_speedwall_movement);
    r.set_lua("TryToGiveAwardForBeingFirstInSpeedwall", try_to_give_award_first_in_speedwall);
    r.set_lua("LoadOnlineSeenUnlocks", load_online_seen_unlocks);
    r.set_lua("SaveOnlineSeenUnlocks", save_online_seen_unlocks);
    r.set_lua("RemoveOnlineSeenUnlocksMenu", remove_online_seen_unlocks_menu);
    r.set_lua("SetOnlineUnlockToSeen", set_online_unlock_to_seen);
}

#[cfg(test)]
mod tests {
    use super::super::LogicHost;
    use super::super::lua_args::push_str;
    use super::*;

    fn setup() -> (Lua, LogicHost) {
        let mut lua = Lua::new();
        let host = LogicHost::default();
        host.install_lua(&mut lua);
        (lua, host)
    }

    fn call(lua: &mut Lua, name: &str, args: Vec<Value>) -> Vec<Value> {
        lua.call_global(name, args).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    #[test]
    fn career_natives() {
        let (mut lua, host) = setup();
        let n = Value::Number;
        call(&mut lua, "SetRecommendationTimeToBeat", vec![n(-2.0)]);
        assert_eq!(host.state.borrow().career.recommendation_time_to_beat as i64, -6000);
        call(&mut lua, "SetFriendInviteId", vec![push_str("12345678901234")]);
        assert_eq!(host.state.borrow().hook_playlist.friend_invite_id, 12_345_678_901_234);
        call(&mut lua, "SetJoinPlayerId", vec![push_str("42")]);
        assert!(host.state.borrow().hook_playlist.join_friend_set);
        assert_eq!(host.state.borrow().hook_playlist.join_friend_id_copy, 42);
        let r = call(&mut lua, "SetTargetRecommendedEvent", vec![push_str("3")]);
        assert!(matches!(r[0], Value::Bool(false)), "no recommendations: refused");
        host.state.borrow_mut().hook_online.has_recommendations = true;
        let r = call(&mut lua, "SetTargetRecommendedEvent", vec![push_str("3")]);
        assert!(matches!(r[0], Value::Bool(true)));
        assert_eq!(host.state.borrow().hook_online.target_index, 3);
        call(&mut lua, "SetEasyDriveOptionSeen", vec![n(4.0), Value::Bool(true)]);
        let r = call(&mut lua, "HasEasyDriveOptionBeenSeen", vec![n(4.0)]);
        assert!(matches!(r[0], Value::Bool(true)));
        call(&mut lua, "StoreTotalSP", vec![n(5.0)]);
        let s = host.state.borrow();
        assert!(s.requests.contains(&Request::StoreTotalSp { sp: 5 }));
    }
}
