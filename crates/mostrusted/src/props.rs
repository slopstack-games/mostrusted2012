//! Collision for props, compound objects and dynamic objects, and collision surfaces of the world.
//!
//! Every placed object with a GeneSys `PhysicalDefinition` (`mw_assets::props`) becomes a rigid body
//! with the definition's collision volumes (boxes and cylinders; built on the loader thread), mass,
//! centre of mass, friction and restitution. It starts out pinned (`RigidBody::Static`), like the
//! game's props standing in the world. A hit breaks it free when the impact exceeds the object's
//! break force (`AdditionalInfo` field `67f0233c`, unnamed: 0 for cones and bins, 500–2500 for signs
//! and fences, 5000 for street lamps, 20000 for gantries, 100000 = never); then it becomes a dynamic
//! body that falls and tumbles. Objects with mass 0 never break free. Knocked objects are put back
//! (or removed, `mbRemoveInsteadOfReset`) once the car is farther away than the prop's reset distance
//! (`mfResetDistanceSquared`, 100 m on every prop; compounds and dynamic objects use 100 m too).
//!
//! Units of the break force: read as the impulse (N s) the hitting body would have to deliver to stop
//! against the object, `mass × approach speed`. With a 1.5 t car that puts the break speeds at 0 for
//! cones, 1–6 km/h for signs and fences, 12 km/h for lamp posts and 48 km/h for gantries; the street
//! lamps' `ForceToSmash` (10000) would be 24 km/h. A hypothesis that gives plausible speeds; the
//! game's own rule is in the executable (research log, "prop physics").
//!
//! Compound objects (lamp posts, traffic lights) have per-part physical definitions too; the parts
//! have no models of their own that are decoded yet, so a compound breaks free as one body (the
//! root definition's volumes and mass).
//!
//! Collision layers: the world (`PolygonSoupList` and anything without layers) is [`GameLayer::World`];
//! the car's body is [`GameLayer::Car`] and a low bumper box that only meets props is
//! [`GameLayer::CarBumper`] (the body box sits above the wheels, so low props would slip under it);
//! props are [`GameLayer::Prop`] and collide with the world, the car and each other. Traffic cars are
//! [`GameLayer::Traffic`] (`traffic.rs`): lane followers meet the car, props and traffic; crashed ones the world
//! too. The wheels' ray casts only see the world.

use std::sync::Arc;

use avian3d::prelude::*;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use mw_assets::props::ObjectKind;
use mw_assets::surfaces::SurfaceInfo;
use mw_bundle::ResourceId;
use mw_formats::CollisionTag;

use crate::drive::Car;

/// Collision layers. The default (first) layer is the world, so colliders spawned without layers are
/// world geometry.
#[derive(PhysicsLayer, Default, Clone, Copy, Debug)]
pub enum GameLayer {
    #[default]
    World,
    Car,
    /// A low box on the car that only collides with props.
    CarBumper,
    Prop,
    /// Traffic cars (`traffic.rs`).
    Traffic,
}

impl GameLayer {
    pub fn car() -> CollisionLayers {
        CollisionLayers::new(GameLayer::Car, [GameLayer::World, GameLayer::Prop, GameLayer::Traffic])
    }
    pub fn car_bumper() -> CollisionLayers {
        CollisionLayers::new(GameLayer::CarBumper, [GameLayer::Prop, GameLayer::Traffic])
    }
    pub fn prop() -> CollisionLayers {
        CollisionLayers::new(
            GameLayer::Prop,
            [GameLayer::World, GameLayer::Car, GameLayer::CarBumper, GameLayer::Prop, GameLayer::Traffic],
        )
    }
}

