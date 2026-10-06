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

- **Where the drivetrain sits** (2026-10-06): per tick the gearbox runs first (`FUN_009b3030` →
  `FUN_009da600` → the state machine `FUN_009da340`), then `FUN_008f7c50` → `FUN_009b32c0` runs the engine
  (`FUN_009d9960`) and then the clutch (`FUN_009d7ac0`), then the brakes and the tyre solve. Vehicle object
  layout: engine at +0x1cf0 (rpm +0x1d10, lagged throttle +0x1d14), driveline / gear index at +0x1d40 (+0x1d70),
  clutch engagement at +0x1e20, gearbox state machine at +0x1a70 (state +0x1bd8, its throttle at +0x1aa0). The
  engine data pointer reads the `Engine` GeneSys struct: `Power_curve` fields from +0x18, `Engine_inertia`
  +0xf4, `Engine_resistance` +0xf8, `c6deb7a6` +0xfc, `36676b9c` +0x100, `232d4b68` +0x108 (a limiter cut
  time: while the engine's limiter flag (+0x44) is up the throttle is scaled by the time since the cut over
  this; 0 on the XKR).
- **Engine** (`FUN_009d9960`): the engine is its own rotating inertia, coupled to the wheels only through the
  clutch. Per tick, with rpm the value at the start of the tick:
  1. idle governor g = clamp((`Idle_RPM` + 20 − rpm) × 0.05, 0, 1); throttle u = max(gearbox throttle, 0.5 g)
     (0x01186f30, 0x011871c0, 0x011873f0).
  2. closed-throttle torque X: the free engine's `Engine_resistance` × inertia while an upshift is under way
     (states 6–8) or the clutch is below 0.1 and no downshift is under way (0x01187570); otherwise the friction
     curve (`FUN_009d6ec0`: `b3066e01` at `Idle_RPM`, `5a6aaf9f` at `f71be7c6`, `a53d9b5d` at `Max_RPM`,
     piecewise linear). B = X × (1 − g): the governor fades the drag out.
  3. throttle peak-hold and lag: hold = max(hold − `c6deb7a6` × K × dt, u); lag += clamp(`36676b9c` × K × dt,
     0, 1) × (hold − lag), with K from `.bss` 0x01187250 (not read; 60 reproduces the original's idle, below).
     During an upshift both are set to u (no lag). The lagged value (engine +0x24, scaled by the limiter cut while
     its flag is up, clamped 0..1) is the effective throttle T.
  4. torque = B + T × (full-load torque − B); during a downshift (states 10–12) instead −`Engine_resistance` ×
     inertia × 2.5 (0x01187518) for the whole shift, whatever the rpm.
  5. the rpm is clamped at `Max_RPM`, then a **torque noise** is added: n(t × rpm / 120) × N(rpm) × (`455b2e4a`
     + (`dc7b0a4b` − `455b2e4a`) × load). t is a clock (engine +0x30) that wraps after rpm / 120 s; n is
     `FUN_009e3780`, value noise on an integer lattice (the classic 15731 / 789221 / 1376312589 hash, divided by
     2³⁰) with cosine interpolation; N is a second three-point curve (`FUN_009d6f90`: `15d1d117` at idle,
     `d857ea54` at `f71be7c6`, `ddfc5326` at `Max_RPM`; XKR 0 / 2 / 10 N m); load is the sound's engine load
     (vehicle +0x1f20, `FUN_009b1f70`). So the noise reaches the rpm, not only the sound; it is zero at idle on
     the XKR.
  6. rpm += torque × dt / inertia × 60/2π (0x01187310).
  `FUN_009262a0` (max of the lagged throttle with a list of override values held by an object not identified)
  sits between steps 3 and 4.
- **The original's idle** (`ref/xkr_launch`, XKR, clutch open): 1002–1033 rpm, mean ~1015, alternating tick to
  tick. That is the governor's limit cycle: the term B reacts to the rpm within the tick (gain ≈ −3 per tick)
  while the throttle reacts through the lag. Steps 1–6 with K = 60 give 1003–1031, mean 1016, the same
  alternating pattern and an effective throttle of 0.25 (the game's sound load reads 0.25); K = 1 or an
  instant lag do not (987–1029 / 968–1091).
- **Clutch** (`FUN_009d7ac0`): one impulse per tick, before the tyres: with Δω the engine speed minus the
  driveline's (`FUN_009d55f0`: the axles' mean wheel spin × share × gear ratio × final drive),
  T = min(5000 N m (0x01186f70), clutch × |Δω| × I_eff / dt × k) along Δω (k: the `.bss` vector 0x011873e0,
  one lane per entry, no writer in the dump; taken as 1, to be read live), where 1/I_eff = 1/`Engine_inertia` +
  ½ ratio² × the driven wheels' mean inverse inertia × share² (Newton-refined reciprocal). The engine loses
  T dt / I, the driveline gets T × ratio. **No efficiency factor** anywhere. Each driven wheel gets half of its
  axle's torque; `TransmissionInertia` is added to each driven wheel's inertia. A second half-step pass
  (`FUN_009050b0`, dt / 2 and 2 / dt) runs after the tyre solve when a world flag (+0x531b9 of the caller's
  object) is set; not modelled.
  Because the tyres then hold the wheel at road speed, the clutch never locks under load: it slips with
  T ≈ I_eff × Δω / dt. In the original's launch the engine runs far above the wheels: ×1.9 in 1st, ×1.3 in
  2nd, ×1.14 in 3rd at full throttle (rpm against w2_omega × ratio × final drive), with the tyres at κ ≈ 0
  (`ref/xkr_launch`).
- **Gearbox state machine** (`FUN_009da340`, state at gearbox +0x168; every state change is forced to 0 by
  byte +0x173): 0/1 start-up (`FUN_009ce3d0`, `FUN_009cd7a0`), 2 standstill (`FUN_009d6370`: clutch open,
  reverse selection, burnout), 3 launch (`FUN_009d17d0`), 4 close the clutch (`FUN_009cd820`), 5 in gear
  (`FUN_009d9ee0`), 6/7/8 upshift declutch / neutral / clutch-in (`FUN_009d6880`, `FUN_009d95a0`,
  `FUN_009d6af0`), 9 shift request (`FUN_009d88e0`), 10/11/12 downshift declutch / neutral / clutch-in
  (`FUN_009ce490`, `FUN_009d69e0`, `FUN_009d6ca0`), 13 after a downshift (`FUN_009da1d0`). Before each state
  function, `FUN_009d6170` copies the pedal to the gearbox throttle (the brake in reverse). Gear index 0 is
  reverse, 1 first; the original sits in index 0 with the clutch open when parked.
- **Launch** (state 3): with no throttle the clutch opens and the state returns to standstill; otherwise
  engagement = max(throttle, the clutch's previous value, clamp(rpm / the gear's `Gear_up_RPM` ± 50 random)) —
  the ± 50 (`FUN_009dc4e0`, `.data` 0x010cfa68) is drawn per gear and is only used here — and at ≥ 0.95
  (0x01187080) with rpm > `Idle_RPM` the clutch closes fully (state 4 → 5). Below `Idle_RPM` × 0x011874a0
  (not read) it stalls back to standstill.
- **In gear** (state 5): the clutch opens (standstill, gear 1) when rpm < `Idle_RPM`, or when neither pedal is
  pressed below 20 mph (0x011872d0, inferred: its lanes 2–3 read 20). Upshift when rpm > `Gear_up_RPM` of the
  gear (no random offset) **and** (mph > `Gear_up_speed` **or** nitrous), while the gear is below the top
  usable one; the target is the highest gear whose lower neighbour's `Gear_up_speed` is passed (`FUN_009d4c80`).
  Otherwise `FUN_009d93b0` downshifts when (mph < the lower gear's `Gear_up_speed` − 2 (0x011872e0) or rpm <
  the gear's `Gear_down_RPM`) and vehicle +0x1ed0 > 0x01187580 (neither identified; braking from 63 mph the
  original downshifts 3 → 2 → 1 with no throttle, so it is not the throttle) and no nitrous; the target is the
  highest lower gear whose `Gear_down_RPM` is below the rpm it would have (`FUN_009d8d60`), none → no shift.
