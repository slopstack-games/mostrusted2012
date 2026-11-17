# Research log

Dated findings and open questions about the game's formats and behaviour. Add new entries at the top. When a
question is answered, move the answer into the parser's doc comment and [formats.md](formats.md), and note
the date here.

Install used for verification: Steam build, `VERSION.TXT` = 1.4.0.0, 2,020 bundles.

Older entries (before 2026-11, and the early topical findings) are in
[research-log-archive.md](research-log-archive.md); move a month there once the next one has begun.

---

## Dated findings

### 2026-11-17: ownership check: a retail executable (`mw_gamedir::GameDir::ownership`)

User decision: the base game counts as owned when `NFS13.exe` is a known retail build; DLC is never inferred from
the files (the Steam depot ships every pack; the DLC lock is the logic's entitlements, agent round of the same day).
- `GameDir::ownership` hashes `NFS13.exe` (SHA-256, ~14 MB) against `RETAIL_BUILDS` (release, hash). Known: Steam
  1.4.0.0 (app 1262560). The EA app build is not known yet: add its hash once a copy is confirmed.
- `mostrusted` refuses to start on an unknown executable, before indexing, in both the native and browser builds
  (`run`). The message gives the hash to report. Tools (`mwtool`) do not check.
- Hashes are the one exception to "nothing derived from game data" (CLAUDE.md), and they hold no content.
- Tests: `mw_gamedir::ownership_tests` (the Steam install is retail; a fake executable is refused).
### 2026-11-17: sixth 20 HSM hooks verified and enabled (120 of 187; `natives/hooks_select.rs`, `hooks_flow.rs`)

Handlers read from `re/out/live/text.asm`, callees from the dump. Argument names found by hash search over the
snapshot's `.bss`: `liOptionId` (`DAT_01177a98` = 0x74b4f5f4), `liSelectedWorldId` (`DAT_01177bd8` = 0x4ee0ce22);
`lpcValue` strings go through `strtol(s, 0, 10)` (`FUN_00d751e0`) or `atol`. Mapping (all return EXIT unless noted):
CancelMilestoneTracking 0x675e50 (`FUN_0071d1e0` on profile db +0x5a3c: clears the tracking fields) ; HasEasyDriveOptionBeenSeen 0x676920
(`FUN_0072eb90`: seen byte of the "ez_options" row, TRUE/FALSE); SetEasyDriveOptionSeen 0x676970 (`FUN_00674d20`: ez_options row via
`FUN_007291b0`, or the "ez_mods" row via `FUN_007316d0` when `FUN_006ecae0` knows the id); SetEasyDriveOptionSeenToTrue 0x6769d0
(`FUN_00674d90`: online flag (GMM+4)+0xe8 and id != 0 -> `FUN_00703c10` only, else the rows with 1 then `FUN_007040b0`); SetSelectedWorld
0x675ad0 (`FUN_00645780` on G+0x2f468); SetAvailableVehicleListEvent 0x675cc0 (existing Request); CalculateDistanceToEventByIdString 0x675e00
(`FUN_0078ff50`: UIM+0x1f8f0 +0xcff0 = n, +0xd544 = 1); SetRolloutFromVehicleIdString 0x675ee0 (n > 0: `FUN_006c7950`(local player, n, 0));
SetHighlightedRolloutByIdString 0x675f40 (rollout manager +0x1c4 = n); SetHighlightedVehicleByIdString 0x680390 (`FUN_0067ef40`: +0x1c8 = n when
changed, record copied to +0x1cc or cleared); IsTargetScoreHit 0x6803e0 (local player +0x418 >= event record +0xcc, TRUE/FALSE);
ShowTargetScoreReminderMessage 0x680440 (`FUN_0067f7f0`: message 0x5f5ee "score" = target - score); SelectPaintPackFromEasyDrive 0x68d3a0
(`FUN_0068d200`); UpdatePendingRollout_EasyDrive 0x6903a0 (`FUN_00690030`) and _Stock 0x68ae60 (`FUN_0068aa40`: UI sound +0x64, then
`FUN_006830a0`(id, 0, 0)); SetFinalEventData 0x6766f0 (`FUN_006b9d00`: local player +0x5b8 = GMM+0x2ca3 + 1, +0x5c8 / +0x5cc = GMM+0x2d28 / +0x2d2c, +0x5d0 = 0);
StartEventFromBinding 0x676790 (`FUN_00674be0`: `lpcValue` is a binding read as int, out value starts at -1; not -1: `FUN_006e74f0`(id, 0));
SetWaypointToEventFromBinding 0x6767d0 (`FUN_00674c60`: same, then `FUN_00792390`, `FUN_00796ff0`(id), UIM+0x2c5e0 = -1);
UpdateProgressAwardsStrings 0x6768c0 (`FUN_0071fd20`(-1, +0x10e8) and (6, +0x12e8) on G+0x2f3f8); SetFMNMenuShowingVITAExclusiveEvents
0x676a30 (`FUN_006e0470`(1): GMM+0x2e85; was parked, now enabled). Test `the_sixth_twenty`; `the_twenty_ported_hooks` counts 120.
Model decisions: the "ez_options" seen flags are a mirror in `SelectFields` (the ez_mods branch is not mirrored; the profile db row write is the
`Request`); `event_target_score` (the event record +0xcc) and the player's score are filled by the runtime; the meaning of GMM+0x2ca3 / +0x2d28 / +0x2d2c is open;
a binding that does not resolve reads as -1 (assumed: the visitor leaves the preset out value).
Open: the runtime handles none of the new `Request`s (CancelMilestoneTracking, SetEasyDriveOptionSeen*, SetSelectedWorld, SetRolloutFromVehicle,
LoadHighlightedVehicle, ShowTargetScoreReminder, SelectPaintPack, UpdatePendingRollout, SetWaypointToEvent, UpdateProgressAwardsStrings).
Next 20 candidates (all unported; the ten Offline ones first): CalculateBlacklistDescription (0x676a70), ResolveBoolBinding (0x680160),
SetVehicleAndRolloutFromSaveGameByVehicleID (0x680680), SetVehicleAndRolloutFromSaveGameByVehicleIDString (0x687a30), UpdateAwardProgressBindings
(0x7eb4e0), IncrementSetting (0x7eb610), DecrementSetting (0x7eb680), ExitWidgetGroup (0x7f24f0), UpdateDeferredViewModelSilent (0x7f9940),
HasScreenRegisterdDeferredViewModel (0x7f9990); then small Online ones: SetJumpToStoreFlag, ClearEntitlementJumpID, ClearNetworkedTimer,
ClearOnlineEasyDrive, RefreshOnlineEasyDrive, SetGameVisibilityInviteOnly, SetGameVisibilityJoinable, SetGameVisibilityPublic,
SetOnlineRaceCarFrozen, SetOnlineRaceCarUnfrozen.
### 2026-11-17: 10 HUD / pursuit HSM behaviour types (`hsm/behaviours_hud.rs`; 85 types)

Route check: `mwtool hsm --run --steps 900` (docs/commands.md, free roam) enters four still-placeholder types:
`HORN_PRESS`, `WAITSYSMENU`, `COSTSTATESC`, `WAITFORCHASE` (all under "Controlling Vehicle Offline"; the rest of the route's 64 codes
were ported). Static walk of the machines 390148 / 371633 / 371635 added the next candidates in tree order: `DISABLETIME`,
`DISABLEWORLD`, `HOSTEZGUIDE`, `CLEARUIPOSTF`, then `SETPENDINGON`, `OVERRIDEPAGE` (all ported). Method as the earlier batches (factory = `mov edx`
of the name stub, vtable from the factory, slots from `text.asm`, parameter / exit keys decoded from snapshot 1; addresses in decomp.md).
Requests: CostStatePrune, CostStateScan, DisableStatsTimer, SetWorldRendering, DisableAudio, HostEasyGuide, ReleaseEasyGuide, ClearUiPostFx,
SetPendingDialog, OverridePage. Queries (default): HornPaused (false), HornDown (false), SystemMenuActive (false), CostStateActive (false),
LocalPursuitState (0), WorldRendering (true), EasyGuideShown (true), EasyGuideClosed (true). One test per type in `flow_tests.rs`.
Result: the free-roam command still reaches "Freedrive Update" and "Freedrive HUD" with an identical state trace (the four route types held before
and hold now, waiting for engine answers: system menu, pursuit state, horn input).
Model decisions: `COSTSTATESC`'s scan (0x7548e0 from 0x754989, the cost-record walk) is one Request, not ported; `HORN_PRESS`'s timer has no reader in the machine and
is kept in the instance, the input checks are two Queries; the clock for both timers is the game clock (`FUN_0046ee10`, step x rate; a stopped
clock = 0). `HOSTEZGUIDE`'s object fields are Queries / Requests (+0x54 state, +0x58 flag). `SETPENDINGON` has no known meaning for G+0x2f70c (named after the parameter).
Open, next unported on the machines: `SHOWDIALOG` (73 uses; enter 0x745e10 / update 0x740a20 / exit 0x740a80 read: DIALOG, POS_EXIT, NEG_EXIT via UI dialog
object G480+0x47740), `HSMPAUSEINTR` (0x73d100 / 0x74d040 / 0x74cfe0, scans UI objects), `LIMITEDEDSP` (0x75aff0 / 0x762650: 7-state online request),
`SUPPRESSPREF` (0x740f70 / 0x740f90 / 0x740fb0), `TAKEPHOTO` / `PHOTO_*`, `RUNPRCONTEXT`, `CLEARUIPOSTF` siblings; the sign-in family (`SIGNIN_CNTRL`, `SI_*`, `HSM_KEYB_ENT`, `HSM_SERV_OP`) is the bulk (offline-unreachable).
### 2026-11-17: the `UIConfig`, `SignInState`, `UISequences` and `GeneralSequence` binding roots

Registered like `Mode` (`FUN_00a4d2a0` root / type / object, `FUN_00a4e2f0` fields, `FUN_00a4e350` getters). Code:
`natives/ui_config_view.rs`, `sign_in_view.rs`, `sequence_view.rs`; one `ROOTS` line each. Tests per root.

**`UIConfig`** (`FUN_007a03c0`, `UIConfigManager`, 0x41c). Constructor values: `Platform` (+0x208) = the `.rdata`
string "PC" (0xfde864), `IsPlatformPC` 1, X360 / PS3 / VITA 0, `IsHighDef` 1, `NumberOfMonitors` 1. Language
getters read the UI language id at +0xb4 (0 until set): `IsEnglish` 0x78ec50 (1 or 2), German 6, Russian 7, Japanese
0xb, Traditional Chinese 0xc, Arabic 0xe (same codes as the language menu). `ShouldShowSubtitleOptions` (0x7a9df0) and
`IsLeftToRightLanguage` (0x7a9e20) answer true for language 0, else read a byte of the UI language table record
(`FUN_00794f30`), not ported. Model decisions: LTR false only for Arabic, subtitle options shown. Open: the runtime
does not set the language (`screen.language` stays 0, so `IsEnglish` is false until something sets it).
`HudAspectCorrection.*` / `Data.*` stay with the UI data.

**`SignInState`** (`FUN_00888d40`, `SignInViewModel` 0x3148, `Params` = `SignInParamsViewModel` at +8). Read from the
getters: `IsChatRestricted` (0x889bf0) stores 0, `IsMultiplayerAllowed` (0x889be0) stores 1, `IsAccountDeactivated`
(0x881610: error code +0x135c == 0x1b), `AuthenticatedWithBlaze` (0x889c10 -> 0x4c2070 -> 0xa82f20: the Blaze
connection object is null), `IsReadyForToS` (0x881650: byte +0x1589), plain bytes for `AccountCreated`,
`FirstUseInTitle`, `EMailHasEAAccount`, ...; all false / empty offline. **Finding:** the original's
`IsChatRestricted` is false, not true; the boot's "Get Online Silently" then goes to the silent sign-in (state
1597850 / `ServerOperation PCLogin`, the Blaze login, not ported: an offline run would wait for it). The earlier
`--answer SignInState.Params.IsChatRestricted=TRUE` is therefore an online stand-in (model decision: skip the
login), not a value of the code, and it stays in the documented commands. `SignInState.Error` is not a registered
member (does not resolve).

