//! Offline Lua natives of the vehicle / trigger / camera-world groups (handlers read from the exe's code,
//! `re/out/live/text.asm`; research log 2026-11-18, tenth batch). Vehicle queries go through the vehicle manager
//! (`*0x010edae4`: `FUN_00573390` exists = byte +0x3069 and `FUN_005733b0` loaded = byte +0x3068 of a 0x1c40 record,
//! `FUN_00571160` the record by id), the physics table (`*0x010edb04`+4 +0x55c48[id] > -1 = physical) and the zone
//! list (`FUN_005378d0`). Their engine state is set by the runtime ([`WorldFields`]); the natives that act on it
//! record a [`Request`].

use std::collections::HashMap;

use mw_lua::{Lua, Value};

use super::lua_args::{ftol32, toboolean, tonumber};
use super::players::Player;
use super::{LogicState, Registry, Request};

type R = mw_lua::Result<Vec<Value>>;

/// One vehicle of the vehicle manager, by id.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct WorldVehicle {
    /// `FUN_00573390`: byte +0x3069 of the record.
    pub exists: bool,
    /// `FUN_005733b0`: byte +0x3068.
    pub loaded: bool,
    /// The physics table entry (`*0x010edb04`+4 +0x55c48[id]) is > -1.
    pub physical: bool,
    /// `FUN_0057a050`: bit 0 of the cop flags (`IsVehicleCop`).
    pub cop: bool,
    /// The int at +0x160 of the record `FUN_00571160` returns (`IsInCar`).
    pub car_id: i32,
    /// The transform's position (`vtable[0]`(record) +0x30) and direction (+0x20).
    pub position: [f32; 3],
    pub direction: [f32; 3],
    /// `FUN_005734a0`'s record: velocity +0x120 and forward +0x100 (`GetSignedVehicleSpeed`).
    pub velocity: [f32; 3],
    pub forward: [f32; 3],
    /// `FUN_0083ea90`(id): the vehicle's speed in mph (physics record +0x1620, else the speed vector's length).
    pub speed_mph: f32,
}

/// One entry of the zone list (`FUN_005378d0`, 0x30 bytes): `kind` +0x11 (0 = box, 1 = sphere), position +0..+8,
/// the sphere radius +0x14, the box size +0x24..+0x2c.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Trigger {
    pub kind: i8,
    pub position: [f32; 3],
    pub radius: f32,
    pub size: [f32; 3],
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct WorldFields {
    pub vehicles: HashMap<i32, WorldVehicle>,
    /// The zone list entries by id (a 16-bit index).
    pub triggers: HashMap<i16, Trigger>,
    /// `FUN_005470e0` on `*0x010edadc`: the camera's world has finished streaming.
    pub camera_world_streamed: bool,
    /// UIM+0x2d204 / +0x2d20c (`ClearLoadingScreenLayout` clears bit 1 / the word).
    pub loading_flags: u32,
    pub loading_20c: u32,
    /// `FUN_0083b730` -> `FUN_005ce7b0`: the ICE take durations by take id (`GetIceTakeDuration`).
    pub ice_take_duration: HashMap<i32, f32>,
}

/// The local slot (`PM+0xa6b4`), -1 when none.
fn local_slot(state: &LogicState) -> i32 {
    state.players.local.map_or(-1, |i| i as i32)
}

/// The vehicle the handlers default to: the local player's (+0xa0) when its slot is occupied, else -1.
fn default_vehicle(state: &LogicState) -> i32 {
    state.players.local_player().map_or(-1, |p| p.vehicle)
}

/// A slot's occupied record (the original indexes `PM+0xa5d0` without a bound; an out-of-range slot reads as empty).
fn slot_player(state: &LogicState, slot: i32) -> Option<&Player> {
    state.players.slots.get(usize::try_from(slot).ok()?)?.as_ref()
}

/// `args.len() == 1` -> the vehicle id argument 1, else the default.
fn vehicle_arg(state: &LogicState, args: &[Value]) -> i32 {
    if args.len() == 1 { ftol32(tonumber(args, 1)) } else { default_vehicle(state) }
}

