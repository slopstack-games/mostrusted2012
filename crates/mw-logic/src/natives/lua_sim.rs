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
    /// Camera system (G+0x2f470) byte +8: `AllowCameraSwitching`.
    pub allow_camera_switching: bool,
    /// G+0x291f8: `EnableNitrousBalancing`.
    pub nitrous_balancing: bool,
    /// The input state words the button natives compare with 2 (`FUN_0047aa20`(input manager, 0)): +0x1b4
    /// (the exit-loading-screen and skip-cutscene buttons), +0x19c (the spectator camera button).
    pub button_1b4: i32,
    pub button_19c: i32,
    /// The mask words at G+0x2f0a4 (`FUN_0083c960` sets / `FUN_0083c930` clears a bit; the byte +0x1c+word counts
    /// the words that changed): word 1 = `SetCarLocked`, word 2 = `DisableMenuControls` (bit 1 of each).
    pub input_masks: [u32; 8],
    pub input_mask_counts: [u8; 8],
    /// G+0x2efb5: `SetRearViewMirrorRender`.
    pub rear_view_mirror: bool,
    /// The sign-in view model's error record (`*(G+0x2f3e0)`+0x10d0 + 0x135c / 0x1360 / 0x139c / 0x136d: code,
    /// argument, two flag bytes); the reset values are 0 / 0 / 0 / 0.
    pub sign_in_error: SignInError,
    /// G+0x291f0: `EnableNitrousEarning`.
    pub nitrous_earning: bool,
    /// The sign-in VM's bytes +0x2438 / +0x2439 / +0x243a (`SetSpammable`).
    pub spammable: [bool; 3],
    /// Clock manager +4 -> +0x6c: the network timer state (1 = syncing, 2 = synced).
    pub net_timer_state: i32,
    /// Online manager +4 -> +0x6a20 != 0 (`IsOnline`).
    pub online: bool,
    /// The current event record's byte +0x6b (`IsInOnlineEvent`).
    pub in_online_event: bool,
    /// G'+0x2f6fc: `SetPreEventSpeedwallPos`.
    pub speedwall_pos: i32,
    /// The feedback / selection fields of [`lua_feedback`](super::lua_feedback).
    pub feedback: super::lua_feedback::FeedbackFields,
    /// The vehicle / trigger / camera-world fields of [`lua_world`](super::lua_world).
    pub world: super::lua_world::WorldFields,
    /// The camera / queue / distance fields of [`lua_queries`](super::lua_queries).
    pub queries: super::lua_queries::QueryFields,
}

/// The error record of the sign-in view model (VM +0x135c, +0x1360, +0x139c, +0x136d).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SignInError {
    pub code: u32,
    pub arg: i32,
    pub flag_139c: bool,
    pub flag_136d: bool,
}

/// The handlers that only check their arguments and return nothing (no state touched): `SetHeadlightsState`
/// (0x006781f0), `ShowX360FriendsPopup` (0x007eb290), `ShowPartyInvitePopup` (0x007eb2e0), `OnPhotoSharedToWall`
/// (0x007eb3b0), `UpdateSpammableFriends` (0x007e9c20) and the ones with one optional argument and nothing else:
/// `ShowX360GamerCard` (0x007eb2a0), `ShowX360GamerCardEasyDrive` (0x007eb2c0), `ShowX360GamerCardByPlayerIndex`
/// (0x007eb2f0).
fn no_effect(_: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![])
}

/// `IsFriendsUiOnScreen()` (0x007e9f20): pushes false (no overlay on PC).
fn is_friends_ui_on_screen(_: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Bool(false)])
}

/// `StopTraffic()` (0x00678240): message 6 to the traffic system.
fn stop_traffic(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::TrafficMessage { kind: 6 });
    Ok(vec![])
}

/// `ResetTraffic()` (0x00678270): message 8.
fn reset_traffic(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::TrafficMessage { kind: 8 });
    Ok(vec![])
}

/// `StartTraffic()` (0x006782a0): message 0xa.
fn start_traffic(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::TrafficMessage { kind: 0xa });
    Ok(vec![])
}

