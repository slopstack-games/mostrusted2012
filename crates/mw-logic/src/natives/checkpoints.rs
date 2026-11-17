//! The checkpoint manager of `CHECKPTS` ("Checkpoints", vtable 0xfd94a0, 0x200 B of instance state; decomp.md
//! "Races" and "The checkpoint manager"): which gate is next, passing it, laps, the position, split times, the
//! gate triggers and markers, the finish. Ported from the code: enter 0x749670, enter update 0x75d8d0, update
//! `FUN_0075da20` (with `FUN_00759cf0` for the AI racers, `FUN_00749bb0` position, `FUN_00749570` lead timer,
//! `FUN_00759860` / `FUN_00759b70` gate activation, `FUN_007545b0` marker looks), exit 0x75d960 (`FUN_00759fa0`).
//!
//! **What the runtime fills each frame** ([`CheckpointInputs`]): the route entries the distance-to-finish manager
//! built (`FUN_00645290`: entry 0 the spawn, then every gate of every lap, DTF+0x10 / +0x510 / +0x660 / +0x664),
//! the event's `CheckpointInfo` entries, the racers' boxes and reset state, the distance-to-finish per slot
//! (DTF+0xdf0; engine: road paths), the race clock (GMM+0x2d28, [`GameModeManager::word_2d28`](super::game_modes::GameModeManager)),
//! and the few engine flags the update reads (each field names its source).
//!
//! **What the logic decides** ([`CheckpointProgress`]): the checkpoint counter (+0x50; the target is route entry
//! counter + 1), the lap (+0x54), the position (+0x58), the AI racers' counters (+0xb4 + slot), the split times
//! (player +0x4c0), the finish (exit through `EXIT`, the local player's final data via `FUN_006b9d00`, GMM+0x2e78
//! cleared when the place is too low). **What the runtime carries out**: the [`CheckpointOut`] requests (gate
//! triggers on / off, markers, GPS, the distance manager's target, score events, AI racers done).
//!
//! The wrong-way / missed-checkpoint machine (`FUN_006585b0`) belongs to the distance manager and stays in the
//! runtime (`race.rs`); the update only reads its outcome ([`CheckpointInputs::reroute`]).

use super::LogicState;
use super::players::SLOTS;
use super::state::Request;
use super::trigger_boxes::{CarBox, TriggerBox, V3, hits};

/// One route entry (DTF+0x10, 0x14 B: position, trigger id, cumulative distance).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RouteEntry {
    /// The entry point (a gate's box centre lowered by half its height; the spawn point for entry 0).
    pub position: V3,
    /// The trigger id (+0xc; entry 0: the `SpawnPosition` point).
    pub trigger: u32,
    /// The gate's box (none for entry 0).
    pub gate: Option<TriggerBox>,
}

/// One `CheckpointInfo` of the event (OfflineEvent +0xc8, 0x14 B each, count +0xda).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CheckpointInfo {
    /// +0xc `Sequence`: played when the gate is passed (0 = none).
    pub sequence: u32,
    /// +0x10 `Type` (1–4 pick another marker look, `FUN_007545b0`; 0 in all the data).
    pub kind: u16,
    /// +0x12 `a1ec8d81`: an AI racer passing the gate plays the sequence too.
    pub ai_sequence: bool,
    /// +0x13 `eb1ecc26`: passing it stores a split time.
    pub store_split: bool,
}

/// The marker looks (`FUN_007545b0`): `CheckpointConfig` (GameLogicConfig +0x2a8) +0x28 `NormalPoi` (target), +0x24
/// (next gate), +0x0c / +0x30 / +0x1c / +0x20 (target of `Type` 1–4); for the finish `MapPoiConfig` (+0x2ac) +0x28 /
/// +0x2c. Defaults: the values in the install's config (decomp.md "Races", "Markers").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkerKeys {
    pub target: u32,
    pub next: u32,
    pub by_type: [u32; 4],
    pub finish: u32,
    pub finish_next: u32,
}

impl Default for MarkerKeys {
    fn default() -> Self {
        MarkerKeys { target: 1_013_519, next: 1_013_518, by_type: [0; 4], finish: 1_013_510, finish_next: 1_695_650 }
    }
}

