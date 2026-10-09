//! Engine sounds of the cars the player does not drive (docs/decomp.md "Other cars' engine sounds"):
//!
//! - **AI racers** play their car's `<id>_COMPETITORSOUND` (a `VehicleSound` resource, type 0x207, in the car's
//!   `_HI` / `_LO` bundles; the player's car uses `<id>_VEHICLESOUND`): an `EngineSound2` with fewer sequences
//!   than the player's (one Ginsu set, the speed and load mixers, a listener-distance mixer, one gear-change
//!   variant per direction) driven by the same engine code ([`crate::engine_driver`]).
//! - **Traffic** plays `<component>_COMPETITORSOUND` from `VEHICLES/TRAFFICATTRIBS.BNDL` (`FUN_006077c0` case 5
//!   loads one per vehicle type of the lane data): an `EngineSound2` with two sequences, wave loops bound to
//!   `Traffic.EngineSounds[#].RPM` and road / tyre noise bound to `NormalisedSpeed`, whose
//!   `TrafficEffectSequenceItem` writes the RPM from the normalised speed (a curve with dips where the gears
//!   change). [`TrafficEngineDriver`] supplies the bindings (`FUN_005ee480` registers them).
//! - **Placement in the world**: each `MixerChannel` has a `DistanceFalloff` (`SoundDistanceFalloff`), a
//!   listener focus and a doppler mode; [`Spatial`] ports the game's 3D gain (`FUN_00ab9690`) and doppler
//!   (`FUN_00ab7260`).
//!
//! `VehicleSound` resource layout (all 11 traffic ones and the cars' two; imports at the pointer slots):
//!
//! | offset | contents |
//! |---|---|
//! | 0x10 | `TyreSound` (the silent placeholder set) |
//! | 0x18 / 0x1c | offset / count of a handle list (the player's only: 3 sequences) |
//! | 0x48 | `EngineSound2` |
//! | 0x50 | `Vehicles.Sound.TransmissionWhine` (the player's only) |
//! | 0x60 / 0x64 | offset / count of the horn sequences (`FUN_006042c0` plays `horns[car % count]`) |

use std::sync::Arc;

use mw_audio::sequence::{Bank, ItemKind, ModValue, Params};
use mw_bundle::ResourceId;
use mw_formats::genesys::{Object, Value, field_hash};

use crate::sound::number;
use crate::sound::{EngineBank, SoundLoader, handle_of};
use crate::{AssetStore, Result};

/// `EngineSound2.Sequences` and the channel list the game instantiates per engine (`FUN_00600bb0`).
const ENGINE_SEQUENCES: u32 = 0x1f64_dde4;
const ENGINE_CHANNELS: u32 = 0x1d82_ab85;

/// What a `VehicleSound` resource points to.
#[derive(Debug, Clone, Default)]
pub struct VehicleSoundRes {
    pub engine: Option<ResourceId>,
    pub horns: Vec<ResourceId>,
}

/// `SoundDistanceFalloff` (fields by their offsets in the type; the reader is `FUN_00ab9690`). Two curves of the
/// listener distance `d`, each `t^p` of a clamped linear ramp:
/// - the direct gain: inside `MinDistance` (+0x40) from `InitialGain` (+0x28) at 0 m to `PeakGain` (+0x50),
///   outside it from `PeakGain` down to 0 at +0x34 (`f5bece87`), with the exponent `CurveType` (+0x10);
/// - a second value from +0x54 (`1ede038c`) at +0x48 (`7d704038`) to +0x58 (`04f8e36b`) at +0x3c (`fc29b05f`),
///   exponent +0x14 (`d13efb2a`), times the direct gain (the output's slot 6; its consumer was not found).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistanceFalloff {
    pub curve: f32,
    pub curve2: f32,
    pub initial_gain: f32,
    pub max_distance: f32,
    pub second_far_distance: f32,
    pub min_distance: f32,
    pub second_near_distance: f32,
    pub peak_gain: f32,
    pub second_near: f32,
    pub second_far: f32,
}

impl Default for DistanceFalloff {
    /// No falloff (a channel without `DistanceFalloff`).
    fn default() -> Self {
        Self {
            curve: 1.0,
            curve2: 1.0,
            initial_gain: 1.0,
            max_distance: f32::MAX,
            second_far_distance: f32::MAX,
            min_distance: f32::MAX,
            second_near_distance: 0.0,
            peak_gain: 1.0,
            second_near: 1.0,
            second_far: 1.0,
        }
    }
}

