//! The game-mode manager (GMM, G+0x2f3fc) running a mode's HSM machine: `RUNGAMEMODE` asks for the event with
//! [`Request::StartGameMode`] (`FUN_00701cb0`, its logic fields in [`start_game_mode`]); the GMM's per-frame update
//! (`FUN_0070d580`, its machine part in [`update`]) starts the mode's top-level state once the event's data and the
//! mode's weapon list are loaded. The original calls the HSM manager's update (`FUN_00776010`) and then the GMM's in
//! the same frame (`FUN_006abc10`), so whoever owns the [`Hsm`] calls [`update`] right after [`Hsm::update`]; the
//! interpreter's host contract is unchanged (Requests out, Queries in).
//!
//! Free drive: the boot's `SetNewGame` makes the free-drive event the pending one (G+0x2f6f8, `FUN_0068fa10`: 390703
//! "FAIRHAVEN", mode 390145), "Run Game Mode" (390011) starts it, and the GMM starts the mode's instance 390147
//! "Freedrive Game Mode" (`GameMode` +0x1c; read live in both snapshots: GMM+0x2cfc = its serial).

use std::cell::RefCell;

use super::{LogicState, Request};
use crate::hsm::{Host, Hsm};

/// A `GameMode` item (GAMEMODES.BNDL) as the GMM uses it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModeInfo {
    /// +0x18: its id (what an event's +0x1c names).
    pub id: u32,
    /// +0x1c `0501d88b`: the instance id of its top-level HSM state.
    pub hsm: u32,
    /// +0x40 `WeaponList`: set on the local player before the machine starts.
    pub weapon_list: u32,
    /// +0x20 `8e9585cb`: the HUD's top-level instance (`Mode.HudHsm`).
    pub hud_hsm: u32,
    /// The `GameMode` object's scalar fields by field hash, what `Mode.Data.<field>` reads (the binding hash of a
    /// name of 4+ bytes is the GeneSys field hash): [`super::mode_view::data_fields`].
    pub data: Vec<(u32, crate::binding::Value)>,
}

/// `FUN_006e5ea0`(event): the event's mode is a free-drive mode (`GameLogicConfig` +0xd8 or +0x130).
pub fn is_freedrive_event(s: &LogicState, event: u32) -> bool {
    let mode = event_mode(s, event);
    mode == s.config.freedrive_mode || mode == s.config.online_freedrive_mode
}

/// `FUN_006e5dc0`(event) +0x1c: the event's game mode; an unknown event gives the null handle (`DAT_01179528`),
/// read here as mode 0.
pub fn event_mode(s: &LogicState, event: u32) -> u32 {
    s.game_modes.events.iter().find(|e| e.id == event).map_or(0, |e| e.mode)
}

/// `FUN_00701cb0`(event) on the GMM, the fields the game logic reads. Without `HasTransitionedEvents` (+0x2e7f) the
/// current event (+0x2ce0) becomes the pending one (G+0x2f6f8; `RUNGAMEMODE` passes that same value), the mode
/// handle (+0x2cf4) its event's mode, the machine serial (+0x2cfc) −1, and `IsFreedriveEvent` (+0x2e90) is
/// `FUN_006e5ea0`. Not ported: the event data manager's own start (`FUN_006f06d0`), the profile and player
/// set-up calls (`FUN_006a0cf0`, `FUN_0065ff60`, `FUN_00844fb0`, `FUN_006a16a0`), the timers and the other flags
/// (+0x2e8b..+0x2e93 compare the event with `GameLogicConfig` ids), the trailing calls (`FUN_006f89f0`,
/// `FUN_006d1380`, `FUN_006bb6a0`, `FUN_008432d0`, `FUN_006835c0`, `FUN_00843380`, `FUN_0071d1e0`): the runtime's
/// [`Request::StartGameMode`].
pub fn start_game_mode(s: &mut LogicState, event: u32) {
    if s.game_modes.has_transitioned_events {
        return;
    }
    let pending = s.game.pending_event;
    s.game_modes.current_event = pending;
    s.game_modes.current_mode = event_mode(s, event);
    s.game_modes.machine = None;
    s.game_modes.freedrive_event = is_freedrive_event(s, event);
}

