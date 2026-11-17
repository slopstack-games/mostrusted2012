//! Offline Lua natives of the free-drive / event lookups, the deferred view models, the easy-drive guide, the
//! control settings and the license plate (handlers verified against the dump, research log 2026-11-17 "Lua
//! natives, third batch"). G = `*0x01177868`; the screen manager S = `*0x0117e9f4`, its object `S+0x2020` = G'
//! (the same game-logic object); the options manager OM = `*(G+0x2f3e0)`; the UI manager UIM = `*(G+0x2f480)`.

use mw_lua::{Lua, Value};

use super::lua_args::{int, push_int, string_or_invalid};
use super::{LogicState, Registry, Request, hooks_flow};

type R = mw_lua::Result<Vec<Value>>;

/// One record of the downloads list (G+0x2f42c +0x4d8, 0x604 bytes from +0x600).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Download {
    /// The record's first int.
    pub id: i32,
    /// The view-model name (+8, a 0x?? byte string).
    pub name: String,
    /// +1: the download is a standing entry (kept by `ClearViewModel` with +2 cleared) rather than a pending one
    /// (removed).
    pub standing: bool,
    /// +2.
    pub flag_2: bool,
}

/// The easy-drive guide (UIM+0x47640): a stack of pages the categories were entered through.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct EasyGuide {
    /// +0xac: the depth; the entries (pairs of u32 at +0x5c + 8 × i / +0x60 + 8 × i) below it.
    pub stack: Vec<(u32, u32)>,
    /// +0xd8 / +0xdc: the current category page.
    pub current: (u32, u32),
    /// +0x58 (`StartViewingEasyDrivePages` sets it).
    pub viewing: bool,
    /// What `FUN_007997f0` reads when the guide's source (the object at +0xfc, its +0xb0) has state 2: the first
    /// page's id pair (`(*(+0xb8)+0x28)+0x3c+(+0xbc)`). `None`: the state is not 2. Filled by the runtime.
    pub root_page: Option<(u32, u32)>,
}

/// The control-settings page's state (the settings OM+0x4210, `FUN_00879010` / `FUN_00879c70`).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ControlSettings {
    /// +0x908: the 0x1bed8-byte buffer is allocated (`PrepareControlSettings`; `ReleaseSettings` frees it).
    pub buffer: bool,
    /// The page record `(*(+0x7b0)+0x10)(2)`: +0x18 is nonzero (the controls object, OM+0x4b18).
    pub page_object: bool,
    /// Record +8 (the controls object has more than one entry), +0xc (its entry count − 1), +0x1c (active).
    pub page_8: u32,
    pub page_c: i32,
    pub page_active: bool,
    /// The controls object's +0x410: its entry count (the runtime fills it).
    pub entries: u32,
    /// +0x1bec1 and +0x1bec2 (a bind waits for a press: `StartActionBind` sets both), +0x1beb4, +0x1beb8.
    pub bind_waiting: bool,
    pub bind_action: i32,
    pub bind_arg: i32,
}

/// A license plate of the plate list ((the object at +0x2c)+0x28 data +0x10 handles).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Plate {
    /// The plate's handle (compared with `DAT_01178070` / `DAT_01178074`).
    pub handle: (u32, u32),
    /// Its +0x10 (what `FUN_006e4d10` stores).
    pub value: u32,
}

/// Fields of the managers the natives of this file read and write.
#[derive(Debug, Default)]
pub struct MiscFields {
    /// The loaded view models of G+0x2f42c (names; entries of 0x28 bytes from +0x20, count +0x4d0).
    pub loaded_view_models: Vec<String>,
    /// The ids `UpdateDeferredViewModel` returns, by view-model name: the runtime sets one when it starts the
    /// load that the request asked for (model decision: the original's out value of `FUN_0081d1f0`, not traced).
    pub deferred_ids: std::collections::HashMap<String, i32>,
    pub easy_guide: EasyGuide,
    pub control_settings: ControlSettings,
    /// The license plates (list at `FUN_006ee760` / `FUN_006ee770`) and `DAT_01178070/74`, the current one.
    pub plates: Vec<Plate>,
    pub current_plate: (u32, u32),
    /// The plate value per player slot: record (slot + 2) × 0x1c of `FUN_006e4d10`'s object (slot −1: index 1).
    pub player_plates: [u32; 26],
}

