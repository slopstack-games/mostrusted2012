//! The game's drift model, driven by each car's `HandlingBehaviour.Drift` (`DriftBehaviour`) data.
//!
//! NFS Most Wanted (2012) does not leave drifting to the tyres alone. A drift state starts when the rear tyres
//! slide past an entry angle (lowered for a moment by a brake tap). In it a signed *drift scale* integrates the
//! driver's throttle, steering and brake; it sets a yaw torque, a side force towards the inside of the drift
//! and a "speed hold" that pays back the speed the slide scrubs, while the yaw rate and sideways velocity are
//! damped. The rules below follow the game's own drift code (docs/handling.md "Drift" lists each field's role
//! and what is still unknown).
//!
//! Sign convention: angles and the drift direction are positive to the left (the car's +X axis), as are the
//! steering input and the yaw rate in `drive.rs`. A drift to the left has the nose left of the velocity.

use mw_assets::handling::VehicleHandling;

const MPH: f32 = 0.44704;
/// Time base of the per-tick damping fractions (`Drift_angular_damping`, `ff5dcf85`): the game applies them
/// as `(1 - d)^(dt * K)`; K is not in the data, and the game ticks at 60 Hz.
const DATA_HZ: f32 = 60.0;
/// Brake tap: a press above `TAP_HIGH` released below `TAP_LOW` within `TAP_WINDOW` seconds (game constants).
const TAP_WINDOW: f32 = 0.6;
const TAP_HIGH: f32 = 0.25;
const TAP_LOW: f32 = 0.1;
/// A drift lasts at least this long before the exit angle can end it (s, game constant).
const MIN_DRIFT_TIME: f32 = 0.5;
/// Forced exits: airborne this long (s), or slower than this (m/s) (game constants).
const MAX_AIR_TIME: f32 = 1.0;
const MIN_DRIFT_SPEED: f32 = 0.5;
/// No drift entry during a burnout: `(1 - mph / BURNOUT_MPH) * throttle * brake` must stay below 0.1. The
/// game's speed constant is not in the data (model value).
const BURNOUT_MPH: f32 = 20.0;
/// Throttle and brake gates of the speed hold (game constants).
const HOLD_MIN_THROTTLE: f32 = 0.3;
const HOLD_MAX_BRAKE: f32 = 0.1;
// The game caps the side-force factor at 5 (`.bss` 0x01187380, read from the running game).
/// The cap our model uses until the side force is measured in the original: with the game's cap the force
/// reaches ~3 g, so our reading of its size or direction is off somewhere (research log, session 3).
const SIDE_FORCE_CAP_MODEL: f32 = 1.0;
/// The side-force boost decays at this rate (per second) and saturates at this yaw rate (rad/s).
const BOOST_DECAY: f32 = 2.0;
const BOOST_YAW_RATE: f32 = 2.0;

