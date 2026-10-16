//! Coronas: the glow, beam and flare sprites at car lamps (docs/decomp.md "Coronas"; data
//! `mw_assets::coronas`). One system for every car: the player's and the AI racers' (the race-car path,
//! `FUN_00581640`), the cops' light bars (effect group 3) and traffic (`FUN_005fd190`).
//!
//! **Per entry** (`FUN_004a5f70`): an entry of `VehicleVfxBehaviour.Coronas` shows while its `Group` bit is in the
//! car's effect mask; intensity = gate(`da6ac384`: 0 always, 1 by day = 1 − night, 2 = night) × `Brightness` × L²
//! × the group's night scale (`VfxGlobals.VehicleCoronaScalesForNight`, lerp(1, field, night): group 7 × 1.3);
//! size = (`f65c1f07`, `f65c1f06`); per locator (and mirrored through x = −x when `Mirror`), the `Animation`'s
//! linear value at the car's flash time (skipped at ≤ 0).
//!
//! **Per corona** (`FUN_004a5790`): p = the locator in view space, d = |p|, θ = acos(p̂′ · dir) with dir the
//! locator's +Z axis and p̂′ towards the camera; the definition's curve at x = (θ, s·d, s·d, s·d) gives C; the
//! corona's strength I = C.x · C.z · anim (skipped at ≤ 0.05), its half-size S = size · C.y. A sprite whose half
//! extent is at most k = 2 / 1280 · s · d (one pixel at 1280 wide) or whose colour is at most 0.01 is culled.
//! - **Glows** (`Glows`, `FUN_004a4cf0`): G = curve(I, I, I, d); half extents G.w · S · G.xy; colour G.z ·
//!   entry colour · glow colour · intensity; a camera-facing quad rotated by `RotationOffset` − the camera's roll,
//!   its depth pulled `a9531774` metres (−0.5) towards the camera and depth-tested.
//! - **Beams** (`Beams`, `FUN_004a5040`): B = curve(I × 4); a quad along the lamp axis from `034c050f`.x to .y
//!   metres, half widths B.x / B.y across n = p × dir; colour B.z × …; depth-tested.
//! - **Flares** (`Flares`, `FUN_004a52e0`): F = curve(I × 4); half extents S · F.xy, centred at (`Position` ·
//!   p.x, `Position` · p.y, p.z) in view space (ghosts along the line through the screen centre), turned to the
//!   radial direction when `2b9cc12c`.w = 1; not depth-tested but scaled by the corona's visibility: 7 × 7
//!   depth taps one pixel apart around the point 0.5 m (`c4e3e3a7`) nearer the camera (`Vfx_CoronaVisibilityTest`),
//!   the fraction behind it. Here the test runs in the flare's pixel program against the depth prepass.
//!
//! Effect masks (`FUN_0057a5e0`): race cars light groups 0, 4, 6, 7 always (+0x1a8c), 1 with the brake, 2 in
//! reverse, 3 with the cop light bar; traffic groups 0 and 4 only, L = 0.7 (`FUN_005fd190`), parked cars none, the
//! flash time × (0.9 + 0.2 × the car's random).
//!
//! MODEL / open: s = tan(half the horizontal field of view) (the view's +0x8b908, inferred from the one-pixel
//! cull); the flares are added to the HDR scene like the glows (the game draws them after the tonemap with the
//! exposure applied, layer 8); the headlight glint (`VfxGlobals.20feab72`) and `ExposureScaleCoronas` are not
//! drawn. World objects' coronas (street lamps, traffic lights, speed cameras) come from `world_coronas.rs` through
//! [`WorldCoronas`]. `MW_NO_CORONAS=1` turns them all off (`MW_NO_WORLD_CORONAS=1` only the world's);
//! `MW_CORONA_LOG=1` logs the sprite counts.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, mpsc};

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::{CompressedImageFormatSupport, CompressedImageFormats};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, CompareFunction, RenderPipelineDescriptor,
    ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use mw_assets::coronas::{CarCorona, CoronaDef, Sprite};
