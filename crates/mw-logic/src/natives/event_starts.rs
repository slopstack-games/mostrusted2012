//! The event-start markers of `EVENTSTARTS` ("Event Starts", vtable 0xfd950c, 0x1f0 B; decomp.md "Event starts"):
//! which starts show, the prompt, the burnout that starts an event, the nearest start's prefetch. Ported from the
//! code: enter 0x754f30, update `FUN_00755220` (with `FUN_0074a600` building the list, `FUN_00755160` sweeping it,
//! `FUN_0074ab90` the nearest start, `FUN_0074ad50` entering a start), exit 0x74ab80 (`FUN_0074aaf0`).
//!
//! **What the runtime fills** ([`EventStartInputs`]): the world's event list with each event's start trigger and
//! the profile checks the list build makes (each field names its function), the player's car (vehicle id,
//! position, speed, throttle / brake), the reference point (UI manager +0x2cd50), the pursuit state, the clock.
//! **What the logic decides** ([`EventStartState`]): the shown starts (+0x4c, 16 B each, at most 20), the prompt
//! (GMM+0x2c70 / +0x2ca7), the burnout hold (+0x1a8), the event chosen (`FUN_006e74f0` on the game-mode
//! manager) and the exit through `EXIT` with `CHECKENTRY`. **What the runtime carries out**: [`EventStartOut`]
//! (the markers, the hint, the Autolog recommendation start, the nearest start's prefetch).

use super::LogicState;
use super::state::Request;
use super::trigger_boxes::{V3, dist2, sub};

/// At most this many starts (`FUN_0074a600`: 0x14).
pub const MAX_STARTS: usize = 20;
/// The reference point is taken again after it moved 10 m (`.rdata` 0x00fd6790 = 100, squared).
pub const REFRESH_D2: f32 = 100.0;
/// Height differences count 5× in the start radius (`.rdata` 0x00fd67b4).
pub const HEIGHT_WEIGHT: f32 = 5.0;
/// Throttle and brake count as held above this (`.rdata` 0x00f8a810).
pub const INPUT_HELD: f32 = 0.5;
/// Seconds of burnout that start the event (`.rdata` 0x00f8a820).
pub const HOLD_SECONDS: f32 = 1.0;
/// The nearest start is looked for every 2 s (`.rdata` 0x00fd3434) ...
pub const NEAREST_PERIOD: f32 = 2.0;
/// ... within 500 m (`.rdata` 0x00fd2498 = 250000, squared).
pub const NEAREST_D2: f32 = 250_000.0;
/// In the intro free drive only this event shows (`FUN_0074a600`: 0x1d21f2).
pub const INTRO_EVENT: u32 = 0x1d_21f2;

/// The behaviour's parameters (enter 0x754f30, read by name).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EventStartParams {
    /// `CHECKENTRY` (+0x1ad): entering a start exits the state.
    pub check_entry: bool,
    /// `POIID` (+0x190), `RECPOIID` (+0x194), `MWPOIIDC` (+0x198: a Most Wanted race not won yet), `MWPOIIDUC`
    /// (+0x19c: one won): the marker looks.
    pub poi: u32,
    pub rec_poi: u32,
    pub mw_poi: u32,
    pub mw_poi_won: u32,
    /// `TRACKEDONLY` (+0x1ae): only the tracked event (UI manager +0x2c8e4) can be entered.
    pub tracked_only: bool,
    /// `RSPCTRADIUS` (+0x1af): starts beyond `ACTIVERADIUS` of the reference point are left out.
    pub respect_radius: bool,
    /// `ACTIVERADIUS` (`atof`; enter keeps its square at +0x1c0).
    pub active_radius: f32,
}

/// One event of the world's list (GMM+4 → +0x11c / +0x120) with what the list build asks about it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StartEvent {
    /// +0x18.
    pub event: u32,
    /// +0x1c: the game mode.
    pub mode: u32,
    /// `FUN_00ac54c0`(0x5f3f3): an `OfflineEvent`.
    pub offline: bool,
    /// +0xe9: a Most Wanted race.
    pub most_wanted: bool,
    /// `FUN_006e3a40`: the Most Wanted race has been won.
    pub most_wanted_won: bool,
    /// `FUN_006e5f00`: a race event (its hint depends on the laps).
    pub race: bool,
    /// +0xf6.
    pub laps: u8,
    /// `FUN_006e56f0` → `FUN_0053c020`: the start trigger and its position (`None`: no such trigger).
    pub start: Option<(u32, V3)>,
    /// +0x5c `AllowedVehicleList` (`FUN_006eca10`(list, car, 1): empty = any car).
    pub allowed_vehicles: Vec<u32>,
    /// The profile checks: `FUN_00688f60` and `FUN_0068f190` (unlocked, owned), the game pack (`FUN_00643c10` /
    /// `FUN_006401b0`), the event state (`FUN_006f0b60` not 2 / 3, or `FUN_006402e0`).
    pub unlocked: bool,
    /// `FUN_006e58a0`: available in the selected world's list.
    pub available: bool,
    /// `FUN_00723b10`(event): an Autolog recommendation, its index.
    pub recommendation: Option<u32>,
}

