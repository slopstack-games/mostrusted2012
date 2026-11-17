//! The cars' collision effects: sparks, glass, smoke and body debris when a car hits another car, a wall or the
//! ground, as the original plays them (docs/decomp.md "Car collision effects").
//!
//! Every frame, for each race car (`FUN_00584260`, the player and AI racers) and each traffic or parked car
//! (`FUN_00600220`), when the car moves faster than `VehicleImpacts.34429d6c` (1 m/s):
//! - its **vehicle contacts** (the object contact list, 0x70-byte records) in order: a contact whose value is
//!   above `VehicleCollisionSettings.e9ff2375` (5) once `19458ef1` (0.2 s) have passed since the car's last
//!   collision effect is an impact (`VfxGlobals.VehicleImpacts`, a new instance), anything else a scrape
//!   (`VehicleScrapes`). The effect is one of the 36 `VfxVehicleCollisions` handles: by the other car (a race car
//!   `FUN_004a02b0`, traffic / parked `FUN_004a0600`; a traffic car always uses the traffic half), whether this is
//!   the local player (`FUN_006a10d0`; for a traffic car: whether the other car is the local player) and the
//!   impact zone of the contact on this car (`FUN_00929da0`, 3 front, 2 / 4 front corners, 9 / 5 sides, 7 rear,
//!   8 / 6 rear corners; roof, underside and centre play nothing). The first contact with an effect wins.
//! - otherwise its **world contacts**: the `SurfaceVfx` of the contact's surface (dry block), the impact effect
//!   of the zone (`FUN_0049fe80`: the named field for the local player, the unnamed one after it otherwise; the
//!   traffic version `FUN_005f58b0`), a new instance after 0.2 s, else the same one refreshed.
//!
//! Up to four instances per car (+0x370 effect, +0x390 frames since refreshed, +0x28c instance): the same effect
//! keeps its instance (refreshed, `IsEmitting` on for the frame), a new impact or another effect replaces the
//! slot holding it, an empty one or the one refreshed longest ago. Every frame each live instance gets the car's
//! matrix as `Transform` and its `IsEmitting` bit cleared; a refresh sets it again. The effects emit from the
//! car's collision mesh (`EmitterCollisionMesh`, `particles::mesh`) around the contact, in car space, with
//! `Velocity` = the body's velocity at the contact and `ImpactClosingVelocity` = the contact value.
//!
//! Model decisions: the contact value (the game's record +0x2c, unit not traced; its effects' controllers scale
//! it by 22.35 = 50 mph in m/s) is the contact's closing speed (m/s) before the solve, the same stand-in as the
//! drift exits and the camera shake; the contact normal points from the car towards what it hit; a step's
//! contacts are gathered per frame (the largest value per other body); `MovementSinceLastFrame` = the car's
//! speed × the frame time; traffic cars' ground plane is level through their origin; the dry surface block.
//!
//! `MW_CRASH_VFX_LOG=1` logs every effect started with its contact value (`=2` also every refresh).

use std::collections::HashMap;
use std::sync::{Mutex, mpsc};

use avian3d::prelude::*;
use bevy::prelude::*;
use mw_bundle::ResourceId;

use crate::drive::{Car, Player};
use crate::particles::{CollisionMeshes, EmitterKey, Instances, MeshContact, Params, SLOT_CRASH, Vfx};
use crate::traffic::{TrafficCar, TrafficSystem};

/// Instances a car keeps (`FUN_00584260`'s loops over +0x370 / +0x390).
const SLOTS: usize = 4;

/// `MW_CRASH_VFX_LOG`: 1 logs every effect started, 2 also every refresh.
fn log_level() -> u8 {
    static ON: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("MW_CRASH_VFX_LOG").ok().and_then(|v| v.parse().ok()).unwrap_or(0))
}

fn log_on() -> bool {
    log_level() > 0
}

pub struct CrashVfxPlugin;

impl Plugin for CrashVfxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FrameContacts>()
            .init_resource::<CarMeshes>()
            .init_resource::<CrashVfxStats>()
            .add_systems(
                FixedPostUpdate,
                collect_contacts.after(PhysicsSystems::StepSimulation).before(crate::car_impacts::EvaluateImpacts),
            )
            .add_systems(
                Update,
                (load_meshes, play_effects).chain().before(crate::particles::simulate).run_if(resource_exists::<Vfx>),
            )
            .add_systems(Last, report_stats);
    }
}

