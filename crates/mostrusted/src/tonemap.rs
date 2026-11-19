//! The game's auto exposure and tonemap in place of Bevy's tonemapping (`tonemap.wgsl`).
//!
//! Three full-screen passes in the `Core3d` post-process set, before Bevy's tonemapping (which the camera
//! turns off with `Tonemapping::None`): meter the displayed luma onto a small grid, adapt a 1×1 exposure
//! state (ping-ponged between two textures so it carries over frames), then tonemap the HDR target with
//! the adapted exposure. Settings come from the environment keyframe (`mw_assets::environment::
//! CameraExposure`) through the [`GameTonemap`] resource. Between the adaptation and the tonemap the game's
//! bloom chain runs (`bloom.wgsl`): three downsampled RGBM levels, blurred and combined coarse to fine, added
//! in the composite. `MW_NO_BLOOM=1` turns it off (the game's bloom-off path: B = (1, 1, 1, 0)).
//!
//! The composite writes the gamma-encoded frame to an 8-bit target; the final pass (the exe's `0x01017050`,
//! the variant the original runs in free roam: depth of field + tint) then blends in the depth-of-field
//! image (`MW_NO_DOF=1` off), applies the brightness / contrast of `ColourCubeScalesOffsets` (the sun-facing
//! boost, `MW_NO_SUN_CONTRAST=1` off) and the colour cube (decomp.md "Post-processing").

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, texture_3d, texture_depth_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::{FallbackImage, GpuImage};
use bevy::render::view::ViewTarget;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

/// Edge of the metering grid; must match `METER` in tonemap.wgsl.
const METER_SIZE: u32 = 32;

/// Tonemap settings from the environment keyframe, plus the frame time for adaptation.
#[derive(Resource, Clone, ExtractResource)]
pub struct GameTonemap {
    pub settings: mw_assets::environment::CameraExposure,
    /// The PostFX keyframe (vignette).
    pub post_fx: mw_assets::environment::PostFxKeyframe,
    /// Seconds since the last frame.
    pub dt: f32,
    /// Restart adaptation from the middle of the exposure range (set for the first frame).
    pub reset: bool,
    /// The keyframe's colour grade ([`colour_cube_image`]).
    pub colour_cube: Option<Handle<Image>>,
}

/// A `ColourCube` as a 3-D texture (raw gamma-encoded values: Rgba8Unorm, not sRGB), laid out as the
/// game's TintBlend volume: x = input red, y = green, z = blue (`mw_formats::ColourCube`).
pub fn colour_cube_image(cube: &mw_formats::ColourCube) -> Image {
    let n = cube.size;
    let mut data = Vec::with_capacity(n * n * n * 4);
    // Texture order is x fastest (= red), then y (green), then z (blue).
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                data.extend(cube.at(r, g, b));
                data.push(255);
            }
        }
    }
    let size = Extent3d { width: n as u32, height: n as u32, depth_or_array_layers: n as u32 };
    let mut image = Image::new(
        size,
        TextureDimension::D3,
        data,
        TextureFormat::Rgba8Unorm,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = bevy::image::ImageSampler::linear();
    image
}

/// The game's TintBlend (`FUN_008e0930`): a weighted sum of cubes (the kernels are not in the dump, so the
/// rounding is ours). A missing cube stands for the base slot, the identity in free roam. Weights are used
/// as given (the caller passes ones that sum to 1, as `FUN_008c9e40` makes them).
pub fn blend_colour_cubes(cubes: &[(Option<mw_formats::ColourCube>, f32)]) -> Option<Image> {
    let n = cubes.iter().find_map(|(c, _)| c.as_ref().map(|c| c.size))?;
    let identity = |i: usize| (i * 255 / (n - 1).max(1)) as f32;
    let mut sum = vec![[0.0f32; 3]; n * n * n];
    for (cube, w) in cubes {
        let cube = cube.as_ref().filter(|c| c.size == n);
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    let v = cube.map_or([identity(r), identity(g), identity(b)], |c| c.at(r, g, b).map(f32::from));
                    let o = &mut sum[r + n * (g + n * b)];
                    for k in 0..3 {
                        o[k] += v[k] * w;
                    }
                }
            }
        }
    }
    let bgr = sum.iter().flat_map(|c| [2, 1, 0].map(|k| c[k].round().clamp(0.0, 255.0) as u8)).collect();
    Some(colour_cube_image(&mw_formats::ColourCube { size: n, bgr }))
}

