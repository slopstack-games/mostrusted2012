//! Particle effects: the game's `BearEffect`s (`mw_formats::bear`) simulated on the CPU and drawn as
//! translucent forward geometry with ports of the game's particle shaders.
//!
//! Like the game's Bear runtime, each effect instance has one particle pool per layer and runs the
//! layer's action groups in their stored order: group 0 (emission controller, per-particle updates,
//! spawners with their initialisers) once per step, group 1 (size and colour curves, fades, renderer)
//! when building the frame's vertices. Each renderer writes into a batch per material: camera-facing
//! quads (`particles.wgsl`, the `VfxParticles_Diffuse*` family) or motion-blurred sparks
//! (`sparks.wgsl`, `VfxParticles_MotionBlurSpriteUntex`).
//!
//! Which effect plays where comes from the game's tables (`mw_assets::vfx`): tyre smoke from the
//! `PlayerWheelEffects.DefaultTyre` of the surface under each wheel (the collision tag's surface) while the
//! wheel's smoke ramps are up (`wheel_emitters`), that surface's `WheelVfx` kick-up behind rolling wheels, and
//! the default surface's directional impact effects (`FrontImpactEffect`, ...) for collisions of the car body
//! (sparks; side contacts scrape continuously; the surface of the body's contact is not looked up yet).
//!
//! What the game feeds its effect parameters is code, so the inputs here are stand-ins: `Intensity` is
//! the tyre's sliding speed / 15 m/s (or the impact speed / 15 m/s), `Velocity` the car's velocity at the
//! contact, `Transform` the contact point with the car's orientation (impacts: Y along the contact
//! normal). The least certain action semantics are noted in `mw_formats::bear::Op`.

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::sync::{Arc, Mutex, mpsc};

use avian3d::prelude::{Collisions, PhysicsSystems};
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
use mw_assets::vfx::{ParticleMaterial as GameMaterial, VfxLibrary};
use mw_bundle::ResourceId;
use mw_formats::bear::{BearEffect, Bindings, ConeEmitter, Layer, Op, Renderer, Spawner};

use crate::drive::Car;

#[path = "particle_mesh.rs"]
mod mesh;
pub(crate) use mesh::{CollisionMeshes, MeshContact};

pub struct ParticlesPlugin;

impl Plugin for ParticlesPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "particles.wgsl");
        bevy::asset::embedded_asset!(app, "sparks.wgsl");
        app.add_plugins((MaterialPlugin::<QuadMaterial>::default(), MaterialPlugin::<SparkMaterial>::default()))
            .add_message::<ImpactEvent>()
            .init_resource::<Instances>()
            .init_resource::<BearGlobals>()
            .init_resource::<BearView>()
            .init_resource::<EffectRequests>()
            .init_resource::<CollisionMeshes>()
            .init_resource::<mesh::DebrisModels>()
            .insert_resource(VfxDebug::from_env())
            .add_systems(Update, (start_loading, receive_vfx))
            .add_systems(FixedPostUpdate, detect_impacts.after(PhysicsSystems::Writeback))
            .add_systems(
                Update,
                (
                    load_requested,
                    wheel_emitters,
                    impact_emitters,
                    prop_impact_effects,
                    demo_emitter,
                    simulate,
                    build_meshes,
                    mesh::build_debris,
                )
                    .chain()
                    .after(receive_vfx)
                    .run_if(resource_exists::<Vfx>),
            );
    }
}

/// A collision of a car's body, emitted after each physics step for every touching contact.
///
/// Shared event for effects, sound and damage: other branches (crash damage in `damage.rs`, impact
/// sounds) may define their own; unify them on merge. `normal` points out of the obstacle (towards the
/// car); `impulse` is the step's total normal impulse (N·s); `closing_speed` the approach speed along the
/// normal before the solver (m/s, positive when approaching); `sliding_speed` the tangential speed of the
/// contact (m/s). The world's colliders are one trimesh per track unit and stay "touching" (speculative
/// contacts) while the car is near them, so a new impact shows as a high closing speed, not as a new pair.
#[derive(Message, Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct ImpactEvent {
    pub car: Entity,
    /// The other body of the pair (`None`: a collider without a body, the world).
    pub other: Option<Entity>,
    pub position: Vec3,
    pub normal: Vec3,
    pub impulse: f32,
    pub closing_speed: f32,
    pub sliding_speed: f32,
}

/// Debug switches from the environment: `MW_VFX_DEMO=<effect id>[,<seconds>[,<speed>]]` (see
/// [`demo_emitter`]) and `MW_VFX_DEBUG=1` (log live particle counts four times a second).
#[derive(Resource, Default)]
pub(crate) struct VfxDebug {
    demo: Option<(ResourceId, f32, f32)>,
    log: bool,
}

impl VfxDebug {
    fn from_env() -> Self {
        let demo = std::env::var("MW_VFX_DEMO").ok().and_then(|spec| {
            let mut parts = spec.split(',');
            let id = parts.next().and_then(ResourceId::parse)?;
            let every = parts.next().and_then(|s| s.parse().ok()).unwrap_or(2.0);
            let speed = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            Some((id, every, speed))
        });
        Self { demo, log: std::env::var_os("MW_VFX_DEBUG").is_some() }
    }
}

// ---------------------------------------------------------------------------------------------------
// Loading

struct VfxLoaded {
    lib: VfxLibrary,
    effects: Vec<BearEffect>,
    materials: Vec<(GameMaterial, Option<Image>)>,
}

#[derive(Resource)]
struct VfxLoading(Mutex<mpsc::Receiver<anyhow::Result<VfxLoaded>>>);

/// Effect definitions and render batches.
#[derive(Resource)]
pub(crate) struct Vfx {
    lib: VfxLibrary,
    effects: HashMap<ResourceId, Arc<BearEffect>>,
    batches: HashMap<ResourceId, Batch>,
}

impl Vfx {
    pub(crate) fn has_effect(&self, id: ResourceId) -> bool {
        self.effects.contains_key(&id)
    }

    pub(crate) fn lib(&self) -> &VfxLibrary {
        &self.lib
    }
}

struct Batch {
    mesh: Handle<Mesh>,
    spark: bool,
    quads: Vec<QuadVertex>,
    sparks: Vec<SparkVertex>,
    /// The mesh holds only the invisible placeholder (nothing drawn): an empty batch is not re-uploaded
    /// (each mesh change makes Bevy re-check every mesh entity, `mark_3d_meshes_as_changed_if_their_assets_changed`).
    placeholder: bool,
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
        .name("vfx loader".into())
        .spawn(move || {
            let _ = tx.send(load_vfx(&store, bc));
        })
        .expect("spawn vfx loader");
    commands.insert_resource(VfxLoading(Mutex::new(rx)));
}

/// Parse the effects the runtime plays and their materials and textures (loader thread).
fn load_vfx(store: &AssetStore, bc: bool) -> anyhow::Result<VfxLoaded> {
    let lib = store.vfx_library()?;
    let ids: Vec<ResourceId> = lib
        .surfaces
        .iter()
        .flat_map(|s| [&s.dry, &s.wet])
        .flat_map(|e| {
            [e.wheel, e.player_wheels.default_tyre, e.ai_wheels.default_tyre, e.player_wheels.on_rims]
                .into_iter()
                .chain(e.impacts)
                .chain(e.ai_impacts)
        })
        .chain(lib.vehicle_impacts.iter().copied())
        .chain(lib.vehicle_scrapes.iter().copied())
        .flatten()
        .collect();
    let (effects, materials) = load_effects(store, bc, ids);
    info!("vfx: {} effects, {} materials", effects.len(), materials.len());
    Ok(VfxLoaded { lib, effects, materials })
}

type LoadedMaterials = Vec<(GameMaterial, Option<Image>)>;

/// Parse effects and their materials and textures (loader thread).
fn load_effects(store: &AssetStore, bc: bool, mut ids: Vec<ResourceId>) -> (Vec<BearEffect>, LoadedMaterials) {
    ids.sort();
    ids.dedup();
    let mut effects = Vec::new();
    let mut material_ids = Vec::new();
    for id in ids {
        match store.bear_effect(id) {
            Ok(fx) => {
                material_ids.extend(fx.layers.iter().filter_map(|l| l.renderer().and_then(|r| r.material)));
                effects.push(fx);
            }
            Err(e) => warn!("effect {id}: {e}"),
        }
    }
    material_ids.sort();
    material_ids.dedup();
    let mut materials = Vec::new();
    for id in material_ids {
        // Mesh renderers import Models; only materials are drawn.
        if store.type_of(id) != Some(mw_bundle::ResourceType::Material) {
            continue;
        }
        match store.particle_material(id) {
            Ok(m) => {
                let image =
                    m.texture.and_then(|t| store.texture(t).ok()).and_then(|t| crate::convert::image(&t, true, bc));
                materials.push((m, image));
            }
            Err(e) => warn!("particle material {id}: {e}"),
        }
    }
    (effects, materials)
}

/// What a render batch needs from the world: the game's key light and sky for the lit quads.
#[derive(bevy::ecs::system::SystemParam)]
struct BatchMaker<'w, 's> {
    commands: Commands<'w, 's>,
    lighting: Option<Res<'w, crate::deferred::GameLighting>>,
    images: ResMut<'w, Assets<Image>>,
    meshes: ResMut<'w, Assets<Mesh>>,
    quad_materials: ResMut<'w, Assets<QuadMaterial>>,
    spark_materials: ResMut<'w, Assets<SparkMaterial>>,
}

