//! What compound objects (street lamps, traffic lights, billboards), their parts and dynamic objects show when
//! they are hit: their `WCVfxBehaviour`s (`mw_assets::props::WcVfx`; docs/decomp.md "Props", "World-component
//! effects"). Plain props' impact effects are `particles::prop_impact_effects`.
//!
//! Every frame the game tells each object's behaviours about the contacts of its body (`FUN_0053cba0`):
//! - each **body-pair contact** (`FUN_005461c0` → the behaviour's `FUN_005af3e0`), only while the body is not
//!   pinned: the behaviour's `ImpactEffect` starts once (when the other body is a vehicle; the flag is spent on
//!   the first contact either way), half a metre above the contact point, Z along the vehicle's velocity
//!   (`FUN_005a7750`); a behaviour flagged `0228a2f8` switches on (`FUN_005b30d0`: its spot effects start at
//!   their locators, `FUN_005af900`; lights and coronas on), one flagged `713589ad` switches off
//!   (`FUN_005b3150`: lights and coronas off). A street lamp knocked loose by a car thus throws sparks (`…24f3d`)
//!   where it was hit, bursts at its foot (`…7a95f`, the "StreetlampSingle_WC VFX" behaviour) and its light
//!   goes out ("SetLightsOn", off by contact);
//! - the **world contact** (`FUN_005462d0` → `FUN_005af630`): when the contact point moves faster than the
//!   behaviour's `29436f61` (1 m/s), its contact effect (`4524a2be`, the lamp parts' sparks `…1621e7`) starts at
//!   the point with Z along the contact normal, again whenever the last one has ended.
//!
//! Model decisions (marked where they are made): which hitters count as vehicles (the race cars; the game asks
//! its vehicle table, `FUN_00573360`); the effects' `Intensity` (the w of a velocity the game passes, not traced:
//! the props' closing-speed rule for impacts, 1 for spot effects); the spot effects' `Velocity` (the object
//! body's); the point velocity of a world contact taken about the body's centre of mass.

use avian3d::prelude::*;
use bevy::prelude::*;
use mw_bundle::ResourceId;

use crate::drive::Car;
use crate::particles::{EffectRequests, EmitterKey, Instances, Params, Vfx};
use crate::props::{PropBody, PropCarContact};

/// The emitter slots of contact effects: this + the behaviour's index.
const SLOT_CONTACT: u32 = 0x2000;

/// `MW_PROP_VFX_LOG=1`: log every behaviour effect, switch and light change.
fn log_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MW_PROP_VFX_LOG").is_some())
}

pub struct PropVfxPlugin;

impl Plugin for PropVfxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PropVfxQueue>()
            .init_resource::<PropVfxStats>()
            .add_systems(FixedPostUpdate, behaviour_contacts.after(PhysicsSystems::StepSimulation))
            .add_systems(Update, (switch_lights, play_queued.run_if(resource_exists::<Vfx>)));
    }
}

/// The emitter frame of an impact effect (`FUN_005a7750` for behaviours, the same maths inline in `FUN_0054bf70`
/// for props): at the contact point raised by (0, 0.5, 0) (`.rdata` 0x00fae8f0 / 0x00fab370), Z = the hitter's
/// velocity direction (`FUN_00513200`, falling back to `fallback_z`), X = normalize(up × Z) (falling back to
/// +X, `.rdata` 0x00fae840 / 0x00fae850), Y = Z × X: Y is up for a level hit, Z points along the travel.
pub(crate) fn impact_frame(point: Vec3, velocity: Vec3, fallback_z: Vec3) -> Mat4 {
    let z = velocity.try_normalize().unwrap_or(fallback_z);
    let x = Vec3::Y.cross(z).try_normalize().unwrap_or(Vec3::X);
    let y = z.cross(x);
    Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), (point + Vec3::Y * 0.5).extend(1.0))
}

/// The state of one object's behaviours (the game's runtime behaviour, `FUN_005b3350`).
#[derive(Component)]
struct BehaviourState {
    /// Per behaviour: switched on, and its impact effect still to come (+0x5e).
    on: Vec<bool>,
    impact_pending: Vec<bool>,
    /// The body has been loose since the object was (re)placed.
    loose: bool,
}

impl BehaviourState {
    fn new(prop: &PropBody) -> Self {
        Self {
            on: prop.physics.vfx.iter().map(|v| v.active_at_start).collect(),
            impact_pending: prop.physics.vfx.iter().map(|v| v.impact_effect.is_some()).collect(),
            loose: false,
        }
    }
}

/// An effect to start (`play_queued`), or a contact effect to keep up.
#[derive(Clone, Copy)]
struct Start {
    effect: ResourceId,
    params: Params,
    key: Option<EmitterKey>,
}

#[derive(Resource, Default)]
struct PropVfxQueue {
    /// Waiting for their effect to load (one-shots wait up to half a second).
    starts: Vec<(f32, Start)>,
    /// Lights to switch off (`true`) or back on, per object.
    lights: Vec<(Entity, bool)>,
}

