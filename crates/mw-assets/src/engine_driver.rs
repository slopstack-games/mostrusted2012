//! Game state → the engine bank's bindings. The bindings are the game's own value names
//! (`"Players.LocalPlayer.GetPlayerAudibleRPM"`, `"RaceCars.EntityEngineSound[#].PopGain"`, ...). The RPM,
//! load and harmonic values follow the game's code (research log 2026-10-04, "engine sound from the game
//! code"); the pops, backfires, limiter and gear-change rules are still ours (marked "assumed"). Shared by
//! the runtime and `mwtool engine-sound`, so offline renders match the game.
//!
//! `MW_AUDIO_RPM_LINEAR=1` feeds the physics RPM straight to the sequences instead of the game's curve
//! (A/B listening).

use std::sync::Arc;

use mw_audio::sequence::Params;

use crate::sound::{EngineBank, EngineTuning, cue, gate};

/// What the physics knows about the engine this tick.
#[derive(Debug, Clone, Copy)]
pub struct EngineInput {
    pub rpm: f32,
    pub throttle: f32,
    pub gear: i32,
    pub speed_mph: f32,
    pub idle_rpm: f32,
    pub redline_rpm: f32,
    /// The handling's top speed; the engine load leans towards full at high speed.
    pub top_speed_mph: f32,
}

/// The audible RPM axis every car's sound maps onto: the game's `RaceCarEngineSound` holds a fixed
/// 1000..8000 range and reports `min + (max - min) × normalised RPM` as the audible RPM. The sequences
/// then clamp it to their own axis (1790..7500 on the 911).
pub const AUDIBLE_RPM_RANGE: (f32, f32) = (1000.0, 8000.0);

/// Normalised RPM as the game's sound code computes it: the idle..max fraction raised to `2 - a`, where
/// `a` is the `EngineSound2` curve term crossfaded between its off-load and full-load values by the engine
/// load (0.7 on the 911: an exponent of 1.3, so mid-range revs sound lower than they are).
pub fn normalised_rpm(t: &EngineTuning, rpm: f32, idle: f32, max: f32, load: f32) -> f32 {
    let x = ((rpm - idle) / (max - idle).max(1.0)).clamp(0.0, 1.0);
    let a = t.rpm_curve_off + (t.rpm_curve_on - t.rpm_curve_off) * load.clamp(0.0, 1.0);
    let e = (2.0 - a).clamp(0.1, 4.0);
    if (e - 1.0).abs() < 1e-3 { x } else { x.powf(e) }
}

/// Engine firing frequency (Hz): one cylinder fires every two revolutions.
pub fn firing_frequency(t: &EngineTuning, audible_rpm: f32) -> f32 {
    audible_rpm / 120.0 * t.cylinders
}

/// Engine load as the game shapes it: the throttle-based demand blended towards full load by the square of
/// the speed fraction (so a cruise at high speed still leans on the on-load layers), followed one-pole and
/// peak-held over the last few frames, so lifts fade the load layers rather than cutting them.
#[derive(Debug, Clone)]
pub struct LoadFilter {
    smoothed: f32,
    history: [f32; 6],
    k: usize,
}

impl Default for LoadFilter {
    fn default() -> Self {
        Self { smoothed: 0.0, history: [0.0; 6], k: 0 }
    }
}

