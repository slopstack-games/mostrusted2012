//! Engine, clutch and gearbox after the game's code (docs/decomp.md, "Drivetrain and brakes").
//!
//! The engine is its own rotating inertia: the throttle blends its torque from a closed-throttle friction
//! curve to the full-load curve, and it is coupled to the driven wheels only through a clutch that matches
//! the two speeds within a torque limit. The gearbox shifts on rpm *and* road speed, through declutch /
//! neutral / clutch-in phases. Constants the game keeps outside the data are marked as model values.

use mw_assets::handling::VehicleHandling;

const RPM_PER_RAD_S: f32 = 60.0 / std::f32::consts::TAU;
/// Clutch torque capacity (N m; game constant read from the running game, `.bss` 0x01186f70).
const CLUTCH_CAP: f32 = 5000.0;
/// Launch: the clutch closes fully once its engagement reaches this (game constant).
const CLUTCH_CLOSED: f32 = 0.95;
/// Shift phases as fractions of the shift time: declutch, neutral (the rest is clutch-in); game constants
/// read from the running game (`.bss` 0x01186ee0, 0x01186fd0). Downshifts take `DOWNSHIFT_TIME` of the time.
const SHIFT_DECLUTCH: f32 = 0.1;
const SHIFT_NEUTRAL: f32 = 0.4;
const DOWNSHIFT_TIME: f32 = 0.5;
/// The downshift blip drives the engine at `Engine_resistance` × this (game constant, 0x01187518).
const BLIP_GAIN: f32 = 2.5;
/// Below this clutch engagement the engine runs free (game constant, 0x01187570).
const DECLUTCHED: f32 = 0.1;
/// Each gear's upshift rpm varies by up to this much (game constant).
const UPSHIFT_JITTER: f32 = 50.0;
/// Downshift on speed this far (mph) below the lower gear's upshift speed (game constant, 0x011872e0).
const DOWNSHIFT_HYSTERESIS: f32 = 2.0;
/// The idle governor: throttle up to `IDLE_GOVERNOR_THROTTLE` as the revs fall below idle + `IDLE_GOVERNOR_RPM`, at
/// `IDLE_GOVERNOR_GAIN` per rpm (game constants, 0x01186f30 / 0x011871c0 / 0x011873f0).
const IDLE_GOVERNOR_RPM: f32 = 20.0;
const IDLE_GOVERNOR_GAIN: f32 = 0.05;
const IDLE_GOVERNOR_THROTTLE: f32 = 0.5;

/// Engine and gearbox data for one car.
#[derive(Debug, Clone)]
pub struct DrivetrainParams {
    pub handling: VehicleHandling,
    pub idle_rpm: f32,
    pub max_rpm: f32,
    /// Closed-throttle torque (N m, negative) at idle, at `f71be7c6` (peak-torque rpm) and at `Max_RPM`.
    pub friction: [(f32, f32); 3],
    pub engine_inertia: f32,
    /// `Engine_resistance` (rad/s², negative): the free engine's deceleration with the clutch open.
    pub engine_resistance: f32,
    /// Forward ratios (1st first), their shift points and upshift speeds (mph).
    pub ratios: Vec<f32>,
    pub upshift_rpm: Vec<f32>,
    pub downshift_rpm: Vec<f32>,
    pub upshift_mph: Vec<f32>,
    pub reverse_ratio: f32,
    pub final_drive: f32,
    /// Forward gears in use (the top one is locked on many cars).
    pub gears: usize,
    pub shift_time: f32,
    /// `Power_To_Front`, axle limited-slip factors (front, rear; 0 open, 1 locked) and the
    /// `TransmissionInertia` added to each driven wheel.
    pub front_share: f32,
    pub diff: [f32; 2],
    pub transmission_inertia: f32,
}

