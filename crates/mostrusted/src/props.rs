//! Collision for props, compound objects and dynamic objects, and collision surfaces of the world.
//!
//! Every placed object with a GeneSys `PhysicalDefinition` (`mw_assets::props`) becomes a rigid body
//! with the definition's collision volumes (boxes and cylinders; built on the loader thread), mass,
//! centre of mass, friction and restitution. It starts out pinned (`RigidBody::Static`), like the
//! game's props standing in the world. A hit breaks it free when the impact exceeds the object's
//! break threshold (`AdditionalInfo` field `67f0233c`, unnamed: 0 for cones and bins, 500–2500 for signs
//! and fences, 5000 for street lamps, 20000 for gantries, 100000 = never; see [`PropPhysics::break_force`]);
//! then it becomes a dynamic body that falls and tumbles. Objects with mass 0 never break free; objects without
//! an `AdditionalInfo` (trash bags, crates) are never pinned and move at the first touch. Knocked
//! objects are put back (or removed, `mbRemoveInsteadOfReset`) once the car is farther away than the
//! prop's reset distance (`mfResetDistanceSquared`, 100 m on every prop; compounds and dynamic objects use
//! 100 m too).
//!
//! **A knocked prop barely touches the car** (docs/decomp.md, "Props"). The game solves a race car's
//! contact with a dynamic object like any contact, but adds only a share of the impulse to each body
//! (`CollisionResponses_RaceCar.RaceCarVsDynamic`, `FUN_00941170` → `FUN_009bb640` → the contact passes
//! `FUN_009be420` / `FUN_009bed70`): the player's car takes 0.1 of the linear and 0.05 of the angular
//! impulse, the object all of its linear and 0.4 of its angular impulse (AI racers 0.01 / 0.01). Avian
//! cannot scale one pair's impulses, so knocked props ([`GameLayer::LooseProp`]) do not meet the race
//! cars in avian at all; [`prop_contacts`] finds those contacts and solves them with the game's scales
//! before each physics step. Standing (pinned) props stay in avian's solver: one that holds stops the car
//! like a wall.
//!
//! Compound objects (lamp posts, traffic lights, billboards) break free as one body (the root definition's
//! volumes and mass) and come apart into their parts when they smash ([`smash_compounds`]): each part is a
//! body of its own with its own physical definition, drawn with the triangles of the model skinned to its
//! skeleton bone (split on the loader thread).
//!
//! Only props near a car carry a collider ([`stream_props`]): the game keeps at most 80 physics objects
//! (cars and props, world +0x51500), so it cannot hold the map's 18,000 props as bodies either.
//!
//! Collision layers: the world (`PolygonSoupList` and anything without layers) is [`GameLayer::World`];
//! the car's body is [`GameLayer::Car`] and a low bumper box that only meets props is
//! [`GameLayer::CarBumper`] (the body box sits above the wheels, so low props would slip under it);
//! standing props are [`GameLayer::Prop`] and collide with the world, the cars and each other; knocked
//! ones [`GameLayer::LooseProp`] (everything but the race cars). Traffic cars are [`GameLayer::Traffic`]
//! (`traffic.rs`): lane followers meet the car, props and traffic; crashed ones the world too. The wheels'
//! ray casts only see the world.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use avian3d::collision::collider::contact_query;
use avian3d::prelude::*;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use mw_assets::props::{DynamicResponse, DynamicResponses, ObjectKind};
use mw_assets::surfaces::SurfaceInfo;
use mw_bundle::ResourceId;
use mw_formats::CollisionTag;

use crate::drive::Car;

/// What can knock a prop or a traffic car loose: a race car or crashed traffic.
type Hitter = Or<(With<Car>, With<crate::traffic::TrafficCar>)>;

/// Collision layers. The default (first) layer is the world, so colliders spawned without layers are
/// world geometry.
#[derive(PhysicsLayer, Default, Clone, Copy, Debug)]
pub enum GameLayer {
    #[default]
    World,
    Car,
    /// A low box on the car that only collides with props.
    CarBumper,
    /// Standing (pinned) props.
    Prop,
    /// Traffic cars (`traffic.rs`).
    Traffic,
    /// Knocked props: the race cars' contacts with them are solved by [`prop_contacts`], not by avian.
    LooseProp,
}

