//! Tests of the ported HSM hooks, through the registry as the HSM host calls them; expectations from the handlers
//! (hooks_flow.rs, hooks_event.rs, hooks_ui.rs).

use super::game_modes::EventInfo;
use super::players::Player;
use super::progression::{INTRO_PLAYED, LAST_EVENT};
use super::{LogicState, Registry, Request};
use crate::hsm::behaviours::{FALSE, TRUE};
use crate::hsm::{EXIT, Key};
use mw_formats::json::Json;

fn call(r: &Registry, s: &mut LogicState, name: &str, params: Option<&str>) -> Key {
    let json = params.map(|p| Json::parse(p).unwrap());
    let k = r.call_hook(s, name, json.as_ref()).unwrap();
    assert!(s.unported.is_empty(), "{name} is not ported");
    k
}

fn with_local_player() -> LogicState {
    let mut s = LogicState::default();
    s.players.local = Some(0);
    s.players.slots[0] = Some(Player { vehicle: 3, vehicle_alive: true, heat_3d8: 1.0, heat_3dc: 2.0, heat_3e8: 3.0, ..Default::default() });
    s
}

#[test]
fn flags_and_their_tests() {
    let r = Registry::original();
    let mut s = LogicState::default();
    s.game_modes.new_game = true;
    assert_eq!(call(&r, &mut s, "IsNewGame", None), TRUE);
    assert_eq!(call(&r, &mut s, "ClearNewGameFlag", None), EXIT);
    assert_eq!(call(&r, &mut s, "IsNewGame", None), FALSE);
    assert_eq!(call(&r, &mut s, "SetGameModeWon", Some(r#"{ "Won": true }"#)), EXIT);
    assert!(s.game_modes.won);
    call(&r, &mut s, "SetGameModeWon", Some(r#"{ "Won": false }"#));
    assert!(!s.game_modes.won);
    call(&r, &mut s, "SetAllowLoadingScreen", Some(r#"{ "lbEnable": true }"#));
    assert!(s.game_modes.allow_loading_screen);
    assert_eq!(call(&r, &mut s, "HasIntroEasyDriveBeenActive", None), FALSE);
    call(&r, &mut s, "SetHasIntroEasyDriveBeenActive", None);
    assert_eq!(call(&r, &mut s, "HasIntroEasyDriveBeenActive", None), TRUE);
    assert_eq!(call(&r, &mut s, "IsChallengeActive", None), FALSE);
    s.game_modes.current_event = 7;
    assert_eq!(call(&r, &mut s, "IsChallengeActive", None), TRUE);
    assert_eq!(call(&r, &mut s, "ShouldShowSafehouseExitSequence", None), TRUE);
    assert_eq!(call(&r, &mut s, "ShowX360GamerCard", None), EXIT);
    assert!(s.requests.is_empty());
}

#[test]
fn new_game_and_continue() {
    let r = Registry::original();
    let mut s = with_local_player();
    s.config.freedrive_mode = 390145;
    s.config.default_rollout = 523000;
    s.game.listed_events = vec![20, 30];
    s.game_modes.events = vec![
        EventInfo { id: 10, mode: 390145, available: true, rollout_ok: true, ..Default::default() },
        EventInfo { id: 30, mode: 390145, available: true, rollout_ok: true, ..Default::default() },
    ];
    call(&r, &mut s, "SetNewGame", Some(r#"{ "NewGame": true }"#));
    assert!(s.game_modes.new_game && !s.game_modes.recap && s.game.save_pending && s.progression.save_requested);
    // First pass: the listed events only.
    assert_eq!((s.game.pending_event, s.game.pending_event_aux), (30, -1));
    assert_eq!(s.players.local_player().unwrap().heat_3dc, 0.0);
    assert_eq!(s.requests, [Request::NewGame { new_game: true, default_rollout: 523000, saved_position: -1 }]);
    // Continue: recap on; nothing listed → second pass takes the first available.
    let mut s = with_local_player();
    s.config.freedrive_mode = 390145;
    s.game_modes.events = vec![EventInfo { id: 10, mode: 390145, available: true, rollout_ok: true, ..Default::default() }];
    call(&r, &mut s, "SetNewGame", Some(r#"{ "NewGame": false }"#));
    assert!(s.game_modes.recap && !s.game_modes.new_game);
    assert_eq!(s.game.pending_event, 10);
    assert_eq!(s.players.local_player().unwrap().heat_3dc, 2.0);
    // No free-drive event at all: −1.
    let mut s = LogicState::default();
    call(&r, &mut s, "SetFreedriveEventToPlay", None);
    assert_eq!(s.game.pending_event, u32::MAX);
}

#[test]
fn new_game_intro() {
    let r = Registry::original();
    let mut s = LogicState::default();
    // No `last_played` row (−1 < 1): play the intro.
    assert_eq!(call(&r, &mut s, "ShouldPlayNewGameIntro", None), TRUE);
    s.config.intro_event = 1560354;
    s.config.intro_race_event = 1909234;
    call(&r, &mut s, "SetIntroEventNext", None);
    assert_eq!(s.progression.last_played(INTRO_PLAYED), 0);
    assert_eq!((s.game_modes.requested_event, s.game_modes.event_requested), (1560354, true));
    assert_eq!(s.game.pending_event, 1560354);
    s.game_modes.current_event = 1909234;
    assert_eq!(call(&r, &mut s, "IsIntroRace", None), TRUE);
    call(&r, &mut s, "SetNewGameIntroCompleted", None);
    assert_eq!(s.progression.last_played(INTRO_PLAYED), 1);
    assert_eq!(call(&r, &mut s, "IsIntroRace", None), FALSE);
    assert_eq!(call(&r, &mut s, "ShouldPlayNewGameIntro", None), FALSE);
    assert_eq!(s.requests.last(), Some(&Request::SaveProfile));
    // A pending online event: never.
    let mut s = LogicState::default();
    s.game.pending_event = 5;
    s.game_modes.events = vec![EventInfo { id: 5, online: true, ..Default::default() }];
    assert_eq!(call(&r, &mut s, "ShouldPlayNewGameIntro", None), FALSE);
}

#[test]
fn car_control_for_the_intro() {
    let r = Registry::original();
    let mut s = LogicState::default();
    s.game.input_blocks.masks[1] = 1 << 6 | 1;
    s.game.input_blocks.counts[1] = 2;
    call(&r, &mut s, "HACKSuspendCarControlForIntro", None);
    assert_eq!(s.game.input_blocks.suspend_time, 3.0);
    call(&r, &mut s, "HACKResumeCarControlAfterIntro", None);
    assert_eq!((s.game.input_blocks.masks[1], s.game.input_blocks.counts[1]), (1, 1));
    assert_eq!(s.game.input_blocks.suspend_time, 0.0);
    // Clearing a clear bit keeps the count.
    call(&r, &mut s, "HACKResumeCarControlAfterIntro", None);
    assert_eq!(s.game.input_blocks.counts[1], 1);
}

#[test]
fn progression_ready() {
    let r = Registry::original();
    let mut s = with_local_player();
    s.progression.loaded = true;
    s.progression.world_count_ready = true;
    assert_eq!(call(&r, &mut s, "IsProgressionReady", None), FALSE);
    s.progression.collectables_ready = true;
    assert_eq!(call(&r, &mut s, "IsProgressionReady", None), TRUE);
    s.players.local = None;
    assert_eq!(call(&r, &mut s, "IsProgressionReady", None), FALSE);
}

#[test]
fn mode_timer_and_events() {
    let r = Registry::original();
    let mut s = LogicState::default();
    call(&r, &mut s, "StartGameModeTimer", None);
    call(&r, &mut s, "StartGameModeTimer", None);
    call(&r, &mut s, "StopGameModeTimer", None);
    assert_eq!(s.requests, [Request::GameModeTimer { running: true }, Request::GameModeTimer { running: false }]);
    // Retry: the last event from `last_played`[1]; the handle only for an online event of that id.
    s.progression.set_last_played(LAST_EVENT, 42);
    s.game_modes.events = vec![EventInfo { id: 42, online: true, ..Default::default() }];
    call(&r, &mut s, "RetryLastEvent", None);
    assert_eq!((s.game_modes.requested_event, s.game_modes.requested_online_event), (42, Some(0)));
    call(&r, &mut s, "StartEventFromEasyDrive", Some(r#"{ "lpcValue": "-1" }"#));
    assert_eq!(s.game_modes.requested_event, 42);
    call(&r, &mut s, "StartEventFromEasyDrive", Some(r#"{ "lpcValue": "730004" }"#));
    assert_eq!((s.game_modes.requested_event, s.game_modes.requested_online_event), (730004, None));
    s.game_modes.has_transitioned_events = true;
    call(&r, &mut s, "ResetGameMode", None);
    assert!(!s.game_modes.has_transitioned_events);
    s.game_modes.current_mode = 390185;
    s.config.escape_mode = 390185;
    assert_eq!(call(&r, &mut s, "ShouldShowEscapeSequence", None), TRUE);
}

#[test]
fn awards() {
    let r = Registry::original();
    let mut s = LogicState::default();
    s.game_modes.current_event = 9;
    s.game_modes.award_amount = 500;
    s.game_modes.awards = vec![1, 2];
    s.game_modes.results_2300[0x18 / 4] = 5;
    call(&r, &mut s, "AwardPostEventAwards", None);
    assert_eq!(
        s.requests,
        [Request::AwardPostEventAwards { event: 9, amount: 500, award_2ce8: 0, awards: vec![1, 2] }]
    );
    call(&r, &mut s, "ClearPostEventAwards", None);
    assert!(s.game_modes.awards.is_empty() && s.game_modes.award_amount == 0 && s.game_modes.results_2300[6] == 0);
}

#[test]
fn players_cops_and_respawns() {
    let r = Registry::original();
    let mut s = with_local_player();
    s.players.slots[5] = Some(Player { kind: 2, cop: true, ..Default::default() });
    s.players.slots[6] = Some(Player { kind: 1, cop: true, ..Default::default() });
    s.players.slots[12] = Some(Player { kind: 2, loaded: true, ..Default::default() });
    s.players.slots[13] = Some(Player { kind: 2, loaded: false, ..Default::default() });
    call(&r, &mut s, "RemoveAllCops", None);
    assert_eq!(s.requests, [Request::RemovePlayer { slot: 5, reason: 3 }]);
    assert_eq!(call(&r, &mut s, "AreAllAIPlayersLoaded", None), FALSE);
    s.players.slots[13] = None;
    assert_eq!(call(&r, &mut s, "AreAllAIPlayersLoaded", None), TRUE);
    s.game.pending_event = 77;
    s.requests.clear();
    call(&r, &mut s, "RequestRespawnAtEventStart", None);
    call(&r, &mut s, "RequestRespawnToLastSwapSpot", None);
    assert_eq!(
        s.requests,
        [Request::Respawn { event: 77, to_swap_spot: false }, Request::Respawn { event: 0, to_swap_spot: true }]
    );
    let p = s.players.local_player().unwrap();
    assert!(p.respawn_to_swap_spot && p.respawn_event == 0);
    call(&r, &mut s, "ResetChaseStats", None);
    let p = s.players.local_player().unwrap();
    assert_eq!((p.heat_3d8, p.heat_3dc, p.heat_3e8), (0.0, 0.0, 0.0));
    assert_eq!(call(&r, &mut s, "HasLocalPlayerSetRollout", None), TRUE);
    // Damage reset only while the GMM has not transitioned events.
    s.requests.clear();
    s.game_modes.has_transitioned_events = true;
    call(&r, &mut s, "ResetLocalPlayerDamage", None);
    assert!(s.requests.is_empty());
    s.game_modes.has_transitioned_events = false;
    call(&r, &mut s, "ResetLocalPlayerDamage", None);
    call(&r, &mut s, "ClearNitrous", None);
    assert_eq!(s.requests, [Request::ResetVehicleDamage { vehicle: 3 }, Request::ClearNitrous { slot: 0 }]);
}

#[test]
fn ui_hooks() {
    let r = Registry::original();
    let mut s = LogicState::default();
    s.game.loading_screen_requests = 0x24;
    s.game.loading_screen_2d21c = 9;
    call(&r, &mut s, "ToggleLoadingScreenOff", None);
    assert_eq!((s.game.loading_screen_requests, s.game.loading_screen_2d21c), (0x4, 0));
    call(&r, &mut s, "StopAllSequences", None);
    call(&r, &mut s, "OnStartEvent", None);
    assert_eq!(
        s.requests,
        [Request::ClearHudMessageQueues, Request::StopAllSequences, Request::PlayUiSound { field: 0x60 }]
    );
}
