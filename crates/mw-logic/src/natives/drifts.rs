//! Drift scoring of the drift game mode: `HSMDODRIFTS` ("HSMDoDriftScoring", factory 0x77fc80, vtable 0xfdc148,
//! 0x1340 B of instance state; decomp.md "Drift scoring"). Ported from the code: enter 0x75b2f0, enter update
//! `FUN_0074c370`, update 0x75e7e0 (with `FUN_0074c960` the gate timers, `FUN_00756990` crashes into the apex
//! markers, `FUN_0075b6a0` the combo, `FUN_0074cbf0` the gate lines and the score, `FUN_00743bb0` whether the score
//! runs, `FUN_0074c6c0` a gate's outcome, `FUN_00756c20` bank, `FUN_00756b10` lose), exit `FUN_0075b460`; the binding
//! root `DriftScoring` (`FUN_00756d60`'s registrations and their getters 0x76b450, 0x768890, 0x768910, 0x76a170,
//! 0x765bb0, 0x76b480).
//!
//! **What the runtime fills** ([`DriftInputs`]): the drift-marker list (GeneSys 2173069, `Gameplay.DriftMarker`)
//! with each marker's two apex points and trigger box, the event's marker ids (event +0x54, count +0xd2), the
//! medal scores (`FUN_006f1900`), the frame time and the engine flags the update reads, the local car's box,
//! speed, body slip (drift +0xe0), drift exit code (drift +0x10), contact flags, the nitrous record's "drifting"
//! byte, its contacts with the markers' props and the route checks of the distance manager. The distance to the
//! finish comes from [`CheckpointInputs::cars`](super::checkpoints::CheckpointInputs) (DTF+0xdf0, the same field).
//!
//! **What the logic decides** ([`DriftState`]): the drift score (+0x1304), the combo timer (+0x1308), the banked
//! score (+0x1314), the gates and their outcomes, the medal at the end (`FUN_006b9d00`, GMM+0x2e78), the player's
//! score (+0x418). **What the runtime carries out**: the [`DriftOut`] requests (loads, apex-marker props and their
//! damaged swap, map icons, score events), [`Request::NitrousAward`] (100 on enter) and
//! [`Request::SpeedPoints`] (the banked score on exit).

use super::LogicState;
use super::state::Request;
use super::trigger_boxes::{CarBox, TriggerBox, V3, add, dot, hits, sub};
use crate::binding::{Lookup, Segment, Value};

/// A slot of a scoring behaviour that the logic carries out (`HSMDODRIFTS`, `DOSMSHNGRB`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Enter,
    EnterUpdate,
    Update,
    ExitUpdate,
    Exit,
}

/// The `GameLogicConfig` (`GlobalConfig` +0x24) fields the drift code reads. Defaults: the install's
/// `GLOBALCONFIG.BNDL` values (fields unnamed in the type table; offsets from the code).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DriftConfig {
    /// +0x228: the exponent of the angle factor (1.75).
    pub angle_exponent: f32,
    /// +0x22c: the body slip at which the angle factor is full (45°).
    pub max_angle: f32,
    /// +0x230: a clean gate's score (50; the binding `Bonus`).
    pub gate_bonus: f32,
    /// +0x234: the combo's grace after a drift ends normally (exit codes 5 / 6; 0.5 s).
    pub grace: f32,
    /// +0x238: the smallest body slip the angle factor counts (7.5°).
    pub min_angle: f32,
    /// +0x23c: the score rate (0.725).
    pub rate: f32,
    /// +0x240: the exponent of the speed factor (0.5).
    pub speed_exponent: f32,
    /// +0x2d4 (u16): the step the score is shown and banked in (10).
    pub step: u16,
    /// `MapPoiConfig` (+0x2ac) +0x24: the map icon of a drift marker (2211026).
    pub poi_key: u32,
}

impl Default for DriftConfig {
    fn default() -> Self {
        DriftConfig {
            angle_exponent: 1.75,
            max_angle: 45.0,
            gate_bonus: 50.0,
            grace: 0.5,
            min_angle: 7.5,
            rate: 0.725,
            speed_exponent: 0.5,
            step: 10,
            poi_key: 2_211_026,
        }
    }
}

/// One `Gameplay.DriftMarker` (0x1c B): id +0xc, the two apex points +0x10 / +0x14 (point triggers), the gate's
/// trigger +0x18.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DriftMarker {
    pub id: u32,
    /// The apex points (`FUN_0053c020`: the trigger records' positions); the props stand on them, upright, facing
    /// the point's local Z (`FUN_00743940`, `FUN_0042f250`).
    pub post_ids: [u32; 2],
    pub posts: [V3; 2],
    /// The trigger past the gate (`FUN_00537190`(vehicle, trigger, −1)).
    pub trigger: Option<TriggerBox>,
}

