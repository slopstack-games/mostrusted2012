//! Game state → the vehicle sound bank's bindings ([`crate::sfx`]). The three skid intensities follow the
//! game's `FUN_009d1bf0` (docs/decomp.md, "Tyre skid sound intensities"): one reference wheel of the driven
//! axle, ramps from `TyreSoundParams` (`HAWAII/SURFACELIST.BNDL`) and a per-tick rise / fall lerp. What the
//! code does not show is marked "assumed".

use std::sync::Arc;

use mw_audio::sfx_player::Params;

use crate::sfx::names;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    LatSlip,
    Spin,
    SpinBraking,
    Speed,
    ImpactVelocity,
    CrashMass,
    One,
    Jitter,
    Lfo(f32),
}

fn classify(name: &str) -> Option<Kind> {
    Some(match name.rsplit('.').next().unwrap_or(name) {
        "LatSlipIntensityForSkidSound" => Kind::LatSlip,
        "SpinIntensityForSkidSound" => Kind::Spin,
        "SpinBrakingIntensityForSkidSound" => Kind::SpinBraking,
        "GetSpeedMph" | "GetPlayerSpeed" => Kind::Speed,
        "GetMostRecentPlayerImpactClosingVelocity" => Kind::ImpactVelocity,
        "GetMassForLastCrash" => Kind::CrashMass,
        "EngineOn" | "GetIsLocalPlayer" => Kind::One,
        "Jitter" => Kind::Jitter,
        "Lfo" => Kind::Lfo(5.0),
        "ShuntLfo" => Kind::Lfo(4.0),
        "SkidLfo" => Kind::Lfo(9.0),
        _ => return None,
    })
}

/// The skid sound's reference wheel this tick (`FUN_009d1bf0` reads one wheel: the front-left one when the
/// engine drives only the front axle, else the first rear wheel).
#[derive(Debug, Clone, Copy, Default)]
pub struct WheelSlip {
    /// Slip angle (degrees, either sign).
    pub slip_angle: f32,
    /// Difference between the tread speed (ω r) and the ground speed along the wheel (m/s, either sign):
    /// a spinning or a locked wheel.
    pub slip_speed: f32,
}

/// What the physics knows this tick.
#[derive(Debug, Clone, Default)]
pub struct VehicleInput {
    pub speed_mph: f32,
    /// The reference wheel ([`WheelSlip`]), `None` while it is in the air.
    pub skid_wheel: Option<WheelSlip>,
    /// Brake pedal (0..1).
    pub brake: f32,
    /// Handbrake pulled.
    pub handbrake: bool,
}

/// One ramp of `TyreSoundParams` (`LatSlip`, `LongSpin`, `LongSpinBraking`): the input's start and full values,
/// the speed (mph) at which the intensity reaches full (0 = no speed factor), the exponent and the per-tick
/// (60 Hz) lerp rates up and down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkidRamp {
    pub start: f32,
    pub full: f32,
    pub full_speed_mph: f32,
    pub curve_power: f32,
    pub rise: f32,
    pub fall: f32,
}

impl SkidRamp {
    fn ratio(&self, input: f32) -> f32 {
        let r = ((input - self.start) / (self.full - self.start).max(1.19e-7)).clamp(0.0, 1.0);
        if self.curve_power > 0.0 && self.curve_power != 1.0 { r.powf(self.curve_power) } else { r }
    }

    fn speed_factor(&self, mph: f32) -> f32 {
        if self.full_speed_mph > 0.0 { (mph.abs() / self.full_speed_mph).clamp(0.0, 1.0) } else { 1.0 }
    }

    /// The game's per-tick lerp towards `target` (rise rate when above, fall rate when below), made
    /// independent of the frame time: `dt` seconds are `60 dt` ticks.
    fn approach(&self, x: f32, target: f32, dt: f32) -> f32 {
        let k = if target > x { self.rise } else { self.fall }.clamp(0.0, 1.0);
        let k = 1.0 - (1.0 - k).powf(dt * 60.0);
        x + (target - x) * k
    }
}

/// The skid-sound tuning (`Genesys.Gen.TyreSoundParams`, one instance in the install, `…1b0ca7`). Its
/// `Lateral` and `Longitudinal` groups are never read by the code.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TyreSoundParams {
    /// Input: the slip angle (degrees).
    pub lat_slip: SkidRamp,
    /// Input: the slip speed (mph).
    pub long_spin: SkidRamp,
    /// Input: the brake pedal (0..1).
    pub long_spin_braking: SkidRamp,
}

impl Default for TyreSoundParams {
    /// The data's values (`…1b0ca7`), used until the bundle is read.
    fn default() -> Self {
        Self {
            lat_slip: SkidRamp {
                start: 0.05,
                full: 40.0,
                full_speed_mph: 10.0,
                curve_power: 1.0,
                rise: 0.5,
                fall: 0.1,
            },
            long_spin: SkidRamp {
                start: 2.0,
                full: 20.0,
                full_speed_mph: 0.0,
                curve_power: 1.0,
                rise: 0.12,
                fall: 0.2,
            },
            long_spin_braking: SkidRamp {
                start: 0.3,
                full: 1.0,
                full_speed_mph: 40.0,
                curve_power: 1.0,
                rise: 0.05,
                fall: 0.5,
            },
        }
    }
}

