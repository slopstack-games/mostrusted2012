# What the game's code says (decompilation findings)

The user supplied a Ghidra decompilation of a memory dump of the running game (`re/NFS13_dump/`, git-ignored;
see [reverse-engineering.md](reverse-engineering.md)). This file records what was learned from it, **in prose
and formulas only**: no decompiled code goes into the repository. Function addresses refer to that dump
(`FUN_<address>`), so the findings can be re-checked.

Status words: **confirmed** = read directly from the code; **inferred** = reasoned from the code and the data
(units, the meaning of unnamed values). Values are the Porsche 911 Carrera S unless stated.

## Reading the dump

- `re/NFS13_dump/code/<block>.c` holds the pseudo-C, one file per 64 KB of address space; `functions.csv` has
  sizes and callees, `strings.csv` the strings.
- **GeneSys data is read in place.** A field is read as `*(resource + 0x28) + FIELD_OFFSET + INSTANCE_OFFSET`,
  so the middle constant is the field's offset in the GeneSys type. `mwtool genesys-types` gives every type's
  field offsets, which identifies which code reads which handling field (and settles what unnamed fields do).
  Nested structs are inline (their own 12-byte header included); `HandlingBehaviour` holds handles to its
  sub-behaviours.
- **Constants** referenced as `DAT_00f…` / `DAT_010…` live in `.rdata` / initialised `.data`, which are not
  encrypted in `NFS13.exe` on disk, so they can be read from the file. Constants at `0x0118xxxx` are `.bss`
  vectors filled by static initialisers; many initialisers are missing from the dump, so about 30 tuning
  constants (listed under each section) are still unknown. The memory dump the pseudo-C was made from
  (`NFS13_dump.exe`, not on this machine) holds their live values; reading `.bss` from it, or from the running
  game with `mwtool memscan`, would settle them.
- Upgrade / tuning overrides: `FUN_00574870(index, value)` applies the car's upgrade profile to a parameter
  (multiply, add or replace); with no upgrade the data value is used as is.
- The sound system registers named getters on `RaceCarEntity` (`FUN_0057e280`), which helped map the vehicle
  object (speed in mph at +0x1620, time in air +0x1630, ...).

## Update loop

- **Fixed 60 Hz tick** (confirmed: a 1/60 s step, with a 2/60 catch-up step when the game falls behind).
  Vehicles update once per tick, no sub-steps; the step is scaled by the clock's time scale (slow motion).
