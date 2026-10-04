# Most Rusted: project plan

Goal: a native Rust reimplementation of Need for Speed: Most Wanted (2012) that runs on the original game's
assets, read at runtime from the user's install. The guiding milestone is **"drive a car through Fairhaven"**.
Everything is ordered to reach that first, then grow toward the full game.

Status legend: `[x]` done · `[~]` in progress · `[ ]` not started. Update the checkboxes as work lands; record
findings in [research-log.md](research-log.md).

---

## Constraints that shape the plan

1. **No game data in the repo.** Code reads the install at runtime. Extracted files, screenshots of game
   content, Ghidra databases and decompiled code stay local and git-ignored.
2. **The executable is encrypted (EA DRM)**, so static analysis of the code is not available
   ([reverse-engineering.md](reverse-engineering.md)). The plan is therefore **data-first**:
   - Formats are worked out from the data itself (surveying every instance of a resource type) plus community
     documentation (burnout.wiki, DGIorio's tools).
   - Game *behaviour* comes from data wherever possible. Most tuning and rules live in GeneSys objects
     (≈190k GenesysType/GenesysObject resources), so reading GeneSys is the key to game logic.
   - Where behaviour is code-only (handling model, AI decisions, shaders), we **reimplement and approximate**,
     parameterised by the game's data, and document where we diverge.
3. **Bevy moves fast.** It is pinned to one minor version (currently 0.19). Engine-specific code stays inside
   `crates/mostrusted`; format crates never depend on Bevy. Upgrade Bevy deliberately, in its own change.

---

## Milestones

### M0: Foundation `[x]`
- [x] Game install detection (Steam libraries, env var, CLI, EA app paths)
- [x] BundleV2 reader, verified against all 2,020 bundles / 432k resources
- [x] Texture, VertexDescriptor, Renderable, Material, Shader, Model, InstanceList parsers
- [x] Cross-bundle asset index (`mw-assets`)
- [x] `mwtool` (list / extract / verify / textures / models / world / shaders / find)
- [x] Bevy runtime: background streaming loader, BC textures on GPU, free camera, `--screenshot`

### M1: Faithful static world `[~]`
Make a static frame look like the game.

1. `[x]` **RE tooling first.** `mwtool survey <Type>` histograms every u8/u16/u32/f32 field across all instances of
   a resource type, flagging constants, pointers (values inside chunk 0), counts and import slots. Most format
   work after this is "survey → hypothesise → verify across all instances".
2. `[x]` **Vehicle assembly**: `VehicleGraphicsSpec` (0x106) gives part models, wheel placement and the skeleton.
   - Done: wheels sit in the arches (right side mirrored), and `--vehicle` accepts a car name. Still open: LOD
     selection by distance, and skinning to the skeleton.
3. `[x]` **Remaining world layers** from `ZoneHeader` (0x206): CompoundInstanceList / CompoundObject,
   PropInstanceList / PropObject, DynamicInstanceList, LightInstanceList, GroundcoverCollection / Instances.
   - Done: props, compound objects and dynamic objects are placed (+21068 placements over the map, 0
     unresolved; `mwtool world` prints counts per layer); lights are parsed (3628) but not rendered.
     Groundcover is decoded and drawn (1.36 M sprites over the map as merged crossed-quad meshes,
     `--no-groundcover` to skip). Still open: light rendering, groundcover billboarding/wind/fade, compound
     sub-parts / LOD slot choice.
   - Done when: street furniture, props and grass appear, and the count of `mwtool world` placements grows
     accordingly.
4. **Shader families as custom Bevy materials** (WGSL), largest visual payoff first. The game's compiled
   shaders are standard DXBC, and `mwtool shader-asm <name>` disassembles them, so port the real maths:
   1. `[x]` TerrainPBR splat (mask + noise overlay weights, three tiling layers): `layered.wgsl` mode 2
   2. `[x]` DriveableSurface base (colour map × world-space detail × 2): mode 1. Still open: lines/decals as
      overlays, wetness, planar reflection
   3. `[x]` PlotPBR (mask-weighted tinted layers + decal): `layered.wgsl` mode 3; road line variants
      (TEXCOORD2 line texture over the road): mode 4
   4. `[ ]` Lightmaps (identify lightmap UV set and texture source; `PersistentLightmap`); road vertex colour is
      baked light (`colour² × intensity × 0.318`)
   5. `[~]` Vehicle paint: `--paint` colour; untextured parts use `PbrMaterialDiffuseColour`. Still open: the real
      paint shader, glass, lights/emissive, and tyres rendering too light
   6. `[ ]` Water (placeholder colour now), trees (translucency, `InstanceTint`), lit windows
   - Longer term: translate DXBC to WGSL automatically (DXBC → SPIR-V via dxil-spirv/vkd3d, then naga), as IW4L
     does for SM3, instead of hand-porting each family.
   - Material parameter name hashes must be mapped to names (Shader parameter tables have names). Build that
     map once and log unknowns.
5. **Environment**: `EnvironmentDictionary/TimeLine/Keyframe` (0x112–0x114) for sun, fog and ambient;
   `ColourCube` (0x52) for grading; a sky.
6. **Vertex data gaps**: packed tangent-frame quaternions in vehicle `TexCoord(4/5)` (SNORM16) → real
   normals; vertex colour (AO / blend weights).

### M2: Game data layer `[ ]`
Unlocks names, tuning and rules. Can run in parallel with M1.

1. `[x]` **GeneSys reader** (`mwtool genesys`; field-name hash cracked 2026-10-04, ~1,200 names recovered,
   `mwtool genesys-crack` to find more): `GenesysType` (0x14) = schema (fields, types, offsets), `GenesysObject` (0x15) =
   instances. Expose a dynamic object model in a new crate `mw-genesys` and a `mwtool genesys dump`
   command that writes JSON.
2. `[x]` **AttribSys** (`AttribSysSchema/Vault`, 0x10/0x11): not used. The 1.4.0.0 install has no resources of
   either type; all tuning data is GeneSys.
3. `[x]` **Language** (0x201): localised strings (UTF-16 table).
4. `[x]` **VehicleList** (0x105) joined with language: `mwtool vehicles`, 110 entries / 93 with bundles. Still open:
   decode the remaining entry fields (floats look like stats) and join with GeneSys.
5. Snapshot tests: JSON dumps of a few known objects. Tests that need the game are skipped when it isn't found
   (see *Testing*).

### M3: Drivable prototype `[~]` (north star)
1. `[x]` **Collision**: `PolygonSoupList` (0x60) per track unit → static avian3d trimesh colliders (built on the
   loader thread; quads split strip-wise since 2026-10-04, which closed the holes). Still open: surface types via collision tags (DGIorio's exporter documents
   `mu16CollisionTag`), and prop/dynamic collision.
2. `[~]` **Vehicle physics**: driven by the game's handling data (mass, CoM, wheels, springs, torque curve, gears,
   brakes, top speed, the tyres' lateral/longitudinal grip curves, speed-banded steering); see
   [handling.md](handling.md). `--test accel|brake|corner|handbrake` measures it on a flat track. Still open:
   the drift assist (`DriftBehaviour` yaw torque / side force), slip factors, aero, and calibration against
   measurements from the original game.
   Original plan: a raycast-wheel vehicle (suspension, tyre slip curves, drivetrain, gearbox) tuned
   from `VehiclePhysicsSpec` (0x107) and GeneSys physics config (`PHYSICS/PHYSICSCONFIG.BNDL`). The original
   model's code is unavailable, so feel is matched by comparing against the real game, not by porting it.
3. `[~]` Chase camera from each car's `VehicleCameraContainer` (eye, look-at, speed FOV, roll, braking pitch,
   high-speed shake), interpolated between physics ticks; keyboard and gamepad input. Still open: the bonnet and
   bumper views, impact shake, and the unnamed acceleration/yaw-spring blocks.
