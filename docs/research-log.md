# Research log

Dated findings and open questions about the game's formats and behaviour. Add new entries at the top. When a
question is answered, move the answer into the parser's doc comment and [formats.md](formats.md), and note
the date here.

Install used for verification: Steam build, `VERSION.TXT` = 1.4.0.0, 2,020 bundles.

---

## Open questions

### Textures
- Header flags at 0x20: values 0x00/0x10/0x20/0x30. Is one of them sRGB? (Currently all colour textures are
  treated as sRGB.)
- Byte 0x2C ranges 0–10 and does not mean missing mips (full chains are present). Streaming LOD bias?

### Renderable
- Mesh bytes 0x24–0x27 (sub-part code, then 1, 0, 4 in samples).
- The compressed OBB at mesh+0x00: the exporter documents its quantisation. Decode it for culling.
- Buffer descriptor words `{0, 1, 3|2}`: D3D11 bind flags?
- Is the SNORM position scale always 10.0, or does it ever vary per mesh/renderable?

### Vertex data
- `Color` (RGBA8): roads = baked light (rgb² × GlobalLightMapIntensities.x × 0.318) + AO (a); vehicles = AO (x)
  and specular occlusion (y); trees = AO (y). (Answered 2026-10-04 for these families.)
- World layouts often have Tangent + Binormal but no Normal (cables): is the normal `cross(T, B)`, and with which
  sign?

### Materials / shaders
- Lightmap shaders sample `LightMapTextureSampler` at TEXCOORD1, but no material or instance override binds it.
  Where does the texture come from (a separate resource per track unit, GeneSys, runtime)?
- Bevy's clear coat now renders on vehicles (they have tangents since 2026-10-04).

### World
- InstanceList instance `unknown_04` (f32) and `unknown_08` (u32).
- ZoneHeader (0x206, one per track unit, version 9): import slots at 0x04 InstanceList, 0x0C
  DynamicInstanceList, 0x14 LightInstanceList, 0x1C CompoundInstanceList, 0x24 PropInstanceList, 0x2C one
  GroundcoverCollection, then `u32 count` at 0x34 and `ptr` at 0x38 to more GroundcoverCollection imports
  (2026-10-04, TRK_UNIT1). No visibility data in it. Not parsed yet.
- How the game draws near water (city rivers, the airfield lake): no zone lists a water mesh over the river
  next to downtown (unit 13), and no main geometry covers it. A separate water system? (`--backdrop-zone 13`
  shows the gap.)
- ZoneList zone byte 0x40 (0 or 1) and the header u32 at 0x18 (3).
- What does `WORLDLIST.BNDL` contain, and how does it relate to `HAWAII/MAPS/*.BNDL`?

### Data
- GeneSys field names: the hash is known (2026-10-04). About 1,200 of 5,520 distinct field hashes are named;
  the rest need better word lists. `655d3f5f` (the `CgsCore.UniqueId` at +4 of every object) is unnamed.
- The 64-bit type hash at type+0x10: check it against `field_hash` of the type name (and a 64-bit variant);
  it may also be a layout fingerprint.
- GeneSys: three objects per vehicle bundle still fail to decode (pointers read as huge offsets).
- AttribSys: no `AttribSysSchema` / `AttribSysVault` resources exist in the install (checked 2026-10-04).
- Drift (2026-10-06, from the code): every field's role is settled (decomp.md "Drift"). Still open: the speed
  hold's `.bss` constant 0x01187250, the counter-steer `powf` exponent (taken as `Max_counter_steering_power`),
  and whether the outward side force reproduces the original's drifts (needs a recording).
- Tyre data not yet understood: `Lateral/Long_Slip_Factors`, `Grip_factor` / `Sliding_grip_factor`,
  `Available_grip`, and the lateral curve's fourth field (0.5 everywhere). Five more `Tyres` floats per axle
  (400, 1000, 2) are unnamed.
- VehicleList entry fields beyond ID / names / year (floats look like stats; u16 near 0x6A may be RPM).

### Environment and weather
- What switches to the `Alternate` keyframes (7 hazy twins, one per time-of-day band; no timeline lists
  them). A gameplay state (events, pursuit), a random weather roll, or a menu option? Nothing in GeneSys
  references them except the keyframes.
- `Weather.Thunderstorm_Intensity` (0.23 on night keyframes): how the game shows it (no lightning shader,
  texture, effect or sound was found). `Weather` `0141d842` (0.25 day / 0.15 evening, night and Alternates)
  and `e8eb5d48` (0 everywhere) are unnamed.
- Who reads the wind (`WindBearing` 130° ± `WindBearingVariability` 25°, `WindSpeedMin/Max` 1 / 3.5)? The
  tree shader's sway is per material and only time-driven; cloud offsets are CPU-side (`Clouds.Speed` 5).
- `WetScaleParams` values (puddle tiling, reflection distortion, reflection blur): CPU-set; ours are guesses
  (`materials::WET_SCALE_PARAMS`). The planar reflection plane height and its "Modified" projection.
- When surfaces use `SurfaceType.Wet_Conditions` (wet tyre marks, spray, sounds) instead of `Dry_Conditions`.
- `EnvironmentSettings.DayNightTransition` (`1ab46d30` 8.5, `0e2be130` 20, two (1, 0) pairs) and the
  `Rendering` block (sky material handle, `MoonDiscAlpha`, `SunDiscAlpha`, `31c63004` 180, 3.5 × 2).
- `WorldPaintMap` header u32 at 0x24 (4878 / 1926 / 94).

### Vehicle damage
- How the game turns an impact into zone damage: which `ImpactDamageGraphs` curve (two of them; the second
  has its low point the other way round, `(0, 6 %)`?), what units (mph assumed), how damage is split between
  zones (we use the nearest zone plus a 60° falloff), and what `CrashingRules` / `CollisionInfoDamageProfile`
  / `DamageBarProfile.ImpactLocationDamageScale` / `Physics.LandingDamage` add. Global objects in
  `PHYSICS/PHYSICSCONFIG.BNDL` and `GAMEPLAY.BNDL`.
- `VehicleDamageBehaviour.Bodypart`: the hinged parts (doors, bonnet, bumpers, mirrors) presumably swing
  physically once their `DamageThreshold` looseness exceeds 0. Which bone does each hinge drive? (Hinge
  positions don't coincide with bone positions; bone names are unknown.) What are the three spring-like
  Vector3s (`99e38802`, `19beda43`, `13ac399e`), `9a601fda` (rear bumper `[-3, -5, 0.35]`) and the
  `DamageThreshold` value `31ad3690`?
- `ControlMesh` (GeneSys, one per car, five Vector4s like `[2, 5, 0.2]`, `[0.01, 0.75, 0.75, 0.85]`): a
  separate resource type 0x210 exists in the type list; is it the crumple / damage-volume tuning? The
  car's `0x108e7a` GeneSys object (truncated decode) may be related.
- `WheelDamage` (four per car: steering angle −20..30°, camber-like `b44cbfcf` −10..−20): damaged-wheel
  alignment, not applied yet.
- Damage volume grid (3 × 5 crumple values per car) for the crumple normal map: the shader side is known;
  how the game fills it from the zone levels is not.
### Particles (BearEffect)
- The emission controllers (0x33 / 0x34 / 0x42) hold four curves, four scales and selector bytes
  (`14 03 00 18 | 01 05 05 01 | 02 03 06 04` on the tyre smoke): which input drives which curve and how the
  curves combine is not decoded; the runtime uses curve 0 over `Intensity`.
- What the game writes into `WheelParams` (0x40 bytes), `VehicleParams` (0x30), `CollisionMeshParams` and
  `WheelContact`, and what it means by `Velocity` / `Intensity` for each effect (code).
- Spawner fields: the three angle ranges of the wheel spawner (0x05: `180,180 | 0,180 | 0,360`), its
  `0.9..1.2` range, the 0x07 `0.25 ×3` triple, the u32 flags at the end of every spawner.
- `Bounce` / `Integrate` drag (0–0.999) and the two trailing floats; how bounce planes come from
  `VehicleParams` / `WheelContact`.
- `ViewFade` (0x38): its 12 leading floats (all 1 but one 0.6462) and the `20 51` / `20 40` pair.
- Renderer words: the first float (0.01–4, read as a soft-particle fade distance), the rgb(a) after it,
  the trailing u32 flags; the mesh renderers (0x1E/0x1F/0x22/0x23) and their Model imports; 0x4D
  (`VfxSplatter_Dynamic`).
- `VfxVehicleCollisions` (36 handles: 2 empty + 16 effects, twice) field names, and which collision picks
  which slot. `SurfaceType` weather conditions `f395b019` / `7348be25` (dry / wet by their contents), the
  `VfxWheelEffects` fields `61ac1863` / `e7a7150c`, and the rest of `SurfaceVfx`.
- `gVfxSparkBrightness` (spark shader global) and the particle VS's exposure term
  (`lerp(1, exposure.w, gVfxConstants.x)`): `VfxGlobals.Rendering` has `ExposureScale` 0.4,
  `ExposureScaleCoronas` 0.4, `ExposureGamma` 2.2 (cracked) and unnamed `dacaf024` 0.4, `a77107f8` 10.
- Collision surface ids (`PolygonSoupList` tags) → `Surfaces.Surfaces` index, so dirt / grass get their dust.
- World effects: `mwtool genesys all --type WCVfxBehaviour` finds 3,931 objects (3,911 in `TRK_UNIT*`): 2,302
  have `Spot_effects` (`Spot_effect` handle + `Locator_group` id; e.g. `…24f3d`, `…eee1b`, `…af136`), 1,095 an
  `ImpactEffect` (`…24f3d` 613×, `…1621e7` 410×: prop smashes), most have `Lights` (`LightDefinition`) or
  `Coronas`. How a locator group resolves to positions (compound / world object parts?) is open. No
  exhaust / backfire / nitrous flame effect or type was found (`NitrousParameters` is gameplay tuning).

---

## Findings

### 2026-10-07: the chase camera's "speed shake" split into its sources; the high-speed shake ported

- `ref/live1` decomposed (decomp.md "High-speed shake and the eye's vertical spring"): with the impact
  shake idle, the view's pitch residual against the followed frame is a matrix at cam + 0x360 (correlation
  0.98), the eye's height residual is cam + 0x550 (`FUN_005d12b0`'s vertical spring, 0.995). Body pitch passes
  only weakly (the followed frame's pitch is 0.05° RMS high-passed, the body's 0.15-0.2°).
- `High_Speed_Shake` read in `FUN_005d0b90` → `FUN_005e34e0` → `FUN_005e2a70` / `FUN_009e7a30`: a 199-sample
  quaternion curve (`.bss` 0x01188300, ICE `SHAKE_DATA`, filled at run time) ping-ponged at 30 samples/s ×
  Frequency, scaled by Amplitude × f^Power / 10, plus a pitch bias, turning the view about a point 10 m
  behind the eye. Ported with a stand-in curve of the recorded statistics (model decision); the replay
  matches the recorded std per speed band (0.030° / 0.068-0.071° at 40-80 / 80-140 mph).
- Open: the curve's own samples (where the table is filled: not in the dump; the ICE strings `SHAKE_DATA`,
  `SHAKE_QUAT_*`, `SHAKE_POS_*`, `Shake Type` at 0x0107c4xx..ccxx are the lead); the speed input S (the recorded
  scale is ~30 % below the data law under 60 mph); why the random walk at cam + 0x440 never moves; the
  vertical spring's input and `.bss` scale (needs `disas.py` on `FUN_005d12b0`). Roll (`FUN_005c9380`), the
  deceleration pitch and the yaw-stiffness producer were not reached this round.

### 2026-10-07: the chase camera's impact shake read and ported

- `FUN_005dfe90` read: a noise spring (game LCG, ±`WobbleMagnitude` kicks, step `Maximum_frequency_multiplier`
  × dt) plus a ±`ShakeMagnitude` jitter, times `Maximum_amplitude` × level, as view angles in degrees.
  `FUN_005cd040`'s arguments mapped to the `Impact_shake` fields through the call in `FUN_005d0b90`; the
  decompiler drops one argument there (the body), so the mapping was settled with the disassembly of the call
  site and by matching the recording (decay 0.835 / frame, x autocorrelation 0.5, x std 0.167, jitter uniform
  ±1). Details: decomp.md "Camera effects".
- Ported (camera.rs `ImpactShakeState`); the guessed `CameraGameplayShakeEffect` springs are gone.
- Open: contact + 0xc's unit (taken as m/s normal speed), the landing request's |v| units (`FUN_005c8e40`, not
  ported), `FUN_005d9740`'s Euler order and the rotation pivot; the speed shake (the level at speed is ~0.01,
  so the impact path does not explain the 0.2-0.3° wobble at 40-80 mph; not found); the strings search for
  other camera effects was not done.

### 2026-10-07: free-roam bloom comes from the environment keyframe, not PostFX

- **Source of the renderer's bloom block** (+0xac74, decomp.md "Post-processing" has the chain): the copy's
  source is `FUN_008ce250`'s local filled from the PostFX manager +0x1300…+0x1330 (`FUN_008e3b50`; found by
  counting the pushes, the decompiled argument lists of both calls are off by one). `FUN_008ecf80` blends
  the PostFX keyframes there and then, when the environment override flag +0x15e0 is set, overwrites every
  word but 7 from +0x15f0…+0x1628, which `FUN_008e3e30` fills from the blended environment state
  (`FUN_008c2080` / `FUN_008c0f30`, +0x24d0…+0x2510).
- **Value match**: live colour (0.292, 0.474, 0.604) = `EnvironmentKeyframe.Camera.BloomColour` of 29
  keyframes (the noon set, e.g. `…124575`); B.w = `99436e68` (0.387…0.599 by day, live 0.595 / 0.510);
  words 9–11 = `9dd29928` (0.009) and 1, 1 (`2b7c4634`, `e59ff4ec`). The research log's earlier note that
  the PostFX `Bloom` unpack order puts floats at integer slots is resolved: in free roam those words are not
  the PostFX keyframe's at all. The integer words (4, 5, 5, 1) have no float Camera field: runtime values.
- **Conflict to check**: `e59ff4ec` is also used as the exposure adaptation speed (`CameraExposure.speed`,
  role inferred from the day curve). If it is a bloom weight, the speed field is something else.
- **Dead end**: the minute-long drift of the colour and weights is not a lerp between two keyframes of the
  noon timeline (colour ≈ 25 % towards the orange keyframes, B.w and weights inconsistent with that); more
  inputs are blended (regions, weather). Not followed further.
- **Level combine** traced (`FUN_008e6f40`): C3 = blur(L3), C2 = (w10·blur(L2) + w11·C3) / (w10 + w11),
  C1 = (w9·blur(L1) + (w10 + w11)·C2) / Σ; the add program `0x010117c0` blurs t0 vertically with the 5-tap
  kernel and adds t1 × `kColorAddWeight.w`. Which weight is z or w is inferred from the first call.
- Data parsed and blended (`CameraExposure.bloom_*`); render passes not ported this session (budget).

### 2026-10-06: the live-read `.bss` values applied to the handling code

Each site re-read in the dump with the value in hand:
- **Clutch k** 0x011873e0 = 2π/60: `FUN_009d7ac0`'s Δω is engine rpm (+0x20) minus the driveline's rpm
  (`FUN_009d55f0` multiplies by 60/2π), so k converts it to rad/s. Ours already worked in rad/s: no change.
- **Standstill hold** 0x01186e60 = −2 (mph, the scale 0x0118a060): the hold acts only when rolling backwards
  faster than 2 mph, never at rest. Ours was 0; now −2.
- **Downshift gate** 0x01187580 = 1 s on the ground; 0x01186630 = 1.8 is a speed (body |v| in m/s,
  `FUN_009ae840`): the ground timer resets only with no wheel down *and* the body faster than 1.8 m/s. Ported.
- **Clutch-in throttle**: the disassembly (the decompiler lost the `pow` arguments) shows the exponent is a
  different vector, 0x01187470 = 0.5, and 0x011874d0 = 0.7 is a scale: upshift (`FUN_009d6af0`) throttle =
  0.7 √progress with no pedal term, downshift (`FUN_009d6ca0`) = pedal × 0.7 √progress. Ported (was pedal ×
  progress).
- **Stall** 0x011874a0 = 0.9: rpm < 0.9 `Idle_RPM` drops to standstill (state 2) in the launch (`FUN_009d17d0`)
  and in `FUN_009d6880`, `FUN_009d95a0`, `FUN_009d6af0`, `FUN_009d69e0`, `FUN_009d6ca0`, `FUN_009da1d0` (not the
  downshift declutch `FUN_009ce490`). Ported.
- **Kick-down window** 0x011871e0 = 0.2 s (was 1 s). **Punch** 0x0118a018 = mph → m/s (`Punch.0798d257` is
  mph/s); `nitrous::PUNCH_SCALE`, modes 1 / 3 still not run. 0x01187250 = 60 confirms the time bases.
- **Tyre brake hold** `FUN_009bc0c0`: the vector 0x01186b70 is 9.81 in every lane; only its y lane is used: the
  slip speed is taken from the contact velocity with `param_8 × 9.81` off its vertical component, i.e. the
  velocity after this tick's gravity if `param_8` is dt (the 4th argument of `FUN_009c6f00`, not traced). Not
  ported.
- **Strut stop**: share to the chassis = max(d − min(+0x470, 0.1), 0.1 d) / d (0x01186bf0, 0x01186b10 = 0.1).
  Equal to ours when the constraint's +0x470 is 0. Dead end: the chain `10edb04,4,53220,184,0` lands on the
  vehicle (vtable 0x01023758), and the wheel constraint (+0x210 blend-input scale, +0x470) was not located from
  it in the time available; both stay open.
- Skid sound: 0x0118a060 confirms mph; 0x011870d0 = −0.1 is compared with +0x108 of an unidentified wheel
  object in `FUN_009d1bf0` (a signed value, likely travel / contact distance). 0x01186f60 (0.5) not looked at.
- Telemetry `ref/live1`: the upshift clutch-in lasts 3–4 ticks (clutch 0.25 → 1); the sound load (lagged throttle)
  stays below 0.7 √progress during it and passes 0.7 only after it, consistent with the 0.7 cap but not a proof
  (the load is lagged).
- 911 tests: accel, brake, corner, handbrake, drift, engine unchanged within 0.05 s; nitrous 0–200 km/h
  10.33 → 10.87 s (the upshift clutch-in throttle is now 0.7 √progress); landing / climb fine (climb
  overtravel 0.119 → 0.040 m).

### 2026-10-06 (session 5, late): vignette ported; bloom's live block does not fit the keyframe

From the 20 renderer dumps of `ref/live1/ren` (3 s apart) and the snapshot `snap2`; details in decomp.md
"Post-processing".

