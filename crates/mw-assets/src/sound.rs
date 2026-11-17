//! GeneSys sound objects → [`mw_audio::sequence::Bank`]: sequences, their items, the mixer channels they
//! play into (following sends through the plug-in chains) and the waves (decoded once and cached).
//!
//! Field names were cracked against their hashes and checked against the values (research log
//! 2026-10-04, "sound sequences"); unnamed ones are written as hashes here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mw_audio::sequence::{
    Bank, Channel, Clip, Controller, ControllerAction, Fade, Item, ItemKind, ModValue, Plugin, PluginKind, Sequence,
    Target, field,
};
use mw_bundle::ResourceId;
use mw_formats::genesys::{Object, Value, field_hash};

use crate::AssetStore;

const START_TIME: u32 = 0x6c82_122b;
const END_TIME: u32 = 0xddb9_c614;
const FADE_OUT: u32 = 0xa50f_68e1;
const POINT_X: u32 = 0x12d3_a8aa;
const POINT_Y: u32 = 0x12d3_a8ab;
const MIXER_CHANNEL_CHAIN: u32 = 0x1a00_9288;
const CHAIN_PLUGINS: u32 = 0x1d8b_4b21;
const TRIGGER_PROBABILITY: u32 = 0xdb46_6b79;
/// `JumpTimelineController.DestinationTime` (cracked 2026-10-15; it sorts before `TriggerTime` as the field order
/// requires).
const DESTINATION_TIME: u32 = 0xb6a5_1260;
/// `SequenceTimelineController.2d0e4348` (bool, name not cracked): read as "monitor the binding"
/// ([`mw_audio::sequence::Controller`]).
const CONTROLLER_MONITOR: u32 = 0x2d0e_4348;

pub(crate) fn number(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f32),
        Value::UInt(u) => Some(*u as f32),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn num_hash(o: &Object, hash: u32) -> Option<f32> {
    number(o.get(hash))
}

fn num(o: &Object, name: &str) -> Option<f32> {
    number(o.get(field_hash(name)))
}

fn text(o: &Object, hash: u32) -> String {
    match o.get(hash) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

pub(crate) fn handle_of(v: Option<&Value>) -> Option<ResourceId> {
    match v? {
        Value::Handle(h) => *h,
        Value::Ref(r) => Some(*r),
        _ => None,
    }
}

pub(crate) fn handle(o: &Object, name: &str) -> Option<ResourceId> {
    handle_of(o.get(field_hash(name)))
}

pub(crate) fn handles(o: &Object, hash: u32) -> Vec<ResourceId> {
    match o.get(hash) {
        Some(Value::Array(a)) => a.iter().filter_map(|v| handle_of(Some(v))).collect(),
        _ => Vec::new(),
    }
}

fn numbers(o: &Object, hash: u32) -> Vec<f32> {
    match o.get(hash) {
        Some(Value::Array(a)) => a.iter().filter_map(|v| number(Some(v))).collect(),
        _ => Vec::new(),
    }
}

fn structs(o: &Object, hash: u32) -> impl Iterator<Item = &Object> {
    let items = match o.get(hash) {
        Some(Value::Array(a)) => a.as_slice(),
        _ => &[],
    };
    items.iter().filter_map(|v| match v {
        Value::Struct(s) => Some(s),
        _ => None,
    })
}

fn sub(o: &Object, hash: u32) -> Option<&Object> {
    match o.get(hash)? {
        Value::Struct(s) => Some(s),
        _ => None,
    }
}

/// `SequenceItem.ModulatingDoubleValue`.
fn mod_value(o: &Object) -> ModValue {
    let mut curve: Vec<(f32, f32)> = structs(o, field_hash("AnimPoints"))
        .filter_map(|p| Some((number(p.get(POINT_X))?, number(p.get(POINT_Y))?)))
        .collect();
    curve.sort_by(|a, b| a.0.total_cmp(&b.0));
    ModValue {
        binding: text(o, field_hash("Binding")),
        exponent: num(o, "Binding_Exponent").unwrap_or(1.0),
        invert: num(o, "Binding_Invert").unwrap_or(0.0) > 0.5,
        evaluate_once: num(o, "Binding_Evaluate_Once").unwrap_or(0.0) > 0.5,
        range: (num(o, "Binding_Range_Min").unwrap_or(0.0), num(o, "Binding_Range_Max").unwrap_or(1.0)),
        out: (num(o, "Output_Value_Min").unwrap_or(0.0), num(o, "Output_Value_Max").unwrap_or(1.0)),
        value: num(o, "Value").unwrap_or(1.0),
        curve,
    }
}

fn fade(o: Option<&Object>) -> Fade {
    o.map_or_else(Fade::default, |f| Fade {
        time: num(f, "Time").unwrap_or(0.0),
        curve: num(f, "Curve").unwrap_or(1.0),
    })
}

/// Builds banks, caching decoded waves and Ginsu recordings across calls.
pub struct SoundLoader<'a> {
    store: &'a AssetStore,
    waves: Mutex<HashMap<ResourceId, Option<Arc<Clip>>>>,
    ginsus: Mutex<HashMap<ResourceId, Option<Arc<mw_audio::ginsu::Ginsu>>>>,
    /// Channel ids reached so far, in discovery order.
    channels: Vec<ResourceId>,
}

