//! The UI manager's layer sets (UIM+0x31bf0 .. +0x31c1e; docs/decomp.md "UI layer sets"): the requested set
//! (+0x31c08, written by `SET_LAY_SET`, a feedback message's `HudMode` and the legacy screen loader), the loaded
//! layer-set list of the current UI group (+0x31bf0 / +0x31bf4, id +0x31c04) and the set that is entered
//! (+0x31bf8 / +0x31bfc), stepped by `FUN_007a2400` (state 5 of the group loader `FUN_007a48d0`).
//!
//! **The retail PC game never loads a list.** The group id comes from the legacy screen loader's JSON
//! (`FUN_007e4c90`: key hash `DAT_0117e56c` = 0x3905d433, the set from `DAT_0117e550` = 0x3b034b51) or from a
//! `UI\GROUPS\<id>.BNDL` bundle (`FUN_0079dbf0`); the install has neither (no such folder, none of its 399 JSON key
//! names hash to those keys, no GeneSys type with the list's layout), and two memory snapshots of the running game
//! read group 0, loader state 0, a null list and an empty layer table (+0x31b70). So on PC the set is a number that
//! is written, saved and restored, and nothing is shown or hidden by it: [`LayerSets::list`] stays `None` and
//! [`LayerSets::update`] does nothing. The rules are ported for the day a list exists (another platform's data).
//!
//! **A list** (record +0xc id, +0x14 / +0x18 its sets) holds layer-set definitions: +0x34 id, +0x38 an input focus
//! id handed to the manager's +0x30370 block, +0x40 / +0x46 its layers (0x1c-byte entries: a `UILayoutInstanceParams`
//! handle and a key at +0x18; a layer is found in the table +0x31030 by (key, the instance's id +0x54),
//! `FUN_007967e0`), +0x3c / +0x44 0x34-byte context entries (`FUN_00632a40` on enter, `FUN_00632af0` on leave: not
//! traced, not modelled) and +0x49 "leave after the layers' out transition".
//!
//! **The update** (`FUN_007a2400`), while the list is the requested group's (+0xc == +0x31c04) and the hold counter
//! (+0x31c18) is below 1:
//! 1. no set requested and the list is not empty: the first definition's id is requested;
//! 2. a set is entered and it is not the requested one: without +0x49 each layer is deactivated (`FUN_00791e60`
//!    (index, 0): its +0xd9 and its bit in the mask +0x2c cleared) and the set is left (`FUN_00796680`); with +0x49
//!    the out timer (+0x31c10, ticks) is raised to each layer's out time (its +0x18, seconds) and the layer is told to
//!    go out (vtable +0x20 (0, 0));
//! 3. a running out timer counts the frame down; below 3 ticks the entered set's layers are deactivated and the set
//!    is left;
//! 4. a set is requested and none is entered: each definition with that id is entered (`FUN_007a08d0`: every layer
//!    activated, shown (vtable +0x20 (.., 1)), given the depth `FUN_0061e6a0`(5) + its index); none found, the
//!    request is cleared; else the entered set's focus id (+0x38) is handed over.
//!
//! A list of another group is unloaded (`FUN_0079de00`: its layers destroyed, the set left).
//!
//! Model decisions: the out time of a layer is asked from the host each frame (the original re-reads the layer
//! object's +0x18 every frame; it is taken as the time its out transition still needs, else the timer would never
//! run down); the depth is the layer's index in its set (the base `FUN_0061e6a0`(5) is the host's).

/// Ticks per second of the out timer (`.rdata` 0xf9f604; 0xf9f608 = its inverse).
pub const TICKS_PER_SECOND: f64 = 3.0e6;

/// A layer of a set: the 0x1c-byte entry of the definition's list (+0x40).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayerRef {
    /// +0x18.
    pub key: u32,
    /// The `UILayoutInstanceParams`' UniqueId (+0x54).
    pub instance: u32,
}

/// A layer-set definition.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayerSetDef {
    /// +0x34.
    pub id: u32,
    /// +0x38 (0 = none).
    pub focus: u32,
    /// +0x49: the set is left only after its layers went out.
    pub wait_out: bool,
    pub layers: Vec<LayerRef>,
}

/// The layer-set list of a UI group.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayerSetList {
    /// +0xc: the group.
    pub group: u32,
    pub sets: Vec<LayerSetDef>,
}

