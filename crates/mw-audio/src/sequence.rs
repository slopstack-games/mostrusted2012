//! The game's sound sequences and mixer graph, as data plus a player.
//!
//! Most Wanted's sounds are described by GeneSys objects (research log 2026-10-04, "sound sequences"):
//!
//! - A **Sequence** has a *binding* (a game value named by a string such as
//!   `"Players.LocalPlayer.GetPlayerAudibleRPM"`) clamped to `Binding_Min..Binding_Max`, and items placed on
//!   that axis between `StartTime` and `EndTime`. A sequence without a binding is a timeline: started by an
//!   event, its axis is seconds since the start.
//! - A **WaveSequenceItem** plays one of its `Waves` while the axis is inside its range, faded in and out
//!   over `Fade_In.Time` / `Fade_Out.Time` (axis units, shaped by `Curve` as an exponent), with `Gain` and
//!   `Pitch` given by *modulating values*. Looping waves (the SPS header's loop flag) run while the item is
//!   active; one-shots fire when the axis enters the range.
//! - A **GinsuSequenceItem** plays a Ginsu recording granularly at the axis value (RPM).
//! - A **MixerChannelSequenceItem** automates a mixer channel's gain or a plug-in parameter.
//! - A **modulating value** maps its own binding through `Binding_Range_Min..Max` (then `Binding_Invert`,
//!   `Binding_Exponent`) and its curve (`AnimPoints`) to `Output_Value_Min..Max`; without a binding the
//!   curve's input is the item's normalised position on the sequence axis; with neither it is `Value`.
//! - **Mixer channels** have a gain and a plug-in chain (EQ, distortion, sub-harmoniser, sends to other
//!   channels). Sounds play into a channel; the chain processes it and sends it on, ending at the master.
//!
//! [`Bank`] holds this data (built by `mw_assets::sound` from the GeneSys objects); [`Player`] renders it.
//! Game values arrive through [`Params`], which is shared with the game thread.
//!
//! Not decoded / assumed (marked where used): the two modulation-type enums (`Binding_Modulation_Type`,
//! `Animation_Modulation_Type`) are ignored, the curve tangents (curves are piecewise linear), the plug-in
//! formulas (see [`crate::dsp`]), channel gain applied before the chain.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::dsp::{self, Biquad};
use crate::ginsu::{Ginsu, GinsuSynth};

/// Output sample rate of the player.
pub const OUTPUT_RATE: u32 = 48_000;
/// Control-rate block: modulations, envelopes and filter coefficients update every this many samples.
const BLOCK: usize = 32;

/// Plug-in field name hashes (`mw_formats::genesys::field_hash`).
pub mod field {
    pub const GAIN: u32 = 0x4761_696e;
    pub const FREQUENCY: u32 = 0x0609_6291;
    pub const Q: u32 = 0x7c56_e7e1;
    pub const ORDER: u32 = 0xcf5a_e8f3;
    pub const BANDWIDTH: u32 = 0x3fd6_b877;
    pub const DISTORTION: u32 = 0xce99_ab25;
    pub const INTERVAL: u32 = 0x76c1_a226;
    pub const TIME: u32 = 0x5469_6d65;
    pub const FEEDBACK: u32 = 0x499b_1089;
}

/// A decoded wave (mono).
#[derive(Debug, Clone)]
pub struct Clip {
    pub samples: Vec<i16>,
    pub rate: u32,
    pub looping: bool,
}

/// `SequenceItem.ModulatingDoubleValue`.
#[derive(Debug, Clone)]
pub struct ModValue {
    pub binding: String,
    pub exponent: f32,
    pub invert: bool,
    pub evaluate_once: bool,
    pub range: (f32, f32),
    pub out: (f32, f32),
    pub value: f32,
    /// `(x, y)` points, sorted by x.
    pub curve: Vec<(f32, f32)>,
}

impl ModValue {
    pub fn constant(value: f32) -> Self {
        Self {
            binding: String::new(),
            exponent: 1.0,
            invert: false,
            evaluate_once: false,
            range: (0.0, 1.0),
            out: (0.0, 1.0),
            value,
            curve: Vec::new(),
        }
    }

    /// Value at an item position `t` (0..1) with the binding's current value `b`.
    pub fn eval(&self, t: f32, b: Option<f32>) -> f32 {
        let input = match b {
            Some(v) => {
                let (lo, hi) = self.range;
                let mut r = if (hi - lo).abs() > 1e-9 {
                    ((v - lo) / (hi - lo)).clamp(0.0, 1.0)
                } else if v >= lo {
                    1.0
                } else {
                    0.0
                };
                if self.invert {
                    r = 1.0 - r;
                }
                if self.exponent > 0.0 && self.exponent != 1.0 { r.powf(self.exponent) } else { r }
            }
            None if !self.curve.is_empty() => t.clamp(0.0, 1.0),
            None => return self.value,
        };
        let c = if self.curve.is_empty() { input } else { curve_at(&self.curve, input) };
        self.out.0 + (self.out.1 - self.out.0) * c
    }
}

/// Piecewise-linear curve lookup, clamped at the ends.
pub fn curve_at(curve: &[(f32, f32)], x: f32) -> f32 {
    let Some(first) = curve.first() else { return x };
    if x <= first.0 {
        return first.1;
    }
    for w in curve.windows(2) {
        let ((a, ya), (b, yb)) = (w[0], w[1]);
        if x <= b {
            let f = if b > a { (x - a) / (b - a) } else { 0.0 };
            return ya + (yb - ya) * f;
        }
    }
    curve.last().map_or(x, |l| l.1)
}

/// `Fade_In` / `Fade_Out`: length on the sequence axis and the exponent of the ramp.
#[derive(Debug, Clone, Copy, Default)]
pub struct Fade {
    pub time: f32,
    pub curve: f32,
}

/// What a mixer automation sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The item's channel gain (multiplied with the channel's static gain).
    ChannelGain,
    /// A field of the first plug-in in the item's channel chain that has it.
    ChannelPlugin(u32),
    /// A field of the plug-in with this unique id.
    Plugin { uid: u32, field: u32 },
}

