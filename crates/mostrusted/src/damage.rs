//! Crash damage: the car body deforms the way the original's does.
//!
//! The game skins every car body to a small skeleton and poses it from per-zone damage clips (front,
//! rear, sides, corners; frames = damage level), see `mw_assets::damage`. Here the body meshes become
//! Bevy skinned meshes with one joint entity per bone, impacts on the body collider raise the damage
//! level of the zones facing the contact point, and the joints are re-posed from the clips.
//!
//! Impact strength: the approach speed along the contact normal (avian's pre-solve `normal_speed`)
//! through the game's `ImpactDamageGraphs` curve. A standing lamp post that holds counts like a wall.
//! Knocked props do not reach this: their contacts with the race cars are solved by `props::prop_contacts`
//! outside avian (the car takes a tenth of the impulse) and deal no damage here (model decision: the game's
//! "prop impact" rule, `FUN_00941170` → `FUN_009a9fd0`, is not ported). Which curve and units the game uses for which kind
//! of impact is not known yet (research log, "vehicle damage").

use std::path::PathBuf;
use std::sync::Arc;

use avian3d::prelude::*;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use mw_assets::damage::{ImpactGraph, Similarity, VehicleDamage};

use crate::drive::{Car, CarInput, PendingCar};

pub struct DamagePlugin;

impl Plugin for DamagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NextCarDamage>()
            .add_observer(attach_damage)
            .add_systems(Update, (remember_pending_damage, apply_initial_damage, repair_on_reset, pose_joints).chain())
            .add_systems(FixedPostUpdate, detect_impacts.after(PhysicsSystems::StepSimulation));
        if app.world().contains_resource::<CrashTest>() {
            app.add_systems(Startup, spawn_crash_wall)
                .add_systems(Update, run_crash_test)
                .add_systems(PostUpdate, crash_camera.before(TransformSystems::Propagate));
        }
    }
}

/// Damage data of the car about to spawn (taken from `PendingCar`, which is removed on spawn).
#[derive(Resource, Default)]
struct NextCarDamage(Option<(Arc<VehicleDamage>, ImpactGraph)>);

/// The damage model of a car spawned outside the player's `PendingCar` path (an AI car, `ai.rs`): inserted with
/// the car so [`attach_damage`] takes it instead of the pending player car's.
#[derive(Component, Clone)]
pub struct DamageSource(pub Option<(Arc<VehicleDamage>, ImpactGraph)>);

/// Damage state of a car.
#[derive(Component)]
pub struct CarDamage {
    model: Arc<VehicleDamage>,
    graph: ImpactGraph,
    /// Damage level (0..1) per zone clip.
    pub levels: Vec<f32>,
    /// Position (0..1) of each animated body part clip (`car_anim.rs`), posed on top of the damage.
    parts: Vec<f32>,
    joints: Vec<Entity>,
    collider: Entity,
    dirty: bool,
    /// Largest impact so far: (approach speed m/s, car-space point).
    pub worst_impact: Option<(f32, Vec3)>,
    /// Current impact event: peak approach speed so far and seconds since the last approaching contact.
    event_peak: f32,
    event_quiet: f32,
}

/// Contacts closer together than this belong to one impact (a crash into a wall touches several
/// triangles over a few steps, and scraping along it keeps making new contacts).
const IMPACT_GAP: f32 = 0.3;