impl LoadFilter {
    /// `demand` is the pedal-based load (0..1), `speed` the speed over the top speed.
    pub fn update(&mut self, dt: f32, demand: f32, speed: f32, rate: f32) -> f32 {
        // Speed term: (speed fraction × 0.75)², the handling's `937338e7` factor on the 911.
        let s = (speed.clamp(0.0, 1.0) * 0.75).powi(2);
        let target = s + (1.0 - s) * demand.clamp(0.0, 1.0);
        // The game scales its rate by an engine-independent constant; 20 per unit reads as a 100 ms
        // response at the data's 0.5.
        let a = (rate * 20.0 * dt).clamp(0.0, 1.0);
        self.smoothed += (target - self.smoothed) * a;
        self.history[self.k] = self.smoothed;
        self.k = (self.k + 1) % self.history.len();
        let held = self.history.iter().copied().fold(0.0f32, f32::max);
        target.min(1.0).max(held.min(1.0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    AudibleRpm,
    NormalisedRpm,
    Load,
    EngineOn,
    Speed,
    Harmonic1,
    Harmonic2,
    Subharmonic,
    PopProbability,
    RevLimitGain,
    GearElapsed,
    Upshift,
    Downshift,
    Jitter,
    Lfo(f32),
    Random(u32),
    Other,
}

fn classify(name: &str) -> Kind {
    let last = name.rsplit('.').next().unwrap_or(name);
    match last {
        "GetPlayerAudibleRPM" | "GetAudibleRPM" => Kind::AudibleRpm,
        "GetNormalisedRPM" => Kind::NormalisedRpm,
        "GetPlayerEngineLoad" | "GetEngineLoad" => Kind::Load,
        "EngineOn" => Kind::EngineOn,
        "GetPlayerSpeed" | "GetSpeedMph" => Kind::Speed,
        "TrackedHarmonic1Frequency" => Kind::Harmonic1,
        "TrackedHarmonic2Frequency" => Kind::Harmonic2,
        "SubharmoniserFrequency" => Kind::Subharmonic,
        "GetPopProbability" => Kind::PopProbability,
        "RevLimitGain" => Kind::RevLimitGain,
        "GetPlayerNormalisedGearChangeTimeElapsed" => Kind::GearElapsed,
        "Jitter" => Kind::Jitter,
        "Lfo" => Kind::Lfo(5.0),
        "ShuntLfo" => Kind::Lfo(4.0),
        "SkidLfo" => Kind::Lfo(9.0),
        _ if name == gate::UPSHIFT => Kind::Upshift,
        _ if name == gate::DOWNSHIFT => Kind::Downshift,
        _ => match name.strip_prefix("Signals.GetRandomNumber[").and_then(|r| r.strip_suffix(']')) {
            Some(n) => Kind::Random(n.parse().unwrap_or(2)),
            None => Kind::Other,
        },
    }
}

/// Seconds a gear change takes in normalised units (assumed): the data's gear-change items sit at
/// normalised times 0 (clunk) and 0.9–1 (engage), and the power cut runs 1–6.
const SHIFT_SECONDS: f32 = 0.25;

pub struct EngineDriver {
    params: Arc<Params>,
    slots: Vec<(usize, Kind)>,
    tuning: EngineTuning,
    /// `MW_AUDIO_RPM_LINEAR`: bypass the game's RPM curve.
    linear_rpm: bool,
    load: LoadFilter,
    /// Last engine load written (the RPM curve depends on it).
    load_value: f32,
    prev_throttle: f32,
    prev_gear: i32,
    gear_elapsed: f32,
    upshift: bool,
    pop: f32,
    limiter: f32,
    jitter_timer: f32,
    jitter: f32,
    time: f32,
    rng: u32,
    started: bool,
}

impl EngineDriver {
    pub fn new(bank: &EngineBank, params: Arc<Params>) -> Self {
        let slots: Vec<(usize, Kind)> =
            params.names().iter().enumerate().map(|(i, n)| (i, classify(n))).filter(|s| s.1 != Kind::Other).collect();
        for (i, k) in &slots {
            let name = &params.names()[*i];
            let tau = match k {
                Kind::AudibleRpm | Kind::NormalisedRpm | Kind::Harmonic1 | Kind::Harmonic2 | Kind::Subharmonic => 0.01,
                // Throttle taps crossfade the load layers instead of switching them (research log).
                Kind::Load => 0.05,
                Kind::GearElapsed | Kind::Upshift | Kind::Downshift | Kind::Random(_) | Kind::Jitter => 0.0,
                _ => 0.02,
            };
            params.set_smoothing(name, tau);
        }
        let mut d = Self {
            params,
            slots,
            tuning: bank.tuning,
            linear_rpm: std::env::var_os("MW_AUDIO_RPM_LINEAR").is_some(),
            load: LoadFilter::default(),
            load_value: 0.0,
            prev_throttle: 0.0,
            prev_gear: 1,
            gear_elapsed: 100.0,
            upshift: true,
            pop: 0.0,
            limiter: 0.0,
            jitter_timer: 0.0,
            jitter: 0.5,
            time: 0.0,
            rng: 0x1234_5678,
            started: false,
        };
        // Values before the first update: engine on, gear-change sequences idle.
        d.write(
            &EngineInput {
                rpm: 0.0,
                throttle: 0.0,
                gear: 1,
                speed_mph: 0.0,
                idle_rpm: 800.0,
                redline_rpm: 7000.0,
                top_speed_mph: 180.0,
            },
            0.0,
        );
        d
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// The engine RPM as the sound sequences see it (`GetPlayerAudibleRPM`): the game maps the normalised
    /// RPM ([`normalised_rpm`], curved by load) onto a fixed 1000..8000 axis, whatever the car's idle and
    /// redline. The sequences then clamp it (1790..7500 on the 911), so a 911 plays its idle loop alone up
    /// to about 2300 physics rpm and reaches the top grain at the redline. The earlier reading that the axis
    /// is physics RPM (research log "engine ground truth") fit the recordings but not the code.
    pub fn audible_rpm(&self, i: &EngineInput) -> f32 {
        if self.linear_rpm {
            return i.rpm;
        }
        let n = normalised_rpm(&self.tuning, i.rpm, i.idle_rpm, i.redline_rpm, self.load_value);
        AUDIBLE_RPM_RANGE.0 + (AUDIBLE_RPM_RANGE.1 - AUDIBLE_RPM_RANGE.0) * n
    }

    /// Advance by `dt` seconds with this tick's engine state.
    pub fn update(&mut self, dt: f32, i: &EngineInput) {
        self.time += dt;
        if !self.started {
            self.started = true;
            self.params.trigger(cue::IGNITION);
            self.prev_gear = i.gear;
        }
        let norm = ((i.rpm - i.idle_rpm) / (i.redline_rpm - i.idle_rpm).max(1.0)).clamp(0.0, 1.0);
        // Gear changes (assumed rules): restart the shift timeline, pick the group by direction; a full-throttle
        // upshift at high revs sometimes backfires.
        if i.gear != self.prev_gear && i.gear != 0 && self.prev_gear != 0 {
            self.upshift = i.gear.abs() > self.prev_gear.abs();
            self.gear_elapsed = 0.0;
            if self.upshift && i.throttle > 0.8 && norm > 0.6 && self.random() < 0.5 {
                self.params.trigger(cue::BACKFIRE);
            }
        }
        self.prev_gear = i.gear;
        self.gear_elapsed = (self.gear_elapsed + dt / SHIFT_SECONDS).min(100.0);
        // Overrun pops: lifting off at high revs arms a crackle that fades over ~1.5 s (assumed); a sharp
        // lift from near the redline sometimes backfires.
        if i.throttle < 0.1 && self.prev_throttle >= 0.3 {
            if norm > 0.45 {
                self.pop = 1.0;
            }
            if norm > 0.75 && self.random() < 0.35 {
                self.params.trigger(cue::BACKFIRE);
            }
        }
        if i.throttle > 0.2 {
            self.pop = 0.0;
        }
        self.pop *= (-dt / 0.6).exp();
        self.prev_throttle = i.throttle;
        // Rev limiter: bouncing on the limit fires the limiter cue every 80 ms.
        let on_limiter = i.throttle > 0.5 && i.rpm >= i.redline_rpm - 60.0;
        self.limiter -= dt;
        if on_limiter && self.limiter <= 0.0 {
            self.limiter = 0.08;
            self.params.trigger(cue::LIMITER);
        }
        self.jitter_timer -= dt;
        if self.jitter_timer <= 0.0 {
            self.jitter_timer = 0.05;
            self.jitter = self.random();
        }
        self.write(i, dt);
        if on_limiter {
            self.set_kind(Kind::RevLimitGain, 1.0);
        }
    }

    fn set_kind(&self, kind: Kind, v: f32) {
        for (slot, k) in &self.slots {
            if *k == kind {
                self.params.set(&self.params.names()[*slot], v);
            }
        }
    }

    fn write(&mut self, i: &EngineInput, dt: f32) {
        let shifting = self.gear_elapsed < 1.0;
        // Engine load: the pedal-based demand (reverse: also the pedal), cut during the shift, shaped by
        // speed and held as the game does.
        let demand = if shifting { 0.0 } else { i.throttle.clamp(0.0, 1.0) };
        let load = self.load.update(dt, demand, i.speed_mph / i.top_speed_mph.max(1.0), self.tuning.load_rate);
        self.load_value = load;
        let audible = self.audible_rpm(i);
        let norm = normalised_rpm(&self.tuning, i.rpm, i.idle_rpm, i.redline_rpm, load);
        let firing = firing_frequency(&self.tuning, audible);
        let names = self.params.names().to_vec();
        for &(slot, kind) in &self.slots.clone() {
            let v = match kind {
                Kind::AudibleRpm => audible,
                Kind::NormalisedRpm => norm,
                Kind::Load => load,
                Kind::EngineOn => 1.0,
                Kind::Speed => i.speed_mph,
                Kind::Harmonic1 => firing * self.tuning.harmonic1,
                Kind::Harmonic2 => firing * self.tuning.harmonic2,
                Kind::Subharmonic => firing * self.tuning.subharmonic,
                Kind::PopProbability => self.pop * norm.max(0.3),
                Kind::RevLimitGain => 0.0,
                Kind::GearElapsed => self.gear_elapsed,
                Kind::Upshift => f32::from(u8::from(self.upshift)),
                Kind::Downshift => f32::from(u8::from(!self.upshift)),
                Kind::Jitter => self.jitter,
                Kind::Lfo(hz) => 0.5 + 0.5 * (self.time * hz * std::f32::consts::TAU).sin(),
                Kind::Random(n) => ((self.jitter * n as f32) as u32).min(n.saturating_sub(1)) as f32,
                Kind::Other => continue,
            };
            self.params.set(&names[slot], v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpm_curve_follows_the_game() {
        // The 911: curve terms 0.7 → exponent 1.3; half the rev range sounds like 41 % of it.
        let t = EngineTuning { rpm_curve_on: 0.7, rpm_curve_off: 0.7, ..EngineTuning::default() };
        let n = normalised_rpm(&t, 4400.0, 1000.0, 7800.0, 1.0);
        assert!((n - 0.5f32.powf(1.3)).abs() < 1e-4, "{n}");
        assert_eq!(normalised_rpm(&t, 900.0, 1000.0, 7800.0, 1.0), 0.0);
        assert_eq!(normalised_rpm(&t, 9000.0, 1000.0, 7800.0, 1.0), 1.0);
        // Terms of 1 give a linear curve; the load picks between the on / off terms.
        let lin = EngineTuning { rpm_curve_on: 1.0, rpm_curve_off: 0.5, ..EngineTuning::default() };
        assert!((normalised_rpm(&lin, 4400.0, 1000.0, 7800.0, 1.0) - 0.5).abs() < 1e-6);
        assert!((normalised_rpm(&lin, 4400.0, 1000.0, 7800.0, 0.0) - 0.5f32.powf(1.5)).abs() < 1e-4);
        // Six cylinders at 4000 rpm fire 200 times a second.
        assert!((firing_frequency(&t, 4000.0) - 200.0).abs() < 1e-3);
    }

    #[test]
    fn load_leans_on_speed_and_holds_its_peak() {
        let mut f = LoadFilter::default();
        let dt = 1.0 / 64.0;
        for _ in 0..64 {
            f.update(dt, 1.0, 0.0, 0.5);
        }
        assert!(f.update(dt, 1.0, 0.0, 0.5) > 0.99);
        // Lifting off: the held peak keeps the load up for a few frames, then it decays.
        let just_after = f.update(dt, 0.0, 0.0, 0.5);
        assert!(just_after > 0.9, "{just_after}");
        for _ in 0..64 {
            f.update(dt, 0.0, 0.0, 0.5);
        }
        assert!(f.update(dt, 0.0, 0.0, 0.5) < 0.05);
        // Off the pedal at the top speed the load still leans towards full: (0.75)² = 0.56.
        let mut g = LoadFilter::default();
        for _ in 0..200 {
            g.update(dt, 0.0, 1.0, 0.5);
        }
        assert!((g.update(dt, 0.0, 1.0, 0.5) - 0.5625).abs() < 0.01);
    }
}