#[derive(Debug, Clone)]
// Items are built once per bank (a few hundred), so the size difference doesn't matter.
#[allow(clippy::large_enum_variant)]
pub enum ItemKind {
    Wave {
        waves: Vec<Arc<Clip>>,
        gain: ModValue,
        pitch: ModValue,
        probability: Option<ModValue>,
    },
    Ginsu {
        ginsu: Arc<Ginsu>,
    },
    Mixer {
        params: Vec<(Target, ModValue)>,
    },
    /// `RaceCarEffectSequenceItem`: `(Automated_Property, value)` pairs the game posts to the car's engine model
    /// (message 0x2a, `FUN_008f5490`). The sound itself plays nothing for them; the engine driver evaluates them
    /// (`mw_assets::engine_driver`): 1 adds to the normalised RPM, 3 multiplies the engine load, 4 is a floor
    /// under it.
    Effect {
        values: Vec<(u32, ModValue)>,
    },
    /// `LfoSequenceItem`: `(Automated_Property, value)` pairs that drive the engine sound's LFO (0 = rate in Hz,
    /// 1 = depth); evaluated by the engine driver, which supplies the `…Lfo` bindings.
    Lfo {
        values: Vec<(u32, ModValue)>,
    },
}

#[derive(Debug, Clone)]
pub struct Item {
    pub start: f32,
    pub end: f32,
    pub fade_in: Fade,
    pub fade_out: Fade,
    /// Item plays only while this binding is true (empty = always).
    pub enabled: String,
    /// Mixer channel id (`MixerChannel` resource); `None` plays straight to the master.
    pub channel: Option<u64>,
    pub kind: ItemKind,
}

impl Item {
    /// Envelope at axis position `x`: 0 outside the range, fades inside it.
    pub fn envelope(&self, x: f32) -> f32 {
        if !(x >= self.start && x <= self.end) {
            return 0.0;
        }
        let ramp = |d: f32, f: Fade| {
            if f.time <= 0.0 {
                return 1.0;
            }
            let r = (d / f.time).clamp(0.0, 1.0);
            if f.curve > 0.0 && f.curve != 1.0 { r.powf(f.curve) } else { r }
        };
        ramp(x - self.start, self.fade_in) * ramp(self.end - x, self.fade_out)
    }
}

#[derive(Debug, Clone)]
pub struct Sequence {
    /// Resource id, for diagnostics.
    pub id: u64,
    /// Empty: a timeline started by a cue.
    pub binding: String,
    pub min: f32,
    pub max: f32,
    pub items: Vec<Item>,
    /// The sequence only plays while this binding is true (set by the bank builder, empty = always).
    pub gate: String,
    /// One-shot items fire again whenever the previous one ends while the item is active, at a rate
    /// proportional to the binding (pops and crackles: `Gurgle_Sequence` is bound to a pop probability).
    pub retrigger: bool,
    /// Started only by its cue, and a cue is ignored while the previous start still sounds (the game's
    /// pop and backfire sequences, `FUN_0058ccb0`). A bound sequence started this way plays the one-shot
    /// items whose range holds the binding at that moment.
    pub exclusive_cue: bool,
}

/// Plug-in kinds the player implements. Others (pans, limiters, reverb) pass the signal through.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PluginKind {
    /// Send the signal at this point of the chain to another channel (`None`: a bus outside the bank,
    /// treated as the master).
    Send {
        to: Option<u64>,
    },
    Peaking,
    HighShelf,
    LowPass,
    HighPass,
    BandPass,
    Distortion,
    SubHarmoniser,
    Delay,
    Gain,
    Passthrough,
}

#[derive(Debug, Clone)]
pub struct Plugin {
    pub uid: u32,
    pub kind: PluginKind,
    /// Static parameter values by field-name hash ([`field`]).
    pub fields: Vec<(u32, f32)>,
}

impl Plugin {
    pub fn get(&self, f: u32) -> Option<f32> {
        self.fields.iter().find(|(h, _)| *h == f).map(|(_, v)| *v)
    }
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub id: u64,
    pub gain: f32,
    pub plugins: Vec<Plugin>,
    /// The channel's mixing group (`MixingGroup`).
    pub group: u32,
    /// Mixing groups this channel's submix collects (`SubmixDspPlugIn.Mixing_Groups`): channels of those
    /// groups without a chain of their own play into this one.
    pub collects: Vec<u32>,
}

/// Everything one player renders.
#[derive(Debug, Clone, Default)]
pub struct Bank {
    pub channels: Vec<Channel>,
    pub sequences: Vec<Sequence>,
    /// Named events → candidate timeline sequences (one is picked at random per trigger).
    pub cues: Vec<(String, Vec<usize>)>,
    pub master_gain: f32,
}

impl Bank {
    /// Base names of every binding used (expression suffixes such as `(NOT)` stripped).
    pub fn param_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        let mut add = |s: &str| {
            if let Some(e) = Expr::parse(s)
                && !names.contains(&e.base)
            {
                names.push(e.base);
            }
        };
        let mod_names = |m: &ModValue, add: &mut dyn FnMut(&str)| add(&m.binding);
        for s in &self.sequences {
            add(&s.binding);
            add(&s.gate);
            for it in &s.items {
                add(&it.enabled);
                match &it.kind {
                    ItemKind::Wave { gain, pitch, probability, .. } => {
                        mod_names(gain, &mut add);
                        mod_names(pitch, &mut add);
                        if let Some(p) = probability {
                            mod_names(p, &mut add);
                        }
                    }
                    ItemKind::Mixer { params } => params.iter().for_each(|(_, m)| mod_names(m, &mut add)),
                    ItemKind::Effect { values } | ItemKind::Lfo { values } => {
                        values.iter().for_each(|(_, m)| mod_names(m, &mut add))
                    }
                    ItemKind::Ginsu { .. } => {}
                }
            }
        }
        names
    }

    /// Merge another bank (channels with the same id are kept once).
    pub fn merge(&mut self, other: Bank) {
        let base = self.sequences.len();
        for c in other.channels {
            if !self.channels.iter().any(|x| x.id == c.id) {
                self.channels.push(c);
            }
        }
        self.sequences.extend(other.sequences);
        for (name, seqs) in other.cues {
            let seqs = seqs.into_iter().map(|i| i + base);
            match self.cues.iter_mut().find(|(n, _)| *n == name) {
                Some((_, v)) => v.extend(seqs),
                None => self.cues.push((name, seqs.collect())),
            }
        }
    }
}

/// A binding string: a game value name plus an optional test, as the data writes them:
/// `"LocalPlayer.IsInTunnel(NOT)"`, `"LocalPlayer.GetPlayerSpeed(GTEQ:100)"`, `"...(BETWEEN,20,60)"`,
/// `"...GetEngineLoad(LTEQ:0.1)"`, `"Signals.GetRandomNumber[2](EQUALS:0)"`.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub base: String,
    pub op: Op,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    Value,
    Not,
    Gteq(f32),
    Lteq(f32),
    Equals(f32),
    Between(f32, f32),
}