/// What the runtime supplies each frame.
#[derive(Debug, Clone, Default)]
pub struct EventStartInputs {
    pub events: Vec<StartEvent>,
    /// The local player's car (`FUN_006788d0` of the player's vtable +0x18; +0x1a4).
    pub vehicle: u32,
    /// The local player's position (`FUN_006b9ea0`).
    pub position: V3,
    /// |speed| in mph (`FUN_006ba3e0`).
    pub speed_mph: f32,
    /// Player 0's input record +0x30 / +0x3c (`FUN_0047aa20`(0)): throttle and brake.
    pub throttle: f32,
    pub brake: f32,
    /// The reference point (UI manager +0x2cd50).
    pub reference: V3,
    /// `FUN_006c2410`: the local player is in a pursuit.
    pub in_pursuit: bool,
    /// The game clock (`FUN_0046ee10`): frame time × scale; `paused` = its +0x1c clear.
    pub dt: f32,
    pub paused: bool,
    /// GMM+0x2e83 (read on enter and every frame; meaning open): set, the radius is not respected.
    pub gmm_2e83: bool,
    /// `FUN_006f0f00`(1) clear or (2) set, and `FUN_006c2d80` clear: the list may be built.
    pub list_allowed: bool,
    /// `FUN_006e0430`: the intro free drive.
    pub intro: bool,
    /// `FUN_006bef40`: some player slot is occupied.
    pub any_player: bool,
    /// G+0x2f454 +0x5a58 == −1: the nearest-start prefetch is idle.
    pub prefetch_idle: bool,
    /// The UI manager's +0x12568 is idle (+0x1258c / +0x12590 = `DAT_0117ab84` / 88, +0x125a0, +0x125a4, +0x1256c
    /// all 0): a started event leaves the state.
    pub ui_idle: bool,
    /// `GameLogicConfig`: +0xd8 (`1b3c1ddc`, the free-drive mode, never listed), +0x244 (`49caf423`, the start
    /// radius 22 m), +0x284 (`ff4aef6f`, 70 mph), +0x160 (`PromptEvent`, 18364), +0x1ec (speed run 955569), +0x60
    /// (ambush 390185).
    pub freedrive_mode: u32,
    pub start_radius: f32,
    pub max_mph: f32,
    pub prompt: u32,
    pub speed_run_mode: u32,
    pub ambush_mode: u32,
}

/// One shown start (+0x4c, 16 B: event, trigger, marker handle, remove flag +0xc, recommended +0xd).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShownStart {
    pub event: u32,
    pub trigger: u32,
    pub position: V3,
    pub key: u32,
    pub remove: bool,
    pub recommended: bool,
}

/// The behaviour's state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventStartState {
    pub params: EventStartParams,
    pub shown: Vec<ShownStart>,
    /// +0x48: the nearest-start timer.
    pub nearest_timer: f32,
    /// +0x1a0: the tracked event kept while a pursuit hides the starts.
    pub saved_tracked: u32,
    /// UI manager +0x2c8e4: the tracked event (the map's GPS destination; the runtime sets it, the logic clears it).
    pub tracked: u32,
    /// +0x1a4: the car the list was built for.
    pub vehicle: u32,
    /// +0x1a8: the burnout hold (s).
    pub hold: f32,
    /// +0x1ac: hidden by a pursuit.
    pub hidden: bool,
    /// +0x1b0: GMM+0x2e83 last frame.
    pub flag_2e83: bool,
    /// +0x1b1: an event was started (waiting for the UI).
    pub started: bool,
    /// +0x1d0: the reference point the list was built around.
    pub reference: V3,
    /// +0x1e0: GMM+0x2e7a (`StoryMode`) + 2 × `FUN_006e0520` (always 0).
    pub mode_bits: u32,
    /// The update returned 1 this frame (`Query::EventStartTriggered`).
    pub triggered: bool,
}

/// The logic's event-start manager.
#[derive(Debug, Clone, Default)]
pub struct EventStarts {
    pub inputs: EventStartInputs,
    pub state: EventStartState,
}

