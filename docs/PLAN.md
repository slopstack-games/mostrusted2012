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
     constants (`handling.watch`; handling.md "Measuring the original"). Since 2026-10-04 code may also run
     *inside* the user's running copy for measurement and verification (the probe: [probe.md](probe.md),
     policy in reverse-engineering.md "Code inside the running game"): tick-exact logs of per-wheel loads and
     caps, data pokes, and our Rust port of a function substituted for the original's to prove it exact. Since
     2026-10-06 the policy also allows mods of the original (anything but a DRM bypass; probe.md "Tier 3").
3. **Bevy moves fast.** It is pinned to one minor version (currently 0.19). Engine-specific code stays inside
   `crates/mostrusted`; format crates never depend on Bevy. Upgrade Bevy deliberately, in its own change.

---

## Where it stands against the original (user review, 2026-10-04)

The game runs, streams Fairhaven and lets you drive any car, but it does not yet play like Most Wanted. The
user's review after session 3, with where each gap is tracked:

| Gap | State | Item |
|-----|-------|------|
| Handling does not feel like the game | re-read from the code 2026-10-06: drivetrain (14-state gearbox, live constants), tyres (μ × static load + downforce share, no calibrated scale; 2.6 g skidpads), steering, drift (side force outwards) and handbrake (no lock). The brake-tap drift starts as in the original (2026-10-07: replays of `ref/live1` enter on the same tick; the old test script would not drift in the original either; open: one over-rotation in the drift, `AngularDamping(0.4)` unsourced), the low-speed standstill hold, a few `.bss` constants; needs recordings of the original. Session 5: the tyre pass's brake hold and the standstill hold / downshift ground-time gate ported; on `ref/xkr_launch` the launch slip now roughly matches, its braking phase is two collisions in the original and cannot judge the brakes (a clean braking capture is needed); the steering's neutral offset (a counter-steer assist that follows body slip) ported from the telemetry relation. 2026-10-09: no general angular damping in the original (read live), post-impact yaw damping ported; our 0.4 damping kept as a model decision until the missing yaw resistance in slides is found | M3.2 |
| Tyre screech sounds as if a horn is held at the same time | fixed 2026-10-06 (not yet heard by the user): the 32 kHz waves decoded to a 47-sample loop (a 681 Hz buzz); skid intensities now follow the game's `FUN_009d1bf0`. A capture of the original would confirm | M5.4 |
| Shading has a yellow tint over everything | `[~]` main cause fixed 2026-10-06: the ColourCube bytes were read with red and blue swapped, so the noon grade warmed instead of cooled (noon frame warm bias +33 → +13). Session 5: the cloud layer was decoded as sRGB (dark patches in the day sky), fixed; the bloom, vignette and colour-cube blend inputs are traced to renderer fields but each needs a live read before it can be ported without guessing (research log). Left: those live reads, a screenshot of the original, the provisional skylight, night warmth | M1.5, M1.7 |
| Animations are missing | none play: driver, steering wheel, animated body parts (`PhysicsDrivenVehicleAnimationBehaviour`), world animations (`TriggerAnimationBehaviour`, `AnimatedDynamicEntityBehaviour`), tree wind | M3.8 |
| Inputs are not mapped like the original | `[~]` 2026-10-06 (session 5): the original's action list, default keyboard / pad tables and mapping rule read from the code and made our defaults (`input.rs`: arrows / A / Z / Space, RT / LT / X, C / LB camera, numpad / d-pad Easydrive). Open: look back / look around have no default binding in the tables (ours: End / right stick), nitrous and horn have no runtime yet, remapping | M3.9 |
| UI incomplete, no Easydrive | `[~]` 2026-10-06 (session 5): Easydrive's menu tree is read from the UI data (23 tiers in the free-roam screen) and shown in game (`easydrive.rs`, `--show-easydrive`); Change car works through it, every other entry is inert. Its look and position are our own (the game's layouts are not read), conditional entries are not filtered. 2026-10-09: **start-up flow from the game's data** (`frontend.rs`): title screen (sequence 1022357, title movies through ffmpeg, PRESS ANY KEY), the boot transition holding over the loading (car pictures, tips), the in-game loading screen 108177, the free-roam start (new game: the default rollout at start trigger 1156487; continue: the saved car at its nearest swap spot), the pause menu 431082 (`pause.rs`: CONTINUE, MAP, SETTINGS). 2026-10-10: the map screen 899991 (`map_screen.rs`, event selection) and the event-loading card (`event_card.rs`). Not built: car select / customise screens, the online and Autolog pages, the new-game intro event | M7 |
| No crash camera | crash cam since 2026-10-06 (`crash.rs`): rules, slow motion, bystander shots, letterbox and rolling reset from the data; the shots' placement / FOV and the reset pose are model decisions until compared with a capture of the original | M3.3 |
| No AI vehicles | `[~]` **Traffic runs** (2026-10-09, `traffic.rs`, `--no-traffic`, `--traffic-density`): the TR car models in their palette paints spawn, follow the lanes, stop at red lights, queue behind cars and the player and are removed by the game's rules (active cells, emitters, density = the event's base (free drive 1.0) × `Density`, load budget of crashing cars, fuzzy option choice, speed controller, route by span load, junction light cycles; decomp.md "Traffic runtime"); hit by the player they become crashed bodies; hard hits on traffic and AI cars crash by the game's car-against-car rules (2026-10-09, `car_impacts.rs`: crash camera for the player, AI reset, wrecked traffic). Model decisions: no manoeuvres (swerve / cut-up), crashed cars are boxes without wheels, models only within 250 m, no per-car lights; parked cars placed and spawned by the game's rules since 2026-10-10 (static vehicle records, hit ones crash like traffic); traffic engines sound since 2026-10-09 (the game's pool of five, `car_audio.rs`), horns and crash / skid sounds since 2026-10-09 (`car_sfx.rs`). No racers or cops 2026-10-09: **AI racers run**: every car steps the same physics from its own controls (`CarControls`, `Player` marker); the racer's driving law (state 8: steering PID on the look-ahead error with the car's `PIDController` gains, pedal PID, nitrous, emergency brake, crash recovery and reset) and the rubber band are ported (`ai_driver.rs`, `ai.rs`; decomp.md "AI racers"); `AIData` (the road graph) decoded. `--race <id>` puts opponents on the event's grid and they finish its laps; `--ai <cars>` / `--test ai` on a road-graph route. Open: the racing line (`FUN_004e70e0`), the lane planner, racer interaction, the opponents' own cars, a live check that physical racers really have no corner speed. No traffic or cops yet (traffic lanes decoded 2026-10-06) | M6.1, M6.4 |
| Prop collisions | `[~]` **On by default** since 2026-10-09 (`--no-prop-physics` opts out): the game's car-against-object response (the car takes 0.1 / 0.05 of the contact impulse, `RaceCarVsDynamic`), the momentum break-free rule, reset when camera and car are past the reset distance, impact effects on `ForceToDamage` and impact sounds (decomp.md "Props"); props activate within 30 m of a car. Compounds break into their parts (`ForceToSmash` after `08c87eeb`, smash proxies, links; 2026-10-09) and loose props sound when they land. Open: the damaged model, the compound "Collision FX" effect, the sound threshold, the game's pools | M3.1 |
| Game logic in general | 2026-10-09 (later): race HUD, 3-2-1 countdown and results from the game's layouts (`hud.rs`), races startable from Easydrive's RACES tier. 2026-10-09: events decoded (`mwtool events`: 111 offline events with checkpoints, starts, grids, cars) and races playable from start to result (`--race <id>`, `race.rs`: the traced intro / freeze / gate / lap / position / wrong-way / finish rules, the original's chevron route markers; `--test race`); no AI opponents, HUD, intro cutscenes, pursuits, Most Wanted list or progression yet. Later 2026-10-09: **race cutscenes** play from the game's sequences (`cutscene.rs`): the event's intro cinematic (camera clips, ICE camera takes decoded in `mw_formats::camera_take`, rolling start on the grid, skip with Esc / Back; offline races have no countdown, `--no-cutscenes` keeps the frozen 3-2-1 start) and its outro after the finish; camera axes, horizontal FOVs, `DUTCH` and the outro's frozen camera settled by captures of the original (2026-10-09); open: audio / UI / post-FX items. 2026-10-06 (session 5): the free-roam objects are decoded (`mwtool freeroam`: 74 speed cameras, 216 billboards, 152 gates, 183 jack spots, 22 body shops) and speed cameras log the pass speed (`freeroam.rs`, the game's 50 mph threshold); no HUD message, Speed Points, smashes or jack-spot swaps yet | M6 |
| Road / terrain textures scaled wrong, foliage in the wrong place (user, 2026-10-06) | `[~]` Texture scale: every ground family's UV maths, the mip chains, UV formats and detail strength match the game; three road decal variants (465 meshes) and the anisotropy (the game's 4×) fixed; **not resolved** (lead: the texture header's first-mip byte and a global quality flag, needs a live read and a capture of the original). Foliage: placement decodes are right (`mwtool placement-check`); groundcover uses the game's fields and group draw distance; **the cause was a forward redraw**: masked materials (trees) were drawn again without fog and with Bevy's lighting, fixed the same day; hills beyond 500 m are still flat fog colour (gradient texture alpha not traced); floating groundcover at the CONTINENTAL DRIFT start fixed (2026-10-09): only the collections of the camera's zone and its neighbours are drawn, and sprites are byte/127 × 2^e m (the e = 6 / 8 groups are tree cards that forest the hills) | M1.3, M1.4, M1.5 |
| Cars stuck in the floor after jumps / steep climbs, jitter, "stuck in a gear" (user, 2026-10-06) | `[~]` sinking fixed (the game's hard strut stops; `--test landing`, `--test climb`); gear holding: the cruising gearbox and its mode switch ported, 1st gear at part throttle still revs high; jitter fixed (the anti-roll bar's damping coefficient from `FUN_00933a60`) | M3.2 |
| Liveries, dirt, active aero, nitrous, mods (user, 2026-10-06) | `[~]` liveries drawn; dirt / dust build up per wheel from the surfaces' `VehicleEffects` and damage scratches the paint (2026-10-09, `paint_wear.rs`, decomp.md "Vehicle paint, liveries and dirt"; open: the per-wheel skid values, wet surface block, rain drops, crumple normal map); two-tone paint bound; active aero for the speed-only cars; nitrous (tank, refill events and burn from the code, 2026-10-09: decomp.md "Nitrous refill"; takedown / crash escape / gas station / gate awards not fed, speedbreaker and jump modes not run); mods chosen through Easydrive and applied (about half of the parameters identified) | M1.4, M3.2, M3.8 |
| UI, tyre marks, optimisation, camera, water (user, 2026-10-06) | `[~]` Easydrive drawn from the game's layout (its RACES tier starts races, 2026-10-09); race HUD / countdown / results from the game's layouts and additive UI blending (2026-10-09); tyre marks drawn (lay rule is a model decision); `--bench` and a baseline (51 fps driving downtown, shadows cost 6.6 ms; mesh merging gave nothing); 2026-10-09: direct draws, inline tree optimisation and bakes off the frame's cores against frame drops (M4.7); the chase camera's update found (`FUN_005d4970`) and, 2026-10-10, ported stage by stage and checked frame by frame against `ref/live1` (`--camera-replay`, `tools/camcompare.py`: eye within 2–4 cm, view pitch 0.23°, FOV 0.23° RMS); no wall push-in, as the original (its whole camera path read with REA, 2026-10-14; `MW_CAMERA_PUSH_IN=1` for A/B); `Water_Opaque` ported; groundcover tufts lit | M7, M3.7, M4, M3.3, M1.4 |
| Porsche badge text and texture broken (user, 2026-10-09) | fixed 2026-10-09: every car's `_Alpha1bit` badges / logos / hub caps now cut out at the shader's 0.5 (diffuse or normal-map alpha) and the `_Alpha_` parts blend alpha-over (MaterialState); those blended parts use Bevy's forward lighting, `g_wheelConstants.x` (`_BlurFade`) taken as 0 (research log) | M1.4 |

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
   - Done: wheels sit in the arches (right side mirrored), and `--vehicle` accepts a car name. Wheel positions
     follow the game's rule from `VehicleInfo`'s axles, not the spec (2026-10-10, `mw_assets::wheel_layout`;
     player, traffic, viewer; checked live on the Raptor). Still open: LOD
     selection by distance, and skinning to the skeleton.
3. `[x]` **Remaining world layers** from `ZoneHeader` (0x206): CompoundInstanceList / CompoundObject,
   PropInstanceList / PropObject, DynamicInstanceList, LightInstanceList, GroundcoverCollection / Instances.
   - Done: props, compound objects and dynamic objects are placed (+21068 placements over the map, 0
     unresolved; `mwtool world` prints counts per layer); lights are drawn since 2026-10-10 (point / cone / spot, plus the street lamps' own lights; `world_lights.rs`).
     Groundcover is decoded and drawn (1.36 M sprites over the map, one mesh per group, `--no-groundcover` to
     skip); since 2026-10-09 by the port of the game's `Groundcover` vertex program (camera-facing about world
     up, wind sway, the CPU builder's per-sprite range drop and shrink, `groundcover.wgsl`). Still open: light
     rendering, the builder's 20000-sprite / 300-draw caps, compound sub-parts / LOD slot choice.
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
      Badges / decals done (2026-10-09): `_Alpha1bit` cut out at the program's 0.5 (diffuse or normal-map
      alpha), `_Alpha_` forward alpha-over from their MaterialState (lit by Bevy, not their forward program).
      Open: metallic flakes / pearlescent, `LocalEmissive` (5 meshes), the paint's crumple normal map / AO
      (`CrumpleTextureSampler` at TEXCOORD1, weighted like the scratches) and rain drops (`GenericSamplerB`,
      `GenericVector4_A.zw`), cracked glass, tyre textures (bound at runtime?), the glass
      shader's own key light + irradiance (uses Bevy's lighting), light coronas / beams (VFX).
   6. `[~]` Trees: premultiplied albedo `(rgb/a)²`, vertex AO, alpha test 129/255. Open: wind
      (`amplitude`/`speed`: object-space x, z += sin(Time × speed + uv.y) × amplitude × uv.x; per material and
      time-driven only, the keyframe wind does not reach it; needs a vertex shader for `Layered`). Water:
      scrolling wave normals, algae albedo, near-mirror reflection. Open: river
      floor (TEXCOORD2), foam, caustics.
   7. `[~]` Lightmaps: `LightMapTextureSampler` (TEXCOORD1) is bound by the Model (+0x10) and by instance
      overrides (found 2026-10-10); drawn for WorldPBR and the roads, not yet for PlotPBR. Lit windows and illuminated signs are drawn
      (2026-10-10; scaled by the game's night level). The roads' baked vertex light is drawn.
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
   tint lies over the whole image**: main cause found 2026-10-06 (ColourCube channel order, research log). Open:
   `EnvironmentKeyframe.Camera.TintColour` (blue at night, warm in the evening) and `BloomColour` are not
   applied yet; which CPU code feeds them into `GBufferComposite` (A, B, D) is not found.
6. `[x]` **Vertex data**: packed tangent-frame quaternions (world RGBA8 `TEXCOORD1/2/3`, vehicle SNORM16
   `TEXCOORD4`) decode to normals + tangents (`mw_formats::tangent_frame`, checked against face normals);
   vertex colour is decoded.
7. `[~]` **The game's deferred pipeline** (see "Recreate the game's architecture"): done: the G-buffer pass
   writes the game's four targets (Bevy's deferred prepass as host), the ported key light + ambient pass and
   Fog pass (`deferred.rs`, `game_lighting.wgsl`), the game's 3-cascade shadow lookup on Bevy-rendered cascades.
   The game's auto exposure, tonemap and ColourCube grade (`tonemap.rs`); the grade is the final pass built
   into `NFS13.exe` (`mwtool shader-asm --exe`, decomp.md "Post-processing and colour grade"), cube channel
   order fixed 2026-10-06. 2026-10-14: the TintBlend blends the two keyframes' cubes with the timeline weights
   (live read), the final pass is the free-roam variant (depth of field + tint) with the sun-facing brightness /
   contrast, the bloom levels have the original's fixed sizes (320 × 176, 80 × 44, 20 × 11) and the vignette's k / s
   come from the code (`MW_NO_DOF`, `MW_NO_SUN_CONTRAST`, `MW_NO_CUBE_BLEND`, `MW_BLOOM_DIVISORS`).
   Open: the game's cascade fit / split distances, shadow-only proxy casters (`WorldShadow_*`), the irradiance quadric / second
   irradiance cube, cop / line lights (point / cone / spot lights and their per-hour switch ported 2026-10-10), planar reflection
   (`[~]` opt-in mirrored camera for wet roads; not yet for `WorldPBR_Horizontal_PlanarReflection`),
   bloom and the composite tint in `GBufferComposite` (`[x]` vignette 2026-10-06, bloom 2026-10-07, level sizes
   2026-10-14), the composite's motion blur (traced, ≤ 2 px at 200 km/h, not ported), the depth-of-field blur
   passes' sub-texel offsets, the sun lens flare (`LensFlareColour`), `PostAA`.

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
   from their `PhysicalDefinition` data (`props.rs`, `--test smash`, 2026-10-04). **On by default since
   2026-10-09** with the game's rules (decomp.md "Props"): a knocked prop gives the car a tenth of the contact
   impulse (`CollisionResponses_RaceCar.RaceCarVsDynamic`, solved in `prop_contacts`), standing props come loose
   on the hitter's momentum (`67f0233c`), resets follow `mfResetDistanceSquared` from camera and car; no more
   wedging (a lamp at 60 km/h costs about 1 km/h). Compounds break into their parts (2026-10-09). Still open:
   the damaged model, the game's activation pools. Surface grip, rolling resistance and slide drag per wheel are in the tyre model since
   2026-10-06; the fade on a surface change, the drift's surface factor and the player's bumps on rough
   surfaces (`Surface_roughness` Perlin noise) since 2026-10-09.
2. `[~]` **Vehicle physics**: since 2026-10-04 (session 3) the model follows the game's own code, read from the
   user's decompiled dump ([decomp.md](decomp.md), [handling.md](handling.md) "The model"): 60 Hz tick; engine
   inertia, friction curve, clutch, shift on rpm and speed, top-gear lock (`drivetrain.rs`; since 2026-10-06 the
   gearbox state machine, shift phases, idle governor, throttle hold / lag and torque noise as in the code, all
   drivetrain constants sourced, decomp.md); per-wheel spin with
   brakes and axle LSDs; tyres as capped velocity constraints with the game's curves and slip blends, no ABS /
   TC (`tyre.rs`); spring from sag, asymmetric dampers, anti-roll bars, per-axis drag and lift, in-air gravity and
   take-off damping (`chassis.rs`); the game's steering rules and drift state machine (`drift.rs`).
   `--test accel|brake|corner|handbrake|drift|donut` measures it on a flat track, tick-exact. **Grip scale
   settled from the code (2026-10-06)**: the caps are μ × N × data scales × surface grip, N = static corner load
   + ½ the axle's downforce (not the spring force; confirmed on `ref/xkr_launch`), solved once per wheel per tick
   after the brake, wheel by wheel on the chassis's effective mass; the calibrated 0.4 / 0.7 and our Crr 0.015
   are gone (decomp.md "Tyres"). Consequences to check against the original: 2.6 g skidpads, no brake-tap drift
   from the scripted inputs, a slow low-speed stop (the standstill hold `99407bbc` is not ported). Still open:
   the tyre constraint's +0x210 blend-input scale (probe), the standstill hold's input (vehicle +0x1aa8), strut
   travel and damper slots, the other missing `.bss` constants, the steering
   neutral offset, and calibration against the original game. `[x]` 2026-10-06: update loop, steering, drift and handbrake checked
   line by line against the code (decomp.md): the tyres use the previous tick's steering angle; the in-drift
   steering remap's zero point and the counter-steer rate's gate; drift damping time base 60 (code + live `.bss`);
   the side force pushes **outwards** (sign of the drift scale, opposite to the yaw torque) with its factor capped
   at 5; side force only with every wheel down, torque / scale / speed hold always while drifting; the handbrake
   is a countdown state that blocks and ends drifts and gives the next drift its scale, a rear brake torque by max,
   never a lock; burnout releases one axle's brakes; the low-speed turn-around assist (`FUN_008f8360`) ported.
   Open from that: the speed-hold constant at `.bss` 0x01187250 (60 assumed), the counter-steer exponent
   (inferred `Max_counter_steering_power`), the brake-tap drift now spinning the 911 (needs the original's run). **User review (2026-10-04): it still does not drive like the
   original.** Next: record the original with `handling.watch` (corner, braking, launch, handbrake turn,
   brake-tap drift, donut, slides) and fit against `--test` runs with the same inputs; check the drift side force's
   direction in a recorded drift (`drift_side_f*` in the watch file).
   The quantities telemetry cannot reach (N, the grip caps, the solver's impulses, the side-force vector, who
   writes the grip multipliers) come from the probe ([probe.md](probe.md)) in this order:
   `[x]` Tier 0 `mwtool memscan poke` / `unpoke` (data only, refuses `.text`, restores on exit; 2026-10-05) →
   `[x]` Phase 0 load path (`orig launch --probe`, hello DLL as an `xinput1_3` stand-in; verified 2026-10-05) →
   `[x]` `mwtool tune`: live tuning menu beside the game (global `.bss` constants + the player car's fields,
   data pokes, restored on quit; verified on the running game 2026-10-05; probe.md "Tuning menu") →
   `[ ]` Tier 1 `crates/mw-probe` logging hooks: the detour installer is **not written by the assistant**
   (its safety system stopped it twice, the second time on 2026-10-05 when asked to try again for an in-game
   overlay; probe.md "Status"); the user decides how that step is done →
   `[ ]` first measurements: the caps and the +0x210 blend scale in a steady corner (N is settled: static +
   downforce share, 2026-10-06), side force in a brake-tap drift, handbrake lock →
   `[ ]` Tier 2: handling maths into a Bevy-free `mw-handling` crate, the lateral curve as the first
   in-place replacement validated with `orig run` + `mwtool compare`.
3. `[~]` Cameras from each car's `VehicleCameraContainer` (`camera.rs`, 2026-10-04): chase, bonnet and bumper
   modes (C / gamepad Y, `--camera-mode`); the chase camera's yaw spring (speed band, drift spring), speed
   movement, acceleration spring, roll and pitch from the named thresholds, high-speed and impact shake, world
   collision, look-back (B / right stick back) and look-around (right stick, right mouse drag) that springs
   back; the free camera looks with the right stick. **Crash camera** (`crash.rs`, 2026-10-06): a body impact
   on scenery closing faster than the player's `WorldRules` (70 mph, `FUN_009a9fd0`) is a crash: no control,
   the feedback sequence `0100000000068b43` on real time (slow motion 1 → 0.1, 0.14, 0.22; the `Freeway` and
   car-relative bystander shots; no HUD; letterbox), then the rolling reset of `GameLogicConfig`
   (decomp.md "Crashes", handling.md "Crash camera"; `--test crash` takes crash-cam screenshots,
   `MW_CRASH_TRACE`). Still open there: the damage rule (needs damage-bar units), the reset onto the road and
   its timer (`GameMode`), the bystander maths (placement, tracking, FOV are model decisions), takedown /
   landing rules (traffic / AI car impacts ported 2026-10-09, `car_impacts.rs`), a capture of the original to compare. Still open: the in-car cameras' unnamed fields (rigid now),
   the remaining acceleration / yaw-spring / gradient / height fields, FOV convention and calibration against
   the original (handling.md, "Cameras").
4. `[~]` The world streams nearest-first from the spawn point (or the `--camera` pose); since 2026-10-09 by
   `ZoneList` zone distance, each unit reported when complete, and the player's car is placed only once the zone
   at its target and the neighbours within 200 m are resident (the game's `FUN_00647e00` rule). Still open: streaming that
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
   (`ImpactEvent`); the car's spot effects (`exhaust.rs`, 2026-10-09): exhaust smoke at idle, nitrous flames,
   backfire flames from the game's pop rule (`FUN_009b20b0`), with the `UpdateBindings` controller, the cone
   emitter, emitter-space layers and the colour gradient read from the code. Open: what the game feeds
   `WheelParams` / `Intensity` to the wheel effects and the wheel spawners' binding slots (0x42, 0x05, 0x07);
   the nitrous modes' groups 9 / 20 and the wreck's 12; the cone emitter's angles; `UVDistortion`; the fade
   over distance (0x38); a capture of the original's flames; surface ids from collision tags (dirt / grass dust; everything is tarmac now) and wet surfaces;
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
9. `[~]` **Input mapping like the original** (2026-10-06, session 5; `input.rs`, decomp.md "Input and control
   bindings"): the original's 96 actions, its default keyboard table (Windows virtual keys, `FUN_00483470`) and
   pad table (`FUN_00482250`) and the per-binding mapping rule (`max(0, (raw − min) / (max − min))`, stick dead
   zone 0.2, trigger 0.1, keys 0 / 1 with no ramp) are our defaults: Left / Right steer, A accelerate, Z brake,
   Space handbrake, C change camera, Esc pause (opens the car menu), numpad 4 / 6 / 8 / 2 Easydrive; pad left
   stick, RT / LT, X handbrake, LB camera, right stick camera, Start pause, Back change vehicle, d-pad Easydrive.
   Our debug keys (Tab / F1 car menu, F2 volume, F3, F5–F7) stay. Open: look back / look around have no default
   binding in the tables (ours: End / right stick; trace the camera code reading actions 10 and 37), nitrous
   (Left Shift / A) and horn (W / B) have no runtime, remapping through the settings file, whether the saved ini
   replaces or extends the defaults, the pad object's live dead zones.
- Done when: you can pick a car from the catalogue, spawn on a road and drive around the city with collision.
  **Basic version reached on 2026-10-03; in-game car picking since 2026-10-04 (item 5).** Remaining: handling feel, real tuning data. Spawn points
  from the data since 2026-10-09 (the free-roam start trigger, swap spots; `frontend.rs`)
  (from `RoadData` / AI data rather than hand-picked coordinates).

### M4: Streaming and performance `[ ]`
1. Load and *unload* track units by distance; budget memory (bundles are dropped after decode already).
2. Partial bundle reads: load only needed resources instead of whole files (offsets are in the TOC).
3. `[~]` Model LOD distances: `[x]` the game's per-placement LOD choice and draw distance (2026-10-10, `lod.rs`,
   decomp.md "Model LOD and draw distance"; open: SHADOW renderables, car LODs); `[ ]` `HAWAII/PVS.BNDL` visibility. The ZoneList is decoded
   (zones = track units, polygons, neighbours); a streaming world would load the current zone and its
   neighbours and draw only the current zone's backdrop (`--backdrop-zone N` shows one), instead of the
   static composition used while everything is loaded.
4. GPU instancing for repeated models; profile with Tracy (`bevy/trace_tracy`).
5. **Stream around the player** (user, 2026-10-09: "just load what is needed around the player instead of the
   whole map and everything"): load and unload track units, props, groundcover, traffic models and world
   effects by the player's zone and its neighbours as the game does (its streaming / PVS code, not a distance
   guess), including the cars and AI only when an event needs them.
6. `[~]` **Loading screens** (user, 2026-10-09; done 2026-10-09 except the noted gaps, decomp.md "Boot, loading
   screens and the free-roam start"): the boot transition 1836020 holds over the stream-in with the car pictures
   and tips; the in-game loading screen 108177 (black, spinner) shows until the game's world-loaded rule holds; sound
   muted and controls off meanwhile. Open: the event-loading cards (`Events.Loading.*` transitions), the streaming
   stall (pause while the car's 25 m are not resident), the transition's sounds and road-ribbon streaks. Was: the game's own loading screens and their flow (UI screens with
   `LoadingProfile` / `LoadingIcon`, the tips / car shots they show) while the start area streams in, instead of
   showing the world assembling.
7. `[~]` **Frame drops** (user, 2026-10-09; research log "frame drops"): views draw with direct draws instead of
   Bevy's multi-draw-indirect bookkeeping (render thread 2–3× cheaper downtown, ~950 visible materials), avian's
   tree optimisation inline (no stalls on the async pool), environment bakes on their own 2 threads, `MW_FRAME_LOG`
   per-frame log, the game's model LODs and draw distances (2026-10-10: visible meshes downtown 4789 → 1688).
   Open: clamp game time to 2/60 s per frame as the game does (written, not committed), bindless materials, a static environment map for forward materials, pipeline
   pre-warm at race start.
- Target: steady 60 fps at 1080p while driving through downtown on mid-range hardware; startup to first frame
  under 3 s.

### M5: Audio `[~]`
1. `[x]` EA audio decoders (`crates/mw-audio`): Ginsu (EA-XAS v0) and SPS / `Wave` (EALayer3 → MPEG frames →
   nanomp3). `mwtool audio`, `mwtool engine-sound`.
2. `[~]` Engine sound: the car's whole `EngineSound2` bank plays through a generic sequence runtime
   (`mw_audio::sequence`, built by `mw_assets::sound`, driven by `mw_assets::engine_driver`): all six always-on
   sequences (Ginsu layers, idle loops, load crossfades, bus automation), the mixer graph with its EQ /
   distortion / sub-harmoniser / sends, pops, backfires (the game's rule since 2026-10-09: `FUN_009b20b0` starts
   `Backfire_Sequence` / `Pop_Sequence`, `GetPopProbability` binds the gurgle), ignition, rev limiter, gear-change clunks; per-car
   loudness normalised (research log "the engine plays the car's whole sound bank"). Ground truth checked
   against the raw recordings (research log "engine ground truth": decoder clean, grains are phase-aligned
   cycles, single-layer resynthesis at the recording's own floor). The game's own rules from its code (research
   log "engine sound from the game code"; 2026-10-09 "engine sound stuck in gear", decomp.md "Engine sound"):
   audible RPM over the bank's Ginsu span (not 1000..8000) × ((idle..max fraction − 0.05) / 0.95)^(2 − a) with
   `a` from `EngineSound2`, load = the engine's lagged throttle blended towards full by speed × clutch (0.5 per
   tick), gear-change sequences on the game's shift clock with their RPM kick / shunt / power-cut effects, the
   rev limiter only in the top two gears, harmonics from the firing frequency
   × cylinders, Ginsu RPM glide 33 ms, Peaking EQ confirmed RBJ (linear gain), per-car DspParam Frequency / send
   overrides, A/B env switches (`MW_AUDIO_DRY`, `_NO_DIST`, `_NO_SUB`, `_NO_EQ`, `_DIST=raw`, `_RPM_LINEAR`).
   Still open: the Distortion and SubHarmoniser laws (their code is not in the decompilation: stand-ins), the
   remaining DspParam indices, the state lists (Burnout / Drift / In_air / Reversing), bus items, the LFO items'
   writer, whether a sequence clamps its binding at `Binding_Max` (the XKR's first Ginsu set tops out at 5424),
   `EngineGainDueToPower`, reverb; a listening check of the new RPM curve.
3. `[x]` Music: shuffled `UI/SONGS/*.SPS` playlist. Still open: radio/track names, ducking.
4. `[~]` Vehicle sound effects from the sound data (`mw_assets::sfx`, `crates/mostrusted/src/sfx.rs`): tyre
   screech (the default surface's Drift / Spin / Brake sequences driven by wheel slip), road noise, wind,
   impacts and scrapes from the `CollisionSoundSet`, landings (research log "tyre screech, road, wind,
   impacts"). The screech "horn" (user, 2026-10-04) is fixed (2026-10-06, research log "the tyre screech
   horn was a decoder bug"): every 32 kHz wave decoded to its 47-sample PCM prefix and looped as a 681 Hz buzz;
   the skid intensities now follow the game's `FUN_009d1bf0` (one reference wheel, `TyreSoundParams` ramps and
   rise / fall rates; decomp.md "Tyre skid sound intensities"). To confirm against the original: capture a long
   slide (`mwtool capture slide1` while holding a 20–30° drift for 5 s, then a straight-line lock-up from 60 mph)
   and compare its spectrum with `mostrusted --replay` of the same run: no narrow lines more than ~15 dB over
   their third-octave in the drift. Open: `GetPlayerSuspensionLoad` (vehicle +0x1f28) for the skid channels'
   gains, the runtime constants of the spin ramp, per-surface sounds (collision tags), bumps, ambience, wall
   reflections, horn, nitrous, speech.
5. `[x]` Volume settings (F2 / gamepad Select overlay, `--volume`, `~/.config/mostrusted/settings.cfg`) and the
   live sources play with `ONCE` (LOOP buffered them 0.74 s at a time: the engine held each rpm, then jumped).
6. `[~]` Other cars' sounds (2026-10-09, `car_audio.rs`, `mw_assets::car_sound`; decomp.md "Other cars' engine
   sounds"): AI racers play their `<id>_COMPETITORSOUND` engine through the engine driver and the surface's
   `*_Competitor` skids; traffic plays the game's pool of five engine instances (`FUN_0060bc20`: score by speed,
   distance and view every 0.3 s, release beyond 85 m with a 1.5 s fade, steal the oldest) with the
   `TrafficEffectSequenceItem` RPM; every voice is placed by its channel's `SoundDistanceFalloff`, focus and doppler
   (`FUN_00ab9690`, `FUN_00ab7260`). `mwtool traffic-sound`, `mwtool engine-sound --competitor`, `MW_AUDIO_METER=1`.
   Traffic horns, traffic crash / skid sounds and AI racers' crash sounds since 2026-10-09 (`car_sfx.rs`, decomp.md
   "Traffic horns, crashes and skids"; `mwtool traffic-sound sfx`). Open: the player's horn, the player's impacts by
   the traced slot table, scrapes, pass-bys, occlusion, the distance filter (3D output
   slots 3–6), the panner, the bus levels between the player's sounds and the other cars', a capture of the
   original with traffic passing to compare levels.

### M6: Gameplay `[ ]`
Nothing of the game's logic runs yet: no AI vehicles, events, pursuits, progression. The decomp now shows the
code side (game modes, HSMs, AI) next to the data. Roughly in dependency order:
1. `[~]` Traffic: `TrafficLaneData` (0x701) decoded (2026-10-06, session 5; formats.md, `mwtool traffic
   [--png] [--at x,z]`, `AssetStore::traffic_lanes()` / `lanes_near`, `traffic_data()`): hulls, lane sections,
   rungs, flows and their vehicle lists, junction light cycles, stop lines. Runtime since 2026-10-09
   (`traffic.rs`; decomp.md "Traffic runtime"): spawn / despawn, lanes, lights, following, hits. Open: the
   manoeuvres, the game's own switch to a physical car and its wheel poses, (parked cars since 2026-10-10),
   per-car lights and indicators, wrecked-traffic sounds and car alarms (horns, crash and skid sounds play since
   2026-10-09), the hour
   array's reader, `TrafficGraphicsStub`.
2. Road network and AI paths: `RoadData` (decoded 2026-10-09, `mw_formats::road_data`), `RoadMapData`, `AIData`, `NavigationMesh`.
3. `[~]` Triggers and events: `TriggerData` (0x202) decoded (2026-10-06, session 5; formats.md, `mwtool
   freeroam`): speed cameras, billboards, security gates, jack spots, body shops, with the collectable rules
   from the code (decomp.md "Free-roam rules"); speed cameras, security gates and billboards
   run with their Speed Points (`freeroam.rs`, 2026-10-10: the firing rule and 4 s queue, 500 / 250 / 500 SP on
   the first find from the `ScoringAction` data, the SP total and "+SP" message on the HUD, progress in the
   profile; `--test freeroam`, `mwtool scoring`). Open: the overlap's narrow phase, the speed camera's
   world-space plate and flash, the billboard's jump-distance speedwall, SP of events / takedowns, hints,
   jack-spot swaps; `GAMELOGIC/GAMEMODES.BNDL`, `GAMEPLAY.BNDL`, the Most Wanted list.
4. `[~]` Cops / pursuit (2026-10-10, `police.rs`, `pursuit.rs`, decomp.md "Police and pursuits"): patrol cop in
   free roam, pursuit by speeding past or ramming a cop, heat levels with their cops, chase / cooldown / escaped
   / busted, the HUD's heat and BUSTING bars; `--test pursuit`, `--no-police`, `mwtool police`. Open: roadblocks,
   spike strips, the cops' ram / Bruiser / Rhino driving, sirens and lights, busted / escaped sequences, Speed
   Points, cooldown spots. Takedowns, wrecks (deformation itself landed under M3.5: there is no
   `DeformationModel`; damage is skeletal animation).
5. `HSMData` (0x501): hierarchical state machines that drive game flow. Interpret, or reimplement by hand.
6. `[~]` Cutscenes: game-logic `Sequence`s play (2026-10-09, `cutscene.rs`, decomp.md "Cutscenes and sequences"):
   race intros and outros with their camera clips, ICE takes, teleports, AI drive, animated cars, slow motion,
   letterbox and skip. Open: audio, post-FX, UI layout and speech items; nested timeline controllers; the other
   sequences (tutorials, busted / escaped, Most Wanted intros with `MovieSequenceItem`). Characters
   (`CharacterSpec`) are low priority.

### M7: Front-end and release `[~]`
- `[~]` Native Bevy UI (fonts from `UI/FONTS` if feasible), menus, car select, settings, save data (our own format).
  Done: an in-game car select (`menu.rs`: catalogue by make / model / year, type-to-search, paint swatches from
  each car's `VehiclePaint`, keyboard + mouse + gamepad). It uses Bevy's ASCII-only default font, so names are
  folded to ASCII ("Coupe"). Done (2026-10-04, paused WIP): the UI data (GeneSys screens / layouts / bindings,
  `mw_formats::ui`, `mwtool ui screens|show|render|font`), the game's bitmap fonts (`UI/FONTS`, `game_text.rs`)
  and a free-roam driving HUD (`hud.rs`: speedometer, nitrous gauge (the tank), odometer, minimap
  from the map tiles; `--mph`, `--no-hud`, F3 = debug text). Done: volume settings (F2). **Easydrive, first version** (2026-10-06,
  session 5; `mw_assets::easydrive`, `easydrive.rs`, `mwtool ui easydrive`, `--show-easydrive`): the menu tree
  (tiers, link and hook items, labels) comes from the free-roam screen's data; link items navigate, Change car
  drives the picked car; the panel's look and position are ours, other entries are inert. **Race HUD** (2026-10-09,
  `hud.rs`, `ui_quads.rs`): the race screen's (604153) position / lap / distance, time and wrong-way /
  checkpoint-missed layouts, the countdown and results sequences' layouts (the online race mode's, model
  decision), triggered / transition-in behaviours animated, additive UI elements blended as the game's
  `MaterialState` says (the nitrous "dark box" fix); Easydrive's RACES tier starts a race (`race::StartRace`).
  **Front end** (2026-10-09, `frontend.rs`, `pause.rs`, `mw_assets::frontend`, `mw_formats::hsm`, `mwtool
  frontend|hsm`): the boot flow read from the game-logic HSM (title, press start, transition, loading screen,
  start point, new / continued game, `--profile`), the pause menu with CONTINUE / MAP (offline races) /
  SETTINGS. 2026-10-10: **the map screen** 899991 from the pause menu's MAP (`map_screen.rs`: the city image,
  the car's and the Most Wanted events, the event card, SET DESTINATION / START RACE, the original's turn and
  scale measured from a capture) and **the event-loading card** (`event_card.rs`, transition 1014321) before the
  race and its intro. Missing: car select / customise, online pages, the intro event, the Most Wanted list panel,
  completion state. Open: those, the car menu in the game font, widget
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
| Ported functions | the Rust port runs *inside* the original in place of the game's function (the probe, [probe.md](probe.md) Tier 2); the same scripted manoeuvre with and without it must give the same telemetry |

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
| Legal | No assets or decompiled code in the repo; no DRM circumvention tooling; game install required. The probe runs only in the user's own licensed copy, offline, after the game has unpacked itself, and never touches the wrapper or licence code (reverse-engineering.md "Code inside the running game") |
| The DRM wrapper rejects in-memory patches | Phase 0 of the probe tests one patch for ten minutes of play; fallback is hardware breakpoints (no code change) |
| Scope | Ship vertical slices (drive in city) before breadth |

---

## Suggested next sessions

**Start with [next-session.md](next-session.md).** State after session 5 (2026-10-06): nine systems were re-read
from the decompiled code in parallel; what is blocked now is mostly blocked on **live reads of the running
original** (`.bss` constants and renderer fields; the list is in next-session.md).

1. **A live-read session** with the original running (`sudo sysctl kernel.yama.ptrace_scope=0`, `tools/orig/orig
   launch`): the handling `.bss` constants, the post-processing fields (bloom flag and set, vignette A / D, the
   seven colour-cube slots and weights), the pad dead zones. Then port bloom, the vignette and the cube blend.
2. **Handling** (M3.2): new reference runs on a clear road: straight-line braking first (`ref/xkr_launch` hits
   something while braking, so it cannot judge the brakes; its clean stretch suggests the rear brakes barely
   act in the original), a slide in a second car, handbrake turn, steady corner, donut; the tyre constraint's
   +0x210 blend scale (two leads in decomp.md "Tyres").
3. **The user's visual reports** (road / terrain texture scale, foliage placement): get a location, then the
   WorldPBR `Horizontal_*` variants, sampler modes, and the game's groundcover draw distance.
4. **Easydrive** (M7): the game's own layouts for its look, the conditional entries, the hooks (customise, paint,
   races) as their systems arrive; **input**: nitrous, horn (landed 2026-10-10, `player_sfx.rs`), look back.
5. **Traffic** (M6.1): runtime landed 2026-10-09; next the manoeuvres, crashed cars on wheels (parked cars landed 2026-10-10),
   per-car lights and sounds, and a capture of the original's traffic to compare density and speeds. **Free-roam** (M6.3): HUD messages, Speed Points, billboard / gate smashes, jack spots.
6. **Animations** (M3.8), compound smash effects (M3.1; compounds break into parts and prop collisions are on by default since
   2026-10-09), crash camera against a capture (M3.3).