impl Expr {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().trim_end_matches(" TRUE").trim();
        if s.is_empty() {
            return None;
        }
        let Some(open) = s.rfind('(').filter(|_| s.ends_with(')')) else {
            return Some(Self { base: s.to_string(), op: Op::Value });
        };
        let (base, arg) = (&s[..open], &s[open + 1..s.len() - 1]);
        let num = |t: &str| t.trim().parse::<f32>().ok();
        let op = if arg == "NOT" {
            Op::Not
        } else if let Some(v) = arg.strip_prefix("GTEQ:").and_then(num) {
            Op::Gteq(v)
        } else if let Some(v) = arg.strip_prefix("LTEQ:").and_then(num) {
            Op::Lteq(v)
        } else if let Some(v) = arg.strip_prefix("EQUALS:").and_then(num) {
            Op::Equals(v)
        } else if let Some(rest) = arg.strip_prefix("BETWEEN,") {
            let mut it = rest.split(',').filter_map(num);
            Op::Between(it.next()?, it.next()?)
        } else {
            Op::Value
        };
        Some(Self { base: base.to_string(), op })
    }

    pub fn apply(&self, v: f32) -> f32 {
        self.op.apply(v)
    }
}

impl Op {
    pub fn apply(self, v: f32) -> f32 {
        let b = |c: bool| if c { 1.0 } else { 0.0 };
        match self {
            Op::Value => v,
            Op::Not => b(v < 0.5),
            Op::Gteq(t) => b(v >= t),
            Op::Lteq(t) => b(v <= t),
            Op::Equals(t) => b((v - t).abs() < 1e-3),
            Op::Between(a, c) => b(v >= a && v <= c),
        }
    }
}

/// Game values shared between the game thread (writer) and the audio thread (reader), plus a queue of
/// cue triggers.
pub struct Params {
    names: Vec<String>,
    index: HashMap<String, usize>,
    values: Vec<AtomicU32>,
    /// Smoothing time constant per value (s).
    smoothing: Vec<AtomicU32>,
    triggers: Mutex<Vec<String>>,
}

impl Params {
    pub fn new(names: Vec<String>) -> Self {
        let index = names.iter().enumerate().map(|(i, n)| (n.clone(), i)).collect();
        let values = names.iter().map(|_| AtomicU32::new(0f32.to_bits())).collect();
        let smoothing = names.iter().map(|_| AtomicU32::new(0.02f32.to_bits())).collect();
        Self { names, index, values, smoothing, triggers: Mutex::new(Vec::new()) }
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn index(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }

    /// Set a value by name (ignored when no sequence uses it).
    pub fn set(&self, name: &str, v: f32) {
        if let Some(i) = self.index(name) {
            self.values[i].store(v.to_bits(), Ordering::Relaxed);
        }
    }

    pub fn get(&self, i: usize) -> f32 {
        self.values.get(i).map_or(0.0, |v| f32::from_bits(v.load(Ordering::Relaxed)))
    }

    /// How fast the player follows a value (one-pole time constant in seconds; 0 = immediately).
    pub fn set_smoothing(&self, name: &str, seconds: f32) {
        if let Some(i) = self.index(name) {
            self.smoothing[i].store(seconds.to_bits(), Ordering::Relaxed);
        }
    }

    fn smoothing(&self, i: usize) -> f32 {
        f32::from_bits(self.smoothing[i].load(Ordering::Relaxed))
    }

    /// Start a cue (a timeline sequence; unknown cues are ignored).
    pub fn trigger(&self, cue: &str) {
        if let Ok(mut t) = self.triggers.lock()
            && t.len() < 64
        {
            t.push(cue.to_string());
        }
    }

    fn take_triggers(&self) -> Vec<String> {
        self.triggers.try_lock().map(|mut t| std::mem::take(&mut *t)).unwrap_or_default()
    }
}

// --- Player ------------------------------------------------------------------------------------------

/// A resolved binding: parameter index and test.
#[derive(Debug, Clone, Copy)]
struct Bind {
    param: usize,
    op: Op,
}

fn bind(params: &Params, s: &str) -> Option<Bind> {
    let e = Expr::parse(s)?;
    Some(Bind { param: params.index(&e.base)?, op: e.op })
}

struct RMod {
    m: ModValue,
    bind: Option<Bind>,
}

struct ItemState {
    enabled: Option<Bind>,
    channel: Option<usize>,
    gain: Option<RMod>,
    pitch: Option<RMod>,
    probability: Option<RMod>,
    params: Vec<(Target, RMod)>,
    /// Ginsu voice index in the group.
    ginsu: Option<usize>,
    active: bool,
    voice: Option<usize>,
    held: Option<(f32, f32)>,
}

struct SeqState {
    bind: Option<Bind>,
    gate: Option<Bind>,
    /// Timelines: running and their time.
    playing: bool,
    t: f32,
    duration: f32,
    items: Vec<ItemState>,
    /// A bound exclusive-cue sequence was started this block.
    cued: bool,
}

struct WaveVoice {
    clip: Arc<Clip>,
    pos: f64,
    step: f64,
    gain: f32,
    dgain: f32,
    target: f32,
    channel: Option<usize>,
    owner: (usize, usize),
    stopping: bool,
    done: bool,
}

struct GinsuVoice {
    synth: GinsuSynth,
    channel: Option<usize>,
    gain: f32,
    dgain: f32,
    /// Recording rate over output rate.
    rate_ratio: f32,
}

/// All Ginsu items share one period schedule and one random stream, so recordings made in the same run
/// (two microphones, three load sweeps) start their engine cycles together (research log "engine sound
/// rewritten"). The first voice sets the period.
struct GinsuGroup {
    voices: Vec<GinsuVoice>,
    rpm: f32,
    target: f32,
    /// RPM glide (the game's Ginsu player ramps linearly to a new RPM over 33 ms): start value and
    /// samples left.
    glide_from: f32,
    glide_left: f32,
    t: f32,
    period: f32,
    seed: u32,
}

/// The game's Ginsu player glides to a new RPM over this long.
const RPM_GLIDE_SECONDS: f32 = 0.033;

impl GinsuGroup {
    fn next(&mut self, sr: f32, out: &mut [f32]) {
        if self.voices.is_empty() {
            return;
        }
        if self.glide_left > 0.0 {
            let n = RPM_GLIDE_SECONDS * sr;
            self.glide_left -= 1.0;
            let f = 1.0 - (self.glide_left / n).clamp(0.0, 1.0);
            self.rpm = self.glide_from + (self.target - self.glide_from) * f;
        } else {
            self.rpm = self.target;
        }
        if self.t >= self.period {
            self.t -= self.period;
            self.seed ^= self.seed << 13;
            self.seed ^= self.seed >> 17;
            self.seed ^= self.seed << 5;
            for v in &mut self.voices {
                v.synth.start_period(self.rpm, self.seed);
            }
            let v0 = &self.voices[0];
            self.period = v0.synth.period_for(self.rpm) / v0.rate_ratio.max(1e-3);
        }
        let x = (self.t / self.period).min(1.0);
        self.t += 1.0;
        for v in &mut self.voices {
            v.gain = (v.gain + v.dgain).max(0.0);
            if v.gain > 1e-5 {
                let s = v.synth.sample(x) * v.gain;
                match v.channel {
                    Some(c) => out[c] += s,
                    None => out[out.len() - 1] += s,
                }
            }
        }
    }
}

struct PluginState {
    plugin: Plugin,
    /// Current values (static or automated), by field hash.
    values: Vec<(u32, f32)>,
    biquads: Vec<Biquad>,
    sub: dsp::SubHarmoniser,
    delay: dsp::Delay,
    dirty: bool,
    send: Option<Option<usize>>,
}

impl PluginState {
    fn value(&self, f: u32, default: f32) -> f32 {
        self.values.iter().find(|(h, _)| *h == f).map_or(default, |(_, v)| *v)
    }