/// `FUN_006dfe00`: an event runs (+0x2ce0 ≠ 0) and its data is loaded ((GMM+4)+0xdc == 4).
pub fn mode_started(s: &LogicState) -> bool {
    s.game_modes.current_event != 0 && s.game_modes.event_data_ready
}

/// `FUN_0070d580`, the machine part: with no machine, a started mode (`FUN_006dfe00`) and an offline event, set the
/// mode's weapon list on the local player (`FUN_006bb720`) and, once it is loaded (`FUN_006bb6f0`), start the
/// mode's top-level instance (`FUN_00775ed0`(the HSMData of (GMM+4)+0x104, `GameMode` +0x1c, GMM+0x2d00)) and keep
/// its serial (+0x2cfc; −1 when the instance does not exist, tried again next frame). Not ported: online events
/// (`FUN_006ded90` / `FUN_004c1ec0` decide; held here), the mode intro set-up after the start (`FUN_0070caa0`), the
/// running mode's upkeep (`FUN_007090f0`) and the end of the event once its machine finished (`FUN_0070b1e0`),
/// and the event data managers' own updates at the top of the function.
pub fn update(state: &RefCell<LogicState>, hsm: &mut Hsm, host: &mut dyn Host) {
    let instance = {
        let mut s = state.borrow_mut();
        if let Some(machine) = s.game_modes.machine {
            if !hsm.is_running(machine) {
                machine_finished(&mut s);
            }
            return;
        }
        if !mode_started(&s) || s.game_modes.online_event {
            return;
        }
        let current = s.game_modes.current_mode;
        let Some((hsm, weapon_list)) =
            s.game_modes.modes.iter().find(|m| m.id == current).map(|m| (m.hsm, m.weapon_list))
        else {
            return;
        };
        s.game_modes.weapon_list = weapon_list;
        if s.game_modes.weapon_list_loaded != weapon_list {
            return;
        }
        hsm
    };
    let serial = hsm.start(instance, host);
    let mut s = state.borrow_mut();
    s.game_modes.machine = serial;
    if serial.is_some() {
        after_start(&mut s);
    }
}

/// `FUN_0070caa0` (called right after the machine started), the part that decides what follows the event: with
/// the event record's +0x8c set, that event is requested with flag 1 (`FUN_006e74f0`); else, in story mode, for an
/// event that is not free drive (+0x2e90) and not the game intro (+0x2e91), the free-drive event
/// (`FUN_0068fa10`, [`pick_freedrive_event`]) is requested with flag 1 (the way back to free roam, the next mode
/// then starts with +0x2e7f set); else the request is cleared (+0x2ce4 = 0, +0x2e7e = 0). Not ported: the rest of
/// the function (rollout, weapon, timer and camera set-up for the event, online branch).
pub fn after_start(s: &mut LogicState) {
    let next = s.game_modes.events.iter().find(|e| e.id == s.game_modes.current_event).map_or(0, |e| e.next_event);
    let gm = &s.game_modes;
    if next != 0 {
        s.game_modes.request_event(next, true);
    } else if gm.story_mode && !gm.freedrive_event && !gm.in_game_intro_freedrive {
        let event = super::hooks_flow::pick_freedrive_event(s);
        s.game_modes.request_event(event, true);
    } else {
        s.game_modes.request_event(0, false);
    }
}