/// What the runtime supplies (see the module doc).
#[derive(Debug, Clone)]
pub struct DriftInputs {
    pub config: DriftConfig,
    /// The marker list (GeneSys 2173069, +0x10 / count +0x14), in list order.
    pub markers: Vec<DriftMarker>,
    /// The current event's marker ids (`FUN_006e66a0` → +0x54, count +0xd2).
    pub event_markers: Vec<u32>,
    /// The resource loader's state +0x12c4 is 2 or 3 (busy): the enter update waits.
    pub loading: bool,
    /// `FUN_006f1900`(GMM, event, 0 / 1 / 2): the gold, silver and bronze scores (read on enter; −1 = none).
    pub medals: [i32; 3],
    /// The game clock's frame time (`FUN_0046ee10`: +0x14 × +0x10) and its running flag +0x1c.
    pub dt: f32,
    pub clock_running: bool,
    /// The trigger manager's (`*0x010edaf0`) byte +0x84e0 (`FUN_00532dd0`; set by a trigger message at 0x53a7a8,
    /// cleared at 0x5367d0; meaning not traced). Default true.
    pub trigger_flag: bool,
    /// The local vehicle has a simulation record (world +0x55c48 + vehicle × 4 > −1).
    pub in_sim: bool,
    /// The local car's box (the overlap job's; also the gate line's target: `FUN_00a102b0` builds a box of the
    /// body's +0x40 extents, `FUN_004b1cb0` casts the line against it).
    pub car: Option<CarBox>,
    /// |velocity| (`FUN_006ba3b0` → `FUN_0083bef0`), m/s.
    pub speed: f32,
    /// The drift's body slip, degrees 0..180 (the vehicle's virtual +0x48 → vehicle +0x2b90 = drift +0xe0).
    pub body_slip: f32,
    /// The drift's exit code (vehicle +0x2ac0 = drift +0x10; decomp.md "Drift": 1 / 3 collisions, 5 airborne,
    /// 6 slip fell, 7 slow, 8 reverse, 9 handbrake, 11 special state).
    pub drift_exit: i32,
    /// Vehicle +0x30ed / +0x30ef / +0x30f0: the contact bytes `FUN_0090dfa0` sets (+0x30ed the world).
    pub contact: bool,
    /// The nitrous record's event-5 byte (game +0x264f0 + slot × 200 + 5, "Drifting": in a drift at speed for
    /// 0.75 s; decomp.md "Nitrous refill").
    pub drifting: bool,
    /// The local car's contacts this frame (world +0x5df4c / +0x5df5c, entry +100) that are a marker's prop: the
    /// marker's index in the gate list, one entry per contact, in contact order.
    pub marker_contacts: Vec<usize>,
    /// `FUN_0065dcf0` on the distance manager: the wrong-way state (+0x439b0 2 / 3 without +0x439c0, the clock
    /// not past GMM+0x2d18, no final time, no missed gate).
    pub wrong_way: bool,
    /// `FUN_0065dd60`(slot, position): the car's distance from its nearest route point (m).
    pub route_offset: f32,
}

impl Default for DriftInputs {
    fn default() -> Self {
        DriftInputs {
            config: DriftConfig::default(),
            markers: Vec::new(),
            event_markers: Vec::new(),
            loading: false,
            medals: [-1; 3],
            dt: 0.0,
            clock_running: true,
            trigger_flag: true,
            in_sim: true,
            car: None,
            speed: 0.0,
            body_slip: 0.0,
            drift_exit: 0,
            contact: false,
            drifting: false,
            marker_contacts: Vec::new(),
            wrong_way: false,
            route_offset: 0.0,
        }
    }
}

/// One gate of the event (0x70 B at +0x50, count +0x12b0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DriftGate {
    pub marker: DriftMarker,
    /// +0x68: the gate has its outcome (`DriftMarkerUsed`).
    pub used: bool,
    /// +0x6c: the countdown after the line was crossed in a drift (−1 = not crossed).
    pub timer: f32,
}

/// The behaviour's state (the logic decides it).
#[derive(Debug, Clone, PartialEq)]
pub struct DriftState {
    /// Between enter and exit: the binding root `DriftScoring` is registered.
    pub active: bool,
    /// +0x1300: the enter update's phase (4 = `ScoringActive`).
    pub phase: u32,
    pub gates: Vec<DriftGate>,
    /// +0x1304: the running drift score.
    pub drift: f32,
    /// +0x1308: the combo's grace timer (−1 = none).
    pub combo_timer: f32,
    /// +0x130c: the smallest distance to the finish so far.
    pub min_to_finish: f32,
    /// +0x1310 `Drift`: +0x1304 in steps.
    pub drift_shown: i32,
    /// +0x1314 `BankedScore`.
    pub banked: i32,
    /// +0x1318 `LastCombo`.
    pub last_combo: i32,
    /// +0x131c / +0x1320 / +0x1324: gold, silver, bronze.
    pub medals: [i32; 3],
    /// +0x1328 `GatesSuccessfullyPassed`, +0x132c `CurrentGateScore`.
    pub gates_passed: i32,
    pub gate_score: i32,
    /// +0x1330: the nitrous record's drifting byte last frame.
    pub was_drifting: bool,
    /// +0x1331 `JustLostCombo`, +0x1332 `DriftJustTickedUp`, +0x1333 `WasLastGateASuccess`, +0x1334
    /// `HasJustHitGate`, +0x1335 `HasJustCrashedIntoGate`.
    pub just_lost: bool,
    pub ticked_up: bool,
    pub last_gate_success: bool,
    pub just_hit_gate: bool,
    pub just_crashed: bool,
}

