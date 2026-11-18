//! Offline Lua natives of the thirteenth batch (handlers read from the exe's code, `re/out/live/text.asm`; research
//! log 2026-11-18, thirteenth batch): the feedback sequence handle and skip, `SetVector3Variable`,
//! `SetExitPointID`, `GetCompetitorName`, the input / contact / unlock queries, the forced feedback message and
//! the three sound starters. State the handlers read is set by the runtime ([`RequestFields`]); the natives that
//! act on the sound / message systems record a [`Request`].

use std::collections::HashMap;

use mw_formats::json::Json;
use mw_lua::{Lua, Value};

use super::feedback_queue::HUD;
use super::lua_args::{int, push_int, push_str, string_or_invalid, toboolean, tonumber, touserdata};
use super::lua_widgets::{var_path, with_view_model};
use super::{LogicState, Registry, Request, vm_path};

type R = mw_lua::Result<Vec<Value>>;

/// What `FUN_0079fb40` returns when all 30 slots are taken.
pub const SOUND_SLOTS_FULL: i32 = 0x7fff_ffff;
/// The sound slots of UIM+0x2d2f0 (0x28 bytes each, in-use byte +0x719).
pub const SOUND_SLOTS: usize = 30;

/// State read by the handlers of this file.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestFields {
    /// UIM+0x2d2f0 +0: the counter `FUN_0079fb40` increments for each sound it starts (its result is the id).
    pub sound_counter: i32,
    /// The in-use bytes of the 30 slots; the runtime clears a slot when its sound ends, the natives set it.
    pub sound_slot_busy: [bool; SOUND_SLOTS],
    /// Controller object (`FUN_006c3850`) byte +0x499: some input was detected (`HasInputBeenDetected`).
    pub controller_input: bool,
    /// The local vehicle's input struct (+0x1928 of `FUN_00571160`) differs from a neutral one (`FUN_00670fc0`).
    pub vehicle_input: bool,
    /// The sequence id of the feedback sequence's current member (`UIM+0x30a64` handle +0x28, +0x30a68 base,
    /// +0x14); `None` = the handle is null (the original passes 0).
    pub skip_sequence_id: Option<i32>,
    /// `FUN_00904850` / `FUN_00904880`: per vehicle the contact list as the two bodies of each contact (id +0x60 /
    /// +0x62 of the entry, with the kind byte +6 of its table 0x010edb68 record). A vehicle without a list
    /// (`*(*(0x010edb04+4) +0x55c48 + v*4) < 0`) has no entry.
    pub vehicle_contacts: HashMap<i32, Vec<[ContactBody; 2]>>,
    /// `FUN_005734a0(vehicle)` +4: the vehicle's body id.
    pub vehicle_body_id: HashMap<i32, i16>,
    /// `FUN_0068cc30(id, 0)` on the two unlock tables: G'+0x2f3e4 (`IsAssetUnlocked(id, false)`) and the one at
    /// G'+0x2020 + 0x29200 (`IsAssetUnlocked(id, true)`). An id the table does not know is unlocked.
    pub unlocked: [HashMap<i32, bool>; 2],
}

/// One side of a contact: the record's id (+4) and kind (+6; 7 = a vehicle body).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct ContactBody {
    pub id: i16,
    pub kind: u8,
}

impl Default for RequestFields {
    fn default() -> Self {
        Self {
            sound_counter: 0,
            sound_slot_busy: [false; SOUND_SLOTS],
            controller_input: false,
            vehicle_input: false,
            skip_sequence_id: None,
            vehicle_contacts: HashMap::new(),
            vehicle_body_id: HashMap::new(),
            unlocked: Default::default(),
        }
    }
}

/// `FUN_0079fb40`: the first free slot takes the sound; the counter is incremented and is the sound's id.
/// All slots busy: `0x7fffffff`, nothing changes.
fn take_sound_slot(state: &mut LogicState) -> i32 {
    let f = &mut state.sim.requests;
    match f.sound_slot_busy.iter().position(|b| !b) {
        Some(i) => {
            f.sound_slot_busy[i] = true;
            f.sound_counter = f.sound_counter.wrapping_add(1);
            f.sound_counter
        }
        None => SOUND_SLOTS_FULL,
    }
}

/// `StopFeedbackSequence()` (0x00682710): UIM+0x30a5c / 60 = the null handle pair (0 / 0).
fn stop_feedback_sequence(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.sim.feedback.sequence_handle = 0;
    Ok(vec![])
}

/// `RequestSkipSequence()` (0x006827b0): `FUN_007b1060` on UIM+0x30840 with the id of the sequence handle's
/// member (0 without a handle).
fn request_skip_sequence(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let id = state.sim.requests.skip_sequence_id.unwrap_or(0);
    state.requests.push(Request::SkipSequence { id });
    Ok(vec![])
}

