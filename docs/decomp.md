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
  9. `FUN_008f7ab0` (profiler scope "Update wall scrape"): if the car's bit (index vehicle +0x1768) is set in
     the contact table at vehicle +0x303c (bitset words at table +0xc) and its record (table +0x10, 0x80 bytes
     per car) has its fourth float (+0x1c) above 6 (`.rdata` 0x01021ff0), lanes 0 and 3 of the steering block
     are zeroed: +0x1c80 (angle as a fraction of `Maximum_steering_angle`) and +0x1c8c (radians, the tyres'
     input); +0x1c84 and the range +0x1c88 are kept (from the disassembly: two shufps pairs against a zero
     register). The next tick's tyres run straight and the steering ramps out from the centre. The same record
     field drives the camera's impact shake (`FUN_005cd040`, range 5..60, reached through world + 0x5ddf0) and the
     drift's code-1 exit; its writer (the collision analyzer) and unit were not traced.
  10. `FUN_009ae670`: the "every wheel down" flag (+0x1772, read by the next tick's drift), a final
      integration, the speed again; then the time since the last world contact (+0x3010) grows by dt.
- **Integration**: each stage adds forces and torques to the body's accumulators and `FUN_00907840` integrates
  them into the velocity before the next stage; the damping stages (`FUN_00907b50` angular, `FUN_0091dad0`
  linear) and the turn-around assist set velocities directly. Neither `FUN_00907840` nor the island's
  `FUN_009282d0` (gravity × body +0x9c, a speed cap at +0xbc = 447 m/s) damps anything.
- **Angular damping** (2026-10-09). `FUN_00907b50(body, d, dt)` multiplies the angular velocity in body axes
  by (1 − d)^(dt × 60) per axis (d a four-lane vector, nothing done when dt is 0). Its per-tick callers on a
  driving car:
  1. `FUN_009b2bb0` → `FUN_009aa700` with the record at vehicle +0xe0: the `Physics.DampingConstraint` the car
     uses, per axis a damping below and above a speed threshold (linear lanes by horizontal, upward and
     downward speed; angular lanes by |ω| about each body axis), applied through `FUN_009b7160` (linear) and
     `FUN_00907b50`. `PHYSICS/DAMPINGCONSTRAINTSLIST.BNDL` holds `CrashingVehicle` (angular 0.02 above 2 / 3 /
     4 rad/s, linear 0.005 above 50), `DrivingVehicle` (**empty**) and `Prop`. Live (both snapshots, player
     at 115 and 36 mph): every damping 0, every threshold FLT_MAX, the record's pointer +0x60 set: **a driving
     car gets no damping here**.
  2. `FUN_008f7950` (from `FUN_00904da0`, before the driveline): yaw damping after a collision. Three timers
     (vehicle +0x2e80, +0x2e90, +0x2ea0) are set to 1 by `FUN_008f5620` / `FUN_008f5640` / `FUN_008f5660`
     (skipped in the special state +0x1774), called by the collision analyzer `FUN_00918600` (16 car slots,
     the slot table at world +0x53220 / +0x55c48) through its three handlers: `FUN_00918120` for the world
     record (the 0x80-byte table the wall scrape reads; only on a contact's first tick, the analyzer's flag
     at +0x220 + slot holding it off while the contact lasts; the slot's type byte must be 1, as the
     player's is live), `FUN_00917bb0` for another car (0x90-byte entries) and `FUN_009175e0` for an object
     (0xa0-byte entries). Each handler first needs a record value above 1.19e-7 (`.rdata` 0x01023160). The
     tick damps the body-up lane only, d = 0.05 (0x01021fd0) × the largest timer, then lowers the timers by
     dt / 1, dt / 1 and dt / 0.3 (0x01021ef0, 0x01021fe0), clamped at 0 (read from the disassembly in snap2).
     Ported (drive.rs `ImpactYawDamping`).
  3. `FUN_009d11d0`: the drift's damping (below). `FUN_009cfce0`: the airborne state. `FUN_008f9f40` (another
     physics object's update with constants at 0x010220e0) and `FUN_00919240` (a network message, case 0x32)
     are not the player car's tick.
- **Chassis inertia** (live, 2026-10-09): the body record (vehicle +0x10 [+0x1e0]) holds the local inverse
  inertia at +0xe0, the world one at +0x40..+0x6c, the inverse mass at +0x8c. Both snapshot cars match a solid
  box with the chassis's half extents about the centre of mass, × `Inertia_Scale`, exactly (1762 kg, 0.923 /
  0.581 / 2.342: 3418.5 / 3720.5 / 698.8 kg m²; 1807 kg, 122769: 3348.6 / 3724.0 / 774.5): the bounding
  box's offset from the centre of mass and the convex hulls do not enter. Ours is the same formula. The rigid
  body's `AngularDrag` / `LinearDrag` (1 and 1 on cars) were not found applied to a car.
- **Body movement** has no per-tick stage: `BodyMovementBehaviour` is read when the vehicle is built
  (`FUN_008fda70`: roll damping, the in-air springs `ddbd44c0` / `b80e5de3`); its `Take_off_behaviour` is the
  airborne state's data. The type `BodyMovementBehaviour.Angle_control` exists but no car's data has one.
- Speed-banded lookups use the speed in mph signed by the direction of travel (the full speed, not the forward
  component).

### Profiler scope tree (Vita↔PC cross-check, 2026-10-07)

Both builds carry an identical indented profiler-label table (PC `strings.csv` 0x01022d38..0x01022f10; Vita
0x816f4590..). It names the per-vehicle tick's scopes; the labels have no code xref on either build (the
profiler computes the address), so the FUN_ bindings come from the call tree + the anchors above, not the
strings. Depth (indent) and PC binding:

- d0 **Update Race Car Physics** → the per-object-type update **vmethod** (vtable-dispatched; `FUN_00904da0`
  is the body, which has no direct caller in `functions.csv`, consistent with virtual dispatch).
- d1 Attribsys Update · Apply handling forces · Update physics object · LineTests · Update network state —
  the d1 siblings are pushed by the (undumped, data-section) object-manager dispatcher, so most don't bind to
  a FUN_. "Apply handling forces" is the force-accumulate/integrate stage around `FUN_00907840`.
- d1 **Update RaceCarPhysics** → d2 **Update RoadVehiclePhysics** (candidate `FUN_009b4d70`, step 1 above) and
  d2 **Update Car** = **`FUN_00904da0`** (confirmed — the "Order in a tick" body).
  - d3 **Update Handbrake** = `FUN_008f7c50`(+state `FUN_009d27b0`); **Update Engine** = `FUN_009d9960`;
    **Update Driving** = **`FUN_00904260`** (its callee list matches steps 1–10 exactly); **Update sound** =
    `FUN_009b3030`→`FUN_009b1f70` (newly tied to this scope).
  - d4 under Driving: **Update steering** = `FUN_009aae10`→`FUN_009d73f0`; **Update wall scrape** =
    **`FUN_008f7ab0`** (verified: reads the contact table at vehicle +0x303c, clears the steering-output
    block +0x1c80..+0x1c8c — newly tied to this scope and confirming step 9).
- A **second, shallower "Update steering" at d2** (after Update Car) exists in the table — a separate steering
  invocation at the RaceCarPhysics-wrapper level, distinct from the d4 one inside Driving. decomp.md
  previously documented only the single `FUN_009d73f0` steering; the second scope is a new structural note
  (PC candidate: the second `FUN_009aae10` call in `FUN_008fa750`, UNCERTAIN).
- d1 Update vehicle info · **Update constraints** (⊇ the island solver `FUN_0093f5a0` family, which hosts the
  still-undecompiled anti-roll/damper solve `FUN_00932af0`) · Update freezing · **Update wheel fr input** =
  "wheel friction input", candidate `FUN_00905910` (the per-tick tyre normal-load prep; vmethod, no caller).
- d0 **UpdatePreSolveVel** / **UpdatePostPhysics** — pre-solve velocity and post-solve write-back in the
  solver-driver chain `FUN_00906490`→`FUN_00904a40`→`FUN_009024e0`→`FUN_00901d50`→`FUN_0093f680`→`FUN_0093f5a0`
  (not isolated to single functions; UNCERTAIN).

**Label order vs execution**: the table lists wall-scrape before the d4 steering, but the PC *executes*
steering (step 7) before wall-scrape (step 9) — the label layout is the authored scope order, not the tick
execution order. Everything else matches. The d1/d0 phases that don't bind are all vtable-dispatched through
the data section absent from the memory dump; the Vita build does not name them either.

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
  target is offset + input × (limit − offset). q is vehicle +0x2c70, the first lane of a 4-float vector
  (+0x2c70..+0x2c7c) that `FUN_00904260` and `FUN_00902930` copy to the stack and pass through `FUN_009aae10` /
  `FUN_009d42d0` into `FUN_009d73f0`; `FUN_008f8cd0` reads it as the typed index `[0xb1c]`. No writer was found
  in the whole dump (the 0x006c/0x006e/0x0074/0x0076 writers of a +0x2c70 address the game-mode object at
  +0x2f3fc; `FUN_008e8c40`'s `+ 0xb1c` is a byte offset). The original's telemetry (`ref/xkr_launch`, the probe's
  `steer_q`) shows what it is: **q = body slip / 35°**, slip = atan2(v · right, |v · forward|), the ratio
  0.0286 per degree to four digits from 0.5° to 33°, times speed / 10 mph below 10 mph (constant over 1–4 m/s).
  The steering follows −0.3 q in that run (`steer_norm` −0.20 at q 0.63, −0.09 at q 0.21, outside the drift),
  which turns the front wheels towards the direction of travel (the front slip angle drops by the wheel angle):
  a counter-steer assist. 35° is also the XKR's `Maximum_steering_angle`.
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
- **Exit** (`FUN_009d83e0`, codes at drift +0x10): forced by a collision (1: the car's world record in the contact
  table above 6, `.data` 0x010cfb40, the same field as step 9 of the update loop; 3: any of the car's object
  entries, count byte at table +0xdc + index, 0x90-byte entries from the pointer at table +0xec + index × 8,
  value at entry +0xc above 6, `.rdata` 0x01026230; later codes overwrite earlier ones), more than
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
  on every car: 1). Times the surface factor (`FUN_009cede0`: the mean effective lateral surface grip of the four
  wheels, "Tyres"), `VehicleInfo.40634c09.y` (1 on every car) and the upgrade function (identity without an
  upgrade).
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
- Unknown: the constant at 0x01187250, the counter-steer exponent's source, the collision exits' units. (The
  per-wheel surface factor is the mean effective lateral surface grip, "Tyres", 2026-10-09.)

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
  T = min(5000 N m (0x01186f70), clutch × |Δω| × I_eff / dt × k) along Δω (k: the `.bss` vector 0x011873e0 = 2π/60 in every lane, read live 2026-10-06: the game's Δω is in rpm,
  engine +0x20 against `FUN_009d55f0`'s rad/s × 60/2π, so k turns it into rad/s; ours works in rad/s already), where 1/I_eff = 1/`Engine_inertia` +
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
- **Gearbox modes** (2026-10-06, session 5): `FUN_009da600` runs `FUN_009ce630` after the pedals are copied
  and before the state machine. Mode at vehicle +0x1bdc (gearbox +0x16c), a qualifying timer at +0x140 (reset
  to 0 on any tick that does not qualify) and the time since the last switch at +0x144 (+= dt every tick,
  0 on a switch). Mode 0 → 1 when the timer passes 3 s (`.rdata` 0x01026180) with 0 < throttle (control
  block +0x1bf4) < 0.95 (0x01026160), brake (+0x1c00) ≤ 0 and a flag clear (`FUN_009b3030` passes it from
  vtable +0x34 of the vehicle; not identified). Mode 1 → 0 when throttle > 0.95 for 0.05 s (0x01026170).
  Step 2 of the update loop, `FUN_00900660`, applies a change: mode 0 makes the gearbox pair at vehicle
  +0x4c0 active (→ +0x4b8), mode 1 the one at +0x4c8 (`FUN_008f37f0`); both reload the per-gear tables
  (`FUN_009d7300` → `FUN_009d7110`, which copies the active box's `Gear_change_time` (+0x40) to gearbox
  +0x26c), and the switch to mode 0 clamps the gear index to the box's last gear (its gear count − 1, reverse
  being index 0). Mode 1 is the `CruisingGearbox` (it is the box entered at part throttle, and
  `FUN_009dc3c0`'s kick-down time reads it): its own ratios, `Gear_up_RPM`, `Gear_up_speed`, `Gear_down_RPM`
  and change time, and its top gear is not locked on the 911. `FUN_009dc3c0`: shift time = (change time +
  gearbox +0xb8) × (1 + 0.15 × random), where the change time is the cruising box's while in mode 0 and
  +0x144 < `.bss` 0x011871e0 (just after a kick-down), else +0x26c (the active box's). The upshift rules of
  state 5 are unchanged: they read the active box. `FUN_009d17d0` (launch) closes the clutch only when
  max(throttle, clutch, rpm / gearbox +0x130) ≥ 0.95; +0x130 (the gear's `Gear_up_RPM` ± 50) is refreshed by
  `FUN_009dc4e0` from the standstill and in-gear states, not on a mode switch.
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
  0x01186e60 = −2 mph (read live 2026-10-06; no absolute value: only rolling backwards faster than 2 mph), input = brake pedal (1 with the handbrake mask), raised to the throttle when
  the gear index is not 0; otherwise 0; then max'ed with the override +0x1c10. With the handbrake mask and
  |speed| < 1 it also releases the pedal brake. `FUN_009b1180` reads it: the rigid body (vehicle +0x1760) gets
  Δv = −sign(+0x1620) × `99407bbc` × input × dt along its forward axis (body +0x20) added to its velocity
  (+0xb0) every tick, with no clamp at zero (`FUN_009073b0` stores it, or zero above the body's speed cap
  +0xbc). The lock mask +0x1b90 is OR'ed into the per-wheel "brake engaged" mask passed to `FUN_009cdfd0`.
- **Downshift gate** (2026-10-06): vehicle +0x1ed0 is the **time on the ground**: `FUN_009ae840` adds dt
  while any wheel has contact and resets it while all four are in the air and the body moves faster than `.bss` 0x01186630 = 1.8 m/s (air time
  +0x1630 counts then). So `FUN_009d93b0` does not downshift until the car has been down for 0x01187580 = 1 s (read live).
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
- **Anti-roll bars** per axle, built when the vehicle is (`FUN_008fda70` → `FUN_00933a60`, twice: the front
  pair of wheel objects at vehicle +0xa08 / +0xbd8, the rear pair at +0xda8 / +0xf78, with the chassis body at
  vehicle +0x10). `FUN_00933a60` takes a constraint element of type 2 from the world's element pools (registered
  in `FUN_009333b0` with 0x28 extra bytes; the springs are type 0, `FUN_00933930`) and stores the two bodies'
  ids (+0x60 of each), an index (+0xf0 of the third object), then per lane the stiffness k × 60 (`.bss`
  0x01185d70) at element +0x20 and the damping 2 (`.rdata` 0x010245d0) × ζ × √(k × I) at +0x30, k the unscaled
  data stiffness. The inputs: ζ = `Front_roll_damping` / `Rear_roll_damping` (`BodyMovement` +0x68 / +0x7c,
  1.2), I = the reciprocal (rcpps with two Newton steps) of lane z of the chassis body's +0xe0 vector (its inverse
  inertia, so the roll inertia about the forward axis; inferred), k = the vehicle's slots +0x7fc / +0x800, whose
  writer was not found; that they hold `782f05c1` / `2bb0f6a0` (+0x64 / +0x78, the field before each roll
  damping) is from the field order. So the bar's damping ratio against its own stiffness is ζ / √60 ≈ 0.15, not
  ζ. The per-tick solve that reads the element was not found: no decompiled function reads +0x20 and +0x30 of a
  type-2 element; the island step `FUN_0093f5a0` calls `FUN_00932af0` (1104 bytes, no callees), which the dump
  has no body for and is the likeliest candidate (with the spring dampers, also unfound). So the bar's input
  (compression difference, body roll against the axle) and its integration form are not known.
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
- **Anti-roll scale** 60 (`.bss` 0x01185d70, read live) on the stiffness only (`FUN_00933a60`); the unit
  (N m per radian in our model) is inferred.
- **Strut travel**: searched for the joint built from `WheelSuspensionConstraint` (`Angle` 5, `7fc0dc41` 0.09 /
  0.08, `447b4e1c` / `668c92ac` −1.5, `c2a87807` 1.5, `b867a203` 0.23, `b1c680d5`): no reader of these offsets
  turned up in the vehicle build (`FUN_009b4e60` and its callees) or in `009A`–`009D` (searched `+ 0x14` / `+ 0x24`
  reads off a resource); `7fc0dc41` (0.09 front, 0.08 rear on the 911) is the likeliest travel. Open.
- **Strut end stop** (`FUN_009c5a90`, the wheel constraint's contact pass, and `FUN_009c2670`, its normal
  impulse pass). The constraint (`FUN_009c24b0`: +0x460 a point on the chassis, the strut's end; +0x470 a slack;
  +0x490 a velocity; flag +0x4ac from the builder) keeps a list of contact records (0x40 bytes: normal, point,
  accumulated impulse). `FUN_009c5a90` predicts each contact's penetration at the end of the tick (points plus
  velocity × dt) and pushes the wheel out along the normal by it; with +0x4ac set it measures the same from the
  strut's end point on the chassis, and where that is positive beyond `min(+0x470, .bss 0x01186bf0)` it stores the
  excess share in +0x474 (clamped 0..1, at least × `.bss 0x01186b10`) and sets +0x4ae. `FUN_009c2670` then
  applies the non-negative normal impulse (target from the record's bias and n · +0x490, accumulated, clamped
  at 0) to the wheel, and while +0x4ae is set also to the chassis at the strut point through the chassis's
  inverse mass and inertia, scaled by +0x474. So the end of the strut is a hard contact on the chassis, not a
  spring: a landing cannot drive the body into the ground. Ported as `strut_stops` (handling.md item 5).
- **Island step and sub-steps** (2026-10-07, from the disassembly in the live snapshots). `FUN_0093f5a0` calls
  each pass once; the iterations live in two callees, both counted by the world (constructor `FUN_009330d0`,
  same values live): `FUN_00932190` loops world +0x460 = 2 times (joints `FUN_009c62e0` / `FUN_009c5840`, the
  wheel constraints' contact pass `FUN_009c5a90`, contacts `FUN_009be060`, `FUN_00928cd0`), and `FUN_0093f200`
  loops world +0x464 = 2 times at dt / 2 (joints `FUN_009c4e10` / `FUN_009c1fb0`, contacts `FUN_009be420`,
  `FUN_00928fb0`, the wheel constraints' impulse pass `FUN_009c2670`, the force elements `FUN_00932f90`, then
  gravity and velocity integration `FUN_009282d0`). Positions appear to be integrated once afterwards
  (`FUN_009328a0`; inferred).
- **`FUN_00932af0` is the island sleep pass**, not a force: the island sleeps once every body stays under 0.4 m/s
  (world +0x46c) and 0.3 rad/s (+0x470) and every wheel's spin under 1 rad/s (+0x474) for 2 s (+0x468) of
  accumulated dt; then it zeroes the bodies' velocities and accumulators, sets body +0xf4 bit 0 and wheel record
  +0x64, and flags the island's elements (+0xc bit 0) so the solver skips them (parked cars in the snapshots).
- **Force elements** (`FUN_00932f90`, three island lists): +0x68 dampers (`FUN_0092f160`, then `FUN_0092f510`,
  likely the springs), +0x70 anti-roll bars (`FUN_0092ee70`). Elements with +0xc bit 0 set are skipped.
- **Anti-roll bar** (`FUN_0092ee70`, read from the disassembly; element from `FUN_00933a60`: wheel record
  indices at +4 / +8 (× 0x80), chassis at int16 +0x10 (× 0x100), k × 60 at +0x20 (`.bss` 0x01185d70 = 60,
  four lanes, live), damping at +0x30). Both wheel centres (wheel record +0x00, world space) are taken into the
  chassis's frame (rows +0x00/+0x10/+0x20, position +0x30); with Δx, Δy their differences across and up,
  θ = atan(Δy / Δx) (CRT `atanf` `FUN_00edc7b2`). The roll rate is the chassis's angular velocity (+0xc0) along
  its forward row: absolute, not relative to the axle (the subtracted term is a zeroed register, 0x92ee86 /
  0x92eec5). F = (k θ − c ω_roll) / Δx (0x92f00e..0x92f031). `FUN_009275b0` caps each wheel's share at
  |Δy| / (4 W dt²), W = the chassis's inverse effective mass along its up row at the wheel centre
  (`FUN_009e8230`) plus the wheel's 1/m (only if wheel record +0x0c ≠ 1; it was 1 in every snapshot), with the
  full tick's dt (ctx +0x1e0 = 1/dt; the 4 is 2.0 at 0x010245d0 twice). `FUN_00909dd0` adds +F_A along the up
  row at wheel A's centre and −F_B at wheel B's to the chassis accumulators, and `FUN_00907840` integrates them
  at once with dt / 2; the wheels get their share only under the same +0x0c mask. Net torque about the forward
  axis k θ − c ω_roll, towards the axle line. Ported as `anti_roll` in drive.rs.
- **Damper** (`FUN_0092f160`, from its pseudo-C, whose selection logic is clean): chassis at element +4, wheel record at
  +8, a chassis-local anchor at +0x20. n = the unit vector from the anchor (in world space) to the wheel centre;
  the relative velocity is the wheel's velocity along n minus the chassis's at the anchor. Positive (the wheel
  moving away from the mount: the strut extending, rebound) takes element +0x18 (`0c5f89b3` × c from
  `FUN_009b29b0`), otherwise +0x2c (c); the impulse is −coefficient × relative velocity × 0.5 (0x010245b0) ×
  the sub-step's scale, capped through the same effective mass (`FUN_009e8230`) so the relative velocity cannot
  change sign, applied to the chassis (`FUN_0097e440`) and to the wheel only under the +0x0c mask. The spring is
  likely `FUN_0092f510` (the same list, elements +0x10 / +0x14 / +0x20..+0x3c), not read.
- Unknown: the spring element `FUN_0092f510`, the strut travel (+0x460 is set
  by the builder's caller; not traced), the stop's slack (+0x470, `.bss` 0x01186bf0, 0x01186b10; not touched by
  the sleep pass or the bar, whose context offsets +0x210 / +0x470 only coincide), what wheel record +0x0c is and
  whether it ever changes (e.g. in the air).

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
- **Surface factors** (`FUN_009d8bd0` per wheel with a contact, per tick, `this` = vehicle +0x17e0; called from
  `FUN_009aaf30` ← `FUN_009b4d70` ← `FUN_00904260`; read in full 2026-10-09): the collision tag's surface (bits
  13–19) → `Surfaces.Surfaces[i]` (`SurfaceType`) → the weather block (wet when the surface system's state is ≥ 2
  and the game car's byte +0x1c17 is clear, else dry) → the contact kind (`FUN_0054a760`: Normal; Rims when the
  wheel's +0x1c1 is set, Flats when +0x1c2; the wet kinds fall back to the dry ones, null to `Default_surface`) →
  `SurfacePhysics` → the tyre class's `TyreSpecificOverrides` (`FUN_0054d750`; class = game car +0x1a88, 2 at
  creation in `FUN_00586810`, rewritten from upgrade data by `FUN_00589a60` / `FUN_00589b00`): class 0
  `OffRoadOverrides` (+0x28), 1 `08b66463` (+0x0c), 2 `RoadOverrides` (+0x44, also any other value), 3
  `SportOverrides` (+0x60), 4 `TrackOverrides` (+0x7c). The record it fills: [0] `7585f9cb` (slide-drag scale),
  [1] `Surface_roughness.8193fd9b`, [2] `Surface_roughness.7cb58933` × 0.001 (`.rdata` 0x00f8bb34), [3]
  `5c2a9f18` (rolling resistance), [4] the physics' `UniqueId`, [5] `Surface_Grip`, [6] `Surface_speed` (not read
  by the car code). Then per wheel, in order:
  1. **Bumps** (`FUN_009d77b0` → `FUN_009d4650`), only when the caller's flag is set (`FUN_00904260`: the car has
     a player record, game car +0x1a80 via `FUN_008f5420`, whose +0x9c is 0; read as the human player's car):
     with f = [1] (per metre), A = [2] (m), P / N the contact point and normal, n(x) = Perlin's improved noise
     (`FUN_009e31e0`: Ken Perlin's 2002 reference, fade 6t⁵ − 15t⁴ + 10t³, the reference permutation twice at
     `.data` 0x010cfcb0), the contact becomes P' = P + N × A × n(f P); two tangents t1 = normalize(N × X),
     t2 = normalize(N × t1) are sampled 0.03 m away (`.bss` 0x01186ea0, read in a snapshot) the same way and the
     normal becomes normalize((Q1' − P') × (Q2' − P')). The results go to vehicle +0x19d0 + 4i (A × n),
     +0x19f0 + 16i (point) and +0x1a30 + 16i (normal); `FUN_009aaf30` writes them into the wheel's contact
     (`FUN_009df850`) and the wheel constraint's contact record (+0x10 point, +0x20 normal), so the suspension
     and the tyre work on the bumped ground. Without the flag the contact is unchanged. Values: asphalt f 25,
     A 0.12 mm; paving (406000) 0.3, 12 mm; dirt (283003) 0.5, 20 mm; grass (283005) 0.1, 80 mm; 681641 1.25,
     50 mm; rims 4, 5 mm.
  2. **Grip** (`FUN_009d4460` on the wheel's 0x7c-byte record at +0x17e0 + 0x7c i): the car's
     `VehicleSurfaceProfile` link for the `UniqueId` (`FUN_009d4b70`, first link listing it). On a change of
     surface (`UniqueId` ≠ record +0x54) `FUN_009ceb00` ends a fade of the surface entered, then puts the surface
     left into a free fade slot (three; with none free, the one whose lower grip is highest): start = its
     (lateral, longitudinal) grip, duration = (its link grip − the lower of the two) × 3 (`.rdata` 0x01025dcc) ×
     the time spent on it × 0.5 (0x00f8a810); `FUN_009cec40` then sets lateral = lerp(`Surface_Grip`, link
     `f43804ae`, `fad9b7b5` front / `5567a3e0` rear), longitudinal the same with `f448cca5` / `42829f58` (front =
     wheels 0 and 1), the link grip as the fades' target (1 without a link) and the time to 0. Every tick each
     fade runs from its start towards the **current** link grip over its duration (then frees, at 1), the time on
     the surface grows (capped at 2 s, 0x01026028), and `FUN_009cece0` sets the effective grip (+0x70 lateral,
     +0x74 longitudinal) = the lower of the surface's and every fade's. The tyre caps read the effective values
     (`FUN_009ceda0` / `FUN_009cedc0` from `FUN_00905910`). Grass → asphalt on the 911 after 2 s on grass: 0.835
     back to 1 over (1.3 − 0.835) × 3 × 1 = 1.4 s; asphalt → grass: no fade (link grip 1 = grip).
  3. **Rolling resistance and slide drag** (`FUN_009d4a10`, below).
  Asphalt (`SurfacePhysics` 283001, the dark-grey road type): `Surface_Grip` 1 and link grip 1 on every car, so 1;
  the game's `Default_surface` (250315) is 1 too. Grass (283005) on the 911: 0.7 blended to 0.835 / 0.88 (front)
  and 0.82 / 0.88 (rear) by its link (grip 1.3); dirt (283003, link grip 1.25): 0.875 / 0.90.
- **Drift surface factor** (`FUN_009cede0`, `this` = vehicle +0x17e0): 0.25 (0x00fa8830) × the sum of the four
  wheels' effective lateral grip; the drift's yaw torque and side force are multiplied by it.
- Another reader of the front wheels' effective lateral grip: `FUN_00905580` → `FUN_009d5270` (two lanes into a
  force term at its +0x10 / +0x20, purpose not read).
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
`FUN_00904da0` → `FUN_009d6e30`, after the dirt build-up `FUN_009d19c0`), which stores them at +0xd0 / +0xd4 /
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

