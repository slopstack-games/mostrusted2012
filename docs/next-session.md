# Handoff: state at the end of the 2026-10-04 session

Read this first when you pick the work up again, then delete or trim it once it's done. Nothing from this session
is committed yet (`git status` shows it all). Scratch files from the session (traces, wavs, dumps) were in the
session scratchpad and are gone; regenerate them with the commands below.

## 1. Engine sound: rev-up is a staircase (next task, diagnosed, not fixed)

User report: "the engine audio still doesn't smoothly transition up the rev range".

**Cause (measured):** `GinsuSynth` (`crates/mw-audio/src/ginsu.rs`) plays every grain (one engine cycle = two
crank turns, 911: 3684 samples at 1575 rpm, 734 at 7814 rpm) at its *recorded* pitch, and only picks a new
grain when the current one ends. Under acceleration the target moves 10–20 grains per grain length, so pitch
rises in steps. On a 2000→7500 rpm sweep over 2.5 s, the synth's firing frequency (flat-six: rpm / 20 Hz) lags
by up to 118 cents (p90), with steps of up to 370 cents between 5 ms windows.

**Planned fix:** resample each grain continuously: playback rate = smoothed rpm / recorded rpm of the grain
(the recorded rpm comes from inverting `rpm_table` at the grain's centre, or from the grain length:
`120 × sample_rate / len` matched within 1% on the 911). Use a fractional read position with linear (or cubic)
interpolation, clamp the rate to about 0.7–1.4, and keep the cross-fade with the old grain's continuation and
the target/target+1 alternation. Then re-measure:

```sh
# 1. a dense sweep trace (lines: seconds rpm load), 64 Hz like the game
python3 -c "import numpy as np; print('\n'.join('%.4f %.1f 1'%(t,2000+5500*min(t,2.5)/2.5) for t in np.arange(0,3,1/64)))" > fast.tr
./target/release/mwtool engine-sound 911 fast.wav --trace fast.tr
# 2. pitch tracking: per 30 ms window, take the harmonic-sum f0 within ±25% of rpm/20 and compare it with the
#    expected value (median error in cents, p90 |error|, and the 5 ms step jitter after removing the expected slope)
```

Goal: p90 error well under 25 cents and no step above about 30 cents. Also check that steady holds don't get
worse (2-20 Hz envelope modulation at 2500/4500/6500 rpm; baseline 0.018 / 0.034 / 0.005 off load).

Already fixed this session (in the research log, "2026-10-04 (late night): jumpy engine sound"): the
on/off-load recording pairing (signed RPM tables), the clutch slip in every gear, coasting kick-downs, and the
equal-power on/off fade. Tools: `--test engine` (human-like pedal script), `MW_ENGINE_TRACE=<file>` (RPM every
physics tick), `mwtool engine-sound <car> out.wav --trace <file>`, and `mwtool audio <bundle>` now prints
whether each Ginsu sweep rises or falls.

## 2. Handling: make it work like the original (user's main request, in progress)

The user asked for the car physics to behave the way the original game's do. `NFS13.exe` is DRM-encrypted
(see `docs/reverse-engineering.md`), so this is done from the data. Done so far:

- `VehicleHandling::param("Drift.Yaw_torque.Maximum_yaw_torque")` reads any `HandlingBehaviour` field by
  dotted path; unnamed fields are written as their 8-digit hex hash (`"Drift.Drift_trigger.8e4032c0"`).
- `--test drift` (110 km/h, steer 0.8 + 0.3 s brake tap, then hold the steering on 0.6 throttle for 6 s). Baseline
  911: peak slip 10–14°, held beyond 10° for 0–1.3 s. The original holds a drift as long as you steer.
