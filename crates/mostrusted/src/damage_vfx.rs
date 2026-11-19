//! The damage sequences' effects: smoke and the like at a car's locators once a zone is damaged enough
//! (`mw_assets::damage_vfx`; docs/decomp.md "Damage sequences and their effects").
//!
//! Each of the car's eight damage sequences runs on the level of one zone (`RaceCarEntityTimelineController`,
//! the zone level capped at 0.99 unless wrecked, × 100), and its `VfxLocatorGroupVehicleSpotEffectSequenceItem`s
//! play while that axis is inside their range: on entering, an effect per locator of the item's group (at most 4)
//! starts with the item's `Intensity`, is refreshed while the item stays active, and stops emitting when the axis
//! leaves the range (a repair). An item entered within 0.1 s of the car's spawn stays silent until entered again
//! (the play gate on vehicle +0x1964).
//!
//! Parameters per effect (`FUN_00865f20` → `FUN_00864ec0`): `Transform` = car × locator × `LocalOffset`;
//! `Velocity` = the car's (vehicle +0x120, not per locator); `Intensity` = the item's; the bounce plane is the car's
//! up axis through its origin (the branch the game takes unless `FUN_00572810`, not identified).
//!
//! MODEL: the car is never "wrecked" (the damage bar's used segments are not kept, `car_impacts.rs`), so the
//! full-damage items (range 100..100) never play; vehicle +0x1c2a is taken as set; `LocalRotation` is not applied
//! (its Euler order in `FUN_008644e0` was not read; zero in every item of the data); an effect still alive when its
//! item is entered again is restarted rather than taken over.
//!
//! `MW_DAMAGE_VFX_LOG=1` logs every start and stop; `MW_DAMAGE_VFX_NO_GATE=1` skips the spawn gate so `--damage`
//! shows its effects (a debug switch for looking at them, not game behaviour).

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use mw_assets::damage_vfx::{ItemState, Slot, Step, axis};

use crate::damage::CarDamage;
use crate::particles::{EffectRequests, EmitterKey, Instances, Params, Vfx};

/// Emitter slots: this + 0x100 × sequence + 4 × item + locator.
const SLOT_DAMAGE: u32 = 0x3000;
/// The sequences' weight (sequence player +0x40, 1 from its init; the damage module never changes it).
const WEIGHT: f32 = 1.0;

pub struct DamageVfxPlugin;

impl Plugin for DamageVfxPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (attach, run.before(crate::particles::simulate).run_if(resource_exists::<Vfx>)).chain(),
        );
    }
}

/// A car's damage-sequence item states.
#[derive(Component)]
pub struct DamageVfx {
    /// Per sequence, per item.
    items: Vec<Vec<ItemState>>,
    /// Seconds since the car spawned (vehicle +0x1964 counts 0.1 s down from the spawn).
    age: f32,
}

fn attach(mut commands: Commands, cars: Query<(Entity, &CarDamage), Without<DamageVfx>>) {
    for (entity, dmg) in &cars {
        let items = dmg.vfx().iter().map(|s| vec![ItemState::default(); s.items.len()]).collect();
        commands.entity(entity).insert(DamageVfx { items, age: 0.0 });
    }
}

fn run(
    time: Res<Time>,
    vfx: Res<Vfx>,
    mut instances: ResMut<Instances>,
    mut requests: ResMut<EffectRequests>,
    mut cars: Query<(Entity, &CarDamage, &mut DamageVfx, &GlobalTransform, Option<&LinearVelocity>)>,
) {
    let log = std::env::var_os("MW_DAMAGE_VFX_LOG").is_some();
    let no_gate = std::env::var_os("MW_DAMAGE_VFX_NO_GATE").is_some();
    let dt = time.delta_secs();
    for (entity, dmg, mut state, transform, v) in &mut cars {
        let state = &mut *state;
        // `FUN_00862010`: armed only when the timer has run out (|t| ≤ FLT_EPSILON, `.rdata` 0x00ff21cc).
        let ready = state.age >= mw_assets::damage_vfx::SPAWN_GATE || no_gate;
        state.age += dt;
        let world = transform.to_matrix();
        let (_, rotation, origin) = transform.to_scale_rotation_translation();
        let up = rotation * Vec3::Y;
        let ground = up.extend(-up.dot(origin));
        let velocity = v.map_or(Vec3::ZERO, |v| v.0);
        for (s, seq) in dmg.vfx().iter().enumerate() {
            let Some(level) = seq.zone().and_then(|z| dmg.levels.get(z)) else { continue };
            let t = axis(*level, false);
            for (i, it) in seq.items.iter().enumerate() {
                let Some(effect) = it.item.effect else { continue };
                let st = &mut state.items[s][i];
                let step = st.tick(&it.item, t, ready);
                let key = |k: usize| EmitterKey {
                    owner: entity,
                    slot: SLOT_DAMAGE + 0x100 * s as u32 + 4 * i as u32 + k as u32,
                };
                match step {
                    Step::Idle => continue,
                    Step::Stop => {
                        // `0x00862890`: clear each live effect's emitting bit; the particles live on.
                        for k in 0..it.locators.len() {
                            instances.release(key(k));
                        }
                        if log {
                            info!("damage vfx: stop effect {effect} (zone {:?}, axis {t:.0})", seq.zone());
                        }
                        continue;
                    }
                    Step::Update => {}
                }
                if !vfx.has_effect(effect) {
                    requests.request(effect);
                    continue;
                }
                let local_t = t - it.item.start;
                let intensity = it.item.intensity_at(local_t, WEIGHT);
                for (k, loc) in it.locators.iter().enumerate() {
                    let locator = Mat4::from_rotation_translation(
                        Quat::from_array(loc.rotation).normalize(),
                        Vec3::from(loc.position),
                    );
                    let frame = world * locator * Mat4::from_translation(Vec3::from(it.item.offset));
                    let params = Params {
                        transform: frame,
                        velocity,
                        intensity,
                        emitting: true,
                        moved: 0.0,
                        ground,
                        impact: 0.0,
                        movement: 0.0,
                        mesh: None,
                        paint: Vec3::ZERO,
                        wheel: [0.0; 3],
                    };
                    match st.slots[k] {
                        // `FUN_00864ec0` state 0: start unless an effect lives on the slot, then state 1.
                        Slot::Armed => {
                            if let Some(p) = instances.keyed_mut(key(k)) {
                                *p = params;
                            } else if instances.restart(&vfx, key(k), effect, params) && log {
                                info!(
                                    "damage vfx: start effect {effect} at locator {k} (zone {:?}, axis {t:.0}, item {}..{}, intensity {intensity})",
                                    seq.zone(),
                                    it.item.start,
                                    it.item.end
                                );
                            }
                            st.slots[k] = Slot::Playing;
                        }
                        // State 1: refresh while it lives.
                        Slot::Playing => {
                            if let Some(p) = instances.keyed_mut(key(k)) {
                                *p = params;
                            }
                        }
                        Slot::Idle | Slot::Off => {}
                    }
                }
            }
        }
    }
}
