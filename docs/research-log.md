# Research log

Dated findings and open questions about the game's formats and behaviour. Add new entries at the top. When a
question is answered, move the answer into the parser's doc comment and [formats.md](formats.md), and note
the date here.

Install used for verification: Steam build, `VERSION.TXT` = 1.4.0.0, 2,020 bundles.

Older entries (before 2026-11, and the early topical findings) are in
[research-log-archive.md](research-log-archive.md); move a month there once the next one has begun.

---

## Dated findings

### 2026-11-17: 20 HSM hooks ported (`natives/hooks_flow.rs`, `hooks_event.rs`)

Handlers are not in `functions.csv` (vtable-only); read from `re/out/live/text.asm` (awk from the address to the next int3) with the helpers' bodies from the dump. Ported (all offline): CalculateShouldShowRecapSequence (GMM+0x2e88 := 1), ClearRecapFlag, ClearMatchmakingState (FUN_00675470), PopulateSettings / SaveSettings (FUN_0086e0a0 / FUN_0086fe20 on the settings at +0x2f3e0 of the object behind 0x117e9f4+0x2020, not G), SetNewGame (FUN_0068fd50), SetSinglePlayerRollout, SetRolloutFromProfile, IsNewGame, ClearNewGameFlag, ShouldPlayNewGameIntro, SetNewGameIntroCompleted, SetIntroEventNext, IsIntroRace, IsProgressionReady, SaveProfile, StartGameModeTimer, StopGameModeTimer, ResetGameMode, IsOnlineEvent. Each returns EXIT (0x1177ef0) or TRUE / FALSE (0x1177fe0 / 0x1177ee0). State: LogicState.game / config / game_modes / players / progression (offsets in field docs); engine work as Request variants. About 70 further hooks from a stopped pass are in register_parked (unverified, not in Registry::original(); original_with_parked() for tests). Model decisions: missing / mistyped arguments read as zero; ResetGameMode sends no telemetry record. mwtool hsm --run stands in a loaded profile, so IsProgressionReady / ShouldPlayNewGameIntro are no longer answered by hand; the boot trace is identical. Open: verify the parked hooks one by one.

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
