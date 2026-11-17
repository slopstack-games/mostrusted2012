//! The original's managers, as far as the natives read and write them (one sub-struct per manager, in the file of
//! the area that owns it; fields name their offset in the original, as decomp.md does).

/// The logic state the natives act on.
#[derive(Debug, Default)]
pub struct LogicState {
    /// What the natives asked the engine to do, in call order; the runtime drains it every frame.
    pub requests: Vec<Request>,
    /// Calls of natives that are known but not ported yet (`"hook Name"` / `"lua Name"`), for logs and tools.
    pub unported: Vec<String>,
    /// What bindings read ([`ViewModels`](super::ViewModels)).
    pub view_models: super::ViewModels,
    /// What the runtime reports about the engine ([`Engine`]).
    pub engine: Engine,
    /// G (`*0x01177868`): the fields the hooks touch directly.
    pub game: super::game_logic::GameLogic,
    /// `GameLogicConfig` fields the hooks read (filled by the runtime from the install).
    pub config: super::game_logic::LogicConfig,
    /// GMM (G+0x2f3fc).
    pub game_modes: super::game_modes::GameModeManager,
    /// PM (G+0x2f3dc).
    pub players: super::players::PlayerManager,
    /// The progression database and the profile manager (G+0x14c08).
    pub progression: super::progression::Progression,
}

/// Engine facts the logic reads, set by the runtime (or a tool) before the logic runs.
#[derive(Debug)]
pub struct Engine {
    /// `FUN_00897960` on `*0x010edafc`: the graphics are prepared (`BWAIT4GFX`).
    pub graphics_prepared: bool,
    /// The loading-screen manager's request mask (UI manager +0x2d204) is nonzero and +0x2d200 clear
    /// (`HSB_WAIT4LS` / `HSB_WAIT4NL`).
    pub loading_screen_showing: bool,
    /// The UI clock (`*0x010edacc` → +4 → +0x18): ticks at 3,000,000 per second (i64 at +8) and the u32 at +0,
    /// advanced by [`LogicHost::frame`](super::LogicHost::frame).
    pub ui_ticks: i64,
    pub ui_counter: u32,
}

impl Default for Engine {
    fn default() -> Self {
        Engine { graphics_prepared: true, loading_screen_showing: false, ui_ticks: 0, ui_counter: 0 }
    }
}

