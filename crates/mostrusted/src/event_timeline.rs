//! The current event's `Timeline` sequences (PLAN M8.6; decomp.md "The event's Timeline sequences"): the game-mode
//! manager starts every sequence of the event record's `Timeline` (+0xa0, count +0xe6) when the mode timer starts
//! (`FUN_006f0f80` → `FUN_00a45070` on the general sequence manager, the handles kept at GMM+0x2e50) and stops them at
//! the mode reset and the event change (`FUN_006e6440` → `FUN_00a44d50`). The logic makes those calls
//! (`mw_logic::natives::game_modes::start_timer` / `stop_timeline`: `Request::PlayEventTimeline` /
//! `StopEventTimeline`); here each sequence is read (on a thread), its clock runs with its timeline controllers on
//! the game clock, and its items reach the item players: `cutscene_spawns.rs` (`GameplaySequenceItem` — the
//! forced heat level of most races —, `SpawnAICopCar`, `SpawnTraffic`, ...) and `sequence_picture.rs` (PostFX).
//!
//! In the install 30 events have a `Timeline` (one or two sequences each, all starting at 0 s): Gameplay type 1
//! (the forced heat level) or 0xe (the heat set to a level's start) with a `SpeechEventSequenceItem`, cops
//! (`SpawnAICopCar`), PostFX with a jump controller. Not carried out: the speech items (not parsed; logged by the
//! sequence reader as other kinds).
//!
//! Model decision: a sequence that has reached its end keeps its clock at the end until the logic stops it (the
//! sequence manager's finished state; the item players then see every item past its end).

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mw_assets::sequence::Sequence;
use mw_assets::timeline::SequenceClock;
use mw_bundle::ResourceId;

pub struct EventTimelinePlugin;

impl Plugin for EventTimelinePlugin {
    fn build(&self, app: &mut App) {
        // After the logic's frame (its requests carried out), before `sequence_picture.rs` (PostUpdate) and the
        // spawn items (`cutscene_spawns.rs`).
        app.init_resource::<EventTimeline>().add_systems(Update, step.after(crate::logic::LogicTick));
    }
}

type Job = Arc<Mutex<Option<Result<Sequence, String>>>>;

/// One `Timeline` sequence the logic started.
struct Run {
    sequence: u32,
    job: Option<Job>,
    seq: Option<Arc<Sequence>>,
    clocks: Option<SequenceClock>,
    clock: f32,
    finished: bool,
}

/// The event's `Timeline` sequences that play (the GMM's handles +0x2e50).
#[derive(Resource, Default)]
pub struct EventTimeline {
    runs: Vec<Run>,
}

impl EventTimeline {
    /// `PlayEventTimeline`: `FUN_00a45070`, the sequence starts from 0 (read from the event's feedback groups
    /// `groups` when the store's index does not hold it).
    pub fn play(&mut self, store: &Arc<mw_assets::AssetStore>, sequence: u32, groups: Vec<u32>) {
        let job: Job = Arc::new(Mutex::new(None));
        let (out, store) = (job.clone(), store.clone());
        let _ = crate::platform::thread::Builder::new().name("mw-event-timeline".into()).spawn(move || {
            let id = ResourceId(0x0100_0000_0000_0000 | u64::from(sequence));
            let got = store
                .sequence(id)
                .or_else(|_| store.find_sequence(&groups, sequence).and_then(|id| store.sequence(id)))
                .map_err(|e| e.to_string());
            *out.lock().unwrap() = Some(got);
        });
        self.runs.push(Run { sequence, job: Some(job), seq: None, clocks: None, clock: 0.0, finished: false });
        info!("event timeline: sequence {sequence} started");
    }

    /// `StopEventTimeline`: `FUN_00a44d50`, the sequence's state 7 (its items end).
    pub fn stop(&mut self, sequence: u32) {
        let before = self.runs.len();
        self.runs.retain(|r| r.sequence != sequence);
        if self.runs.len() != before {
            info!("event timeline: sequence {sequence} stopped");
        }
    }

    /// The loaded sequences with their clocks: (UniqueId, sequence, time).
    pub fn clocks(&self) -> impl Iterator<Item = (u32, &Arc<Sequence>, f32)> {
        self.runs.iter().filter_map(|r| r.seq.as_ref().map(|s| (r.sequence, s, r.clock)))
    }
}

fn step(
    virt: Res<Time<Virtual>>,
    mut timeline: ResMut<EventTimeline>,
    picture: Option<ResMut<crate::sequence_picture::SequencePicture>>,
    logic: Option<NonSend<crate::world_logic::WorldLogic>>,
) {
    // The general sequence manager runs on the game clock (it stops with the pause).
    let dt = if virt.is_paused() { 0.0 } else { virt.delta_secs() };
    timeline.runs.retain_mut(|r| {
        if let Some(job) = &r.job {
            let Some(got) = job.lock().unwrap().take() else { return true };
            r.job = None;
            match got {
                Ok(seq) => {
                    info!("event timeline: sequence {} loaded: {:?}", r.sequence, seq.summary());
                    r.seq = Some(Arc::new(seq));
                }
                Err(e) => {
                    // `FUN_007983b0` finds no such sequence: nothing plays.
                    warn!("event timeline: sequence {}: {e}", r.sequence);
                    return false;
                }
            }
        }
        let Some(seq) = r.seq.clone() else { return true };
        if !r.finished {
            let clocks = r.clocks.get_or_insert_with(|| SequenceClock::new(&seq.clocks, 0.0));
            // A controller's `EnabledBinding` reads the logic's view models.
            let mut holds = |b: &str| logic.as_ref().is_some_and(|l| l.state.borrow().resolve_bool(b) == Some(true));
            let mut env = crate::seq_clock::GameEnv { holds: &mut holds, menu: None, skip: 0 };
            let jumps = clocks.tick(&seq.clocks, dt, &mut env);
            crate::seq_clock::log_jumps("event timeline", &seq, &jumps);
            r.clock = clocks.time();
            if r.clock >= seq.length || clocks.quit() {
                r.finished = true;
                r.clock = r.clock.max(seq.length);
                info!("event timeline: sequence {} finished at {:.2} s", r.sequence, r.clock);
            }
        }
        true
    });
    if let Some(mut picture) = picture {
        for (id, seq, clock) in timeline.clocks() {
            picture.play(&format!("event timeline {id}"), seq, clock);
        }
    }
}