impl<'a> SoundLoader<'a> {
    pub fn new(store: &'a AssetStore) -> Self {
        Self { store, waves: Default::default(), ginsus: Default::default(), channels: Vec::new() }
    }

    pub fn object(&self, id: ResourceId) -> Option<Object> {
        self.store.decode_genesys(&self.store.load(id).ok()?).ok()
    }

    /// A `Wave` resource: an SPS stream at +0x80; the stream header's loop flag says whether it loops.
    pub fn wave(&self, id: ResourceId) -> Option<Arc<Clip>> {
        if let Some(c) = self.waves.lock().ok()?.get(&id) {
            return c.clone();
        }
        let clip = (|| {
            let res = self.store.load(id).ok()?;
            let data = res.header().get(0x80..)?;
            let (h, _) = mw_audio::sps::header(data).ok()?;
            let pcm = mw_audio::sps::decode(data).ok()?;
            let samples = if pcm.channels > 1 {
                pcm.samples.chunks(pcm.channels as usize).map(|f| f[0]).collect()
            } else {
                pcm.samples
            };
            // The 0.1 s silent placeholder (`…79ff`) and empty decodes are skipped.
            if samples.iter().all(|&s| s == 0) {
                return None;
            }
            Some(Arc::new(Clip { samples, rate: pcm.sample_rate, looping: h.looping }))
        })();
        self.waves.lock().ok()?.insert(id, clip.clone());
        clip
    }

    pub fn ginsu(&self, id: ResourceId) -> Option<Arc<mw_audio::ginsu::Ginsu>> {
        if let Some(g) = self.ginsus.lock().ok()?.get(&id) {
            return g.clone();
        }
        let g = self.store.load(id).ok().and_then(|r| mw_audio::ginsu::Ginsu::parse(r.header()).ok()).map(Arc::new);
        self.ginsus.lock().ok()?.insert(id, g.clone());
        g
    }

    pub(crate) fn note_channel(&mut self, id: Option<ResourceId>) {
        if let Some(id) = id
            && !self.channels.contains(&id)
        {
            self.channels.push(id);
        }
    }

    /// A `Genesys.Gen.Sequence` (or `None` if it isn't one).
    pub fn sequence(&mut self, id: ResourceId) -> Option<Sequence> {
        let o = self.object(id)?;
        if o.type_name != "Genesys.Gen.Sequence" {
            return None;
        }
        let mut items = Vec::new();
        for it in structs(&o, field_hash("SequenceItems")) {
            let kind = it.type_name.rsplit('.').next().unwrap_or("");
            let channel = handle(it, "Mixer_Channel");
            let kind = match kind {
                "WaveSequenceItem" | "WaveSelectSequenceItem" => {
                    let waves: Vec<Arc<Clip>> =
                        handles(it, field_hash("Waves")).into_iter().filter_map(|w| self.wave(w)).collect();
                    if waves.is_empty() {
                        continue;
                    }
                    ItemKind::Wave {
                        waves,
                        gain: sub(it, field_hash("Gain")).map_or(ModValue::constant(1.0), mod_value),
                        pitch: sub(it, field_hash("Pitch")).map_or(ModValue::constant(1.0), mod_value),
                        probability: structs(it, TRIGGER_PROBABILITY).next().map(mod_value),
                        auto_pitch: num(it, "Auto_Pitch").unwrap_or(0.0) > 0.5,
                    }
                }
                "GinsuSequenceItem" => {
                    let Some(g) = handle(it, "Ginsu").and_then(|g| self.ginsu(g)) else { continue };
                    ItemKind::Ginsu { ginsu: g }
                }
                "MixerChannelSequenceItem" => {
                    let mut params = Vec::new();
                    for p in structs(it, field_hash("Double_Params")) {
                        let plugin = num(p, "PlugIn").unwrap_or(0.0) as u32;
                        let name = match p.get(field_hash("Parameter_Name")) {
                            Some(Value::UInt(u)) => *u as u32,
                            Some(Value::Int(i)) => *i as u32,
                            _ => 0,
                        };
                        let prop = num(p, "Automated_Property").unwrap_or(0.0) as u32;
                        let target = match (plugin, name, prop) {
                            (0, 0, 0) => Target::ChannelGain,
                            // Plug-in 0 with a parameter name: the first plug-in of the channel's chain that has
                            // that parameter (on the 911 the load curves set the 1 kHz peaking gain and the
                            // distortion amount of the strain channels this way).
                            (0, f, 9) if f != 0 => Target::ChannelPlugin(f),
                            (0, field::GAIN, _) => Target::ChannelGain,
                            // The channel's pitch (the supercharger whines, `Vehicles.Sound.TurboWhine`).
                            (0, 0, 6) => Target::ChannelPitch,
                            (0, 0, _) => continue,
                            (0, f, _) => Target::ChannelPlugin(f),
                            (uid, f, _) => Target::Plugin { uid, field: f },
                        };
                        let Some(v) = sub(p, field_hash("Value")) else { continue };
                        params.push((target, mod_value(v)));
                    }
                    if params.is_empty() {
                        continue;
                    }
                    ItemKind::Mixer { params }
                }
                // `Automated_Values` of `RaceCarEffectSequenceItem.EffectDoubleValue { ModulatingValue,
                // Automated_Property }` and `LfoSequenceItem.LfoDoubleValue { Value, Automated_Property }`.
                // `TrafficEffectSequenceItem` has the same layout; its property 0 is a traffic engine's RPM
                // (`mw_assets::car_sound`).
                "RaceCarEffectSequenceItem" | "LfoSequenceItem" | "TrafficEffectSequenceItem" => {
                    let lfo = kind == "LfoSequenceItem";
                    let value_field = field_hash(if lfo { "Value" } else { "ModulatingValue" });
                    let values: Vec<(u32, ModValue)> = structs(it, field_hash("Automated_Values"))
                        .filter_map(|v| {
                            let prop = num(v, "Automated_Property").unwrap_or(0.0) as u32;
                            Some((prop, mod_value(sub(v, value_field)?)))
                        })
                        .collect();
                    if values.is_empty() {
                        continue;
                    }
                    if lfo { ItemKind::Lfo { values } } else { ItemKind::Effect { values } }
                }
                _ => continue,
            };
            self.note_channel(channel);
            items.push(Item {
                start: number(it.get(START_TIME)).unwrap_or(0.0),
                end: number(it.get(END_TIME)).unwrap_or(0.0),
                fade_in: fade(sub(it, field_hash("Fade_In"))),
                fade_out: fade(sub(it, FADE_OUT)),
                enabled: text(it, field_hash("EnabledBinding")),
                channel: channel.map(|c| c.0),
                kind,
            });
        }
        Some(Sequence {
            id: id.0,
            binding: text(&o, field_hash("Binding")),
            min: num(&o, "Binding_Min").unwrap_or(0.0),
            max: num(&o, "Binding_Max").unwrap_or(0.0),
            items,
            gate: String::new(),
            retrigger: false,
            exclusive_cue: false,
            controllers: structs(&o, field_hash("TimelineControllers"))
                .filter_map(|c| {
                    let kind = c.type_name.rsplit('.').next().unwrap_or("");
                    let action = match kind {
                        "JumpTimelineController" => ControllerAction::Jump(number(c.get(DESTINATION_TIME))?),
                        "QuitSequenceTimelineController" => ControllerAction::Quit,
                        _ => return None,
                    };
                    Some(Controller {
                        at: num(c, "TriggerTime")?,
                        enabled: text(c, field_hash("EnabledBinding")),
                        monitor: number(c.get(CONTROLLER_MONITOR)).unwrap_or(0.0) > 0.5,
                        action,
                    })
                })
                .collect(),
        })
    }

