//! The sequences' timeline controllers, ported from the code (docs/decomp.md "Timeline controllers"): the clock
//! of a `Sequence` and the `TimelineControllers` that move it (jumps, loops, waits for a button or a binding,
//! quits), and the clocks of its conducted sequences (`ConductorSequenceItem`), each with its own controllers.
//!
//! **The controller loop** (`FUN_00a42140`, called each tick by the sequence update `FUN_00a45220` before the item
//! loop `FUN_00a42b90`): the sequence's clock object (`FUN_00a44610`: `59e12b27` 0 = the time clock, vtable
//! 0x0102b83c) sits first in the controller list, ahead of the data's controllers. Each one whose condition holds
//! (vtable +0xc, `FUN_00a434b0`) maps the time to a new one (+0x4); the clock (`FUN_00a41fb0`) advances it by the
//! tick when nobody moved it since its last call. When a controller's result differs from the clock's time, the
//! clock takes it and the loop starts again from the first data controller. The sequence's time and its delta are
//! stored after the loop (the delta is 0 on a tick with a jump).
//!
//! **The condition** (`FUN_00a434b0`): no `EnabledBinding` = always; `TriggerType` 1 = while the binding holds;
//! `TriggerType` 0 = on a tick where the binding's value differs from the last one seen (initially 0).
//!
//! **The controllers** (factories registered at 0x00a451bd–0x00a45211; `.data` 0x010d10fc / 0x010c67e4 = −FLT_MAX is
//! "unset"; "continuous" = this call's time is the last one it returned plus the tick):
//! - `JumpTimelineController` (vtable 0x0102b85c, init `FUN_00a44320`, map `FUN_00a43870`): jumps to
//!   `b6a51260` when the time equals `TriggerTime` or crosses it continuously (last ≤ trigger < t). With
//!   `2d0e4348` ("monitor") it also jumps whenever the time is between the two: forward jumps in
//!   [trigger, destination), backward ones in [destination, trigger) (hold at the destination while the binding
//!   holds).
//! - `JumpNTimesTimelineController` (0x0102b77c, `FUN_00a442d0`, `FUN_00a437c0`): the crossing jump at most
//!   `f2139690` times; the count restarts once the time is past the trigger without a jump.
//! - `JumpOnButtonPress` (0x0102b79c, `FUN_00827240`, `FUN_00827280`): while the time is in [trigger, destination)
//!   and the button is pressed, jump. `Action` picks the input action: 0 AUTOLOG_ACCEPT, 1 SKIP_CUTSCENE,
//!   2 AUTOLOG_ALT, 3 AUTOLOG_START; with `1aeaf3ad` (and always for 1) only a fresh press (state 2) counts, else
//!   held or pressed (1 or 2).
//! - `JumpWaitForButtonPress` (0x00ff10a4, `FUN_00827710`, `FUN_00827760`): a loop that waits for AUTOLOG_ACCEPT:
//!   crossing the trigger jumps (back) to the destination unless the button was pressed since; the press is
//!   forgotten once the time is past the trigger or before the destination.
//! - `JumpMonitorTimelineController` (0x0102b7bc, `FUN_00826ef0`, `FUN_00827010`; its +0x18 method
//!   `FUN_00826f50` picks the monitored item: the last `ConductorSequenceItem` with start ≤ trigger and
//!   end ≥ destination): while the time (or the last one, continuously) is in [trigger, destination) and in the
//!   conductor's range, and its conducted sequence runs, the time follows the conductor: item start + the
//!   conducted sequence's time, when they differ by more than 1 ms (`.data` 0x010c67ec). The outros wait on
//!   their conducted sequence's loops this way.
//! - `QuitSequenceTimelineController` (0x0102b7dc, `FUN_00a441d0`, `FUN_00a439c0`): while the time is in
//!   [min, max) of trigger and destination, the time becomes +FLT_MAX (`.data` 0x010d1100): the sequence ends.
//!
//! Conducted sequences (`ConductorSequenceItem`, update `FUN_00855890`): the conductor ticks its child's own
//! sequence update (`FUN_00a45220`) every tick while it runs, so the child runs on its own clock, started at 0
//! (`FUN_00854500`(0)) when the conductor item starts.
//!
//! Model decisions: a conducted sequence starts the tick its conductor's range is entered, and its clock moves from
//! the next tick; the controller loop gives up after 64 restarts (the original would spin).