- **Shifts**: time T = (`Gear_change_time` (the `CruisingGearbox` one for a kick-down, `FUN_009dc3c0`)) ×
  (1 + 0.15 × random) (`.data` 0x010cfa64); downshifts × 0.5 (0x01186fa0). Phases, each counted down per tick
  and acted on in the tick it ends: **declutch** 0.1 T (0x01186ee0; clutch = time left / phase, 1 → 0),
  **neutral** (1 − 0.1 − 0.4) T = 0.5 T (clutch 0; the gear changes at its end, an upshift's target chosen
  again for the speed then), **clutch-in** 0.4 T (0x01186fd0; clutch 0 → 1). Upshifts cut the throttle from
  the first declutch tick (shift style 1, the only one the code sets) and bring it back during clutch-in as
  pedal × progress^0x011874d0 (not read; `FUN_00ed9e1b` is `pow`); downshifts keep the pedal in declutch, cut it
  in neutral and bring it back the same way. The original's 1→2 in `ref/xkr_launch` shows exactly this: one
  declutch tick, five neutral ticks, gear 2 on the fifth, then 0.25 / 0.5 / 0.75 / 1 (T ≈ 0.167 s for the XKR's
  0.15 s). **Top-gear lock**: flags `13bfea5a` / `7b20506a` lock the last gear unless a profile option ("long
  gears", inferred) or nitrous is active (`FUN_009d4c20`), so the 911's race box has 6 usable gears, not 7.
- **Driver inputs** (`FUN_009aacc0`, called by `FUN_008fa750` for the player): vehicle +0x1bf0 is the gearbox's
  control block: +0x1bf4 throttle, +0x1c00 brake, +0x1c10 a hold override (0 from the player), +0x1c30 a
  forced-brake mask (cleared on every write; when set, `FUN_009d6170` makes brake 1 and throttle 0), +0x1c40 a
  mask the player sets from handbrake input (+0x1fc0) > 0.1 (`.rdata` 0x01021cd8).
- **Standstill hold** (2026-10-06, session 5): `FUN_009d6170` writes the hold input at gearbox +0x38 (vehicle
  +0x1aa8): when the signed forward speed (×`.bss` 0x0118a060, the scale of vehicle +0x1620) is below `.bss`
  0x01186e60 (no absolute value), input = brake pedal (1 with the handbrake mask), raised to the throttle when
  the gear index is not 0; otherwise 0; then max'ed with the override +0x1c10. With the handbrake mask and
  |speed| < 1 it also releases the pedal brake. `FUN_009b1180` reads it: the rigid body (vehicle +0x1760) gets
  Δv = −sign(+0x1620) × `99407bbc` × input × dt along its forward axis (body +0x20) added to its velocity
  (+0xb0) every tick, with no clamp at zero (`FUN_009073b0` stores it, or zero above the body's speed cap
  +0xbc). The lock mask +0x1b90 is OR'ed into the per-wheel "brake engaged" mask passed to `FUN_009cdfd0`.
- **Downshift gate** (2026-10-06): vehicle +0x1ed0 is the **time on the ground**: `FUN_009ae840` adds dt
  while any wheel has contact and resets it while all four are in the air above `.bss` 0x01186630 (air time
  +0x1630 counts then). So `FUN_009d93b0` does not downshift until the car has been down for 0x01187580 s.
  It reaches the gearbox through `FUN_009b3030` → `FUN_009da600` (argument 5), with +0x1620 (forward speed)
  as argument 3 and the brake ramp +0x1610 as argument 6.
- **Launch delay** (2026-10-06, not resolved): in `ref/xkr_launch` the rpm stays at idle and the brake ramp at 1
  for 0.45 s after the throttle, then the engine revs one tick before gear index 0 → 1: the gearbox throttle is
  zero during the wait, which is what the forced-brake mask +0x1c30 does (`FUN_009d6170`). Its setter is not
  found (only `FUN_009aacc0` clears it). In standstill (`FUN_009d6370`) the gear index goes 0 → 1 once the
  throttle passes `.rdata` 0x01026130, and state 3 (launch) needs the handbrake mask clear and brake ≥
  0x01186ec0 or throttle ≥ 0x01187230; its timers +0x60/+0x80/+0x90/+0xf0/+0x100 gate reverse selection, not
  the launch. The engine-load ramp 0.25 → 0.62 → 0.81 → 0.91 → 0.95 in the reference is a lag of 0.5 per tick.
- Not resolved in the drivetrain: K (0x01187250), the clutch-in throttle exponent (0x011874d0), the stall
  fraction (0x011874a0), the hold speed (0x01186e60), the downshift ground time (0x01187580), the writer of the
  forced-brake mask +0x1c30, and the objects behind `FUN_009262a0`'s override list.
- **Differentials** (`FUN_009d8140`): `Front_` / `Rear_Differential` = c limit the axle's wheel-speed ratio to
  1/c (0 open, 1 locked; 911 rear 0.75). `Center_Differential` is 0 on every car.
- **Top speed**: `Top_speed_MPH` is never read by the physics. Top speed follows from torque, drag (with its
  over-speed ramp, below) and the rpm clamp in the highest unlocked gear.
- **Brakes** (`FUN_009b1180`): torque per wheel = `6ade4bb6` (front) / `66251131` (rear) × pedal² × a ramp that
  rises at 10 × pedal per second. A brake can stop a wheel but never reverse it. **No ABS, no traction
  control.** During a burnout one axle's brakes are released (see "Handbrake and burnout"). `99407bbc` (15
  everywhere) is the standstill hold deceleration (above).
- **Handbrake**: see "Handbrake and burnout" above. `35e18207` / `6ce0fda7` belong to the drift, not to grip.
- The drivetrain constants are now accounted for (above); the ones not yet read are listed at the end of the
  gearbox entries.

## Suspension, wheels, aerodynamics

- **Wheels**: the definition has 24–27 kg cylinder bodies on a strut with a spring-damper; in the solver each
  wheel is a compact record (spin, spin inertia) tied to the chassis by a wheel constraint holding its ground
  contact points, whose normal impulses are iterated in sub-steps (`FUN_0093f200` → `FUN_009c2670`,
  `FUN_00932190` → `FUN_009c5a90`); the tyre friction comes after, once per tick ("Tyres"). The tyre's grip load
  is **not** the spring's force but the static corner load plus a downforce share ("Tyres").
- **Spring**: k = W / `41543f85`, with W the static load of the chassis (wheels excluded) on that wheel, solved
  from the geometry. So `41543f85` is the **static sag** (0.08 m front / 0.07 m rear), and every car's suspension
  frequency is √(g / sag) / 2π (911: 1.76 / 1.88 Hz). `df57b7ad` (true everywhere) selects this; `dadfbfc8` is
  an absolute stiffness used otherwise.
- **Damper**: c = 2 × `Damping_Ratio` × √(k W / g) in one direction and `0c5f89b3` × c in the other (911: ×2.0
  front, ×1.6 rear; probably rebound). `0c5f89b3` is not a frequency. The setup (`FUN_009b29b0`, through
  `FUN_00933930` / `FUN_00925620`) stores rest length, k, `0c5f89b3` × c, W and c at spring +0x10, +0x14, +0x18,
  +0x1c, +0x2c (the in-air blend in `FUN_008fe200` rewrites +0x14, +0x18 and +0x2c); a per-axle factor (an
  upgrade input, 1 by default) multiplies k. The function that applies the two dampers was not found (the island
  solver files hold no spring solve that reads +0x18 and +0x2c as plain floats; it uses vector loads), so which
  slot is rebound is still open.
- **Anti-roll bars** per axle, stiffness `BodyMovement.782f05c1` / `2bb0f6a0` (unit unknown), damping ratio
  `Front_` / `Rear_roll_damping` (1.2) against the body's roll inertia.
- **Drag** (`FUN_009cf850`): per body axis, F = −½ K C |v| v with C = `Side_drag_coefficient` (2),
  `Top_drag_coefficient` (2), `Front_drag_coefficient` (0.75); K = 1.2 (`.bss` 0x011872a0, read live). Above `f1538c51` mph (= the top speed) the front coefficient ramps to `b8786ac2` (1.5) at `4aadb133`
  (229 mph). `b1322654` (0.5) scales the front drag by a 0..1 state (slipstream, inferred).
- **Downforce** (`FUN_009cfaf0`): F = `Lift_coefficient` (−1) × `ca386558` (0.5) × |v_forward| |v| along the
  body's up axis at body (0, 0, `b821de16`) (the origin while airborne), in every state (911: 3.6 kN at
  300 km/h). The tyres feel the full −`Lift_coefficient` × |v_forward| |v| (without `ca386558`), split between
  the axles ("Tyres"). `Down_force` and `Down_force_lift_coefficient` are not read by the car code.
- **Rolling resistance exists** and comes from the surface: each wheel in contact is pushed back with the
  surface's `5c2a9f18` × its grip load (0.01 on asphalt), plus a slip-angle dependent slide drag
  (`FUN_009d4a10`; "Tyres").
- **Airborne**: a state machine (grounded, taking off, airborne, landing; `FUN_009d50b0`). While taking off or
  airborne, (`In_air_gravity_coefficient` − 1) × g is added to the chassis and the wheels. For the first 0.35 s
  (`123d2559`) the angular velocity is damped per axis (pitch `8b456140`, yaw `312bd67d`, roll by roll rate) and a
  nose-down pitch kick applies. Over long jumps the springs blend over 1.9 s to a stiff undamped in-air spring
  (`ddbd44c0` / `b80e5de3`).
- **Anti-roll scale** 60 (`.bss` 0x01185d70, read live) on the data values; the unit (N m per degree) is
  inferred.
- **Strut travel**: searched for the joint built from `WheelSuspensionConstraint` (`Angle` 5, `7fc0dc41` 0.09 /
  0.08, `447b4e1c` / `668c92ac` −1.5, `c2a87807` 1.5, `b867a203` 0.23, `b1c680d5`): no reader of these offsets
  turned up in the vehicle build (`FUN_009b4e60` and its callees) or in `009A`–`009D` (searched `+ 0x14` / `+ 0x24`
  reads off a resource); `7fc0dc41` (0.09 front, 0.08 rear on the 911) is the likeliest travel. Open.