/// Physics of an object definition, built on the loader thread.
#[derive(Debug, Clone)]
pub struct PropPhysics {
    pub object: ResourceId,
    pub kind: ObjectKind,
    /// Compound collider in object space.
    pub collider: Collider,
    /// kg; 0 = immovable.
    pub mass: f32,
    /// Density that gives the collider `mass` (avian derives the inertia from it).
    pub density: f32,
    pub centre_of_mass: Vec3,
    pub friction: f32,
    pub restitution: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    /// Impulse (N s) that breaks the object free (`f32::INFINITY` = never).
    pub break_force: f32,
    /// `ForceToSmash` (compound objects): logged, parts are not split yet.
    pub smash_force: Option<f32>,
    pub reset_distance: f32,
    pub remove_instead_of_reset: bool,
}

impl PropPhysics {
    /// Build from decoded physics; `None` if the object has no collision volumes.
    pub fn new(p: &mw_assets::props::ObjectPhysics) -> Option<Self> {
        use mw_assets::props::Volume;
        let def = p.definition.as_ref()?;
        let mut shapes: Vec<(Vec3, Quat, Collider)> = Vec::new();
        let mut volume = 0.0;
        let mut com = Vec3::ZERO;
        let mut mass = 0.0;
        let (mut friction, mut restitution, mut lin, mut ang) = (0.0, 0.0, 0.0, 0.0);
        for body in &def.bodies {
            let body_m = crate::loader::game_matrix(&body.transform);
            for v in &body.volumes {
                // Cylinders and capsules: the game's axis is local Z, parry's is Y (no nested compounds,
                // parry rejects them, so the axis turn goes into the volume's own rotation).
                let to_z = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
                let (m, shape, vol, turn) = match v {
                    Volume::Box { transform, half_extents: h } => {
                        let [x, y, z] = h.map(|c| c.max(0.01));
                        (transform, Collider::cuboid(2.0 * x, 2.0 * y, 2.0 * z), 8.0 * x * y * z, Quat::IDENTITY)
                    }
                    Volume::Sphere { transform, radius } => {
                        (transform, Collider::sphere(*radius), 4.19 * radius.powi(3), Quat::IDENTITY)
                    }
                    Volume::Cylinder { transform, radius, half_length } => (
                        transform,
                        Collider::cylinder(*radius, 2.0 * half_length),
                        std::f32::consts::PI * radius * radius * 2.0 * half_length,
                        to_z,
                    ),
                    Volume::Capsule { transform, radius, half_length } => (
                        transform,
                        Collider::capsule(*radius, 2.0 * half_length),
                        std::f32::consts::PI * radius * radius * 2.0 * half_length,
                        to_z,
                    ),
                    // Convex hull resources are not decoded (none are used by world objects).
                    Volume::Hull { .. } => continue,
                };
                let (_, rot, pos) = (body_m * crate::loader::game_matrix(m)).to_scale_rotation_translation();
                let rot = rot.normalize() * turn;
                shapes.push((pos, rot, shape));
                volume += vol;
            }
            let [cx, cy, cz] = body.com_offset;
            com += body.mass * body_m.transform_point3(Vec3::new(cx, cy, cz));
            mass += body.mass;
            friction = body.friction;
            restitution = body.restitution;
            lin = body.linear_drag;
            ang = body.angular_drag;
        }
        if shapes.is_empty() {
            return None;
        }
        let info = def.info.unwrap_or_default();
        let break_force =
            if mass <= 0.0 || info.force_67f0233c >= 100_000.0 { f32::INFINITY } else { info.force_67f0233c };
        Some(Self {
            object: p.object,
            kind: p.kind,
            collider: Collider::compound(shapes),
            mass,
            density: if volume > 0.0 { mass.max(1.0) / volume } else { 1.0 },
            centre_of_mass: if mass > 0.0 { com / mass } else { Vec3::ZERO },
            friction,
            restitution,
            // `LinearDrag` / `AngularDrag` are 1 or 10 (0.1 on unconfigured objects); a tenth of them as
            // avian damping keeps a knocked prop from sliding forever without making it float.
            linear_damping: 0.1 * lin,
            angular_damping: 0.1 * ang,
            break_force,
            smash_force: info.force_to_smash,
            reset_distance: p.reset_distance.unwrap_or(100.0),
            remove_instead_of_reset: p.remove_instead_of_reset,
        })
    }
}