- **Vignette, solved except two scalars.** D = (inner × s, k), A = ((outer − inner) × s, 1 − k), the vertex
  shader's constant = (centre, `Scale` × k), all from `PostFxKeyframe.Vignette` of the default keyframe
  `…78dc` (A.rgb / D.rgb = −0.54921 = 0.45078585 − 1 in every dump; centre (0.5, 0.3), `Scale` × 1.8245 =
  (1.2771, 1.0126)). k = 1.8245 is constant over all dumps, including one under another PostFX state; s moves
  slowly between 1.0025 and 1.0385 (2.81 in that state). Neither is explained: `Sharpness` = 0.34 fits no
  simple formula tried (1 / (1 − x), (1 + x)², …), and no renderer instruction stores to +0xa980 / +0xa990 by a
  literal displacement (byte search of the snapshot's code, 0x880000…0x900000), so the writer uses another
  base. The add pass's alpha is v = smoothstep(saturate(|p| + A.w)) (PS `0x010117c0`), the VS (`0x01011e18`)
  takes p = (rotate(2·uv − 1, angle) − 2·centre + 1) × scale with uv the D3D texture coordinate (y down, so
  the centre sits 30 % from the top and the bottom corners darken most). **Ported** in `tonemap.rs` /
  `tonemap.wgsl` with k as a measured constant and s = 1 (model decision); `MW_NO_VIGNETTE=1` turns it off.
  Check at the `--camera=-1131,27,1627,…` view, noon (displayed linear luminance, on / off): centre 1.00,
  top-left 0.78, top-right 0.63, bottom-left 0.51, bottom-right 0.31 (after the tonemap curve and the colour
  cube, so not the raw factor; the raw factor is 1 at the centre, 0.45 where v = 1, i.e. both bottom
  corners, and ≈ 0.66 at the top corners).
- **Bloom, not ported.** The copy into +0xac74 is at 0x008ccbdc (`FUN_008c9e40`, x87 load/store of 13 words,
  sets bit 1 of +0xaa9c). Four of the words are integer bit patterns (4, 5, 5, 1, sliding to 1, 1, 1, 0 in a
  transition), the colour drifts linearly over the minute (0.342 → 0.417 red) and B.w (+0xac84) 0.595 → 0.510;
  the default keyframe has a grey 0.402 colour and floats at those slots. So free roam blends other PostFX
  keyframes over time (the selector, presumably a PostFX timeline, is not known), and the unpack order of
  `FUN_008e51b0` noted below puts float fields where the live block holds integers: the field mapping is not
  confirmed. No PostFxKeyframe has a Colour near the live one. Next: disassemble the source of that copy (its
  `edi`) and the PostFX state's keyframe selection from the snapshot.
- The colour-cube slot weights were not reached.

### 2026-10-06 (session 6): fog, groundcover size, top mip and the sun from the live dumps

Read across all 20 `ref/live1/ren` dumps (slot records per dump, car positions from `ren.log`).
- **Hour of the run**: slot 0x0d (key light direction) goes from (0.411, −0.765, −0.495) to (0.441,
  −0.760, −0.477). With `BearingCentre` −232.37 / `BearingVariation` 35.06 / `BearingPhaseOffset` −0.5 the
  bearing atan2(z, x) of the to-sun vector (129.7° → 132.8°) gives **~12:13 → ~12:34**, rising as it should.
  Our old convention (bearing from +Z towards +X) puts that vector at −39.6°, outside the reachable
  92.6°–162.7°; the two other sign variants either are unreachable or run backwards in time. Fixed in
  `SunMoonPath::key_light_direction` (x = cos b, z = sin b): looking from (887, 27, −555) towards +X/−Z
  at 12:00 the sun moved from in front of the camera (glare on the road) to behind it. The elevation (49.9°
  → 49.5°, falling) means the arc peaks near 12:00; our sine arc between 05:00 and 21:00 peaks at 13:00
  (open, not changed: no code source for rise / set yet).
- **Fog `ScattCoeffs`**: computed in `FUN_008c0f30` with a constant scale of 1 (decomp.md "Fog"), so near /
  far are **not** scaled. Live (0.0007576, 0.06061, 2) at (865, −578) = 80–1400 m, Power 2 = exactly what
  our timeline `1245c8` gives there from 10:00 to 14:00. Driving north (dumps 13–16) the slot slid to
  80–1171 m, Power 1.52 over ~10 s: the original blends between painted timelines in space / time, we switch.
  So the "flat fog beyond 500 m" view is a timeline whose keyframe really reads 500 m; what keeps the
  original's hills visible must be the gradient alpha or the layer term, not near / far. Dead end: renderer
  +0x1ef80 and +0x2110 in snap2 hold no fog values (the `this` of `FUN_008c0f30` is another object).
- **`LayerFogCoeffs`**: formula traced (decomp.md); w = z / (1 + 2^y) holds in every dump. Not ported: the
  entry weights and the density factor (0.53 and rising at the run's place) are untraced, and the shader
  term needs them.
- **Groundcover**: the vertex program reads only `cb1[0].x`, size = BINORMAL.xy (byte / 127) × it, the quad
  turned about Y to face the camera. Slot 0x7f = 4 in 15 dumps, 0 in 4, 256 in 2 (other passes reuse the
  slot). Size now byte / 127 × 4 (was byte / 32, 0.8 % larger).
- **Top mip**: `FUN_00409510` passes the "start at mip 1" flag only when `DAT_010eac98` is set and
  `FUN_00414a50` (+0x113) returns **0**; the earlier note had it inverted. `FUN_0086cf50` / `FUN_0086cf80`
  map +0x113 to a settings option (1 = option 1, reload on change). The live value 1 therefore means full
  resolution: the original does not drop the top mip at the user's settings, ours already loads mip 0.
  The "scaled wrong" road report needs another explanation.

### 2026-10-06 (session 5, late): live-read session against the running original

The user ran the original (free roam, about five minutes: standing, then driving up to 134 mph, then standing);
everything below was read from its memory, read-only. Reads through `/proc/<pid>/mem` worked with
`kernel.yama.ptrace_scope` = 1 on this machine (no sysctl needed this time).

**What was saved** (git-ignored; raw memory never leaves `re/out` / `ref`):

- `re/out/live/snap1`, `snap2`: full snapshots of the process's readable memory below 4 GiB (1.4 GB each;
  `mem.bin` + `index.tsv`), standing and at speed. They hold the unpacked code as well, so call sites the
  decompiler lost (`this` in ECX) can be disassembled, and every `.bss` value and object can still be followed
  after the game has quit. Helpers in the git-ignored `.claude/tools/`: `live.py [-s snap] <va|chain> [n]`
  (chain = `10edb04,4,53220`: deref then add), `disas.py <snap> <va> <len>`, `slots.py [-s snap]`,
  `snapshot.py`, the recorders `camrec.py` / `renrec.py`, the loader `camload.py`.
- `ref/live1/telemetry.csv`: the handling watch list at 120 Hz (35,610 rows).
- `ref/live1/cam.bin` (+ `cam.json`): 30,983 records at ~120 Hz of the whole camera director (0x3050 bytes), the
  player vehicle object +0x1600…+0x2d00, its rigid body block (0x100) and the per-player camera state (0x200).
- `ref/live1/ren/ren_NNNN.bin`: 20 dumps, 3 s apart, of the renderer object (0x20000 bytes from renderer − 0x80)
  and every shader constant slot; `ren.log` has the car position and speed of each.

**Objects found** (addresses are this run's; the route is what matters):

- The **frame object** `G` (`this` of `FUN_0046f730`, called from `FUN_00476f20`): found by scanning for
  `[G+0x2db10c] == 7` and `[G+0x623afc] ∈ {1, 2}`; no global pointer to it is known yet. The camera module
  (`DAT_010edaf4`) lies inside it.
- The **renderer** (`this` of `FUN_008c9e40` and of `FUN_0089d5c0`, i.e. their `param_1`) = `G + 0x2bc400 + 0x80`
  (disassembly: `FUN_0046f730` passes `G + 0x2bc400` to `FUN_008ce250`, which passes `this + 0x80` on). Its
  render mode at +0x1ed0c − 0x80 read 7.
- The **shader constant context** (`this` of `FUN_00a5d660`, in ESI in `FUN_0089d5c0`) = `*(renderer + 0x2e50)`:
  `ctx[slot]` points at the slot's float4s (count per slot in the table at `0x0118cec8`). Slots are rewritten
  by each pass, so a read shows what the last writer left (a mid-frame snapshot shows another pass's values).
- The **chase camera** is where decomp.md said: director = `*(*0x010edaf4 + 4) + 0x6a0`, chase camera at
  +0x8d0 (vtable `0xf999b4` confirmed). Camera matrix at camera +0x10 (rows right / up / back, position at +0x40),
  the followed car's frame at +0x4d0 (position +0x500), **field of view in degrees at +0x90** (70 at rest,
  102.7 at 134 mph in this run).

**Static values** (`.bss`, initialised at start-up; vectors repeat the value four times):

| address | value | what the code uses it for (decomp.md / handling.md) | we assumed |
|---|---|---|---|
| 0x011873e0 | 0.10472 (= 2π/60) | factor in the clutch torque of `FUN_009d7ac0`: an rpm → rad/s conversion, not a tuning scale | 1 |
| 0x01186e60 | −2.0 | standstill hold speed limit | 0 |
| 0x01187580 | 1.0 | seconds on the ground before a downshift is allowed | 0 |
| 0x01187250 | 60 | per-tick → per-second factor | 60 |
| 0x011874d0 | 0.7 | clutch-in throttle exponent | - |
| 0x011874a0 | 0.9 | stall fraction | - |
| 0x01186b74 | 9.81 | the y term in `FUN_009bc0c0` (gravity) | not modelled |
| 0x0118a060, 0x0118a090 | 2.23694 | m/s → mph | - |
| 0x011870d0 | −0.1 | load floor of the tyre spin sound ramp | - |
| 0x01186630 | 1.8 | unidentified (drivetrain entry) | - |
| 0x01186f60 | 0.5 | unidentified (drivetrain entry) | - |
| 0x01186bf0, 0x01186b10 | 0.1, 0.1 | strut stop slack | 0 |
| 0x0118a018 | 0.44704 | mph → m/s (the nitrous punch "scale" is a unit conversion) | - |
| 0x011871e0 | 0.2 | kick-down time window, seconds | 1 |
| 0x0110f590 | 3600 | traffic cull radius squared (60 m) | - |
| 0x0110e8f0 | 45 | camera: `_DAT_0110e8f0` of the yaw spring (`FUN_005c90a0`) | - |
| 0x0110e4e0 | 0.9848 (cos 10°) | camera: `_DAT_0110e4e0` of `FUN_005c7a80` | - |

**Renderer values while driving in daylight** (renderer offsets as in decomp.md "Post-processing"):

- **Bloom is on in free roam**: the flag bytes at +0x9834…+0x9837 are all 1 and bit 1 of +0xaa9c is set. The
  set at +0xac74: colour (0.292, 0.474, 0.604), then 4 (integer), 0.5, 5, 5 (integers), 1.0, 1 (integer),
  0.009, 1.0, 1.0, 0.
- **Vignette**: D (+0xa980) = (1.0065, 1.0065, 1.0065, 1.8245), A (+0xa990) = (−0.5528, −0.5528, −0.5528,
  −0.8245). The composite multiplies by A·v + D, so the factor is 1.0065 where the vignette weight is 0 and
  0.4537 where it is 1 (the keyframe's inner 1 and outer 0.45, stretched by about 1 %); the alpha pair gives
  1.8245 → 1.0. The neighbours +0xa9a0…+0xa9bc read (0, nan, nan, nan) and (0.5, 0.3, 1.277, 1.013).
- **Constant slots** at one moment (car at (887, 24.5, −555)): 0x23 `ScattCoeffs` = (0.000895, 0.0716, 1.502,
  0.4); 0x24 = (0.2808, 0.4115, 0.4959, 0.25); 0x27 = (0.96915, 0.00412, 0, 0); 0x83 `LayerFogCoeffs` =
  (0.1512, −6.888, 0.0826, 0.0819); 0x7f `GroundcoverConstants` = (4, 4, 4, 4); 0x0d key light direction =
  (0.397, −0.766, −0.505). The other 15 dumps hold the rest of the run.

**Other one-off reads**:

- The object at `0x010eac98` (pointer `0x014de500`), byte +0x113 (what `FUN_00414a50` returns) = **1**.
- The GeneSys object at `DAT_0110f940`: type word `0x00069444`/`0x00069445` near its header, data
  (`*(obj + 0x28)`) floats from +0x10: 4, 50, 10, 0.25, 0.4, 0.7, 2, 80, 6, 25, 0.6, 0.649, 0.3, 8, **0.24
  (+0x48)**, 4, −20, 15, 20, 14, 25, −8, 20, 2, 0.05, 5, 0.05, 0.5, 0.15, 0.5, 2, 2, 0, 8, 4, 10.
  `DAT_0110f944` = 0.
- The player index was 48 again (vehicle table slot offset 0x184).

**Traffic, from the snapshots** (after the game had quit):

- `DAT_0110f940` **is `TrafficParamConfig`** (`0100000000069444` in `GLOBALCONFIG.BNDL`): all 36 floats read
  live equal the data file's fields in order (field +0x48 = 0.24). The inference in decomp.md is confirmed.
- The traffic system object (`world` in decomp.md's traffic section) was found by searching for the copy of
  `TrafficSystemConfig`'s 4.78 outside the resource: at `world + 0xa2b74` it reads **4.78** in both snapshots
  (the base, unmodified), followed by 1.0, an empty marker, then **1.0 standing / 1.1106 at speed** at +0xa2b80
  and 0 at +0xa2b84. The word at +0xa2b40 is 0xd and the one at +0xa2b44 is 1, so the mode decomp.md puts at
  +0xa2b40 is probably at +0xa2b44. +0xa2bfc does not hold a float (bytes 01 01 ff 7f); the float after it
  (+0xa2c00) is 1.0, so the runtime speed scale is very likely that one and was 1.0 in free roam. The point at
  +0x36114 is not the player's position: (1814, 23.7, 1650) while the car stood at (1400, 34, 1178), and
  (925, 22.4, −101) while it drove at (887, 24.5, −555), so it runs several hundred metres ahead of or away from
  the car (a streaming or look-ahead point; not identified).

Not read (no route to the object yet): pad dead zones, the free-roam state bytes, the colour-cube frame state.
All of them can still be followed in the snapshots.

### 2026-10-06 (session 5, round 6): the shadow pass's cost

**Tooling** (`shadow_cull.rs`, a PostUpdate system after Bevy's `check_dir_light_mesh_visibility` that filters
each cascade's `CascadesVisibleEntities` before extraction). Env switches: `MW_SHADOW_CASCADE_OFF=0,2`,
`MW_SHADOW_CLASS_OFF=alpha,world,backdrop,prop,car,other` (mesh entities carry a `ShadowClass`: world placements
split into opaque `world` / alpha-tested `alpha` / `backdrop` = model keys with bit 62 or 63, `prop` from
`props::spawn_prop`, `car` from `drive.rs`, `other` = no class), `MW_SHADOW_MIN_TEXELS=k`,
`MW_SHADOW_MAP_SIZE`, `MW_SHADOW_DISTANCE`, `MW_SHADOW_MODEL_RANGE=1`, `MW_SHADOW_FOOTPRINT=0`. The bench line now
prints each cascade's texel size.

**Measurement caveat**: the machine was not quiet (another agent's rustc and a browser; load average 4.4-8,
given per run below). Frame means also clamp at 16.6-16.8 ms in many runs (60 Hz) although the line says
"X11": the same city view read 12.4 ms in one run and 16.67 ms in the next with identical settings, so
`AutoNoVsync` under XWayland is paced at 60 Hz part of the time. Below ~16.7 ms the frame mean is not a cost
signal; the render-world CPU (prepare + graph) is used alongside. Not fixed (open: force `Immediate` or run
the bench in gamescope / a bare X server).

**Attribution**, drive view (`--drive 911 --units 30 --spawn=815,60,1760,270 --auto-throttle 0.3 --bench 8`),
two runs each (run 1 at load 5-8, run 2 at load 5-7); baseline casters 147 / 316 / 1586 per cascade, texel
0.028 / 0.084 / 0.251 m:

| switch | casters | frame mean ms | render world ms (prep + graph) | saves |
|---|---|---|---|---|
| baseline | 147/316/1586 | 26.3 (disturbed) / 18.4 | 25.4 / 17.7 | |
| `--no-shadows` | - | 16.6 / 16.7 (capped) | 12.9 / 12.4 | ≥ 1.7 frame, 5 ms render world |
| cascade 0 off | 0/316/1587 | 19.0 / 18.7 | 18.3 / 18.0 | nothing |
| cascade 1 off | 147/0/1587 | 18.2 / 19.6 | 17.5 / 18.8 | nothing |
| cascade 2 (outer) off | 147/316/0 | 16.6 / 16.8 (capped) | 14.3 / 14.5 | ≥ 1.6 frame, ~3.3 ms render world |
| alpha-tested off | 128/259/1351 | 19.6 / 19.0 | 18.9 / 18.3 | nothing |
| opaque world placements off | 83/123/307 | 16.7 / 16.9 (capped) | 14.2 / 14.0 | ≥ 1.6 frame, ~3.6 ms render world |
| backdrop off | 143/310/1574 | 19.6 / 19.7 | 18.9 / 19.0 | nothing |
| props + car off | 102/271/1541 | 17.8 / 19.1 | 17.2 / 18.4 | within noise |
| other off | 132/301/1571 | 18.3 / 19.1 | 17.6 / 18.5 | nothing |
| map 1024 / 4096 | same | 19.2, 21.0 / 20.6, 19.0 | 18.5-20.3 | nothing (not GPU raster) |
| min 1 texel | 147/312/1571 | 18.5 / 18.6 | 17.8 / 18.0 | nothing (16 casters) |

The cost is **CPU per caster per cascade view** in Bevy's render world (specialise / queue / batch), not
rasterisation: map size does not matter, and the outer cascade's ~1 250 opaque world placements are most of it
(about 1-2 µs each). A sub-texel size cull cannot help: the outer texel is 0.25 m and only 16 casters (85 at
4 texels) are smaller.

**Default-on change: receiver-footprint cull** (`shadow_cull.rs` module docs). Our lighting pass samples a
cascade only for visible points whose light-space xy is inside it, and fades the shadow to nothing at the last
far bound (180 m). Bevy keeps every caster inside each cascade's square box, which bounds a sphere around the
frustum slice. Criterion: project the caster's world AABB corners and the view frustum's corners (from
`minimum_distance` to the last far bound, the whole shadow range, not just the cascade's slice) with the cascade's
`clip_from_world`; drop the caster from that cascade if the two xy rectangles do not overlap after widening the
frustum's by 4 texels (2×2 PCF tap and the receiver's normal offset). Casters toward the sun are not culled by
depth, so off-screen casters whose shadow reaches the view stay.

| view | casters before -> after | frame mean ms before / after (two rounds) | render world ms |
|---|---|---|---|
| drive | 147/316/1586 -> 138/277/853 | 18.98, 19.35 -> 18.20, 18.58 (load 6-7) | 18.3, 18.7 -> 17.5, 17.9 |
| city overview | 28/29/108 -> 28/28/56 | 13.10, 13.18 -> 12.55 (second run disturbed: load spike, 24.6 ms) | 12.4, 12.4 -> 11.8 |

About **0.8 ms per frame** in the drive view. Image check (`--hour 12`, standing car, PIL): drive view before vs
after MAD 0.45/255, 0.9 % of pixels > 8/255, and the amplified difference shows only the idling tachometer and
one-pixel edge shimmer of the car and tiles, no shadow change; city overview MAD 0.04/255, 0.25 % > 8/255.

**Left behind flags (picture changes)**:
- `MW_SHADOW_MODEL_RANGE=1`: the model's own **shadow distance** (`Model` +0x0C, stored squared, now parsed as
  `Model::shadow_distance`; survey of 12 679 models: 100, 200, 400, 800, 1000 m and 81 other values). A caster
  is dropped while the camera is farther than that from its AABB centre (the comparison point is a model
  decision; the field's use is not yet traced in the decomp). Drive: outer cascade 1586 -> 1090 alone,
  frame 18.98 / 18.88 -> 17.83 / 17.42 ms; with the footprint cull 853 -> 648, 18.20 / 18.58 -> 17.74 /
  17.78 ms. Picture: 0.6 % of pixels > 8/255, upper half (far buildings' shadows). Fidelity-sourced data,
  so it should become the default once the decomp confirms the meaning and the distance origin.
- `MW_SHADOW_DISTANCE=120` (instead of 180): 147/271/774 casters, 17.1 ms (one clean run; the other disturbed).
- `MW_SHADOW_MAP_SIZE`: no gain either way.

**LODs for shadows** (step 3, not done): the loader only ever loads LOD0 (`AssetStore::model_meshes(id, 0)`).
The measured cost is per caster (draw), not per vertex, so a coarser LOD in the shadow views would not cut it;
the `SHADOW` state (`Model::STATE_SHADOW`, index 6) could map a model to a dedicated shadow renderable, which
would need a second mesh entity per placement (Bevy cannot swap meshes per view) and so add entities. Distance
LOD selection for the main view (from `lod_distances`) is the fidelity task; it would need `VisibilityRange`
per LOD entity, which Bevy's directional-light culling also honours per view.

### 2026-10-06 (session 5, round 6): the chase camera's position springs and avoidance

Goal: read `FUN_005d12b0` and the other "weight" helpers, resolve the yaw spring's value, locate avoidance.
Result: **`FUN_005d12b0` partly read; its speed band, heights and longitudinal (z) offset ported**;
avoidance not in the decompiled code (decomp.md "Speed and acceleration movement", "Avoidance: where the
search ended").

- `FUN_005d12b0`: `h = clamp((v − b6010ee1) / (53e4b4f9 − b6010ee1), 0, 1)` (mph); eye / look-at heights go
  to `7d4ec535` / `b028673b` linearly in `h`, unsmoothed (the round-4 guesses were right). The eye's z offset
  is a **first-order lag** (rate `fdd25ee3` = 2/s accelerating, `285f2260` = 2.5/s decelerating) towards
  `0a78c15f · h + (bd3a7cfc | 7c9039e2) · |a_long| / a_max`, `a_long` the acceleration projected on the
  velocity. So the braking offset is `7c9039e2` = 0.5 m, not `8e1310f6` = 1.5 m (that one feeds a vertical
  stage at cam + 0x554 whose locals the decompiler lost). `.bss` limits not read: `DAT_0110e770` (a_max and
  clamp), `DAT_0110e4d0`, `DAT_0110e680`, `DAT_0110e610`, `DAT_0110e640`; `a_max` = 20 mph/s is a model value.
- Yaw spring: nothing more resolvable from the pseudo-C; stays a model decision.
- Avoidance: the director's update `FUN_005d5e20` is not a function in the dump; every remaining helper of
  the chase update and the manager `FUN_005e23c0` calls only camera-range maths, leaves and the CRT (callee
  lists in decomp.md). No world / car / prop query exists in the decompiled camera code. Next: disassemble
  0x005d5e20 from the unpacked exe (probe), or watch cam + 0x580..0x58c / 0x5a0 live against a wall.
- Not read this round (budget): `FUN_005d0b90`, `FUN_005c7a80`, `FUN_005c9380`.
- **Traces, 911, before → after** (boom = horizontal eye distance in car space, m; accel_z = z offset):
  accel: boom peak 6.83 → 7.19, settle 5.60 → 5.60, z offset peak −1.33 → −2.64 (it now includes the speed
  term: −1.00 at top speed); brake (100 → 0 km/h): eye forward 1.50 → 0.32 m, boom at stop 3.10 → 4.28;
  drift: boom 6.79 / 5.12 → 7.10 / 5.38; corner: 6.77 / 5.51 → 7.10 / 5.52; crash: boom min 4.08 → 4.60,
  settle 4.60 both; landing: boom 6.83 / 6.08 → 7.10 / 6.26, eye height 1.32…2.00 → 1.32…2.03. Yaw lag,
  lateral, FOV and pitch unchanged (rules not touched).

### 2026-10-06 (session 5, round 5): the chase camera ported from FUN_005d4970

- **Structure of the update** (decomp.md "The followed frame"): the "sway" is a per-axis smoothing of the
  car's orientation as Euler angles (pitch, yaw, roll), `angle += (1 − retain) · wrap(target − angle)`,
  `retain = 1 − clamp(K · dt · 60, 0, 1)` (`FUN_005e14d0` → `FUN_005c89c0` → `FUN_005c75b0`), stiffnesses at
  cam + 0x6d0 / 0x6d4 / 0x6dc; the pitch's retain × `min(|speed| · 0.02, 1)`; the pitch stiffness eases 0.1 per
  update to a per-player default (`FUN_005c8e40`, with an airborne branch and a landing shake). No lag clamp.
  Ported (`camera.rs`); our 0.6 rad yaw-lag cap and the "up vector at 2/s" slope following are gone.
- **Yaw spring field map settled** (stored order via `mwtool genesys`): y10 `Decay_rate` 0.01, y14 `78527964`
  10, y18 `High_speed` 100, y1c `63fb2207` 0.05, y20 `397fce82` 10, y24 `Low_speed` 0, y28 `5a74d44d` 0.075.
  The yaw-rate term divides by y14 − y20 = 0, so it is inert in the shipped data. The stiffness formula
  `lerp(lerp(y28, y18, f), mods(0x29), s)` stays unresolved (`f` unshown by the decompiler, `mods(0x29)` and
  `_DAT_0110e8f0` need a live read); our speed blend 0.075 → 0.05 is kept and marked as ours.
- **Weight**: `FUN_005c8880` is a vertical bob, a 4-sample mean of `0.5 (v2 + v3)` read at `obj + 0x1f0 + n·4`
  (likely the rear wheels) × state + 0x94, Y only; not ported. `FUN_005d12b0` (3500 bytes, writes
  cam + 0x550..0x58c) is the likely speed / acceleration movement; not read.
- **Avoidance**: no world or collision query anywhere in the call tree of `FUN_005c2130`, `FUN_005d12b0`,
  `FUN_005dfe90` (all leaves are maths). Remaining candidates listed in decomp.md.
- **Traces, 911, before (round-4 baseline) → after** (peak / settle; lateral and boom in car space):
  drift yaw lag −15.5 / −14.7° → −14.6 / −14.0°, lateral 1.32 / 1.27 → 1.29 / 1.21 m, boom 6.80 / 5.0 m
  unchanged, height 1.74 / 1.69 → 1.74 / 1.68 m, FOV 73.1 / 71.6 → 73.1 / 71.2°; corner yaw lag −15.7 / −15.4° →
  −13.8 / −13.6°, lateral 1.49 / 1.47 → 1.32 / 1.29 m, boom 6.79 / 5.5 → 6.77 / 5.51 m; accel unchanged (yaw 0,
  boom 6.80 / 5.60 m, FOV 70 → 95°); crash: yaw lag 1.1° peak, boom min 4.10 m (4.05), settle 5.47 m. The
  smaller lag is the linear-in-dt gain (stiffer than `1 − e^(−r·dt)` at the tests' 4× time step); at 60 Hz
  real time the two are identical. The visible change is the pitch: rigid with the car at standstill and
  following faster below 50 m/s; the car's body roll, followed before through the eased up vector, is now
  its own smoothed angle (`K_ROLL`, a model value).
- Still needed for the rest of the port, from a live read (the original was not run this round): state + 0x70,
  + 0x74, + 0x94, cam + 0x6dc, `_DAT_0110e8f0`, `_DAT_01189f60`, and the stack `f` of `FUN_005c90a0`.

### 2026-10-06 (session 5, round 5): static mesh merging

**What was built** (`loader.rs` `Worker::send_merged`, `convert::merge`; opt-in `--merge-static`): per track
unit, static placements (no physics, not a backdrop, not groundcover, not the car) whose model is placed
**once** in the unit and whose parts are all opaque or alpha-tested, unpainted, with a plain attribute set,
are moved to world space and merged per (`Look` after instance tint / AO, attribute set) into meshes of at
most 2^18 vertices drawn at the origin. The vertex transform reproduces Bevy's mesh vertex stage: positions
by the matrix, normals by the inverse transpose (normalised), tangents by the matrix (normalised) with w
times the determinant's sign. The winding is not reversed for mirroring transforms: Bevy 0.19 does not flip
the front face for a negative determinant either (only `meshlet` does), all world materials have
`cull_mode: None`, and `double_sided` ones flip back-face normals, so keeping the winding keeps the picture.

**Merging repeated models runs out of GPU memory**: the first version merged every static placement; full-map
loads then failed with wgpu "Out of Memory" (8 GB RTX 3070, others' runs sharing it), because instanced
models (trees, lamps, kerbs) were copied into every merged mesh. Repeated models already share one bin (one
per mesh x material), so merging them saves nothing; only single-use models are merged now.

**Result: no measurable gain, so it is off by default.** Back-to-back A/B on one binary (`--no-merge` then
merged, two rounds, load average 6-11 from other agents' builds and runs), frame mean ms:

| view | unmerged | merged | mesh entities (visible) unmerged -> merged | shadow casters per cascade |
|---|---|---|---|---|
| cliff | 23.6 / 14.2 | 17.9 / 16.9 | 107 869 (3 258) -> 98 702 (2 945) | 79/162/655 -> 84/160/642 |
| city | 16.7 / 18.2 | 21.8 / 16.6 | 107 869 (3 652) -> 98 702 (3 233) | 28/29/108 -> 50/53/128 |
| drive | 22.4 / 20.1 | 20.4 / 20.1 | 20 756 (3 846) -> 18 791 (3 484) | 147/316/1586 -> 161/304/1446 |

Render-world CPU (prepare + graph) moved with the frame time, within the noise (cliff 22.4 / 13.4 vs
16.3 / 14.2 ms; drive 21.6 / 19.4 vs 19.8 / 19.4 ms). RSS 1.2-2.4 GB in both, dominated by noise; load wall
time unchanged within noise (merging is cheap next to decoding). Earlier baseline runs on master at load 8-15:
cliff 20.7 / 17.9 ms, city 18.7 ms, drive 48.5 / 39.7 ms (other agents' games and rustc running).

Why it does not help: merging cuts only ~10 % of the visible mesh entities. Bins are per mesh x material and
world materials are almost all specific to one model (their textures), so single-use models rarely share a
`Look` with another model in the same unit; repeated models are already one bin. To cut bins substantially
the materials would have to be shared (texture arrays / atlases, bindless), which is a renderer change.

Images (`--screenshot`, PIL, mean abs diff per channel, pixels off by > 8/255): cliff 0.81/0.47/0.26 and
2.6 %, city 0.37/0.24/0.12 and 0.6 %, night (20 units, hour 1.5) 0.10 and 0.1 %. Two unmerged runs of the
cliff differ by more (1.22/0.79/0.44, 2.9 %): the scrolling sea and the swaying tree canopies; no
difference outside them.

Next by the numbers: shadows (2-3 ms cliff, ~7 ms drive in round 4): fewer casters per cascade (far or tiny
objects out of the outer cascade; the drive's third cascade has ~1 450 casters) and fewer shadow views, after
measuring the game's own shadow distance and split.

### 2026-10-06 (session 5, round 5): the chase camera, second attempt

Goal: find the chase camera's update from round 4's leads and port its sway, weight and avoidance.
Result: **the code is found; nothing ported yet** (the budget went into the search; decomp.md "## Cameras"
has the map).

- Lead 1 (the 0x3050 objects) worked, indirectly: the object at camera manager + 0x6a0 is a 0x3050-byte
  camera director built by `FUN_00422640`, which stores 15 camera sub-objects' vtables. Their update methods
  (vtable +8, read from `.rdata`) were checked for the car's speed and the camera data: the sub-camera at
  director + 0x8d0 (vtable 0xf999b4) updates through **`FUN_005d4970`**, which reads the speed and calls about
  20 helpers, of which `FUN_005c90a0` (yaw spring), `FUN_005c9380` (roll), `FUN_005c7a80` (gradient
  adjustments) and `FUN_005d0b90` (speed-based movement, impact shake, final write) read
  `CameraGameplayExternal`.
- Why round 4 missed it: the camera reads its data through a per-player state at camera + 0x290
  (resource at state + 0x98, instance offset at state + 0x9c), so the read is
  `*(*(state + 0x98) + 0x28) + idx + OFF`, usually with a sub-struct base first (`+ 0xbc`, then `+0x10..`).
- First rule read (`FUN_005c90a0`): the yaw spring is chosen by **the car's yaw rate in car space**, not only
  by speed: `t = clamp(1 − (|ω_y| − y20) / (y14 − y20), 0, 1)`, rate-limited by `y10` per call into
  cam + 0x6d8, and blended with a speed factor `clamp(|v| / v_ref, 0, 1)` (`v_ref` is `.bss` `_DAT_0110e8f0`)
  and the car's mods-profile value 0x29. The y-names are sub-offsets of `Yaw_spring_modification`; which hash
  is `Low_speed` / `High_speed` / `Decay_rate` is open, and so is the last blend (one unread local).
- Lead 3: `CameraGameplayExternalGlobals` is only impact shake + lens + a shake effect. Lead 4: no
  `ProcessNode.Camera*` instances exist in any bundle; the chase camera is plain code, not a node graph.
- Avoidance is not located: `FUN_005dfe90` (shared with the scripted cameras, the impact shake and the final
  writer) looks like transform composition; the unread helpers listed in decomp.md are the candidates.
- No camera rule changed, so the round-4 baseline traces still describe our camera.

### 2026-10-06 (session 5, round 5): groundcover look

- **Why the tufts were black**: the groundcover quads used the Standard look with Bevy's double-sided
  material, which flips the world normal on back faces. Our sprite normal is the instance's ground normal
  (pointing up), so every back-facing quad got a downward normal and the deferred pass left it unlit. Test
  (park lawn, `--camera=-760,45,1960,-900,30,2080 --hour 12`): with the new groundcover mode but Bevy's
  flipped normal kept, the tufts stay solid black exactly as before; with the vertex normal used unflipped
  they read as olive grass clumps in the lawn's light. The game never meets the case: its sprites always
  face the camera.
- **The programs** (`mwtool shader-asm Groundcover`; facts in formats.md "Sprite vertices"): one texture
  (`DiffuseSampler`, an atlas), cell from the COLOR alpha byte (= the instance's region byte) via
  `GroundcoverTextureRegions`, `uv = region.xy + region.zw × (u, 1 − v)`; alpha test at 0.5; RT1 = texel.rgb /
  texel.a × sqrt(2 tint), so albedo = (rgb / a)² × 2 tint, like the trees' premultiplied texels; RT0 = the
  normalised vertex normal; RT2 / RT3 untouched (no specular); no lighting in the shader. The vertex normal
  is mostly the ground normal: `normalize(sprite vector) + max(8 − height, 0.75)·(n + (0, 0.25, 0))`.
- **The CPU vertices** (`FUN_00eb9500`): position, NORMAL = bytes +8..+10 / 127, BINORMAL = (+6, +7, +11) /
  127 (width and height shrunk by the range fade), COLOR = bytes +12..+15 as RGBA (tint, region), UV halfs
  (0,0), (1,0), (1,1), (0,1). Constant `DAT_01081004` = 1/127 (.rdata).
- **Sprite size, still open, narrowed**: `gObjectCB` is not a separate buffer: the draw record points at the
  collection's own material record (`collection +0x30 + material × 0x110`, 0x110 = the cbuffer's 272
  bytes), so `GroundcoverConstants` = the record's first 16 bytes. On disk those hold the material import
  (all zero in `TRK_UNIT1`'s collection 03c936db), so whatever resolves the import must also write the size
  scale there; the 0x20F load / fix-up function was not found this round (next: the resource-type handler
  for 0x20F, or a live read of the record in the running game). Sizes stay at byte / 32 (model decision).
- **Done**: `Mode::Groundcover` (materials.rs / layered.wgsl): albedo = (texel / a)² × 2 tint clamped to 1,
  unflipped vertex normal, no specular. The quads' vertex normals follow the game's formula without the
  camera term (−0.1·d), since the crossed quads are static; the tint is no longer clamped per factor.
  Camera-facing sprites need a vertex-shader path in the layered material (not cheap): still crossed quads.

### 2026-10-06 (session 5, round 4): the chase camera from the code

Goal: port the chase camera's sway, weight and avoidance from the code that reads `CameraGameplayExternal`.
Result: **not found; nothing ported** (decomp.md "## Cameras" lists every search and candidate).

- Searched every function for reads of the External camera's field offsets (eye 0x148, look-at 0x130, pivot
  0x13c, acceleration 0x64..0x8c, roll 0xa0..0xb8, yaw spring 0xcc..0xe4, pitch 0xf8..0x108, top level
  0x160..0x170) in the GeneSys read form `*(res + 0x28) + OFF + index` (both orders), as `*(float *)(base + OFF)`
  and as `base[OFF / 4]`, and for the container's four camera handles. Only false positives (render-state
  setup at `FUN_008be4e0` / `FUN_008be1c0` / `FUN_008bdc50`, matrix maths, an unrelated global GeneSys resource).
  Conclusion: the camera does not read its data in any of the forms the handling code uses; it probably copies
  it once into its own object, or reaches it through a node graph.
- Leads: `FUN_005d4270` registers `CameraEntityModuleState` with getters for `GameplayExternalCameraTransform`
  and `GameplayExternalFOV` (`LAB_005dae70`, `LAB_005daeb0`): labels, not decompiled; their bodies (the probe or a
  disassembly of the unpacked code) give the field that holds the Gameplay External camera. `FUN_005e23c0` keeps
  eight-deep histories of a car's speed (+0x1620), the time step, the body's velocity and a transform (a
  target filter, by its shape), next to the camera module; its caller's caller `FUN_005d2cd0` looks like the
  camera manager's update (loads `CAMERAS.BNDL`, updates that history, then calls virtual method +8 of a
  0x3050-byte object in an array at manager + 0x6a0): the vtable of those objects is the next thing to find. GeneSys has camera process nodes
  (`CameraTransformTracker`, `VelocityLerp`, `Collider`, `EntityAvoider`, `ImpactShake`, `FOVAutoZoom`), which
  suggests the avoidance is more than a push-in along the boom; whether the gameplay camera uses them is open.
- Baseline traces of the current (data-only) camera, 911, for the next attempt to compare against (peak /
  settle over the last second; yaw lag = camera yaw − car yaw; lateral, boom = |(x, z)|, height in car space):
  drift: yaw lag −15.5° / −14.7°, lateral 1.32 / 1.27 m, boom 4.6 → 6.80 (launch) / 5.0 m, height 1.74 / 1.69 m,
  FOV 73.1 / 71.6°; accel: yaw 0, boom 6.81 at 4.5 s / 5.6 m at 308 km/h, height 1.73 / 1.70, FOV 70 → 95°;
  corner (skidpad, 74 km/h): yaw lag −15.7 / −15.4°, lateral 1.49 / 1.47 m, boom 6.79 / 5.5 m, FOV 71.5°.
  crash (114 km/h into the wall at 7.05 s): yaw lag ±0.7°, lateral ±0.06 m; the boom jumps 6.60 → 5.44 m in
  one frame at the hit and bottoms at 4.05 m, settling at 5.6 m; height dips to 1.34 m; impact pitch 10.5°.
  Nothing in our camera steers round the wall or other cars: the only world response is the push-in along the
  ray from the look-at point.

### 2026-10-06 (session 5, round 4): the anti-roll bar and roll damping from the code

The jitter (roll 0.45° p-p at 11.2 Hz at 100 km/h, 0.06° at rest, gone without the bar's damper) came from our
bar damping, which used `Front_` / `Rear_roll_damping` (1.2) as a ratio of critical against the scaled stiffness.

- **Where the game builds the bar**: `FUN_008fda70` (vehicle build) reads `BodyMovement` +0x68
  (`Front_roll_damping`) and +0x7c (`Rear_roll_damping`) and calls `FUN_00933a60` once per axle with the axle's
  two wheel objects, the chassis body, a stiffness (vehicle slots +0x7fc / +0x800), the ratio and an inertia: the
  reciprocal of lane z of the chassis's +0xe0 vector (inverse inertia → roll inertia, inferred). `FUN_00933a60`
  allocates a type-2 constraint element (types registered in `FUN_009333b0`; springs are type 0 via
  `FUN_00933930`) and stores k × 60 (`.bss` 0x01185d70, read live earlier) and 2 (`.rdata` 0x010245d0) × ζ ×
  √(k × I), with k **unscaled** inside the root. So the effective ratio is ζ / √60 ≈ 0.155: ours was 7.7× too
  damped, and with last tick's compression rates the explicit damper rang at 11 Hz.
- **Not found** (searched for about half the budget): the writer of the stiffness slots +0x7fc / +0x800 (they
  are copied, not read from `BodyMovement` directly in any decompiled function; `782f05c1` / `2bb0f6a0` at +0x64 /
  +0x78 is from the field order and earlier sessions), and the per-tick solve of the type-2 element: no
  decompiled function reads element +0x20 / +0x30; the island step `FUN_0093f5a0` calls `FUN_00932af0` (1104
  bytes, no callees), which has no body in the dump: likely the force-element pass (bars and spring dampers).
  Searched: readers of `BodyMovement` +0x64 / +0x78 in all files, the pool at world +0x3ac / +0x3c4, `switch` on
  the element type in 0092–009C, functions loading +0x20 and +0x30 vectors with `sqrt`. So the bar's input
  (compression difference vs. body roll against the axle), where it acts and its integration form stay ours.
- **Ported**: the damping coefficient (chassis.rs `roll_damping` = 2 ζ √k_data, × √I in `drive::anti_roll_loads`).
  Jitter at 100 km/h 0.449° @ 11.2 Hz → 0.011° @ 1.0 Hz, at rest 0.060° → 0.000°. Accel, corner (2.57 g),
  handbrake, drift unchanged; brake 38.5 → 38.6 m; landing deepest 0.055 → 0.058 m; climb deepest 0.101 →
  0.056 m, 133 → 126 km/h (handling.md "Measurements").
- **Open**: the stiffness unit (per radian in our model), the solve (`FUN_00932af0` needs a disassembly or a
  live trace: a probe write-watch on the element's +0x20 lanes would name the reader).

### 2026-10-06 (session 5, round 4): dirt colours, two-tone paint

**Why the dirt drew pink / cyan.** Not the maths, the data or the colour space: our `layered.wgsl` port of
`VehicleNFS13_BodyPaint`'s G-buffer program matches the DXBC line by line (the `gObjectCB` listing in
`mwtool shader-asm` confirms `GenericVector4_A/B/C` at cb1[0..2], `PbrPaintDiffuseColour` cb1[4], scratch cb1[5..6],
`PbrPaintSpecularColour` cb1[7]; effects texture `t1` at `v3.zw` = vertex TEXCOORD2). Debug frames (albedo forced to
the raw effects texture, to its alpha, to `p[10]`, to the UV) showed that even a flat `DirtColour` albedo came out
pink and cyan, so the tint was added after the albedo. Cause: `Look::retain_textures` re-derived the generic vehicle
flags for paint meshes after `with_paint` had written them: p1.x = "has spec map" = slot 1, which on paint meshes is
the effects texture, so F0 became the effects map's rgb at TEXCOORD0 (and p2.y = "has diffuse" multiplied the livery
into the paint before the livery blend). Paint meshes now keep their own flags there (only clearing a slot whose
texture failed to load). After the fix the red and silver 911 at `--dirt 0.6,0.3` show light dust patches and darker
streaks low on the doors and behind the wheels, no hue shift; a flat `DirtColour` albedo renders brown-grey.
The effects texture stays linear: the program squares `e.w` and `e.y` itself (a gamma-2 decode), which it would
not do on an sRGB view. Note from the asm: partial dust darkens (lerp(paint, d·C, d), the same formula as ours),
and covered areas lose F0 and the clear coat, so dust on a white car reads as grey specks.

**The writer of the paint constants** is `FUN_008e9c00` (008E0000.c, the per-object constant block written
through `FUN_00a621a0(slot)`, not `FUN_00a5d660`, which is why the earlier search found no writer). For a render
record (stride 0x2b0 at `param_1 + 0x1a0`) with a paint block `P` at record word 0xa2:
- 0x2a = `P+0x10` (primary diffuse), 0x2b = `P+0x20` (primary specular), 0x2e = `P+0x30` (scratch colour);
- 0x30 = `P+0x40 − P+0x10`, 0x31 = `P+0x50 − P+0x20`: the **secondary** diffuse / specular **minus the primary**,
  so `_TwoPaint*`'s `mad(Secondary, index, Primary)` is a lerp to the secondary paint;
- 0x9d (`GenericVector4_B`) = `P+0x60..0x6c` = DirtColour, 0x9e (`_C`) = `P+0x70..0x7c` = DustColour;
- 0x9c (`_A`) = (`DirtColour.w` × record word 0x28, `DustColour.w` × word 0x29, word 0x2a, word 0x2b).
  So the shader's dirt amount is the car's dirt level × `DirtColour.w` (0.83 on the cars seen) and the dust
  amount its dust level × `DustColour.w` (0.85). Now applied to `--dirt`.

**Two-tone paint.** The second paint comes from the car's `Genesys.Gen.VehicleColourPalette`: field `Colours`
(`16671e74`, a list of paint handles) and `ColourGroups`, each a `ColourGroup` struct of `PrimaryColour`
(`2968e77a`), `SecondaryColour` (`750cf08e`) and `WheelColour` (names cracked with `mwtool genesys-crack`; the
sort order by name matches). The secondary paint's own `PaintColour` is 1 (the primary's 0). Two more palette
fields, `17acabdb` and `db17b169`, are 1 on the police cars seen (unnamed). Bound now: `vehicle_paints` gives a
paint the (diffuse, specular) of the `SecondaryColour` of the group whose `PrimaryColour` it is, and the shader
adds (secondary − primary) × (`mPaintColourIndex` or the mask's green). Check: Ford Police Interceptor
(122713) at `--paint-index 2` (its group's primary, a mid blue) draws its `mPaintColourIndex = 1` roof mesh white.
**Open:** the paint the palette's `Colours` list names (1099e2, dark blue, index 0, our default) is not the group's
`PrimaryColour` (18cf62) on that car, so by default the roof stays blue; on the Corvette Z06 police (122699)
`Colours` and the group agree but both `_TwoPaint_Livery` meshes have index 0. How the game picks the group for a
car (cop cars may always use group 0) needs the code that fills the paint block `P` (word 0xa2 of the render
record; `FUN_008e1280` initialises the record, called from `FUN_008b7a20` and `FUN_008cffa0`).

**Dirt build-up: where it stopped.** The dirt / dust levels are words 0x28 / 0x29 of the render record, copied
as one float4 (with 0x2a / 0x2b) from a parameter of `FUN_008e1280` (record init); its callers `FUN_008b7a20`
and `FUN_008cffa0` were not read, so the source (surface, speed, distance, car washes) is not traced. Dirt stays
off unless `--dirt` is given.

### 2026-10-06 (session 5, round 4): water shaders

User report: water "still looks very weird". Where it is (`MW_WATER_DEBUG=1` on any run logs each water
mesh: shader, renderable, vertex centroid (local for instanced meshes), whether it has TEXCOORD2, and its
textures by sampler): 16 `Water_Opaque_Singlesided` meshes (rivers and ponds: e.g. near 1647,13,2976,
2837,13,4777, 7454,13,4356, 1559,14,−212, −2269,17,−4193; the park pond near −850,·,2000 and the river under
the road-cliff bridge are instanced), 9 `Water_Proto_Cheap` (sea / far water: 6419,19,951, −385,17,5343,
2919,18,−5855, 6259,10,2539, …). Viewpoints `water-river`, `water-pond` (viewpoints.md).

**Before** (pond, noon): a flat slate-blue surface, no waves, no floor, no reflection structure. Causes,
term by term against the DXBC (formats.md "Shaders", Water):
- Pass: the family is opaque in our materials (deferred, our lighting + Fog pass) and in the game (4 G-buffer
  targets): already right; not the Bevy forward-pass bug the trees had.
- F0: ours forced max(specularAndRoughness.x, 0.6) to fake the mirror, so the deferred sun specular and the
  sky term used a wrong F0. The game writes specularAndRoughness.x and gets the mirror from the LUT read at a
  negative N·V instead. Fixed: F0 from the material; RT3 now computed as the program does (reflect(−V, N),
  horizon clamp of R.y, LOD roughness × 5.5, env-BRDF at N·V = 0, × (1 − s.y)) into the RT3 extra, with the
  generic sky term switched off (specular occlusion 0) so it is not counted twice.
- River floor: ours used a constant 0.35 grey because Bevy has two UV sets. TEXCOORD2 now rides in the COLOR
  attribute (`convert::WATER_UVS`; the water layout has no vertex colour), and the floor, its refraction offset
  (SpecDepthAndRefraction.w at 2 × TEXCOORD2 scrolling Time / 2) and the caustics (DiffuseAndCaustics.w at
  world xz / causticTile) are ported.
- Surf (foam): not drawn before; now SurfNormal / Surf / SurfMask as in the program (white albedo, normal and
  roughness lerped by Surf.y × SurfMask.y).
- Waves: coordinates and scroll already matched (TEXCOORD1 × normalMapTile, v + Time / waveAnimationSpeeds).
- `Water_Proto_Cheap`: albedo 0, roughness .y, same reflection: matches after the reflection fix.

**After**: pond shows the floor and algae near the banks and wave structure; the river close up shows the
floor with caustic nets in the shallow part and the sky reflection in the deep part, also at night (floor and
caustics are albedo, so night ambient lights them).

**Open**: (1) the material's `ReflectionTextureSampler` cube (`…157de9`, 128² BC3: grey mountain silhouette in
rgb, alpha 1 above it) multiplies the sky reflection by (rgb + a); we have no cube slot per material, so it is
left out (factor 1): reflected mountains are missing and the reflection below the silhouette is too bright.
(2) The env-BRDF LUT's address mode (AmbientSpecSampler, s14) is assumed clamp; with wrap a negative N·V
would read the head-on column (reflection ≈ F0) instead of the mirror. (3) A straight diagonal edge between
a floor-visible and a deep region at `water-river`: data (two meshes / SpecDepth) or ours, not checked.
(4) `PlotPBR_AO_StandingWater_Opaque_Lightmap_Singlesided` (28 meshes) not ported. (5) No capture of the
original's water to compare against.

### 2026-10-06 (session 5, round 4): Easydrive and HUD from the game's layouts

**Differences found** (our frames at 1280×720 against the layout data, `mwtool ui show`; largest first):

1. **Easydrive panel** was our own (dark box at 4 % / 28 %, orange highlight, Bevy-sized rows, 8 rows). The game's
   is the `EasyDrive` widget's layout 1773380: a single `UIElement_ElementStack` at (3, 43) whose rows are
   further stacks and item layouts (formats.md "UI element stacks"). Element stacks were not parsed at all, so
   nothing the game draws through them (Easydrive, the `EasyDrivePanel`, any list UI) could be drawn.
2. **Speed digits faint**: the speed label's own colour is grey with alpha 0.45 (`1670378/9`, font 581007,
   size 38), so faint digits are what the data says for that label; why the original reads bright is not
   known (a material / blend on the label, or a code-set colour). Not changed.
3. **Odometer ". 0 KM"**: the layout splits the reading: `{Widget.Nitrous.Odometer(DIST,LONG)}` right-justified
   (118×21, anchor 5, font 581007 size 19, alpha 0.56) and a separate "." label at (1209, 696) (font 1206239).
   Our `DIST` format prints `0.0`, which collides with that dot; the game's `DIST,LONG` formatter (code) must
   print the digits around the separate dot (probably integer part, gap, tenths). Not changed: needs the formatter.
4. **Minimap**: drawn as an opaque black square with grey roads; whether the original's map is masked / tinted
   by the frame layout (1206246) was not checked this round.
5. Nitrous bar and "KM/H" / "KM" unit labels: placed and coloured as the data says (unit labels alpha 0.51).

**Easydrive now** (`crates/mostrusted/src/easydrive.rs`): layout 1773380 drawn through `UiScene` (as the HUD)
with the `Widget.EasyDrive.*` bindings filled from the menu state: `Items._length` / `Items[i].Name` (visible
rows), `SelectedIndex` (the selected row template), `CurrentItem.Name`, `TierHistoryLength` /
`TierHistory[i].Name` (header: the titles of the open tiers), `EasyDrive.Length` (footer arrows), item flags
(`Locked`, `New`, `PositionNotNull`) false, `Shutdown` empty (picks the plain name label), `Widget.<TYPE>.IsActive`
false (the generic row). Element stacks are generic in `UiScene::stack`: header, one cell per entry (first true
template, else default), footer; inside a cell `#` / `#parent_index#` are substituted and dot-less bindings are
fields of the entry. Before: our dark box with orange bar; after: the game's white selected bar with its arrow,
translucent dark row strips, game font, at the layout's place (top left).

**Still ours / open**: row pitch = the lowest element bottom of the cell layout (26 px; the `EasyDrivePanel`
moves by 26.42 per item, so the stack's real cell size rule is not decoded: `CellSizeDefinition`, `Padding`
unread); 10 rows shown (from the footer's `Length >= 11` threshold; `NumItemsShown` is code); the six glow images
of the selected row (`01010000001942a0`) and every layout's APPROACH / FADE transitions and idle behaviours are
not animated; the header at the root tier is empty (root has no title); the closed-menu prompt (1805584) and the
`EasyDrivePanel` side panel (descriptions, 1902272) are not drawn (which shows when is widget-adapter code).
HUD layouts not drawn at all: pursuit / cooldown widgets, event prompt (`Prompt`), messages (`UIMessages.*`),
Most Wanted / AdditionalHUD layouts beyond the minimap and road name: each needs its game state (heat, events,
message queue) first.

### 2026-10-06 (session 5, round 4): mods applied, chosen through Easydrive

- 25 of the 55 profile indices are identified from `FUN_00574870`'s call sites (decomp.md "Nitrous and
  upgrades"); `mwtool mods` prints their field names. The rest keep `p<n>`.
- Applying (`mw_assets::mods::apply_to`, `crates/mostrusted/src/mods_menu.rs`): the stock handling is cloned and
  each identified index goes through value × Π + Σ / replace; a `NitrousUpgrade` sets the tank burn to its
  variant's `Nitrous.053c5a6a`. `--mods slot=category` (indices as `mwtool mods` prints them); the spawn logs
  `mods: …`. A selection change rebuilds `CarParams` live.
- 911: no mods = baseline (accel 3.72 s / 11.58 s; nitrous 3.47 s, tank 4.17 s). `12=0` (LONG GEARS PRO: torque
  ×1.05, final drive ×0.85, gear-up speed ×1.175): 0–200 km/h 11.05 → 10.72 s, top 307.6 → 309.1 km/h. `7=0`
  (POWERSHOT NITROUS PRO, burn 24 → 18): tank empty 4.17 → 5.57 s, 0–200 10.33 → 8.93 s.
- Open: the Easydrive "Customize car" pages are not wired yet; the unidentified indices (incl. all body / damage
  ones); `TyreUpgrade` (its `a316f8e3` is a `Tyres` handle, not applied); the top-gear lock that long gears lift.

### 2026-10-06 (session 5, round 4): tyre marks

- **Look** (`Vfx_TyreMarksNew`, G-buffer pass = the `ps_4_0` with three targets): vertex = POSITION.xyz strip centre
  (.w an age-like value), TEXCOORD.xyz the vector across the strip (.w = U along it), COLOR = (Slip weight, Spin
  weight, side 0/1 = V, opacity). Position = centre + across x `WidthScale` x (V - 0.5) + 0.03 m along the up
  vector it derives from the across vector, then 0.35 - 0.33 x saturate(dot(to_eye, up)) m towards the eye (depth
  bias). Opacity out = (1 - `Animation`.x x POSITION.w) x COLOR.w. Pixel: `RunDiffuse` premultiplied, `SlipDiffuse`
  "over" it with weight Slip.a x COLOR.x, `SpinDiffuse` likewise with COLOR.y, all x saturate(opacity), discard
  below 1/2550; RT1 = colour (premultiplied), RT0 = (0,0,0,a), RT2 = `Specular` x a. Details in formats.md "Shaders".
- **Data**: `SurfaceVfx.TyreMarksMaterials` (field `75cb5de9`, inline struct) holds one material per tyre class:
  `eff16a09`, `Offroad`, `Road`, `Sport`, `Track`; also `SurfaceVfx.RimMarksMaterial`. 48 of 65 surfaces have one;
  surfaces 0 and 1 (the `Surfaces` list head) have none. Tarmac `Road` material `…117f16`: Run `…7a932` (8x8, fully
  transparent: no marks from rolling), Slip `…7a92a` (32x32 black, alpha fading at the strip edges), Spin
  `…117f0e`; `WidthScale` 0.65, `Animation` 0, `Specular` 0. `Offroad` = `…117f14`, `Track` = `…117f23`.
- Per car, `TyreVfxBehaviour` (`Front_Tyres` / `Rear_Tyres` -> `Skid_Marks` / `Smoke`, each `Lat_Slip`
  (`LatParams`: `Curve_Power` 0.35 and seven unnamed floats, e.g. 0.1, 60, 50, 0.45, 23.5, 0, 0.2 on `…68fe4`),
  `Long_Spin` and `64222c3f` (`LongParams`)): the intensity curves of the marks. Field names beyond `Curve_Power`
  are not cracked and **the code reading them was not found**: no string in strings.csv names tyre marks, skid
  marks or `TyreVfx`, the field hashes and the struct stride (0x130 = `FrontRearParams`) do not appear as
  constants, and `* 8 + 0x18` (`TyreMarksMaterials` indexed by tyre class) has no hit. Open.
- **Built** (`crates/mostrusted/src/tyre_marks.rs`, `tyre_marks.wgsl`): the shader port above (lit in place,
  blend ONE / INV_SRC_ALPHA), strips per wheel of the driven car. Model decisions (until the rule is traced):
  the Slip / Spin weights are the per-wheel lateral / spin intensities of the skid **sound** rule
  (`FUN_009d1bf0`, decomp.md); a mark is laid while max(lat, spin) >= 0.05; a strip point every 0.25 m; U = metres
  of strip; the across vector is a unit vector (width = `WidthScale` = 0.65 m, wide for a 911: the game may scale
  it by the tyre width); fade linear over 60 s on the CPU instead of `Animation`; at most 4000 segments (oldest
  dropped); the first surface with a material stands in for every contact (surface types not decoded), `Road`
  tyres, dry only. `MW_TYRE_MARKS_LOG=1` logs live segments (drift test: ~100 per second while sliding).
- A mark mesh with the same vertex layout as the skinned car meshes made Bevy's multi-draw batching fail
  (`skinned_mesh_layout` validation error); an extra unused `UV_1` attribute avoids it.

### 2026-10-06 (session 5, round 4): performance baseline and first optimisations

**Benchmark** (`bench.rs`): `--bench <s>` on any headless run waits for `Loaded::done`, warms up 4 s, measures
for `<s>` seconds and prints one `BENCH RESULT` line: frames, mean / median / 1 %-low frame time (Bevy's
`FrameTimeDiagnosticsPlugin`), the main-world schedule time (First..Last), the render-world CPU time split into
prepare (extract .. bind groups, which includes the swapchain acquire and so any GPU wait) and graph (the
`Render` set: encoding, submit, present), shadow casters per cascade, entity / mesh-entity / visible counts,
mesh and image asset counts at start and end (both constant: no streaming during the measurement), RSS and
GPU / backend. `MW_BENCH_PASSES=1` adds Bevy's `RenderDiagnosticsPlugin` (per-pass CPU / GPU timestamps and
pipeline statistics; Bevy's shadow passes and our own lighting / sky / tonemap nodes are not instrumented).
`--no-shadows` turns the key light's shadow maps off (attribution only).

Vsync: on Wayland (this machine: KWin, NVIDIA) wgpu offers no `Immediate`, and `AutoNoVsync` falls back to
`Mailbox`, which the compositor paces at 60 Hz: every run read 59-60 fps, even with one track unit. Under
XWayland `AutoNoVsync` gets `Immediate` (a 1-unit scene went from 16.9 to 11.1 ms), so `--bench` drops
`WAYLAND_DISPLAY` at startup when `DISPLAY` is set; the result line says "X11" or "Wayland, vsync-paced".
`--frame-stats` numbers taken on Wayland before today were capped the same way.

All runs: RTX 3070, Vulkan, 1280x720 window, release build, while other agents were compiling (load average
6-21 on 16 threads). Run-to-run noise is large (up to 2x): each configuration was run twice, the table gives
both. Commands: `--camera=-1131,27,1627,-1190,15,1720` (cliff, full map), `--camera=-650,260,1850,-1000,60,2150`
(city overview, full map), `--drive 911 --units 30 --spawn=815,60,1760,270 --auto-throttle 0.3` (drive), each
with `--mute --bench 10`.

| view | frame mean ms | median | 1 % low | main world ms | entities / mesh / visible | RSS |
|---|---|---|---|---|---|---|
| cliff | 28.0 / 19.4 (later 17.6) | 26.6 / 17.4 | 59 / 45 | 18.9 / 10.1 | 178 811 / 107 868 / 3 257 | 1.3-1.8 GB |
| city | 18.7 / 18.5 | 17.3 / 16.9 | 41 / 69 | 10.1 / 9.9 | 178 811 / 107 868 / 3 651 | 1.9 GB |
| drive | 29.0 / 35.2 (later 20.3 / 18.3) | 28.8 / 31.8 | 60 / 90 | 9.2 / 12.4 | 34 394 / 20 755 / 3 845 | 1.6-1.9 GB |

**Attribution** (same viewpoint, two runs; "saves" against the quieter baseline runs):

| switch | cliff frame ms | drive frame ms | saves |
|---|---|---|---|
| baseline | 19.4 / 18.7 | 20.3 / 18.3 | |
| `--units 20` (23 341 entities) | 19.6 / 19.1 | | nothing: the 150 unloaded-from-view units cost no frame time |
| `--no-groundcover` | 44 (busy) / 16.0, main 8.1 ms | | ~2 ms main world (15 000 group entities, visibility) |
| `--no-backdrop` | | 21.2 / 29.9 | nothing measurable |
| `--no-shadows` | 21.7 / 16.6, main 6.5 ms | 11.2 / 13.1, main 4.0 / 3.7 ms | 2-3 ms cliff, ~7 ms drive: the largest single cost |

GPU utilisation during the drive bench (nvidia-smi, 1 Hz) was 25-40 % at 21-50 fps: the runtime is **CPU
bound**, and mostly in the **render world**: render-world CPU (prepare + graph) 13.5-19 ms against a main
world of 5-8 ms, so with pipelined rendering the render world sets the frame time. GPU timestamps of the
instrumented passes are small (deferred G-buffer 0.4-0.5 ms, bin unpacking 0.5-1.7 ms).

A chrome trace of the drive bench (`--features bevy/trace_chrome`, one build; the trace itself slows frames
~2x, so read proportions, not absolutes) puts the render world's CPU in Bevy's GPU-driven batching, per view:
`write_binned_instance_buffers<Shadow>` 2.4 ms and `<Opaque3dDeferred>` 1.8 ms, `prepare_preprocess_bind_groups`
2.5, `queue_submit` 1.9, `unpack_bins` 1.6, `update_sparse_buffers` 1.3, shadow pass encoding 1.2 ms per frame.
These scale with views (the 3 shadow cascades are 3 more views) and with bins (one per distinct mesh x
material visible in a view; world geometry is almost all unique meshes, so bins ~ visible entities), not with
the loaded entity count. Main world: avian physics 2.75 ms per frame (1.27 fixed steps per frame, 6 substeps),
visibility propagation and frustum tests ~2.5 ms of CPU across threads, `calculate_bounds` 0.7 ms. Our own
systems are small: `hud::draw_hud` 0.4 ms, everything else under 0.1 ms (`groundcover_distance` included).

The floor: one track unit with `--no-shadows` (3 647 entities) ran at 11.1 ms (main world 3.5 ms) on a
quieter machine and 19.4 ms (main world 12.4 ms) at load average 15, so on this shared machine CPU contention
alone moves every number by up to 8 ms, and the full map adds only ~0-8 ms over that floor. Some runs landed
on exactly 600 frames in 10 s (60.0 fps) on the X11 path too; an unloaded machine is needed to rule out a
remaining pacing cap before comparing small changes. A drive pair run back to back: `--no-shadows` 16.7 ms
(main 4.2, render 6.6 + 7.7 ms) against 36.8 ms with shadows (main 14.5, render 17.3 + 18.1 ms).

So the candidate list's distance culling, unloading far units and merging groundcover would not move the
frame time at these viewpoints: the work is in what is visible, per view. What would: fewer bins per view
(merging static world meshes per material within a track unit, which also cuts shadow bins), fewer shadow
views or casters (the cascade split and the 180 m shadow distance in `deferred.rs` are provisional, not from
the game: measuring the game's `ShadowMap_ScaleOffset` / distance first is the fidelity-safe order), and
physics on its own thread budget. No optimisation was landed in this round: the measured costs did not
match a change that keeps the frame identical and fits the round.

### 2026-10-06 (session 5, round 3): vehicle liveries, paint masks and dirt

- **Paint variants** across all car bundles (`mwtool models <bundle> --verbose`): plain `BodyPaint` (53 cars),
  `_Lightmap` 31, `_Livery` 10, `_Livery_Lightmap` 5, `_TwoPaint` 6, `_TwoPaint_Livery` 5 (the police cars
  122699, 122706, 122713, 510072, 2076005), `_TwoPaintMask` 9, `_TwoPaintMask_Lightmap` 2, `_NormalMap_NoDamage` 4.
- **Liveries are material data, not paint options.** The livery is the material's `DiffuseTextureSampler`
  texture (BC3, alpha 0 where the paint shows); `Genesys.Gen.VehiclePaint` has no texture field (its 21 fields:
  `PbrDiffuse/Specular/Pearlescent/ScratchColour`, `DirtColour`, `DustColour`, clear coat and scratch floats,
  `Display_Texture` = the menu swatch, `Type`, `PaintColour`). So `--paint-index` and liveries are independent;
  no `--livery` flag.
- **Paint pixel program** (longest `ps_4_0`, object CB `gObjectCB`): `GenericVector4_A/B/C`, then
  `PbrPaintClearcoatSettings`, `PbrPaintDiffuseColour`, `PbrPaintScratchColour`, `PbrPaintScratchSettings`,
  (`PbrPaintSecondaryDiffuseColour`, `PbrPaintSecondarySpecularColour` in the two-paint variants),
  `PbrPaintSpecularColour`, `VehicleID`; material CB `mPaintColourIndex` (0 / 1 per mesh). Maths:
  `_Livery`: paint = lerp(paint, livery.rgb, livery.a), livery at TEXCOORD0. `_TwoPaint`: diffuse =
  `PbrPaintDiffuseColour + mPaintColourIndex × PbrPaintSecondaryDiffuseColour` (specular likewise);
  `_TwoPaintMask`: the same with the DiffuseTextureSampler's green at TEXCOORD0 in place of the index.
- **Dirt / dust** (all paint variants): EffectsTextureSampler (material-bound, shared) at TEXCOORD2:
  dirt a = sat(e.w² × A.x), dust d = sat(2 × sat(e.y² × A.y) × e(uv×7).x); coverage = a + d(1 - a);
  colour lerps towards a·B(1 - d) + d·C (B, C = GenericVector4_B / _C, whose data are the paint's
  `DirtColour` / `DustColour`); roughness lerps to 1, F0 × (1 - coverage), clear-coat reflection ×
  (1 - coverage)⁴. **Scratches**: e(uv×(2,3)).z band × sat(2·e.z - 1) × TEXCOORD5.w (the damage weight from the
  vertex program) lerp the colour to `PbrPaintScratchColour` and roughness / F0 to `PbrPaintScratchSettings.xy`.
- **Code**: the object-constant names are registered with slot numbers in one table function in the
  0x8c4xxx range (calls to `FUN_00a5d490`): `PbrPaintDiffuseColour` 0x2a, `PbrPaintScratchColour` 0x2e,
  `ScratchSettings` 0x2f, `SecondaryDiffuse` 0x30, `SecondarySpecular` 0x31, `GenericVector4_A` 0x9c. Who writes
  these slots for the car (the per-object setter, not `FUN_00a5d660`) was not found yet.
- **Built**: liveries drawn (`materials.rs` paint branch, `layered.wgsl` mode 6). Police Corvette
  (`--vehicle "corvette z06 police"`) shows its POLICE / FAIRHAVEN CITY livery over the dark-blue paint.
  `--dirt dirt[,dust]` enables the dirt port.
- **Open**: (1) the secondary paint's source (no VehiclePaint field names it; two-paint meshes with index 1
  draw the primary paint); (2) the dirt port tinted clean paint pink with orange blotches even at zero
  amounts in the first test, so it is only enabled by `--dirt` until that is found (suspect: UV_1 = TEXCOORD2
  on these meshes or the p9–p11 uniforms); (3) the dirt accumulation rule and who sets GenericVector4_A;
  (4) scratches (need the damage weight); (5) whether the mask texture is sRGB (read linear here).

### 2026-10-06 (session 5, round 3): fog as the game computes it; fog on trees

- **Why trees floated unfogged** (measured): with the Fog/lighting pass forced to output red, every pixel
  turned red except the trees. Bevy 0.19 queues `AlphaMode::Mask` materials in the forward `AlphaMask3d`
  phase even when their render method is deferred (`bevy_pbr` material.rs: only the opaque branch checks
  `OpaqueRendererMethod::Deferred`), so trees (`Tree_Translucent_1Bit_*`), and every other 1-bit look, were
  written to the G-buffer, lit and fogged by our pass, and then drawn again on top with Bevy's forward PBR and
  no fog. In the game the tree programs only write the G-buffer (two render targets, `discard` below the alpha
  reference, no fog constants) and the Fog pass fogs them with the terrain. Fix: layered.wgsl's forward
  branch discards when `DEFERRED_PREPASS` and `MAY_DISCARD` are both defined (views without a deferred prepass,
  such as the planar reflection, keep forward shading). After: trees and the hills under them are fogged alike
  from the high and low cameras; at road level the trees are now lit by the game's key light and ambient.
- **The Fog pass** (`mwtool shader-asm Fog`, ps 01010008ce7671f3; port in `apply_fog`): distance = Euclidean
  camera → pixel in world metres (view-space reconstruction from depth); t = saturate(d × ScattCoeffs.x −
  ScattCoeffs.y)^ScattCoeffs.z; t' = t × gradient alpha; in-scattering t' × saturate(ScatteringSettings.y /
  (1 + ScatteringSettings.x × cos)²) × ScatteringColor (we clamped at 64 instead of 1); a height-layer term
  exp(−|d × (C.z / (1 + exp(C.x Δy + C.y)) − C.w) / Δy|) from `LayerFogCoeffs`; sky pixels (depth 1) get only
  the layer term and the scatter. The fog colour is a 1D texture sampled at the screen u.
- **CPU side** (docs/decomp.md "Fog"): `ScatteringSettings` = (g, (1 − g²) × strength / 4π); now applied, so
  in-scattering is on again (0.0039 / (1 + 0.971 cos)² at noon: ≤ 0.001 away from the sun, 1 into it).
- **Still open**: whether near / far are scaled on the CPU (the shader's form matches 1 / (far − near),
  near / (far − near), Power, so 80–500 m stays; the mountains beyond 500 m stay fully fogged); the gradient
  texture builder (rgb approximated, alpha taken as 1: if its alpha is below 1 at noon the hills keep detail,
  which is the most likely reason the original's hills are hazy rather than flat); the `LayerFogCoeffs`
  source; the sky-pixel fog. A live read of slots 0x23, 0x27, 0x83 and of the gradient texture in the running
  game would settle all four. Groundcover goes through layered.wgsl masks too; forward glass, particles and
  sparks still get no fog (they need the fog uniform in their bind groups).

### 2026-10-06 (session 5, round 3): landings, steep climbs, jitter

- **Reproduced** with new test-track tests (`--test landing`, `--test climb`, `--test jitter`; handling.md
  "Measurements"). Before: the 911 lands from a 3 m drop at 150 km/h with −8.6 m/s; 0.14 m of travel and the
  load cap (4 × corner weight) cannot stop that, the wheels end 0.34 m past full compression and the body box
  rests on the ground; the climb (120 km/h into 30°) ends STUCK at 0 km/h, sunk 0.478 m.
- **Why it stayed stuck**: once the wheel mount is under the ground the wheel ray starts inside it: against the
  test track's solid box the hit is at distance 0 with a zero normal (spring force × 0), against a trimesh (the
  world) the ray misses the back face (wheel airborne, in-air gravity on). Nothing lifts the car off its body box.
- **The game**: the strut's end is a hard contact on the chassis (`FUN_009c5a90` → `FUN_009c2670`, decomp.md).
  Ported: the speculative non-negative impulse at each wheel contact once the ground reaches the strut's end;
  the ray now starts a wheel radius above the mount (stopgap). After: landing sink 0.055 m, no overtravel,
  settled 0.45 s after touchdown; the climb completes. Regression tests unchanged.
- **Jitter** is a roll buzz at 11.2 Hz: 0.45° p-p (37°/s) at 100 km/h, 0.06° at rest. Turning the anti-roll bar
  off removes it (0.03°), and so does its damper alone (0.008°): the bar damper (`Front_` / `Rear_roll_damping`
  × critical against the roll inertia, an interpretation) is explicit and uses the previous tick's compression
  rates, which rings. Not fixed: the bar's unit and damping are not traced in the code (decomp.md "Unknown").
  Pitch 0.75° p-p at ~1 Hz at 100 km/h is the test's throttle controller, not jitter.
- The body collider reports world contacts on the 5.7° kicker without touching (speculative contacts in
  `collisions_with`), which also feeds `wall_time` for the turn-around assist; worth checking.
- Open: the strut travel (+0x460, from the builder's caller), the slack constants (`.bss` 0x01186bf0, 0x01186b10:
  a live read), the sub-step count of `FUN_0093f5a0`.

### 2026-10-06 (session 5, round 3): gearbox modes and part-throttle shifting

User report: "sometimes it kind of sounds like the vehicles are stuck in a gear". Reproduced on the test
track with `--auto-throttle` (log once a second, km/h : gear : rpm). Before: the 911 at 0.3 held first gear
for 17 s, 1100 → 7650 rpm, shifting at 72 km/h; at 0.5 it shifted 1→2 at 68 km/h, 7391 rpm; the XKR at 0.5
held first to 6499 rpm (52 km/h) and every gear to ~6500; the Veyron at 0.7 shifted at 6300–6600 rpm in every
gear. Cause: we only had the race gearbox (911: `Gear_up_RPM` 7700 in every gear, `Gear_up_speed` 25 / 40 /
80 / 120 / 150 / 175 mph), whose shift points sit near the limiter whatever the pedal.

The code (decomp.md "Gearbox modes"): `FUN_009ce630` switches to the `CruisingGearbox` after 3 s of
0 < pedal < 0.95 with no brake, and back after 0.05 s above 0.95; `FUN_00900660` applies it (tables reloaded,
gear clamped on the way back); `FUN_009dc3c0` takes the cruising `Gear_change_time` just after a kick-down.
The 911's cruising box: 5500 rpm, 15 / 45 / 70 / 85 / 105 / 135 mph, `Gear_down_RPM` 2000, 0.9 s per shift,
4th / 6th / 7th 1.30 / 0.88 / 0.71 instead of 1.225 / 0.92 / 0.77, top gear not locked.

After (same runs): 911 at 0.3: cruising box from 3 s, 2nd at 68 km/h, 3rd at 88 km/h (5477 rpm); at 0.7:
2nd at 67, 3rd at 81, 4th at 128, 5th at 154 km/h, all near 5500 rpm, then 4500–5400 rpm cruising; XKR at
0.5: 1 → 3 at 56 km/h (the target skips gears as `FUN_009d4c80` does), 4th at 100 km/h, 3000–4000 rpm;
Veyron at 0.7: shifts at 4100–4500 rpm, 7th (cruising box) at 247 km/h. Full throttle never qualifies, so
`--test accel` (911: 0–100 km/h 3.72 s, quarter mile 11.58 s, top 307.6 km/h) and the `ref/xkr_launch` replay
are unchanged (compare output identical).

Still open: (1) the first gear at part throttle still runs to ~0.95 × the race `Gear_up_RPM` (911: 7262 rpm at
68 km/h at 0.3): the launch state `FUN_009d17d0` keeps the clutch slipping until max(throttle, clutch, rpm /
+0x130) ≥ 0.95, and +0x130 was set from the race box at standstill. That is the game's rule, but our clutch at
partial engagement is almost rigid, so the rpm only gets there with the road speed; the game's clutch
(`FUN_009d7ac0`) may let the engine flare ahead and close sooner. Port the clutch's slip torque to settle it.
(2) `.bss` 0x011871e0 (kick-down window, assumed 1 s), gearbox +0xb8 (added to the shift time) and the
vehicle's vtable +0x34 flag (blocks the cruising box) are unread. (3) The sound side (limiter and gear-change
sequences of the bank) was not examined: the engine sound still follows the physics rpm.

### 2026-10-06 (session 5, round 3): nitrous and the upgrade profile

- `FUN_009d2aa0` decoded (decomp.md "Nitrous and upgrades"): mode 0 is a torque-multiplier lerp to
  `Engine.Nitrous_strength.e9de413e` (1.9 on the 911); modes 1 / 3 (punch, jump) add a forward force × mass
  faded from `Punch/Jump` fade-start mph to vehicle +0x7e8. Ported as `mw_handling::nitrous` with the gearbox's
  nitrous rules; `--test nitrous` on the 911: 0–100 3.47 s (3.72 without), 0–200 10.33 s (11.05), quarter
  11.25 s (11.58).
- `FUN_00574870` decoded: a 50-entry profile (operand, op, index, 64-bit active mask), multiply / add /
  replace. The mods are `Vehicles.PerformanceUpgrades` (two categories of slots; ModifierUpgrade /
  NitrousUpgrade / TyreUpgrade); `mw_assets::mods` loads them, `mwtool mods <car>` prints slot → choice →
  `p<index> x|+|= value`. Open: the index → handling-field map (call sites with literal 0x00..0x36), which
  category is pro, the slot names (field hashes), and applying a selection (`--mods`) in the runtime.
- Open: the multiplier's reader (assumed engine torque), the tank code and its refill rules (the data is
  `NitrousParameters`, five variants per car), `.bss` 0x0118a018 (punch / jump force scale), vehicle +0x30e8.

### 2026-10-06 (session 5, round 3): car animations, active aero

- **Where active aero lives**: `Genesys.Vehicles.AnimationStateBehaviour` → an `HSMData` state machine + its
  `Animation` clips on the damage skeleton (layout in formats.md "Vehicle animations"; 16 models).
  `PhysicsDrivenVehicleAnimationBehaviour` / `AnimatedBodyPart` turned out to be the exposed suspension and
  wheel-angle bones of four open-wheel cars, not aero (dead end for aero; useful later for suspension).
  `AnimatedRaceCarBehaviour` has no vehicle instances checked yet.
- **HSM conditions are in the data as text**: the 911's machine reads `speedinmph`, condition `Speed above
  70mph`, operands `70` and `40`; LFA `70` / `50`; the SLS / SL65 labels give 74 / 75 mph but no bare operand
  string (probably stored binary). Veyron, McLaren and Venom read `brake` too (`Brake ON and Speed above
  80mph`, `Wait 0.4s`); the Huayra reads `steering` (`Speed between 50 and 137mph and Turning Left`); the
  Agera R reads `gas`. Unknown operands per state: `False` (reverse play?) and `0.25` (blend time?).
- **Code**: the variable name strings (`.rdata` 0x00fae7e0..0x00fae7f4) have no references in the dump;
  the condition evaluator and the variables' producers are not found (docs/decomp.md "Vehicle animations").
  Next step: decode the HSMData property tree so every machine can run, and look
  for the HSM interpreter through the `HSM.AnimationCondition` type string.
- **Built**: `mw_assets::car_anim` (loader, simple speed rule), clips composed on top of the damage pose
  (`VehicleDamage::pose_with`), `car_anim.rs` (`--anim all=1` / `0=0.5` pins, otherwise the rule in
  `--drive`), `mwtool damage <car>` lists the parts. 911 `--test accel`: deploys at 70.2 mph, fully out
  1.6 s later. The `--vehicle` viewer draws static instances without the skeleton, so `--anim` needs `--drive`.

### 2026-10-06 (session 5, round 2): traffic rules in the code

Details and formulas are in decomp.md "Traffic rules (session 5, round 2)".

- **Found (confirmed by reading the code).**
  - Frame function `FUN_0060ebc0`; its local-game tick is `FUN_0060eab0`.
  - Active area = the player's hull cell plus that cell's listed neighbours (`FUN_005f0270`). The union and
    diff of that set is in `FUN_0060c9f0`.
  - Emitters are lane sections at the edge of the active area or with no predecessor (`FUN_0061c780`).
  - Spawn interval `60 / (m·Density)` with a random factor (`FUN_005fb120`): **`TrafficFlow.Density` is cars
    per minute per lane**.
  - A new car is skipped unless the gap to the last car on the section is at least `Speed × 2 s`
    (`FUN_0060e5e0`). The 2 s is read from the `DAT_0110f940` object.
  - Vehicle pick by `Proportion` from the flow type that section byte `+0x25` selects (`FUN_00608c80`). This
    settles the byte: it is the `TrafficFlowType` index.
  - Density multiplier = `TrafficSystemConfig` 0x80 (4.78), ×1.2 when the player is slow (20 → 60 mph ramp),
    ×`PursuitScale` 0.5 in a pursuit (`FUN_005f0780`).
  - Load budget 200 within a `.bss` radius: 20 per moving car, 5 per slow or stopped car, 30 per physical car.
    No spawning at 200 or above; above it the farthest car is removed every 0.5 s (`FUN_005fd6a0`).
  - The game's random source is a 64-bit LCG (`0x5851f42d4c957f2d`) feeding an 8-slot delay ring.
- **Inferred.**
  - The `DAT_0110f940` object is `TrafficParamConfig`: the values at every offset read fit (0x10 = 4 m
    standstill gap, 0x60 = 25 m minimum look-ahead, 0x6c = 2 s spawn headway, 0x88 = 2 s look-ahead time,
    0xac = 3).
  - Car following in `FUN_005fc220`: look-ahead `1.5 × max(2 s·v, 25 m)` (`FUN_005f1380` scans for the car
    ahead), and gap = `dist − back − 4 m − 3·x`.
  - World vehicle slots 256-405 / 406-429 may be parked cars.
- **Dead ends.**
  - The 24-entry hour array of `TrafficSystemConfig` has no direct reader among the users of `world+0xb17b0`.
    The only other way the base density changes is message 4 in `FUN_0060a380` (the sender is not traced).
  - `sys` 0x74 / 0x78 (1500 / 205) are read only by the setup function `FUN_005ef0f0`, into SIMD constant
    blocks at `world+0x35ff0..`. Their use is not traced.
  - `fn` cannot show the start value of the random lerp in `FUN_005fb120`: x87 temporaries were lost in
    decompilation.
- **Stopped at.**
  - `FUN_00617d00`: read only up to branch 1. It is a four-way behaviour arbiter: `FUN_005ea6a0` scores four
    options, the best one wins, and the cruise target is `Speed·mph·world+0xa2bfc`. The scorer and the
    acceleration limits (`MinAcceleration` −20 is in the same object) are not traced.
  - The rung advance and pose: `FUN_005f0d90`, `FUN_005f7cf0`, `FUN_005fcbd0` (the switch to a physical car).
  - The other tick steps: `FUN_00605d30`, `FUN_00602ae0` (it calls the "Crashing Traffic" network helpers
    `FUN_006180d0` / `FUN_0061a550`), `FUN_005fbbf0`, `FUN_0060ba30`, `FUN_0060b9a0`.
  - Junction records: reached through hull lane data `+0x28`. `FUN_0060c9f0` walks a `u8` at junction `+0x35`
    with entries near `+0x56`.
  - The player-stopped-for-30 s action `FUN_005fd3f0`.
- **`.bss` values needed (live read).**
  - `DAT_0110f940` / `DAT_0110f944`: which GeneSys object, and fields 0x48 and 0x6c.
  - `DAT_0118a090` (a rate scale in `FUN_00617d00`) and `world+0xa2bfc` (the global cruise-speed scale).
  - `DAT_0110f590`: the squared radius of the load budget.
  - The base density `world+0xa2b74` during free roam at several hours, to see whether the hour array reaches
    it.

### 2026-10-06 (session 5, round 2): free-roam scoring, trigger hits, jack spots

Read from the dump only (no live reads); details and formulas in decomp.md "Trigger hits" and "Billboards,
gates and other kinds".

- Settled: the header +0x1c index is per-250 m-cell lists of trigger indices; the streamer `FUN_0053bc70`
  activates the 2 × 2 cells around each player car and keeps at most 300 triggers live (`FUN_00539e50`, the
  "300 entries" of the loader). Box size (+0x24) is the **full** extent (the shape gets size × 0.5); points
  get no shape and are never overlap-tested. Flags bit 0 makes a box **one-way** (`FUN_00536f70`: counts only
  if `dot(vehicle vector, box Z axis) ≥ 0`); bit 1 is not read by the trigger code (meaning still open).
- Firing (`FUN_00707650`): on entry only (diff with last frame's per-vehicle list), already-found triggers
  dropped except speed cameras, then a ring buffer drained one hit per 4 s (`DAT_00fcb050`) into
  `FUN_007059b0`. The overlap test itself runs in the `"TriggerUpdate"` job (`FUN_00536a10` builds per-vehicle
  queries from the car's half-extent box); its narrow phase was not read, so "car box vs trigger shape" is
  inferred, not confirmed. Our runtime's origin-in-box test stays a model decision until then.
- Billboards: counted when the **prop breaks** (`FUN_005b5340`, the only poster of game-logic message 0x14,
  checks the smashed object's linked trigger id at +0x90 and the billboard kind). Gates: overlap path, plus a
  helper kind 7 / 'I' that hits the nearest '0'/'H' gate (`FUN_00537600`).
- Speed camera result goes to the Autolog leaderboard request in `FUN_008749e0` (`"eventId"`, board
  `"RoadRules"`); offline `FUN_00874830(id)`.
- Jack spots: a GeneSys list at manager +0x41b0 pairs two trigger ids (+0x20, +0x28) with a key (+0x24);
  `FUN_007061d0(key)` marks a pair found. Probably `CarSwapSpot` (44 bytes, eight UniqueIds), not confirmed:
  `mwtool genesys all --type CarSwapSpot` printed nothing, so the instances are embedded or in a bundle the
  tool does not scan.
- Dead ends: `BillboardWorth` / `ItemWorth` / `HighValueWorth` are Smash-and-Grab mode variables
  (`HSMBehaviourHSMDoSmashAndGrabScoring`), not free-roam points. No other poster of message 0x14 exists (all
  functions using the game-logic queue `DAT_010edb10` checked).
- Stopped at: the job's narrow-phase test (find the function `FUN_00da2e50` dispatches for "TriggerUpdate";
  `FUN_00536f70` is its per-hit callback), the body shop tail of `FUN_007059b0`, Speed Points (strings
  `CurrentSpeedPoints`, `SPEEDPOINTS`, `upload_sp_buckets`; listeners in `FUN_006a8050`), jack spot car
  spawning and the swap input (the `JackSpots` script and `JackspotScript_ScreenScript`), kinds 11
  (`FUN_006e4770`) and those of `FUN_006e16c0` / `FUN_006e1690`. Live reads wanted: game-state bytes
  `+0x2e90`, `+0x2e91`, `+0x2e95` (jack spot and speed-camera modes) during free roam.

### 2026-10-06 (session 5, round 2): braking instability and the steering's neutral offset

Question: in `ref/xkr_launch` (Jaguar XKR, rows at 60 Hz) the original turns and stops from 64 mph in 1.3 s
under straight braking (6.0–7.1 s, brake 1, steer 0); our replay brakes straight and is at 29 mph at 7.5 s.

- **The original hits something twice; the tyres do not start it.** Between two rows (6.017 → 6.033 s) the body
  velocity jumps from (28.4, 0.0, −1.83) to (21.0, 0.53, −4.91) m/s with a vertical kick and the yaw rate from 0
  to 61°/s: a 7.4 m/s change in one 1/60 s tick (≈ 45 g), at (1834.4, 38.8, 1983.1). The second one at
  6.600 → 6.617 s, (1845.2, 38.8, 1981.6): the velocity turns 34° in one tick (15.5 m/s straight on → 8.9 forward,
  5.8 sideways) and the speed drops from 34.8 to 25.6 mph. Before either the wheels show nothing: κ ≈ 0 on all
  four at 6.00 s, α < 0.2°. After the second the car slides at 30° body slip with the fronts locked (ω 0, κ −1 from
  6.78 s) and stops at 2.3 g (sliding sideways, possibly still in contact). The wheel `N` stays 4580–4750 front,
  4050–4300 rear throughout.
- **Between the hits the braking matches ours.** 6.08 → 6.58 s the original loses 5.6 m/s (1.13 g, slip within
  ±2°, the drift flag on from 6.07 s with `steer_norm` creeping to +0.26 from the drift's remap); our replay
  brakes at 1.1 g over its first 0.5 s and 1.4–1.6 g later. So the mismatch in the window is the collisions,
  which the flat test track of `--replay` cannot reproduce (our car is also 5 mph faster at the brake point:
  68.8 against 63.7 mph, an acceleration difference). This run is not usable for brakes or tyres after 6.0 s.
- **Open lead (brakes):** in the clean stretch the original's rear wheels show κ 0 … +0.07 under full brake with
  the clutch in (gear 3, 4300–4600 rpm), the fronts −0.05 → −0.51: the rears are not visibly braked although
  the XKR has 2000 N·m rear brake torque. Ours shows rear κ −0.1 … −0.3. Needs a clean straight-line brake capture.
- **q (vehicle +0x2c70) is the body slip / 35°.** The probe's `steer_q` equals atan2(v · right, |v · forward|) /
  35° to four digits from 0.5° to 33° (both events), times speed / 10 mph below 10 mph (q / slip / speed constant
  over 1–4 m/s). Outside the drift `steer_norm` follows −0.3 q at the steering rate (−0.20 at q 0.63, −0.09 at
  q 0.21), and that steers the fronts towards the velocity (their slip angle is the body slip plus yaw minus the
  wheel angle): a counter-steer assist. Writer still not found: every `0x2c70` in the dump (0x004–0x011) is either
  a read (`FUN_00904260`, `FUN_00902930`, `FUN_008f8cd0` as `[0xb1c]`) or a write to the game-mode object at
  +0x2f3fc (`FUN_006e0580` adds to it, 0x006c / 0x0074 / 0x0076 set it); the 4-float vector +0x2c70..+0x2c7c is
  never stored by offset, so it is likely written through a pointer to the block. 35° equals the XKR's
  `Maximum_steering_angle`: a capture of a car with another maximum would tell a constant from the field.
- Ported (drive.rs `steer_towards`, `steer_slip_fraction`): the offset q × `Maximum_steering_angle` ×
  `553bc073` (0.3; 1 in reverse; 0 in a drift), its side-limit extension `a40a6b17` (1) / `16fd6022` (0), and the
  centring test against the offset (decomp.md "Steering"). The replay is unchanged (RMS speed 15.1 mph, yaw rate
  7.8°/s, slip 5.5°: our car never slips there); 911: 0–100 km/h 3.72 s, 100–0 km/h 38.5 m, skidpad 2.57 g, all
  unchanged; handbrake test slip 1.2° → 1.1° (yaw 59°/s), drift test peak 4.5° (no drift) both ways.
- Not reached: the tyre +0x210 combined-slip scale and the brake torque split in the code (the run gave no
  clean evidence on them).

### 2026-10-06 (session 5, round 2): groundcover draw distance and sprites

Found the CPU side of groundcover: `FUN_00eb9500` (and a twin, `FUN_00eb8390`) walks a group's instances
and writes four 0x2C-byte vertices per visible sprite into a dynamic buffer (the `Groundcover` vertex
layout: position, normal, binormal, colour, half-float uv), then records the draw with the material at
`collection+0x30 + material·0x110` (found by grepping for that stride) and the group's byte +0x30. So the
GPU never sees the packed instances. What it says:

- **Group test**: distance from the camera (`param_6`) to the nearest point of the group box ≤ group+0x3C ×
  `DAT_010c6f74`. `DAT_010c6f74` is .data, 125.0 on disk (could be changed at runtime; nothing seen writing it).
  group+0x3C is the box margin `r`, so a group with 1 m sprites is drawn to 125 m.
- **Group byte +0x30 is a size exponent e, not a quad count**: the test scales size bytes by
  `(1 << (e & 0x1f)) × 1/127` (`DAT_01081004`). Values 2/4/6/8. Our old "1 to 4 crossed quads" was a guess;
  the game draws one quad per instance.
- **Instance +6 / +7 are width / height** (signed `char`), not yaw / size: BINORMAL = (w·f/127, h·f/127,
  byte 11/127). The shader uses BINORMAL.x × `cb1[0].x` as the width and .y × `cb1[0].x` as the height, and
  the sign of the width for its normal. Where the two bytes are equal (the "yaw equals size" note) the
  sprite is square.
- **Per-sprite test and fade**: drawn if in front of the camera plane (`(p − eye)·forward ≥ 0`, `param_7`)
  and |p − eye|² ≤ R², R = ((w>>1 & 63) + (h>>1 & 63)) × 2^e/127 × 125; then f = min((1 − d²/R²) ×
  `DAT_01081000` (5.0), 1) multiplies the width and height bytes (truncated to an integer). Sprites shrink
  to nothing over the last fifth of their range in squared distance; no alpha fade (the pixel programs
  only `discard` alpha < 0.5). Cap `DAT_010c6f70` = 20000 sprites per frame and 300 draws (`param_11+0x12c0`).
- **Vertex program**: right = (−d.z, 0, d.x) with d = normalize(instance.xz − ViewPosition.xz): yaw-only,
  turns about world up; vertex = p + right × (width × (u − 0.5) + sway × v) + up × height × v, sway =
  sin(Time.x × `speed` + p.x) × `amount` (material parameters), so only the top sways, sideways. Normal for
  lighting = sign(width)·right·(u − 0.5) + up·v − 0.1·d, plus max(8 − height, 0.75) × (normal + (0, 0.25, 0)).
  Texture region = `GroundcoverTextureRegions[colour.a × 255]`, uv (u, 1 − v) × scale + offset.
- **Not found**: who fills `GroundcoverConstants` (slot 0x7F; no `FUN_00a5d660(0x7f)` call). The draw record
  carries e, so `cb1[0].x` is probably derived from it; with 2^e it would be 2^e/127 m per byte, which is
  the old byte/32 only for e = 2 and up to 2 m per byte for e = 8 (implausible for grass, unchecked). The
  size stays byte/32 (model decision, `GroundcoverInstance::size_m`). `mwtool placement-check --groups -v`
  now prints e, r and the largest width / height bytes per group (`MW_GC_VERBOSE=1`). Result: **r is in
  2^e/127-per-byte units**, r ≈ ((w + h) / 2) × 2^e/127 of the largest sprite (e 2: 62/62 → 1.95 vs r 1.974;
  e 4: 47/47 → 5.92 vs 5.938, 37/18 → 3.46 vs 3.504; e 6: 21/87 → 27.2 vs 27.6; e 8: 7/32 → 19.7 × 2.016 =
  39.7 vs 40.9), so 125·r is the largest per-sprite range of the group. e = 8 groups are drawn to ~5 km;
  with byte/32 they are 1 m tufts (the far specks the audit saw), with 2^e/127 they would be 14 x 64 m
  sprites (far-vegetation cards?). Which is right needs a look at the original or the 0x7F writer; this
  session could not run screenshots (the agent sandbox refused to launch `mostrusted`), so sizes stay byte/32.
- **Unit visibility**: the builder only sees groups handed to it (by the `GroundcoverJob`, string at
  0x1002028 with no code reference in the dump); not traced whether a group needs its track unit loaded.

Implemented: one mesh per group, hidden by `groundcover_distance` (main.rs) when the camera is farther than
125·r from its box. Not done: the per-sprite range / shrink (needs a vertex-shader path; merged group meshes
draw every sprite of a visible group at full size) and the camera-facing turn (two crossed quads stand in).

### 2026-10-06 (session 5, round 2): ground texture filtering, sampler states, WorldPBR ground variants

Follow-up to the "wrong scale on road and terrain" report, after the UV audit below found the UV maths
matching. Five candidates checked; only the anisotropy differed, and fixing it does not remove the coarse
asphalt grain, so the cause is still open.

- **Mip maps: match.** `mwtool sampler-usage --textures` over all track units: every texture bound to a
  ground shader is BC1 / BC3 with a full chain down to 1×1. The only exceptions are 4×4 placeholders with a
  single level and 7 textures from other families. The chain is stored mip 0 first and tightly packed:
  on 154 BC textures of `TRK_UNIT1`, mip 1's block-endpoint image correlates with mip 0's downsampled 2×2
  at a median of 0.92. Our BC upload path (`convert::image`) passes every level (`mip_level_count`), with
  trilinear filtering. Only non-BC or non-4-aligned textures fall back to a single decoded level, and no
  ground texture does.
- **Sampler states: decoded** (docs/formats.md "SamplerState"; layout from the function that fills the
  `D3D11_SAMPLER_DESC`). Every ground colour, detail and normal map is wrap / 4× anisotropic / bias 0. Ours
  was one sampler for everything: wrap, 8× anisotropic, bias 0. Changed it to 4× (convert.rs). The
  effect is small: in the fixed `road-close` view, the mean horizontal gradient of the mid-distance road
  fell from 9.6 to 8.5, and the near road did not change.
  Still open: slots on trilinear / MipLODBias −0.9 (RoadDepth, Line, Decal normal/spec, SpecColour, some
  Plot/Terrain tiling maps; sharper in the game than ours) and the 37 mirror / clamp states on non-ground
  slots. Both need a per-slot sampler: `TexKey` would carry the state, and the shader would call
  `textureSampleBias`, because a wgpu sampler has no LOD bias.
- **Address modes: match** for the ground (all wrap).
- **`WorldPBR_Horizontal_*`: match.** In the vertex program, TEXCOORD0 and TEXCOORD1 pass through unscaled
  (`o1.xy = v1.xy`, `o1.zw = v2.xy`), and only the wet map gets world XZ × `cb0[6].x`. In the pixel program,
  albedo = Diffuse(uv0) × vertex colour × `MaterialDiffuse` × wet darkening, and AO = Overlay(uv1).y. Our
  `Mode::WorldPbr` does the same: diffuse on uv0 as the base colour, AO on `uv_b` = uv1.
- **Vertex UV formats: match.** Every TEXCOORD stream that the ground shader layouts carry for UVs is
  R16G16Float. Our half decoder (`dxgi::f16_to_f32`) is exact, subnormals included.
- **Detail map strength: matches.** The DriveableSurface G-buffer pixel program computes
  `ColourMap(v1.xy) × DetailMap(v1.zw) × 2 × wet darkening` with no distance fade and no strength
  parameter.
- **Still unexplained:** the high-contrast grain shows right behind the car, where mip 0 is sampled, so it
  comes from the detail texture itself at its authored tiling, not from filtering. Next checks:
  1. **Lead: the Texture header byte at 0x2C.** We read it as `most_detailed_mip`. It is 1 on every
     mipped texture and 0 on the single-level ones (survey of 20000 textures). The upload loop
     (0x00d8ee30) writes every level with `UpdateSubresource`, mip 0 included. The function that creates
     the shader-resource view (0x00d8e670) sets `MostDetailedMip` to this byte only when its flag argument
     is set, and to 0 otherwise. `MipLevels` is −1. Its single caller (0x00409510) passes 1 when the
     global at 0x010eac98 is non-zero and 0x00414a50 returns **false** (corrected 2026-10-06; the live
     value is true, so the top mip is kept). A texture-quality setting.
     With the flag set, the game draws every mipped texture one level down: half the resolution, texels
     twice as large and softer. That fits "scaled wrong" and the harsh grain of ours.
     Next: read 0x010eac98 live (memscan / probe) with the user's settings, and find what 0x00414a50 tests.
     If both hold, start our views at `most_detailed_mip` too (wgpu `TextureViewDescriptor::base_mip_level`,
     or drop level 0 at upload).
  2. Take a Steam F12 capture of the original at the `road-close` viewpoint.

### 2026-10-06 (session 5): ground texture scale audit

Report: "some textures, especially on the road and terrain are still scaled wrong" (no location). Every UV path
of the ground families with the most world triangles (`mwtool shader-usage`) was read from the DXBC
(`mwtool shader-asm`, vertex program outputs, the pixel program's `sample` coordinates, and the `gMaterialCB`
offsets in the reflection header) and compared with `materials.rs` / `layered.wgsl`.

- **DriveableSurface** (+ `_Lightmap`, `_AlphaMask` by name): VS `o1.xy = TEXCOORD0` (raw), `o1.zw = world.xz ×
  DetailMapUvScale.xy` (world = position × gObjectCB world, so instance transforms included), `o5.xy = world.xz
  × gGlobalCB[6].x` (puddle map). PS: ColourMap, Normal, SpecColour, RoadDepth at uv0; DetailMap_Diffuse and
  DetailMap_Normal at the world coordinate. **Ours already matched.**
- **DriveableSurface_DEPRECATED_Line_DirectionV** (1643 meshes): uv0 as above, detail at world xz ×
  DetailMapUvScale, Line_Diffuse and Line_NormalPlusSpec at **raw TEXCOORD2** (`o9.zw`). TEXCOORD4 (float3)
  and TEXCOORD2.x outside (−0.3, 1.3) only feed the retroreflective emission flag, not coordinates. **Matched.**
- **TerrainPBR** (`_Normal_`, `_TangentSpaceNormal_`): gMaterialCB packs `Tiling_Ratio_Noise` (float, offset 0),
  `Tiling_Ratios_X` (float3, offset 4), `Tiling_Ratios_Y` (float3, offset 16); the material stores each as a
  float4, used from .x / .xyz. VS: mask and macro normal at raw uv0; layer i (diffuse + normal) at
  `uv1 × (X[i], Y[i])`; noise at `uv1 × Noise`. **Matched.** Values over TRK_UNIT1 (`mwtool models --verbose`)
  are plausible: X/Y 1–300 per layer, Noise 50–200; no identity / zero / missing values seen.
- **PlotPBR** (`DriveableGrass_…`, `AO_Normal_Specular_…`): layers i at `uv0 × (X[i], Y[i])` (VS), mask at
  `uv0 × maskTilingRatios.xy`, decal at `uv0 × decalTilingRatios.xy` (PS), AO map and lightmap at raw uv1.
  **Matched.**
- **Mismatches found (fixed):** the road variants whose names contain "decal" or "retroreflective" fell to the
  plain material with whichever texture `DIFFUSE_SAMPLERS` picked, on uv0:
  - `DriveableSurface_Decal(_Lightmap)` (343 meshes): the DXBC writes the whole road (colour × detail × 2 as
    above) and lerps `Decal_Diffuse` / `Decal_NormalPlusSpec` sampled at **TEXCOORD2** over it by the decal
    alpha (albedo, normal slope, F0 to `Decal_Roughness_Specular_Fresnel.y`, roughness to .x, Fresnel to .z);
    written opaque. Ours drew Decal_Diffuse on uv0, translucent. Now the line path (Mode::RoadLine, untinted).
  - `DriveableSurface_FloatingDecal(_1Bit)` (65 meshes): the plain road maths (detail at world xz ×
    DetailMapUvScale); `_1Bit` discards where `EdgeAlphaPlusAoMap(uv0).y < 0.5`. Ours drew the **detail map on
    uv0** (wrong scale). Now Mode::Road, with the edge-alpha discard (slot 2, p2.x flag).
  - `DriveableSurface_DEPRECATED_RetroreflectivePaint_Lightmap` (57 meshes): its G-buffer pass never samples
    the Line_* maps (road at uv0 / world xz, AO map and lightmap at uv1). Ours drew Line_Diffuse on uv0. Now
    Mode::Road.
  Screenshot at `--camera=-1150,22,1700,-1160,20.5,1716`: before, faint smeared streaks on the asphalt; after,
  distinct oil / tyre stains at their own places; lane lines unchanged (~0.1–0.15 m).
- **Open:** the WorldPBR `Horizontal_*` ground variants (≈1200 meshes) were not audited; per-mesh vertex
  descriptors were not surveyed (all these shaders declare TEXCOORD0/1/2 as R16G16Float); sampler address
  modes not checked; non-1Bit FloatingDecal blending (still Translucent by name) not read from the DXBC; the
  decal / line normal is added to the road slope in ours where the DXBC lerps it by the overlay alpha. If the
  user's report persists, it is more likely a WorldPBR ground variant or texture filtering than these families.

### 2026-10-06 (session 5): placement audit (groundcover, trees, props)

User report: "foliage etc sometimes is in the wrong place" (no location). New checker `mwtool placement-check
[--bundle <unit>] [--top N] [--groups]` (`crates/mwtool/src/placement.rs`): loads the collision of every track
unit (1,652,030 triangles, 8 m grid), drops each placement onto the surface under its origin (vertical line,
nearest surface in y) and reports per layer, per model, per unit (groundcover) and the largest offsets. Trees
(models with a `Tree_*` shader) are their own layer and are checked at their lowest vertex (model min y times
the matrix's y row), not the origin. `--groups` rates every groundcover group (good = median |dy| < 0.1 m) by
layer / blob / start / quads; `MW_GC_VARIANTS=1` adds alternative decodes of the u16 fractions.

Numbers (dy = placement y - ground y; share of placements that have ground under them):

| layer | n | no ground | median | abs(dy) < 0.1 | < 0.5 | > 2 up | > 2 down |
|---|---|---|---|---|---|---|---|
| prop | 12,290 | 21 | +0.01 | 85.4% | 96.0% | 0.1% | 0.5% |
| compound | 4,301 | 37 | +0.01 | 77.6% | 88.2% | 5.5% | 1.2% |
| dynamic | 4,477 | 65 | +0.01 | 40.2% | 64.7% | 11.9% | 5.1% |
| instance (non-tree, origin) | 23,904 | 1,149 | +0.01 | 28.5% | 45.1% | 17.4% | 14.0% |
| instance tree (lowest vertex) | 9,620 | 234 | -0.64 | 10.2% | 37.9% | 2.4% | 26.5% |
| groundcover q1 / q2 / q3 / q4 | 5.09 M / 2.91 M / 0.29 M / 0.51 M | 37-77% | +0.01 | 42 / 32 / 45 / 26% | 64 / 55 / 61 / 41% | 8-25% | 10-16% |

(Groundcover n counts a collection once per unit that lists it; collections are shared between units.
Backdrop `InstanceList` entries have no collision under their origin: 1,907 of 1,907, as expected for far LODs.)

Results, no decode error found:
- **Groundcover position decode is right.** Per group, 50.2% of 11,579 groups (55% of instances) sit within
  0.1 m of collision with the current decode; every alternative is far worse (x/z swapped 8.8%, x flipped 9.4%,
  z flipped 9.7%, x and z flipped 4.7%, y flipped 3.5%). Good groups are spread evenly over blob index
  (43-57%), start = 0 or not (53 / 50%) and group order, so the group -> blob/start mapping is not the problem.
  Layer 0 (small tufts) 61% good, layers 1-2 (bushes, 2-4 crossed quads) 28%: they grow on terrain that
  collision covers coarsely or not at all. The worst unit (TRK_UNIT165, median +5.1 m) was checked visually at
  (1562, 22.5, 1456): the tufts stand on a raised lawn planter that has render geometry but no collision.
- **Groundcover anchor matches the shader.** `Groundcover` vs_4_0: vertex = instance position + width x (u - 0.5)
  along the camera-perpendicular horizontal + height x v up (`mad r0.xyz, r1.w, l(0,1,0,0), r0.xyz` with
  r1.w = v4.y x size x `cb1[0].x`), so the sprite stands on its base, as the loader draws it. The billboard is
  cylindrical: the quad's horizontal axis is perpendicular to (position - camera).xz, not the stored yaw (our
  static crossed quads use yaw; open). `cb1[0].x` is still unknown; the dump registers the cbuffer
  `GroundcoverConstants` (slot 0x7F, 16 bytes) and `GroundcoverTextureRegions` (0x80, 16 x 16 bytes) in
  `FUN_008c34b0`; the writer of slot 0x7F was not traced.
- **Trees are planted, not floating**: per-model medians of the lowest vertex are -0.1 to -0.9 m (trunks sunk
  into the ground), 2.4% more than 2 m up. Outliers: `a97b9688` (1,545 placements, lowest vertex median -5.8 m,
  origin -2.0 m: the model reaches 3.8 m below its origin; looks like a deliberate deep root, not checked
  visually) and `f86cfc31` (12, +2.2 m, probably on planters without collision).
- **Tree vertex programs do not move vertices** beyond wind sway (`Tree_Translucent_1Bit_Doublesided`: xz +=
  sin(time x speed + phase) x amplitude x weight, phase / weight from a vertex attribute), so the leaf cards are
  meant to be where the mesh puts them.
- Props, compounds: placed on the ground. Dynamics with large offsets are a handful of models (`34280415`
  +43 m x 33, `cdf4b833` +20 m x 40, `42d010d9` -18 m x 23); not identified (no debug names), probably objects
  on roofs / under bridges where the nearest collision is another level.
- Screenshot `lamp-tree` (-1112,24,1640 -> -1130,20,1630): lamp, trees, hedges and kerb stand correctly. In the
  same shot, dark groundcover specks hang against the sky in front of the cliff on the left.

Most likely cause of the report (not fixed, open): **groundcover is drawn at every distance.** The game presumably
draws groundcover only near the camera (unverified; open point "fade distance"), but we draw all collections everywhere, so tufts of
far collections (e.g. `e557779c`, shared by units 36-57, instances up to y = 422 over the mountains) are drawn
over far terrain that we render with a different / coarser backdrop LOD or not at all; they appear as floating
specks. Next step: find the fade distance (the GroundcoverJob, string at 0x1002028, culls groups per frame; or a
live read) and cull groups beyond it in the loader / renderer. Also open: camera-facing billboards instead of
yaw-oriented crossed quads.

### 2026-10-06 (session 5): tyres: longitudinal constraint and the launch

- Read `FUN_009c6f00` (the per-wheel tyre pass called from `FUN_009c75b0`), `FUN_009bc340`, `FUN_009bc0c0`,
  `FUN_009bbff0` and the clutch's torque line in `FUN_009d7ac0` (decomp.md "Tyres", step 3). `FUN_009bc340` is
  bookkeeping (a copy of the solve state and two contact anchors), not a force. `FUN_009bc0c0` is the
  longitudinal constraint: a brake hold first (the brake budget left after the spin step, / r, against the
  chassis alone, the spin untouched), then the slip that remains with the coupled mass 1/m + r²/I, both within
  the one cap; only the coupled part turns the wheel. The drive torque never enters the tyre pass: the clutch
  puts it into the driveline before. κ for the caps is taken from the spin after the clutch and the brake step.
- Ours already matched the coupled part, the order (brake step → caps → longitudinal → lateral) and the κ
  definition. It lacked the brake hold: a wheel the brake had stopped was spun back up by the coupled pass and
  stopped again next tick, so braking moved through the wheel's inertia and the rears slipped. Ported
  (drive.rs); the old "locked wheel stays at 0" special case now falls out of it.
- `ref/xkr_launch` replay (XKR) with the current model, before → after: RMS speed 15.2 → 15.1 mph, yaw 7.8 →
  7.8 °/s; during the braking the rear κ goes −0.14 / −0.26 → −0.03 / −0.02 (original +0.01 / −0.03) and the
  front κ −0.07 / −0.02 → −0.11 / −0.15 (original −0.13 / −0.54). 911 tests unchanged: 0–100 km/h 3.72 s,
  100–0 km/h 38.5 m / 2.63 s, skidpad 2.57 g.
- The "revs hang at ~6400 after the 1→2 shift, κ ≈ 0.6 in 2nd" no longer reproduces with the merged model: ours
  now reads κ 0.42 / 0.33 in 1st, 0.49 just after the shift and 0 from 3.5 s (original 0.10–0.12 in 1st, 0.39
  after the shift, 0 from 3.75 s). What is left: ours spins more in 1st and reaches the limiter (6750 against
  6120 rpm), and it leaves ~0.3 s early (launch timing, 3612 against 1568 rpm at 1.5 s).
- Open: the clutch torque is multiplied by a `.bss` vector at 0x011873e0 (one lane per entry; no writer in the
  dump), taken as 1. Below 1 the clutch is softer, the engine runs further above the wheels and less torque
  reaches them in 1st: the first live read to make. The relative velocity in `FUN_009bc0c0` also subtracts
  a `.bss` scalar 0x01186b74 × an input lane from its y component (probably gravity × dt; not modelled).
- The +0x210 blend-input scale: two leads, not followed to the end (decomp.md "Tyres" → Unknown): the
  constraint builder `FUN_009c24b0` zeroes it, and `FUN_009a56b0` (from `FUN_0095b100`) writes a vector there
  on some object with a flag at +0x4aa. If only the builder writes it, the blends get 0 (pure curves), not 1.

### 2026-10-06 (session 5): drivetrain: standstill, downshift gate, launch delay

- Standstill hold found and ported (decomp.md "Drivetrain and brakes"): `FUN_009d6170` builds the input at
  vehicle +0x1aa8 (brake, or throttle in a forward gear, below a signed speed threshold), `FUN_009b1180` turns it
  into a body velocity change of `99407bbc` m/s² against the direction of travel. The threshold (`.bss`
  0x01186e60) is unread; any positive value would hold the XKR's launch (6 m/s² < 15), which the reference does
  not show, so it is set to 0 pending a live read.
- Downshift gate: vehicle +0x1ed0 is the time on the ground (`FUN_009ae840`); ported with an assumed limit 0.
- Launch delay: the 0.45 s idle wait matches the forced-brake mask (vehicle +0x1c30) of the control block, whose
  setter is not found; the standstill timers serve reverse selection. Dead end: the gearbox states 0/1 and
  `FUN_009262a0` were not reached this session.

### 2026-10-06 (session 5): post-processing: composite constants, bloom, vignette, tint

Traced from the dump and the exe programs (details in decomp.md "Post-processing and colour grade"). No
rendering change this session: every stage still missing has at least one value that could not be traced
without the running game, and the brief was to leave a stage out rather than guess it.

- **The constant names are not in the code.** `kHdrToneMapping`, `kColorAddWeight`, `kVignette…`,
  `ColourCubeScalesOffsets`, `BloomColour` only occur inside the DXBC blobs (RDEF); no function references them.
  `GBufferComposite`'s pixel constants are `GenericVector4_A` / `_B` / `_D` (gObjectCB), set through the global
  constant table (`FUN_00a5d660(index)` returns the slot): A = 0x9c, B = 0x9d, C = 0x9e, D = 0x9f.
- **B and C** (`FUN_0089d5c0`, branch for material type 0x36): when bloom is on (a flag at +0x1518 of the view
  settings object and the renderer's +0x9836, both runtime), B = (Bloom.Colour.rgb, Bloom `ff564907`) and C =
  (1 / Bloom `76826c13`, Bloom `da328699`, 0, 0), with C = (1, 1) when `76826c13` ≤ 0.001 (`0x00f8bb34`).
  When bloom is off, B = (1, 1, 1, 0) (`.rdata 0x0100cc10`) and C = (1, 1, 0, 0) (`0x0100caf0`): B.w = 0
  removes the bloom term, so our composite without bloom equals the game's with bloom off.
- **PostFxKeyframe.Bloom** is unpacked by `FUN_008e51b0` into 13 floats (the order the renderer keeps them at
  +0xac74…): Colour.xyz, `da328699` (DarkBloomWeight), `ff564907` (Threshold?), `b303e3e6`, `41e5a069`,
  `3efef892` (Luminance), `76826c13` (DarkBloomWhitePoint), `c9f605e2`, `a3d336c1`, `cf7d9668`, 0. Field
  names are the cracked ones `mwtool genesys` prints (unconfirmed). `29c5d30d` (Saturation) is not read.
  `FUN_008e9610` blends the current and next keyframe's set (lerp by the transition) and then towards the
  state by its weight; `FUN_008eccb0` runs it for each PostFX state whose flag at +0xf4 is set.
  Bloom set-up `FUN_008eaaf0` receives `c9f605e2`, `a3d336c1`, `cf7d9668` (default keyframe `…78d9`: 1, 0, 0)
  plus three vignette vectors; that they become `kColorAddWeight` is likely but not followed to the end.
- **Bloom programs** (exe): downsample `0x0100ffa0` = 4 RGBM taps × 32, × 1/128 (the average), re-encoded
  as RGBM with M = max channel (≥ 1e-6, ≤ 1) rounded up to 1/255: RGBM covers 0…32. Blur VS `0x010109c8`
  places taps at 0, ±1.407333, ±3.294215 × `kScale` along one axis; blur PS `0x01010d38` weights them 6.533238
  (centre), 9.728171, 3.005209 (sum 32, i.e. normalised for the RGBM range: a 5-tap linear-sampled Gaussian)
  and divides by the 1×1 tonemap texture's w (the exposure) back to HDR units.
- **Vignette A and D**: A = renderer +0xa990, D = +0xa980, each multiplied per component by a vec4 at
  +0x15d0 of the view settings (set by `FUN_008e3e30`, an override setter). Nothing in the dump writes +0xa980 /
  +0xa990 with a literal offset; the writer was not found. The keyframe data (`Vignette.5e9c4fa6` = inner
  (1, 1, 1, 1), `5f000214` = outer colour, (0.04, 0.18, 0.27) in `…78d9`) suggest D = inner and A = outer −
  inner, but that is not traced, so the vignette stays out.
- **TintBlend weights** (`FUN_008c9e40`, the frame's post set-up, before `FUN_008e0930`): seven (resource,
  offset) cube slots from the frame state at +0x2c, +0x34, +0x3c, +0x44, +0x4c, +0x64, +0x6c with weights
  +0x54, +0x58, +0x5c, +0x60, +0x74, +0x78 (the last two replaced by +0x1560 / +0x1564 and the first two by
  +0x1568 / +0x156c when the view's +0x155e override is set). A missing slot falls back to the base cube
  (+0x2c). If the six weights sum above 1 they are scaled to sum 1; the base cube gets 1 − sum. The PostFX
  state's two `ColourCubeSettings` cubes and their [0, 1]-clamped weights (`FUN_008ecf80` → state +0x12e8…
  +0x12fc) are very likely the last two slots. Where the EnvironmentKeyframe's `ColourCube` enters (one of
  the base / first four slots) was not found.
- **EnvironmentKeyframe.Camera** carries the same two field hashes as the bloom (`da328699` at +68,
  `76826c13` at +72) next to the colour vector `96a4b35b` at +16: an environment bloom override, reader not
  found.

Open, needs the running game (`.bss` / heap reads, not possible this session): the bloom-enabled flag
(view settings +0x1518, renderer +0x9836); renderer +0xa980 / +0xa990 (vignette D / A) and +0xac74…+0xaca4
(the bloom set) on a frame; the seven TintBlend slots and weights in the frame state (+0x2c…+0x78), which
answers which slot holds the environment cube; which PostFxKeyframe is active in free roam (`…78d9` vs
`…78dc`). With those, bloom (traced kernel above), the vignette and the cube blend can be ported without
model decisions; the bloom level weights (`kColorAddWeight.z/.w`) also need a live read of the combine
program's constant buffer.

Note for the yellow-tint question: the default keyframe's bloom colour is blue (0.29, 0.42, 0.60) and its
vignette outer colour is a dark blue; both push the image away from yellow, so their absence is a plausible
part of the remaining warm cast (not measured).

### 2026-10-06 (session 5): sky, sun path and shadow cascades

- **The near-black upper sky** (noon, camera looking up) was the cloud layer decoded as sRGB (next item): with
  that fixed the patches read teal-blue. Still open: the clear sky's gradient (top stop ≈ (0, 0.11, 0.21)) runs
  at scale 1 under an exposure set by the noon key light (~11 units). The game scales the gradient by `BaseCloudColour.w` = s²
  (`FUN_008b27e0`; decomp.md "Sky, sun and shadows"), s a runtime environment-state float whose keyframe field
  is not identified, so the scale stays 1 (open). Dead end: fitting `SkyReflectionAverageColour` as the
  hemisphere average of gradient × s² + glow for the unnamed Sky fields `e570557e` (1.80 in the first
  keyframe), `d2f5bcb1` (1.07), `9f2051a4` and `BaseCloudIntensity` gives per-channel ratios 0.4–1.1 with a
  spread of 0.3–0.9 over 175 keyframes: none of them is the scale on its own (the average probably includes
  the clouds).
- **Clouds were decoded as sRGB**: the cloud textures (`…3f34`, `…3f3c`, …) are `BC3_UNORM`, and NewSky
  multiplies the sample straight into its HDR sum. Ours raised the RGB to 2.2, darkening the cloud colour
  ~2.5× (0.65 → 0.39 in the thick parts). Fixed: the texels are used as stored.
- **Dome mapping matches**: `FUN_008bca20` builds the dome (`Sky vertex buffer`, 0x7e vertices) as unit
  vectors (radius `DAT_00f8a820` = 1.0), so the gradient coordinate |POSITION.y| is our |dir.y|. Its
  `Sky texcoord buffer` holds x = the distance along the vertex direction to a sphere (centre and radius at
  `DAT_010cf400..0c`, .bss: the cloud shell, used by the horizon fade) and y, z = the cloud layers' planar
  coordinates (built from angles of the dome grid: constants π, 2π, π/2 at `0x00ffc024..2c`). Ours maps the
  cloud texture by azimuth relative to the sun and by elevation: not the game's (open, needs the grid maths).
- **Key light**: `KeyLightDirection` (constant slot 0xd) is copied from renderer +0x2920 in `FUN_008ad7d0`;
  the writer of +0x2920 (a struct copy, no direct store in the dump) and the reader of
  `EnvironmentSettings.SunMoonPath` (offset 0xf0, fields at 0x100–0x128) were not found. The provisional
  `SunMoonPath::key_light_direction` model is unchanged (open).
- **Shadow cascades**: `FUN_004a40a0` loads `ShadowMap_Constants{,2,3,4}` (slots 0x1c–0x1f) from environment
  state +0xf00..0xf3f; those are packed by `FUN_008a4910` (called from `FUN_008c8b00` with renderer +0x16f10, the
  shadow object; it reads its matrices at +0x90.., +0x220.., +0x3b0..). The fit that fills them (split
  distances, cascade extents) is not located yet, so our splits stay provisional (open).

### 2026-10-06 (session 5): input bindings

- Found from the action-name strings (`STEERLEFT` …, `.rdata` 0x00f9fd48 on): their pointer table at `.data`
  0x010c3540 (96 actions), the default builders `FUN_00482250` (XInput pad), `FUN_00483470` (keyboard, Windows
  virtual-key codes, not DirectInput scan codes) and `FUN_004843b0` (mouse), all calling `FUN_00ac72a0`, and
  the ini reader `FUN_0047cf90`. Tables and the mapping are in decomp.md "Input and control bindings".
- The Settings-screen reading is confirmed (A / Z / arrows / Space; RT / LT / left stick / X; Esc and Start
  pause; PgUp / PgDn tabs; R Ctrl = AUTOLOG_SETTINGS). New: nitrous = Left Shift / A, camera = C / LB, horn =
  W / B, look around = Delete, Page Down, Home, End / right stick, change vehicle = Q / Back, Easydrive = Num 4 /
  6 / 8 / 2 / d-pad (min 0.8, so only a near-full press counts). LOOK_BACK and LOOK_AROUND exist as actions but
  neither table binds them: how the original looks back by default (right stick down = ROTATECAMERADOWN?) is
  open; trace the camera code that reads action 10 / 37.
- Mapping: per binding `max(0, (raw − min) / (max − min))` (`FUN_00ac6850`); pad sticks per axis with dead zone
  0.2 rescaled, triggers 0.1 rescaled, `|v| / 32768`, `byte / 255` (`FUN_004119c0`). Keys are 0 / 1: no ramp
  before the controls are handed to `FUN_009aacc0` was seen on this path (the ramps, if any, are downstream:
  decomp.md "Update loop").
- Runtime: `crates/mostrusted/src/input.rs` holds the driving defaults and the mapping (unit tests); drive.rs,
  camera.rs and menu.rs read through it. Ours: steer arrows / left stick, accelerate A / RT, brake Z / LT,
  handbrake Space / X, camera C / LB, look around Delete / PgDn / Home / End / right stick (End or the stick
  fully back looks back, our rule), car menu also on Esc / Start / Back (PAUSE / CHANGE_VEHICLE). W / S / B and
  pad A / Y / R3 lost their old driving roles. No nitrous in the runtime yet.
- Open: the dead zones are `.rdata` initial values; a live read of the pad object (+0x140 / +0x144) would show
  whether the options change them. Whether the saved ini replaces or adds to the defaults.

### 2026-10-06 (session 5): Easydrive and the front end

- **Where the menu lives**: not in standalone GeneSys objects (`mwtool genesys all --type
  Widget_EasyDriveTierDefinition` finds none) but inline in the driving screens' `Screen2` objects
  (`UI/SCREENS2/371621.BNDL` free roam with the full tree; 1019407, 1019814, 1560359, 2173025, 2210073, 371670,
  371678, 371720, 371725, 604153 carry variants). The screen's widget groups (`WidgetGroup`, children in field
  `c7413006`) are the unit: one group holds the menu widget (`Widget_ShowLayouts` "EasyDrive": 3 layouts
  `01000000001b0f44`, `01000000001b8d10`, `01000000001a24e2`, scripts `Menu_Fast`, `EasyDrive_ScreenScript`,
  `EasyDriveSpeech`, `ExitOnIdleScript`) plus every item type (`Widget_EasyDriveTypeDefinition`); every other
  group with a `Widget_EasyDriveTierDefinition` is one tier with its item lists. Loader `mw_assets::easydrive`,
  listing `mwtool ui easydrive [screen]`. Layout in formats.md "Easydrive".
- **Tree** (labels from the English table): root `TO_BASE` (no title): RETRY LAST EVENT (`TO_RTY_CNF`, only
  with `Mode.RetryLastEventEnabled`), AUTOLOG RECOMMENDS (`TO_AR`), RACES (`TO_FMN`), EXCLUSIVE RACES (VITA only),
  CUSTOMIZE CAR (`TO_MOD`), CHANGE CAR (`TO_JACK`), MOST WANTED CARS (`TO_BLACKLIST`), MULTIPLAYER (`TO_MP`),
  STORE. Change car = MANUFACTURERS (`JSMANUFACT` source) -> MODELS (`TO_JS_MOD`, `JSMODEL`) -> LOCATIONS
  (`TO_JS_LOC`, jack spots) -> OPTIONS. Customise = MOD CLASS (+ LIVERIES -> `TO_PAINT`, `PAINTITEM`) -> MOD TYPE
  -> MOD STANDARD. Confirm tiers (retry, store, leave single player) hold NO / YES and the speech variants.
- **Behaviour from the data**: link items select through `LINKITEM`'s callback `ExitTierVia(Token)` (enabled
  by `Unlocked`), so a link token is the HSM transition naming the next tier. Generated tiers name their code
  callbacks: `EZStateJack::SelectManufacturer(Id)`, `EZStateJack::SelectModel`, `JACKSPOT::JackIt(Placement)`,
  `SelectPaintPackFromEasyDrive(PaintId)`, `UpdatePendingRollout_EasyDrive(UpgradeEnum)`,
  `SetVehicleAndRolloutFromSaveGameByVehicleIDString(Id)`. Inputs: the menu widget's user actions bind
  `EasyUp`/`easy_up` -> `EasyDrive::EasyDriveOnPrevious`, `EasyDown`/`easy_down` -> `OnNext`, `EasyLeft` ->
  `GoBackTier`, `EasyRight` -> `SelectCurrentItem` (only if `NumItemsShown >= 1`), `Hack_EasyDriveQuit`/`cancel`
  -> `QuickClose`; the HackEasyDrive widget opens it with `Hack_EasyRight`/`easy_right`. Idle close:
  `ExitOnIdleScript` fires `TIMEOUT` on `ScreenTimer` "EasyDriveTimer", `Delay: 1000` (unit not checked).
- **Code**: the strings (`ExitTierVia`, `EasyDriveOnNext`, `easy_right`, `SelectPaintPackFromEasyDrive` ...)
  are in `strings.csv` but `callers` finds no reference to any of them (registered through tables the dump has no
  xrefs for); no function found yet. Open: which `ConstantItems` / `ConstantItems2` adapter entries come from
  which list (the adapters are code), how `Signals.GetIntValue` sources are filled, the PC keys behind the
  `easy_*` mappings, the layouts' look.
- **Runtime first version** (`crates/mostrusted/src/easydrive.rs`, `--show-easydrive`): tree, titles and labels
  from the data; link items open their tier; the CHANGE CAR tier lists the catalogue's makes, then models, and
  drives the car via `GarageAction::Drive` (skipping the jack-spot location step); everything else is shown and
  inert. **Model decisions**: panel place / colours / sizes (fonts are the game's body and title fonts, the
  layouts are not read yet), 8 rows shown, keyboard `E` to open and Esc / gamepad B for `cancel` (navigation uses the original's EASY_* bindings, Num 4 / 6 /
  8 / 2 and the d-pad, since the merge with the input work; the arrows steer),
  all list items of a tier shown regardless of their source's condition (so confirm tiers show four rows and the
  root shows retry / VITA / store), paint not wired.

### 2026-10-06 (session 5): free-roam rules

- `TriggerData` (one instance, `HAWAII/TRIGGERS.BNDL`, 437680 bytes) decoded: header with three record arrays
  (6404 boxes, 53 spheres, 2585 points) found by reading `FUN_0053c2f0`, which indexes the three as one list
  (0x30 / 0x18 / 0x20 strides). Not Burnout Paradise's layout (no landmarks / generic-region split, no
  start position). Parser `mw_formats::TriggerData`, listing `mwtool freeroam`.
- Collectable kind codes come from `FUN_006e4920` (telemetry names). Base-game counts: 216 Billboard,
  152 SecGate, 74 SpeedCam, 183 JackspotLocation, 185 JackspotCar, 22 BodyShop. All are distinct places (no two
  of a kind within 5 m except one gate pair); every jack spot location has a car box beside it (2 extra cars).
  Heights (20–90 m) are road / car-park heights for those parts of the map; a check against the road
  collision is not done.
- Counts vs the game's totals: no "x of N" found in the data yet. `CollectablesConfig` (GLOBALCONFIG) has five
  id lists with counts 4, 35, 101, 35, 35 at its u16 fields 0x68..0x70; the 35/101 lists hold jack spot location
  and car ids (checked against TriggerData), probably per content pack; the 4-item list is not in TriggerData.
- Speed camera rule from `FUN_006ed0a0`: counts at |speed| ≥ `CollectablesConfig.d89eeda4` = 50 mph, records
  `(int)(100 × mph)`. Jack spots need the script flag `JackSpots.IsSwapSpotVehicleSpawned[n]` (`FUN_006e44f0`).
  Details in decomp.md "Free-roam rules".
- Dead ends: the strings "SpeedCam", "SecGate" … are referenced only from code (`FUN_006e4920`), no data
  tables point at them; `FUN_006f3f30(n)` is a show-once hint, not scoring; `GameLogicConfig.FirstSeenScores`
  holds ids, not point values.
- Open: (1) the spatial index (header +0x1c: 45 (ptr, count) pairs then 39 more at 0x5fa78, u32 trigger indices;
  +0x20/+0x24 = 45 × 39 cells, +0x28 origin (-4794.6, -4303.2), +0x30 cell 250 × 250 m, all inferred); (2) box
  `size`: full or half extents (full fits the objects: 10.5 m billboards, 19–35 m road-wide camera boxes);
  (3) record flags byte (+0x12): 2 on speed cams, jack spot locations, body shops, 0 on gates / billboards /
  jack spot cars; (4) the other ~55 kind codes (kind 4 points = 2518, probably spawn / roaming locations;
  kind 29 spheres = 41); (5) the overlap test that fires a trigger and the smash requirement for gates and
  billboards; (6) Speed Points per event; (7) `.bss` values to read live: `0x0118a060` (already open).

### 2026-10-06 (session 5): traffic

- `TrafficLaneData` (0x701): a single instance, `HAWAII/TRAFFIC.BNDL` (2.3 MB, 57 imports), next to 35
  `TrafficFlow`, 23 `TrafficFlowType`, 11 `TrafficVehicle` and one `TrafficVehicleTraits` GeneSys objects.
  Decoded and parsed (`mw_formats::TrafficLaneData`, `mwtool traffic [--png]`): 294 hull cells (~400 m quads,
  130 with lanes), 6758 lane sections, 52 607 rungs, 1005 spans, 5512 neighbour links, 36 junctions, 189 stop
  lines, 392 static vehicles, 374 km of lanes. Ground truth: the lane map drawn top-down is Fairhaven's road
  network (freeway loop, downtown grid, mountain roads), and 99.6 % of each hull's rungs lie in its cell.
- Verified across all hulls: record sizes (section 0x30, rung 0x20, span 12, neighbour 4, static vehicle 0x50,
  junction 0x150), every section's rung range within its hull, neighbour ranges within the list, forward /
  backward hull links below the hull count, section length = last cumulative rung length (cumulative lengths
  restart per section), span `f32` pairs are value and reciprocal. Section byte 0x24 = `TrafficFlow` index:
  confirmed in the code (`FUN_005f4d10` reads the flow's `Speed` through it, scaled by 0.44704 → mph).
- Dead end: hull header byte 0x04 read as u16 gave impossible neighbour counts; it is u8 neighbours + u8 static
  vehicles (static-record region sizes are multiples of 0x50 matching byte 0x05).
- Open (format): junction record (0x150: an id, 150 / 20 pairs, light masks), stop line (2 bytes, `00 01`),
  static vehicle record (matrix + 16 bytes?), section byte 0x25 (0..21, probably a `TrafficFlowType` index),
  span `u16` at +8 (a hull index?), hull header u16 0x08 / 0x0A (equal but once: traffic light range?), header
  0x2C-0x34 (three pointers at the flow import list), the vehicle-type records at 0x38 / 0x3C (0.65, 1200).
- Open (code): spawn / despawn distances, car count, per-flow density, hour-of-day density use
  (`TrafficSystemConfig` array), junction behaviour, reactions to the player, switch to a physical car.
  Leads: readers of `world+0x697fc`, `FUN_005e9350`, the "Crashing Traffic" net message in `00610000.c`.
  `.bss` value needed from a live read: GeneSys object at `DAT_0110f940` field 0x48 (scales
  `TrafficVehicle` field 0x10 in `FUN_005f4d10`).

### 2026-10-06: no Burnout Paradise bike code survives (dead end)

- Dump strings (23,963): no `bike`, `motorcycle`, `rider`, `wheelie` or `stoppie`. The only vehicle-type enum
  strings are `VEHICLE_TYPE_RACER` / `VEHICLE_TYPE_COP`, which sit among the Blaze `REQUEST_TYPE_*` matchmaking
  strings. They are an Autolog racer-or-cop flag, not a chassis type.
- GeneSys (`mwtool genesys-types`, 12,410 lines): no bike, rider or lean type. `BodyMovementBehaviour`
  (`Take_off_behaviour`, `Angle_control`) is the car body's movement, the source of the anti-roll bar values.
  It is not a helicopter or bike type.
- Install: no bike-named files. `VEHICLES/` holds only `VEH_*` car bundles and the shared lists.
- `VehicleInfo.Axle` stores a list of wheel handles plus a u16 count (2 on every car checked), so the data
  format could describe one wheel per axle. The handling code assumes left/right pairs, though: per-axle
  differentials (`FUN_009d8140`) and per-axle anti-roll bars (decomp.md).
- Caveat: engine functions are unnamed and have no RTTI, so a nameless dead function cannot be ruled out. With no
  strings and no data types left, though, nothing of the bike code is usable.

### 2026-10-06: the drivetrain from the code; the "2nd-gear clutch bug" is the tyres

- **Read from the dump** (details and addresses in decomp.md "Drivetrain and brakes"): the gearbox is a
  14-state machine (`FUN_009da340`); the engine (`FUN_009d9960`) and clutch (`FUN_009d7ac0`) run after it each
  tick. Corrections to our model: the shift phases are declutch 10 % / **neutral 50 %** / **clutch-in 40 %**
  (we had 40 / 50), with the gear changing at the end of neutral; the ±50 rpm random offset only feeds the
  launch clutch, not the upshift point; downshifts blip the engine for the whole shift; the rpm downshift rule
  is not gated by the throttle (its gate, vehicle +0x1ed0 against `.bss` 0x01187580, is unidentified; the
  original downshifts 3 → 2 → 1 while braking with no throttle); in gear the clutch opens below 20 mph with
  neither pedal, or below idle. The declutched engine still takes the throttle (torque = B + T × (full − B),
  B the resistance faded by the governor); ours ignored it, which is why our idle sat at a flat 1000 / 969.
  The throttle is peak-held (`c6deb7a6` per tick) and lagged (`36676b9c` per tick), bypassed during upshifts;
  a value-noise torque (rpm / 120 lattice points per second, amplitude from a second `Power_curve` three-point
  curve, 0 at idle on the XKR) reaches the rpm. Clutch-in brings the throttle back as pedal × progress^k.
- **The idle**: with the hold / lag time base 60 the governor's limit cycle gives 1003–1031 rpm, mean 1016,
  alternating tick to tick, against the original's 1002–1033, mean ~1015 (`ref/xkr_launch`); 1 gives 987–1029,
  an instant lag 968–1091. The `.bss` vector 0x01187250 that holds it is not read yet (handling.md "Model
  decisions").
- **The 1→2 shift** in `ref/xkr_launch`, tick by tick: one declutch tick (clutch 1 → 0), five neutral ticks,
  gear 2 on the fifth, then 0.25 / 0.5 / 0.75 / 1: T ≈ 0.167 s from the XKR's 0.15 s × (1 + 0.15 × r). Ours now
  does the same.
- **The original's clutch never locks under load.** Its engine runs ×1.9 / ×1.3 / ×1.14 above the driven
  wheels in 1st / 2nd / 3rd at full throttle (rpm against w2_omega × ratio × final drive) while the rear tyres
  sit at κ ≈ 0: the clutch is one speed-matching impulse per tick before the tyre solve, and the tyres pull the
  wheels back to road speed, so it slips with T ≈ I_eff × Δω / dt (≈ 250 / ratio² N m per rad/s, consistent
  across the three gears). Our clutch is the same impulse; the difference after the shift is the tyres: ours
  spin the rear tyres (κ 0.6 in 2nd, κ 0.2–0.3 with `MW_LONG_GRIP=1`) instead of holding them, so the engine
  sits on the tyres' slip instead of the clutch's. With `MW_LONG_GRIP=1` the 2nd-gear rpm takes the original's
  shape (4600 → 4430 → 6460) but the car is 0.5 s quicker to 60 mph: the longitudinal tyre cap at small slip,
  not the drivetrain, is the gap (handed on to the tyre work).
- **Replay** (`--replay ref/xkr_launch`, before → after): speed RMS over the launch (1–6 s) 1.86 → 0.83 mph;
  speed at 1 / 1.5 / 2 / 3 s after the throttle 4.8 / 13.1 / 20.4 / 36.3 → 6.6 / 14.8 / 21.3 / 38.0 mph
  (original 6.6 / 15.4 / 22.4 / 39.3); 60 mph after 4.55 → 4.58 s (original 4.63); 1→2 at 2.02 → 1.92 s
  (original 1.87); 2nd-gear rpm 6250–6680 → 5770–6720 (original 4750–6750; RMS 1105 → 636); idle 1000 flat →
  1003–1031. Worse: the first second, where the replay holds the handbrake (the original was parked in gear
  index 0 with the rear wheels locked): our handbrake takes the driven wheels off the clutch (`drive.rs`), so the
  free engine now revs to the limiter on the throttle (the original's rose at +109 rpm per tick with the clutch
  closed, i.e. loaded by ~150–190 N m, which fits the clutch formula with the driveline still connected, best
  with the front axle: `FUN_009d5520` / `FUN_009d7ac0` force the front share to 1 when vehicle vfunc +0x44 is
  true). Left for the handbrake work; 1st-gear rpm RMS 568 → 2493 because of it.
- **911 tests**: `--test accel` 0–60 mph 3.72 → 3.82 s, 0–100 km/h 3.87 → 3.98 s, 0–200 km/h 11.12 → 11.40 s,
  quarter mile 11.68 → 11.82 s, top 307.4 → 307.6 km/h (the clutch-in throttle ramp and the throttle lag at
  launch); `--test engine` idles 1002–1046 rpm (was 1000), revs and shifts as before, and now downshifts 3 → 2
  on the brake. `--test brake` 100–0 km/h 40.1 → 42.3 m (downshift blips while braking).
- Open: the launch reaches full torque in its first tick in the original (+140 rpm per tick = the full-load
  torque at 1028–1312 rpm) while the lag needs ~10 ticks; the 0.45 s the original waits between throttle and
  revs (parked, gear index 0); the constants 0x01187250, 0x011874d0, 0x011874a0, 0x01187580 and vehicle
  +0x1ed0. Procedure: with the game in free roam, `mwtool memscan`, `dump NFS13.exe+0xd87250` (and +0xd874d0,
  +0xd874a0, +0xd87580); add `engine_lag f32 … 0x184 0x1d14` and `gearbox_state i32 … 0x184 0x1bd8` and
  `x1ed0 f32 … 0x184 0x1ed0` to `handling.watch` and re-capture the launch and a braking run.

### 2026-10-06: the grip caps traced to the end; N is the static load; surfaces; no calibrated grip scale

- **No hidden factor.** `FUN_009c5da0`'s caps are μ × N × camber × (surface grip × axle `*_scale` × the per-wheel
  grip multipliers); `FUN_00905910` builds every input per wheel each tick. The multipliers (wheel +0x140) are 1
  outside a timed gameplay effect (`FUN_00900120`, lanes 0 / 1) and a post-event ramp (`FUN_009b2bb0`, lanes 2 / 3);
  the axle scales come from `Tyres` (`FUN_009d4ce0`); the surface grip is `Surface_Grip` blended by the car's
  `VehicleSurfaceProfile` link (`FUN_009cec40`), 1 on asphalt for every car. `LAT_GRIP` / `LONG_GRIP` (0.4 /
  0.7, fitted to real-world 911 figures) are gone. Details in decomp.md "Tyres".
- **N is not the spring force**: static corner load + ½ × the axle's downforce share (Tyre `5f032abb` would blend
  in the strut force; 0 on all 197 cars). `ref/xkr_launch` shows it: `w*_load_in` is 4576 / 4596 / 4071 / 4051 N
  throughout (= 1762 kg × g by the centre of mass), `w*_N` adds |`Lift_coefficient`| × |v_fwd| |v| (1.00 × v² in
  all) split 42.1 % front at body z `b821de16` and 47.6 % at `d21c15d0` with the brake ramp at 1. The body's lift
  force keeps the extra `ca386558` factor; the tyres do not. The lateral curve's exponent is × static / N.
- **The reference run agrees with the literal scale**: braking with only the fronts biting (κ ≈ −0.2) the XKR
  slows at 0.95–1.0 g = 1.74 × front static load / weight; sliding at 20–30° with locked fronts, a steady 1.95 g.
- **Its κ field is the post-solve slip**: the original's rear tyres read κ ≈ 0 while pulling 0.6 g in 2nd
  (the clutch slips instead); ours, read after the tyre pass, does the same (0.40 vs 0.42 at the 1→2 shift, 0
  from 3.5 s); before the pass ours reads 0.2–0.6. Telemetry now records the post-pass slip ratio. Tested with the
  drivetrain branch (04533ea) cherry-picked: 2nd-gear revs 4913 / 5278 / 5975 vs the original's 4831 / 5101 /
  5776 at 3.5 / 4 / 4.5 s; speed ~2 mph ahead; 60 mph 0.26 s early; 1st gear still spins more than the original
  (post-pass κ 0.7 vs 0.1 at 2 s).
- **Solver order and mass** (`FUN_009289f0` → `FUN_009c75b0`): one pass per tick per wheel, wheel after wheel,
  after the contact iterations; brake budget on the wheel's spin first, then the tyre; the force goes into the
  **chassis** (the 0x100-byte body records; the wheels are compact 0x80-byte spin records) with the chassis's
  effective mass at the contact (`FUN_009ca610`). Ported: brake-then-one-pass (the 6 sequential passes are gone),
  impulses applied wheel by wheel so each sees the velocity the previous left, chassis effective mass.
- **Surfaces and rolling resistance** (`FUN_009d8bd0`, `FUN_0054d750`, `FUN_009d4a10`): each wheel's surface
  (collision tag) → `SurfacePhysics` `RoadOverrides`: `Surface_Grip`, `5c2a9f18` (rolling, 0.01 on asphalt) and
  `7585f9cb` (slide-drag scale); the car's link adds |mph| × a drag ramping with the slip angle (0 → 4 N per mph
  over 15° on asphalt). Ported per wheel (`WheelSurface`), replacing our Crr 0.015; the flat test track counts as
  the game's `Default_surface` (rolling 0.01, grip 1, no slide drag). Not ported: the fade when the surface changes.
- **Side effects of the true grip**: the 911 and Veyron hold 2.6 g on the skidpad, the scripted brake-tap drift no
  longer reaches the 7° rear slip (results in handling.md), and braking below ~30 km/h tails off exponentially
  (the tyre pass is no longer saturated); the game's standstill hold (`BrakeBehaviour.99407bbc` = 15 m/s²,
  `FUN_009b1180`, scaled by vehicle +0x1aa8 whose meaning is unknown) is what should finish a stop: not ported.
- **Standstill**: the slip angle's floor in the game is a physics epsilon (`DAT_01188270`, compared against
  throughout the solver), so slip angles at rest are noise in both games; with `Minimum_speed_to_enter_drift` 0
  (XKR) our drift state now flickers on for a tick at 0 mph in the replay. The gate belongs in drift.rs (the
  original's bodies come to rest exactly). Our rolling resistance is clamped to what stops the contact (ours).
- **Open**: the tyre constraint's +0x210 scale on the slip-blend inputs (no writer in the dump; if it is large,
  any brake or wheelspin collapses the lateral grip, which is how a brake tap could start a drift); which state
  moves the downforce point to `35d32b7d`; vehicle +0x1aa8; the strut travel (`WheelSuspensionConstraint`
  `7fc0dc41`?) and the rebound damper slot (no reader found).

### 2026-10-06: update loop, steering, drift and handbrake re-read from the code

Every rule in the tick from the steering to the drift, checked against the dump line by line (decomp.md "Update
loop", "Low-speed turn-around assist", "Steering", "Drift", "Handbrake and burnout" have the details and
addresses). Changes to the model:
- **Order**: the steering runs after the tyres (`FUN_00904260` step 7), so each tick's tyres use the previous
  tick's wheel angle; ours now does too. The full-rate test (0.99) reads the raw stick, not the in-drift remap.
- **Drift side force pushes outwards.** It acts along the body's side axis with the sign of the drift scale S,
  and the yaw torque with −sign(S) about the up axis (`FUN_009ce240`, `FUN_009d0860`). The torque must turn the
  car into the drift (it is the donut torque, largest at full lock into the drift), so the force points away from
  the turn's centre. Ours pushed inwards; that was why the cap of 5 gave a 3 g "grip circle". The cap (`.bss`
  0x01187380) applies to the dimensionless factor, so the force can reach 5 × `09832a93` = 70 m/s²; it is only
  applied with every wheel down, while torque, scale and speed hold run whenever drifting.
- **Damping time base K = 60** is in the code: `(1 − c)^(dt × K)` in `FUN_00907b50` / `FUN_0091dad0` with K at
  `.bss` 0x01184cc0 / 0x01185640 (both read live as 60). Damping now applies before the forces and on the exit
  tick, as in `FUN_009d91a0`.
- **Steering in a drift**: the early counter-steer limit counts from `Slip_angle_to_enter_drift` (not 0) to
  `fb0656a3`; the counter-steer opening is raised to a power (`powf`, exponent not visible; taken as
  `Max_counter_steering_power`); the wheels themselves are clamped to the limit; the counter-steer rate blends in
  with body slip from the entry angle to `5277ec9e`, only while counter-steering against the rotation. The yaw
  boost of the side force is gated by the same counter-steer flag (ours had it inverted).
- **Handbrake** (`FUN_009d27b0`): a countdown reset to `Minimum_handbrake_time` while pulled, so it stays on
  that long after release. While on: rear brake torque = max(handbrake strength, service) (no forced lock: the
  game's lock mask comes from other states), the front axle takes all of the drive (`FUN_009d5520`: also on
  rear-driven cars; matches the drivetrain agent's live reading of `ref/xkr_launch`), no drift can start, and a
  drift ends (code 9). A drift that starts on the tick after release gets the handbrake's scale. The state is read
  through the vehicle's virtual +0x44 (vtable 0x01023758, entry 0x00920810, not in the dump), identified by
  these uses.
- **Burnout** (20 mph live): releases the front brakes on front-driven cars, else the rear (AWD keeps its front
  brakes); blends the driveline split towards that axle; blocks drift entry.
- **Ported**: the low-speed turn-around assist (`FUN_008f8360`, wall contacts from avian), the drift's exit
  steering scale `ca545e03`, the counter-steer torque gate (`c901c1e8` / `e40fa397`, 0 everywhere), the
  `de67c45d` forward hold (0 everywhere). Body movement has no per-tick stage (`BodyMovementBehaviour` is read at
  setup); `Angle_control` exists as a type only. `VehicleInfo.40634c09` (the drift torque's per-car scalar) is
  (1, 1, 1) on all 197 cars.
- **Results** (handling.md "Measurements"): the 911's brake-tap drift now spins (peak 72°, down to 10 km/h)
  instead of holding 22°; the MP4-12C holds 33° for 5.2 s (was 12°); the F-150's skidpad falls from 2.0 to 1.2 g;
  handbrake taps give 3–4° (F-150 2.7°, was 13.9°). Not tuned; needs the original's runs.
- **Open**: the speed-hold constant at `.bss` 0x01187250 (60 assumed; `memscan dump NFS13.exe+0xd87250`); the
  counter-steer exponent's source; the steering neutral offset's input (vehicle +0x2c70: no writer found); the
  collision exits and wall-hit steering reset (a contact value > 6, unit unknown); the handbrake state machine's
  `.bss` thresholds 0x011870f0 / 0x01187070 (no reader of its outputs found); the per-wheel surface factor.
- Dead ends: `FUN_00920810` (vtable +0x44) and the drift's `powf` exponent are not recoverable from the dump;
  `0x006e…` writers of a +0x2c70 belong to another class; `FUN_009d4f20` reads a curve, not `BodyMovement`.

### 2026-10-06: the tyre screech "horn" was a decoder bug; skid intensities from the game's code

- **Cause of the horn**: every 32 kHz `Wave` decoded to just its 47-sample PCM prefix. `mw_audio::sps` rebuilds
  EALayer3 granules into free-format MPEG frames of the nominal 640 kbps size, capped at 2304 bytes; at 32 kHz
  the nominal size is 2880, so frames were exactly 2304 bytes, and minimp3 (nanomp3) only searches free-format
  sizes *below* 2304 (`k < MAX_FREE_FORMAT_FRAME_SIZE`). It found no frame, and the 47 samples were played as a
  loop: a 681 Hz buzz (32000 / 47) with all its harmonics, times the item's pitch (1.1 → 749 Hz, 1.3 → 885,
  1.4 → 953: the "peak ≈ 950 Hz" of the 2026-10-04 measurement). Fix: cap at 2303. Affected: 18 of 281 waves
  in `HAWAII/SURFACESOUNDS.BNDL`, 9 of 119 in `SOUND/METALCAR.BNDL`, 16 of 141 in `SOUND/GLOBAL.BNDL`; all now
  decode to their header's sample count (e.g. `…f34e3` 65014 samples, 2.03 s), with no discontinuity at
  granule edges (|diff|/rms 0.14–0.19 inside and at edges).
- **The default surface's `Drift` sequence** (`…f4eae`, bound to `LatSlipIntensityForSkidSound` 0..1) has six
  wave layers: [0] 0.03–0.40 `…f34e3` pitch 1.3, gain 1–1.5 by speed 40–80 mph; [1] 0.065–0.33 `…f4ead` /
  `…f4eac` (48 kHz) pitch 0.6–0.7 by `Jitter`; [3] 0.18–0.44 `…f4eb4` (48 kHz) gain 2; [4] 0.33–0.83 `…f4eb2` /
  `…f4eb1` pitch 1.1 gain 2.5; [5] 0.62–1 `…f4eb5` / `…f4eb6` pitch 1.1 gain 2.5; [6] 0.43–1 `…f34e3` pitch
  1.4, gain 3 → 1 as speed rises 0 → 100 mph. Layers 0, 4, 5 and 6 are the 32 kHz waves, so above 0.44 the
  drift was nothing but the buzz. Decoded, they are tyre squeal like the others: lines at most +8 to +14 dB
  over their third-octave (the recordings' own squeal), energy mostly at 500 Hz–2 kHz.
- **Truly tonal layers by design**: the `Brake` sequence's items [2] (`…d4a7e`, a 5.24 kHz tone, pitch 0.9 →
  4.7 kHz) and [3] (`…d4a7a`, a 2.75 kHz sine) on channel `…d4a85` (gain 0.36), with curves peaking at 0.31 /
  0.19 between braking intensity 0.03 and 0.63. In a braking sweep they hold −25 to −28 dB of the total: faint,
  as the data sets them.
- **Measured** (`mwtool vehicle-sound --sweep lat --mph 40`, 13 s, intensity 0 → 1; "lines" = energy in bins
  more than 20 dB above their third-octave median): before, lines held −0.7 to −1.6 dB of the total from
  intensity 0.5 up (≈ 70–85 % of the sound was the buzz, peaks +33 to +41 dB at 750 / 1500 / 2244 / 3744 Hz);
  after, no such bins at any intensity, strongest line +8 to +15 dB. Level at full drift −16.2 → −11.9 dBFS.
  Scripted 12 s render: drift segment −16.7 → −12.5 dBFS, peak prominence +45 → +10 dB.
- **Skid intensities, from `FUN_009d1bf0`** (decomp.md "Tyre skid sound intensities"): one reference wheel
  (the first rear wheel, or the first front one on front-drive cars); lateral = slip angle 0.05° → 40° times
  speed 0 → 10 mph; spin = |tread − ground| 2 → 20 mph (a locked wheel counts); braking = pedal 0.3 → 1 times
  speed 0 → 40 mph, halved, or the speed factor alone under a vehicle predicate (assumed: handbrake); per-tick
  rise / fall lerps 0.5 / 0.1, 0.12 / 0.2, 0.05 / 0.5. The values come from `TyreSoundParams` (`…1b0ca7`,
  `AssetStore::tyre_sound_params`). Ours had: lateral full at 12° (so a 15° slide sat on the drift's top
  layers, the ones that were broken), spin on/off, braking from locked wheels or 0.45 × pedal, max over all
  wheels, 80 ms smoothing. In `--test drift` the lateral intensity now peaks at 0.75 (5.5 s above 0.1, 3.7 s
  in 0.6–0.8); `--test brake` locks the wheels, so the spin sequence plays (as the game's formula does) and
  braking reaches 0.48.
- **Open**: the runtime constants `0x0118a060` (slip-speed scale, read as mph) and `0x011870d0` (wheel-load
  threshold for spin) and what vehicle +0x1b80 and the virtual method +0x44 are (memscan dumps / the probe
  would settle them); `GetPlayerSuspensionLoad` (`FUN_006d92d0` → vehicle +0x1f28, smoothed per tick by
  `FUN_009aae90` towards the mean of two axle ratios) drives the drift / brake channels' gains (0.4 → 1, exp 2)
  and is not fed yet (0: those channels sit at 0.4 / 0.5); `BusMixer` / `Lfo` sequence items are ignored; our
  ×4 / ×3.5 / ×2.5 skid mix gains are still assumed. Confirming capture of the original: see PLAN M5.4.

### 2026-10-06: the yellow tint: ColourCube channels were swapped; the post chain lives in NFS13.exe
- **Where the grade happens**: the earlier entry ("tonemap, auto exposure and colour cube") found no PC post
  shader in `SHADERS.BNDL` that samples a cube except the rear-view mirror and assumed the main view does the
  same. The real final pass is one of 74 DXBC programs in `NFS13.exe`'s `.rdata`, which is not encrypted
  (`mwtool shader-asm --exe list`, new). `0x01016148` and its 11 variants (created in `FUN_008e41e0`):
  distortion offset, optional DOF, splatter overlay, then `out = Sampler3dTint(rgb ×
  ColourCubeScalesOffsets.xyz + .w)`, the last operation on the frame. The whole chain is in
  [decomp.md](decomp.md) "Post-processing and colour grade".
- **The grade volume is built on the CPU** ("TintBlend", `FUN_008e5f80` / `FUN_008e0930` / `FUN_008e0ac0`):
  a job blends up to seven ColourCubes with weights into a 16³ four-byte buffer, which is copied into the
  texture row by row. The kernels (`0x00eb56d0`… via the table at `0x010e8f70`, indexed by cube count) are not
  in the dump.
- **Cause of the yellow**: the cube bytes were read as "blue fastest, stored R, G, B". A straight blend + row
  copy keeps the stored order, and the shader samples x = red, so the stored data is the texture's own layout:
  red fastest, each entry B, G, R (+ alpha → B8G8R8A8). That is the same bytes with red and blue swapped in
  both the index and the output. The identity cube `…3e93` reads as the identity either way, which is why the
  old reading passed its test. With the fix, the noon cube `…4e220` turns mid grey 102 into (99, 112, 116):
  it cools the mid-tones against the orange key light (11.0, 7.95, 5.21), where the old reading warmed them to
  (116, 112, 99) on top of it. Supporting evidence from the PostFX states' bindings: the cooldown cube
  (`Status.IsInCooldown`, `…7898`) now reads blue-steel (grey 102 → (77, 102, 118)) instead of sepia, and the
  noon / dawn cubes give teal shade under neutral highlights. **Inferred, not confirmed**: the kernels that
  would prove it are missing from the dump; a screenshot of the original settles it (procedure below).
- **Change**: `mw_formats::ColourCube` now reads the layout above (`at(r, g, b)` returns the graded RGB for
  input (r, g, b)); `tonemap.rs` builds the texture with x = red as before.
- **Measured** (road-cliff viewpoint, `--units 20`, sRGB means; warm = (R + G)/2 − B):

  | hour | region | before RGB / warm | after RGB / warm |
  |------|--------|-------------------|------------------|
  | 13 | whole frame | 114 110 79 / +33.0 | 105 110 95 / +12.7 |
  | 13 | road | 70 63 58 / +8.4 | 57 63 71 / −10.6 |
  | 13 | concrete post (shade) | 90 93 94 / −2.7 | 73 93 108 / −24.3 |
  | 13 | white cliff rock (sun) | 171 169 154 / +15.6 | 165 169 164 / +3.1 |
  | 13 (downtown plaza) | whole frame | 176 171 148 / +26.2 | 174 171 163 / +9.5 |
  | 9 | whole frame | 118 117 98 / +19.6 | 131 126 102 / +26.3 |
  | 18 | whole frame | 116 115 105 / +10.6 | 126 122 108 / +16.3 |
  | 22 | whole frame | 147 132 115 / +24.0 | 169 151 125 / +34.5 |

  Noon (cube `…4e220`) loses its cast. 09:00 (`…1244fc`) and 18:00 (`…0f0862`) are close to neutral cubes, and
  22:00 uses the identity cube, so their warmth comes from elsewhere: the key light and fills, the missing
  TintColour (21–23 h: (0.745, 0.776, 1.0), blue), the missing bloom (noon BloomColour (0.29, 0.47, 0.6), blue)
  and vignette. The foliage stays golden: it is the textures' autumn colour.
- **Now questionable**: the provisional skylight in `sky::irradiance_cube` (SKY_FILL) was added to stop the
  grade turning shade olive, which was the swapped cube. With the fix, shade in the sun-lit noon frame reads
  blue (post −24). For comparison, the same noon frame with the skylight off (SKY_FILL = 0, not committed):
  road 48 48 49 (−1.5), post 37 36 37 (−0.5), whole frame +18.8: neutral greys, but the shade is much darker.
  Left in place until the original's shade colour is measured.
- **Open**: the TintBlend weights (does the environment cube replace or blend with the PostFX states'
  identity cube, and are keyframe pairs blended or the nearest taken, as we do?); who fills the composite's A,
  B, D (`EnvironmentKeyframe.Camera.TintColour`, `BloomColour`, the `PostFxKeyframe` vignette / bloom fields);
  bloom; the night warmth.
- **Confirming screenshot of the original** (not taken: needs the user): free roam, pause or park on the
  coastal road at (−1131, 27, 1627) looking towards (−1190, 15, 1720), clear weather, 13:00, no pursuit /
  nitrous (those states add their own cubes). `tools/orig/orig launch`, drive there, `tools/orig/orig shot
  <scratch>/orig_h13.png`; ours: `mostrusted --camera=-1131,27,1627,-1190,15,1720 --units 20 --hour 13
  --screenshot <scratch>/ours_h13.png`. Compare the asphalt and the concrete post's (R + G)/2 − B: the old
  reading predicts warm (+8 road), the new one cool (−11 road). The time of day in the original must be read
  from its clock (`GameTimeOfDay`), since the hour is not settable from the menu.

### 2026-10-06: crash rules and the crash camera (M3.3)
- **Rules** (`Physics.CrashingRules`, 14 instances: 10 in GAMELOGIC/GAMEPLAY, 4 in PHYSICS/PHYSICSCONFIG): ten
  `ImpactRules` arrays (named so far: `LandingRules`, `PlayerRules`, `RoadblockRules`, `TrafficRules`,
  `WorldRules`) with their counts; each rule has three float-array thresholds and an optional
  `DamageOverride`. `FUN_009a9fd0` is the test: damage over `1617a710`, closing speed (mph) over `2414aa68`, a
  third value over `d6c0e908` (empty everywhere). The player's rules come through its damage bar profile
  (`DamageBarProfiles.PlayerOffline` → `CrashingRules` handle at +0x34). Offline player: world 30 damage /
  70 mph. Write-up: decomp.md "Crashes"; values: handling.md "Crash camera".
- **Crash camera** = `Sequences2[1]` of `FeedbackMessageGroup` 457707 (`0100000000068b43`): slow motion
  (1 → 0.1, 0.14, 0.22), two `CameraSequenceItem`s with `BystanderCameraParams`, `HideHudSequenceItem`, a
  letterbox `UILayout`, a `SceneAnalysis` and mixer automation, 5.5 s on the real-time clock. Identified by
  content only: the code that plays it on a crash was not found (the item handlers sit behind vtables).
- **Names cracked** (`genesys-crack`, camera vocabulary): `Camera_Transform_Binding`,
  `Target_Transform_Binding`, `X/Y/Z_Offset`, `Target_X/Y/Z_Offset`, `Screen_Offset_X/Y`, `Maximum_FOV`,
  `Minimum_FOV`, `Use_Zoom`. Unnamed: `86980811` (10 / 1000), `cc8312fe` (50 / 5.4), `005c8fc7` (bool),
  `72b09fef` (10 / 3.07), `c6b768dd`, `8b9e63bb`, `af29af91`; 4-word searches only gave nonsense.
  `SAS.Analyser["Freeway"]`: analyser names are field hashes of the string (`Freeway` = 892525534,
  `Landing` = 1890182743).
- **Reset after a crash** (`FUN_006d0f40`, `FUN_006c9fb0`, `FUN_006d0180`): the last pre-crash pose, rolling at
  `min(max(30, 0.7 × mph), 50)` (`GameLogicConfig` `75d82dd5`, `a44eb2f3`, `74521df4`). The wait is
  `GameMode+0x44`; no `GameMode` object is in the PC bundles (`mwtool genesys all --type GameMode` finds none,
  and `SmashAndGrabConfig.GameMode` 2210000 is not a resource).
- **Dead ends**: the world impact's damage value and closing speed are computed inside the contact solver
  (`FUN_0093bc40` copies the thresholds into the response block; `FUN_008fab30` and `FUN_009bb640` move
  flags but the comparison was not found); the `DAT_0118a090` scale in the closing-speed test is `.bss`
  (read it live: `mwtool memscan`, `dump NFS13.exe+0xd8a090`; 2.2369 expected); the bystander and analyser
  maths (no in-place reads of `BystanderCameraParams`; analysers built by `FUN_007d9840`…).
- Implemented: `mw_assets::crash` (rules, sequence, bystander params, analyser, letterbox, reset speeds; all
  rules and the sequence verified by `crates/mw-assets/tests/crash.rs`), `crates/mostrusted/src/crash.rs`
  (detection, slow motion via `Time<Virtual>`, shots, HUD, letterbox, reset), `--crash-wall-width`,
  `MW_CRASH_TRACE`.
- Open: confirm against a capture of the original's crash cam (procedure in the session report: a
  `tools/orig` run into a wall at > 70 mph closing, screenshots at 0.3 / 1.25 / 3.25 / 5 s, and the bystander
  camera's position and FOV read with `memscan`); the analyser's y = −8; the shot FOV with zoom off; whether
  free roam resets rolling.

### 2026-10-05: `mwtool tune`; present and input paths of the original

- Read live through the tuning menu (XKR, car park, idle): the player car at table index 48; idle 1012 rpm;
  wheel loads N 4576 / 4596 / 4071 / 4051 (front heavier, sum ≈ 17.3 kN ≈ the car's weight); grip multipliers
  all 1; the `.bss` constants as in `consts.txt`. The `drift_threshold` field (vehicle +0x2c10) holds FLT_MAX
  while not drifting, so it is state set on drift entry, not a tunable threshold.
- From the dump (research for an in-game overlay that was not built; probe.md "Status"): the present wrapper is
  `FUN_0046fb60`, which calls `IDXGISwapChain::Present` (vtable +0x20) on the swapchain in `DAT_011acde4`
  (device in `DAT_011acd64`, created in `FUN_00d9bb20`); the frame end is `FUN_00476f20`. The pad is polled
  on an input thread (`FUN_0040f140`, also `FUN_004119c0`) through `XInputGetState`; DirectInput 8 is created in
  `FUN_0040fa40` for keyboard and mouse. Dead ends: `GetAsyncKeyState` is imported but never called;
  `DebugPrint` is a script binding to the debugger log, not on-screen text; no game text-drawing helper.
- Open: the pointer from the vehicle object to its loaded GeneSys handling resource, which would put grip curves,
  gear ratios and drift behaviour on the tuning menu; whether the duplicate value next to each `.bss` constant
  (e.g. 1.2 at both 0x011872a0 and 0x011872a4) is a second copy the code also reads.

### 2026-10-05: the probe loads (Phase 0); memscan poke (Tier 0)

- The unpacked game loads `xinput1_3.dll` and `dinput8.dll` by name (`orig modules`, from `/proc/<pid>/maps`),
  both from Proton's builtin directory by default, and EA's `IGO32.dll` overlay injects itself from the EA
  Desktop install in the prefix. A `xinput1_3.dll` next to `NFS13.exe` plus `WINEDLLOVERRIDES=xinput1_3=n,b`
  (Proton appends its own overrides to ours) loads our DLL; `LoadLibraryW` of the full `system32` path then
  gets Wine's builtin behind its placeholder file, so forwarding works. Wine's `xinput1_3` has no
  `XInputGetDSoundAudioDeviceGuid` export.
- The DLL is loaded after the wrapper has unpacked `.text` (the game asks for XInput during its own init), so no
  waiting was needed. The vehicle-table chain `[[[NFS13.exe+0xcedb04]+4]+0x53220]` resolves ~1 s after that,
  on the title screen already; `player_index` = 48 is the gameplay signal. Seven minutes of free roam with the
  DLL loaded: no change in behaviour, input or readable state.
- `memscan poke` verified: a code address is refused, a `.bss` constant can be changed and is restored by
  `unpoke` and at exit. The detour step of Tier 1 is not written (probe.md "Status").

### 2026-10-04 (session 3, end): the exe's headers; the probe designed

- `NFS13.exe` on disk (PE headers, not code): image base `0x400000` with **no `DYNAMIC_BASE`** (DllCharacteristics
  0x8100 = NX compatible + terminal-server aware), so the game is never relocated and every address in the dump
  and in decomp.md is the live address. The import table names only `SHLWAPI.dll`, `KERNEL32.dll` and
  `USER32.dll`: those are the wrapper's; the game's imports are bound after it unpacks. `.text` is 0xb80e81
  bytes at `0x401000`; the wrapper is the unnamed 0x12f6-byte code section at `0x11dd000`.
- The game runs in Proton 11 (new WoW64: 32-bit PE in a 64-bit Unix process). The prefix has Wine's builtin
  `dinput8`, `winmm`, `version` and `xinput1_1..1_4` in `system32`. Toolchain for a 32-bit Windows DLL is not
  installed (no `i686-w64-mingw32-gcc`, no `i686-pc-windows-gnu` target).
- Design for running our own code inside the running game written up in [probe.md](probe.md); policy added to
  reverse-engineering.md. Open: which DLL names the unpacked game loads (`/proc/<pid>/maps`), whether this Wine
  implements `AppInit_DLLs`, whether the wrapper re-checks `.text` after start-up.

### 2026-10-04 (session 3, end): driving the original ourselves; the first reference run

- **`tools/orig/orig`** runs the original headless inside gamescope (Steam launch option `tools/orig/orig-wrap
  %command%`; normal launches unaffected) and drives it: XTest keys into gamescope's own X display, screenshots
  via `gamescopectl screenshot`, memory reads from `handling.watch`, scripted steps (`waitfor speed_mph > 60`,
  `capture`). Getting keys in: headless gamescope never sends WM_TAKE_FOCUS, so Wine ignored all keys until the
  tool sends it to Wine's top-level window itself (`activate()`). The menus then take keys; driving reads only the
  device selected in Settings > Gameplay > Controller, so automated driving needs "Keyboard" there (restore the
  user's "Xbox 360 Controller for Windows" afterwards, as done this session). The virtual uinput pad is not seen
  while Steam Input is on for the game (Steam takes controllers through hidraw; nothing opened the device).
- **The original's controls** (this install): keyboard Left / Right steer, A accelerate, Z brake, Space
  handbrake; pad left stick, RT, LT, X handbrake.
- **First reference run** (`ref/xkr_launch`, Jaguar XKR, keyboard full throttle 5 s from rest, then full brake;
  replayed with `--replay`, compared with `mwtool compare`): the launch matches closely (original / ours at
  3, 4, 5, 6 s: 22.7 / 20.3, 39.5 / 36.6, 53.4 / 53.4, 63.7 / 65.3 mph; distance at 6 s 68.9 / 66.4 m; shifts
  at the same times). Ours trails by ~2 mph off the line. **Bug found: in 2nd gear our revs sit at ~6400 while
  the original's rise 4830 → 6430 with speed: our clutch does not lock after the 1→2 shift (XKR).** The braking
  half is unusable: the original yawed at −34°/s with ~1 g sideways and entered the drift state (it hit or spun
  on something in the car park). The original idles at 1004–1027 rpm (ours: a flat 1000 or 969).

### 2026-10-04 (session 3, later): constants read from the running game; the engine's 0.74 s steps

- **memscan fix**: `/proc/<pid>/maps` paths with spaces ("Need for Speed(TM) Most Wanted") were cut at the first
  space, so `NFS13.exe+…` addresses never resolved (the first `consts.txt` / `porsch.csv` were empty).
- **Live vehicle**: `handling.watch` (git-ignored) reads the player's car through
  `[[[[NFS13.exe+0xcedb04]+4]+0x53220]+4+8·i]` with i = 48 in free roam (`player_index`). Checked against the
  speedometer and the body velocity. First readings (911, accelerating in 2nd): tyre loads front ~2460 N, rear
  ~4000 N (sum = the car's weight), per-wheel grip multipliers all 1, drift entry threshold 7°.
- **`.bss` constants** (docs/decomp.md, read with `dump`): drag K 1.2; anti-roll scale 60; drift damping time
  bases 60 / 60; side-force cap 5; burnout speed 20 mph; clutch cap 5000 N m; shift phases 10 % declutch / 40 %
  neutral; downshifts × 0.5 time; downshift hysteresis 2 mph; idle governor 20 rpm, 0.05 per rpm, up to 0.5
  throttle; downshift blip × 2.5; declutched below 0.1; the steering's full-rate threshold 0.99 (inputs below it
  steer outwards at the raw `High_speed_steer_rate`); the slip-ratio stop threshold 0.01; g 9.81; and 60/2π at
  0x01187310 (the check value). Applied to the model.
- **Not consistent yet**: with the game's side-force cap (5) our side force reaches ~3 g and the brake-tap drift
  turns into a tight grip circle, so our reading of its size, boost or direction is off; the model keeps a cap of
  1 until the side force (`drift_side_f*` in the watch file) is recorded in a drift. With the 0.99 steering
  threshold the handbrake test's half-stick tap gives 4° (17.9° with the earlier guesses); whether that matches
  the original needs a recording.
- **Engine "steps" in game** (user: "a constant rpm held for about a second, then the next"): recorded the game's
  own audio stream (`parecord --monitor-stream`) and tracked its pitch: plateaus of ~0.7 s while the rpm rose
  smoothly (27 rpm per tick). The offline replay of the same rpm trace was smooth. Cause: `PlaybackSettings::LOOP`
  makes bevy_audio wrap the source in rodio's `repeat_infinite`, which buffers it 32768 samples (0.743 s) at a
  time, each rendered with the parameters of the moment it starts. The engine and SFX sources never end, so they
  now play with `ONCE`: the recorded pitch rises continuously. Dead ends on the way: an adaptive per-parameter
  ramp in `sequence::Player` and interpolating the rpm between physics ticks (both kept; neither was the cause).

### 2026-10-04 (session 3): handling rebuilt from the game's code

The user supplied a decompilation of the game (`re/NFS13_dump/`, git-ignored; rules in reverse-engineering.md).
Findings per subsystem are in [decomp.md](decomp.md); the model built from them is in handling.md "The model".
- **Method**: GeneSys fields are read in place as `*(res + 0x28) + FIELD_OFFSET + instance`, so the offsets from
  `mwtool genesys-types` identify the handling code; `.rdata` / `.data` constants come from the unencrypted parts
  of `NFS13.exe` on disk. About 30 tuning constants live in `.bss` with no initialiser in the dump; the memory
  dump the pseudo-C came from (`NFS13_dump.exe`, not on this machine) or `mwtool memscan` would give them.
- **Settled**: 60 Hz tick; the steering range / rate rules and the in-drift input remap; the drift state machine
  (rear-tyre slip entry, brake-tap threshold, signed drift scale, Bézier yaw torque with the donut top, body-axis
  side force, peak-latched damping, speed hold); engine as its own inertia with a friction curve, clutch, no
  efficiency loss, shift on rpm and speed, top-gear lock (911: 6 gears); brakes pedal² with a ramp, no ABS / TC;
  spring k = load / sag, asymmetric damper, anti-roll bars; per-axis drag with an over-speed ramp and lift
  (`Down_force` unused, which explains the parked downforce branch's regressions: wrong fields, ~7× too strong);
  the tyre as a capped velocity constraint with the game's curve shapes and slip blends (no friction circle).
- **Field names corrected**: `0c5f89b3` is a damper ratio (not a frequency), `41543f85` the static sag (not
  travel), `Engine_resistance` the declutched deceleration (not engine braking), `35e18207` / `6ce0fda7` the
  handbrake's drift scale (not grip), `0dce38f7` / `8f151151` / `b5290695` drift-entry angles and rate.
- **Results** (911): 0–100 3.88 s, top 311 km/h with no drag fit (data 304; drag K = 1), 100–0 40 m, skidpad
  1.01 g, brake-tap drift 22° held 4.5 s. Calibrated (model values): lateral / longitudinal grip scales 0.4 /
  0.7, clutch cap 1.6 × peak torque.
- **Bugs met on the way**: braking with brake and tyre as separate steps made locked wheels cycle (the tyre spun
  them back up each tick) — now solved together in sequential-impulse passes, as the game does; clamping the
  revs after integrating ate the drive near the limiter (the game clamps first); the speed hold's gain is per
  tick (the game logs it × 60); a degenerate tyre frame after a crash gave a NaN camber.
- **Open**: the absolute grip scale and per-wheel grip multipliers (measure N with memscan); the handbrake's lock
  mask condition (inferred: the handbrake); the steering neutral offset's input; the full-throttle donut (the 911
  no longer spins from full lock and throttle at rest; the rear slip stays under the 7° entry); the clutch cap
  and shift-phase constants.

### 2026-10-04 (engine sound from the game code): audible RPM, load, harmonics, the Ginsu player, the EQs

The user supplied a Ghidra decompilation of the exe (`re/NFS13_dump`, git-ignored). It settles several of the
"engine ground truth" entry's open questions; where the code is missing (not decompiled) this says so.
Function addresses are given so the reading can be rechecked; no decompiled code is copied.

- **Audible RPM is not the physics RPM** (reverses the previous entry's change). `RaceCarEngineSound` holds
  a fixed RPM range of 1000..8000 (constructor `FUN_0058c3f0`; never overwritten) and `GetAudibleRPM` /
  `Players.*.GetPlayerAudibleRPM` (`FUN_0083eb60`) return `min + (max − min) × normalised RPM`. The
  normalised RPM (vehicle `+0x1f10`, `FUN_009aea50` ← `FUN_009ae930`) is the idle..max fraction of the engine
  RPM (`Engine.Power_curve.Idle_RPM` / `Max_RPM`, `+0x2c` / `+0x38`) raised to `2 − a`, where `a` is an
  `EngineSound2` term crossfaded by engine load between `cb44d82d` (off load) and `a33d84fe` (full load), or
  `913bb900` when a vehicle flag (`+0x1f45`) is set. The 911 has 0.7 for all three: exponent 1.3, so 4400
  physics rpm (half the range) plays as 1000 + 7000 × 0.5^1.3 = 3840 audible rpm, and the idle loop alone
  (sequence axis 1790..7500) covers physics revs up to about 2300. The pow's argument order is read from the
  register use around the SSE `powf` call; the small offset `_DAT_01186680` and the second branch of
  `FUN_009ae930` (a term in a global tuning field `+0xd0`) were not resolved. A sum of a short history buffer
  is added before the clamp (gear-change blips?, not reproduced). Implemented in
  `engine_driver::normalised_rpm` / `audible_rpm`; `MW_AUDIO_RPM_LINEAR=1` restores audible = physics RPM for
  A/B.
- **Engine load** (`GetEngineLoad` → vehicle `+0x1f20`, `FUN_009b1f70`): a one-pole follower, rate =
  `EngineSound2.99d4b91e` (0.5) × an unresolved constant, towards `s + (1 − s) × h`, with `s = (clamp01(speed /
  reference) × Engine.937338e7 (0.75) × ?)²` and `h` a 0..1 value at vehicle `+0x1d14` whose writer was not
  found (read as the pedal demand); the output is the maximum of the instant blend and the last six smoothed
  values (peak hold). Implemented as `engine_driver::LoadFilter` with a 100 ms response and the top speed as
  the reference; the hold keeps the on-load layers for ~100 ms after a lift.
- **Tracked harmonics** (`FUN_00586330`, `FUN_00574ff0`): firing frequency = audible RPM / 120 × cylinders
  (`EngineSound2.ed754b99`, 6 on the 911); `TrackedHarmonic1Frequency` = firing × `7cbc11ca` (1),
  `TrackedHarmonic2Frequency` = firing × `7cbc11c9` (3; was 2 × harmonic 1 here), `SubharmoniserFrequency` =
  firing × `283e07de` (1), tunnel harmonic × `dbcddcfc` (6) with gain `11a77b36` and Q `64cbe1ff` (not
  bound in the 911's data). `CombRaspDelayTime` = 1 / (firing × `f0291dd3`). The LFO bindings are
  `0.5 × sin` of a phase times an amplitude (`FUN_00aabc30`), `SkidLfo` a sawtooth; `Jitter` = two member
  sums. `EngineOn` is vehicle `+0x1ef0`, `EngineGainDueToPower` `+0x1d1c` (writers not found).
  `GetNormalisedRPM` and `GetAudibleRPM` themselves are thunks the dump lacks.
- **Ginsu player** (`GinsuPlayer`, process `FUN_00aaf0a0`; helpers `FUN_00aad240` grain index at a
  position, `FUN_00aacf60` position for an RPM, `FUN_00aad040` grain start, `FUN_00aad0f0` grain length,
  `FUN_00aad3c0` copy / linear resample): the recording plays at its native rate (the copy is called with
  equal in / out counts: pitch comes from the grain choice alone, not resampling); every other mixer block
  (256 samples, the block counter at mixer `+0xc480`) the player may jump: it picks a grain within a window
  around the RPM's grain (window = parameter 1 of the plug-in, default unknown), favouring staying or stepping
  back one or two cycles at random, and jumps by whole grains from the fractional current position (phase
  preserving) with a 0.25 ms linear crossfade (12 samples at 48 kHz). The RPM parameter glides linearly over
  33 ms. Our overlap-add synth already keeps phase; adopted the 33 ms glide (`sequence::GinsuGroup`,
  replacing the 10 ms one-pole). Chunking keeps each copy ≤ 64 input samples (two XAS blocks).
- **DSP**: `PeakingIir2` (`FUN_00d01e60` / coefficients `FUN_00ced070`) is the RBJ peaking EQ with
  `A = √Gain` (Gain linear, 1 = bypass) and Q clamped to 0.2..20, which is what `dsp::Biquad::peaking` does.
  `HighPassButterworth` (`FUN_00cff950` → `FUN_00cde880`) and `BandPassIir2` (`FUN_00ce65b0`) are standard
  designs with the frequency clamped to (0.001π, 0.999π). **`Distortion` ('Dst0') and `SubHarmoniser`
  ('SUBH') are registered (`FUN_004b2cc0`, descriptors at `010d1930` / `010d2410`) but their code lies in
  gaps Ghidra did not decompile (`00ab0e30..`, `00aad940..`), so they stay stand-ins.** Their parameter
  tables give ranges: Distortion default 1, 1e-5..1000 (a drive), Model −100..100; SubHarmoniser Interval
  default 2, 0..25. Stand-in changes: the distortion is now `tanh(d·x)/d` (unity for small signals, so the
  drive-4 strain bus no longer rivals the clean path; `MW_AUDIO_DIST=raw` keeps `tanh(d·x)`), the octave
  divider outputs at the input's RMS with 2 ms edges and tracks nothing above 1.5 kHz.
- **Per-car `DspParam` overrides**: the index ↔ runtime parameter mapping is still not in the dump. Two
  readings are consistent across the data and now applied: `Parameter_name` 1 on a filter = Frequency (the
  911's global 6.5 kHz EQs 472213 / 472218 become 400 Hz × 2 and 1 kHz × 3; HighPass 472227 before one
  sub-harmoniser becomes 6810 Hz, i.e. that bus is switched off), 0 on a Send = its gain (472306 = 0 mutes
  the comb-rasp delay path). Delay "4 = 0.2" and BandPass "5 = 40" exceed the plug-ins' parameter counts
  and are left alone. Send gains (`SendDspPlugIn.Gain`, all 1 in the 911) are now applied.
- **Measurements** (`mwtool engine-sound 911`, scripts in the session scratch `engine/`): steady 4000 rpm
  full load, bands in dB relative to the 500 Hz band, raw set-A recording 20 Hz −23 / 60 Hz −6 / 120 Hz +6 /
  250 Hz +8 / 2 kHz −9 / 4 kHz −15. Wet before: −12 / −0.5 / +6 / +3 / −10 / −14 (the +11 dB sub-bass).
  Wet now: −24 / +0.5 / +7 / +9 / −12 / −19 (within 2 dB of the recording up to 1 kHz; 60–120 Hz +6 dB of
  which the sub-harmoniser stand-in is 5 dB; the top end 3–6 dB darker since the 6.5 kHz EQs moved).
  Harmonic / total energy 0.98 (before 0.87, recording 0.97). Off load the 100 Hz EQs (× 4 Q 2, × 5 Q 0.5) still
  put the 60–120 Hz bands 7–13 dB over the off-load recordings: that is what the data's EQ does. Default
  sweep: f0 tracks the game mapping within −41 cents median (the glide lag), local pitch spread p90 124
  cents (before 224); dry spectral centroid 422 Hz (before 469: the curve lowers mid-sweep pitch).
- **A/B switches**: `MW_AUDIO_DRY=1` (all plug-ins off), `MW_AUDIO_NO_DIST`, `MW_AUDIO_NO_SUB`,
  `MW_AUDIO_NO_EQ`, `MW_AUDIO_DIST=raw`, `MW_AUDIO_RPM_LINEAR=1`; `MW_AUDIO_METER=1` with `mwtool
  engine-sound` prints each channel's level and plug-in chain with the applied parameters.
- **Dead ends**: the `GetNormalisedRPM` / `GetAudibleRPM` thunk targets (`LAB_005a2190` / `LAB_005a2240`)
  are not in the dump; vehicle `+0x1d14` (load source) has no scalar writer in the decompiled files; the class
  descriptors' parameter tables carry no names; the `DspParam` reader (`EngineSound2 +0xd8..+0xe4`) was not
  located (the +0xd8 hits in `0057/0058` are other GeneSys types).
- **Open**: listen to `after2_911.wav` vs `before_911.wav` (scratch) and in game with / without
  `MW_AUDIO_RPM_LINEAR`; the Distortion model and the SubHarmoniser law need the two undecompiled
  functions (re-run Ghidra over `00aad940..00aade60` and `00ab0e30..00ab1110`); the Ginsu window parameter
  default; the `DspParam` indices 4 / 5; `EngineGainDueToPower`; the state lists.

### 2026-10-04 (session 2): "everything is yellow": skylight in the ambient
- User report: the frame looks yellow. Causes: the noon key light is warm (11.0, 7.95, 5.21), the ambient rig
  is small (up fill 0.065 × 2.6), and the noon ColourCube warms shadows and mid-tones (grey 102 → (116, 112,
  99), 68 → (61, 55, 49); highlights stay neutral). With no blue skylight in the shade, the grade turns it
  olive (the car's shadow on the grey test track).
- The data's own balance: SkyReflectionAverageColour (1.12, 1.69, 2.44) is strongly blue. Warm sun + that sky
  on a horizontal surface sums to about neutral white, which looks like how the keyframe is meant to balance.
- Change (provisional, `sky::irradiance_cube`): the irradiance cube = the rig's fills + the upper hemisphere
  at SkyReflectionAverageColour, cosine-weighted ((1 + n.y)/2). Shade is now blue-grey, white stone white.
- Found on the way: the `Diffuse_1Bit_*` forward pass lights with `IrradianceQuadric` (7 × vec4 in
  gGlobalCB: second-order SH as a quadric), the CPU-built ambient; its G-buffer pass writes only RT0 / RT1
  (albedo = texture × materialDiffuse, roughness 1). The golden foliage is the textures' own colour
  (Fairhaven's autumn trees), not a shading error.
- To settle the ambient: a reference screenshot from the original (shade colour next to sunlit grey).

### 2026-10-04 (audio session, later): tyre screech, road, wind, impacts and scrapes from the sound data
- **Tyres** come from the surface data, not from `TyreSound` (its waves in `VEHICLES/WAVES.BNDL` are all the
  silent 0.1 s placeholder `…79ff`). `HAWAII/SURFACELIST.BNDL`: `Surfaces { Default_surface, Chevron_surface,
  Surfaces [65] }` → `SurfaceType { two condition sets (f395b019 / 7348be25, read as dry / wet) { Flats, Normal,
  Offroad, Rims, Track: { Surface_Physics, Surface_Rumble, Surface_Sounds } } }` → `SurfaceSounds { Brake,
  Drift, Spin (+ _Competitor), Reflection, two speed-bound road-noise sequences (4648873a, 1376f03b), Bumps,
  Transitions, TyreParams }`. Default surface, dry, Normal → `…3d399`. The skid sequences bind
  `RaceCars.Entities["Id"].LatSlipIntensityForSkidSound` / `SpinIntensityForSkidSound` /
  `SpinBrakingIntensityForSkidSound` (0..1) and layer loops and one-shots across that range (the drift: 15
  items; it swaps layers rather than getting louder). The game computes the intensities from `TyreSoundParams`
  (fields unnamed; Lateral 0.4 / 1.3 / 25 / 20 / 18 / 80 mph...). Ours (assumed): lateral = slip angle 4° → 12°
  (= 0.4 → 1.3 × a ~9° peak angle) faded in from 2 to 8 m/s; spin = a wheel spinning under power; braking =
  a locked / sliding wheel, or 0.45 × pedal under hard ABS braking. Inputs from `Car::wheel_contacts()` (slip
  split along the car's heading).
- **Wind**: `VehicleSound.Player_Sequences` (`SOUND/GLOBAL.BNDL`; 12 entries once the handle-array fix
  applies) has a speed-bound sequence (`…87ea6`, 0–180 mph) with wind layers crossfaded 45–90 / 70–138 /
  100–250 and 19 mixer items (bus ducking by speed: those target the engine's buses, which live in the other
  player, so they do nothing here). The other player sequences are wall reflections (`WallTracker...`), cop
  distance, dust clouds and damage; not played.
- **Impacts**: every car's `Vehicle_Sound.Collision_Sounds` (UniqueId 140990) is the `CollisionSoundSet` in
  `SOUND/METALCAR.BNDL` (`…226be`): 31 unnamed sequence slots (timelines of crunch and debris waves, random
  pick per item) and 24 wave pools (named: `Scratch`, `Sparks`, `Landing` (empty)). Bindings inside: 
  `RaceCars.GetMostRecentPlayerImpactClosingVelocity` (ranges 2–60, read as mph), `GetMassForLastCrash`,
  `MostWantedCarInLastCrash`. Which slot is which collision type is unknown: the 21 timeline slots are sorted
  by loudness into light / medium / heavy cues (closing speed < 4, < 12, ≥ 12 m/s) at 0.4 / 0.8 / 1.4 of the
  impact level. `Scratch` (two 3 s one-shots) becomes a loop (crossfaded seam) bound to the body's sliding
  speed; `Sparks` a cue on hard hits and scrape starts. Landings after ≥ 0.25 s airborne use the impact cue at
  the touchdown speed. Body contacts come from `particles::ImpactEvent` (closing and sliding speed).
- **Playback**: one more sequence player (`crates/mostrusted/src/sfx.rs`), separate from the engine's; it uses
  copies of the engine's player and loader (`mw_audio::sfx_player`, `sfx_dsp`, `mw_assets::sfx_loader`, no
  Ginsu) while the engine path is reworked, with a 20 ms release on stopped voices. Our mix (the bank ends in
  buses outside the bundles, like the engine's): master 1.4, skid sequences ×4 / ×3.5 / ×2.5, impacts ×3,
  scrape ×2.5 (applied to the items' gain values).
- **Measurements** (`mwtool vehicle-sound`, scripted 12 s; `tests/vehicle_sfx.rs`): road + wind at 50–60 mph
  −24 to −26 dBFS, drift −16, locked brakes −15 to −16, wheelspin −17, heavy impact −6, scrape −21, wind at
  120–160 mph −22; no limiter hits. Drift spectral flatness 0.000 (a tonal squeal, peak ≈ 950 Hz, centroid
  820 Hz), locked brakes 0.05 (centroid 1.3 kHz), scrape 0.30 (noisy, centroid 3.7 kHz), road and wind < 0.01
  (rumble, centroid 170–290 Hz). Discontinuity peaks (3.0) are in the scrape and match the raw `Scratch` wave's
  own (3.0). In game (`MW_SFX_TRACE`): `--test drift` lateral intensity above 0.3 for 5.9 s of the 5.8 s drift,
  braking only during the 0.36 s brake tap; `--test handbrake` locked 3.8 s; `--test brake` ABS squeal 1.9 s;
  `--test crash` one heavy impact at 31.9 m/s; `--test accel` no wheelspin (traction control).
- Open: surface types (collision tags) to pick other surfaces' sounds and their bumps / transitions; which
  collision slot is which (car vs wall vs traffic, front / side / rear?); `TyreSoundParams` field names;
  ambience (`AmbienceSequences` loop with `JumpTimelineController`s, not implemented), wall reflections, horn
  (`Vehicle_Sound.Horn` UniqueId 143356, no input), turbo / blow-off (EngineSound2 bindings, engine path).

### 2026-10-04 (engine ground truth): what the recordings say, status and next steps

Session cut short (user paused). The user hears the engine as "still wrong"; this session measured the synth
against the raw recordings instead of against its own metrics. Scripts are in the session scratch dir
(`/tmp/claude-1000/engine-scratch/*.py`: `ana.py` helpers, `grains.py`, `grain_phase.py`, `hold.py`,
`resynth_compare.py`, `graph.py`, `compare_bands.py`); regenerate the wavs with the commands below.

- **Decoder is clean** (911 main rev-up `01000000001d1c7c`): spectral flatness 0.0035 (tonal, no hiss), |diff|
  at XAS frame boundaries / inside frames 0.99, no DC, no clipping. Not the problem.
- **Grains are phase-aligned engine cycles.** Grain length = `120 · sr / rpm` within 0.2 % of the RPM table
  (median ratio 0.9985) on every recording, and the cross-correlation of consecutive grains (time-normalised)
  peaks at lag 0 for 95 % of pairs (p90 0.005 cycle; the off-load sweep 78 %). So concatenating grains at their
  boundaries is phase-continuous and jumps between nearby grains need no phase alignment. Consecutive cycles
  correlate only 0.74 (median): the engine itself varies cycle to cycle.
- **The single-layer synth is faithful.** New `mwtool ginsu-resynth <bundle> <index> out.wav [--hold rpm]`
  (`GinsuSynth::render`) drives one recording with its own RPM curve. Against the original: level within
  ±0.8 dB (p10/p90), per-50 ms spectral distance over 1/6-octave bands median 3.5 dB, p90 5.3, which equals
  the recording's own cycle-to-cycle floor (next cycle: median 3.6 dB, p90 5.0). Steady holds at 2,500 and
  4,000 rpm match the raw grains at that RPM within ~2 dB per order (order 3 dominant, order 6 +4 dB high at
  4,000). Only blemish: pitch spread p90 71 cents vs 45 for the original (the 1-in-8 random jumps).
- **The RPM axis is real engine RPM** (changed: `EngineDriver::audible_rpm` now passes the physics RPM
  through; the sequences clamp it to 1790..7500). Evidence: the idle loop `01000000001d1c7a` has its
  autocorrelation peak at 40.0 Hz and spectral peaks at 119 / 159 Hz, exactly the first grains of the Ginsu
  sweep (~1580 rpm: lag len/3 = 40.5 Hz, orders 4.5 / 6 = 119 / 159 Hz), and the sweep ends at 7814 rpm = the
  handling's `Max_RPM` 7800. The old remap (idle 1000 → 1790, redline → 7500) compressed the pitch range to
  0.84× and made 4,000 rpm sound like 4,300. Below 1790 rpm the car now plays the idle loop only, as the data
  says. Not verified by ear.
- **Mix findings, unchanged** (steady 4,000 rpm hold, `hold.py`): the dry six-layer bank is close to the sum of
  both microphone sets' recordings (harmonic / total energy 0.99, order 3 dominant). The wet chain adds +5 dB
  at 60 Hz and +11 dB at 20–60 Hz over the recordings (full load) and +11/+13/+10 dB at 60/120/250 Hz off load,
  and drops the harmonic ratio to 0.87 (added inharmonic content). Channel meter on the hold: bus B `0b381c`
  input +5.3 dB RMS (Peaking 100 Hz ×4 Q2 and ×5 Q0.5 are +12/+14 dB), the set-A strain path `073483` fed at
  −16.5 dB RMS into `tanh(4·x)`, i.e. a near-square wave about as loud as the clean path. These stand-ins are
  the prime suspects for "wrong"; the data cannot settle the distortion law or the sub-harmoniser level.
- **Data details**: plug-in field `2a740b25` is the output channel count (1 until the `Pan2D`, then 6). The
  911's `DspParam` overrides: Peaking 472213 / 472218 param 1 = 400 / 1000 (reads as Frequency: the global
  6.5 kHz EQs become 400 Hz / 1 kHz for this car), Delay 472308 param 4 = 0.2 (Feedback) and its Send 472306
  param 0 = 0 (the delay path is muted), HighPass 472227 param 1 = 6810 (does not read as a frequency before a
  sub-harmoniser), BandPass 750019 param 5 = 40 (bandwidth?). Field order in the types does not give these
  indices; still not applied. `GinsuDspPlugIn` carries no playback parameters (`a4831678` 1 / 0 only).
- **A/B wavs** (scratch dir, regenerate): `911/01000000001d1c7c_ginsu.wav` raw on-load sweep vs
  `resynth_on.wav` (same sweep through the synth), `resynth_hold4000.wav` vs the raw at 4,000 rpm,
  `hold4000_on.wav` / `hold4000_on_dry.wav` (bank wet / dry at 4,000 rpm full load, old RPM mapping),
  `synth_wet.wav` / `synth_dry.wav` / `synth_legacy.wav` (default rev-up and coast, old mapping),
  `game.trace` (in-game RPM from `--test engine`).
- **Next**: (1) listen to raw vs `resynth_hold4000` vs `hold4000_on_dry` vs `hold4000_on` to place the fault
  (synth, mix or effects); (2) if the wet chain is at fault, scale the unknown effects so they never dominate
  the recorded signal (distortion drive relative to the input level, sub-harmoniser at the band's level, EQ
  gains as given); (3) the 1-in-8 random jump: test following the recording strictly with jumps only when the
  walk leaves the window; (4) decode the `DspParam` index ↔ runtime parameter mapping; (5) check the audio
  callback cost (rodio underruns after the SFX merge).

### 2026-10-04 (session 2): wheel layout, the Venom's front axle
- User report: wheels off on some cars (Hennessey Venom GT Spyder). Survey over every car
  (`cargo test --release -p mw-assets --test wheel_survey -- --ignored --nocapture`): body z extent vs spec
  axle positions. Every car has front / rear overhangs of 0.46–1.36 m except the Venom: front axle at z = 0.125
  (2.20 m overhang, 1.63 m wheelbase on a 4.66 m body). The Veyron and one Caterham entry have swapped
  Front / Rear labels (already handled: the loader classifies axles by position).
- The 0.125 is in every source: VehicleGraphicsSpec wheels, the physics `RigidBody` transforms (`0d0c29b4`),
  `SuspensionWheel`, and the LO bundle. No other vector in the car's GeneSys data sits at a front-axle
  position. The Venom is Ultimate Speed Pack DLC. How the original shows it correctly is unknown (dead end:
  no override found in the bundles).
- Arches from the mesh: along z, the lowest point of the outer 25 % of the body's half-width jumps from the
  sill (~0.1–0.2 m) to ~0.55–0.73 m over each wheel. 911: arches centred at +1.25 / −1.25 (data +1.24 /
  −1.21); Venom: front arch +1.05…+1.55 (centre +1.30), rear around −1.5 (data −1.505).
- Fix (`mw_assets::wheel_arches`): an axle whose overhang exceeds 1.6 m moves to the arch nearest to a
  0.95 m overhang (within 0.6 m); applied to the spec wheels (viewer and drive) and the physics wheels. Only
  the Venom triggers it.
- Unrelated: the dark rectangle near the horizon in `--vehicle` shots is in the sky / cloud panorama (it
  shows behind the 911 too).
### 2026-10-04 (particles): BearEffect decoded, tyre smoke, kick-up and sparks
- **Where**: all 214 `BearEffect` (0x301) resources are in `GLOBALEFFECTS.BNDL` (26 are empty 0x30-byte
  placeholders), with 102 materials, 129 textures, 12 models (debris meshes) and one `BearGlobalParameters`.
  No other bundle has any; the vehicle and world bundles reference effects only through GeneSys handles.
- **Format** (layout tables in `mw-formats/src/bear.rs`, burnout.wiki "Bear Effect/NFS Most Wanted" for the
  layer header): effect header (version `0x269987a1`, layer count, action-group mask 3), a parameter table at
  0x20 (`u8 n, 0, u16 size, u32, u8 kind[n]`, then names; the wiki's table offset field points at a copy) with
  a handle array `{u16 offset | 1, u16 global id}`; per layer a 0x20 header, a name (`<number>_Test`), a
  channel table (`u8 n, u8 external, u16 bucket bytes, u8 kind[n]`, names), an action table (`u16 offset[n]`
  then `u8 kind[n]`) and four action-group offsets. A group is `u8 sequences, primaries, secondaries, pad`,
  `{u8 primaries, u8 per-bucket}` per sequence, `{u8 action, u8 secondaries}` per primary, `u8 action` per
  secondary: verified on all 1,113 groups (counts add up, indices in range). Group 0 = update (spawners are
  primaries whose secondaries initialise new particles), group 1 = render; 2 and 3 never occur.
- **Operands**: action payloads are 4-byte words; channel references are `u16 lo = bucket offset | scratch
  bit, u16 hi = 4 × channel index` (the scratch copy holds `Radius*`, `Colour*`, unpacked `Velocity*`),
  parameter references `hi = 0x40 + 2 × index`, engine references `hi = 0xEB40` (layer state; `.1` written by
  the controllers), `0xEB42` (view, `.a0` in every renderer), `0xEB44` once. Material / model imports sit
  inside the renderers (0x24 +72, 0x25 / 0x29 +32, 0x27 +80, 0x28 +112, 0x1F +56, 0x22 / 0x23 +24).
- **Action kinds** (52 seen; by which channels they touch, which group / role they have and their value
  ranges): spawners 0x00 (transform: box extent, LOD distance range, angle ranges, speed range, rate /s at
  +0x60, per metre +0x64, burst count u32 +0x6C; rate × lifetime ≈ max particles across layers), 0x01
  (collision mesh: speed, two angle ranges, LOD, rate u32 +0x54 (2000 while scraping), burst u32 +0x58
  (128)), 0x05 (wheel: LOD, speed, three angle ranges, lateral −1..1, height, per metre? +0x70, rate +0x74),
  0x07 (wheel contact), 0x06 (contact list), 0x2B (super-emitter), 0x4C (splatter); initialisers 0x0C
  lifetime (age = 0, 1 / rand(min, max)), 0x12 radius, 0x0A sprite rotation (turns) + spin, 0x0B 3-D
  rotation, 0x0D alpha, 0x13 bounce plane, 0x14 / 0x15 place in the emitter frame (quaternion + offset), 0x16
  inherit a velocity parameter × rand(min, max), 0x47 seed, 0x0F / 0x11 light / collision radius; updates 0x2F
  age (normalised), 0x3D integrate (gravity −9.8 for debris, +0.4 for smoke, drag), 0x3E integrate + bounce
  (restitution and friction ranges), 0x2C / 0x2D rotation, 0x40 rigid debris, 0x44 bounds, 0x49 / 0x4A / 0x4B
  velocity pack / unpack (`PackedVelocity`); render group 0x3C radius curve (four Bézier control points over
  age), 0x1C colour (rows 0–4 = polynomial coefficients in age for r, g, b and a brightness/alpha column;
  a 5-key u8 alpha curve at +144 with a scale at +156; sparks: `1 − t²`, `0.41 + 0.14t − 0.54t²`, … orange
  to black, column 3 = 12 − 4t − 7t²; smoke: blue-grey, alpha curve peaking at 0.31), 0x17 alpha × colour,
  0x48 seeded brightness, 0x38 camera fades (near fade 1–10 m), 0x1D atlas frame from age, 0x45 view;
  renderers 0x24 quads, 0x27 quads with atlas frames, 0x28 seeded quads, 0x25 / 0x29 motion-blurred sprites,
  0x1E / 0x1F / 0x22 / 0x23 meshes, 0x4D splatter, 0x30 lights. 5-key curves: `u8 values[5], u8 times[5]`
  (`ff ff ff ff ff | 00 fc fd fe ff` = 1).
- **Which effect where**: `HAWAII/SURFACELIST.BNDL` → `Surfaces.Surfaces` (65 `SurfaceType`, 0 = default
  tarmac) → two weather conditions (dry / wet) → `Surface_vfx` → `SurfaceVfx`: `WheelVfx` (kick-up, 0x07
  spawner on `WheelContact`), `PlayerWheelEffects` / `AIWheelEffects` → `VfxWheelEffects` (`DefaultTyre` =
  tyre smoke `…49247`, 7 layers, `WheelParams` input; `OnRims` = rim sparks), ten directional impact effects,
  tyre-mark materials, `RimMarksMaterial`. Tarmac: impacts front / rear / bottom = `…a394b` (two spark bursts
  of 128), left / right = `…7bd97` (continuous sparks at 2000/s: scraping). `VfxGlobals` →
  `VehicleImpacts` / `VehicleScrapes` (`VfxVehicleCollisions`), `Misc` → `BurstTyreWheelEffects`,
  `SpikeStripSparks`. Names cracked with direction / surface / effect vocabularies (added to
  `genesys_names.txt`). Dead ends: no hits for the 36 `VfxVehicleCollisions` fields with 4-word direction ×
  severity × impact vocabularies; `PuddlePuncturedBurnoutHit`, `RoofWheelsHeavyDebris`,
  `ContinuousCarCrashTrails`, `TrailsMeshVfxGlancing` were false positives on float fields.
- **Shaders**: `VfxParticles_Diffuse*` take CPU-expanded quads (world positions, COLOR, TEXCOORD.xy corner /
  .z age, TEXCOORD1.zw translucency / key-as-ambient, TEXCOORD3 lighting normal + wrap); the VS lights per
  vertex (irradiance quadric × 1.3 + wrap-lit key light), picks two atlas frames from `NumTiles` /
  `EnableLooping` (row index from frame / tiles.y), the PS blends them, alpha-tests at 1/255 and writes
  premultiplied colour with alpha × `gVfxConstants.z` (one blend state for normal and additive);
  `_SoftParticles` variants fade by scene depth. `VfxParticles_MotionBlurSpriteUntex` integrates a moving
  disc's (1 − r²) profile over the time each pixel is covered (analytic coverage from the two roots), colour
  × `Overbright` (6) × `gVfxSparkBrightness`, additive.
- **Implemented** (`crates/mostrusted/src/particles.rs`, `particles.wgsl`, `sparks.wgsl`): a CPU runtime that
  runs each layer's groups in stored order with the decoded actions, batches per material, and draws with the
  two ported shaders (forward, premultiplied / additive). Tyre smoke per wheel from the contact patch's
  sliding speed (`drive::WheelContact`, minimal read-only addition to `drive.rs`), kick-up per rolling wheel,
  impact bursts and scrape sparks from avian contacts of the car body (`ImpactEvent`). `--screenshot-time`,
  `--screenshot-after-impact`, `MW_VFX_DEMO`, `MW_VFX_DEBUG` for checks.
- **Stand-ins / guesses** (each marked in the code): emission = spawner rate × controller curve 0 over
  `Intensity` (sliding speed / 15 m/s) × camera-distance LOD + per metre of sliding; quadratic drag
  (`v /= 1 + drag·|v|·dt`: smoke stops within metres; a linear reading carried it 25 m with the car); spawn
  directions as azimuth / elevation from the first two angle ranges; collision sparks spread over ±0.3 m
  (the game spreads them over the collision mesh); `SPARK_BRIGHTNESS` 0.15 for `gVfxSparkBrightness`
  (without it sparks tonemap to flat white); streaks over 1/30 s; the irradiance cube for the quadric;
  always the dry tarmac surface.
- **Pitfalls**: avian reports one contact pair per track-unit trimesh that stays touching (speculative), so
  `collision_started` never fires on impact; impacts are detected by closing speed (> 3 m/s, 0.25 s cooldown).
  The spark quads are built in y-down pixel space and were back-face culled (culling is off for particles).
  Empty meshes upset Bevy's mesh allocator ("Use-after-free … unallocated key"): batches keep one invisible
  quad.
- **Verification**: every effect parses (`mwtool verify`, corpus test `bear_effects_decode`: 214 effects, 572
  layers, 415 quad / spark renderers); `mw-assets` resolves every table entry and material. Screenshots:
  handbrake and drift tests on the test track (smoke clouds behind the rear tyres), a crash into the guard rail
  west of downtown (`--auto-throttle 1 --screenshot-after-impact 0.04`: spark spray and dust),
  `MW_VFX_DEMO=010000000007bd97,0.25,15` at street level (orange specks and a white-hot streak).

### 2026-10-04 (audio session): the engine plays the car's whole sound bank
- **Bindings are strings.** `CgsCore.StringBase` = `{i32 offset from the field, u32 length incl. NUL}`; the
  "raw" binding values in earlier dumps were these. Sequences bind named game values:
  `Players.LocalPlayer.GetPlayerAudibleRPM`, `GetPlayerEngineLoad`, `RaceCars.EngineSound[#].EngineOn`,
  `GetPopProbability`, `RevLimitGain`, `TrackedHarmonic1Frequency`, `SubharmoniserFrequency`,
  `GetPlayerNormalisedGearChangeTimeElapsed`, `LatSlipIntensityForSkidSound`, `GetSpeedMph`, ... with optional
  tests (`(NOT)`, `(GTEQ:100)`, `(BETWEEN,20,60)`).
- **Field names cracked** (python CRC brute force over sound vocabulary, each checked against the values; added
  to `genesys_names.txt`): `StartTime` / `EndTime` (item range on the sequence axis), `Fade_Out`,
  `Binding_Max`, `Binding_Exponent`, `Binding_Invert`, `Binding_Evaluate_Once`, `Binding_Range_Min/Max`,
  `Output_Value_Min/Max`, `Binding_Modulation_Type`, `Animation_Modulation_Type`, `AnimPointCount`,
  `TangentIn/OutX/Y`, `WavesCount`, `Trigger_Probability(Count)`, `Double_ParamsCount`, and in EngineSound2
  `Ignition_Sequence`, `Pop_Sequence`, `Backfire_Sequence`, `Gurgle_Sequence`. Dead ends: TyreSoundParams
  fields (5,000-word 3-word search and `genesys-crack` at 4 words found nothing trustworthy); 3-word hits with
  suffixes are mostly CRC collisions (`Down_Lift_Soft_Chance`).
- **Two array-count bugs in the decoder**: a WaveSequenceItem's `Waves` showed 0 items (their count is the
  second trailing u8, `WavesCount`, confirmed by array spacing), and EngineSound2's twelve handle lists were
  read with overlapping lengths (Burnout 18, Drift 18, Sequences 3). Handle arrays sit back to back, every slot
  with a handle import, so the decoder now takes such a run up to the next array as the length: Burnout 1,
  Drift 2, In_air 4, **Sequences 6**, channels 14. VehicleSound's lists change the same way (Player_Sequences
  12). The engine had been playing 3 of its 6 always-on sequences.
- **Sequence semantics (from the data; formats.md)**: items fade in/out *inside* their range (wind layers
  45–90 / 70–138 / 100–250 mph with 20–40 mph fades crossfade exactly); a modulating value maps its binding
  (or, without one, the item's normalised position) through its curve onto `Output_Value_Min..Max`, else is
  `Value`. Load curves therefore scale up to 2× and 4× (the earlier code read the curve y as the gain).
  Assumed: the two modulation-type enums are ignored (frequency automations with `Value` 24000 and either type
  only make sense as "output replaces"); curves are piecewise linear.
- **The engine's mixer graph is data.** Each `MixerChannel` has a gain, a mixing group and a plug-in chain;
  `SendDspPlugIn.Voice_Routing` sends to another channel, a `SubmixDspPlugIn` collects channels of the groups it
  lists (the idle loop reaches the set-A bus this way). 911: set A → submix → {EQ (100 Hz ×5, 6.5 kHz ×2), EQ +
  distortion 4} → vehicle bus; set B → submix → {EQ 2/6 kHz, band-pass + sub-harmoniser + low-pass, delay} →
  {EQ, EQ + distortion 1–4 by load} → second bus + sub-harmoniser bus. `MixerChannelSequenceItem` parameters:
  `PlugIn` 0 + `Parameter_Name` 0 + `Automated_Property` 0 = channel gain; a plug-in unique id + a field hash
  (`Frequency` = 0x06096291) + 9 = that plug-in's field; `PlugIn` 0 + field hash + 9 = the channel's first
  plug-in with that field (the load curves drive the strain channels' 1 kHz peak gain and distortion amount).
  The two vehicle buses send to each other; the player drops sends that close a loop.
- **New runtime** (`mw_audio::sequence` + `dsp`, built by `mw_assets::sound`, driven by
  `mw_assets::engine_driver`): every sequence of the bank, Ginsu items in one shared period schedule, wave items
  (loops vs one-shots by the SPS loop flag), the graph with RBJ biquads, `tanh` distortion, a flip-flop octave
  divider, delay. Engine bank = `Sequences` (RPM ×2 with idle loops, load crossfades, speed, normalised RPM bus
  automation), Gurgle (pops, retriggered at a rate ∝ pop probability), Backfire / Ignition / rev-limiter cues,
  one upshift and one downshift gear-change group (by content: the upshift group has the LFO shunt and power
  cut). Our rules (assumed): audible RPM = physics idle..redline mapped onto the RPM sequence's axis
  (1790..7500 on the 911, so idle plays the recorded idle loop and the Ginsu fades in over 100 rpm); load =
  throttle, cut for one normalised shift time (0.25 s); pops armed by lifting off above 45 % revs, fading over
  0.6 s; backfires on 35 % of lifts above 75 % revs and half of full-throttle upshifts above 60 %; tracked
  harmonic = audible rpm × the recording's strongest half-order (Goertzel over grains: 911 3, Mustang 4, GT-R
  3, Veyron 4); per-car loudness normalised (raw bank levels spanned 24 dB, Mito −31.5 to GT-R −7.6 dBFS on one
  sweep, because the mixes end in buses outside the vehicle bundle; now −16.7 to −11.2).
- **Measurements** (911, `mwtool engine-sound`; scripts in the session scratchpad): 4 s rev + 4 s coast:
  pitch vs the order-3 firing frequency median −12 cents, p90 72 (old six-layer mixer: 98); peak |diff|/RMS per
  2 ms ≤ 1.0 (old 1.2); limiter 0.002 % of samples; max 10 ms loudness step 6.3 dB. Off load is now ~8 dB
  below full load (old mixer: same level), idle ~15 dB below full throttle (the data's idle loop gain 0.3). The
  plug-ins add +4.9 dB at 63 Hz and +2.7 dB at 500 Hz and −1.3 dB at 250 Hz against `--dry`. Recorded game
  trace (`--test engine`): steady segments 6–47 cents p90; worst windows are the idle loop (fixed pitch by
  design) and shifts. Corpus test `mw-assets/tests/engine_banks.rs`: all 73 engine banks render finite,
  audible and without hitting the limiter. Discontinuity peaks left are real transients (ignition starter,
  backfires).
- Open: the state lists (Burnout, Drift, In_air, Reversing) and which gear-change list is which; per-car
  DspParam overrides (parameter indices unknown); `BusMixerChannelSequenceItem`, `LfoSequenceItem`,
  `RaceCarEffectSequenceItem` (camera shake?) not played; the true plug-in formulas; the game's audible-RPM and
  engine-load definitions.
### 2026-10-04 (camera session): VehicleCameraContainer decoded, camera.rs
- **Camera types** (`mwtool genesys-types | grep -i camera`): the container holds `CameraGameplayExternal`
  (392 bytes: six sub-blocks, four vec3s, FOVs, a handle to `CameraGameplayShakeEffect` and pointers to
  `CameraGameplayExternalGlobals` (`Impact_shake`, `LensProperties`, a shake effect) and
  `CameraGameplayGradientAdjustments`), `CameraGameplayBonnet` / `Bumper` (128 bytes, same layout) and
  `CameraRearView` (+ `CameraRearViewGlobals`). All 79 containers in the install read
  (`crates/mw-assets/tests/cameras.rs`); 30 of ~150 fields vary between cars (eye, look-at, speed movement,
  roll and pitch amounts, shake speed, in-car eyes), the rest are global constants.
- **Field order is alphabetical** within a size group (case-insensitive): every named camera field is in
  order (`Maximum_Roll_Angle` < `Minimum_Angular_Velocity_for_Roll` < `Minimum_Lateral_…` <
  `Roll_Angle_Change_Intensity`; `Amplitude` < `Damping` < `Maximum_Angle` < `Minimum_Angle` <
  `Spring_Coefficient`). That bounds an unnamed field's name and was used to accept or reject hits.
- **Names cracked** (python templates with mixed capitalisation, plus `mwtool genesys-crack`, which gained a
  `title-small` style: Title_Snake with `for`/`to`/`at`… in lower case, the style of
  `Minimum_Lateral_Acceleration_for_Roll`): pitch `Maximum_` / `Minimum_deceleration_for_pitch_change` (12 / 2),
  `Pitch_change_curve_power`, `Pitch_change_intensity`; roll `Minimum_Lateral_Acceleration_for_Roll` (10),
  `Minimum_Angular_Velocity_for_Roll` (0.2), `Roll_Angle_Change_Intensity`; shake `Minimum_` /
  `Maximum_Speed_for_Shake` (0 / 180 mph, the 911's top speed is 189); shake springs `Spring_Coefficient`,
  `Minimum_` / `Maximum_Angle`, `Lower_` / `Upper_Translation_Limit`; `Maximum_amplitude`,
  `Maximum_frequency_multiplier`, `YawDriftSpring`, `Maximum_Gradient`; mirror `Horizontal_FOV`,
  `Aspect_ratio`, `Near_` / `Far_clip_plane`, `X_axis_rotation`. Plausible without a confirming sibling:
  `Camera_Roll_Factor_Power`, `ShakeMagnitude`, `WobbleMagnitude`, `AccelerationResponse`, `FocalDistance`.
  Also added the names handling.rs already hashed but genesys_names.txt lacked (`MinFieldOfView`, …), so the
  dumps now show them.
- **Units**: the thresholds only make sense in mph/s: `Minimum_Lateral_Acceleration_for_Roll` = 10 m/s² would
  mean no roll below 1 g; as mph/s the roll range is 0.46–1.37 g and the pitch range 0.09–0.55 g.
- **Dead ends**: (1) the two roll maxima (`02dd11bc` 2.5–3, `3f8ced67` 30–40) track the minima's meaning and
  sort right, but `Maximum_Angular_Velocity_for_Roll`, `Maximum_Lateral_Acceleration_for_Roll`, all one- and
  two-character edits of both (13.9 M candidates), `…_for_Maximum_Roll`, `Camera_Roll_Maximum_…` and Max/Full/Peak
  variants miss. (2) No `Acceleration_movement`, `Speed_based_movement`, `Yaw_spring_modification`,
  `Speed_based_height_change` or bonnet field cracked with 4–5-word vocabularies (snake, Title, title-small,
  Camel); only nonsense hits (`Vertical_Speed_Impact_Mph`, `Low_at_Min_Z_Lateral`, …). (3) No look-around /
  free-look fields (Look, Free, Orbit, Return, Recentre, Yaw, Pitch, Limit, Speed templates). (4)
  `CAMERAS.BNDL` holds 45 `CameraTake`s named after event outros, intros, busted and "FinalCam" shots
  (`Sprint_Race_Event_Outro_ICE`, `Busted`, `EMP_Zoom_Hit`): cinematic cameras, not gameplay. (5) A web search
  found no community notes on these types.
- **Bonnet / bumper**: the bumper eye (0, 0.425, 1.5) is inside the 911's nose (its bumper is at z ≈ 2.2), so the
  car must not be drawn in that view; their FOVs (95→105, 95→120) look very wide when read as vertical.
- Implemented in `crates/mostrusted/src/camera.rs` (moved out of drive.rs): yaw spring per 60 Hz tick from
  the speed band (and `YawDriftSpring` with a lean towards the velocity while drifting), speed-based eye /
  look-at movement, an acceleration spring (eye trails by a / ω², clamped to −2 / +1.5 m), roll and pitch from
  the named thresholds and curve powers, high-speed shake, impact springs kicked by body-collider contacts,
  a ray against the world so the eye stays in front of walls, look-back, look-around (right stick, right
  mouse drag) that springs back, the bonnet and bumper modes (C / gamepad Y), `--camera-mode`,
  `MW_CAMERA_TRACE`. Table and results in handling.md ("Cameras").
- Open: the in-car cameras' 19 floats (they are rigid for now), the remaining acceleration and yaw-spring
  fields, `Speed_based_height_change`, `Gradient_Adjustments`, whether the FOVs are vertical (compare
  with the original; `mwtool memscan` could find the camera's position and FOV in the running game).
### 2026-10-04 (weather): status / next steps
- Works: runtime environment (`weather.rs`: painted timeline under the camera, fractional hours, F5/F6/F7,
  `--weather`, `--timeline`), road wet path (`--road-wetness`), opt-in planar reflection
  (`--planar-reflection`), `mwtool paint-map`. fmt / clippy (0 warnings) / tests / `mwtool verify` (0
  failures) pass.
- Integration note: the planar reflection adds a second `Camera3d`; every main-camera query must use
  `crate::MainCamera` (done in main, drive, garage, damage, weather). New queries (e.g. the camera agent's
  `camera.rs`) need it too before the reflection can be on by default.
- Pre-existing, not from this work: the daytime sky renders black in the main view (the mirrored view,
  which skips the game tonemap, shows it blue), so the game tonemap / grade likely crushes it.
- Next: make the planar reflection default once camera queries are merged; give it a mip chain; tune
  `WetScaleParams`; port `PlotPBR_AO_StandingWater` and `WorldPBR_Horizontal_PlanarReflection`; tree sway
  (needs a `Layered` vertex shader); blend when the painted timeline changes; LayerFog from its paint map.

### 2026-10-04 (weather): what the data has for weather, timeline paint map, wet roads
- **No rain.** No precipitation exists in the PC data: no GeneSys type or field with rain / precipitation /
  wetness vocabulary (12,410 type rows searched), no rain / drop / splash / lightning shader (`mwtool shaders`),
  no such texture in `ENVIRONMENT.BNDL`, `GLOBALEFFECTS.BNDL`, `GLOBALTEXTUREDICTIONARY.BNDL`,
  `HAWAII/GLOBALRESOURCES.BNDL`, no rain string in `GLOBALEFFECTS` effects, and the `Weather` block holds only
  floats (no effect or texture handle). A rain effect would be an invention, so none is drawn.
- **`Weather` block** (cracked with weather vocabularies + `/usr/share/dict` words, accepted only where the
  values fit): `WindBearing` 130, `WindBearingVariability` 25, `WindSpeedMin` 1, `WindSpeedMax` 3.5 (all
  175 keyframes), `Thunderstorm_Intensity` (0.23 on the 42 night keyframes, 00:00–04:00, else 0), `0141d842`
  (0.25 on daytime keyframes, 0.15 in the evening, at night and on every Alternate), `e8eb5d48` (0).
  Also cracked: `SparkExposure` (Vfx: −3 night, 0 morning / evening, 0.75 noon), `DistanceBlend` (HeatHaze,
  100), and five array counts that equal their arrays' lengths (`ColourSunCount` 5, `SkyColorGradientCount`
  30, `ScatteringColourCount` 3, `TintColourCount` 4, `LensFlareColourCount` 5). Rejected as flukes
  (two-word hits that don't read as names or don't fit): `NearPlane`, `CapSun`, `AnimateSpeed`,
  `ExposureDefault`, `IntensityStandard`, `BrightnessAdjustment`, `BrightnessOverride`.
  `655d3f5f` (`CgsCore.UniqueId`) is the object's own ID low bits at top level and 0 in sub-structs.
- **Alternate keyframes**: 154 of 175 keyframes name one of only 7 Alternates (the earlier "seven keyframes
  have Alternates" was the other way round): night 00–04 (`…124683`), dawn 05–06 (`…1d3c66`, `…12466e`),
  morning 07–10 (`…1d3c62`), noon 12–15 (`…1d3c65`), evening 17–23 (`…1d3c63`), and `…f083a` for the
  front-end timeline `…9efca`. Hour 04 keyframes have none. No timeline lists an Alternate. They are hazier:
  fog far 550–1200 m (vs 1500+), key light ×0.6–0.8, brighter shadow fill, their own colour cube, cloud
  `LightColour` 1.72 → 1.0. Nothing in the data selects them; `--weather` / F7 switch to them.
- **Which timeline lights a place: `WorldPaintMap` (0x91).** `EnvironmentSettings` fields `409f546f` /
  `ae067627` hold the low bits of `…2823d` / `…285cb` in `ENVIRONMENT.BNDL`: quadtree maps over 1 km cells
  painting the 25 timelines and the 19 `LayerFog` objects (formats.md). `VfxGlobals` `5e608031` names a third
  (`GLOBALEFFECTS.BNDL` `…285cc`, one `ParticleLighting`). Cell = floor(x / 1000) + 3, floor(z / 1000) + 5 for
  the timeline map: checked by drawing the 169 zone outlines over it (`mwtool paint-map … --png`): region
  borders follow zone edges and the river, and every zone is painted. The drive spawn is under `…1245c8`,
  downtown under `…1245c6`, the north-west under the default `…69cbb`. Timelines repeat a keyframe over
  consecutive hours (00–03, 05–06, ...), which only makes sense with interpolation, so hours now blend
  linearly between entries (provisional).
- **Wet roads are always on.** `DriveableSurface` main pass decoded: t5 = RoadDepth (material), t6 =
  `puddleMapSampler` (engine-bound), t7 = planar reflection, t8 = env-BRDF LUT; `WetScaleParams` (global CB)
  x = puddle tiling of world xz, y = reflection distortion by (N − N_vertex).xz, z = reflection mip per
  roughness. wet = puddle.w × saturate(TEXCOORD5.x); k = lerp(1, (RoadDepth.y × puddle.x, puddle.y, puddle.z),
  wet); N = N_v + k.x (N − N_v); roughness × k.y; albedo × k.z; F0 = lerp(0.02, F0, k.x); indirect = albedo ×
  baked vertex light + planar(mirrored projection + distortion, mip) × envBRDF(N·V, roughness, F0) × wet. There
  is no global wetness input, so roads are as wet as the puddle map says, day and night. TEXCOORD5.x is a
  per-vertex mask: 0 on 22 % of TRK_UNIT1's road vertices, ≥ 1 on 78 % (max 3.9, saturated).
- **The puddle map** is `GLOBALTEXTUREDICTIONARY.BNDL` `…7811` (256², no material binds it): x ≈ 0.9 around /
  low in puddles (normal kept), y ≈ 0.07 / 0 (roughness scale), z ≈ 0.8 / 0.45 (darkening), w ≈ 0.55 / 1
  (wet amount): identified by these channels matching the shader's use (it is the only texture with them).
- `PlotPBR_AO_StandingWater` (28 meshes) runs the same wet maths with its own `PuddleTextureSampler` at
  TEXCOORD1 and no RoadDepth (not ported).
- **`SurfaceType`** has `Dry_Conditions` / `Wet_Conditions` blocks (cracked): 57 of 65 surface types give the
  wet one other tyre-mark materials, `Surface_vfx`, sounds or `VehicleEffects`. What makes a surface wet is
  unknown.
- Implemented: `weather.rs` (environment rebuilt at runtime on a background thread: F5 / F6 hour, F7
  Alternate, timeline from the paint map under the camera, `--hour` fractions, `--timeline`), the road wet
  path in `layered.wgsl` (puddle map, RoadDepth, vertex wetness mask in COLOR, vertex AO), and an optional
  planar reflection (`reflection.rs`, `--planar-reflection`: a mirrored camera through the same deferred
  pipeline, clipped below the plane; the sky cube stands in without it). Not done: rain (not in the data),
  tree sway (decoded, needs a vertex shader for the Layered material), cloud scroll, thunderstorms.
- Dead end: the first keyframe survey script overwrote entries and reported 7 keyframes *with* Alternates;
  `mwtool genesys` truncates arrays at 16 items by default, which hid timeline hours 19–23 (use
  `--max-items 40`).
### 2026-10-04 (props session): prop physics, collision surface tags — status / next steps
- **Prop physics data**: `PropObject` matches burnout.wiki's `Game::World::PropObject`: 0x0C
  `mPhysicalDefinitionHandle`, 0x14 impact effect, 0x1C impact sound, 0x24 `mfResetDistanceSquared`
  (10000), 0x28 `mbRemoveInsteadOfReset`. `WorldObject` / `CompoundObject` have the physical definition at
  0x1C; compound parts (ptr 0x30, count 0x38, 0x70 bytes each) are embedded WorldObjects with their own
  physical definitions (+0x1C) and part links at +0x64. All in `ObjectDef` (world.rs).
- GeneSys `PhysicalDefinition`: bounds (`62dbd683` centre, `98e902ed` half extents), `AdditionalInfo`,
  `RigidBodies` (transform `0d0c29b4`, `ComOffset`, `Inertia_Scale`, `Mass`, `Friction`, `44ac37c1` probably
  restitution, `LinearDrag`, `AngularDrag`, `BoxVolumes` (transform `d084ac7c`, half extents `3727fead`),
  `CylinderVolumes` (`Radius`, `HalfLength`, axis = local Z), capsule / sphere / convex-hull lists).
  New names (genesys-crack, single candidates): `HalfLength`, `BoundingConvexHull`, `ComOffset`,
  `ForceToDamage`, `ForceToSmash`. World objects use only boxes (422) and cylinders (12). `mwtool props`.
- `DynamicAdditionalInfo` / `CompoundAdditionalInfo`: `ForceToDamage`, `67f0233c` (unnamed; 0 cones/bins,
  500–2500 signs/fences, 5000 street lamps, 20000 gantries, 100000 on mass-0 objects), `Strength`, `Score`,
  compound `ForceToSmash` (10000 on street lamps). Read `67f0233c` as the break-free threshold, units
  assumed impulse N s (m_hitter × approach speed: lamps break above ~12 km/h for a 1.5 t car). Hypothesis.
  Dead end: guessing `ForceToMove/Break/Detach/Topple/...` for `67f0233c` (no hit with ~150-word lists).
- 112 prop, 52 compound, 271 dynamic definitions (55 dynamics with physics; 3120 dynamic placements have
  none). Mass 0 objects have default-looking values (friction 0.1, drag 0.1) → treated as immovable.
- **Collision tags** (burnout.wiki `muCollisionTag`, but the PC bit order is reversed vs the wiki's PS3
  table): as LE u32, flags bits 0–9 (DRIVABLE 1, ROAD 2, KERB 4, ..., FATAL 0x40), surface index bits 13–19,
  track unit bits 20–31 (all 1,652,030 triangles carry their own unit number). Surface index → the
  65-entry `Surfaces` list in `HAWAII/SURFACELIST.BNDL` (`SurfaceType`: debug `Colour`, per-contact
  `Surface_Physics` with `Surface_Grip` 1.0 asphalt / 0.75 dirt / 0.7 grass, sounds, vfx). No surface
  names in the data. First read the wiki's order literally: surface indices up to 90 (dead end).
  `mwtool surfaces`, `mw_assets::surfaces`, test `crates/mw-assets/tests/props.rs`.
- **Runtime** (`crates/mostrusted/src/props.rs`): props/compounds/dynamics with physics get a compound
  collider (loader thread), pinned static until a contact hook sees impulse ≥ break force, then dynamic with
  the inelastic momentum exchange; reset at 100 m. Layers: World / Car / CarBumper (low box that only meets
  props) / Prop; wheel rays see only the world. Damage scales loose-prop contacts by m_prop/(m_car+m_prop).
  Colliders are streamed: only props within 120 m of the car carry a body (removed past 160 m).
  `CarSurface` component = surface under the car (via `WorldSurfaces::surface_at`).
- Verified: `--test smash` at 60 km/h knocks cone, street lamp and fence, car stops at the immovable wall
  (z 167.8 vs wall 170); at 8 km/h the lamp holds and the car stops against it. World drive from the
  default spawn knocks a street lamp and the roadside crash barrels / tyres; screenshots looked at.
- Measurements (noisy, shared machine, load average 5–14): full map 17,948 placements with physics; load
  24.6 s vs 18.5 s with `--no-prop-physics` (the decode is per definition, so most of the gap is probably
  noise, re-measure); frame time with all 18k static bodies ~27–29 ms vs 13–27 ms without — streaming (50
  active) did not clearly change it under that load. Needs a clean A/B (`--frame-stats`), ideally timing the
  physics schedule alone.
- Next: clean perf A/B; compound parts as separate breakable bodies (need part→mesh mapping, maybe the
  skeleton / locators); `67f0233c` name; use `ForceToDamage` / damaged model; hook surface grip into the
  tyre model and skid audio; a knocked lamp leaning on a slope can wedge the car (realistic, but check).
- Merge (2026-10-04, integration branch): prop physics is **opt-in** (`--prop-physics`; `--test smash` turns it
  on). On master's physics the default world drive (`--drive 911 --units 12 --auto-throttle 0.4` or 0.6) knocks
  the street lamp at z 1714, then stops dead against it and the crash barrels at z ~1722 (0 km/h at 0.6
  throttle). `--test smash` at 60 km/h now pushes the cone and the knocked lamp ahead of the car and stops at
  the fence (z 117.7, fence standing: its contact is car → lamp → fence, so the hook never sees the car's
  impulse); on the props branch alone the car was deflected after the lamp and drove past the fence. The
  "Verified" line above no longer holds on either. Frame time on the full map (free camera, vsync-capped):
  16.6 ms mean with and without prop physics, a few more spikes with it. The chase camera's wall ray and the
  planar reflection's ground ray now use the World layer only (the bumper box would otherwise block them).

### 2026-10-04 (session 2): NewSky shader read
- Main PS (`…79648d36`): glow = pow(saturate(dir · KeyLightDir), SunColour.w) × SunColour.rgb (exponent 0.65 at
  noon: a broad glow over the sun's half, not a disc); gradient = lerp(B(|y|), A(|y|), w) × BaseCloudColour.w
  with w = saturate(dot(norm(sun.xz), norm(dir.xz)) + DomeRanges.w) from the VS; four cloud layers, each
  lerp(sky, cloud.rgb × BaseCloudColour.rgb, saturate((cloud.a − CloudParamsPS[i]) × CloudParamsPS2.y)); output
  sqrt(colour / 4). VS: per-layer uv = (−TEXCOORD.y × s + o, −TEXCOORD.z × s' + o' + 1) from
  `BaseCloud{A..D}ParamsVS`; horizon fade 1 − ((TEXCOORD.x − DomeRanges.x) × DomeRanges.y)^DomeRanges.z.
- Ported: the glow (the old power-500 × 1.067 glow is gone; `2ec8df83` / `2ec8df82` are probably the sun
  disc, a separate draw). Not decodable from shaders: gradient B, the gradient scale BaseCloudColour.w and the
  per-layer cloud parameters (CPU-side). Dead end: `SkyReflectionAverageColour` / gradient hemisphere average
  ranges 0.3–5.6 over the day, so it is not a fixed multiple of the gradient.

### 2026-10-04 (session 2): sun and moon path
- `EnvironmentSettings` (`…69cad`) `SunMoonPath`: `BearingCentre` −232.37 (cracked), `BearingVariation` 35.06,
  `BearingPhaseOffset` −0.5; unnamed `0bb64ed2` 50, `c915f96d` 40, `1dc20d63` 28, `ffbdcf8b` 25,
  `ac4fe424` 35, `138500fe` 0, `a5caac0f` 8, `644383ee` 20 (crack with sun/moon/elevation/time vocabularies:
  no further hits). `CarSelect` blocks hold KeyLightBearing / KeyLightElevation for the garage scenes;
  `DayNightTransition` `1ab46d30` 8.5, `0e2be130` 20.
- Model (`SunMoonPath::key_light_direction`, provisional): bearing = centre + variation × sin(2π(h/24 +
  offset)) (centre at 12:00, ±35° over the day); sun elevation a sine arc 05:00–21:00 peaking at `0bb64ed2`
  (the default timeline's key light switches on at 05:00 and peaks at 13:00); moon from `644383ee` (20) to
  `a5caac0f` (8) peaking at `c915f96d`. **Unknown: the bearing's reference axis and direction**; assumed
  clockwise from +Z towards +X (noon: towards +X −Z). Needs one reference screenshot from the original
  (shadow direction at a known place and hour) to settle.
- The sky model's darkening away from the sun (gradient position, research log "deferred pipeline") is now
  visible with the sun behind the road-cliff camera.

### 2026-10-04 (session 2): the game's tonemap, auto exposure and colour cube
- **Light buffer units**: `LightBuffer_KeylightAndAmbient` adds RT3 × HDRConstants.y / Tonemap.w, undoing the
  exposure the G-buffer pass applied, so the light accumulation buffer holds plain game HDR units.
- **Tonemap** (`GBufferComposite`, both variants): hdr (6-tap motion blur) + bloom × B.w × B.rgb ×
  saturate(bloom × T.w), × (A.rgb × bloom.a + D.rgb), × T.w, then 1 − 2^(−x) and a polynomial sRGB fit
  (√c·1.271696 + (0.070706c − 0.317854)c − 0.022711). T.w (1×1 tonemap texture) = log2(e) × 2^(e + C0.z)
  (`_BuildConstant`), so display = sRGB(1 − exp(−hdr × 2^(e + bias))). DXBC `exp` / `log` are base 2.
- **Auto exposure** (`Blit2d_AutomaticExposureMeter`, `_AverageLuma`, `_AverageLumaFinal`): meter = luma
  (0.299, 0.587, 0.114) of the displayed colour under the current exposure; 16-tap averages down to one
  value L. State (e, L̄): L̄ = prev + 2^(−7.213475|C1.w|)(L − prev); scene = log2(1 − L^2.2) / −2^e;
  target = lerp(C2.y, C2.x, saturate((scene − C2.w)/(C2.z − C2.w))); e steps by |C1.w·C1.z| towards the
  target outside a ±C1.y band (proportionally inside), clamped to [C0.x, C0.y]; C0.w ≠ 0 fixes e = C0.x.
- **Keyframe Camera fields** (survey of 188 keyframes + the default timeline by hour): exposure range
  `3bec5831` / `3bec5027` (−8…8; noon −0.2…3, night 2.7…3.5), `ExposureCompensationBias` (cracked; 3 at
  night, else 0), scene-luminance range `4b48cdce` / `ec890b00`, luma targets `9355acc0` / `8357903e`, band
  `0abf68c5`, speed `e59ff4ec`. min ≤ max holds on every keyframe for both pairs. Dead end: the first reading
  (`4b48cdce`/`ec890b00` as the exposure range, pinned at 2 at noon) gave E = 4 and washed-out frames; with
  the S-curve grade on top it is clearly too hot. Also cracked: `DarkBloomWeight`, `DarkBloomWhitePoint`.
- **ColourCube (0x52)**: 16-byte header (N = 16, N, 0, 0) + N³ RGB8, blue fastest; `…3e93` is the identity.
  13 in ENVIRONMENT.BNDL, 8 in POSTFX.BNDL. Gamma in, gamma out. No PC post shader samples it except
  `UIRearViewMirrorShader` / `UIPersistentImage` (cube(saturate(rgb × ColourTintAndBloomHeadroom) × scale +
  offset)); the main view's use is assumed to be the same. The noon cube is a contrast S-curve (grey 51 → 41,
  153 → 192) with a warm cast.
- Implemented (`tonemap.rs`, `tonemap.wgsl`): meter (32² grid, 4×4 taps each), adapt (ping-ponged 1×1 state),
  tonemap + grade, replacing Bevy's tonemapping; the camera exposure makes the HDR target hold game units.
  Not yet: bloom, the composite tint (A, D), motion blur, PostAA, ColourTintAndBloomHeadroom.

### 2026-10-04 (session 2): key-light shadows (CSM lookup ported)
- `LightBuffer_KeylightAndAmbient` shadow lookup decoded (`gGlobalCB`: `ShadowMap_Constants{,2}`,
  `ShadowMap_ScaleOffset[3]`, `ShadowMap_WorldToLight0`, comparison sampler s15 on `shadowMap0` t15):
  - One atlas, three cascades stacked vertically: cascade i at uv = ls.xy × (0.5, −1/6) + (0.5, (2i + 1)/6).
  - Cascade 0 is `WorldToLight0`; cascades 1 and 2 reuse it with an xy scale + offset (`ScaleOffset[0].xy` /
    `.zw`, `[1].xy` / `[2].xy`) and a z offset only (`[1].z`, `[2].z`): same orientation and depth scale.
  - Selection: the first cascade with max(|x|, |y|) < 0.99; outside all three the reference depth is 0 (lit).
  - One `sample_c_lz` tap (2×2 hardware PCF), then lerp(C2.y, s, fade) with fade = saturate(C.w − view depth ×
    C2.w)², then squared, then × saturate(1 − 0.8 (1 − AO)).
  - Casters: `WorldShadow_Opaque_{Single,Double}sided` (white PS, position-only VS with
    `ViewProjectionModified`) are shadow-only proxy meshes; the loader still skips them (`skip_shader`). Vehicle
    and character shaders carry a `MaterialShadowMapBias` parameter (caster-side bias).
- Implemented: Bevy renders three directional cascades (2048², splits 20 m / … / 180 m, fade over the last 30 m:
  provisional, the game sets them from code) and `game_lighting.wgsl::game_shadow` samples them with the game's
  selection, single tap, fade and square. Bevy's receiver normal / depth bias stands in for the caster bias.
  Groundcover casts no shadows.
- Observation: shaded areas are much darker than in the game. Noon rig: key 11 units vs up fill 0.065 × 2.6;
  with only the rig cube as ambient the ratio is ~20:1. The game's RT3 indirect term also carries lightmaps and
  vertex light, which are not bound yet (M1.4.7): probably the missing sky light in shade.
### 2026-10-04 (vehicle lights and glass): VehicleNFS13 light, lens and glass shaders ported
- **Usage survey** (`mwtool models <car> --verbose` over all 74 `VEH_*_HI` bundles): besides wheels and
  body, the light / glass families are `Glass` (140 meshes), `Glass_Colourise` (118),
  `Body_Textured_NormalMap_Lightmap` (77) + `_Licenseplate` (73), `Glass_Textured_Lightmap` (73),
  `_Emissive_NoDamage_NoEffects` (72), `_EmissiveFourChannel_NoDamage_NoEffects` (71), `Refraction` (62),
  `BodyPaint_Lightmap` (31), `Body_Lightmap` (7), `LocalEmissive` (5, not ported).
- **`g_selfIlluminationMask`** (gObjectCB, set per car at runtime) switches the lights. In `Refraction` it
  multiplies the emissive texture's rgba against Brake / Headlight / Reversing / Taillight colours, so
  x = brake, y = headlights, z = reversing, w = tail. The lightmapped shaders use `mask.xyzy` against the
  Red / Green / Blue / Alpha channel colours: red = brake (911: 909 × 10, a huge HDR value), green =
  headlight glow, blue = reversing, alpha = tail glow, which also follows y. Ported as `--lights
  brake,headlights,reverse,tail` (default `0,1,0,1`); driving sets x from the brake pedal and z in reverse.
- **Lightmapped lights**: `EmissiveFourChannel`: sat(LM(uv0)² × mask.xyzy) × mSelfIlluminationMultiplier,
  dotted with the four channel colours, added to RT3. Its albedo is `PbrMaterialDiffuseColour` (no diffuse
  sampler), roughness the normal map's alpha. `_Emissive_`: sat(LM(TEXCOORD3).y² × mask.y) × multiplier ×
  green colour. It does **not** normal-map (o0.xyz = vertex normal) and its roughness is the normal texture's
  **green**. `Lightmap` variants (body, licence plate, paint): the same green term × the surface albedo after
  the coat. Roughness = `PbrMaterialRoughness`, F0 = `PbrMaterialSpecularColour` (the normal map's alpha is
  not used, which the old code got wrong). The lightmap UV is TEXCOORD3 for all of these except
  EmissiveFourChannel (TEXCOORD0). Emission is in the game's HDR units, so it is scaled by `GAME_UNIT` like
  the sky.
- **`Refraction` is an opaque G-buffer pass** (the old code drew it translucent grey): it renders the lamp
  interior. The eye ray in tangent space (VS output from TANGENT / BINORMAL / NORMAL), bent towards
  −(external normal map) by ParallaxConstants.z, offsets the UV by height = displacement.x × P.x + P.y (uv +
  (d.x, −d.y) × h). At the offset UV: internal normal map → RT0 normal, displacement.z → roughness,
  displacement.y → AO, `ColourSampler` → F0, Fresnel = lerp(colour.x, 1, PbrMaterialFresnel), albedo =
  `PbrMaterialDiffuseColour`. RT3 holds only (sharp cube(reflect(eye ray, n)) × ClearcoatSpecular + LED
  emission) × lerp(colour, mExternalGlassColour, its w) × AO, without the vehicle flag. On the 911,
  mExternalGlassColour = 14.1 × white, so the reflector is bright. The tangent sign for these float-frame
  meshes now comes from the stored binormal (`convert::mesh`).
- **`Glass_Colourise`** is a multiply-blend lens filter: the VS outputs lerp(1, MaterialColour_SimpleMultiply,
  g_PerVehicleFog.w) and the PS writes it with alpha 1. This is what makes tail lights red (911: (0.83, 0,
  0.009)). Ported with `AlphaMode::Multiply`.
- **`Glass*`** (forward, premultiplied, decoded for the clean / dry / uncracked path; `GenericVector4_A` =
  dirt, dust and wetness amounts at runtime): the volume colour is the **vertex colour** (Textured:
  lerp(vertex colour, diffuse, diffuse.a)), replaced by `DebugOverride_GlassVolumeColour` when its w > 0
  (artists use this: the 911's lamp glass has w = 0.05, an R8 piece w = 1). F = OpacityMin + (1 − OpacityMin)
  × FresnelFactor × (1 − N·V)⁵. colour = volume.rgb × volume.a × (key × N·L × shadow / π + irradiance) ×
  (1 − F) + lerp(cube(R), EnvironmentSpecular(R), SurfaceSoftness) × F, alpha = volume.a × (1 − F) + F,
  plus the running light LM(uv0).y × mask.y × multiplier × RunningColour (not squared here). Windows have
  vertex alpha ≈ 0.7 with near-black rgb (dark tint); headlight covers have alpha 0 (clear but for Fresnel).
  The diffuse term uses Bevy's forward lighting for now.
- Dirt / dust / scuffs are runtime-driven (`GenericVector4_A/B/C` × Effects texture). With them at 0 the
  body shaders reduce to the formulas above. Wheels: `Wheel_Textured_Roughness` keeps the vertex normal and
  reads roughness from the normal map's alpha.
- The tyre shader samples Diffuse / Specular / Normal and their Blur variants, but tyre materials bind only
  Effects / BlurEffects. Those textures probably come from somewhere at runtime; unresolved.
- Vehicle vertex colours now reach the shader as Bevy's COLOR attribute (UV_1 carries the lightmap UV), so
  the vehicle branch rebuilds its albedo from p3 × diffuse texture instead of Bevy's base colour.
- Checked visually: 911 (tail outline glow, red brake LEDs with `--lights 1,1,1,1`, projector headlight),
  Audi R8 GT (red tail clusters, LED DRL dots glowing at night), Veyron (round red tail lamps, LED bars).
### 2026-10-04 (damage): car deformation is skeletal animation, one clip per impact zone
- Started from PLAN M6.4's guess of a `DeformationModel`-like resource (Burnout Paradise's 0x10004). **Dead
  end:** vehicle bundles have no such type. Their resource types: Renderable, Model, Material, Texture,
  GeneSys, Wave/Ginsu, ConvexHull, PolygonSoupList, VehicleGraphicsSpec, VehicleSound, **Skeleton,
  AnimationList (×2), Animation, HSMData**.
- GeneSys types with damage in their name: `VehicleDamageBehaviour` (+ `.Bodypart`, `.DamageSequence`,
  `.SpotEffect`), `ControlMesh`, `WheelDamage`, global `ImpactDamageGraphs`, `CrashingRules`,
  `CollisionInfoDamageProfile`, `DamageBarProfile`, `LandingDamage`. `DamageSequence` handles point at audio
  / VFX `Sequence`s (sounds and spot effects when parts break), not geometry.
- **The vehicle vertex shader** (`mwtool shader-asm VehicleNFS13_BodyPaint`) is a 4-bone skinning shader:
  `gBoneMatrixArray` (48 × 3 rows) indexed by `BLENDINDICES × 3`, first weight `1 − (w.y + w.z + w.w)`.
  The `_NoDamage` shader variants drop `TEXCOORD1` and the crumple sampler; one variant reads
  `gavDamageVolumeValues` (3 × 5 bilinear grid over model XZ) for the crumple normal-map blend.
- `VehicleDamageBehaviour.AnimationList` → an `AnimationList` with 8 clips over the car's 37 bones
  (911). burnout.wiki documents `AnimationCollection` / `Anim` / `AnimSkel` for NFS MW (field names from
  PS3 debug symbols); the codec details were worked out here: rotations are 32-bit smallest-three (`0xDFF7FDFF`
  = identity; decoded angles grow smoothly frame by frame, which a wrong bit layout would not do), scales
  are one float per bone. Frame 0 is all zeros / identity, so translations are offsets from the bind pose.
- Which clip is which zone: from the bones each clip moves (bones cluster on the body: `mwtool damage 911`
  prints each bone's vertex centroid). On 50 of the 72 cars the moved bones point exactly at front, rear,
  left, right, FL, FR, RL, RR in clip order; on the rest a few clips are one 45° sector off (bones placed
  off-centre), never mirrored, so the order is fixed and the clip index is the zone.
- Posing the 911 at full front + left + front-left damage in the software preview: the bonnet buckles
  upward at its crease, the front bumper and plate disappear, the left mirror drops off. The bonnet rising
  (rather than sinking into the engine bay) also confirms the rotation sign.
- Skeleton bone IDs are not `field_hash`, zlib CRC-32 or BZIP2 CRC-32 of 1–3-word combinations of ~80 car part
  words in three casings (dead end for now; the hinge names `BonnetHinge` etc. don't match either).
- Runtime (`crates/mostrusted/src/damage.rs`): Bevy GPU skinning with one joint entity per bone; the
  body collider's contacts give the impact (avian's pre-solve `normal_speed`, speed-weighted contact
  centroid, near-vertical normals ignored as landings) → `ImpactDamageGraphs` curve → zone levels → clip
  frames. `--test crash`: the 911 into a wall at 115 km/h gets 29 % front damage (bonnet creased, plate and
  bumper pieces gone); at 172 km/h 56 %; a 35° glancing hit at 141 km/h gives 39 % front-left and 14 % left.
### 2026-10-04 (night): backdrop LOD tiers drawn on top of each other ("buildings on streets, mountains")
- **Report**: buildings randomly on streets and mountains on top of the map. **Cause**: every track unit's
  InstanceList backdrop (entries past the always-loaded count, 1,907 placements, 231 distinct models) was drawn
  for all 169 units at once. A unit's backdrop is what the game draws *while the player is in that unit*: its
  potentially visible set outside the loaded area, at the LOD that suits the distance. The same hillside is a
  22,747-triangle tile (`b2fe2fa7`) in the backdrop of units 49-53/137 and a 2,342-triangle tile
  (`28a73b07`) in that of 26 units farther away (same 1.9 km footprint, heights differ by up to 7 m); building
  blocks have a full and a LOD2 twin (`37852c3c` / `462b7b99`); 144 units list the whole-map horizon model
  `9eaa3c27` from GLOBALRESOURCES. Union = every tier stacked: coarse terrain pokes through roads and lawns,
  a beige slab covered the road at the road-cliff viewpoint, building-block stand-ins overlapped streets.
  The 2026-10-03 LOD2 strip only removed one symptom.
- **Evidence the game draws one unit's backdrop**: rasterised all main and backdrop geometry on an 8 m grid
  (temporary `examples/backdrop_raster.rs`, not committed). Only 12 % of a unit's backdrop cells fall on the main
  geometry of that unit and its ZoneList neighbours (2 % on its own), i.e. the backdrop fills the area that
  is *not* streamed. Tier pairs are listed by disjoint zone sets, and zone 137 lists the fine tile while its
  edge neighbour 48 lists the coarse one, so the game cannot draw neighbours' backdrops together.
  `--backdrop-zone 6` (only unit 6's list) at the city-lod-check viewpoint: roads clean, distant hills that
  unit 6 cannot see are missing (PVS).
- **ZoneList decoded** (`HAWAII/PVS.BNDL`, `mw_formats::ZoneList`, layout in formats.md): 169 zones = the 169
  track units (zone ID = unit number), each a 4-12 point polygon on the ground plane with district and
  arena IDs and a neighbour list (flag 3 = shared edge, mostly mutual; flag 1 = nearby, often one-way).
  `TRK_UNIT1298-1390` (district 152518) form a 3x3 grid east of the map with almost nothing in them.
- **Fix** (`mw_assets::backdrop`, default `BackdropMode::Composed`): compose one backdrop per place from the
  view of the zone that owns it. Inside a zone a backdrop triangle is drawn when that zone lists its model;
  outside all zones the models listed by the nearest listing zone win (the whole-map model only fills in);
  water meshes are composed separately (a near zone's lake bed must not hide a farther zone's water plane),
  and where a zone lists no water the nearest listing zone's water fills in. Triangles are split (adaptive,
  48 m, always above 512 m, depth 12) so tiles are cut at zone borders. Each backdrop model is sent once
  instead of once per listing unit (54,712 instead of 56,434 instances). `--all-backdrops` restores the old
  union, `--backdrop-zone N` shows one unit's list, `--no-backdrop` none. With `--units N` the zones that
  are not streamed count as outside and are filled from the nearest loaded zone's view (their stand-ins).
- Before/after (same cameras, `--all-backdrops` vs default): road-cliff: rocky coarse tile over the lawn and a
  flat slab over the road are gone; city-lod-check: terrain no longer smears over the curved road and the
  rails; south view: the lighter coarse patch on the front ridge is gone.
- Dead ends: (1) extra track units: all 169 bundles are in the zone list, none is a variant. (2) Per-model
  keep/drop: 49 of 231 models are only partly right (big terrain tiles span their own zone and far areas), so
  the decision has to be per triangle. (3) Nearest zone as the owner outside zones left holes where the
  nearest zone does not see a far tile; (4) letting water compete with terrain dropped lake water where a
  near zone lists the lake bed; (5) a 6-level split decided 600 m water pieces by one centroid (holes in the
  airfield lake).
- Remaining: cut faces where one zone's tile meets an area another zone represents differently (a mountain
  north of the airfield, `eec26d7c`, listed by 36/37/42, is cut into an arch over zones 46/55 that do not
  list it); trees in a few far places float where no zone lists ground under them (also with
  `--all-backdrops`); the river beside downtown is filled by a cheap far water plane (y ≈ 29); a few small
  triangle gaps where loaded and unloaded zones meet with `--units`. Full map load 19.4 s -> 23.9 s
  (indexing 1.7 s, the rest is triangle splitting on the loader thread).
### 2026-10-04 (drift session): tick-exact tests, drift model from `DriftBehaviour`
- **Handling tests are now deterministic.** `--test` scripting ran in `Update` at 4× time, so the input changed
  on frame boundaries, not physics ticks: 0–100 km/h on the 911 measured 3.93, 4.33 and 4.91 s on different
  runs. The script now runs in `FixedUpdate` before `car_physics`, counts physics ticks from the car's first one,
  and `car_physics` applies the scripted input itself. Two runs of every test print identical `TEST RESULT`
  lines (accel 3.92 s, brake 37.1 m, corner 1.01 g, drift, handbrake, donut).
- **Field names cracked** (one-word templates, prefix × subject × suffix, ~155k candidates against the 44
  unnamed `DriftBehaviour` hashes, so about 10⁻³ false hits expected): `Slip_angle_to_enter_drift` (8e4032c0),
  `Minimum_speed_to_enter_drift` (d71a857c), `Drift_angle_to_exit_drift` (4971b7ae), `Drift_angle_for_side_force`
  (3ca074df), `Minimum_drift_angle_for_side_force` (f6c50d59), `Drift_scale_for_maximum_side_force` (aadfc7b1),
  `Speed_for_maximum_side_force` (9e6f2943), `Drift_scale_from_counter_steering` (005c9352),
  `Low_slip_angle_for_counter_steering` (0e2a4c75), and `Mid_yaw_torque` (e324fa82, confirmed). Handbrake:
  `Brake_strength_at_low_speed` (b0bb7eaa), `Brake_strength_at_high_speed` (4af55f8a, formerly "handbrake
  torque"), `Low_speed_mph`, `High_speed_mph`, `Minimum_handbrake_time`, and the unused
  `Handbrake_grip_values.{Front,Rear}_wheel_grip_factor`. Free 5- and 6-word searches over the same vocabulary
  gave only nonsense beyond these (as expected from their false-hit rates); still unnamed: the yaw-curve
  breakpoints `2ef426dd` / `70528afa` / `a5c9496f`, trigger `8f151151` / `0dce38f7` / `b5290695`, and
  most of `Other`.
- **Survey facts** (79 `Drift` blocks): `Slip_angle_to_enter_drift` ≥ `8f151151` on every car; drift-happy cars
  have the lowest entry angles (Lancia 5.5°, 350Z 6°, M3 6.25°), AWD supercars the highest (8.5–9°). The two
  Ford Police Interceptors have `Maximum_drift_angle` 180, `0dce38f7` 90, `b5290695` 100 and `35e937b8` 50:
  thresholds pushed out of reach, i.e. drifting off. `Max_yaw_torque` relative to weight varies 3× (911 high,
  F-150 low).
- **Drift model** (`crates/mostrusted/src/drift.rs`, details and per-field status in handling.md): Off →
  Entering (drift scale over `0dce38f7`, or handbrake, while steering) → Drifting (slip over `8f151151`, or
  directly over `Slip_angle_to_enter_drift`) → Off (under `Drift_angle_to_exit_drift`). Yaw torque from the
  Min/Mid/Max curve against steering, faded towards `Maximum_drift_angle`; `Drift_angular_damping` on the yaw
  rate; inward side force; automatic counter-steer; a drift level that decays below `Drift_angle_for_decay`.
  Donuts from `Donut_yaw_torque`, scaled by `Speed_influence` × mph.
- **Dead ends while calibrating** (911 unless noted, steering 0.8 after a brake tap at 110 km/h):
  - Torque and damping only (no fade): the car spins at once (82°, down to 10 km/h). The data's damping read at
    60 Hz (3.1 /s) gives ω ≈ τ / (3.1 I) ≈ 2 rad/s at 0.8 steering; at 120 Hz it holds 20–33° but still drifts
    up slowly, at 240 Hz the car stays under the side-force decay angle and never really drifts.
  - The tyre moment hardly changes with the drift angle once the rear saturates, so something must make the
    torque depend on the angle; a linear fade to `Maximum_drift_angle` worked except on full lock and throttle
    (spin), power 2 is stable everywhere but shallow (12–28°), 1.5 gives 14–30°.
  - Damping the yaw rate relative to the path's turn rate instead of the absolute yaw rate: spins at 0.8.
  - Front wheels following the player's lock in a drift: everything over-rotates and scrubs speed (911 down to
    24 km/h) but the heavy cars drift; blending the lock in below `Low_slip_angle_for_counter_steering` + 10°
    lets the F-150 and Veyron hold 11–14°, but the 911 spins on full lock and throttle and the F-150 crabs
    sideways with no yaw. Kept the full counter-steer.
  - Side force pointing outward instead of inward changes the angles by only 2–4°.
  - Donut torque without a speed factor: the car pirouettes at 200–270°/s on the spot.
- Open: the yaw-curve breakpoints, what `Default_steering` (0.2–0.4) does, the side force's units, the
  per-tick time base, and the heavy cars (they barely drift here; if they drift in the original, its rear
  grip in a drift is lower than this model's). Measuring drift angles in the original game would settle most of
  these (handling.md, "Matching the original game").

### 2026-10-04 (evening): engine sound rewritten from the game's audio data
- User report after the resampling fix: the engine "still plays snippets in full and isn't blending them".
  Cause: grains (whole engine cycles, up to 77 ms) were butt-joined with a 2 ms cross-fade while
  alternating between two grains, and only two of a car's six recordings played.
- **The game's mix is data** (`Genesys.Gen.EngineSound2` in the vehicle bundle, `mw_assets::engine_sound`):
  `Sequences` → `Sequence` objects. Two hold three `GinsuSequenceItem`s each (Ginsu + `MixerChannel` +
  RPM range 1790–7500 on the 911), all playing at once: per microphone set a full-load rev-up, a part-load
  rev-up and an off-load fall. A third sequence (0..1 binding: engine load) has `MixerChannelSequenceItem`s
  whose gain curves (`AnimPoints`: x `12d3a8aa`, y `12d3a8ab`, four tangent fields not decoded) crossfade
  the channels. 911 set A: full 0 → 1 from load 0.53 to 1, part a triangle peaking at 0.47, off 0.86 → 0 by
  0.5; set B similar. Channel `Gain` 0.709 / 0.709 / 1.0; `MasterGain` 1.25. Bus channels carry EQ /
  distortion / sub-harmoniser / filter curves (not used yet). A second `EngineSound2` with one sequence uses
  the short Ginsu copies (probably distant / low quality). Veyron, GT-R, Mustang and Focus also give six
  layers.
- The two microphone sets are one recording run: equal grain counts (506 / 506), boundaries offset by a
  constant trim (~1,200 samples), cross-correlation 0.2. Independent synths put them out of phase (sweep
  pitch-tracking p90 150 cents vs ~65 for one layer); a shared period schedule and shared randomness fixed
  that (p90 63 cents).
- **New synth** (`mw_audio::ginsu`): overlap-add of engine cycles. Each output period blends the current
  cycle out and the next grain's preceding cycle in with cos² / sin² windows, both resampled to the period
  of the current RPM (pitch follows RPM continuously). Consecutive grains share samples, so straight playback
  is seamless; the walk takes the next cycle 7 periods in 8 within ±2 grains of the RPM's grain, else a
  random grain in that window. All layers share one schedule (the first layer sets the period) and one random
  stream. Load is smoothed over 50 ms, RPM over 10 ms; the sum goes through `tanh`.
- Measurements (911, `mwtool engine-sound`): 2,000 → 7,500 rpm sweep p90 pitch error 63 cents (tracker floor
  ~65 for a single layer), worst discontinuity ratio 0.71 (was 0.82); 2-20 Hz envelope modulation on steady
  holds 0.008–0.038 for most rpm / load cells, close to the raw recordings' 0.04–0.05; exceptions full load
  at 2,500 rpm (0.098: long low-rpm cycles make any small window loop at a few Hz) and off load at 4,500 rpm
  (0.082; the off-load recording has only ~41 grains per 1,000 rpm). Strategies compared: strict walk with
  jump-back (loops), random grain every period (more modulation), wider windows (more modulation).
- Assumed / not decoded: the load binding is fed with throttle (reverse: brake); the sequences' binding raw
  values, curve tangents, whether RPM is clamped to the items' 1790–7500 range, the bus effects, engine
  states (Burnout / Drift / In_air / Reversing lists).

### 2026-10-04 (later): the game's deferred pipeline in Bevy, environment timeline
- **Pipeline**: opaque game materials now run in Bevy's deferred prepass and write the game's four G-buffer
  targets (RGBA8 each, packed into Bevy's 128-bit deferred texel) with deferred-lighting ID 2. A full-screen
  system in the `Core3d` schedule (`deferred.rs`, `game_lighting.wgsl`) ports `LightBuffer_KeylightAndAmbient`
  and the `Fog` pass and lights those pixels; it runs before Bevy's own deferred lighting (which only touches
  ID 1) and stores the ID depth, because Bevy's pass discards it. RT3 (indirect) is computed in the G-buffer
  pass as the game does: sky reflection cube at mip = roughness × 5.5 × env-BRDF (Karis' analytic fit stands
  in for the game's 3-D LUT) × AO², plus the vehicle clear coat's sharp reflection. Translucent materials stay
  forward with Bevy lighting. Verified: tinting the pass output shows it draws every opaque pixel.
- **Environment timeline decoded** (a Sonnet agent surveyed and cracked the names; formats.md has the
  layout): 25 timelines × 21 hourly keyframes; default timeline `…69cbb`, noon keyframe `…124619` (key light
  (11, 7.95, 5.21), ambient scale 2.6, fog near 80 / far 1525 / power 1.5, cloud texture `…3f46`). 29 names
  added to `genesys_names.txt`, each checked against its hash. Seven `Alternate` keyframes are bad-weather
  twins (dimmer key light, nearer fog); nothing found yet selects a timeline for free roam. (Superseded on
  2026-10-04, weather entry: 154 keyframes share 7 Alternates; a WorldPaintMap selects the timeline.)
- The renderer now takes key light, ambient rig (→ irradiance cube), sky gradient + sun colour + cloud layer
  (→ skybox and reflection cube) and fog from the keyframe at `--hour` (default 13). Game HDR units × 1200 =
  cd/m² / lux (`GAME_UNIT`) until the game's exposure is ported.
- **Dead end / correction**: the two HDR panoramas used earlier this session are a sunrise capture (glow 3°
  above the horizon) that no keyframe references; no longer used.
- Provisional (marked in code): sun direction (hand-picked; SunMoonPath fields mostly unnamed), sun glow
  exponent 500 and scale 1.067 (unnamed sky fields `2ec8df83` / `2ec8df82`), cloud threshold = 1 − Opacity
  and sharpness = 1 / Feathering, cloud dome mapping, gradient position = sin(elevation), the fog colour's sun
  gradient (pow 4 of the azimuth cosine), no height-layer fog, in-scattering disabled (the raw phase 0.971
  in strength/(1 + phase·cosθ)² floods the view; the CPU-side mapping to ScatteringSettings is unknown).
- Remaining Bevy stand-ins: Bevy's tonemapping and default exposure (the game has auto exposure, a tonemap
  pass and a ColourCube grade), `GeneratedEnvironmentMapLight` for forward materials, no shadows (the game
  uses a cascaded shadow map), the env-BRDF fit.

### 2026-10-04 (later): engine rev-up staircase fixed
- `GinsuSynth` now resamples every grain by `rpm / recorded rpm` (Catmull-Rom read position, clamped to
  0.7–1.4) and fades into the old grain's continuation at the same rate. Each grain's recorded rpm comes from
  inverting the RPM table at its centre (`Ginsu::rpm_at_position`); on the 911 it matches the grain-length
  estimate `120 · sr / len` within 1 % from 3,000 rpm up (1.7 % on the first grain).
- The mixer's RPM smoothing went from 30 ms to 10 ms: with resampled grains the 30 ms filter was the main
  remaining lag (20–40 cents on a 2,200 rpm/s sweep). 10 ms still bridges the 64 Hz updates.
- Measured on a 2,000 → 7,500 rpm sweep in 2.5 s, 30 ms windows, harmonic-sum f0 at rpm/20 (±25 %), scored
  against the rpm delayed by each version's control latency: median −30 → −5 cents, p90 |error| 84 → 59
  cents, p95 window-to-window step 131 → 91 cents. Discontinuities (peak |diff| / local RMS per 2 ms block):
  max 0.88 → 0.76. Steady holds: envelope modulation unchanged (0.020 / 0.035 / 0.003 at 2,500 / 4,500 /
  6,500 rpm off load); on-load holds sit within 3 cents of the expected pitch.
- Metric caveats (dead ends): below ~3,000 rpm an engine cycle (48 ms at 2,500) is longer than the window, so
  per-window errors there are tracker noise; around 4,900–5,300 rpm the 911's recording has partials that
  pull a 5-harmonic comb to −200 cents on steady holds with both synths; the off-load recording has little
  energy at the firing frequency at 4,500 rpm (the tracker hits its bound). A comb on the engine-cycle
  fundamental (rpm/120, 30 harmonics, ±10 %) failed even on a clean synthetic sweep.

### 2026-10-04 (shaders): G-buffer decodes, tangent frames, instance overrides, paint, sky
- **G-buffer semantics** (from the main pass of each family; t-registers mapped through sampler slots of the
  programs that keep RDEF names): RT0 normal + roughness, RT1 sqrt(albedo) + AO, RT2 F0 + Fresnel strength,
  RT3 sqrt(indirect × exposure). Reflection = cube.rgb × SkyReflectionAverageLight + cube.a ×
  SkyReflectionAverageColour, times an env-BRDF LUT sampled at (N·V, roughness, F0) × 0.9375 + 0.03125.
  Exposure = Tonemap(0.5, 0.5).w × HDRConstants.x. A full per-family write-up (13 groups, ~330 lines) was made
  in the session scratchpad; the ported maths is in `layered.wgsl` with comments. Highlights:
  - WorldPBR: F0 = cubic(spec.x) (sRGB→linear approximation), roughness = spec.y, AO = AO map(uv1).y ×
    instance w. ColouredSpecular: F0 = SpecularColour(uv0).rgb × MaterialDiffuse, no cubic. PersistentLightmap
    = Lightmap without the `DynamicLighting_GlobalLightMapIntensities` factor. Lit windows: emissive =
    spec.w² × AOMap.w × LitWindowColour × AdditiveIntensities.y × albedo (night only, not drawn).
  - Terrain: tiling normals hold x, y and roughness (w), z reconstructed; the `Normal` variant's macro normal
    map is world space (texel swizzled x, z, −y); AO = 2·mask.g; F0 / roughness / Fresnel per layer from
    `Layer{1,2,3}_Roughness_Specular_Fresnel`.
  - Roads: normals blended in slope space (road map + world-space detail map + vertex normal, then
    normalize(p.x, 1, p.y)); roughness = Normal.a × Detail.a; F0 = SpecColour.g; wetness from a puddle map,
    RoadDepth and vertex TEXCOORD5.x; reflection only planar × wet mask, so dry roads have no indirect
    specular. Road vertex colour: rgb = baked light, a = AO.
  - Plots: blended tiling normals (rgb), roughness = lerp(decal.z, 1, layer.x × normal.a), F0 = layer.y.
  - Trees: G-buffer albedo = (texel / alpha)² (premultiplied, gamma space), AO = vertex colour.y × instance w,
    wind = sin(time·speed + uv.y) × amplitude × uv.x on x and z, no specular.
  - Water: two wave normal maps scrolling by time / waveAnimationSpeeds; the reflection's Fresnel lookup uses
    dot(−V, N), negative on visible water, so it always takes the grazing value (near-mirror water).
  - Vehicles: textured parts F0 = spec rgb, roughness = normal-map alpha, AO / specular occlusion = vertex
    colour x / y; clear coat F0 = PbrMaterialClearcoatSpecular rising to 1 at grazing angles by
    PbrMaterialClearcoatFresnel; base scaled by (1 − coat). The tyre material binds no diffuse / spec / normal
    textures although its pass reads them (D3D11 unbound = 0): black rubber plus the dust layer.
- **Packed tangent frames decoded** (world RGBA8, vehicle SNORM16): see formats.md. Test
  `mw-assets/tests/tangent_frames.rs`: normals agree with face normals on 98.8 % (TRK_UNIT1) / 97.3 % (911)
  of triangles; tangents follow +U on 82 % / 68 % (vehicle normal maps also use other UV sets).
- **InstanceList per-instance overrides** (+0x10) and the world matrix's w column (tint / AO): formats.md.
  Prop / compound instances have a similar block at +0x50 (`InstanceAdditiveAndAO`), not decoded; their tint is
  treated as 1.
- **VehiclePaint field names** cracked with the paint shader's constant names as vocabulary (14 names, in
  `genesys_names.txt`). `a1d177bd` (0.019–0.05) is read as the coat F0 by range and position; unconfirmed.
- **Sky**: `ENVIRONMENT.BNDL` panoramas and cloud layers (formats.md). One daytime pair (`…3f39` HDR,
  `…3f3a` clouds) is used as skybox and environment-light source until the timeline is decoded.
- **Bevy stand-ins** at the time (superseded by the deferred pipeline entry above): `StandardMaterial`
  lighting fed with the ported G-buffer values; `GeneratedEnvironmentMapLight` from the sky cube instead of the
  game's reflection cubes + LUT + irradiance quadric (needs Bevy features `dfg_lut` and `bluenoise_texture`;
  without `dfg_lut` the environment specular silently does nothing); `DistanceFog`; Bevy's clear coat (F0
  fixed at 0.04, strength set to the coat's grazing reflectance); water near-mirror approximated with F0 0.6;
  directional light at a hand-picked angle (sun azimuth aligned with the panorama's glow); no shadows.
- Open: lightmap texture source (the lightmap shaders read `LightMapTextureSampler`, which neither the
  material nor the instance overrides bind); road vertex AO and baked light; water river floor (third UV set),
  foam, caustics; paint flakes / pearlescent / livery; vehicle glass and lights; TerrainPBR TangentSpaceNormal
  macro frame.

### 2026-10-04 (late night): jumpy engine sound
- **Ginsu RPM tables are signed.** The short recordings start at about -2832 samples, so as u32 their tables
  looked like they were falling. They are in fact *rising*. They are lower-quality copies of the main rev-ups:
  same RPM range, same first and last grain sizes, fewer grains.
- **Recording layout per car**: two sets of three long recordings, in resource order (likely engine and
  exhaust): the main rev-up under load, a second rising sweep from lower RPM, and a falling (off-load) sweep.
  Then come the short copies of the two main rev-ups. Grains are one engine cycle, i.e. two crank turns (911:
  3684 samples at 1575 rpm, 734 at 7814 rpm, at 48 kHz). `pick_engine` used to take the rising recording with
  the most grains and, separately, the "falling" one with the most grains. On the 911 that paired set A's
  rev-up with set B's off-load sweep, so timbre, loudness and RPM range changed at every throttle change. It now
  takes the rev-up that has a short copy, plus the falling sweep of the same set (nearest in resource order).
  92 of 93 cars get such a pair; one car has no Ginsu at all.
- **RPM signal** (`MW_ENGINE_TRACE=<file>` with `--test engine`, rendered by `mwtool engine-sound --trace`):
  1. The launch-clutch slip applied in every gear below about 5600 rpm, so a throttle tap in 3rd at 115 km/h
     moved the revs about 600 rpm in one tick. The slip now applies only in 1st and reverse.
  2. The race gearbox's downshift points (911: 3700–6000 rpm) also applied off the throttle, so coasting and
     braking kicked down with jumps of about 1800 rpm. These are now kick-down points: off throttle the box uses
     the cruising gearbox's points (2000 rpm), blended by pedal position.
- The on/off-load mix is now an equal-power cross-fade (the recordings are uncorrelated, so a linear fade
  dips 3 dB).
- Measured (911, 10 ms loudness steps and 20 ms spectral-centroid steps): the off-load sweep through the
  synth is now smoother than its raw recording (centroid steps over 3 semitones in 6% vs 9% of windows), and
  loudness steps over the human-like pedal trace fell from 4.0 to 3.0 dB (p95).
- Dead end: picking the next grain at random around the target instead of alternating made held-RPM
  modulation worse (0.069 vs 0.018 at 2500 rpm off load).
- **Rev-up staircase**: grains play at their recorded pitch and change only at grain ends, so under
  acceleration the pitch steps (2000→7500 rpm in 2.5 s: up to 118 cents behind, steps up to 370 cents per 5 ms).
  Fix planned: resample grains by current rpm / grain rpm (see next-session.md).
- Still open: a steady 4500 rpm off load warbles at about 18 Hz (two dissimilar neighbouring grains
  alternate), and the second sweep and second set are not used yet (layering, `EngineSound2` data).

### 2026-10-04: world layers
- Prop, Compound, Dynamic and Light instance lists share one record layout (see formats.md): 0x60 bytes, matrix
  at +0x00 (InstanceList has it at +0x20), object import at +0x40. Whole install: 507 lists, 21068 objects,
  3424 object definitions, 3628 lights, all parse and every object resolves to a model
  (`crates/mw-assets/tests/world_layers.rs`). Map placements 35431 -> 56499 (props 12290, compounds 4301,
  dynamics 4477).
- Visual check (unit 1 road, spawn area): tyres, barrels, pallet, road sign and street lamps stand upright on
  the ground, not mirrored. No axis fix needed.
- CompoundObject model slot 0x24 is a lower-poly twin of slot 0x04 (same bbox, ~1/3 to 1/2 the triangles);
  we draw slot 0x04. Guess: 0x24 is a distance LOD. Unverified.
- Translation-row w of the 3x4 matrices is 0.94 for props (240/255 as float?) and 1.0/0 elsewhere. Unknown.
- **Groundcover** (0x20F collection + 0x214 instance blobs), decoded from scratch (layout in the doc comment of
  `crates/mw-formats/src/groundcover.rs`): the game has no mesh for it, the `Groundcover` vertex shader expands
  16-byte instances into camera-facing wind-swayed sprites. Per collection: up to 5 materials (one atlas
  texture each with 16 texture regions), 3 layers, a bounding-volume tree of 0x60-byte nodes whose leaves
  point at 0x40-byte groups (box, material index, quads, blob + start + count). Instance x/y/z are u16
  fractions of the group box. All 400 collections / 1,364,853 instances in the install parse, group counts add
  up to the layer totals, and every blob resolves (`all_groundcover_parses`).
  - Dead ends: first thought `start` was a global index (it is `start << 16 | blob`, blobs hold <= ~8190
    instances), and that material = layer (it is the group's +0x24; a collection with 1 material uses it for
    all layers). The first sub-record fields were mistaken for leaf headers.
  - Verification: neighbour instances within 1.5 m differ in y by 0.0 (median) when the axes are (x, y, z) and by
    0.6 when permuted; against collision triangles ~25% of sampled instances sit within 5 mm of the surface
    (the rest are over bridges/water/other layers). Visually small grass tufts stand on the lawn at
    (-1135, 24.8, 1606).
  - Guesses: size byte / 32 = height in metres (the box margin r equals the largest size byte / 32); byte 6
    is yaw (random in some groups, equal to the size in others); tint = albedo * 2 (the shader outputs
    sqrt(2 * tint)); sprites per instance = `kind / 2` crossed quads. Instance density is low (1.36 M for the
    whole map) so verges are mostly bare; some dark tufts are dark because the tint is.
  - Runtime: merged into one mesh per collection and material on the loader thread (+2.7 s load of ~32 s,
    peak RSS +23 MB), static crossed quads instead of the shader's billboards, `--no-groundcover` to skip.
- The big leaf-card quads sticking out of tree canopies in the road viewpoint predate the new layers (identical
  with only `InstanceList` placements and `--no-groundcover`); not investigated.
- Open: groundcover wind/billboarding and fade distance, `cb1[0].x` size constant, light definitions
  (GeneSys, not decoded); CompoundObject parts (0x14/0x18 arrays of 0x30-byte sub-transforms, probably
  breakable pieces) unexplored; ZoneHeader is just a list of imports to the above (not needed to place things).

### 2026-10-04 (night): collision holes, engine note, chase camera
- **PolygonSoupList quads are in triangle-strip order** (the fourth vertex is opposite the first): split them as
  (a, b, c) + (b, d, c). The fan split made every quad a bow-tie with a triangular hole, which caused the gaps in
  the floor and phantom surfaces (the "drive-911" viewpoint car used to stop on one). Checked by normals: all
  225,442 collision quads in the install give two same-facing triangles with the strip split (1 exception);
  `crates/mw-assets/tests/corpus.rs`.
- **Engine note**: the synth was faithful (rendered pitch follows the recording; the autocorrelation f0 readings
  of rpm/40 above ~4,500 rpm on throttle are octave errors, because that recording has a strong every-other-cycle
  pattern). The fault was the RPM signal: at launch it held the clutch rpm flat for ~1.5 s. Now it rises with the
  wheels, and `Engine_inertia` limits rev changes. `mwtool engine-sound --trace` renders a recorded trace
  through the same `EngineMixer` the game uses.
- **Chase camera**: `VehicleCameraContainer.External` gives the eye and look-at points, the FOV range and
  roll/pitch/shake amounts (table in [handling.md](handling.md)). The FOV is vertical judging by the framing.
  Physics runs at 64 Hz; the car now uses avian's `TransformInterpolation` + Hermite easing, and the camera
  smooths car-relative offsets only (smoothing its world position made it trail by v/6 m).

### 2026-10-04 (later): GeneSys field-name hash cracked, handling from named data
- **The field-name hash is CRC-32** (polynomial 0x04C11DB7, MSB first, not reflected), computed with the
  shift-register ("non-augmented") algorithm: the register starts as the complement of the first four bytes,
  each later byte is shifted in at the bottom while the byte shifted out goes through the table, and the result
  is complemented. Names of four bytes therefore hash to themselves (`Mass` = 0x4d617373), which was the clue.
  `mw_formats::genesys::field_hash`.
- How it was found: nested types are named `Parent.Child` (`Engine.Sound`, `DriftBehaviour.Side_force`), and
  the field holding one is named `Child`, which gave known (name, hash) pairs. Two fields in one type differed
  by XOR 0x0816 = `"in" ^ "ax"` (…min / …max): the hash is XOR-linear with one byte per 8 bits, i.e. polynomial
  division. XORing two same-length names cancels the unknown init, and dividing their difference by small
  quotients gave the polynomial (0x04C11DB7 from every pair); the per-length constant then fixed init/xorout.
  Earlier attempts failed because they hashed with the standard (augmented) CRC.
- Recovering names: `mwtool genesys-crack` builds names from word lists in the game's styles
  (`Capital_snake_case` for designer data, `CamelCase` and `Title_Snake` elsewhere). The exe's strings are
  encrypted, so word lists come from type-name components and English. False positives are real
  (`High_race_angle_efficiency` collides with `Mass`; `Typed_Visual` with `655d3f5f`), so names were
  accepted when they read naturally, fit their sibling fields and fit the values (`Top_speed_MPH` = 189).
- Handling names that settled earlier guesses: `RaceGearbox` vs `CruisingGearbox` (the race box is the
  player's), `Gear_change_time` (0.15 vs 0.9 s), `Differential` = final drive, `Power_To_Front`,
  `Engine_resistance`, `Idle_RPM` / `Max_RPM`.
- **Tyres**: seven curves per axle: `Pure_Lateral_Grip` / `Brake_Slip_Lateral_Grip` /
  `Traction_Slip_Lateral_Grip` (`Gradient`, `Peak_Force_Coefficient`, `Peak_Slip_Angle`) and
  `{Brake,Traction}_{Pure,Slip}_Long_Grip` (`Peak_Force_Coefficient` at `Peak_Slip_Ratio`,
  `Max_Slip_Force_Coefficient` at `Max_Slip_Ratio`). On most cars `Gradient = 3 × peak / angle`: the curve is a
  Hermite from 0 to the peak with a flat end tangent. Details in [handling.md](handling.md).
- **Steering**: per speed band (`Medium_speed` / `High_speed`, mph) a wheel-angle range and separate rates for
  steering in, across and back to centre.
- `RigidBody` vector `62dbd683` is the bounding-box centre (the physical definition's value minus the body
  offset gives the body's), so the body transform's translation is indeed the centre of mass.
- `VehicleCameraContainer`, `VehiclePaint` and `WheelSizes` objects sit in every vehicle bundle: likely sources
  for chase-camera offsets and paint colours.

### 2026-10-04: texture scaling, audio
- `mwtool shader-usage` ranks shaders by world triangles. Every top family was checked in its vertex shader:
  WorldPBR / World_* use UV0 unscaled (correct already). PlotPBR scales UV0 per layer by
  `Tiling_Ratios_X/Y`, its mask by `maskTilingRatios` and its decal by `decalTilingRatios`, tints each layer
  by `tiling{1,2,3}Diffuse`, and blends `decal.x * decal_Diffuse.rgb` by `decal.y * decal_Diffuse.w`. Road
  line variants draw `Line_Diffuse_Sampler` at TEXCOORD2, tinted by `Line_Diffuse`, over the normal road
  albedo by the line alpha. `World_GbufferBlend` meshes are decals (alpha blended). Cables use a gradient
  texture, not a diffuse texture. Vehicle shaders use UV0.
- **Audio codecs**: every `.SPS` and `Wave` resource is EAAudioCore v1 with codec 5 = EALayer3 v1 (mono to
  5.1, 16–48 kHz). 135 `.SPS` files start with a `D` block (headerless; their headers live elsewhere, not
  decoded yet). Engine sounds are Ginsu (`Gnsu30`) with EA-XAS v0.
- **Ginsu layout**: vgmstream's audio offset is 4 bytes short for MW: there is a u32 (0) at 0x20, the RPM
  table starts at 0x24. Evidence: at vgmstream's offset XAS frame starts jump ~3× more than frame interiors;
  4 bytes later the ratio is 0.99. The RPM table holds recording *positions* (samples) at evenly spaced RPMs
  (rising for on-load sweeps, falling for off-load); the grain table holds grain boundaries.
- Each car has eight Ginsu recordings (two sets of three sweeps plus two short ones); which is engine /
  exhaust / interior is not decoded yet. On-load = rising RPM table, off-load = falling.
- Synth check without listening: a rendered rev sweep's spectral centroid rises 715 → 1400 Hz and falls back
  on the coast; grain switches must cross-fade with the old grain's *continuation*, not its already-played
  tail (that clicked every ~40 ms).

### 2026-10-03 (night): GeneSys decoded, handling, LOD stand-ins, controller
- **GeneSys decoder works** (`mw-formats/src/genesys.rs`, `mwtool genesys`). Field flags: low byte = alignment,
  `0x100` pointer (in-chunk sub-object; with an import on the field = external ref), `0x200` fixed array,
  `0x800` dynamic array, `0x1000` array of pointers. Array lengths are trailing small unsigned fields, assigned
  to arrays sorted by (alignment descending, offset): RigidBody's bool array is laid out first but counted last.
  Every struct starts with a type import at +0.
- Field names of four characters or fewer are stored verbatim (`Mass` = `0x4d617373`); longer names are hashed
  with the still-unknown function.
- All vehicle handling data is in GeneSys; see [handling.md](handling.md) for the mappings and evidence.
- Three objects per vehicle bundle fail to decode (pointers read as huge offsets). Their types are not needed
  for handling; still open.
- **Wheel labels can lie**: the Veyron's spec names its rear wheels "Front". All cars face +Z, so wheels are now
  classified by position (front = z above the midpoint; mirror when x < 0).
- **Backdrop instances**: InstanceList entries past the "always loaded" count are world-space geometry at the
  identity transform (terrain, water, roads, horizon). They also include LOD2 stand-in buildings
  (shaders containing `LOD2`) that overlap the detailed buildings; those meshes are now stripped from backdrop
  placements (`--keep-lod-proxies` brings them back). This was the "misplaced assets" report.
  (Superseded on 2026-10-04: the backdrop is a per-unit LOD view; see the backdrop composition entry.)
- **Controller**: gilrs's bundled SDL mapping DB lacked the 8BitDo Ultimate 2 (`03000000c82d00000b31…`), so its
  triggers were unmapped. The current SDL GameControllerDB is bundled and passed via `SDL_GAMECONTROLLERCONFIG`;
  `--gamepad-debug` logs raw input.
- Physics pitfalls: avian `LinearDamping` acts like ~1.7 kN of drag at 240 km/h (now 0); suspension damping
  must use velocity along the ground normal, not the car's up axis (porpoising at speed).

### 2026-10-03 (evening): collision and driving
- **PolygonSoupList** decoded (layout in `mw-formats/src/collision.rs`): world-space, vertices `u16 ×
  500/32768 + i8 multiplier × 500 m`, quads before triangles. One list per track unit (TRK_UNIT1: 158 soups,
  6.9k triangles). It includes invisible walls / barriers.
- GenesysType kinds: 0 int, 1 uint, 2 float, 3 bool, 4 char, 5 enum, 6 compound value (Vector3, StringBase),
  7 class/struct, 8 resource handle. The type name is at `*(*(type+0x20))`, size at 0x18, fields inline from
  0x24 (0x1C each: type import, type-hash low32, 0, name hash, size, offset, u16 count, u16 align). Objects
  start with the base type's fields (0xC bytes for the common base).
- Drivable prototype: avian3d 0.7. Pitfalls hit: (1) wheel rays must exclude the body's child collider entity;
  (2) suspension damping from finite differences launches the car on first contact, so use the body velocity at
  the mount; (3) explicit `Mass` alone leaves collider-derived (density 1) inertia, so set `AngularInertia` and
  `NoAuto*` explicitly.

### 2026-10-03 (later): vehicles, names, shaders
- **Imports**: the high bit of the import offset word is a kind flag. `0` = pointer slot, `0x8xxxxxxx` = handle
  (the ID is stored in place). GenesysObject, ZoneHeader, the VehicleGraphicsSpec locator tables and others use
  handles. `mw_bundle::ImportKind`.
- **VehicleGraphicsSpec** decoded (layout in `mw-formats/src/vehicle.rs`): body part models, skeleton, collision
  PolygonSoupList, and four 0x90-byte wheels (pos, quat, scale, part models + spin mask + name). Wheel models
  are authored for the left (+X) side; right wheels are mirrored (scale.x = -1). Verified visually on the 911 and
  the Veyron.
- Vehicle IDs: `VEH_<id>_HI/LO.BNDL` uses the VehicleList entry ID (field 0x00). The GraphicsSpec resource ID
  is a GameChanger ID that also carries it (e.g. `0x0101000100108e4f` → 1085007).
- **Language**: `{u32 index, u32 count, ptr ids, ptr (offset,len) pairs}`, strings UTF-16LE.
  `UI/LANGUAGE/0001.BNDL` is English.
- **VehicleList**: 110 × 0x70-byte entries; 0x08/0x10 Language IDs, 0x6C year.
- **Shader parameters**: shaders store parameter names, hashes and defaults (0x14 / 0x1C / 0x20 / 0x24 /
  0x28), so material parameter hashes resolve to names (`PbrMaterialDiffuseColour`, `Tiling_Ratios_X`, ...).
- **ShaderProgramBuffer = DXBC** (SM4.0, with RDEF reflection, so names survive). Pixel programs are imported by
  the Shader; vertex programs sit behind VertexProgramState → ShaderProgramBuffer. `mwtool shader-asm <name>`.
  The renderer is deferred (4 G-buffer targets; albedo stored as `sqrt`).
- HLSL `TEXCOORDn` == VertexDescriptor `TexCoord(n)` (raw semantic 5+n).
- **DriveableSurface** albedo = `ColourMap(uv0) × DetailMap_Diffuse(world.xz × DetailMapUvScale) × 2`. Line,
  decal and paint variants are overlays.
- **TerrainPBR** albedo = Σ layer_i(uv1 × (Tiling_Ratios_X[i], Tiling_Ratios_Y[i])) × w_i × 2·mask.g, where
  `w.x = overlay(mask.r, noise)`, `w.z = overlay(mask.b, 1-noise)`, `w.y = 1 - mask.r - mask.b`, normalised;
  noise = `NoiseTexture(uv1 × Tiling_Ratio_Noise).g`.
- Vehicle bodies: `VehicleNFS13_BodyPaint` has no parameters (paint colour comes from elsewhere, likely player /
  GeneSys data); trim uses `VehicleNFS13_Body` with `PbrMaterialDiffuseColour`.
- GeneSys (started): `GenesysType` = reflected type. Header: `u8 2, u8 kind, u16 field count`, base-type import
  at 0x04, size at 0x0C, name hash at 0x10, name string near the end ("Genesys.Gen.NitrousParameters.TrafficNearMiss",
  "CgsCore.UniqueId", "uint16_t"...). Fields are 0x1C bytes: `{type import, type hash, 0, name hash, size,
  offset, u16 count, u16 align}`. Name hashes are not CRC32 of the full type name; the hash is still unknown.
- The `--screenshot` pose was being moved by gamepad stick drift; the camera controller is now disabled in
  screenshot mode.

### 2026-10-03: initial survey
- All bundles are `bnd2` v5, platform 1 (PC). Resource entries are 0x48 bytes (0x44 of data, padded for
  the u64). Every resource in the install decompresses to its declared size.
- `NFS13.exe` `.text` is encrypted (entropy 8.0); the entry point is in an appended DRM stub. See
  [reverse-engineering.md](reverse-engineering.md).
- Vertex attribute and texture formats are raw `DXGI_FORMAT` values.
- Vehicle/character positions: `R16G16B16A16_SNORM` × 10.0 (Porsche 911 = 4.49 × 1.97 × 1.30 m).
- Shaders import their VertexDescriptor at 0x9C (all 373 shaders).
- Material sampler `u16` keys match Shader sampler keys; shader sampler names identify the role.
- InstanceList matrices sometimes have `w = 1.0` in rows 0–2; ignore the w column.
- World: 169 `HAWAII/TRK_UNIT*.BNDL`, 35,424 InstanceList placements, ≈26M LOD0 triangles, 0 decode errors.
- Config bundles (`VEHICLELIST`, `GLOBALCONFIG`, `PHYSICSCONFIG`, `GAMEPLAY`) are almost entirely GeneSys
  types and objects.
- Audio: `.SPS` streams start with an EA SNR/SPS block header (`0x48`). Movies: `.VP6` with EA `MVhd`
  container.
- No signage is mirrored when the game's coordinates are used as-is (Y up, metres).