use mw_bundle::ResourceId;

use crate::drive::{Car, CarControls};
use crate::weather::GAME_UNIT;

pub struct CoronasPlugin;

impl Plugin for CoronasPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "corona.wgsl");
        app.add_plugins((MaterialPlugin::<GlowMaterial>::default(), MaterialPlugin::<FlareMaterial>::default()));
        app.add_systems(PostUpdate, view_behind.before(bevy::transform::TransformSystems::Propagate));
        if std::env::var_os("MW_NO_CORONAS").is_some() {
            return;
        }
        app.init_resource::<Coronas>()
            .init_resource::<WorldCoronas>()
            .add_systems(Startup, load_globals)
            .add_systems(Update, (request_materials, receive_materials))
            .add_systems(PostUpdate, build.after(bevy::transform::TransformSystems::Propagate));
    }
}

/// `MW_VIEW_BEHIND=<m>[,<height>[,<front>]]` (checks): the camera that far behind the player's car (2 m up unless
/// given), looking at it, after the driving camera has moved; with a third value `1`, that far in front of the
/// nearest lit traffic car instead (its headlights).
#[allow(clippy::type_complexity)]
fn view_behind(
    player: Query<&Transform, (With<crate::drive::Player>, With<Car>)>,
    traffic: Query<(&Transform, &CarCoronas), Without<Car>>,
    mut camera: Query<&mut Transform, (crate::MainCamera, Without<Car>, Without<CarCoronas>)>,
) {
    static AT: std::sync::LazyLock<Option<(f32, f32, bool)>> = std::sync::LazyLock::new(|| {
        let v = std::env::var("MW_VIEW_BEHIND").ok()?;
        let mut it = v.split(',').filter_map(|x| x.trim().parse::<f32>().ok());
        Some((it.next()?, it.next().unwrap_or(2.0), it.next().is_some_and(|f| f > 0.5)))
    });
    let Some((back, height, front)) = *AT else { return };
    let (Ok(car), Ok(mut cam)) = (player.single(), camera.single_mut()) else { return };
    let target =
        if front {
            let Some(t) = traffic.iter().filter(|(_, c)| c.traffic && c.level > 0.0).map(|(t, _)| t).min_by(|a, b| {
                a.translation.distance(car.translation).total_cmp(&b.translation.distance(car.translation))
            }) else {
                return;
            };
            (*t, -back)
        } else {
            (*car, back)
        };
    let (t, back) = target;
    let fwd = t.rotation * Vec3::Z;
    let eye = t.translation - fwd * back + Vec3::Y * height;
    *cam = Transform::from_translation(eye).looking_at(t.translation + Vec3::Y * 1.2, Vec3::Y);
}

/// Coronas of world objects for this frame (`world_coronas.rs`: street lamps, traffic lights, speed cameras;
/// `FUN_00497180` → `FUN_004a6fa0` → `FUN_004a6b00`). They go through the same `FUN_004a5790` with colour,
/// intensity and size 1 and the lamp's level as the animation value.
#[derive(Resource, Default)]
pub struct WorldCoronas {
    pub items: Vec<WorldCorona>,
    /// Definitions whose sprite materials may not be loaded yet.
    pub new_defs: Vec<Arc<CoronaDef>>,
}

pub struct WorldCorona {
    pub def: Arc<CoronaDef>,
    pub pos: Vec3,
    pub dir: Vec3,
    pub anim: f32,
}