/// What the post chain needs of the view besides the environment: the key light (towards the sun) for the
/// sun-facing grade boost. Updated every frame from the key light entity.
#[derive(Resource, Clone, Default, ExtractResource)]
pub struct PostView {
    pub to_sun: Vec3,
    /// A sequence's `PostFxOverrideSequenceItem` (`sequence_fx.rs`): added to the final pass's brightness b and
    /// contrast c. `FUN_008ecf80` writes the PostFX state's +0x1660 = property 0 × 0.6 (`.rdata` 0x010219a4) and
    /// +0x1664 = property 1 × (0.8 above 0, else 0.4; 0x010219a0 / 0x00fae200), the terms `FUN_008c9e40` adds.
    pub override_grade: Vec2,
    /// The sequences' PostFX state slots in order (`sequence_picture.rs`): vignette and bloom blended over the
    /// base (`FUN_008e4b30`, `FUN_008e9610`).
    pub post_fx_slots: Vec<crate::sequence_picture::SlotBlend>,
    /// The states' colour cubes (TintBlend's two state slots, `FUN_008e4a90`) as one cube, and their summed
    /// weight: the TintBlend scales the weights to sum 1 when they exceed it (`FUN_008c9e40`), so with the
    /// environment pair's 1 the grade is (env + w · state) / (1 + w).
    pub state_cube: Option<Handle<Image>>,
    pub state_cube_weight: f32,
}

fn update_post_view(mut view: ResMut<PostView>, key_light: Query<&GlobalTransform, With<crate::weather::KeyLight>>) {
    if let Some(t) = key_light.iter().next() {
        view.to_sun = -t.forward().as_vec3();
    }
}

#[derive(ShaderType, Clone, Copy, Default)]
struct TonemapUniform {
    camera: Vec4,
    exposure: Vec4,
    setting: Vec4,
    flags: Vec4,
    /// Vignette (decomp.md "Post-processing"): A = renderer +0xa990 (`kVignetteOuterRgbPlusAdd`),
    /// D = +0xa980, and the vertex shader's (centre, scale × k) and angle.
    vignette_a: Vec4,
    vignette_d: Vec4,
    vignette_centre_scale: Vec4,
    vignette_angle: Vec4,
    /// Bloom B = renderer slot 0x9d (`GenericVector4_B`): `BloomColour`.rgb, intensity in w (0 = off).
    bloom: Vec4,
    /// The final pass: x = contrast c, y = brightness b (`ColourCubeScalesOffsets` = (c·15/16, b + 0.5 −
    /// c/2 + 1/32)), z = 1 with depth of field.
    grade: Vec4,
    /// Depth of field: x = the camera's near plane (Bevy's reversed depth = near / view depth), y = the
    /// circle of confusion at infinity x0, z = the focus distance (CoC = max(x0 (1 − focus / z), 0)).
    dof: Vec4,
}

/// One bloom pass's constants (bloom.wgsl): x = `kColorAddWeight.z`, y = `.w`, z = the exposure bias.
#[derive(ShaderType, Clone, Copy, Default)]
struct BloomUniform {
    weights: Vec4,
}

/// The lower bound `FUN_008e6f40` clamps the three level weights to (`DAT_00f9d474`, `.rdata`).
const BLOOM_MIN_WEIGHT: f32 = 0.01;

/// Bloom level sizes. The original's levels are the renderer's targets 0x50, 0x51, 0x52 (`FUN_008eaaf0`
/// passes R + 0xa8 + 0x140…0x148 to `FUN_008e6f40`): 320 × 176, 80 × 44 and 20 × 11 whatever the screen
/// size (live reads 2026-10-14: snap1 at 1920 × 1080 and a probe dump at 1280 × 720 hold the same three),
/// so the bloom's reach is a fixed fraction of the frame. `MW_BLOOM_DIVISORS=4,8,16` gives screen fractions
/// instead, for A/B (the first port's levels).
const BLOOM_LEVELS: [(u32, u32); 3] = [(320, 176), (80, 44), (20, 11)];

fn bloom_sizes((w, h): (u32, u32)) -> [(u32, u32); 3] {
    if let Some(d) = std::env::var("MW_BLOOM_DIVISORS").ok().and_then(|v| {
        let d: Vec<u32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        (d.len() == 3 && d.iter().all(|&x| x > 0)).then(|| [d[0], d[1], d[2]])
    }) {
        return d.map(|d| ((w / d).max(1), (h / d).max(1)));
    }
    BLOOM_LEVELS
}

