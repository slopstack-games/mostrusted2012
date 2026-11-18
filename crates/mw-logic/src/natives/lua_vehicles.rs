//! Offline Lua natives of the twelfth batch (handlers read from the exe's code, `re/out/live/text.asm`; research
//! log 2026-11-18, twelfth batch): vehicle / point distances, steering, trigger tests and orientation, the camera
//! transitions, a few fire-and-forget calls and the debug stubs. State the handlers read is set by the runtime
//! ([`VehicleFields`]); the natives that act on it record a [`Request`].

use std::collections::HashMap;

use mw_lua::{Lua, Value};

use super::lua_args::{ftol32, int, string_or_invalid, toboolean, tonumber};
use super::lua_world::vec3;
use super::players::Player;
use super::{LogicState, Registry, Request};
use crate::binding;

type R = mw_lua::Result<Vec<Value>>;

/// The names `FUN_00678140` maps to a camera interpolation type (`PTR_DAT_010c5420`, compared case-insensitively);
/// the index is the type, an unknown name is 1.
const INTERPOLATIONS: [&str; 5] = ["Cut", "Linear", "LinearSine", "Cubic", "CubicSine"];

/// State read by the handlers of this file.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct VehicleFields {
    /// `.bss` 0x0118a090: the factor `GetClosingSpeed` multiplies the relative speed by (m/s to mph; the runtime
    /// fills it with the game's value, 0 until then).
    pub speed_to_mph: f32,
    /// The local controller object (`FUN_006c3850`) +0x18 / +0x24: `GetSteering` of the local slot is right - left.
    pub steer_left: f32,
    pub steer_right: f32,
    /// `FUN_0083b540` (G+0x2ef4c, keyed by the record's +0xb0) +0xc: the steering of a remote slot.
    pub remote_steering: HashMap<i32, f32>,
    /// `FUN_00537190` on `*0x010edaf0`: per vehicle the (trigger id +0xc, state byte +0x10) of the triggers it is
    /// in (the list at +0xc20 + vehicle * 0x24, count +0xc40).
    pub vehicle_triggers: HashMap<i32, Vec<(i32, i8)>>,
    /// UIM+0x2d1b0 +0x9c is set and `*(G+0x2f480)` +0x32688 is non-zero (`FUN_00799d90`).
    pub loading_screen_set: bool,
    /// `FUN_00792090` found the element of UIM+0x2d1b0 +0x9c (a missing element reads as showing) ...
    pub loading_screen_element_found: bool,
    /// ... and its virtual +0x14 result.
    pub loading_screen_element_showing: bool,
    /// G+0x30850 +0xdcc[+0xdd4]: the hashed names of the feedback sequence's triggers.
    pub feedback_triggers: Vec<u32>,
    /// G+0x25470 + 16 * slot, lane 0: `GetCompetitorDistance(slot)`.
    pub competitor_distance: HashMap<i32, f32>,
    /// `FUN_005f2800` over the slot's vehicle: traffic runs around it (`IsTrafficRunningForCompetitor`).
    pub traffic_running: HashMap<i32, bool>,
    /// The road starts (`*(G+0x153ac) + *(*(G+0x153a8) + 0x28) + 4`, 0x34 bytes each): the position `+0x28 +0x14`.
    pub road_starts: Vec<[f32; 3]>,
}

fn boolean(b: bool) -> R {
    Ok(vec![Value::Bool(b)])
}

fn number(n: f64) -> R {
    Ok(vec![Value::Number(n)])
}

/// The local player's vehicle (`+0xa0`) when the local slot is occupied, else -1.
fn local_vehicle(state: &LogicState) -> i32 {
    state.players.local_player().map_or(-1, |p| p.vehicle)
}

fn position(state: &LogicState, vehicle: i32) -> [f32; 3] {
    state.sim.world.vehicles.get(&vehicle).map_or([0.0; 3], |v| v.position)
}

