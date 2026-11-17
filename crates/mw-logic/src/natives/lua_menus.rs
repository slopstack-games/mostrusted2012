//! Lua natives of the front-end menus that store a value of a manager and leave the rebuilding of lists to the
//! engine ([`Request`]): vehicle category / colour / modifier, challenge vehicle set, pack selection, the
//! platform sign-in state, pending downloads, award progress, new game. Handlers verified against the dump
//! (research log 2026-11-17). G = `*0x01177868`; the options manager OM = `*(G+0x2f3e0)`.

use mw_lua::{Lua, Value};

use super::lua_args::{int, toboolean};
use super::{LogicState, Registry, Request, hooks_flow};

type R = mw_lua::Result<Vec<Value>>;

/// The platform sign-in manager (OM+0x10d0), as far as `SignInStarted` / `SignInFinished` touch it.
#[derive(Debug, Default, PartialEq)]
pub struct SignInManager {
    /// +0x1588: a sign-in is running.
    pub active: bool,
    /// +0x158a (`SignInStarted` passes 1, `SignInFinished` clears it).
    pub flag: bool,
    /// +0x135c.
    pub state_135c: u32,
    /// +0x1360 (-1 at the start of a sign-in).
    pub state_1360: i32,
    /// +0x139c.
    pub flag_139c: bool,
    /// +0x136d.
    pub flag_136d: bool,
    /// +0x1370 (count u16, entries released one by one by `FUN_00884340`, then the buffer freed): the friends /
    /// profile lists fetched online; their entries are the runtime's, only their number is kept.
    pub list_1370: usize,
    /// +0x1380, the same.
    pub list_1380: usize,
}

/// Fields of the menus' managers the natives in this file write or read.
#[derive(Debug, Default)]
pub struct MenuFields {
    pub sign_in: SignInManager,
    /// The downloads in progress (the manager at G+0x2f42c, +0x4d8: records of 0x604 bytes from +0x600, count
    /// +0xb478; the id is the record's first int). Empty offline.
    pub downloads: Vec<i32>,
    /// The rollout manager G+0x2f404, +0x1bc: the vehicle category the list shows.
    pub vehicle_category: i32,
    /// The pack page (OM+0x4ce0): the pack ids from +0x44 + 4*index (index 0 is "none"; the original reads the
    /// object behind the pointer, here its id).
    pub packs: Vec<u32>,
    /// The selected pack's id (`FUN_00642a90` on OM+0x4ce0 +0xd74; 0 = none).
    pub selected_pack: u32,
    /// +0xdce.
    pub pack_changed: bool,
}

/// `SetNewGame(new)` (0x006902a0): `FUN_0068fd50(new != 0)`, the hook `SetNewGame`'s function.
fn set_new_game(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    hooks_flow::set_new_game(state, toboolean(&args, 1));
    Ok(vec![])
}

/// `SelectChallengeVehicleSet(set)` (0x00679cd0): (G+0x2f418)+0x334, then `FUN_006f3f90` on G+0x2f41c (counts
/// the list entries of the challenge page for that set).
fn select_challenge_vehicle_set(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.challenges.challenge_vehicle_set = int(&args, 1);
    state.requests.push(Request::RecountChallengeVehicles);
    Ok(vec![])
}

/// `SelectVehicleCategory(category)` (0x006902d0): (G+0x2f404)+0x1bc, then `FUN_0068b7c0` (the vehicle list is
/// rebuilt for it).
fn select_vehicle_category(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.menus.vehicle_category = int(&args, 1);
    state.requests.push(Request::RebuildVehicleList);
    Ok(vec![])
}

/// `SetSelectedVehicleColour(colour)` (0x00679ff0 → `FUN_0086bc60` → `FUN_00657b50` on OM+0x4c90).
fn set_selected_vehicle_colour(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::SetSelectedVehicleColour { colour: int(&args, 1) });
    Ok(vec![])
}

/// `SetVehicleModifier(kind, item)` (0x00679f20 → `FUN_0086bc80` → `FUN_00657d10` on OM+0x4c90).
fn set_vehicle_modifier(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::SetVehicleModifier { kind: int(&args, 1), item: int(&args, 2) });
    Ok(vec![])
}

/// `SignInStarted()` (0x007eac10 → `FUN_00881f10(1)` on OM+0x10d0).
fn sign_in_started(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let s = &mut state.menus.sign_in;
    s.flag = true;
    s.active = true;
    s.state_135c = 0;
    s.state_1360 = -1;
    s.flag_139c = false;
    s.flag_136d = false;
    Ok(vec![])
}

/// `SignInFinished()` (0x007eac40 → `FUN_00886f00` on OM+0x10d0): both fetched lists released and emptied, the
/// two flags cleared.
fn sign_in_finished(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let s = &mut state.menus.sign_in;
    s.list_1370 = 0;
    s.list_1380 = 0;
    s.flag = false;
    s.active = false;
    Ok(vec![])
}

/// `SelectPack(index)` (0x007ea480 → `FUN_008739f0` on OM+0x4ce0): the pack's id (0 for index < 1) selected
/// (`FUN_00642a90`: 0 selects the default, an id is looked up among the packs), +0xdce = 1. Model decision: an
/// index past the page's packs (the original reads past the array) selects none.
fn select_pack(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let index = int(&args, 1);
    let id = if index < 1 { 0 } else { state.menus.packs.get(index as usize).copied().unwrap_or(0) };
    state.menus.selected_pack = id;
    state.menus.pack_changed = true;
    state.requests.push(Request::SelectPack { id });
    Ok(vec![])
}

/// `IsViewModelDownloadComplete(id)` (0x007ee5d0 → `FUN_0081d1b0` on G+0x2f42c): false while a download with
/// that id is in the list.
fn is_view_model_download_complete(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = int(&args, 1);
    Ok(vec![Value::Bool(!state.menus.downloads.contains(&id))])
}

/// `UpdateAwardProgressBindings()` (0x007e9f40): `FUN_00724680` and `FUN_00724690` on G+0x2f3f8, each
/// `FUN_0071fd20` (all awards at +0x10e8, category 6 at +0x12e8).
fn update_award_progress_bindings(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::UpdateAwardProgressBindings);
    Ok(vec![])
}

pub fn register(r: &mut Registry) {
    r.set_lua("SetNewGame", set_new_game);
    r.set_lua("SelectChallengeVehicleSet", select_challenge_vehicle_set);
    r.set_lua("SelectVehicleCategory", select_vehicle_category);
    r.set_lua("SetSelectedVehicleColour", set_selected_vehicle_colour);
    r.set_lua("SetVehicleModifier", set_vehicle_modifier);
    r.set_lua("SignInStarted", sign_in_started);
    r.set_lua("SignInFinished", sign_in_finished);
    r.set_lua("SelectPack", select_pack);
    r.set_lua("IsViewModelDownloadComplete", is_view_model_download_complete);
    r.set_lua("UpdateAwardProgressBindings", update_award_progress_bindings);
}