/// Counts for tests and logs.
#[derive(Resource, Default, Debug)]
pub struct PropVfxStats {
    pub impacts: usize,
    pub spot_effects: usize,
    pub contact_effects: usize,
    pub lights_off: usize,
}

/// What a contact does to an object's behaviours (`FUN_005af3e0` / `FUN_005af630`).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn behaviour_contacts(
    mut commands: Commands,
    time: Res<Time>,
    collisions: Collisions,
    mut car_contacts: MessageReader<PropCarContact>,
    mut objects: Query<(
        Entity,
        &PropBody,
        Option<&mut BehaviourState>,
        Option<&Position>,
        Option<&Rotation>,
        Option<&LinearVelocity>,
        Option<&AngularVelocity>,
        Option<&ComputedCenterOfMass>,
    )>,
    bodies: Query<(), Or<(With<PropBody>, With<Car>, With<crate::traffic::TrafficBody>)>>,
    mut queue: ResMut<PropVfxQueue>,
    mut stats: ResMut<PropVfxStats>,
) {
    let now = time.elapsed_secs();
    let mut by_car: Vec<PropCarContact> = car_contacts.read().copied().collect();
    by_car.sort_by_key(|c| c.prop);
    for (e, prop, state, pos, rot, v, w, com) in &mut objects {
        if prop.physics.vfx.is_empty() {
            continue;
        }
        let Some(mut state) = state else {
            commands.entity(e).insert(BehaviourState::new(prop));
            if log_on() && prop.physics.compound.is_some() {
                info!("prop vfx: {} placed at {:.1}", prop.physics.object, prop.home.translation);
            }
            continue;
        };
        if prop.pinned {
            // Put back (`FUN_0055ae50`; ResetProps re-creates the behaviours): as at the start.
            if state.loose {
                *state = BehaviourState::new(prop);
                queue.lights.push((e, true));
            }
            continue;
        }
        state.loose = true;
        let (Some(pos), Some(rot)) = (pos, rot) else { continue };
        let world = Mat4::from_rotation_translation(rot.0, pos.0);
        let v = v.map_or(Vec3::ZERO, |v| v.0);
        let w = w.map_or(Vec3::ZERO, |w| w.0);
        let centre = pos.0 + rot.0 * com.map_or(Vec3::ZERO, |c| c.0);

        // Body-pair contacts: the race cars' (`prop_contacts`), then avian's with other dynamic bodies.
        struct Pair {
            point: Vec3,
            vehicle_velocity: Option<Vec3>,
            closing: f32,
        }
        let mut pairs: Vec<Pair> = by_car
            .iter()
            .filter(|c| c.prop == e)
            .map(|c| Pair { point: c.point, vehicle_velocity: Some(c.car_velocity), closing: c.closing_speed })
            .collect();
        let mut world_contact: Option<(Vec3, Vec3)> = None;
        for p in collisions.collisions_with(e) {
            if !p.is_touching() || p.manifolds.is_empty() {
                continue;
            }
            let first = p.body1 == Some(e) || p.collider1 == e;
            let other = if first { p.body2.unwrap_or(p.collider2) } else { p.body1.unwrap_or(p.collider1) };
            let Some((m, point)) = p.manifolds.iter().find_map(|m| m.points.first().map(|pt| (m, pt.point))) else {
                continue;
            };
            if bodies.contains(other) {
                // Race cars are handled above (they meet loose props only through `prop_contacts`).
                pairs.push(Pair { point, vehicle_velocity: None, closing: 0.0 });
            } else if world_contact.is_none() {
                // Out of the world, towards the object.
                let normal = if first { -m.normal } else { m.normal };
                world_contact = Some((point, normal));
            }
        }

        for pair in &pairs {
            for (i, b) in prop.physics.vfx.iter().enumerate() {
                if std::mem::take(&mut state.impact_pending[i])
                    && let (Some(effect), Some(hv)) = (b.impact_effect, pair.vehicle_velocity)
                {
                    // Model decision: `Intensity` (the w of the vehicle's velocity, not traced) as for props.
                    let frame = impact_frame(pair.point, hv, world.z_axis.truncate());
                    queue.starts.push((
                        now,
                        Start {
                            effect,
                            params: Params {
                                transform: frame,
                                velocity: hv,
                                intensity: (pair.closing / 15.0).clamp(0.0, 1.0),
                                ground: Vec4::new(0.0, 1.0, 0.0, -pair.point.y),
                                impact: pair.closing,
                                ..default()
                            },
                            key: None,
                        },
                    ));
                    stats.impacts += 1;
                    if log_on() {
                        info!(
                            "prop vfx: {} {:?} impact effect {effect} at {:.2}, hitter {:.1} m/s",
                            prop.physics.object,
                            b.label,
                            pair.point,
                            hv.length()
                        );
                    }
                }
                if b.activate_on_collision && !state.on[i] {
                    state.on[i] = true;
                    if log_on() {
                        info!(
                            "prop vfx: {} {:?} on ({} spot effects)",
                            prop.physics.object,
                            b.label,
                            b.spot_effects.len()
                        );
                    }
                    // `FUN_005af900`: one effect per locator, at locator × object.
                    for spot in &b.spot_effects {
                        let Some(effect) = spot.effect else { continue };
                        for &(q, p) in &spot.locators {
                            // The game's locator quaternion rotates row vectors as glam's does column vectors.
                            let local = Mat4::from_rotation_translation(Quat::from_array(q).normalize(), Vec3::from(p));
                            let at = world * local;
                            queue.starts.push((
                                now,
                                Start {
                                    effect,
                                    params: Params {
                                        transform: at,
                                        velocity: v,
                                        intensity: 1.0,
                                        ground: Vec4::new(0.0, 1.0, 0.0, -prop.home.translation.y),
                                        ..default()
                                    },
                                    key: None,
                                },
                            ));
                            stats.spot_effects += 1;
                        }
                    }
                }
                if b.deactivate_on_collision && state.on[i] {
                    state.on[i] = false;
                    if log_on() {
                        info!("prop vfx: {} {:?} off ({} lights)", prop.physics.object, b.label, b.lights);
                    }
                    if b.lights > 0 {
                        queue.lights.push((e, false));
                        stats.lights_off += 1;
                    }
                }
            }
        }

        // The world contact: contact effects while the point slides fast enough.
        if let Some((point, normal)) = world_contact {
            let point_velocity = v + w.cross(point - centre);
            for (i, b) in prop.physics.vfx.iter().enumerate() {
                let Some(effect) = b.contact_effect else { continue };
                if point_velocity.length() <= b.contact_speed {
                    continue;
                }
                // Z along the contact normal, X = normalize(+X × Z) (`.rdata` 0x00fae840), Y = Z × X.
                let z = normal.try_normalize().unwrap_or(Vec3::Y);
                let x = Vec3::X.cross(z).try_normalize().unwrap_or(Vec3::Y.cross(z).normalize_or(Vec3::X));
                let frame = Mat4::from_cols(x.extend(0.0), z.cross(x).extend(0.0), z.extend(0.0), point.extend(1.0));
                queue.starts.push((
                    now,
                    Start {
                        effect,
                        params: Params {
                            transform: frame,
                            velocity: point_velocity,
                            intensity: 1.0,
                            emitting: true,
                            ground: Vec4::new(0.0, 1.0, 0.0, -point.y),
                            ..default()
                        },
                        key: Some(EmitterKey { owner: e, slot: SLOT_CONTACT + i as u32 }),
                    },
                ));
                stats.contact_effects += 1;
                if log_on() {
                    info!(
                        "prop vfx: {} {:?} contact effect {effect} at {point:.2}, point speed {:.1} m/s",
                        prop.physics.object,
                        b.label,
                        point_velocity.length()
                    );
                }
            }
        }
    }
}

