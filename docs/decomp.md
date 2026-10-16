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

- **Wheels**: the definition has 20–27 kg cylinder bodies on a strut with a spring-damper; in the solver each
  wheel is a compact record (centre, velocity, mass = the data's wheel mass, spin, spin inertia): a **point mass**
  that the island step integrates (2026-10-10), held off the ground by a wheel constraint's contact record and
  tied to the chassis by its spring-damper element (`FUN_0092f160`, `FUN_0092f510` below); the contact's
  normal impulses are iterated in sub-steps (`FUN_0093f200` → `FUN_009c2670`, `FUN_00932190` → `FUN_009c5a90`);
  the tyre friction comes after, once per tick ("Tyres"). The tyre's grip load is **not** the spring's force but
  the static corner load plus a downforce share ("Tyres").
- **Spring**: k = W / `41543f85`, with W the static load of the chassis (wheels excluded) on that wheel, solved
  from the geometry. So `41543f85` is the **static sag** (0.08 m front / 0.07 m rear), and every car's suspension
  frequency is √(g / sag) / 2π (911: 1.76 / 1.88 Hz). `df57b7ad` (true everywhere) selects this; `dadfbfc8` is
  an absolute stiffness used otherwise.
- **Damper**: c = 2 × `Damping_Ratio` × √(k W / g) in one direction and `0c5f89b3` × c in the other (911: ×2.0
  front, ×1.6 rear; probably rebound). `0c5f89b3` is not a frequency. The setup (`FUN_009b29b0`, through
  `FUN_00933930` / `FUN_00925620`) stores rest length, k, `0c5f89b3` × c, W and c at spring +0x10, +0x14, +0x18,
  +0x1c, +0x2c (the in-air blend in `FUN_008fe200` rewrites +0x14, +0x18 and +0x2c); a per-axle factor (an
  upgrade input, 1 by default) multiplies k. +0x18 is used while the strut extends (rebound): the damper element's
  solve `FUN_0092f160` (below; settled 2026-10-07, read in full 2026-10-10).
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
  spring: a landing cannot drive the body into the ground. Ported in drive.rs (`mw_handling::suspension::stop_excess`; handling.md item 5).
- **Island step and sub-steps** (2026-10-07, from the disassembly in the live snapshots). `FUN_0093f5a0` calls
  each pass once; the iterations live in two callees, both counted by the world (constructor `FUN_009330d0`,
  same values live): `FUN_00932190` loops world +0x460 = 2 times (joints `FUN_009c62e0` / `FUN_009c5840`, the
  wheel constraints' contact pass `FUN_009c5a90`, contacts `FUN_009be060`, `FUN_00928cd0`), and `FUN_0093f200`
  loops world +0x464 = 2 times at dt / 2 (joints `FUN_009c4e10` / `FUN_009c1fb0`, contacts `FUN_009be420`,
  `FUN_00928fb0`, the wheel constraints' impulse pass `FUN_009c2670`, the force elements `FUN_00932f90`, then
  gravity and velocity integration `FUN_009282d0`). Positions are integrated **between** the two, by
  `FUN_009324f0` (2026-10-10, read): each awake body through `FUN_0091f380` with dt, and each wheel record's centre
  x += v dt with the velocity the contact passes left; so the order of a tick is contact passes → integration →
  sub-steps → tyres (`FUN_009289f0`) → wheel spin angle (`FUN_009325e0`: +0x1c += spin × dt, wrapped) →
  `FUN_009328a0` (not read in full: it calls `FUN_0091f380` on each body with its linear velocity zeroed and caps
  the speeds; earlier taken for the position integration) → sleep. The step's context (`this` of `FUN_0093f200`;
  snap1 0x07b72b80, the earlier "island at 0x07b72a80" was 0x100 off) is the world + 0x2b0: +0x00 bodies, +0x08
  wheel records, +0x20 wheel constraints (8), +0x1b0 / +0x1b4 the two loop counts (2, 2; world +0x460 / +0x464),
  +0x1bc dt (1/60), then the sleep thresholds. The sub-step's dt is context +0x1d0 × 1 / count and the force
  elements' last argument context +0x1e0 (1/dt); neither was read live (outside the step they hold other data),
  but h = dt / 2 is confirmed by the wheels' velocities (below).
- **`FUN_00932af0` is the island sleep pass**, not a force: the island sleeps once every body stays under 0.4 m/s
  (world +0x46c) and 0.3 rad/s (+0x470) and every wheel's spin under 1 rad/s (+0x474) for 2 s (+0x468) of
  accumulated dt; then it zeroes the bodies' velocities and accumulators, sets body +0xf4 bit 0 and wheel record
  +0x64, and flags the island's elements (+0xc bit 0) so the solver skips them (parked cars in the snapshots).
- **Force elements** (`FUN_00932f90`, three island lists): the +0x68 list holds one spring-damper element per
  wheel (type 0, `FUN_00933930`); a first loop runs every damper (`FUN_0092f160`), a second every spring
  (`FUN_0092f510`) on the same elements, then the +0x70 anti-roll bars (`FUN_0092ee70`). Elements with +0xc bit 0
  set are skipped. Arguments: chassis (× 0x100 from element +4), wheel record (× 0x80 from +8), the sub-step's dt,
  two unused, context +0x1e0 (1/dt).
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
- **Damper** (`FUN_0092f160`, read in full from the disassembly 2026-10-10): A = the chassis-local anchor
  (element +0x20) in world space, n = (wheel centre − A) / |…| (zero below FLT_EPSILON, 0x010246b0), v_rel =
  v_wheel · n − v_chassis(A) · n (positive: the strut extending). Coefficient: element +0x18 (`0c5f89b3` × c,
  rebound) while v_rel > 0, else +0x2c (c). W = the chassis's inverse effective mass at A along n (`FUN_009e8230`)
  plus the wheel's 1/m only where wheel record +0x0c ≠ 1.0 (`.rdata` 0x010245c0). λ = −coef × v_rel × 0.5
  (`.rdata` 0x010245b0) × h; the predicted v_rel + λ W is kept if its sign (1, −1 or 0, the −1 from 0x01024570)
  equals v_rel's, else replaced by 0; the impulse that reaches it, through W (rcpps + two Newton steps), goes −λ n
  to the chassis at A (`FUN_0097e440`, `FUN_00907840` with h) and +λ n / m to the wheel under the same +0x0c mask.
  Element +0x3c accumulates λ / dt. **+0x0c is the w of the wheel centre**, a point (1.0 in all 124 wheel records
  of snap1), so the damper acts on the chassis alone. Its × 0.5 halves the damping the data gives.
- **Spring** (`FUN_0092f510`, read in full from the disassembly 2026-10-10): same A, n, v_rel; x = |wheel centre −
  A| − element +0x10 (rest length); k = +0x14. W = the chassis's inverse effective mass at A along n + the wheel's
  1/m (always). The free result v_rel − k x h W is kept while x < 0 and +0x0c = 1.0 (compressed); otherwise it is
  limited by M = max(−x, x) × (1/dt) (the −1 at 0x01024570): extended (x > 0) to at least −M, at x = 0 to at most
  M (= 0). λ = (target − v_rel) / W goes −λ n to the chassis at A and +λ n / m to the wheel (no mask); +0x3c += λ /
  dt. So a compressed spring is a plain explicit impulse k |x| h per sub-step on both, and an extended one may pull
  the wheel back to its rest length no faster than in one tick.
- **Wheel contact** (2026-10-10, from the disassembly of `FUN_009c5a90`, `FUN_009c52d0`, `FUN_009c2670`). Each
  wheel constraint uses one contact record (0x40 bytes at constraint +0x80 + +0x4a0 × 0x40: normal (its w the
  accumulated normal impulse), ground point, contact offset from the wheel centre (its w = 1.0 once active), +0x30
  the wheel's normal velocity, mass, −1). In each contact pass: d = (ground point + ground velocity (+0x480) × dt −
  (wheel centre + offset + v_wheel × dt)) · n; nothing when d < 0; else `FUN_009c52d0` activates the record (w = 1,
  counts at +0x5a / +0x5b) and the **wheel's** velocity rises by n d / dt, after which the strut stop runs (its own
  d from the strut end, above). The impulse pass gives the wheel max((record +0x30 + n · v_ground (+0x490)) ×
  constraint +0x0c − n · v_wheel, 0) along n, on active records only; constraint +0x0c is 0 in all four of snap1, so
  it only stops the wheel approaching the ground. The record's normal w and constraint +0x3c accumulate mass × that.
- **Wheel gravity** (`FUN_009282d0`, second loop, from the disassembly): each awake wheel record's velocity gains
  (0, 1, 0) (`.rdata` 0x01024590) × 9.81 (`.bss` 0x01185f30) × h × h, both factors the sub-step's dt argument:
  +0.0007 m/s up per sub-step. The chassis's loop adds body +0x9c (−9.81) × (0, 1, 0) × h. So the wheels fall only
  through their springs.
- **Checked in snap1** (XKR at 51 m/s): the spring elements (found by their anchors at 0x07bc7880 + i × 0x80) hold
  k 41603 / 41784 / 47896 / 47663, rebound c 6344 / 6371 / 6420 / 6389, W 4576 / 4596 / 4071 / 4051 N, c 3524 /
  3540 / 4013 / 3993; ours from the data: k 41671 / 47750, c 3530 / 4000, rebound 6354 / 6401 (0.3 %). The
  wheels are compressed x = −0.1646 / −0.1659 / −0.1123 / −0.1140 (from the chassis pose and the wheel records),
  so one sub-step's spring kick k |x| h / m is 2.283 / 2.301 / 1.792 / 1.820 m/s with h = 1/120 (twice that with
  1/60); the wheel records' velocities along their contact normals read −2.279 / −2.307 / −1.791 / −1.813. That is
  the state a tick leaves: the second impulse pass stopped the wheel, the second spring kicked it (the damper does
  not touch it). Parked cars' wheel records read −1.0 to −1.3 m/s the same way.
- **Strut stop, read in full (2026-10-09)**. `FUN_009c5a90` runs per wheel constraint in each of the island's
  contact passes (`FUN_00932190`, `this` = constraint, then wheel record, chassis body, dt, 1/dt). With d = the
  predicted penetration of the ground into the tyre placed at the strut end (+0x460, chassis-local; d = (ground
  point − (strut end + contact offset + v_chassis dt)) · n) and s = min(+0x470, 0.1 `.bss` 0x01186bf0): excess
  e = max(max(d − min(d, s), 0), 0.1 d) (0x01186b10), share +0x474 = clamp(e / max(d, FLT_EPSILON), 0, 1), flag
  +0x4ae (cleared at the top of every call); the chassis gets n × e × (1/dt) through its inverse effective mass
  at the strut end (`FUN_009e8230`, rcpps + Newton), accumulated by `FUN_009ca290` and applied by
  `FUN_00907840`. The wheel record itself gets its velocity raised by its own predicted penetration / dt along n
  before that (+0x40 += n d_wheel / dt; the stop runs only when that d_wheel ≥ 0; "Wheel contact" below, corrected
  2026-10-10), so the wheel follows the ground fully. `FUN_009c2670` (each impulse sub-step) gives the wheel max((bias + n · v_ground)
  − n · v_wheel, 0) (+0x50 = mass accumulates the impulse) and, while +0x4ae is set, the chassis share × m_eff ×
  max(−v_n, 0) at the strut end. **+0x470 is the tyre's sidewall**: the builder `FUN_009c24b0` copies it from its
  sixth argument, which `FUN_0095b4d0` computes as max(wheel shape radius (wheel object +0x19c → +0x40) − wheel
  object +0xcc, 0); +0xcc is `WheelSizes.337afbd9` (rim radius) of the axle's `VehicleInfo.Axles[i].Wheel_Sizes`.
  Snapshot snap1 (XKR, 1762 kg): radius 0.339 / 0.338, rim 0.265 → +0x470 = 0.074 / 0.073 ✓; +0x474 never
  written (0xcdcdcdcd) in snap1, 0.1 on the fronts in snap2. So within the sidewall the chassis takes a tenth of
  the penetration per pass, beyond it the rim hits hard.
- **Wheel and strut objects (snap1, XKR)**: wheel record (world +0x08, 0x80 each): +0x00 centre, +0x40 velocity,
  +0x50 mass 25, +0x54 spin inertia, +0x58 1/m, +0x5c 1/I. Wheel object (vehicle +0x870 + i × 0x1d0 in the
  frame of `FUN_0095b4d0`; +0x198 → wheel record, +0x19c → its cylinder shape, radius at +0x40): +0xc0 =
  chassis-local rest position (0.795, −0.133, 1.291) front, (0.799, −0.144, −1.458) rear, w = rim 0.265. The strut
  object (pointer at vehicle +0x10f0 + 8i): +0x40 top (0.8018, −0.077, 1.291), +0x50 bottom (0.788, −0.232),
  +0x60 an arm pivot on the far side (−1.197, 0.020), +0x70 1.5° (0.02618 rad); rear top −0.077, bottom −0.237.
  So the bump travel above rest is 0.056 front / 0.067 rear on the XKR (ours: 0.06 for every car), droop 0.099 /
  0.093, the strut tilted 5° (`Angle`). The spring element (+0x10 rest length 0.463 / 0.449, +0x14 k 41603 /
  47896, +0x18 rebound c, +0x1c W, +0x20 anchor (0.795, 0.22, 1.291), +0x2c c) puts the rest position at the sag
  (0.463 − (0.22 + 0.133) = 0.11 ✓). Where the strut top (−0.077 chassis-local on both axles) comes from in the
  data was not found.
- Unknown: the strut travel (+0x460 is set by the builder's caller; not traced); what holds the wheel record on
  its strut across (the joints `FUN_009c62e0` / `FUN_009c4e10` were not read; ours moves the wheel along the body's
  up axis only); the collision detection's margin for making a wheel's contact record; the writer of the wheel
  object's on-ground byte (+0x24, vehicle +0x894 for wheel 0; `ref/live1`'s `w*_contact`), so whether it means an
  active contact record is not known. Ported in `mw_handling::suspension` and drive.rs (handling.md item 5).

## Wheel placement (2026-10-10, ported in `mw_assets::wheel_layout`)

- **Vehicle build** (`FUN_009b40c0`, called by `FUN_009b4e60`; confidence high): the head fills each wheel's
  cylinder `Radius` and `HalfLength`; then for every `VehicleInfo.Axles` entry (+0x98, u16 count +0xa0, 0x28
  bytes each) and each of its `SuspensionWheel` handles (axle +0x18, count +0x24) it **overwrites** the wheel
  body's position (`FUN_009add40`) with x = `SuspensionWheel.dcbe38af` (+0x38), y = `VehicleInfo.caac331c` (+0x90)
  + R − axle `2f5fb582` (+0x1c), z = axle `eaee225f` (+0x20). The strut top is (x, `caac331c` + constraint +0x24,
  z), direction (0, 1, 0) (0x01025030); a per-axle scale (`param_9` / `param_10`, 1.0 from `FUN_008f9120`, likely
  an upgrade hook) multiplies the strut-top-to-wheel distance. The springs are then made at this geometry
  (`FUN_009b29b0`: rest length = |anchor − wheel| + W / k), so this is where the wheel rests. The wheel
  `RigidBody` transforms (`0d0c29b4`) in the data are never used for position.
- **Spawn matrix** (`FUN_00593ac0`): per race-car slot and wheel (count = spec +0x13, record remapped by
  slot +0x14 + i) the render object gets the spec's part models (+0x78, count +0x80, spin mask +0x7c) and a rest
  matrix of the spec position (+0x00) and scale (+0x20); x < 0 turns it by π about Y (0x00fad9e0) and sets
  +0x81. The quaternion (+0x10) is never read. Stored at `mgr + 0x23b50 + slot × 0x240 + i × 0x90`.
- **Per frame** (`FUN_005798a0` from `FUN_00594870`, slots with a physics vehicle, `FUN_00572810`): the rows and
  translation are replaced by the physics wheel's world matrix (vehicle +0x870 + i × 0x1d0: rows +0x60/+0x70/+0x80,
  translation +0x90) times the inverse of the car's render matrix (`FUN_005423d0`: local offset vehicle
  +0x160..+0x19c × chassis body); no scale. Render +0x74 = |wheel object +0xd0| (wheel record +0x2c, likely the
  spin speed for the blur choice). The wheel matrix is built in `FUN_00902930`: translation = wheel record +0x00
  (the body's world centre, `FUN_009dd280`), rotation = chassis × a damage bend (`SuspensionWheel.Damage` +0x18 /
  +0x20 / +0x2c), wheel record +0x1c (spin), +0x3c (unknown) and the steer on front wheels; right wheels (rest x at
  wheel object +0xc0 < 0) get rows 0 and 2 negated; `FUN_00491050` blends towards another pose by |+0x15c|
  (normally 0).
- **Right wheels are mirrored**, not only turned: the wheel shaders' flipped technique (`g_flipUvsOnFlippedTechnique`)
  has vertex programs that scale the position by (10, 10, −10) instead of (10, 10, 10) and a rasterizer state
  with front-face culling (2) instead of back (3), so turn × flip = a mirror on X.
- **Traffic** (`FUN_005f7cf0` → `FUN_005ee7f0`, from `FUN_005fcbd0`, when the LOD at job +0x28c < 4 or +0x268 ≤ 400):
  per wheel, the axle with the larger `eaee225f` is the front; within it the larger `dcbe38af` is the unmirrored
  wheel; translation = R·((x, R_FL, z) − T) with R / T the rows / translation of `VehicleInfo.509931cd` and R_FL
  the front-left body's radius (body index from `RaceCarPhysicalDefinition` +0x18) for every wheel, then times the
  body pose (car +0xe0 rows, +0x110 translation, angles +0x1f0 / +0x1f4); spec byte +0x8e = right side (Ry(π)),
  +0x8f = steered (angle car +0x1f8). No raycast per wheel. Ground probes (`FUN_005f52b0`) sit at `VehicleInfo`
  +0x8c / +0x94 (the axle z) with w = R_FL. Not traced: how the car frame is placed over the road (we take the
  road at y = −T.y in model space, as the player's tyres rest there).
- **Live** (probe dump, the Raptor, 2026-10-10): drawn wheel centres within 2 mm of the rule, parked and after
  driving and stopping (research log).

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
  Vehicle +0x1f00 = `Idle_RPM + (Max_RPM − Idle_RPM) × +0x1f10` (the pop rule's RPM, so the pop rule sees the
  sequences' RPM offsets too: the gear-change kick's end is what backfires, research log 2026-10-15). Both run only while the
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
  `Pop_Sequence` are bound to `GetPopProbability` (items at 0.6–1 / 0.75–1); the gurgle is started with the bank's
  `Sequences` when the sound starts ("Engine start and stop" below).
- **Effect group bits** (`FUN_0057a5e0`, per car, entity +0x3c8): bit 14 for two updates after a backfire
  (entity +0x3f4 = 2, counted down when the world flag `FUN_00897b70` is up), bit 8 while the nitrous input
  (+0x1944) is above 0 and the car is the one `FUN_006a0020` / `FUN_0069ff20` accept, bit 9 instead while the
  nitrous mode byte +0x195b is 1, bit 20 while it is 3 (with `FUN_006a15c0(…, 13) == 2` and `FUN_008f5350`),
  bit 19 while the engine runs (`FUN_00572c20`: gearbox state not 0 / 1), bits 0 / 4 / 6 / 7 (`0xd1`) for the
  lights, 1 / 2 brake and reverse, 3 (+0x1c27), 15 (`0x80c7` group). `FUN_005b2740` adds bit 12 when the car is
  wrecked and 13 for a non-zero vector of the player state; the coronas, lights and spot effects test
  `1 << Group` against it.

## Engine start and stop (2026-10-15, ported in `crates/mostrusted/src/ignition.rs`)

- **Gearbox start-up states** (`FUN_009da340`, gearbox +0x168 = vehicle +0x1bd8): 0 off, 1 cranking. State 0
  (`FUN_009ce3d0`): a timer (gearbox +0xe0 = vehicle +0x1b50) counts up; once past 0.5 s (`.bss` 0x01186ef0, from
  `.rdata` 0x01026050 by `00f76430`; `009a9bc0` returns the time left) a throttle or brake input (control block
  +0x04 / +0x10 lanes) or a set flag lane (+0x60, latched on release at +0x70) moves it to 1. State 1
  (`FUN_009cd7a0`): moves to 2 when +0x174 (vehicle +0x1be4) or +0x175 (+0x1be5) is set, unless +0x173 (+0x1be3,
  "held off") is set; clears the brake lanes. Both set the control block's +0x50..+0x5c lanes to −1 (their reader
  is the state machine's "any lane set → state 9" test). Resets: `FUN_009cd620(char)` (0 → state 0, else 2),
  `FUN_009cd790` (state 1), from `FUN_008f7b40` syncing entity +0x1a84 (0 / 1 / 2) for remote cars.
- **Flags**: +0x1be5 = effect property 7 of a sound sequence (`FUN_008f5490`: |value| > FLT_EPSILON); property 6
  sets +0x1be3 the same way and forces state 0. +0x1be4 = `RaceCarEngineSound` +0x1c8 (ignition played) or
  vehicle +0x30ff (`FUN_00900120`). Message 0x1e {car, flag} (`FUN_00587490` sets entity +0x1c32 and sends it;
  `FUN_005886a0` for entity message 0x10; script binding `FUN_0068b070`) sets +0x1be3 = flag and state 0.
- **Ignition sound** (`FUN_0058c930`, per frame from `FUN_005911d0`): in state 1 with +0x1c8 clear it starts
  `Ignition_Sequence` (`EngineSound2` +0x1c) once (slot +0x134) and sets +0x1c8 when that instance has ended (or
  at once without a sequence); in state 0 it clears +0x1c8 and stops the instance.
- **EngineOn** (vehicle +0x1ef0, `RaceCarEngineSound.EngineOn` via `FUN_0059ba00` / `FUN_0090d760`): end of
  `FUN_009b3030`, +0.05 per tick (`.bss` 0x011866c0, `.rdata` 0x01025370) up to 1 while the state is not 0 / 1,
  −0.05 down to 0 otherwise. `00830000.c` returns audible RPM × EngineOn (a getter). `Player.IsEngineOn`
  (`FUN_006d8ab0`) = `FUN_00572c20` (state not 0 / 1).
- **Sound start** (`RaceCarEngineSound` state 2 → 3, `00580000.c` near `FUN_00575340`): starts every sequence of
  `EngineSound2.Sequences` (`FUN_00586550`, handles at +0x17c..), then `Gurgle_Sequence` (+0x14) with
  `FUN_005866f0` (handle +0x1ac): the gurgle runs from the start like the bound sequences.
- **Throttle shunts** (`FUN_0058d1c0`): pedal (entity +0x192c, the raw throttle input; in reverse it lights the
  brake lamps, `FUN_0057a5e0`) up by more than 0.3 since the last frame → a random sequence of `EngineSound2`
  +0x60 (`c99783d2`, count +0xff); down by more than 0.3 → one of +0x5c (`6fd79c8d`, count +0xfe), both only in
  gears above 1 (`FUN_0090d460` = the gear, 0 at a standstill); a fall also starts +0x24 (`dacf5fd5`, empty in all
  banks) in slot +0x98. Not ported.
- **Nitrous sound**: `HeatLevelSoundSet` (bundle `GAMELOGIC\HL_%d.BNDL`, `FUN_0065fee0`, loaded per heat level
  by the pursuit code `FUN_00663cd0`) +0x14 `NitrousLocal`; its player was not found. Research log 2026-10-15.

## Other cars' engine sounds (2026-10-09)

Read for the user's report "traffic sounds are also missing"; ported in `mw_assets::car_sound` (data, maths) and
`crates/mostrusted/src/car_audio.rs` (runtime). `world` = the traffic object (`0x23f4e780` in `snap2`).

- **Which bank** (confirmed): every car has two `VehicleSound` resources (type 0x207, id = CRC-32 of the lower-case
  name): `<id>_VEHICLESOUND` (the player's; strings at 0x00fad464) and `<id>_COMPETITORSOUND` (0x00fad474; also
  the only one in the `_LO` bundle). Layout: +0x10 `TyreSound` (the silent placeholder), +0x18 / +0x1c a handle list
  (the player's only, 3 sequences), +0x48 `EngineSound2`, +0x50 `Vehicles.Sound.TransmissionWhine` (player's
  only), +0x60 / +0x64 offset / count of the **horn** sequences (the 911's one wave item has `Snapshot_Property
  "Player Vehicle Horn"`). The competitor `EngineSound2` is a reduced bank: one Ginsu set (the 911: 1790..7500),
  the speed / load mixers, a `GetDistanceToListener` mixer (0..130 m: EQ and gains by distance; two channels
  fade in over the first 12 m), idle loop, pops, backfire, limiter, one gear-change variant per direction. Its
  raw level is 24..29 dB below full scale at full throttle over six cars (the player's banks: 2..11 dB).
- **Traffic loading** (`FUN_006077c0`, confirmed): case 0 loads `Sound/MetalCar.bndl` (the collision set, to
  `world+0xb17a8`) and `VEHICLES\WAVES.BNDL`; case 5 loads, for every vehicle id in the lane data's list (byte
  +0x26 count, list +0x40), `"%d_COMPETITORSOUND"` by name hash (`FUN_00adaef0(hash, 10, …)`) into the table at
  `world+0xa02f0` (8 bytes per type, indexed by the lane car's type byte +0x69). The 11 traffic `VehicleSound`s
  are in `VEHICLES/TRAFFICATTRIBS.BNDL`; four distinct `EngineSound2` banks, each with two sequences: wave loops
  bound to `Traffic.EngineSounds[#].RPM` and a `NormalisedSpeed` sequence (road / tyre layers gated by
  `IsWetSurface`, `IsReversing`, `Random(LTEQ:0.2)`) holding a `TrafficEffectSequenceItem`.
- **The instance** (`Game::Traffic::EngineSoundInstance`, 0x50 bytes, five at `world+0xa0ab0+0x1e0+i·0x50`; the
  bindings `FUN_005ee480`, the `EngineSounds[#]` accessor `LAB_00617c00`): +0x00 `Idle_RPM`, +0x04 top speed
  (mph), +0x10..+0x1c a ratio, +0x20 / +0x24 mixer-channel instances (`EngineSound2` +0x6c list, count +0x103,
  `FUN_00600bb0`), +0x28.. one voice handle per sequence (count +0x102), +0x2c / +0x30 the `EngineSound2`
  handle, +0x34 the car entity (kind byte +6 = 8 for traffic), +0x38 the slot index, +0x3c RPM, +0x40 / +0x44
  occlusion (smoothed by 0.1 per frame towards +0x44, `DAT_00fb1a94`; kind-7 entities read `GetOcclusion`), +0x48
  `Random`, +0x4c state (0 free, 1 releasing, 2 playing). `FUN_005ee550` fills it from the type's
  `VehicleComponent.Handling` (+0x44): +0x00 = `Engine.Power_curve.Idle_RPM`, +0x04 = `Aerodynamics.f1538c51`
  (+0x10 → +0x60; 150 / 154 / 180 mph), the ratio = gear 2's `Gear_ratio` × `Differential` of the gearbox at
  `Engine` +0x114 (else +0x110). Snapshot (`snap2`): instance 0 idle 800, top 180, ratio 7.048, RPM 2081, random
  0.127, state 2, voices 433 / 434; instance 4 state 1 (releasing).
- **Bindings** (confirmed): `RPM` (`FUN_00617b20` / `FUN_005ec1a0`) = +0x3c, or when it is 0 (±1.2e-7) on a kind-8
  car `max(Idle_RPM, entity +0x1fc × 9.5493 × ratio)` (`.bss` 0x0110f930 = 30/π read in `snap2`; entity +0x1fc
  40.5 there). `NormalisedSpeed` (`FUN_005ec210`) = `clamp(|v · forward| × 0x0118a060 / top speed, 0, 1)` (the
  constant is the mph factor of the skid code). `IsWetSurface` (`LAB_00614970`) = the world's wet counter > 0
  (`FUN_00547170` → `FUN_00546c70`), the same for every car. `IsReversing` (`FUN_00614990`) = `v · forward < 0`.
  `Random` = car +0x5c (when the car's +0x6b bit 0 is set) or a fresh draw of the traffic RNG.
- **The RPM comes from the sound data** (inferred): the `TrafficEffectSequenceItem` (property 0, `EndTime` 1)
  of the `NormalisedSpeed` sequence maps the speed through a curve with dips (gear changes) onto exactly the
  RPM sequence's axis (2000..5550, 2000..5000, 2500..6500, 1000..5000 in the four banks), and the snapshot's +0x3c
  (2081, 2961) differs from the wheel-spin fallback (2726), so the effect item writes +0x3c. Its handler was not
  found (reached through a vtable).
- **The pool** (`FUN_0060bc20`, from `FUN_0060d290`, confirmed): every 0.3 s (`.rdata` 0x00fb1db0) the traffic
  cars within 85 m (`.bss` 0x0110f63c, read in `snap2`) of the local player's car (`FUN_00573420`) are scored
  `min(|v|² × 1/400, 1) × 0.4 + (0.5 + 0.1 in view) × (1 − min(d² / 85², 1))` (0x0110f770, 0x0110f5f0,
  0x0110f910, 0x0110f600, 0x0110fbc0; "in view" = `FUN_00443060`, a sphere of radius 4 (0x0110f5a0) against the
  camera frustum's planes); cars whose type has no engine or whose +0x6a low nibble is 1 are skipped; the best
  five (`FUN_0061cce0`) with a score above 0 keep their instance or get a free one (`FUN_005ec310`), which starts
  with `Random`, the bank and the car. Instances whose car is beyond 85 m (0x0110f630) are released
  (`FUN_005ee350`: their voices stop with a 1.5 s fade, `FUN_00a44e40(voice, 1.5)`, `.rdata` 0x00fb1bb0; the
  slot frees when they have ended). If more of the five found no slot than instances are still fading, the
  playing instance with the lowest voice handle (`FUN_005ec340`: the oldest) is released for each.
- **Per frame** (`FUN_005ee720` → `FUN_005ee250` per instance): the channel instances get the car's transform and
  velocity and the occlusion value; an instance whose entity is gone (+0x19 clear) stops its voices.
- **Placement** (`FUN_00ab9ef0` / `FUN_00abb5f0` → `FUN_00ab9690`, `FUN_00ab9b90`, `FUN_00ab7260`; confirmed from
  the code and the disassembly; which argument is the listener is inferred from the azimuth, which uses the
  listener's axes): per voice, `d` = |emitter − listener|, `u` = the unit vector from the listener to
  the emitter. The channel's `DistanceFalloff` (`MixerChannel` +0x0c → `SoundDistanceFalloff`; field offsets as in
  the type) gives `G(d)`: inside `MinDistance` (+0x40) `PeakGain + (InitialGain − PeakGain) · t^CurveType` with
  `t = 1 − d / MinDistance` (+0x28, +0x50, +0x10), outside `PeakGain · t^CurveType` with `t = clamp(1 − (d −
  Min) / (f5bece87 − Min))` (+0x34); `FUN_00ed9e1b` is the CRT's SSE2 `pow` (exponent in xmm1, from the
  disassembly). Direction terms: with `x` = emitter forward · u and `s = (d6365a79 − 0.5) · 2` (channel +0x28),
  `E = (1 − x)/2 + (1 + x)/2 · (1 − s)` (or the first half × (1 + s) when s < 0); with `y` = listener forward · u
  and `s₂ = (Focus − 0.5) · 2` (+0x2c), `L = (1 + y)/2 + (1 − y)/2 · k · (1 − s₂)` (k = listener +0x74). Output
  slot 0 (the gain) = `clamp(E · L · G, 0, 1)`. Slot 6 = `E · L · G · H(d)` with a second curve `H` from
  +0x54 at +0x48 to +0x58 at +0x3c, exponent +0x14 (its consumer was not found). Slot 2 = an azimuth from
  `FUN_00ab5a60` (listener forward / up); slots 3 / 4 = `gain × +0x30 × +0x2c` and `(1 − gain) × +0x24 / +0x20`,
  both moved towards other values by the voice's occlusion; slot 5 = +0x1c..+0x18 over +0x44..+0x38 of distance
  (readers not found). **Doppler** (slot 1, `FUN_00ab7260`): `|c / ((v_emitter − v_listener) · u + c)|` clamped to
  0..8 (0x0103451c) with `c` by the channel's `435ec694` (+0x54): 1 → 680, 2 → 340, 3 → 170 m/s (0x01034858,
  0x01034504, 0x0103485c), otherwise none. The traffic engine channel (`…f9485`, falloff `…7277`: min 4 m, max
  150 m, curve 5, gains 1, focus 0.7, doppler 340); the competitor engines' (`…726f`: min 8 m, max 250 m, curve
  5, gains 2, focus 0.5, doppler 340); the competitor skids' (`…74cf`: min 4, max 280, curve 3.5).
- **Voice pools** (`004B0000.c`, load state 7, `FUN_00ab9140`): `Voice3dShared` 10, `Voice3dMonoShared` 32,
  `Voice3dDefragShared` 6, `VoiceMonoShared` 16 voices (profiler scope "Update Voice Pools"). How sequences take
  voices from them was not read.
- **AI racers**: one `RaceCarEngineSound` per race car entity (the same code as the player's, "Engine sound"
  above), with the competitor bank. No limit on competitor engines was found. Their skids are the surface's
  `Drift_Competitor` / `Spin_Competitor` / `Brake_Competitor` sequences (same skid bindings); the code that
  starts them was not traced. The competitor gear-change sequences bind
  `LocalPlayer.GetPlayerNormalisedGearChangeTimeElapsed`, the player's own shift clock by name.
- **Traffic car events** (`FUN_006042c0`, from `FUN_0060d290` for every traffic car within 60 m, `.data`
  0x010c4e04 = 3600 = 60²): collision sounds through the `MetalCar` set (`world+0xb17ac` +0x6c / +0x70); **horns**:
  when the car's flags (+0x60) have bit 0 or 0x200 and not 0x800 and one of two horn slots (`world+0xa2b9c`) is
  free, it plays horn `[car index % count]` of its `VehicleSound` list (`FUN_00600ed0`) and sets 0x800 (cleared
  with bits 0 / 0x200). Bit 0x200 comes from `FUN_005ebb40` called by the traffic behaviour `FUN_00ebe8c0` when
  a timer at car +0x48 exceeds 55 (`.rdata` 0x01082f08; unit not settled) against `TrafficConfig` +0x20 (with an
  argument the decompiler dropped, likely a random draw); bit 0 from `FUN_005f77e0` (car +0x5c ≤ config +0x24 once
  +0x200 passes 0.5) and `FUN_005f65a0`. **Skids** for physical traffic: six slots at `world+0xb174c`, the
  nearest car with +0x6b bit 3 and flag 0x1000 (set by `FUN_00606210`) plays its surface's sound (`FUN_0054a5d0`,
  `SurfaceSounds` +0x54 / +0x58). Not ported. The `PassbySequenceBehaviour` "TrafficPassby" objects (fast / slow
  pass-by sequences) have no reader in the dump.
