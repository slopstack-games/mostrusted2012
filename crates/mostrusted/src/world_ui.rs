//! UI quads placed in the world, drawn the way the game draws its world layouts (`DrawWorldLayouts` of the
//! points-of-interest widget: race gate markers): in the UI pass after the tonemap, so they are not exposed,
//! fogged or bloomed, and depth-tested against the scene. The game's `UIDepthTestedShader` (`mwtool shader-asm
//! UIDepthTestedShader`) samples the deferred depth buffer and draws texture × colour where the scene is not
//! in front; here the main pass's depth buffer is bound read-only with the same comparison.
//!
//! Producers fill [`WorldUiQuads`] each frame (triangles in world space); the render world draws them after
//! `tonemap::game_tonemap`, before Bevy's tonemapping (off for the main camera).

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::mesh::VertexBufferLayout;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer_sized};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::texture::{FallbackImage, GpuImage};
use bevy::render::view::{ExtractedView, ViewDepthTexture, ViewTarget};
use bevy::render::{RenderApp, RenderStartup};

pub struct WorldUiPlugin;

impl Plugin for WorldUiPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "world_ui.wgsl");
        app.init_resource::<WorldUiQuads>().add_plugins(ExtractResourcePlugin::<WorldUiQuads>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app.add_systems(RenderStartup, init_pipelines).add_systems(
            Core3d,
            world_ui_pass.after(crate::tonemap::GameTonemapPass).before(tonemapping).in_set(Core3dSystems::PostProcess),
        );
    }
}

/// One vertex: world position, UV, linear RGBA.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldUiVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub colour: [f32; 4],
}

/// Triangles sharing a texture and blend.
#[derive(Debug, Clone, Default)]
pub struct WorldUiBatch {
    /// `None`: untextured (solid colour).
    pub texture: Option<Handle<Image>>,
    /// Source alpha, one (`UIAdditiveShader`); otherwise alpha-over.
    pub additive: bool,
    /// Triangle list.
    pub vertices: Vec<WorldUiVertex>,
}

/// What to draw this frame, in order.
#[derive(Resource, Debug, Clone, Default, ExtractResource)]
pub struct WorldUiQuads(pub Vec<WorldUiBatch>);

#[derive(Resource)]
struct WorldUiPipelines {
    view_layout: BindGroupLayoutDescriptor,
    texture_layout: BindGroupLayoutDescriptor,
    alpha: CachedRenderPipelineId,
    additive: CachedRenderPipelineId,
}

/// The main camera's colour target after the game tonemap (`Hdr`: float).
const TARGET_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

fn init_pipelines(mut commands: Commands, asset_server: Res<AssetServer>, cache: Res<PipelineCache>) {
    let view_layout = BindGroupLayoutDescriptor::new(
        "world_ui_view_layout",
        &BindGroupLayoutEntries::single(ShaderStages::VERTEX, uniform_buffer_sized(false, None)),
    );
    let texture_layout = BindGroupLayoutDescriptor::new(
        "world_ui_texture_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (texture_2d(TextureSampleType::Float { filterable: true }), sampler(SamplerBindingType::Filtering)),
        ),
    );
    let shader = load_embedded_asset!(asset_server.as_ref(), "world_ui.wgsl");
    let pipeline = |label: &'static str, dst: BlendFactor| {
        let colour =
            BlendComponent { src_factor: BlendFactor::SrcAlpha, dst_factor: dst, operation: BlendOperation::Add };
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: vec![view_layout.clone(), texture_layout.clone()],
            vertex: VertexState {
                shader: shader.clone(),
                entry_point: Some("vertex".into()),
                buffers: vec![VertexBufferLayout::from_vertex_formats(
                    VertexStepMode::Vertex,
                    [VertexFormat::Float32x3, VertexFormat::Float32x2, VertexFormat::Float32x4],
                )],
                ..default()
            },
            fragment: Some(FragmentState {
                shader: shader.clone(),
                entry_point: Some("fragment".into()),
                targets: vec![Some(ColorTargetState {
                    format: TARGET_FORMAT,
                    blend: Some(BlendState { color: colour, alpha: colour }),
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            primitive: PrimitiveState { cull_mode: None, ..default() },
            // Reverse Z: drawn where nothing of the scene is nearer.
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: default(),
                bias: default(),
            }),
            ..default()
        })
    };
    commands.insert_resource(WorldUiPipelines {
        alpha: pipeline("world_ui_alpha", BlendFactor::OneMinusSrcAlpha),
        additive: pipeline("world_ui_additive", BlendFactor::One),
        view_layout,
        texture_layout,
    });
}

