//! HSM hooks of the boot → front end → free-roam flow: the new game / continue setup, the intro, the profile and
//! settings, and the game-mode manager's flags the front end sets and tests. Each is ported from its handler
//! (address in the doc; decomp.md "Native registries: HSM hooks"); the handlers read their argument from the
//! state's data-definition member ([`hook_args`](super::hook_args)) and return the exit point.

use super::game_modes::GameModeManager;
use super::hook_args::{self as arg};
use super::progression::{INTRO_PLAYED, SAVED_POSITION};
use super::{LogicState, Registry, Request};
use crate::hsm::behaviours::{FALSE, TRUE};
use crate::hsm::{EXIT, Key};
use mw_formats::json::Json;

type Args<'a> = Option<&'a Json>;

pub fn register(r: &mut Registry) {
    r.set_hook("IsNewGame", is_new_game);
    r.set_hook("ClearNewGameFlag", clear_new_game_flag);
    r.set_hook("SetNewGame", set_new_game_hook);
    r.set_hook("SetSinglePlayerRollout", set_single_player_rollout);
    r.set_hook("SetRolloutFromProfile", set_rollout_from_profile);
    r.set_hook("ShouldPlayNewGameIntro", should_play_new_game_intro);
    r.set_hook("SetNewGameIntroCompleted", set_new_game_intro_completed);
    r.set_hook("SetIntroEventNext", set_intro_event_next);
    r.set_hook("IsIntroRace", is_intro_race);
    r.set_hook("IsProgressionReady", is_progression_ready);
    r.set_hook("SaveProfile", save_profile_hook);
    r.set_hook("PopulateSettings", populate_settings);
    r.set_hook("SaveSettings", save_settings);
    r.set_hook("CalculateShouldShowRecapSequence", calculate_should_show_recap_sequence);
    r.set_hook("ClearRecapFlag", clear_recap_flag);
    r.set_hook("ClearMatchmakingState", clear_matchmaking_state);
}

/// Hooks ported from their handlers by an earlier, unfinished pass: not verified against the dump yet, so not
/// part of [`Registry::original`]; tests install them with [`Registry::original_with_parked`].
pub fn register_parked(r: &mut Registry) {
    // New game / continue, the intro.
    r.set_hook("SetFreedriveEventToPlay", set_freedrive_event_to_play);
    r.set_hook("SetIntroLoadComplete", set_intro_load_complete);
    r.set_hook("HACKSuspendCarControlForIntro", hack_suspend_car_control_for_intro);
    r.set_hook("HACKResumeCarControlAfterIntro", hack_resume_car_control_after_intro);
    r.set_hook("ShouldShowSafehouseExitSequence", should_show_safehouse_exit_sequence);
    r.set_hook("SetSafehouseFromProfile", exit_only);
    // Profile, settings, progression.
    r.set_hook("ShutdownGame", shutdown_game);
    // Game-mode manager flags.
    r.set_hook("IsChallengeActive", is_challenge_active);
    r.set_hook("SetStoryMode", set_story_mode);
    r.set_hook("SetGameModeWon", set_game_mode_won);
    r.set_hook("SetAllowLoadingScreen", set_allow_loading_screen);
    r.set_hook("SetRetryAtEnd", set_retry_at_end);
    r.set_hook("SetSeenEventsForVehicleToTrue", set_seen_events_for_vehicle_to_true);
    r.set_hook("SetFMNMenuShowingVITAExclusiveEvents", set_fmn_menu_showing_vita_exclusive_events);
    r.set_hook("SetFMNMenuShowingCommonEvents", set_fmn_menu_showing_common_events);
    r.set_hook("ForceOpenEasyDrive", force_open_easydrive);
    r.set_hook("ClearForceOpenEasyDriveFlag", clear_force_open_easydrive_flag);
    r.set_hook("PlayEndOfIntroSequenceInEasyDrive", play_end_of_intro_sequence_in_easydrive);
    r.set_hook("ShouldPlayEndOfIntroSequenceInEasyDrive", should_play_end_of_intro_sequence_in_easydrive);
    r.set_hook("ClearPlayEndOfIntroSequenceInEasyDrive", clear_play_end_of_intro_sequence_in_easydrive);
    r.set_hook("SetHasIntroEasyDriveBeenActive", set_has_intro_easydrive_been_active);
    r.set_hook("HasIntroEasyDriveBeenActive", has_intro_easydrive_been_active);
    r.set_hook("ClearCreditsTransition", clear_credits_transition);
    r.set_hook("SwitchToGameplayLoadWorld", switch_to_gameplay_load_world);
    r.set_hook("ClearJumpToStoreFlag", clear_jump_to_store_flag);
    r.set_hook("LeaveGame", leave_game);
    r.set_hook("ExitForMultiplayerMatchMaking", exit_for_multiplayer_matchmaking);
    // Handlers that only return EXIT on PC (console / online features compiled out).
    for name in [
        "ApplySelectedVehicleRollout",
        "ClearE3MultiplayerProgression",
        "ShowPartyInvitePopup",
        "OnPhotoSharedToWall",
        "ShowX360GamerCard",
        "ShowX360GamerCardEasyDrive",
        "ShowX360GamerCardByPlayerIndex",
    ] {
        r.set_hook(name, exit_only);
    }
}