    fn set(&mut self, f: u32, v: f32) {
        match self.values.iter_mut().find(|(h, _)| *h == f) {
            Some((_, old)) => {
                if (*old - v).abs() > 1e-6 * old.abs().max(1.0) {
                    *old = v;
                    self.dirty = true;
                }
            }
            None => {
                self.values.push((f, v));
                self.dirty = true;
            }
        }
    }

    fn update(&mut self, sr: f32) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let freq = self.value(field::FREQUENCY, 0.0);
        match self.plugin.kind {
            PluginKind::Peaking => {
                self.biquads.resize(1, Biquad::IDENTITY);
                let (g, q) = (self.value(field::GAIN, 1.0), self.value(field::Q, 0.7));
                self.biquads[0].peaking(sr, freq, g, q);
            }
            PluginKind::HighShelf => {
                self.biquads.resize(1, Biquad::IDENTITY);
                let g = self.value(field::GAIN, 1.0);
                self.biquads[0].high_shelf(sr, freq, g);
            }
            PluginKind::LowPass | PluginKind::HighPass => {
                let qs = dsp::butterworth_q(self.value(field::ORDER, 2.0) as u32);
                self.biquads.resize(qs.len(), Biquad::IDENTITY);
                for (b, q) in self.biquads.iter_mut().zip(qs) {
                    if self.plugin.kind == PluginKind::LowPass {
                        b.low_pass(sr, freq, q)
                    } else {
                        b.high_pass(sr, freq, q)
                    }
                }
            }
            PluginKind::BandPass => {
                self.biquads.resize(1, Biquad::IDENTITY);
                let bw = self.value(field::BANDWIDTH, freq).max(1.0);
                self.biquads[0].band_pass(sr, freq, freq / bw);
            }
            _ => {}
        }
    }

    #[inline]
    fn process(&mut self, x: f32, sr: f32, bypass: Bypass) -> f32 {
        match self.plugin.kind {
            PluginKind::Peaking
            | PluginKind::HighShelf
            | PluginKind::LowPass
            | PluginKind::HighPass
            | PluginKind::BandPass => {
                if bypass.eq {
                    x
                } else {
                    self.biquads.iter_mut().fold(x, |x, b| b.process(x))
                }
            }
            PluginKind::Distortion if bypass.distortion => x,
            PluginKind::Distortion if bypass.raw_distortion => dsp::distort_raw(x, self.value(field::DISTORTION, 1.0)),
            PluginKind::Distortion => dsp::distort(x, self.value(field::DISTORTION, 1.0)),
            PluginKind::SubHarmoniser if bypass.sub_harmoniser => x,
            PluginKind::SubHarmoniser => self.sub.process(x, self.value(field::INTERVAL, 1.0) as u32, sr),
            PluginKind::Delay => {
                let (t, fb) = (self.value(field::TIME, 0.0), self.value(field::FEEDBACK, 0.0));
                self.delay.process(x, t, fb, sr)
            }
            PluginKind::Gain => x * self.value(field::GAIN, 1.0),
            PluginKind::Send { .. } | PluginKind::Passthrough => x,
        }
    }
}

struct ChannelState {
    gain_static: f32,
    gain: f32,
    dgain: f32,
    plugins: Vec<PluginState>,
    /// Channels without sends play straight into the master.
    has_send: bool,
}

/// Which plug-in families to bypass, for A/B listening (`MW_AUDIO_DRY=1` bypasses them all; `MW_AUDIO_NO_DIST`,
/// `MW_AUDIO_NO_SUB`, `MW_AUDIO_NO_EQ` one family each; `MW_AUDIO_DIST=raw` uses the uncompensated
/// `tanh(d · x)` waveshaper).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Bypass {
    pub distortion: bool,
    pub sub_harmoniser: bool,
    pub eq: bool,
    pub raw_distortion: bool,
}

impl Bypass {
    pub fn from_env() -> Self {
        let set = |k: &str| std::env::var_os(k).is_some();
        Self {
            distortion: set("MW_AUDIO_NO_DIST"),
            sub_harmoniser: set("MW_AUDIO_NO_SUB"),
            eq: set("MW_AUDIO_NO_EQ"),
            raw_distortion: std::env::var("MW_AUDIO_DIST").is_ok_and(|v| v == "raw"),
        }
    }
}

/// Spreads a value's changes over time. The game sets values once per frame while the audio thread renders a
/// whole device buffer at a time, so every sample of a buffer would see the same target and the pitch would
/// climb in steps of the buffer length. Each new target is instead reached by a linear ramp as long as the
/// interval (in control blocks) since the previous change, which makes the curve continuous at the cost of one
/// update interval of latency.
#[derive(Debug, Clone, Copy)]
struct Ramp {
    from: f32,
    to: f32,
    pos: u32,
    len: u32,
    since_change: u32,
}

/// `MW_AUDIO_NO_RAMP=1` turns the ramps off (A/B).
fn no_ramp() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var_os("MW_AUDIO_NO_RAMP").is_some())
}

/// Longest ramp, in control blocks (~0.15 s at 44.1 kHz).
const MAX_RAMP_BLOCKS: u32 = 200;