fn ramp(d: f32, from: f32, to: f32, exponent: f32) -> f32 {
    let t = if to > from { (1.0 - (d - from) / (to - from)).clamp(0.0, 1.0) } else { 0.0 };
    // `FUN_00ed9e1b` is the CRT's SSE2 `pow` (base in xmm0, exponent in xmm1, from the disassembly).
    t.powf(exponent)
}

impl DistanceFalloff {
    pub fn from_object(o: &Object) -> Self {
        let f = |name: &str, h: u32| number(o.get(field_hash(name))).or_else(|| number(o.get(h)));
        let d = Self::default();
        Self {
            curve: f("CurveType", 0x213a_4107).unwrap_or(d.curve),
            curve2: f("", 0xd13e_fb2a).unwrap_or(d.curve2),
            initial_gain: f("InitialGain", 0x7f2d_01d0).unwrap_or(d.initial_gain),
            max_distance: f("", 0xf5be_ce87).unwrap_or(d.max_distance),
            second_far_distance: f("", 0xfc29_b05f).unwrap_or(d.second_far_distance),
            min_distance: f("MinDistance", 0x1dc2_e7d2).unwrap_or(d.min_distance),
            second_near_distance: f("", 0x7d70_4038).unwrap_or(d.second_near_distance),
            peak_gain: f("PeakGain", 0x260b_cfe7).unwrap_or(d.peak_gain),
            second_near: f("", 0x1ede_038c).unwrap_or(d.second_near),
            second_far: f("", 0x04f8_e36b).unwrap_or(d.second_far),
        }
    }

    /// The direct gain at `d` metres (before the direction terms and the final clamp to 1).
    pub fn gain(&self, d: f32) -> f32 {
        if d < self.min_distance {
            let t = ramp(d, 0.0, self.min_distance, self.curve);
            self.peak_gain + (self.initial_gain - self.peak_gain) * t
        } else {
            self.peak_gain * ramp(d, self.min_distance, self.max_distance, self.curve)
        }
    }

    /// The second curve (slot 6 of the output, before the direction terms and the direct gain).
    pub fn second(&self, d: f32) -> f32 {
        let t = ramp(d, self.second_near_distance, self.second_far_distance, self.curve2);
        self.second_far + (self.second_near - self.second_far) * t
    }
}

/// How a `MixerChannel` places its voices in the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spatial {
    pub falloff: DistanceFalloff,
    /// `d6365a79` (+0x28): the emitter's directivity; 0.5 = none, above it the emitter is quieter behind.
    pub emitter_focus: f32,
    /// `Focus` (+0x2c): the listener's; above 0.5 sounds behind the listener are quieter.
    pub listener_focus: f32,
    /// Speed of sound for the doppler shift (`435ec694`, +0x54: 1 → 680, 2 → 340, 3 → 170 m/s, other: none;
    /// `.rdata` 0x01034858 / 0x01034504 / 0x0103485c).
    pub doppler_speed: Option<f32>,
}

impl Default for Spatial {
    fn default() -> Self {
        Self { falloff: DistanceFalloff::default(), emitter_focus: 0.5, listener_focus: 0.5, doppler_speed: None }
    }
}

/// The doppler factor's upper clamp (`.rdata` 0x0103451c).
pub const DOPPLER_MAX: f32 = 8.0;

impl Spatial {
    /// The direct gain (`FUN_00ab9690`, output slot 0): `clamp(E × L × falloff(d), 0, 1)`. `emitter_cos` is the
    /// emitter's forward axis against the unit vector from the listener to the emitter, `listener_cos` the
    /// listener's forward against the same vector. `E = (1 − x)/2 + (1 + x)/2 · (1 − s)` with `s = (focus −
    /// 0.5) · 2` (or the other half scaled by `1 + s` when `s < 0`); `L` the same of `y` with the listener focus
    /// on the half behind the listener (times a listener scalar at listener +0x74, taken as 1).
    pub fn gain(&self, distance: f32, emitter_cos: f32, listener_cos: f32) -> f32 {
        let lobe = |c: f32, focus: f32, away_is_back: bool| {
            let c = c.clamp(-1.0, 1.0);
            let s = (focus - 0.5) * 2.0;
            let (mut back, mut front) =
                if away_is_back { ((1.0 + c) * 0.5, (1.0 - c) * 0.5) } else { ((1.0 - c) * 0.5, (1.0 + c) * 0.5) };
            if s >= 0.0 {
                back *= 1.0 - s;
            } else {
                front *= 1.0 + s;
            }
            back + front
        };
        // The emitter: x = +1 when it faces away from the listener (its back towards it).
        let e = lobe(emitter_cos, self.emitter_focus, true);
        // The listener: y = −1 when the emitter is behind it.
        let l = lobe(listener_cos, self.listener_focus, false);
        (e * l * self.falloff.gain(distance)).clamp(0.0, 1.0)
    }

