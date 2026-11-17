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
    call(&r, &mut s, "SetNewGame", Some(r#"{ "NewGame": true }"#));
    assert!(s.game_modes.new_game && !s.game_modes.recap && s.game.save_pending && s.progression.save_requested);
    // First pass: the listed events only.
    assert_eq!((s.game.pending_event, s.game.pending_event_aux), (30, -1));
    assert_eq!(s.players.local_player().unwrap().heat_3dc, 0.0);
    assert_eq!(s.requests, [Request::NewGame { new_game: true, default_rollout: 523000, saved_position: -1 }]);
    // Continue: recap on; nothing listed → second pass takes the first available.
    let mut s = with_local_player();
    s.config.freedrive_mode = 390145;
    s.game_modes.events =
        vec![EventInfo { id: 10, mode: 390145, available: true, rollout_ok: true, ..Default::default() }];
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

/// The registry the boot path uses: the 100 hooks verified against their handlers (`the_second_twenty` ... `the_fifth_twenty`).
#[test]
fn the_twenty_ported_hooks() {
    let r = Registry::original();
    let ported: Vec<_> = r.hooks().iter().filter(|h| h.ported()).map(|h| h.name).collect();
    assert_eq!(ported.len(), 100, "{ported:?}");
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
