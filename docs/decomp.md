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
  3. the caps (`FUN_009c5da0`), then one pass of the longitudinal constraint coupled with the wheel's spin
     (`FUN_009bc340`, `FUN_009bc0c0`) and of the lateral one (`FUN_009c6f00`);
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
- Unknown: the constraint +0x210 scale on the blend inputs (no writer in the dump; read it with the probe), which
  state picks `35d32b7d` for the downforce point, and the meaning of the contact's +0x14 lane.

## Engine sound

See the research log, "engine sound from the game code": the sound runs on its own 1000–8000 rpm axis
(audible = 1000 + 7000 × normalised rpm^(2 − a)), engine load is the pedal blended towards full by speed, and the
harmonic multipliers come from `EngineSound2`. The Distortion and SubHarmoniser plug-ins fall in undecompiled
gaps of the dump.
