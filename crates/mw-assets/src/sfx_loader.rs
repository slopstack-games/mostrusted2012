//! Copy of the GeneSys → sequence-bank loader in `sound.rs`, building `mw_audio::sfx_player` banks for the
//! vehicle sounds (`sfx.rs`). Kept separate while the engine path (`sound.rs`, `mw_audio::sequence`) is being
//! reworked; fold back together once that settles.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mw_audio::sfx_player::{
    Channel, Clip, Fade, Item, ItemKind, Jump, ModValue, Plugin, PluginKind, Sequence, Target, field,
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
/// `JumpTimelineController`'s destination time (unnamed; `TriggerTime` is named).
const JUMP_DESTINATION: u32 = 0xb6a5_1260;

pub(crate) fn number(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f32),
        Value::UInt(u) => Some(*u as f32),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
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

fn handle_of(v: Option<&Value>) -> Option<ResourceId> {
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

/// Builds banks, caching decoded waves across calls.
pub struct SoundLoader<'a> {
    store: &'a AssetStore,
    waves: Mutex<HashMap<ResourceId, Option<Arc<Clip>>>>,
    /// Channel ids reached so far, in discovery order.
    channels: Vec<ResourceId>,
    /// Mix snapshot channel gains by the `field_hash` of the channel name: an item without a `Mixer_Channel`
    /// plays on the snapshot channel its `Snapshot_Property` names (`"Competitor Vehicle Collision Small"` →
    /// `Competitor_Vehicle_Collision_Small`), whose gain scales the item.
    snapshot: HashMap<u32, f32>,
}

impl<'a> SoundLoader<'a> {
    pub fn new(store: &'a AssetStore) -> Self {
        Self { store, waves: Default::default(), channels: Vec::new(), snapshot: HashMap::new() }
    }

    /// Items naming a snapshot channel (`Snapshot_Property`, no `Mixer_Channel`) take its gain from `gains`.
    pub fn with_snapshot(mut self, gains: HashMap<u32, f32>) -> Self {
        self.snapshot = gains;
        self
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

    fn note_channel(&mut self, id: Option<ResourceId>) {
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
                    }
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
                _ => continue,
            };
            self.note_channel(channel);
            let mut kind = kind;
            let snapshot = text(it, field_hash("Snapshot_Property"));
            if channel.is_none()
                && !snapshot.is_empty()
                && let Some(&g) = self.snapshot.get(&field_hash(&snapshot.replace(' ', "_")))
                && let ItemKind::Wave { gain, .. } = &mut kind
            {
                gain.value *= g;
                gain.out = (gain.out.0 * g, gain.out.1 * g);
            }
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
            jumps: structs(&o, field_hash("TimelineControllers"))
                .filter(|c| c.type_name.ends_with("JumpTimelineController"))
                .filter_map(|c| {
                    Some(Jump {
                        trigger_time: num(c, "TriggerTime")?,
                        destination: number(c.get(JUMP_DESTINATION)).or_else(|| num(c, "DestinationTime"))?,
                        enabled: text(c, field_hash("EnabledBinding")),
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
