//! Offline Lua natives of the feedback / sound / selection groups (handlers read from the exe's code,
//! `re/out/live/text.asm`; research log 2026-11-18, ninth batch). G = `*0x01177870`, UIM = `*(G+0x2f480)`, the
//! selection record = `*(G+0x2f404)`, PM = `*(G+0x2f3dc)`. Natives that act on the sound / feedback / message
//! systems record a [`Request`]; the fields they read are set by the runtime ([`FeedbackFields`]).

use mw_lua::{Lua, Value};

use super::lua_args::int;
use super::{LogicState, Registry, Request};

type R = mw_lua::Result<Vec<Value>>;

/// One slot of the selection record (`*(G+0x2f404)` + `0x827c` + slot x `0xd8`): +0x30 / +0x34.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct SelectedCar {
    pub vehicle_id: i32,
    pub colour_id: i32,
}

/// One of the 7 feedback group entries (UIM+0x32858 + 0xb0 + n x 0xc4): id +0, state +4 (3 = loaded).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct FeedbackGroup {
    pub id: i32,
    pub state: i32,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct FeedbackFields {
    /// The twelve slots of the selection record (`GetSelectedVehicleId` / `GetSelectedColourId`).
    pub selected: [SelectedCar; 12],
    /// Selection record +0x8f90 / +0x8f8c (`SelectDefaultRollout`: id, then 0).
    pub default_rollout: i32,
    pub rollout_8f8c: i32,
    /// The road table (G+0x153a8 base +0x28, G+0x153ac offset): the int at +0xc (`GetRoadCount`).
    pub road_count: i32,
    /// UIM+0x30a5c: the feedback sequence's handle (nonzero = a sequence exists).
    pub sequence_handle: u32,
    /// UIM+0x30868: the sequence's load state (1 = loaded).
    pub sequence_state: i32,
    /// UIM+0x316e1: set by `StartFeedbackSequence`.
    pub sequence_start: bool,
    /// UIM+0x32858 + 0xb0: the feedback group table (7 entries, state 0 = free).
    pub groups: [FeedbackGroup; 7],
    /// The race manager (G+0x2f47c): its vtable +0xc call says a race runs, and the float at
    /// `+0x510 x 20 + 0xc` (`GetRaceDistance`).
    pub race_active: bool,
    pub race_distance: f32,
}

/// The local slot's selection record (PM+0xa6b4 <= 0xb), else the null record the original reads (0).
fn selected(state: &LogicState) -> SelectedCar {
    match state.players.local {
        Some(i) if i <= 0xb => state.sim.feedback.selected[i],
        _ => SelectedCar::default(),
    }
}

/// `GetSelectedVehicleId()` (0x00688380): the int at +0x30 of the local slot's selection record.
fn get_selected_vehicle_id(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Number(f64::from(selected(state).vehicle_id))])
}

/// `GetSelectedColourId()` (0x006883e0): the int at +0x34.
fn get_selected_colour_id(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Number(f64::from(selected(state).colour_id))])
}

/// `GetRoadCount()` (0x00680c60): the int at +0xc of the road table.
fn get_road_count(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Number(f64::from(state.sim.feedback.road_count))])
}

/// `IsFeedbackSequenceLoaded()` (0x0067de00): UIM+0x30a5c != 0 and UIM+0x30868 == 1.
fn is_feedback_sequence_loaded(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let f = &state.sim.feedback;
    Ok(vec![Value::Bool(f.sequence_handle != 0 && f.sequence_state == 1)])
}

/// `IsFeedbackGroupLoaded(id)` (0x0067df00 -> `FUN_00792590`): the first of the 7 entries in use (state != 0) whose id
/// matches decides: loaded when its state is 3; no match is false.
fn is_feedback_group_loaded(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = int(&args, 1);
    let loaded = state.sim.feedback.groups.iter().find(|g| g.state != 0 && g.id == id).is_some_and(|g| g.state == 3);
    Ok(vec![Value::Bool(loaded)])
}