fn vehicle(state: &LogicState, id: i32) -> Option<&WorldVehicle> {
    state.sim.world.vehicles.get(&id)
}

fn boolean(b: bool) -> R {
    Ok(vec![Value::Bool(b)])
}

fn number(n: f64) -> R {
    Ok(vec![Value::Number(n)])
}

/// The `{ x, y, z }` table `FUN_00699ea0` builds.
pub(super) fn vec3(lua: &mut Lua, v: [f32; 3]) -> R {
    let t = lua.create_table();
    for (k, c) in ["x", "y", "z"].into_iter().zip(v) {
        t.set_str(k, Value::Number(f64::from(c)));
    }
    Ok(vec![Value::Table(t)])
}

/// `IsVehicleActive([vehicle])` (0x0067d450): `FUN_00573390`.
fn is_vehicle_active(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = vehicle_arg(state, &args);
    boolean(vehicle(state, id).is_some_and(|v| v.exists))
}

/// `IsVehicleLoaded([vehicle])` (0x0067d4e0): `FUN_0083bde0` -> `FUN_005733b0`.
fn is_vehicle_loaded(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = vehicle_arg(state, &args);
    boolean(vehicle(state, id).is_some_and(|v| v.loaded))
}

/// `IsVehiclePhysical([vehicle])` (0x0067d580): the physics table entry is > -1.
fn is_vehicle_physical(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = vehicle_arg(state, &args);
    boolean(vehicle(state, id).is_some_and(|v| v.physical))
}

/// `IsVehicleCop([vehicle])` (0x0067d610): false for -1, else `FUN_0083be20` -> `FUN_0057a050`.
fn is_vehicle_cop(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = vehicle_arg(state, &args);
    boolean(id != -1 && vehicle(state, id).is_some_and(|v| v.cop))
}

/// `IsInCar(carId, [vehicle])` (0x0067be90): the vehicle exists and its car id (+0x160) is `carId`; the vehicle is
/// argument 2 when there is more than one argument.
fn is_in_car(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let car = ftol32(tonumber(&args, 1));
    let id = if args.len() > 1 { ftol32(tonumber(&args, 2)) } else { default_vehicle(state) };
    boolean(vehicle(state, id).is_some_and(|v| v.exists && v.car_id == car))
}

/// `GetVehiclePosition([vehicle])` (0x00681d90): the transform's position as `{ x, y, z }`.
fn get_vehicle_position(state: &mut LogicState, lua: &mut Lua, args: Vec<Value>) -> R {
    let id = vehicle_arg(state, &args);
    let p = vehicle(state, id).map_or([0.0; 3], |v| v.position);
    vec3(lua, p)
}

/// `GetVehicleDirection([vehicle])` (0x00681e50): the transform's direction as `{ x, y, z }`.
fn get_vehicle_direction(state: &mut LogicState, lua: &mut Lua, args: Vec<Value>) -> R {
    let id = vehicle_arg(state, &args);
    let d = vehicle(state, id).map_or([0.0; 3], |v| v.direction);
    vec3(lua, d)
}

/// `GetVehicleDistanceToVehicle(a, b)` (0x0067cc30): the distance between the two positions (lane 0 of
/// `(z^2 + y^2) + x^2`, `sqrtps`).
fn get_vehicle_distance_to_vehicle(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let a = vehicle(state, ftol32(tonumber(&args, 1))).map_or([0.0; 3], |v| v.position);
    let b = vehicle(state, ftol32(tonumber(&args, 2))).map_or([0.0; 3], |v| v.position);
    let d: [f32; 3] = std::array::from_fn(|i| {
        let x = a[i] - b[i];
        x * x
    });
    number(f64::from(((d[2] + d[1]) + d[0]).sqrt()))
}