impl BatchMaker<'_, '_> {
    /// One render batch (an entity with a mesh rebuilt every frame) per material not batched yet.
    fn add(&mut self, materials: LoadedMaterials, batches: &mut HashMap<ResourceId, Batch>) {
        let (key_colour, key_direction, sky, irradiance) = match &self.lighting {
            Some(l) => (l.key_colour, l.key_direction, l.sky_brightness, Some(l.irradiance.clone())),
            None => (Vec3::splat(9000.0), Vec3::new(-0.4, -0.8, -0.4).normalize(), 1200.0, None),
        };
        for (m, image) in materials {
            if batches.contains_key(&m.id) {
                continue;
            }
            debug!(
                "particle material {}: {} tiles {:?} looping {} overbright {}",
                m.id, m.shader, m.tiles, m.looping, m.overbright
            );
            let mesh = self.meshes.add(empty_mesh());
            let spark = m.shader.contains("MotionBlurSprite");
            let mut e = self.commands.spawn((
                Mesh3d(mesh.clone()),
                Transform::IDENTITY,
                Visibility::default(),
                NoFrustumCulling,
            ));
            if spark {
                let scale = m.overbright * SPARK_BRIGHTNESS;
                e.insert(MeshMaterial3d(
                    self.spark_materials
                        .add(SparkMaterial { params: SparkParams { scale: Vec4::new(scale, 0.0, 0.0, 0.0) } }),
                ));
            } else {
                let blend = if m.shader.contains("NoBlend") { 0.0 } else { 1.0 };
                e.insert(MeshMaterial3d(self.quad_materials.add(QuadMaterial {
                    params: QuadParams {
                        atlas: Vec4::new(
                            m.tiles[0] as f32,
                            m.tiles[1] as f32,
                            if m.looping { 1.0 } else { 0.0 },
                            blend,
                        ),
                        key_colour: key_colour.extend(sky),
                        // w: the pixel program alpha-tests (all but the `NoAlphaTest` variants).
                        key_direction: key_direction.extend(if m.shader.contains("NoAlphaTest") { 0.0 } else { 1.0 }),
                    },
                    texture: image.map(|i| self.images.add(i)),
                    irradiance: irradiance.clone(),
                })));
            }
            batches.insert(m.id, Batch { mesh, spark, quads: Vec::new(), sparks: Vec::new(), placeholder: false });
        }
    }
}

fn receive_vfx(mut maker: BatchMaker, loading: Option<Res<VfxLoading>>) {
    let Some(loading) = loading else { return };
    let msg = match loading.0.lock().unwrap().try_recv() {
        Ok(m) => m,
        Err(mpsc::TryRecvError::Empty) => return,
        Err(mpsc::TryRecvError::Disconnected) => {
            maker.commands.remove_resource::<VfxLoading>();
            return;
        }
    };
    maker.commands.remove_resource::<VfxLoading>();
    let loaded = match msg {
        Ok(l) => l,
        Err(e) => {
            warn!("no particle effects: {e}");
            return;
        }
    };
    let mut batches = HashMap::new();
    maker.add(loaded.materials, &mut batches);
    let effects = loaded.effects.into_iter().map(|fx| (fx.id, Arc::new(fx))).collect();
    maker.commands.insert_resource(Vfx { lib: loaded.lib, effects, batches });
}

/// Loaded effects and their materials, sent back from the worker thread.
type LoadedEffects = (Vec<BearEffect>, LoadedMaterials);

/// Effects asked for after the start (the cars' spot effects, `exhaust.rs`): loaded on a worker thread
/// and added to [`Vfx`].
#[derive(Resource, Default)]
pub(crate) struct EffectRequests {
    asked: std::collections::HashSet<ResourceId>,
    want: Vec<ResourceId>,
    pending: Option<Mutex<mpsc::Receiver<LoadedEffects>>>,
}

impl EffectRequests {
    /// Load an effect unless it was asked for before.
    pub(crate) fn request(&mut self, id: ResourceId) {
        if self.asked.insert(id) {
            self.want.push(id);
        }
    }
}

fn load_requested(
    game: Res<crate::Game>,
    support: Option<Res<CompressedImageFormatSupport>>,
    mut requests: ResMut<EffectRequests>,
    mut vfx: ResMut<Vfx>,
    mut maker: BatchMaker,
) {
    if let Some(rx) = &requests.pending {
        let got = rx.lock().unwrap().try_recv();
        match got {
            Ok((effects, materials)) => {
                info!("vfx: {} more effects, {} materials", effects.len(), materials.len());
                maker.add(materials, &mut vfx.batches);
                for fx in effects {
                    vfx.effects.insert(fx.id, Arc::new(fx));
                }
                requests.pending = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => requests.pending = None,
            Err(mpsc::TryRecvError::Empty) => return,
        }
    }
    if requests.want.is_empty() {
        return;
    }
    let ids: Vec<ResourceId> =
        std::mem::take(&mut requests.want).into_iter().filter(|id| !vfx.effects.contains_key(id)).collect();
    if ids.is_empty() {
        return;
    }
    let bc = support.is_some_and(|s| s.0.contains(CompressedImageFormats::BC));
    let store = game.store.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("vfx loader (more)".into())
        .spawn(move || {
            let _ = tx.send(load_effects(&store, bc, ids));
        })
        .expect("spawn vfx loader");
    requests.pending = Some(Mutex::new(rx));
}

// ---------------------------------------------------------------------------------------------------
// Simulation

/// How long a one-shot effect's continuous spawners run (seconds); the game keeps an impact effect
/// emitting while the contact lasts.
const ONE_SHOT_EMIT: f32 = 0.15;

/// Persistent emitters are keyed by owner and slot (wheel smoke, wheel kick-up, scrape, car spot effects).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct EmitterKey {
    pub owner: Entity,
    pub slot: u32,
}

const SLOT_TYRE: u32 = 0;
const SLOT_KICKUP: u32 = 0x100;
/// Car collision effects (`crash_vfx.rs`): this + the slot (0..4).
pub(crate) const SLOT_CRASH: u32 = 0x200;
/// Car spot effects (`exhaust.rs`): this + 0x10 × effect index + locator index.
pub(crate) const SLOT_SPOT: u32 = 0x1000;

/// The parameters the game sets on an effect instance (stand-ins, see the module docs).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Params {
    /// World from emitter space (origin at the emitter, the car's axes; impacts: Y = contact normal).
    pub transform: Mat4,
    pub velocity: Vec3,
    pub intensity: f32,
    pub emitting: bool,
    /// Metres the emitter moved (or slid) this step, for per-metre emission.
    pub moved: f32,
    /// Ground plane under the emitter (normal, -distance), for bouncing particles.
    pub ground: Vec4,
    /// `ImpactClosingVelocity` and `MovementSinceLastFrame` (binding sources 17 / 18).
    pub impact: f32,
    pub movement: f32,
    /// `CollisionMeshParams`: the car's collision mesh and the contact in its space (`EmitterCollisionMesh`).
    pub mesh: Option<MeshContact>,
    /// `VehicleParams` +0x10: the car's paint colour (linear rgb), for `RenderMesh` (0x22) debris.
    pub paint: Vec3,
    /// The wheel's smoke ramps (spin, lateral, lock; `mw_handling::tyre_vfx`) for `UpdateWheelBindings`
    /// sources 0x0F..0x11 (see [`wheel_source`]).
    pub wheel: [f32; 3],
}

impl Default for Params {
    fn default() -> Self {
        Self {
            transform: Mat4::IDENTITY,
            velocity: Vec3::ZERO,
            intensity: 0.0,
            emitting: false,
            moved: 0.0,
            ground: Vec4::new(0.0, 1.0, 0.0, f32::MAX),
            impact: 0.0,
            movement: 0.0,
            mesh: None,
            paint: Vec3::ZERO,
            wheel: [0.0; 3],
        }
    }
}

/// The parts of the game's Bear view block (`FUN_0046adb0` sets it up, `FUN_004a1470` refreshes it each frame)
/// that the render actions read: the camera's forward axis and its speed along it (+0x21c, `|Δposition ·
/// forward| / dt`, which stretches `UpdateParametricFadeOverDistance`'s near fade). The block's +0x218 (depth
/// origin offset, 0), +0x220 (size cap scale, 1.0) and +0x224 (saturation, 1.0) keep their initial values here.
#[derive(Resource, Default, Clone, Copy)]
pub(crate) struct BearView {
    pub eye: Vec3,
    pub forward: Vec3,
    pub speed: f32,
    prev: Option<Vec3>,
}

