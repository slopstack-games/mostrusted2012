//! The AI racer's driving law, ported from the game's racer state (state 8, `FUN_0050da90`; docs/decomp.md
//! "AI racers"). Pure maths, no Bevy systems: `ai.rs` gathers the inputs every fixed tick and writes the result
//! into the car's `CarControls`, which run through the same physics as the player's (the game writes every race
//! car's controls with `FUN_009aacc0`).
//!
//! Per tick, in the game's order: the command builder `FUN_0050a740` (steering `FUN_00508d50` → `FUN_00506c00`,
//! target speed), nitrous (`FUN_004ff620`, `FUN_004fdb60`), the emergency brake (`FUN_005095b0` → `FUN_00508070`)
//! and the pedal controller (`FUN_004fa0e0`). The crash recovery (state 11, `FUN_004eeba0`) takes over after the
//! car has stuck against something (`FUN_004fb9f0`).

use bevy::prelude::*;

use crate::ai::AiRoute;
use crate::drive::CarControls;

const MPH: f32 = 0.44704;

/// `AIConfig` (`GLOBALCONFIG.BNDL` 010000000007fb0e), the fields the racer reads, by instance offset. Defaults are
/// the data's values; `ai.rs` reads them from the data at startup.
#[derive(Debug, Clone, Copy)]
pub struct AiConfig {
    /// 0x10: the AI's nitrous refill (tank units per second, `FUN_00667c10`).
    pub nitrous_refill: f32,
    /// 0x18: the lane offset's speed (m/s, `FUN_0050b000`).
    pub lane_rate: f32,
    /// 0x28: emergency-brake time to contact (s).
    pub contact_time: f32,
    /// 0x124 / 0x128: nitrous keeps firing below this turn ahead, starts below that (degrees).
    pub nitrous_keep_turn: f32,
    pub nitrous_start_turn: f32,
    /// 0x140: tank fraction needed to start nitrous.
    pub nitrous_min_tank: f32,
    /// 0x144: shortest look-ahead (m).
    pub min_look: f32,
    /// 0x14c: nitrous starts only this far below the target speed (mph).
    pub nitrous_margin_mph: f32,
    /// 0x19c: no crash detection this close to the route's end (m).
    pub stuck_end_radius: f32,
    /// 0x1a0 / 0x1a4: reset after this long in the crash recovery (s; the second when off screen).
    pub reset_time: f32,
    pub reset_time_hidden: f32,
    /// 0x1ec: a corner node shortens the look-ahead when its inside clearance is below this (m).
    pub corner_clearance: f32,
    /// 0x1f0: the look-ahead scan covers this many seconds of travel.
    pub look_scan_time: f32,
    /// 0x1f4: a node turning more than this (degrees) shortens the look-ahead.
    pub sharp_turn: f32,
    /// 0x1fc: shortest look-ahead time (s).
    pub min_look_time: f32,
    /// 0xc8: the corner speed rule's factor (`FUN_004ea860`).
    pub corner_speed_scale: f32,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            nitrous_refill: 10.0,
            lane_rate: 3.0,
            contact_time: 1.5,
            nitrous_keep_turn: 30.0,
            nitrous_start_turn: 5.0,
            nitrous_min_tank: 0.25,
            min_look: 15.0,
            nitrous_margin_mph: 10.0,
            stuck_end_radius: 10.0,
            reset_time: 10.0,
            reset_time_hidden: 3.0,
            corner_clearance: 8.0,
            look_scan_time: 0.6,
            sharp_turn: 90.0,
            min_look_time: 0.45,
            corner_speed_scale: 1.8,
        }
    }
}

