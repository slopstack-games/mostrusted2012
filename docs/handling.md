# Handling: game data → physics

Goal: cars should drive like they do in the original game. The handling *code* is in the encrypted executable,
but the handling *data* is in each car's `VEHICLES/VEH_<id>_HI.BNDL` as GeneSys objects
(`HandlingBehaviour`, `RaceCarPhysicalDefinition`, `WheelSuspensionSpringConstraint`, ...). Most Rusted reads
that data (`crates/mw-assets/src/handling.rs`) and feeds it to its own vehicle model
(`crates/mostrusted/src/drive.rs`).

Inspect a car's data with:

```sh
mwtool handling "911"                                      # extracted, typed parameters
mwtool genesys VEHICLES/VEH_1085007_HI.BNDL --type HandlingBehaviour   # everything, by field name
```

## Field names

GeneSys stores field names only as hashes. The hash function is known (`mw_formats::genesys::field_hash`, see
the research log, 2026-10-04), so names can be confirmed by guessing: `mwtool genesys-crack` searches word
lists. Recovered names live in `crates/mw-formats/src/genesys_names.txt`, and `mwtool genesys` prints them.
In code, `VehicleHandling::param("Drift.Yaw_torque.Maximum_yaw_torque")` reads any `HandlingBehaviour` field by
dotted path (unnamed fields as their 8-digit hex hash).
A 32-bit hash also matches wrong guesses now and then (`High_race_angle_efficiency` hashes like `Mass`), so a
name is trusted only when it reads naturally, fits its neighbours (`Low_speed_steer_rate` /
`High_speed_steer_rate` / `Reverse_steer_rate`) and fits the values.

## Data used by the physics model

| Name | Where | Meaning | Evidence |
|------|-------|---------|----------|
| `Mass`, `Inertia_Scale` | RigidBody (first = body, then wheels) | kg; per-axis multiplier on the box inertia | 911 1295 kg, Veyron 1728 kg |
| `0d0c29b4` | RigidBody | body transform; translation = centre of mass | rear-biased on the 911 (z −0.27). The neighbouring vector `62dbd683` is the bounding-box centre |
| `98e902ed` | RigidBody | half extents; for wheels (radius, radius, half width) | wheel radius 0.34 = arch size |
| `TorqueCurve` (`TangentIn`/`TangentOut`) | Engine.Power_curve | torque points (rpm, Nm) with Hermite tangents | 911: 440 Nm at 5600 rpm (real: 440 at 5600) |
| `Idle_RPM`, `Max_RPM` | Power_curve | idle and limiter rpm | 1000 / 7800 (911) |
| `Top_speed_MPH` | Engine | top speed, mph | 911 189 (real 188), Veyron SS 268 (real 268) |
| `Engine_resistance` | Engine | engine braking torque (Nm, negative) | −370 / −319 |
| `Power_To_Front` | Engine.Differentials | front torque share | 0 on the RWD 911, 0.37 on the AWD Veyron |
| `RaceGearbox` / `CruisingGearbox` | Engine | the player's gearbox / an early-shifting one | race shifts near the limiter |
| `Gear_ratio`, `Gear_up_RPM`, `Gear_down_RPM`, `Differential` | Gearbox | ratios, shift points, final drive | 911 3.91, 2.29 … × 3.44 (real PDK) |
| `Gear_change_time` | Gearbox | time without drive per shift | 0.15 s race, 0.9 s cruising |
| `6ade4bb6` / `66251131` | BrakeBehaviour | brake torque per front / rear wheel (Nm) | 100–0 km/h ≈ 35 m |
| `Brake_strength_at_low_speed` / `_at_high_speed` | HandbrakeAbility | handbrake torque per rear wheel (N m) at `Low_speed_mph` / `High_speed_mph`, linear between | 911: 350 at 30 mph, 1200 at 135 mph (see Handbrake below) |
| `0c5f89b3` / `Damping_Ratio` / `41543f85` | WheelSuspensionSpringConstraint | spring frequency (Hz) / damping ratio / travel (m) | 2.0 / 0.35 / 0.08 front on the 911 |
| `In_air_gravity_coefficient` | Aerodynamics | gravity multiplier while airborne | 1.4 |
| `Engine_inertia` | Engine | engine rotating inertia (kg m²): limits how fast the revs rise (torque / inertia) and fall (resistance / inertia) | 0.293 on the 911 |
| tyre curves, steering | see below | | |