/// A placed object with physics.
#[derive(Component)]
pub struct PropBody {
    pub physics: Arc<PropPhysics>,
    /// Where it stands in the world (restored on reset).
    pub home: Transform,
    /// Pinned in place (static) until hit hard enough.
    pub pinned: bool,
    /// Carries a rigid body and collider (near the car; [`stream_props`]).
    pub active: bool,
}

/// Asks [`unpin_props`] to break a prop free after this physics step, with the momentum of the hit.
#[derive(Component)]
struct Unpin {
    /// Velocity the prop takes from the hit.
    velocity: Vec3,
    /// The body that hit it, and the velocity change it gets in return.
    hitter: Option<(Entity, Vec3)>,
}

/// Counts for logs and tests.
#[derive(Resource, Default, Debug)]
pub struct PropStats {
    /// Props with a collider right now.
    pub active: usize,
    pub knocked: usize,
    pub reset: usize,
}

pub struct PropsPlugin;

impl Plugin for PropsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PropStats>()
            .init_resource::<WorldSurfaces>()
            .add_systems(FixedPostUpdate, unpin_props.after(PhysicsSystems::StepSimulation))
            .add_systems(Update, (reset_far_props, stream_props, track_car_surface));
    }
}

/// Spawn a prop: the render parts as children of a rigid body carrying the collider.
pub fn spawn_prop(
    commands: &mut Commands,
    physics: Arc<PropPhysics>,
    transform: Transform,
    parts: &crate::ModelParts,
    range: Option<crate::shadow_cull::ShadowRange>,
) {
    commands
        .spawn((transform, Visibility::default(), PropBody { physics, home: transform, pinned: true, active: false }))
        .with_children(|c| {
            for (mesh, mat) in parts {
                let mut e =
                    c.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), crate::shadow_cull::ShadowClass::Prop));
                if let Some(r) = range {
                    e.insert(r);
                }
            }
        });
}

/// The rigid body and collider of a prop, inserted while it is near the car.
fn physics_bundle(p: &PropPhysics) -> impl Bundle {
    (
        RigidBody::Static,
        p.collider.clone(),
        GameLayer::prop(),
        ColliderDensity(p.density),
        CenterOfMass(p.centre_of_mass),
        Friction::new(p.friction),
        Restitution::new(p.restitution),
        LinearDamping(p.linear_damping),
        AngularDamping(p.angular_damping),
        // In the same bundle as the collider: avian reads the hook flags when the collider enters its tree.
        if p.break_force.is_finite() { ActiveCollisionHooks::MODIFY_CONTACTS } else { ActiveCollisionHooks::empty() },
    )
}

/// Props get their collider within this distance of the car (m)...
const ACTIVATE_DISTANCE: f32 = 120.0;
/// ...and lose it again beyond this one (while pinned).
const DEACTIVATE_DISTANCE: f32 = 160.0;

/// Stream prop physics with the car: tens of thousands of static bodies cost avian several ms a frame
/// even when nothing touches them, so only props near the car carry a rigid body.
fn stream_props(
    mut commands: Commands,
    time: Res<Time>,
    mut timer: Local<f32>,
    car: Query<&GlobalTransform, With<Car>>,
    mut props: Query<(Entity, &mut PropBody, &Transform)>,
    mut stats: ResMut<PropStats>,
) {
    *timer += time.delta_secs();
    if *timer < 0.1 {
        return;
    }
    *timer = 0.0;
    let car = car.single().ok().map(|t| t.translation());
    let mut active = 0;
    for (e, mut prop, t) in &mut props {
        let d = car.map_or(f32::MAX, |c| c.distance(t.translation));
        if !prop.active && d < ACTIVATE_DISTANCE {
            prop.active = true;
            commands.entity(e).insert(physics_bundle(&prop.physics));
        } else if prop.active && prop.pinned && d > DEACTIVATE_DISTANCE {
            prop.active = false;
            commands.entity(e).remove::<(
                RigidBody,
                Collider,
                CollisionLayers,
                ColliderDensity,
                CenterOfMass,
                Friction,
                Restitution,
                LinearDamping,
                AngularDamping,
                ActiveCollisionHooks,
            )>();
        }
        active += prop.active as usize;
    }
    stats.active = active;
}