/// A racer as the update reads it (player record and its vehicle).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RaceCar {
    /// The vehicle's box (the overlap job's query, `FUN_00536a10`).
    pub car: CarBox,
    /// The player has a vehicle (+0xa0 ≠ −1).
    pub has_vehicle: bool,
    /// The world's reset flag of its vehicle (G+0x25694 + vehicle).
    pub resetting: bool,
    /// `FUN_00661da0`(slot): the distance manager's target after a reset (the first entry the car is not past;
    /// `None` = its current target, no change).
    pub reset_target: Option<usize>,
    /// DTF+0xdf0 + slot: the distance to the finish (`None` = not computed, `DAT_010c53c0`).
    pub to_finish: Option<f32>,
}

/// What the runtime supplies each frame (see the module doc).
#[derive(Debug, Clone, Default)]
pub struct CheckpointInputs {
    /// DTF+0x10, count +0x510.
    pub route: Vec<RouteEntry>,
    /// DTF+0x660: gates per lap (the event's `Checkpoints` count, also the `CheckpointInfo` index modulus +0xdc).
    pub per_lap: usize,
    /// The event's `CheckpointInfo` entries.
    pub infos: Vec<CheckpointInfo>,
    /// The distance manager is ready (its vtable +8 and +0xc; the enter update waits for it).
    pub loaded: bool,
    /// GameMode +0x6e (`f1123d7b`): the local player's gate markers and score events.
    pub markers: bool,
    pub marker_keys: MarkerKeys,
    /// OfflineEvent +0xe9 (`c8f26580`): only first place wins.
    pub first_place_only: bool,
    /// GMM+0x2e8b (`FUN_006e03e0`): the lead timer runs (read on enter).
    pub lead_events: bool,
    /// GMM+0x2e48: a split is shown when a gate stores one (GMM+0x2e7d).
    pub split_display: bool,
    /// G+0x2f458 +0x85c == 3: the finish waits.
    pub finish_held: bool,
    /// `FUN_00656220` (the wrong-way / missed state asks for a new route) while the GPS is idle (UIM+0x23d34 or
    /// +0x28184 clear): a new GPS route to the target (`FUN_007684d0`, path type 9).
    pub reroute: bool,
    /// The game clock's frame time (`FUN_0046ee10`: +0x14 × +0x10, 0 while +0x1c is clear).
    pub dt: f32,
    /// The racers by player slot (the local player's and PM+0xa664's).
    pub cars: [Option<RaceCar>; SLOTS],
    /// PM+0xa664 (count +0xa694): the AI racers' slots, in list order.
    pub racers: Vec<usize>,
}

/// One gate's reference count (+0xdc, 0xc B each, count +0x1fc): gate index within the lap, its marker, users.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GateRef {
    pub lap_index: usize,
    /// The entry its marker stands at (the marker handle +4; `None` = 0x7fffffff).
    pub marker: Option<usize>,
    pub count: i8,
}

/// The lead timer (+0x48 time, +0x4c enabled, +0x4d fired; `FUN_00749570`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LeadTimer {
    pub time: f32,
    pub enabled: bool,
    pub fired: bool,
}

/// The behaviour's state (the logic decides it).
#[derive(Debug, Clone, PartialEq)]
pub struct CheckpointProgress {
    /// +0x50: the local player's checkpoint counter (target = route entry `cp + 1`).
    pub cp: usize,
    /// +0x54: the lap, from 1.
    pub lap: u32,
    /// +0x58: the position (0 before the first update).
    pub position: u32,
    /// +0x5c: the AI racer count on enter (PM+0xa694): racers removed since count as ahead.
    pub racers_at_start: usize,
    /// +0x60 + 4 × slot: the cached distances to the finish (FLT_MAX on enter).
    pub distances: [f32; SLOTS],
    /// +0xb4 + slot: the AI racers' checkpoint counters.
    pub ai_cp: [usize; SLOTS],
    /// +0xc0 + slot: the AI racers' reset flags last frame.
    pub ai_resetting: [bool; SLOTS],
    /// +0xd8: the local player's reset flag last frame.
    pub resetting: bool,
    /// +0xdc / +0x1fc.
    pub gates: Vec<GateRef>,
    pub lead: LeadTimer,
    /// Player +0x4c0 (count +0x5b0): the split times (race clock ticks).
    pub splits: Vec<u64>,
    /// AI racers that reached their last gate (racer +0x5d7).
    pub ai_done: [bool; SLOTS],
    /// The update returned 1 (exit through `EXIT`).
    pub finished: bool,
}

