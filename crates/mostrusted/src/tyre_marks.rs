//! Tyre marks: strips laid on the road behind sliding and spinning tyres, drawn with the game's
//! `Vfx_TyreMarksNew` material (`tyre_marks.wgsl`).
//!
//! Data: the surface's `SurfaceVfx.TyreMarksMaterials` (one material per tyre: `Road`, `Sport`, `Track`,
//! `Offroad`, one unnamed; `vfx::SurfaceEffects::tyre_marks`). Each material has three diffuse layers: `Run`
//! (always drawn), `Slip` (weighted by the lateral intensity) and `Spin` (weighted by the spin intensity).
//! Surface types are not decoded from the collision mesh yet, so every wheel uses the default surface
//! (tarmac, dry) and the `Road` tyre, as the particles do.
//!
//! Rule: the code that lays the marks was not found in the dump (docs/decomp.md "Tyre marks"). The
//! intensities reuse the traced tyre skid *sound* rule (`FUN_009d1bf0`, docs/decomp.md): lateral =
//! clamp((|α| − 0.05°) / (40° − 0.05°)) × clamp(v / 10 mph), spin = clamp((|tread − ground| − 2 mph) /
//! (20 − 2 mph)), here per wheel instead of one reference wheel. Strip building, the minimum intensity, the
//! lifetime and the segment limit are model decisions (research log, 2026-10-06 round 4).
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

/// Model decisions (not traced; research log): a new strip point every `SEGMENT` metres, marks laid while the
/// intensity is at least `MIN_INTENSITY`, faded out linearly over `LIFETIME` seconds, at most `MAX_SEGMENTS`
/// live (the oldest go first). Texture U advances by one per `U_LENGTH` metres of strip.
const SEGMENT: f32 = 0.25;
const MIN_INTENSITY: f32 = 0.05;
const LIFETIME: f32 = 60.0;
const MAX_SEGMENTS: usize = 4000;
const U_LENGTH: f32 = 1.0;
const MS_TO_MPH: f32 = 2.236_936;

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

struct Loaded {
    material: GameMaterial,
    images: [Option<Image>; 3],
}

#[derive(Resource)]
struct Loading(Mutex<mpsc::Receiver<anyhow::Result<Loaded>>>);

#[derive(Resource)]
struct MarkDraw {
    mesh: Handle<Mesh>,
}

/// One point of a strip.
#[derive(Clone, Copy)]
struct Point {
    pos: Vec3,
    normal: Vec3,
    side: Vec3,
    u: f32,
    slip: f32,
    spin: f32,
    opacity: f32,
}

/// A quad between two strip points, with the time it was laid.
#[derive(Clone, Copy)]
struct Segment {
    a: Point,
    b: Point,
    born: f32,
}

#[derive(Resource, Default)]
struct Marks {
    segments: VecDeque<Segment>,
    /// Per wheel of the driven car: the strip's last point while it is being laid.
    strips: Vec<Option<Point>>,
    log: bool,
    last_log: f32,
}

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
    // Surfaces 0 and 1 have no tyre mark materials; with surface types not decoded yet, the first surface
    // that has one stands in for the road (model decision).
    let with_marks: Vec<usize> = lib
        .surfaces
        .iter()
        .enumerate()
        .filter(|(_, s)| s.dry.tyre_marks[TYRE_MARKS_ROAD].is_some())
        .map(|(i, _)| i)
        .collect();
    let (index, id) = with_marks
        .first()
        .and_then(|&i| Some((i, lib.surfaces[i].dry.tyre_marks[TYRE_MARKS_ROAD]?)))
        .ok_or_else(|| anyhow::anyhow!("no surface has a tyre marks material"))?;
    info!("tyre marks: {} of {} surfaces have marks; using surface {index}", with_marks.len(), lib.surfaces.len());
    let material = store.tyre_mark_material(id)?;
    let images = material
        .textures
        .map(|t| t.and_then(|t| store.texture(t).ok()).and_then(|t| crate::convert::image(&t, true, bc)));
    info!(
        "tyre marks: material {id} textures {:?} width scale {:?} animation {:?} specular {:?}",
        material.textures, material.width_scale, material.animation, material.specular
    );
    Ok(Loaded { material, images })
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
    let [run, slip, spin] = loaded.images.map(|i| i.map(|i| images.add(i)));
    let m = &loaded.material;
    let material = materials.add(MarkMaterial {
        params: MarkParams {
            material: Vec4::new(m.width_scale.unwrap_or(0.0), m.specular.unwrap_or(0.0), 0.0, 0.0),
            key_colour: key_colour.extend(sky),
            key_direction: key_direction.extend(0.0),
        },
        run,
        slip,
        spin,
        irradiance,
    });
    let mesh = meshes.add(empty_mesh());
    commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material), Transform::IDENTITY, NoFrustumCulling));
    marks.log = std::env::var_os("MW_TYRE_MARKS_LOG").is_some();
    commands.insert_resource(MarkDraw { mesh });
}

