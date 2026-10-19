//! A car's engine voice as the game's audio data describes it (`Genesys.Gen.EngineSound2`).
//!
//! `EngineSound2.Sequences` lists `Sequence` objects. The engine ones hold `GinsuSequenceItem`s: each plays
//! one Ginsu recording into its own `MixerChannel` over an RPM range, and all items of a sequence play at
//! the same time. The 911 has two such sequences of three recordings (two microphone sets: a full-load
//! rev-up, a part-load rev-up and an off-load fall). Another sequence, bound to a 0..1 parameter (engine
//! load), holds `MixerChannelSequenceItem`s whose gain curves (`AnimPoints`) crossfade those channels: off
//! load at low load, part load in the middle, full load at the top. Each channel also has a static `Gain`.
//! Research log 2026-10-04 (engine sound data).
//!
//! Not decoded: what the bindings' raw values select (the load parameter is assumed to be throttle-based
//! engine load), the curve points' tangent fields (curves are read as piecewise linear), and the effect
//! plug-ins on the bus channels (EQ, distortion, sub-harmoniser, filter curves).

use mw_bundle::{Bundle, ResourceId, ResourceType};
use mw_formats::genesys::{Object, Value, field_hash};

use crate::{AssetStore, Result};

/// One Ginsu recording in the engine mix.
#[derive(Debug, Clone)]
pub struct EngineLayer {
    pub ginsu: ResourceId,
    pub mixer_channel: Option<ResourceId>,
    /// The channel's static gain (linear).
    pub gain: f32,
    /// Gain against engine load (0..1), as `(load, gain)` points sorted by load; empty = always 1.
    pub load_curve: Vec<(f32, f32)>,
    /// RPM range of the sequence item.
    pub rpm_range: (f32, f32),
}

#[derive(Debug, Clone, Default)]
pub struct EngineSoundSpec {
    pub layers: Vec<EngineLayer>,
    pub master_gain: f32,
}

/// Curve point fields: x (load) and y (gain); the other four fields are tangents (not decoded).
const POINT_X: u32 = 0x12d3a8aa;
const POINT_Y: u32 = 0x12d3a8ab;
/// Sequence item RPM range.
const ITEM_MAX: u32 = 0xddb9c614;
const ITEM_MIN: u32 = 0x6c82122b;

fn number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Float(f) => Some(*f as f64),
        Value::Int(i) => Some(*i as f64),
        Value::UInt(u) => Some(*u as f64),
        _ => None,
    }
}

fn handle(o: &Object, name: &str) -> Option<ResourceId> {
    match o.get(field_hash(name))? {
        Value::Handle(h) => *h,
        _ => None,
    }
}

fn items<'a>(o: &'a Object, name: &str) -> impl Iterator<Item = &'a Value> {
    match o.get(field_hash(name)) {
        Some(Value::Array(a)) => a.iter(),
        _ => [].iter(),
    }
}

fn structs<'a>(o: &'a Object, name: &str) -> impl Iterator<Item = &'a Object> {
    items(o, name).filter_map(|v| match v {
        Value::Struct(s) => Some(s),
        _ => None,
    })
}

/// A `MixerChannelSequenceItem`'s gain curve: the double parameter with plug-in 0 and parameter name 0.
fn gain_curve(item: &Object) -> Option<Vec<(f32, f32)>> {
    for p in structs(item, "Double_Params") {
        let plugin = number(p.get(field_hash("PlugIn"))).unwrap_or(-1.0);
        let name = number(p.get(field_hash("Parameter_Name"))).unwrap_or(-1.0);
        if plugin != 0.0 || name != 0.0 {
            continue;
        }
        let Some(Value::Struct(value)) = p.get(field_hash("Value")) else { continue };
        let mut pts: Vec<(f32, f32)> = structs(value, "AnimPoints")
            .filter_map(|pt| Some((number(pt.get(POINT_X))? as f32, number(pt.get(POINT_Y))? as f32)))
            .collect();
        if pts.is_empty() {
            continue;
        }
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        return Some(pts);
    }
    None
}

impl EngineSoundSpec {
    /// Gain of a curve at a load (piecewise linear, clamped at the ends).
    pub fn curve_at(curve: &[(f32, f32)], x: f32) -> f32 {
        let Some(first) = curve.first() else { return 1.0 };
        if x <= first.0 {
            return first.1.max(0.0);
        }
        for w in curve.windows(2) {
            let ((a, ya), (b, yb)) = (w[0], w[1]);
            if x <= b {
                let f = if b > a { (x - a) / (b - a) } else { 0.0 };
                return (ya + (yb - ya) * f).max(0.0);
            }
        }
        curve.last().map_or(1.0, |l| l.1.max(0.0))
    }
}

/// A car's engine voice: the full sequence bank when the car has `EngineSound2` data, else Ginsu layers.
#[derive(Debug, Clone)]
// One voice per car, built once and moved a few times: the size difference does not matter, and boxing would touch
// every match on it.
#[allow(clippy::large_enum_variant)]
pub enum EngineVoice {
    Bank(crate::sound::EngineBank),
    Layers(Vec<mw_audio::ginsu::EngineLayer>, f32),
}

