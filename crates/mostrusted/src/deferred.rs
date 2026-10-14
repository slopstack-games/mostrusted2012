//! The game's deferred lighting, hosted in Bevy's deferred pipeline.
//!
//! Opaque game materials write the game's four G-buffer targets into Bevy's deferred texture with
//! lighting-pass ID 2 (`layered.wgsl`). This module adds a full-screen pass in the `Core3d` schedule,
//! just before Bevy's own deferred lighting (which handles ID 1 and so skips these pixels), that runs
//! the port of the game's `LightBuffer_KeylightAndAmbient` shader (`game_lighting.wgsl`) on them.
//!
//! Inputs come from the main world as a [`GameLighting`] resource: the key light and the irradiance
//! cube. The pass reuses the mesh view bind group (view, depth, deferred texture, fog) as Bevy's
//! deferred lighting does.
//!
//! The same render pass then adds the world's point, cone and spot lights ([`WorldLightList`], filled by
//! `world_lights.rs`): the port of the game's `LightBuffer_Point` / `_Cone` / `_Spot` programs, one
//! instanced light volume each, blended additively as the game does.

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::core_3d::main_opaque_pass_3d;
use bevy::core_pipeline::deferred::DEFERRED_LIGHTING_PASS_ID_DEPTH_FORMAT;
use bevy::core_pipeline::deferred::copy_lighting_id::DeferredLightingIdDepthTexture;
use bevy::core_pipeline::prepass::DeferredPrepass;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::pbr::deferred::deferred_lighting;
use bevy::pbr::{MeshPipeline, MeshPipelineKey, MeshPipelineSystems, MeshViewBindGroup, ViewKeyCache};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, storage_buffer_read_only, texture_cube, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, ViewTarget};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::{Shader, ShaderDefVal};

/// Edge length of each of the key light's shadow cascades. The game packs its three cascades into one
/// atlas, stacked vertically (`LightBuffer_KeylightAndAmbient` addresses cascade i at v = (2i + 1) / 6);
/// Bevy keeps them as layers of an array texture, which the lighting pass samples the same way.
pub const SHADOW_MAP_SIZE: usize = 2048;

/// [`SHADOW_MAP_SIZE`], or `MW_SHADOW_MAP_SIZE` (a picture-changing A/B switch, research log session 5
/// round 6).
pub fn shadow_map_size() -> usize {
    std::env::var("MW_SHADOW_MAP_SIZE").ok().and_then(|v| v.parse().ok()).unwrap_or(SHADOW_MAP_SIZE)
}

/// [`SHADOW_DISTANCE`], or `MW_SHADOW_DISTANCE` metres (a picture-changing A/B switch).
fn shadow_distance() -> f32 {
    static D: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *D.get_or_init(|| std::env::var("MW_SHADOW_DISTANCE").ok().and_then(|v| v.parse().ok()).unwrap_or(SHADOW_DISTANCE))
}
/// Shadows end here (metres of view depth) and fade out over the last `SHADOW_FADE` metres. The game sets
/// both through `ShadowMap_Constants{,2}.w` from code; these values are provisional.
const SHADOW_DISTANCE: f32 = 180.0;
const SHADOW_FADE: f32 = 30.0;

/// The key light's cascades: three, as the game's shadow atlas has. Split distances are provisional (the
/// game computes `ShadowMap_ScaleOffset` on the CPU).
pub fn shadow_cascades() -> bevy::light::CascadeShadowConfig {
    bevy::light::CascadeShadowConfigBuilder {
        num_cascades: 3,
        minimum_distance: 0.1,
        first_cascade_far_bound: 20.0,
        maximum_distance: shadow_distance(),
        overlap_proportion: 0.2,
    }
    .build()
}

/// Scene inputs of the game's lighting pass.
#[derive(Resource, Clone, ExtractResource)]
pub struct GameLighting {
    /// Key light colour (linear) × illuminance in lux.
    pub key_colour: Vec3,
    /// Direction the key light travels (towards the ground).
    pub key_direction: Vec3,
    /// Towards the sky's sun (`SunMoonPath::sky_sun_direction`): the fog's corona (`SunCoronaDirection` = minus
    /// this) and its sun-side colour.
    pub sky_sun: Vec3,
    /// cd/m² per unit of the irradiance cube.
    pub sky_brightness: f32,
    /// Irradiance cube (`sky::irradiance_cube`).
    pub irradiance: Handle<Image>,
    /// The keyframe's fog, colours in cd/m².
    pub fog: FogSettings,
}