- **Ported** (`car_audio.rs`): the traffic pool and instances as above, the bindings, the effect-item RPM, the
  falloff, direction and doppler maths; AI engines through `EngineDriver` with `GetDistanceToListener`; AI skids
  through `VehicleDriver`. Model decisions: the listener is the camera (its velocity from its movement, cuts
  ignored); one placement per voice (the first channel of the bank's list, the first item channel of the
  competitor drift sequence), not per channel; our pan (linear balance, centre at full level) instead of the
  azimuth; slots 3–6 and occlusion not applied (no distance filter); listener scalar k = 1; `Random` = the lane
  car's personality (crashed cars 0.5); `IsWetSurface` = the bad-weather keyframes; the RPM fallback is the idle
  RPM (every bank has an effect item); the 1.5 s release is a linear gain ramp; voices at zero gain for a block are
  not rendered (instead of the voice pools); one engine and one tyre voice per AI car; the other cars' banks play
  at the data's own level (master and channel gains; the competitor banks are not normalised like the player's,
  the competitor skids get none of our skid mix gains), so the level between the player's sounds and the others'
  is a model relation (the player's normalisation is one); the AI gear-change clock is the player's, as the
  binding names it. Not ported: pass-bys, occlusion (horns, traffic crash and skid sounds: next section).

## Traffic horns, crashes and skids (2026-10-09)

Read for the request "traffic horns, traffic crash and skid sounds as the original has them"; data in
`mw_assets::car_sfx`, runtime `crates/mostrusted/src/car_sfx.rs`. `world` = the traffic object, "car" = the 0x80-byte
vehicle slot at `world + i·0x80` (+0x00 speed, +0x48 horn timer, +0x58 blink timer, +0x5c random, +0x60 flags,
+0x69 type, +0x6a / +0x6b state bits), "lane car" = the lane follower at `world + (i + 0x37f)·0x80` (+0x22 mode,
+0x34 waiting time, +0x3c personality). `FUN_0060d290` (per frame) calls `FUN_006042c0` for every car within
60 m of the local player's car (`.data` 0x010c4e04 = 3600), with the car's 0x200-byte sound emitter at
`world + 0x69d30 + i·0x200` as `this` (disassembly at 0x0060d590); every sound below plays on that emitter.

- **Horn wish** (`FUN_00ebe8c0`, the traffic behaviour, run by `FUN_00ebeb00` only while the car is within
  `TrafficVehicleConfig.6e54f849` = 150 m of the behaviour's reference point, `param_1[0x2c]` = 150² from its
  constructor; confirmed): if the timer (+0x48) ≤ 55 (`.rdata` 0x01082f08) or the lane car's mode (+0x22) ≠ 0:
  clear 0x200, timer += frame time (the time `FUN_005f53b0` blinks the lights with: seconds); else `FUN_005ebb40`:
  timer = 0 and, when the lane car's personality (+0x3c, `[edi+0x3c]` in the disassembly) ≤ `TrafficParamConfig
  .831be501` (+0x20, 0.4), set 0x200. In `snap2` the timer is FLT_MAX on every car not yet checked (31.1 and
  20.6 s on the two that had been), so a car honks the first time it stops for an obstacle (mode 0, the arbiter's
  "stop for the obstacle": the player's car or crashed traffic) with a 40 % chance, then at most every 55 s.
  Bit 0 is the manoeuvres' horn: the swerve (`FUN_005f77e0`, `SwerveManoeuvre` fields +0x1a8..+0x1e0) and
  `FUN_005ffc10` set it when their phase timer (+0x200) passes 0.5 s and the car's random +0x5c ≤
  `TrafficParamConfig.fa75a686` (+0x24, 0.7); the jack-knife (`FUN_005f65a0`) holds it through some phases. (The
  research log's "TrafficConfig +0x20 / +0x24" is this `TrafficParamConfig`, `DAT_0110f940`.)
- **Horn sound** (`FUN_006042c0`): with bit 0 or 0x200 set and 0x800 clear, and horns in the type's competitor
  `VehicleSound` (+0x64 count), the first free of two horn slots (`world+0xa2b9c`, freed by `FUN_0060d290` once
  `FUN_00a422d0` reports the voice gone) plays `horns[car index % count]` (`FUN_00600ed0`; `FUN_005ecd70` stores
  the voice) and sets 0x800, cleared when neither bit is set. Six distinct horn timelines over the 11 traffic
  types (1–3 per type): a long blast with click-in / release items and a `JumpTimelineController` that skips the
  rest, unconditionally in two (one blast), on `Signals.GetRandomNumber[100](LTEQ,50)` in four (one blast, or the
  blast and up to three short ones). Channel `…076b23`: min 5 m, max 120 m, curve 3.94, peak gain 5 (clamped to 1:
  full level to ~45 m), focus 0.6, doppler 680.
- **Wrecked traffic and car alarms** (`FUN_006042c0`, not ported): a car whose race-car entity's damage bar is
  full (+0x1180 ≥ profile +0x74) plays the global `VehicleSound.385b1faf` list round robin (`world+0xb1731`; four
  timelines: a crunch, then a 1.5 s loop by a jump 12 → 10.5 s) and sets 0x400. A parked car (+0x6a low nibble
  1) within 10.5 m (`.rdata` 0x00fb2884 = 110.25) of the player, with `FUN_00577190` below 6 (0x00fb1b58; not
  identified) and a flag of the player car's data (+0xf6), or with bit 0x10, plays the alarm `MiscSound.c1109b6e`
  (an 8 s timeline) when `(car index % 10) × 0.1 ≤ MiscSound.7f18169d` (0.4: half the cars) and blinks the hazard
  lights (0x20: 15 s at 0.4 s, `FUN_005f53b0`).
- **Traffic car contacts** (`FUN_006042c0`, physical cars only, +0x6b bit 3): of the car's contact records
  (`FUN_0090d610` count, 0x30 bytes), the one with the largest +0xc above `VehicleSound.392b1040` (the global
  `VehicleSound`, `world+0xb270`; 2.235, read as m/s = 5 mph) whose other car (+0x28) is not nearer to the player
  than this one plays `CollisionSoundSet` (`SOUND/METALCAR.BNDL`, `world+0xb17a8`) slot +0x6c (`333a28fc`, the
  competitor light front impact) through the emitter's `vtable+0x54`. World contacts (bitset `+0x5de10`) go to
  `FUN_0057caf0`.
- **World impacts** (`FUN_0057caf0`, traffic and race cars): the contact's surface (`record[3] >> 13 & 0x7f`) gives
  `SurfaceType.Collision_Sounds` (+0x1e0, `FUN_0054a4f0`; seven sets over the surfaces). Not crashing (entity
  `vtable+0x58`, byte +0x1c2b, false): an impact above 2.235 plays one wave of the set's pool `7e01f1e6` (+0x118,
  round robin) on the channel set's +0x60 with gain `0.714 + 0.286 × clamp((|v| − 30 mph) / 70 mph)` (|v| the car's speed;
  `.data` 0x010c4730 = 0.44704, `.rdata` 0x00faba74 / 0x00faba78 / 0x00fae1c8 / 0x00faba7c), and for the player
  the set's slot +0xf4. While the car is crashing: zone sounds from +0x154..+0x164 (next section; not ported).
  The traffic channel set (`world2 + 0x12a00`) equals the competitors' (`+0x12950`) in `snap2`; the names come from
  `FUN_004b5f40` (+0x60 = `Competitor_World_Collision`, a channel of the mix snapshot `Mix_Main`).
- **Race car impacts** (`FUN_00591ac0` per race car → `FUN_0057e920`), thresholds in the global `VehicleSound`:
  against a race car by `s = max(this car's value, the other's)`: heavy above `edc5149f` (+0xcc, 30), medium from
  `ad43123c` (+0xfc, 14), else light; skipped when both are ≤ 1.19e-7; without a human player in the pair only
  the car with the larger value plays it; a human player's object always plays it (the AI side stays silent).
  World contacts, at most every `567b6cc3` (0.1 s): `FUN_0057caf0`, then, if the surface has a set, medium above
  `89672c4c` (+0xc8, 30), light above `945a7617` (+0xf8, 14). Direction (`FUN_00570b60`): the contact in the car's
  frame against its forward axis: front above cos 30° (`.bss` 0x0110d0f0 = 0.866 in `snap2`), rear below −0.866,
  else side. Slots (competitor | player): heavy front / rear / side +0x34 / +0x3c / +0x44 | +0x0c / +0x1c / +0x24;
  medium +0x54 / +0x5c / +0x64 | +0x9c / +0xa4 / +0xac; light +0x6c / +0x74 / +0x7c | +0xd4 / +0xe4 / +0xec. A
  player's heavy impact adds a wave of pool +0x108 (`a8f44b4f`); a crash (flag +0x1c2d) forces heavy and adds pool
  +0x150 (`0960076c`) and slot +0xb4. The level also sets a value from `VehicleSound` +0xe4 / +0xe8 / +0xec
  (0.067 / 0.042 / 0.009; its reader not traced).
- **Snapshot channels**: an item without `Mixer_Channel` names a channel of the mix snapshot in `Snapshot_Property`
  ("Competitor Vehicle Collision Small" → the field `Competitor_Vehicle_Collision_Small` of `Mix_Main` `…72a0`,
  from `MiscSound.20c447e8` → `MixSnapshotSet.External_Camera`): gains 0.259 / 0.399 / 0.633 (small / medium /
  big), `Competitor_World_Collision` 0.794 (falloff `…7273`: min 4 m, max 250 m). Names added to
  `genesys_names.txt`.
- **Traffic skids** (`FUN_0060d290`, after the events): six slots at `world+0xb1748` (+0 time, +4 voice, +8 car,
  +0xc state). A playing slot whose car is no longer physical or lost flag 0x1000 stops with a 0.1 s release
  (`FUN_00a44e40(voice, 0.1)`, `.rdata` 0x00fb1a94). Then the car nearest the player (within 60 m), if physical
  with 0x1000 and without a slot, starts its wheel surface's `SurfaceSounds.6bdefb10` (+0x54, `FUN_0054a5d0`):
  a timeline with one looping wave (0..4 s, fade-in 0.5 s, gain 3) and a jump 4 → 1 s, so it loops until stopped.
  Flag 0x1000 (`FUN_00606210`, the physical cars' wheel effects): wheel 0 leaves marks (+0x1be, with a tyre-mark
  set for its surface) and the body's speed² exceeds `.bss` 0x0110f580 = 25 (read in `snap2`).
- **AI racers' skids** (`FUN_0058f6d0`, from the surface update `FUN_0058fe10`; competitors walk wheels 0 / 2
  only, the player all four): state 0 starts `Drift` (+0x1c) or, for a non-player, `Drift_Competitor` (+0x24), then
  `Brake` / `Brake_Competitor` (+0x0c / +0x14); 1 → 2 swaps the second for `Spin` / `Spin_Competitor` (+0x44 /
  +0x4c) when `FUN_00573d70` turns false, 2 → 1 back, each swap releasing the old voice over 0.15 s (`.rdata`
  0x00fad428). `FUN_00573d70` = |vehicle +0x2d58| > 0.01 (0x00f9d474 / 0x00fa934c), a field not identified. The
  road sequences are `SurfaceSounds` +0x34 (player) / +0x3c (others). Not ported (ours run all three competitor
  sequences from their bindings).
- **`JumpTimelineController`** (now run by `mw_audio::sfx_player`): when a timeline's clock crosses `TriggerTime`
  and `EnabledBinding` holds (empty = always) the clock moves to `b6a51260`. `Signals.GetRandomNumber[n]` is drawn
  when the jump is reached (assumed uniform 0..n−1); the comma test form `(LTEQ,50)` is parsed.
- **Ported** (`car_sfx.rs`): horn wish and sound as above (chance from the data), two horn slots, `horns[slot %
  count]`; crashed traffic's car contacts (slot 12, nearer car of the pair, never against the player's car) and
  world contacts (the surface wave); AI racers' car-car and world / traffic contacts with the levels, directions
  and slots above; the six traffic skid slots. Model decisions: our contacts are every physics step's, not impact
  records, so every impact (AI against AI too) needs the 2.235 m/s closing speed and is spaced 0.1 s per car (at
  the game's eps test AI cars leaning on each other started a sound every 0.1 s: 624 skipped in a race); closing
  speed for the game's impact values (one per pair, so the faster AI car plays an AI–AI impact); the default
  surface for every impact wave and skid; no scrape branch; AI against traffic as a world contact; props excluded
  (their own sounds, `props.rs`); a crashed car "skids" when it touches the world faster than 5 m/s (our crashed
  cars have no wheels to mark); the behaviour radius measured from the player's car; at most 8 impact voices,
  impacts at zero gain not started; one placement per voice (its first wave item's channel or snapshot channel).
  Not ported: manoeuvre horns (no manoeuvres), wrecked-traffic sounds, car alarms, the AI skid state machine. The
  player's horn and impacts by this table: next section (2026-10-10).

## Traffic reactions (2026-10-15)

Read for "horns and headlight flashing when you drive towards them"; ported in `traffic_reaction.rs`. Car = the
0x80-byte slot as above; "AI vehicle" = the 0x620-byte physical-traffic object at `world+0x2c910 + k·0x620`
(+0x4c car index, +0x1fc request, +0x1fd manoeuvre, +0x1d3 trailer, +0x200 physical time).

- **Becoming physical**: `FUN_0061cf50(car, link, player, …, request, …)` takes a free AI vehicle (`world+0x32b20`
  list, 16), sets car +0x6b bit 3 and calls `FUN_00609b70`, which forces request 2 for any car whose +0x6a low
  nibble is not 0 (parked) and runs `FUN_006071d0(request)`. Callers: the contact loop in `FUN_0060c3a0` (world
  vehicles with contacts; request from `FUN_005f2eb0`), the queues in `FUN_0060e030` (the arbiter's option 0 →
  `world+0x32bac` with request 0; a per-player list at `world+0x36120 + p·0x680` +0x6b0 / count +0x770 whose writer
  was not found; message 0x10 entries from `FUN_005efdd0` → `world+0x32b30`).
- **`FUN_005f2eb0`** (the contact's request): `FUN_00929da0` zone of the contact point (+0x10 of the contact
  record) on this car's box, stored at car +0x71; 4 when the record's +0x2c > 30 mph (`.rdata` 0x00fb1ec0), the car's
  speed (+0x00) > 40 mph (0x00fb1ec4) and the zone is 5–9; else 6.
- **Manoeuvre choice** (`FUN_006071d0` → scores `FUN_005eac20`, the best of 11 wins; the objects at
  `world+0x339d0` get their vtables in `FUN_00420720`): request 2 / 6 → 1; request 1 → 1 by fuzzy memberships;
  3 / 4 / 5 → 6 (input +0xd0 > 0.66), 2 (> 0.33) or 4; with a trailer 3 / 4 / 5 at speed → 5 (jack-knife) and 0 →
  1; score 0 is always `fuzzy[0x510]` (0.2), so request 0 gives manoeuvre 0. Update functions (vtable +8): 0
  `FUN_00604290`, 1 `FUN_005f5c40`, 2 `FUN_005f77e0`, 3 `FUN_005f6150`, 4 `FUN_005f6e30`, 5 `FUN_005f65a0`, 6
  `FUN_005ffc10`, 7 `FUN_005f6aa0`, 8 `FUN_005f73a0`, 9 `FUN_005ebca0`, 10 `FUN_005e9260`; start +0xc. Each
  update returns −1 (stay) or the next index (`FUN_005eb500`); `FUN_005ffa10` adds dt to +0x200 first and switches
  to 10 when the damage bar is full.
- **Manoeuvre 1** (`FUN_005f5c40`): standing (|v| components ≤ 0.5 or moving away) → stop flags; else steering /
  speed targets, and "moving" when parked or faster than 30 mph (`.data` 0x010c4d0c). Parked and moving → bit 0x10
  (unless 0x20); otherwise car +0x48 = 0 and bit 0 when car +0x5c ≤ `TrafficParamConfig` +0x20. Returns 0 once
  +0x200 > 5 s for a lane car; a parked car stays.
- **Car +0x5c**: a draw of the second generator (`world+0x361c0`, ring `+0x361a0`) when `FUN_00608fb0` gives the
  car its world vehicle (`FUN_005f50b0`), separate from the lane car's personality (+0x3c).
- **Lights** (`FUN_00ebe8c0`, then `FUN_005f53b0(dt, int(TrafficVehicleConfig +0x28 × dt))`; parked cars:
  `FUN_0060b9a0`): +0x6f = 1 → `FUN_00614620(1, rng)` (sets 0x100 | 8, step 0, pattern = high word % 3), else
  0x100 cleared; a non-physical lane car clears 8 and 0x100. `FUN_005f53b0`: bits 0x30 count +0x4c; 0x20
  (hazard) between 0.5 and 15 s toggles 8 every 0.4 s (`.rdata` 0x00fae200); 0x100 steps the pattern table
  (`.rdata` 0x00fb2168, 3 × 9 floats; even steps clear 8, odd steps set it, a step ≤ 0 ends and clears 0x100 | 8);
  +0x64 moves by the step towards 255 (bit 8) or 0, held at 0 for a parked car outside the hazard. 0x40 / 0x80
  (indicators, from lane car +0x72 / +0x8 / +0xc in `FUN_00ebe8c0`) toggle 2 / 4 at a period from
  `TrafficVehicleConfig` +0x80 / +0xac and drive +0x65.
- **Drawing** (`FUN_005fd190`): per car, its light objects (type 0x2aab, up to two) are queued (`FUN_004970b0`,
  96 records of 0x70) with intensity `+0x64 / 255 × 0.5 + 0.7` (0 for parked cars and manoeuvre 10) and a per-car
  factor `0.9 + 0.2 × +0x5c`.
- **Alarm** (`FUN_006042c0`): bit 0x10 (or `FUN_005f2740`, a 10.5 m proximity test) plays `MiscSound` +0xb8
  (`c1109b6e`) when the car's `TrafficVehicle` +0x3c flag is set and `(car index % 10) × 0.1 ≤ MiscSound` +0xe0;
  then starts the hazard (0x20, +0x58 = 0.4) and clears 0x10.

## The player's horn, impacts and scrapes (2026-10-10)

Read for the requests "the player's horn" and "scrape / grind sounds against walls and other cars"; data
`mw_assets::car_sfx::PlayerCarSfx`, runtime `crates/mostrusted/src/player_sfx.rs`. "Entity" = the 0x1c40-byte
`RaceCarEntity` (`*(*0x010edae4 + 4) + 0x3050 + i·0x1c40`, vtable 0x00f99f98 in `snap2`); its vtable +0x50 plays a
wave (`FUN_005756e0`), +0x54 a sequence (`FUN_005874e0`), +0x58 = byte +0x1c2b (the crash state, set by message 0
through `FUN_00570bf0`), +0x5c = |+0xc84| > ε. `FUN_00594870` (per frame) builds a block for `FUN_00591ac0` from the
global `VehicleSound` (`SOUND/GLOBAL.BNDL`): +0x00 `1dc7d3aa` (0.247), +0x10 `f5ddbc19` (0.106), +0x20 `bf4ff152`
(1), +0x30 `e3d31382` / 2 (4), +0x40 `e40a7d46` (0.278), +0x50 the round-robin bytes, +0x5c the `VehicleSound`.
The channel set for the player (`FUN_004b5f40`, base +0x127f0) is +0x00 `Landings_Large`, +0x08
`Roll_Whoosh_Player`, +0x10 `Player_Vehicle_Collision_Big`, +0x18 `Player_Wrecking_Collision`, +0x20 `…_Bass_Hit`,
+0x28 `…_Glass_Smash`, +0x30 `…_Prop_Collision`, +0x38 `…_Scraping`, +0x40 `…_Sirens`, +0x48 `Sirens_Roadblock`,
+0x50 `…_Debris`, +0x58 `…_Suspension_Hits`, +0x60 `…_World_Collision` (all channels of the mix snapshot
`Mix_Main`). `CollisionSoundSet` pools are `u32` pointers at +0x108.. with `u16` counts at +0x168..: +0x118
`7e01f1e6`, +0x11c `1ae351a5`, +0x140 `97b6a045`, +0x148 `Scratch`, +0x14c `Sparks`, +0x154 `3578b364`, +0x158
`5da2551f`, +0x15c `1db229d6`, +0x164 `1cb422d5`.

- **Horn**: HORN is action 5 (keyboard W, pad B; "Input and control bindings"). The player's list is
  `<id>_VEHICLESOUND` +0x60 / +0x64 (the 911: one sequence `…1d1c69`, a 0..5 s wave item, gain 1.75, on `Player
  Vehicle Horn`). Found 2026-10-16: `FUN_00587720` plays list [0] on the horn input (+0x1c25) through
  `FUN_00587650` and releases it with a 0.1 s fade (0x00fad540); a car with a siren steps its siren instead
  ("Police sirens, light bars and radio").
- **Impacts** (`FUN_00591ac0` for the player's entity): as the AI racers' (previous section) with the player's half
  of the slot table: heavy front / rear / side +0x0c / +0x1c / +0x24, medium +0x9c / +0xa4 / +0xac, light +0xd4 /
  +0xe4 / +0xec; a heavy one adds a wave of +0x108 (`a8f44b4f`). A world contact runs `FUN_0057caf0` with the
  player's channel set (the wave on `Player_Vehicle_World_Collision`) and, for the player, the surface set's +0xf4
  (`fd3bb937` = `…0b3935` in every surface set: a `RaceCarEffectSequenceItem` timeline automating race-car
  properties 1 and 3, no sound; not ported).
- **The crash branch of `FUN_0057caf0`** (the old note "scraping: a state machine over +0x154..+0x164,
  `FUN_00929da0`" was wrong): it runs when the entity is crashing (vtable +0x58) and the surface has a set.
  `FUN_00929da0` classifies the contact point against the car's box (0..10); with the car upright (`row2 · c ≥
  .bss 0x0110cfe0`) and |`row0 · c`| above `.bss 0x0110c900` the zone is 5, not upright 0; a zone plays when it
  differs from the last (+0x18d) or, upright, on an impact above 2.235: 0 → +0x15c, 1 → +0x158, 2–4 → +0x118, 5 / 9
  → +0x164, 6–8 → +0x154, on the channel set's +0x18. Tumbling sounds; not ported.
- **The grind** (`FUN_0057f2e0`, after the contacts): for each of the car's object contacts (`FUN_00904880` /
  `FUN_00904850` on vehicle +0x5e098, 0x70-byte records, other object = shorts +0x60 + +0x62) with |+0x2c| ≥ `.bss`
  0x0110cac0 (0.05 in `snap2`): the same object as the playing grind (+0x1a68) keeps it (timer +0x1afc = 0); a new
  object that is a race car (`FUN_005766f0`, one of the 16) or in the traffic vehicle table (`FUN_005ea300`,
  `world+0xa2bcc` .. +0x1af: traffic and parked cars), when this is the player (+0x1c18) or the other is no human
  player, stops the old voices (`FUN_00aba570`, fade 0.05 s, `.rdata` 0x00fac2b4), starts a wave of +0x140
  (`FUN_00576a20` case 0, round robin byte +5) on the set's +0x38 (`Player_Vehicle_Scraping`), sets +0xc30 =
  −block+0x30 and plays sparks (`FUN_00575750`: when +0x1ae0 > `567b6cc3` (0.1 s), a `Sparks` wave on +0x50, pitch
  `1 + (r − 1) · 0.3` with r ∈ [1, 2) from the random ring, +0x1ae0 = 0). Keeping it: pitch (voice vtable +0x34) =
  clamp(R^(2x − 1), 1/R, R) with R = block+0x20 (1) and x = clamp(vehicle +0x1620 / +0x7e8, 0, 1) (`FUN_0059f6a0`;
  read as speed / top speed); gain (+0x28) = `powf(1.09, −(+0xc30) − block+0x30)` (`FUN_00ed9e1b` is the CRT
  `powf`; `.bss` 0x0110ce70 = 1.09); +0xc30 grows by dt every frame (`FUN_00591ac0`), so the gain is 1.09^(−t), t
  since the grind started. When the voice's remaining time (voice +0x24 − +0x20) is under block+0x40 (0.278 s) a
  new wave starts and the old one fades over that time (state 2, +0x50 / +0x54). After the loop, while +0x1afc <
  0.1 s the gain is that × (0.1 − +0x1afc) / 0.1; after it both voices fade out over 0.05 s.
- **Damage sounds** (`FUN_00585030` → `FUN_00576220`, when the vehicle's damage block (+0x1110) has +0x71 set and
  this frame has a world contact (bitset +0x5ddec) or the last vehicle contact of kind 1 / 7 / 8 was kind 7 (a race
  car: the object table `DAT_010edb68` entry's `u16` +6, low byte; 0x307 = race car, 0x208 = traffic slot, read in
  `snap2`)): `FUN_00585030` keeps per damage-grid cell (15, entity +0xab0) the last scratch level min(raw × scale,
  1) and a sticky flag `raw > threshold[cell]` (`.bss` 0x0110c990: 0.01 / 0.15 / 0.01, 0.25 / 0.15 / 0.25, 0.25 ×
  6, 0.01 / 0.25 / 0.01 by row from the back), and returns the largest rises with the cell centres (0x0110cc90,
  x −1..1, z −2..2) mapped onto the car's box. `FUN_00576220` (player only, +0x1c18): a flag rise above
  `b0407c0b` (0.121) plays a wave of +0x11c on the set's +0x28 (`Player_Vehicle_Glass_Smash`), gain `(Δ − 0.121) /
  (1 − 0.121)`; with a race car the last contact, a scratch rise above block+0x10 (0.106) plays a `Scratch` wave on
  +0x50 (`Player_Vehicle_Debris`), gain `(Δ − 0.106) / (1 − 0.106)`, both at the cell's position.
- **No wall-scrape loop**: walls give only `FUN_0057caf0` impact waves, at most every 0.1 s while the contact value
  is above 2.235, and the damage sounds. The old `sfx.rs` scrape loop (the `Scratch` waves looped by the sliding
  speed, sparks above 5 m/s) had no source and is gone.
- MODEL decisions (`player_sfx.rs`): the contact value (record +0x2c, unit not traced) is "the pair pushed this
  physics step" (a positive normal impulse); closing speed for the impact level; the default surface; per-step
  contacts spaced 0.1 s; the voices at the car (the game places them at the contact or cell); 12 one-shot voices
  at most; the grind's `x` from the car's speed and top speed.

## World-component sounds, landings and the crash's contact sounds (2026-10-16)

Read for the play-test item "missing sound effects" (billboards, light poles, light crashes, landings); data
`mw_assets::world_sfx`, runtime `crates/mostrusted/src/world_sfx.rs`. The thresholds come from the global
`VehicleSound` at `[DAT_010edb08 + 4] + 0xb270` (confirmed: `FUN_00594870` reads +0xa8 = `1dc7d3aa` from the
same handle): +0x94 `0925bb50` = 1, +0x98 `392b1040` = 2.235. The "contact value" every rule compares is the w
of a contact record's second vector (a pair record's +0x2c, the world record's +0x1c), not traced; read as the
closing speed (2.235 m/s = 5 mph) as before.

- **`WCPlaySoundBehaviour`** (type 0x2ad8, 80 bytes; layout in formats.md): 41 objects in the install. A
  `PropObject` names one at +0x1c (`FUN_0054e8d0` hands it the prop's contacts); compound objects (58 of them),
  dynamic objects (46) and compound parts (125) carry them in their behaviour lists (the old port played only
  props' and parts' `Collision_Waves`, on no channel, gain by speed: no source). The 11-part panel compounds (the
  216 billboards) have none.
- **Props** (`FUN_0054e8d0` per frame): each body-pair record → `FUN_0054bf70`: (1) the first record whose force
  passes `ForceToDamage` with a vehicle (`FUN_00573360`, the 16 race cars) starts the impact effect, and when the
  prop has a sound (`FUN_0055f310`), the vehicle's speed² is above `.bss` 0x0110c420 (400, `snap2`) and |value|
  > +0x98, queues type 2; (2) while the prop's timer +0xbc is 0, |value| > +0x98 with a vehicle queues type 5;
  every record sets the timer to 0.6 s (`.rdata` 0x00faaf30). The world record → `FUN_0054c680`: while +0xc0 is
  0 and |value| > +0x94: type 3 when the prop's speed² (vtable +0x10) is above `.bss` 0x0110c1a0 (25) and the
  first surface entry's `e8986e0a` list is not empty, else type 4 when its `Waves` are not; +0xc0 = 0.35 s
  (`.rdata` 0x00faaf2c). Both timers count down per frame (the tail of the prop update `FUN_00548ff0`). The channel
  is the behaviour's `MixerChannel` (+0x30), else, for car hits, the vehicle's channel set +0x30
  (`FUN_00572430`: the player's `Player_Vehicle_Prop_Collision`, else the set at +0x12950) and, for world
  contacts, `"World_Prop_Collision"` (`FUN_00410440` / `FUN_009ed180` hash the name). `FUN_005491a0` queues the
  request at the object's position + the behaviour's `Offset` (+0x20), gain 1.
- **The behaviour's runtime** (vtable 0x00faf308: slot 2 `FUN_005a7d20` off, slot 3 `FUN_005b8490` update,
  slot 8 `FUN_005ab070` pair, slot 9 `FUN_005b10f0` world): with +0x16 (`0228a2f8`, set on all 41) and state
  +0x28 ≤ 1: a pair contact above +0x98 with a vehicle queues type 5 (state 2) and raises "activate"; a world
  contact above +0x94 queues type 4 (state 3). Slot 3 returns the state to 1 once no emitter lists the object
  (`FUN_004b26b0`), i.e. when its wave ended. No pinned test: a standing lamp a car bumps plays.
- **Emitters** (`FUN_004b37e0` queues 0x30-byte entries at `+0x5e50` (count byte +0x6a50): object id, type,
  behaviour, position, channel, gain; `FUN_004ba990` sorts and hands them out; `FUN_004b2530` keeps 8 emitters
  of 0x180 bytes): an entry joins the emitter of the same (type, behaviour) started less than 0.1 s ago
  (`.rdata` 0x00fa339c) without a new wave (the position moves to the newest object); else a free emitter, else
  the one with the lowest type below the new type is stolen. `FUN_004b7b20` starts the wave: type 2 the first
  `PropSurfaceSound`'s `71f062ee` (+0x18), 3 `e8986e0a` (+0x14), 4 `Waves` (+0x1c), 5 `Collision_Waves` (+0x38;
  `8dfd5f40` +0x3c instead when three or more objects share the emitter, empty on all 41); the index is a global
  counter (mod 1024) modulo the list's length. `FUN_004ba1c0` keeps the voice's position and gain each frame and
  frees the emitter when it ends.
- **A car's first touch of a prop** (`FUN_00591ac0` → `FUN_00587960` per object contact of the vehicle): above
  +0x98, for a non-traffic object (`FUN_005ea300`), when more than 0.1 s (+0x110) passed since the last
  (+0x1adc) and the touch has just started (+0xc78 = 0: `FUN_005767a0` tracks the touched object at +0x1a6c,
  resets the time +0xc78 for a new one, drops it 0.1 s after the last contact; race cars are not tracked) →
  `FUN_00576440`: MetalCar `CollisionSoundSet` pool +0x130 `939c92be` (front, `FUN_00574150` with cos 30°),
  +0x138 `d02a49d8` (rear, `FUN_00571c20`), else +0x134 `ae166d1e`, on the set's channel +0xc4 (`b26e4dfe` =
  `…7357a`), gain 1, at the contact point; the player also starts sequence +0xbc (`…b3934`: race-car effects
  only); while crashing, the player once per crash (+0x70b) pool +0x150 `0960076c` on channel set +0x18
  (`Player_Wrecking_Collision`) and sequence +0xb4.
- **Landings** (`FUN_00576b40`, per frame after the contacts): per wheel (sim +0x870, 0x1d0 apart; grounded
  byte +0x24) a hold timer at entity +0x1b6c.. set to 0.05 s (`.rdata` 0x00fac2b4) while grounded and counted
  down otherwise (`FUN_00591ac0` tail). The first wheel that is grounded with its hold at 0, for the player or a
  crashing car (+0x1c18 / +0x1c2d), plays a wave of its surface's `CollisionSoundSet` (`FUN_0054a4f0`, the tag's
  bits 13–19): `Landing` (+0x124) on the channel set's +0x00 (`Landings_Large`) or, crashing, +0x158
  `5da2551f` on +0x18 (plus `FUN_00575880`: pool +0x128 `d03c72cd`, empty in every surface set), gain
  `(1 − a) · 0.35 + a`, `a = min(s, 2.5) / 2.5` (`.bss` 0x0110cd80 / 0x0110d0c0, `snap2`), `s` = the larger of
  the last frame's velocity (+0xc90, written after the call) along the wheel frame's rows 0 and 2
  (`FUN_005423d0`). When no wheel's hold was running (all four were in the air) and one landed, `TimeInAir`
  (+0x1ae4, reflected "RaceCarEntity"/"TimeInAir") picks a sequence of that set: the player ≥ +0x88
  (`3453bdbb`, 1 s) → +0x14 `d12772a1`, above +0x110 (0.1 s) → +0xdc `929af217`; others above 0.1 s → +0x4c.
  `TimeInAir` grows while no wheel is grounded and is 0 otherwise. Data: the default set's long sequence
  (`…73590`) is a 3 s `Landings Large` wave plus bus / race-car items; its short one (`…95b27`) has no wave.
- **The crash branch of `FUN_0057caf0`** (the crash state, vtable +0x58, and a surface set): `FUN_00929da0`
  classifies the contact against the car's box (point in the box frame / half extents, scaled to the largest
  axis: |p| ≤ 1.2e-7 → 10; x > 0.5: front (z > 0.7) 2, rear (z < −0.7) 8, else 1 on the underside edge (y < −0.8
  and |y| > 0.8 + 0.4 (|x| − 0.5), `.data` 0x010cf910 × 0x010cf914) or 9; |x| ≤ 0.5: 3 / 7, y > 0.8 → 0,
  y < −0.8 → 1, else 10; x < −0.5: 4 / 6, 1 or 5). With the car's up axis y below −0.9 (`.bss` 0x0110cfe0) the
  zone is 0, with |right.y| above 0.9 (0x0110c900) 5, each played only when it differs from the last (+0x18d);
  otherwise the classified zone plays when the value is above +0x98. 0 → +0x15c `1db229d6`, 1 → +0x158, 2–4 →
  +0x118 (the world-impact pool), 5 / 9 → +0x164 `1cb422d5`, 6–8 → +0x154 `3578b364`, on +0x18, gain by speed
  (the world wave's). The non-crash branch (the world wave, the player's slot) does not run then.
- **The crash camera's sequence** (`…68b43`) has no wave items: its audio is automation, slow motion aside: three
  `BusMixerChannelSequenceItem`s (buses `9469eeda` = 556003 to 0 between 0.32 and 3.8 s, 259953 down to 0.6,
  259945 held at 0.6) and two `MixerChannelSequenceItem`s (`…74d2` parameter 0 at 0 for 5 s, `…735a3` property
  5 by a curve). Which buses those ids name was not found (the 23 `BusMixerChannel` objects carry other unique
  ids); not ported.
- **Collectable messages**: the scoring actions' sequences carry waves (`995814` "+SP" line: `…18c1fd` at 0 s;
  `1107082` speed camera: `…f690e` at 1.333 s, `…f690d` at 5.967 s; `995322` gate panel: the same two at 0.5 /
  5.967 s); played at their item times with the item's gain.
- Model decisions (`world_sfx.rs`): the contact value = closing speed; the AI's prop channel =
  `Competitor_Prop_Collision` (its channel set +0x30 not read); the landing gain's axes = the car's right and
  forward (the wheel frame's rows were not identified); the crash zones use the default surface's set and the
  body collider as the box; the collectables' waves are 2D; a prop's "damaging" contact is our impact-effect
  test (`ForceToDamage`).

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
- **`FUN_00eb9500`** (the vertex builder, per group): skips the group unless the eye is within
  `group + 0x3C` × 125 (`DAT_010c6f74`) of its box and `FUN_00eb8290` (box against the six frustum planes)
  passes; then per instance: position = box min + u16 × (max − min) × 1/65535 (`DAT_00f8f370`); kept if
  `(p − eye) · param_7 ≥ 0` (in front of the camera plane), `FUN_00eb8290` passes for it and |p − eye|² ≤ R²
  with R = (byte)((h >> 1 & 63) + (w >> 1 & 63)) × 2^e × 1/127 (`DAT_01081004`) × 125; f = (1 − d²/R²) × 5
  (`DAT_01081000`, cached in `DAT_011b4904`), at most 1 (`DAT_00f8a820`); the width / height bytes become
  `(char)(int)(byte × f)`. It writes four 0x2C-byte vertices (formats.md) into a scratch buffer and copies them
  to the frame's vertex ring (`FUN_00ac69f0`); the draw record (0x10 bytes) holds the ring offset, the material
  record (`collection + 0x30` + material × 0x110), the vertex count, e (+0xC) and two sort bytes. Caps: 20000
  sprites a frame (`DAT_010c6f70`, counter at `param_1 + 0x610`) and 300 records (`param_11 + 0x12c0`). The
  `Groundcover` vertex program then turns each sprite to the camera (formats.md, ported in
  `groundcover.wgsl`, 2026-10-09).
- **Our port** (`groundcover.wgsl`): the builder's per-sprite step and the vertex program run together in
  one vertex shader over static per-group meshes (4 vertices per sprite at its base). Model decisions: the
  shader `Time` is taken as seconds (not read live); the 20000-sprite / 300-record caps are not applied; the
  range, facing and in-front test use the view being drawn (the game builds once a frame for the main
  camera); the group test stays on the CPU (`groundcover.rs`).
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
- **`SuperEmitter`** (0x2B; compile `0x0044b2c0` → `FUN_00448370`, a copy of words 0..0x13 plus the byte of
  word 0x14 at +0x54; run `0x0045dc40` → `FUN_0045d020`, read with REA): the parent layer is the high bits
  (>> 23) of the parent-channel operands at words 9..12 (0x80 | 4·channel: position, velocity, age, 1 /
  lifetime); own position / velocity / radius at words 13..15. Per live parent (`t = age · recip < 1`, a LOD
  byte against `.rdata` 0x00f9cf9c): `n = round((1 + (w19 − 1)·t)·(w17·|v·dt| + w16·dt) + carry)`, the
  remainder carried in the compiled word 0x14 (one for the action). Each child: `u` uniform, position = parent
  position − u·v·dt; direction at polar angle `rand(w0, w1)`° from Z = normalize(−v·dt) (fallback +Y) about a
  uniform azimuth (X = normalize(Z × up)), speed `rand(w2, w3)`, plus `rand(w4, w5)` × the parent's velocity;
  radius `rand(w6, w7)` × `(1 + (w18 − 1)·t′)` with t′ the parent's age `u` steps back.
- **Renderer draw words** (`FUN_00458e90` 0x24, `FUN_0045ab10` 0x27, `FUN_0045b620` 0x28): two words per
  renderer go to the draw state +0x23c / +0x240 (reset to 0 / 1.0 per draw): 0x24 words 7 / 8, 0x27 words 9 / 10,
  0x28 words 0x11 / 0x12. `FUN_0046ce40` copies them into a 0x80-byte draw record (+0x50 / +0x54, with the
  matrix at +0x00, +0x200 at +0x40, the min depth +0x220 at +0x58, the layer byte +0x60, the pass byte +0x61, a
  priority +0x62) queued through `FUN_00a1a180`. **The particle draw loop** `FUN_004a40a0` ("BearRenderWait_Render",
  from `FUN_004a4940`, render stage 6) sorts the records by depth and, per record: for passes 0x23 / 0x24 / 0x29 /
  0x2a `FUN_004976d0(+0x50)` writes `gDepthFadeConstantsA` / `B` (slots 0x72 / 0x73; the soft programs' depth
  fade); if +0x5c, slot 0x77 = +0x40; then **`gVfxConstants` (slot 0x75) = base + (0, 0, +0x54, 1)** with base
  `.data` 0x01104120 = (0, 1, 0, 0) or, for layer 6, 0x011035e0 = (1, 0, 0, 0) (snapshot reads), and
  `gVfxSparkBrightness` (0x79) = view +0x13a0 (layer 0x1a goes to `FUN_00497800`). So the +0x240 word is
  `gVfxConstants.z` (confirmed 2026-10-16): the pixel programs' alpha factor and alpha test, and the vertex program's
  key-light factor; .x blends the colour towards × the exposure texture's w, .y blends the vertex program's LDR
  encode (sRGB-like curve of colour × 0.5) to the linear result.
- **Layer and pass** (`FUN_0046a430`, called by the compile functions `FUN_00447e40` 0x24, `FUN_00448020`
  0x27, `FUN_00448110` 0x28, `FUN_00448220` 0x29): layer = [5, 6][blend byte], pass = soft ? [0x23, 0x2a] :
  alt ? [0x25, 0x26] : [0x00, 0x28]. Most smoke and dust layers are layer 6 (LDR constants), the nitrous glows
  and sparks layer 5 (HDR). Like the coronas' layers 7 / 8, layer 6 is very likely drawn after the tonemap; its
  target was not traced.
- **Other renderer words**: half extents radius × aspect / radius / aspect (0x24 word 10, 0x27 word 11, 0x28 word
  0x14); a size cap radius ≤ cap × (view depth + view +0x218) × view +0x220 (0x24 word 9; 0x27 word 19 × the
  binding slot of byte +0x5b; 0x28 word 0x13); `FUN_00440100` writes TEXCOORD1 = (shadow-map u, v, translucency,
  key light as ambient) from words (15, 17) / (16, 18) / (0x19, 0x1b) and `FUN_0043fc30` the wrap (TEXCOORD3.w)
  from 16 / 17 / 0x1a. The view block (`FUN_0046adb0`: +0x218 = 0, +0x21c = 0, +0x220 = 1, +0x224 = 1;
  `FUN_004a1470` sets +0x21c = |Δcamera · forward| / dt each frame) is the block the render actions' first
  operand points at (+0x20 forward, +0x30 position, +0x60.. the globals). `FUN_0043fa40` sorts by depth from
  camera + +0x218 × forward and culls particles behind it.
- **`RenderSpriteVelocity`** (0x29, run `FUN_0045c2f0`): streak = velocity × 1/30 (`.bss` 0x010ed2a0) × word 18 ×
  slot[byte +0x4c], length clamped to words 12–13; head colour = the colour channel (alpha kept), tail × (1, 1, 1,
  word 16) (`.rdata` 0x00f9d1c0); in view mode +0x228 == 1 the colour is (0, 0, 0, word 17).
  `RenderSpriteMotionBlur` (0x25, `FUN_004598e0`) writes the colour's rgb with alpha 1.0 (`.rdata` 0x00f9d13c)
  and the renderer's word 11 at the streak's other end; its colour curves' alpha is unused.
- **`UpdateParametricRadius`** (0x3C, compile `0x0044bbb0`, run `FUN_004612d0`): the compile turns words 5–8
  (knee k, values a, b, c) into an offset and two slopes; r(x) = a + x(b − a)/k below k, b + (x − k)(c − b)/(1 − k)
  above, x = age × 1/lifetime; radius × r × (w9 + (1 − w9)·slot[byte +0x28]). (Read as a cubic Bézier before.)
- **`UpdateParametricFadeOverDistance`** (0x38, compile `0x0044ba90` → `FUN_004493f0`, run `FUN_00460970`):
  d = (position − (camera + (max(w19, view +0x218) + n)·forward)) · forward (− radius with byte +0x69),
  n = w20 + (w22 − w20)·min(view +0x21c / w23, 1) with byte +0x68 (else w20); colour × (B + clamp(−d/n)(A − B) +
  clamp(d/(w21 − w20))(C − B)) with A, B, C = words 0–3, 4–7, 8–11 (`.rdata` −1.0 at 0x00f8f340), then a
  luminance blend by view +0x224 (weights `.data` 0x01104150 = 0.299, 0.587, 0.114; 1.0 = none); bytes +0x6b /
  +0x6a: alpha / radius × clamp((|v|² − w25²) / (w24² − w25²)).
- **Wheel controllers and spawners**: `UpdateWheelBindings` (0x42, run `FUN_00463330`): sources 0 / 1 time,
  2 view +0x68, 3 |Velocity|, 4 camera distance, 5 view +0x60, 6 byte +0x7c, 7 / 8 sin / cos, 9 the scale, 10–13
  view +0x6c / byte +0x7d / +0x70 / byte +0x7e, 0x0e–0x11 the `WheelContact` parameter's +0x40..+0x4c, 0x12
  clamp(|WC +0x1c| / s), 0x13 view +0x78; no offsets; the same combine ops. `UpdateBindings_Wheel` (0x34, run
  `FUN_004601a0`): the 0x33 table plus 0x14–0x16 = W[1..3], 0x17 = clamp(|W[0]| / s), 0x18 = the byte at W +0x30
  for its `WheelParams` operand W. `EmitterWheel` (0x05, `FUN_004498e0` / `FUN_0044fb20`) and
  `EmitterWheelContactPlus` (0x07, `FUN_00449a90` / `FUN_00450760`): count = (rate·dt + per-metre·|speed|·dt) ×
  slot[byte +0x78 / +0x68], a LOD factor (`FUN_0043ecd0`, words 6–7 / 10–11) and a carry rounded to nearest.
  The per-wheel update (`FUN_00589e20`) fills the wheel effects' parameter blocks (0xf0 bytes at pool +0x500):
  the smoke's +0x80 = (`uStack_4c0`, spin, lateral, lock ramps); the kick-up's (`FUN_004acd80` at start,
  `FUN_0059bb50` per frame) = contact point, (lateral, ·, longitudinal ground speed, ·), contact normal + a clock,
  (0, the three ramps), the wheel's velocity + a front-axle flag, the car's matrix with the colour override in w.
  How the Bear runtime maps that block onto an effect's `WheelContact` was not found.
- `InitialisePosOffset` (0x14) keeps a particle in the emitter's space; `ParametricPosVelToWorldSpace` (0x1A /
  0x1B) places it with the current `Transform` when it is drawn, so the exhaust flames stay on the pipes.

## Car collision effects (2026-10-16, ported in `crash_vfx.rs`, `particle_mesh.rs`)

Read for the request "broken glass when crashing into traffic, crash particles in general".
- **Race cars** (`FUN_00584260`, per frame per race-car slot; `param_3/4` = the `VfxGlobals` handle): when the
  car's velocity (vehicle +0x14b0) is longer than `VehicleImpacts.34429d6c` (+0x130, 1.0), it walks the car's object
  contacts (`FUN_00904880` / `FUN_00904850`, 0x70-byte records: point +0x10, value +0x2c, normal +0x30, object
  shorts +0x60 / +0x62). The time since the last effect (+0x3f0, += dt) above `VehicleCollisionSettings.19458ef1`
  (+0x9c, 0.2 s) and a value above `e9ff2375` (+0x98, 5) pick `VehicleImpacts` (+0xd0) and force a new instance,
  anything else `VehicleScrapes` (+0xd8). The other object's type byte (table `DAT_010edb68` entry u16 +6) 7 (race
  car) uses `FUN_004a02b0`, 8 (traffic / parked) `FUN_004a0600`; both take the impact zone of the contact
  (`FUN_00929da0`) and whether this car is the local player (`FUN_006a10d0`). `VfxVehicleCollisions` field index
  (declaration order): player vs race car 2 + k, player vs traffic 10 + k, others vs race car 20 + k, vs traffic
  28 + k, with k = 0 front (zone 3), 1 front left (2), 2 front right (4), 3 left (9), 4 rear (7), 5 rear left (8),
  6 rear right (6), 7 right (5); zones 0 / 1 / 10 and fields 0, 1, 18, 19 play nothing. The data: impacts front
  `…117eca` (flash, sparks, dust, glass cards, debris meshes), front corners `…1402f0` (the same with painted
  debris), sides / rear `…14021c` (no glass); scrapes `…140279` / `…7bcd0` (continuous sparks, painted bits). The
  first contact with an effect wins.
- **World contacts** (when no vehicle contact played): the bitsets at world +0x5ddf8 then +0x5ddec (0x80-byte
  records at +0x5ddfc / +0x5ddf0: point, normal, a value in [7], the collision tag in [0xc]) → `FUN_0057b0a0`: the
  surface's `SurfaceVfx` (`FUN_00547170` / `FUN_0054aaa0`, wet block by entity +0x1c17), `FUN_0049fe80` maps the
  zone to an impact field pair (0 top, 1 bottom, 2 front left, 3 front, 4 front right, 5 right, 6 rear right, 7 rear,
  8 rear left, 9 left) and takes the named one for the local player, the unnamed one after it otherwise. Its contact
  frame (`param_9`) is not what the effect gets.
- **Instances** (4 slots: handles +0x370, frames since refreshed +0x390, instances +0x28c): each frame every live
  instance's `IsEmitting` bit (record byte +0x5e8 bit 1, inferred) is cleared and its `Transform` / velocity set to
  the car's; the chosen effect refreshes the slot holding it (bit set, parameters copied) unless a new impact is
  forced or the instance died, else it goes to an empty slot or the one refreshed longest ago (`FUN_0049f880`
  creates). +0x3f0 and the slot's counter reset whenever an effect plays.
- **Parameters** (`FUN_004a9f70` via vtable +0x4c): `Transform` = the car's matrix (`FUN_005423d0`: body matrix ×
  the centre-of-mass offset rows), `Velocity` = v + ω × (p − centre) at the contact, `CollisionMeshParams` = the
  entity's mesh handle (+0x184) and the contact point and normal in car space, `ImpactClosingVelocity` = the
  contact value (the effects' controllers divide it by 22.35 m/s), `MovementSinceLastFrame` (vtable +0x48),
  `PreCrashVelocity` (vtable +0x4c), `VehicleParams` (+0x3a0; vec4 1 read as the paint colour by `RenderMesh`).
- **Traffic** (`FUN_00600220` → `FUN_005f58b0` for the world): the same with the traffic picker always
  (`FUN_004a0600`), "player" meaning the other car is the local player's race car; the speed gate uses the traffic
  car's own velocity.
- **`EmitterCollisionMesh` 0x01** (compile `FUN_00449630`: data word i → compiled i + 4 for speed / angles, angles to
  turns with ranges × 4 (`.rdata` 0x00f9d1b0 = 1/360, 0x00f9c610 = 4); run `FUN_0044bf90`): only while
  `IsEmitting`; the layer clock window (data +0x18) or an unspent burst; `n = burst·slot + rate·dt·slot` (slot bytes
  +0x5c..+0x60: radius, rate, burst, speed, angle); per polygon of the car's `PolygonSoupList` (first three
  vertices) whose centroid is within radius (+0x48) of the contact, at most +0x4c along the normal and facing it
  (`(n̂·N + 1)/2 > +0x50`): `area · n · camera fade` particles (carry from 0.9999, rounded); position
  `v0 + a(v2 − v0) + b(1 − a)(v1 − v0)`; direction about the triangle normal (polar angle from the compiled angle
  words and `min(|φ|, ½ − |φ|)`, `.bss` sin/cos polynomials = cos 2πx); speed `rand(speed)·slot`. The third channel
  gets `CollisionMeshParams` +0x20 (a local light colour).
- **`Initialise3dRotation` 0x0B** (`FUN_00453070`), **`Update3dRotation` 0x2D** (`FUN_0045ddf0`),
  **`UpdatePosVelWithWorldCollision` 0x40** (`FUN_00462750`), **`InitialiseCollisionRadius` 0x11** (`FUN_0044a200`),
  **`Render3dSprite(SubUV)` 0x1E / 0x1F** (`FUN_00455670` / `FUN_00456110`), **`RenderMesh(NoPaintColour)` 0x22 /
  0x23** (`FUN_00457720`): layouts and maths in `mw_formats::bear` (`Op::Rotation3dInit`, `Op::Rotation3d`,
  `Op::WorldCollide`, `Op::CollisionRadiusInit`, the renderers) and `particle_mesh.rs`. The glass is the 3-D sprite:
  a card folded 22.5° along its local Z (8 vertices, `.rdata` 0x00f9d220 / 0x00f9d230), centre pushed `r` towards
  the camera, wrap ±2k on edges / spine. Debris models in `GLOBALEFFECTS.BNDL` hold 16 pieces each
  (`BLENDINDICES`, `gaTransforms[3·i]`); a particle draws piece `serial & 15` with `R(q)·r` (deferred `VfxMesh*`
  programs; `VfxMeshCarPaint`: albedo = tex.rgb + paint·(1 − tex.a)).
- **0x3D / 0x3E drag** (`FUN_004615f0`, `FUN_004618d0`): `v ← v·(1 − drag)^dt + Δ` (`powf`), Δ = gravity·dt +
  wind (`(1 − R)·c·W`, engine block +0x50) + turbulence·dt·(2r − 1); 0x3E bounces only when the bounce impulse beats
  Δ, its restitution / friction are drawn once per update, and its plane blends from a parameter plane (within 2 m
  of a point) to the particle's own beyond 3 m. Ported (`particles.rs` `retain` / `acceleration`); the wind term
  and 0x3D's table scales are left out (model decision).

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
   0 at |p₀| = 1 − 1/k to 1 at |p₀| = 1. **Writer found 2026-10-14**: the block is the post object's +0xc00
   (post object = renderer +0x9d80), written by the final-pass set-up `FUN_008e8c40` from the Vignette block
   at post +0x11d0: k = 1 + 500 × (0.5 + 0.5 × `Sharpness`)^16 (`pow` with the `.rdata` double 16 at
   0x01021988, × the float 500 at 0x01021980) = 1.82445 for 0.34, the live 1.8245; s is the final pass's
   contrast c (below, "Final pass"), which explains its drift with the camera heading. Ported with both.
   (Earlier note: no instruction in the renderer code (0x880000…0x900000 of the snapshot) stores to
   +0xa980 / +0xa990 with a literal displacement, because the writer addresses them as post +0xc00 / +0xc10.)
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
   the finest combined level. **Level sizes (2026-10-14)**: `FUN_008eaaf0` hands `FUN_008e6f40` the
   renderer's target list (R + 0xa8) entries 0x50, 0x51, 0x52 as L1…L3 and 0x4c…0x4f, 0x22 as the blur /
   combine temporaries; in snap1 (renderer at 0x2404ce00, found from its DOF block; 1920 × 1080) and in a
   probe dump at 1280 × 720 (renderer at 0x2464ce00 through the `.bss` pointer 0x01183168) those are the same
   fixed 320 × 176, 80 × 44 and 20 × 11 (temporaries the same sizes, pairwise), while the depth-of-field
   targets follow the screen (960 × 540 / 640 × 360). Ported as fixed sizes. The bloom is therefore 4–6×
   wider than the first port's 1/4, 1/8, 1/16 levels at 1080p. Remaining model decisions in the port (originally: the level sizes (the
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
   variants. Which variant, its constants, the depth of field and the brightness / contrast: "Final pass,
   depth of field and grade (2026-10-14)" below.
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

**TintBlend slots, live (2026-10-14).** `FUN_008c9e40` stores the seven cube data pointers at renderer
+0xac00…+0xac18 and the weights at +0xac1c (base) … +0xac34. In all 20 `ref/live1` dumps and in snap1: base =
the identity cube (weight 0), slots 1 and 2 = the two environment keyframes' cubes with weights (1 − t, t) of
the timeline blend (dumps: 0.924 / 0.076 sliding to 0.809 / 0.191 over the minute; snap1: the noon cube
`…4e220` in both, 0.352 / 0.648), slots 3 and 4 = the same pair at weight 0 (very likely the Alternate /
weather pair), slot 5 an unidentified cube and slot 6 the heat cube `…789d`, both 0 in free roam (the PostFX
states' `ColourCubeSettings`; dump 15, during a state change, gives slot 5 weight 0.56). Cubes identified by
matching the data pointer's bytes (header (16, data pointer), then the entries) against the extracted
resources. Ported: the bake blends the two keyframes' cubes (`tonemap::blend_colour_cubes`).

Consequence for the ColourCube layout: a straight weighted blend keeps the stored order, so the stored entries
are the texture's own (x = input red fastest, bytes B, G, R + alpha), not "blue fastest, stored R, G, B"
(inferred; the kernels themselves are missing from the dump; `mw_formats::ColourCube`).

### Final pass, depth of field and grade (2026-10-14)

From the dump, the full `.text` objdump and live reads (the 20 `ref/live1/ren` renderer dumps; snap1, where the
renderer is at 0x2404ce00). The post object is renderer +0x9d80 (`FUN_008eae00`'s `this`, from the call at
0x008cdaf3 in `FUN_008c9e40`).

- **Post flags** (post +0xd1c = renderer +0xaa9c; virtual setters 0x00890c90…0x00890d50): bit 1 depth of
  field, 2 bloom, 4 vignette (the post +0xc00 block), 8 tint (the colour cube), 0x10 fisheye warp, 0x20 the
  final pass's motion blur, 0x40 cube scale / offset. Live: 0x7f, all on, in every dump.
- **Variant choice** (`FUN_008e8c40`, assembly at 0x008e90b8, as the decompiler dropped it): index = (motion
  blur ? 4 × mode + 4 : 0) | (depth of field ? 2 : 0) | (tint ? 1 : 0), mode = the view's +0x290 (+0x1220 +
  0x290 of the post object, 2 live); with mode 2 the motion-blur bit is only set when the depth of field is
  off. Entries are 0xe8 apart from post +0x0: 0 `0x01016a08`, 1 `0x01016148`, 2 `0x01017aa0`, 3
  `0x01017050`, 4 / 5 `0x01019620` / `0x01018698` (camera motion blur), 6 / 7 the same with depth of field,
  8 / 9 `0x0101dff8` / `0x0101c4f0` (motion blur + mini depth of field), 10 / 11 their depth-of-field forms.
  **Free roam runs index 3, `0x01017050`: depth of field + tint, no motion blur.**
- **Constants** (post +0xae0, VS: `BlurMatrixXXX/YYY/WWW`, `HeatHazeOffsetScale`, `HeatHazeLayerTransform`,
  `ScreenSize`; post +0xb40, PS: `DofParamsA`, `DofParamsB`, `BloomColour`, `FisheyeWarpScalePowerOffset`,
  `MotionBlurAmounts`, `BlurMatrixZZZ`, `ColourCubeScalesOffsets`, `g_DOF_True_K4K5K6`, `HeatHazeCoeffs`,
  `ScreenSize`, `MiniDOFScales1/2`; the name tables at 0x010214b4 / 0x01021524). `MotionBlurAmounts` =
  (max − min, min, −(max − min), −min) of `PostFxKeyframe.General` `a548ceb6` / `26e9df6b` (0 / 0.053; min
  clamped to 1/1024; live (0.052, 0.00098, …)), unused by variant 3.
- **ColourCubeScalesOffsets** = (c × 15/16 three times, b + 0.5 − c / 2 + 1/32) (0x0100d714 = 0.9375,
  0x00fa9324 = 1/32) with b / c the brightness / contrast `FUN_008c9e40` passes: b = options brightness ×
  0.01 − 0.5 + PostFX state +0x1660 + renderer +0x2cd0 × (1 − t), c = options contrast × 0.01 + 0.5 + PostFX
  state +0x1664 + renderer +0x2cd4 × (1 − t), where t = (1 + normalize(s.x, 0.001, s.z) · renderer +0x1b70) / 2,
  s = renderer +0x2930 (the to-sun vector: same heading as the key light, another elevation) and +0x1b70 the
  camera's forward. Renderer +0x2cd0 / +0x2cd4 are the time-blended `EnvironmentKeyframe.Camera` `d1fd3a11` /
  `e49dc52c` (0.015 / 0.03 at noon; live 0.0139 / 0.0334). With the options at 50 and the PostFX terms 0 the
  live constants follow exactly: dump 0 (t = 0.466) c = 1.0178, dump 10 (t = 0.828) c = 1.0063, b ≈ 0.007 /
  0.002. So looking away from the sun adds up to 3 % contrast and 1.5 % brightness; the same c scales the
  vignette's A / D (the "s" above).
- **Depth of field** (`FUN_008e8760` → `FUN_008e7530`, `FUN_008e7900`; programs `0x01012858`, `0x010130f0`
  family): K = `g_DOF_True_K4K5K6` (post +0x10a0) from the camera's block (camera +0x130…+0x13c via renderer
  +0xae10…+0xae1c, when the camera's DOF mode +0x170 is 1 or 2; else from the view override +0x15c0, else
  defaults 10000 / 35): A = 73.448, B = 90 (focus distance), C = 102.83 (× 0.001 = the focal length f in
  metres), all three live in every read; their source (data or code) was not found. x0 = A / 2 × f / (B − f) =
  0.0420; K.x = −x0 B / proj.m32, K.y = x0 − …, so CoC = max(K.x d + K.y, 0) = max(x0 (1 − B / z), 0) in view
  depth z (live K = (12.602, −12.56): K.x + K.y = 0.042, zero at d = 0.99667 = 90 m with the projection's 0.3).
  Downsample (`FUN_008e7530`, target 0x1f, 960 × 540): 4 taps at ± half a source texel, weights |CoC| × 0.99 +
  0.01, alpha = their mean. Blur (`FUN_008e7900`, targets 0x20 / 0x21; mode 4 / 8 / 12 by the target's size
  against 640 × 360, mode 8 at 1080p): 4 taps along an offset, tap i weighted by saturate((alpha − i / 4) ×
  10), so under the free-roam CoC (< 0.25) only tap 0 counts. Final pass: lerp(frame, dof, min(15 |CoC|, 1)):
  nothing within 90 m, up to 63 % of the half-resolution copy at infinity. Ported in `post.wgsl` without the
  blur passes (their one surviving tap's sub-texel offset was not traced).
- **The composite's motion blur** (`GBufferComposite`, slots 0x8a `MotionBlurVertexConstant` / 0x8b
  `MotionBlurPixelConstant`, registered at 0x008c49a8): on when renderer +0xb230 == 2 and the view's +0x1500 is
  set (live: on). The four vectors are renderer +0xb1f0…+0xb220 (a linearised one-frame reprojection: vel =
  uv × P.z + P.xy with P = M0 u + M1 v + M2 d + M3) times s = General `26e9df6b` (0.053 live; the code divides a
  value by 2 or 1 and by 1, 3 or 16 depending on view flags, the live product matches 0.053). The shader blurs
  7 taps along vel / 42 per step, weighted by the light buffer's alpha. Measured size: (1 − d) × 0.068 × (0.5 −
  u) UV at 51 mph, i.e. ≈ 0.5 px at the screen edge 5 m away and ≤ 2 px at 200 km/h: not ported (invisible at
  this amount). The writer of +0xb1f0 was not found by displacement.
- **Distortion and splatter**: `FUN_008e1510` clears the distortion target and draws a render list into it
  (heat haze); `FUN_008e2cb0` draws the splatter overlay. Both are empty in clear free roam; not ported.

## Model LOD and draw distance (2026-10-10, ported in `crates/mostrusted/src/lod.rs`)

**The per-placement rule** (`FUN_008a02f0`, reached through the thunk `FUN_008ae140`; read in full). Arguments:
the `Model` header, the placement's 4×4 matrix, a shadow-pass flag, the camera position, the view's LOD scale
**squared** and the placement's own LOD scale. It:
1. takes the bounding sphere of the renderable that state 0 (LOD0) maps to (the `Renderable` header's first
   float4), transforms the centre by the placement matrix (`v · M`), and measures `d = max(0, |c − eye| − r)`;
   the radius is used as stored (not scaled by the matrix);
2. forms `v = d² / (s_inst² · S²)` (S² the view scale squared, `s_inst` the placement's scale);
3. in the shadow pass returns when `v > Model+0x0C` (the shadow distance, stored squared);
4. returns (draws nothing) when `v > lod[n − 1]`, `lod` = `Model+0x08` (squared distances), `n` = `Model+0x18`;
5. otherwise picks the first LOD with `v ≤ lod[i]` (an SSE compare of four entries and a fall-through switch
   on `n` = 4..1, so at most four LODs are ever chosen) and draws the renderable that state maps to
   (`Model+0x04` table into `Model+0x00`). The shadow pass draws state 6 (SHADOW) instead when the state count
   (`Model+0x16`) is at least 7 and state 6 is not 0xFF.

**Callers and the per-placement scale.** `FUN_00550020` (its main-view and shadow-pass calls; `FUN_005691b0`,
`FUN_0056ae50`, `FUN_00564d30` and others make the same call, not read) walks six index lists and passes, per kind: `InstanceList` entries
(model at +0x00, **scale at +0x04**, +0x08 / +0x0C the next two arguments, overrides at +0x10, matrix at +0x20);
the prop list's 0x60-byte records (object +0x40 → `PropObject` +0x04 = model, **scale at +0x4C**, overrides at
+0x50, matrix at +0x00); runtime world objects (compounds / dynamics: model at object +0x10, scale at +0x14,
matrix at +0x30). The object lists' float after the instance id is that scale (survey, `mwtool lods`: compounds
+0x50, dynamics +0x4C; 1.0 for most placements, 0.5–10 for a few hundred). `FUN_00538b00` (behind
`FUN_00539530`) is another list walker with the same call; it skips models whose byte `Model+0x19` has a bit of
the pass's mask set (the mask's meaning not traced).

**The view's LOD scale** (`FUN_008b5fe0`, per frame; stored squared at renderer +0x1ee8c, also uploaded as
shader constant 0x95 = (S², 1/S², 0, 0)): `S = max(L · 0.75 / tan(fov / 2), 1)`, `fov` = renderer +0x17050, the
camera's horizontal FOV in radians (the same value `FUN_005d7260` turns into the projection: tan of half,
`DAT_00f8a810` 0.5; `DAT_00fb69f4` 0.75; the clamp `DAT_00f8a820` 1.0). `L` comes from the "LodScaling"
display option (`FUN_00414a80` reads settings +0x124; written to `config.NFS13Save` by `FUN_00414e50`): 0 → 1,
1 → `DAT_00faecec` 1.5, 2 → width · height · `DAT_0100e908` (1/720) · `DAT_00fa2fbc` (1/1280), capped at 1.5.
The default (`FUN_00414b40`) is 0, or 1 when the machine has ≥ 3 processors, adapter class 6 and more than
0x30000000 bytes (768 MB, presumably video memory); this install's
config has `LodScaling=1`. At the chase camera's resting 70° horizontal FOV, S = 1.5 · 0.75 / tan 35° = 1.61.

**Vehicles** have their own rule (`FUN_008aa970`, 16 car slots): the distance from the camera to each car
(+0x30..0x38 of its render object) against four thresholds `(T − x) · S` with T = 15, 30, 60, 150 m
(`DAT_010c6f4c..58`, `.data`) and x a per-call offset; the LOD index is the number of thresholds passed (4 =
beyond 150 m), clamped to 2 for cars whose render object has bit 4 of +0x1a40 set, 0 for all while `FUN_006a0140` holds; a
debug override
(`DAT_0118315e` / `DAT_01183160`) forces one LOD. Not ported (our cars draw their LOD0).

**Port and model decisions** (`lod.rs`): the rule above per placement root, on `InstanceList` (non-backdrop),
prop, compound and dynamic placements, each LOD's own renderable decoded and spawned (hidden until chosen).
Model decisions: the eye for the shadow decision is the main camera (the shadow views' caller passes a view
position whose origin was not traced); the SHADOW state's renderable is not drawn (casters are the visible LOD's
meshes; 1835 of the 3668 world models have one); compounds that can break draw their LOD0 pieces at every LOD
(only the cull applies); backdrops keep the zone composition (no LOD rule); `--merge-static` merges LOD0 without
the rule; L = 1 (`MW_LOD_SCALING`), the render target's pixels for L = 2.

## Model pass mask (2026-10-16, applied to the main view in `loader.rs`)

`Model` +0x19 is a bit mask of the **views that skip the model**. Instances: `FUN_00553810` inserts each
`InstanceList` instance into the culling structure (a 0x30-byte record through `FUN_00541850`: bounding sphere
from the LOD0 renderable, flags `(Model+0x19) << 24 | 0x840`, `| 0x80` with the renderable's flag bit; the other
inserters `FUN_00553540`, `FUN_00553b10`, `FUN_00553cb0`, `FUN_00554230` put the byte in the same place). The
frame setup in `FUN_008cffa0` queues one query per view (`FUN_008dbd90(query, required flags, excluded flags,
…, frustum, LOD scale)`):

| view | query | excluded | bit |
|------|-------|----------|-----|
| main (camera frustum +0x16f90, LOD scale +0x1ee8c) | `DAT_01183254` | `0x4000000` | 0x04 |
| cube-map faces (six, `DAT_01183258..`) | | `0x2000000` | 0x02 |
| sun shadow cascades (three, `DAT_01183270..`) | | `0x1000000` | 0x01 |
| far shadow (`DAT_0118327c`) | | `0x3000000` | 0x01, 0x02 |
| view at +0x172e0, probably the planar reflection (roads are excluded) | `DAT_01183284` | `0x8000000` | 0x08 |
| a view at +0x17150 (gated by `FUN_00798840`) | `DAT_01183280` | `0x10000000` | 0x10 |
| another main-scale view (+0x1c9e0) | `DAT_01183288` | `0x4000000` | 0x04 |

The consumer of the queries is a job not in the dump. Runtime objects (compounds, dynamics) go through
`FUN_00538b00`, which skips a model when `Model+0x19 & mask != 0`; its callers pass 4 (main view, LOD scale
+0x1ee8c), 1 (shadow, state 6), 2, 8 and 0x10. Data: roads 0x09, lit signs 0x01, props / fences 0x08 / 0x1a,
230 coarse terrain tiles 0x04 (shadow, cube-map and reflection stand-ins: drawn in the main view they cut through
roads), 1007 instances and 1337 dynamics 0x1f. Port: placements of models with bit 4 are not spawned
(`mw_formats::Model::in_main_view`); bits 1, 2, 8 are not applied (model decision, research log 2026-10-16).

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

## World lights (2026-10-10)

Point, cone, spot and line lights are GeneSys `Light_Point` / `Light_Cone` / `Light_Spot` / `Light_Line`
objects (all extend `Light_Base`: `Colour`, `Intensity`, `AttenuationThreshold`, `FlashPattern`, five
`OnDuring…` bools). They reach the renderer in three steps.

- **Submission** (`FUN_0054ade0`, per active light record, 0xa0 bytes each at world +0x7b8a0): by the
  definition's type id (0x285b point, 0x2859 cone, 0x841d8 spot, 0x841ec line, 0x285c projected texture) it
  calls `FUN_008ce920` (point), `FUN_008ce9c0` (cone), `FUN_008cebf0` (spot) or `FUN_008cee40` (line) with the
  light's matrix translation (+0x30) as position, the matrix's z axis (+0x20) as direction, `Colour`, the
  record's current intensity (+0x14), the definition's `AttenuationThreshold` (+0x48) and, in degrees,
  `ConeAngle` (+0x60) and `ConeInnerAngle` (+0x70). Those store colour.rgb = Colour × intensity, colour.w =
  the threshold (below 0.001, `.rdata` 0x00f8bb34, a renderer default at +0xa40 / +0xa44 / +0xa48 instead),
  the direction normalised, and the angles in radians (× 0.0174533, 0x00ffc01c). `FUN_008dfa00` /
  `FUN_008dfbf0` / `FUN_008dfdf0` append to per-type lists with the squared distance to the camera; a full
  list is handed to `FUN_008df590` (not read; presumably it keeps the nearest; the list sizes are runtime values).
- **Constants** (the light pass `FUN_008b8840`): for a point light `FUN_008b0940` takes m = the largest of
  colour.rgb and writes `deferred_lightColour` = (rgb, threshold / m) and a volume matrix scaled by
  sqrt(m / threshold); the shader's `saturate(1 / d² − colour.w)` so reaches zero exactly at that radius. Cone
  (`FUN_008af930`) and spot lights (`FUN_008afee0`) do the same and add `deferred_lightDirection` = (−direction,
  s) and `deferred_lightPosition.w` = c from a table of 45 volumes 2° apart (`FUN_008ae860`, renderer +0xad0,
  step 0.0349066 at 0x0100e760; index = angle in degrees × 0.5, at most 44): c = cos(i × 2°); a cone light's s =
  1 / (1 − c), a spot light's s = 1 / (cos(ConeInnerAngle) − c), or FLT_MAX (0x0100ce20) when the inner cone is
  not inside the edge. A light whose volume the camera is outside of is drawn at once as a volume; the others
  are batched into full-screen passes (`FUN_008b0650`, `FUN_008af2f0`, `FUN_008af610`: the `Point2…4`
  programs). Slots: `deferred_lightPosition` 0x50, `deferred_lightColour` 0x51, `deferred_lightDirection` 0x52.
- **Shaders** (`LightBuffer_Point`, `_Cone`, `_Spot`; blend ONE, ONE): L = light − pixel, d² = |L|² + 0.001,
  light = colour.rgb × saturate(1 / d² − colour.w) × saturate((L̂ · direction.xyz − position.w) × direction.w)
  (the cone factor only for cone and spot lights), times the key light program's BRDF: Lambert albedo (1 − F0)
  N·L / π for the world and the Ashikhmin-Shirley diffuse for cars (RT3.w), plus the same normalised
  Blinn-Phong specular. No shadow and no AO term.

**Where lights come from.** `LightInstanceList` records (3,628: matrix, definition import at +0x40, an id at
+0x4c) and the `Lights` of objects' `WCVfxBehaviour`s (4,921 placements: street lamps, traffic lights): a
light definition plus a `LocatorGroup`, resolved in the object's `mLocators` (formats.md). The submission above
also divides the current intensity by the definition's `Intensity` and passes the ratio with the record's
+0x48 handle to `FUN_00497200` (the light's corona, when it has one).

**What switches a light by hour** (traced 2026-10-10 in the decrypted image; addresses as in the dump).
- *Environment state* (`FUN_008a8480`, state E = environment manager +0x1b8c0; t = time of day in seconds,
  `EnvironmentSettings` S): with A = S+0x124 and B = S+0x128 (`SunMoonPath.a5caac0f` = 8 and `644383ee` = 20,
  × 3600), **E+0x76 "day" = A < t < B**. The distance to a boundary is the smaller of frac((X − t) / 86400)
  and frac((t − X) / 86400), × 24 (`.rdata` 0x0100e75c); d = the smaller of the two boundaries' distances.
  **E+0x7c "night level" = 0 by day, else clamp((d − `LightTransitionOffset`) / max(`LightTransitionDuration`,
  1e-4), 0, 1)** (S+0x174, S+0x170), so it ramps up from 20:00 to 21:00 and down from 07:00 to 08:00. E+0x84
  is the same with the Shadow… pair; E+0x74 = night and level 1, E+0x75 = night and level < 1, E+0x79 = t <
  14400 or t > 79200 (04:00, 22:00). The function then runs `DayNightTransition`'s other two hours (S+0x178 =
  8.5, S+0x17c = 20) through the same arithmetic (what that stores was not followed). E+0x77 / E+0x78 (rain,
  wet) come from weather bits at manager +0x1b61c (`FUN_00893c80`).
- *Per light* (`FUN_0055ae50`, the world update, loops the 0x460 light records and calls `FUN_0054d980`):
  record +0x14 = base × `FUN_0089b400`(E, flags, id). The base is the definition's `Intensity`, or
  `FUN_00547470`(Intensity, FlashPattern) when `FlashPattern.Type` ≠ 0 (record flag 0x20). The flags are
  record +0x19, filled in `FUN_00548eb0` from the definition: 1 = `OnDuringNight`, 2 = `OnDuringTwilight`, 4 =
  `OnDuringDay`, 8 = `OnDuringRain`, 0x10 = `OnDuringWet`; plus 0x20 when the record's flag 0x10 is clear and
  4 when its flag 8 is set. `FUN_0089b400` returns 1 if (day and flag 4) or (rain and flag 8) or (wet and
  flag 0x10), and 1 if flags 1 and 4 are both set; otherwise x = the night level, or, without flag 0x20,
  clamp((level − r / 92) × 92, 0, 1) with r = (id × 119) mod 91 (id = record +0x1a); the result is 1 − x with
  flag 4, else x. The twilight bit is never tested there. Record flag 0x10 and the id are set only for
  lights that objects carry (`FUN_005328c0`: the id is the owner's 16-bit handle), so street lamps come on
  one after another through the transition and `LightInstanceList` lights fade with the level. A record
  whose intensity rises above 0 is activated (`FUN_0054be40`), one that falls to 0 is removed.
- *Flash patterns* (`FUN_00547470`): t = game clock + `Offset`, period = `Duration` + `Frequency`; phase
  p = t mod period − `Frequency`. Type 1: p < 0 full, else level = max(|p / Duration − 0.5| × 2, floor);
  type 2: p < 0 full, else a random level (LCG table at 0x0110bf80) of at least the floor; type 3: p < 0 the
  floor, else full; output = (I − floor) × level + floor with floor = `3ac4f0ce`.
- *`DynamicLighting_GlobalLightMapIntensities`* (slot 0x11) is written in `FUN_008c2080` through the second
  constant accessor `FUN_00a621a0(0x1191030, slot)`: (level, level or 1 while it rains, level, 0) with level =
  E+0x7c. The same code writes zero vectors to `DynamicLighting_LightMapChannels0123_Red/Green/Blue` (0x12 to
  0x14, the 4567 set is never written) and (1, 1, 1, 1) to `DynamicLighting_Illuminance` (0x10).
- *Light lists* (`FUN_008dfa00` and siblings): `FUN_008df590` is an introsort of the 0x40-byte entries
  (depth 2·log2(n) − 2, insertion sort below 29); when a list reaches its capacity it is sorted and its count
  cut to the list's keep size, so the nearest lights survive. Both sizes are set at run time (not read).
- *Cone table index*: `FUN_00ede840` is the CRT `ceil` (truncate, add 1 when positive with a fraction), so a
  cone uses the next volume up: index = ceil(angle / 2), at most 44.
- *Lightmap and AO map samplers*: `LightMapTextureSampler` and `AOMapTextureSampler` are the engine's global
  sampler ids 0x1a and 0x1b (`FUN_008c34b0`, registered through `FUN_00a5def0`; `puddleMapSampler` is 0xb). No
  code names those ids with a literal; the textures come from data: a `Model` resource's block at +0x10 (the
  layout of an `InstanceList` instance's overrides: names + texture and sampler-state imports) and the
  instances' own override blocks.
- *List sizes* (`FUN_008ae1d0`, per view, keep / capacity from `.data` pairs; view 0 = the main view): point
  (renderer +0x000, `0x010c7060`) 128 / 160, cone (+0x360, `0x010c6f88`) 224 / 256, projected texture (+0x1b0,
  `0x010c70a8`) 8 / 12, spot (+0x6c0, `0x010c6fd0`) 224 / 256, the 0x40-byte list at +0x510 (`0x010c70f0`) 32 / 40,
  the 0x60-byte list at +0x870 (`0x010c7018`) 128 / 160; the point / cone / spot lists of views 1, 2 and 3–5 keep 4 / 6, 128 / 160 and 24 / 32. Renderer
  +0x2165..+0x216e (per-type enables: +0x2167 cone, +0x2168 point, +0x2169 projected, +0x216a cop, +0x216b spot)
  are all set to 1 there; +0x217a + view is 1 for view 0 only.

## Car lights on the world (2026-10-16, ported in `car_lights.rs`)

Read for "vehicle lights don't light up the world". Data: `VehicleVfxBehaviour.Lights` (formats.md "Car light
sources"; `mwtool car-lights <car|all>`).

- **Who**: `FUN_00588da0` loops the 16 race-car slots (render instances 0x1c40 apart from manager +0x3050) and
  calls `FUN_005818f0` for each slot whose instance bytes +0x19/+0x18 (`+0x3069`/`+0x3068`) are set, with the
  slot's effect mask (`+0x1f818 + slot × 0x400` = block +0x3c8), light level L (+0x3cc), headlight level H
  (+0x3d4), flash clock (game time × +0x3e4) and **viewed** = the slot is the local player's (`FUN_006a10d0`) or,
  when `FUN_006a11e0`, the watched car. Traffic (`FUN_005fd190`) queues coronas only (`FUN_004970b0`); the six
  light-submission functions (`FUN_008ce920` point, `…9c0` cone, `…cebf0` spot, `…cee40` line, `…cf0c0` projected,
  `…cf1d0` cop) are called only from `FUN_0054ade0` (world), `FUN_005818f0` (cars), `FUN_004a2920` (effects) and
  `FUN_008cef70` (checked over the full listing), so traffic and parked cars cast no light.
- **Levels** (`FUN_0057a5e0` writes the slot block's +0x3c8; the levels are written next to it, `00570000.c` ~line 8600): +0x3cc = +0x3d4 = 1 on a
  running car (+0x3d4 = +0x3dc when that is > 0; bit 0x8000 replaces +0x3cc by a flicker noise). The mask: 0xd1
  (groups 0, 4, 6, 7) when entity +0x1a8c (set at spawn by `FUN_00591350`; request +0x194c at 0x587254: 2 on / 1 off,
  written only by cutscene cars `FUN_005b4750` from their data byte +0x5b; the script function
  `SetHeadlightsState` (0x6781f0) is a stub), | 8 with the cop bar, | 2 brake (+0x1930 > 0, held 0.5 s by
  +0x3e8), | 4 when `FUN_0090d460`(slot) = −1 (read as reverse; not followed) and | 6 then when +0x192c > 0; groups 0, 1, 2, 4, 6, 7 cleared when
  +0x1a84 = 0 and +0x1a40 bit 2 is clear. In `FUN_005818f0`, bit 7 (tail) is cleared when instance +0x1930 > 0
  (the brake; same 0x1c40 structure, inferred). Snapshot: slot 1 (a cop) mask 0xd1, slot 0 0x80000 (lights off;
  the player's car was not driving).
- **Per entry** (0x40 bytes: +0x10 `Colour`, +0x20 `Animation`, +0x28 `LightDefinition`, +0x30 `LocatorGroup`,
  +0x34 `a3de907e`, +0x38 `Group`, +0x3a `da6ac384`): skipped unless the group bit is in the mask, and group 6
  when viewed. Groups 4, 6, 17 are headlights (level H, flag `cVar15`); others level L × the animation
  (`FUN_0049ef00`). Not viewed: × `FUN_0089b400`(flags, 0) with flags 0x61 (gate 2: night level), 0x64 (gate 1:
  1 − night), 0x65 (gate 0: 1). Viewed: no gate (the player's head and tail lights are lit by day too); with
  instance +0x1c17 and group 4, H = 1. Colour = definition `Colour` × entry `Colour`; intensity = `Intensity` ×
  `a3de907e` × level, each locator's world matrix = the quaternion's rotation (`√2 q` expansion, rows as
  `FUN_005818f0` builds them) × the car's matrix. By definition type: spot 0x841d8 → `FUN_008cebf0` (renderer
  +0x216b; direction = row 2, `ConeAngle` +0x60, `ConeInnerAngle` +0x70, threshold +0x48); cone 0x2859 →
  `FUN_008ce9c0` (+0x2167; × the damage factor, skipped at ≤ 1.19e-7 `.rdata` 0x00fab7ac); point 0x285b →
  `FUN_008ce920` (+0x2168, × damage); custom 0x285a → `FUN_008cf1d0` (the cop path, research log
  2026-10-16); projected 0x285c **only when viewed** → `FUN_008cf0c0` (+0x2169, × damage).
- **Damage factor** `FUN_0049fda0`(local position, FL, FR, RL, RR): the zone levels +0x112c / +0x1134 / +0x1144 /
  +0x113c (capped 0.99 while not wrecked) × 100 + 0.5 rounded (x87, to even) as bytes (FL / FR stay 0 for the
  viewed car); a lamp with local z in (−0.6, 0.1) is never out; |x| < 0.1 → 0.5 per side lit; else the side's
  byte (x > 0 = left) against the global object's +0x94 (0.01 in snap1, × 100 + 0.5 truncated = 1): any damage
  turns cone, point and projected lamps off. Spot lights take no damage factor.
- **Projected light** (`FUN_005818f0` → `FUN_008cf0c0` → list +0x1b0 → `FUN_008b1860`): the projector frame is
  a look-at (`FUN_00a639c0`) from the locator's world position along its row 2 with row 1 up; half-angles
  atan(Rxx) × `FovScale.x` and atan(Ryy) × `FovScale.y` (Rxx / Ryy = the locator's local rotation diagonal; CRT
  `atanf` at 0x00edc7b2, confirmed by its constants π/2, −1/3, 1/5 in snap1; × 2 × 57.2958, then × 0.0174533 ×
  0.5); every car's projector locator is unrotated, so 45° × FovScale (the main beam: 55.35° × 38.25°). Entry
  (0xc0 bytes): +0x00 position, +0x10 colour × intensity and threshold, +0x20 flag byte, +0x30..+0x6f the frame,
  +0x70 `Attenuation`, +0x80 the half-angles, +0x90 `664ab465`, +0xa0 the material, +0xa4 0, +0xa5
  `FUN_00897bf0()` (renderer +0x1f004: the headlight shadow map was rendered this frame). `FUN_008b1860`: far = the
  larger root of a t d² + b t d + (c t − max rgb) = 0 (`FUN_008a0a20`; t = threshold), near 0.1 (`.rdata`
  0x00f9f660); projection rows (1/tan hx, 0, 0, 0), (0, 1/tan hy, 0, 0), (0, 0, (f + n)/(f − n), 1),
  (0, 0, −2 f n /(f − n), 0) (`.rdata` 0x00f9d488 = −2); constants: `deferred_lightPosition` (0x50) = position −
  camera, `deferred_attenuation` (0x53) = `Attenuation`, `deferred_lightColour` (0x51), and
  `deferred_lightProjectionMatrix` (0x5b) = the camera-relative view × projection. The volume (`FUN_008a1c10`):
  a 4 × 4 × 4 grid over clip x ∈ [`664ab465`.x, .z], y ∈ [.y, .w], z ∈ [−1, 1] (z fractions 0, 0.002, 0.01, 1),
  drawn through the inverse view-projection; a camera inside (its position + 0.2 × view direction projects within
  the volume) draws full screen instead. Material states: 0x19 / 0x1a (inside) and 0 / 0x1b (outside) by the
  entry's flag byte, **0x31 / 0x32 when +0xa5 is set** (the `HeadlightShadows` programs).
- **Headlight shadows**: `FUN_008cfe20` sets renderer +0x1f004 = 1 and renders the map (`FUN_008b7a20`;
  `SortShadowHeadlightWorld` / `SortShadowHeadlightCarsMisc`) when +0x1cc23 (the display option `HeadlightShadows`,
  `param_1[0x118]` of the config writer at 0x414f7f; 1 in snap1, `true` in this machine's `config.NFS13Save`),
  +0x1f005 (= `FUN_005837e0`: the viewed car has a lit projected light, no gate; 0 in both snapshots, whose
  player slot had its lights off), +0x1cc10/11 and +0xe1f9 hold. Programs (`mwtool shader-asm
  LightBuffer_ProjectedTexture`): slot 0x788 (volume) / 0x5c8 (full screen) = diffuse only × the headlight
  shadow (`shadowMapHeadlightSampler`, slot 15: one `sample_c_lz` at (c.x / w × 0.5 + 0.5, 1 − (c.y / w × 0.5 +
  0.5)) against c.z / w); 0x6a8 / 0x4e8 diffuse without shadow; 0x338 / 0x418 with the specular term, no shadow.
  The diffuse-only programs take no (1 − F0): Lambert albedo N·L / π, cars 0.387508 albedo (1 − (1 − N·V/2)⁵)
  (1 − (1 − N·L/2)⁵) N·L. All: lit where |c.xyz| < c.w, cookie = `ProjectiveTextureSampler` at
  c.xy / w × 0.5 + 0.5 (not flipped), attenuation saturate(1 / (a d² + b √(d² + 0.001) + c)) with d² = |L|².
- **The cars' data** (`mwtool car-lights all`, 104 bundles): group 4 = `Light_ProjectedTexture` `…253a9`
  (intensity 11.5, threshold 0.05, `FovScale` (1.23, 0.85), attenuation (0.001, 0.25, 10), window (−1, −1, 1,
  0.098): reaches 360 m), group 17 = `…253aa` (10, 0.3, (1.27, 1), (0, 0.2, 0), (−0.8, −1, 0.8, 0.65); its bit is
  never set by `FUN_0057a5e0`), group 6 = `Light_Spot` `…f0821` (3, threshold 0.333 → 3 m, 61.6° / 46.2°),
  group 7 = red spot `…1246fa` (0.55, 0.4; 1 m), group 1 = red spot `…1246f9` (1.25, 0.47), group 2 = cone
  `…253af` (1 × 0.5, 0.2, 65°); every projector's material is `…6ccb`, its cookie the 512² `BC1_UNORM` `…77f9`
  (two lobes and a sharp cut-off in the top half).

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
- **Which entry** (confirmed; per pair in "Car against car" below): `FUN_009575e0` (player–player, AI–AI) uses `PlayerRules` (0x24) for two players
  ("race car impact"), else `22061460` (0x10, "AI impact") or `RoadblockRules` (0x2c, "roadblock impact")
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

### Car against car (2026-10-09, ported in `crates/mostrusted/src/car_impacts.rs`)

Read from the dump and the snapshot disassembly; `.rdata` values with `exe-const`, `.bss` ones from `snap2`.

- **Pair dispatch** (`FUN_00960f20`, from `FUN_00961750`; `this` = the collision manager + 0x414, holding
  handles to `CollisionInfo`, `ImpactDamageGraphs`, `CollisionResponses_*`): by the categories of
  `FUN_00929350` (0 player, 1 AI, 2 crashing race car, 3 traffic, 4 other). Player–player and AI–AI:
  `FUN_009575e0` (a mask selects "race car impact" `PlayerRules` for 0–0, "AI impact" `22061460` /
  "roadblock impact" for 1–1); player–AI `FUN_009588c0`; player–traffic `FUN_00959be0`; AI–traffic
  `FUN_0095a680`; traffic–traffic `FUN_00958130`; crashing race cars `FUN_00959210` / `FUN_00959730` /
  `FUN_00957f90`; props `FUN_00941170` / `FUN_009415f0` / `FUN_00940d50`.
- **Rules per side** (`FUN_009a9fd0(rule, closing m/s, damage, third)`: damage > `1617a710`, third >
  `d6c0e908`, closing × 2.23694 > `2414aa68`; vehicle +0x1777 never crashes): player–traffic: the player's
  `TrafficRules` ("traffic impact"), or `5ccf9942` ("crashing traffic impact") when the traffic car's crashed
  flag +0x1774 is set; the traffic car's `PlayerRules`. AI–traffic: the AI's the same; the traffic car's
  "AI impact". Traffic–traffic: `TrafficRules` / `5ccf9942` both ways. Player–AI: the player's "AI impact",
  **cleared unless an event runs** (`FUN_006a0040`: event object +0x2ce0); the AI's `PlayerRules` ("player
  impact"). AI–AI: "AI impact" both ways. `FUN_006a3c40` (team / mode gates) zeroes damage in some modes.
- **Profiles** (`DamageBarProfiles`, vehicle +0x1178): the player `PlayerOffline` (`TrafficRules` 20, "AI
  impact" 33, `5ccf9942` 150 or 100 mph, `WorldRules` 30 or 70 mph); AI cars (driver kind +0x30e0 = 2) the
  profile `2ae17c10` (snapshot: an AI car's profile held its scales 1.125 / 1.65): `PlayerRules` 50, "AI impact"
  50, `TrafficRules` 25; traffic `Traffic`: `PlayerRules` 50, "AI impact" 50, `TrafficRules` 15 (`DamageOverride`
  0 with the flag set).
- **Impact zone** (`FUN_00929da0`): contact point in the car's physical box (definition +0x10 centre, +0x20 half
  extents), divided by the largest |component|; x > 0.5: front (z > 0.7) 2, rear (z < −0.7) 8, underside 1
  (y < −0.8 and |y| > 0.8 + 0.4(|x| − 0.5)), else 9; |x| ≤ 0.5: 3 / 7 / roof 0 (y > 0.8) / underside 1 / 10;
  x < −0.5: 4 / 6 / 1 / 5; a point at the centre is 10. Location scale (`FUN_0092ebf0`, the profile's
  `ImpactLocationDamageScale` +0x18…+0x30): 1 → +0x18, 2 / 4 → +0x1c, 3 → +0x20, 6 / 8 → +0x24, 7 → +0x28, 5 / 9 →
  +0x2c, 0 → +0x30, 10 → 1; performance mods 0x32…0x36 modify it for players (`FUN_00574870`, not ported).
- **Damage curve** (`FUN_009250d0`, `ImpactDamageGraphs` = `CollisionInfo.Damage_Graphs`): below the mid speed,
  `low_d + max(0, (v − low_v)/(mid_v − low_v))·(mid_d − low_d)`, above it on to the high point without a cap;
  speeds in mph (× 0.44704, `.bss` 0x0118a010). Graph `97aab742` 5 → 0, 80 → 33, 160 → 100; graph `3fd69cce` 0 → 6,
  60 → 32, 120 → 100; `674ea963` = 25 mph.
- **Generic damage** (`FUN_00941a50`; all pairs with a non-race car): closing = the relative velocity along the
  pair normal (its length when the normal is zero); base = graph 1 (0 below 0.25 m/s, `DAT_01024720`); × min(max
  speed / max(25 mph, 0.5), 1); × the location scale; × the strength factor (`FUN_0092ea20`): the other car's
  +0x1608 + its table[zone].x over (this car's +0x1604 if the other's `VehicleComponent` byte +0x8f
  (`05cd1d74`) is set, else the handler's scale) + this car's table[zone].y (table at vehicle +0x16a0, stride
  0x10). Scales: player–traffic player +0x15f4 / traffic +0x15fc; AI–traffic AI +0x15f4 / traffic +0x15f8;
  traffic–traffic +0x15f4 both. The undamaged-by-strength value goes to the third threshold.
- **Strength stats** (`FUN_008ffb20`): `Vehicle_Gameplay` (the `VehicleComponent.Gameplay`) `DamageStats`
  +0xc (`ac7a3b27`, driver kind 2 = AI), +0x28 (`8570ce40`, default), +0x44 (`304e4b58`, online,
  `FUN_006a0290`): u16 +0xc → 0x15f8, +0xe → 0x1600, +0x10 → 0x1604, +0x12 → 0x15fc, +0x14 → 0x15f4, +0x16 →
  0x15f0 (each ≥ 1), u8 `Strength` +0x18 → 0x1608; mods 8…0x13 adjust them for players. Snapshot check: the
  911 held 50, 50, 50, 50, 50, 100, 60 (default set); traffic cars 30–35 strength, defence 30, +0x1604 = 1.
- **Glancing cap** (`FUN_0093c0b0` → `FUN_0092e580` both ways, player / AI against traffic): motion frames (forward
  = velocity or the body's forward, `FUN_009ae1c0`), poses predicted one frame (`FUN_009e8310`); when
  |forward_B · right_A| ≤ 0.7 (`CollisionInfo` `226dddf1`) and the boxes' corners overlap by less than 0.5 m
  (`12236fc8`) across B's path, the race car's damage is capped at 4 (`.bss` 0x01185d60).
- **Race car against race car** (`FUN_0093c240`): w_A = |v_A|·(1 + c_A − c_B + e_A − e_B), c = 0.1 for zones
  2 / 3 / 4 (`DAT_01023b64`), e = small input-dependent extras (`FUN_0092e7b0`, 0.025…0.1 from vehicle
  +0x1bf0…+0x1c20, not ported); w = d_A w_A − d_B w_B; base = graph 2 of |w| (zero when ≤ 0.1 or |v_A − v_B| <
  0.25); shares p = w·cos(α/2) with α = acos(ŵ · d_A) and acos(−ŵ · d_B) (`FUN_00924bb0`: acos `FUN_00eddb3b`,
  then a sin polynomial of 2π(¼ − x/2π) = cos x); A's damage = min(w_B / 25 mph, 1) × A's location scale × B's
  share × base × A's strength factor (scales: player–AI player +0x15f8 / AI +0x15fc; AI–AI +0x15f8 both). The car
  that drives into the other takes little: a stationary car's weight is 0, so it deals nothing.
- **Landing on a car** (`FUN_008fbb90`, from `FUN_00901160`, tried A on B then B on A): neither crashed, B's
  +0x1630 ≥ 0.2 and A's ≤ 0 (inferred: time on the ground), the contact within 0.8 (`DAT_010cf570`) of A's half
  extents in x and z → A crashes (the matching rule's `DamageOverride` only; a missing rule still crashes).
- **Re-test gate**: `FUN_009575e0` computes damage only while vehicle +0x1640 + 4·other ≥ 0.5 (`FUN_009588c0`:
  ≠ 0); the array is reset to 0.5 (`FUN_009b3d70`); its writer was not found.
- After the rules: `FUN_009b2530` applies the damage to the damage bar (wrecked when used up); `FUN_009253a0`
  passes the crashes on.

Model decisions (`car_impacts.rs`): the per-location table at +0x16a0 is zero (it was zero on the player's car
in both snapshots); the `05cd1d74` flag is clear (it is on every car checked); `FUN_0092e7b0`'s extras are 0;
the glancing test uses the current poses; a pair is tested when its contact starts and then not for 0.5 s;
"grounded" = a wheel on the ground; no damage bar; a crashed AI car has no control and is reset after the
player's crash cameras' length (4.5 s) onto its route at its progress, rolling at the `GameLogicConfig` reset
speed; a crashed traffic car is marked wrecked (later hits on it use `5ccf9942`). Not ported: takedown
feedback, battling (`FUN_008f8ac0`, damage × profile +0x4c), the crashing-race-car pairs, props' "prop impact".

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
- **Compound smash** (`FUN_00555c90`, after physics): when a compound changes from pinned to loose its timer
  (manager +0x59e0, per compound) is set to `08c87eeb` (`AdditionalInfo` +0x34, seconds); every frame while it
  is loose the game first tests `timer <= 0` and then subtracts dt. With the timer run out, the body's
  world-contact force (`FUN_008f6470` record +0x2c) or any pair record's force (`FUN_00901b60` array, 0x70
  bytes, +0x3c) strictly above `ForceToSmash` (+0x30) smashes it: the compound object takes the body's
  transform and velocities, message 2 (enable compound) and 0x11 are posted and the body is removed
  (`FUN_0054e210`). `ForceToSmash` 0 (billboard / panel compounds, delay 0) smashes on the first contact
  after the knock.
- **Enable compound** (message 2 in `FUN_0055a680` → queue at streamer +0xc3d14 → `FUN_00558bb0`, at most 48
  part slots in all, far compounds evicted): `FUN_00554500` creates one 0x140-byte part object per part
  (`FUN_00535ff0` → `FUN_005341a0`: type 3, +0x90 part hash, +0x130 the compound, +0x134 = part +0x60 =
  **skeleton bone**, +0x135/6/7 = part +0x64/5/6, linear and angular velocity **zeroed**), placed by
  `FUN_00532b00` at bone bind matrix × compound world (`FUN_005321b0`: the bone's quaternion and model-space
  position, no parent chain). `FUN_005547f0` then registers every part whose +0x64 is 0xFF in the part slots
  0x5a0.. (`FUN_0054dfe0`: pinned from the part's own `AdditionalInfo` +0x28) and activates it
  (`FUN_0054e390`); the others are parked at (0, −100, 0) (`FUN_00532c30`). The compound's model is skinned to
  its skeleton (`CompoundObject` +0x08), one bone per part, so the parts draw the triangles of their bone.
  Joints (`CompoundObject` +0x2c / +0x3c: GeneSys handles naming two part hashes at +0x10 / +0x14 and a value
  at +0x18 → `FUN_0054e120`, message 0x2f) exist in the code but **no compound in the install has any**.
  `EnableCompoundBehaviour` +0x1a (`543527d0`) would split a compound at stream-in (`FUN_00554230`); false on
  all 52.
- **Part replacement** (message 3, `FUN_0055a680`): a part smashed through its `SmashProxyBehaviour`
  (`ForceToSmash` at behaviour +0x1c, read by the getter at 0x5b9800; the code that compares it was not
  found) is replaced by the chain first = part +0x66, next = +0x65 of each new part, until 0xFF; each new part
  is registered, activated and placed by `FUN_00536050`: old world × old bone⁻¹ × new bone, linear velocity =
  old velocity + ω × (new position − old body's centre of mass), same ω. Then the old part is parked and its
  body removed. Only the single-arm street lamps use it: the whole pole (120 kg, proxy 100000 N) splits into
  a lower (50 kg) and an upper (70 kg) half.
- **Links** (`FUN_0054e750`, every frame): a pinned part with links (part +0x68 pointer, +0x6c count, u8 part
  indices) is unpinned (`FUN_0054e570` + `FUN_0054e390`) once none of its linked parts is registered and
  pinned. Used by the 11-panel structures (20 kg panels pinned at 500 kg m/s, a 230 kg frame).
- **Smash effects / sounds**: none in the smash path itself. The effects come from the objects' behaviours
  ("World-component behaviours" below); the parts' `WCPlaySoundBehaviour`s are behaviour data.
- **Reset** (`FUN_0055ae50`, every frame, no timer): a moving prop (64-slot pool, `FUN_00553fe0`) is put back
  (`FUN_00553540`) or removed (`FUN_00553740`, `mbRemoveInsteadOfReset` or a game flag) once **both** the camera
  and the player's car are farther than `mfResetDistanceSquared`. ResetProps (message 0x15, `FUN_0055a680`)
  re-creates everything.
- **Sound**: a prop's contacts play its `PropObject` +0x1c `WCPlaySoundBehaviour` (`FUN_0054bf70` for pair
  contacts, `FUN_0054c680` for the world contact); compounds, dynamic objects and parts run the same behaviour
  through its runtime object. Settled 2026-10-16: "World-component sounds" below.

### World-component behaviours: what a contact shows (2026-10-16)
- **Dispatch**: `FUN_0053cba0` walks the world-component table (0xe4 entries at +0x20, 8 bytes: flag, object
  with its physics id at +4); for each object with a body it passes every body-pair record (`FUN_00901ab0`,
  0x70 bytes: +0x10 point, +0x2c, +0x30, +0x3c force; the other body = +0x60 + +0x62 − own id) to
  `FUN_005461c0` and the world-contact record (`FUN_008f6470`: +0x10 point, +0x20 normal, +0x1c, +0x2c) to
  `FUN_005462d0`. Both run the object's (up to 16) behaviours' vtable slot 8 / 9 with two out flags: "activate"
  → slot 1 unless already on, "deactivate" → slot 2 if on (on-mask at entry +0x42).
- **`WCVfxBehaviour`** (type 0x2ae0; handler vtable 0x00faaa58 whose slot 1 `FUN_005b3350` builds the runtime
  behaviour, vtable 0x00faf238): GeneSys +0x18 `0228a2f8` / +0x19 `713589ad` / +0x1a `543527d0` (the base
  behaviour's bools; copied to runtime +0x16 / +0x17), +0x1c `ImpactEffect` (+0x5e "pending"), +0x24
  `4524a2be` (+0x5f), +0x2c `c8d7741f` (> 0: a random flash timer, +0x4c / +0x50), +0x30 `29436f61`, +0x34
  `5af50d49` (a probability against a random number, +0x5c), +0x38 `17b6b189` (a distance, squared into
  +0x30..+0x3c), +0x40 / +0x44 / +0x48 the `Coronas` / `Lights` / `Spot_effects` arrays with counts at +0x54 /
  +0x56 / +0x58, +0x4c `19e7191a` / +0x50 `392e10d5` (hours: 24 / 0), the bools after +0x5a unread.
- **Slot 8 `FUN_005af3e0`** (pair contact): only when the object's body is not pinned (body +0xf4 bit 2): +0x16
  set → clear it, activate; +0x17 set → clear it, deactivate. If its effect is alive (+0x60) the effect record
  gets a new frame (`FUN_005a7750` from the contact point and the body's velocity, zero while asleep); else, with
  +0x5e set: clear +0x5e and, when the other body is a vehicle (`FUN_00573360`: an active entry of the 16-slot
  vehicle table), start `ImpactEffect` (`FUN_0049f750`) with velocity = the vehicle's body velocity
  (`FUN_004acd40`) and the frame of `FUN_005a7750`: position = point + (0, 0.5, 0) (`.rdata` 0x00fae8f0), Z =
  normalize(velocity) (fallback: the object's row 2), X = normalize((0, 1, 0) × Z) (fallback +X), Y = Z × X.
  `FUN_0054bf70` (plain props) builds the same frame from the hitter's velocity (vehicle +0x120), fallback
  identity, position + (0, 0.5, 0) (`.rdata` 0x00fab370).
- **Slot 9 `FUN_005af630`** (world contact): with `4524a2be` set, when the body's velocity at the contact point
  (`FUN_0059eb20`) is longer than `29436f61`, the effect starts at the point (or, while one is alive, gets the
  new frame): Z = the contact normal, X = normalize(+X × Z), Y = Z × X.
- **Slot 1 `FUN_005b30d0`** (on): when +0x5c (the probability roll) and neither the timer nor the flash mode:
  spot effects (`FUN_005af900`: one effect per locator of each `Spot_effects` group, at locator × the object's
  matrix, velocity = a vector from the object's vtable +8), lights `FUN_00532760`, coronas `FUN_005328c0`.
  **Slot 2 `FUN_005b3150`** (off): lights `FUN_005327b0`, coronas `FUN_00534080`, spot effects `FUN_005afb30`.
  Slot 3 `FUN_005b31a0` runs the time-of-day / flash switching every frame.
- Data: a street lamp switches its "SetLightsOn" behaviours (light, coronas) off and "Collision FX" /
  "StreetlampSingle_WC VFX" on at its first loose contact: sparks where the car hit and the `…7a95f` burst at its
  foot; its parts throw `…1621e7` sparks on their first vehicle contact and while sliding along the ground.
- Port (`prop_vfx.rs`): the rules above for compounds, their parts and dynamic objects; pair contacts are the
  race cars' (`props::PropCarContact`, from `prop_contacts`) and avian's with other props and traffic. Model
  decisions in the module docs and the research log (2026-10-16).

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
- Compounds (`smash_compounds`): the rules above. The smash force is the largest of the race car's contact
  force from `prop_contacts` and each avian contact pair's normal impulse / step; the loader splits the
  compound model by dominant bone into one piece per part (`Variant::bones`), and a standing compound draws
  the pieces of its first parts plus the triangles of bones no part owns. Model decisions: parts spawn awake
  (the game creates bodies asleep, next to the moving compound); no delay for `SmashProxyBehaviour` (its
  comparison was not found; read as a contact force like the compound's); a smashed compound and its parts go
  back together once neither its home nor any part is within the reset distance of the car and camera (the
  game resets each part slot on its own); bone-0 / unowned triangles vanish with the smash. The sounds moved to
  `world_sfx.rs` (2026-10-16, "World-component sounds").

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

### Boot, loading screens and the free-roam start (2026-10-09, `frontend.rs`, `pause.rs`, `mw_assets::frontend`)

Notation: **G** = `*0x01177868`, **GMM** = G+0x2f3fc (game-mode manager), **UIM** = G+0x2f480 (UI manager),
**LSM** = UIM+0x2d1b0 (loading-screen manager). Resident(p, r) = `FUN_0054b540` → `FUN_0054b340`: the `ZoneList`
zone holding p is loaded and so is every neighbour zone within r of p (`FUN_00561660`, circle against zone).
Code side from a sub-agent's trace of the dump and the live snapshot's disassembly; the flow is data
(`mwtool hsm`, formats.md "HSMData").

**Flow** (HSM "Top State" 21597 of `GAMELOGIC/GAMEMODES.BNDL`; it starts "Boot", "Run Always Active Systems",
"Enable Black Background", the boot page and "Switch to 60Hz Mode" together):
- "Run PageRunner2 Context - Boot Flow" (`CONTEXT_NAME` boot, `DEF_PAGE_REF` 491012): "Language Select" (a locale
  check; on PC it exits) → "Press Start": "Activate Press Start Widget Group" (`GROUP` 491032) starts "Exit When
  Intro Sequence Binding Is True" (`GeneralSequence.Triggers["BOOT"].IsTriggerSet[1]`) → "Press Start"
  (`ANYBUTTON` False). The group's `Background` widget plays sequence 1022357: jump 0 → 28.1 s unless
  `Language.IsJapanese`, intro movie 1689283 (28–34.4 s), music waves, the press-start layout 1274392 (34.38 s
  on: the prompt pair "PRESS ANY KEY" / "PRESS START" that hide each other on `Controls.IsXInputControllerConnected`
  through triggered idle steps, the autosave notice, the copyright), the looping title movie 1689287 (34.4 s on),
  the general trigger "BOOT" at 37.4 s; the music loops 100.88 → 73.952 s. `WAITFORANYCO` (enter 0x741690, update
  0x7416a0) polls every controller, arms on a press and finishes on the release; `BDELAY` takes its times from
  the node's `EnterTime` / `UpdateTime` / `LeaveTime` parameters (no code constant). `LOAD_ALL` (0x740f00) sets the
  gate `DAT_010f77ed` that world loading waits for.
- Leaving the page plays the group's `ScreenTransition` 1836019 (exit condition `Signals.True`), sequence 1836020
  (4.8 s): black quad, a fade from black (layout 1899282, 0–1 s), the car-picture sequence 1899171 conducted from
  0.25 s (two layers `Transitions.Image1` / `Image2` streamed from `LoadingTipsConfig.TransitionImages`,
  `Signals.GetIntValue[1]` picking; image 2 fades in at 9 s, the clock loops 20 → 0.1 s; holds at 8.75 → 8.5 /
  18.75 → 18.5 on `TransitionImages.ImageNReady`), the strip with the logo and "LOADING..." (layout 1899042, its
  scrolling text the tip `Transitions.Tip` when the item's `Text1` is empty), whooshes; **jump 4.25 → 4.05 while
  `Screen.IsActive(NOT)`**, so the transition covers the loading until the next screen is up, and 0.1 → 0.017
  while `Screen.TransitionsLoaded(NOT)`.
- "Post Controller Bind Boot": "Autoselect and Load World" (`LOADWORLD`, factory 0x77bc40: enter 0x74e040 only
  posts the load-world message with the world name from G+0x2f468), "Do Profile Load", controller watch → "InGame":
  "Wait For Required Resources" (feedback group 97039 loaded, vehicle list loaded, `IsProgressionReady` polled every
  0.25 s, `CalculateShouldShowRecapSequence`) → "Setup Start Point" ("HSM.Is Brand New Profile": YES → "Set Up New
  Game" = `SetNewGame`, `SetSinglePlayerRollout`; NO → "Set Up Continue Game" = `SetNewGame`,
  `SetRolloutFromProfile`) → TOGAME → "Should Play New Game Intro" (FALSE → "Playing", TRUE → "Set Intro Event
  Next") / TOMAP → "AutoLog"; "Disable Black Background" → "Playing".
- **World loaded** (`WorldLoaded` binding, `LAB_00712010` = GMM+0x2ce0 != 0 && GMM+0x2e77): GMM+0x2e77 is set
  in `FUN_007090f0` (GMM per frame, offline path while 0; cleared again if the local player has no car) when (a)
  the world is ready (`FUN_005470e0`: world+0xef858 == 12, the end of the load state machine `FUN_00556b60`, every
  queued request in `FUN_00531bc0`'s list at status 4, streaming mode world+0xef868 not 4 / 5), (b) Resident(camera,
  10 m) (`DAT_00fc9340` = 10; the active camera's position, medium confidence; online the car's), (c) traffic ready
  (event byte 0xf0 = 0, or `FUN_005f2800`: traffic off or the hull at p and its lane neighbours loaded with the
  models resident, or density world+0xa2b74 = 0) or 1.5 s passed (`DAT_00fc5aac`, scaled frame time).
- **`ShowLoadingScreen`** binding (`LAB_00719bd0` → `FUN_006f49c0`): GMM+0x2e81 (Lua `SetAllowLoadingScreen`) and
  not GMM+0x2e7f (`HasTransitionedEvents`) and (no event, or the online timer not synced, or the world not loaded,
  or a request bit in GMM+0x2ea8: bit 0 = a respawn into a non-resident target while the mode timer runs, bit 1 =
  `FUN_0074d040` with the black background until the world is ready and 15 frames passed). The HSM's "Manage
  Loading Screen" toggles `HSB_TOG_LS` (vtable 0xfd96f8, `SHOW` / `LAYOUT` = 108177) on `Mode.ShowLoadingScreen`;
  "Loading Screen Now Showing" starts "Mute All Sound" / "Mute All Rumble" (`MUTE_SOUND`, reason `LoadingScreen`,
  table 0x010c5c28) and disables car control and Easydrive.
- **The presenter** (`FUN_007a45f0`): a request mask LSM+0x54, one bit per reason (2 Lua `ShowLoadingScreen`, 3
  `BPLAYIT`, 5 `HSB_TOG_LS`, 6, 8, 12 respawn helpers, 13 `UI\GROUPS` loads, 15 car select); the lowest pending
  reason's layout shows in layer 0x17. Layout lists from `UIConfig` (UIM+0xb8): +0x80 `eee932fa` general, +0x74
  `a65d5d07` reason 5, +0x84 `8c082a97` reason 8, shuffled once with a PCG-style LCG (multiplier
  0x5851f42d4c957f2d, clock seed), at most 4 each; both general lists hold only **108177** (black, a 64 px
  spinner at (640, 618) whose idle behaviour turns it 360° per 1000 ms). When requests end it hides the layout and
  waits the layout's own out duration (layout+0x18) before unloading. No minimum display time, no fade constant.
- **Streaming stall** (`FUN_006a2200` from `FUN_004766a0`): pauses the simulation (pause reason 3) while the world
  is not ready or the car's surroundings are not Resident(car, 25 m) (`DAT_00faba94`; `FUN_00587120` per tick into
  +0x1c33; free camera: the camera, `DAT_00fa8844` 25 m). Not ported (we stream everything).
- **Start position** (`FUN_006d0180`, the respawn executor; offline, `IsFreedriveEvent` GMM+0x2e90): if not a new
  game (`IsNewGame` GMM+0x2e87) and the event's kind == `GameLogicConfig` +0xd8 (390145, free drive): the profile's
  last swap spot (stats 0x14 / 0x13 / 0xe) if its car is the current car, else `FUN_006c1840(car, 1)` then `(car,
  0)`: the nearest swap spot holding this car (discovered first) from the car's position; else, and on a new
  game, `FUN_006c19f0`: **`GameLogicConfig` +0x12c (`beab0bc2`) = start trigger 1156487** (1718.8, 31.7, 2220.9,
  the Freedrive event 390703 "FAIRHAVEN" start). Swap spots are `CarSwapSpot`s (GAMEPLAY.BNDL, 183): `Placement`
  = jack-spot box, `VehicleId`, `0e3bcabe` = the car's box trigger, whose direction is its local Z
  (`FUN_0042f250`). Placement (`FUN_006cd550` → `FUN_006ca580`): the trigger's pose, zero velocity, no road search
  or drop offset on this route (the crash reset's road search `FUN_0064b290` is separate). Safehouses are not used
  (`SetSafehouseFromProfile` is an empty stub on PC, `SETSPAWNPOS` exits at once).
- **Waiting for streaming** (`FUN_00647e00`): radius max(200 m, mph × 0.44704 × 3 s); a target not resident holds
  the car, moves the streaming focus there (message 4) and sets the loading-screen bit 0; `FUN_0064b290` re-checks
  every frame and places the car once resident (message 5 clears the focus).
- **New game** (Lua `SetNewGame(true)` → `FUN_0068fd50`): resets the profile tables, the starting car from
  `GameLogicConfig` +0xa4 = `DefaultRollout` 523000 (`Rollout.Vehicle` 122672, Aston Martin V12 Vantage),
  IsNewGame, and the first unlocked event of kind 390145 as the pending event. `ShouldPlayNewGameIntro`
  (`FUN_00675210`): profile stat 0xb < 1 and `FUN_006e60a0` fails (the intro event 1560354 "WELCOME TO FAIRHAVEN",
  `GameLogicConfig.4b9afa9f`, not played here).
- Camera at spawn: no snap found (`FUN_006d0180`, `FUN_0064b290`, `FUN_006bedb0`, `FUN_0063dad0`, `FUN_00845f50`
  do not touch the camera director).
- **Pause menu** (screen 431082): `Menu` widget, layout 431094 (element stack over `Widget.Menu.Items`, selected
  row by `Widget.Menu.SelectedIndex(EQUALS:#)`, rows a nested stack over `{parent}`); items `StdData` (431091):
  RESUME "CONTINUE", MWL "DRIVER DETAILS", MAP, MPLAYER, FRIENDS, TO_SETTINGS "SETTINGS", STORE; in an event `Data`
  (431113) adds RETRY, RETRY_LAST, LEAVE_EVENT; actions previous / next, `ExitGroupVia` the item's action, RESUME
  on cancel.

**Port** (`frontend.rs`, `pause.rs`; model decisions marked in the code): plain runs go title → transition →
playing (new game or `--profile` continue), interactive `--drive` runs show 108177 until ready; "ready" = car placed,
Resident(car, 10 m), then the 1.5 s timeout (traffic readiness not checked); cars are placed only once
Resident(target, 200 m) (`drive::spawn_car`); the loader streams nearest zone first and reports each unit. Ours:
the title's backdrop and movie drawn full screen (movies through ffmpeg; no VP6 decoder), the sequences' waves
played by `music.rs` (2026-10-15, "Music" below), four random pictures and eight random offline tips per
run, the `ImageNReady` holds never wait, the strip's tip not scrolled, `UIRoadRibbonShader` elements left out,
our profile file (car, position) saved every 10 s; the pause menu pauses the virtual clock, MAP lists the offline
races (the map screen 899991 is not built), SETTINGS opens our volume panel, the online items are inert, pad A / B
for accept / back.

### Music (2026-10-15, `music.rs`, `mw_assets::music`)

From a sub-agent's trace of the dump (high confidence unless marked). mgr = the `UISongManager` at
UIM+0x30370 (UIM = G+0x2f480).
- **Manager states** (`FUN_007a4010`, state at mgr+0x408): 2 loads `UI\SONGS\SONGS.BNDL`; 3 resolves list
  0x1c70a (the Autolog player's list); 6 resolves the requested list (+0x400) into +0x410; 7 picks the start
  index and builds the play order (`FUN_00798980`); 8 plays (`FUN_007a1750` every frame); a different request
  goes to 5: the same list as before clears the pause, a new one switches at once (`FUN_0079d0d0`) unless
  "allow playout" (+5), when state 9 lets the song end first (`FUN_00aba790`); 10-12 unload.
- **Start index** (state 7): the session table at mgr+0x08 (`FUN_00792900`: list -> next index + 1); else, for a
  list with +0x23 and a FeedbackMessage, the profile's "SongLists" table (`SongListID` -> `SongIndex`,
  `FUN_0072efb0`); else random. The RNG at +0x3a0: `s = s * 0x5851f42d4c957f2d + 1`, output the high word before
  the step, seeded from a timer (`FUN_00415fc0`); floats come through an 8-deep delay line at +0x380.
- **Play order** (`FUN_00798980(shuffle)`): bytes at +0x48, 0..n-1, Fisher-Yates when shuffle (profile option
  5, applied by `FUN_0073daf0`; its default not found). The next song (`FUN_0079d1e0`): index + 1, wrapping
  re-builds the order; offline, `FUN_00798a90` tests the candidate: rejected if among the last 4 played (ring
  +0xc8); a song with |`bc7081c6`| <= 1.19e-7 is accepted; else with r uniform, in free drive (GMM+0x2e90,
  `IsFreedriveEvent`) a song with f > 0 is rejected when r <= f, in events a song with f < 0 when r <= -f; a
  reject swaps the candidate with order[idx + rnd % (n - idx)], after n - idx tries the last is taken. So a pass
  still plays every song once; the bias orders it (electronic first in free roam, rock first in events).
- **Playback**: songs stream on the mixer channel "Music_UI" (`FUN_00aba490`); no crossfade (the next song starts
  the frame after one ends; a skip or a list switch stops the stream); while the game clock is paused no new song
  starts unless a `SetSongListSequenceItem` allowed it (mgr+7). Leaving a list stores (cur + 1) % n in the session
  table and the profile (`FUN_00729330`).
- **MUSIC_ADVANCE** (action 0x59): M on the keyboard (`FUN_00483470`), pad slot 13 (`FUN_00482250`; RB by its
  neighbours, medium confidence); a tap (released within 1 s) skips (`FUN_007a15c0` -> `FUN_0079f760`), only on
  lists with +0x22.
- **`PLAYSONGLIST`** (factory 0x77c610, enter 0x757e90, update 0x744b50, leave 0x744b00): enter calls
  `FUN_0079f7b0(list, ALLOWPLAYOUT)`; the list (`FUN_0074f540`) is SONGLIST, else with USEDEFAULT the event's
  list (event +0x68, when the world is in state 4) else the current GameMode's `MusicPlaylist`. List 0 pauses
  (moves the request to +0x404; "Pause Music", "Clear Credits Songlist"). STOPONLEAVE False: the node exits at
  once and the music goes on; True: leaving pauses. The HSM: "Freedrive Update" -> "Play Freedrive Song List"
  (USEDEFAULT, ALLOWPLAYOUT, STOPONLEAVE); "Show World Offline Event" -> after the intros "Is Intro Race" ->
  "Play Intro Race Music" (2026345) or "Play Default Game Mode Music"; RETRY -> "Pause Music"; leaving the
  in-game menu -> "Resume Music"; the in-game menu ("In Game Menu" 469008) starts "Mute World And Music"
  (sequence 1870238: bus 259952 and the "Music UI" snapshot property 1 -> 0 within 0.28 s, held while open).
- **`MUTE_SOUND`** (factory 0x77c0f0, enter 0x73e220; reasons table 0x010c5c28, counters `FUN_00790920`): a 0.1 s
  fade (0x01104740), 0.1 s hold after release (0x01104630), applied by `FUN_00abb520` only to buses with
  `BusRoute` 6 ("Master World"). Music (route 4 -> Final -> Master) is not muted: a song keeps playing on the
  loading screens. Bus volumes (`FUN_004b8b90`): Music = MusicVolume * 0.1, Speech = SpeechVolume * 0.1, World /
  UI / Movie = SfxVolume * 0.1.
- **`DynamicMusicManager`** (B+0x47544, `FUN_00831d80`): only sequence-based songs (none in data) and the
  `DynamicMusic.MusicState` string set by `MusicStateSequenceItem` (start 0x833900, end 0x830940); no code
  changes the music for pursuits, cooldowns or races (searched).
- **Title**: not the song manager: the title sequence 1022357's own waves (intro 1772189 28-73.385 s gain 0.7,
  bridge 1870153 73-73.9 s, loop 1772190 from 73.385 s, a looping stream, while the clock jumps 100.88 ->
  73.952 s); the boot transition 1836020 plays a whoosh (0.1-3.989 s) and a 2.5 s looping bed 14e291
  (0.75-4.8 s, so it covers the hold while the world loads).
- **Port** (model decisions in `music.rs` / `music/songs.rs`): no shuffle, the event's own list not read, the
  intro race recognised by its mode (1560350), the in-game menu's fade back is the same 0.28 s ramp, the title's
  intro starts ~0.8 s into the stream (decoding) instead of the clock waiting, the 0.1 s mute ramps not ported,
  the "now playing" popup not shown.
- **Not found**: the reader of the pause flag A+0x2c3b8, whether a stopped stream ramps, profile option 5's
  default, the caller that starts the manager at boot (`FUN_00790770`), the unmute after a preloaded list, what
  event +0x68 is.

### Menu input and the map screen's controls (2026-10-14, `input.rs`, `pause.rs`, `map_screen.rs`)

- **Menu actions**: a screen widget's `UserActionInstance` names a `UserActionDefinition` whose `InputMapping`
  string (`menu_up`, `accept`, `cancel`, `start`, `tab_left`, `trigger_left`, …) is turned into an action id by
  `FUN_007e7910` (table at `.rdata` 0x00fe7260; the Autolog browser's copy is `FUN_007dbc30` / 0x00fe58c0):
  `menu_left` / `right` / `up` / `down` 42–45, their `_alt` twins 46–49, `scroll_up` / `down` 50–51, `accept` 52,
  `cancel` 53, `settings` 54, `alt` 55, `tab_left` / `right` 56–57, `trigger_left` / `right` 58–59,
  `view_profile` 60, `start` 61. Default bindings (keyboard `FUN_00483470`, pad `FUN_00482250`): 42–45 arrows /
  left stick X, Y (max ∓1 / ±1); 46–49 none / d-pad; 50–51 Num 8 / 2, right stick Y; 52 Enter / A; 53 Backspace /
  B; 54 Right Ctrl / Y; 55 E / X; 56–57 Page Up / Down, LB / RB; 58–59 Z / A, LT / RT; 60 Tab / Back; 61 Esc /
  Start.
- **The `_alt` twin**: reading 42–45 goes through `FUN_0047a3f0`, which returns the twin (46–49) unless the main
  action's state equals `TABLE[main × 4 + alt]` (`.rdata` 0x00f9fab0: states idle 0, held 1, pressed 2, released
  3; held wins over pressed / released, pressed over released). So every `menu_*` mapping reads the arrows, the
  left stick and the d-pad.
- **Firing** (`FUN_007f50e0`, per user action each frame): the definition's `Analogue` flag (+0x34) calls the
  handler every frame with the value; else its repeat flag (+0x35, field `2da735e3`) fires on pressed or on the
  auto-repeat tick, and for the direction actions 0x2a–0x31 only when the value is > 0.05; else it fires on
  pressed. Auto-repeat (`FUN_0047c360`, per action): while held a timer runs; the first tick at 0.2 s
  (`.rdata` 0x00f9fb7c), then (flag +0x67c set) every 0.1 s (0x00f9fb78); released resets it. **Measured on the
  original** (2026-10-14, keyboard Down held in the pause menu 0.45 / 0.55 / 1.0 / 1.25 s → 3 / 3 / 6 / 7 rows):
  every 0.2 s throughout; the port uses the measurement (why the 0.1 s step does not show is open).
- **Pause menu** (431082, `Menu`): `menu_up` / `menu_down` (repeat) → `OnPrevious` / `OnNext`; `accept` → the
  item's action; `cancel` and `start` → `RESUME`.
- **Map screen** (899991): group 1222343's `MapManager` widget (native script `LiveMapManager`) takes
  `MainMapNav_Up` / `Right` / `Left` / `Down` (`menu_*`, **analogue**, only while `Widget.MapControls.IsActive`)
  as `OnMenuUp` / … (`FUN_00803ce0` stores the four values at +0x269c / +0x2698 / +0x2694 / +0x26a0); group
  899993: `accept` → `TO_OPTIONS`, `cancel` (`Back_MapScreen`) → `TO_GAME`; the tabs group 1607001: `tab_left` /
  `right` → `Tabs::OnTabLeft` / `Right`, `start` (`Drive`) → `TO_GAME`; filters 1135638 / 1135667: `trigger_left`
  / `right` → `MapManager::OnPrevFilter` / `OnNextFilter`; options 1222205: `menu_up` / `down` (repeat) →
  `OnPrevious` / `OnNext`, `accept` → `InvokeOption`, `cancel` → `Back` (`BACK` → the map group again).
  `TO_GAME` ends "Show Main Map" and the in-game menu: back to driving (HSM 469008 / 1695290).
- **The map controls' update** (`FUN_008170d0`, every frame while active; config `C` = `MainMapConfig` 1135320 in
  `UI/UICONFIG.BNDL`, reached through a GeneSys handle at +0x5c of the UI config): input `(x, y)` = (right − left,
  up − down). If `x² + y² > C+0x60²` (0.4): the first such frame stores the cursor (+0x2630) as the start
  (+0x2660); the hold time (+0x26b0) accumulates; once it exceeds `C+0x78` (0.1 s) the cursor moves by
  `C+0x74` (1500) × dt along the camera's rows (`y` up the screen, `x` across), is clamped to the box `C+0x20`
  (−3500, −4319) … `C+0x10` (2500, 5040), the selection is cleared (`+0x2610 = 0`, `FUN_00813a20`) and +0x26d6
  (moved by hand) is set. When the input drops back into the dead zone after being out: if the hold was ≤
  `C+0x6c` (0.4 s) and the last length > `C+0x70` (0.1) (and the flick flag +0x26d7, set at construction), a
  **flick** (`FUN_00814f60`): from the start point's screen position, items other than the selected one, on screen
  or up to `C+0x90` widths / `C+0x8c + C+0x94` heights off it, whose screen direction (x scaled by the view's
  aspect) is within `C+0x64` (30°) of the input direction, else within `C+0x68` (45°); the one nearest the start in
  the world is selected. A failed flick to the left (normalised x < −`DAT_0117f578`, .bss) exits `TO_MWL` (the Most
  Wanted list panel); otherwise, and for longer holds, a **snap** (`FUN_00813e60`): the item nearest the cursor on
  screen within `C+0x58` (0.4 screen heights). A successful snap or flick clears +0x26d6 and the glide clock.
  Projection (`FUN_00803fc0`): `(ndc + 1) / 2` in x and y (y up).
