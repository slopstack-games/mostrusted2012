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
| `4af55f8a` | HandbrakeAbility | handbrake torque | 1200 |
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
`*_center_steer_rate` (42 / 10) to return to centre. `Counter_steer_rate_in_drift` (120),
`Maximum_steering_angle` (35) and a `Steering_angle_curve` sub-object are not used yet.

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
| driver aids | — | ABS on the service brakes, traction control unless the handbrake is pulled or the car already slides (the real cars have both; the game's launch times need them) |
| steering bands | — | low-speed values below `Medium_speed`, high-speed above `High_speed`, linear between |
| clutch | — | at launch the revs start at 55% of the limiter and rise linearly with wheel speed until the clutch locks at 1.3× that (`CLUTCH_LOCK`); holding them flat made the engine note stall for 1.5 s; only in 1st and reverse (in other gears every throttle tap jumped the revs) |
| downshifts | — | the race gearbox's `Gear_down_RPM` are kick-down points; off throttle the cruising gearbox's are used, blended by pedal (otherwise coasting jumps the revs ~1800 rpm per downshift) |
| `DRIVETRAIN_EFFICIENCY` | 0.85 | typical; not found in the data |
| drag | fitted | quadratic drag chosen so the car tops out at `Top_speed_MPH` (the aero block has `*_drag_coefficient` fields whose units are unclear) |
| wheel axle/side | by position | some specs label wheels the other way round (the Veyron's "Front" wheels are at the back) |

## Measurements

Run on the flat test track with `mostrusted --drive <car> --test accel|brake|corner|handbrake` (4× real time,
logs `TEST RESULT`). `corner` holds 80 km/h and winds the steering in over 15 s; `handbrake` taps the
handbrake for 0.4 s with half steering at 80 km/h; `drift` does the game's brake-tap drift at 110 km/h (steer 0.8,
0.3 s brake tap, then 0.6 throttle) and reports how long body slip stays above 10°; `engine` runs a pedal
sequence for engine-sound checks.

Results vary a little between runs (0–100 km/h 3.93 vs 4.33 s on the 911) because the scripted input runs
in `Update` at 4× real time, not per physics tick. Move it to `FixedUpdate` before fine calibration.
Brake-tap drift on the 911 (no drift model yet): peak slip 10–14°, above 10° for at most 1.3 s; the original
holds the drift for as long as you steer.

| Car | 0–100 km/h | ¼ mile | top speed | 100–0 km/h | skidpad (80 km/h) |
|-----|-----------|--------|-----------|------------|-------------------|
| Porsche 911 Carrera S | 4.00 s | 11.79 s | 296.6 km/h (data 304) | 38.3 m | 1.01 g at 60% steering, 2.5° body slip |
| Bugatti Veyron SS | 2.58 s | 9.61 s | 423.2 km/h (data 431) | 34.2 m | 0.99 g |
| McLaren MP4-12C | 3.56 s | 10.94 s | 316.5 km/h | 37.6 m | 0.93 g |

Real-world references: 911 Carrera S PDK ≈ 3.9–4.3 s / 304 km/h; Veyron SS ≈ 2.5 s / 431 km/h; MP4-12C ≈ 3.3 s.

Handbrake tap (911): body slip peaks at about 5° and the car grips again; a full one-second pull spins it
(84°). The game holds drifts with `DriftBehaviour` (yaw torque, side force, drift scale, entry and exit
rules), which is not implemented yet. That is the next step for drifting.

## Matching the original game

Real-world specs are only a proxy. To tune towards the actual game, measure the same tests in NFS Most Wanted
(2012) and add them to the table above:

1. **0–100 km/h and top speed:** on a long straight (e.g. the airfield), full throttle from standstill; time it
   from a recording (the in-game speedometer is km/h or mph depending on settings).
2. **100–0 km/h braking distance:** brake hard at 100 km/h; measure the distance from markers or a recording.
3. **Steady-state cornering:** the speed at which a car holds a known-radius roundabout or curve.
4. **Drift:** the handbrake entry speed and how long a drift holds (the game has explicit drift tuning).

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