/// `FUN_00697120`: `{ x, y, z }` of the table argument `i` (`lua_getfield` + `lua_tonumber`; a missing field is 0).
fn table_vec3(args: &[Value], i: usize) -> [f32; 3] {
    let Some(Value::Table(t)) = args.get(i - 1) else { return [0.0; 3] };
    ["x", "y", "z"].map(|k| Lua::tonumber(&t.get_str(k)).unwrap_or(0.0) as f32)
}

/// `(z^2 + y^2) + x^2` of the difference, the lane order the handlers' `shufps` / `addps` sums in.
fn squared_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d: [f32; 3] = std::array::from_fn(|i| {
        let x = a[i] - b[i];
        x * x
    });
    (d[2] + d[1]) + d[0]
}

/// The point-to-vehicle handlers: the vehicle is argument 2 when there are two arguments, else the local one.
fn point_vehicle_distance_squared(state: &LogicState, args: &[Value]) -> f32 {
    let point = table_vec3(args, 1);
    let vehicle = if args.len() == 2 { ftol32(tonumber(args, 2)) } else { local_vehicle(state) };
    squared_distance(position(state, vehicle), point)
}

/// `GetPointDistanceToVehicle(point, [vehicle])` (0x0067ca30): `sqrtps` of the squared distance.
fn get_point_distance_to_vehicle(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    number(f64::from(point_vehicle_distance_squared(state, &args).sqrt()))
}

/// `GetPointDistanceSquaredToVehicle(point, [vehicle])` (0x0067cb30): the same without the root.
fn get_point_distance_squared_to_vehicle(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    number(f64::from(point_vehicle_distance_squared(state, &args)))
}

/// A slot's player record; `None` for an empty / out-of-range slot (the original dereferences it).
fn slot_player(state: &LogicState, slot: i32) -> Option<&Player> {
    state.players.slots.get(usize::try_from(slot).ok()?)?.as_ref()
}

/// `FUN_006ba2a0`: the player has a vehicle (+0xa0 != -1) and it exists.
fn has_vehicle(p: &Player) -> bool {
    p.vehicle != -1 && p.vehicle_alive
}

/// `GetClosingSpeed(slotA, slotB)` (0x0067cdc0): both slots' vehicles exist (`FUN_006ba2a0`): the length of
/// their velocity difference (`FUN_006ba3b0` -> `FUN_0083bef0`, +0x120 of the record) times `.bss` 0x0118a090;
/// else 0.
fn get_closing_speed(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (a, b) = (int(&args, 1), int(&args, 2));
    let (Some(pa), Some(pb)) = (slot_player(state, a), slot_player(state, b)) else { return number(0.0) };
    if !(has_vehicle(pa) && has_vehicle(pb)) {
        return number(0.0);
    }
    let vel = |v: i32| state.sim.world.vehicles.get(&v).map_or([0.0; 3], |w| w.velocity);
    let d = squared_distance(vel(pa.vehicle), vel(pb.vehicle)).sqrt();
    number(f64::from(d * state.sim.vehicles.speed_to_mph))
}

/// `GetSteering([slot])` (0x006821b0) -> `0x0067db30`: the local slot (`PM+0xa6b4`) reads the controller's
/// right - left (+0x24 - +0x18 of `FUN_006c3850`, in extended precision); another slot looks its record's +0xb0 up
/// in G+0x2ef4c (`FUN_0083b530` / `FUN_0083b540`, +0xc) and reads 0 without one.
fn get_steering(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let local = state.players.local.map_or(-1, |i| i as i32);
    let slot = if args.len() == 1 { ftol32(tonumber(&args, 1)) } else { local };
    if slot == local {
        let v = &state.sim.vehicles;
        return number(f64::from(v.steer_right) - f64::from(v.steer_left));
    }
    number(f64::from(state.sim.vehicles.remote_steering.get(&slot).copied().unwrap_or(0.0)))
}

