//! Game state → the engine bank's bindings. The bindings are the game's own value names
//! (`"Players.LocalPlayer.GetPlayerAudibleRPM"`, `"RaceCars.EntityEngineSound[#].PopGain"`, ...). Ported from the
//! game's code (docs/decomp.md "Engine sound"): the normalised and audible RPM (`FUN_009ae930` /
//! `FUN_009aea50`), the engine load (`FUN_009b1f70`), the gear-change sequences and their clock
//! (`FUN_0058ca50`, `FUN_00911650`), the effect items the sequences post back to the engine model
//! (`RaceCarEffectSequenceItem` → `FUN_008f5490`: RPM offsets, load multipliers and floors), the LFO
//! (`FUN_00586330`), the rev limiter's gate (`FUN_0058cdb0`) and the pops and backfires (`FUN_009b20b0`,
//! [`ExhaustPops`]). Model values are marked "assumed". Shared by the runtime and `mwtool engine-sound`, so offline
//! renders match the game.
//!
//! `MW_AUDIO_RPM_LINEAR=1` feeds the physics RPM straight to the sequences instead of the game's curve
//! (A/B listening). `MW_AUDIO_NO_FX=1` drops the effect items (A/B of the gear-change shunt and power cut).

use std::sync::Arc;

use mw_audio::sequence::{ItemKind, ModValue, Params};

use crate::sound::{EngineBank, EngineTuning, cue, gate};

/// What the physics knows about the engine this tick.
#[derive(Debug, Clone, Copy)]
pub struct EngineInput {
    pub rpm: f32,
    /// The pedal (offline renders without a drivetrain also use it as the engine's throttle).
    pub throttle: f32,
    pub gear: i32,
    pub speed_mph: f32,
    pub idle_rpm: f32,
    pub redline_rpm: f32,
    /// The handling's top speed (vehicle +0x7e4: 174 on the XKR, its `Top_speed_MPH`, read live): the engine
    /// load leans towards full as the speed approaches it.
    pub top_speed_mph: f32,
    /// The drivetrain's state, when there is one (the game, `--replay` traces); `None` for offline sweeps.
    pub drive: Option<DriveState>,
}

/// What the engine sound reads from the drivetrain besides the RPM.
#[derive(Debug, Clone, Copy, Default)]
pub struct DriveState {
    /// The engine's lagged throttle (engine +0x24, vehicle +0x1d14): the engine load follows it, so it falls
    /// to 0 through an upshift's cut and comes back with the clutch.
    pub engine_throttle: f32,
    /// Clutch engagement (vehicle +0x1e20): the load's speed term is scaled by it.
    pub clutch: f32,
    /// `GetPlayerNormalisedGearChangeTimeElapsed` (`FUN_00911650`): time since the last shift started over that
    /// shift's time.
    pub gear_change_elapsed: f32,
    /// Shifts started so far and the direction of the last one (a new count starts a gear-change sequence).
    pub shifts_started: u32,
    pub last_shift_up: bool,
    /// The cruising gearbox is in use (vehicle +0x1bdc = 1): its own gear-change lists, no pops.
    pub cruising: bool,
    /// The gear is one of the gearbox's last two (`FUN_00911690`): only then can the rev limiter play.
    pub top_gears: bool,
}

/// The audible RPM range of `RaceCarEngineSound` (+0x1c0 / +0x1c4) as its constructor sets it; `FUN_00575340`
/// replaces it with the span of the bank's Ginsu items when the sound starts ([`EngineBank::audible_range`]),
/// so this is only the fallback for a bank without them.
pub const AUDIBLE_RPM_RANGE: (f32, f32) = (1000.0, 8000.0);

/// The bottom of the rev range the sound ignores: the idle..max fraction is rescaled from this up to 1
/// (`FUN_009ae930`, `.bss` 0x01186680 = 0.05, read live).
const RPM_FLOOR: f32 = 0.05;

/// Normalised RPM as the game's sound code computes it (`FUN_009ae930` → `FUN_009aea50`, vehicle +0x1f10): the
/// idle..max fraction, rescaled above 5 % of the range, raised to `2 - a`, where `a` is the `EngineSound2` curve
/// term crossfaded between its off-load and full-load values by the engine load (0.7 on the 911: an exponent of
/// 1.3, so mid-range revs sound lower than they are); then the sequences' RPM offsets (effect property 1) are
/// added and the result clamped to 0..1. The engine-damage factor of `FUN_009ae930` (a power of the global
/// tuning +0xd0 by the RMS of vehicle +0x19d0) is 1 on an undamaged car and left out.
pub fn normalised_rpm(t: &EngineTuning, rpm: f32, idle: f32, max: f32, load: f32) -> f32 {
    normalised_rpm_with(t, rpm, idle, max, load, 0.0)
}