    /// The mixer channels reached by the sequences loaded so far, following the sends of their plug-in
    /// chains.
    pub fn channels(&mut self) -> Vec<Channel> {
        let mut out = Vec::new();
        let mut k = 0;
        while k < self.channels.len() {
            let id = self.channels[k];
            k += 1;
            let Some(o) = self.object(id) else { continue };
            if o.type_name != "Genesys.Gen.MixerChannel" {
                continue;
            }
            let mut plugins = Vec::new();
            let mut collects = Vec::new();
            let chain = handle_of(o.get(MIXER_CHANNEL_CHAIN)).and_then(|c| self.object(c));
            for pid in chain.as_ref().map(|c| handles(c, CHAIN_PLUGINS)).unwrap_or_default() {
                let Some(p) = self.object(pid) else { continue };
                let kind = match p.type_name.rsplit('.').next().unwrap_or("") {
                    "SubmixDspPlugIn" => {
                        collects.extend(numbers(&p, field_hash("Mixing_Groups")).into_iter().map(|g| g as u32));
                        PluginKind::Passthrough
                    }
                    "SendDspPlugIn" => {
                        let to = handle(&p, "Voice_Routing");
                        self.note_channel(to);
                        PluginKind::Send { to: to.map(|t| t.0) }
                    }
                    "PeakingDspPlugIn" => PluginKind::Peaking,
                    "HighShelfDspPlugIn" => PluginKind::HighShelf,
                    "LowPassButterworthDspPlugIn" => PluginKind::LowPass,
                    "HighPassButterworthDspPlugIn" => PluginKind::HighPass,
                    "BandPassDspPlugIn" => PluginKind::BandPass,
                    "DistortionDspPlugIn" => PluginKind::Distortion,
                    "SubHarmoniserDspPlugIn" => PluginKind::SubHarmoniser,
                    "DelayDspPlugIn" => PluginKind::Delay,
                    "GainDspPlugIn" => PluginKind::Gain,
                    _ => PluginKind::Passthrough,
                };
                let fields = p.fields.iter().filter_map(|(h, v)| Some((*h, number(Some(v))?))).collect::<Vec<_>>();
                let uid = num(&p, "UniqueId").or_else(|| number(p.get(0x655d_3f5f))).unwrap_or(0.0) as u32;
                plugins.push(Plugin { uid, kind, fields });
            }
            let group = num(&o, "MixingGroup").unwrap_or(0.0) as u32;
            out.push(Channel { id: id.0, gain: num(&o, "Gain").unwrap_or(1.0), plugins, group, collects });
        }
        out
    }
}