### Tyres

`Tyres.Tyres` holds `Front_tyres` and `Rear_tyres`, each with seven grip curves, plus per-axle
`Front/Rear_lateral_scale` and `_longitudinal_scale` (1.0 on the cars checked):

- **Lateral** (`Pure_Lateral_Grip`, `Brake_Slip_Lateral_Grip`, `Traction_Slip_Lateral_Grip`): `Gradient`,
  `Peak_Force_Coefficient`, `Peak_Slip_Angle` (degrees), and a fourth field that is 0.5 on every car. "Pure"
  applies with no longitudinal slip; the other two while the wheel brakes or spins up.
- **Longitudinal** (`Brake_Pure_Long_Grip`, `Traction_Pure_Long_Grip`, `Brake_Slip_Long_Grip`,
  `Traction_Slip_Long_Grip`): `Peak_Force_Coefficient` at `Peak_Slip_Ratio`, then
  `Max_Slip_Force_Coefficient` at `Max_Slip_Ratio`. "Slip" applies while the tyre also slides sideways.

Shape of the lateral curve: across all 197 handling objects, `Gradient × Peak_Slip_Angle / Peak` is 3.0 for
most pure-lateral curves. That is the initial slope of a cubic Hermite from (0, 0) to the peak with a flat
tangent there, so the curve is modelled that way (the ratio varies from 1.5 to 6 elsewhere, and the same
Hermite covers those). Grip stays at the peak beyond the peak angle.

The 911's front pure curve peaks at 25° of slip and its rear at 10° with a stiff gradient. The rear
`Brake_Slip_Lateral_Grip` (gradient 0.069, peak at 50°) gives a locked rear almost no grip at small angles,
which is what makes a handbrake turn rotate the car. Steering ranges are large (35° at low speed) to suit
these peak angles.

Not used yet: `Lateral/Long_Slip_Factors` (probably how slip blends the pure and slip curves), `Grip_factor`
and `Sliding_grip_factor` (3.1 front / 4.5 rear on the 911), `Available_grip` (28000 / 35000).

### Steering

`SteeringBehaviour`: `Medium_speed` 65 and `High_speed` 135 (mph) split two speed bands. Each band has a
`*_steering_range` (wheel angle in degrees: 35 low, 20 high, 35 reverse) and three rates in degrees per
second: `*_steer_rate` (32 / 22) to steer further, `*_counter_steer_rate` (90 / 42) to steer across, and
`*_center_steer_rate` (42 / 10) to return to centre. In a drift the wheels counter-steer at
`Counter_steer_rate_in_drift` (120), up to `Maximum_steering_angle` (35); see Drift below. The
`Steering_angle_curve` sub-object is not used yet.

### Drift (`DriftBehaviour`, `crates/mostrusted/src/drift.rs`)

The game has an explicit drift state (79 `Drift` blocks across the install: every player car plus traffic; the
two Ford Police Interceptors have `Maximum_drift_angle` 180 and unreachable triggers, which reads as drifting
switched off). The code is encrypted, so the model below is built from the field names and values. **Status**:
*name* = field name confirmed by hash and the use follows from it; *hyp.* = the field is unnamed or its use is
a guess that fits the values; *model* = not in the data at all.