/// `IsStoryMode()` (0x006756d0): GMM+0x2e7a != 0.
fn is_story_mode(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Bool(state.game_modes.story_mode)])
}

/// `IsChallengeActive()` (0x00675640): GMM+0x2ce0 != 0.
fn is_challenge_active(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Bool(state.game_modes.current_event != 0)])
}

/// `SetRearViewMirrorRender(on)` (0x00679c60): G+0x2efb5 = `lua_toboolean`(1).
fn set_rear_view_mirror_render(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.sim.rear_view_mirror = toboolean(&args, 1);
    Ok(vec![])
}

/// `SetTOSAsLastError` / `SetCancelAsLastError` / `SetPWMisMatchAsLastError` / `SetEmailMisMatchAsLastError`
/// (0x007eaa80 / 0x007eaab0 / 0x007eaae0 / 0x007eab10 -> `FUN_00883690` / `FUN_008836c0` / `FUN_008836f0` /
/// `FUN_00883720` on VM+0x10d0): the error code (+0x135c) = 2 / 3 / 0x1a / 0x1e, +0x1360 = -1, bytes +0x139c and
/// +0x136d = 0.
fn set_sign_in_error(state: &mut LogicState, code: u32) -> R {
    state.sim.sign_in_error = SignInError { code, arg: -1, flag_139c: false, flag_136d: false };
    Ok(vec![])
}

fn set_tos_as_last_error(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    set_sign_in_error(state, 2)
}

fn set_cancel_as_last_error(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    set_sign_in_error(state, 3)
}

fn set_pw_mismatch_as_last_error(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    set_sign_in_error(state, 0x1a)
}

fn set_email_mismatch_as_last_error(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    set_sign_in_error(state, 0x1e)
}

/// `GetSignInError()` (0x007eaa30): the error code (VM +0x242c = +0x10d0 + 0x135c) as an unsigned number.
fn get_sign_in_error(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Number(f64::from(state.sim.sign_in_error.code))])
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

/// `SetMinimumCollisionDamage(x)` (0x00677e30): checks the arguments and does nothing else.
fn set_minimum_collision_damage(_: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![])
}

/// `GetCompetitorType(index)` (0x0067b400): the kind (player +0x9c) of the slot, as the string at
/// `0x010c5414[kind]` (0 `LocalPlayer`, 1 `Network`, 2 `AI`; the table shares its memory with unrelated strings after
/// that). Model decision: an empty slot or a kind past 2 gives the empty string.
fn get_competitor_type(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let i = int(&args, 1);
    let p = usize::try_from(i).ok().and_then(|i| state.players.slots.get(i)).and_then(|s| s.as_ref());
    let name = match p.map(|p| p.kind) {
        Some(0) => "LocalPlayer",
        Some(1) => "Network",
        Some(2) => "AI",
        _ => "",
    };
    Ok(vec![Value::from(name)])
}

/// `SetUseOverheadCamera(on)` (0x0067bc20): `lua_toboolean`; the camera message 0xd (0, on).
fn set_use_overhead_camera(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::CameraOverhead { on: toboolean(&args, 1) });
    Ok(vec![])
}

/// `SetUseFinishLineCamera(on, id)` (0x0067bc70): the camera message 0xe (0, on, id) with id = -1 unless `on`,
/// when it is the second argument (`lua_tonumber` + `_ftol2`).
fn set_use_finish_line_camera(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let on = toboolean(&args, 1);
    let id = if on { int(&args, 2) } else { -1 };
    state.requests.push(Request::CameraFinishLine { on, id });
    Ok(vec![])
}

/// `AllowCameraSwitching(on)` (0x0067bcd0): the camera system's byte +8 = `lua_toboolean`.
fn allow_camera_switching(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.sim.allow_camera_switching = toboolean(&args, 1);
    Ok(vec![])
}

/// `SnapCameraToBehindCar()` (0x00677f70): the camera message 1 (0).
fn snap_camera_to_behind_car(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::CameraSnapBehind);
    Ok(vec![])
}

