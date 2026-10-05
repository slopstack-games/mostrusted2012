# Handoff: end of 2026-10-04 (session 3)

Read this first. The working method changed this session: **the original is the reference** (CLAUDE.md
"Fidelity rule"). Measure the running game, replay the same inputs in Most Rusted, compare; cite a source for
every behaviour change. Details: [decomp.md](decomp.md), [handling.md](handling.md) ("The model", "Reference
runs"), the session-3 entries at the top of [research-log.md](research-log.md), the gap table in
[PLAN.md](PLAN.md) ("Where it stands").

## Tools now available

- **Drive the original yourself**: `tools/orig/orig launch` (headless gamescope through Steam; the launch option
  is set), `shot`, `key`, `hold`, `click`, `read`, `run <script>` (`waitfor`, `capture`, `pad`), `quit`. Look at
  each screenshot before the next input. Title screen: `key Return` (Press any key), then `key Return` on the
  "signed in to the EA servers" prompt (offline). The game loads the user's save (Jaguar XKR, downtown car park).
  **To drive with keys**, Settings > Gameplay > Controller must be "Keyboard" (Esc, Down ×5, Return, Down ×3,
  Return, Down ×3, Left); **switch it back to "Xbox 360 Controller for Windows" (Right) before quitting** and
  leave Settings with Esc so it saves. Keys then: A accelerate, Z brake, Left / Right steer, Space handbrake.
  Analog steering needs the virtual pad, which the game only sees if the user disables Steam Input for it
  (ask). Memory reads need `sudo sysctl kernel.yama.ptrace_scope=0` after each boot (ask the user).
- **Reference runs**: `mwtool capture <name>` (state + the game's audio into `ref/<name>/`), `mostrusted --drive
  <car> --replay ref/<name>/telemetry.csv`, `mwtool compare ref/<name>/telemetry.csv ref/<name>/telemetry.ours.csv`.
- **Constants** from the running game: `mwtool memscan` (`dump NFS13.exe+0x<addr − 0x400000>`).
- **Live tuning**: `mwtool tune` beside the running game changes handling constants and the player car's fields
  with a keypress and restores them on quit ([probe.md](probe.md) "Tuning menu"). Next for it: find the pointer
  from the vehicle object to its GeneSys handling resource so grip curves, gear ratios and drift data can go on it.
- **The probe** ([probe.md](probe.md) "Status"): `memscan poke` works (Tier 0), and the DLL loads into the
  running game through `orig launch --probe <name>` as an `xinput1_3` stand-in and logs readiness to
  `ref/probe/<name>/probe.log` (Phase 0). The function-patching step (Tier 1) is not written and the assistant
  will not write it; the user decides how to proceed. Until then, unmeasured quantities come from pokes and
  telemetry: e.g. `mwtool capture corner --poke NFS13.exe+0xd872a0=0.6` halves the drag constant for one run.

## Next, in order

1. **Fix the 2nd-gear clutch** (`drivetrain.rs`): after the 1→2 upshift our XKR's revs stay at ~6400 instead
   of following the wheels (`ref/xkr_launch`); then re-run the replay. Also the idle: the original wobbles
   1004–1027 rpm, ours sits at 1000 / 969.
2. **More reference runs, automated** (find a long straight road first: drive away from the car park and use
   `read body_px body_pz` + screenshots; note good start points in docs/viewpoints.md): full braking from
   ~60 mph on a clear road, a handbrake turn, a brake-tap drift and a long steady corner (with keys the
   steering is all-or-nothing; ask the user about disabling Steam Input for the pad), a full-lock donut.
   Compare each and fix the largest difference first. Open model questions they answer: the grip scales
   (`LAT_GRIP` 0.4 / `LONG_GRIP` 0.7), the drift side force (the game's cap of 5 gives ~3 g with our reading),
   the handbrake.
3. **Tyre screech "horn"**: capture a long slide in the original (its `audio.wav`) and compare spectra with ours
   (`sfx.rs`); the tonal layer should show up as a stable peak.
4. **Yellow tint**: `tools/orig/orig shot` at a known place and hour in the original vs `mostrusted --camera ...
   --hour ... --screenshot` at the same pose (pose from `read body_px body_py body_pz` + the camera); compare
   the grade, exposure and ambient.
5. Then the rest of the gap table: input mapping (bindings seen so far are in PLAN M3.9), crash camera,
   Easydrive / front end, animations, prop collisions, AI traffic, game logic.

## Before committing

`cargo fmt`, `cargo clippy --release --workspace` (0 warnings), `cargo test --release --workspace`,
`./target/release/mwtool verify` (0 failures). Never commit game data, derived files or decompiled code
(`ref/`, `consts.txt`, CSVs, wavs, screenshots stay out; `ref/`, `*.csv`, `*.watch` are git-ignored).