/// The camera's depth-of-field block (camera +0x130…+0x13c, copied to renderer +0xae10 by `FUN_008c9e40`
/// when the camera's DOF mode +0x170 is 1 or 2, as in free roam): 73.448, focus 90 m, 102.83 (× 0.001 =
/// the lens's focal length in metres). Read live (snap1 and all 20 `ref/live1` dumps); which data or code
/// sets them was not found, so they are measured constants.
const DOF_APERTURE: f32 = 73.448;
const DOF_FOCUS: f32 = 90.0;
const DOF_FOCAL: f32 = 102.83 * 0.001;

/// The CoC at infinity, `FUN_008c9e40`: A / 2 × f / (B − f) (0.0420; live K = (12.602, −12.56) gives
/// K.x + K.y = 0.042).
fn dof_coc_infinity() -> f32 {
    DOF_APERTURE * 0.5 * DOF_FOCAL / (DOF_FOCUS - DOF_FOCAL)
}

fn dof_enabled() -> bool {
    std::env::var_os("MW_NO_DOF").is_none()
}

/// Contrast and brightness of the final pass (`FUN_008c9e40` → `FUN_008eae00` → `FUN_008e8c40`): the
/// options' 50 % give c = 1, b = 0; the environment adds `contrast` / `brightness` × (1 − t) with t = (1 +
/// forward · the flat to-sun heading) / 2, so the boost is full looking away from the sun and gone looking
/// into it. The PostFX state's +0x1660 / +0x1664 terms (0 in free roam, live) are left out.
fn sun_grade(s: &mw_assets::environment::CameraExposure, forward: Vec3, to_sun: Vec3) -> (f32, f32) {
    if std::env::var_os("MW_NO_SUN_CONTRAST").is_some() {
        return (1.0, 0.0);
    }
    let heading = Vec3::new(to_sun.x, 0.001, to_sun.z).normalize_or_zero();
    let away = 1.0 - 0.5 * (1.0 + heading.dot(forward));
    (1.0 + s.contrast * away, s.brightness * away)
}

/// `kColorAddWeight.z` / `.w` of the three add passes, finest level first (`FUN_008e6f40`, decomp.md
/// "Level combine"): C3 = blur(L3); C2 = (b·blur(L2) + a·C3) / (a + b); C1 = (c·blur(L1) + (a + b)·C2) / Σ.
fn bloom_add_weights(w: [f32; 3]) -> [[f32; 2]; 3] {
    let [c, b, a] = w.map(|x| x.max(BLOOM_MIN_WEIGHT));
    [[c / (a + b + c), (a + b) / (a + b + c)], [b / (a + b), a / (a + b)], [1.0, 0.0]]
}

fn bloom_enabled() -> bool {
    std::env::var_os("MW_NO_BLOOM").is_none()
}

/// The vignette's falloff gain k = 1 + 500 (0.5 + 0.5 `Sharpness`)^16 (`FUN_008e8c40`: `pow` with the
/// `.rdata` double 16 at 0x01021988 and the float 500 at 0x01021980): 1.82445 for the default keyframe's
/// 0.34, the live 1.8245.
fn vignette_k(sharpness: f32) -> f32 {
    1.0 + 500.0 * (0.5 + 0.5 * sharpness).powi(16)
}

/// The vignette constants the original's pass reads, from the PostFX keyframe (`FUN_008e8c40`, the block at
/// post object +0xc00 = renderer +0xa980): D = inner × s, A = (outer − inner) × s with s the final pass's
/// contrast c (the live s = 1.0025…1.0385 is c, decomp.md "Post-processing"), k in their w.
fn vignette_constants(v: &mw_assets::environment::Vignette, s: f32) -> [Vec4; 4] {
    let k = vignette_k(v.sharpness);
    let inner = Vec3::from(v.inner);
    let a = Vec3::from(v.outer) - inner;
    [
        (a * s).extend(1.0 - k),
        (inner * s).extend(k),
        Vec4::new(v.centre[0], v.centre[1], v.scale[0] * k, v.scale[1] * k),
        // +0xc20.x (kVignetteAngle) = the Vignette block's second word, 0 in the default keyframe.
        Vec4::ZERO,
    ]
}

pub struct GameTonemapPlugin;

/// The game tonemap pass in `Core3d` (passes drawing over the tonemapped image run after it).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct GameTonemapPass;