| Field (911 value, range over cars) | Use in the model | Status |
|------|------|------|
| `Drift_trigger.Slip_angle_to_enter_drift` (7°, 5.5–12) | body slip at which a slide becomes a drift by itself (power oversteer, handbrake). Easy drifters are lowest (Lancia 5.5, 350Z 6, M3 6.25), AWD supercars highest (8.5–9) | name |
| `Drift_trigger.Minimum_speed_to_enter_drift` (0 mph; 5 on vans) | no drift below it (and never below 5 m/s) | name; floor *model* |
| `Drift_trigger.0dce38f7` (2; 90 on the disabled cops) | the drift scale must exceed it to ask for a drift. Brake + steering exceeds it on every car; steering alone never does (`_from_steering` ≤ 2) | hyp. |
| `Drift_trigger.8f151151` (5°, 4.5–12, always ≤ `Slip_angle_to_enter_drift`) | slip that confirms a drift the driver asked for (state *Entering* → *Drifting*) | hyp. |
| `Drift_trigger.b5290695` (10; 100 when disabled) | unused (a speed or angle threshold) | — |
| `Drift_scale.Drift_scale_from_braking` (1, 0.45–1), `_from_steering` (1.6, 1.2–2), `_from_counter_steering` (1, 0.1–1.65), `Drift_scale_decay` (0.5) | drift scale = max(previous − decay × t, brake × B + \|steer\| × S + counter-steer × C); the handbrake counts as full brake | names; the formula *hyp.* |
| `Drift_scale.c1bc8c79` (6), `ce8870ad` (0.3), `e6abd30f` (0.0001) | unused | — |
| `Yaw_torque.Minimum_yaw_torque` / `Mid_yaw_torque` / `Maximum_yaw_torque` (1875 / 10625 / 18125 N m) with `2ef426dd` (−0.1 everywhere), `70528afa` (0.1, 0–0.5), `a5c9496f` (0.5, 0.35–0.8) | yaw torque against steering towards the drift, piecewise linear through (−0.1, 0), (0.1, Min), (0.5, Mid), (1, Max). Below −0.1 (counter-steer): `Counter_steer_yaw_torque` (0 everywhere) | names; breakpoints *hyp.* |
| `Yaw_torque.Initial_yaw_torque`, `Torque_decay` | 0 on every car, unused | — |
| `Other.Maximum_drift_angle` (90°) | the yaw torque fades as (1 − angle / 90°)^1.5 | name; fade *model* |
| `Other.Drift_angular_damping` (0.05, 0.015–0.075) | fraction of the yaw rate removed per 60 Hz tick (−ln(0.95) × 60 ≈ 3.1 /s × yaw inertia × yaw rate) while entering or drifting | name; time base *model* |
| `Drift_exit.Drift_angle_to_exit_drift` (5°, 2–5) | the drift ends below this angle, or when the car slides the other way | name |
| `Drift_exit.Time_to_blend_damping` (0.25 s) | the drift damping fades out over this time after an exit | name |
| `Other.Side_force_multiplier` (5, 3–6) | side acceleration in m/s² towards the inside of the drift, perpendicular to the velocity ("keeps the car on its line") | name; unit and direction *hyp.* |
| `Side_force.Minimum_drift_angle_for_side_force` (5°) / `Drift_angle_for_side_force` (1.5°) | side force from 0 at 5° to full at 6.5° | names; ramp *hyp.* |
| `Side_force.Speed_for_maximum_side_force` (90 mph), `Drift_scale_for_maximum_side_force` (0.3) | side force × min(speed / 90 mph, 1) × min(scale / 0.3, 1) | names |
| `Side_force.Drift_angle_for_decay` (6.5°), `Decay_rate` (0.018) | below the angle a drift level decays by the rate per tick; it scales both the yaw torque and the side force, so a drift that has gone shallow dies out | names; scaling the torque too *model* |
| `Side_force.ff5dcf85` (0.025, 0.02–0.025) | the level's recovery per tick above the decay angle | hyp. |
| `Other.Low_slip_angle_for_counter_steering` (5°, 3–5), `Max_counter_steering_power` (1.25, 1–2.5) | while drifting the front wheels counter-steer by themselves: wheel angle = −1.25 × (angle − 5°), capped at `Maximum_steering_angle`, at `Counter_steer_rate_in_drift`. The steering input sets the yaw torque instead | names; formula *model* |
| `Other.Default_steering` (0.24), `Steering_angle_multiplier` (1), `fe68bc4b` (40), `35e937b8` (15), `fb0656a3` (18), `ac261613`, `939b8fdc`, `55930ac1`; `Side_force.09832a93` (14); `Drift_exit.ca545e03` (1) | unused | — |

