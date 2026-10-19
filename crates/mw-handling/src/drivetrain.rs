//! Engine, clutch and gearbox after the game's code (docs/decomp.md, "Drivetrain and brakes").
//!
//! The engine is its own rotating inertia (`FUN_009d9960`): an idle governor, a peak-held and lagged throttle,
//! a closed-throttle friction curve blended to the full-load curve, and a small torque noise. It is coupled to
//! the driven wheels only through a clutch (`FUN_009d7ac0`) that matches the two speeds within a torque limit,
//! once per tick before the tyres; as the tyres then hold the wheels at road speed, the clutch slips for as long
//! as the engine has torque to spare (the original's engine runs well above the wheels in the low gears). The
//! gearbox is the state machine of `FUN_009da340`: standstill, launch, in gear, and declutch / neutral /
//! clutch-in phases for each shift. Constants the game keeps outside the data are read from the running game
//! (`.bss`, docs/decomp.md "Reading the dump") or from `NFS13.exe` (`.rdata` / `.data`); the few still unknown
//! are marked as model values.

use mw_assets::handling::VehicleHandling;

const RPM_PER_RAD_S: f32 = 60.0 / std::f32::consts::TAU;
/// Clutch torque capacity (N m; `.bss` 0x01186f70, read from the running game).
const CLUTCH_CAP: f32 = 5000.0;
/// Launch: the clutch closes fully once its engagement reaches this (`.bss` 0x01187080, 0.95 in the dump).
const CLUTCH_CLOSED: f32 = 0.95;
/// Shift phases as fractions of the shift time (`.bss` 0x01186ee0 = 0.1, 0x01186fd0 = 0.4): declutch first,
/// then neutral for the rest (`1 − 0.1 − 0.4`), then clutch-in. The gear changes at the end of neutral.
const SHIFT_DECLUTCH: f32 = 0.1;
const SHIFT_CLUTCH_IN: f32 = 0.4;
const SHIFT_NEUTRAL: f32 = 1.0 - SHIFT_DECLUTCH - SHIFT_CLUTCH_IN;
/// Downshifts take this fraction of the shift time (`.bss` 0x01186fa0).
const DOWNSHIFT_TIME: f32 = 0.5;
/// Shift time varies by up to this fraction (`.data` 0x010cfa64).
const SHIFT_TIME_JITTER: f32 = 0.15;
/// The launch clutch engages with the revs towards each gear's upshift rpm, randomised by ± this (rpm; `.data`
/// 0x010cfa68, `FUN_009dc4e0`).
const UPSHIFT_RPM_JITTER: f32 = 50.0;
/// During a downshift the engine is driven at −`Engine_resistance` × this (`.bss` 0x01187518).
const BLIP_GAIN: f32 = 2.5;
/// Below this clutch engagement the engine runs free (`.bss` 0x01187570).
const DECLUTCHED: f32 = 0.1;
/// Gearbox mode (`FUN_009ce630`): the cruising gearbox takes over after `CRUISE_ENTER_TIME` s with the pedal
/// between 0 and this (exclusive) and no brake, and hands back after `CRUISE_EXIT_TIME` s above it
/// (`.rdata` 0x01026160, 0x01026180, 0x01026170).
const CRUISE_THROTTLE: f32 = 0.95;
const CRUISE_ENTER_TIME: f32 = 3.0;
const CRUISE_EXIT_TIME: f32 = 0.05;
/// After a switch back to the race gearbox, shifts take the cruising gearbox's `Gear_change_time` for this
/// long (`FUN_009dc3c0`, `.bss` 0x011871e0 = 0.2 s, read live 2026-10-06).
const KICKDOWN_WINDOW: f32 = 0.2;
/// Downshift on speed this far (mph) below the lower gear's upshift speed (`.bss` 0x011872e0).
const DOWNSHIFT_HYSTERESIS: f32 = 2.0;
/// Downshifts wait until the car has been on the ground this long (`FUN_009d93b0`: vehicle +0x1ed0 against
/// `.bss` 0x01187580 = 1.0 s, read live 2026-10-06).
const DOWNSHIFT_GROUNDED_TIME: f32 = 1.0;
/// In gear with neither pedal pressed, the clutch opens below this speed (mph; `.bss` 0x011872d0, inferred
/// from its lanes 2-3 read live).
const COAST_DECLUTCH_MPH: f32 = 20.0;
/// The idle governor: throttle `IDLE_GOVERNOR_THROTTLE` × g with g = (idle + `IDLE_GOVERNOR_RPM` − rpm) ×
/// `IDLE_GOVERNOR_GAIN` clamped to 0..1; the closed-throttle torque fades out with g (`.bss` 0x01186f30,
/// 0x011871c0, 0x011873f0).
const IDLE_GOVERNOR_RPM: f32 = 20.0;
const IDLE_GOVERNOR_GAIN: f32 = 0.05;
const IDLE_GOVERNOR_THROTTLE: f32 = 0.5;
/// Time base of the throttle's peak-hold decay and lag (`Engine.c6deb7a6` / `36676b9c` are per-tick fractions;
/// `.bss` 0x01187250 = 60, read live 2026-10-06); with it the idle governor reproduces the original's idle
/// (1003–1031 rpm, mean 1016, against 1002–1033, ~1015).
const THROTTLE_TIME_BASE: f32 = 60.0;
/// Clutch-in: the throttle comes back as `CLUTCH_IN_THROTTLE_SCALE` × progress^this (`FUN_009d6af0` up,
/// `FUN_009d6ca0` down, the exponent passed to the CRT `pow` from `.bss` 0x01187470 = 0.5, read live 2026-10-06).
const CLUTCH_IN_THROTTLE_EXP: f32 = 0.5;
/// Scale of the clutch-in throttle (`.bss` 0x011874d0 = 0.7, read live 2026-10-06). The upshift's clutch-in sets
/// the throttle to 0.7 √progress whatever the pedal; the downshift's multiplies the pedal by it.
const CLUTCH_IN_THROTTLE_SCALE: f32 = 0.7;
/// The gearbox stalls to standstill below `Idle_RPM` × this during a launch or a shift (`.bss` 0x011874a0 = 0.9,
/// read live 2026-10-06).
const STALL_FRACTION: f32 = 0.9;
/// The torque noise is sampled at the firing rate of one cylinder: rpm / 120 lattice points per second.
const NOISE_RPM_DIV: f32 = 120.0;

