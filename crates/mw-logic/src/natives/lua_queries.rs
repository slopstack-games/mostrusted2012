//! Offline / online Lua natives of the eleventh batch (handlers read from the exe's code, `re/out/live/text.asm`;
//! research log 2026-11-18, eleventh batch): the camera vectors, queue and sequence queries, trigger / vehicle
//! distances and a few fire-and-forget calls. State the handlers read is set by the runtime ([`QueryFields`]); the
//! natives that act on it record a [`Request`].

use std::collections::HashMap;

use mw_lua::{Lua, Value};

use super::lua_args::{ftol32, int, string_or_invalid, tonumber};
use super::lua_world::vec3;
use super::{LogicState, Registry, Request};

type R = mw_lua::Result<Vec<Value>>;

/// `.rdata` 0x00fbcc00: the position `GetTriggerDistanceFromPlayer` uses when the player's vehicle does not exist
/// (all lanes `FLT_MAX`).
const NO_VEHICLE_POSITION: [f32; 3] = [f32::MAX; 3];
/// `.rdata` 0x00fbd9d8: `GetPlayerTopSpeedMPH` without a vehicle.
const NO_TOP_SPEED: f64 = 999.900_024_414_062_5;

/// State read by the handlers of this file.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct QueryFields {
    /// Camera manager (`*0x010edaf4`+4): +0x140 `GetCameraPosition`, +0x130 `GetCameraDirection`.
    pub camera_position: [f32; 3],
    pub camera_direction: [f32; 3],
    /// The active gameplay camera's (`+0x3690[+0x36d0]`) position, +0x40 (`GetGameplayCameraPosition`).
    pub gameplay_camera_position: [f32; 3],
    /// UIM+0x2d2f0 sound queue (`FUN_00792af0`): the 30 entries of 0x28 bytes as (id +0, active byte +9).
    pub sound_queue: Vec<(i32, bool)>,
    /// UIM+0x17e0 message queues (`FUN_007b29b0`): the ids (+0x330) of every queued message.
    pub message_queue: Vec<i32>,
    /// G+0x30850 +0x20c is non-zero (a feedback sequence is set) ...
    pub feedback_sequence_set: bool,
    /// ... and `FUN_007b0c20` (its time has run out).
    pub feedback_sequence_finished: bool,
    /// Traffic entity positions (`FUN_005ea2e0` / `FUN_005ea4c0` on `*0x010edae8`, +0x30), by 16-bit id.
    pub traffic_positions: HashMap<i16, [f32; 3]>,
    /// Trigger positions by id (`FUN_0053c020` on `*0x010edaf0`; not the zone list of `GetTriggerPosition`).
    pub trigger_points: HashMap<i32, [f32; 3]>,
    /// The distance to the finish per competitor (G+0x2f47c +0xdf0[i]); `FLT_MAX` (.data 0x010c53c0) = none.
    pub distance_to_finish: HashMap<i32, f32>,
    /// `FUN_0042e1b0` +0x3c of the vehicle's car data, by vehicle id (`GetPlayerTopSpeedMPH`).
    pub top_speed_mph: HashMap<i32, f32>,
    /// `FUN_004c1fa0` on `*0x010edb58`: the number of online players.
    pub online_players: i32,
    /// Player manager +0xc978[i] (0 = the friend list action succeeded) and byte +0xc98c[i] (finished).
    pub friend_action_result: HashMap<i32, i32>,
    pub friend_action_finished: HashMap<i32, bool>,
}

fn boolean(b: bool) -> R {
    Ok(vec![Value::Bool(b)])
}

fn number(n: f64) -> R {
    Ok(vec![Value::Number(n)])
}

/// `GetCameraPosition()` (0x00681080).
fn get_camera_position(state: &mut LogicState, lua: &mut Lua, _: Vec<Value>) -> R {
    vec3(lua, state.sim.queries.camera_position)
}

/// `GetCameraDirection()` (0x006810c0).
fn get_camera_direction(state: &mut LogicState, lua: &mut Lua, _: Vec<Value>) -> R {
    vec3(lua, state.sim.queries.camera_direction)
}

/// `GetGameplayCameraPosition()` (0x00681100).
fn get_gameplay_camera_position(state: &mut LogicState, lua: &mut Lua, _: Vec<Value>) -> R {
    vec3(lua, state.sim.queries.gameplay_camera_position)
}

