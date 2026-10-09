//! The game's auto exposure and tonemap in place of Bevy's tonemapping (`tonemap.wgsl`).
//!
//! Three full-screen passes in the `Core3d` post-process set, before Bevy's tonemapping (which the camera
//! turns off with `Tonemapping::None`): meter the displayed luma onto a small grid, adapt a 1×1 exposure
//! state (ping-ponged between two textures so it carries over frames), then tonemap the HDR target with
//! the adapted exposure. Settings come from the environment keyframe (`mw_assets::environment::
//! CameraExposure`) through the [`GameTonemap`] resource. Between the adaptation and the tonemap the game's
//! bloom chain runs (`bloom.wgsl`): three downsampled RGBM levels, blurred and combined coarse to fine, added
//! in the composite. `MW_NO_BLOOM=1` turns it off (the game's bloom-off path: B = (1, 1, 1, 0)).

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, texture_3d, uniform_buffer};
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
}

/// One bloom pass's constants (bloom.wgsl): x = `kColorAddWeight.z`, y = `.w`, z = the exposure bias.
#[derive(ShaderType, Clone, Copy, Default)]
struct BloomUniform {
    weights: Vec4,
}

/// The lower bound `FUN_008e6f40` clamps the three level weights to (`DAT_00f9d474`, `.rdata`).
const BLOOM_MIN_WEIGHT: f32 = 0.01;

/// Bloom level sizes as fractions of the screen, finest first. Not traced (the targets are created outside
/// the bloom code, `FUN_008eaaf0` receives them in a struct at +0x130…+0x148): a model decision. The first
/// downsample's taps at ±1 source texel on both diagonals average a 4 × 4 block exactly when it divides by
/// 4; the later ones then divide by 2.
const BLOOM_DIVISORS: [u32; 3] = [4, 8, 16];

/// `kColorAddWeight.z` / `.w` of the three add passes, finest level first (`FUN_008e6f40`, decomp.md
/// "Level combine"): C3 = blur(L3); C2 = (b·blur(L2) + a·C3) / (a + b); C1 = (c·blur(L1) + (a + b)·C2) / Σ.
fn bloom_add_weights(w: [f32; 3]) -> [[f32; 2]; 3] {
    let [c, b, a] = w.map(|x| x.max(BLOOM_MIN_WEIGHT));
    [[c / (a + b + c), (a + b) / (a + b + c)], [b / (a + b), a / (a + b)], [1.0, 0.0]]
}

fn bloom_enabled() -> bool {
    std::env::var_os("MW_NO_BLOOM").is_none()
}

/// The vignette's falloff gain k, read live (renderer +0xa980.w = k, +0xa990.w = 1 − k, +0xa9b0.zw =
/// `Scale` × k; constant over all 20 dumps of 2026-10-06 and with both PostFX states seen). Which code
/// derives it (from `Sharpness` = 0.34?) was not found: a measured constant, not a keyframe field yet.
const VIGNETTE_K: f32 = 1.8245;