/// `GetSignedVehicleSpeed([vehicle])` (0x0067cd00): velocity (+0x120) . forward (+0x100) of `FUN_005734a0`'s record.
fn get_signed_vehicle_speed(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = vehicle_arg(state, &args);
    let (v, f) = vehicle(state, id).map_or(([0.0; 3], [0.0; 3]), |v| (v.velocity, v.forward));
    let p: [f32; 3] = std::array::from_fn(|i| v[i] * f[i]);
    number(f64::from((p[2] + p[1]) + p[0]))
}

/// `IsCarWrecked([slot])` (0x0067daa0): the slot's vehicle is alive (`FUN_006ba2a0`) and byte +0x23e.
fn is_car_wrecked(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let slot = if args.len() == 1 { ftol32(tonumber(&args, 1)) } else { local_slot(state) };
    boolean(slot_player(state, slot).is_some_and(|p| p.vehicle_alive && p.wrecked))
}

/// The speed of a slot's vehicle (`FUN_006ba3e0` -> `FUN_0083ea90`), `None` without a vehicle.
fn player_speed(state: &LogicState, slot: i32) -> Option<f32> {
    let p = slot_player(state, slot).filter(|p| p.vehicle != -1)?;
    Some(vehicle(state, p.vehicle).map_or(0.0, |v| v.speed_mph))
}

/// `GetPlayerSpeedMPH([slot])` (0x0067cec0): the speed of the slot's vehicle (argument 1 when it is the only
/// argument, else the local slot), 0 for an empty slot / no vehicle.
fn get_player_speed_mph(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let slot = if args.len() == 1 { ftol32(tonumber(&args, 1)) } else { local_slot(state) };
    number(f64::from(player_speed(state, slot).unwrap_or(0.0)))
}

/// `IsPlayerCarMoving()` (0x0067d010): the local player's vehicle is alive and loaded (`FUN_006ba2a0`,
/// `FUN_006ba2f0`) and |speed| > 0.5 (`.rdata` 0x00f8a810).
fn is_player_car_moving(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let Some(p) = state.players.local_player() else { return boolean(false) };
    if !(p.vehicle_alive && p.loaded) {
        return boolean(false);
    }
    let slot = local_slot(state);
    boolean(player_speed(state, slot).is_some_and(|s| s.abs() > 0.5))
}

/// `GetTriggerRadius(id, ...)` (0x0067d3f0): `FUN_0042c0b0` of the zone: a box (kind 0) is the length of half its
/// size (`.rdata` 0x00f8a810 = 0.5), a sphere (kind 1) its radius (+0x14), else 0. Model decision: an id without
/// an entry reads 0.
fn get_trigger_radius(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = ftol32(tonumber(&args, 1)) as i16;
    let r = match state.sim.world.triggers.get(&id) {
        Some(t) if t.kind == 0 => {
            let h: [f32; 3] = std::array::from_fn(|i| t.size[i] * 0.5);
            let s = f64::from(h[1] * h[1]) + f64::from(h[0] * h[0]) + f64::from(h[2] * h[2]);
            s.sqrt()
        }
        Some(t) if t.kind == 1 => f64::from(t.radius),
        _ => 0.0,
    };
    number(r)
}

/// `GetTriggerPosition(id)` (0x00681fd0): the zone's +0 / +4 / +8 as `{ x, y, z }`; an id <= -1 gives zeros (model
/// decision: so does an id without an entry).
fn get_trigger_position(state: &mut LogicState, lua: &mut Lua, args: Vec<Value>) -> R {
    let id = ftol32(tonumber(&args, 1)) as i16;
    let p = if id > -1 { state.sim.world.triggers.get(&id).map_or([0.0; 3], |t| t.position) } else { [0.0; 3] };
    vec3(lua, p)
}

/// `HasFinishedStreamingCameraWorld()` (0x00678200): `FUN_005470e0` on `*0x010edadc`.
fn has_finished_streaming_camera_world(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    boolean(state.sim.world.camera_world_streamed)
}

/// `GetIceTakeDuration(id)` (0x0067b960): `FUN_0083b730` -> `FUN_005ce7b0`(id).
fn get_ice_take_duration(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = ftol32(tonumber(&args, 1));
    number(f64::from(state.sim.world.ice_take_duration.get(&id).copied().unwrap_or(0.0)))
}