/// `GetVehicleSpeedMph(vehicle)` (0x00681f00): the physics record's speed (+0x1620) or the speed vector's length
/// times the mph factor (.bss 0x0118a090): the value `FUN_0083ea90` gives (`WorldVehicle::speed_mph`). A vehicle
/// without a record reads 0.
fn get_vehicle_speed_mph(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = int(&args, 1);
    number(state.sim.world.vehicles.get(&id).map_or(0.0, |v| f64::from(v.speed_mph)))
}

/// `GetTrafficEntityPosition(id)` (0x00682120): the entity's +0x30 as `{ x, y, z }` (an id without an entry reads
/// zeros: model decision, the original indexes unchecked).
fn get_traffic_entity_position(state: &mut LogicState, lua: &mut Lua, args: Vec<Value>) -> R {
    let id = int(&args, 1) as i16;
    let p = state.sim.queries.traffic_positions.get(&id).copied().unwrap_or([0.0; 3]);
    vec3(lua, p)
}

/// `IsSoundInQueue(id)` (0x0067e1b0): `FUN_00792af0` over the first 30 entries: one is active and has the id.
fn is_sound_in_queue(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = int(&args, 1);
    boolean(state.sim.queries.sound_queue.iter().take(30).any(|&(i, active)| active && i == id))
}

/// `IsMessageInQueue(id)` (0x0067dc80): `FUN_007b29b0` finds a queued message with the id (+0x330).
fn is_message_in_queue(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = int(&args, 1);
    boolean(state.sim.queries.message_queue.contains(&id))
}

/// `IsFeedbackSequenceActive()` (0x0067dda0): a sequence is set (+0x20c) and `FUN_007b0c20` says it has not run
/// out.
fn is_feedback_sequence_active(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let q = &state.sim.queries;
    boolean(q.feedback_sequence_set && !q.feedback_sequence_finished)
}

/// `ClearTargetRoute()` (0x006757b0): clears the route lists at G+0x2f480 +0x28190 / +0x1f8f0 / +0x23d40
/// (`FUN_00792390`) and resets bytes / floats (+0x2ce33 = 0, +0x2c5d0 = 0, +0x2c5e0 = -1, +0x23d30 = 0,
/// +0x28180 = 0).
fn clear_target_route(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::ClearTargetRoute);
    Ok(vec![])
}

/// `SetRouteFromPlayerToEvent(event)` (0x006802d0): `FUN_0067fd00`(event).
fn set_route_from_player_to_event(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::SetRouteFromPlayerToEvent { event: int(&args, 1) });
    Ok(vec![])
}

/// `SetRouteFromPlayerToSafehouse(x)` (0x00679f90): the handler reads one number and discards it (a no-op).
fn set_route_from_player_to_safehouse(_: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![])
}

/// The local player's vehicle (`+0xa0`) when the local slot is occupied, else -1.
fn local_vehicle(state: &LogicState) -> i32 {
    state.players.local_player().map_or(-1, |p| p.vehicle)
}

/// `GetTriggerDistanceFromPlayer(id)` (0x0067d220): the distance from the trigger's position (`FUN_0053c020`) to
/// the local vehicle's position (`FUN_00573420`; `.rdata` 0x00fbcc00 = `FLT_MAX` lanes when the vehicle does not
/// exist, giving infinity). Model decisions: the position is `WorldVehicle::position`, a trigger id without an
/// entry reads zeros (the original dereferences NULL).
fn get_trigger_distance_from_player(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let vehicle = local_vehicle(state);
    let t = state.sim.queries.trigger_points.get(&int(&args, 1)).copied().unwrap_or([0.0; 3]);
    let v = match state.sim.world.vehicles.get(&vehicle) {
        Some(v) if v.exists => v.position,
        _ => NO_VEHICLE_POSITION,
    };
    let d: [f32; 3] = std::array::from_fn(|i| {
        let x = t[i] - v[i];
        x * x
    });
    number(f64::from(((d[2] + d[1]) + d[0]).sqrt()))
}

