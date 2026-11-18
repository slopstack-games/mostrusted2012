//! The game's speech manager (G+0x2c3c0; docs/decomp.md "Radio"): the speech events of
//! `EN_US/SPEECH/SPEECHEVENTSEQUENCES.BNDL` (parsed by `mw_assets::police_speech`), which one plays, when it starts,
//! which line it says. Engine-free: the host evaluates the events' condition bindings and plays the lines
//! ([`SpeechOut`]).
//!
//! - **Requests** (`FUN_008408a0` → `FUN_008406a0`): [`SpeechManager::request`] answers as the original does
//!   ([`PLAYED`] 0x13 when the event became the current one, else why not). One speech at a time and no queue: a
//!   request while another is pending (state 8) or playing (state 9) replaces it only with a lower `Priority`
//!   number (or the forced flag); while playing, only when the new event has lines (vtable +0x1c, `FUN_0083c6b0`),
//!   the old one is stopped and the new one skips its `Delay` (+0x24).
//! - **Eligibility** (`FUN_0083d270`): fewer plays than the maximum (+0x49, 0 = no limit), inside the mode-time
//!   window (GMM+0x2d28 in minutes: at least +0x38, at most +0x34 when that is > 0), at least `Period` minutes since
//!   it last started (the event clock +0x1c, reset by each start, reads 0 while it plays), the player's car when
//!   +0x47 asks, not `Cutscene_Behaviour` 2 in a cutscene, a one-off (+0x46) not heard before.
//! - **Update** (`FUN_00842ba0`): a pending event starts unless a cutscene runs and its `Cutscene_Behaviour` is not 0;
//!   a playing one keeps going outside cutscenes (or with behaviour 0); in one, with behaviour 1 within its first
//!   second (`DAT_00f8a820` = 1.0) it is stopped and requested again (it waits for the cutscene's end), else
//!   stopped. Then every event's own update (vtable 0xf994d8 +8, 0x845b10): its clock runs; started (3) it waits
//!   `Delay` (not after an interruption), then its sequence plays (4) until it ends (2); stopped (5) its voices
//!   end (2).
//! - **Stop** (`StopSpeech` → `FUN_008423a0` states 8 / 9): the current event stops, except event 0x1c8a28
//!   ([`UNSTOPPABLE`]), and the random state is reset (seed 0x31613979fb637ec0, 8 steps).
//! - **Lines** (`FUN_008363c0`): an item with lines of its own says one picked uniformly with the LCG
//!   (×0x5851f42d4c957f2d + 1, the high word of the old state mod n).
//!
//! MODEL (not traced in the code): the sequence's timing: items start at their `StartTime` on the event's clock
//! and a line holds that clock at its item's `EndTime` until it has been said (the line item registers itself as
//! the sequencer's +0x58 while its sound plays, `FUN_00830fe0`, and reports the sound's position as its time; the
//! generic sequencer `FUN_00a43c10` that reads that was not followed), so the 1 s gaps between items stay gaps
//! between lines; the sequence is over when nothing is left to start or say; the item `Probability` as a percentage draw; the container pick (`FUN_00833cb0`
//! picks by `Speaker` / `Character` so voices do not repeat, `FUN_0083d3e0` the line: here uniform); an
//! event item plays one of its events inline; the vocabulary lines (car model / colour, `FUN_00834130`) are not
//! said, so an event of only those is not eligible; the plays counter (+0x25) counts starts (its increment was
//! not found); the one-off set is the session's (the original keeps it in the profile's `OneOffSpeech` table,
//! `FUN_0072f0e0` / `FUN_007293f0`); the reseed of `FUN_00415fc0` after the fixed seed is left out.

use std::collections::{HashMap, HashSet};

/// A line to say: its wave, the voice's channel gain, its length (s; `WaveDictionary`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    pub wave: u32,
    pub gain: f32,
    pub duration: f32,
}

#[derive(Debug, Clone)]
pub enum SpeechItem {
    /// `ScriptLineSequenceItem`: from `start` to `end` s, with `probability` (0–1), one of its own `lines`, else
    /// one of the `containers`' lines; `vocabulary` (`9f199cf7`) names a vocabulary slot (not ported).
    Lines { start: f32, end: f32, probability: f32, lines: Vec<Line>, containers: Vec<Vec<Line>>, vocabulary: u16 },
    /// `SpeechEventSequenceItem`: one of these events at `start` s.
    Events { start: f32, end: f32, events: Vec<u32> },
}

