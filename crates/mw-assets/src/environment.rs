//! Time of day: `Genesys.Gen.EnvironmentTimeline` / `EnvironmentKeyframe` objects in
//! `HAWAII/ENVIRONMENT.BNDL` (there are no 0x112–0x114 resources in the PC install; it is all GeneSys).
//!
//! A timeline lists `{Keyframe handle, hour}` entries (21 per timeline; hours 11, 15 and 16 are absent,
//! so a missing hour takes the previous entry). A keyframe holds the key light, the six-direction ambient
//! rig, the sky (gradient, sun colour, reflection averages), the cloud layer, fog, camera / bloom and
//! weather settings, plus a `ColourCube` handle. Field names were cracked with the shader constant names
//! as vocabulary (research log 2026-10-04); values are in the game's HDR units.

use mw_bundle::ResourceId;
use mw_formats::genesys::{Object, Value, field_hash};

use crate::{AssetStore, Error, Result};

/// The lowest-numbered timeline, entry 1 of the timeline paint map (which covers the north-west of the
/// map with it). Used where nothing is painted.
pub const DEFAULT_TIMELINE: ResourceId = ResourceId(0x0100_0000_0006_9cbb);
/// The `EnvironmentSettings` object (sun / moon path, rear-view mirror, car-select lighting).
pub const SETTINGS: ResourceId = ResourceId(0x0100_0000_0006_9cad);
/// The `WorldPaintMap` that paints the 25 timelines over the world (`EnvironmentSettings` field
/// `409f546f` holds its ID's low bits): which timeline lights each place.
pub const TIMELINE_PAINT_MAP: ResourceId = ResourceId(0x0100_0000_0002_823d);
/// The `WorldPaintMap` of the 19 `LayerFog` objects (`EnvironmentSettings` field `ae067627`).
pub const LAYER_FOG_PAINT_MAP: ResourceId = ResourceId(0x0100_0000_0002_85cb);
/// The road shader's `puddleMapSampler` (bound by the engine, not by any material): a 256² tiling map in
/// `GLOBALTEXTUREDICTIONARY.BNDL`. x = how much of the road normal survives (low in puddles), y =
/// roughness scale (≈ 0.07, ≈ 0 in puddles), z = albedo darkening (0.45 in puddles, ≈ 0.8 around), w = wet
/// amount (1 in puddles, ≈ 0.55 around). Identified by its channels matching the shader's use of them.
pub const PUDDLE_MAP: ResourceId = ResourceId(0x0100_0000_0000_7811);

/// Sub-structure field hashes whose names are not cracked.
const SKY: u32 = 0x7dfab0cc;
const FOG: u32 = 0x6ada1ed8;
/// Timeline entry list and the entry's hour.
const TIMELINE_ENTRIES: u32 = 0xccdbc945;
const ENTRY_HOUR: u32 = 0xda6ac384;

#[derive(Debug, Clone, Default)]
pub struct AmbientRig {
    /// Fill towards the key light's azimuth, away from it, up, down, and to its left and right.
    pub key: [f32; 3],
    pub shadow: [f32; 3],
    pub up: [f32; 3],
    pub down: [f32; 3],
    pub left: [f32; 3],
    pub right: [f32; 3],
    pub scale: f32,
}

#[derive(Debug, Clone, Default)]
pub struct Fog {
    pub colour: [f32; 3],
    /// Five floats: an rgb colour towards the sun and two gradient parameters (not decoded).
    pub colour_sun: Vec<f32>,
    pub near: f32,
    pub far: f32,
    pub power: f32,
    pub scattering_colour: [f32; 3],
    pub scattering_phase: f32,
    pub scattering_strength: f32,
}

#[derive(Debug, Clone, Default)]
pub struct Clouds {
    pub texture: Option<ResourceId>,
    pub dark: [f32; 3],
    pub light: [f32; 3],
    pub opacity: f32,
    pub feathering: f32,
    pub intensity: f32,
    /// `Scale` (3.5 at noon, 5 on the noon Alternate) and `Speed` (5): the NewSky layers' uv scale and
    /// scroll speed are set from these on the CPU (mapping not decoded).
    pub scale: f32,
    pub speed: f32,
}