/// `FUN_00537190`: the vehicle's trigger list has the trigger id (+0xc) with a state byte (+0x10) equal to `wanted`
/// or `wanted == -1`. A vehicle without a list is in none.
fn in_trigger(state: &LogicState, vehicle: i32, trigger: i32, wanted: i8) -> bool {
    state
        .sim
        .vehicles
        .vehicle_triggers
        .get(&vehicle)
        .is_some_and(|l| l.iter().any(|&(id, s)| id == trigger && (wanted == s || wanted == -1)))
}

/// `IsInTrigger(trigger, [state])` (0x0067bd10): `FUN_00537190`(local vehicle, trigger, state), state -1 with one
/// argument; false with any other count.
fn is_in_trigger(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let vehicle = local_vehicle(state);
    let inside = match args.len() {
        1 => in_trigger(state, vehicle, ftol32(tonumber(&args, 1)), -1),
        2 => in_trigger(state, vehicle, ftol32(tonumber(&args, 1)), ftol32(tonumber(&args, 2)) as i8),
        _ => false,
    };
    boolean(inside)
}

/// `IsInTriggerList(trigger...)` (0x0067bdd0): the 0-based position of the first argument the local vehicle is in
/// (any state), -1 when none.
fn is_in_trigger_list(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let vehicle = local_vehicle(state);
    let found = (0..args.len()).find(|&i| in_trigger(state, vehicle, ftol32(tonumber(&args, i + 1)), -1));
    number(found.map_or(-1.0, |i| i as f64))
}

/// `FUN_0042ee30`'s third row (the z axis) of the rotation matrix of the zone's quaternion (+0x14: x, y, z, w;
/// each scaled by `.rdata` 0x00f9acf8 = sqrt 2).
fn box_z_axis(q: [f32; 4]) -> [f32; 3] {
    let k = std::f32::consts::SQRT_2;
    let (x, y, z, w) = (q[0] * k, q[1] * k, q[2] * k, q[3] * k);
    [w * y + z * x, y * z - w * x, (1.0 - x * x) - y * y]
}

/// The zone of an id: none for an id <= -1 (the handlers test the 16-bit id) or without an entry.
fn zone(state: &LogicState, id: i32) -> Option<&super::lua_world::Trigger> {
    let id = id as i16;
    if id > -1 { state.sim.world.triggers.get(&id) } else { None }
}

/// `GetTriggerDirection(id)` (0x00682070): `FUN_0042f250` of the zone: a box (kind 0) is the z axis of its
/// rotation, kind 2 the vector at +0x14, a sphere (1) `(1, 0, 0)`; an id <= -1 gives `(0, 0, 1)` (`.rdata`
/// 0x00fbcc30). Model decision: an id without an entry is `(0, 0, 1)`.
fn get_trigger_direction(state: &mut LogicState, lua: &mut Lua, args: Vec<Value>) -> R {
    let d = match zone(state, ftol32(tonumber(&args, 1))) {
        Some(t) => match t.kind {
            0 => box_z_axis(t.rotation),
            2 => [t.rotation[0], t.rotation[1], t.rotation[2]],
            _ => [1.0, 0.0, 0.0],
        },
        None => [0.0, 0.0, 1.0],
    };
    vec3(lua, d)
}

/// `GetTriggerRotation(id)` (0x0067d340): a box's heading, `-atan2(dir.x, dir.z)` of `FUN_0042efe0` (`FUN_00edd184`
/// is `atan2` on doubles, the sign flip `.rdata` 0x00f8f390); 0 for the other kinds and an id <= -1.
fn get_trigger_rotation(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let r = match zone(state, ftol32(tonumber(&args, 1))) {
        Some(t) if t.kind == 0 => {
            let d = box_z_axis(t.rotation);
            -(f64::from(d[0]).atan2(f64::from(d[2])) as f32)
        }
        _ => 0.0,
    };
    number(f64::from(r))
}