impl SpeechItem {
    pub fn start(&self) -> f32 {
        match self {
            SpeechItem::Lines { start, .. } | SpeechItem::Events { start, .. } => *start,
        }
    }

    pub fn end(&self) -> f32 {
        match self {
            SpeechItem::Lines { end, .. } | SpeechItem::Events { end, .. } => *end,
        }
    }

    /// Every line it can say.
    pub fn all_lines(&self) -> impl Iterator<Item = &Line> {
        let (own, containers): (&[Line], &[Vec<Line>]) = match self {
            SpeechItem::Lines { lines, containers, .. } => (lines, containers),
            SpeechItem::Events { .. } => (&[], &[]),
        };
        own.iter().chain(containers.iter().flatten())
    }
}

/// A `Speech.SpeechEventSequence` (76 bytes; docs/formats.md "Speech").
#[derive(Debug, Clone, Default)]
pub struct SpeechEvent {
    /// The UniqueId the game requests.
    pub id: u32,
    pub items: Vec<SpeechItem>,
    /// `a137b739` (+0x2c): binding strings, all must hold with `all_conditions` (+0x44), else any.
    pub conditions: Vec<String>,
    pub all_conditions: bool,
    /// +0x30, s before the sequence plays.
    pub delay: f32,
    /// +0x3c, minutes since it last started.
    pub period: f32,
    /// +0x34 (`4246e1c3`): latest mode time (minutes; 0 = none).
    pub latest: f32,
    /// +0x38 (`ef972179`): earliest mode time (minutes).
    pub earliest: f32,
    /// +0x40 `Cutscene_Behaviour`: 0 plays on, 1 waits (restarts when stopped in its first second), 2 never in one.
    pub cutscene_behaviour: u8,
    /// +0x48, 0 the most important.
    pub priority: u8,
    /// +0x49, 0 = no limit.
    pub max_plays: u8,
    /// +0x46.
    pub one_off: bool,
    /// +0x47.
    pub needs_car: bool,
}

impl SpeechEvent {
    /// `FUN_0083c6b0` (vtable +0x1c): something to say. MODEL: vocabulary slots count as nothing.
    pub fn has_lines(&self) -> bool {
        self.items.iter().any(|i| match i {
            SpeechItem::Lines { .. } => i.all_lines().next().is_some(),
            SpeechItem::Events { events, .. } => !events.is_empty(),
        })
    }
}

/// What the manager reads from the game.
pub trait SpeechWorld {
    /// A condition binding (`FUN_00a4f160` on the view-model root G+0xe8), test included.
    fn condition(&mut self, binding: &str) -> bool;
    /// The game mode's clock GMM+0x2d28 in minutes.
    fn mode_minutes(&self) -> f32;
    /// `FUN_006c36c0`: the local player has a car.
    fn has_car(&self) -> bool;
    /// +0x29a8: a cutscene runs.
    fn in_cutscene(&self) -> bool;
}

/// The manager's answers to a request (`FUN_008406a0` / `FUN_008408a0`).
pub const NOT_LOADED: u32 = 1;
pub const UNKNOWN_EVENT: u32 = 3;
pub const NOT_ELIGIBLE: u32 = 4;
pub const CONDITIONS_FAILED: u32 = 0xe;
pub const NOTHING_TO_SAY: u32 = 0x10;
pub const LESS_IMPORTANT: u32 = 0x11;
pub const AS_IMPORTANT: u32 = 0x12;
pub const PLAYED: u32 = 0x13;
/// The event `StopSpeech` does not stop (`FUN_008423a0`).
pub const UNSTOPPABLE: u32 = 0x1c8a28;

/// What the host does.
#[derive(Debug, Clone, PartialEq)]
pub enum SpeechOut {
    /// Say `line` for `event`.
    Say { event: u32, line: Line },
    /// Silence `event`'s voices.
    Stop { event: u32 },
}

const LCG_MUL: u64 = 0x5851_f42d_4c95_7f2d;
const SEED: u64 = 0x3161_3979_fb63_7ec0;
/// `DAT_00f8a820`: a speech stopped by a cutscene within this many seconds is requested again.
const RESTART_WINDOW: f32 = 1.0;