/// The vignette constants the original's pass reads, from the PostFX keyframe. The live renderer holds
/// D = inner × s and A = (outer − inner) × s with s = 1.0025…1.0385 (drifting over the run, 2.81 during a
/// state change); s is not derived yet and is taken as 1 here (a model decision, ≤ 4 % brightness).
fn vignette_constants(v: &mw_assets::environment::Vignette) -> [Vec4; 4] {
    let inner = Vec3::from(v.inner);
    let a = Vec3::from(v.outer) - inner;
    [
        a.extend(1.0 - VIGNETTE_K),
        inner.extend(VIGNETTE_K),
        Vec4::new(v.centre[0], v.centre[1], v.scale[0] * VIGNETTE_K, v.scale[1] * VIGNETTE_K),
        // +0xa9a0.x (kVignetteAngle) read 0; no keyframe field is known to feed it.
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
        app.add_plugins(ExtractResourcePlugin::<GameTonemap>::default()).add_systems(Last, update_time);
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

/// Bloom render targets for one screen size: RGBM levels (Rgba8Unorm, as the game's encode), the horizontal
/// blur and the combined result of each level (float, HDR game units). Finest first.
struct BloomTargets {
    size: (u32, u32),
    levels: [TextureView; 3],
    blurred: [TextureView; 3],
    combined: [TextureView; 3],
}

impl BloomTargets {
    fn new(device: &RenderDevice, (w, h): (u32, u32)) -> Self {
        let make = |label: &'static str, d: u32, format: TextureFormat| {
            target_2d(device, label, ((w / d).max(1), (h / d).max(1)), format)
        };
        let per_level =
            |label: &'static str, format: TextureFormat| [0, 1, 2].map(|i| make(label, BLOOM_DIVISORS[i], format));
        Self {
            size: (w, h),
            levels: per_level("bloom_level", TextureFormat::Rgba8Unorm),
            blurred: per_level("bloom_blur_h", TextureFormat::Rgba16Float),
            combined: per_level("bloom_combined", TextureFormat::Rgba16Float),
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
    meter_view: TextureView,
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
    let shader = load_embedded_asset!(asset_server.as_ref(), "tonemap.wgsl");
    let bloom_shader = load_embedded_asset!(asset_server.as_ref(), "bloom.wgsl");
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
        meter: pipeline("game_tonemap_meter", "meter", TextureFormat::R32Float),
        adapt: pipeline("game_tonemap_adapt", "adapt", TextureFormat::Rgba32Float),
        tonemap: pipeline("game_tonemap", "tonemap", TextureFormat::Rgba16Float),
        layout,
        sampler,
        meter_view: render_target(&device, "game_tonemap_meter", METER_SIZE, TextureFormat::R32Float),
        states: [
            render_target(&device, "game_tonemap_state_a", 1, TextureFormat::Rgba32Float),
            render_target(&device, "game_tonemap_state_b", 1, TextureFormat::Rgba32Float),
        ],
    });
}

fn prepare_buffer(
    tonemap: Option<Res<GameTonemap>>,
    mut buffer: ResMut<TonemapBuffer>,
    mut bloom_buffers: ResMut<BloomBuffers>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let Some(t) = tonemap else { return };
    let s = &t.settings;
    let vignette_on = std::env::var_os("MW_NO_VIGNETTE").is_none() && t.post_fx.vignette.scale != [0.0; 2];
    let [vignette_a, vignette_d, vignette_centre_scale, vignette_angle] = if vignette_on {
        vignette_constants(&t.post_fx.vignette)
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
            Vec3::from(s.bloom_colour).extend(s.bloom_intensity)
        } else {
            Vec4::new(1.0, 1.0, 1.0, 0.0)
        },
        camera: Vec4::new(s.min, s.max, s.bias, 0.0),
        exposure: Vec4::new(0.0, s.tolerance, s.speed, t.dt),
        setting: Vec4::new(s.target_high, s.target_low, s.luminance_high, s.luminance_low),
        flags: Vec4::new(f32::from(u8::from(t.reset)), f32::from(u8::from(t.colour_cube.is_some())), 0.0, 0.0),
    });
    buffer.0.write_buffer(&device, &queue);
    for (b, [z, w]) in bloom_buffers.0.iter_mut().zip(bloom_add_weights(s.bloom_weights)) {
        b.set(BloomUniform { weights: Vec4::new(z, w, s.bias, 0.0) });
        b.write_buffer(&device, &queue);
    }
}

#[allow(clippy::too_many_arguments)]
fn game_tonemap(
    // Not the planar reflection: it keeps HDR game units for the road shader to sample.
    view: ViewQuery<&ViewTarget, Without<crate::reflection::PlanarReflectionCamera>>,
    pipelines: Option<Res<TonemapPipelines>>,
    cache: Res<PipelineCache>,
    buffer: Res<TonemapBuffer>,
    bloom_buffers: Res<BloomBuffers>,
    settings: Option<Res<GameTonemap>>,
    images: Res<RenderAssets<GpuImage>>,
    fallback: Res<FallbackImage>,
    mut frame: Local<usize>,
    mut bloom_targets: Local<Option<BloomTargets>>,
    mut ctx: RenderContext,
) {
    let target = view.into_inner();
    let Some(p) = pipelines else { return };
    let (Some(meter), Some(adapt), Some(tonemap), Some(uniform)) = (
        cache.get_render_pipeline(p.meter),
        cache.get_render_pipeline(p.adapt),
        cache.get_render_pipeline(p.tonemap),
        buffer.0.binding(),
    ) else {
        return;
    };
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
    run(post.destination, tonemap, &bind(post.source, write));
}