- Unknown: the anti-roll stiffness unit, which damper slot is rebound, the strut travel.

## Tyres (`FUN_009c75b0` per-wheel solve, `FUN_009c5da0` caps, `FUN_00905910` per-tick inputs)

Traced 2026-10-06 to the end of every factor; values checked against the reference run `ref/xkr_launch`, whose
watch file logs the wheel's grip load (`w*_N`, wheel +0x100) and static load (`w*_load_in`, +0x104).

- **Where the tyre sits in the solver.** The physics definition has a 25 kg cylinder body per wheel, but in the
  solver each wheel is a compact record (0x80 bytes: orientation rows, spin at +0x2c, spin inertia +0x54 and its
  inverse +0x5c; the vehicle's wheel block points at it from +0x198) tied to the chassis (a full 0x100-byte body
  record) by a 0x500-byte wheel constraint (built by `FUN_009c24b0`) that holds the wheel's ground contact
  points and the tyre. An island step (`FUN_0093f5a0`) first iterates the joints and the contacts' normal
  impulses in sub-steps (`FUN_0093f200` → `FUN_009c2670`; `FUN_00932190` → `FUN_009c5a90`), then solves the
  tyre friction **once per tick**, wheel after wheel (`FUN_009289f0` → `FUN_009c75b0`):
  1. the brake (`FUN_009bbff0`): an angular impulse budget (the brake torque left over from `FUN_009b1180`)
     that slows the wheel's spin towards zero and never past it;
  2. the contact frame (`FUN_009c2aa0`, `FUN_009ca610`); skipped while the contact points carry no normal
     impulse;
  3. the tyre pass `FUN_009c6f00`: it clears the solve state (constraint +0x100…), writes the caps into it
     (`FUN_009c5da0`; κ is taken there from the spin × radius as it stands after the clutch and the brake
     step), then `FUN_009bc340`, which only copies that state with two contact anchor points into +0x220 (no
     force, no spin change), then the longitudinal constraint `FUN_009bc0c0`, then the lateral clamp in
     `FUN_009c6f00` itself. `FUN_009bc0c0` (read 2026-10-06), with s the slip speed along the tyre, m the
     chassis's effective mass at the contact along it (`FUN_009e8230`), I the wheel's spin inertia, r its
     radius and C the longitudinal force cap:
     - first a **brake hold**: λ1 = min(B / r, |s| m, C), B being the brake budget `FUN_009bbff0` left over
       (tyre object +0x30 = constraint +0x2a0; unbounded, FLT_MAX `DAT_01025900`, when the lock flag +0x2a8 is
       1). It acts on the chassis only; the wheel's spin is not touched;
     - then the **coupled part**: λ2 = min((|s| − λ1 / m) / (1/m + r²/I), C − λ1);
     - force on the chassis −sign(s) (λ1 + λ2); torque on the wheel's spin λ2 × r only. No slip curve inside:
       within the cap the tread is held at the road speed in one pass.
     The drive torque is not an input of the tyre pass: the clutch (`FUN_009d7ac0`) has already put it into the
     driveline's per-axle torque before the solver runs;
  4. the impulses go into the chassis (force at the contact point, its component of torque about the chassis's
     long axis removed unless flags +0x251..0x253 are set) and the wheel's spin (`FUN_009c2910`).
  The solver works in forces (its vectors hold dt and 1/dt; the brake and the relaxation term only balance that
  way). The lateral force asked for is the chassis's effective mass at the contact along the side axis
  (`FUN_009ca610` via `FUN_009e8230`) × v_lat / dt, capped; the longitudinal one couples that mass with the
  wheel's spin inertia. Because the wheels are solved one after the other, each sees the chassis velocity the
  previous ones left.
- **The caps** are written only while the contact is valid and its `+0x14` lane exceeds 0.5 (`DAT_00f8a810`;
  read as the ground normal's world-up component, inferred), otherwise the tyre has no grip:
  - lateral = μ_lat × N × camber × L,
  - longitudinal = μ_long × N × camber × G as a force, and the same × the wheel radius as the torque cap on
    the wheel's spin,
  - L = surface lateral grip × axle `Front_` / `Rear_lateral_scale` × min(grip multipliers 0, 2),
  - G = surface longitudinal grip × axle `*_longitudinal_scale` × min(grip multipliers 1, 3) × (1 − burnout)
    on driven wheels (`FUN_009ab230`: (1 − |mph| / 20) × throttle × brake, clamped).
  The axle scales are read once by the axle's tyre object (`FUN_009d4ce0`: front or rear block of `Tyres`, which
  also builds the SIMD curve tables in `FUN_009c2f50` / `FUN_009c3230`).
- **N is not the suspension force.** `FUN_00905910` sets N = the wheel's static load (wheel +0x104: the strut's
  static load, the chassis weight on that corner solved from the geometry, copied once in `FUN_009aef20`) + ½ ×
  the axle's downforce (lane 1 of the aero state's front / rear vectors, written by `FUN_009cfaf0`). It blends to
  max(strut force, 0) by Tyre `5f032abb`, which is 0 on all 197 cars. So **the grip caps do not follow load
  transfer** (pitch, roll, bumps). Confirmed on `ref/xkr_launch`: `w*_load_in` stays at 4576 / 4596 / 4071 /
  4051 N (= 1762 kg × g split by the centre of mass between the axles) through the launch and the braking, and
  `w*_N` = that + the downforce share.
- **Downforce seen by the tyres** (`FUN_009cfaf0`, called from the airborne state machine `FUN_009d50b0`):
  D = −`Lift_coefficient` × |v_forward| × |v| (0 when the car makes lift); the force on the body is `ca386558` × D
  at body (0, 0, `b821de16`) (at the origin while airborne). D is split between the axles by where the point
  z = lerp(`b821de16`, `d21c15d0`, brake pressure) sits between them (`35d32b7d` replaces `b821de16` under a
  state mask not identified). XKR run: D = 1.00 × v², front share 42.1 % on the throttle (= (−0.15 + 1.308) /
  2.749) and 47.6 % with the brake ramp at 1 (`d21c15d0` = 0), linear in the ramp in between.
- **Load ratio in the curve.** The lateral curve's exponent n is multiplied by static load / N
  (`FUN_009c5da0` passes it as `FUN_009bcdb0`'s fourth argument), so downforce raises the cap and softens the
  initial rise.
- **Slip inputs.** α = atan(−v_lat / |v_long|) in degrees; the relaxation (blend from the previous α by
  |v| dt / `7ab2a90a`, lag clamped to ±`c5cf33ea`) is off on every car (`c5cf33ea` = 0). κ (`FUN_009ba3e0`): the
  braking form (ωr − v) / v when |ωr| < 0.01 (`.bss` 0x01186af0, read live) or the rim is slower than the road,
  else (ωr − v) / ωr, clamped to ±10 (`DAT_010258c0`). The pure → slip blends take κ and α multiplied by a
  value at constraint +0x210 whose writer was not found (taken as 1; see "Unknown").
- **Surface factors** (`FUN_009d8bd0` per wheel, per tick): the collision tag's surface (bits 13–19) →
  `SurfaceType` → the contact kind's `SurfacePhysics` (normal; flats / rims after tyre damage) → the car's tyre
  class override (`RoadOverrides` by default; `FUN_0054d750`): `Surface_Grip`, `5c2a9f18` (rolling resistance),
  `7585f9cb` (slide-drag scale) and the roughness. The car's `VehicleSurfaceProfile` link for that
  `SurfacePhysics` (`FUN_009d4b70`) blends the grip (`FUN_009cec40`): lateral = lerp(`Surface_Grip`,
  link `f43804ae`, `fad9b7b5` front / `5567a3e0` rear), longitudinal the same with `f448cca5` / `42829f58`. When the
  surface changes, the old values fade over a time and the lower of old and new applies (`FUN_009ceb00`,
  `FUN_009d4460`, `FUN_009cece0`; not ported). Asphalt (`SurfacePhysics` 283001, the dark-grey road type):
  `Surface_Grip` 1 and link grip 1 on every car, so 1; the game's `Default_surface` (250315) is 1 too.
- **Per-wheel grip multipliers** (wheel +0x140, four lanes): lanes 0 / 1 belong to a timed gameplay effect
  (`FUN_00900120`: per-wheel mask +0x30f4, values blended over +0x2ed0…; reset to 1 when it ends), lanes 2 / 3 to
  a post-event timer (`FUN_009b2bb0`) that ramps × 0 back to 1. They are 1 in normal driving (read live: all 1).
- **Rolling resistance and slide drag** (`FUN_009d4a10`): each wheel in contact gets a force along its rolling
  direction against its motion, of rolling × N + |mph| × slide-drag scale × lerp(`Min_drag`, `232a1438`,
  clamp((|α| − `7a4cd931`) / (`810e99bc` − `7a4cd931`))), α being the wheel's (relaxed) slip angle (+0x10c).
  Asphalt: 0.01 × N and 1 × (0 → 4 N per mph over 0–15° of slip) for most cars (11 have no slide drag); the
  default surface: 0.01 × N, no slide drag. Grass, dirt and gravel have 0.025–0.16 and slide drag up to 15.
- **Curves**: lateral (`FUN_009bcdb0`): with n = `Gradient` × `Peak_Slip_Angle` / `Peak_Force_Coefficient` ×
  the load ratio and x = min(|α| / peak angle, 1), μ = peak × (1 − (1−x)^n cos²(πx/2) − (1−x)^max(2,n)
  sin²(πx/2)); flat beyond. Longitudinal (`FUN_009bc720`): piecewise linear through (`Peak_Slip_Ratio`,
  `Peak_Force_Coefficient`) and (`Max_Slip_Ratio`, `Max_Slip_Force_Coefficient`). Blends, no friction circle:
  lateral pure → brake / traction slip curve by |κ| through `*_Lateral_Slip_Factors` (`89f75d1a` start,
  `c35e15f1` full, `23a3702c` exponent, `24269da0` max); longitudinal by α in degrees through `*_Long_Slip_Factors`
  (`e0e7e515`, `bf1bc91d`, `1d0b87f6`, `d9d33254`). Brake curves when κ < 0. Camber factor: 1 at `e84632d1`°
  falling to `8dd08eeb` at `d699cd4c`°.
- **The resolved scale.** Nothing is missing: on asphalt the caps are the data μ × the static corner load (+ the
  downforce share), so the 911's lateral peak is ~2.9 g of grip, its braking peak ~1.8 g. The reference run
  agrees: the XKR braking with only the fronts biting (κ ≈ −0.2, rears ≈ 0) slows at 0.95–1.0 g = 1.74 × the
  front static load / the car's weight; sliding at 20–30° with locked fronts it slows at a steady 1.95 g. The old
  0.4 / 0.7 grip scales were calibrated to real-world cars, not to the game.
- Never read: `Available_grip`, `Grip_factor`, `Sliding_grip_factor`, `f5c703c9`, `9b0ab57e`. `AligningTorque` is
  null on all cars. Tyre forces do not roll the body: the torque about the car's long axis is removed (inferred
  from the flags).