/// [`normalised_rpm`] with the effect items' RPM offset.
pub fn normalised_rpm_with(t: &EngineTuning, rpm: f32, idle: f32, max: f32, load: f32, offset: f32) -> f32 {
    let r = (rpm - idle) / (max - idle).max(1.0);
    let x = ((r - RPM_FLOOR).max(0.0) / (1.0 - RPM_FLOOR)).clamp(0.0, 1.0);
    let a = t.rpm_curve_off + (t.rpm_curve_on - t.rpm_curve_off) * load.clamp(0.0, 1.0);
    let e = (2.0 - a).clamp(0.1, 4.0);
    let curved = if (e - 1.0).abs() < 1e-3 { x } else { x.powf(e) };
    (curved + offset).clamp(0.0, 1.0)
}

/// `99a16939` at +0x7c; 0.1 and 0.9 on the 911).
#[derive(Debug, Clone, Copy, Default)]
pub struct PopParams {
    /// A tick backfires when `1 − drop · r · 10` falls below this (vehicle +0x1f42).
    pub backfire: f32,
    /// Otherwise it pops when it falls below this (vehicle +0x1f41).
    pub pop: f32,
}

impl PopParams {
    pub fn from_handling(h: &crate::handling::VehicleHandling) -> Self {
        Self {
            backfire: h.param("Engine.Sound.38d82e04").unwrap_or(0.0),
            pop: h.param("Engine.Sound.99a16939").unwrap_or(0.0),
        }
    }
}

/// What the pop rule reads each tick.
#[derive(Debug, Clone, Copy)]
pub struct PopInput {
    /// The engine-sound RPM fraction ([`normalised_rpm`], vehicle +0x1f10).
    pub fraction: f32,
    /// The car's `Idle_RPM` and `Max_RPM` (`Engine` +0x2c / +0x38).
    pub idle_rpm: f32,
    pub max_rpm: f32,
    /// The pedal after the gearbox's cuts (vehicle +0x1aa0).
    pub gearbox_throttle: f32,
    /// The gearbox is past its start-up states (+0x1bd8 not 0 or 1).
    pub running: bool,
    /// The cruising gearbox is in use (+0x1bdc = 1): no pops.
    pub cruising: bool,
}

/// What a tick's pop rule fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopEvent {
    None,
    /// A pop (vehicle +0x1f41): starts the engine's `Pop_Sequence`.
    Pop,
    /// A backfire (vehicle +0x1f42): starts `Backfire_Sequence` and the backfire flames (effect group 14).
    Backfire,
}

/// Audible RPM above which the exhaust can pop (`.rdata` 0x010251e0).
const POP_MIN_RPM: f32 = 4000.0;
/// Scale of the per-tick drop in the pop test (`.rdata` 0x010250c0).
const POP_DROP_SCALE: f32 = 10.0;
/// `GetPopProbability` reaches 1 at this RPM fraction (`.bss` 0x01186810, read live).
const POP_PROBABILITY_FRACTION: f32 = 0.8;

/// The game's exhaust pops and backfires (`FUN_009b20b0`, run every tick by the engine-sound update
/// `FUN_009b3030` after the gearbox; docs/decomp.md "Exhaust pops and backfires"). With the audible RPM
/// `A = Idle + (Max − Idle) · fraction` (vehicle +0x1f00), it looks at how far `A` fell over the last tick:
/// above 4000 audible RPM, outside the cruising gearbox, `x = 1 − drop / (Max − Idle) · r · 10` with `r`
/// uniform in 0..1; a backfire when the car's backfire value is above `x`, else a pop when its pop value
/// is. The pop probability the gurgle / pop sequences are bound to (`GetPopProbability`, +0x1eb4) is
/// `clamp(fraction / 0.8) · (1 − gearbox throttle)²`. The game evaluates this before it updates the
/// audible RPM, so both use the previous tick's values (kept here the same way).
#[derive(Debug, Clone)]
pub struct ExhaustPops {
    /// Audible RPM and fraction at the last tick and the one before (+0x1f00 / +0x1f10, +0x1eb0).
    last: Option<(f32, f32)>,
    before: Option<f32>,
    /// `GetPopProbability` (vehicle +0x1eb4).
    pub probability: f32,
    /// The last tick's drop as a fraction of the rev range (diagnostics).
    pub drop: f32,
    rng: u32,
}

impl Default for ExhaustPops {
    fn default() -> Self {
        Self { last: None, before: None, probability: 0.0, drop: 0.0, rng: 0x2545_f491 }
    }
}