    /// The doppler pitch factor (`FUN_00ab7260`): `|c / ((v_emitter − v_listener) · u + c)|`, clamped to
    /// 0..[`DOPPLER_MAX`], with `u` the unit vector from the listener to the emitter; 1 without a doppler mode.
    pub fn doppler(&self, relative_speed_away: f32) -> f32 {
        match self.doppler_speed {
            Some(c) => (c / (relative_speed_away + c)).abs().clamp(0.0, DOPPLER_MAX),
            None => 1.0,
        }
    }

    pub fn from_channel(o: &Object, falloff: Option<&Object>) -> Self {
        let n = |name: &str, h: u32| number(o.get(field_hash(name))).or_else(|| number(o.get(h)));
        let doppler_speed = match n("", 0x435e_c694).map(|v| v as i32) {
            Some(1) => Some(680.0),
            Some(2) => Some(340.0),
            Some(3) => Some(170.0),
            _ => None,
        };
        Self {
            falloff: falloff.map(DistanceFalloff::from_object).unwrap_or_default(),
            emitter_focus: n("", 0xd636_5a79).unwrap_or(0.5),
            listener_focus: n("Focus", 0xf094_08fd).unwrap_or(0.5),
            doppler_speed,
        }
    }
}

/// A car engine voice of another car: its bank and how it sits in the world.
#[derive(Debug, Clone)]
pub struct CompetitorEngine {
    pub engine: EngineBank,
    pub spatial: Spatial,
}

/// A traffic car's engine: the bank and the values its bindings read.
#[derive(Debug, Clone)]
pub struct TrafficEngine {
    pub bank: Bank,
    pub spatial: Spatial,
    /// `NormalisedSpeed` divides by this (mph): `HandlingBehaviour.Aerodynamics.f1538c51` (+0x60; 150 / 154 /
    /// 180 over the traffic cars, the values read live in the instances, `FUN_005ee550`).
    pub top_speed_mph: f32,
    /// `Engine.Power_curve.Idle_RPM` (instance +0x00): the RPM when no effect item has set one.
    pub idle_rpm: f32,
}

impl AssetStore {
    /// A `VehicleSound` resource by its name (`"1085007_COMPETITORSOUND"`; resource id = CRC-32 of the lower-case
    /// name, as the game's `FUN_00aceea0` hashes it).
    pub fn vehicle_sound(&self, name: &str) -> Option<VehicleSoundRes> {
        let res = self.load(ResourceId::from_name(name)).ok()?;
        let h = res.header();
        let u32_at = |o: usize| h.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let (offset, count) = (u32_at(0x60)?, u32_at(0x64)?);
        let horns = (0..count.min(64)).filter_map(|k| res.import_at(offset + 8 * k)).collect();
        Some(VehicleSoundRes { engine: res.import_at(0x48), horns })
    }

    /// How a `MixerChannel` places its voices ([`Spatial`]).
    pub fn channel_spatial(&self, channel: ResourceId) -> Option<Spatial> {
        let ch = self.genesys_object(channel).ok()?;
        let falloff = handle_of(ch.get(field_hash("DistanceFalloff")).or_else(|| ch.get(0x0e7f_a14a)))
            .and_then(|f| self.genesys_object(f).ok());
        Some(Spatial::from_channel(&ch, falloff.as_ref()))
    }

    /// The [`Spatial`] of the first item channel of a `Sequence`.
    pub fn sequence_spatial(&self, sequence: ResourceId) -> Option<Spatial> {
        let s = self.genesys_object(sequence).ok()?;
        let Some(Value::Array(items)) = s.get(field_hash("SequenceItems")) else { return None };
        let channel = items.iter().find_map(|it| match it {
            Value::Struct(o) => handle_of(o.get(field_hash("Mixer_Channel"))),
            _ => None,
        })?;
        self.channel_spatial(channel)
    }