/// `UpdateDeferredViewModel(name)` (0x007f96a0): `lua_tolstring` (`"<invalid>"` when NULL), the out value starts
/// at −1 and `FUN_007f72f0`(name, 1, &out) runs; the value is pushed as a number. The load itself is the
/// runtime's ([`Request::UpdateDeferredViewModel`]); the pushed id is the one it set for that name, else −1.
fn update_deferred_view_model(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let name = string_or_invalid(&args, 1);
    let id = state.misc.deferred_ids.get(&name).copied().unwrap_or(-1);
    state.requests.push(Request::UpdateDeferredViewModel { name });
    Ok(vec![push_int(id)])
}

/// `ClearViewModel(name)` (0x007e9840 → `FUN_0081ed30` on G+0x2f42c): nothing unless a loaded view model has
/// that name (`FUN_0081dfd0`, string compare); then `FUN_0081ec00`: its entry is replaced by the last and the
/// count dropped, and each download of that name is removed when +1 is clear, else its +2 cleared.
fn clear_view_model(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let name = string_or_invalid(&args, 1);
    let Some(i) = state.misc.loaded_view_models.iter().position(|n| *n == name) else {
        return Ok(vec![]);
    };
    state.misc.loaded_view_models.swap_remove(i);
    state.menus.downloads.retain_mut(|d| {
        if d.name != name {
            return true;
        }
        if d.standing {
            d.flag_2 = false;
            true
        } else {
            false
        }
    });
    state.requests.push(Request::ReleaseViewModel { name });
    Ok(vec![])
}

/// `GetCurrentFreedriveEvent()` (0x00690210): `FUN_0068fa10` ([`hooks_flow::pick_freedrive_event`]) pushed as
/// a number (an int: no event is −1).
fn get_current_freedrive_event(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![push_int(hooks_flow::pick_freedrive_event(state) as i32)])
}

/// `FUN_0068fba0`: the next event of the progression: an event of another mode than the free-drive mode
/// (`GameLogicConfig` +0xd8) that is available (`FUN_006e58a0`), listed at G+0x29204 and passes the rollout check
/// (`FUN_0068cc30`); the first pass skips the ones done for the local vehicle (`FUN_006ec6a0`), the second takes
/// any. None: `FUN_0068fa10`'s pick.
pub fn pick_next_event(s: &LogicState) -> u32 {
    let mode = s.config.freedrive_mode;
    for pass in 0..2 {
        for e in &s.game_modes.events {
            if e.mode == mode || !e.available || s.game.listed_events.binary_search(&e.id).is_err() || !e.rollout_ok {
                continue;
            }
            if pass != 0 || !e.completed {
                return e.id;
            }
        }
    }
    hooks_flow::pick_freedrive_event(s)
}

/// `GetNextEventForProgression()` (0x00690240): [`pick_next_event`] pushed as a number.
fn get_next_event_for_progression(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![push_int(pick_next_event(state) as i32)])
}

/// `StartActionBind(action, arg)` (0x007eb200 → `FUN_008790c0` → `FUN_00878790` on OM+0x4210): both arguments
/// `lua_tonumber` + `_ftol2`; +0x1bec1 / +0x1bec2 = 1, +0x1beb4 = action, +0x1beb8 = arg; the rest is the
/// runtime's.
fn start_action_bind(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (action, arg) = (int(&args, 1), int(&args, 2));
    let c = &mut state.misc.control_settings;
    c.bind_waiting = true;
    c.bind_action = action;
    c.bind_arg = arg;
    state.requests.push(Request::StartActionBind { action, arg });
    Ok(vec![])
}

/// `SetGameMode(mode)` (0x007e9990): reads the argument with `lua_tonumber` and drops it (the handler does
/// nothing else).
fn set_game_mode(_: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![])
}

/// `SetDistrict(district)` (0x007e99b0): likewise a no-op.
fn set_district(_: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![])
}

/// `PrepareControlSettings()` (0x007eb140 → `FUN_00879010` on OM+0x4210): the 0x1bed8-byte buffer allocated
/// (its +0x1bec8..+0x1bed4 zeroed), then the page record 2 gets the controls object (OM+0x4b18): +8 = more
/// than one entry, +0xc = entries − 1, +0x1c = 1.
fn prepare_control_settings(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let c = &mut state.misc.control_settings;
    c.buffer = true;
    c.page_object = true;
    c.page_8 = (c.entries > 1) as u32;
    c.page_c = c.entries.wrapping_sub(1) as i32;
    c.page_active = true;
    state.requests.push(Request::PrepareControlSettings);
    Ok(vec![])
}