use crate::sequence::{ClockNode, Controller, ControllerKind};

/// "Unset" (`.data` 0x010d10fc, −FLT_MAX).
pub const UNSET: f32 = -f32::MAX;
/// The time a quit moves the clock to (`.data` 0x010d1100, +FLT_MAX).
pub const QUIT: f32 = f32::MAX;
/// Input action ids the controllers read (docs/decomp.md "Input and control bindings").
pub const SKIP_CUTSCENE: u16 = 34;
pub const AUTOLOG_ACCEPT: u16 = 52;
pub const AUTOLOG_ALT: u16 = 55;
pub const AUTOLOG_START: u16 = 61;

/// What the controllers read from the game.
pub trait Env {
    /// An `EnabledBinding` holds (`TriggerType` 1).
    fn holds(&mut self, binding: &str) -> bool;
    /// An `EnabledBinding`'s value (`TriggerType` 0; `None` = unresolved, never a change).
    fn value(&mut self, binding: &str) -> Option<f64> {
        Some(if self.holds(binding) { 1.0 } else { 0.0 })
    }
    /// The local user's input action state: 0 idle, 1 held, 2 pressed, 3 released (`.rdata` 0x00f9fab0).
    fn action(&mut self, action: u16) -> u8;
}

/// A jump taken (for logs): which clock ([`ClockNode`] index), which controller, from and to.
#[derive(Debug, Clone, PartialEq)]
pub struct Jump {
    pub node: usize,
    pub controller: usize,
    pub kind: &'static str,
    pub from: f32,
    pub to: f32,
}

#[derive(Debug, Clone, Default)]
struct CtlState {
    /// The time it last returned unchanged (+0x14 / +0x1c by type).
    last: f32,
    /// `JumpTimelineController` +0x18: destination > trigger.
    forward: bool,
    /// `JumpNTimesTimelineController` +0x20: jumps taken.
    count: i32,
    /// `JumpWaitForButtonPress` +0x20: pressed since.
    latch: bool,
    /// The condition's last value (`TriggerType` 0, +0x8).
    value: i64,
}

/// One clock with its controllers' state (a sequence instance's +0x0 time, +0x4 delta, the clock object and the
/// controller list).
#[derive(Debug, Clone)]
pub struct TimelineState {
    pub time: f32,
    /// The last tick's time change (0 on a tick with a jump).
    pub delta: f32,
    /// The clock object's time (+0x14 of 0x0102b83c).
    clock: f32,
    ctl: Vec<CtlState>,
    /// A quit ended it.
    pub quit: bool,
}

impl TimelineState {
    /// A clock at `start` with fresh controllers (`FUN_00a44610`: the clock object gets the start time).
    pub fn new(controllers: &[Controller], start: f32) -> Self {
        let ctl = controllers
            .iter()
            .map(|c| match c.kind {
                // `FUN_00a44320`: last unset, direction from the data.
                ControllerKind::Jump => {
                    CtlState { last: UNSET, forward: c.destination > c.trigger_time, ..Default::default() }
                }
                // `FUN_00a442d0`, `FUN_00827710`: last 0.
                ControllerKind::JumpNTimes(_) | ControllerKind::JumpWaitForButtonPress => CtlState::default(),
                // `FUN_00826ef0`: last unset (0x010c67e4).
                _ => CtlState { last: UNSET, ..Default::default() },
            })
            .collect();
        Self { time: start, delta: 0.0, clock: start, ctl, quit: false }
    }