/// `IsLoadingScreenShowing()` (0x00678650): `FUN_00799d90`(UIM+0x2d1b0): false unless the screen object (+0x9c) and
/// `*(UIM)+0x32688` are set; then true when `FUN_00792090` finds no element, else the element's virtual +0x14.
fn is_loading_screen_showing(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let v = &state.sim.vehicles;
    boolean(v.loading_screen_set && (!v.loading_screen_element_found || v.loading_screen_element_showing))
}

/// `IsFeedbackSequenceTriggerActive(name)` (0x00682670): a sequence is set (G+0x30850 +0x20c) and the hash of the
/// name (`"<invalid>"` for a non-string) is among its triggers.
fn is_feedback_sequence_trigger_active(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let hash = binding::hash(&string_or_invalid(&args, 1));
    boolean(state.sim.queries.feedback_sequence_set && state.sim.vehicles.feedback_triggers.contains(&hash))
}

/// `GetRaceCarDistanceFromPlayer(vehicle)` (0x0067d100): the distance between the vehicle and the local one when
/// both exist (`FUN_00573390`), else 0.
fn get_race_car_distance_from_player(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let local = local_vehicle(state);
    let other = int(&args, 1);
    let exists = |v: i32| state.sim.world.vehicles.get(&v).is_some_and(|w| w.exists);
    if !(exists(other) && exists(local)) {
        return number(0.0);
    }
    number(f64::from(squared_distance(position(state, local), position(state, other)).sqrt()))
}

/// `GetCompetitorDistance(a, [b])` (0x0067b4a0): one argument reads lane 0 of G+0x25470 + 16 * a; two use
/// `FUN_0063c3b0`: the distance between the two slots' vehicles when both exist (`FUN_006ba2a0`), else -1
/// (`.rdata` 0x00fb6240).
fn get_competitor_distance(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    if args.len() == 1 {
        return number(f64::from(state.sim.vehicles.competitor_distance.get(&int(&args, 1)).copied().unwrap_or(0.0)));
    }
    let (a, b) = (int(&args, 1), int(&args, 2));
    let (Some(pa), Some(pb)) = (slot_player(state, a), slot_player(state, b)) else { return number(-1.0) };
    if !(has_vehicle(pa) && has_vehicle(pb)) {
        return number(-1.0);
    }
    number(f64::from(squared_distance(position(state, pb.vehicle), position(state, pa.vehicle)).sqrt()))
}

/// `IsTrafficRunningForCompetitor(slot)` (0x0067d8c0) -> `FUN_0083d410`: true for an empty slot or a kind-2
/// record (+0x9c); false when the vehicle does not exist or is not loaded (`FUN_005733b0`); else
/// `FUN_005f2800` (the runtime's `traffic_running`).
fn is_traffic_running_for_competitor(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let slot = int(&args, 1);
    let Some(p) = slot_player(state, slot) else { return boolean(true) };
    if p.kind == 2 {
        return boolean(true);
    }
    let loaded = state.sim.world.vehicles.get(&p.vehicle).is_some_and(|v| v.loaded);
    boolean(has_vehicle(p) && loaded && state.sim.vehicles.traffic_running.get(&slot) == Some(&true))
}

/// `GetRoadStartPosition(i)` (0x00687f80): the road's start position (+0x28 +0x14) as three numbers; an index
/// beyond the count reads the first road. Model decision: no roads / a negative index read zeros (the original
/// reads unchecked).
fn get_road_start_position(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let roads = &state.sim.vehicles.road_starts;
    let i = int(&args, 1);
    let p = if i < 0 { [0.0; 3] } else { roads.get(i as usize).or(roads.first()).copied().unwrap_or([0.0; 3]) };
    Ok(p.iter().map(|&c| Value::Number(f64::from(c))).collect())
}

/// `ChangeRaceCar(slot)` (0x0067bf50): `FUN_006c7950` on the slot's player record.
fn change_race_car(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::ChangeRaceCar { slot: int(&args, 1) });
    Ok(vec![])
}