impl Default for CheckpointProgress {
    fn default() -> Self {
        CheckpointProgress {
            cp: 0,
            lap: 1,
            position: 0,
            racers_at_start: 0,
            distances: [f32::MAX; SLOTS],
            ai_cp: [0; SLOTS],
            ai_resetting: [false; SLOTS],
            resetting: false,
            gates: Vec::new(),
            lead: LeadTimer::default(),
            splits: Vec::new(),
            ai_done: [false; SLOTS],
            finished: false,
        }
    }
}

/// The checkpoint manager: runtime inputs and the logic's progress.
#[derive(Debug, Clone, Default)]
pub struct Checkpoints {
    pub inputs: CheckpointInputs,
    pub progress: CheckpointProgress,
}

/// A score / speech event the update sends (`FUN_006552a0` / `FUN_006a8050`, the config's event id).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaceEvent {
    /// A gate other than the finish passed with markers on: `ScoringConfig` (global config +0x44) +0x60.
    CheckpointPassed,
    /// First place lost: the config at global +0x48, +0x12c.
    LostLead,
    /// First place taken: +0x2dc.
    TookLead,
    /// First place held 1.5 s (`DAT_00fd2338`) while the mode timer runs: +0x2e0.
    HeldLead,
}

/// What `CHECKPTS` asks the engine to do.
#[derive(Debug, Clone, PartialEq)]
pub enum CheckpointOut {
    /// `FUN_007160b0`: switch the gate's trigger on / off (reference counted per gate of a lap).
    Trigger {
        trigger: u32,
        on: bool,
    },
    /// `FUN_00790c70`: a point of interest of look `key` at route entry `entry` (`FUN_00749930`'s matrix).
    Marker {
        entry: usize,
        key: u32,
    },
    /// `FUN_00790da0`: remove the marker at `entry`.
    MarkerRemove {
        entry: usize,
    },
    /// `FUN_007a0e40`: the GPS line over these route segments (empty: cleared, `FUN_00759fa0`).
    Gps {
        entries: Vec<usize>,
    },
    /// `FUN_00665350`(slot, entry, 1): the distance manager's target for the slot (DTF+0xd90).
    Target {
        slot: usize,
        entry: usize,
    },
    /// `FUN_007684d0`: a new GPS route from the car to route entry `entry` (path type 9).
    Reroute {
        entry: usize,
    },
    /// `FUN_007983b0` / `FUN_00a44f30`: play a `CheckpointInfo` sequence.
    Sequence {
        slot: usize,
        sequence: u32,
    },
    Event(RaceEvent),
    /// `FUN_00843030`(AI handle, 3, 0) and racer +0x5d7 = 1: the AI racer stops racing.
    AiDone {
        slot: usize,
    },
}

fn out(s: &mut LogicState, o: CheckpointOut) {
    s.requests.push(Request::Checkpoint(o));
}

/// The requests `CHECKPTS` makes that the logic carries out (called from [`super::LogicHost::apply`]).
pub fn apply(s: &mut LogicState, r: &Request) {
    match r {
        Request::CheckpointsEnter => enter(s),
        Request::CheckpointsPlace => place(s),
        Request::CheckpointsStep => {
            if update(s) {
                s.checkpoints.progress.finished = true;
            }
        }
        Request::CheckpointsLeave => leave(s),
        _ => {}
    }
}

/// Enter (0x749670): gate list empty, lead timer reset and enabled by GMM+0x2e8b, counter 0, lap 1, position 0,
/// +0x5c = the AI racer count, the AI counters and reset bytes 0, distances FLT_MAX. (It then has the distance
/// manager build the route, `FUN_00645290`: the runtime's [`CheckpointInputs::route`].)
pub fn enter(s: &mut LogicState) {
    let c = &mut s.checkpoints;
    c.progress = CheckpointProgress {
        lead: LeadTimer { time: 0.0, enabled: c.inputs.lead_events, fired: false },
        racers_at_start: c.inputs.racers.len(),
        ..Default::default()
    };
}

