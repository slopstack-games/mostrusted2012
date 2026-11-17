# Commands

Every tool, flag, environment switch and test, grouped roughly by system. Not loaded into context by default:
grep it for a system, file or flag (`grep -n -i 'police\|siren' docs/commands.md`) and read only the lines you
need. New commands and switches go here (one line each), not into CLAUDE.md.

Use `--camera=` with `=` when the first value is negative. Release builds are needed: dependencies are
optimised in dev too, but world loading is slow without `--release`.

```sh
tools/hl <command...>                       # run in a headless 1280x720 gamescope (no desktop window); MW_SHOW=1 runs it directly
#   other sizes: MW_HL_SIZE=WxH (the gamescope display) with MW_WINDOW=WxH (mostrusted sizes its window at startup) and
#   MW_RESIZE_AT=<frame>,WxH (a runtime resize; window_test.rs), e.g. MW_HL_SIZE=1920x1080 MW_WINDOW=1280x720
#   MW_RESIZE_AT=120,1920x1080 tools/t --drive 911 --units 6 --show-easydrive --screenshot <scratch>/r.png --screenshot-delay 200
#   MW_UI_TEXT_LOG=1: every UI label laid out (text, font, size, spacing, box, glyph quads) on stderr
#   silent too: MW_MUTE=1 (= --mute, also silences Bevy's GlobalVolume) and PULSE_SINK / PIPEWIRE_NODE = tools/silent-sink
#   (a null sink, created once; default output untouched); MW_SOUND=1 keeps the sound. tools/orig launches are silent the
#   same way (orig-wrap), MW_SOUND=1 at `orig launch` keeps it.
tools/t <mostrusted args>                   # tools/hl + result lines only (full log scratch/run.log)
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
./target/release/mwtool packs                      # game packs, entitlements, their grants, each DLC pack's cars and events
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
./target/release/mwtool wheels all --png <dir>     # wheels: spec vs the game's VehicleInfo axle rule, side views
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
#   memory reads without ptrace: write ref/probe/hello/dump.req (`label base [offset ...] length`, docs/probe.md)
# Capture helpers through the probe (docs/probe.md "Capture helpers"; the probe dir comes from `orig launch --probe`)
./target/release/mwtool probe status              # raw probe commands: status, place, tp, car, repair, notraffic, poke / hold / peek, release, arm, script, log, stop
./target/release/mwtool tp hodges-kerb100 [--kmh 100]   # the game's own placement (FUN_00647e00); spots in tools/orig/spots.txt, or -- x,ground_y,z,heading
./target/release/mwtool tp --save <name> | --list | --raw -- x,y,z,h (move the bodies directly) | --hops 100 -- <a;b;spot> (raw hops)
./target/release/mwtool car 911                   # the game's swap (FUN_006c2020): any car by id or name, owned or not
./target/release/mwtool mods 911 nitrous=2,gearbox=1   # swap with upgrades fitted, no unlock (groups tyres/nitrous/gearbox/chassis/body, a mod name `powershot=2` or game slot `3=1`; levels 0|none 1|base 2|pro; `none`, `all=<level>`); prints the matching `--mods`, logs to <probe dir>/mods.txt (capture meta.txt copies it); `mwtool mods 911` lists names and game slots
./target/release/mwtool probe rollout              # the player's rollout record (vehicle id, the five upgrade groups' slot:level)
# Input scripts: nitrous `nan` leaves the lane to the game's own nitrous (tank, its throttle rule); a number writes it
./target/release/mwtool conditions --hour 13 --no-traffic --repair --nitrous 0   # --release lets the clock run again
./target/release/mwtool inputs ref/x/inputs.csv 2.5:0,0.45,0,0,0 2.8:0,0,1,0,0   # seconds:steer(+ right),throttle,brake,handbrake,nitrous
./target/release/mwtool capture kerb100a --probe --watch kerb.watch --script ref/x/inputs.csv --spot hodges-kerb100 --kmh 100   # one row per tick
./target/release/mwtool capture kerbrest911a --probe --watch kerb.watch --script ref/x/inputs.csv --spot hodges-kerbrest --settle 4
./target/release/mwtool surfaces --near=-2050,1000,700 --flat [--find 2] [--flat-tol 0.1] [--flat-heading 139.5]   # longest level straights
./target/release/mwtool surfaces --near=-2050,1000,450 --flat --flat-kerb   # square approaches to kerbs (run-up, step, level after)
./target/release/mwtool freeroam --at=-210,58,-2090   # every trigger volume holding a point
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
# --replay plays a recorded in_nitrous column (probe captures) straight into the engine multiplier and gearbox, no tank; logs nos_mult
# In the world from the recording's start (speed given at once): --spawn=x,y,z,heading with --replay; MW_REPLAY_FOLLOW=1
#   holds the recorded speed (throttle / brake), =path also steers along the recorded path (pure pursuit); a recording
#   that starts moving is placed as the game's placement does it (gear by speed, revs from the wheels)
tools/hl ./target/release/mostrusted --drive "F-150 SVT Raptor" --replay ref/kerb_drop2/telemetry.csv --spawn=192.03,50,1474.32,32.8 --no-traffic
./target/release/mwtool compare ref/corner1/telemetry.csv ref/corner1/telemetry.ours.csv

# Visual check without a human: renders, saves, exits
./target/release/mostrusted --camera=-1131,27,1627,-1190,15,1720 --screenshot <scratch>/road.png   # see docs/viewpoints.md
./target/release/mostrusted --vehicle "911" --camera 4.5,1.4,4.0,0,0.4,0 --screenshot <scratch>/car.png
# Matched pairs with captures of the original (ref/shots/<place>-<hour>/notes.txt): same camera, horizontal FOV
./target/release/mostrusted --camera=197.1,50.1,1482.7,202.4,48.7,1491.1 --fov 70 --hour 13 --units 20 --no-traffic --screenshot <scratch>/p.png
# MW_ENV_REPORT=1 logs the environment bake against the original's shader constants; MW_RIG_AMBIENT=1 = old ambient (A/B)
# Driving cameras: --camera-mode chase|bonnet|bumper|lookback (C / gamepad LB cycles; End or the right
# stick fully back looks back; right stick / right mouse drag looks around; the free camera looks with the
# right stick too). MW_CAMERA_TRACE=<file> logs the camera every frame; MW_CAMERA_PUSH_IN=1 keeps the eye out of
# walls (ours: the original's camera has no world query)
MW_CAMERA_TRACE=<scratch>/cam.trace ./target/release/mostrusted --drive 911 --test drift
# Chase camera against the original, frame by frame: our camera code over a recording's car motion (ref/live1)
MW_TOOLS=.claude/tools python3 tools/camcompare.py inputs ref/live1 <dir>/in.csv
./target/release/mostrusted --camera-replay <dir>/in.csv,<dir>/ours.csv
MW_TOOLS=.claude/tools python3 tools/camcompare.py compare ref/live1 <dir>/ours.csv   # per-component RMS / lag
./target/release/mostrusted --drive 911 --units 12 --auto-throttle 0.6 --camera-mode bonnet --screenshot <scratch>/bonnet.png --screenshot-delay 400
# Time of day and weather (weather.rs; interactive: F5/F6 hour, F7 weather). The timeline is the one painted
# under the camera (--timeline <id|default> overrides); --weather = the Alternate (bad-weather) keyframes.
# Roads are always wet as in the game (--road-wetness 0 dries them); --planar-reflection mirrors the scene in them
./target/release/mostrusted --camera=-1131,27,1627,-1190,15,1720 --units 20 --hour 1.5 --weather --planar-reflection --screenshot <scratch>/wet.png
# Post-processing (tonemap.rs): the original's vignette from the default PostFX keyframe; MW_NO_VIGNETTE=1 for A/B.
# Bloom (bloom.wgsl, colour / intensity / level weights from the environment keyframe): MW_NO_BLOOM=1 for A/B
# (MW_BLOOM_DIVISORS=4,8,16: other level sizes). Final pass (post.wgsl, the original's free-roam variant): depth
# of field MW_NO_DOF=1, sun-facing brightness / contrast MW_NO_SUN_CONTRAST=1, keyframe cube blend
# MW_NO_CUBE_BLEND=1; MW_POST_LOG=1 prints the grade and DOF constants
MW_NO_VIGNETTE=1 ./target/release/mostrusted --camera=-1131,27,1627,-1190,15,1720 --units 20 --hour 13 --screenshot <scratch>/novig.png
./target/release/mwtool paint-map 010000000002823d --png <scratch>/timelines.png --at=-1158,1700   # WorldPaintMap + zone outlines
# Handling tests on a flat track (4x real time), prints TEST RESULT lines and exits
./target/release/mostrusted --drive 911 --test accel     # or brake, corner (skidpad g), handbrake, drift, engine,
#   nitrous, landing, climb, jitter (MW_SUSP_TRACE=<file> logs the suspension every tick,
#   MW_WHEEL_TRACE=<file> the point-mass wheels: compression, rate, velocity after the sub-steps and after the contact
#   passes, contact impulse, flags; MW_WHEEL_RAY=1 the old ray wheel probe, MW_WHEEL_DECAL=1 wheels meet decal triangles;
#   MW_STRUT_JOINT=0|pos|vel the strut joint left out / one of its passes only, MW_STRUT_STOP=1 the strut stop on every
#   car (the game's only on a crashed one), MW_ROUGH=0 no surface bumps; A/B, research log 2026-10-19 "kerb captures")
./target/release/mostrusted --drive 911 --test smash --smash-speed 60 --screenshot <scratch>/smash.png   # props: cone, lamp, fence, wall
#   --smash-set compounds: lamps etc. breaking into parts. Props collide in the world by default (--no-prop-physics)
./target/release/mostrusted --drive 911 --test kerb        # kerb strike (MW_TEST_KERB_H / _DEPTH / _ANGLE / _UP, MW_TEST_KMH)
#   crashes=N in its result; MW_SPECULATIVE_CONTACTS=1 counts avian's speculative body contacts as impacts again (crash
#   rule, wall scrape, impact yaw damping; the old behaviour), MW_CRASH_CONTACTS=1 adds every body-world contact point to
#   MW_CRASH_TRACE (closing speed, penetration, impulse, normal, point in the car's frame); research log 2026-10-19
#   "false crashes at kerbs"
./target/release/mostrusted --drive 911 --test-track --track-surface 4 --test accel   # the flat track as any surface (4 = grass)
./target/release/mwtool props -v                    # physics of every prop / compound / dynamic object (+ WCVfxBehaviours)
#   prop-break effects (prop_vfx.rs): MW_PROP_VFX_LOG=1 logs impact / spot / contact effects and lights out;
#   MW_VFX_DEMO=<effect id>,<every s>,<speed> plays any effect ahead of the camera (loads it on demand)
#   car collision effects (crash_vfx.rs: glass, sparks, debris, dust): MW_CRASH_VFX_LOG=1 logs each started
#   effect (zone, contact value; =2 also refreshes), a CRASH VFX count line at exit
./target/release/mwtool lods --at=815,60,1760          # model LOD tables / placement LOD scales; what the game draws from a point
#   runtime: MW_NO_LOD=1 (everything at LOD0, A/B), MW_LOD_SCALING=0|1|2 (the game's LodScaling option), MW_LOD_LOG=1
./target/release/mwtool surfaces                    # surfaces: grip, rolling, drag, dirt build-up, triangle counts, ground area
./target/release/mwtool surfaces --classes          # per surface: tag flags of its type, grip/rolling/drag per tyre class, kick-up / smoke / marks
#   MW_SURFACE_LOG=1 (mostrusted): a line each time a wheel moves onto another surface (index, physics, tyre class, grip, drag, bumps)
#   --near x,z[,r] (surfaces around a point), --find <index> (where a surface is), --steps (kerbs / steps nearby), --tris (every triangle: tag flags, normal, vertices)
./target/release/mwtool ui screens                  # game UI: screens / widgets (ui show <screen>, ui render <screen> out.png, ui font <n> out.png, ui easydrive)
#   ui show prints each widget's group (the HSM's "Activate Widget Group" ids) and elements' in / idle behaviours; ui text --grep <s>
./target/release/mwtool widgets                     # JSON widget definitions + screen scripts: check all (exit 1 on a failure); widgets <screen>, --json <Name>_<UniqueId>|<id>, --scripts
./target/release/mwtool traffic --png <scratch>/lanes.png   # TrafficLaneData: counts, flows, vehicle lists, lane map
./target/release/mwtool lights --near x,z[,r] --hour 21     # world lights (lists + street lamps): types, flags, ranges
./target/release/mwtool car-lights 911                      # a car's light sources (VehicleVfxBehaviour.Lights; `all` surveys)
#   car_lights.rs: race cars light the world (the player: headlight projector + its shadow map); MW_NO_CAR_LIGHTS=1
#   (A/B), MW_HEADLIGHT_SHADOWS=0 (the original's HeadlightShadows option off), MW_CAR_LIGHTS_LOG=1
#   night lighting (world_lights.rs): lamps and lit windows follow --hour; MW_NO_WORLD_LIGHTS=1 for A/B
./target/release/mwtool freeroam --list --kind SpeedCam     # TriggerData: speed cameras, billboards, gates, jack spots
#   --objects: each collectable's nearest placed object and its instance ids (billboard compound = trigger id)
./target/release/mwtool scoring                     # ScoringActions: Speed Points, message sequence / title (--all, --sequence <id>, --text <ids>)
# HUD sequences' world items (sequence_fx.rs: the speed camera's flash light, brightness flash, sounds):
# MW_SEQUENCE_FX_LOG=1, MW_SEQUENCE_FX_SHOTS=<dir> (shots at 0.05 / 0.34 / 0.6 / 1.05 s of each sequence).
# World coronas (world_coronas.rs: street lamps, speed cameras): MW_NO_WORLD_CORONAS=1 (A/B); MW_CORONA_LOG=1 also
# prints the world queue's size and cost
# Junction traffic lights (traffic_lamps.rs, colours from the traffic cycle; needs traffic, --traffic-density 0 keeps
# the cycle without cars): MW_TRAFFIC_LAMP_LOG=1 (lamp positions, colour changes),
# MW_TRAFFIC_LAMP_SHOTS=<dir>[,<period s>[,<count>]] (shots from the car's arrival, then exit)
MW_TRAFFIC_LAMP_SHOTS=<dir>,2.5,12 ./target/release/mostrusted --drive 911 --units 12 --spawn=-265,49,1819,128 --traffic-density 0 --camera-mode bonnet --mute --hour 1
# Collectables and Speed Points (freeroam.rs): a speed camera, a billboard and a gate; TEST RESULT freeroam ...
./target/release/mostrusted --drive 911 --units 12 --mute --no-traffic --test freeroam --profile <scratch>/p.cfg --screenshot <scratch>/fr.png
./target/release/mwtool placement-check --top 10             # placements vs the collision under them, per layer / model
#   --floating: trees / cards / props / groundcover above every drawn surface (static view clusters, then per camera
#   zone by drawing rule; MW_FLOAT_NEAR=300 range, MW_FLOAT_ZONES=1 per zone, MW_FLOAT_DIFF=<zone> lists, MW_FLOAT_CSV=f)
# World view (zone_view.rs): default = the camera zone's backdrop + groundcover, main geometry of the zone and its
# neighbours, as the game swaps them; --composed-backdrop = one backdrop per place, all main geometry (before 2026-10-19)
./target/release/mwtool terrain-check --top 30               # drawn ground above road meshes (terrain through the road), ranked
#   --all-passes / runtime MW_ALL_PASSES=1: also the models whose pass mask (Model +0x19) skips the main view (A/B)
#   only roads drawn at the same time count (LOD shells, --scale 1.5); --only main|lod|lod-ground|backdrop|shadow,
#   --within <m> (seen from closer), --driven (collision at the road); each cluster prints a --camera on the road
./target/release/mwtool terrain-check --masks                # pass masks per layer / shader family; never-drawn models
./target/release/mwtool terrain-check --model 11e76144       # one model: pass mask, LOD / shadow distances, renderable per state
# Runtime pass masks (pass_mask.rs): MW_PASS_MASK_LOG=1 (meshes per view layer), MW_STANDIN_SHADOWS=1 (the ground
# stand-ins' SHADOW meshes cast; off by default, model decision)
./target/release/mwtool water                                # water meshes: height, listing zones, drawn water over dry ground (MW_WATER_AT=x,z ...)
./target/release/mwtool mods 911                             # performance upgrades: slot -> parameter changes
./target/release/mwtool sampler-usage --filter Driveable     # the game's sampler states per shader slot
# Car options: --dirt 0.6,0.3 (dirt, dust), --anim all=1 (pose animated parts in --drive), --mods 12=0 (slot=level
# as `mwtool mods` prints them; also through Easydrive: Customize car); nitrous is Left Shift / pad A
./target/release/mostrusted --drive 911 --units 6 --show-easydrive --easydrive-path 4,4,1,1 --screenshot <scratch>/mods.png --screenshot-delay 200
# Frame-time benchmark (prints a BENCH RESULT line; --no-shadows, MW_BENCH_PASSES=1 for per-pass GPU times)
./target/release/mostrusted --drive 911 --units 30 --spawn=815,60,1760,270 --auto-throttle 0.3 --mute --bench 10   # --merge-static: A/B the mesh merge
# World streaming (streaming.rs): --drive streams the camera's zone and its neighbours and unloads the rest;
# --load-all keeps every unit (the old behaviour; the default for a free camera without --drive, which --stream
# streams instead); --units N loads N units once. Streaming soak test: the player driven by the AI along a 20 km
# road-graph route (MW_AI_ROUTE_M, default 3000); sample the run's GPU memory every 0.5 s with
# `nvidia-smi --query-compute-apps=pid,used_memory --format=csv,noheader,nounits` (the line of its PID)
MW_AI_ROUTE_M=20000 MW_FRAME_LOG=<scratch>/frames.txt ./target/release/mostrusted --drive 911 --test ai --ai-seconds 420 --no-traffic
# MW_TYRE_MARKS_LOG=1 (tyre-mark segments alive), MW_WATER_DEBUG=1 (lists the water meshes)
# The driving HUD (hud.rs) is on in --drive runs except tests: --no-hud, --mph, F3 shows the debug status text
# Engine RPM every physics tick, for `mwtool engine-sound --trace` (use with --test engine)
MW_ENGINE_TRACE=<scratch>/rpm.trace ./target/release/mostrusted --drive 911 --test engine
# Driving without a human: fixed throttle, logs "car at x y z, speed, compression" once a second
./target/release/mostrusted --drive 911 --units 12 --auto-throttle 0.4 --screenshot <scratch>/drive.png --screenshot-delay 400
# Session 9 (2026-10-15): mwtool music (song lists, --wave <id> --out <dir>), mwtool ui map-items (the map's item looks),
# --smash-set bags, --map-waypoint <event> (GPS route; MW_GPS_LOG=1), MW_XINPUT_CONNECTED=0|1 (title prompt pair),
# MW_CAR_SFX_LOG=1 (other cars' horns / alarms), MW_NITROUS_AT=t0,t1 on mwtool engine-sound; I / L3 engine off / on
# Police sirens / light bars / radio (police_fx.rs, police_audio.rs, police_lightbar.rs, police_speech.rs):
./target/release/mwtool siren <scratch>/siren.wav --car "Ford Police Interceptor"   # sets, channels, a render; cop lights
./target/release/mwtool speech [<event id>] [--out <dir>]                           # radio events; decode an event's lines
# MW_SIREN_LOG=1, MW_SPEECH_LOG=1 (MW_AUDIO_METER=1 lists the sirens); a cop car driven by the player: the horn
# steps its siren (MW_SIREN_PRESS=t0,t1,... presses it headless); MW_LIGHTBAR_SHOTS=<dir> (16 shots 1/8 s apart),
# MW_COP_LIGHTS=1 (every bar on), MW_NO_LIGHTBAR=1 (A/B), MW_LIGHTBAR_SHOTS_AFTER=<s> (delay the shots)
# Coronas (coronas.rs: glows / beams / flares at every car's lamps): MW_NO_CORONAS=1 (A/B), MW_CORONA_LOG=1 (counts),
# MW_CORONA_TRACE=1 (glows), MW_VIEW_BEHIND=<m>[,<h>[,1]] (camera m behind the player's car, or in front of the nearest
# traffic car with ,1)
MW_SIREN_PRESS=12 MW_LIGHTBAR_SHOTS=<dir> ./target/release/mostrusted --drive "Ford Police Interceptor" --units 12 --hour 1 --no-traffic --screenshot <scratch>/bar.png --screenshot-delay 1500
# The Steam Controller is read from HID (steam_controller.rs; MW_STEAM_CONTROLLER=0 off); --gamepad-debug logs pads
# Controls are the original's defaults (input.rs): arrows steer, A accelerate, Z brake, Space handbrake, W horn; pad
# left stick, RT / LT, X, B horn. Easydrive (easydrive.rs): numpad 6 or d-pad right opens (E too), 8 / 2 move, 4 back; headless:
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
# The original's game-logic machine ("Top State") run beside the hand-written flows (logic.rs, PLAN M8 step 7):
# logs `hsm:` lines (states, hooks, Requests). Carried out so far: page contexts / widget groups (their Lua scripts
# run), the boot page (the title starts on the Press Start group's script, the transition on its request), the song
# list, UI sounds, sim pause, show world, mute, chevrons, triggers (list: logic.rs module doc; MW_MUSIC=1
# MW_MUSIC_LOG=1 to see the song list it picks under --mute)
tools/t --front-end --hsm --front-end-shots scratch/fe --mute; grep 'hsm:' scratch/run.log
# Races through the logic: any event start (map, Easy Drive, pause menu, markers) is the logic's event request; the race
# mode 730003 starts race.rs / the intro / the outro and ends the event (logic.rs doc). --hsm-event <id> (implies --hsm and
# the start-up flow) chooses the event once free roam waits on the Easy Drive prompt; with --test race: the gates test
tools/t --hsm-event 462174 --auto-start 1 --test race --mute; grep -E 'hsm:.*(machine|request (Respawn|PlaySequence|StartGameMode))' scratch/run.log
# MW_HSM_SHOTS=<dir>: screenshots card / intro / race / outro / free roam along it, then exit (add MW_RACE_TEST_AFTER=40
# so the test waits in free roam). --test bodyshop: drive through the nearest body shop (same car entity, resprayed in place)
./target/release/mwtool bindings [--find x] [--parse] [--no-genesys] [--out f.tsv]  # every binding path, parsed (mw_logic::binding)
./target/release/mwtool hsm [<node id>] [--find text]     # game-logic state machines (HSMData); mwtool frontend
./target/release/mwtool hsm <id> --parents               # which nodes start a node (child or transition)
./target/release/mwtool hsm --data                       # the data definitions (JSON view models / hook arguments)
./target/release/mwtool hsm --bundle all                 # parse every HSMData of the install (37)
# (--answer Query.<Name>=<v> answers an engine wait of the ported boot behaviours, e.g. Query.ProfileLoaded=false; every Request is printed)
# Headless HSM interpreter (mw_logic::hsm) from a top-level instance (default 20527 = boot "Top State"): logs every
# state entered / exited per frame; unported behaviours hold, hooks answer EXIT, bindings don't resolve unless
# --answer NAME=VALUE (state id | behaviour code | binding path with '.' | hook name; VALUE exit point / run / value;
# Host.LoadingScreenShowing / Host.GraphicsPrepared answer the host queries). --steps 300 --dt 0.0333
./target/release/mwtool hsm --run --steps 90 --answer 1222788=DONE   # boot → "Playing" (IsProgressionReady, ShouldPlayNewGameIntro are ported: the driver stands in a loaded profile and sets the title's BOOT general trigger; `UIConfig`, `Net`, `SignInState`, `UISequences`, `GeneralSequence` are the logic's own offline roots; the answer left is an online stand-in, the ConnectToAutolog state 1222788; the Blaze silent sign-in (ServerOperation PCLogin, `HSM_SERV_OP`) is ported and fails over offline)
# ... and on into free roam: the driver also plays the game-mode manager (mode_machine::update after each frame; events,
# GameMode items and GameLogicConfig ids from the install; event data and weapon lists load at once), so "Run Game Mode"
# starts event 390703 and the GMM machine 390147 "Freedrive Game Mode":
./target/release/mwtool hsm --run --steps 600 --answer 1222788=DONE   # → "Freedrive Update" + machine 371633 "Freedrive HUD" (hook requests print as "hook request"); the same online stand-in as above, everything else from the logic's roots
# ... and on into a race: `--event <id>` plays the player's choice of that offline event from free roam (the Easy Drive
# menu's list item stands in; circuit 462174, sprint 906146): the free-drive machine ends, the GMM loads the event and starts
# "Race Game Mode" 730003 (decomp.md "Route free roam → race"); the roots answer `Players`, `Event`, `Transitions` and
# `GameLogicState` (research-log 2026-11-17), the driver keeps only the Easy Drive choice and the checkpoint stand-ins.
# --pause-at <frame> holds the pause button from then on (the pause menu opens: `GameLogicState.IsControlGroupEnabled["MENU"]` is true),
# --answer Query.CheckpointsFinished=true ends the race (outro, then free roam again; --answer 1560270=FALSE: not busted)
./target/release/mwtool hsm --run --steps 1500 --event 462174 --answer 1222788=DONE --answer Query.CheckpointsFinished=true --answer 1560270=FALSE
./target/release/mwtool natives [--area offline|online|platform|debug|unknown] [--unported]   # the 187 hooks + 385 Lua natives, handler address, ported or not
./target/release/mwtool lua --list                       # the UI's 115 Lua chunks (mw-lua): id, name (checked against the id), size
./target/release/mwtool lua [--run <name>] [--call <Fn>|all] [--trace]   # main chunk + Construct with logging stub natives
#   --natives <file>: stub only the names listed (the game's registry, one per line; others stay nil); --out <tsv>: per script
./target/release/mwtool frontend --sequence 1014321      # one UI sequence's items; --spots lists every jack spot
# The map screen (map_screen.rs; pause menu MAP): arrows / left stick / d-pad push the cursor (tap: flick to the
# next event that way; hold: free cursor, snaps on release), Enter / A options, START RACE plays the event-loading
# card (event_card.rs) and starts the race with its intro; Backspace / B (or Esc / Start) back to driving
./target/release/mostrusted --drive "v12 vantage" --units 12 --show-map --map-select 1909234 --map-options --screenshot <scratch>/map.png --screenshot-delay 120
MW_CARD_SHOTS=<dir> ./target/release/mostrusted --drive "v12 vantage" --units 20 --spawn=1600,40,2350 --map-start 1909234   # card shots, intro, exit
# Scripted keys / virtual pad for the menus (input_script.rs): `<s> key|keyup|pad|padup|lx|ly|rx|ry|shot|quit [arg]`
# steps, clock from when the pause menu or map is on screen; MW_MAP_LOG=1 logs the map's selection and cursor
# (MW_INPUT_SCRIPT_START=playing: from when the game is playing, e.g. the HUD's widget actions under --hsm)
# Key names also cover the driving keys: Space (handbrake), Tab, F1, KeyR, KeyC, AltLeft, ShiftLeft.
# --sound keeps the sound in --test runs (they are silent otherwise). Live output of a headless run, recorded from
# the null sink (never MW_SOUND=1: that plays on the desktop):
#   S=$(tools/silent-sink); parecord --device=$S.monitor --file-format=wav --format=float32le out.wav &
#   tools/hl env MW_MUTE=0 ./target/release/mostrusted ...
MW_MAP_LOG=1 MW_INPUT_SCRIPT="0.5 lx 1; 0.6 lx 0; 1.5 shot <scratch>/m.png; 2 quit" ./target/release/mostrusted --drive "v12 vantage" --units 12 --spawn=1600,40,2350 --show-map
# Races (race.rs, ai.rs, cutscene.rs): the event's intro, AI opponents on the grid, gates, laps, results
./target/release/mwtool events [<id> [--png f]] [--all]  # offline events (each opponent's rollout car, colours, Most Wanted / cop flags); mwtool camera-take [take] (ICE cameras)
./target/release/mostrusted --race 462174                # CONTINENTAL DRIFT; --no-cutscenes, --spawn= overrides the start
./target/release/mostrusted --race 462174 --test race --units 12   # TEST RESULT race ... (gates, laps, wrong way)
# MW_RACE_TEST_MISS=<k> skips route entry k (missed_at_skip=); MW_RACE_TEST_AFTER=<s> waits s after the event's end
# (racers_left=); MW_RACE_TEST_JACK=1 then runs the jack-spot swap; MW_RACE_WW_TRACE=1 logs the wrong-way machine
MW_CUTSCENE_SHOT=intro:9 ./target/release/mostrusted --race 462174 --units 12 --screenshot <scratch>/i.png --screenshot-delay 100000
./target/release/mostrusted --drive 911 --ai "911,veyron super" --test ai --ai-seconds 60   # AI racers
# Traffic (traffic.rs; density from the event): --no-traffic, --traffic-density x, MW_TRAFFIC_LOG=1
./target/release/mwtool traffic --at x,z[,r]             # lanes and parked cars near a point (plain: junctions, data, parked survey)
./target/release/mwtool ai-graph > graph.tsv           # the AI road graph (node, x y z, width, flags, links), e.g. for tools/orig autopilots
MW_POLICE_LOG=1 ./target/release/mostrusted --drive 911 --test pursuit --units 12 --no-traffic   # cops: state, aim, stop flags
# When a pursuit starts (cop_sight.rs) and the patrol on a traffic ghost (cop_patrol.rs): slow pass, speeding, ram, patrol
MW_COPS_SHOTS=<dir> ./target/release/mostrusted --drive 911 --test cops --units 30 --mute   # TEST RESULT cops ...
MW_POLICE_LOG=1 MW_COP_DISTANCE=80 ./target/release/mostrusted --drive 911 --units 20   # free-roam cops spawn 80 m away
./target/release/mostrusted --drive 911 --test crash-traffic       # head-on with traffic: crash camera (car_impacts.rs)
./target/release/mostrusted --drive 911 --test crash-parked --units 20 --spawn=925,24,-101 --screenshot <scratch>/p.png  # into a parked car
./target/release/mostrusted --drive 911 --test crash-ai --ai 911 --no-traffic --spawn=925,23,-101,10   # (the default spawn has a barrier ahead)
# crash-ai prints TEST RESULT crash-ai (AI speed / yaw response, crash, damage, lanes). A ram of a moving AI car:
MW_TEST_AI_KMH=60 MW_TEST_KMH=80 MW_TEST_RAM_ANGLE=90 ./target/release/mostrusted --drive 911 --test crash-ai --ai 911 --no-traffic --spawn=925,23,-101,10   # 0 rear, 90 side, 180 head on
./target/release/mostrusted --race 462174 --test crash-ai --no-cutscenes   # the same ram on the nearest opponent, 4 s after the go
MW_RACE_CAR_CONTACTS=0 ...    # race car vs race car back to avian's rigid solve (A/B; car_contacts.rs); MW_CAR_CONTACT_LOG=1 logs each solved pair
tools/ram-sweep <out>         # crash-ai at several speeds / angles, both solves: one TEST RESULT line each
# Car state: --nitrous holds nitrous, --damage all=0.5 starts damaged, R washes the car, --dirt 0.6,0.3
./target/release/mostrusted --drive 911 --test exhaust   # pops / backfires (MW_EXHAUST_LOG=1)
./target/release/mwtool damage <car> --keys              # damage clip keys; --level all=0.5 --preview <png>
./target/release/mwtool traffic-sound all|sfx out.wav    # traffic engines / horns / impacts offline; engine-sound <car> --competitor
./target/release/mwtool traffic-sound player[:<id>] out.wav  # the player's horn, impacts, grind, sparks, glass, scratch (player_sfx.rs)
./target/release/mwtool traffic-sound world out.wav  # landings, crash contacts, a car's prop touch, props' own sounds (world_sfx.rs)
# MW_WORLD_SFX_LOG=1: every world / prop / landing / crash / collectable sound started (also loads them in muted
# test runs, as MW_CAR_SFX_LOG=1 does for player_sfx.rs): --test smash, --smash-set compounds, --test landing
# MW_HORN_AT=t0,t1 holds the horn between game times (headless); MW_AUDIO_METER=1 lists the player's voices too
./target/release/mwtool ui pois UI/SCREENS2/371621.BNDL [--key 1013512,1013521]   # a screen's point-of-interest looks: map icon / world layouts, instance visibility / colours / timelines, trigger types, behaviour kinds
# Event starts (event_starts.rs): MW_EVENT_START_LOG=1 (shown starts, panel triangles), MW_EVENT_START_TEST=1 (holds throttle + brake in a start's radius: the burnout start)
# Debug logs: MW_NITROUS_LOG=1, MW_DIRT_LOG=1, MW_GATE_LOG=1, MW_TYRE_DEBUG=1, MW_FLASH_LOG=1 [MW_FLASH_SHOTS=dir]
# Performance: MW_FRAME_LOG=<file> (per-frame schedule / render times), --bench prints p95 / p99 / hitches;
# A/B switches MW_INDIRECT=1 (Bevy's multi-draw-indirect), MW_AVIAN_ASYNC=1, MW_BAKE_THREADS=0, MW_FOV_VERTICAL=1,
# MW_UNCAPPED_DELTA=1 (Bevy's 250 ms catch-up), MW_SKY_FILTER_EVERY_FRAME=1, MW_NO_PREWARM=1, MW_MAIN_MULTI=1
# (multi-threaded main-world schedules), MW_GPU_CLUSTERING=1 (research log 2026-10-10)
```

