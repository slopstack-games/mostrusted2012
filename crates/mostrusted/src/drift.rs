//! The game's drift model, driven by each car's `HandlingBehaviour.Drift`
//! (`DriftBehaviour`) and `Donut` (`DonutAbility`) data.
//!
//! NFS Most Wanted (2012) does not leave drifting to the tyres alone: a drift
//! state takes over once the car slides far enough or the driver asks for one
//! (brake tap or handbrake while steering). In it a yaw torque follows the
//! steering, the rotation is damped, the front wheels counter-steer by
//! themselves and a side force keeps the car on its line. The code is in the
//! encrypted executable, so this is a reconstruction from the data's field
//! names and values; docs/handling.md lists which parts are data and which are
//! model decisions.
//!
//! Sign convention: angles and the drift direction are positive to the left
//! (the car's +X axis), as are steering input and yaw rate in `drive.rs`.

use mw_assets::handling::VehicleHandling;

const MPH: f32 = 0.44704;
/// The per-tick rates in the data (`Decay_rate`, `Drift_angular_damping`, ...)
/// are taken as fractions per 60 Hz tick and rescaled to the actual step.
const DATA_HZ: f32 = 60.0;
/// No drift below this ground speed (m/s, model decision; the data's
/// `Minimum_speed_to_enter_drift` is 0 on most cars).
const MIN_DRIFT_SPEED: f32 = 5.0;
/// A requested drift that has not reached its entry slip after this long is
/// abandoned (s, model decision).
const ENTRY_TIME: f32 = 1.0;
/// The yaw torque fades as `(1 - angle / Maximum_drift_angle)^power` (model
/// decision: without a fade the torque does not depend on the angle, and the
/// car either spins or straightens; 1.5 holds every car checked).
const TORQUE_FADE_POWER: f32 = 1.5;
/// Donuts start at full lock and throttle (model decision).
const DONUT_INPUT: f32 = 0.8;

/// `DriftBehaviour` / `DonutAbility` values for one car.
#[derive(Debug, Clone)]
pub struct DriftParams {
    /// `Drift_trigger.Slip_angle_to_enter_drift` (degrees of body slip).
    pub enter_slip: f32,
    /// `Drift_trigger.8f151151` (always ≤ the above): the slip that confirms
    /// a drift the driver asked for (hypothesis).
    pub enter_slip_requested: f32,
    /// `Drift_trigger.0dce38f7` (2; 90 where drifting is disabled): the drift
    /// scale that asks for a drift (hypothesis).
    pub scale_trigger: f32,
    /// `Drift_trigger.Minimum_speed_to_enter_drift` (mph).
    pub enter_min_speed: f32,
    /// `Drift_exit.Drift_angle_to_exit_drift` (degrees).
    pub exit_angle: f32,
    /// `Drift_exit.Time_to_blend_damping` (s).
    pub blend_time: f32,
    /// `Drift_scale.Drift_scale_from_braking` / `_from_steering` /
    /// `_from_counter_steering` (scale at full input) and `_decay` (per second).
    pub scale_from_braking: f32,
    pub scale_from_steering: f32,
    pub scale_from_counter: f32,
    pub scale_decay: f32,
    /// Yaw torque against steering towards the drift: (input, N m) from
    /// `Yaw_torque.2ef426dd` (−0.1 on every car) with no torque, then
    /// `70528afa` → `Minimum_yaw_torque`, `a5c9496f` → `Mid_yaw_torque`, and
    /// full steering → `Maximum_yaw_torque` (hypothesis).
    pub yaw_curve: [(f32, f32); 4],
    /// `Yaw_torque.Counter_steer_yaw_torque` (0 on every car).
    pub counter_yaw: f32,
    /// `Other.Drift_angular_damping` (fraction of the yaw rate per tick).
    pub angular_damping: f32,
    /// `Other.Maximum_drift_angle` (degrees).
    pub max_angle: f32,
    /// `Other.Side_force_multiplier`: side acceleration (m/s²) at full side
    /// force (unit is a model decision).
    pub side_force: f32,
    /// `Side_force.Minimum_drift_angle_for_side_force` (degrees): none below,
    /// rising to full over `Side_force.Drift_angle_for_side_force` degrees.
    pub side_min_angle: f32,
    pub side_ramp: f32,
    /// `Side_force.Drift_scale_for_maximum_side_force`.
    pub side_full_scale: f32,
    /// `Side_force.Speed_for_maximum_side_force` (mph).
    pub side_full_speed: f32,
    /// `Side_force.Drift_angle_for_decay` (degrees) and `Decay_rate` (per
    /// tick): below the angle the drift fades out.
    pub decay_angle: f32,
    pub decay_rate: f32,
    /// `Side_force.ff5dcf85` (0.02–0.025, per tick): the recovery rate above
    /// the decay angle (hypothesis).
    pub growth_rate: f32,
    /// `Steering.Counter_steer_rate_in_drift` (degrees per second).
    pub counter_steer_rate: f32,
    /// `Other.Low_slip_angle_for_counter_steering` (degrees): the front wheels
    /// start counter-steering at this drift angle, by
    /// `Other.Max_counter_steering_power` times the excess.
    pub counter_low_angle: f32,
    pub counter_power: f32,
    /// `Steering.Maximum_steering_angle` (degrees): counter-steer limit.
    pub max_wheel_angle: f32,
    /// `Yaw_torque.Donut_yaw_torque` (N m).
    pub donut_torque: f32,
    /// `Yaw_torque.Donut_speed_limit_low` / `_high` (mph): a donut starts
    /// above the low limit (and below `Donut_scale.Max_speed_for_donut`) and
    /// ends above the high one (hypothesis).
    pub donut_speeds: [f32; 2],
    pub donut_start_speed: f32,
    /// `Donut.Donut_scale.Gas_influence` / `Steering_influence` / `Speed_influence`
    /// (per mph): the donut torque scales with throttle and steering, and with
    /// speed so the car circles instead of spinning on the spot (hypothesis).
    pub donut_gas: f32,
    pub donut_steering: f32,
    pub donut_speed: f32,
}