/// Guard (not from the game): event items expanded inline per update of one sequence, so a cycle of events at
/// zero delay (an event that plays itself) cannot hang the frame.
const MAX_INLINE_EVENTS: usize = 64;

/// An event's runtime record (0x40 bytes at manager +0x400).
#[derive(Debug, Default)]
struct EventState {
    /// +0x30: 2 idle, 3 started (waiting its delay), 4 playing, 5 stopping.
    state: u8,
    /// +0x1c: seconds since it last started (`Period` × 60 at load).
    clock: f32,
    /// +0x24: started after an interruption: no delay.
    restarted: bool,
    /// +0x25.
    plays: u8,
    /// +0x10 / +0x14 / +0x18: the request's arguments.
    args: [i32; 3],
    run: Option<Run>,
}

/// The playing sequence (MODEL timing, see the module doc).
#[derive(Debug, Default)]
struct Run {
    /// The sequence clock.
    t: f32,
    /// Items to start: (start, end) on the sequence clock.
    items: Vec<(f32, f32, SpeechItem)>,
    /// Lines sounding: their item's end on the sequence clock, seconds left.
    lines: Vec<(f32, f32)>,
}

/// The speech manager (states 7 idle, 8 pending, 9 playing; the loading states are the host's).
#[derive(Debug)]
pub struct SpeechManager {
    events: HashMap<u32, SpeechEvent>,
    states: HashMap<u32, EventState>,
    /// +0x2808.
    state: u8,
    /// +0x2810.
    current: Option<u32>,
    /// +0x299c: how long the current one has played outside cutscenes.
    played: f32,
    rng: u64,
    heard: HashSet<u32>,
}

impl SpeechManager {
    /// The events loaded (`FUN_008423a0` state 6: `FUN_0083d170` per event, 0x840a40 each), state 7.
    pub fn new(events: impl IntoIterator<Item = SpeechEvent>) -> Self {
        let events: HashMap<u32, SpeechEvent> = events.into_iter().map(|e| (e.id, e)).collect();
        let states = events
            .values()
            .map(|e| (e.id, EventState { state: 2, clock: e.period * 60.0, args: [-1, -1, 0], ..Default::default() }))
            .collect();
        let mut m = Self { events, states, state: 7, current: None, played: 0.0, rng: SEED, heard: HashSet::new() };
        m.reseed();
        m
    }

    pub fn event(&self, id: u32) -> Option<&SpeechEvent> {
        self.events.get(&id)
    }

    /// `FUN_0083c530`: the event pending or playing.
    pub fn current(&self) -> Option<u32> {
        self.current
    }

    /// Times `id` has played (`FUN_0083d250`).
    pub fn plays(&self, id: u32) -> u8 {
        self.states.get(&id).map_or(0, |s| s.plays)
    }

    fn reseed(&mut self) {
        self.rng = SEED;
        for _ in 0..8 {
            self.rng = self.rng.wrapping_mul(LCG_MUL).wrapping_add(1);
        }
    }

    /// The high word of the state mod `n`, then a step (`FUN_008363c0`, `FUN_0083c5a0`).
    fn random(&mut self, n: usize) -> usize {
        let s = self.rng;
        self.rng = s.wrapping_mul(LCG_MUL).wrapping_add(1);
        ((s >> 32) as u32 % n.max(1) as u32) as usize
    }

    /// `FUN_0083d270`.
    fn eligible(&self, ev: &SpeechEvent, world: &dyn SpeechWorld) -> bool {
        let Some(st) = self.states.get(&ev.id) else { return false };
        // MODEL: nothing we can say (vocabulary only).
        if !ev.has_lines() {
            return false;
        }
        if ev.max_plays != 0 && st.plays >= ev.max_plays {
            return false;
        }
        let minutes = world.mode_minutes();
        if minutes < ev.earliest || (ev.latest > 0.0 && ev.latest < minutes) {
            return false;
        }
        let since = if st.state == 2 { st.clock } else { 0.0 } / 60.0;
        if since < ev.period {
            return false;
        }
        if ev.needs_car && !world.has_car() {
            return false;
        }
        if world.in_cutscene() && ev.cutscene_behaviour == 2 {
            return false;
        }
        !(ev.one_off && self.heard.contains(&ev.id))
    }