/// `IsHudMessageQueueEmpty()` (0x00678370): the HUD queue (UIM+0x7230) holds a message (vtable +0x10, the count)
/// and is not suspended (+0x7255 = the queue's +0x25), negated (`feedback_queue.rs`).
fn is_hud_message_queue_empty(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let q = &state.feedback_queues.queues[super::feedback_queue::HUD];
    Ok(vec![Value::Bool(!(!q.messages.is_empty() && !q.suspended))])
}

/// `GetRaceDistance()` (0x00680c00): the race manager's float when its vtable +0xc call is true, else 0.
fn get_race_distance(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let f = &state.sim.feedback;
    Ok(vec![Value::Number(f64::from(if f.race_active { f.race_distance } else { 0.0 }))])
}

/// `StartFeedbackSequence()` (0x0067dd70): when the sequence handle (UIM+0x30a5c) is nonzero, byte +0x316e1 = 1.
fn start_feedback_sequence(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    if state.sim.feedback.sequence_handle != 0 {
        state.sim.feedback.sequence_start = true;
    }
    Ok(vec![])
}

/// `LoadFeedbackGroup(id)` (0x0067de60 -> `FUN_007982e0`(UIM+0x32858, id, G+0x2ed60)).
fn load_feedback_group(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::FeedbackGroup { load: true, id: int(&args, 1) });
    Ok(vec![])
}

/// `UnloadFeedbackGroup(id)` (0x0067deb0 -> `FUN_00798370`(UIM+0x32858, id)).
fn unload_feedback_group(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::FeedbackGroup { load: false, id: int(&args, 1) });
    Ok(vec![])
}

/// `RemoveMessage(id)` (0x0067dce0 -> `FUN_007b6130`(UIM+0x17e0, id)): only when argument 1 passes the type test
/// (`FUN_00ebff20`, taken to be `lua_isnumber`: model decision).
fn remove_message(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    if matches!(args.first(), Some(Value::Number(_))) {
        state.requests.push(Request::RemoveMessage { id: int(&args, 1) });
    }
    Ok(vec![])
}

/// `StopSound(id)` (0x0067e160 -> `FUN_007a2100`(UIM+0x2d2f0, id)).
fn stop_sound(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::StopSound { id: int(&args, 1) });
    Ok(vec![])
}

/// `ApplyMixSnapshotSet(id)` (0x0067e3e0 -> `FUN_00798fb0`(UIM+0x2d2f0, id, 1)); pushes nil.
fn apply_mix_snapshot_set(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::MixSnapshot { id: int(&args, 1), on: true });
    Ok(vec![Value::Nil])
}

/// `ClearMixSnapshotSet()` (0x006785d0 -> `FUN_00798fb0`(UIM+0x2d2f0, 0, 0)); pushes nil.
fn clear_mix_snapshot_set(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::MixSnapshot { id: 0, on: false });
    Ok(vec![Value::Nil])
}

/// `PlayMusic(id)` (0x0067e210 -> `FUN_0079f7b0`(UIM+0x30370, G, id, 0, 0, 0)).
fn play_music(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::PlayMusic { id: int(&args, 1) });
    Ok(vec![])
}

/// `PlaySpeech(a, b, c)` (0x0067e260): only when a != 0 and b, c != -1: `FUN_008408a0`(G+0x2c3c0, a, b, c, 0).
fn play_speech(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (a, b, c) = (int(&args, 1), int(&args, 2), int(&args, 3));
    if a != 0 && b != -1 && c != -1 {
        state.requests.push(Request::PlaySpeech { a, b, c });
    }
    Ok(vec![])
}

/// `TryPopPageFromHistory()` (0x0067e520): a type test on argument 1, then pushes false; nothing is popped.
fn try_pop_page_from_history(_: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Bool(false)])
}

/// `SelectDefaultRollout(id)` (0x00679bb0): selection record +0x8f90 = `_ftol2`(arg 1), +0x8f8c = 0.
fn select_default_rollout(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.sim.feedback.default_rollout = int(&args, 1);
    state.sim.feedback.rollout_8f8c = 0;
    Ok(vec![])
}