impl DriftParams {
    /// `None` when the car has no drift data or the data disables drifting
    /// (`Maximum_drift_angle` 180 with unreachable triggers, as on some cop cars).
    pub fn new(h: &VehicleHandling) -> Option<Self> {
        let d = |path: &str| h.param(&format!("Drift.{path}"));
        let max_angle = d("Other.Maximum_drift_angle")?;
        if max_angle >= 180.0 {
            return None;
        }
        let or = |path: &str, default: f32| d(path).unwrap_or(default);
        let donut = |path: &str, default: f32| h.param(&format!("Donut.Donut_scale.{path}")).unwrap_or(default);
        let min_yaw = or("Yaw_torque.Minimum_yaw_torque", 1500.0);
        let mid_yaw = or("Yaw_torque.Mid_yaw_torque", 9000.0);
        let max_yaw = or("Yaw_torque.Maximum_yaw_torque", 15000.0);
        let x0 = or("Yaw_torque.2ef426dd", -0.1);
        let x1 = or("Yaw_torque.70528afa", 0.15).max(x0 + 0.01);
        let x2 = or("Yaw_torque.a5c9496f", 0.45).max(x1 + 0.01);
        let enter_slip = or("Drift_trigger.Slip_angle_to_enter_drift", 8.0);
        let donut_low = or("Yaw_torque.Donut_speed_limit_low", 5.0);
        Some(Self {
            enter_slip,
            enter_slip_requested: or("Drift_trigger.8f151151", 5.0).min(enter_slip),
            scale_trigger: or("Drift_trigger.0dce38f7", 2.0),
            enter_min_speed: or("Drift_trigger.Minimum_speed_to_enter_drift", 0.0),
            exit_angle: or("Drift_exit.Drift_angle_to_exit_drift", 3.0),
            blend_time: or("Drift_exit.Time_to_blend_damping", 0.25),
            scale_from_braking: or("Drift_scale.Drift_scale_from_braking", 1.0),
            scale_from_steering: or("Drift_scale.Drift_scale_from_steering", 1.6),
            scale_from_counter: or("Drift_scale.Drift_scale_from_counter_steering", 1.0),
            scale_decay: or("Drift_scale.Drift_scale_decay", 0.5),
            yaw_curve: [(x0, 0.0), (x1, min_yaw), (x2, mid_yaw), (1.0, max_yaw)],
            counter_yaw: or("Yaw_torque.Counter_steer_yaw_torque", 0.0),
            angular_damping: or("Other.Drift_angular_damping", 0.05).clamp(0.0, 0.5),
            max_angle,
            side_force: or("Other.Side_force_multiplier", 5.0),
            side_min_angle: or("Side_force.Minimum_drift_angle_for_side_force", 5.0),
            side_ramp: or("Side_force.Drift_angle_for_side_force", 1.5).max(0.1),
            side_full_scale: or("Side_force.Drift_scale_for_maximum_side_force", 0.3).max(0.01),
            side_full_speed: or("Side_force.Speed_for_maximum_side_force", 90.0).max(1.0),
            decay_angle: or("Side_force.Drift_angle_for_decay", 6.5),
            decay_rate: or("Side_force.Decay_rate", 0.018).clamp(0.0, 1.0),
            growth_rate: or("Side_force.ff5dcf85", 0.025).clamp(0.0, 1.0),
            counter_steer_rate: h.param("Steering.Counter_steer_rate_in_drift").unwrap_or(120.0),
            counter_low_angle: or("Other.Low_slip_angle_for_counter_steering", 4.0),
            counter_power: or("Other.Max_counter_steering_power", 1.25).max(0.0),
            max_wheel_angle: h.param("Steering.Maximum_steering_angle").unwrap_or(35.0),
            donut_torque: or("Yaw_torque.Donut_yaw_torque", 0.0),
            donut_speeds: [donut_low, or("Yaw_torque.Donut_speed_limit_high", 35.0).max(donut_low + 1.0)],
            donut_start_speed: donut("Max_speed_for_donut", 10.0),
            donut_gas: donut("Gas_influence", 1.0),
            donut_steering: donut("Steering_influence", 0.0),
            donut_speed: donut("Speed_influence", 0.025),
        })
    }