/// What an update did, in order (the runtime's side of the calls).
#[derive(Debug, Clone, PartialEq)]
pub enum LayerOp {
    /// `FUN_00791e60`(index, 0): the layer is no longer part of what is drawn.
    Deactivate(LayerRef),
    /// vtable +0x20 (0, 0): the layer plays its out transition.
    GoOut(LayerRef),
    /// `FUN_007a08d0`: activated, shown, at depth base + `order`.
    Show { layer: LayerRef, order: u16 },
    /// `FUN_00796680`.
    Left { set: u32 },
    /// `FUN_007a08d0`'s end, with the set's focus id (+0x38).
    Entered { set: u32, focus: u32 },
    /// `FUN_0079de00`: the list's layers are destroyed.
    Unloaded { group: u32 },
}

/// The UI manager's layer-set state.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayerSets {
    /// +0x31c08: the requested set (0 = none).
    pub set: u32,
    /// +0x31c04: the requested UI group (0 on PC).
    pub group: u32,
    /// +0x31bf0: the loaded list (never on PC).
    pub list: Option<LayerSetList>,
    /// +0x31bf8: the entered set, an index into the list's sets.
    pub entered: Option<usize>,
    /// +0x31c10 (ticks).
    pub out_timer: i64,
    /// +0x31c18: frames the update is held for.
    pub hold: i32,
}

impl LayerSets {
    /// The entered set's id.
    pub fn entered_id(&self) -> Option<u32> {
        Some(self.list.as_ref()?.sets.get(self.entered?)?.id)
    }

    /// Whether a layer is part of the entered set (`None`: no list, so layer sets decide nothing).
    pub fn shows(&self, layer: LayerRef) -> Option<bool> {
        let list = self.list.as_ref()?;
        Some(self.entered.and_then(|e| list.sets.get(e)).is_some_and(|s| s.layers.contains(&layer)))
    }

    /// `FUN_007a2400` (and `FUN_0079de00` for a list of another group): one frame. `frame` is the frame's time in
    /// seconds, `out_time` a layer's remaining out time in seconds.
    pub fn update(&mut self, frame: f32, out_time: &dyn Fn(LayerRef) -> f32) -> Vec<LayerOp> {
        let mut ops = Vec::new();
        let Some(list) = self.list.as_ref() else { return ops };
        if self.group != list.group {
            if let Some(set) = self.entered.take().and_then(|e| list.sets.get(e)) {
                ops.push(LayerOp::Left { set: set.id });
            }
            ops.push(LayerOp::Unloaded { group: list.group });
            self.list = None;
            return ops;
        }
        if self.hold >= 1 {
            return ops;
        }
        if self.set == 0
            && let Some(first) = list.sets.first()
        {
            self.set = first.id;
        }
        if let Some(set) = self.entered.and_then(|e| list.sets.get(e))
            && set.id != self.set
        {
            for &layer in &set.layers {
                if set.wait_out {
                    let out = ticks(out_time(layer));
                    if out > self.out_timer {
                        self.out_timer = out;
                    }
                    ops.push(LayerOp::GoOut(layer));
                } else {
                    ops.push(LayerOp::Deactivate(layer));
                }
            }
            if !set.wait_out {
                ops.push(LayerOp::Left { set: set.id });
                self.entered = None;
            }
        }
        if self.out_timer > 0 {
            self.out_timer -= ticks(frame);
            if self.out_timer / 3 < 1
                && let Some(set) = self.entered.take().and_then(|e| list.sets.get(e))
            {
                ops.extend(set.layers.iter().map(|&l| LayerOp::Deactivate(l)));
                ops.push(LayerOp::Left { set: set.id });
            }
        }
        if self.set != 0 && self.entered.is_none() {
            for (i, set) in list.sets.iter().enumerate().filter(|(_, s)| s.id == self.set) {
                ops.extend(set.layers.iter().enumerate().map(|(n, &layer)| LayerOp::Show { layer, order: n as u16 }));
                self.entered = Some(i);
            }
            match self.entered.and_then(|e| list.sets.get(e)) {
                Some(set) => ops.push(LayerOp::Entered { set: set.id, focus: set.focus }),
                None => self.set = 0,
            }
        }
        ops
    }
}

fn ticks(seconds: f32) -> i64 {
    (f64::from(seconds) * TICKS_PER_SECOND) as i64
}

