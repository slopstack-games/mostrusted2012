> **Standing goal (every session):** the game must end up as close as possible to the original NFS Most Wanted
> (2012) in every respect: driving, look, sound, AI, UI, cutscenes and game logic. Work from the original's code
> (the dump, REA on the carved image), its data, and measurements / captures of the running original (CLAUDE.md
> "Fidelity rule"); never settle for "plausible". Open differences are tracked below, in docs/PLAN.md and the
> README roadmap.

# Handoff: 2026-10-19 (session 11, Linux): start here

**State**: branch `session-11` (from master after PR #6); about 20 worktree agents merged. Clippy 0 warnings, 199
tests, `mwtool verify` 0 failures; 911 accel 0–60 mph 3.58 s / quarter mile 11.60 s, nitrous 0–60 3.20 s, brake
38.1 m, corner 2.57 g, drift peak 37.3°; `--race 462174 --test race` 27/27 gates, Most Wanted races spawn their
rival. Every run of our game is headless and silent (`tools/t`, `tools/hl`; never a window or sound on the user's
desktop: CLAUDE.md); the original too (`tools/orig`, null sink). The research log has one dated 2026-10-19 entry per
area below.

| Area | What landed (research log 2026-10-19) | Open |
|---|---|---|
| Kerbs / road harshness | wheel = the game's cylinder probe; strut travel from data; decal triangles skipped; placement at speed; strut joint; strut stop only when crashed | chassis lifts more than the original at a kerb; yaw from surface bumps; launch: ours spins one rear wheel, the original both (tyre / differential) |
| **False crashes at kerbs (user high priority)** | the crash rule and wall-scrape counted avian's speculative contacts; now only touching / pushed ones | **needs the user's play test**; the original's chassis hulls as our body; a capture on the kerb-end line |
| Terrain / floating trees | pass mask per view; per-zone backdrop and groundcover swap (`zone_view.rs`) | stand-in shadows (capture at 17:30); view 0x10 |
| GPU out of memory | world streaming as the game does it (`streaming.rs`): ~1.3 GB instead of ~4.5 GB | idle-machine frame times; free camera still loads everything |
| Probe capture helpers | `mwtool tp / car / mods / conditions / inputs`, `capture --probe`, tick-exact scripted inputs, Hodges strip | far placements kill the original (GPU OOM suspected); airport arena not reachable |
| Nitrous | no downshift while it burns (`FUN_009d93b0`); mods cheat in the probe | a capture with the game's own tank; every car gets nitrous in our handling even without the mod |
| Races | Most Wanted rivals (per-slot rollouts), loading card on race starts, progress kept across a respray | Most Wanted 7's cops; the win's shutdown / jack spot / unlock |
| UI | placement / scaling from the code (HUD aspect correction, menu letterbox), free-drive HUD screen, Easydrive prompt, SP strip, plates, nitrous gauge by mod, pause rows | odometer spacing, digit glow, GPS / pursuit / ticker layouts, Most Wanted list panel |
| Surfaces | tyre classes; kick-up rule; per-surface lookup verified (cobbles are dirt-like in the data) | a coast capture on paving vs asphalt |
| Sounds | traffic pitch (whine), transmission / turbo whines, bus gains, per-wheel surface sounds, doppler fix, skids follow the engine's level | **user: listen to drifts and the rest**; Ginsu grain player; bus EQ / compressors; collisions / props / AI levels vs the engine |

Decisions with the user: portrait loading screens keep the original's bars.

## Open items (user play tests; newest first)

- **User requests of 2026-11-17 (end of session 11), in this order** (decided with the user):
  1. **Skid / crash sound "extremely loud, distorted, a glitch"**, in both the desktop and browser versions,
     on the player's drifts / handbrake and on crashes. Not reproduced headless (research log "skid / crash sound
     glitch"):
     - The live output (recorded from the null sink, `docs/commands.md`) matches the tap: handbrake turn, a
       58 mph crash and wheelspin against the obstacle, peaks 0.25, no clipping, no clicks.
     - Against the original's own audio (`ref/xkr_launch`, brake vs full throttle) our skid is not louder.
     - Next: the user's exact situation (car, pad / keyboard, place, the moment it starts), or a recording of
       it. Then a capture of the original's drift and crash with audio (`mwtool capture --probe`) for the
       reference.
  2. **Handbrake doesn't swing the car** (user): at 60 mph with full lock, Space locks the rears (braking skid
     0.95) but the slip only reaches 0.16 in 1 s. Trace the handbrake's drift entry in the code (handling.md /
     decomp.md "drift").
  3. **Ownership and DLC**: done the same day (retail exe hash `GameDir::ownership`; the DLC lock in `dlc.rs` over
     the ported entitlements). Left: lock badges in the menus' lists, the store offline (nothing) (user decision): an install whose retail data passes a checksum counts as the owned
     base game (no other check); **all DLC cars and areas locked** until a real entitlement source exists. The
     install is one Steam depot with every DLC inside, so files can't show DLC ownership. Find what the original
     gates on: gamepacks, `IsAssetUnlocked`, `Mode.CurrentVehicleGamepack.Release`, the airport arena's
     "Show Airport Required Dialog".
  4. **Story progression** (user: "still a glorified sandbox"): the career, the Most Wanted list and the unlock
     rules from the original's logic. Run as sonnet agent rounds (M8) with the runtime carrying out the
     requests.
  5. **Split the big docs** for cheaper sessions (decomp.md ~570 KB, the archive ~720 KB, formats.md, PLAN.md):
     by system, keeping `tools/doc` working.