impl Default for DriftState {
    fn default() -> Self {
        DriftState {
            active: false,
            phase: 0,
            gates: Vec::new(),
            drift: 0.0,
            combo_timer: NO_TIMER,
            min_to_finish: f32::MAX,
            drift_shown: 0,
            banked: 0,
            last_combo: 0,
            medals: [-1; 3],
            gates_passed: 0,
            gate_score: 0,
            was_drifting: false,
            just_lost: false,
            ticked_up: false,
            last_gate_success: true,
            just_hit_gate: false,
            just_crashed: false,
        }
    }
}

/// The drift scoring: runtime inputs and the logic's state.
#[derive(Debug, Clone, Default)]
pub struct DriftScoring {
    pub inputs: DriftInputs,
    pub state: DriftState,
}

/// `DAT_00f8f340`: the "none" timer.
const NO_TIMER: f32 = -1.0;
/// `DAT_00f8a810`: a gate's countdown after its line was crossed in a drift (0.5 s).
pub const GATE_TIME: f32 = 0.5;
/// `.bss` 0x0117a760 (0.7, read from snapshot 1) × the up vector `.rdata` 0x00fd6700: the gate line's height.
pub const LINE_HEIGHT: f32 = 0.7;
/// `DAT_00fd3434`: how far the distance to the finish may grow back before the score stops (2 m).
pub const BACKTRACK: f32 = 2.0;
/// `DAT_00fd3438`: the route offset beyond which the score stops (30 m).
pub const MAX_ROUTE_OFFSET: f32 = 30.0;
/// The nitrous `FUN_00658ad0` gives on enter (`.rdata` 0x00fd2104).
pub const ENTER_NITROUS: f32 = 100.0;
/// The HUD sequence the exit stops (every instance, `FUN_00a43290` / `FUN_00a431e0` on G+0x2f480 +0x12170).
pub const DRIFT_SEQUENCE: u32 = 2_211_197;
/// The gate list's size in the instance (0x50 + 42 × 0x70 = +0x12b0); the code does not check it.
pub const MAX_GATES: usize = 42;

/// The model of a marker prop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerModel {
    /// `DriftApexMarker_Scenery?ID=2211029` (string 0x00fd3440, loaded into +0x12f0).
    Normal,
    /// `DriftApexMarker_Damaged_Scenery?ID=2211033` (0x00fd34a0, +0x12f8): every gate with an outcome.
    Damaged,
}

/// A score event the drift code sends (`FUN_006a8050` with a `ScoringConfig` id, global config +0x44).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftEvent {
    /// +0xb4 (2317085): a marker hit before its gate had an outcome.
    GateCrashed,
    /// +0xb8 `DriftGatePassed` (2317084).
    GatePassed,
    /// +0xbc (2211195): the combo banked.
    ComboBanked,
    /// +0xc0 (2211045): the combo lost.
    ComboLost,
}

impl DriftEvent {
    /// The `ScoringConfig` field.
    pub fn config_offset(self) -> u32 {
        match self {
            DriftEvent::GateCrashed => 0xb4,
            DriftEvent::GatePassed => 0xb8,
            DriftEvent::ComboBanked => 0xbc,
            DriftEvent::ComboLost => 0xc0,
        }
    }
}

/// What `HSMDODRIFTS` asks the engine to do.
#[derive(Debug, Clone, PartialEq)]
pub enum DriftOut {
    /// Enter update phase 0: `FUN_00adccd0`(2173069, 0x11): load the marker list.
    LoadMarkerList,
    /// Phase 1: load the two marker models (`FUN_00adaef0` into +0x12f0 / +0x12f8).
    LoadMarkerModels,
    /// `FUN_00793350`(`MapPoiConfig` +0x24, apex point 0, 0, 0, gate index): the gate's map icon.
    Poi {
        gate: usize,
        key: u32,
    },
    /// `FUN_00790da0`: remove it.
    PoiRemove {
        gate: usize,
    },
    /// `FUN_00743940` / `FUN_00843d30`: the prop on apex point `post` (0 / 1) of the gate; `Damaged` replaces the
    /// standing one at its transform (`FUN_00843e60`, the object's matrix from `DAT_010edb68`).
    Prop {
        gate: usize,
        post: usize,
        model: MarkerModel,
    },
    /// `FUN_00843e60`: release it.
    PropRemove {
        gate: usize,
        post: usize,
    },
    Event(DriftEvent),
    /// Stop every instance of the sequence (the exit's loop over G+0x2f480 +0x12458).
    StopSequence {
        sequence: u32,
    },
}

fn out(s: &mut LogicState, o: DriftOut) {
    s.requests.push(Request::Drift(o));
}

/// The slots of `HSMDODRIFTS` (called from [`super::LogicHost::apply`]).
pub fn apply(s: &mut LogicState, r: &Request) {
    match r {
        Request::DriftScoring(Slot::Enter) => enter(s),
        Request::DriftScoring(Slot::EnterUpdate) => {
            enter_update(s);
        }
        Request::DriftScoring(Slot::Update) => update(s),
        Request::DriftScoring(Slot::Exit) => leave(s),
        _ => {}
    }
}

