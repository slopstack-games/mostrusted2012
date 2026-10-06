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
  T = min(5000 N m (0x01186f70), clutch × |Δω| × I_eff / dt) along Δω, where 1/I_eff = 1/`Engine_inertia` +
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
- Not resolved in the drivetrain: K (0x01187250), the clutch-in throttle exponent (0x011874d0), the stall
  fraction (0x011874a0), the downshift gate (vehicle +0x1ed0 against 0x01187580), the objects behind
  `FUN_009262a0`'s override list, and why the original's engine reaches full torque in the first tick of the
  launch (`ref/xkr_launch` 1.45 s: +140 rpm per tick = the full-load torque, while the lag would give ~0.4 of
  it). The launch also waits 0.45 s after the throttle with the car parked (brake ramp 1, gear index 0), which
  the standstill state's timers or the parked mask in `FUN_009d6170` (controls +0x40) may explain.
- **Differentials** (`FUN_009d8140`): `Front_` / `Rear_Differential` = c limit the axle's wheel-speed ratio to
  1/c (0 open, 1 locked; 911 rear 0.75). `Center_Differential` is 0 on every car.
- **Top speed**: `Top_speed_MPH` is never read by the physics. Top speed follows from torque, drag (with its
  over-speed ramp, below) and the rpm clamp in the highest unlocked gear.
- **Brakes** (`FUN_009b1180`): torque per wheel = `6ade4bb6` (front) / `66251131` (rear) × pedal² × a ramp that
  rises at 10 × pedal per second. A brake can stop a wheel but never reverse it. **No ABS, no traction
  control.** During a burnout the driven axle's brakes are released. `99407bbc` (15 everywhere) is a standstill
  hold deceleration.
- **Handbrake**: strength blends from `Brake_strength_at_low_speed` to `_at_high_speed` between `Low_speed_mph`
  and `High_speed_mph`, rear wheels only, combined with the service brake by max (not added). `35e18207` /
  `6ce0fda7` belong to the drift (above), not to grip.
- The drivetrain constants are now accounted for (above); the ones not yet read are listed at the end of the
  gearbox entries.

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