/// What `EVENTSTARTS` asks the engine to do.
#[derive(Debug, Clone, PartialEq)]
pub enum EventStartOut {
    /// `FUN_00793350`(key, position, 0, 1, recommendation): a point of interest at the start (yaw 0; with a
    /// recommendation also `FUN_00723e50`).
    Marker { event: u32, trigger: u32, position: V3, key: u32, recommendation: Option<u32> },
    /// `FUN_00790da0`: the start's point of interest goes.
    MarkerRemove { event: u32 },
    /// `FUN_006f3f30`(hint): 0xe a race (one lap), 0xf a circuit, 0x11 a speed run, 0x12 an ambush.
    Hint { hint: u32 },
    /// `FUN_00725130`(index, 0, 0): the started event is an Autolog recommendation (instead of `FUN_006e74f0`).
    Recommendation { index: u32 },
    /// `FUN_00723b10`(event, .., 1) and `FUN_006f1dc0`(event): the nearest start within 500 m (its leaderboard).
    Nearest { event: u32 },
}

fn out(s: &mut LogicState, o: EventStartOut) {
    s.requests.push(Request::EventStart(o));
}

/// The `EVENTSTARTS` requests the logic carries out (from [`super::LogicHost::apply`]).
pub fn apply(s: &mut LogicState, r: &Request) {
    match r {
        Request::EventStartParams(p) => enter(s, *p),
        Request::EventStartsStep => s.event_starts.state.triggered = update(s),
        Request::EventStartsLeave => remove_all(s),
        _ => {}
    }
}

/// Enter (0x754f30): the parameters, the reference point and GMM+0x2e83 taken, hold / pursuit / started / list
/// cleared, the list built (with car 0: +0x1a4 is cleared after, so the first update rebuilds for the real car).
pub fn enter(s: &mut LogicState, params: EventStartParams) {
    let tracked = s.event_starts.state.tracked;
    s.event_starts.state = EventStartState {
        params,
        reference: s.event_starts.inputs.reference,
        flag_2e83: s.event_starts.inputs.gmm_2e83,
        mode_bits: s.game_modes.story_mode as u32,
        tracked,
        ..Default::default()
    };
    build(s);
}

/// `FUN_0074aaf0`: every marker goes, the list is empty.
pub fn remove_all(s: &mut LogicState) {
    for e in std::mem::take(&mut s.event_starts.state.shown) {
        out(s, EventStartOut::MarkerRemove { event: e.event });
    }
}

/// The update (`FUN_00755220`); true = exit through `EXIT` (only with `CHECKENTRY`).
pub fn update(s: &mut LogicState) -> bool {
    let vehicle = s.event_starts.inputs.vehicle;
    let st = &mut s.event_starts.state;
    let old = std::mem::replace(&mut st.vehicle, vehicle);
    let bits = s.game_modes.story_mode as u32;
    let mut rebuild = bits != st.mode_bits || vehicle != old;
    st.mode_bits = bits;
    let f = s.event_starts.inputs.gmm_2e83;
    if !f && st.params.respect_radius && dist2(st.reference, s.event_starts.inputs.reference) >= REFRESH_D2 {
        st.reference = s.event_starts.inputs.reference;
        rebuild = true;
    }
    if f != st.flag_2e83 {
        rebuild = true;
        st.flag_2e83 = f;
    }
    nearest(s);
    if rebuild {
        s.event_starts.state.shown.iter_mut().for_each(|e| e.remove = true);
        if !s.event_starts.state.hidden {
            build(s);
        }
        sweep(s);
    }
    if !s.event_starts.inputs.in_pursuit {
        if s.event_starts.state.hidden {
            s.event_starts.state.tracked = s.event_starts.state.saved_tracked;
            build(s);
        }
        s.event_starts.state.hidden = false;
        if s.event_starts.state.params.check_entry
            && s.event_starts.inputs.speed_mph.abs() < s.event_starts.inputs.max_mph
            && enter_start(s)
        {
            return true;
        }
    } else {
        if !s.event_starts.state.hidden {
            let st = &mut s.event_starts.state;
            st.saved_tracked = std::mem::take(&mut st.tracked);
            remove_all(s);
        }
        s.event_starts.state.hidden = true;
    }
    false
}