    /// One tick of the controller loop (`FUN_00a42140`). `monitored(k)` answers a `JumpMonitorTimelineController`
    /// (controller index `k`): its conductor's (start, end) and the conducted sequence's time, when it runs.
    /// Returns the jumps taken.
    pub fn tick(
        &mut self,
        controllers: &[Controller],
        dt: f32,
        env: &mut dyn Env,
        monitored: &mut dyn FnMut(usize) -> Option<(f32, f32, f32)>,
        node: usize,
    ) -> Vec<Jump> {
        let mut jumps = Vec::new();
        if self.quit {
            return jumps;
        }
        let mut t = self.time;
        let mut cached = UNSET;
        let mut jumped = false;
        let mut k = 0usize;
        let mut restarts = 0;
        while k <= controllers.len() {
            let from = t;
            if k == 0 {
                // The time clock (`FUN_00a41fb0`): advance unless something moved it.
                if t == self.clock {
                    t += dt;
                }
                self.clock = t;
            } else {
                let c = &controllers[k - 1];
                if !enabled(c, &mut self.ctl[k - 1], env) {
                    k += 1;
                    continue;
                }
                t = map(c, &mut self.ctl[k - 1], t, dt, env, &mut || monitored(k - 1));
            }
            if cached == UNSET {
                cached = self.clock;
            }
            if cached != t {
                if k > 0 {
                    jumps.push(Jump { node, controller: k - 1, kind: controllers[k - 1].kind.name(), from, to: t });
                }
                jumped = true;
                self.clock = t;
                cached = t;
                restarts += 1;
                if restarts > 64 {
                    break;
                }
                k = 1;
                continue;
            }
            k += 1;
        }
        self.delta = if jumped { 0.0 } else { t - self.time };
        self.time = t;
        if t == QUIT {
            self.quit = true;
        }
        jumps
    }
}

impl TimelineState {
    /// A clock resumed at `clock` by a player that keeps only the time: every controller's last time is the clock
    /// (what it is after a continuous tick). `JumpNTimesTimelineController`'s count, `JumpWaitForButtonPress`'s
    /// press and a `TriggerType` 0 binding's last value start afresh each time.
    pub fn resume(controllers: &[Controller], clock: f32) -> Self {
        let mut s = Self::new(controllers, clock);
        for c in &mut s.ctl {
            c.last = clock;
        }
        s
    }
}

/// One tick for a player that keeps only the time ([`TimelineState::resume`]): the new time and the jumps.
pub fn step(controllers: &[Controller], clock: f32, dt: f32, env: &mut dyn Env) -> (f32, Vec<Jump>) {
    let mut s = TimelineState::resume(controllers, clock);
    let jumps = s.tick(controllers, dt, env, &mut |_| None, 0);
    (s.time, jumps)
}

/// `FUN_00a434b0`.
fn enabled(c: &Controller, st: &mut CtlState, env: &mut dyn Env) -> bool {
    if c.enabled.is_empty() {
        return true;
    }
    if c.trigger_type == 1 {
        return env.holds(&c.enabled);
    }
    match env.value(&c.enabled) {
        Some(v) if v as i64 != st.value => {
            st.value = v as i64;
            true
        }
        _ => false,
    }
}

