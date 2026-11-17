//! Lua natives of the single-player career that only store or read a value of the game's managers (the ones
//! that rebuild lists, start events or change the profile are not ported yet: research log 2026-11-17 "Lua
//! natives"). G = `*0x01177868`; GMM = G+0x2f3fc (game-mode manager); UIM = G+0x2f480.

use mw_lua::{Lua, Value};

use super::lua_args::{int, push_int, toboolean, tonumber};
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

/// `SetOfflineEventToPlay(id)` (0x006799d0 → `FUN_0063e380(id, false)` on G).
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
}
