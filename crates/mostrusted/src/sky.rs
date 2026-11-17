//! Sky and ambient lighting from the environment keyframe (`mw_assets::environment`).
//!
//! The sky follows the game's `NewSky` dome shader: the keyframe's sky gradient (horizon to zenith),
//! a sun glow in `SunColour`, and the keyframe's cloud layer (2048×512 BC3, alpha = coverage) blended in
//! by an alpha threshold. It is baked into cubes: the skybox, the prefiltered reflection cube the
//! material passes read (the game renders its reflection cubes at runtime), and the irradiance cube of
//! the lighting pass, built from the keyframe's six-direction ambient rig.
//!
//! From the `NewSky` shader: the glow is pow(saturate(dir · L), SunColour.w) × SunColour.rgb; the shader
//! blends two 1-D gradients (A towards the sun, B away, by saturate(dot(sun.xz, dir.xz) + DomeRanges.w))
//! sampled at |dome y|, scaled by BaseCloudColour.w, under four cloud layers.
//! The gradient coordinate |dome y| equals our |dir.y| (`FUN_008bca20` builds a unit dome). The gradient
//! scale BaseCloudColour.w is a squared environment-state value (`FUN_008b27e0`, docs/decomp.md "Sky, sun
//! and shadows") whose keyframe field is not identified, so it is 1 here. Provisional (not decoded): gradient B (only SkyColorGradient is named, so it serves
//! both sides), how Opacity / Feathering map to the cloud threshold, the cloud layers' dome mapping. The two HDR panoramas in the bundle
//! (`…3f35`, `…3f39`) are a sunrise capture that no keyframe references; they are not used.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::math::Vec3;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use mw_assets::AssetStore;
use mw_assets::environment::{EnvironmentKeyframe, gradient_at};
use mw_bundle::ResourceId;

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
    // The cloud textures are BC3_UNORM (not _SRGB) and NewSky multiplies the sample straight into its HDR
    // sum, so the stored values are used as they are (no gamma decode).
    let unorm = |c: u8| c as f32 / 255.0;
    let px = rgba.as_chunks::<4>().0.iter().map(|c| [unorm(c[0]), unorm(c[1]), unorm(c[2]), unorm(c[3])]).collect();
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
    /// NewSky's `GradientTextureB` (away from the sun).
    gradient_b: Vec<(f32, [f32; 3])>,
    /// The cloud layers: the keyframe's texture and the one of the keyframe it is blending towards.
    clouds: Option<Panorama>,
    clouds_b: Option<Panorama>,
    /// Towards the sky's sun (`SunMoonPath::sky_sun_direction`).
    sun: Vec3,
    /// The key light's colour × sin(elevation) / π: what lights the model ground (see [`Sky::ground`]).
    key_on_ground: [f32; 3],
    /// The cosine-weighted mean of the sky over the upper hemisphere (filled by [`Sky::new`]).
    sky_up: [f32; 3],
}

/// Albedo of the ground the irradiance and reflection cubes see below the horizon. The game convolves its
/// irradiance cube from a cube rendered at run time (exe program `0x01004eb8`: a normalised cosine-power
/// average of 32 taps), which shows the lit world below the horizon; the world is not rendered into our
/// cubes, so a flat ground of this albedo, lit by the key light and the sky, stands in for it. Model value,
/// chosen against the live `IrradianceQuadric`'s downward value at 14:13 (research log 2026-10-14).
const GROUND_ALBEDO: f32 = 0.1;

impl Sky {
    /// `sun`: towards the sky's sun; `key_dir`: towards the key light.
    pub fn new(store: &AssetStore, key: &EnvironmentKeyframe, sun: Vec3, key_dir: Vec3) -> Self {
        let clouds = key.clouds.texture.and_then(|t| load_clouds(store, t));
        let clouds_b = key
            .clouds
            .texture_b
            .filter(|t| key.clouds.crossfade > 0.0 && Some(*t) != key.clouds.texture)
            .and_then(|t| load_clouds(store, t));
        let lift = key_dir.normalize().y.max(0.0) * std::f32::consts::FRAC_1_PI;
        let mut sky = Self {
            key: key.clone(),
            gradient_b: key.sky_gradient_b(),
            clouds,
            clouds_b,
            sun: sun.normalize(),
            key_on_ground: key.key_light.map(|c| c * lift),
            sky_up: [0.0; 3],
        };
        sky.sky_up = sky.upper_hemisphere_mean();
        sky
    }

