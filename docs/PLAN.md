# Most Rusted: project plan

Goal: a native Rust reimplementation of Need for Speed: Most Wanted (2012) that runs on the original game's
assets, read at runtime from the user's install. The guiding milestone is **"drive a car through Fairhaven"**.
Everything is ordered to reach that first, then grow toward the full game.

**End target: the game works exactly the same and looks exactly the same as the original.** Approximations
(Bevy PBR standing in for the game's deferred lighting, a handling model tuned by feel) are stepping stones, not
the goal. Each one is recorded as a divergence and is replaced by the game's real behaviour (ported shader maths,
data-driven rules, measured references from the original) as it becomes understood.

**Recreate the game's architecture, not a Bevy game that resembles it.** Systems are rebuilt as closely as
possible to how the original works, with Bevy as the host (windowing, ECS, GPU access, render graph) rather
than as the design:
- **Rendering**: the original is a deferred renderer. The target pipeline is the game's: its G-buffer layout
  (RT0 normal + roughness, RT1 sqrt(albedo) + AO, RT2 F0 + Fresnel strength, RT3 sqrt(indirect: reflection,
  lightmap, vertex light) × exposure), its deferred lighting (key light with cascaded shadows, irradiance
  quadric, sky reflection cubes with the env-BRDF LUT), sky dome, planar reflections for wet roads, its
  post-processing and tonemap, driven by the environment timeline. Shaders are ported from the DXBC (by hand
  now, later by DXBC → WGSL translation) as custom render-graph passes, not mapped onto `StandardMaterial`.
- **Game logic**: the same split the data implies (GeneSys types, HSM state machines, triggers, AI data) rather
  than ad-hoc Bevy systems.
- Bevy features stand in only temporarily (today: `StandardMaterial` lighting, `GeneratedEnvironmentMapLight`,
  `DistanceFog`); list each stand-in in [research-log.md](research-log.md) so it can be replaced.

Status legend: `[x]` done · `[~]` in progress · `[ ]` not started. Update the checkboxes as work lands; record
findings in [research-log.md](research-log.md).

---

## Constraints that shape the plan

1. **No game data in the repo.** Code reads the install at runtime. Extracted files, screenshots of game
   content, Ghidra databases and decompiled code stay local and git-ignored.
