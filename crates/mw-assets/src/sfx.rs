//! The player car's sounds other than the engine, from the game's sound data:
//!
//! - **Tyres and road**: the default surface's `SurfaceSounds` (`HAWAII/SURFACELIST.BNDL`: `Surfaces.
//!   Default_surface` → `SurfaceType`, dry conditions → `Normal` contact → `Surface_Sounds`): the `Drift`,
//!   `Spin` and `Brake` sequences (bound to `LatSlipIntensityForSkidSound`, `SpinIntensityForSkidSound`,
//!   `SpinBrakingIntensityForSkidSound`) and two road-noise sequences bound to `GetSpeedMph`. Surface types
//!   are not decoded yet, so every road is the default surface.
//! - **Wind**: the speed-bound player sequences of `VehicleSound.Player_Sequences` (`SOUND/GLOBAL.BNDL`):
//!   wind layers crossfaded over 45–250 mph.
//! - **Impacts and scrapes**: the `CollisionSoundSet` (`SOUND/METALCAR.BNDL`) every car's `Vehicle_Sound`
//!   points to. Its 31 sequence slots are not named; they are timelines of crunch / debris waves, sorted here
//!   into light, medium and heavy cues by their loudness. Its `Scratch` waves become a scrape loop bound to
//!   the sliding speed, and its `Sparks` waves a cue.

use std::sync::Arc;

use mw_audio::sfx_player::{Bank, Fade, Item, ItemKind, ModValue, Sequence};
use mw_bundle::ResourceType;
use mw_formats::genesys::{Object, Value, field_hash};

use crate::AssetStore;
use crate::sfx_loader::{SoundLoader, handle, handles};
use crate::vehicle_driver::{SkidRamp, TyreSoundParams};

/// Cue and binding names of the vehicle bank. The `Custom.` bindings are ours (the scrape layer is
/// assembled here from the collision set's `Scratch` waves).
pub mod names {
    pub const IMPACT_LIGHT: &str = "impact.light";
    pub const IMPACT_MEDIUM: &str = "impact.medium";
    pub const IMPACT_HEAVY: &str = "impact.heavy";
    pub const SPARKS: &str = "impact.sparks";
    pub const SCRAPE_SPEED: &str = "Custom.ScrapeSpeed";
}

/// Sequence fields of `CollisionSoundSet` (unnamed hashes, offset order) and its wave pools.
const COLLISION_SEQUENCES: [u32; 31] = [
    0xc90e8812, 0xd12772a1, 0xd92bbf4e, 0xb3779198, 0x50e30e9d, 0x3aecf1ae, 0x7f33f42c, 0x156fdafa, 0x33f0a487,
    0xc6308362, 0xc968909e, 0xa334be48, 0x333a28fc, 0x59b8870e, 0x33e4a9d8, 0xad3eb9ae, 0x8392784d, 0xe48c9a9d,
    0x455dbd2e, 0xfd2ccbbb, 0x9770e56d, 0xa9ab938f, 0xbebd0ad5, 0xb26e4dfe, 0x9938110b, 0x077e73d9, 0x929af217,
    0x3f3ae0de, 0x5566ce08, 0xfd3bb937, 0xb7355c0d,
];
/// Dry-weather conditions of a `SurfaceType` (the first of its two condition sets; the second has the same
/// skid sequences and a wet road noise).
const SURFACE_DRY: u32 = 0xf395_b019;
/// Road-noise sequences of `SurfaceSounds` (unnamed, bound to the car's speed).
const SURFACE_ROAD: [u32; 2] = [0x4648_873a, 0x1376_f03b];

/// Our mix (assumed: the bank's sounds reach the master through buses outside the bundles we load, as the
/// engine's do). Measured on `mwtool vehicle-sound` against the engine at full throttle (≈ −14 dBFS): road and
/// wind at 60 mph ≈ −25 dBFS, a 30° drift ≈ −12.5 (−16.7 before the 32 kHz waves decoded: four of the drift
/// layers were a 47-sample buzz), locked brakes ≈ −13, a heavy impact ≈ −8.
const MASTER_GAIN: f32 = 1.4;
const SKID_GAINS: [(&str, f32); 3] = [("Drift", 4.0), ("Spin", 3.5), ("Brake", 2.5)];
const IMPACT_GAIN: f32 = 3.0;
const SCRAPE_GAIN: f32 = 2.5;

fn first_object(store: &AssetStore, bundle: &str, type_name: &str) -> Option<Object> {
    let b = store.bundle(bundle)?;
    b.entries_of_type(ResourceType::GenesysObject)
        .filter_map(|e| store.decode_genesys(&b.load(e).ok()?).ok())
        .find(|o| o.type_name == type_name)
}

