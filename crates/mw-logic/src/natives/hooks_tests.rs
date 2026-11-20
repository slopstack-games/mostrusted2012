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
    s.players.slots[0] = Some(Player {
        vehicle: 3,
        vehicle_alive: true,
        heat_3d8: 1.0,
        heat_3dc: 2.0,
        heat_3e8: 3.0,
        ..Default::default()
    });
    s
}

#[test]
fn flags_and_their_tests() {
    let r = Registry::original_with_parked();
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
    let r = Registry::original_with_parked();
    let mut s = with_local_player();
    s.config.freedrive_mode = 390145;
    s.config.default_rollout = 523000;
    s.game.listed_events = vec![20, 30];
    s.game_modes.events = vec![
        EventInfo { id: 10, mode: 390145, available: true, rollout_ok: true, ..Default::default() },
        EventInfo { id: 30, mode: 390145, available: true, rollout_ok: true, ..Default::default() },
    ];
    s.progression.set_last_played(INTRO_PLAYED, 1);
    call(&r, &mut s, "SetNewGame", Some(r#"{ "NewGame": true }"#));
    assert!(s.game_modes.new_game && !s.game_modes.recap && s.game.save_pending && s.progression.save_requested);
    // `FUN_0072bf20`: the tables emptied.
    assert_eq!(s.progression.last_played(INTRO_PLAYED), -1);
    // First pass: the listed events only.
    assert_eq!((s.game.pending_event, s.game.pending_event_aux), (30, -1));
    assert_eq!(s.players.local_player().unwrap().heat_3dc, 0.0);
    assert_eq!(s.requests, [Request::NewGame { new_game: true, default_rollout: 523000, saved_position: -1 }]);
    // Continue: recap on; nothing listed → second pass takes the first available.
    let mut s = with_local_player();
    s.config.freedrive_mode = 390145;
    s.game_modes.events =
        vec![EventInfo { id: 10, mode: 390145, available: true, rollout_ok: true, ..Default::default() }];
    s.progression.set_last_played(INTRO_PLAYED, 1);
    call(&r, &mut s, "SetNewGame", Some(r#"{ "NewGame": false }"#));
    assert!(s.game_modes.recap && !s.game_modes.new_game);
    assert_eq!(s.progression.last_played(INTRO_PLAYED), 1);
    assert_eq!(s.game.pending_event, 10);
    assert_eq!(s.players.local_player().unwrap().heat_3dc, 2.0);
    // No free-drive event at all: −1.
    let mut s = LogicState::default();
    call(&r, &mut s, "SetFreedriveEventToPlay", None);
    assert_eq!(s.game.pending_event, u32::MAX);
}

#[test]
fn new_game_intro() {
    let r = Registry::original_with_parked();
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
    let r = Registry::original_with_parked();
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
    let r = Registry::original_with_parked();
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
    let r = Registry::original_with_parked();
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
fn mode_timer_starts_the_event_timeline_and_the_deadline() {
    use super::mode_machine::ModeInfo;
    use crate::binding::Value;
    let r = Registry::original_with_parked();
    let mut s = LogicState::default();
    s.game_modes.events =
        vec![EventInfo { id: 7, mode: 730000, timeline: vec![1556501, 985051], ..Default::default() }];
    s.game_modes.modes =
        vec![ModeInfo { id: 730000, data: vec![(0xfe4f_2d8c, Value::Float(900.0))], ..Default::default() }];
    s.game_modes.current_event = 7;
    s.game_modes.current_mode = 730000;
    s.game_modes.display_timeout = true;
    s.engine.ui_ticks = 1000;
    call(&r, &mut s, "StartGameModeTimer", None);
    assert_eq!(
        s.requests,
        [
            Request::PlayEventTimeline { sequence: 1556501 },
            Request::PlayEventTimeline { sequence: 985051 },
            Request::GameModeTimer { running: true }
        ]
    );
    assert_eq!(s.game_modes.deadline, Some(1000 + 2_700_000_000));
    assert!(!s.game_modes.display_timeout);
    s.engine.ui_ticks += 3_000_000;
    assert_eq!(super::game_modes::display_time_left(&s), 2_697_000_000);
    s.requests.clear();
    call(&r, &mut s, "ResetGameMode", None);
    assert_eq!(
        s.requests,
        [
            Request::StopEventTimeline { sequence: 1556501 },
            Request::StopEventTimeline { sequence: 985051 },
            Request::ResetGameMode
        ]
    );
    assert!(s.game_modes.timeline.is_empty());
}

#[test]
fn awards() {
    let r = Registry::original_with_parked();
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
    let r = Registry::original_with_parked();
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
    let r = Registry::original_with_parked();
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

/// The registry the boot path uses: the 187 hooks verified against their handlers (`the_second_twenty` ... `the_ninth_twenty`).
#[test]
fn the_twenty_ported_hooks() {
    let r = Registry::original();
    let ported: Vec<_> = r.hooks().iter().filter(|h| h.ported()).map(|h| h.name).collect();
    assert_eq!(ported.len(), 187, "{ported:?}");
    let mut s = LogicState::default();
    s.players.local = Some(0);
    s.players.slots[0] = Some(Player::default());

    // Flag hooks (0x676b20 / 0x676bd0): GMM+0x2e88.
    assert_eq!(call(&r, &mut s, "CalculateShouldShowRecapSequence", None), EXIT);
    assert!(s.game_modes.recap);
    assert_eq!(call(&r, &mut s, "ClearRecapFlag", None), EXIT);
    assert!(!s.game_modes.recap);

    // IsOnlineEvent (0x676af0): (GMM+4)+0xe8.
    assert_eq!(call(&r, &mut s, "IsOnlineEvent", None), FALSE);
    s.game_modes.online_event = true;
    assert_eq!(call(&r, &mut s, "IsOnlineEvent", None), TRUE);

    // ClearMatchmakingState (0x676ff0 -> FUN_00675470).
    assert_eq!(call(&r, &mut s, "ClearMatchmakingState", None), EXIT);

    // The engine-side hooks only ask the runtime.
    assert_eq!(call(&r, &mut s, "SetSinglePlayerRollout", None), EXIT);
    assert_eq!(call(&r, &mut s, "SetRolloutFromProfile", None), EXIT);
    assert_eq!(call(&r, &mut s, "PopulateSettings", None), EXIT);
    assert_eq!(call(&r, &mut s, "SaveSettings", None), EXIT);
    assert_eq!(
        s.requests,
        [
            Request::SetSinglePlayerRollout,
            Request::SetRolloutFromProfile,
            Request::PopulateSettings,
            Request::SaveSettings
        ]
    );
    s.requests.clear();

    // SaveProfile (0x67a760): the save request, G+0x2922c = 1.
    assert_eq!(call(&r, &mut s, "SaveProfile", None), EXIT);
    assert_eq!(s.requests, [Request::SaveProfile]);
    assert!(s.game.save_pending);
}

/// The second 20 verified hooks (handlers read from the snapshot's disassembly, 2026-11-17), through
/// `Registry::original` (not the parked set).
#[test]
fn the_second_twenty() {
    let r = Registry::original();
    let mut s = with_local_player();
    // Bool members of the data definition into GMM bytes (0x676030 / 0x675ff0 / 0x676160 / 0x676bf0).
    call(&r, &mut s, "SetStoryMode", Some(r#"{ "StoryMode": true }"#));
    call(&r, &mut s, "SetGameModeWon", Some(r#"{ "Won": true }"#));
    call(&r, &mut s, "SetAllowLoadingScreen", Some(r#"{ "lbEnable": true }"#));
    call(&r, &mut s, "SetRetryAtEnd", Some(r#"{ "Retry": true }"#));
    let g = &s.game_modes;
    assert!(g.story_mode && g.won && g.allow_loading_screen && g.retry_at_end);
    call(&r, &mut s, "SetRetryAtEnd", Some(r#"{ "Retry": false }"#));
    assert!(!s.game_modes.retry_at_end);
    // IsChallengeActive: GMM+0x2ce0 != 0; the safehouse sequence: always TRUE on PC (FUN_006e0520 = 0).
    assert_eq!(call(&r, &mut s, "IsChallengeActive", None), FALSE);
    s.game_modes.current_event = 3;
    assert_eq!(call(&r, &mut s, "IsChallengeActive", None), TRUE);
    assert_eq!(call(&r, &mut s, "ShouldShowSafehouseExitSequence", None), TRUE);
    assert_eq!(call(&r, &mut s, "SetIntroLoadComplete", None), EXIT);
    assert!(s.game_modes.intro_load_complete);
    assert_eq!(call(&r, &mut s, "ShutdownGame", None), EXIT);
    assert!(s.game.shutdown);
    assert_eq!(s.requests, [Request::ShutdownGame]);
    s.requests.clear();
    // SetFreedriveEventToPlay: no free-drive event -> -1, aux -1, flag 0.
    call(&r, &mut s, "SetFreedriveEventToPlay", None);
    assert_eq!((s.game.pending_event, s.game.pending_event_aux, s.game.pending_event_flag), (u32::MAX, -1, false));
    // RetryLastEvent / StartEventFromEasyDrive: FUN_006e74f0(id, 0); -1 does nothing.
    s.progression.set_last_played(LAST_EVENT, 42);
    call(&r, &mut s, "RetryLastEvent", None);
    assert_eq!(s.game_modes.requested_event, 42);
    call(&r, &mut s, "StartEventFromEasyDrive", Some(r#"{ "lpcValue": "-1" }"#));
    assert_eq!(s.game_modes.requested_event, 42);
    call(&r, &mut s, "StartEventFromEasyDrive", Some(r#"{ "lpcValue": "7" }"#));
    assert_eq!(s.game_modes.requested_event, 7);
    // The engine-side hooks, each one request, EXIT.
    s.game_modes.current_event = 9;
    s.game_modes.current_mode = 11;
    s.game_modes.award_amount = 5;
    s.game_modes.awards = vec![4];
    s.game.pending_event = 77;
    for name in [
        "SetEventFromAvailableVehicles",
        "ResetCurrentEventStats",
        "SetTimeOfDayFromEvent",
        "PreparePostEventAwards",
        "AwardPostEventAwards",
        "AddAllAIPlayers",
        "RemoveAllRaceCars",
        "RequestRespawnAtEventStart",
        "ClearHudMessageQueues",
    ] {
        assert_eq!(call(&r, &mut s, name, None), EXIT, "{name}");
    }
    assert_eq!(
        s.requests,
        [
            Request::SetEventFromAvailableVehicles,
            Request::ResetCurrentEventStats { event: 9, mode: 11, online: false },
            Request::SetTimeOfDayFromEvent,
            Request::PreparePostEventAwards,
            Request::AwardPostEventAwards { event: 9, amount: 5, award_2ce8: 0, awards: vec![4] },
            Request::AddAllAIPlayers,
            Request::RemoveAllRaceCars,
            Request::Respawn { event: 77, to_swap_spot: false },
            Request::ClearHudMessageQueues,
        ]
    );
    assert_eq!(s.players.local_player().unwrap().respawn_event, 77);
}

/// The third 20 verified hooks (handlers read from the snapshot's disassembly, 2026-11-17), through
/// `Registry::original`.
#[test]
fn the_third_twenty() {
    let r = Registry::original();
    let mut s = with_local_player();
    // Players: cops are AI-driven (kind 2) with the cop flag; AI racers must have a vehicle and be loaded.
    s.players.slots[5] = Some(Player { kind: 2, cop: true, ..Default::default() });
    s.players.slots[6] = Some(Player { kind: 1, cop: true, ..Default::default() });
    s.players.slots[12] = Some(Player { kind: 2, loaded: true, ..Default::default() });
    assert_eq!(call(&r, &mut s, "AreAllAIPlayersLoaded", None), TRUE);
    s.players.slots[13] = Some(Player { kind: 2, loaded: true, vehicle: -1, ..Default::default() });
    assert_eq!(call(&r, &mut s, "AreAllAIPlayersLoaded", None), FALSE);
    assert_eq!(call(&r, &mut s, "RemoveAllCops", None), EXIT);
    assert_eq!(s.requests, [Request::RemovePlayer { slot: 5, reason: 3 }]);
    s.requests.clear();
    // Engine-side hooks: one request each.
    for name in ["RemoveRoadblocks", "ResetProps", "SyncTimeOfDay", "RequestRespawn"] {
        assert_eq!(call(&r, &mut s, name, None), EXIT, "{name}");
    }
    assert_eq!(
        s.requests,
        [
            Request::RemoveRoadblocks,
            Request::ResetProps,
            Request::SyncTimeOfDay,
            Request::Respawn { event: 0, to_swap_spot: false }
        ]
    );
    s.requests.clear();
    call(&r, &mut s, "RequestRespawnToLastSwapSpot", None);
    let p = s.players.local_player().unwrap();
    assert!(p.respawn_to_swap_spot && p.respawn_event == 0);
    // Heat: ResetChaseStats (request + all three), ResetLocalPlayerHeat (all three), Transfer (+0x3dc only).
    s.players.local_player_mut().unwrap().heat_3d8 = 1.0;
    call(&r, &mut s, "TransferVehicleHeatToPlayer", None);
    let p = s.players.local_player().unwrap();
    assert_eq!((p.heat_3d8, p.heat_3dc, p.heat_3e8), (1.0, 0.0, 3.0));
    call(&r, &mut s, "ResetLocalPlayerHeat", None);
    let p = s.players.local_player().unwrap();
    assert_eq!((p.heat_3d8, p.heat_3dc, p.heat_3e8), (0.0, 0.0, 0.0));
    s.requests.clear();
    call(&r, &mut s, "ResetChaseStats", None);
    assert_eq!(s.requests, [Request::ResetChaseStats]);
    // Rollout set / damage reset / nitrous.
    assert_eq!(call(&r, &mut s, "HasLocalPlayerSetRollout", None), TRUE);
    s.requests.clear();
    s.game_modes.has_transitioned_events = true;
    call(&r, &mut s, "ResetLocalPlayerDamage", None);
    assert!(s.requests.is_empty());
    s.game_modes.has_transitioned_events = false;
    call(&r, &mut s, "ResetLocalPlayerDamage", None);
    call(&r, &mut s, "ClearNitrous", None);
    assert_eq!(s.requests, [Request::ResetVehicleDamage { vehicle: 3 }, Request::ClearNitrous { slot: 0 }]);
    s.players.local_player_mut().unwrap().vehicle = -1;
    assert_eq!(call(&r, &mut s, "HasLocalPlayerSetRollout", None), FALSE);
    // Escape sequence: a free-drive event or the configured mode; the recommendation retry only in free drive.
    s.config.escape_mode = 390185;
    assert_eq!(call(&r, &mut s, "ShouldShowEscapeSequence", None), FALSE);
    s.game_modes.current_mode = 390185;
    assert_eq!(call(&r, &mut s, "ShouldShowEscapeSequence", None), TRUE);
    s.game_modes.current_mode = 0;
    s.game_modes.freedrive_event = true;
    assert_eq!(call(&r, &mut s, "ShouldShowEscapeSequence", None), TRUE);
    s.requests.clear();
    s.progression.set_last_played(LAST_EVENT, 42);
    call(&r, &mut s, "FixUpRecommendationRetry", None);
    assert_eq!(s.requests, [Request::RecommendationRetry { event: 42 }]);
    s.game_modes.freedrive_event = false;
    call(&r, &mut s, "FixUpRecommendationRetry", None);
    assert_eq!(s.requests.len(), 1);
    // GMM flags and the post-event awards.
    s.game_modes.awards = vec![1];
    s.game_modes.award_amount = 9;
    call(&r, &mut s, "ClearPostEventAwards", None);
    assert!(s.game_modes.awards.is_empty() && s.game_modes.award_amount == 0);
    call(&r, &mut s, "SetSeenEventsForVehicleToTrue", None);
    call(&r, &mut s, "ForceOpenEasyDrive", None);
    assert!(s.game_modes.seen_events_for_vehicle && s.game_modes.force_open_easydrive);
    // The intro's car control: G+0x2f0c8 = 3.0, then group 1 bit 6 cleared and 0.
    s.game.input_blocks.masks[1] = 1 << 6 | 1;
    s.game.input_blocks.counts[1] = 2;
    call(&r, &mut s, "HACKSuspendCarControlForIntro", None);
    assert_eq!(s.game.input_blocks.suspend_time, 3.0);
    call(&r, &mut s, "HACKResumeCarControlAfterIntro", None);
    assert_eq!(
        (s.game.input_blocks.masks[1], s.game.input_blocks.counts[1], s.game.input_blocks.suspend_time),
        (1, 1, 0.0)
    );
}

/// The fourth twenty, each checked against its handler (research-log 2026-11-17), through the verified registry.
#[test]
fn the_fourth_twenty() {
    let r = Registry::original();
    let mut s = with_local_player();
    // Handlers that only return EXIT on PC.
    for name in ["SetSafehouseFromProfile", "ApplySelectedVehicleRollout"] {
        assert_eq!(call(&r, &mut s, name, None), EXIT);
    }
    assert!(s.requests.is_empty());
    // Engine-only bodies.
    for (name, req) in [
        ("ClearHudInstantQueue", Request::ClearHudInstantQueue),
        ("StopSpeech", Request::StopSpeech),
        ("SkipCurrentTrack", Request::SkipCurrentTrack),
        ("ClearPendingResets", Request::ClearPendingResets),
        ("SetCollectablesNotInWorld", Request::SetCollectablesNotInWorld),
        ("TransferCurrentHeatToVehicle", Request::TransferCurrentHeatToVehicle),
    ] {
        assert_eq!(call(&r, &mut s, name, None), EXIT);
        assert_eq!(s.requests.pop(), Some(req), "{name}");
    }
    // StopAllSequences: the HUD queues (FUN_0067f8c0), then both sequence players.
    call(&r, &mut s, "StopAllSequences", None);
    assert_eq!(s.requests, [Request::ClearHudMessageQueues, Request::StopAllSequences]);
    s.requests.clear();
    // Damage: the message needs the local car loaded; flag 1 deactivates, 0 reactivates.
    call(&r, &mut s, "DeactivateDamage", None);
    assert!(s.requests.is_empty());
    s.players.local_player_mut().unwrap().loaded = true;
    call(&r, &mut s, "DeactivateDamage", None);
    call(&r, &mut s, "ReactivateDamage", None);
    call(&r, &mut s, "CleanAndRepairVehicle", None);
    assert_eq!(
        s.requests,
        [
            Request::VehicleDamage { vehicle: 3, deactivated: true },
            Request::VehicleDamage { vehicle: 3, deactivated: false },
            Request::CleanAndRepairVehicle { vehicle: 3 },
        ]
    );
    s.requests.clear();
    // Speed traps: an online event, or the event kind 0x34 / 0x47.
    assert_eq!(call(&r, &mut s, "IsSpeedTrapEvent", None), FALSE);
    s.game_modes.current_event_kind = 0x47;
    assert_eq!(call(&r, &mut s, "IsSpeedTrapEvent", None), TRUE);
    s.game_modes.current_event_kind = 0x10;
    s.game_modes.online_event = true;
    assert_eq!(call(&r, &mut s, "IsSpeedTrapEvent", None), TRUE);
    // Arguments.
    call(&r, &mut s, "SetValidProgressionData", Some(r#"{ "lbValid": true }"#));
    call(&r, &mut s, "SetRollout", Some(r#"{ "liRolloutId": 523000 }"#));
    assert_eq!(s.requests, [Request::SetValidProgressionData { valid: true }, Request::SetRollout { rollout: 523000 }]);
    // Flags.
    s.game_modes.force_open_easydrive = true;
    s.game_modes.credits_transition = true;
    s.game_modes.jump_to_store = true;
    s.game_modes.fmn_vita_exclusive = true;
    s.game.loading_screen_requests = 0xff;
    s.game.loading_screen_2d21c = 5;
    for name in [
        "ClearForceOpenEasyDriveFlag",
        "ClearCreditsTransition",
        "ClearJumpToStoreFlag",
        "SetFMNMenuShowingCommonEvents",
        "ToggleLoadingScreenOff",
    ] {
        assert_eq!(call(&r, &mut s, name, None), EXIT);
    }
    let g = &s.game_modes;
    assert!(!g.force_open_easydrive && !g.credits_transition && !g.jump_to_store && !g.fmn_vita_exclusive);
    assert_eq!((s.game.loading_screen_requests, s.game.loading_screen_2d21c), (0xdf, 0));
}

/// The fifth twenty, each checked against its handler (research-log 2026-11-17), through the verified registry.
#[test]
fn the_fifth_twenty() {
    let r = Registry::original();
    let mut s = with_local_player();
    // Easydrive sounds: UniqueId at the given field, started on the sequence player.
    for (name, field) in [
        ("OnOpenEasyDrive", 0x50),
        ("OnStartEvent", 0x60),
        ("OnSelectEasyDriveOption", 0x58),
        ("OnOpenEasyDriveMods", 0x54),
        ("OnCloseEasyDriveMods", 0x4c),
    ] {
        assert_eq!(call(&r, &mut s, name, None), EXIT);
        assert_eq!(s.requests.pop(), Some(Request::PlayUiSound { field }), "{name}");
    }
    // Flags (GMM = G+0x2f3fc): +0x2caa, +0x2f28, +0x2e9b, +0x2e9a, +0x2e9d.
    assert_eq!(call(&r, &mut s, "ShouldPlayEndOfIntroSequenceInEasyDrive", None), FALSE);
    assert_eq!(call(&r, &mut s, "PlayEndOfIntroSequenceInEasyDrive", None), EXIT);
    assert_eq!(call(&r, &mut s, "ShouldPlayEndOfIntroSequenceInEasyDrive", None), TRUE);
    assert_eq!(call(&r, &mut s, "ClearPlayEndOfIntroSequenceInEasyDrive", None), EXIT);
    assert_eq!(call(&r, &mut s, "ShouldPlayEndOfIntroSequenceInEasyDrive", None), FALSE);
    assert_eq!(call(&r, &mut s, "HasIntroEasyDriveBeenActive", None), FALSE);
    assert_eq!(call(&r, &mut s, "SetHasIntroEasyDriveBeenActive", None), EXIT);
    assert_eq!(call(&r, &mut s, "HasIntroEasyDriveBeenActive", None), TRUE);
    for name in ["SwitchToGameplayLoadWorld", "LeaveGame", "ExitForMultiplayerMatchMaking"] {
        assert_eq!(call(&r, &mut s, name, None), EXIT);
    }
    let g = &s.game_modes;
    assert!(g.switch_to_gameplay_load_world && g.leave_game && g.exit_for_mp_matchmaking);
    // Handlers that only return EXIT on PC (0x117efa0 holds the same key as EXIT).
    for name in [
        "ClearE3MultiplayerProgression",
        "ShowPartyInvitePopup",
        "OnPhotoSharedToWall",
        "ShowX360GamerCard",
        "ShowX360GamerCardEasyDrive",
        "ShowX360GamerCardByPlayerIndex",
    ] {
        assert_eq!(call(&r, &mut s, name, None), EXIT, "{name}");
    }
    assert!(s.requests.is_empty());
    // ForceKeyboardShutdown: engine only.
    assert_eq!(call(&r, &mut s, "ForceKeyboardShutdown", None), EXIT);
    assert_eq!(s.requests, [Request::ForceKeyboardShutdown]);
}

/// The sixth twenty, each checked against its handler (research-log 2026-11-17), through the verified registry.
#[test]
fn the_sixth_twenty() {
    use super::hooks_select::{LI_OPTION_ID, LI_SELECTED_WORLD_ID};
    use mw_formats::genesys::field_hash;
    assert_eq!(field_hash("liOptionId"), LI_OPTION_ID);
    assert_eq!(field_hash("liSelectedWorldId"), LI_SELECTED_WORLD_ID);
    let r = Registry::original();
    let mut s = with_local_player();

    // Engine-only: the handler's call, with the arguments it reads.
    assert_eq!(call(&r, &mut s, "CancelMilestoneTracking", None), EXIT);
    assert_eq!(call(&r, &mut s, "SetSelectedWorld", Some(r#"{"liSelectedWorldId": 2}"#)), EXIT);
    assert_eq!(call(&r, &mut s, "SetAvailableVehicleListEvent", Some(r#"{"liEventId": 77}"#)), EXIT);
    assert_eq!(call(&r, &mut s, "SelectPaintPackFromEasyDrive", Some(r#"{"lpcValue": "12"}"#)), EXIT);
    assert_eq!(call(&r, &mut s, "UpdatePendingRollout_EasyDrive", Some(r#"{"lpcValue": "345"}"#)), EXIT);
    assert_eq!(call(&r, &mut s, "UpdatePendingRollout_EasyDrive_Stock", Some(r#"{"lpcValue": "9"}"#)), EXIT);
    assert_eq!(call(&r, &mut s, "UpdateProgressAwardsStrings", None), EXIT);
    assert_eq!(call(&r, &mut s, "ShowTargetScoreReminderMessage", None), EXIT);
    assert_eq!(
        s.requests,
        [
            Request::CancelMilestoneTracking,
            Request::SetSelectedWorld { world: 2 },
            Request::SetAvailableVehicleListEvent { event: 77 },
            Request::SelectPaintPack { pack: 12 },
            Request::UpdatePendingRollout { vehicle: 345, stock: false },
            Request::PlayUiSound { field: 0x64 },
            Request::UpdatePendingRollout { vehicle: 9, stock: true },
            Request::UpdateProgressAwardsStrings,
            Request::ShowTargetScoreReminder { remaining: 0 },
        ]
    );
    s.requests.clear();

    // Easy-drive options: the "ez_options" mirror.
    let opt = |id: i32| format!(r#"{{"liOptionId": {id}}}"#);
    assert_eq!(call(&r, &mut s, "HasEasyDriveOptionBeenSeen", Some(&opt(5))), FALSE);
    assert_eq!(call(&r, &mut s, "SetEasyDriveOptionSeen", Some(r#"{"liOptionId": 5, "lbSeen": true}"#)), EXIT);
    assert_eq!(call(&r, &mut s, "HasEasyDriveOptionBeenSeen", Some(&opt(5))), TRUE);
    assert_eq!(call(&r, &mut s, "SetEasyDriveOptionSeenToTrue", Some(r#"{"lpcValue": "6"}"#)), EXIT);
    assert_eq!(call(&r, &mut s, "HasEasyDriveOptionBeenSeen", Some(&opt(6))), TRUE);
    s.game_modes.online_event = true;
    assert_eq!(call(&r, &mut s, "SetEasyDriveOptionSeenToTrue", Some(r#"{"lpcValue": "7"}"#)), EXIT);
    assert_eq!(call(&r, &mut s, "HasEasyDriveOptionBeenSeen", Some(&opt(7))), FALSE);
    s.game_modes.online_event = false;
    assert_eq!(
        s.requests,
        [
            Request::SetEasyDriveOptionSeen { option: 5, seen: true },
            Request::SetEasyDriveOptionSeenToTrue { option: 6, online: false },
            Request::SetEasyDriveOptionSeenToTrue { option: 7, online: true },
        ]
    );
    s.requests.clear();

    // UI manager / rollout manager fields.
    assert_eq!(call(&r, &mut s, "CalculateDistanceToEventByIdString", Some(r#"{"lpcValue": "42"}"#)), EXIT);
    assert_eq!((s.select.distance_event, s.select.distance_requested), (42, true));
    assert_eq!(call(&r, &mut s, "SetHighlightedRolloutByIdString", Some(r#"{"lpcValue": "8"}"#)), EXIT);
    assert_eq!(s.select.highlighted_rollout, 8);
    assert_eq!(call(&r, &mut s, "SetHighlightedVehicleByIdString", Some(r#"{"lpcValue": "11"}"#)), EXIT);
    assert_eq!(call(&r, &mut s, "SetHighlightedVehicleByIdString", Some(r#"{"lpcValue": "11"}"#)), EXIT);
    assert_eq!(s.requests, [Request::LoadHighlightedVehicle { vehicle: 11 }]);
    s.requests.clear();
    // A vehicle id of 0 or less selects nothing.
    assert_eq!(call(&r, &mut s, "SetRolloutFromVehicleIdString", Some(r#"{"lpcValue": "0"}"#)), EXIT);
    assert!(s.requests.is_empty());
    assert_eq!(call(&r, &mut s, "SetRolloutFromVehicleIdString", Some(r#"{"lpcValue": "31"}"#)), EXIT);
    assert_eq!(s.requests, [Request::SetRolloutFromVehicle { vehicle: 31 }]);
    s.requests.clear();

    // IsTargetScoreHit: score >= target.
    s.game_modes.event_target_score = 1000;
    s.players.local_player_mut().unwrap().score = 999;
    assert_eq!(call(&r, &mut s, "IsTargetScoreHit", None), FALSE);
    s.players.local_player_mut().unwrap().score = 1000;
    assert_eq!(call(&r, &mut s, "IsTargetScoreHit", None), TRUE);
    assert_eq!(call(&r, &mut s, "ShowTargetScoreReminderMessage", None), EXIT);
    assert_eq!(s.requests, [Request::ShowTargetScoreReminder { remaining: 0 }]);
    s.requests.clear();

    // SetFinalEventData: the local player's +0x5b8 / +0x5c8 / +0x5cc / +0x5d0.
    s.game_modes.byte_2ca3 = 2;
    s.game_modes.word_2d28 = 0x1234;
    s.game_modes.word_2d2c = 0x5678;
    s.players.local_player_mut().unwrap().final_5d0 = true;
    assert_eq!(call(&r, &mut s, "SetFinalEventData", None), EXIT);
    let p = s.players.local_player().unwrap();
    assert_eq!((p.final_5b8, p.final_5c8, p.final_5cc, p.final_5d0), (3, 0x1234, 0x5678, false));

    // Events from a binding: a binding that does not resolve is -1, nothing happens.
    assert_eq!(call(&r, &mut s, "StartEventFromBinding", Some(r#"{"lpcValue": "Nope.Nothing"}"#)), EXIT);
    assert_eq!(call(&r, &mut s, "SetWaypointToEventFromBinding", Some(r#"{"lpcValue": "Nope.Nothing"}"#)), EXIT);
    assert!(s.requests.is_empty() && !s.game_modes.event_requested);

    // The Vita flag (GMM+0x2e85).
    assert_eq!(call(&r, &mut s, "SetFMNMenuShowingVITAExclusiveEvents", None), EXIT);
    assert!(s.game_modes.fmn_vita_exclusive);
}

/// The seventh twenty, each checked against its handler (research-log 2026-11-17), through the verified registry.
#[test]
fn the_seventh_twenty() {
    use super::hooks_misc::{LPC_VIEW_MODEL_NAME, PATH};
    use super::lua_settings::{SettingsCategory, ValueSetting};
    use mw_formats::genesys::field_hash;
    assert_eq!(field_hash("Path"), PATH);
    assert_eq!(field_hash("lpcViewModelName"), LPC_VIEW_MODEL_NAME);
    let r = Registry::original();
    let mut s = with_local_player();

    // CalculateBlacklistDescription: lpcValue - 1, the flags cleared.
    s.hook_misc.blacklist_flags = [true; 3];
    let v = |n: i32| format!(r#"{{"lpcValue": "{n}"}}"#);
    assert_eq!(call(&r, &mut s, "CalculateBlacklistDescription", Some(&v(15))), EXIT);
    assert_eq!(s.hook_misc.blacklist_flags, [false; 3]);
    assert_eq!(s.requests, [Request::CalculateBlacklistDescription { rank: 14 }]);
    s.requests.clear();

    // ResolveBoolBinding: FALSE when the binding does not resolve or the member is missing.
    assert_eq!(call(&r, &mut s, "ResolveBoolBinding", Some(r#"{"Path": "Nope.Nothing"}"#)), FALSE);
    assert_eq!(call(&r, &mut s, "ResolveBoolBinding", None), FALSE);

    // The saved vehicle and rollout: an id above 0 only.
    assert_eq!(call(&r, &mut s, "SetVehicleAndRolloutFromSaveGameByVehicleID", Some(r#"{"liId": 0}"#)), EXIT);
    assert_eq!(call(&r, &mut s, "SetVehicleAndRolloutFromSaveGameByVehicleID", Some(r#"{"liId": 12}"#)), EXIT);
    assert_eq!(call(&r, &mut s, "SetVehicleAndRolloutFromSaveGameByVehicleIDString", Some(&v(-3))), EXIT);
    assert_eq!(call(&r, &mut s, "SetVehicleAndRolloutFromSaveGameByVehicleIDString", Some(&v(77))), EXIT);
    assert_eq!(call(&r, &mut s, "UpdateAwardProgressBindings", None), EXIT);
    assert_eq!(
        s.requests,
        [
            Request::SetVehicleAndRolloutFromSaveGame { vehicle: 12 },
            Request::SetVehicleAndRolloutFromSaveGame { vehicle: 77 },
            Request::UpdateProgressAwardsStrings,
        ]
    );
    s.requests.clear();

    // Settings steppers (category and index both read as 0 without their members): TRUE only when the value moved.
    assert_eq!(call(&r, &mut s, "IncrementSetting", None), FALSE);
    s.settings.insert(SettingsCategory {
        id: 0,
        settings: vec![Box::new(ValueSetting { value: 4, saved: 4, min: 3, max: 5, ..Default::default() })],
    });
    let value = |s: &mut LogicState| s.settings.setting_mut(0, 0).unwrap().value();
    assert_eq!(call(&r, &mut s, "IncrementSetting", None), TRUE);
    assert_eq!(value(&mut s), 5);
    assert_eq!(call(&r, &mut s, "IncrementSetting", None), FALSE);
    assert_eq!(call(&r, &mut s, "DecrementSetting", None), TRUE);
    assert_eq!(call(&r, &mut s, "DecrementSetting", None), TRUE);
    assert_eq!(value(&mut s), 3);
    assert_eq!(call(&r, &mut s, "DecrementSetting", None), FALSE);
    assert_eq!(value(&mut s), 3);

    // ExitWidgetGroup: without its text member the empty code has the key 0: nothing is exited.
    assert_eq!(call(&r, &mut s, "ExitWidgetGroup", None), EXIT);
    assert_eq!(s.screen.widget_group_exit, 0);

    // Deferred view models.
    let n = |name: &str| format!(r#"{{"lpcViewModelName": "{name}"}}"#);
    assert_eq!(call(&r, &mut s, "UpdateDeferredViewModelSilent", Some(&n("Garage"))), EXIT);
    assert_eq!(s.requests, [Request::UpdateDeferredViewModelSilent { name: "Garage".into() }]);
    s.requests.clear();
    assert_eq!(call(&r, &mut s, "HasScreenRegisterdDeferredViewModel", Some(&n("Garage"))), FALSE);
    s.hook_misc.registered_deferred_view_models.insert("Garage".into());
    assert_eq!(call(&r, &mut s, "HasScreenRegisterdDeferredViewModel", Some(&n("Garage"))), TRUE);
    assert_eq!(call(&r, &mut s, "HasScreenRegisterdDeferredViewModel", Some(&n("garage"))), FALSE);
    s.hook_misc.screen_changing = true;
    assert_eq!(call(&r, &mut s, "HasScreenRegisterdDeferredViewModel", Some(&n("Garage"))), FALSE);

    // The store jump: the flag, the id from the binding (0 when it does not resolve), cleared again.
    s.hook_misc.entitlement_jump_id = 9;
    assert_eq!(call(&r, &mut s, "SetJumpToStoreFlag", None), EXIT);
    assert!(s.game_modes.jump_to_store);
    assert_eq!(s.hook_misc.entitlement_jump_id, 0);
    s.hook_misc.entitlement_jump_id = 9;
    assert_eq!(call(&r, &mut s, "ClearEntitlementJumpID", None), EXIT);
    assert_eq!(s.hook_misc.entitlement_jump_id, 0);

    // The online ones.
    for (name, vis) in
        [("SetGameVisibilityInviteOnly", 2), ("SetGameVisibilityJoinable", 3), ("SetGameVisibilityPublic", 4)]
    {
        assert_eq!(call(&r, &mut s, name, None), EXIT);
        assert_eq!(s.hook_misc.game_visibility, vis, "{name}");
    }
    for name in [
        "ClearNetworkedTimer",
        "ClearOnlineEasyDrive",
        "RefreshOnlineEasyDrive",
        "SetOnlineRaceCarFrozen",
        "SetOnlineRaceCarUnfrozen",
    ] {
        assert_eq!(call(&r, &mut s, name, None), EXIT);
    }
    assert_eq!(
        s.requests,
        [
            Request::ClearNetworkedTimer,
            Request::ClearOnlineEasyDrive,
            Request::RefreshOnlineEasyDrive,
            Request::SetRaceCarFrozen { frozen: true },
            Request::SetRaceCarFrozen { frozen: false },
        ]
    );
}

/// The eighth twenty, each checked against its handler (research-log 2026-11-17), through the verified registry.
#[test]
fn the_eighth_twenty() {
    let r = Registry::original();
    let mut s = with_local_player();
    let v = |n: i32| format!(r#"{{"lpcValue": "{n}"}}"#);

    // The recommendations: nothing offline.
    assert_eq!(call(&r, &mut s, "SetTargetRecommendedEvent", Some(&v(2))), FALSE);
    assert_eq!(call(&r, &mut s, "SetCarListForRecommendedEvent", Some(&v(1))), FALSE);
    assert_eq!(call(&r, &mut s, "SelectRecommendationDomByIndexString", Some(&v(1))), EXIT);
    assert_eq!(call(&r, &mut s, "SetCurrentRecommendedEvent", None), EXIT);
    assert!(s.requests.is_empty());
    s.hook_online.has_recommendations = true;
    s.hook_online.event_ids = vec![100, 200, 300];
    assert_eq!(call(&r, &mut s, "SetTargetRecommendedEvent", Some(&v(2))), TRUE);
    assert_eq!((s.hook_online.target_index, s.hook_online.target_param), (2, 0));
    assert_eq!(call(&r, &mut s, "SetCurrentRecommendedEvent", None), EXIT);
    assert_eq!(s.hook_online.current_index, 2);
    assert_eq!(call(&r, &mut s, "SetCarListForRecommendedEvent", Some(&v(0))), FALSE);
    assert_eq!(call(&r, &mut s, "SetCarListForRecommendedEvent", Some(&v(1))), TRUE);
    assert_eq!(s.hook_online.car_list_event, 200);
    assert_eq!(call(&r, &mut s, "SelectRecommendationDomByIndexString", Some(&v(-1))), EXIT);
    assert_eq!(call(&r, &mut s, "SelectRecommendationDomByIndexString", Some(&v(1))), EXIT);
    assert_eq!(call(&r, &mut s, "SelectRecommendationDomByEventIdString", Some(&v(300))), EXIT);
    assert_eq!(call(&r, &mut s, "SelectRecommendationDomByEventIdString", Some(&v(999))), EXIT);
    assert_eq!(call(&r, &mut s, "RefreshRecommendations", None), EXIT);
    assert_eq!(
        s.requests,
        [
            Request::SetRecommendedEvent { index: 2, param: 0 },
            Request::SetRecommendedEvent { index: 2, param: 0 },
            Request::SetCarListForEvent { event_id: 200 },
            Request::SelectRecommendationDom { index: 1, param: 0 },
            Request::SelectRecommendationDom { index: 3, param: 0 },
            Request::ReleaseRecommendationDom,
            Request::RefreshRecommendations,
        ]
    );
    s.requests.clear();

    // GenerateRecomendationPostEventString: the flag and done < total.
    s.hook_online.post_event_wanted = true;
    s.hook_online.events_done = 1;
    s.hook_online.events_total = 3;
    assert_eq!(call(&r, &mut s, "GenerateRecomendationPostEventString", None), EXIT);
    assert!(s.hook_online.post_event_string);
    s.hook_online.events_done = 3;
    call(&r, &mut s, "GenerateRecomendationPostEventString", None);
    assert!(!s.hook_online.post_event_string);

    // The thank-you queue, the autolog alert, the DLC indicator (its member's name is not known: reads 0).
    s.entitlements.thank_you_count = 2;
    assert_eq!(call(&r, &mut s, "PopThankYou", None), EXIT);
    assert_eq!(s.entitlements.thank_you_count, 1);
    call(&r, &mut s, "PopThankYou", None);
    call(&r, &mut s, "PopThankYou", None);
    assert_eq!(s.entitlements.thank_you_count, 0);
    s.hook_online.autolog_alert_active = true;
    assert_eq!(call(&r, &mut s, "StopAutologAlert", None), EXIT);
    assert!(!s.hook_online.autolog_alert_active);
    s.hook_online.autolog_alert_handle = 5;
    call(&r, &mut s, "StopAutologAlert", None);
    s.entitlements.dlc_message_indicator = 4;
    assert_eq!(call(&r, &mut s, "SetDlcMessageIndicator", None), EXIT);
    assert_eq!(s.entitlements.dlc_message_indicator, 0);

    // Sign-in: the Lua natives' code; the silent one leaves the flag clear.
    assert_eq!(call(&r, &mut s, "SignInStarted", None), EXIT);
    assert!(s.menus.sign_in.flag && s.menus.sign_in.active);
    assert_eq!(call(&r, &mut s, "SignInFinished", None), EXIT);
    assert!(!s.menus.sign_in.flag && !s.menus.sign_in.active);
    assert_eq!(call(&r, &mut s, "SilentSignInStarted", None), EXIT);
    assert!(!s.menus.sign_in.flag && s.menus.sign_in.active);
    assert_eq!(s.menus.sign_in.state_1360, -1);

    // The platform ones.
    assert_eq!(call(&r, &mut s, "IsOriginIgoAvailable", None), FALSE);
    s.hook_online.origin_igo_available = true;
    assert_eq!(call(&r, &mut s, "IsOriginIgoAvailable", None), TRUE);
    assert_eq!(call(&r, &mut s, "InvalidateKinectSelectedClass", None), EXIT);
    assert_eq!(s.hook_online.kinect_selected_class, -1);
    assert_eq!(call(&r, &mut s, "CacheMwlAndLock", None), EXIT);
    assert!(s.hook_online.mwl_cache_locked);

    // The engine-only ones are requests.
    for name in
        ["OpenStoreOverlay", "RemoveOnlineSeenUnlocksMenu", "RefreshTopOfSpeedwallViewModel", "ResetPlaylistScoreState"]
    {
        assert_eq!(call(&r, &mut s, name, None), EXIT);
    }
    assert_eq!(
        s.requests,
        [
            Request::PopThankYou,
            Request::PopThankYou,
            Request::StopAutologAlert,
            Request::CacheMwl,
            Request::OpenStoreOverlay,
            Request::ClearOnlineEasyDrive,
            Request::RefreshTopOfSpeedwall,
            Request::ResetPlaylistScoreState,
        ]
    );
}

/// The ninth twenty, each checked against its handler (research-log 2026-11-17), through the verified registry.
#[test]
fn the_ninth_twenty() {
    use crate::natives::entitlements::GamePack;
    use crate::natives::hooks_playlist::Friend;
    let r = Registry::original();
    let mut s = with_local_player();
    let v = |t: &str| format!(r#"{{"lpcValue": "{t}"}}"#);

    // Requests only.
    for name in ["LoadLastOnlineRollout", "SetupPlaylistsVote", "MatchmakePlaygroup"] {
        assert_eq!(call(&r, &mut s, name, None), EXIT);
    }
    assert_eq!(s.requests, [Request::LoadLastOnlineRollout, Request::SetupPlaylistsVote, Request::MatchmakePlaygroup]);
    s.requests.clear();

    // UploadLocalPlayerXP: uploaded above 0, zeroed either way.
    assert_eq!(call(&r, &mut s, "UploadLocalPlayerXP", None), EXIT);
    assert!(s.requests.is_empty());
    s.players.local_player_mut().unwrap().online_xp = 250;
    call(&r, &mut s, "UploadLocalPlayerXP", None);
    assert_eq!(s.requests, [Request::UploadLocalPlayerXp { xp: 250 }]);
    assert_eq!(s.players.local_player().unwrap().online_xp, 0);
    s.requests.clear();

    // ProcessOnlinePostEventDrivingTests: an online event and the pending byte.
    call(&r, &mut s, "ProcessOnlinePostEventDrivingTests", None);
    s.game_modes.online_event = true;
    call(&r, &mut s, "ProcessOnlinePostEventDrivingTests", None);
    assert!(s.requests.is_empty());
    s.hook_playlist.driving_tests_pending = true;
    call(&r, &mut s, "ProcessOnlinePostEventDrivingTests", None);
    assert_eq!(s.requests, [Request::ProcessOnlineDrivingTests]);
    s.requests.clear();

    // The playlists: the criterion playlist only without a current one; cancels only with one.
    call(&r, &mut s, "PlayCriterionPlaylist", None);
    call(&r, &mut s, "HostCancelPlaylist", None);
    assert_eq!(s.requests, [Request::PlayCriterionPlaylist]);
    s.hook_playlist.playlist_active = true;
    call(&r, &mut s, "PlayCriterionPlaylist", None);
    call(&r, &mut s, "HostCancelPlaylist", None);
    call(&r, &mut s, "SoloPlayerCancelPlaylist", None);
    call(&r, &mut s, "PlayCustomPlaylist", Some(r#"{"liPlaylistIndex": 3}"#));
    call(&r, &mut s, "EZPlayCustomPlaylist", Some(&v("2")));
    assert_eq!(
        s.requests[1..],
        [
            Request::CancelPlaylist { solo: false },
            Request::CancelPlaylist { solo: true },
            Request::PlayCustomPlaylist { index: 3 },
            Request::PlayCustomPlaylist { index: 1 },
        ]
    );
    s.requests.clear();

    // The join wishes.
    s.hook_playlist.desired_game = 5;
    s.hook_playlist.desired_game_set = true;
    assert_eq!(call(&r, &mut s, "QuickJoinGameDesired", None), EXIT);
    assert!(s.hook_playlist.quick_join_desired && !s.hook_playlist.create_game_desired);
    assert_eq!((s.hook_playlist.desired_game, s.hook_playlist.desired_game_set), (0, false));
    assert_eq!(call(&r, &mut s, "CreateGameDesired", None), EXIT);
    assert!(s.hook_playlist.create_game_desired);
    s.hook_playlist.playlist_source_2a9e0 = 9;
    s.hook_playlist.mid_game_join_flag = true;
    call(&r, &mut s, "OnMidGameJoin", None);
    assert!(!s.hook_playlist.mid_game_join_flag);
    assert_eq!(s.hook_playlist.playlist_mgr_4, 9);

    // Friends: none offline; the airport pack is not owned offline.
    s.entitlements.game_packs = vec![GamePack { id: 2196330, entitlement: 5, release: 2 }];
    s.entitlements.airport_pack = 2196330;
    assert_eq!(call(&r, &mut s, "CanJoinAirportGameByFriendIndexBinding", Some(&v("1"))), FALSE);
    call(&r, &mut s, "SetJoinFriendIdByIndexBinding", Some(&v("1")));
    assert!(!s.hook_playlist.join_friend_set);
    s.hook_playlist.friends =
        vec![Friend { id: 77, in_airport_game: true }, Friend { id: 1 << 40, in_airport_game: false }];
    assert_eq!(call(&r, &mut s, "CanJoinAirportGameByFriendIndexBinding", Some(&v("1"))), FALSE);
    assert_eq!(call(&r, &mut s, "CanJoinAirportGameByFriendIndexBinding", Some(&v("2"))), TRUE);
    assert_eq!(call(&r, &mut s, "CanJoinAirportGameByFriendIndexBinding", Some(&v("3"))), FALSE);
    assert_eq!(call(&r, &mut s, "CanJoinAirportGameByFriendIndexBinding", Some(&v("0"))), FALSE);
    call(&r, &mut s, "SetJoinFriendIdByIndexBinding", Some(&v("2")));
    assert!(s.hook_playlist.join_friend_set);
    assert_eq!((s.hook_playlist.join_friend_id, s.hook_playlist.join_friend_id_copy), (1 << 40, 1 << 40));
    call(&r, &mut s, "SetFriendInviteIdEasyDrive", Some(&v("12345678901234")));
    assert_eq!(s.hook_playlist.friend_invite_id, 12_345_678_901_234);
    call(&r, &mut s, "SetFriendInviteIdEasyDrive", Some(&v("")));
    assert_eq!(s.hook_playlist.friend_invite_id, 0);
    s.entitlements.add(5);
    assert_eq!(call(&r, &mut s, "CanJoinAirportGameByFriendIndexBinding", Some(&v("1"))), TRUE);

    // Sweet revenge: stored for the local player, then the rollout applied.
    s.requests.clear();
    assert_eq!(call(&r, &mut s, "SetRolloutSweetRevengeFromEasyDrive", Some(&v("3"))), EXIT);
    assert_eq!(s.hook_playlist.sweet_revenge[0], 3);
    assert_eq!(s.requests, [Request::ApplyLocalRollout]);

    // The carousel and the MWL lock.
    call(&r, &mut s, "ActivateFriendCarouselViewModel", None);
    assert!(s.hook_playlist.friend_carousel_active);
    call(&r, &mut s, "DeactivateFriendCarouselViewModel", None);
    assert!(!s.hook_playlist.friend_carousel_active);
    call(&r, &mut s, "CacheMwlAndLock", None);
    assert!(s.hook_online.mwl_cache_locked);
    call(&r, &mut s, "UnlockMwlCache", None);
    assert!(!s.hook_online.mwl_cache_locked);
}

/// The last seven hooks (research-log 2026-11-17 "the last seven hooks").
#[test]
fn the_last_seven() {
    let r = Registry::original();
    let mut s = with_local_player();
    let v = |t: &str| format!(r#"{{"lpcValue": "{t}"}}"#);
    assert_eq!(r.hooks().iter().filter(|h| h.ported()).count(), 187);

    s.hook_playlist.show_floor_flag = true;
    for name in [
        "HACKPlayFirstCustomPlaylist",
        "RestartGamesComShowFloorFlow",
        "ProcessPlaylistEndMilestones",
        "GetMostWantedListFromServer",
        "OnWallPostOrComment",
    ] {
        assert_eq!(call(&r, &mut s, name, None), EXIT, "{name}");
    }
    assert!(!s.hook_playlist.show_floor_flag);
    assert_eq!(
        s.requests,
        [
            Request::PlayCustomPlaylist { index: 0 },
            Request::RestartShowFloorFlow,
            Request::ProcessPlaylistEndMilestones,
            Request::GetMostWantedList,
        ]
    );
    s.requests.clear();

    // The player index is 1-based in the string (`FUN_007ecdc0`: value - 1); nothing for an unusable one.
    assert_eq!(call(&r, &mut s, "MutePlayer", Some(&v("3"))), EXIT);
    assert_eq!(call(&r, &mut s, "UnmutePlayer", Some(&v("1"))), EXIT);
    assert_eq!(call(&r, &mut s, "MutePlayer", Some(&v("0"))), EXIT);
    assert_eq!(call(&r, &mut s, "UnmutePlayer", None), EXIT);
    assert_eq!(s.requests, [Request::MutePlayer { index: 2 }, Request::UnmutePlayer { index: 0 }]);
    assert!(s.unported.is_empty());
}