/// Start the queued effects (loading them first).
fn play_queued(
    time: Res<Time>,
    vfx: Res<Vfx>,
    mut requests: ResMut<EffectRequests>,
    mut instances: ResMut<Instances>,
    mut queue: ResMut<PropVfxQueue>,
) {
    let now = time.elapsed_secs();
    queue.starts.retain(|(t, s)| {
        if !vfx.has_effect(s.effect) {
            requests.request(s.effect);
            // A contact effect is asked for again by the next contact; a one-shot waits a little.
            return s.key.is_none() && now - t < 0.5;
        }
        match s.key {
            Some(key) => instances.emit(&vfx, key, Some(s.effect), s.params),
            None => instances.one_shot(&vfx, Some(s.effect), s.params),
        }
        false
    });
}

/// A light-carrying behaviour switched off by a contact: the object's lights go out (`FUN_005b3150` →
/// `FUN_005327b0`) until the object is put back.
fn switch_lights(
    mut queue: ResMut<PropVfxQueue>,
    props: Query<&PropBody>,
    lights: Option<ResMut<crate::world_lights::LightSources>>,
    mut off: Local<std::collections::HashMap<Entity, Vec<mw_assets::lights::WorldLight>>>,
) {
    let Some(mut lights) = lights else {
        queue.lights.clear();
        return;
    };
    for (e, on) in std::mem::take(&mut queue.lights) {
        if on {
            if let Some(back) = off.remove(&e) {
                lights.add(back);
            }
            continue;
        }
        let Ok(prop) = props.get(e) else { continue };
        if off.contains_key(&e) {
            continue;
        }
        let taken = lights.take_object_lights(prop.physics.object, prop.home.translation, prop.physics.radius + 1.0);
        if log_on() {
            info!("prop vfx: {} lights off: {}", prop.physics.object, taken.len());
        }
        off.insert(e, taken);
    }
    // Objects that are gone take their switched-off lights with them. They are not put back: a streamed-out
    // object's unit has already been dropped (`LightSources::remove_unit` runs when the unit leaves, its entities
    // are despawned over the next frames), so lights added now would stay lit where nothing stands; the unit
    // sends them again when it comes back.
    off.retain(|e, _| props.contains(*e));
}
