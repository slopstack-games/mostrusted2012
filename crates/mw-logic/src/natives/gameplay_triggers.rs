//! The event's gameplay triggers (the manager at G+0x2f408; decomp.md "Gameplay triggers"): a `GameplayTrigger`'s
//! input is a world trigger the player's car drives into; firing it runs its outputs (a sequence, speech, AI
//! players, roadblocks). Ported:
//!
//! - [`activate`] (`FUN_006f58b0`, from the event change `FUN_0070b1e0` and `FUN_0070caa0`): the active list +0x44
//!   (count +0x244) = the current event record's `a9d6149c` (+0x7c, count +0xf5) triggers that exist; the fired
//!   list (+0x248, count +0x848) and the queue (+0x850, count +0xfd0) are emptied with the event (`FUN_006e2d..`).
//! - [`queue`] (`FUN_006f5d50`, also `GameplaySequenceItem` type 8 and the scoring actions): unless the trigger has
//!   +0x2b or +0x2c, nothing while the player is crashed (player +0x23f, `FUN_006ba290`); one entry per trigger.
//! - [`step`] (`FUN_006fbfb0`, every frame from `FUN_006abc10`): only with an event (GMM+0x2ce0), the local player's
//!   vehicle (`FUN_006ba2a0`) and the mode timer (GMM+0x2e74). First the re-arms (`FUN_006f21b0`): a fired trigger
//!   with +0x24 = 1 is forgotten once its sequence no longer plays, with 2 once +0x18 s have passed (the stamp
//!   +0x18 × 3e6 ticks, `FUN_00ac6da0`). Then each active trigger not fired is queued when one of its inputs is in
//!   the car's hit list (`FUN_005372f0`, up to 16). Then each queued trigger: gone or fired → dropped; while the
//!   player is crashed one with +0x2c waits, one with +0x2b goes on; its `Predicate` (`FUN_0063e470`) holding, it
//!   fires ([`Fire`], `FUN_006f9670`) and is appended to the fired list; dropped either way.
//! - The fire (`FUN_006f9670`) per output whose `Predicate` holds: the AI players (queued, `FUN_006e8ae0`), the
//!   roadblocks, the first `SequenceOutput` on the player its `SequenceType` names, the speech list
//!   (`FUN_006e0a90`). The first output's sequence is the one the re-arm (mode 1) watches.
//!
//! Not ported (MODEL): the reset sweep of `FUN_006fbfb0` (after a reset, the triggers within 32 m of where it
//! began, `FUN_004b3b10`, are hit too); inputs of trigger kind 0x19 (`FUN_0069fd30` → message 3 `FUN_00716130`).

use std::collections::HashMap;

use super::LogicState;
use super::trigger_boxes::{CarBox, TriggerBox, hits};

/// A trigger's output (`GameplayTrigger.Output`, the fields the fire reads).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Output {
    pub predicate: String,
    /// +0x14 / +0x29: speech event ids.
    pub speech: Vec<u32>,
    /// +0x18 / +0x24: AI players.
    pub ai_players: usize,
    /// The first `SequenceOutput`: (sequence, `SequenceType`).
    pub sequence: Option<(u32, u16)>,
    /// +0x20 / +0x28: roadblocks.
    pub roadblocks: usize,
}

/// A `GameplayTrigger` (filled by the runtime from the install).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Def {
    pub id: u32,
    pub predicate: String,
    /// `Input[].Trigger`.
    pub inputs: Vec<u32>,
    pub outputs: Vec<Output>,
    /// +0x24: 1 re-arms when its sequence ended, 2 after `rearm_time`, else never in the event.
    pub rearm: u16,
    pub rearm_time: f32,
    /// +0x2b / +0x2c.
    pub while_crashed: bool,
    pub after_crash: bool,
}

/// A fired trigger (+0x248, 0x18 B each).
#[derive(Debug, Clone, PartialEq)]
pub struct Fired {
    pub id: u32,
    /// The first played output's sequence (+0x258).
    pub sequence: Option<u32>,
    /// Re-arm mode 2: seconds left (+0x250).
    pub timer: f32,
}