/// `SetVector3Variable(script, name, x, y, z [, prefix])` (0x007f4010 → `FUN_007f2bd0`): x, y, z read as floats
/// (`fstp dword`), stored as a JSON array of three doubles (tag 4 each).
fn set_vector3_variable(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let handle = touserdata(&args, 1);
    let path = var_path(&args, 2, 6);
    let v: Vec<Json> = (3..=5).map(|i| Json::Double(f64::from(tonumber(&args, i) as f32))).collect();
    with_view_model(state, handle, |vm| vm_path::set(vm, &path, Json::Array(v)));
    Ok(vec![])
}

/// `SetExitPointID(script, name)` (0x0067a860): the script object's +0x10 / +0x14 = the packed name
/// (`FUN_009ed220`, `hsm::key`). No script object (the original: a null pointer, it would crash) does nothing.
fn set_exit_point_id(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let name = string_or_invalid(&args, 2);
    if let Some(h) = touserdata(&args, 1)
        && let Some(s) = state.widget_scripts.scripts.iter_mut().find(|s| s.handle == h)
    {
        s.exit_point = mw_formats::hsm::key(&name);
    }
    Ok(vec![])
}

/// `GetCompetitorName(slot)` (0x0067b380): the player record's wide name (+0x50) narrowed by `FUN_00a49460`
/// (a character >= 0x80 becomes `?`, at most 255 characters). Model decision: an empty slot gives "".
fn get_competitor_name(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let p = usize::try_from(int(&args, 1)).ok().and_then(|i| state.players.slots.get(i)).and_then(|s| s.as_ref());
    let name: String = p.map_or_else(String::new, |p| {
        p.name.chars().take(255).map(|c| if (c as u32) < 0x80 { c } else { '?' }).collect()
    });
    Ok(vec![push_str(&name)])
}

/// `HasInputBeenDetected()` (0x0067ace0): the controller's detected byte, or (a local player whose vehicle
/// exists) the vehicle's input differing from a neutral one, or a feedback sequence handle being set.
fn has_input_been_detected(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let f = &state.sim.requests;
    let vehicle = state.players.local_player().is_some_and(|p| p.vehicle_alive) && f.vehicle_input;
    Ok(vec![Value::Bool(f.controller_input || vehicle || state.sim.feedback.sequence_handle != 0)])
}

/// `IsHittingRaceCar(a, b)` (0x0067d6b0): vehicle `a` has a contact list and one of its contacts, both bodies of
/// kind 7 (vehicles), has vehicle `b`'s body id as one side.
fn is_hitting_race_car(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let f = &state.sim.requests;
    let hit = match (f.vehicle_contacts.get(&int(&args, 1)), f.vehicle_body_id.get(&int(&args, 2))) {
        (Some(list), Some(&body)) => {
            list.iter().any(|[x, y]| x.kind == 7 && y.kind == 7 && (x.id == body || y.id == body))
        }
        _ => false,
    };
    Ok(vec![Value::Bool(hit)])
}

/// `IsAssetUnlocked(id, second_table)` (0x007eab40 → `FUN_0068f190` → `FUN_0068cc30(id, 0)`): the table is the
/// second (G'+0x2020 + 0x29200) when the boolean is true; an id the table does not know is unlocked.
fn is_asset_unlocked(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let table = usize::from(toboolean(&args, 2));
    let unlocked = state.sim.requests.unlocked[table].get(&int(&args, 1)).copied().unwrap_or(true);
    Ok(vec![Value::Bool(unlocked)])
}

/// `ShowForcedFeedBackMessage(text, id)` (0x0067db90): a non-empty HUD queue (vtable +0x10 result > 0) loses its
/// head (`FUN_007b3380`), then the message goes first (`FUN_007b8100`(text, id) → `FUN_007b8080` with position 0;
/// `feedback_queue.rs`).
fn show_forced_feedback_message(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let text = string_or_invalid(&args, 1);
    let id = int(&args, 2);
    let f = &mut state.feedback_queues;
    if !f.queues[HUD].messages.is_empty() {
        f.queues[HUD].drop_head();
    }
    f.post(HUD, Some(0), id as u32, &text, Vec::new());
    Ok(vec![])
}

/// Take a slot and record the sound start; returns the slot's id.
fn start_sound(state: &mut LogicState, args: &[Value], kind: u32, flags: u32, position: [f32; 3]) -> i32 {
    let slot = take_sound_slot(state);
    state.requests.push(Request::PlaySound {
        name: string_or_invalid(args, 1),
        id: int(args, 2),
        kind,
        flags,
        position,
        slot,
    });
    slot
}

/// `PlaySound(name, id)` (0x0067df60): kind 0 / flags 0 at the zero position; pushes the sound's id.
fn play_sound(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    Ok(vec![push_int(start_sound(state, &args, 0, 0, [0.0; 3]))])
}

