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
- **Traced, not ported** (each needs a live read first): bloom, vignette, the colour-cube blend; the sun
  direction's writer and the shadow cascade fit were not found.
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
| traffic parameters | `DAT_0110f940` / `f944` (object; field 0x48), `DAT_0110f590` (cull radius), `DAT_0118a090`, world +0xa2bfc, world +0xa2b74 at several hours | traffic speed scale, cull radius, the hour's density |
| texture first-mip flag | global at 0x010eac98 (checked by `FUN_00414a50`) | whether the original skips the top mip of every texture |
| groundcover size | constant-buffer slot 0x7F (`GroundcoverConstants`, `cb1[0].x`) | sprite size in metres |
| free-roam state | game-state bytes +0x2e90, +0x2e91, +0x2e95 | gates on the collectable handler |

## Next, in order

1. The live-read session above; then port bloom, vignette and the cube blend (kernels and sources are in
   decomp.md "Post-processing and colour grade").
2. Handling: new reference runs on a clear road (tools/orig): **straight-line braking** first (the original's
   rear wheels show κ ≈ 0…+0.07 under full brake in the clean stretch, ours −0.1…−0.3: the brake split), a
   slide in a second car (settles q's 35°), handbrake turn, steady corner, donut; the +0x210 leads
   (`FUN_009c24b0` / `FUN_009a56b0`); the launch delay.
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