    /// The `NewSky` pixel program (`mwtool shader-asm NewSky`, the main-view variant) above the horizon:
    /// gradient = lerp(B, A, saturate(dot(dir.xz, sun.xz) + DomeRanges.w)) sampled at |dir.y|, ×
    /// BaseCloudColour.w, plus the glow pow(saturate(dir · sun), SunColour.w) × SunColour.rgb; then each cloud
    /// layer is blended over it by saturate((alpha − threshold) × CloudParamsPS2.y) towards cloud.rgb ×
    /// BaseCloudColour.rgb. The constants as the live dumps show them (research log 2026-10-14):
    /// `DomeRanges.w` = `Sky.2ec8df82`, `BaseCloudColour` = (`Clouds.LightColour`, `Sky.6acb8a54`²),
    /// `SunColour` = (`SunColour.rgb` × `SunColour.w`, `Sky.2ec8df83`), `CloudParamsPS` = (t, 1 − t) for the
    /// two keyframes' layers, `CloudParamsPS2.y` = `BaseCloudIntensity`. Cloud coordinates: [`cloud_uv`].
    fn above(&self, d: Vec3, clouds: bool) -> [f32; 3] {
        let k = &self.key;
        let y = d.y.abs();
        let (a, b) = (gradient_at(&k.sky_gradient, y), gradient_at(&self.gradient_b, y));
        let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z).try_normalize().unwrap_or(Vec3::ZERO);
        let w = (flat(d).dot(flat(self.sun)) + k.gradient_sun_offset).clamp(0.0, 1.0);
        let scale = k.sky_scale * k.sky_scale;
        let glow = d.dot(self.sun).clamp(0.0, 1.0).powf(k.sun_glow_exponent.max(1.0));
        let mut c = [0, 1, 2].map(|i| (b[i] + (a[i] - b[i]) * w) * scale + k.sun_colour[i] * k.sun_colour[3] * glow);
        if clouds {
            let (u, v) = cloud_uv(d, self.sun);
            let layers =
                [(self.clouds.as_ref(), k.clouds.crossfade), (self.clouds_b.as_ref(), 1.0 - k.clouds.crossfade)];
            for (layer, threshold) in layers {
                let Some(cl) = layer else { continue };
                let t = cl.sample(u, v);
                let cover = ((t[3] - threshold) * k.clouds.intensity).clamp(0.0, 1.0);
                for i in 0..3 {
                    c[i] += (t[i] * k.clouds.light[i] - c[i]) * cover;
                }
            }
        }
        c
    }

    /// Below the horizon: a flat ground of [`GROUND_ALBEDO`] lit by the key light (unshadowed) and the sky
    /// (model; the game's cubes show the rendered world there).
    fn ground(&self) -> [f32; 3] {
        [0, 1, 2].map(|i| GROUND_ALBEDO * (self.key_on_ground[i] + self.sky_up[i]))
    }

    pub fn radiance(&self, d: Vec3, clouds: bool) -> [f32; 3] {
        if d.y < 0.0 { self.ground() } else { self.above(d, clouds) }
    }

    /// Cosine-weighted mean of the sky (with clouds) over the upper hemisphere.
    fn upper_hemisphere_mean(&self) -> [f32; 3] {
        let n = 4096;
        let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
        let mut sum = [0.0f32; 3];
        for i in 0..n {
            let u = (i as f32 + 0.5) / n as f32;
            let (sin, cos) = (u.sqrt(), (1.0 - u).sqrt());
            let phi = i as f32 * golden;
            let c = self.above(Vec3::new(sin * phi.cos(), cos, sin * phi.sin()), true);
            for k in 0..3 {
                sum[k] += c[k];
            }
        }
        sum.map(|v| v / n as f32)
    }

    /// The irradiance (normalised cosine-weighted mean radiance) around a direction, for comparisons with the
    /// original's `IrradianceQuadric` (`mwtool environment`).
    pub fn irradiance(&self, n: Vec3) -> [f32; 3] {
        let (t, b) = frame(n);
        let count = 8192;
        let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
        let mut sum = [0.0f32; 3];
        for i in 0..count {
            let u = (i as f32 + 0.5) / count as f32;
            let (sin, cos) = (u.sqrt(), (1.0 - u).sqrt());
            let phi = i as f32 * golden;
            let d = (t * (sin * phi.cos()) + b * (sin * phi.sin()) + n * cos).normalize();
            let c = self.radiance(d, true);
            for k in 0..3 {
                sum[k] += c[k];
            }
        }
        sum.map(|v| v / count as f32)
    }
}

/// Where a direction samples the cloud texture (2048 × 512): u by azimuth relative to the sun, v by elevation.
/// Model: the game's dome carries its own cloud coordinates (an OBJ parsed in `FUN_008bca20`, scaled and
/// scrolled by `BaseCloud*ParamsVS`), not decoded.
fn cloud_uv(d: Vec3, sun: Vec3) -> (f32, f32) {
    let elevation = d.y.clamp(0.0, 1.0).asin();
    let u = (d.x.atan2(d.z) - sun.x.atan2(sun.z)) / std::f32::consts::TAU + 0.5;
    (u, 1.0 - elevation / std::f32::consts::FRAC_PI_2)
}

