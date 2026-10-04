//! Sky and ambient lighting from the environment keyframe (`mw_assets::environment`).
//!
//! The sky follows the game's `NewSky` dome shader: the keyframe's sky gradient (horizon to zenith),
//! a sun glow in `SunColour`, and the keyframe's cloud layer (2048×512 BC3, alpha = coverage) blended in
//! by an alpha threshold. It is baked into cubes: the skybox, the prefiltered reflection cube the
//! material passes read (the game renders its reflection cubes at runtime), and the irradiance cube of
//! the lighting pass, built from the keyframe's six-direction ambient rig.
//!
//! Provisional (not decoded): the sun direction, the glow exponent and scale, how Opacity / Feathering
//! map to the cloud threshold, and the cloud layer's dome mapping. The two HDR panoramas in the bundle
//! (`…3f35`, `…3f39`) are a sunrise capture that no keyframe references; they are not used.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::math::Vec3;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use mw_assets::AssetStore;
use mw_assets::environment::EnvironmentKeyframe;
use mw_bundle::ResourceId;

/// Sun glow `pow(cos, power) × SunColour × scale`: the keyframe's unnamed sky fields `2ec8df83` (500) and
/// `2ec8df82` (1.067) read as power and scale (unconfirmed).
const SUN_GLOW_POWER: f32 = 500.0;
const SUN_GLOW_SCALE: f32 = 1.067;

/// An RGBA float image sampled bilinearly, wrapping in u and clamping in v.
struct Panorama {
    w: usize,
    h: usize,
    px: Vec<[f32; 4]>,
}

impl Panorama {
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let x = u.rem_euclid(1.0) * self.w as f32 - 0.5;
        let y = (v.clamp(0.0, 1.0) * self.h as f32 - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let xi = |d: i64| ((x0 as i64 + d).rem_euclid(self.w as i64)) as usize;
        let yi = |d: usize| (y0 as usize + d).min(self.h - 1);
        let p = |x: usize, y: usize| self.px[y * self.w + x];
        let (a, b, c, d) = (p(xi(0), yi(0)), p(xi(1), yi(0)), p(xi(0), yi(1)), p(xi(1), yi(1)));
        std::array::from_fn(|i| {
            let top = a[i] + (b[i] - a[i]) * fx;
            let bot = c[i] + (d[i] - c[i]) * fx;
            top + (bot - top) * fy
        })
    }
}

fn load_clouds(store: &AssetStore, id: ResourceId) -> Option<Panorama> {
    let (w, h, rgba) = store.texture(id).ok()?.decode_rgba8(0)?;
    let srgb = |c: u8| (c as f32 / 255.0).powf(2.2);
    let px =
        rgba.as_chunks::<4>().0.iter().map(|c| [srgb(c[0]), srgb(c[1]), srgb(c[2]), c[3] as f32 / 255.0]).collect();
    Some(Panorama { w: w as usize, h: h as usize, px })
}

/// Direction of a cube texel (wgpu/D3D face order +X, -X, +Y, -Y, +Z, -Z).
fn texel_dir(face: usize, s: f32, t: f32) -> Vec3 {
    match face {
        0 => Vec3::new(1.0, -t, -s),
        1 => Vec3::new(-1.0, -t, s),
        2 => Vec3::new(s, 1.0, t),
        3 => Vec3::new(s, -1.0, -t),
        4 => Vec3::new(s, -t, 1.0),
        _ => Vec3::new(-s, -t, -1.0),
    }
    .normalize()
}

fn f32_to_f16(v: f32) -> u16 {
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let man = b & 0x7f_ffff;
    if v.is_nan() {
        return 0x7e00;
    }
    if exp >= 31 {
        return sign | 0x7c00;
    }
    if exp <= 0 {
        return sign; // flush tiny values to zero
    }
    sign | ((exp as u16) << 10) | ((man >> 13) as u16)
}

/// The sky as a function of direction, in the game's HDR units (scale by the sky brightness).
pub struct Sky {
    key: EnvironmentKeyframe,
    clouds: Option<Panorama>,
    /// Towards the sun.
    sun: Vec3,
}

impl Sky {
    pub fn new(store: &AssetStore, key: &EnvironmentKeyframe, sun: Vec3) -> Self {
        let clouds = key.clouds.texture.and_then(|t| load_clouds(store, t));
        Self { key: key.clone(), clouds, sun: sun.normalize() }
    }

    pub fn radiance(&self, d: Vec3, clouds: bool) -> [f32; 3] {
        let k = &self.key;
        if d.y < 0.0 {
            // Below the horizon: the horizon colour fading into dark ground.
            let h = k.sky_gradient_at(0.0);
            let f = (-d.y / 0.25).min(1.0);
            return [0, 1, 2].map(|i| h[i] * (1.0 - 0.8 * f));
        }
        let mut c = k.sky_gradient_at(d.y);
        let glow = d.dot(self.sun).max(0.0).powf(SUN_GLOW_POWER) * SUN_GLOW_SCALE;
        for (c, sun) in c.iter_mut().zip(k.sun_colour) {
            *c += sun * glow;
        }
        if let Some(cl) = self.clouds.as_ref().filter(|_| clouds) {
            // NewSky: lerp(sky, cloud.rgb × BaseCloudColour, saturate((cloud.a - threshold) × sharpness));
            // threshold = 1 - Opacity and sharpness = 1 / Feathering here.
            let elevation = d.y.clamp(0.0, 1.0).asin();
            let u = (d.x.atan2(d.z) - self.sun.x.atan2(self.sun.z)) / std::f32::consts::TAU + 0.5;
            let v = 1.0 - elevation / std::f32::consts::FRAC_PI_2;
            let t = cl.sample(u, v);
            let a = ((t[3] - (1.0 - k.clouds.opacity)) / k.clouds.feathering.max(0.02)).clamp(0.0, 1.0);
            for i in 0..3 {
                let cloud = t[i] * k.clouds.light[i] * k.clouds.intensity;
                c[i] += (cloud - c[i]) * a;
            }
        }
        c
    }
}