/// `FUN_0074a600`: the world's events in order: offline events not of the free-drive mode, unlocked, allowed for
/// the car (in the intro only 0x1d21f2), with a start trigger, within the radius unless it is not respected (or the
/// event is a Most Wanted race not won yet), available. A start already shown with the same recommendation state
/// stays; otherwise a new marker is added while fewer than 20 are listed (entries still flagged for removal
/// count).
pub fn build(s: &mut LogicState) {
    let i = &s.event_starts.inputs;
    if !i.list_allowed {
        return;
    }
    let st = &s.event_starts.state;
    let r2 = st.params.active_radius * st.params.active_radius;
    let mut adds = Vec::new();
    let mut shown = st.shown.clone();
    for e in &i.events {
        if !e.offline {
            continue;
        }
        let mw_not_won = e.most_wanted && !e.most_wanted_won;
        let respect = st.params.respect_radius && !st.flag_2e83 && !mw_not_won;
        if e.mode == i.freedrive_mode || !e.unlocked {
            continue;
        }
        if !e.allowed_vehicles.is_empty() && !e.allowed_vehicles.contains(&st.vehicle) {
            continue;
        }
        if i.intro && e.event != INTRO_EVENT {
            continue;
        }
        let Some((trigger, position)) = e.start else { continue };
        if respect && dist2(position, st.reference) > r2 {
            continue;
        }
        if !e.available {
            continue;
        }
        let recommended = e.recommendation.is_some();
        if let Some(old) = shown.iter_mut().find(|o| o.trigger == trigger && o.event == e.event) {
            if old.recommended == recommended {
                old.remove = false;
                continue;
            }
            old.remove = true;
        }
        if shown.len() >= MAX_STARTS {
            continue;
        }
        let p = &st.params;
        let key = if recommended {
            p.rec_poi
        } else if e.most_wanted {
            if mw_not_won { p.mw_poi } else { p.mw_poi_won }
        } else {
            p.poi
        };
        shown.push(ShownStart { event: e.event, trigger, position, key, remove: false, recommended });
        adds.push(EventStartOut::Marker { event: e.event, trigger, position, key, recommendation: e.recommendation });
    }
    s.event_starts.state.shown = shown;
    for a in adds {
        out(s, a);
    }
}

/// `FUN_00755160`: the starts flagged for removal go (each replaced by the last).
fn sweep(s: &mut LogicState) {
    let mut i = 0;
    while i < s.event_starts.state.shown.len() {
        if s.event_starts.state.shown[i].remove {
            let e = s.event_starts.state.shown.swap_remove(i);
            out(s, EventStartOut::MarkerRemove { event: e.event });
        } else {
            i += 1;
        }
    }
}

/// `FUN_0074ab90`: every 2 s of game clock the nearest shown start within 500 m of the player is prefetched while
/// the prefetch is idle.
fn nearest(s: &mut LogicState) {
    let i = &s.event_starts.inputs;
    if !i.any_player {
        return;
    }
    let st = &mut s.event_starts.state;
    if !i.paused {
        st.nearest_timer += i.dt;
    }
    if st.nearest_timer < NEAREST_PERIOD {
        return;
    }
    st.nearest_timer = 0.0;
    let best = st
        .shown
        .iter()
        .filter(|e| e.event != 0)
        .map(|e| (dist2(e.position, i.position), e.event))
        .fold(None, |b: Option<(f32, u32)>, c| if b.is_none_or(|b| c.0 < b.0) { Some(c) } else { b });
    if let Some((d2, event)) = best
        && d2 < NEAREST_D2
        && i.prefetch_idle
    {
        out(s, EventStartOut::Nearest { event });
    }
}