#[allow(clippy::too_many_arguments)]
fn world_ui_pass(
    view: ViewQuery<
        (&ViewTarget, &ViewDepthTexture, &ExtractedView),
        Without<crate::reflection::PlanarReflectionCamera>,
    >,
    quads: Option<Res<WorldUiQuads>>,
    pipelines: Option<Res<WorldUiPipelines>>,
    cache: Res<PipelineCache>,
    images: Res<RenderAssets<GpuImage>>,
    fallback: Res<FallbackImage>,
    mut ctx: RenderContext,
) {
    let (target, depth, extracted) = view.into_inner();
    let (Some(quads), Some(p)) = (quads, pipelines) else { return };
    if quads.0.iter().all(|b| b.vertices.is_empty()) || target.main_texture_format() != TARGET_FORMAT {
        return;
    }
    let (Some(alpha), Some(additive)) = (cache.get_render_pipeline(p.alpha), cache.get_render_pipeline(p.additive))
    else {
        return;
    };
    let device = ctx.render_device().clone();
    let clip_from_world = extracted
        .clip_from_world
        .unwrap_or_else(|| extracted.clip_from_view * extracted.world_from_view.to_matrix().inverse());
    let view_buffer = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("world_ui_view"),
        contents: &floats_to_bytes(&clip_from_world.to_cols_array()),
        usage: BufferUsages::UNIFORM,
    });
    let view_group = device.create_bind_group(
        "world_ui_view_group",
        &cache.get_bind_group_layout(&p.view_layout),
        &BindGroupEntries::single(view_buffer.as_entire_binding()),
    );
    let texture_layout = cache.get_bind_group_layout(&p.texture_layout);
    let mut draws = Vec::new();
    for b in quads.0.iter().filter(|b| !b.vertices.is_empty()) {
        let image = b.texture.as_ref().and_then(|h| images.get(h));
        let (view, sampler) = match image {
            Some(i) => (&i.texture_view, &i.sampler),
            None if b.texture.is_some() => continue,
            None => (&fallback.d2.texture_view, &fallback.d2.sampler),
        };
        let group = device.create_bind_group(
            "world_ui_texture_group",
            &texture_layout,
            &BindGroupEntries::sequential((view, sampler)),
        );
        let data: Vec<f32> =
            b.vertices.iter().flat_map(|v| v.position.into_iter().chain(v.uv).chain(v.colour)).collect();
        let buffer = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("world_ui_vertices"),
            contents: &floats_to_bytes(&data),
            usage: BufferUsages::VERTEX,
        });
        draws.push((group, buffer, b.vertices.len() as u32, b.additive));
    }
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("world_ui"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: target.main_texture_view(),
            depth_slice: None,
            resolve_target: None,
            ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
        })],
        depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_bind_group(0, &view_group, &[]);
    for (group, buffer, count, add) in &draws {
        pass.set_pipeline(if *add { additive } else { alpha });
        pass.set_bind_group(1, group, &[]);
        pass.set_vertex_buffer(0, *buffer.slice(..));
        pass.draw(0..*count, 0..1);
    }
}

fn floats_to_bytes(f: &[f32]) -> Vec<u8> {
    f.iter().flat_map(|v| v.to_le_bytes()).collect()
}