/// The original's TRUE / FALSE exit points.
pub(crate) fn exit_if(b: bool) -> Key {
    if b { TRUE } else { FALSE }
}

/// Handlers whose body is `return EXIT` on PC: `SetSafehouseFromProfile` (0x676070), `ApplySelectedVehicleRollout`
/// (0x6762c0), `ClearE3MultiplayerProgression` (0x676360), `ShowPartyInvitePopup` (0x7eb730),
/// `OnPhotoSharedToWall` (0x7eb7c0); `ShowX360GamerCard` (0x7eb6f0), `ShowX360GamerCardEasyDrive` (0x7eb710),
/// `ShowX360GamerCardByPlayerIndex` (0x7eb740) look their argument up and drop it.
fn exit_only(_: &mut LogicState, _: Args) -> Key {
    EXIT
}

/// `IsNewGame` (0x675a50): GMM+0x2e87.
fn is_new_game(s: &mut LogicState, _: Args) -> Key {
    exit_if(s.game_modes.new_game)
}

/// `ClearNewGameFlag` (0x675a80).
fn clear_new_game_flag(s: &mut LogicState, _: Args) -> Key {
    s.game_modes.new_game = false;
    EXIT
}

/// `SetNewGame` (0x690360): `NewGame` (bool) → `FUN_0068fd50`.
fn set_new_game_hook(s: &mut LogicState, args: Args) -> Key {
    set_new_game(s, arg::boolean(args, arg::NEW_GAME));
    EXIT
}

/// `FUN_0068fd50`(new game): set up a new or a continued game. The logic fields are ported; the profile tables,
/// the rollout, the saved position and the world's collectables are the runtime's ([`Request::NewGame`]).
pub fn set_new_game(s: &mut LogicState, new_game: bool) {
    s.game.save_pending = true;
    s.progression.request_save();
    s.requests.push(Request::NewGame {
        new_game,
        default_rollout: s.config.default_rollout,
        saved_position: s.progression.last_played(SAVED_POSITION),
    });
    if new_game {
        s.game_modes.new_game = true;
        if let Some(p) = s.players.local_player_mut() {
            p.heat_3d8 = 0.0;
            p.heat_3dc = 0.0;
            p.heat_3e8 = 0.0;
        }
        s.game_modes.recap = false;
    } else {
        s.game_modes.recap = true;
    }
    let event = pick_freedrive_event(s);
    set_pending_event(s, event, false);
    s.game_modes.byte_2eac = false;
    s.game.save_pending = true;
    s.progression.request_save();
}

/// `FUN_0063e380` on the event launcher G+0x2f4c8.
pub fn set_pending_event(s: &mut LogicState, event: u32, flag: bool) {
    s.game.pending_event_aux = -1;
    s.game.pending_event = event;
    s.game.pending_event_flag = flag;
}