/// `PlaySoundAtPlayer(name, id, slot)` (0x0067e040): kind 1 / flags 8 at the position of the slot's vehicle
/// (zero when the slot is empty; model decision: also zero when the vehicle is not in the world).
fn play_sound_at_player(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let position = usize::try_from(int(&args, 3))
        .ok()
        .and_then(|i| state.players.slots.get(i))
        .and_then(|s| s.as_ref())
        .and_then(|p| state.sim.world.vehicles.get(&p.vehicle))
        .map_or([0.0; 3], |v| v.position);
    Ok(vec![push_int(start_sound(state, &args, 1, 8, position))])
}

/// `PlayGameplayStream(name, id)` (0x0067e2d0): kind 4 / flags 0; pushes the id, nil when no slot was free.
fn play_gameplay_stream(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let slot = start_sound(state, &args, 4, 0, [0.0; 3]);
    Ok(vec![if slot == SOUND_SLOTS_FULL || slot < 0 { Value::Nil } else { push_int(slot) }])
}

pub fn register(r: &mut Registry) {
    r.set_lua("StopFeedbackSequence", stop_feedback_sequence);
    r.set_lua("RequestSkipSequence", request_skip_sequence);
    r.set_lua("SetVector3Variable", set_vector3_variable);
    r.set_lua("SetExitPointID", set_exit_point_id);
    r.set_lua("GetCompetitorName", get_competitor_name);
    r.set_lua("HasInputBeenDetected", has_input_been_detected);
    r.set_lua("IsHittingRaceCar", is_hitting_race_car);
    r.set_lua("IsAssetUnlocked", is_asset_unlocked);
    r.set_lua("ShowForcedFeedBackMessage", show_forced_feedback_message);
    r.set_lua("PlaySound", play_sound);
    r.set_lua("PlaySoundAtPlayer", play_sound_at_player);
    r.set_lua("PlayGameplayStream", play_gameplay_stream);
}

#[cfg(test)]
mod tests {
    use super::super::LogicHost;
    use super::super::feedback_queue::MessageDef;
    use super::super::lua_world::WorldVehicle;
    use super::super::players::Player;
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

    fn n(x: f64) -> Value {
        Value::Number(x)
    }

    fn s(x: &str) -> Value {
        push_str(x)
    }

    fn is_true(r: &[Value]) -> bool {
        matches!(r[0], Value::Bool(true))
    }

    #[test]
    fn feedback_sequence() {
        let (mut lua, host) = setup();
        host.state.borrow_mut().sim.feedback.sequence_handle = 7;
        call(&mut lua, "RequestSkipSequence", vec![]);
        host.state.borrow_mut().sim.requests.skip_sequence_id = Some(12);
        call(&mut lua, "RequestSkipSequence", vec![]);
        call(&mut lua, "StopFeedbackSequence", vec![]);
        let st = host.state.borrow();
        assert_eq!(st.sim.feedback.sequence_handle, 0);
        assert_eq!(st.requests, vec![Request::SkipSequence { id: 0 }, Request::SkipSequence { id: 12 }]);
    }

    #[test]
    fn vector_variable_and_exit_point() {
        let (mut lua, host) = setup();
        let h = host.state.borrow_mut().widget_scripts.create("W", "Script");
        let u = Value::LightUserdata(h);
        call(&mut lua, "SetVector3Variable", vec![u.clone(), s("pos"), n(1.5), n(2.0), n(-3.0), s("G")]);
        call(&mut lua, "SetExitPointID", vec![u, s("DONE")]);
        call(&mut lua, "SetExitPointID", vec![Value::Nil, s("X")]);
        let st = host.state.borrow();
        let vm = &st.widget_scripts.get(h).unwrap().view_model;
        let pos = vm.entries().iter().find(|(k, _)| k == "G").and_then(|(_, g)| g.get("pos")).unwrap();
        assert_eq!(pos, &Json::Array(vec![Json::Double(1.5), Json::Double(2.0), Json::Double(-3.0)]));
        assert_eq!(st.widget_scripts.get(h).unwrap().exit_point, mw_formats::hsm::key("DONE"));
    }

    #[test]
    fn competitor_name() {
        let (mut lua, host) = setup();
        host.state.borrow_mut().players.slots[1] = Some(Player { name: "Caf\u{e9}".into(), ..Default::default() });
        let r = call(&mut lua, "GetCompetitorName", vec![n(1.0)]);
        assert_eq!(r[0].as_str_lossy().as_deref(), Some("Caf?"));
        let r = call(&mut lua, "GetCompetitorName", vec![n(2.0)]);
        assert_eq!(r[0].as_str_lossy().as_deref(), Some(""));
    }

