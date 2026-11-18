//! Police radio: the speech events of `EN_US/SPEECH/SPEECHEVENTSEQUENCES.BNDL` and their waves in
//! `EN_US/SWC/SPEECHEVENTSEQUENCES.SWC0` (docs/formats.md "Speech"; research log 2026-10-16).
//!
//! - `Speech.SpeechEventSequence` (110, 76 bytes; the event's UniqueId is what the game requests): `SequenceItems`
//!   (+0x20), conditions `a137b739` (+0x2c, binding strings, all must hold when `47906457` (+0x44) is set, else
//!   any), `Delay` (+0x30, s before the first line), `4246e1c3` / `ef972179` (+0x34 / +0x38, latest / earliest
//!   mode time, minutes), `Period` (+0x3c, minutes between plays), `Cutscene_Behaviour` (+0x40), one-off
//!   `c78f4f4a` (+0x46), needs the player's car `a40bb99b` (+0x47), `Priority` (+0x48, 0 highest), max plays
//!   `27e5d881` (+0x49, 0 = no limit).
//! - Items: `Speech.ScriptLineSequenceItem` (`StartTime`, lines `cea5cd0e` and containers `39050096` of lines;
//!   `Probability` in percent, `9f199cf7` a vocabulary slot) and `SpeechEventSequenceItem` (`8239ac66`: sub-events, one is played).
//! - `Speech.ScriptLine`: `Character` (the voice's `Character.AudioSettings`: its `Mixer_Channel`), `f8b496b9` →
//!   `LocalisedWave` = the low 32 bits of a `WaveContainerTable` id.
//! - `WaveContainerTable` (type 0x82): `"wct "`, u32 version 2, u32 header size 0x18, u32 count, u32 name
//!   offset, u32 0, then 16-byte entries `{u64 id, u32 offset in the SWC0, u32 size}`; each entry is a plain SPS
//!   stream (EALayer3, mono, 48 kHz) that `mw_audio::sps::decode` reads. `WaveDictionary` (type 0x84): each
//!   wave's length.
//! - The types and the manager that plays them are `mw_audio::speech`. The radio filtering is in the
//!   recordings (99.5–100 % of the energy in 300–3400 Hz).

use std::collections::HashMap;

use mw_bundle::{ResourceId, ResourceType};
use mw_formats::genesys::{Object, Value, field_hash};

use crate::AssetStore;

pub const BUNDLE: &str = "EN_US/SPEECH/SPEECHEVENTSEQUENCES.BNDL";
pub const CONTAINER: &str = "EN_US/SWC/SPEECHEVENTSEQUENCES.SWC0";

pub use mw_audio::speech::{Line, SpeechEvent, SpeechItem};

/// The speech events and the wave container's index.
#[derive(Debug, Default)]
pub struct SpeechData {
    pub events: HashMap<u32, SpeechEvent>,
    /// Wave id (low 32 bits) → offset and size in the SWC0.
    pub waves: HashMap<u32, (u64, u32)>,
    /// Wave id → length in seconds (the `WaveDictionary`).
    pub durations: HashMap<u32, f32>,
    /// The install and the SWC0's game-relative path, when it exists.
    pub container: Option<(mw_gamedir::GameDir, String)>,
}

fn num(o: &Object, hash: u32) -> f32 {
    match o.get(hash) {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f32,
        Some(Value::UInt(u)) => *u as f32,
        Some(Value::Bool(b)) => f32::from(u8::from(*b)),
        _ => 0.0,
    }
}