impl CarDamage {
    pub fn zone_levels(&self) -> impl Iterator<Item = (&'static str, f32)> + '_ {
        self.model.clips.iter().zip(&self.levels).map(|(c, l)| (c.zone.name, *l))
    }

    pub fn animations(&self) -> Option<&mw_assets::car_anim::CarAnimations> {
        self.model.animations.as_ref()
    }

    pub fn parts(&self) -> &[f32] {
        &self.parts
    }

    /// The game's repair (`FUN_0058f060`, vehicle-manager message 0x16): every damage level back to 0.
    pub fn repair(&mut self) {
        self.levels.iter_mut().for_each(|l| *l = 0.0);
        self.worst_impact = None;
        self.dirty = true;
    }

    /// Set the zone damage levels (0..1, in clip order; `SetPlayerDamageSequenceItem`, `cutscene_spawns.rs`).
    pub fn set_levels(&mut self, levels: &[f32]) {
        for (l, v) in self.levels.iter_mut().zip(levels) {
            *l = v.clamp(0.0, 1.0);
        }
        self.dirty = true;
    }

    /// Set the animated body part positions; re-poses the skeleton when they changed.
    pub fn set_parts(&mut self, parts: &[f32]) {
        if self.parts != parts {
            self.parts = parts.to_vec();
            self.dirty = true;
        }
    }
}

fn remember_pending_damage(pending: Option<Res<PendingCar>>, mut next: ResMut<NextCarDamage>) {
    if let Some(p) = pending
        && p.is_added()
    {
        next.0 = p.spec.damage.clone().map(|d| (d, p.spec.impact_graph));
    }
}

fn similarity_transform(s: &Similarity) -> Transform {
    let [x, y, z, w] = s.rotation;
    Transform {
        translation: Vec3::from(s.translation),
        rotation: Quat::from_xyzw(x, y, z, w),
        scale: Vec3::splat(s.scale),
    }
}

/// When a car spawns: one joint entity per bone (children of the car, in car space = model space), and
/// a `SkinnedMesh` on each body mesh (the car's direct children with a mesh; wheels hang below pivots).
/// Runs as an observer so the meshes are never drawn with joint attributes but without a skin.
#[allow(clippy::too_many_arguments)]
fn attach_damage(
    add: On<Add, Car>,
    mut commands: Commands,
    mut next: ResMut<NextCarDamage>,
    sources: Query<&DamageSource>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
    colliders: Query<(), With<Collider>>,
    mut bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
) {
    let car = add.entity;
    let source = match sources.get(car) {
        Ok(s) => s.0.clone(),
        Err(_) => next.0.take(),
    };
    let Some((model, graph)) = source else { return };
    let kids: Vec<Entity> = children.get(car).map(|c| c.iter().collect()).unwrap_or_default();
    let Some(collider) = kids.iter().copied().find(|&e| colliders.contains(e)) else { return };
    let joints: Vec<Entity> = model
        .bind
        .iter()
        .enumerate()
        .map(|(i, b)| {
            commands.spawn((Name::new(format!("damage bone {i}")), similarity_transform(b), ChildOf(car))).id()
        })
        .collect();
    let inverse: Vec<Mat4> = model.bind.iter().map(|b| similarity_transform(b).to_matrix().inverse()).collect();
    let inverse_bindposes = bindposes.add(SkinnedMeshInverseBindposes::from(inverse));
    for &e in kids.iter().filter(|&&e| meshes.contains(e)) {
        commands.entity(e).insert(SkinnedMesh { inverse_bindposes: inverse_bindposes.clone(), joints: joints.clone() });
    }
    let levels = vec![0.0; model.clips.len()];
    let parts = vec![0.0; model.animations.as_ref().map_or(0, |a| a.clips.len())];
    info!("damage model: {} bones, {} zone clips", model.bind.len(), model.clips.len());
    commands.entity(car).insert(CarDamage {
        model,
        graph,
        levels,
        parts,
        joints,
        collider,
        dirty: false,
        worst_impact: None,
        event_peak: 0.0,
        event_quiet: f32::MAX,
    });
}

/// Turn contact impacts on the body collider into zone damage.
fn detect_impacts(
    time: Res<Time>,
    collisions: Collisions,
    mut cars: Query<(&mut CarDamage, &Position, &Rotation, &ComputedMass)>,
    props: Query<&crate::props::PropBody>,
) {
    for (mut dmg, pos, rot, car_mass) in &mut cars {
        dmg.event_quiet += time.delta_secs();
        // Approaching contact points this step. A box face against a wall gives a point per corner, so
        // the impact point is their centroid (weighted by approach speed) and the speed the fastest.
        // Contacts with near-vertical normals are the body landing on the ground; the game has separate
        // landing damage rules (`Physics.LandingDamage`, not decoded), so they don't count here.
        let collider = dmg.collider;
        let points: Vec<(f32, Vec3)> = collisions
            .collisions_with(collider)
            .flat_map(|pair| {
                let other = if pair.collider1 == collider { pair.collider2 } else { pair.collider1 };
                let scale = match props.get(other) {
                    Ok(p) if !p.pinned => {
                        let m = p.physics.mass.max(1.0);
                        m / (car_mass.value() + m)
                    }
                    _ => 1.0,
                };
                pair.manifolds.iter().map(move |m| (m, scale))
            })
            .filter(|(m, _)| m.normal.y.abs() < 0.7)
            .flat_map(|(m, scale)| m.points.iter().map(move |p| (-p.normal_speed * scale, p.point)))
            .filter(|(s, _)| *s > 0.0)
            .collect();
        let speed = points.iter().map(|p| p.0).fold(0.0, f32::max);
        if speed <= 0.0 {
            continue;
        }
        let strong = points.iter().filter(|p| p.0 >= 0.5 * speed);
        let weight: f32 = strong.clone().map(|p| p.0).sum();
        let point = strong.fold(Vec3::ZERO, |acc, p| acc + p.1 * p.0) / weight;
        // One impact event deals the damage of its fastest contact, not the sum of all its contacts.
        if dmg.event_quiet > IMPACT_GAP {
            dmg.event_peak = 0.0;
        }
        dmg.event_quiet = 0.0;
        let amount = dmg.graph.damage(speed) - dmg.graph.damage(dmg.event_peak);
        dmg.event_peak = dmg.event_peak.max(speed);
        if amount < 0.005 {
            continue;
        }
        let local = rot.0.inverse() * (point - pos.0);
        let weights = dmg.model.zone_weights(local.to_array());
        let mut changed = false;
        for (level, w) in dmg.levels.iter_mut().zip(weights) {
            let new = (*level + amount * w).min(1.0);
            changed |= new != *level;
            *level = new;
        }
        if dmg.worst_impact.is_none_or(|(s, _)| speed > s) {
            dmg.worst_impact = Some((speed, local));
        }
        dmg.dirty |= changed;
        info!(
            "impact {:.1} km/h at car ({:.2} {:.2} {:.2}) -> +{:.0}% damage; zones {:?}",
            speed * 3.6,
            local.x,
            local.y,
            local.z,
            amount * 100.0,
            dmg.zone_levels()
                .filter(|(_, l)| *l > 0.0)
                .map(|(n, l)| format!("{n} {:.0}%", l * 100.0))
                .collect::<Vec<_>>()
        );
    }
}

/// `--damage`: zone levels the player car starts with, as (zone name, clip index or `all`, level).
#[derive(Resource, Default)]
pub struct InitialDamage(Vec<(String, f32)>);

impl InitialDamage {
    pub fn parse(args: &[String]) -> anyhow::Result<Self> {
        let pins = args
            .iter()
            .map(|a| {
                let (k, v) = a.split_once('=').ok_or_else(|| anyhow::anyhow!("--damage {a}: expected zone=0..1"))?;
                Ok((k.to_string(), v.parse::<f32>()?.clamp(0.0, 1.0)))
            })
            .collect::<anyhow::Result<_>>()?;
        Ok(Self(pins))
    }
}

fn apply_initial_damage(
    initial: Res<InitialDamage>,
    mut cars: Query<&mut CarDamage, (Added<CarDamage>, With<crate::drive::Player>)>,
) {
    if initial.0.is_empty() {
        return;
    }
    for mut dmg in &mut cars {
        let names: Vec<&str> = dmg.model.clips.iter().map(|c| c.zone.name).collect();
        for (zone, level) in &initial.0 {
            for (i, l) in dmg.levels.iter_mut().enumerate() {
                if zone == "all" || zone == names[i] || zone.parse::<usize>() == Ok(i) {
                    *l = *level;
                }
            }
        }
        info!("--damage: zones {:?}", dmg.zone_levels().collect::<Vec<_>>());
        dmg.dirty = true;
    }
}

/// The reset key (R, which also puts the player's car back on its wheels) repairs that car (not the opponents').
fn repair_on_reset(keys: Res<ButtonInput<KeyCode>>, mut cars: Query<&mut CarDamage, With<crate::drive::Player>>) {
    if !keys.just_pressed(KeyCode::KeyR) {
        return;
    }
    for mut dmg in &mut cars {
        dmg.levels.iter_mut().for_each(|l| *l = 0.0);
        dmg.worst_impact = None;
        dmg.dirty = true;
    }
}

/// Re-pose the skeleton after damage changed.
fn pose_joints(mut cars: Query<&mut CarDamage>, mut transforms: Query<&mut Transform>) {
    for mut dmg in &mut cars {
        if !dmg.dirty {
            continue;
        }
        dmg.dirty = false;
        let parts: Vec<(&mw_formats::Animation, f32)> = match &dmg.model.animations {
            Some(a) => a.clips.iter().map(|c| &c.animation).zip(dmg.parts.iter().copied()).collect(),
            None => Vec::new(),
        };
        let pose = dmg.model.pose_with(&dmg.levels, &parts);
        for (joint, bone) in dmg.joints.iter().zip(&pose) {
            if let Ok(mut t) = transforms.get_mut(*joint) {
                *t = similarity_transform(bone);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// `--test crash`: drive into a wall on the test track, screenshot before and after.

#[derive(Resource)]
pub struct CrashTest {
    /// Screenshot path; `-before` / `-after` are appended to the file stem.
    out: Option<PathBuf>,
    /// Wall yaw (degrees): 0 = square to the car, positive turns the wall so the car's left front
    /// corner hits first.
    angle: f32,
    /// Distance from the spawn point to the wall face (m): sets the impact speed.
    distance: f32,
    /// Wall width (m).
    width: f32,
    phase: Phase,
    timer: f32,
    top_speed: f32,
    /// Crash camera screenshots taken (`crash.rs`), and the one after the reset.
    crash_shots: usize,
    reset_shot: bool,
}

/// Crash-camera screenshots: sequence time (real seconds after the impact) and file suffix. The first is
/// the gameplay camera slowing down, then one per bystander shot, then the chase camera after the reset.
const CRASH_SHOTS: [(f32, &str); 3] =
    [(0.3, "crashcam-0-slowmo"), (1.25, "crashcam-1-freeway"), (3.25, "crashcam-2-car")];
const RESET_SHOT: f32 = 5.0;

impl CrashTest {
    pub fn new(out: Option<PathBuf>, angle: f32, distance: f32, width: f32) -> Self {
        Self {
            out,
            angle,
            distance,
            width,
            phase: Phase::Settle,
            timer: 0.0,
            top_speed: 0.0,
            crash_shots: 0,
            reset_shot: false,
        }
    }

    fn path(&self, suffix: &str) -> Option<PathBuf> {
        let out = self.out.as_ref()?;
        let stem = out.file_stem()?.to_string_lossy();
        let ext = out.extension().map_or("png".into(), |e| e.to_string_lossy());
        Some(out.with_file_name(format!("{stem}-{suffix}.{ext}")))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// Car spawned; let it settle and the environment prefilter finish, then shoot "before".
    Settle,
    Drive,
    /// Hit something: brake and wait for the car to come to rest.
    Crashed,
    Done,
}

fn spawn_crash_wall(
    mut commands: Commands,
    test: Res<CrashTest>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let size = Vec3::new(test.width, 3.0, 2.0);
    let rot = Quat::from_rotation_y(test.angle.to_radians());
    let centre = Vec3::new(0.0, size.y / 2.0, test.distance + size.z / 2.0);
    commands.spawn((
        RigidBody::Static,
        Collider::cuboid(size.x, size.y, size.z),
        Transform::from_translation(centre).with_rotation(rot),
        Mesh3d(meshes.add(Cuboid::from_size(size))),
        MeshMaterial3d(materials.add(StandardMaterial { base_color: Color::srgb(0.6, 0.55, 0.5), ..default() })),
    ));
}

fn run_crash_test(
    mut commands: Commands,
    time: Res<Time>,
    mut test: ResMut<CrashTest>,
    mut input: ResMut<CarInput>,
    cars: Query<(&Car, &CarDamage, &Transform), With<crate::drive::Player>>,
    crash: Res<crate::crash::CrashState>,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok((car, dmg, transform)) = cars.single() else { return };
    test.timer += time.delta_secs();
    test.top_speed = test.top_speed.max(car.speed.abs());
    let damaged = dmg.levels.iter().any(|l| *l > 0.0);
    match test.phase {
        Phase::Settle => {
            input.scripted = Some([0.0, 0.0, 0.0, 1.0]);
            if test.timer > 2.5 {
                if let Some(p) = test.path("before") {
                    commands.spawn(Screenshot::primary_window()).observe(save_to_disk(p));
                }
                test.phase = Phase::Drive;
                test.timer = 0.0;
            }
        }
        Phase::Drive => {
            input.scripted = Some([1.0, 0.0, 0.0, 0.0]);
            if damaged || crash.active.is_some() {
                info!(
                    "TEST crash: hit at {:.0} km/h (top speed {:.0} km/h)",
                    car.speed.abs() * 3.6,
                    test.top_speed * 3.6
                );
                test.phase = Phase::Crashed;
                test.timer = 0.0;
            } else if test.timer > 30.0 {
                info!("TEST crash: no impact after 30 s (car at {:?})", transform.translation);
                test.phase = Phase::Done;
            }
        }
        Phase::Crashed => {
            input.scripted = Some([0.0, 0.0, 0.0, 1.0]);
            if let Some(active) = &crash.active {
                // The crash camera: screenshots during the sequence and of the chase camera after the reset.
                if let Some(&(at, name)) = CRASH_SHOTS.get(test.crash_shots)
                    && active.t >= at
                {
                    info!(
                        "TEST crash: crash camera at {:.2} s ({:.0} mph impact) -> {name}",
                        active.t, active.closing_mph
                    );
                    if let Some(p) = test.path(name) {
                        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(p));
                    }
                    test.crash_shots += 1;
                }
                if !test.reset_shot && active.reset_done && active.t >= RESET_SHOT {
                    info!("TEST crash: after the reset at {:.2} s", active.t);
                    if let Some(p) = test.path("crashcam-3-reset") {
                        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(p));
                    }
                    test.reset_shot = true;
                }
                test.timer = 0.0;
                return;
            }
            if test.timer > 3.0 && car.speed.abs() < 0.5 {
                let (speed, point) = dmg.worst_impact.unwrap_or_default();
                info!(
                    "TEST crash: worst impact {:.0} km/h at car ({:.2} {:.2} {:.2}); damage {}",
                    speed * 3.6,
                    point.x,
                    point.y,
                    point.z,
                    dmg.zone_levels().map(|(n, l)| format!("{n} {:.0}%", l * 100.0)).collect::<Vec<_>>().join(", ")
                );
                if let Some(p) = test.path("after") {
                    commands.spawn(Screenshot::primary_window()).observe(save_to_disk(p));
                }
                test.phase = Phase::Done;
                test.timer = 0.0;
            }
        }
        Phase::Done => {
            if test.timer > 1.0 {
                exit.write(AppExit::Success);
            }
        }
    }
}

/// Look at the car's nose (where the damage is) instead of following it.
fn crash_camera(
    test: Res<CrashTest>,
    crash: Res<crate::crash::CrashState>,
    cars: Query<&GlobalTransform, (With<CarDamage>, With<crate::drive::Player>)>,
    mut camera: Query<&mut Transform, crate::MainCamera>,
) {
    // The crash camera has the view while its sequence runs.
    if crash.active.is_some() {
        return;
    }
    let (Ok(car), Ok(mut cam)) = (cars.single(), camera.single_mut()) else { return };
    let (target, eye) = if test.angle == 0.0 {
        // Head-on: the car stays at the wall, so look from the left side, behind the wall plane.
        (Vec3::new(0.0, 0.5, 1.5), Vec3::new(3.4, 1.6, -0.4))
    } else {
        // Glancing: the car bounces away; look at the front-left corner.
        (Vec3::new(0.4, 0.5, 1.6), Vec3::new(3.0, 1.5, 4.4))
    };
    *cam = Transform::from_translation(car.transform_point(eye)).looking_at(car.transform_point(target), Vec3::Y);
}