    #[test]
    fn input_detection() {
        let (mut lua, host) = setup();
        assert!(!is_true(&call(&mut lua, "HasInputBeenDetected", vec![])));
        {
            let mut st = host.state.borrow_mut();
            st.players.local = Some(0);
            st.players.slots[0] = Some(Player { vehicle_alive: true, ..Default::default() });
            st.sim.requests.vehicle_input = true;
        }
        assert!(is_true(&call(&mut lua, "HasInputBeenDetected", vec![])));
        {
            let mut st = host.state.borrow_mut();
            st.sim.requests.vehicle_input = false;
            st.sim.feedback.sequence_handle = 3;
        }
        assert!(is_true(&call(&mut lua, "HasInputBeenDetected", vec![])));
        host.state.borrow_mut().sim.feedback.sequence_handle = 0;
        host.state.borrow_mut().sim.requests.controller_input = true;
        assert!(is_true(&call(&mut lua, "HasInputBeenDetected", vec![])));
    }

    #[test]
    fn contacts_and_unlocks() {
        let (mut lua, host) = setup();
        {
            let r = &mut host.state.borrow_mut().sim.requests;
            let car = |id| ContactBody { id, kind: 7 };
            r.vehicle_contacts.insert(1, vec![[car(10), car(20)]]);
            r.vehicle_contacts.insert(2, vec![[car(10), ContactBody { id: 20, kind: 3 }]]);
            r.vehicle_body_id.insert(5, 20);
            r.unlocked[1].insert(9, false);
        }
        assert!(is_true(&call(&mut lua, "IsHittingRaceCar", vec![n(1.0), n(5.0)])));
        assert!(!is_true(&call(&mut lua, "IsHittingRaceCar", vec![n(2.0), n(5.0)])));
        assert!(!is_true(&call(&mut lua, "IsHittingRaceCar", vec![n(3.0), n(5.0)])));
        assert!(is_true(&call(&mut lua, "IsAssetUnlocked", vec![n(9.0)])));
        assert!(!is_true(&call(&mut lua, "IsAssetUnlocked", vec![n(9.0), Value::Bool(true)])));
        assert!(is_true(&call(&mut lua, "IsAssetUnlocked", vec![n(77.0), Value::Bool(true)])));
    }

    #[test]
    fn messages_and_sounds() {
        let (mut lua, host) = setup();
        {
            let mut st = host.state.borrow_mut();
            let def = |id| MessageDef { id, time: 1.0, ..Default::default() };
            st.feedback_queues.set_group(1, Some(vec![def(4), def(5), def(6)]));
            st.feedback_queues.post(HUD, None, 6, "queued", Vec::new());
        }
        call(&mut lua, "ShowForcedFeedBackMessage", vec![s("hi"), n(4.0)]);
        call(&mut lua, "ShowForcedFeedBackMessage", vec![s("again"), n(5.0)]);
        {
            let mut st = host.state.borrow_mut();
            st.players.slots[0] = Some(Player { vehicle: 3, ..Default::default() });
            st.sim.world.vehicles.insert(3, WorldVehicle { position: [1.0, 2.0, 3.0], ..Default::default() });
        }
        assert_eq!(call(&mut lua, "PlaySound", vec![s("a"), n(1.0)])[0].as_number(), Some(1.0));
        assert_eq!(call(&mut lua, "PlaySoundAtPlayer", vec![s("b"), n(2.0), n(0.0)])[0].as_number(), Some(2.0));
        assert_eq!(call(&mut lua, "PlayGameplayStream", vec![s("c"), n(3.0)])[0].as_number(), Some(3.0));
        host.state.borrow_mut().sim.requests.sound_slot_busy = [true; SOUND_SLOTS];
        let full = call(&mut lua, "PlaySound", vec![s("d"), n(1.0)]);
        assert_eq!(full[0].as_number(), Some(f64::from(SOUND_SLOTS_FULL)));
        assert!(matches!(call(&mut lua, "PlayGameplayStream", vec![s("e"), n(1.0)])[0], Value::Nil));
        let st = host.state.borrow();
        assert_eq!(st.sim.requests.sound_counter, 3);
        // Each forced message took the head's place: only the last is left, first; serials 1 (queued), 2, 3.
        let hud = &st.feedback_queues.queues[HUD];
        let left: Vec<_> = hud.messages.iter().map(|m| (m.def.id, m.name.as_str(), m.serial)).collect();
        assert_eq!(left, [(5, "again", 3)]);
        assert_eq!(
            st.requests[1],
            Request::PlaySound { name: "b".into(), id: 2, kind: 1, flags: 8, position: [1.0, 2.0, 3.0], slot: 2 }
        );
        assert!(matches!(&st.requests[2], Request::PlaySound { kind: 4, slot: 3, .. }));
    }
}