/// The game's `BearGlobalParameters` the binding controllers read (`FUN_004a1470` fills them each frame).
/// Only the one the car effects use is known: +0x74, the player car's engine load (vehicle +0x1f20 through
/// `FUN_0090d6d0`). Set by `exhaust.rs`.
#[derive(Resource, Default, Clone, Copy)]
pub(crate) struct BearGlobals {
    pub player_load: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Particle {
    pos: Vec3,
    vel: Vec3,
    /// Normalised age 0..1 (`Age`) and 1 / lifetime (`RecipLifetime`).
    age: f32,
    recip_life: f32,
    radius: f32,
    rot: f32,
    spin: f32,
    alpha: f32,
    seed: u32,
    plane: Vec4,
    /// `Rotation` as a quaternion and `AngularVelocity` (rad/s, world) of the 3-D layers (0x0B / 0x2D).
    quat: Quat,
    omega: Vec3,
    /// `CollisionRadius` of the debris that collides with the world (0x40); negative once at rest.
    cr: f32,
    /// The particle's number in its layer (the mesh renderers pick one of a model's 16 pieces by it).
    serial: u32,
}

struct LayerSim {
    particles: Vec<Particle>,
    /// Fractional particles carried to the next step.
    carry: f32,
    /// Emission scale from the layer's controller (spawners other than `EmitterConePlus`: the first
    /// curve over `Intensity`, a stand-in).
    rate_scale: f32,
    /// The binding slots the controllers write (`mw_formats::bear::Bindings`; slot 0 reads 1).
    slots: [f32; Bindings::SLOTS],
    /// Seconds since the layer started (the controllers' clock) and the random phases of source 15.
    time: f32,
    phase: [f32; 4],
    /// `EmitterConePlus`: its own clock and particle accumulator (`burst + 0.9999` at the start).
    cone_time: f32,
    accum: f32,
    /// `EmitterCollisionMesh`: its clock, whether the burst is spent and the carried fraction (0.9999 at the start).
    mesh_clock: f32,
    mesh_burst_spent: bool,
    mesh_carry: f32,
    /// Particles born so far (their serials).
    spawned: u32,
}

impl LayerSim {
    /// A binding slot (out of range: 0).
    fn slot(&self, i: u8) -> f32 {
        self.slots.get(i as usize).copied().unwrap_or(0.0)
    }
}

/// One `UpdateBindings` run (`FUN_0045ffd0`): each curve over its source, then the combine steps.
/// `UpdateBindings_Wheel` (0x34, `FUN_004601a0`) adds sources 0x14..0x18 over its `WheelParams` parameter W:
/// W[1], W[2], W[3], `clamp(|W[0]| / s)` and the flag byte at W +0x30. The per-wheel update fills W with
/// (`uStack_4c0`, the three smoke ramps) (`FUN_00589e20`, block +0x80): 0x14..0x16 read spin, lateral, lock here;
/// W[0] and the flag are not identified and read 0.
#[allow(clippy::too_many_arguments)]
fn run_bindings(
    b: &Bindings,
    curves: &[mw_formats::bear::Curve5; 4],
    scales: &[f32; 4],
    sim: &mut LayerSim,
    p: &Params,
    globals: &BearGlobals,
    camera_distance: f32,
    kind: u8,
) {
    let t = sim.time;
    for i in 0..4 {
        let s = scales[i];
        let frac = |x: f32| x - x.floor();
        let ratio = |v: f32| if s != 0.0 { v / s } else { 0.0 };
        let x = match b.sources[i] {
            0 => s * t,
            1 => frac(s * t),
            3 => s * p.velocity.length(),
            4 => s * camera_distance,
            7 => 0.5 + 0.5 * (s * t * TAU).sin(),
            8 => 0.5 + 0.5 * (s * t * TAU).cos(),
            // `FUN_0045fc60` replaces source 9's scale with a random number when it compiles the action.
            9 => sim.phase[i],
            10 => p.intensity,
            15 => frac(s * (t + sim.phase[i])),
            16 => globals.player_load,
            17 => ratio(p.impact),
            18 => ratio(p.movement),
            k @ 0x14..=0x16 if kind == 0x34 => p.wheel[(k - 0x14) as usize],
            // Other globals (time of day, weather and quality bytes, ...): not traced; byte +0x7d is 0.
            _ => 0.0,
        };
        sim.slots[1 + i] = curves[i].eval(x) + b.offsets[i];
    }
    b.combine(&mut sim.slots);
}

/// One `UpdateWheelBindings` run (0x42, `FUN_00463330`): each curve over its source (no offsets), into slots
/// 1..=4, then the combine steps into 5..=8. Sources (`s` = the curve's scale, `t` the layer's clock):
///
/// | source | input |
/// |---|---|
/// | 0 / 1 | `s·t` / `frac(s·t)` |
/// | 3 | `s·|Velocity|` |
/// | 4 | `s·` distance from the transform to the camera |
/// | 7 / 8 | `0.5 + 0.5·sin / cos(2π·s·t)` |
/// | 9 | `s` |
/// | 0x0E..0x11 | the `WheelContact` parameter's +0x40..+0x4c ([`wheel_source`]) |
/// | 0x12 | `clamp(|WheelContact +0x1c| / s)` (not identified: 0) |
/// | 2, 5, 6, 10..13, 0x13 | view-block globals (+0x68, +0x60, byte +0x7c, +0x6c, +0x7d, +0x70, +0x7e, +0x78; 0 here) |
fn run_wheel_bindings(
    b: &Bindings,
    curves: &[mw_formats::bear::Curve5; 4],
    scales: &[f32; 4],
    sim: &mut LayerSim,
    p: &Params,
    camera_distance: f32,
) {
    let t = sim.time;
    for i in 0..4 {
        let s = scales[i];
        let frac = |x: f32| x - x.floor();
        let x = match b.sources[i] {
            0 => s * t,
            1 => frac(s * t),
            3 => s * p.velocity.length(),
            4 => s * camera_distance,
            7 => 0.5 + 0.5 * (s * t * TAU).sin(),
            8 => 0.5 + 0.5 * (s * t * TAU).cos(),
            9 => s,
            k @ 0x0e..=0x11 => wheel_source(p, k),
            _ => 0.0,
        };
        sim.slots[1 + i] = curves[i].eval(x);
    }
    b.combine(&mut sim.slots);
}

/// `UpdateWheelBindings` sources 0x0E..0x11 read the `WheelContact` parameter's +0x40..+0x4c. Model
/// decision: they are taken as (0, spin, lateral, lock), the wheel's smoke ramps. The block the per-wheel
/// update fills (`FUN_00589e20` → `FUN_004acd80` / `FUN_0059bb50`) holds (`fStack_4b0` = 0, the three ramps) one
/// row earlier, at +0x30, and the wheel's velocity at +0x40; how the Bear runtime copies the block into the
/// effect's `WheelContact` was not found. The data favours the ramps: over the 34 `UpdateWheelBindings`
/// actions sources 0x0F..0x11 are common and 0x0E (which would be the constant 0 or the world X velocity)
/// never appears.
fn wheel_source(p: &Params, source: u8) -> f32 {
    match source {
        0x0f => p.wheel[0],
        0x10 => p.wheel[1],
        0x11 => p.wheel[2],
        _ => 0.0,
    }
}

/// `EmitterConePlus` (`FUN_0044cf30`): how many particles this update emits, or `None` when it does not.
fn cone_count(
    sp: &Spawner,
    c: &ConeEmitter,
    sim: &mut LayerSim,
    p: &Params,
    lod: f32,
    camera_distance: f32,
    dt: f32,
) -> Option<usize> {
    sim.cone_time += dt;
    if !p.emitting && c.emitting != mw_formats::bear::Operand::None {
        return None;
    }
    let t = sim.cone_time;
    if t < c.time_window[0] || (t > c.time_window[1] && sim.accum <= 1.0) {
        return None;
    }
    let speed = p.velocity.length();
    if speed < c.speed_window[0] || speed > c.speed_window[1] {
        return None;
    }
    // The camera window compares the squared distance.
    let d2 = camera_distance * camera_distance;
    if d2 < c.camera_window[0] || d2 > c.camera_window[1] {
        return None;
    }
    let count = (sp.per_metre * dt * speed + sp.rate * dt) * sim.slot(c.state[0]) * lod;
    if sim.accum > 1.0 {
        sim.accum *= sim.slot(c.state[1]) * lod;
    }
    sim.accum += count;
    let n = sim.accum.round();
    if n < 1.0 {
        return None;
    }
    sim.accum -= n;
    Some(n as usize)
}

struct EffectInstance {
    def: Arc<BearEffect>,
    key: Option<EmitterKey>,
    params: Params,
    layers: Vec<LayerSim>,
    /// One-shot effects fire their bursts once,
    burst_done: bool,
    /// and their continuous spawners for this long (seconds).
    emit_for: f32,
    rng: Rng,
}

impl EffectInstance {
    fn new(def: Arc<BearEffect>, key: Option<EmitterKey>, params: Params, seed: u32) -> Self {
        let mut rng = Rng(seed | 1);
        let layers = def
            .layers
            .iter()
            .map(|l| {
                let mut slots = [0.0; Bindings::SLOTS];
                slots[0] = 1.0;
                // `FUN_0044ccc0`: the accumulator starts at the burst count + 0.9999, so the first update
                // that emits rounds up to at least one particle.
                let burst = l.spawner().filter(|s| s.cone.is_some()).map_or(0.0, |s| s.burst as f32);
                LayerSim {
                    particles: Vec::new(),
                    carry: 0.0,
                    rate_scale: 1.0,
                    slots,
                    time: 0.0,
                    phase: std::array::from_fn(|_| rng.next()),
                    cone_time: 0.0,
                    accum: burst + 0.9999,
                    mesh_clock: 0.0,
                    mesh_burst_spent: false,
                    mesh_carry: mesh::CARRY_START,
                    spawned: 0,
                }
            })
            .collect();
        Self { def, key, params, layers, burst_done: false, emit_for: 0.0, rng }
    }

    fn alive(&self) -> bool {
        self.layers.iter().any(|l| !l.particles.is_empty()) || !self.burst_done || self.emit_for > 0.0
    }
}

#[derive(Resource, Default)]
pub(crate) struct Instances {
    list: Vec<EffectInstance>,
    seed: u32,
}

impl Instances {
    /// Refresh a persistent emitter, or start it when `start` is set and it emits (car spot effects).
    pub(crate) fn refresh_or_start(
        &mut self,
        vfx: &Vfx,
        key: EmitterKey,
        effect: ResourceId,
        params: Params,
        start: bool,
    ) {
        let Some(def) = vfx.effects.get(&effect) else { return };
        if let Some(inst) = self.list.iter_mut().find(|i| i.key == Some(key) && Arc::ptr_eq(&i.def, def)) {
            inst.params = params;
            return;
        }
        if start && params.emitting {
            self.emit(vfx, key, Some(effect), params);
        }
    }