Read 2026-10-09 (research log "engine sound stuck in gear"); ported in `mw_assets::engine_driver`. Per vehicle tick
`FUN_009b3030` runs the gearbox, the pop rule (`FUN_009b20b0`, next section), the normalised RPM
(`FUN_009ae930` → `FUN_009aea50`) and the engine load (`FUN_009b1f70`), then clears four effect lists
(`FUN_00926270` × 4). The sound update (`FUN_005911d0`, per frame) reads those values, runs the sequences and
posts their effects back for the next tick.

- **Audible RPM range** (`GetPlayerAudibleRPM` / `GetAudibleRPM`, `FUN_0083eb60`): `min + (max − min) × +0x1f10`
  with min / max at `RaceCarEngineSound` +0x1c0 / +0x1c4. The constructor sets 1000 / 8000 (`.rdata`
  0x00f95dc0, 0x00fadebc), but **`FUN_00575340`, run when the sound starts (state 3, called from
  `00580000.c` near line 7653), replaces them with the lowest `StartTime` and the highest `EndTime` of the
  `GinsuSequenceItem`s (type 0x73417) in `EngineSound2.Sequences`** (+0x68, count +0x102; item `EndTime` +0x18,
  `StartTime` +0x1c). So the whole rev range plays across the car's recordings: 1790..7500 on the 911, 1750..6000
  on the XKR; over the 73 engine banks the top runs 4400..10000 (median 6400). The earlier reading (fixed
  1000..8000) missed this function. Confirmed on `ref/xkr_launch`: the recorded pitch follows `1750 + 4250 ×
  +0x1f10` (research log).
- **Normalised RPM** (vehicle +0x1f10): `x = max((rpm − Idle_RPM) / (Max_RPM − Idle_RPM) − 0.05, 0) / 0.95`
  (`.bss` 0x01186680 = 0.05, read live; the reciprocal is Newton-refined), times `(tuning +0xd0)^(4m)` with `m`
  the scaled RMS of vehicle +0x19d0 (`FUN_009cdd40`, × `.bss` 0x01187220: an engine-damage term, 1 on an
  undamaged car; not ported), clamped 0..1; then `FUN_009aea50` raises it to `2 − a` (`.rdata` 0x01025080 = 2;
  `a` = `EngineSound2` +0xb0 `cb44d82d` → +0xac `a33d84fe` by the load +0x1f20, or +0xb4 `913bb900` with flag
  +0x1f45), adds the property-1 effect list (+0x1e78, `FUN_00926330`) and clamps to 0..1 (`.rdata` 0x010245c0).
  Vehicle +0x1f00 = `Idle_RPM + (Max_RPM − Idle_RPM) × +0x1f10` (the pop rule's RPM). Both run only while the
  world flag `FUN_004b1a70` is set. Snapshot check (XKR, `snap2`): 4040.6 rpm → 0.5017 against 0.5040 computed
  with `a` = 1.
- **Engine load** (`FUN_009b1f70`, `GetPlayerEngineLoad` = +0x1f20, state +0x1f24): `h` = the engine's lagged
  throttle (+0x1d14 = engine +0x24; max'ed with an override list on the engine object, `FUN_009262a0`, writer
  not found), `s = (clamp01(speed (+0x1620) / +0x7e4) × Engine.937338e7 (+0x104) × clutch (+0x1e20))²`
  (`FUN_009aeba0`; +0x7e4 = 174 on the XKR = its `Top_speed_MPH`, read live), `state += clamp(60 × EngineSound2.
  99d4b91e × dt, 0, 1) × (s + (1 − s) h − state)` (`.bss` 0x011865e0 = 60, read live: 0.5 per tick), output =
  max(state × Π property-3 list (+0x1e40, `FUN_009262f0`), max of the property-4 list (+0x1e5c,
  `FUN_0092ae80`)). There is no peak hold (the "last six values" of the earlier reading are these lists). The
  load is computed before the engine's tick, so it sees the previous tick's lagged throttle.
- **Effect properties** (`RaceCarEffectSequenceItem.Automated_Property`, posted as message 0x2a and applied by
  `FUN_008f5490`; jump table at 0x008f55a0): 0 multiplies vehicle +0x2e70; 1 → list +0x1e78 (added to the
  normalised RPM); 3 → +0x1e40 (multiplies the load); 4 → +0x1e5c (floor under the load); 5 → +0x1e94 (reader
  not found); 6 / 7 set the engine-start flags +0x1be3 / +0x1be5 (6 also resets the gearbox state). In the 911's
  data: the upshift sequences cut the load over the shift (×0.94 → 1 → 0.83 → 0.13 at 0.9 → 1, normalised time
  0..1), kick the RPM +0.2 at engagement (1..1.25) and shunt it (LFO, ±0.023..±0.15 and load ×0.8..1.2, 1..2);
  the downshift sequences hold the load up (property 4, 0..4); the speed sequence scales load and RPM by
  `GetPlayerSuspensionLoad` (+0x1f28), `GetPlayerTractionLoad` (`FUN_009ab130`) and `Pitch`; in-air, burnout and
  drift sequences lower the RPM (−0.4..−0.8).
- **Gear-change sequences** (`FUN_0058ca50`): gearbox +0xd0 (vehicle +0x1b40) is set on the tick an upshift starts
  (`FUN_009d1560`), +0xd4 on a downshift (`FUN_009d16f0`). The sound then stops the previous gear-change instance
  (`FUN_00a44d50`) and starts the next sequence of the list, round robin with one index for all lists
  (+0x1b4 = (+0x1b4 + 1) mod count): `EngineSound2` +0x50 `83ea3a92` (upshift, race gearbox, count +0xfc), +0x54
  `9492327c` (upshift, cruising gearbox +0x1bdc = 1, count +0xfb), +0x48 `ce819b08` (downshift, race, +0xfa),
  +0x4c `c342c096` (downshift, cruising, +0xf9). Their axis, `GetPlayerNormalisedGearChangeTimeElapsed`
  (`FUN_00911650`), is gearbox +0xb4 / +0xb0, at least 0: +0xb4 counts dt every tick (`FUN_009da340`) and is set
  to 0 when a shift starts, +0xb0 is that shift's time (`FUN_009dc3c0`, × 0.5 for a downshift). So 0..1 is the
  shift itself (declutch to clutch-in) and the clunk plays as the clutch opens, not when the gear index changes.
- **Rev limiter** (`FUN_0058cdb0`): only when the gear index is the gearbox's last or second to last
  (`FUN_00911690`: +0x1d70 against the gear count − 1 / − 2) and +0x1f10 > 0.99 (`.rdata` 0x00fad930). Each
  cycle of its LFO (+0xd4 phase / 2π) restarts the `c62d11ac` sequence and draws the cycle's rate (+0xe4 =
  `6e6be213`..`9529a29e`, 6..7 on the 911), depth (+0xe8 = `18adb27c`..`f81e0914`) and floor (+0xec =
  `a1049379`..`34c8d45a`), and posts property 1 = (LFO − 1) × depth and property 3 = LFO' × (1 − floor) + floor;
  the LFO's own depth +0xd8 is set to 0 at the start and no other writer was found. `RevLimitGain` (the limiter
  wave's gain) is `LAB_005a22e0`, not decompiled.
- **LFOs** (`FUN_00586330`): `RaceCarEngineSound` +0x11c `Lfo`, +0x120 `ShuntLfo`, +0x128 `TurboLfo` = 0.5 ×
  sin(phase) × depth (`FUN_00aabc30`), +0x124 `SkidLfo` (`FUN_00aabc60`), +0x12c `Jitter` = +0x80 + +0x84.
  `LfoSequenceItem` automates properties 0 / 1 / 2 (8..20, 0.3..0.6, 5..20 in the gear changes); which object they
  write was not found (read: 0 = rate in Hz, 1 = depth).
- **Ported** (`mw_assets::engine_driver`, `mw_handling::drivetrain` `shift_elapsed` / `shift_time` /
  `shifts_started`): all of the above except the damage term, the override list, property 0 / 5 / 6 / 7, the
  limiter's RPM / load dip and the effect items bound to suspension load, traction load or pitch. Model
  decisions: the LFO items set the rate (property 0) and depth (1) of `Lfo`, or of `ShuntLfo` when the sequence's
  own effects read it, depth 0 when none is active, property 2 ignored; the limiter's cycle rate is in cycles per
  second; effect items with a binding we do not supply are left out (not read as 0); the effects reach the next
  update (the game: the next tick); `RevLimitGain` = 1 while limiting; the round-robin index starts at 0; offline
  traces without the gear-change clock use 0.15 s per shift.

The harmonic multipliers come from `EngineSound2` (research log "engine sound from the game code"). The Distortion
and SubHarmoniser plug-ins fall in undecompiled gaps of the dump.

## Exhaust pops and backfires (2026-10-09)

- **`FUN_009b20b0`** (run by the engine-sound update `FUN_009b3030` every tick, after the gearbox `FUN_009da600`
  and before `FUN_009aea50` refreshes the audible RPM, so it sees the previous tick's values). The audible RPM
  `A` (vehicle +0x1f00) is `Idle_RPM + (Max_RPM − Idle_RPM) × f`, `f` (+0x1f10) the sound's RPM fraction raised
  to `2 − a` (`FUN_009ae930` / `FUN_009aea50`, the `EngineSound2` curve terms; research log "engine sound from
  the game code"). The car's own `Idle_RPM` / `Max_RPM` (Engine +0x2c / +0x38 through vehicle +0x4b0 /
  +0x1d30 → +0x240, both the `Engine` struct; read live: 3609 = 1000 + 5200 × 0.5017), not a fixed 1000..8000.
  1. `drop = max(A[t−2] − A[t−1], 0)` (+0x1eb0 keeps the previous `A`).
  2. If the gearbox is past its start-up states (+0x1bd8 not 0 / 1), `A > 4000` (`.rdata` 0x010251e0), the
     cruising gearbox is not in use (+0x1bdc ≠ 1) and a flag +0x1f45 is clear (set from `FUN_0069ffa0` on the
     car's slot; not identified): `x = 1 − drop / (Max_RPM − Idle_RPM) × r × 10` (0x010250c0; `r` from
     `FUN_00d5af50`, uniform 0..1 on the RNG at `.bss` 0x01186618). `Engine.Sound.38d82e04` (Engine +0x70) > x
     → **backfire** (+0x1f42 = 1); else `Engine.Sound.99a16939` (+0x7c) > x → **pop** (+0x1f41 = 1).
  3. A third flag (+0x1f43, `9305b427` at +0x74 against `1 − r`, gated by `61e24fd9` (+0x84) × a rev term
     (`.bss` 0x01186660 = 0.4) against the gearbox throttle below 0.5) has no reader in the dump; not ported.
  4. `GetPopProbability` (+0x1eb4, the getter `FUN_0059ee50` registered on `RaceCarEntity`, `FUN_0090d640`) =
     `clamp(f / 0.8, 0, 1) × (1 − T)²` with `T` the gearbox throttle (+0x1aa0) and 0.8 from `.bss` 0x01186810
     (read live).
  The data: backfire 0.025–0.65 over the cars (0.1 on the 911, 0.65 on the BMW M3 GTR, the McLarens and the SWAT
  van), pop 0–1 (0.9 on the 911). With our drivetrain the 911's upshifts drop at most 9 % of the rev range in a
  tick, so it pops on upshifts but almost never backfires; the M3 GTR needs only 3.5 %.
- **Sounds** (`FUN_005911d0` → `FUN_0058ccb0`, with the car's `EngineSound2` from the entity +0x1c / +0x20
  → +0x48): +0x1f42 starts `Backfire_Sequence` (`EngineSound2` +0x0c) and +0x1f41 `Pop_Sequence` (+0x2c) through
  `FUN_005866f0`, each only while its previous instance (slots +0x94 / +0x90, cleared by `FUN_00a422d0` when
  ended) is gone. `Ignition_Sequence` (+0x1c) starts when the gearbox leaves its start-up states
  (`FUN_0058c930`); `c62d11ac` (+0x34) is the rev limiter's (`FUN_0058cdb0`). `Gurgle_Sequence` (+0x14) and
  `Pop_Sequence` are bound to `GetPopProbability` (items at 0.6–1 / 0.75–1); who starts the gurgle was not found.
- **Effect group bits** (`FUN_0057a5e0`, per car, entity +0x3c8): bit 14 for two updates after a backfire
  (entity +0x3f4 = 2, counted down when the world flag `FUN_00897b70` is up), bit 8 while the nitrous input
  (+0x1944) is above 0 and the car is the one `FUN_006a0020` / `FUN_0069ff20` accept, bit 9 instead while the
  nitrous mode byte +0x195b is 1, bit 20 while it is 3 (with `FUN_006a15c0(…, 13) == 2` and `FUN_008f5350`),
  bit 19 while the engine runs (`FUN_00572c20`: gearbox state not 0 / 1), bits 0 / 4 / 6 / 7 (`0xd1`) for the
  lights, 1 / 2 brake and reverse, 3 (+0x1c27), 15 (`0x80c7` group). `FUN_005b2740` adds bit 12 when the car is
  wrecked and 13 for a non-zero vector of the player state; the coronas, lights and spot effects test
  `1 << Group` against it.

## Groundcover drawing (2026-10-09)

- **Job**: `FUN_0089ffe0` (called from `FUN_008ad290` with the list at `+0x2e94` of its object) copies the
  collection list (`FUN_008d6900`: 64 entries of `{collection, offset}`, count at +0x200) and the camera into a
  job named "Groundcover" whose function is `FUN_00eb7f40` → `FUN_00eb7f60` → `FUN_00eb9f50`.
- **`FUN_00eb9f50`**: for each of the 64 collection slots, for each layer (3, `collection + 0x28`) whose flag
  `DAT_010c6f78[layer]` is set, walks the layer's box tree (`FUN_00eb9d70`, nodes 0x60 bytes, groups 0x40) and
  calls the vertex builder `FUN_00eb9500` (or its twin `FUN_00eb8390` under a flag set) per group. The
  builder's distance tests and sprite vertices are in formats.md (GroundcoverCollection).
- **`FUN_008c9aa0`** (the draw loop over the job's records, stride 0x10, count at +0x12c0, sorted first by
  `FUN_008dee30`): binds the material's 16 texture regions (`material + 0x10`) to constant slot 0x80
  (`FUN_004ad9f0`) when the material changes, and when the record's byte +0xC (the group's size exponent e)
  changes writes `(float)(1 << e)` into the four floats of slot 0x7f (`GroundcoverConstants`, through
  `FUN_00a5d660`), then draws (`FUN_00a5cb70`, 0x90). So the sprite size is BINORMAL × 2^e metres: the
  e = 6 / 8 groups are 10–60 m tree cards over the hills, not 1 m tufts.