/// `SET_LAY_SET` and the other plain writers of +0x31c08.
pub fn apply(s: &mut super::LogicState, r: &super::Request) {
    if let super::Request::SetLayerSet { set } = r {
        s.layer_sets.set = *set;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(n: u32) -> LayerRef {
        LayerRef { key: 1, instance: n }
    }

    fn list() -> LayerSetList {
        LayerSetList {
            group: 9,
            sets: vec![
                LayerSetDef { id: 10, focus: 0, wait_out: false, layers: vec![layer(1), layer(2)] },
                LayerSetDef { id: 20, focus: 77, wait_out: true, layers: vec![layer(3)] },
            ],
        }
    }

    #[test]
    fn without_a_list_the_set_is_only_a_number() {
        let mut s = LayerSets { set: 5, ..Default::default() };
        assert!(s.update(0.016, &|_| 0.0).is_empty());
        assert_eq!((s.set, s.entered, s.shows(layer(1))), (5, None, None));
    }

    #[test]
    fn first_set_by_default_then_the_requested_one() {
        let mut s = LayerSets { group: 9, list: Some(list()), ..Default::default() };
        let ops = s.update(0.016, &|_| 0.0);
        assert_eq!(s.set, 10);
        assert_eq!(
            ops,
            [
                LayerOp::Show { layer: layer(1), order: 0 },
                LayerOp::Show { layer: layer(2), order: 1 },
                LayerOp::Entered { set: 10, focus: 0 }
            ]
        );
        assert_eq!((s.shows(layer(1)), s.shows(layer(3))), (Some(true), Some(false)));
        // Another set: the old one is left at once (no +0x49), the new one entered in the same frame.
        s.set = 20;
        let ops = s.update(0.016, &|_| 0.0);
        assert_eq!(ops[..3], [LayerOp::Deactivate(layer(1)), LayerOp::Deactivate(layer(2)), LayerOp::Left { set: 10 }]);
        assert_eq!(ops[3..], [LayerOp::Show { layer: layer(3), order: 0 }, LayerOp::Entered { set: 20, focus: 77 }]);
        assert_eq!(s.entered_id(), Some(20));
    }

    #[test]
    fn a_waiting_set_leaves_after_its_layers_went_out() {
        let mut s = LayerSets { group: 9, list: Some(list()), set: 20, ..Default::default() };
        s.update(0.016, &|_| 0.0);
        s.set = 10;
        // The layer still needs 0.25 s, then less each frame.
        let left = std::cell::Cell::new(0.25f32);
        let out = |_| left.get();
        let ops = s.update(0.1, &out);
        assert_eq!(ops, [LayerOp::GoOut(layer(3))]);
        assert_eq!(s.entered_id(), Some(20));
        left.set(0.15);
        assert_eq!(s.update(0.1, &out), [LayerOp::GoOut(layer(3))]);
        left.set(0.05);
        let ops = s.update(0.1, &out);
        assert_eq!(ops[..3], [LayerOp::GoOut(layer(3)), LayerOp::Deactivate(layer(3)), LayerOp::Left { set: 20 }]);
        assert_eq!(ops.last(), Some(&LayerOp::Entered { set: 10, focus: 0 }));
    }

    #[test]
    fn unknown_set_is_cleared_and_another_group_unloads() {
        let mut s = LayerSets { group: 9, list: Some(list()), set: 10, ..Default::default() };
        s.update(0.016, &|_| 0.0);
        s.set = 99;
        let ops = s.update(0.016, &|_| 0.0);
        assert_eq!(ops.last(), Some(&LayerOp::Left { set: 10 }));
        // Nothing matched: the request is cleared, and the next frame falls back to the first set.
        assert_eq!((s.set, s.entered), (0, None));
        s.update(0.016, &|_| 0.0);
        assert_eq!(s.entered_id(), Some(10));
        s.group = 0;
        assert_eq!(s.update(0.016, &|_| 0.0), [LayerOp::Left { set: 10 }, LayerOp::Unloaded { group: 9 }]);
        assert!(s.list.is_none());
    }

    #[test]
    fn set_lay_set_through_the_logic_host() {
        use crate::hsm::Host;
        use crate::hsm::behaviours::Query;
        let mut host = crate::natives::LogicHost::default();
        assert_eq!(host.query(&Query::LayerSet), Some(0));
        host.request(crate::natives::Request::SetLayerSet { set: 12 });
        assert_eq!(host.query(&Query::LayerSet), Some(12));
        assert_eq!(host.state.borrow().layer_sets.set, 12);
    }

    #[test]
    fn hold_counter_blocks() {
        let mut s = LayerSets { group: 9, list: Some(list()), hold: 1, ..Default::default() };
        assert!(s.update(0.016, &|_| 0.0).is_empty());
        assert_eq!(s.set, 0);
    }
}
