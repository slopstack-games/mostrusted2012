# Handling: game data → physics

Goal: cars should drive like they do in the original game. The handling *data* is in each car's
`VEHICLES/VEH_<id>_HI.BNDL` as GeneSys objects
(`HandlingBehaviour`, `RaceCarPhysicalDefinition`, `WheelSuspensionSpringConstraint`, ...). Most Rusted reads
that data (`crates/mw-assets/src/handling.rs`) and feeds it to its own vehicle model
(`crates/mostrusted/src/drive.rs` with `drivetrain.rs`, `tyre.rs`, `drift.rs`, `chassis.rs`). Since
2026-10-04 the model follows the game's own code, read from the user's decompiled dump: [decomp.md](decomp.md)
has the findings with function addresses; this file says how the runtime uses them and what is still a model
value.

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

## The model (2026-10-04, from the game's code)

One physics tick is 1/60 s (`Time::<Fixed>` at 60 Hz, as the game). Per tick, in `car_physics`:

1. **Pedals and gear**: in reverse gear the pedals swap; brake at a standstill engages reverse. Brake pressure
   ramps up at 10 × pedal per second; both pedals at low speed count as a burnout (releases the driven brakes).
2. **Engine, clutch, gearbox** (`mw-handling/src/drivetrain.rs`; decomp.md "Drivetrain and brakes" has the
   functions and constants): the engine is its own inertia (`Engine_inertia`). An idle governor opens the
   throttle up to 0.5 within 20 rpm of `Idle_RPM` and fades the closed-throttle drag out; the throttle is
   peak-held (decaying at `c6deb7a6` per tick) and lagged (`36676b9c` per tick). The torque blends by that
   throttle from the closed-throttle torque (the `Power_curve` friction curve, or `Engine_resistance` × inertia
   with the clutch open or during an upshift) to the full-load `TorqueCurve`; downshifts drive the engine at
   −2.5 × `Engine_resistance` × inertia for the whole shift; a small torque noise at the firing rate follows a
   second three-point curve. The revs are clamped at `Max_RPM` after the torque is known. The clutch is one
   speed-matching impulse per tick before the tyres, capped at 5000 N m; the torque goes through gear × final
   drive unchanged (no efficiency loss), split by `Power_To_Front`, half per wheel. As the tyres hold the wheels
   at road speed, it slips under load (the original's engine runs ×1.9 / ×1.3 / ×1.14 above the wheels in
   1st / 2nd / 3rd at full throttle). Pulling away the clutch engages with max(throttle, rpm / the gear's
   `Gear_up_RPM` ± 50) and closes at 0.95; it opens with no throttle while pulling away, below idle, or in gear
   below 20 mph with neither pedal. Upshift when rpm > `Gear_up_RPM` **and** mph > `Gear_up_speed`, skipping
   gears whose entry speed is already passed; downshift below the lower gear's `Gear_up_speed` − 2 mph or below
   `Gear_down_RPM`, to the highest lower gear still above its own down rpm. Shifts take `Gear_change_time` × (1 +
   0.15 × random) (downshifts half that): declutch 10 %, neutral 50 % (the gear changes at its end), clutch-in
   40 %; upshifts cut the throttle until the clutch-in, which brings it back with the clutch. Only the race
   gearbox is used; `13bfea5a` / `7b20506a` lock its top gear (the 911 has 6, not 7).
3. **Steering** (game rules, below).
4. **Wheels**: each wheel has a spin ω (inertia ½ m r² of its rigid body, plus `TransmissionInertia` on driven
   wheels). Drive torque spins it; the axle limited-slip differentials (`Front_` / `Rear_Differential` = c) keep
   the wheel speeds within a ratio of 1 / c.
5. **Suspension**: a ray per wheel. Spring k = W / sag (W = static corner load, sag = `41543f85`); damper
   c = 2 `Damping_Ratio` √(k W / g) in bump and `0c5f89b3` × c in rebound; bump travel 0.06 m above rest (model,
   from the 911's strut); anti-roll bars per axle (`Body_movement` `782f05c1` / `2bb0f6a0` × 60, read as N m per
   degree; damping `Front_` / `Rear_roll_damping` × critical against the roll inertia). The spring force holds
   the body up; it does not set the tyre grip.