- **Not traced**: who fills the 64-slot list at `+0x2e94` (which track units' collections are resident). The
  data fit "the player's zone and its `ZoneList` neighbours": at most 63 distinct collections over every
  zone's neighbourhood. The runtime uses that rule (`groundcover.rs`, model decision).

## Car spot effects (`FUN_005b28d0`, 2026-10-09)

- Per `VehicleVfxBehaviour.SpotEffect` (0x20 bytes: effect handle +0x0c, `LocatorGroup` +0x14, `Group` +0x18,
  `da6ac384` +0x1a, `Looping` +0x1c, `Mirror` +0x1d): the locator group comes from the car's
  `VehicleGraphicsSpec` locator table (`FUN_0057c4b0` → `FUN_0042b870`: `{count, ids, {n, locators, ?}}`, a
  linear search falling back to the first group; locators are a quaternion and a position, 0x20 bytes).
  `da6ac384` gates by the world flag +0x1b934 (`FUN_008979d0`): 0 always, 1 while clear, 2 while set. One
  effect instance per locator (twice with `Mirror`: the second pass negates the transform's X axis).
- An instance is created (`FUN_0049f7c0`) when none is alive and either `Looping` or the group bit rose since
  the last update (the previous mask is kept at +0x34); while alive its record gets the transform (vehicle ×
  locator), the velocity at the locator (`v + ω × (p − centre)`), the plane under the car and `IsEmitting` =
  the group bit. Groups 10–12 that switch off instead flip a "stop" bit in the record.
- `FUN_0049bc60` copies the record into the effect's named parameters: `Transform`, `Velocity` (+0x40),
  `PreCrashVelocity` (+0x50: a vector from vtable +0x4c), **`Intensity` = the velocity's w = 1.0**
  (`DAT_00f8a820`; three effects `0x24f27`, `0x24f28`, `0xd4fe4` scale it), `IsEmitting` = 1,
  `ColourOverride` (+0x60), `ImpactClosingVelocity` (+0x70: entity +0x1bdc), `MovementSinceLastFrame` (+0x74: a
  float from vtable +0x48), `VehicleParams` (+0x80..+0xac: the plane and two more vectors).

## Bear particle runtime (2026-10-09)

- Action kinds index a table at `.data` 0x010c2d48 (`FUN_0043f8a0`): `{name, compile, run, info}` per kind,
  names in `.rdata` from 0x00f9d664 (`EmitterConePlus` 0x00, `EmitterCollisionMesh` 0x01, ..., `EmitterWheel`
  0x05, `EmitterWheelContactPlus` 0x07, `InitialisePosOffset` 0x14,
  `InitialisePosVelToWorldSpaceWithLocalTransform` 0x15, `InitialiseVelocityInheritance` 0x16,
  `ParametricPosVelToWorldSpace` 0x1A / `…WithLocalTransform` 0x1B, `ParametricSetColourGradient` 0x1C,
  `RenderSprite` 0x24, `RenderSpriteSubUV` 0x27, `RenderSpriteUVDistortion` 0x28, `UpdateBindings` 0x33,
  `UpdateBindings_Wheel` 0x34, `UpdateParametricFadeOverDistance` 0x38, `UpdatePosVel` 0x3D,
  `UpdatePosVelWithBouncePlane` 0x3E, `UpdateWheelBindings` 0x42, ... `RenderSplatter` 0x4D; full list in
  formats.md). Several run functions are outside the dump's functions (`disas.py` on the snapshots).
- **`UpdateBindings`** (compile `FUN_0045fc60`, run `FUN_0045ffd0`): four curves, each over an input chosen by a
  source byte (`FUN_00448620`), plus an offset, into the layer's binding slots 1–4, then four combine steps
  (`FUN_004492a0`) into slots 5–8. The inputs include the layer's clock, the velocity's length, the distance
  to the camera, `Intensity`, `ImpactClosingVelocity`, `MovementSinceLastFrame` and `BearGlobalParameters`
  fields that `FUN_004a1470` fills every frame: +0x30 the camera position, +0x64 dt, +0x74 the **player car's
  engine load** (vehicle +0x1f20 through `FUN_0090d6d0`), +0x60 a speed × 0.44704, +0x7d = 0. The curves are
  `Curve5` bytes ÷ 255 (`FUN_0046af80`, `FUN_0046aee0`). Table: `mw_formats::bear::Bindings`.
- **`EmitterConePlus`** (compile `FUN_0044ccc0`, run `FUN_0044cf30`): emits only with `IsEmitting` set, the
  layer time within a window, the squared camera distance within a window (90 000 = 300 m on the car effects)
  and the velocity's length within a window (0–6 m/s on the exhaust smoke); the count is
  `(rate + per_metre × |Velocity|) × dt × slot[b0]` × the distance fade, accumulated from `burst + 0.9999`
  and rounded; speeds × `slot[b2]`, cone angles × `slot[b3]` (bytes b0..b3 at +0x70). The cone's angle maths is
  a vectorised sincos on symmetric randoms (−0.5..0.5) further down `FUN_0044cf30`; not decoded.
- **`ParametricSetColourGradient`** (compile `FUN_00449030`, run `FUN_004553b0`): `rgb = c.rgb × c.w`,
  `c = rows[0] + Σ clamp(u · A_k + B_k) · rows[k+1]` with `u = frac(age · rate)`; alpha from the `Curve5`
  (converted to the same ramp form by `FUN_0046b090`) × a binding slot. The runtime read it as a polynomial in
  the age with the fourth column in the alpha until now.
- `InitialisePosOffset` (0x14) keeps a particle in the emitter's space; `ParametricPosVelToWorldSpace` (0x1A /
  0x1B) places it with the current `Transform` when it is drawn, so the exhaust flames stay on the pipes.

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
   the view's vec4 at +0x15d0 (override, `FUN_008e3e30`). **Vignette fields** (live reads 2026-10-06, 20
   dumps): +0xa980 = (inner.rgb × s, k), +0xa990 = ((outer − inner).rgb × s, 1 − k), +0xa9a0 = (angle 0, …),
   +0xa9b0 = (centre.xy, `Scale`.xy × k), with inner / outer / centre / Scale the `PostFxKeyframe.Vignette`
   fields `5e9c4fa6` / `5f000214` / `19f17784` / `Scale` (A / D = outer − inner to 5 digits in every dump).
   `FUN_008c9e40` passes +0xa9b0, +0xa9a0 and +0xa990 to the bloom set-up `FUN_008eaaf0`: they are the add
   pass's `kVignetteCentreXyScaleXy`, `kVignetteAngle` and `kVignetteOuterRgbPlusAdd` (its .w is the weight
   offset). So v = smoothstep(saturate(k·|p₀| + 1 − k)) with p₀ the unscaled offset: the weight rises from
   0 at |p₀| = 1 − 1/k to 1 at |p₀| = 1. k = 1.8245 in every dump (also under another PostFX state, which
   changed centre and Scale), so it is not derived from a blended field; `Sharpness` (0.34) is the likely
   source but no formula was found. s drifts 1.0025…1.0385 over the run (2.81 in the dump taken during a
   state change). Ported in `tonemap.rs` with k = 1.8245 as a measured constant and s = 1 (model
   decision). No instruction in the renderer code (0x880000…0x900000 of the snapshot) stores to
   +0xa980 / +0xa990 with a literal displacement: the writer goes through another base and is still unknown.
   **Bloom copy**: the copy at 0x008ccbdc (inside `FUN_008c9e40`) sets bit 1 of +0xaa9c and moves 13 words
   from the blended set (+0x80…+0xb0 of its source) to +0xac74…+0xaca4 with x87 load/store pairs, so integer
   GeneSys fields arrive as raw bit patterns: live, the words at +0xac80, +0xac88, +0xac8c, +0xac94 are the
   integers 4, 5, 5, 1 (falling to 1, 1, 1, 0 through a transition), while +0xac84 (B.w) = 0.595 → 0.510
   and +0xac98…+0xaca0 (the three set-up arguments) = (0.0083, 0.927, 1.043) drifting to (0.0073, 0.815,
   1.109), and the colour drifts (0.342, 0.482, 0.585) → (0.417, 0.495, 0.557).
   **Where the block comes from** (2026-10-07, disassembly of the call sites): `FUN_008ce250` passes
   `&local_2a0` as `FUN_008c9e40`'s last argument, so the copy source (+0x80) is its `local_220`, which
   `FUN_008e3b50` (this = the PostFX manager) fills from manager +0x1300…+0x1330 (the decompiler's argument
   list for that call is shifted by one; the pushes settle it). `FUN_008ecf80` writes +0x1300…: zeroes it,
   blends the PostFX keyframes / states into it (`FUN_008eccb0` → `FUN_008e9610`, words in the
   `FUN_008e51b0` order), then, on the first state when the **environment override flag** +0x15e0 is set,
   overwrites words 0–2 with +0x15f0 (colour), 3 and 8 with +0x1610 / +0x1614, 4–6 with +0x1620…+0x1628 and
   9–12 with +0x1600…+0x160c; word 7 (`3efef892`, 1.0) stays from the PostFX blend. +0x15d0…+0x1628 are set by
   `FUN_008e3e30`, called from `FUN_008c2080` with outputs of `FUN_008c0f30`, which copies them from the
   blended environment state (+0x24d0 vignette multiplier, +0x24e0 colour, +0x24f0, +0x2500…+0x2510).
   So in free roam the bloom is the **time-blended EnvironmentKeyframe.Camera**, not a PostFX keyframe:
   word 0–2 = `BloomColour` (`96a4b35b`; the first live read (0.292, 0.474, 0.604) is exactly the noon
   keyframes' value, 29 keyframes carry it), word 4 = B.w = `99436e68` (0.387…0.599 on the day keyframes;
   live 0.595 → 0.510), words 9–11 = `9dd29928` (0.009 by day = live) and 1, 1 (`2b7c4634` / `e59ff4ec` are 1
   by day; which is word 10 or 11 is not traced). The integer words 3, 5, 6, 8 (4, 5, 5, 1) come from the
   environment state's +0x24f0 group and from `FUN_008e3e30`'s sixth argument (lost by the decompiler); no
   float Camera field holds them, so they are runtime integers (likely pass or level counts), not data.
   The drift of the colour and words 9–11 was not reproduced by a lerp between two noon-timeline keyframes
   (colour fits ≈ 25 % towards the orange (0.784, 0.565, 0.434) keyframes, B.w and words 10 / 11 do not):
   the environment blend has more inputs (regional timelines, weather) than one pair.
   **Level combine** (`FUN_008eaaf0` → `FUN_008e6f40`, add pass `FUN_008e6810`, program `0x010117c0`): three
   blurred levels, finest L1 to coarsest L3; weights a = word 11, b = word 10, c = word 9, each clamped to at
   least `DAT_00f9d474`. The add program blurs its t0 vertically with the same 5-tap kernel (0.204164,
   0.304005 ×2, 0.093913 ×2), × `kColorAddWeight.z`, plus t1 (the coarser result) × `.w`, and writes the
   vignette weight in alpha. Calls: C3 = blur(L3) (z = 1, no t1); C2 = (b·blur(L2) + a·C3) / (a + b);
   C1 = (c·blur(L1) + (a + b)·C2) / (a + b + c). With the day values (0.009, 1, 1) the finest level adds
   under 0.5 %: the bloom is the mean of the two coarse levels. The argument → z / w assignment follows from
   the first call (coarsest level must get z = 1) but the decompiled signature of `FUN_008e6810` is off by
   one, so it is inferred. Level resolutions and the integer words' roles are not traced.
   Our data: `CameraExposure.bloom_colour` / `bloom_intensity` / `bloom_weights` (mw-assets environment.rs),
   blended with the timeline like the exposure.
   **Bloom passes (ported 2026-10-07, `bloom.wgsl` from tonemap.rs, `MW_NO_BLOOM=1`).** `FUN_008e6f40` runs
   L1 = down(source) with the tonemap texture (`FUN_008e6470(L1, src, 1, tonemap)`, PS `0x010104c0`),
   L2 = down(L1), then `FUN_008e36d0` / `FUN_008e2f80` on L2 (not followed; likely the exposure meter's
   input), L3 = down(L2), then the three add calls coarse to fine. `FUN_008e6470` sets `kInvWidthHeight` =
   (1 / src width, 1 / src height, 2 / src width, 1 / src height); VS `0x0100fc50` puts the four taps at
   uv ± (1, 1) and uv ± (1, −1) source texels. Level encode: rgb × T.w / 128 summed over 4 taps (= the
   average × T.w / 32), M = ceil(min(max channel, 1) × 255) / 255, rgb / M saturated (so a channel clips at
   32 after the decode); `0x0100ffa0` decodes 4 taps × 32 and re-encodes. H blur (VS `0x010109c8`, PS
   `0x01010d38`): taps at x + {0, ±1.407333, ±3.294215} × `kScale`, decoded rgb × a × the weights (sum 32),
   ÷ T.w, alpha 1 (a float target). Add (VS `0x01011280`, PS `0x010117c0`): the same taps along y with the
   weights ÷ 32 on t0, × `kColorAddWeight.z`, + t1 (sampled at the same uv) × `.w`. The composite samples
   the finest combined level. Model decisions in the port: the level sizes 1/4, 1/8, 1/16 of the screen (the
   targets are passed in through `*(renderer + 0x220)` +0x130…+0x148 at the call `0x008cc828`, created
   elsewhere; the snapshot `snap2` is another run than the renderer dumps, where that pointer is
   0x04ca25f0, and the renderer was not located in it within the time spent; the first step's taps make an
   exact 4 × 4 box only at a divisor of 4), `kScale` = 1 / the level's width (or height) (not read), the
   vignette weight computed per pixel instead of in the combined level's alpha, bloom always on (the flags
   were 1 in free roam), and the level weights' minimum 0.01 = `DAT_00f9d474` (`.rdata`).
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
   one falls back to the base; not read live yet) with weights +0x54, +0x58, +0x5c, +0x60, +0x74, +0x78 (the first two and the
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

### Fog

The `Fog` pass (a full-screen quad, premultiplied output; shader maths in game_lighting.wgsl `apply_fog`)
reads five global constant slots, registered in `FUN_008c34b0`: `SunCoronaDirection` 0xe, `ScattCoeffs`
0x23, `ScatteringColor` 0x26, `ScatteringSettings` 0x27, `LayerFogCoeffs` 0x83, plus a 1D `GradientTextureA`
sampled at the screen u.

- **Writers**: the light-buffer setup in `008A0000.c` (around the `FUN_00a5d660(0x83)` / `(0x27)` writes;
  S = the interpolated environment state passed as its third argument) copies `ScattCoeffs` = (S+0xc50,
  S+0xc54, S+0xc58, 1.0), `LayerFogCoeffs` = S+0xc70..0xc7c, `ScatteringColor` = S+0xca0..0xcac,
  `SunCoronaDirection` = −(S+0xeb0..0xebc) and **`ScatteringSettings` = (g, (1 − g²) × s / (π × 4), 0, 0)**
  with g = S+0xcb4, s = S+0xcb0 (π at `0x01029fc0`, 4 at `0x00f9c610`, `.rdata`): a Henyey–Greenstein-style
  normalisation, so g is the keyframe's `ScatteringPhase` (0.971 at noon) and s its `ScatteringStrength`
  (0.854) by inference. The shader saturates the phase, so it peaks at 1 looking into the sun's corona.
  The scene path (`008C0000.c`, around the `FUN_00a5d660(0x23)` writes) re-writes 0x23 / 0x83 / 0x24 from
  renderer +0x26d0 / +0x26f0 / +0x2700.
- **State fill**: `FUN_008c2080` (called from `FUN_008c8b00` with renderer +0x1ef80..+0x1efc0) stores
  `ScattCoeffs` at state +0xc50 (`param_9[0x314]`), `LayerFogCoeffs` at +0xc70, slot 0x24's vector at
  +0xc80, and the scattering scalars at +0xcb0 / +0xcb4.
- **Where the coefficients are computed** (`FUN_008c0f30`, called from `FUN_008c2080`; its 12th and 18th
  arguments are the `ScattCoeffs` and `LayerFogCoeffs` outputs; `this` holds the blended fog block at
  +0x2240..+0x22dc and the two painted `LayerFog` entries at +0x2110.. / +0x2130..):
  - `ScattCoeffs` = (1 / max(far − near, ε), near / (far − near), Power, +0x22cc) with near = +0x22c0, far =
    +0x22c4, Power = +0x22c8 of the interpolated keyframe fog; the scale in front is the constant 1.0
    (`0x00f8a820`): **near / far are not scaled**. Checked live: at (865, 24.8, −578) the slot holds
    (0.0007576, 0.06061, 2) = keyframe fog 80–1400 m, Power 2, which is what our timeline `1245c8` gives
    there at every hour from 10:00 to 14:00. The interpolation is linear in near / far / Power (not in
    the coefficients): driving north across a timeline border the slot moved over ~10 s to (0.000916,
    0.0733, 1.519) = 80–1171 m, Power 1.52, i.e. a spatial blend between painted timelines that we do not
    do (we switch). w (0.70 → 0.40 across the same border) is not read by the Fog shader.
  - `LayerFogCoeffs` (DXBC `exp`/`log` are base 2): blend weight b = W1 / (W0 + W1) of the two entries
    (+0x22d8 / +0x22dc; with the enable byte +0x2148 clear or no weight: base −200, depth 1, distance
    10000, follow 0, density 0.0001); base, LayerDepth, distance (`13dacf47`), follow (`5d346b33`) lerped by
    b; base' = base + (min(base, cam_y) − base) × follow; density = (W0 + W1) × lerp(1, +0x2df8, +0x2df4).
    Then x = 4 / max(depth, 1) (4 at `0x010cf48c`), y = x × (cam_y − base'), z = (log2 50 / distance) ×
    density × (4 / ln 2) / x (`0x00f9c368` via `FUN_0042b510`, `0x0100e990` = 5.77078), w = z / (1 + 2^y);
    zero when +0x21d0 is clear. Live check: w = z / (1 + 2^y) holds to 4 digits in every dump; at the
    run's place (paint index 3 = `LayerFog` `…3eac`: entries base 308.2 / depth 25 / distance 700 /
    follow 0.94 and 22 / 35 / 492.6 / 0) x = 0.1556 gives b ≈ 0.93 and y matches a camera ~24 m high, but
    the density factor (0.53, rising over time) and the source of W0 / W1 are not traced.
- Renderer +0x1ef80.. is copied from another
  environment state (+0xc50.., `FUN_0089b4b0` at `008C0000.c` ~line 10007). **Not traced**: where
  1 / (far − near), near / (far − near) and Power are computed from `EnvironmentKeyframe.Fog` (kept as
  before, matching the shader's form), what fills `LayerFogCoeffs` (the Fog struct's `LayerBase` −100,
  `LayerHeight` 100, `926ad850` 0.65, `e391466f` 0.225, `e391466c` 0 are the candidates), and the builder of
  `GradientTextureA` (rgb and the alpha that caps the fog opacity; `Colour.w` and `ColourSun[3]` are 1 at noon,
  `ColourSun[4]` 0.395 and `02799979` 10, `078c9088` 0.562, `14d20653` 1 are unexplained).

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
  predicate `FUN_006dfed0` holds (not identified), a timer from the game mode (`GameMode+0x44`, `09d6656d` = 10 s; the
  `GameMode`s are items of the GAMELOGIC/GAMEMODES.BNDL container, formats.md "Events") counts down by the scaled frame time; on expiry, or at
  once when the mode's `+0x68` flag is set, the reset starts. Its target is the saved
  pose, turned to face the race's current target gate when the event has a checkpoint list, or the nearest
  racer when it has none and the mode's `+0x70` flag is set (corrected 2026-10-09, see "Races"), moved onto the road by
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

## Props (2026-10-09)

Runtime: `crates/mostrusted/src/props.rs`; data: `mw_assets::props` (`AdditionalInfo`, `DynamicResponses`).

### Car against a dynamic object: shared impulses (confirmed)
- **Dispatch**: `FUN_00961750` builds the contact response for a body pair through `FUN_00960f20`, which
  switches on both bodies' categories from `FUN_00929350`: 0 the player's race car (`RaceCarPhysicsObject`,
  not crashing, +0xc38 ≠ 2), 1 an AI race car, 2 a crashing race car (+0x5dd set), 3 traffic or a trailer, 4
  anything else (props, compounds, world objects: `DynamicPhysicsObject`). 0 vs 4 → `FUN_00941170`; 1 vs 4 →
  `FUN_009415f0`; 3 vs 4 → `FUN_00940d50`; 2 vs 4 → the default record (`FUN_009251f0`) with record +0x132 set;
  4 vs 4 → `FUN_0092e3e0`. Other pairs: 0/1 → `FUN_009588c0` (`PlayerVsAI`, +0x100), 0/2 → `FUN_00959210`
  (`PlayerVsCrashingRaceCar`, +0x2c0), 1/2 → `FUN_00959730` (+0x368), 1/3 → `FUN_0095a680` (`AIVsTraffic`,
  +0x1e8), 3/3 → `FUN_00958130`.
- **The record** (`FUN_009251f0` defaults: four scale lanes 1.0 (`.rdata` 0x010245c0), restitution 0): the player
  handler reads `CollisionResponses_RaceCar` +0x40c (`RaceCarVsDynamic`, field `c7d59471`): record +0x20 ← +0x34
  (`e75eb28b`), +0x40 ← +0x30 (`207258c8`), +0x30 ← +0x2c (`357a73bd`), +0x50 ← +0x28 (`1caa1e4f`), +0x60 ←
  +0x38 (`Restitution`). With the car's flag +0x1777 set, +0x20 / +0x40 come from the embedded `BasicParams`
  (`e3882eb1` 0.4, `e5ea1414` 0.01) instead. When the object's body has flag +0xf4 bit 2, both car lanes are
  raised to at least 0.01 (`.rdata` 0x00f9d474). Record +0x00 / +0x10 ← the object's main rigid body's
  `Friction` (definition +0x40 array of 0xe0-byte bodies, index at definition +0x48, field +0xa0). The AI
  handler reads +0x3d0 (`2e3c3d30`) the same way; the traffic handler `CollisionResponses_Global` +0x9c
  (`TrafficVsDynamic`: +0x20 `16c25fa9`, +0x1c `31e69a76`, +0x14 `357a73bd`, +0x10 `1caa1e4f`, +0x18
  `Restitution`). The "prop impact" / "payload impact" crash test (`FUN_009a9fd0` with the car's
  `CrashingRules` +0x28 / +0x20 entries) runs in the same handlers.
- **Applied** (`FUN_009bb640` copies the record into the contact constraint: +0x90 = (record +0x20, +0x30,
  +0x40, +0x50), +0x30 = (friction, friction, restitution), +0x80 = (1, 1, 0, 0), +0xf1 / +0xf2 = record
  +0x132 / +0x133). The sub-step contact pass `FUN_009be420` computes each point's impulse from the relative
  velocity and its stored effective mass (point +0x2c) and adds impulse × +0x90 to body A's linear accumulator,
  × +0x94 to B's, and the angular impulse × +0x98 / × +0x9c; bodies with +0xf4 bit 2 get nothing. The friction
  pass `FUN_009bed70` uses the same four lanes. The first pass `FUN_009be060` (the island's first loop) uses
  +0x80: full linear, no angular. Constraint +0x4c accumulates the normal impulse.
- **Values** (`PHYSICS/PHYSICSCONFIG.BNDL`, `010000000007f0c9` / `…f0c6`): player 0.1 car linear, 0.05 car
  angular, 1 object linear, 0.4 object angular, restitution 0; AI 0.01 / 0.01 / 1 / 0.4; traffic 1 / 0.55 / 1 /
  0.4. So a knocked prop changes the player's velocity by a tenth of what a rigid contact would: at 60 km/h a
  150 kg lamp takes about 0.55 km/h off a 1.5 t car.
- Inferred, not read: that the effective mass at point +0x2c is the unscaled one (the prestep that writes it
  was not found).

### The prop manager: activation, break-free, reset (confirmed unless stated; sub-agent trace)
- The world streamer holds a prop manager (streamer + 0xde950) driven by `FUN_00554d80` (activation, from
  `FUN_00556880`) and `FUN_00555c90` (after physics, from `FUN_0055ac30`); up to 3664 objects in four index
  ranges (instance objects, compounds 0x334.., compound parts 0x5a0.. (48), props 0x5d0..), with bitsets for
  active, touched, awake, newly active, pinned and pinned-next-frame. Registration (`FUN_0054dfe0`) copies
  `AdditionalInfo` +0x28 (`94c94bab`) into the pinned set and posts a static box proxy from the definition's
  bounds.
- **Activation**: the broadphase pair list (`FUN_00a9a720`, up to 255 pairs) is scanned each frame; a prop whose
  box overlaps an awake body (body +0xf4 bit 0 clear) is touched; active = touched | awake. New ones get a
  `DynamicPhysicsObject` created **asleep** (`FUN_009ae320`) with body flag bit 2 = pinned
  (`FUN_008fcad0` → `FUN_009aac50`); ones no longer active lose their body (message 0xc), the static proxy
  stays. The physics side has 48 slots, evicting the lowest `1/(d² + 1)` (+2 for some objects) with d the
  camera distance (×200 + 10000 outside the frustum; `FUN_0090f530` / `FUN_0090f3b0`).
- **Break-free** (`FUN_0054e570`, from the overlap pass `FUN_0054ec10` / `FUN_0054ed10`): only when
  `AdditionalInfo` +0x26 (`5586d09d`) is set; unpins when (hitter mass +0x70.w × hitter velocity +0xb0)² >
  (+0x1c `67f0233c`)²: the hitter's **whole momentum**, before any contact is solved. Then message 0xd clears
  bit 2 (`FUN_00919c80` case 0xd → `FUN_009aac50(0)`).
- **Damage / effect** (`FUN_0054bf70`): when a body-pair record's force (`FUN_00901740` +0x3c = Σ normal impulse
  / dt, N) exceeds `ForceToDamage` (+0x18), once per knocked lifetime: damaged state (model `PropObject` +8)
  and, for a vehicle hitter, the impact effect (`PropObject` +0x14) oriented by the hitter's velocity.
- **Compound smash** (`FUN_00555c90`): loose, its timer (`08c87eeb`, +0x34, seconds) run out, and the world
  contact force or a pair force above `ForceToSmash` (+0x30) → messages 2 and 0x11, the body removed. Parts
  are linked by joints (`FUN_005547f0`, `FUN_0054e120`, message 0x2f); a pinned part whose links are all loose
  is unpinned (`FUN_0054e750`). The joint-break rule was not found.
- **Reset** (`FUN_0055ae50`, every frame, no timer): a moving prop (64-slot pool, `FUN_00553fe0`) is put back
  (`FUN_00553540`) or removed (`FUN_00553740`, `mbRemoveInsteadOfReset` or a game flag) once **both** the camera
  and the player's car are farther than `mfResetDistanceSquared`. ResetProps (message 0x15, `FUN_0055a680`)
  re-creates everything.
- **Sound**: a prop's world contact plays its `PropObject` +0x1c sound (`WCPlaySoundBehaviour`) or
  "World_Prop_Collision" above a threshold from an audio object, then waits 0.35 s (`.rdata` 0xfaaf2c,
  `FUN_0054c680`).

### Port and model decisions (`props.rs`)
- Knocked props are outside the race cars' avian layers; `prop_contacts` solves car–prop contacts with the
  scales above in two passes per step (the island's two sub-steps), normal impulses clamped at 0 and friction
  within the object's `Friction` × normal. The break test runs there too, before the step, on the car's
  momentum. Traffic keeps avian's ordinary contact against loose props (its scales are near 1).
- Model decisions: unscaled effective mass; overlap push-out (0.2 of the depth per step, ≤ 4 m/s, 1 cm slop)
  and speculative contacts for gaps within a step's travel; loose-from-the-start props stand static until
  the first touch; no `AdditionalInfo` = never loose; colliders within 30 m of a race car or crashed traffic (the game activates on
  box overlap, with pools); sound gain ∝ closing / 12 m/s, 2 m/s threshold; traffic and loose props break
  standing ones through the contact hook.

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

### Race HUD, countdown and results (2026-10-09, `hud.rs`, `ui_quads.rs`)

What is data (research log 2026-10-09): the screens and layouts, the countdown / results `Sequence`s and their
`LayoutSequenceItem` times and texts, the UI blend states (`MaterialState` blocks: alpha-over for
`FlaptGenericShader`, src alpha / one for `UIAdditiveShader`), the behaviour steps. What the dump gave: nothing
new; `WITHORDINAL` / `ASORDINAL` exist only as strings (0x00f9e4e8, no code reference), the binding
formatters, widget adapters and the tween code are not in the decompiled set. The HSM text order is the
source for when the countdown starts (`Mode.IntroTime(LTEQ:3)`) and `FUN_006e2670` for `IntroTime` itself.

Bindings filled from the race (`RaceState`, the player's `RacerProgress`): `Players.LocalPlayer.EventPosition`
(hidden while 0), `.Lap` (clamped to 1..laps), `Event.Data.Laps`, `.DistanceLeft` (m), `Mode.Time` (race clock),
`DistanceToFinishManager.IsGoingWrongWay` / `HasMissedCheckpoint` (racing only), `Mode.IntroTime` = trunc(start −
now + 1 s), `ActiveGameMode.RaceLeaderboard[i].*` (finished racers by time, then by position), `Net.NumPlayersInGame`
(racers), `Widget.HUD.SummaryHeadline` / `SummaryDetail` (582390's string palettes filled), `Event.GameMode.Name`.

Model decisions (marked in `hud.rs`, `binding.rs`, `ui.rs`):
- The countdown and results sequences are the **online** race mode's (275829, 1019828): the offline mode's
  fields are 0. The countdown's items after 5 s ("FALSE START", 8–11 s) are left out. The results play after the
  finish once no sequence hides the HUD (the event outro's `HideHudSequenceItem` comes first), for their 8 s.
- Formatter output: `TIME` = `M:SS.hh`; `WITHORDINAL` = English `1ST` / `2ND` / `3RD` / `NTH`; `DIST` takes
  metres and prints whole metres / yards, `DIST,LONG` km / miles to one decimal, no unit (the layouts have
  their own unit labels where they show one).
- Leaderboard: racer names are the AI cars' names (the original's racer names were not located); TKDN / WRCK /
  SP columns are empty and `Widget.HUD.LocalPlayerScore` is 0 (no Speed Points); `~Signals.GetIntValue[#]` in a
  row = row + 1.
- Behaviour steps (the vocabulary is the data's, the meaning our reading of the names): each step runs from its
  `Delay` over its `Duration` (ms) in list order; `IN` fades in from invisible (and holds the element invisible
  until it starts), `OUT` fades out, `TO,INVISIBLE` hides, `dx,dy` approaches the rest position from that offset;
  colour steps, four-number steps and bare ids are ignored. Triggered behaviours play on the trigger's edges (a
  trigger that has not changed since first seen is settled); an element's own transition-in plays when it starts
  being drawn (again after 0.25 s unseen). Not played: transition-out, idle behaviours, the layouts' instance
  timelines (`APPROACH`, `FADE`).
- Masks with a texture (soft edges, `MaskSampler0/1` in the shader) are axis-aligned rectangle clips.

### Minimap orientation and icons (2026-10-09, `hud.rs`)

The minimap is a 3-D view: the race / free-roam HUD's `Widget_PointOfInterest` lays the map tiles as a world-fixed
plane (tile layout 371427 under R(`MapRot`) · T(`MapOrigin`) · R(90, 0, 0), `MapRot` = the grid's yaw, 0) and
renders it with its `UIMapCameraConfig`: look at `Players.LocalPlayer.GetPosition` from 600 m
(`Mode.MinimapDistance`), FOV 90, `LookAtRotation` = `Players.LocalPlayer.GetPlayerDirection`. That binding
(`FUN_006db960` → `FUN_006be9b0`) is −atan2(f.x, f.z) in degrees of f = the vehicle matrix's row 2
(`FUN_0083c030` returns +0x20..+0x2c), i.e. the car's local **+Z**. So the map turns with the car, its facing up.
**Bug fixed**: ours took Bevy's `forward()` (local −Z), so the map showed what was behind the car at the top
(rotated 180°, turning the wrong way); `GetPlayerDirection` had the same sign error. The tiles are not mirrored:
the stitched `UIMapGrid` (column = +x, row = +z from `Position`) overlays the `TrafficLaneData` lanes exactly
(checked 2026-10-09), and screen right is (−f.z, f.x) as in the 3-D view.

Icons: each `PointOfInterestVisuals` has a map layout (`b8a6b50a`) placed at `Pos` with its elements turned by
`Rot`. Drawn now: the player (key 1013507, `MapPoiConfig.29401278`: the green arrow 371790, offline variant),
the target and next gates and the finish (the keys of "Races" → "Markers": bar icons 371998 / 604244, finish
604240) and the other racers (1013521, `MapPoiConfig.5bfb3553`: the orange arrow 816012). Model decisions: an
icon's turn is its heading relative to the map (the player's arrow points up); icons keep their reference-pixel
size × the HUD scale; icons whose centre is off the map are left out (the widget's `EnableClamping` /
`IsClamped` is code not traced); which `MapPoiConfig` field the code uses for the player and for racers was not
traced (the two named hold the matching arrows). Still ours: the map scale (4 m per reference pixel from the
tile resolution, not the 600 m / 90° camera) and the tiles drawn opaque grey (the game adds them, opacity 0.7).
- Easydrive's RACES list: the offline races (730000) whose `AllowedVehicleList` holds the driven car (all races
  when none does), nearest start first. "START RACE" puts the car on the event's start 0.5 m up; the start
  must be inside the streamed world (`--units`), else the car falls and is reset. One race per run.

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
  and `FUN_007061d0`, `FUN_00707510`, `FUN_0065fc00`, and the overlap path `FUN_00707650` (see "Trigger
  hits" below).
- **Speed Points**: not found in this pass. The handler only sends events (ids in the config at global
  +0x44); the points are presumably awarded by whatever listens to them (`FUN_006a8050` dispatch).
  `CollectablesConfig` holds no point values (its floats are 2.5 at 0x60 and the 50 mph threshold).

### Trigger hits: streaming, overlap and firing (session 5, round 2)

How a car "hits" a trigger, traced from the hit handler's callers back to the trigger streamer. Confirmed by
reading the code unless marked *inferred*.

- **Streaming by grid cell** (`FUN_0053bc70`, every frame, trigger manager at `this`, the `TriggerData` handle
  at `this+0xc18`/`+0xc1c`): for each of the 16 player slots (`FUN_00571160`) with an active vehicle, take
  its position x (matrix +0x30) and z (+0x38) and compute `cx = (int)((x − origin.x) / cell.x)`,
  `cz = (int)((z − origin.z) / cell.z)` with origin at header +0x28/+0x2c and cell size at +0x30/+0x34
  (250 m). The range is `cx .. min(cx + 1, dimX − 1)` and `cz .. min(cz + 1, dimZ − 1)` (dims at header
  +0x20/+0x24), with one less at the top when `pos + 0.5 × cell` falls in the same cell (`DAT_00f8a810` =
  0.5); i.e. *inferred*: the 2 × 2 block of cells nearest the car. Cell index = `dimX × cz + cx`. The union of
  all players' cells (list at `this+0x8020`, count `+0x8280`) is diffed with last frame's: cells that left are
  released (`FUN_0053b390` → `FUN_0053a420`), new ones added (`FUN_0053b2c0` → `FUN_00539e50` per trigger
  index in the cell). This settles the header +0x1c index as **per-cell lists of trigger indices**.
- **Active trigger slots** (`FUN_00539e50(index)`): at most **300** triggers are live at once (slots of 0x60
  bytes at `this+0xe70`, a 10-word bitset at `+0x7ef0`, a reference count byte per slot, so a trigger shared by
  several cells is created once); this is the "300 entries" seen in `FUN_0053c2f0`. Per shape (record +0x11):
  **box** → a box shape with **half extents = size × 0.5** (`FUN_00a102b0`, `DAT_00faa580` = 0.5 × 4), so
  the record's size (+0x24) is the **full** extent; **sphere** → a sphere of radius +0x14 (`FUN_00a0e190`);
  **point** → no shape at all, so points are never overlap-tested (they are locations, read by id elsewhere).
  The shape is registered with `FUN_00541850` (type 2, filter 0x20).
- **Overlap job** (`FUN_00536a10`, profiler name `"TriggerUpdate"`): for each of 24 vehicle slots that is
  active and in state 0 or 2 (`+0x9c`), builds a 0x80-byte query: the car's half-extent box (for a vehicle
  with a full body: a vector at +0x20 of the object behind vehicle `+0xd8`/`+0xdc`, *inferred* the body's
  half extents; otherwise `(max − min) / 2` of its physics AABB, again × 0.5), its pose and a velocity-like
  vector (`FUN_00573450`: vehicle `+0x100`, which is the matrix's third row, the forward axis, not a velocity
  (2026-10-09, "Nitrous refill"); or the physics body's `+0xb0` when its flag bit `+0xf4 & 1` is
  clear, else zero). The job is dispatched with `FUN_00da2e50` over the active slots; its narrow-phase test
  was not read (*inferred*: the car's box against the trigger box / sphere, not the car's origin).
- **Per-vehicle hit list** (`FUN_00536f70(slot, vehicle)`, a job callback): appends the trigger index to the
  vehicle's list (16 × u16 at `this+0xc20 + v × 0x24`, count at `+0x20`). Spheres are always appended. Boxes
  with **flags bit 0 clear** are always appended; with **bit 0 set** only when
  `dot(vehicle vector, trigger axis) ≥ 0` (`DAT_00faa600` = 0.0; the axis is row 2 of the shape's matrix, its
  local Z) — a **one-way trigger** that counts only when crossed in the facing direction (*inferred*: the
  vector is the velocity). Flags bit 1 (set on speed cameras, jack spot locations, body shops) is not read
  here. `FUN_005372f0(vehicle, out, 16)` copies the list out.
- **Firing** (`FUN_00707650`, game-logic per frame, player vehicle `+0xa0`): reads the player's list and the
  extra indices queued at `this+0x1100` (count `+0x1104`), and **fires on entry only**: an index that was in
  last frame's list (`this+0x10dc`, count `+0x10fc`) is skipped. Kind filter: `FUN_006e42f0(kind)` (true for
  6, '4'/'G', billboard kinds, '0'/'H', 7/'I', and some more through `FUN_006e16c0` / `FUN_006e1690`); jack
  spot kinds ('<' '=' and letters) pass only while the game-state bytes `+0x2e90` or `+0x2e91` are set;
  kind 11 goes to `FUN_006e4770` instead (once per entry). Already-found triggers (`FUN_006fa530`) are dropped
  except speed cameras (repeatable unless `+0x2e95`). Survivors go into a ring buffer (`this+0x2f94`,
  capacity `+0x2f98`, head `+0x2f9c`, count `+0x2fa0`) that is **drained one entry per 4 s**
  (`DAT_00fcb050` = 4.0; the timer is held at 4 while the queue is empty, so a lone hit is handled the same
  frame; `FUN_006ba290()` true also skips the wait). The drained record is re-filtered and handed to
  `FUN_007059b0(player, record, 0, 0, 0)`. *Inferred consequence*: a speed camera's speed is read when its
  queue entry is handled, up to 4 s × queue position after the pass when several hits come together.
- **Other callers of the handler**: `FUN_007061d0(key)` and `FUN_00707510` walk a GeneSys list at
  `this+0x41b0`/`+0x41b4` (u16 count at +0x14, pointer array at +0x10) whose entries hold two trigger ids at
  +0x20 and +0x28 and a key at +0x24, and mark both triggers found with `FUN_007059b0(…, 0, 1, 1)`:
  `FUN_007061d0` for the entry whose key matches, `FUN_00707510` for all entries once when a config short
  (`+0xd0`) is 1. *Inferred*: the entries are `CarSwapSpot` structs (44 bytes, eight UniqueIds 0x0c..0x28,
  `mwtool genesys-types`; the read offsets 0x20/0x24/0x28 are its fields `0aff900c` / `a3b7a168` /
  `0e3bcabe`): jack spot location and car trigger ids with the car's id as key, so owning a car marks its
  jack spot pair found. `FUN_0065fc00` hits the trigger of a 0x34-byte record list (`+0x20` = trigger id).

### Billboards, gates and other kinds (session 5, round 2)

- **Billboards are counted by the prop breaking**, confirmed: game-logic message 0x14 (handled in
  `FUN_006ab390` case 0x14: unless `FUN_006e0440()`, and only if the message's flag byte is set, call
  `FUN_007059b0`; then always `FUN_006ed900(id)` and `FUN_0074c1a0(id)`) is posted by exactly one function,
  `FUN_005b5340`: a smashable object (byte `+6` == 2) carries a linked trigger id at `+0x90`; when it breaks
  and the linked trigger's kind is a billboard (`FUN_006e16f0`: '2' 'A' 'D' 'J' 'M') and its byte `+0x10d` is
  set, it posts 0x14 with the trigger record and the flag "broken by the local player's vehicle" (vehicle
  id compare); otherwise it posts message 0x12 with the object's position (`FUN_006fa900`). `FUN_005b52a0`
  posts 0x12 with flag 0 (*inferred*: the object reset). Billboard kinds also pass the overlap filter above,
  so whether the volume alone counts is open; the smash path is the one that carries the player flag.
- **Security gates** are counted by the overlap path. A second kind, **7 / 'I'**, is a gate helper:
  `FUN_007059b0` takes it through `FUN_006e1760` and searches the trigger nearest to the record's position
  among kinds "0H" (`FUN_00537600(pos, "0H", 2)`, `FUN_00543e00` → index); if that gate is not found yet it is
  hit recursively (`FUN_007059b0(…, 0, 0, 0)`), otherwise the 4 s queue timer is reset to 4. Whether the gate
  prop must break as well was not found (no gate kind in `FUN_005b5340`).
- **Body shop** (kind 6): in `FUN_007059b0` it only sets a local flag and falls through to the common tail;
  what it does (repair, repaint, heat) was not traced. Next: the tail of `FUN_007059b0` after that flag.
- **Hints**: `FUN_006ef070(4)` (speed camera), `FUN_006f3f30(5)` (billboard), `FUN_006f3f30(7)` (gate) are
  show-once tutorial hints; their texts were not resolved.
- **Speed camera record** (`FUN_008749e0(id, player, centi_mph, 0, id)`): stores the values and, when online
  (`FUN_007e9360`), builds a leaderboard request with `"eventId"` = the camera id and the board name
  `"RoadRules"` (or `"Survival"` in another mode): the Autolog **speed camera leaderboard**. Offline it only
  calls `FUN_00874830(id)` (*inferred*: local best) and a virtual callback. No points are given here.
- **Speed Points**: still not found. `BillboardWorth`, `ItemWorth`, `HighValueWorth` and `Score` are
  variables of `HSMBehaviourHSMDoSmashAndGrabScoring` (a Smash-and-Grab event mode), not free roam (dead
  end). Leads: the strings `CurrentSpeedPoints`, `SpeedPoints`, `SPEEDPOINTS`, `upload_sp_buckets`,
  `SP_%d_%d`, and the listeners of the events the handler sends (`FUN_006a8050`).

## Races (2026-10-09)

Data: `mw_assets::events` (formats.md "Events"); runtime: `crates/mostrusted/src/race.rs` (rules, API),
`chevron.rs` (route markers). Checkpoints, laps, wrong way, position and finish are C++; the race's sequence
(intro, countdown, freezing the cars, starting the timer, outro, results) is the game-logic `HSMData`
state machine (`GAMELOGIC/GAMEMODES.BNDL` 01000000000ef1e8, state names in plain text), which calls C++
through HSM behaviour codes and Lua-callable functions. Notation: **G** = the game-logic state object; **GMM**
= the game-mode manager at G+0x2f3fc; **PM** = the player manager at G+0x2f3dc (player objects 0x7f0 bytes
apart; AI racers in slots 12..23); **DTF** = the `DistanceToFinishManager` at G+0x2f47c (vtable 0xfbfd38, built
by `FUN_006b51a0`). Times are 64-bit ticks of 1/3,000,000 s (`DAT_00f9f604`). Confirmed unless marked.

- **HSM layer.** Behaviour codes (factory → vtable; slot 0 enter, 3 update returning 1 when done, 6 exit):
  `CHECKPTS` (vt 0xfd94a0, update `FUN_0075da20`, exit 0x75d960), `CHKPNTSONLIN` (online, `FUN_0075f4e0`),
  `FREEZE_CAR` (vt 0xfd9914), `GAMEMODEINTR` (0x73c750), `HSM.CheckForFalseStarts`, `ENABLETIME` /
  `DISABLETIME`, `SETSPAWNPOS`, `TPORTTOEVNT`, `SETUPRESULTS`, `ENDGAME`, `MODEENDCHECK`, `HIDECHEV`,
  `HSM.DOSPEEDT`, `EXITWHENGAME` (found in the snapshot's text; most are not in the decompiled set). Lua
  functions registered by `FUN_007763b0`: `StartGameModeTimer` / `StopGameModeTimer` (GMM `TimerRunning`
  +0x2e74 through `FUN_006f4650`), `RequestRespawn`, `RequestRespawnAtEventStart`, `RetryLastEvent`,
  `OnStartEvent`, `AddAllAIPlayers`, `RemoveAllRaceCars`, `HACKSuspendCarControlForIntro`. HSM state texts
  include "Wait For Countdown Start (Exit When Binding Is True)" with `Mode.IntroTime(LTEQ:3)`, "Play
  countdown (Run Gamelogic Sequence)" with `Mode.Data.CountdownSequence`, "HSM.PauseAIForRaceStart",
  "Disable Car Controls and Easydrive", "Start Game Mode Timer", "Request Respawn At Event Start", "Add All
  AI Players", "Race Game Mode / Run Race Logic / Race Update", "Do Checkpoints", "Event Outro Sequence",
  "Activate Results Widget". The transition table is not decoded, so the order is *inferred* from the texts.
- **Intro and countdown** (`FUN_0070caa0`, `FUN_006e2640`): `GAMEMODEINTR` sets the start time GMM+0x2d18 =
  now + GameMode +0x48 (`15f07dbf`): 10 s for races and ambushes, 0 for speed runs and two DLC modes (7.5 / 9
  online). `IntroTime` (`FUN_006e2670`) = (start − now + 1 s) in whole seconds (30 with no start set);
  `FUN_006e00b0` "intro running" = now < start. **Corrected 2026-10-09 (see "Cutscenes and sequences")**: the
  3-2-1 is online only. The states "Wait For Countdown Start (Mode.IntroTime(LTEQ:3))", "Play countdown
  (Mode.Data.CountdownSequence)" and "HSM.CheckForFalseStarts" sit in the HSM text under "In Game Flow Online
  Event", and `CountdownSequence` (GameMode +0x10) is 0 for every offline mode. An offline race starts from its
  `EventIntro` sequence, which ends with the cars teleported onto the grid already rolling (no freeze, no
  countdown); the place of the `GAMEMODEINTR` 10 s in that flow is open. At go the HSM calls `StartGameModeTimer` → `FUN_006f0f80`: starts the
  event's `Timeline` sequences (OfflineEvent +0xa0), arms the timeout GameMode +0x50 (`fe4f2d8c`, 900 s),
  resets the wrong-way state (`FUN_006581c0`). The race clock is GMM+0x2d28; what advances it was not found.
- **Holding the cars**: `FREEZE_CAR` sets PM+0xcba8 to 1 on enter, 0 on exit; the player input builder
  `FUN_006c1a80` takes handbrake = max(input, PM+0xcba8), so a frozen car is held by the handbrake alone and the
  throttle still revs it. Player flags +0x27c..+0x280 zero throttle / brake / steering and force the handbrake
  ("Disable Car Controls"). The AI is held by `HSM.PauseAIForRaceStart` (not traced). False starts are checked
  only in the online flow.
- **Starting grid**: `TPORTTOEVNT` (0x741270 → `FUN_006cd380`) sets the spawn trigger id and runs the reset
  `FUN_006c9fb0`; `FUN_0042f250` gives a trigger's direction (a point's +0x14 vector, a box's local Z). AI
  (`FUN_00686f30`): per `AIPlayerInformation` (count OfflineEvent +0xd6) the car stands on its `Placement`
  point facing its direction, spawned with zero velocity (`FUN_006ce030`) into slots 12..23; racers without
  flags +0xb5 / +0xb3 join the opponent list; with OfflineEvent +0xee (`c75c9d90`) set, a slot whose car is the
  player's car is skipped.
- **Route entries** (`FUN_00645290`; online `FUN_00642140`): DTF+0x10 holds 0x14-byte entries {position, trigger
  id, cumulative route distance}, count DTF+0x510: entry 0 is `SpawnPosition`, then for each of `Laps` (+0xf6)
  every id of `Checkpoints` (+0x64), positioned at the box centre lowered by half its height (on the road).
  DTF+0x660 = per-lap count, +0x664 = laps. `AlternateRoutes` attach alternative trigger ids to an entry in every
  lap (`FUN_0063dd80`). Each segment between entries is a path through the AI road network requested by
  `FUN_006423e0` (`FUN_00843240`, through an alternate trigger when one exists); `FUN_006676b0` stores it and
  sets cumdist[k] = cumdist[k−1] + the path length.
- **Checkpoint test** (`CHECKPTS`: `FUN_0075da20` for the player, `FUN_00759cf0` for AI): only the current
  target is tested (checkpoint counter `cp` at behaviour+0x50; the gate is entry cp + 1), every frame, with
  `FUN_00537190(vehicle slot, trigger id, −1)`: is the trigger in the vehicle's overlap hit list (`FUN_00536f70`,
  "Trigger hits" above). So it is the car's box against the trigger box, a held overlap, in order; the kind does
  not matter, and every checkpoint box in the data has flags 0 (not one-way). Gates are reference counted per
  `cp % perLap` and switched on / off through `FUN_007160b0` (`FUN_00759860`, `FUN_00759b70`); when the player
  activates one, the GPS line is built over the next min(8, perLap − 1) segments (`FUN_007a0e40`). A hit calls
  `FUN_00665350` (DTF's per-player target, DTF+0xd90 = cp + 1). Then `CheckpointInfo[cp % perLap]`: `eb1ecc26`
  (+0x13) stores the race time as a split (player+0x4c0 + 8n); a nonzero `Sequence` is played (for AI only
  with `a1ec8d81`); in the data every entry has Type 0 and Sequence 0. An event (config +0x44 → +0x60) is sent
  for every checkpoint but the last two. After a reset (`G+0x25694 + slot` pending flag), `FUN_00661da0` moves the
  target on by at most one when the car is already past it (more than 0.5 m along), without counting a lap.
- **Laps**: the lap counter (behaviour+0x54, from 1; the `Lap` binding, player+0x5bc) goes up after a hit when
  (cp + 1) % perLap == 0. The finish is the hit on the last entry (cp + 1 == count − 1).
- **Distance to finish** (`FUN_00661b40`, per slot at DTF+0xdf0): cumdist[last] − cumdist[target] + what is
  left of the target's segment polyline (up to 150 points of x, y, z, length at DTF+0x2778) after the car's
  projection onto its nearest piece (`FUN_0051e850`, clamped); past the end it adds the distance driven on the
  following segments; more than 20 m behind a segment's start it adds the straight distance to that start.
- **Position** (`FUN_00749bb0`; behaviour+0x58, written to player+0x5b8): 1 + the opponents ahead, where ahead
  = finished (its cp ≥ count − 1) or a strictly smaller distance to finish (cached per slot, FLT_MAX at init);
  equal distances do not overtake. Quirk: an opponent also counts as ahead while the player's cp is below its
  "was resetting" byte (behaviour+0xc0 + slot), which only fires at cp 0 (looks like an original bug). Events:
  "took the lead" (config +0x2dc), "lost the lead" (+0x12c), first place held 1.5 s (`DAT_00fd2338`,
  `FUN_00749570`). DTF's own 12-entry leaderboard (`FUN_0066aa60`, insertion sort `FUN_0066d680`; `Position`,
  `OvertakenCount` bindings) puts finished players first by finish time, then the rest by distance, ties
  keeping the previous order.
- **Wrong way / missed checkpoint** (`FUN_006585b0`, state machine at DTF+0x439b0, run by `FUN_0066b3f0` for
  the local player): d = distance to finish through the target + the car's distance from its projection, d' =
  through the next target. State 0: reset the timer, store the position; d rose → 1. State 1: after 1 s (`.bss`
  0x01177580) with d rising while d' falls → *missed checkpoint* (flag set) → 2; else after 2 s (0x01177388) and
  more than √10 m moved (`DAT_00fb77b8` = 10, squared) → *wrong way* (flag clear) → 2; else d fell → 0. State 2:
  d stops rising → 3. State 3: d rises → 2; after 1 s (0x01177378) → 0. `IsGoingWrongWay` (`FUN_0065dcf0`) =
  state 2/3 without the flag; `HasMissedCheckpoint` (`FUN_006584d0`) = state 2/3 with it, only for checkpoints
  1..count − 2; both false during the intro. No respawn or penalty: `CHECKPTS` asks for a new GPS route to
  the target (`FUN_007684d0` → path type 9) when `FUN_00656220` holds, and speed runs stop counting distance
  for the average speed (`FUN_006e0580`).
- **Finish** (`FUN_006b9d00`, when the race state G+0x2f458 +0x85c is not 3): the player's place and finish
  time (= race clock) are stored; the won flag GMM+0x2e78 is cleared when the place is above 3, or above 1 with
  OfflineEvent +0xe9 (`c8f26580`, set on the Most Wanted races). `CHECKPTS` returns 1 and the HSM goes to the
  outro and results. An AI racer reaching its last checkpoint gets AI goal 3 (`FUN_00843030`, *inferred* stop
  racing) and racer+0x5d7 = 1; no time is stored. On `CHECKPTS` exit the markers and triggers are released
  (`FUN_00759fa0`) and every remaining AI racer gets +0x1ea7 = 1 and goal 3. `FUN_006f46e0` also fails the
  event past `TargetTime[0]` (count +0xe4, when GameMode +0x58 is 0), below the last `TargetSpeed` (speed runs:
  average speed GMM+0x2c88), below `TargetScore` (+0xcc), at the timeout while DisplayTimeout is on, or with
  player+0x434 set outside free drive. `SetUseFinishLineCamera` has no code reference (Lua / HSM).
- **Respawn** (`RequestRespawn` 0x675b10 → `FUN_006cd340` → `FUN_006c9fb0(0, 0)`): in a race (DTF count ≠ 0) the
  car goes back to the saved safe pose **facing the current target gate** (direction = gate position from
  `FUN_00658310` − saved position); without a checkpoint list and with GameMode +0x70 (`bc5ea1ae`) it faces the
  nearest racer of the opponent list (`FUN_006c3430`). `RequestRespawnAtEventStart` (0x675b40) uses the spawn id
  at G+0x2f6f8. `RetryLastEvent` and `ResetCurrentEventStats` are HSM-driven.
- **Event types** (`FUN_00701cb0`): the event's GameMode id against GameLogicConfig fields: `IsRaceEvent` =
  730000 (+0x170), speed run 955569 (+0x1ec; GMM+0x2e8c), `IsAmbushEvent` = 390185 (+0x60),
  `IsScoreBasedEvent` = 2173009 (+0xb0), free drive 390145. Within races `TypeName` tells sprint (979289),
  circuit (979287) and the Most Wanted 1-vs-1 races (457003).
- **Markers**: GameMode +0x6e (`f1123d7b`) turns the player's gate markers on (race, speed run, DLC modes; off
  in free drive and ambushes). `FUN_00759860(player, cp, active)` registers a point of interest at entry cp + 1
  and, when active and cp + 1 < count − 1, calls itself for cp + 1 with active = 0: so the **target gate and the
  one after it** are marked, no others. `FUN_007545b0` picks the look's key: target = `CheckpointConfig`
  `NormalPoi` (+0x28, 1013519) or the per-Type field of `CheckpointInfo[cp % perLap]` (Type 1–4: +0x0c, +0x30,
  +0x1c, +0x20; every entry in the data has Type 0), next gate = +0x24 (1013518); when the gate is the last entry
  (cp == count − 2) `MapPoiConfig` +0x28 (1013510) as the target, +0x2c (1695650) as the next gate.
  `FUN_00749930` builds the point's matrix: the trigger box's rotation (`FUN_0042ee30`, quaternion at record +0x14)
  with its rows scaled by size.x / 22, size.y / 18, size.z × 0.5, at the box centre lowered by half its height
  (record +0x24..+0x2c is the size); `FUN_00790c70` adds it to the POI list (0x60-byte records at +0x7b24, up to
  260). **The keys are `PointOfInterestVisuals.9a5a94d3`, not the objects' UniqueIds** (2026-10-09; formats.md
  "Points of interest"): the looks are listed by the race HUD screen's `Widget_PointOfInterest`, each with a map
  icon and world layouts. The world layouts' bindings are evaluated by `FUN_00805870`: `Transform` = the record's
  matrix, `Pos` = its translation, `Rot` = its Euler angles in degrees (`FUN_005bad40`; the row scaling drops out
  for a level gate), `Scale` = the row lengths (unused by the gate looks), `DistanceToCamera` = |camera (+0x140
  of `DAT_010edaf4`+4) − position|. So the marker is a UI panel in the gate's plane, 0.1 m per layout pixel ×
  0.5…8 with distance 20…500 m, 0.25 m above the box bottom: a thin pole (4 × 115 px, `UIDepthTestedShader`) with
  "CHECK" and the race time on glow strips at its top, fading in from 20 to 60 m; the next gate a faint bar and
  "CHECK" that fades out within 10 m; the finish "FINISH". `UIDepthTestedShader` compares against the deferred
  depth by hand, i.e. it is drawn in the UI pass after post-processing. The road chevrons (`CustomChevron` lines
  drawn with the `Chevron` / `ChevronBlockRoad` materials of `CheckpointConfig` `53a76c21` / `beb301a3`, shaders
  ported in `chevron.wgsl`) and the GPS line come from the route (G+0x2f480); the strip builder was not found.
