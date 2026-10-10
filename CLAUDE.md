# Most Rusted: working notes

Rust reimplementation of NFS Most Wanted (2012) that runs on the original assets, read at runtime from the
user's install (IW4L-style). Read these before starting work:

- [docs/PLAN.md](docs/PLAN.md): milestones and what to do next. Update the checkboxes when things land.
- [docs/research-log.md](docs/research-log.md): open format questions and dated findings. Add to it whenever
  something new is learned, including dead ends.
- [docs/formats.md](docs/formats.md): settled format knowledge.
- [docs/reverse-engineering.md](docs/reverse-engineering.md): why the exe can't be statically analysed, and the
  rules for the user's decompiled dump in `re/`.
- [docs/decomp.md](docs/decomp.md): what the game's code says (handling maths, update loop), from that dump.
- [docs/handling.md](docs/handling.md): GeneSys handling field mappings, model constants, test results.
- [docs/probe.md](docs/probe.md): code inside the running original (measurement hooks, pokes, in-place
  verification of ports); its policy is in reverse-engineering.md "Code inside the running game".
- [docs/next-session.md](docs/next-session.md): handoff from the last session (if present): start there.

## Hard rules

- **Never commit game data or anything derived from it**: extracted resources, DDS/PNG/OBJ exports,
  Ghidra databases, decompiled code (findings go into docs in prose only). Use the session scratchpad or git-ignored dirs (`/extracted`, `/re/out`).