/// Contact hook: a dynamic body hitting a pinned prop breaks it free when the impulse it would take
/// to stop against the prop (mass × approach speed) reaches the prop's break force. The contact is
/// dropped for that step and [`unpin_props`] hands the prop the momentum instead, so the car does
/// not bounce off an object that gives way.
#[derive(SystemParam)]
pub struct PropHooks<'w, 's> {
    props: Query<'w, 's, &'static PropBody>,
    bodies: Query<'w, 's, (&'static RigidBody, &'static ComputedMass, &'static LinearVelocity)>,
    /// Lane-following traffic colliders, and the bodies that knock them off their lane (`traffic.rs`).
    traffic: Query<'w, 's, (), With<crate::traffic::TrafficBody>>,
    hitters: Query<'w, 's, (), Or<(With<Car>, With<crate::traffic::TrafficCar>)>>,
}

impl CollisionHooks for PropHooks<'_, '_> {
    fn modify_contacts(&self, contacts: &mut ContactPair, commands: &mut Commands) -> bool {
        // A car (the player's, or crashed traffic) touching a lane follower turns it into a crashing car; the
        // contact is dropped for this step and solved between two dynamic bodies from the next one.
        for (collider, other) in [(contacts.collider1, contacts.body2), (contacts.collider2, contacts.body1)] {
            if self.traffic.get(collider).is_ok() {
                let Some(other) = other else { return true };
                let dynamic = self.bodies.get(other).is_ok_and(|(rb, _, _)| rb.is_dynamic());
                if dynamic && self.hitters.get(other).is_ok() {
                    commands.entity(collider).insert(crate::traffic::TrafficHit);
                    return false;
                }
                return true;
            }
        }
        let (prop_entity, prop, other, sign) = if let Ok(p) = self.props.get(contacts.collider1) {
            (contacts.collider1, p, contacts.body2, 1.0)
        } else if let Ok(p) = self.props.get(contacts.collider2) {
            (contacts.collider2, p, contacts.body1, -1.0)
        } else {
            return true;
        };
        if !prop.pinned {
            return true;
        }
        let Some(other) = other else { return true };
        let Ok((rb, mass, velocity)) = self.bodies.get(other) else { return true };
        if !rb.is_dynamic() {
            return true;
        }
        // Approach speed along the normal (normal_speed < 0 while closing) and the normal pointing from
        // the other body into the prop.
        let Some((approach, normal)) = contacts
            .manifolds
            .iter()
            .flat_map(|m| m.points.iter().map(move |p| (-p.normal_speed, m.normal)))
            .max_by(|a, b| a.0.total_cmp(&b.0))
        else {
            return true;
        };
        let impulse = mass.value() * approach.max(0.0);
        if impulse < prop.physics.break_force || approach <= 0.0 {
            return true;
        }
        // Normal points from collider1 to collider2: flip it so it points from the hitter into the prop.
        let into_prop = -sign * normal;
        // Perfectly inelastic exchange along the normal, then the prop keeps the hitter's tangential speed.
        let (m1, m2) = (mass.value(), prop.physics.mass.max(1.0));
        let dv = approach * m1 / (m1 + m2);
        let prop_velocity = into_prop * dv + (velocity.0 - into_prop * velocity.0.dot(into_prop)) * 0.5;
        let hitter_dv = -into_prop * approach * m2 / (m1 + m2);
        commands.entity(prop_entity).insert(Unpin { velocity: prop_velocity, hitter: Some((other, hitter_dv)) });
        contacts.manifolds.clear();
        false
    }
}

fn unpin_props(
    mut commands: Commands,
    mut props: Query<(Entity, &mut PropBody, &Unpin, &Transform)>,
    mut velocities: Query<&mut LinearVelocity>,
    mut stats: ResMut<PropStats>,
) {
    for (e, mut prop, unpin, t) in &mut props {
        commands.entity(e).remove::<Unpin>();
        if !prop.pinned {
            continue;
        }
        prop.pinned = false;
        stats.knocked += 1;
        let p = &prop.physics;
        info!(
            "prop {} ({:?}, {:.0} kg) knocked at ({:.1} {:.1} {:.1}), {:.1} m/s; smash force {:?}",
            p.object,
            p.kind,
            p.mass,
            t.translation.x,
            t.translation.y,
            t.translation.z,
            unpin.velocity.length(),
            p.smash_force
        );
        commands.entity(e).insert((RigidBody::Dynamic, LinearVelocity(unpin.velocity)));
        if let Some((hitter, dv)) = unpin.hitter
            && let Ok(mut v) = velocities.get_mut(hitter)
        {
            v.0 += dv;
        }
    }
}

/// Knocked props go back where they stood (or are removed) once the car is beyond their reset distance.
fn reset_far_props(
    mut commands: Commands,
    car: Query<&GlobalTransform, With<Car>>,
    mut props: Query<(Entity, &mut PropBody, &mut Transform)>,
    mut stats: ResMut<PropStats>,
) {
    let Ok(car) = car.single() else { return };
    let car = car.translation();
    for (e, mut prop, mut t) in &mut props {
        if prop.pinned {
            continue;
        }
        let far = |p: Vec3| p.distance(car) > prop.physics.reset_distance;
        if !far(t.translation) || !far(prop.home.translation) {
            continue;
        }
        stats.reset += 1;
        if prop.physics.remove_instead_of_reset {
            commands.entity(e).despawn();
            continue;
        }
        *t = prop.home;
        prop.pinned = true;
        commands.entity(e).insert((
            RigidBody::Static,
            Position(prop.home.translation),
            Rotation(prop.home.rotation),
            LinearVelocity::ZERO,
            AngularVelocity::ZERO,
        ));
    }
}

/// Per-triangle collision tags of one world collider (`Msg::Collider`), in the trimesh's triangle order.
#[derive(Component)]
pub struct SurfaceTags(pub Arc<Vec<u32>>);

/// The game's surface list (`mw_assets::surfaces`), for looking up what a world collider is made of.
/// Systems that care about surfaces (tyre grip, skid audio, dust particles) call
/// [`WorldSurfaces::surface_at`] with a ray that hit a world collider.
#[derive(Resource, Default)]
pub struct WorldSurfaces {
    pub list: Vec<SurfaceInfo>,
}

/// What a ray hit: the decoded collision tag and the surface entry it names.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceHit {
    pub tag: CollisionTag,
    pub surface: Option<SurfaceInfo>,
}

