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
//! surface's `PlayerWheelEffects.DefaultTyre` while a tyre slides, the surface's `WheelVfx` kick-up
//! behind rolling wheels, and the surface's directional impact effects (`FrontImpactEffect`, ...) for
//! collisions of the car body (sparks; side contacts scrape continuously). Surface types are not decoded
//! from the collision mesh yet, so every contact uses the default surface (tarmac).
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
use mw_assets::vfx::{self, ParticleMaterial as GameMaterial, VfxLibrary};
use mw_bundle::ResourceId;
use mw_formats::bear::{BearEffect, Layer, Op, Renderer, Spawner};

use crate::drive::Car;

pub struct ParticlesPlugin;

impl Plugin for ParticlesPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "particles.wgsl");
        bevy::asset::embedded_asset!(app, "sparks.wgsl");
        app.add_plugins((MaterialPlugin::<QuadMaterial>::default(), MaterialPlugin::<SparkMaterial>::default()))
            .add_message::<ImpactEvent>()
            .init_resource::<Instances>()
            .insert_resource(VfxDebug::from_env())
            .add_systems(Update, (start_loading, receive_vfx))
            .add_systems(FixedPostUpdate, detect_impacts.after(PhysicsSystems::Writeback))
            .add_systems(
                Update,
                (wheel_emitters, impact_emitters, demo_emitter, simulate, build_meshes)
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
pub struct ImpactEvent {
    pub car: Entity,
    pub position: Vec3,
    pub normal: Vec3,
    pub impulse: f32,
    pub closing_speed: f32,
    pub sliding_speed: f32,
}

/// Debug switches from the environment: `MW_VFX_DEMO=<effect id>[,<seconds>[,<speed>]]` (see
/// [`demo_emitter`]) and `MW_VFX_DEBUG=1` (log live particle counts four times a second).
#[derive(Resource, Default)]
struct VfxDebug {
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
pub struct Vfx {
    lib: VfxLibrary,
    effects: HashMap<ResourceId, Arc<BearEffect>>,
    batches: HashMap<ResourceId, Batch>,
}

struct Batch {
    mesh: Handle<Mesh>,
    spark: bool,
    quads: Vec<QuadVertex>,
    sparks: Vec<SparkVertex>,
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
    let mut ids: Vec<ResourceId> = lib
        .surfaces
        .iter()
        .flat_map(|s| [&s.dry, &s.wet])
        .flat_map(|e| [e.wheel, e.player_wheels.default_tyre, e.player_wheels.on_rims].into_iter().chain(e.impacts))
        .chain(lib.vehicle_impacts.iter().copied())
        .chain(lib.vehicle_scrapes.iter().copied())
        .flatten()
        .collect();
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
    info!("vfx: {} effects, {} materials", effects.len(), materials.len());
    Ok(VfxLoaded { lib, effects, materials })
}

#[allow(clippy::too_many_arguments)]
fn receive_vfx(
    mut commands: Commands,
    loading: Option<Res<VfxLoading>>,
    lighting: Option<Res<crate::deferred::GameLighting>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut quad_materials: ResMut<Assets<QuadMaterial>>,
    mut spark_materials: ResMut<Assets<SparkMaterial>>,
) {
    let Some(loading) = loading else { return };
    let msg = match loading.0.lock().unwrap().try_recv() {
        Ok(m) => m,
        Err(mpsc::TryRecvError::Empty) => return,
        Err(mpsc::TryRecvError::Disconnected) => {
            commands.remove_resource::<VfxLoading>();
            return;
        }
    };
    commands.remove_resource::<VfxLoading>();
    let loaded = match msg {
        Ok(l) => l,
        Err(e) => {
            warn!("no particle effects: {e}");
            return;
        }
    };
    let (key_colour, key_direction, sky, irradiance) = match &lighting {
        Some(l) => (l.key_colour, l.key_direction, l.sky_brightness, Some(l.irradiance.clone())),
        None => (Vec3::splat(9000.0), Vec3::new(-0.4, -0.8, -0.4).normalize(), 1200.0, None),
    };
    let mut batches = HashMap::new();
    for (m, image) in loaded.materials {
        debug!(
            "particle material {}: {} tiles {:?} looping {} overbright {}",
            m.id, m.shader, m.tiles, m.looping, m.overbright
        );
        let mesh = meshes.add(empty_mesh());
        let spark = m.shader.contains("MotionBlurSprite");
        let mut e =
            commands.spawn((Mesh3d(mesh.clone()), Transform::IDENTITY, Visibility::default(), NoFrustumCulling));
        if spark {
            let scale = m.overbright * SPARK_BRIGHTNESS;
            e.insert(MeshMaterial3d(
                spark_materials.add(SparkMaterial { params: SparkParams { scale: Vec4::new(scale, 0.0, 0.0, 0.0) } }),
            ));
        } else {
            let blend = if m.shader.contains("NoBlend") { 0.0 } else { 1.0 };
            e.insert(MeshMaterial3d(quad_materials.add(QuadMaterial {
                params: QuadParams {
                    atlas: Vec4::new(m.tiles[0] as f32, m.tiles[1] as f32, if m.looping { 1.0 } else { 0.0 }, blend),
                    key_colour: key_colour.extend(sky),
                    key_direction: key_direction.extend(0.0),
                },
                texture: image.map(|i| images.add(i)),
                irradiance: irradiance.clone(),
            })));
        }
        batches.insert(m.id, Batch { mesh, spark, quads: Vec::new(), sparks: Vec::new() });
    }
    let effects = loaded.effects.into_iter().map(|fx| (fx.id, Arc::new(fx))).collect();
    commands.insert_resource(Vfx { lib: loaded.lib, effects, batches });
}

// ---------------------------------------------------------------------------------------------------
// Simulation

/// How long a one-shot effect's continuous spawners run (seconds); the game keeps an impact effect
/// emitting while the contact lasts.
const ONE_SHOT_EMIT: f32 = 0.15;

/// Persistent emitters are keyed by owner and slot (wheel smoke, wheel kick-up, scrape).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct EmitterKey {
    owner: Entity,
    slot: u32,
}

const SLOT_TYRE: u32 = 0;
const SLOT_KICKUP: u32 = 0x100;
const SLOT_SCRAPE: u32 = 0x200;

/// The parameters the game sets on an effect instance (stand-ins, see the module docs).
#[derive(Clone, Copy, Debug)]
struct Params {
    /// World from emitter space (origin at the emitter, the car's axes; impacts: Y = contact normal).
    transform: Mat4,
    velocity: Vec3,
    intensity: f32,
    emitting: bool,
    /// Metres the emitter moved (or slid) this step, for per-metre emission.
    moved: f32,
    /// Ground plane under the emitter (normal, -distance), for bouncing particles.
    ground: Vec4,
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
        }
    }
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
}