/// The controller's time map (vtable +0x4).
fn map(
    c: &Controller,
    st: &mut CtlState,
    t: f32,
    dt: f32,
    env: &mut dyn Env,
    monitored: &mut dyn FnMut() -> Option<(f32, f32, f32)>,
) -> f32 {
    let (trig, dest) = (c.trigger_time, c.destination);
    let continuous = t == st.last + dt;
    match c.kind {
        // `FUN_00a43870`.
        ControllerKind::Jump => {
            let discontinuous = st.last != UNSET && !continuous;
            let crossed = !discontinuous && st.last != UNSET && trig >= st.last && t != UNSET && t > trig;
            if t == trig || crossed {
                return dest;
            }
            if c.monitor {
                let inside = if st.forward {
                    trig != UNSET && t >= trig && dest != UNSET && dest > t
                } else {
                    dest != UNSET && t >= dest && trig != UNSET && trig > t
                };
                if inside {
                    return dest;
                }
            }
            st.last = t;
            t
        }
        // `FUN_00a437c0`.
        ControllerKind::JumpNTimes(n) => {
            if st.count < n {
                if continuous {
                    if !(st.last == UNSET || trig < st.last || t == UNSET) {
                        if t <= trig {
                            st.last = t;
                            return t;
                        }
                        st.count += 1;
                        return dest;
                    }
                } else if t == trig {
                    st.count += 1;
                    return dest;
                }
            }
            if t > trig {
                st.count = 0;
            }
            st.last = t;
            t
        }
        // `FUN_00827280`.
        ControllerKind::JumpOnButtonPress { action, pressed_only } => {
            let (id, fresh) = match action {
                0 => (AUTOLOG_ACCEPT, pressed_only),
                1 => (SKIP_CUTSCENE, true),
                2 => (AUTOLOG_ALT, pressed_only),
                3 => (AUTOLOG_START, pressed_only),
                _ => return t,
            };
            let s = env.action(id);
            let pressed = if fresh { s == 2 } else { s == 1 || s == 2 };
            if trig != UNSET && t >= trig && dest != UNSET && dest > t && pressed { dest } else { t }
        }
        // `FUN_00827760`.
        ControllerKind::JumpWaitForButtonPress => {
            if env.action(AUTOLOG_ACCEPT) == 2 {
                st.latch = true;
            }
            let at_trigger =
                if continuous { !(st.last == UNSET || trig < st.last || t == UNSET || t <= trig) } else { t == trig };
            if at_trigger && !st.latch {
                return dest;
            }
            st.last = t;
            if t > trig || dest > t {
                st.latch = false;
            }
            t
        }
        // `FUN_00827010`.
        ControllerKind::JumpMonitor => {
            let discontinuous = st.last != UNSET && !continuous;
            let last = st.last;
            st.last = t;
            let in_window = (trig != UNSET && t >= trig && dest != UNSET && dest > t)
                || (!discontinuous && trig != UNSET && last >= trig && dest != UNSET && dest > last);
            if !in_window {
                return t;
            }
            let Some((start, end, child)) = monitored() else { return t };
            let in_item = (start != UNSET && t >= start && end != UNSET && end > t)
                || (!discontinuous && start != UNSET && last >= start && end != UNSET && end > last);
            if !in_item {
                return t;
            }
            let target = start + child;
            if (t - target).abs() > 0.001 {
                st.last = last;
                return target;
            }
            t
        }
        // `FUN_00a439c0`.
        ControllerKind::Quit => {
            let (lo, hi) = if trig > dest { (dest, trig) } else { (trig, dest) };
            if lo != UNSET && t >= lo && hi != UNSET && hi > t { QUIT } else { t }
        }
        ControllerKind::Other(_) => t,
    }
}

/// A sequence's clocks: its own and its conducted sequences' ([`ClockNode`]s, pre-order, 0 = the sequence).
#[derive(Debug, Clone)]
pub struct SequenceClock {
    /// Per node: its clock while it runs (`None`: its conductor is not running).
    pub nodes: Vec<Option<TimelineState>>,
}

impl SequenceClock {
    pub fn new(nodes: &[ClockNode], start: f32) -> Self {
        let mut s = Self { nodes: vec![None; nodes.len()] };
        if let Some(root) = nodes.first() {
            s.nodes[0] = Some(TimelineState::new(&root.controllers, start));
        }
        s.update_children(nodes);
        s
    }

    /// The sequence's own time.
    pub fn time(&self) -> f32 {
        self.nodes.first().and_then(|n| n.as_ref()).map_or(0.0, |n| n.time)
    }

    /// Ended by a quit.
    pub fn quit(&self) -> bool {
        self.nodes.first().and_then(|n| n.as_ref()).is_some_and(|n| n.quit)
    }

    /// A node's own (local) time, while it runs.
    pub fn local(&self, node: usize) -> Option<f32> {
        self.nodes.get(node)?.as_ref().map(|n| n.time)
    }

    /// A node's time in the flattened sequence's frame (its offset + its local time), while it runs: the time
    /// a flattened item of that node is played at.
    pub fn frame_time(&self, nodes: &[ClockNode], node: usize) -> Option<f32> {
        Some(nodes.get(node)?.offset + self.local(node)?)
    }

    /// Move the sequence's own clock (a skip or seek by the owner), as an outside jump.
    pub fn seek(&mut self, nodes: &[ClockNode], t: f32) {
        if let Some(Some(root)) = self.nodes.first_mut() {
            root.time = t;
            root.clock = t;
            root.delta = 0.0;
        }
        self.update_children(nodes);
    }