/// `EnableKillTrafficAtLocation(enable, [x, y, z])` (0x0067d7c0): `FUN_00845ec0`(G+0x2ef64, enable, position): the
/// position is the local vehicle's with one argument, `(arg 2, 3, 4)` otherwise, zero when disabled.
fn enable_kill_traffic_at_location(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let enable = toboolean(&args, 1);
    let pos = if !enable {
        [0.0; 3]
    } else if args.len() == 1 {
        position(state, local_vehicle(state))
    } else {
        [tonumber(&args, 2) as f32, tonumber(&args, 3) as f32, tonumber(&args, 4) as f32]
    };
    state.requests.push(Request::EnableKillTrafficAtLocation { enable, position: pos });
    Ok(vec![])
}

/// `FUN_00678140`: the camera interpolation type of a name (`"<invalid>"` for a non-string).
fn interpolation(args: &[Value], i: usize) -> i32 {
    let name = string_or_invalid(args, i);
    INTERPOLATIONS.iter().position(|n| n.eq_ignore_ascii_case(&name)).map_or(1, |p| p as i32)
}

/// `TransitionCameraToLocation(position, direction, time, [interpolation])` (0x0067b9b0): `FUN_00843ba0`(director
/// `*(G+0x2f470)`, from = the camera manager's current position (+0x140) / direction (+0x130), to = the two
/// vectors, time, type).
fn transition_camera_to_location(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let q = &state.sim.queries;
    state.requests.push(Request::TransitionCameraToLocation {
        from_position: q.camera_position,
        from_direction: q.camera_direction,
        to_position: table_vec3(&args, 1),
        to_direction: table_vec3(&args, 2),
        time: tonumber(&args, 3) as f32,
        interpolation: interpolation(&args, 4),
    });
    Ok(vec![])
}

/// `TransitionCameraToGameplay(time, [interpolation])` (0x0067bb00): `FUN_00843ba0` with both camera structs
/// empty.
fn transition_camera_to_gameplay(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (time, interpolation) = (tonumber(&args, 1) as f32, interpolation(&args, 2));
    state.requests.push(Request::TransitionCameraToGameplay { time, interpolation });
    Ok(vec![])
}

/// `SetCameraTransitionLocation(position, direction)` (0x0067bb90): `FUN_00843c40`(director, position, `0 -
/// direction`).
fn set_camera_transition_location(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let d = table_vec3(&args, 2);
    state.requests.push(Request::SetCameraTransitionLocation {
        position: table_vec3(&args, 1),
        direction: [0.0 - d[0], 0.0 - d[1], 0.0 - d[2]],
    });
    Ok(vec![])
}

/// The handlers that only clear the stack or return nothing: `OverwriteDebugCameraState` (0x006781a0),
/// `ClearOverwriteDebugCameraState` (0x006781c0), `ShowX360KinectTunerUi` (0x007eb310) and the debug sequence
/// stubs (0x006782e0 ... 0x00678360, `xor eax, eax; ret`).
fn no_results(_: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![])
}

pub fn register(r: &mut Registry) {
    r.set_lua("GetPointDistanceToVehicle", get_point_distance_to_vehicle);
    r.set_lua("GetPointDistanceSquaredToVehicle", get_point_distance_squared_to_vehicle);
    r.set_lua("GetClosingSpeed", get_closing_speed);
    r.set_lua("GetSteering", get_steering);
    r.set_lua("IsInTrigger", is_in_trigger);
    r.set_lua("IsInTriggerList", is_in_trigger_list);
    r.set_lua("GetTriggerDirection", get_trigger_direction);
    r.set_lua("GetTriggerRotation", get_trigger_rotation);
    r.set_lua("IsLoadingScreenShowing", is_loading_screen_showing);
    r.set_lua("IsFeedbackSequenceTriggerActive", is_feedback_sequence_trigger_active);
    r.set_lua("GetRaceCarDistanceFromPlayer", get_race_car_distance_from_player);
    r.set_lua("GetCompetitorDistance", get_competitor_distance);
    r.set_lua("IsTrafficRunningForCompetitor", is_traffic_running_for_competitor);
    r.set_lua("GetRoadStartPosition", get_road_start_position);
    r.set_lua("ChangeRaceCar", change_race_car);
    r.set_lua("EnableKillTrafficAtLocation", enable_kill_traffic_at_location);
    r.set_lua("TransitionCameraToLocation", transition_camera_to_location);
    r.set_lua("TransitionCameraToGameplay", transition_camera_to_gameplay);
    r.set_lua("SetCameraTransitionLocation", set_camera_transition_location);
    for name in [
        "OverwriteDebugCameraState",
        "ClearOverwriteDebugCameraState",
        "ShowX360KinectTunerUi",
        "PlayDebugFeedbackSequence",
        "LoadDebugSequenceList",
        "IsDebugSequenceListLoaded",
        "LoadDebugSequence",
        "IsDebugSequenceLoaded",
        "PlayDebugSequence",
        "IsDebugSequencePlaying",
        "IsDebugSequencePausedForStreaming",
        "ShouldCaptureDebugSequence",
    ] {
        r.set_lua(name, no_results);
    }
}

