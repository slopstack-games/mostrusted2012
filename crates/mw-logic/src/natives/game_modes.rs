//! The game-mode manager **GMM** (G+0x2f3fc) as far as the HSM hooks read and write it, with its methods the hooks
//! call that act only on its own fields. Offsets are GMM's (decomp.md "Boot, loading screens and the free-roam
//! start", "Races").

use super::{LogicState, Request};

/// An event of the game-mode data (the list at (GMM+4)+0x11c / +0x120: handles to the events; filled by the
/// runtime from the install).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventInfo {
    /// The event's +0x18: its id (UniqueId).
    pub id: u32,
    /// The event's +0x1c: its game mode (compared with `GameLogicConfig` +0xd8 by `FUN_0068fa10`).
    pub mode: u32,
    /// The event's type is `Genesys.Gen.OnlineEvent` (0x0100000000045d55) or derives from it (`FUN_00ac54c0`
    /// walking the type chain for 286037).
    pub online: bool,
    /// `FUN_006e58a0`(GMM+4, id): the event is available to the player (442 bytes of profile checks, not ported:
    /// the runtime fills it).
    pub available: bool,
    /// `FUN_0068cc30`(id, 0) on G+0x2f404's table: the event passes the rollout manager's check (not ported).
    pub rollout_ok: bool,
    /// `FUN_006ec6a0`(id, the local player's vehicle): the event is done (`FUN_006ec620` → `FUN_006e39c0`, else
    /// `FUN_00727b80`'s flag; progression data, not ported: the runtime fills it). `GetNextEventForProgression`.
    pub completed: bool,
}

/// Fields of the GMM the hooks touch.
#[derive(Debug, Clone, Default)]
pub struct GameModeManager {
    /// The events (GMM+4 → +0x11c / +0x120).
    pub events: Vec<EventInfo>,
    /// (GMM+4)+0xe8: the running event is online (`FUN_006dfed0`, `IsOnlineEvent`).
    pub online_event: bool,
    /// The event kind byte (+0x10) of the type record `FUN_0053c020` returns for the current event handle
    /// ((GMM+4)+0xf4 / +0xf8); 0x34 and 0x47 are speed traps (`IsSpeedTrapEvent`). Filled by the runtime.
    pub current_event_kind: i8,
    /// +0x2ce0: the current event's id (0 = none; `IsChallengeActive`).
    pub current_event: u32,
    /// The current game mode's id: the `GameMode` object behind the handle +0x2cf4 / +0x2cf8, its +0x18.
    pub current_mode: u32,
    /// +0x2ce4: the requested event (`FUN_006e74f0`).
    pub requested_event: u32,
    /// +0x2d10 / +0x2d14: the requested event's handle when it is an online event of [`Self::events`] (index),
    /// else the null handle (`DAT_01179528`).
    pub requested_online_event: Option<usize>,
    /// +0x2e7e: an event is requested.
    pub event_requested: bool,
    /// +0x2e80: the request's flag (second argument of `FUN_006e74f0`, only with an event).
    pub event_request_flag: bool,
    /// +0x2ce8 (post-event awards: cleared by `FUN_006e6670`, passed to `FUN_00736a90`).
    pub award_2ce8: u32,
    /// +0x2e70: the post-event award amount (`FUN_00736a90` when > 0).
    pub award_amount: i32,
    /// +0x38a0, count +0x38b4: the awards `PreparePostEventAwards` collected.
    pub awards: Vec<u32>,
    /// +0x2eb4 (cleared with the awards).
    pub award_2eb4: u32,
    /// +0x38cc (cleared with the awards).
    pub award_38cc: u32,
    /// +0x2300..+0x2370 (`FUN_006e28c0` zeroes 20 of these dwords; index = (offset − 0x2300) / 4).
    pub results_2300: [u32; 29],
    /// +0x2ca9: `ForceOpenEasyDrive`.
    pub force_open_easydrive: bool,
    /// +0x2caa: play the end-of-intro sequence in Easydrive.
    pub end_of_intro_in_easydrive: bool,
    /// +0x2e74: `TimerRunning` (`FUN_006f4650`).
    pub timer_running: bool,
    /// +0x2e78: the game mode was won (`SetGameModeWon`).
    pub won: bool,
    /// +0x2e7a: story mode.
    pub story_mode: bool,
    /// +0x2e7f: `HasTransitionedEvents`.
    pub has_transitioned_events: bool,
    /// +0x2e81: the loading screen may show (`ShowLoadingScreen` binding).
    pub allow_loading_screen: bool,
    /// +0x2e82: the intro's load is complete.
    pub intro_load_complete: bool,
    /// +0x2e85: the Find-My-Next menu shows the Vita-exclusive events (`FUN_006e0470`).
    pub fmn_vita_exclusive: bool,
    /// +0x2e86: the events for the vehicle are seen (`FUN_006e0480`).
    pub seen_events_for_vehicle: bool,
    /// +0x2e87: `IsNewGame`.
    pub new_game: bool,
    /// +0x2e88: show the recap sequence.
    pub recap: bool,
    /// +0x2e90: `IsFreedriveEvent` (`FUN_006e03d0`).
    pub freedrive_event: bool,
    /// +0x2e9a: `LeaveGame`.
    pub leave_game: bool,
    /// +0x2e9b: `SwitchToGameplayLoadWorld`.
    pub switch_to_gameplay_load_world: bool,
    /// +0x2e9d: `ExitForMultiplayerMatchMaking`.
    pub exit_for_mp_matchmaking: bool,
    /// +0x2eac (`FUN_006e04a0`; cleared by `FUN_0068fd50`, meaning open).
    pub byte_2eac: bool,
    /// +0x2ead: retry at the end.
    pub retry_at_end: bool,
    /// +0x2f26: the credits transition.
    pub credits_transition: bool,
    /// +0x2f27: jump to the store.
    pub jump_to_store: bool,
    /// +0x2f28: the intro's Easydrive has been active.
    pub intro_easydrive_active: bool,
    /// +0x2e77: the world is loaded for the event (`Mode.WorldLoaded` with an event; `Mode.ShowLoadingScreen`).
    /// Set by the runtime (the front end's world-loaded flag).
    pub world_loaded: bool,
    /// +0x2e76: read by `Mode.ShowLoadingScreen` for online events only (meaning not traced).
    pub byte_2e76: bool,
    /// +0x2ea8: a two-bit set; `Mode.ShowLoadingScreen` holds while any bit is set (writers not traced).
    pub loading_bits: u32,
    /// +0x2e91 / +0x2e92: in the game's intro, free-drive part / the other (`Mode.IsInGameIntro`,
    /// `Mode.IsInGameIntroFreedrive`).
    pub in_game_intro_freedrive: bool,
    pub in_game_intro_other: bool,
    /// +0x2d08: `EndReason`.
    pub end_reason: i32,
    /// +0x2e64 / +0x2e68: `ScoreLimit` / `EventScoreLimit`.
    pub score_limit: i32,
    pub event_score_limit: i32,
    /// +0x2e7b `ShowTakedownCameras`, +0x2e7c `ForceEnd`, +0x2e7d `ShowSplitTime`, +0x2e89 `InMeetup`, +0x2e8a
    /// `LastEventWasRace`, +0x2e8b `IsRaceEvent`, +0x2e8d `IsAmbushEvent`, +0x2e8e `IsScoreBasedEvent`, +0x2e93
    /// `IsBiggerBetter`, +0x2e94 `UsePauseMap`, +0x2e96 / +0x2e97 `PlayerBustedIn(Last)Event`, +0x2e99
    /// `NeedTheMapScreen`, +0x2f10 `DisplayTimeout` (the `Mode` binding table, `FUN_0070a120`).
    pub show_takedown_cameras: bool,
    pub force_end: bool,
    pub show_split_time: bool,
    pub in_meetup: bool,
    pub last_event_was_race: bool,
    pub race_event: bool,
    pub ambush_event: bool,
    pub score_based_event: bool,
    pub bigger_better: bool,
    pub use_pause_map: bool,
    pub player_busted_in_event: bool,
    pub player_busted_in_last_event: bool,
    pub need_map_screen: bool,
    pub display_timeout: bool,
    /// The `GameMode` items (`mode_machine.rs`; filled by the runtime from the install).
    pub modes: Vec<super::mode_machine::ModeInfo>,
    /// +0x2cfc: the serial of the HSM machine the mode runs (−1 in the original = `None`).
    pub machine: Option<u32>,
    /// (GMM+4)+0xdc == 4: the event data manager holds the current event's data (`FUN_006dfe00`; what loads it
    /// is not ported: the runtime sets it).
    pub event_data_ready: bool,
    /// The weapon list set on the local player for the mode (player +0x606c → +0xb4, `FUN_006bb720`).
    pub weapon_list: u32,
    /// ... and the one its loader holds (+0xb0 with the state +0xb8 == 2; set by the runtime). The mode's machine
    /// starts once they agree (`FUN_006bb6f0`).
    pub weapon_list_loaded: u32,
    /// +0x2510: the skip prompt's time (`ModeVM.ShowSkipPrompt`: shown while > 0).
    pub skip_prompt_time: f32,
    /// +0x2526: `HasTargetSpeed` (`ModeVM.ShowPositionFromCheckpoint` needs it clear).
    pub has_target_speed: bool,
}

