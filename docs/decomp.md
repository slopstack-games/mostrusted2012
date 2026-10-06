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
  vectors filled by static initialisers; many initialisers are missing from the dump. Their live values were
  read from the running game with `mwtool memscan` (`dump NFS13.exe+0x<addr − 0x400000>`; research log,
  session 3): drag K 1.2, anti-roll scale 60, drift damping time bases 60, side-force cap 5, burnout 20 mph,
  clutch cap 5000 N m, shift phases 0.1 / 0.4, downshift time × 0.5, hysteresis 2 mph, idle governor 20 rpm /
  0.05 / 0.5, blip × 2.5, declutched below 0.1, steering full-rate threshold 0.99, slip-ratio stop 0.01.
- **The running car** can be read live: `handling.watch` (git-ignored) follows
  `[[[[NFS13.exe+0xcedb04]+4]+0x53220]+4+8·i]` to the player's vehicle object (i = `player_index`, 48 in free
  roam) and lists the fields mapped below.
- Upgrade / tuning overrides: `FUN_00574870(index, value)` applies the car's upgrade profile to a parameter
  (multiply, add or replace); with no upgrade the data value is used as is.
- The sound system registers named getters on `RaceCarEntity` (`FUN_0057e280`), which helped map the vehicle
  object (speed in mph at +0x1620, time in air +0x1630, ...).

## Update loop

- **Fixed 60 Hz tick** (confirmed: a 1/60 s step, with a 2/60 catch-up step when the game falls behind).
  Vehicles update once per tick, no sub-steps; the step is scaled by the clock's time scale (slow motion).
- **Controls** (`FUN_009aacc0` writes them; vehicle offsets): steering +0x1bf0 (positive = **right**), throttle
  +0x1bf4, brake +0x1c00, handbrake +0x1c40 (a mask: pulled above 0.1), nitrous +0x1c20 / mode +0x1c60,
  steering-wheel controller +0x1c64. The pad mapping is `max(0, (raw − offset) × scale)` per binding; steering =
  right − left. No smoothing or ramping of the input, and no central stability control, traction control or ABS.