- **Not found**: what advances the race clock; `HSM.PauseAIForRaceStart` and the false-start check; the AI goal
  enum; the HSM transition table; the chevron strip builder (height, UV scale, which flag picks which material).
  Found since (2026-10-09, "Cutscenes and sequences"): `Mode.Data.CountdownSequence` is GameMode +0x10 (0 offline);
  the outro is the event's `EventOutro` sequence, its camera an ICE take.

### Port and model decisions (`race.rs`, `chevron.rs`)

Ported as above: intro length from the game mode, 3-2-1 from `IntroTime`, handbrake freeze with live throttle,
route entries, target-only gate overlap (car box from `body_half_extents` vs the trigger box, separating
axes), lap and finish rule, distance to finish, position rule, the wrong-way state machine, the won rule
(with `TargetSpeed` / `TargetTime`), respawn facing the target gate (`crash.rs` reads `race::RespawnFacing`).
Model decisions: segments between entries are straight lines (no road-network paths), so positions and the
wrong-way test differ where the road bends away from the line; the car box is centred half its height above
the car's origin; the wrong-way machine runs for every racer (the original: the local player); AI finish times
are kept for results; the speed run's average speed is the counted distance over the race time; the race clock
advances by the fixed tick; the chevron boards are 2 m tall with one texture repeat per 2 m, `baseColour`
white, premultiplied-over blending, `f8ccf73e` set = block-road crosses (from the data: those lines are short and
cross side roads). Gate markers (`gate_marker.rs`, drawn by `world_ui.rs` after the tonemap with the scene's
depth): the looks, transforms, fades and the target / next-gate choice as above; model decisions: layout x runs
to the approaching driver's right and layout y down (the 3D UI's axis convention was not found: the minimap's
tile layout, R(90, 0, 0) after T(`MapOrigin`), maps layout +y to world +z, which taken literally would hang the
pole under the gate's ground point and mirror the text for a driver passing along the gate normal; ours stands
it up and reads from the approach), colour transform type 3 multiplies the layout's opacity, the text
(`FlaptGenericShader`) is depth-tested like the pole, markers show in the intro and race phases while no
sequence hides the HUD. The box bottom is below the road on some gates (gate 0 of 462174: 4 m), as in the data.

## Cutscenes and sequences (2026-10-09)

Data: `mw_assets::sequence` (formats.md "Sequences"), the ICE takes `mw_formats::camera_take` (formats.md "ICE
camera takes"); runtime: `crates/mostrusted/src/cutscene.rs`; listing: `mwtool events <id>` prints both
timelines, `mwtool camera-take [take]` evaluates a take. Confirmed unless marked.

- **Which sequences an offline race plays** (data + HSM text). `OfflineEvent.EventIntro` (+0x70) is the intro, a
  `Sequence` in the event's `IntroMessageGroup` (+0x80, `EN_US/FEEDBACKGROUPS/<id>.BNDL`; HSM: "Does Event Intro
  Exist", "Run PageRunner2 Context - EventIntro" with `Event.Data.IntroMessageGroup`, "Disable Car Controls and
  Easydrive", "Run intro sequence" with `Event.Data.EventIntro`, "Pause Intro When Skipping"). `EventOutro` (+0x74)
  plays after the finish ("Event Outro Sequence", `Event.Data.EventOutro`), then the results; `AROutro` (+0x60,
  the Autolog-recommendations outro, `Event.Data.AROutro` / `Mode.Data.AROutro` = GameMode +0x0c) is the same id
  as `EventOutro` in every offline event. The outros live in the game mode's feedback group: the HSM state "Select
  Game Mode Feedback Group" holds 604180 and 604179 and reads `Mode.Data.FeedbackGroupID` (GameMode +0x75: 2 for
  races and speed runs, 1 for ambushes; which id goes with which value is *inferred* from the order). 457707 is
  the global group. Shared outros: 1689787 (sprints), 1689791 / 1689795 (circuits), 995395 (speed runs),
  1689780 (ambushes); the Most Wanted races have their own. Every one of the 108 intros is a bespoke
  cinematic of 13–54 s; 104 end with an `EventTriggerSequenceItem` and carry a `JumpOnButtonPress` skip.
- **No countdown offline**: GameMode `CountdownSequence` (+0x10), `Intro` (+0x24) and `ResultsSequence` (+0x34)
  are 0 for every offline mode (set for the online ones), and the countdown states are in the online flow (see
  "Races"). The intro instead ends on a rolling start: in 462174 at 18.733 s `TeleportPlayerSequenceItem`s put
  the player on trigger 462172 at `Speed_MPH` 70 and `Players.Racer[1..8]` on the opponents' `Placement` points at
  55, `DriveLocalPlayerWithAISequenceItem` (`MaxSpeedMph` 999) drives the player until 23.1 s, slow motion ramps
  0.05 → 1 from 18.733 to 24 s, the dynamic camera blends to the chase camera from 22.5 to 24 s and the event
  trigger fires at 24 s. *Inferred*: the race starts on that trigger (it is the only item that signals the game
  logic; the HSM's "Exit When Timer Is Running" waits for `StartGameModeTimer`).
- **Sequence clock**: real time, as the crash camera (the slow-motion items scale the simulation only).
  `JumpOnButtonPress` {`TriggerType`, `TriggerTime`, `DestinationTime`, `Action`}: after `TriggerTime` a press
  jumps the clock to `DestinationTime` (462174: from 1 s to 19.5 s, past the cinematic, just after the teleports).
  The button is SKIP_CUTSCENE (34; Esc, pad Back, "Input and control bindings"); *inferred*, `Action` 1 is not
  traced. `ConductorSequenceItem`s start nested sequences (the outros put all their content in one).
- **`AnimatedCameraSequenceItem`**: a clip (`CameraAnimation`) of a two-bone camera skeleton (`CameraSkeleton`,
  identity bind pose, bone 1 animated, 30 fps, f32 quaternions), in world space or, with the `6f3ea350` binding
  `Players.LocalPlayer.GetTransform`, in the car's frame. Lens: the director camera at 0x2dc0 (`FUN_005c6c10`)
  takes FOV = 2·atan(18 / focal length) in degrees (`.rdata` 0x00fb0e94 = 18, 0x00faf850 = 180/π) and an aperture
  focal / f-stop (`DAT_00fb0ea8` = 64); `FocalLength`, `FStop`, `FocalDistance` are modulated values (462174 zooms
  24 → 10.4 mm in its 3 s shot). The bone's axes are *inferred* from the clips (the handler was not found): its X
  stays level in every clip, but its +Y points up in 704 of the 753 camera clips of the 108 intros and down in 49
  (7 of 462174's 8, one or two in eleven other intros), so no single fixed axis mapping renders both kinds
  upright. Ours: the camera looks along the bone's +Z (the game's camera and car convention, z forward; in
  462174 that frames the animated car under the overpass, −Z would look away from it) and takes as up whichever
  of ±Y the clip holds upwards on average. In 575163 the alternative (look −Z, +Y up, the usual exporter camera)
  also gives purposeful shots, so the look direction of the +Y-up clips is not settled.
- **ICE takes** (`CameraSequenceItem.Camera.7aef95c3` = a `CameraTake` UniqueId; all 59 camera items of the
  intros and outros are takes, none bystander shots): EA's ICE library at 0x00e20000–0x00e31000, decoded in
  `mw_formats::camera_take` (layout `FUN_00e26fb0`, 17 channels and 81 elements from `FUN_00e2ccc0` / `.rdata`
  0x010e7858, cubic Hermite keys `FUN_00e28cc0` with tangents `FUN_00e28aa0`, fixed-point decode
  `FUN_00e25d80`, camera `FUN_00e29c20` / `FUN_00e27a70` / `FUN_00e278c0`, FOV 2·atan(15.96 / lens) `FUN_00e27340`).
  ICE axes are x forward, y left, z up; the game converts with game = (ice.y, ice.z, ice.x) (`FUN_00495b70`,
  `FUN_00496560`, `FUN_004ad090`). Eye and look each live in a space per interval: Player (an avatar's transform,
  `NOROLL` = heading only), World, Hybrid (world axes at the avatar), Scene (not modelled). The item's
  `01130178` bindings become avatars 1.. (`FUN_005d22b0` → `FUN_0049b180` → `FUN_0049a3c0`, "BoundAvatar%i"); a
  binding whose `0bd6ff75` flag is set is sampled once when the take starts (`FUN_0049a910` re-reads it every
  frame only while the flag is clear): the race outros freeze the finishing car's frame. `GAME_CAM_BLEND` (read
  back by `FUN_004962f0` case 0) blends to the gameplay camera, 1 = gameplay (*inferred* from the data). The race
  outro take 0x19c8b2 looks almost straight down from 3.6 m above and 4.4 m ahead of the car (lens 19.3 mm) while
  the outro's slow motion drops to 0.01 within 1.3 s and to 0 from 10.5 to 13.5 s; it blends to the chase camera
  at 11.2–11.4 s.
- **Other items played**: `SequenceItems.DynamicCameraSequenceItem` (a `Camera.DynamicCamera` whose process nodes
  read `Camera.GameplayExternalCameraTransform` / `GameplayExternalFOV` and lerp to them; the item's channels drive
  the `CameraTransformLerp` and `ProcessNodeFloat32Lerp` nodes by curve), `AddBehaviourToRaceCarSequenceItem` with
  an `AnimatedRaceCarBehaviour` (an 18-bone clip: bone 1 the car, 2–5 the wheels, the rest `av_*` / `lv_*` velocity
  bones and `controls_1` / `controls_2`; names in the behaviour, CRC ids in the skeleton), `HideHudSequenceItem`,
  the letterbox layout 0x19c66, `SlowMoSequenceItem`, `ClearTrafficSequenceItem` (radius around triggers or a
  binding), `HideRacecarSequenceItem`.
- **Not traced**: the item handlers themselves (reached through vtables; only `CameraSequenceItem` is tested by type
  id), so how the teleport sets the car's speed, what the AI drive's `DrivingStyle` does, whether a cut snaps the
  gameplay camera; `DUTCH` (roll) in ICE takes: the code turns its +0.25 maximum into a 90° roll and 11 takes
  (including 0x167b67 `Default_No_Race_Event_Outro`) hold it there, which looks wrong (unit or decode to check
  against a capture); LAG, BUNGEE, shake and Scene space of takes; the take channels LETTERBOX / FADE / OVERLAY /
  TIME_SCALE downstream; the outros' nested timeline controllers (jumps waiting for UI triggers).