struct LayerSim {
    particles: Vec<Particle>,
    /// Fractional particles carried to the next step.
    carry: f32,
    /// Emission scale from the layer's controller.
    rate_scale: f32,
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
        let layers =
            def.layers.iter().map(|_| LayerSim { particles: Vec::new(), carry: 0.0, rate_scale: 1.0 }).collect();
        Self { def, key, params, layers, burst_done: false, emit_for: 0.0, rng: Rng(seed | 1) }
    }

    fn alive(&self) -> bool {
        self.layers.iter().any(|l| !l.particles.is_empty()) || !self.burst_done || self.emit_for > 0.0
    }
}

#[derive(Resource, Default)]
struct Instances {
    list: Vec<EffectInstance>,
    seed: u32,
}

impl Instances {
    /// Refresh (or start) a persistent emitter.
    fn emit(&mut self, vfx: &Vfx, key: EmitterKey, effect: Option<ResourceId>, params: Params) {
        let Some(def) = effect.and_then(|id| vfx.effects.get(&id)) else { return };
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

    fn one_shot(&mut self, vfx: &Vfx, effect: Option<ResourceId>, params: Params) {
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

fn simulate(
    time: Res<Time>,
    debug: Res<VfxDebug>,
    mut instances: ResMut<Instances>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
) {
    let dt = time.delta_secs().min(0.1);
    if dt <= 0.0 {
        return;
    }
    let eye = camera.iter().next().map(|t| t.translation()).unwrap_or_default();
    for inst in &mut instances.list {
        let def = inst.def.clone();
        for (layer, sim) in def.layers.iter().zip(&mut inst.layers) {
            step_layer(layer, sim, &inst.params, inst.burst_done, eye, dt, &mut inst.rng);
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
fn step_layer(layer: &Layer, sim: &mut LayerSim, p: &Params, burst_done: bool, eye: Vec3, dt: f32, rng: &mut Rng) {
    let Some(group) = &layer.groups[0] else { return };
    for seq in &group.sequences {
        for step in &seq.steps {
            let Some(action) = layer.actions.get(step.action as usize) else { continue };
            match &action.op {
                Op::Controller { curves, .. } => {
                    // The first curve over the effect's intensity (which input drives which curve is in
                    // bytes not decoded yet).
                    sim.rate_scale = curves[0].eval(p.intensity.clamp(0.0, 1.0));
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
                Op::Spawn(sp) => {
                    spawn(layer, sim, sp, &step.secondaries, p, burst_done, eye, dt, rng);
                }
                _ => {}
            }
        }
    }
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
        count += (sp.rate * dt + sp.per_metre * p.moved) * sim.rate_scale * lod;
    }
    if !burst_done && sp.burst > 0 {
        count += sp.burst as f32 * p.intensity.clamp(0.1, 1.0) * lod;
    }
    let n = count.floor() as usize;
    sim.carry = if p.emitting { count - n as f32 } else { 0.0 };
    let overwrite = layer.flags & 2 != 0;
    for i in 0..n {
        if sim.particles.len() >= layer.max_particles as usize {
            if !overwrite {
                break;
            }
            sim.particles.remove(0);
        }
        // Spread births over the step so streams stay continuous.
        let sub = (i as f32 + rng.next()) / n.max(1) as f32;
        let mut q = Particle { alpha: 1.0, recip_life: 1.0, radius: 0.1, plane: p.ground, ..default() };
        let (mut pos, mut vel) = local_birth(sp, rng);
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
                Op::PlaceInFrame { rotation, offset, .. } => {
                    let r = Quat::from_array(*rotation).normalize();
                    pos = p.transform.transform_point3(r * pos + Vec3::from(*offset));
                    vel = p.transform.transform_vector3(r * vel);
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
                _ => {}
            }
        }
        if !placed {
            pos = p.transform.transform_point3(pos);
            vel = p.transform.transform_vector3(vel);
        }
        // Age the particle by its share of the step.
        let back = (1.0 - sub) * dt;
        q.pos = pos - p.velocity * back + vel * back;
        q.vel = vel;
        q.age = back * q.recip_life;
        sim.particles.push(q);
    }
}

/// A new particle's position and velocity in emitter space (Y up / along the contact normal, Z forward).
///
/// Direction: the spawner's first angle range read as an azimuth in the emitter's XZ plane (from X towards
/// Z), the second as an elevation towards Y, in degrees. This fits the data's extremes (impact sparks
/// `0, 0` / `90, 90`: straight off the surface; `0..180` / `0`: a fan along the surface) but is a guess.
fn local_birth(sp: &Spawner, rng: &mut Rng) -> (Vec3, Vec3) {
    let speed = rng.range(sp.speed);
    let azimuth = rng.range(sp.angles[0]).to_radians();
    let elevation = rng.range(sp.angles[1]).to_radians();
    let dir = Vec3::new(elevation.cos() * azimuth.cos(), elevation.sin(), elevation.cos() * azimuth.sin());
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
fn wheel_emitters(
    time: Res<Time>,
    vfx: Res<Vfx>,
    mut instances: ResMut<Instances>,
    cars: Query<(Entity, &Car, &GlobalTransform)>,
) {
    let dt = time.delta_secs();
    let Some(surface) = vfx.lib.surfaces.first() else { return };
    let fx = &surface.dry;
    for (entity, car, transform) in &cars {
        let rot = transform.compute_transform().rotation;
        for (i, contact) in car.wheel_contacts().enumerate() {
            let Some(c) = contact else { continue };
            let frame = Mat4::from_rotation_translation(rot, c.point);
            let ground = c.normal.extend(-c.normal.dot(c.point));
            let slip = c.slip.length();
            instances.emit(
                &vfx,
                EmitterKey { owner: entity, slot: SLOT_TYRE + i as u32 },
                fx.player_wheels.default_tyre,
                Params {
                    transform: frame,
                    velocity: c.velocity,
                    intensity: (slip / 15.0).clamp(0.0, 1.0),
                    emitting: slip > 2.5,
                    moved: slip * dt,
                    ground,
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
                    emitting: speed > 2.0,
                    moved: speed * dt,
                    ground,
                },
            );
        }
    }
}

/// Collect the car bodies' contacts after each physics step.
fn detect_impacts(collisions: Collisions, cars: Query<(), With<Car>>, mut out: MessageWriter<ImpactEvent>) {
    for pair in collisions.iter() {
        let (car, sign) = match (pair.body1, pair.body2) {
            (Some(b), _) if cars.contains(b) => (b, -1.0),
            (_, Some(b)) if cars.contains(b) => (b, 1.0),
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

/// Closing speed (m/s) above which a contact counts as an impact, and the minimum time between bursts.
const IMPACT_SPEED: f32 = 3.0;
const IMPACT_COOLDOWN: f32 = 0.25;

/// Sparks: a burst for each new impact, continuous sparks while a side scrapes.
#[allow(clippy::too_many_arguments)]
fn impact_emitters(
    mut commands: Commands,
    first: Option<Res<FirstImpact>>,
    time: Res<Time>,
    vfx: Res<Vfx>,
    mut instances: ResMut<Instances>,
    mut events: MessageReader<ImpactEvent>,
    cars: Query<(&GlobalTransform, &avian3d::prelude::LinearVelocity)>,
    mut last_burst: Local<HashMap<Entity, f32>>,
) {
    let now = time.elapsed_secs();
    let Some(surface) = vfx.lib.surfaces.first() else { return };
    let fx = &surface.dry;
    for ev in events.read() {
        let Ok((t, v)) = cars.get(ev.car) else { continue };
        let rot = t.compute_transform().rotation;
        // Which side of the car was hit: the contact normal points into the car, so the hit side faces -n.
        let local = rot.inverse() * -ev.normal;
        let side = if local.y.abs() > local.x.abs().max(local.z.abs()) {
            if local.y > 0.0 { vfx::IMPACT_TOP } else { vfx::IMPACT_BOTTOM }
        } else if local.z.abs() >= local.x.abs() {
            if local.z > 0.0 { vfx::IMPACT_FRONT } else { vfx::IMPACT_REAR }
        } else if local.x > 0.0 {
            vfx::IMPACT_LEFT
        } else {
            vfx::IMPACT_RIGHT
        };
        // Emitter frame: Y along the contact normal, Z along the car's travel.
        let y = ev.normal;
        let travel = v.0;
        let z = (travel - y * travel.dot(y)).try_normalize().unwrap_or_else(|| y.any_orthonormal_vector());
        let frame = Mat4::from_cols(y.cross(z).extend(0.0), y.extend(0.0), z.extend(0.0), ev.position.extend(1.0));
        let ground = y.extend(-y.dot(ev.position));
        let last = last_burst.entry(ev.car).or_insert(f32::MIN);
        if ev.closing_speed > IMPACT_SPEED && now - *last > IMPACT_COOLDOWN {
            *last = now;
            if first.is_none() && ev.closing_speed > 12.0 {
                commands.insert_resource(FirstImpact(now));
            }
            debug!(
                "impact at {:.1} (t {now:.2} s): {:.1} m/s closing, impulse {:.0} N·s, side {side}",
                ev.position, ev.closing_speed, ev.impulse
            );
            instances.one_shot(
                &vfx,
                fx.impacts[side],
                Params {
                    transform: frame,
                    velocity: travel,
                    intensity: (ev.closing_speed / 15.0).clamp(0.0, 1.0),
                    emitting: false,
                    moved: 0.0,
                    ground,
                },
            );
        }
        if ev.sliding_speed > 2.0 && ev.impulse > 20.0 {
            // Scraping: the side effect is the one with a continuous rate (the others are bursts).
            let effect = fx.impacts[vfx::IMPACT_LEFT].or(fx.impacts[side]);
            instances.emit(
                &vfx,
                EmitterKey { owner: ev.car, slot: SLOT_SCRAPE },
                effect,
                Params {
                    transform: frame,
                    velocity: travel,
                    intensity: (ev.sliding_speed / 20.0).clamp(0.0, 1.0),
                    emitting: true,
                    moved: 0.0,
                    ground,
                },
            );
        }
    }
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
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut next: Local<f32>,
) {
    let Some((id, every, speed)) = debug.demo else { return };
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
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(cam) = camera.iter().next() else { return };
    let eye = cam.translation();
    let (right, up) = (cam.right().as_vec3(), cam.up().as_vec3());
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
            for q in &sim.particles {
                if let Some(d) = render_particle(layer, q, eye) {
                    drawn.push(d);
                }
            }
            if batch.spark {
                for d in &drawn {
                    push_spark(&mut batch.sparks, d);
                }
            } else {
                drawn.sort_by(|a, b| b.pos.distance_squared(eye).total_cmp(&a.pos.distance_squared(eye)));
                for d in &drawn {
                    push_quad(&mut batch.quads, d, r, eye, right, up);
                }
            }
        }
    }
    for b in vfx.batches.values_mut() {
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
fn render_particle(layer: &Layer, q: &Particle, eye: Vec3) -> Option<Drawn> {
    let mut d = Drawn { pos: q.pos, vel: q.vel, radius: q.radius, rot: q.rot, colour: Vec4::ONE, subuv: q.age };
    let t = q.age.clamp(0.0, 1.0);
    let group = layer.groups[1].as_ref()?;
    for seq in &group.sequences {
        for step in &seq.steps {
            let Some(a) = layer.actions.get(step.action as usize) else { continue };
            match &a.op {
                Op::RadiusCurve { bezier: b, .. } => {
                    let u = 1.0 - t;
                    let s = u * u * u * b[0] + 3.0 * u * u * t * b[1] + 3.0 * u * t * t * b[2] + t * t * t * b[3];
                    d.radius = q.radius * s;
                }
                Op::ColourCurve { rows, alpha, alpha_scale, .. } => {
                    let mut c = Vec4::ZERO;
                    let mut tk = 1.0;
                    for row in rows {
                        c += Vec4::from(*row) * tk;
                        tk *= t;
                    }
                    let rgb = c.truncate().max(Vec3::ZERO);
                    d.colour = rgb.extend(c.w.max(0.0) * alpha.eval(t) * alpha_scale);
                }
                Op::AlphaApply { .. } => d.colour.w *= q.alpha,
                Op::SeedTint { min, max, .. } => {
                    let k = min + (max - min) * hash01(q.seed);
                    d.colour = (d.colour.truncate() * k).extend(d.colour.w);
                }
                Op::ViewFade { near_fade, .. } => {
                    if *near_fade > 0.0 {
                        d.colour.w *= (q.pos.distance(eye) / near_fade).clamp(0.0, 1.0);
                    }
                }
                Op::SubUv { .. } => d.subuv = t,
                _ => {}
            }
        }
    }
    (d.colour.w > 1e-3 && d.radius > 1e-4).then_some(d)
}

fn push_quad(out: &mut Vec<QuadVertex>, d: &Drawn, r: &Renderer, eye: Vec3, right: Vec3, up: Vec3) {
    let (s, c) = d.rot.sin_cos();
    let rx = (right * c + up * s) * d.radius;
    let ry = (up * c - right * s) * d.radius;
    let centre = d.pos + rx * (1.0 - 2.0 * r.pivot[0]) + ry * (2.0 * r.pivot[1] - 1.0);
    let toward = (eye - d.pos).normalize_or_zero();
    let light = Vec4::new(r.lighting[0], r.lighting[1], r.lighting[2], 1.0);
    let frame = Vec2::new(d.subuv, r.depth_fade_guess.max(0.0));
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
    let end = d.pos - d.vel * SPARK_SHUTTER;
    for corner in [Vec2::new(-1.0, -1.0), Vec2::new(1.0, -1.0), Vec2::new(1.0, 1.0), Vec2::new(-1.0, 1.0)] {
        out.push(SparkVertex { start: d.pos, end, corner, radius: d.radius, colour: d.colour });
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