/// `EngineSound2` tuning the game's engine-sound code reads (research log 2026-10-04, "engine sound from
/// the game code"). Field names are not recovered; the roles come from how the code uses them.
#[derive(Debug, Clone, Copy)]
pub struct EngineTuning {
    /// RPM curve terms (`a33d84fe` at full load, `cb44d82d` off load, `913bb900` in the alternate state):
    /// the audible RPM is `norm ^ (2 - a)` of the idle..max fraction, `a` crossfaded by load.
    pub rpm_curve_on: f32,
    pub rpm_curve_off: f32,
    pub rpm_curve_alt: f32,
    /// Engine-load response rate (`99d4b91e`), relative units.
    pub load_rate: f32,
    /// Cylinders (`ed754b99`): firing frequency = RPM / 120 × cylinders.
    pub cylinders: f32,
    /// Multiples of the firing frequency for the tracked harmonic 1 / 2 (`7cbc11ca`, `7cbc11c9`) and the
    /// sub-harmoniser (`283e07de`) bindings.
    pub harmonic1: f32,
    pub harmonic2: f32,
    pub subharmonic: f32,
    /// The rev limiter's cycle (`FUN_0058cdb0`): each cycle restarts the limiter sequence and draws its rate
    /// between `6e6be213` (+0x94) and `9529a29e` (+0x8c) (cycles per second, assumed: the phase's advance is
    /// not in the dump), 6..7 on the 911.
    pub limiter_rate: (f32, f32),
}

impl Default for EngineTuning {
    fn default() -> Self {
        Self {
            rpm_curve_on: 1.0,
            rpm_curve_off: 1.0,
            rpm_curve_alt: 1.0,
            load_rate: 0.5,
            cylinders: 6.0,
            harmonic1: 1.0,
            harmonic2: 3.0,
            subharmonic: 1.0,
            limiter_rate: (12.5, 12.5),
        }
    }
}

/// A car's engine as the game describes it (`Genesys.Gen.EngineSound2`).
#[derive(Debug, Clone)]
pub struct EngineBank {
    pub bank: Bank,
    /// Axis of the Ginsu sequences (`Binding_Min..Binding_Max` of the RPM sequence, 1790..7500 on the 911).
    pub rpm_range: (f32, f32),
    /// The audible RPM range (`GetPlayerAudibleRPM` = min + (max − min) × normalised RPM): the span of the Ginsu
    /// items of `Sequences` (`FUN_00575340`).
    pub audible_range: (f32, f32),
    /// Engine order (per crank revolution) with the most energy in the main recording (diagnostics).
    pub harmonic_order: f32,
    pub tuning: EngineTuning,
    /// Variants in each gear-change list ([`gate::LISTS`] order).
    pub gear_changes: [usize; 4],
    /// `Ignition_Sequence` timing: (seconds until it posts effect property 7, its length). Property 7 sets the
    /// engine-start flag +0x1be5 (`FUN_008f5490`) and the gearbox leaves its cranking state 1 for 2 when that
    /// flag or the "ignition finished" flag (+0x1be4, from `RaceCarEngineSound` +0x1c8) is up (`FUN_009cd7a0`,
    /// `FUN_00900120`); `None` = no ignition sequence, the engine catches at once (`FUN_0058c930`).
    pub ignition: Option<(f32, f32)>,
    /// The car's `Vehicles.Sound.TurboWhine` (the player's `VehicleSound` +0x58), when it has one.
    pub turbo: Option<TurboWhine>,
    /// The gain [`EngineBank::normalise`] gave the bank on top of its data level (1 when not normalised). The
    /// game plays the engine at its data level next to every other sound, so the player's other sounds follow
    /// it (`crates/mostrusted/src/sfx.rs`).
    pub normalisation: f32,
}

/// `Genesys.Gen.Vehicles.Sound.TurboWhine` as the turbo update (`FUN_0058f8f0`) reads it; its `Sequences`
/// (+0x40, the whine loop) are bound sequences of the bank, its one-shots cues ([`cue::BLOW_OFF`], [`cue::SPOOL`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct TurboWhine {
    /// The boost's response while falling: +0x44 `9fb6103d`, +0x48 `126fd4a0` (0.07 / 0.02 on most cars).
    pub fall: (f32, f32),
    /// While rising: +0x4c `ce79c7c7`, +0x50 `8e2e0ad7` (0.2 / 0.05).
    pub rise: (f32, f32),
    /// Length (s) of the blow-off sequences +0x0c, +0x14, +0x1c, +0x24 (`None`: no sequence).
    pub blow_off: [Option<f32>; 4],
    /// Length of the spool sequences +0x2c (boost above 0.5) and +0x34.
    pub spool: [Option<f32>; 2],
}

/// `TurboWhine` fields (offsets from the type table).
const TURBO_BLOW_OFF: [u32; 4] = [0xef6d_1e60, 0x3dba_62a6, 0x3dbf_63a5, 0x4c5b_8fb3];
const TURBO_SPOOL: [u32; 2] = [0x5352_dd98, 0xf675_9d3d];
const TURBO_RATES: [u32; 4] = [0x9fb6_103d, 0x126f_d4a0, 0xce79_c7c7, 0x8e2e_0ad7];

/// RMS of the calibration render (full throttle at 75 % of the rev range) every engine is scaled to.
const ENGINE_LEVEL: f32 = 0.3;

