//! Tyre marks: strips laid on the ground under the driven car's wheels, drawn with the game's
//! `Vfx_TyreMarksNew` material (`tyre_marks.wgsl`).
//!
//! The rule (`FUN_00589e20`, the race car's per-wheel effect update; docs/decomp.md "Tyre marks and tyre smoke"):
//! every wheel on the ground lays a strip with the tyre-mark material of the surface under it (the collision
//! tag's surface → its `SurfaceVfx.TyreMarksMaterials`, the slot of the car's tyre class: `Road` for the default
//! class 2), with two weights per point: the `Slip` layer's = the wheel's lateral ramp and the `Spin` layer's =
//! the larger of its spin and lock ramps, both from the car's `TyreVfxBehaviour.Skid_Marks` (lanes 5 and 4 / 7
//! of the wheel's ramp record, `mw_handling::tyre_vfx`). The material's `Run` layer is drawn always: it is
//! transparent on tarmac (no marks from rolling) and shows on grass and dirt (rolling tracks). A wheel in the air
//! or on a surface without marks ends its strip.
//!
//! Each material has three diffuse layers: `Run`, `Slip` and `Spin` (`vfx::SurfaceEffects::tyre_marks`).
//! Model decisions (research log 2026-10-15): a strip point every `SEGMENT` m (the game adds one per update and
//! its vertex builder was not found), the strip's across vector = the tyre's lateral axis × the tyre's width (the
//! game passes wheel +0xbc with each point, read as the width; the shader scales it by `WidthScale`), the linear
//! fade over `LIFETIME` and the segment limit; points whose weights are 0 on a material whose `Run` layer is
//! fully transparent are not stored (they draw nothing); a change of material starts a new strip. The rim marks
//! (`RimMarksMaterial`, on rims) and the other tyre classes are not used.
//!
//! `MW_TYRE_MARKS_LOG=1` logs the number of live segments once a second.

use std::collections::VecDeque;
use std::sync::{Mutex, mpsc};

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::{CompressedImageFormatSupport, CompressedImageFormats};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, RenderPipelineDescriptor, ShaderType,
    SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use mw_assets::AssetStore;
use mw_assets::vfx::{TYRE_MARKS_ROAD, TyreMarkMaterial as GameMaterial};

use crate::drive::Car;

/// Model decisions (not traced; research log): a new strip point every `SEGMENT` metres, faded out linearly over
/// `LIFETIME` seconds, at most `MAX_SEGMENTS` live (the oldest go first). Texture U advances by one per
/// `U_LENGTH` metres of strip.
const SEGMENT: f32 = 0.25;
const LIFETIME: f32 = 60.0;
const MAX_SEGMENTS: usize = 4000;
const U_LENGTH: f32 = 1.0;
/// Weights below this draw nothing (the pixel program discards below 1/2550 of coverage).
const MIN_WEIGHT: f32 = 1.0 / 255.0;

pub struct TyreMarksPlugin;

impl Plugin for TyreMarksPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "tyre_marks.wgsl");
        app.add_plugins(MaterialPlugin::<MarkMaterial>::default())
            .init_resource::<Marks>()
            .add_systems(Update, (start_loading, receive))
            .add_systems(Update, (lay_marks, build_mesh).chain().after(receive).run_if(resource_exists::<MarkDraw>));
    }
}

/// One tyre-mark material, decoded on the loader thread.
struct LoadedMaterial {
    material: GameMaterial,
    images: [Option<Image>; 3],
    /// Whether the `Run` layer has any coverage (it is fully transparent on tarmac).
    run_visible: bool,
}

struct Loaded {
    materials: Vec<LoadedMaterial>,
    /// By surface index: the slot in `materials` of the surface's mark material (`Road` tyre class).
    by_surface: Vec<Option<usize>>,
}

#[derive(Resource)]
struct Loading(Mutex<mpsc::Receiver<anyhow::Result<Loaded>>>);

