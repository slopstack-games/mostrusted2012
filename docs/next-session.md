# Handoff: 2026-10-14 (session 8, part 2): start here

**State**: master has three agent branches merged on top of part 1: the post chain (bloom levels fixed
320×176 / 80×44 / 20×11, depth of field + colour cube final pass, sun-facing grade, vignette from the code, cube
blend), menu input and the map screen's controls (the original's 13 menu actions, stick / d-pad / auto-repeat,
flick / free cursor / snap from `MainMapConfig`), and lighting (the game's sun path, sky constants, ambient from
the sky, fences, 325 m shadows; the original's time of day found at environment manager + 0x1b604). Clippy 0
warnings, 166 tests, `mwtool verify` 0 failures, 911 accel / corner unchanged. Not pushed. Each area's
research-log entry (2026-10-14) lists its sources and what is open.

- **Police: NOT merged.** Branch `worktree-agent-a6c7a3ed5babae9f8` (worktree kept, commit 4d9bd0d, WIP):
  `cop_driver.rs` (chase aim / lead, LowSpeedChasing block / creep / ram, box-in, Bruiser, Rhino, HeadOn from the
  code), spawns / removal / search / heat per contact from the code, `mwtool ai-graph`. **`--test pursuit`'s bust
  phase timed out in its last run**: find why, run clippy / tests, then merge. decomp.md still says the 1.5 mph/m
  speed law applies to every cop; the code (`FUN_004fa480`) applies it only to cops without a car (fix the doc).
  Next there: roadblocks (rules read, research log), traffic-lane patrol (`FUN_005101e0`), sirens / light bars.
- **Lighting open**: the ambient's brightness (the game's `IrradianceQuadric` CPU build not traced; ours ~0.3–0.5
  vs 0.8–1.0 upward), wet surfaces without `--planar-reflection` reflect the sky cube (blue pavements, white road
  blotches: consider the planar pass by default, measure its cost), trees 2–4× too dark, 18:30 cascade fit,
  materials' own reflection cubes, a second distant matched place. Matched shots of the original:
  `ref/shots/{fairhaven,fence}-<hour>/` (with every shader constant at the moment).
- **Post open**: 22:00 frames blown out (upstream: exposure / lighting), the composite's motion blur (small),
  exposure metering from the 80×44 level, the sun lens flare (no reader found).
- **Map open**: tabs (PgUp / PgDn, LB / RB) and filters, the Most Wanted list panel, pad glyphs in prompts, the
  pause menu's QUIT item.
- The user's original-game save was rewritten by the game on load in several runs; the agents restored it or kept
  backups in `ref/savebackup-2026-10-14-*` (the post agent did not restore: its pre-run copy is
  `ref/savebackup-2026-10-14-post/`).

# Handoff: 2026-10-14 (session 8): start here

**State**: master (4c0e08e) has the six 2026-10-10 worktree branches merged (suspension point masses, wheel
placement, chase camera port, player horn / grind / impact sounds and parked cars, frame time, map screen and
event card); clippy 0 warnings, 155 tests, `mwtool verify` 0 failures, 911 tests as the suspension branch
recorded (wheels moved 4–8 mm by the placement rule). All worktrees removed; the camera agent's `ref/cam`
and the probe's `ref/probe/wheels` were kept. Not pushed.

- **REA** (MCP server `rea`, Ghidra provider) is installed. `tools/snapimage.py` carves the decrypted image from
  `re/out/live/snap1` and exports the **16,257 function starts the dump missed** (vtable-only methods); open
  `re/out/live/NFS13_snap1_vt.exe` with `open_binary` (first query waits ~10+ min for Ghidra's analysis;
  `REA_GHIDRA_STARTUP_TIMEOUT_MS` = 1 h). `re/out/live/text.asm` is an objdump of all `.text` for operand scans.
  reverse-engineering.md "Functions the dump is missing".
- **Chase camera**: the original's per-frame camera path is fully read and has no world query; our wall push-in
  is off (`MW_CAMERA_PUSH_IN=1` for A/B). A capture of the original backed into a wall would confirm.
- **Next with REA**: the remaining "not in the dump" / "writer not found" items (research log 2026-10-14):
  `LAB_005a22e0` (a wave's gain), `AIPlayer::FillInput` 0x006c0890, the constraint +0x210 writer, the
  `world+0xa2bee` start-over flag, `0x0118a060`'s writer, `FUN_008fcd60` (an undocumented body orientation
  controller); the steering q's writer was searched and not found.
- Still open from session 7 below: water level, the wet road's grazing reflection, the original's time of day
  in memory, a user play test of everything merged since 2026-10-09.

# Handoff: 2026-10-10 (session 7, Windows): start here