/// Enter (0x75b2f0): the scores and flags reset (+0x1330 = 0x01000000: only `WasLastGateASuccess` set), the
/// medals read, the tank filled (`FUN_00658ad0`(slot, 100)), the root registered (`FUN_00756d60`).
pub fn enter(s: &mut LogicState) {
    let medals = s.drift.inputs.medals;
    s.drift.state = DriftState { active: true, medals, ..Default::default() };
    if let Some(local) = s.players.local {
        s.requests.push(Request::NitrousAward { slot: local as i32, amount: ENTER_NITROUS });
    }
}

/// The enter update (`FUN_0074c370`); true = entered. Phase 0 loads the list, 1 waits and loads the models, 3 waits,
/// 4 builds the gates: every marker of the list whose id the event names, with its icon and two props.
pub fn enter_update(s: &mut LogicState) -> bool {
    let d = &mut s.drift;
    if d.state.phase == 0 {
        d.state.phase = 1;
        out(s, DriftOut::LoadMarkerList);
    }
    let d = &mut s.drift;
    if d.state.phase == 1 {
        if d.inputs.loading {
            return false;
        }
        d.state.phase = 3;
        out(s, DriftOut::LoadMarkerModels);
    }
    let d = &mut s.drift;
    if d.state.phase == 3 {
        if d.inputs.loading {
            return false;
        }
        d.state.phase = 4;
        let wanted: Vec<DriftMarker> =
            d.inputs.markers.iter().filter(|m| d.inputs.event_markers.contains(&m.id)).copied().collect();
        let key = d.inputs.config.poi_key;
        for marker in wanted.into_iter().take(MAX_GATES) {
            let gate = s.drift.state.gates.len();
            s.drift.state.gates.push(DriftGate { marker, used: false, timer: NO_TIMER });
            out(s, DriftOut::Poi { gate, key });
            for post in 0..2 {
                out(s, DriftOut::Prop { gate, post, model: MarkerModel::Normal });
            }
        }
        return true;
    }
    s.drift.state.phase == 4
}

/// `FUN_00756b10` (and `FUN_00756c20`): the score in steps of +0x2d4, truncated.
fn in_steps(score: f32, step: u16) -> i32 {
    let step = i32::from(step.max(1));
    let v = score as i32;
    if v % step != 0 { v / step * step } else { v }
}

/// `FUN_00756c20`: bank the combo (the drift in steps plus the gate score) when there is one; `send` the event.
fn bank(s: &mut LogicState, send: bool) {
    let d = &mut s.drift;
    let score = in_steps(d.state.drift, d.inputs.config.step);
    if score <= 0 && d.state.gate_score <= 0 {
        return;
    }
    let total = d.state.gate_score + score;
    let st = &mut d.state;
    st.banked += total;
    st.drift = 0.0;
    st.last_combo = total;
    st.combo_timer = NO_TIMER;
    st.gates_passed = 0;
    st.gate_score = 0;
    if send {
        out(s, DriftOut::Event(DriftEvent::ComboBanked));
    }
}

/// `FUN_00756b10`: lose the combo when there is one.
fn lose(s: &mut LogicState) {
    let d = &mut s.drift;
    let score = in_steps(d.state.drift, d.inputs.config.step);
    if score <= 0 && d.state.gate_score <= 0 {
        return;
    }
    let st = &mut d.state;
    st.drift = 0.0;
    st.gate_score = 0;
    st.combo_timer = NO_TIMER;
    st.just_lost = true;
    st.last_combo = 0;
    st.gates_passed = 0;
    out(s, DriftOut::Event(DriftEvent::ComboLost));
}

/// `FUN_0074c6c0`(gate, success): the gate's outcome, once: `HasJustHitGate`, `WasLastGateASuccess`; a clean pass
/// counts and adds the bonus to the gate score; both props turn damaged.
fn resolve_gate(s: &mut LogicState, gate: usize, success: bool) {
    let bonus = s.drift.inputs.config.gate_bonus as i32;
    let st = &mut s.drift.state;
    if st.gates[gate].used {
        return;
    }
    st.just_hit_gate = true;
    st.last_gate_success = success;
    st.gates[gate].used = true;
    if success {
        st.gates_passed += 1;
        st.gate_score += bonus;
        out(s, DriftOut::Event(DriftEvent::GatePassed));
    }
    for post in 0..2 {
        out(s, DriftOut::Prop { gate, post, model: MarkerModel::Damaged });
    }
}

/// The update (0x75e7e0). Never exits by itself (the mode's machine ends it).
pub fn update(s: &mut LogicState) {
    let i = &s.drift.inputs;
    let gated = i.clock_running && s.game_modes.timer_running && i.trigger_flag;
    if gated {
        let dt = i.dt;
        let st = &mut s.drift.state;
        st.just_lost = false;
        st.just_hit_gate = false;
        st.just_crashed = false;
        let racing = s.players.local_player().is_some_and(|p| p.vehicle_alive && !p.wrecked);
        if racing && s.drift.inputs.in_sim {
            let to_finish = to_finish(s);
            let st = &mut s.drift.state;
            st.min_to_finish = if st.min_to_finish > to_finish { to_finish } else { st.min_to_finish };
            gate_timers(s, dt);
            crashes(s);
            combo(s, dt);
            gate_lines_and_score(s, dt);
        }
    }
    // Always: the shown score in steps, whether it went up, the player's score.
    let d = &mut s.drift;
    let shown = in_steps(d.state.drift, d.inputs.config.step);
    d.state.ticked_up = shown > d.state.drift_shown;
    d.state.drift_shown = shown;
    let banked = d.state.banked;
    set_player_score(s, banked);
}