- Unknown: the constraint +0x210 scale on the blend inputs (read it with the probe). Leads (2026-10-06): the
  constraint builder `FUN_009c24b0` zeroes +0x210..0x21c; `FUN_009a56b0` (called from `FUN_0095b100`) copies a
  4-float vector into +0x210..0x21c of an object and sets its byte +0x4aa, which fits inside the 0x500-byte wheel
  constraint but is not proven to be it. If nothing else writes it the blends see 0 (pure curves only), not 1.
  Also which
  state picks `35d32b7d` for the downforce point, and the meaning of the contact's +0x14 lane.

## Tyre skid sound intensities (`FUN_009d1bf0`)

The three bindings the surface's skid sequences follow (`LatSlipIntensityForSkidSound` → `Drift`,
`SpinIntensityForSkidSound` → `Spin`, `SpinBrakingIntensityForSkidSound` → `Brake`) are registered on
`RaceCarEntity` by `FUN_0057e280`; their getters (`LAB_005a2220`, `LAB_005a2200`, `LAB_0059ef60`) are not
decompiled. The values are computed in the handling code, once per 60 Hz tick, by `FUN_009d1bf0` (called from
`FUN_00904da0` → `FUN_009d6e30`, after the surface rumble `FUN_009d19c0`), which stores them at +0xd0 / +0xd4 /
+0xd8 of its per-vehicle object (reset to 1.0 by `FUN_009d1120`). The link from those fields to the getters is
inferred. Confirmed from the code:

- **One reference wheel**, no max or average over wheels: wheel 0 (vehicle +0x870, the first front wheel) when
  `Power_To_Front` (Engine +0xb0) is exactly 1, else wheel 2 (+0xc10, the first rear wheel).
- **Tuning**: a `TyreSoundParams` handle on a per-vehicle object (`*(this+0xe4)+0x50`), replaced by the wheel's
  surface's `SurfaceSounds.TyreParams` (+0x5c) when the wheel is on the ground and that handle is set
  (`FUN_0054a5d0` picks the `SurfaceSounds` from the surface id, wet flag and rim / flat state). The install has
  one `TyreSoundParams` (`…1b0ca7`); its `Lateral` and `Longitudinal` groups are never read.
- **Lateral** = clamp((|α| − 0.05) / (40 − 0.05)) × clamp(v / 10), with α the wheel's slip angle in degrees
  (wheel +0x114) and v the vehicle's speed in mph (+0x1620, absolute). Fields: `LatSlip.2f5a0697` = start,
  `166ed1d6` = full, `4b057923` = full-effect speed.
- **Spin** = clamp((s − 2) / (20 − 2)) with s = |tread speed − ground speed| of the wheel (wheel +0xe4 − +0xe0)
  times a runtime constant (`0x0118a060`, not in the dump; inferred mph). Either sign counts, so a locked wheel
  at speed spins this sequence too. Zeroed when the wheel's load (wheel +0x108) is under another runtime
  constant (`0x011870d0`). Fields: `LongSpin.9cc64e8b` = start, `e4127a7c` = full.
- **Braking** = clamp((b − 0.3) / (1 − 0.3)) × clamp(v / 40) × 0.5, with b at vehicle +0x1aa4 (read as the brake
  pedal: it sits next to the throttle at +0x1aa0), ignored while a vector at +0x1b80 is non-zero. When the
  vehicle's virtual method +0x44 returns true the value is clamp(v / 40) alone (not decompiled; the handbrake
  would fit). Fields: `LongSpinBraking.551d00de` = start, `3fa71b19` = full, `4b057923` = full-effect speed.
- **Smoothing**: per tick x += k (target − x), with k the rise rate (`7961414d`) when the target is above,
  else the fall rate (`07c5dd70`): lateral 0.5 / 0.1, spin 0.12 / 0.2, braking 0.05 / 0.5. Not scaled by the
  time step.
- `Curve_power` (1 everywhere in the data) feeds three `powf` calls whose arguments the decompiler dropped;
  read as an exponent on each ramp.

What this means for the sound: a typical slide of 10–20° is 0.25–0.5 on the drift sequence's axis, which plays
its low and middle layers; only slides past ~30° reach the top layers. Our earlier stand-in (full at 12°) put
every slide on the top layers.

## Engine sound

See the research log, "engine sound from the game code": the sound runs on its own 1000–8000 rpm axis
(audible = 1000 + 7000 × normalised rpm^(2 − a)), engine load is the pedal blended towards full by speed, and the
harmonic multipliers come from `EngineSound2`. The Distortion and SubHarmoniser plug-ins fall in undecompiled
gaps of the dump.

## Post-processing and colour grade (2026-10-06)

The final post-process shaders are not in `SHADERS.BNDL`: 74 DXBC programs sit in `NFS13.exe`'s `.rdata`
(not encrypted on disk; `mwtool shader-asm --exe list`, addresses are virtual addresses as in the dump). The
chain, from the light buffer (HDR game units) to the screen:

1. **Bloom and vignette** (exe programs, set up in `FUN_008e1580` / `FUN_008e1c30`): `0x0100f5f0` /
   `0x010104c0` downsample 4 taps × the exposure (`kHdrToneMapping.w`, or the 1×1 tonemap texture's w) / 128
   into RGBM (M = max channel rounded up to 1/255); Gaussian blurs; `0x010117c0` adds two blurred levels
   (`kColorAddWeight.z`, `.w`) and writes the **vignette weight in alpha**: v = smoothstep(saturate(|p| +
   `kVignetteOuterRgbPlusAdd.w`)), p = (rotate(2·uv − 1, `kVignetteAngle`) − (2·centre − 1)) × scale
   (`kVignetteCentreXyScaleXy`, VS `0x01011e18`).
2. **Tonemap** (`GBufferComposite` in `SHADERS.BNDL`): (hdr + bloom × B.rgb × saturate(bloom × T.w) × B.w) ×
   (A.rgb × v + D.rgb) × T.w → 1 − 2^(−x) → polynomial sRGB fit. T.w = the auto-exposure scale; with v the
   vignette weight, A·v + D is the vignette's inner → outer colour blend (`PostFxKeyframe.Vignette` holds an
   inner (1, 1, 1) and outer colour, (0.45, 0.45, 0.45) in the default keyframe `…78dc`). The constants are
   `GenericVector4_A` / `_B` / `_D`, global constant slots 0x9c / 0x9d / 0x9f written in `FUN_0089d5c0`
   (through `FUN_00a5d660(slot)`): **B** = (`PostFxKeyframe.Bloom.Colour`.rgb, Bloom `ff564907`) when bloom
   is enabled (view settings +0x1518 and renderer +0x9836, runtime flags), else (1, 1, 1, 0), which removes
   the bloom term; slot 0x9e (C, `GenericVector4_C`, not read by the composite) = (1 / Bloom `76826c13`,
   Bloom `da328699`, 0, 0) or (1, 1, 0, 0). **A** = renderer +0xa990 and **D** = renderer +0xa980, each times
   the view's vec4 at +0x15d0 (override, `FUN_008e3e30`); what writes +0xa980 / +0xa990 was not found.
   The Bloom fields reach the renderer (+0xac74…+0xaca4) through `FUN_008e51b0` (unpack, field order in the
   research log), `FUN_008e9610` (keyframe and state blend), `FUN_008ecf80` → `FUN_008ce250` →
   `FUN_008c9e40` (copy when bloom is on); the bloom set-up `FUN_008eaaf0` takes three of them
   (`c9f605e2`, `a3d336c1`, `cf7d9668`) and the vignette vectors at +0xa980…+0xa9bc.
   Blur kernel (exe programs `0x010109c8` VS + `0x01010d38` PS): 5 taps at 0, ±1.407333, ±3.294215 texels
   (× `kScale`), weights 6.533238, 9.728171, 3.005209 (sum 32 = the RGBM range: the downsample `0x0100ffa0`
   averages 4 taps decoded × 32 and re-encodes with M rounded up to 1/255), result ÷ exposure.