/// A request to the engine (the runtime in `mostrusted`, or a tool's stub). Each names the original function that
/// does the work; the runtime does what that function does and updates [`LogicState`] where the doc says so.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Request {
    /// `FUN_0068fd50` (`SetNewGame`, `SetIntroEventNext`), the parts outside [`LogicState`]. New game:
    /// `FUN_0072bf20` (reset the progression tables), `FUN_0068e5c0` (the single-player rollout), the profile's
    /// car from `default_rollout` (`FUN_00683480` / `FUN_00729f20`), `FUN_00707380`, `FUN_00655e90`,
    /// `FUN_00847d70`. Continue: `FUN_00847d70`, and when `saved_position` (`last_played`[0xd]) ≥ 0 the saved
    /// position (`FUN_0083c7b0` → `FUN_00847780`). Both: the world's collectables (`FUN_0070d260`,
    /// `FUN_007000d0`), the recommendations (`FUN_007355b0`(1, 0), `FUN_007357f0`(0)), GMM+0x2feb
    /// (`FUN_006ecfa0`, `GameLogicConfig` +0x1f0).
    NewGame { new_game: bool, default_rollout: u32, saved_position: i64 },
    /// `FUN_0068e5c0` on the rollout manager G+0x2f404: the single-player car and its rollout.
    SetSinglePlayerRollout,
    /// `FUN_00692550`: the car and rollout from the profile.
    SetRolloutFromProfile,
    /// `FUN_006f90c0` (GMM: the game state into the save) and `FUN_00733c70` (the progression database's
    /// snapshot, the save time); then the save itself.
    SaveProfile,
    /// `FUN_0086e0a0`: the settings (G+0x2f3e0)+0x4210 refresh their four pages from the profile.
    PopulateSettings,
    /// `FUN_0086fe20`: the settings are written to the profile.
    SaveSettings,
    /// `DAT_011a2470` = 1: the game quits.
    ShutdownGame,
    /// The mode timer started (`FUN_006f0f80`: the event's `Timeline` sequences, the timeout, the wrong-way
    /// state) or stopped (`FUN_006f1120`); GMM+0x2e74 already holds `running`.
    GameModeTimer { running: bool },
    /// `FUN_006f89f0`: the game mode resets.
    ResetGameMode,
    /// `FUN_00726ca0`(event) on the recommendations (Autolog's recommended event after a retry).
    RecommendationRetry { event: u32 },
    /// `FUN_00673d70`: the recommendations' event for the available vehicles (`FUN_00723b10` / `FUN_00725130`),
    /// else `FUN_006e74f0`(that id, 0) on the GMM ([`GameModeManager::request_event`](super::game_modes::GameModeManager::request_event)).
    SetEventFromAvailableVehicles,
    /// `FUN_0067f700`: the pursuit and event statistics restart (`FUN_00659d00`(1), `FUN_00656920`,
    /// `FUN_006600d0`(event, mode, online), `FUN_00656f60`(the car)).
    ResetCurrentEventStats { event: u32, mode: u32, online: bool },
    /// `FUN_0067f570`: the time of day from the current event.
    SetTimeOfDayFromEvent,
    /// `FUN_006f4960`: posts the time-of-day sync message (offline only).
    SyncTimeOfDay,
    /// `FUN_006fbb60`: collect the post-event awards into the GMM ([`GameModeManager`](super::game_modes::GameModeManager)
    /// `award_amount`, `award_2ce8`, `awards`).
    PreparePostEventAwards,
    /// `FUN_006e65e0`: `FUN_00736a90`(award_2ce8, event, amount) when amount > 0; each award: `FUN_00736ab0`(award,
    /// event) when `FUN_0071f7e0` knows it, else `FUN_006792b0`(award).
    AwardPostEventAwards { event: u32, amount: i32, award_2ce8: u32, awards: Vec<u32> },
    /// `FUN_00686f30`: the event's AI racers are added (slots 12..23).
    AddAllAIPlayers,
    /// `FUN_006d1310`: each slot's vtable +0x20, the AI slots 12..23 removed (`FUN_006ce180`(slot, 3)), PM+0xa660 /
    /// +0xa694 / +0xa698 = 0.
    RemoveAllRaceCars,
    /// `FUN_006ce180`(slot, reason): the player in the slot is removed (the runtime clears the slot).
    RemovePlayer { slot: usize, reason: u32 },
    /// `FUN_008456d0` on G+0x2f484: every roadblock is removed.
    RemoveRoadblocks,
    /// `FUN_00846830`: message 0x15, the props reset.
    ResetProps,
    /// `FUN_006c9fb0`(0, 0) on the local player: respawn at `event`'s start (0 = where the player is) or at the
    /// last swap spot (decomp.md "Start position").
    Respawn { event: u32, to_swap_spot: bool },
    /// `FUN_0065b830`(the pursuit manager G+0x2f410, 1): the chase statistics restart.
    ResetChaseStats,
    /// `FUN_00844940`(vehicle): message 0x16, the vehicle's damage is reset.
    ResetVehicleDamage { vehicle: i32 },
    /// `FUN_0068acb0` (inactive) / `FUN_0068ad50` (active): the local car's damage model.
    VehicleDamage { active: bool },
    /// `FUN_006926b0`: the local car's rollout re-applied clean (`FUN_0068e9d0`(…, 2)) and its damage reset.
    CleanAndRepairVehicle,
    /// `FUN_0065de30`(G+0x264e0, slot): the player's nitrous is emptied.
    ClearNitrous { slot: i32 },
    /// `FUN_0067f8c0`: the HUD's message queues are emptied (UIM+0x12458 records, +0x12468, +0x124f8, +0x12574).
    ClearHudMessageQueues,
    /// UIM+0x126e8's message freed, +0x126e0 = 0.
    ClearHudInstantQueue,
    /// `FUN_00a44dd0` on the sequence players UIM+0x47748 and +0x4774c.
    StopAllSequences,
    /// `FUN_008423a0` on the speech manager G+0x2c3c0.
    StopSpeech,
    /// `FUN_0079f760` on the song manager UIM+0x30370.
    SkipCurrentTrack,
    /// The UniqueId at `field` of the object behind the handle `[DAT_010edb08]+4` +0xb278 (type open), mapped by
    /// `FUN_007983b0` (UIM+0x32858) and started on the sequence player UIM+0x47748 (`FUN_00a45070`).
    PlayUiSound { field: u32 },
}