/// `ClearLoadingScreenLayout()` (0x00678610): UIM+0x2d204 &= ~2, UIM+0x2d20c = 0.
fn clear_loading_screen_layout(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.sim.world.loading_flags &= !2;
    state.sim.world.loading_20c = 0;
    Ok(vec![])
}

/// `ClearAllChevrons()` (0x00677e50): `FUN_006465a0`.
fn clear_all_chevrons(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::ClearAllChevrons);
    Ok(vec![])
}

/// `AwardNitrous(slot, amount)` (0x0067d910): `FUN_00658ad0`(slot, amount as float).
fn award_nitrous(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (slot, amount) = (ftol32(tonumber(&args, 1)), tonumber(&args, 2) as f32);
    state.requests.push(Request::NitrousAward { slot, amount });
    Ok(vec![])
}

/// `SetInfiniteNitrous(slot, on)` (0x0067d960): `FUN_00658c90` then byte +0xc0 = `on`.
fn set_infinite_nitrous(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (slot, on) = (ftol32(tonumber(&args, 1)), toboolean(&args, 2));
    state.requests.push(Request::NitrousInfinite { slot, on });
    Ok(vec![])
}

/// `ResetNitrousAmount([slot])` (0x0067d9f0): the local slot, or argument 1 when there is any argument.
fn reset_nitrous_amount(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let slot = if args.is_empty() { local_slot(state) } else { ftol32(tonumber(&args, 1)) };
    state.requests.push(Request::NitrousReset { slot });
    Ok(vec![])
}

/// `InvalidateRaceTarget(slot)` (0x0067ae10): an occupied slot whose +0xac vehicle is not -1 -> `FUN_00843170`.
fn invalidate_race_target(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let slot = ftol32(tonumber(&args, 1));
    if let Some(p) = slot_player(state, slot).filter(|p| p.race_target != -1) {
        let vehicle = p.race_target;
        state.requests.push(Request::InvalidateRaceTarget { vehicle });
    }
    Ok(vec![])
}

/// `StopUsingIceCamera(id)` (0x0067b890): `FUN_00843b40`(camera system, 0, id).
fn stop_using_ice_camera(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::IceCameraStop { id: ftol32(tonumber(&args, 1)) });
    Ok(vec![])
}

/// `SetIceCameraPaused(id, paused)` (0x0067b8f0): `FUN_00843ad0`(camera system, 0, id, paused).
fn set_ice_camera_paused(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::IceCameraPaused { id: ftol32(tonumber(&args, 1)), paused: toboolean(&args, 2) });
    Ok(vec![])
}

/// `SetSirenState(vehicle, on)` (0x0067c3d0): nothing for vehicle -1.
fn set_siren_state(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let vehicle = ftol32(tonumber(&args, 1));
    if vehicle != -1 {
        state.requests.push(Request::SirenState { vehicle, on: toboolean(&args, 2) });
    }
    Ok(vec![])
}

/// `SetEngineState(vehicle, on)` (0x0067c430).
fn set_engine_state(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::EngineState { vehicle: ftol32(tonumber(&args, 1)), on: toboolean(&args, 2) });
    Ok(vec![])
}

/// `SetCarStill(still, [slot])` (0x0067b130): the slot is the local one, or argument 2 when there are exactly two
/// arguments. `still`: byte +0x28e = 1; else dword +0x28c = 0 and byte +0x290 = 0 (model: an empty slot is ignored).
fn set_car_still(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let still = toboolean(&args, 1);
    let slot = if args.len() == 2 { ftol32(tonumber(&args, 2)) } else { local_slot(state) };
    let player = usize::try_from(slot).ok().and_then(|i| state.players.slots.get_mut(i)).and_then(|p| p.as_mut());
    if let Some(p) = player {
        if still {
            p.car_still = (p.car_still & !0x00ff_0000) | 0x0001_0000;
        } else {
            p.car_still = 0;
            p.car_still_290 = false;
        }
    }
    Ok(vec![])
}