/// `GetPlayerTopSpeedMPH()` (0x0067cf70): +0x3c of the local vehicle's car data, 999.9 (`.rdata` 0x00fbd9d8)
/// without a vehicle. Model decision: a vehicle without an entry reads 0.
fn get_player_top_speed_mph(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    match local_vehicle(state) {
        -1 => number(NO_TOP_SPEED),
        v => number(f64::from(state.sim.queries.top_speed_mph.get(&v).copied().unwrap_or(0.0))),
    }
}

/// `GetDistanceToFinish(i)` (0x0067ae70): G+0x2f47c +0xdf0[i]; nil when it equals `FLT_MAX` (.data 0x010c53c0).
/// Model decision: an index without an entry is nil.
fn get_distance_to_finish(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    match state.sim.queries.distance_to_finish.get(&int(&args, 1)) {
        Some(&d) if d != f32::MAX => number(f64::from(d)),
        _ => Ok(vec![Value::Nil]),
    }
}

/// `ClearResetOnRoad([slot])` (0x0067c370): `FUN_0063d410`(G+0x25690, the slot's vehicle +0xa0); the slot is
/// argument 1 when there is an argument, else the local one. An empty slot is ignored (model: the original
/// dereferences it).
fn clear_reset_on_road(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let slot = if args.is_empty() { state.players.local.map_or(-1, |i| i as i32) } else { ftol32(tonumber(&args, 1)) };
    let player = usize::try_from(slot).ok().and_then(|i| state.players.slots.get(i)).and_then(|p| p.as_ref());
    if let Some(p) = player {
        let vehicle = p.vehicle;
        state.requests.push(Request::ClearResetOnRoad { vehicle });
    }
    Ok(vec![])
}

/// `SaveControllerConfiguration(n)` (0x007eb250): `FUN_008790d0`(+0x4210 of the controller object, n).
fn save_controller_configuration(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::SaveControllerConfiguration { config: int(&args, 1) });
    Ok(vec![])
}

/// `MutePlayer(name)` (0x007ee9d0): `FUN_007ecdc0`(name), `"<invalid>"` for a non-string.
fn mute_player(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::MutePlayerByName { name: string_or_invalid(&args, 1), mute: true });
    Ok(vec![])
}

/// `UnmutePlayer(name)` (0x007eea00): `FUN_007ece70`(name).
fn unmute_player(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::MutePlayerByName { name: string_or_invalid(&args, 1), mute: false });
    Ok(vec![])
}

/// `WasFriendListActionSuccessful(i)` (0x007e9ea0): player manager +0xc978[i] == 0 (no entry: 0).
fn was_friend_list_action_successful(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    boolean(state.sim.queries.friend_action_result.get(&int(&args, 1)).copied().unwrap_or(0) == 0)
}

/// `HasFriendListActionFinished(i)` (0x007e9e50): player manager byte +0xc98c[i] != 0.
fn has_friend_list_action_finished(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    boolean(state.sim.queries.friend_action_finished.get(&int(&args, 1)).copied().unwrap_or(false))
}

/// `StoreLoaded()` (0x007ea420): `FUN_0086f7f0` on the store object (+0x4ce0 of `[G+0x2f3e0]`).
fn store_loaded(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::StoreLoaded { loaded: true });
    Ok(vec![])
}

/// `StoreUnloaded()` (0x007ea450): `FUN_0086d620` on the same object.
fn store_unloaded(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::StoreLoaded { loaded: false });
    Ok(vec![])
}

/// `ClearUpAdverts()` (0x007ea6d0): `FUN_0065a480` on G+0x2a068 of the online game object.
fn clear_up_adverts(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::ClearUpAdverts);
    Ok(vec![])
}

/// `AdvertShown(name)` (0x007ea690): `FUN_00655c60`(G+0x2a068, name), `"<invalid>"` for a non-string.
fn advert_shown(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::AdvertShown { name: string_or_invalid(&args, 1) });
    Ok(vec![])
}

/// `GetNumOnlinePlayers()` (0x00677df0): `FUN_004c1fa0` on `*0x010edb58` (-> `FUN_00a83640`, a count).
fn get_num_online_players(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    number(f64::from(state.sim.queries.online_players))
}

