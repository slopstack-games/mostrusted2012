//! The junctions' traffic lights on their lamp posts (docs/decomp.md "Traffic lights: the lamps"). The traffic
//! system's light cycle (`traffic.rs` `cycle_lights`, the game's `FUN_00605d30`) keeps a record per lamp and,
//! when a lamp changes colour, sends its object two kinds of game-logic message: 0xb switches on the behaviours
//! labelled `TrafficLight_<colour>`, 0xc switches off the other two (`FUN_00553a70` / `FUN_00553ac0` → the
//! object's activate / deactivate by label hash; the hashes of "TrafficLight_Red" / "_Amber" / "_Green" are
//! taken at init in `FUN_005eb060`: `DAT_0110f384` / `388` / `38c`). A group the phase holds goes amber
//! for 2 s, then red; the others go green. The lamp's object is named by its instance id
//! (`mw_assets::traffic_lamps`), found here by where it stands.
//!
//! MODEL: the colour is applied to a lamp whenever its object is (re)attached, not only at the change (in the
//! game a lamp that streams in shows its behaviours' start state until the next change); a lamp is the
//! world object with switchable behaviours within 1 m of its instance's position.

use crate::platform::PollLock;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;

use crate::props::PropBody;
use crate::traffic::{LampColour, TrafficSystem};
use crate::world_coronas::ObjectBehaviours;
use crate::world_lights::LightSources;

pub struct TrafficLampsPlugin;

impl Plugin for TrafficLampsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TrafficLamps>().add_systems(Update, apply);
    }
}

type Job = Arc<Mutex<Option<HashMap<u32, Vec3>>>>;

#[derive(Resource, Default)]
struct TrafficLamps {
    /// Instance id → where the lamp's object stands.
    positions: Option<HashMap<u32, Vec3>>,
    job: Option<Job>,
    /// The colour each lamp was last told, and the entity it was applied to.
    colours: HashMap<u32, (LampColour, Option<Entity>)>,
    shot_clock: f32,
    shots: u32,
}

/// `MW_TRAFFIC_LAMP_LOG=1`: log the changes.
fn log_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MW_TRAFFIC_LAMP_LOG").is_some())
}

const LABELS: [(LampColour, &str); 3] = [
    (LampColour::Red, "TrafficLight_Red"),
    (LampColour::Amber, "TrafficLight_Amber"),
    (LampColour::Green, "TrafficLight_Green"),
];

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn apply(
    mut commands: Commands,
    time: Res<Time>,
    game: Option<Res<crate::Game>>,
    system: Option<ResMut<TrafficSystem>>,
    lights: Option<ResMut<LightSources>>,
    mut lamps: ResMut<TrafficLamps>,
    player: Query<(), (With<crate::drive::Car>, With<crate::drive::Player>)>,
    mut exit: MessageWriter<AppExit>,
    mut objects: Query<(Entity, &GlobalTransform, &PropBody, &mut ObjectBehaviours)>,
) {
    let (Some(game), Some(mut sys), Some(mut lights)) = (game, system, lights) else { return };
    let lamps = &mut *lamps;
    if lamps.positions.is_none() {
        match &lamps.job {
            None => {
                let job: Job = Arc::new(Mutex::new(None));
                let out = job.clone();
                let store = game.store.clone();
                crate::platform::thread::spawn(move || {
                    let handles: Vec<u32> = store
                        .traffic_lanes()
                        .map(|t| {
                            t.hulls
                                .iter()
                                .flat_map(|h| &h.junctions)
                                .flat_map(|j| &j.groups)
                                .flat_map(|g| g.light_objects.iter().copied())
                                .filter(|&o| o != 0)
                                .collect()
                        })
                        .unwrap_or_default();
                    let at = store.instance_positions(&handles);
                    *out.lock().unwrap() = Some(at.into_iter().map(|(k, p)| (k, Vec3::from(p))).collect());
                });
                lamps.job = Some(job);
            }
            Some(job) => {
                let done = job.poll_lock().and_then(|mut s| s.take());
                if let Some(p) = done {
                    info!("traffic lamps: {} lamp objects placed", p.len());
                    if log_on() {
                        let mut v: Vec<_> = p.iter().collect();
                        v.sort_by_key(|(k, _)| **k);
                        for (k, at) in v {
                            info!("traffic lamp {k} at {at:.1}");
                        }
                    }
                    // A record starts zeroed: state 0, red (MODEL: shown as such until the cycle tells it more;
                    // the game sends nothing for the zero state).
                    for &k in p.keys() {
                        lamps.colours.entry(k).or_insert((LampColour::Red, None));
                    }
                    lamps.positions = Some(p);
                    lamps.job = None;
                }
            }
        }
    }
    // `MW_TRAFFIC_LAMP_SHOTS=<dir>[,<period s>[,<count>]]`: screenshots every period (default 4 s, 10 of them)
    // once the lamps are placed, for checks over a few phases.
    if lamps.positions.is_some()
        && let Some(v) = std::env::var_os("MW_TRAFFIC_LAMP_SHOTS")
    {
        let v = v.to_string_lossy().into_owned();
        let mut it = v.split(',');
        let dir = it.next().unwrap_or(".").to_string();
        let period: f32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(4.0);
        let count: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(10);
        // Counted from the player's car on the ground (`--screenshot-delay`'s start).
        if !player.is_empty() {
            lamps.shot_clock += time.delta_secs();
        }
        if lamps.shots >= count && lamps.shot_clock >= period * (count + 1) as f32 {
            exit.write(AppExit::Success);
        }
        if lamps.shots < count && lamps.shot_clock >= period * (lamps.shots + 1) as f32 {
            let path = std::path::Path::new(&dir).join(format!("lamps-{:02}.png", lamps.shots));
            info!("traffic lamps: screenshot {} at {:.1} s", path.display(), lamps.shot_clock);
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(path));
            lamps.shots += 1;
        }
    }
    for (handle, colour) in std::mem::take(&mut sys.lamp_events) {
        if log_on() {
            info!("traffic lamp {handle}: {colour:?}");
        }
        lamps.colours.insert(handle, (colour, None));
    }
    let Some(positions) = &lamps.positions else { return };
    for (handle, (colour, applied)) in lamps.colours.iter_mut() {
        // Still the same entity, already told.
        if applied.is_some_and(|e| objects.contains(e)) {
            continue;
        }
        let Some(&at) = positions.get(handle) else { continue };
        let Some(e) = objects.iter().find(|(_, _, p, _)| p.home.translation.distance(at) < 1.0).map(|(e, ..)| e) else {
            continue;
        };
        let Ok((_, t, prop, mut b)) = objects.get_mut(e) else { continue };
        let radius = prop.physics.radius + 2.0;
        for (c, label) in LABELS {
            let idx: Vec<usize> = b.labelled(label).collect();
            for i in idx {
                b.switch(i, c == *colour, t, radius, &mut lights);
            }
        }
        *applied = Some(e);
    }
}