/// `FUN_004a5790`: coronas below this strength are skipped (`.bss` 0x011040b0).
const MIN_STRENGTH: f32 = 0.05;
/// Sprites whose colour is at most this in every channel are culled (`.bss` 0x011042d0).
const MIN_COLOUR: f32 = 0.01;
/// The size cull: 2 (`.data` 0x010c3e10) / 1280 (`.rdata` 0x00fa2fbc) × s × d.
const CULL_PIXELS: f32 = 2.0 / 1280.0;
/// Traffic cars' L (`FUN_005fd190`: 0.7 + 0.5 × the flash byte / 255; the flash is not routed here).
const TRAFFIC_LEVEL: f32 = 0.7;
/// The effect groups traffic lights (`FUN_005fd190`: mask 0x11).
const TRAFFIC_MASK: u32 = 0x11;
/// Race cars: 0x1a8c set on every car → groups 0, 4, 6, 7 (`FUN_0057a5e0`).
const RACE_MASK: u32 = 0xd1;
/// Seconds the brake group stays lit after the brake is released (`FUN_0057a5e0`, `.rdata` 0x00f8a810).
const BRAKE_HOLD: f32 = 0.5;

/// A car's coronas.
#[derive(Component)]
pub struct CarCoronas {
    pub list: Arc<Vec<CarCorona>>,
    /// Where the car's model sits in the entity (traffic models are lifted by their ride height).
    pub offset: Vec3,
    /// L: the car's light level (1 for race cars, 0.7 for traffic, 0 for parked cars).
    pub level: f32,
    /// Traffic: groups 0 and 4 only.
    pub traffic: bool,
    /// The flash time's rate (traffic: 0.9 + 0.2 × random).
    pub rate: f32,
}

impl CarCoronas {
    pub fn new(list: Arc<Vec<CarCorona>>, offset: Vec3) -> Self {
        Self { list, offset, level: 1.0, traffic: false, rate: 1.0 }
    }

    pub fn traffic(list: Arc<Vec<CarCorona>>, offset: Vec3, parked: bool, seed: u32) -> Self {
        let r = (seed.wrapping_mul(2_654_435_761) >> 8) as f32 / (1u32 << 24) as f32;
        Self { list, offset, level: if parked { 0.0 } else { TRAFFIC_LEVEL }, traffic: true, rate: 0.9 + 0.2 * r }
    }
}

/// `VfxGlobals.VehicleCoronaScalesForNight` (intensity, size) per group: 1, 4/6, 7.
#[derive(Resource, Clone, Copy)]
struct NightScales([[f32; 2]; 3]);

impl Default for NightScales {
    fn default() -> Self {
        Self([[1.0, 1.0], [1.0, 1.0], [1.3, 1.0]])
    }
}

impl NightScales {
    fn get(&self, group: u8, night: f32) -> (f32, f32) {
        let f = match group {
            1 => self.0[0],
            4 | 6 => self.0[1],
            7 => self.0[2],
            _ => return (1.0, 1.0),
        };
        (1.0 + (f[0] - 1.0) * night, 1.0 + (f[1] - 1.0) * night)
    }
}

fn load_globals(game: Option<Res<crate::Game>>, mut commands: Commands) {
    let mut scales = NightScales::default();
    if let Some(game) = game
        && let Ok(objs) = (|| -> anyhow::Result<Vec<mw_formats::genesys::Object>> {
            let b = game.store.bundle("GLOBALEFFECTS.BNDL").ok_or_else(|| anyhow::anyhow!("no bundle"))?;
            let mut out = Vec::new();
            for e in b.entries_of_type(mw_bundle::ResourceType::GenesysObject) {
                if let Ok(o) = game.store.decode_genesys(&b.load(e)?) {
                    out.push(o);
                }
            }
            Ok(out)
        })()
        && let Some(g) = objs.iter().find(|o| o.type_name == "Genesys.Gen.VfxGlobals")
        && let Some(mw_formats::genesys::Value::Struct(s)) =
            g.get(mw_formats::genesys::field_hash("VehicleCoronaScalesForNight"))
    {
        let f = |h: u32| match s.get(h) {
            Some(mw_formats::genesys::Value::Float(v)) => *v,
            _ => 1.0,
        };
        scales = NightScales([
            [f(0x99fe_35ca), f(0xdd63_ad27)],
            [f(0xa103_da39), f(0xdb2c_0603)],
            [f(0x47a5_e70e), f(0xe5e9_3aaf)],
        ]);
    }
    commands.insert_resource(scales);
}