/// What a fire asks the runtime for.
#[derive(Debug, Clone, PartialEq)]
pub enum Fire {
    /// A `SequenceOutput`: play `sequence` on the player of `kind`.
    Sequence { trigger: u32, sequence: u32, kind: u16 },
    /// `FUN_006e0a90`: the first of these speech events whose conditions hold.
    Speech { trigger: u32, events: Vec<u32> },
    /// AI players to spawn (`FUN_006ce030` through the queue `FUN_006e8ae0`; not carried out yet).
    AiPlayers { trigger: u32, count: usize },
    /// Roadblocks to place (`FUN_008456d0`, `FUN_0083f030`; not carried out yet).
    Roadblocks { trigger: u32, count: usize },
}

/// What the runtime supplies each frame.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    /// The local player's car (`None`: no vehicle, `FUN_006ba2a0` false).
    pub car: Option<CarBox>,
    /// The world triggers the active triggers' inputs name, by id.
    pub boxes: HashMap<u32, TriggerBox>,
    /// Sequences of fired triggers still playing (the re-arm of mode 1).
    pub playing: Vec<u32>,
}

/// The manager's state.
#[derive(Debug, Clone, Default)]
pub struct GameplayTriggers {
    pub defs: HashMap<u32, Def>,
    /// +0x44: the current event's triggers.
    pub active: Vec<u32>,
    pub fired: Vec<Fired>,
    /// +0x850: queued triggers.
    pub queued: Vec<u32>,
    pub inputs: Inputs,
}

/// `FUN_006f58b0` with the event's trigger ids (`a9d6149c`): the active list; the fired list and the queue empty.
pub fn activate(s: &mut LogicState, triggers: &[u32]) {
    let g = &mut s.gameplay_triggers;
    g.active = triggers.iter().copied().filter(|id| g.defs.contains_key(id)).collect();
    g.fired.clear();
    g.queued.clear();
}

fn crashed(s: &LogicState) -> bool {
    s.players.local_player().is_some_and(|p| p.crashed)
}

/// `FUN_006f5d50`(trigger).
pub fn queue(s: &mut LogicState, id: u32) {
    let crashed = crashed(s);
    let g = &mut s.gameplay_triggers;
    let Some(d) = g.defs.get(&id) else { return };
    if !d.while_crashed && !d.after_crash && crashed {
        return;
    }
    if !g.queued.contains(&id) {
        g.queued.push(id);
    }
}

fn holds(s: &LogicState, binding: &str) -> bool {
    binding.is_empty() || s.resolve_bool(binding) == Some(true)
}

/// `FUN_006fbfb0`: one frame (`dt` on the game clock); returns what fired.
pub fn step(s: &mut LogicState, dt: f32) -> Vec<Fire> {
    let mut out = Vec::new();
    if s.game_modes.current_event == 0 || s.gameplay_triggers.inputs.car.is_none() || !s.game_modes.timer_running {
        return out;
    }
    // `FUN_006f21b0`: the re-arms.
    {
        let g = &mut s.gameplay_triggers;
        let (defs, playing) = (&g.defs, &g.inputs.playing);
        g.fired.retain_mut(|f| match defs.get(&f.id).map_or(0, |d| d.rearm) {
            1 => f.sequence.is_some_and(|q| playing.contains(&q)),
            2 => {
                f.timer -= dt;
                f.timer >= 1.0 / 3e6
            }
            _ => true,
        });
    }
    // The car's hits queue the active triggers not fired.
    let car = s.gameplay_triggers.inputs.car.unwrap_or_default();
    let mut hit = Vec::new();
    {
        let g = &s.gameplay_triggers;
        for id in &g.active {
            if g.fired.iter().any(|f| f.id == *id) {
                continue;
            }
            let Some(d) = g.defs.get(id) else { continue };
            if d.inputs.iter().any(|t| g.inputs.boxes.get(t).is_some_and(|b| hits(&car, b))) {
                hit.push(*id);
            }
        }
    }
    for id in hit {
        queue(s, id);
    }
    // The queue.
    let crashed = crashed(s);
    let queued = std::mem::take(&mut s.gameplay_triggers.queued);
    let mut keep = Vec::new();
    for id in queued {
        let Some(d) = s.gameplay_triggers.defs.get(&id).cloned() else { continue };
        if crashed && d.after_crash {
            keep.push(id);
            continue;
        }
        if crashed && !d.while_crashed {
            continue;
        }
        if s.gameplay_triggers.fired.iter().any(|f| f.id == id) || !holds(s, &d.predicate) {
            continue;
        }
        let first = fire(s, &d, &mut out);
        s.gameplay_triggers.fired.push(Fired { id, sequence: first, timer: d.rearm_time });
    }
    s.gameplay_triggers.queued.extend(keep);
    out
}