    /// Refresh (or start) a persistent emitter.
    pub(crate) fn emit(&mut self, vfx: &Vfx, key: EmitterKey, effect: Option<ResourceId>, params: Params) {
        let def = effect.and_then(|id| vfx.effects.get(&id));
        // A slot that now plays another effect (or none: a wheel onto a surface without one) stops the old one;
        // its particles live on.
        for inst in self.list.iter_mut().filter(|i| i.key == Some(key)) {
            if !def.is_some_and(|d| Arc::ptr_eq(&inst.def, d)) {
                inst.params.emitting = false;
            }
        }
        let Some(def) = def else { return };
        if let Some(inst) = self.list.iter_mut().find(|i| i.key == Some(key) && Arc::ptr_eq(&i.def, def)) {
            inst.params = params;
            return;
        }
        if !params.emitting {
            return;
        }
        self.seed = self.seed.wrapping_mul(747796405).wrapping_add(2891336453);
        let mut inst = EffectInstance::new(def.clone(), Some(key), params, self.seed);
        inst.burst_done = true;
        self.list.push(inst);
    }

    /// Start `effect` afresh on `key` (bursts included), letting whatever played there live on unowned.
    pub(crate) fn restart(&mut self, vfx: &Vfx, key: EmitterKey, effect: ResourceId, params: Params) -> bool {
        let Some(def) = vfx.effects.get(&effect) else { return false };
        self.release(key);
        self.seed = self.seed.wrapping_mul(747796405).wrapping_add(2891336453);
        self.list.push(EffectInstance::new(def.clone(), Some(key), params, self.seed));
        true
    }

    /// Stop the emission of the instance on `key` and give up the key (its particles live on).
    pub(crate) fn release(&mut self, key: EmitterKey) {
        for inst in self.list.iter_mut().filter(|i| i.key == Some(key)) {
            inst.params.emitting = false;
            inst.key = None;
            inst.burst_done = true;
            inst.emit_for = 0.0;
        }
    }

    /// The parameters of the live instance on `key`, if any.
    pub(crate) fn keyed_mut(&mut self, key: EmitterKey) -> Option<&mut Params> {
        self.list.iter_mut().find(|i| i.key == Some(key)).map(|i| &mut i.params)
    }

    pub(crate) fn one_shot(&mut self, vfx: &Vfx, effect: Option<ResourceId>, params: Params) {
        let Some(def) = effect.and_then(|id| vfx.effects.get(&id)) else { return };
        if self.list.len() > 64 {
            return;
        }
        self.seed = self.seed.wrapping_mul(747796405).wrapping_add(2891336453);
        let mut inst = EffectInstance::new(def.clone(), None, Params { emitting: true, ..params }, self.seed);
        inst.emit_for = ONE_SHOT_EMIT;
        self.list.push(inst);
    }
}

#[derive(Clone, Copy)]
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        // xorshift32
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }
    fn range(&mut self, r: [f32; 2]) -> f32 {
        r[0] + (r[1] - r[0]) * self.next()
    }
}

fn hash01(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9);
    x ^= x >> 15;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

pub(crate) fn simulate(
    time: Res<Time>,
    debug: Res<VfxDebug>,
    globals: Res<BearGlobals>,
    meshes: Res<CollisionMeshes>,
    spatial: avian3d::prelude::SpatialQuery,
    mut instances: ResMut<Instances>,
    mut view: ResMut<BearView>,
    camera: Query<&GlobalTransform, crate::MainCamera>,
) {
    let dt = time.delta_secs().min(0.1);
    // The debris' world collision (0x40): the game tests a cache of the world triangles around the player's car;
    // a ray against the world's colliders stands in for it (front faces only, as the game's test).
    let ground = avian3d::prelude::SpatialQueryFilter::default().with_mask(crate::props::GameLayer::World);
    let world = |from: Vec3, to: Vec3| -> Option<(f32, Vec3)> {
        let d = to - from;
        let len = d.length();
        let dir = Dir3::new(d).ok()?;
        let hit = spatial.cast_ray(from, dir, len, false, &ground)?;
        (hit.normal.dot(d) < 0.0).then_some((hit.distance / len, hit.normal))
    };
    if dt <= 0.0 {
        return;
    }
    let eye = camera.iter().next().map(|t| t.translation()).unwrap_or_default();
    if let Some(cam) = camera.iter().next() {
        // `FUN_004a1470`: view +0x21c = |(position − last position) · forward| / dt.
        let forward = cam.forward().as_vec3();
        view.speed = view.prev.map_or(0.0, |p| ((eye - p).dot(forward) / dt).abs());
        view.prev = Some(eye);
        view.eye = eye;
        view.forward = forward;
    }
    for inst in &mut instances.list {
        let def = inst.def.clone();
        for (i, layer) in def.layers.iter().enumerate() {
            // A `SuperEmitter` layer reads the particles of its parent layer (an earlier one, already updated).
            let parent = layer
                .super_emitter_parent()
                .filter(|&j| j != i)
                .and_then(|j| inst.layers.get(j))
                .map(|l| l.particles.clone());
            let sim = &mut inst.layers[i];
            step_layer(
                layer,
                sim,
                &inst.params,
                &globals,
                &meshes,
                &world,
                inst.burst_done,
                eye,
                dt,
                &mut inst.rng,
                parent.as_deref(),
            );
        }
        inst.burst_done = true;
        if inst.key.is_some() {
            // Persistent emitters emit while their owner refreshes them every frame.
            inst.params.emitting = false;
            inst.params.moved = 0.0;
        } else {
            inst.emit_for -= dt;
            inst.params.emitting &= inst.emit_for > 0.0;
        }
    }
    instances.list.retain(|i| i.alive());
    if debug.log && (time.elapsed_secs() * 4.0).fract() < dt * 4.0 {
        for inst in &instances.list {
            let counts: Vec<usize> = inst.layers.iter().map(|l| l.particles.len()).collect();
            info!(
                "effect {} {:?}: particles {counts:?} intensity {:.2}",
                inst.def.id,
                inst.key.map(|k| k.slot),
                inst.params.intensity
            );
        }
    }
}

/// Run a layer's update group: controller, per-particle actions and spawners, in stored order.
#[allow(clippy::too_many_arguments)]
fn step_layer(
    layer: &Layer,
    sim: &mut LayerSim,
    p: &Params,
    globals: &BearGlobals,
    meshes: &CollisionMeshes,
    world: mesh::WorldCast,
    burst_done: bool,
    eye: Vec3,
    dt: f32,
    rng: &mut Rng,
    parent: Option<&[Particle]>,
) {
    let Some(group) = &layer.groups[0] else { return };
    let origin = p.transform.w_axis.truncate();
    let camera_distance = origin.distance(eye);
    for seq in &group.sequences {
        for step in &seq.steps {
            let Some(action) = layer.actions.get(step.action as usize) else { continue };
            match &action.op {
                Op::Controller { curves, scales, bindings, .. } => {
                    // Spawners other than the cone emitter and 0x07: the first curve over the effect's
                    // intensity (their binding-slot bytes are not decoded).
                    sim.rate_scale = curves[0].eval(p.intensity.clamp(0.0, 1.0));
                    if let Some(b) = bindings {
                        if action.kind == 0x42 {
                            run_wheel_bindings(b, curves, scales, sim, p, camera_distance);
                        } else {
                            run_bindings(b, curves, scales, sim, p, globals, camera_distance, action.kind);
                        }
                    }
                }
                Op::Age { .. } => {
                    for q in &mut sim.particles {
                        q.age += dt * q.recip_life;
                    }
                }
                Op::Rotation2d { damping, .. } => {
                    for q in &mut sim.particles {
                        q.rot += q.spin * TAU * dt;
                        q.spin *= (1.0 - damping * dt).max(0.0);
                    }
                }
                Op::Integrate { gravity, drag, .. } => {
                    for q in &mut sim.particles {
                        integrate(q, *gravity, *drag, dt);
                    }
                }
                Op::Bounce { gravity, drag, friction, restitution, .. } => {
                    for q in &mut sim.particles {
                        integrate(q, *gravity, *drag, dt);
                        // Collide with the bounce plane a quarter radius up, so big smoke sprites stay
                        // mostly above the ground (a guess; the game's plane offset is not decoded).
                        let n = q.plane.truncate();
                        let d = n.dot(q.pos) + q.plane.w - q.radius * 0.25;
                        if q.plane.w != f32::MAX && d < 0.0 {
                            q.pos -= n * d;
                            let vn = q.vel.dot(n);
                            if vn < 0.0 {
                                let tangent = q.vel - n * vn;
                                let keep = 1.0 - rng.range(*friction).clamp(0.0, 1.0);
                                q.vel = tangent * keep - n * vn * rng.range(*restitution);
                            }
                        }
                    }
                }
                Op::Spawn(sp) => match (&sp.mesh, p.mesh.and_then(|m| Some((m, meshes.get(m.mesh)?)))) {
                    (Some(me), Some((contact, tris))) => {
                        mesh::spawn(layer, sim, sp, me, &step.secondaries, p, contact, tris, eye, dt, rng);
                    }
                    _ => spawn(layer, sim, sp, &step.secondaries, p, burst_done, eye, dt, rng),
                },
                op @ Op::WorldCollide { .. } => mesh::world_collide(op, &mut sim.particles, dt, rng, world),
                Op::Rotation3d { damping, .. } => {
                    for q in &mut sim.particles {
                        mesh::rotate(q, *damping, dt);
                    }
                }
                op @ Op::SuperEmitter { .. } => {
                    if let Some(parent) = parent {
                        super_emit(layer, sim, op, &step.secondaries, parent, p, dt, rng);
                    }
                }
                _ => {}
            }
        }
    }
    // The render group's controllers (`ParametricSetColourGradient` reads their slots for the alpha).
    for seq in layer.groups[1].iter().flat_map(|g| &g.sequences) {
        for step in &seq.steps {
            let Some(action) = layer.actions.get(step.action as usize) else { continue };
            if let Op::Controller { curves, scales, bindings: Some(b), .. } = &action.op {
                if action.kind == 0x42 {
                    run_wheel_bindings(b, curves, scales, sim, p, camera_distance);
                } else {
                    run_bindings(b, curves, scales, sim, p, globals, camera_distance, action.kind);
                }
            }
        }
    }
    sim.time += dt;
    sim.particles.retain(|q| q.age < 1.0);
}