/// `FUN_0070d580`, the part after the machine ended (`FUN_00774f70` false): while an event is requested
/// (+0x2e7e) its data is loaded first (`FUN_006f1720`: +0x2d0c 0 → 1 → `FUN_006f06d0`(requested) → 2 → 3 once
/// (GMM+8)+0xdc == 4, one step per frame), then `FUN_0070b1e0` ends the event: [`end_of_event`]. Not ported:
/// the upkeep while the machine runs (`FUN_007090f0`).
fn machine_finished(s: &mut LogicState) {
    if s.game_modes.event_requested && s.game_modes.transition_state != 3 {
        let gm = &mut s.game_modes;
        if gm.transition_state == 0 {
            gm.transition_state = 1;
        }
        if gm.transition_state == 1 {
            let event = gm.requested_event;
            s.requests.push(Request::LoadEventData { event });
        }
        let gm = &mut s.game_modes;
        if gm.transition_state <= 2 {
            gm.transition_state = if gm.event_data_ready { 3 } else { 2 };
        }
        return;
    }
    end_of_event(s);
}

/// `FUN_0070b1e0`, the event changes that the game logic reads. The machine serial (+0x2cfc) is −1 again and the
/// "mode is on" byte (+0x2e78) 1 for the next event. With an event requested (+0x2ce4) and story mode (+0x2e7a;
/// the GMM constructor sets it, 0x70c603): `FUN_0063e380`(requested, online) makes it the pending event
/// (G+0x2f6f8, aux −1), `FUN_006e6bf0` swaps the event data managers and takes it as the current event (+0x2ce0
/// = G+0x2f6f8, +0x2cf4 its mode, +0x2d0c = 0, +0x2e7f = +0x2e80 the request's flag; +0x2e8b `IsRaceEvent` = the mode is `GameLogicConfig`'s
/// +0x170 mode, whose field hash is not known: left to the runtime), +0x2e8a `LastEventWasRace` = the old +0x2e8b. Without a request or story mode `FUN_006e6af0` is the
/// hard reset (world unloaded +0x2e77, no current event, +0x2ce4 = 0, +0x2e8a / +0x2e8b = 0). Not ported: the
/// player, vehicle and camera resets (`FUN_006bf270`, `FUN_006a5820`, `FUN_00844fb0`, ...), the retry /
/// online branches, the 0x2e8d / 0x2e8f classifications of the mode.
pub fn end_of_event(s: &mut LogicState) {
    let was_race = s.game_modes.race_event;
    s.game_modes.machine = None;
    s.game_modes.won = true;
    s.game_modes.timer_running = false;
    s.game_modes.end_flag_2e75 = false;
    s.game_modes.intro_load_complete = false;
    s.game_modes.switch_to_gameplay_load_world = false;
    s.game_modes.transition_state = 0;
    if s.game_modes.requested_event != 0 && s.game_modes.story_mode {
        let event = s.game_modes.requested_event;
        s.game.pending_event = event;
        s.game.pending_event_aux = -1;
        s.game.pending_event_flag = s.game_modes.online_event;
        s.game_modes.current_event = event;
        s.game_modes.current_mode = event_mode(s, event);
        s.game_modes.has_transitioned_events = s.game_modes.event_request_flag;
        s.game_modes.freedrive_event = is_freedrive_event(s, event);
        s.game_modes.last_event_was_race = was_race;
    } else {
        s.game_modes.requested_event = 0;
        s.game_modes.event_requested = false;
        s.game_modes.current_event = 0;
        s.game_modes.current_mode = 0;
        s.game_modes.world_loaded = false;
        s.game_modes.last_event_was_race = false;
        s.game_modes.race_event = false;
    }
}