impl WorldSurfaces {
    /// The collision tag of the world triangle a ray hits on `collider` (one with [`SurfaceTags`]).
    /// Re-casts the ray against that collider's triangle mesh to find the triangle.
    pub fn surface_at(
        &self,
        collider: &Collider,
        tags: &SurfaceTags,
        origin: Vec3,
        dir: Vec3,
        max_distance: f32,
    ) -> Option<SurfaceHit> {
        use avian3d::parry::query::{Ray, RayCast};
        use avian3d::parry::shape::FeatureId;
        let mesh = collider.shape_scaled().as_trimesh()?;
        let ray = Ray::new(origin.to_array().into(), dir.to_array().into());
        let hit = mesh.cast_local_ray_and_get_normal(&ray, max_distance, true)?;
        let FeatureId::Face(i) = hit.feature else { return None };
        // Back faces are numbered past the triangle count.
        let i = i as usize % mesh.indices().len();
        let tag = CollisionTag::from_u32(*tags.0.get(i)?);
        Some(SurfaceHit { tag, surface: self.list.get(tag.surface as usize).cloned() })
    }
}

// ---------------------------------------------------------------------------------------------
// `--test smash`: drive through real props on the test track at a held speed.

/// The smash test's layout along the car's path (+Z from the origin): a traffic cone (5 kg, breaks at
/// any touch), a street lamp (150 kg compound, break force 5000), a 6 m fence panel (50 kg, 2500), and
/// an immovable 5 m wall (a mass-0 dynamic object with break force 100000).
pub fn smash_test_props() -> Vec<crate::loader::TestProp> {
    use crate::loader::TestProp;
    let p =
        |id: u64, kind, z: f32| TestProp { object: ResourceId(id), kind, position: Vec3::new(0.0, 0.0, z), yaw: 0.0 };
    vec![
        p(0x3698c67a, ObjectKind::Prop, 40.0),
        p(0xfed64b97, ObjectKind::Compound, 80.0),
        p(0x406f7f52, ObjectKind::Prop, 120.0),
        p(0xd22c0396, ObjectKind::Dynamic, 170.0),
    ]
}