/// `.data` 0x010c4414: added to the look-ahead distance (m).
const LOOK_PAD: f32 = 3.0;
/// Longest look-ahead time (s), `FUN_00503880`.
const MAX_LOOK_TIME: f32 = 0.6;
/// The steering PID clamps its error and integral at ±20 (`FUN_00436920` with the state at controller +0x00).
const STEER_CLAMP: f32 = 20.0;
/// The speed PID (`FUN_004f8100`, `.rdata` 0x00fa8ecc..): gains, the error / integral clamp, the per-tick pedal
/// step (0x00fa8ee0).
const SPEED_PID: [f32; 3] = [0.6, 0.05, 0.5];
const SPEED_CLAMP: f32 = 9.0;
const PEDAL_STEP: f32 = 0.03;
/// Nitrous only while the car is this far below its target speed (m/s, `.rdata` 0x00fa8f0c).
const NITROUS_SPEED_GAP: f32 = 2.7;
/// The emergency brake acts above this speed (mph, `.rdata` 0x00fa8eb8) and searches 40..150 m.
const EMERGENCY_MIN_MPH: f32 = 70.0;
/// Crash detection (`FUN_004fb9f0`): a contact held this long below this speed starts the recovery.
const STUCK_TIME: f32 = 1.0;
const STUCK_MPH: f32 = 10.0;
/// The recovery drives at 10 mph (`FUN_004f9e80`), steers 4 × the heading error, leaves above 5 mph.
const RECOVER_MPH: f32 = 10.0;
const RECOVER_STEER_GAIN: f32 = 4.0;
const RECOVER_EXIT_MPH: f32 = 5.0;
/// Time a recovery direction is held before it may flip again (s).
const RECOVER_LOCK: f32 = 1.0;

/// A per-car PID controller as the game runs it (`FUN_00436920`): e and the integral clamped, the output uses the
/// integral from before this tick, u = −(Kp e + Ki I + Kd Δe / dt).
#[derive(Debug, Default, Clone, Copy)]
pub struct Pid {
    integral: f32,
    previous: Option<f32>,
}

impl Pid {
    fn step(&mut self, gains: [f32; 3], e: f32, clamp: f32, dt: f32) -> f32 {
        let e = e.clamp(-clamp, clamp);
        let de = self.previous.map_or(0.0, |p| (e - p) / dt.max(1e-4));
        let u = -(gains[0] * e + gains[1] * self.integral + gains[2] * de);
        self.integral = (self.integral + e * dt).clamp(-clamp, clamp);
        self.previous = Some(e);
        u
    }
}

/// Another car, for the emergency brake: ground-plane position, velocity (heading × speed) and circle radius.
#[derive(Debug, Clone, Copy)]
pub struct OtherCar {
    pub position: Vec2,
    pub velocity: Vec2,
    pub radius: f32,
}

/// What the driver sees this tick.
pub struct DriverInput<'a> {
    pub dt: f32,
    pub route: &'a AiRoute,
    /// Distance along the route of the car (projected).
    pub progress: f32,
    pub position: Vec3,
    pub forward: Vec3,
    pub velocity: Vec3,
    /// The car's circle radius for the emergency brake (m).
    pub radius: f32,
    /// Steering PID gains: `Vehicle_Gameplay.PIDController` (Proportional, Integral, Derivative).
    pub steer_gains: [f32; 3],
    /// The rubber band's target speed (m/s; `f32::MAX` without one, as the game's schedule returns outside a race).
    pub schedule_speed: f32,
    /// The car's corner speed limit at its position from the path (m/s, `f32::MAX` for none).
    pub corner_speed: f32,
    pub all_wheels_off: bool,
    /// The body touches the world (the game's per-slot contact flags).
    pub contact: bool,
    pub nitrous_fill: f32,
    pub nitrous_burning: bool,
    /// The player's car is ahead along this car's heading.
    pub player_ahead: bool,
    /// Probe rays at ±25° in the direction of travel hit something (left, right).
    pub probes: [bool; 2],
    pub others: &'a [OtherCar],
}