/// What the logic host does with a request itself before the runtime gets it (the parts of the original's call
/// that only touch logic state).
pub fn apply(s: &mut LogicState, r: &Request) {
    match r {
        Request::StartGameMode { event } => start_game_mode(s, *event),
        Request::AdjustShowWorldCount { delta } => s.game.show_world_count += delta,
        Request::SetInputBlock { group, bit, set: true } if (*group as usize) < s.game.input_blocks.masks.len() => {
            s.game.input_blocks.set(*group as usize, *bit)
        }
        Request::SetInputBlock { group, bit, set: false } if (*group as usize) < s.game.input_blocks.masks.len() => {
            s.game.input_blocks.clear(*group as usize, *bit)
        }
        Request::SetGameModeWon { won } => s.game_modes.won = *won,
        Request::SetEndReason { reason } => s.game_modes.end_reason = i32::from(*reason),
        Request::SetCreditsTransition { on } => s.game_modes.credits_transition = *on,
        // `FUN_0063e380` on G+0x2f4c8 (`DBGSTARTFRID`).
        Request::SetPendingEvent { event, online } => {
            s.game.pending_event = *event;
            s.game.pending_event_aux = -1;
            s.game.pending_event_flag = *online;
        }
        Request::SetKinectSelectedClass { class } => s.hook_online.kinect_selected_class = *class,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::natives::LogicHost;
    use crate::natives::game_modes::EventInfo;

    fn state() -> LogicState {
        let mut s = LogicState::default();
        s.config.freedrive_mode = 10;
        s.game_modes.story_mode = true;
        s.game_modes.events = vec![
            EventInfo { id: 1, mode: 10, available: true, rollout_ok: true, ..Default::default() },
            EventInfo { id: 2, mode: 20, ..Default::default() },
        ];
        s
    }

    #[test]
    fn the_constructor_leaves_the_mode_on_in_story_mode() {
        let h = LogicHost::default();
        let s = h.state.borrow();
        assert!(s.game_modes.won && s.game_modes.story_mode && s.game_modes.show_takedown_cameras);
    }

    #[test]
    fn a_finished_machine_waits_for_the_next_events_data_then_changes_event() {
        let mut s = state();
        s.game_modes.current_event = 1;
        s.game_modes.machine = Some(0);
        s.game_modes.request_event(2, false);
        // Step 1: the load is requested; the data is not in.
        machine_finished(&mut s);
        assert_eq!((s.game_modes.transition_state, s.requests.len()), (2, 1));
        assert!(matches!(s.requests[0], Request::LoadEventData { event: 2 }));
        machine_finished(&mut s);
        assert_eq!(s.game_modes.transition_state, 2);
        assert_eq!(s.game_modes.machine, Some(0));
        // The data is in: ready, then the event changes with the next call.
        s.game_modes.event_data_ready = true;
        machine_finished(&mut s);
        assert_eq!(s.game_modes.transition_state, 3);
        machine_finished(&mut s);
        assert_eq!((s.game_modes.current_event, s.game.pending_event, s.game.pending_event_aux), (2, 2, -1));
        assert_eq!((s.game_modes.current_mode, s.game_modes.machine, s.game_modes.transition_state), (20, None, 0));
        assert!(!s.game_modes.has_transitioned_events && !s.game_modes.freedrive_event);
    }

    #[test]
    fn without_a_request_or_story_mode_the_event_ends_for_good() {
        let mut s = state();
        s.game_modes.current_event = 1;
        s.game_modes.current_mode = 10;
        s.game_modes.machine = Some(0);
        s.game_modes.world_loaded = true;
        end_of_event(&mut s);
        assert_eq!((s.game_modes.current_event, s.game_modes.machine, s.game_modes.world_loaded), (0, None, false));
        s.game_modes.current_event = 1;
        s.game_modes.request_event(2, false);
        s.game_modes.story_mode = false;
        end_of_event(&mut s);
        assert_eq!((s.game_modes.current_event, s.game_modes.requested_event), (0, 0));
    }

    #[test]
    fn a_started_race_asks_for_free_roam_next_with_the_transition_flag() {
        let mut s = state();
        s.game_modes.current_event = 2;
        after_start(&mut s);
        assert_eq!((s.game_modes.requested_event, s.game_modes.event_request_flag), (1, true));
        // Free roam itself asks for nothing.
        s.game_modes.current_event = 1;
        s.game_modes.freedrive_event = true;
        after_start(&mut s);
        assert_eq!((s.game_modes.requested_event, s.game_modes.event_requested), (0, false));
        // The event record's own next event wins.
        s.game_modes.events[1].next_event = 7;
        s.game_modes.current_event = 2;
        after_start(&mut s);
        assert_eq!(s.game_modes.requested_event, 7);
    }
}