- Order within a tick (`FUN_00904260`): controls are copied to the vehicle; suspension, contacts, tyres and
  drivetrain; body movement, handbrake state, nitrous, a low-speed turn-around assist; steering (its angle is
  used by the *next* tick's tyres); the drift (only while drifting and with every wheel down); final velocity
  update and speed. Each stage integrates its forces into the velocity before the next runs (`FUN_00907840`).
- Input: the pad mapping is `max(0, (raw − offset) × scale)` per binding; steering = right − left. No smoothing
  or ramping of the input, and no central stability control, traction control or ABS.
- Speed-banded lookups use the speed in mph signed by the direction of travel (the full speed, not the forward
  component).

## Steering (`FUN_009d31f0`; `FUN_009d39b0` for steering-wheel controllers)

- **Range** (wheel angle at full input): off the throttle it blends from `Low_speed_steering_range` at 0 mph to
  `High_speed_steering_range` at `High_speed`. On the throttle a second curve runs from the low-speed range to
  `41d22d15` at `Medium_speed` and `98820dfe` at `High_speed` (911: 35° → 24° → 19°); the two curves blend by
  `clamp(throttle − brake)`. In reverse gear the range goes to `Reverse_steering_range` by `Reverse_speed`
  (negative mph). Capped at `Maximum_steering_angle`.
- **Rates** (degrees per second) blend from 0 mph to `High_speed` (not from `Medium_speed`). Moving back towards
  centre: the counter-steer rate if the input points across the current angle, else the centring rate (easing
  off part of the way counts as centring). Steering outwards: the steer rate. Reverse gear: the `Reverse_*`
  rates. While drifting the rate blends towards `Counter_steer_rate_in_drift` with the drift angle up to
  `5277ec9e` (40°).
- **Neutral offset**: the wheels settle at `−q × Maximum_steering_angle × 553bc073` (0.3; 1 in reverse, 0 while
  drifting), with extra lock `a40a6b17 × |offset|` on that side. `q` is a vehicle value whose writer was not
  found (perhaps the travel direction relative to the body: self-alignment). Not modelled yet.
- **In a drift** (`FUN_009d2e20`) the input is remapped: no input holds `Default_steering` (0.24) of the range
  into the drift, full input full lock; the opposite input reaches `ac261613` (−0.15) early in a drift
  (`fb0656a3` = 18° of peak slip) and opens to `939b8fdc` (−1, full counter-lock) as the slip grows from
  `Low_slip_angle_for_counter_steering` (5°) to `fe68bc4b` (40°).
- `Steering_angle_curve` is a type that `SteeringBehaviour` does not contain (a leftover).
  `SteeringWheelPhysics` (one shared instance) only applies to steering-wheel controllers.

## Drift (`DriftBehaviour`; `FUN_009d91a0` per tick)

- **Entry** (`FUN_009d5690`): the mean slip angle of the two rear tyres (degrees) exceeds a threshold, every
  wheel is down, the car is going forwards above `Minimum_speed_to_enter_drift` (mph) and not doing a burnout.
  The threshold is `Slip_angle_to_enter_drift`; a **brake tap** (pressed above 0.25 and released below 0.1 within
  0.6 s) drops it to `0dce38f7` (2°), from where it climbs back at `b5290695` (10°/s); holding the brake blends it
  towards `8f151151` (5°). There is no steering requirement and no separate "entering" phase. The handbrake
  does not start a drift by itself: it locks the rear wheels, and a drift started with it begins with a larger
  drift scale (`HandbrakeAbility` `6ce0fda7` at low speed → `35e18207` at high speed, 0.6 → 0.4).
- **Exit**: after at least 0.5 s, when the rear slip falls below `Drift_angle_to_exit_drift`; forced by more than
  1 s airborne or no wheel contact, a collision, reverse gear, or speed under 0.5 m/s.
- **Drift scale** S (0..1 in the drift's direction, never changes sign), integrated per second: `ce8870ad` ×
  throttle × body slip (full at `c1bc8c79` = 6°); `Drift_scale_from_steering` × steering into the drift ×
  throttle (`_from_counter_steering` and negative when counter-steering); `8a2a92f3` × the throttle's rise;
  `Drift_scale_from_braking` × brake (negative when counter-steering); the handbrake value while it is held;
  minus `Drift_scale_decay`. A drift starts at `e6abd30f` (0.0001).
- **Yaw torque** about the body's up axis: a quadratic Bézier in S through `Minimum_`, `Mid_` and a top torque;
  the top is `Donut_yaw_torque` below `Donut_speed_limit_low` mph, blending to `Maximum_yaw_torque` by
  `Donut_speed_limit_high` (always the maximum when counter-steering). So **donuts are the drift torque at low
  speed**, not a separate state. Times a steering factor: `a5c9496f` (0.5) with no steering, 1 at full lock into
  the drift, `70528afa` (0.1) at full counter-lock, plus `2ef426dd` (−0.1) × throttle. `Maximum_drift_angle` is
  not read: there is no fade with the angle.
- **Side force** along the body's side axis, towards the inside of the drift: mass × `09832a93` (14 m/s²) ×
  a ramp in body slip from `Drift_angle_for_side_force` (1.5°) to `Minimum_drift_angle_for_side_force` (5°),
  reduced by `Decay_rate` per degree above `Drift_angle_for_decay` while steering into the drift, × min(S /
  `Drift_scale_for_maximum_side_force`, 1) × a speed ramp from `d79e7ebc` to `Speed_for_maximum_side_force` mph
  × (1 + boost); the boost comes from the yaw rate (gain `Side_force_multiplier`, decaying at 2 per second).
  Capped by an unknown constant.
- **Damping**: while drifting and for `Time_to_blend_damping` after, the yaw rate is multiplied by
  `(1 − b × d)^(dt × K)`, d = the drift's peak S × `Drift_angular_damping`, b fading 1 → 0 after the exit; the
  body's sideways velocity by `(1 − ff5dcf85)^(dt × K)`. K is unknown (60 fits the per-tick reading).
- **Speed hold**: the speed at entry is kept as a target (at most `35e937b8` mph above the current speed); while
  the throttle is above 0.3 and the brake below 0.1 a force mass × shortfall × throttle × `55930ac1` pushes along
  the travel direction.
- Not read anywhere in the drift code: `Max_counter_steering_power`, `Maximum_drift_angle`,
  `Steering_angle_multiplier`, `Torque_decay`, `Counter_steer_yaw_torque` (cached only). Nothing reads
  `DonutAbility` either.
- Unknown: the damping time base K, the side-force cap, the burnout speed scale, a per-car torque scalar
  reached through the physical definition, and a per-wheel surface factor that multiplies torque, side force
  and damping (1 assumed).

## Drivetrain and brakes

- **Engine** (`FUN_009d9960`): the engine is its own rotating inertia (`Engine_inertia`), driven by its net torque
  and coupled to the wheels only through the clutch. Off the throttle the torque is a three-point friction curve
  in `Power_curve`: `b3066e01` at `Idle_RPM`, `5a6aaf9f` at `f71be7c6` (the peak-torque rpm), `a53d9b5d` at
  `Max_RPM` (911: −50 / −100 / −125 N m). The throttle blends linearly from that curve to the full-load
  `TorqueCurve`. The rpm is clamped at `Max_RPM` (`Rev_limiter` is a cut time in seconds, 0 on 179 of 197 cars).
  `Engine_resistance` is not engine braking: it is the free engine's deceleration (rad/s², inferred) while the
  clutch is open or during an upshift; downshifts blip the engine up instead. An idle governor opens the throttle
  near idle; the throttle has a peak-hold and lag (`c6deb7a6`, `36676b9c`); a noise term (`15d1d117` …) only
  roughens the sound.
- **Clutch** (`FUN_009d7ac0`): a speed-matching link between the engine and the driveline, scaled by the clutch
  engagement and capped by an unknown torque. **No efficiency factor** anywhere. Each driven wheel gets half of
  its axle's torque; `TransmissionInertia` is added to each driven wheel's inertia.
- **Differentials** (`FUN_009d8140`): `Front_` / `Rear_Differential` = c limit the axle's wheel-speed ratio to
  1/c (0 open, 1 locked; 911 rear 0.75). `Center_Differential` is 0 on every car.
- **Gearbox**: only the `RaceGearbox` ratios and shift points are used; the `CruisingGearbox` only supplies a
  slower shift time for kick-downs. Upshift when rpm > `Gear_up_RPM` **and** mph > `Gear_up_speed` (may skip
  gears). Downshift when speed < the lower gear's `Gear_up_speed` (with hysteresis) or rpm < `Gear_down_RPM`, to
  the highest lower gear still above its own down rpm. Shift time = `Gear_change_time` × (1 + 0.15 × random),
  split into declutch / neutral / clutch-in with the throttle cut. **Top-gear lock**: flags `13bfea5a` /
  `7b20506a` lock the last gear unless a profile option ("long gears", inferred) or nitrous is active, so the
  911's race box has 6 usable gears, not 7.
- **Launch**: clutch engagement = max(throttle, rpm / `Gear_up_RPM` of 1st), fully closed at ≥ 0.95.
- **Top speed**: `Top_speed_MPH` is never read by the physics. Top speed follows from torque, drag (with its
  over-speed ramp, below) and the rpm clamp in the highest unlocked gear.
- **Brakes** (`FUN_009b1180`): torque per wheel = `6ade4bb6` (front) / `66251131` (rear) × pedal² × a ramp that
  rises at 10 × pedal per second. A brake can stop a wheel but never reverse it. **No ABS, no traction
  control.** During a burnout the driven axle's brakes are released. `99407bbc` (15 everywhere) is a standstill
  hold deceleration.
- **Handbrake**: strength blends from `Brake_strength_at_low_speed` to `_at_high_speed` between `Low_speed_mph`
  and `High_speed_mph`, rear wheels only, combined with the service brake by max (not added). `35e18207` /
  `6ce0fda7` belong to the drift (above), not to grip.
- Unknown: about 20 tuning constants (clutch torque cap, idle governor, shift-phase fractions, hysteresis,
  thresholds).

## Suspension, wheels, aerodynamics

- **Wheels are rigid bodies** (24–27 kg cylinders on a strut joint with a spring-damper), not raycasts. The tyre
  load is the spring's force.
- **Spring**: k = W / `41543f85`, with W the static load of the chassis (wheels excluded) on that wheel, solved
  from the geometry. So `41543f85` is the **static sag** (0.08 m front / 0.07 m rear), and every car's suspension
  frequency is √(g / sag) / 2π (911: 1.76 / 1.88 Hz). `df57b7ad` (true everywhere) selects this; `dadfbfc8` is
  an absolute stiffness used otherwise.
- **Damper**: c = 2 × `Damping_Ratio` × √(k W / g) in one direction and `0c5f89b3` × c in the other (911: ×2.0
  front, ×1.6 rear; probably rebound). `0c5f89b3` is not a frequency.
- **Anti-roll bars** per axle, stiffness `BodyMovement.782f05c1` / `2bb0f6a0` (unit unknown), damping ratio
  `Front_` / `Rear_roll_damping` (1.2) against the body's roll inertia.
- **Drag** (`FUN_009cf850`): per body axis, F = −½ K C |v| v with C = `Side_drag_coefficient` (2),
  `Top_drag_coefficient` (2), `Front_drag_coefficient` (0.75); K unknown (≈ 1 fits the 911's power at its top
  speed). Above `f1538c51` mph (= the top speed) the front coefficient ramps to `b8786ac2` (1.5) at `4aadb133`
  (229 mph). `b1322654` (0.5) scales the front drag by a 0..1 state (slipstream, inferred).
- **Downforce** (`FUN_009cfaf0`): F = `Lift_coefficient` (−1) × `ca386558` (0.5) × |v_forward| |v| along the
  body's up axis at body z = `b821de16`, in every state (911: 3.6 kN at 300 km/h). `Down_force` and
  `Down_force_lift_coefficient` are not read by the car code.
- **Airborne**: a state machine (grounded, taking off, airborne, landing; `FUN_009d50b0`). While taking off or
  airborne, (`In_air_gravity_coefficient` − 1) × g is added to the chassis and the wheels. For the first 0.35 s
  (`123d2559`) the angular velocity is damped per axis (pitch `8b456140`, yaw `312bd67d`, roll by roll rate) and a
  nose-down pitch kick applies. Over long jumps the springs blend over 1.9 s to a stiff undamped in-air spring
  (`ddbd44c0` / `b80e5de3`).
- Unknown: the drag constant K, the anti-roll stiffness unit, which damper slot is rebound, the strut travel.

## Tyres (`FUN_009c75b0` contact solve, `FUN_009c5da0` force limits)

- **The tyre is a velocity constraint** inside the rigid-body contact solver, not a force law. Each solver
  iteration applies the impulse that cancels the contact's sideways sliding, capped at μy × N × scales; the
  longitudinal impulse is solved together with the wheel's spin (its inertia included), capped at μx × N × scales.
  The brake is an impulse budget applied in the same solve (`FUN_009bbff0`). The grip curves only set the caps,
  which is why the game needs no low-speed special case.
- Slip angle = atan(−v_lat / |v_long|) in degrees; slip ratio κ = (ωr − v)/v braking, (ωr − v)/ωr driving,
  clamped to ±10. Wheel spin is real per-wheel state.
- **Lateral curve** (`FUN_009bcdb0`): with n = `Gradient` × `Peak_Slip_Angle` / `Peak_Force_Coefficient` and
  x = min(|α| / peak angle, 1), μ = peak × (1 − (1−x)^n cos²(πx/2) − (1−x)^max(2,n) sin²(πx/2)); flat beyond. It
  equals a Hermite only at n = 3 (the data has 1.4–14). The fourth field (0.5) is never read.
- **Longitudinal curve** (`FUN_009bc720`): piecewise linear through (`Peak_Slip_Ratio`, `Peak_Force_Coefficient`)
  and (`Max_Slip_Ratio`, `Max_Slip_Force_Coefficient`), flat beyond.
- **Blending, no friction circle**: lateral pure → brake/traction slip curve by |κ| through
  `*_Lateral_Slip_Factors` (`89f75d1a` start, `c35e15f1` full, `23a3702c` exponent, `24269da0` max);
  longitudinal by slip angle in degrees through `*_Long_Slip_Factors` (`e0e7e515`, `bf1bc91d`, `1d0b87f6`,
  `d9d33254`). Brake curves when κ < 0.
- **Caps** = μ × N × axle `*_lateral_scale` / `*_longitudinal_scale` × per-wheel surface factors × per-wheel grip
  multipliers (separate for the two directions; writer not found) × a camber factor (1 at `e84632d1`° falling to
  `8dd08eeb` at `d699cd4c`°); the driven wheels' longitudinal cap is reduced by the burnout factor. N is the
  suspension load plus half an axle term (downforce, inferred).
- Never read: `Available_grip`, `Grip_factor`, `Sliding_grip_factor`, `f5c703c9`, `9b0ab57e`. `AligningTorque` is
  null on all cars; the slip-angle relaxation (`7ab2a90a` = 1.8 m) is disabled (`c5cf33ea` = 0).
- Tyre forces do not roll the body: the torque about the car's long axis is removed (inferred from the flags).
- Unknown: the absolute scale (N and the grip multipliers; taken literally μ ≈ 3 would give ~3 g) and the
  "wheel nearly stopped" threshold. Measuring N and the multipliers in the running game would settle the
  model's grip scales.

## Engine sound

See the research log, "engine sound from the game code": the sound runs on its own 1000–8000 rpm axis
(audible = 1000 + 7000 × normalised rpm^(2 − a)), engine load is the pedal blended towards full by speed, and the
harmonic multipliers come from `EngineSound2`. The Distortion and SubHarmoniser plug-ins fall in undecompiled
gaps of the dump.