- **Races and free roam** (2026-10-19), worked on at the end of session 11 (research log "race flow", "event
  starts, cop arrows, search circle", car-against-car response in decomp.md): fixed "checkpoint missed" (road-path
  distances, `FUN_00661b40`), racers removed after a race (`FUN_00521f00`), jack spots after a race (the event
  ends after its results), races on the minimap, cop arrows blue / blinking (not yet seen on screen), race-car
  contacts with the game's impulse scales, start markers in the world. **Open**: the escape / search circle (rule
  traced, not drawn) and cops clamped to the minimap edge; the start panel's event name / reward labels; the burnout
  start never run; race intros' special shaders / visuals; a second race in one run is refused; the AI racers'
  crash reset path and takedown camera; AI–traffic responses; whether the outro's AI-driven player uses the body
  shop. The UI behaviour-kind fix (kinds 13 / 4) is global: check other screens' blinking elements.
- **Too-loud skid / drift sounds NOT resolved** (user, end of 2026-10-19, after the `MW_SFX_ENGINE_LEVEL` fix): it
  seems to appear when starting a race. Suspects: the race start rebuilds / re-binds the player's car (the
  `attach_player` path, the intro's AI-driven player, a respray reload) so the vehicle bank loses the engine-level
  gain (`audio::PlayerEngineLevel` not re-published) or a second bank starts; the `RaceCarEntities.RaceCar[1]` skid
  sets binding to the player once racers exist; the competitor (AI) tyre bank (+4 dB bus) playing at full level near
  the player. Measure with `MW_AUDIO_TAP` across a race start (`--race 462174`), free roam vs race.
- ~~**Body shops / repair stations**~~ **done 2026-11-17** (respray in place, `paint_wear::Respray`; research
  log 2026-11-17) (user, 2026-10-19): only cars with liveries behave right (nitrous refilled,
  repaired, cleaned). Cars with plain paint colours get replaced by a whole new car spawned in a different colour.
  The game resprays the same vehicle (`FUN_00748ff0` → vehicle-manager message 0xc, research log "race flow" /
  Most Wanted races entry); ours reloads the car on a respray (`garage.rs` "loading ..."): change the paint in place
  on the existing car (material / paint parameters), keep the entity, its damage reset and nitrous refill, no
  reload. Related: the race progress stash in `race::attach_player` exists only because of that reload.
- **Exhaust flames**: backfire / nitrous flames look framey and not attached to the exhaust (`exhaust.rs`,
  `particles.rs`: emitter space, in-frame spawn interpolation, flipbook rate).
