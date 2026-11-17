//! Offline Lua natives that read or set one state field: the sim clock, the competitors, the camera, the units
//! and the loading-screen test (handlers read from the exe's code, `re/out/live/text.asm`; research log
//! 2026-11-18). G = `*0x01177870`; PM = `*(G+0x2f3dc)` the player manager; UIM = `*(G+0x2f480)`; the camera
//! manager `*(*0x010edaf4+4)`; the clock manager `*0x010edacc` (its `FUN_0046ee10` = `*(+4) + 0x38`).

use mw_lua::{Lua, Value};

use super::lua_args::{int, push_int, toboolean, tonumber};
use super::{LogicState, Registry, Request};

type R = mw_lua::Result<Vec<Value>>;

/// The sim clock (`FUN_0046ee10`, clock manager +4 → +0x38): the runtime keeps it.
#[derive(Debug, Clone, PartialEq)]
pub struct SimClock {
    /// +0x00: the frame count.
    pub frames: i32,
    /// +0x08 (i64): the sim time in ticks.
    pub ticks: i64,
    /// +0x1c: the clock runs.
    pub running: bool,
    /// +0x14 and +0x10 (floats): `GetSimTimeStep` pushes their product while the clock runs.
    pub scale: f32,
    pub step: f32,
}

impl Default for SimClock {
    fn default() -> Self {
        Self { frames: 0, ticks: 0, running: false, scale: 1.0, step: 0.0 }
    }
}

/// Fields of the clock, camera manager and UIM the natives of this file read.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SimFields {
    pub clock: SimClock,
    /// Camera manager +0x36de: the overhead camera is used.
    pub overhead_camera: bool,
    /// Camera manager +0x150: the field of view in radians (`GetCameraFov` multiplies it by 57.2958, `.rdata`
    /// 0x00fb86a0).
    pub camera_fov: f32,
    /// UIM +0x2cf28: the units are metric.
    pub units_metric: bool,
}

/// `GetFrameCount()` (0x00677c30): the sim clock's first int as a number.
fn get_frame_count(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![push_int(state.sim.clock.frames)])
}

/// `GetSimTime()` (0x00677cc0): the clock's i64 (+8) times the float 3.33333e-7 (`.rdata` 0x00f9f608), the
/// product rounded once to a double (x87).
fn get_sim_time(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Number(state.sim.clock.ticks as f64 * f64::from(3.333_333_4e-7_f32))])
}

/// `GetSimTimeStep()` (0x00677d00): the product of two clock floats (+0x14 × +0x10) while the clock's byte +0x1c
/// is set, else 0.
fn get_sim_time_step(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let c = &state.sim.clock;
    let v = if c.running { c.scale * c.step } else { 0.0 };
    Ok(vec![Value::Number(f64::from(v))])
}

/// `GetLocalPlayerCompetitorIndex()` (0x00677d60): PM+0xa6b4 (the local slot, −1 = none).
fn get_local_player_competitor_index(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![push_int(state.players.local.map_or(-1, |s| s as i32))])
}

/// `IsCompetitorActive(index)` (0x0067b2e0): the argument `lua_tonumber` + `_ftol2`; PM+0xa5d0[index] ≠ 0 (the
/// slot is occupied). Model decision: an index outside the 24 slots reads as empty (the original indexes raw
/// memory).
fn is_competitor_active(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let i = int(&args, 1);
    let active = usize::try_from(i).ok().and_then(|i| state.players.slots.get(i)).is_some_and(|s| s.is_some());
    Ok(vec![Value::Bool(active)])
}

/// `GetCompetitorTeam(index)` (0x0067b330): the int at PM + index × 0x7f0 + 0x268 (the team). Model decision:
/// an empty or out-of-range slot reads 0.
fn get_competitor_team(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let i = int(&args, 1);
    let p = usize::try_from(i).ok().and_then(|i| state.players.slots.get(i)).and_then(|s| s.as_ref());
    Ok(vec![push_int(p.map_or(0, |p| p.team))])
}