/// `SetSelectedWorld(id)` (0x006799a0): `FUN_00645780` on the world list G+0x2f468, as the hook.
fn set_selected_world(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::SetSelectedWorld { world: int(&args, 1) });
    Ok(vec![])
}

/// `SetRolloutFromVehicleID(id)` (0x00679bf0): above 0 -> `FUN_006c7950`(the local player's record, id, 0), as the
/// hook `SetRolloutFromVehicleIdString`.
fn set_rollout_from_vehicle_id(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let vehicle = int(&args, 1);
    if vehicle > 0 {
        state.requests.push(Request::SetRolloutFromVehicle { vehicle });
    }
    Ok(vec![])
}

pub fn register(r: &mut Registry) {
    r.set_lua("GetSelectedVehicleId", get_selected_vehicle_id);
    r.set_lua("GetSelectedColourId", get_selected_colour_id);
    r.set_lua("GetRoadCount", get_road_count);
    r.set_lua("IsFeedbackSequenceLoaded", is_feedback_sequence_loaded);
    r.set_lua("IsFeedbackGroupLoaded", is_feedback_group_loaded);
    r.set_lua("IsHudMessageQueueEmpty", is_hud_message_queue_empty);
    r.set_lua("GetRaceDistance", get_race_distance);
    r.set_lua("StartFeedbackSequence", start_feedback_sequence);
    r.set_lua("LoadFeedbackGroup", load_feedback_group);
    r.set_lua("UnloadFeedbackGroup", unload_feedback_group);
    r.set_lua("RemoveMessage", remove_message);
    r.set_lua("StopSound", stop_sound);
    r.set_lua("ApplyMixSnapshotSet", apply_mix_snapshot_set);
    r.set_lua("ClearMixSnapshotSet", clear_mix_snapshot_set);
    r.set_lua("PlayMusic", play_music);
    r.set_lua("PlaySpeech", play_speech);
    r.set_lua("TryPopPageFromHistory", try_pop_page_from_history);
    r.set_lua("SelectDefaultRollout", select_default_rollout);
    r.set_lua("SetSelectedWorld", set_selected_world);
    r.set_lua("SetRolloutFromVehicleID", set_rollout_from_vehicle_id);
}

#[cfg(test)]
mod tests {
    use super::super::LogicHost;
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

    fn b(r: &[Value]) -> bool {
        matches!(r[0], Value::Bool(true))
    }

    fn n(x: f64) -> Value {
        Value::Number(x)
    }