/// `DriftBehaviour` values for one car (plus the handbrake's drift-scale pair).
#[derive(Debug, Clone)]
pub struct DriftParams {
    /// `Drift_trigger.Slip_angle_to_enter_drift` (degrees of mean rear-tyre slip).
    pub enter_slip: f32,
    /// `Drift_trigger.0dce38f7`: the entry slip right after a brake tap (degrees).
    pub enter_slip_tap: f32,
    /// `Drift_trigger.8f151151`: the entry slip at full brake (blended by the pedal).
    pub enter_slip_brake: f32,
    /// `Drift_trigger.b5290695`: how fast the entry slip climbs back after a tap (degrees per second).
    pub enter_recovery: f32,
    /// `Drift_trigger.Minimum_speed_to_enter_drift` (mph, forwards).
    pub enter_min_speed: f32,
    /// `Drift_exit.Drift_angle_to_exit_drift` (degrees of mean rear-tyre slip).
    pub exit_angle: f32,
    /// `Drift_exit.Time_to_blend_damping` (s): the yaw damping fades out over this time after an exit.
    pub blend_time: f32,
    /// `Drift_scale.*` rates (per second): `_from_steering` / `_from_counter_steering` (× steering × throttle),
    /// `_from_braking` (× brake), `ce8870ad` (× throttle × body slip up to `c1bc8c79` degrees), `8a2a92f3`
    /// (× throttle increase), `_decay` (constant).
    pub scale_from_steering: f32,
    pub scale_from_counter: f32,
    pub scale_from_braking: f32,
    pub scale_from_angle: f32,
    pub scale_angle: f32,
    pub scale_from_throttle_rise: f32,
    pub scale_decay: f32,
    /// `Drift_scale.e6abd30f`: the scale a drift starts with (without the handbrake).
    pub scale_initial: f32,
    /// `HandbrakeAbility` `6ce0fda7` / `35e18207` at `Low_speed_mph` / `High_speed_mph`: the scale a handbrake
    /// drift starts with, and its rate (per second) while the handbrake is held.
    pub handbrake_scale: [f32; 2],
    pub handbrake_speeds: [f32; 2],
    /// `Yaw_torque.Minimum_` / `Mid_` / `Maximum_yaw_torque` (N m): control points of a quadratic Bézier in the
    /// drift scale. Below `Donut_speed_limit_low` the top point is `Donut_yaw_torque`, blending to the maximum
    /// by `Donut_speed_limit_high` (mph).
    pub yaw_min: f32,
    pub yaw_mid: f32,
    pub yaw_max: f32,
    pub donut_torque: f32,
    pub donut_speeds: [f32; 2],
    /// Steering factor on the torque: `a5c9496f` with no steering, 1 at full lock into the drift, `70528afa` at
    /// full counter-lock, plus `2ef426dd` × throttle.
    pub yaw_no_steer: f32,
    pub yaw_counter_steer: f32,
    pub yaw_per_throttle: f32,
    /// `Yaw_torque.Initial_yaw_torque` (N m × brake, against the drift; 0 on every car).
    pub yaw_initial: f32,
    /// `Other.Drift_angular_damping`: fraction of the yaw rate removed per 60 Hz tick, × drift scale.
    pub angular_damping: f32,
    /// `Side_force.ff5dcf85`: fraction of the sideways velocity removed per 60 Hz tick.
    pub lateral_damping: f32,
    /// `Side_force.09832a93` (m/s²): side acceleration at full side force.
    pub side_accel: f32,
    /// The side force grows from `Drift_angle_for_side_force` to `Minimum_drift_angle_for_side_force` (degrees
    /// of body slip).
    pub side_angles: [f32; 2],
    /// `Side_force.Decay_rate` per degree above `Drift_angle_for_decay`, while steering into the drift; the
    /// reduction is capped at `1 - c3db1a2a / 09832a93`.
    pub side_decay_rate: f32,
    pub side_decay_angle: f32,
    pub side_decay_cap: f32,
    /// `Side_force.Drift_scale_for_maximum_side_force`.
    pub side_full_scale: f32,
    /// The side force grows from `d79e7ebc` to `Speed_for_maximum_side_force` (mph).
    pub side_speeds: [f32; 2],
    /// `Other.Side_force_multiplier`: gain of the yaw-rate boost on the side force.
    pub side_boost: f32,
    /// Speed hold: the target speed may run at most `35e937b8` mph ahead of the car; `55930ac1` is the fraction
    /// of the shortfall made up per 60 Hz tick (along the travel direction).
    pub hold_lead: f32,
    pub hold_gain: f32,
    /// In-drift steering (the game remaps the steering input while drifting): `Default_steering` with no input,
    /// counter-steer limited to `ac261613` early in a drift (`fb0656a3` degrees of peak slip makes it count),
    /// opening to `939b8fdc` between `Low_slip_angle_for_counter_steering` and `fe68bc4b` degrees.
    pub steer_default: f32,
    pub steer_counter_early: f32,
    pub steer_counter_late: f32,
    pub steer_peak_angle: f32,
    pub steer_counter_angles: [f32; 2],
    /// `Steering.Counter_steer_rate_in_drift` (degrees per second), reached at `Steering.5277ec9e` degrees.
    pub counter_steer_rate: f32,
    pub counter_steer_rate_angle: f32,
}