- **Order in a tick** (`FUN_00904da0`): engine rev-limiter cut (`FUN_009d21b0`), then driveline and brakes
  (`FUN_008f7c50`: the driveline `FUN_009b32c0` with the engine `FUN_009d9960` and clutch `FUN_009d7ac0`, the
  burnout measure `FUN_009ab230`, the brakes `FUN_009b1180`), then `FUN_00904260`:
  1. `FUN_009b4d70`: speed in mph (vehicle +0x1620: the speed over the ground signed by the forward component),
     air / ground timers (`FUN_009ae840`), the per-wheel update (`FUN_009aaf30` → `FUN_009d8bd0`), the
     engine-sound values (`FUN_009b3030`), then an integration.
  2. `FUN_00900660`: gearbox mode switch.
  3. `FUN_009ae6f0`: drag, lift and the airborne state with the take-off behaviour (`FUN_009cf850`,
     `FUN_009d50b0`); skipped in two special states (+0x1774 / +0x1775).
  4. `FUN_009d27b0`: the handbrake state (below).
  5. `FUN_009d2aa0`: nitrous (`Engine` → `NitrousStrength`).
  6. `FUN_008f8360`: the low-speed turn-around assist (below).
  7. `FUN_009aae10` → `FUN_009d73f0`: steering. Its angle is used by the **next** tick's tyres.
  8. `FUN_009d91a0`: the drift.
  9. `FUN_008f7ab0`: a hard world hit (a contact value above 6, unit not settled) zeroes the front wheels'
     angle for the next tick.
  10. `FUN_009ae670`: the "every wheel down" flag (+0x1772, read by the next tick's drift), a final
      integration, the speed again; then the time since the last world contact (+0x3010) grows by dt.
- **Integration**: each stage adds forces and torques to the body's accumulators and `FUN_00907840` integrates
  them into the velocity before the next stage; the damping stages (`FUN_00907b50` angular, `FUN_0091dad0`
  linear) and the turn-around assist set velocities directly.
- **Body movement** has no per-tick stage: `BodyMovementBehaviour` is read when the vehicle is built
  (`FUN_008fda70`: roll damping, the in-air springs `ddbd44c0` / `b80e5de3`); its `Take_off_behaviour` is the
  airborne state's data. The type `BodyMovementBehaviour.Angle_control` exists but no car's data has one.
- Speed-banded lookups use the speed in mph signed by the direction of travel (the full speed, not the forward
  component).

## Low-speed turn-around assist (`FUN_008f8360`)

Helps a car that has driven into a wall turn away from it. A contact flag (+0x30ed, set by `FUN_0090dfa0` from
the world-contact table) resets a timer (+0x3010) that grows by dt each tick. The assist runs while that timer
is under 0.4 s or its strength is non-zero. It is **active** when throttle > 0.5, |steering| > 0.5, the body
touches the world facing it (dot(−n, forward) > 0 for the contact normal n), and the steering points away from
the wall (the normal's side component and the steering differ in sign) or the car is below 5 mph. Its strength
grows at 1/s while active, is held for 0.5 s after, then fades at 2/s, and restarts from 0 when the steering
changes side. With k = |steering| × strength and h = clamp((dot(−n, forward) − lo) / (0.8 − lo), 0, 1),
lo = 0.5 − 0.4 × clamp(mph / 5, 0, 1) (how head-on the hit is): the sideways velocity is pushed towards 0.5 m/s
in the steering direction (at most 30 × k m/s² × dt per tick), and the yaw rate towards (1 − 0.5 h) rad/s in the
steering direction (at most (20 − 10 h) × k rad/s² × dt). Constants: `.rdata` 0x01021ee0..0x010220ac.

## Steering (`FUN_009d73f0` → `FUN_009d31f0`; `FUN_009d39b0` for steering-wheel controllers)

- **Order inside the step** (`FUN_009d73f0`): the full-rate test reads the **raw** input (|input| ≥ `.bss`
  0x01186e90 = 0.99); in a drift the input is then remapped (`FUN_009d2e20`, below); a scripted hold mode
  (`FUN_009d30c0`, steering +0x71, set only by a scripted event in `FUN_00900120`) is not used in normal driving;
  then `FUN_009d31f0` (pads, keys) or `FUN_009d39b0` (wheels). Afterwards two hold timers track how long the
  input has been past ±0.01 (rates from a global tuning object; nothing in the handling reads them).
- **Range** (wheel angle at full input): off the throttle it blends from `Low_speed_steering_range` at 0 mph to
  `High_speed_steering_range` at `High_speed`. On the throttle a second curve runs from the low-speed range to
  `41d22d15` at `Medium_speed` and `98820dfe` at `High_speed` (911: 35° → 24° → 19°); the two curves blend by
  `clamp(throttle − brake)`. In reverse gear the range goes to `Reverse_steering_range` by `Reverse_speed`
  (negative mph). Each side's limit is min(range + k × |offset|, `Maximum_steering_angle`), k = `a40a6b17` on
  the side the offset points to and `16fd6022` on the other.
- **Neutral offset**: −q × `Maximum_steering_angle` × `553bc073` (0.3; 1 in reverse, 0 while drifting). The
  target is offset + input × (limit − offset). q is vehicle +0x2c70; no writer of it was found in the vehicle
  code (searched 0x008e–0x009e: only reads in `FUN_00904260` and `FUN_00902930`; the 0x006e writers of a
  +0x2c70 belong to another class). Taken as 0.
- **Rates** (degrees per second) blend from 0 mph to `High_speed`. Back towards the neutral position
  (sign(target − angle) = sign(offset − angle)): the counter-steer rate if the input points across the current
  angle, else the centring rate. Outwards: the steer rate with the raw input at full travel, else the raw
  `High_speed_steer_rate`. Reverse gear: the `Reverse_*` rates.
- **Counter-steer rate in a drift**: the rate blends towards `Counter_steer_rate_in_drift` by
  clamp((body slip − `Slip_angle_to_enter_drift`) / (`5277ec9e` − `Slip_angle_to_enter_drift`), 0, 1), only
  while the drift's counter-steer flag is set (drift +0x15: steering against the rotation, set by the side
  force). Body slip is the drift's 0..180° angle between heading and velocity (drift +0xe0).
- **Output**: the angle is stored as a fraction of `Maximum_steering_angle` (+0x1c80) and as radians with the
  sign flipped (+0x1c8c, the tyres' input); the right-hand limit is +0x1c88, which the drift uses as the range.
- **In a drift** (`FUN_009d2e20`) the input is remapped relative to the drift: no input holds `Default_steering`
  (0.24) of the range into the drift, full input full lock; the opposite input reaches a counter-steer limit
  lo = lerp(lerp(−1, `ac261613`, e), lerp(−1, `939b8fdc`, e), o), with
  e = clamp((peak rear slip − `Slip_angle_to_enter_drift`) / (`fb0656a3` − `Slip_angle_to_enter_drift`)) and
  o = clamp((|rear slip| − `Low_slip_angle_for_counter_steering`) / (`fe68bc4b` − `Low_slip...`)) raised to a
  power (`powf`; the exponent is a register the decompiler does not show. Inferred: `Max_counter_steering_power`,
  the only counter-steering field of `DriftBehaviour.Other` not otherwise read, at the one offset (+0x30) missing
  between the fields this function reads). The current wheel angle is also clamped to at most lo ×
  `Maximum_steering_angle` across. Steering-wheel controllers use `SteeringWheelPhysics.942c18c0` instead of
  `ac261613`.
- `Steering_angle_curve` is a type that `SteeringBehaviour` does not contain (a leftover).
  `SteeringWheelPhysics` (one shared instance) only applies to steering-wheel controllers.

## Drift (`DriftBehaviour`; `FUN_009d91a0` per tick)

The drift object is vehicle +0x2ab0; its parameters are cached in a block (drift +0x1c) by `FUN_009d12b0`
(entry angles), `FUN_009d1350` (side force) and `FUN_009d1410` (`Side_force_multiplier`, `55930ac1`,
`de67c45d`), each through the upgrade function `FUN_00574870`. Sign: the game's drift scale S has the sign of
the steering **into** the drift (steering is positive to the right), i.e. S = −sign(mean rear slip) at entry.

- **Per tick** (`FUN_009d91a0`): the mean rear slip (the rear wheels' slip, vehicle +0xd1c / +0xeec) and its
  peak; entry and exit (`FUN_009d83e0`); the damping (`FUN_009d11d0`, while drifting and for
  `Time_to_blend_damping` after); then, while drifting, `FUN_009d8770`: speed hold, drift scale, yaw torque, and
  the side force only when every wheel is down; outside a drift the time since the exit grows and the peak slip
  resets.
- **Entry** (`FUN_009d5690`): not drifting, every wheel down, faster than `Minimum_speed_to_enter_drift` (mph),
  the body not touching the world (+0x30ed), the handbrake off, and the burnout measure below 0.1. The mean rear
  slip must exceed lerp(threshold, `8f151151`, brake). The threshold is `Slip_angle_to_enter_drift`; a **brake
  tap** (brake above 0.25 at some point, released below 0.1 within 0.6 s; `.rdata` 0x010261d0 / 0x010261e0 /
  0x01026140) drops it to `0dce38f7`, from where it climbs back at `b5290695` °/s. The drift starts with
  S = −sign(rear slip) × (`e6abd30f`, or the `HandbrakeAbility` scale lerp(`6ce0fda7`, `35e18207`) by
  `Low_speed_mph`..`High_speed_mph` if the handbrake was on in the previous tick, i.e. it was just let go)
  (`FUN_009cd530`, `FUN_009d29f0`); the target speed is the current speed.
- **Exit** (`FUN_009d83e0`, codes at drift +0x10): forced by a collision (1, 3: contact values above 6), more than
  1 s airborne or no wheel contact (5), speed under 0.5 m/s (and a second speed under 0.5) (7), reverse gear (8),
  **the handbrake** (9), a special state +0x1774 (11); otherwise after 0.5 s when the mean rear slip falls below
  `Drift_angle_to_exit_drift` (6, `FUN_009d1120`). On every exit S = 0, the threshold resets, and if the wheels
  are counter-steering their angle is multiplied by `Drift_exit.ca545e03` (1 on 191 cars, 0.8 / 0.7 on six).
- **Drift scale** S (`FUN_009d5a30`), integrated per second: `ce8870ad` × throttle × clamp(|body slip| /
  `c1bc8c79`) × sign(S); (`_from_counter_steering` if counter-steering else `_from_steering`) × steering ×
  throttle (steering flipped when reversing; counter = steering × S ≤ 0); `8a2a92f3` × the throttle's rise per
  second; ± `Drift_scale_from_braking` × brake (negative when counter-steering); the handbrake scale while the
  handbrake is on; − `Drift_scale_decay` × sign(S). It never changes sign (it stops at ±ε) and is clamped to ±1.
  `Initial_yaw_torque` × brake adds a torque against the drift (0 on every car).
- **Yaw torque** (`FUN_009d0860`) about the body's up axis, into the drift: a quadratic Bézier in |S| through
  `Minimum_`, `Mid_` and a top torque; the top is `Donut_yaw_torque` below `Donut_speed_limit_low` mph,
  blending to `Maximum_yaw_torque` by `Donut_speed_limit_high` (always the maximum when counter-steering). So
  **donuts are the drift torque at low speed**, not a separate state. Times a steering factor: lerp(`a5c9496f`,
  1, |steering|) into the drift, lerp(`a5c9496f`, `70528afa`, |steering|) counter-steering, plus `2ef426dd` ×
  throttle; while counter-steering also × clamp((|rear slip| − `c901c1e8`) / (`e40fa397` − `c901c1e8`)) (both 0
  on every car: 1). Times a per-wheel surface factor (`FUN_009cede0`, 1 assumed), `VehicleInfo.40634c09.y` (1 on
  every car) and the upgrade function (identity without an upgrade).
- **Side force** (`FUN_009d0fb0` → `FUN_009d0df0`, `FUN_009ce240`), only with every wheel down: mass ×
  `09832a93` × f along the body's side axis with the **sign of S, i.e. outwards** (opposite to the yaw torque's
  turn; the push also has a forward component along the travel), projected off the ground normal. f = (ramp in
  body slip from `Drift_angle_for_side_force` to `Minimum_drift_angle_for_side_force` − min(`Decay_rate` ×
  (slip − `Drift_angle_for_decay`)+ while steering into the drift, 1 − `c3db1a2a` / `09832a93`))+ ×
  min(|S| / `Drift_scale_for_maximum_side_force`, 1) × a speed ramp from `d79e7ebc` to
  `Speed_for_maximum_side_force` mph × surface factor × (1 + boost), **capped at 5** (`.bss` 0x01187380, live):
  up to 5 × 14 = 70 m/s². The boost (`FUN_009cd460`) grows to clamp(0.5 × |yaw rate|) × |S| ×
  `Side_force_multiplier` while counter-steering against the rotation (which also sets the steering's
  counter-steer flag) and decays at 2 per second. The body slip used here is the 0..180° angle between heading
  and ground velocity (`FUN_009ce110`).
- **Damping** (`FUN_009d11d0`): the yaw rate (in body axes) is multiplied by `(1 − b × d × f)^(dt × 60)`, d = the
  drift's peak |S| × `Drift_angular_damping`, b = 1 − time since exit / `Time_to_blend_damping`; the body's
  sideways velocity by `(1 − ff5dcf85 × f)^(dt × 60)`. The 60s are `.bss` 0x01184cc0 and 0x01185640 (read live;
  `FUN_00907b50`, `FUN_0091dad0`). Applied before the drift's forces, and on the exit tick too.
- **Speed hold** (`FUN_009d0420`): the target is the speed at entry; it follows the speed whenever the car is
  faster, not on all four wheels, the throttle under 0.3 or the brake over 0.1, and runs at most `35e937b8` mph
  ahead. Force = mass × shortfall × throttle × (`55930ac1` along the ground-relative travel direction (its
  sideways and backward parts clipped by two axes not resolved) + `de67c45d` along the body's forward axis, 0 on
  every car) × the vector at `.bss` 0x01187250 (no initialiser in the dump; the same constant turns per-tick rates
  into per-second ones in `FUN_009d9960` and `FUN_009cfce0`, so 60 by that reading, not read live).
- Not read anywhere in the drift code: `Maximum_drift_angle`, `Steering_angle_multiplier`, `Torque_decay`,
  `Counter_steer_yaw_torque` (cached only). Nothing reads `DonutAbility` either.
- Unknown: the constant at 0x01187250, the counter-steer exponent's source, the collision exits' units, the
  per-wheel surface factor (1 assumed).

## Handbrake and burnout (`FUN_009d27b0`, `FUN_009b1180`, `FUN_009cdfd0`, `FUN_009ab230`)

- **State** (`FUN_009d27b0`, handbrake object vehicle +0x2a20): a countdown reset to `Minimum_handbrake_time`
  on every tick the handbrake is pulled and reduced by dt otherwise; the handbrake is **on while the countdown is
  ≥ 0**, so a pull lasts that long after it is let go. +0x2a40 is this tick's state, +0x2a41 the previous tick's.
  The vehicle's virtual +0x44 returns it (below). The same function runs a small state machine
  on the speed's sign and the yaw rate (pulled moving forwards / standing; held, released and still turning;
  rolling backwards for a timer from `.bss` 0x011870f0, yaw threshold 0x01187070); nothing in the handling path
  was found to read its outputs.
- **Brakes** (`FUN_009b1180` → `FUN_009cdfd0` per wheel): rear wheels only, the torque budget is
  max(handbrake strength, service torque × pedal²); strength = lerp(`Brake_strength_at_low_speed`,
  `_at_high_speed`) by the signed mph from `Low_speed_mph` to `High_speed_mph`. The brakes see the handbrake
  state of the previous tick (they run before `FUN_009d27b0`). **The handbrake does not lock the wheels**: the
  lock mask (a stopping torque that holds a wheel still) comes from other vehicle states (+0x1774 on all wheels,
  +0x1b90 on the front), not from the handbrake. Whether the rear wheels lock is up to the tyre and the torque.
- **Driveline** (`FUN_009b32c0` stores the state at +0x17b0; `FUN_009d5520`, `FUN_009d0380`, clutch
  `FUN_009d7ac0`): while the handbrake is on the front axle's torque share is 1 and the rear's 0, on every car
  (rear-driven ones included); the burnout measure then blends both shares towards the burnout axle's (front if
  `Power_To_Front` = 1, else rear). So with the handbrake on the engine stays coupled through the front wheels;
  the drivetrain agent's reading of `ref/xkr_launch` (parked, rear wheels locked, ~109 rpm per tick with the
  clutch closed, ~150–190 N m of load) fits that.