/// `EnvironmentKeyframe.Weather`. There is no precipitation anywhere in the data (no rain field, texture,
/// shader or effect): weather is wind, a night-time thunderstorm intensity, the Alternate keyframes and
/// the static road wetness (research log "weather").
#[derive(Debug, Clone, Default)]
pub struct Weather {
    /// Degrees (130 on every keyframe), `WindBearingVariability` ±25°.
    pub wind_bearing: f32,
    pub wind_bearing_variability: f32,
    /// 1 and 3.5 on every keyframe (m/s, assumed).
    pub wind_speed_min: f32,
    pub wind_speed_max: f32,
    /// `Thunderstorm_Intensity`: 0.23 on the 42 night keyframes (00:00–04:00), else 0. How the game
    /// shows it is not in the data (no lightning shader or texture found).
    pub thunderstorm_intensity: f32,
    /// `0141d842`: 0.25 on daytime keyframes (05:00–15:00), 0.15 in the evening, at night and on every
    /// Alternate (unnamed).
    pub unknown_0141d842: f32,
}

/// The keyframe's tonemap / auto-exposure settings (`EnvironmentKeyframe.Camera`), in the order the game's
/// `TonemapCameraConstants` / `TonemapExposureConstants` / `TonemapExposureSettingConstants` use them.
/// Field names are not cracked; the roles come from the tonemap shaders (research log "the game's
/// tonemap") and from how the values move over the day, so they are provisional. Exposures are log2.
#[derive(Debug, Clone, Default)]
pub struct CameraExposure {
    /// `3bec5831` / `3bec5027`: the range auto exposure may move in (−8…8 over all keyframes, min ≤ max on
    /// every one; −0.2…3 at noon, 2.7…3.5 at night).
    pub min: f32,
    pub max: f32,
    /// `ExposureCompensationBias`: added to the adapted exposure (3 at night, else 0).
    pub bias: f32,
    /// `4b48cdce` / `ec890b00`: the scene luminance range over which the target luma moves (low ≤ high on
    /// every keyframe).
    pub luminance_low: f32,
    pub luminance_high: f32,
    /// `9355acc0` / `8357903e`: displayed-luma targets at the low and high ends of that range.
    pub target_low: f32,
    pub target_high: f32,
    /// `0abf68c5` (0–0.1): the luma dead band around the target.
    pub tolerance: f32,
    /// `e59ff4ec` (0–2): adaptation speed in stops per second.
    pub speed: f32,
}

#[derive(Debug, Clone, Default)]
pub struct EnvironmentKeyframe {
    pub id: ResourceId,
    pub key_light: [f32; 3],
    pub ambient: AmbientRig,
    /// rgb, and w (0.65 at noon; probably the glow exponent or size, not decoded).
    pub sun_colour: [f32; 4],
    /// Sky gradient stops `(position 0 = horizon .. 1 = zenith, rgb)`, sorted by position.
    pub sky_gradient: Vec<(f32, [f32; 3])>,
    pub sky_reflection_colour: [f32; 3],
    pub sky_reflection_light: [f32; 3],
    pub clouds: Clouds,
    pub fog: Fog,
    pub colour_cube: Option<ResourceId>,
    pub exposure: CameraExposure,
    /// The keyframe's bad-weather twin. 154 of the 175 keyframes name one of only 7 Alternates, one per
    /// time-of-day band (night, dawn ×2, morning, noon, evening, plus one for the front-end timeline
    /// `…9efca`): hazier versions (fog far 550–1200 m instead of 1500+, key light ×0.6–0.8, brighter
    /// shadow fill, thinner sun glow). No timeline lists an Alternate directly; what switches to them
    /// is not in the data.
    pub alternate: Option<ResourceId>,
    pub weather: Weather,
}

fn get(o: &Object, hash: u32) -> Option<&Value> {
    o.get(hash)
}

fn sub(o: &Object, hash: u32) -> Option<&Object> {
    match get(o, hash)? {
        Value::Struct(s) => Some(s),
        _ => None,
    }
}

fn float(o: &Object, hash: u32) -> f32 {
    match get(o, hash) {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f32,
        Some(Value::UInt(u)) => *u as f32,
        _ => 0.0,
    }
}

fn vec4(o: &Object, hash: u32) -> [f32; 4] {
    match get(o, hash) {
        Some(Value::Vector(v)) => *v,
        _ => [0.0; 4],
    }
}