fn integrate(q: &mut Particle, gravity: f32, drag: f32, dt: f32) {
    q.vel.y += gravity * dt;
    // Quadratic drag, implicit (a guess at the coefficient's meaning: smoke stops within metres).
    q.vel /= 1.0 + drag * q.vel.length() * dt;
    q.pos += q.vel * dt;
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    layer: &Layer,
    sim: &mut LayerSim,
    sp: &Spawner,
    secondaries: &[u8],
    p: &Params,
    burst_done: bool,
    eye: Vec3,
    dt: f32,
    rng: &mut Rng,
) {
    let origin = p.transform.w_axis.truncate();
    let lod = if sp.lod[1] > sp.lod[0] {
        1.0 - ((origin.distance(eye) - sp.lod[0]) / (sp.lod[1] - sp.lod[0])).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let mut count = sim.carry;
    if p.emitting {
        // `EmitterWheelContactPlus` (0x07, `FUN_00450760`) scales its count by a binding slot (the layer's
        // `UpdateWheelBindings` output); the other spawners by the controller stand-in.
        let scale = sp.count_slot.map_or(sim.rate_scale, |k| sim.slot(k));
        count += (sp.rate * dt + sp.per_metre * p.moved) * scale * lod;
    }
    if !burst_done && sp.burst > 0 {
        count += sp.burst as f32 * p.intensity.clamp(0.1, 1.0) * lod;
    }
    let mut n = count.floor() as usize;
    sim.carry = if p.emitting { count - n as f32 } else { 0.0 };
    let (mut speed_scale, mut angle_scale) = (1.0, 1.0);
    if let Some(c) = &sp.cone {
        sim.carry = 0.0;
        let Some(k) = cone_count(sp, c, sim, p, lod, origin.distance(eye), dt) else { return };
        n = k;
        speed_scale = sim.slot(c.state[2]);
        angle_scale = sim.slot(c.state[3]);
    }
    let overwrite = layer.flags & 2 != 0;
    // Layers that live in the emitter's space (`Op::ToWorld`) keep their particles there.
    let local = layer.to_world().is_some();
    for i in 0..n {
        if sim.particles.len() >= layer.max_particles as usize {
            if !overwrite {
                break;
            }
            sim.particles.remove(0);
        }
        // Spread births over the step so streams stay continuous.
        let sub = (i as f32 + rng.next()) / n.max(1) as f32;
        let (pos, vel) = local_birth(sp, rng, speed_scale, angle_scale);
        let (mut q, pos, vel) = birth(layer, secondaries, p, pos, vel, local, rng);
        // Age the particle by its share of the step (a local layer moves with its emitter).
        let back = (1.0 - sub) * dt;
        let carried = if local { Vec3::ZERO } else { p.velocity };
        q.pos = pos - carried * back + vel * back;
        q.vel = vel;
        q.age = back * q.recip_life;
        q.serial = sim.spawned;
        sim.spawned = sim.spawned.wrapping_add(1);
        sim.particles.push(q);
    }
}

/// A new particle from its emitter-space position and velocity: the spawner's secondaries (lifetime, radius,
/// rotations, the move into the emitter's frame, inherited velocity, ...). Returns the particle and its world (or,
/// for a local layer, emitter-space) position and velocity.
fn birth(
    layer: &Layer,
    secondaries: &[u8],
    p: &Params,
    mut pos: Vec3,
    mut vel: Vec3,
    local: bool,
    rng: &mut Rng,
) -> (Particle, Vec3, Vec3) {
    let mut q = Particle { alpha: 1.0, recip_life: 1.0, radius: 0.1, plane: p.ground, ..default() };
    let mut placed = false;
    for &s in secondaries {
        let Some(a) = layer.actions.get(s as usize) else { continue };
        match &a.op {
            Op::Lifetime { min, max, .. } => q.recip_life = 1.0 / rng.range([*min, *max]).max(0.001),
            Op::Radius { min, max, .. } => q.radius = rng.range([*min, *max]),
            Op::Rotation2dInit { min, max, spin, .. } => {
                q.rot = rng.range([*min, *max]) * TAU;
                q.spin = *spin;
            }
            Op::AlphaInit { min, max, .. } => q.alpha = rng.range([*min, *max]),
            Op::PlaceInFrame { rotation, offset, transform, .. } => {
                let r = Quat::from_array(*rotation).normalize();
                pos = r * pos + Vec3::from(*offset);
                vel = r * vel;
                // 0x14 (`InitialisePosOffset`, no transform) leaves a local layer's particle in emitter space.
                if !(local && *transform == mw_formats::bear::Operand::None) {
                    pos = p.transform.transform_point3(pos);
                    vel = p.transform.transform_vector3(vel);
                }
                placed = true;
            }
            Op::InheritVelocity { min, max, .. } => {
                if !placed {
                    pos = p.transform.transform_point3(pos);
                    vel = p.transform.transform_vector3(vel);
                    placed = true;
                }
                vel += p.velocity * rng.range([*min, *max]);
            }
            Op::PlaneInit { .. } => q.plane = p.ground,
            Op::SeedInit { .. } => q.seed = (rng.next() * u32::MAX as f32) as u32,
            op @ Op::Rotation3dInit { .. } => mesh::init_rotation(op, &mut q, p, rng),
            // `FUN_0044a200`: indexed by the particle's serial in the game (a random entry here).
            Op::CollisionRadiusInit { table, .. } => {
                q.cr = 0.01 + table[(rng.next() * 16.0) as usize & 15] as f32 * (0.99 / 255.0);
            }
            _ => {}
        }
    }
    if !placed && !local {
        pos = p.transform.transform_point3(pos);
        vel = p.transform.transform_vector3(vel);
    }
    (q, pos, vel)
}

/// `SuperEmitter` (0x2B, run `FUN_0045d020`; `mw_formats::bear::Op::SuperEmitter`): children along each live
/// parent particle's path this update. The count's fraction is carried over in one accumulator for the whole
/// action and rounded to nearest, as the game does (its carry can go negative); the secondaries initialise
/// lifetime, rotation and the like, the super emitter sets position, velocity and radius itself.
#[allow(clippy::too_many_arguments)]
fn super_emit(
    layer: &Layer,
    sim: &mut LayerSim,
    op: &Op,
    secondaries: &[u8],
    parent: &[Particle],
    p: &Params,
    dt: f32,
    rng: &mut Rng,
) {
    let &Op::SuperEmitter { cone, speed, inherit, radius, rate, per_metre, radius_end, rate_end, .. } = op else {
        return;
    };
    let overwrite = layer.flags & 2 != 0;
    for parent in parent.iter().filter(|q| q.age < 1.0) {
        let t = parent.age;
        let back = parent.vel * -dt;
        let distance = back.length();
        let count = (1.0 + (rate_end - 1.0) * t) * (per_metre * distance + rate * dt) + sim.carry;
        let n = count.round();
        sim.carry = count - n;
        if n < 1.0 {
            continue;
        }
        // The frame about the backward direction (fallback +Y when the parent stands still).
        let z = if distance > 0.001 { back / distance } else { Vec3::Y };
        let x = z.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
        let y = x.cross(z);
        for _ in 0..n as usize {
            if sim.particles.len() >= layer.max_particles as usize {
                if !overwrite {
                    return;
                }
                sim.particles.remove(0);
            }
            let u = rng.next();
            let azimuth = rng.next() * TAU;
            let polar = rng.range(cone).to_radians();
            let dir = (azimuth.sin() * y + azimuth.cos() * x) * polar.sin() + polar.cos() * z;
            let mut q = Particle { alpha: 1.0, recip_life: 1.0, plane: p.ground, ..default() };
            init_secondaries(layer, secondaries, &mut q, rng);
            q.pos = parent.pos + back * u;
            q.vel = dir * rng.range(speed) + parent.vel * rng.range(inherit);
            // The parent's age at the birth time (`u` of the update back).
            let born = (t - u * dt * parent.recip_life).max(0.0);
            q.radius = rng.range(radius) * (1.0 + (radius_end - 1.0) * born);
            sim.particles.push(q);
        }
    }
}

/// The initialisers of a spawned particle that do not depend on the emitter (lifetime, rotation, alpha, seed).
fn init_secondaries(layer: &Layer, secondaries: &[u8], q: &mut Particle, rng: &mut Rng) {
    for &s in secondaries {
        match layer.actions.get(s as usize).map(|a| &a.op) {
            Some(Op::Lifetime { min, max, .. }) => q.recip_life = 1.0 / rng.range([*min, *max]).max(0.001),
            Some(Op::Rotation2dInit { min, max, spin, .. }) => {
                q.rot = rng.range([*min, *max]) * TAU;
                q.spin = *spin;
            }
            Some(Op::AlphaInit { min, max, .. }) => q.alpha = rng.range([*min, *max]),
            Some(Op::SeedInit { .. }) => q.seed = (rng.next() * u32::MAX as f32) as u32,
            Some(op @ Op::Rotation3dInit { .. }) => mesh::init_rotation(op, q, &Params::default(), rng),
            _ => {}
        }
    }
}

/// A new particle's position and velocity in emitter space (Y up / along the contact normal, Z forward).
///
/// Direction: the spawner's first angle range read as an azimuth in the emitter's XZ plane (from X towards
/// Z), the second as an elevation towards Y, in degrees. This fits the data's extremes (impact sparks
/// `0, 0` / `90, 90`: straight off the surface; `0..180` / `0`: a fan along the surface) but is a guess.
///
/// `EmitterConePlus` (0x00) emits along the emitter's +Z instead: the car exhaust locators are turned
/// half a turn about X, so +Z leaves the tail pipe backwards while +X would point every pipe at the
/// car's left side and +Y at the road. Its two ranges are read as a yaw (about Y) and a pitch (towards
/// Y) away from +Z with a random sign each (the game draws them from symmetric random numbers in
/// `FUN_0044cf30`; the exact mapping is not decoded), scaled by the binding slot `angle_scale`.
fn local_birth(sp: &Spawner, rng: &mut Rng, speed_scale: f32, angle_scale: f32) -> (Vec3, Vec3) {
    let speed = rng.range(sp.speed) * speed_scale;
    let dir = if sp.kind == 0x00 {
        let mut signed = |r: [f32; 2]| {
            let a = rng.range(r).to_radians() * angle_scale;
            if rng.next() < 0.5 { -a } else { a }
        };
        let (yaw, pitch) = (signed(sp.angles[0]), signed(sp.angles[1]));
        Vec3::new(pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos())
    } else {
        let azimuth = rng.range(sp.angles[0]).to_radians();
        let elevation = rng.range(sp.angles[1]).to_radians();
        Vec3::new(elevation.cos() * azimuth.cos(), elevation.sin(), elevation.cos() * azimuth.sin())
    };
    let pos = match sp.kind {
        // Wheel: across the tyre's width, a little above the ground.
        0x05 => Vec3::new(rng.range(sp.lateral) * 0.12, rng.range(sp.height), 0.0),
        // Collision: the game spawns over the contact area of the car's collision mesh
        // (`CollisionMeshParams`); a patch around the contact point stands in for it.
        0x01 => Vec3::new(rng.range([-0.3, 0.3]), 0.0, rng.range([-0.3, 0.3])),
        _ => Vec3::ZERO,
    };
    (pos, dir * speed)
}

// ---------------------------------------------------------------------------------------------------
// Emitters

/// Tyre smoke while a tyre slides, kick-up behind rolling wheels.
///
/// Per wheel, from the surface under it (`FUN_00589e20`: the collision tag's surface → its `SurfaceVfx`, dry
/// block as for the grip): the smoke is the `PlayerWheelEffects` (the player's car) or `AIWheelEffects` (any
/// other) `DefaultTyre` effect, on while the wheel touches the ground and any of its three smoke ramps
/// (`mw_handling::tyre_vfx`, the car's `TyreVfxBehaviour.Smoke`) is above 0.01. The game passes the three ramps to
/// the effect (`WheelParams`); here their maximum is the `Intensity` (the 0x05 spawner's binding slots are not
/// decoded). Not ported: the on-rims (`OnRims`) and the `61ac1863` variant (picked by a profile setting,
/// `FUN_006a15c0(…, 0xb) == 2`), and the surface's per-axle "always" flags (`SurfaceVfx` +0x156 / +0x157, false
/// on every surface).
fn wheel_emitters(
    time: Res<Time>,
    vfx: Res<Vfx>,
    mut instances: ResMut<Instances>,
    cars: Query<(Entity, &Car, &GlobalTransform, Has<crate::drive::Player>)>,
) {
    let dt = time.delta_secs();
    let Some(default_surface) = vfx.lib.surfaces.first() else { return };
    for (entity, car, transform, player) in &cars {
        let rot = transform.compute_transform().rotation;
        for (i, contact) in car.wheel_contacts().enumerate() {
            let Some(c) = contact else {
                // Off the ground: the wheel's emitters stop (their particles live on).
                for slot in [SLOT_TYRE, SLOT_KICKUP] {
                    instances.emit(&vfx, EmitterKey { owner: entity, slot: slot + i as u32 }, None, default());
                }
                continue;
            };
            let fx = &c.surface.and_then(|s| vfx.lib.surfaces.get(s)).unwrap_or(default_surface).dry;
            let wheels = if player { &fx.player_wheels } else { &fx.ai_wheels };
            let frame = Mat4::from_rotation_translation(rot, c.point);
            let ground = c.normal.extend(-c.normal.dot(c.point));
            let slip = c.slip.length();
            let smoke = c.smoke.max();
            instances.emit(
                &vfx,
                EmitterKey { owner: entity, slot: SLOT_TYRE + i as u32 },
                wheels.default_tyre,
                Params {
                    transform: frame,
                    velocity: c.velocity,
                    intensity: smoke.clamp(0.0, 1.0),
                    emitting: smoke > mw_handling::tyre_vfx::SMOKE_ON,
                    moved: slip * dt,
                    ground,
                    wheel: [c.smoke.spin, c.smoke.lat, c.smoke.lock],
                    ..default()
                },
            );
            let speed = c.velocity.length();
            instances.emit(
                &vfx,
                EmitterKey { owner: entity, slot: SLOT_KICKUP + i as u32 },
                fx.wheel,
                Params {
                    transform: frame,
                    velocity: c.velocity,
                    intensity: (speed / 60.0).clamp(0.0, 1.0),
                    // Model decision: on while the wheel touches the ground and rolls (the game's IsEmitting
                    // for the kick-up was not traced); the kick-up's `EmitterWheelContactPlus` scales its count
                    // by the contact speed and its `UpdateWheelBindings` slot, which starts near 1 m/s.
                    emitting: speed > 0.5,
                    moved: speed * dt,
                    ground,
                    wheel: [c.smoke.spin, c.smoke.lat, c.smoke.lock],
                    ..default()
                },
            );
        }
    }
}

/// Collect the car bodies' contacts after each physics step.
fn detect_impacts(collisions: Collisions, cars: Query<(), With<Car>>, mut out: MessageWriter<ImpactEvent>) {
    for pair in collisions.iter() {
        let (car, other, sign) = match (pair.body1, pair.body2) {
            (Some(b), o) if cars.contains(b) => (b, o, -1.0),
            (o, Some(b)) if cars.contains(b) => (b, o, 1.0),
            _ => continue,
        };
        for m in &pair.manifolds {
            let impulse = m.total_normal_impulse();
            // The fastest-approaching point (normal speeds are measured before the solver).
            let Some(point) = m.points.iter().min_by(|a, b| a.normal_speed.total_cmp(&b.normal_speed)) else {
                continue;
            };
            if impulse <= 0.0 {
                continue;
            }
            // The manifold normal points from collider 1 to collider 2; flip it to point at the car.
            let normal = (m.normal * sign).normalize_or_zero();
            out.write(ImpactEvent {
                car,
                other,
                position: point.point,
                normal,
                impulse,
                closing_speed: (-point.normal_speed).max(0.0),
                sliding_speed: m.tangent_velocity.length(),
            });
        }
    }
}

/// Game time of the first hard impact (closing at over 12 m/s), for `--screenshot-after-impact`.
#[derive(Resource)]
pub struct FirstImpact(pub f32);

/// The first hard impact's time, for `--screenshot-after-impact`. The cars' collision effects themselves are
/// `crash_vfx.rs` (the game's `FUN_00584260` / `FUN_00600220`).
fn impact_emitters(
    mut commands: Commands,
    first: Option<Res<FirstImpact>>,
    time: Res<Time>,
    mut events: MessageReader<ImpactEvent>,
) {
    for ev in events.read() {
        if first.is_none() && ev.closing_speed > 12.0 {
            commands.insert_resource(FirstImpact(time.elapsed_secs()));
            return;
        }
    }
}

/// A prop's impact effect (`PropObject` +0x14) when a car's contact passes its `ForceToDamage`
/// (`props::prop_contacts`, the game's `FUN_0054bf70`): half a metre above the contact point, Z along the hitter's
/// velocity, X = up × Z (`prop_vfx::impact_frame`). Effects load on first use; an impact waits up to half a second
/// for its effect.
fn prop_impact_effects(
    time: Res<Time>,
    vfx: Res<Vfx>,
    mut requests: ResMut<EffectRequests>,
    mut instances: ResMut<Instances>,
    mut impacts: MessageReader<crate::props::PropImpact>,
    mut waiting: Local<Vec<(f32, crate::props::PropImpact)>>,
) {
    let now = time.elapsed_secs();
    for hit in impacts.read() {
        if let Some(id) = hit.effect {
            requests.request(id);
            waiting.push((now, *hit));
        }
    }
    waiting.retain(|(t, hit)| {
        let Some(id) = hit.effect else { return false };
        if !vfx.has_effect(id) {
            return now - t < 0.5;
        }
        // `FUN_0054bf70` falls back to the identity rotation when the hitter's velocity has no direction.
        let frame = crate::prop_vfx::impact_frame(hit.position, hit.velocity, Vec3::Z);
        instances.one_shot(
            &vfx,
            Some(id),
            Params {
                transform: frame,
                velocity: hit.velocity,
                intensity: (hit.closing_speed / 15.0).clamp(0.0, 1.0),
                emitting: false,
                moved: 0.0,
                // Model decision: the ground is taken as level through the contact point.
                ground: Vec4::new(0.0, 1.0, 0.0, -hit.position.y),
                impact: hit.closing_speed,
                movement: 0.0,
                mesh: None,
                paint: Vec3::ZERO,
                wheel: [0.0; 3],
            },
        );
        false
    });
}

/// `MW_VFX_DEMO=<effect id>[,<seconds>[,<speed>]]`: play an effect every few seconds 8 m ahead of and 3 m
/// right of the camera, about on the ground, as if hit from the camera's right (emitter Y = camera left) by
/// something moving away from the camera at `speed` m/s (for looking at any effect; intensity 1,
/// emitting for `ONE_SHOT_EMIT`).
fn demo_emitter(
    time: Res<Time>,
    vfx: Res<Vfx>,
    debug: Res<VfxDebug>,
    mut instances: ResMut<Instances>,
    mut requests: ResMut<EffectRequests>,
    camera: Query<&GlobalTransform, crate::MainCamera>,
    mut next: Local<f32>,
) {
    let Some((id, every, speed)) = debug.demo else { return };
    // Effects outside the start-up set load on demand.
    requests.request(id);
    let now = time.elapsed_secs();
    if now < *next {
        return;
    }
    *next = now + every;
    let Some(cam) = camera.iter().next() else { return };
    let fwd = cam.forward().as_vec3();
    let flat = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::Z);
    let at = cam.translation() + flat * 8.0 + flat.cross(Vec3::Y) * 3.0 - Vec3::Y * 1.0;
    let (y, z) = if speed > 0.0 { (Vec3::Y.cross(flat), flat) } else { (Vec3::Y, flat) };
    let frame = Mat4::from_cols(y.cross(z).extend(0.0), y.extend(0.0), z.extend(0.0), at.extend(1.0));
    let params = Params {
        transform: frame,
        velocity: flat * speed,
        intensity: 1.0,
        emitting: true,
        moved: 0.0,
        ground: Vec4::new(0.0, 1.0, 0.0, -at.y),
        ..default()
    };
    instances.one_shot(&vfx, Some(id), params);
}