## Audio

Engine A/B switches (runtime and `mwtool engine-sound`): `MW_AUDIO_DRY=1` (plug-ins off), `MW_AUDIO_NO_DIST`,
`MW_AUDIO_NO_SUB`, `MW_AUDIO_NO_EQ`, `MW_AUDIO_DIST=raw`, `MW_AUDIO_RPM_LINEAR=1` (audible RPM = physics RPM);
`MW_AUDIO_METER=1` prints channel levels and plug-in chains (and other cars' voices), `MW_AUDIO_NO_FX=1` turns the
sequences' effect items off, `MW_SOUND_TRACE=<file>` logs what the engine sound plays from.
`MW_AUDIO_NO_BUS=1` leaves the game's bus gains out (`mw_assets::buses`; the tyre bank's old model gains come
back), `MW_LISTENER_DT=current` the old listener-velocity step (A/B of the doppler jitter), `MW_LISTENER_TRACE=<file>`
logs the listener's velocity per frame; `MW_SFX_TRACE` lines end with each wheel's surface sound set (−1 in the air).
`mwtool vehicle-sound out.wav --surface <index> [--tyre-class 0..4]` renders the tyre / road sounds of one surface.
`MW_DOPPLER_LOG=1` warns on every audible spatial voice placed with a doppler pitch outside 0.5..2 (`car_audio::place`: emitter and listener velocities; a pitch near 0.05 plays a sound as a subsonic rumble).
`MW_SFX_ENGINE_LEVEL=0` leaves the vehicle sound bank at the data level instead of following the engine bank's
normalisation (A/B, `sfx.rs` `bank_level`).
```sh
# Offline render of the player's engine (channel 0) and vehicle sounds (channel 1) in game time, also in muted /
# --test runs (32-bit float .wav), plus <file>.voices (the bank's sounding voices every 0.25 s)
MW_AUDIO_TAP=<dir>/tap.wav MW_SFX_TRACE=<dir>/sfx.txt tools/t --drive 911 --test drift --mute
```