impl Plugin for GameTonemapPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "tonemap.wgsl");
        embedded_asset!(app, "bloom.wgsl");
        embedded_asset!(app, "post.wgsl");
        embedded_asset!(app, "flash_probe.wgsl");
        app.add_plugins((
            ExtractResourcePlugin::<GameTonemap>::default(),
            ExtractResourcePlugin::<PostView>::default(),
        ))
        .init_resource::<PostView>()
        .add_systems(Last, (update_time, update_post_view));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_resource::<TonemapBuffer>()
            .init_resource::<BloomBuffers>()
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(Render, prepare_buffer.in_set(RenderSystems::Prepare))
            .add_systems(
                Core3d,
                game_tonemap.before(tonemapping).in_set(Core3dSystems::PostProcess).in_set(GameTonemapPass),
            );
    }
}

/// Frame time for the adaptation step; the reset flag lasts one frame.
fn update_time(time: Res<Time>, tonemap: Option<ResMut<GameTonemap>>, mut frames: Local<u32>) {
    let Some(mut tonemap) = tonemap else { return };
    // The flag must survive until the render world has extracted it once.
    *frames += 1;
    tonemap.dt = time.delta_secs();
    if *frames > 2 {
        tonemap.reset = false;
    }
}

#[derive(Resource, Default)]
struct TonemapBuffer(UniformBuffer<TonemapUniform>);

/// The add passes' constants, finest level first (the downsample and horizontal passes read only the bias).
#[derive(Resource, Default)]
struct BloomBuffers([UniformBuffer<BloomUniform>; 3]);

/// Post render targets for one screen size: bloom RGBM levels (Rgba8Unorm, as the game's encode), the
/// horizontal blur and the combined result of each level (float, HDR game units), finest first; the
/// composite's gamma-encoded frame (8 bit) and the half-resolution depth-of-field image (the renderer's
/// 960 × 540 targets 0x1f…0x21 at 1080p).
struct BloomTargets {
    size: (u32, u32),
    levels: [TextureView; 3],
    blurred: [TextureView; 3],
    combined: [TextureView; 3],
    composite: TextureView,
    dof: TextureView,
}

impl BloomTargets {
    fn new(device: &RenderDevice, (w, h): (u32, u32)) -> Self {
        let sizes = bloom_sizes((w, h));
        let per_level =
            |label: &'static str, format: TextureFormat| [0, 1, 2].map(|i| target_2d(device, label, sizes[i], format));
        Self {
            size: (w, h),
            levels: per_level("bloom_level", TextureFormat::Rgba8Unorm),
            blurred: per_level("bloom_blur_h", TextureFormat::Rgba16Float),
            combined: per_level("bloom_combined", TextureFormat::Rgba16Float),
            composite: target_2d(device, "game_composite", (w, h), TextureFormat::Rgba8Unorm),
            dof: target_2d(device, "game_dof", ((w / 2).max(1), (h / 2).max(1)), TextureFormat::Rgba16Float),
        }
    }
}

#[derive(Resource)]
struct TonemapPipelines {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    bloom_layout: BindGroupLayoutDescriptor,
    bloom_down_hdr: CachedRenderPipelineId,
    bloom_down_rgbm: CachedRenderPipelineId,
    bloom_blur_h: CachedRenderPipelineId,
    bloom_add: CachedRenderPipelineId,
    meter: CachedRenderPipelineId,
    adapt: CachedRenderPipelineId,
    tonemap: CachedRenderPipelineId,
    /// The final pass and the depth-of-field downsample (layout `post_layout`).
    post_layout: BindGroupLayoutDescriptor,
    dof_down: CachedRenderPipelineId,
    final_pass: CachedRenderPipelineId,
    /// Bound in place of the prepass depth when a view has none.
    depth_fallback: TextureView,
    meter_view: TextureView,
    /// The `MW_FLASH_LOG` probe (flash_probe.rs): layout and pipeline.
    probe_layout: BindGroupLayoutDescriptor,
    probe: CachedRenderPipelineId,
    /// Ping-pong exposure states; `states[frame % 2]` is written each frame.
    states: [TextureView; 2],
}

fn render_target(device: &RenderDevice, label: &'static str, size: u32, format: TextureFormat) -> TextureView {
    target_2d(device, label, (size, size), format)
}