impl Ramp {
    fn new(v: f32) -> Self {
        Self { from: v, to: v, pos: 1, len: 1, since_change: 0 }
    }

    fn value(&self) -> f32 {
        self.from + (self.to - self.from) * (self.pos as f32 / self.len as f32).min(1.0)
    }

    /// One control block with `target` as the latest value; returns the ramped value.
    fn step(&mut self, target: f32) -> f32 {
        self.since_change = self.since_change.saturating_add(1);
        if target.to_bits() != self.to.to_bits() {
            self.from = self.value();
            self.to = target;
            self.len = self.since_change.clamp(1, MAX_RAMP_BLOCKS);
            self.pos = 0;
            self.since_change = 0;
        }
        self.pos = (self.pos + 1).min(self.len);
        self.value()
    }
}

/// Renders a [`Bank`] driven by [`Params`].
pub struct Player {
    bank: Bank,
    params: Arc<Params>,
    smoothed: Vec<f32>,
    /// Per value: a linear ramp towards the latest target, spread over the time between the game's updates
    /// as the audio thread sees them (`Ramp`).
    ramps: Vec<Ramp>,
    seqs: Vec<SeqState>,
    channels: Vec<ChannelState>,
    order: Vec<usize>,
    /// Position of each channel in `order`.
    rank: Vec<usize>,
    /// Seconds rendered (control-rate clock).
    time: f32,
    /// Per-sample channel inputs; the extra last slot is the master.
    inputs: Vec<f32>,
    voices: Vec<WaveVoice>,
    ginsu: GinsuGroup,
    rng: Rng,
    block: usize,
    sr: f32,
    /// Bypass the plug-ins (A/B listening, `MW_AUDIO_DRY`).
    pub dry: bool,
    /// Bypass some plug-in families ([`Bypass`]).
    pub bypass: Bypass,
    /// Sum of squares of each channel's input after its gain (diagnostics, see [`Player::enable_meter`]).
    meter: Option<Vec<f64>>,
}

/// xorshift32.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// Overall level before the soft limiter.
const MIX_SCALE: f32 = 0.12;

impl Player {
    pub fn new(bank: Bank, params: Arc<Params>) -> Self {
        let sr = OUTPUT_RATE as f32;
        let chan_index: HashMap<u64, usize> = bank.channels.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
        let mut channels: Vec<ChannelState> = bank
            .channels
            .iter()
            .map(|c| {
                let plugins: Vec<PluginState> = c
                    .plugins
                    .iter()
                    .map(|p| PluginState {
                        plugin: p.clone(),
                        values: p.fields.clone(),
                        biquads: Vec::new(),
                        sub: Default::default(),
                        delay: Default::default(),
                        dirty: true,
                        send: match p.kind {
                            PluginKind::Send { to } => Some(to.and_then(|id| chan_index.get(&id).copied())),
                            _ => None,
                        },
                    })
                    .collect();
                let mut plugins = plugins;
                // A channel without sends plays into the submix that collects its mixing group (the 911's
                // idle loop joins the engine's set-A processing this way), else into the master.
                if !plugins.iter().any(|p| p.send.is_some()) {
                    let collector = bank.channels.iter().position(|o| o.id != c.id && o.collects.contains(&c.group));
                    if let Some(to) = collector {
                        plugins.push(PluginState {
                            plugin: Plugin {
                                uid: 0,
                                kind: PluginKind::Send { to: Some(bank.channels[to].id) },
                                fields: vec![],
                            },
                            values: Vec::new(),
                            biquads: Vec::new(),
                            sub: Default::default(),
                            delay: Default::default(),
                            dirty: false,
                            send: Some(Some(to)),
                        });
                    }
                }
                let sends: Vec<Option<usize>> = plugins.iter().filter_map(|p| p.send).collect();
                ChannelState { gain_static: c.gain, gain: c.gain, dgain: 0.0, has_send: !sends.is_empty(), plugins }
            })
            .collect();
        for c in &mut channels {
            for p in &mut c.plugins {
                p.update(sr);
            }
        }
        // Process order: senders before receivers (Kahn); cycles fall back to bank order.
        let n = channels.len();
        let mut indeg = vec![0usize; n];
        for c in &channels {
            for t in c.plugins.iter().filter_map(|p| p.send.flatten()) {
                indeg[t] += 1;
            }
        }
        let mut order = Vec::with_capacity(n);
        let mut ready: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
        while let Some(i) = ready.pop() {
            order.push(i);
            for t in channels[i].plugins.iter().filter_map(|p| p.send.flatten()) {
                indeg[t] -= 1;
                if indeg[t] == 0 {
                    ready.push(t);
                }
            }
        }
        for i in 0..n {
            if !order.contains(&i) {
                order.push(i);
            }
        }

        let mut ginsu = GinsuGroup {
            voices: Vec::new(),
            rpm: 0.0,
            target: 0.0,
            glide_from: 0.0,
            glide_left: 0.0,
            t: 0.0,
            period: 1000.0,
            seed: 0x9e37_79b9,
        };
        let rmod = |m: &ModValue| RMod { m: m.clone(), bind: bind(&params, &m.binding) };
        let seqs: Vec<SeqState> = bank
            .sequences
            .iter()
            .map(|s| SeqState {
                bind: bind(&params, &s.binding),
                gate: bind(&params, &s.gate),
                playing: false,
                cued: false,
                t: 0.0,
                duration: s.items.iter().map(|i| i.end).fold(s.max, f32::max),
                items: s
                    .items
                    .iter()
                    .map(|it| {
                        let channel = it.channel.and_then(|id| chan_index.get(&id).copied());
                        let mut st = ItemState {
                            enabled: bind(&params, &it.enabled),
                            channel,
                            gain: None,
                            pitch: None,
                            probability: None,
                            params: Vec::new(),
                            ginsu: None,
                            active: false,
                            voice: None,
                            held: None,
                        };
                        match &it.kind {
                            ItemKind::Wave { gain, pitch, probability, .. } => {
                                st.gain = Some(rmod(gain));
                                st.pitch = Some(rmod(pitch));
                                st.probability = probability.as_ref().map(rmod);
                            }
                            ItemKind::Mixer { params: ps } => {
                                st.params = ps.iter().map(|(t, m)| (*t, rmod(m))).collect();
                            }
                            ItemKind::Ginsu { ginsu: g } => {
                                st.ginsu = Some(ginsu.voices.len());
                                ginsu.voices.push(GinsuVoice {
                                    synth: GinsuSynth::new(g.clone()),
                                    channel,
                                    gain: 0.0,
                                    dgain: 0.0,
                                    rate_ratio: g.pcm.sample_rate as f32 / sr,
                                });
                            }
                            ItemKind::Effect { .. } | ItemKind::Lfo { .. } => {}
                        }
                        st
                    })
                    .collect(),
            })
            .collect();
        if let Some(v) = ginsu.voices.first() {
            ginsu.rpm = v.synth.ginsu.min_rpm;
            ginsu.target = ginsu.rpm;
            ginsu.period = v.synth.period_for(ginsu.rpm) / v.rate_ratio.max(1e-3);
        }
        let smoothed: Vec<f32> = (0..params.names().len()).map(|i| params.get(i)).collect();
        let ramps = smoothed.iter().map(|&v| Ramp::new(v)).collect();
        Self {
            inputs: vec![0.0; n + 1],
            bank,
            params,
            smoothed,
            ramps,
            seqs,
            channels,
            rank: {
                let mut r = vec![0; order.len()];
                for (k, &c) in order.iter().enumerate() {
                    r[c] = k;
                }
                r
            },
            order,
            time: 0.0,
            voices: Vec::new(),
            ginsu,
            rng: Rng(0x2545_f491),
            block: 0,
            sr,
            dry: false,
            bypass: Bypass::default(),
            meter: None,
        }
    }