impl DriftParams {
    /// `None` when the car has no drift data or the data disables drifting (the two Police Interceptors have
    /// entry angles of 90° and `Maximum_drift_angle` 180).
    pub fn new(h: &VehicleHandling) -> Option<Self> {
        let d = |path: &str| h.param(&format!("Drift.{path}"));
        let enter_slip = d("Drift_trigger.Slip_angle_to_enter_drift")?;
        if enter_slip >= 89.0 || d("Other.Maximum_drift_angle").is_some_and(|a| a >= 180.0) {
            return None;
        }
        let or = |path: &str, default: f32| d(path).unwrap_or(default);
        let hb = |path: &str, default: f32| h.param(&format!("Handbrake.{path}")).unwrap_or(default);
        let side_accel = or("Side_force.09832a93", 14.0);
        let donut_low = or("Yaw_torque.Donut_speed_limit_low", 5.0);
        let side_low = or("Side_force.Drift_angle_for_side_force", 1.5);
        let speed_low = or("Side_force.d79e7ebc", 0.0);
        Some(Self {
            enter_slip,
            enter_slip_tap: or("Drift_trigger.0dce38f7", 2.0),
            enter_slip_brake: or("Drift_trigger.8f151151", 5.0),
            enter_recovery: or("Drift_trigger.b5290695", 10.0),
            enter_min_speed: or("Drift_trigger.Minimum_speed_to_enter_drift", 0.0),
            exit_angle: or("Drift_exit.Drift_angle_to_exit_drift", 5.0),
            blend_time: or("Drift_exit.Time_to_blend_damping", 0.25),
            scale_from_steering: or("Drift_scale.Drift_scale_from_steering", 1.6),
            scale_from_counter: or("Drift_scale.Drift_scale_from_counter_steering", 1.0),
            scale_from_braking: or("Drift_scale.Drift_scale_from_braking", 1.0),
            scale_from_angle: or("Drift_scale.ce8870ad", 0.3),
            scale_angle: or("Drift_scale.c1bc8c79", 6.0).max(0.1),
            scale_from_throttle_rise: or("Drift_scale.8a2a92f3", 0.0),
            scale_decay: or("Drift_scale.Drift_scale_decay", 0.5),
            scale_initial: or("Drift_scale.e6abd30f", 1e-4),
            handbrake_scale: [hb("6ce0fda7", 0.6), hb("35e18207", 0.4)],
            handbrake_speeds: [hb("Low_speed_mph", 30.0), hb("High_speed_mph", 135.0)],
            yaw_min: or("Yaw_torque.Minimum_yaw_torque", 1500.0),
            yaw_mid: or("Yaw_torque.Mid_yaw_torque", 9000.0),
            yaw_max: or("Yaw_torque.Maximum_yaw_torque", 15000.0),
            donut_torque: or("Yaw_torque.Donut_yaw_torque", 15000.0),
            donut_speeds: [donut_low, or("Yaw_torque.Donut_speed_limit_high", 30.0).max(donut_low + 0.1)],
            yaw_no_steer: or("Yaw_torque.a5c9496f", 0.5),
            yaw_counter_steer: or("Yaw_torque.70528afa", 0.1),
            yaw_per_throttle: or("Yaw_torque.2ef426dd", -0.1),
            yaw_initial: or("Yaw_torque.Initial_yaw_torque", 0.0),
            angular_damping: or("Other.Drift_angular_damping", 0.05).clamp(0.0, 0.99),
            lateral_damping: or("Side_force.ff5dcf85", 0.025).clamp(0.0, 0.99),
            side_accel,
            side_angles: [side_low, or("Side_force.Minimum_drift_angle_for_side_force", 5.0).max(side_low + 0.1)],
            side_decay_rate: or("Side_force.Decay_rate", 0.018),
            side_decay_angle: or("Side_force.Drift_angle_for_decay", 6.5),
            side_decay_cap: (1.0 - or("Side_force.c3db1a2a", 0.0) / side_accel.max(1e-3)).clamp(0.0, 1.0),
            side_full_scale: or("Side_force.Drift_scale_for_maximum_side_force", 0.3).max(0.01),
            side_speeds: [speed_low, or("Side_force.Speed_for_maximum_side_force", 90.0).max(speed_low + 1.0)],
            side_boost: or("Other.Side_force_multiplier", 5.0),
            hold_lead: or("Other.35e937b8", 15.0),
            hold_gain: or("Other.55930ac1", 0.05),
            steer_default: or("Other.Default_steering", 0.24),
            steer_counter_early: or("Other.ac261613", -0.15),
            steer_counter_late: or("Other.939b8fdc", -1.0),
            steer_peak_angle: or("Other.fb0656a3", 18.0).max(0.1),
            steer_counter_angles: [
                or("Other.Low_slip_angle_for_counter_steering", 5.0),
                or("Other.fe68bc4b", 40.0).max(or("Other.Low_slip_angle_for_counter_steering", 5.0) + 0.1),
            ],
            counter_steer_rate: h.param("Steering.Counter_steer_rate_in_drift").unwrap_or(120.0),
            counter_steer_rate_angle: h.param("Steering.5277ec9e").unwrap_or(40.0).max(0.1),
        })
    }