#[cfg(test)]
mod tests {
    use super::super::LogicHost;
    use super::super::lua_args::push_str;
    use super::super::lua_world::{Trigger, WorldVehicle};
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

    fn num(r: &[Value]) -> f64 {
        r[0].as_number().unwrap()
    }

    fn is_true(r: &[Value]) -> bool {
        matches!(r[0], Value::Bool(true))
    }

    fn field(r: &[Value], k: &str) -> f64 {
        match &r[0] {
            Value::Table(t) => t.get_str(k).as_number().unwrap(),
            v => panic!("not a table: {v:?}"),
        }
    }

    fn point(lua: &mut Lua, p: [f64; 3]) -> Value {
        let t = lua.create_table();
        for (k, c) in ["x", "y", "z"].into_iter().zip(p) {
            t.set_str(k, Value::Number(c));
        }
        Value::Table(t)
    }

    fn player(vehicle: i32) -> Option<Player> {
        Some(Player { vehicle, vehicle_alive: true, ..Default::default() })
    }

    fn vehicle(position: [f32; 3], velocity: [f32; 3]) -> WorldVehicle {
        WorldVehicle { exists: true, loaded: true, position, velocity, ..Default::default() }
    }

    #[test]
    fn distances_and_closing_speed() {
        let (mut lua, host) = setup();
        {
            let mut s = host.state.borrow_mut();
            s.players.local = Some(0);
            s.players.slots[0] = player(4);
            s.players.slots[1] = player(5);
            s.sim.world.vehicles.insert(4, vehicle([3.0, 0.0, 0.0], [10.0, 0.0, 0.0]));
            s.sim.world.vehicles.insert(5, vehicle([0.0, 4.0, 0.0], [10.0, 3.0, 4.0]));
            s.sim.vehicles.speed_to_mph = 2.0;
            s.sim.vehicles.competitor_distance.insert(1, 7.5);
        }
        let p = point(&mut lua, [0.0, 0.0, 0.0]);
        assert_eq!(num(&call(&mut lua, "GetPointDistanceToVehicle", vec![p.clone()])), 3.0);
        assert_eq!(num(&call(&mut lua, "GetPointDistanceSquaredToVehicle", vec![p, n(5.0)])), 16.0);
        assert_eq!(num(&call(&mut lua, "GetClosingSpeed", vec![n(0.0), n(1.0)])), 10.0);
        assert_eq!(num(&call(&mut lua, "GetClosingSpeed", vec![n(0.0), n(7.0)])), 0.0);
        assert_eq!(num(&call(&mut lua, "GetRaceCarDistanceFromPlayer", vec![n(5.0)])), 5.0);
        assert_eq!(num(&call(&mut lua, "GetRaceCarDistanceFromPlayer", vec![n(9.0)])), 0.0);
        assert_eq!(num(&call(&mut lua, "GetCompetitorDistance", vec![n(0.0), n(1.0)])), 5.0);
        assert_eq!(num(&call(&mut lua, "GetCompetitorDistance", vec![n(0.0), n(3.0)])), -1.0);
        assert_eq!(num(&call(&mut lua, "GetCompetitorDistance", vec![n(1.0)])), 7.5);
    }

