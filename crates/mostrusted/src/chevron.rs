//! The race route's markers: the event's `CustomChevron` lines drawn as the game's holographic boards
//! (`chevron.wgsl`, ports of the shaders `Chevron` and `ChevronBlockRoad`; materials from
//! `GameLogicConfig.CheckpointConfig`, `mw_assets::events::RouteMarkers`).
//!
//! The shaders take world positions and UVs only, so the game builds the strips in code; that builder was not
//! found in the dump. Model decisions (docs/decomp.md "Races"): a line with `f8ccf73e` set is drawn with the
//! block-road crosses, otherwise with the scrolling arrows (inferred from the data: the set lines are short and
//! cross side roads); strips stand on the line's points, [`BOARD_HEIGHT`] tall, one texture repeat per
//! `BOARD_HEIGHT` of length (square texels); `baseColour` (the object constant the code sets) is white; the
//! arrows blend premultiplied-over (their program outputs rgb × a), the crosses alpha-over.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, Extent3d, RenderPipelineDescriptor,
    ShaderType, SpecializedMeshPipelineError, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use mw_assets::AssetStore;
use mw_assets::events::{Chevron, ChevronLook};

/// Model decision: the boards' height (m).
pub const BOARD_HEIGHT: f32 = 2.0;

pub struct ChevronPlugin;

impl Plugin for ChevronPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "chevron.wgsl");
        app.add_plugins(MaterialPlugin::<ChevronMaterial>::default());
    }
}

#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct ChevronParams {
    colour: Vec4,
    mode: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct ChevronMaterial {
    #[uniform(0)]
    params: ChevronParams,
    #[texture(1)]
    #[sampler(2)]
    texture: Option<Handle<Image>>,
}

impl Material for ChevronMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://mostrusted/chevron.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://mostrusted/chevron.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers = vec![layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
        ])?];
        descriptor.primitive.cull_mode = None;
        if let Some(target) = descriptor.fragment.as_mut().and_then(|f| f.targets.get_mut(0)).and_then(|t| t.as_mut()) {
            // Premultiplied over for both (chevron.wgsl premultiplies the block program's output, which equals
            // its alpha-over).
            let c = BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::OneMinusSrcAlpha,
                operation: BlendOperation::Add,
            };
            target.blend = Some(BlendState { color: c, alpha: c });
        }
        Ok(())
    }
}

/// Decode a marker texture with repeating addressing (the strips tile it).
fn texture(store: &AssetStore, look: &ChevronLook) -> Option<Image> {
    let t = store.texture(look.texture?).ok()?;
    let (w, h, px) = t.decode_rgba8(0)?;
    let mut img = Image::new(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::ClampToEdge,
        ..ImageSamplerDescriptor::linear()
    });
    Some(img)
}

/// A vertical strip along the line's points.
fn strip(points: &[[f32; 3]]) -> Mesh {
    let mut pos = Vec::new();
    let mut uv = Vec::new();
    let mut idx = Vec::new();
    let mut u = 0.0;
    for (i, p) in points.iter().enumerate() {
        let p = Vec3::from_array(*p);
        if i > 0 {
            u += p.distance(Vec3::from_array(points[i - 1])) / BOARD_HEIGHT;
            let b = (2 * i) as u32;
            idx.extend_from_slice(&[b - 2, b - 1, b, b, b - 1, b + 1]);
        }
        pos.push(p.to_array());
        pos.push((p + Vec3::Y * BOARD_HEIGHT).to_array());
        uv.push([u, 1.0]);
        uv.push([u, 0.0]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
}

/// Marker entities of a race (despawned with it).
#[derive(Component)]
pub struct RouteMarker;

/// Spawn the event's chevron lines.
pub fn spawn_markers(
    commands: &mut Commands,
    store: &AssetStore,
    chevrons: &[Chevron],
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<ChevronMaterial>,
) {
    let looks = match store.route_markers() {
        Ok(l) => l,
        Err(e) => {
            warn!("race: no route marker materials ({e})");
            return;
        }
    };
    let mut make = |look: &ChevronLook, block: bool| {
        let texture = texture(store, look).map(|i| images.add(i));
        materials.add(ChevronMaterial {
            params: ChevronParams {
                colour: Vec4::from_array(look.colour),
                mode: Vec4::new(look.speed, if block { 1.0 } else { 0.0 }, 0.0, 0.0),
            },
            texture,
        })
    };
    let arrows = make(&looks.arrows, false);
    let block = make(&looks.block, true);
    for c in chevrons.iter().filter(|c| c.points.len() >= 2) {
        let material = if c.flag { block.clone() } else { arrows.clone() };
        commands.spawn((
            RouteMarker,
            Name::new(format!("chevron {}", c.id)),
            Mesh3d(meshes.add(strip(&c.points))),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            bevy::light::NotShadowCaster,
        ));
    }
    info!("race: {} chevron lines", chevrons.len());
}