    /// The handbrake's drift scale (initial value, and rate while held) at `mph`.
    fn handbrake_scale(&self, mph: f32) -> f32 {
        let [lo, hi] = self.handbrake_speeds;
        let t = ((mph.abs() - lo) / (hi - lo).max(1.0)).clamp(0.0, 1.0);
        self.handbrake_scale[0] + (self.handbrake_scale[1] - self.handbrake_scale[0]) * t
    }

    /// Yaw torque magnitude (N m) at drift scale `s` (0..1) and `mph`, before the steering factor. A quadratic
    /// Bézier through the minimum, mid and top torques; the top is the donut torque at walking pace (not when
    /// counter-steering).
    pub fn yaw_torque(&self, s: f32, mph: f32, counter_steering: bool) -> f32 {
        let [lo, hi] = self.donut_speeds;
        let t = if counter_steering { 1.0 } else { ((mph - lo) / (hi - lo)).clamp(0.0, 1.0) };
        let top = self.donut_torque + (self.yaw_max - self.donut_torque) * t;
        let s = s.clamp(0.0, 1.0);
        (1.0 - s) * (1.0 - s) * self.yaw_min + 2.0 * s * (1.0 - s) * self.yaw_mid + s * s * top
    }
}

/// What the drift model needs from the car each physics tick.
pub struct DriftInput {
    /// Steering input, −1..1, positive = left, relative to the current steering range (full lock = ±1).
    pub steer: f32,
    pub throttle: f32,
    pub brake: f32,
    pub handbrake: bool,
    /// Ground speed (m/s) and the signed speed in mph (negative when reversing).
    pub speed: f32,
    pub mph: f32,
    /// Body slip (rad): heading relative to the velocity, positive = nose left.
    pub angle: f32,
    /// Mean slip angle of the rear tyres (degrees), positive = nose left of their travel.
    pub rear_slip: f32,
    /// Yaw rate (rad/s, positive = left).
    pub yaw_rate: f32,
    pub mass: f32,
    /// Every wheel on the ground / at least one.
    pub all_grounded: bool,
    pub any_grounded: bool,
    /// Seconds since the last wheel left the ground (0 while grounded).
    pub air_time: f32,
    pub reverse_gear: bool,
}

/// What the drift model does to the car this tick.
#[derive(Debug, Clone, Copy)]
pub struct DriftOutput {
    /// Torque about the car's up axis (N m, positive = left).
    pub yaw_torque: f32,
    /// Force along the car's +X axis (N, positive = left).
    pub side_force: f32,
    /// Force along the direction of travel (N).
    pub hold_force: f32,
    /// Multipliers for the yaw rate and the sideways (car X) velocity this tick.
    pub yaw_rate_scale: f32,
    pub lateral_scale: f32,
}

impl Default for DriftOutput {
    fn default() -> Self {
        Self { yaw_torque: 0.0, side_force: 0.0, hold_force: 0.0, yaw_rate_scale: 1.0, lateral_scale: 1.0 }
    }
}

/// Where the car is in a drift.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Off,
    Drifting,
}