/// Inputs of the game's Fog pass (`mw_assets::environment::Fog`), colours already scaled to cd/m².
#[derive(Clone, Copy, Default)]
pub struct FogSettings {
    pub colour: Vec3,
    pub colour_sun: Vec3,
    pub near: f32,
    pub far: f32,
    pub power: f32,
    pub scattering_colour: Vec3,
    pub scattering_phase: f32,
    pub scattering_strength: f32,
}

#[derive(ShaderType, Clone, Copy, Default)]
struct GameLightingUniform {
    key_colour: Vec4,
    key_direction: Vec4,
    params: Vec4,
    fog_colour: Vec4,
    fog_colour_sun: Vec4,
    /// 1 / (far - near), near / (far - near), power
    fog_coeffs: Vec4,
    /// rgb, w = strength
    scattering: Vec4,
    /// x = phase
    scattering_params: Vec4,
}

/// A world light in the layout of `game_lighting.wgsl`'s `WorldLight` (see there).
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct GpuWorldLight {
    pub position: Vec4,
    pub colour: Vec4,
    pub direction: Vec4,
    pub cone: Vec4,
}

/// The lights to draw (`world_lights.rs`); `generation` changes whenever the list does.
#[derive(Resource, Clone, Default, ExtractResource)]
pub struct WorldLightList {
    pub lights: std::sync::Arc<Vec<GpuWorldLight>>,
    pub generation: u64,
}

pub struct GameLightingPlugin;

impl Plugin for GameLightingPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "game_lighting.wgsl");
        app.add_plugins(ExtractResourcePlugin::<GameLighting>::default())
            .add_plugins(ExtractResourcePlugin::<WorldLightList>::default())
            .init_resource::<WorldLightList>();
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_resource::<SpecializedRenderPipelines<GameLightingLayout>>()
            .init_resource::<GameLightingBuffer>()
            .init_resource::<WorldLightBuffer>()
            .add_systems(RenderStartup, init_layout.after(MeshPipelineSystems))
            .add_systems(Render, (prepare_pipelines, prepare_buffer, prepare_lights).in_set(RenderSystems::Prepare))
            .add_systems(
                Core3d,
                // Before Bevy's pass: it discards the lighting-ID depth when done.
                game_lighting.before(deferred_lighting).before(main_opaque_pass_3d).in_set(Core3dSystems::MainPass),
            );
    }
}

#[derive(Resource, Default)]
struct GameLightingBuffer(UniformBuffer<GameLightingUniform>);

/// The light list on the GPU: rewritten when [`WorldLightList::generation`] changes.
#[derive(Resource, Default)]
struct WorldLightBuffer {
    buffer: StorageBuffer<Vec<GpuWorldLight>>,
    generation: Option<u64>,
    count: u32,
}

#[derive(Resource)]
struct GameLightingLayout {
    mesh_pipeline: MeshPipeline,
    layout: BindGroupLayoutDescriptor,
    /// The light-volume pipeline's: the same plus the light list.
    light_layout: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
}

/// The full-screen key light + ambient pipeline and the light-volume pipeline.
#[derive(Component)]
struct GameLightingPipeline(CachedRenderPipelineId, CachedRenderPipelineId);