/// Engine and gearbox data for one car.
#[derive(Debug, Clone)]
pub struct DrivetrainParams {
    pub handling: VehicleHandling,
    pub idle_rpm: f32,
    pub max_rpm: f32,
    /// Closed-throttle torque (N m, negative) at idle, at `f71be7c6` (peak-torque rpm) and at `Max_RPM`.
    pub friction: [(f32, f32); 3],
    /// Torque noise amplitude (N m) at the same three rpm (`Power_curve.15d1d117`, `d857ea54`, `ddfc5326`),
    /// scaled by `noise_load` = (no load, full load) (`455b2e4a`, `dc7b0a4b`).
    pub noise: [(f32, f32); 3],
    pub noise_load: (f32, f32),
    pub engine_inertia: f32,
    /// `Engine_resistance` (rad/s², negative): the free engine's deceleration with the clutch open.
    pub engine_resistance: f32,
    /// The throttle's peak-hold decay and lag, per tick (`Engine.c6deb7a6`, `Engine.36676b9c`).
    pub throttle_decay: f32,
    pub throttle_lag: f32,
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
    /// The same engine with the `CruisingGearbox` (its ratios, shift points and `Gear_change_time`).
    pub cruising: Option<Box<DrivetrainParams>>,
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
        let mid_rpm = e("Power_curve.f71be7c6", (idle_rpm + max_rpm) / 2.0);
        let get = |v: Option<&Vec<f32>>| v.cloned().unwrap_or_default();
        let mut race = Self {
            handling: h.clone(),
            idle_rpm,
            max_rpm,
            friction: [
                (idle_rpm, e("Power_curve.b3066e01", -50.0)),
                (mid_rpm, e("Power_curve.5a6aaf9f", -100.0)),
                (max_rpm, e("Power_curve.a53d9b5d", -125.0)),
            ],
            noise: [
                (idle_rpm, e("Power_curve.15d1d117", 0.0)),
                (mid_rpm, e("Power_curve.d857ea54", 0.0)),
                (max_rpm, e("Power_curve.ddfc5326", 0.0)),
            ],
            noise_load: (e("Power_curve.455b2e4a", 0.0), e("Power_curve.dc7b0a4b", 0.0)),
            engine_inertia: h.engine_inertia.unwrap_or(0.25).clamp(0.05, 2.0),
            engine_resistance: h.engine_brake_torque.unwrap_or(-300.0).min(-1.0),
            throttle_decay: e("c6deb7a6", 0.09).max(0.0),
            throttle_lag: e("36676b9c", 0.15).max(0.0),
            cruising: None,
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
        };
        if let Some(c) = h.cruising_gearbox.as_ref().filter(|c| !c.ratios.is_empty()) {
            let mut cruise = race.clone();
            cruise.ratios = c.ratios.clone();
            cruise.upshift_rpm = c.upshift_rpm.clone();
            cruise.downshift_rpm = c.downshift_rpm.clone();
            cruise.upshift_mph = c.upshift_mph.clone();
            cruise.reverse_ratio = c.reverse.abs();
            cruise.final_drive = c.final_drive;
            cruise.gears = if c.top_gear_locked && c.ratios.len() > 1 { c.ratios.len() - 1 } else { c.ratios.len() };
            cruise.shift_time = c.change_time.unwrap_or(race.shift_time).clamp(0.02, 2.0);
            race.cruising = Some(Box::new(cruise));
        }
        race
    }

    /// The gearbox in use: the cruising one when `cruising` and the car has one.
    pub fn active(&self, cruising: bool) -> &DrivetrainParams {
        if cruising { self.cruising.as_deref().unwrap_or(self) } else { self }
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
        three_point(&self.friction, rpm)
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

    fn downshift_rpm(&self, g: i32) -> f32 {
        self.downshift_rpm.get(g as usize - 1).copied().unwrap_or(0.0)
    }
}