impl EngineBank {
    /// Scale the master gain so every car's engine has the same loudness at full throttle. The banks'
    /// mixes reach the master through buses outside the vehicle bundle (sends with no target), so their
    /// raw levels differ by over 20 dB between cars (Mito -31.5 dBFS, GT-R -7.6 on the same sweep); the
    /// game's levels per car are not known, so they are equalised.
    pub fn normalise(&mut self) {
        use crate::engine_driver::{EngineDriver, EngineInput};
        use mw_audio::sequence::{OUTPUT_RATE, Params, Player};
        let params = Arc::new(Params::new(self.bank.param_names()));
        let mut driver = EngineDriver::new(self, params.clone());
        let mut bank = self.bank.clone();
        // No one-shots in the calibration: they would make it random.
        bank.cues.clear();
        bank.sequences.iter_mut().filter(|s| s.retrigger).for_each(|s| s.items.clear());
        let mut player = Player::new(bank, params);
        let rate = OUTPUT_RATE as usize;
        let input = EngineInput {
            rpm: 5500.0,
            throttle: 1.0,
            gear: 3,
            speed_mph: 80.0,
            idle_rpm: 1000.0,
            redline_rpm: 7000.0,
            top_speed_mph: 180.0,
            drive: None,
        };
        let mut sum = 0f64;
        let n = rate;
        for i in 0..rate / 2 + n {
            if i % (rate / 64) == 0 {
                driver.update(1.0 / 64.0, &input);
            }
            let s = player.next_sample_linear();
            if i >= rate / 2 {
                sum += (s * s) as f64;
            }
        }
        let rms = (sum / n as f64).sqrt() as f32;
        if std::env::var_os("MW_AUDIO_METER").is_some() {
            eprintln!(
                "engine bank: calibration RMS {:.1} dBFS before normalising to {:.1}",
                20.0 * rms.max(1e-9).log10(),
                20.0 * ENGINE_LEVEL.log10()
            );
        }
        if rms > 1e-6 {
            self.normalisation = (ENGINE_LEVEL / rms).clamp(0.05, 50.0);
            self.bank.master_gain *= self.normalisation;
        }
    }
}

/// Cue names of the engine bank.
pub mod cue {
    pub const BACKFIRE: &str = "engine.backfire";
    pub const POP: &str = "engine.pop";
    pub const IGNITION: &str = "engine.ignition";
    pub const LIMITER: &str = "engine.limiter";
    /// The local player's nitrous timeline ([`AssetStore::nitrous_bank`]).
    pub const NITROUS: &str = "engine.nitrous";
    /// The turbo's blow-off sequences (`TurboWhine` +0x0c, +0x14, +0x1c, +0x24) and spool sequences (+0x2c,
    /// +0x34), started by the turbo update (`FUN_0058f8f0`).
    pub const BLOW_OFF: [&str; 4] = ["turbo.blowoff0", "turbo.blowoff1", "turbo.blowoff2", "turbo.blowoff3"];
    pub const SPOOL: [&str; 2] = ["turbo.spool0", "turbo.spool1"];
}

/// Gate bindings for the gear-change sequences (not game names: the game starts one sequence of a list per shift,
/// `FUN_0058ca50`; here every variant is in the bank behind its own gate and the driver opens one).
pub mod gate {
    /// The four lists of `EngineSound2` (`FUN_0058ca50`): +0x50 upshifts and +0x48 downshifts with the race
    /// gearbox in use (vehicle +0x1bdc = 0), +0x54 / +0x4c with the cruising one (+0x1bdc = 1).
    pub const LISTS: [&str; 4] = ["Up", "Down", "CruiseUp", "CruiseDown"];

    /// Gate of variant `k` of list `list` (an index into [`LISTS`]).
    pub fn gear_change(list: usize, k: usize) -> String {
        format!("Custom.GearChange.{}.{k}", LISTS[list])
    }

    /// The list and variant of a gate name.
    pub fn parse(name: &str) -> Option<(usize, usize)> {
        let rest = name.strip_prefix("Custom.GearChange.")?;
        let (list, k) = rest.split_once('.')?;
        Some((LISTS.iter().position(|l| *l == list)?, k.parse().ok()?))
    }
}

const ENGINE_SEQUENCES: u32 = 0x1f64_dde4;
const ENGINE_CHANNELS: u32 = 0x1d82_ab85;
/// Gear-change sequence groups: three variants with clunk waves, LFO shunt and a power cut (upshift), two
/// with clunks only (downshift). Which list is which state is not named in the data; picked by content.
/// `EngineSound2` +0x50, +0x48, +0x54, +0x4c: the gear-change lists in [`gate::LISTS`] order (field offsets from
/// the type table; `FUN_0058ca50` picks by the shift's direction and the gearbox mode).
const GEAR_CHANGE_LISTS: [u32; 4] = [0x83ea_3a92, 0xce81_9b08, 0x9492_327c, 0xc342_c096];
const REV_LIMITER_SEQUENCE: u32 = 0xc62d_11ac;