// --- Rendering ------------------------------------------------------------------------------------------------

#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct CoronaParams {
    /// x = cd/m² per game unit, y = the visibility test's tap spacing (pixels), z = flare (1: visibility test).
    misc: Vec4,
}

macro_rules! corona_material {
    ($name:ident, $flare:expr) => {
        #[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
        pub struct $name {
            #[uniform(0)]
            params: CoronaParams,
            #[texture(1)]
            #[sampler(2)]
            texture: Option<Handle<Image>>,
        }

        impl Material for $name {
            fn vertex_shader() -> ShaderRef {
                "embedded://mostrusted/corona.wgsl".into()
            }
            fn fragment_shader() -> ShaderRef {
                "embedded://mostrusted/corona.wgsl".into()
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
                    Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
                    Mesh::ATTRIBUTE_TANGENT.at_shader_location(4),
                    Mesh::ATTRIBUTE_COLOR.at_shader_location(5),
                ])?];
                descriptor.primitive.cull_mode = None;
                if let Some(ds) = descriptor.depth_stencil.as_mut() {
                    ds.depth_write_enabled = Some(false);
                    if $flare {
                        // `Vfx_CoronaFlare`: no depth test; the visibility fraction stands in for it.
                        ds.depth_compare = Some(CompareFunction::Always);
                    }
                }
                // ONE, INV_SRC_ALPHA with alpha 0 (`Vfx_Corona`'s material state): additive.
                let add = BlendComponent {
                    src_factor: BlendFactor::One,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::Add,
                };
                if let Some(t) =
                    descriptor.fragment.as_mut().and_then(|f| f.targets.get_mut(0)).and_then(|t| t.as_mut())
                {
                    t.blend = Some(BlendState { color: add, alpha: add });
                }
                Ok(())
            }
        }
    };
}

corona_material!(GlowMaterial, false);
corona_material!(FlareMaterial, true);

/// One corner of a sprite.
#[derive(Clone, Copy)]
struct Vertex {
    pos: Vec3,
    uv: Vec2,
    /// xyz = the depth-bias target or the visibility test point (world), w = bias (m) or 1 (flare test).
    extra: Vec4,
    colour: Vec4,
}

const EMPTY: Vertex = Vertex { pos: Vec3::ZERO, uv: Vec2::ZERO, extra: Vec4::ZERO, colour: Vec4::ZERO };

struct Batch {
    mesh: Handle<Mesh>,
    vertices: Vec<Vertex>,
    shown: bool,
}

type MaterialJob = mpsc::Receiver<Vec<(ResourceId, bool, Option<Image>)>>;

#[derive(Resource, Default)]
struct Coronas {
    batches: HashMap<ResourceId, Batch>,
    requested: HashSet<ResourceId>,
    jobs: Vec<Mutex<MaterialJob>>,
    log_timer: f32,
}

fn sprite_materials(def: &CoronaDef) -> impl Iterator<Item = ResourceId> + '_ {
    def.glows[1].iter().chain(&def.beams).chain(&def.flares).filter_map(|s| s.material)
}