impl ExhaustPops {
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// One physics tick.
    pub fn tick(&mut self, p: &PopParams, i: &PopInput) -> PopEvent {
        let range = (i.max_rpm - i.idle_rpm).max(1.0);
        let mut event = PopEvent::None;
        if let Some((audible, fraction)) = self.last {
            let drop = (self.before.unwrap_or(audible) - audible).max(0.0);
            self.drop = drop / range;
            if i.running && audible > POP_MIN_RPM && !i.cruising {
                let x = 1.0 - drop / range * self.random() * POP_DROP_SCALE;
                if p.backfire > x {
                    event = PopEvent::Backfire;
                } else if p.pop > x {
                    event = PopEvent::Pop;
                }
            }
            let open = 1.0 - i.gearbox_throttle.clamp(0.0, 1.0);
            self.probability = (fraction / POP_PROBABILITY_FRACTION).clamp(0.0, 1.0) * open * open;
            self.before = Some(audible);
        }
        let fraction = i.fraction.clamp(0.0, 1.0);
        self.last = Some((i.idle_rpm + range * fraction, fraction));
        event
    }
}

/// Engine firing frequency (Hz): one cylinder fires every two revolutions.
pub fn firing_frequency(t: &EngineTuning, audible_rpm: f32) -> f32 {
    audible_rpm / 120.0 * t.cylinders
}

/// `.bss` 0x011865e0 = 60 (read live): the load's follow rate is `EngineSound2.99d4b91e` × this per second, so
/// 0.5 per tick at 60 Hz (the original's load falls 0.99 → 0.40 → 0.11 in the ticks after an upshift starts,
/// `ref/xkr_launch`).
const LOAD_RATE_BASE: f32 = 60.0;

/// The game's engine load (`FUN_009b1f70`, vehicle +0x1f20, `GetPlayerEngineLoad`): a one-pole follower
/// (state at +0x1f24) towards `s + (1 − s) h`, with `h` the engine's lagged throttle and
/// `s = (clamp01(speed / top speed) × Engine.937338e7 × clutch)²`, so a fast car leans on the on-load layers even
/// off the pedal, but not with the clutch open. The result is multiplied by the effect items' load multipliers
/// and raised to their floors (property 3 / 4 lists at +0x1e40 / +0x1e5c).
#[derive(Debug, Clone, Default)]
pub struct LoadFilter {
    smoothed: f32,
}

impl LoadFilter {
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        dt: f32,
        throttle: f32,
        speed: f32,
        speed_factor: f32,
        clutch: f32,
        rate: f32,
        multiplier: f32,
        floor: f32,
    ) -> f32 {
        let s = (speed.clamp(0.0, 1.0) * speed_factor * clutch).clamp(0.0, 1.0).powi(2);
        let target = s + (1.0 - s) * throttle;
        let a = (LOAD_RATE_BASE * rate * dt).clamp(0.0, 1.0);
        self.smoothed += a * (target - self.smoothed);
        (self.smoothed * multiplier).max(floor)
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
    /// The gate of gear-change list `.0`, variant `.1` ([`gate::gear_change`]).
    GearGate(usize, usize),
    Jitter,
    /// The engine sound's LFO (`RaceCarEngineSound` +0x11c) and the shunt LFO (+0x120).
    Lfo,
    ShuntLfo,
    SkidLfo,
    /// `RaceCars.Entities["Id"].GetDistanceToListener` (`FUN_0059b690`: metres from the primary listener to the
    /// car), supplied by the caller for other cars ([`EngineDriver::set_listener_distance`]).
    ListenerDistance,
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
        "Lfo" => Kind::Lfo,
        "ShuntLfo" => Kind::ShuntLfo,
        "SkidLfo" => Kind::SkidLfo,
        "GetDistanceToListener" => Kind::ListenerDistance,
        _ => {
            if let Some((list, k)) = gate::parse(name) {
                return Kind::GearGate(list, k);
            }
            match name.strip_prefix("Signals.GetRandomNumber[").and_then(|r| r.strip_suffix(']')) {
                Some(n) => Kind::Random(n.parse().unwrap_or(2)),
                None => Kind::Other,
            }
        }
    }
}

/// Offline renders (no drivetrain): a gear change takes this long (s; the 911's `Gear_change_time`, assumed for
/// traces that carry no gear-change clock).
const OFFLINE_SHIFT_SECONDS: f32 = 0.15;

/// The rev limiter plays above this normalised RPM (`FUN_0058cdb0`: vehicle +0x1f10 against `.rdata` 0x00fad930).
const LIMITER_FRACTION: f32 = 0.99;

/// An LFO of the engine sound: `0.5 × sin(phase) × depth` (`FUN_00aabc30`), its rate (Hz) and depth set by the
/// active `LfoSequenceItem` (properties 0 and 1; assumed: the item's writer is not in the dump). Without one the
/// depth is 0.
#[derive(Debug, Clone, Copy, Default)]
struct Lfo {
    phase: f32,
    rate: f32,
    depth: f32,
}