    /// How the first channel of an `EngineSound2`'s channel list places the engine (the channels the game
    /// instantiates and positions per engine, `FUN_00600bb0` / `FUN_005ee250`).
    fn engine_spatial(&self, es: &Object) -> Spatial {
        let channel = match es.get(ENGINE_CHANNELS) {
            Some(Value::Array(a)) => a.iter().find_map(|v| handle_of(Some(v))),
            _ => None,
        };
        channel.and_then(|c| self.channel_spatial(c)).unwrap_or_default()
    }

    /// An AI racer's engine: the `EngineSound2` of `<vehicle>_COMPETITORSOUND`.
    pub fn competitor_engine(&self, vehicle: u32) -> Result<Option<CompetitorEngine>> {
        let Some(vs) = self.vehicle_sound(&format!("{vehicle}_COMPETITORSOUND")) else { return Ok(None) };
        let Some(es) = vs.engine else { return Ok(None) };
        let es = self.genesys_object(es)?;
        // Not normalised (unlike the player's bank): the competitor banks' own mix is 24..29 dB below full scale
        // at full throttle over the cars checked, the same range as the traffic banks, so both play at their
        // data level (docs/decomp.md "Other cars' engine sounds", model decisions).
        Ok(Some(CompetitorEngine { engine: self.engine_bank_from(&es, false), spatial: self.engine_spatial(&es) }))
    }

    /// A traffic vehicle type's engine (`component` = its `VehicleComponent` id, e.g. 122816).
    pub fn traffic_engine(&self, component: u32) -> Result<Option<TrafficEngine>> {
        let Some(vs) = self.vehicle_sound(&format!("{component}_COMPETITORSOUND")) else { return Ok(None) };
        let Some(es) = vs.engine else { return Ok(None) };
        let es = self.genesys_object(es)?;
        let mut loader = SoundLoader::new(self);
        let mut bank =
            Bank { master_gain: number(es.get(field_hash("MasterGain"))).unwrap_or(1.0), ..Default::default() };
        if let Some(Value::Array(a)) = es.get(ENGINE_SEQUENCES) {
            for id in a.iter().filter_map(|v| handle_of(Some(v))) {
                if let Some(s) = loader.sequence(id) {
                    bank.sequences.push(s);
                }
            }
        }
        if let Some(Value::Array(a)) = es.get(ENGINE_CHANNELS) {
            for id in a.iter().filter_map(|v| handle_of(Some(v))) {
                loader.note_channel(Some(id));
            }
        }
        bank.channels = loader.channels();
        // The handling the instance reads (`FUN_005ee550`): VehicleComponent → Handling → Aerodynamics / Engine.
        let hb = self
            .genesys_object(ResourceId(0x0100_0000_0000_0000 | component as u64))
            .ok()
            .and_then(|vc| handle_of(vc.get(field_hash("Handling"))))
            .and_then(|h| self.genesys_object(h).ok());
        let sub = |o: &Object, name: &str| match o.get(field_hash(name)) {
            Some(Value::Struct(s)) => Some(s.clone()),
            _ => None,
        };
        let top_speed_mph = hb
            .as_ref()
            .and_then(|hb| sub(hb, "Aerodynamics"))
            .and_then(|a| number(a.get(0xf153_8c51)))
            .filter(|v| *v > 1.0)
            .unwrap_or(150.0);
        let idle_rpm = hb
            .as_ref()
            .and_then(|hb| sub(hb, "Engine"))
            .and_then(|e| sub(&e, "Power_curve"))
            .and_then(|p| number(p.get(field_hash("Idle_RPM"))))
            .unwrap_or(800.0);
        Ok(Some(TrafficEngine { bank, spatial: self.engine_spatial(&es), top_speed_mph, idle_rpm }))
    }
}

/// What a traffic engine's bindings read (`FUN_005ee480` registers them on `Game::Traffic::EngineSoundInstance`).
#[derive(Debug, Clone, Copy, Default)]
pub struct TrafficEngineInput {
    /// `NormalisedSpeed` (`FUN_005ec210`): `clamp(|v · forward| × 2.23694 / top speed (mph), 0, 1)`.
    pub normalised_speed: f32,
    /// `IsWetSurface` (`LAB_00614970`): the world's wet flag (`FUN_00547170` → `FUN_00546c70`), not the car's.
    pub wet: bool,
    /// `IsReversing` (`FUN_00614990`): `v · forward < 0`.
    pub reversing: bool,
    /// `Random` (instance +0x48): drawn once per car when its engine starts.
    pub random: f32,
}