/// `ReleaseSettings()` (0x007eb170 → `FUN_00879c70`): the page record's +0x18 and +0x1c cleared, the buffer
/// (+0x908) freed and nulled.
fn release_settings(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let c = &mut state.misc.control_settings;
    c.page_object = false;
    c.page_active = false;
    c.buffer = false;
    state.requests.push(Request::ReleaseControlSettings);
    Ok(vec![])
}

/// `GoBackEasyGuideCategory()` (0x00675910 → `FUN_00790a60` on UIM+0x47640): when the depth (+0xac) is above 0
/// it drops by one and the current page (+0xd8 / +0xdc) is the entry now at the top.
fn go_back_easy_guide_category(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let g = &mut state.misc.easy_guide;
    if let Some(top) = g.stack.pop() {
        g.current = top;
    }
    Ok(vec![])
}

/// `StartViewingEasyDrivePages()` (0x00680300): UIM+0x47640 depth (+0xac) = 0, +0x58 = 1, then `FUN_007997f0`:
/// when the guide's source is in state 2 the current page is its first page.
fn start_viewing_easy_drive_pages(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let g = &mut state.misc.easy_guide;
    g.stack.clear();
    g.viewing = true;
    if let Some(page) = g.root_page {
        g.current = page;
    }
    Ok(vec![])
}

/// The Dreamshots camera's control group is bit 4 of the car-control block set's group 0 (G+0x2f0a4 and its
/// count at +0x2f0c0; `this+0x138` of the object G+0x2f474 is G).
const DREAMSHOTS_GROUP: usize = 0;
const DREAMSHOTS_BIT: u32 = 4;

/// `EnableDreamshotsCameraControlGroup()` (0x007ea120 → `FUN_0083d030`): the bit is cleared (counted down when
/// it was set).
fn enable_dreamshots_camera_control_group(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.game.input_blocks.clear(DREAMSHOTS_GROUP, DREAMSHOTS_BIT);
    Ok(vec![])
}

/// `DisableDreamshotsCameraControlGroup()` (0x007ea150 → `FUN_0083d000`): the bit is set (counted up when it
/// was clear).
fn disable_dreamshots_camera_control_group(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.game.input_blocks.set(DREAMSHOTS_GROUP, DREAMSHOTS_BIT);
    Ok(vec![])
}

/// `SetLocalPlayerLicensePlate(index)` (0x006801b0 → `FUN_0067fa20`): `_ftol2` of the argument; for 0 ≤ index <
/// the plate count (`FUN_006ee760`), when that plate's handle differs from the current one (`DAT_01178070/74`)
/// its +0x10 is stored for the local player (`FUN_006e4d10`: record (slot + 2) × 0x1c); the telemetry event
/// "GAMEVNTLICE" / "lici" (`FUN_004c3890`, `FUN_004c2900`, `FUN_004c39f0`) is not ported.
fn set_local_player_license_plate(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let index = int(&args, 1);
    let m = &mut state.misc;
    if index >= 0 && (index as usize) < m.plates.len() {
        let plate = &m.plates[index as usize];
        if plate.handle != m.current_plate {
            let slot = state.players.local.map_or(-1, |s| s as i32);
            if let Some(v) = m.player_plates.get_mut((slot + 2) as usize) {
                *v = plate.value;
            }
        }
    }
    Ok(vec![])
}

/// `ChangeVehicleOnCustomRollout(vehicle)` (0x00692760 → `FUN_00691110` on G+0x2f404): `_ftol2` of the argument.
fn change_vehicle_on_custom_rollout(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::ChangeVehicleOnCustomRollout { vehicle: int(&args, 1) });
    Ok(vec![])
}

pub fn register(r: &mut Registry) {
    r.set_lua("UpdateDeferredViewModel", update_deferred_view_model);
    r.set_lua("ClearViewModel", clear_view_model);
    r.set_lua("GetCurrentFreedriveEvent", get_current_freedrive_event);
    r.set_lua("GetNextEventForProgression", get_next_event_for_progression);
    r.set_lua("StartActionBind", start_action_bind);
    r.set_lua("SetGameMode", set_game_mode);
    r.set_lua("SetDistrict", set_district);
    r.set_lua("PrepareControlSettings", prepare_control_settings);
    r.set_lua("ReleaseSettings", release_settings);
    r.set_lua("GoBackEasyGuideCategory", go_back_easy_guide_category);
    r.set_lua("StartViewingEasyDrivePages", start_viewing_easy_drive_pages);
    r.set_lua("EnableDreamshotsCameraControlGroup", enable_dreamshots_camera_control_group);
    r.set_lua("DisableDreamshotsCameraControlGroup", disable_dreamshots_camera_control_group);
    r.set_lua("SetLocalPlayerLicensePlate", set_local_player_license_plate);
    r.set_lua("ChangeVehicleOnCustomRollout", change_vehicle_on_custom_rollout);
}