/// Load the sprite materials of cars that have arrived (textures on a worker thread).
fn request_materials(
    game: Option<Res<crate::Game>>,
    support: Option<Res<CompressedImageFormatSupport>>,
    cars: Query<&CarCoronas, Added<CarCoronas>>,
    mut coronas: ResMut<Coronas>,
    mut world: ResMut<WorldCoronas>,
) {
    let (Some(game), Some(support)) = (game, support) else { return };
    let mut want = Vec::new();
    for c in &cars {
        for e in c.list.iter() {
            for m in e.def.iter().flat_map(|d| sprite_materials(d)) {
                if coronas.requested.insert(m) {
                    want.push(m);
                }
            }
        }
    }
    for d in std::mem::take(&mut world.new_defs) {
        for m in sprite_materials(&d) {
            if coronas.requested.insert(m) {
                want.push(m);
            }
        }
    }
    if std::env::var_os("MW_CORONA_LOG").is_some() && !cars.is_empty() {
        info!("coronas: {} new cars, {} new materials", cars.iter().count(), want.len());
    }
    if want.is_empty() {
        return;
    }
    let bc = support.0.contains(CompressedImageFormats::BC);
    let store = game.store.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let out = want
            .into_iter()
            .filter_map(|id| {
                let m = store.particle_material(id).ok()?;
                let flare = m.shader.contains("Flare");
                let image =
                    m.texture.and_then(|t| store.texture(t).ok()).and_then(|t| crate::convert::image(&t, true, bc));
                Some((id, flare, image))
            })
            .collect();
        let _ = tx.send(out);
    });
    coronas.jobs.push(Mutex::new(rx));
}

fn receive_materials(
    mut commands: Commands,
    mut coronas: ResMut<Coronas>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut glows: ResMut<Assets<GlowMaterial>>,
    mut flares: ResMut<Assets<FlareMaterial>>,
) {
    let mut done = Vec::new();
    coronas.jobs.retain(|j| match j.lock().unwrap().try_recv() {
        Ok(v) => {
            done.extend(v);
            false
        }
        Err(mpsc::TryRecvError::Empty) => true,
        Err(_) => false,
    });
    for (id, flare, image) in done {
        // A degenerate quad until the first sprites (an empty mesh upsets the mesh allocator).
        let mesh = meshes.add(mesh_of(&[EMPTY; 4]));
        let params = CoronaParams { misc: Vec4::new(GAME_UNIT, 1.0, if flare { 1.0 } else { 0.0 }, 0.0) };
        let texture = image.map(|i| images.add(i));
        let mut e = commands.spawn((Mesh3d(mesh.clone()), Transform::IDENTITY, Visibility::Hidden, NoFrustumCulling));
        if flare {
            e.insert(MeshMaterial3d(flares.add(FlareMaterial { params, texture })));
        } else {
            e.insert(MeshMaterial3d(glows.add(GlowMaterial { params, texture })));
        }
        coronas.batches.insert(id, Batch { mesh, vertices: Vec::new(), shown: false });
        let _ = e.id();
    }
}

fn mesh_of(v: &[Vertex]) -> Mesh {
    let quads = v.len() / 4;
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, v.iter().map(|v| v.pos.to_array()).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, v.iter().map(|v| v.uv.to_array()).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, v.iter().map(|v| v.extra.to_array()).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, v.iter().map(|v| v.colour.to_array()).collect::<Vec<_>>())
        .with_inserted_indices(Indices::U32(
            (0..quads as u32).flat_map(|q| [1, 2, 0, 0, 2, 3].map(|i| q * 4 + i)).collect(),
        ))
}

/// The camera as `FUN_004a5790` sees it.
struct View {
    eye: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    /// s: tan(half the horizontal field of view).
    s: f32,
    /// The camera's roll (degrees).
    roll: f32,
}

impl View {
    /// World → the game's view space (x right, y up, z forward).
    fn to_view(&self, p: Vec3) -> Vec3 {
        let d = p - self.eye;
        Vec3::new(d.dot(self.right), d.dot(self.up), d.dot(self.forward))
    }
    fn to_world(&self, v: Vec3) -> Vec3 {
        self.eye + self.right * v.x + self.up * v.y + self.forward * v.z
    }
}