/// DTF+0xdf0 + the local slot (`FLT_MAX` when not computed).
fn to_finish(s: &LogicState) -> f32 {
    s.players.local.and_then(|l| s.checkpoints.inputs.cars[l]).and_then(|c| c.to_finish).unwrap_or(f32::MAX)
}

/// Player +0x418 = the score; +0x348 (a word) too for a human player (+0x9c == 0).
pub(super) fn set_player_score(s: &mut LogicState, score: i32) {
    if let Some(p) = s.players.local_player_mut() {
        p.score = score;
        if p.kind == 0 {
            p.score_348 = score as i16;
        }
    }
}

/// `FUN_0074c960`: a crossed gate whose countdown runs out without another outcome is a clean pass.
fn gate_timers(s: &mut LogicState, dt: f32) {
    for g in 0..s.drift.state.gates.len() {
        let gate = &mut s.drift.state.gates[g];
        if gate.timer >= 0.0 {
            gate.timer -= dt;
            if gate.timer < 0.0 && !gate.used {
                resolve_gate(s, g, true);
            }
        }
    }
}

/// `FUN_00756990`: the car touching a marker's prop: the crash event when the gate is still open, the gate fails,
/// `HasJustCrashedIntoGate`.
fn crashes(s: &mut LogicState) {
    let contacts = std::mem::take(&mut s.drift.inputs.marker_contacts);
    for &g in &contacts {
        if g >= s.drift.state.gates.len() {
            continue;
        }
        if !s.drift.state.gates[g].used {
            out(s, DriftEvent::GateCrashed.into());
        }
        resolve_gate(s, g, false);
        s.drift.state.just_crashed = true;
    }
    s.drift.inputs.marker_contacts = contacts;
}

impl From<DriftEvent> for DriftOut {
    fn from(e: DriftEvent) -> Self {
        DriftOut::Event(e)
    }
}

/// `FUN_0075b6a0`: the combo. When the drifting byte drops, the drift's exit code decides: a collision (1, 2, 3,
/// 11) loses the combo, a normal end (5, 6) starts the grace timer, anything else banks it. While the timer runs, a
/// contact or a crash loses it and its end banks it.
fn combo(s: &mut LogicState, dt: f32) {
    let i = &s.drift.inputs;
    let drifting = i.drifting;
    let was = s.drift.state.was_drifting;
    let mut done = drifting;
    if was && !drifting {
        match i.drift_exit {
            0xb | 1 | 2 | 3 => lose(s),
            5 | 6 => s.drift.state.combo_timer = s.drift.inputs.config.grace,
            _ => bank(s, true),
        }
    }
    if !done && s.drift.state.combo_timer < 0.0 {
        done = true;
    }
    if !done {
        let crashed = s.drift.inputs.contact || s.players.local_player().is_some_and(|p| p.crashed);
        if crashed {
            lose(s);
        }
        s.drift.state.combo_timer -= dt;
        if !crashed && s.drift.state.combo_timer < 0.0 {
            bank(s, true);
        }
    }
    s.drift.state.was_drifting = drifting;
}

/// `FUN_00743bb0`: the score runs while drifting or in the grace, not further from the finish than the best so far
/// plus 2 m, not wrong way and not more than 30 m off the route.
fn scoring(s: &LogicState) -> bool {
    let i = &s.drift.inputs;
    let off = i.wrong_way || i.route_offset > MAX_ROUTE_OFFSET;
    (i.drifting || s.drift.state.combo_timer > 0.0) && to_finish(s) <= s.drift.state.min_to_finish + BACKTRACK && !off
}

/// `FUN_0074cbf0`: each open gate: its line (the apex points raised 0.7 m) crossing the car while drifting starts
/// its countdown, crossing it otherwise fails it, and being in its trigger before crossing fails it. Then, while
/// the score runs: drift += (clamp(slip, +0x238, +0x22c) / +0x22c)^+0x228 × +0x23c × speed × dt × speed^+0x240.
fn gate_lines_and_score(s: &mut LogicState, dt: f32) {
    let i = &s.drift.inputs;
    let car = i.car;
    let drifting = i.drifting;
    for g in 0..s.drift.state.gates.len() {
        let gate = s.drift.state.gates[g];
        if gate.used {
            continue;
        }
        let Some(car) = car else { continue };
        let up = [0.0, LINE_HEIGHT, 0.0];
        let crossed = segment_hits_box(add(gate.marker.posts[0], up), add(gate.marker.posts[1], up), &car);
        if !crossed {
            let inside = gate.marker.trigger.is_some_and(|t| hits(&car, &t));
            if inside && gate.timer < 0.0 {
                resolve_gate(s, g, false);
            }
        } else if !drifting {
            resolve_gate(s, g, false);
        } else {
            s.drift.state.gates[g].timer = GATE_TIME;
        }
    }
    if scoring(s) {
        let i = &s.drift.inputs;
        let c = i.config;
        let speed = i.speed;
        let angle = i.body_slip.max(c.min_angle).min(c.max_angle) / c.max_angle;
        let gain = angle.powf(c.angle_exponent) * (c.rate * (speed * dt) * speed.powf(c.speed_exponent));
        s.drift.state.drift += gain;
    }
}

