//! Exhaust effects: the car's spot effects (`VehicleVfxBehaviour.SpotEffects`, played at locators of its
//! `VehicleGraphicsSpec`) switched by the game's effect group bits, and the exhaust pop rule that fires the
//! backfire flames and the engine's backfire / pop sounds. Research log 2026-10-09 "exhaust effects";
//! docs/decomp.md "Exhaust pops and backfires" and "Car spot effects".
//!
//! - **Pops** (`FUN_009b20b0`, every physics tick after the gearbox): `mw_assets::engine_driver::ExhaustPops`.
//!   A backfire holds group 14 for two updates (`FUN_0057a5e0` sets a countdown of 2) and starts
//!   `Backfire_Sequence`; a pop starts `Pop_Sequence`. The rule reads the sound's audible RPM (vehicle +0x1f00 =
//!   `Idle_RPM + (Max_RPM − Idle_RPM) × +0x1f10`), and +0x1f10 includes the RPM offsets the engine sequences post
//!   (effect property 1, `FUN_009aea50`): the gear changes' engagement kick (+0.2 of the range, falling to 0 in
//!   a tick at its end on the 911) and the LFO shunt are what drop it fast enough for backfires, so the player's
//!   rule is fed the engine sound's offsets and load ([`Exhaust::set_sound`]).
//! - **Groups** (`FUN_0057a5e0` builds the mask, `FUN_005b2740` adds 12 for a wreck): 8 while nitrous burns, 14
//!   backfire, 19 while the engine runs (the gearbox is past its start-up states, `ignition.rs`). 9 and 20 are
//!   the nitrous upgrade's other modes (not run), 12 the wreck (not run).
//! - **Spot effects** (`FUN_005b28d0`): each effect plays at every locator of its group (mirrored too with
//!   `Mirror`); a non-looping one starts on its group's rising edge and emits while the group is on. Its
//!   parameters (`FUN_0049bc60`): `Transform` = car × locator, `Velocity` = the body's velocity at the locator,
//!   `Intensity` = 1, `IsEmitting` = the group bit.
//!
//! `MW_EXHAUST_LOG=1` logs every pop and backfire; `MW_EXHAUST_FORCE=<seconds>` also fires a backfire that
//! often (a debug switch for looking at the flames and hearing the sound, not game behaviour).

use avian3d::prelude::{AngularVelocity, CenterOfMass, LinearVelocity};
use bevy::prelude::*;
use mw_assets::engine_driver::{ExhaustPops, PopEvent, PopInput, PopParams, normalised_rpm_with};
use mw_assets::sound::EngineTuning;
use mw_assets::vfx::{VehicleSpotEffect, spot_group};

use crate::drive::Car;
use crate::particles::{BearGlobals, EffectRequests, EmitterKey, Instances, Params, SLOT_SPOT, Vfx};

pub struct ExhaustPlugin;

impl Plugin for ExhaustPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, exhaust_tick.after(crate::drive::car_physics))
            .add_systems(Update, push_sounds)
            .add_systems(Update, spot_effects.before(crate::particles::simulate).run_if(resource_exists::<Vfx>));
    }
}

/// Game time of the first backfire, for `--screenshot-after-backfire`.
#[derive(Resource)]
pub struct FirstBackfire(pub f32);

/// A car's exhaust state and spot effects.
#[derive(Component)]
pub struct Exhaust {
    effects: Vec<VehicleSpotEffect>,
    params: PopParams,
    tuning: EngineTuning,
    pops: ExhaustPops,
    /// Updates the backfire group stays on.
    backfire_hold: u8,
    /// Whether each effect's group was on at its last update (non-looping effects start on a rising edge;
    /// an effect still loading counts as off).
    was_on: Vec<bool>,
    /// Events not yet handed to the engine sound.
    events: Vec<PopEvent>,
    /// The engine sound's last RPM offset (effect property 1) and load (+0x1f20), when the car has one.
    sound: Option<(f32, f32)>,
    /// Totals, for tests and logs.
    pub backfires: u32,
    pub pop_count: u32,
}