6. **Tyres** (`tyre.rs`): a velocity constraint, capped, solved once per tick as the game does. Grip load N =
   the static corner load + ½ × the axle's downforce (not the spring force). Longitudinal: the brake (an angular
   impulse budget that can stop the wheel but never reverse it) acts on the wheel's spin first, then one pass
   pulls the tread to the road speed (wheel inertia against the corner mass) within ±μx × N × factors. Lateral:
   the force that stops the sideways slide, within ±μy × N × factors. Factors: the axle's data scale, the
   surface grip under the wheel (`Surface_Grip` blended by the car's surface link) and camber. Each wheel also
   gets the surface's rolling resistance and slide drag. The tyre forces do not roll the body (the torque about
   the car's long axis is dropped, as in the game).
7. **Drift** (`drift.rs`, below), then aero, lift, in-air gravity and take-off damping (`chassis.rs`).

### Tyres

- Slip angle α = atan(|v_lat| / |v_long|) in degrees; slip ratio κ = (ωr − v) / v braking, (ωr − v) / ωr driving,
  clamped to ±10.
- **Lateral curve** (not a Hermite): n = `Gradient` × `Peak_Slip_Angle` / `Peak_Force_Coefficient`, x = α / peak
  angle ≤ 1, μ = peak × (1 − (1−x)^n cos²(πx/2) − (1−x)^max(2,n) sin²(πx/2)), flat beyond the peak. The fourth
  field (0.5) is unused.
- **Longitudinal curve**: piecewise linear (0, 0) → (`Peak_Slip_Ratio`, `Peak_Force_Coefficient`) →
  (`Max_Slip_Ratio`, `Max_Slip_Force_Coefficient`), flat beyond.
- **Pure → slip blending** (no friction circle): lateral by |κ| through `Brake_` / `Traction_Lateral_Slip_Factors`
  (`89f75d1a` start, `c35e15f1` full, `23a3702c` exponent, `24269da0` max); longitudinal by α in degrees through
  `Brake_` / `Traction_Long_Slip_Factors` (`e0e7e515` start, `bf1bc91d` full, `1d0b87f6` exponent, `d9d33254`
  max). Brake curves when κ < 0.
- Caps = μ × N × `*_lateral_scale` / `*_longitudinal_scale` × surface grip × camber factor (grip falls from 1
  at `e84632d1`° of camber to `8dd08eeb` at `d699cd4c`°); no hidden scale (docs/decomp.md "Tyres", 2026-10-06).
  N = static corner load + ½ × the axle's share of −`Lift_coefficient` × |v_forward| |v|; the lateral curve's
  exponent is multiplied by static load / N. The driven wheels' longitudinal cap is reduced by the burnout
  factor. No caps where the ground normal's up component is under 0.5.
- Surface (per wheel, from the collision tag under it; the flat test track counts as the game's
  `Default_surface`): grip = `Surface_Grip` blended towards the car's `VehicleSurfaceProfile` link grip by its
  per-axle lateral / longitudinal factors (1 on asphalt for every car); resistance = `5c2a9f18` × N (0.01 on
  asphalt) + |mph| × `7585f9cb` × the link's drag ramp over the slip angle (asphalt: up to 4 N per mph at 15°).
- Not read by the game: `Available_grip`, `Grip_factor`, `Sliding_grip_factor`, `f5c703c9`; `AligningTorque` is
  null on every car; the slip-angle relaxation (`7ab2a90a`, 1.8 m) is disabled by `c5cf33ea` = 0.

### Brakes and handbrake

Brake torque per wheel = `6ade4bb6` (front) / `66251131` (rear) × pedal² × ramp. **No ABS and no traction
control** (the game has neither); wheels lock when the brake beats the tyre. The handbrake's strength blends from
`Brake_strength_at_low_speed` to `_at_high_speed` between `Low_speed_mph` and `High_speed_mph` and combines with
the service brake by max; a pull lasts at least `Minimum_handbrake_time`. *Inferred*: the handbrake locks the rear
wheels outright (the game's brake code has a lock mask; the rear tyres' nearly flat `Brake_Slip_Lateral_Grip`
curves, fully blended at κ = −1, are made for it). `35e18207` / `6ce0fda7` belong to the drift (initial drift
scale with the handbrake), not to grip.

### Steering

`steer_range` / `steer_towards` in drive.rs, after the game's steering update:
- Range off the throttle: `Low_speed_steering_range` at 0 mph → `High_speed_steering_range` at `High_speed`. On the
  throttle: low-speed range → `41d22d15` at `Medium_speed` → `98820dfe` at `High_speed` (911: 35° → 24° → 19°),
  blended by clamp(throttle − brake). Reverse gear: → `Reverse_steering_range` by `Reverse_speed` (negative mph).
  Capped at `Maximum_steering_angle`.
- Rates blend from 0 mph to `High_speed` (reverse gear: the reverse rates). Back towards centre: counter-steer rate
  when the input points across the current angle, else the centring rate; outwards: the steer rate.
- In a drift the input is remapped (`DriftState::steering`): no input holds `Default_steering` (0.24) of the
  range into the drift, full input full lock; counter-steer is limited to `ac261613` (−0.15) early in a drift
  (until `fb0656a3` = 18° of peak rear slip) and opens to `939b8fdc` (−1) as the rear slip grows from
  `Low_slip_angle_for_counter_steering` (5°) to `fe68bc4b` (40°). The rate blends towards
  `Counter_steer_rate_in_drift` with the rear slip up to `5277ec9e` (40°).
- Not modelled yet: the neutral offset (`553bc073` × an unknown vehicle value × `Maximum_steering_angle`) with its
  side-limit extension `a40a6b17` / `16fd6022`.

### Drift (`DriftBehaviour`, `drift.rs`)

See [decomp.md](decomp.md) "Drift" for every field. In short: a drift starts when the **mean rear-tyre slip**
exceeds `Slip_angle_to_enter_drift` (a brake tap drops the threshold to `0dce38f7`, recovering at `b5290695` °/s;
the brake pedal blends it towards `8f151151`), with every wheel down and above `Minimum_speed_to_enter_drift`.
It ends after 0.5 s once the rear slip is below `Drift_angle_to_exit_drift` (or airborne > 1 s, reverse gear,
< 0.5 m/s). A signed **drift scale** integrates throttle × body slip (`ce8870ad`, full at `c1bc8c79`°),
steering into the drift × throttle (`_from_steering`; `_from_counter_steering` and negative when
counter-steering), brake, the handbrake (`HandbrakeAbility` lerp) and a constant decay. It sets a **yaw torque**
(quadratic Bézier in the scale through `Minimum_` / `Mid_yaw_torque` and a top that is `Donut_yaw_torque` below
`Donut_speed_limit_low` and `Maximum_yaw_torque` above `_high`) × a steering factor (`a5c9496f` no steering, 1
full lock into the drift, `70528afa` full counter-lock, + `2ef426dd` × throttle); a **side force** of mass ×
`09832a93` along the body's side axis (ramped by body slip from `Drift_angle_for_side_force` to
`Minimum_drift_angle_for_side_force`, reduced by `Decay_rate` per degree above `Drift_angle_for_decay` while steering
into the drift, × scale and speed ramps, × (1 + a yaw-rate boost of gain `Side_force_multiplier`)); **damping** of
the yaw rate by (1 − peak scale × `Drift_angular_damping`)^(ticks) and of the sideways velocity by `ff5dcf85` per
tick, fading over `Time_to_blend_damping` after the exit; and a **speed hold** that makes up `55930ac1` of the
shortfall per tick towards the entry speed (at most `35e937b8` mph ahead). Donuts are this torque at low speed;
`DonutAbility` is not read by the game. `Maximum_drift_angle`, `Max_counter_steering_power`,
`Steering_angle_multiplier`, `Torque_decay` are not read either.