impl AssetStore {
    /// The engine bank of a vehicle bundle: the `EngineSound2` with the most sequences.
    pub fn engine_bank(&self, bundle: &mw_bundle::Bundle) -> crate::Result<Option<EngineBank>> {
        let mut best: Option<(ResourceId, Object)> = None;
        for e in bundle.entries_of_type(mw_bundle::ResourceType::GenesysObject) {
            let Ok(obj) = self.decode_genesys(&bundle.load(e)?) else { continue };
            if obj.type_name != "Genesys.Gen.EngineSound2" {
                continue;
            }
            let n = handles(&obj, ENGINE_SEQUENCES).len();
            if n > best.as_ref().map_or(0, |b| handles(&b.1, ENGINE_SEQUENCES).len()) {
                best = Some((e.id, obj));
            }
        }
        let Some((es_id, es)) = best else { return Ok(None) };
        let mut eb = self.engine_bank_from(&es, true);
        // The player's `VehicleSound` (the one pointing at this `EngineSound2`, +0x48): its transmission whine
        // (+0x50) and turbo whine (+0x58), started with the engine for the player's car (`FUN_0058c3f0`, entity
        // +0x1c18). Merged after the normalisation, so they keep the data's level against the engine.
        let vs = bundle
            .entries_of_type(mw_bundle::ResourceType::VehicleSound)
            .filter_map(|e| bundle.load(e).ok())
            .find(|r| r.import_at(0x48) == Some(es_id));
        if let Some(vs) = vs {
            self.add_whines(&mut eb, vs.import_at(0x50), vs.import_at(0x58));
        }
        // The local player's nitrous sound (`HeatLevelSoundSet.NitrousLocal`), merged into the engine bank: it
        // ducks the engine's own channels and plays on the bank's master level.
        if let Some(nitrous) = self.nitrous_bank() {
            eb.bank.merge(nitrous);
        }
        Ok(Some(eb))
    }

    /// Add a `Vehicles.Sound.TransmissionWhine` (its `Sequences`: whine loops on `GetTransmissionSpeedMph` /
    /// `GetReverseTransSpeedMph` with `Auto_Pitch`, channel gains on `GetPlayerGas`) and a
    /// `Vehicles.Sound.TurboWhine` (`Sequences`: the whine loop on `GetNormalisedRPM`, gain by the engine load,
    /// channel gain and pitch by the RPM; the blow-off and spool one-shots as cues) to the player's engine bank.
    fn add_whines(&self, eb: &mut EngineBank, transmission: Option<ResourceId>, turbo: Option<ResourceId>) {
        let mut loader = SoundLoader::new(self);
        let mut extra = Bank::default();
        if let Some(tw) = transmission.and_then(|id| self.genesys_object(id).ok()) {
            for id in handles(&tw, ENGINE_SEQUENCES) {
                if let Some(s) = loader.sequence(id) {
                    extra.sequences.push(s);
                }
            }
        }
        if let Some(tb) = turbo.and_then(|id| self.genesys_object(id).ok()) {
            for id in handles(&tb, ENGINE_SEQUENCES) {
                if let Some(s) = loader.sequence(id) {
                    extra.sequences.push(s);
                }
            }
            let mut cue_of = |hash: u32, name: &str| -> Option<f32> {
                let mut s = loader.sequence(handle_of(tb.get(hash))?)?;
                s.exclusive_cue = true;
                let length = s.items.iter().map(|it| it.end).fold(s.max, f32::max);
                extra.sequences.push(s);
                extra.cues.push((name.to_string(), vec![extra.sequences.len() - 1]));
                Some(length)
            };
            let blow_off: Vec<Option<f32>> = (0..4).map(|k| cue_of(TURBO_BLOW_OFF[k], cue::BLOW_OFF[k])).collect();
            let spool: Vec<Option<f32>> = (0..2).map(|k| cue_of(TURBO_SPOOL[k], cue::SPOOL[k])).collect();
            let r = |k: usize| num_hash(&tb, TURBO_RATES[k]).unwrap_or(1.0);
            eb.turbo = Some(TurboWhine {
                fall: (r(0), r(1)),
                rise: (r(2), r(3)),
                blow_off: [blow_off[0], blow_off[1], blow_off[2], blow_off[3]],
                spool: [spool[0], spool[1]],
            });
        }
        extra.channels = loader.channels();
        eb.bank.merge(extra);
    }

    /// The local player's nitrous timeline as a one-cue bank ([`cue::NITROUS`]): `HeatLevelSoundSet.NitrousLocal`
    /// of the heat-level sound bundle (`GAMELOGIC/HL_%d.BNDL`, `FUN_0065fee0`; the install has one, `HL_31208`,
    /// for every heat level). A 12 s timeline: the burn wave (1..8.885 s) and its tail (8.7..10.594 s) on
    /// channel `…7533`, the engine channels ducked while it plays, and jump / quit controllers on
    /// `Players.LocalPlayer.NitrousState.UsingNitrous` that loop the burn while nitrous is used and play the
    /// tail on release. The code that starts it is not in the dump; the driver starts it when nitrous starts
    /// (`EngineDriver::set_nitrous`).
    pub fn nitrous_bank(&self) -> Option<Bank> {
        let mut bundles: Vec<_> = self
            .game()
            .files_with_extension("BNDL")
            .into_iter()
            .filter(|p| p.to_string_lossy().to_ascii_uppercase().replace('\\', "/").starts_with("GAMELOGIC/HL_"))
            .collect();
        bundles.sort();
        for path in bundles {
            let Some(b) = self.bundle(&path) else { continue };
            for e in b.entries_of_type(mw_bundle::ResourceType::GenesysObject) {
                let Ok(res) = b.load(e) else { continue };
                let Ok(obj) = self.decode_genesys(&res) else { continue };
                if obj.type_name != "Genesys.Gen.HeatLevelSoundSet" {
                    continue;
                }
                let mut loader = SoundLoader::new(self);
                let mut s = handle(&obj, "NitrousLocal").and_then(|id| loader.sequence(id))?;
                s.exclusive_cue = true;
                return Some(Bank {
                    channels: loader.channels(),
                    sequences: vec![s],
                    cues: vec![(cue::NITROUS.to_string(), vec![0])],
                    master_gain: 1.0,
                });
            }
        }
        None
    }