3. **Final pass** (12 variants created in `FUN_008e41e0`, e.g. `0x01016148`, `0x01017050`): offset the UV by a
   distortion texture, optionally blend depth of field, composite the "splatter" overlay (premultiplied:
   src × (1 − a) + rgb), then **out = TintBlend(rgb × `ColourCubeScalesOffsets`.xyz + .w)** (3-D texture,
   x = red). No other colour operation follows; the `BloomColour` constant in that buffer is unused by these
   variants.
4. **TintBlend** (the grade volume, 16³ × 4 bytes): `FUN_008e5f80` allocates the 0x4000-byte buffer and names
   it; `FUN_008e0930` starts a job with up to seven source cubes (ColourCube resource data) and a weight each;
   the job dispatches through the table at `0x010e8f70` by cube count (kernels `0x00eb56d0`…`0x00eb64c0`, not
   in the dump); `FUN_008e0ac0` copies the result into the texture row by row, without reordering. The default
   PostFX state (`POSTFX.BNDL` `…78ae`) contributes the identity cube `…78f1`; other states add their own
   (cooldown `…7898`, heat `…789d`, nitrous `…789f`, escaped `…789c`, wreck `…789a` / `…789b`) through
   `ValueBinding` strings. The caller is `FUN_008c9e40` (the frame's post set-up): seven cube slots from the
   frame state (+0x2c base, +0x34, +0x3c, +0x44, +0x4c, +0x64, +0x6c; each a resource + offset, a missing
   one falls back to the base) with weights +0x54, +0x58, +0x5c, +0x60, +0x74, +0x78 (the first two and the
   last two replaceable by a view override at +0x1568… / +0x1560…). If the six weights sum above 1
   (`DAT_00f8a820`) they are scaled to sum 1; the base cube gets 1 − sum. The PostFX state's two
   `ColourCubeSettings` cubes and [0, 1] weights (`FUN_008ecf80`, state +0x12e8…+0x12fc) are very likely the
   last two slots. Which slot holds the environment keyframe's cube, and with which weight, was not found
   (needs a live read of the frame state).

Consequence for the ColourCube layout: a straight weighted blend keeps the stored order, so the stored entries
are the texture's own (x = input red fastest, bytes B, G, R + alpha), not "blue fastest, stored R, G, B"
(inferred; the kernels themselves are missing from the dump; `mw_formats::ColourCube`).

## Sky, sun and shadows

Shader constants are registered by name and slot in `FUN_008c34b0` (one `FUN_00a5d490` call per name, e.g.
`KeyLightDirection` 0xd, `ShadowMap_Constants` 0x1c–0x1f, `ShadowMap_WorldToLight0` 0x21, `DomeRanges` 0x86,
`KeyLightDirAndXZLength` 0x89, `CloudParamsPS` 0x90, `CloudParamsPS2` 0x91, `BaseCloudColour` 0x92) and
written through `FUN_00a5d660(slot)`, which returns the slot's float4. Grep for `FUN_00a5d660(<slot>)` to find
a constant's writer.

- **NewSky constants** (`FUN_008b27e0`, called from `FUN_008bd6d0`); S = the interpolated environment state:
  - `DomeRanges` = (dome[8], 1 / (dome[7] − dome[8]), S+0x11e0, S+0x1334): the horizon-fade start, its
    inverse range and power, and the offset added to the sun-side gradient blend w.
  - `BaseCloud{A..D}ParamsVS` = (1, S+0x12e0+4i, S+0xec0 × k + S+0x1310+4i + S+0x12d0+4i, S+0x1320+4i): one
    scale and a scrolling offset per layer (S+0xec0 is probably the clock).
  - `CloudParamsPS` = S+0x12f0..0x12fc (the per-layer alpha thresholds); `CloudParamsPS2` = (S+0xc00,
    S+0x1300, 0, 0) (.y = the threshold sharpness).
  - `BaseCloudColour` = (S+0xbe0, S+0xbe4, S+0xbe8, s²), s = S+0xe44, or S+0xe48 when the call's last flag is
    set: **the sky gradient is scaled by a squared intensity**. Which keyframe fields feed these state
    offsets is not traced (the state is filled by a struct copy).
  - `KeyLightDirAndXZLength` = (key direction, √(x² + z²)).
- **Dome** (`FUN_008bca20`): 0x7e vertices, positions normalised to radius 1.0 (`DAT_00f8a820`), so the
  gradient coordinate |POSITION.y| is the sine of the elevation. Texcoord x is the distance along the
  vertex direction to the sphere at `DAT_010cf400..0c` (.bss; centre and radius); y, z come from the dome
  grid's angles (π, 2π, π/2 at `0x00ffc024..2c`) and become the cloud UVs in the VS.
- **Key light**: slot 0xd is copied from renderer +0x2920 in `FUN_008ad7d0`; its source is not traced.
- **Shadow constants**: `FUN_004a40a0` copies S+0xf00..0xf3f into `ShadowMap_Constants{,2,3,4}`;
  `FUN_008c8b00` fills them through `FUN_008a4910`(shadow object = renderer +0x16f10, out = S+0xf00, width,
  height), which packs the object's precomputed cascade matrices (+0x90, +0x220, +0x3b0 ...). The cascade
  fit itself is not located.

## Crashes (2026-10-06)

Data: `mw_assets::crash` (layouts in its doc comment); runtime: `crates/mostrusted/src/crash.rs`; values and model
decisions: [handling.md](handling.md), "Crash camera".

- **The test** (`FUN_009a9fd0`, confirmed): an impact is tested against one `ImpactRules` entry of the car's
  `CrashingRules` (reached through its damage bar profile, `vehicle+0x1178` → `DamageBarProfile+0x34`). It is a
  crash when any threshold that is present is exceeded strictly: `1617a710` against the impact's damage,
  `d6c0e908` against a third value, `2414aa68` against the closing speed times a `.bss` constant (inferred to be
  m/s → mph: the thresholds read as mph, 70 for scenery and 38–100 for landings). A car flagged at
  `vehicle+0x1777` never crashes. When it fires, the entry's `DamageOverride` (`DamageToDeal`, flag) replaces
  the impact's damage.
- **Which entry** (confirmed): car against car (`FUN_009575e0`) uses `PlayerRules` (0x24) when both are race
  cars ("race car impact"), else `22061460` (0x10, "AI impact") or `RoadblockRules` (0x2c, "roadblock impact")
  by a flag of the other car; traffic impacts ("traffic impact" / "crashing traffic impact",
  `FUN_00958130`) use `TrafficRules` (0x30). The closing speed there is the sum of both cars' velocities
  along the normal, and the damage values come from the impulse solve (`FUN_0093c240`: impulse over a
  response mass scale, clamped). `FUN_008fbb90` makes a car-on-car contact a crash outright when the contact
  point lies inside 0.8 × a box from the struck car's physics data and that car is on the ground, taking only
  the matching set's `DamageOverride`. Landings (`FUN_00915eb0`) use `LandingRules` (0x1c) with the speed into
  the ground; props and payloads have their own names ("prop impact", `FUN_00940d50`). Scenery:
  `FUN_0093bc40` copies the `WorldRules` (0x34) thresholds and override into the collision response block,
  so the world test happens inside the contact solver (the comparison was not traced; the contact record
  carries the crash flag at `+0xed`, and `FUN_009175e0` turns it into a crash).
- **Entering the crash** (`FUN_009145c0`, `FUN_009b2d40`, confirmed): the impact handler calls the crash
  entry with reason "Impact" / "damage applied"; it saves the car's pre-crash state (velocity at
  `+0x1124…` copied to `+0x114c…`, the "pre-carnage velocity" the sound reads) and sets `vehicle+0x1774`.
  Other entries: "wrecked" / "no health" when the damage bar's used segments reach the profile's segment
  count (`DamageBarProfile+0x74`), "voodoo logic" / "upside down" (`FUN_009168b0`, from the per-frame check in
  `FUN_00918600` when the body's up vector's y falls below a `.bss` threshold) and "over fatal collision"
  (`FUN_00916b70`, scripted). The player receives the result as a message (`FUN_006d3550`): crashed /
  wrecked flags at `player+0x23f` / `+0x23e`, the cause (0 world, 1 traffic, 4 battling; telemetry
  `GAME EVNT CRSH` / `WRCK`, `FUN_006be330`).