fn sub(o: &Object, hash: u32) -> Option<&Object> {
    match o.get(hash)? {
        Value::Struct(s) => Some(s),
        _ => None,
    }
}

/// A one-shot made into a loop: its last 30 ms are crossfaded into its start, so the wrap is seamless.
fn seamless_loop(c: &mw_audio::sfx_player::Clip) -> mw_audio::sfx_player::Clip {
    let k = (c.rate as usize * 3 / 100).min(c.samples.len() / 4);
    let n = c.samples.len() - k;
    let mut samples = c.samples[..n].to_vec();
    for (i, s) in samples.iter_mut().take(k).enumerate() {
        let w = i as f32 / k as f32;
        *s = (*s as f32 * w + c.samples[n + i] as f32 * (1.0 - w)) as i16;
    }
    mw_audio::sfx_player::Clip { samples, rate: c.rate, looping: true }
}

/// A sequence with every wave item's gain scaled (its static value and output range), our mix level.
fn scaled(mut s: Sequence, g: f32) -> Sequence {
    for it in &mut s.items {
        if let ItemKind::Wave { gain, .. } = &mut it.kind {
            gain.value *= g;
            gain.out = (gain.out.0 * g, gain.out.1 * g);
        }
    }
    s
}

/// Loudness of a timeline sequence: the sum over its wave items of the mean RMS of their waves times the item
/// gain's static value.
fn loudness(s: &Sequence) -> f32 {
    s.items
        .iter()
        .filter_map(|i| match &i.kind {
            ItemKind::Wave { waves, gain, .. } => {
                let rms: f32 = waves
                    .iter()
                    .map(|w| {
                        let n = w.samples.len().max(1) as f64;
                        (w.samples.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / n).sqrt() as f32
                    })
                    .sum::<f32>()
                    / waves.len().max(1) as f32;
                Some(rms * gain.value.abs())
            }
            _ => None,
        })
        .sum()
}

/// One `TyreSoundParams` ramp struct (field roles from `FUN_009d1bf0`, docs/decomp.md): `Curve_power`, the
/// fall rate `07c5dd70`, the rise rate `7961414d`, and the given start / full / full-speed fields.
fn skid_ramp(o: &Object, start: u32, full: u32, speed: Option<u32>) -> Option<SkidRamp> {
    let f = |h: u32| crate::sfx_loader::number(o.get(h));
    Some(SkidRamp {
        start: f(start)?,
        full: f(full)?,
        full_speed_mph: speed.map_or(Some(0.0), f)?,
        curve_power: f(field_hash("Curve_power")).unwrap_or(1.0),
        rise: f(0x7961_414d)?,
        fall: f(0x07c5_dd70)?,
    })
}

impl AssetStore {
    /// The skid-sound tuning (`TyreSoundParams`, `HAWAII/SURFACELIST.BNDL`; the install has one). `None` when
    /// it is missing or a field does not read.
    pub fn tyre_sound_params(&self) -> Option<TyreSoundParams> {
        let o = first_object(self, "HAWAII/SURFACELIST.BNDL", "Genesys.Gen.TyreSoundParams")?;
        const FULL_SPEED: u32 = 0x4b05_7923;
        Some(TyreSoundParams {
            lat_slip: skid_ramp(sub(&o, 0x0735_e463)?, 0x2f5a_0697, 0x166e_d1d6, Some(FULL_SPEED))?,
            long_spin: skid_ramp(sub(&o, 0x0300_16b3)?, 0x9cc6_4e8b, 0xe412_7a7c, None)?,
            long_spin_braking: skid_ramp(sub(&o, 0x6422_2c3f)?, 0x551d_00de, 0x3fa7_1b19, Some(FULL_SPEED))?,
        })
    }