2. **The executable is encrypted (EA DRM) on disk**, so the file itself can't be analysed
   ([reverse-engineering.md](reverse-engineering.md)). Since 2026-10-04 the user has supplied a decompilation of
   a memory dump of the running game (`re/NFS13_dump/`, git-ignored). The plan stays **data-first**, with the
   code as a reference for behaviour:
   - Formats are worked out from the data itself (surveying every instance of a resource type) plus community
     documentation (burnout.wiki, DGIorio's tools).
   - Game *behaviour* comes from data wherever possible. Most tuning and rules live in GeneSys objects
     (≈190k GenesysType/GenesysObject resources), so reading GeneSys is the key to game logic.
   - Where behaviour is code (handling, AI decisions, game rules), read what the code does (GeneSys fields are
     read in place, so `mwtool genesys-types` offsets locate the code that uses them), write it down in prose in
     [decomp.md](decomp.md) and reimplement it. Never copy decompiled code into the repo.
   - Check against the running original: `mwtool memscan` / `telemetry` read its live state and missing
     constants (`handling.watch`; handling.md "Measuring the original").
3. **Bevy moves fast.** It is pinned to one minor version (currently 0.19). Engine-specific code stays inside
   `crates/mostrusted`; format crates never depend on Bevy. Upgrade Bevy deliberately, in its own change.

---

## Where it stands against the original (user review, 2026-10-04)

The game runs, streams Fairhaven and lets you drive any car, but it does not yet play like Most Wanted. The
user's review after session 3, with where each gap is tracked:

| Gap | State | Item |
|-----|-------|------|
| Handling does not feel like the game | the model follows the game's code since session 3, but grip scales, the drift side force and the handbrake are unmeasured; needs recordings of the original | M3.2 |
| Tyre screech sounds as if a horn is held at the same time | a tonal layer plays with the screech; not investigated | M5.4 |
| Shading has a yellow tint over everything | the tonemap / ColourCube grade / ambient are suspects; compare against screenshots of the original at the same place and hour | M1.5, M1.7 |
| Animations are missing | none play: driver, steering wheel, animated body parts (`PhysicsDrivenVehicleAnimationBehaviour`), world animations (`TriggerAnimationBehaviour`, `AnimatedDynamicEntityBehaviour`), tree wind | M3.8 |
| Inputs are not mapped like the original | our own keyboard / pad layout; the game's bindings not found yet | M3.9 |
| UI incomplete, no Easydrive | HUD (speed, nitrous, odometer, minimap) and our own menus; the game's front end and Easydrive (`EasyDrive_ScreenScript`, `Widget_EasyDrive*`) not built | M7 |
| No crash camera | crash damage works, but no crash cam (`Physics.CrashingRules`, `CameraSequenceItem` / `CameraShakeSequenceItem` sequences) | M3.3 |
| No AI vehicles | no traffic, racers or cops | M6.1, M6.4 |
| Prop collisions | decoded and simulated but opt-in (`--prop-physics`): the car wedges against knocked props | M3.1 |
| Game logic in general | no events, pursuits, Most Wanted list, jack spots, progression | M6 |

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
   - `[x]` **Backdrop composition** (2026-10-04): InstanceList backdrops are per-unit LOD views; drawing all
     169 stacked every tier ("buildings on streets, mountains"). `ZoneList` (PVS.BNDL) is decoded and
     `mw_assets::backdrop` keeps, per place, what the owning zone lists. Still open: cut faces where zone
     views meet, near water (rivers) that no instance list draws.
4. **Shader families: G-buffer inputs** `[~]` (WGSL in `layered.wgsl`, selected by `materials::Look`). Each
   family computes exactly what the game's G-buffer pass writes (normal, roughness, sqrt albedo, AO, F0,
   Fresnel); today these feed Bevy's PBR lighting, later the ported deferred pipeline (item 7). Per-family
   decodes: research log 2026-10-04 (shaders).
   1. `[x]` TerrainPBR: splat albedo, three tiling normal maps (z reconstructed, roughness in w), world-space
      macro normal, per-layer roughness / F0, AO = 2·mask.g. Open: the TangentSpaceNormal macro frame.
   2. `[x]` DriveableSurface: albedo, slope-space normal blend (road + world-space detail + vertex), roughness =
      Normal.a × Detail.a, F0 = SpecColour.g, no cube reflection when dry. Line variants: line albedo, normal,
      roughness / F0. Done (2026-10-04): the wet path (global puddle map `…7811`, RoadDepth, vertex wetness mask
      TEXCOORD5.x: flatter normals, smoother, darker, F0 0.02 in puddles, planar reflection × env-BRDF × wet;
      always on, as in the game), vertex-alpha AO, planar reflection (`--planar-reflection`; sky cube
      otherwise). Open: `WetScaleParams` values, the reflection's mip chain, retroreflective headlight glow,
      decal variants, `PlotPBR_AO_StandingWater` (same wet maths, 28 meshes).
   3. `[x]` PlotPBR: albedo, blended tiling normals, roughness through the decal, per-instance AO map.
   4. `[x]` WorldPBR (every variant) and World_Diffuse_Specular: normal map, roughness = spec.y, F0 =
      cubic(spec.x) or SpecularColour × MaterialDiffuse, AO map (material Overlay or per-instance
      `AOMapTextureSampler`), instance tint and AO from the InstanceList w column / `diffuseTintColour`.
   5. `[~]` Vehicles: textured parts (F0 = spec rgb, roughness per variant: `PbrMaterialRoughness`, normal
      alpha, or normal green for `_Emissive_`; normal mapping only on `NormalMap` variants), vertex AO /
      specular occlusion, solid parts from `PbrMaterial*` constants, black tyres (their texture slots are
      unbound), paint from the car's `VehiclePaint` (`--paint-index`) with a clear coat. Lights done
      (2026-10-04): lightmapped lights (`EmissiveFourChannel`, `Emissive`, `Lightmap`, `BodyPaint_Lightmap`),
      lamp interiors (`Refraction`: parallax, LED channels, reflector), lens filters (`Glass_Colourise`,
      multiply), light state mask (`--lights`, brake / reverse follow the pedals). Glass done: `Glass*` with
      vertex-colour / override volume, Fresnel sky reflection, running light (forward, Bevy diffuse light).
      Open: metallic flakes / pearlescent, livery and two-paint masks, `LocalEmissive` (5 meshes), dirt / dust
      / scuffs (runtime `GenericVector4_A/B/C`), cracked glass, tyre textures (bound at runtime?), the glass
      shader's own key light + irradiance (uses Bevy's lighting), light coronas / beams (VFX).
   6. `[~]` Trees: premultiplied albedo `(rgb/a)²`, vertex AO, alpha test 129/255. Open: wind
      (`amplitude`/`speed`: object-space x, z += sin(Time × speed + uv.y) × amplitude × uv.x; per material and
      time-driven only, the keyframe wind does not reach it; needs a vertex shader for `Layered`). Water:
      scrolling wave normals, algae albedo, near-mirror reflection. Open: river
      floor (TEXCOORD2), foam, caustics.
   7. `[ ]` Lightmaps: the lightmap shaders read `LightMapTextureSampler` (TEXCOORD1) but neither the material
      nor the instance overrides bind it; find the source. Lit windows (night emissive) not drawn.
   - Longer term: translate DXBC to WGSL automatically (DXBC → SPIR-V via dxil-spirv/vkd3d, then naga), as IW4L
     does for SM3, instead of hand-porting each family.