    /// The engine bank of one `EngineSound2` object (the player's, from [`AssetStore::engine_bank`], or a
    /// competitor's, `mw_assets::car_sound`); `normalise` scales it to the common engine level
    /// ([`EngineBank::normalise`], the player's), otherwise it keeps the data's own mix level.
    pub fn engine_bank_from(&self, es: &Object, normalise: bool) -> EngineBank {
        let es = es.clone();
        let mut loader = SoundLoader::new(self);
        let mut bank = Bank { master_gain: num(&es, "MasterGain").unwrap_or(1.0), ..Default::default() };
        for id in handles(&es, ENGINE_SEQUENCES) {
            if let Some(s) = loader.sequence(id) {
                bank.sequences.push(s);
            }
        }
        // The audible RPM range (`FUN_00575340`, run when the engine sound starts: `RaceCarEngineSound` +0x1c0 /
        // +0x1c4): the lowest `StartTime` and the highest `EndTime` of the Ginsu items in `Sequences`, so the
        // car's whole rev range plays across its recordings (1790..7500 on the 911, 1750..6000 on the XKR). The
        // constructor's 1000..8000 is only the fallback for a bank without Ginsu items.
        let audible_range = bank
            .sequences
            .iter()
            .flat_map(|s| &s.items)
            .filter(|i| matches!(i.kind, ItemKind::Ginsu { .. }))
            .fold(None, |r: Option<(f32, f32)>, i| {
                Some(r.map_or((i.start, i.end), |(lo, hi)| (lo.min(i.start), hi.max(i.end))))
            })
            .filter(|(lo, hi)| hi > lo)
            .unwrap_or(crate::engine_driver::AUDIBLE_RPM_RANGE);
        // The engine's buses (so their chains are in the graph even before anything automates them).
        for id in handles(&es, ENGINE_CHANNELS) {
            loader.note_channel(Some(id));
        }
        if let Some(mut s) = handle(&es, "Gurgle_Sequence").and_then(|id| loader.sequence(id)) {
            s.retrigger = true;
            bank.sequences.push(s);
        }
        let add_cue = |bank: &mut Bank, name: &str, s: Option<Sequence>| {
            if let Some(s) = s {
                bank.sequences.push(s);
                bank.cues.push((name.to_string(), vec![bank.sequences.len() - 1]));
            }
        };
        // The pop rule (`FUN_009b20b0`) starts `Pop_Sequence` on a pop and `Backfire_Sequence` on a backfire
        // (`FUN_0058ccb0`), each only once its previous instance has ended. The pop sequence is bound to the
        // pop probability: a start plays its items whose range holds the probability then.
        let s = handle(&es, "Pop_Sequence").and_then(|id| loader.sequence(id)).map(|mut s| {
            s.exclusive_cue = true;
            s
        });
        add_cue(&mut bank, cue::POP, s);
        let s = handle(&es, "Backfire_Sequence").and_then(|id| loader.sequence(id)).map(|mut s| {
            s.exclusive_cue = true;
            s
        });
        add_cue(&mut bank, cue::BACKFIRE, s);
        let s = handle(&es, "Ignition_Sequence").and_then(|id| loader.sequence(id));
        add_cue(&mut bank, cue::IGNITION, s);
        let s = handle_of(es.get(REV_LIMITER_SEQUENCE)).and_then(|id| loader.sequence(id));
        add_cue(&mut bank, cue::LIMITER, s);
        // Every variant of the four gear-change lists, each behind its own gate; the driver opens the one the game
        // would start (`FUN_0058ca50`: the next one of the list, round robin).
        let mut gear_changes = [0usize; 4];
        for (l, list) in GEAR_CHANGE_LISTS.into_iter().enumerate() {
            for id in handles(&es, list) {
                if let Some(mut s) = loader.sequence(id) {
                    s.gate = gate::gear_change(l, gear_changes[l]);
                    gear_changes[l] += 1;
                    bank.sequences.push(s);
                }
            }
        }
        bank.channels = loader.channels();
        // Per-car channel trims (`Gains`: GainParam { Mixer_channel, Gain }).
        for w in structs(&es, field_hash("Gains")) {
            let Some(g) = sub(w, field_hash("Gain")) else { continue };
            let (Some(ch), Some(v)) = (handle(g, "Mixer_channel"), num(g, "Gain")) else { continue };
            if let Some(c) = bank.channels.iter_mut().find(|c| c.id == ch.0) {
                c.gain *= v;
            }
        }
        // Per-car plug-in overrides (`DspParam { Mixer_channel, PlugIn, Value, Parameter_name }` in four
        // lists: `Engine_params` and three unnamed ones). The index ↔ runtime parameter table is not in
        // the decompiled code; the readings applied here are the ones consistent across the data
        // (research log 2026-10-04, "engine sound from the game code"): 1 = the first parameter, Frequency,
        // of a filter (the 911's 6.5 kHz EQs become 400 Hz / 1 kHz), 0 on a send = its gain (0 mutes the
        // comb-rasp delay path). Other indices (Delay 4, BandPass 5) are left alone.
        for list in [field_hash("Engine_params"), 0xe792_2a45, 0x6b1a_9df7, 0x7b5b_ec24] {
            for w in structs(&es, list) {
                let Some(p) = sub(w, field_hash("Param")) else { continue };
                let (Some(ch), Some(uid), Some(v)) = (handle(p, "Mixer_channel"), num(p, "PlugIn"), num(p, "Value"))
                else {
                    continue;
                };
                let index = num(p, "Parameter_name").unwrap_or(-1.0) as i32;
                let Some(c) = bank.channels.iter_mut().find(|c| c.id == ch.0) else { continue };
                let Some(plugin) = c.plugins.iter_mut().find(|p| p.uid == uid as u32) else { continue };
                let target = match (plugin.kind, index) {
                    (PluginKind::Send { .. }, 0) => Some(field::GAIN),
                    (
                        PluginKind::Peaking
                        | PluginKind::HighShelf
                        | PluginKind::LowPass
                        | PluginKind::HighPass
                        | PluginKind::BandPass,
                        1,
                    ) => Some(field::FREQUENCY),
                    _ => None,
                };
                if let Some(f) = target {
                    match plugin.fields.iter_mut().find(|(h, _)| *h == f) {
                        Some(slot) => slot.1 = v,
                        None => plugin.fields.push((f, v)),
                    }
                }
            }
        }
        let rpm_seq = bank.sequences.iter().find(|s| s.items.iter().any(|i| matches!(i.kind, ItemKind::Ginsu { .. })));
        let rpm_range = rpm_seq.map_or((1000.0, 7000.0), |s| (s.min, s.max));
        let main = bank.sequences.iter().flat_map(|s| &s.items).find_map(|i| match &i.kind {
            ItemKind::Ginsu { ginsu } => Some(ginsu.clone()),
            _ => None,
        });
        let harmonic_order = main.map_or(3.0, |g| dominant_order(&g));
        let d = EngineTuning::default();
        let f = |hash: u32, default: f32| number(es.get(hash)).unwrap_or(default);
        let tuning = EngineTuning {
            rpm_curve_on: f(0xa33d_84fe, d.rpm_curve_on),
            rpm_curve_off: f(0xcb44_d82d, d.rpm_curve_off),
            rpm_curve_alt: f(0x913b_b900, d.rpm_curve_alt),
            load_rate: f(0x99d4_b91e, d.load_rate),
            cylinders: f(0xed75_4b99, d.cylinders).max(1.0),
            harmonic1: f(0x7cbc_11ca, d.harmonic1),
            harmonic2: f(0x7cbc_11c9, d.harmonic2),
            subharmonic: f(0x283e_07de, d.subharmonic),
            limiter_rate: (f(0x6e6b_e213, d.limiter_rate.0), f(0x9529_a29e, d.limiter_rate.1)),
        };
        let ignition = bank.cues.iter().find(|(n, _)| n == cue::IGNITION).and_then(|(_, v)| v.first()).map(|&i| {
            let s = &bank.sequences[i];
            let length = s.items.iter().map(|it| it.end).fold(s.max, f32::max);
            // Property 7 posts a non-zero value from the item's start (`FUN_008f5490`: |v| > FLT_EPSILON sets
            // the flag).
            let catch = s
                .items
                .iter()
                .filter(|it| match &it.kind {
                    ItemKind::Effect { values } => {
                        values.iter().any(|(p, m)| *p == 7 && m.eval(0.0, None).abs() > 1e-6)
                    }
                    _ => false,
                })
                .map(|it| it.start)
                .fold(length, f32::min);
            (catch, length)
        });
        let mut eb = EngineBank {
            bank,
            rpm_range,
            audible_range,
            harmonic_order,
            tuning,
            gear_changes,
            ignition,
            turbo: None,
            normalisation: 1.0,
        };
        if normalise {
            eb.normalise();
        }
        eb
    }
}