#[derive(Resource)]
pub struct SmashTest {
    out: Option<std::path::PathBuf>,
    speed: f32,
    phase: SmashPhase,
    timer: f32,
    stalled: f32,
    top_speed: f32,
    knocked: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SmashPhase {
    Settle,
    Drive,
    Done,
}

impl SmashTest {
    pub fn new(out: Option<std::path::PathBuf>, speed: f32) -> Self {
        Self { out, speed, phase: SmashPhase::Settle, timer: 0.0, stalled: 0.0, top_speed: 0.0, knocked: 0 }
    }

    fn path(&self, suffix: &str) -> Option<std::path::PathBuf> {
        let out = self.out.as_ref()?;
        let stem = out.file_stem()?.to_string_lossy();
        let ext = out.extension().map_or("png".into(), |e| e.to_string_lossy());
        Some(out.with_file_name(format!("{stem}-{suffix}.{ext}")))
    }
}

/// Runs `--test smash` (added when the [`SmashTest`] resource is present).
pub struct SmashTestPlugin;

impl Plugin for SmashTestPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, run_smash_test.before(crate::drive::car_physics))
            .add_systems(PostUpdate, smash_camera.before(TransformSystems::Propagate));
    }
}

#[allow(clippy::too_many_arguments)]
fn run_smash_test(
    mut commands: Commands,
    time: Res<Time>,
    mut test: ResMut<SmashTest>,
    mut input: ResMut<crate::drive::CarInput>,
    cars: Query<(&Car, &Transform)>,
    props: Query<(&PropBody, &Transform), Without<Car>>,
    stats: Res<PropStats>,
    mut exit: MessageWriter<AppExit>,
) {
    use bevy::render::view::screenshot::{Screenshot, save_to_disk};
    let Ok((car, transform)) = cars.single() else { return };
    let dt = time.delta_secs();
    test.timer += dt;
    let speed = car.speed.abs();
    test.top_speed = test.top_speed.max(speed);
    match test.phase {
        SmashPhase::Settle => {
            input.scripted = Some([0.0, 0.0, 0.0, 1.0]);
            if test.timer > 2.5 && props.iter().count() >= 4 {
                if let Some(p) = test.path("before") {
                    commands.spawn(Screenshot::primary_window()).observe(save_to_disk(p));
                }
                test.phase = SmashPhase::Drive;
                test.timer = 0.0;
            }
        }
        SmashPhase::Drive => {
            // Hold the target speed: throttle on a simple proportional law, brake when well over.
            let err = test.speed - speed;
            let throttle = (0.15 + err * 0.4).clamp(0.0, 1.0);
            let brake = if err < -1.5 { 0.3 } else { 0.0 };
            input.scripted = Some([throttle, brake, 0.0, 0.0]);
            if stats.knocked > test.knocked {
                info!("TEST smash: prop knocked at car z {:.1}, car {:.0} km/h", transform.translation.z, speed * 3.6);
                test.knocked = stats.knocked;
            }
            test.stalled = if speed < 0.5 && test.timer > 3.0 { test.stalled + dt } else { 0.0 };
            if test.stalled > 2.0 || transform.translation.z > 220.0 || test.timer > 60.0 {
                info!(
                    "TEST RESULT smash: target {:.0} km/h, top {:.0} km/h, car at z {:.1} ({}), {} props knocked",
                    test.speed * 3.6,
                    test.top_speed * 3.6,
                    transform.translation.z,
                    if test.stalled > 2.0 { "stopped" } else { "drove through" },
                    stats.knocked
                );
                for (p, t) in &props {
                    info!(
                        "TEST RESULT smash: {:?} {} ({:.0} kg, break {}): {} at ({:.1} {:.1} {:.1}), placed at z {:.0}",
                        p.physics.kind,
                        p.physics.object,
                        p.physics.mass,
                        p.physics.break_force,
                        if p.pinned { "standing" } else { "knocked" },
                        t.translation.x,
                        t.translation.y,
                        t.translation.z,
                        p.home.translation.z
                    );
                }
                if let Some(p) = test.path("after") {
                    commands.spawn(Screenshot::primary_window()).observe(save_to_disk(p));
                }
                test.phase = SmashPhase::Done;
                test.timer = 0.0;
            }
        }
        SmashPhase::Done => {
            input.scripted = Some([0.0, 1.0, 0.0, 0.0]);
            if test.timer > 1.5 {
                exit.write(AppExit::Success);
            }
        }
    }
}