fn structs(o: &Object, hash: u32) -> Vec<&Object> {
    match o.get(hash) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| match v {
                Value::Struct(s) => Some(s),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

impl SpeechData {
    /// One line's PCM (mono, 48 kHz).
    pub fn line_pcm(&self, wave: u32) -> Option<mw_audio::Pcm> {
        let (offset, size) = *self.waves.get(&wave)?;
        let (game, path) = self.container.as_ref()?;
        let buf = game.read_at(path, offset, size as usize).ok()?;
        mw_audio::sps::decode(&buf).ok()
    }
}

impl AssetStore {
    /// The speech events and the SWC0 index ([`SpeechData`]).
    pub fn speech_data(&self) -> crate::Result<SpeechData> {
        let b = self.bundle(BUNDLE).ok_or(crate::Error::Missing(ResourceId::NULL))?;
        let mut out = SpeechData {
            container: self.game().resolve(CONTAINER).map(|p| (self.game().clone(), p)),
            ..Default::default()
        };
        // The wave container table.
        for e in b.entries_of_type(ResourceType::WaveContainerTable) {
            let res = b.load(e)?;
            let d = res.header();
            let u32_at = |o: usize| d.get(o..o + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]));
            let (Some(header), Some(count)) = (u32_at(8), u32_at(12)) else { continue };
            for i in 0..count as usize {
                let o = header as usize + 16 * i;
                let (Some(id), Some(offset), Some(size)) = (u32_at(o), u32_at(o + 8), u32_at(o + 12)) else { break };
                out.waves.insert(id, (offset as u64, size));
            }
        }
        // The wave dictionary: u32 0, u32 2, u32 tail offset, u32 entries offset, u32 count, `{f32 ms, u32 id}`.
        for e in b.entries_of_type(ResourceType::WaveDictionary) {
            let res = b.load(e)?;
            let d = res.header();
            let u32_at = |o: usize| d.get(o..o + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]));
            let (Some(at), Some(count)) = (u32_at(12), u32_at(16)) else { continue };
            for i in 0..count as usize {
                let o = at as usize + 8 * i;
                let (Some(ms), Some(id)) = (u32_at(o), u32_at(o + 4)) else { break };
                out.durations.insert(id, f32::from_bits(ms) / 1000.0);
            }
        }
        // Voices: AudioSettings id → its mixer channel's gain.
        let mut objects = Vec::new();
        for e in b.entries_of_type(ResourceType::GenesysObject) {
            if let Ok(o) = self.decode_genesys(&b.load(e)?) {
                objects.push((e.id, o));
            }
        }
        let channel_gain: HashMap<ResourceId, f32> = objects
            .iter()
            .filter(|(_, o)| o.type_name == "Genesys.Gen.Character.AudioSettings")
            .filter_map(|(id, o)| {
                let ch = match o.get(field_hash("Mixer_Channel")) {
                    Some(Value::Handle(Some(h))) | Some(Value::Ref(h)) => *h,
                    _ => return None,
                };
                let ch = self.genesys_object(ch).ok()?;
                Some((
                    *id,
                    match ch.get(field_hash("Gain")) {
                        Some(Value::Float(g)) => *g,
                        _ => 1.0,
                    },
                ))
            })
            .collect();
        let durations = out.durations.clone();
        let line = |l: &Object| -> Option<Line> {
            let w =
                structs(l, 0xf8b4_96b9).first().map(|w| num(w, 0xf437_d598)).or_else(|| match l.get(0xf8b4_96b9) {
                    Some(Value::Struct(s)) => Some(num(s, 0xf437_d598)),
                    _ => None,
                })? as u32;
            let gain = match l.get(field_hash("Character")) {
                Some(Value::Handle(Some(h))) | Some(Value::Ref(h)) => channel_gain.get(h).copied().unwrap_or(1.0),
                _ => 1.0,
            };
            // MODEL: a line missing from the dictionary lasts 3 s.
            let duration = durations.get(&w).copied().unwrap_or(3.0);
            (w != 0).then_some(Line { wave: w, gain, duration })
        };
        for (_, o) in &objects {
            if o.type_name != "Genesys.Gen.Speech.SpeechEventSequence" {
                continue;
            }
            let mut items = Vec::new();
            for it in structs(o, field_hash("SequenceItems")) {
                let start = num(it, field_hash("StartTime"));
                let end = num(it, field_hash("EndTime"));
                if it.type_name.ends_with("ScriptLineSequenceItem") {
                    let lines: Vec<Line> = structs(it, 0xcea5_cd0e).into_iter().filter_map(line).collect();
                    let containers = structs(it, 0x3905_0096)
                        .into_iter()
                        .map(|c| structs(c, 0xcea5_cd0e).into_iter().filter_map(line).collect())
                        .collect();
                    let p = num(it, field_hash("Probability"));
                    items.push(SpeechItem::Lines {
                        start,
                        end,
                        probability: if p > 0.0 { p / 100.0 } else { 1.0 },
                        lines,
                        containers,
                        vocabulary: num(it, 0x9f19_9cf7) as u16,
                    });
                } else if it.type_name.ends_with("SpeechEventSequenceItem") {
                    let events = match it.get(0x8239_ac66) {
                        Some(Value::Array(a)) => a
                            .iter()
                            .filter_map(|v| match v {
                                Value::Int(i) => Some(*i as u32),
                                Value::UInt(u) => Some(*u as u32),
                                _ => None,
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                    items.push(SpeechItem::Events { start, end, events });
                }
            }
            let conditions = match o.get(0xa137_b739) {
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(|v| match v {
                        Value::Str(s) if !s.is_empty() => Some(s.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            let id = num(o, 0x655d_3f5f) as u32;
            out.events.insert(
                id,
                SpeechEvent {
                    id,
                    items,
                    conditions,
                    all_conditions: num(o, 0x4790_6457) > 0.5,
                    delay: num(o, field_hash("Delay")),
                    period: num(o, field_hash("Period")),
                    latest: num(o, 0x4246_e1c3),
                    earliest: num(o, 0xef97_2179),
                    cutscene_behaviour: num(o, field_hash("Cutscene_Behaviour")) as u8,
                    priority: num(o, field_hash("Priority")) as u8,
                    max_plays: num(o, 0x27e5_d881) as u8,
                    one_off: num(o, 0xc78f_4f4a) > 0.5,
                    needs_car: num(o, 0xa40b_b99b) > 0.5,
                },
            );
        }
        Ok(out)
    }
}