/// What the corona's sprites need (`FUN_004a5790`'s context).
struct Ctx {
    /// View-space position and direction.
    p: Vec3,
    dir: Vec3,
    colour: Vec4,
    intensity: f32,
    size: Vec2,
    strength: f32,
    d: f32,
    k: f32,
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn build(
    time: Res<Time>,
    night: Option<Res<crate::world_lights::NightLevel>>,
    scales: Option<Res<NightScales>>,
    camera: Query<(&GlobalTransform, &Projection), crate::MainCamera>,
    cars: Query<(
        Entity,
        &GlobalTransform,
        &CarCoronas,
        Option<&Car>,
        Option<&CarControls>,
        Option<&crate::police_fx::FlashCar>,
        &InheritedVisibility,
    )>,
    mut coronas: ResMut<Coronas>,
    world: Res<WorldCoronas>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut visibility: Query<&mut Visibility>,
    batch_entities: Query<(Entity, &Mesh3d)>,
    mut braked: Local<HashMap<Entity, f32>>,
) {
    let coronas = &mut *coronas;
    let Some((cam, projection)) = camera.iter().next() else { return };
    let night = night.map_or(0.0, |n| n.0);
    let scales = scales.map(|s| *s).unwrap_or_default();
    let s = match projection {
        Projection::Perspective(p) => (p.fov * 0.5).tan() * p.aspect_ratio,
        _ => 1.0,
    };
    let right = cam.right().as_vec3();
    let up = cam.up().as_vec3();
    // Roll: the angle of the camera's right axis out of the horizontal plane.
    let roll = right.y.clamp(-1.0, 1.0).asin().to_degrees();
    let view = View { eye: cam.translation(), right, up, forward: cam.forward().as_vec3(), s, roll };
    for b in coronas.batches.values_mut() {
        b.vertices.clear();
    }
    let now = time.elapsed_secs();
    let mut counts = [0usize; 3];
    braked.retain(|e, _| cars.contains(*e));
    for (e, t, c, car, controls, flash, vis) in &cars {
        if !vis.get() || c.level <= 0.0 {
            continue;
        }
        let mut mask = if c.traffic { TRAFFIC_MASK } else { RACE_MASK };
        if !c.traffic {
            // The brake lights stay on for 0.5 s (`.rdata` 0x00f8a810) after the brake is let go: car +1000 is
            // zeroed while braking and counts the scaled frame time, group 1 lit below 0.5 (`FUN_0057a5e0`; its
            // player-state test `FUN_006c37e0` taken as true for every race car).
            if controls.is_some_and(|k| k.brake > 0.0) {
                braked.insert(e, now);
            }
            if braked.get(&e).is_some_and(|&at| now - at < BRAKE_HOLD) {
                mask |= 1 << 1;
            }
            if car.is_some_and(|car| car.gear < 0) {
                mask |= 1 << 2;
            }
        }
        if flash.is_some_and(|f| f.bar_on()) {
            mask |= 1 << 3;
        }
        let rate = flash.map_or(c.rate, |f| f.rate);
        let clock = now * rate;
        let m = t.affine();
        for e in c.list.iter() {
            if mask & (1 << e.group) == 0 {
                continue;
            }
            let Some(def) = &e.def else { continue };
            let gate = match e.gate {
                1 => 1.0 - night,
                2 => night,
                _ => 1.0,
            };
            let (gi, gs) = scales.get(e.group, night);
            let intensity = gate * e.brightness * c.level * c.level * gi;
            if intensity <= 0.0 {
                continue;
            }
            let size = Vec2::new(e.size[0], e.size[1]) * gs;
            for l in &e.locators {
                let anim = e.animation.as_ref().map_or(1.0, |a| a.value(clock));
                if anim <= 0.0 {
                    continue;
                }
                let q = Quat::from_array(l.rotation);
                let local = Vec3::from(l.position) + c.offset;
                let axis = q * Vec3::Z;
                let mirrors: &[f32] = if e.mirror { &[1.0, -1.0] } else { &[1.0] };
                for &mx in mirrors {
                    let mirror = Vec3::new(mx, 1.0, 1.0);
                    let pos = m.transform_point3(local * mirror);
                    let dir = m.transform_vector3(axis * mirror).normalize_or_zero();
                    corona(&view, def, e.colour, intensity, size, anim, pos, dir, &mut coronas.batches, &mut counts);
                }
            }
        }
    }
    // World objects (`FUN_004a6b00`): colour, intensity and size 1, the lamp level as the animation value.
    for w in &world.items {
        corona(&view, &w.def, [1.0; 4], 1.0, Vec2::ONE, w.anim, w.pos, w.dir, &mut coronas.batches, &mut counts);
    }
    for (entity, mesh3d) in &batch_entities {
        let Some(b) = coronas.batches.values_mut().find(|b| b.mesh == mesh3d.0) else { continue };
        let show = !b.vertices.is_empty();
        if show && let Some(mut mesh) = meshes.get_mut(&b.mesh) {
            *mesh = mesh_of(&b.vertices);
        }
        if show != b.shown {
            b.shown = show;
            if let Ok(mut v) = visibility.get_mut(entity) {
                *v = if show { Visibility::Visible } else { Visibility::Hidden };
            }
        }
    }
    coronas.log_timer -= time.delta_secs();
    if std::env::var_os("MW_CORONA_LOG").is_some() && coronas.log_timer <= 0.0 {
        coronas.log_timer = 1.0;
        info!(
            "coronas: {} glows, {} beams, {} flares (night {night:.2}, s {s:.3}, {} materials)",
            counts[0],
            counts[1],
            counts[2],
            coronas.batches.len()
        );
    }
}

/// `FUN_004a5790` for one corona at `pos` facing `dir` (world).
#[allow(clippy::too_many_arguments)]
fn corona(
    view: &View,
    def: &CoronaDef,
    colour: [f32; 4],
    intensity: f32,
    size: Vec2,
    anim: f32,
    pos: Vec3,
    dir: Vec3,
    batches: &mut HashMap<ResourceId, Batch>,
    counts: &mut [usize; 3],
) {
    let p = view.to_view(pos);
    let dir_v = Vec3::new(dir.dot(view.right), dir.dot(view.up), dir.dot(view.forward));
    let d = p.length();
    if d <= 1e-3 || p.z <= 0.0 {
        return;
    }
    let theta = (-p / d).dot(dir_v).clamp(-1.0, 1.0).acos();
    let sd = view.s * d;
    let c = def.curve.eval([theta, sd, sd, sd], [1.0; 4]);
    let strength = c[0] * c[2] * anim;
    if strength <= MIN_STRENGTH {
        return;
    }
    let ctx = Ctx {
        p,
        dir: dir_v,
        colour: Vec4::from(colour),
        intensity,
        size: size * c[1],
        strength,
        d,
        k: CULL_PIXELS * view.s * d,
    };
    for g in &def.glows[1] {
        if glow(view, &ctx, g, batches) {
            counts[0] += 1;
        }
    }
    for b in &def.beams {
        if beam(view, &ctx, b, batches) {
            counts[1] += 1;
        }
    }
    // The visibility test point: 0.5 m (`c4e3e3a7`) towards the camera.
    let test = view.to_world(p + p / d * def.values[2]);
    for f in &def.flares {
        if flare(view, &ctx, f, test, batches) {
            counts[2] += 1;
        }
    }
}

fn sprite_colour(ctx: &Ctx, s: &Sprite, k: f32) -> Option<Vec4> {
    let rgb = k * ctx.colour.truncate() * Vec3::from_slice(&s.colour[..3]) * ctx.intensity;
    (rgb.max_element() > MIN_COLOUR).then(|| rgb.extend(ctx.colour.w * s.colour[3]))
}

fn push_quad(
    batches: &mut HashMap<ResourceId, Batch>,
    s: &Sprite,
    corners: [Vec3; 4],
    uvs: [Vec2; 4],
    extra: Vec4,
    colour: Vec4,
) -> bool {
    let Some(b) = s.material.and_then(|m| batches.get_mut(&m)) else { return false };
    for (p, uv) in corners.into_iter().zip(uvs) {
        b.vertices.push(Vertex { pos: p, uv, extra, colour });
    }
    true
}

const QUAD_UV: [Vec2; 4] = [Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)];