5. `[~]` **Environment**: the timeline is GeneSys (`EnvironmentTimeline` / `EnvironmentKeyframe`,
   `mw_assets::environment`). Key light, ambient rig, sky gradient / sun colour / cloud layer and fog come
   from the keyframe at `--hour`. Done (2026-10-04, `weather.rs`): the timeline painted under the camera
   (`WorldPaintMap`), hours blended between keyframes, rebuilt at runtime (F5 / F6 hour, F7 weather), the
   `Alternate` (bad-weather) keyframes (`--weather`). The data has no rain. Open: sun direction
   (SunMoonPath), the `NewSky` dome mapping and its second gradient, fog in-scattering and layer fog (the
   `LayerFog` paint map is decoded), bloom, what selects Alternates, wind (cloud scroll; no consumer found),
   `Thunderstorm_Intensity`, blending when the painted timeline changes, the upper daytime sky rendering
   near-black in the main view (also on master before the weather merge). **User review (2026-10-04): a yellow
   tint lies over the whole image**; check the ColourCube grade, exposure and ambient against screenshots of the
   original at the same place and hour.
6. `[x]` **Vertex data**: packed tangent-frame quaternions (world RGBA8 `TEXCOORD1/2/3`, vehicle SNORM16
   `TEXCOORD4`) decode to normals + tangents (`mw_formats::tangent_frame`, checked against face normals);
   vertex colour is decoded.
7. `[~]` **The game's deferred pipeline** (see "Recreate the game's architecture"): done: the G-buffer pass
   writes the game's four targets (Bevy's deferred prepass as host), the ported key light + ambient pass and
   Fog pass (`deferred.rs`, `game_lighting.wgsl`), the game's 3-cascade shadow lookup on Bevy-rendered cascades.
   The game's auto exposure, tonemap and ColourCube grade (`tonemap.rs`).
   Open: the game's cascade fit / split distances, shadow-only proxy casters (`WorldShadow_*`), the irradiance quadric / second
   irradiance cube, point / spot / cop lights (`LightBuffer_*`, `TiledLighting_*`), planar reflection
   (`[~]` opt-in mirrored camera for wet roads; not yet for `WorldPBR_Horizontal_PlanarReflection`),
   bloom and the composite tint in `GBufferComposite`, motion blur, `PostAA`.

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
   loader thread; quads split strip-wise since 2026-10-04, which closed the holes). Collision tags decoded (surface index into
   `SURFACELIST.BNDL`, flags; `CarSurface`), and props / compounds / dynamic objects collide and break free
   from their `PhysicalDefinition` data (`props.rs`, `--test smash`, 2026-10-04). **Not usable yet**: opt-in
   (`--prop-physics`) because the car wedges against knocked lamps / crash barrels and stops dead (research
   log). Still open: that, the game's smash rules (`CollisionResponses_*`), compound parts as separate pieces,
   surface grip in the tyre model, a clean perf A/B.