/// A curve linear through three (x, y) points, flat outside.
fn three_point(p: &[(f32, f32); 3], x: f32) -> f32 {
    let seg = |a: (f32, f32), b: (f32, f32)| a.1 + (b.1 - a.1) * ((x - a.0) / (b.0 - a.0).max(1.0)).clamp(0.0, 1.0);
    if x < p[1].0 { seg(p[0], p[1]) } else { seg(p[1], p[2]) }
}

/// The engine's torque noise (`FUN_009e3780`): value noise on an integer lattice, cosine-interpolated, in −1..1.
fn noise(x: f32) -> f32 {
    let lattice = |i: u32| {
        let n = (i << 13) ^ i;
        let h = n.wrapping_mul(n.wrapping_mul(n).wrapping_mul(15731).wrapping_add(789_221)).wrapping_add(1_376_312_589)
            & 0x7fff_ffff;
        1.0 - h as f32 / 1_073_741_824.0
    };
    let x = x.max(0.0);
    let i = x.floor();
    let w = (1.0 - ((x - i) * std::f32::consts::PI).cos()) * 0.5;
    let i = i as u32;
    lattice(i) * (1.0 - w) + lattice(i.wrapping_add(1)) * w
}

/// A gear change in progress (`FUN_009d1560` up, `FUN_009d16f0` down).
#[derive(Debug, Clone, Copy)]
struct Shift {
    target: i32,
    up: bool,
    phase: Phase,
    /// The shift's total time and the time left in the current phase (s).
    total: f32,
    left: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Declutch,
    Neutral,
    ClutchIn,
}

