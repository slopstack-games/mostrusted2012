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
}

/// A request to the engine (the runtime in `mostrusted`, or a tool's stub).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Request {
    // --- From HSM behaviours (`hsm::behaviours`; each names the behaviour and the original's call).
    /// `RUN_PG_RNNR2` enter (0x76f7f0 → `FUN_0083dc70` on the PageRunner2 at G+0x2ee60): push the PageRunner2
    /// context `name` with its default page `page` (the `DEF_PAGE_REF` id printed "%d"); a context of that name
    /// already on the stack is replaced. The runtime then answers `Query::PageContextExists` /
    /// `PageContextExit` / `PageContextBusy` for it.
    PushPageContext { name: String, page: String },
    /// `RUN_PG_RNNR2` exit (0x76eb50): first `FUN_0083b920` (→ `FUN_007f5f80` on the page runner; what it does
    /// is open), then, if the context is still on the stack, suspend it (`suspend`: `FUN_007e0c80` flags the top
    /// context +0x40 / +0x41, it stays) or pop it (`FUN_007f09d0` → `FUN_007e0cd0`).
    LeavePageContext { name: String, suspend: bool },
    /// `RUN_PG_RNNR2` with `RESTOREGRP` true, when it finishes: the UI manager's focus group (`*`(G+0x2f480)
    /// +0x31c04) = `group` (saved on enter, `Query::FocusGroup`), with +0x31c1c and +0x31c1e set (restore
    /// pending).
    RestoreFocusGroup { group: u64 },
    /// `ACTI_WDGT_GR` update (0x76e4e0 → `FUN_007f9350` on the screen manager's page, G+0x2f458 +0x280):
    /// activate the widget group `group` of the current page (its widgets show and take input).
    ActivateWidgetGroup { group: i64 },
    /// `ACTI_WDGT_GR` exit begin (0x76e5e0): play the page's `ScreenTransition` for leaving `group` through
    /// `exit` (`FUN_007f78a0`: a transition entry of the group whose exit point is `exit` or empty), else the
    /// group's own one (`FUN_007f58e0`); neither starts while the transitions' state (G+0x2f458 +0x4b0 +0x3ac)
    /// is 3. `Query::WidgetTransitionStarted` answers whether one started.
    PlayWidgetGroupTransition { group: i64, exit: u64 },
    /// `ACTI_WDGT_GR` exit update (0x76ffd0 → `FUN_007f9bf0` on the page): deactivate `group` (`instant`: no out
    /// animation); when the page's out time (+0x68) is 0 its widgets are released at once (`FUN_007f1b40` on
    /// page +8), else when the out animation ends. `Query::WidgetGroupOutTime` answers the out animation.
    DeactivateWidgetGroup { group: i64, instant: bool },
    /// `ACTI_WDGT_GR` update: the page's exit point (+0x208) was taken by this group: clear it.
    ClearWidgetExit,
    /// `SETBLACKBACK` enter (0x745b10): the UI manager's black background (UIM+0x2d260) on / off.
    SetBlackBackground(bool),
    /// `CHANGEVSYNCS` enter / exit (0x73b980, 0x73b9e0): G+0x2f723, the 60 Hz ("full rate") mode on / off (exit
    /// restores the value saved on enter).
    SetFullRate(bool),
    /// `MUTE_SOUND` enter (0x73e220) / exit (0x73e2a0): count a mute `reason` (`MUTEREASON` in the table at
    /// 0x010c5c28: 0 Unknown, 1 LoadingScreen, 2 CarSelect) up (`on`) or down (`FUN_00790920` on UIM+0x2d2f0:
    /// one byte per reason at +2999); all sound is muted while any count is nonzero (`FUN_004b8b20`).
    MuteSound { reason: u8, on: bool },
    /// `LOADWORLD` enter (0x74e040): load the world selected in the world list (G+0x2f468: entry +0x8c of the
    /// list +0x88): `FUN_008465c0` on G+0x2ef6c with it, and the load-world message (type 3, the world's name,
    /// `FUN_00844190` on G+0x2ef70 → `FUN_0084c960`).
    LoadWorld,
    /// `LOADPROFILE` enter update (0x73da30 → `FUN_0071f010`): start loading the profile (save system
    /// G+0x2f454: buffer cleared, a load request; +0x4942 = loading). `Query::ProfileLoaded` answers the end.
    LoadProfile,
    /// `LOADPROFILE` exit (0x744040 → `FUN_0073daf0` on G+0x2f454 +0x4964): apply the loaded profile's
    /// settings (audio volumes from the options record `FUN_0046efe0` +0x16c / +0x170 / +0x174 × 1/100 to the
    /// mixer channels 3–8 `FUN_004b8b90`, the UI manager's copies at +0x306c8, ...).
    ApplyProfileSettings,
    /// `HSB_P1BIND` (0x73e730): release the primary user (G+0x2f721: the sign-in manager sets the user record
    /// `*`(0x010edad8)+4 +0xb28c back to 1 = unbound) while `true`.
    ReleasePrimaryUser(bool),
    /// `HSB_P1BIND` update (0x744810): listen for the press that binds the primary user (G+0x2f720, user slot
    /// G+0x2f48c = 0; `any_button` G+0x2f722: any button, else start / enter) while `listen`.
    ListenForPrimaryUser { listen: bool, any_button: bool },
    /// `HSB_P1BIND` (0x73e730 → `FUN_006ce500(2)` on the players G+0x2f3dc): remove every player; players
    /// +0xa6b8 = −1.
    RemovePlayers,
    /// `HSB_P1BIND` (0x744810 → `FUN_006d6090(0, 0)`): create the local player for the bound user;
    /// `Query::LocalPlayerReady` answers when it exists (players +0xa6b8 is then set to 0).
    CreateLocalPlayer,
    /// `RUNGAMEMODE` enter (0x7450e0 → `FUN_00701cb0` on the game-mode manager G+0x2f3fc): start the event /
    /// game mode `event` (G+0x2f6f8): its mode's HSM machine runs (the runtime starts the instance from the
    /// mode's data and answers `Query::GameModeMachine` with its serial). Unless the local player exists with
    /// its byte G+0x25694[player +0xa0] set, also posts message 5 (`FUN_00846710`; meaning open).
    StartGameMode { event: u32 },
    /// `RUNGAMEMODE` exit while the mode still runs (0x73eed0): end reason 2 (GMM+0x2d08), the UI manager's
    /// mode lists freed (`FUN_0067f8c0`, `FUN_006796b0`), the current event cleared (`FUN_006e74f0(0, 0)`); the
    /// behaviour asks the mode's machine to exit itself.
    StopGameMode,
    /// `RUNGAMELOGIC` enter (0x75c870): play the sequence `sequence` (a sequence resource of a loaded bundle,
    /// `FUN_007983b0`; none found: nothing plays) on `player` (`Style` 0 = the UI manager's default player
    /// +0x4774c, 1 = the sim-timer player +0x47748 (also `UseSimTimer` true), 2–5 = the players at +0x12568 /
    /// +0x126dc / +0x1245c / +0x124ec, played with `FUN_00a45070`; `None` = no style: played on its own with
    /// `FUN_00a44f30`); `playback` names it for `Query::SequencePlaying` / [`StopSequence`](Self::StopSequence).
    PlaySequence { playback: u32, sequence: u32, player: Option<u8> },
    /// `RUNGAMELOGIC` exit (0x73edd0): stop the playback if it still plays (`FUN_00a44d50` / `FUN_00a43150`).
    StopSequence { playback: u32 },
    /// `LOAD_ALL` (0x740f00): open the gate `DAT_010f77ed` that world loading waits for.
    LoadAll,
    /// `DBAUTOSELWD` (0x7702e0): select the world `Query::AutoSelectableWorld` found (the GeneSys config's debug
    /// world, else the world list's only entry) in the world list (G+0x2f468, `FUN_00645780`).
    AutoSelectWorld,
    /// `SELECT_PROF` update (0x73f270): the platform user `user` is known; the profile's name is `user` printed
    /// "%llu" (into `DAT_010ecc10`, `FUN_00428860`).
    SetProfileUser { user: u64 },
    /// `SELECT_PROF` exit (0x75cd00): post message 0xc {0, 1} on the queue `DAT_010edb0c` (`FUN_0076aad0`; its
    /// receiver is open).
    ProfileUserSelected,
    /// `BINGAME` enter update, once (0x73d130): post the in-game start messages: `FUN_008433f0(0, v)` on G+0x2f470
    /// (queue `DAT_010edb24`, v = profile option 11 (`FUN_0072f610(0xb)`) clamped to 0–3, kept at +4; then +8 set)
    /// and `FUN_00847d70` (queue `DAT_010edb38`).
    EnterGame,
    /// `BINGAME` exit update, once (0x73d1d0): remove the players (`FUN_006ce500(0)`), the local player's vtable
    /// +0x20 when it has one, reset the UI manager's two lists (`FUN_007b3410` ×2) and stop the two sequence
    /// players' sequences (`FUN_00a44dd0` ×2). `Query::GameLeaving` answers the wait.
    LeaveGame,
    /// `HSM_PLAYING` enter (0x73e7e0): `FUN_006fa720` on G+0x14e40 (meaning open), the session counters (G+0x2f3ec
    /// +0x130 = profile stat 9, +0x134 = 0); the "GAMESESSSTRT" telemetry is not ported.
    StartPlaying,
    /// `HSM_PLAYING` exit (0x73e930): reset the players' slots (`FUN_006d1310`), clear the loading-screen request
    /// bit 10 (UIM+0x2d204) and UIM+0x2d230; the "GAMESESSENDS" telemetry is not ported.
    StopPlaying,
}