#[derive(Resource)]
struct MarkDraw {
    /// One mesh per material.
    meshes: Vec<Handle<Mesh>>,
    by_surface: Vec<Option<usize>>,
    run_visible: Vec<bool>,
}

/// One point of a strip.
#[derive(Clone, Copy)]
struct Point {
    pos: Vec3,
    normal: Vec3,
    /// Across the strip, the tyre's width long.
    across: Vec3,
    u: f32,
    slip: f32,
    spin: f32,
}

/// A quad between two strip points, with the time it was laid and its material.
#[derive(Clone, Copy)]
struct Segment {
    a: Point,
    b: Point,
    born: f32,
    material: usize,
}

#[derive(Resource, Default)]
struct Marks {
    segments: VecDeque<Segment>,
    /// Per wheel of the driven car: the strip's last point and material while it is being laid.
    strips: Vec<Option<(Point, usize)>>,
    log: bool,
    last_log: f32,
    /// Bumped whenever a segment is added or dropped; the meshes are rebuilt then and every `REFADE` seconds.
    generation: u64,
}

/// The fade is slow (`LIFETIME`): refresh the faded mesh this often while nothing changes.
const REFADE: f32 = 0.5;

fn start_loading(
    mut commands: Commands,
    game: Res<crate::Game>,
    support: Option<Res<CompressedImageFormatSupport>>,
    mut started: Local<bool>,
) {
    if *started {
        return;
    }
    let Some(support) = support else { return };
    *started = true;
    let bc = support.0.contains(CompressedImageFormats::BC);
    let store = game.store.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("tyre marks loader".into())
        .spawn(move || {
            let _ = tx.send(load(&store, bc));
        })
        .expect("spawn tyre marks loader");
    commands.insert_resource(Loading(Mutex::new(rx)));
}

fn load(store: &AssetStore, bc: bool) -> anyhow::Result<Loaded> {
    let lib = store.vfx_library()?;
    let mut ids = Vec::new();
    let by_surface: Vec<Option<usize>> = lib
        .surfaces
        .iter()
        .map(|s| {
            let id = s.dry.tyre_marks[TYRE_MARKS_ROAD]?;
            Some(ids.iter().position(|&i| i == id).unwrap_or_else(|| {
                ids.push(id);
                ids.len() - 1
            }))
        })
        .collect();
    if ids.is_empty() {
        anyhow::bail!("no surface has a tyre marks material");
    }
    let mut materials = Vec::with_capacity(ids.len());
    for id in ids {
        let material = store.tyre_mark_material(id)?;
        let textures = material.textures.map(|t| t.and_then(|t| store.texture(t).ok()));
        let run_visible = textures[0]
            .as_ref()
            .and_then(|t| t.decode_rgba8(0))
            .is_none_or(|(_, _, px)| px.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
        let images = textures.map(|t| t.and_then(|t| crate::convert::image(&t, true, bc)));
        info!(
            "tyre marks: material {id} textures {:?} width scale {:?} animation {:?} specular {:?} run layer {}",
            material.textures,
            material.width_scale,
            material.animation,
            material.specular,
            if run_visible { "visible" } else { "transparent" }
        );
        materials.push(LoadedMaterial { material, images, run_visible });
    }
    info!(
        "tyre marks: {} of {} surfaces have marks, {} materials",
        by_surface.iter().flatten().count(),
        by_surface.len(),
        materials.len()
    );
    Ok(Loaded { materials, by_surface })
}

fn receive(
    mut commands: Commands,
    loading: Option<Res<Loading>>,
    lighting: Option<Res<crate::deferred::GameLighting>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<MarkMaterial>>,
    mut marks: ResMut<Marks>,
) {
    let Some(loading) = loading else { return };
    let msg = match loading.0.lock().unwrap().try_recv() {
        Ok(m) => m,
        Err(mpsc::TryRecvError::Empty) => return,
        Err(mpsc::TryRecvError::Disconnected) => {
            commands.remove_resource::<Loading>();
            return;
        }
    };
    commands.remove_resource::<Loading>();
    let loaded = match msg {
        Ok(l) => l,
        Err(e) => {
            warn!("no tyre marks: {e}");
            return;
        }
    };
    let (key_colour, key_direction, sky, irradiance) = match &lighting {
        Some(l) => (l.key_colour, l.key_direction, l.sky_brightness, Some(l.irradiance.clone())),
        None => (Vec3::splat(9000.0), Vec3::new(-0.4, -0.8, -0.4).normalize(), 1200.0, None),
    };
    let mut draw = MarkDraw { meshes: Vec::new(), by_surface: loaded.by_surface, run_visible: Vec::new() };
    for l in loaded.materials {
        let [run, slip, spin] = l.images.map(|i| i.map(|i| images.add(i)));
        let m = &l.material;
        let material = materials.add(MarkMaterial {
            params: MarkParams {
                material: Vec4::new(m.width_scale.unwrap_or(0.0), m.specular.unwrap_or(0.0), 0.0, 0.0),
                key_colour: key_colour.extend(sky),
                key_direction: key_direction.extend(0.0),
            },
            run,
            slip,
            spin,
            irradiance: irradiance.clone(),
        });
        let mesh = meshes.add(empty_mesh());
        commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material), Transform::IDENTITY, NoFrustumCulling));
        draw.meshes.push(mesh);
        draw.run_visible.push(l.run_visible);
    }
    marks.log = std::env::var_os("MW_TYRE_MARKS_LOG").is_some();
    commands.insert_resource(draw);
}