/// The vehicle bundle of a race car, for its collision mesh (inserted when the car is spawned, `drive.rs`).
#[derive(Component)]
pub struct CrashVfxCar {
    pub bundle: String,
    /// Its paint (`VehiclePaint` objects in ID order), for painted debris.
    pub paint_index: usize,
}

/// Counts of started effects (printed when the app exits, `CRASH VFX` line).
#[derive(Resource, Default, Debug)]
pub struct CrashVfxStats {
    pub impacts: u32,
    pub scrapes: u32,
    pub surface: u32,
    pub refreshed: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum OtherKind {
    RaceCar { player: bool },
    Traffic,
    World,
}

/// One contact of a car this frame.
#[derive(Clone, Copy, Debug)]
struct Contact {
    other: Option<Entity>,
    kind: OtherKind,
    /// The world collider (for the surface lookup).
    collider: Entity,
    point: Vec3,
    /// From the car towards what it touches.
    normal: Vec3,
    /// Closing speed before the solve (m/s).
    value: f32,
}

/// Contacts of the cars since the last frame, per car: the largest value per other body.
#[derive(Resource, Default)]
struct FrameContacts(HashMap<Entity, Vec<Contact>>);

impl FrameContacts {
    fn add(&mut self, car: Entity, c: Contact) {
        let list = self.0.entry(car).or_default();
        match list.iter_mut().find(|o| o.other == c.other && (c.other.is_some() || o.collider == c.collider)) {
            Some(o) if o.value >= c.value => {}
            Some(o) => *o = c,
            None => list.push(c),
        }
    }
}

/// Gather the car bodies' contacts after each physics step (and the first contacts with lane followers, which
/// the contact hook drops: `car_impacts::PendingImpacts`).
#[allow(clippy::type_complexity)]
fn collect_contacts(
    collisions: Collisions,
    mut frame: ResMut<FrameContacts>,
    race: Query<Has<Player>, With<Car>>,
    traffic: Query<(), With<TrafficCar>>,
    props: Query<(), With<crate::props::PropBody>>,
    pending: Res<crate::car_impacts::PendingImpacts>,
) {
    let kind = |e: Option<Entity>| -> Option<OtherKind> {
        let Some(e) = e else { return Some(OtherKind::World) };
        if let Ok(player) = race.get(e) {
            Some(OtherKind::RaceCar { player })
        } else if traffic.contains(e) {
            Some(OtherKind::Traffic)
        } else if props.contains(e) {
            None
        } else {
            Some(OtherKind::World)
        }
    };
    let is_car = |e: Option<Entity>| e.is_some_and(|e| race.contains(e) || traffic.contains(e));
    for pair in collisions.iter() {
        if !pair.is_touching() {
            continue;
        }
        for (car, other, collider, sign) in
            [(pair.body1, pair.body2, pair.collider2, 1.0), (pair.body2, pair.body1, pair.collider1, -1.0)]
        {
            if !is_car(car) {
                continue;
            }
            let Some(kind) = kind(other) else { continue };
            for m in &pair.manifolds {
                if m.total_normal_impulse() <= 0.0 {
                    continue;
                }
                let Some(point) = m.points.iter().min_by(|a, b| a.normal_speed.total_cmp(&b.normal_speed)) else {
                    continue;
                };
                // The manifold normal points from collider 1 to collider 2.
                let normal = (m.normal * sign).normalize_or_zero();
                frame.add(
                    car.expect("checked"),
                    Contact {
                        other,
                        kind,
                        collider,
                        point: point.point,
                        normal,
                        value: (-point.normal_speed).max(0.0),
                    },
                );
            }
        }
    }
    for imp in &pending.0 {
        let value = (imp.va - imp.vb).dot(imp.normal).max(0.0);
        for (car, other, normal) in [(imp.a, imp.b, imp.normal), (imp.b, imp.a, -imp.normal)] {
            if !is_car(Some(car)) {
                continue;
            }
            let Some(kind) = kind(Some(other)) else { continue };
            frame.add(car, Contact { other: Some(other), kind, collider: other, point: imp.point, normal, value });
        }
    }
}

type MeshArrival = (String, Option<Vec<[Vec3; 3]>>, Vec<Vec3>);

/// The cars' collision meshes by bundle: loaded on a worker thread, then indices into [`CollisionMeshes`].
#[derive(Resource, Default)]
struct CarMeshes {
    by_bundle: HashMap<String, Option<u32>>,
    /// The race cars' paint colours (diffuse, linear) by bundle.
    paints: HashMap<String, Vec<Vec3>>,
    pending: Option<Mutex<mpsc::Receiver<MeshArrival>>>,
    want: Vec<String>,
}

impl CarMeshes {
    /// The mesh of a bundle once loaded (asks for it the first time).
    fn get(&mut self, bundle: &str) -> Option<u32> {
        match self.by_bundle.get(bundle) {
            Some(m) => *m,
            None => {
                self.by_bundle.insert(bundle.to_string(), None);
                self.want.push(bundle.to_string());
                None
            }
        }
    }
}

fn load_meshes(game: Res<crate::Game>, mut loads: ResMut<CarMeshes>, mut meshes: ResMut<CollisionMeshes>) {
    if let Some(rx) = loads.pending.take() {
        let mut got = Vec::new();
        let done = loop {
            match rx.lock().unwrap().try_recv() {
                Ok(m) => got.push(m),
                Err(mpsc::TryRecvError::Empty) => break false,
                Err(mpsc::TryRecvError::Disconnected) => break true,
            }
        };
        for (bundle, tris, paints) in got {
            loads.paints.insert(bundle.clone(), paints);
            let id = tris.map(|t| meshes.add(t));
            if log_on() {
                info!("crash vfx: collision mesh of {bundle}: {id:?}");
            }
            loads.by_bundle.insert(bundle, id);
        }
        if !done {
            loads.pending = Some(rx);
            return;
        }
    }
    if loads.want.is_empty() {
        return;
    }
    let want = std::mem::take(&mut loads.want);
    let store = game.store.clone();
    let (tx, rx) = mpsc::channel();
    crate::platform::thread::Builder::new()
        .name("collision meshes".into())
        .spawn(move || {
            for bundle in want {
                let tris = match store.vehicle_collision_mesh(&bundle) {
                    Ok(t) => t.map(|t| t.into_iter().map(|v| v.map(Vec3::from)).collect()),
                    Err(e) => {
                        warn!("collision mesh of {bundle}: {e}");
                        None
                    }
                };
                let paints = store
                    .bundle(&bundle)
                    .and_then(|b| store.vehicle_paints(&b).ok())
                    .map(|p| p.iter().map(|p| Vec3::from(p.diffuse)).collect())
                    .unwrap_or_default();
                let _ = tx.send((bundle, tris, paints));
            }
        })
        .expect("spawn collision mesh loader");
    loads.pending = Some(Mutex::new(rx));
}

/// A car's collision-effect state (+0x3f0 the time since the last effect, the four instance slots).
#[derive(Default)]
struct CarState {
    since: f32,
    slots: [Option<(ResourceId, u32)>; SLOTS],
}

/// One car this frame.
struct Owner {
    entity: Entity,
    traffic: bool,
    player: bool,
    world: Mat4,
    rotation: Quat,
    position: Vec3,
    velocity: Vec3,
    omega: Vec3,
    centre_of_mass: Vec3,
    /// The physical box (car space).
    box_centre: Vec3,
    half: Vec3,
    ground: Vec4,
    mesh: Option<u32>,
    paint: Vec3,
}

/// `FUN_004a02b0` (another race car) / `FUN_004a0600` (traffic): the `VfxVehicleCollisions` field for a zone.
/// Fields 0, 1, 18 and 19 are never read.
fn vehicle_field(zone: u8, traffic_other: bool, player: bool) -> Option<usize> {
    let k = match zone {
        3 => 0,
        2 => 1,
        4 => 2,
        9 => 3,
        7 => 4,
        8 => 5,
        6 => 6,
        5 => 7,
        _ => return None,
    };
    let base = match (player, traffic_other) {
        (true, false) => 2,
        (true, true) => 10,
        (false, false) => 20,
        (false, true) => 28,
    };
    Some(base + k)
}

/// `FUN_0049fe80`: the `SurfaceVfx` impact effect for a zone (index into `SurfaceEffects::impacts`).
fn surface_field(zone: u8) -> Option<usize> {
    Some(match zone {
        0 => 8,
        1 => 9,
        2 => 1,
        3 => 0,
        4 => 2,
        5 => 7,
        6 => 5,
        7 => 3,
        8 => 4,
        9 => 6,
        _ => return None,
    })
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn play_effects(
    time: Res<Time>,
    vfx: Res<Vfx>,
    mut instances: ResMut<Instances>,
    mut frame: ResMut<FrameContacts>,
    mut loads: ResMut<CarMeshes>,
    mut stats: ResMut<CrashVfxStats>,
    surfaces: Res<crate::props::WorldSurfaces>,
    tags: Query<(&Collider, &crate::props::SurfaceTags)>,
    race: Query<(
        Entity,
        &Car,
        &GlobalTransform,
        &LinearVelocity,
        &AngularVelocity,
        Option<&CenterOfMass>,
        Has<Player>,
        Option<&CrashVfxCar>,
    )>,
    traffic_cars: Query<(
        Entity,
        &TrafficCar,
        &GlobalTransform,
        &LinearVelocity,
        Option<&AngularVelocity>,
        Option<&CenterOfMass>,
    )>,
    traffic: Option<Res<TrafficSystem>>,
    mut states: Local<HashMap<Entity, CarState>>,
) {
    let dt = time.delta_secs();
    let contacts = std::mem::take(&mut frame.0);
    let lib = vfx.lib();
    let settings = lib.collisions;
    let mut owners = Vec::new();
    for (entity, car, t, v, w, com, player, src) in &race {
        let ground = {
            let c: Vec<_> = car.wheel_contacts().flatten().collect();
            if c.is_empty() {
                Vec4::new(0.0, 1.0, 0.0, f32::MAX)
            } else {
                let n = c.iter().map(|c| c.normal).sum::<Vec3>().normalize_or(Vec3::Y);
                let p = c.iter().map(|c| c.point).sum::<Vec3>() / c.len() as f32;
                n.extend(-n.dot(p))
            }
        };
        let (_, rotation, position) = t.to_scale_rotation_translation();
        owners.push(Owner {
            entity,
            traffic: false,
            player,
            world: t.to_matrix(),
            rotation,
            position,
            velocity: v.0,
            omega: w.0,
            centre_of_mass: com.map_or(Vec3::ZERO, |c| c.0),
            box_centre: car.params.centre_of_mass,
            half: car.params.body_half_extents,
            ground,
            mesh: src.and_then(|s| loads.get(&s.bundle)),
            paint: src
                .and_then(|s| loads.paints.get(&s.bundle)?.get(s.paint_index).copied())
                .unwrap_or(Vec3::splat(0.5)),
        });
    }
    if let Some(sys) = traffic.as_deref() {
        for (entity, tc, t, v, w, com) in &traffic_cars {
            let Some(vt) = sys.vehicles().get(tc.vehicle) else { continue };
            let (_, rotation, position) = t.to_scale_rotation_translation();
            owners.push(Owner {
                entity,
                traffic: true,
                player: false,
                world: t.to_matrix(),
                rotation,
                position,
                velocity: v.0,
                omega: w.map_or(Vec3::ZERO, |w| w.0),
                centre_of_mass: com.map_or(Vec3::ZERO, |c| c.0),
                box_centre: Vec3::from(vt.box_centre),
                half: Vec3::from(vt.half_extents),
                ground: Vec4::new(0.0, 1.0, 0.0, -position.y),
                mesh: loads.get(&vt.bundle),
                paint: vt.paints.get(tc.paint).map_or(Vec3::splat(0.5), |(_, p)| Vec3::from(p.diffuse)),
            });
        }
    }
    states.retain(|e, _| owners.iter().any(|o| o.entity == *e));
    let default_surface = lib.surfaces.first();
    for o in &owners {
        let state = states.entry(o.entity).or_default();
        state.since += dt;
        let key = |i: usize| EmitterKey { owner: o.entity, slot: SLOT_CRASH + i as u32 };
        // The slots: the frames since each was refreshed; dead instances free theirs.
        for (i, slot) in state.slots.iter_mut().enumerate() {
            match instances.keyed_mut(key(i)) {
                Some(params) => {
                    params.transform = o.world;
                    params.velocity = o.velocity;
                    if let Some(s) = slot.as_mut() {
                        s.1 = s.1.saturating_add(1);
                    }
                }
                None => *slot = None,
            }
        }
        if o.velocity.length() <= settings.min_speed {
            continue;
        }
        let may_restart = state.since > settings.retrigger;
        let to_local = |p: Vec3| o.rotation.inverse() * (p - o.position);
        let list = contacts.get(&o.entity).map(Vec::as_slice).unwrap_or_default();
        // Vehicle contacts first, then the world's.
        let mut chosen: Option<(ResourceId, Contact, bool, &str, u8)> = None;
        for c in list.iter().filter(|c| c.kind != OtherKind::World) {
            let impact = may_restart && c.value > settings.impact_value;
            let set = if impact { &lib.vehicle_impacts } else { &lib.vehicle_scrapes };
            let zone = crate::car_impacts::impact_zone(to_local(c.point), o.box_centre, o.half);
            let (traffic_other, player) = match (o.traffic, c.kind) {
                (true, OtherKind::RaceCar { player }) => (true, player),
                (true, _) => (true, false),
                (false, OtherKind::Traffic) => (true, o.player),
                (false, _) => (false, o.player),
            };
            if let Some(effect) = vehicle_field(zone, traffic_other, player).and_then(|f| set.get(f).copied().flatten())
            {
                chosen = Some((effect, *c, impact, if impact { "impact" } else { "scrape" }, zone));
                break;
            }
        }
        if chosen.is_none() {
            for c in list.iter().filter(|c| c.kind == OtherKind::World) {
                let zone = crate::car_impacts::impact_zone(to_local(c.point), o.box_centre, o.half);
                let surface = tags
                    .get(c.collider)
                    .ok()
                    .and_then(|(col, t)| surfaces.surface_at(col, t, c.point - c.normal * 0.5, c.normal, 1.0))
                    .and_then(|h| lib.surfaces.get(h.tag.surface as usize))
                    .or(default_surface);
                let Some(fx) = surface.map(|s| &s.dry) else { continue };
                let table = if o.player { &fx.impacts } else { &fx.ai_impacts };
                if let Some(effect) = surface_field(zone).and_then(|f| table[f]) {
                    chosen = Some((effect, *c, may_restart, "surface", zone));
                    break;
                }
            }
        }
        let Some((effect, c, new, set, zone)) = chosen else { continue };
        let Some(mesh) = o.mesh else { continue };
        let params = Params {
            transform: o.world,
            velocity: o.velocity + o.omega.cross(c.point - (o.position + o.rotation * o.centre_of_mass)),
            intensity: 1.0,
            emitting: true,
            moved: o.velocity.length() * dt,
            ground: o.ground,
            impact: c.value,
            movement: o.velocity.length() * dt,
            mesh: Some(MeshContact { mesh, point: to_local(c.point), normal: o.rotation.inverse() * c.normal }),
            paint: o.paint,
            wheel: [0.0; 3],
        };
        // The slot already playing this effect, else an empty one, else the one refreshed longest ago.
        let i = state
            .slots
            .iter()
            .position(|s| s.is_some_and(|s| s.0 == effect))
            .or_else(|| state.slots.iter().position(Option::is_none))
            .unwrap_or_else(|| (0..SLOTS).max_by_key(|&i| state.slots[i].map_or(u32::MAX, |s| s.1)).unwrap_or(0));
        let same = state.slots[i].is_some_and(|s| s.0 == effect);
        let refreshed = same && !new && instances.keyed_mut(key(i)).map(|p| *p = params).is_some();
        if !refreshed {
            if !instances.restart(&vfx, key(i), effect, params) {
                continue;
            }
            match set {
                "impact" => stats.impacts += 1,
                "scrape" => stats.scrapes += 1,
                _ => stats.surface += 1,
            }
        } else {
            stats.refreshed += 1;
        }
        state.slots[i] = Some((effect, 0));
        state.since = 0.0;
        if log_level() > u8::from(refreshed) {
            info!(
                "crash vfx: {} {:?} {} {set} effect {effect} slot {i} vs {:?}, value {:.1} m/s, zone {zone} at {:.2} (car {:.2})",
                if o.traffic {
                    "traffic"
                } else if o.player {
                    "player"
                } else {
                    "AI"
                },
                o.entity,
                if refreshed { "refresh" } else { "start" },
                c.kind,
                c.value,
                c.point,
                to_local(c.point),
            );
        }
    }
}

fn report_stats(mut exit: MessageReader<AppExit>, stats: Res<CrashVfxStats>) {
    if exit.read().next().is_some() && stats.impacts + stats.scrapes + stats.surface > 0 {
        info!(
            "CRASH VFX: {} vehicle impacts, {} vehicle scrapes, {} surface effects started, {} refreshes",
            stats.impacts, stats.scrapes, stats.surface, stats.refreshed
        );
    }
}