    #[test]
    fn steering_and_triggers() {
        let (mut lua, host) = setup();
        {
            let mut s = host.state.borrow_mut();
            s.players.local = Some(0);
            s.players.slots[0] = player(4);
            s.sim.vehicles.steer_left = 0.25;
            s.sim.vehicles.steer_right = 0.75;
            s.sim.vehicles.remote_steering.insert(3, -0.5);
            s.sim.vehicles.vehicle_triggers.insert(4, vec![(10, 1), (11, 2)]);
            // A rotation about Y by 90 degrees: y = sin(45), w = cos(45).
            let h = std::f32::consts::FRAC_1_SQRT_2;
            s.sim.world.triggers.insert(0, Trigger { kind: 0, rotation: [0.0, h, 0.0, h], ..Default::default() });
            s.sim.world.triggers.insert(1, Trigger { kind: 1, ..Default::default() });
            s.sim.world.triggers.insert(2, Trigger { kind: 2, rotation: [0.0, 1.0, 0.0, 0.0], ..Default::default() });
        }
        assert_eq!(num(&call(&mut lua, "GetSteering", vec![])), 0.5);
        assert_eq!(num(&call(&mut lua, "GetSteering", vec![n(0.0)])), 0.5);
        assert_eq!(num(&call(&mut lua, "GetSteering", vec![n(3.0)])), -0.5);
        assert_eq!(num(&call(&mut lua, "GetSteering", vec![n(5.0)])), 0.0);
        assert!(is_true(&call(&mut lua, "IsInTrigger", vec![n(10.0)])));
        assert!(is_true(&call(&mut lua, "IsInTrigger", vec![n(10.0), n(1.0)])));
        assert!(!is_true(&call(&mut lua, "IsInTrigger", vec![n(10.0), n(2.0)])));
        assert!(!is_true(&call(&mut lua, "IsInTrigger", vec![n(12.0)])));
        assert!(!is_true(&call(&mut lua, "IsInTrigger", vec![])));
        assert_eq!(num(&call(&mut lua, "IsInTriggerList", vec![n(1.0), n(11.0), n(10.0)])), 1.0);
        assert_eq!(num(&call(&mut lua, "IsInTriggerList", vec![n(1.0)])), -1.0);
        let d = call(&mut lua, "GetTriggerDirection", vec![n(0.0)]);
        assert!((field(&d, "x") - 1.0).abs() < 1e-6 && field(&d, "z").abs() < 1e-6, "{d:?}");
        assert_eq!(field(&call(&mut lua, "GetTriggerDirection", vec![n(1.0)]), "x"), 1.0);
        assert_eq!(field(&call(&mut lua, "GetTriggerDirection", vec![n(2.0)]), "y"), 1.0);
        assert_eq!(field(&call(&mut lua, "GetTriggerDirection", vec![n(-1.0)]), "z"), 1.0);
        let r = num(&call(&mut lua, "GetTriggerRotation", vec![n(0.0)]));
        assert!((r + std::f64::consts::FRAC_PI_2).abs() < 1e-5, "{r}");
        assert_eq!(num(&call(&mut lua, "GetTriggerRotation", vec![n(1.0)])), 0.0);
        assert_eq!(num(&call(&mut lua, "GetTriggerRotation", vec![n(-1.0)])), 0.0);
    }