/// `FUN_004a4cf0`.
fn glow(view: &View, ctx: &Ctx, s: &Sprite, batches: &mut HashMap<ResourceId, Batch>) -> bool {
    let g = s.curve.eval([ctx.strength, ctx.strength, ctx.strength, ctx.d], [1.0; 4]);
    let half = Vec2::new(g[3] * ctx.size.x * g[0], g[3] * ctx.size.y * g[1]);
    if half.x <= ctx.k || half.y <= ctx.k {
        return false;
    }
    let Some(colour) = sprite_colour(ctx, s, g[2]) else { return false };
    if std::env::var_os("MW_CORONA_TRACE").is_some() {
        eprintln!(
            "glow d {:.1} strength {:.3} size {:.2?} g {:.3?} half {:.2?} colour {:.3?} mat {:?}",
            ctx.d, ctx.strength, ctx.size, g, half, colour, s.material
        );
    }
    let phi = (s.value_b - view.roll).to_radians();
    let a = Vec2::new(phi.cos(), -phi.sin()) * half.x;
    let b = Vec2::new(phi.sin(), phi.cos()) * half.y;
    let c = ctx.p;
    let corner = |sa: f32, sb: f32| view.to_world(c + (a * sa + b * sb).extend(0.0));
    // w: the depth bias (`a9531774`, m along the view axis; negative = towards the camera).
    let extra = view.to_world(c).extend(s.value_a);
    push_quad(
        batches,
        s,
        [corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)],
        QUAD_UV,
        extra,
        colour,
    )
}