/// `FUN_004b1cb0` against the car's box (`FUN_00a102b0`): the segment `a`–`b` meets the box (slab test in the
/// box's frame; *inferred*: the query's narrow phase was not read).
pub fn segment_hits_box(a: V3, b: V3, car: &CarBox) -> bool {
    let p = sub(a, car.centre);
    let d = sub(b, a);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for k in 0..3 {
        let o = dot(p, car.axes[k]);
        let v = dot(d, car.axes[k]);
        let h = car.half[k];
        if v.abs() < 1e-9 {
            if o.abs() > h {
                return false;
            }
            continue;
        }
        let (mut lo, mut hi) = ((-h - o) / v, (h - o) / v);
        if lo > hi {
            std::mem::swap(&mut lo, &mut hi);
        }
        t0 = t0.max(lo);
        t1 = t1.min(hi);
        if t0 > t1 {
            return false;
        }
    }
    true
}

/// `FUN_0073ce40`: the medal reached: 0 gold, 1 silver, 2 bronze, 3 none (also while the mode timer is off).
fn medal(s: &LogicState) -> u8 {
    let st = &s.drift.state;
    if !s.game_modes.timer_running {
        return 3;
    }
    let b = st.banked;
    if st.medals[0] <= b {
        0
    } else if st.medals[1] <= b {
        1
    } else if st.medals[2] <= b {
        2
    } else {
        3
    }
}

/// The exit (`FUN_0075b460`): bank without the event, stop the HUD sequence, the player's score, the place from
/// the medal (`FUN_006b9d00`: medal + 1; none clears GMM+0x2e78), the banked score to the Speed Points total when
/// the end reason is 0 / 1 (`FUN_00734f60`(score, 0, car)), the root unregistered, the icons and props removed.
pub fn leave(s: &mut LogicState) {
    bank(s, false);
    out(s, DriftOut::StopSequence { sequence: DRIFT_SEQUENCE });
    let banked = s.drift.state.banked;
    set_player_score(s, banked);
    let m = medal(s);
    if m == 3 {
        s.game_modes.won = false;
    }
    finish(s, i32::from(m) + 1);
    let reason = s.game_modes.end_reason & 0xffff;
    if banked > 0 && (reason == 1 || reason == 0) {
        let car = s.players.local_player().map_or(0, |p| p.vehicle_id);
        s.requests.push(Request::SpeedPoints { sp: banked, bucket: 0, car });
    }
    s.drift.state.active = false;
    let n = s.drift.state.gates.len();
    for gate in 0..n {
        for post in 0..2 {
            out(s, DriftOut::PropRemove { gate, post });
        }
        out(s, DriftOut::PoiRemove { gate });
    }
    s.drift.state.gates.clear();
}

/// `FUN_006b9d00`(place, GMM+0x2d28 / +0x2d2c) on the local player: the final event data.
pub(super) fn finish(s: &mut LogicState, place: i32) {
    let (lo, hi) = (s.game_modes.word_2d28, s.game_modes.word_2d2c);
    if let Some(p) = s.players.local_player_mut() {
        p.final_5b8 = place;
        p.final_5c8 = lo;
        p.final_5cc = hi;
        p.final_5d0 = false;
    }
}

/// The root `DriftScoring` while the behaviour runs (`FUN_00756d60`; the getters for the computed members).
pub fn lookup(s: &LogicState, members: &[Segment]) -> Lookup {
    let st = &s.drift.state;
    if !st.active {
        return Lookup::Null;
    }
    let Some((first, _)) = members.split_first() else { return Lookup::Found(Value::Object(true)) };
    let is = |n: &str| first.hash == crate::binding::hash(n);
    let v = if is("BankedScore") {
        Value::Int(st.banked)
    } else if is("Drift") {
        Value::Int(st.drift_shown)
    } else if is("CurrentGateScore") {
        Value::Int(st.gate_score)
    } else if is("LastCombo") {
        Value::Int(st.last_combo)
    } else if is("DriftJustTickedUp") {
        Value::Bool(st.ticked_up)
    } else if is("JustLostCombo") {
        Value::Bool(st.just_lost)
    } else if is("WasLastGateASuccess") {
        Value::Bool(st.last_gate_success)
    } else if is("GatesSuccessfullyPassed") {
        Value::Int(st.gates_passed)
    } else if is("HasJustHitGate") {
        Value::Bool(st.just_hit_gate)
    } else if is("HasJustCrashedIntoGate") {
        Value::Bool(st.just_crashed)
    } else if is("Bonus") {
        // 0x76b450: (int) GameLogicConfig +0x230.
        Value::Int(s.drift.inputs.config.gate_bonus as i32)
    } else if is("TargetScore") {
        Value::Int(target_score(s))
    } else if is("MedalIndex") {
        Value::UInt(u32::from(medal(s)))
    } else if is("TargetMedalIndex") {
        // 0x76a170 → `FUN_00743cb0`: gold and silver aim at gold (0), bronze at silver (1), none at bronze (2).
        let m = medal(s);
        Value::UInt(u32::from(m.saturating_sub(1)))
    } else if is("ScoringActive") {
        Value::Bool(st.phase == 4)
    } else if is("DriftMarkerUsed") {
        // 0x76b480: the argument is the gate index (0-based); out of range reads false.
        let used = first.index.pos().and_then(|n| usize::try_from(n).ok()).and_then(|n| st.gates.get(n));
        Value::Bool(used.is_some_and(|g| g.used))
    } else {
        return Lookup::Null;
    };
    Lookup::Found(v)
}