/// `FUN_0068fa10`: the free-drive event to play: the first event of the free-drive mode (`GameLogicConfig` +0xd8)
/// that is available (`FUN_006e58a0`) and passes the rollout check (`FUN_0068cc30`); the first pass takes only
/// events listed at G+0x29204, the second any. None: −1 (0xffffffff).
pub fn pick_freedrive_event(s: &LogicState) -> u32 {
    let mode = s.config.freedrive_mode;
    for pass in 0..2 {
        for e in &s.game_modes.events {
            if e.mode != mode || !e.available {
                continue;
            }
            if pass == 0 && s.game.listed_events.binary_search(&e.id).is_err() {
                continue;
            }
            if e.rollout_ok {
                return e.id;
            }
        }
    }
    u32::MAX
}

/// `SetSinglePlayerRollout` (0x690340): `FUN_0068e5c0` on the rollout manager G+0x2f404.
fn set_single_player_rollout(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::SetSinglePlayerRollout);
    EXIT
}

/// `SetRolloutFromProfile` (0x692790): `FUN_00692550`.
fn set_rollout_from_profile(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::SetRolloutFromProfile);
    EXIT
}

/// `SetFreedriveEventToPlay` (0x690310): the pending event = `FUN_0068fa10`'s pick, flag 0.
fn set_freedrive_event_to_play(s: &mut LogicState, _: Args) -> Key {
    let event = pick_freedrive_event(s);
    set_pending_event(s, event, false);
    EXIT
}

/// `ShouldPlayNewGameIntro` (0x676c30) → `FUN_00675210`: not when the pending event is an online event
/// (`FUN_006e60a0`); else when `last_played`[0xb] < 1 (no row reads −1).
fn should_play_new_game_intro(s: &mut LogicState, _: Args) -> Key {
    let play = !s.game_modes.is_online_event(s.game.pending_event) && s.progression.last_played(INTRO_PLAYED) < 1;
    exit_if(play)
}

/// `SetNewGameIntroCompleted` (0x67a840) → `FUN_006798a0`: `last_played`[0xb] = 1, then a profile save as
/// `SaveProfile`'s.
fn set_new_game_intro_completed(s: &mut LogicState, _: Args) -> Key {
    s.progression.set_last_played(INTRO_PLAYED, 1);
    save_profile(s);
    EXIT
}

/// `SetIntroEventNext` (0x6903e0) → `FUN_00690190`: a new game, `last_played`[0xb] = 0, then the intro event
/// (`GameLogicConfig` +0xfc) requested from the GMM and made the pending event.
fn set_intro_event_next(s: &mut LogicState, _: Args) -> Key {
    set_new_game(s, true);
    s.progression.set_last_played(INTRO_PLAYED, 0);
    let intro = s.config.intro_event;
    s.game_modes.request_event(intro, false);
    set_pending_event(s, intro, false);
    EXIT
}

/// `IsIntroRace` (0x680650) → `FUN_0067ffe0`: the current event is `GameLogicConfig` +0x100 and
/// `last_played`[0xb] is exactly 0.
fn is_intro_race(s: &mut LogicState, _: Args) -> Key {
    exit_if(s.game_modes.current_event == s.config.intro_race_event && s.progression.last_played(INTRO_PLAYED) == 0)
}

/// `SetIntroLoadComplete` (0x6761a0): GMM+0x2e82 = 1.
fn set_intro_load_complete(s: &mut LogicState, _: Args) -> Key {
    s.game_modes.intro_load_complete = true;
    EXIT
}

/// `HACKSuspendCarControlForIntro` (0x676c60): G+0x2f0c8 = 3.0 (`.rdata` 0x00f92eb0).
fn hack_suspend_car_control_for_intro(s: &mut LogicState, _: Args) -> Key {
    s.game.input_blocks.suspend_time = 3.0;
    EXIT
}

/// `HACKResumeCarControlAfterIntro` (0x676c90): `FUN_0083c930`(G+0x2f0a4, group 1, bit 6), then G+0x2f0c8 = 0.
fn hack_resume_car_control_after_intro(s: &mut LogicState, _: Args) -> Key {
    s.game.input_blocks.clear(1, 6);
    s.game.input_blocks.suspend_time = 0.0;
    EXIT
}

/// `ShouldShowSafehouseExitSequence` (0x675aa0): FALSE when `FUN_006e0520` is true; that returns 0 on PC, so
/// always TRUE.
fn should_show_safehouse_exit_sequence(_: &mut LogicState, _: Args) -> Key {
    TRUE
}