impl Exhaust {
    pub fn new(effects: Vec<VehicleSpotEffect>, params: PopParams, tuning: EngineTuning) -> Self {
        for e in &effects {
            debug!(
                "spot effect {} group {} at {} locators (group {}){}{}",
                e.effect,
                e.group,
                e.locators.len(),
                e.locator_group,
                if e.looping { ", looping" } else { "" },
                if e.mirror { ", mirrored" } else { "" }
            );
        }
        info!("exhaust: backfire {:.3}, pop {:.3}, {} spot effects", params.backfire, params.pop, effects.len());
        Self {
            was_on: vec![false; effects.len()],
            effects,
            params,
            tuning,
            pops: ExhaustPops::default(),
            backfire_hold: 0,
            events: Vec::new(),
            sound: None,
            backfires: 0,
            pop_count: 0,
        }
    }

    /// The engine sound's RPM offset and load this update (`EngineDriver::state`): the pop rule's audible RPM
    /// is the sound's (vehicle +0x1f10 with the effect items' offsets, `FUN_009aea50`).
    pub fn set_sound(&mut self, rpm_offset: f32, load: f32) {
        self.sound = Some((rpm_offset, load));
    }

    /// The car's effect group bits this update (`FUN_0057a5e0`), counting the backfire hold down.
    fn mask(&mut self, car: &Car, running: bool) -> u32 {
        let mut mask = if running { 1u32 << spot_group::ENGINE_RUNNING } else { 0 };
        if car.drivetrain.nitrous {
            mask |= 1 << spot_group::NITROUS;
        }
        if self.backfire_hold > 0 {
            self.backfire_hold -= 1;
            mask |= 1 << spot_group::BACKFIRE;
        }
        mask
    }
}

/// The pop rule, once per physics tick after the drivetrain.
fn exhaust_tick(
    mut commands: Commands,
    time: Res<Time>,
    first: Option<Res<FirstBackfire>>,
    mut globals: ResMut<BearGlobals>,
    mut cars: Query<(&Car, &mut Exhaust, Has<crate::drive::Player>, Option<&crate::ignition::EngineStart>)>,
) {
    let log = std::env::var_os("MW_EXHAUST_LOG").is_some();
    for (car, mut ex, player, start) in &mut cars {
        let p = &car.params;
        // The sound's engine load (vehicle +0x1f20, `FUN_009b1f70`) and RPM offsets, from the car's engine sound
        // when it has one; otherwise the engine's lagged throttle (+0x1d14) stands in for the load.
        let (offset, load) = ex.sound.unwrap_or((0.0, car.drivetrain.throttle));
        // The Bear effects' engine-load input is the player's car (`BearGlobalParameters`); AI and traffic cars run
        // the pop rule too but must not overwrite it.
        if player {
            globals.player_load = load;
        }
        let input = PopInput {
            fraction: normalised_rpm_with(&ex.tuning, car.rpm, p.idle_rpm, p.redline_rpm, load, offset),
            idle_rpm: p.idle_rpm,
            max_rpm: p.redline_rpm,
            gearbox_throttle: car.drivetrain.gearbox_throttle,
            running: start.is_none_or(|s| s.running()),
            cruising: car.drivetrain.cruising,
        };
        let params = ex.params;
        let mut event = ex.pops.tick(&params, &input);
        if let Some(every) = std::env::var("MW_EXHAUST_FORCE").ok().and_then(|v| v.parse::<f32>().ok()) {
            let now = time.elapsed_secs();
            let dt = time.delta_secs();
            if every > 0.0 && (now / every).floor() != ((now - dt) / every).floor() {
                event = PopEvent::Backfire;
            }
        }
        if log && ex.pops.drop > 0.02 {
            info!("exhaust drop {:.3} of the range at {:.0} rpm, gear {}", ex.pops.drop, car.rpm, car.gear);
        }
        match event {
            PopEvent::Backfire => {
                // `FUN_0057a5e0`: the backfire group stays on for two updates.
                ex.backfire_hold = 2;
                ex.backfires += 1;
                if first.is_none() && ex.backfires == 1 {
                    commands.insert_resource(FirstBackfire(time.elapsed_secs()));
                }
            }
            PopEvent::Pop => ex.pop_count += 1,
            PopEvent::None => {}
        }
        if event != PopEvent::None {
            if log {
                info!(
                    "exhaust {event:?} at {:.0} rpm, gear {}, gearbox throttle {:.2}",
                    car.rpm, car.gear, car.drivetrain.gearbox_throttle
                );
            }
            ex.events.push(event);
        }
        // Keep at most a second's worth for the sound.
        if ex.events.len() > 60 {
            ex.events.remove(0);
        }
    }
}