impl GameLayer {
    pub fn car() -> CollisionLayers {
        // Cars collide with each other too (race cars, `ai.rs`).
        CollisionLayers::new(GameLayer::Car, [GameLayer::World, GameLayer::Car, GameLayer::Prop, GameLayer::Traffic])
    }
    pub fn car_bumper() -> CollisionLayers {
        CollisionLayers::new(GameLayer::CarBumper, [GameLayer::Prop, GameLayer::Traffic])
    }
    pub fn prop() -> CollisionLayers {
        CollisionLayers::new(
            GameLayer::Prop,
            [
                GameLayer::World,
                GameLayer::Car,
                GameLayer::CarBumper,
                GameLayer::Prop,
                GameLayer::Traffic,
                GameLayer::LooseProp,
            ],
        )
    }
    pub fn loose_prop() -> CollisionLayers {
        CollisionLayers::new(
            GameLayer::LooseProp,
            [GameLayer::World, GameLayer::Prop, GameLayer::Traffic, GameLayer::LooseProp],
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
    /// Radius of a sphere about the object's origin that holds the collider (m).
    pub radius: f32,
    /// kg; 0 = immovable.
    pub mass: f32,
    /// Density that gives the collider `mass` (avian derives the inertia from it).
    pub density: f32,
    pub centre_of_mass: Vec3,
    pub friction: f32,
    pub restitution: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    /// The momentum (kg m/s) a hitter needs to knock the object loose (`f32::INFINITY` = never), compared
    /// with [`hit_momentum`].
    pub break_force: f32,
    /// Contact force (N) that starts the impact effect (`ForceToDamage`).
    pub force_to_damage: f32,
    /// Compound objects: `ForceToSmash` (N), the contact force that breaks a knocked compound into its parts
    /// once it has been loose for `smash_delay` seconds (`08c87eeb`; `FUN_00555c90`). Compound parts: their
    /// `SmashProxyBehaviour.ForceToSmash`, which replaces the part by its replacement chain.
    pub smash_force: Option<f32>,
    pub smash_delay: f32,
    pub reset_distance: f32,
    pub remove_instead_of_reset: bool,
    /// Props: the impact effect (`BearEffect`) and sound (`WCPlaySoundBehaviour`); compound parts: the part's
    /// sound behaviour.
    pub impact_effect: Option<ResourceId>,
    pub impact_sound: Option<ResourceId>,
    /// Compound objects: the parts that replace it when it smashes.
    pub compound: Option<CompoundBreak>,
}

/// The parts of a compound object (`mw_assets::props::CompoundPhysics`), ready to spawn.
#[derive(Debug, Clone)]
pub struct CompoundBreak {
    pub parts: Vec<PartBreak>,
}

/// One part of a compound object.
#[derive(Debug, Clone)]
pub struct PartBreak {
    /// The part's own body; `None` without collision volumes (it is not spawned).
    pub physics: Option<Arc<PropPhysics>>,
    /// The part's frame in the compound's space: its skeleton bone's bind pose (`FUN_00532b00`).
    pub bone: Mat4,
    pub bone_index: u32,
    /// The part this one replaces when that part smashes; `None` = one of the compound's first parts.
    pub replaces: Option<usize>,
    pub next: Option<usize>,
    pub first_replacement: Option<usize>,
    /// Parts this one is linked to: while pinned, it comes loose once none of them is still pinned
    /// (`FUN_0054e750`).
    pub links: Vec<usize>,
}

impl PropPhysics {
    /// Build from decoded physics; `None` if the object has no collision volumes.
    pub fn new(p: &mw_assets::props::ObjectPhysics) -> Option<Self> {
        let mut out = Self::from_definition(p.object, p.kind, p.definition.as_ref()?)?;
        out.reset_distance = p.reset_distance.unwrap_or(100.0);
        out.remove_instead_of_reset = p.remove_instead_of_reset;
        out.impact_effect = p.impact_effect;
        out.impact_sound = p.impact_sound;
        let reset = out.reset_distance;
        out.compound = p.compound.as_ref().map(|c| CompoundBreak {
            parts: c
                .parts
                .iter()
                .map(|part| PartBreak {
                    physics: part.definition.as_ref().and_then(|d| {
                        let mut ph = Self::from_definition(p.object, p.kind, d)?;
                        ph.reset_distance = reset;
                        ph.smash_force = part.smash_force;
                        ph.smash_delay = 0.0;
                        ph.impact_sound = part.impact_sound;
                        Some(Arc::new(ph))
                    }),
                    bone: crate::loader::game_matrix(&part.bone),
                    bone_index: part.bone_index,
                    replaces: part.replaces,
                    next: part.next,
                    first_replacement: part.first_replacement,
                    links: part.links.clone(),
                })
                .collect(),
        });
        Some(out)
    }

    /// The body of one physical definition (an object's, or a compound part's).
    fn from_definition(
        object: ResourceId,
        kind: ObjectKind,
        def: &mw_assets::props::PhysicalDefinition,
    ) -> Option<Self> {
        use mw_assets::props::Volume;
        let mut shapes: Vec<(Vec3, Quat, Collider)> = Vec::new();
        let mut volume = 0.0;
        let mut com = Vec3::ZERO;
        let mut mass = 0.0;
        let mut radius: f32 = 0.0;
        let (mut friction, mut restitution, mut lin, mut ang) = (0.0, 0.0, 0.0, 0.0);
        for body in &def.bodies {
            let body_m = crate::loader::game_matrix(&body.transform);
            for v in &body.volumes {
                // Cylinders and capsules: the game's axis is local Z, parry's is Y (no nested compounds,
                // parry rejects them, so the axis turn goes into the volume's own rotation).
                let to_z = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
                let (m, shape, vol, turn, reach) = match v {
                    Volume::Box { transform, half_extents: h } => {
                        let [x, y, z] = h.map(|c| c.max(0.01));
                        let reach = Vec3::new(x, y, z).length();
                        (transform, Collider::cuboid(2.0 * x, 2.0 * y, 2.0 * z), 8.0 * x * y * z, Quat::IDENTITY, reach)
                    }
                    Volume::Sphere { transform, radius } => {
                        (transform, Collider::sphere(*radius), 4.19 * radius.powi(3), Quat::IDENTITY, *radius)
                    }
                    Volume::Cylinder { transform, radius, half_length } => (
                        transform,
                        Collider::cylinder(*radius, 2.0 * half_length),
                        std::f32::consts::PI * radius * radius * 2.0 * half_length,
                        to_z,
                        radius.hypot(*half_length),
                    ),
                    Volume::Capsule { transform, radius, half_length } => (
                        transform,
                        Collider::capsule(*radius, 2.0 * half_length),
                        std::f32::consts::PI * radius * radius * 2.0 * half_length,
                        to_z,
                        radius + half_length,
                    ),
                    // Convex hull resources are not decoded (none are used by world objects).
                    Volume::Hull { .. } => continue,
                };
                let (_, rot, pos) = (body_m * crate::loader::game_matrix(m)).to_scale_rotation_translation();
                let rot = rot.normalize() * turn;
                radius = radius.max(pos.length() + reach);
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
        // Objects that do not start pinned (cones, bins: threshold 0) are the game's asleep dynamic bodies; ours
        // stand static until the first touch by a moving body, which is the same (model decision). Pinned ones
        // come loose only when they are knockable (`FUN_0054e570`). An object without an `AdditionalInfo` (the
        // trash bag 03102fec, 353 placements; crates and others: 19 prop and 3 dynamic definitions) is never
        // pinned: `FUN_0054dfe0` clears the pinned bit when the definition's info pointer (+0x30) is null, and
        // the break test (`FUN_0054e570`) and the damage threshold (`FUN_0054bf70`) skip their info reads then.
        let break_force = if mass <= 0.0 {
            f32::INFINITY
        } else if def.info.is_none() || !info.starts_pinned {
            0.0
        } else if !info.knockable {
            f32::INFINITY
        } else {
            info.force_67f0233c
        };
        Some(Self {
            object,
            kind,
            collider: Collider::compound(shapes),
            radius,
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
            force_to_damage: info.force_to_damage,
            smash_force: info.force_to_smash,
            smash_delay: info.smash_delay.unwrap_or(0.0),
            reset_distance: 100.0,
            remove_instead_of_reset: false,
            impact_effect: None,
            impact_sound: None,
            compound: None,
        })
    }
}

/// What a hit is measured by against a standing prop's threshold: the hitter's momentum `|m v|` with its whole
/// velocity, not the closing part (`FUN_0054e570`: the body's mass +0x70.w times its velocity +0xb0, squared
/// and compared with the threshold squared). A 1.5 t car knocks a street lamp (5000) loose above 3.3 m/s.
pub fn hit_momentum(hitter_mass: f32, velocity: Vec3) -> f32 {
    hitter_mass * velocity.length()
}

/// A placed object with physics.
#[derive(Component)]
pub struct PropBody {
    pub physics: Arc<PropPhysics>,
    /// Where it stands in the world (restored on reset).
    pub home: Transform,
    /// Pinned in place (static) until hit hard enough.
    pub pinned: bool,
    /// Carries a rigid body and collider (near a car; [`stream_props`]).
    pub active: bool,
    /// Game time of the last impact sound.
    last_impact: f32,
    /// The impact effect has played since the prop was last put back (the game's damage state +0xb4).
    damaged: bool,
    /// Speed (m/s) the player's car lost to this prop's contacts (shared by normal impulse; `--test smash`).
    pub speed_loss: f32,
    /// Seconds since it was knocked loose (the game counts its compound timer down from `08c87eeb`).
    loose_for: f32,
    /// The largest contact force (N, normal impulse / step) of a race car's contact this step ([`prop_contacts`]).
    car_force: f32,
}

impl PropBody {
    fn new(physics: Arc<PropPhysics>, home: Transform, pinned: bool) -> Self {
        Self {
            physics,
            home,
            pinned,
            active: false,
            last_impact: f32::NEG_INFINITY,
            damaged: false,
            speed_loss: 0.0,
            loose_for: 0.0,
            car_force: 0.0,
        }
    }
}

/// A compound object's model split by part (its skeleton bones, built on the loader thread).
#[derive(Clone, Default)]
pub struct CompoundMeshes {
    /// One per part (`PartBreak`); `None` where the model has no triangles on the part's bone.
    pub parts: Vec<Option<crate::ModelParts>>,
    /// The triangles of bones no part owns: drawn with the standing compound and gone with it.
    pub rest: Option<crate::ModelParts>,
}

/// A compound object and its break-up (see [`smash_compounds`]).
#[derive(Component)]
pub struct Compound {
    meshes: CompoundMeshes,
    range: Option<crate::shadow_cull::ShadowRange>,
    /// The part bodies after the smash, by part index.
    parts: Vec<Option<Entity>>,
    smashed: bool,
}

impl Compound {
    /// Broken into its parts (until it is put back).
    pub fn is_smashed(&self) -> bool {
        self.smashed
    }
}

/// A part of a smashed compound: a prop of its own until the compound is put back.
#[derive(Component)]
pub struct CompoundPiece {
    pub compound: Entity,
    pub part: usize,
}

/// Asks [`unpin_props`] to break a prop free after this physics step (hits by traffic or knocked props; race
/// cars' hits are handled in [`prop_contacts`]).
#[derive(Component)]
struct Unpin {
    /// The hitter's closing speed (m/s) and the contact point.
    closing: f32,
    point: Vec3,
}

/// A car hit a prop hard enough to show: the prop's impact effect and sound (`particles.rs`,
/// [`prop_sounds`]).
#[derive(Message, Clone, Copy, Debug)]
pub struct PropImpact {
    pub position: Vec3,
    /// Out of the prop, towards the car.
    pub normal: Vec3,
    /// The car's velocity.
    pub velocity: Vec3,
    pub closing_speed: f32,
    pub effect: Option<ResourceId>,
    pub sound: Option<ResourceId>,
    pub by_player: bool,
}

/// A standing prop (or a part of a split compound) was knocked loose. `freeroam.rs` counts billboards with it
/// (the game's smashable object posts game-logic message 0x14 with "by the local player's vehicle",
/// `FUN_005b5340`).
#[derive(Message, Clone, Copy, Debug)]
pub struct PropKnocked {
    pub prop: Entity,
    pub by_player: bool,
}

/// Counts for logs and tests.
#[derive(Resource, Default, Debug)]
pub struct PropStats {
    /// Props with a collider right now.
    pub active: usize,
    pub knocked: usize,
    pub reset: usize,
    /// Contacts solved by [`prop_contacts`] since the start, and the largest share of a car's speed one
    /// physics step of them took away.
    pub contacts: usize,
    pub max_speed_loss: f32,
    /// The speed (m/s) the player's car has lost to knocked props in total (`--test smash`).
    pub player_speed_loss: f32,
    /// Impact effects started (a contact passed `ForceToDamage`).
    pub effects: usize,
    /// Compounds smashed into their parts, parts smashed by their `SmashProxyBehaviour`, and the part bodies
    /// spawned by both.
    pub smashed: usize,
    pub splits: usize,
    pub pieces: usize,
    /// Where the last compound or part smashed.
    pub last_smash: Vec3,
}

pub struct PropsPlugin;

impl Plugin for PropsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PropStats>()
            .init_resource::<WorldSurfaces>()
            .init_resource::<PropGrid>()
            .init_resource::<PropSounds>()
            .add_message::<PropImpact>()
            .add_message::<PropKnocked>()
            .add_systems(FixedPostUpdate, prop_contacts.before(PhysicsSystems::StepSimulation))
            .add_systems(
                FixedPostUpdate,
                (unpin_props, smash_compounds, prop_world_sounds).chain().after(PhysicsSystems::StepSimulation),
            )
            .add_systems(Update, (index_props, reset_far_props, stream_props, track_car_surface, prop_sounds).chain());
    }
}

/// The meshes of one model under the game's LOD rule: (parts, the LODs they are drawn at, shadow range) per
/// set, and the LOD table with the placement's scale.
pub struct MeshSets {
    pub sets: Vec<(crate::ModelParts, u8, Option<crate::shadow_cull::ShadowRange>)>,
    pub lod: Option<(Arc<crate::lod::LodTable>, f32)>,
}

/// A billboard compound's two standing models (`loader::Loader::compound_standing`): the advert
/// (`CompoundObject` +0x24) and the "WANTED" poster with the picture it shows once found (+0x28), each under
/// a group entity of its own. `freeroam.rs` shows one of them.
#[derive(Component)]
pub struct BillboardFaces {
    pub advert: Entity,
    pub found: Entity,
    /// Which one is shown (`None` until the found list is known).
    pub shown: Option<bool>,
}

/// Spawn `sets` under `c`; returns the meshes with their LOD masks when they are LOD-managed.
fn spawn_sets(
    c: &mut bevy::ecs::hierarchy::ChildSpawnerCommands,
    sets: &[(crate::ModelParts, u8, Option<crate::shadow_cull::ShadowRange>)],
    managed: bool,
) -> Vec<(Entity, u8)> {
    let mut lod_meshes = Vec::new();
    for (parts, mask, range) in sets {
        for id in spawn_meshes(c, parts, Transform::IDENTITY, *range) {
            if managed {
                let state = (crate::lod::LodShadow(true), crate::lod::initial_visibility(*mask));
                c.commands().entity(id).insert(state);
                lod_meshes.push((id, *mask));
            }
        }
    }
    lod_meshes
}

/// Spawn a prop: the render parts as children of a rigid body carrying the collider. A compound object draws
/// its standing model (`standing`: `sets` are of `CompoundObject` +0x24, not of the pieces' model) until it
/// breaks; without one, the pieces of its first parts stand instead of the whole model (the model also holds
/// the parts that replace those later, on top of them). `found`: a billboard's second standing model
/// ([`BillboardFaces`]).
#[allow(clippy::too_many_arguments)]
pub fn spawn_prop(
    commands: &mut Commands,
    physics: Arc<PropPhysics>,
    transform: Transform,
    sets: &[(crate::ModelParts, u8, Option<crate::shadow_cull::ShadowRange>)],
    range: Option<crate::shadow_cull::ShadowRange>,
    pieces: Option<CompoundMeshes>,
    lod: Option<(Arc<crate::lod::LodTable>, f32)>,
    standing: bool,
    found: Option<MeshSets>,
) {
    let pieces = pieces.filter(|p| physics.compound.as_ref().is_some_and(|c| c.parts.len() == p.parts.len()));
    let mut e = commands.spawn((transform, Visibility::default(), PropBody::new(physics.clone(), transform, true)));
    // The meshes under the game's LOD rule (`lod.rs`) and the LODs each is drawn at.
    let mut lod_meshes = Vec::new();
    let managed = lod.is_some();
    match pieces {
        Some(pieces) if standing => {
            let n = pieces.parts.len();
            match found {
                Some(found) => {
                    // Each face in a group of its own, with its own LOD table.
                    let mut groups = [Entity::PLACEHOLDER; 2];
                    e.with_children(|c| {
                        for (i, (sets, lod)) in
                            [(sets, lod.clone()), (&found.sets[..], found.lod)].into_iter().enumerate()
                        {
                            let vis = if i == 0 { Visibility::Inherited } else { Visibility::Hidden };
                            let mut g = c.spawn((Transform::IDENTITY, vis));
                            let mut meshes = Vec::new();
                            g.with_children(|c| meshes = spawn_sets(c, sets, lod.is_some()));
                            if let Some((table, scale)) = lod {
                                g.insert(crate::lod::ModelLod::new(table, scale, meshes));
                            }
                            groups[i] = g.id();
                        }
                    });
                    e.insert(BillboardFaces { advert: groups[0], found: groups[1], shown: None });
                    e.insert(Compound { meshes: pieces, range, parts: vec![None; n], smashed: false });
                    return;
                }
                None => e.with_children(|c| lod_meshes = spawn_sets(c, sets, managed)),
            };
            e.insert(Compound { meshes: pieces, range, parts: vec![None; n], smashed: false });
        }
        None => {
            e.with_children(|c| lod_meshes = spawn_sets(c, sets, managed));
        }
        Some(pieces) => {
            let compound = physics.compound.as_ref().expect("checked above");
            e.with_children(|c| {
                let mut ids = Vec::new();
                for (part, meshes) in compound.parts.iter().zip(&pieces.parts) {
                    if let (None, Some(meshes)) = (part.replaces, meshes) {
                        ids.extend(spawn_meshes(c, meshes, Transform::IDENTITY, range));
                    }
                }
                if let Some(rest) = &pieces.rest {
                    ids.extend(spawn_meshes(c, rest, Transform::IDENTITY, range));
                }
                // The pieces are LOD0's: drawn at every LOD, only the draw distance applies.
                if managed {
                    for id in ids {
                        c.commands().entity(id).insert(crate::lod::LodShadow(true));
                        lod_meshes.push((id, crate::lod::ALL_LODS));
                    }
                }
            });
            let n = compound.parts.len();
            e.insert(Compound { meshes: pieces, range, parts: vec![None; n], smashed: false });
        }
    }
    if let Some((table, scale)) = lod {
        e.insert(crate::lod::ModelLod::new(table, scale, lod_meshes));
    }
}

fn spawn_meshes(
    c: &mut bevy::ecs::hierarchy::ChildSpawnerCommands,
    parts: &crate::ModelParts,
    transform: Transform,
    range: Option<crate::shadow_cull::ShadowRange>,
) -> Vec<Entity> {
    let mut ids = Vec::with_capacity(parts.len());
    for (mesh, mat) in parts {
        let mut e = c.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(mat.clone()),
            transform,
            crate::shadow_cull::ShadowClass::Prop,
        ));
        if let Some(r) = range {
            e.insert(r);
        }
        ids.push(e.id());
    }
    ids
}