- **No control while crashed**: inferred from the player binding "Can the player drive away from his current
  crash?" (`player+0x42c`); the car keeps its physics and tumbles.
- **Safe pose** (`FUN_006d0f40`, confirmed): every frame the player is neither crashed nor wrecked, it saves
  the car's position, orientation and speed in mph (`vehicle+0x1620`) at `player+0x30…0x4c`, `+0x90`.
- **Reset** (`FUN_006d39d0`, `FUN_006c9fb0`, `FUN_006d0180`, confirmed): while crashed, in modes where the
  predicate `FUN_006dfed0` holds (not identified), a timer from the game mode (`GameMode+0x44`, `09d6656d`; no
  `GameMode` instance was found in the PC bundles) counts down by the scaled frame time; on expiry, or at
  once when the mode's `+0x68` flag is set, the reset starts. Its target is the saved
  pose (or the nearest race-route point when the mode's `+0x70` flag is set), moved onto the road by
  `FUN_00648060` (not read further). The car is placed rolling at `max(GameLogicConfig 75d82dd5 (30),
  a44eb2f3 (0.7) × saved mph)`, capped after a crash at `74521df4` (50) (0 when `FUN_006bd930` holds, not identified), when the mode
  allows rolling resets (a flag in the game-mode manager); otherwise from rest.
- **The crash camera** is a feedback `Sequence` (the second of `FeedbackMessageGroup` 457707's `Sequences2`,
  inferred from its content: the only sequence with slow motion and bystander shots, 5.5 s long). The code that
  plays it on the crash event was not found. `CameraSequenceItem` (`0x3f858`) is the one item type the dump
  tests by ID (`FUN_0049a050`); the other items' handlers are reached through vtables. The sequence clock is
  real time (inferred: at its 0.14 / 0.22 time scales the shots would otherwise last over 10 s each).
- Not found: the bystander camera's maths (its params are read through cached pointers, not in place), the
  scene analysers' update (vtables at `0x00fe4638` and nearby, `FUN_007d9840` and its siblings build them),
  the `SlowMoSequenceItem` handler.

## Input and control bindings

- **Actions**: 96 (ids 0..0x5f), one 16-byte record each in the name table at `.data` 0x010c3540
  (`{name, 0, id, flags}`; flags bit 0 = listed in the saved configuration). Driving: STEERLEFT 0, STEERRIGHT 1,
  ACCELERATE 2, BRAKE 3, HANDBRAKE 4, HORN 5, CHANGECAMERA 6, ROTATECAMERALEFT / RIGHT / UP / DOWN 7–10,
  SNAPSHOT 11, DREAMSHOT 12, WEAPON_DEFENCE 13, SWAP_CARS 14, LEADERBOARD 15, TAKEDOWNSTREAK 16, IGNITION 17,
  RESPAWN 18, RESPAWNOFFLINE 19, PAUSE 20, BIND 21, SHUTDOWN1–6 22–27, SIM_PAUSE1–3 28–30, CHANGE_VEHICLE 31,
  NITROUS 32, SPOT 33, SKIP_CUTSCENE 34, VOTE_A / B 35–36, LOOK_BACK 37, LOOK_AROUND 38, ENGINE_START / OFF 39–40,
  EASY_OPEN 41; then AUTOLOG_* menu actions 42–68, EASY_LEFT / RIGHT / UP / DOWN 69–72, UNIVERSAL_ACCEPT / ALT
  73–74, PM_CAMERA_* (photo mode) 75–… and dialog / keyboard actions up to 95. Context groups by name:
  CAR_ONLY, DIALOG, DEBUG, UNIVERSAL, EASYDRIVE (`FUN_0047c660`).
- **Binding record** (`FUN_00ac72a0`, 0x14 bytes, appended to a list at this+0xd0, count at this+0x2c2c):
  u16 action, u8 device, u8 enabled, u16 control, f32 min, f32 max, f32 scale = 1/(max − min) (1 when
  |max − min| ≤ FLT_EPSILON). **Value** (`FUN_00ac6850`): `max(0, (raw − min) × scale)`, no upper clamp, for the
  current and the previous frame; the pair gives the state idle / held / pressed / released. So STEERLEFT is the
  stick X with min 0, max −1 and STEERRIGHT the same with max +1; keys and buttons are 0 / 1, so keyboard
  steering arrives as −1 / 0 / +1 with no ramp at this level.
- **Pad** (device 1, XInput, polled in `FUN_004119c0`; 20 control slots of 12 bytes): 0 LX, 1 LY, 2 RX, 3 RY,
  4 L3, 5 R3, 6–9 d-pad up / down / left / right, 10 LT, 11 RT, 12 LB, 13 RB, 14 Back, 15 Start, 16 A, 17 B,
  18 X, 19 Y (from the `wButtons` bit tests). Sticks: `a = |v| / 32768` (`.rdata` 0x00f91580), per axis (not
  radial) `sign × clamp((a − 0.2) / (1 − 0.2), 0, 1)`, dead zone 0.2 at this+0x140 (`.rdata` 0x00f9156c).
  Triggers: `clamp((byte / 255 − 0.1) / (1 − 0.1), 0, 1)`, dead zone 0.1 at this+0x144 (`.rdata` 0x00f91570;
  `FUN_00414920` copies a different value into +0x144 of its own object, not traced). Buttons 0 / 1.
- **Default pad table** (`FUN_00482250`; min 0, max 1 unless noted): steer LX (left max −1, right +1),
  accelerate RT, brake LT, handbrake X, horn B, change camera LB, rotate camera RX (left max −1) / RY (up +1,
  down max −1), nitrous A, weapon-defence B, swap cars Y, change vehicle Back, leaderboard Back, takedown streak
  d-pad right, snapshot R3, dreamshot A, ignition L3, respawn X, respawn-offline Back, spot Back, skip cutscene
  Back, vote A / B = Y / B, pause Start, BIND Start, shutdown 1–6 LB, LT, RB, RT, Start, Back, sim-pause 1–3 LT,
  RT, A; EASY_LEFT / RIGHT / UP / DOWN d-pad left / right / up / down with **min 0.8** (`.rdata` 0x00fa1db8).
  LOOK_BACK, LOOK_AROUND, ENGINE_START / OFF and EASY_OPEN have no default binding.
- **Default keyboard table** (`FUN_00483470`, device 2, Windows virtual-key codes, all min 0 max 1): steer
  Left / Right arrows, accelerate A, brake Z, handbrake Space, horn W, change camera C, rotate camera Delete
  (left) / Page Down (right) / Home (up) / End (down), nitrous Left Shift, weapon-defence W, swap cars E, change
  vehicle Q, leaderboard Tab, takedown streak Num 6, snapshot P, dreamshot Enter, ignition I, respawn Space,
  respawn-offline Tab, spot Tab, skip cutscene Esc, vote 1 / 2, pause Esc, BIND Enter, Easydrive Num 4 / 6 / 8 / 2,
  Autolog menus on the arrows, Enter, Backspace, PgUp / PgDn (tabs), F1, F10 / F11. No LOOK_BACK, LOOK_AROUND or
  EASY_OPEN key.
- **Mouse** (`FUN_004843b0`, device 3): button 0 weapon-defence, button 1 swap cars.
- **Saved configuration** (`FUN_0047cf90`): `GetPrivateProfileStringW` per action whose flags bit 0 is set, the
  line `ACTION=<device id>,<control name>,<min>,<max>`; the control name is looked up in the device's control list
  and the binding added with `FUN_00ac72a0` (whether the defaults are cleared first was not traced). `NumConfigs` / `Config_` /
  `Controllers_` sections (`FUN_0047d250`, 0x00480000 block) hold several configurations and the chosen device.
- Downstream, `FUN_008fa750` passes stored control fields (vehicle-controller words 0x7ed–0x7f3) to
  `FUN_009aacc0`; the steering value is right − left (Update loop above).

## Front end, HUD and Easydrive

_(session 5)_ Easydrive's tree, labels, item types and input actions are data (research log 2026-10-06
session 5). The code side is not located yet: the callback names the data invokes (`ExitTierVia`,
`EasyDriveOnNext`, `EZStateJack::SelectManufacturer`, `SelectPaintPackFromEasyDrive`, ...) appear in
`strings.csv`, but the dump has no references to them, so the functions that register the hooks, generate the
manufacturer / model / race lists (`Signals.GetIntValue` sources) and map `easy_*` to keys are still to be found.

## Free-roam rules: collectibles, jack spots, events

Speed cameras, billboards, security gates, jack spots and body shops are trigger volumes in the world's one
`TriggerData` (`HAWAII/TRIGGERS.BNDL`, formats.md "TriggerData"); their kind byte (record +0x10) says what they
are. `mwtool freeroam [--list|--kind SpeedCam]` lists them.