- **Textures**: the tyre tread is blank (tyre material family / samplers, `mwtool shaders`).
- **Wet roads in shadow** still look lit (wet layer's reflection not shadowed).
- **Wheel placement** wrong on some cars, e.g. the Veyron Super Sport.
- **Missing animations** (not listed yet: survey).
- **Menu / Easydrive functionality** missing in many places (survey first).
- **UI content** left: see the table.
- **Sounds**: cars still a bit glitchy (the game's Ginsu player needs window W from `0x00aaef50` / `0x00aad630`).
- From session 10's list, not worked on yet: collectable effects (speed camera flash on pass), gas station
  transition (fade / cut, repair, respray, nitrous refill `FUN_00748ff0`), collectable stats UI (speed lists,
  counts), roof landings and out of bounds (crash camera + reset onto the nearest road), car lights on the world,
  crash particles (glass, debris), camera clipping (find what the original does), dirt cloud size and dirt feel.

## Future plans (user, 2026-10-20)

- **Split screen** in our game (two local players: two views, two HUDs, two input devices; the original has no
  split screen, so this is an addition on top of the faithful game, kept switchable).
- **Mod menu for the original**: the debug / mod tools (the probe's teleport, car select, mods fitting, fixed
  conditions, scripted inputs and future mods) usable while playing the original, through an in-game UI overlay
  (a menu drawn inside the running game, e.g. from the probe DLL hooking the game's present / the D3D11 swap chain,
  driven by keys or the pad), not only from `mwtool` on the command line. Same policy as the probe: runtime only,
  offline, the exe on disk untouched (reverse-engineering.md "Code inside the running game").
- **Browser build** (2026-11-17): steps 1-3 done (engine-free crates build and run as wasm32, `tools/wasm-check`;
  game files read through `mw_gamedir::Source` / `WebSource`; threads = Web Workers on shared memory via
  `platform.rs`, main thread polls with `poll_lock`, verified in headless Brave); step 4: the runtime renders in
  a browser on WebGPU (`tools/web/build`, `tools/web/serve.py`, `tools/web/headless`), free camera, driving and
  audio (mixer on an AudioWorklet thread, patched crates in `third_party/`). Next: saves in browser storage,
  frame time, memory for whole-world views (PLAN.md M9).
- **Game logic engine** (PLAN.md M8, 2026-11-17): bindings, HSM interpreter, Lua 5.1 VM, widget data and the native
  registry (`mw_logic::natives`, `mwtool natives`) are in.
  - Ported and verified against the dump: **120 HSM behaviour types**, **160 / 187 HSM hooks** and **100 / 385 Lua
    natives**. Every offline Lua native the UI chunks read is ported. Binding roots: `Mode`, `ModeVM`, `Net`, the online
    match states, `UIConfig`, `SignInState`, `UISequences`, `GeneralSequence`, `EntitlementManager`, `Players`, `Event`,
    `Transitions`, `GameLogicState` (root table in `natives/state.rs`). Under `--hsm` races run through the logic
    (`--hsm-event <id>`: free roam → race mode 730003 → back), and the runtime carries out the song list, UI sounds,
    sim pause, show world, mute, chevrons and triggers. Open: the load requests (world, profile, feedback groups,
    layouts, sequences, players), the menus' own widgets (EasyDrive, map) instead of the hand-written ones, the AI /
    timer / add-AI requests; the body shop's respray reloads the car (with `--units 12` the reloaded car has no
    ground: `--race 462174 --test race --units 12` stalls after gate 3; repaint in place). The boot reaches "Freedrive Update" with no unhandled exit.
  - Next round's candidates: the research-log entries of 2026-11-17 (fifth hooks, `behaviours_mode2.rs`,
    `behaviours_event.rs`, the binding roots: `EntitlementManager` with `HasPdlc*` is where the DLC lock belongs,
    then `UIConfig`, `SignInState`, `UISequences`, `GeneralSequence`). Many new Requests have no runtime handler yet
    (`PlaySongList`, `PauseAi`, checkpoints, event starts, car swap, `PlayUiSound`).
  - The game-mode manager starts the mode machine (`natives/mode_machine.rs`). `mwtool hsm --run` reaches
    "Freedrive Update" and the HUD machine (docs/commands.md).
  - **The runtime runs the HSM in shadow mode** (`mostrusted --hsm`, logic.rs: research-log "the original's HSM runs
    inside the runtime"). It follows the title, press and world load into free roam. Requests are logged, not
    carried out.
  - **Front-end screens from the HSM** (same day, research-log "front-end screens from the HSM"):
    - Page contexts and widget groups run in `natives/pages.rs`, and the widgets' Lua scripts run.
    - Under `--hsm` the boot page's title and transition are started by the original's script / HSM.
  - **Next (integrator, default model)**: carry out the Requests one at a time and remove the hand-written flow each
    one replaces.
    - Done the same day (research-log "widget input and conductors"): widget actions fire from the menu input,
      and conductors run headless (only the title is drawn).
    - `Mode` root done (research-log "the `Mode` binding root").
    - Next blockers: `ModeVM` (`GameModeViewModel`, EasyDrive's "Open EasyDrive Request") and `Net.IsInOnlineGame`.
      The runtime's mode machine now waits in the Game Recap page (1560241): check the original's boot with a save.
    - A generic sequence player that draws conductors' sequences (frontend.rs / event_card.rs each have their
      own), then the pause menu (431082, Lua `PlaySequence`) from the HSM.
    - Drawing the HUD screen 371621 from its widgets instead of `hud.rs`.
    - Then `LoadWorld`, `LoadProfile` and the game mode. Fill the runtime-filled `LogicState` fields: ranks, awards, `garage`,
    `current_event_kind`. Add `Mode` / `Event` binding roots in place of the stand-ins.
  - **Next batches (sonnet agents)**: the candidates are in the 2026-11-17 research-log entries, listed below.
    Rounds run cheapest as `sonnet` agents with a fixed batch (CLAUDE.md "Parallel agents"), one fresh session per
    round.
    - Behaviour types: MUTEALLRUMBL, PAUSEMSGS, PLAYSONGLIST, WEAPONSOK, MINIMAPICONS, ...
    - Hooks: the parked OnOpenEasyDrive family and others.
    - Lua natives: the state-field ones first.

New this session (docs/commands.md): `tools/hl`, `tools/silent-sink`, `mwtool tp / car / mods / conditions / inputs /
probe`, `capture --probe --script`, `mwtool surfaces --flat / --tris / --classes`, `placement-check --floating`,
`terrain-check` LOD modes, `--load-all`, `--stream`, `--composed-backdrop`, `--replay ... --spawn`, `MW_MUTE`,
`MW_AUDIO_TAP`, `MW_SFX_ENGINE_LEVEL`, `MW_AUDIO_NO_BUS`, `MW_SURFACE_LOG`, `MW_SPECULATIVE_CONTACTS`,
`MW_CRASH_CONTACTS`, `MW_WINDOW` / `MW_RESIZE_AT`, `MW_UI_TEXT_LOG`, `MW_WHEEL_RAY`, `MW_STRUT_JOINT`.