/// A fixed side view over the props instead of the chase camera.
fn smash_camera(mut camera: Query<&mut Transform, crate::MainCamera>) {
    let Ok(mut cam) = camera.single_mut() else { return };
    *cam = Transform::from_xyz(-16.0, 6.0, 60.0).looking_at(Vec3::new(0.0, 1.5, 118.0), Vec3::Y);
}

/// The world surface under the car (updated a few times a second), for audio, particles and grip.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct CarSurface(pub SurfaceHit);

fn track_car_surface(
    mut commands: Commands,
    time: Res<Time>,
    mut timer: Local<f32>,
    surfaces: Res<WorldSurfaces>,
    spatial: SpatialQuery,
    cars: Query<(Entity, &GlobalTransform, Option<&CarSurface>), With<Car>>,
    world: Query<(&Collider, &SurfaceTags)>,
) {
    *timer += time.delta_secs();
    if *timer < 0.25 {
        return;
    }
    *timer = 0.0;
    let filter = SpatialQueryFilter::default().with_mask(GameLayer::World);
    for (car, t, old) in &cars {
        let origin = t.translation() + Vec3::Y * 1.0;
        let Some(hit) = spatial.cast_ray(origin, Dir3::NEG_Y, 4.0, true, &filter) else { continue };
        let Ok((collider, tags)) = world.get(hit.entity) else { continue };
        let Some(s) = surfaces.surface_at(collider, tags, origin, Vec3::NEG_Y, 4.0) else { continue };
        if old.is_none_or(|o| o.0.tag.surface != s.tag.surface || o.0.tag.flags != s.tag.flags) {
            info!(
                "car on surface {} (flags {:#05x}, grip {:?}, unit {})",
                s.tag.surface,
                s.tag.flags,
                s.surface.as_ref().and_then(|i| i.grip),
                s.tag.unit
            );
            commands.entity(car).insert(CarSurface(s));
        }
    }
}

/// `--camera` together with `--drive`: watch from a fixed pose instead of the chase camera (to film a
/// car knocking props over).
#[derive(Resource)]
pub struct FixedCamera(pub Transform);

pub fn apply_fixed_camera(fixed: Res<FixedCamera>, mut camera: Query<&mut Transform, crate::MainCamera>) {
    if let Ok(mut cam) = camera.single_mut() {
        *cam = fixed.0;
    }
}
