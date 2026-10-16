//! World objects' coronas (street lamps, traffic lights, speed cameras) and the behaviours a sequence switches
//! (docs/decomp.md "Coronas" → world objects, "Speed camera flash"). Data: `mw_assets::coronas::ObjectCoronas`
//! (the `Coronas` of an object's `WCVfxBehaviour`s) and the lights the same behaviours carry.
//!
//! **Which coronas show** (`FUN_005b31a0`, the behaviour's update): a behaviour packs its five corona bools into a
//! flags byte (`FUN_005b0220`: 1 night, 2 twilight, 4 day, 8 rain, 0x10 wet). With all five set its coronas only
//! show while the behaviour is switched on (`FUN_005b30d0` → `FUN_00532760`: from the start when `543527d0`, or by a
//! sequence's `ModifyBehavioursInAreaSequenceItem`, `sequence_fx.rs`); otherwise they show while the flags meet the
//! environment's state bits (`DayPhase::env_mask`) and the hour lies in the behaviour's window (`FUN_005a7850`).
//! **How bright**: the object is queued while the camera is within the behaviour's reach (the largest |locator| +
//! the definition's `88b68533`, `FUN_00532a80`); each locator is a corona at the lamp level `FUN_0089b400`(flags,
//! the owner's handle) with colour, intensity and size 1 (`FUN_004a6fa0` → `FUN_004a6b00` → `FUN_004a5790`),
//! skipped below 0.001 (`.rdata` 0x00f8bb34).
//!
//! MODEL: a behaviour that goes off at a contact (`713589ad`) is taken as off while its object is knocked loose
//! (`prop_vfx.rs` switches its lights the same way); the stagger id is the lamp light's (`world_lights.rs`), the
//! game's being a runtime handle; part locators are object space; the blink (`c8d7741f`) and the rain / wet bits
//! are not modelled; a smashed compound shows no coronas.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mw_assets::coronas::ObjectCoronas;
use mw_assets::lights::{ObjectLight, WorldLight};
use mw_bundle::ResourceId;

use crate::coronas::{WorldCorona, WorldCoronas};
use crate::props::{Compound, CompoundPiece, PropBody};
use crate::world_lights::LightSources;

pub struct WorldCoronasPlugin;

impl Plugin for WorldCoronasPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ObjectFxCache>().add_systems(Update, (attach, queue_coronas).chain());
    }
}

/// What an object's behaviours switch: their coronas and lights.
pub struct ObjectFx {
    pub coronas: Vec<ObjectCoronas>,
    /// Every light of the object's behaviours (the switched ones are not in `LightSources` until switched on).
    pub lights: Vec<ObjectLight>,
}

type Job = Arc<Mutex<Option<Option<Arc<ObjectFx>>>>>;

#[derive(Resource, Default)]
struct ObjectFxCache {
    done: HashMap<ResourceId, Option<Arc<ObjectFx>>>,
    jobs: HashMap<ResourceId, Job>,
}

/// An object with switchable behaviours, and their state.
#[derive(Component)]
pub struct ObjectBehaviours {
    pub object: ResourceId,
    pub fx: Arc<ObjectFx>,
    /// Per `fx.coronas` entry: switched on (from `543527d0`, then by sequences).
    pub on: Vec<bool>,
    /// The lamp's stagger id (its light's, `WorldLight::stagger`), once found.
    stagger: Option<u16>,
    stagger_tries: u16,
}

/// Props without switchable behaviours (checked once).
#[derive(Component)]
struct NoBehaviours;

fn wanted(prop: &PropBody) -> bool {
    prop.physics.kind != mw_assets::props::ObjectKind::Prop
        && prop.physics.vfx.iter().any(|v| v.coronas > 0 || (v.lights > 0 && !v.active_at_start))
}

/// Give compound and world objects with coronas or switched lights their [`ObjectBehaviours`] (the object's data
/// is read on a worker thread, once per definition).
#[allow(clippy::type_complexity)]
fn attach(
    mut commands: Commands,
    game: Option<Res<crate::Game>>,
    mut cache: ResMut<ObjectFxCache>,
    props: Query<(Entity, &PropBody), (Without<ObjectBehaviours>, Without<NoBehaviours>, Without<CompoundPiece>)>,
    mut world: Option<ResMut<WorldCoronas>>,
) {
    let Some(game) = game else { return };
    let finished: Vec<ResourceId> =
        cache.jobs.iter().filter(|(_, j)| j.lock().unwrap().is_some()).map(|(id, _)| *id).collect();
    for id in finished {
        let fx = cache.jobs.remove(&id).and_then(|j| j.lock().unwrap().take()).flatten();
        if let (Some(fx), Some(w)) = (&fx, world.as_mut()) {
            for c in &fx.coronas {
                w.new_defs.extend(c.entries.iter().map(|(d, _)| d.clone()));
            }
        }
        cache.done.insert(id, fx);
    }
    for (e, prop) in &props {
        if !wanted(prop) {
            commands.entity(e).insert(NoBehaviours);
            continue;
        }
        let object = prop.physics.object;
        match cache.done.get(&object) {
            Some(Some(fx)) => {
                let on = fx.coronas.iter().map(|c| c.active_at_start).collect();
                commands.entity(e).insert(ObjectBehaviours {
                    object,
                    fx: fx.clone(),
                    on,
                    stagger: None,
                    stagger_tries: 0,
                });
            }
            Some(None) => {
                commands.entity(e).insert(NoBehaviours);
            }
            None if !cache.jobs.contains_key(&object) => {
                let job: Job = Arc::new(Mutex::new(None));
                let out = job.clone();
                let store = game.store.clone();
                let kind = prop.physics.kind;
                std::thread::spawn(move || {
                    let fx = store.object_def(object, kind).ok().map(|def| {
                        Arc::new(ObjectFx { coronas: store.object_coronas(&def), lights: store.object_lights(&def) })
                    });
                    *out.lock().unwrap() = Some(fx);
                });
                cache.jobs.insert(object, job);
            }
            None => {}
        }
    }
}