impl DrivetrainParams {
    pub fn new(h: &VehicleHandling) -> Self {
        let e = |path: &str, default: f32| h.param(&format!("Engine.{path}")).unwrap_or(default);
        let gb = h.gearbox.clone();
        let ratios = gb
            .as_ref()
            .map(|g| g.ratios.clone())
            .filter(|r| !r.is_empty())
            .unwrap_or(vec![3.5, 2.2, 1.5, 1.15, 0.95, 0.8]);
        let locked = gb.as_ref().is_some_and(|g| g.top_gear_locked);
        let gears = if locked && ratios.len() > 1 { ratios.len() - 1 } else { ratios.len() };
        let idle_rpm = h.idle_rpm.unwrap_or(900.0);
        let max_rpm = h.redline_rpm.unwrap_or(7000.0);
        let get = |v: Option<&Vec<f32>>| v.cloned().unwrap_or_default();
        Self {
            handling: h.clone(),
            idle_rpm,
            max_rpm,
            friction: [
                (idle_rpm, e("Power_curve.b3066e01", -50.0)),
                (e("Power_curve.f71be7c6", (idle_rpm + max_rpm) / 2.0), e("Power_curve.5a6aaf9f", -100.0)),
                (max_rpm, e("Power_curve.a53d9b5d", -125.0)),
            ],
            engine_inertia: h.engine_inertia.unwrap_or(0.25).clamp(0.05, 2.0),
            engine_resistance: h.engine_brake_torque.unwrap_or(-300.0).min(-1.0),
            upshift_rpm: get(gb.as_ref().map(|g| &g.upshift_rpm)),
            downshift_rpm: get(gb.as_ref().map(|g| &g.downshift_rpm)),
            upshift_mph: get(gb.as_ref().map(|g| &g.upshift_mph)),
            reverse_ratio: gb.as_ref().map_or(3.0, |g| g.reverse.abs()),
            final_drive: gb.as_ref().map_or(3.5, |g| g.final_drive),
            ratios,
            gears,
            shift_time: h.gear_change_time.unwrap_or(0.15).clamp(0.02, 1.0),
            front_share: h.front_drive_share.unwrap_or(0.0).clamp(0.0, 1.0),
            diff: [e("Differentials.Front_Differential", 0.0), e("Differentials.Rear_Differential", 0.0)],
            transmission_inertia: e("Transmission.TransmissionInertia", 0.0).max(0.0),
        }
    }

    /// Full-load torque (N m) at `rpm`.
    pub fn full_torque(&self, rpm: f32) -> f32 {
        if self.handling.torque_curve.is_empty() {
            return 400.0;
        }
        self.handling.torque_at(rpm).max(0.0)
    }

    /// Closed-throttle torque (N m, negative) at `rpm`: linear through the three points.
    pub fn friction_torque(&self, rpm: f32) -> f32 {
        let f = &self.friction;
        let seg =
            |a: (f32, f32), b: (f32, f32)| a.1 + (b.1 - a.1) * ((rpm - a.0) / (b.0 - a.0).max(1.0)).clamp(0.0, 1.0);
        if rpm < f[1].0 { seg(f[0], f[1]) } else { seg(f[1], f[2]) }
    }

    /// Gear ratio of gear `g` (1 = first, −1 = reverse, 0 = neutral).
    pub fn ratio(&self, g: i32) -> f32 {
        match g {
            g if g > 0 => self.ratios.get(g as usize - 1).copied().unwrap_or(1.0),
            g if g < 0 => -self.reverse_ratio,
            _ => 0.0,
        }
    }

    fn upshift_rpm(&self, g: i32) -> f32 {
        self.upshift_rpm.get(g as usize - 1).copied().filter(|&r| r > 0.0).unwrap_or(self.max_rpm)
    }

    fn upshift_mph(&self, g: i32) -> f32 {
        self.upshift_mph.get(g as usize - 1).copied().unwrap_or(0.0)
    }
}

#[derive(Debug, Clone, Copy)]
struct Shift {
    target: i32,
    time: f32,
    total: f32,
    up: bool,
    committed: bool,
}

