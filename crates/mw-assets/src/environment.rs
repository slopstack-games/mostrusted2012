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

/// The timeline free roam most plausibly uses: the lowest-numbered one, with the most common fog variant
/// (what selects a timeline in game is not found yet).
pub const DEFAULT_TIMELINE: ResourceId = ResourceId(0x0100_0000_0006_9cbb);

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
        let mut out = Self { id, colour_cube: handle(o, h("ColourCube")), ..Default::default() };
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

impl AssetStore {
    /// The keyframe a timeline uses at an hour (0–23): the entry for that hour, else the latest earlier
    /// one.
    pub fn environment_keyframe_at(&self, timeline: ResourceId, hour: u32) -> Result<EnvironmentKeyframe> {
        let tl = self.decode_genesys(&self.load(timeline)?)?;
        let entries: Vec<(i64, ResourceId)> = match tl.get(TIMELINE_ENTRIES) {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|e| match e {
                    Value::Struct(s) => {
                        let hour = float(s, ENTRY_HOUR) as i64;
                        Some((hour, handle(s, field_hash("Keyframe"))?))
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let id = entries
            .iter()
            .filter(|(h, _)| *h <= hour as i64)
            .max_by_key(|(h, _)| *h)
            .or_else(|| entries.first())
            .map(|(_, id)| *id)
            .ok_or(Error::Missing(timeline))?;
        self.environment_keyframe(id)
    }

    pub fn environment_keyframe(&self, id: ResourceId) -> Result<EnvironmentKeyframe> {
        EnvironmentKeyframe::from_object(id, &self.decode_genesys(&self.load(id)?)?)
    }
}
