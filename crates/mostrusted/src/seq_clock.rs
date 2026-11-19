//! What the sequence players hand the timeline controllers (`mw_assets::timeline`): their bindings and the local
//! user's input actions, and the log line per jump taken.

use bevy::prelude::*;
use mw_assets::sequence::Sequence;
use mw_assets::timeline::{Env, Jump, SKIP_CUTSCENE};

/// The controllers' view of the game for one tick.
pub struct GameEnv<'a> {
    /// An `EnabledBinding` holds.
    pub holds: &'a mut dyn FnMut(&str) -> bool,
    /// The menu actions (AUTOLOG_ACCEPT / ALT / START by id), when read.
    pub menu: Option<&'a crate::input::MenuInput>,
    /// SKIP_CUTSCENE's state (Esc / pad Back; not a menu action): 2 pressed this frame, 1 held, 0 idle.
    pub skip: u8,
}

impl Env for GameEnv<'_> {
    fn holds(&mut self, binding: &str) -> bool {
        (self.holds)(binding)
    }

    fn action(&mut self, action: u16) -> u8 {
        if action == SKIP_CUTSCENE {
            return self.skip;
        }
        u8::try_from(action).ok().and_then(|id| self.menu?.state_by_id(id)).unwrap_or(0)
    }
}

/// SKIP_CUTSCENE's state from the devices (2 pressed, 1 held, 0 idle).
pub fn skip_state(dev: &crate::input::Devices) -> u8 {
    let action = crate::input::defaults::SKIP_CUTSCENE;
    if dev.just_pressed(action) {
        2
    } else if dev.held(action) {
        1
    } else {
        0
    }
}

/// One log line per jump: who plays, the sequence, the clock (0 = its own, n = a conducted sequence's), the
/// controller and the times. A `JumpMonitorTimelineController` re-syncs to its conducted sequence nearly every
/// frame (it reads the child's time of the last tick): only moves of more than 0.1 s (a wait or a skip) are logged.
pub fn log_jumps(who: &str, seq: &Sequence, jumps: &[Jump]) {
    for j in jumps {
        if j.kind == "JumpMonitorTimelineController" && (j.to - j.from).abs() <= 0.1 {
            continue;
        }
        let conductor = seq.clocks.get(j.node).map_or(0, |n| n.item_id);
        let enabled = seq.clocks.get(j.node).and_then(|n| n.controllers.get(j.controller)).map_or("", |c| &c.enabled);
        info!(
            "{who}: sequence {} clock {} (conductor {conductor}) {} #{} {:.3} -> {:.3} s{}",
            seq.unique_id,
            j.node,
            j.kind,
            j.controller,
            j.from,
            j.to,
            if enabled.is_empty() { String::new() } else { format!(" ({enabled})") }
        );
    }
}