    /// One tick: each running clock's controllers (parents before their children, as the conductor items tick
    /// their sequences in the parent's item loop), then the conducted sequences that start or stop.
    pub fn tick(&mut self, nodes: &[ClockNode], dt: f32, env: &mut dyn Env) -> Vec<Jump> {
        let mut jumps = Vec::new();
        for i in 0..nodes.len() {
            let Some(mut state) = self.nodes[i].take() else { continue };
            let children = &self.nodes;
            let mut monitored = |k: usize| {
                let child = nodes[i].monitored.get(k).copied().flatten()?;
                let t = children[child].as_ref()?.time;
                Some((nodes[child].start, nodes[child].end, t))
            };
            jumps.extend(state.tick(&nodes[i].controllers, dt, env, &mut monitored, i));
            self.nodes[i] = Some(state);
            // Its children (pre-order: they follow it) start or stop with the conductor's range.
            self.update_children_of(nodes, i);
        }
        jumps
    }

    fn update_children(&mut self, nodes: &[ClockNode]) {
        for i in 0..nodes.len() {
            self.update_children_of(nodes, i);
        }
    }

    fn update_children_of(&mut self, nodes: &[ClockNode], parent: usize) {
        let pt = self.nodes[parent].as_ref().filter(|p| !p.quit).map(|p| p.time);
        for (c, n) in nodes.iter().enumerate().skip(parent + 1) {
            if n.parent != Some(parent) {
                continue;
            }
            let runs = pt.is_some_and(|t| t >= n.start && t < n.end);
            match (&self.nodes[c], runs) {
                (None, true) => self.nodes[c] = Some(TimelineState::new(&n.controllers, 0.0)),
                (Some(_), false) => self.clear(nodes, c),
                _ => {}
            }
        }
    }

    fn clear(&mut self, nodes: &[ClockNode], node: usize) {
        self.nodes[node] = None;
        for (c, n) in nodes.iter().enumerate().skip(node + 1) {
            if n.parent == Some(node) {
                self.clear(nodes, c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoInput(bool);
    impl Env for NoInput {
        fn holds(&mut self, _: &str) -> bool {
            self.0
        }
        fn action(&mut self, _: u16) -> u8 {
            0
        }
    }

    fn ctl(kind: ControllerKind, trig: f32, dest: f32, enabled: &str, monitor: bool) -> Controller {
        Controller { kind, trigger_type: 1, monitor, trigger_time: trig, destination: dest, enabled: enabled.into() }
    }

    fn run(nodes: &[ClockNode], env: &mut dyn Env, ticks: usize, dt: f32) -> (SequenceClock, Vec<Jump>) {
        let mut c = SequenceClock::new(nodes, 0.0);
        let mut all = Vec::new();
        for _ in 0..ticks {
            all.extend(c.tick(nodes, dt, env));
        }
        (c, all)
    }

    #[test]
    fn loop_back_and_quit() {
        // A loop 5 → 1 (the UI screens' idle loops), then a quit window.
        let root = ClockNode::root(vec![ctl(ControllerKind::Jump, 5.0, 1.0, "", false)], 10.0);
        let (c, jumps) = run(&[root], &mut NoInput(true), 60, 0.25);
        assert_eq!(jumps.len(), 3, "{jumps:?}");
        assert!(c.time() < 5.0);
        let root = ClockNode::root(vec![ctl(ControllerKind::Quit, 0.75, 0.98, "", false)], 10.0);
        let (c, _) = run(&[root], &mut NoInput(true), 4, 0.25);
        assert!(c.quit());
    }

    #[test]
    fn monitor_holds_on_the_conductor() {
        // The race outro's shape: the parent follows its conductor from 10 s while the child loops 10.6 ↔ 11.4.
        let root = ClockNode {
            monitored: vec![Some(1)],
            ..ClockNode::root(vec![ctl(ControllerKind::JumpMonitor, 10.0, 15.5, "", false)], 15.5)
        };
        let child = ClockNode {
            parent: Some(0),
            start: 0.0,
            end: 15.5,
            offset: 0.0,
            item_id: 0,
            controllers: vec![ctl(ControllerKind::Jump, 11.4, 10.6, "Widget.AwardMod.IsActive", false)],
            monitored: vec![None],
            length: 15.0,
        };
        let nodes = [root, child];
        let (c, jumps) = run(&nodes, &mut NoInput(true), 600, 1.0 / 30.0);
        assert!(c.time() < 11.5 && c.time() >= 10.0, "{}", c.time());
        assert!(jumps.iter().any(|j| j.node == 1));
        // Released: both play on to the end.
        let (c, _) = run(&nodes, &mut NoInput(false), 600, 1.0 / 30.0);
        assert!(c.time() > 15.0);
    }
}