impl Shift {
    fn length(&self, phase: Phase) -> f32 {
        self.total
            * match phase {
                Phase::Declutch => SHIFT_DECLUTCH,
                Phase::Neutral => SHIFT_NEUTRAL,
                Phase::ClutchIn => SHIFT_CLUTCH_IN,
            }
    }
}

/// Engine and gearbox state.
#[derive(Debug, Clone)]
pub struct Drivetrain {
    pub gear: i32,
    /// Engine speed (rpm).
    pub rpm: f32,
    /// Clutch engagement (0 open, 1 closed).
    pub clutch: f32,
    /// Time since a wheel last touched the ground (vehicle +0x1ed0, `FUN_009ae840`), set by the caller: the
    /// downshift gate.
    pub grounded_time: f32,
    /// Standing or pulling away: the clutch slips in with the throttle or the revs (the game's standstill and
    /// launch states).
    launching: bool,
    shift: Option<Shift>,
    /// The engine's effective throttle (after the gearbox's cuts, the peak hold and the lag), for the sound.
    pub throttle: f32,
    /// The pedal after the gearbox's cuts (vehicle +0x1aa0, before the engine's hold and lag): the exhaust
    /// pops read it (`FUN_009b20b0`).
    pub gearbox_throttle: f32,
    /// Peak-held throttle and lagged throttle (`FUN_009d9960`, engine +0x28 / +0x24).
    hold: f32,
    lag: f32,
    /// Time accumulator of the torque noise (engine +0x30).
    noise_time: f32,
    seed: u32,
    /// The current gear's upshift rpm with its random offset (gearbox +0x130), for the launch clutch.
    up_rpm: f32,
    /// The cruising gearbox is in use (vehicle +0x1bdc = 1), the qualifying time towards a switch
    /// (gearbox +0x140) and the time since the last switch (+0x144).
    pub cruising: bool,
    mode_time: f32,
    since_mode: f32,
    /// Nitrous (nitrous.rs), set by the caller each tick: the engine torque multiplier (`FUN_009d2aa0`'s
    /// component +0x10) and whether it burns (`FUN_009d4c20`: upshift on rpm alone, top gear unlocked).
    pub nitrous_scale: f32,
    pub nitrous: bool,
    /// The gear-change clock the engine sound reads (`GetPlayerNormalisedGearChangeTimeElapsed`,
    /// `FUN_00911650`: gearbox +0xb4 / +0xb0): the time since the last shift started, counted every tick
    /// (`FUN_009da340`) and reset to 0 when a shift starts (`FUN_009d1560` up, `FUN_009d16f0` down), and that
    /// shift's time (its `total`, halved for a downshift).
    pub shift_elapsed: f32,
    pub shift_time: f32,
    /// Shifts started so far and whether the last one was an upshift (gearbox +0xd0 / +0xd4 flag the start
    /// tick: the sound starts the gear-change sequence then, `FUN_0058ca50`).
    pub shifts_started: u32,
    pub last_shift_up: bool,
}

impl Drivetrain {
    pub fn new(p: &DrivetrainParams, gear: i32) -> Self {
        let launching = gear == 1;
        let mut d = Self {
            cruising: false,
            mode_time: 0.0,
            // Long since any switch: a car's first shifts use the race box's own change time.
            since_mode: f32::MAX,
            gear,
            rpm: p.idle_rpm,
            clutch: if launching { 0.0 } else { 1.0 },
            grounded_time: 1.0,
            launching,
            shift: None,
            throttle: 0.0,
            gearbox_throttle: 0.0,
            hold: 0.0,
            lag: 0.0,
            noise_time: 0.0,
            seed: 0x9e37,
            nitrous_scale: 1.0,
            nitrous: false,
            up_rpm: p.max_rpm,
            shift_elapsed: f32::MAX,
            shift_time: 0.0,
            shifts_started: 0,
            last_shift_up: true,
        };
        d.set_up_rpm(p);
        d
    }