/// A cube map (`size` per face, `mips` levels, RGBA16F) filled per texel direction and mip level.
fn cube_image(size: usize, mips: usize, f: impl Fn(Vec3, usize) -> [f32; 3] + Sync) -> Image {
    match bake_pool() {
        Some(pool) => pool.install(|| cube_image_in_pool(size, mips, &f)),
        None => cube_image_in_pool(size, mips, f),
    }
}

/// The environment bakes run on a small pool of their own: on the global rayon pool a bake (a timeline change
/// while driving) took every core for its duration and starved the main and render threads (research log
/// 2026-10-09, frame drops). The texels are the same; only the bake takes longer. `MW_BAKE_THREADS=n` sets the
/// pool size, 0 = the global pool (A/B).
fn bake_pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let n = std::env::var("MW_BAKE_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
        // No pool where threads can't be made (a browser): the global pool, which then runs on the caller.
        (n > 0)
            .then(|| {
                rayon::ThreadPoolBuilder::new()
                    .num_threads(n)
                    .thread_name(|i| format!("environment bake {i}"))
                    .build()
                    .ok()
            })
            .flatten()
    })
    .as_ref()
}

fn cube_image_in_pool(size: usize, mips: usize, f: impl Fn(Vec3, usize) -> [f32; 3] + Sync) -> Image {
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
        // Blinn-Phong power of the game's key-light model at this roughness: 2^(11.0001 - 11 r).
        let r = (mip as f32 / 5.5).min(1.0);
        let power = (11.0001 - 11.0 * r).exp2() * 0.25;
        lobe(sky, d, power.max(1.0), 64 << mip.min(3))
    })
}

/// The irradiance cube of the lighting pass (`cubeIrradianceSampler`). The game filters it on the GPU from a
/// cube rendered at run time (exe program `0x01004eb8`: Σ L × cos^p / Σ cos^p over 32 taps around the normal,
/// stored as sqrt(x / 4); the lighting pass squares it and multiplies by 4). Here: the normalised cosine
/// average of [`Sky::radiance`] (the sky above, the model ground below) around each texel's direction. The
/// keyframe's six-direction `LightRig` is not in it: the live `IrradianceQuadric` is 5× the rig upwards and
/// follows the sky (research log 2026-10-14). `MW_RIG_AMBIENT=1` builds the old rig + sky-average cube (A/B).
pub fn irradiance_cube(sky: &Sky, key: &EnvironmentKeyframe, to_key: Vec3) -> Image {
    if std::env::var_os("MW_RIG_AMBIENT").is_some() {
        return rig_irradiance_cube(key, to_key);
    }
    // The radiance on a latitude-longitude grid, then the cosine-weighted mean per texel.
    let (w, h) = (96usize, 48usize);
    let mut samples = Vec::with_capacity(w * h);
    for j in 0..h {
        let theta = (j as f32 + 0.5) / h as f32 * std::f32::consts::PI;
        let (st, ct) = theta.sin_cos();
        for i in 0..w {
            let phi = (i as f32 + 0.5) / w as f32 * std::f32::consts::TAU;
            let d = Vec3::new(st * phi.cos(), ct, st * phi.sin());
            samples.push((d, st, sky.radiance(d, true)));
        }
    }
    cube_image(16, 1, move |n, _| {
        let mut sum = [0.0f32; 3];
        let mut weight = 0.0;
        for (d, area, c) in &samples {
            let wgt = n.dot(*d).max(0.0) * area;
            weight += wgt;
            for k in 0..3 {
                sum[k] += c[k] * wgt;
            }
        }
        sum.map(|v| v / weight.max(1e-6))
    })
}

/// The earlier stand-in (`MW_RIG_AMBIENT=1`): the keyframe's six-direction light rig oriented to the key light's
/// azimuth, weighted by the squared cosine to each axis, × `AmbientIrradianceScale`, plus the upper hemisphere
/// at `SkyReflectionAverageColour`, cosine-weighted.
fn rig_irradiance_cube(key: &EnvironmentKeyframe, sun: Vec3) -> Image {
    let fwd = Vec3::new(sun.x, 0.0, sun.z).try_normalize().unwrap_or(Vec3::Z);
    let side = Vec3::Y.cross(fwd);
    let r = &key.ambient;
    let axes = [(fwd, r.key), (-fwd, r.shadow), (Vec3::Y, r.up), (-Vec3::Y, r.down), (side, r.left), (-side, r.right)];
    let sky = key.sky_reflection_colour;
    cube_image(16, 1, |d, _| {
        let mut c = [0.0f32; 3];
        for (axis, fill) in axes {
            let w = d.dot(axis).max(0.0).powi(2);
            for i in 0..3 {
                c[i] += fill[i] * w * r.scale;
            }
        }
        let w = 0.5 * (1.0 + d.y);
        for i in 0..3 {
            c[i] += sky[i] * w;
        }
        c
    })
}
