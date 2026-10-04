# Handoff: state at the pause on 2026-10-04 (session 2)

Read this first when resuming. Details of everything below are in [research-log.md](research-log.md) (dated
"session 2" entries and the agents' entries from the same day), [handling.md](handling.md) and
[formats.md](formats.md).

## 1. Merged on master today

- **Lighting:** the game's 3-cascade shadow lookup (on Bevy-rendered cascades), the game's auto exposure +
  tonemap + per-keyframe `ColourCube` grade (`tonemap.rs`), the sun direction from `SunMoonPath`, the NewSky
  glow formula, skylight in the ambient (fixed the yellow cast).
- **Vehicles:** lights, lenses and glass shaders; damage (skeletal damage clips, `--test crash`); car menu
  (Tab / F1 / Start; `--swap-to`); a wheel-layout repair for broken data (the Venom's front axle);
  suspension rest position (wheels no longer rise through the fenders).
- **Handling:** drift model (`drift.rs`), tick-exact tests, `--test donut`.
- **World:** zone-composed backdrops (fixed buildings on streets / stray mountains; `--backdrop-zone N`,
  `--all-backdrops`).
- **Audio:** engine sequence bank + mixer graph (the user still says the engine sounds wrong, see 3), tyre
  screech, road, wind, impacts and scrapes (`sfx.rs`, own player copy).
- **VFX:** `BearEffect` runtime: tyre smoke, wheel dust, sparks (`particles.rs`).
- **Camera:** data-driven chase / bonnet / bumper views, look-back, right-stick / right-mouse look-around
  (`camera.rs`; C / gamepad Y cycles views).
- **Tools:** `mwtool memscan` / `telemetry` (read-only measurement of the running original; see handling.md
  "Measuring the original"; needs `sudo sysctl kernel.yama.ptrace_scope=0`).

## 2. Agent branches waiting to be merged (paused mid-work)

Four background agents were asked to commit their work in progress and stop. Their branches (worktrees under
`.claude/worktrees/`) still need reviewing and merging. Check each report / last commit message first:
- Merged into the integration branch `worktree-agent-a4ed5d38fe649a2e0` (from master, not yet merged into
  master; three merge commits, physics tests unchanged: accel 0-100 3.80 s / top 296.6, drift peak 25.5°,
  crash 116 km/h → front 29 %):
  - **weather** (`worktree-agent-a61cdc331105fad7d`, finished). The game has **no rain** in its data. Runtime
    hour / Alternate switching (F5/F6/F7, `--weather`, `--hour 13.5`), the timeline painted per map cell
    (`WorldPaintMap`, `--timeline default`), roads always wet per the shader (`--road-wetness 0`), opt-in
    `--planar-reflection`. `camera.rs` and `particles.rs` now use the `crate::MainCamera` filter, and the
    reflection works with the new chase camera. The near-black upper daytime sky is already on master (seen
    in master's own road screenshot), not caused by this branch.
  - **prop collisions** (`worktree-agent-a448f6bbd0543fac1`, paused WIP): prop / compound / dynamic colliders
    that break free, collision surface tags (`CarSurface`), `mwtool props|surfaces`, `--test smash`. Now
    **opt-in** (`--prop-physics`): the default world drive wedges against a knocked lamp and the crash barrels,
    and `--test smash` stops at the fence (research log). Its drive.rs hooks (World-layer masks on the
    wheel / spawn rays, car layers, bumper box) must be re-applied to the new physics.
  - **game UI** (`worktree-agent-a17939d9314475550`, paused WIP): UI data (`mwtool ui`), game fonts, driving
    HUD (on when driving; `--no-hud`, `--mph`, F3). Speed digits look faint and the odometer reads ".0 KM".
- Still waiting: `worktree-agent-a23d46f081ecde4ae`: **engine sound** (Fable): ground-truth comparison against
  the raw Ginsu recordings, decoder check, synthesis model, mix. The user's top audio complaint.

Merge procedure used all day: `git merge --no-edit <branch>`, resolve conflicts (usually both sides are
additive: plugin lists in main.rs, `mod` lists, genesys_names.txt, research-log entries at the top of
Findings), build, then run `--test accel/drift/crash` and a screenshot. The tests should be unchanged unless
the branch is about handling. Then `cargo fmt`, clippy at 0 warnings, tests, `mwtool verify`.

## 3. Open user requests, in priority order

1. **Engine sound still wrong** (Fable agent branch above). The user hasn't said what exactly; ask (harsh?
   pitch? synthetic? idle?) and suggest `MW_AUDIO_DRY=1` for A/B. Two audio underruns showed up ~5 s after
   world load once the SFX merge landed (setup hitch, not steady).
2. **Cars must feel heavier / more planted, and not ride over props or climb kerbs so easily.** Parked
   experiment on branch `wip/wheel-shapecast-downforce` (414dfae):
   - Wheels swept as cylinders (`Collider::cylinder(radius, 2·half_width)`, axle along X) with
     `cast_shape` instead of rays; steep contacts (normal·up < 0.5) push along the surface normal, and the
     tyre frame uses the car's up axis there.
   - Data downforce F = ½ρ · `Down_force_lift_coefficient` (0.2) · `Down_force` (30) · v² while grounded
     (hypothesis: coefficient × reference area). 131 player cars have 30 / 0.2; 66 entries (traffic?) have 0.
   - **Regressions, cause not found:** 911 top 297 → 250 km/h, 0–100 3.80 → 4.56 s, 100–0 37 → 43 m, corner
     0.98 → 1.89 g, donut broken (2°/s). Disabling the downforce (×0.0) gave identical numbers, so either
     that rebuild didn't take or the cylinder cast causes all of it. Suspects: the cast's contact point
     (`point1` may sit on the cylinder's rim edge, off-centre), `normal1` semantics, the cast starting in
     contact at full bump, and bottoming out at speed with downforce (bump travel is only ~8 cm). Start by
     adding a debug log of `hit.distance`, `point1`, `normal1` per wheel on flat ground.
   - Also consider the prop collision branch (props will block cars once they have colliders) and
     measuring the original with `mwtool memscan` for the real ride height under load.
3. **Lighting still to match:** the ambient's skylight term and the sun bearing's axis are provisional (one
   reference screenshot from the original at a known place and hour would settle both). The sky facing away
   from the sun is too dark (gradient B and the gradient scale are CPU-side). The bonnet camera sits too far
   back (it shows the dashboard). Groundcover tufts render as black blobs.
4. **Handling calibration:** measure the original with memscan (drift angle at half / full lock, whether the
   Raptor and Veyron drift, 0–100); heavy and AWD cars barely drift.

## Before committing

`cargo fmt`, `cargo clippy --release --workspace` (0 warnings), `cargo test --release --workspace`,
`./target/release/mwtool verify` (0 failures). Never commit game data or derived files (screenshots, wavs,
CSV / watch files are git-ignored or stay in the scratchpad).