/// The enter update's placement (0x75d8d0, once the distance manager is ready): the first gate for the local
/// player, then for every occupied AI slot 12..23.
pub fn place(s: &mut LogicState) {
    // (Only with entries; a runtime that answers the count itself without filling the route gets nothing placed.)
    if count(s) == 0 {
        return;
    }
    if let Some(local) = s.players.local {
        activate(s, local, 0, true);
    }
    for slot in super::players::FIRST_AI_SLOT..SLOTS {
        if s.players.slots[slot].is_some() {
            activate(s, slot, 0, true);
        }
    }
}

/// Exit (0x75d960): every gate's marker and trigger released (`FUN_00759fa0`, the GPS cleared), then every AI
/// racer still in a car and not a cop is done (+0x5d7) and gets goal 3.
pub fn leave(s: &mut LogicState) {
    let gates = std::mem::take(&mut s.checkpoints.progress.gates);
    for g in gates {
        if let Some(entry) = g.marker {
            out(s, CheckpointOut::MarkerRemove { entry });
        }
        if g.count > 0 {
            let trigger = entry_trigger(s, g.lap_index + 1);
            out(s, CheckpointOut::Trigger { trigger, on: false });
        }
    }
    out(s, CheckpointOut::Gps { entries: Vec::new() });
    for slot in super::players::FIRST_AI_SLOT..SLOTS {
        let racing = s.players.slots[slot].as_ref().is_some_and(|p| p.vehicle != -1 && !p.cop);
        if racing {
            s.checkpoints.progress.ai_done[slot] = true;
            out(s, CheckpointOut::AiDone { slot });
        }
    }
}

fn entry_trigger(s: &LogicState, entry: usize) -> u32 {
    s.checkpoints.inputs.route.get(entry).map_or(0, |e| e.trigger)
}

fn per_lap(s: &LogicState) -> usize {
    s.checkpoints.inputs.per_lap.max(1)
}

fn count(s: &LogicState) -> usize {
    s.checkpoints.inputs.route.len()
}

/// `FUN_00749860`: `CheckpointInfo[cp % per lap]` (`None` past the infos).
fn info(s: &LogicState, cp: usize) -> Option<CheckpointInfo> {
    s.checkpoints.inputs.infos.get(cp % per_lap(s)).copied()
}

/// `FUN_007545b0`(cp, active): the marker look for the gate at entry cp + 1.
fn marker_key(s: &LogicState, cp: usize, active: bool) -> u32 {
    let k = s.checkpoints.inputs.marker_keys;
    if cp + 2 == count(s) {
        return if active { k.finish } else { k.finish_next };
    }
    if !active {
        return k.next;
    }
    match info(s, cp).map(|i| i.kind) {
        Some(t @ 1..=4) => k.by_type[t as usize - 1],
        _ => k.target,
    }
}

/// `FUN_00759860`(slot, cp, active): the gate at entry cp + 1 gets a user (switched on with its first), the local
/// player's marker (target look, and the gate after it with the next-gate look) and GPS line when markers are on,
/// and the slot's distance target.
pub fn activate(s: &mut LogicState, slot: usize, cp: usize, active: bool) {
    let li = cp % per_lap(s);
    let i = match s.checkpoints.progress.gates.iter().position(|g| g.lap_index == li) {
        Some(i) => i,
        None => {
            s.checkpoints.progress.gates.push(GateRef { lap_index: li, marker: None, count: 0 });
            s.checkpoints.progress.gates.len() - 1
        }
    };
    let trigger = entry_trigger(s, cp + 1);
    if s.checkpoints.progress.gates[i].count == 0 && active {
        out(s, CheckpointOut::Trigger { trigger, on: true });
    }
    if active {
        s.checkpoints.progress.gates[i].count = s.checkpoints.progress.gates[i].count.wrapping_add(1);
    }
    if Some(slot) == s.players.local && s.checkpoints.inputs.markers {
        if let Some(entry) = s.checkpoints.progress.gates[i].marker.take() {
            out(s, CheckpointOut::MarkerRemove { entry });
        }
        let key = marker_key(s, cp, active);
        out(s, CheckpointOut::Marker { entry: cp + 1, key });
        s.checkpoints.progress.gates[i].marker = Some(cp + 1);
        if !active {
            return;
        }
        let n = count(s);
        if cp + 1 < n.saturating_sub(1) {
            activate(s, slot, cp + 1, false);
        }
        // The next min(8, per lap − 1) segments from the target on, not past the last entry.
        let want = (per_lap(s) - 1).clamp(1, 8);
        let entries: Vec<usize> = (cp + 1..n).take(want).collect();
        out(s, CheckpointOut::Gps { entries });
    }
    if active {
        out(s, CheckpointOut::Target { slot, entry: cp + 1 });
    }
}