/// `EnableLocalRaceCarRumble(on)` (0x0067b450): `lua_toboolean`; the message 9 (`FUN_00844900`).
fn enable_local_race_car_rumble(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::RumbleEnable { on: toboolean(&args, 1) });
    Ok(vec![])
}

/// `IsExitLoadingScreenButtonPressed()` (0x00677c70): the input state word +0x1b4 equals 2.
fn is_exit_loading_screen_button_pressed(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Bool(state.sim.button_1b4 == 2)])
}

/// `IsSkipCutsceneButtonPressed()` (0x00678700): the same word +0x1b4 equals 2.
fn is_skip_cutscene_button_pressed(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Bool(state.sim.button_1b4 == 2)])
}

/// `IsSpectatorCameraButtonPressed()` (0x006780f0): the input state word +0x19c equals 2.
fn is_spectator_camera_button_pressed(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Bool(state.sim.button_19c == 2)])
}

/// Set (`FUN_0083c960`) or clear (`FUN_0083c930`) bit 1 of mask word `word`; the counter byte changes when the
/// word does.
fn set_mask_bit(state: &mut LogicState, word: usize, on: bool) {
    let s = &mut state.sim;
    let old = s.input_masks[word];
    if on {
        s.input_masks[word] |= 2;
        if old != s.input_masks[word] {
            s.input_mask_counts[word] = s.input_mask_counts[word].wrapping_add(1);
        }
    } else {
        s.input_masks[word] &= !2;
        if old != s.input_masks[word] {
            s.input_mask_counts[word] = s.input_mask_counts[word].wrapping_sub(1);
        }
    }
}

/// `SetCarLocked(locked)` (0x0067b220): bit 1 of mask word 1 at G+0x2f0a4, set when `lua_toboolean`.
fn set_car_locked(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    set_mask_bit(state, 1, toboolean(&args, 1));
    Ok(vec![])
}

/// `DisableMenuControls(disable)` (0x0067b280): bit 1 of mask word 2 at G+0x2f0a4.
fn disable_menu_controls(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    set_mask_bit(state, 2, toboolean(&args, 1));
    Ok(vec![])
}

/// `EnableNitrousBalancing([on])` (0x0067b7b0): G+0x291f8 = `lua_toboolean`(1), or true without an argument.
fn enable_nitrous_balancing(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.sim.nitrous_balancing = if args.is_empty() { true } else { toboolean(&args, 1) };
    Ok(vec![])
}

fn stop_all_sounds(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::StopSounds { kind: 0 });
    Ok(vec![])
}

fn stop_speech(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::StopSounds { kind: 2 });
    Ok(vec![])
}

fn stop_gameplay_stream(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::StopSounds { kind: 4 });
    Ok(vec![])
}

fn stop_music(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::StopMusic);
    Ok(vec![])
}

fn stop_unwanted_sound(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::StopUnwantedSound);
    Ok(vec![])
}

fn clear_all_feedback_queues(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::ClearFeedbackQueues { background: true });
    Ok(vec![])
}

fn clear_all_non_background_feedback_queues(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::ClearFeedbackQueues { background: false });
    Ok(vec![])
}

/// Handlers that only check their arguments: `FlagUseMaxTriCacheSize` (0x006781e0), `ToggleUnlockAllVehicles`
/// (0x006757a0), `ViewInvites` (0x006758d0), `SetUseDebugCamera` (0x00678180, checks one argument),
/// `SetMatchMakingState` (0x007e9a30, reads and discards a number) and `DebugPrint` (0x007e9940, reads and discards
/// its arguments).
fn arg_check_only(_: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![])
}

/// `IsRemoteCompetitor(index)` (0x0067b1d0 -> `FUN_006c3750`): the slot's kind (+0x9c) is 1 (network). Model
/// decision: an empty slot reads false.
fn is_remote_competitor(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let i = int(&args, 1);
    let p = usize::try_from(i).ok().and_then(|i| state.players.slots.get(i)).and_then(|s| s.as_ref());
    Ok(vec![Value::Bool(p.is_some_and(|p| p.kind == 1))])
}