/// `IsProgressionReady` (0x676ba0) → `FUN_006e3910` on PPM: the progression database is loaded, the local player
/// has a record, `FUN_0083bd20`, and the collectables manager is ready.
fn is_progression_ready(s: &mut LogicState, _: Args) -> Key {
    let p = &s.progression;
    exit_if(p.loaded && s.players.local_player().is_some() && p.world_count_ready && p.collectables_ready)
}

/// `SaveProfile` (0x67a760).
fn save_profile_hook(s: &mut LogicState, _: Args) -> Key {
    save_profile(s);
    EXIT
}

/// The save of `SaveProfile` and `FUN_006798a0`: `FUN_006f90c0` (GMM: the game state into the save) and
/// `FUN_00733c70` (the progression database's snapshot) are the runtime's ([`Request::SaveProfile`]); G+0x2922c
/// = 1 and PPM's save request (`FUN_006e0ef0`).
pub fn save_profile(s: &mut LogicState) {
    s.requests.push(Request::SaveProfile);
    s.game.save_pending = true;
    s.progression.request_save();
}

/// `PopulateSettings` (0x7eb5e0): `FUN_0086e0a0` on the settings (G+0x2f3e0)+0x4210.
fn populate_settings(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::PopulateSettings);
    EXIT
}

/// `SaveSettings` (0x7eb5b0): `FUN_0086fe20` on the same.
fn save_settings(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::SaveSettings);
    EXIT
}

/// `CalculateShouldShowRecapSequence` (0x676bd0): GMM+0x2e88 = 1 (no calculation on PC).
fn calculate_should_show_recap_sequence(s: &mut LogicState, _: Args) -> Key {
    s.game_modes.recap = true;
    EXIT
}

/// `ClearRecapFlag` (0x676b20).
fn clear_recap_flag(s: &mut LogicState, _: Args) -> Key {
    s.game_modes.recap = false;
    EXIT
}

/// `ClearMatchmakingState` (0x676ff0) → `FUN_00675470`: G+0x2f490..+0x2f4c0 and +0x2f708 zeroed.
fn clear_matchmaking_state(s: &mut LogicState, _: Args) -> Key {
    s.game.matchmaking = Default::default();
    EXIT
}

/// `ShutdownGame` (0x676e80): `DAT_011a2470` = 1.
fn shutdown_game(s: &mut LogicState, _: Args) -> Key {
    s.game.shutdown = true;
    s.requests.push(Request::ShutdownGame);
    EXIT
}

/// `IsChallengeActive` (0x675a20): GMM+0x2ce0 ≠ 0.
fn is_challenge_active(s: &mut LogicState, _: Args) -> Key {
    exit_if(s.game_modes.current_event != 0)
}

/// `SetStoryMode` (0x676030): `StoryMode` (bool) → GMM+0x2e7a.
fn set_story_mode(s: &mut LogicState, args: Args) -> Key {
    s.game_modes.story_mode = arg::boolean(args, arg::STORY_MODE);
    EXIT
}

/// `SetGameModeWon` (0x675ff0): `Won` (bool) → GMM+0x2e78.
fn set_game_mode_won(s: &mut LogicState, args: Args) -> Key {
    s.game_modes.won = arg::boolean(args, arg::WON);
    EXIT
}

/// `SetAllowLoadingScreen` (0x676160): `lbEnable` (bool) → GMM+0x2e81.
fn set_allow_loading_screen(s: &mut LogicState, args: Args) -> Key {
    s.game_modes.allow_loading_screen = arg::boolean(args, arg::LB_ENABLE);
    EXIT
}

/// `SetRetryAtEnd` (0x676bf0): `Retry` (bool) → GMM+0x2ead.
fn set_retry_at_end(s: &mut LogicState, args: Args) -> Key {
    s.game_modes.retry_at_end = arg::boolean(args, arg::RETRY);
    EXIT
}

/// A hook that sets one GMM flag and exits through EXIT.
fn set_flag(s: &mut LogicState, f: fn(&mut GameModeManager) -> &mut bool, v: bool) -> Key {
    *f(&mut s.game_modes) = v;
    EXIT
}

