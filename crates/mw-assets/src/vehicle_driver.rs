//! Game state → the vehicle sound bank's bindings ([`crate::sfx`]). The skid intensities are computed by the
//! game from `TyreSoundParams` (`HAWAII/SURFACELIST.BNDL`), whose fields are not named; the rules here are
//! ours and marked "assumed".

use std::sync::Arc;

use mw_audio::sfx_player::Params;

use crate::sfx::names;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    LatSlip,
    Spin,
    SpinBraking,
    Speed,
    Scrape,
    ImpactVelocity,
    CrashMass,
    One,
    Jitter,
    Lfo(f32),
}

fn classify(name: &str) -> Option<Kind> {
    if name == names::SCRAPE_SPEED {
        return Some(Kind::Scrape);
    }
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

/// One wheel's contact this tick, in the car's frame of motion.
#[derive(Debug, Clone, Copy, Default)]
pub struct WheelSlip {
    /// Ground speed of the wheel (m/s).
    pub speed: f32,
    /// Sideways sliding speed of the contact patch (m/s).
    pub lateral: f32,
    /// Longitudinal slip speed: positive = sliding forward over the ground (locked / braking past the limit),
    /// negative = spinning under power.
    pub longitudinal: f32,
}

/// What the physics knows this tick.
#[derive(Debug, Clone, Default)]
pub struct VehicleInput {
    pub speed_mph: f32,
    /// Wheels on the ground.
    pub wheels: Vec<WheelSlip>,
    /// Brake pedal (0..1): hard ABS braking squeals a little even without locking.
    pub brake: f32,
    /// Sliding speed of a body contact with the world (m/s), 0 when none.
    pub scrape_speed: f32,
}

pub struct VehicleDriver {
    params: Arc<Params>,
    slots: Vec<(usize, Kind)>,
    time: f32,
    jitter: f32,
    jitter_timer: f32,
    rng: u32,
    scraping: bool,
    last: [f32; 5],
}

/// Skid intensity from a slip angle (assumed): silent below 4°, full at 12°. `TyreSoundParams.Lateral` holds
/// 0.4 and 1.3, which read as fractions of the tyre's peak slip angle (~9°) give the same range.
fn lateral_intensity(w: &WheelSlip) -> f32 {
    let angle = w.lateral.abs().atan2(w.speed.max(2.0)).to_degrees();
    let fade = ((w.speed - 2.0) / 6.0).clamp(0.0, 1.0);
    ((angle - 4.0) / 8.0).clamp(0.0, 1.0) * fade
}

impl VehicleDriver {
    pub fn new(params: Arc<Params>) -> Self {
        let slots: Vec<(usize, Kind)> =
            params.names().iter().enumerate().filter_map(|(i, n)| Some((i, classify(n)?))).collect();
        for (i, k) in &slots {
            let tau = match k {
                // Skids rise and fall over ~80 ms so single-tick slips don't chirp.
                Kind::LatSlip | Kind::Spin | Kind::SpinBraking => 0.08,
                Kind::ImpactVelocity | Kind::CrashMass | Kind::Jitter => 0.0,
                _ => 0.03,
            };
            params.set_smoothing(&params.names()[*i], tau);
        }
        let d = Self {
            params,
            slots,
            time: 0.0,
            jitter: 0.5,
            jitter_timer: 0.0,
            rng: 0x51ed_270b,
            scraping: false,
            last: [0.0; 5],
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

    /// The last update's speed (mph), lateral / spin / braking skid intensities and scrape speed (m/s).
    pub fn last(&self) -> [f32; 5] {
        self.last
    }

    /// Advance by `dt` seconds.
    pub fn update(&mut self, dt: f32, i: &VehicleInput) {
        self.time += dt;
        let lat = i.wheels.iter().map(lateral_intensity).fold(0.0, f32::max);
        // Spinning under power: any driven wheel past its grip (assumed: full intensity).
        let spin = if i.wheels.iter().any(|w| w.longitudinal < -0.5) { 1.0 } else { 0.0 };
        // Locked or sliding under braking, plus a squeal under hard ABS braking at speed (assumed).
        let fade = |v: f32| ((v - 2.0) / 8.0).clamp(0.0, 1.0);
        let locked = i
            .wheels
            .iter()
            .filter(|w| w.longitudinal > 0.5 && w.speed > 1.0)
            .map(|w| fade(w.speed))
            .fold(0.0, f32::max);
        let abs = if i.brake > 0.7 { 0.45 * i.brake * fade(i.speed_mph * 0.447) } else { 0.0 };
        self.last = [i.speed_mph, lat, spin, locked.max(abs), i.scrape_speed];
        self.set(Kind::LatSlip, lat);
        self.set(Kind::Spin, spin);
        self.set(Kind::SpinBraking, locked.max(abs));
        self.set(Kind::Speed, i.speed_mph);
        self.set(Kind::Scrape, i.scrape_speed);
        if i.scrape_speed > 5.0 && !self.scraping {
            self.params.trigger(names::SPARKS);
        }
        self.scraping = i.scrape_speed > 1.5;
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
        if closing_speed > 8.0 {
            self.params.trigger(names::SPARKS);
        }
    }
}