- **The glide** (`FUN_00807a60`): while +0x26d6 is clear and an item is selected, the glide clock (+0x26b4) runs
  and each frame `cursor += E(t / C+0x5c) × (item − cursor)` (`C+0x5c` = 0.8 s), then the cursor sits on the item.
  `E` (`FUN_00630980`): symmetric about (0.5, 0.5) (`E(x) = 1 − E(1 − x)` below 0.5), `1.25 (x − 0.2) + 0.125` up to
  0.8, then `0.875 + 0.625 (6.25 (1 − x) + 1)(x − 0.8)`.
- **Camera**: `Widget.MapManager.SelectedPosition` (the map camera's `LookAt` binding) is the cursor itself
  (getter at 0x807e94). +0x2650 is the cursor clamped to `C+0x40` … `C+0x30` (set on activation, `FUN_008078a0`).
  The view's +0x2f5a8 gets `C+0x84` (0.2) on activation / moves / snaps and `C+0xb0` (0.5) after a flick; its
  reader was not found. On activation with nothing selected, the local player's item is selected and the cursor
  put on it (0x817046).
- **Zoom**: `MainMapZoom` (Lua, compiled 5.1 chunk in the screen bundle) holds three states (distance 8000 / 5500 /
  3400, angle 30 / 38 / 46, FOV 50) but its PC list is the first and third; `ToggleZoomState` has no caller in the
  screen data or the exe (it is the Vita's touch zoom), so on PC the map stays at state 1. `LiveMapManager.
  ZoomOverride` switches the camera to the code's `ZoomDistance` / `ZoomAngle` (+0x26b8 = 8000, +0x26bc = 30)
  while +0x26d9 is set; any navigation clears it. No action scrolls or zooms the map by the right stick.

**Port** (`map_screen.rs`, `pause.rs`, `input.rs` `MenuInput`): all of the above except: the map is drawn flat (the
cursor moves at 1500 m/s along the screen's axes, no foreshortening of the up axis; the camera rows at the view's
+0x2f370 were not read), `FUN_0078f1d0` (a gate on moving) taken as true, the failed left flick snaps instead of
opening the Most Wanted list (not built), the tabs are inert, accept opens options only on events. The filters
work since 2026-10-15 (next section).

### The world map's items and filters; the minimap's free-roam icons (2026-10-15, `map_screen.rs`, `hud.rs`)

**Configuration** (`FUN_0080ec90`, the `LiveMapManager` reading its screen script by field name): the screen
script `LiveMapManager_ScreenScript` 1135185 holds `MapItemTypes` (16 `UI.MapItemType` {`DataSource` = the
option menu's name, `650f3bfb` icon key, `c3f39ca3` card key, `ItemType`, `OptionsHeight`, two flags}), filed by
`ItemType` at +0x2298 + 4 × type; `CollectablesName` (item type → language id) at +0x22e4; `SelectionPOI` (+0x2608),
`AIPlayerPOI` (+0x2330), `AICopPOI` (+0x2334), `CheckpointPOI` (+0x2338), `FinishLinePOI` (+0x233c),
`BlackListDriver` (+0x2340), `MeetupPOI` / `MeetupAreaPOI` (+0x2344 / +0x2348), `DefaultFilterIndex`,
`FilterListBinding`. Field names were confirmed by hashing them (`field_hash`). Keys resolve to the screen POI
widget's `PointOfInterestVisuals` (`mwtool ui map-items` lists all of it).

| type | source | icon key → texture | card key |
|---|---|---|---|
| 0 | LocalPlayer | 1135501 → arrow | 1013506 |
| 1 | Event | 1135497 → green diamond `0f789a` | 1013511 |
| 3 | SpeedCamera ("SPEED CAMERA") | 1135495 → `1d0861` | 1135735 (speedwall card) |
| 4 | Jump ("BILLBOARD") | 1135493 → `1d0817` | 1135737 |
| 5 | SecurityGate | 1135497 | 1135353 |
| 7 / 14 | Jackspot ("JACK SPOT") | 1135494 (icon visible on `JackSpots.IsSwapSpotFound[#]`) / 955695 (always) → wheel `160c27` | 1524715 |
| 9 | Event (Most Wanted) | 955947 → red diamond `0e962f` | 1013511 |
| 13 | BodyShop ("BODYSHOP") | 1524672 → wrench `160bfc` | 1135353 |
| 15–18 | JackspotDlc1 | 2113015, 2241160–2 (the packs' wheels) | 1524715 |

**Filters** (`FUN_00818550` next filter: index +0x26c4 counts 1…n and wraps to 1, then `FUN_008181b0` rebuilds the
list, `FUN_00813e60` snaps from the cursor, the index is saved as `MapFilterIndex`). The offline map's filter
widget (group 1135638, `Widget_MapFilters` 1135664) lists RACES (type 1), JACK SPOTS (7), RECOMMENDS (9), SPEED
CAMERAS (3), BILLBOARDS (4); `DefaultFilterIndex` = 1. `FUN_008181b0` by filter type: 1 → events
(`FUN_008154f0`); 3 → `TriggerData` kinds '4', 'G' as item 3; 4 → '2', 'A', 'D', 'J', 'M' as item 4; 5 → '0' as
item 5; 6 → 0x17 as item 6; 7 → '<' as item 7 and as item 14, 'B' / 'E' / 'K' / 'N' as 15–18; 9 → events with
other rules (Autolog recommendations); 0xc → body shops (kind 6) as item 13; 0xb → `FUN_00808d30`.
- **Collectables** (`FUN_00817500(kind, item type)`): every trigger of the kind whose id is in the world database's
  `Collect3` table (`FUN_00728b20`, column `TriggerID`; body shops skip the test); jack spots skip spots of a pack
  (`FUN_00643c10`), need `FUN_006fd0a0`, and go to item 14 when `FUN_006fcf90` holds (a profile list looked up by
  the spot's +0x24), else to item 7.
- **Events** (`FUN_008154f0`, test `FUN_00808e60`): every event of the world's list
  (`world+0x2f3fc → +4 → +0x11c`) that is an offline event, has its start trigger, is not of the excluded game
  mode (`GameLogicConfig` +0xd8), is not a locked pack's; with the RACES arguments, events the driven car may not
  enter (`FUN_006eca10` on `AllowedVehicleList`) and Most Wanted races not yet available (`FUN_006f9640`) are
  "locked" and hidden (`MainMapConfig.d9572ca3`, +0xb4, = false would show them). The item type is
  `MapItemTypes[1]`, or **`MapItemTypes[9]` when the event's +0xe9 (`c8f26580`, first place only: the Most Wanted
  races) is set** (and the event's type is 390131, `OfflineEvent`).
- **Dynamic items** (`FUN_00808290`): racers (`AIPlayerPOI`; `BlackListDriver` for a racer whose +0x578 → +0xb4 flag
  is set) while an event runs, cops (`AICopPOI`) while an event runs or in a pursuit (`FUN_006c2410`), the race's
  checkpoints and finish, meet-ups (online). The racer / cop look 1135614 has both arrows and switches on
  `Players.PlayerBase[#].IsCop` (racer `160b30` orange; cop `160b32` blue and `160b33` red).

**The free-roam HUD's collectable icons** (`FUN_006ffd90`, called per collectable from the game-mode update at
0x007030f6): a point of interest is created within 400 m of the player (`.rdata` 0x00fcb044) and removed beyond
450 m (`.data` 0x010c5944, initial value); the look by kind (`FUN_006ffba0`): speed cameras → `CollectablesConfig`
+0x54 (`ced801cd` 1013522, a world look without a map icon), shown when collected and in free roam; billboards →
`MapPoiConfig` +0xc (`a32aa8bd` 1013491, world look only); body shops → `CollectablesConfig` +0x10 (`d71dba55`
1444895, the wrench), always; jack spots → by pack +0x38…+0x44, else +0x48 (`507e9d87` 1013508, the wheel, visible
on `JackSpots.IsSwapSpotFound[#]`) or, when `FUN_006fcf90` holds, +0x34 (`a1b2d86a` 955694); a jack spot whose car
is locked for the profile (`FUN_006ed400` → `FUN_0068f190`) is shown only while `FUN_006fcf90` does not hold and its
car can be offered. The local player's look is `MapPoiConfig` +0x54 (`29401278`, 1013507) and another player's
+0x60 (`86c1695b`, 1013487) (`FUN_006440a0`: compares the record's player index with
`world+0xa6b4`); the pursuit's search area is +0x5c (`52b0c553`, 1013492, the 128² circle).
- **Dead end**: the HUD screens hold event looks with world layouts (1013512 green, 955952 / 955955 red,
  1013520, 1132924) but no code or GeneSys data names those keys (searched the image for the immediates and
  every GenesysObject dump); events are not drawn on the minimap here.

**Port**: the world map lists one filter at a time with the data's icons and cards (events keep the card measured
on the 2026-10-10 capture, look 1135809), Most Wanted races in red, cops of a pursuit with the cop arrow; the HUD
draws body shops and found jack spots within 400 m (450 m hysteresis) and cops with their own arrow. Model
decisions: `Collect3` taken as every trigger of the kind (74 speed cameras, 216 billboards with the packs' kinds);
`FUN_006fcf90` never holds (jack spots are item 7: on the map only found spots are listed, as their icon shows only
then); the packs' jack spots use the base look; RECOMMENDS lists nothing; the filter index is not saved;
collectables have no options; on the HUD every jack spot counts as unlocked.

### Navigation: SET DESTINATION and the GPS route (2026-10-15, `gps.rs`, `map_screen.rs`, `hud.rs`)

Notation: UIM = G+0x2f480 (`UIMapManager`, 0xd550 bytes); RC = UIM+0x1f8f0, the route container, three route
objects of 0x4450 bytes: RC+0 (the waypoint route), RC+0x4450 (a second route, the race's GPS line over the next
gates, `FUN_007a0e40`), RC+0x88a0 (the **shown** route, the one followed and drawn). Route fields: +0x0000 up to
512 line points of 0x20 bytes, +0x4000 point count, +0x4008 the path request's done flag, +0x4018 the path
(nodes as `i16` at +2.., count +0xf0, length +0x2dc), +0x4300 / +0x4400 a list of chained paths and its count
(race segments), +0x4404 start, +0x4410 velocity, +0x441c destination, +0x4428..+0x4434 the line's bounding box,
+0x4438 the check timer, +0x443c progress (u), +0x4440 remaining distance, +0x4444 state (1 pending, 2 built),
+0x4449 no re-routing, +0x444a no progress tracking.

- **Setting** (`Map:OnSetWaypoint`; the action's name hash at `.bss` 0x0117f49c, matched in the map's action
  handler at 0x008188e0 → `FUN_0080eb60`): the selected item (`LiveMapManager` +0x2610) of type 1, 9 or 10
  (events) → `FUN_00796ff0(RC, G, event id)`: the destination is the event's start trigger position
  (`FUN_006e56f0` → `FUN_0053c020`), UIM+0x2c8e4 = the event id; any other item → its position
  (`FUN_007ed6e0` on the POI list at UIM+0x14c50, else `FUN_008091d0`: collectables (3–7, 13–18) the trigger
  position, the player (0) its car) → `FUN_007970e0`; the second route is cleared first (`FUN_00792390`),
  UIM+0x2c5e0 = −1, `LiveMapManager` +0x26db / +0x26d8 = 1. The menu item has no exit token: the options stay
  open. `SetWaypointToEventFromBinding` (Lua, `LAB_006767d0` → `FUN_00674c60`) does the same for an event id;
  `SetWaypointToJackspot`, `SetRouteFromPlayerToEvent` (`FUN_006802d0`), `SetRouteFromPlayerToSafehouse`,
  `ClearTargetRoute` are Lua hooks (not read). Every collectable's option menu (`SpeedCamera`, `Jump`,
  `SecurityGate`, `BodyShop`, `Jackspot`, `JackspotDlc1`, `JackspotLocked`) starts with SET DESTINATION.
- **Request** (`FUN_00796df0(route, G, start, velocity, destination, 1)`): the start is the local player's
  position (`FUN_006e02f0`); when the velocity is not zero (any component beyond ±1.19e-7) it is moved onto the
  road graph: the nearest node (`FUN_004375b0`), then `FUN_00436c00` twice along the velocity (two nodes
  ahead); then `FUN_00843240(start, destination, 1, 1, route+0x4008)` queues a path request of type 9 (0x28-byte
  message, `FUN_0084b640`) for the AI's A* worker. The two option bytes are the A* route options: in
  `FUN_00527300` they switch the × 5 (`.rdata` 0x00fa97dc) shortcut and × 100 (0x00fa300c) jump factors, which
  multiply the **turn penalty only** (step = exit cost × 1/511 (0x00fa9ea4) + penalty × factors). The penalty
  `FUN_0052e250(a, b)` is 5 (0x00f9c588) · max(0, 0.98 (0x00fa9eac) − cos(b − a))³ with angles in degrees ×
  π/180 (the polynomial with `.rdata` 0x01075470.. is a cosine: 1 at 0, −1 at π); a = the angle (×2°) of the
  exit at the next junction leading back to the current one, b = the exit's angle; 0xff means none (no penalty).
  What the exit angle measures (heading out, or in) was not resolved, so the penalty's effect on straight vs
  U-turn is open.
- **Line** (`FUN_007a0f20` state 1: when the path is in, `FUN_0079e4f0`; a failed build asks again):
  `FUN_0079b890` walks the path's spline points (`FUN_00520a50`, per node pair), keeping every
  ((count >> 9) + 1)th so at most 512, plus the last node; each point: position, (0, 0, 1), the width = node
  +0x25 × 0.25 (`.rdata` 0x00f9c584; leading nodes under 0.001 m skipped, a zero width takes the previous),
  interpolated between the nodes, u = distance along × 1 / path length. `FUN_0079e4f0` then (single path or
  chained race segments) recomputes u over the kept points, replaces the side with normalise(−dz, 0, dx) of
  p[i+1] − p[i−1] (the last point its own segment), pushes points out by 10 m (0x00fa929c) sideways where the line
  doubles back on itself (p[i] = p[i−2]), appends the destination when it differs from the last point, and
  stores the bounding box.
- **Following** (`FUN_007a0f20` state 2, every frame from the container update `FUN_007a2aa0`, which first
  copies the car's position into +0x4404 and its velocity into +0x4410 of all three routes, the velocity zeroed
  when it points backwards along the shown line): the timer (+0x4438, 0.5 s `.rdata` 0x00f8a810) runs down with
  the game clock; at 0: reload, `FUN_005232f0` = the nearest point on the path to the car in 3D (over every
  spline piece) and the distance along it; progress = along / length, remaining (+0x4440) = length − along
  (the `UIMapManager.RouteDistance` binding, `LAB_007a7350`, reads the shown route's); unless +0x4449, with a
  single path, a car 15 m or more from that point (squared ≥ 225, `.rdata` 0x00fd6b48) asks for the route again
  (`FUN_00796df0` from the car, with its velocity). Then (unless +0x444a) the progress is refined each frame: the
  line point nearest the car with |u − progress| < 50 m (0x00fa9ed8) / length, heights × 1.25 (0x00fe0f90), then
  the car projected (`FUN_009f7ef0`, ground plane) onto the nearer neighbouring segment, u interpolated.
- **Shown route** (`FUN_007a2aa0`): when no route was pending at the start of the frame's update but one was
  and is now built, the shown route (RC+0x88a0) becomes a copy of the waypoint route (`FUN_007aa050`), with the
  second route's paths chained on when both are built (`FUN_0079ef00`, joining paths whose ends meet within
  1 m); RC+0xccd8 = −1, flags +0xcce9 / +0xd542 = 1.
- **Arrival** (`FUN_007a2aa0`): with the shown route built, a single path (count +0xcca0 = 0), RC+0xd543 clear
  and `FUN_006e0430` false (game-mode manager +0x2e91), a car within 5 m on the ground plane of the shown route's
  destination (squared < `.data` 0x010c61c8 = 25, the initial value) → `FUN_00673770`: every route cleared. The
  same function is called from the event code (0x0075e100, 0x00761d70, 0x00781b70, 0x00787c20). Starting an
  event near its start (`FUN_0074ad50`) clears UIM+0x2c8e4 when it is the waypoint's event.
- **Drawing: not found.** The shown route's points are read by code outside the decompiled set (no reference to
  RC+0x88a0 / +0xc8a0 / +0x443c besides the ones above). Not the `UIRoadRibbonShader` (the front end's light
  streaks, `frontend.rs`), not a `PointOfInterestVisuals` look, not `TextStyle` data (`FUN_0079a2f0`, which reads
  colours, is a text style reader). `Widget_PointOfInterest` +80 (`062ec799`) is `MarginProportion` (by its
  name hash at `.bss` 0x0117f9a8, `FUN_0080b570`), not a line colour.

**Port**: `gps.rs` as above; the path is `AiData::path` (A* on link lengths, no turn penalty) through
`ai::spline_through`'s samples; the map screen draws the route over the city image and the minimap into its
pixels. Model decisions: the line's look (light blue, the road's drivable width, at least 2 px on the minimap and
3 px on the map, only from the car's progress on); a failed search clears the destination; an event start
clears the route when a race starts (`RaceState` appears); the 5 m arrival test runs whenever a destination is set.

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
`IsClamped` is code not traced); the player's field is +0x54 (`FUN_006440a0`, 2026-10-15), the racers' was not
traced (`5bfb3553` holds the matching arrow; cops are drawn with it and `IsCop` true since 2026-10-15). Still ours: the map scale (4 m per reference pixel from the
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
- **Speed Points**: found 2026-10-10, see "Speed Points" below. The events the handler sends are scoring
  events naming `ScoringConfig` actions (the config at global +0x44 is the `ScoringConfig`).

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

### Compound models: standing, found, broken (2026-10-15)

- The standing compound object (type 2, created by `FUN_005396d0` at stream-in or `FUN_00533980`) keeps
  `CompoundObject` +0x24 at +0x100 and +0x04 at +0x104 and draws its +0x10 (draw passes `FUN_00538b00`,
  `FUN_00550020` pass the object's +0x10 to `FUN_008ae140`). +0x10 = +0x28 when the flag +0x10f is set, else
  +0x24; an empty slot falls back to +0x04. `FUN_00533a00` repeats the choice (with +0x10d set: +0x04) for every
  compound when `DAT_0110bfb0` is raised (by 0x546ff0, a callback registered at init through `FUN_00adb5e0`).
  `FUN_00532430` (the break) sets +0x10d and +0x10 = +0x104 (+0x04, skinned to the parts).
- +0x10f is the last argument of `FUN_00554230`: whether the instance id (record +0x4c) is in the sorted list at
  progress +0x22c8 (`FUN_006a0400`), which a function near 0x700080 (start not resolved; guarded by `FUN_00532dc0`) fills from the profile's collected
  triggers (`FUN_00728b20`: table `Collect3`, column "TriggerID") of billboard kinds; kinds 7 / 'I' go to
  +0x1194 (`FUN_006a03f0`, the list checked first: found there, the object is not created).
- Per-object material overrides: the draw copies the object's override block (+0x20 of the runtime object,
  from the instance record) into the constant and sampler slots (`FUN_00a621a0`, `FUN_00a62820`) before the
  model draw. Billboards bind `DiffuseSampler` this way (formats: `ObjectInstance::overrides`).
- No `AdditionalInfo`: `FUN_0054dfe0` clears the object's pinned bit when the physical definition's info
  pointer (+0x30) is null; the break test `FUN_0054e570` and the damage test `FUN_0054bf70` only read the info
  when it is there (no damage threshold then).

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
  that is only the "found" bookkeeping. What a pass does is the `DOBODYSHOP` behaviour's handler: "Body shops"
  below (2026-10-10).
- **Hints**: `FUN_006ef070(4)` (speed camera), `FUN_006f3f30(5)` (billboard), `FUN_006f3f30(7)` (gate) are
  show-once tutorial hints; their texts were not resolved.
- **Speed camera record** (`FUN_008749e0(id, player, centi_mph, 0, id)`): stores the values and, when online
  (`FUN_007e9360`), builds a leaderboard request with `"eventId"` = the camera id and the board name
  `"RoadRules"` (or `"Survival"` in another mode): the Autolog **speed camera leaderboard**. Offline it only
  calls `FUN_00874830(id)` (*inferred*: local best) and a virtual callback. No points are given here.
- **Speed Points**: found 2026-10-10 (next section). `BillboardWorth`, `ItemWorth`, `HighValueWorth` and
  `Score` are variables of `HSMBehaviourHSMDoSmashAndGrabScoring` (the Smash-and-Grab game mode), not free roam.
- **Does a billboard's volume alone count?** Yes (2026-10-10): `FUN_006e42f0` passes the billboard kinds
  ('2' 'A' 'D' 'J' 'M') like cameras and gates (offline always; online only with the free-roam flag +0x2e90),
  so entering the box queues the billboard and `FUN_007059b0` counts it. The prop's break (message 0x14) calls
  the handler as well, without the queue. Whichever comes first is the first find; the other only sends the
  0-point `HitJump` action.
- **The linked trigger of a billboard prop** is in the data: a `CompoundInstanceList` record's word at +0x4c is
  the id of the billboard's `TriggerData` record, at the same position, on all 216 billboards (`mwtool freeroam
  --objects`). Security gates have no such link (their fence props are plain props 1–2 m from the trigger) and
  are counted by the volume only.

### Speed Points (2026-10-10)

Speed Points (SP) are data plus one short code path. Confirmed by reading the code unless marked *inferred*.

- **Data.** `ScoringConfig` (`GLOBALCONFIG.BNDL`, UniqueId 390964, the object at global +0x44) has ~190
  UniqueId fields, one per thing code can score; code names an action by the field's offset. Each id is a
  `ScoringAction` (72 bytes; all 223 sit in one list object of `GAMELOGIC/GAMEPLAY.BNDL`, UniqueId 390077):
  `Predicate` (+0xc, a binding that must hold), id (+0x14), `GameplayTrigger` (+0x18), `Sequence` (+0x1c) and
  `Title` (+0x20) of its HUD message, a heat amount (+0x24, `1a88cb4c`), a per-player-count bonus array (+0x2c,
  count +0x3c), **`Score`** (+0x30, count +0x40 `ScoreCount`) and **the Speed Points array** (+0x34, field
  `8c1feb6d`, count +0x42 `SPCount`), `Scope` (+0x38), `Queue`, `Priority`. That +0x34 is SP is settled by
  three things: its count field's name, the titles' text ("Billboard +{sp}" followed by the SP glyph), and the
  code below. `mwtool scoring` prints the table.
- **A scoring event** (`FUN_006552a0` builds one entry, `FUN_0066c1b0` an empty event of 8): 9 words per
  entry: action id, a float (default: a sentinel, `DAT_00f8f340`), then −1s; word 2 overrides the score and
  word 3 the SP when they are not −1. Every collectable event leaves them at −1.
- **Sending** (`FUN_006a8050(action id, …, player, …)`): dropped offline unless the game mode's byte +0x2e74
  is set (the "timer running" flag of `StartGameModeTimer`; *inferred*: set in free roam); then `FUN_006a75f0`
  → the player's `FUN_006d08a0`.
- **Awarding** (`FUN_006d08a0(player, event)`), per entry: find the action by id in the player's action list
  (+0x3ac / +0x3b0); its `Predicate` must hold (`FUN_0063e470`); count it in the player's per-action counters.
  Offline (the mode's online byte `FUN_006dfed0` clear): score = `Score[0]`, or `Score[level − 1]` with the
  pursuit level `FUN_0066a7b0` when 0 < level ≤ `ScoreCount`; **SP = the entry's word 3, or `SP[0]` when it is
  −1; when SP > 0, `FUN_00735ca0(SP, the player's car id)`**. (Online the score comes from `FUN_006a4820` /
  `FUN_006a48c0`: indexed by finishing position or streak, plus the player-count bonus, with server patches
  `FUN_006a3530`.) Then the heat amount goes to `FUN_006bdac0`, the score is doubled while a double-score
  timer runs on `Scope` 1 actions, and is added to the player's score (+0x418) and stats.
- **The total** (`FUN_00735ca0` → `FUN_00734f60(SP, bucket, car)`): the bucket is 3, 1, 5 (free roam) or 0 by
  game-state flags and only feeds statistics (`SPBucket` rows, telemetry `GAME EVNT SPBK` with `epts` / `spbk`
  / `tpts`). `FUN_00734400(SP)` stores **total + SP** as the new total (`FUN_00726bb0`; the current total at
  +0x1c, the next Most Wanted event's requirement at +0x20, whose crossing in free roam sends a message). No
  multiplier offline. `ScoringViewModel.CurrentSpeedPoints` (view model +0x8c, `FUN_006a4960`) is the HUD's
  binding.
- **The collectables' actions** (offsets in `FUN_007059b0`, `FUN_006ed0a0`, `FUN_006ed330`; names cracked,
  all sort into their fields' places):

  | Offset | Field | When | SP |
  |--------|-------|------|----|
  | +0x294 | `SpeedCameraTriggered` | every counted camera pass (≥ 50 mph) | 0 |
  | +0x28c | `SpeedCamDiscovered` | a camera's first counted pass | 500 |
  | +0x260 | `SecurityGateHit` | a gate's first hit, in free roam (mode +0x2e90) | 0 |
  | +0x288 | `SmashGateDiscovered` | with it | 250 |
  | +0xfc | `HitJump` | every billboard hit, offline | 0 |
  | +0x1c | `BillboardFound` | a billboard's first hit | 0 |
  | +0x18 | `BillboardDiscovered` | with it | 500 |
  | +0x3c | `CarSwapFound` | a jack spot location's first find | 0 |
  | +0x38 | `CarSwapDiscovered` | a jack spot car's first find | 350 |

  Other offline actions with SP in the data (their triggering code was not traced): takedowns 115–155 by
  kind (head-on 115, slam / shunt 125, T-bone 135, plain 150, traffic 155, "Splashdown" 155), `CrashEscape` 150, shunt / slam / ram 15,
  "First Place" 50, an unnamed "COLLECTABLE" 55. Event results (place, medals) are not `ScoringAction`s with
  offline SP: the 900 / 650 / … position tables are `Scope` 1 (online) actions. Where an offline event's SP
  comes from ("End of event awards", `EventAward`, `ProgressionAwardPoints` strings) was not traced.
- **HUD messages.** An action with a `Sequence` and a `Title` shows the title with `{sp}` filled: the four
  "Discovered" actions use sequence 995814 (layout 1470322 for 1 s plus a sound; "Billboard +500"). An
  action's `GameplayTrigger` (list object 457533 in `GAMEPLAY.BNDL`) names output sequences:
  `SpeedCameraTriggered` → 1107082 (6 s: a PostFX flash, a world-space plate "SPEED CAMERA" with
  `Collectables.LastCameraSpeed(SPEED, DECIMAL)` whose layout instance is scaled 0.005, and speedwall stacks
  that only show with friends' entries), `SecurityGateHit` → 995322 (the "SPEEDWALL / SECURITY GATES
  {Collectables.SpeedwallColFound} / {SpeedwallColAmt}" panel, 0.5–6 s). *Inferred*: the trigger fires its
  outputs when the action scores (its inputs were not read).
- **The total's label** (free-roam HUD layout 979928, `{ScoringViewModel.CurrentSpeedPoints}`): visible with
  the widget `SPVisible`, which the game-logic state machine "SP Visibility" (HSM node 1695370) drives: "Show
  SP Points" for `TIME` 8 s at the start, then again for 8 s whenever the binding
  `ScoringViewModel.CurrentSpeedPoints` changes, and through a pursuit.
- **No nitrous from security gates.** `FUN_0074cf40(this, worth, action)` (sends the action, adds `worth` to
  `this+0x1224`, awards `DAT_00fa929c` nitrous through `FUN_00658ad0`) is a method of the Smash-and-Grab mode's
  scoring behaviour: +0x1224 is its `Score` variable, and its four callers (`FUN_0075ba30`, `FUN_0075ec80`
  and two in `FUN_00760xxx`) read its `ItemWorth` / `BillboardWorth` / `HighValueWorth` (+0x1248..+0x1250).
  `HSM.DoSmashAndGrabScoring` only runs under "Smash And Grab Mode" (HSM 2210003). `FUN_00658ad0` has no
  other caller but the gas station (`FUN_00748ff0`). The free-roam gate branch of `FUN_007059b0` sends the two
  actions above, a hint and two stats, nothing else.

### Jack spots: the parked cars and the swap (2026-10-10, `jackspots.rs`)

The HSM's "Freedrive States" start five behaviours: "Do Body Shop" (979770), "Car Swap Logic" (979767), "Regular
Autosave" (a `SaveProfile` every 120 s), "Speech Manager" and "Do Blacklist Shutdowns". "Car Swap Logic" is
`HSMBehaviourCarSwapLogic`, registered as the script object `JackSpots` (`FUN_0075d510`; variables
`IsSwapSpotVehicleSpawned`, `IsSwapSpotFound`, `HasCarInSwapSpotBeenDriven`, `UsedSpotBefore`, `OriginalCar`,
`IsSwapComplete`, `SwapTargetManufacturerMovie`, `SwapSpotBadgeId`, `BadgeIds`, `GamePackImageIds`). Read from
the code unless marked. **B** = the behaviour object.

- **Slots** (`FUN_00747aa0`): one 0xa0-byte slot per `CarSwapSpot` (the list at collectables manager +0x41b0,
  which settles that list's type), from B+0x50, count B+0x73f0, plus one empty slot (index B+0x74b4). Slot
  +0x60 = `Placement` (the location trigger), +0x64 = `VehicleId`, +0x68 = `0e3bcabe` (the car's trigger), +0x6c
  = `e36e9d9d` (a **second parking place**: in the data a trigger 2.3–3.8 m beside the car's, facing the same
  way), +0x70 = `ColourId`, +0x40 / +0x50 = the car trigger's position and direction (`FUN_0042f250`), +0x74 the
  parked vehicle's handle, +0x78 its state (0 none, 1 requested, 2 spawning, 3 standing), +0x90 the squared
  distance, +0x98 eligible, +0x99 in range. Spots whose car's game pack is not owned are left out
  (`FUN_00643c10` … `FUN_00647700`).
- **Distances** (`FUN_007615f0` → `FUN_007610c0`): a physics query for triggers (type 0x20, up to 150) in a
  sphere of **300 m** (`.rdata` 0x00fd6800) around the player's vehicle keeps the jack spot location kinds
  (`FUN_006e16c0`: '<' 'B' 'E' 'K' 'N'), sorted by squared distance. The nearest **five** give their slot its
  distance; every other slot gets FLT_MAX (`.rdata` 0x00fd1304). The nearest one's vehicle is preloaded
  (`FUN_0071c8e0`); the nearest two fill the badge / game-pack image lists for the UI.
- **Which cars stand** (`FUN_0075d290`, per slot each frame): wanted = eligible and ((400 ≤ d² ≤ R) or
  (standing and d² ≤ R)) with R = **90000** (`.data` 0x010c5ca4; 1e6 at 0x010c5ca8 in the intro free drive) and
  400 at `.data` 0x010c5cac: a car is requested between **20 m and 300 m**, never closer, and a standing one
  stays inside 300 m. Wanted and state 0: spawn request `FUN_0066bf90(vehicle id, colour, position, direction,
  zero velocity)`; not wanted and state ≠ 0: removed (`FUN_00758b00`), so a car still loading when the player
  comes within 20 m is dropped. Eligibility (`FUN_00742400`, refreshed on entering range and one slot per frame):
  the slot has a vehicle, the profile has the car (`FUN_0068f190`), and it is not the slot named by
  `DAT_010c5bb4` (set from Lua: Easydrive's `JackIt`); in the intro free drive (game-mode manager +0x2e91) only
  location 1212078 (the 911 of "Exit When In Jack Spot Car") is eligible.
- **Prompt and input** (`FUN_007615f0`): per slot, if the player is not in a pursuit (police table +0x874 ≤ 0)
  and player +0x23f is clear, |speed| (`FUN_006ba3e0`, mph) ≤ `GameLogicConfig` +0x284 (`ff4aef6f`) = **70**,
  the slot's car stands, and the player's vehicle is **in the slot's location trigger** (`FUN_00742210` with
  flag 1: the vehicle's trigger-hit list holds the `Placement` id, `FUN_00537190`; in the intro instead within
  22.5 m, `.rdata` 0x00fd6b70): unless the just-swapped latch B+0x74c9 is set, the prompt is shown
  (game-mode manager +0x2ca7 = 1, +0x2c70 = `GameLogicConfig` +0x16c `a1e83efa` = language 955092 "PRESS
  {Controls.ButtonPrompt["SWAP_CARS"]} TO DRIVE CAR", or +0x168 `PromptStore` 2094166 "… TO GO TO STORE" for
  a car the profile lacks; the HUD's `Mode.PromptString` bar, layout 604307). The input is player 0's action
  record at +0xc4 = action **14 SWAP_CARS** (state int at 0x1c + 12 × action; E / pad Y / mouse button 1) in
  state 2. A press starts the swap (B+0x7428 = slot, B+0x7458 = car trigger, player +0x784 = location;
  `FUN_0075d590`) or, for a locked car, opens the store (`FUN_00748090`). The latch clears once no slot passes
  the test.
- **The swap** (`FUN_00761d70`, state at B+0x74a0): 1 wait for B+0x74c4 (set outside the decompiled set;
  *inferred*: the manufacturer movie / transition has covered the screen; `FUN_0075d590` starts the config's
  sequences `GameLogicConfig` +0x8c 995872 and +0x90 1016424 and posts message 0x1f) → 2 make sure the slot's
  car stands, then `FUN_00758f00`: the player's vehicle becomes the slot's car and colour (`FUN_006c2020` →
  vehicle-manager message 3 and the repair 0x16; the profile's saved rollout for that car is applied,
  `FUN_00730ec0`), the parked vehicle becomes the car the player came in (`FUN_00844580`), the slot's vehicle
  id becomes that car's and the two vehicles exchange their physical state (message 0x17 twice) → 3 poses
  (`FUN_00647e00`): the player's vehicle on the parked car's pose (`FUN_00742530` / `FUN_00742580`), the
  parked vehicle on the second place (slot +0x6c) the first time and on the car trigger (+0x68) the next,
  alternating by slot byte +0x97; with no second place, where the player's car was → 4 wait until both are
  placed → 5 sequences → 6 both triggers of the slot are hit as found (`FUN_007059b0(…, 0, 0, 1)`: the car
  trigger and the location), first use sends the score event `ScoringConfig` +0x40 (955227, "CAR JACK USED") → 7
  the colours are exchanged too (`FUN_00844660`: the player's car gets the slot's `ColourId`, the parked one
  the player's old colour, which the slot keeps), B+0x74c9 = 1, back to 0.
- **Reset**: B+0x74b8 is the slot last swapped at; when the player is more than 300 m (d² > 90000, `.rdata`
  0x00fd67e0) from its car trigger, `FUN_007596b0` removes its car and rebuilds the slot from the data
  (`FUN_0075d800`). When the profile's car changes any other way, all cars are removed and all slots rebuilt
  (`FUN_00761d70`: `FUN_007592a0`, `FUN_00747aa0`).
- **Found**: as above, the location trigger counts on entry once the slot's car stands, in free drive.
  `FUN_00759410` tracks the nearest standing car (a sequence, `GameLogicConfig` +0x104 = 1560090, attached to
  it, and `FUN_006a2480(d², found)`; *inferred*: the HUD's jack spot indicator).
- Not traced: the script-variable getters (the registration function is not in the decompiled set), who sets
  B+0x74c4, the store path, which point-of-interest look jack spots use on the map.

**Port** (`jackspots.rs`; model decisions listed in its header): the slots, the 300 m / nearest-five distances,
the 20–300 m standing rule, the prompt (layout 604307 with `Mode.PromptString`), SWAP_CARS, the swap's result
(car, colour, poses, the car left on the second place, both triggers found, last swap spot), the latch and both
resets. Ours: origin-in-box for "in the trigger", every car available, an immediate swap with no movie, static
parked cars with one box collider, no pursuit gate, no map icon, no score event.

### Body shops (2026-10-10, `bodyshop.rs`)

- The HSM has a "Do Body Shop" behaviour (type `4985a74f`, code `DOBODYSHOP`, `FUN_0077b600`) in ten places:
  free drive's "Freedrive States" and the other driving modes. Its per-pass handler is `FUN_00748ff0(this,
  player index)`, called only from the behaviour's update `FUN_00749220` (missing from the pseudo-C; read
  from a disassembly of the user's dump image).
- **When a pass counts** (`FUN_00749220`, every frame, for each of the 24 player slots with a vehicle that is
  the local player or of kind 2 (+0x9c), with player bytes +0x23e / +0x23f clear, not `FUN_006ba8b0`, not the
  player at manager +0xa698; a kind-0 player with +0x250 set is skipped): it walks the vehicle's trigger-hit
  list (`FUN_005372f0`) for records of **kind 6**. If the shop's id is in the cooldown list (this+0x60: up to 4
  × {id, seconds}, count this+0x80) the player's inside byte (this+0x48 + slot) just stays set. Otherwise the
  shop is put on the list with **5 s** (`.rdata` 0x00fd2120; kind-0 players only, while fewer than 4 entries)
  and, **if the inside byte was clear** (and the global byte 0x01179dde, a one-shot "skip the next pass", is
  clear), the handler runs; the byte is set. In no shop the byte clears. So: a pass on entering, each shop at
  most once per 5 s; no speed or damage condition. The timers count down by the scaled frame time and expired
  entries are removed. After a pass (this+0x84) once the colour id differs from the one kept in this+0x88, the
  new colour is stored in the profile's rollout for the car (`FUN_006b96f0`, `FUN_00729f20`; offline only).
- **The handler**: unless the game-mode byte +0xe8 is set (`FUN_006dfed0`; *inferred* online), vehicle-manager
  message **0xc** (`FUN_008446a0`); it returns for a player whose +0x9c is not 0; stats 0x4a and 0x49 +1; then
  `FUN_00639b00` (pursuit side), `FUN_006beb70` (zeroes five player words at +0x7c), message **0x16**
  (`FUN_00844940`), `FUN_00658ad0(player, 100)` (`.rdata` 0x00fd2104: a full nitrous tank), `FUN_006b97e0` when
  `FUN_006c2360` (player +0x3d8 = a table's u16 +0x10e; *inferred* the heat value), and the score events
  `ScoringConfig` +0x164 (955328) and +0x28 (955329).
- **Messages** (the vehicle manager's queue, dispatched by `FUN_00596880`): **0x16 → `FUN_0058f060`**, the
  repair (damage grid zeroed, physics messages 0x12 = wash and 0x11): the link that was inferred before is
  direct. **0xc → `FUN_00590cb0`** = `FUN_0058bc60` then `FUN_0058edd0`: the vehicle's colour id (+0x164) becomes
  the **next entry of its `VehicleColourPalette`, cyclic**: with `ColourGroups` (count byte +0x18) the groups'
  primary colours, else the plain list `16671e74` (count +0x19); a colour that is not in the palette stays. Not
  random. A second counter (+0x18e, modulo `GameLogicConfig` byte +0x2de) advances with it (use not traced).
  `FUN_0058edd0` copies the paint's values into the render instance. Message 3 (`FUN_00596200`) is "become
  vehicle id / colour", used by the swap.
- **Found**: the collectable hit (`FUN_007059b0`, kind 6) marks the shop in its kind's bitset (profile stat
  `bodyshops_found`; language 1444768 "BODY SHOP FOUND").
- Texts: 1444769 "Body Shops will repair your car, or spray it a different colour if it's undamaged",
  1682561 "Bodyshops repair your scrapes in an instant, just drive through". The code does both on every
  pass: neither the update nor the handler reads the damage.

**Port** (`bodyshop.rs`): on entering a kind-6 box (the inside byte and the 5 s list as above; ours tests the
car's origin against the box) the car is repaired and washed,
gets a full tank and the next palette colour (`AssetStore::vehicle_palette`); the shop is kept as found in the
profile. Not ported: the pursuit side, the score events, stats.

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
- **Starting grid**: `TPORTTOEVNT` (0x741270 → `FUN_006cd380`) passes the current **event id** (GMM+4 +0xe4,
  written by `FUN_006f06d0`; corrected 2026-10-09, it is not a trigger id: the respawn executor resolves the
  position from the event) and runs the reset
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
- **`AnimatedCameraSequenceItem`**: a clip (`CameraAnimation`) of the two-bone camera skeleton `0x6cc42`
  (`CameraSkeleton`; bone 1's bind rotation is 180° about Y, formats.md "Sequences"; the clip animates bone 1 only,
  30 fps, f32 quaternions), in world space or, with the `6f3ea350` binding `Players.LocalPlayer.GetTransform`, in
  the car's frame. Lens: the director camera at 0x2dc0 (`FUN_005c6c10`) takes FOV = 2·atan(18 / focal length) in
  degrees (`.rdata` 0x00fb0e94 = 18, 0x00faf850 = 180/π), the horizontal FOV of a 36 mm frame, and an aperture
  focal / f-stop (`DAT_00fb0ea8` = 64); `FocalLength`, `FStop`, `FocalDistance` are modulated values (462174 zooms
  24 → 10.4 mm in its 3 s shot). **Axes (settled 2026-10-09)**: the bone's key is the camera matrix as it is, look
  +Z, up +Y, X left (the rows the game camera holds: `FUN_00495b70` writes left, up, forward, position), roll
  included and the bind rotation not applied. Sources: (1) the subject survey: in 244 of the 251 camera clips that
  run while an `AnimatedRaceCarBehaviour` drives a car (or are bound to the player), the car stays within 40° of
  the bone's +Z, against 7 of −Z; (2) a capture of the original playing 462174's intro (`orig`, 2026-10-09,
  research log): its second to eighth shots are upside down in the original too, because those clips hold +Y down
  (the first shot rolls 175° over its 3 s, the barrel roll that turns the intro over), and every shot frames what
  ours frames with this rule. The earlier per-clip "up = whichever ±Y is upward" rule was wrong. The handler was
  not found in the decompiled set (the sequence-item vtables near `.rdata` 0x00ff2ad8 point at code Ghidra did not
  decompile, e.g. 0x866c60); the type id 0x6cc34 appears only in the `(type id, name hash)` table at 0x00ff2a60.
  **Bound clips** (`GetTransform`) follow the car live in ours; not traced.
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
  frame only while the flag is clear): the race outros freeze the finishing car's frame (confirmed by captures,
  2026-10-09: in the original the outro camera stays over the spot where the car crossed the line and the car,
  finishing at 59 mph, leaves the shot within 0.5 s; finished at rest it sits at the left edge of the top-down
  view, as in ours). The camera matrix (`FUN_00e278c0`): forward = look − eye, left = up × forward (ICE up =
  +z, or the subject's up when locked), up = forward × left, then `FUN_00e230c0` rolls it by `DUTCH`: the
  value × 65536 (`.rdata` 0x0106f9d0, f64) rounded to a 16-bit angle, ×2π/65536 (`FUN_00e22760`, 0x0106f5a8 =
  2π, 0x0107c138 = 1/65536), so `DUTCH` is in turns and its ±0.25 range is ±90°; the roll matrix
  (rows (1,0,0), (0,c,s), (0,−s,c)) pre-multiplies the camera rows, left' = c·left + s·up, up' = −s·left +
  c·up (what `camera_take` does). `FUN_00495b70` hands the rows to the game camera as left, up, forward,
  position in game axes. `GAME_CAM_BLEND` (read
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
  gameplay camera; LAG, BUNGEE, shake and Scene space of takes; the take channels LETTERBOX / FADE / OVERLAY /
  TIME_SCALE downstream; the outros' nested timeline controllers (jumps waiting for UI triggers).

### Port and model decisions (`cutscene.rs`)

Ported: the intro before the race (the race waits in `Waiting` on `RaceState::cutscene`, and the intro starts once
the opponents are placed), the outro after the player's finish, the skip, animated cameras, ICE takes with their
bindings and `GAME_CAM_BLEND`, the dynamic camera blend, teleports with their speed (and `RaceState::rolling_start`:
no freeze), the AI driving the player (`ai.rs`'s driver on the player's car with `MaxSpeedMph` as its schedule
speed), animated race cars, slow motion, the HUD, the letterbox (shared with the crash camera), traffic clearing.
Settled 2026-10-09 (captures): the camera bone's key is used as the camera matrix (look +Z, up +Y, roll kept);
every FOV (lens, ICE take, chase, bonnet, crash) is horizontal (`camera::projection_fov`); the outro's take
freezes the finishing car's frame. Model decisions: the race starts on the intro's event trigger (else at its end)
with no countdown; `GetTransform` bindings of animated cameras follow the car live; the animated car is placed
on its bone every tick with the clip's velocity
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
  `NormalisedSpeed` (speed / top speed), `IsWetSurface`, `IsReversing` and a per-car `Random`; the pool of five
  instances and the banks: "Other cars' engine sounds" (2026-10-09).
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
- Start with `m = base × net` (`world+0xa2b7c = +0xa2b74 × +0xa2b6c`; `net` = 1 in a local game). `base`
  (`world+0xa2b74`) is reset to `sys[0x80]` = 4.78 (`FUN_005e9a40`, copied from `+0xa2b70`), but **the event
  sets it** (message 4, below): **1.0 in free roam** (both snapshots read 1.0 there, with `+0xa2b70` = 4.78
  beside it). In a network game, `net = lerp(sys[0xc0], sys[0xd8], (activeHulls − c)·k)`, capped at 1.
- Player speed (local game, when `FUN_0069ffe0()`): with `s` = the player's speed,
  `t = clamp((s − sys[0x8c]·mph) / (sys[0x94]·mph − sys[0x8c]·mph), 0, 1)`, and `m *= lerp(sys[0x90]=1,
  sys[0x88]=1.2, t)`. With `sys[0x8c]` = 60 and `sys[0x94]` = 20, that is ×1.2 at or below 20 mph and ×1 at or
  above 60 mph.
- Pursuit (`FUN_006a1200()`): `m *= sys[0x10c]` (`PursuitScale` 0.5).

**Who sets the base: the event** (2026-10-09, confirmed). The traffic message queue (`DAT_010edb30` =
`world+0xb1810`, read in `FUN_0060a380`) is fed by small posting wrappers at 0x00845e80…0x00846520 (found by
scanning the snapshot's decrypted `.text` for the queue's address): `FUN_00845fb0(flag, value)` posts message
4 (+8 value, +0xc flag: set → base = value, clear → base = `sys[0x80]`), `FUN_008462a0` posts message 0xf (base
= 0 and the system restarts, `FUN_00615020`: no traffic). Callers:
- `FUN_0070caa0` (the event set-up, offline branch): if the event's `TrafficEnabled` (+0xf0) is clear → message
  0xf; else message 4 with the event's `ce4961e7` (+0xb0) when its `90f52617` (+0xf1) is set, else
  `TrafficSystemConfig` hour array `[hour]` (`sys[0x0c..0x6c]`, reached through `AIConfig` +0x24 → +0x34 →
  +0x24 handles; resolved in the snapshot). The online branch (`+0xe8`) uses the `OnlineEvent`'s `999297ea`
  (+0x38) and `b9157a03` (+0x48).
- `FUN_006e6380` (per frame): when the event uses the hour array and the hour (`FUN_00897980`, an int at
  +0x1b614) changed, message 4 with the new hour's value.

Data (`GAMELOGIC/GAMEMODES.BNDL`, 111 `OfflineEvent`s): the three free-drive events (game mode 390145) have
density 1, fixed, traffic on; races (730000) 0.1–1 (most 0.2–0.5); `TrafficEnabled` is off in 5 events; only 2
use the hour table (0.05 at night … 0.5). So free roam runs at **1.0**, not the 4.78 the port used.

**Load budget and culling** (confirmed, `FUN_005fd6a0`; corrected 2026-10-09). It walks the slots set in **both**
the live mask `world+0x1af40` and the crash-record mask `world+0x1af40 + 0xa8` (`+0x1afe8`, from the
disassembly: `and edx,[eax]` with `[eax+0xa8]`). A slot gets a crash record (0x620 bytes at `+0x2c910`, 16 of
them) only in `FUN_0061cf50`, called from `FUN_0060c3a0` when a physics body touched the car (vehicle +0x6b bit
0) and from `FUN_0060c2f0`: **only crashing traffic counts**; lane followers never do (both snapshots: no crash
records, load 0, with 1 and 3 lane cars within 60 m). The measuring point `world+0x36110` is **the player's car**
(snap2: (925.30, 22.38, −101.09) against the car's (925.35, 22.37, −100.81); snap1 likewise). A crashing car within
`sqrt(DAT_0110f590)` (60 m) adds:
- `sys[0x9c]` = 30 if the car is physical (byte `+0x1d2` of its 0x620-byte record at `+0x2c910`);
- otherwise `sys[0xa4]` = 5 if the value at `+0x1c0` is below `sys[0xb4]` = 6 (speed, inferred);
- otherwise `sys[0xa0]` = 20.

At a load of 200 or more, spawning stops and a cooldown (`+0xa2b84`) counts down. When it runs out, the
**farthest** crashing car (at any distance) is removed (`FUN_005ec4b0`, with its linked slot) and the cooldown
restarts at `sys[0xa8]` (0.5 s). The budget allows about 6 physical crashing cars near the player.

**Live traffic counts** (snapshots, vehicle table: live mask `world+0x1af40`, positions at `world+0xd7b0 +
slot·0x40`, player position at `*(*(0x10edae4)+4) + 0x3050 + i·0x1c40` + 0x110, i = `world+0xa2b30`):

| snapshot | player | speed | lane cars (slots 0–255) | within 100 / 200 / 250 / 400 / 600 m | parked (256–405) |
|----------|--------|-------|-------------------------|--------------------------------------|------------------|
| snap1 | (1814, 23.6, 1649) | 115 mph | 96 | 12 / 27 / 34 / 56 / 86 | 60 |
| snap2 | (925, 22.4, −101) | 38 mph | 108 | 1 / 5 / 9 / 31 / 75 | 77 |

(`traffic.rs` at the same places, standing, after 25 s: snap1 spot 103 cars, 10 / 28 / 37 / 61 / 87; snap2 spot
126–135, 3–5 / 8–10 / 10–14 / 30–38 / 85–90. With the old base 4.78: 255–256, 17 / 71 / 86 / 132 / 216.)

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
- While `world+0xa2be6` is set, score 0 (the manoeuvre) is forced to zero (corrected 2026-10-16: the scores are
  16-byte SIMD vectors and the mask `DAT_00fb2330` clears only the first lane). Scores are also zeroed when
  `world+0xa2ba8 ≤ 16 − world+0x32b20`, a cap on concurrent manoeuvres (inferred); in a local game that test is
  skipped at a load of 200 or more. A car whose state byte `+0x78` is 2 (a patrolling cop's ghost, "How a cop
  patrols") gets score 0 zeroed the same way: a ghost never starts a manoeuvre.
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
  cars; ported 2026-10-10, "Parked cars" below).
- **Parked cars** (2026-10-10, ported in `traffic.rs`; confirmed unless marked):
  - *Placement* (`FUN_0060db90`, after the lanes; the whole function returns while the multiplier
    `world+0xa2b7c` is 0): per record, one LCG step whose high word (taken before the step, without the ring)
    gives `h % 100 + 1 ≤ chance`; skipped when `world+0xa2bf1` or `FUN_006a1200()` (a pursuit, inferred) holds
    and flag bit 0 is set; `FUN_00608c80(flow type)` draws the vehicle; a slot from the free list
    `world+0x34bd7`/`+0x34c70` gets the 12-byte record at `world+0x2bbd8 + slot·12` (`u16` hull, `u8` record,
    `u8` state = 1, then the pick).
  - *Spawn* (`FUN_0060b2f0`): each placed record (state bit 0, not 8) is spawned at the record's matrix by
    `FUN_006080d0` into vehicle slot `256 + slot` with no colour asked, so the paint is the type's round robin
    (`world+0xa0390`), as for traffic. While `world+0xa2bee` is set (cleared at the end; its writer not
    found), a record within `sqrt(world+0x35f94)` = 30 m (900 in `snap2`) of any of the 16 race cars
    (`FUN_00573390` / `FUN_00573420`; the player, AI racers and, inferred, the jack-spot cars) is flagged 0x18 and not
    spawned. `FUN_005f0aa0` releases records flagged 4 (`FUN_005ec6b0`).
  - *Snapshots* (the record table read live): `snap2` active hulls 179, 212–214, 232–234 (ours the same set): 77
    records live, ours 79 (the original lacks hull 232's records 3 and 4, both pursuit-flagged, at (1535, −527);
    why is open). `snap1` hulls 237–239, 253–255, 269: 66 placed (60 spawned, 6 flagged 0x18 by a race car
    within 30 m: upper decks of the car park at (1800, 30, 1990)), ours 72 (the original also lacks hull 255's
    ground-floor records 5–7, 13, 15, 16; open).
  - MODEL: the "start over" flag is read as "no hull was active" (the first area after loading), the clearance
    is tested against the player only, the pursuit test is false; parked cars are removed with their hull (as
    lane cars). A parked car is a kinematic body until a dynamic body touches it, then a crashed car like hit
    traffic (the same contact hook, `car_impacts.rs` impact). Records without collision under them (yards,
    car-park decks: many of the 392) are scenery the player cannot reach.
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
  cruise. `world+0xa2be6` zeroes score 0 only (2026-10-16), and `world+0xa2ba8` (`sys[0x134]` = 7) ≤ 16 −
  `world+0x32b20` gates the scores (not identified; not ported).
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
entry was not found), the kinematic wheel and suspension pose (`FUN_005f7cf0`), the world vehicle
allocation for lane followers (`FUN_00608fb0`, 80 m / 30 m tests from `sys[0x118]` / `sys[0x11c]`), and per-car
lights.

**Runtime model decisions** (each marked `MODEL:` in traffic.rs): option 0 keeps a car stopping for its
obstacle (no manoeuvres); the draw range measures from the player's car (the load budget's point is the player's car in the game too, confirmed 2026-10-09); models and
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
  5·max(0, 0.98 − cos Δ)³ (`FUN_0052e250`, 2026-10-15: a cosine; the angle reference is open), × 5 on shortcut exits and × 100 on jump
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

## Police and pursuits (2026-10-10)

Read from the dump (pseudo-C; a few getters the decompiler left as labels from the disassembly of the dumped
image), `.rdata` constants from the exe on disk, data with `mwtool police` / `mwtool genesys`. Ported in
`crates/mostrusted/src/pursuit.rs` (the rules) and `police.rs` (cops), data in `mw_assets::police`. Labels:
**confirmed** = read in the code or data, **inferred** otherwise. `rec` = the player's pursuit record (0x8b0
bytes each, at `*(world + 0x2f410)`), `P` = the player object (0x5e0 bytes), `GLC` = `GameLogicConfig`
(`GlobalConfig` +0x24; `GlobalConfig` is `DAT_010edac0` / `DAT_010edac4`, +0x0c `AIConfig`, +0x44
`ScoringConfig`), `H` = the player's current `HeatLevel` record (`FUN_006bdc00`), `AIC` = `AIConfig`.

### The data (confirmed)

- **`HeatLevel`** (0x118 bytes; 40 rows in the list 390428 of `GAMELOGIC/GAMEPLAY.BNDL`, `GLC` +0xf8). Free
  roam uses the seven rows `GLC` +0xa0 names (count +0x2d0): level numbers 0–6 (`b2266b6e`, +0x116) with
  `Threshold` (+0x10e, the heat at which the level *ends*) 0 / 2 / 8 / 16 / 26 / 38 / 52. The other rows are
  the events' levels (fixed level flag +0x113).
- **Heat** is a float on the player (`P` +0x3d8, `Player.ChaseHeat`). The level for a heat value
  (`FUN_00669640`): level 0 when the heat is 0 and the player is not chased, else the first row from the second
  on whose threshold is above the heat. The row below it (`FUN_00669860`) gives the lower bound of the level's
  band. Heat is capped 0.001 below the highest unlocked row's threshold (`FUN_006673a0`).
- **What adds heat** (`FUN_006d08a0`, the scoring-action processor → `FUN_006bdac0`): every `ScoringAction`
  has a heat value (`1a88cb4c`, +0x24). In the data: chase initiated 0.5, back from cooldown into the chase
  0.5 (`ScoringConfig` +0x58, no title), a cop takedown 1 (the cop's `AIPlayerType.23f2735d` action), roadblock
  1, roadblock smash 0.25, slam / ram 0.2, traffic hit 0.1. And time (`FUN_00657e60`): while chased, `Threshold
  / 2d163b85` (+0x100, seconds: 300 / 330 / 380 / 430 / 530 on levels 1–5, −1 = none on 0 and 6) per second.
- **Per-level fields read by the code** (values of free-roam levels 0 / 1 …): cooldown time `c422193a`
  (+0x8c: 14 / 20 / 20 / 20 / 19 / 18 / 17 s), lose radius `ed9f5c6a` (+0xdc, 150 m), cooldown allowed
  `249ed427` (+0x110), level-up message `a504e53b` (+0x6c), cop spawn distance free / chase / cooldown (+0xd4 /
  +0x80 / +0x88: 350 / 200 / 400 m at level 0, 300 / 175 / 300 at level 1), removal distance (+0xd0 / +0x7c /
  +0x84: 425 / 250 / 450; 350 / 350 / 400), sight ranges (+0xa4 chasing 100 / 140, +0xa0 alert 90 / 130, +0xa8
  other 50 / 75 m), no-cone range (+0x90 for a target under 5 mph, +0x94: 50 m), cone half-angle (+0x98 /
  +0x9c, 40°), memory `ac12feb2` (+0xbc, 2–3 s), drive-to-point timeout (+0xc0, 10 s), speeding limit
  `78f472fa` (+0xd8: **110 mph at level 0**, 300 above), search / go-to speed cap (+0x104, 60–75 mph),
  low-speed-chase limit (`Coordination.7a65b100`, 40–50 mph).
- **Cops per level**: `Coordination` (+0x0c) holds the main `PursuitBehaviour` (`dd6594c8`, +0x30), a fallback
  (`e8cb6438`, +0x0c) and a cycle array (+0x58). A behaviour lists `CopBehaviours`: `10a80aa8` a
  `Gameplay.CopType` id (list 1056172; → `AIPlayerType` → `Rollout` → `Vehicle`), `a7620186` a speed cap in
  mph, `CopBehaviour` (0 / 1 Chasing, 2 ChasingBruiser, 3 Rhino, 4 Interceptor, 5 Idle, else keep,
  `FUN_005139a0`), `Positioning` (int8). Level 0: one Ford Police Interceptor (122713) that keeps its type's
  default behaviour (patrol); level 1: two; level 2: three and roadblocks; level 3: Dodge Chargers (122706,
  160 mph cap); level 4: heavy Interceptors (510072) and a Rhino; level 5: Corvettes (122699, 200 mph); level 6:
  six cops with S.W.A.T. vans (1467242). `mwtool police` prints the table.
- **The bust rule's constants**: `GLC` +0x288 = 20 m, +0x28c = 25 mph, +0x290 = 4 s; cop radar range +0x218 =
  200 m.

### The pursuit manager (`FUN_0066b900` → `FUN_0066ae60` per player per frame; confirmed)

State `rec` +0x89c: 0 none, 1 chase, 2 busted, 3 cooldown, 4 escaped (`Player.IsInChase` … read it; the
telemetry event names them "Busted" / "Escaped", `FUN_0066a820`). Each update first counts the cops chasing
the player (`FUN_0065ba70` → `rec` +0x88c: cop-type players (`AIPlayerType` byte +0xb5) with a physical car,
not crashed, for which `FUN_006bbb70` holds: the cop is in state 0, 0x10, 0x12, 0x16, 0x17, 0x18 or 0x1a, or
its coordinator record is marked), then runs the bust meter, then the state:

- **0 → chase** when a cop chases the player or the player took a cop down (`FUN_006ccd20` → `FUN_006b8d00`
  sets `rec` +0x8a4) or touches a roadblock's car (`FUN_0065b930`); in modes without a cooldown at once.
  `FUN_0065c550` stores the player's position as the last-seen point and one of four "CHASE INITIATED" scoring
  actions is posted (`ScoringConfig` +0x48 / +0x4c / +0x50 / +0x54, by who spotted the player; all add 0.5
  heat).
- **Chase**: the last-seen point (`rec` +0x860) follows the coordinator's freshest sighting while a cop
  knows where the player is, or the player while a roadblock stands within 100 m (`FUN_0065bca0`); the player
  is *out of range* (+0x8a5) when farther than `H` +0xdc (150 m) from it. Heat rises with time. A player
  wrecked by an AI car is busted (`FUN_0065c880`). Out of range → **cooldown** (`FUN_00661810` →
  `FUN_0065cbe0`). The map shows a circle of twice the lose radius at the last-seen point (`FUN_0065be70`).
- **Cooldown** (`FUN_006615c0`): unless a cop is within 20 m (0x00fb74c8), the heat falls by (band width of
  the level, at least 1) / `H` +0x8c per second (× 4, 0x00fb74b0, when the player's vehicle field +0x1a84 is
  0: **not identified**, see open points). `Player.TimeTilEvaded` (`P` +0x3e8) = heat / rate below level 2,
  else the level's cooldown time, 0 with no heat. A chasing cop or a roadblock within 100 m → chase again
  (`FUN_0065c970`, +0.5 heat); otherwise at `TimeTilEvaded` ≤ 0 → **escaped** (`FUN_0065ca70`). So a cooldown
  from level 1 takes up to 20 s, from level 3 up to 60 s.
- **Bust meter** (`rec` +0x874, `Player.StaticBustProgress`; `FUN_00664d60` → `FUN_0065cda0`, chase only):
  with the nearest chasing cop (not crashed): it fills by dt / 4 s while the cop's car touches the player's
  and the player is slower than 25 mph, or while the cop is within 20 m (and 4 m of height, 0x00fb82d0) and
  the player slower than 12.5 mph (× 0.5, 0x00f8a810); otherwise it drains at the same rate. The BUSTING bar
  shows above 0.3 until it is back at 0 (`FUN_006bdf50`).
- **Pursuit bar** (`rec` +0x870, −1 … 1): with d = that cop's distance (≤ 100 m), far = clamp((d − 20) / 80),
  close = 1 − clamp((d − 20) / 30), fast = clamp((mph − 25) / 275): clamp(0.8 far − 0.3 − 0.2 (1 − fast) close,
  ±0.5); with a bust meter ≥ 0.01 instead min(0, −0.5 − 0.5 × meter). Three ranges (`FUN_00661500`: −1…−0.5,
  −0.65…0, 0…1) give `Player.PursuitBarEnum` / `PursuitBarProgress`. At −1 (meter full) the manager posts
  camera event 0x1a with the two cars; state 2 is set by the game-logic state machine's action 0xf
  (`FUN_00854990` → `FUN_00658140`). **Inferred**: that the event leads to that action.
- **After the outcome** (HSM 457517 "Do Pursuits", data): BUSTED → busted sequence 1014946,
  `ResetLocalPlayerHeat`, `ResetProps`, `RemoveAllCops`, `RemoveRoadblocks`, `ResetChaseStats`; ESCAPED →
  escaped sequence 1394359 (if `ShouldShowEscapeSequence`), `ResetChaseStats`. The coordinator spawns no cops
  for 90 s after a pursuit (0x00fa2cc0).
- Getters: `HeatLevelInt` = the level number, `HeatLevelFloat` = it plus the band progress (`FUN_006bdc50`,
  also `HeatProgress`), `IsHiddenFromCops` = coordinator byte +0x418 clear, `CopRadarProgress` = nearest cop's
  distance / 200 m (`FUN_0065d470`), `ShowCopRadar` = that below 1 with no pursuit.

### Cops (the AI coordinator `FUN_00526c30`; from a sub-agent's trace, confirmed unless stated)

- Behaviour commands (name table at 0x010c4360, mapped to driver states by `FUN_004efc40`): Chasing → state 0,
  ChasingGoTo 1, GoTo 4, Idle 5, Searching 0xc, Patrolling 0xd, Waiting 0xe, WaitingOnPlayer 0x10,
  WaitingInterceptor 0x11, Rhino 0x12, HeadOn 0x13, LowSpeedChasing 0x16, LowSpeedChasingAhead 0x17,
  ChasingBruiser 0x18, Interceptor 0x1a, RollingRoadBlock 0x1b. A cop type without a behaviour patrols.
- **Count and cars** (`FUN_00526e30`, once a second): one cop per `CopBehaviours` entry of the current
  behaviour, at most 3 new per pass, only while the behaviour's `f3d7aeeb` holds; surplus cops (farthest
  first) are marked unwanted. The sequencer (`FUN_0051f010`) runs the main behaviour for `4fea2957` seconds,
  then the cycle items, with the fallback after `f58d1d54` events.
- **Where**: a road-graph node between d and √2·d from the player, d = `H` +0xd4 / +0x80 / +0x88 (no pursuit
  / chase / cooldown; `FUN_00513e80`); ahead, behind or from a side road by weights of the `GameMode`
  (`FUN_005261d0`, not decoded); spawn speed `AIPlayerType.SpawnSpeed` (90 mph). Removed (when off screen)
  beyond `H` +0xd0 / +0x7c / +0x84 (`FUN_00522c40`). No catch-up teleport: despawn and respawn is the rubber
  band (inferred).
- **Sight** (`FUN_006c5810`, `FUN_006c6280`): the range by the cop's state; within the no-cone range or the
  cone, then a raycast; a clear line sets "sees", the last-seen point and the memory timer.
- **Wanted** (`FUN_005192b0`): see "When a pursuit starts (2026-10-16)" below. A cop that knows a wanted
  player chases; when the target is no longer known it drives to the last-seen point and searches
  (`FUN_005156c0`, `FUN_0051a4e0`), else patrols ("How a cop patrols (2026-10-16)").
- **Chasing** (state 0): slot = `AIC`[0x1c4] (−5) − `Positioning` × `AIC`[0x5c] (12) metres behind the
  target, ±3.5 m to the side. **Speed** (corrected 2026-10-14/16): `FUN_004fa480` computes the gap law of
  `FUN_004fa580` (the target's speed − (lead + slot) × 1.5 mph/m within ±60 mph, at least 30 mph, within 20 s
  of the target) but, for a cop **with a physical car**, discards it (`fstp st0` at 0x004fa56b) and returns
  FLT_MAX: a physical chasing cop drives flat out under its `CopBehaviours` mph cap. The gap law only paces cops
  without a physical car (far from the player, advanced along their path). Within 40 m it aims 0.5 s ahead of
  the target (ram mode, `FUN_004ffcb0`); `cop_driver.rs` has the states. Bruisers steer
  into the car from alongside; a Rhino comes head-on; below the level's low-speed limit within 60 m cops pin,
  ram or stop (state 0x16). Roadblocks: at a trigger region 300–600 m ahead on the player's predicted route,
  `ae2d8f7d` s after the behaviour starts and every `cbbeb3e0` s. No cop spike-strip logic and no helicopter
  reader were found (`Helicopter` is 0 on all 40 levels).

### When a pursuit starts (2026-10-16; confirmed unless stated)

Nothing starts a pursuit by proximity. The pursuit manager starts the chase when a cop *chases* the player
(`FUN_0065ba70`'s count) or the player took a cop down / hit a roadblock car (above); the coordinator puts a
cop into a chasing state only when the player is **wanted** and the cop **knows** where the player is.

- **Perception** (`FUN_006c5810` per cop, one target player per call round-robin; block = AI player + 0x250,
  per target slot i): seen-age +0x0 + 4i, memory +0x60 + 4i (= `H` +0xbc on sight; while above 0 the
  last-seen point +0x170 + 0x10i follows the target's true position), "sees" +0x105 + i, "in cone"
  +0x11d + i. Range: `H` +0xa4 (chasing: the driver in state 0, 0x10, 0x12, 0x16, 0x17, 0x18, 0x1a,
  `FUN_004f2b20`, or the coordinator's chase role `FUN_00516390`), else `H` +0xa0 when the coordinator marks the
  cop alerted (`FUN_00516400`, coordinator record bytes +0x1c / +0x1f, writer not traced), else `H` +0xa8
  (level 0: 100 / 90 / 50 m). `H` is the *target's* heat level (`FUN_0066aa30`). Beyond the range, or beyond
  the no-cone range (`H` +0x90 for a target at ≤ 5 mph, 0x00fa8538, else +0x94; 50 m) and outside the cone
  (half angle `H` +0x98 alerted / +0x9c otherwise, 40°, × 0x00fbfe0c deg→rad): "in cone" = 0, "sees" =
  `FUN_00657ee0` (the target within 100 m, 0x00fb77e4, of one of the two 0xff0-byte records of world +0x2f484:
  **not identified**), and the rammed flag +0x104 is cleared. Otherwise a ray is queued from the cop to the target
  (`FUN_006c6280` reads it back): a clear line sets "sees" (sticky); while it sees, "in cone" = the target
  inside the cone angle (`FUN_006c05b0`), the last-seen point and the memory are refreshed. A target within 20 m
  (0x00fc4f70) of the last-seen point counts as seen.
- **Rammed** (same function): each physical vehicle in the cop's contact list (world +0x5de2c count, +0x5de3c
  records of 0xa0 bytes, other vehicle at +0x80) whose 0.5 s timer (+0xc0 + 4v, 0x00fc0600) has run out adds
  the contact list's value at record +0x1c (units not traced) to +0x100; vehicles for which `FUN_0057a050`
  holds are skipped (a vehicle-type flag, not identified). Past `AIConfig` +0x60 (`a7a0cd14`, 2) the cop is
  rammed (+0x104 = 1) and the sum restarts.
- **Wanted** (`FUN_005192b0`, per player, any of): (1) a cop has the player "in cone" and the player's speed
  (`FUN_0083ea90`, mph) exceeds `H` +0xd8 (`78f472fa`: **110 mph** at level 0, 300 above); (2) traffic hits
  seen: `FUN_00515090` runs a window (coordinator +0x3e0 + 4i) down by dt and clears the count (+0x3e4 + 4i)
  when it runs out; while any cop has the player in cone, every entry of the frame's list at traffic world
  +0x33144 (count +0x331bc, max 20; filled by `FUN_0060c3a0` when the player's contact turns a lane follower or
  parked car physical, `FUN_005e9800`) counts one, the first opening a 10 s window (0x00fa99d0): **two traffic
  cars hit within 10 s in a cop's view** make the player wanted; (3) any cop rammed (+0x354 of the AI player);
  (4) `FUN_00656140` false (the pursuit system not set up, or the level without `249ed427`; never in free roam);
  (5) a chase or cooldown runs; (6) `FUN_006a0340` && `FUN_006a1570` (a per-player float at world +0x2f47c
  +0xdf0 not FLT_MAX: **not identified**); (7) coordinator byte +0x665 (**not identified**). "Known"
  (+0x400 + i) = a cop sees the player, remembers it, or has +0x385 / +0x39d set.
- Ported in `cop_sight.rs` (perception, wanted) and `police.rs`; `--test cops` checks a slow pass (no
  pursuit), speeding past (chase) and a ram (chase); the traffic-hit window has unit tests.

### How a cop patrols (2026-10-16; from a sub-agent's trace, confirmed unless stated)

Patrolling (state 0xd; vtable 0x00fa8694: enter 0x5060e0 → reset `FUN_00504fd0`, exit `FUN_00506120` releases
the ghost with traffic message 0x1c; update `FUN_005101e0`, dispatch on +0x90). The game drives a patrolling
cop **with traffic**: the cop gets a *ghost*, a lane follower with car +0x78 = 2 and +0x79 = the cop's
vehicle, never drawn and without a body (`FUN_00608fb0` skips the model for it), and rides it.

- **Mode 0** (`FUN_00501460`): the nearest lane point (`FUN_004fd790` → `FUN_005f3350`: nearest rung segment
  that the point projects onto, any lane direction) to the cop, or with +0x97 (set by Searching when it hands
  over) to a point 150 m (0x00fa8eec) away in a random direction, not within 60 m of another player's car;
  radius / height by the failure count +0x78: 400 m / 5 m, 400 m / any, 1000 m / 5 m, 1000 m / any.
- **Mode 1** (`FUN_0050bdd0`): drive there over the road graph (A*, the racers' command builder); every 4 s
  (0x00fa92b8) look the point up again, take it when 30 m off; within 30 m the speed ramps from 10 mph to 0,
  within 6 m the handbrake, within 8 m mode 2. Three 50 m probes (0°, ±15°) force full lock on a hit; a crash
  check sends the cop to the crash recovery state 11. Beyond the ramp the speed is read from D+0x5668 (not
  traced for this mode).
- **Mode 2** (`FUN_00509df0`): stopped; traffic message 0x1a asks for a ghost 5 m (0x00fa92f8) ahead of the
  cop; the traffic tick (`FUN_0060aba0`, one request a frame) places it at the nearest rung within 20 m
  (0x00fb2470), |dy| ≤ 5 m, in an active hull, with no player clearance, at the lane's cruise speed
  (`FUN_005f4d10`); asked again after 2 s (0x00f8a808) or on failure.
- **Mode 3** (`FUN_0050ce00`): merge: drive to the ghost; while the ghost is ahead and more than 15 m away it
  crawls (override +0x7a = 3: 10 mph; message 0x1b; values 1 stop, 2 5 mph, 3 10 mph in `FUN_0061a450`);
  behind the cop it drives normally; caught up when the cop's position + 1.5 s of travel is within 2 m of it
  with the ghost ahead; more than 40 m from the player and off screen: caught up at once; 8 s (0x00fa8e64):
  back to mode 0. The cop's speed is `.bss` 0x011061f8 (**not read**).
- **Mode 4** (`FUN_0050d490`): ride: the racers' steering, target speed the ghost's, brake while the ghost
  stands; within 5 m of the ghost's pose the body is set to it (physics message 0x2e, transform and velocity,
  every frame); farther, a pose request on the driver (D+0x584c, reader not traced). The ghost gone: mode 0.
- The ghost obeys traffic's rules: lane speed, the 5 m/s turn cap, junction routes by load, red lights, the car
  ahead, obstacles; only score 0 (the manoeuvre) is zeroed.
- **Spawn** (`FUN_0051b4e0`, up to 10 tries): the ring point snapped to the nearest road node, then the nearest
  lane point within 60 m / 5 m, facing **the lane's direction**; at least r from the player and 200 m (0x010c44cc)
  from every player's car; spawned with message 0xd at forward × `.bss` 0x01106228 (the "20 mph" is not
  verified). The cop type's default command (`FUN_006b8a40`: 12 → Patrolling) applies; `CopBehaviour` 6 keeps it.
- **Siren**: no switch was found tied to the driver states (the mix groups `Player_Vehicle_Sirens`,
  `Sirens_Roadblock`, `Sirens_Competitor` exist); the natural candidate is the chasing predicate
  `FUN_006bbb70` (inferred). `police::CopStatus` exposes it as `chasing`.
- Ported in `cop_patrol.rs` and `traffic.rs` (ghosts: `add_ghost`, `ghost`, `set_ghost_override`,
  `remove_ghost`, `nearest_lane`).

### The HUD (data)

Free-roam screen 371670, `AdditionalHUD`: layout 1213038 (HEAT / COOLDOWN labels, the stepped level bar bound
to `HeatLevelInt` over 0–6, a fill bound to `Widget.HUD.Heat`; elements gated by `IsInChase` / `IsInCooldown`),
581357 (the BUSTING bar, `StaticBustProgress` in sevenths), 955232 ("PURSUIT: {score}"), and the minimap frame's
pursuit tints. Speed Points: the pursuit code posts scoring actions through `FUN_006a8050` (chase initiated
+0x48…+0x54, heat level up +0xe8 / down +0xe4, escaped +0x1a8, roadblock +0x250): that is where pursuit
outcomes award SP once scoring exists.

### Port and model decisions (`pursuit.rs`, `police.rs`, `hud.rs`)

Ported as read: heat and levels, time heat, chase start, last-seen / lose radius, cooldown, escape, bust
meter, pursuit bar, cop count and types per level (main behaviour), spawn and removal distances, sight
(range, cone, raycast, memory, rams; 2026-10-16), wanted by speeding, two traffic hits in view or ramming,
the patrol on a traffic ghost, slots, spawn kinds, off-screen removal, the 90 s pause.
Model decisions (not from the game):
- Pursuit driving: `cop_driver.rs` (Chasing, LowSpeedChasing + box-in, Bruiser, Rhino, HeadOn) on road-graph
  or straight paths with the AI racers' driver; the Interceptor drives as Chasing; world queries are physics
  rays (the game asks its nav mesh).
- The patrol (2026-10-16): the ghost is ported as read; MODEL: the merge speed is the lane's cruise speed (at
  least 20 mph; `.bss` 0x011061f8 unread), a cop within 5 m of its ghost has caught up too, mode 1 beyond the
  ramp is capped at the level's go-to / search speed, the ride steers the body onto the ghost's pose by
  velocity in the ground plane and yaw (the game sets the full transform), a cop more than 5 m off its ghost is
  put on it when unseen and away from the player, else starts over (the D+0x584c reader is not traced), a
  ghost its cop no longer asks for goes after 2 s, and every cop not riding a ghost is an obstacle for traffic
  like a crashed car. Without traffic (`--no-traffic`) the patrol drives the road graph at the go-to speed.
- Perception: the alerted flag (`FUN_00516400`) is taken as "in a pursuit and going to the last-seen point,
  searching or waiting"; `FUN_00657ee0` (out-of-range sight) is false; the ram sum adds the closing speed in m/s
  of each new contact with the player's car (the game's +0x1c value is not traced; other vehicles not counted);
  wanted terms (6) and (7) are false.
- "Touching" = the cop's centre within the player's box grown by the cop's half width + 0.3 m; a cop crash within
  30 m of the player counts as a takedown.
- Only the main behaviour of a level (no sequencer, fallback or cycle); every heat level counts as unlocked.
- The outcome stands 13 s (the sequences' length; their cameras are not ported); after a pursuit every cop off
  screen is removed.
- The cooldown's × 4 factor does not apply to the player (+0x1a84 = 2, live read 2026-10-14); `Widget.HUD.Heat` = the level's band progress; which HUD layouts
  show when (the widget's code was not read).
- Not there at all: roadblocks, spike strips, wrecked-by-cop busts (our crashes have no wrecker), cooldown
  spots / engine off, Speed Points, the busted / escaped sequences (sirens, light bars and the radio: next
  section), pursuits inside events, the PostFX heat / cooldown colour cubes.

Open points and what would settle them: the patrol's `.bss` speeds 0x011061f8 (merge) and 0x01106228 (spawn)
and the contact record's +0x1c (ram sum) by a live read (probe `dump.req`); the out-of-range sight records
(world +0x2f484) and wanted terms (6) / (7); the link from camera event 0x1a to HSM action 0xf (a probe hook on `FUN_00658140`); the
spawn weights (`GameMode` +0x5a…+0x64: decode the type); `FUN_0050e370`, 0x0050f970, 0x0050dd40 (disassembly);
a capture of a real pursuit (`mwtool capture`: cop distances, heat and bust timing) to compare the whole.

## Coronas (2026-10-16, ported in `coronas.rs`, `corona.wgsl`)

- **Queues** (per view, `FUN_004a73b0`): race cars `FUN_00581640` / traffic `FUN_005fd190` → `FUN_004970b0`
  (≤ 0x60 cars) → `FUN_004a6630` → `FUN_004a6490` (sort by definition, skip at |car − camera| − 2.5 ≥
  `793b2691`) → `FUN_004a5f70`; world objects `FUN_00497180` → `FUN_004a6fa0` → `FUN_004a6b00`; lights
  `FUN_00497200` → `FUN_004a7290` → `FUN_004a6d50`; all end in `FUN_004a5790` (this = corona manager +0x50).
- **Definition** (`Genesys.Gen.Corona`, 224 bytes; runtime copy `FUN_0049d950`): +0x10 curve, +0xa4 `793b2691`
  max distance, +0xa8 `88b68533` crowded max distance, +0xac `c4e3e3a7` test-point offset (−0.5), +0xb0 Beams,
  +0xb4 Flares, +0xb8 `60c1d15f` (cube views), +0xbc Glows, +0xc0 / +0xc4 view types 2 / 3, counts at
  +0xca..+0xd4. Sub-entries (224 bytes): +0x10 curve, +0xa0 Colour, +0xb0 vector (glow `2d6e41d4`, flare
  `2b9cc12c` = (Position, Position, 1, radial), beam `034c050f` = (start, end, ·, w)), +0xc0 Material, +0xc8 (glow
  depth bias `a9531774`), +0xcc RotationOffset, +0xd0 `be0fbea3` (layer 7 / 8).
- **Curve** `FUN_004ab450`: per channel, below `c2cad262` t = `a2a9209e` + `c08c80f4`·x else `1eb30847` +
  `7c96a82d`·x; t = min(t, 1); out = (t · (`0b494d31` below, `b75365e8` above) + `d95ae391`) × scale.
- **Per entry** `FUN_004a5f70`: intensity = gate × `Brightness` × L² × night scale (group 1, 4/6, 7 →
  `VehicleCoronaScalesForNight` pairs, lerp(1, v, night)); size = (`f65c1f07`, `f65c1f06`) × size scale; per
  locator the `Animation` (linear) at the record's flash time, skipped ≤ 0; `Mirror` → second draw with x negated
  (`.bss` 0x011036a0 = (−1, 1, 1, −1)).
- **Per corona** `FUN_004a5790`: p, dir in view space; θ = acos(clamp(−p̂ · dir)); C = curve(θ, s·d, s·d, s·d) with
  s the view's +0x8b908; I = C.x·C.z·anim, skipped ≤ 0.05 (`.bss` 0x011040b0); S = size·C.y; size cull k = 2
  (0x010c3e10) / 1280 (0x00fa2fbc) · s · d; colour cull 0.01 (0x011042d0). Glows `FUN_004a4cf0` (curve at (I, I, I,
  d), half extents G.w·S·G.xy, colour G.z · colours · intensity), beams `FUN_004a5040` (curve at I × 4, widths B.x /
  B.y along the axis), flares `FUN_004a52e0` (curve at I × 4, ghosts at Position × p.xy, one visibility slot of
  ≤ 0x400 per corona; test point p + p̂·`c4e3e3a7`).
- **Drawing**: `Vfx_Corona` (glows, beams; layer 7, `gVfxConstants` (0, 1): linear HDR, depth-tested LESS_EQUAL
  without writes, the depth pulled by position.w), `Vfx_CoronaFlare` (layer 8, (1, 0): × the tonemap's exposure,
  gamma-encoded, no depth test, alpha × the visibility texel), `Vfx_CoronaVisibilityTest` (`FUN_004a5560`: one point
  per slot, 7 × 7 depth taps one texel apart, fraction behind). Effect masks: decomp.md "Exhaust pops and
  backfires" and research log 2026-10-16 (traffic: groups 0 and 4, L = 0.7 + 0.5 × flash / 255, rate 0.9 + 0.2 ×
  random).
- **Brake hold** (`FUN_0057a5e0`): car +1000 is zeroed while the brake is applied (+0x1930 > 0; in reverse with
  +0x192c > 0 it is zeroed too and groups 1 and 2 light) and adds the scaled frame time otherwise; while it is
  below 0.5 (`.rdata` 0x00f8a810) and `FUN_006a0110` (`FUN_006c37e0`: the player's state +0x9c is 2) holds, group 1
  stays lit. **Electrical flicker**: with car +0x3f5 or +0x1c19 set the mask gets 0x80c7 (bit 15 plus groups 0, 1,
  2, 6, 7); the setters of those two bytes and what bit 15 does in `FUN_004a5f70` were not read.
- **World objects** (2026-10-16, ported in `world_coronas.rs`): the runtime `WCVfxBehaviour` (`FUN_005b3350`,
  vtable 0x00faf238: +4 activate `FUN_005b30d0`, +8 deactivate `FUN_005b3150`, update `FUN_005b31a0`) packs its
  bools with `FUN_005b0220`: corona flags +0x2c = 1 `62f1c8f6` (data +0x5c), 2 `6e32f7bb` (+0x5e), 4 `74f925cc`
  (+0x5b), 8 `ad62f71e` (+0x5d), 0x10 `74ea21c1` (+0x5f), zero without coronas; light flags +0x2e = 1 `553d49f7`
  (+0x63), 2 `2dcdc3c8` (+0x65), 4 `45803296` (+0x62), 8 `0486dd39` (+0x64), 0x10 `4593369b` (+0x66), zero without
  lights. The counts are `9f8c05b0` (coronas, +0x54) and `e41cffe1` (lights, +0x56); +0x69 / +0x6b = they are non-zero.
  +0x6a (+0x6c) = coronas (lights) whose flags are not all five set: those are switched by the update, the rest only
  by activation. The update (`FUN_005b31a0`, round robin by +0x57): +0x6e = the hour (`DAT_0110d5d1`) in [+0x55,
  +0x56) = [`392e10d5`, `19e7191a`) (`FUN_005a7850`, wrapping), +0x70 = (+0x2c & the environment bits
  `DAT_0110d5d0`) ≠ 0, +0x6f the same for +0x2e; coronas are in the object's list (`FUN_00532760` add,
  `FUN_005327b0` remove; object +0xc4, at most 4) while +0x6e, +0x70 and the blink (+0x66: `c8d7741f` > 0, a
  random-rate square wave on +0x50) allow; lights likewise (`FUN_005328c0` / `FUN_00534080`). Activation adds both
  at once when the flags are all set and there is no hour window or blink. The object is queued (`FUN_00532a80`,
  `FUN_00533780` → `FUN_00497180`) while the camera is within +0xe8 = (the largest |locator| + the definition's
  +0xa8 `88b68533`)²; `FUN_004a6fa0` then takes each listed behaviour whose view bit (+0x2d, from the definitions'
  sprite counts, `FUN_005af1f0`) matches, level = `FUN_0089b400`(+0x2c, the object handle) (skipped below 0.001),
  and queues each locator (object matrix × locator; direction = the locator's z axis) with that level;
  `FUN_004a6b00` sorts by definition, skips coronas beyond the runtime definition's +0x64, and calls `FUN_004a5790`
  with anim = (level × 4), colour (1, 1, 1, 1), intensity 1, size (1, 1). Data: 37 corona behaviours; street
  lamps ("SetLightsOn") mostly night + twilight (flags 3), traffic lights and the speed camera all five (0x1f, by
  activation only), `…171cc4` none (never shown). **Light coronas** (`FUN_00497200`, ≤ 0xc0 per frame): from the
  light submission with a handle at +0x48 of the GeneSys object the record's +0xc / +0x10 names (not the light
  definition: its +0x48 is `AttenuationThreshold`) and current / definition intensity; that object was not traced
  (not ported).

## Speed camera flash (2026-10-16, ported in `sequence_fx.rs`)

A counted pass sends `SpeedCameraTriggered`, whose `GameplayTrigger` starts sequence 1107082 (`GAMEPLAY.BNDL`,
the sequence object `0x10e48a`; items listed by `mwtool frontend --sequence 1107082`). Its world items:

- **`GameLogic.ModifyBehavioursInAreaSequenceItem`** ×2 (0–0.1 s and 1.0–1.1 s; `Label` "SpeedCameraLight",
  `TransformBinding` `Players.LocalPlayer.GetPosition`, `Radius` 50, `ActionType` 0). Type id 0xf7a5c; the factory
  table at 0x00856271 → `0x008612a0` (runtime item, vtable 0x00ff2088). Start (slot 3, `FUN_00856370`): the
  binding's position + `TranslationOffset`, `FUN_004f4b00`(pos, `Radius`, type 0x2000, list of 10) for the
  objects around it, the label hashed (`FUN_00a1d5c0`), and per object `FUN_008467d0`(handle, hash, `ActionType` ≠
  1) which queues game-logic message 0xd (on) or 0xe (off); end (slot 9, `0x00851e20`): the reverse for the same
  objects. The world's message loop (`FUN_0055a680`, cases 0xd / 0xe) calls `FUN_00548510` /
  `FUN_00548560` → the object's virtual +4 / +8 with (index, hash): the behaviours with that label activate /
  deactivate. The speed camera compound `1e63a9d5` (74 placements) carries behaviour `…f8729` "SpeedCameraLight"
  (`543527d0` clear: off from the start; all flags set: lights and corona by activation only) with light `…f872d`
  (`Light_Point`, white × 500, threshold 0.1: 70.7 m) and corona `…f872c` at locator group 1017639. So the camera
  lights the world twice for 0.1 s and is dark otherwise.
- **`SequenceItems.PostFxOverrideSequenceItem`** (0–1.54 s, property 0; type 0xf7d8c, factory `0x0086a690`,
  vtable 0x00ff3020): each frame (`FUN_008635a0`) every `Automated_Values` entry's curve is evaluated at the
  item's time and stored by `Automated_Property` 0 / 1 / 2 into a 14-word block with flag bytes +0x32 / +0x33 /
  +0x34, handed to `FUN_0083b7a0` on game logic +0x2ef70 (flag +0x38). The PostFX manager (`FUN_008ecf80`) reads
  it: property 0 → its +0x1660 = value × 0.6 (`.rdata` 0x010219a4), property 1 → +0x1664 = value × 0.8 above 0,
  × 0.4 otherwise (0x010219a0, 0x00fae200), property 2 → +0x1508 and +0x1504 × value × 2 unless +0x1501.
  +0x1660 / +0x1664 are the final pass's brightness b / contrast c terms (above, "Final pass"). The curve here:
  0 → 0.5 at 0.05 (77 ms) → 0 at 0.095, 0 → 0.656 at 0.219 (0.34 s) → 0 at 0.279: two white-outs of b ≈ +0.3 /
  +0.39. The block is cleared every frame (`FUN_0083c9f0` at 0x006ab1d0). Other users of the item: the nitrous
  sequences (`…b3a2a`, property 2 then 0), `…f5fc1` (properties 1 and 2), `…f8ec8` (the same flash curve,
  0.55 s).
- **Sound**: `WaveSequenceItem`s at 1.333 s (`…f690e`) and 5.967 s (`…f690d`) on mixer channel `…7538` (gain
  0.224); a `BusMixerChannelSequenceItem` on bus route 259950 "Dominant 3D" (gain 0.435 rising to 1 over 4 s,
  not ported).
- **Timeline controllers**: at 1.3 s jump back to 1.15 s while `CollectablesVM.CameraSpeedwall.HasDownloaded`
  is false, at 1.233 s jump to 6.5 s once `…TimedOut`: the plate waits for the speed wall. Items are gated by
  `ShowMiniWall(NOT)`. Not run in the port (the clock runs straight, as the HUD's).

## Police sirens, light bars and radio (2026-10-16)

Read for "cops have no sounds, lights etc."; ported in `police_fx.rs`, `police_audio.rs`, `police_lightbar.rs`,
`police_speech.rs` (data `mw_assets::police_sound` / `police_lights` / `police_speech`). "Entity" = the
0x1c40-byte race-car entity; S = entity +0x17f0 (the siren voice object), B = entity +0x1840 (its parameters).

**Siren data and set choice** (confirmed): `FUN_005776a0` loads `CopSoundSet` `…23013` from
`VEHICLES\WAVES.BNDL` into manager +0x339e4. `FUN_00593ac0` (car creation) reads the car's `VehicleSound`
+0x40: a set of its own (510072, the SWAT car: `…b3af6`), else `CopSoundSet` +0x14 `9f467b30`
`[slot % count(+0x1a)]` (122713, 122699, 122706: `…23039` / `…23014` by slot parity); `efd7715f` has no reader.
The set handle lands at entity +0x1850 (`FUN_00591350`). Only cars with `VehicleComponent.54eb210e` (+0x8e) run
the siren (`FUN_00594870`).

**Requests** (`FUN_0058d770`, once per update, then zeroed): entity +0x1950 = 1 → level 0, 2 / 3 / 4 → level
1 / 2 / 3 (B+0x2c), 5 → blip timer B+0x1c = 0.8 s (`.rdata` 0x00fac278, counted down by dt); +0x1954 = 1 / 2 →
light flag +0x1c27 off / on. While the car is wrecked (damage byte +0xc20 ≥ the limit +0x74 of the object at
+0xc18) or crashing (vtable +0x58) the light flag is cleared and the siren stops. Writers of the requests:
- `FUN_00587720` (from `FUN_00591ac0`): for a siren car with a player owner, not crashing, each rising edge of
  the horn input (+0x1c25) steps off → level 1 + lights → 2 → 3 → off + lights off. For other cars the same
  function **plays the horn**: `VehicleSound` +0x60 list [0] through `FUN_00587650` (start 0.025 s, `.rdata`
  0x00fac294), released with a 0.1 s fade (`FUN_00a44e40`, 0x00fad540) — the player's horn runtime.
- Message 0xd (`FUN_00844300` → `FUN_00572d50`): byte 8 → lights on / off, byte 9 → siren level 1 / off. Sent by
  the script binding `SetSirenState(vehicle, on)` (handler 0x0067c3d0, registered at 0x0068d798; both bytes =
  `on`), `FUN_008470d0` (a game-mode routine: on), and `FUN_006c65f0` 1 s after a cop's takedown
  (`FUN_006ccd20`: lights off, siren level 1, then the car is reset).
- `FUN_005b4750` (cutscene cars, `AnimatedRaceCarBehaviour` +0x54: 0 off, 1–3 level, 5 blip; +0x5a lights).
- **Not found**: what switches the AI pursuit cops' sirens on (none of the above in the code read). MODEL
  (`police_fx::cop_fx_state`): cops that chase or search in a pursuit or cooldown have `SetSirenState(on)`.

**The voice** (`FUN_0058bdc0`, this = S): with the blip timer running and no wave pending, a random `Blips` wave
(LCG at 0x0110c690); else with a level, `Sirens[min(level − 1, n − 1)].Wave[slot % count]` (looped). A change
while playing: `FUN_00571ef0` (state 4); `FUN_00574510` keeps two voice slots: the new wave starts at once
(`FUN_00572070`) with gain = min(1, play time / 0.2 s) (`.bss` 0x0110d020), the old one at 1 − that; a stop fades
over 0.15 s (`.bss` 0x0110c940). `FUN_00571f30` sets each voice's gain = 1.6 (0x00fad920) × the fade and its
pitch = ((0.5 + 0.5 sin φ)·D + 1 − D) · B+0x28 (`FUN_00aabc10`; φ += global `VehicleSound.73e48fd4` (9.253) ·
dt · 2π, D = B+0x24 · clamp(1 − B+0x20 / `75ec96d4` (0.375 s)); an impact sets B+0x24 to 0.067 / 0.042 / 0.009
and B+0x20 = 0; not ported). Channel (`FUN_00572430`): the local player's car `Player_Vehicle_Sirens`, others
`Sirens_Competitor`, roadblock cars (+0xa40 bit 2) `Sirens_Roadblock`. When a car whose level was non-zero is
wrecked and no other car holds the slot at manager +0x339d0, `SirenSoundSet` +0x1c (`…b3883`, the dying
siren: the yelp 0–0.5 s, then one of four 2 s wind-downs, gains 3 / 3.5, channel `…76b17`) plays once.

**Light bar** (effect group 3 = +0x1c27, `FUN_0057a5e0`; `coplights` = 1.0 when +0x1954 == 2, getter 0x5a744a):
- `FlashAnimation` (`FUN_0049ef00`): T = Σ `Duration`; the segment is the one where frac(t / T) minus the
  segments' shares in order reaches ≤ 0 (else the last); f = (`Frequency` · t + `Phase`) · n with the absolute t;
  value = lerp(P[trunc f mod n], P[(trunc f + 1) mod n], f − floor f). t = the clock × entity +0x3e4, the
  per-car rate frac(slot × 0.39 + 0.5) × 0.2 + 0.9 (`FUN_00593ac0`). The Interceptor's bar: 8 s of a 20-sample
  pattern at 0.4 Hz then 8 s of 16 samples at 0.5 Hz (8 samples / s), outer → inner → centre, the sides 2
  samples apart.
- `_LocalEmissive` (G-buffer program at shader slot 0x878): S = Σᵢ Iᵢ (1 − sat(|Pᵢ − p|² · M² · Rᵢ))² over 16
  lights (`gaSelfIllumLocal_PosIntensity` slot 0x48, `g_SelfIllumLocal_RecipRadiusSq` 0x49, p = the skinned
  bind-pose position), emission = S · `EmissiveColour` · `mSelfIlluminationMultiplier` ·
  EmissiveTexture(uv0).x into RT3. `FUN_00583290` → `FUN_0057fcf0`: with group 3 on, the coronas of group 3
  with an `Animation` (both day and night entries) give v = anim(t)^10 (0x00f9f698), kept above 1e-4, the 16
  strongest; each locator whose +Z axis faces the camera is one light (P = its position, I = v, R = 222.767,
  `.data` 0x010c4794). The colour comes from the `Glass_Colourise` lenses (red (0.60, 0.07, 0.006), blue
  (0.07, 0.13, 1.48)). Side finding: `g_selfIlluminationMask` = 4.0 (0x00fad8c0) × entity +0x3cc × the
  brake / headlight / reverse bits there (our `--lights` default may be 4× low; not changed).
- The bar's light: group-3 `Light_Custom`s (blue `…253a4` / red `…253a5`: intensity 35, threshold 0.05), in
  `FUN_005818f0` intensity × `a3de907e` × entity +0x3cc × the animation (linear), through `FUN_008cf1d0` into
  the `LightBuffer_Cop` program: att = sat(1 / d² − c.w) × ((1 − L̂.y)⁶ above the pixel, 1 + 0.75 L̂.y below),
  diffuse only (Lambert for the world, the Ashikhmin-Shirley diffuse 0.3875 for cars).
- Coronas (`FUN_00581640` → `FUN_004970b0` → `FUN_004a6630`, per entry `FUN_004a5f70`): intensity = gate
  (`da6ac384`: 0 always, 1 by day fading with the night level, 2 at night; `FUN_0089b400`) × `Brightness` × L² ×
  the group's night scale × the linear animation value; size `f65c1f07` × `f65c1f06`; the draw `FUN_004a5790` /
  curves `FUN_004ab450` were not decoded (not drawn).
- Head and tail lights are on for every car (+0x1a8c = 1, `FUN_00591350`), off when +0x1a84 == 0 and +0x1a40
  bit 2 is clear.

**Radio** (speech manager: init `FUN_00840390`, update `FUN_00842ba0`, load `FUN_008404c0` / `FUN_008423a0`):
- Request `FUN_008408a0(UniqueId)`: the event (`FUN_0083d1e0`), its conditions (all or any by +0x44), then
  `FUN_0083d270` rejects it when played ≥ its maximum, outside its mode-time window, less than `Period` minutes
  since it last ended, needing a car without one, `Cutscene_Behaviour` 2 in a cutscene, or a one-off heard;
  `FUN_008406a0`: one speech at a time, no queue; a new one interrupts only with a lower `Priority` number.
- Per event (vtable 0xf994d8, `vt_00845b10`): `Delay`, then the items at their `StartTime`; a line item picks
  one line uniformly over all its lines and containers (`FUN_0083c5a0`).
- Triggers: `GameplayTrigger`s of the scoring actions (`FUN_006f9670` → `FUN_006e0a90`: the first event of the
  output whose conditions hold): chase started (`ScoringConfig` +0x48 / +0x4c / +0x50 / +0x54), resumed (+0x58),
  heat up (+0xe8 / +0xec), takedown (trigger 1378651), cooldown (+0xc4), escaped, roadblocks, hits, respray;
  HSM 983342 "Speech Manager": in a chase every 5 s by `GetRandomNumber[4]` (1186587 / 1378016 / 1871411 /
  1378788; within 25 m of a chasing cop 1186587 / 1378788 / 1186559 ×2), in the cooldown 1871531 (1871318 in a
  building). Event ids per trigger: `police_speech.rs` and the research log.

## Vehicle paint, liveries and dirt

The paint shaders' object constants are registered by name with slot numbers in one table function in the 0x8c4xxx range (a run of `FUN_00a5d490` calls; slot → name: 0x29 `VehicleID`, 0x2a `PbrPaintDiffuseColour`, 0x2c `PbrPaintSpecularSettings`, 0x2d `PbrPaintClearcoatSettings`, 0x2e `PbrPaintScratchColour`, 0x2f `PbrPaintScratchSettings`, 0x30 / 0x31 secondary diffuse / specular, 0x33 `gvDamageVolumeParam`, 0x34 `g_damageConstants`, 0x35 `g_selfIlluminationMask`, 0x37 `g_PerVehicleFog`, 0x49 `g_SelfIllumLocal_RecipRadiusSq`, 0x9c `GenericVector4_A`, B / C follow). They are per-object constants, written through `FUN_00a621a0(slot)` in `FUN_008e9c00` from a render record (stride 0x2b0) and its paint block `P` (record word 0xa2 = the render instance + 0x20).

**The paint block** (read in a memory snapshot of the running game, `re/out/live/snap1`, render instance 0 = `*(*(0x10edae4) + 4) + 0x3050`, instances 0x1c40 apart, and matched field by field to the car's `VehiclePaint` 0x163210): `P+0x10` = `PbrDiffuseColour`.rgb with `PbrSpecularRoughness` in w, `+0x20` = `PbrSpecularColour`.rgb with `PbrSpecularFresnel` in w, `+0x30` = `PbrScratchColour`, `+0x40` / `+0x50` = the secondary paint's, **`+0x60` = `DustColour`, `+0x70` = `DirtColour`** (corrected: the 2026-10-06 note read them the other way round without checking), `+0x80..0x88` = (`PbrSpecularRoughness`, `42e05e51`, `PbrSpecularFresnel`), `+0x8c` = `PbrScratchRoughness`, `+0x90` = `0802d981`, `+0x94` = `PbrClearcoatRoughness`, `+0x98` = `a1d177bd`, `+0x9c` = `PbrClearcoatFresnel`. So 0x2a / 0x2b = primary diffuse / specular, 0x2e = scratch colour, 0x2f = (`PbrScratchRoughness`, `0802d981`) = the scratch's roughness and F0, 0x30 / 0x31 = secondary − primary, **0x9d (`GenericVector4_B`) = DustColour, 0x9e (`_C`) = DirtColour**, 0x9c = (DustColour.w × record word 0x28, DirtColour.w × word 0x29, word 0x2a, word 0x2b).

**The record's levels** (`FUN_00583290`, which builds the records from the render instances): word 0x28 = instance word 0x6f6 × 2 (`.rdata` 0x00fac280), word 0x29 = instance word 0x6f4 × 3 (0x00fabaf8), word 0x2a = a global from `FUN_00897bc0` (inferred the rain-drop amount: the main paint program scales its `GenericSamplerB` drop normals by `GenericVector4_A.z`), word 0x2b = 1 / the function's float parameter (the drops' UV scale, `A.w`); words 0x24..0x27 (`g_damageConstants`) = 0. Each frame `FUN_00591ac0` copies the instance's levels from the vehicle: word 0x6f6 = vehicle +0x2c80 (**dust**), word 0x6f4 = vehicle +0x2c84 (**dirt**).

**Build-up** (`FUN_009d19c0`, `this` = vehicle +0x2c80; called each physics tick from `FUN_00904da0` → `FUN_009d6e30`, before the skid-sound values `FUN_009d1bf0`; earlier read here as the surface rumble). The writers were found by scanning the snapshot's decrypted `.text` for the displacement 0x2c80 (`lea ecx, [reg+0x2c80]` before calls at 0x008fd7df, 0x008fdf99, 0x00905059): the Ghidra dump shows the sub-object only through `this`. For each of the four wheels (vehicle +0x890 + i × 0x1d0: surface id = bits 13–19, on-ground byte at +0x894), when the surface's current weather block has a `VehicleEffects` object (`FUN_0054a840`: the `Wet_Conditions` block when the surface manager's state +0x10 ≥ 1 and the render instance's byte +0x1c17 is clear, else `Dry_Conditions`; formats.md "SurfaceVehicleEffects"):
- t = clamp((|v| − `f8bbba83`) / max(`0d9ece32` − `f8bbba83`, 2⁻²³)), v = vehicle +0x1620 (mph), so 30 → 120 mph on most surfaces;
- dirt (+0x2c84) += max(w₀, w₁, w₂, t) × dt / max(`Dirt.0979a504`, dt) × `Dirt.ea1c1262` / 4, clamped to 0..1, with w the wheel's three values at this +0x10 + i × 0x30 (smoothed per-wheel skid ramps written by `FUN_009cef90` from a tyre-sound-like parameter object, not ported);
- dust (+0x2c80) += t × dt / max(`Dust.0979a504`, dt) × `Dust.ea1c1262` / 4, clamped to 0..1.
A negative rate washes the car (surfaces 27 and 43: −0.5, 15–60 mph). At 120 mph on grass (0.3, 5 s) each level gains 0.06 / s; on dry asphalt only dust grows (0.05 → 0.01 / s at full speed). `FUN_009ce7b0` (car spawn) starts both at the render instance's words 0x1bd8 / 0x1bd0 (zeroed by `FUN_00586810`).
- **Set and wash**: message 0x13 of the physics queue (`FUN_00919240`) sets +0x2c80..+0x2c8c to one value (sent by `FUN_00589770`, which also sets the instance's levels; no direct caller), message 0x12 zeroes them. `FUN_0058f060` (a vtable method: zeroes the 15 damage-grid values, `FUN_009b2430(1)`, posts 0x12 and 0x11) is the repair; the body shops reach it through vehicle-manager message 0x16 (`FUN_00596880` case 0x16; their handler `FUN_00748ff0` posts 0xc = next palette colour and 0x16; "Body shops", 2026-10-10).

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
  Fixed awards through `FUN_00658ad0`: 100 at a gas station (`FUN_00748ff0`, an object of type 6 in the car's
  contacts) and `DAT_00fa929c` per item smashed in the Smash-and-Grab game mode (`FUN_0074cf40`; corrected
  2026-10-10: this is that mode's scoring behaviour, not free-roam security gates, see "Speed Points").
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

## Vehicle damage pose (2026-10-09)

How the eight damage clips (and the skeleton) become the body's bone matrices; ported in
`mw_assets::damage::VehicleDamage::pose_with` (status per item, from a sub-agent's trace of the dump).

- **Per frame** (`FUN_00580200`, per race-car slot, confirmed): the pose is fetched through the entity's vtable
  +0x40, given its skeleton (`FUN_009ecfd0`) and reset (`FUN_009ecfe0`: translation 0, scale 1, rotation identity),
  so additive clips never accumulate over frames. The animation modules run (through `FUN_005a0480`), then
  `FUN_009f4560` turns the local pose into model space (when the pose's +0x7ac flag is clear) and `FUN_0059fa70`
  writes the skinning palette (0x30 entries).
- **The damage update** (`FUN_005b62f0`, confirmed; reached through a vtable, inferred from the chain): clip i takes
  the i-th zone level of the race-car entity (+0xbd0, +0xbe0, +0xbe8, +0xbd8, +0xbcc, +0xbd4, +0xbe4, +0xbdc; with 9
  or more clips every clip uses the global value +0xbb4 × +0xbbc). Each level is capped at **0.99** while entity
  +0xc20 is below data +0x74 (read as: used damage-bar segments below the profile's count, i.e. not wrecked;
  `DamageBarProfile+0x74` is the wreck count, see "Crashes"), and multiplied by 1 − clamp((d − 40) k, 0, 1) with the
  camera distance d (k at `.data` 0x0110d53c, not read); off screen or beyond 50 m every level is 0. A clip with
  |level| ≤ `FLT_EPSILON` is skipped. Otherwise p = min(⌊100 level + 0.5⌋, 100), time = p × 0.01 × the clip's end
  time (controller +0x20; that this is the last frame's time is inferred, its set-up was not found), snapped down
  to a whole frame (`FUN_009ece70`: frame = ⌊time / frame duration⌋ mod frames), then the clip is applied with
  weight (1, 1, 1, 1) by `FUN_009f3250`. Clips go in index order 0..7.
- **The additive blend** (`FUN_009f3250`, confirmed): walks the bones through the skeleton's child / peer links
  (root first; the root's translation comes from the root track +0x38 and is always applied). Per bone, w = bone
  mask (Anim +0x44) × weight, skipped when w ≤ 0; each channel only when the clip animates it (remap byte not
  0xFF; a channel the clip does not animate keeps its value). With f the sampled frame, r the reference frame
  (Anim +0x36, decoded by `FUN_009f9d60` alongside f0 / f1; 0 in every vehicle clip):
  t += w (T[f] − T[r]); D = Q[f] ⊗ Q[r]⁻¹, q = slerp(q, D ⊗ q, w) (shorter arc, nlerp under 5°: cos > 0.9961947);
  s = lerp(s, S[f], w), absolute, so the last applied clip that scales a bone wins. So a key that is the same in
  every frame adds nothing (the XKR's boot lid, research log 2026-10-09).
- **Model space** (`FUN_009f4560`, confirmed): q = q_parent ⊗ q_local; t = t_parent + s_parent × rotate(q_parent,
  (bind_b − bind_parent) + t_local); s = s_parent × s_local. Bind positions only: the skeleton's bind rotations
  (AnimBone +0x10; e.g. the steering-wheel bones are tilted 18°) are never read.
- **Skinning palette** (`FUN_0059fa70`, confirmed): bones whose remap field (AnimBone +0x2C) is −1 are skipped; the
  others fill palette slot = remap, 0x40 bytes: 3 × 3 = R(q) × s (`FUN_00542240`), translation = t − bind_b · M
  (bind position only). `FUN_0057c5a0` builds attached-part matrices the same way.
- **Absolute blend**: `FUN_009f22a0` (t, q, s lerped / slerped towards the sample, no reference frame, decodes f0 /
  f1 through `FUN_009fa720`) is used by the race-car modules `FUN_005b0990`, `FUN_005b4510`, `FUN_005b3900`,
  `FUN_005b4750` on their own pose buffers (not identified).
- **Active aero: not identified.** Candidate `FUN_0047bd00`, a four-state clip player (0 time 0; 1 play forward to
  the end time at controller +0x20; 2 play back to the start at +0x1c; 3 time = end × a normalised parameter) that
  calls the additive `FUN_009f3250` with weight (1, 1, 1, 1). Ours runs part clips through the same blend after
  the damage (model decision until the player is confirmed).
- Not read: the distance-fade constant k, the update's rate limiter (`FUN_00897b70`, time accumulated at +0xe0)
  and what happens on frames it skips. Dead ends: `FUN_005bb770` and `FUN_00eaa990` apply procedural axis-angle
  rotations, not damage; `FUN_00a09c60` and its callers (`FUN_00a0b6d0` …) are collision (GJK), not pose code.

## Tyre marks and tyre smoke

**Found (2026-10-15).** Two paths, both per wheel (wheel object = vehicle +0x870 + 0x1d0 i; +0x20 its contact's
collision tag: surface = bits 13–19, bit 1 = the `ROAD` flag; +0x24 on the ground; +0x1c1 on rims, +0x1c2 flat).

- **The race car's effects** (`FUN_00589e20`, per car and frame, four wheels; `bVar37` = an AI car): the surface's
  `SurfaceVfx` (`FUN_0054aaa0`: `SurfaceType` weather block +0xc0, the wet block's when the surface manager's state
  is ≥ 1, the car's +0x1c17 is clear and it is set, else the dry one's) gives
  - the smoke: `AIWheelEffects` (+0x40) or `PlayerWheelEffects` (+0xa0) → `VfxWheelEffects`: `OnRims` (+0x1c)
    when the car's wheel state +0x1aa8 + 4i is 1, else `61ac1863` (+0x14) when `FUN_006a15c0(…, 0xb)` returns 2,
    else `DefaultTyre` (+0x0c). On while the wheel is down and the largest of its three smoke ramps (below) is
    above 0.01 (`.rdata` 0x00fad910), or on rims, or while the car moves on a surface whose byte +0x156 (front
    wheels) / +0x157 (rear) is set (false on every surface). The ramps are written into the effect's parameter
    block (+0x84..+0x8c, the `WheelParams` intensities). Four effect slots per wheel.
  - a second kick-up: `WheelVfx` (+0x138; AI `4cf3cf8f` +0x140) when a condition on the wheel's contact
    velocities holds (not read in full).
  - `bb02333c` (+0x100) for the local player's rear wheels, placed with the camera's matrix (lens dirt or
    similar; not ported); `5830a883` (+0x58) is copied to car +0x3c0.
  - the **tyre marks**: material = `RimMarksMaterial` (+0xe8) on rims, else `TyreMarksMaterials` by the car's
    tyre class (car +0x1a88: classes 0/1, 2, 3/4 read a short at +0x6c / +0x6e / +0x70 of a per-car object that
    picks the slot: 0 `Offroad` (+0x20), 1 `eff16a09` (+0x18), 2 `Road` (+0x28), 3 `Sport` (+0x30), 4 `Track`
    (+0x38)). A strip point is added **every update while the wheel is down and the material exists**
    (`FUN_004a3b80` starts a strip, `FUN_004a3c60` continues it, `FUN_004a3d10` ends it when the wheel lifts),
    with position = contact + `fStack_220` × the across vector, the wheel's +0x10 vector, Slip weight = the
    `Skid_Marks` lateral ramp (lane 5), Spin weight = max(spin, lock) (lanes 4, 7), wheel +0xbc (read as the
    tyre width; set per wheel by the vehicle build next to the normal / rim / flat radii +0xac / +0xcc / +0xec)
    and the material. The points go into a queue of 0x50-byte records (`FUN_004af040`); the vertex builder
    that consumes them was not found.
- **The ramps** (`FUN_009cef90`, `this` = vehicle +0x2c90 + 0x30 i, called four times per tick from
  `FUN_009d6e30` between the dirt build-up `FUN_009d19c0` and the skid sound `FUN_009d1bf0`). The record's +0x20
  points at an object whose +0x08 / +0x10 are the axle's `TyreVfxBehaviour` `Smoke` / `Skid_Marks` blocks (by
  which readers use lanes 0..2 and 4, 5, 7). For a block S (offsets inside `SkidMarkParams` / `SmokeParams`):
  s = |wheel +0xe4 − +0xe0| × 2.236936 (mph), v = |vehicle +0x1620| (mph), α = |wheel +0x114| (degrees);
  spin = clamp((s − S+0x5c) / max(S+0x58 − S+0x5c, 2⁻²³))^S+0x4c × S+0x54, zero while wheel +0x108 < −0.1;
  lateral = clamp((α − S+0x30) / (S+0x24 − S+0x30))^S+0x1c × clamp((v − S+0x34) / (S+0x28 − S+0x34)) × S+0x2c;
  lock = clamp((s − S+0x84) / (S+0x80 − S+0x84))^S+0x74 × S+0x7c, only while wheel +0x108 < −0.1 (`.data`
  0x011870d0, −0.1 in every lane). Each lane moves by rate × dt × 60 (0x01187200) of the gap to its target, the
  rate S+0x60 / +0x38 / +0x88 (`6274d1ad`) rising and S+0x50 / +0x20 / +0x78 (`27c5dd70`) falling. Offsets →
  fields: `Lat_Slip` at +0x0c (`Curve_Power` +0x1c, `27c5dd70` +0x20, `53b71e27` +0x24, `2c72e92a` +0x28,
  `35caad30` +0x2c, `6a83c966` +0x30, `26028ef5` +0x34, `6274d1ad` +0x38), `Long_Spin` at +0x3c (+0x4c … +0x60),
  `64222c3f` at +0x64 (+0x74 … +0x88). Ported as `mw_handling::tyre_vfx` (+0x108 taken as κ: inferred).
- **Other vehicles** (`FUN_00606210`, up to 431 vehicle slots; per wheel, also extra contacts beyond four): the
  surface's `SurfaceVfx` via `FUN_0054aaa0`; a kick-up `01d1b4fd` (+0x118) or `f967c1ce` (+0x120, by a byte
  +0x3d of the slot's object) into the shared `SurfaceFx` wheel-contact list (`FUN_00497450` → `FUN_0049ba00`,
  ≤ 256 contacts per frame) when wheel +0x1bf is set and the car's speed (max of +0x1fc0, +0x1fb8) > 0; tyre
  marks with `TyreMarksMaterials.Road` (+0x28) while wheel +0x1be is set, weight wheel +0x138. Those come from
  `FUN_009dfd40` (from `FUN_009e00c0`, the simplified wheel update of `FUN_008f7e10` / `FUN_008f8cd0` and three
  more): m = max(+0x130, clamp((+0x118 − 0.5) / 0.5) × 0.55, clamp((+0x134 − 0.1) / 0.9)), raised to at least
  max(+0x130, 0.5) on polygons without the `ROAD` flag; +0x1be = m > 0.1 and not on rims, +0x138 = m when above
  0.1 (constants: `.rdata` 0x01026830 / 0x01026b70 / 0x01026990 / 0x01026840 copied by static initialisers at
  0x00f772f0..0x00f77340). +0x134 = clamp(|wheel +0xdc|, 0, 15) / 15; +0x130 = clamp(|+0x110| − the axle tyre
  object's +0x4c, 0, 1), zero while the wheel is at rest (`FUN_009df090`); +0x118 = a curve
  (`TyreVfxBehaviour` `64222c3f` fields through the object at wheel +0x190) over |+0xd8| × 2.236936, times
  clamp((p − F) / (1 − F)) of an input vector p (`FUN_009df2e0` / `FUN_009d4f20`). +0xd8, +0xdc, +0x110 are
  written through vector lane setters that have no callers in the dump; not identified. Not ported.
- Dead ends: the earlier search (2026-10-06) looked for the field hashes and `TyreMarksMaterials` × 8 + 0x18;
  the code indexes the inline struct with a switch instead. The lane setters at 0x008f2c70..0x008f2fc0 and
  0x00908650..0x009087c0 have no callers or vtable entries in the snapshot.

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
  subtracting x, from 0.136); the look-at point follows about half of it (corrected in round 12:
  `FUN_005c7a80` adds cam + 0x550 × cam + 0x6a4, ~0.5, to cam + 0x5c4), so the view still tilts as the eye bobs.
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

**Deceleration pitch**: not read this round (round 12: in `FUN_005c7a80`, below).

### The followed frame, the composition, slopes and braking (round 12, 2026-10-10; ported)

Checked frame by frame with `mostrusted --camera-replay` / `tools/camcompare.py` (research log 2026-10-10: our
code over the recorded car motion of `ref/live1`, which holds three cars). `.bss` values come from the memory
snapshot `re/out/live/snap2` (`live.py -s snap2 <va>`), which contains the exe's data and bss.

- **The car frame is the wheels' plane.** `FUN_005dbe80` copies vehicle + 0x2db0..0x2dec, written each update by
  `FUN_0090a5f0`: forward = normalize((p1 − p2) + (p3 − p4)), row 0 = normalize((p3 − p1) + (p4 − p2)), up their
  cross product, of the four points at vehicle + 0x14..+0x20 (by the frame's behaviour the wheels: against the
  body it has 3° RMS less roll and none of its pitch under acceleration, correlation −0.56), the position the
  body's (model origin = body − R · centre of mass); 0x2e30 / 0x2e34 are the body's angles relative to it.
- **`FUN_005d4970`'s head**: cam + 0x4f4 (the stored forward's height) = `clamp(lerp(old, fwd.y, 0.12),
  fwd.y ± 0.3)` per update (`_DAT_00fb0a20`, `_DAT_00fafb8c`; replay 1.6e−4). The followed frame (fStack_190 →
  `FUN_005c8420` / `FUN_005c82c0` → cam + 0x510) is rebuilt from that forward and the world's up (0, 1, 0)
  (`_DAT_00fb05c0`): no roll, so cam + 0x598 stays ~0. Its Euler angles are the target of round 6's smoothing.
- **`FUN_005c2130`, the composition** (args: target angles, lag angles, lag for the cosine, S, the bob vector
  of `FUN_005c8880`, the followed frame, the output): a look-at matrix from cam + 0x5a0 (eye) to + 0x5c0, rolled by
  cam + 0x668 (= 0x660, the roll); its position += `|cos(yaw lag)|` × cam + 0x580..0x58c (the range-reduced
  polynomial at `01075470..` is sin(2π(|round(u) − u − 0.5| − 0.25)) with u = −x/2π: a cosine) and −= the pivot
  cam + 0x5b0 (state + 0x50); turned by the lag angles then the target angles; += `|cos lag|` × pivot z × the
  frame's forward, += the bob in the frame; **forward = the frame × look-at (cam + 0x5c0) − that eye**, i.e. the
  look-at point rides in the followed frame (current yaw), not the smoothed one; rows 0 / 1 rebuilt from the
  turned up. Rebuilt from the recorded fields: eye 1–2 cm, view 0.15° (the high-speed shake) / 0.035° RMS.
- **`FUN_005c7a80`** (args dt, k60; runs right after cam + 0x5a0 / 0x5b0 are reset from state + 0x40 / + 0x50):
  - *Deceleration pitch*, cam + 0x670: with `Deceleration_pitch_change` at instance + 0xe8: + 0xf8 maximum (12),
    + 0xfc `Maximum_Pitch_Change` (4.5°), + 0x100 minimum (2), + 0x104 curve power (2.25), + 0x108 intensity (2).
    While the frame's forward is within 10° of the velocity (`.bss` 0x0110e4e0 = 0.9848) and â·v̂ < 0, target =
    `powf(clamp((|a|·|â·v̂| − min) / (max − min)), power) × k60 × max° × π/180`, a in m/s² (`FUN_005d9610`), else
    0; cam + 0x670 += `min(step, max(−step, gap))`, step = dt × intensity × gap. Replay 0.0003 rad RMS.
  - *Gradient*: s = the frame's Euler pitch (nose down +) clamped to ±`Maximum_Gradient` (`Gradient_Adjustments`,
    handle + 0x178: `Downwards` at + 0x0c (floats + 0x1c `5b1750df` 0.5, + 0x20 `47a50f4c` 1.5), `Upwards` at
    + 0x24 (+ 0x34 0.5, + 0x38 4), `Maximum_Gradient` + 0x40 10°). cam + 0x680 → (s < 0 ? + 0x34 : + 0x1c) × s,
    cam + 0x690 → (s < 0 ? + 0x38 : + 0x20) × s, each by `clamp(0.1 k60) × k60` per update, 0 while airborne.
    Replay 0.0007 rad / 0.005 m.
  - *Look-at bob share* cam + 0x6a0: target 0.5 (`_DAT_00fb05e0`), 0.4 airborne (`_DAT_00fb0850`), `0.4 + 2.6 u k60`
    (`_DAT_00fb0840`) while the landing flag 0x716 is set and the spring (0x550, 0x554) is below zero and falling,
    u = `clamp((cam[+0x6b0] − 10) × 0.00204)`; rate 0.05 (`.bss` 0x0110e790) or `lerp(0.05, 1 (0x0110e550), u)`
    in that case, × k60. Recorded mean 0.507.
  - *Results*: cam + 0x5c0 = eye + Rx(0x670 − 0x680) × (state + 0x60 − eye) (positive = looking down); cam + 0x5c4
    += cam + 0x550 × 0x6a4 (the previous update's spring: the look-at point **does** follow half of the eye's
    spring, correcting round 11); cam + 0x5a4 −= cam + 0x694. Look-at height replay 0.03 m.
- **`FUN_005d12b0`'s z stage** with the `.bss` values (22.352, 0.75, 4, 0.01, 5): cam + 0x588 replays to 0.011 m
  RMS (0.028 m without the `g` term, vehicle + 0x2b20: a broadcast value in [−1, 1], 0 on 94 % of updates, not
  identified). The rate is `(g · k60 · (1 − w) + w) × clamp(gap, ±0.75)`; the airborne term (cam + 0x6c8) eases
  0.1 × k60 per update towards 1.85 while FUN_008f5350 says airborne and mph > 5.
- **FOV (`FUN_005c76a0`)**: the speed fraction goes through `powf(f, state + 0x80 / state + 0x7c)` (95 / 70; the
  call at 0x5c7721, its result dropped by the decompiler); with it the acceleration term's factor fits 1.00 per
  update (so `(+0x14 of FUN_0046ee10) × FUN_0046ee20()` is 1/60 at normal speed).
- **Yaw stiffness**: s of `FUN_005c90a0` = `clamp(|x| / 45)` (`.bss` 0x0110e8f0 = 45) with x the slip angle in
  degrees and m = `YawDriftSpring` (fit 0.0603 against the McLaren's 0.06; 0.00035 RMS). The standing value 0.09
  holds below ~1.0–1.4 m/s (switch not read).
- **`FUN_005c8e40`**: while airborne (air time > 0, no contact, `FUN_00911430` check) K_pitch eases to 0.1; the
  landing request uses lane 1 of `FUN_005d9610` (the vertical acceleration, m/s²) and fits the four recorded
  landings; the air-time height offset written to cam + 0x574 is overwritten by `FUN_005d12b0` (0x574 = 0x554)
  before any read.

### Avoidance: the original has none (round 7; settled 2026-10-14)

- Round 7 stopped at the director's update `0x005d5e20`: not a function start in the dump. It is one of the
  16,257 vtable-only functions the dump's analysis missed (reverse-engineering.md "Functions the dump is
  missing"); read with REA from the carved image (`tools/snapimage.py`, 232 bytes, vtable slot `0x00f99d1c`).
- **The whole per-frame camera path is now read, and none of it queries the world, a car's surroundings or
  props**:
  - the manager `FUN_005d2cd0`: makes sure `CAMERAS.BNDL` is loaded, updates the target history, calls director
    method +8, then clears the camera message queue (`DAT_010edb24`: count at +0x10, +0x20) and its flag bits.
    Nothing runs after the director.
  - the director update `0x005d5e20`: clears +0x3040, copies the active camera resource's settings into the
    director (`FUN_005cf040`: four values from the `CameraGameplayExternal` sub-blocks, splatted to +0x1c40..
    +0x1c7c, through `FUN_005c4810` / `005c4ea0` / `005c6f00` / `005cc940`), runs the message handler, handles
    slot 11 (a scripted camera whose blend `FUN_004962f0(0)` channel is copied, else `FUN_005cf160` /
    `FUN_005c3420`), then calls method +8 (the update) of all 15 sub-cameras at +0x2ff0 in order.
  - the message handler `FUN_005d5800`: walks the queued camera messages for this player (type at +0, player at
    +8) and switches on 24 types: select a camera (through the table `DAT_00fafb60`), set flags, scripted
    shots with blend matrices (`FUN_005d28b0`, `FUN_005bfcb0`, `FUN_005bf990`), shake / FOV / settings calls
    (`FUN_005d2d50`, `005d2e10`, `005d2ed0`, `005d40e0`, `005c46e0`, `005bfba0`, `005d4650`, `00a01f60`), a
    four-float setting at +0x1254, and method +0x24 on every sub-camera for type 16. No geometry.
  - the chase update `FUN_005d4970`: 43 direct callees, all read now; the five not covered before are an
    orthonormalising matrix builder (`FUN_0053fee0`), two state initialisers (`FUN_0041c8a0`: identity
    matrices, 90°, 0.15, 15000, 1; `FUN_005dd680`), a blend of two values by `CameraGameplayExternal` +0xa0
    (`FUN_006c5270`) and a lookup of a linked object in state 4 (`FUN_006c76b0`). Its one indirect call
    (0x005d4a29) is virtual method +0 of the followed object (`FUN_005734a0`), whose result is read as a
    transform.
- No `ProcessNode.CameraTransformCollider` / `EntityAvoider` instance exists in the data (below).
- **Result**: the chase camera does not avoid walls; the eye passes through them. Ours did a push-in along
  the boom (a model decision); it is now off, `MW_CAMERA_PUSH_IN=1` for A/B. Not checked: a render-side step
  (near plane only, by `FUN_0041c8a0`'s 0.15). A capture of the original backed into a wall would confirm.

### Other leads checked in round 5

- `CameraGameplayExternalGlobals` holds only `Impact_shake` (96 bytes), `LensProperties` (28) and a
  `CameraGameplayShakeEffect`: no spring or avoidance settings; `CameraRearViewGlobals` is four floats. The
  avoidance's tuning is therefore code constants or `CameraGameplayExternal` fields, not the globals.
- No `ProcessNode.Camera*` instance exists in any bundle (`mwtool genesys all --type CameraTransformEntityAvoider`
  and `--type ProcessNode` return nothing; `CAMERAS.BNDL` holds a `CameraTakeList` and `CameraTake`s, the
  scripted shots). The chase camera is not a node graph; the node types are unused by the shipped data.
- The script getters (`LAB_005dae70` / `LAB_005daeb0`) were not needed once the director was found.

### Next steps for the port

The chase update is read and ported apart from: `FUN_005c8880`'s object (the bob; 7 mm, left out), vehicle +
0x2b20 (the z rate's `g`), the standing yaw-stiffness switch (car + 0x3c in `FUN_005c89c0`), `FUN_005c8420` /
`FUN_005c82c0` (they modify the followed frame before cam + 0x510; the recorded 0x510 equals the world-up
frame of cam + 0x4d0 to 0.07° yaw, so their effect is small) and the four points of `FUN_0090a5f0`. Compare
any change with `mostrusted --camera-replay` / `tools/camcompare.py` (research log 2026-10-10). In the running
game the director is at `M + 0x6a0` with `M = u32 at (u32 at 0x010edaf4) + 4`, the chase camera at + 0x8d0, its
state pointer at + 0x290.

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