/// The driver's state between ticks (the controller at driver +0x55e0 and the state machine).
#[derive(Debug, Default, Clone)]
pub struct DriverState {
    steer_pid: Pid,
    speed_pid: Pid,
    /// The pedal integrated by the speed PID (controller +0x74): > 0 throttle, < 0 brake.
    pedal: f32,
    /// The lateral offset from the path (m; `FUN_0050b000`'s planner is not ported, so it stays 0).
    pub lane_offset: f32,
    stuck_time: f32,
    pub recovering: Option<Recovery>,
    /// Time accumulated in the recovery (driver +0x5854): a reset is asked for past `reset_time`.
    recovery_time: f32,
    /// Diagnostics: look-ahead distance (m), lateral error (m), target speed (m/s), emergency brake on.
    pub look: f32,
    pub lateral_error: f32,
    pub target_speed: f32,
    pub emergency: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Recovery {
    pub reverse: bool,
    lock: f32,
}

/// The driver's output: the controls, and whether it asks to be put back on the road (the game's reset request).
pub struct DriverOutput {
    pub controls: CarControls,
    pub reset: bool,
}

impl DriverState {
    pub fn tick(&mut self, cfg: &AiConfig, i: &DriverInput) -> DriverOutput {
        let dt = i.dt;
        let flat = |v: Vec3| Vec2::new(v.x, v.z);
        let speed = i.velocity.length();
        let h = flat(i.forward).normalize_or(Vec2::Y);
        let mut out = CarControls::default();

        // --- Crash detection (`FUN_004fb9f0`) and the reset timer (`FUN_004e75f0`) --------------------------
        let near_end = !i.route.closed && i.route.length() - i.progress < cfg.stuck_end_radius;
        if i.contact && speed < STUCK_MPH * MPH && !near_end {
            self.stuck_time += dt;
        } else if !i.contact {
            self.stuck_time = 0.0;
        }
        if self.recovering.is_some() {
            self.recovery_time += dt;
        } else {
            self.recovery_time = (self.recovery_time - 0.25 * dt).max(0.0);
        }
        // Model decision: we cannot tell whether the car is on screen; the longer (on-screen) time applies.
        let reset = self.recovery_time > cfg.reset_time;
        if reset {
            self.recovery_time = 0.0;
            self.recovering = None;
            self.stuck_time = 0.0;
        }
        if self.recovering.is_none() && self.stuck_time > STUCK_TIME {
            self.recovering = Some(Recovery { reverse: false, lock: 0.0 });
            self.stuck_time = 0.0;
        }
        if let Some(r) = self.recovering.as_mut() {
            out = recover(r, i, speed, h, dt);
            let signed = i.velocity.dot(i.forward);
            if !r.reverse && signed > RECOVER_EXIT_MPH * MPH && !i.probes.iter().any(|&p| p) {
                self.recovering = None;
                self.pedal = 0.0;
            }
            return DriverOutput { controls: out, reset };
        }

        // --- Steering (`FUN_00506c00`) --------------------------------------------------------------------
        let look_time = self.look_time(cfg, i, speed);
        let look = (speed * look_time + LOOK_PAD).max(cfg.min_look);
        let (path_point, _) = i.route.at(i.progress + look);
        let aim = flat(i.position) + h * look;
        // (pos + L·h − P) · (−h.z, h.x): positive when the aim point is to the right of the path.
        let lateral = (aim - flat(path_point)).dot(Vec2::new(-h.y, h.x));
        let u = self.steer_pid.step(i.steer_gains, lateral - self.lane_offset, STEER_CLAMP, dt);
        // u is the game's steering (positive = right); ours is positive to the left. No steering in the air
        // (`FUN_008f5350`).
        out.steer = if i.all_wheels_off { 0.0 } else { -u.clamp(-1.0, 1.0) };
        self.look = look;
        self.lateral_error = lateral;

        // --- Target speed (`FUN_00508d50`): the schedule, lowered by the obstacle check ----------------------
        let target = i.schedule_speed.min(i.corner_speed);
        self.target_speed = target;

        // --- Nitrous (`FUN_004ff620`, then state 8's `FUN_004fdb60`) ----------------------------------------
        if speed + NITROUS_SPEED_GAP < target {
            let turn = turn_ahead(i.route, i.progress, speed * 1.5).to_degrees();
            out.nitrous = if i.nitrous_burning {
                turn < cfg.nitrous_keep_turn
            } else {
                i.nitrous_fill > cfg.nitrous_min_tank
                    && speed <= i.schedule_speed - cfg.nitrous_margin_mph * MPH
                    && turn < cfg.nitrous_start_turn
            };
            if i.player_ahead && speed < i.schedule_speed {
                out.nitrous = true;
            }
        }

        // --- Emergency brake (`FUN_00508070`) ---------------------------------------------------------------
        self.emergency = false;
        if speed > EMERGENCY_MIN_MPH * MPH {
            let r = (speed * cfg.contact_time * 0.5).clamp(40.0, 150.0);
            let centre = flat(i.position) + h * (r - 10.0);
            let me = OtherCar { position: flat(i.position), velocity: h * speed, radius: i.radius };
            self.emergency = i
                .others
                .iter()
                .filter(|o| o.position.distance(centre) < r)
                .any(|o| time_to_contact(&me, o).is_some_and(|t| t < cfg.contact_time));
        }

        // --- Pedal controller (`FUN_004fa0e0`) --------------------------------------------------------------
        let pid = self.speed_pid.step(SPEED_PID, speed - target, SPEED_CLAMP, dt);
        self.pedal = (self.pedal + PEDAL_STEP * pid).clamp(-1.0, 1.0);
        if self.pedal > 0.0 {
            out.throttle = self.pedal;
        } else {
            out.brake = -self.pedal;
        }
        if self.emergency {
            out.brake = 1.0;
        }
        if target.abs() < 0.5 && speed < 10.0 * MPH {
            out.handbrake = true;
            if speed < 0.5 {
                out.throttle = 0.0;
                out.brake = 1.0;
            }
        }
        if i.all_wheels_off {
            out.brake = 0.0;
        }
        DriverOutput { controls: out, reset }
    }