    /// Yaw torque (N m) for steering `s` towards the drift direction (−1..1).
    pub fn yaw_torque(&self, s: f32) -> f32 {
        let c = &self.yaw_curve;
        if s < c[0].0 {
            return self.counter_yaw;
        }
        for w in c.windows(2) {
            let ((x0, y0), (x1, y1)) = (w[0], w[1]);
            if s <= x1 {
                return y0 + (y1 - y0) * (s - x0) / (x1 - x0);
            }
        }
        c[3].1
    }
}

/// What the drift model needs from the car each physics tick.
pub struct DriftInput {
    /// Steering input, −1..1, positive = left.
    pub steer: f32,
    pub throttle: f32,
    pub brake: f32,
    pub handbrake: bool,
    /// Ground speed (m/s), forward speed (m/s, negative = reversing).
    pub speed: f32,
    pub forward_speed: f32,
    /// Drift angle (rad): heading relative to the velocity, positive = nose left.
    pub angle: f32,
    /// Yaw rate (rad/s, positive = left).
    pub yaw_rate: f32,
    pub yaw_inertia: f32,
    pub mass: f32,
    pub grounded: bool,
}

/// Forces the drift model adds this tick.
#[derive(Debug, Default, Clone, Copy)]
pub struct DriftOutput {
    /// Torque about the car's up axis (N m, positive = left).
    pub yaw_torque: f32,
    /// Force across the velocity (N, positive = to the left of travel).
    pub side_force: f32,
}

/// Where the car is in a drift.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Off,
    /// The driver asked for a drift: the yaw torque swings the tail out until
    /// the slip confirms it.
    Entering,
    Drifting,
    /// Spinning on the spot or at walking pace on full lock and throttle.
    Donut,
}

/// Drift state carried between ticks.
#[derive(Debug, Default, Clone)]
pub struct DriftState {
    pub phase: Phase,
    /// +1 drifting left (nose left of the velocity), −1 right.
    pub direction: f32,
    /// Drift scale: how hard the driver asks for a drift.
    pub scale: f32,
    /// Drift level (0..1): fades below `Drift_angle_for_decay` and scales the
    /// drift's yaw torque and side force.
    pub level: f32,
    /// Time since the last exit, for blending the damping out.
    pub since_exit: f32,
    /// Time in the current phase.
    pub time: f32,
    /// The last tick's output (diagnostics).
    pub last: DriftOutput,
}

impl DriftState {
    /// In a drift proper (not entering one, not a donut).
    pub fn active(&self) -> bool {
        self.phase == Phase::Drifting
    }