/// Engine order (multiple of the crank frequency, in half orders) with the most energy across the grains
/// of a recording, by Goertzel filters at each grain's recorded RPM.
pub fn dominant_order(g: &mw_audio::ginsu::Ginsu) -> f32 {
    let sr = g.pcm.sample_rate as f32;
    let n = g.grains.len().saturating_sub(1);
    let mut energy = [0f64; 16];
    for gi in (0..n).step_by((n / 40).max(1)) {
        let (a, b) = g.grain(gi);
        if b <= a + 64 {
            continue;
        }
        let rpm = g.rpm_at_position((a + b) as f32 * 0.5);
        // Two cycles around the grain for resolution.
        let (lo, hi) = (a, (b + (b - a)).min(g.pcm.samples.len()));
        for (k, e) in energy.iter_mut().enumerate() {
            let order = 0.5 * (k + 1) as f32;
            let f = rpm / 60.0 * order;
            if f > sr * 0.45 || f < 20.0 {
                continue;
            }
            let w = std::f32::consts::TAU * f / sr;
            let c = 2.0 * w.cos();
            let (mut s1, mut s2) = (0f32, 0f32);
            for &x in &g.pcm.samples[lo..hi] {
                let s0 = x as f32 + c * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            *e += (s1 * s1 + s2 * s2 - c * s1 * s2) as f64 / (hi - lo) as f64;
        }
    }
    let k = (0..energy.len()).max_by(|&a, &b| energy[a].total_cmp(&energy[b])).unwrap_or(5);
    0.5 * (k + 1) as f32
}