fn target_2d(device: &RenderDevice, label: &'static str, (w, h): (u32, u32), format: TextureFormat) -> TextureView {
    device
        .create_texture(&TextureDescriptor {
            label: Some(label),
            size: Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&TextureViewDescriptor::default())
}

fn init_pipelines(
    mut commands: Commands,
    device: Res<RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    asset_server: Res<AssetServer>,
    cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "game_tonemap_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                uniform_buffer::<TonemapUniform>(false),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_2d(TextureSampleType::Float { filterable: false }),
                texture_2d(TextureSampleType::Float { filterable: false }),
                texture_3d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_2d(TextureSampleType::Float { filterable: true }),
            ),
        ),
    );
    let bloom_layout = BindGroupLayoutDescriptor::new(
        "game_bloom_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                uniform_buffer::<BloomUniform>(false),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: false }),
            ),
        ),
    );
    let post_layout = BindGroupLayoutDescriptor::new(
        "game_post_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                uniform_buffer::<TonemapUniform>(false),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_depth_2d(),
                texture_3d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_3d(TextureSampleType::Float { filterable: true }),
            ),
        ),
    );
    let probe_layout = BindGroupLayoutDescriptor::new(
        "flash_probe_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: false }),
                texture_2d(TextureSampleType::Float { filterable: false }),
                texture_2d(TextureSampleType::Float { filterable: false }),
            ),
        ),
    );
    let probe_shader = load_embedded_asset!(asset_server.as_ref(), "flash_probe.wgsl");
    let shader = load_embedded_asset!(asset_server.as_ref(), "tonemap.wgsl");
    let bloom_shader = load_embedded_asset!(asset_server.as_ref(), "bloom.wgsl");
    let post_shader = load_embedded_asset!(asset_server.as_ref(), "post.wgsl");
    let queue = |label: &'static str,
                 layout: &BindGroupLayoutDescriptor,
                 shader: &Handle<Shader>,
                 entry: &'static str,
                 format: TextureFormat| {
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: shader.clone(),
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState { format, blend: None, write_mask: ColorWrites::ALL })],
                ..default()
            }),
            ..default()
        })
    };
    let pipeline = |label, entry, format| queue(label, &layout, &shader, entry, format);
    let bloom = |label, entry, format| queue(label, &bloom_layout, &bloom_shader, entry, format);
    let sampler = device.create_sampler(&SamplerDescriptor {
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        ..default()
    });
    commands.insert_resource(TonemapPipelines {
        bloom_down_hdr: bloom("game_bloom_down_hdr", "down_hdr", TextureFormat::Rgba8Unorm),
        bloom_down_rgbm: bloom("game_bloom_down_rgbm", "down_rgbm", TextureFormat::Rgba8Unorm),
        bloom_blur_h: bloom("game_bloom_blur_h", "blur_h", TextureFormat::Rgba16Float),
        bloom_add: bloom("game_bloom_add", "add", TextureFormat::Rgba16Float),
        bloom_layout,
        probe: queue("flash_probe", &probe_layout, &probe_shader, "probe", TextureFormat::Rgba32Float),
        probe_layout,
        meter: pipeline("game_tonemap_meter", "meter", TextureFormat::R32Float),
        adapt: pipeline("game_tonemap_adapt", "adapt", TextureFormat::Rgba32Float),
        tonemap: pipeline("game_tonemap", "tonemap", TextureFormat::Rgba8Unorm),
        dof_down: queue("game_dof_down", &post_layout, &post_shader, "dof_down", TextureFormat::Rgba16Float),
        final_pass: queue("game_final", &post_layout, &post_shader, "final_pass", TextureFormat::Rgba16Float),
        post_layout,
        depth_fallback: device
            .create_texture(&TextureDescriptor {
                label: Some("game_post_depth_fallback"),
                size: Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&TextureViewDescriptor::default()),
        layout,
        sampler,
        meter_view: render_target(&device, "game_tonemap_meter", METER_SIZE, TextureFormat::R32Float),
        states: [
            render_target(&device, "game_tonemap_state_a", 1, TextureFormat::Rgba32Float),
            render_target(&device, "game_tonemap_state_b", 1, TextureFormat::Rgba32Float),
        ],
    });
}