    pub fn params(&self) -> &Arc<Params> {
        &self.params
    }

    fn value(&self, b: Option<Bind>) -> Option<f32> {
        b.map(|b| b.op.apply(self.smoothed.get(b.param).copied().unwrap_or(0.0)))
    }

    /// Start metering channel levels ([`Player::meter`]).
    pub fn enable_meter(&mut self) {
        self.meter = Some(vec![0.0; self.channels.len()]);
    }

    /// Channel ids with the energy (sum of squares) of their input so far.
    pub fn meter(&self) -> Vec<(u64, f64)> {
        let Some(m) = &self.meter else { return Vec::new() };
        self.bank.channels.iter().zip(m).map(|(c, e)| (c.id, *e)).collect()
    }

    /// Number of active wave voices (diagnostics).
    pub fn voice_count(&self) -> usize {
        self.voices.len()
    }

    fn control(&mut self) {
        let dt = BLOCK as f32 / self.sr;
        self.time += dt;
        let warming = self.time < 0.2;
        for i in 0..self.smoothed.len() {
            let tau = self.params.smoothing(i);
            let ramp = tau > 0.0 && !no_ramp();
            let target = if ramp { self.ramps[i].step(self.params.get(i)) } else { self.params.get(i) };
            let k = if tau <= 0.0 { 1.0 } else { (dt / tau).min(1.0) };
            self.smoothed[i] += (target - self.smoothed[i]) * k;
            if !self.smoothed[i].is_finite() {
                self.smoothed[i] = 0.0;
            }
        }
        for cue in self.params.take_triggers() {
            let Some((_, cands)) = self.bank.cues.iter().find(|(n, _)| *n == cue) else { continue };
            if cands.is_empty() {
                continue;
            }
            let pick = cands[self.rng.next() as usize % cands.len()];
            if self.bank.sequences[pick].exclusive_cue {
                let busy = self.seqs[pick].playing || self.voices.iter().any(|v| v.owner.0 == pick && !v.done);
                if busy {
                    continue;
                }
                if self.seqs[pick].bind.is_some() {
                    self.seqs[pick].cued = true;
                    continue;
                }
            }
            let s = &mut self.seqs[pick];
            s.playing = true;
            s.t = 0.0;
            for it in &mut s.items {
                it.active = false;
                it.held = None;
            }
        }

        let nch = self.channels.len();
        let mut gain_mul = vec![1.0f32; nch];
        let mut overrides: Vec<(usize, Target, f32)> = Vec::new();
        let mut starts: Vec<(usize, usize)> = Vec::new();
        for si in 0..self.seqs.len() {
            let seq = &self.bank.sequences[si];
            let gate_ok = self.value(self.seqs[si].gate).is_none_or(|v| v > 0.5);
            let bound = self.seqs[si].bind.is_some();
            let raw = self.value(self.seqs[si].bind);
            let x = if bound {
                let v = raw.unwrap_or(0.0);
                if seq.max > seq.min { Some(v.clamp(seq.min, seq.max)) } else { Some(v) }
            } else if self.seqs[si].playing {
                self.seqs[si].t += dt;
                Some(self.seqs[si].t - dt)
            } else {
                None
            };
            let x = x.filter(|_| gate_ok);
            for ii in 0..seq.items.len() {
                let item = &seq.items[ii];
                let enabled = self.value(self.seqs[si].items[ii].enabled).is_none_or(|v| v > 0.5);
                let env = match x {
                    Some(x) if enabled => item.envelope(x),
                    _ => 0.0,
                };
                let inside = matches!(x, Some(x) if enabled && x >= item.start && x <= item.end);
                let t_item =
                    x.map_or(
                        0.0,
                        |x| {
                            if item.end > item.start { (x - item.start) / (item.end - item.start) } else { 0.0 }
                        },
                    );
                let was_active = self.seqs[si].items[ii].active;
                self.seqs[si].items[ii].active = inside;
                let st = &self.seqs[si].items[ii];
                match &item.kind {
                    ItemKind::Ginsu { .. } => {
                        if let Some(g) = st.ginsu {
                            if g == 0
                                && let Some(x) = x
                                && (x - self.ginsu.target).abs() > 1e-3
                            {
                                // A new target starts a 33 ms glide from the current RPM.
                                self.ginsu.glide_from = self.ginsu.rpm;
                                self.ginsu.glide_left = RPM_GLIDE_SECONDS * self.sr;
                                self.ginsu.target = x;
                            }
                            let v = &mut self.ginsu.voices[g];
                            v.dgain = (env - v.gain) / BLOCK as f32;
                        }
                    }
                    ItemKind::Effect { .. } | ItemKind::Lfo { .. } => {}
                    ItemKind::Mixer { .. } => {
                        if inside {
                            for (target, m) in &st.params {
                                let v = m.m.eval(t_item, self.value(m.bind));
                                match (target, st.channel) {
                                    (Target::ChannelGain, Some(c)) => gain_mul[c] *= v.max(0.0),
                                    (Target::ChannelGain, None) => {}
                                    (t, c) => overrides.push((c.unwrap_or(usize::MAX), *t, v)),
                                }
                            }
                        }
                    }
                    ItemKind::Wave { waves, .. } => {
                        if waves.is_empty() {
                            continue;
                        }
                        let looping = waves.iter().all(|w| w.looping);
                        let eval =
                            |m: &Option<RMod>, d: f32| m.as_ref().map_or(d, |m| m.m.eval(t_item, self.value(m.bind)));
                        let gain = eval(&st.gain, 1.0);
                        let pitch = eval(&st.pitch, 1.0);
                        if looping {
                            match st.voice {
                                Some(v) if v < self.voices.len() && self.voices[v].owner == (si, ii) => {
                                    let voice = &mut self.voices[v];
                                    voice.target = env * gain;
                                    voice.step = pitch.max(0.05) as f64 * voice.clip.rate as f64 / self.sr as f64;
                                    voice.stopping = env <= 0.0;
                                }
                                _ if env > 0.0 => starts.push((si, ii)),
                                _ => {}
                            }
                        } else {
                            // No one-shots while the game is still setting its values.
                            let entering = if seq.exclusive_cue && bound {
                                inside && self.seqs[si].cued
                            } else {
                                inside && !was_active && !(warming && bound)
                            };
                            let retrigger = seq.retrigger
                                && inside
                                && !warming
                                && !st.voice.is_some_and(|v| v < self.voices.len() && self.voices[v].owner == (si, ii));
                            if entering && !seq.retrigger {
                                starts.push((si, ii));
                            } else if retrigger {
                                // About 12 per second at full binding value.
                                let rate = 12.0 * raw.unwrap_or(1.0).clamp(0.0, 1.0) * env.max(0.2);
                                if self.rng.unit() < rate * dt {
                                    starts.push((si, ii));
                                }
                            }
                        }
                    }
                }
            }
            self.seqs[si].cued = false;
            // Timelines end once past their last item and silent.
            if !bound && self.seqs[si].playing && self.seqs[si].t > self.seqs[si].duration {
                let busy = self.voices.iter().any(|v| v.owner.0 == si && !v.done);
                if !busy {
                    self.seqs[si].playing = false;
                    for it in &mut self.seqs[si].items {
                        it.active = false;
                    }
                }
            }
        }
        for (si, ii) in starts {
            self.start_voice(si, ii);
        }
        // Voice gain ramps; drop finished voices (indices held by items are re-checked by owner).
        for v in &mut self.voices {
            let target = if v.stopping { 0.0 } else { v.target };
            v.dgain = (target - v.gain) / BLOCK as f32;
            if v.stopping && v.gain <= 1e-4 {
                v.done = true;
            }
        }
        if self.voices.iter().any(|v| v.done) {
            let mut map = vec![None; self.voices.len()];
            let mut kept = Vec::with_capacity(self.voices.len());
            for (i, v) in std::mem::take(&mut self.voices).into_iter().enumerate() {
                if !v.done {
                    map[i] = Some(kept.len());
                    kept.push(v);
                }
            }
            self.voices = kept;
            for s in &mut self.seqs {
                for it in &mut s.items {
                    it.voice = it.voice.and_then(|v| map.get(v).copied().flatten());
                }
            }
        }
        // Channel gains and plug-in parameters.
        for (c, ch) in self.channels.iter_mut().enumerate() {
            let target = ch.gain_static * gain_mul[c];
            ch.dgain = (target - ch.gain) / BLOCK as f32;
            for p in &mut ch.plugins {
                // Back to static values unless automated below.
                for k in 0..p.plugin.fields.len() {
                    let (f, v) = p.plugin.fields[k];
                    p.set(f, v);
                }
            }
        }
        for (c, target, v) in overrides {
            match target {
                Target::Plugin { uid, field } => {
                    for ch in &mut self.channels {
                        for p in ch.plugins.iter_mut().filter(|p| p.plugin.uid == uid) {
                            p.set(field, v);
                        }
                    }
                }
                Target::ChannelPlugin(field) => {
                    if let Some(ch) = self.channels.get_mut(c)
                        && let Some(p) = ch.plugins.iter_mut().find(|p| p.plugin.get(field).is_some())
                    {
                        p.set(field, v);
                    }
                }
                Target::ChannelGain => {}
            }
        }
        let sr = self.sr;
        for ch in &mut self.channels {
            for p in &mut ch.plugins {
                p.update(sr);
            }
        }
    }