/// `SetRaceCarInvulnerable(index, on)` (0x0068b010): the message 0x1f with (competitor, flag).
fn set_race_car_invulnerable(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (competitor, on) = (int(&args, 1), toboolean(&args, 2));
    state.requests.push(Request::RaceCarFlag { kind: 0x1f, competitor, on });
    Ok(vec![])
}

/// `SetRaceCarDamageAllowed(index, on)` (0x0068b070): the message 0x1e.
fn set_race_car_damage_allowed(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (competitor, on) = (int(&args, 1), toboolean(&args, 2));
    state.requests.push(Request::RaceCarFlag { kind: 0x1e, competitor, on });
    Ok(vec![])
}

/// `EnableNitrousEarning(on)` (0x0067d9c0): G+0x291f0 = `lua_toboolean`(1).
fn enable_nitrous_earning(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.sim.nitrous_earning = toboolean(&args, 1);
    Ok(vec![])
}

/// `SetSpammable(a, b, c)` (0x007ea9c0): the sign-in VM's bytes +0x2438 = a, +0x2439 = b, +0x243a = c.
fn set_spammable(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.sim.spammable = [toboolean(&args, 1), toboolean(&args, 2), toboolean(&args, 3)];
    Ok(vec![])
}

/// `IsNetworkTimerSyncing()` (0x00677ec0 -> `FUN_0046eec0`): the clock manager's state (+0x6c) is 1.
fn is_network_timer_syncing(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Bool(state.sim.net_timer_state == 1)])
}

/// `IsNetworkTimerSynced()` (0x00677f00 -> `FUN_0046eed0`): the state is 2.
fn is_network_timer_synced(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Bool(state.sim.net_timer_state == 2)])
}

/// `IsOnline()` (0x00677da0 -> `FUN_004c1ec0`): the online manager's field +0x6a20 is nonzero.
fn is_online(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Bool(state.sim.online)])
}

/// `IsInOnlineEvent()` (0x00681030): the byte +0x6b of the current event's record.
fn is_in_online_event(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Bool(state.sim.in_online_event)])
}

/// `GetPlayerVehicleIndex([index])` (0x0067c9b0): the slot is the argument when exactly one is given, else the
/// local slot (PM+0xa6b4); pushes the slot's vehicle (+0xa0) as a signed number. Model decision: an empty slot
/// gives -1 (the original dereferences it).
fn get_player_vehicle_index(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let slot = if args.len() == 1 { usize::try_from(int(&args, 1)).ok() } else { state.players.local };
    let p = slot.and_then(|i| state.players.slots.get(i)).and_then(|s| s.as_ref());
    Ok(vec![push_int(p.map_or(-1, |p| p.vehicle))])
}

/// `SetPreEventSpeedwallPos(pos)` (0x00679eb0): G'+0x2f6fc = `_ftol2`(arg 1).
fn set_pre_event_speedwall_pos(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.sim.speedwall_pos = int(&args, 1);
    Ok(vec![])
}

/// `SetTrafficDensity(on, density)` (0x0067da40): the float is read only when `on`, else 0.
fn set_traffic_density(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let on = toboolean(&args, 1);
    let density = if on { tonumber(&args, 2) as f32 } else { 0.0 };
    state.requests.push(Request::TrafficDensity { on, density });
    Ok(vec![])
}

/// `RefreshGarageItemsCache()` (0x006759c0).
fn refresh_garage_items_cache(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::RefreshGarageItemsCache);
    Ok(vec![])
}

/// `UnregisterSpeechCommand(id)` (0x007ea7f0): `_ftol2`(arg 1).
fn unregister_speech_command(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::UnregisterSpeechCommand { id: int(&args, 1) });
    Ok(vec![])
}