fn lay_marks(
    time: Res<Time>,
    draw: Res<MarkDraw>,
    cars: Query<&Car, With<crate::drive::Player>>,
    mut marks: ResMut<Marks>,
) {
    let now = time.elapsed_secs();
    let marks = &mut *marks;
    // The driven car (the first one).
    if let Some(car) = cars.iter().next() {
        let contacts: Vec<_> = car.wheel_contacts().collect();
        marks.strips.resize(contacts.len(), None);
        for (i, contact) in contacts.into_iter().enumerate() {
            let Some(c) = contact else {
                marks.strips[i] = None;
                continue;
            };
            let Some(material) = c.surface.and_then(|s| draw.by_surface.get(s).copied().flatten()) else {
                marks.strips[i] = None;
                continue;
            };
            let (slip, spin) = (c.marks.lat, c.marks.spin.max(c.marks.lock));
            if !draw.run_visible[material] && slip < MIN_WEIGHT && spin < MIN_WEIGHT {
                marks.strips[i] = None;
                continue;
            }
            let across = c.across.reject_from(c.normal).normalize_or_zero() * c.width;
            let mut p = Point { pos: c.point, normal: c.normal, across, u: 0.0, slip, spin };
            match marks.strips[i] {
                Some((last, m)) if m == material => {
                    let d = p.pos.distance(last.pos);
                    if d >= SEGMENT {
                        p.u = last.u + d / U_LENGTH;
                        marks.segments.push_back(Segment { a: last, b: p, born: now, material });
                        marks.generation += 1;
                        while marks.segments.len() > MAX_SEGMENTS {
                            marks.segments.pop_front();
                        }
                        marks.strips[i] = Some((p, material));
                    }
                }
                _ => marks.strips[i] = Some((p, material)),
            }
        }
    }
    while marks.segments.front().is_some_and(|s| now - s.born > LIFETIME) {
        marks.segments.pop_front();
        marks.generation += 1;
    }
    if marks.log && now - marks.last_log >= 1.0 {
        marks.last_log = now;
        let wheels: Vec<String> = cars
            .iter()
            .next()
            .map(|car| {
                car.wheel_contacts()
                    .map(|c| match c {
                        Some(c) => format!(
                            "s{} slip {:.2} spin {:.2} lock {:.2}",
                            c.surface.map_or(-1, |s| s as i64),
                            c.marks.lat,
                            c.marks.spin,
                            c.marks.lock
                        ),
                        None => "air".into(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        info!(
            "tyre marks: {} segments alive (limit {MAX_SEGMENTS}, lifetime {LIFETIME} s); wheels: {}",
            marks.segments.len(),
            wheels.join(" | ")
        );
    }
}

fn build_mesh(
    time: Res<Time>,
    marks: Res<Marks>,
    draw: Res<MarkDraw>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut built: Local<(u64, f32)>,
) {
    let now = time.elapsed_secs();
    if built.0 == marks.generation && now - built.1 < REFADE {
        return;
    }
    *built = (marks.generation, now);
    for (material, handle) in draw.meshes.iter().enumerate() {
        let segments: Vec<&Segment> = marks.segments.iter().filter(|s| s.material == material).collect();
        let _ = meshes.insert(handle, strip_mesh(&segments, now));
    }
}

fn strip_mesh(segments: &[&Segment], now: f32) -> Mesh {
    if segments.is_empty() {
        // Empty vertex buffers upset Bevy's mesh allocator (particles.rs): keep one degenerate triangle.
        return empty_mesh();
    }
    let n = segments.len() * 4;
    let (mut pos, mut normal, mut uv, mut side, mut colour) = (
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    );
    let mut indices = Vec::with_capacity(segments.len() * 6);
    for s in segments {
        let fade = (1.0 - (now - s.born) / LIFETIME).clamp(0.0, 1.0);
        let base = pos.len() as u32;
        for p in [s.a, s.b] {
            for v in [0.0, 1.0] {
                pos.push(p.pos.to_array());
                normal.push(p.normal.to_array());
                uv.push([p.u, v]);
                side.push(p.across.extend(0.0).to_array());
                colour.push([p.slip, p.spin, v, fade]);
            }
        }
        indices.extend([base, base + 1, base + 2, base + 1, base + 3, base + 2]);
    }
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normal);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    // Unused: keeps the vertex layout off the cars' (a shared layout with the skinned car meshes broke the
    // multi-draw batching: "skinned_mesh_layout" validation errors).
    m.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.0f32; 2]; n]);
    m.insert_attribute(Mesh::ATTRIBUTE_TANGENT, side);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, colour);
    m.insert_indices(Indices::U32(indices));
    m
}