2. `[~]` **Vehicle physics**: since 2026-10-04 (session 3) the model follows the game's own code, read from the
   user's decompiled dump ([decomp.md](decomp.md), [handling.md](handling.md) "The model"): 60 Hz tick; engine
   inertia, friction curve, clutch, shift on rpm and speed, top-gear lock (`drivetrain.rs`); per-wheel spin with
   brakes and axle LSDs; tyres as capped velocity constraints with the game's curves and slip blends, no ABS /
   TC (`tyre.rs`); spring from sag, asymmetric dampers, anti-roll bars, per-axis drag and lift, in-air gravity and
   take-off damping (`chassis.rs`); the game's steering rules and drift state machine (`drift.rs`).
   `--test accel|brake|corner|handbrake|drift|donut` measures it on a flat track, tick-exact. Still open: the
   absolute grip scale (model 0.4 / 0.7) and the other missing `.bss` constants (memscan, or the original memory
   dump), the handbrake lock condition, the steering neutral offset, wheels as bodies instead of rays, and
   calibration against the original game. **User review (2026-10-04): it still does not drive like the
   original.** Next: record the original with `handling.watch` (corner, braking, launch, handbrake turn,
   brake-tap drift, donut, slides) and fit against `--test` runs with the same inputs; resolve the drift side
   force (with the game's cap of 5 ours reaches ~3 g, so its size or direction is misread).
3. `[~]` Cameras from each car's `VehicleCameraContainer` (`camera.rs`, 2026-10-04): chase, bonnet and bumper
   modes (C / gamepad Y, `--camera-mode`); the chase camera's yaw spring (speed band, drift spring), speed
   movement, acceleration spring, roll and pitch from the named thresholds, high-speed and impact shake, world
   collision, look-back (B / right stick back) and look-around (right stick, right mouse drag) that springs
   back; the free camera looks with the right stick. **Missing: the crash camera** (cut-away replay of a crash /
   takedown; look for it in the crash sequences: `Physics.CrashingRules`, `CameraSequenceItem`,
   `CameraShakeSequenceItem`, the decomp's crash path). Still open: the in-car cameras' unnamed fields (rigid now),
   the remaining acceleration / yaw-spring / gradient / height fields, FOV convention and calibration against
   the original (handling.md, "Cameras").
4. `[~]` The world streams nearest-first from the spawn point (or the `--camera` pose). Still open: streaming that
   follows the car (see M4).
5. `[x]` **Car menu** (`menu.rs`, `garage.rs`; Tab / F1 / gamepad Start): pick any of the 93 drivable cars and
   one of its paints mid-session. The loader thread takes runtime `CarRequest`s (between track units, without
   reloading the world); the new car replaces the driven one in place (position, heading, velocity, a gear to
   match) or drops onto the ground below the free camera; "free camera" leaves the car. Old cars despawn with
   their colliders and engine sound; models stay cached (paint variants are separate cache keys), so repeated
   swaps add no meshes, materials or entities. `--swap-to a,b@paint,free --swap-delay N` tests it headlessly.
6. `[~]` **Crash damage** (2026-10-04): car bodies are skinned to their damage `Skeleton` and posed from the
   per-zone damage clips of the `VehicleDamageBehaviour`'s `AnimationList` (formats.md "Vehicle damage",
   `crates/mostrusted/src/damage.rs`). Body-collider impacts → `ImpactDamageGraphs` → zone levels → clip
   frames: bonnets buckle, bumpers / plates / lights / mirrors break off; R repairs. `mwtool damage <car>
   [--level front=0.5 --preview out.png]` inspects and previews it, `mostrusted --drive 911 --test crash
   [--crash-angle 35] [--crash-distance 250] --screenshot <scratch>/crash.png` drives into a wall and saves
   before / after shots. Still open: the game's impact → damage rules (curve choice, units, zone split,
   landing damage), the crumple normal map (damage volume grid in the vehicle shaders), hinged body parts
   swinging and detaching as physics debris (`Bodypart`), `WheelDamage` alignment, damage sounds / sparks
   (`DamageSequence`), handling effects, car-vs-car (the detection is generic; needs a second car).