    pub fn step(&mut self, p: &DriftParams, i: &DriftInput, dt: f32) -> DriftOutput {
        let angle = i.angle.to_degrees();
        let tick = dt * DATA_HZ;
        let mph = i.speed / MPH;

        // Drift scale: the driver's request, from braking (or the handbrake)
        // and steering, plus counter-steering in a drift. It follows the
        // inputs up at once and decays at `Drift_scale_decay` per second.
        let brake = if i.handbrake { 1.0 } else { i.brake };
        let counter = if self.phase == Phase::Off { 0.0 } else { (-i.steer * self.direction).max(0.0) };
        let demand = brake * p.scale_from_braking
            + (i.steer.abs() - counter) * p.scale_from_steering
            + counter * p.scale_from_counter;
        self.scale = (self.scale - p.scale_decay * dt).max(demand).max(0.0);

        let fast = i.speed >= (p.enter_min_speed * MPH).max(MIN_DRIFT_SPEED);
        let rolling = i.grounded && i.forward_speed > -0.5;
        let donut_input = i.throttle >= DONUT_INPUT && i.steer.abs() >= DONUT_INPUT && !i.handbrake;
        self.time += dt;
        match self.phase {
            _ if !i.grounded => self.set(Phase::Off),
            Phase::Off | Phase::Entering | Phase::Drifting
                if p.donut_torque > 0.0
                    && rolling
                    && donut_input
                    && mph >= p.donut_speeds[0]
                    && mph < p.donut_start_speed =>
            {
                self.direction = i.steer.signum();
                self.set(Phase::Donut);
            }
            Phase::Donut => {
                if !donut_input || mph > p.donut_speeds[1] || i.steer * self.direction < 0.0 || !rolling {
                    self.set(Phase::Off);
                }
            }
            _ if !fast || i.forward_speed < 1.0 => self.set(Phase::Off),
            Phase::Off => {
                if angle.abs() >= p.enter_slip && i.steer * angle.signum() >= p.yaw_curve[0].0 {
                    // Sliding far enough by itself (power oversteer, handbrake).
                    self.direction = angle.signum();
                    self.set(Phase::Drifting);
                } else if (self.scale > p.scale_trigger || i.handbrake) && i.steer.abs() > p.yaw_curve[1].0.max(0.05) {
                    // Asked for: drift scale over the trigger, or the handbrake
                    // pulled while steering (model decision).
                    self.direction = i.steer.signum();
                    self.set(Phase::Entering);
                }
            }
            Phase::Entering => {
                if angle * self.direction >= p.enter_slip_requested {
                    self.set(Phase::Drifting);
                } else if i.steer * self.direction < p.yaw_curve[1].0 || self.time > ENTRY_TIME {
                    // Steering released or reversed, or the tail never came out.
                    self.set(Phase::Off);
                }
            }
            Phase::Drifting => {
                if angle * self.direction < p.exit_angle {
                    // Back under the exit angle, or sliding the other way now.
                    self.set(Phase::Off);
                }
            }
        }

        let damping = |c: f32| -(1.0 - c.min(0.99)).ln() * DATA_HZ * i.yaw_inertia * i.yaw_rate;
        let a = angle * self.direction;
        let s_d = i.steer * self.direction;
        let mut out = DriftOutput::default();
        match self.phase {
            Phase::Off => {
                // Blend the drift damping out after an exit.
                self.since_exit += dt;
                let blend = 1.0 - (self.since_exit / p.blend_time.max(1e-3)).min(1.0);
                if blend > 0.0 {
                    out.yaw_torque = -damping(p.angular_damping * blend);
                }
            }
            Phase::Donut => {
                let input = (p.donut_gas * i.throttle + p.donut_steering * i.steer.abs()).min(1.0);
                let speed = (p.donut_speed * mph).min(1.0);
                out.yaw_torque = self.direction * p.donut_torque * input * speed - damping(p.angular_damping);
            }
            Phase::Entering | Phase::Drifting => {
                // The drift level fades below `Drift_angle_for_decay` and
                // recovers above it, so a drift that has gone shallow dies out.
                if self.phase == Phase::Drifting {
                    if a < p.decay_angle {
                        self.level *= (1.0 - p.decay_rate).powf(tick);
                    } else {
                        self.level = (self.level + p.growth_rate * tick).min(1.0);
                    }
                }
                // Yaw torque from the steering, fading out towards the maximum
                // angle, against the drift's damping of the yaw rate.
                let fade = (1.0 - a.max(0.0) / p.max_angle).clamp(0.0, 1.0).powf(TORQUE_FADE_POWER);
                out.yaw_torque = self.direction * p.yaw_torque(s_d) * fade * self.level - damping(p.angular_damping);
                if self.phase == Phase::Drifting {
                    // Side force towards the inside of the drift.
                    let by_angle = ((a - p.side_min_angle) / p.side_ramp).clamp(0.0, 1.0);
                    let by_speed = (i.speed / (p.side_full_speed * MPH)).min(1.0);
                    let by_scale = (self.scale / p.side_full_scale).min(1.0);
                    out.side_force =
                        self.direction * p.side_force * i.mass * by_angle * by_speed * by_scale * self.level;
                }
            }
        }
        self.last = out;
        out
    }