fn empty_mesh() -> Mesh {
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 3]);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; 3]);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; 3]);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.0f32; 2]; 3]);
    m.insert_attribute(Mesh::ATTRIBUTE_TANGENT, vec![[1.0f32, 0.0, 0.0, 0.0]; 3]);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; 3]);
    m.insert_indices(Indices::U32(vec![0, 1, 2]));
    m
}

#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct MarkParams {
    material: Vec4,
    key_colour: Vec4,
    key_direction: Vec4,
}

/// `Vfx_TyreMarksNew` (`tyre_marks.wgsl`).
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct MarkMaterial {
    #[uniform(0)]
    params: MarkParams,
    #[texture(1)]
    #[sampler(2)]
    run: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    slip: Option<Handle<Image>>,
    #[texture(5)]
    #[sampler(6)]
    spin: Option<Handle<Image>>,
    #[texture(7, dimension = "cube")]
    #[sampler(8)]
    irradiance: Option<Handle<Image>>,
}

impl Material for MarkMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://mostrusted/tyre_marks.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://mostrusted/tyre_marks.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
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
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
            Mesh::ATTRIBUTE_TANGENT.at_shader_location(3),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(5),
        ])?];
        descriptor.primitive.cull_mode = None;
        // ONE, INV_SRC_ALPHA (premultiplied), as the game's pixel program expects.
        let c = BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::OneMinusSrcAlpha,
            operation: BlendOperation::Add,
        };
        if let Some(t) = descriptor.fragment.as_mut().and_then(|f| f.targets.get_mut(0)).and_then(|t| t.as_mut()) {
            t.blend = Some(BlendState { color: c, alpha: c });
        }
        Ok(())
    }
}