/// Hand the pop probability and the events to the engine sound.
fn push_sounds(mut cars: Query<&mut Exhaust>, mut sounds: Query<(&mut crate::audio::EngineSound, &ChildOf)>) {
    for (mut sound, parent) in &mut sounds {
        let Ok(mut ex) = cars.get_mut(parent.parent()) else { continue };
        let Some(driver) = sound.driver.as_mut() else { continue };
        let state = driver.state();
        ex.set_sound(state.effects.rpm_offset, state.load);
        let probability = ex.pops.probability;
        driver.push_pops(probability, PopEvent::None);
        for e in ex.events.drain(..) {
            driver.push_pops(probability, e);
        }
    }
    for mut ex in &mut cars {
        ex.events.clear();
    }
}

/// Play the car's spot effects for the groups that are on.
#[allow(clippy::type_complexity)]
fn spot_effects(
    vfx: Res<Vfx>,
    mut instances: ResMut<Instances>,
    mut requests: ResMut<EffectRequests>,
    mut cars: Query<(
        Entity,
        &Car,
        &mut Exhaust,
        &GlobalTransform,
        &LinearVelocity,
        &AngularVelocity,
        Option<&CenterOfMass>,
        Option<&crate::ignition::EngineStart>,
    )>,
) {
    for (entity, car, mut ex, transform, v, w, com, start) in &mut cars {
        let mask = ex.mask(car, start.is_none_or(|s| s.running()));
        let ex = &mut *ex;
        let world = transform.to_matrix();
        let centre = transform.transform_point(com.map_or(Vec3::ZERO, |c| c.0));
        // The road plane under the car (its wheel contacts), for particles that bounce.
        let contacts: Vec<_> = car.wheel_contacts().flatten().collect();
        let ground = if contacts.is_empty() {
            Vec4::new(0.0, 1.0, 0.0, f32::MAX)
        } else {
            let n = contacts.iter().map(|c| c.normal).sum::<Vec3>().normalize_or(Vec3::Y);
            let point = contacts.iter().map(|c| c.point).sum::<Vec3>() / contacts.len() as f32;
            n.extend(-n.dot(point))
        };
        for (i, fx) in ex.effects.iter().enumerate() {
            let on = mask & (1 << fx.group) != 0;
            // `da6ac384` 2 needs a world flag (+0x1b934) not traced; assumed clear.
            if fx.condition == 2 {
                continue;
            }
            if !vfx.has_effect(fx.effect) {
                // Every effect of the car is loaded as soon as the car is there, so the first backfire has its
                // flames.
                requests.request(fx.effect);
                continue;
            }
            // A non-looping effect starts on the group's rising edge only; a looping one whenever it is not
            // playing while the group is on.
            let start = fx.looping || (on && !ex.was_on[i]);
            ex.was_on[i] = on;
            for (k, loc) in fx.locators.iter().enumerate() {
                let local = Mat4::from_rotation_translation(
                    Quat::from_array(loc.rotation).normalize(),
                    Vec3::from(loc.position),
                );
                let sides: &[Mat4] = if fx.mirror { &[Mat4::IDENTITY, MIRROR] } else { &[Mat4::IDENTITY] };
                for (m, side) in sides.iter().enumerate() {
                    let frame = world * *side * local;
                    let at = frame.w_axis.truncate();
                    let params = Params {
                        transform: frame,
                        velocity: v.0 + w.0.cross(at - centre),
                        intensity: 1.0,
                        emitting: on,
                        moved: 0.0,
                        ground,
                        impact: 0.0,
                        movement: 0.0,
                        mesh: None,
                        paint: Vec3::ZERO,
                    };
                    let key = EmitterKey { owner: entity, slot: SLOT_SPOT + 0x10 * i as u32 + 8 * m as u32 + k as u32 };
                    instances.refresh_or_start(&vfx, key, fx.effect, params, start);
                }
            }
        }
    }
}

/// Reflection across the car's centre line (x → −x), for `Mirror` effects.
const MIRROR: Mat4 = Mat4::from_cols(Vec4::NEG_X, Vec4::Y, Vec4::Z, Vec4::W);