    fn random(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    /// `FUN_009dc4e0`: the gear's upshift rpm ± `UPSHIFT_RPM_JITTER`, kept for the launch clutch.
    fn set_up_rpm(&mut self, p: &DrivetrainParams) {
        let r = self.random();
        self.up_rpm = p.upshift_rpm(self.gear.max(1)) + (r * 2.0 - 1.0) * UPSHIFT_RPM_JITTER;
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
        brake: f32,
        wheel_omega: f32,
        mph: f32,
        inv_inertia: f32,
        coupled: bool,
        dt: f32,
    ) -> f32 {
        let race = p;
        let p = race.active(self.cruising);
        // `FUN_009da340` counts the gear-change clock before running the state (a shift that starts now
        // resets it to 0).
        self.shift_elapsed += dt;
        let thr = self.gearbox_state(p, throttle, brake, mph, dt);
        self.gearbox_throttle = thr;
        self.gearbox_mode(race, throttle, brake, dt);
        let (up, down) = match self.shift {
            Some(s) => (s.up, !s.up),
            None => (false, false),
        };
        let ratio = p.ratio(self.gear) * p.final_drive;

        // --- Engine (FUN_009d9960) -----------------------------------------------------------------------
        // The idle governor opens the throttle near idle and fades the closed-throttle torque out.
        let governor = ((p.idle_rpm + IDLE_GOVERNOR_RPM - self.rpm) * IDLE_GOVERNOR_GAIN).clamp(0.0, 1.0);
        let thr = thr.max(governor * IDLE_GOVERNOR_THROTTLE);
        // Clutch open (or an upshift under way): the free engine's resistance instead of the friction curve.
        let declutched = up || (self.clutch < DECLUTCHED && !down) || !coupled;
        let closed = if declutched { p.engine_resistance * p.engine_inertia } else { p.friction_torque(self.rpm) };
        let base = closed * (1.0 - governor);
        // The throttle is peak-held (decaying) and lagged; an upshift sets both directly.
        let k = THROTTLE_TIME_BASE * dt;
        self.hold = (self.hold - p.throttle_decay * k).max(thr);
        self.lag += (p.throttle_lag * k).clamp(0.0, 1.0) * (self.hold - self.lag);
        if up {
            self.hold = thr;
            self.lag = thr;
        }
        let effective = self.lag.clamp(0.0, 1.0);
        let load = self.throttle;
        self.throttle = effective;
        // Downshifts blip the engine for the whole shift.
        let mut net = if down {
            -p.engine_resistance * p.engine_inertia * BLIP_GAIN
        } else {
            base + effective * (p.full_torque(self.rpm) * self.nitrous_scale - base)
        };
        // The game clamps the revs at `Max_RPM` after the torque is known (so this tick's torque still reaches
        // the clutch), then adds the noise sampled at rpm / 120 lattice points per second.
        let rpm = self.rpm.min(p.max_rpm);
        let per = rpm / NOISE_RPM_DIV;
        self.noise_time += dt;
        if self.noise_time > per {
            self.noise_time -= per;
        }
        let amplitude = three_point(&p.noise, self.rpm) * (p.noise_load.0 + (p.noise_load.1 - p.noise_load.0) * load);
        net += noise(self.noise_time * per) * amplitude;
        let mut omega = rpm / RPM_PER_RAD_S + net * dt / p.engine_inertia;

        // --- Clutch (FUN_009d7ac0): match the engine and the driveline within the torque capacity --------------
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

    /// `FUN_009ce630` (run by `FUN_009da600` after the pedals are copied) and `FUN_00900660` (which applies the
    /// mode next tick: reloads the gearbox tables and, back in the race box, clamps the gear to its last one).
    /// Part throttle (0 < pedal < 0.95, no brake) held for 3 s selects the cruising gearbox; the pedal above
    /// 0.95 for 0.05 s selects the race one again. The game also keeps the race box while an unidentified flag
    /// is up (a virtual call on the vehicle in `FUN_009b3030`); assumed down here.
    fn gearbox_mode(&mut self, race: &DrivetrainParams, throttle: f32, brake: f32, dt: f32) {
        if race.cruising.is_none() {
            return;
        }
        self.since_mode += dt;
        let qualifies = if self.cruising {
            throttle > CRUISE_THROTTLE
        } else {
            throttle > 0.0 && throttle < CRUISE_THROTTLE && brake <= 0.0
        };
        self.mode_time = if qualifies { self.mode_time + dt } else { 0.0 };
        let limit = if self.cruising { CRUISE_EXIT_TIME } else { CRUISE_ENTER_TIME };
        if self.mode_time > limit {
            self.cruising = !self.cruising;
            self.since_mode = 0.0;
            let top = race.active(self.cruising).ratios.len() as i32;
            if self.gear > top {
                self.gear = top;
            }
        }
    }

    /// The gearbox state machine (`FUN_009da340`) for the gearbox in use: sets the clutch and the gear, returns
    /// the throttle after the gearbox's cuts.
    fn gearbox_state(&mut self, p: &DrivetrainParams, throttle: f32, brake: f32, mph: f32, dt: f32) -> f32 {
        // Stall check (`FUN_009d17d0` launch; `FUN_009d6880`, `FUN_009d95a0`, `FUN_009d6af0` upshift phases;
        // `FUN_009d69e0`, `FUN_009d6ca0` downshift neutral / clutch-in; `FUN_009da1d0` after a downshift; not the
        // downshift's declutch `FUN_009ce490`): revs below `Idle_RPM` × `.bss` 0x011874a0 drop the gearbox to
        // standstill (state 2, clutch open, first gear).
        let stalling = self.rpm < p.idle_rpm * STALL_FRACTION;
        if let Some(mut s) = self.shift {
            if stalling && !(!s.up && s.phase == Phase::Declutch) {
                self.drop_to_standstill(p);
                return throttle;
            }
            return self.shift_phase(p, &mut s, throttle, mph, dt);
        }
        if self.launching && stalling {
            self.drop_to_standstill(p);
            return throttle;
        }
        if self.launching {
            // Launch (FUN_009d17d0): with no throttle the clutch stays open; otherwise it engages with the
            // throttle or with the revs towards the gear's upshift rpm, and closes once that reaches 0.95.
            if throttle <= 0.0 {
                self.clutch = 0.0;
                return throttle;
            }
            let by_rpm = (self.rpm / self.up_rpm.max(1.0)).clamp(0.0, 1.0);
            let c = throttle.max(self.clutch).max(by_rpm);
            if c >= CLUTCH_CLOSED && self.rpm > p.idle_rpm {
                self.clutch = 1.0;
                self.launching = false;
            } else {
                self.clutch = c;
            }
            return throttle;
        }
        if self.gear <= 0 {
            return throttle;
        }
        // In gear (FUN_009d9ee0): below idle, or coasting slowly with neither pedal, the clutch opens.
        if self.rpm < p.idle_rpm || (throttle <= 0.0 && brake <= 0.0 && mph.abs() < COAST_DECLUTCH_MPH) {
            self.clutch = 0.0;
            self.launching = true;
            self.gear = 1;
            self.set_up_rpm(p);
            return throttle;
        }
        let g = self.gear;
        // Upshift on rpm and speed; nitrous waives the speed condition and unlocks the top gear (FUN_009d4c20).
        let top = if self.nitrous { p.ratios.len() as i32 } else { p.gears as i32 };
        if g < top && self.rpm > p.upshift_rpm(g) && (mph > p.upshift_mph(g) || self.nitrous) {
            let target = self.upshift_target(p, mph);
            self.start_shift(p, target, true);
            return throttle;
        }
        // Downshift (FUN_009d93b0) on speed (below the lower gear's upshift speed, with hysteresis) or rpm,
        // to the highest lower gear whose downshift rpm the revs would still be above.
        // Only once the car has been on the ground for longer than `.bss` 0x01187580 (1 s).
        if g > 1
            && self.grounded_time > DOWNSHIFT_GROUNDED_TIME
            && (mph < p.upshift_mph(g - 1) - DOWNSHIFT_HYSTERESIS || self.rpm < p.downshift_rpm(g))
        {
            let target = self.downshift_target(p);
            if target >= 1 && target != g {
                self.start_shift(p, target, false);
            }
        }
        throttle
    }

    /// Gearbox state 2 (standstill, `FUN_009d6370`): clutch open in first gear; the launch follows with throttle.
    fn drop_to_standstill(&mut self, p: &DrivetrainParams) {
        self.shift = None;
        self.clutch = 0.0;
        self.launching = true;
        self.gear = 1;
        self.set_up_rpm(p);
    }

    /// One tick of a shift's phases (upshift: FUN_009d6880, FUN_009d95a0, FUN_009d6af0; downshift:
    /// FUN_009ce490, FUN_009d69e0, FUN_009d6ca0). Each phase counts its time down and acts on the tick it ends.
    fn shift_phase(&mut self, p: &DrivetrainParams, s: &mut Shift, throttle: f32, mph: f32, dt: f32) -> f32 {
        s.left -= dt;
        let len = s.length(s.phase);
        let mut thr = throttle;
        match s.phase {
            Phase::Declutch => {
                // Upshifts cut the throttle from here to the clutch-in.
                if s.up {
                    thr = 0.0;
                }
                if s.left > 0.0 {
                    self.clutch = s.left / len;
                } else {
                    self.clutch = 0.0;
                    s.phase = Phase::Neutral;
                    s.left = s.length(Phase::Neutral);
                }
            }
            Phase::Neutral => {
                thr = 0.0;
                if s.left <= 0.0 {
                    // The upshift's target is chosen again for the speed now.
                    self.gear = if s.up && s.target > self.gear { self.upshift_target(p, mph) } else { s.target };
                    self.set_up_rpm(p);
                    s.phase = Phase::ClutchIn;
                    s.left = s.length(Phase::ClutchIn);
                }
            }
            Phase::ClutchIn => {
                let progress = 1.0 - s.left / len;
                if progress < 1.0 {
                    self.clutch = progress;
                    let ramp = CLUTCH_IN_THROTTLE_SCALE * progress.max(0.0).powf(CLUTCH_IN_THROTTLE_EXP);
                    thr = if s.up { ramp } else { throttle * ramp };
                } else {
                    self.clutch = 1.0;
                    self.shift = None;
                    return thr;
                }
            }
        }
        self.shift = Some(*s);
        thr
    }

    /// `FUN_009d4c80`: the next gear up, or a higher one whose entry speed is already passed.
    fn upshift_target(&self, p: &DrivetrainParams, mph: f32) -> i32 {
        (self.gear + 1..=p.ratios.len() as i32).filter(|&k| p.upshift_mph(k - 1) < mph).max().unwrap_or(self.gear + 1)
    }

    /// `FUN_009d8d60`: the highest lower gear in which the revs would be above its downshift rpm (0 if none).
    fn downshift_target(&self, p: &DrivetrainParams) -> i32 {
        let shaft = self.rpm / p.ratio(self.gear);
        (1..self.gear).filter(|&k| p.downshift_rpm(k) < shaft * p.ratio(k)).max().unwrap_or(0)
    }

    fn start_shift(&mut self, p: &DrivetrainParams, target: i32, up: bool) {
        // FUN_009dc3c0: in the race box shortly after a kick-down, the cruising box's (slower) change time.
        let base = match p.cruising.as_deref() {
            Some(c) if !self.cruising && self.since_mode < KICKDOWN_WINDOW => c.shift_time,
            _ => p.shift_time,
        };
        let total = base * (1.0 + SHIFT_TIME_JITTER * self.random()) * if up { 1.0 } else { DOWNSHIFT_TIME };
        let mut s = Shift { target, up, phase: Phase::Declutch, total, left: 0.0 };
        s.left = s.length(Phase::Declutch);
        self.shift = Some(s);
        self.shift_elapsed = 0.0;
        self.shift_time = total;
        self.shifts_started = self.shifts_started.wrapping_add(1);
        self.last_shift_up = up;
    }

    /// `GetPlayerNormalisedGearChangeTimeElapsed` (`FUN_00911650`): the time since the last shift started over
    /// that shift's time, at least 0 (large before the first shift).
    pub fn gear_change_elapsed(&self) -> f32 {
        if self.shift_time > 0.0 { (self.shift_elapsed / self.shift_time).max(0.0) } else { f32::MAX }
    }

    /// The gear is one of the gearbox's last two (`FUN_00911690`: gear index = count − 1 or count − 2, reverse
    /// being index 0): the engine sound's rev limiter only plays then.
    pub fn in_top_gears(&self, p: &DrivetrainParams) -> bool {
        let n = p.active(self.cruising).ratios.len() as i32;
        self.gear > 0 && (self.gear == n || self.gear == n - 1)
    }

    /// Change straight into `gear` (reverse engagement, a reset): no shift phases, the clutch pulls away.
    pub fn engage(&mut self, gear: i32) {
        self.gear = gear;
        self.shift = None;
        self.launching = true;
        self.clutch = 0.0;
    }

    /// Puts the car in `gear` while rolling, clutch closed (`--replay` starts in the recording's gear).
    pub fn put_in_gear(&mut self, gear: i32) {
        self.gear = gear;
        self.shift = None;
        self.launching = false;
        self.clutch = 1.0;
    }

    /// The game's placement of a moving car (`FUN_008f8190` with the moving flag, from the placement message
    /// `FUN_00596050` → `FUN_00580f90`; the trigger teleport's `FUN_00647e00` with a speed). Its reset
    /// `FUN_009a9b10` (through vtable +0x58) puts the gearbox in standstill (`FUN_009cd620`: state 2, the shift
    /// cleared, its clock +0xb4 / +0xb0 at 0 / 1), the gear index to 1 and the driveline's speed to `Idle_RPM`
    /// (`FUN_009d4be0`), the clutch to 0 (`FUN_009cd150`) and the rpm to `Idle_RPM` (`FUN_009d22f0`; the throttle
    /// hold and lag are kept). Then `FUN_009b3550`: the driveline's speed from the wheels (`FUN_009d55f0`, the
    /// wheels already spinning at road speed) times **first gear's** ratio (the gear index the reset left), the
    /// gear chosen by speed alone (`FUN_009d78e0`: the highest usable gear whose lower neighbour's
    /// `Gear_up_speed` is passed, reverse below 0 mph), and at least `Idle_RPM` of it: the rpm and the gearbox in
    /// gear with the clutch closed (`FUN_009cd710`), else idle in standstill with the clutch open (`FUN_009cd750`).
    /// The rpm is not clamped here (the 911 at 62 mph starts at 10 402 rpm in third, `ref/kerb100a`; the engine
    /// clamps it at `Max_RPM` on its next tick). `wheel_omega` is the driven wheels' mean spin as [`Self::step`]
    /// takes it.
    pub fn place_moving(&mut self, p: &DrivetrainParams, wheel_omega: f32, mph: f32) {
        let p = p.active(self.cruising);
        self.shift = None;
        self.shift_elapsed = 0.0;
        self.shift_time = 1.0;
        let rpm = wheel_omega * p.ratio(1) * p.final_drive * RPM_PER_RAD_S;
        let top = if self.nitrous { p.ratios.len() as i32 } else { p.gears as i32 };
        self.gear = if mph < 0.0 { -1 } else { (2..=top).filter(|&k| p.upshift_mph(k - 1) < mph).max().unwrap_or(1) };
        if rpm >= p.idle_rpm {
            self.rpm = rpm;
            self.clutch = 1.0;
            self.launching = false;
        } else {
            self.rpm = p.idle_rpm;
            self.clutch = 0.0;
            self.launching = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_bounded_and_continuous() {
        let mut prev = noise(0.0);
        for i in 1..2000 {
            let v = noise(i as f32 * 0.01);
            assert!((-1.0..=1.0).contains(&v));
            assert!((v - prev).abs() < 0.1);
            prev = v;
        }
    }
}