/// Drift state carried between ticks.
#[derive(Debug, Clone)]
pub struct DriftState {
    pub phase: Phase,
    /// +1 drifting left (nose left of the velocity), −1 right.
    pub direction: f32,
    /// Drift scale (0..1, in `direction`).
    pub scale: f32,
    /// Current entry threshold (degrees of rear slip): drops after a brake tap and climbs back.
    threshold: f32,
    /// Brake-tap detector: time the brake has been down, and whether it went past the high mark.
    tap_time: f32,
    tap_high: bool,
    prev_throttle: f32,
    /// Time in the drift, and since the last exit (the damping fades out over `Time_to_blend_damping`).
    pub time: f32,
    pub since_exit: f32,
    /// Peak yaw damping of the drift (kept after the exit for the fade-out).
    peak_damping: f32,
    /// Peak rear slip in this drift (degrees).
    pub peak_slip: f32,
    /// Speed the drift tries to hold (m/s).
    target_speed: f32,
    /// Side-force boost from the yaw rate.
    boost: f32,
    /// The last tick's output (diagnostics).
    pub last: DriftOutput,
}

impl Default for DriftState {
    fn default() -> Self {
        Self {
            phase: Phase::Off,
            direction: 1.0,
            scale: 0.0,
            threshold: f32::MAX,
            tap_time: 0.0,
            tap_high: false,
            prev_throttle: 0.0,
            time: 0.0,
            since_exit: f32::MAX,
            peak_damping: 0.0,
            peak_slip: 0.0,
            target_speed: 0.0,
            boost: 0.0,
            last: DriftOutput::default(),
        }
    }
}

impl DriftState {
    pub fn active(&self) -> bool {
        self.phase == Phase::Drifting
    }

    pub fn step(&mut self, p: &DriftParams, i: &DriftInput, dt: f32) -> DriftOutput {
        self.threshold = self.threshold.min(p.enter_slip);
        let tap = self.brake_tap(i.brake, dt);
        match self.phase {
            Phase::Off => self.try_enter(p, i, tap, dt),
            Phase::Drifting => self.check_exit(p, i),
        }

        let mut out = DriftOutput::default();
        if self.active() {
            self.time += dt;
            self.since_exit = 0.0;
            self.peak_slip = self.peak_slip.max(i.rear_slip.abs());
            if i.all_grounded {
                out = self.forces(p, i, dt);
            }
        } else {
            self.since_exit += dt;
            self.peak_slip = 0.0;
        }
        // Damping: the yaw rate by the drift's peak damping (fading out after an exit), the sideways velocity
        // in full while the fade lasts.
        let blend = (1.0 - self.since_exit / p.blend_time.max(1e-3)).clamp(0.0, 1.0);
        if blend > 0.0 {
            self.peak_damping = self.peak_damping.max(self.scale * p.angular_damping);
            let ticks = dt * DATA_HZ;
            out.yaw_rate_scale = (1.0 - blend * self.peak_damping).max(0.0).powf(ticks);
            out.lateral_scale = (1.0 - p.lateral_damping).powf(ticks);
        }
        self.prev_throttle = i.throttle;
        self.last = out;
        out
    }

    /// True on the tick a short, firm brake press is released.
    fn brake_tap(&mut self, brake: f32, dt: f32) -> bool {
        if brake >= TAP_LOW {
            self.tap_time += dt;
            self.tap_high |= brake > TAP_HIGH;
            return false;
        }
        let tap = self.tap_high && self.tap_time > 0.0 && self.tap_time < TAP_WINDOW;
        self.tap_time = 0.0;
        self.tap_high = false;
        tap
    }

    fn try_enter(&mut self, p: &DriftParams, i: &DriftInput, tap: bool, dt: f32) {
        let burnout = (1.0 - i.mph.abs() / BURNOUT_MPH).max(0.0) * i.throttle * i.brake;
        if !i.all_grounded || i.mph <= p.enter_min_speed || burnout >= 0.1 {
            return;
        }
        if tap {
            self.threshold = self.threshold.min(p.enter_slip_tap);
        }
        let threshold = self.threshold + (p.enter_slip_brake - self.threshold) * i.brake.clamp(0.0, 1.0);
        if i.rear_slip.abs() > threshold {
            self.phase = Phase::Drifting;
            self.direction = i.rear_slip.signum();
            self.scale = if i.handbrake { p.handbrake_scale(i.mph) } else { p.scale_initial }.max(f32::EPSILON);
            self.target_speed = i.speed;
            self.time = 0.0;
            self.peak_damping = 0.0;
            self.boost = 0.0;
        }
        self.threshold = (self.threshold + p.enter_recovery * dt).min(p.enter_slip);
    }