// ---------------------------------------------------------------------------------------------------
// Rendering

#[derive(Clone, Copy)]
struct QuadVertex {
    pos: Vec3,
    normal: Vec3,
    uv: Vec2,
    frame: Vec2,
    light: Vec4,
    colour: Vec4,
}

#[derive(Clone, Copy)]
struct SparkVertex {
    start: Vec3,
    end: Vec3,
    corner: Vec2,
    radius: f32,
    colour: Vec4,
}

/// A particle after the layer's render group.
struct Drawn {
    pos: Vec3,
    vel: Vec3,
    radius: f32,
    rot: f32,
    colour: Vec4,
    subuv: f32,
    quat: Quat,
    /// `RenderSpriteVelocity` (0x29): the streak's tail point and its alpha factor; otherwise unused.
    tail: Vec3,
    tail_alpha: f32,
}

/// Stand-in for `gVfxSparkBrightness`, a global the game's code sets (not in the data found so far): the
/// sparks' colours reach 6 × 12 × orange (Overbright × the colour curve), which tonemaps to flat white.
const SPARK_BRIGHTNESS: f32 = 0.15;

/// Exposure time of the motion-blurred sparks' streaks (the game blurs over the frame; a fixed 1/30 s
/// keeps the streak length independent of the frame rate).
const SPARK_SHUTTER: f32 = 1.0 / 30.0;

