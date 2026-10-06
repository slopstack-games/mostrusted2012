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
   ramps up at 10 × pedal per second; both pedals at low speed count as a burnout (releases one axle's brakes).
   The handbrake state (countdown of `Minimum_handbrake_time`) updates here.
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
   40 %; upshifts cut the throttle until the clutch-in, which brings it back with the clutch. Two gearboxes
   (`FUN_009ce630`, `FUN_00900660`): the race one (`RaceGearbox`; `13bfea5a` / `7b20506a` lock its top gear, so
   the 911 has 6, not 7) at full throttle, and the `CruisingGearbox` (own ratios, `Gear_up_RPM` /
   `Gear_up_speed` / `Gear_down_RPM`, slower `Gear_change_time`; the 911's: 5500 rpm, 15 / 45 / 70 / 85 / 105 /
   135 mph, 2000 rpm, 0.9 s, 7 gears) after 3 s of part throttle (0 < pedal < 0.95, no brake). The pedal
   above 0.95 for 0.05 s switches back (a kick-down: the gear is clamped to the race box's gears and the race
   box's `Gear_down_RPM` then downshifts, with the cruising change time for a short window).
3. **Steering** (game rules, below). The angle computed here is used by the next tick's tyres, as in the game.
4. **Wheels**: each wheel has a spin ω (inertia ½ m r² of its rigid body, plus `TransmissionInertia` on driven
   wheels). Drive torque spins it; the axle limited-slip differentials (`Front_` / `Rear_Differential` = c) keep
   the wheel speeds within a ratio of 1 / c.
5. **Suspension**: a ray per wheel. Spring k = W / sag (W = static corner load, sag = `41543f85`); damper
   c = 2 `Damping_Ratio` √(k W / g) in bump and `0c5f89b3` × c in rebound; bump travel 0.06 m above rest (model,
   from the 911's strut); anti-roll bars per axle (`FUN_00933a60`, built by `FUN_008fda70`): stiffness
   `Body_movement` `782f05c1` / `2bb0f6a0` × 60 (applied as N m per radian of roll), damping
   c = 2 × `Front_` / `Rear_roll_damping` × √(data stiffness × roll inertia), the × 60 **outside** the root, so the
   bar runs at 1.2 / √60 ≈ 0.15 of critical (it was 1.2 × critical against the scaled stiffness, 7.7× too much,
   and rang at 11 Hz). Its input (left-right compression difference over the track) and explicit force form are
   ours: the solve that applies the bar was not found (decomp.md). The spring force holds
   the body up; it does not set the tyre grip. **Strut end stop** (`FUN_009c5a90` → `FUN_009c2670`): where the
   ground would come past the end of the bump travel by the end of the tick, the wheel contact acts on the chassis
   as a hard, non-negative normal impulse at the contact (target normal velocity ≥ overtravel / dt, so the
   predicted penetration is 0), solved as sequential impulses over the four wheels (`strut_stops` in drive.rs).
   The wheel ray starts one radius above the mount so ground that has come past the strut still registers.
6. **Tyres** (`tyre.rs`): a velocity constraint, capped, solved once per tick as the game does. Grip load N =
   the static corner load + ½ × the axle's downforce (not the spring force). Longitudinal: the brake (an angular
   impulse budget that can stop the wheel but never reverse it) acts on the wheel's spin first; what is left of
   it then holds the tread like a locked wheel (a force on the chassis alone, at most that torque / r), and one
   coupled pass takes out the remaining slip (wheel spin inertia against the chassis's effective mass), all
   within ±μx × N × factors (`FUN_009bc0c0`). Lateral:
   the force that stops the sideways slide, within ±μy × N × factors. Factors: the axle's data scale, the
   surface grip under the wheel (`Surface_Grip` blended by the car's surface link) and camber. Each wheel also
   gets the surface's rolling resistance and slide drag. The tyre forces do not roll the body (the torque about
   the car's long axis is dropped, as in the game).
7. **Turn-around assist** and **drift** (`drift.rs`, below), then aero, lift, in-air gravity and take-off damping
   (`chassis.rs`). The game runs aero / airborne before the handbrake, assist, steering and drift
   (decomp.md "Update loop").

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
control** (the game has neither); wheels lock when the brake beats the tyre. A burnout (both pedals below 20 mph)
releases the front brakes on a front-driven car and the rear brakes otherwise. The handbrake is a state with a
countdown (`FUN_009d27b0`): reset to `Minimum_handbrake_time` on every tick it is pulled, so it stays on that
long after it is let go. While it is on, the rear brake torque is max(handbrake strength, service torque), the
strength blending from `Brake_strength_at_low_speed` to `_at_high_speed` between `Low_speed_mph` and
`High_speed_mph` (signed speed); the front axle takes all of the drive (rear share 0, also on rear-driven cars);
no drift can start and a running drift ends.
It does **not** lock the rear wheels outright (the game's lock mask comes from other states). The brakes and the
driveline see the previous tick's state, the drift this tick's. `35e18207` / `6ce0fda7` are the drift scale a
drift starts with on the tick after the handbrake is let go, not grip.

### Steering

`steer_range` / `steer_towards` in drive.rs, after the game's steering update (`FUN_009d31f0`):
- Range off the throttle: `Low_speed_steering_range` at 0 mph → `High_speed_steering_range` at `High_speed`. On the
  throttle: low-speed range → `41d22d15` at `Medium_speed` → `98820dfe` at `High_speed` (911: 35° → 24° → 19°),
  blended by clamp(throttle − brake). Reverse gear: → `Reverse_steering_range` by `Reverse_speed` (negative mph).
  Capped at `Maximum_steering_angle`.
- Rates blend from 0 mph to `High_speed` (reverse gear: the reverse rates). Back towards centre: counter-steer rate
  when the input points across the current angle, else the centring rate; outwards: the steer rate if the raw
  stick is at full travel (≥ 0.99), else the raw `High_speed_steer_rate`.
- The game steers after the tyres: each tick's tyres use the angle set on the tick before.
- In a drift the input is remapped (`DriftState::steering`): no input holds `Default_steering` (0.24) of the
  range into the drift, full input full lock; counter-steer is limited to `ac261613` (−0.15) early in a drift
  (until the peak rear slip has gone from `Slip_angle_to_enter_drift` to `fb0656a3` = 18°) and opens to
  `939b8fdc` (−1) as the rear slip grows from `Low_slip_angle_for_counter_steering` (5°) to `fe68bc4b` (40°),
  that opening raised to `Max_counter_steering_power` (inferred, see decomp.md); the wheels themselves are kept
  within that limit. While counter-steering against the rotation the rate blends towards
  `Counter_steer_rate_in_drift` with the body slip from `Slip_angle_to_enter_drift` to `5277ec9e` (40°). A drift
  that ends while counter-steering multiplies the wheel angle by `Drift_exit.ca545e03`.
- Neutral offset (2026-10-06): with no input the wheels stand at q × `Maximum_steering_angle` × `553bc073`
  (0.3 on the 911 and XKR; 1 in reverse, 0 in a drift), turned towards the direction of travel, and each side's
  limit grows by |offset| × `a40a6b17` (1) on the offset's side and × `16fd6022` (0) on the other. q (vehicle
  +0x2c70) is the body slip / 35°, faded by speed / 10 mph below 10 mph: a telemetry reading, its writer is not
  found (decomp.md "Steering").
- Not modelled: the zeroing of the wheel
  angle on a hard wall hit (`FUN_008f7ab0`, contact unit unknown).

### Low-speed turn-around assist

After the body touches a wall facing it, full throttle (> 0.5) and steering (> 0.5) away from the wall, or any
steering below 5 mph, slide the car sideways (towards 0.5 m/s) and yaw it (towards 1 rad/s, less the more
head-on the hit) in the steering direction; the effect ramps in at 1/s, holds 0.5 s and fades at 2/s
(`TurnAssist` in drive.rs, from `FUN_008f8360`). Wall contacts come from avian's contacts of the body collider
with the world layer.

### Drift (`DriftBehaviour`, `drift.rs`)

See [decomp.md](decomp.md) "Drift" for every field. In short: a drift starts when the **mean rear-tyre slip**
exceeds `Slip_angle_to_enter_drift` (a brake tap drops the threshold to `0dce38f7`, recovering at `b5290695` °/s;
the brake pedal blends it towards `8f151151`), with every wheel down, above `Minimum_speed_to_enter_drift`, not
touching a wall, the handbrake off and no burnout. It ends after 0.5 s once the rear slip is below
`Drift_angle_to_exit_drift`, or at once when airborne > 1 s, in reverse gear, under 0.5 m/s or when the handbrake
comes on. A signed **drift scale** integrates throttle × body slip (`ce8870ad`, full at `c1bc8c79`°), steering
into the drift × throttle (`_from_steering`; `_from_counter_steering` and negative when counter-steering), brake,
the handbrake and a constant decay; it starts at `e6abd30f`, or at the `HandbrakeAbility` value right after the
handbrake is let go. It sets a **yaw torque** into the drift (quadratic Bézier in the scale through `Minimum_` /
`Mid_yaw_torque` and a top that is `Donut_yaw_torque` below `Donut_speed_limit_low` and `Maximum_yaw_torque`
above `_high`) × a steering factor (`a5c9496f` no steering, 1 full lock into the drift, `70528afa` full
counter-lock, + `2ef426dd` × throttle); with every wheel down a **side force** of mass × `09832a93` × f along the
body's side axis **outwards**, opposite to the torque's turn (f ramped by body slip from
`Drift_angle_for_side_force` to `Minimum_drift_angle_for_side_force`, reduced by `Decay_rate` per degree above
`Drift_angle_for_decay` while steering into the drift, × scale and speed ramps, × (1 + a yaw-rate boost of gain
`Side_force_multiplier` while counter-steering), capped at 5); **damping** of the yaw rate by (1 − peak scale ×
`Drift_angular_damping`)^(60 dt) and of the sideways velocity by (1 − `ff5dcf85`)^(60 dt), fading over
`Time_to_blend_damping` after the exit; and a **speed hold** that pushes along the travel with mass × shortfall ×
throttle × `55930ac1` × 60 towards the entry speed (at most `35e937b8` mph ahead). Donuts are this torque at low
speed; `DonutAbility` is not read by the game. `Maximum_drift_angle`, `Steering_angle_multiplier`, `Torque_decay`
are not read either.

### Aerodynamics and airborne (`chassis.rs`)

Drag per body axis F = −½ K C |v| v with C = (`Side_drag_coefficient`, `Top_drag_coefficient`,
`Front_drag_coefficient`), K = 1.2 (read from the running game, `.bss` 0x011872a0); above
`f1538c51` mph the front coefficient ramps to `b8786ac2` at `4aadb133` mph. Lift = `Lift_coefficient` ×
`ca386558` × |v_forward| |v| along the body's up axis at z = `b821de16` (−0.5 v² N on the 911: downforce).
`Down_force` / `Down_force_lift_coefficient` are not read by the game. Once a wheel leaves the ground gravity
becomes `In_air_gravity_coefficient` × g, and for `Take_off_behaviour.123d2559` s the body's pitch, yaw and roll
rates are damped per tick (`8b456140`, `312bd67d`, roll by roll rate). `Top_speed_MPH` is not read by the
game's physics (the camera's FOV still uses it).

### Nitrous (`mw_handling::nitrous`, 2026-10-06)

Update step 5 (`FUN_009d2aa0`, decomp.md "Nitrous and upgrades"): while the tank burns, the controls' nitrous
amount (`NitrousUsage.845c3879` = 1) makes the engine's torque multiplier lerp(1, `Engine.Nitrous_strength.e9de413e`,
amount) (1.9 on the 911), and the gearbox upshifts on rpm alone with the top gear unlocked (`FUN_009d4c20`). The
tank (0–100) burns `NitrousUsage.053c5a6a` per second (24 stock: 4.17 s; the other variants carry 18 and 16) and
starts a burn only above 10 % (`7ebf362a` = 0.1). It starts full and does not refill yet. Left Shift / pad A;
the HUD gauge follows the tank. The punch (mode 1) and jump (mode 3) forces are described but not run.

`--test nitrous` on the 911 (accel without nitrous unchanged: 0–100 3.72 s, quarter 11.58 s, top 307.6 km/h):
0–60 mph 3.37 s, 0–100 km/h 3.47 s, tank empty at 4.17 s (122 km/h, third gear), 0–200 km/h 10.33 s (11.05
without), quarter mile 11.25 s, top 307.4 km/h. Low gears are traction-limited, so most of the gain is in third.

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
| `MinFieldOfView` / `MaxFieldOfView` | 70 / 95 | **data** FOV, `lerp(min, max, clamp(mph / 125))` (`FUN_005c76a0`); read as vertical (see below) |
| `Speed_based_movement.0a78c15f` | −1 (−1.2…0) | **code** (`FUN_005d12b0`) eye z target at full speed, smoothed with the acceleration offset below |
| `….7d4ec535` | 1.7 (1.45…4.4) | **code** eye height at full speed: `eye.y += (this − eye.y) · h`, unsmoothed |
| `….b028673b` | 1.5 (1.35…3.1) | **code** look-at height at full speed, the same form |
| `….b6010ee1` / `53e4b4f9` | 0 / 50 (50/75/100) | **code** speed band (mph): `h = clamp((v − b6010ee1) / (53e4b4f9 − b6010ee1), 0, 1)` |
| `High_Speed_Shake.Amplitude`, `Frequency`, `Power` | 1.2, 1, 1.5 | **data** shake = Amplitude × f^Power |
| `….Minimum_Speed_for_Shake` / `Maximum_Speed_for_Shake` | 0 / 180 (125…208) | **data** f = speed fraction between them (mph) |
| `Acceleration_movement.bd3a7cfc` / `7c9039e2` | −2 (−2.5…−1.9) / 0.5 | **code** eye z offset at full acceleration / deceleration (m), × `|a_long|` / a_full |
| `….fdd25ee3` / `285f2260` | 2 / 2.5 | **code** rate (1/s) of the first-order lag of the z offset while accelerating / decelerating |
| `….8e1310f6`, `a8cc9c9b`, `65b4f9c6`, `0da33093` | 1.5, 1.75, −1.2, 2.5 | read by `FUN_005d12b0`'s vertical stage (cam + 0x554), not ported |
| `Camera_Roll.Minimum_Lateral_Acceleration_for_Roll`, `3f8ced67` | 10, 30 (30…40) | **data** / *hyp.* (the maximum, by its place and values) lateral range, mph/s |
| `….Minimum_Angular_Velocity_for_Roll`, `02dd11bc` | 0.2, 2.5 (2.5…3) | **data** / *hyp.* yaw-rate range, rad/s |
| `….Maximum_Roll_Angle` | 4.7 (3…6) | **data** roll = sign × max × max(f_lat, f_yaw)^`Camera_Roll_Factor_Power` (2.5) |
| `….Roll_Angle_Change_Intensity` | 2 (2…2.5) | *hyp.* rate (1/s) the roll follows its target |
| `Deceleration_pitch_change.Minimum_` / `Maximum_deceleration_for_pitch_change` | 2 / 12 (11…12) | **data** deceleration range, mph/s |
| `….Maximum_Pitch_Change`, `Pitch_change_curve_power` | 4.5 (2.5…6), 2.25 | **data** pitch = max × f^power (nose down) |
| `….Pitch_change_intensity` | 2 (1.75…2) | *hyp.* rate (1/s) the pitch follows its target |
| `Yaw_spring_modification.Low_speed` / `High_speed` | 0 / 100 | **data** speed band (mph) |
| `….5a74d44d` / `63fb2207` | 0.075 / 0.05 | yaw spring at **high / low** speed (recorded cam + 0x6d4: 0.05 at low speed rising to 0.075 at 100 mph), a fraction per 60 Hz tick (sorted after `Low_speed` / `High_speed`): 4.7/s at rest, 3.1/s at 100 mph |
| `YawDriftSpring` | 0.055 (0.045…0.07) | **data** name; *hyp.* the yaw spring while drifting (3.4/s) |
| `Global_params.f50dccb9` (`CameraGameplayShakeEffect`) | see below | **data**, not used by the chase camera's impact shake |
| `Global_params.Impact_shake.*` | see below | **code** `FUN_005cd040` / `FUN_005dfe90` (decomp.md "Camera effects") |

Impact shake (`Impact_shake`, the same on every car; ported as camera.rs `ImpactShakeState`): a level decays
by 0.165 per 60 Hz update and is raised (max) by each contact to `min((clamp((x − 5) / 55, 0, 1) × x)^0.75, 4)`;
the view turns by `2 × level × (x_noise + jitter)` degrees about the camera's x, y (noise spring kicked by
±`WobbleMagnitude` 0.5, step 30 × dt, plus a ±`ShakeMagnitude` 1 jitter) and z (jitter only). Model decisions:
x = the body collider's normal contact speed in m/s, a touch-down after ≥ 0.2 s with no wheel down counts as a
contact of the vertical speed, the lanes map to x / y / z in that order, and the shake pivots on the eye (the
original's pivot argument was not traced). Results: `--test crash` levels 2.9 and 4.0 (the cap), view 2.3-2.6°
RMS over the 0.3 s after the hit, 10-13° peak; `--test landing` (151 km/h) level 0.77, 1.6° RMS, 7.9° peak.
Recorded original: crashes 2.2-2.6 (one 3.8), landings 1.0-1.1, 3.0-3.6° RMS per axis at level ~2.5.
`CameraGameplayShakeEffect` (springs: `Pitch` 250 / 0.4 / 450 / ±20°, third translation axis 100 / 0.5 / 100 /
±0.2 m) is not on this path; its user is unknown (the previous port kicked these springs, a guess, removed).

Bonnet / bumper (`CameraGameplayBonnet` / `Bumper`): eye `6e08f788` ((0, 1.05, 0.8) / (0, 0.425, 1.5)), FOV
95→105 / 95→120 (by speed², like the chase camera), and `3f017c6b` ((0, 1.32, 0.21) on the roof / (0, 0.6, −1.5)
at the tail), *hyp.* the eye of the look-back view. 19 other floats are unnamed (one candidate name,
`AccelerationResponse` = 0.05, fits but has no sibling to confirm it); they are not used yet, so the in-car
views are rigid. The bumper eye sits inside the body's nose, so the car is hidden in that view.

Unused and unnamed: the `Acceleration_movement` floats `b40b3fb7` (0), `77e7de37` (50), `f393abd1` (0/−0.9) and the
vertical stage's four (above), `Yaw_spring_modification.78527964` / `397fce82` (10 / 10) and `Decay_rate` (0.01),
`Speed_based_movement.b6010ee1` (0), `Speed_based_height_change` (1.59 / 1.973; heights, but they do not
track the eye as closely as `7d4ec535`), `Gradient_Adjustments` (`Downwards` 0.5 / 1.5, `Upwards` 0.5 / 4,
`Maximum_Gradient` 10°), the top-level `c6ca2dc7` (5 / 10), `6d906d74` (4) and `67660e20` ((0.55, 0.09, 1)),
`Lens_Properties` (`FocalDistance` 90, 1.4, false: depth of field, off), and the rest of `Impact_shake`
(`ShakeMagnitude` / `WobbleMagnitude` vectors, frequencies).

**Source status (2026-10-06):** the whole chase-camera model above is built from field names and values, not
from the game's code. A search of the dump for the code that reads `CameraGameplayExternal` (decomp.md
"## Cameras") did not find it, so the camera's sway (the target frame, yaw / lateral springs, the drift
spring's use), weight (how the distance, height, pitch and FOV springs are integrated) and avoidance (what is
cast and how the camera responds; the GeneSys types `CameraTransformCollider` and
`CameraTransformEntityAvoider` suggest more than a push-in along the boom) are all still model decisions or
*hyp.* rows. No camera rule changed in that round; the baseline trace numbers (drift, accel, corner, crash)
are in the research log, 2026-10-06 round 4, to compare a port against.