impl Lfo {
    fn value(&self) -> f32 {
        0.5 * self.phase.sin() * self.depth
    }
}

/// An item that acts on the engine model or its LFO rather than playing a sound.
#[derive(Debug, Clone)]
struct FxItem {
    start: f32,
    end: f32,
    lfo: bool,
    values: Vec<(u32, ModValue, Kind)>,
}

/// A sequence with [`FxItem`]s: its binding, gate and axis.
#[derive(Debug, Clone)]
struct FxSequence {
    binding: Kind,
    gate: Option<(usize, usize)>,
    min: f32,
    max: f32,
    /// Its LFO items drive the shunt LFO (the sequence's effects read `ShuntLfo`), else the main one.
    shunt: bool,
    items: Vec<FxItem>,
}

/// What the effect items post to the engine model for the next tick (`FUN_008f5490`): property 1 is summed onto
/// the normalised RPM (list +0x1e78, `FUN_00926330`), property 3 multiplies the load (+0x1e40, `FUN_009262f0`),
/// property 4 is a floor under it (+0x1e5c, `FUN_0092ae80`). Properties 5 (+0x1e94) and 6 / 7 (engine start
/// flags) have no sound reader found and are ignored.
#[derive(Debug, Clone, Copy)]
pub struct Effects {
    pub rpm_offset: f32,
    pub load_multiplier: f32,
    pub load_floor: f32,
}

impl Default for Effects {
    fn default() -> Self {
        Self { rpm_offset: 0.0, load_multiplier: 1.0, load_floor: 0.0 }
    }
}

/// What the driver computed on its last update (diagnostics, `MW_SOUND_TRACE`).
#[derive(Debug, Clone, Copy, Default)]
pub struct SoundState {
    pub load: f32,
    pub normalised: f32,
    pub audible: f32,
    pub gear_elapsed: f32,
    pub effects: Effects,
    pub limiter: bool,
}

pub struct EngineDriver {
    params: Arc<Params>,
    slots: Vec<(usize, Kind)>,
    tuning: EngineTuning,
    /// `MW_AUDIO_RPM_LINEAR`: bypass the game's RPM curve.
    linear_rpm: bool,
    /// `GetPlayerAudibleRPM`'s range (the bank's Ginsu span).
    audible_range: (f32, f32),
    load: LoadFilter,
    /// The sound load (vehicle +0x1f20) of the last update: the RPM curve of the next one crossfades by it.
    load_value: f32,
    /// `Engine.937338e7` (Engine +0x104): the speed term of the load.
    speed_load: f32,
    /// Variants per gear-change list, the shared round-robin index (`FUN_0058ca50`, +0x1b4) and the open gate.
    gear_changes: [usize; 4],
    gear_index: usize,
    open_gate: Option<(usize, usize)>,
    shifts_seen: Option<u32>,
    /// Offline (no drivetrain): the gear-change clock from gear index changes.
    prev_gear: i32,
    offline_shifts: u32,
    offline_elapsed: f32,
    offline_up: bool,
    fx: Vec<FxSequence>,
    use_fx: bool,
    effects: Effects,
    lfo: Lfo,
    shunt: Lfo,
    /// The exhaust pop rule: run here at `update`'s rate unless the caller runs it in its physics tick and
    /// hands the results over ([`EngineDriver::push_pops`]).
    pops: ExhaustPops,
    pop_params: PopParams,
    external_pops: bool,
    pop_probability: f32,
    /// The rev limiter (`FUN_0058cdb0`): on, its cycle phase (cycles) and rate.
    limiter_on: bool,
    limiter_phase: f32,
    limiter_rate: f32,
    jitter_timer: f32,
    jitter: f32,
    time: f32,
    rng: u32,
    started: bool,
    state: SoundState,
    listener_distance: Option<f32>,
}

