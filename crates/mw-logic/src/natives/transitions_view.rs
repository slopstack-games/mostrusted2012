//! The `Transitions` binding root: the screen-transition manager of the widget-screen manager (G+0x2f458 +0x4b0,
//! the object `FUN_007f4760` updates every frame). It is not registered through `FUN_00a4d2a0` like the managers
//! before it: its class record (`.rdata` 0x00fbf9f4: name getter 0x6afda0, factory 0x7edbd0, name "Transitions")
//! is a screen-module class, and the factory 0x7edbd0 is its property getter, which compares the requested
//! property hash with hash globals the static initialisers fill (`FUN_009ed180` of "Transitioning", "Image1",
//! "Image2", "AllowInput" and one more for `Tip`; the initialisers are in the part of the code the text dump
//! does not cover, found in the memory snapshot at file offset 0xb67261 and on):
//!
//! | Member | Reads |
//! |---|---|
//! | `Transitioning` | `this+0x3ac == 3` |
//! | `Image1` / `Image2` | the u32 at `this+0x394` / `this+0x398` |
//! | `Tip[n]` | the u32 at `this+0x374 + (n & 7) * 4` (the argument's index, 0-based) |
//! | `AllowInput` | the byte at `this+0x3c5` |
//!
//! `this+0x3ac` is the transition state `FUN_007f4760` steps: 0 idle, 1 loading the transition's screen, 2 ready,
//! 3 **playing** (the transition's animation runs), 4 finishing (back to 0 when `FUN_007edca0` says the out
//! animation is done). `FUN_007f5850` turns 2 into 4. The same state is the HSM query `Query::TransitionState`
//! and its byte +0x3c4 `Query::TransitionFinished`.
//!
//! **Who sets the state.** Nothing in the logic does: the screen manager's frame update (`FUN_007f4760`) moves
//! it, started by the UI's transition requests (the page runner's `PlayWidgetGroupTransition`, the sequences'
//! screen-transition items). The runtime owns it: while its transition plays it calls
//! [`Transitions::set_playing`]`(true)` (state 3) and `set_playing(false)` (state 0) when it ends. With no engine
//! (`mwtool hsm`) the state stays 0, so `Transitions.Transitioning(NOT)` is true, which is what the original's
//! idle manager answers.

use crate::binding::{Lookup, Segment, Value, hash};

use super::LogicState;

/// The transition manager's fields the getters read.
#[derive(Debug, Clone, Default)]
pub struct Transitions {
    /// +0x3ac: 0 idle, 1 loading, 2 ready, 3 playing, 4 finishing.
    pub state: u32,
    /// +0x3c5.
    pub allow_input: bool,
    /// +0x394 / +0x398: the two loading-screen images (UniqueIds).
    pub image1: u32,
    pub image2: u32,
    /// +0x374..+0x394: the tip texts (language string ids), 8 slots.
    pub tips: [u32; 8],
}

/// The state value of a transition that is playing.
pub const PLAYING: u32 = 3;

impl Transitions {
    /// The runtime reports its screen transition: playing (state 3) or over (idle).
    pub fn set_playing(&mut self, playing: bool) {
        self.state = if playing { PLAYING } else { 0 };
    }

    pub fn transitioning(&self) -> bool {
        self.state == PLAYING
    }
}

/// `Transitions.<member>[...]`.
pub fn lookup(s: &LogicState, members: &[Segment]) -> Lookup {
    let Some((first, _)) = members.split_first() else { return Lookup::Found(Value::Object(true)) };
    let t = &s.transitions;
    let is = |n: &str| first.hash == hash(n);
    if is("Transitioning") {
        Lookup::Found(Value::Bool(t.transitioning()))
    } else if is("AllowInput") {
        Lookup::Found(Value::Bool(t.allow_input))
    } else if is("Image1") {
        Lookup::Found(Value::UInt(t.image1))
    } else if is("Image2") {
        Lookup::Found(Value::UInt(t.image2))
    } else if is("Tip") {
        // The getter masks the 0-based index with 7.
        let n = first.index.pos().unwrap_or(1).wrapping_sub(1) as u32;
        Lookup::Found(Value::UInt(t.tips[(n & 7) as usize]))
    } else {
        Lookup::Null
    }
}

#[cfg(test)]
mod tests {
    use super::super::LogicState;

    #[test]
    fn transitions_root() {
        let mut s = LogicState::default();
        assert_eq!(s.resolve_bool("Transitions.Transitioning"), Some(false));
        assert_eq!(s.resolve_bool("Transitions.Transitioning(NOT)"), Some(true));
        s.transitions.set_playing(true);
        assert_eq!(s.resolve_bool("Transitions.Transitioning"), Some(true));
        assert_eq!(s.resolve_bool("Transitions.Transitioning(NOT)"), Some(false));
        // The other states are not "transitioning".
        for st in [0, 1, 2, 4] {
            s.transitions.state = st;
            assert_eq!(s.resolve_bool("Transitions.Transitioning"), Some(false));
        }
        s.transitions.set_playing(false);
        assert_eq!(s.transitions.state, 0);
        s.transitions.image1 = 7;
        s.transitions.tips[2] = 99;
        s.transitions.allow_input = true;
        assert_eq!(s.resolve_int("Transitions.Image1"), Some(7));
        assert_eq!(s.resolve_int("Transitions.Tip[3]"), Some(99));
        assert_eq!(s.resolve_int("Transitions.Tip[11]"), Some(99));
        assert_eq!(s.resolve_bool("Transitions.AllowInput"), Some(true));
    }
}
