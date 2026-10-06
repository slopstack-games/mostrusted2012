//! Animated body parts of the player car (active aero), posed on the damage skeleton (`damage.rs`).
//!
//! Data: `mw_assets::car_anim` (the car's `AnimationStateBehaviour`: an HSM and its clips). Rule, for the
//! simple one-clip speed machines (`SpeedRule`): the part deploys when `speedinmph` reaches the
//! condition's speed and retracts below the machine's lower speed operand; the clip plays from its first to
//! its last frame at its own frame rate, and backwards at the same rate (the states' `False` / `0.25`
//! operands, probably a reverse flag and a blend time, are not decoded). Machines that also read `brake`,
//! `steering` or `gas`, or wait on timers, are not run yet (docs/decomp.md "Vehicle animations").
//!
//! `--anim <clip index|all>=<0..1>` pins parts at a position instead (for screenshots).

use bevy::prelude::*;

use crate::damage::CarDamage;
use crate::drive::Car;

/// Metres per second in one mph.
const MPH: f32 = 0.44704;

pub struct CarAnimPlugin;

impl Plugin for CarAnimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AnimOverride>().add_systems(Update, animate_parts);
    }
}

/// `--anim` pins: (clip index, or `None` for all clips, position 0..1).
#[derive(Resource, Default)]
pub struct AnimOverride(pub Vec<(Option<usize>, f32)>);

impl AnimOverride {
    pub fn parse(args: &[String]) -> anyhow::Result<Self> {
        let pins = args
            .iter()
            .map(|a| {
                let (k, v) = a.split_once('=').ok_or_else(|| anyhow::anyhow!("--anim {a}: expected part=0..1"))?;
                let v: f32 = v.parse()?;
                let k = if k == "all" { None } else { Some(k.parse()?) };
                Ok((k, v.clamp(0.0, 1.0)))
            })
            .collect::<anyhow::Result<_>>()?;
        Ok(Self(pins))
    }
}

/// Per car: whether each part is commanded open, and a log clock.
#[derive(Component, Default)]
struct PartState {
    deployed: Vec<bool>,
    log: f32,
}

fn animate_parts(
    mut commands: Commands,
    time: Res<Time>,
    pins: Res<AnimOverride>,
    mut cars: Query<(Entity, &Car, &mut CarDamage, Option<&mut PartState>)>,
) {
    let dt = time.delta_secs();
    for (entity, car, mut dmg, state) in &mut cars {
        let Some(anims) = dmg.animations() else { continue };
        let n = anims.clips.len();
        let Some(mut state) = state else {
            info!(
                "animated parts: {:?}, {} clips, variables {:?}, rule {:?}",
                anims.label, n, anims.variables, anims.speed_rule
            );
            commands.entity(entity).insert(PartState { deployed: vec![false; n], log: 0.0 });
            continue;
        };
        let mph = car.speed.abs() / MPH;
        let mut parts = dmg.parts().to_vec();
        if pins.0.is_empty() {
            let Some(rule) = anims.speed_rule else { continue };
            for (i, clip) in anims.clips.iter().enumerate() {
                let open = state.deployed[i];
                let open = if open { mph >= rule.retract_mph } else { mph >= rule.deploy_mph };
                if open != state.deployed[i] {
                    info!("anim {i} {} at {mph:.1} mph", if open { "deploying" } else { "retracting" });
                }
                state.deployed[i] = open;
                let step = dt / clip.duration().max(1e-3);
                let target = if open { 1.0 } else { 0.0 };
                parts[i] += (target - parts[i]).clamp(-step, step);
            }
        } else {
            for &(k, v) in &pins.0 {
                parts.iter_mut().enumerate().filter(|(i, _)| k.is_none_or(|k| k == *i)).for_each(|(_, p)| *p = v);
            }
        }
        state.log += dt;
        if state.log >= 0.5 && pins.0.is_empty() {
            state.log = 0.0;
            info!("anim parts at {mph:.1} mph: {parts:.2?}");
        }
        dmg.set_parts(&parts);
    }
}