    /// `FUN_0041fbf0` (vtable +0x34): the conditions, all or any.
    fn conditions(ev: &SpeechEvent, world: &mut dyn SpeechWorld) -> bool {
        let mut ok = ev.all_conditions;
        for c in &ev.conditions {
            let v = world.condition(c);
            if ev.all_conditions {
                ok &= v;
            } else {
                ok |= v;
            }
        }
        ok
    }

    /// The event's conditions hold (what a gameplay trigger checks, `FUN_006e0a90`).
    pub fn conditions_hold(&self, id: u32, world: &mut dyn SpeechWorld) -> bool {
        self.events.get(&id).is_some_and(|e| Self::conditions(e, world))
    }

    /// `FUN_008408a0`(id, args) → `FUN_008406a0`(event, `force`, args): the answer codes above.
    pub fn request(&mut self, id: u32, force: bool, args: [i32; 3], world: &mut dyn SpeechWorld) -> u32 {
        let Some(ev) = self.events.get(&id) else { return UNKNOWN_EVENT };
        if !Self::conditions(ev, world) {
            return CONDITIONS_FAILED;
        }
        if !self.eligible(ev, &*world) {
            return NOT_ELIGIBLE;
        }
        let (priority, has_lines, one_off) = (ev.priority, ev.has_lines(), ev.one_off);
        let current_priority = self.current.and_then(|c| self.events.get(&c)).map_or(u8::MAX, |c| c.priority);
        match self.state {
            7 => {
                self.take(id, args, one_off);
                self.state = 8;
                PLAYED
            }
            8 if priority < current_priority || force => {
                self.take(id, args, one_off);
                PLAYED
            }
            9 if priority < current_priority || force => {
                if !has_lines {
                    if let Some(s) = self.states.get_mut(&id) {
                        s.args = [-1, -1, 0];
                    }
                    return NOTHING_TO_SAY;
                }
                if let Some(old) = self.current {
                    self.stop_event(old);
                }
                self.take(id, args, one_off);
                if let Some(s) = self.states.get_mut(&id) {
                    s.restarted = true;
                }
                self.state = 8;
                PLAYED
            }
            8 | 9 if priority == current_priority => AS_IMPORTANT,
            8 | 9 => LESS_IMPORTANT,
            _ => 0,
        }
    }

    /// The event becomes the current one; a one-off is remembered (`FUN_0083c4c0`).
    fn take(&mut self, id: u32, args: [i32; 3], one_off: bool) {
        self.current = Some(id);
        if let Some(s) = self.states.get_mut(&id) {
            s.args = args;
        }
        if one_off {
            self.heard.insert(id);
        }
    }

    /// Vtable +0x14 (0x83c660): an active event stops (5).
    fn stop_event(&mut self, id: u32) {
        if let Some(s) = self.states.get_mut(&id)
            && s.state > 2
        {
            s.state = 5;
        }
    }

    /// `StopSpeech` (`FUN_008423a0` in states 7–9): false when the current event cannot be stopped.
    pub fn stop(&mut self) -> bool {
        if self.state == 8 || self.state == 9 {
            if let Some(c) = self.current {
                if c == UNSTOPPABLE {
                    return false;
                }
                self.stop_event(c);
                self.current = None;
            }
            self.state = 7;
        }
        if self.state == 7 {
            self.reseed();
            self.current = None;
        }
        true
    }