#[cfg(test)]
mod tests {
    use super::super::game_modes::EventInfo;
    use super::super::{LogicHost, LogicState};
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

    fn num(r: &[Value]) -> f64 {
        match r.first() {
            Some(Value::Number(n)) => *n,
            other => panic!("not a number: {other:?}"),
        }
    }

    fn ev(id: u32, mode: u32, completed: bool) -> EventInfo {
        EventInfo { id, mode, available: true, rollout_ok: true, completed, ..Default::default() }
    }

    fn st(host: &LogicHost) -> std::cell::RefMut<'_, LogicState> {
        host.state.borrow_mut()
    }

    #[test]
    fn freedrive_and_next_event() {
        let (mut lua, host) = setup();
        assert_eq!(num(&call(&mut lua, "GetCurrentFreedriveEvent", vec![])), -1.0, "no event: 0xffffffff");
        assert_eq!(num(&call(&mut lua, "GetNextEventForProgression", vec![])), -1.0, "falls back to the same");
        {
            let mut s = st(&host);
            s.config.freedrive_mode = 7;
            s.game.listed_events = vec![10, 20, 30];
            s.game_modes.events = vec![ev(10, 7, false), ev(20, 3, true), ev(30, 3, false), ev(40, 3, false)];
        }
        assert_eq!(num(&call(&mut lua, "GetCurrentFreedriveEvent", vec![])), 10.0);
        // Pass 1 skips the completed 20; 40 is not listed.
        assert_eq!(num(&call(&mut lua, "GetNextEventForProgression", vec![])), 30.0);
        st(&host).game_modes.events[2].completed = true;
        assert_eq!(num(&call(&mut lua, "GetNextEventForProgression", vec![])), 20.0, "pass 2 takes any");
        st(&host).game_modes.events[1].rollout_ok = false;
        st(&host).game_modes.events[2].available = false;
        assert_eq!(num(&call(&mut lua, "GetNextEventForProgression", vec![])), 10.0, "the free-drive pick");
    }

    #[test]
    fn deferred_view_models() {
        let (mut lua, host) = setup();
        let r = call(&mut lua, "UpdateDeferredViewModel", vec![Value::from("Garage")]);
        assert_eq!(num(&r), -1.0);
        assert_eq!(host.state.borrow().requests, [Request::UpdateDeferredViewModel { name: "Garage".into() }]);
        st(&host).misc.deferred_ids.insert("Garage".into(), 5);
        assert_eq!(num(&call(&mut lua, "UpdateDeferredViewModel", vec![Value::from("Garage")])), 5.0);
        // A non-string argument is "<invalid>".
        call(&mut lua, "UpdateDeferredViewModel", vec![Value::Bool(true)]);
        assert_eq!(
            host.state.borrow().requests.last(),
            Some(&Request::UpdateDeferredViewModel { name: "<invalid>".into() })
        );

        {
            let mut s = st(&host);
            s.misc.loaded_view_models = vec!["A".into(), "B".into(), "C".into()];
            let d = |id, name: &str, standing| Download { id, name: name.into(), standing, flag_2: true };
            s.menus.downloads = vec![d(1, "B", false), d(2, "B", true), d(3, "C", false)];
            s.requests.clear();
        }
        call(&mut lua, "ClearViewModel", vec![Value::from("Z")]);
        assert!(host.state.borrow().requests.is_empty(), "unknown name: nothing");
        call(&mut lua, "ClearViewModel", vec![Value::from("A")]);
        let s = host.state.borrow();
        assert_eq!(s.misc.loaded_view_models, ["C", "B"], "the last takes the place");
        assert_eq!(s.menus.downloads.len(), 3);
        drop(s);
        call(&mut lua, "ClearViewModel", vec![Value::from("B")]);
        let s = host.state.borrow();
        assert_eq!(s.misc.loaded_view_models, ["C"]);
        assert_eq!(s.menus.downloads.iter().map(|d| (d.id, d.flag_2)).collect::<Vec<_>>(), [(2, false), (3, true)]);
        assert_eq!(s.requests.last(), Some(&Request::ReleaseViewModel { name: "B".into() }));
    }

    #[test]
    fn controls_and_easy_guide() {
        let (mut lua, host) = setup();
        call(&mut lua, "SetGameMode", vec![Value::Number(2.0)]);
        call(&mut lua, "SetDistrict", vec![]);
        assert!(host.state.borrow().requests.is_empty());
        call(&mut lua, "StartActionBind", vec![Value::Number(3.7), Value::Number(-1.2)]);
        {
            let s = host.state.borrow();
            let c = &s.misc.control_settings;
            assert!(c.bind_waiting);
            assert_eq!((c.bind_action, c.bind_arg), (3, -1));
            assert_eq!(s.requests, [Request::StartActionBind { action: 3, arg: -1 }]);
        }
        st(&host).misc.control_settings.entries = 5;
        call(&mut lua, "PrepareControlSettings", vec![]);
        {
            let s = host.state.borrow();
            let c = &s.misc.control_settings;
            assert!(c.buffer && c.page_object && c.page_active);
            assert_eq!((c.page_8, c.page_c), (1, 4));
        }
        call(&mut lua, "ReleaseSettings", vec![]);
        {
            let s = host.state.borrow();
            let c = &s.misc.control_settings;
            assert!(!c.buffer && !c.page_object && !c.page_active);
            assert_eq!(s.requests.last(), Some(&Request::ReleaseControlSettings));
        }
        // Easy guide: back pops to the entry now on top; start viewing resets to the first page.
        st(&host).misc.easy_guide.stack = vec![(1, 2), (3, 4)];
        call(&mut lua, "GoBackEasyGuideCategory", vec![]);
        assert_eq!(host.state.borrow().misc.easy_guide.current, (3, 4));
        call(&mut lua, "GoBackEasyGuideCategory", vec![]);
        call(&mut lua, "GoBackEasyGuideCategory", vec![]);
        assert_eq!(host.state.borrow().misc.easy_guide.current, (1, 2), "empty: unchanged");
        st(&host).misc.easy_guide.stack = vec![(9, 9)];
        call(&mut lua, "StartViewingEasyDrivePages", vec![]);
        {
            let s = host.state.borrow();
            assert!(s.misc.easy_guide.stack.is_empty() && s.misc.easy_guide.viewing);
            assert_eq!(s.misc.easy_guide.current, (1, 2), "source not in state 2");
        }
        st(&host).misc.easy_guide.root_page = Some((7, 8));
        call(&mut lua, "StartViewingEasyDrivePages", vec![]);
        assert_eq!(host.state.borrow().misc.easy_guide.current, (7, 8));
        // Dreamshots: bit 4 of group 0, counted.
        st(&host).game.input_blocks.masks[0] = 0x10;
        st(&host).game.input_blocks.counts[0] = 1;
        call(&mut lua, "EnableDreamshotsCameraControlGroup", vec![]);
        assert_eq!(
            (host.state.borrow().game.input_blocks.masks[0], host.state.borrow().game.input_blocks.counts[0]),
            (0, 0)
        );
        call(&mut lua, "DisableDreamshotsCameraControlGroup", vec![]);
        call(&mut lua, "DisableDreamshotsCameraControlGroup", vec![]);
        assert_eq!(
            (host.state.borrow().game.input_blocks.masks[0], host.state.borrow().game.input_blocks.counts[0]),
            (0x10, 1)
        );
    }

    #[test]
    fn license_plate_and_rollout() {
        let (mut lua, host) = setup();
        {
            let mut s = st(&host);
            s.misc.plates = vec![Plate { handle: (1, 1), value: 11 }, Plate { handle: (1, 2), value: 22 }];
            s.misc.current_plate = (1, 1);
            s.players.local = Some(0);
        }
        call(&mut lua, "SetLocalPlayerLicensePlate", vec![Value::Number(0.0)]);
        assert_eq!(host.state.borrow().misc.player_plates[2], 0, "the current plate: nothing");
        call(&mut lua, "SetLocalPlayerLicensePlate", vec![Value::Number(5.0)]);
        call(&mut lua, "SetLocalPlayerLicensePlate", vec![Value::Number(-1.0)]);
        assert_eq!(host.state.borrow().misc.player_plates[2], 0, "out of range");
        call(&mut lua, "SetLocalPlayerLicensePlate", vec![Value::Number(1.9)]);
        assert_eq!(host.state.borrow().misc.player_plates[2], 22);
        call(&mut lua, "ChangeVehicleOnCustomRollout", vec![Value::Number(42.5)]);
        assert_eq!(host.state.borrow().requests, [Request::ChangeVehicleOnCustomRollout { vehicle: 42 }]);
        assert!(host.state.borrow().unported.is_empty());
    }
}