impl EngineDriver {
    pub fn new(bank: &EngineBank, params: Arc<Params>) -> Self {
        let slots: Vec<(usize, Kind)> =
            params.names().iter().enumerate().map(|(i, n)| (i, classify(n))).filter(|s| s.1 != Kind::Other).collect();
        for (i, k) in &slots {
            let name = &params.names()[*i];
            let tau = match k {
                Kind::AudibleRpm | Kind::NormalisedRpm | Kind::Harmonic1 | Kind::Harmonic2 | Kind::Subharmonic => 0.01,
                // The load is already the game's smoothed value.
                Kind::Load => 0.0,
                Kind::GearElapsed
                | Kind::GearGate(..)
                | Kind::Random(_)
                | Kind::Jitter
                | Kind::Lfo
                | Kind::ShuntLfo
                | Kind::RevLimitGain => 0.0,
                _ => 0.02,
            };
            params.set_smoothing(name, tau);
        }
        let fx = bank
            .bank
            .sequences
            .iter()
            .filter_map(|s| {
                let items: Vec<FxItem> = s
                    .items
                    .iter()
                    .filter_map(|it| {
                        let (values, lfo) = match &it.kind {
                            ItemKind::Effect { values } => (values, false),
                            ItemKind::Lfo { values } => (values, true),
                            _ => return None,
                        };
                        let values = values.iter().map(|(p, m)| (*p, m.clone(), classify(&m.binding))).collect();
                        Some(FxItem { start: it.start, end: it.end, lfo, values })
                    })
                    .collect();
                let binding = classify(&s.binding);
                // Cue timelines (in-air, burnout, ignition) are not started by this driver.
                (!items.is_empty() && binding != Kind::Other).then(|| FxSequence {
                    binding,
                    gate: gate::parse(&s.gate),
                    min: s.min,
                    max: s.max,
                    shunt: items.iter().flat_map(|i| &i.values).any(|v| v.2 == Kind::ShuntLfo),
                    items,
                })
            })
            .collect();
        let mut d = Self {
            params,
            slots,
            tuning: bank.tuning,
            linear_rpm: std::env::var_os("MW_AUDIO_RPM_LINEAR").is_some(),
            audible_range: bank.audible_range,
            load: LoadFilter::default(),
            load_value: 0.0,
            speed_load: 0.75,
            gear_changes: bank.gear_changes,
            gear_index: 0,
            open_gate: None,
            shifts_seen: None,
            prev_gear: 1,
            offline_shifts: 0,
            offline_elapsed: f32::MAX,
            offline_up: true,
            fx,
            use_fx: std::env::var_os("MW_AUDIO_NO_FX").is_none(),
            effects: Effects::default(),
            lfo: Lfo::default(),
            shunt: Lfo::default(),
            pops: ExhaustPops::default(),
            pop_params: PopParams::default(),
            external_pops: false,
            pop_probability: 0.0,
            limiter_on: false,
            limiter_phase: 0.0,
            limiter_rate: 1.0,
            jitter_timer: 0.0,
            jitter: 0.5,
            time: 0.0,
            rng: 0x1234_5678,
            started: false,
            state: SoundState::default(),
            listener_distance: None,
        };
        // Values before the first update: engine on, gear-change sequences closed.
        d.write(
            &EngineInput {
                rpm: 0.0,
                throttle: 0.0,
                gear: 1,
                speed_mph: 0.0,
                idle_rpm: 800.0,
                redline_rpm: 7000.0,
                top_speed_mph: 180.0,
                drive: None,
            },
            f32::MAX,
        );
        d
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// The engine RPM as the sound sequences see it (`GetPlayerAudibleRPM`, `FUN_0083eb60`): the normalised RPM
    /// mapped onto the span of the bank's Ginsu items (`FUN_00575340`), so idle plays the bottom of the recordings
    /// and `Max_RPM` their top (1790..7500 on the 911, 1750..6000 on the XKR).
    pub fn audible_rpm(&self, i: &EngineInput) -> f32 {
        self.audible(i, normalised_rpm_with(&self.tuning, i.rpm, i.idle_rpm, i.redline_rpm, self.load_value, 0.0))
    }

    fn audible(&self, i: &EngineInput, norm: f32) -> f32 {
        if self.linear_rpm {
            return i.rpm;
        }
        self.audible_range.0 + (self.audible_range.1 - self.audible_range.0) * norm
    }

    /// `Engine.937338e7` of the car's handling: how far the speed pushes the load towards full.
    pub fn set_speed_load(&mut self, v: f32) {
        self.speed_load = v;
    }

    /// The drivetrain's state, or one made up from the pedal and the gear index for offline renders.
    fn drive_state(&mut self, dt: f32, i: &EngineInput) -> DriveState {
        if let Some(d) = i.drive {
            return d;
        }
        if i.gear != self.prev_gear && i.gear > 0 && self.prev_gear > 0 {
            self.offline_shifts += 1;
            self.offline_up = i.gear > self.prev_gear;
            self.offline_elapsed = 0.0;
        } else {
            self.offline_elapsed += dt;
        }
        self.prev_gear = i.gear;
        DriveState {
            engine_throttle: i.throttle.clamp(0.0, 1.0),
            clutch: 1.0,
            gear_change_elapsed: self.offline_elapsed / OFFLINE_SHIFT_SECONDS,
            shifts_started: self.offline_shifts,
            last_shift_up: self.offline_up,
            cruising: false,
            top_gears: true,
        }
    }

    /// `FUN_0058ca50`: a shift starts the next gear-change sequence of the list for its direction and gearbox,
    /// stopping the one before.
    fn start_gear_change(&mut self, up: bool, cruising: bool) {
        let list = usize::from(!up) + if cruising { 2 } else { 0 };
        let n = self.gear_changes[list];
        if n == 0 {
            return;
        }
        self.gear_index = (self.gear_index + 1) % n;
        self.open_gate = Some((list, self.gear_index));
    }

    /// The distance from the listener to this car (`GetDistanceToListener`), for the banks of other cars; the
    /// player's sound leaves it unset (the binding then stays at 0).
    pub fn set_listener_distance(&mut self, d: Option<f32>) {
        self.listener_distance = d;
    }

    /// The value of a binding the effect items read (`None`: not provided, the value counts as absent).
    fn binding(&self, k: Kind, i: &EngineInput, elapsed: f32) -> Option<f32> {
        Some(match k {
            Kind::Speed => i.speed_mph,
            Kind::GearElapsed => elapsed,
            Kind::Lfo => self.lfo.value(),
            Kind::ShuntLfo => self.shunt.value(),
            Kind::Load => self.load_value,
            Kind::ListenerDistance => self.listener_distance?,
            _ => return None,
        })
    }

    /// Evaluate the effect and LFO items of the running sequences (the game's sound update, which posts them to
    /// the vehicle for its next tick).
    fn evaluate_effects(&mut self, dt: f32, i: &EngineInput, elapsed: f32) -> Effects {
        let mut fx = Effects::default();
        let (mut lfo, mut shunt) = (None, None);
        for s in &self.fx {
            if s.gate.is_some() && s.gate != self.open_gate {
                continue;
            }
            let Some(x) = self.binding(s.binding, i, elapsed) else { continue };
            let x = if s.max > s.min { x.clamp(s.min, s.max) } else { x };
            for it in &s.items {
                if !(x >= it.start && x <= it.end) {
                    continue;
                }
                let t = if it.end > it.start { (x - it.start) / (it.end - it.start) } else { 0.0 };
                let mut rate_depth = (None, None);
                for (prop, m, k) in &it.values {
                    let b = if m.binding.is_empty() {
                        None
                    } else {
                        match self.binding(*k, i, elapsed) {
                            Some(v) => Some(v),
                            // A binding this driver does not provide (suspension / traction load, pitch): the
                            // value is left out rather than read as 0.
                            None => continue,
                        }
                    };
                    let v = m.eval(t, b);
                    if it.lfo {
                        match prop {
                            0 => rate_depth.0 = Some(v),
                            1 => rate_depth.1 = Some(v),
                            _ => {}
                        }
                        continue;
                    }
                    match prop {
                        1 => fx.rpm_offset += v,
                        3 => fx.load_multiplier *= v,
                        4 => fx.load_floor = fx.load_floor.max(v),
                        _ => {}
                    }
                }
                if it.lfo {
                    let slot = if s.shunt { &mut shunt } else { &mut lfo };
                    *slot = Some(rate_depth);
                }
            }
        }
        for (l, set) in [(&mut self.lfo, lfo), (&mut self.shunt, shunt)] {
            match set {
                Some((rate, depth)) => {
                    l.rate = rate.unwrap_or(l.rate);
                    l.depth = depth.unwrap_or(l.depth);
                }
                None => l.depth = 0.0,
            }
            l.phase = (l.phase + std::f32::consts::TAU * l.rate * dt) % std::f32::consts::TAU;
        }
        fx
    }

    /// Advance by `dt` seconds with this tick's engine state.
    pub fn update(&mut self, dt: f32, i: &EngineInput) {
        self.time += dt;
        let d = self.drive_state(dt, i);
        if !self.started {
            self.started = true;
            self.params.trigger(cue::IGNITION);
            self.shifts_seen = Some(d.shifts_started);
        }
        if self.shifts_seen != Some(d.shifts_started) {
            self.shifts_seen = Some(d.shifts_started);
            self.start_gear_change(d.last_shift_up, d.cruising);
        }
        let elapsed = d.gear_change_elapsed.min(1.0e6);
        // The sequences' effects of the last sound update, applied in this vehicle tick.
        let effects = if self.use_fx { self.effects } else { Effects::default() };
        // `FUN_009aea50` runs before `FUN_009b1f70`: the RPM curve crossfades by the previous load.
        let norm =
            normalised_rpm_with(&self.tuning, i.rpm, i.idle_rpm, i.redline_rpm, self.load_value, effects.rpm_offset);
        let load = self.load.update(
            dt,
            d.engine_throttle.clamp(0.0, 1.0),
            i.speed_mph.abs() / i.top_speed_mph.max(1.0),
            self.speed_load,
            d.clutch.clamp(0.0, 1.0),
            self.tuning.load_rate,
            effects.load_multiplier,
            effects.load_floor,
        );
        self.load_value = load;
        // Exhaust pops (`FUN_009b20b0`), unless the caller runs them in its physics tick. Offline renders have
        // no gearbox: the pedal stands in for the gearbox throttle.
        if !self.external_pops {
            let input = PopInput {
                fraction: norm,
                idle_rpm: i.idle_rpm,
                max_rpm: i.redline_rpm,
                gearbox_throttle: i.throttle,
                running: true,
                cruising: d.cruising,
            };
            let event = self.pops.tick(&self.pop_params, &input);
            self.fire(event);
            self.pop_probability = self.pops.probability;
        }
        // The rev limiter (`FUN_0058cdb0`): only in the gearbox's last two gears with the normalised RPM above
        // 0.99. Each cycle restarts the limiter sequence and draws the next cycle's rate from the data.
        let limiting = d.top_gears && norm > LIMITER_FRACTION;
        if limiting && !self.limiter_on {
            self.limiter_on = true;
            self.limiter_phase = 0.0;
            self.limiter_cycle();
        } else if self.limiter_on {
            self.limiter_phase += self.limiter_rate * dt;
            if self.limiter_phase >= 1.0 {
                self.limiter_phase -= self.limiter_phase.floor();
                if limiting {
                    self.limiter_cycle();
                } else {
                    self.limiter_on = false;
                }
            }
        }
        self.jitter_timer -= dt;
        if self.jitter_timer <= 0.0 {
            self.jitter_timer = 0.05;
            self.jitter = self.random();
        }
        self.write_values(i, norm, load, elapsed);
        // The sound update: evaluate the running sequences' effects for the next tick.
        self.effects = self.evaluate_effects(dt, i, elapsed);
        self.state = SoundState {
            load,
            normalised: norm,
            audible: self.audible(i, norm),
            gear_elapsed: elapsed,
            effects,
            limiter: self.limiter_on,
        };
    }

    fn limiter_cycle(&mut self) {
        self.params.trigger(cue::LIMITER);
        let (lo, hi) = self.tuning.limiter_rate;
        let r = self.random();
        self.limiter_rate = (lo + (hi - lo) * r).max(0.5);
    }

    /// What the last update computed.
    pub fn state(&self) -> SoundState {
        self.state
    }

    /// The car's pop rule data (`Engine.Sound`).
    pub fn set_pop_params(&mut self, p: PopParams) {
        self.pop_params = p;
    }

    /// The engine load the sound uses (the RPM curve crossfades by it), for the caller's pop rule.
    pub fn load(&self) -> f32 {
        self.load_value
    }

    pub fn tuning(&self) -> &EngineTuning {
        &self.tuning
    }

    /// Results of the pop rule run by the caller in its physics tick: `GetPopProbability` and the tick's
    /// event. Once called, [`EngineDriver::update`] stops running its own.
    pub fn push_pops(&mut self, probability: f32, event: PopEvent) {
        self.external_pops = true;
        self.pop_probability = probability;
        self.fire(event);
    }

    /// A backfire starts `Backfire_Sequence`, a pop `Pop_Sequence` (`FUN_0058ccb0`, which starts each only
    /// when its previous instance has ended).
    fn fire(&self, event: PopEvent) {
        match event {
            PopEvent::Backfire => self.params.trigger(cue::BACKFIRE),
            PopEvent::Pop => self.params.trigger(cue::POP),
            PopEvent::None => {}
        }
    }

    fn write(&mut self, i: &EngineInput, elapsed: f32) {
        let norm = normalised_rpm_with(&self.tuning, i.rpm, i.idle_rpm, i.redline_rpm, self.load_value, 0.0);
        self.write_values(i, norm, self.load_value, elapsed);
    }

    fn write_values(&self, i: &EngineInput, norm: f32, load: f32, elapsed: f32) {
        let audible = self.audible(i, norm);
        let firing = firing_frequency(&self.tuning, audible);
        let names = self.params.names();
        for &(slot, kind) in &self.slots {
            let v = match kind {
                Kind::AudibleRpm => audible,
                Kind::NormalisedRpm => norm,
                Kind::Load => load,
                Kind::EngineOn => 1.0,
                Kind::Speed => i.speed_mph,
                Kind::Harmonic1 => firing * self.tuning.harmonic1,
                Kind::Harmonic2 => firing * self.tuning.harmonic2,
                Kind::Subharmonic => firing * self.tuning.subharmonic,
                Kind::PopProbability => self.pop_probability,
                // The getter (`LAB_005a22e0`) is not in the dump: full while the limiter runs (assumed).
                Kind::RevLimitGain => f32::from(u8::from(self.limiter_on)),
                // The sequences clamp it to their axis (0..15 / 0..6).
                Kind::GearElapsed => elapsed.min(1000.0),
                Kind::GearGate(l, k) => f32::from(u8::from(self.open_gate == Some((l, k)))),
                Kind::Jitter => self.jitter,
                Kind::Lfo => self.lfo.value(),
                Kind::ShuntLfo => self.shunt.value(),
                // `FUN_00aabc60` (a sawtooth, not read): kept as a 9 Hz ramp.
                Kind::SkidLfo => (self.time * 9.0).fract() - 0.5,
                Kind::Random(n) => ((self.jitter * n as f32) as u32).min(n.saturating_sub(1)) as f32,
                Kind::ListenerDistance => match self.listener_distance {
                    Some(d) => d,
                    None => continue,
                },
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
        // The 911: curve terms 0.7 → exponent 1.3 over the range above 5 %.
        let t = EngineTuning { rpm_curve_on: 0.7, rpm_curve_off: 0.7, ..EngineTuning::default() };
        let n = normalised_rpm(&t, 4400.0, 1000.0, 7800.0, 1.0);
        assert!((n - (0.45f32 / 0.95).powf(1.3)).abs() < 1e-4, "{n}");
        assert_eq!(normalised_rpm(&t, 900.0, 1000.0, 7800.0, 1.0), 0.0);
        assert_eq!(normalised_rpm(&t, 1300.0, 1000.0, 7800.0, 1.0), 0.0);
        assert_eq!(normalised_rpm(&t, 9000.0, 1000.0, 7800.0, 1.0), 1.0);
        // Terms of 1 give a linear curve; the load picks between the on / off terms.
        let lin = EngineTuning { rpm_curve_on: 1.0, rpm_curve_off: 0.5, ..EngineTuning::default() };
        let x = 0.45 / 0.95;
        assert!((normalised_rpm(&lin, 4400.0, 1000.0, 7800.0, 1.0) - x).abs() < 1e-6);
        assert!((normalised_rpm(&lin, 4400.0, 1000.0, 7800.0, 0.0) - x.powf(1.5)).abs() < 1e-4);
        // Effect offsets add after the curve and the sum is clamped.
        assert!((normalised_rpm_with(&lin, 4400.0, 1000.0, 7800.0, 1.0, 0.2) - (x + 0.2)).abs() < 1e-6);
        assert_eq!(normalised_rpm_with(&lin, 7700.0, 1000.0, 7800.0, 1.0, 0.2), 1.0);
        // Six cylinders at 4000 rpm fire 200 times a second.
        assert!((firing_frequency(&t, 4000.0) - 200.0).abs() < 1e-3);
    }

    #[test]
    fn load_follows_the_engine_throttle_at_half_per_tick() {
        let mut f = LoadFilter::default();
        let dt = 1.0 / 60.0;
        for _ in 0..60 {
            f.update(dt, 1.0, 0.0, 0.75, 1.0, 0.5, 1.0, 0.0);
        }
        assert!(f.update(dt, 1.0, 0.0, 0.75, 1.0, 0.5, 1.0, 0.0) > 0.99);
        // An upshift cuts the engine's throttle: the load halves every tick (`ref/xkr_launch`: 0.99 → 0.40 →
        // 0.11), with no hold.
        let a = f.update(dt, 0.0, 0.0, 0.75, 0.0, 0.5, 1.0, 0.0);
        assert!((a - 0.5).abs() < 0.01, "{a}");
        let b = f.update(dt, 0.0, 0.0, 0.75, 0.0, 0.5, 1.0, 0.0);
        assert!((b - 0.25).abs() < 0.01, "{b}");
        // Off the pedal at the top speed with the clutch in, the load leans towards full: (0.75)² = 0.56; with
        // the clutch open it does not.
        let mut g = LoadFilter::default();
        for _ in 0..200 {
            g.update(dt, 0.0, 1.0, 0.75, 1.0, 0.5, 1.0, 0.0);
        }
        assert!((g.update(dt, 0.0, 1.0, 0.75, 1.0, 0.5, 1.0, 0.0) - 0.5625).abs() < 0.01);
        for _ in 0..200 {
            g.update(dt, 0.0, 1.0, 0.75, 0.0, 0.5, 1.0, 0.0);
        }
        assert!(g.update(dt, 0.0, 1.0, 0.75, 0.0, 0.5, 1.0, 0.0) < 0.01);
        // Multipliers and floors apply to the output, not the state.
        assert!((g.update(dt, 0.0, 1.0, 0.75, 0.0, 0.5, 1.0, 0.7) - 0.7).abs() < 1e-6);
    }
}