pub fn register(r: &mut Registry) {
    r.set_lua("GetCameraPosition", get_camera_position);
    r.set_lua("GetCameraDirection", get_camera_direction);
    r.set_lua("GetGameplayCameraPosition", get_gameplay_camera_position);
    r.set_lua("GetVehicleSpeedMph", get_vehicle_speed_mph);
    r.set_lua("GetTrafficEntityPosition", get_traffic_entity_position);
    r.set_lua("IsSoundInQueue", is_sound_in_queue);
    r.set_lua("IsMessageInQueue", is_message_in_queue);
    r.set_lua("IsFeedbackSequenceActive", is_feedback_sequence_active);
    r.set_lua("ClearTargetRoute", clear_target_route);
    r.set_lua("SetRouteFromPlayerToEvent", set_route_from_player_to_event);
    r.set_lua("SetRouteFromPlayerToSafehouse", set_route_from_player_to_safehouse);
    r.set_lua("GetTriggerDistanceFromPlayer", get_trigger_distance_from_player);
    r.set_lua("GetPlayerTopSpeedMPH", get_player_top_speed_mph);
    r.set_lua("GetDistanceToFinish", get_distance_to_finish);
    r.set_lua("ClearResetOnRoad", clear_reset_on_road);
    r.set_lua("SaveControllerConfiguration", save_controller_configuration);
    r.set_lua("MutePlayer", mute_player);
    r.set_lua("UnmutePlayer", unmute_player);
    r.set_lua("WasFriendListActionSuccessful", was_friend_list_action_successful);
    r.set_lua("HasFriendListActionFinished", has_friend_list_action_finished);
    r.set_lua("StoreLoaded", store_loaded);
    r.set_lua("StoreUnloaded", store_unloaded);
    r.set_lua("ClearUpAdverts", clear_up_adverts);
    r.set_lua("AdvertShown", advert_shown);
    r.set_lua("GetNumOnlinePlayers", get_num_online_players);
}

#[cfg(test)]
mod tests {
    use super::super::LogicHost;
    use super::super::lua_args::push_str;
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

    #[test]
    fn cameras_and_queues() {
        let (mut lua, host) = setup();
        {
            let q = &mut host.state.borrow_mut().sim.queries;
            q.camera_position = [1.0, 2.0, 3.0];
            q.camera_direction = [0.0, 0.0, 1.0];
            q.gameplay_camera_position = [4.0, 5.0, 6.0];
            q.traffic_positions.insert(7, [9.0, 8.0, 7.0]);
            q.sound_queue = vec![(5, true), (6, false)];
            q.message_queue = vec![11];
            q.feedback_sequence_set = true;
            q.online_players = 3;
        }
        assert_eq!(field(&call(&mut lua, "GetCameraPosition", vec![]), "y"), 2.0);
        assert_eq!(field(&call(&mut lua, "GetCameraDirection", vec![]), "z"), 1.0);
        assert_eq!(field(&call(&mut lua, "GetGameplayCameraPosition", vec![]), "x"), 4.0);
        assert_eq!(field(&call(&mut lua, "GetTrafficEntityPosition", vec![n(7.0)]), "x"), 9.0);
        assert_eq!(field(&call(&mut lua, "GetTrafficEntityPosition", vec![n(8.0)]), "x"), 0.0);
        assert!(is_true(&call(&mut lua, "IsSoundInQueue", vec![n(5.0)])));
        assert!(!is_true(&call(&mut lua, "IsSoundInQueue", vec![n(6.0)])), "inactive entry");
        assert!(is_true(&call(&mut lua, "IsMessageInQueue", vec![n(11.0)])));
        assert!(!is_true(&call(&mut lua, "IsMessageInQueue", vec![n(12.0)])));
        assert!(is_true(&call(&mut lua, "IsFeedbackSequenceActive", vec![])));
        host.state.borrow_mut().sim.queries.feedback_sequence_finished = true;
        assert!(!is_true(&call(&mut lua, "IsFeedbackSequenceActive", vec![])));
        assert_eq!(num(&call(&mut lua, "GetNumOnlinePlayers", vec![])), 3.0);
    }