- **Loading** (`FUN_0053c2f0`): loads `TRIGGERS.BNDL`, then walks boxes (0x30-byte records, header +4/+8),
  spheres (0x18, +0xc/+0x10) and points (0x20, +0x14/+0x18) as one list and stores id (+0xc) → list index in a
  sorted map. 300 entries are reserved per side for something at `DAT_010edb3c+0x622a0/4` (open).
- **Kind codes** (`FUN_006e4920`, the `GAME EVNT COLL` telemetry event with tags `coty` = kind name, `coid` =
  id, and Online/Offline): '0' (48) / 'H' SecGate; '2' (50) / 'A' 'D' 'J' 'M' Billboard; '4' (52) / 'G'
  SpeedCam; '<' (60) / 'B' 'E' 'K' 'N' JackspotLocation; '=' (61) / 'C' 'F' 'L' 'O' JackspotCar; 6 BodyShop.
  The letters are the same kinds for other content packs; the base TRIGGERS.BNDL only has the digit codes.
- **Hit handler** (`FUN_007059b0(this, player, trigger record, …)`): looks the record's id up in its id → index
  map (`this+0xc`), finds the per-kind "already found" bitset (`this+0x420`, 0x94-byte groups: 0x90 bytes of
  bits, the kind char at +0x90) and dispatches on the kind:
  - **Speed camera** ('4'/'G'): skipped if the camera is in a second bitset (`this+0x1108`) unless a flag at
    `…+0x2e95` is set; otherwise `FUN_006ed0a0`. That reads the car's speed with `FUN_006ba3e0` →
    `FUN_0083ea90` (vehicle +0x1620, the speed in mph, see "Update loop"; without a vehicle |velocity| ×
    `0x0118a060`) and takes its absolute value. **The camera only counts at or above
    `CollectablesConfig` field `d89eeda4` (offset 0x64) = 50 mph**: the threshold is read as
    `GameLogicConfig` (global config +0x24) → field +0xc (the `CollectablesConfig` handle) → +0x64. The recorded
    value is `(int)(100 × |mph|)` (`DAT_00ff8ec0` = 100.0), i.e. hundredths of a mph, handed to the speed-wall
    / Autolog record `FUN_008749e0(id, player, centi_mph, 0, id)`; `FUN_006e1c10` compares it with the
    friends' records. The first time a camera is found (bit clear) a list entry in `this+0x34` (5 words per
    camera) is marked and the "found" event (`GameLogicConfig`-side id at +0x28c of the config at global +0x44)
    is sent; every pass sends the event at +0x294. Hint `FUN_006ef070(4)` is shown once.
  - **Jack spot location** ('<' …): counts only while `FUN_006e44f0(id)` is true, which reads the script
    variable `JackSpots.IsSwapSpotVehicleSpawned[n]` (n = index + 1), i.e. only once the parked car is there;
    first find sends the event at config +0x3c and updates the list (`FUN_006fc600`, `FUN_00702710`).
  - **Jack spot car** ('=' …): ignored if `FUN_006ed400(id)` (already used/driven) and requires the same
    spawned flag; first find sends +0x38.
  - **Security gate** ('0'/'H'): events +0x260 / +0x288 on first find, hint 7 (`FUN_006f3f30(7)`), stats
    0x44 and 0x57 (`FUN_006687e0`).
  - **Billboard** (other kinds reaching the tail, '2' …): `FUN_006ed330` remembers the first billboard of
    the frame (`this+0x3008/0x300c`, the "JustHitBillboard"/"LastItemBillboard" flags of the strings) and
    sends event +0xfc; first find sends +0x1c / +0x18, hint 5, stat 0x45.
- **Who calls the handler**: `FUN_006ab390` (the game-logic message loop over the queue at `DAT_010edb10`)
  and `FUN_007061d0`, `FUN_00707510`, `FUN_0065fc00`. The overlap test that posts "car entered trigger" is
  not traced yet, so whether the car's origin, its body or a smash of the prop fires a gate / billboard is open.
- **Speed Points**: not found in this pass. The handler only sends events (ids in the config at global
  +0x44); the points are presumably awarded by whatever listens to them (`FUN_006a8050` dispatch).
  `CollectablesConfig` holds no point values (its floats are 2.5 at 0x60 and the 50 mph threshold).

## Traffic and AI vehicles

First pass (session 5); the lane format is in formats.md "TrafficLaneData (0x701)". What is traced so far:

- **Loading** (`FUN_006077c0`, the game's load state machine): case 0 queues `\TRAFFIC.BNDL` with a
  `"TrafficData"` loader into `world+0xa3c90`; case 1 copies the lane data pointer to `world+0x697fc` (and
  `+0xa3c94` to `+0x69800`) and calls `FUN_005e9350` with a byte at GeneSys field `0x26` and a word at `0x40` of
  that object; case 4 loads `VEHICLES\TRAFFICATTRIBS.BNDL` (the traffic `VehicleComponent`s the lane data's
  vehicle id list names). Network names in `00610000.c` ("Traffic Hull Sync", "Crashing Traffic", "No Crashing
  Traffic") show traffic state is synced per hull, and a traffic car switches between a non-crashing (on rails)
  and a crashing (physical) state.
- **Placing a car on a lane** (`FUN_005f4d10(car, hull, section, rung_pos, ..., lane_data, ...)`): stores hull,
  section and the rung position (float; its integer part is the rung within the section, the first rung index is
  added for each of 4 wheel/probe slots); the section record is `section * 0x30 + hull[+0x10]`. The **cruise
  speed** is `TrafficFlow.Speed` (field 0x24) of the flow the section's byte `+0x24` selects in the lane data's
  flow import list (header 0x10), times `0.44704` (`DAT_010c4d24`, `.data`): Speed is in **mph** (flows hold 80
  on the main roads). It is written to two slots (current and target speed). Another speed-like value is
  `TrafficVehicle` field 0x10 times field 0x48 of the global GeneSys object at `DAT_0110f940` (`.bss`, needs a
  live read). The car's front / back extents along the lane are `box.centre ± box.half_extent` (`param_12+8`,
  `+0x18`); when `TrafficVehicle` field 0x20 is set the back extent grows by 10 m (`DAT_00fb23c0`), most
  likely a trailer (the manoeuvre config has a `JackKnifeManoeuvre`). Unset links are `0xffff`, the free-space
  distance starts at `FLT_MAX` (`DAT_00fb1784`).
- **Kinematic body** (`FUN_005f7cf0`, 5.7 kB): a traffic car with 4+ wheels and speed above `DAT_00fb1db8`
  gets per-wheel pose maths (suspension / body roll on rails); not traced further.
- **Engine sound** (`FUN_005ee480`): registers `Game::Traffic::EngineSoundInstance` with RPM,
  `NormalisedSpeed` (speed / top speed), `IsWetSurface`, `IsReversing` and a per-car `Random`.
- **Impacts**: `FUN_00958130` (see "Crashes"), `TrafficRules`.

Tuning lives in `GLOBALCONFIG.BNDL`: `TrafficConfig` (0x69443) → `SystemConfig` (0x69446), `ParamConfig`,
`VehicleConfig`, `FuzzyConfig`, `ManoeuvreConfig` (JackKnife, Panic, Cutup, Weave, NearMiss, Swerve,
SlowSpeedPanic). Almost all field names are unhashed. `TrafficSystemConfig` starts with a 24-entry array
(0.05 at 00:00-02:00 rising to 0.5 at 08:00, 0.3-0.4 by day): the **density by hour of day**. Its scalars
include 1500, 205, 300 / 200 / 250 (distance-like), `PursuitScale` 0.5, `PayloadDelay` 0. Which code reads
them (spawn / despawn radius, car count, near-miss and horn logic, the switch to a physical car) is still open:
the next step is to find the readers of `world+0x697fc` and of the `TrafficConfig` handles.

### Traffic rules (session 5, round 2)

Notation: `sys[x]` = `TrafficSystemConfig` field at instance offset `x` (handle `world+0xb17b0`, instance offset
`world+0xb17b4`; offsets from `mwtool genesys-types`, values from `GLOBALCONFIG.BNDL`). `prm[x]` = the GeneSys
object at `DAT_0110f940` / `DAT_0110f944` (`.bss`). Its field values fit `TrafficParamConfig` (0x69441) at every
offset read, which is why it is taken to be that object: 0x10 = 4, 0x60 = 25, 0x6c = 2, 0x88 = 2, 0xac = 3. This
is **inferred**. A live read of `DAT_0110f940` would confirm it. `mph` = `0.44704` (`DAT_010c4d24`).

**Tick order** (confirmed). `FUN_0060ebc0` is the traffic system's frame function. It handles the message queue
(`FUN_0060a380`), then branches on the mode at `world+0xa2b40`: 0 = start-up, 1 = local game, 2 = teardown.
In mode 1 it calls `FUN_0060eab0`, which runs:
- `FUN_006024f0(first, 0x100)`: the per-car update over the 256 lane-follower slots (from `+0x1bfd4`, stride
  0x80). Each car's step is `FUN_005fc220`.
- `FUN_0060c9f0`: the active hull set (below).
- `FUN_0060e5e0`: the emitters (spawning).
- `FUN_00605d30`, `FUN_00602ae0`, `FUN_005f0aa0`, `FUN_005fbbf0`, `FUN_0060ba30`, `FUN_0060b9a0`: not traced.

The world's vehicle table is 0x1af (431) slots of 0x80 bytes at `world+0x60`. Teardown frees three ranges of
it: slots 0-255 (traffic), 256-405 (150) and 406-429 (24); the roles of the last two are inferred (parked
cars?). Slot `+0x62` holds a linked slot that is removed together with the car (a trailer, inferred).

**Active area** (confirmed, `FUN_005f0270` in a local game). The active area is the hull cell the player is in
(`FUN_0043cf00` with the last cell as a hint) plus that cell's own neighbour list (hull cell `+0x04`, count
`+0x18`, up to 10). In a network game there is one such list per player, 16 lists at `world+0x67d84`, stride
0x18. `FUN_0060c9f0` takes their union (`world+0x37c80`, count `+0x37dc0`) and compares it with the previous
frame's set. Hulls that left the set lose their per-hull state (`world+0x2ae00` hull → slot, records of 0x498
bytes at `+0x37f58`). So there is no despawn *radius*: traffic lives in the cells around the player's cell.