/// 0x768890: past silver (mode timer running) → gold; otherwise silver when bronze is reached, else bronze.
fn target_score(s: &LogicState) -> i32 {
    let st = &s.drift.state;
    let running = s.game_modes.timer_running;
    if running && (st.banked >= st.medals[0] || st.banked >= st.medals[1]) {
        return st.medals[0];
    }
    if medal(s) == 2 { st.medals[1] } else { st.medals[2] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::natives::players::Player;

    const DT: f32 = 0.1;

    fn marker(id: u32, z: f32) -> DriftMarker {
        let trigger =
            TriggerBox { id: id * 10 + 2, centre: [0.0, 2.0, z + 10.0], size: [20.0, 4.0, 2.0], ..Default::default() };
        DriftMarker {
            id,
            post_ids: [id * 10, id * 10 + 1],
            posts: [[-5.0, 0.0, z], [5.0, 0.0, z]],
            trigger: Some(trigger),
        }
    }

    fn car_at(z: f32) -> CarBox {
        CarBox::level([0.0, 0.7, z], [0.0, 0.0, 1.0], [1.0, 0.7, 2.3])
    }

    fn state() -> LogicState {
        let mut s = LogicState::default();
        s.players.local = Some(0);
        s.players.slots[0] = Some(Player { vehicle: 1, vehicle_alive: true, vehicle_id: 911, ..Default::default() });
        s.game_modes.timer_running = true;
        s.game_modes.won = true;
        let i = &mut s.drift.inputs;
        i.markers = vec![marker(1, 50.0), marker(2, 150.0), marker(3, 250.0)];
        i.event_markers = vec![3, 1];
        i.medals = [3000, 2000, 1000];
        i.dt = DT;
        i.car = Some(car_at(0.0));
        s
    }

    fn outs(s: &mut LogicState) -> Vec<DriftOut> {
        std::mem::take(&mut s.requests)
            .into_iter()
            .filter_map(|r| if let Request::Drift(o) = r { Some(o) } else { None })
            .collect()
    }

    fn step(s: &mut LogicState, z: f32) {
        s.drift.inputs.car = Some(car_at(z));
        apply(s, &Request::DriftScoring(Slot::Update));
    }

    fn entered() -> LogicState {
        let mut s = state();
        apply(&mut s, &Request::DriftScoring(Slot::Enter));
        assert!(s.requests.contains(&Request::NitrousAward { slot: 0, amount: 100.0 }));
        s.drift.inputs.loading = true;
        assert!(!enter_update(&mut s), "waits for the marker list");
        assert_eq!(s.drift.state.phase, 1);
        s.drift.inputs.loading = false;
        assert!(enter_update(&mut s));
        s
    }

    #[test]
    fn gates_built_from_the_event_list_in_list_order() {
        let mut s = entered();
        let ids: Vec<u32> = s.drift.state.gates.iter().map(|g| g.marker.id).collect();
        assert_eq!(ids, [1, 3], "list order, only the event's markers");
        let o = outs(&mut s);
        assert_eq!(o[..2], [DriftOut::LoadMarkerList, DriftOut::LoadMarkerModels]);
        assert!(o.contains(&DriftOut::Poi { gate: 1, key: 2_211_026 }));
        assert!(o.contains(&DriftOut::Prop { gate: 0, post: 1, model: MarkerModel::Normal }));
        assert_eq!(s.resolve_bool("DriftScoring.ScoringActive"), Some(true));
        assert_eq!(s.resolve_bool("DriftScoring.WasLastGateASuccess"), Some(true));
    }

    #[test]
    fn drift_through_a_gate_scores_and_banks() {
        let mut s = entered();
        outs(&mut s);
        let i = &mut s.drift.inputs;
        i.drifting = true;
        i.speed = 25.0;
        i.body_slip = 30.0;
        step(&mut s, 40.0);
        // (30 / 45)^1.75 × 0.725 × 25 × 0.1 × 25^0.5
        let want = (30.0f32 / 45.0).powf(1.75) * (0.725 * (25.0 * DT) * 25.0f32.powf(0.5));
        assert!((s.drift.state.drift - want).abs() < 1e-4, "{} {want}", s.drift.state.drift);
        // Across the line while drifting: the countdown, then the clean pass 0.5 s after the last contact.
        step(&mut s, 50.0);
        assert_eq!(s.drift.state.gates[0].timer, GATE_TIME);
        for z in [53.0, 56.0, 59.0, 62.0, 65.0, 68.0] {
            step(&mut s, z);
        }
        assert!(s.drift.state.gates[0].used);
        assert_eq!((s.drift.state.gates_passed, s.drift.state.gate_score), (1, 50));
        assert!(s.drift.state.last_gate_success);
        let o = outs(&mut s);
        assert!(o.contains(&DriftOut::Event(DriftEvent::GatePassed)));
        assert!(o.contains(&DriftOut::Prop { gate: 0, post: 0, model: MarkerModel::Damaged }));
        assert_eq!(s.resolve_bool("DriftScoring.DriftMarkerUsed[0]"), Some(true));
        assert_eq!(s.resolve_int("DriftScoring.GatesSuccessfullyPassed"), Some(1));
        // The drift ends normally (code 6): the grace runs (the score still counts in it), then the combo banks.
        s.drift.inputs.drifting = false;
        s.drift.inputs.drift_exit = 6;
        step(&mut s, 70.0);
        assert!(s.drift.state.combo_timer > 0.0);
        let mut drift = s.drift.state.drift;
        while s.drift.state.combo_timer >= 0.0 {
            drift = s.drift.state.drift;
            step(&mut s, 70.0);
        }
        // The bank (`FUN_0075b6a0`) runs before that frame's score (`FUN_0074cbf0`), which starts the next combo.
        let total = 50 + in_steps(drift, 10);
        assert_eq!((s.drift.state.banked, s.drift.state.last_combo), (total, total));
        assert_eq!(s.players.local_player().unwrap().score, total);
        assert!(outs(&mut s).contains(&DriftOut::Event(DriftEvent::ComboBanked)));
        assert_eq!(s.resolve_int("DriftScoring.BankedScore"), Some(i64::from(total)));
    }

    #[test]
    fn crossing_without_drifting_fails_and_a_crash_loses_the_combo() {
        let mut s = entered();
        outs(&mut s);
        step(&mut s, 50.0);
        assert!(s.drift.state.gates[0].used);
        assert!(!s.drift.state.last_gate_success && s.drift.state.just_hit_gate);
        assert_eq!(s.drift.state.gate_score, 0);
        // A combo running, then a collision exit (code 1): lost.
        let i = &mut s.drift.inputs;
        i.drifting = true;
        i.speed = 30.0;
        i.body_slip = 45.0;
        for _ in 0..10 {
            step(&mut s, 100.0);
        }
        assert!(s.drift.state.drift > 10.0);
        s.drift.inputs.drifting = false;
        s.drift.inputs.drift_exit = 1;
        step(&mut s, 100.0);
        assert_eq!(s.drift.state.drift, 0.0);
        assert!(s.drift.state.just_lost);
        assert_eq!(s.drift.state.banked, 0);
        assert!(outs(&mut s).contains(&DriftOut::Event(DriftEvent::ComboLost)));
        // Hitting gate 3's marker before passing it: the crash event and a failed gate.
        s.drift.inputs.marker_contacts = vec![1];
        step(&mut s, 240.0);
        assert!(s.drift.state.gates[1].used && s.drift.state.just_crashed);
        assert!(outs(&mut s).contains(&DriftOut::Event(DriftEvent::GateCrashed)));
    }

    #[test]
    fn exit_medal_and_speed_points() {
        let mut s = entered();
        s.drift.state.banked = 2100;
        s.drift.state.drift = 57.0;
        assert_eq!(s.resolve_int("DriftScoring.MedalIndex"), Some(1));
        assert_eq!(s.resolve_int("DriftScoring.TargetScore"), Some(3000));
        assert_eq!(s.resolve_int("DriftScoring.TargetMedalIndex"), Some(0));
        assert_eq!(s.resolve_int("DriftScoring.Bonus"), Some(50));
        apply(&mut s, &Request::DriftScoring(Slot::Exit));
        assert_eq!(s.drift.state.banked, 2150, "the running drift banked in steps of 10");
        let p = s.players.local_player().unwrap();
        assert_eq!((p.score, p.score_348, p.final_5b8), (2150, 2150, 2));
        assert!(s.game_modes.won);
        assert!(s.requests.contains(&Request::SpeedPoints { sp: 2150, bucket: 0, car: 911 }));
        assert!(s.requests.contains(&Request::Drift(DriftOut::StopSequence { sequence: DRIFT_SEQUENCE })));
        assert_eq!(lookup(&s, &[]), Lookup::Null, "unregistered");
        // No medal: the mode is not won.
        let mut s = entered();
        s.drift.state.banked = 900;
        apply(&mut s, &Request::DriftScoring(Slot::Exit));
        assert!(!s.game_modes.won);
        assert_eq!(s.players.local_player().unwrap().final_5b8, 4);
    }

    #[test]
    fn segment_against_box() {
        let car = car_at(0.0);
        assert!(segment_hits_box([-5.0, 0.7, 0.0], [5.0, 0.7, 0.0], &car));
        assert!(!segment_hits_box([-5.0, 0.7, 3.0], [5.0, 0.7, 3.0], &car));
        assert!(!segment_hits_box([-5.0, 0.7, 0.0], [-2.0, 0.7, 0.0], &car));
        assert!(!segment_hits_box([-5.0, 2.0, 0.0], [5.0, 2.0, 0.0], &car));
    }
}