Also part of the model (*model*): pulling the handbrake while steering asks for a drift directly; traction
control lets go in the drift state; and in it the driven wheels use their `Traction_Slip_Lateral_Grip` curve in
proportion to the throttle (the wheels are not simulated spinning up, so this stands in for wheelspin).

States: **Off** → **Entering** (scale over the trigger, or handbrake, with steering beyond `70528afa`; the yaw
torque swings the tail out; abandoned after 1 s or when the steering is released) → **Drifting** (confirmed at
`8f151151`, or directly at `Slip_angle_to_enter_drift`) → **Off** (below `Drift_angle_to_exit_drift`).

Why the extra pieces: with the data's torque alone the car either spins or straightens, because once the rear
tyres saturate their restoring moment hardly changes with the angle. The angle-dependent fade gives each
steering input its own steady angle. The counter-steer keeps the front tyres from adding a nose-in moment that
grows with the angle (without it the 911 spins and the drift scrubs most of its speed; with the player's lock
blended in at small angles the heavy cars hold drifts, but the 911 spins on full lock and throttle). Damping the
yaw rate relative to the path instead of the absolute yaw rate was unstable.

### Handbrake (`HandbrakeAbility`)

`Brake_strength_at_low_speed` (150–750) and `_at_high_speed` (300–1800, the field read before as "handbrake
torque") blend between `Low_speed_mph` (30) and `High_speed_mph` (135). The handbrake has no ABS: a rear wheel
locks once its brake force exceeds the tyre's grip, otherwise it brakes and the lateral curve blends towards
`Brake_Slip_Lateral_Grip`. A pull lasts at least `Minimum_handbrake_time` (0.1 s). Two unnamed fields,
`35e18207` (0.3–0.6) and `6ce0fda7` (0.5–1.25, the larger of the two on every car), are read as the rear lateral
grip factor at low and high speed while the handbrake is held (*hyp.*, by analogy with the strength pair). The
type also declares `Handbrake_grip_values` (`Front_wheel_grip_factor`, `Rear_wheel_grip_factor`), which no car
uses.

### Donuts

`Donut_yaw_torque` (6000–60000 N m) applies on full lock and throttle (≥ 0.8, *model*). A donut starts between
`Donut_speed_limit_low` (5 mph) and `Donut_scale.Max_speed_for_donut` (10 mph) and ends above
`Donut_speed_limit_high` (30 on the 911, 25–50) or when the inputs are released (*hyp.*). The torque scales with
`Gas_influence` × throttle + `Steering_influence` × steering (1 and 0 on every car) and with `Speed_influence`
× mph (0.025, capped at 1; *hyp.*: without it the car pirouettes on the spot at 200°/s). The drift damping
applies. `Donut_grip_values` (eight unnamed fields) is unused.

## Chase camera

`VehicleCameraContainer` holds `Bonnet`, `Bumper` and `External` cameras. The runtime uses `External`:

| Field | 911 | Meaning |
|-------|-----|---------|
| `6e08f788` | (0, 1.6, −4.6) | eye in car space; the bonnet (0, 1.05, 0.8) and bumper (0, 0.425, 1.5) cameras use the same field for their in-car positions |
| `006e1686` | (0, 1.2, 0.75) | look-at point |
| `MinFieldOfView` / `MaxFieldOfView` | 70 / 95 | FOV at rest and at top speed. Read as vertical: as horizontal, 70° crops the car much tighter than the original's chase view |
| `Camera_Roll.Maximum_Roll_Angle` | 4.7 | roll into corners (degrees, per g of lateral acceleration) |
| `Deceleration_pitch_change.Maximum_Pitch_Change` | 4.5 | nose-down tilt under braking (degrees per g) |
| `Speed_based_movement.High_Speed_Shake` | `Amplitude` 1.2, `Frequency` 1, `Power` 1.5 | shake grows with (speed / top speed)^Power |

The `Acceleration_movement`, `Yaw_spring_modification` and `Speed_based_height_change` blocks are not named
yet. The pull-back under acceleration and the yaw lag are model choices until they are.

## Model decisions (not from data)

| Constant | Value | Why |
|----------|-------|-----|
| `LONG_SCALE` | 0.6 | longitudinal force coefficient → μ; calibrated on 0–100 km/h and 100–0 km/h |
| `LAT_SCALE` | 0.44 | lateral force coefficient → μ; the 911 holds 1.0 g on the skidpad (the data's lateral coefficients are ~1.6× its longitudinal ones on the same tyre, so the two need separate scales) |
| curve blend | — | lateral curve: pure → brake/traction-slip by (longitudinal force / limit)²; longitudinal: pure → slip by slip angle / pure peak angle |
| driver aids | — | ABS on the service brakes (not the handbrake), traction control unless the handbrake is pulled, the car already slides or is in the drift state (the real cars have both; the game's launch times need them) |
| drift time base | 60 Hz | per-tick drift rates (`Drift_angular_damping`, `Decay_rate`, `ff5dcf85`) are fractions per 60 Hz tick; at 120 Hz the 911's drifts are 6–10° shallower |
| `TORQUE_FADE_POWER` | 1.5 | drift yaw torque × (1 − angle / `Maximum_drift_angle`)^1.5; with 1.0 the 911 spins on full lock and throttle, with 2.0 its drifts are 3–5° shallower |
| drift counter-steer | −`Max_counter_steering_power` × (angle − `Low_slip_angle_for_counter_steering`) | front wheel angle while drifting, so the front tyres stay near zero slip |
| drift wheelspin | throttle | in the drift state the driven wheels' lateral curve blends to `Traction_Slip_Lateral_Grip` by the pedal |
| `MIN_DRIFT_SPEED`, `ENTRY_TIME`, `DONUT_INPUT` | 5 m/s, 1 s, 0.8 | no drift below 5 m/s; an asked-for drift is dropped after 1 s; donuts need 80% lock and throttle |
| steering bands | — | low-speed values below `Medium_speed`, high-speed above `High_speed`, linear between |
| clutch | — | at launch the revs start at 55% of the limiter and rise linearly with wheel speed until the clutch locks at 1.3× that (`CLUTCH_LOCK`); holding them flat made the engine note stall for 1.5 s; only in 1st and reverse (in other gears every throttle tap jumped the revs) |
| downshifts | — | the race gearbox's `Gear_down_RPM` are kick-down points; off throttle the cruising gearbox's are used, blended by pedal (otherwise coasting jumps the revs ~1800 rpm per downshift) |
| `DRIVETRAIN_EFFICIENCY` | 0.85 | typical; not found in the data |
| drag | fitted | quadratic drag chosen so the car tops out at `Top_speed_MPH` (the aero block has `*_drag_coefficient` fields whose units are unclear) |
| wheel axle/side | by position | some specs label wheels the other way round (the Veyron's "Front" wheels are at the back) |

## Measurements

Run on the flat test track with `mostrusted --drive <car> --test accel|brake|corner|handbrake|drift|donut`
(4× real time, logs `TEST RESULT`). `corner` holds 80 km/h and winds the steering in over 15 s; `handbrake`
taps the handbrake for 0.4 s with half steering at 80 km/h, then holds the steering on 0.3 throttle; `drift`
does the game's brake-tap drift at 110 km/h (steer 0.8, 0.3 s brake tap, then 0.6 throttle for 6 s) and
reports peak slip, how long body slip stays above 10° (`held`) and its mean, and the time in the drift state;
`donut` holds full throttle on full lock from rest; `engine` runs a pedal sequence for engine-sound checks.
For sweeps the drift script takes `MW_TEST_STEER`, `MW_TEST_THROTTLE`, `MW_TEST_KMH`, `MW_TEST_HOLD` and
`MW_TEST_RELEASE` (let go of the steering after that many seconds).

The test script runs in `FixedUpdate` and counts physics ticks, so results are identical between runs (since
2026-10-04; before, 0–100 km/h varied from 3.93 to 4.91 s on the 911 with frame timing).

| Car | 0–100 km/h | ¼ mile | top speed | 100–0 km/h | skidpad (80 km/h) |
|-----|-----------|--------|-----------|------------|-------------------|
| Porsche 911 Carrera S | 3.92 s | 11.78 s | 296.6 km/h (data 304) | 37.1 m | 1.01 g at 60% steering, 2.5° body slip |
| Bugatti Veyron SS | 2.58 s | 9.61 s | 423.2 km/h (data 431) | 34.2 m | 0.99 g |
| McLaren MP4-12C | 3.56 s | 10.94 s | 316.5 km/h | 37.6 m | 0.93 g |

Real-world references: 911 Carrera S PDK ≈ 3.9–4.3 s / 304 km/h; Veyron SS ≈ 2.5 s / 431 km/h; MP4-12C ≈ 3.3 s.
(The Veyron and McLaren rows predate tick-exact tests.)

Drifts (2026-10-04, drift model). `--test drift` unless noted; angles are body slip:

| Car | Result |
|-----|--------|
| 911 Carrera S | peak 24.4°, steady 20° for the whole 6 s, 110 → 96 km/h. Before the drift model: peak 9.9°, never held |
| 911, steering 0.6 / 1.0 with full throttle | steady 14° / 30° (89 km/h) |
| 911, steering released after 4 s | straightens and leaves the drift within 1 s |
| 911 `--test handbrake` | peak 16.7° (was 4.8°), the drift dies out after ~1.5 s at half steering and 0.3 throttle |
| 911 `--test donut` | 87°/s at 17 km/h, 3.2 m radius, body slip swinging 0–23° |
| Mustang Boss 302 | steady ~14°, held 5.6 s |
| McLaren MP4-12C | peak 13.6°, in the drift state 4.7 s at 10–12° |
| Caterham R500 (446 kg, max torque 3700 N m) | peak 22.5°, then a shallow 8–12° drift |
| Veyron SS (AWD) | peak 12°, drops back to grip after 1.6 s |
| F-150 Raptor (2552 kg) | peak 9°, drops back to grip; full lock and throttle: 15°, 3.8 s in the drift state. Donut 70°/s |
| Aston Martin V12 Vantage, `--test corner` | 1.74 g at 90% steering: it slides past `Slip_angle_to_enter_drift` and the side force adds lateral g |

The heavy and AWD cars hold shallow drifts or none: their data torques are small next to the restoring moment
of their saturated rear tyres (Raptor: 17000 N m against ~13000 N m from the rear tyres plus damping of 22600
N m per rad/s of yaw rate). If they drift properly in the original, the game's rear grip in a drift must be
lower than this model's.

## Matching the original game

Real-world specs are only a proxy. To tune towards the actual game, measure the same tests in NFS Most Wanted
(2012) and add them to the table above:

1. **0–100 km/h and top speed:** on a long straight (e.g. the airfield), full throttle from standstill; time it
   from a recording (the in-game speedometer is km/h or mph depending on settings).
2. **100–0 km/h braking distance:** brake hard at 100 km/h; measure the distance from markers or a recording.
3. **Steady-state cornering:** the speed at which a car holds a known-radius roundabout or curve.
4. **Drift:** the drift angle at half and full steering after a brake tap at a known speed, the speed lost over
   a few seconds, and whether a truck (F-150 Raptor) or the Veyron holds a drift at all. These are what the drift
   model's constants (`TORQUE_FADE_POWER`, the 60 Hz time base, the counter-steer gain) should be fitted to.

Then adjust the model-decision constants (or use more fields) until `--test` matches, and record the change
with its evidence in this file.

### Measuring the original with `mwtool memscan` / `telemetry`

Stopwatch measurements only give end results. To see *how* the original's handling works (when a drift
triggers, how the drift scale builds and decays, how grip falls off with slip), read the car's state from the
running game's memory and log it every frame. `mwtool memscan` and `mwtool telemetry`
(`crates/mwtool/src/memscan.rs`) are **read-only**: they read data values from `/proc/<pid>/mem` of the
game process the user is running. Nothing is written or injected, and the protected code is never read,
traced or disassembled (docs/reverse-engineering.md). Linux / Proton only for now.

**Setup (once per boot):** `sudo sysctl kernel.yama.ptrace_scope=0` (or `sudo setcap cap_sys_ptrace+ep
target/release/mwtool` after each build). Start the game, enter free roam, then in a terminal:
`./target/release/mwtool memscan --watch handling.watch`.

**Finding a value.** The game may pause when it loses focus. That helps: do the manoeuvre, pause (or alt-tab)
so the values freeze, type the step, go back. The pattern, for the car's speed:

```text
(car standing still)        scan f32 unknown      # snapshot everything (takes a few seconds)
(drive off, pause)          inc
(drive faster, pause)       inc
(brake a little, pause)     dec
(stand still, pause)        unchanged
(repeat until a few dozen remain)
                            list 20               # compare with the speedometer: m/s × 3.6 = km/h
                            watch #3 10           # live values for 10 s while you drive
                            save speed f32 #3
```

When the shown value is known, `scan f32 range LO HI` starts much narrower than `unknown`. Once one value of
the car is found, `dump #3 64` shows its neighbours: velocity vectors, positions, RPM and the like usually
sit in the same object, and their offsets from the speed are stable.

**Surviving a restart.** Heap addresses change between runs. `ptr <addr>` lists 32-bit values pointing just
below an address (object base + offset). One inside `NFS13.exe+…` is a static pointer: save the chain as
`save speed f32 NFS13.exe+0x123456 0x48`. Otherwise run `ptr` on the pointer's own location for a two-level
chain.

**Values to find, in order:** speed, velocity vector and position (fixes units and axes), throttle / brake /
steering inputs as the game sees them (after its own filtering), RPM and gear, yaw rate, then the drift
state: the drift scale (rises on a brake tap while turning, decays afterwards: `inc` then `dec` passes),
the drifting flag (`scan u8` or `i32`, 0/1 toggling with drifts), and per-wheel slip.

**Recording:** `./target/release/mwtool telemetry handling.watch -o drift_911.csv --hz 120` while doing the
manoeuvres from the list above (one CSV per car and manoeuvre). The CSVs contain measurements of game state,
not game code or assets, but keep them out of the repo with other local captures. They drive the fit: run
the same inputs through `--test`, compare the traces, and record each fitted rule here with its evidence.

### Suspension rest position (2026-10-04)

The graphics spec's wheel position is the wheel **at rest** (in its arch under the car's weight). The suspension
mount therefore sits `travel` (data, `WheelSuspensionSpringConstraint` `41543f85`: 0.07–0.08 m on the 911, 0.15
on the Raptor) above it; the wheel droops below the rest position by up to the static sag (`corner mass × g /
k`, ~0.06–0.10 m) and compresses at most `travel` above it. Before this, the spec position was treated as full
droop, so the body rode one sag too low and the wheels rose `travel + sag` into the fenders (user report).
911 tests after the change: 0–100 3.80 s (was 3.92), corner 0.98 g (1.01), drift mean 19.9° (20.2°).