impl AssetStore {
    /// A car's engine voice ([`AssetStore::engine_bank`], falling back to [`AssetStore::engine_voice`]).
    pub fn engine(&self, bundle: &Bundle) -> Result<Option<EngineVoice>> {
        if let Some(b) = self.engine_bank(bundle)?
            && b.bank.sequences.iter().any(|s| !s.items.is_empty())
        {
            return Ok(Some(EngineVoice::Bank(b)));
        }
        Ok(self.engine_voice(bundle)?.map(|(l, g)| EngineVoice::Layers(l, g)))
    }

    /// Everything needed to play a car's engine: the layers of its `EngineSound2` with their Ginsu
    /// recordings loaded, and the master gain; cars without engine-sound data fall back to an on-load /
    /// off-load pair picked from the bundle's Ginsu resources.
    pub fn engine_voice(&self, bundle: &Bundle) -> Result<Option<(Vec<mw_audio::ginsu::EngineLayer>, f32)>> {
        use std::sync::Arc;
        let parse = |id: ResourceId| -> Option<Arc<mw_audio::ginsu::Ginsu>> {
            Some(Arc::new(mw_audio::ginsu::Ginsu::parse(self.load(id).ok()?.header()).ok()?))
        };
        if let Some(spec) = self.engine_sound(bundle)? {
            let layers: Vec<_> = spec
                .layers
                .iter()
                .filter_map(|l| {
                    Some(mw_audio::ginsu::EngineLayer {
                        ginsu: parse(l.ginsu)?,
                        gain: l.gain,
                        load_curve: l.load_curve.clone(),
                    })
                })
                .collect();
            if !layers.is_empty() {
                return Ok(Some((layers, spec.master_gain)));
            }
        }
        let ginsus = bundle
            .entries_of_type(ResourceType::Ginsu)
            .filter_map(|e| mw_audio::ginsu::Ginsu::parse(bundle.load(e).ok()?.header()).ok())
            .collect();
        Ok(mw_audio::ginsu::pick_engine(ginsus)
            .map(|(on, off)| (mw_audio::ginsu::EngineMixer::on_off_layers(on, off), 1.0)))
    }

    /// The engine voice of a vehicle bundle: the `EngineSound2` with the most Ginsu layers.
    pub fn engine_sound(&self, bundle: &Bundle) -> Result<Option<EngineSoundSpec>> {
        let mut best: Option<EngineSoundSpec> = None;
        for e in bundle.entries_of_type(ResourceType::GenesysObject) {
            let Ok(obj) = self.decode_genesys(&bundle.load(e)?) else { continue };
            if obj.type_name != "Genesys.Gen.EngineSound2" {
                continue;
            }
            let spec = self.engine_sound_from(&obj);
            if spec.layers.len() > best.as_ref().map_or(0, |b| b.layers.len()) {
                best = Some(spec);
            }
        }
        Ok(best)
    }

    fn engine_sound_from(&self, es: &Object) -> EngineSoundSpec {
        let sequences: Vec<Object> = items(es, "Sequences")
            .filter_map(|v| match v {
                Value::Handle(Some(id)) => self.decode_genesys(&self.load(*id).ok()?).ok(),
                _ => None,
            })
            .collect();
        // Gain curves per mixer channel, from every sequence's MixerChannelSequenceItems.
        let mut curves: Vec<(ResourceId, Vec<(f32, f32)>)> = Vec::new();
        for seq in &sequences {
            for item in structs(seq, "SequenceItems").filter(|i| i.type_name.ends_with("MixerChannelSequenceItem")) {
                if let (Some(ch), Some(c)) = (handle(item, "Mixer_Channel"), gain_curve(item)) {
                    curves.push((ch, c));
                }
            }
        }
        let mut layers = Vec::new();
        for seq in &sequences {
            for item in structs(seq, "SequenceItems").filter(|i| i.type_name.ends_with("GinsuSequenceItem")) {
                let Some(ginsu) = handle(item, "Ginsu") else { continue };
                let channel = handle(item, "Mixer_Channel");
                let gain = channel
                    .and_then(|ch| self.decode_genesys(&self.load(ch).ok()?).ok())
                    .and_then(|c| number(c.get(field_hash("Gain"))))
                    .unwrap_or(1.0) as f32;
                let load_curve = channel
                    .and_then(|ch| curves.iter().find(|(c, _)| *c == ch))
                    .map(|(_, c)| c.clone())
                    .unwrap_or_default();
                let rpm_range = (
                    number(item.get(ITEM_MIN)).unwrap_or(0.0) as f32,
                    number(item.get(ITEM_MAX)).unwrap_or(f64::from(u16::MAX)) as f32,
                );
                layers.push(EngineLayer { ginsu, mixer_channel: channel, gain, load_curve, rpm_range });
            }
        }
        let master_gain = number(es.get(field_hash("MasterGain"))).unwrap_or(1.0) as f32;
        EngineSoundSpec { layers, master_gain }
    }
}