fn rgb(o: &Object, hash: u32) -> [f32; 3] {
    let v = vec4(o, hash);
    [v[0], v[1], v[2]]
}

fn floats(o: &Object, hash: u32) -> Vec<f32> {
    match get(o, hash) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| match v {
                Value::Float(f) => Some(*f),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn handle(o: &Object, hash: u32) -> Option<ResourceId> {
    match get(o, hash)? {
        Value::Handle(h) => *h,
        _ => None,
    }
}

impl EnvironmentKeyframe {
    pub fn from_object(id: ResourceId, o: &Object) -> Result<Self> {
        if o.type_name != "Genesys.Gen.EnvironmentKeyframe" {
            return Err(Error::Format(mw_formats::Error::Malformed(format!(
                "{id} is {}, not an EnvironmentKeyframe",
                o.type_name
            ))));
        }
        let h = field_hash;
        let mut out = Self {
            id,
            colour_cube: handle(o, h("ColourCube")),
            alternate: handle(o, h("Alternate")),
            ..Default::default()
        };
        if let Some(w) = sub(o, h("Weather")) {
            out.weather = Weather {
                wind_bearing: float(w, h("WindBearing")),
                wind_bearing_variability: float(w, h("WindBearingVariability")),
                wind_speed_min: float(w, h("WindSpeedMin")),
                wind_speed_max: float(w, h("WindSpeedMax")),
                thunderstorm_intensity: float(w, h("Thunderstorm_Intensity")),
                unknown_0141d842: float(w, 0x0141d842),
            };
        }
        if let Some(rig) = sub(o, h("LightRig")) {
            out.key_light = rgb(rig, h("KeyLightColour"));
            out.ambient = AmbientRig {
                key: rgb(rig, h("KeyFillColour")),
                shadow: rgb(rig, h("ShadowFillColour")),
                up: rgb(rig, h("UpFillColour")),
                down: rgb(rig, h("DownFillColour")),
                left: rgb(rig, h("LeftFillColour")),
                right: rgb(rig, h("RightFillColour")),
                scale: float(rig, h("AmbientIrradianceScale")),
            };
        }
        if let Some(c) = sub(o, h("Camera")) {
            out.exposure = CameraExposure {
                min: float(c, 0x3bec5831),
                max: float(c, 0x3bec5027),
                bias: float(c, field_hash("ExposureCompensationBias")),
                luminance_low: float(c, 0x4b48cdce),
                luminance_high: float(c, 0xec890b00),
                target_low: float(c, 0x9355acc0),
                target_high: float(c, 0x8357903e),
                tolerance: float(c, 0x0abf68c5),
                speed: float(c, 0xe59ff4ec),
            };
        }
        if let Some(sky) = sub(o, SKY) {
            out.sun_colour = vec4(sky, h("SunColour"));
            out.sky_reflection_colour = rgb(sky, h("SkyReflectionAverageColour"));
            out.sky_reflection_light = rgb(sky, h("SkyReflectionAverageLight"));
            // Stops of five floats: position, r, g, b, 1 (stored unsorted).
            let g = floats(sky, h("SkyColorGradient"));
            out.sky_gradient = g.as_chunks::<5>().0.iter().map(|c| (c[0], [c[1], c[2], c[3]])).collect();
            out.sky_gradient.sort_by(|a, b| a.0.total_cmp(&b.0));
            out.clouds.intensity = float(sky, h("BaseCloudIntensity"));
        }
        if let Some(c) = sub(o, h("Clouds")) {
            // The texture is stored as the low bits of its resource ID.
            let t = float(c, h("BaseCloudTexture")) as u64;
            out.clouds.texture = (t != 0).then_some(ResourceId(0x0100_0000_0000_0000 | t));
            out.clouds.dark = rgb(c, h("DarkColour"));
            out.clouds.light = rgb(c, h("LightColour"));
            out.clouds.opacity = float(c, h("Opacity"));
            out.clouds.feathering = float(c, h("Feathering"));
            out.clouds.scale = float(c, h("Scale"));
            out.clouds.speed = float(c, h("Speed"));
        }
        if let Some(f) = sub(o, FOG) {
            let sc = floats(f, h("ScatteringColour"));
            out.fog = Fog {
                colour: rgb(f, h("Colour")),
                colour_sun: floats(f, h("ColourSun")),
                near: float(f, h("NearDistance")),
                far: float(f, h("FarDistance")),
                power: float(f, h("Power")),
                scattering_colour: [0, 1, 2].map(|i| sc.get(i).copied().unwrap_or(0.0)),
                scattering_phase: float(f, h("ScatteringPhase")),
                scattering_strength: float(f, h("ScatteringStrength")),
            };
        }
        Ok(out)
    }

    /// Sky gradient colour at a position (0 = horizon, 1 = zenith).
    pub fn sky_gradient_at(&self, t: f32) -> [f32; 3] {
        let g = &self.sky_gradient;
        let Some(first) = g.first() else { return [0.0; 3] };
        if t <= first.0 {
            return first.1;
        }
        for w in g.windows(2) {
            let ((a, ca), (b, cb)) = (w[0], w[1]);
            if t <= b {
                let f = if b > a { (t - a) / (b - a) } else { 0.0 };
                return [0, 1, 2].map(|i| ca[i] + (cb[i] - ca[i]) * f);
            }
        }
        g.last().map_or([0.0; 3], |l| l.1)
    }
}

/// `EnvironmentSettings.SunMoonPath`: where the key light comes from over the day. Only the bearing fields
/// have names; the elevation model and the bearing's reference axis are provisional (research log
/// "sun and moon path").
#[derive(Debug, Clone)]
pub struct SunMoonPath {
    /// Degrees.
    pub bearing_centre: f32,
    pub bearing_variation: f32,
    /// Fraction of a day.
    pub bearing_phase_offset: f32,
    /// `0bb64ed2` (50): the sun's highest elevation, degrees.
    pub sun_max_elevation: f32,
    /// `c915f96d` (40): the moon's highest elevation, degrees.
    pub moon_max_elevation: f32,
    /// `644383ee` (20) / `a5caac0f` (8): moonrise and moonset hours.
    pub moonrise: f32,
    pub moonset: f32,
}

/// Sunrise and sunset hours. Not in the data as far as found: the default timeline's key light turns on at
/// 05:00 (red dawn keyframe) and peaks at 13:00, which a sine arc from 05:00 to 21:00 matches.
const SUNRISE: f32 = 5.0;
const SUNSET: f32 = 21.0;

impl SunMoonPath {
    pub fn from_settings(o: &Object) -> Self {
        let p = sub(o, field_hash("SunMoonPath"));
        let f = |hash: u32| p.map_or(0.0, |p| float(p, hash));
        Self {
            bearing_centre: f(field_hash("BearingCentre")),
            bearing_variation: f(field_hash("BearingVariation")),
            bearing_phase_offset: f(field_hash("BearingPhaseOffset")),
            sun_max_elevation: f(0x0bb64ed2),
            moon_max_elevation: f(0xc915f96d),
            moonrise: f(0x644383ee),
            moonset: f(0xa5caac0f),
        }
    }

    /// Unit vector towards the key light (the sun by day, the moon by night) at an hour, in game
    /// coordinates (Y up). Bearing: `BearingCentre + BearingVariation × sin(2π (hour / 24 +
    /// BearingPhaseOffset))`, measured clockwise from +Z towards +X (assumed). Elevation: a sine arc
    /// peaking at the max elevation between rise and set.
    pub fn key_light_direction(&self, hour: f32) -> [f32; 3] {
        let bearing = self.bearing_centre
            + self.bearing_variation * (std::f32::consts::TAU * (hour / 24.0 + self.bearing_phase_offset)).sin();
        let arc = |t: f32, max: f32| max * (std::f32::consts::PI * t.clamp(0.0, 1.0)).sin();
        let elevation = if (SUNRISE..=SUNSET).contains(&hour) {
            arc((hour - SUNRISE) / (SUNSET - SUNRISE), self.sun_max_elevation)
        } else {
            // The moon is up from moonrise through midnight to moonset.
            let span = (self.moonset + 24.0 - self.moonrise).max(1.0);
            let t = (hour - self.moonrise).rem_euclid(24.0) / span;
            arc(t, self.moon_max_elevation)
        }
        .max(3.0);
        let (b, e) = (bearing.to_radians(), elevation.to_radians());
        [b.sin() * e.cos(), e.sin(), b.cos() * e.cos()]
    }
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| mix(a[i], b[i], t))
}

impl EnvironmentKeyframe {
    /// A keyframe between `self` (t = 0) and `other` (t = 1): every value interpolated linearly, the sky
    /// gradient resampled at the union of both keyframes' stops, the cloud texture, colour cube and
    /// handles taken from the nearer keyframe.
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let near = if t < 0.5 { self } else { other };
        let mut positions: Vec<f32> = self.sky_gradient.iter().chain(&other.sky_gradient).map(|s| s.0).collect();
        positions.sort_by(f32::total_cmp);
        positions.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
        let sky_gradient =
            positions.iter().map(|&p| (p, mix3(self.sky_gradient_at(p), other.sky_gradient_at(p), t))).collect();
        let (a, b) = (&self.ambient, &other.ambient);
        let (fa, fb) = (&self.fog, &other.fog);
        let (ea, eb) = (&self.exposure, &other.exposure);
        let (wa, wb) = (&self.weather, &other.weather);
        let (ca, cb) = (&self.clouds, &other.clouds);
        Self {
            id: near.id,
            key_light: mix3(self.key_light, other.key_light, t),
            ambient: AmbientRig {
                key: mix3(a.key, b.key, t),
                shadow: mix3(a.shadow, b.shadow, t),
                up: mix3(a.up, b.up, t),
                down: mix3(a.down, b.down, t),
                left: mix3(a.left, b.left, t),
                right: mix3(a.right, b.right, t),
                scale: mix(a.scale, b.scale, t),
            },
            sun_colour: [0, 1, 2, 3].map(|i| mix(self.sun_colour[i], other.sun_colour[i], t)),
            sky_gradient,
            sky_reflection_colour: mix3(self.sky_reflection_colour, other.sky_reflection_colour, t),
            sky_reflection_light: mix3(self.sky_reflection_light, other.sky_reflection_light, t),
            clouds: Clouds {
                texture: near.clouds.texture,
                dark: mix3(ca.dark, cb.dark, t),
                light: mix3(ca.light, cb.light, t),
                opacity: mix(ca.opacity, cb.opacity, t),
                feathering: mix(ca.feathering, cb.feathering, t),
                intensity: mix(ca.intensity, cb.intensity, t),
                scale: mix(ca.scale, cb.scale, t),
                speed: mix(ca.speed, cb.speed, t),
            },
            fog: Fog {
                colour: mix3(fa.colour, fb.colour, t),
                colour_sun: (0..fa.colour_sun.len().max(fb.colour_sun.len()))
                    .map(|i| {
                        let g = |v: &[f32]| v.get(i).copied().unwrap_or(0.0);
                        mix(g(&fa.colour_sun), g(&fb.colour_sun), t)
                    })
                    .collect(),
                near: mix(fa.near, fb.near, t),
                far: mix(fa.far, fb.far, t),
                power: mix(fa.power, fb.power, t),
                scattering_colour: mix3(fa.scattering_colour, fb.scattering_colour, t),
                scattering_phase: mix(fa.scattering_phase, fb.scattering_phase, t),
                scattering_strength: mix(fa.scattering_strength, fb.scattering_strength, t),
            },
            colour_cube: near.colour_cube,
            exposure: CameraExposure {
                min: mix(ea.min, eb.min, t),
                max: mix(ea.max, eb.max, t),
                bias: mix(ea.bias, eb.bias, t),
                luminance_low: mix(ea.luminance_low, eb.luminance_low, t),
                luminance_high: mix(ea.luminance_high, eb.luminance_high, t),
                target_low: mix(ea.target_low, eb.target_low, t),
                target_high: mix(ea.target_high, eb.target_high, t),
                tolerance: mix(ea.tolerance, eb.tolerance, t),
                speed: mix(ea.speed, eb.speed, t),
            },
            alternate: near.alternate,
            weather: Weather {
                wind_bearing: mix(wa.wind_bearing, wb.wind_bearing, t),
                wind_bearing_variability: mix(wa.wind_bearing_variability, wb.wind_bearing_variability, t),
                wind_speed_min: mix(wa.wind_speed_min, wb.wind_speed_min, t),
                wind_speed_max: mix(wa.wind_speed_max, wb.wind_speed_max, t),
                thunderstorm_intensity: mix(wa.thunderstorm_intensity, wb.thunderstorm_intensity, t),
                unknown_0141d842: mix(wa.unknown_0141d842, wb.unknown_0141d842, t),
            },
        }
    }
}