    /// The look-ahead time (`FUN_00503880`): the path nodes within `look_scan_time` × speed metres are summed for
    /// their turn; the scan stops at the first node whose inside clearance is under `corner_clearance` and that
    /// turns more than `sharp_turn`. T = that distance / speed, at least `min_look_time`, at most 0.6 s.
    fn look_time(&self, cfg: &AiConfig, i: &DriverInput, speed: f32) -> f32 {
        if speed < 0.1 {
            return MAX_LOOK_TIME;
        }
        let scan = cfg.look_scan_time * speed;
        let mut d = scan;
        for n in i.route.nodes.iter().filter(|n| n.s > i.progress && n.s <= i.progress + scan) {
            // Inside clearance: the drivable half-width at the node (the game's +0x38 / +0x3c; their formula is
            // not read, model decision).
            let clearance = i.route.half_width_at(n.s);
            if clearance < cfg.corner_clearance && n.turn.abs().to_degrees() > cfg.sharp_turn {
                d = n.s - i.progress;
                break;
            }
        }
        (d / speed).max(cfg.min_look_time).min(MAX_LOOK_TIME)
    }
}

/// The larger of the left and the right heading-change sums over the path nodes in the next `distance` metres
/// (`FUN_004e4eb0`), radians.
pub fn turn_ahead(route: &AiRoute, from: f32, distance: f32) -> f32 {
    let (mut left, mut right) = (0.0f32, 0.0f32);
    for n in route.nodes.iter().filter(|n| n.s > from && n.s <= from + distance) {
        if n.turn > 0.0 {
            left += n.turn;
        } else {
            right -= n.turn;
        }
    }
    left.max(right)
}

/// 2D circle-circle time to contact (`FUN_00508070`): 0 when overlapping, `None` when separating or never.
fn time_to_contact(a: &OtherCar, b: &OtherCar) -> Option<f32> {
    let d = b.position - a.position;
    let w = b.velocity - a.velocity;
    let r = a.radius + b.radius;
    let c = d.length_squared() - r * r;
    if c <= 0.0 {
        return Some(0.0);
    }
    let bq = d.dot(w);
    if bq >= 0.0 {
        return None;
    }
    let aq = w.length_squared();
    let disc = bq * bq - aq * c;
    if disc < 0.0 || aq < 1e-6 {
        return None;
    }
    Some((-bq - disc.sqrt()) / aq)
}

/// The crash recovery (state 11, `FUN_004eeba0`): steer 4 × the heading error to the path (mirrored in reverse),
/// drive at 10 mph through the banded controller `FUN_004f9e80`, flip direction when the probes in the direction of
/// travel hit, at most once a second.
fn recover(r: &mut Recovery, i: &DriverInput, speed: f32, h: Vec2, dt: f32) -> CarControls {
    r.lock = (r.lock - dt).max(0.0);
    if r.lock <= 0.0 && i.probes.iter().any(|&p| p) {
        r.reverse = !r.reverse;
        r.lock = RECOVER_LOCK;
    }
    let (_, tangent) = i.route.at(i.progress + 5.0);
    let t = Vec2::new(tangent.x, tangent.z).normalize_or(h);
    // Heading error: positive when the path points to the car's left (+X is left of +Z).
    let error = (h.x * t.y - h.y * t.x).atan2(h.dot(t)) * -1.0;
    let mut steer = (RECOVER_STEER_GAIN * error).clamp(-1.0, 1.0);
    let pedal = banded_pedal(speed / MPH - RECOVER_MPH);
    let mut c = CarControls::default();
    if r.reverse {
        steer = -steer;
        c.brake = pedal.max(0.0);
        c.throttle = (-pedal).max(0.0);
    } else {
        c.throttle = pedal.max(0.0);
        c.brake = (-pedal).max(0.0);
    }
    c.steer = steer;
    c
}

/// `FUN_004f9e80` with e = speed − 10 mph (mph): 0 within 1 mph, full pedal below −1, full brake above 9, a
/// ramp from 4 to 9 mph. Model decision: the ramp's shape between the read thresholds (the smoothing helper
/// `FUN_005114e0` is not read).
fn banded_pedal(e: f32) -> f32 {
    if e.abs() < 1.0 {
        0.0
    } else if e <= -1.0 {
        1.0
    } else if e > 9.0 {
        -1.0
    } else if e > 4.0 {
        -(e - 4.0) / 5.0
    } else {
        0.0
    }
}

/// The race's rubber band for one opponent (`RaceBalancingData.OpponentBalancingData[k]` and the event's
/// `RaceBalancingProfile`), read in `FUN_00645c90` → `FUN_0063ef30` / `FUN_006432a0`: a target-speed schedule,
/// not a grip or torque multiplier. The driver's target speed is this schedule (driver +0x5668).
#[derive(Debug, Clone, Copy)]
pub struct RubberBand {
    /// `RaceBalancingProfile.MaxSpeed` × the event's pace multiplier for the player's car (m/s).
    pub max_speed: f32,
    /// `AheadDistance` (+0x10) and `BehindDistance` (+0x14), metres; the blend t runs between them.
    pub ahead: f32,
    pub behind: f32,
    /// Race progress breakpoints: early (+0x24) and late (+0x18).
    pub early_x: f32,
    pub late_x: f32,
    /// Speed factors early (+0x2c behind, +0x28 ahead) and late (+0x20 behind, +0x1c ahead).
    pub early: [f32; 2],
    pub late: [f32; 2],
}

impl RubberBand {
    /// The schedule speed (m/s) at race progress `x` (0..1, `FUN_00654110`), with `lead` = the player's remaining
    /// distance minus the AI's (positive = the AI is ahead) and `gap` the straight-line distance between the cars.
    /// Not ported: the time correction (`FUN_006433f0`: × (1 + 0.1 clamp((race time − expected time) / 30, ±1)),
    /// the expected time needs the route's segment speeds, `FUN_00661eb0`).
    pub fn speed(&self, x: f32, lead: f32, gap: f32) -> f32 {
        let t = if lead < self.behind {
            0.0
        } else if lead >= self.ahead {
            1.0
        } else {
            let s = gap.copysign(lead);
            ((s - self.behind) / (self.ahead - self.behind).max(1e-3)).clamp(0.0, 1.0)
        };
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let a = lerp(self.early[0], self.early[1], t);
        let b = lerp(self.late[0], self.late[1], t);
        let f = if x < self.early_x {
            a
        } else if x < self.late_x {
            lerp(a, b, (x - self.early_x) / (self.late_x - self.early_x).max(1e-4))
        } else {
            b
        };
        f * self.max_speed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rubber_band_blends_by_lead_and_progress() {
        let rb = RubberBand {
            max_speed: 100.0,
            ahead: 100.0,
            behind: -100.0,
            early_x: 0.0,
            late_x: 0.9,
            early: [1.0, 0.9],
            late: [0.7, 0.6],
        };
        // Far behind at the start: the early "behind" factor.
        assert!((rb.speed(0.0, -500.0, 500.0) - 100.0).abs() < 1e-3);
        // Far ahead at the end: the late "ahead" factor.
        assert!((rb.speed(1.0, 500.0, 500.0) - 60.0).abs() < 1e-3);
        // Level halfway through the race: t = 0.5, x between the breakpoints.
        let v = rb.speed(0.45, 0.0, 0.0);
        assert!((v - 100.0 * (0.95 + (0.65 - 0.95) * 0.5)).abs() < 1e-3, "{v}");
    }

    #[test]
    fn contact_time_of_closing_circles() {
        let a = OtherCar { position: Vec2::ZERO, velocity: Vec2::new(0.0, 10.0), radius: 1.0 };
        let b = OtherCar { position: Vec2::new(0.0, 22.0), velocity: Vec2::ZERO, radius: 1.0 };
        assert!((time_to_contact(&a, &b).unwrap() - 2.0).abs() < 1e-4);
        let c = OtherCar { position: Vec2::new(0.0, -22.0), velocity: Vec2::ZERO, radius: 1.0 };
        assert!(time_to_contact(&a, &c).is_none());
    }

    #[test]
    fn pid_uses_the_old_integral() {
        let mut p = Pid::default();
        let u = p.step([1.0, 1.0, 0.0], 2.0, 20.0, 0.5);
        assert_eq!(u, -2.0);
        assert_eq!(p.step([0.0, 1.0, 0.0], 2.0, 20.0, 0.5), -1.0);
    }
}
