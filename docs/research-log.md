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
- Drift data still guessed (2026-10-04): the yaw-curve breakpoints (`2ef426dd`, `70528afa`, `a5c9496f`),
  triggers `8f151151` / `0dce38f7` / `b5290695`, `Default_steering`, the side force's unit, the per-tick time
  base, and the handbrake's `35e18207` / `6ce0fda7`. See handling.md, Drift.
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