/// `SetSeenEventsForVehicleToTrue` (0x676a10): `FUN_006e0480`(1) → GMM+0x2e86.
fn set_seen_events_for_vehicle_to_true(s: &mut LogicState, _: Args) -> Key {
    set_flag(s, |g| &mut g.seen_events_for_vehicle, true)
}

/// `SetFMNMenuShowingVITAExclusiveEvents` (0x676a30): `FUN_006e0470`(1) → GMM+0x2e85.
fn set_fmn_menu_showing_vita_exclusive_events(s: &mut LogicState, _: Args) -> Key {
    set_flag(s, |g| &mut g.fmn_vita_exclusive, true)
}

/// `SetFMNMenuShowingCommonEvents` (0x676a50): `FUN_006e0470`(0).
fn set_fmn_menu_showing_common_events(s: &mut LogicState, _: Args) -> Key {
    set_flag(s, |g| &mut g.fmn_vita_exclusive, false)
}

/// `ForceOpenEasyDrive` (0x676e20): GMM+0x2ca9 = 1.
fn force_open_easydrive(s: &mut LogicState, _: Args) -> Key {
    set_flag(s, |g| &mut g.force_open_easydrive, true)
}

/// `ClearForceOpenEasyDriveFlag` (0x676e40).
fn clear_force_open_easydrive_flag(s: &mut LogicState, _: Args) -> Key {
    set_flag(s, |g| &mut g.force_open_easydrive, false)
}

/// `PlayEndOfIntroSequenceInEasyDrive` (0x676ec0): GMM+0x2caa = 1.
fn play_end_of_intro_sequence_in_easydrive(s: &mut LogicState, _: Args) -> Key {
    set_flag(s, |g| &mut g.end_of_intro_in_easydrive, true)
}

/// `ShouldPlayEndOfIntroSequenceInEasyDrive` (0x676ee0).
fn should_play_end_of_intro_sequence_in_easydrive(s: &mut LogicState, _: Args) -> Key {
    exit_if(s.game_modes.end_of_intro_in_easydrive)
}

/// `ClearPlayEndOfIntroSequenceInEasyDrive` (0x676f10).
fn clear_play_end_of_intro_sequence_in_easydrive(s: &mut LogicState, _: Args) -> Key {
    set_flag(s, |g| &mut g.end_of_intro_in_easydrive, false)
}

/// `SetHasIntroEasyDriveBeenActive` (0x676fa0): GMM+0x2f28 = 1.
fn set_has_intro_easydrive_been_active(s: &mut LogicState, _: Args) -> Key {
    set_flag(s, |g| &mut g.intro_easydrive_active, true)
}

/// `HasIntroEasyDriveBeenActive` (0x676fc0).
fn has_intro_easydrive_been_active(s: &mut LogicState, _: Args) -> Key {
    exit_if(s.game_modes.intro_easydrive_active)
}

/// `ClearCreditsTransition` (0x676f30): GMM+0x2f26 = 0.
fn clear_credits_transition(s: &mut LogicState, _: Args) -> Key {
    set_flag(s, |g| &mut g.credits_transition, false)
}

/// `SwitchToGameplayLoadWorld` (0x676f50): GMM+0x2e9b = 1.
fn switch_to_gameplay_load_world(s: &mut LogicState, _: Args) -> Key {
    set_flag(s, |g| &mut g.switch_to_gameplay_load_world, true)
}

/// `ClearJumpToStoreFlag` (0x677080): GMM+0x2f27 = 0.
fn clear_jump_to_store_flag(s: &mut LogicState, _: Args) -> Key {
    set_flag(s, |g| &mut g.jump_to_store, false)
}

/// `LeaveGame` (0x6765a0): GMM+0x2e9a = 1.
fn leave_game(s: &mut LogicState, _: Args) -> Key {
    set_flag(s, |g| &mut g.leave_game, true)
}

/// `ExitForMultiplayerMatchMaking` (0x677040): GMM+0x2e9d = 1.
fn exit_for_multiplayer_matchmaking(s: &mut LogicState, _: Args) -> Key {
    set_flag(s, |g| &mut g.exit_for_mp_matchmaking, true)
}