/// `FUN_00759b70`(slot, cp): the gate at entry cp + 1 loses a user (its local marker removed); with none left
/// its trigger goes off and, unless it is the local player's next gate, its entry is dropped (the last moved in).
pub fn release(s: &mut LogicState, slot: usize, cp: usize) {
    let pl = per_lap(s);
    let li = cp % pl;
    let Some(i) = s.checkpoints.progress.gates.iter().position(|g| g.lap_index == li) else { return };
    let g = &mut s.checkpoints.progress.gates[i];
    g.count = g.count.wrapping_sub(1);
    if Some(slot) == s.players.local && s.checkpoints.inputs.markers {
        let marker = s.checkpoints.progress.gates[i].marker.take();
        if let Some(entry) = marker {
            out(s, CheckpointOut::MarkerRemove { entry });
        }
    }
    if s.checkpoints.progress.gates[i].count == 0 {
        let trigger = entry_trigger(s, cp + 1);
        out(s, CheckpointOut::Trigger { trigger, on: false });
        if li != (s.checkpoints.progress.cp + 1) % pl {
            s.checkpoints.progress.gates.swap_remove(i);
        }
    }
}

/// The race clock (GMM+0x2d28 / +0x2d2c, ticks of 1/3,000,000 s).
fn clock(s: &LogicState) -> u64 {
    u64::from(s.game_modes.word_2d28) | (u64::from(s.game_modes.word_2d2c) << 32)
}

/// The update (`FUN_0075da20`); true = finished (exit through `EXIT`).
pub fn update(s: &mut LogicState) -> bool {
    let n = count(s);
    if n == 0 {
        return true;
    }
    let Some(local) = s.players.local else { return false };
    let car = s.checkpoints.inputs.cars[local];
    if s.checkpoints.inputs.loaded {
        let resetting = car.is_some_and(|c| c.resetting);
        if !resetting && s.checkpoints.progress.resetting {
            // `FUN_00661da0`: after a reset the target moves on when the car is already past it.
            let cp = s.checkpoints.progress.cp;
            let target = car.and_then(|c| c.reset_target).unwrap_or(cp + 1);
            if cp + 1 < target {
                release(s, local, cp);
                s.checkpoints.progress.cp = target - 1;
                activate(s, local, target - 1, true);
            }
        }
        s.checkpoints.progress.resetting = resetting;
        if s.checkpoints.inputs.reroute {
            let entry = s.checkpoints.progress.cp + 1;
            out(s, CheckpointOut::Reroute { entry });
        }
    }
    update_ai(s);
    let ticks = clock(s);
    let last = n - 1;
    if s.checkpoints.progress.cp < last {
        update_position(s, local);
        let position = s.checkpoints.progress.position;
        lead_timer(s, position);
        let cp = s.checkpoints.progress.cp;
        let hit = match (car, s.checkpoints.inputs.route[cp + 1].gate) {
            (Some(c), Some(gate)) if c.has_vehicle => hits(&c.car, &gate),
            _ => false,
        };
        if hit {
            let info = info(s, cp);
            let store = info.is_some_and(|i| i.store_split);
            s.game_modes.split_shown = store && s.checkpoints.inputs.split_display;
            if store {
                s.checkpoints.progress.splits.push(ticks);
            }
            if cp + 1 < last && s.checkpoints.inputs.markers {
                out(s, CheckpointOut::Event(RaceEvent::CheckpointPassed));
            }
            release(s, local, cp);
            if let Some(sequence) = info.map(|i| i.sequence).filter(|&q| q != 0) {
                out(s, CheckpointOut::Sequence { slot: local, sequence });
            }
            let cp = cp + 1;
            s.checkpoints.progress.cp = cp;
            if cp < last {
                if cp.is_multiple_of(per_lap(s)) {
                    s.checkpoints.progress.lap += 1;
                }
                activate(s, local, cp, true);
            }
        }
    }
    if s.checkpoints.progress.cp < last || s.checkpoints.inputs.finish_held {
        return false;
    }
    // `FUN_006b9d00` on the local player: place and finish time; the won flag cleared when the place is too low.
    let place = s.checkpoints.progress.position;
    if let Some(p) = s.players.local_player_mut() {
        p.final_5b8 = place as i32;
        p.final_5c8 = ticks as u32;
        p.final_5cc = (ticks >> 32) as u32;
        p.final_5d0 = false;
    }
    let limit = if s.checkpoints.inputs.first_place_only { 1 } else { 3 };
    if place > limit {
        s.game_modes.won = false;
    }
    true
}

