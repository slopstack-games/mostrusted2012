# Handoff: state at the end of the 2026-10-04 session

Read this first when you pick the work up again, then delete or trim it once it's done. Section 0 is from the
later shader session; sections 1–3 are from before it and still open. Nothing from this session
is committed yet (`git status` shows it all). Scratch files from the session (traces, wavs, dumps) were in the
session scratchpad and are gone; regenerate them with the commands below.

## 0. Shaders and textures (shader session, same day)

The user's direction (now in PLAN.md): recreate the game's architecture, so the rendering pipeline should work
like the original's deferred renderer instead of Bevy PBR. Done this session, all in the ported-G-buffer style
so it carries over:

- Packed tangent frames → normals + tangents (`mw_formats::tangent_frame`), vertex colours decoded.
- `layered.wgsl` modes: roads (slope-space normals, roughness, F0, no dry reflection), terrain (tiling normals,
  macro normal, per-layer roughness / F0, AO), plots (tiling normals, AO map), WorldPBR + World_Diffuse_Specular
  (normal / spec / AO / ColouredSpecular), vehicles (mode 6), trees (mode 7), water (mode 8). 10 texture slots,
  12 constant vec4s.
- InstanceList per-instance overrides (tint, AO map) → per-instance model variants (`InstanceLook`, bit 61 keys).
- Car paint from `VehiclePaint` (`--paint-index N`, `--paint r,g,b` still overrides the colour).
- Sky cube from `ENVIRONMENT.BNDL` (`sky.rs`) as `Skybox` + `GeneratedEnvironmentMapLight`.

Later the same day the deferred pipeline and the environment timeline landed (research log "the game's
deferred pipeline in Bevy"). Next, in order:
1. **Exposure, tonemap, colour cube**: the scene uses Bevy's tonemapping and a fixed `GAME_UNIT` scale. Port
   `Blit2d_GeneratePCTonemapConstants_*`, `ToneMapSetConstants`, the keyframe's Camera exposure fields and the
   `ColourCube` (0x52) grade; then `GBufferComposite`, bloom and `PostAA`.
2. **Shadows**: the key light's cascaded shadow map (`ShadowMap_*` constants in the lighting pass); render a
   depth cascade from the sun and sample it as `sample_c_lz` does.
3. **Sun direction**: name the SunMoonPath fields in EnvironmentSettings (`…69cad`); compare with the sky.
4. Sky dome details (`NewSky` gradient B, dome mapping, four cloud layers), fog in-scattering mapping.
5. Lightmap source; road vertex light + AO; water river floor / foam; vehicle glass, lights, flakes; point /
   spot lights at night.

Visual checks: `docs/viewpoints.md`. Vehicle-only shots need `--screenshot-delay 60` so the environment
prefilter has run.

## 1. Engine sound (rewritten, evening)

The engine now plays all six Ginsu layers from the car's `EngineSound2` data with the game's load curves,
through a new overlap-add synth in lockstep (research log "engine sound rewritten from the game's audio
data"). Ask the user how it sounds; if something is still off, likely candidates: the load parameter (now
throttle, smoothed 50 ms), RPM below the items' 1790 rpm range (idle is pitched down from the lowest grain),
the missing bus effects, and the engine states. Measurement scripts (pitch tracking, clicks, hold modulation,
throttle-step levels) were in the session scratchpad; the research log describes each metric.

## 2. Handling: make it work like the original (user's main request, in progress)

The user asked for the car physics to behave the way the original game's do. `NFS13.exe` is DRM-encrypted
(see `docs/reverse-engineering.md`), so this is done from the data. State after the drift session
(2026-10-04, research log "tick-exact tests, drift model"):

- `--test` scripting runs in `FixedUpdate` and is tick-exact: two runs give identical `TEST RESULT` lines.
- `crates/mostrusted/src/drift.rs` implements the `DriftBehaviour` state (Off / Entering / Drifting / Donut).
  Every field, its reading and whether it is a name-backed use, a hypothesis or a model decision is in
  handling.md, "Drift". The 911 brake-tap drift now holds a steady 20° (14° at 0.6 steering, 30° on full lock
  and throttle) and exits within a second of releasing the steering. Handbrake strength blends between
  `Brake_strength_at_low_speed` / `_at_high_speed`; donuts use `Donut_yaw_torque`.
- New field names (genesys_names.txt): `Slip_angle_to_enter_drift`, `Drift_angle_to_exit_drift`, the
  side-force fields, `Low_slip_angle_for_counter_steering`, the handbrake's `Brake_strength_at_*`,
  `Low/High_speed_mph`, `Minimum_handbrake_time`.

Next, in order:
1. Measure drifts in the original game (angle at half and full steering after a brake tap, speed lost, whether
   the F-150 Raptor and Veyron hold a drift) and fit `TORQUE_FADE_POWER`, the 60 Hz time base and the
   counter-steer gain to them. Heavy/AWD cars barely drift in this model.
2. Name the remaining drift fields (`2ef426dd` / `70528afa` / `a5c9496f` yaw-curve breakpoints, `8f151151`,
   `0dce38f7`, `Default_steering`'s role); try longer structured searches with `--words` per position.
3. Roll limit (`Roll_angle_limit`), `Body_movement` stiffness/damper fields, slip factors, aero.

## 3. World layers (M1.3): done, with loose ends

- Props, compounds and dynamic objects: 56,499 placements on the full map, up from 35,431 (checked visually).
- Groundcover: `crates/mw-formats/src/groundcover.rs`. There is no mesh; the game's shader turns 16-byte instances into
  sprites. 1,364,853 instances parse. They are drawn as one merged mesh per collection and material, and
  `--no-groundcover` skips them. Load time is 34.4 s with them and 31.7 s without.
- Clippy is back to 0 warnings and tests pass. Layouts are in `docs/formats.md`.
- Loose ends:
  - Grass tufts are very dark (the tint is used as albedo × 2, not tuned).
  - Sprite size and yaw bytes are guesses.
  - There is no wind, billboarding or fade distance.
  - Little grass shows at the roadside viewpoints (density is low there).
  - An `#[ignore]`d diagnostic test `diag_gc_ground` remains in `crates/mw-assets/tests/world_layers.rs`.
  - Lights are parsed but not drawn.
  - The stray leaf-card quads in tree canopies **predate** this work (a tree shader or alpha issue).

## 4. Backdrop composition (map "loaded wrong": buildings on streets, mountains)

- Cause: every unit's InstanceList backdrop (a per-unit LOD view of the rest of the map) was drawn at once, so
  every LOD tier stacked. `ZoneList` (`HAWAII/PVS.BNDL`) is decoded; `mw_assets::backdrop::BackdropSelector`
  keeps per place what the owning zone lists (research log 2026-10-04, night). Debug: `--all-backdrops`
  (old union), `--backdrop-zone N` (one unit, as the game), `--no-backdrop`.
- Loose ends: cut faces where zone views meet (viewpoint `airfield-arch`), near water in city rivers, a
  streaming world should draw only the current zone's backdrop (M4).

## Before committing

`cargo fmt`, `cargo clippy --release --workspace` (0 warnings), `cargo test --release --workspace`,
`./target/release/mwtool verify` (0 failures). Never commit game data or derived files.