#[allow(clippy::too_many_arguments)]
fn prepare_buffer(
    tonemap: Option<Res<GameTonemap>>,
    post_view: Option<Res<PostView>>,
    images: Res<RenderAssets<GpuImage>>,
    views: Query<
        &bevy::render::view::ExtractedView,
        (With<ViewTarget>, Without<crate::reflection::PlanarReflectionCamera>),
    >,
    mut buffer: ResMut<TonemapBuffer>,
    mut bloom_buffers: ResMut<BloomBuffers>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let Some(t) = tonemap else { return };
    let s = &t.settings;
    // The main view: its forward for the sun-facing grade, its near plane for the depth of field (Bevy's
    // reversed infinite projection: clip_from_view.w_axis.z = near).
    let (forward, near) = views
        .iter()
        .next()
        .map_or((Vec3::NEG_Z, 0.1), |v| (v.world_from_view.forward().as_vec3(), v.clip_from_view.w_axis.z));
    let to_sun = post_view.as_ref().map_or(Vec3::Y, |p| p.to_sun);
    let (contrast, brightness) = sun_grade(s, forward, to_sun);
    let extra = post_view.as_ref().map_or(Vec2::ZERO, |p| p.override_grade);
    // The sequences' PostFX states over the base: the default keyframe's vignette and the environment's bloom
    // (`FUN_008ecf80`: slot by slot, `FUN_008ee430` / `FUN_008e9610` lerps by the slot's weight).
    let mut vignette = t.post_fx.vignette;
    let mut bloom = crate::sequence_picture::BloomWords {
        colour: s.bloom_colour,
        intensity: s.bloom_intensity,
        weights: s.bloom_weights,
    };
    for slot in post_view.iter().flat_map(|p| &p.post_fx_slots) {
        if let Some((w, v)) = &slot.vignette {
            vignette = crate::sequence_picture::lerp_vignette(&vignette, v, *w);
        }
        if let Some((w, b)) = &slot.bloom {
            bloom = bloom.lerp(b, *w);
        }
    }
    let state_cube = post_view
        .as_ref()
        .filter(|p| p.state_cube.as_ref().is_some_and(|h| images.get(h).is_some()))
        .map_or(0.0, |p| p.state_cube_weight);
    let (contrast, brightness) = (contrast + extra.y, brightness + extra.x);
    // MW_POST_LOG=1: the final pass's grade and the depth-of-field constants, about once a second.
    if std::env::var_os("MW_POST_LOG").is_some() {
        static FRAMES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed).is_multiple_of(60) {
            info!(
                "post: contrast {contrast:.4} brightness {brightness:.4} (env {:.4} / {:.4}), near {near:.3}, \
                 CoC at infinity {:.4}, focus {DOF_FOCUS} m",
                s.contrast,
                s.brightness,
                dof_coc_infinity()
            );
        }
    }
    let vignette_on = std::env::var_os("MW_NO_VIGNETTE").is_none() && vignette.scale != [0.0; 2];
    let [vignette_a, vignette_d, vignette_centre_scale, vignette_angle] = if vignette_on {
        vignette_constants(&vignette, contrast)
    } else {
        // Weight 0 everywhere and factor 1: A = 0, D = 1.
        [Vec4::new(0.0, 0.0, 0.0, -1.0), Vec4::ONE, Vec4::new(0.5, 0.5, 0.0, 0.0), Vec4::ZERO]
    };
    buffer.0.set(TonemapUniform {
        vignette_a,
        vignette_d,
        vignette_centre_scale,
        vignette_angle,
        // B with bloom on (`FUN_0089d5c0`): the environment keyframe's colour and intensity in free roam
        // (decomp.md "Where the block comes from"); off: (1, 1, 1, 0).
        bloom: if bloom_enabled() {
            Vec3::from(bloom.colour).extend(bloom.intensity)
        } else {
            Vec4::new(1.0, 1.0, 1.0, 0.0)
        },
        camera: Vec4::new(s.min, s.max, s.bias, 0.0),
        exposure: Vec4::new(0.0, s.tolerance, s.speed, t.dt),
        setting: Vec4::new(s.target_high, s.target_low, s.luminance_high, s.luminance_low),
        flags: Vec4::new(f32::from(u8::from(t.reset)), f32::from(u8::from(t.colour_cube.is_some())), state_cube, 0.0),
        grade: Vec4::new(contrast, brightness, f32::from(u8::from(dof_enabled())), 0.0),
        dof: Vec4::new(near, dof_coc_infinity(), DOF_FOCUS, 0.0),
    });
    buffer.0.write_buffer(&device, &queue);
    for (b, [z, w]) in bloom_buffers.0.iter_mut().zip(bloom_add_weights(bloom.weights)) {
        b.set(BloomUniform { weights: Vec4::new(z, w, s.bias, 0.0) });
        b.write_buffer(&device, &queue);
    }
}

