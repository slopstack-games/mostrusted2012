# Handoff: end of 2026-10-06 (session 5)

Read this first. Method as before (CLAUDE.md "Fidelity rule"): the original is the reference, behaviour comes
from the decompiled code ([decomp.md](decomp.md)), every change names its source. This session re-read nine
systems from the dump in parallel; the session-5 entries at the top of [research-log.md](research-log.md) hold
the details, [PLAN.md](PLAN.md) "Where it stands" the state per gap.

## What landed

- **Handling**: the tyre pass's brake hold (`FUN_009bc0c0`: the leftover brake acts on the chassis without
  turning the wheel), the standstill hold (`Brakes.99407bbc` through `FUN_009d6170` / `FUN_009b1180`) and the
  downshift's ground-time gate (`FUN_009ae840`). 911: 0–100 km/h 3.72 s, 100–0 in 38.5 m, skidpad 2.57 g
  (unchanged). `ref/xkr_launch`: RMS speed 15.1 mph; launch slip now close (κ 0.4 / 0.49 against 0.1 / 0.39),
  ours pulls away ~0.3 s early and revs to 6630 against 6120. **The big gap is the braking phase** (6.0–7.3 s):
  the original enters a drift (yaw −34 °/s, 1 g lateral) and stops in 1.3 s, ours stays straight and takes far
  longer.
- **Input**: the original's default bindings and mapping rule (`input.rs`). W / S no longer drive: arrows, A, Z.
- **Easydrive**: menu tree from the UI data, shown in game; Change car works, the rest is inert, the look is ours.
- **Free-roam**: `TriggerData` decoded (`mwtool freeroam`); speed cameras log passes (`freeroam.rs`).
- **Traffic**: `TrafficLaneData` decoded (`mwtool traffic`), lanes queryable by position. No runtime.
- **Rendering**: the day sky's dark patches (cloud textures decoded as sRGB) fixed; three road decal shader
  variants drawn as the game does; game-font text no longer draws under its own panel (glyph layout ran after
  Bevy's UI stack system: this was also the HUD's "faint digits").
- **Traced, not ported** (each needs a live read first): bloom, vignette, the colour-cube blend; the sun
  direction's writer and the shadow cascade fit were not found.
- **Audits without a fix**: ground texture scale (the main families match the DXBC) and foliage placement
  (`mwtool placement-check`: no decode error). The user's two reports are **not resolved**; ask for a location
  (F3 shows the position).

## Live reads wanted (the original running; `sudo sysctl kernel.yama.ptrace_scope=0` first, ask the user)

`mwtool memscan`: `dump NFS13.exe+0x<addr − 0x400000>` for `.bss`; object fields through `handling.watch`.

| What | Where | Used for (assumed today) |
|---|---|---|
| clutch torque scale | `.bss` 0x011873e0 (vector) | multiplies the clutch torque in `FUN_009d7ac0` (1) |
| standstill hold speed limit | `.bss` 0x01186e60 | hold active below this forward speed (0: only rolling backwards) |
| downshift ground time | `.bss` 0x01187580 | seconds on the ground before a downshift (0) |
| per-tick → per-second factor | `.bss` 0x01187250 | throttle lag, drift speed hold (60) |
| clutch-in throttle exponent, stall fraction | `.bss` 0x011874d0, 0x011874a0 | shift throttle return, launch stall |
| slip-velocity term | `.bss` 0x01186b74 | y term in `FUN_009bc0c0` (not modelled; gravity?) |
| skid spin scale, load floor | `.bss` 0x0118a060, 0x011870d0 | tyre spin sound ramp |
| unidentified | `.bss` 0x01186630, 0x01186f60 | named in the drivetrain entry of the research log |
| tyre blend-input scale | wheel constraint +0x210 | pure → combined slip blends (1; 0 if only the builder writes it) |
| forced brake mask | controls +0x1c30 during a launch | the 0.45 s launch delay (not modelled) |
| bloom | view settings +0x1518, renderer +0x9836 (flag), +0xac74…+0xaca4 (set) | whether bloom is on in free roam and its values |
| vignette | renderer +0xa980 (D), +0xa990 (A) | composite inner / outer colours |
| colour-cube blend | frame state +0x2c…+0x78 (seven slots, six weights) | which slot holds the environment cube |
| pad dead zones | pad object +0x140 / +0x144 | 0.2 / 0.1 from `.rdata` defaults |
| traffic scale | field 0x48 of the object at `DAT_0110f940` | scales a `TrafficVehicle` field in `FUN_005f4d10` |

## Next, in order

1. The live-read session above; then port bloom, vignette and the cube blend (kernels and sources are in
   decomp.md "Post-processing and colour grade").
2. Handling: why the original drifts under braking in `ref/xkr_launch` and ours does not (inputs in the
   telemetry at 6.0–7.3 s; the +0x210 leads `FUN_009c24b0` / `FUN_009a56b0`), the launch delay, then new
   reference runs (handbrake turn, steady corner, donut; tools/orig).
3. The user's visual reports with a location: WorldPBR `Horizontal_*` variants, sampler address modes, the
   game's groundcover draw distance (`GroundcoverJob`), camera-facing groundcover.
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