/// `FUN_00759cf0`: the AI racers' gates (only the target, as for the player); the last one makes the racer done.
fn update_ai(s: &mut LogicState) {
    if !s.checkpoints.inputs.loaded {
        return;
    }
    let last = count(s) - 1;
    for slot in s.checkpoints.inputs.racers.clone() {
        let Some(car) = s.checkpoints.inputs.cars.get(slot).copied().flatten() else { continue };
        if !car.has_vehicle || s.checkpoints.progress.ai_cp[slot] >= last {
            continue;
        }
        let was = s.checkpoints.progress.ai_resetting[slot];
        s.checkpoints.progress.ai_resetting[slot] = car.resetting;
        if !car.resetting && was {
            let cp = s.checkpoints.progress.ai_cp[slot];
            let target = car.reset_target.unwrap_or(cp + 1);
            if cp + 1 < target {
                release(s, slot, cp);
                s.checkpoints.progress.ai_cp[slot] = target - 1;
                activate(s, slot, target - 1, true);
            }
        }
        let cp = s.checkpoints.progress.ai_cp[slot];
        let Some(gate) = s.checkpoints.inputs.route[cp + 1].gate else { continue };
        if !hits(&car.car, &gate) {
            continue;
        }
        if let Some(i) = info(s, cp).filter(|i| i.ai_sequence && i.sequence != 0) {
            out(s, CheckpointOut::Sequence { slot, sequence: i.sequence });
        }
        release(s, slot, cp);
        let cp = cp + 1;
        s.checkpoints.progress.ai_cp[slot] = cp;
        if cp < last {
            activate(s, slot, cp, true);
        } else {
            s.checkpoints.progress.ai_done[slot] = true;
            out(s, CheckpointOut::AiDone { slot });
        }
    }
}

/// `FUN_00749bb0`: position = (racers on enter − racers now) + 1 + the racers ahead: finished (their counter at
/// the last entry), or closer to the finish (cached distances; equal is not ahead), or — the original's quirk —
/// while the player's counter is below the racer's reset byte. Lead events on a change to or from first place.
fn update_position(s: &mut LogicState, local: usize) {
    let last = count(s) - 1;
    let p = &mut s.checkpoints.progress;
    let inputs = &s.checkpoints.inputs;
    if let Some(d) = inputs.cars[local].and_then(|c| c.to_finish) {
        p.distances[local] = d;
    }
    let mut position = p.racers_at_start as i64 - inputs.racers.len() as i64 + 1;
    for &slot in &inputs.racers {
        if p.ai_cp[slot] < last {
            if let Some(d) = inputs.cars[slot].and_then(|c| c.to_finish) {
                p.distances[slot] = d;
            }
            if p.cp < p.ai_resetting[slot] as usize || p.distances[slot] < p.distances[local] {
                position += 1;
            }
        } else {
            position += 1;
        }
    }
    let position = position.max(0) as u32;
    let old = p.position;
    p.position = position;
    if old == 1 && position > 1 {
        out(s, CheckpointOut::Event(RaceEvent::LostLead));
    } else if position == 1 && old >= 2 {
        out(s, CheckpointOut::Event(RaceEvent::TookLead));
    }
    // `FUN_006b9cd0`(position, lap, distance) on the local player (+0x5b8 = position).
    if let Some(pl) = s.players.local_player_mut() {
        pl.final_5b8 = position as i32;
    }
}