#[allow(clippy::too_many_arguments)]
fn game_tonemap(
    // Not the planar reflection: it keeps HDR game units for the road shader to sample.
    view: ViewQuery<(&ViewTarget, Option<&ViewPrepassTextures>), Without<crate::reflection::PlanarReflectionCamera>>,
    pipelines: Option<Res<TonemapPipelines>>,
    cache: Res<PipelineCache>,
    buffer: Res<TonemapBuffer>,
    bloom_buffers: Res<BloomBuffers>,
    settings: Option<Res<GameTonemap>>,
    post_view: Option<Res<PostView>>,
    images: Res<RenderAssets<GpuImage>>,
    fallback: Res<FallbackImage>,
    mut frame: Local<usize>,
    mut bloom_targets: Local<Option<BloomTargets>>,
    flash_probe: Option<Res<crate::flash_probe::FlashProbe>>,
    mut ctx: RenderContext,
) {
    let (target, prepass) = view.into_inner();
    let Some(p) = pipelines else { return };
    let (Some(meter), Some(adapt), Some(tonemap), Some(dof_down), Some(final_pass), Some(uniform)) = (
        cache.get_render_pipeline(p.meter),
        cache.get_render_pipeline(p.adapt),
        cache.get_render_pipeline(p.tonemap),
        cache.get_render_pipeline(p.dof_down),
        cache.get_render_pipeline(p.final_pass),
        buffer.0.binding(),
    ) else {
        return;
    };
    let post_layout = cache.get_bind_group_layout(&p.post_layout);
    let depth = prepass.and_then(|p| p.depth_view()).unwrap_or(&p.depth_fallback);
    let layout = cache.get_bind_group_layout(&p.layout);
    let bloom_layout = cache.get_bind_group_layout(&p.bloom_layout);
    let bloom_pipelines = (
        cache.get_render_pipeline(p.bloom_down_hdr),
        cache.get_render_pipeline(p.bloom_down_rgbm),
        cache.get_render_pipeline(p.bloom_blur_h),
        cache.get_render_pipeline(p.bloom_add),
    );
    let bloom_uniforms = [0, 1, 2].map(|i| bloom_buffers.0[i].binding());
    // Missing cube: the fallback is bound and the shader skips the grade (flags.y = 0).
    let cube = settings.as_ref().and_then(|s| s.colour_cube.as_ref()).and_then(|h| images.get(h));
    let (cube_view, cube_sampler) = match cube {
        Some(c) => (&c.texture_view, &c.sampler),
        None => (&fallback.d3.texture_view, &fallback.d3.sampler),
    };
    // The sequences' PostFX state cube (`sequence_picture.rs`; weight in the uniform's flags.z).
    let state_cube = post_view.as_ref().and_then(|p| p.state_cube.as_ref()).and_then(|h| images.get(h));
    let state_cube_view = state_cube.map_or(&fallback.d3.texture_view, |c| &c.texture_view);
    let (read, write) = (&p.states[(*frame + 1) % 2], &p.states[*frame % 2]);
    *frame += 1;
    let device = ctx.render_device().clone();
    let main = target.main_texture();
    let size = (main.width(), main.height());
    if bloom_targets.as_ref().is_none_or(|t| t.size != size) {
        *bloom_targets = Some(BloomTargets::new(&device, size));
    }
    let bt = bloom_targets.as_ref().expect("just created");
    let bloom_on = bloom_enabled();
    let bloom_view = if bloom_on { &bt.combined[0] } else { &fallback.d2.texture_view };
    let bind = |hdr: &TextureView, state: &TextureView| {
        device.create_bind_group(
            "game_tonemap_bind_group",
            &layout,
            &BindGroupEntries::sequential((
                uniform.clone(),
                hdr,
                &p.sampler,
                &p.meter_view,
                state,
                cube_view,
                cube_sampler,
                bloom_view,
            )),
        )
    };
    let mut run = |target_view: &TextureView, pipeline: &RenderPipeline, group: &BindGroup| {
        let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
            label: Some("game_tonemap"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target_view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations { load: LoadOp::Clear(Default::default()), store: StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    };
    // The meter's bind group also binds the meter texture it renders to; the shader never reads it there,
    // but a texture can't be both, so the adapt state stands in.
    let meter_group = device.create_bind_group(
        "game_tonemap_meter_bind_group",
        &layout,
        &BindGroupEntries::sequential((
            uniform.clone(),
            target.main_texture_view(),
            &p.sampler,
            read,
            read,
            cube_view,
            cube_sampler,
            bloom_view,
        )),
    );
    run(&p.meter_view, meter, &meter_group);
    run(write, adapt, &bind(target.main_texture_view(), read));
    // Bloom (bloom.wgsl): L1 = down(hdr), L2 = down(L1), L3 = down(L2); then coarse to fine, blur along x and
    // add (blur along y, plus the coarser combined level). Uses this frame's exposure, as the game's
    // downsample reads the tonemap texture written before it.
    if let (true, (Some(down_hdr), Some(down_rgbm), Some(blur_h), Some(add)), [Some(u0), Some(u1), Some(u2)]) =
        (bloom_on, bloom_pipelines, bloom_uniforms)
    {
        let uniforms = [u0, u1, u2];
        let bloom_bind = |u: &BindingResource, source: &TextureView, coarser: &TextureView| {
            device.create_bind_group(
                "game_bloom_bind_group",
                &bloom_layout,
                &BindGroupEntries::sequential((u.clone(), source, &p.sampler, coarser, write)),
            )
        };
        // The coarser input of passes that don't read it: any texture that isn't their target.
        let unused = &fallback.d2.texture_view;
        let hdr = target.main_texture_view();
        run(&bt.levels[0], down_hdr, &bloom_bind(&uniforms[0], hdr, unused));
        run(&bt.levels[1], down_rgbm, &bloom_bind(&uniforms[0], &bt.levels[0], unused));
        run(&bt.levels[2], down_rgbm, &bloom_bind(&uniforms[0], &bt.levels[1], unused));
        for i in (0..3).rev() {
            run(&bt.blurred[i], blur_h, &bloom_bind(&uniforms[i], &bt.levels[i], unused));
            let coarser = if i == 2 { unused } else { &bt.combined[i + 1] };
            run(&bt.combined[i], add, &bloom_bind(&uniforms[i], &bt.blurred[i], coarser));
        }
    }
    let post = target.post_process_write();
    // Composite (`GBufferComposite`): bloom, vignette, exposure and the curve into the 8-bit frame.
    run(&bt.composite, tonemap, &bind(post.source, write));
    // Final pass (`0x01017050`): depth of field from a half-resolution copy (`FUN_008e7530`), brightness /
    // contrast and the colour cube.
    let post_bind = |colour: &TextureView, dof: &TextureView| {
        device.create_bind_group(
            "game_post_bind_group",
            &post_layout,
            &BindGroupEntries::sequential((
                uniform.clone(),
                colour,
                &p.sampler,
                dof,
                depth,
                cube_view,
                cube_sampler,
                state_cube_view,
            )),
        )
    };
    if dof_enabled() {
        run(&bt.dof, dof_down, &post_bind(&bt.composite, &fallback.d2.texture_view));
    }
    run(post.destination, final_pass, &post_bind(&bt.composite, &bt.dof));
    // MW_FLASH_LOG: reduce the HDR and the tonemapped frame onto the probe grid (flash_probe.rs).
    if let (Some(probe), Some(pipeline)) = (flash_probe, cache.get_render_pipeline(p.probe))
        && let Some(gpu) = images.get(&probe.0)
    {
        let group = device.create_bind_group(
            "flash_probe_bind_group",
            &cache.get_bind_group_layout(&p.probe_layout),
            &BindGroupEntries::sequential((post.source, post.destination, write)),
        );
        run(&gpu.texture_view, pipeline, &group);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bloom_levels_are_the_originals_fixed_targets() {
        assert_eq!(bloom_sizes((1920, 1080)), [(320, 176), (80, 44), (20, 11)]);
        assert_eq!(bloom_sizes((1280, 720)), [(320, 176), (80, 44), (20, 11)]);
    }

    #[test]
    fn vignette_gain_from_sharpness() {
        // The live renderer held k = 1.8245 with the default keyframe's Sharpness 0.34.
        assert!((vignette_k(0.34) - 1.8245).abs() < 1e-3);
    }

    #[test]
    fn dof_coc_at_infinity_matches_the_live_constants() {
        // K = (12.602, -12.56): K.x + K.y is the CoC at depth 1.
        assert!((dof_coc_infinity() - 0.042).abs() < 5e-4);
    }

    #[test]
    fn cube_blend_weights_entries() {
        let n = 2;
        let a = mw_formats::ColourCube { size: n, bgr: vec![0; n * n * n * 3] };
        let b = mw_formats::ColourCube { size: n, bgr: vec![200; n * n * n * 3] };
        let img = blend_colour_cubes(&[(Some(a), 0.75), (Some(b), 0.25)]).expect("cube");
        assert!(img.data.as_ref().expect("data").chunks(4).all(|p| p[..3] == [50, 50, 50]));
        // A missing cube is the identity: the last entry is white.
        let zero = mw_formats::ColourCube { size: n, bgr: vec![0; n * n * n * 3] };
        let img = blend_colour_cubes(&[(Some(zero), 0.5), (None, 0.5)]).expect("cube");
        let last = img.data.as_ref().expect("data").chunks(4).last().expect("texel").to_vec();
        assert_eq!(&last[..3], &[128, 128, 128]);
    }
}