/// The rigid body and collider of a prop, inserted while it is near a car.
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

/// Props get their collider when a car comes within this distance of their bounding sphere (m)...
const ACTIVATE_DISTANCE: f32 = 30.0;
/// ...and lose it again beyond this one (while standing). Model decision: the game's own activation rule was
/// not found; 30 m covers 0.1 s (the streaming period) at over 1000 km/h.
const DEACTIVATE_DISTANCE: f32 = 45.0;
/// Side of the prop grid's cells (m).
const GRID_CELL: f32 = 32.0;
/// The largest prop radius the grid search allows for (m; the widest gantries reach about 17 m).
const MAX_PROP_RADIUS: f32 = 20.0;

/// The props by where they stand, for [`stream_props`].
#[derive(Resource, Default)]
struct PropGrid {
    cells: HashMap<(i32, i32), Vec<Entity>>,
    /// Props carrying a collider.
    active: HashSet<Entity>,
}

fn grid_cell(p: Vec3) -> (i32, i32) {
    ((p.x / GRID_CELL).floor() as i32, (p.z / GRID_CELL).floor() as i32)
}

/// Add new props to the grid.
#[allow(clippy::type_complexity)]
fn index_props(
    mut grid: ResMut<PropGrid>,
    props: Query<(Entity, &PropBody), (Added<PropBody>, Without<CompoundPiece>)>,
) {
    for (e, p) in &props {
        grid.cells.entry(grid_cell(p.home.translation)).or_default().push(e);
    }
}