4. `[~]` The world streams nearest-first from the spawn point. Still open: streaming that follows the car (see M4).
- Done when: you can pick a car from the catalogue, spawn on a road and drive around the city with collision.
  **Basic version reached on 2026-10-03.** Remaining: handling feel, real tuning data, spawn points on roads
  (from `RoadData` / AI data rather than hand-picked coordinates).

### M4: Streaming and performance `[ ]`
1. Load and *unload* track units by distance; budget memory (bundles are dropped after decode already).
2. Partial bundle reads: load only needed resources instead of whole files (offsets are in the TOC).
3. Model LOD distances (`Model.lod_distances`) and `HAWAII/PVS.BNDL` visibility.
4. GPU instancing for repeated models; profile with Tracy (`bevy/trace_tracy`).
- Target: steady 60 fps at 1080p while driving through downtown on mid-range hardware; startup to first frame
  under 3 s.

### M5: Audio `[~]`
1. `[x]` EA audio decoders (`crates/mw-audio`): Ginsu (EA-XAS v0) and SPS / `Wave` (EALayer3 → MPEG frames →
   nanomp3). `mwtool audio`, `mwtool engine-sound`.
2. `[~]` Engine sound: Ginsu granular resynthesis driven by RPM and throttle (on-load / off-load recordings).
   Recordings per car are known (two sets of rev-up / low sweep / off-load sweep, plus short copies; research
   log 2026-10-04) and the matched on/off pair is used. Still open: the rev-up pitch steps at grain boundaries
   under acceleration (fix planned: resample grains, see next-session.md), which set is engine vs exhaust,
   `EngineSound2` / `VehicleSound` data, mixing layers, surface/tyre/wind sounds, reverb.