**State**: branch `windows-port` (not merged into master, not pushed): the workspace builds and runs on Windows
(clippy 0 warnings, 141 tests, `mwtool verify` 0 failures, the 911's accel numbers as on Linux); the Linux
side was only type-checked for the non-Bevy crates from Windows, so run build / clippy / tests once on Linux.
Details: the 2026-10-10 research-log entry and the Windows paragraph in CLAUDE.md.

## Paused work: four agent worktrees with uncommitted changes (stopped mid-task by the user)

Nothing of these is committed, built or checked; each worktree is `.claude/worktrees/agent-<id>` on branch
`worktree-agent-<id>`, based on `windows-port`. Resume each agent, or read its diff and finish it by hand.

| Worktree | Feature | What is there |
|---|---|---|
| `agent-a76d4bd5cc796831f` | police, pursuits, heat | new `police.rs`, `pursuit.rs`, `mw-assets/src/police.rs`; edits in `ai.rs`, `main.rs`, mwtool (a stray `build.log` to delete) |
| `agent-a7e84698a6ee37b0c` | speed cameras, billboards, gates, Speed Points | new `mw-assets/src/scoring.rs`; edits in `freeroam.rs`, `hud.rs`, `props.rs`, `frontend.rs`, `mw-formats/src/world.rs` |
| `agent-a63370b2e36b9cd8f` | night lighting (world lights, lit windows) | new `world_lights.rs`, `mw-assets/src/lights.rs`; edits in `deferred.rs`, `game_lighting.wgsl`, `layered.wgsl`, `materials.rs`, `loader.rs` |
| `agent-a2ccbae6cc907f425` | jack spots, body shops | new `jackspots.rs`, `bodyshop.rs`; edits in `damage.rs`, `garage.rs`, `input.rs`, `hud.rs`, `easydrive.rs`, `frontend.rs`, `mw-assets/src/paint.rs` |

Expect merge conflicts in `main.rs`, `hud.rs`, `frontend.rs`, `mw-assets/src/lib.rs`, `mw-formats/src/world.rs`,
`genesys_names.txt` and mwtool's `main.rs`. The decompiled dump on the Windows machine is under the game
install, in `opencode_extract/NFS13_dump` (not `re/`).

## Open from this session

- The user: **"waterlevel is too low"** (place not given yet). At `water-river` ours shows mostly the river bed
  with caustics; needs a matched screenshot of the original there (method in the research log).
- Matched pair downtown: the wet road's reflection is a bright tan band at grazing angles (the original's is
  dark grey); brick paving bright and dry-looking; no lit windows / lamps (the night-lighting worktree).