- Community references (burnout.wiki, DGIorio's MIT tools) may be read for format knowledge; don't copy code.
- **Modding the original is allowed** (docs/reverse-engineering.md): code mods run in the
  user's own copy, offline. Data files may be replaced if backed up and restorable. Byte signatures and anything from the dump stay in git-ignored `re/`.

## Fidelity rule: the original is the reference

The goal is to play and look exactly like the original, not like a plausible racing game. Reverse engineering
the original's inner workings (physics, rendering, audio, AI, rules) to get there is allowed and preferred:
the dump, shader disassembly, the probe, memory reads and frame captures (docs/reverse-engineering.md).
**How vehicles drive is read from the decompiled code**, not guessed or
calibrated from data: trace each rule and constant to its function (`.rdata` constants from the exe on disk,
`.bss` ones from their initialiser or a live read). A value becomes a model decision only after a documented,
failed search of the code. Every behaviour or look change names its source: a data field, a function in the decomp ([docs/decomp.md](docs/decomp.md)), a
constant or trace read from the running game, or a capture of the original (`mwtool capture`, Steam F12
screenshots of the original at a known place and hour, the game's own audio stream). No tuning by feel: a model
value that is not from one of those is marked as such (handling.md "Model decisions") and replaced when it can be
measured. For handling: record the original (`mwtool capture <name>`), replay the same inputs
(`mostrusted --drive <car> --replay ref/<name>/telemetry.csv`) and compare (`mwtool compare`) before and after a
change. Reference captures live in the git-ignored `ref/`.

## Layout

```
crates/mw-gamedir   install detection, case-insensitive paths      (no deps on other crates)
crates/mw-bundle    bnd2 v5 reader, ResourceId, ResourceType, Reader
crates/mw-formats   per-type parsers; take &mw_bundle::Resource     (never depends on Bevy)
crates/mw-audio     EA audio decoders (SPS/EALayer3, Ginsu/XAS) and the engine-sound synth
crates/mw-assets    AssetStore: index over all bundles, cross-bundle loads, scene helpers
crates/mw-handling  the game's handling maths, engine-free (tyres, drivetrain, drift, nitrous, surfaces)
crates/mwtool       CLI for inspection / extraction / previews
crates/mostrusted   Bevy 0.19 runtime (loader thread → channel → ECS), avian3d 0.7 physics (drive.rs)
crates/mw-probe     DLL loaded into the running original (docs/probe.md; i686-pc-windows-gnu)
```

Game install on this machine: `~/.local/share/Steam/steamapps/common/Need for Speed(TM) Most Wanted`
(found automatically).

Windows hosts build and run the same workspace (MSVC toolchain). The install is found through Steam's registry
key and the default Steam / EA app folders; settings and the profile go to `%APPDATA%\mostrusted`. `mwtool
memscan` / `telemetry` / `tune` / `capture` read the game through `ReadProcessMemory` (`memscan/windows.rs`);
`capture` records no audio there (it uses pactl / parecord). The probe DLL builds with
`rustup target add i686-pc-windows-msvc` and `cargo build --release -p mw-probe --target i686-pc-windows-msvc`.
`tools/orig` (gamescope) is Linux-only.

## Commands

```sh
cargo build --release --workspace
cargo clippy --release --workspace          # keep at 0 warnings
cargo test --release --workspace
cargo fmt                                    # rustfmt.toml: max_width 120

./target/release/mwtool verify               # must report 0 failures
./target/release/mwtool list <bundle>        # bundle paths are relative to the game dir
./target/release/mwtool extract <bundle> <dir>
./target/release/mwtool models <bundle> <dir>      # writes preview.png (software render, viewable)
./target/release/mwtool world <dir> --bundle HAWAII/TRK_UNIT1.BNDL
./target/release/mwtool shaders                    # samplers, parameters, vertex layout per shader
./target/release/mwtool shader-asm DriveableSurface # DXBC disassembly of a shader's programs
./target/release/mwtool survey VehicleGraphicsSpec --len 0x60   # per-field stats across all instances
./target/release/mwtool vehicles                   # car catalogue (id, year, name, bundle)
./target/release/mwtool texture <id> out.png
./target/release/mwtool genesys VEHICLES/VEH_1085007_HI.BNDL --type HandlingBehaviour   # or: genesys all --type X
./target/release/mwtool handling 911                # typed handling parameters (docs/handling.md)
./target/release/mwtool genesys-types > types.tsv    # every GeneSys type and field (hash, offset, type)
./target/release/mwtool genesys-crack --words a.txt --max-words 4 --type Steering   # guess field names (check hits!)
#   fields are stored sorted by size then name: a real hit sorts between its neighbours (formats.md, GeneSys)
./target/release/mwtool shader-usage               # world triangles per shader (prioritise ports)
./target/release/mwtool audio UI/SONGS/116502.SPS <dir>   # or a bundle: its Ginsu/Wave resources -> .wav
./target/release/mwtool engine-sound 911 out.wav   # rev sweep through the engine synth (--trace: `seconds rpm load` lines)
# Crash damage (skeletal damage clips): inspect / preview a car, or drive into a wall and save before/after shots
./target/release/mwtool damage 911 --level front=0.5 --preview <scratch>/dmg.png
./target/release/mostrusted --drive 911 --test crash --screenshot <scratch>/crash.png
# Measuring the original game (read-only, Linux/Proton; docs/handling.md "Measuring the original")
./target/release/mwtool memscan --watch handling.watch        # interactive value scanner; `help`
./target/release/mwtool telemetry handling.watch -o run.csv --hz 120
#   poke / unpoke in memscan, or --poke NFS13.exe+0xd872a0=0.6 on telemetry / capture: change a data value for a run
./target/release/mwtool tune                  # live tuning menu next to the running game (docs/probe.md "Tuning menu");
#   pages of handling constants + the player car's fields, restored on quit; --print lists them, --apply loads a preset
# The probe DLL (docs/probe.md): loads into the running original for one run and logs to ref/probe/<name>/
cargo build --release -p mw-probe --target i686-pc-windows-gnu    # needs mingw-w64-gcc + the rustup target
tools/orig/orig launch --probe hello   # orig modules lists the game's DLLs; orig quit removes the copied DLL
# Drive the original yourself (tools/orig/orig --help): it runs headless in gamescope (always focused, input
# only through gamescope's display, screenshots via gamescopectl); one-time: `tools/orig/orig setup` and set the
# printed Steam launch option. Look at each screenshot before choosing the next input.
tools/orig/orig launch                 # start the game through Steam inside headless gamescope
tools/orig/orig shot <scratch>/o.png   # screenshot; key Return / hold w 3 / click x y / read speed_mph gear
tools/orig/orig run script.orig        # steps: key, hold, pad steer=0.5 throttle=1, waitfor speed_mph > 60, capture
tools/orig/orig quit
tools/orig/orig run-in -- ./target/release/mostrusted --drive 911 --test-track   # same harness on our game
# Reference runs: record the original (car state + its own audio, Enter stops), replay the inputs, compare
./target/release/mwtool capture corner1                     # -> ref/corner1/{telemetry.csv,audio.wav}
./target/release/mostrusted --drive 911 --replay ref/corner1/telemetry.csv   # -> telemetry.ours.csv
./target/release/mwtool compare ref/corner1/telemetry.csv ref/corner1/telemetry.ours.csv

# Visual check without a human: renders, saves, exits
./target/release/mostrusted --camera=-1131,27,1627,-1190,15,1720 --screenshot <scratch>/road.png   # see docs/viewpoints.md
./target/release/mostrusted --vehicle "911" --camera 4.5,1.4,4.0,0,0.4,0 --screenshot <scratch>/car.png
# Driving cameras: --camera-mode chase|bonnet|bumper|lookback (C / gamepad LB cycles; End or the right
# stick fully back looks back; right stick / right mouse drag looks around; the free camera looks with the
# right stick too). MW_CAMERA_TRACE=<file> logs the camera every frame
MW_CAMERA_TRACE=<scratch>/cam.trace ./target/release/mostrusted --drive 911 --test drift
./target/release/mostrusted --drive 911 --units 12 --auto-throttle 0.6 --camera-mode bonnet --screenshot <scratch>/bonnet.png --screenshot-delay 400
# Time of day and weather (weather.rs; interactive: F5/F6 hour, F7 weather). The timeline is the one painted
# under the camera (--timeline <id|default> overrides); --weather = the Alternate (bad-weather) keyframes.
# Roads are always wet as in the game (--road-wetness 0 dries them); --planar-reflection mirrors the scene in them
./target/release/mostrusted --camera=-1131,27,1627,-1190,15,1720 --units 20 --hour 1.5 --weather --planar-reflection --screenshot <scratch>/wet.png
# Post-processing (tonemap.rs): the original's vignette from the default PostFX keyframe; MW_NO_VIGNETTE=1 for A/B.
# Bloom (bloom.wgsl, colour / intensity / level weights from the environment keyframe): MW_NO_BLOOM=1 for A/B
MW_NO_VIGNETTE=1 ./target/release/mostrusted --camera=-1131,27,1627,-1190,15,1720 --units 20 --hour 13 --screenshot <scratch>/novig.png
./target/release/mwtool paint-map 010000000002823d --png <scratch>/timelines.png --at=-1158,1700   # WorldPaintMap + zone outlines
# Handling tests on a flat track (4x real time), prints TEST RESULT lines and exits
./target/release/mostrusted --drive 911 --test accel     # or brake, corner (skidpad g), handbrake, drift, engine,
#   nitrous, landing, climb, jitter (MW_SUSP_TRACE=<file> logs the suspension every tick)
./target/release/mostrusted --drive 911 --test smash --smash-speed 60 --screenshot <scratch>/smash.png   # props: cone, lamp, fence, wall
#   --smash-set compounds: lamps etc. breaking into parts. Props collide in the world by default (--no-prop-physics)
./target/release/mostrusted --drive 911 --test kerb        # kerb strike (MW_TEST_KERB_H / _DEPTH / _ANGLE, MW_TEST_KMH)
./target/release/mostrusted --drive 911 --test-track --track-surface 4 --test accel   # the flat track as any surface (4 = grass)
./target/release/mwtool props -v                    # physics of every prop / compound / dynamic object
./target/release/mwtool lods --at=815,60,1760          # model LOD tables / placement LOD scales; what the game draws from a point
#   runtime: MW_NO_LOD=1 (everything at LOD0, A/B), MW_LOD_SCALING=0|1|2 (the game's LodScaling option), MW_LOD_LOG=1
./target/release/mwtool surfaces                    # surfaces: grip, rolling, drag, dirt build-up, triangle counts
#   --near x,z[,r] (surfaces around a point), --find <index> (where a surface is), --steps (kerbs / steps nearby)
./target/release/mwtool ui screens                  # game UI: screens / widgets (ui show <screen>, ui render <screen> out.png, ui font <n> out.png, ui easydrive)
./target/release/mwtool traffic --png <scratch>/lanes.png   # TrafficLaneData: counts, flows, vehicle lists, lane map
./target/release/mwtool lights --near x,z[,r] --hour 21     # world lights (lists + street lamps): types, flags, ranges
#   night lighting (world_lights.rs): lamps and lit windows follow --hour; MW_NO_WORLD_LIGHTS=1 for A/B
./target/release/mwtool freeroam --list --kind SpeedCam     # TriggerData: speed cameras, billboards, gates, jack spots
#   --objects: each collectable's nearest placed object and its instance ids (billboard compound = trigger id)
./target/release/mwtool scoring                     # ScoringActions: Speed Points, message sequence / title (--all, --sequence <id>, --text <ids>)
# Collectables and Speed Points (freeroam.rs): a speed camera, a billboard and a gate; TEST RESULT freeroam ...
./target/release/mostrusted --drive 911 --units 12 --mute --no-traffic --test freeroam --profile <scratch>/p.cfg --screenshot <scratch>/fr.png
./target/release/mwtool placement-check --top 10             # placements vs the collision under them, per layer / model
./target/release/mwtool mods 911                             # performance upgrades: slot -> parameter changes
./target/release/mwtool sampler-usage --filter Driveable     # the game's sampler states per shader slot
# Car options: --dirt 0.6,0.3 (dirt, dust), --anim all=1 (pose animated parts in --drive), --mods 12=0 (slot=level
# as `mwtool mods` prints them; also through Easydrive: Customize car); nitrous is Left Shift / pad A
./target/release/mostrusted --drive 911 --units 6 --show-easydrive --easydrive-path 4,4,1,1 --screenshot <scratch>/mods.png --screenshot-delay 200
# Frame-time benchmark (prints a BENCH RESULT line; --no-shadows, MW_BENCH_PASSES=1 for per-pass GPU times)
./target/release/mostrusted --drive 911 --units 30 --spawn=815,60,1760,270 --auto-throttle 0.3 --mute --bench 10   # --merge-static: A/B the mesh merge
# MW_TYRE_MARKS_LOG=1 (tyre-mark segments alive), MW_WATER_DEBUG=1 (lists the water meshes)
# The driving HUD (hud.rs) is on in --drive runs except tests: --no-hud, --mph, F3 shows the debug status text
# Engine RPM every physics tick, for `mwtool engine-sound --trace` (use with --test engine)
MW_ENGINE_TRACE=<scratch>/rpm.trace ./target/release/mostrusted --drive 911 --test engine
# Driving without a human: fixed throttle, logs "car at x y z, speed, compression" once a second
./target/release/mostrusted --drive 911 --units 12 --auto-throttle 0.4 --screenshot <scratch>/drive.png --screenshot-delay 400
# Controls are the original's defaults (input.rs): arrows steer, A accelerate, Z brake, Space handbrake; pad left
# stick, RT / LT, X. Easydrive (easydrive.rs): numpad 6 or d-pad right opens (E too), 8 / 2 move, 4 back; headless:
./target/release/mostrusted --drive 911 --units 12 --show-easydrive --screenshot <scratch>/easydrive.png --screenshot-delay 60
# Car menu (Esc / Tab / F1 / gamepad Start or Back in interactive runs). Headless: swap cars in turn (`name@paint`, `free` =
# free camera), screenshot after the last; logs model / mesh / material / entity counts per step (leak check)
./target/release/mostrusted --drive 911 --units 12 --auto-throttle 0.3 --swap-to "veyron super,aventador@2" --swap-delay 200 --screenshot <scratch>/swap.png
./target/release/mostrusted --camera=-1131,27,1627,-1190,15,1720 --units 12 --swap-to 911,free,veyron --screenshot <scratch>/spawn.png
./target/release/mostrusted --drive 911 --units 12 --show-menu --screenshot <scratch>/menu.png --screenshot-delay 60
# Volumes: F2 / gamepad Select in game (saved to ~/.config/mostrusted/settings.cfg); per run:
./target/release/mostrusted --drive 911 --volume master=0.8,engine=1,sfx=0.6,music=30%

# Game flow (frontend.rs, pause.rs): no mode flag starts the game's front end (title, loading, free roam in the
# default car at the game's start point; Esc / pad Start pauses). --drive / --race / --test / --camera / --replay
# skip it. --new-game, --profile <file> (~/.config/mostrusted/profile.cfg), --front-end-shots <dir>, --show-loading
./target/release/mostrusted
./target/release/mwtool hsm [<node id>] [--find text]     # game-logic state machines (HSMData); mwtool frontend
# Races (race.rs, ai.rs, cutscene.rs): the event's intro, AI opponents on the grid, gates, laps, results
./target/release/mwtool events [<id> [--png f]] [--all]  # offline events; mwtool camera-take [take] (ICE cameras)
./target/release/mostrusted --race 462174                # CONTINENTAL DRIFT; --no-cutscenes, --spawn= overrides the start
./target/release/mostrusted --race 462174 --test race --units 12   # TEST RESULT race ... (gates, laps, wrong way)
MW_CUTSCENE_SHOT=intro:9 ./target/release/mostrusted --race 462174 --units 12 --screenshot <scratch>/i.png --screenshot-delay 100000
./target/release/mostrusted --drive 911 --ai "911,veyron super" --test ai --ai-seconds 60   # AI racers
# Traffic (traffic.rs; density from the event): --no-traffic, --traffic-density x, MW_TRAFFIC_LOG=1
./target/release/mwtool traffic --at x,z                 # lanes near a point (plain: junction cycles, decoded data)
./target/release/mostrusted --drive 911 --test crash-traffic       # head-on with traffic: crash camera (car_impacts.rs)
./target/release/mostrusted --drive 911 --test crash-ai --ai 911 --no-traffic --spawn=925,23,-101,10   # (the default spawn has a barrier ahead)
# Car state: --nitrous holds nitrous, --damage all=0.5 starts damaged, R washes the car, --dirt 0.6,0.3
./target/release/mostrusted --drive 911 --test exhaust   # pops / backfires (MW_EXHAUST_LOG=1)
./target/release/mwtool damage <car> --keys              # damage clip keys; --level all=0.5 --preview <png>
./target/release/mwtool traffic-sound all|sfx out.wav    # traffic engines / horns / impacts offline; engine-sound <car> --competitor
# Debug logs: MW_NITROUS_LOG=1, MW_DIRT_LOG=1, MW_GATE_LOG=1, MW_TYRE_DEBUG=1, MW_FLASH_LOG=1 [MW_FLASH_SHOTS=dir]
# Performance: MW_FRAME_LOG=<file> (per-frame schedule / render times), --bench prints p95 / p99 / hitches;
# A/B switches MW_INDIRECT=1 (Bevy's multi-draw-indirect), MW_AVIAN_ASYNC=1, MW_BAKE_THREADS=0, MW_FOV_VERTICAL=1
```

Use `--camera=` with `=` when the first value is negative. Release builds are needed: dependencies are
optimised in dev too, but world loading is slow without `--release`.

## Workflow for a new resource format

1. `mwtool extract` a bundle with the type; hexdump `chunk0` (and `chunk1` if it has a body) plus
   `imports.txt`. Import offsets show where the pointer-to-resource fields are.
2. Survey **all** instances across the install with `mwtool survey <Type>` to separate constants, pointers,
   import slots, counts, floats and enums.
3. Check burnout.wiki and DGIorio's `nfsmw_exporter` (its `write_*` functions show layouts) for prior
   knowledge.
4. Write the parser in `mw-formats` with a layout table in the doc comment. Use `mw_bundle::Reader`
   (bounds-checked) and `Resource::import_at(offset)` for resource pointers.
5. Verify against every instance, plus one ground-truth check (visual or real-world value).
6. Record findings and open questions in `docs/research-log.md`; move settled facts into `docs/formats.md`.

## Shader work

The game's shaders are DXBC: port the real maths instead of guessing. Run `mwtool shaders` to find a family's
samplers, parameters and vertex layout, and `mwtool shader-asm <name>` for the pixel and vertex programs (the
longest `ps_4_0` is usually the main G-buffer pass; render target 1 holds `sqrt(albedo)`). Ported families live
in `crates/mostrusted/src/layered.wgsl`, selected by `materials::Look`.

## Audio work

I can't listen to audio, so verify decoders numerically (see the research log): spectral flatness against
white noise, sample-to-sample smoothness (diff/RMS), discontinuities at codec frame or grain boundaries, and
spectral centroid against RPM for engine sounds. Export with `mwtool audio` and analyse the `.wav` in Python.
Engine A/B switches (runtime and `mwtool engine-sound`): `MW_AUDIO_DRY=1` (plug-ins off), `MW_AUDIO_NO_DIST`,
`MW_AUDIO_NO_SUB`, `MW_AUDIO_NO_EQ`, `MW_AUDIO_DIST=raw`, `MW_AUDIO_RPM_LINEAR=1` (audible RPM = physics RPM);
`MW_AUDIO_METER=1` prints channel levels and plug-in chains (and other cars' voices), `MW_AUDIO_NO_FX=1` turns the
sequences' effect items off, `MW_SOUND_TRACE=<file>` logs what the engine sound plays from.

## Parallel agents

Worktree agents work well here if builds are throttled (docs/next-session.md has the details):
- At most ~4 agents building at once; each worktree gets a real copy of a finished dependency build
  (`.claude/tools/copy-deps.sh <main checkout> <worktree>`, never hard links) and builds only
  `nice -n 19 cargo build --release -p mostrusted -p mwtool`. Agents skip clippy / tests; the integrator runs them
  once on the merged tree. `.claude/worktrees/.cargo/config.toml` caps build jobs.
- Worktrees may start from an older commit: agents `git merge --ff-only master` first.
- /tmp (and the session scratchpad) is a RAM-backed tmpfs: binaries, traces and captures go into the worktree.
- Agents that run the original (`tools/orig`) mute its audio stream (`pactl set-sink-input-mute`) and quit it after.
- Remove merged worktrees and their branches at the end of a session (≈12 GB each).

## Conventions

- Game coordinates are used as-is: metres, Y up, D3D row-major matrices (`v * M`). Convert with
  `loader::game_matrix`. Don't flip axes; signage confirms handedness is already right.
- Resource IDs are displayed as 16 hex digits (`{:016x}`), matching the bundles' debug string tables.
  `ResourceId::community_string()` gives the `AA_BB_CC_DD` form used by modding tools.
- Parsers return `mw_formats::Result` and must not panic on malformed data.
- Bevy-specific code stays in `crates/mostrusted`. Heavy decode work happens on the loader thread, which
  sends ready-made `Mesh` / `Image` values to the main thread.
