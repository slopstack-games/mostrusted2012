//! The runtime side of the event's gameplay triggers (`mw_logic::natives::gameplay_triggers`, the manager at
//! G+0x2f408; decomp.md "Gameplay triggers"). The logic holds the active list, the queue, the fired list and the
//! rules; here:
//!
//! - **load**: every `GameplayTrigger` (`mw_assets::gameplay_triggers`) becomes the logic's definition, and the
//!   world trigger boxes their inputs name (`TriggerData`) its input boxes (read on a thread at start-up);
//! - **feed**: each frame the local player's car box (`race.rs`'s overlap box from the body's half extents) and the
//!   fired triggers' sequences still playing (the mode-1 re-arm);
//! - **step**: [`mw_logic::natives::gameplay_triggers::step`] (`FUN_006fbfb0`) on the game clock;
//! - **carry out** what fired (`FUN_006f9670`): a `SequenceOutput` plays through `event_timeline.rs` (its items
//!   reach `cutscene_spawns.rs` and `sequence_picture.rs`), a speech list through the speech manager
//!   (`SpeechCore::request_first`). Logged only: AI-player and roadblock outputs.
//!
//! MODEL: spheres among the input triggers are not tested (the logic's trigger boxes are boxes; the original's
//! overlap job hits spheres too).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mw_logic::natives::gameplay_triggers::{self as gt, Def, Fire, Output};
use mw_logic::natives::trigger_boxes::TriggerBox;

use crate::drive::{Car, Player};

pub struct GameplayTriggersPlugin;

impl Plugin for GameplayTriggersPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Triggers>()
            .add_systems(Startup, load)
            .add_systems(Update, step.after(crate::logic::LogicTick).before(crate::event_timeline::EventTimelineStep));
    }
}

struct Loaded {
    defs: Vec<Def>,
    boxes: HashMap<u32, TriggerBox>,
    spheres: usize,
}

#[derive(Resource, Default)]
struct Triggers {
    job: Option<Arc<Mutex<Option<Loaded>>>>,
    logged: HashSet<String>,
    /// The last logged (event, active list).
    active: (u32, Vec<u32>),
}

fn load(game: Res<crate::Game>, mut t: ResMut<Triggers>) {
    let job = Arc::new(Mutex::new(None));
    let (out, store) = (job.clone(), game.store.clone());
    let _ = crate::platform::thread::Builder::new().name("mw-gameplay-triggers".into()).spawn(move || {
        let defs: Vec<Def> = match store.gameplay_triggers() {
            Ok(list) => list
                .into_iter()
                .map(|g| Def {
                    id: g.id,
                    predicate: g.predicate,
                    inputs: g.inputs,
                    outputs: g
                        .outputs
                        .into_iter()
                        .map(|o| Output {
                            predicate: o.predicate,
                            speech: o.speech,
                            ai_players: o.ai_players,
                            sequence: o.sequence.map(|s| (s.sequence, s.kind)),
                            roadblocks: o.roadblocks,
                        })
                        .collect(),
                    rearm: g.rearm,
                    rearm_time: g.rearm_time,
                    while_crashed: g.while_crashed,
                    after_crash: g.after_crash,
                })
                .collect(),
            Err(e) => {
                warn!("gameplay triggers: {e}");
                Vec::new()
            }
        };
        let wanted: HashSet<u32> = defs.iter().flat_map(|d| d.inputs.iter().copied()).collect();
        let mut boxes = HashMap::new();
        let mut spheres = 0;
        if let Ok(td) = store.trigger_data() {
            for tr in td.triggers.iter().filter(|tr| wanted.contains(&tr.id)) {
                match tr.shape {
                    mw_formats::TriggerShape::Box { rotation, size } => {
                        boxes.insert(
                            tr.id,
                            TriggerBox::from_quat(tr.id, tr.position, rotation, size, tr.flags & 1 != 0),
                        );
                    }
                    _ => spheres += 1,
                }
            }
        }
        *out.lock().unwrap() = Some(Loaded { defs, boxes, spheres });
    });
    t.job = Some(job);
}

#[allow(clippy::too_many_arguments)]
fn step(
    logic: Option<NonSend<crate::world_logic::WorldLogic>>,
    mut t: ResMut<Triggers>,
    game: Res<crate::Game>,
    virt: Res<Time<Virtual>>,
    player: Query<(&Transform, &Car), With<Player>>,
    mut timeline: ResMut<crate::event_timeline::EventTimeline>,
    speech: Option<Res<crate::police_speech::SpeechLink>>,
    race: Option<Res<crate::race::RaceState>>,
) {
    let Some(logic) = logic else { return };
    let mut st = logic.state.borrow_mut();
    let got = t.job.as_ref().and_then(|job| job.lock().unwrap().take());
    if let Some(l) = got {
        info!(
            "gameplay triggers: {} triggers, {} input boxes ({} input triggers not boxes)",
            l.defs.len(),
            l.boxes.len(),
            l.spheres
        );
        let g = &mut st.gameplay_triggers;
        g.defs = l.defs.into_iter().map(|d| (d.id, d)).collect();
        g.inputs.boxes = l.boxes;
        // An event that started before the definitions arrived: its triggers become active now.
        let current = st.game_modes.current_event;
        if current != 0
            && st.gameplay_triggers.active.is_empty()
            && let Some(ids) =
                st.game_modes.events.iter().find(|e| e.id == current).map(|e| e.gameplay_triggers.clone())
        {
            gt::activate(&mut st, &ids);
        }
        t.job = None;
    }
    if t.job.is_some() {
        return;
    }
    st.gameplay_triggers.inputs.car =
        player.single().ok().map(|(tf, car)| crate::race::car_box(tf, car.params.body_half_extents));
    let playing: Vec<u32> =
        st.gameplay_triggers.fired.iter().filter_map(|f| f.sequence).filter(|&q| timeline.playing(q)).collect();
    st.gameplay_triggers.inputs.playing = playing;
    let dt = if virt.is_paused() { 0.0 } else { virt.delta_secs() };
    let active = (st.game_modes.current_event, st.gameplay_triggers.active.clone());
    if t.active != active {
        info!("gameplay triggers: event {} active {:?}", active.0, active.1);
        t.active = active;
    }
    let fires = gt::step(&mut st, dt);
    drop(st);
    let groups =
        race.as_ref().map_or_else(Vec::new, |r| vec![r.event.feedback_message_group, r.event.intro_message_group]);
    for f in fires {
        info!("gameplay triggers: {f:?}");
        match f {
            Fire::Sequence { trigger, sequence, .. } => {
                timeline.play_trigger(&game.store, trigger, sequence, groups.clone())
            }
            Fire::Speech { events, .. } => {
                let queued = speech.as_deref().is_some_and(|l| l.0.lock().unwrap().request_first(&events));
                if !queued {
                    info!("gameplay triggers: speech {events:?}: the speech manager is not loaded");
                }
            }
            Fire::AiPlayers { trigger, .. } | Fire::Roadblocks { trigger, .. } => {
                let kind = if matches!(f, Fire::AiPlayers { .. }) { "AI players" } else { "roadblocks" };
                if t.logged.insert(format!("{kind} {trigger}")) {
                    info!("gameplay triggers: {kind} of trigger {trigger} not carried out yet");
                }
            }
        }
    }
}