/// `GetCameraTarget()` (0x00678060 → `FUN_006bf100` on PM): the camera target slot (+0xa6bc) when it is set and
/// occupied, else the local slot (+0xa6b4); pushed as a signed number.
fn get_camera_target(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let pm = &state.players;
    let target = match pm.camera_target {
        Some(t) if pm.slots.get(t).is_some_and(|s| s.is_some()) => t as i32,
        _ => pm.local.map_or(-1, |s| s as i32),
    };
    Ok(vec![push_int(target)])
}

/// `ResetCameraTarget()` (0x00678030 → `FUN_006cab10`(PM, −1)): PM+0xa6bc = −1 (the message of the other path is
/// skipped for −1).
fn reset_camera_target(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.players.camera_target = None;
    Ok(vec![])
}

/// `GetUseOverheadCamera()` (0x006780b0): the camera manager's byte +0x36de.
fn get_use_overhead_camera(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Bool(state.sim.overhead_camera)])
}

/// `GetCameraFov()` (0x00677fa0): the camera manager's float +0x150 × 57.2958 (single precision), pushed as a
/// number.
fn get_camera_fov(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Number(f64::from(state.sim.camera_fov * 57.295_78_f32))])
}

/// `SetCameraFov(fov)` (0x0067bab0): `lua_tonumber` narrowed to a float; the camera message 0xf (`FUN_00843ca0`
/// (G+0x2f470, 0, 1, fov)) is posted: the camera's reaction is the runtime's.
fn set_camera_fov(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let fov = tonumber(&args, 1) as f32;
    state.requests.push(Request::CameraFov { set: true, fov });
    Ok(vec![])
}

/// `ResetCameraFov()` (0x00677ff0): the same message 0xf with (0, 0, 0.0).
fn reset_camera_fov(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::CameraFov { set: false, fov: 0.0 });
    Ok(vec![])
}

/// `AreUnitsMetric()` (0x006783e0): UIM's byte +0x2cf28.
fn are_units_metric(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Bool(state.sim.units_metric)])
}

/// `ShouldLoadingScreenBeShowing()` (0x006786a0): UIM+0x2d204 ≠ 0 and +0x2d200 = 0, the fact
/// [`Engine::loading_screen_showing`](super::state::Engine) holds.
fn should_loading_screen_be_showing(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Bool(state.engine.loading_screen_showing)])
}

/// `SetCarFrozen(frozen)` (0x0067b0c0): `lua_toboolean`; PM+0xcba8 = the flag as 1.0 / 0.0 (clamped to
/// 0..1.0, `.rdata` 0x00f8a820).
fn set_car_frozen(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.players.car_frozen = if toboolean(&args, 1) { 1.0 } else { 0.0 };
    Ok(vec![])
}

pub fn register(r: &mut Registry) {
    r.set_lua("GetFrameCount", get_frame_count);
    r.set_lua("GetSimTime", get_sim_time);
    r.set_lua("GetSimTimeStep", get_sim_time_step);
    r.set_lua("GetLocalPlayerCompetitorIndex", get_local_player_competitor_index);
    r.set_lua("IsCompetitorActive", is_competitor_active);
    r.set_lua("GetCompetitorTeam", get_competitor_team);
    r.set_lua("GetCameraTarget", get_camera_target);
    r.set_lua("ResetCameraTarget", reset_camera_target);
    r.set_lua("GetUseOverheadCamera", get_use_overhead_camera);
    r.set_lua("GetCameraFov", get_camera_fov);
    r.set_lua("SetCameraFov", set_camera_fov);
    r.set_lua("ResetCameraFov", reset_camera_fov);
    r.set_lua("AreUnitsMetric", are_units_metric);
    r.set_lua("ShouldLoadingScreenBeShowing", should_loading_screen_be_showing);
    r.set_lua("SetCarFrozen", set_car_frozen);
}