7. `[~]` **Particle effects (VFX)**: the game's `BearEffect`s (`mw_formats::bear`, all 214 parse) and the
   GeneSys tables that place them (`mw_assets::vfx`), run by a CPU runtime that executes each layer's action
   groups (`particles.rs`) and drawn with ports of `VfxParticles_Diffuse*` (lit, premultiplied, atlas frames)
   and `VfxParticles_MotionBlurSpriteUntex` (analytic motion-blurred sparks). Done: tyre smoke from sliding
   tyres, kick-up behind rolling wheels, impact spark bursts and scrape sparks from the car body's contacts
   (`ImpactEvent`). Open: what the game feeds `WheelParams` / `Intensity` (code) and the controllers' input
   selectors; surface ids from collision tags (dirt / grass dust; everything is tarmac now) and wet surfaces;
   mesh debris renderers (0x1E/0x1F/0x22/0x23 with the bundle's debris models), light particles (0x0F/0x30),
   splatter (0x4D), super-emitters (0x2B); `OnRims`, burst tyres, spike strips, `VehicleImpacts` /
   `VehicleScrapes` for car-to-car contacts; tyre marks (`Vfx_TyreMarksNew`, `TyreMarksMaterials`); smoke
   colour (`Vehicles.SmokeColour`); world effects: 2,302 `WCVfxBehaviour` objects in the track units list
   `Spot_effects` (effect + `Locator_group`) and 1,095 an `ImpactEffect` (prop smashes), placement through
   the locator groups not decoded; soft-particle depth fade check;
   `gVfxSparkBrightness` and the exposure term; sort groups.
8. `[ ]` **Animations**: nothing animates yet besides wheels and damage poses. Car: the driver and steering
   wheel, animated body parts (`PhysicsDrivenVehicleAnimationBehaviour` / `.AnimatedBodyPart`: spoilers, flaps,
   suspension parts), `Genesys.Vehicles.AnimationStateBehaviour`, `AnimatedRaceCarBehaviour` (scripted cars).
   World: `TriggerAnimationBehaviour`, `AnimatedDynamicEntityBehaviour` (gates, barriers), tree and flag wind.
   Needs the `Animation` / `AnimationList` decoding that damage already uses.
9. `[ ]` **Input mapping like the original**: today our own layout (W/S/A/D, space handbrake, Tab / F1 menu, C
   camera, F2 volume; pad triggers, stick, A handbrake, Y camera, Start menu). The original's keyboard and pad
   bindings (nitrous, handbrake, camera, look back, Easydrive, horn, ...) are not found in the data yet; find
   them (GeneSys / UI data, or the decomp's input code) and match them, including remapping.
- Done when: you can pick a car from the catalogue, spawn on a road and drive around the city with collision.
  **Basic version reached on 2026-10-03; in-game car picking since 2026-10-04 (item 5).** Remaining: handling feel, real tuning data, spawn points on roads
  (from `RoadData` / AI data rather than hand-picked coordinates).

### M4: Streaming and performance `[ ]`
1. Load and *unload* track units by distance; budget memory (bundles are dropped after decode already).
2. Partial bundle reads: load only needed resources instead of whole files (offsets are in the TOC).
3. Model LOD distances (`Model.lod_distances`) and `HAWAII/PVS.BNDL` visibility. The ZoneList is decoded
   (zones = track units, polygons, neighbours); a streaming world would load the current zone and its
   neighbours and draw only the current zone's backdrop (`--backdrop-zone N` shows one), instead of the
   static composition used while everything is loaded.
4. GPU instancing for repeated models; profile with Tracy (`bevy/trace_tracy`).
- Target: steady 60 fps at 1080p while driving through downtown on mid-range hardware; startup to first frame
  under 3 s.

### M5: Audio `[~]`
1. `[x]` EA audio decoders (`crates/mw-audio`): Ginsu (EA-XAS v0) and SPS / `Wave` (EALayer3 → MPEG frames →
   nanomp3). `mwtool audio`, `mwtool engine-sound`.
2. `[~]` Engine sound: the car's whole `EngineSound2` bank plays through a generic sequence runtime
   (`mw_audio::sequence`, built by `mw_assets::sound`, driven by `mw_assets::engine_driver`): all six always-on
   sequences (Ginsu layers, idle loops, load crossfades, bus automation), the mixer graph with its EQ /
   distortion / sub-harmoniser / sends, pops, backfires, ignition, rev limiter, gear-change clunks; per-car
   loudness normalised (research log "the engine plays the car's whole sound bank"). Ground truth checked
   against the raw recordings (research log "engine ground truth": decoder clean, grains are phase-aligned
   cycles, single-layer resynthesis at the recording's own floor). The game's own rules from its code (research
   log "engine sound from the game code"): audible RPM = 1000..8000 × (idle..max fraction)^(2 − a) with `a`
   from `EngineSound2`, load = speed-blended, smoothed and peak-held demand, harmonics from the firing frequency
   × cylinders, Ginsu RPM glide 33 ms, Peaking EQ confirmed RBJ (linear gain), per-car DspParam Frequency / send
   overrides, A/B env switches (`MW_AUDIO_DRY`, `_NO_DIST`, `_NO_SUB`, `_NO_EQ`, `_DIST=raw`, `_RPM_LINEAR`).
   Still open: the Distortion and SubHarmoniser laws (their code is not in the decompilation: stand-ins), the
   remaining DspParam indices, the state lists (Burnout / Drift / In_air / Reversing), bus / LFO items,
   `EngineGainDueToPower`, reverb; a listening check of the new RPM curve.
3. `[x]` Music: shuffled `UI/SONGS/*.SPS` playlist. Still open: radio/track names, ducking.
4. `[~]` Vehicle sound effects from the sound data (`mw_assets::sfx`, `crates/mostrusted/src/sfx.rs`): tyre
   screech (the default surface's Drift / Spin / Brake sequences driven by wheel slip), road noise, wind,
   impacts and scrapes from the `CollisionSoundSet`, landings (research log "tyre screech, road, wind,
   impacts"). **Bug (user, 2026-10-04): the tyre screech sounds as if a horn were held at the same time**:
   a tonal layer (wrong sequence, wave or pitch binding) plays with it. Open: that, per-surface sounds
   (collision tags), bumps, ambience, wall reflections, horn, nitrous, speech.
5. `[x]` Volume settings (F2 / gamepad Select overlay, `--volume`, `~/.config/mostrusted/settings.cfg`) and the
   live sources play with `ONCE` (LOOP buffered them 0.74 s at a time: the engine held each rpm, then jumped).

### M6: Gameplay `[ ]`
Nothing of the game's logic runs yet: no AI vehicles, events, pursuits, progression. The decomp now shows the
code side (game modes, HSMs, AI) next to the data. Roughly in dependency order:
1. Traffic: `TrafficLaneData` (0x701), `TrafficGraphicsStub`, traffic vehicle attribs.
2. Road network and AI paths: `RoadData`, `RoadMapData`, `AIData`, `NavigationMesh`.
3. Triggers and events: `TriggerData`, `GAMELOGIC/GAMEMODES.BNDL`, `GAMEPLAY.BNDL`, Jack spots, Most Wanted list.
4. Cops / pursuit, takedowns, wrecks (deformation itself landed under M3.5: there is no `DeformationModel`;
   damage is skeletal animation).
5. `HSMData` (0x501): hierarchical state machines that drive game flow. Interpret, or reimplement by hand.
6. Characters / cutscenes (`CharacterSpec`, `Skeleton`, `Animation`) are low priority.

### M7: Front-end and release `[~]`
- `[~]` Native Bevy UI (fonts from `UI/FONTS` if feasible), menus, car select, settings, save data (our own format).
  Done: an in-game car select (`menu.rs`: catalogue by make / model / year, type-to-search, paint swatches from
  each car's `VehiclePaint`, keyboard + mouse + gamepad). It uses Bevy's ASCII-only default font, so names are
  folded to ASCII ("Coupe"). Done (2026-10-04, paused WIP): the UI data (GeneSys screens / layouts / bindings,
  `mw_formats::ui`, `mwtool ui screens|show|render|font`), the game's bitmap fonts (`UI/FONTS`, `game_text.rs`)
  and a free-roam driving HUD (`hud.rs`: speedometer, nitrous gauge (full: no nitrous yet), odometer, minimap
  from the map tiles; `--mph`, `--no-hud`, F3 = debug text). Done: volume settings (F2). **Missing: Easydrive**
  (the in-game d-pad menu: `EasyDrive_ScreenScript`, `EasyDriveSpeech_ScreenScript`, `Widget_EasyDrive*` type /
  tier / link / hook definitions) and the game's front end. Open: those, the car menu in the game font, widget
  adapters (which layouts show when), HUD polish (faint speed digits, odometer ".0 KM"), save data.
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

**Start with [next-session.md](next-session.md).** Priorities from the user's review (2026-10-04):

1. **Handling against the original**: record it with `handling.watch` and fit (M3.2); the drift side force.
2. **Tyre screech "horn"** (M5.4) and the **yellow tint** (M1.5 / M1.7): both are visible / audible every
   second of play.
3. **Input mapping** like the original (M3.9) and the **crash camera** (M3.3).
4. **Easydrive** and the game's HUD / front end (M7).
5. **Animations** (M3.8): driver, steering wheel, animated body parts.
6. **Prop collisions** usable by default (M3.1).
7. **AI vehicles**: traffic first (M6.1), then racers and cops; game logic (M6).