/// Stream prop physics with the cars: tens of thousands of static bodies cost avian several ms a frame
/// even when nothing touches them, so only props near a race car or a traffic car carry a rigid body.
#[allow(clippy::too_many_arguments)]
fn stream_props(
    mut commands: Commands,
    time: Res<Time>,
    mut timer: Local<f32>,
    mut grid: ResMut<PropGrid>,
    cars: Query<(&GlobalTransform, &RigidBody), Hitter>,
    mut props: Query<(&mut PropBody, &Transform)>,
    mut stats: ResMut<PropStats>,
    mut effects: Option<ResMut<crate::particles::EffectRequests>>,
) {
    *timer += time.delta_secs();
    if *timer < 0.1 {
        return;
    }
    *timer = 0.0;
    // Race cars and crashed (dynamic) traffic; lane followers meet standing props as solid obstacles and never
    // knock them loose, so they need no colliders around them.
    let cars: Vec<Vec3> = cars.iter().filter(|(_, rb)| rb.is_dynamic()).map(|(t, _)| t.translation()).collect();
    // Activate the standing props near each car.
    let reach = ACTIVATE_DISTANCE + MAX_PROP_RADIUS;
    let span = (reach / GRID_CELL).ceil() as i32;
    let mut wanted = Vec::new();
    for c in &cars {
        let (cx, cz) = grid_cell(*c);
        for x in cx - span..=cx + span {
            for z in cz - span..=cz + span {
                let Some(list) = grid.cells.get(&(x, z)) else { continue };
                for &e in list {
                    let Ok((prop, t)) = props.get(e) else { continue };
                    if !prop.active && c.distance(t.translation) < ACTIVATE_DISTANCE + prop.physics.radius {
                        wanted.push(e);
                    }
                }
            }
        }
    }
    for e in wanted {
        let Ok((mut prop, _)) = props.get_mut(e) else { continue };
        if prop.active {
            continue;
        }
        prop.active = true;
        // Load the impact effect ahead of a hit.
        if let (Some(fx), Some(id)) = (effects.as_mut(), prop.physics.impact_effect) {
            fx.request(id);
        }
        commands.entity(e).insert(physics_bundle(&prop.physics));
        grid.active.insert(e);
    }
    // Drop the standing ones no car is near any more.
    let mut gone = Vec::new();
    for &e in &grid.active {
        let Ok((mut prop, t)) = props.get_mut(e) else {
            gone.push(e);
            continue;
        };
        if !prop.pinned {
            continue;
        }
        let far = DEACTIVATE_DISTANCE + prop.physics.radius;
        if cars.iter().all(|c| c.distance(t.translation) > far) {
            prop.active = false;
            gone.push(e);
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
    }
    for e in gone {
        grid.active.remove(&e);
    }
    stats.active = grid.active.len();
}

/// Contact hook: a dynamic body hitting a standing prop breaks it free when [`break_impulse`] reaches the
/// prop's threshold. The contact is dropped for that step: the prop gives way, and from the next step on
/// a race car's contact with it is solved by [`prop_contacts`], traffic's by avian.
#[derive(SystemParam)]
pub struct PropHooks<'w, 's> {
    props: Query<'w, 's, &'static PropBody>,
    bodies: Query<'w, 's, (&'static RigidBody, &'static ComputedMass, &'static LinearVelocity)>,
    /// Lane-following traffic colliders, and the bodies that knock them off their lane (`traffic.rs`).
    traffic: Query<'w, 's, (), With<crate::traffic::TrafficBody>>,
    hitters: Query<'w, 's, (), Hitter>,
    /// Race cars: their hits on standing props are tested in [`prop_contacts`] before the step.
    race_cars: Query<'w, 's, (), With<Car>>,
}

impl CollisionHooks for PropHooks<'_, '_> {
    fn modify_contacts(&self, contacts: &mut ContactPair, commands: &mut Commands) -> bool {
        // A car (the player's, or crashed traffic) touching a lane follower turns it into a crashing car; the
        // contact is dropped for this step and solved between two dynamic bodies from the next one.
        for (first, collider, other, own) in [
            (true, contacts.collider1, contacts.body2, contacts.body1),
            (false, contacts.collider2, contacts.body1, contacts.body2),
        ] {
            if self.traffic.get(collider).is_ok() {
                let Some(other) = other else { return true };
                let Ok((rb, _, v_hitter)) = self.bodies.get(other) else { return true };
                if rb.is_dynamic() && self.hitters.get(other).is_ok() {
                    // The impact as the car-against-car handlers see it (`car_impacts.rs`): the contact point,
                    // the normal from the hitter to the traffic car and both velocities before the solve.
                    let (point, normal) = contacts
                        .manifolds
                        .first()
                        .map(|m| {
                            let n = if first { -m.normal } else { m.normal };
                            (m.points.first().map_or(Vec3::NAN, |p| p.point), n)
                        })
                        .unwrap_or((Vec3::NAN, Vec3::ZERO));
                    let v_traffic = own.and_then(|b| self.bodies.get(b).ok()).map_or(Vec3::ZERO, |(_, _, v)| v.0);
                    commands.entity(collider).insert(crate::traffic::TrafficHit {
                        hitter: other,
                        point,
                        normal,
                        v_hitter: v_hitter.0,
                        v_traffic,
                    });
                    return false;
                }
                return true;
            }
        }
        let (prop_entity, prop, other) = if let Ok(p) = self.props.get(contacts.collider1) {
            (contacts.collider1, p, contacts.body2)
        } else if let Ok(p) = self.props.get(contacts.collider2) {
            (contacts.collider2, p, contacts.body1)
        } else {
            return true;
        };
        if !prop.pinned {
            return true;
        }
        let Some(other) = other else { return true };
        let Ok((rb, mass, velocity)) = self.bodies.get(other) else { return true };
        // Cars, crashed traffic and knocked props knock standing props loose (`FUN_0054ec10` for a prop
        // against another body, `FUN_0054ed10` for two props).
        let loose_prop = self.props.get(other).is_ok_and(|p| !p.pinned);
        if !rb.is_dynamic() || self.race_cars.get(other).is_ok() || (self.hitters.get(other).is_err() && !loose_prop) {
            return true;
        }
        // Approach speed along the normal (normal_speed < 0 while closing), for effects and logs.
        let Some((closing, point)) = contacts
            .manifolds
            .iter()
            .flat_map(|m| m.points.iter().map(|p| (-p.normal_speed, p.point)))
            .max_by(|a, b| a.0.total_cmp(&b.0))
        else {
            return true;
        };
        // The game's test is strict (`|m v|² > T²`).
        if hit_momentum(mass.value(), velocity.0) <= prop.physics.break_force {
            return true;
        }

        commands.entity(prop_entity).insert(Unpin { closing, point });
        contacts.manifolds.clear();
        false
    }
}

fn unpin_props(
    mut commands: Commands,
    time: Res<Time>,
    mut props: Query<(Entity, &mut PropBody, &Unpin, &Transform)>,
    mut stats: ResMut<PropStats>,
    mut impacts: MessageWriter<PropImpact>,
    mut knocked: MessageWriter<PropKnocked>,
) {
    for (e, mut prop, unpin, t) in &mut props {
        commands.entity(e).remove::<Unpin>();
        if !prop.pinned {
            continue;
        }
        prop.pinned = false;
        knocked.write(PropKnocked { prop: e, by_player: false });
        prop.last_impact = time.elapsed_secs();
        stats.knocked += 1;
        let p = &prop.physics;
        info!(
            "prop {} ({:?}, {:.0} kg) knocked at ({:.1} {:.1} {:.1}), closing {:.1} m/s; smash force {:?}",
            p.object, p.kind, p.mass, t.translation.x, t.translation.y, t.translation.z, unpin.closing, p.smash_force
        );
        commands.entity(e).insert((RigidBody::Dynamic, GameLayer::loose_prop()));
        // The knock's sound; the impact effect follows from the contact force on the next step
        // ([`prop_contacts`]).
        if unpin.closing >= IMPACT_SOUND_SPEED {
            impacts.write(PropImpact {
                position: unpin.point,
                normal: (unpin.point - t.translation).normalize_or(Vec3::Y),
                velocity: Vec3::ZERO,
                closing_speed: unpin.closing,
                effect: None,
                sound: p.impact_sound,
                by_player: false,
            });
        }
    }
}

/// Contact passes per physics step: the game runs its contact solver once in each of the island's two
/// sub-steps (world +0x464 = 2, `FUN_0093f200` → `FUN_009be420`).
const CONTACT_PASSES: usize = 2;
/// Model decisions: how fast an overlap is pushed apart (fraction per step, the speed cap in m/s, the
/// overlap allowed) and the closing speed an impact effect / sound needs, with the time between two on one
/// prop. The game's contact targets (point +0x30..+0x3c) were not traced.
const PUSH_OUT: f32 = 0.2;
const PUSH_OUT_MAX: f32 = 4.0;
const PUSH_OUT_SLOP: f32 = 0.01;
/// The closing speed (m/s) a car's contact needs to play the prop's impact sound (model decision: the
/// game's threshold for world contacts is a float in an audio object, [DAT_010edb08+4] + 0xb270, not read),
/// and the quiet time before the next one (0.35 s, `.rdata` 0xfaaf2c, `FUN_0054c680`).
const IMPACT_SOUND_SPEED: f32 = 2.0;
const IMPACT_SOUND_GAP: f32 = 0.35;
/// A crashed race car's contacts with objects get the default record (`FUN_00960f20`, category 2 against
/// 4: `FUN_009251f0`, every scale 1).
const FULL_RESPONSE: DynamicResponse =
    DynamicResponse { car_linear: 1.0, car_angular: 1.0, object_linear: 1.0, object_angular: 1.0, restitution: 0.0 };

/// One body's velocity state for [`solve_contacts`].
#[derive(Clone, Copy)]
struct BodyState {
    v: Vec3,
    w: Vec3,
    inv_mass: f32,
    /// World-space inverse inertia.
    inv_inertia: Mat3,
    /// World-space centre of mass.
    com: Vec3,
}

impl BodyState {
    fn point_velocity(&self, p: Vec3) -> Vec3 {
        self.v + self.w.cross(p - self.com)
    }
    /// Inverse effective mass along `dir` at `p`.
    fn inv_effective(&self, p: Vec3, dir: Vec3) -> f32 {
        let rn = (p - self.com).cross(dir);
        self.inv_mass + rn.dot(self.inv_inertia * rn)
    }
    fn apply(&mut self, p: Vec3, impulse: Vec3, linear: f32, angular: f32) {
        self.v += linear * self.inv_mass * impulse;
        self.w += angular * (self.inv_inertia * (p - self.com).cross(impulse));
    }
}

/// A car-against-prop contact: `normal` points from the car into the prop; `depth` > 0 is an overlap,
/// < 0 a gap (speculative contact).
struct PropContact {
    prop: usize,
    point: Vec3,
    normal: Vec3,
    depth: f32,
    friction: f32,
    normal_impulse: f32,
    friction_impulse: f32,
}

/// Solve the contacts between one car and the props it touches the game's way: the impulse from the
/// relative velocity (and the overlap), each body taking its share of it (`r`). Normal and friction
/// impulses accumulate and are clamped like the game's (`FUN_009be420`: normal ≥ 0; `FUN_009bed70`:
/// friction within μ × normal). Returns the closing speed before the solve per contact.
fn solve_contacts(
    car: &mut BodyState,
    props: &mut [BodyState],
    contacts: &mut [PropContact],
    r: &DynamicResponse,
    dt: f32,
) -> Vec<f32> {
    let closing: Vec<f32> = contacts
        .iter()
        .map(|c| (car.point_velocity(c.point) - props[c.prop].point_velocity(c.point)).dot(c.normal))
        .collect();
    for _ in 0..CONTACT_PASSES {
        for c in contacts.iter_mut() {
            let prop = &mut props[c.prop];
            let n = c.normal;
            let vn = (car.point_velocity(c.point) - prop.point_velocity(c.point)).dot(n);
            // Overlap pushes apart; a gap lets the bodies close it within the step.
            let target = if c.depth > 0.0 {
                -(PUSH_OUT * (c.depth - PUSH_OUT_SLOP).max(0.0) / dt).min(PUSH_OUT_MAX)
            } else {
                -c.depth / dt
            };
            let k = car.inv_effective(c.point, n) + prop.inv_effective(c.point, n);
            if k <= 0.0 {
                continue;
            }
            let total = (c.normal_impulse + (vn - target) / k).max(0.0);
            let dl = total - c.normal_impulse;
            c.normal_impulse = total;
            car.apply(c.point, -dl * n, r.car_linear, r.car_angular);
            prop.apply(c.point, dl * n, r.object_linear, r.object_angular);
            // Friction against the sliding direction.
            let rel = car.point_velocity(c.point) - prop.point_velocity(c.point);
            let vt = rel - n * rel.dot(n);
            let speed = vt.length();
            if speed < 1e-4 {
                continue;
            }
            let t = vt / speed;
            let kt = car.inv_effective(c.point, t) + prop.inv_effective(c.point, t);
            if kt <= 0.0 {
                continue;
            }
            let limit = c.friction * c.normal_impulse;
            let total = (c.friction_impulse + speed / kt).clamp(-limit, limit);
            let dl = total - c.friction_impulse;
            c.friction_impulse = total;
            car.apply(c.point, -dl * t, r.car_linear, r.car_angular);
            prop.apply(c.point, dl * t, r.object_linear, r.object_angular);
        }
    }
    closing
}

/// A prop's velocity state from its collider's mass properties (also valid while it is still static) and the
/// data's centre of mass.
fn prop_state(
    physics: &PropPhysics,
    pos: &Position,
    rot: &Rotation,
    v: &LinearVelocity,
    w: &AngularVelocity,
    props: &ColliderMassProperties,
) -> BodyState {
    let frame = Mat3::from_quat(rot.0 * props.local_inertial_frame);
    let inv = props.principal_angular_inertia.map(|i| if i > 1e-6 { 1.0 / i } else { 0.0 });
    BodyState {
        v: v.0,
        w: w.0,
        inv_mass: if props.mass > 0.0 { 1.0 / props.mass } else { 0.0 },
        inv_inertia: frame * Mat3::from_diagonal(inv) * frame.transpose(),
        com: pos.0 + rot.0 * physics.centre_of_mass,
    }
}

static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static PIECE_TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// The race cars' contacts with props, before each physics step (see the module docs):
/// 1. a standing prop the car's boxes reach is knocked loose when the car's momentum passes its threshold
///    (the game tests on bounding-box overlap, before the contact solve: `FUN_00554d80` → `FUN_0054ec10` →
///    `FUN_0054e570`); one that holds is left to avian, which stops the car against it;
/// 2. the contacts with knocked props are solved with the game's response scales.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn prop_contacts(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    surfaces: Res<WorldSurfaces>,
    crash: Option<Res<crate::crash::CrashState>>,
    mut cars: Query<
        (
            &mut Car,
            &Position,
            &Rotation,
            &mut LinearVelocity,
            &mut AngularVelocity,
            &ComputedMass,
            &ComputedAngularInertia,
            &ComputedCenterOfMass,
            Has<crate::drive::Player>,
        ),
        Without<PropBody>,
    >,
    shapes: Query<(&Collider, &Transform), Without<PropBody>>,
    mut props: Query<
        (
            &mut PropBody,
            &Collider,
            &Position,
            &Rotation,
            &mut LinearVelocity,
            &mut AngularVelocity,
            &ColliderMassProperties,
            &Transform,
        ),
        Without<Car>,
    >,
    mut stats: ResMut<PropStats>,
    mut impacts: MessageWriter<PropImpact>,
    mut knocked: MessageWriter<PropKnocked>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let now = time.elapsed_secs();
    let filter = SpatialQueryFilter::from_mask([GameLayer::LooseProp, GameLayer::Prop]);
    let crashed = crash.is_some_and(|c| c.active.is_some());
    // `MW_PROP_TRACE=1`: log every solved car-against-prop contact.
    let trace = *TRACE.get_or_init(|| std::env::var_os("MW_PROP_TRACE").is_some());
    for (mut car, pos, rot, mut v, mut w, mass, inertia, com, player) in &mut cars {
        car.prop_impact = 0.0;
        let speed = v.0.length();
        // Look ahead by the step's travel so a thin sign is not jumped over.
        let margin = speed * dt + 0.05;
        let mut entities: Vec<Entity> = Vec::new();
        let mut contacts: Vec<PropContact> = Vec::new();
        for collider in [car.body_collider, car.bumper_collider] {
            let Ok((shape, local)) = shapes.get(collider) else { continue };
            let shape_pos = pos.0 + rot.0 * local.translation;
            let shape_rot = rot.0 * local.rotation;
            let probe = match shape.shape_scaled().as_cuboid() {
                Some(c) => {
                    let h = c.half_extents + Vec3::splat(margin);
                    Collider::cuboid(2.0 * h.x, 2.0 * h.y, 2.0 * h.z)
                }
                None => shape.clone(),
            };
            for e in spatial.shape_intersections(&probe, shape_pos, shape_rot, &filter) {
                let Ok((mut prop, prop_shape, prop_pos, prop_rot, .., t)) = props.get_mut(e) else { continue };
                if prop.pinned {
                    // The car's whole momentum against the threshold (strictly greater, as the game's).
                    if hit_momentum(mass.value(), v.0) <= prop.physics.break_force {
                        continue;
                    }
                    prop.pinned = false;
                    knocked.write(PropKnocked { prop: e, by_player: player });
                    stats.knocked += 1;
                    let p = &prop.physics;
                    info!(
                        "prop {} ({:?}, {:.0} kg) knocked at ({:.1} {:.1} {:.1}) by a car at {:.1} m/s (momentum {:.0}, \
                         threshold {}); smash force {:?}",
                        p.object,
                        p.kind,
                        p.mass,
                        t.translation.x,
                        t.translation.y,
                        t.translation.z,
                        speed,
                        mass.value() * speed,
                        p.break_force,
                        p.smash_force
                    );
                    commands.entity(e).insert((RigidBody::Dynamic, GameLayer::loose_prop()));
                }
                let Ok(Some(c)) =
                    contact_query::contact(shape, shape_pos, shape_rot, prop_shape, *prop_pos, *prop_rot, margin)
                else {
                    continue;
                };
                let index = match entities.iter().position(|&x| x == e) {
                    Some(i) => i,
                    None => {
                        entities.push(e);
                        entities.len() - 1
                    }
                };
                // avian rotates the world contact points into each shape's frame without moving them
                // (`contact_query::contact`).
                let p1 = shape_rot * c.local_point1;
                let p2 = prop_rot.0 * c.local_point2;
                contacts.push(PropContact {
                    prop: index,
                    point: 0.5 * (p1 + p2),
                    normal: (shape_rot * c.local_normal1).normalize_or_zero(),
                    depth: c.penetration,
                    friction: prop.physics.friction,
                    normal_impulse: 0.0,
                    friction_impulse: 0.0,
                });
            }
        }
        if contacts.is_empty() {
            continue;
        }
        let response = match (player, crashed) {
            (true, true) => FULL_RESPONSE,
            (true, false) => surfaces.responses.player,
            (false, _) => surfaces.responses.ai,
        };
        let inv_inertia = inertia.rotated(rot.0).inverse();
        let mut car_state = BodyState {
            v: v.0,
            w: w.0,
            inv_mass: mass.inverse(),
            inv_inertia: Mat3::from_cols(inv_inertia * Vec3::X, inv_inertia * Vec3::Y, inv_inertia * Vec3::Z),
            com: pos.0 + rot.0 * com.0,
        };
        let mut states: Vec<BodyState> = entities
            .iter()
            .map(|&e| {
                let (prop, _, p, r, pv, pw, m, _) = props.get(e).expect("queried above");
                prop_state(&prop.physics, p, r, pv, pw, m)
            })
            .collect();
        let before = car_state.v;
        let closing = solve_contacts(&mut car_state, &mut states, &mut contacts, &response, dt);
        stats.contacts += contacts.len();
        if trace {
            for (c, s) in contacts.iter().zip(&closing) {
                let p = &states[c.prop];
                info!(
                    "prop contact t {now:.3}: car {:.2} -> {:.2} m/s, closing {s:.2}, depth {:.3}, normal {:.2}, \
                     impulse {:.1} / {:.1}, prop v {:.2} w {:.2} (1/m {:.4}), point {:.2}",
                    before.length(),
                    car_state.v.length(),
                    c.depth,
                    c.normal,
                    c.normal_impulse,
                    c.friction_impulse,
                    p.v,
                    p.w,
                    p.inv_mass,
                    c.point
                );
            }
        }
        if speed > 1.0 {
            stats.max_speed_loss = stats.max_speed_loss.max((before.length() - car_state.v.length()) / speed);
        }
        if player {
            let loss = before.length() - car_state.v.length();
            stats.player_speed_loss += loss;
            let total: f32 = contacts.iter().map(|c| c.normal_impulse).sum();
            if total > 0.0 {
                for (i, &e) in entities.iter().enumerate() {
                    let share: f32 = contacts.iter().filter(|c| c.prop == i).map(|c| c.normal_impulse).sum();
                    if let Ok((mut prop, ..)) = props.get_mut(e) {
                        prop.speed_loss += loss * share / total;
                    }
                }
            }
        }
        v.0 = car_state.v;
        w.0 = car_state.w;
        for (i, &e) in entities.iter().enumerate() {
            let Ok((mut prop, _, _, _, mut pv, mut pw, ..)) = props.get_mut(e) else { continue };
            pv.0 = states[i].v;
            pw.0 = states[i].w;
            // The contacts that pushed this prop: the strongest closing speed, and the force (N) the game's
            // pair record holds (normal impulse / dt, `FUN_00901740` +0x3c).
            let touching = contacts.iter().zip(&closing).filter(|(c, _)| c.prop == i && c.normal_impulse > 0.0);
            let Some((c, s)) = touching.clone().max_by(|a, b| a.1.total_cmp(b.1)) else { continue };
            let force: f32 = touching.map(|(c, _)| c.normal_impulse).sum::<f32>() / dt;
            car.prop_impact = car.prop_impact.max(*s);
            prop.car_force = prop.car_force.max(force);
            // `ForceToDamage` passed by a vehicle's contact: the prop's impact effect, once per knocked lifetime
            // (`FUN_0054bf70`).
            let effect = (!prop.damaged && force > prop.physics.force_to_damage).then(|| {
                prop.damaged = true;
                stats.effects += prop.physics.impact_effect.is_some() as usize;
                prop.physics.impact_effect
            });
            let sound = *s >= IMPACT_SOUND_SPEED && now - prop.last_impact > IMPACT_SOUND_GAP;
            if sound {
                prop.last_impact = now;
            }
            if effect.is_some() || sound {
                impacts.write(PropImpact {
                    position: c.point,
                    normal: -c.normal,
                    velocity: before,
                    closing_speed: *s,
                    effect: effect.flatten(),
                    sound: if sound { prop.physics.impact_sound } else { None },
                    by_player: player,
                });
            }
        }
    }
}

/// Knocked props go back where they stood (or are removed, `mbRemoveInsteadOfReset`) as soon as both the
/// camera and the player's car are farther from them than their reset distance (`FUN_0055ae50`, every frame,
/// no timer; `mfResetDistanceSquared` against both squared distances).
#[allow(clippy::type_complexity)]
fn reset_far_props(
    mut commands: Commands,
    car: Query<&GlobalTransform, (With<Car>, With<crate::drive::Player>)>,
    camera: Query<&GlobalTransform, crate::MainCamera>,
    mut props: Query<(Entity, &mut PropBody, &mut Transform, Option<&mut Compound>), Without<CompoundPiece>>,
    pieces: Query<&Transform, With<CompoundPiece>>,
    mut grid: ResMut<PropGrid>,
    mut stats: ResMut<PropStats>,
) {
    let Ok(car) = car.single() else { return };
    let car = car.translation();
    let camera = camera.iter().next().map_or(car, |c| c.translation());
    for (e, mut prop, mut t, compound) in &mut props {
        if prop.pinned {
            continue;
        }
        let d = prop.physics.reset_distance;
        let near = |p: Vec3| p.distance(car) <= d || p.distance(camera) <= d;
        // A smashed compound goes back once neither its home nor any of its parts is near (model decision: the
        // game resets each part in its own slot).
        let smashed = compound.as_ref().is_some_and(|c| c.smashed);
        if near(if smashed { prop.home.translation } else { t.translation }) {
            continue;
        }
        if let Some(c) = &compound
            && c.parts.iter().flatten().any(|&p| pieces.get(p).is_ok_and(|pt| near(pt.translation)))
        {
            continue;
        }
        stats.reset += 1;
        if let Some(mut c) = compound {
            for p in c.parts.iter_mut().filter_map(Option::take) {
                commands.entity(p).despawn();
            }
            if c.smashed {
                c.smashed = false;
                // Back without a body: [`stream_props`] gives it one again when a car comes near.
                prop.active = false;
                grid.active.remove(&e);
                commands.entity(e).insert(Visibility::Inherited);
            }
        }
        if prop.physics.remove_instead_of_reset {
            commands.entity(e).despawn();
            continue;
        }
        *t = prop.home;
        prop.pinned = true;
        prop.damaged = false;
        prop.loose_for = 0.0;
        if prop.active {
            commands.entity(e).insert((
                RigidBody::Static,
                GameLayer::prop(),
                Position(prop.home.translation),
                Rotation(prop.home.rotation),
                LinearVelocity::ZERO,
                AngularVelocity::ZERO,
            ));
        }
    }
}

/// The physics components a prop carries while it has a body ([`physics_bundle`]).
type PhysicsComponents = (
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
);

/// Distance (m) from the player's car within which a loose prop's knock against the world is heard. Model
/// decision: the impact sounds are played without 3D placement (`prop_sounds`).
const WORLD_SOUND_DISTANCE: f32 = 40.0;

/// A knocked prop (or a compound's part) landing on the world plays its impact sound (`FUN_0054c680`: the
/// object's `WCPlaySoundBehaviour`, or "World_Prop_Collision" without one, not played here; then 0.35 s quiet).
/// Model decision: the game's threshold is a float in an audio object (+0x94 of the handle at +0xb270), not read;
/// the speed the contact takes off the prop (normal impulse / mass) stands in for it, with the car contacts'
/// [`IMPACT_SOUND_SPEED`].
#[allow(clippy::type_complexity)]
fn prop_world_sounds(
    time: Res<Time>,
    collisions: Collisions,
    player: Query<&GlobalTransform, (With<Car>, With<crate::drive::Player>)>,
    mut props: Query<(Entity, &mut PropBody, &Position)>,
    world: Query<(), (Without<PropBody>, Without<Car>, Without<crate::traffic::TrafficBody>)>,
    mut impacts: MessageWriter<PropImpact>,
) {
    let Ok(player) = player.single() else { return };
    let player = player.translation();
    let now = time.elapsed_secs();
    for (e, mut prop, pos) in &mut props {
        if prop.pinned || prop.physics.impact_sound.is_none() || pos.0.distance(player) > WORLD_SOUND_DISTANCE {
            continue;
        }
        if now - prop.last_impact <= IMPACT_SOUND_GAP || prop.physics.mass <= 0.0 {
            continue;
        }
        let hit = collisions
            .collisions_with(e)
            .filter(|p| world.contains(if p.collider1 == e { p.collider2 } else { p.collider1 }))
            .flat_map(|p| p.manifolds.iter().map(move |m| (m, p.collider1 == e)))
            .map(|(m, first)| (m.total_normal_impulse(), if first { -m.normal } else { m.normal }, m.points.first()))
            .max_by(|a, b| a.0.total_cmp(&b.0));
        let Some((impulse, normal, point)) = hit else { continue };
        let speed = impulse / prop.physics.mass;
        if speed < IMPACT_SOUND_SPEED {
            continue;
        }
        prop.last_impact = now;
        impacts.write(PropImpact {
            position: point.map_or(pos.0, |p| p.point),
            normal,
            velocity: Vec3::ZERO,
            closing_speed: speed,
            effect: None,
            sound: prop.physics.impact_sound,
            by_player: true,
        });
    }
}

/// The largest contact force (N) on a body this step: its avian contact pairs (the world, traffic, other props)
/// and a race car's contacts from [`prop_contacts`] (the game compares the world-contact record and each pair
/// record with the threshold, `FUN_00555c90`: normal impulse over the step).
fn contact_force(collisions: &Collisions, e: Entity, car_force: f32, dt: f32) -> f32 {
    collisions.collisions_with(e).map(|p| p.total_normal_impulse_magnitude() / dt).fold(car_force, f32::max)
}

/// Compound objects come apart (`FUN_00555c90`, after the physics step; docs/decomp.md "Props"):
/// - a knocked compound smashes once it has been loose for `08c87eeb` seconds and a contact force passes its
///   `ForceToSmash` (strictly): its body goes (`FUN_0054e210`) and its first parts take its place, each a body
///   of its own at its skeleton bone (messages 2 → `FUN_00558bb0` → `FUN_00554500` / `FUN_005547f0`; placed by
///   `FUN_00532b00`), at rest (`FUN_005341a0` clears the part's velocities), pinned as their own `AdditionalInfo`
///   says;
/// - a part with a `SmashProxyBehaviour` is replaced by its replacement chain when a contact force passes the
///   behaviour's `ForceToSmash` (message 3 in `FUN_0055a680`): each new part at its own bone relative to where
///   the old one is, moving with it (`FUN_00536050`: the old body's velocity at the new part's position, the
///   same spin);
/// - a pinned part linked to other parts comes loose once none of them is still pinned (`FUN_0054e750`).
#[allow(clippy::type_complexity)]
fn smash_compounds(
    mut commands: Commands,
    time: Res<Time>,
    collisions: Collisions,
    mut compounds: Query<(Entity, &mut PropBody, &mut Compound, &Position, &Rotation), Without<CompoundPiece>>,
    mut pieces: Query<
        (Entity, &mut PropBody, &CompoundPiece, &Position, &Rotation, &LinearVelocity, &AngularVelocity),
        Without<Compound>,
    >,
    mut stats: ResMut<PropStats>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let now = time.elapsed_secs();
    for (e, mut prop, mut compound, pos, rot) in &mut compounds {
        let force = contact_force(&collisions, e, prop.car_force, dt);
        prop.car_force = 0.0;
        if prop.pinned || compound.smashed {
            continue;
        }
        let (Some(smash), Some(parts)) = (prop.physics.smash_force, prop.physics.compound.as_ref()) else { continue };
        // The game checks the timer before it counts it down (`<= 0`, then `-= dt`).
        if prop.loose_for < prop.physics.smash_delay || force <= smash {
            prop.loose_for += dt;
            continue;
        }
        stats.smashed += 1;
        stats.last_smash = pos.0;
        let world = Mat4::from_rotation_translation(rot.0, pos.0);
        let mut spawned = 0;
        for i in (0..parts.parts.len()).filter(|&i| parts.parts[i].replaces.is_none()) {
            let p = spawn_part(&mut commands, e, &compound, parts, i, world, Vec3::ZERO, Vec3::ZERO);
            spawned += p.is_some() as usize;
            compound.parts[i] = p;
        }
        stats.pieces += spawned;
        info!(
            "compound {} smashed at t {now:.2} ({:.2} s loose, contact force {force:.0} N > {smash}): {spawned} parts",
            prop.physics.object, prop.loose_for
        );
        compound.smashed = true;
        commands.entity(e).remove::<PhysicsComponents>().insert(Visibility::Hidden);
    }

    // `MW_PIECE_TRACE=1`: where the parts of smashed compounds are, twice a second.
    if *PIECE_TRACE.get_or_init(|| std::env::var_os("MW_PIECE_TRACE").is_some()) && (now * 2.0).fract() < dt * 2.0 {
        for (_, prop, piece, pos, _, v, _) in &pieces {
            info!(
                "piece t {now:.2}: part {} of {:?} ({:.0} kg, {}) at {:.2}, v {:.2}",
                piece.part,
                piece.compound,
                prop.physics.mass,
                if prop.pinned { "pinned" } else { "loose" },
                pos.0,
                v.0
            );
        }
    }

    // Parts: smash proxies and links.
    let mut splits = Vec::new();
    for (e, mut prop, piece, pos, rot, v, w) in &mut pieces {
        let force = contact_force(&collisions, e, prop.car_force, dt);
        prop.car_force = 0.0;
        if !prop.pinned && prop.physics.smash_force.is_some_and(|f| force > f) {
            splits.push((e, piece.compound, piece.part, *pos, *rot, v.0, w.0, prop.physics.centre_of_mass, force));
        }
    }
    for (e, owner, part, pos, rot, v, w, com, force) in splits {
        let Ok((_, owner_prop, mut compound, ..)) = compounds.get_mut(owner) else { continue };
        let Some(parts) = owner_prop.physics.compound.clone() else { continue };
        let old = &parts.parts[part];
        let old_world = Mat4::from_rotation_translation(rot.0, pos.0);
        let old_com = old_world.transform_point3(com);
        // The compound's frame as the old part sees it now (old world × old bone⁻¹).
        let frame = old_world * old.bone.inverse();
        let mut next = old.first_replacement;
        let mut spawned = 0;
        while let Some(i) = next {
            let new = &parts.parts[i];
            let com_i =
                (frame * new.bone).transform_point3(new.physics.as_ref().map_or(Vec3::ZERO, |p| p.centre_of_mass));
            let v_i = v + w.cross(com_i - old_com);
            let p = spawn_part(&mut commands, owner, &compound, &parts, i, frame, v_i, w);
            spawned += p.is_some() as usize;
            compound.parts[i] = p;
            next = new.next;
        }
        compound.parts[part] = None;
        commands.entity(e).despawn();
        stats.splits += 1;
        stats.pieces += spawned;
        stats.last_smash = pos.0;
        info!(
            "compound {} part {part} smashed (contact force {force:.0} N > {:?}): {spawned} parts",
            owner_prop.physics.object,
            old.physics.as_ref().and_then(|p| p.smash_force)
        );
    }

    // Links (`FUN_0054e750`, every frame): a pinned part comes loose once no linked part is still pinned.
    let mut unpin = Vec::new();
    for (_, owner_prop, compound, ..) in &compounds {
        let Some(parts) = &owner_prop.physics.compound else { continue };
        if !compound.smashed {
            continue;
        }
        let pinned = |i: usize| compound.parts[i].is_some_and(|p| pieces.get(p).is_ok_and(|q| q.1.pinned));
        for (i, part) in parts.parts.iter().enumerate() {
            let Some(p) = compound.parts[i] else { continue };
            if !part.links.is_empty() && pinned(i) && !part.links.iter().any(|&l| pinned(l)) {
                unpin.push(p);
            }
        }
    }
    for p in unpin {
        if let Ok((_, mut q, ..)) = pieces.get_mut(p) {
            q.pinned = false;
            stats.knocked += 1;
            commands.entity(p).insert((RigidBody::Dynamic, GameLayer::loose_prop()));
        }
    }
}

/// Spawn part `i` of a compound as a prop of its own: `frame` is the compound's frame in the world (the part
/// sits at its bone in it), `v` / `w` its velocity.
#[allow(clippy::too_many_arguments)]
fn spawn_part(
    commands: &mut Commands,
    owner: Entity,
    compound: &Compound,
    parts: &CompoundBreak,
    i: usize,
    frame: Mat4,
    v: Vec3,
    w: Vec3,
) -> Option<Entity> {
    let part = &parts.parts[i];
    let physics = part.physics.clone()?;
    let (_, rotation, translation) = (frame * part.bone).to_scale_rotation_translation();
    let t = Transform { translation, rotation, scale: Vec3::ONE };
    debug!("compound part {i} ({:.0} kg) at {translation:.2}", physics.mass);
    // Pinned as the part's own `AdditionalInfo` says (`FUN_0054dfe0`, `94c94bab`); the rest move at once (the
    // game creates them asleep, but next to the moving compound: model decision).
    let pinned = physics.break_force > 0.0;
    let mut prop = PropBody::new(physics.clone(), t, pinned);
    prop.active = true;
    let mut e = commands.spawn((t, Visibility::default(), prop, CompoundPiece { compound: owner, part: i }));
    e.insert(physics_bundle(&physics));
    if !pinned {
        e.insert((RigidBody::Dynamic, GameLayer::loose_prop(), LinearVelocity(v), AngularVelocity(w)));
    }
    if let Some(Some(meshes)) = compound.meshes.parts.get(i) {
        let local = Transform::from_matrix(part.bone.inverse());
        e.with_children(|c| {
            spawn_meshes(c, meshes, local, compound.range);
        });
    }
    Some(e.id())
}

/// Decoded impact sounds, per `WCPlaySoundBehaviour` and per wave.
#[derive(Resource, Default)]
struct PropSounds {
    waves: HashMap<ResourceId, Vec<ResourceId>>,
    pcm: HashMap<ResourceId, Option<Arc<mw_audio::Pcm>>>,
    next: usize,
}

/// Closing speed (m/s) of a full-volume prop impact sound. Model decision: the behaviour's own volume
/// rules (`WCPlaySoundBehaviour` fields `0228a2f8` and others) are not decoded.
const PROP_SOUND_FULL_SPEED: f32 = 12.0;

/// Play the impact sound of a prop the player's car hits: one of its `Collision_Waves` in turn.
fn prop_sounds(
    mut commands: Commands,
    game: Res<crate::Game>,
    mut impacts: MessageReader<PropImpact>,
    mut sounds: ResMut<PropSounds>,
    audio: Option<ResMut<Assets<crate::audio::PcmAudio>>>,
) {
    let Some(mut audio) = audio else {
        impacts.clear();
        return;
    };
    for hit in impacts.read() {
        if game.mute || !hit.by_player {
            continue;
        }
        let Some(sound) = hit.sound else { continue };
        let store = game.store.clone();
        let waves =
            sounds.waves.entry(sound).or_insert_with(|| store.impact_sound_waves(sound).unwrap_or_default()).clone();
        if waves.is_empty() {
            continue;
        }
        sounds.next = sounds.next.wrapping_add(1);
        let wave = waves[sounds.next % waves.len()];
        let pcm = sounds
            .pcm
            .entry(wave)
            .or_insert_with(|| {
                let res = store.load(wave).ok()?;
                let pcm = mw_audio::sps::decode(res.header().get(0x80..)?).ok()?;
                (!pcm.samples.is_empty()).then(|| Arc::new(pcm))
            })
            .clone();
        let Some(pcm) = pcm else { continue };
        let gain = (hit.closing_speed / PROP_SOUND_FULL_SPEED).clamp(0.15, 1.0);
        let scaled = mw_audio::Pcm {
            channels: pcm.channels,
            sample_rate: pcm.sample_rate,
            samples: pcm.samples.iter().map(|&s| (s as f32 * gain) as i16).collect(),
        };
        let handle = audio.add(crate::audio::PcmAudio { pcm: Arc::new(scaled) });
        commands.spawn((
            AudioPlayer::<crate::audio::PcmAudio>(handle),
            PlaybackSettings::DESPAWN.with_volume(bevy::audio::Volume::Linear(0.0)),
            crate::settings::Channel::Sfx,
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
    /// The list entry for ground without surface tags (the flat test track): 0, the game's `Default_surface`,
    /// unless `--track-surface` picks another.
    pub untagged: usize,
    /// How cars' contacts with knocked props are solved (`PHYSICS/PHYSICSCONFIG.BNDL`; the data's values
    /// until the loader has read it).
    pub responses: DynamicResponses,
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

/// The smash test's layout along the car's path (+Z from the origin): a traffic cone (5 kg, loose from the
/// start), a sign (50 kg, momentum 500, with an impact effect and sound), a street lamp (150 kg compound,
/// 5000), a 6 m fence panel (50 kg, 2500), and an immovable 5 m wall (a mass-0 dynamic object).
/// `--smash-set compounds`: six compound objects 30 m apart: the single-arm street lamp (934 placements; its pole
/// splits in two by a `SmashProxyBehaviour`), the double lamp, two pole compounds, a 600 kg object and an
/// 11-panel structure whose panels are pinned and linked. `--smash-set bags`: objects without an
/// `AdditionalInfo` (never pinned): the trash bag (10 kg, 353 placements), a 0.2 kg crate and a 7 kg one.
pub fn smash_test_props(set: &str) -> Vec<crate::loader::TestProp> {
    use crate::loader::TestProp;
    let p =
        |id: u64, kind, z: f32| TestProp { object: ResourceId(id), kind, position: Vec3::new(0.0, 0.0, z), yaw: 0.0 };
    if set == "bags" {
        return vec![
            p(0x03102fec, ObjectKind::Prop, 40.0),
            p(0x80a6d3c6, ObjectKind::Prop, 60.0),
            p(0x9593a9cf, ObjectKind::Prop, 80.0),
        ];
    }
    if set == "compounds" {
        // The panel structure reaches 2.5 m below its origin (bounds centre 0.04, half height 2.56): raised to
        // stand on the track.
        return [
            (0xfed64b97, 0.0),
            (0x621c484f, 0.0),
            (0xe547f377, 0.0),
            (0xf3afb6cd, 0.0),
            (0x1e63a9d5, 0.0),
            (0xac10f734, 2.6),
        ]
        .iter()
        .enumerate()
        .map(|(i, &(id, y))| {
            let mut prop = p(id, ObjectKind::Compound, 40.0 + 30.0 * i as f32);
            prop.position.y = y;
            prop
        })
        .collect();
    }
    vec![
        p(0x3698c67a, ObjectKind::Prop, 40.0),
        p(0x39c4412d, ObjectKind::Prop, 60.0),
        p(0xfed64b97, ObjectKind::Compound, 80.0),
        p(0x406f7f52, ObjectKind::Prop, 120.0),
        p(0xd22c0396, ObjectKind::Dynamic, 170.0),
    ]
}

/// Seconds of test time between a knock and its `-knock<n>` screenshot, and between a smash and its
/// `-smash<n>` one.
const KNOCK_SHOT_DELAY: f32 = 0.25;
const SMASH_SHOT_DELAY: f32 = 0.6;

#[derive(Resource)]
pub struct SmashTest {
    out: Option<std::path::PathBuf>,
    speed: f32,
    phase: SmashPhase,
    timer: f32,
    stalled: f32,
    top_speed: f32,
    knocked: usize,
    /// `PropStats::player_speed_loss` at the last knock.
    loss_mark: f32,
    /// When to save the `-knock<n>` screenshot after a knock (test time).
    knock_shot: Option<f32>,
    /// `--smash-set compounds`: the camera follows the knocks, and a `-smash<n>` screenshot follows each smash.
    follow: bool,
    focus: f32,
    smashed: usize,
    smash_shot: Option<f32>,
    /// How many props the set places (the drive starts once they are all spawned).
    props: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SmashPhase {
    Settle,
    Drive,
    Done,
}

impl SmashTest {
    pub fn new(out: Option<std::path::PathBuf>, speed: f32, follow: bool, props: usize) -> Self {
        Self {
            out,
            props,
            speed,
            phase: SmashPhase::Settle,
            timer: 0.0,
            stalled: 0.0,
            top_speed: 0.0,
            knocked: 0,
            loss_mark: 0.0,
            knock_shot: None,
            follow,
            focus: 30.0,
            smashed: 0,
            smash_shot: None,
        }
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
    cars: Query<(&Car, &Transform), With<crate::drive::Player>>,
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
            if test.timer > 2.5 && props.iter().count() >= test.props {
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
            let report_loss = |test: &mut SmashTest, what: &str| {
                if test.knocked > 0 {
                    info!(
                        "TEST RESULT smash: speed lost to knocked props since the last knock ({what}): {:.2} km/h",
                        (stats.player_speed_loss - test.loss_mark) * 3.6
                    );
                }
                test.loss_mark = stats.player_speed_loss;
            };
            if stats.knocked > test.knocked {
                report_loss(&mut test, "until the next knock");
                info!("TEST smash: prop knocked at car z {:.1}, car {:.0} km/h", transform.translation.z, speed * 3.6);
                test.knocked = stats.knocked;
                test.knock_shot = Some(test.timer + KNOCK_SHOT_DELAY);
                if test.smash_shot.is_none() {
                    test.focus = transform.translation.z;
                }
            }
            if stats.smashed + stats.splits > test.smashed {
                test.smashed = stats.smashed + stats.splits;
                test.smash_shot = Some(test.timer + SMASH_SHOT_DELAY);
                test.focus = stats.last_smash.z - 4.0;
            }
            if test.smash_shot.is_some_and(|t| test.timer >= t) {
                test.smash_shot = None;
                if let Some(p) = test.path(&format!("smash{}", test.smashed)) {
                    commands.spawn(Screenshot::primary_window()).observe(save_to_disk(p));
                }
            }
            if test.knock_shot.is_some_and(|t| test.timer >= t) {
                test.knock_shot = None;
                if let Some(p) = test.path(&format!("knock{}", test.knocked)) {
                    commands.spawn(Screenshot::primary_window()).observe(save_to_disk(p));
                }
            }
            test.stalled = if speed < 0.5 && test.timer > 3.0 { test.stalled + dt } else { 0.0 };
            if test.stalled > 2.0 || transform.translation.z > 220.0 || test.timer > 60.0 {
                report_loss(&mut test, "until the end");
                info!(
                    "TEST RESULT smash: {} prop contacts solved, largest one-step speed loss {:.2}%, {} impact effects",
                    stats.contacts,
                    stats.max_speed_loss * 100.0,
                    stats.effects
                );
                info!(
                    "TEST RESULT smash: {} compounds smashed, {} parts smashed by their proxy, {} part bodies spawned",
                    stats.smashed, stats.splits, stats.pieces
                );
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
                        "TEST RESULT smash: {:?} {} ({:.0} kg, break {}): {} at ({:.1} {:.1} {:.1}), placed at z {:.0}, \
                         car speed lost to it {:.2} km/h",
                        p.physics.kind,
                        p.physics.object,
                        p.physics.mass,
                        p.physics.break_force,
                        if p.pinned { "standing" } else { "knocked" },
                        t.translation.x,
                        t.translation.y,
                        t.translation.z,
                        p.home.translation.z,
                        p.speed_loss * 3.6
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

/// A fixed side view over the props instead of the chase camera; with `--smash-set compounds`, a view from
/// beside the last knock.
fn smash_camera(test: Res<SmashTest>, mut camera: Query<&mut Transform, crate::MainCamera>) {
    let Ok(mut cam) = camera.single_mut() else { return };
    *cam = if test.follow {
        let z = test.focus;
        Transform::from_xyz(-11.0, 4.5, z - 4.0).looking_at(Vec3::new(0.0, 2.5, z + 9.0), Vec3::Y)
    } else {
        Transform::from_xyz(-16.0, 6.0, 60.0).looking_at(Vec3::new(0.0, 1.5, 118.0), Vec3::Y)
    };
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