### Aerodynamics and airborne (`chassis.rs`)

Drag per body axis F = −½ K C |v| v with C = (`Side_drag_coefficient`, `Top_drag_coefficient`,
`Front_drag_coefficient`), K = 1.2 (read from the running game, `.bss` 0x011872a0); above
`f1538c51` mph the front coefficient ramps to `b8786ac2` at `4aadb133` mph. Lift = `Lift_coefficient` ×
`ca386558` × |v_forward| |v| along the body's up axis at z = `b821de16` (−0.5 v² N on the 911: downforce).
`Down_force` / `Down_force_lift_coefficient` are not read by the game. Once a wheel leaves the ground gravity
becomes `In_air_gravity_coefficient` × g, and for `Take_off_behaviour.123d2559` s the body's pitch, yaw and roll
rates are damped per tick (`8b456140`, `312bd67d`, roll by roll rate). `Top_speed_MPH` is not read by the
game's physics (the camera's FOV still uses it).

## Cameras (`VehicleCameraContainer`, `crates/mostrusted/src/camera.rs`)

Every vehicle bundle has one `VehicleCameraContainer` (79 in the install) with four cameras: `External`
(`CameraGameplayExternal`, the chase camera), `Bonnet` and `Bumper` (`CameraGameplayBonnet` / `Bumper`, the
same layout) and `Rear_view` (`CameraRearView`, the mirror: `Horizontal_FOV` 60, `Aspect_ratio` 3.33,
`Near_clip_plane` 0.2, `Far_clip_plane` 400). Reader: `mw_assets::camera::VehicleCameras`. The runtime offers
the modes the data defines: chase, bonnet, bumper (C or gamepad Y cycles them; `--camera-mode`
picks one at startup). Units are mph and mph per second, like the handling data. Struct fields are stored
sorted by size and then by name, which places each unnamed field alphabetically between its named
neighbours (formats.md, "GeneSys").