/// Metres per second to miles per hour. The game scales the wheel's slip speed by a runtime constant
/// (`.bss` 0x0118a060 = 2.23694, read live 2026-10-06: m/s → mph) before comparing it with `LongSpin`'s 2 / 20.
const MPH_PER_MS: f32 = 2.236_936;
/// `FUN_009d1bf0` halves the pedal-driven braking intensity (constant `0x01026058` = 0.5).
const PEDAL_BRAKING_SCALE: f32 = 0.5;

pub struct VehicleDriver {
    params: Arc<Params>,
    slots: Vec<(usize, Kind)>,
    /// Skid tuning ([`TyreSoundParams`]).
    pub tyre: TyreSoundParams,
    /// Smoothed skid intensities: lateral, spin, braking.
    skid: [f32; 3],
    time: f32,
    jitter: f32,
    jitter_timer: f32,
    rng: u32,
    last: [f32; 4],
}

impl VehicleDriver {
    pub fn new(params: Arc<Params>) -> Self {
        let slots: Vec<(usize, Kind)> =
            params.names().iter().enumerate().filter_map(|(i, n)| Some((i, classify(n)?))).collect();
        for (i, k) in &slots {
            let tau = match k {
                // The skids are smoothed here, as the game does ([`SkidRamp::approach`]).
                Kind::LatSlip | Kind::Spin | Kind::SpinBraking => 0.0,
                Kind::ImpactVelocity | Kind::CrashMass | Kind::Jitter => 0.0,
                _ => 0.03,
            };
            params.set_smoothing(&params.names()[*i], tau);
        }
        let d = Self {
            params,
            slots,
            tyre: TyreSoundParams::default(),
            skid: [0.0; 3],
            time: 0.0,
            jitter: 0.5,
            jitter_timer: 0.0,
            rng: 0x51ed_270b,
            last: [0.0; 4],
        };
        d.set(Kind::One, 1.0);
        d.set(Kind::CrashMass, 0.5);
        d
    }

    fn set(&self, kind: Kind, v: f32) {
        for (slot, k) in &self.slots {
            if *k == kind {
                self.params.set(&self.params.names()[*slot], v);
            }
        }
    }

    /// The last update's speed (mph) and lateral / spin / braking skid intensities.
    pub fn last(&self) -> [f32; 4] {
        self.last
    }

    /// Advance by `dt` seconds.
    pub fn update(&mut self, dt: f32, i: &VehicleInput) {
        self.time += dt;
        let t = self.tyre;
        let mph = i.speed_mph.abs();
        // Targets as `FUN_009d1bf0` computes them. Lateral: the slip angle's ramp times the speed's.
        // In the air the wheel has no slip (assumed: the game keeps reading the wheel's last solve).
        let w = i.skid_wheel.unwrap_or_default();
        let lat = t.lat_slip.ratio(w.slip_angle.abs()) * t.lat_slip.speed_factor(mph);
        // Spin: the tread against the ground in mph, either way (a spinning or a locked wheel). The game zeroes
        // it while a wheel value (+0x108 of the object `FUN_009d1bf0` reads) is below `.bss` 0x011870d0 = −0.1
        // (read live 2026-10-06); which value is not identified, assumed: the wheel is in the air.
        let spin = if i.skid_wheel.is_some() { t.long_spin.ratio(w.slip_speed.abs() * MPH_PER_MS) } else { 0.0 };
        // Braking: the pedal's ramp, times the speed's, halved; a predicate of the vehicle (its virtual method
        // +0x44, not decompiled) replaces it with the speed factor alone. Assumed: that predicate is the
        // handbrake.
        let speed = t.long_spin_braking.speed_factor(mph);
        let braking =
            if i.handbrake { speed } else { t.long_spin_braking.ratio(i.brake) * speed * PEDAL_BRAKING_SCALE };
        self.skid[0] = t.lat_slip.approach(self.skid[0], lat, dt);
        self.skid[1] = t.long_spin.approach(self.skid[1], spin, dt);
        self.skid[2] = t.long_spin_braking.approach(self.skid[2], braking, dt);
        let [lat, spin, braking] = self.skid;
        self.last = [i.speed_mph, lat, spin, braking];
        self.set(Kind::LatSlip, lat);
        self.set(Kind::Spin, spin);
        self.set(Kind::SpinBraking, braking);
        self.set(Kind::Speed, i.speed_mph);
        self.jitter_timer -= dt;
        if self.jitter_timer <= 0.0 {
            self.jitter_timer = 0.05;
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 17;
            self.rng ^= self.rng << 5;
            self.jitter = (self.rng >> 8) as f32 / (1u32 << 24) as f32;
        }
        self.set(Kind::Jitter, self.jitter);
        for (slot, k) in &self.slots {
            if let Kind::Lfo(hz) = k {
                let v = 0.5 + 0.5 * (self.time * hz * std::f32::consts::TAU).sin();
                self.params.set(&self.params.names()[*slot], v);
            }
        }
    }

    /// An impact: closing speed (m/s) picks the cue (assumed thresholds: light below 4 m/s, heavy above 12) and
    /// feeds the sequences' closing-velocity binding (which reads as mph: its ranges run 2–60).
    pub fn impact(&self, closing_speed: f32) {
        if closing_speed < 1.0 {
            return;
        }
        self.set(Kind::ImpactVelocity, closing_speed / 0.447_04);
        let cue = if closing_speed < 4.0 {
            names::IMPACT_LIGHT
        } else if closing_speed < 12.0 {
            names::IMPACT_MEDIUM
        } else {
            names::IMPACT_HEAVY
        };
        self.params.trigger(cue);
    }
}
