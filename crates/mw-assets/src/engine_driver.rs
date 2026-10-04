//! Game state → the engine bank's bindings. The bindings are the game's own value names
//! (`"Players.LocalPlayer.GetPlayerAudibleRPM"`, `"RaceCars.EntityEngineSound[#].PopGain"`, ...); how the
//! game computes most of them is not in the data, so the rules here are ours (marked "assumed"). Shared by
//! the runtime and `mwtool engine-sound`, so offline renders match the game.

use std::sync::Arc;

use mw_audio::sequence::Params;

use crate::sound::{EngineBank, cue, gate};

/// What the physics knows about the engine this tick.
#[derive(Debug, Clone, Copy)]
pub struct EngineInput {
    pub rpm: f32,
    pub throttle: f32,
    pub gear: i32,
    pub speed_mph: f32,
    pub idle_rpm: f32,
    pub redline_rpm: f32,
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
    order: f32,
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
            order: bank.harmonic_order,
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
        d.write(&EngineInput {
            rpm: 0.0,
            throttle: 0.0,
            gear: 1,
            speed_mph: 0.0,
            idle_rpm: 800.0,
            redline_rpm: 7000.0,
        });
        d
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// The engine RPM as the sound sequences see it: the physics RPM itself. The sequences clamp it to their
    /// axis (1790..7500 on the 911), so below the axis only the recorded idle loop plays. Evidence that the
    /// axis is real engine RPM and not a remapped idle..redline range (research log 2026-10-04, "engine
    /// sound ground truth"): the 911's idle loop has the periodicity and harmonics of the Ginsu sweep's first
    /// grains (~1580 rpm), and the sweep's top (7814 rpm) is the handling's `Max_RPM` (7800).
    pub fn audible_rpm(&self, i: &EngineInput) -> f32 {
        i.rpm
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
        self.write(i);
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

    fn write(&mut self, i: &EngineInput) {
        let audible = self.audible_rpm(i);
        let norm = ((i.rpm - i.idle_rpm) / (i.redline_rpm - i.idle_rpm).max(1.0)).clamp(0.0, 1.0);
        let shifting = self.gear_elapsed < 1.0;
        // Engine load: the pedal (reverse: also the pedal), cut during the shift.
        let load = if shifting { 0.0 } else { i.throttle.clamp(0.0, 1.0) };
        let harmonic = audible / 60.0 * self.order;
        let names = self.params.names().to_vec();
        for &(slot, kind) in &self.slots.clone() {
            let v = match kind {
                Kind::AudibleRpm => audible,
                Kind::NormalisedRpm => norm,
                Kind::Load => load,
                Kind::EngineOn => 1.0,
                Kind::Speed => i.speed_mph,
                Kind::Harmonic1 | Kind::Subharmonic => harmonic,
                Kind::Harmonic2 => harmonic * 2.0,
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
