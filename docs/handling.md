# Handling: game data → physics

Goal: cars should drive like they do in the original game. The handling *code* is in the encrypted executable,
but the handling *data* is in each car's `VEHICLES/VEH_<id>_HI.BNDL` as GeneSys objects
(`HandlingBehaviour`, `RaceCarPhysicalDefinition`, `WheelSuspensionSpringConstraint`, ...). Most Rusted reads
that data (`crates/mw-assets/src/handling.rs`) and feeds it to its own vehicle model
(`crates/mostrusted/src/drive.rs`).

Inspect a car's data with:

```sh
mwtool handling "911"                                      # extracted, typed parameters
mwtool genesys VEHICLES/VEH_1085007_HI.BNDL --type HandlingBehaviour   # everything, by field hash
```

## Field mappings

GeneSys stores field names as 32-bit hashes (names of four characters or fewer appear verbatim, e.g. `Mass` =
`0x4d617373`). The meanings below were inferred by comparing cars with known real-world specs.

| Hash | Where | Meaning | Evidence | Confidence |
|------|-------|---------|----------|------------|
| `4d617373` | RigidBody | mass (kg) | ASCII "Mass"; 911 1295, Veyron 1728 | high |
| `0d0c29b4` | RigidBody | body transform (translation used as centre of mass) | rear-biased on the 911 (z −0.27) | medium |
| `98e902ed` | RigidBody | half extents; for wheels (radius, radius, half width) | wheel radius 0.34 = arch size | high |
| `fd4709e4` | Engine.Power_curve | torque curve points (rpm, Nm) + Hermite tangents | 911: 440 Nm at 5600 rpm (real: 440 at 5600); Veyron 1500 Nm | high |
| `cc887670` / `1cdde238` | Power_curve | idle / limiter rpm | 1000 / 7800 (911), 6750 (Veyron) | high |
| `f2368dfe` | Engine | top speed in mph | 911 189 (real 188), Veyron SS 268 (real 268) | high |
| `4623dd88` | Engine | engine braking torque (Nm, negative) | −370 / −319 | medium |
| `bef03791` | Engine.Differentials | front torque share | 0 on RWD 911, 0.37 on AWD Veyron | medium |
| `e8ce7d74` | Gearbox.Gear | gear ratio | 911 3.91, 2.29 … (real PDK 3.91, 2.29) | high |
| `c80f787a` / `8f085ebd` | Gearbox.Gear | upshift / downshift rpm | 7700 vs 7800 limiter | medium |
| `ad14f0ff` | Gearbox | final drive | 911 3.44 (real PDK 3.44) | high |
| `2b64bdcf` vs `fd49acb2` | Engine | two gearboxes; the second shifts near the limiter and is used | — | low |
| `6ade4bb6` / `66251131` | BrakeBehaviour | brake torque per front / rear wheel (Nm) | 100–0 km/h ≈ 35 m when per wheel | medium |
| `4af55f8a` | HandbrakeAbility | handbrake torque | 1200 | low |
| `0c5f89b3` / `87e422a9` / `41543f85` | WheelSuspensionSpringConstraint | spring natural frequency (Hz) / damping ratio / travel (m) | 2.0 / 0.35 / 0.08 front on the 911 | medium |
| `66676f77` | Tyre.Long_Grip_Curve | longitudinal grip "peak" | 1.3–1.6; not a plain friction coefficient | low |

Not yet used: drift, donut, steering tables (`SteeringBehaviour`: lock angles in degrees by speed?), body
movement, aerodynamics (192 / 229 look like speeds in mph), tyre lateral curves and slip factors,
`SteeringWheelPhysics`, nitrous.

## Model decisions (not from data)

| Constant | Value | Why |
|----------|-------|-----|
| `DRIVETRAIN_EFFICIENCY` | 0.85 | typical; not found in the data |
| `GRIP_SCALE` | 0.72 | maps the grip-curve peak to μ; calibrated so 0–100 km/h matches real cars |
| `SHIFT_TIME` | 0.15 s | the second gearbox has a 0.15 field |
| `PEAK_SLIP_ANGLE` | 0.12 rad | generic tyre |
| drag | fitted | quadratic drag chosen so the car tops out at the game's top-speed field |
| wheel axle/side | by position | some specs label wheels the other way round (the Veyron's "Front" wheels are at the back) |

## Measurements

Run on the flat test track with `mostrusted --drive <car> --test accel|brake` (4× real time, logs `TEST RESULT`).

| Car | 0–100 km/h | ¼ mile | top speed | 100–0 km/h |
|-----|-----------|--------|-----------|------------|
| Porsche 911 Carrera S | 4.04 s | 11.84 s | 296.6 km/h (data 304) | 37.0 m |
| Bugatti Veyron SS | 2.53 s | 9.41 s | 423.1 km/h (data 431) | 34.8 m |
| McLaren MP4-12C | 4.36 s | 11.69 s | 316.4 km/h | 36.0 m |

Real-world references: 911 Carrera S PDK ≈ 4.3 s / 304 km/h; Veyron SS ≈ 2.5 s / 431 km/h; MP4-12C ≈ 3.3 s.

## Matching the original game

Real-world specs are only a proxy. To tune towards the actual game, measure the same tests in NFS Most Wanted
(2012) and add them to the table above:

1. **0–100 km/h and top speed:** on a long straight (e.g. the airfield), full throttle from standstill; time it
   from a recording (the in-game speedometer is km/h or mph depending on settings).
2. **100–0 km/h braking distance:** brake hard at 100 km/h; measure the distance from markers or a recording.
3. **Steady-state cornering:** the speed at which a car holds a known-radius roundabout or curve.
4. **Drift:** the handbrake entry speed and how long a drift holds (the game has explicit drift tuning).

Then adjust the model-decision constants (or decode more fields) until `--test` matches, and record the change
with its evidence in this file.