    fn start_voice(&mut self, si: usize, ii: usize) {
        let item = &self.bank.sequences[si].items[ii];
        let ItemKind::Wave { waves, .. } = &item.kind else { return };
        if waves.is_empty() {
            return;
        }
        let st = &self.seqs[si].items[ii];
        if let Some(p) = &st.probability {
            let pr = p.m.eval(0.0, self.value(p.bind));
            if self.rng.unit() >= pr {
                return;
            }
        }
        let gain = st.gain.as_ref().map_or(1.0, |m| m.m.eval(0.0, self.value(m.bind)));
        let pitch = st.pitch.as_ref().map_or(1.0, |m| m.m.eval(0.0, self.value(m.bind)));
        let channel = st.channel;
        let pick = self.rng.next() as usize % waves.len();
        let clip = waves[pick].clone();
        let looping = clip.looping;
        let step = pitch.max(0.05) as f64 * clip.rate as f64 / self.sr as f64;
        // Looping voices fade in from silence at a random point of the loop (layers sharing a wave don't
        // phase); one-shots start at full gain.
        let pos = if looping { (self.rng.next() as usize % clip.samples.len().max(1)) as f64 } else { 0.0 };
        let gain0 = if looping { 0.0 } else { gain };
        self.voices.push(WaveVoice {
            clip,
            pos,
            step,
            gain: gain0,
            dgain: 0.0,
            target: gain,
            channel,
            owner: (si, ii),
            stopping: false,
            done: false,
        });
        self.seqs[si].items[ii].voice = Some(self.voices.len() - 1);
        if self.voices.len() > 96 {
            // Voice limit: drop the oldest one-shot.
            if let Some(i) = self.voices.iter().position(|v| !v.clip.looping) {
                self.voices[i].stopping = true;
            }
        }
    }

    /// Next output sample (mono, -1..1).
    pub fn next_sample(&mut self) -> f32 {
        self.next_sample_linear().tanh()
    }