### Port and model decisions (`cutscene.rs`)

Ported: the intro before the race (the race waits in `Waiting` on `RaceState::cutscene`, and the intro starts once
the opponents are placed), the outro after the player's finish, the skip, animated cameras, ICE takes with their
bindings and `GAME_CAM_BLEND`, the dynamic camera blend, teleports with their speed (and `RaceState::rolling_start`:
no freeze), the AI driving the player (`ai.rs`'s driver on the player's car with `MaxSpeedMph` as its schedule
speed), animated race cars, slow motion, the HUD, the letterbox (shared with the crash camera), traffic clearing.
Model decisions: the race starts on the intro's event trigger (else at its end) with no countdown; the camera
bone looks along +Z with the clip's upward ±Y as up; `GetTransform` bindings of animated cameras follow the car live; both FOVs are
used as vertical (as camera.rs); the animated car is placed on its bone every tick with the clip's velocity
(zero across a cut), the root motion ignored; an opponent teleported onto the player's own start is left where it
is (462174's `Racer[8]`: the original seats fewer opponents, our opponents' cars are a model); `Racer[n]` = grid
slot n − 1; after the finish the AI drives the player along a road-graph route from the finish (the race route
ends there); nested timeline controllers are not run, so the outro plays straight through; audio, post-processing,
image palettes, UI layouts other than the letterbox, speech and music items are not played; the outro's results
panels are the HUD's.

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
was inferred and is **confirmed** by a live read (2026-10-06: all 36 floats equal the data file's, research log). `mph` = `0.44704` (`DAT_010c4d24`).

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
- `interval = 60 / d × lerp(sys[0xc8]=1, sys[0xb8]=1, u)`, with `u` uniform in [0,1) (the start was read from
  the x87 code, see "Traffic runtime").

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

`FUN_00617d00` is a **behaviour arbiter**, not a plain speed controller (confirmed; fully read in "Traffic
runtime" below):
- `FUN_005ea6a0` scores six options from the gap, the closing speed and the car state with the
  `TrafficFuzzyLogicConfig` envelopes.
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
The controller, the stop lines and traffic lights, the route choice and the 150-slot range (parked cars) were
read later: "Traffic runtime" below. Still not traced: the swerve and near-miss reactions, the horn, the switch
to a physical car and the 24-slot range.

### Traffic runtime (2026-10-09, ported in `crates/mostrusted/src/traffic.rs`)

Read from the dump and the disassembly for the runtime; `sys[x]` / `prm[x]` as above, `fuzzy[x]` = the block at
`world+0x69810` that `FUN_005ea6a0` reads (`ECX` in `FUN_00617d00`'s call, from the disassembly). The traffic
object `world` sits at `0x23f4e780` in `snap2` (16-aligned; the research log's `world+0xa2b74` is `+0xa2b70`
from this base, which `FUN_005ef0f0` fills with `sys[0x80]`). All `prm` field names except `MinAcceleration` and
`SwerveScale` are unhashed; `mw_assets::traffic` names them by use.

- **Spawn interval** (`FUN_005fb120`, settled from the x87 code): `interval = 60 / d × lerp(sys[0xc8], sys[0xb8],
  u)`; both are 1, so the interval is exactly `60 / d`.
- **Placement** (`FUN_00608720`, confirmed): a car is not placed within `sys[0xd4]` = **50 m** of any player with
  a vehicle (player kind `+0x9c` = 2). The paint is **round-robin per vehicle type** (`world+0xa0390[type]`
  incremented modulo the palette's paint count, stored at car `+0x76`), unless the caller asks for one colour
  (matched against the palette entries' ids; `TrafficFlowType.Colour` is 0 everywhere). The car's front / back
  extents come from the vehicle type's `RaceCarPhysicalDefinition` box (`centre.z ± half.z`; a trailer adds
  10 m behind). The random `u` drawn first is the car's **personality** `x` (car `+0x3c`).
- **Hull population** (`FUN_0060db90`, called from `FUN_0060e5e0` for every newly active hull; confirmed): with a
  carry `c = u · 0.99`, every section entered from another hull (backward hull ≠ this one) starts a chain
  (`FUN_0060ce20`): the section, then its first forward link while it stays in the hull, then the starting
  section's 2nd and 3rd links recursively. Per section (`FUN_0060afb0`): `n = length / (Speed·mph) / (60 / d) + c`
  cars; `⌊n⌋` cars spaced `length / ⌊n⌋` apart from `spacing × frac(n)`, each moved on by `u × sys[0x84]` (0.3) ×
  spacing; the fraction carries to the next section. The same function then places the hull's **parked cars**
  (static vehicle records): byte `+0x42` is a percentage chance, bit 0 of `+0x43` skips them in a pursuit, `u16
  +0x40` is the flow type; they go into the 150 slots after the traffic ones (the "256–405" range is parked
  cars; not ported).
- **Per-car decision** (`FUN_005fc220`, confirmed). Inputs to the scorer:
  - *obstacle* (from the list `FUN_006024f0` builds: players' cars, 18 other objects, physical traffic): the one
    in the cone `fwd·d ∈ [0, 50]`, `cos ≥ 0.7071` (`FUN_00511540`; heights scaled by 2.5) with the smallest
    `|d| × weight × (1 − cos)` (weight 0.5 for the local player and for crashed traffic, 1 otherwise); its
    distance ahead `fwd·d`, height `up·d`, closing speed `dir·(v_own − v_obs)`, lateral offset `right·d ×
    (0.5 + 0.5 |right_obs·right|)`, and its speed along our heading;
  - *lead*: distance to a held stop line (below), gap to the car ahead `max(0, dist − front − prm[0x10] −
    prm[0xac]·x)`, closing speed `v − v_ahead`;
  - *own*: impatience `prm[0x84]·x + wait`, a flag (1 in modes 0 / 2, or in mode 4 behind a car in mode 0 / 2),
    and `prm[0x80]` (0.15) in mode 1.

  Unset inputs are FLT_MAX. A block for the case of the player behind a slow car (`.bss` thresholds at
  `0x0110f640` / `0x0110f780`) was not ported.
- **The scorer** (`FUN_005ea6a0`, from the disassembly): memberships are trapezoids `clamp(min((x−a)/(b−a),
  (d−x)/(d−c)), 0, 1)`, four lanes per input; the lanes match `TrafficFuzzyLogicConfig` envelopes by value
  (obstacle distance: `07b6e570`, `DistanceNear`, `249c401d`, `3a5dd3ed`; height `HeightNear`; closing
  `13ca8b67`, `01d18569`, `5483011b`, `0072dac3`, also used for the lead's closing; lateral `6eec31b8`; stop
  line `87f03da4`; gap `050a5e6e`; impatience `3eddeb08`). Six scores:
  `S1 = min(H, L, max(min(Dnear, Cclosing), min(Dclose, 1 − Copening)))` (stop for the obstacle),
  `S2 = min(H, max(Dnear, Dmid), L, 1 − Cclosing)` (match it), `S0 = 1` if `max(S1, S2) > 0` and in mode 1, else
  `min(impatience, max(flag, S1, S2)) + mode-1 bonus` (start a manoeuvre), `S3 = stop line`, `S4 = min(gap,
  1 − leader opening)`, `S5 = fuzzy[0x510]` (0.2, read live; its writer was not found).
- **The arbiter** (`FUN_00617d00`, confirmed): takes the first score that beats the best so far by 0.05
  (`DAT_00fb2390`; all zero → option 0). Output (mode, target speed `A`, distance `B`), cruise `= Speed·mph ×
  world+0xa2c00`: 0 → mode 5 plus a request in the 5-entry manoeuvre queue (`world+0x32bac`) with a rate
  `lerp(prm[0x98], prm[0x94], x) × 2.23694`; 1 → mode 0, `A = 0`, `B = max(0, obstacle − prm[0x9c])` (10 m);
  2 → mode 2, `A = min(cruise, max(min(v, v_obs), prm[0x3c]·gap))`, same `B`; 3 → mode 3, `A = 0`, `B = stop
  line − prm[0xb4]·x`; 4 → mode 4, `A = min(cruise, max(min(v, v_ahead), prm[0x3c]·gap))`, `B = gap`; 5 → mode 5,
  cruise. Two gates zero all scores: `world+0xa2be6`, and `world+0xa2ba8` (`sys[0x134]` = 7) ≤ 16 −
  `world+0x32b20` (not identified; not ported).
- **Speed controller** (`FUN_0061a190`, confirmed): the cruise is capped at 5 m/s (`DAT_00fb278c`) when the lane
  direction `3v + 10` m ahead along the planned route (`FUN_005f11a0`) is at most `cos = 0.85` from the
  heading. Mode 5 (and 1 with flag 8): `a = clamp(min(cruise, 500) − v, prm[0x64] = −8, prm[0x4c] = 4)`. Modes
  0–4: `a = prm[0x50]` (−20) when `B < prm[0x78]` (0.05 m), else `(min(A, cruise)² − v²) / 2B` clamped to
  `[−20, prm[0x44] = 8]`. Integration (`FUN_0061a450`): `v ← max(0, v + a·dt)`, 0 below `prm[0x70]` (0.05) when
  not accelerating; brake lights (`+0x21` bit 0) when `a < −0.6` or `v < 2.5` without accelerating. Waiting time
  (`FUN_00614c20`): grows by dt in modes 0 / 2 below 5 m/s unless the car directly ahead is traffic in mode
  0 / 2 / 4, reset otherwise.
- **Advance** (`FUN_0061a550`, confirmed): distance `a·dt²/2 + v₀·dt`, or the stopping distance `v₀² / 2|a|`
  when shorter; the rung position moves over the cumulative rung lengths. At the section end the next section
  is the planned route's head, else chosen now; none, or a hull outside the area, removes the car
  (`FUN_005f04c0`).
- **Route** (`FUN_00618470`, `FUN_005fc020`, confirmed): of the up to three forward links the car takes the one
  with the lowest load: cars in the link section's span × the span's reciprocal capacity, summed along the
  span's continuation chain (a tortoise-and-hare walk; the counts are read from the first hull's record), 1 for
  a hull outside the area; ties go to the first link. The route is planned up to `prm[0x34]` (25 m) ahead, at
  most 5 sections.
- **The pose** (`FUN_005e7f20`, `FUN_006132d0`): the position interpolates the rung midpoints; the heading
  blends 0.4 (`DAT_010c4d20`, `.data`) of the way to the rung segment's direction per update.
- **Leader scan** (`FUN_005f1380`, confirmed): the next car on the same section (cars keep a sorted list,
  `world+0x28f80`), the rearmost car on each forward link and on their forward links while within the
  look-ahead, and (within `prm[0x5c]` = 14 m of the section end) cars on the other sections that feed the same
  successor (`FUN_005f0b90`), unless stopped at a stop line slower than `prm[0x74]` (5 m/s). Distances end at
  the other car's back extent.
- **Stop lines** (`FUN_005f0d90`, `FUN_00439b60`, confirmed): stop lines are `u16` rung positions in 8.8 fixed
  point; a section lists its first and count. The car looks for the first stop line ahead in its section, else
  along its route; it stops for it only when the hull record's flag (`+0x250 + index`) is set, within
  `prm[0xb0]` (100.3 m).
- **Traffic lights** (`FUN_00605d30`, confirmed; formats.md has the junction record): per active hull and
  junction a timer counts down; at 0 the phase advances (`(phase + 1) mod count`) and the timer gains the new
  phase's duration × 0.1 s; each light group's stop-line flags are set to its bit in the phase mask (set =
  red); the light props get messages 0xb / 0xc. Typical cycle: 15 s, 2 s all red, 15 s, 2 s all red.
- **Spawn check** (`FUN_0060e5e0`): the headway test subtracts the last car's back extent from its position
  (`pos − back − new ≥ Speed·mph × 2 s`), which measures to its front; ported as read.

Not found or not ported: the manoeuvres (option 0's queue: swerve, cut-up, weave, panic, jack-knife), the
game's own switch to a physical car (`FUN_005fcbd0` sets up the kinematic wheel job, not the crash; the crash
entry was not found), the kinematic wheel and suspension pose (`FUN_005f7cf0`), the point the load budget
measures from (`world+0x36110`, several hundred metres from the car in the snapshots), the world vehicle
allocation for lane followers (`FUN_00608fb0`, 80 m / 30 m tests from `sys[0x118]` / `sys[0x11c]`), and per-car
lights.

**Runtime model decisions** (each marked `MODEL:` in traffic.rs): option 0 keeps a car stopping for its
obstacle (no manoeuvres); the load budget and the draw range measure from the player's car; models and
colliders only within 250 m of the player (performance); a crashed car is a dynamic box down to the ground
(no wheels), turned physical when the player's car or another crashed car touches it; cars and crashed cars
on hulls that leave the area are removed (inferred from the per-hull state going away); the heading blend is
scaled to the fixed step; the random seed; junction timers start at zero; traffic waits for the player's car.

## AI racers (2026-10-09)

Read from the dump by three parallel searches; ported in `ai_driver.rs` (the law) and `ai.rs` (routes, spawning,
the race hookup). Labels as above: **confirmed** = read from the code / disassembly, **inferred** otherwise.
`D` = a driver object (12 of them, 0x5870 bytes each, at manager + 0x630, the manager at `DAT_010edb50`),
`AIC[x]` = `AIConfig` (`GLOBALCONFIG.BNDL` 010000000007fb0e) at instance offset x, reached through
`DAT_010edac0` / `DAT_010edac4`.

### Same physics as the player (confirmed)

- The fixed tick (`FUN_004766a0`) runs the AI pass first (`FUN_0052a450` → `FUN_004e78f0` → per driver
  `FUN_004e75f0`: clear the command block at D+0x57f0, run the state's Update, the stuck / reset timer), then the
  player manager (`FUN_00597280` → `FUN_00580640`: per player, clear the input record at player+0x1928 and fill
  it through the entity's vfunc +0x28; for an AI racer `AIPlayer::FillInput` 0x006c0890, not decompiled, read
  from the disassembly), then the world step, where `FUN_0091c5c0` writes **every** race-car slot's controls
  with `FUN_009aacc0` from its input record (`FUN_005734c0(i)`). AI racers drive the full handling model.
- Input record: +0x04 throttle, +0x08 brake, +0x0c hold override, +0x10 handbrake (analog; the mask is set
  above 0.1), +0x14 steering, +0x1c nitrous amount, byte +0x33 nitrous mode. Command block (D+0x57f0, 0x30
  bytes): throttle, brake, hold, steering, handbrake, nitrous, target speed (+0x18, m/s, FLT_MAX = none),
  modes, "throttle set" / "brake set" bytes (+0x2c / +0x2d) that stop later stages overwriting them.
- When a racer has no physical car (far away; the slot's car index < 0) it advances **virtually** along its
  path (`FUN_005056b0`, `FUN_005046e0`): speed rising at most AIC[0x54] = 15 m/s², capped by the path's
  corner speed byte and the top speed (`Vehicle_Gameplay` +0xac mph, or 110), then teleported in when a car
  appears (`FUN_005705f0`). Not ported (all our opponents are physical).
- No handling cheat was found on the control path. The AI's advantages are the rubber band (below), a nitrous
  tank that refills at AIC[0x10] = 10 units/s (`FUN_00667c10`, the AI branch; the AI earns nothing from drift
  or near misses) and the shunt steering.

### The racer state (state 8, `FUN_0050da90`)

The driver is a state machine (`FUN_004ee970` switches; vtable [0] Enter, [1] Exit, [2] Update, [3] Id).
Behaviour command 1 maps to state 8 (vtable 0x00fa8868), the racer: `AIPlayerType.Behaviour` 8 → 1
(`FUN_006b8a40`); 75 of the 77 racer types events use have it. State 9 runs state 8 and switches to the chase
state when the player is ahead; 0 / 18 / 22–24 / 26 are cop roles, 4 drives to a point, 11 is the crash
recovery, 13 the traffic-lane driving. State 8's Update, in order: path upkeep (`FUN_0050a070`, the lane
planner `FUN_0050b000`), crash check (`FUN_004fb9f0` → state 11), then for a physical car the command
builder `FUN_0050a740`, nitrous (`FUN_004fdb60`), racer interaction (`FUN_005050d0`), the emergency brake and
pedal controller (`FUN_005095b0`), and the post to D+0x57f0 (`FUN_004dff40`).

**Steering** (`FUN_0050a740` → `FUN_00508d50` → `FUN_00506c00`, confirmed):
- Look-ahead time T (`FUN_00503880`): over the path nodes within AIC[0x1f0] = 0.6 × speed metres, sum the
  node turns; stop at the first node whose inside clearance (+0x38 when the summed turn is positive, else +0x3c)
  is below AIC[0x1ec] = 8 m and that turns more than AIC[0x1f4] = 90° (or has node flag 0x80); d = the path
  length to it (else to the last node scanned). T = d / speed, at least AIC[0x1fc] = 0.45, at most the
  planner's cap D+0x549c (lane planner +0x51c, clamped to AIC[0x200] 0.15 … 0.6) and at most 0.6.
- Look-ahead distance L = max(AIC[0x144] = 15, speed × T + 3) (3: `.data` 0x010c4414).
- Lateral error (`FUN_00505e60`): P = the path's Hermite point at arc length (car's projection + L); with h the
  car's heading in the ground plane, e = (pos + L·h − P) · (−h.z, h.x), positive when the aim point lies to the
  right of the path ((−h.z, h.x) is the car's right; checked in a memory snapshot against the body matrix).
- PID (`FUN_00436920`): e − lane offset clamped to ±20, integral clamped to ±20, u = −(Kp e + Ki I + Kd Δe/dt)
  with the integral from before the tick. Gains per car from `Vehicle_Gameplay.PIDController` (instance +0xb0:
  Proportional +0x18, Integral +0x14, Derivative +0x10; the 911 0.3 / 0.01 / 0.1; Kp 0.2–2, Kd 0.1–1.25, Ki
  0.01 across the cars). Steering = clamp(u, ±1) (positive = right), 0 with every wheel off the ground
  (`FUN_008f5350`).
- Lane offset: interpolated from the old to the planner's new offset at AIC[0x18] = 3 m/s (`FUN_0050b000`).
  The planner (`FUN_00502e20`) runs only with obstacle records: 32 offsets across the road scored against the
  obstacles (`FUN_004febe0`, inferred) plus bonuses (AIC[0x3c] 0.2 nearest the racing line, AIC[0x20] 0.1
  nearest the car, AIC[0x1c] 0.05 nearest the current target), kept 1.5 m inside the road. With nothing
  around, the offset is 0 and the car follows its path exactly.

**Target speed** (`FUN_00508d50`, confirmed): min(settings speed D+0x818 if ≥ 0, else D+0x5668), lowered to
0 when the narrow obstacle sweep along the path hits (`FUN_00503bf0`; the wide sweep only forces brake = 1;
sweep length min(AIC[0x44] 500, max(1.1 × speed, the stopping distance))), and by the ram mode. For an AI
racer the settings speed is −1 (`FUN_006c0700` builds the settings from defaults) and D+0x5668 is the rubber
band's schedule speed, FLT_MAX outside a race (the rubber-band search found the copy `FUN_00514590`; the
path-follower search listed only the init and the traffic substates as writers of D+0x5668: to be settled by a
live read). **A physical racer has no corner speed rule**: every access to
the path node's speed byte (+0x4d) and corner radius (+0x2c) was listed; the readers are the virtual-progress
paths only and `FUN_004e5270`, reached from the uncalled 0x004ff480 (a braking look-ahead); 0x004ff550 (speed
from the turn angle, AIC[0xcc..0xd8]: 130 → 80 mph over 10° → 80°) is uncalled too. On a clear road the
racer holds full throttle and takes corners by steering (the look-ahead shortens at tight corners) and the
rubber band's cap. A live read of the target speed (D+0x5808) and pedals during a race would confirm it.

**Pedals** (`FUN_004fa0e0`, confirmed): a speed PID (`FUN_004f8100`: Kp 0.6, Ki 0.05, Kd 0.5, `.rdata`
0x00fa8ecc..; error = speed − target and integral clamped to ±9) integrates a pedal once per tick: pedal =
clamp(pedal + 0.03 × u, ±1) (0x00fa8ee0); pedal > 0 is throttle, else brake. A target below 0.5 m/s under
10 mph pulls the handbrake, and below 0.5 m/s also brakes fully; airborne, no brake.

**Emergency brake** (`FUN_005095b0` → `FUN_00508070`, confirmed): above 70 mph (0x00fa8eb8), the cars within
r = clamp(speed × AIC[0x28] × 0.5, 40, 150) of pos + forward × (r − 10) are tested for a 2D circle-circle time
to contact (velocities = heading × speed; overlap = 0, separating = never; radii from the car's box); under
AIC[0x28] = 1.5 s the brake is forced to 1.

**Nitrous** (confirmed): `FUN_004ff620` only while speed + 2.7 m/s (0x00fa8f0c) < target and the driver's
nitrous mode is not 2. To start: tank fraction > AIC[0x140] = 0.25, speed ≤ the schedule speed − AIC[0x14c]
= 10 mph, and the larger of the left / right turn sums over the next speed × 1.5 m of path (`FUN_004e4eb0`)
under AIC[0x128] = 5°; to keep firing, under AIC[0x124] = 30°. State 8 adds `FUN_004fdb60`: on when the local
player is ahead along the AI's heading and the AI is below its schedule speed.

**Racer interaction** (`FUN_005050d0`, inferred meaning): with the nearest race car, mode 2 follows / rams a
car with a better race position close ahead (AIC[0x164] 20 m, AIC[0x16c..0x174] speed windows, at least
80 mph); mode 1 shunts (own speed 90–200 mph, after a random delay lerp(AIC[0x188] 0.2, AIC[0x17c] 1) s, lateral
gap within both half-widths + 5 m, |Δspeed| ≤ 15 mph): steering = clamp(AIC[0x198] 0.15 × the lateral gap, ±1),
into the other car. Only within −20 … +80 m of the local player. Not ported.

**Crash recovery** (confirmed): `FUN_004fb9f0` starts state 11 when the car's contact flags (VS flags 0x1 /
0x200 / 0x4000, meaning unknown) stay set over 1 s below 10 mph (cleared after 0.2 s free; not within
AIC[0x19c] = 10 m of the target). State 11 (`FUN_004eeba0`): steering = clamp(−4 × the heading error to the
path, ±1), mirrored in reverse; the pedal from `FUN_004f9e80`, a banded controller towards 10 mph (0 within
1 mph, full above / below by 9 / 1 mph, a ramp from 4 to 9 mph; its smoothing helper `FUN_005114e0` not read);
the direction flips when probe rays at ±25° (0.436 rad, `.data` 0x010c4234) hit or the car sticks again, at
most once a second; back to the previous state above 5 mph with the probes clear. D+0x5854 accumulates time
in state 11 (decaying at 0.25/s otherwise); past AIC[0x1a0] = 10 s, or AIC[0x1a4] = 3 s when an (inferred)
off-screen check passes, the driver asks for a reset (`FUN_0083d4b0`).

### The path (confirmed unless stated)

- Routes: an A* search on a worker thread (`"AStarWorker"`, `FUN_00528fe0`) over the **AIData** road graph's
  junctions (`FUN_00527300`: step cost = exit cost / 511 (= length / 200 m), plus a turn penalty
  5·max(0, 0.98 − trig(Δ))³ (`FUN_0052e250`; sin or cos unresolved), × 5 on shortcut exits and × 100 on jump
  exits when route options are set; heuristic the straight distance / 200), expanded back into nodes
  (`FUN_00524e20`). The format is in formats.md "AIData (0x200)".
- A race path (`FUN_005075e0` → `FUN_004edb80` → `FUN_004e7d80`) takes one node per route point (closer than
  0.15 m skipped), **resampled every 15 m** (`FUN_004e95e0`; at most 48 nodes in the 64-node ring at D+0x8a0),
  half-widths 5 m (`.rdata` 0x00fa8538), Hermite tangents 0.5 × (next − previous) (`FUN_004e6160`), side
  clearances from nav raycasts along ±the side vector up to 100 m minus 0.5 m (`FUN_004e58e0`), the node turn
  wrap(heading of the next segment − heading of the previous) (`FUN_004e6a10`).
- **A racing line**, not the centre line: `FUN_004ec380` → `FUN_004e70e0` relaxes per-node particles (0.99
  velocity damping, bending forces) within ±half the road width of the route points (`FUN_004e6e60`), 35
  iterations per tick for one driver per frame, until 10 iterations change less than 0.01 (or 3000); the force
  law is only partly read. Node +0/+4 hold the line, +0x10/+0x14 the route points. Not ported.
- Corner speeds (`FUN_004ea510` finds corners: a run of same-sign node turns passing 3° (0x00fa884c) and
  totalling 25° (0x00fa8848), its ends 3° in from either side, split at the middle node, each half ≥ 5 m;
  `FUN_004ea860`): per half, the chord L (the road's full width instead when that is over 1.5 L, 0x00fa8850),
  for each segment with Δ = |wrap(entry heading − segment heading)| > 0.01: R = (L/2) / cos((π − Δ)/2), the
  half's speed = clamp(AIC[0xc8] 1.8 × min(top speed, √(9.81 μ R)), 0, 255) m/s with μ = the rear tyres'
  `Pure_Lateral_Grip.Peak_Force_Coefficient` (2.99 on the 911) and the top speed from the aerodynamics data
  (`f1538c51`, 189 mph on the 911); no smoothing between nodes. Read only by the virtual progress.
- Braking distance (`FUN_00518520`, live in the obstacle sweep): d = 1.666 ln((a v0² + b) / (a v1² + b)) /
  (2a), a = (vehicle+0x688 × −vehicle+0x7d8 + vehicle+0x7d4) / mass, b = vehicle+0x688 × 9.81 (inferred: a grip
  μ, the lift and a drag coefficient; snapshot values 2.05 / −1 / 0.8 on a 1807 kg car); the writer of +0x688
  was not found.
- Nodes built straight from AIData (`FUN_004f2310`, the wander and traffic builders) take the node position and
  half-widths of drivable width × 0.125 m; no reader of the fast width (+0x24) was found.

### Rubber band (confirmed)

Manager at world+0x2f478 (`DAT_01177868`), frame function `FUN_00645c90` (one racer per frame, round robin
over player slots 12–23, opponent k = slot − 12), initialised by `FUN_0070caa0` → `FUN_0063a0f0` from
`OfflineEvent` +0x94 (`RaceBalancingData`) and +0x98 (`RaceBalancingProfile`), with a pace multiplier mgr+0xa0 =
the event's `AllowedVehicleList` entry `a17a5e72` for the player's car (1.0 … 2.3). Per racer:
- t (`FUN_0063ef30`): with lead = the player's remaining distance − the AI's (race tracker world+0x2f47c,
  +0xdf0) and e = `OpponentData[k]`: lead < `BehindDistance` (+0x14) → 0, lead ≥ `AheadDistance` (+0x10) → 1,
  else clamp((s − Behind) / (Ahead − Behind)) with s the straight distance between the cars signed by lead.
- f (`FUN_006432a0`): A = lerp(e+0x2c, e+0x28, t), B = lerp(e+0x20, e+0x1c, t); A before progress e+0x24, B
  after e+0x18, linear between. Progress x = 1 − remaining / total (`FUN_00654110`).
- Speed = f × `MaxSpeed` (`RaceBalancingProfile` +0x10, cracked name) × pace × 0.44704 for a physical car;
  then × (1 + K clamp((race time − expected time) / 30, ±1)) with K = 0.1 (0.2 virtual) (`FUN_006433f0`), the
  expected time from the route's segment speeds (`FUN_00642da0`, `FUN_00661eb0`) plus 3 s per player crash
  (RBD +0x10) and an offset (RBD +0x14). Pushed to D+0x5668 every frame (`FUN_0083c820` → `FUN_00514590`).
- It is a **target-speed cap**, not a grip or torque multiplier. Ported without the time correction.

`AIPlayerType` (GAMELOGIC/GAMEPLAY.BNDL, 200 types; the racer personality copied by `FUN_006c0700`): its
speed caps (0x78 / 0x7c, 90 mph) are read only by non-racer behaviours; nitrous usage (u16 0xaa) has no
reader; the car comes from its rollout (0x14). Not used yet.

### Model decisions (ours, `ai.rs` / `ai_driver.rs`)

- The path: the road-graph centre line through AIData nodes (a Catmull-Rom spline), not the game's relaxed
  racing line; the inside clearance is the node's drivable half-width, not a nav raycast; no lane planner (the
  offset stays 0, as the game's does on a clear road).
- Race routes: the shortest node path (A* on link lengths, no turn penalty or exit factors) between the start
  and every gate, laps unrolled; test routes (`--ai` without `--race`): from the player's spawn, the
  best-aligned neighbour at every node (`FUN_00436c00`'s rule) for 3 km.
- Opponents' cars: the event's allowed cars in turn (the opponents' own cars, `AIPlayerInformation`
  `9b4e35f7` / `2968e77a` and the type's rollout, are not resolved).
- The emergency brake's circle radius = the larger horizontal half extent of the body.
- The recovery's probe length 6 m; the banded pedal's shape between its read thresholds; the reset puts the car
  on the path at its progress (the game's reset placement not traced); the off-screen 3 s reset is not used.
- `MW_AI_CORNER_SPEED=1`: applies the game's uncalled braking look-ahead (0x004ff480) on the ported corner speeds,
  with the braking distance's aero term left out (a = 0). Off by default, as in the game.
- Not ported: the racer interaction (side swipe / ram), the virtual progress, the time correction of the
  rubber band, the slipstream.

## Vehicle paint, liveries and dirt

The paint shaders' object constants are registered by name with slot numbers in one table function in the 0x8c4xxx range (a run of `FUN_00a5d490` calls; slot → name: 0x29 `VehicleID`, 0x2a `PbrPaintDiffuseColour`, 0x2c `PbrPaintSpecularSettings`, 0x2d `PbrPaintClearcoatSettings`, 0x2e `PbrPaintScratchColour`, 0x2f `PbrPaintScratchSettings`, 0x30 / 0x31 secondary diffuse / specular, 0x33 `gvDamageVolumeParam`, 0x34 `g_damageConstants`, 0x35 `g_selfIlluminationMask`, 0x37 `g_PerVehicleFog`, 0x49 `g_SelfIllumLocal_RecipRadiusSq`, 0x9c `GenericVector4_A`, B / C follow). They are per-object constants, written through `FUN_00a621a0(slot)` in `FUN_008e9c00` from a render record (stride 0x2b0) and its paint block `P` (record word 0xa2 = the render instance + 0x20).

**The paint block** (read in a memory snapshot of the running game, `re/out/live/snap1`, render instance 0 = `*(*(0x10edae4) + 4) + 0x3050`, instances 0x1c40 apart, and matched field by field to the car's `VehiclePaint` 0x163210): `P+0x10` = `PbrDiffuseColour`.rgb with `PbrSpecularRoughness` in w, `+0x20` = `PbrSpecularColour`.rgb with `PbrSpecularFresnel` in w, `+0x30` = `PbrScratchColour`, `+0x40` / `+0x50` = the secondary paint's, **`+0x60` = `DustColour`, `+0x70` = `DirtColour`** (corrected: the 2026-10-06 note read them the other way round without checking), `+0x80..0x88` = (`PbrSpecularRoughness`, `42e05e51`, `PbrSpecularFresnel`), `+0x8c` = `PbrScratchRoughness`, `+0x90` = `0802d981`, `+0x94` = `PbrClearcoatRoughness`, `+0x98` = `a1d177bd`, `+0x9c` = `PbrClearcoatFresnel`. So 0x2a / 0x2b = primary diffuse / specular, 0x2e = scratch colour, 0x2f = (`PbrScratchRoughness`, `0802d981`) = the scratch's roughness and F0, 0x30 / 0x31 = secondary − primary, **0x9d (`GenericVector4_B`) = DustColour, 0x9e (`_C`) = DirtColour**, 0x9c = (DustColour.w × record word 0x28, DirtColour.w × word 0x29, word 0x2a, word 0x2b).

**The record's levels** (`FUN_00583290`, which builds the records from the render instances): word 0x28 = instance word 0x6f6 × 2 (`.rdata` 0x00fac280), word 0x29 = instance word 0x6f4 × 3 (0x00fabaf8), word 0x2a = a global from `FUN_00897bc0` (inferred the rain-drop amount: the main paint program scales its `GenericSamplerB` drop normals by `GenericVector4_A.z`), word 0x2b = 1 / the function's float parameter (the drops' UV scale, `A.w`); words 0x24..0x27 (`g_damageConstants`) = 0. Each frame `FUN_00591ac0` copies the instance's levels from the vehicle: word 0x6f6 = vehicle +0x2c80 (**dust**), word 0x6f4 = vehicle +0x2c84 (**dirt**).

**Build-up** (`FUN_009d19c0`, `this` = vehicle +0x2c80; called each physics tick from `FUN_00904da0` → `FUN_009d6e30`, before the skid-sound values `FUN_009d1bf0`; earlier read here as the surface rumble). The writers were found by scanning the snapshot's decrypted `.text` for the displacement 0x2c80 (`lea ecx, [reg+0x2c80]` before calls at 0x008fd7df, 0x008fdf99, 0x00905059): the Ghidra dump shows the sub-object only through `this`. For each of the four wheels (vehicle +0x890 + i × 0x1d0: surface id = bits 13–19, on-ground byte at +0x894), when the surface's current weather block has a `VehicleEffects` object (`FUN_0054a840`: the `Wet_Conditions` block when the surface manager's state +0x10 ≥ 1 and the render instance's byte +0x1c17 is clear, else `Dry_Conditions`; formats.md "SurfaceVehicleEffects"):
- t = clamp((|v| − `f8bbba83`) / max(`0d9ece32` − `f8bbba83`, 2⁻²³)), v = vehicle +0x1620 (mph), so 30 → 120 mph on most surfaces;
- dirt (+0x2c84) += max(w₀, w₁, w₂, t) × dt / max(`Dirt.0979a504`, dt) × `Dirt.ea1c1262` / 4, clamped to 0..1, with w the wheel's three values at this +0x10 + i × 0x30 (smoothed per-wheel skid ramps written by `FUN_009cef90` from a tyre-sound-like parameter object, not ported);
- dust (+0x2c80) += t × dt / max(`Dust.0979a504`, dt) × `Dust.ea1c1262` / 4, clamped to 0..1.
A negative rate washes the car (surfaces 27 and 43: −0.5, 15–60 mph). At 120 mph on grass (0.3, 5 s) each level gains 0.06 / s; on dry asphalt only dust grows (0.05 → 0.01 / s at full speed). `FUN_009ce7b0` (car spawn) starts both at the render instance's words 0x1bd8 / 0x1bd0 (zeroed by `FUN_00586810`).
- **Set and wash**: message 0x13 of the physics queue (`FUN_00919240`) sets +0x2c80..+0x2c8c to one value (sent by `FUN_00589770`, which also sets the instance's levels; no direct caller), message 0x12 zeroes them. `FUN_0058f060` (a vtable method: zeroes the 15 damage-grid values, `FUN_009b2430(1)`, posts 0x12 and 0x11) is the repair; that the gas stations reach it (their handler `FUN_00748ff0` posts manager messages 0xc and 0x16) is inferred.

**Scratches** (`VehicleNFS13_BodyPaint` vertex program, the damage variant): TEXCOORD5.w = max(Σ bilinear `gavDamageVolumeValues[row × 3 + col].x`, `g_damageConstants.x`) at (col, row) = `gvDamageVolumeParam.xy` × (10 × POSITION.xz) + `.zw`, the unskinned bind-pose position (POSITION is R16G16B16A16 Snorm, × 10 = metres). The pixel program (longest `ps_4_0`): s = (1 − 2 |e(uv × (2, 3)).z − 0.5|) × sat(2 e.z − 1) × TEXCOORD5.w with e the effects map at TEXCOORD2; the colour lerps to `PbrPaintScratchColour` by s, roughness to `ScratchSettings.x`, F0 to `ScratchSettings.y`, the clear coat's reflection × (1 − s)(1 − coverage)⁴; the same weight blends the `CrumpleTextureSampler` normal (TEXCOORD1) into the normal and its alpha into the AO (not ported). `FUN_0057e840` sets `gvDamageVolumeParam` = (2 / (max.x − min.x), 4 / (max.z − min.z), −min.x × that, −min.z × that) from the render instance's box (+0x140 / +0x150, `FUN_0057c120`: the model's mesh boxes); the snapshot's car: (0.889, 0.786, 1.0, 2.005), a 2.25 × 5.09 m box. `FUN_00585030` (from `FUN_00591ac0`, when the vehicle's +0x1181 flag is set) fills the 15 values from the vehicle's eight zone levels at +0x112c..+0x1148, each capped at 0.99 (`.rdata` 0x00f9cad4) while the car is not wrecked (damage bar byte +0x1180 below the profile's segment count), times `VehicleInfo.76bfd668` (+0x84 of the object the instance's vtable +0x34 handle leads to, read in the snapshot: 10), capped at 1. Rows run from min z (back) to max z, columns from min x (the car's right) to max x: back row twice (+0x113c, +0x1140, +0x1144), middle row (+0x1138, min(+0x1148, +0x1138), +0x1148), front row twice (+0x1134, +0x1130, +0x112c). The zone order on the vehicle is therefore front-left, front, front-right, right, rear-right, rear, rear-left, left (+0x112c …), consistent with `FUN_005b1680`, whose zone switch (cases 1..8) reads +0x1130, +0x1140, +0x1148, +0x1138, +0x112c, +0x1134, +0x1144, +0x113c: front, rear, left, right, front-left, front-right, rear-left, rear-right, our damage clip order (inferred: the enum's names are not in the dump). Ported in `paint_wear.rs` / `layered.wgsl`.

Liveries and two-tone paint: research log 2026-10-06 (session 5, rounds 3 and 4). Which `ColourGroup` of the car's `VehicleColourPalette` supplies the secondary paint is still open (the code that fills `P`).

## Nitrous and upgrades

- **Profile index → field** (2026-10-06, the call sites of `FUN_00574870`; table `mw_assets::mods::PARAMS`,
  printed by `mwtool mods`). Field offsets were matched against `mwtool genesys-types` (nested struct fields
  sit at struct offset + field offset) and names checked by hash:
  - 0: engine torque scale: `FUN_009cd8f0` folds it into a scale on 1.0; `FUN_009d7110` multiplies three
    `Engine.Power_curve` values (`8c227e4d`, `91c58956`, `12ad185d`, the resource beside the gearbox) by it.
  - `FUN_009d7110` / `FUN_009d2740` (gearbox, +0x248, identified by its `uint16` gear count at +0x50 and the
    per-gear loop): 20 `Gear_change_time`, 38 `Differential` (final drive), 39 each gear's `Gear_down_RPM`,
    40 each gear's `Gear_up_speed`.
  - `FUN_009cf6a0` (`SteeringBehaviour`): 2 `High_speed_counter_steer_rate`, 3 `Reverse_speed`, 21
    `High_speed_center_steer_rate`, 31 `High_speed_steer_rate`, 32 `High_speed_steering_range`, 36
    `Reverse_steering_range`, 37 `16fd6022`.
  - Drift (`FUN_009d12b0`, `FUN_009d1350`, `FUN_009d1410`, `FUN_009d0200`, all re-run by `FUN_009d79a0`):
    34 / 47 / 46 `Drift_trigger.8e4032c0` / `0dce38f7` / `8f151151`; 35 / 43 / 42 / 44 `Side_force.09832a93` /
    `f6c50d59` / `ff5dcf85` / `df1c4fa8`; 45 `Other.30580747`; 28 `Other.55930ac1` and `de67c45d`; 26
    `Yaw_torque.179fe51e` and `4cad24de`; 27 `Yaw_torque.e324fa82`; 1 the computed yaw torque in `FUN_009d0860`.
  - Not identified (call site known, field not traced): 4–7 (`FUN_005d4970`-area scales on 1.0), 8–13 and 19
    (values read from `ushort` / `byte` data), 14–17 (interpolated pairs in the 0x0057xxxx file), 18, 22–25
    (vehicle +0x560 / +0x588 / +0x680 / +0x6a8, `FUN_008f77f0`-area), 29, 30, 33 (`FUN_0057e1d0`, scale on 1.0),
    41, 48–54 (a per-state switch, 0x005Cxxxx).
- **Categories**: category 0 holds the "PRO" names (e.g. `LONG GEARS PRO`), category 1 the base level of the
  same slots. Unlocks (won events) are not modelled: every mod is available (divergence).

- **`FUN_009d2aa0`** (update step 5, called from `FUN_00904260` with the controls' nitrous lanes +0x1c20..+0x1c2c
  and the mode +0x1c60, both written by `FUN_009aacc0` from the player's control record (`FUN_005734c0(i)` +0x1c
  amount, +0x33 mode) in `FUN_0091c5c0`). `this` holds [0] the engine owner (handling resource at +0x240/+0x244;
  `Engine` → `Nitrous_strength` at engine +0x118) and [1] the vehicle. It resets a four-lane multiplier at
  `this`+0x10 to 1 (`.rdata` 0x01026060 = 1,1,1,1), then:
  - vehicle +0x30e8 set (a special state, not identified): multiplier = `Engine.d00887ca` (engine +0xf0, 1 on the
    911); with amount > 0 and a wheel down (`FUN_008f5350`: not all four wheel flags at wheel +0xa30, stride 0x1d0,
    clear) a forward force clamp(R+0x14 − mph, 0, 10 (`.rdata` 0x010261b0)) × mass (vehicle +0x10 → +0x7c) ×
    amount, R = the resource at vehicle +0x7c4/+0x7c8.
  - mode 0: multiplier = lerp(1, `Nitrous_strength.e9de413e` (+0x3c), amount) per lane.
  - mode 1 (punch): multiplier = the strength; with a wheel down a forward force (1 − clamp((mph − `Punch.5cb7c5aa`)
    / (vehicle +0x7e8 − `Punch.5cb7c5aa`), 0, 1)) × amount × `Punch.0798d257` × `.bss` 0x0118a018 × mass.
  - mode 3 (jump): airborne and not in state +0x1774: the same force with `Jump.0798d257` / `Jump.b959e02f`;
    multiplier lerp(1, `Jump.e9de413e`, amount).
  Forces go through `FUN_009076a0` (body-frame vector rotated by the body matrix, added to the force
  accumulator at body +0x70); `FUN_00907840` finishes. 911 values: strength 1.9, Punch {40, 100 mph}, Jump {20,
  1.6, 100 mph}.
- The tank and its refill are traced in "Nitrous refill" below (2026-10-09). Its data is
  `Genesys.Gen.NitrousParameters` (several variants in each vehicle bundle).
- **`FUN_00574870(index, value)`**, the upgrade profile: `this` holds 50 entries {f32 operand at +8i, op at
  +8i+4}, a parameter index per entry at +400+4i, and a 64-bit active mask at +700 (two u32). For each active
  entry whose index matches: op 0 multiplies, op 1 adds (summed), op 2 returns the operand at once; the result is
  value × Π + Σ (identity with no entries). Call sites pass indices 0x00..0x36 as literals; which handling field
  each index belongs to was not mapped this session (open: read each call site).
- **The mods' data**: `Genesys.Gen.Vehicles.PerformanceUpgrades` in each vehicle bundle (911: `…6a59f`), two
  `Category` fields (`cb186d88`, `6eeb691f`; their names in Language ids differ by one, so most likely the pro
  and base levels), each up to 18 slot fields holding a `ModifierUpgrade` {`Icon`, `Name` (Language ids),
  `Modifier`: `PerformanceModifier` {`Modifications`: [`PerformanceModificationItem` {`Value`, `566a7ed1`
  profile index, `ModificationType` 0 / 1 / 2}]}}, a `NitrousUpgrade` (`5b7038f6`: the `NitrousParameters`
  variant it selects) or a `TyreUpgrade` (`a316f8e3`: tyre index). `mwtool mods <car>` prints them.

## Nitrous refill

Traced 2026-10-09 (`mw_handling::nitrous`, `mostrusted/src/nitrous.rs`). Times are countdowns in ticks of
3e6 per second (`.rdata` 0x00f9f604), run out below 3 ticks (`FUN_0064cd40`); speeds are mph: |vehicle
+0x120| × `.bss` 0x0118a090 (2.23694) for the events, the signed vehicle +0x1620 for the start.

- **State**: a 200-byte record per player at game +0x264f0 (game = `[0x1177868]`, the nitrous system at
  +0x264e0; `FUN_00658c90` clears it to 0, also through the `ClearNitrous` script command). +0x00..+0x0c one
  "active" byte per refill event (the HUD's `NitrousViewModel` `IsOncoming`, `IsSlipStreaming`, distances and
  near-miss counts come from them, `FUN_00663610` / `FUN_00666030`), +0x10 the amount (0..100, `.rdata`
  0x00fb77e4), +0x14 the amount added this tick, +0x18 the control amount, +0x1c seconds since the burn
  started, +0x20 its distance, +0x28..+0x58 the event countdowns, +0x60 the minimum-burn countdown, +0x68 the
  amount at the start, +0x6c the mode (0 idle, 1 nitrous, 2 punch, 3 speedbreaker, 4 jump), +0x70 the usage
  block, +0x74 / +0x75 the near-miss window and its cancel flag, +0x76 a takedown this tick, +0x78 / +0x90 the
  near-miss car ids (10 each) with counts at +0x8c / +0xa4, +0xa8 the crash escapes seen, +0xb0 / +0xb4 /
  +0xb8 the balancing multipliers (high-speed refill, burn, every award; 1 unless `EnableNitrousBalancing`
  (`FUN_00663190`, `Balancing` block: lerp by the distance to the leader), +0xbc a second burn multiplier (1),
  +0xc0 infinite nitrous (amount held at 100). Every award goes through `FUN_0066e030`: amount × +0xb8, added
  to +0x10 and +0x14, clamped to 0..100. The HUD's `NitrousAmount` is +0x10 × 0.01 (`FUN_00658a90`).
- **Variant** (`FUN_0065de50`): an event override (`FUN_0068b380`), else the fitted nitrous mod (mod slots 3,
  4, 12, 13 at levels 1 / 2: `PerformanceUpgrades` handles at +0x88..+0x94 / +0x34..+0x40), else
  `Vehicle_Gameplay.Nitrous_Online` when the event is online (`[[0x10edb58]+4]+0x6a20` ≠ 0), otherwise
  `Nitrous_Offline`. Every car's `Nitrous_Offline` (911 `…6a338`) has all four usages disabled and
  `Restrictions` {100, 300, 300}: **a car without a nitrous mod has no nitrous offline**.
- **Input** (`FUN_006cd3e0`, per player): the nitrous action's state 2 (taken as "pressed this frame", 1 as
  "held") starts the first enabled mode (`FUN_00662e30`: `NitrousUsage.Enabled` → 1, `PunchUsage` → 2,
  `SpeedbreakerUsage` → 3, `JumpUsage` → 4) if the mode is 0 and: mode 1 (`FUN_00662a50`) amount ≥
  `Restrictions.c441f502` and |mph| ≥ `Restrictions.d5b487c7`; mode 2 (`FUN_00662b40`) amount ≥
  `PunchUsage.fd8a5ed5`, same speed; modes 3 / 4 (`FUN_00662c30` / `FUN_00662d00`) the speed only. The start
  (`FUN_00662ea0` / `FUN_00662f20`) keeps the usage block and sets the minimum-burn countdown to the block's
  word +0x1c (`NitrousUsage.7ebf362a` = 0.1 s; for the punch `fd8a5ed5`, 50 / 100 s, so a punch runs until
  the tank is empty). Not held (neither state): the mode drops to 0 once that countdown has run out
  (`FUN_00658a40`). `fc79a57e` is `Enabled`; `ddfca712` was not seen read.
- **Burn** (`FUN_00669ab0`, after the events): mode ≠ 0 (and for the punch only once +0x1c ≥
  `PunchDelay` `a82e62ca`): amount −= usage `053c5a6a` × dt × +0xb4 × +0xbc, ≥ 0; the control amount +0x18 =
  clamp(+0x1c / usage `845c3879`, 0, 1) while amount > 0 (`845c3879` = 1 s: **the nitrous ramps in over a
  second**; 0 for the punch: instant after the first tick); then +0x1c += dt; empty → mode 0. Idle: +0x18,
  +0x1c, +0x20 = 0. The control block (`FUN_006c1a80`) takes +0x18 as the nitrous amount (+0x1c) and the mode
  (`FUN_00658b50`: 1 → 0, 2 → 1, 3 → 2, 4 → 3) and, with any mode on, sets the **throttle to 1** while the
  amount is above 0 or the signed speed is under 15 mph (`.rdata` 0x00fc397c).
- **AI racers** (player kind +0x9c = 2) skip the events: a constant refill `AIConfig` +0x10 × dt × +0xb8.
- **Events** (`FUN_00667c10`, a 13-way switch; block offsets in `NitrousParameters`; "awards" = only when the
  block's `aeba495e` is set; `4d907940` is in every block but not read):
  - 0 MinSpeed (+0x16c): speed > `d629c55f` → `d63cff99`/s. `aeba495e` is false in every variant.
  - 1 HighSpeed (+0x14c): speed / the vehicle list's top speed (entry +0x3c, 189 mph on the 911) × 100 >
    `21948e6c` (35) → `d63cff99`/s × +0xb0.
  - 2 TakingShortcut (+0x1ac): speed > `d629c55f` (70) on a `RoadData` road with flag bit 1 → rate. No road has
    the bit and `aeba495e` is false everywhere: never fires.
  - 3 TrafficOncoming (+0x0c): speed > `d629c55f` (20) and in an oncoming lane (`FUN_0065eec0`) for
    `1732c75e` (0.25 s) → `d63cff99`/s; after leaving the lane it keeps paying for `d6eeeb05` (0.5 s), or 3 s
    (`.rdata` 0x00fb865c) when off the road network.
  - 4 Slipstreaming (+0xe8): 100 × the slipstream amount (racer vehicle +0xc60) > `efb8084e` (50) and speed >
    `d629c55f` (50) → rate.
  - 5 Drifting (+0x7c): in a drift (drift +0x14, the vehicle's virtual +0x34) at speed ≥ `d629c55f` (30) for
    `TimeDrifting` (`f003dfe5`, 0.75 s) → rate.
  - 6 Donutting (+0x58): the racer vehicle's virtual +0x6c (`FUN_00572860`: in a drift and drift +0x80 ≤ 0.95,
    `.rdata` 0x00fad860; drift +0x80 = 1 − the yaw torque's blend from `Donut_speed_limit_low` to `_high`,
    0 while counter-steering, `FUN_009d0860`) at speed ≤ `02318b5f` (30) for `959a0ec3` s → rate.
  - 7 CatchingAir (+0x10c): vehicle +0x1630 (time in the air) > `c8e9863d` (0.1 s) → rate.
  - 10 HittingAnotherCompetitor (+0xa0): speed > `d629c55f` (50) and the car's contact (`FUN_00658e80`) is a
    player of kind 2 for `1732c75e` (0.05 s) → `d63cff99` once per contact.
  - 11 FatalHit (+0x20c): +0x76 set this tick (`FUN_006b8d00`, from `FUN_006ccd20` when a car is wrecked and
    the player responsible has kind 0: a takedown) → `d63cff99` (100) at once.
  - 12 CrashEscape (+0x12c): speed > `d629c55f` (50) on the tick the stats counter (game +0x2f3ec → +0x2c,
    `FUN_00656a90`) goes up → `d63cff99` (100). The counter counts cars of the list at +0xa664 (inferred: the
    police) that crash within a range and ahead of the player without being its takedown (`FUN_00748150`).
  Near misses (8, 9; `FUN_00665f30` → `FUN_0065e280` → `FUN_00663270`): unless the car touches a traffic car
  (`FUN_006564c0`, which cancels the window until a query comes back empty), a world sphere query: centre
  the vehicle position (+0x110), radius 5 m (`.rdata` 0x00fb82b0 splatted into `.bss` 0x011777a0), broadphase
  category 8 (traffic only), each car's bounding sphere (2.3–2.6 m) against it, answered a tick later. Each
  car is "same direction" when its forward axis · our velocity ≥ 0, else oncoming; distinct ids per window,
  10 each. The first car opens a 0.5 s window (`.rdata` 0x00fb8660 = 1.5e6 ticks); when it closes, each kind
  with a count > 0 awards its `d63cff99` once (2.5 / 3 / 4.5 and 5 / 6 / 9) if the speed is above its
  `d629c55f` (80 mph both).
  Fixed awards: 100 at a gas station (`FUN_00748ff0`, an object of type 6 in the car's contacts) and 10 per
  smashed gate in free roam (`FUN_0074cf40`), both through `FUN_00658ad0`.
- **Road position** (`FUN_00668650` → `FUN_00666200`, one of 24 player slots per tick): road, section and
  segment on `RoadData` (formats: `mw_formats::road_data`): the first road and section (file order) whose
  boxes contain the car and whose Hermite segment (`FUN_00a3ba80`, basis from `.bss` 0x0118a640) the car is
  within (max radius + 1 m, under 5 m in height; `FUN_006592b0`, `FUN_0065f410`, `FUN_006638f0`). The oncoming
  test (`FUN_0065eec0`): the car projected on the segment's chord, the curve's point and tangent there, the
  lateral offset along normalize(tangent × up); (byte +0x1c, +0x1d) when the forward axis · tangent ≤ 0, else
  (+0x1e, +0x1f) of the segment's first point; oncoming when lo < offset < hi (metres).
- **Slipstream** (`FUN_0063c790`, per player): both faster than 50 mph (`.rdata` 0x00fb6330 × `.bss`
  0x0118a010), the other 1 to 80 m away (0x00fb6260, 0x00fb6340), in sight (a world ray, offline), and the
  direction to it within acos 0.95 of our velocity (0x00fb6350): best = max of (direction to it · its velocity
  direction); amount = clamp((best − 0.95) / 0.05, 0, 1) (0x00fb6360), stored per player and copied to the
  racer vehicle +0xc60 and the physics vehicle +0x1dd0.
- The HUD messages are the blocks' `Message` Language ids: AIR 24009, DRIFTING 24010, NEAR MISS 24011,
  ONCOMING NEAR MISS 24012, ONCOMING 24013, TAKING SHORTCUT 24016, SLAM 24017, SLIPSTREAMING 24018, CRASH
  ESCAPE 955550, TAKEDOWN 1660283, DONUT 1007667.

## Vehicle animations

The variable names the animation state machines read sit together in `.rdata`: `brake` 0x00fae7e0,
`steering` 0x00fae7e8, `speedinmph` 0x00fae7f4 (then `coplights`, `Rear_Bumper_Hinge`). No function of the
dump references them by address (searched for the addresses and `s_` labels; probably reached through a
table built at start-up), so the code that feeds them and evaluates `HSM.AnimationCondition` is not found
yet. Until it is, the rule in `car_anim.rs` comes from the data only (the HSM's condition strings and
operands, docs/formats.md "Vehicle animations"): a one-clip `speedinmph` machine deploys at the condition's
speed and retracts below its other operand (911: 70 / 40 mph, LFA 70 / 50), playing the clip at its frame
rate both ways. Whether a deployed wing changes the handling is open: `AnimationStateBehaviour` has no aero
field, and no write from the animation side into what `FUN_009cf850` / `FUN_009cfaf0` read was found (the
search could not start without the condition code).

## Tyre marks

Not found. The skid mark code would read `TyreVfxBehaviour.{Front,Rear}_Tyres.Skid_Marks` (stride 0x130 per axle,
`LatParams` floats at +0x10..+0x2c) and `SurfaceVfx.TyreMarksMaterials` (+0x18 + 8 x tyre class); none of these
offsets, the field hashes, or a tyre-mark string turns up in the dump (research log 2026-10-06 round 4). The
runtime borrows the per-wheel form of the skid sound intensities (`FUN_009d1bf0`, above) as a model decision.

## Cameras

**Found (2026-10-06, session 5 round 5); the followed frame ported in round 6 (below).** The chase camera's code and the way it reads
`CameraGameplayExternal` are located below; the per-rule reading is only started (the yaw-spring helper).

### The camera director and its 15 cameras

- The camera manager (`*(DAT_010edaf4 + 4)`, `DAT_010edaf4` = game object + 0x6a968) holds a 0x3050-byte
  **camera director** at manager + 0x6a0 (only slot 0 is used: `mgr + 0x400` is reset to 0 before each call).
  Its constructor is `FUN_00422640` (no direct caller in the dump; the director's own vtable is `0x00f99d14`,
  whose method +8 is `FUN_005d5e20`, the call `FUN_005d2cd0` makes each frame; how it picks the active
  sub-camera is not read yet). The manager's
  `FUN_005c7060` only zeroes the director's first 16 bytes; `FUN_005c44b0` (after loading `CAMERAS.BNDL`)
  calls director +4, `FUN_005c4580` director +0x10, `FUN_005ce510` director +0xc, `FUN_004bc620` and several
  HUD / AI callers director +0x28 (a "which camera is active" query, by its use).
- `FUN_00422640` stores 15 camera sub-objects' vtables. Offset in the director, vtable, and the update
  (vtable +8; read from `.rdata` with `exe-const`):

  | offset | vtable | +8 update | size | note |
  |---|---|---|---|---|
  | 0x120 | 0xf99954 | `FUN_005d2f60` | 1739 | vehicle-attached (same helpers as 0x520) |
  | 0x520 | 0xf99984 | `FUN_005d3630` | 1692 | vehicle-attached |
  | **0x8d0** | **0xf999b4** | **`FUN_005d4970`** | 3728 | **the gameplay chase camera** (below) |
  | 0x10a0 | 0xf990fc | `FUN_005caeb0` | 6438 | box / orbit camera round the car (8 corner points) |
  | 0x1270 | 0xf99128 | `FUN_005cace0` | 457 | |
  | 0x1400 | 0xf99154 | `FUN_005c98e0` | 101 | |
  | 0x1570 | 0xf999e8 | `FUN_005c9a70` | 1941 | script-driven (channels `FUN_004962f0(n)`), FOV `FUN_005deca0`, `FUN_005df170` |
  | 0x1980 | 0xf99180 | `FUN_005d2060` | 581 | |
  | 0x1c90 | 0xf991ac | `0x5c02b0` | - | not a function start in the dump |
  | 0x1f30 | 0xf991d8 | `FUN_005ca490` | 1393 | |
  | 0x22e0 | 0xf99204 | `FUN_005cf770` | 2385 | script-driven, same helpers as 0x1570 |
  | 0x2830 | 0xf99230 | `FUN_005ca410` | 57 | |
  | 0x2950 | 0xf9925c | `FUN_005c5bc0` | 3991 | script-driven, same helpers as 0x1570 |
  | 0x2b00 | 0xf99288 | `FUN_005ce820` | 1277 | |
  | 0x2dc0 | 0xf99a14 | `FUN_005c6c10` | 667 | |

### How the chase camera reads its data

Every camera keeps a pointer at camera + 0x290 to a per-player camera state; that state holds the
`CameraGameplayExternal` resource at +0x98 and the instance's byte offset at +0x9c, so a field is read as
`*(float *)(*(*(state + 0x98) + 0x28) + state[+0x9c] + OFF)` (often with the sub-struct's base computed first,
`base + 0xbc + idx`, then `+0x10..+0x28`). This double indirection is why the round-4 search for
`res + 0x28 + OFF + idx` found nothing. Functions with this read: `FUN_005c90a0` (yaw spring, 0xbc),
`FUN_005c9380` (camera roll, 0x90 and 0xac), `FUN_005c7a80` (gradient adjustments handle 0x178),
`FUN_005c7340` / `FUN_005c7410` (globals handle 0x174), `FUN_005d0b90` (0x28 and 0x2c..0x38 in
`Speed_based_movement`, globals + 0x44). Other state fields: +0x84 a mod / profile handle
(`FUN_00574870(0x29, …)`, the mods getter of decomp.md "Nitrous and upgrades"), +0x4 / +0x20 passed to the
final stages.

### `FUN_005d4970`, the chase camera's update (order of calls)

Target car = `*(cam + 0x110) + 0x34`; speed read at vehicle + 0x1620 (line ~301); time step from
`FUN_0046ee10` (+0x10 × +0x14). Then: `FUN_005c1b90`, optional `FUN_005c1c30` (flag cam + 0x712),
**`FUN_005c90a0` (yaw spring modification)**, **`FUN_005c9380` (roll)**, `FUN_005c8420`, `FUN_005c82c0`,
`FUN_005c3af0`, `FUN_005c8820`, `FUN_005c2070`, `FUN_005c89c0`, `FUN_005c75b0`, **`FUN_005c7a80`** (2108
bytes; gradient adjustments, a spring on cam + 0x670..0x67c), `FUN_005c76a0`, `FUN_005c8ad0`, `FUN_005c1f90`,
`FUN_005c8880`, `FUN_005c2130`, `FUN_005c8e40`, **`FUN_005d0b90`** (1817 bytes: speed-based movement, impact
shake via `FUN_005cd040` with globals + 0x44, `FUN_005e05a0`, then `FUN_005dfe90`, a 1580-byte stage also called by
the scripted cameras, by the impact shake `FUN_005cd040` and by `FUN_005c0c80`, so more likely a transform
composition than the collider, and `FUN_005c0c80` which writes the result). `FUN_005c90a0` and `FUN_005d0b90`
have `FUN_005d4970` as their only caller. Where the world / car avoidance happens is not located yet: the
candidates are the unread helpers `FUN_005c8420` (1018 bytes), `FUN_005c9380`'s tail, `FUN_005c2130` (1119)
and `FUN_005c76a0` / `FUN_005c8ad0` (982 / 879). After `FUN_005d0b90` come `FUN_005c0550`, `FUN_005c7580`, `FUN_005c7340`. It also stores the car's frame into cam + 0x4d0..0x50c.

### Yaw spring modification (`FUN_005c90a0`, read)

Round 7: nothing more resolvable from the pseudo-C (the blend local `f`, `mods(0x29)` and `_DAT_0110e8f0`
need the disassembly or a live read); our speed blend 0.075 → 0.05 stays a model decision.

`Yaw_spring_modification` sits at 0xbc; its seven floats at sub + 0x10..0x28 are, in stored order, the hashes
`19c1db65`, `78527964`, `d48a307f`, `63fb2207`, `397fce82`, `2517d995`, `5a74d44d` (which of them are
`Low_speed` / `High_speed` / `Decay_rate` is not settled; call them y10..y28 by sub-offset). Per frame:
- `ω` = the car's angular velocity (body + 0xc0..0xc8, zero when body flag +0xf4 bit 0 is set) rotated into
  car space by the matrix at vehicle + 0x2db0; the yaw component `|ω_y|` is used.
- `t = clamp(1 − (|ω_y| − y20) / (y14 − y20), 0, 1)`: 1 below the low yaw rate y20, 0 above y14.
- the state at cam + 0x6d8 moves towards `t` by at most `y10` per call (a rate limit, not scaled by dt in
  this function).
- a speed factor `s = clamp(|speed| / v_ref, 0, 1)` (lane 0 of a vector `|(speed, ω_car)|` built as
  `max(−x, x)` with `_DAT_00fb05a0` = −1 from `.rdata`, divided by the vector `_DAT_0110e8f0` (`.bss`, needs a
  live read: the reference speed) and clamped by `_DAT_00fb05f0`), then
  cam + 0x6d4 = `lerp(lerp(y28, y18, f), mods(0x29), s)`, where `mods(0x29)` is `FUN_00574870(0x29, state +
  0x84)`, the car's value at index 0x29 of its mods profile. `f` is a stack float the decompiler never shows
  being written (an aliasing loss); by its place it is most likely the rate-limited `t` at cam + 0x6d8, which
  would make the spring y28 at high yaw rate and y18 at low, but that needs the disassembly or a live read.
  Read part: the yaw spring depends on **the car's yaw rate** (not only speed) through a rate-limited blend,
  and is handed over to the mods-profile value as speed rises, which our model does not do.

### The followed frame: per-axis angle smoothing (round 6, read and ported)

The core of `FUN_005d4970`'s "sway" is not a position spring but a smoothing of the car's orientation as
three Euler angles, in this order inside the update:
- **Inputs.** `|speed|` (vehicle + 0x1620, m/s) is broadcast to a 4-lane vector `S` (`max(−x, x)` with
  `_DAT_00fb05a0` = −1). The time step is `dt = +0x10 × +0x14` of `FUN_0046ee10()` (zero when its +0x1c flag is
  clear, i.e. paused); `k60 = dt × DAT_00f8b534` (= **60**, `.rdata`), so every rate below is "per 60 Hz tick".
  The car's matrix is copied to cam + 0x510..0x54c.
- **`FUN_005e14d0` (matrix → Euler angles).** Lane 0 = pitch = `−asin(m[2].y)` (the forward row's height,
  clamped to ±1 by `_DAT_00fb1360` / `_DAT_00fb13b0`), lane 1 = yaw (`atan2`), lane 2 = roll (`atan2`). If
  `|cos pitch| − 1` is within `DAT_00f9d474` (= **0.01**) of gimbal lock, it returns the previous angles (the
  third argument, cam + 0x590) instead. The target angles of this frame are `Euler(car)`.
- **`FUN_005c89c0` (per-axis gains).** With the stiffnesses `K = (cam + 0x6d0, cam + 0x6d4, cam + 0x6dc)`:
  `retain_i = 1 − clamp(K_i × k60, 0, 1)` (`DAT_00f8a820` = 1); the pitch lane is further multiplied by
  `min(|speed| × 0.02, 1)` (`_DAT_00fb0880` = 0.02, `_DAT_00fb05f0` = 1; the vector arrives in EDX, which by
  the call sequence is the `|speed|` vector `S`), so **below 50 m/s the camera's pitch follows the car's
  pitch faster, and at standstill it is rigid**. Output `blend_i = 1 − retain_i`. If the car's flag +0x3c is
  set (a reset), K_yaw is reset to the per-player default (state + 0x74).
- **`FUN_005c75b0` (the step).** `d = wrap(target − angle)` per lane (`FUN_005e1840`, the angle difference),
  `angle(cam + 0x590) += blend × d`, then re-wrapped. The output (smoothed angles) equals
  `target − retain × d`. Overrides: with flag cam + 0x31c the yaw lane is replaced by `_DAT_01189f60` (`.bss`)
  or 0 (a fixed-yaw mode); if cam + 0x6f0 is outside (`DAT_00f91588`, `DAT_00faf87c`) it is added to the yaw (a
  yaw offset, by its place the look-around / look-back; who writes 0x6f0 is not read).
- **No clamp on the lag**: nothing in these three helpers limits how far the angles trail the car.
- **Pitch stiffness K_pitch (`FUN_005c8e40`, called late in the update, so it acts on the next frame).**
  `K_pitch += (state[+0x70] − K_pitch) × 0.1` per update (`DAT_00f9f660` = 0.1, not scaled by dt). While the
  car is airborne (vehicle + 0x1630 > 0, a "has-air" check through `FUN_00911430` and `FUN_008f5350`) it eases
  towards 0.1 instead and accumulates the air time at cam + 0x6e4; on landing a shake request
  `cam + 0x280 = max(cam + 0x280, clamp((|v| − DAT_00fa929c) × _DAT_00fb0eac, 0, 1) × _DAT_00fafe14)` and a
  height offset `cam + 0x574 = airtime × DAT_00f8a810 × DAT_00fb0d7c × DAT_00fafe20` are set (constants not
  read this round). `state + 0x70` / `+ 0x74` (the per-player default pitch / yaw stiffness) are written
  outside the functions read: their source is open.
- **Roll stiffness K_roll (cam + 0x6dc)**: no writer in the helpers dumped this round (`FUN_005c9380`'s writes
  go to cam + 0x660..0x66c, its own roll state); open.
- **Yaw stiffness K_yaw (cam + 0x6d4)**: `FUN_005c90a0` above. Field map, settled with `mwtool genesys`
  (it prints in stored order): y10 = `Decay_rate` (0.01), y14 = `78527964` (10), y18 = `High_speed` (100),
  y1c = `63fb2207` (0.05), y20 = `397fce82` (10), y24 = `Low_speed` (0), y28 = `5a74d44d` (0.075). With the
  shipped data y14 = y20 = 10, so the yaw-rate term `1 − (|ω_y| − 10) / 0` is +∞ → t = 1 for every real yaw
  rate: **the yaw-rate modulation is inert in the shipped data**. The lerp `lerp(y28, y18, f)` then mixes the
  low-speed spring 0.075 with `High_speed` = 100, which only makes sense if `f` stays 0 (or is a speed
  fraction and the read operand is wrong); `y1c` (0.05) is never read here. Unresolved without the
  disassembly: `f`, `mods(0x29)` and `_DAT_0110e8f0`.
- After the angles: `FUN_005c7a80` (gradient spring), `FUN_005c76a0`, `FUN_005c8ad0` (both take `S`), then
  **`FUN_005d12b0`** (3500 bytes; called with `S`, the gains and `k60`; writes cam + 0x550..0x58c and
  0x5a0; callees `FUN_005dc270`, `FUN_005d76d0`, `FUN_005bd470`): the most likely place of the eye / look-at
  offsets (speed and acceleration movement), not read. `FUN_005c2130` (1119 bytes; callees `FUN_005d9740`,
  `FUN_005e1ae0`) takes the target angles, the smoothed angles, the car matrix and an output frame: the
  composition, and still the best avoidance candidate with `FUN_005d0b90`'s `FUN_005dfe90`.
- **`FUN_005c8880` (a vertical bob, read; weight).** Unless `FUN_0057a010(target)` fails (then zero), it
  reads two floats through `FUN_009cdd20(n)` = `*(float *)(obj + 0x1f0 + n × 4)` for n = 2 and 3 (the
  object in ECX is not identified; per-index values 2 and 3 suggest the rear wheels, e.g. their compression),
  takes `0.5 × (v2 + v3)` (`_DAT_00fb05e0` = 0.5), pushes it into a 4-sample history at cam + 0x490..0x49c
  (`FUN_005d8330`) and returns the history's mean (sum / count at cam + 0x4a0) × state + 0x94, in the **Y lane
  only** (`_DAT_00fb05c0` = (0, 1, 0, 0)). The result goes to `FUN_005c2130` as an offset: the camera rises
  and falls with a 4-frame average of those two values. Not ported (object and state + 0x94 unread).
- **Avoidance, negative result.** None of `FUN_005c2130`, `FUN_005d12b0`, `FUN_005dfe90`, `FUN_005d9740`,
  `FUN_005e1ae0`, `FUN_005dc270`, `FUN_005d76d0`, `FUN_005bd470`, `FUN_005d9610` calls anything outside the
  camera's own 0x5bxxxx–0x5exxxx range and the CRT (no collision / world query); the leaves are maths. So
  the world / car avoidance is not in the chase update's own call tree as read: the remaining places are
  `FUN_005c76a0` (calls `FUN_005d9590`), `FUN_005c8ad0`, `FUN_005c3af0` (`FUN_00513200`), `FUN_005e05a0` and
  `FUN_005c0c80` inside `FUN_005d0b90`, or a separate pass on the final eye (the manager `FUN_005e23c0`).

### Speed and acceleration movement: `FUN_005d12b0` (round 7, partly read; the z offset ported)

Called with the target's velocity vector (param 2), the `|speed|` vector `S` (param 3, mph: vehicle + 0x1620),
`dt` (param 4) and the gains (param 5). Returns at once unless the state flag `state + 0xa0` is set; if the
target is not valid (`FUN_0057a010`), zeroes cam + 0x570..0x58c. `A` = `Acceleration_movement` (instance
+ 0x54), `M` = `Speed_based_movement` (+ 0x0c); sub-offsets with the 911's values: A+0x10 `bd3a7cfc` −2,
+0x14 `7c9039e2` 0.5, +0x18 `285f2260` 2.5, +0x1c `b40b3fb7` 0, +0x20 `77e7de37` 50, +0x24 `8e1310f6` 1.5,
+0x28 `65b4f9c6` −1.2, +0x2c `fdd25ee3` 2, +0x30 `f393abd1` 0, +0x34 `a8cc9c9b` 1.75, +0x38 `0da33093` 2.5;
M+0x34 `0a78c15f` −1, +0x38 `b028673b` 1.5, +0x3c `7d4ec535` 1.7, +0x40 `b6010ee1` 0, +0x44 `53e4b4f9` 50.
- **Longitudinal acceleration (read).** `FUN_005d9610` returns an acceleration vector `a` (by its use; source
  not read, likely the target history of `FUN_005e23c0`). Unless `|v|` or `|a|` is below `DAT_00fb0770`
  (1.19e−7), `a_long = clamp(sign(v̂·â) · |a| · |v̂·â|, ±DAT_0110e770)`: the acceleration projected on the
  velocity, clamped by the `.bss` vector `DAT_0110e770` (not read). `mask = a_long < 0` (decelerating).
- **Speed fraction (read).** `h = clamp((S − M.b6010ee1) / (M.53e4b4f9 − M.b6010ee1), 0, 1)`.
- **Heights (read, direct).** `cam + 0x5a4 += (M.7d4ec535 − state[+0x44]) · h` and `cam + 0x5c4 +=
  (M.b028673b − state[+0x64]) · h` (these vectors are re-set each frame elsewhere; `state + 0x44` / `+0x64`
  by the formula's shape are the data eye / look-at heights): eye and look-at heights go linearly to their
  full-speed values, unsmoothed. Matches the round-4 *hyp.* rows, now with the lower bound `b6010ee1`.
- **Longitudinal (z) offset (read; three `.bss` limits).** Target lane z =
  `M.0a78c15f · h + (mask ? A.7c9039e2 : A.bd3a7cfc) · |a_long| / DAT_0110e770`. The velocity
  `cam + 0x578 = (g · k · (1 − w) + w) · clamp(target − cam[+0x588], ±DAT_0110e4d0)`, `w = mask ? A.285f2260 :
  A.fdd25ee3`, `g` = `|lane 0|` of the vector at vehicle + 0x2b20 (not identified), `k` = param 5; zero when a
  local vector's squared length is below `DAT_0110e610`. Then `cam + 0x578 −= cam + 0x6c8`, where
  `cam + 0x6c0..0x6cc` low-pass (rate `0.1 · k`, `_DAT_00fb07a0`) towards `1.85` (`_DAT_00fb09d0`, lane 0)
  while `S > DAT_0110e640` (`.bss`), else 0. Position `cam + 0x580..0x58c += velocity · dt`, z clamped to
  `±DAT_0110e680` (`.bss`); `cam + 0x584` (y) is then replaced by `cam + 0x550`. So the eye's distance is a
  **first-order lag** at 2/s (accelerating) or 2.5/s (decelerating) towards a target set by speed and the
  normalised longitudinal acceleration, not a second-order spring. The braking target is `7c9039e2` (0.5 m),
  not `8e1310f6` (1.5), which belongs to the vertical stage.
- **Vertical stage (read in outline, not ported).** `cam + 0x554` (the y of 0x550..0x55c) integrates
  `cam + 0x560 · dt · (…)`, with the gain chosen by the target sign between `A.8e1310f6` (1.5) and
  `A.a8cc9c9b` (1.75) (or 3, `_DAT_00fb0900`, when flag cam + 0x716 is set), `A.65b4f9c6` (−1.2) and
  `A.0da33093` (2.5) stored to cam + 0x558 / 0x55c, 20 (`_DAT_00fb07f0`) to 0x564, and the constants 15 / 185,
  0.8 / 0.75, 0.25, 7, 60, −5 / 8, −3 / 4 and `(cam[+0x6b0] − 10) / 490`, an airborne branch through
  `FUN_008f5350`, `FUN_005d76d0` × `.bss` `0118a060` (the speed scale). The decompiler's output loses the
  locals here (a zero vector is used as an operand); this needs the disassembly.
- **Tail (not read).** `cam + 0x5e0 += (x − cam[+0x5e0]) · 1.6667 · dt` (`_DAT_00fb0a00`) after eight
  `FUN_00edec52` / four `FUN_00edc7b2` CRT calls (trigonometry, by the CRT range).
- `.bss` needed: `DAT_0110e770` (acceleration normaliser and clamp), `DAT_0110e4d0` (step clamp),
  `DAT_0110e680` (offset clamp), `DAT_0110e610`, `DAT_0110e640` (speed threshold of the 1.85 term).

### The live recording (round 8, `ref/live1`)

A 258 s recording of the chase camera object (cam = director + 0x8d0) and the player car at ~120 Hz, 14,383
distinct camera frames (the game updates at 60 Hz). What it settles:
- **Camera placement.** Eye = followed position (cam + 0x500) + R(smoothed angles cam + 0x590) ×
  (0, cam + 0x5a4, cam + 0x5a8 + cam + 0x588): the residual median is 0 on all axes. cam + 0x5a0..0x5ac is
  reset each update from state + 0x40..0x4c (this car: (0, 1.575, −4.6, 0)), and cam + 0x5b0.. from state +
  0x50.. (look-at), then modified by `FUN_005c76a0`, `FUN_005c8ad0` and `FUN_005d12b0`. The recorded eye z is
  −5.94 at rest and −4.0 at 130 mph (plus cam + 0x588, down to −1.18); the w lane (cam + 0x5ac) fits
  `−1.2 + 4.55 × (1 − tan 35° / tan(FOV/2))`, a FOV-compensating dolly: `FUN_005c1f90` (round 9, below).
- **Live state block** (camera + 0x290): +0x44 / +0x48 eye y / z, +0x64 look-at y 1.15, +0x58 / +0x68 0.891,
  **+0x70 K_pitch 0.55**, **+0x74 K_yaw default 0.09**, **+0x78 K_roll 1.0**, +0x7c / +0x80 FOV 70 / 95,
  +0x84 0.055, **+0x88 / +0x8c FOV speed band 0 / 125 mph**, +0x90 10 (FOV per unit of cam + 0x284), +0x94 4.
  +0x98 / +0x9c were 0 in this block, so the run's car was not identified (its eye (0, 1.575, −4.6) and look-at
  height 1.15 identify it against `mwtool genesys`, which this round did not run to completion).
- **Followed frame**: the yaw step of `FUN_005c75b0` with the recorded K_yaw reproduces cam + 0x594 to 0.0002°
  median. K_yaw (cam + 0x6d4) = 0.09 standing, else `0.05 + 0.025 × clamp(mph / 100)`; cam + 0x6d8 is always
  1; K_pitch (cam + 0x6d0) 0.55, 0.1 in the air. The roll lane cam + 0x598 stays within 0.25°: the followed
  frame has no roll; the camera's roll (its right vector's height) tracks cam + 0x660 (`FUN_005c9380`'s
  state, ±3°, correlation 0.89). The speed in `FUN_005c89c0`'s pitch gate is mph.
- **FOV (`FUN_005c76a0`, read)**: `FOV = lerp(state+0x7c, state+0x80, clamp((mph − state+0x88) /
  (state+0x8c − state+0x88), 0, 1)) + state+0x90 × cam+0x284`; cam + 0x284 stayed 0. The same function ramps
  cam + 0x6f8 up while vehicle + 0x1944 > 0 (by its shape nitrous) and writes it × `DAT_00fafc4c` to target +
  0x38 (0 in this run). The recorded FOV deviates from this by 3.8° RMS, correlated with acceleration: the
  term at the end of the same function (cam + 0x6ec, round 9, below).
- **`FUN_005c8ad0` (read)**: scales the eye vector's x, z and w lanes (not y) by `1 + |sin-poly(yaw lag)| ×
  (r − 1) × DAT_00f8a810`, `r` a ratio of the target object's +0xd0 − +0xc0 over +0xd8 − +0xc8 (by its shape the
  car's box width over length): the camera moves in as the car turns side-on to it. Ported in round 9 (below); this is the nearest thing
  to "avoidance" in the chase code.

### Eye position and the FOV's acceleration term (round 9, read and ported)

Order inside `FUN_005d4970`: `FUN_005c76a0` (FOV), `FUN_005c8ad0` (side-on), `FUN_005c1f90` (dolly), later
`FUN_005d12b0` (heights, the z offset cam + 0x588). Replay: `tools/camreplay.py` (eye = cam + 0x5a0..0x5a8).
- **FOV acceleration term (`FUN_005c76a0`, tail).** `a` = `FUN_005d9610`'s vector, `v` = `FUN_005d9590`'s,
  `f` = the target's forward row (target object + 0x2dd0). Target = `clamp(|a|, 0, 20)` (`_DAT_00fb06a0` = 0
  subtracted, `_DAT_00fb07f0` = 20), zero unless `f·â > 0` and `f·v̂ > 0` (accelerating forwards while moving
  forwards). cam + 0x6ec moves towards it by 0.01 of the gap per update when rising (`_DAT_00fb0810`) and 0.05
  when falling (`_DAT_00fb0800`), not scaled by dt. Then `FOV += (+0x14 of FUN_0046ee10) × FUN_0046ee20() ×
  cam+0x6ec × 60` (`DAT_00f8b534`); the two time getters are not identified (ported as 1 per 60 Hz update).
  A replay of cam + 0x6ec from the recorded body velocity, differentiated per game frame, in m/s², matches the
  recorded value to 0.094 RMS (mph/s gives 2.7): `a` is the body's acceleration in m/s². The nitrous ramp
  (cam + 0x6f8: +2 × dt while vehicle + 0x1944 > 0, else −0.5 × dt, clamped to [0, 1]; × 5 to target + 0x38) is
  not a FOV term here: it goes to the target object.
- **Side-on pull-in (`FUN_005c8ad0`).** The eye's x, z, w lanes (y × `_UNK_00fb05f4` = 1) scale by
  `1 + |sin(lag_yaw)| × (r − 1) × 0.5` (`DAT_00f8a810`), the sine from a range-reduced polynomial of the yaw
  lane of its second argument (the followed yaw minus the car's), `r` = (+0xd0 − +0xc0) / (+0xd8 − +0xc8) of
  the target's object (a box's max − min: width over length). Fit to the recording (eye before the dolly vs
  `|sin(lag)|`): slope −0.287, so r = 0.425 for this run's car. Ported with the body's collision box.
- **FOV dolly (`FUN_005c1f90`).** `d = 3.15 − 3.15 / tan(FOV × 0.5 × π/180) + 0.15` (`DAT_00fb0e90` = 3.15,
  `_DAT_00fb0720` = 0.15, `_DAT_00faf84c` = π/180; the CRT call is the tangent); the eye moves by `d` along
  the unit vector from eye to look-at (cam + 0x5c0..0x5c8), and its w lane by `d` (look-at w holds the
  distance): −1.2 m (back) at 70°, +0.41 m (in) at 95°. This is the recorded −5.94 at rest / −4.0 at speed:
  undoing it leaves the eye at the data's −4.6 (±2 %, the side-on term) at every speed.
- **Replay (`tools/camreplay.py`)**: eye z RMS 0.70 m (data eye) → 0.24 m (ported, with our FOV; ~0.13 m with
  the recorded FOV); FOV RMS 3.76° (base lerp) → 2.23°. Remaining FOV error: about −1.3 to −2.7° mean between 0
  and 120 mph (the term is too large while it is high; the unidentified time getters are the likely factor).
- **Not read this round**: the yaw stiffness producer (`FUN_005c90a0`'s `f`, `_DAT_0110e8f0`; the fit
  `0.05 + 0.025 × mph/100` stays a model decision), `FUN_005c9380` (roll), the deceleration pitch.

### Camera effects (round 8)

- **Impact shake (`FUN_005cd040`, read; called from `FUN_005d0b90`, `FUN_005cf1d0`, `FUN_005cf320`)**. Level
  cam + 0x280 decays each update by `× (1 − param_6)` (recorded: 10.8 /s, factor 0.835 per 60 Hz frame).
  Each update it walks the target's contact list (`FUN_00904880` count, `FUN_00904850` entries, material
  table `DAT_010edb68` by entry + 0x42), normalises each contact's value `clamp((x − lo) / (hi − lo), 0, 1)`
  × per-lane scale, and raises the level to the clamped maximum (never adds). Amplitude = param_13 × level,
  passed with data vectors to `FUN_005dfe90` (not read) which moves the camera. The string
  `GameplayExternalImpactShakeAngles` ("the current angles of the impact camera shake effect") names its
  output. Recorded triggers: crashes (speed drops 70→65, 99→79, 61→50 mph gave levels 2.6, 2.2, 2.3; scrapes
  0.1-0.6) and landings (air 0.8 s at 131 mph → 1.1; 1.0 s at 80 mph → 1.0); the landing request of
  `FUN_005c8e40` is `clamp((|v| − 10) × 0.00204, 0, 1) × 6.5` (`DAT_00fa929c`, `_DAT_00fb0eac`,
  `_DAT_00fafe14`, `.rdata`), |v|'s units unconfirmed (neither mph nor m/s fits both landings).
- **Speed shake**: the camera's angle relative to the followed frame, high-passed (0.15 s), is 0.05° RMS in
  pitch at rest, 0.30° at 40-80 mph and 0.16-0.19° above, with energy at 3-4 Hz; roll 0.18° / 0.11°. The eye's
  height residual has 0.15-0.27 m standard deviation above 40 mph. cam + 0x120..0x12c is a vector that is 0
  at rest and 0.1-0.2 at speed (by its behaviour a shake state; not correlated frame-by-frame with the view
  angles); cam + 0x130..0x170 and 0x330 / 0x334 are noise-like at all speeds (generator states).
- **Impact shake, settled (round 9; ported in camera.rs `ImpactShakeState`)**. The call in `FUN_005d0b90`
  passes the `Impact_shake` fields one by one (globals = the `+0x174` handle; Impact_shake starts at globals
  + 12): decay per update `79f982f0` (0.165, not dt-scaled: 0.835 per 60 Hz frame, matches the recorded
  10.8 /s), contact range lo `96f848b2` (5) / hi `a2c5d2eb` (60), scale `192db931` (1), power `d4c1621b`
  (0.75; `FUN_00ed9e1b` is the CRT `powf`, base and exponent in xmm0 / xmm1), cap `4ffbfd8f` (4),
  `Maximum_amplitude` (2, the last argument: degrees per unit of level), `Maximum_frequency_multiplier` (30,
  times the update dt) and a 7-float block for the stage: `91b99f00` (1), `WobbleMagnitude` (0.5 ×3),
  `ShakeMagnitude` (1 ×3). Only contacts of material classes 7 and 8 (mask 0x180) count, value = |contact +
  0xc|; a second value comes from a per-body table (+0x1c). Level = `max(level, min((clamp((x − 5) / 55, 0,
  1) × x)^0.75, 4))`. `b4632dc1` ([10, 10, 50]) is not read on this path; `CameraGameplayShakeEffect`
  (the springs with Amplitude 250 / Spring 450) is not used by it either.
- **`FUN_005dfe90` (read, this = cam + 0x120)**: a noise generator object (cam + 0x110: eight shuffled floats
  and the 64-bit LCG 0x5851f42d4c957f2d + 1) draws per lane a jitter `j ∈ ±ShakeMagnitude` and, for lanes 0
  and 1, a kick `k ∈ ±WobbleMagnitude`: `v += k + (−x − v) × 1; x += v × 30 × dt` (x at cam + 0x130, v at +
  0x13c; the third lane's x / v are never stored, so it is jitter only). Output angles (degrees, cam + 0x120)
  = `2 × level × (x + j)`; they become a rotation matrix (`FUN_005d9740`, radians) pre-multiplied onto the
  camera matrix, with a translation that rotates about a pivot (fourth argument, not traced; ours pivots on
  the eye). Checked on `ref/live1`: x lag-1 autocorrelation 0.50 (30 × 1/60), x std 0.167 (theory 0.167), v
  std 0.333 (0.333), `out / (2 level) − x` uniform in ±1 (std 0.58), decay 0.835 per frame; at level > 1.5
  the output is 3.6 / 3.0 / 3.2° RMS for mean level 2.5. Lane → axis (x, y, z of the camera) is assumed in
  that order; `FUN_005d9740`'s Euler order was not traced.
- **The noise stage's time step (2026-10-09)**: the step is the fixed update tick, not the frame time.
  Solving `x' = x + h·v'` for h on every update of `ref/live1` gives h = 0.5 (= 30 × 1/60) on 13 417 of
  13 430 updates and 1.0 (the 2/60 catch-up step) twice, including the update right after a 4.7 s stall and a
  0.37 s one (the rest are records torn by the 120 Hz sampler). With `91b99f00` = 1 the stage is
  `x ← (1 − h)·x + h·kick`, stable only for h < 2: stepped with a variable frame time, frames longer than
  1/15 s grow x geometrically. Ours (camera.rs) now runs it in 1/60 s steps, at most 8 per frame.
- **Landing**: the recorded landing levels (~1.0) fit the contact path with x ≈ the vertical speed into the
  ground in m/s (a 1 s fall, 9.8 m/s, gives 0.9); the `FUN_005c8e40` request (`× 6.5`) explains the one
  level above the cap (5.4) in the recording; its |v| units remain open, so it is not ported. Ours feeds the
  vertical speed at touch-down (after ≥ 0.2 s with no wheel down) as a contact value: model decision, since
  our wheels are ray casts and not in a contact list. The unit "normal speed in m/s" for contact + 0xc is also
  a model decision (its writer was not read); it reproduces crash levels 2-3 and landing levels ~1.

### High-speed shake and the eye's vertical spring (round 10)

What the "speed shake" in `ref/live1` is made of, split by source (records with the impact shake output
cam + 0x120 at zero, 95 % of the frames above 40 mph):
- **Body motion through the followed frame.** The camera's world pitch, high-passed (0.15 s), is 0.25° RMS
  at 40-80 mph, but the followed frame's own pitch (cam + 0x590) only 0.05° and the body's 0.15-0.2° (energy
  at 4-10 Hz); correlation camera / body pitch 0.16. The body's motion is mostly smoothed away by K_pitch;
  the followed position (cam + 0x500) is the body position (correlation 0.99 in height) less a constant.
- **The view relative to the followed frame** (camera forward against the eye-to-look-at line): pitch
  residual 0.08° MAD, almost all of it (correlation 0.98) a small rotation matrix at **cam + 0x360..0x39c**
  that `FUN_005d0b90` pre-multiplies onto the camera matrix right after `FUN_005e34e0`, before the impact
  shake. Its effect on the view is 2.7 × its angle, because it also moves the eye (below).
- **The eye's height**: the eye's residual in the frame is 0.02 m MAD but 0.13-0.21 m standard deviation
  (bumps, landings); it is cam + 0x550 (= cam + 0x584, correlation 0.995): the vertical stage of
  `FUN_005d12b0` (round 7), a spring: cam + 0x550 integrates its rate cam + 0x554 (slope 1.03 per dt), and
  cam + 0x568 switches between 7.155 and 6.708 = √(20 × 1.6²) and √(20 × 1.5²) (20 = cam + 0x564, `_DAT_00fb07f0`),
  by its form the spring's damping for the two branches. Read from the disassembly and ported in round 11 (below).

**The high-speed shake (read; ported as camera.rs `SpeedShakeState`).**
- `FUN_005d0b90`, first stage: `High_Speed_Shake` (instance 0x18; `Amplitude` 0x28, `Frequency` 0x2c,
  `Maximum_Speed_for_Shake` 0x30, `Minimum_Speed_for_Shake` 0x34, `Power` 0x38, by `mwtool genesys-types`
  offsets) gives `f = clamp((S − Min) / max(Max − Min, 1.19e−7), 0, 1)`, amplitude `A = Amplitude ×
  powf(f, Power)` (the CRT `powf` `FUN_00ed9e1b`; the decompiler shows only the clamp), then
  `FUN_005e34e0(this = cam + 0x360, 1, A, Frequency, dt)`, which passes the `.data` vectors 10, 0.07, 0 (`.bss`
  `0110ed84`, live 0) and 0.04 (`010c4c58..60`) to `FUN_005e2a70`.
- `FUN_005e2a70`: matrix = identity; `k = A / 10`. Play position cam + 0x420 += Frequency × dt, wrapped at
  13.267 s (`01028160`); a second phase cam + 0x430 += 30 × Frequency × dt wrapped at 255 feeds a periodic
  term multiplied by the 0 vector (inert). A random walk cam + 0x440 (two LCG draws, cubed, step clamped to
  ±0.07) times k × 0.04 is added to the pitch; recorded, it stayed at 1.0692-1.0696 all run, so the draws'
  effect as decompiled is not what runs (taken as the constant 1.0693). `FUN_009e7a30` samples the curve: 199
  quaternion x, y, z entries at `.bss` 0x01188300 (the ICE `SHAKE_DATA`; no writer in the dump, so filled at
  run time), index = position × 30 (`01028170`), forwards then mirrored (398 − i), slerped between
  neighbours, each lane times k, w = √(1 − |v|²). Rotation → Euler (`FUN_005e14d0`), pitch += the walk term,
  matrix (`FUN_005d9740`), and the translation's y −= 10 × sin(pitch): the view turns about a point 10 m
  behind the eye (looking down drops the eye).
- Checked on the recording: lane i of the matrix's rotation vector is `2 × k × curve_i` with the same k in
  all three lanes per 10-mph band, plus a pitch offset k × 0.043 (= 1.0693 × 0.04); k rises from 0.002 at
  35 mph to 0.065 at 135 mph. The install's entry that fits best is (1.2, 1, 200, 0, 1.5) above 80 mph; below
  60 mph the recorded k is ~30 % lower (power 2 fits better there; no car has power 2, so S may be a
  filtered speed: open). The curve's statistics are slow (spectral centroid 0.2-0.7 Hz at Frequency 1),
  so this effect is a slow sway, not the 3-4 Hz wobble; the faster content is body pitch and bumps.
- Replay (`tools/camreplay.py`, pitch of the matrix in degrees, mean / std, recorded vs ported): 1-40 mph
  +0.004 / 0.007 vs +0.009 / 0.012; 40-80 mph +0.027 / 0.030 vs +0.043 / 0.030; 80-140 mph +0.077 / 0.071 vs
  +0.083 / 0.068. Before: a guessed 1.4 Hz eye translation (0.012 m per unit of Amplitude) with no rotation.

### The eye's vertical spring, roll and yaw stiffness (round 11, from the disassembly)

The decompiler drops the arguments and locals on these calls; read from `disas.py snap2` instead.
**Arguments of `FUN_005d12b0`** (thiscall, `ret 0x10`): the target's velocity, `S`, the update's `dt` broadcast
(the local at `FUN_005d4970` + 0xd0, overwritten with `dt` just before `FUN_005c9380`) and `k60 = dt × 60`.

**Vertical stage of `FUN_005d12b0` (ported, camera.rs `HeightSpring`).** cam + 0x550..0x56c is a spring
object: position x (0x550), rate v (0x554), lower / upper limit (0x558 / 0x55c, rewritten every update from
`Acceleration_movement` `65b4f9c6` −1.2 and `0da33093` 2.5), mass factor m (0x560, recorded 1.0, writer not
read), stiffness K (0x564 = 20, `_DAT_00fb07f0`), damping c (0x568 and 0x56c, always equal). Per update:
- `airborne` = `FUN_008f5350` on the car (true when none of the four wheel contact bytes at +0xa30 + i ×
  0x1d0 is set). `a` = `FUN_005d9610`'s acceleration (m/s²), `v` = the velocity argument.
- If |v.x| and |v.z| are both ≤ 0.1 (`_DAT_00fb07a0`), v.y and a.y are zeroed; if then all three of v's
  lanes are ≤ 0.1 the input stage is skipped (the spring still steps).
- Gain g = 3 (`_DAT_00fb0900`) while the landing flag cam + 0x716 is set, else `8e1310f6` (1.5) when a.y > 0,
  `a8cc9c9b` (1.75) otherwise. Damping ratio ζ = 0.8 (`_DAT_00fb0970`) while v ≥ 0, 0.75 (`_DAT_00fb0980`) while
  v < 0, or, with the landing flag and v < 0, `0.5 + clamp((cam[+0x6b0] − 10) / 490, 0, 1) × k60 × 0.25`
  (`_DAT_00fb05e0`, `0820`, `0830`, `0990`). `FUN_005dc270` → `FUN_005dbbc0` sets c = `2 ζ √(K / m)`
  (`_DAT_00fb0600` = 2; zero if K or 1/m is below 1.19e−7): the recorded 7.155 / 6.708 / 4.472.
- Input u, grounded: `−g·a.y × clamp(powf(clamp(|g·a.y| / N, 0, 1), E), 0, 1)` with N, E = 185, 0.8
  (`_DAT_00fb0960`, `0970`) for a.y > 0, else 15, 0.5 (`0950`, `05e0`). Airborne: `−v.y × f`, f = `4 − 3 × k60 ×
  clamp(v.y / 7, 0, 1)` while the car rises (`_DAT_00fb06b0`, `09b0`, `09a0`), else `8 − 5 × k60 × clamp(h_mph /
  60, 0, 1)` (`06c0`, `0790`, `0730`), h_mph = `FUN_005d76d0` (vector length) of the horizontal velocity × the
  `.bss` `0118a060` (2.23694, m/s → mph). Then `v += m × u × dt`.
- `FUN_005bd470(dt)`: c is chosen by whether x and v have the same sign (0x568) or not (0x56c); `v −= c·v·dt·m`,
  `v −= K·x·dt·m`, `x += v·dt`; x clamped to the limits, v zeroed when it lands on one (`_DAT_00fae830` /
  `0890` = ∓1 are only the sign masks). cam + 0x574 = v; cam + 0x584 = x.
- **Landing window** (the tail of `FUN_005d4970`, after `FUN_005c7340`): `FUN_008f5350` again; a grounded
  update after an airborne one (cam + 0x715) sets cam + 0x716 and returns; while it is set, cam + 0x70c += dt
  and cam + 0x6b0 = max(itself, a.y); after 0.3 s (`_DAT_00fafb8c`) all three reset. Recorded: 0x716 is set
  for 0.30 s after every touch-down; 0x6b0 reached 192 / 221 / 784 after 0.8 / 1.0 / 1.7 s in the air.
- **Use**: the eye's height in the followed frame is cam + 0x5a4 + x (recorded residual 0.018 m after
  subtracting x, from 0.136); the look-at point does not move with it (adding x to it triples the view's pitch
  residual), so the view tilts as the eye bobs.
- Replay (`tools/camreplay.py`; body velocity, its per-frame difference as a, airborne = vehicle + 0x1630 > 0,
  the landing window simulated): 5.8 mm RMS against cam + 0x550, correlation 0.999; std per band recorded /
  replayed 0.018 / 0.019 (1-40 mph), 0.148 / 0.148 (40-80), 0.212 / 0.210 (80-140). With the recorded 0x716 /
  0x6b0 instead: 8.6 mm.

**Roll, `FUN_005c9380` (ported).** `Camera_Roll` sub-offsets (stored order, `mwtool genesys-types`): +0x10
`Camera_Roll_Factor_Power`, +0x14 `02dd11bc` (maximum yaw rate), +0x18 `3f8ced67` (maximum lateral), +0x1c
`Maximum_Roll_Angle`, +0x20 `Minimum_Angular_Velocity_for_Roll`, +0x24 `Minimum_Lateral_Acceleration_for_Roll`,
+0x28 `Roll_Angle_Change_Intensity`. Per update (skipped while paused):
- `lat` = (body velocity − cam + 0x5d0) / dt projected on the car's row 0 (vehicle + 0x2db0; `FUN_004aeb10` is
  the body's +0xb0 velocity, stored back to cam + 0x5d0 afterwards); `w` = the body's angular velocity
  (`FUN_0059f490`, +0xc0) on the car's up row. Units m/s² and rad/s: the lateral range (10, 30) is 1-3 g.
- `f = max(powf(f_lat, P), powf(f_w, P))`, each `clamp((|x| − min) / max(max − min, 2.38e−7), 0, 1)`; target
  = `f × Maximum_Roll_Angle × π/180` (`_DAT_00fb0630`), +target when lat < 0, else −target.
- `step = intensity × π/180 × 20 × dt × (target − cam[+0x660])` (`_DAT_00fb07f0` = 20); cam + 0x660 +=
  `min(step, max(−step, gap))`: a first-order lag at 0.70/s for intensity 2.
- The game's row 0 is forward × up (the car's right side as rendered), so lat > 0 is a right turn; the view's
  right side rises by −cam + 0x660 (regression slope −1.06, correlation 0.89 with the shakes in): the camera
  leans out of the turn by up to `Maximum_Roll_Angle`.
- Replay with the 911's values (4.7°, (10, 30) m/s², (0.2, 2.5) rad/s, power 2.5, intensity 2): 0.023° RMS
  against cam + 0x660 (0.010° at a fixed 60 Hz step), correlation 0.999; recorded std 0.56°, p1/p99 −2.3° /
  +1.6°. The run's car therefore has the 911's `Camera_Roll`. Our previous model (mph/s, a 2/s rate) was 1.7°
  RMS off in magnitude.

**Yaw stiffness, `FUN_005c90a0` (read; producing part settled, one term open).** The x87 tail: K1 =
`y1c + (y28 − y1c) × f` (`63fb2207` 0.05 → `5a74d44d` 0.075), f = `min(cam[+0x6d8], clamp((mph − Low_speed) /
(High_speed − Low_speed), 0, 1))` (cam + 0x6d8 is the rate-limited yaw-rate term of round 7, 1 with the shipped
data); then `FUN_00574870(0x29, state + 0x84)` (a mods-profile value, m) and cam + 0x6d4 = `K1 + (m − K1) × s`,
s = `clamp(|x| / _DAT_0110e8f0, 0, 1)` with x the vector the car's vtable + 0x48 returns. So the fitted
`0.05 + 0.025 × mph / 100` is K1 exactly (the earlier "high / low swapped" note is this order). Open: the
source of x and the `.bss` reference. Recorded K − K1 is 0.002 RMS while moving, negative at speed (down to
−0.28 of the way to 0.09) and best (weakly) explained by the slip angle with m ≈ 0.059, near `YawDriftSpring`
(0.055); lateral speed, yaw rate and world-velocity lanes fit no better (RMS 0.00173-0.00207 vs 0.00196 for
K1 alone). The standing value 0.09 (= state + 0x74) is not produced by this formula with s from motion; it
is likely `FUN_005c89c0`'s reset path. Not changed in camera.rs (its moving part already equals K1).

**Deceleration pitch**: not read this round.

### Avoidance: where the search ended (round 7)

- The director's update `FUN_005d5e20` is **not in the dump** ("not a function start; not in any decompiled
  function"), so a post-step after the active sub-camera cannot be read there; it needs the disassembly of the
  unpacked exe (probe) at 0x005d5e20.
- Callees of every remaining chase-update helper, none of which is a physics or world query:
  `FUN_005c76a0` (`0046ee20`, `0057a010`, `005d9610`, `005734a0`, `0046ee10`, `005d9590`), `FUN_005c8ad0`
  (`0057a010` only), `FUN_005c3af0` (`005d9610`, `005734a0`, `00513200`: a 181-byte leaf), `FUN_005e05a0`
  (`005bd470`, `005d9740`), `FUN_005c0c80` (`005dfe90`), `FUN_005c8420` (`004aeb10`, a 73-byte leaf,
  `0057a010`), `FUN_005c82c0` (`00491050` → `00490e30` → `0048c090`; a speed-scaled maths helper, by its
  inputs), the manager `FUN_005e23c0` (`0046ee10`, `005423d0`, a 231-byte leaf) and `FUN_005d2cd0`.
- Conclusion: within the decompiled code the chase camera has **no world, car or prop query**; if the
  original avoids walls it is in the undecompiled director update or a later render-side stage. Our push-in
  along the boom stays a model decision.

### Other leads checked in round 5

- `CameraGameplayExternalGlobals` holds only `Impact_shake` (96 bytes), `LensProperties` (28) and a
  `CameraGameplayShakeEffect`: no spring or avoidance settings; `CameraRearViewGlobals` is four floats. The
  avoidance's tuning is therefore code constants or `CameraGameplayExternal` fields, not the globals.
- No `ProcessNode.Camera*` instance exists in any bundle (`mwtool genesys all --type CameraTransformEntityAvoider`
  and `--type ProcessNode` return nothing; `CAMERAS.BNDL` holds a `CameraTakeList` and `CameraTake`s, the
  scripted shots). The chase camera is not a node graph; the node types are unused by the shipped data.
- The script getters (`LAB_005dae70` / `LAB_005daeb0`) were not needed once the director was found.

### Next steps for the port

Read, in this order, with the field map above: `FUN_005c90a0` to its end (the local that blends y28 / y18),
`FUN_005c9380` (roll), `FUN_005c7a80` (gradient spring on cam + 0x670), `FUN_005c8420` / `FUN_005c82c0`
(likely the boom and the accelerations), `FUN_005c76a0` / `FUN_005c8ad0` / `FUN_005c2130` (look for ray casts:
callees in the 0x00a6xxxx collision range or `FUN_005e34e0`), `FUN_005d0b90`. `.bss` values a live read must
supply: `_DAT_0110e8f0` (the yaw-spring reference speed / rates vector), `_DAT_0110e4e0` (used in
`FUN_005c7a80`). In the running game the director is at `M + 0x6a0` with `M = u32 at (u32 at 0x010edaf4) + 4`, so the chase
camera is at that + 0x8d0, its state pointer at + 0x290, the yaw state at + 0x6d4 / + 0x6d8 and the gradient
spring at + 0x670: a probe watch on those gives reference traces to port against.

### Earlier search (round 4)

What follows is the first attempt's search, kept so that it is not repeated:

- **Field offsets.** `CameraGameplayExternal` (392 bytes, `mwtool genesys-types`): `Speed_based_movement` at
  0x0c (its floats 0x34..0x44), `Acceleration_movement` 0x54 (floats 0x64..0x8c), `Camera_Roll` 0x90 (0xa0..0xb8),
  `Yaw_spring_modification` 0xbc (0xcc..0xe4), `Deceleration_Pitch_Change` 0xe8 (0xf8..0x108),
  `Speed_based_height_change` 0x10c, then the vectors `67660e20` 0x124, look-at `006e1686` 0x130, pivot
  `d68bbc59` 0x13c, eye `6e08f788` 0x148, the floats 0x160..0x170 (FOV pair, `YawDriftSpring` and two unnamed),
  the handles to `CameraGameplayExternalGlobals` 0x174 and `CameraGameplayGradientAdjustments` 0x178.
  `VehicleCameraContainer` holds the four camera handles at 0x10 / 0x14 / 0x18 / 0x1c.
- **Read forms searched** over every function: `*(int *)(res + 0x28) + OFF + index` in both operand orders
  (the form the handling code uses), `*(float *)(base + OFF)` with at least 9 of 22 External offsets,
  `base[OFF / 4]` (a float or int pointer kept to the instance) together with the vehicle's speed (+0x1620), and
  the container's four handle offsets together. Only false positives came back:
  `FUN_008be4e0` / `FUN_008be1c0` / `FUN_008bdc50` read +0x130 / +0x134 / +0x148 of a GeneSys instance but set up
  render state (a texture handle and shader constants); `FUN_005ef0f0` reads a global GeneSys resource
  (`DAT_0110f474`) at 0x10..0x130 but not the eye; `FUN_005ea6a0` / `FUN_005ea970` are matrix maths.
- **Vehicle side.** About 40 functions read the speed at vehicle +0x1620. One lead: `FUN_005e23c0` (called from
  `FUN_005ce610` ← `FUN_005d2cd0` ← `FUN_004766a0`) keeps eight-deep ring buffers of a car's speed, of the frame's
  time step × scale, of the active body's velocity (+0xb0) and of a transform, reset when the tracked car
  changes: a smoothed history of the target's motion, which fits a camera target filter but is not tied to any
  camera field. `FUN_009d5ec0` (caller `FUN_009b4b70`) seeds an object with eight random floats from a 64-bit
  LCG (multiplier 0x5851f42d4c957f2d) and zeroes its state: a noise source, possibly for a shake; unconfirmed.
- **Strings** appear inline in the pseudo-C. `CAMERAS.BNDL` is loaded in `005C0000.c` (an `IceList` load into
  `param_1 + 0x3798`). `FUN_005d4270` registers the script interface `CameraEntityModuleState` ("Camera
  Interface") with getters `DistToCar`, `ZoomLevel`, `IsInternalDrivingCam`, `IsCockpitCam` ("the in-car view -
  called bonnet in code"), `PictureParadiseAllowed`, `CurrentCameraTransform`, `GameplayExternalCameraTransform`
  (`LAB_005dae70`), `DrivingBehaviourCameraTransform` (`LAB_005dae00`), `GameplayExternalFOV` (`LAB_005daeb0`) and
  `DrivingBehaviourFOV` (`LAB_005dae40`). The getters are labels the decompiler did not turn into functions, so
  the field of the camera manager that holds the Gameplay External camera is not readable from the dump; the
  getter bodies (disassembly of the unpacked exe, or the probe) would give it and from it the camera's class and
  update. Note the 0x005c..0x005e region: the camera module (`FUN_005d4270`), the target history
  `FUN_005e23c0` below and its callers sit together.
- **Types that hint at the structure.** GeneSys also has camera process nodes: `ProcessNode.
  CameraTransformTracker` (two floats, a bool), `CameraTransformVelocityLerp` (two floats, a bool),
  `CameraTransformLerp`, `CameraTransformLerpLookAt` (three floats), `CameraTransformCollider` (a string, likely a
  collision set's name), `CameraTransformEntityAvoider` (six floats, a bool), `CameraTransformImpactShake`,
  `CameraFOVAutoZoom`, `CameraDistanceFrom`. If the gameplay camera is such a node graph, the avoidance is
  `EntityAvoider` + `Collider` and the code is reached through the node types' vtables rather than through
  `CameraGameplayExternal` offsets.

- **The camera manager's update (likely).** `FUN_005d2cd0` (called from the game-update dispatcher
  `FUN_004766a0`), when the flag `DAT_010f77ed` is set: makes sure `CAMERAS.BNDL` is loaded (`FUN_005c44b0`),
  updates the target history (`FUN_005ce610` → `FUN_005e23c0`), sets a flag bit in `DAT_010edb24`, then calls
  virtual method +8 of the object in slot `*(mgr + 0x400)` of an array at `mgr + 0x6a0` with stride 0x3050
  (slot index reset to 0 first; `FUN_004bc620` calls method +0x28 of the same array for a given slot).
  If these 0x3050-byte objects are the cameras, method +8 is the per-frame update and the class's vtable
  (found where a constructor stores it at `+0x6a0 + n × 0x3050`) leads to the chase camera. Not confirmed.

Next steps: read the `CameraEntityModuleState` getters' bodies (`LAB_005dae70`, `LAB_005daeb0`) from the
unpacked code with the probe, follow `FUN_005ce610` / `FUN_005d2cd0` / `FUN_004766a0` (they surround the
target history and sit next to the camera module), and look for the process-node update
functions by their field offsets (`EntityAvoider` floats at 0x18..0x2c, bool 0x30).