    fn check_exit(&mut self, p: &DriftParams, i: &DriftInput) {
        let forced = i.air_time > MAX_AIR_TIME || !i.any_grounded || i.speed < MIN_DRIFT_SPEED || i.reverse_gear;
        if forced || (self.time > MIN_DRIFT_TIME && i.rear_slip.abs() < p.exit_angle) {
            self.phase = Phase::Off;
            self.scale = 0.0;
            self.since_exit = 0.0;
            self.threshold = p.enter_slip;
        }
    }

    fn forces(&mut self, p: &DriftParams, i: &DriftInput, dt: f32) -> DriftOutput {
        let mut out = DriftOutput::default();
        let dir = self.direction;
        // Steering relative to the drift: positive into it, negative counter-steering (reversing flips it).
        let into = i.steer * dir * i.mph.signum();
        let counter = into <= 0.0;
        let body_slip = i.angle.to_degrees().abs();

        // Drift scale: integrates throttle (more with body slip), steering on the throttle, brake and the
        // handbrake, and decays at a constant rate; it never changes direction.
        let rate = p.scale_from_angle * (body_slip / p.scale_angle).min(1.0) * i.throttle
            + if counter { p.scale_from_counter } else { p.scale_from_steering } * into * i.throttle
            + p.scale_from_throttle_rise * (i.throttle - self.prev_throttle).max(0.0) / dt.max(1e-4)
            + if counter { -1.0 } else { 1.0 } * p.scale_from_braking * i.brake
            + if i.handbrake { p.handbrake_scale(i.mph) } else { 0.0 }
            - p.scale_decay;
        self.scale = (self.scale + rate * dt).clamp(f32::EPSILON, 1.0);

        // Yaw torque into the drift.
        let steer = i.steer.abs().min(1.0);
        let counter_steering = i.steer * dir < 0.0;
        let factor = if counter_steering {
            p.yaw_no_steer + (p.yaw_counter_steer - p.yaw_no_steer) * steer
        } else {
            p.yaw_no_steer + (1.0 - p.yaw_no_steer) * steer
        } + p.yaw_per_throttle * i.throttle;
        out.yaw_torque =
            dir * factor * p.yaw_torque(self.scale, i.mph, counter_steering) - dir * p.yaw_initial * i.brake;

        // Side force towards the inside of the drift.
        if i.yaw_rate * i.steer > 0.0 {
            let boost = (i.yaw_rate.abs() / BOOST_YAW_RATE).min(1.0) * self.scale * p.side_boost;
            self.boost = self.boost.max(boost);
        }
        self.boost = (self.boost - BOOST_DECAY * dt).max(0.0);
        let [a0, a1] = p.side_angles;
        let ramp = ((body_slip - a0) / (a1 - a0)).clamp(0.0, 1.0);
        let decay = if into > 0.0 { p.side_decay_rate * (body_slip - p.side_decay_angle).max(0.0) } else { 0.0 };
        let [s0, s1] = p.side_speeds;
        let by_speed = ((i.mph.abs() - s0) / (s1 - s0)).clamp(0.0, 1.0);
        let by_scale = self.scale.min(p.side_full_scale) / p.side_full_scale;
        let f = (ramp - decay.min(p.side_decay_cap)).max(0.0) * by_scale * by_speed * (1.0 + self.boost);
        out.side_force = dir * i.mass * p.side_accel * f.min(SIDE_FORCE_CAP_MODEL);

        // Speed hold: pull the car back up to the speed it entered the drift with (at most `hold_lead` ahead).
        if i.speed > self.target_speed || i.throttle < HOLD_MIN_THROTTLE || i.brake > HOLD_MAX_BRAKE {
            self.target_speed = i.speed;
        }
        self.target_speed = self.target_speed.min(i.speed + p.hold_lead * MPH);
        // `55930ac1` is the fraction of the shortfall made up per 60 Hz tick (the game reports this force × 60:
        // it is a per-tick impulse).
        out.hold_force = i.mass * (self.target_speed - i.speed).max(0.0) * i.throttle * p.hold_gain * DATA_HZ;
        out
    }