**`UISequences`** (`UISequenceService` 0x3258) and **`GeneralSequence`** (`GeneralSequenceService` 0x98). Getters:
`HideHudRequests` +0x3254, `IsAnimationTriggerSet[n]` (0x82bcb0, bit set at +0x20), `IsActionHostUsed["X"]`
(`FUN_0082fbe0`: byte 1 of the entry at +0x34 + 2 * i; unknown = false; set by `FUN_00829b20`), `UseDefaultHud` /
`UseThermalHud` / `UseNightHud` (0x82c690 / 0x82c720 / 0x82c7a0: mode = first nonzero request counter of +8 (none
match), +0x10 (thermal 2), +0x14 (night 3), +0x18 / +0x1c (neither), else 1 = default), `Params[n].Text1..3` (16 slots
of 0x320 B, valid byte +0x4c; invalid = null). General triggers: the service's bits at +0x88 (`IsTriggerSet[n]`,
0x82e950) and up to five named ones (`Triggers["NAME"]` = `FUN_0082fb00`: sorted table at +0xc, bits from +0x20, a
missing name is a null object; `Trigger.IsTriggerSet` 0x82c650). Setter `FUN_00829d30(name, n)` sets bit n - 1 (name 0:
the service's; a new name is added unless five exist), clearer `FUN_00829810` (an emptied name leaves the table,
`FUN_0082ea50`). `hash("BOOT")` is 1112493908, the `GeneralTriggerSequenceItem` name field the assets keep.
**The settable path:** `LogicState::set_general_trigger(name_hash, index)` / `clear_general_trigger` (and
`state.sequences` for HUD requests, action hosts, params, animation bits); the runtime calls them when the sequence
item fires / ends. `mostrusted/src/logic.rs` now does that for "BOOT" instead of answering the binding itself
(not compiled in this session: engine-free task; the integrator's build checks it); `mwtool hsm --run` sets BOOT
at start. Model decision: the named table is keyed by the binding's name hash (the getter looks up hash + 1, the
setter the value as given; the callers' packing is not traced).

**Result.** The documented free-roam command needs two answers instead of nine (`IsChatRestricted=TRUE`,
`1222788=DONE`; the `Mode.*`, `UIConfig.Platform`, BOOT and `Transitions` answers are gone) and still reaches
"Freedrive Update" and machine 371633; the 90-step boot still reaches "Playing". Captures that would confirm:
a live read of UIConfig +0xb4 and the SignIn VM bytes at the title.

### 2026-11-17: fifth 20 HSM hooks verified and enabled (100 of 187; `natives/hooks_{flow,ui}.rs`)

All 20 were already ported but parked; handlers re-read from `re/out/live/text.asm`, now enabled in `Registry::original`.
Easydrive sounds (OnOpenEasyDrive 0x6877b0 / OnStartEvent 0x687830 / OnSelectEasyDriveOption 0x6878b0 /
OnOpenEasyDriveMods 0x687930 / OnCloseEasyDriveMods 0x6879b0): identical bodies, the UniqueId at `[obj+0xb27c + [obj+0xb278]+0x28 + f]`
(obj = `[DAT_010edb08]+4`) with f = 0x50 / 0x60 / 0x58 / 0x54 / 0x4c; if non-zero `FUN_007983b0`(UIM+0x2f480... +0x32858) maps it and
`FUN_00a45070` starts it on UIM+0x47748 (`Request::PlayUiSound`), then EXIT. GMM (= `[G+0x2f3fc]`) byte flags: +0x2caa
(PlayEndOfIntroSequenceInEasyDrive sets 1, Clear sets 0, Should tests), +0x2f28 (SetHasIntroEasyDriveBeenActive / Has...), +0x2e9b
(SwitchToGameplayLoadWorld), +0x2e9a (LeaveGame), +0x2e9d (ExitForMultiplayerMatchMaking), all set 1 and return EXIT; the tests
return TRUE / FALSE. EXIT-only bodies: ClearE3MultiplayerProgression (0x676360), ShowPartyInvitePopup (0x7eb730), OnPhotoSharedToWall
(0x7eb7c0), ShowX360GamerCard / ...EasyDrive / ...ByPlayerIndex (0x7eb6f0 / 0x7eb710 / 0x7eb740: a `FUN_009f16b0` hash lookup, result
dropped). The three PC handlers return `DAT_0117efa0`, not 0x1177ef0; both hold the same 8-byte key in `snap1/mem.bin`
(`00000068 a1614568`), so EXIT. The 20th, ForceKeyboardShutdown (0x677060): `FUN_00845dc0` sets bit 3 of `*DAT_010edb0c`, posts
{0xf, 0} through `FUN_0084dcc0`, clears the bit (`Request::ForceKeyboardShutdown`). Test `the_fifth_twenty`; `the_twenty_ported_hooks` counts 100.
Model decisions: none new. Open: the runtime does not handle `Request::ForceKeyboardShutdown` / `PlayUiSound` yet.
Next candidates (offline, unported): SetFMNMenuShowingVITAExclusiveEvents (parked), CancelMilestoneTracking (0x675e50:
`FUN_0071d1e0` on profile db +0x5a3c), HasEasyDriveOptionBeenSeen, SetEasyDriveOptionSeen, SetEasyDriveOptionSeenToTrue,
SetSelectedWorld, SetAvailableVehicleListEvent, CalculateDistanceToEventByIdString, SetRolloutFromVehicleIdString,
SetHighlightedRolloutByIdString, SetHighlightedVehicleByIdString, IsTargetScoreHit, ShowTargetScoreReminderMessage,
SelectPaintPackFromEasyDrive, UpdatePendingRollout_EasyDrive, UpdatePendingRollout_EasyDrive_Stock, SetFinalEventData,
StartEventFromBinding, SetWaypointToEventFromBinding, UpdateProgressAwardsStrings.
### 2026-11-17: 9 more HSM behaviour types of the free-drive machines (`hsm/behaviours_mode2.rs`)

`MUTEALLRUMBL`, `PAUSEMSGS`, `PLAYSONGLIST`, `WEAPONSOK`, `MINIMAPICONS`, `SYNCPROFILEW`, `HIDECHEV`, `PAUSEAI`,
`DOBODYSHOP` (67 types in all; addresses and slots: decomp.md "The HSM interpreter", fourth batch). Method as the
earlier batches: factory from the name registration (string push, `mov edx, factory` near 0x778xxx), vtable from the
factory, slot bodies from `text.asm`, parameter keys from snapshot 1. Requests: MuteRumble, PauseMessages,
ForcePauseMessageQueues, PlaySongList, StopSongList, WeaponsAllowed, ResetPlayerWeapons, UpdateMinimapIcons,
SetProfileSyncFlag, HideChevron, PauseAi, UpdateBodyShop. Queries (default): CurrentEventSongList (0),
DefaultEventSongList (0), EventWeaponsAllowed (false), GmmFlag2e80 (false), ProfileServiceConnected (false),
ProfileSyncFlag (false). One test per type in `hsm/flow_tests.rs`; `mwtool hsm --run` (docs/commands.md) still
reaches "Freedrive Update" with "Do Body Shop" and "Mute All Rumble (Paused)" now running real behaviours.
Model decisions: `SYNCPROFILEW`'s connected states 1..14 (the online profile sync) are not ported, connected is
treated as offline; `DOBODYSHOP`'s update and `MINIMAPICONS`' update are single per-frame Requests (the work is
engine state; the body shop's zone-list reads `FUN_005372f0` / `FUN_005378d0` and the minimap visibility check
0x74e0b0 are not ported); `EventWeaponsAllowed` defaults to false (the free-drive record's byte +0xf2 was not read);
`PAUSEMSGS` keeps queue names, not pointers. Open: the host side of these 12 Requests (message system, song
player, rumble); `ResetPlayerWeapons` = `FUN_006c8e90` only summarised; the free-roam route still holds
`HSMCARSWAPLO` etc. (agent B2). Captures that would confirm: G+0x2ef50..60 and G480+0x325e8 in free roam.
### 2026-11-17: 8 event-flow HSM behaviour types (`hsm/behaviours_event.rs`)

`SETENDREASON`, `MODEENDCHECK`, `HSB_WAIT4IN`, `SET_LAY_SET`, `DOBLACKLISTS`, `CHECKPTS`, `EVENTSTARTS`, `HSMCARSWAPLO` (66 types
in all). Method as the entry below, with one correction: the factory of a name is the function *before* the name stub
(the stub's `mov edx, <factory>`), not the one after it; its `mov [eax], vtable` gives the vtable. Entry-point and exit
constants (`ENDED` 0x117a2d8, `RETRY` 0x117a668, `EXIT`, `TIME_OUT` 0x117a978, `CREDITS` 0x117a0e8) decode with the
packed-name alphabet from snapshot 1. Details per type: decomp.md "The HSM interpreter".
Model decisions: `SETENDREASON` / `MODEENDCHECK` write GMM+0x2e78 / +0x2d08 through `Request::SetGameModeWon` /
`SetEndReason`, which `natives::mode_machine::apply` applies to `GameModeManager::won` / `end_reason` (the `Mode`
binding root reads them); `FUN_006f46e0` (won, offline: medal / time maths on event data), `FUN_006f1530` (limit
reached), `FUN_0074e790`'s list scan and `FUN_006e27e0` (display time left; its `FUN_00eda646` is `_ftol` of 0.0, read
from the asm) are `Query`s the runtime answers (the host answers only the online `won`). `HSB_WAIT4IN`'s candidate
exits are `Query::InputExit { index }` (first one the parent handles wins, as the original's chain; the two cues
`FUN_0079fb40` are not ported). `DOBLACKLISTS`: phases 0, 2, 3, 6 and the credits tail ported; the steps of phases
1 / 4 / 5 are engine requests whose effect on the phase is a query; the view model `BlacklistShutDown` is published
with defaults only. `CHECKPTS`, `EVENTSTARTS`, `HSMCARSWAPLO` (0x200, 0x1f0, 0x74d0 bytes of instance state): lifecycle,
exit conditions and parameters ported, the per-frame work is `Request`s (checkpoint passing, event-start markers,
the 7-phase car swap) for the runtime: open, with "Rules that are C++ ... are ported from the code" still to do.
Test: 11 in `hsm/flow_tests.rs`; `mwtool hsm --run` still reaches "Freedrive Update".

### 2026-11-17: skid / crash sound glitch: not reproduced headless (user: "extremely loud, distorted")

- **Original vs ours** (`ref/xkr_launch`: the original's own audio, the inputs replayed with the tap, levels
  relative to the full-throttle phase): hard braking is +1.8 dB in the original and +1.0 dB in ours (our skid
  channel −7.8 dB). Our car decelerates more slowly under the same brake input (20–30 mph where the original has
  stopped), a handling difference.
- **Live output = tap**: real-time drives recorded from the null sink's monitor (`parecord`; `MW_INPUT_SCRIPT`
  with `MW_INPUT_SCRIPT_START=playing`): a handbrake turn, a 58 mph crash, then wheelspin against the obstacle
  (spin 1.0 at 0 mph and 7800 rpm in first gear while the throttle is held, the `Spin` set at −40 dB). Peaks 0.25
  at the user's master 0.35, no clipping, no step over 0.2. The live mix matches the tap's two channels × 0.6 ×
  0.35 within a dB.
- **Race intro**: the intro's teleports give ~1 m/s landings, which start the `Spin` / landing sequences; the
  vehicle channel there is ~8 dB over a free-roam drift.
- Not the cause: the volume channels (equal bases 0.6), a doubled vehicle bank (built once), the engine
  normalisation missing from the tap (it is in the bank's master gain), the device format (48 kHz stereo like
  the null sink).
- The browser runs at master 1.0 (no settings file): +9 dB against the desktop's saved 0.35.
- Open: the user's exact situation; a capture of the original's drift and crash with audio.

### 2026-11-17: the `ModeVM`, `Net`, `JoinGameState` / `CreateMatchState` / `MatchMakeState` roots

Root dispatch: `LogicState`'s `binding::Host` now looks the root hash up in a table (`state.rs` `ROOTS`), one module
per root (`natives/<root>_view.rs`); a member a module does not answer falls through to published definitions.

**`ModeVM`** (`modevm_view.rs`). `GameModeViewModel` is registered by `FUN_006fe520` with ECX = the view model
embedded in the game-mode manager at manager +0x20 (`FUN_0070be40` calls `FUN_006e7670` on `esi+0x20` at 0x70c5de,
then `FUN_0070a120` on the manager with the stack argument `param_2 + 0xe8`, the binding manager the registrations go
to). So a VM offset is a manager offset minus 0x20: `ForceOpenEasyDrive` VM +0x2c89 = manager +0x2ca9 (the hook's
byte), `ModsAppliedInOutro` VM +0x24fc = manager +0x251c (`GarageFields::mod_flag`); the VM keeps back-pointers to
the manager (+0x1914) and the game logic (+0x1910). Getters ported: `CanSkipIntro` (0x711d90, always true),
`ShowSkipPrompt` (0x711dc0: manager +0x2510 float > 0.0, the constant at 0xf8f344), `ShowPositionFromCheckpoint`
(0x716e50: manager +0x2526 `HasTargetSpeed` clear and the current mode id is not 0x212851). The 6fe520 block also
registers more `GameModeManager`-typed (`Mode`) fields (`HasTargetSpeed`, `DisablePauseMenu`, `PersonalBest`, ...):
not ported as `Mode` members (unresolved members read false / 0 as before). Not ported: the blacklist getters,
`IsWaitingForBustRetry` (0x6e2d10: +0x2e90 clear and `UISequences.IsActionHostUsed["RETRY"](NOT)`),
`WasBlacklistBeatenAndNotShutdown` (0x6e8860), `GetFinishedString` (0x6e85c0), `ShouldShowInCarPosition`
(0x716ec0, a UI global), `CurrentContext`.

**`Net`** (`net_view.rs`; `FUN_004c4740`, 28 getters, all `LAB_`, read from text.asm). Offline answers from the
getters' no-connection branches: `IsPlayerConnectedToFirstyParty` true (0x4cf2a0 constant), `AreAllPlayersConnected`
true (0xa83320: empty slots count as fine), `IsInX360Party` false (constant); `IsInOnlineGame`, `IsLoggedInToServer`,
`IsConnectedToServer`, `IsHost`, `IsGameAdmin`, `IsInLobby`, `IsInPlaygroup`, `AreWeHostMigrating`, ... false; counts 0.
Model decision: the Blaze / session objects never exist offline (not ported). The indexed members do not resolve.

**`JoinGameState`, `CreateMatchState`, `MatchMakeState`** (`online_state_view.rs`): `IsValid` (0x64d820 / 0x64d870 /
0x64d850: a validity field of the zero-initialised object) and `DoRematchmaking` (0x651660, needs the playgroup
pointer): false (model decision).

**Result.** `mwtool hsm --run` (docs/commands.md "Freedrive Update") needs no `Net.IsLoggedInToServer` answer any
more and still reaches "Freedrive Update". "Open EasyDrive Request" (1560481) takes FALSE through `ModeVM.ForceOpenEasyDrive`
and 1532918 ("Is In Online Game") takes FALSE: both exits handled. Tests `modevm_view`, `net_view`,
`online_state_view`.

Open: entitlement getters (`HasPdlc1..4`: `FUN_006401b0` on global ids), `EntitlementManager.DlcMessageIndicator`
and `ThankYouCount` (0x651480, 0x6514c0); `UIConfig` / `SignInState` / `Autolog` / `PlayDirector` / `StoreViewModel`
and the other HSM-read roots (listing: `mwtool bindings` rows with source `hsm:`) are unported; `Recommends`,
`Screen`, `Transitions`, `Demo`, `ActiveGameMode` are not registered roots (published data or the engine).

### 2026-11-17: the `Mode` binding root (`natives/mode_view.rs`)

**Registration.** `FUN_0070a120` (`ECX` = the game-mode manager) registers the root "Mode" (`.rdata` 0x00fc7ef0)
of type "GameModeManager":
- 34 fields (`FUN_00a4e2f0`: name, offset into the manager, count, property type).
- 86 getters (`FUN_00a4e350`: name, getter, count, property type). The full table with offsets and getter
  addresses is in the dump at 0x70a120.
- Property types: 9 bool, 0 / 1 / 2 integers, 8 float, 0x11 time, 0xf / 0x10 strings, 0x1a object.
- It then calls `FUN_006fe520`, which registers "ModeVM" (`GameModeViewModel`) on its argument (the caller
  `FUN_0070be40` passes param_2 + 0xe8). ModeVM is not ported.

**Port.**
- `LogicState` is now the binding host: `Mode` first, then the view models. A `Mode` member the table does not
  answer falls through to a data definition published as "Mode" (`Mode.Time`, `Mode.DisablePauseMenu`).
- The resolve call sites moved from `view_models.resolve_*` to `LogicState::resolve_*`.
- Fields: every manager byte the logic keeps. New `GameModeManager` fields: +0x2e77 world loaded, +0x2e76,
  +0x2ea8, +0x2e91 / +0x2e92, +0x2d08, +0x2e64 / +0x2e68, and the event-kind flags.
- `Mode.Data` = the current `GameMode` object. Its members are the GeneSys fields by hash
  (`Events::game_mode_fields`, `mode_view::data_fields`).
- Getters read from `text.asm`:

  | Getter | Address | Reads |
  |---|---|---|
  | `WorldLoaded` | 0x712010 | an event and +0x2e77 |
  | `ShowLoadingScreen` | `FUN_006f49c0` | +0x2e81, +0x2e7f, event, online / +0x2e76, +0x2e77, then the two bits of +0x2ea8 |
  | `HasIntroLoadCompleted` | 0x70da10 | +0x2e82 |
  | `IsInGameIntro` / `…Freedrive` | 0x711e90 / 0x711e70 | +0x2e91 or +0x2e92 / +0x2e91 |
  | `HudHsm` | 0x716f40 | the mode's `GameMode` +0x20 |
  | `IsPauseAllowed` | `FUN_006e62a0` | |
  | `RetryLastEventEnabled` | 0x711ee0 | +0x2eac |
  | `LeaveGameRequested` | `FUN_00711f30` | |
  | `ShowRecommendsIntro` / `RecommendsEventActive` | 0x7120c0 | G+0x2f6d0..d8: none offline |

- The online session getters are false offline.
- The runtime sets +0x2e77 from `FrontEnd::ready`. The `mwtool hsm --run` driver sets it at once.

**Result.**
- Boot to free roam no longer needs the six `Mode` answers. `mwtool hsm --run` with only the boot's answers and
  `Transitions` reaches "Freedrive Update", and the HUD machine 371633 comes from the mode's data.
- "Is Recommends" takes FALSE.
- In the runtime the mode machine now takes the recap branch. `Mode.ShowRecap` (+0x2e88) is set by the ported
  `CalculateShouldShowRecapSequence` for the continued profile, so "Show Game Recap" pushes page 1560241 and waits
  for its group (`mwtool hsm --run` uses a fresh logic state).
- Test `mode_view::tests::mode_root`.

Model decisions:
- The local player's +0x2c8 (`LeaveGameRequested`) is taken as clear.
- Members not ported (times, online / Autolog / recommendation getters, career statistics) do not resolve.

Open:
- `ModeVM` (`ForceOpenEasyDrive` at +0x2c89 of its object: the EasyDrive prompt's "Open EasyDrive Request").
- `Net.IsInOnlineGame`.
- Whether the original shows the recap on a continued profile (a capture of the original's boot with a save).
- The recap page in the runtime.

### 2026-11-17: widget input and conductors (`natives/pages.rs`, `logic.rs`, `input.rs`)

**The input mapping table** (`FUN_007e7910`, `.rdata` 0x00fe7260, 33 entries of {name, id, directional flag}; an
unknown name gives 0x60):
- The documented 42–61, then `to_main_menu` 62, `play_it` 63, `fullscreen_photo` 64, `friend_recommends` 65,
  `hitbox` 66, `dbg_win_event` 67, `dbg_switch_progression` 68, `easy_left` / `right` / `up` / `down` 69–72,
  `universal_accept` 73, `universal_alt` 74. The flag is 1 for 42–51 only.
- Default bindings: keyboard `FUN_00483470` (Windows virtual-key codes), pad `FUN_00482250` (pad codes
  cross-checked against 46–49 and 52–55):

  | Action | Keyboard | Pad |
  |---|---|---|
  | `easy_*` 69–72 | Numpad 4 / 6 / 8 / 2 | d-pad (codes 8 / 9 / 6 / 7, plus an argument `DAT_00fa1db8` not read) |
  | `universal_accept` 73 | Enter | A |
  | `universal_alt` 74 | Backspace | X |
  | `to_main_menu` 62 | F1 | Start |
  | `scroll_up` / `down` 50–51 | Numpad 8 / 2 | right stick Y |
  | `play_it` 63, 65 | A | — |
  | `fullscreen_photo` 64 | Right Ctrl | — |
  | `dbg_*` 67 / 68 | F10 / F11 | — |

**Action definitions:** `UserActionDefinition` = GeneSys object `0x01000000_<id>` in the screen bundles:
`InputMapping`, `Analogue`, field `2da735e3` = repeat (`AssetStore::ui_action_definition`).

**Widget input** (`LogicHost::input`, every frame before the HSM update):
- Each active widget's actions fire per `FUN_007f50e0`: analogue every frame; repeat on the press or the
  auto-repeat tick, directional ones only above 0.05; otherwise on the press.
- Routing follows `FUN_007f40d0`:
  - Cross-widget `Widget::Function`: offered to the target's scripts until one handles it.
  - Lua: every script of the widget out of its 0.15 s cooldown; a refusal by `ActiveBinding` also starts it.
  - Native: `ExitGroupVia` → the group exit; other names → the HSM hook.
- `input.rs` `MenuAction` gained 50, 51, 60, 62, 69–74 and `MenuInput::by_id`.
- Test `tests/pages.rs::calibration_actions` (boot screen group 1532543): accept → NEXT, cancel → BACK, the Lua
  `IncrementSetting` runs.

**Conductors:**
- The title's conductor "PressStart" is the front end's title: `StopSequence` / `DestroyConductor` stop it.
- Any other `StartSequence` runs headless (`logic.rs` `Playback`): the sequence loaded on a thread, its clock
  stepped with its jump controllers (bindings through the logic's view models), and the conductor set idle at
  the end, so `HasSequenceFinished` / `IsConductorIdle` keep the original's timing.
- A binding that gives no sequence id makes the conductor idle at once.

Model decisions:
- Every active widget takes input (the focus / modal rules were not traced).
- An analogue value reaches a Lua callback as an extra argument.
- A native callback calls its hook without arguments.
- No success / failure sounds yet (the definition's handles `7866a8e2` / `1cb2a09a` were not read).
- `MW_INPUT_SCRIPT_START=playing` lets a script run in free roam.

Open:
- Drawing conductors' sequences: a generic sequence player (frontend.rs and event_card.rs each draw their own
  today). `PlaySequence` (Lua) is on the boot, pause (431082) and two more screens; the native `PlaySequenceFast`
  is on 44 screens and is not ported.
- In free roam nothing fireable is active yet. The HUD's `ShowMap` (Tab) is gated by `Mode.IsInGameIntro(NOT)`, and
  EasyDrive's group 1135023 opens only through "Open EasyDrive Request". Both need the `Mode` roots.

### 2026-11-17: front-end screens from the HSM: page contexts, widget groups, the boot page (`natives/pages.rs`, `logic.rs`)

**Page contexts and widget groups** (`mw_logic::natives::pages`, inside `LogicHost`; `LogicHost::apply` carries out
the logic's part of a request for every host).
- `PushPageContext` pushes {name, page} and asks the host for the page's screen (`Pages::wanted` →
  `pages::load_screen` → `Pages::provide`; feature `assets`). `PageReady` is true once the screen is loaded.
- `ActivateWidgetGroup` builds each widget of the group as decomp.md "Widget definitions (JSON) and widget scripts"
  specifies: one Lua state per widget with the natives bound, the chunk run, `Construct(this)`,
  `SetConfigurationItem` per scalar `Script` entry, then `OnActivated`.
- `PreInputUpdate(dt)` runs every frame. `DeactivateWidgetGroup` / `LeavePageContext` call `OnDeactivating` +
  `Destruct`.
- The widgets' Lua states live in `LogicHost::pages`, outside the `RefCell` the natives borrow.

Model decisions:
- The `Widget` root is one published JSON object of the active widgets' typed fields. The original resolves it
  through the script's view model, slot 8. Adapters are not published yet.
- Every native is bound, not only `LuaHooks`.
- `WidgetExit` (page +0x208) is read from WM+0x488.
- Native screen scripts are not run.

**The boot page, from the original's data** (`mostrusted --hsm`; `frontend.rs` "driven" mode):
- "Run PageRunner2 Context - Boot Flow" pushes 491012 (12 widgets). "Activate Press Start Widget Group" activates
  491032. Its `Background` widget's `PlaySequence` script creates the conductor "PressStart" and starts
  `Widget.Background.Sequence`, which resolves to 1022357, and `frontend.rs` plays the title.
- The key press only binds the primary user (`HSB_P1BIND`). The HSM then asks for the group's transition
  (`PlayWidgetGroupTransition` 491032 → the boot transition 1836020).
- `TransitionFinished` follows the front end's transition. `TransitionState` stays at its default 2, so the group
  deactivates after 10 frames (the original's cap) while the transition holds over the load, and the script stops
  its sequence.
- The boot then leaves the page and, in free drive, pushes the HUD screen 371621 (98 widgets) and activates its
  groups.
- Headless timings: title started by the logic at 0.58 s, key at 10.85 s, transition at 10.87 s, playing at
  15.65 s. Without `--hsm` the flow is unchanged (key 10.76 s, playing 15.56 s).
- Test: `mw-logic/tests/pages.rs` (needs the install).

Open:
- `CreateConductor` / `StopSequence` / `DestroyConductor` are still logged only. The front end's sequence player
  should become the conductor.
- The other group transitions (data: each group's `ScreenTransition`) and `TransitionState`.
- The HUD screen's widgets run their scripts, but nothing draws from them yet (`hud.rs` is still hand-written).
- `Mode.*` roots: `Mode.ShowRecommendsIntro` and two others leave "Select HUD" / the EasyDrive prompt with an
  unhandled exit.

### 2026-11-17: the original's HSM runs inside the runtime, shadow mode (`--hsm`, `mostrusted/src/logic.rs`)

`mostrusted --hsm` runs "Top State" (20527) every frame on `LogicHost`, next to the hand-written flows. The
game-mode manager's update (`mode_machine::update`, `FUN_006abc10` order) runs after each HSM update. Requests are
logged (`hsm:` lines, the first of each kind) and applied to the logic state (`mode_machine::apply`); the runtime
does not carry any of them out yet. Hook-queued requests are drained right after the hook, so the log keeps the
original's call order. Answers from runtime state: the title's "BOOT" trigger (`FrontEnd::boot_trigger`),
`PrimaryUserBound` (the title key press while `ListenForPrimaryUser` listens), `NewProfile` (`--new-game` or no
saved profile), the loading screen (`Phase::Loading`), `Mode.WorldLoaded` (`FrontEnd::ready`) and
`Transitions.Transitioning` (`Phase::Transition`). The other answers are stand-ins (model decisions), the same as
`mwtool hsm --run`: platform PC, offline, chat restricted, `ConnectToAutolog` exits DONE, snapshot 1's `Mode`
bindings (HudHsm 371633, FeedbackGroupID 2, ...), events available when listed, weapon lists loaded at once,
unported behaviours hold and unported hooks answer EXIT.

Headless front-end run (`tools/t --front-end --hsm --front-end-shots scratch/fe --mute`):
- The machine waits on the title until BOOT (frame ~550, ≈ 9.3 s, as the title sequence sets it), then in
  "Press Start" until the key press.
- Then, in two frames, it goes boot → profile ("Set Up Continue Game" for a saved profile) → "Playing" →
  "Run Game Mode" → machine 390148 → "Wait for Loading" → "Show World Freedrive" / "Freedrive Update", and the
  free-drive HUD machine starts.

"Wait for Loading" passed at once because our world was already loaded (10.3 s, before the press). Every other
engine wait answers "done" at once because the runtime does not carry out the Requests yet: profile, world and
feedback-group loads.

Open: carry out the Requests one by one and drop the hand-written flow each replaces: `PushPageContext` /
`ActivateWidgetGroup` (front-end screens), `LoadWorld`, `LoadProfile`, `StartGameMode`, the HUD machine's requests.
`Mode` / `Event` binding roots (the GMM's binding table) instead of the stand-ins.

### 2026-11-17: fourth 20 HSM hooks verified and enabled (80 of 187; `natives/hooks_{flow,event,ui}.rs`)

Handlers read from `re/out/live/text.asm`, callees with `fn`. Already ported but parked, now verified byte for byte and
enabled: SetSafehouseFromProfile (0x676070) and ApplySelectedVehicleRollout (0x6762c0) return EXIT only;
ClearHudInstantQueue (0x67a7b0: frees UIM+0x126e8 through `FUN_00cc4780`, +0x126e8 = 0, +0x126e0 = 0);
StopAllSequences (0x680480: `FUN_0067f8c0` = ClearHudMessageQueues, then `FUN_00a44dd0` on UIM+0x47748 / +0x4774c);
StopSpeech (0x676100: `FUN_008423a0` on G+0x2c3c0; the Lua native of the same name is a separate handler 0x6784e0);
SkipCurrentTrack (0x676b40: `FUN_0079f760` on UIM+0x30370); ToggleLoadingScreenOff (0x677010: UIM+0x2d204 &= ~0x20,
+0x2d21c = 0); ClearForceOpenEasyDriveFlag (GMM+0x2ca9 = 0), ClearCreditsTransition (GMM+0x2f26 = 0),
ClearJumpToStoreFlag (0x677080: GMM+0x2f27 = 0; the table lists it as Online but the PC body is this one store),
SetFMNMenuShowingCommonEvents (`FUN_006e0470`(GMM, 0) = GMM+0x2e85 = 0).
Fixed and enabled: DeactivateDamage / ReactivateDamage (`FUN_0068acb0` / `FUN_0068ad50`, identical but for a flag
byte 1 / 0): only with the local slot occupied and the car loaded (`FUN_006ba2f0`), post message 0x1f {vehicle (+0xa0 or
-1), flag} through `FUN_0069a5e0` (`Request::VehicleDamage` now carries both). CleanAndRepairVehicle (`FUN_006926b0`):
the local record's rollout copy (vtable +0x18, 0x2b dwords, two zeroed) applied by `FUN_0068e9d0`(+0x98, copy, 2), then
`FUN_00844940`(vehicle) when +0xa0 != -1 (`Request::CleanAndRepairVehicle { vehicle }`).
New: ClearPendingResets (0x676080: `FUN_0063d410`(G+0x25690, i) for i in 0..16; per slot it clears the loading-screen bit
0x1000 / UIM+0x2d238 when the slot is the local car's and resets the entry: `Request::ClearPendingResets`);
TransferCurrentHeatToVehicle (0x6761c0: the local vehicle id = (record vtable +0x18)+0x30 via `FUN_006ba320`; if non-zero
`FUN_0072a280`(profile db G+0x2f454+0x4964, id, 0) writes the "garage" row's "heat" column = 0; `Request::TransferCurrentHeatToVehicle`);
IsSpeedTrapEvent (0x680530 = `FUN_0067fe30`: online event, else the event type record's byte +0x10 (via
`FUN_0053c020` on the handle (GMM+4)+0xf4) is 0x34 or 0x47, `FUN_006e1720`; `GameModeManager::current_event_kind`);
SetValidProgressionData (0x687750: `lbValid` = 0xce68ae4f from `.bss` 0x1177a6c; `FUN_00686e70` zeroes 11 entries of
G+0x2f3e4, then its +0xa4 = flag and G+0x2f3e8 +0x70 = 0: `Request::SetValidProgressionData`);
SetCollectablesNotInWorld (0x676330: `FUN_0070d260`, `FUN_007000d0` on G+0x14e40); SetRollout (0x68d2f0: `liRolloutId`
= 0xd04318ef from `.bss` 0x1177c78, `FUN_0068bbf0`(G+0x2f404, id): copies the rollout with that id into the local slot's
record and applies it). The `.bss` hashes were read from `re/out/live/snap1/mem.bin` and matched by `field_hash`.
Test `the_fourth_twenty`; `the_twenty_ported_hooks` now counts 80.

Model decisions: no local player = nothing happens (the original dereferences `FUN_006bf330` unchecked);
`current_event_kind` is filled by the runtime (the type record behind the current event handle is not modelled).
Open: what G+0x2f3e4 / G+0x2f3e8 are (SetValidProgressionData); whether the local vehicle id of
TransferCurrentHeatToVehicle (+0x30 of the vtable +0x18 object) equals the garage "vehicleid".
Next candidates (parked, offline or trivial; verify the same way): OnOpenEasyDrive, OnStartEvent, OnSelectEasyDriveOption,
OnOpenEasyDriveMods, OnCloseEasyDriveMods, PlayEndOfIntroSequenceInEasyDrive, ShouldPlayEndOfIntroSequenceInEasyDrive,
ClearPlayEndOfIntroSequenceInEasyDrive, SetHasIntroEasyDriveBeenActive, HasIntroEasyDriveBeenActive,
SwitchToGameplayLoadWorld, LeaveGame, ExitForMultiplayerMatchMaking, ClearE3MultiplayerProgression, ShowPartyInvitePopup,
OnPhotoSharedToWall, ShowX360GamerCard, ShowX360GamerCardEasyDrive, ShowX360GamerCardByPlayerIndex,
SetFMNMenuShowingVITAExclusiveEvents.
### 2026-11-17: HSM boot placeholders ported, the game-mode manager starts free drive (`hsm/behaviours_mode.rs`, `natives/mode_machine.rs`)

13 more behaviour types (58 in all; details decomp.md "The HSM interpreter", third batch): the boot's three
placeholders `PRELOADCOPS` (streams `VEHICLES\VEH_<id>_HI/LO.BNDL` for the player's car, the default cop 0x1df59
and the cop table), `HSB_RNLAYOUT`, `LIST_INVITE` (waits for ever offline), then the free-drive mode's
`REQTRAFRSTRT`, `GETSPEEDWALL`, `PREFETCHSCRE`, `ALLOWSIMPAUS`, `SETSIMPAUSES`, `BPAUSED`, `HSB_RUNNING`,
`ENABLETRIGS`, `HSB_DISCONGP` and `RUNHUDSUBNET` (the same vtable slots as `RUNSUBNET`: one class under two names).
**What `StartGameMode` does** (from `FUN_00701cb0` / `FUN_0070d580`, values read in both snapshots): the event is
G+0x2f6f8, on the boot the free-drive event 390703 that `SetNewGame` picks; the GMM starts the event's `GameMode`
+0x1c instance (390147 "Freedrive Game Mode") once the event data and the mode's `WeaponList` are loaded. Ported
engine-free as `natives::mode_machine` (`start_game_mode` applied by `LogicHost::request`, `update` called by the
`Hsm` owner after `Hsm::update`, queries `NextEvent` / `GameModeStarted` / `GameModeEvent` / `GameModeMachine` /
`FreedriveEvent` answered from `LogicState` by `LogicHost::query`); mw-assets reads `GameMode` +0x1c / +0x20 / +0x40
and `GameLogicConfig` ids (`StartConfig::ids`). `mwtool hsm --run` plays the GMM and reaches "Freedrive Update" with
the "Freedrive HUD" machine at frame 3 (route in decomp.md) with six binding answers on top of the boot's five
(`Mode.HasTransitionedEvents`, `Mode.Data.FeedbackGroupID`, `Mode.WorldLoaded`, `Mode.NeedTheMapScreen`,
`Mode.HudHsm`, `Transitions.Transitioning(NOT)`; values as snapshot 1 reads them): the `Mode` binding root is the
GMM's binding table, not ported.
Model decisions: the driver marks an event available when it is in GAMEPLAY's `EventList` (stands in for
`FUN_006e58a0`'s world list lookup, which picks 390703 as the snapshots show) and loads event data and weapon lists at
once; `HSB_RNLAYOUT`'s slot is named by the layout id in Requests (the original keeps a slot handle); the
`.bss` qword 0x0117ac30 it zeroes is not ported; `HSB_DISCONGP`'s per-frame re-block is a Request every update (the
logic host applies it to `InputBlocks`); online paths of `REQTRAFRSTRT` / the GMM are ported as read but untested.
Open: the runtime host (`mostrusted/src/logic.rs`) should call `mode_machine::apply` in `request`, fall back to
`LogicHost::query`, and call `mode_machine::update` after `Hsm::update`; `FUN_0070b1e0` (end of event),
`FUN_007090f0`, `FUN_0070caa0`; `RUNSUBNET`'s cross-resource load. Next candidate types on the free-roam route (held
as placeholders): `MUTEALLRUMBL`, `PAUSEMSGS`, `PLAYSONGLIST`, `WEAPONSOK`, `MINIMAPICONS`, `SYNCPROFILEW`,
`HIDECHEV`, `PAUSEAI`, `DOBODYSHOP`, `HSMCARSWAPLO`, `DOBLACKLISTS`, `MODEENDCHECK`, `CHECKPTS`, `EVENTSTARTS`,
`HSB_WAIT4IN`, `SET_LAY_SET`, `SETENDREASON`. Captures that would confirm: G+0x2f6f8 and GMM+0x2cfc right after
the boot of a fresh profile; the `PRELOADCOPS` instance's lists (+0x98 / +0xcc) in free roam.

### 2026-11-17: third 20 HSM hooks verified and enabled (60 of 187; `natives/hooks_{flow,event}.rs`)

Handlers read from `re/out/live/text.asm` (vtable-only), helpers with `fn`: RemoveAllCops 0x67a740 (`FUN_006795c0`, per
occupied slot `FUN_006bf190` = kind +0x9c == 2 and (+0x578)+0xb5, then `FUN_006ce180`(slot, 3)), AreAllAIPlayersLoaded,
RequestRespawn / ...ToLastSwapSpot (`FUN_006cd340` / `FUN_006cd3a0`: +0x2c4 = 0, (+0x435 = 1), `FUN_006c9fb0`(0, 0)),
HasLocalPlayerSetRollout, ResetChaseStats (`FUN_0065b830`(pursuit, 1) then heat +0x3d8 / +0x3dc / +0x3e8 = 0),
ResetLocalPlayerHeat, TransferVehicleHeatToPlayer (+0x3dc only), ResetLocalPlayerDamage (`FUN_006ba2a0` then
GMM+0x2e7f = 0 then message 0x16), ClearNitrous (`FUN_0065de30` = thunk to `FUN_00658c90`, arg PM+0xa6b4),
ShouldShowEscapeSequence (`FUN_006e03d0` = GMM+0x2e90, or the mode record (GMM+0x2cf4 / +0x2cf8)+0x18 == config +0x60),
FixUpRecommendationRetry (`FUN_006e04d0`: only when GMM+0x2e90, `FUN_00726ca0`(`last_played` value via `FUN_00732650`)),
SyncTimeOfDay (`FUN_006f4960`), ClearPostEventAwards (`FUN_006e6670` + `FUN_006e28c0`, all 25 offsets match),
RemoveRoadblocks (`FUN_008456d0`), ResetProps (`FUN_00846830`, message 0x15), HACKSuspend / Resume (G+0x2f0c8 = 3.0, `.rdata`
0xf92eb0 = 0x40400000; `FUN_0083c930` group 1 bit 6), SetSeenEventsForVehicleToTrue (`FUN_006e0480`, GMM+0x2e86),
ForceOpenEasyDrive (GMM+0x2ca9). All exit through EXIT except the three bool tests (TRUE 0x1177fe0 / FALSE 0x1177ee0).
All 20 were already right; one fix: AreAllAIPlayersLoaded now needs a vehicle (+0xa0 != -1) as well as `loaded`
(`FUN_006ba2f0` returns false without one). Test `the_third_twenty` (through `Registry::original`).

Model decision: AreAllAIPlayersLoaded in the original walks the records from PM+0x5f50 with a stride of 0x5e0
(the records are 0x7f0 apart), so slots 13..23 read shifted memory; the port reads each slot's own record. Open: confirm
with a probe read whether slots 13+ ever matter (they would only differ with AI racers).
Next candidates (still unported, offline): SetSafehouseFromProfile, ClearHudInstantQueue, StopAllSequences, StopSpeech,
CleanAndRepairVehicle, DeactivateDamage, ReactivateDamage, ApplySelectedVehicleRollout, ClearPendingResets, TransferCurrentHeatToVehicle,
IsSpeedTrapEvent, SetValidProgressionData, SetCollectablesNotInWorld, SetRollout, SkipCurrentTrack, ToggleLoadingScreenOff,
ClearForceOpenEasyDriveFlag, SetFMNMenuShowingCommonEvents, ClearJumpToStoreFlag, ClearCreditsTransition.
### 2026-11-17: Lua natives, fourth batch: the last 15 the UI's chunks read (100 of 385; `natives/lua_garage.rs`)

The intersection of `mwtool natives --area offline --unported` with the native names in `mwtool lua --out` column 2
is now empty: every offline Lua native a shipped UI chunk names is ported. Handlers from the dump, `re/out/live/text.asm`
for 0x67a6b0 (AddNewGarageItem), 0x7e9c30 (ReleaseAllImages), 0x692740, 0x690270, 0x680340 (the dump has no function
there; `fn` does not resolve register `this`, the asm does).
- Rank natives: the `this` of `FUN_006ec470` / `FUN_006ec4f0` / `FUN_006ec5a0` is **G+0x14c08**, the profile manager
  (PPM); the rank list is at PPM+0x94 (count +0x98, (record, offset) pairs, rank at `*(rec+0x28)+0x18+off`, threshold key at
  +0xc). GetMaxRank = max rank (0 empty). GetRankForSP(sp) = `FUN_006ec4f0`(sp - local player record +0x408), where the
  record is `FUN_006bf330` of PM (G+0x2f3dc): rollout `(*(G+0x2f3e4)+0x44) == 4` → the rank with the nearest threshold at
  or below the argument (`FUN_006893c0` = smallest positive matching row, else 0), else 1. GetSPForRank(rank) =
  `FUN_006ec5a0`(rank) + the same +0x408: rank 0 → 1; above the max → `FUN_006ec230` (largest value over the listed ranks);
  else that rank's threshold in mode 4, 0 otherwise.
- AddNewGarageItem(kind, id, flag): `FUN_006fe380` on G+0x14c08 (sic, same object): `FUN_006f8950` validity (item-type tree),
  then a sorted id → byte map gets the flag. HasEarnedAward(awardId, uniqueId): `FUN_007322a0`, query "OfflineAward" on
  the progression database (G+0x2f454 +0x4964), true when a row exists. SelectEasyGuideCategory(id): `FUN_00792fc0` on
  UIM+0x47640 (find the category with that id among the current page's children, push the current page, descend).
  SetFocusVehicleId: `FUN_0086bbf0`(id, 0) on OM+0x4c90. SetLocalPlayerLicenseRegistration: `FUN_00679760` → `FUN_006ee6f0`
  (8 chars into the local player's 0x1c-byte record, +4). SetRolloutFromProfile / SetSinglePlayerRollout: the same
  requests as their hooks. SetVehicleAndRolloutFromSaveGameByVehicleID (Lua 0x680340 = `FUN_006800a0`, > 0 only). Requests
  (engine-only): ApplyModUpgrade (`FUN_0068ab10`, also sets GMM+0x251c), ReleaseAllImages (`FUN_007e1340`: image
  slots in state 2 → 3), ResetNeutalVaues (flag 8 of `*0x010edb18`, message {8,0} queued), SetAvailableVehicleListEvent
  (`FUN_00871330` on OM+0x5ee0), SetVehicleAndRolloutFromSaveGame. Tests in `lua_garage.rs`; `mwtool lua`: 115 scripts run, 0 errors.

Model decisions: the rank table (`GarageFields::ranks`, thresholds precomputed from `FUN_006893c0`'s data lookups), `sp_base`,
`rank_mode`, the garage validity set, earned awards and the guide's per-page `children` are filled by the runtime, not read
from memory. GetRankForSP with no threshold at or below the argument: the original dereferences a null record; we return 1.
Open: filling those fields from the install (the rank/threshold data, OfflineAward rows); the engine side of the five new requests.

Next candidates (offline Lua natives still unported, 163 minus these; none is named by a UI chunk, so they are reached
from the HSM behaviours/hooks, GeneSys sequences or the in-race feedback/event scripts, which this session did not
trace; by family): camera (TransitionCameraTo*, Get/SetCameraFov, ICE camera, overhead / finish-line camera), feedback
messages and sequences (ShowFeedbackMessage, StartFeedbackSequence, Load/UnloadFeedbackGroup, loading-screen natives),
sound (PlaySound, PlayMusic, PlaySpeech, mix snapshots), race/competitor queries (GetCompetitor*, GetRaceDistance,
GetDistanceToFinish, IsInTrigger*, GetTrigger*), vehicle control and queries (SpawnVehicle, TeleportVehicle,
SetCarLocked/Frozen, GetVehicle*, speed getters), traffic (StopTraffic, SetTrafficDensity), nitrous, and the rollout /
route ones (SetEventFromAvailableVehicles, SelectDefaultRollout, SetRolloutFromVehicleID, SetRouteFromPlayerTo*,
SetSelectedWorld, GetSelectedVehicleId / ColourId, IsAssetUnlocked, SaveControllerConfiguration). The rollout/route and
selected-id ones are the cheapest next (state fields like this batch); the camera / sound / traffic / vehicle ones are
mostly engine requests plus a few getters that need runtime-filled fields.

### 2026-11-17: Lua natives, third batch: 15 more (85 of 385; `natives/lua_misc.rs`)

Each read from its handler (dump; `re/out/live/text.asm` for the six the dump lacks: ReleaseSettings 0x7eb170,
PrepareControlSettings 0x7eb140, GoBackEasyGuideCategory 0x675910, Enable/DisableDreamshotsCameraControlGroup
0x7ea120 / 0x7ea150, StartViewingEasyDrivePages 0x680300). Natives: UpdateDeferredViewModel (0x7f96a0, FUN_007f72f0),
ClearViewModel (FUN_0081ed30 / FUN_0081ec00: the entry swapped with the last, downloads of that name removed or
their +2 cleared), GetCurrentFreedriveEvent (FUN_0068fa10, shared with the hook), GetNextEventForProgression
(FUN_0068fba0: other-mode events that are available, listed at G+0x29204 and pass the rollout check; pass 1 skips
`EventInfo::completed` = FUN_006ec6a0, pass 2 takes any, else the free-drive pick), StartActionBind (FUN_00878790),
SetGameMode / SetDistrict (read their argument and do nothing), Prepare/ReleaseControlSettings (FUN_00879010 /
FUN_00879c70), GoBackEasyGuideCategory (FUN_00790a60), StartViewingEasyDrivePages (+FUN_007997f0), Enable/Disable
DreamshotsCameraControlGroup (bit 4 of group 0 of the input-block set G+0x2f0a4: FUN_0083d030 clears, FUN_0083d000
sets, each counting at +0x2f0c0; `InputBlocks::set` added), SetLocalPlayerLicensePlate (FUN_0067fa20),
ChangeVehicleOnCustomRollout (FUN_00691110). Tests in `lua_misc.rs`; `Download` records now carry name and flags.

Model decisions: UpdateDeferredViewModel pushes `MiscFields::deferred_ids[name]` (else -1): the original's out
value (set by FUN_0081d1f0 inside FUN_00820550) was not traced; the load is `Request::UpdateDeferredViewModel`.
SetLocalPlayerLicensePlate's telemetry event (FUN_004c3890 "GAME EVNT LICE") is not ported. The `this` of
FUN_006e4d10 / FUN_006ee760 (plate list) is not identified; plates are filled by the runtime.
Open (closed in the fourth batch): GetRankForSP's `this` is G+0x14c08.
### 2026-11-17: 11 more HSM behaviour types, the boot needs no behaviour answers (`hsm/behaviours_game.rs`)

The 6 types removed from the first batch were verified against `text.asm` (factories are not in the dump: vtable
from the factory's `mov [esi],vtable`, slots from `.rdata`, bodies from `text.asm` plus `fn` for callees) and
ported: DBAUTOSELWD, SELECT_PROF, WAIT4FEEDBK, WAITFORVEHIC, ISNEWPROFILE, ISAUTOTESTIN, plus HSB_STRSCRPT (only
slot 1 own, returns 1). Then the unported types the boot still ran as placeholders: REBOOT_REQ, ENABLETIME,
COLLECTABLES, LOAD_FDBK_GR (7 + 4 = 11 ported in all). Details and addresses: decomp.md "The HSM
interpreter" (second batch). Findings: SELECT_PROF names the profile `%llu` of the persona id (`FUN_00428860` into
`DAT_010ecc10`) and posts message 0xc on exit; the `.bss` parameter keys read from snapshot 1 are `GROUP`,
`GROUPBINDING` (shared by WAIT4FEEDBK and LOAD_FDBK_GR) and `UNLOADONEXIT`; the earlier WIP's readings were all
confirmed (no corrections). Name stubs (`FUN_00779e10` etc.) return (hash, factory) in edx: that is how a name
maps to its factory.
Request: AutoSelectWorld, SetProfileUser, ProfileUserSelected, StartCollectables, StopCollectables,
LoadFeedbackGroup, UnloadFeedbackGroup, AdjustTimeEnabled. Query (default): AutoSelectableWorld (true), UserId (1:
model decision, offline has no persona), FeedbackGroupLoaded (true), VehicleListLoaded (true), NewProfile (true),
RebootRequested (false), CollectablesReady (true). Tests in `hsm/flow_tests.rs`, one per type.
`mwtool hsm --run` reaches "Playing" with 5 answers instead of 11; each of the 5 is needed: the BOOT trigger,
`UIConfig.Platform`, `Net.IsLoggedInToServer`, `SignInState.Params.IsChatRestricted` (engine bindings / online
services) and state 1222788 `ConnectToAutolog` (online Autolog sign-in; its subtree "PC Login" waits for a
sign-in screen). None is a behaviour answer. Still placeholders on the boot path:
PRELOADCOPS (0x744c50 / 0x75c1c0 / 0x758030, resource streaming), HSB_RNLAYOUT (0x77f5f0, 7 slots), LIST_INVITE
(0x74dd80, online invites) are the next candidates. ENABLETIME's counter meaning (G+0x25624) is inferred from the
name.

### 2026-11-17: second 20 HSM hooks verified and enabled (`natives/hooks_{flow,event,ui}.rs`)

Handlers read from `re/out/live/text.asm` (TRUE = `DAT_01177fe0`, FALSE = `DAT_01177ee0`, EXIT = `DAT_01177ef0`).
All 20 matched the earlier port, no Rust fix needed; they moved from `register_parked` to `register` (now 40 of 187
hooks). Verified: SetFreedriveEventToPlay (0x690310: `FUN_0068fa10` pick, then `FUN_0063e380`(G+0x2f4c8, id, 0)
= pending +0x230 / aux +0x234 = -1 / flag +0x238), SetIntroLoadComplete (GMM+0x2e82), ShouldShowSafehouseExitSequence
(`FUN_006e0520` is `xor al,al; ret`, so TRUE), ShutdownGame (`DAT_011a2470`), SetStoryMode / SetGameModeWon /
SetAllowLoadingScreen / SetRetryAtEnd (bool member tag 5 from `StoryMode` / `Won` / `lbEnable` / `Retry` into
GMM+0x2e7a / 0x2e78 / 0x2e81 / 0x2ead), IsChallengeActive (GMM+0x2ce0 ≠ 0), RetryLastEvent (`FUN_00732650`(PPM, 1)
= `last_played`[1], then `FUN_006e74f0`(id, 0)), StartEventFromEasyDrive (`lpcValue` tag 2, strtol base 10, skip on
-1), RequestRespawnAtEventStart (`FUN_006cd380`: player +0x2c4 = G+0x2f6f8, `FUN_006c9fb0`(0, 0)),
AwardPostEventAwards (`FUN_006e65e0`: amount GMM+0x2e70 > 0 → `FUN_00736a90`(+0x2ce8, event, amount); each award
+0x38a0[..+0x38b4]: `FUN_00736ab0` if `FUN_0071f7e0` else `FUN_006792b0`), ResetCurrentEventStats (`FUN_0067f700`:
`FUN_006600d0`(event +0x2ce0, mode (+0x2cf4/+0x2cf8)+0x18, online)), RemoveAllRaceCars (`FUN_006d1310`: vtable +0x20 of
all 24 slot pointers, `FUN_006ce180`(slot, 3) for occupied 12..23, PM+0xa660 / 0xa694 / 0xa698 = 0),
ClearHudMessageQueues (`FUN_0067f8c0`: UIM queues +0x7230 / +0xa6f0, records +0x12458 (stride 0x7c) and +0x12468 /
+0x124f8 / +0x12574 / +0x126e8). Engine-only bodies (a `Request`, the hook itself just calls with no argument and
exits EXIT): SetEventFromAvailableVehicles (`FUN_00673d70`), SetTimeOfDayFromEvent (`FUN_0067f570`),
PreparePostEventAwards (`FUN_006fbb60`, 330 lines), AddAllAIPlayers (`FUN_00686f30`).
Model decision: the runtime must fill `LogicState` where the `Request` doc says so (PreparePostEventAwards writes
GMM+0x2eb4 / 0x2e70 / 0x2ce8 / 0x38a0 awards, which `AwardPostEventAwards` reads); until it does, those stay at
their defaults. Test: `hooks_tests.rs::the_second_twenty` (through `Registry::original`).
Next candidates (still parked, same files): RemoveAllCops, AreAllAIPlayersLoaded, RequestRespawn,
RequestRespawnToLastSwapSpot, ResetChaseStats, ResetLocalPlayerHeat, TransferVehicleHeatToPlayer,
HasLocalPlayerSetRollout, ResetLocalPlayerDamage, ClearNitrous, ShouldShowEscapeSequence, FixUpRecommendationRetry,
SyncTimeOfDay, ClearPostEventAwards, RemoveRoadblocks, ResetProps, HACKSuspend/ResumeCarControlForIntro.
### 2026-11-17: 15 HSM behaviour types of the boot flow (`hsm/behaviours_game.rs`, `behaviours_ui.rs`)

The stopped agent's WIP (22 types, built from `fn` and `text.asm`) was checked against the dump function by function (factory → vtable → each method; the C bodies of 0x76f7f0 / 0x76eac0 / 0x76eb50 / 0x76e4e0 / 0x76e5e0 / 0x73e730 / 0x744810 / 0x7448e0 / 0x7450e0 / 0x73ee50 / 0x73eed0 / 0x75c870 / 0x75e6f0 / 0x73d130 / 0x73d1d0 / 0x73d3d0 / 0x73e220 / 0x73b980; the vtable-only ones 0x76ffd0, 0x73ddf0, 0x73da30 / 0x73da60 / 0x744040, 0x73e7e0 / 0x73e930, 0x740f00, 0x745b10, 0x73b9e0, 0x73e2a0 from the WIP's reading of `text.asm`, not re-read). Ported (15): RUN_PG_RNNR2, ACTI_WDGT_GR, HSB_P1BIND, SETBLACKBACK, CHANGEVSYNCS, MUTE_SOUND, LOADWORLD, LOADPROFILE, IN_ON_GAME, RUNGAMEMODE, GAMEMODE (partial: DISCONNECT only), RUNGAMELOGIC, LOAD_ALL, BINGAME, HSM_PLAYING. Tests: `hsm/flow_tests.rs`, one or two per type. Request variants (`natives/state.rs`): PushPageContext, LeavePageContext, RestoreFocusGroup, ActivateWidgetGroup, PlayWidgetGroupTransition, DeactivateWidgetGroup, ClearWidgetExit, SetBlackBackground, SetFullRate, MuteSound, LoadWorld, LoadProfile, ApplyProfileSettings, ReleasePrimaryUser, ListenForPrimaryUser, RemovePlayers, CreateLocalPlayer, StartGameMode, StopGameMode, PlaySequence, StopPlayback (the WIP's `StopSequence` clashed with the Lua native's), LoadAll, EnterGame, LeaveGame, StartPlaying, StopPlaying, SessionLost; `Host::request` / `Host::query` with `behaviours::Query` (defaults named per variant; `mwtool hsm --run --answer Query.<Name>=<v>` overrides, `PrimaryUserBound` follows the driver's press).
Removed, not verified in this pass (next candidates, code is in the git history of branch worktree-agent-ad67203ead166fb17): DBAUTOSELWD 0x7702e0, SELECT_PROF (0x73f270 / 0x75cd00), WAIT4FEEDBK 0x746a50, WAITFORVEHIC 0x741b70, ISNEWPROFILE 0x73d350, ISAUTOTESTIN 0x76e8a0, HSB_STRSCRPT (all slots base).
Model decisions: engine waits default to "ready" (a missing engine never blocks); GAMEMODE's ActiveGameMode upkeep, Autolog refreshes and the GAMEEVNTSTRT / GAMESESSSTRT telemetry are not ported; the playback ids of `PlaySequence` are a process counter (the original's handle comes from the sequence player). `mwtool hsm --run` reaches "Playing" with the old answer set and with HSB_P1BIND, LOAD_ALL, LOADPROFILE, LOADWORLD no longer answered (docs/commands.md); the remaining unported behaviours of that run: SELECT_PROF, DBAUTOSELWD, WAIT4FEEDBK, WAITFORVEHIC, ISNEWPROFILE, ISAUTOTESTIN.
### 2026-11-17: Lua natives verified against their handlers (60), 10 more ported (`natives/lua_menus.rs`)

Method: each handler read from the dump (`fn <addr>`) or, where the dump lacks it (15 of them), `re/out/live/text.asm`; argument conversion, pushed values, state offsets and callee compared. Verdicts: **58 verified, 2 fixed, 0 unregistered**.
Fixed: `ChangePostFXSetting` (0x007e9f80) reads the key with `fistp` at the default rounding (`ROUND`), not truncation (delta stays `_ftol2`); `StartTextEntry` (0x007f2380) reads the 6th argument (flag bit) the same way. New `lua_args::round_int`. Comment fix: `SetOfflineEventToPlay`'s `FUN_0063e380` runs on the object at G+0x2f4c8 (fields +0x230 / +0x234 / +0x238), not G.
Verified (field, offset and conversion match): SetOfflineEventToPlay, SetTargetOfflineEvent, SetStoryMode (GMM+0x2e7a), GetChallengeVehicleSet, SelectDrivingTestListGroup, SetRequestedEasyGuideContent, StopViewingEasyDrivePages (UIM+0x47698 byte, +0x47730), GetCurrentSafeHouse, SetSelectedVehicleId / SetSafeHouse / SetTargetRouteFromSafehouse / ChangePerkOnCustomRollout (argument read, nothing else), print, the five Resolve*Binding (FUN_0067f4e0 / 6794e0 / 679550 / 7e7cc0 / 7ecac0), ExitWidgetGroup (FUN_007eef40: the `{...}` text = chars 1..len-2), the Set{Bool,Int,Double,String,UniqueId}Variable and DeleteVariable (optional prefix when more than 3 / 2 arguments; 64-byte path buffer), SetModalMode, GetPlatformName ("PC"), SmoothStep (x87 maths and clamp, NaN passes through), SetLanguage / GetInitialLanguageSelectLanguage (maps are inverse), Create/Destroy/Start/Stop conductor and sequence natives, HasSequenceFinished, IsConductorIdle, GetTextEntryState (+0x48), GetEnteredText (+0x38fc), RequestDialog, GetDialogResult (-1 when none, entry removed), ClearDialog, DoRpc, all settings natives (category by id, vtable slots 2..12 as in lua_settings.rs; Min = slot 7, Max = slot 6; unsigned pushes), SaveSettings / RevertSettings / RevertCategorySettings, RegisterPostFXSetting (record layout and key 3 start value), ResetPostFXSettings.
Model decisions found while verifying (marked in the docs, not wrong): `StartSequence` / `HasSequenceFinished` do not parse the sequence binding (the original does nothing / answers true when it does not parse); `RequestDialog` opens the id without looking for a definition; `StartTextEntry` does not clear the keyboard object's +0x44 (the runtime owns +0x48); `SaveSettings` skips the category's slot 5 (after-save hook); `IsSettingValid` asks "does the index name a setting" instead of the category's slot 2; unknown conductor / category ids dereference null in the original and are idle / 0 here.
New (10, all offline, `natives/lua_menus.rs`, state `LogicState.menus`): SetNewGame (0x006902a0, the hook's `hooks_flow::set_new_game`), SelectChallengeVehicleSet (0x00679cd0: G+0x2f418+0x334, `Request::RecountChallengeVehicles` = FUN_006f3f90), SelectVehicleCategory (0x006902d0: G+0x2f404+0x1bc, `RebuildVehicleList` = FUN_0068b7c0), SetSelectedVehicleColour (0x00679ff0 -> FUN_00657b50), SetVehicleModifier (0x00679f20 -> FUN_00657d10), SignInStarted (0x007eac10 -> FUN_00881f10(1): OM+0x10d0 fields +0x158a/+0x1588 = 1, +0x135c = 0, +0x1360 = -1, +0x139c/+0x136d = 0), SignInFinished (0x007eac40 -> FUN_00886f00: both fetched lists emptied, +0x158a/+0x1588 = 0), SelectPack (0x007ea480 -> FUN_008739f0/FUN_00642a90, +0xdce = 1; index past the packs selects none, model), IsViewModelDownloadComplete (0x007ee5d0 -> FUN_0081d1b0: false only while a download record with that id exists; empty offline), UpdateAwardProgressBindings (0x007e9f40 -> FUN_00724680 / FUN_00724690 = FUN_0071fd20(-1 / 6)). SignInFinished is now ported, so `ViewLegal.OnCancel` completes with nothing unported (the end-to-end test asserts `unported.is_empty()`). `mwtool natives`: 70 / 385 Lua natives. Not ported from the list: UpdateCachedMostWantedList (online: notification centre request), GetCurrentFreedriveEvent (FUN_0068fa10 walks the world's event tables), UpdateDeferredViewModel (FUN_007f72f0 searches the screen's view-model definitions). Next candidates: those three, the camera / feedback / sound natives of `LUA` (Offline area), the competitor getters (GetCompetitor*).

### 2026-11-17: 20 HSM hooks ported (`natives/hooks_flow.rs`, `hooks_event.rs`)

Handlers are not in `functions.csv` (vtable-only); read from `re/out/live/text.asm` (awk from the address to the next int3) with the helpers' bodies from the dump. Ported (all offline): CalculateShouldShowRecapSequence (GMM+0x2e88 := 1), ClearRecapFlag, ClearMatchmakingState (FUN_00675470), PopulateSettings / SaveSettings (FUN_0086e0a0 / FUN_0086fe20 on the settings at +0x2f3e0 of the object behind 0x117e9f4+0x2020, not G), SetNewGame (FUN_0068fd50), SetSinglePlayerRollout, SetRolloutFromProfile, IsNewGame, ClearNewGameFlag, ShouldPlayNewGameIntro, SetNewGameIntroCompleted, SetIntroEventNext, IsIntroRace, IsProgressionReady, SaveProfile, StartGameModeTimer, StopGameModeTimer, ResetGameMode, IsOnlineEvent. Each returns EXIT (0x1177ef0) or TRUE / FALSE (0x1177fe0 / 0x1177ee0). State: LogicState.game / config / game_modes / players / progression (offsets in field docs); engine work as Request variants. About 70 further hooks from a stopped pass are in register_parked (unverified, not in Registry::original(); original_with_parked() for tests). Model decisions: missing / mistyped arguments read as zero; ResetGameMode sends no telemetry record. mwtool hsm --run stands in a loaded profile, so IsProgressionReady / ShouldPlayNewGameIntro are no longer answered by hand; the boot trace is identical. Open: verify the parked hooks one by one.
### 2026-11-17: Lua natives, first 60 ported (M8 step 3; `natives/lua_{args,widgets,settings,career}.rs`)

Merged the stopped agent's WIP onto the wired registry (`Registry::set_lua`, `LogicHost::install_lua`) and
verified it. Sources: each handler in the dump (addresses in the doc comments), Lua 5.1 `lapi.c` as linked in the
exe (`lua_args.rs`: `tolstring` NULL gives `"<invalid>"`, `tonumber` + `_ftol2` for ints). Ported (60 of 385, mwtool
natives): bindings (`Resolve{Bool,Int,Double,String,WideString}Binding`), `ExitWidgetGroup`, `Set{Bool,Int,Double,
String,UniqueId}Variable`, `DeleteVariable`, `DoRpc`, `SetModalMode`, `print`, `GetPlatformName` (`"PC"`),
`SmoothStep`, `SetLanguage`, `GetInitialLanguageSelectLanguage`, conductors / sequences (6), dialogs (3), text
entry (3), settings (14 + 3 post-FX), career fields (12). State: `LogicState::{screen, widget_scripts, text_entry,
settings, post_fx, game, game_modes, ui_manager, challenges}`; engine-only effects are `Request::{WidgetRpc,
SetLanguage, CreateConductor, DestroyConductor, StartSequence, StopSequence, ShowDialog, CloseDialog,
StartTextEntry, SaveSettings}`.
- **Tests**: `natives/lua_tests.rs` (a test per area, every native called as a Lua global); `tests/lua_natives.rs`
  (shipped chunks `ViewLegal`, `Breadcrumb` through `LogicHost`). `ViewLegal.OnCancel` calls the unported
  `SignInFinished` then `ExitWidgetGroup("CANCEL")`; `ViewLegal.OnAction` without a configured binding errors in
  the script on a chunk-level nil local (context, not a native).
- **Model decisions**: `SetUniqueIdVariable` stores an int (the typed JSON has no tag 7); the script variables
  are republished as the single root `Widget` (one member per widget; the bindings' root is one name hash, the
  original resolves `Widget.<w>.<..>` through slot 8 of the script objects); `GetCurrentSafeHouse` answers 0
  and the `SetSafeHouse`-like natives only read their argument until the career state exists.
- **Open**: `SignInFinished` / `SignInStarted` (online, trivial), the profile / career natives that rebuild lists.

### 2026-11-17: native registries wired (PLAN M8 step 3, `mw_logic::natives`)

- `Registry::original()` lists the 187 HSM hooks (`FUN_007763b0`) and 385 Lua natives (`FUN_009f7e50`) with handler
  addresses; ported ones carry a fn, the rest record `"hook Name"` / `"lua Name"` in `LogicState::unported`.
  `Area` classification of the entries is still `unknown` (later).
- `LogicHost` (one `Registry` + `LogicState`) implements `hsm::Host` (hooks through `Registry::call_hook`, binding
  reads and publish / retract through `state.view_models`) and `install_lua` binds every Lua native as a global.
  `ViewModels` = published data-definition roots plus runtime-registered root providers.
- HSM data definitions are read with `mw_formats::json::Json` (engine reader, key order, int / double tags).
- `mwtool natives [--area a] [--unported]`; `mwtool hsm --run` and `mwtool lua` run on the logic host. Results
  unchanged: boot reaches "Playing" with the answer set, 115 Lua scripts run clean.

### 2026-11-17: browser audio: it plays, matches native's level, drops out while streaming
- **Start**: browsers start an `AudioContext` suspended until a gesture; the game (cpal via Bevy/rodio) creates
  its context while starting. `tools/web/index.html` keeps every context and resumes them on the first
  click / key / touch (picking the install folder counts). Headless: `--autoplay-policy=no-user-gesture-required`.
- **Measurement** (no listening): the page's `?record_audio=<s>` routes everything cpal connects to the
  speakers through a tap into a `MediaRecorder` (`audio/webm;codecs=pcm`, lossless, off the main thread) and
  POSTs it to `serve.py` (`/__audio`); `tools/web/headless <name> '<flags>' <timeout> <seconds>` runs it.
  Native reference: the same drive in `tools/hl env MW_MUTE=0 ...` (null sink only) recorded with `parecord` from
  the sink's monitor. Analysis: RMS per 0.1 s, exact-zero runs (dropouts), clicks, spectral centroid per second.
- **Result** (`--drive 911 --units 12 --auto-throttle 0.6`, 34 s after the car is placed): level and pitch
  match: median RMS 0.17 browser vs 0.059 native = the user's saved master 0.35 (the browser has no settings
  file: defaults, master 1.0); centroid 1.4–2.4 kHz in both. **Dropouts**: 42 (913 ms, longest 328 ms, 13
  clicks) in the browser, 0 natively, clustered in the first ~8 s after placement and around 32 s, i.e. while
  units stream in: cpal's WebAudio host refills two 2048-frame buffers (~93 ms) from main-thread timeouts, so
  any main-thread stall over ~46 ms (GPU uploads, mesh / image preparation, a main-thread game file read) gaps.
- **Main-thread file reads** (`WebSource` now warns on reads ≥ 16 ms): the startup index is 4,040 small reads
  (~4.4 s total, the text path); while driving only GLOBALEFFECTS (91 ms), ENVIRONMENT (102 ms) and a feedback
  group (41 ms): not the main cause of the gaps.
- **Fixed the same day** (user: patch both): `third_party/` carries rodio 0.22.2 and bevy_audio 0.19.1 through
  `[patch.crates-io]` (third_party/README.md): bevy_audio opens its sink on cpal's AudioWorklet host on wasm32 with
  atomics (falling back to WebAudio), the sink wrapped in `send_wrapper` (the worklet stream holds the page's
  `AudioContext`, not `Send`; Bevy's systems run on the main thread there); rodio's 32 lock sites on the callback
  path go through `lock_spin` (spins on `try_lock` in the threaded browser build, the ordinary lock natively).
  Same drive, 31 s after placement: **0 dropouts, 0 clicks** (was 42 / 913 ms), median RMS 0.168 (was 0.17);
  log line "audio: AudioWorklet output". Native unchanged: median RMS 0.0590 (0.0586 before), 0 dropouts;
  243 tests, accel and race tests as before. Original plan, for the record: take the mixer off the main thread with cpal's `audioworklet` host (the callback
  runs on the audio rendering thread over shared memory). Bevy's `AudioOutput` is crate-private and opens
  cpal's *default* host (always WebAudio), so this needs a patched `bevy_audio` (or our own output plugin), and
  rodio's `Player` takes plain `Mutex::lock`s on its controls (volume / speed / seek / position) every few
  samples while the main thread sets them: a contended lock traps on the worklet thread (no `Atomics.wait`
  there) as on the main thread; those have to become try-locks or atomics. Bigger buffers alone would trade the
  gaps for ~0.4 s of latency.
- Headless setup: `brave --headless=new` with `--use-angle=vulkan --disable-vulkan-surface` presented frames
  only with a profile left by an earlier run; with fresh profiles it gets no WebGPU adapter, and without
  `--disable-vulkan-surface` the swap chain's shared images fail (device lost). Brave inside `tools/hl`'s
  headless gamescope (X11, `--enable-features=Vulkan`) works from a fresh profile: `tools/web/headless`.
  Killing the launcher leaves Brave's processes running (they restored old tabs into later runs): the runner
  kills by profile.

### 2026-11-17: Binding evaluator (M8 step 1, `mw_logic::binding`, `mwtool bindings`)
The binding language's parser and evaluator read from the code (decomp.md "Binding evaluator": functions,
visitor vtables, constants); ported as `mw_logic::binding` (engine-free, wasm-checked): `Binding::parse`
(`FUN_00a50650`, HSM / Lua / most UI) and `parse_rooted` (`FUN_00a503e0`), `compile` with `{binding}` splices,
`eval_bool` / `eval_int` / `eval_float` / `eval_double` / `eval_string` / `eval_text` over a `Host` (or `Roots`:
root name → `Provider`). Sources: the dump (parsers, arithmetic, comparisons, LERP) and the snapshot's
disassembly `re/out/live/text.asm` for the vtable-only visitor methods; REA could not run (installed Ghidra 12.2,
the `rea` server accepts 12.1.x only).
- **Found in the code, not in the data**: 22 operators (ADD MINUS MULT DIV POW FLOOR_TO CEIL_TO ROUND_TO AXES MOD
  MIN MAX EQUALS GTEQ LTEQ BETWEEN HASHEQUALS NOT UPPERCASE POSTFIX IS_EMPTY LERP); LERP / BETWEEN take their
  arguments from the following op records by position (`MINVALUE` etc. are not in the exe); indices are 1-based
  (none = 1), `["key"]` hashes; the operators that apply depend on the *result type* (bool: comparisons, NOT /
  IS_EMPTY once; float / double: LERP then arithmetic; int: arithmetic; string: UPPERCASE / POSTFIX only); a
  missing value is false and only IS_EMPTY inverts it; the bool of an int is its *original* value ≠ 0 (the
  arithmetic only feeds comparisons); EQUALS on a string compares hash(upper(first 23 bytes)); op tokens are
  upper-cased and at most 23 bytes; from the first `(` on the text is one segment. `~` in a widget path means
  *absolute* (`FUN_007cb300`), not "relative to the list item" as `mw_formats::binding` assumed.
- **Verification**: `mwtool bindings` collects 5,024 distinct binding texts (JSON 2,544, GeneSys 3,503 string
  fields, HSM parameters 263, Lua constants 239; 156 whole-value `{binding}`s, 37 Lua concatenation fragments
  skipped): **all parse**. 10 unit tests on the operators against the semantics above (hashes checked against
  the AXES table in `.bss`). The warnings are data quirks the original has too: `Signals.GetRandomNumber[100]
  (LTEQ,50)` (GeneSys `EnabledBinding`: LTEQ without `:` compares with 0), `Players.LocalPlayer.GetTeam(EQUALS, 0)`
  / `(EQUALS, 1)` (both compare with 0), `LocalPlayer.TimeTilEvaded(BETWEEN(10, 19))` and
  `...IsRankingUp(IS_EMPTY(NOT))` (nested parens: no operator), `(CAPS)` ×6 (not an operator or formatter: a
  no-op), `MAINRANGE` ×4 (typo, harmless: positional).
- **Model decisions**: op records past the count read 0 (the original reads stale memory); i64 DIV / MOD by 0
  leave the value (the original faults); nesting of `{binding}` string values capped at 8; the int result of a
  string or object is 0 (the original leaves the caller's initial value); `Value::List(n)` answers `_length` in
  `Roots` (the original registers `_length` members on its list types, static initialisers at 0x00f593xx–
  0x00f797xx); the 64-bit fixed-point time type (slot 10) and vectors (only `axes()`) are not values.
- **Open**: the text widget's formatters (`SPEED`, `TIME`, `DIST`, `WITHORDINAL`, ... `.rdata` 0x00f9e484, vtable
  0x00fd82f8 first half, 0x0046xxxx) still live in `mw_formats::binding` (`hud.rs`); the root registry's
  contents (which roots are static types vs dynamic providers, the `Signals` functions); `%name%` widget
  variables (`FUN_007c83f0`); binding splices of JSON adapters (`FUN_00a4f410`, `BindingSplices`); which of the
  7 rooted-form call sites serve which data; `EnabledBindingEvaluationMode` (`Constantly` / `On Entry`, 13 JSON
  widgets) is widget-level. `mw_formats::binding` (used by `hud.rs`) differs from the original in `~`, NOT /
  IS_EMPTY counting, named LERP arguments and string EQUALS; move `hud.rs` onto `mw_logic::binding`.
### 2026-11-17: game logic: the HSM interpreter (PLAN M8 step 2)
Worktree agent. The HSMData parser completed and the original's interpreter ported (decomp.md "The HSM
interpreter", formats.md "HSMData").
- **Parser** (`mw_formats::hsm`): all official structures (burnout.wiki names): top-level states with instance
  ids, entry points with spawn entry ids, child exit points with parent exit points, typed parameters incl.
  nested STRUCTUREs, data definitions (path + JSON text), behaviour interfaces (entry / exit point ids,
  parameter defaults), fallback byte. All 37 HSMData of the install parse (`mwtool hsm --bundle all`).
- **Found**: parameter ids are not always canonical packed names: 484 `BINDING` parameters (and the exe's
  constant for EXITBINDTRUE / HSB_SEL_EX_B / HSM_B_TRUE / HSB_B_NOT_0 / HSB_EX_BI_CH) are "BINDING" NUL "PATH"
  (0x54a48941b5db2e54), while BINDINGTIMED / RUNSUBNET use the plain `BINDING`; lookups compare all 64 bits, so
  each behaviour uses its exact id. Hooks return the exit point; a missing hook gives `EXITINVALID` (also every
  instance's initial exit point). The fallback byte only matters for unregistered ids (RUN_CHILDREN →
  HSB_RUNCHILD, else HSB_PLCHLDR = exit through EXIT); a top-level state always falls back to HSB_PLCHLDR.
  Children are not ticked while their parent is entering; instant states chain within one frame (later passes
  tick the children started in the same call). The boot's instance id 20527 is G+0x2f710 (live). 5 top-level
  states name states GAMEMODES lacks (RUNSUBNET's enter requests a load when the state is missing: other HSM
  resources may hold them; not seen in the install).
- **Interpreter** (`mw_logic::hsm`): 12 machine slots, results ring, the 4-phase tick, children passes,
  transitions, entry points, subtree exit requests, data definitions published / retracted as view models, hook
  arguments from the JSON (`serde_json`). Behaviour types are a registry of factories by behaviour id; 19 ported
  (list in decomp.md); `Host` trait for bindings (typed bool / text / int), hooks, view models, graphics /
  loading-screen queries; `Clock` with the two clocks the behaviours read. 10 lifecycle tests.
- **Verified** (`mwtool hsm --run`, answers in docs/commands.md): the boot runs Top State → Boot ("Wait For
  Graphics Prepare") + Run Always Active Systems + Enable Black Background + the boot page (Language Select: the
  PC locale check exits at once → Press Start: widget group 491032, "Exit When Intro Sequence Binding Is True" →
  Press Start) + Switch to 60Hz → Post Controller Bind Boot (Autoselect and Load World: DEBUG Autoselect World →
  Load World via ENTRY; Do Profile Load: Start Loading All Data → Select Profile → Load Profile → PopulateSettings;
  Platform Selector PC → Get Online Silently → ConnectToAutolog) → InGame (Wait For Required Resources: feedback
  group 97039, vehicle list, IsProgressionReady, CalculateShouldShowRecapSequence → Setup Start Point: Is Brand
  New Profile YES → SetNewGame, SetSinglePlayerRollout → TOGAME → Should Play New Game Intro FALSE) → "Playing"
  (Wait For Message Groups, Load Message Groups, Preload Cop Cars, Switch to 30Hz mode, Listen for Invite Flow).
  Same order as decomp.md "Boot, loading screens and the free-roam start".
- **Model decisions**: missing child exit records / entry points are reported (`Trace::Missing`) and ignored
  (the original dereferences null); 10,000 passes per children update (the original has no guard); a binding
  counts as parsed when its path is non-empty (until agent A's evaluator is connected); `WAIT4TIME`'s
  `CurrentHSMTimer` view model is not published; the driver holds unported behaviours and answers unknown hooks
  with EXIT (driver only).
- **Open**: the ~180 used behaviour types left (RUN_PG_RNNR2 / ACTI_WDGT_GR / HSB_P1BIND / RUNGAMEMODE /
  GAMEMODE / RUNGAMELOGIC / LOADWORLD / LOADPROFILE / MUTE_SOUND / SETBLACKBACK / CHANGEVSYNCS first); which
  engine clocks +0x18 / +0x38 of the timer manager are; the byte at UI manager +0x2d200; the data-definition
  `adapters` member (hash 0xb21d9989) and binding writes into published view models; connecting the binding
  evaluator (typed reads `FUN_00a4f160` bool, `FUN_00a4f2e0` text, `FUN_00a4f200` int, the parse context
  machine +0xe8) and the native registry for hooks. A probe hook on `FUN_00776470` logging (state id, phase) in
  the original would confirm the boot path frame by frame.
### 2026-11-17: Lua 5.1 VM (`mw-lua`): the UI's 115 chunks run (PLAN M8 step 5)
Worktree agent. C = confirmed (dump / data / differential test), I = inferred.
- **Decision: our own interpreter, not `rilua`.** rilua 0.1.27 (MIT / Apache, single author, the WoW-addon
  variant of 5.1.1, ~49k lines with compiler, GC, io / os, taint): its undump hard-codes an 8-byte `size_t` (the
  game's chunks have 4), host functions are plain `fn(&mut LuaState)` pointers (no captured state, so no
  name → closure natives), MSRV 1.92, and table order / number output are not the game's. Ours is ~4k lines with
  no compiler (everything ships as bytecode) and owns the details fidelity needs. The chunks use little: no
  SELF, 3 TFORLOOP, 1 POW; `string.sub/gsub/find/len`, `table.insert/remove`, `math.floor/ceil/sin/pi`,
  `tostring`, `tonumber`, `pairs`, `print`, `error` (C, opcode / GETGLOBAL scan).
- **What follows the original** (crate docs): `lundump.c` (size_t 4 or 8 accepted), the 38 opcodes of
  `lvm.c`, `ltable.c` ported (main positions, `luaS_hash`, `hashnum`, rehash sizes, re-insertion order,
  `luaH_getn`, keys created by `t[k] = nil`), so `pairs` order matches; `lstrlib.c` matcher and `format`;
  `ltablib.c` quicksort; 5.1.3 message texts and `ldebug.c` variable names (`?:0: attempt to call global 'X'
  (a nil value)`: stripped chunks, source `=?`, line 0); `luaL_where` positions only with line info (never in
  the game's chunks). Natives are global closures receiving their registered name (`FUN_009f86d0`).
- **The game's CRT and libs** (C, `.rdata` of the dump): `"%.14g"` at 0x01084474; pre-2015 MSVC CRT strings
  `1#INF` / `1#QNAN` / `1#IND` at 0x0109f3c8.. → numbers print as `1e+020`, `1.#INF`, `-1.#IND`;
  `"table contains non-strings"` at 0x01084574 is the 5.1.3 text (5.1.5 changed it; the host-diff test avoids
  it). `lua_tointeger` rounds to nearest-even (luaconf.h's `fistp` path for `_MSC_VER` x86, I: build flags not
  seen), `%d` truncates, out of range = 0x80000000; `math.random` = MSVC `rand()` (its state is shared with the
  game's C++, so the draw order is not reproducible).
- **How the game calls scripts** (C, dump): `FUN_009ef230` = `lua_getfield(globals, <script>)`,
  `lua_getfield(-1, <callback>)`, flag "is a function"; `FUN_009ef2e0` = `lua_pcall(L, n, MULTRET, 0)` and on
  error **pops the message without logging it**: script errors are silent. `FUN_007ec680`: `Construct(<light
  userdata = the script object>)`, then per JSON config item `SetConfigurationItem(name, value)` with value a
  string, an integer (`FUN_00ec0380` = `lua_pushinteger`), a boolean, or a double pushed as `(double)(float)v`
  (a float round trip). `mw_lua::Lua::call_field` does the lookup + call and returns the error (the host should
  drop it like the game).
- **Natives**: `print` is a native (`FUN_007e9920`: `lua_tolstring(L, 1)`, nothing else, no error for any
  argument), registered after the libraries, so it replaces base `print`. `ExitWidgetGroup` (`FUN_007f22e0`,
  argument 1 as a string, `"<invalid>"` otherwise) is registered with `FUN_009f7e50` too but missing from the
  385-name list (`scratch/scripting/reg_9f7e50.json`); the dump's 216 `FUN_009f7e50("…")` calls add only it:
  **386 natives**. The chunks read **171** of them (most used: `print` 70 scripts, `ResolveIntBinding` 69,
  `ExitWidgetGroup` 55, `ResolveBoolBinding` 48, `ResolveStringBinding` 43, `SetBoolVariable` 36,
  `SetIntVariable` 35, `SetStringVariable` 28, `DoRpc` 22, `WriteStringToOnlineSession` 19,
  `UpdateDeferredViewModel` 17). Read but not registered (nil in the game): `ToggleOfflineEventFilter`,
  `MarkAutologAlertAsSeen`, `GoBackEasyGuide`, `SaveCustomRollouts`, `SelectCustomRollout`, `SetRolloutColour`,
  `Change{Weapon,Package,UpgradePackage}OnCustomRollout`, `SetNo{EMail,Password}AsLastError`, plus script
  variables (`lsAction`, `liNumItems`, `liRecommendIndex`, `lsCategoryBinding`) and two other scripts' tables.
- **Scripts share a state** (I): `CarSelectMW` calls `CarSelectMenu.Back(...)` and `EditSpeedlists` reads
  `Breadcrumb`, both other scripts' global tables, so at least a screen's scripts live in one Lua state.
- **Verified**: `cargo test -p mw-lua`: 9 differential scripts compiled by the host `luac5.1` give the same
  output as `lua5.1` (numbers, coercion, patterns, `format`, `pairs` order with deletions and rehashes, table
  library and `sort`, closures / varargs / the `arg` table, error texts with variable names, metatables); MSVC
  formatting unit tests; on the install, all 115 chunks load, their names hash to their ids, main + `Construct`
  run, and `ViewLegal.OnAction` calls `ResolveStringBinding` then `ExitWidgetGroup("TOS" / "PP")`.
  `mwtool lua --natives <386 names>`: main + `Construct` of all 115 without error (9 natives called);
  `--call all` (every callback, no arguments) reaches 109 natives, every error comes from the missing context
  (nil arguments, globals another callback sets, the unregistered names above), none from the VM.
  Per-script table: `mwtool lua --natives <list> --call all --out <tsv>` (script, native, calls, first
  arguments; `nil` rows = not registered); the list is the union of `reg_9f7e50.json` and the dump's calls.
- **Model decisions**: tables / functions hash and print (`table: 0000002A`) by a per-state counter, not an
  address; at most 200 nested calls (LUAI_MAXCCALLS; the game allows deeper Lua recursion); memory is
  reference counted, cycles live until the state is dropped (the game stops its collector,
  `lua_gc(STOP)`); `collectgarbage` answers 0. Not provided (no chunk uses them): coroutines, `io`, `os`,
  `package`, `load*` / `dofile`, `getfenv` / `setfenv`, `string.dump`, weak tables.
- **Open**: one state for all scripts or one per screen (the state list of `FUN_009f86d0`), and when the game
  steps the collector; the arguments of `OnActivated`, `OnDeactivating`, `PreInputUpdate` and action callbacks
  (`OnAction` gets something `ResolveStringBinding` resolves); porting the 171 natives (mw-logic registry).
### 2026-11-17: JSON widget definitions and screen scripts, typed (M8 step 4, data side)
Worktree agent. Code: `mw_formats::json` (the engine's JSON reader), `mw_formats::widget` (typed definitions,
naming, the native screen-script table), `mw_assets::widgets` (screen → widget groups → widgets → JSON → scripts;
adapter hosts), `mwtool widgets`. Spec: decomp.md "Widget definitions (JSON) and widget scripts"; format:
formats.md "Widget definitions (JSON `TextFile`s)".
- **Result** (`mwtool widgets`, exit 0): 859 unique JSON files; 846 named by the 94 screens' `Widget_*` structs
  (CRC-32 of `"<Name>_<UniqueId>.json"`), all parse; the other 13 (`EN_US/FEEDBACKGROUPS`) are the adapters of
  `SequenceItems.AdapterHost` sequence items (referenced by handle). Scripts: 419 Lua uses of 115 chunks (every
  chunk of the install, none missing), 181 native uses of 22 names, 0 unresolved; the ScreenScript objects'
  `LuaAsset` equals the JSON's everywhere; `LayoutLayer` / `ModalMode` names map 1:1 to the GeneSys enum values on
  all 1,876 screen widgets (7 layers: `MODAL_BACKGROUND` 4 was not known).
- **Keys = GeneSys field names** (C): the JSON keys hash (GeneSys field hash) to the twin struct's field hashes,
  so the 400 distinct keys name 400 GeneSys fields (269 not in `genesys_names.txt`). Not added to the names
  file (it holds names found by dictionary attack; a bulk copy of the data's keys is closer to extracted data):
  open whether to read them from the user's install at run time.
- **Untyped (listed set)**: 18 widget types keep their own fields in `WidgetDef::fields` (online / Autolog / login /
  report / store and social blades / driver details / notification tabs / player points); `mwtool widgets`
  prints them per type. Everything else is typed, with a hard error on any unknown key inside typed records.
- **Native screen scripts** (C, snapshot `snap1`): factory table at manager +0x308 (count +0x408 = 24), the
  manager at 0x24473560 in that snapshot; each entry {`field_hash(name)`, factory}; each factory sits next to a
  registration stub returning (hash of the name string, factory). 24 = the 22 used + `RearViewMirror` and
  `MWL_VITA`. Names, factories and sizes in `NATIVE_SCREEN_SCRIPTS`.
- **Corrections to the research entry below**: `LuaHooks` *is* used at run time: `FUN_007f84c0` binds each listed
  name into the widget's Lua state (`FUN_009f1c30`; non-natives other than `tonumber` / `tostring` / `error` /
  `pairs` / `mfpFunction` set the error flag); `LuaModulesIncluded` picks "open the listed `LuaModules`" vs "open
  all five". One Lua state per widget (widget +0x84). Actions are read from the GeneSys `UserActionInstance`, not
  the JSON (the JSON key hashes for them are never referenced). `IsCallbackInLua` = "offer to the widget's
  scripts" (native scripts too); `"Widget::Function"` callbacks go to another widget's scripts.
- **Model decisions**: none in behaviour (no runtime yet). Parser choices: the JSON reader is our own port of the
  engine's (file-ordered objects, int vs double tags, first-match lookup), not serde_json, because
  `SetConfigurationItem` order and the int / double split are observable by the scripts; `lua_configuration()`
  rounds doubles through f32 as `FUN_007ec680` does.
- **Open**: the Lua init's two unnamed keys (hashes `67d01e38` bool, `cc2e0dc5` binding; absent from the data);
  vtable slots 7, 9–11; callers of `OnActivated` / `OnDeactivating` (`FUN_007ebc80` / `FUN_007ebcb0`); where
  `Widget::Function` is split (record +0x104); the release-build effect of the error flag +0x2028 (39 `LuaHooks`
  names are not natives); when the widget's Lua state is created; per-native behaviour (24 C++ classes);
  `RearViewMirror` / `MWL_VITA` never used by the shipped screens.

### 2026-11-17: game logic: scripting, flow, AI (survey of the install, the exe and prior work)
Research agent; full tables, registries and URLs in the main checkout's git-ignored
`scratch/engine-scripting-research.md` (raw outputs and scripts in `scratch/scripting/`). Whole install scanned
(2,020 bundles, 353,223 resources). C = confirmed, I = inferred.
- **One script VM: PUC-Rio Lua 5.1.3** (version string at 0x01083c3e), standard 38 opcodes, 32-bit size_t,
  double numbers, debug info stripped (C). No Squirrel, Scaleform / ActionScript or RAGE-style custom VM; the
  other "VMs" are C++ interpreters of data: HSM, bindings, the sequence player, PageRunner.
- **Lua = UI only**: 115 unique chunks (229 copies, ~25k instructions, 1,103 functions), `TextFile` (0x70) in
  `UI/SCREENS2`; id = CRC-32 of the lower-cased `"<ScriptName>.lua"` (all 115 match). Menus, map, car select,
  settings, online / store. No game-mode, AI or physics Lua (C).
- **JSON** `TextFile`s: 859 unique widget definitions (439 Lua callbacks, 224 native actions, binding adapters,
  script configuration); id = CRC-32 of `"<WidgetName>_<UniqueId>.json"` lower-cased (`FUN_007edd50`,
  `FUN_007e0850`) (C).
- **HSMData**: 17 unique machines: GAMEMODES' game-logic machine (2,706 nodes, 36 entry points, 199 behaviour
  types) and 16 active-aero machines (C). Node +0x14 is a data definition, not Lua (formats.md corrected).
- **GeneSys**: 1,138 type names, 83,655 objects; 8,485 `Sequence`s using 73 `*SequenceItem` types; 243
  `*_ScreenScript` objects (`LuaAsset`, `LuaHooks`, `LuaModules`, `LuaModulesIncluded` cracked); 24 types with
  no chunk = native C++ screen scripts (C). `LuaCode`, `LuaData`, AttribSys, BinaryFile, FlaptFile: 0 resources.
- **Bindings**: ≥2,701 distinct paths in JSON, Lua and HSM, ~100 roots (`Widget`, `Signals`, `Mode`,
  `Players`, …); operators seen EQUALS, GTEQ, LTEQ, MINUS, ADD, MIN, BETWEEN, NOT (C syntax, I full grammar).
- **Lua host** `FUN_009f86d0`: base + optional package / table / io / string / math (module table 0x0102a1fc),
  GC pause 100 / stepmul 200 then **GC stopped**; every native registered as a global closure with its name as
  upvalue (`LuaHooks` is not a runtime whitelist, I). Natives: `FUN_009f7e50` registers **385** Lua natives;
  `FUN_007763b0` **187** HSM hooks keyed by name hash (35 names in both). Widget scripts: factory
  `FUN_007f2850` (Lua script object vtable 0x00fe7f78, or one of up to 32 native screen scripts);
  `FUN_007ec680` calls `Construct` then `SetConfigurationItem` per item; other callbacks by name (I).
- **HSM never calls Lua**: it calls natives through `CALLHSMHOOK` / `CALLHSMHOOKS` / `HSMHOOKEXIT` (121 hook
  names used), arguments from the node's data definition. The exe registers 268 behaviour types (203 used).
- **Flow vs rules**: flow = the HSM; content = GeneSys (111 `OfflineEvent`, 12 `GameMode`, `EventList`,
  `AIPlayerInformation`, `ScoringAction`); cutscenes = sequences run by `RUNGAMELOGIC` nodes; menus = JSON
  widgets + Lua + native screen scripts; rules (checkpoints, positions, timers, scoring, rewards, unlocks) are
  C++ reached through hooks (`StartGameModeTimer`, `AwardPostEventAwards`, `RemoveAllCops`, …).
- **AI**: racers, cops, traffic are C++ state machines; no HSM / Lua / behaviour-tree data for AI (C); tuned by
  `AIConfig`, 200 `AIPlayerType` (`Behaviour` picks the state), 40 `HeatLevel` → `PursuitBehaviour` →
  `CopBehaviour`, `CopType`, `RoadblockRules`, `TrafficManoeuvreConfig`. HSM only pauses / preloads / removes AI.
- **Prior work**: burnout.wiki "HSM Data/NFS Most Wanted" (official field names; high value); BNDL Explorer
  (github.com/FrankHUN88453/BNDLExplorer: MW2012 editor, JSON / Lua text editors, HSM viewer; the PS3
  prototype's debug data names ~3/4 of GeneSys field hashes; licence to check, format knowledge only); DGIorio's
  tools (bundles, `HashBreaker`); nothing from GDC; no other reimplementation found.
- About 15 functions the scripts call do not exist in the exe (`ToggleOfflineEventFilter`,
  `SaveCustomRollouts`): dead or online paths (I).

Consequence (user, 2026-11-17: a 1:1 rewrite): CLAUDE.md "Fidelity rule" and PLAN.md M8: the logic data is
executed by ports of these interpreters. Open: what `HSB_STRSCRPT` runs; Lua state per screen or per script, GC
and error handling; the second HSM-style registry at 0x54d (RUNSEQUENCE / EXITBINDTRUE / EnableCompound);
`NavigationMesh` and `GameplayLinkData`; the full binding grammar from the evaluator; the offline / online split
of the natives.

### 2026-11-17: browser port, step 4: the runtime renders in a browser (WebGPU, free camera)
`mostrusted` builds for wasm32 with threads (`tools/web/build`: nightly, std with atomics, wasm-bindgen 0.2.129;
153 MB module without debug info, not yet optimised with wasm-opt) and runs in headless Brave on WebGPU:
`--camera=-1131,27,1627,-1190,15,1720 --units 4 --hour 13` loads the same 4 track units, 2881 models, 690 textures,
3788 instances as natively and renders the road at Hawaii's bridge (frame POSTed by `--screenshot`; matches the
native frame up to the canvas framing). What it took:
- `main()` split: native `main` locates the install and calls `run(cli, game)`; in a browser `web.rs` exports
  `start_with_files(FileList, args)` (the folder picked on the page) and `start_with_listing(base, listing, args)`
  (a served install: `tools/web/serve.py` serves the player's own install over loopback with HTTP ranges, the
  build, and the COOP / COEP headers). Flags are the native ones, from the page's `?args=` (`+` = space).
  `WebSource::from_urls` serves both; the base URL must be absolute (worker threads run from `blob:` scripts).
- Compiled out of the browser build: hidapi / `steam_controller.rs` (the Gamepad API covers pads), the SDL
  gamepad-mapping environment write (`set_var` panics there). Window = the page's `#mostrusted` canvas.
  `--screenshot` POSTs a PNG to the server (`platform::save_screenshot`). Bevy's `webgpu` feature.
- **`Layered` material within WebGPU's 16 samplers per stage**: its 12 texture / sampler pairs were visible to
  vertex and fragment (the vertex stage counted 18 with StandardMaterial's 6), and the fragment stage had 18.
  Now fragment-only, and two samplers: every game texture already had the same sampler
  (`convert::game_sampler`), bound once on `tex0` (a missing first texture is a white 1×1 image carrying that
  sampler, as Bevy's fallback is white); the planar reflection reads with the sky cube's sampler (both linear,
  clamp). Native frames unchanged: before vs after differs no more than two runs of the same build (2.5-3.3 %
  vs 3.0 % of pixels, traffic / particles).
- Settings, profile and saves: no config directory in a browser, so defaults and nothing saved (as before
  with no `$HOME`); browser storage is open.

Later the same day: the worker panic was the loader's `Receiver::recv_timeout` (std reads `Instant` inside):
`platform::recv_timeout` polls in a browser (`try_recv`, 1 ms worker sleeps = atomic waits, no clock); our
threads now set `Error.stackTraceLimit` (platform's `Builder` wraps wasm_thread's), which named it. Clean run
after that; **driving works in the browser** (`--drive 911 --units 12 --auto-throttle 0.4`: car, HUD, Easydrive,
minimap, streaming; slower per frame than native: 35 vs 47 km/h at frame 400). Open: audio needs a user gesture (headless runs are muted);
`AssetStore::new` indexes 2020 TOCs on the page's main thread (text path, a few seconds); the window size on the
page vs `--units` / memory (4 GB cap) for larger worlds; driving (step 5).

### 2026-11-17: browser port, step 3: threads as Web Workers, a main thread that never blocks
Decision: the browser build uses **shared-memory threads** (wasm32 with `atomics`, nightly `-Z build-std`, COOP /
COEP headers). Without threads every loader would have to run in a separate worker instance and send its results
as messages (the browser has no synchronous file reads on the page's main thread), a rewrite of every load path.
Bevy 0.19 supports atomics builds (bevy_render wraps its wgpu handles with `send_wrapper` there); its own task
pools stay single-threaded on the web, our loader threads don't.

- `mostrusted/src/platform.rs`: `thread::{spawn, Builder}` = `std::thread` natively, `wasm_thread` (0.3.3, same
  API, each thread a Web Worker on the shared memory) in a browser; all 26 `spawn` and 16 `Builder` sites use it
  (`steam_controller.rs` and `movie.rs` stay `std`: native-only, hidapi / ffmpeg). `PollLock::poll_lock` (a
  `try_lock`; busy = "not yet") for every main-thread read of a worker's result slot (24 `take()`s, the
  `is_some` / `clone` / `retain` polls in sequence_fx, world_coronas, music, world_sfx): a contended lock **traps**
  on a browser's main thread (`memory.atomic.wait32` is not allowed there). `StreamFocus`: the focus is polled
  (a busy lock skips one frame), releases go through an mpsc channel (a skipped release would be lost).
  `Instant` = `bevy::platform::time::Instant` (`std::time::Instant::now` panics in a browser; same type
  natively), seeds from `platform::time_seed` (`Date.now()` there). The bake pool falls back to the global
  rayon pool when threads can't be made; rayon-core 1.13's global pool falls back to the calling thread itself.
- `WebSource` changed from `FileReaderSync` (worker-only, `File` handles tied to one thread, `unsafe impl Send`)
  to **blob URLs + synchronous range `XMLHttpRequest`s**: the source is plain strings, `Send + Sync` without
  unsafe, readable from any worker; on the page's main thread binary sync requests are not allowed, so it reads
  `x-user-defined` text and takes the low byte of each char (works, slower).
- Builds: `tools/wasm-check --threads` (nightly, rust-src; the linker flags `--shared-memory --import-memory
  --max-memory=4294967296` and the `__wasm_init_tls` / `__tls_*` exports are required: without them the memory is
  not shared and posting it to a worker fails with `DataCloneError: #<Memory> could not be cloned`).

Verified natively: clippy 0, 201 tests, 911 accel unchanged (0-60 mph 3.58 s, quarter mile 11.60 s),
`--race 462174 --test race` 27/27 gates. In headless Brave (scratch wasm-bindgen module, `--target web`,
COOP / COEP served, `crossOriginIsolated` true): `WebSource` made on the page from 7 install files, `GameDir`
validated on the main thread (VERSION.TXT and a 1 MiB range of VEH_122769_HI through the text path, 23 ms); a
`wasm_thread` loader worker read the same range (identical FNV hash), built the `AssetStore` (4406 resources,
9 ms), then four workers loaded and parsed GLOBALEFFECTS / VEH_122769_HI / GLOBALTEXTUREDICTIONARY / SHADERS
at the same time through the shared store with the native counts and 0 errors (16-58 ms each); the page polled
the results with `try_lock` only.

Open for step 4: `mostrusted` itself on wasm32 (hidapi and ffmpeg compiled out, settings / profile / saves into
browser storage, the debug trace files off, the page / worker bootstrap); main-thread locks in the engine-free
crates (`AssetStore`'s bundle cache) are still blocking if the main thread loads: loads belong on workers.

### 2026-11-17: browser port, step 2: game files read through `mw_gamedir::Source`
Every game-data read in the engine-free crates now goes through `GameDir` (`read`, `read_at`, `len`, `resolve`)
over a `Source` trait (`mw-gamedir/src/source.rs`): `DirSource` (disk, native), `WebSource` (wasm32: the folder
the player picks with `<input webkitdirectory>`, `File` handles read on demand with `FileReaderSync`), `MemSource`
(tests). Moved onto it: `AssetStore::new` (TOCs via the new `Bundle::read_toc_with`), the store's bundle opens,
`instance_positions` (traffic_lamps.rs), SPS music streams (music.rs; the index now holds game-relative paths) and
the police speech container (`SpeechData::container` = the `GameDir` and the SWC0's path, read with `read_at`).
`GameDir::path` / `root` stay for native users that hand paths to other programs (mwtool, the ffmpeg movie
decoder): on a browser source they return `None` / an empty path, so movies are off there. The install root
inside a picked folder is the shallowest folder holding `SHADERS.BNDL` (picking the install or a parent works).

Constraint: `FileReaderSync` exists only in Web Workers, so in the browser the loads must run in a worker
(the main thread gets errors). `WebSource` is `Send + Sync` only without the `atomics` target feature (one
thread); a threaded build (step 3) has to keep the handles in the loading worker. The other `std::fs` users in
`mostrusted` are settings / profile / save files and debug traces: browser storage, step 4.

Verified: native unchanged (clippy 0, 201 tests, `mwtool verify` 0 failures over 2020 bundles, 911 accel
0-60 mph 3.58 s / quarter mile 11.60 s, `mwtool speech 1186352` decodes its 144 lines, `mwtool music --wave
010000000005dc99` 44.38 s). In a browser: a scratch wasm-bindgen 0.2.129 module in a dedicated worker in headless
Brave, fed `File`s of 7 install files under `Need for Speed(TM) Most Wanted/` plus one outside it: `GameDir`
validates (version 1.4.0.0), any-case resolve and `read_at` work (EOF error past the end), `AssetStore::new`
indexes 4406 resources (rayon falls back to the calling thread), and GLOBALEFFECTS / VEH_122769_HI /
GLOBALTEXTUREDICTIONARY load and parse with the native counts (521/129/12, 831/28/21, 33/33/0), 0 errors.
`FileReaderSync` is absent on the page's main thread, as expected.

### 2026-11-17: browser port, step 1: the engine-free crates build and run as wasm32
Question from the user: can the game run in a browser? Plan agreed: (1) engine-free crates on
`wasm32-unknown-unknown`, (2) file reads behind a trait (native `std::fs` / browser `File` handles from a
user-picked install folder: the data is never hosted), (3) the ~17 `std::thread::spawn` sites and rayon onto a
task-pool helper, (4) `mostrusted` on WebGPU behind a `web` feature (free camera first, no audio), (5) driving,
then audio. The native build stays the reference for captures and comparisons.

Step 1 result: `mw-gamedir`, `mw-bundle`, `mw-formats`, `mw-audio`, `mw-assets`, `mw-handling` compile for
wasm32 with no source changes (`tools/wasm-check`). A throwaway raw-ABI module (scratchpad, not committed) ran in
Node 24 on real bundles: `Bundle::from_bytes`, `load` of every resource (decompression via flate2/miniz),
`Texture::parse` and `Renderable::parse`. Results identical to the same code built natively: GLOBALEFFECTS 521
resources / 129 textures / 12 renderables, VEH_122769_HI 831 / 28 / 21, GLOBALTEXTUREDICTIONARY 33 / 33 / 0,
SHADERS 3099, HAWAII/GLOBALRESOURCES 549 / 261 / 101; zero load or parse errors. Speed: wasm 1.4-1.6x the native
time (GLOBALRESOURCES 607 ms against 390 ms, single thread both).

Compiles but fails at runtime on wasm32-unknown-unknown (input for step 2): every `std::fs` call
(`mw-gamedir` install detection and `read_dir` case lookup, `Bundle::open` / `read_toc`, `mw-assets` `music.rs`
and `police_speech.rs` partial reads with `File::open` + seek). `std::env::var*` returns nothing there (the
`MW_*` switches are simply off). `AssetStore::new` uses rayon `par_iter` over every bundle's TOC: rayon-core 1.13
falls back to the calling thread on wasm. Runtime-only blockers kept for later steps: threads in `mostrusted`,
the 4 GB wasm32 memory cap (the free camera still loads the whole world, ~4.5 GB GPU), `hidapi` (Steam
Controller) to be compiled out, storage buffers in `game_lighting.wgsl` need WebGPU (no WebGL2).

## Open questions

### Textures
- Header flags at 0x20: values 0x00/0x10/0x20/0x30. Is one of them sRGB? (Currently all colour textures are
  treated as sRGB.)
- Byte 0x2C ranges 0–10 and does not mean missing mips (full chains are present). Streaming LOD bias?

### Renderable
- Mesh bytes 0x24–0x27 (sub-part code, then 1, 0, 4 in samples).
- The compressed OBB at mesh+0x00: the exporter documents its quantisation. Decode it for culling.
- Buffer descriptor words `{0, 1, 3|2}`: D3D11 bind flags?
- Is the SNORM position scale always 10.0, or does it ever vary per mesh/renderable?

### Vertex data
- `Color` (RGBA8): roads = baked light (rgb² × GlobalLightMapIntensities.x × 0.318) + AO (a); vehicles = AO (x)
  and specular occlusion (y); trees = AO (y). (Answered 2026-10-04 for these families.)
- World layouts often have Tangent + Binormal but no Normal (cables): is the normal `cross(T, B)`, and with which
  sign?

### Materials / shaders
- Lightmap shaders sample `LightMapTextureSampler` at TEXCOORD1, but no material or instance override binds it.
  Where does the texture come from (a separate resource per track unit, GeneSys, runtime)?
- Bevy's clear coat now renders on vehicles (they have tangents since 2026-10-04).

### World
- InstanceList instance `unknown_04` (f32) and `unknown_08` (u32).
- ZoneHeader (0x206, one per track unit, version 9): import slots at 0x04 InstanceList, 0x0C
  DynamicInstanceList, 0x14 LightInstanceList, 0x1C CompoundInstanceList, 0x24 PropInstanceList, 0x2C one
  GroundcoverCollection, then `u32 count` at 0x34 and `ptr` at 0x38 to more GroundcoverCollection imports
  (2026-10-04, TRK_UNIT1). No visibility data in it. Not parsed yet.
- How the game draws near water (city rivers, the airfield lake): no zone lists a water mesh over the river
  next to downtown (unit 13), and no main geometry covers it. A separate water system? (`--backdrop-zone 13`
  shows the gap.) Answered 2026-10-16: zone 13 has no river (unmodelled block interiors); every zone lists the
  water it shows.
- ZoneList zone byte 0x40 (0 or 1) and the header u32 at 0x18 (3).
- What does `WORLDLIST.BNDL` contain, and how does it relate to `HAWAII/MAPS/*.BNDL`?

### Data
- GeneSys field names: the hash is known (2026-10-04). About 1,200 of 5,520 distinct field hashes are named;
  the rest need better word lists. `655d3f5f` (the `CgsCore.UniqueId` at +4 of every object) is unnamed.
- The 64-bit type hash at type+0x10: check it against `field_hash` of the type name (and a 64-bit variant);
  it may also be a layout fingerprint.
- GeneSys: three objects per vehicle bundle still fail to decode (pointers read as huge offsets).
- AttribSys: no `AttribSysSchema` / `AttribSysVault` resources exist in the install (checked 2026-10-04).
- Drift (2026-10-06, from the code): every field's role is settled (decomp.md "Drift"). Still open: the speed
  hold's `.bss` constant 0x01187250, the counter-steer `powf` exponent (taken as `Max_counter_steering_power`),
  and whether the outward side force reproduces the original's drifts (needs a recording).
- Tyre data not yet understood: `Lateral/Long_Slip_Factors`, `Grip_factor` / `Sliding_grip_factor`,
  `Available_grip`, and the lateral curve's fourth field (0.5 everywhere). Five more `Tyres` floats per axle
  (400, 1000, 2) are unnamed.
- VehicleList entry fields beyond ID / names / year (floats look like stats; u16 near 0x6A may be RPM).

### Environment and weather
- What switches to the `Alternate` keyframes (7 hazy twins, one per time-of-day band; no timeline lists
  them). A gameplay state (events, pursuit), a random weather roll, or a menu option? Nothing in GeneSys
  references them except the keyframes.
- `Weather.Thunderstorm_Intensity` (0.23 on night keyframes): how the game shows it (no lightning shader,
  texture, effect or sound was found). `Weather` `0141d842` (0.25 day / 0.15 evening, night and Alternates)
  and `e8eb5d48` (0 everywhere) are unnamed.
- Who reads the wind (`WindBearing` 130° ± `WindBearingVariability` 25°, `WindSpeedMin/Max` 1 / 3.5)? The
  tree shader's sway is per material and only time-driven; cloud offsets are CPU-side (`Clouds.Speed` 5).
- `WetScaleParams` values (puddle tiling, reflection distortion, reflection blur): CPU-set; ours are guesses
  (`materials::WET_SCALE_PARAMS`). The planar reflection plane height and its "Modified" projection.
- When surfaces use `SurfaceType.Wet_Conditions` (wet tyre marks, spray, sounds) instead of `Dry_Conditions`.
- `EnvironmentSettings.DayNightTransition` (`1ab46d30` 8.5, `0e2be130` 20, two (1, 0) pairs) and the
  `Rendering` block (sky material handle, `MoonDiscAlpha`, `SunDiscAlpha`, `31c63004` 180, 3.5 × 2).
- `WorldPaintMap` header u32 at 0x24 (4878 / 1926 / 94).

### Vehicle damage
- How the game turns an impact into zone damage: which `ImpactDamageGraphs` curve (two of them; the second
  has its low point the other way round, `(0, 6 %)`?), what units (mph assumed), how damage is split between
  zones (we use the nearest zone plus a 60° falloff), and what `CrashingRules` / `CollisionInfoDamageProfile`
  / `DamageBarProfile.ImpactLocationDamageScale` / `Physics.LandingDamage` add. Global objects in
  `PHYSICS/PHYSICSCONFIG.BNDL` and `GAMEPLAY.BNDL`.
- `VehicleDamageBehaviour.Bodypart`: the hinged parts (doors, bonnet, bumpers, mirrors) presumably swing
  physically once their `DamageThreshold` looseness exceeds 0. Which bone does each hinge drive? (Hinge
  positions don't coincide with bone positions; bone names are unknown.) What are the three spring-like
  Vector3s (`99e38802`, `19beda43`, `13ac399e`), `9a601fda` (rear bumper `[-3, -5, 0.35]`) and the
  `DamageThreshold` value `31ad3690`?
- `ControlMesh` (GeneSys, one per car, five Vector4s like `[2, 5, 0.2]`, `[0.01, 0.75, 0.75, 0.85]`): a
  separate resource type 0x210 exists in the type list; is it the crumple / damage-volume tuning? The
  car's `0x108e7a` GeneSys object (truncated decode) may be related.
- `WheelDamage` (four per car: steering angle −20..30°, camber-like `b44cbfcf` −10..−20): damaged-wheel
  alignment, not applied yet.
- Damage volume grid (3 × 5 crumple values per car) for the crumple normal map: the shader side is known;
  how the game fills it from the zone levels is not.
### Particles (BearEffect)
- The emission controllers: 0x33 / 0x34 decoded 2026-10-09 (`Bindings`); 0x42 (`UpdateWheelBindings`, 4 inputs)
  is not, and the binding-slot bytes of the wheel and collision spawners (0x05, 0x07, 0x01) are not: those still
  use curve 0 over `Intensity`.
- What the game writes into `WheelParams` (0x40 bytes), `VehicleParams` (0x30), `CollisionMeshParams` and
  `WheelContact`, and what it means by `Velocity` / `Intensity` for each effect (code).
- Spawner fields: the three angle ranges of the wheel spawner (0x05: `180,180 | 0,180 | 0,360`), its
  `0.9..1.2` range, the 0x07 `0.25 ×3` triple, the u32 flags at the end of every spawner.
- `Bounce` / `Integrate` drag (0–0.999) and the two trailing floats; how bounce planes come from
  `VehicleParams` / `WheelContact`.
- `ViewFade` (0x38): its 12 leading floats (all 1 but one 0.6462) and the `20 51` / `20 40` pair.
- Renderer words: the first float (0.01–4, read as a soft-particle fade distance), the rgb(a) after it,
  the trailing u32 flags; the mesh renderers (0x1E/0x1F/0x22/0x23) and their Model imports; 0x4D
  (`VfxSplatter_Dynamic`).
- `VfxVehicleCollisions` (36 handles: 2 empty + 16 effects, twice) field names, and which collision picks
  which slot. `SurfaceType` weather conditions `f395b019` / `7348be25` (dry / wet by their contents), the
  `VfxWheelEffects` fields `61ac1863` / `e7a7150c`, and the rest of `SurfaceVfx`.
- `gVfxSparkBrightness` (spark shader global) and the particle VS's exposure term
  (`lerp(1, exposure.w, gVfxConstants.x)`): `VfxGlobals.Rendering` has `ExposureScale` 0.4,
  `ExposureScaleCoronas` 0.4, `ExposureGamma` 2.2 (cracked) and unnamed `dacaf024` 0.4, `a77107f8` 10.
- Collision surface ids (`PolygonSoupList` tags) → `Surfaces.Surfaces` index, so dirt / grass get their dust.
- World effects: `mwtool genesys all --type WCVfxBehaviour` finds 3,931 objects (3,911 in `TRK_UNIT*`): 2,302
  have `Spot_effects` (`Spot_effect` handle + `Locator_group` id; e.g. `…24f3d`, `…eee1b`, `…af136`), 1,095 an
  `ImpactEffect` (`…24f3d` 613×, `…1621e7` 410×: prop smashes), most have `Lights` (`LightDefinition`) or
  `Coronas`. Contact-driven ones (impact, contact and spot effects, lights off) are ported since 2026-10-16
  (`prop_vfx.rs`); ambient ones that are on from the start (power-plant smoke, "AmbFX Dummy") are not. The
  exhaust effects are the cars' spot effects (2026-10-09).

---