/// `FUN_0074ad50` (below the start speed, no pursuit, no swap under way): within the start radius (height ×5) of
/// a start (the tracked one with `TRACKEDONLY`) the prompt shows; throttle and brake both held add the clock's
/// time to the hold; above 1 s the event is chosen (`FUN_006e74f0`, or the recommendation), its hint shown, and
/// once the UI is idle the state exits. The hold resets on a frame without a burnout.
fn enter_start(s: &mut LogicState) -> bool {
    if s.event_starts.inputs.in_pursuit || s.game_modes.swap_in_progress {
        return false;
    }
    let i = s.event_starts.inputs.clone();
    let mut holding = false;
    for k in 0..s.event_starts.state.shown.len() {
        let e = s.event_starts.state.shown[k];
        let tracked = s.event_starts.state.tracked;
        if s.event_starts.state.params.tracked_only && tracked != e.event {
            continue;
        }
        let d = sub(e.position, i.position);
        let dy = HEIGHT_WEIGHT * d[1];
        if d[0] * d[0] + dy * dy + d[2] * d[2] >= i.start_radius * i.start_radius {
            continue;
        }
        s.game_modes.prompt = Some(i.prompt);
        if !(i.throttle > INPUT_HELD && i.brake > INPUT_HELD) {
            continue;
        }
        if !i.paused {
            s.event_starts.state.hold += i.dt;
            holding = true;
        }
        if s.event_starts.state.hold > HOLD_SECONDS {
            let ev = i.events.iter().find(|c| c.event == e.event);
            match ev.and_then(|c| c.recommendation) {
                Some(index) => out(s, EventStartOut::Recommendation { index }),
                None => s.game_modes.request_event(e.event, false),
            }
            if tracked == e.event {
                s.event_starts.state.tracked = 0;
            }
            if let Some(c) = ev {
                let hint = if c.race {
                    Some(if c.laps < 2 { 0xe } else { 0xf })
                } else if c.mode == i.speed_run_mode {
                    Some(0x11)
                } else if c.mode == i.ambush_mode {
                    Some(0x12)
                } else {
                    None
                };
                if let Some(hint) = hint {
                    out(s, EventStartOut::Hint { hint });
                }
            }
            s.event_starts.state.started = true;
        }
    }
    s.game_modes.event_start_held = holding;
    let st = &mut s.event_starts.state;
    if !st.started {
        if !holding {
            st.hold = 0.0;
        }
        return false;
    }
    if i.ui_idle {
        st.started = false;
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> LogicState {
        let mut s = LogicState::default();
        let ev = |event: u32, x: f32| StartEvent {
            event,
            mode: 730_000,
            offline: true,
            race: true,
            laps: 1,
            start: Some((event + 1, [x, 0.0, 0.0])),
            unlocked: true,
            available: true,
            ..Default::default()
        };
        let i = &mut s.event_starts.inputs;
        i.events = vec![
            ev(100, 50.0),
            ev(200, 3000.0),
            StartEvent { allowed_vehicles: vec![77], ..ev(300, 60.0) },
            StartEvent { most_wanted: true, ..ev(400, 9000.0) },
            StartEvent { mode: 390_145, ..ev(500, 70.0) },
        ];
        i.vehicle = 5;
        i.list_allowed = true;
        i.any_player = true;
        i.ui_idle = true;
        i.dt = 0.25;
        i.freedrive_mode = 390_145;
        i.start_radius = 22.0;
        i.max_mph = 70.0;
        i.prompt = 18_364;
        let params = EventStartParams {
            check_entry: true,
            poi: 1_013_512,
            mw_poi: 955_952,
            mw_poi_won: 955_955,
            respect_radius: true,
            active_radius: 2000.0,
            ..Default::default()
        };
        enter(&mut s, params);
        s
    }

    fn shown(s: &LogicState) -> Vec<(u32, u32)> {
        s.event_starts.state.shown.iter().map(|e| (e.event, e.key)).collect()
    }

    #[test]
    fn which_starts_show() {
        let mut s = world();
        assert!(!update(&mut s));
        // 200 is beyond 2000 m, 300 needs car 77, 500 is free drive; the Most Wanted race shows at any distance.
        assert_eq!(shown(&s), [(100, 1_013_512), (400, 955_952)]);
        s.event_starts.inputs.vehicle = 77;
        update(&mut s);
        assert_eq!(shown(&s), [(100, 1_013_512), (400, 955_952), (300, 1_013_512)]);
        s.event_starts.inputs.in_pursuit = true;
        update(&mut s);
        assert!(shown(&s).is_empty(), "a pursuit hides them");
        s.event_starts.inputs.in_pursuit = false;
        update(&mut s);
        assert_eq!(shown(&s).len(), 3);
    }

    #[test]
    fn burnout_in_a_start_starts_the_event() {
        let mut s = world();
        update(&mut s);
        s.event_starts.inputs.position = [45.0, 0.5, 3.0];
        s.game_modes.prompt = None;
        assert!(!update(&mut s));
        assert_eq!(s.game_modes.prompt, Some(18_364), "the prompt shows inside the start");
        s.event_starts.inputs.throttle = 1.0;
        s.event_starts.inputs.brake = 1.0;
        for _ in 0..4 {
            assert!(!update(&mut s));
        }
        assert!(s.game_modes.event_start_held);
        // 5 × 0.25 s > 1 s: chosen and (the UI idle) exits.
        assert!(update(&mut s));
        assert_eq!(s.game_modes.requested_event, 100);
        assert!(s.requests.contains(&Request::EventStart(EventStartOut::Hint { hint: 0xe })));
        // Letting go resets the hold; too fast shows no prompt.
        s.event_starts.inputs.throttle = 0.0;
        update(&mut s);
        assert_eq!(s.event_starts.state.hold, 0.0);
        s.event_starts.inputs.speed_mph = 80.0;
        s.game_modes.prompt = None;
        update(&mut s);
        assert_eq!(s.game_modes.prompt, None);
    }
}