    #[test]
    fn distances_and_speeds() {
        let (mut lua, host) = setup();
        assert_eq!(num(&call(&mut lua, "GetPlayerTopSpeedMPH", vec![])), 999.900_024_414_062_5);
        {
            let mut s = host.state.borrow_mut();
            s.players.local = Some(0);
            s.players.slots[0] = Some(Player { vehicle: 4, ..Default::default() });
            s.sim.world.vehicles.insert(
                4,
                WorldVehicle { exists: true, position: [3.0, 0.0, 0.0], speed_mph: 61.5, ..Default::default() },
            );
            s.sim.queries.trigger_points.insert(2, [0.0, 4.0, 0.0]);
            s.sim.queries.top_speed_mph.insert(4, 180.0);
            s.sim.queries.distance_to_finish.insert(0, 250.0);
            s.sim.queries.distance_to_finish.insert(1, f32::MAX);
        }
        assert_eq!(num(&call(&mut lua, "GetVehicleSpeedMph", vec![n(4.0)])), 61.5);
        assert_eq!(num(&call(&mut lua, "GetVehicleSpeedMph", vec![n(40.0)])), 0.0);
        assert_eq!(num(&call(&mut lua, "GetTriggerDistanceFromPlayer", vec![n(2.0)])), 5.0);
        assert_eq!(num(&call(&mut lua, "GetPlayerTopSpeedMPH", vec![])), 180.0);
        assert_eq!(num(&call(&mut lua, "GetDistanceToFinish", vec![n(0.0)])), 250.0);
        assert!(matches!(call(&mut lua, "GetDistanceToFinish", vec![n(1.0)])[0], Value::Nil));
        host.state.borrow_mut().sim.world.vehicles.get_mut(&4).unwrap().exists = false;
        assert!(num(&call(&mut lua, "GetTriggerDistanceFromPlayer", vec![n(2.0)])).is_infinite());
    }

    #[test]
    fn friend_list_and_requests() {
        let (mut lua, host) = setup();
        {
            let q = &mut host.state.borrow_mut().sim.queries;
            q.friend_action_result.insert(1, 5);
            q.friend_action_finished.insert(1, true);
        }
        assert!(is_true(&call(&mut lua, "WasFriendListActionSuccessful", vec![n(0.0)])));
        assert!(!is_true(&call(&mut lua, "WasFriendListActionSuccessful", vec![n(1.0)])));
        assert!(is_true(&call(&mut lua, "HasFriendListActionFinished", vec![n(1.0)])));
        assert!(!is_true(&call(&mut lua, "HasFriendListActionFinished", vec![n(0.0)])));
        {
            let mut s = host.state.borrow_mut();
            s.players.local = Some(0);
            s.players.slots[0] = Some(Player { vehicle: 4, ..Default::default() });
            s.players.slots[2] = Some(Player { vehicle: 9, ..Default::default() });
        }
        call(&mut lua, "ClearTargetRoute", vec![]);
        call(&mut lua, "SetRouteFromPlayerToEvent", vec![n(42.0)]);
        call(&mut lua, "SetRouteFromPlayerToSafehouse", vec![n(1.0)]);
        call(&mut lua, "ClearResetOnRoad", vec![]);
        call(&mut lua, "ClearResetOnRoad", vec![n(2.0)]);
        call(&mut lua, "ClearResetOnRoad", vec![n(7.0)]);
        call(&mut lua, "SaveControllerConfiguration", vec![n(3.0)]);
        call(&mut lua, "MutePlayer", vec![Value::Nil]);
        call(&mut lua, "UnmutePlayer", vec![push_str("bob")]);
        call(&mut lua, "StoreLoaded", vec![]);
        call(&mut lua, "StoreUnloaded", vec![]);
        call(&mut lua, "ClearUpAdverts", vec![]);
        call(&mut lua, "AdvertShown", vec![push_str("ad")]);
        assert_eq!(
            host.state.borrow().requests,
            vec![
                Request::ClearTargetRoute,
                Request::SetRouteFromPlayerToEvent { event: 42 },
                Request::ClearResetOnRoad { vehicle: 4 },
                Request::ClearResetOnRoad { vehicle: 9 },
                Request::SaveControllerConfiguration { config: 3 },
                Request::MutePlayerByName { name: "<invalid>".into(), mute: true },
                Request::MutePlayerByName { name: "bob".into(), mute: false },
                Request::StoreLoaded { loaded: true },
                Request::StoreLoaded { loaded: false },
                Request::ClearUpAdverts,
                Request::AdvertShown { name: "ad".into() },
            ]
        );
    }
}