**Emitters** (confirmed, `FUN_0061c780`). An emitter is a section of an active hull that:
- has a flow with `Density` (`TrafficFlow` +0x20) ≠ 0, and
- does not continue a lane already inside the area: its first backward link (section `+0x0E` hull, `+0x17`
  section) is missing (`0xff`) or lies in a hull outside the active set.

So cars enter only at lane starts and at the edge of the area, never in the middle of it (no view test was seen in the emitter code). Each emitter
holds a timer at `world+0x2a600`. Its first value is `interval × 0.5 + phase`, where `phase` starts at 0, grows
by `sys[0x98]` (0.7) per emitter and is wrapped to [0,1).

**Spawn interval** (confirmed, `FUN_005fb120`):
- `D` = the section's flow `Density`, `m` = the global multiplier `world+0xa2b7c`.
- `d = max(m·D, min(D, sys[0xd0]=0))`, floored at `1e-15` (`DAT_00fb1e90`).
- `interval = 60 / d × lerp(a, sys[0xb8]=1, u)`, with `u` uniform in [0,1). The decompiler lost the lerp's
  start `a`.

So **`Density` is cars per minute per lane**, at multiplier 1.

The random source is a 64-bit LCG: multiplier `0x5851f42d4c957f2d`, increment 1, state at `world+0x36190`. Its
high word goes through `>>9 | 0x3f800000` into an 8-slot ring of floats in [1,2) at `+0x36170`, and each draw
returns the slot written 8 draws earlier.

**Spawn step** (confirmed, `FUN_0060e5e0`). Spawning runs only while the load `world+0xa2b80` is below 200
(`DAT_00fb1ab0`). Each emitter timer counts down by dt (`world+0xa2b5c`). When it reaches ≤ 0:
1. Pick a vehicle type from the section's flow type (`FUN_00608c80`, section byte `+0x25` = `TrafficFlowType`
   index).
2. Place the car `Speed·mph·|timer|` metres into the section (`FUN_006146b0` on the cumulative rung lengths),
   that is, where it would be had it spawned on time.
3. Check the last car already on that section (`+0x37f58 + section·2`, `0xffff` = none). The new car is placed
   (`FUN_00608720` → `FUN_005f4d10`) only if the gap to that car's back is at least `Speed·mph × prm[0x6c]`
   (2 s headway). Otherwise this slot is skipped.
4. Add the next interval to the timer.

**Vehicle pick** (confirmed, `FUN_00608c80`):
- `total` = the sum of `Proportion` (inline struct +0x1c) over the flow type's entries. When a class filter is
  given (≠ −1), only entries whose vehicle has that class (`u16` at +0x14 of the vehicle's +0x28 object) count.
- `r = u·total`. The first entry whose running sum reaches `r` is taken.
- Overrides: in a local game, a forced flow type at `world+0xa2bf4` wins, and a mode flag
  (`FUN_006a04c0()+0x2eb8`) can remap the flow type through `FUN_0061bd80`.

**Density multiplier** (confirmed, `FUN_005f0780` every frame):
- Start with `m = base`. `base` (`world+0xa2b74`) is `sys[0x80]` = **4.78**, unless message 4 in
  `FUN_0060a380` sets another value or restores `sys[0x80]`. In a network game, `m` is also scaled by
  `lerp(sys[0xc0], sys[0xd8], (activeHulls − c)·k)`, capped at 1.
- Player speed (local game, when `FUN_0069ffe0()`): with `s` = the player's speed,
  `t = clamp((s − sys[0x8c]·mph) / (sys[0x94]·mph − sys[0x8c]·mph), 0, 1)`, and `m *= lerp(sys[0x90]=1,
  sys[0x88]=1.2, t)`. With `sys[0x8c]` = 60 and `sys[0x94]` = 20, that is ×1.2 at or below 20 mph and ×1 at or
  above 60 mph.
- Pursuit (`FUN_006a1200()`): `m *= sys[0x10c]` (`PursuitScale` 0.5).

The **24-entry hour array (`sys[0x0c..0x6c]`) has no direct reader** among the `world+0xb17b0` users. It may
reach `base` through message 4 (not traced).

**Load budget and culling** (confirmed, `FUN_005fd6a0`). Every car within `sqrt(DAT_0110f590)` (`.bss`) of the
point `world+0x36110` (the player or camera) adds to the load `world+0xa2b80`:
- `sys[0x9c]` = 30 if the car is physical (byte `+0x1d2` of its 0x620-byte record at `+0x2c910`);
- otherwise `sys[0xa4]` = 5 if the value at `+0x1c0` is below `sys[0xb4]` = 6 (speed, inferred);
- otherwise `sys[0xa0]` = 20.

At a load of 200 or more, a cooldown (`+0xa2b84`) counts down. When it runs out, the **farthest** car in the
radius is removed (`FUN_005ec4b0`, with its linked slot) and the cooldown restarts at `sys[0xa8]` (0.5 s). The
budget allows about 10 moving cars, about 40 stopped ones, or about 6 physical ones near the player.

Separately, `FUN_00602ce0` runs `FUN_005fd3f0` (not traced) once the player has been slower than `sys[0x108]`
(2) for `sys[0x104]` (30) seconds.

**Car following** (partly confirmed, `FUN_005fc220`). `prm` values are inferred, see above.
- Look-ahead: `L = 1.5 × max(prm[0x88]·v, prm[0x60])`, that is `1.5 × max(2 s·v, 25 m)`, where `v` = speed at
  `+0x1c` of the car state. The constant 1.5 is `DAT_00fb1bb0`.
- `FUN_005f1380` scans the lanes ahead (following forward links) for the next car within `L`. It returns the
  car's index or `0xffff`, and the distance.
- `gap = max(0, dist − back(+0x4c) − prm[0x10] − prm[0xac]·x(+0x3c))`. With `prm[0x10]` = 4 m and
  `prm[0xac]` = 3, `x` is probably a speed, giving a time headway (inferred).
- `closing = v − v_ahead`, where `v_ahead` is the other car's `+0x1bf9c` slot.
- These, with `prm[0x84]·x + (+0x34)` and `prm[0x80]` (only when the state byte `+0x22` = 1), go to
  `FUN_00617d00`.

`FUN_00617d00` is a **behaviour arbiter**, not a plain speed controller (partly confirmed):
- `FUN_005ea6a0` scores four options from the gap, the closing speed and the car state. The scoring is
  probably the `FuzzyConfig` (inferred).
- The scores are forced to zero while `world+0xa2be6` is set. They are also zeroed when `world+0xa2ba8 ≤ 16 −
  world+0x32b20`, a cap on concurrent manoeuvres (inferred); in a local game that test is skipped at a load of
  200 or more. A car whose state byte `+0x78` is 2 gets fixed scores (`DAT_00fb2330`).
- The best score picks the branch.
- The cruise target in every branch is `Speed·mph·world+0xa2bfc` (a runtime speed scale).
- Branch 0 sets the car's mode to 5 and a rate of `lerp(prm[0x98]=4, prm[0x94]=8, x(+0x3c)) ×
  DAT_0118a090` (`.bss`). It may also queue the car into a 5-entry list at `world+0x32bac`, as a manoeuvre
  request (inferred).
- Branch 1 uses `max(0, score − prm[0x9c]=10)`.
- The remaining branches were not read.

`prm` names `MinAcceleration` (−20, offset 0x50) and `SwerveScale` (4, offset 0xb8), the only hashed names.
Not traced: the controller itself (`FUN_00617d00`), the swerve and near-miss reactions, the horn, junction /
stop-line handling (junction records are reached through the hull's `+0x28` pointer; `FUN_0060c9f0` walks a
`u8` count at junction `+0x35` with entries near `+0x56`), the switch to a physical car (`FUN_005fcbd0` →
`FUN_005f7cf0`) and the 150 / 24-slot ranges.
