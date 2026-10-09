//! Showing `mw_assets::ui::Quad`s (the game's UI layouts, drawn by `UiScene::draw`) as Bevy UI nodes: the
//! driving HUD, the race HUD and results, and Easydrive all draw through [`show_quads`].
//!
//! Blending follows the element's `UIMaterial`: its render material's shader carries a `MaterialState`
//! (resource type 0x6) whose blend block packs one D3D11 render-target blend per u32 (enable 1 bit, src 5, dst 5,
//! op 3, src alpha 5, dst alpha 5, op alpha 3, write mask 4; research log 2026-10-09). `FlaptGenericShader`
//! (most UI) is `0x7931498b` = src alpha / inverse src alpha, add: Bevy's own UI blend, so those quads are plain
//! `ImageNode`s. `UIAdditiveShader` is `0x7911488b` = src alpha / **one**, add: lens flares, glints and streaks
//! authored on black, which drawn alpha-over show as opaque dark boxes. Those quads use [`AdditiveQuadMaterial`]
//! (`ui_quads.wgsl`) with that blend state.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, RenderPipelineDescriptor, ShaderType,
};
use bevy::shader::ShaderRef;
use mw_assets::ui::{Quad, UiBlend};
use mw_bundle::ResourceId;

pub struct UiQuadsPlugin;

impl Plugin for UiQuadsPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "ui_quads.wgsl");
        app.add_plugins(UiMaterialPlugin::<AdditiveQuadMaterial>::default());
    }
}

#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct QuadParams {
    colour: Vec4,
    uv: Vec4,
}

/// One additive UI quad: texture, UV rectangle and tint (`ui_quads.wgsl`).
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct AdditiveQuadMaterial {
    #[uniform(0)]
    params: QuadParams,
    #[texture(1)]
    #[sampler(2)]
    texture: Handle<Image>,
}

impl UiMaterial for AdditiveQuadMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://mostrusted/ui_quads.wgsl".into()
    }

    fn specialize(descriptor: &mut RenderPipelineDescriptor, _key: UiMaterialKey<Self>) {
        if let Some(target) = descriptor.fragment.as_mut().and_then(|f| f.targets.get_mut(0)).and_then(|t| t.as_mut()) {
            // `UIAdditiveShader`'s MaterialState 01000006905172cc: colour and alpha both src alpha, one, add.
            let add = BlendComponent {
                src_factor: BlendFactor::SrcAlpha,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            };
            target.blend = Some(BlendState { color: add, alpha: add });
        }
    }
}

/// A pool of quad nodes under one root, reused frame to frame (grown as needed).
#[derive(Default)]
pub struct QuadPool {
    entities: Vec<Entity>,
    /// Per node: its additive material while it shows an additive quad.
    additive: Vec<Option<Handle<AdditiveQuadMaterial>>>,
}

/// Marks the nodes of a [`QuadPool`] (so other UI queries can exclude them).
#[derive(Component)]
pub struct QuadNode;

/// The query [`show_quads`] needs: use as `Query<PoolNode, With<QuadNode>>`.
pub type PoolNode<'a> = (&'a mut Node, Option<&'a mut ImageNode>, &'a mut UiTransform, &'a mut Visibility);

/// Show `quads` as absolutely placed nodes under `root`, reusing (and growing) the node `pool`. `map_image` fills
/// the minimap quad.
#[allow(clippy::too_many_arguments)]
pub fn show_quads(
    commands: &mut Commands,
    root: Entity,
    pool: &mut QuadPool,
    quads: &[Quad],
    textures: &HashMap<ResourceId, (Handle<Image>, Vec2)>,
    map_image: Option<Handle<Image>>,
    nodes: &mut Query<PoolNode, With<QuadNode>>,
    materials: &mut Assets<AdditiveQuadMaterial>,
) {
    while pool.entities.len() < quads.len() {
        let e = commands
            .spawn((
                Node { position_type: PositionType::Absolute, ..default() },
                ImageNode::default(),
                UiTransform::default(),
                Visibility::Hidden,
                Pickable::IGNORE,
                QuadNode,
                ChildOf(root),
            ))
            .id();
        pool.entities.push(e);
        pool.additive.push(None);
    }
    for (i, &e) in pool.entities.iter().enumerate() {
        let Ok((mut node, image, mut transform, mut vis)) = nodes.get_mut(e) else { continue };
        let Some(q) = quads.get(i) else {
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
            continue;
        };
        let (handle, rect, flip) = match (q.map, q.texture.and_then(|t| textures.get(&t))) {
            (true, _) => (map_image.clone().unwrap_or_default(), None, (false, false)),
            (false, Some((h, size))) => {
                let [u0, v0, u1, v1] = q.uv;
                let r = Rect::new(u0.min(u1) * size.x, v0.min(v1) * size.y, u0.max(u1) * size.x, v0.max(v1) * size.y);
                (h.clone(), Some(r), (u0 > u1, v0 > v1))
            }
            (false, None) if q.texture.is_some() => {
                *vis = Visibility::Hidden;
                continue;
            }
            // Solid colour: Bevy's default image is one white pixel.
            (false, None) => (Handle::default(), None, (false, false)),
        };
        let additive = q.blend == UiBlend::Additive && !q.map;
        if additive {
            let params = QuadParams { colour: Vec4::from_array(q.colour), uv: Vec4::from_array(q.uv) };
            match &pool.additive[i] {
                Some(m) => {
                    // Touch the asset only when something changed (a change re-uploads its bind group).
                    if materials.get(m).is_some_and(|mat| mat.params != params || mat.texture != handle)
                        && let Some(mut mat) = materials.get_mut(m)
                    {
                        mat.params = params;
                        mat.texture = handle.clone();
                    }
                }
                None => {
                    let m = materials.add(AdditiveQuadMaterial { params, texture: handle.clone() });
                    commands.entity(e).remove::<ImageNode>().insert(MaterialNode(m.clone()));
                    pool.additive[i] = Some(m);
                }
            }
        } else {
            let color = Color::linear_rgba(q.colour[0], q.colour[1], q.colour[2], q.colour[3]);
            // `Stretch`: a quad fills its rectangle whatever the texture's aspect (`Auto` keeps the image's aspect
            // inside the node: a solid full-screen quad, a 1×1 image, came out square).
            let wanted = || ImageNode {
                image: handle.clone(),
                color,
                rect,
                flip_x: flip.0,
                flip_y: flip.1,
                image_mode: bevy::ui::widget::NodeImageMode::Stretch,
                ..default()
            };
            if pool.additive[i].take().is_some() {
                commands.entity(e).remove::<MaterialNode<AdditiveQuadMaterial>>().insert(wanted());
            } else if let Some(mut image) = image
                && (image.image != handle
                    || image.color != color
                    || image.rect != rect
                    || image.flip_x != flip.0
                    || image.flip_y != flip.1)
            {
                *image = wanted();
            }
        }
        let (left, top, w, h) = (Val::Px(q.rect[0]), Val::Px(q.rect[1]), Val::Px(q.rect[2]), Val::Px(q.rect[3]));
        if node.left != left || node.top != top || node.width != w || node.height != h {
            node.left = left;
            node.top = top;
            node.width = w;
            node.height = h;
        }
        let rot = Rot2::degrees(q.rotation);
        if transform.rotation != rot {
            transform.rotation = rot;
        }
        if *vis != Visibility::Inherited {
            *vis = Visibility::Inherited;
        }
    }
}