#[cfg(test)]
mod tests {
    use super::super::players::Player;
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
        r[0].as_number().unwrap()
    }

    fn flag(r: &[Value]) -> bool {
        matches!(r[0], Value::Bool(true))
    }

    fn st(host: &LogicHost) -> std::cell::RefMut<'_, LogicState> {
        host.state.borrow_mut()
    }

    #[test]
    fn clock() {
        let (mut lua, host) = setup();
        st(&host).sim.clock = SimClock { frames: 42, ticks: 3_000_000, running: true, scale: 0.5, step: 0.25 };
        assert_eq!(num(&call(&mut lua, "GetFrameCount", vec![])), 42.0);
        assert!((num(&call(&mut lua, "GetSimTime", vec![])) - 1.0).abs() < 1e-6);
        assert_eq!(num(&call(&mut lua, "GetSimTimeStep", vec![])), 0.125);
        st(&host).sim.clock.running = false;
        assert_eq!(num(&call(&mut lua, "GetSimTimeStep", vec![])), 0.0);
    }

    #[test]
    fn competitors_and_camera_target() {
        let (mut lua, host) = setup();
        assert_eq!(num(&call(&mut lua, "GetLocalPlayerCompetitorIndex", vec![])), -1.0);
        assert_eq!(num(&call(&mut lua, "GetCameraTarget", vec![])), -1.0);
        {
            let mut s = st(&host);
            s.players.slots[0] = Some(Player { team: 2, ..Default::default() });
            s.players.slots[3] = Some(Player { team: 5, ..Default::default() });
            s.players.local = Some(0);
        }
        assert_eq!(num(&call(&mut lua, "GetLocalPlayerCompetitorIndex", vec![])), 0.0);
        assert_eq!(flag(&call(&mut lua, "IsCompetitorActive", vec![Value::Number(3.9)])), true, "truncated");
        assert_eq!(flag(&call(&mut lua, "IsCompetitorActive", vec![Value::Number(1.0)])), false);
        assert_eq!(flag(&call(&mut lua, "IsCompetitorActive", vec![Value::Number(99.0)])), false);
        assert_eq!(num(&call(&mut lua, "GetCompetitorTeam", vec![Value::Number(3.0)])), 5.0);
        assert_eq!(num(&call(&mut lua, "GetCameraTarget", vec![])), 0.0, "no target: the local slot");
        st(&host).players.camera_target = Some(3);
        assert_eq!(num(&call(&mut lua, "GetCameraTarget", vec![])), 3.0);
        st(&host).players.camera_target = Some(7);
        assert_eq!(num(&call(&mut lua, "GetCameraTarget", vec![])), 0.0, "empty target slot: the local one");
        st(&host).players.camera_target = Some(3);
        call(&mut lua, "ResetCameraTarget", vec![]);
        assert_eq!(st(&host).players.camera_target, None);
    }

    #[test]
    fn camera_units_and_flags() {
        let (mut lua, host) = setup();
        st(&host).sim.camera_fov = 1.0;
        assert!((num(&call(&mut lua, "GetCameraFov", vec![])) - 57.2958).abs() < 1e-3);
        call(&mut lua, "SetCameraFov", vec![Value::Number(0.5)]);
        call(&mut lua, "ResetCameraFov", vec![]);
        assert_eq!(
            st(&host).requests,
            [Request::CameraFov { set: true, fov: 0.5 }, Request::CameraFov { set: false, fov: 0.0 }]
        );
        assert_eq!(flag(&call(&mut lua, "GetUseOverheadCamera", vec![])), false);
        st(&host).sim.overhead_camera = true;
        assert_eq!(flag(&call(&mut lua, "GetUseOverheadCamera", vec![])), true);
        st(&host).sim.units_metric = true;
        assert_eq!(flag(&call(&mut lua, "AreUnitsMetric", vec![])), true);
        assert_eq!(flag(&call(&mut lua, "ShouldLoadingScreenBeShowing", vec![])), false);
        st(&host).engine.loading_screen_showing = true;
        assert_eq!(flag(&call(&mut lua, "ShouldLoadingScreenBeShowing", vec![])), true);
        call(&mut lua, "SetCarFrozen", vec![Value::Bool(true)]);
        assert_eq!(st(&host).players.car_frozen, 1.0);
        call(&mut lua, "SetCarFrozen", vec![Value::Bool(false)]);
        assert_eq!(st(&host).players.car_frozen, 0.0);
        assert!(st(&host).unported.is_empty());
    }
}