/// `FUN_004a5040`.
fn beam(view: &View, ctx: &Ctx, s: &Sprite, batches: &mut HashMap<ResourceId, Batch>) -> bool {
    let b = s.curve.eval([ctx.strength; 4], [1.0; 4]);
    if b[0].max(b[1]) <= ctx.k {
        return false;
    }
    let Some(colour) = sprite_colour(ctx, s, b[2]) else { return false };
    let n = ctx.p.cross(ctx.dir).normalize_or_zero();
    let ps = ctx.p + ctx.dir * s.vector[0];
    let pe = ctx.p + ctx.dir * s.vector[1];
    let corners = [ps - n * b[0], pe - n * b[1], pe + n * b[1], ps + n * b[0]].map(|v| view.to_world(v));
    let uvs = [Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)];
    let extra = view.to_world(ctx.p).extend(s.vector[3]);
    push_quad(batches, s, corners, uvs, extra, colour)
}

/// `FUN_004a52e0`.
fn flare(view: &View, ctx: &Ctx, s: &Sprite, test: Vec3, batches: &mut HashMap<ResourceId, Batch>) -> bool {
    let f = s.curve.eval([ctx.strength; 4], [1.0; 4]);
    let half = Vec2::new(ctx.size.x * f[0], ctx.size.y * f[1]);
    if half.x <= ctx.k || half.y <= ctx.k {
        return false;
    }
    let Some(colour) = sprite_colour(ctx, s, f[2]) else { return false };
    let c = Vec3::new(s.vector[0] * ctx.p.x, s.vector[1] * ctx.p.y, ctx.p.z);
    let xy = Vec2::new(ctx.p.x, ctx.p.y);
    let n = if s.vector[3] == 1.0 && xy.length_squared() > 1e-4 { xy.normalize() } else { Vec2::X };
    let phi = (s.value_b + view.roll).to_radians();
    let (sn, cs) = phi.sin_cos();
    let nx = Vec2::new(n.x * cs - n.y * sn, n.x * sn + n.y * cs);
    let ny = Vec2::new(-nx.y, nx.x);
    let a = nx * half.x;
    let b = ny * half.y;
    let corner = |sa: f32, sb: f32| view.to_world(c + (a * sa + b * sb).extend(0.0));
    push_quad(
        batches,
        s,
        [corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)],
        QUAD_UV,
        test.extend(1.0),
        colour,
    )
}