- **Which function**: the state is read through the vehicle's virtual +0x44 (vtable 0x01023758 written by
  `FUN_00920720`; entry +0x44 = 0x00920810, a stub the dump did not decompile). That it is the handbrake is
  inferred from its three uses: the rear brakes' handbrake mask, no drift entry, drift exit code 9.
- **Drift**: no drift starts while the handbrake is on, a drift ends when it comes on (exit code 9), and a drift
  that starts on the tick after it is let go begins with the handbrake's drift scale (above).
- **Burnout** (`FUN_009ab230`, copied to +0x17c0 by `FUN_009b32c0`): clamp((1 − |mph| / 20) × throttle ×
  brake, 0, 1), 20 mph from `.bss` 0x011866f0 (read live). It releases one axle's brakes by (1 − burnout): the
  front if `Power_To_Front` = 1, else the rear (so an AWD car keeps its front brakes); it blocks drift entry at
  0.1 or more; and the tyre code reduces the driven wheels' longitudinal cap by it.

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
  control.** During a burnout one axle's brakes are released (see "Handbrake and burnout"). `99407bbc` (15
  everywhere) is a standstill hold deceleration.
- **Handbrake**: see "Handbrake and burnout" above. `35e18207` / `6ce0fda7` belong to the drift, not to grip.
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