/// Where the environment at an hour comes from: the two timeline keyframes around it and the blend.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelinePosition {
    pub from: ResourceId,
    pub to: ResourceId,
    /// 0 = `from`, 1 = `to`.
    pub t: f32,
}

impl AssetStore {
    pub fn sun_moon_path(&self) -> Result<SunMoonPath> {
        Ok(SunMoonPath::from_settings(&self.decode_genesys(&self.load(SETTINGS)?)?))
    }

    /// A timeline's `(hour, keyframe)` entries in hour order.
    pub fn timeline_entries(&self, timeline: ResourceId) -> Result<Vec<(f32, ResourceId)>> {
        let tl = self.decode_genesys(&self.load(timeline)?)?;
        let mut entries: Vec<(f32, ResourceId)> = match tl.get(TIMELINE_ENTRIES) {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|e| match e {
                    Value::Struct(s) => Some((float(s, ENTRY_HOUR), handle(s, field_hash("Keyframe"))?)),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        entries.sort_by(|a, b| a.0.total_cmp(&b.0));
        if entries.is_empty() {
            return Err(Error::Missing(timeline));
        }
        Ok(entries)
    }

    /// The keyframe a timeline uses at an hour (0–23): the entry for that hour, else the latest earlier
    /// one.
    pub fn environment_keyframe_at(&self, timeline: ResourceId, hour: u32) -> Result<EnvironmentKeyframe> {
        let entries = self.timeline_entries(timeline)?;
        let id = entries
            .iter()
            .rfind(|(h, _)| *h <= hour as f32)
            .or_else(|| entries.first())
            .map(|(_, id)| *id)
            .ok_or(Error::Missing(timeline))?;
        self.environment_keyframe(id)
    }

    /// The two entries around an hour (0–24, wrapping) and how far between them it lies. Timelines hold
    /// a keyframe over several hours by repeating it (00–03, 05–06, 07–08, ...), which only makes sense
    /// if the game interpolates between entries, so the blend is linear in time (provisional).
    pub fn timeline_position(&self, timeline: ResourceId, hour: f32) -> Result<TimelinePosition> {
        let entries = self.timeline_entries(timeline)?;
        let hour = hour.rem_euclid(24.0);
        let i = entries.iter().rposition(|(h, _)| *h <= hour).unwrap_or(entries.len() - 1);
        let (h0, from) = entries[i];
        let (h1, to) = entries[(i + 1) % entries.len()];
        let span = (h1 - h0).rem_euclid(24.0);
        let t = if span > 0.0 { (hour - h0).rem_euclid(24.0) / span } else { 0.0 };
        Ok(TimelinePosition { from, to, t })
    }

    /// The environment of a timeline at an hour, blended between its keyframes; with `alternate`, from
    /// the keyframes' Alternate (bad-weather) twins where they have one.
    pub fn environment_at(&self, timeline: ResourceId, hour: f32, alternate: bool) -> Result<EnvironmentKeyframe> {
        let p = self.timeline_position(timeline, hour)?;
        let load = |id: ResourceId| -> Result<EnvironmentKeyframe> {
            let k = self.environment_keyframe(id)?;
            match k.alternate.filter(|_| alternate) {
                Some(alt) => self.environment_keyframe(alt),
                None => Ok(k),
            }
        };
        let from = load(p.from)?;
        if p.from == p.to || p.t <= 0.0 {
            return Ok(from);
        }
        Ok(from.lerp(&load(p.to)?, p.t))
    }

    /// The timeline painted at a world position (`TIMELINE_PAINT_MAP`), or [`DEFAULT_TIMELINE`] where
    /// nothing is painted.
    pub fn environment_timeline_at(&self, x: f32, z: f32) -> Result<ResourceId> {
        let map = mw_formats::WorldPaintMap::parse(&self.load(TIMELINE_PAINT_MAP)?)?;
        Ok(map.entry_at(x, z).map_or(DEFAULT_TIMELINE, |e| ResourceId(0x0100_0000_0000_0000 | e as u64)))
    }

    pub fn environment_keyframe(&self, id: ResourceId) -> Result<EnvironmentKeyframe> {
        EnvironmentKeyframe::from_object(id, &self.decode_genesys(&self.load(id)?)?)
    }
}