    /// The player car's tyre, road, wind and impact sounds ([`crate::sfx`]).
    pub fn vehicle_sfx_bank(&self) -> crate::Result<Bank> {
        let mut loader = SoundLoader::new(self);
        let mut bank = Bank { master_gain: MASTER_GAIN, ..Default::default() };

        // Tyres and road: the default surface.
        let surfaces = first_object(self, "HAWAII/SURFACELIST.BNDL", "Genesys.Gen.Surfaces");
        let sounds = surfaces
            .as_ref()
            .and_then(|s| handle(s, "Default_surface"))
            .and_then(|id| loader.object(id))
            .and_then(|t| {
                let normal = sub(&t, SURFACE_DRY).and_then(|c| sub(c, field_hash("Normal")))?;
                handle(normal, "Surface_Sounds")
            })
            .and_then(|id| loader.object(id));
        if let Some(ss) = &sounds {
            let ids = SKID_GAINS.iter().map(|&(n, g)| (field_hash(n), g)).chain(SURFACE_ROAD.map(|h| (h, 1.0)));
            for (h, gain) in ids {
                let id = match ss.get(h) {
                    Some(Value::Handle(Some(id))) => *id,
                    _ => continue,
                };
                if let Some(s) = loader.sequence(id) {
                    bank.sequences.push(scaled(s, gain));
                }
            }
        }

        // Wind: the speed-bound player sequences.
        if let Some(vs) = first_object(self, "SOUND/GLOBAL.BNDL", "Genesys.Gen.VehicleSound") {
            for id in handles(&vs, field_hash("Player_Sequences")) {
                let Some(s) = loader.sequence(id) else { continue };
                if s.binding.ends_with("GetPlayerSpeed")
                    && s.items.iter().any(|i| matches!(i.kind, ItemKind::Wave { .. }))
                {
                    bank.sequences.push(s);
                }
            }
        }

        // Impacts, scrapes, sparks.
        if let Some(cs) = first_object(self, "SOUND/METALCAR.BNDL", "Genesys.Gen.CollisionSoundSet") {
            let mut crashes: Vec<(f32, Sequence)> = COLLISION_SEQUENCES
                .iter()
                .filter_map(|&h| match cs.get(h) {
                    Some(Value::Handle(Some(id))) => loader.sequence(*id),
                    _ => None,
                })
                .filter(|s| s.binding.is_empty() && s.items.iter().any(|i| matches!(i.kind, ItemKind::Wave { .. })))
                .map(|s| (loudness(&s), s))
                .collect();
            crashes.sort_by(|a, b| a.0.total_cmp(&b.0));
            let n = crashes.len();
            for (k, (_, s)) in crashes.into_iter().enumerate() {
                let (cue, level) = match k * 3 / n.max(1) {
                    0 => (names::IMPACT_LIGHT, 0.4),
                    1 => (names::IMPACT_MEDIUM, 0.8),
                    _ => (names::IMPACT_HEAVY, 1.4),
                };
                bank.sequences.push(scaled(s, IMPACT_GAIN * level));
                let i = bank.sequences.len() - 1;
                match bank.cues.iter_mut().find(|(c, _)| c == cue) {
                    Some((_, v)) => v.push(i),
                    None => bank.cues.push((cue.to_string(), vec![i])),
                }
            }
            let pool = |name: &str| -> Vec<Arc<mw_audio::sfx_player::Clip>> {
                handles(&cs, field_hash(name)).into_iter().filter_map(|w| loader.wave(w)).collect()
            };
            let wave_item = |waves, start: f32, end: f32, fade: Fade, gain: ModValue| Item {
                start,
                end,
                fade_in: fade,
                fade_out: Fade::default(),
                enabled: String::new(),
                channel: None,
                kind: ItemKind::Wave { waves, gain, pitch: ModValue::constant(1.0), probability: None },
            };
            // The scratch waves are 3 s one-shots; played as loops they hold for as long as the body slides.
            let scratch: Vec<_> = pool("Scratch").into_iter().map(|c| Arc::new(seamless_loop(&c))).collect();
            if !scratch.is_empty() {
                // Sliding contact (m/s): fades in from 1 to 3 m/s, louder up to 20.
                let mut gain = ModValue::constant(1.0);
                gain.binding = names::SCRAPE_SPEED.to_string();
                gain.range = (1.0, 20.0);
                gain.out = (0.25 * SCRAPE_GAIN, SCRAPE_GAIN);
                let fade = Fade { time: 2.0, curve: 1.0 };
                bank.sequences.push(Sequence {
                    id: 0,
                    binding: names::SCRAPE_SPEED.to_string(),
                    min: 0.0,
                    max: 60.0,
                    items: vec![wave_item(scratch, 1.0, 60.0, fade, gain)],
                    gate: String::new(),
                    retrigger: false,
                });
            }
            let sparks = pool("Sparks");
            if !sparks.is_empty() {
                bank.sequences.push(Sequence {
                    id: 0,
                    binding: String::new(),
                    min: 0.0,
                    max: 0.0,
                    items: vec![wave_item(sparks, 0.0, 1.0, Fade::default(), ModValue::constant(0.7))],
                    gate: String::new(),
                    retrigger: false,
                });
                bank.cues.push((names::SPARKS.to_string(), vec![bank.sequences.len() - 1]));
            }
        }
        bank.channels = loader.channels();
        Ok(bank)
    }
}