    #[test]
    fn queries_and_requests() {
        let (mut lua, host) = setup();
        assert!(!is_true(&call(&mut lua, "IsLoadingScreenShowing", vec![])));
        {
            let mut s = host.state.borrow_mut();
            s.players.local = Some(0);
            s.players.slots[0] = player(4);
            s.players.slots[1] = Some(Player { vehicle: 5, vehicle_alive: true, kind: 2, ..Default::default() });
            s.sim.world.vehicles.insert(4, vehicle([1.0, 2.0, 3.0], [0.0; 3]));
            s.sim.vehicles.loading_screen_set = true;
            s.sim.queries.feedback_sequence_set = true;
            s.sim.vehicles.feedback_triggers = vec![binding::hash("go")];
            s.sim.vehicles.road_starts = vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]];
            s.sim.vehicles.traffic_running.insert(0, true);
            s.sim.queries.camera_position = [9.0, 8.0, 7.0];
        }
        assert!(is_true(&call(&mut lua, "IsLoadingScreenShowing", vec![])), "no element found: showing");
        host.state.borrow_mut().sim.vehicles.loading_screen_element_found = true;
        assert!(!is_true(&call(&mut lua, "IsLoadingScreenShowing", vec![])));
        assert!(is_true(&call(&mut lua, "IsFeedbackSequenceTriggerActive", vec![push_str("go")])));
        assert!(!is_true(&call(&mut lua, "IsFeedbackSequenceTriggerActive", vec![push_str("stop")])));
        assert!(is_true(&call(&mut lua, "IsTrafficRunningForCompetitor", vec![n(1.0)])), "kind 2");
        assert!(is_true(&call(&mut lua, "IsTrafficRunningForCompetitor", vec![n(9.0)])), "empty slot");
        assert!(is_true(&call(&mut lua, "IsTrafficRunningForCompetitor", vec![n(0.0)])));
        host.state.borrow_mut().sim.vehicles.traffic_running.clear();
        assert!(!is_true(&call(&mut lua, "IsTrafficRunningForCompetitor", vec![n(0.0)])));
        let road = call(&mut lua, "GetRoadStartPosition", vec![n(1.0)]);
        assert_eq!((num(&road), road[1].as_number(), road[2].as_number()), (4.0, Some(5.0), Some(6.0)));
        assert_eq!(num(&call(&mut lua, "GetRoadStartPosition", vec![n(9.0)])), 1.0);
        assert_eq!(call(&mut lua, "IsDebugSequencePlaying", vec![]).len(), 0);

        let a = point(&mut lua, [1.0, 2.0, 3.0]);
        let b = point(&mut lua, [0.0, 0.0, 1.0]);
        call(&mut lua, "ChangeRaceCar", vec![n(2.0)]);
        call(&mut lua, "EnableKillTrafficAtLocation", vec![Value::Bool(true)]);
        call(&mut lua, "EnableKillTrafficAtLocation", vec![Value::Bool(true), n(1.0), n(2.0), n(3.0)]);
        call(&mut lua, "EnableKillTrafficAtLocation", vec![Value::Bool(false), n(1.0), n(2.0), n(3.0)]);
        call(&mut lua, "TransitionCameraToLocation", vec![a.clone(), b.clone(), n(2.5), push_str("cubic")]);
        call(&mut lua, "TransitionCameraToGameplay", vec![n(1.5)]);
        call(&mut lua, "SetCameraTransitionLocation", vec![a, b]);
        assert_eq!(
            host.state.borrow().requests,
            vec![
                Request::ChangeRaceCar { slot: 2 },
                Request::EnableKillTrafficAtLocation { enable: true, position: [1.0, 2.0, 3.0] },
                Request::EnableKillTrafficAtLocation { enable: true, position: [1.0, 2.0, 3.0] },
                Request::EnableKillTrafficAtLocation { enable: false, position: [0.0; 3] },
                Request::TransitionCameraToLocation {
                    from_position: [9.0, 8.0, 7.0],
                    from_direction: [0.0; 3],
                    to_position: [1.0, 2.0, 3.0],
                    to_direction: [0.0, 0.0, 1.0],
                    time: 2.5,
                    interpolation: 3,
                },
                Request::TransitionCameraToGameplay { time: 1.5, interpolation: 1 },
                Request::SetCameraTransitionLocation { position: [1.0, 2.0, 3.0], direction: [0.0, 0.0, -1.0] },
            ]
        );
    }
}