/// Engine and gearbox state.
#[derive(Debug, Clone)]
pub struct Drivetrain {
    pub gear: i32,
    /// Engine speed (rpm).
    pub rpm: f32,
    /// Clutch engagement (0 open, 1 closed).
    pub clutch: f32,
    /// The clutch is slipping to pull away (from standstill or after a stall).
    launching: bool,
    shift: Option<Shift>,
    /// Throttle after the gearbox (cut while shifting), for the engine sound.
    pub throttle: f32,
    seed: u32,
    /// This gear's upshift rpm offset (the game randomises it by ±50 rpm per gear).
    up_jitter: f32,
}

impl Drivetrain {
    pub fn new(p: &DrivetrainParams, gear: i32) -> Self {
        Self {
            gear,
            rpm: p.idle_rpm,
            clutch: 1.0,
            launching: gear == 1,
            shift: None,
            throttle: 0.0,
            seed: 0x9e37,
            up_jitter: 0.0,
        }
    }

    fn random(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    /// One tick. `wheel_omega` is the mean spin of the driven wheels (rad/s), `mph` the road speed along the
    /// car (signed); `inv_inertia` the driven wheels' mean inverse inertia; `coupled` false when no wheel is on
    /// the driveline (handbrake on a rear-driven car): the engine runs free. Returns the torque (N m) to share
    /// between the driven wheels.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        p: &DrivetrainParams,
        throttle: f32,
        wheel_omega: f32,
        mph: f32,
        inv_inertia: f32,
        coupled: bool,
        dt: f32,
    ) -> f32 {
        self.select_gear(p, throttle, mph);
        let mut thr = throttle;
        let mut declutched = false;
        if let Some(s) = &mut self.shift {
            s.time += dt;
            let f = s.time / s.total;
            self.clutch = if f < SHIFT_DECLUTCH {
                1.0 - f / SHIFT_DECLUTCH
            } else if f < SHIFT_DECLUTCH + SHIFT_NEUTRAL {
                0.0
            } else {
                ((f - SHIFT_DECLUTCH - SHIFT_NEUTRAL) / (1.0 - SHIFT_DECLUTCH - SHIFT_NEUTRAL)).min(1.0)
            };
            if !s.committed && f >= SHIFT_DECLUTCH + SHIFT_NEUTRAL {
                s.committed = true;
                self.gear = s.target;
                self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                self.up_jitter = ((self.seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 2.0 * UPSHIFT_JITTER;
            }
            let up = s.up;
            if f < SHIFT_DECLUTCH + SHIFT_NEUTRAL {
                // Upshifts cut the throttle; downshifts blip the engine (below).
                thr = 0.0;
                declutched = up;
            }
            if f >= 1.0 {
                self.shift = None;
                self.clutch = 1.0;
            }
        }
        let ratio = p.ratio(self.gear) * p.final_drive;
        let shaft_rpm = wheel_omega * ratio * RPM_PER_RAD_S;
        // Pulling away: the clutch engages with the throttle or the revs, and closes at 0.95.
        if self.shift.is_none() {
            if self.launching {
                let by_rpm = self.rpm / p.upshift_rpm(1).max(p.idle_rpm + 1.0);
                self.clutch = self.clutch.max(throttle).max(by_rpm.clamp(0.0, 1.0));
                if throttle <= 0.0 && shaft_rpm.abs() < p.idle_rpm {
                    self.clutch = 0.0;
                }
                if self.clutch >= CLUTCH_CLOSED && self.rpm > p.idle_rpm {
                    self.clutch = 1.0;
                    self.launching = false;
                }
            } else if (throttle <= 0.0 && shaft_rpm.abs() < p.idle_rpm) || self.rpm < p.idle_rpm {
                // Stopped with no throttle, or the revs fell below idle: open the clutch and pull away again
                // (the game's standstill state).
                self.launching = true;
                self.clutch = 0.0;
            }
        }
        self.throttle = thr;

        // Engine torque: the idle governor opens the throttle near idle (and fades the friction out).
        let governor = ((p.idle_rpm + IDLE_GOVERNOR_RPM - self.rpm) * IDLE_GOVERNOR_GAIN).clamp(0.0, 1.0);
        let thr = thr.max(governor * IDLE_GOVERNOR_THROTTLE);
        let friction = p.friction_torque(self.rpm) * (1.0 - governor);
        // Downshifts rev-match: while the clutch is open the engine is blipped up to the new gear's speed.
        let target_rpm =
            self.shift.map_or(0.0, |s| wheel_omega.abs() * (p.ratio(s.target) * p.final_drive).abs() * RPM_PER_RAD_S);
        let downshift_blip = self.shift.is_some_and(|s| !s.up) && self.clutch == 0.0 && self.rpm < target_rpm;
        let net = if downshift_blip {
            -p.engine_resistance * p.engine_inertia * BLIP_GAIN
        } else if declutched || self.clutch < DECLUTCHED || !coupled {
            p.engine_resistance * p.engine_inertia * (1.0 - governor)
        } else {
            friction + thr * (p.full_torque(self.rpm) - friction)
        };
        // The game clamps the revs at `Max_RPM` before integrating (so this tick's torque still reaches the
        // clutch).
        let mut omega = self.rpm.min(p.max_rpm) / RPM_PER_RAD_S + net * dt / p.engine_inertia;

        // Clutch: match the engine and the driveline within the torque capacity.
        let mut torque = 0.0;
        if ratio != 0.0 && self.clutch > 0.0 && coupled {
            let delta = omega - wheel_omega * ratio;
            let inv_eff = 1.0 / p.engine_inertia + ratio * ratio * 0.5 * inv_inertia;
            let t_c = (delta.abs() / inv_eff / dt * self.clutch).min(CLUTCH_CAP) * delta.signum();
            omega -= t_c * dt / p.engine_inertia;
            torque = t_c * ratio;
        }
        self.rpm = (omega * RPM_PER_RAD_S).max(0.0);
        torque
    }

    fn select_gear(&mut self, p: &DrivetrainParams, throttle: f32, mph: f32) {
        if self.shift.is_some() || self.launching || self.gear <= 0 {
            return;
        }
        let g = self.gear;
        let max_gear = p.gears as i32;
        if g < max_gear && self.rpm > p.upshift_rpm(g) + self.up_jitter && mph > p.upshift_mph(g) {
            // May skip gears: the highest one whose entry speed is already passed.
            let target = (g + 1..=max_gear).filter(|&k| p.upshift_mph(k - 1) < mph).max().unwrap_or(g + 1);
            self.start_shift(p, target, true);
            return;
        }
        if g > 1 {
            let down_rpm = p.downshift_rpm.get(g as usize - 1).copied().unwrap_or(0.0);
            let slow = mph < p.upshift_mph(g - 1) - DOWNSHIFT_HYSTERESIS;
            if slow || (throttle > 0.1 && self.rpm < down_rpm) {
                let shaft = self.rpm / p.ratio(g);
                let target = (1..g)
                    .rev()
                    .find(|&k| {
                        let rpm = shaft * p.ratio(k);
                        rpm > p.downshift_rpm.get(k as usize - 1).copied().unwrap_or(0.0) && rpm < p.max_rpm * 0.97
                    })
                    .unwrap_or(if slow { g - 1 } else { g });
                if target < g {
                    self.start_shift(p, target, false);
                }
            }
        }
    }

    fn start_shift(&mut self, p: &DrivetrainParams, target: i32, up: bool) {
        let total = p.shift_time * (1.0 + 0.15 * self.random()) * if up { 1.0 } else { DOWNSHIFT_TIME };
        self.shift = Some(Shift { target, time: 0.0, total, up, committed: false });
    }

    /// Change straight into `gear` (reverse engagement, a reset): no shift phases, the clutch pulls away.
    pub fn engage(&mut self, gear: i32) {
        self.gear = gear;
        self.shift = None;
        self.launching = true;
        self.clutch = 0.0;
    }
}