## Windows

Windows hosts build and run the same workspace (MSVC toolchain). The install is found through Steam's registry
key and the default Steam / EA app folders; settings and the profile go to `%APPDATA%\mostrusted`. `mwtool
memscan` / `telemetry` / `tune` / `capture` read the game through `ReadProcessMemory` (`memscan/windows.rs`);
`capture` records no audio there (it uses pactl / parecord). The probe DLL builds with
`rustup target add i686-pc-windows-msvc` and `cargo build --release -p mw-probe --target i686-pc-windows-msvc`.
`tools/orig` (gamescope) is Linux-only.

## Browser (wasm32)

Port plan and status: research-log 2026-11-17 "browser port, step 1". One-time `rustup target add wasm32-unknown-unknown`.
```sh
tools/wasm-check    # the engine-free crates (gamedir, bundle, formats, audio, assets, handling, lua) build for wasm32
tools/wasm-check --threads   # the same for the threaded browser build (nightly + rust-src, std rebuilt with atomics,
                             # target/wasm-threads); its RUSTFLAGS (shared memory, TLS exports) link the final module
# In the browser: game files come from mw_gamedir::WebSource (the picked folder as blob URLs, sync range requests);
# threads are Web Workers (mostrusted/src/platform.rs, wasm_thread); the page needs COOP same-origin + COEP
# require-corp; the main thread never blocks on a lock (PollLock::poll_lock)
tools/web/build                  # the browser build into target/web (WASM_BINDGEN=<path> if not on PATH)
# GitHub Pages: Actions → "Browser build to GitHub Pages" → Run workflow (.github/workflows/pages.yml, manual only;
# Settings → Pages → Source "GitHub Actions"). WEB_TOOLCHAIN=<nightly> picks the toolchain (the workflow's input).
# Pages sends no COOP / COEP headers: tools/web/coi-sw.js (a service worker index.html registers) adds them.
python3 tools/web/serve.py       # serve it + your install on 127.0.0.1:8080 (--install, --port, --out)
#   http://127.0.0.1:8080/ (pick the install folder) or /?serve=1&args=--camera=-1131,27,1627,-1190,15,1720+--units+4
tools/web/headless <name> '<flags>' [timeout] [record_s]   # Brave in tools/hl's gamescope, silent; waits for
#   scratch/web/<name>.png (flags incl. --screenshot <name>.png) or, with record_s, <name>.webm: the page's lossless
#   recording of the game's audio (?record_audio=<s>); console in scratch/web/<name>.browser.log
```