impl GameModeManager {
    /// `FUN_006e74f0`(id, flag): request an event; its handle is kept only for an online event of that id.
    pub fn request_event(&mut self, id: u32, flag: bool) {
        self.requested_event = id;
        self.event_requested = id != 0;
        self.event_request_flag = flag && id != 0;
        self.requested_online_event = None;
        if id != 0 {
            for (i, e) in self.events.iter().enumerate() {
                if e.id == id && e.online {
                    self.requested_online_event = Some(i);
                }
            }
        }
    }

    /// `FUN_006e60a0`(id): the event of that id is an online event (no such event: false).
    pub fn is_online_event(&self, id: u32) -> bool {
        self.events.iter().any(|e| e.id == id && e.online)
    }

    /// `FUN_006e6670`: clear the post-event awards (and `FUN_006e28c0` the results block).
    pub fn clear_post_event_awards(&mut self) {
        self.award_2eb4 = 0;
        self.award_amount = 0;
        self.award_2ce8 = 0;
        self.awards.clear();
        self.award_38cc = 0;
        for off in [
            0x2300, 0x2318, 0x231c, 0x2330, 0x2344, 0x2320, 0x2334, 0x2348, 0x2324, 0x2338, 0x234c, 0x2328, 0x233c,
            0x2350, 0x232c, 0x2340, 0x2354, 0x2368, 0x236c, 0x2370,
        ] {
            self.results_2300[(off - 0x2300) / 4] = 0;
        }
    }
}

/// `FUN_006f4650`(running): on a change, start (`FUN_006f0f80`) or stop (`FUN_006f1120`) the mode timer.
pub fn set_timer_running(state: &mut LogicState, running: bool) {
    if state.game_modes.timer_running != running {
        state.game_modes.timer_running = running;
        state.requests.push(Request::GameModeTimer { running });
    }
}