/// Lateral and spin intensities of one wheel (the skid sound rule of `FUN_009d1bf0`, per wheel).
fn intensities(slip_angle_deg: f32, speed_mph: f32, tread_slip_mph: f32) -> (f32, f32) {
    let lat = ((slip_angle_deg.abs() - 0.05) / (40.0 - 0.05)).clamp(0.0, 1.0) * (speed_mph / 10.0).clamp(0.0, 1.0);
    let spin = ((tread_slip_mph.abs() - 2.0) / (20.0 - 2.0)).clamp(0.0, 1.0);
    (lat, spin)
}

fn lay_marks(time: Res<Time>, cars: Query<&Car>, mut marks: ResMut<Marks>) {
    let now = time.elapsed_secs();
    let marks = &mut *marks;
    // The driven car (the first one).
    if let Some(car) = cars.iter().next() {
        let wheels: Vec<_> = car.wheel_contacts().zip(car.wheel_slips()).collect();
        marks.strips.resize(wheels.len(), None);
        for (i, (contact, (_ratio, angle))) in wheels.into_iter().enumerate() {
            let Some(c) = contact else {
                marks.strips[i] = None;
                continue;
            };
            let speed = c.velocity.length() * MS_TO_MPH;
            let (lat, spin) = intensities(angle, speed, c.tread_slip * MS_TO_MPH);
            let opacity = lat.max(spin);
            if opacity < MIN_INTENSITY || c.velocity.length() < 0.1 {
                marks.strips[i] = None;
                continue;
            }
            let along = c.velocity.normalize();
            let side = c.normal.cross(along).normalize_or_zero();
            let mut p = Point { pos: c.point, normal: c.normal, side, u: 0.0, slip: lat, spin, opacity };
            match marks.strips[i] {
                None => marks.strips[i] = Some(p),
                Some(last) => {
                    let d = p.pos.distance(last.pos);
                    if d >= SEGMENT {
                        p.u = last.u + d / U_LENGTH;
                        marks.segments.push_back(Segment { a: last, b: p, born: now });
                        while marks.segments.len() > MAX_SEGMENTS {
                            marks.segments.pop_front();
                        }
                        marks.strips[i] = Some(p);
                    }
                }
            }
        }
    }
    while marks.segments.front().is_some_and(|s| now - s.born > LIFETIME) {
        marks.segments.pop_front();
    }
    if marks.log && now - marks.last_log >= 1.0 {
        marks.last_log = now;
        info!("tyre marks: {} segments alive (limit {MAX_SEGMENTS}, lifetime {LIFETIME} s)", marks.segments.len());
    }
}

fn build_mesh(time: Res<Time>, marks: Res<Marks>, draw: Res<MarkDraw>, mut meshes: ResMut<Assets<Mesh>>) {
    let Some(mut mesh) = meshes.get_mut(&draw.mesh) else { return };
    if marks.segments.is_empty() {
        *mesh = empty_mesh();
        return;
    }
    let now = time.elapsed_secs();
    let n = marks.segments.len() * 4;
    let (mut pos, mut normal, mut uv, mut side, mut colour) = (
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    );
    let mut indices = Vec::with_capacity(marks.segments.len() * 6);
    for s in &marks.segments {
        let fade = (1.0 - (now - s.born) / LIFETIME).clamp(0.0, 1.0);
        let base = pos.len() as u32;
        for p in [s.a, s.b] {
            for v in [0.0, 1.0] {
                pos.push(p.pos.to_array());
                normal.push(p.normal.to_array());
                uv.push([p.u, v]);
                side.push(p.side.extend(0.0).to_array());
                colour.push([p.slip, p.spin, v, p.opacity * fade]);
            }
        }
        indices.extend([base, base + 1, base + 2, base + 1, base + 3, base + 2]);
    }
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normal);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    m.insert_attribute(Mesh::ATTRIBUTE_TANGENT, side);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, colour);
    m.insert_indices(Indices::U32(indices));
    *mesh = m;
}

fn empty_mesh() -> Mesh {
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 3]);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; 3]);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; 3]);
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