- `Car::roll` / `Car::pitch` (relative to the ground under the wheels); the corner test prints roll (2.6° at full
  steering, inside the data's `Roll_angle_limit` of 3.75°).
- The baseline tests vary between runs (0–100 km/h 3.93 vs 4.33 s, drift peak 10 vs 13.8°): the scripted input
  runs in `Update` at 4× time, so it isn't tick-exact. Move test scripting to `FixedUpdate` before calibrating.

**Field naming:** a background agent was cracking the unnamed handling fields with `mwtool genesys-crack`
(small topical vocabularies, accept only names that fit their siblings and values). Its accepted names, if
any, are in `crates/mw-formats/src/genesys_names.txt`; check `git diff` on that file. (The agent may still be running when the session ends.) Confirmed by hand:
`e324fa82` = `Mid_yaw_torque` (10625, between Minimum 1875 and Maximum 18125). Plausible but unconfirmed:
HandbrakeAbility `8c74b0f6` `Low_speed_mph` (30), `92a9eddb` `High_speed_mph` (135), `df620dba`
`Minimum_handbrake_time` (0.1); Gas_effect `c8a6d616` `Maximum_effect`, `ada4cf87` `Minimum_gas_rate`.

**Survey across all 197 HandlingBehaviour objects** (re-create with `mwtool genesys all --type
HandlingBehaviour`; four outliers with `Maximum_drift_angle` 180 and trigger values 100 / 90 look like drift
disabled, probably traffic or cop entries):
- Drift.Yaw_torque: Minimum (0–2190) / `e324fa82` Mid (1875–14375) / Maximum (3700–20000) Nm, plus
  `70528afa` (0–0.5), `a5c9496f` (0.35–0.8) and `2ef426dd` (−0.1 on every car). Hypothesis: a curve of yaw torque
  against steering input with points at about −0.1 (min), `70528afa`, `a5c9496f` (mid) and 1.0 (max).
  `Counter_steer_yaw_torque`, `Initial_yaw_torque` and `Torque_decay` are 0 everywhere.
  `Donut_yaw_torque` 6000–60000 between `Donut_speed_limit_low` / `_high` (mph).
- Drift.Other: `Default_steering` 0.2–0.4, `Drift_angular_damping` 0.015–0.075, `Max_counter_steering_power`
  1–2.5, `Side_force_multiplier` 3–6, `Maximum_drift_angle` 90; unnamed `fe68bc4b` 40/45, `0e2a4c75` 3–5,
  `35e937b8` 10–15, `fb0656a3` 15/18, `ac261613` −0.1 to −0.7, `939b8fdc` −1/−0.8.
- Drift.Side_force: `Decay_rate` 0.012–0.022, `Drift_angle_for_decay` 4–18°; unnamed `3ca074df` 0.5–2,
  `f6c50d59` 2.5–16, `09832a93` 10–15.5, `9e6f2943` 75–100, `aadfc7b1` 0.15/0.3, `ff5dcf85` ~0.025.
- Drift.Drift_scale: `_from_braking` 0.45–1, `_from_steering` 1.2–2, `_decay` 0.5–0.6; unnamed `c1bc8c79` 5–7,
  `005c9352` 0.1–1.65, `ce8870ad` 0.3–0.8.
- Drift.Drift_trigger: `8e4032c0` 5.5–12 (per car), `8f151151` 4.5–12, `b5290695` 10, `0dce38f7` 2;
  Drift_exit: `4971b7ae` 2–5, `Time_to_blend_damping` 0.25.
- Body_movement: `Roll_angle_limit` 2.75–7°, `Front/Rear_roll_damping` ~1.2; unnamed `ddbd44c0`
  20k–525k and `b80e5de3` 20k–300k (stiffnesses?), `782f05c1` / `2bb0f6a0` 100–3750 (dampers?).
- Tyres: `Available_grip` (20000–50000) does **not** scale with mass (the 8000 kg truck has 28000, as the
  911 does): an absolute force budget per tyre class, not a coefficient. `Grip_factor` = `Sliding_grip_factor`
  on every car (2.7–10).
- Aero: `Down_force` 0/30, `Linear_drag` 0/0.9/2, `Side_drag_coefficient` 2, `Yaw_drag_coefficient` 0/3, three
  speed-like unnamed fields (`f1538c51`, `6fa7fa07`, `4aadb133`, 80–270), not equal to `Top_speed_MPH`.

**Plan:** implement a drift state in a new `crates/mostrusted/src/drift.rs`. Drift scale builds from brake and
steering inputs and decays. A trigger fires on scale, speed and slip angle. While drifting: yaw torque from the
steering curve (min/mid/max), angular damping, a side force that keeps the car on its line and decays below
`Drift_angle_for_decay`, the `Maximum_drift_angle` limit, and an exit with blended damping. Then add the roll
limit, handbrake low/high speed, and donuts. Calibrate with `--test drift` / `handbrake` / `corner`, and record
every interpretation in `docs/handling.md` (data vs model decision).

## 3. World layers (M1.3): done, with loose ends

- Props, compounds and dynamic objects: 56,499 placements on the full map, up from 35,431 (checked visually).
- Groundcover: `crates/mw-formats/src/groundcover.rs`. There is no mesh; the game's shader turns 16-byte instances into
  sprites. 1,364,853 instances parse. They are drawn as one merged mesh per collection and material, and
  `--no-groundcover` skips them. Load time is 34.4 s with them and 31.7 s without.
- Clippy is back to 0 warnings and tests pass. Layouts are in `docs/formats.md`.
- Loose ends:
  - Grass tufts are very dark (the tint is used as albedo × 2, not tuned).
  - Sprite size and yaw bytes are guesses.
  - There is no wind, billboarding or fade distance.
  - Little grass shows at the roadside viewpoints (density is low there).
  - An `#[ignore]`d diagnostic test `diag_gc_ground` remains in `crates/mw-assets/tests/world_layers.rs`.
  - Lights are parsed but not drawn.
  - The stray leaf-card quads in tree canopies **predate** this work (a tree shader or alpha issue).

## Before committing

`cargo fmt`, `cargo clippy --release --workspace` (0 warnings), `cargo test --release --workspace`,
`./target/release/mwtool verify` (0 failures). Never commit game data or derived files.