    #[test]
    fn reads() {
        let (mut lua, host) = setup();
        assert_eq!(call(&mut lua, "GetSelectedVehicleId", vec![])[0].as_number(), Some(0.0), "no local slot");
        {
            let mut s = host.state.borrow_mut();
            s.players.local = Some(2);
            s.sim.feedback.selected[2] = SelectedCar { vehicle_id: 77, colour_id: 5 };
            s.sim.feedback.road_count = 9;
            s.sim.feedback.groups[1] = FeedbackGroup { id: 4, state: 3 };
            s.sim.feedback.groups[2] = FeedbackGroup { id: 6, state: 1 };
            s.sim.feedback.groups[3] = FeedbackGroup { id: 8, state: 0 };
            s.sim.feedback.race_active = true;
            s.sim.feedback.race_distance = 1.5;
        }
        assert_eq!(call(&mut lua, "GetSelectedVehicleId", vec![])[0].as_number(), Some(77.0));
        assert_eq!(call(&mut lua, "GetSelectedColourId", vec![])[0].as_number(), Some(5.0));
        assert_eq!(call(&mut lua, "GetRoadCount", vec![])[0].as_number(), Some(9.0));
        assert_eq!(call(&mut lua, "GetRaceDistance", vec![])[0].as_number(), Some(1.5));
        assert!(b(&call(&mut lua, "IsFeedbackGroupLoaded", vec![n(4.0)])));
        assert!(!b(&call(&mut lua, "IsFeedbackGroupLoaded", vec![n(6.0)])));
        assert!(!b(&call(&mut lua, "IsFeedbackGroupLoaded", vec![n(8.0)])), "free entry");
        assert!(!b(&call(&mut lua, "IsFeedbackSequenceLoaded", vec![])));
        assert!(b(&call(&mut lua, "IsHudMessageQueueEmpty", vec![])));
        {
            let mut s = host.state.borrow_mut();
            s.sim.feedback.sequence_handle = 1;
            s.sim.feedback.sequence_state = 1;
            let f = &mut s.feedback_queues;
            f.set_group(1, Some(vec![super::super::feedback_queue::MessageDef { id: 3, ..Default::default() }]));
            f.post(super::super::feedback_queue::HUD, None, 3, "", Vec::new());
        }
        assert!(b(&call(&mut lua, "IsFeedbackSequenceLoaded", vec![])));
        assert!(!b(&call(&mut lua, "IsHudMessageQueueEmpty", vec![])));
        host.state.borrow_mut().feedback_queues.queues[super::super::feedback_queue::HUD].suspended = true;
        assert!(b(&call(&mut lua, "IsHudMessageQueueEmpty", vec![])));
        assert!(!b(&call(&mut lua, "TryPopPageFromHistory", vec![])));
    }

    #[test]
    fn acts() {
        let (mut lua, host) = setup();
        call(&mut lua, "StartFeedbackSequence", vec![]);
        assert!(!host.state.borrow().sim.feedback.sequence_start, "no sequence");
        host.state.borrow_mut().sim.feedback.sequence_handle = 3;
        call(&mut lua, "StartFeedbackSequence", vec![]);
        assert!(host.state.borrow().sim.feedback.sequence_start);
        call(&mut lua, "SelectDefaultRollout", vec![n(12.9)]);
        assert_eq!(host.state.borrow().sim.feedback.default_rollout, 12);
        call(&mut lua, "LoadFeedbackGroup", vec![n(4.0)]);
        call(&mut lua, "UnloadFeedbackGroup", vec![n(4.0)]);
        call(&mut lua, "RemoveMessage", vec![n(9.0)]);
        call(&mut lua, "RemoveMessage", vec![]);
        call(&mut lua, "StopSound", vec![n(2.0)]);
        assert!(call(&mut lua, "ApplyMixSnapshotSet", vec![n(5.0)])[0].is_nil());
        assert!(call(&mut lua, "ClearMixSnapshotSet", vec![])[0].is_nil());
        call(&mut lua, "PlayMusic", vec![n(7.0)]);
        call(&mut lua, "PlaySpeech", vec![n(1.0), n(2.0), n(3.0)]);
        call(&mut lua, "PlaySpeech", vec![n(0.0), n(2.0), n(3.0)]);
        call(&mut lua, "PlaySpeech", vec![n(1.0), n(-1.0), n(3.0)]);
        call(&mut lua, "SetSelectedWorld", vec![n(2.0)]);
        call(&mut lua, "SetRolloutFromVehicleID", vec![n(0.0)]);
        call(&mut lua, "SetRolloutFromVehicleID", vec![n(33.0)]);
        assert_eq!(
            host.state.borrow().requests,
            [
                Request::FeedbackGroup { load: true, id: 4 },
                Request::FeedbackGroup { load: false, id: 4 },
                Request::RemoveMessage { id: 9 },
                Request::StopSound { id: 2 },
                Request::MixSnapshot { id: 5, on: true },
                Request::MixSnapshot { id: 0, on: false },
                Request::PlayMusic { id: 7 },
                Request::PlaySpeech { a: 1, b: 2, c: 3 },
                Request::SetSelectedWorld { world: 2 },
                Request::SetRolloutFromVehicle { vehicle: 33 },
            ]
        );
        assert!(host.state.borrow().unported.is_empty());
    }
}