fn build_meshes(
    mut vfx: ResMut<Vfx>,
    instances: Res<Instances>,
    view: Res<BearView>,
    camera: Query<&GlobalTransform, crate::MainCamera>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(cam) = camera.iter().next() else { return };
    let eye = cam.translation();
    let (right, up) = (cam.right().as_vec3(), cam.up().as_vec3());
    let view = BearView { eye, forward: cam.forward().as_vec3(), ..*view };
    let vfx = &mut *vfx;
    for b in vfx.batches.values_mut() {
        b.quads.clear();
        b.sparks.clear();
    }
    let mut drawn = Vec::new();
    for inst in &instances.list {
        for (layer, sim) in inst.def.layers.iter().zip(&inst.layers) {
            let Some(r) = layer.renderer() else { continue };
            let Some(batch) = r.material.and_then(|m| vfx.batches.get_mut(&m)) else { continue };
            drawn.clear();
            // `ParametricPosVelToWorldSpace`: emitter-space particles placed with the current transform.
            let to_world = layer.to_world().map(|(r, o)| {
                inst.params.transform * Mat4::from_rotation_translation(Quat::from_array(r).normalize(), Vec3::from(o))
            });
            for q in &sim.particles {
                let placed;
                let q = match to_world {
                    Some(m) => {
                        placed = Particle { pos: m.transform_point3(q.pos), vel: m.transform_vector3(q.vel), ..*q };
                        &placed
                    }
                    None => q,
                };
                if let Some(d) = render_particle(layer, q, &view, &sim.slots) {
                    drawn.push(d);
                }
            }
            if batch.spark {
                for d in &drawn {
                    push_spark(&mut batch.sparks, d);
                }
            } else {
                drawn.sort_by(|a, b| b.pos.distance_squared(eye).total_cmp(&a.pos.distance_squared(eye)));
                // The renderers' size cap (`FUN_00458e90` / `FUN_0045ab10` / `FUN_0045b620`): the radius is at
                // most the cap × the view depth (from the view's depth origin, +0x218 = 0; its scale +0x220 = 1).
                let cap = r.size_cap * sim.slot(r.size_cap_slot);
                for d in &drawn {
                    let depth = (d.pos - eye).dot(view.forward);
                    let radius = if cap > 0.0 { d.radius.min(cap * depth.max(0.0)) } else { d.radius };
                    if matches!(r.kind, 0x1E | 0x1F) {
                        mesh::push_card(&mut batch.quads, d, r, eye);
                    } else {
                        push_quad(&mut batch.quads, d, radius, r, eye, right, up);
                    }
                }
            }
        }
    }
    for b in vfx.batches.values_mut() {
        let empty = if b.spark { b.sparks.is_empty() } else { b.quads.is_empty() };
        if empty && b.placeholder {
            continue;
        }
        b.placeholder = empty;
        // Empty vertex buffers upset Bevy's mesh allocator: keep one invisible quad.
        if b.spark && b.sparks.is_empty() {
            let corner = Vec2::ZERO;
            b.sparks.extend(
                [SparkVertex { start: Vec3::ZERO, end: Vec3::ZERO, corner, radius: 0.0, colour: Vec4::ZERO }; 4],
            );
        }
        if !b.spark && b.quads.is_empty() {
            let v = QuadVertex {
                pos: Vec3::ZERO,
                normal: Vec3::Y,
                uv: Vec2::ZERO,
                frame: Vec2::ZERO,
                light: Vec4::ZERO,
                colour: Vec4::ZERO,
            };
            b.quads.extend([v; 4]);
        }
        let mesh = if b.spark { spark_mesh(&b.sparks) } else { quad_mesh(&b.quads) };
        let _ = meshes.insert(&b.mesh, mesh);
    }
}

/// The layer's render group for one particle: size and colour curves, alpha, fades, atlas frame.
/// `slots` are the layer's binding slots; the colour override parameter (`use_tint`) is not set by any
/// emitter here and reads white.
fn render_particle(layer: &Layer, q: &Particle, view: &BearView, slots: &[f32]) -> Option<Drawn> {
    let tint = Vec4::ONE;
    let slot = |k: u8| slots.get(k as usize).copied().unwrap_or(0.0);
    let mut d = Drawn {
        pos: q.pos,
        vel: q.vel,
        radius: q.radius,
        rot: q.rot,
        colour: Vec4::ONE,
        subuv: q.age,
        tail: q.pos,
        tail_alpha: 1.0,
        quat: q.quat,
    };
    let t = q.age.clamp(0.0, 1.0);
    let group = layer.groups[1].as_ref()?;
    for seq in &group.sequences {
        for step in &seq.steps {
            let Some(a) = layer.actions.get(step.action as usize) else { continue };
            match &a.op {
                &Op::RadiusCurve { knee, values: v, floor, slot: k, .. } => {
                    // `FUN_004612d0` with the slopes `0x0044bbb0` compiles from the three values.
                    let r = if t < knee || knee >= 1.0 {
                        v[0] + (v[1] - v[0]) * t / knee.max(1e-6)
                    } else {
                        v[1] + (v[2] - v[1]) * (t - knee) / (1.0 - knee)
                    };
                    d.radius = q.radius * r * (floor + (1.0 - floor) * slot(k));
                }
                Op::ColourCurve { rows, ramp, alpha, rates, alpha_slot, use_tint, .. } => {
                    // `FUN_004553b0`: ramp weights over the (wrapped) age, rgb times the brightness column.
                    let frac = |x: f32| x - x.floor();
                    let u = frac(t * rates[0]);
                    let mut c = Vec4::from(rows[0]);
                    for k in 0..4 {
                        let w = (u * ramp[0][k] + ramp[1][k]).clamp(0.0, 1.0);
                        c += Vec4::from(rows[k + 1]) * w;
                    }
                    let mut rgb = (c.truncate() * c.w).max(Vec3::ZERO);
                    let mut a =
                        alpha.eval(frac(t * rates[1])) * slots.get(*alpha_slot as usize).copied().unwrap_or(0.0);
                    if *use_tint {
                        rgb = tint.truncate();
                        a *= tint.w;
                    }
                    d.colour = rgb.extend(a);
                }
                Op::AlphaApply { .. } => d.colour.w *= q.alpha,
                Op::SeedTint { min, max, .. } => {
                    let k = min + (max - min) * hash01(q.seed);
                    d.colour = (d.colour.truncate() * k).extend(d.colour.w);
                }
                Op::ViewFade { .. } => view_fade(&a.op, &mut d, view),
                Op::SubUv { .. } => d.subuv = t,
                _ => {}
            }
        }
    }
    match layer.renderer() {
        // `RenderSpriteMotionBlur` (0x25, `FUN_004598e0`) writes the colour's rgb with alpha 1.0 (`.rdata`
        // 0x00f9d13c) into the streak's vertices: the colour curve's alpha (all zero on the impact sparks) is
        // not used. (The streak's other end takes the renderer's word 11 in the game; not ported, both ends 1.)
        Some(r) if r.kind == 0x25 => {
            d.colour.w = 1.0;
            d.tail = d.pos - d.vel * SPARK_SHUTTER;
        }
        // `RenderSpriteVelocity` (0x29, `FUN_0045c2f0`): the colour keeps its alpha; the streak is the velocity
        // × 1/30 s × the renderer's scale × a binding slot, its length clamped, and its tail's alpha × a word.
        Some(r) if r.kind == 0x29 => {
            let v = d.vel * SPARK_SHUTTER * r.streak_scale * slot(r.streak_slot);
            let len = v.length();
            let clamped = len.clamp(r.streak[0], r.streak[1]);
            d.tail = d.pos - if len > 1e-6 { v * (clamped / len) } else { Vec3::ZERO };
            d.tail_alpha = r.tail_alpha;
        }
        _ => {}
    }
    (d.colour.w > 1e-3 && d.radius > 1e-4).then_some(d)
}