    /// Next output sample before the soft limiter.
    pub fn next_sample_linear(&mut self) -> f32 {
        if self.block == 0 {
            self.control();
        }
        self.block = (self.block + 1) % BLOCK;
        let n = self.channels.len();
        let master = n;
        self.ginsu.next(self.sr, &mut self.inputs);
        for v in &mut self.voices {
            v.gain = (v.gain + v.dgain).max(0.0);
            let len = v.clip.samples.len();
            if len == 0 {
                v.done = true;
                continue;
            }
            let i = v.pos as usize;
            let s = if i < len {
                let f = (v.pos - i as f64) as f32;
                let a = v.clip.samples[i] as f32;
                let b = if i + 1 < len {
                    v.clip.samples[i + 1] as f32
                } else if v.clip.looping {
                    v.clip.samples[0] as f32
                } else {
                    0.0
                };
                (a + (b - a) * f) / 32768.0
            } else {
                0.0
            };
            let out = s * v.gain;
            match v.channel {
                Some(c) => self.inputs[c] += out,
                None => self.inputs[master] += out,
            }
            v.pos += v.step;
            if v.pos >= len as f64 {
                if v.clip.looping {
                    v.pos -= len as f64;
                } else {
                    v.done = true;
                    v.stopping = true;
                    v.gain = 0.0;
                }
            }
        }
        let sr = self.sr;
        let bypass = self.bypass;
        for k in 0..self.order.len() {
            let c = self.order[k];
            let ch = &mut self.channels[c];
            ch.gain = (ch.gain + ch.dgain).max(0.0);
            let mut x = self.inputs[c] * ch.gain;
            if let Some(m) = self.meter.as_mut() {
                m[c] += (x * x) as f64;
            }
            if x == 0.0 && ch.plugins.iter().all(|p| p.biquads.is_empty()) {
                continue;
            }
            let mut sent_master = false;
            for p in &mut ch.plugins {
                if let Some(to) = p.send {
                    // `SendDspPlugIn.Gain` (1 in the data; a per-car override can mute a send).
                    let sent = x * p.value(field::GAIN, 1.0);
                    match to {
                        Some(t) => {
                            // Senders come before receivers; a send back up the order would close a
                            // feedback loop (the 911's two vehicle buses send to each other) and is dropped.
                            if self.rank[t] > k {
                                self.inputs[t] += sent;
                            }
                        }
                        None if !sent_master => {
                            self.inputs[master] += sent;
                            sent_master = true;
                        }
                        None => {}
                    }
                    continue;
                }
                if !self.dry {
                    x = p.process(x, sr, bypass);
                }
            }
            if !ch.has_send {
                self.inputs[master] += x;
            }
        }
        let out = self.inputs[master] * self.bank.master_gain * MIX_SCALE;
        self.inputs.iter_mut().for_each(|v| *v = 0.0);
        if out.is_finite() { out } else { 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expressions() {
        let e = Expr::parse("LocalPlayer.IsInTunnel(NOT)").unwrap();
        assert_eq!(e.base, "LocalPlayer.IsInTunnel");
        assert_eq!(e.apply(1.0), 0.0);
        let e = Expr::parse("LocalPlayer.GetPlayerSpeed(BETWEEN,20,60)").unwrap();
        assert_eq!((e.apply(10.0), e.apply(30.0)), (0.0, 1.0));
        let e = Expr::parse("RaceCars.Entities[\"Id\"].GetSpeedMph").unwrap();
        assert_eq!(e.op, Op::Value);
        assert!(Expr::parse("").is_none());
    }

    #[test]
    fn modulating_value() {
        let mut m = ModValue::constant(0.3);
        assert_eq!(m.eval(0.5, None), 0.3);
        m.binding = "x".into();
        m.range = (40.0, 80.0);
        m.out = (1.0, 1.5);
        assert!((m.eval(0.0, Some(60.0)) - 1.25).abs() < 1e-6);
        assert_eq!(m.eval(0.0, Some(100.0)), 1.5);
        m.binding.clear();
        m.curve = vec![(0.0, 0.0), (0.5, 1.0), (1.0, 1.0)];
        m.out = (0.0, 2.0);
        assert!((m.eval(0.25, None) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn envelope_fades_inside_the_range() {
        let it = Item {
            start: 45.0,
            end: 90.0,
            fade_in: Fade { time: 20.0, curve: 1.0 },
            fade_out: Fade { time: 20.0, curve: 1.0 },
            enabled: String::new(),
            channel: None,
            kind: ItemKind::Mixer { params: vec![] },
        };
        assert_eq!(it.envelope(40.0), 0.0);
        assert!((it.envelope(55.0) - 0.5).abs() < 1e-6);
        assert_eq!(it.envelope(67.0), 1.0);
        assert!((it.envelope(80.0) - 0.5).abs() < 1e-6);
    }

    fn sine_clip(f: f32, looping: bool) -> Arc<Clip> {
        let samples =
            (0..48_000).map(|i| ((std::f32::consts::TAU * f * i as f32 / 48_000.0).sin() * 16000.0) as i16).collect();
        Arc::new(Clip { samples, rate: 48_000, looping })
    }

    #[test]
    fn bound_loop_follows_its_binding_through_a_send() {
        let wave = Item {
            start: 10.0,
            end: 100.0,
            fade_in: Fade { time: 10.0, curve: 1.0 },
            fade_out: Fade::default(),
            enabled: String::new(),
            channel: Some(1),
            kind: ItemKind::Wave {
                waves: vec![sine_clip(440.0, true)],
                gain: ModValue::constant(1.0),
                pitch: ModValue::constant(1.0),
                probability: None,
            },
        };
        let bank = Bank {
            channels: vec![
                Channel {
                    id: 1,
                    gain: 1.0,
                    plugins: vec![Plugin { uid: 5, kind: PluginKind::Send { to: Some(2) }, fields: vec![] }],
                    group: 0,
                    collects: vec![],
                },
                Channel { id: 2, gain: 0.5, plugins: vec![], group: 0, collects: vec![] },
            ],
            sequences: vec![Sequence {
                id: 0,
                binding: "Speed".into(),
                min: 0.0,
                max: 200.0,
                items: vec![wave],
                gate: String::new(),
                retrigger: false,
                exclusive_cue: false,
            }],
            cues: vec![],
            master_gain: 1.0,
        };
        let params = Arc::new(Params::new(bank.param_names()));
        let mut p = Player::new(bank, params.clone());
        let rms = |p: &mut Player| {
            let v: Vec<f32> = (0..4800).map(|_| p.next_sample()).collect();
            (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt()
        };
        params.set("Speed", 5.0);
        rms(&mut p);
        assert!(rms(&mut p) < 1e-4, "silent below the range");
        params.set("Speed", 50.0);
        rms(&mut p);
        let loud = rms(&mut p);
        // Sine of amplitude 16000/32768 (RMS 0.345) × channel 0.5 × MIX_SCALE: RMS ≈ 0.0207.
        assert!((loud - 0.345 * 0.5 * MIX_SCALE).abs() < 0.003, "{loud}");
    }
}