pub fn register(r: &mut Registry) {
    r.set_lua("FlagUseMaxTriCacheSize", arg_check_only);
    r.set_lua("ToggleUnlockAllVehicles", arg_check_only);
    r.set_lua("ViewInvites", arg_check_only);
    r.set_lua("SetUseDebugCamera", arg_check_only);
    r.set_lua("SetMatchMakingState", arg_check_only);
    r.set_lua("DebugPrint", arg_check_only);
    r.set_lua("IsRemoteCompetitor", is_remote_competitor);
    r.set_lua("SetRaceCarInvulnerable", set_race_car_invulnerable);
    r.set_lua("SetRaceCarDamageAllowed", set_race_car_damage_allowed);
    r.set_lua("EnableNitrousEarning", enable_nitrous_earning);
    r.set_lua("SetSpammable", set_spammable);
    r.set_lua("IsNetworkTimerSyncing", is_network_timer_syncing);
    r.set_lua("IsNetworkTimerSynced", is_network_timer_synced);
    r.set_lua("IsOnline", is_online);
    r.set_lua("IsInOnlineEvent", is_in_online_event);
    r.set_lua("GetPlayerVehicleIndex", get_player_vehicle_index);
    r.set_lua("SetPreEventSpeedwallPos", set_pre_event_speedwall_pos);
    r.set_lua("SetTrafficDensity", set_traffic_density);
    r.set_lua("RefreshGarageItemsCache", refresh_garage_items_cache);
    r.set_lua("UnregisterSpeechCommand", unregister_speech_command);
    r.set_lua("SetMinimumCollisionDamage", set_minimum_collision_damage);
    r.set_lua("GetCompetitorType", get_competitor_type);
    r.set_lua("SetUseOverheadCamera", set_use_overhead_camera);
    r.set_lua("SetUseFinishLineCamera", set_use_finish_line_camera);
    r.set_lua("AllowCameraSwitching", allow_camera_switching);
    r.set_lua("SnapCameraToBehindCar", snap_camera_to_behind_car);
    r.set_lua("EnableLocalRaceCarRumble", enable_local_race_car_rumble);
    r.set_lua("IsExitLoadingScreenButtonPressed", is_exit_loading_screen_button_pressed);
    r.set_lua("IsSkipCutsceneButtonPressed", is_skip_cutscene_button_pressed);
    r.set_lua("IsSpectatorCameraButtonPressed", is_spectator_camera_button_pressed);
    r.set_lua("SetCarLocked", set_car_locked);
    r.set_lua("DisableMenuControls", disable_menu_controls);
    r.set_lua("EnableNitrousBalancing", enable_nitrous_balancing);
    r.set_lua("StopAllSounds", stop_all_sounds);
    r.set_lua("StopSpeech", stop_speech);
    r.set_lua("StopGameplayStream", stop_gameplay_stream);
    r.set_lua("StopMusic", stop_music);
    r.set_lua("StopUnwantedSound", stop_unwanted_sound);
    r.set_lua("ClearAllFeedbackQueues", clear_all_feedback_queues);
    r.set_lua("ClearAllNonBackgroundFeedbackQueues", clear_all_non_background_feedback_queues);
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
    for n in [
        "SetHeadlightsState",
        "ShowX360FriendsPopup",
        "ShowPartyInvitePopup",
        "OnPhotoSharedToWall",
        "UpdateSpammableFriends",
        "ShowX360GamerCard",
        "ShowX360GamerCardEasyDrive",
        "ShowX360GamerCardByPlayerIndex",
    ] {
        r.set_lua(n, no_effect);
    }
    r.set_lua("IsFriendsUiOnScreen", is_friends_ui_on_screen);
    r.set_lua("StopTraffic", stop_traffic);
    r.set_lua("ResetTraffic", reset_traffic);
    r.set_lua("StartTraffic", start_traffic);
    r.set_lua("IsStoryMode", is_story_mode);
    r.set_lua("IsChallengeActive", is_challenge_active);
    r.set_lua("SetRearViewMirrorRender", set_rear_view_mirror_render);
    r.set_lua("SetTOSAsLastError", set_tos_as_last_error);
    r.set_lua("SetCancelAsLastError", set_cancel_as_last_error);
    r.set_lua("SetPWMisMatchAsLastError", set_pw_mismatch_as_last_error);
    r.set_lua("SetEmailMisMatchAsLastError", set_email_mismatch_as_last_error);
    r.set_lua("GetSignInError", get_sign_in_error);
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
        assert!(flag(&call(&mut lua, "IsCompetitorActive", vec![Value::Number(3.9)])), "truncated");
        assert!(!flag(&call(&mut lua, "IsCompetitorActive", vec![Value::Number(1.0)])));
        assert!(!flag(&call(&mut lua, "IsCompetitorActive", vec![Value::Number(99.0)])));
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
        assert!(!flag(&call(&mut lua, "GetUseOverheadCamera", vec![])));
        st(&host).sim.overhead_camera = true;
        assert!(flag(&call(&mut lua, "GetUseOverheadCamera", vec![])));
        st(&host).sim.units_metric = true;
        assert!(flag(&call(&mut lua, "AreUnitsMetric", vec![])));
        assert!(!flag(&call(&mut lua, "ShouldLoadingScreenBeShowing", vec![])));
        st(&host).engine.loading_screen_showing = true;
        assert!(flag(&call(&mut lua, "ShouldLoadingScreenBeShowing", vec![])));
        call(&mut lua, "SetCarFrozen", vec![Value::Bool(true)]);
        assert_eq!(st(&host).players.car_frozen, 1.0);
        call(&mut lua, "SetCarFrozen", vec![Value::Bool(false)]);
        assert_eq!(st(&host).players.car_frozen, 0.0);
        assert!(st(&host).unported.is_empty());
    }

    #[test]
    fn batch_six() {
        let (mut lua, host) = setup();
        call(&mut lua, "SetMinimumCollisionDamage", vec![Value::Number(1.0)]);
        {
            let mut s = st(&host);
            s.players.slots[0] = Some(Player { kind: 0, ..Default::default() });
            s.players.slots[1] = Some(Player { kind: 2, ..Default::default() });
        }
        let t = |lua: &mut Lua, i: f64| call(lua, "GetCompetitorType", vec![Value::Number(i)]);
        assert_eq!(t(&mut lua, 0.0)[0].as_str_lossy().as_deref(), Some("LocalPlayer"));
        assert_eq!(t(&mut lua, 1.0)[0].as_str_lossy().as_deref(), Some("AI"));
        assert_eq!(t(&mut lua, 5.0)[0].as_str_lossy().as_deref(), Some(""));
        call(&mut lua, "SetUseOverheadCamera", vec![Value::Bool(true)]);
        call(&mut lua, "SetUseFinishLineCamera", vec![Value::Bool(true), Value::Number(3.0)]);
        call(&mut lua, "SetUseFinishLineCamera", vec![Value::Bool(false), Value::Number(3.0)]);
        call(&mut lua, "SnapCameraToBehindCar", vec![]);
        call(&mut lua, "EnableLocalRaceCarRumble", vec![Value::Bool(true)]);
        call(&mut lua, "StopAllSounds", vec![]);
        call(&mut lua, "StopSpeech", vec![]);
        call(&mut lua, "StopGameplayStream", vec![]);
        call(&mut lua, "StopMusic", vec![]);
        call(&mut lua, "StopUnwantedSound", vec![]);
        call(&mut lua, "ClearAllFeedbackQueues", vec![]);
        call(&mut lua, "ClearAllNonBackgroundFeedbackQueues", vec![]);
        assert_eq!(
            st(&host).requests,
            [
                Request::CameraOverhead { on: true },
                Request::CameraFinishLine { on: true, id: 3 },
                Request::CameraFinishLine { on: false, id: -1 },
                Request::CameraSnapBehind,
                Request::RumbleEnable { on: true },
                Request::StopSounds { kind: 0 },
                Request::StopSounds { kind: 2 },
                Request::StopSounds { kind: 4 },
                Request::StopMusic,
                Request::StopUnwantedSound,
                Request::ClearFeedbackQueues { background: true },
                Request::ClearFeedbackQueues { background: false },
            ]
        );
        call(&mut lua, "AllowCameraSwitching", vec![Value::Bool(true)]);
        assert!(st(&host).sim.allow_camera_switching);
        call(&mut lua, "EnableNitrousBalancing", vec![]);
        assert!(st(&host).sim.nitrous_balancing);
        call(&mut lua, "EnableNitrousBalancing", vec![Value::Bool(false)]);
        assert!(!st(&host).sim.nitrous_balancing);
        st(&host).sim.button_1b4 = 2;
        assert!(flag(&call(&mut lua, "IsExitLoadingScreenButtonPressed", vec![])));
        assert!(flag(&call(&mut lua, "IsSkipCutsceneButtonPressed", vec![])));
        assert!(!flag(&call(&mut lua, "IsSpectatorCameraButtonPressed", vec![])));
        st(&host).sim.button_19c = 2;
        assert!(flag(&call(&mut lua, "IsSpectatorCameraButtonPressed", vec![])));
        call(&mut lua, "SetCarLocked", vec![Value::Bool(true)]);
        call(&mut lua, "SetCarLocked", vec![Value::Bool(true)]);
        call(&mut lua, "DisableMenuControls", vec![Value::Bool(true)]);
        let (m, c) = {
            let s = st(&host);
            (s.sim.input_masks[1], s.sim.input_mask_counts[1])
        };
        assert_eq!((m, c), (2, 1));
        call(&mut lua, "SetCarLocked", vec![Value::Bool(false)]);
        call(&mut lua, "DisableMenuControls", vec![Value::Bool(false)]);
        assert_eq!(st(&host).sim.input_masks, [0; 8]);
        assert_eq!(st(&host).sim.input_mask_counts, [0; 8]);
        assert!(st(&host).unported.is_empty());
    }

    #[test]
    fn seventh_batch() {
        let (mut lua, host) = setup();
        for n in [
            "SetHeadlightsState",
            "ShowX360FriendsPopup",
            "ShowPartyInvitePopup",
            "OnPhotoSharedToWall",
            "UpdateSpammableFriends",
            "ShowX360GamerCard",
            "ShowX360GamerCardEasyDrive",
            "ShowX360GamerCardByPlayerIndex",
        ] {
            assert!(call(&mut lua, n, vec![Value::Number(1.0)]).is_empty(), "{n}");
        }
        assert!(st(&host).requests.is_empty());
        assert!(!flag(&call(&mut lua, "IsFriendsUiOnScreen", vec![])));
        call(&mut lua, "StopTraffic", vec![]);
        call(&mut lua, "ResetTraffic", vec![]);
        call(&mut lua, "StartTraffic", vec![]);
        assert_eq!(
            st(&host).requests,
            [
                Request::TrafficMessage { kind: 6 },
                Request::TrafficMessage { kind: 8 },
                Request::TrafficMessage { kind: 0xa }
            ]
        );
        st(&host).game_modes.story_mode = false;
        assert!(!flag(&call(&mut lua, "IsStoryMode", vec![])));
        st(&host).game_modes.story_mode = true;
        assert!(flag(&call(&mut lua, "IsStoryMode", vec![])));
        assert!(!flag(&call(&mut lua, "IsChallengeActive", vec![])));
        st(&host).game_modes.current_event = 7;
        assert!(flag(&call(&mut lua, "IsChallengeActive", vec![])));
        call(&mut lua, "SetRearViewMirrorRender", vec![Value::Bool(true)]);
        assert!(st(&host).sim.rear_view_mirror);
        call(&mut lua, "SetRearViewMirrorRender", vec![Value::Bool(false)]);
        assert!(!st(&host).sim.rear_view_mirror);
        assert_eq!(num(&call(&mut lua, "GetSignInError", vec![])), 0.0);
        for (n, code) in [
            ("SetTOSAsLastError", 2.0),
            ("SetCancelAsLastError", 3.0),
            ("SetPWMisMatchAsLastError", 26.0),
            ("SetEmailMisMatchAsLastError", 30.0),
        ] {
            call(&mut lua, n, vec![]);
            assert_eq!(num(&call(&mut lua, "GetSignInError", vec![])), code, "{n}");
            assert_eq!(st(&host).sim.sign_in_error.arg, -1);
        }
    }

    #[test]
    fn batch_eight() {
        let (mut lua, host) = setup();
        for n in [
            "FlagUseMaxTriCacheSize",
            "ToggleUnlockAllVehicles",
            "ViewInvites",
            "SetUseDebugCamera",
            "SetMatchMakingState",
            "DebugPrint",
        ] {
            assert!(call(&mut lua, n, vec![Value::Number(1.0)]).is_empty(), "{n}");
        }
        {
            let mut s = st(&host);
            s.players.slots[0] = Some(Player { kind: 0, vehicle: 4, ..Default::default() });
            s.players.slots[2] = Some(Player { kind: 1, vehicle: 9, ..Default::default() });
            s.players.local = Some(0);
        }
        let n = |v: f64| vec![Value::Number(v)];
        assert!(flag(&call(&mut lua, "IsRemoteCompetitor", n(2.0))));
        assert!(!flag(&call(&mut lua, "IsRemoteCompetitor", n(0.0))));
        assert!(!flag(&call(&mut lua, "IsRemoteCompetitor", n(5.0))));
        assert_eq!(num(&call(&mut lua, "GetPlayerVehicleIndex", vec![])), 4.0);
        assert_eq!(num(&call(&mut lua, "GetPlayerVehicleIndex", n(2.0))), 9.0);
        assert_eq!(num(&call(&mut lua, "GetPlayerVehicleIndex", n(7.0))), -1.0);
        call(&mut lua, "SetRaceCarInvulnerable", vec![Value::Number(2.0), Value::Bool(true)]);
        call(&mut lua, "SetRaceCarDamageAllowed", vec![Value::Number(1.0), Value::Bool(false)]);
        call(&mut lua, "SetTrafficDensity", vec![Value::Bool(true), Value::Number(0.5)]);
        call(&mut lua, "SetTrafficDensity", vec![Value::Bool(false), Value::Number(0.5)]);
        call(&mut lua, "RefreshGarageItemsCache", vec![]);
        call(&mut lua, "UnregisterSpeechCommand", n(6.9));
        assert_eq!(
            st(&host).requests,
            [
                Request::RaceCarFlag { kind: 0x1f, competitor: 2, on: true },
                Request::RaceCarFlag { kind: 0x1e, competitor: 1, on: false },
                Request::TrafficDensity { on: true, density: 0.5 },
                Request::TrafficDensity { on: false, density: 0.0 },
                Request::RefreshGarageItemsCache,
                Request::UnregisterSpeechCommand { id: 6 },
            ]
        );
        call(&mut lua, "EnableNitrousEarning", vec![Value::Bool(true)]);
        assert!(st(&host).sim.nitrous_earning);
        call(&mut lua, "SetSpammable", vec![Value::Bool(true), Value::Bool(false), Value::Bool(true)]);
        assert_eq!(st(&host).sim.spammable, [true, false, true]);
        call(&mut lua, "SetPreEventSpeedwallPos", n(12.7));
        assert_eq!(st(&host).sim.speedwall_pos, 12);
        st(&host).sim.net_timer_state = 1;
        assert!(flag(&call(&mut lua, "IsNetworkTimerSyncing", vec![])));
        assert!(!flag(&call(&mut lua, "IsNetworkTimerSynced", vec![])));
        st(&host).sim.net_timer_state = 2;
        assert!(flag(&call(&mut lua, "IsNetworkTimerSynced", vec![])));
        assert!(!flag(&call(&mut lua, "IsOnline", vec![])));
        st(&host).sim.online = true;
        st(&host).sim.in_online_event = true;
        assert!(flag(&call(&mut lua, "IsOnline", vec![])));
        assert!(flag(&call(&mut lua, "IsInOnlineEvent", vec![])));
        assert!(st(&host).unported.is_empty());
    }
}