/// A cube map (`size` per face, `mips` levels, RGBA16F) filled per texel direction and mip level.
fn cube_image(size: usize, mips: usize, f: impl Fn(Vec3, usize) -> [f32; 3] + Sync) -> Image {
    use rayon::prelude::*;
    let mut data = Vec::new();
    // wgpu wants layer-major order: every mip of face 0, then face 1, ...
    for face in 0..6 {
        for mip in 0..mips {
            let n = (size >> mip).max(1);
            let texels: Vec<[f32; 3]> = (0..n * n)
                .into_par_iter()
                .map(|i| {
                    let s = 2.0 * ((i % n) as f32 + 0.5) / n as f32 - 1.0;
                    let t = 2.0 * ((i / n) as f32 + 0.5) / n as f32 - 1.0;
                    f(texel_dir(face, s, t), mip)
                })
                .collect();
            for c in texels {
                for v in [c[0], c[1], c[2], 1.0] {
                    data.extend_from_slice(&f32_to_f16(v).to_le_bytes());
                }
            }
        }
    }
    let mut image = Image::new(
        Extent3d { width: size as u32, height: size as u32, depth_or_array_layers: 6 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data_order = bevy::render::render_resource::TextureDataOrder::LayerMajor;
    image.texture_descriptor.mip_level_count = mips as u32;
    image.texture_view_descriptor =
        Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..Default::default() });
    image.sampler = ImageSampler::linear();
    image
}

/// An orthonormal frame around `n`.
fn frame(n: Vec3) -> (Vec3, Vec3) {
    let up = if n.y.abs() < 0.99 { Vec3::Y } else { Vec3::X };
    let t = up.cross(n).normalize();
    (t, n.cross(t))
}

/// Average of the sky over directions around `d`, weighted by `pow(cos, power)` (cosine-power lobe),
/// from `count` stratified samples.
fn lobe(sky: &Sky, d: Vec3, power: f32, count: usize) -> [f32; 3] {
    let (t, b) = frame(d);
    let mut sum = [0.0f32; 3];
    let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
    for i in 0..count {
        // Importance-sample cos^power: cos(theta) = u^(1 / (power + 1)).
        let u = (i as f32 + 0.5) / count as f32;
        let cos = u.powf(1.0 / (power + 1.0));
        let sin = (1.0 - cos * cos).max(0.0).sqrt();
        let phi = i as f32 * golden;
        let dir = (t * (sin * phi.cos()) + b * (sin * phi.sin()) + d * cos).normalize();
        let c = sky.radiance(dir, false);
        for k in 0..3 {
            sum[k] += c[k];
        }
    }
    sum.map(|v| v / count as f32)
}

/// Skybox: the sky with its sharp cloud layer.
pub fn sky_cube(sky: &Sky) -> Image {
    cube_image(512, 1, |d, _| sky.radiance(d, true))
}

/// Mip levels of [`reflection_cube`]: the game reads its reflection cube at mip = roughness × 5.5.
pub const REFLECTION_MIPS: usize = 7;

/// Prefiltered reflection cube (128² base): mip m holds the sky blurred for roughness m / 5.5, as the
/// game's reflection lookup `cube(R, roughness × 5.5)` expects.
pub fn reflection_cube(sky: &Sky) -> Image {
    cube_image(128, REFLECTION_MIPS, |d, mip| {
        if mip == 0 {
            return sky.radiance(d, true);
        }
        // Blinn-Phong power of the game's key-light model at this roughness: exp(11 - 11 r).
        let r = (mip as f32 / 5.5).min(1.0);
        let power = (11.0 - 11.0 * r).exp() * 0.25;
        lobe(sky, d, power.max(1.0), 64 << mip.min(3))
    })
}

/// Irradiance cube for the ambient term, from the keyframe's six-direction light rig: an ambient cube
/// oriented to the key light's azimuth (key fill towards it, shadow fill away, left / right across, up,
/// down), weighted by the squared cosine to each axis, × `AmbientIrradianceScale`. Multiply by albedo.
pub fn irradiance_cube(key: &EnvironmentKeyframe, sun: Vec3) -> Image {
    let fwd = Vec3::new(sun.x, 0.0, sun.z).try_normalize().unwrap_or(Vec3::Z);
    let side = Vec3::Y.cross(fwd);
    let r = &key.ambient;
    let axes = [(fwd, r.key), (-fwd, r.shadow), (Vec3::Y, r.up), (-Vec3::Y, r.down), (side, r.left), (-side, r.right)];
    cube_image(16, 1, |d, _| {
        let mut c = [0.0f32; 3];
        for (axis, fill) in axes {
            let w = d.dot(axis).max(0.0).powi(2);
            for i in 0..3 {
                c[i] += fill[i] * w * r.scale;
            }
        }
        c
    })
}