/// `FUN_006f9670`: the outputs whose `Predicate` holds; returns the first played sequence.
fn fire(s: &LogicState, d: &Def, out: &mut Vec<Fire>) -> Option<u32> {
    let mut first = None;
    for o in &d.outputs {
        if !holds(s, &o.predicate) {
            continue;
        }
        if o.ai_players > 0 {
            out.push(Fire::AiPlayers { trigger: d.id, count: o.ai_players });
        }
        if o.roadblocks > 0 {
            out.push(Fire::Roadblocks { trigger: d.id, count: o.roadblocks });
        }
        if let Some((sequence, kind)) = o.sequence.filter(|(q, _)| *q != 0) {
            out.push(Fire::Sequence { trigger: d.id, sequence, kind });
            first.get_or_insert(sequence);
        }
        let events: Vec<u32> = o.speech.iter().copied().filter(|&e| e != 0).collect();
        if !events.is_empty() {
            out.push(Fire::Speech { trigger: d.id, events });
        }
    }
    first
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: u32, input: u32, sequence: u32, rearm: u16) -> Def {
        Def {
            id,
            inputs: vec![input],
            outputs: vec![Output { sequence: Some((sequence, 3)), ..Default::default() }],
            rearm,
            rearm_time: 2.0,
            ..Default::default()
        }
    }

    fn state() -> LogicState {
        let mut s = LogicState::default();
        s.game_modes.current_event = 7;
        s.game_modes.timer_running = true;
        s.players.local = Some(0);
        s.players.slots[0] = Some(Default::default());
        for d in [def(1, 100, 11, 0), def(2, 200, 22, 2), def(3, 300, 33, 1)] {
            s.gameplay_triggers.defs.insert(d.id, d);
        }
        activate(&mut s, &[1, 2, 3, 9]);
        s
    }

    fn put_car_in(s: &mut LogicState, trigger: Option<u32>) {
        let g = &mut s.gameplay_triggers;
        g.inputs.boxes.clear();
        for id in [100, 200, 300] {
            let x = if Some(id) == trigger { 0.0 } else { 1000.0 };
            g.inputs.boxes.insert(id, TriggerBox::from_quat(id, [x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [4.0; 3], false));
        }
        g.inputs.car = Some(CarBox::level([0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.7, 2.2]));
    }

    #[test]
    fn a_hit_fires_once_and_rearms_by_mode() {
        let mut s = state();
        assert_eq!(s.gameplay_triggers.active, [1, 2, 3]);
        put_car_in(&mut s, Some(100));
        assert_eq!(step(&mut s, 0.1), [Fire::Sequence { trigger: 1, sequence: 11, kind: 3 }]);
        assert!(step(&mut s, 0.1).is_empty(), "mode 0: once per event");
        put_car_in(&mut s, Some(200));
        assert_eq!(step(&mut s, 0.1).len(), 1);
        assert!(step(&mut s, 1.0).is_empty(), "mode 2: 1 s of 2 left");
        // The re-arm runs first in the update (`FUN_006f21b0`), so the car still in the box fires it again.
        assert_eq!(step(&mut s, 1.5).len(), 1, "mode 2: again after its time");
        put_car_in(&mut s, Some(300));
        s.gameplay_triggers.inputs.playing = vec![33];
        assert_eq!(step(&mut s, 0.1).len(), 1);
        assert!(step(&mut s, 0.1).is_empty(), "mode 1: not while its sequence plays");
        s.gameplay_triggers.inputs.playing.clear();
        assert_eq!(step(&mut s, 0.1).len(), 1, "mode 1: again once it ended");
    }

    #[test]
    fn nothing_without_the_mode_timer_or_while_crashed() {
        let mut s = state();
        put_car_in(&mut s, Some(100));
        s.game_modes.timer_running = false;
        assert!(step(&mut s, 0.1).is_empty());
        s.game_modes.timer_running = true;
        s.players.slots[0].as_mut().unwrap().crashed = true;
        assert!(step(&mut s, 0.1).is_empty());
        s.players.slots[0].as_mut().unwrap().crashed = false;
        assert_eq!(step(&mut s, 0.1).len(), 1);
    }
}