    /// While drifting: the front wheels' target angle (rad, positive = left)
    /// and steering rate (rad/s). The wheels counter-steer against the drift
    /// angle beyond `Low_slip_angle_for_counter_steering`, by
    /// `Max_counter_steering_power` times the excess (model decision), at
    /// `Counter_steer_rate_in_drift`; the steering input sets the yaw torque.
    pub fn counter_steer(&self, p: &DriftParams, angle: f32) -> Option<(f32, f32)> {
        if self.phase != Phase::Drifting {
            return None;
        }
        let excess = (angle.to_degrees() * self.direction - p.counter_low_angle).max(0.0);
        let wheel = (excess * p.counter_power).min(p.max_wheel_angle);
        Some(((-self.direction * wheel).to_radians(), p.counter_steer_rate.to_radians()))
    }

    fn set(&mut self, phase: Phase) {
        if phase == self.phase {
            return;
        }
        if phase == Phase::Off {
            self.since_exit = 0.0;
        }
        if phase != Phase::Off {
            self.level = 1.0;
        }
        self.phase = phase;
        self.time = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> DriftParams {
        let mut h = VehicleHandling::default();
        // No data at all: the model falls back to typical values, but
        // `Maximum_drift_angle` is required.
        h.params.insert(
            ["Drift", "Other", "Maximum_drift_angle"].iter().map(|s| mw_formats::genesys::field_hash(s)).collect(),
            90.0,
        );
        DriftParams::new(&h).expect("drift params")
    }

    fn input(steer: f32, angle_deg: f32) -> DriftInput {
        DriftInput {
            steer,
            throttle: 0.6,
            brake: 0.0,
            handbrake: false,
            speed: 25.0,
            forward_speed: 25.0,
            angle: angle_deg.to_radians(),
            yaw_rate: 0.3,
            yaw_inertia: 2500.0,
            mass: 1300.0,
            grounded: true,
        }
    }

    #[test]
    fn yaw_curve_is_monotonic_and_hits_the_data_points() {
        let p = params();
        assert_eq!(p.yaw_torque(-0.5), p.counter_yaw);
        assert_eq!(p.yaw_torque(p.yaw_curve[0].0), 0.0);
        assert_eq!(p.yaw_torque(1.0), p.yaw_curve[3].1);
        let mut last = 0.0;
        for k in 0..=20 {
            let t = p.yaw_torque(-0.1 + 1.1 * k as f32 / 20.0);
            assert!(t >= last);
            last = t;
        }
    }

    #[test]
    fn slide_enters_and_straightening_exits() {
        let p = params();
        let mut s = DriftState::default();
        s.step(&p, &input(0.8, p.enter_slip + 1.0), 1.0 / 64.0);
        assert_eq!(s.phase, Phase::Drifting);
        assert_eq!(s.direction, 1.0);
        let out = s.step(&p, &input(0.8, 20.0), 1.0 / 64.0);
        assert!(out.yaw_torque > 0.0 && out.side_force > 0.0);
        s.step(&p, &input(0.0, p.exit_angle - 1.0), 1.0 / 64.0);
        assert_eq!(s.phase, Phase::Off);
    }

    #[test]
    fn brake_tap_while_steering_asks_for_a_drift() {
        let p = params();
        let mut s = DriftState::default();
        let mut i = input(-1.0, 0.0);
        i.brake = 1.0;
        s.step(&p, &i, 1.0 / 64.0);
        assert_eq!(s.phase, Phase::Entering);
        assert_eq!(s.direction, -1.0);
        // The tail comes out: the drift is confirmed.
        s.step(&p, &input(-1.0, -(p.enter_slip_requested + 0.5)), 1.0 / 64.0);
        assert_eq!(s.phase, Phase::Drifting);
    }
}