/// `FUN_005a7850`: the hour lies in the behaviour's window [from, to), which may wrap past midnight.
fn in_hours((from, to): (u8, u8), hour: u8) -> bool {
    let inside = hour < to && from <= hour;
    let outside_wrap = hour < to || from <= hour;
    if to < from { outside_wrap } else { inside }
}

/// The lights this object's behaviour `behaviour` carries, placed by the object's transform.
pub fn behaviour_lights(
    fx: &ObjectFx,
    object: ResourceId,
    behaviour: ResourceId,
    t: &GlobalTransform,
) -> Vec<WorldLight> {
    let m = t.affine();
    fx.lights
        .iter()
        .filter(|l| l.behaviour == behaviour)
        .map(|l| {
            let q = Quat::from_array(l.locator.rotation);
            let pos = m.transform_point3(Vec3::from(l.locator.position));
            let dir = m.transform_vector3(q * Vec3::Z).normalize_or_zero();
            WorldLight {
                def: l.def.clone(),
                position: pos.to_array(),
                direction: dir.to_array(),
                object: Some(object),
                stagger: None,
            }
        })
        .collect()
}

#[allow(clippy::type_complexity)]
fn queue_coronas(
    state: Option<Res<crate::weather::EnvironmentState>>,
    sources: Res<LightSources>,
    camera: Query<&GlobalTransform, crate::MainCamera>,
    mut objects: Query<(&GlobalTransform, &PropBody, &mut ObjectBehaviours, Option<&Compound>)>,
    world: Option<ResMut<WorldCoronas>>,
    mut staggers: Local<(usize, HashMap<ResourceId, Vec<(Vec3, u16)>>)>,
    mut timing: Local<(u32, f32, u32)>,
) {
    let Some(mut world) = world else { return };
    world.items.clear();
    // `MW_NO_WORLD_CORONAS=1`: A/B switch (cars' coronas stay).
    static OFF: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var_os("MW_NO_WORLD_CORONAS").is_some());
    if *OFF {
        return;
    }
    let (Some(state), Some(cam)) = (state, camera.iter().next()) else { return };
    let started = std::time::Instant::now();
    // The object lights' stagger ids by object, rebuilt when the light list changes.
    if staggers.0 != sources.lights.len() {
        staggers.0 = sources.lights.len();
        staggers.1.clear();
        for l in &sources.lights {
            if let (Some(o), Some(s)) = (l.object, l.stagger) {
                staggers.1.entry(o).or_default().push((Vec3::from(l.position), s));
            }
        }
    }
    let eye = cam.translation();
    let hour = state.want.hour;
    let phase = sources.transition.phase(hour);
    let mask = phase.env_mask();
    let hour_byte = hour.rem_euclid(24.0).floor() as u8;
    for (t, prop, mut b, compound) in &mut objects {
        if compound.is_some_and(|c| c.is_smashed()) || b.fx.coronas.is_empty() {
            continue;
        }
        let pos = t.translation();
        let d = pos.distance(eye);
        if b.fx.coronas.iter().all(|c| d >= c.reach) {
            continue;
        }
        // The lamp's stagger: its light's (the light arrives with the same track unit, maybe a little later).
        if b.stagger.is_none() && b.stagger_tries < 600 {
            b.stagger_tries += 1;
            let r = prop.physics.radius + 2.0;
            let home = prop.home.translation;
            b.stagger =
                staggers.1.get(&b.object).and_then(|v| v.iter().find(|(p, _)| p.distance(home) <= r)).map(|&(_, s)| s);
        }
        let m = t.affine();
        for (i, c) in b.fx.coronas.iter().enumerate() {
            if d >= c.reach {
                continue;
            }
            // Off while knocked loose (MODEL, as `prop_vfx.rs` does for the lights).
            if c.off_by_contact && !prop.pinned {
                continue;
            }
            let shown = if c.by_time() { c.flags & mask != 0 && in_hours(c.hours, hour_byte) } else { b.on[i] };
            if !shown {
                continue;
            }
            let level = phase.switch_level(c.flags & 4 != 0, c.flags & 1 != 0, b.stagger);
            if level < 0.001 {
                continue;
            }
            for (def, locators) in &c.entries {
                for l in locators {
                    let q = Quat::from_array(l.rotation);
                    world.items.push(WorldCorona {
                        def: def.clone(),
                        pos: m.transform_point3(Vec3::from(l.position)),
                        dir: m.transform_vector3(q * Vec3::Z).normalize_or_zero(),
                        anim: level,
                    });
                }
            }
        }
    }
    // `MW_CORONA_LOG=1`: the queue's size and cost, about once a second.
    if std::env::var_os("MW_CORONA_LOG").is_some() {
        let (n, total, frames) = &mut *timing;
        *n += 1;
        *total += started.elapsed().as_secs_f32();
        *frames += 1;
        if *frames >= 60 {
            info!(
                "world coronas: {} queued from {} objects, {:.3} ms per frame",
                world.items.len(),
                objects.iter().len(),
                *total / *n as f32 * 1000.0
            );
            *frames = 0;
            *total = 0.0;
            *n = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hour_window() {
        assert!(in_hours((0, 24), 0) && in_hours((0, 24), 23));
        assert!(in_hours((20, 6), 22) && in_hours((20, 6), 3) && !in_hours((20, 6), 12));
        assert!(in_hours((8, 20), 12) && !in_hours((8, 20), 20));
    }
}