    /// One frame (`FUN_00842ba0`), `dt` the game clock's step.
    pub fn update(&mut self, dt: f32, world: &mut dyn SpeechWorld) -> Vec<SpeechOut> {
        let cut = world.in_cutscene();
        match (self.state, self.current) {
            (8, Some(c)) => {
                let behaviour = self.events.get(&c).map_or(0, |e| e.cutscene_behaviour);
                if !cut || behaviour == 0 {
                    // Vtable +0x10 (0x83c640).
                    if let Some(s) = self.states.get_mut(&c) {
                        if s.state == 5 {
                            s.restarted = true;
                        }
                        s.clock = 0.0;
                        s.state = 3;
                        s.plays = s.plays.saturating_add(1);
                    }
                    self.played = 0.0;
                    self.state = 9;
                }
            }
            (9, Some(c)) => {
                let behaviour = self.events.get(&c).map_or(0, |e| e.cutscene_behaviour);
                let active = self.states.get(&c).is_some_and(|s| s.state > 2);
                let mut idle = true;
                if active {
                    if !cut || behaviour == 0 {
                        self.played += dt;
                        idle = false;
                    } else if self.played < RESTART_WINDOW && behaviour == 1 {
                        let args = self.states.get(&c).map_or([-1, -1, 0], |s| s.args);
                        self.stop_event(c);
                        self.state = 7;
                        self.current = None;
                        if self.request(c, false, args, world) != PLAYED {
                            self.current = None;
                            self.state = 7;
                        }
                        idle = false;
                    } else {
                        self.stop_event(c);
                    }
                }
                if idle {
                    self.current = None;
                    self.state = 7;
                }
            }
            (8 | 9, None) => self.state = 7,
            _ => {}
        }
        let mut out = Vec::new();
        let mut ids: Vec<u32> = self.states.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            self.event_update(id, dt, &mut out);
        }
        out
    }

    /// Vtable +8 (0x845b10).
    fn event_update(&mut self, id: u32, dt: f32, out: &mut Vec<SpeechOut>) {
        let Some(ev) = self.events.get(&id) else { return };
        let Some(s) = self.states.get_mut(&id) else { return };
        s.clock += dt;
        match s.state {
            3 => {
                if !s.restarted && ev.delay > s.clock {
                    return;
                }
                // MODEL: the flag is spent by this start.
                s.restarted = false;
                let items = ev.items.iter().map(|i| (i.start(), i.end(), i.clone())).collect();
                s.run = Some(Run { t: 0.0, items, lines: Vec::new() });
                s.state = 4;
                self.run(id, 0.0, out);
            }
            4 => self.run(id, dt, out),
            5 => {
                s.run = None;
                s.state = 2;
                out.push(SpeechOut::Stop { event: id });
            }
            _ => {}
        }
    }

    /// The sequence's items whose time has come; a line holds the clock at its item's end until it has been said;
    /// the sequence ends when nothing is left to start or say (MODEL, module doc).
    fn run(&mut self, id: u32, dt: f32, out: &mut Vec<SpeechOut>) {
        let Some(mut run) = self.states.get_mut(&id).and_then(|s| s.run.take()) else { return };
        for l in &mut run.lines {
            l.1 -= dt;
        }
        run.lines.retain(|l| l.1 > 0.0);
        let hold = run.lines.iter().map(|l| l.0).fold(f32::INFINITY, f32::min);
        run.t = (run.t + dt).min(hold.max(run.t));
        // Inline events played in this call: an event that (through its items) plays itself at no delay would
        // otherwise expand forever within one frame.
        let mut inlined = 0;
        while let Some(k) = run.items.iter().position(|(t, _, _)| *t <= run.t) {
            let (t, end, item) = run.items.remove(k);
            match item {
                SpeechItem::Lines { probability, lines, containers, .. } => {
                    if probability < 1.0 && (self.random(1000) as f32) / 1000.0 >= probability {
                        continue;
                    }
                    let line = if !lines.is_empty() {
                        lines[self.random(lines.len())]
                    } else {
                        let filled: Vec<&Vec<Line>> = containers.iter().filter(|c| !c.is_empty()).collect();
                        if filled.is_empty() {
                            continue;
                        }
                        let c = filled[self.random(filled.len())];
                        c[self.random(c.len())]
                    };
                    run.lines.push((end.max(t), line.duration));
                    out.push(SpeechOut::Say { event: id, line });
                }
                SpeechItem::Events { events, .. } => {
                    if events.is_empty() || inlined >= MAX_INLINE_EVENTS {
                        continue;
                    }
                    inlined += 1;
                    let sub = events[self.random(events.len())];
                    if let Some(e) = self.events.get(&sub) {
                        let at = t + e.delay;
                        run.items.extend(e.items.iter().map(|i| (at + i.start(), at + i.end(), i.clone())));
                    }
                }
            }
        }
        let Some(s) = self.states.get_mut(&id) else { return };
        if run.items.is_empty() && run.lines.is_empty() {
            s.state = 2;
        } else {
            s.run = Some(run);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct W {
        cut: bool,
        ok: bool,
    }
    impl SpeechWorld for W {
        fn condition(&mut self, _: &str) -> bool {
            self.ok
        }
        fn mode_minutes(&self) -> f32 {
            10.0
        }
        fn has_car(&self) -> bool {
            true
        }
        fn in_cutscene(&self) -> bool {
            self.cut
        }
    }

    fn ev(id: u32, priority: u8) -> SpeechEvent {
        let line = Line { wave: id, gain: 1.0, duration: 2.0 };
        SpeechEvent {
            id,
            priority,
            all_conditions: true,
            delay: 0.5,
            items: vec![SpeechItem::Lines {
                start: 0.0,
                end: 1.0,
                probability: 1.0,
                lines: vec![line],
                containers: vec![],
                vocabulary: 0,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn one_at_a_time_by_priority() {
        let mut m = SpeechManager::new([ev(1, 2), ev(2, 2), ev(3, 0), ev(4, 3)]);
        let w = &mut W { cut: false, ok: true };
        assert_eq!(m.request(1, false, [-1, -1, 0], w), PLAYED);
        assert_eq!(m.request(2, false, [-1, -1, 0], w), AS_IMPORTANT);
        assert_eq!(m.request(4, false, [-1, -1, 0], w), LESS_IMPORTANT);
        assert_eq!(m.request(9, false, [-1, -1, 0], w), UNKNOWN_EVENT);
        // Starts, waits its delay, says its line.
        assert!(m.update(0.1, w).is_empty());
        assert!(m.update(0.3, w).is_empty());
        let out = m.update(0.3, w);
        assert!(matches!(out.as_slice(), [SpeechOut::Say { event: 1, .. }]));
        // A more important one interrupts: the old one stops, the new one skips its delay.
        assert_eq!(m.request(3, false, [-1, -1, 0], w), PLAYED);
        let out = m.update(0.1, w);
        assert!(out.contains(&SpeechOut::Stop { event: 1 }));
        assert!(out.iter().any(|o| matches!(o, SpeechOut::Say { event: 3, .. })));
        assert_eq!(m.current(), Some(3));
        // Ends after its line.
        for _ in 0..30 {
            m.update(0.1, w);
        }
        assert_eq!(m.current(), None);
        assert_eq!(m.plays(3), 1);
    }

    #[test]
    fn self_playing_event_does_not_hang() {
        // An event whose event item plays itself with no delay: one frame must still return.
        let mut e = ev(1, 1);
        e.delay = 0.0;
        e.items.push(SpeechItem::Events { start: 0.0, end: 0.0, events: vec![1] });
        let mut m = SpeechManager::new([e]);
        let w = &mut W { cut: false, ok: true };
        assert_eq!(m.request(1, false, [-1, -1, 0], w), PLAYED);
        let out = m.update(0.1, w);
        assert!(out.iter().any(|o| matches!(o, SpeechOut::Say { event: 1, .. })));
        m.update(0.1, w);
    }

    #[test]
    fn conditions_period_and_cutscenes() {
        let mut e = ev(1, 1);
        e.period = 1.0;
        e.conditions = vec!["X".into()];
        e.all_conditions = true;
        let mut c = ev(2, 1);
        c.cutscene_behaviour = 2;
        let mut m = SpeechManager::new([e, c]);
        assert_eq!(m.request(1, false, [-1, -1, 0], &mut W { cut: false, ok: false }), CONDITIONS_FAILED);
        assert_eq!(m.request(2, false, [-1, -1, 0], &mut W { cut: true, ok: true }), NOT_ELIGIBLE);
        let w = &mut W { cut: false, ok: true };
        assert_eq!(m.request(1, false, [-1, -1, 0], w), PLAYED);
        for _ in 0..40 {
            m.update(0.1, w);
        }
        assert_eq!(m.current(), None);
        // Less than its period (1 min) since it started.
        assert_eq!(m.request(1, false, [-1, -1, 0], w), NOT_ELIGIBLE);
        for _ in 0..600 {
            m.update(0.1, w);
        }
        assert_eq!(m.request(1, false, [-1, -1, 0], w), PLAYED);
        assert!(m.stop());
        assert_eq!(m.current(), None);
    }
}