pub fn register(r: &mut Registry) {
    r.set_lua("IsVehicleActive", is_vehicle_active);
    r.set_lua("IsVehicleLoaded", is_vehicle_loaded);
    r.set_lua("IsVehiclePhysical", is_vehicle_physical);
    r.set_lua("IsVehicleCop", is_vehicle_cop);
    r.set_lua("IsInCar", is_in_car);
    r.set_lua("GetVehiclePosition", get_vehicle_position);
    r.set_lua("GetVehicleDirection", get_vehicle_direction);
    r.set_lua("GetVehicleDistanceToVehicle", get_vehicle_distance_to_vehicle);
    r.set_lua("GetSignedVehicleSpeed", get_signed_vehicle_speed);
    r.set_lua("IsCarWrecked", is_car_wrecked);
    r.set_lua("GetPlayerSpeedMPH", get_player_speed_mph);
    r.set_lua("IsPlayerCarMoving", is_player_car_moving);
    r.set_lua("GetTriggerRadius", get_trigger_radius);
    r.set_lua("GetTriggerPosition", get_trigger_position);
    r.set_lua("HasFinishedStreamingCameraWorld", has_finished_streaming_camera_world);
    r.set_lua("GetIceTakeDuration", get_ice_take_duration);
    r.set_lua("ClearLoadingScreenLayout", clear_loading_screen_layout);
    r.set_lua("ClearAllChevrons", clear_all_chevrons);
    r.set_lua("AwardNitrous", award_nitrous);
    r.set_lua("SetInfiniteNitrous", set_infinite_nitrous);
    r.set_lua("ResetNitrousAmount", reset_nitrous_amount);
    r.set_lua("InvalidateRaceTarget", invalidate_race_target);
    r.set_lua("StopUsingIceCamera", stop_using_ice_camera);
    r.set_lua("SetIceCameraPaused", set_ice_camera_paused);
    r.set_lua("SetSirenState", set_siren_state);
    r.set_lua("SetEngineState", set_engine_state);
    r.set_lua("SetCarStill", set_car_still);
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

    fn field(r: &[Value], k: &str) -> f64 {
        match &r[0] {
            Value::Table(t) => t.get_str(k).as_number().unwrap(),
            v => panic!("not a table: {v:?}"),
        }
    }

    #[test]
    fn vehicles() {
        let (mut lua, host) = setup();
        assert!(!b(&call(&mut lua, "IsVehicleActive", vec![])), "no local player");
        {
            let mut s = host.state.borrow_mut();
            s.players.local = Some(0);
            s.players.slots[0] = Some(Player { vehicle: 4, vehicle_alive: true, loaded: true, ..Default::default() });
            s.players.slots[5] = Some(Player { vehicle: 9, wrecked: true, vehicle_alive: true, ..Default::default() });
            s.sim.world.vehicles.insert(
                4,
                WorldVehicle {
                    exists: true,
                    loaded: true,
                    car_id: 77,
                    position: [1.0, 2.0, 2.0],
                    direction: [0.0, 0.0, 1.0],
                    velocity: [1.0, 2.0, 3.0],
                    forward: [0.0, 0.5, 1.0],
                    speed_mph: -3.0,
                    ..Default::default()
                },
            );
            s.sim.world.vehicles.insert(
                8,
                WorldVehicle {
                    exists: true,
                    physical: true,
                    cop: true,
                    position: [4.0, 6.0, 2.0],
                    ..Default::default()
                },
            );
        }
        assert!(b(&call(&mut lua, "IsVehicleActive", vec![])), "defaults to the local player's vehicle");
        assert!(b(&call(&mut lua, "IsVehicleLoaded", vec![])));
        assert!(!b(&call(&mut lua, "IsVehiclePhysical", vec![])));
        assert!(!b(&call(&mut lua, "IsVehicleCop", vec![])));
        assert!(b(&call(&mut lua, "IsVehicleActive", vec![n(8.0)])));
        assert!(!b(&call(&mut lua, "IsVehicleLoaded", vec![n(8.0)])));
        assert!(b(&call(&mut lua, "IsVehiclePhysical", vec![n(8.0)])));
        assert!(b(&call(&mut lua, "IsVehicleCop", vec![n(8.0)])));
        assert!(!b(&call(&mut lua, "IsVehicleCop", vec![n(-1.0)])));
        assert!(!b(&call(&mut lua, "IsVehicleActive", vec![n(99.0)])));
        assert!(b(&call(&mut lua, "IsInCar", vec![n(77.0)])));
        assert!(!b(&call(&mut lua, "IsInCar", vec![n(76.0)])));
        assert!(!b(&call(&mut lua, "IsInCar", vec![n(77.0), n(8.0)])));
        let p = call(&mut lua, "GetVehiclePosition", vec![n(4.0)]);
        assert_eq!((field(&p, "x"), field(&p, "y"), field(&p, "z")), (1.0, 2.0, 2.0));
        let d = call(&mut lua, "GetVehicleDirection", vec![]);
        assert_eq!((field(&d, "x"), field(&d, "z")), (0.0, 1.0));
        let z = call(&mut lua, "GetVehiclePosition", vec![n(50.0)]);
        assert_eq!(field(&z, "y"), 0.0);
        assert_eq!(call(&mut lua, "GetVehicleDistanceToVehicle", vec![n(4.0), n(8.0)])[0].as_number(), Some(5.0));
        assert_eq!(call(&mut lua, "GetSignedVehicleSpeed", vec![])[0].as_number(), Some(4.0));
        assert!(!b(&call(&mut lua, "IsCarWrecked", vec![])));
        assert!(b(&call(&mut lua, "IsCarWrecked", vec![n(5.0)])));
        assert!(!b(&call(&mut lua, "IsCarWrecked", vec![n(6.0)])), "empty slot");
        assert_eq!(call(&mut lua, "GetPlayerSpeedMPH", vec![])[0].as_number(), Some(-3.0));
        assert_eq!(call(&mut lua, "GetPlayerSpeedMPH", vec![n(5.0)])[0].as_number(), Some(0.0));
        assert_eq!(call(&mut lua, "GetPlayerSpeedMPH", vec![n(7.0)])[0].as_number(), Some(0.0));
        assert!(b(&call(&mut lua, "IsPlayerCarMoving", vec![])));
        host.state.borrow_mut().sim.world.vehicles.get_mut(&4).unwrap().speed_mph = 0.5;
        assert!(!b(&call(&mut lua, "IsPlayerCarMoving", vec![])), "not above 0.5");
        host.state.borrow_mut().sim.world.vehicles.get_mut(&4).unwrap().speed_mph = -0.6;
        assert!(b(&call(&mut lua, "IsPlayerCarMoving", vec![])));
        host.state.borrow_mut().players.slots[0].as_mut().unwrap().loaded = false;
        assert!(!b(&call(&mut lua, "IsPlayerCarMoving", vec![])));
    }

    #[test]
    fn triggers_and_world() {
        let (mut lua, host) = setup();
        {
            let mut s = host.state.borrow_mut();
            let boxed = Trigger { kind: 0, position: [1.0, 2.0, 3.0], size: [6.0, 8.0, 0.0], radius: 9.0 };
            s.sim.world.triggers.insert(0, boxed);
            let sphere = Trigger { kind: 1, position: [4.0, 5.0, 6.0], size: [0.0; 3], radius: 9.0 };
            s.sim.world.triggers.insert(1, sphere);
            s.sim.world.triggers.insert(2, Trigger { kind: 2, ..Default::default() });
            s.sim.world.camera_world_streamed = true;
            s.sim.world.ice_take_duration.insert(3, 2.5);
            s.sim.world.loading_flags = 7;
            s.sim.world.loading_20c = 9;
        }
        assert_eq!(call(&mut lua, "GetTriggerRadius", vec![n(0.0)])[0].as_number(), Some(5.0));
        assert_eq!(call(&mut lua, "GetTriggerRadius", vec![n(1.0)])[0].as_number(), Some(9.0));
        assert_eq!(call(&mut lua, "GetTriggerRadius", vec![n(2.0)])[0].as_number(), Some(0.0));
        let p = call(&mut lua, "GetTriggerPosition", vec![n(1.0)]);
        assert_eq!((field(&p, "x"), field(&p, "y"), field(&p, "z")), (4.0, 5.0, 6.0));
        let m = call(&mut lua, "GetTriggerPosition", vec![n(-1.0)]);
        assert_eq!(field(&m, "x"), 0.0);
        assert!(b(&call(&mut lua, "HasFinishedStreamingCameraWorld", vec![])));
        assert_eq!(call(&mut lua, "GetIceTakeDuration", vec![n(3.0)])[0].as_number(), Some(2.5));
        assert_eq!(call(&mut lua, "GetIceTakeDuration", vec![n(4.0)])[0].as_number(), Some(0.0));
        assert!(call(&mut lua, "ClearLoadingScreenLayout", vec![]).is_empty());
        let s = host.state.borrow();
        assert_eq!((s.sim.world.loading_flags, s.sim.world.loading_20c), (5, 0));
    }

    #[test]
    fn acts() {
        let (mut lua, host) = setup();
        {
            let mut s = host.state.borrow_mut();
            s.players.local = Some(1);
            s.players.slots[1] = Some(Player::default());
            s.players.slots[3] = Some(Player { race_target: 6, ..Default::default() });
            s.players.slots[4] = Some(Player { race_target: -1, ..Default::default() });
        }
        call(&mut lua, "ClearAllChevrons", vec![]);
        call(&mut lua, "AwardNitrous", vec![n(2.9), n(0.5)]);
        call(&mut lua, "SetInfiniteNitrous", vec![n(1.0), Value::Bool(true)]);
        call(&mut lua, "ResetNitrousAmount", vec![]);
        call(&mut lua, "ResetNitrousAmount", vec![n(3.0)]);
        call(&mut lua, "InvalidateRaceTarget", vec![n(3.0)]);
        call(&mut lua, "InvalidateRaceTarget", vec![n(4.0)]);
        call(&mut lua, "InvalidateRaceTarget", vec![n(5.0)]);
        call(&mut lua, "StopUsingIceCamera", vec![n(2.0)]);
        call(&mut lua, "SetIceCameraPaused", vec![n(2.0), Value::Bool(true)]);
        call(&mut lua, "SetSirenState", vec![n(-1.0), Value::Bool(true)]);
        call(&mut lua, "SetSirenState", vec![n(8.0), Value::Bool(true)]);
        call(&mut lua, "SetEngineState", vec![n(8.0), Value::Bool(false)]);
        assert_eq!(
            host.state.borrow().requests,
            [
                Request::ClearAllChevrons,
                Request::NitrousAward { slot: 2, amount: 0.5 },
                Request::NitrousInfinite { slot: 1, on: true },
                Request::NitrousReset { slot: 1 },
                Request::NitrousReset { slot: 3 },
                Request::InvalidateRaceTarget { vehicle: 6 },
                Request::IceCameraStop { id: 2 },
                Request::IceCameraPaused { id: 2, paused: true },
                Request::SirenState { vehicle: 8, on: true },
                Request::EngineState { vehicle: 8, on: false },
            ]
        );
        call(&mut lua, "SetCarStill", vec![Value::Bool(true)]);
        assert_eq!(host.state.borrow().players.slots[1].as_ref().unwrap().car_still, 0x0001_0000);
        call(&mut lua, "SetCarStill", vec![Value::Bool(true), n(3.0)]);
        host.state.borrow_mut().players.slots[3].as_mut().unwrap().car_still_290 = true;
        call(&mut lua, "SetCarStill", vec![Value::Bool(false), n(3.0)]);
        let s = host.state.borrow();
        let p3 = s.players.slots[3].as_ref().unwrap();
        assert_eq!((p3.car_still, p3.car_still_290), (0, false));
        assert_eq!(s.players.slots[1].as_ref().unwrap().car_still, 0x0001_0000, "other slot untouched");
        assert!(s.unported.is_empty());
    }
}