/// `FUN_00749570`: first place held longer than 1.5 s (`DAT_00fd2338`) while the mode timer runs (GMM+0x2e74) sends
/// its event once; any other place resets the timer.
fn lead_timer(s: &mut LogicState, position: u32) {
    const HELD: f32 = 1.5;
    let timer_running = s.game_modes.timer_running;
    let dt = s.checkpoints.inputs.dt;
    let l = &mut s.checkpoints.progress.lead;
    if !l.enabled || l.fired {
        return;
    }
    if position == 1 {
        if timer_running {
            l.time += dt;
        }
    } else if position > 1 {
        l.time = 0.0;
        return;
    }
    if l.time > HELD {
        l.fired = true;
        out(s, CheckpointOut::Event(RaceEvent::HeldLead));
    }
}

/// The checkpoint queries `CHECKPTS` asks (`Query::CheckpointCount` / `CheckpointsLoaded` / `CheckpointsFinished`).
pub fn count_query(s: &LogicState) -> u64 {
    count(s) as u64
}

/// A synthetic route for tools and tests: entry 0 at the spawn, then the gates of every lap in order.
pub fn route_from_gates(spawn: V3, gates: &[TriggerBox], laps: usize) -> Vec<RouteEntry> {
    let mut route = vec![RouteEntry { position: spawn, trigger: 0, gate: None }];
    for _ in 0..laps.max(1) {
        route.extend(gates.iter().map(|g| RouteEntry { position: g.ground(), trigger: g.id, gate: Some(*g) }));
    }
    route
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::natives::players::Player;

    fn gate(id: u32, z: f32) -> TriggerBox {
        TriggerBox { id, centre: [0.0, 2.0, z], size: [20.0, 4.0, 2.0], ..Default::default() }
    }

    fn state(gates: &[TriggerBox], laps: usize) -> LogicState {
        let mut s = LogicState::default();
        s.players.local = Some(0);
        s.players.slots[0] = Some(Player { vehicle: 1, ..Default::default() });
        s.game_modes.won = true;
        let i = &mut s.checkpoints.inputs;
        i.route = route_from_gates([0.0; 3], gates, laps);
        i.per_lap = gates.len();
        i.loaded = true;
        i.markers = true;
        i.cars[0] = Some(RaceCar { car: car_at(0.0), has_vehicle: true, ..Default::default() });
        s
    }

    fn car_at(z: f32) -> CarBox {
        CarBox::level([0.0, 0.7, z], [0.0, 0.0, 1.0], [1.0, 0.7, 2.3])
    }

    fn drive(s: &mut LogicState, z: f32) -> bool {
        s.checkpoints.inputs.cars[0].as_mut().unwrap().car = car_at(z);
        apply(s, &Request::CheckpointsStep);
        s.checkpoints.progress.finished
    }

    fn outs(s: &mut LogicState) -> Vec<CheckpointOut> {
        std::mem::take(&mut s.requests)
            .into_iter()
            .filter_map(|r| if let Request::Checkpoint(o) = r { Some(o) } else { None })
            .collect()
    }

    #[test]
    fn three_gates_in_order_finish() {
        let gates = [gate(11, 50.0), gate(12, 100.0), gate(13, 150.0)];
        let mut s = state(&gates, 1);
        apply(&mut s, &Request::CheckpointsEnter);
        apply(&mut s, &Request::CheckpointsPlace);
        let o = outs(&mut s);
        assert!(o.contains(&CheckpointOut::Trigger { trigger: 11, on: true }));
        assert!(o.contains(&CheckpointOut::Marker { entry: 1, key: 1_013_519 }));
        assert!(o.contains(&CheckpointOut::Marker { entry: 2, key: 1_013_518 }), "{o:?}");
        assert!(o.contains(&CheckpointOut::Target { slot: 0, entry: 1 }));
        // Driving through gate 2's box first does nothing: only the target is tested.
        assert!(!drive(&mut s, 100.0));
        assert_eq!(s.checkpoints.progress.cp, 0);
        assert!(!drive(&mut s, 50.0));
        assert_eq!(s.checkpoints.progress.cp, 1);
        let o = outs(&mut s);
        assert!(o.contains(&CheckpointOut::Trigger { trigger: 11, on: false }));
        assert!(o.contains(&CheckpointOut::Marker { entry: 2, key: 1_013_519 }), "{o:?}");
        assert!(o.contains(&CheckpointOut::Marker { entry: 3, key: 1_695_650 }), "the finish as next gate: {o:?}");
        assert!(o.contains(&CheckpointOut::Event(RaceEvent::CheckpointPassed)));
        assert!(!drive(&mut s, 75.0));
        assert!(!drive(&mut s, 100.0));
        assert_eq!(s.checkpoints.progress.cp, 2);
        s.game_modes.word_2d28 = 3_000_000 * 42;
        assert!(drive(&mut s, 150.0), "the last gate finishes");
        let p = s.players.local_player().unwrap();
        assert_eq!((p.final_5b8, p.final_5c8), (1, 126_000_000));
        assert!(s.game_modes.won);
        apply(&mut s, &Request::CheckpointsLeave);
        assert!(s.checkpoints.progress.gates.is_empty());
    }

    #[test]
    fn missed_gate_is_never_passed_and_laps_count() {
        let gates = [gate(11, 50.0), gate(12, 100.0)];
        let mut s = state(&gates, 2);
        apply(&mut s, &Request::CheckpointsEnter);
        apply(&mut s, &Request::CheckpointsPlace);
        // Around gate 1 (missed), through gate 2: still waiting for gate 1.
        s.checkpoints.inputs.cars[0].as_mut().unwrap().car.centre[0] = 30.0;
        apply(&mut s, &Request::CheckpointsStep);
        assert!(!drive(&mut s, 100.0));
        assert_eq!((s.checkpoints.progress.cp, s.checkpoints.progress.lap), (0, 1));
        // A reset puts the target on (FUN_00661da0 says the car is past entry 1: entry 2 next).
        let c = s.checkpoints.inputs.cars[0].as_mut().unwrap();
        c.resetting = true;
        apply(&mut s, &Request::CheckpointsStep);
        let c = s.checkpoints.inputs.cars[0].as_mut().unwrap();
        c.resetting = false;
        c.reset_target = Some(2);
        c.car = car_at(0.0);
        apply(&mut s, &Request::CheckpointsStep);
        assert_eq!(s.checkpoints.progress.cp, 1);
        s.checkpoints.inputs.cars[0].as_mut().unwrap().reset_target = None;
        assert!(!drive(&mut s, 100.0));
        assert_eq!((s.checkpoints.progress.cp, s.checkpoints.progress.lap), (2, 2), "lap 2 after the lap's last gate");
        assert!(!drive(&mut s, 50.0));
        assert!(drive(&mut s, 100.0));
    }

    #[test]
    fn position_and_ai_racer() {
        let gates = [gate(11, 50.0), gate(12, 100.0)];
        let mut s = state(&gates, 1);
        s.players.slots[12] = Some(Player { vehicle: 2, ..Default::default() });
        s.checkpoints.inputs.racers = vec![12];
        s.checkpoints.inputs.cars[12] =
            Some(RaceCar { car: car_at(40.0), has_vehicle: true, to_finish: Some(60.0), ..Default::default() });
        s.checkpoints.inputs.cars[0].as_mut().unwrap().to_finish = Some(100.0);
        apply(&mut s, &Request::CheckpointsEnter);
        apply(&mut s, &Request::CheckpointsPlace);
        assert!(!drive(&mut s, 0.0));
        assert_eq!(s.checkpoints.progress.position, 2);
        // The racer passes both gates: done, and it counts as ahead.
        s.checkpoints.inputs.cars[12].as_mut().unwrap().car = car_at(50.0);
        drive(&mut s, 0.0);
        s.checkpoints.inputs.cars[12].as_mut().unwrap().car = car_at(100.0);
        drive(&mut s, 0.0);
        assert!(s.checkpoints.progress.ai_done[12]);
        assert!(outs(&mut s).contains(&CheckpointOut::AiDone { slot: 12 }));
        drive(&mut s, 50.0);
        assert!(drive(&mut s, 100.0));
        assert_eq!(s.checkpoints.progress.position, 2);
        s.checkpoints.inputs.first_place_only = true;
        assert!(update(&mut s));
        assert!(!s.game_modes.won, "second place loses a first-place-only race");
    }
}