/// `normalised_speed` of [`TrafficEngineInput`] from the car's velocity along its forward axis (m/s).
pub fn traffic_normalised_speed(forward_speed: f32, top_speed_mph: f32) -> f32 {
    (forward_speed.abs() * 2.236_94 / top_speed_mph.max(1.0)).clamp(0.0, 1.0)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TrafficKind {
    Rpm,
    NormalisedSpeed,
    Wet,
    Reversing,
    Random,
    Other,
}

fn traffic_kind(name: &str) -> TrafficKind {
    match name.rsplit('.').next().unwrap_or(name) {
        "RPM" => TrafficKind::Rpm,
        "NormalisedSpeed" => TrafficKind::NormalisedSpeed,
        "IsWetSurface" => TrafficKind::Wet,
        "IsReversing" => TrafficKind::Reversing,
        "Random" => TrafficKind::Random,
        _ => TrafficKind::Other,
    }
}

/// Game state → a traffic engine's bindings. The RPM comes from the bank's `TrafficEffectSequenceItem`s
/// (property 0 → instance +0x3c): the item's curve of its sequence's axis (the normalised speed) between its
/// output range (2000..5550 on `…18231b`'s, the RPM sequence's own axis). Instance +0x3c starts at 0 and the
/// getter (`FUN_005ec1a0`) then falls back to `max(Idle_RPM, wheel spin × 60/2π × 2nd gear × final drive)`;
/// every traffic bank has an effect item spanning its whole axis, so here the fallback is the idle RPM.
pub struct TrafficEngineDriver {
    params: Arc<Params>,
    slots: Vec<(usize, TrafficKind)>,
    /// Effect items: (sequence binding kind, axis, (start, end, value of property 0)).
    effects: Vec<(TrafficKind, (f32, f32), Vec<(f32, f32, ModValue)>)>,
    idle_rpm: f32,
    rpm: f32,
}

impl TrafficEngineDriver {
    pub fn new(engine: &TrafficEngine, params: Arc<Params>) -> Self {
        let slots = params.names().iter().enumerate().map(|(i, n)| (i, traffic_kind(n))).collect();
        let effects = engine
            .bank
            .sequences
            .iter()
            .filter_map(|s| {
                let items: Vec<_> = s
                    .items
                    .iter()
                    .filter_map(|it| match &it.kind {
                        ItemKind::Effect { values } => {
                            values.iter().find(|(p, _)| *p == 0).map(|(_, m)| (it.start, it.end, m.clone()))
                        }
                        _ => None,
                    })
                    .collect();
                (!items.is_empty()).then(|| (traffic_kind(&s.binding), (s.min, s.max), items))
            })
            .collect();
        Self { params, slots, effects, idle_rpm: engine.idle_rpm, rpm: 0.0 }
    }

    /// The RPM the last update wrote.
    pub fn rpm(&self) -> f32 {
        self.rpm
    }

    pub fn update(&mut self, i: &TrafficEngineInput) {
        let value = |k: TrafficKind, rpm: f32| match k {
            TrafficKind::Rpm => Some(rpm),
            TrafficKind::NormalisedSpeed => Some(i.normalised_speed),
            TrafficKind::Wet => Some(f32::from(u8::from(i.wet))),
            TrafficKind::Reversing => Some(f32::from(u8::from(i.reversing))),
            TrafficKind::Random => Some(i.random),
            TrafficKind::Other => None,
        };
        let mut rpm = None;
        for (kind, (lo, hi), items) in &self.effects {
            let Some(x) = value(*kind, 0.0) else { continue };
            let x = if hi > lo { x.clamp(*lo, *hi) } else { x };
            for (start, end, m) in items {
                if x >= *start && x <= *end {
                    let t = if end > start { (x - start) / (end - start) } else { 0.0 };
                    rpm = Some(m.eval(t, None));
                }
            }
        }
        self.rpm = rpm.filter(|r| r.abs() > f32::EPSILON).unwrap_or(self.idle_rpm);
        let names = self.params.names();
        for &(slot, k) in &self.slots {
            if let Some(v) = value(k, self.rpm) {
                self.params.set(&names[slot], v);
            }
        }
    }
}

/// The traffic engine pool (`FUN_0060bc20`, every 0.3 s): candidates are traffic cars within 85 m of the local
/// player's car (`.bss` 0x0110f63c), scored `min(|v|² / 20², 1) × 0.4 + (0.5 + 0.1 if in the view frustum) ×
/// (1 − min(d² / 85², 1))` (`.bss` 0x0110f770 = 1/400, 0x0110f5f0 = 0.4, 0x0110f910 = 0.5, 0x0110f600 = 0.1,
/// 0x0110fbc0 = 1/85²; the frustum test `FUN_00443060` uses a 4 m sphere); the best five with a score above 0
/// keep or get an engine; an engine whose car is farther than 85 m is released (1.5 s fade, `.rdata`
/// 0x00fb1bb0).
pub mod pool {
    pub const SLOTS: usize = 5;
    pub const PERIOD: f32 = 0.3;
    pub const RADIUS: f32 = 85.0;
    pub const RELEASE_FADE: f32 = 1.5;
    pub const FRUSTUM_SPHERE: f32 = 4.0;

    pub fn score(distance_sq: f32, speed_sq: f32, in_view: bool) -> f32 {
        let speed = (speed_sq / 400.0).min(1.0) * 0.4;
        let near = 1.0 - (distance_sq / (RADIUS * RADIUS)).min(1.0);
        speed + (0.5 + if in_view { 0.1 } else { 0.0 }) * near
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn traffic_falloff() -> DistanceFalloff {
        // `…7277`, the traffic engine channel's.
        DistanceFalloff {
            curve: 5.0,
            curve2: 0.2,
            initial_gain: 1.0,
            max_distance: 150.0,
            second_far_distance: 120.0,
            min_distance: 4.0,
            second_near_distance: 4.0,
            peak_gain: 1.0,
            second_near: 63.095_734,
            second_far: 0.251_188_64,
        }
    }

    #[test]
    fn falloff_follows_the_game() {
        let f = traffic_falloff();
        assert_eq!(f.gain(0.0), 1.0);
        assert_eq!(f.gain(4.0), 1.0);
        let at = |d: f32| (1.0 - (d - 4.0) / 146.0).powf(5.0);
        assert!((f.gain(50.0) - at(50.0)).abs() < 1e-6);
        assert_eq!(f.gain(150.0), 0.0);
        assert_eq!(f.gain(300.0), 0.0);
        // Inside MinDistance the gain runs from InitialGain at 0 to PeakGain.
        let g = DistanceFalloff { initial_gain: 2.0, peak_gain: 1.0, ..f };
        assert!((g.gain(0.0) - 2.0).abs() < 1e-6);
        assert!((g.gain(2.0) - (1.0 + 0.5f32.powf(5.0))).abs() < 1e-6);
        assert!((f.second(4.0) - 63.095_734).abs() < 1e-3);
        assert!((f.second(120.0) - 0.251_188_64).abs() < 1e-6);
    }

    #[test]
    fn focus_and_doppler() {
        let s =
            Spatial { falloff: traffic_falloff(), emitter_focus: 0.5, listener_focus: 0.7, doppler_speed: Some(340.0) };
        // In front of the listener: full; behind: 1 − (0.7 − 0.5) · 2 = 0.6.
        assert!((s.gain(2.0, 0.0, 1.0) - 1.0).abs() < 1e-6);
        assert!((s.gain(2.0, 0.0, -1.0) - 0.6).abs() < 1e-6);
        assert!((s.gain(2.0, 0.3, 0.0) - 0.8).abs() < 1e-6);
        // Clamped to 1 even with a gain above it.
        let loud = Spatial { falloff: DistanceFalloff { initial_gain: 5.0, peak_gain: 5.0, ..traffic_falloff() }, ..s };
        assert_eq!(loud.gain(30.0, 0.0, 1.0), 1.0);
        // Approaching at 34 m/s (−34 away): 340 / 306; receding: 340 / 374.
        assert!((s.doppler(-34.0) - 340.0 / 306.0).abs() < 1e-6);
        assert!((s.doppler(34.0) - 340.0 / 374.0).abs() < 1e-6);
        assert_eq!(Spatial::default().doppler(50.0), 1.0);
        assert!(s.doppler(-339.9) <= DOPPLER_MAX);
    }

    #[test]
    fn pool_score() {
        // A car at the player's position moving at 20 m/s scores 0.4 + 0.5 (+0.1 in view).
        assert!((pool::score(0.0, 400.0, false) - 0.9).abs() < 1e-6);
        assert!((pool::score(0.0, 400.0, true) - 1.0).abs() < 1e-6);
        assert!((pool::score(85.0 * 85.0, 0.0, true)).abs() < 1e-6);
        assert!(pool::score(40.0 * 40.0, 100.0, false) > pool::score(40.0 * 40.0, 25.0, false));
    }
}