    /// While drifting the game remaps the steering input: no input holds `Default_steering` of lock into the
    /// drift, full input full lock, and counter-steer is limited early in a drift (`ac261613`), opening up to
    /// full counter-lock as the rear slip grows. Returns the remapped input and the extra steering rate
    /// (degrees per second, towards `Counter_steer_rate_in_drift` with the rear slip).
    pub fn steering(&self, p: &DriftParams, input: f32, rear_slip: f32) -> Option<(f32, f32)> {
        if !self.active() {
            return None;
        }
        let dir = self.direction;
        let rel = input * dir;
        let early = (self.peak_slip / p.steer_peak_angle).clamp(0.0, 1.0);
        let [c0, c1] = p.steer_counter_angles;
        let open = ((rear_slip.abs() - c0) / (c1 - c0)).clamp(0.0, 1.0);
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let lo = lerp(lerp(-1.0, p.steer_counter_early, early), lerp(-1.0, p.steer_counter_late, early), open);
        let mapped = if rel >= 0.0 { lerp(p.steer_default, 1.0, rel) } else { lerp(p.steer_default, lo, -rel) };
        let rate_blend = (rear_slip.abs() / p.counter_steer_rate_angle).clamp(0.0, 1.0);
        Some((mapped.max(lo) * dir, rate_blend))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> DriftParams {
        let mut h = VehicleHandling::default();
        // No data at all: the model falls back to typical values, but the entry angle is required.
        h.params.insert(
            ["Drift", "Drift_trigger", "Slip_angle_to_enter_drift"]
                .iter()
                .map(|s| mw_formats::genesys::field_hash(s))
                .collect(),
            7.0,
        );
        DriftParams::new(&h).expect("drift params")
    }

    fn input(steer: f32, rear_slip: f32) -> DriftInput {
        DriftInput {
            steer,
            throttle: 0.6,
            brake: 0.0,
            handbrake: false,
            speed: 25.0,
            mph: 25.0 / MPH,
            angle: rear_slip.to_radians(),
            rear_slip,
            yaw_rate: 0.3,
            mass: 1300.0,
            all_grounded: true,
            any_grounded: true,
            air_time: 0.0,
            reverse_gear: false,
        }
    }

    const DT: f32 = 1.0 / 60.0;

    #[test]
    fn yaw_curve_runs_from_minimum_to_maximum() {
        let p = params();
        assert_eq!(p.yaw_torque(0.0, 60.0, false), p.yaw_min);
        assert!((p.yaw_torque(1.0, 60.0, false) - p.yaw_max).abs() < 1e-3);
        assert!((p.yaw_torque(1.0, 0.0, false) - p.donut_torque).abs() < 1e-3);
        let mut last = 0.0;
        for k in 0..=20 {
            let t = p.yaw_torque(k as f32 / 20.0, 60.0, false);
            assert!(t >= last);
            last = t;
        }
    }

    #[test]
    fn slide_enters_and_straightening_exits() {
        let p = params();
        let mut s = DriftState::default();
        s.step(&p, &input(0.8, p.enter_slip + 1.0), DT);
        assert_eq!(s.phase, Phase::Drifting);
        assert_eq!(s.direction, 1.0);
        let out = s.step(&p, &input(0.8, 20.0), DT);
        assert!(out.yaw_torque > 0.0 && out.side_force > 0.0);
        // The exit angle only counts after the minimum drift time.
        s.step(&p, &input(0.0, p.exit_angle - 1.0), DT);
        assert_eq!(s.phase, Phase::Drifting);
        for _ in 0..40 {
            s.step(&p, &input(0.0, p.exit_angle - 1.0), DT);
        }
        assert_eq!(s.phase, Phase::Off);
    }

    #[test]
    fn brake_tap_lowers_the_entry_angle() {
        let p = params();
        let mut s = DriftState::default();
        let slip = (p.enter_slip_tap + p.enter_slip) / 2.0;
        s.step(&p, &input(-1.0, -slip), DT);
        assert_eq!(s.phase, Phase::Off);
        let mut i = input(-1.0, 0.0);
        i.brake = 1.0;
        for _ in 0..10 {
            s.step(&p, &i, DT);
        }
        s.step(&p, &input(-1.0, -slip), DT);
        assert_eq!(s.phase, Phase::Drifting);
        assert_eq!(s.direction, -1.0);
    }
}