/// `UpdateParametricFadeOverDistance` (0x38, `FUN_00460970`; `mw_formats::bear::Op::ViewFade`): the colour
/// over the depth along the camera's forward axis: `colours[0]` at the camera, `colours[1]` at the near
/// distance (which grows with the camera's speed on layers that ask for it), towards `colours[2]` at `far`;
/// then optional speed fades of the alpha and the radius.
fn view_fade(op: &Op, d: &mut Drawn, view: &BearView) {
    let &Op::ViewFade {
        colours,
        offset,
        near,
        far,
        near_fast,
        fast_speed,
        speed_full,
        speed_zero,
        speed_near,
        subtract_radius,
        speed_radius,
        speed_alpha,
    } = op
    else {
        return;
    };
    let n = if speed_near && fast_speed > 0.0 {
        near + (near_fast - near) * (view.speed / fast_speed).min(1.0)
    } else {
        near
    };
    // The depth origin: the camera + (max(offset, view +0x218 = 0) + n) along its forward axis.
    let origin = view.eye + view.forward * (offset.max(0.0) + n);
    let mut depth = (d.pos - origin).dot(view.forward);
    if subtract_radius {
        depth -= d.radius;
    }
    let near_w = if n != 0.0 { (-depth / n).clamp(0.0, 1.0) } else { 0.0 };
    let far_w = if far != near { (depth / (far - near)).clamp(0.0, 1.0) } else { 0.0 };
    let (a, b, c) = (Vec4::from(colours[0]), Vec4::from(colours[1]), Vec4::from(colours[2]));
    d.colour *= b + (a - b) * near_w + (c - b) * far_w;
    if speed_alpha || speed_radius {
        let range = speed_full * speed_full - speed_zero * speed_zero;
        let k = if range != 0.0 {
            ((d.vel.length_squared() - speed_zero * speed_zero) / range).clamp(0.0, 1.0)
        } else {
            1.0
        };
        if speed_alpha {
            d.colour.w *= k;
        }
        if speed_radius {
            d.radius *= k;
        }
    }
}

fn push_quad(out: &mut Vec<QuadVertex>, d: &Drawn, radius: f32, r: &Renderer, eye: Vec3, right: Vec3, up: Vec3) {
    let (s, c) = d.rot.sin_cos();
    let aspect = if r.aspect > 0.0 { r.aspect } else { 1.0 };
    let rx = (right * c + up * s) * radius * aspect;
    let ry = (up * c - right * s) * radius / aspect;
    let centre = d.pos + rx * (1.0 - 2.0 * r.pivot[0]) + ry * (2.0 * r.pivot[1] - 1.0);
    let toward = (eye - d.pos).normalize_or_zero();
    // w: the draw's opacity (`gVfxConstants.z`, the renderer's word after the depth fade).
    let light = Vec4::new(r.lighting[0], r.lighting[1], r.lighting[2], r.opacity);
    // The depth fade only on the soft passes (0x23 / 0x2a: `FUN_004a40a0` → `FUN_004976d0`).
    let frame = Vec2::new(d.subuv, if r.soft { r.depth_fade.max(0.0) } else { 0.0 });
    for (cx, cy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let offset = rx * cx + ry * cy;
        out.push(QuadVertex {
            pos: centre + offset,
            // A rounded normal: towards the camera, bent out to the corners.
            normal: (toward + offset.normalize_or_zero() * 0.7).normalize_or_zero(),
            uv: Vec2::new((cx + 1.0) * 0.5, (1.0 - cy) * 0.5),
            frame,
            light,
            colour: d.colour,
        });
    }
}

fn push_spark(out: &mut Vec<SparkVertex>, d: &Drawn) {
    let tail = d.colour * Vec4::new(1.0, 1.0, 1.0, d.tail_alpha);
    for corner in [Vec2::new(-1.0, -1.0), Vec2::new(1.0, -1.0), Vec2::new(1.0, 1.0), Vec2::new(-1.0, 1.0)] {
        // corner.x < 0: the end (tail) disc.
        let colour = if corner.x < 0.0 { tail } else { d.colour };
        out.push(SparkVertex { start: d.pos, end: d.tail, corner, radius: d.radius, colour });
    }
}

fn quad_indices(quads: usize) -> Indices {
    Indices::U32((0..quads as u32).flat_map(|q| [0, 1, 2, 0, 2, 3].map(|i| q * 4 + i)).collect())
}

fn empty_mesh() -> Mesh {
    let v = QuadVertex {
        pos: Vec3::ZERO,
        normal: Vec3::Y,
        uv: Vec2::ZERO,
        frame: Vec2::ZERO,
        light: Vec4::ZERO,
        colour: Vec4::ZERO,
    };
    quad_mesh(&[v; 4])
}

fn quad_mesh(v: &[QuadVertex]) -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, v.iter().map(|v| v.pos.to_array()).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, v.iter().map(|v| v.normal.to_array()).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, v.iter().map(|v| v.uv.to_array()).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, v.iter().map(|v| v.frame.to_array()).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, v.iter().map(|v| v.light.to_array()).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, v.iter().map(|v| v.colour.to_array()).collect::<Vec<_>>())
        .with_inserted_indices(quad_indices(v.len() / 4))
}

fn spark_mesh(v: &[SparkVertex]) -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, v.iter().map(|v| v.start.to_array()).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, v.iter().map(|v| v.end.to_array()).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, v.iter().map(|v| v.corner.to_array()).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, v.iter().map(|v| [v.radius, 0.0]).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, v.iter().map(|v| v.colour.to_array()).collect::<Vec<_>>())
        .with_inserted_indices(quad_indices(v.len() / 4))
}

// ---------------------------------------------------------------------------------------------------
// Materials

#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct QuadParams {
    atlas: Vec4,
    key_colour: Vec4,
    key_direction: Vec4,
}

/// `VfxParticles_Diffuse*`: lit, premultiplied quads (`particles.wgsl`).
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct QuadMaterial {
    #[uniform(0)]
    params: QuadParams,
    #[texture(1)]
    #[sampler(2)]
    texture: Option<Handle<Image>>,
    #[texture(3, dimension = "cube")]
    #[sampler(4)]
    irradiance: Option<Handle<Image>>,
}

fn blend(src: BlendFactor, dst: BlendFactor) -> BlendState {
    let c = BlendComponent { src_factor: src, dst_factor: dst, operation: BlendOperation::Add };
    BlendState { color: c, alpha: BlendComponent { src_factor: BlendFactor::One, ..c } }
}

fn set_blend(descriptor: &mut RenderPipelineDescriptor, state: BlendState) {
    // Sprites face the camera either way round (the spark quads are built in y-down pixel space).
    descriptor.primitive.cull_mode = None;
    if let Some(target) = descriptor.fragment.as_mut().and_then(|f| f.targets.get_mut(0)).and_then(|t| t.as_mut()) {
        target.blend = Some(state);
    }
}

impl Material for QuadMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://mostrusted/particles.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://mostrusted/particles.wgsl".into()
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
            Mesh::ATTRIBUTE_UV_1.at_shader_location(3),
            Mesh::ATTRIBUTE_TANGENT.at_shader_location(4),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(5),
        ])?];
        set_blend(descriptor, blend(BlendFactor::One, BlendFactor::OneMinusSrcAlpha));
        Ok(())
    }
}

#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct SparkParams {
    scale: Vec4,
}

/// `VfxParticles_MotionBlurSpriteUntex`: additive motion-blurred discs (`sparks.wgsl`).
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct SparkMaterial {
    #[uniform(0)]
    params: SparkParams,
}

impl Material for SparkMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://mostrusted/sparks.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://mostrusted/sparks.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
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
            Mesh::ATTRIBUTE_UV_1.at_shader_location(3),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(5),
        ])?];
        set_blend(descriptor, blend(BlendFactor::SrcAlpha, BlendFactor::One));
        Ok(())
    }
}