`External` fields and what the runtime does with them (**data** = name and use both settled; *hyp.* = the
meaning is read from values or position; values are the 911's, ranges are across all cars):

| Field | 911 (range) | Use |
|-------|-------------|-----|
| `6e08f788` | (0, 1.6, −4.6) | **data** eye in car space (also the bonnet / bumper eye) |
| `006e1686` | (0, 1.2, 0.75) | **data** look-at point |
| `d68bbc59` | (0, 0, 0.75) | *hyp.* pivot: always (0, 0, look-at z); the yaw and look-around orbit turn around it |
| `MinFieldOfView` / `MaxFieldOfView` | 70 / 95 | **data** FOV, blended by (speed / top speed)²; read as vertical (see below) |
| `Speed_based_movement.0a78c15f` | −1 (−1.2…0) | *hyp.* eye z change at full speed (pulls the eye back 1 m) |
| `….7d4ec535` | 1.7 (1.45…4.4) | *hyp.* eye height at full speed (tracks the eye height on every car) |
| `….b028673b` | 1.5 (1.35…3.1) | *hyp.* look-at height at full speed (≈ look-at height + 0.25) |
| `….53e4b4f9` | 50 (50/75/100) | *hyp.* speed (mph) where the three changes above are complete |
| `High_Speed_Shake.Amplitude`, `Frequency`, `Power` | 1.2, 1, 1.5 | **data** shake = Amplitude × f^Power |
| `….Minimum_Speed_for_Shake` / `Maximum_Speed_for_Shake` | 0 / 180 (125…208) | **data** f = speed fraction between them (mph) |
| `Acceleration_movement.bd3a7cfc` | −2 (−2.5…−1.9) | *hyp.* furthest the eye falls back under acceleration (m) |
| `….8e1310f6` | 1.5 | *hyp.* furthest the eye closes in under braking (m) |
| `….285f2260` | 2.5 | *hyp.* angular frequency (rad/s) of the critically damped spring the eye hangs on: it trails by a / ω², ≈ 1 m at a 911 launch |
| `Camera_Roll.Minimum_Lateral_Acceleration_for_Roll`, `3f8ced67` | 10, 30 (30…40) | **data** / *hyp.* (the maximum, by its place and values) lateral range, mph/s |
| `….Minimum_Angular_Velocity_for_Roll`, `02dd11bc` | 0.2, 2.5 (2.5…3) | **data** / *hyp.* yaw-rate range, rad/s |
| `….Maximum_Roll_Angle` | 4.7 (3…6) | **data** roll = sign × max × max(f_lat, f_yaw)^`Camera_Roll_Factor_Power` (2.5) |
| `….Roll_Angle_Change_Intensity` | 2 (2…2.5) | *hyp.* rate (1/s) the roll follows its target |
| `Deceleration_pitch_change.Minimum_` / `Maximum_deceleration_for_pitch_change` | 2 / 12 (11…12) | **data** deceleration range, mph/s |
| `….Maximum_Pitch_Change`, `Pitch_change_curve_power` | 4.5 (2.5…6), 2.25 | **data** pitch = max × f^power (nose down) |
| `….Pitch_change_intensity` | 2 (1.75…2) | *hyp.* rate (1/s) the pitch follows its target |
| `Yaw_spring_modification.Low_speed` / `High_speed` | 0 / 100 | **data** speed band (mph) |
| `….5a74d44d` / `63fb2207` | 0.075 / 0.05 | *hyp.* yaw spring at low / high speed, a fraction per 60 Hz tick (sorted after `Low_speed` / `High_speed`): 4.7/s at rest, 3.1/s at 100 mph |
| `YawDriftSpring` | 0.055 (0.045…0.07) | **data** name; *hyp.* the yaw spring while drifting (3.4/s) |
| `Global_params.f50dccb9` (`CameraGameplayShakeEffect`) | see below | **data** impact shake springs |
| `Global_params.Impact_shake.Maximum_amplitude` | 2 | **data** cap on an impact's strength |

Impact shake (`CameraGameplayShakeEffect`, the same on every car): per axis a damped spring with
`Amplitude`, `Damping` (translation: `00fd0a79`, *hyp.*), `Spring_Coefficient` and limits (`Minimum_` /
`Maximum_Angle` in degrees, `Lower_` / `Upper_Translation_Limit` in metres). Only two axes are active: rotation
`Pitch` (250, 0.4, 450, ±20°) and the third translation axis (100, 0.5, 100, −0.2…0.2 m; taken as fore-aft).
An impact kicks each spring's velocity by Amplitude × strength; with k = 450 and ζ = 0.4 it rings at 3.4 Hz and
dies in ~0.4 s. A 115 km/h wall hit (`--test crash`, strength capped at 2) peaks at 13° of pitch.

Bonnet / bumper (`CameraGameplayBonnet` / `Bumper`): eye `6e08f788` ((0, 1.05, 0.8) / (0, 0.425, 1.5)), FOV
95→105 / 95→120 (by speed², like the chase camera), and `3f017c6b` ((0, 1.32, 0.21) on the roof / (0, 0.6, −1.5)
at the tail), *hyp.* the eye of the look-back view. 19 other floats are unnamed (one candidate name,
`AccelerationResponse` = 0.05, fits but has no sibling to confirm it); they are not used yet, so the in-car
views are rigid. The bumper eye sits inside the body's nose, so the car is hidden in that view.

Unused and unnamed: the other 9 `Acceleration_movement` floats (0.5, 0, 50, −1.2, 2, 0/−0.9, 1.75, 2.5),
`Yaw_spring_modification.78527964` / `397fce82` (10 / 10) and `Decay_rate` (0.01),
`Speed_based_movement.b6010ee1` (0), `Speed_based_height_change` (1.59 / 1.973; heights, but they do not
track the eye as closely as `7d4ec535`), `Gradient_Adjustments` (`Downwards` 0.5 / 1.5, `Upwards` 0.5 / 4,
`Maximum_Gradient` 10°), the top-level `c6ca2dc7` (5 / 10), `6d906d74` (4) and `67660e20` ((0.55, 0.09, 1)),
`Lens_Properties` (`FocalDistance` 90, 1.4, false: depth of field, off), and the rest of `Impact_shake`
(`ShakeMagnitude` / `WobbleMagnitude` vectors, frequencies).

Model decisions (not data): the FOV is vertical (as horizontal the chase view is far tighter than the
original's; the bonnet and bumper views look very wide this way); acceleration is measured from the car's
speed between physics ticks; the drift lean (`DRIFT_LEAN` 0.5 of the slip angle) and the yaw-lag cap (0.6 rad);
the high-speed shake's scale (0.012 m per unit of Amplitude, 1.4 Hz at Frequency 1); impact strength =
approach speed / 10 m/s; camera collision (a ray from the look-at point to the eye, 0.25 m margin, at least
0.8 m from the look-at point, eases back out at 3/s); look-around (right stick: ±110° yaw, −10…30° pitch, 20%
dead zone, follows at 10/s, springs back at 4/s; right mouse drag; stick fully back, the stick's button or B looks back, a cut to
the view turned 180° round the pivot). No look-around fields were found in the camera types (searched Look,
Free, Orbit, Return, Recentre, Yaw, Pitch, Limit, Speed templates).

Trace: `MW_CAMERA_TRACE=<file>` writes one line per frame (speed, acceleration, yaw rate, lateral
acceleration, slip, the eye in car space, yaw lag, FOV, roll, pitch, acceleration offset, impact pitch).
911 results (2026-10-04): `--test accel` pulls the eye back 1.2 m at launch (eye z −4.6 → −6.7 by 75 km/h with
the speed term), easing to −0.35 m at 200 km/h; FOV 70 → 88.6° at 265 km/h. `--test brake` from 100 km/h: the
eye closes 1.5 m in and pitches 4.5° nose down at 1.1 g. `--test drift`: the camera's yaw sits 15.5° off the
car's in a 19.7° drift (half the slip angle plus 6° of spring lag), the eye 1.5 m to the side; roll peaks at
3.4° at the 1 g entry.

## Model decisions (not from data)

| Constant | Value | Why |
|----------|-------|-----|
| ~~`LAT_GRIP` / `LONG_GRIP`~~ | removed 2026-10-06 | the code's caps are μ × N × data axle scale × surface grip × camber × gameplay grip multipliers (1 outside timed effects): `FUN_009c5da0`, `FUN_00905910`, `FUN_009d8bd0` (decomp.md "Tyres"); N and the downforce share confirmed on `ref/xkr_launch` |
| tyre effective mass | the corner's sprung mass, all four wheels from the same velocity | the game uses the chassis's effective mass at the contact (`FUN_009ca610`) and solves the wheels one after the other, each seeing the velocity the previous left (`FUN_009289f0`); same steady state, different transients |
| blend-input scale | 1 | the pure → slip blends take κ and α × the tyre constraint's +0x210, whose writer is not in the dump; measure with the probe |
| surface-change fade | none (instant) | the game fades the grip of the surface just left over a time and uses the lower of old and new (`FUN_009ceb00`, `FUN_009d4460`); not ported |
| resistance at rest | clamped to what stops the contact | ours: the game applies rolling resistance in full against the sign of the motion (`FUN_009d4a10`), which flips every tick at a standstill |
| test-track surface | `Surfaces.Default_surface` | ours: the flat track has no collision tags; the game's default surface (grip 1, rolling 0.01, no slide drag) |
| `CLUTCH_CAP` | 5000 N m | **from the game**: `.bss` 0x01186f70 (read live), the cap in `FUN_009d7ac0` |
| shift phases | 10 / 50 / 40 % | **from the game**: declutch 0x01186ee0 = 0.1, clutch-in 0x01186fd0 = 0.4, neutral the rest (`FUN_009d1560`, `FUN_009d6880`, `FUN_009ce490`); confirmed tick by tick in `ref/xkr_launch` |
| downshift hysteresis | 2 mph | **from the game**: 0x011872e0 in `FUN_009d93b0` |
| idle governor | idle + 20 rpm, 0.05 / rpm, up to 0.5 throttle | **from the game**: 0x01186f30 / 0x011871c0 / 0x011873f0 in `FUN_009d9960` |
| throttle time base | 60 | `c6deb7a6` / `36676b9c` × the `.bss` vector 0x01187250 (not read; the dump has no initialiser). 60 as the drift's time bases; it reproduces the original's idle (1003–1031 rpm against 1002–1033) where 1 or an instant lag do not. Measure: `memscan` `dump NFS13.exe+0xd87250` |
| clutch-in throttle | pedal × progress¹ | `FUN_009d6af0` / `FUN_009d6ca0` raise the progress to `.bss` 0x011874d0 (not read). Measure: `dump NFS13.exe+0xd874d0` |
| coast declutch | 20 mph | `.bss` 0x011872d0 in `FUN_009d9ee0`: lanes 2–3 read 20 (`consts.txt`), lanes 0–1 assumed the same |
| downshift gate | always open | `FUN_009d93b0` also needs vehicle +0x1ed0 > `.bss` 0x01187580 (neither identified; the original downshifts while braking with no throttle). Not modelled |
| stall check in shifts | none | `Idle_RPM` × `.bss` 0x011874a0 (not read) drops the gearbox to standstill during a shift or launch |
| launch torque | lagged | the original reaches full torque in the first tick of its launch (`ref/xkr_launch`), which the lag does not; source not found (`FUN_009262a0`'s override list?) |
| `BURNOUT_MPH` | 20 | burnout speed scale (missing constant) |
| `DRAG_K` | 1.2 | read from the running game (`.bss` 0x011872a0); was 1 |
| `BUMP_TRAVEL` | 0.06 m | from the 911's strut geometry; not read per car. Searched for the strut joint's builder (`WheelSuspensionConstraint` readers in the vehicle build, `009A`–`009D`): not found; `7fc0dc41` (0.09 / 0.08 m on the 911) is the likeliest field |
| damper slots | `0c5f89b3` × c in rebound | the setup stores c and `0c5f89b3` × c (spring +0x2c / +0x18, `FUN_009b29b0`); the function applying them was not found |
| anti-roll unit | N m per degree, scale 60 | the scale is read live (`.bss` 0x01185d70); the unit is inferred |
| drift time base | 60 Hz | damping fractions are per tick; the game ticks at 60 Hz |
| handbrake | locks the rear wheels | inferred (above) |
| ~~rolling resistance Crr 0.015~~ | removed 2026-10-06 | the game's comes from the surface: `5c2a9f18` × N per wheel plus the slide drag (`FUN_009d4a10`) |
| wheel contact | one ray per wheel | the game ties each wheel record to the chassis with a constraint holding the wheel's ground contact points, iterated in sub-steps (`FUN_0093f200`); the tyre friction is one pass after them |
| wheel axle/side | by position | some specs label wheels the other way round (the Veyron's "Front" wheels are at the back) |

## Measurements

Run on the flat test track with `mostrusted --drive <car> --test accel|brake|corner|handbrake|drift|donut` (logs
`TEST RESULT`; tick-exact, so results repeat). `corner` holds 80 km/h and winds the steering in over 15 s;
`handbrake` taps the handbrake for 0.4 s with half steering at 80 km/h, then holds the steering on 0.3 throttle;
`drift` does a brake-tap drift at 110 km/h (steer 0.8, 0.3 s brake tap, then 0.6 throttle for 6 s; overrides
`MW_TEST_STEER`, `MW_TEST_THROTTLE`, `MW_TEST_KMH`, `MW_TEST_HOLD`, `MW_TEST_RELEASE`); `donut` holds full
throttle on full lock from rest (`MW_TEST_HANDBRAKE=<s>` also pulls the handbrake at the start).

With the model from the game's code (2026-10-04):

| Car | 0–100 km/h | top speed | 100–0 km/h | skidpad | drift test | handbrake |
|-----|-----------|-----------|------------|---------|------------|-----------|
| Porsche 911 Carrera S | 3.88 s | 311 km/h (data 304) | 40.2 m | 1.01 g | 22° held 4.5 s, settles at 68 km/h | 17.9° |
| Bugatti Veyron SS | 3.78 s | 420 km/h (data 431) | 42.2 m | 1.00 g | 11° held 3.5 s | 9.5° |
| McLaren MP4-12C | 3.18 s | 346 km/h | 35.4 m | 0.91 g | 12° held 3.9 s | 14.3° |
| Ford F-150 Raptor | 7.75 s | 273 km/h | 50.8 m | (drifts on the skidpad: 2 g with the side force) | no drift (peak 8°) | 17.8° |

With the code's grip caps (2026-10-06: μ × static load + downforce share, surfaces, brake-then-one-pass, wheel by
wheel on the chassis's effective mass; no calibrated scale):

| Car | 0–100 km/h | top speed | 100–0 km/h | skidpad | drift test | handbrake |
|-----|-----------|-----------|------------|---------|------------|-----------|
| Porsche 911 Carrera S | 3.60 s | 308 km/h | 37.1 m, 3.07 s | 2.57 g | no drift (peak rear slip 4.4°) | 3.7° |
| Bugatti Veyron SS | 2.37 s | 421 km/h | 30.4 m | 2.58 g | no drift (6.2°) | 4.4° |
| Ford F-150 Raptor | 6.70 s | 269 km/h | 37.6 m | 2.04 g | no drift (2.6°) | 3.7° |

Before (same day, calibrated 0.4 / 0.7, Crr 0.015): 911 3.87 s / 40.1 m / 1.02 g / 24.6° drift; Veyron 3.75 s /
42.4 m / 1.00 g; F-150 7.37 s / 51.7 m / 2.01 g (drifting on the skidpad). The scripted brake-tap drift no longer
reaches the entry angle with full grip; whether its inputs drift the original needs a recording (and the +0x210
blend scale, decomp.md "Tyres"). XKR reference replay (`ref/xkr_launch`): launch within 1–3 mph of the original
(was 1–3 mph behind; with the drivetrain branch 04533ea the 2nd-gear revs match too); the braking half of that run
hit something in the original, so its RMS (speed 10.1 mph) says little.

Real-world references: 911 Carrera S PDK ≈ 3.9–4.3 s / 304 km/h; Veyron SS ≈ 2.5 s / 431 km/h; MP4-12C ≈ 3.3 s.
The Veyron's launch is traction-limited at the calibrated grip (its AWD split is 0.37 front).
`--test donut` no longer spins the 911 (full throttle on full lock grips and drives a widening circle: the rear
slip peaks at 5°, under the 7° drift entry); whether the original donuts from that input needs checking in the
game.

Before the switch (the data-only model): 911 0–100 3.80 s, 100–0 37 m, skidpad 0.98 g, drift 20° held 6 s at
96 km/h.

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

### Reference runs: capture, replay, compare (2026-10-04)

The handling is judged against the original, run for run:

1. **Capture** the original while you drive: `./target/release/mwtool capture <name>` (Enter stops). It writes
   `ref/<name>/telemetry.csv` (the car's state from `handling.watch`: speed, inputs, rpm, gear, drift state,
   per-wheel load / slip / grip, body velocity, angular velocity, axes, position; 60 Hz) and `audio.wav` (the
   game's own audio stream through `parecord`, nothing else from the desktop).
2. **Replay** the same inputs in Most Rusted: `./target/release/mostrusted --drive <car> --replay
   ref/<name>/telemetry.csv`. The car gets up to the recording's starting speed on the flat test track, then the
   recorded throttle, brake, steering and handbrake play back tick by tick (the steering's sign is found from the
   recording's yaw rate). Our state goes to `telemetry.ours.csv`.
3. **Compare**: `./target/release/mwtool compare ref/<name>/telemetry.csv ref/<name>/telemetry.ours.csv` prints
   speed, yaw rate, lateral g, body slip, rpm, gear, drift state and the path (in each run's start frame) side by
   side, and RMS differences.

Captures can also be driven without a person: `tools/orig/orig` runs the original headless in gamescope and
scripts its inputs (keys or a virtual pad with analog steering and pedals), waits on values from the watch file
(`waitfor speed_mph > 60`) and starts `capture` in the background, so a manoeuvre is repeatable to the tick of
the script. Start each capture driving straight on a flat road (the replay starts straight), same car as the replay.
Useful runs: a launch, full braking from speed, a steady fast corner, a handbrake turn, a brake-tap drift, a
power-on drift, a full-lock donut from rest, slaloms. The world differs (our test track is flat), so compare the
car's response, not where it ends up on the map.

### Measuring the original with `mwtool memscan` / `telemetry`

Stopwatch measurements only give end results. To see *how* the original's handling works (when a drift
triggers, how the drift scale builds and decays, how grip falls off with slip), read the car's state from the
running game's memory and log it every frame. `mwtool memscan` and `mwtool telemetry`
(`crates/mwtool/src/memscan.rs`) read data values from `/proc/<pid>/mem` of the game process the user is
running; the protected code is never read, traced or disassembled (docs/reverse-engineering.md). Since
2026-10-05 `poke` (and `--poke name=value` on `telemetry` / `capture`) writes a *data* value for an experiment
and restores it afterwards; code mappings are refused (docs/probe.md "Tier 0"). Linux / Proton only for now.

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

The graphics spec's wheel position is the wheel **at rest**. The game's spring is set so it holds the static
corner load exactly at the data's sag (`41543f85`), so the wheel sits in its arch under the car's weight, droops
by up to the sag and compresses at most `BUMP_TRAVEL` above rest.