- The original's time of day is not located in memory, so pairs cannot be made at the same hour yet.
- `ref/win-live1/handling.watch` lacks the wheel contact / spin / slip columns (copy the Linux machine's file).
- The default `--drive` spawn knocks a street lamp over; `--test crash-ai` needs `--spawn=925,23,-101,10`.
- A visual audit agent was stopped before reporting; its screenshots are in the session scratchpad only.
- The user's original-game save was rewritten by the game at 01:26 during the driven run; the copy from before
  is in `ref/savebackup-2026-10-10/` (not restored).
- Not started: map / car-select screens, Most Wanted list, streaming, traffic manoeuvres.

# Handoff: end of 2026-10-09 (session 6, part 3): start here

**State**: master (c11c9db and later) builds; clippy 0 warnings, 140 tests, `mwtool verify` 0 failures; the 911's
accel / brake / corner / drift results are unchanged from the morning (landing deepest 0.104 m and climb
settle 4.7 s changed with the kerb fix, by the game's rule). Every system below has its own decomp.md section and a
dated research-log entry (newest first) with sources, model decisions and next steps. **Ask the user how it
plays now**: since their last play test the FOV, the start-up flow, traffic density, crashes, kerbs and frame
times all changed.

## Landed today (after part 2 below)

- **Fixes from the user's play test**: engine pitch range per bank (was pinned at the top of each gear);
  floating grass (only the camera zone's + neighbours' groundcover; sprite size 2^e) and camera-facing sprites
  (the game's vertex program); checkpoint markers (3D UI panels of `PointOfInterestVisuals`) and the minimap
  (+Z forward, the game's icons); nitrous refill (traced: starts empty, per-event awards); surfaces (bumps,
  grip fade, drift factor); dirt / dust build-up and scratches; props on by default (game's contact response)
  and compounds breaking into parts; the white flashbang (camera shake spring stepped per frame); traffic
  and AI engine sounds; horns, traffic crash / skid sounds, AI impact slots; car badges / decals alpha
  (1-bit cut-out at 0.5, `_Alpha_` parts blended); the Jaguar's damage (the game's additive damage-clip blend,
  30 cars affected); kerbs (the strut stop gives over the tyre sidewall).
- **Traffic density** is the event's (free roam 1.0, was a fixed 4.78), matching the snapshots' counts; hard
  hits on traffic / AI cars **crash** by the game's car-against-car rules (`car_impacts.rs`).
- **FOV is horizontal** everywhere (captured from the original: all our cameras were ~1.8× too wide); the race
  intro's upside-down / rolled shots are correct (the original shows them too); the outro camera stays fixed.
- **Start-up flow** from the game's HSM data (`frontend.rs`, `pause.rs`, `mw_formats::hsm`): boot, title (movies
  via `ffmpeg`), the loading transition and in-game loading screen, the free-roam start (Aston V12 Vantage at
  trigger 1156487, or the profile's jack spot), pause menu (Esc / Start). `mostrusted` with no mode flag now
  starts the front end; `--drive`, `--race`, `--test`, `--camera`, `--replay` skip it.
- **Performance**: direct draws instead of Bevy's multi-draw-indirect path (render thread 2–3× cheaper
  downtown; `MW_INDIRECT=1` for A/B), avian's tree optimisation inline, environment bakes on their own 2
  threads, `MW_FRAME_LOG=<file>`, `--bench` prints p95 / p99 / hitches.

## Next, in order (each has details in its research-log entry)

1. The user's verdict on the above, then their next reports.
2. Performance: fewer draws (the game's model LOD / draw distances; 3,465 visible prop meshes downtown at full
   detail), the 2/60 s game-time cap per frame (measure with the handling tests), pre-warm the race start's
   pipelines, the sky environment map re-filtered every frame. Then M4.5 streaming around the player.
3. Kerbs part 2: the wheel as a point mass with the game's spring / damper (`FUN_0092f510`, `FUN_0092f160`), a
   like-for-like run against `ref/live1`'s kerb drop-off at 66 s.
4. Front end: the map screen 899991 (event selection), car select / customise (1571371, 1135879), the
   new-game intro event 1560354, event-loading cards, the transition's sounds.
5. Open model decisions worth a capture of the original: checkpoint marker orientation / size, burnout smoke
   density, traffic / AI sound levels against the player's engine, dirt colour, landing sink.
6. Smaller: the player's horn, scrapes, parked cars (static records decoded), traffic manoeuvres, the damaged
   prop models, opponents' own cars and names, the AI racing line.

## Housekeeping

- The day's agent worktrees were removed (all merged).
- /tmp is a RAM tmpfs: big artefacts go into a worktree, not the scratchpad (memory note).
- The user's original-game save changed during the intro capture (SP from races); a backup from the start of
  that run is in the git-ignored `ref/savebackup-2026-10-09/`.

New commands of part 3 (also in CLAUDE.md now):
```sh
./target/release/mostrusted                                   # front end: title, loading, free roam (--new-game, --profile <f>)
./target/release/mostrusted --drive 911 --test crash-traffic  # head-on with traffic; --test crash-ai --ai 911 --no-traffic
./target/release/mostrusted --drive 911 --test kerb           # MW_TEST_KERB_H / _DEPTH / _ANGLE, MW_TEST_KMH
./target/release/mostrusted --drive 911 --test smash --smash-set compounds
./target/release/mostrusted --drive xkr --damage all=0.5      # start damaged; mwtool damage <car> --keys
./target/release/mwtool hsm [id] [--find text]                # game-logic state machines; mwtool frontend
./target/release/mwtool traffic-sound sfx out.wav             # horns / impacts / skids offline
./target/release/mwtool surfaces --near x,z --steps           # kerbs and steps near a point
# --front-end-shots <dir>, --show-loading, --show-pause, MW_FRAME_LOG=<f>, MW_INDIRECT=1, MW_AVIAN_ASYNC=1,
# MW_BAKE_THREADS=0, MW_FOV_VERTICAL=1, MW_GATE_LOG=1
```

# Handoff: 2026-10-09 (session 6, part 2): races, AI, traffic, exhaust, race UI, cutscenes

Six worktree agents, merged into master and checked together (clippy 0 warnings, 118 tests, `mwtool verify` 0
failures, the 911's accel / brake / corner / drift results unchanged). Details and sources are in each
system's decomp.md section and its dated research-log entry.

- **Races** (`race.rs`, `mw_assets::events`, `chevron.rs`): 111 offline events from `GAMELOGIC/GAMEMODES.BNDL`
  with checkpoints and starts from `HAWAII/TRIGGERS.BNDL`; gates, laps, ranking, wrong way / missed checkpoint,
  win rules; the route chevrons. Offline races have **no 3-2-1**: the intro hands over a rolling start.
- **AI racers** (`ai.rs`, `ai_driver.rs`, `mw_formats::ai_data`): every car runs the same physics from
  `CarControls`; the player's car carries `Player` (filter on it). Driving law, rubber band and recovery from
  the code; the route is the `AIData` road graph's centre line (not the racing line).
- **Traffic** (`traffic.rs`): population, lane following, junction lights, queues, the fuzzy option scores; a hit
  car turns into a loose body. No manoeuvres, parked cars or per-car lights yet.
- **Exhaust** (`exhaust.rs`): the game's pop / backfire rule and sounds; spot effects at the tail-pipe locators
  (nitrous flame, backfire, idle vapour); several Bear particle actions decoded.
- **Race UI** (`hud.rs`, `ui_quads.rs`): UI blend states (the nitrous "dark box" was an additive flare drawn
  alpha-over), the offline race HUD (screen 604153), countdown / results sequences (online stand-ins),
  RACES in Easydrive starts a race at runtime.
- **Cutscenes** (`cutscene.rs`, `mw_assets::sequence`, `mw_formats::camera_take`): each event's intro and the
  mode's outro, ICE camera takes, teleports, slow motion, letterbox, skip (Esc / pad Back).

Most important open items: **a capture of the original's race intro** (several intro shots come out rolled
~40°; the camera-clip axis convention is a model decision, decomp.md "Cutscenes and sequences") and of its
outro (ours frames an opponent top-down); the opponents' own cars and names; the AI racing line; traffic
manoeuvres; the minimap arrow.

Commands (not yet in CLAUDE.md, which has uncommitted local edits):
```sh
# round 2 (user reports): engine pitch range, floating grass, checkpoint markers + minimap, nitrous refill,
# surfaces, dirt / scratches, props on by default, white flashes, traffic / AI sounds
./target/release/mostrusted --drive 911 --test-track --track-surface 4 --test accel   # the test track as grass (any surface index)
./target/release/mwtool surfaces [--near x,z[,r]] [--find <index>]   # surfaces, build-up columns, where a surface is
./target/release/mostrusted --race 462174 --no-cutscenes --spawn=-531.5,58,2113.3,144.3   # look at a gate marker
./target/release/mostrusted --drive 911 --test smash --smash-speed 60     # props: speed lost per prop (now on by default)
./target/release/mwtool traffic-sound all out.wav --passby 40   # traffic engine banks; engine-sound <car> --competitor
# --no-prop-physics (replaces --prop-physics), R washes the car, --dirt sets the start levels
# MW_NITROUS_LOG=1, MW_DIRT_LOG=1, MW_FLASH_LOG=1 [MW_FLASH_SHOTS=dir], MW_SOUND_TRACE=<f>, MW_AUDIO_NO_FX=1,
# MW_GATE_LOG=1, MW_GC_E=<e>, MW_AUDIO_METER=1 (now lists other cars' voices)
./target/release/mwtool events [<id> [--png f]] [--all]       # offline events, a route on the map
./target/release/mwtool camera-take [take] [--step s]          # ICE camera takes
./target/release/mwtool traffic --at x,z                       # lanes near a point (plain: junction cycles, data)
./target/release/mostrusted --race 462174                      # CONTINENTAL DRIFT: intro, 8 AI, traffic
./target/release/mostrusted --race 462174 --test race --units 12   # TEST RESULT race … (--no-cutscenes: frozen start)
MW_CUTSCENE_SHOT=intro:9 ./target/release/mostrusted --race 462174 --units 12 --screenshot f.png --screenshot-delay 100000
./target/release/mostrusted --drive 911 --ai "911,veyron super" --test ai --ai-seconds 60
./target/release/mostrusted --drive 911 --test exhaust         # pops / backfires; --nitrous holds nitrous
./target/release/mostrusted --drive xkr --units 12 --spawn=-600,52,2300 --show-easydrive --easydrive-path 2,0,1
# --no-traffic, --traffic-density x, --no-cutscenes, MW_TRAFFIC_LOG=1, MW_EXHAUST_LOG=1, MW_RACE_TEXT=1
```

# Handoff: 2026-10-09 (session 6), on top of session 5 below

- **Angular damping traced** (commit 0102e9b): a driving car gets no general angular damping in the original
  (its `DrivingVehicle` damping constraint is empty, read live); only the drift's and a post-collision yaw
  damping (`FUN_008f7950`, now ported as `ImpactYawDamping`). The chassis inertia (a box, × `Inertia_Scale`) and
  the wheel spin inertias were read live and equal ours; the tyre blend-input scale is 1 (settled).
- **Our `AngularDamping(0.4)` stays as a listed model decision** (`MW_ANGULAR_DAMPING=<v>` for A/B): four clean
  brake-tap windows of `ref/live1` match better with it, so the original resists yaw in a slide through
  something in the tyre / chassis model we have not found. What is ruled out and the per-window numbers are in
  the research log (2026-10-09). Best next step: a **clean capture** of a slide on a flat, open road (tools/orig)
  with a second car, then the residual-yaw-moment comparison from that entry.
- **Replay harness fixed** (65e2132): the game's speed definition, a full-throttle approach to the start speed,
  the recording's starting gear. Earlier replay comparisons (session 5 numbers) used the forward-speed
  column and may read worse than they were. Windows of `ref/live1` must be cut at the first `w*_contact` = 0
  (the city has kerbs and seams; our replay runs on the flat track).

# Handoff: end of 2026-10-06 (session 5)

**State**: master builds and passes clippy, the tests and `mwtool verify`; the user was about to test it by
hand (controls: arrows, A / Z, Space, Left Shift nitrous; E / numpad 6 / d-pad right opens Easydrive).
Their verdict on the camera, the handling after the strut-stop / anti-roll / gearbox changes, tyre marks, water
and the UI is the first thing to ask for.

Read this first. Method as before (CLAUDE.md "Fidelity rule"): the original is the reference, behaviour comes
from the decompiled code ([decomp.md](decomp.md)), every change names its source. This session re-read nine
systems from the dump in parallel; the session-5 entries at the top of [research-log.md](research-log.md) hold
the details, [PLAN.md](PLAN.md) "Where it stands" the state per gap.

## What landed

- **Handling**: the tyre pass's brake hold (`FUN_009bc0c0`: the leftover brake acts on the chassis without
  turning the wheel), the standstill hold (`Brakes.99407bbc` through `FUN_009d6170` / `FUN_009b1180`) and the
  downshift's ground-time gate (`FUN_009ae840`). 911: 0–100 km/h 3.72 s, 100–0 in 38.5 m, skidpad 2.57 g
  (unchanged). `ref/xkr_launch`: RMS speed 15.1 mph; launch slip now close (κ 0.4 / 0.49 against 0.1 / 0.39),
  ours pulls away ~0.3 s early and revs to 6630 against 6120. **The run's braking phase (after 6.0 s) cannot
  judge brakes or tyres**: the original car hits something twice (6.02 s: −7.4 m/s in one tick; 6.60 s: the
  velocity turns 34° in one tick) and then slides; our replay runs on the flat test track. Between the hits it
  brakes at 1.13 g (ours 1.1–1.6 g). A clean straight-line braking capture is needed.
- **Steering's neutral offset** (round 2): q (vehicle +0x2c70, the `steer_q` column) = body slip / 35°, scaled by
  speed / 10 mph below 10 mph: outside a drift the front wheels turn towards the direction of travel by 0.3 q,
  a counter-steer assist. Ported from the telemetry relation (its writer was not found in the dump; 35° is also
  the XKR's `Maximum_steering_angle`, so a second car's capture must tell a constant from that field).
- **Input**: the original's default bindings and mapping rule (`input.rs`). W / S no longer drive: arrows, A, Z.
- **Easydrive**: menu tree from the UI data, shown in game; Change car works, the rest is inert, the look is ours.
- **Free-roam**: `TriggerData` decoded (`mwtool freeroam`); speed cameras log passes (`freeroam.rs`).
- **Traffic**: `TrafficLaneData` decoded (`mwtool traffic`), lanes queryable by position. No runtime.
- **Rendering**: the day sky's dark patches (cloud textures decoded as sRGB) fixed; three road decal shader
  variants drawn as the game does; game-font text no longer draws under its own panel (glyph layout ran after
  Bevy's UI stack system: this was also the HUD's "faint digits").
- **Vignette ported** (`tonemap.rs`, `MW_NO_VIGNETTE=1`; decomp.md "Post-processing"): A·v + D from the
  default PostFX keyframe's Vignette, the gain k = 1.8245 a live read whose source is open, the small live
  stretch s of A and D (1.0025…1.0385) not modelled.
- **Bloom ported** (2026-10-07, `bloom.wgsl` run from tonemap.rs, `MW_NO_BLOOM=1`; decomp.md
  "Post-processing", "Bloom passes (ported)"): the exe's downsample / blur / add programs and the composite
  term, with the time-blended `EnvironmentKeyframe.Camera` values. Model decisions: level sizes 1/4, 1/8, 1/16
  of the screen (not traced), the vignette weight per pixel instead of in the bloom alpha, bloom always on in
  free roam (the live flags were 1). No capture of the original to compare. Left in post: the colour-cube
  blend (slot weights not read yet); the sun direction's writer and the shadow cascade fit were not found.
- **The user's two visual reports** (road / terrain texture scale; foliage in the wrong place), state:
  - Texture scale: UV maths of every ground family, mip chains, UV formats and the detail-map strength match the
    game; three road decal variants and the anisotropy (8× → the game's 4×, `SamplerState` decoded, `mwtool
    sampler-usage`) were fixed. **Not resolved.** One lead: texture header byte 0x2C is used as the first mip to
    draw when the global flag at 0x010eac98 is set (texture quality?): read it live and take an F12 capture of
    the original at the `road-close` viewpoint. Open: per-slot samplers (the −0.9 LOD bias slots).
  - Foliage: no decode error in any layer (`mwtool placement-check`). Groundcover now uses the game's field
    meanings (size exponent, signed width / height) and its group draw distance (125 × the group's range);
    sprite size in metres, per-sprite range and shrink, and camera-facing sprites are open. **The likely cause
    is fog** (found at the end of the session): at noon the keyframe's fog reads 80–500 m, so hills beyond
    500 m are flat fog colour while forward-rendered trees get no fog and hang in front of them
    (`--camera=-650,260,1850,-1000,60,2150 --hour 12`). An agent was working on the game's fog rule and fog on
    forward materials when this was written: check the research log's round-3 entry and whether it merged.
- **Rules traced, no runtime yet** (round 2, decomp.md): traffic population (active cells, emitters at lane
  starts, `Density` = cars per minute per lane × 4.78, the 2 s spawn gap, the load budget of 200) and free-roam
  trigger hits (full-extent boxes, fire on entry, one-way boxes, one hit handled every 4 s; billboards count
  when their prop breaks). Not found: Speed Points, the jack-spot swap, junction behaviour, the switch from a
  lane follower to a physical car.

## Round 3 (evening): the user's later reports and requests

- **Cars sinking into the floor** after landings and steep climbs: fixed. The game makes the end of the strut a
  hard contact on the chassis (`FUN_009c5a90` → `FUN_009c2670`); ported as `strut_stops` in drive.rs. New
  tests: `--test landing | climb | jitter` (`MW_SUSP_TRACE=<file>` logs every tick). 911: landing sink 0.055 m
  (was stuck 0.48 m low), climb ends on the plateau. Model decisions: stop slack 0 (`.bss` 0x01186bf0 /
  0x01186b10), 4 passes, the wheel ray starting one radius above the mount.
- **Jitter**: measured, not fixed: the body rolls ±0.2° at 11.2 Hz at 100 km/h (0.06° at rest). It is the
  anti-roll bar's damper (previous tick's compression rates at 1.2 × critical) ringing; with that damper off
  the roll drops to 0.008°. The bar's units and where the game applies it are not traced: do that first.
- **"Stuck in a gear"**: the game switches to each car's `CruisingGearbox` after 3 s of part throttle
  (`FUN_009ce630`, `FUN_00900660`) and back above 0.95 pedal; ported. Open: 1st gear still runs to ~0.95 × the
  race `Gear_up_RPM` at part throttle (the launch state closes the clutch late; our partly engaged clutch is
  nearly rigid: port the slip in `FUN_009d7ac0`); the sound side was not looked at.
- **Fog / floating trees**: Bevy draws `AlphaMode::Mask` materials a second time in its forward pass even when
  they are deferred, so trees and every 1-bit look were lit and fogged, then painted over without fog. The
  forward branch of `layered.wgsl` now discards in that case. Open: hills beyond 500 m are still flat fog
  colour (the Fog pass multiplies t by the alpha of a gradient texture whose builder was not found; ours uses
  alpha 1), the height-layer term (`LayerFogCoeffs`), fog on forward glass / particles.
- **Liveries**: drawn (`VehicleNFS13_BodyPaint_Livery*`: the car's own diffuse texture over the paint by its
  alpha). **Dirt does not work**: the maths is ported behind `--dirt <dirt>[,<dust>]` but the colours come out
  pink / cyan (the paint's `DirtColour` / `DustColour` are the expected brown / beige, so the fault is in our
  path). Open: that, the second paint colour of two-tone cars, who sets the dirt amounts, scratches.
- **Nitrous**: `FUN_009d2aa0` ported (`crates/mw-handling/src/nitrous.rs`; Left Shift / pad A; the HUD gauge
  follows the tank; `--test nitrous`: 911 0–100 km/h 3.47 s against 3.72 s). Model decisions: the multiplier
  scales engine torque (its reader was not found), tank capacity 100, starts full, no refill (the refill events
  are in `NitrousParameters`, the tank code is not traced); punch / jump modes need `.bss` 0x0118a018.
- **Mods**: the upgrade profile (`FUN_00574870`: 50 entries of operand, op, parameter index; value × Π + Σ) and
  the data (`Vehicles.PerformanceUpgrades`; `mwtool mods <car>`) are decoded; **nothing applies them yet**.
  Next: the parameter index → handling field map (call sites pass 0x00–0x36), then `--mods`.
- **Active aero**: `AnimationStateBehaviour` (an `HSMData` state machine + `Animation` clips on the damage
  skeleton). Cars whose rule is one clip on speed alone run (911, LFA, SLS, SL65, Panamera, 918: deploy at the
  state's speed, e.g. 70 mph, retract below the lower operand); `--anim all=1` poses any car's parts in
  `--drive`. Open: decoding the state machine layout (air brakes on `brake`, flaps on `steering`), the
  evaluator code, whether a deployed wing changes the aero.
- At the default spawn the car loses ~18 km/h at z ≈ 1716 with `--auto-throttle`: it runs into a barrier there
  (user, 2026-10-06); not a physics fault. Use `--test-track` or another `--spawn` for speed logs.

## Rounds 4–5 (late evening): UI, mods, tyre marks, performance, camera, water

- **Easydrive** is drawn from the game's own layout (UI element stacks, `mw_assets::ui`; the menu layout
  1773380): the white selected bar, translucent rows, stacked tier titles. Open: the row spacing rule, the
  glow / slide animations, the closed-menu prompt and the side panel, conditional entries.
- **Mods through Easydrive** (`mods_menu.rs`, pages in `easydrive.rs`): Customize car → class → mod → standard
  / PRO, applied at once; `--mods slot=level`; `--easydrive-path 4,4,1,1` walks the menu headlessly. The slot
  field names are cracked (`Body1`/`2`, `Chassis1`–`3`, `Nitrous1`–`4`, `PowerTrain1`/`2`, `Stock*`). 25 of the
  55 parameter indices of `FUN_00574870` are identified (engine torque scale, gearing, steering, drift); the
  rest (4–19, 22–25, 29, 30, 33, 41, 48–54: tyres, chassis, body) are not, so those mods do little yet.
  `TyreUpgrade` is not applied. Class labels are ours; all mods are unlocked (no events yet).
- **Tyre marks** (`tyre_marks.rs`, `tyre_marks.wgsl`: the `Vfx_TyreMarksNew` port with the surface's
  `TyreMarksMaterials` texture). The rule for when a mark is laid was not found in the dump: strength follows
  the skid-sound intensities (model decision), 0.65 m wide (too wide), 60 s fade.
- **Jitter**: fixed. The anti-roll bar's damping is 2ζ√(k·I) with k before the ×60 scale (`FUN_00933a60`), i.e.
  ~0.15 of critical; ours was 7.7× more and rang at 11 Hz. The per-tick solve that applies the bar has no body
  in the dump (`FUN_00932af0`), so the bar's input and integration are still ours.
- **Dirt**: colours fixed (a flag reset in `Look::retain_textures` made the shader read the effects texture as
  specular); off unless `--dirt`. The build-up rule is not traced (`FUN_008e1280`'s callers `FUN_008b7a20`,
  `FUN_008cffa0`). **Two-tone paint** from `VehicleColourPalette` (primary / secondary pairs, `FUN_008e9c00`);
  which pair the game picks by default is open.
- **Water**: `Water_Opaque_Singlesided` ported (floor and caustics on the third UV set, foam, the game's
  reflection term); `MW_WATER_DEBUG=1` lists the water meshes; viewpoints `water-river`, `water-pond`. Open: the
  material's reflection cube (a mountain silhouette), the lighting LUT's clamp, `PlotPBR_AO_StandingWater`.
- **Performance**: `--bench <s>` (fixed viewpoints in the research log), `--no-shadows`, `MW_BENCH_PASSES=1`.
  Quiet machine, RTX 3070, 1280×720: driving downtown 19.7 ms (51 fps) with shadows, 13.1 ms (76 fps) without;
  the cliff view sits at a 60 fps cap and cannot show gains. CPU-bound in Bevy's per-view batching; the loaded
  map's size costs nothing. Merging static meshes per material and unit gave no measurable gain (few materials
  are shared) and is off by default (`--merge-static`). **Shadows** (≈5 ms of render-world time while driving): the cost is CPU work per
  caster per cascade view, mostly the outer cascade's opaque world placements; shadow-map size and a
  texel-size cull change nothing. Default-on since round 6 (`shadow_cull.rs`): a caster is dropped from a
  cascade when its outline from the sun misses the camera view's outline (outer cascade 1586 → 853 casters,
  ≈0.8 ms; `MW_SHADOW_FOOTPRINT=0` turns it off). Behind flags because they change the picture:
  `MW_SHADOW_MODEL_RANGE=1` (each model's own shadow distance, `Model` +0x0C: trace it in the code, then make
  it the default), `MW_SHADOW_DISTANCE=<m>`. Next: lift the bench's 60 fps cap, LOD selection by the models'
  distances (`VisibilityRange`).
- **Chase camera** (the user: it lacks the original's sway, weight and avoidance): four passes. The update is
  `FUN_005d4970` (decomp.md "## Cameras": director table, the data read through the state at camera +0x290,
  call order). **From the code now**: the followed frame (the car's Euler angles eased per axis, no lag cap),
  the speed band, the eye / look-at heights, and the distance as a first-order lag (targets and rates from the
  data; the braking offset is 0.5 m, ours was 1.5 m). **Still ours**: the yaw spring's value (two unresolved
  inputs in `FUN_005c90a0`), the vertical stage at cam +0x554, roll, pitch, the bob (`FUN_005c8880`), the
  gradient spring (`FUN_005c7a80`), speed movement / impact shake (`FUN_005d0b90`), FOV, and the collision
  push-in. **Avoidance was not found**: no world query exists in the decompiled camera code; the director's
  update `FUN_005d5e20` is not a function in the dump (disassemble it through the probe). Live values needed:
  `DAT_0110e770`, `0110e4d0`, `0110e680`, `0110e610`, `0110e640`, `0110e8f0`, `0118a060`, and state +0x44 /
  +0x64 / +0x70 / +0x74 / +0x94. The user has not judged the result yet.
- **Groundcover** tufts are lit and textured (they were black: Bevy's double-sided material flipped the ground
  normal on back faces; `Mode::Groundcover` with the game's albedo, atlas cell and normal). Open: the sprite
  size (a load-time write into the collection's material record), camera-facing sprites, per-sprite range.

## Live reads wanted (the original running; `sudo sysctl kernel.yama.ptrace_scope=0` first, ask the user)

`mwtool memscan`: `dump NFS13.exe+0x<addr − 0x400000>` for `.bss`; object fields through `handling.watch`.

| What | Where | Used for (assumed today) |
|---|---|---|
| slip-velocity term | `.bss` 0x01186b74 | y term in `FUN_009bc0c0` (not modelled; gravity?) |
| unidentified | `.bss` 0x01186630, 0x01186f60 | named in the drivetrain entry of the research log |
| tyre blend-input scale | wheel constraint +0x210 | pure → combined slip blends (1; 0 if only the builder writes it) |
| forced brake mask | controls +0x1c30 during a launch | the 0.45 s launch delay (not modelled) |
| bloom | view settings +0x1518, renderer +0x9836 (flag), +0xac74…+0xaca4 (set) | whether bloom is on in free roam and its values |
| vignette | renderer +0xa980 (D), +0xa990 (A) | composite inner / outer colours |
| colour-cube blend | frame state +0x2c…+0x78 (seven slots, six weights) | which slot holds the environment cube |
| pad dead zones | pad object +0x140 / +0x144 | 0.2 / 0.1 from `.rdata` defaults |
| traffic parameters | `DAT_0110f940` / `f944` (object; field 0x48), `DAT_0110f590` (cull radius), `DAT_0118a090`, world +0xa2bfc, world +0xa2b74 at several hours | traffic speed scale, cull radius, the hour's density |
| texture first-mip flag | settled 2026-10-06: +0x113 = 1 means the top mip is **kept** (`FUN_00409510` passes 1 only when the byte is 0) | nothing: our full-resolution loading matches |
| groundcover size | settled 2026-10-06: `cb1[0].x` = 4 live, size = byte / 127 × 4 (in code) | settled 2026-10-09: 2^e from the draw loop; camera-facing sprites ported (`groundcover.wgsl`) |
| free-roam state | game-state bytes +0x2e90, +0x2e91, +0x2e95 | gates on the collectable handler |
| fog | 0x23 settled (not scaled; ours equals live); 0x83 formula in decomp.md "Fog" | `LayerFog` blend weights (+0x22d8 / +0x22dc) and density factor (+0x2df4 / +0x2df8); the gradient texture's builder and alpha; the original fades between painted timelines over ~10 s (we switch) |
| sun path elevation | slot 0x0d over 20 dumps: 49.9° falling to 49.5° between ~12:13 and ~12:34 | the arc peaks near 12:00, not 13:00 (our sunrise 5 / sunset 21 are assumed) |
| strut stop slack | `.bss` 0x01186bf0, 0x01186b10 | the hard stop's slack (0) |

## Next, in order

1. The live-read session above; then the cube blend (bloom and the vignette are ported; kernels and sources
   are in decomp.md "Post-processing and colour grade"). For bloom, read the level target sizes live: the
   struct `*(renderer + 0x220)` (+0x254 in another mode) holds them at +0x130…+0x148 (width, height at the
   texture's +0, +4), and compare a frame with the original at a known place and hour.
2. Handling: new reference runs on a clear road (tools/orig): **straight-line braking** first (the original's
   rear wheels show κ ≈ 0…+0.07 under full brake in the clean stretch, ours −0.1…−0.3: the brake split), a
   slide in a second car (settles q's 35°), handbrake turn, steady corner, donut; the +0x210 leads
   (`FUN_009c24b0` / `FUN_009a56b0`); the launch delay.
3. The user's visual reports with a location: WorldPBR `Horizontal_*` variants, sampler address modes, the
   game's groundcover draw distance (`GroundcoverJob`); camera-facing groundcover done 2026-10-09.
4. Easydrive's own layouts and conditional entries; nitrous, horn, look back; free-roam HUD messages and
   Speed Points; the traffic runtime.

## Working with parallel agents (learned the hard way this session)

- Do **not** hard-link one `target/` into several worktrees: cargo rewrites shared files in place, every build
  invalidates the others and each worktree rebuilds Bevy (load average 110). Give each worktree a real copy of
  one finished dependency build (`.claude/tools/copy-deps.sh <src worktree> <dst worktree>`, 4 GB each).
- Every agent builds with the same package set (`cargo build --release -p mostrusted -p mwtool`); other sets
  resolve Bevy's features differently and rebuild it. Agents do not run clippy / tests / `--workspace`; the
  integrator runs them once on the merged tree.
- Cap parallelism (`.claude/worktrees/.cargo/config.toml`, `[build] jobs`), at most ~4 building agents at once.
- Worktrees may start from an older commit than HEAD: tell agents to `git merge --ff-only master` first.
- Helpers for the dump (git-ignored `.claude/tools/`): `fn <addr>` prints a function, `callers <addr>`,
  `exe-const <va> [n] [f32|u32|str]` reads `.rdata` / `.data` constants from the exe on disk.

## Before committing

`cargo fmt`, `cargo clippy --release --workspace` (0 warnings), `cargo test --release --workspace`,
`./target/release/mwtool verify` (0 failures). Never commit game data, derived files or decompiled code
(`ref/`, `consts.txt`, CSVs, wavs, screenshots stay out; `ref/`, `*.csv`, `*.watch` are git-ignored).

## Tools (unchanged from session 3)

- **Drive the original**: `tools/orig/orig launch | shot | key | hold | click | read | run <script> | quit`
  (headless gamescope through Steam). Title screen: `key Return` twice (offline prompt). For keyboard driving,
  Settings > Gameplay > Controller must be "Keyboard" (Esc, Down ×5, Return, Down ×3, Return, Down ×3, Left);
  switch it back to the Xbox controller (Right) before quitting and leave Settings with Esc so it saves.
- **Reference runs**: `mwtool capture <name>`, `mostrusted --drive <car> --replay ref/<name>/telemetry.csv`,
  `mwtool compare`. Copy the run to a scratch dir when several replays may run at once (the replay writes
  `telemetry.ours.csv` next to its input).
- **Live tuning / pokes**: `mwtool tune`, `mwtool memscan poke` ([probe.md](probe.md)). The probe's
  function-patching step (Tier 1) is still not written.