3. `[x]` Music: shuffled `UI/SONGS/*.SPS` playlist. Still open: radio/track names, ducking.
4. `[ ]` One-shot effects from `Wave` resources (collisions, gear shifts, nitrous), speech.

### M6: Gameplay `[ ]`
Roughly in dependency order:
1. Traffic: `TrafficLaneData` (0x701), `TrafficGraphicsStub`, traffic vehicle attribs.
2. Road network and AI paths: `RoadData`, `RoadMapData`, `AIData`, `NavigationMesh`.
3. Triggers and events: `TriggerData`, `GAMELOGIC/GAMEMODES.BNDL`, `GAMEPLAY.BNDL`, Jack spots, Most Wanted list.
4. Cops / pursuit, damage (`DeformationModel`-equivalent data in vehicle bundles), takedowns.
5. `HSMData` (0x501): hierarchical state machines that drive game flow. Interpret, or reimplement by hand.
6. Characters / cutscenes (`CharacterSpec`, `Skeleton`, `Animation`) are low priority.

### M7: Front-end and release `[ ]`
- Native Bevy UI (fonts from `UI/FONTS` if feasible), menus, car select, settings, save data (our own format).
- Movies: `UI/MOVIES/*.VP6` (EA `MVhd` container, VP6). Decode through ffmpeg or skip.
- Packaging for Linux and Windows; CI with build + lint + game-less tests.

---

## Testing and verification

| Layer | How |
|-------|-----|
| Pure logic (ids, CRC, half floats, DDS header, strip unrolling) | ordinary unit tests, no game needed |
| Parsers against real data | tests that call `GameDir::locate(None)` and **skip with a message** when no install is found. Prefer whole-corpus checks ("every Renderable parses") over single samples |
| Corpus health | `mwtool verify` must stay at 0 failures; add per-type parse checks as parsers land (`mwtool verify --parse`) |
| Visual | fixed viewpoints via `--camera … --screenshot`. Keep a list of named viewpoints in `docs/viewpoints.md`; images stay local (they contain game content) |
| Behaviour (physics, AI) | scripted scenarios with recorded telemetry, compared against reference captures from the real game |

Before calling a format "done": it must parse every instance in the install without error, and the result
must be checked visually or numerically against a known real-world value (as the 911's dimensions confirmed the
SNORM ×10 scale).

---

## Risks

| Risk | Mitigation |
|------|-----------|
| Code-only behaviour (handling, AI) can't be ported | Data-driven reimplementation; match feel empirically; document divergences |
| Shader look depends on unknown shader code | `ShaderProgramBuffer` (0x8) holds compiled DXBC. Disassembling the bytecode (data, not the exe) can reveal lighting maths. Consider DXBC→WGSL translation later, as IW4L does for SM3 |
| Bevy API churn | Pin version; isolate engine code; upgrade in dedicated changes |
| Legal | No assets or decompiled code in the repo; no DRM circumvention tooling; game install required |
| Scope | Ship vertical slices (drive in city) before breadth |

---

## Suggested next sessions

**Start with [next-session.md](next-session.md)** (handoff from 2026-10-04: engine pitch staircase, drift model, groundcover).

0. Engine sound mixing from `EngineSound2` data; collision/skid sound effects from `Wave` resources.

1. Drift assist from `DriftBehaviour` (names mostly recovered: `Maximum_yaw_torque`, `Side_force_multiplier`,
   `Drift_scale_from_braking`, ...); then collect reference measurements from the original game (procedure in
   [handling.md](handling.md)) and calibrate.
2. Name more GeneSys fields with `mwtool genesys-crack` (camera: `VehicleCameraContainer`; paint:
   `VehiclePaint`); fix the remaining GeneSys decode failures.
3. World-layer loose ends (M1.3): light rendering, groundcover tint/billboarding/wind, stray tree leaf cards.
4. Lightmaps / PlotPBR / vertex-colour lighting (M1.4).
5. Streaming that follows the car (M4).
