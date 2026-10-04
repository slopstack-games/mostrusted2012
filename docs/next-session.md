# Handoff: state at the end of 2026-10-04 (session 3)

Read this first when resuming. Details: [decomp.md](decomp.md) (what the game's code says),
[handling.md](handling.md) "The model", and the session-3 entries at the top of [research-log.md](research-log.md).

## 1. Merged on master this session

- **Handling rebuilt from the game's code** (the user's decompiled dump in `re/`, git-ignored; prose findings only,
  rules in reverse-engineering.md): 60 Hz tick; engine inertia, clutch, shift on rpm and speed, top-gear lock
  (`drivetrain.rs`); per-wheel spin, brakes without ABS / TC, axle LSDs, handbrake locks and decouples the rear;
  tyres as capped velocity constraints with the game's curves and slip blends (`tyre.rs`); spring from sag,
  asymmetric dampers, anti-roll bars, drag and lift, in-air gravity, take-off damping (`chassis.rs`); the game's
  steering rules and drift state machine (`drift.rs`). 911: 0–100 3.87 s, top 307 km/h, 100–0 40 m, 1.02 g,
  brake-tap drift 22° held 4.4 s.
- **Constants read from the running original** with `mwtool memscan` (drag K, clutch cap, shift phases, idle
  governor, steering threshold, ...; list in decomp.md). `handling.watch` (git-ignored, repo root) reads the live
  car: chain `[[[[NFS13.exe+0xcedb04]+4]+0x53220]+0x184]` (player index 48). memscan now handles paths with spaces.
- **Engine sound**: the game's audible-RPM mapping, load filter, harmonics, distortion stand-in, DspParam
  overrides (Fable agent's branch). **The in-game "steps"** (rpm held ~0.7 s, then a jump) were bevy_audio's LOOP
  buffering the live source 32768 samples at a time: engine and SFX now play with `ONCE`.
- **Volume settings**: F2 / gamepad Select overlay (master, engine, effects, music), saved to
  `~/.config/mostrusted/settings.cfg`, `--volume master=0.8,music=30%`.
- **Earlier branches**: weather (finished), prop collisions (opt-in `--prop-physics`: the car can wedge against
  knocked lamps / barrels), game UI / HUD (on by default; speed digits faint, odometer ".0 KM").

## 2. Open, in priority order

1. **Record the original with `handling.watch`** (the user has the game running with an 8BitDo pad):
   `./target/release/mwtool telemetry handling.watch -o <name>.csv --hz 60` for: a steady fast corner (absolute
   grip scale: our `LAT_GRIP` 0.4 / `LONG_GRIP` 0.7 are calibrations), full braking (lock?), a launch, a
   handbrake turn (rear `w2_omega` / `w3_omega` → 0?), a brake-tap drift (`drift_side_f*`, `drift_boost`,
   `drift_scale`: with the game's side-force cap of 5 our side force reaches ~3 g, so our reading of its size or
   direction is wrong; the model caps it at 1), a full-lock full-throttle donut from rest, and normal driving
   with slides (`steer_q`, the steering neutral offset's input).
2. **Handbrake test** gives 4° with the game's steering threshold (0.99: partial inputs steer at the raw
   high-speed rate); compare with the recording before changing anything.
3. Props wedging the car (`--prop-physics`), HUD polish, the black daytime sky, groundcover blobs, the bonnet
   camera position.

## Before committing

`cargo fmt`, `cargo clippy --release --workspace` (0 warnings), `cargo test --release --workspace`,
`./target/release/mwtool verify` (0 failures). Never commit game data, derived files or decompiled code
(`consts.txt`, CSVs, wavs stay out; `*.csv`, `*.watch` are git-ignored).