fn init_layout(mut commands: Commands, mesh_pipeline: Res<MeshPipeline>, asset_server: Res<AssetServer>) {
    let layout = BindGroupLayoutDescriptor::new(
        "game_lighting_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                uniform_buffer::<GameLightingUniform>(false),
                texture_cube(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let light_layout = BindGroupLayoutDescriptor::new(
        "world_light_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (
                uniform_buffer::<GameLightingUniform>(false),
                texture_cube(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                storage_buffer_read_only::<Vec<GpuWorldLight>>(false),
            ),
        ),
    );
    commands.insert_resource(GameLightingLayout {
        mesh_pipeline: mesh_pipeline.clone(),
        layout,
        light_layout,
        shader: load_embedded_asset!(asset_server.as_ref(), "game_lighting.wgsl"),
    });
}

impl SpecializedRenderPipeline for GameLightingLayout {
    /// The view's key, and whether this is the light-volume pipeline.
    type Key = (MeshPipelineKey, bool);

    fn specialize(&self, (key, volumes): Self::Key) -> RenderPipelineDescriptor {
        // The mesh view bind group's layout depends on these keys; declare the matching defs.
        let mut defs: Vec<ShaderDefVal> = vec!["DEFERRED_LIGHTING_PIPELINE".into(), "DEFERRED_PREPASS".into()];
        for (flag, def) in [
            (MeshPipelineKey::DEPTH_PREPASS, "DEPTH_PREPASS"),
            (MeshPipelineKey::NORMAL_PREPASS, "NORMAL_PREPASS"),
            (MeshPipelineKey::MOTION_VECTOR_PREPASS, "MOTION_VECTOR_PREPASS"),
            (MeshPipelineKey::ENVIRONMENT_MAP, "ENVIRONMENT_MAP"),
            (MeshPipelineKey::SCREEN_SPACE_AMBIENT_OCCLUSION, "SCREEN_SPACE_AMBIENT_OCCLUSION"),
            (MeshPipelineKey::DISTANCE_FOG, "DISTANCE_FOG"),
            (MeshPipelineKey::ATMOSPHERE, "ATMOSPHERE"),
        ] {
            if key.contains(flag) {
                defs.push(def.into());
            }
        }
        if key.contains(MeshPipelineKey::TONEMAP_IN_SHADER) {
            defs.push("TONEMAP_IN_SHADER".into());
            defs.push(ShaderDefVal::UInt(
                "TONEMAPPING_LUT_TEXTURE_BINDING_INDEX".into(),
                bevy::pbr::TONEMAPPING_LUT_TEXTURE_BINDING_INDEX,
            ));
            defs.push(ShaderDefVal::UInt(
                "TONEMAPPING_LUT_SAMPLER_BINDING_INDEX".into(),
                bevy::pbr::TONEMAPPING_LUT_SAMPLER_BINDING_INDEX,
            ));
            let method = key.intersection(MeshPipelineKey::TONEMAP_METHOD_RESERVED_BITS);
            if method == MeshPipelineKey::TONEMAP_METHOD_TONY_MC_MAPFACE {
                defs.push("TONEMAP_METHOD_TONY_MC_MAPFACE".into());
            } else if method == MeshPipelineKey::TONEMAP_METHOD_NONE {
                defs.push("TONEMAP_METHOD_NONE".into());
            }
        }
        if self.mesh_pipeline.binding_arrays_are_usable {
            defs.push("MULTIPLE_LIGHT_PROBES_IN_ARRAY".into());
            defs.push("MULTIPLE_LIGHTMAPS_IN_ARRAY".into());
        }
        if self.mesh_pipeline.clustered_decals_are_usable {
            defs.push("CLUSTERED_DECALS_ARE_USABLE".into());
        }
        defs.push("BLUE_NOISE_TEXTURE".into());
        defs.push("DFG_LUT".into());
        let view_layout = self.mesh_pipeline.get_view_layout(key.into());
        // Light volumes: `light_vertex` / `light_fragment`, added to the light buffer (the game's blend
        // state for its light programs is ONE, ONE), back faces only so the camera may stand inside one.
        let add = BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        };
        let (label, group, vs, fs, blend, cull_mode) = if volumes {
            (
                "world_light_pipeline",
                self.light_layout.clone(),
                "light_vertex",
                "light_fragment",
                Some(BlendState { color: add, alpha: add }),
                Some(Face::Front),
            )
        } else {
            ("game_lighting_pipeline", self.layout.clone(), "vertex", "fragment", None, None)
        };
        RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: vec![view_layout.main_layout, view_layout.binding_array_layout, group],
            vertex: VertexState {
                shader: self.shader.clone(),
                shader_defs: defs.clone(),
                entry_point: Some(vs.into()),
                ..default()
            },
            primitive: PrimitiveState { cull_mode, ..default() },
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs: defs,
                entry_point: Some(fs.into()),
                targets: vec![Some(ColorTargetState {
                    format: key.target_format(),
                    blend,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            depth_stencil: Some(DepthStencilState {
                format: DEFERRED_LIGHTING_PASS_ID_DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(CompareFunction::Equal),
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            ..default()
        }
    }
}

type DeferredCameras<'w, 's> =
    Query<'w, 's, (Entity, &'static ExtractedView), (With<ExtractedCamera>, With<DeferredPrepass>)>;

fn prepare_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    view_key_cache: Res<ViewKeyCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<GameLightingLayout>>,
    layout: Option<Res<GameLightingLayout>>,
    cameras: DeferredCameras,
) {
    let Some(layout) = layout else { return };
    for (entity, view) in &cameras {
        let Some(key) = view_key_cache.get(&view.retained_view_entity) else { continue };
        let id = pipelines.specialize(&pipeline_cache, &layout, (*key, false));
        let volumes = pipelines.specialize(&pipeline_cache, &layout, (*key, true));
        commands.entity(entity).insert(GameLightingPipeline(id, volumes));
    }
}

fn prepare_buffer(
    lighting: Option<Res<GameLighting>>,
    mut buffer: ResMut<GameLightingBuffer>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let Some(l) = lighting else { return };
    let f = &l.fog;
    let span = (f.far - f.near).max(1.0);
    buffer.0.set(GameLightingUniform {
        key_colour: l.key_colour.extend(0.0),
        key_direction: l.key_direction.normalize_or_zero().extend(0.0),
        // Shadow fade as the game's `saturate(ShadowMap_Constants.w - depth × ShadowMap_Constants2.w)`.
        params: Vec4::new(l.sky_brightness, 1.0 / SHADOW_FADE, shadow_distance() / SHADOW_FADE, 1.0),
        fog_colour: f.colour.extend(0.0),
        fog_colour_sun: f.colour_sun.extend(0.0),
        fog_coeffs: Vec4::new(1.0 / span, f.near / span, f.power, 0.0),
        // ScatteringSettings = (g, (1 - g²) × strength / 4π) as the game writes it per frame (decomp.md
        // "Fog"); g = ScatteringPhase and strength = ScatteringStrength (the live slot 0x27 at 14:13, (0.96412,
        // 0.0047362), is exactly the 14 h / 17 h keyframes' blend of both).
        scattering: f.scattering_colour.extend(
            (1.0 - f.scattering_phase * f.scattering_phase) * f.scattering_strength / (4.0 * std::f32::consts::PI),
        ),
        // x = g, yzw = towards the sky's sun (the game's SunCoronaDirection is minus this).
        scattering_params: Vec4::new(f.scattering_phase, l.sky_sun.x, l.sky_sun.y, l.sky_sun.z),
    });
    buffer.0.write_buffer(&device, &queue);
}

/// Upload the light list when it has changed.
fn prepare_lights(
    list: Option<Res<WorldLightList>>,
    mut buffer: ResMut<WorldLightBuffer>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let Some(list) = list else { return };
    if buffer.generation == Some(list.generation) {
        return;
    }
    buffer.generation = Some(list.generation);
    buffer.count = list.lights.len() as u32;
    let mut lights = (*list.lights).clone();
    if lights.is_empty() {
        // A runtime-sized array needs an element to bind.
        lights.push(GpuWorldLight::default());
    }
    buffer.buffer.set(lights);
    buffer.buffer.write_buffer(&device, &queue);
}

#[allow(clippy::too_many_arguments)]
fn game_lighting(
    view: ViewQuery<(&MeshViewBindGroup, &ViewTarget, &DeferredLightingIdDepthTexture, &GameLightingPipeline)>,
    pipeline_cache: Res<PipelineCache>,
    layout: Res<GameLightingLayout>,
    lighting: Option<Res<GameLighting>>,
    buffer: Res<GameLightingBuffer>,
    lights: Res<WorldLightBuffer>,
    images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let (mesh_view_bind_group, target, id_depth, pipeline) = view.into_inner();
    let Some(render_pipeline) = pipeline_cache.get_render_pipeline(pipeline.0) else { return };
    let (Some(lighting), Some(uniform)) = (lighting, buffer.0.binding()) else { return };
    let Some(irradiance) = images.get(&lighting.irradiance) else { return };
    let bind_group = ctx.render_device().create_bind_group(
        "game_lighting_bind_group",
        &pipeline_cache.get_bind_group_layout(&layout.layout),
        &BindGroupEntries::sequential((uniform.clone(), &irradiance.texture_view, &irradiance.sampler)),
    );
    // The light volumes, once their pipeline has compiled and there is something to draw.
    let volumes = if lights.count > 0
        && let Some(volume_pipeline) = pipeline_cache.get_render_pipeline(pipeline.1)
        && let Some(list) = lights.buffer.binding()
    {
        let group = ctx.render_device().create_bind_group(
            "world_light_bind_group",
            &pipeline_cache.get_bind_group_layout(&layout.light_layout),
            &BindGroupEntries::sequential((uniform, &irradiance.texture_view, &irradiance.sampler, list)),
        );
        Some((volume_pipeline, group))
    } else {
        None
    };
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("game_lighting"),
        color_attachments: &[Some(target.get_color_attachment())],
        depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
            view: &id_depth.texture.default_view,
            // Stored: Bevy's deferred lighting pass reads it next.
            depth_ops: Some(Operations { load: LoadOp::Load, store: StoreOp::Store }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_render_pipeline(render_pipeline);
    pass.set_bind_group(0, &mesh_view_bind_group.main, &mesh_view_bind_group.main_offsets);
    pass.set_bind_group(1, &mesh_view_bind_group.binding_array, &[]);
    pass.set_bind_group(2, &bind_group, &[]);
    pass.draw(0..3, 0..1);
    if let Some((volume_pipeline, group)) = &volumes {
        pass.set_render_pipeline(volume_pipeline);
        pass.set_bind_group(2, group, &[]);
        pass.draw(0..36, 0..lights.count);
    }
}