**Round 7 (2026-10-06): from the code now** (decomp.md "Speed and acceleration movement"): the speed band
and the eye / look-at heights (direct, linear in `h`), and the eye's z offset: a first-order lag
(`z += (target − z) · rate · dt`) towards `0a78c15f · h + (bd3a7cfc | 7c9039e2) · |a_long| / a_full`, rate
`fdd25ee3` accelerating / `285f2260` decelerating, `a_long` the acceleration along the velocity (signed by
the speed's direction in our port). This replaces our critically damped spring (ω = 2.5) and the 1.5 m
braking limit (`8e1310f6` belongs to the vertical stage). Model decisions in that rule: `a_full` = 20 mph/s
(`ACCEL_FULL_MPHS`, stands for `.bss` `DAT_0110e770`), the offset clamp ±3 m (`ACCEL_Z_LIMIT`, for
`DAT_0110e680`), no step clamp (`DAT_0110e4d0`), the rate's `g · k · (1 − w)` term (vehicle + 0x2b20)
and the 1.85 low-pass above `DAT_0110e640` left out. Still ours: the vertical acceleration stage, roll,
deceleration pitch, the yaw spring's value (0.075 → 0.05 by speed), the drift lean, the bob
(`FUN_005c8880`), the gradient spring, FOV, shakes and the collision push-in (no world query exists in the
decompiled camera code; the director update `FUN_005d5e20` is not in the dump). `FUN_005d0b90`,
`FUN_005c7a80` and `FUN_005c9380` are still unread.

**Round 5 (2026-10-06):** the code is found (decomp.md "## Cameras": the chase camera's update is
`FUN_005d4970`, director sub-camera + 0x8d0) but not yet ported, so the rows above are unchanged. What the
first helper read already contradicts: our yaw spring blends `5a74d44d` / `63fb2207` by speed only, while the
game's `FUN_005c90a0` picks it by the car's yaw rate in car space (thresholds in `Yaw_spring_modification`
+0x14 / +0x20, rate-limited by +0x10 per update) blended with a speed factor and the mods profile's value 0x29.
That row stays a guess until the helper is read to its end.

**Round 6 (2026-10-06): what is from the code now and what is still ours.**
- *From the code* (decomp.md "The followed frame"): the camera follows the car's whole orientation as three
  Euler angles (pitch, yaw, roll; `FUN_005e14d0`, keeping the previous angles within 0.01 of a vertical
  forward axis), each smoothed by `angle += (1 − retain) · wrap(target − angle)` with
  `retain = 1 − clamp(K · dt · 60, 0, 1)` (`FUN_005c89c0` / `FUN_005c75b0`; this linear-in-dt form replaces
  our `1 − exp(−rate · dt)`, identical at 60 Hz, stiffer at long frames); the pitch's retain is multiplied by
  `min(|speed| · 0.02, 1)` (rigid at standstill, full lag from 50 m/s); the pitch stiffness eases 0.1 per
  update to its default (`FUN_005c8e40`). The yaw-lag cap (0.6 rad) is removed: no clamp exists in the code
  read. This replaces our "up vector eased at 2/s" slope following and the yaw ease.
- *From the live recording (2026-10-06, `ref/live1`, 14,383 distinct camera frames, 0-134 mph; checked with
  `tools/camreplay.py`, see decomp.md "Cameras: the live recording")*: the yaw step rule of `FUN_005c75b0`
  reproduces the recorded smoothed yaw (cam + 0x594) to 0.0002° median when fed the recorded stiffness.
  Ported from the recording and the live state block: pitch stiffness 0.55 (state + 0x70; was 2/60), its
  speed gate `min(0.02 × speed, 1)` in **mph** (vehicle + 0x1620; was m/s), the followed frame without roll
  (cam + 0x598 stays within 0.25°; the view's only roll is `FUN_005c9380`'s cam + 0x660), and the FOV
  `lerp(70, 95, clamp(mph / 125, 0, 1))` set directly each frame (`FUN_005c76a0`, state + 0x88 / + 0x8c =
  0 / 125; was `(speed / top speed)²` eased at 3/s). Replay error, before → after: yaw 3.00° → 2.17° RMS
  (p99 6.3° → 3.0°), pitch 0.35° → 0.21°, frame roll 0.99° → 0.01°, FOV 11.1° → 3.8° RMS.
- *Fitted from the recording (model decision)*: the yaw stiffness. Recorded cam + 0x6d4 is 0.09 at a
  standstill (= state + 0x74, the per-player default) and otherwise `0.05 + 0.025 × clamp(mph / 100)`
  (93 % of frames within 0.002); that is the data pair `63fb2207` 0.05 → `5a74d44d` 0.075 **rising** with
  speed (we had it falling), but which operand of `FUN_005c90a0`'s lerp gives it is still unread. The
  standing threshold (0.5 mph) is ours.
- *Still ours*: the drift lean and `YawDriftSpring` (no drift in the recording), the eye position (the
  recorded eye sits at z = −5.94 at rest, not the data's −4.6, and moves forward to −4.0 at 130 mph; its
  look-z lane follows `−1.2 + 4.55 × (1 − tan 35° / tan(FOV/2))` to 0.04 m RMS, a FOV-compensating dolly
  that is not read or ported), the roll and deceleration pitch models, the shakes (decomp.md "Camera
  effects") and the collision push-in. The FOV residual (3.8° RMS) correlates with acceleration (+0.1° per
  mph/s): a further term after `FUN_005c76a0` that is not found. The airborne branch of `FUN_005c8e40` is not
  ported (the car exposes no airborne state).
- *Model decisions with the game's address*: `.bss` values still unread: `_DAT_0110e8f0` (yaw spring's
  reference speed), `_DAT_01189f60` (fixed-yaw mode's yaw).

Model decisions (not data): the FOV is vertical (as horizontal the chase view is far tighter than the
original's; the bonnet and bumper views look very wide this way); acceleration is measured from the car's
speed between physics ticks; the drift lean (`DRIFT_LEAN` 0.5 of the slip angle);
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

## Crash camera (`mw_assets::crash`, `crates/mostrusted/src/crash.rs`, 2026-10-06)

What the code does is in [decomp.md](decomp.md) ("Crashes"); here the data and what we made of it.

**Data**:
- Rules: `DamageBarProfiles.PlayerOffline` (PHYSICS/PHYSICSCONFIG) → `DamageBarProfile` `01000000001028bf` →
  `CrashingRules` `0100000000102928`. `WorldRules`: damage > 30 or closing speed > 70 mph is a crash
  (`DamageOverride` 50, flag off). The offline player has no `LandingRules` and no `PlayerRules`;
  `TrafficRules` 20 (damage), `RoadblockRules` 40, the AI / cop set (`22061460`) 33, two prop-ish sets 150 /
  150 + 100 mph. Eight more rule sets in GAMELOGIC/GAMEPLAY belong to modes (landing at 38 / 60 / 100 mph).
- Sequence `0100000000068b43` (`FeedbackMessageGroup` 457707, `Sequences2[1]`), real-time seconds:

  | time | item |
  |------|------|
  | 0–0.5 | slow motion 1 → 0.1006 (two-key curve, flat tangents); gameplay camera |
  | 0.5–2.0 | slow motion 0.14; bystander `0100000000068b41`: camera at `SAS.Analyser["Freeway"]` (a `SimpleSceneAnalyser`, offset (−5, −8, 10)) + (0, 3, 0), aim at the car (`Players.CameraTarget`), `Screen_Offset_Y` 0.207, `Tracking_Speed` 0.2, `TrackingAcceleration` 0.01, FOV 10…70.93 with `Use_Zoom` off, `005c8fc7` true |
  | 2.0–4.5 | slow motion 0.22; bystander `0100000000068b42`: camera at the car + (−1.5, 1, 3), aim at the car, FOV 10…90, zoom off, `005c8fc7` false |
  | 0–4.5 | `HideHudSequenceItem`; `SceneAnalysisSequenceItem` (`010000000006ccb3`, five analysers) |
  | 0–4.75 | `UILayout` `0100000000019c66`: black letterbox bars, 94 px top / 100 px bottom of 720, slide 150 px out over 0.25 s |
  | 0–5.5 | mixer and bus automation (music and engine ducking), `RaceCarEffectSequenceItem` property 5 (0.45 → 1 → 0.46) |

- Reset (`GameLogicConfig`): rolling at `max(30, 0.7 × pre-crash mph)`, capped at 50 mph after a crash.

**Model decisions** (no source yet):

| What | Ours | Missing source |
|------|------|----------------|
| crash test | closing speed only (avian's pre-solve `normal_speed`, contacts with \|normal.y\| < 0.7 against static scenery) | the impact damage in damage-bar units (`1617a710` rule) and the solver's own closing speed |
| reset time | when the cameras end (4.5 s) | `GameMode+0x44` (no `GameMode` instance found) |
| reset pose | the pose 1 s of game time before the impact, upright, moving forward | the game saves the last pre-crash frame and moves it onto the road (`FUN_00648060`) |
| `Freeway` shot | analyser offset x / z in the pre-crash heading's frame from the car at the shot's start, on the ground there (ray down), plus `Y_Offset`; fixed for the shot | the analyser's update (its y = −8 is not understood) |
| car shot | car position + offset turned by the car's current heading | how `Players.CameraTarget` binds (full rotation?) |
| aim | moves `Tracking_Speed` of the way to the target per 60 Hz tick; `TrackingAcceleration` unused | the tracker's maths |
| screen offset | the view pitches so the target sits `Screen_Offset_Y` × half-height above centre | sign and units |
| FOV | `Maximum_FOV` (vertical, as camera.rs) while `Use_Zoom` is off | which FOV a non-zooming bystander holds |
| control | none until the reset | inferred from `Player.CanDriveAway` |
| not done | `RaceCarEffect` property 5, audio ducking, upside-down crashes (`.bss` threshold), traffic / cop / prop rules | |

Test: `mostrusted --drive 911 --test crash [--crash-wall-width 2] --screenshot <scratch>/crash.png` adds
`-crashcam-0-slowmo` (0.3 s), `-1-freeway` (1.25 s), `-2-car` (3.25 s) and `-3-reset` (5 s) to the before / after
shots; `MW_CRASH_TRACE=<file>` logs each impact's decision, the sequence frame by frame (time scale, shot, eye,
aim, FOV, car) and the reset. 911 into the 80 m wall (2026-10-06): `impact 7.150 71.5 70.0 1` (71.5 mph closing,
a crash), shot 0's eye at (−5.07, 3.0, 87.92) beyond the wall, the reset to (0, 0, 49.04) rolling at 50 mph
(pre-crash 71.5 × 0.7 capped). With the default 30 m wall the `Freeway` shot looks at the wall's back (the shot
stands 10 m past the impact); a 2 m pillar shows the car.

## Model decisions (not from data)

| Constant | Value | Why |
|----------|-------|-----|
| ~~`LAT_GRIP` / `LONG_GRIP`~~ | removed 2026-10-06 | the code's caps are μ × N × data axle scale × surface grip × camber × gameplay grip multipliers (1 outside timed effects): `FUN_009c5da0`, `FUN_00905910`, `FUN_009d8bd0` (decomp.md "Tyres"); N and the downforce share confirmed on `ref/xkr_launch` |
| tyre effective mass | the corner's sprung mass, all four wheels from the same velocity | the game uses the chassis's effective mass at the contact (`FUN_009ca610`) and solves the wheels one after the other, each seeing the velocity the previous left (`FUN_009289f0`); same steady state, different transients |
| nitrous multiplier target | engine full-throttle torque | `FUN_009d2aa0` writes a four-lane multiplier at its component's +0x10; the reader was not found (the call's `this` is not recoverable in the caller `FUN_00904260`); scaling the full-throttle torque is assumed |
| nitrous tank | 100 units, starts full, no refill, burn starts above 10 % | the tank is gameplay code, not traced; capacity from `Restrictions.c441f502` = 100 (1 in most instances), rates from `NitrousParameters` (handling.md "Nitrous") |
| punch / jump force scale | 0.44704 (`nitrous::PUNCH_SCALE`) | **from the game**: `.bss` 0x0118a018 (read live), mph → m/s, so `0798d257` is mph/s; the modes 1 / 3 are still not run |
| blend-input scale | 1 | the pure → slip blends take κ and α × the tyre constraint's +0x210, whose writer is not in the dump; measure with the probe |
| surface-change fade | none (instant) | the game fades the grip of the surface just left over a time and uses the lower of old and new (`FUN_009ceb00`, `FUN_009d4460`); not ported |
| resistance at rest | clamped to what stops the contact | ours: the game applies rolling resistance in full against the sign of the motion (`FUN_009d4a10`), which flips every tick at a standstill |
| test-track surface | `Surfaces.Default_surface` | ours: the flat track has no collision tags; the game's default surface (grip 1, rolling 0.01, no slide drag) |
| `CLUTCH_CAP` | 5000 N m | **from the game**: `.bss` 0x01186f70 (read live), the cap in `FUN_009d7ac0` |
| shift phases | 10 / 50 / 40 % | **from the game**: declutch 0x01186ee0 = 0.1, clutch-in 0x01186fd0 = 0.4, neutral the rest (`FUN_009d1560`, `FUN_009d6880`, `FUN_009ce490`); confirmed tick by tick in `ref/xkr_launch` |
| downshift hysteresis | 2 mph | **from the game**: 0x011872e0 in `FUN_009d93b0` |
| idle governor | idle + 20 rpm, 0.05 / rpm, up to 0.5 throttle | **from the game**: 0x01186f30 / 0x011871c0 / 0x011873f0 in `FUN_009d9960` |
| throttle time base | 60 | **from the game**: `c6deb7a6` / `36676b9c` × `.bss` 0x01187250 = 60 (read live 2026-10-06) |
| clutch-in throttle | 0.7 √progress | **from the game**: `FUN_009d6af0` (upshift: 0.7 √progress whatever the pedal) / `FUN_009d6ca0` (downshift: pedal × 0.7 √progress); scale `.bss` 0x011874d0 = 0.7, exponent 0x01187470 = 0.5 (the CRT pow argument, from the disassembly), read live 2026-10-06. Gearbox +0x170 == 4 (throttle 1) not identified |
| coast declutch | 20 mph | `.bss` 0x011872d0 in `FUN_009d9ee0`: lanes 2–3 read 20 (`consts.txt`), lanes 0–1 assumed the same |
| downshift gate | 1 s on the ground | **from the game**: vehicle +0x1ed0 (`FUN_009ae840`, reset only with no wheel down and the body above `.bss` 0x01186630 = 1.8 m/s) > `.bss` 0x01187580 = 1.0 (read live 2026-10-06) |
| kick-down window (`KICKDOWN_WINDOW`) | 0.2 s | **from the game**: `.bss` 0x011871e0 = 0.2 (read live 2026-10-06) in `FUN_009dc3c0`; gearbox +0xb8 added to the time still taken as 0 |
| gearbox-mode flag | down | `FUN_009da600` passes a flag from a virtual call on the vehicle (`FUN_009b3030`, vtable +0x34) that keeps the race gearbox while up; not identified (race start? AI?), so the cruising box is always allowed |
| stall check | `Idle_RPM` × 0.9 | **from the game**: `.bss` 0x011874a0 = 0.9 (read live); the launch and every shift phase but the downshift declutch drop to standstill (clutch open, 1st) below it |
| launch torque | lagged | the original reaches full torque in the first tick of its launch (`ref/xkr_launch`), which the lag does not; source not found (`FUN_009262a0`'s override list?) |
| `BURNOUT_MPH` | 20 | **from the game**: `.bss` 0x011866f0 (read live) in `FUN_009ab230` (now `drift::BURNOUT_MPH`) |
| `DRAG_K` | 1.2 | read from the running game (`.bss` 0x011872a0); was 1 |
| `BUMP_TRAVEL` | 0.06 m | from the 911's strut geometry; not read per car. Searched for the strut joint's builder (`WheelSuspensionConstraint` readers in the vehicle build, `009A`–`009D`): not found; `7fc0dc41` (0.09 / 0.08 m on the 911) is the likeliest field |
| damper slots | `0c5f89b3` × c in rebound | the setup stores c and `0c5f89b3` × c (spring +0x2c / +0x18, `FUN_009b29b0`); the function applying them was not found |
| anti-roll unit | N m per radian, scale 60 | the scale is read live (`.bss` 0x01185d70) and applied by the bar's builder `FUN_00933a60` to the stiffness only (the damping is 2 ζ √(k_data I), from the same function); the unit, and so the angle the stiffness multiplies, is inferred: the solve that applies the bar is likely the undecompiled `FUN_00932af0` |
| anti-roll input and form | compression difference, explicit force | the bar element (type 2, `FUN_00933a60`) names the axle's two wheel objects and the chassis, so it may act on the body's roll against the axle rather than on the compression difference; its per-tick solve was not found. Ours applies k Δx / t² + c Δẋ / t² at the wheels with last tick's compression rates (stable at the game's damping) |
| drift time base | 60 | **from the game**: damping `(1 − c)^(dt × 60)`, the 60 at `.bss` 0x01184cc0 / 0x01185640 (read live), `FUN_00907b50`, `FUN_0091dad0` |
| drift speed-hold scale | 60 | **from the game**: `.bss` 0x01187250 = 60 (read live 2026-10-06), `FUN_009d0420` |
| counter-steer opening exponent | `Max_counter_steering_power` | `FUN_009d2e20` raises the opening to a power whose exponent the decompiler drops (a register load); inferred as the one counter-steering field the code does not otherwise read |
| steering neutral offset's q | body slip / 35° × min(1, speed / 10 mph), clamped to ±1 | read from the original's telemetry (`ref/xkr_launch`: q / slip = 0.0286/° over 0.5°–33°, the fade constant over 1–4 m/s); its writer (vehicle +0x2c70) was searched by offset in the whole dump (0x004–0x011: the other writers are of the game-mode object at +0x2f3fc, `FUN_006e0580` …) and as a typed index (`[0xb1c]`): not found. 35° equals the XKR's `Maximum_steering_angle` (one car cannot separate them); the ±1 clamp is ours |
| drift collision exits, wall-hit steering reset | not modelled | `FUN_009d83e0` codes 1 / 3 and `FUN_008f7ab0` compare a contact value with 6; which contact quantity (impulse, speed) and unit was not settled |
| per-wheel surface factor on drift torque, side force, damping | 1 | `FUN_009cede0` (mean of a per-wheel value); its writer was not traced |
| ~~rolling resistance Crr 0.015~~ | removed 2026-10-06 | the game's comes from the surface: `5c2a9f18` × N per wheel plus the slide drag (`FUN_009d4a10`) |
| wheel contact | one ray per wheel | the game ties each wheel record to the chassis with a constraint holding the wheel's ground contact points, iterated in sub-steps (`FUN_0093f200`); the tyre friction is one pass after them |
| strut stop slack | 0 | the chassis gets share = max(d − min(+0x470, 0.1), 0.1 d) / d of the stop impulse at penetration d (`.bss` 0x01186bf0 = 0.1, 0x01186b10 = 0.1, read live 2026-10-06); equals ours (share 1) only when the constraint field +0x470 is 0, not read (the wheel constraint was not located from the vehicle in the snapshot). Not changed: ours has no wheel bodies to take the rest |
| `STOP_ITERATIONS` | 4 | passes of the strut-stop impulses per tick; the game iterates contacts in sub-steps of the island step (`FUN_0093f5a0`), count not read |
| wheel ray start | one wheel radius above the mount | stands in for the game's wheel body colliding with the world: a ray from a mount already under the ground returns a zero normal (solid box) or nothing (trimesh back face), and the car stayed sunk on its body box |
| wheel axle/side | by position | some specs label wheels the other way round (the Veyron's "Front" wheels are at the back) |

## Measurements

Ground tests (2026-10-06, `--test landing|climb|jitter`; `MW_SUSP_TRACE=<file>` writes a line per tick from 10 m
before the obstacle: height, sink below rest, vertical speed, pitch, body contact, each wheel's compression /
travel + overtravel). `landing`: 130 km/h into a 5.7° kicker to a 3 m plateau that ends in a sheer drop;
`climb`: 120 km/h into a 30° incline (`MW_TEST_SLOPE`, `MW_TEST_KMH`); both are trimesh ramps. `jitter`: 4 s at
rest, then 4 s at a steady 100 km/h; peak-to-peak and dominant frequency of height, pitch, roll and their rates.
911 before the strut stop: landing touches down at −8.6 m/s, all four wheels 0.34 m past full compression, the
body box on the ground (sink 0.478 m) for the rest of the run; climb STUCK at 0 km/h with the same sink. After:
landing sink 0.055 m (the bump travel), overtravel 0, settled 0.45 s after touchdown (test settle 2.72 s from
10 m before the ramp); climb ends ok on the plateau (deepest 0.10 m, a one-tick 0.12 m overtravel on the rears at
the kink, the body's nose touches the 30° slope and the car hops; settle 4.27 s). Jitter (unchanged by the fix):
roll 0.45° p-p at 11.2 Hz at 100 km/h (roll rate 37°/s p-p), 0.06° at rest; it is the anti-roll bar's damper
(without it 0.008°): see the research log. Accel / brake / corner unchanged (3.72 s, 38.5 m / 2.63 s, 2.57 g).
Anti-roll damping from `FUN_00933a60` (2026-10-06, round 4): jitter at 100 km/h roll 0.449° p-p at 11.2 Hz →
0.011° at 1.0 Hz (roll rate 37.3 → 0.07°/s p-p), at rest 0.060° → 0.000°; pitch 0.76° → 0.74° p-p at 1 Hz (not
the bar). Accel 3.72 s and corner 2.57 g (roll 0.0° in both) unchanged; brake 38.5 → 38.6 m (2.63 s); landing
sink 0.006 m, deepest 0.055 → 0.058 m; climb deepest 0.101 → 0.056 m, body contact 0.28 → 0.35 s, settle
4.27 → 4.62 s, 133 → 126 km/h on the plateau; handbrake (59°/s) and drift (peak 4.5°) unchanged.

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

**Current master (2026-10-06, all three re-reads merged: drivetrain, tyres, steering / drift / handbrake).** The
tables below were each measured on one branch alone; together they give:

| Car | 0–100 km/h | top speed | 100–0 km/h | skidpad | drift test | handbrake | donut |
|-----|-----------|-----------|------------|---------|------------|-----------|-------|
| Porsche 911 Carrera S | 3.72 s | 307.5 km/h | 39.5 m, 3.23 s | 2.57 g | no drift (peak 4.5°), exit 74 km/h | 1.2°, 59°/s | 130°/s at 40 km/h, r 4.9 m |
| Bugatti Veyron SS | 2.45 s | 420.5 km/h | 34.3 m, 2.73 s | 2.58 g | 19.3° peak, held 4.2 s, exit 92 km/h | 1.5°, 50°/s | 136°/s at 39 km/h, r 4.6 m |

XKR replay of `ref/xkr_launch` on master: the revs rise ~0.5 s early after the handbrake release (3612 against
1568 rpm at 1.5 s: the original's unexplained 0.45 s launch delay plus the handbrake's front-axle drive), so the
launch runs ~5 mph ahead from 2 s; the 1→2 shift and the 2nd-gear revs follow the original's shape. The
original's run ends in an impact at 6.5 s, so the RMS (speed 15.2 mph) says little. Next: record the original's
brake-tap drift, handbrake turn and donut (`mwtool capture`) to check the drift balance between cars.

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

Earlier the same day, still on the calibrated grip, after the update loop, steering, drift and handbrake were re-read from the code (2026-10-06: side force
outwards with its cap of 5, handbrake as a countdown state that ends drifts and moves the drive to the front axle,
no handbrake lock, steering one tick behind the tyres; research log), before → after:

| Car | skidpad | handbrake test (peak slip, yaw) | drift test | donut |
|-----|---------|---------------------------------|------------|-------|
| 911 | 1.02 → 1.02 g | 4.3° 36°/s → 2.9° 29°/s | 22° held 4.4 s, exit 68 km/h → spins: peak 72°, held 2.6 s, mean 43°, down to 10 km/h | unchanged (15°/s at 120 km/h) |
| Veyron SS | 1.00 → 1.00 g | 6.7° 45°/s → 3.3° 26°/s | no drift (4°) → unchanged | unchanged |
| MP4-12C | 0.91 → 0.91 g | 2.9° 34°/s → 3.6° 40°/s | 12° held 3.7 s → 33° held 5.2 s, exit 51 km/h | unchanged |
| F-150 Raptor | 2.01 → 1.22 g | 13.9° 58°/s → 2.7° 23°/s | no drift (7°) → unchanged | 106°/s at 32 km/h → 29°/s at 16 km/h |

The F-150's skidpad drops from 2 g because the drift's side force no longer pushes it into the corner. Whether
the original's brake-tap drift spins the 911 with this script (steering held at 0.8 into the drift on 0.6
throttle) needs a recording.

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
