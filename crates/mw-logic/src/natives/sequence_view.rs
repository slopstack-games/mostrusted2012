//! The `UISequences` and `GeneralSequence` binding roots: the UI sequence service (`UISequenceService`, root
//! "UISequences", `FUN_00829xxx` constructor near 0x82bxxx, type size 0x3258) and the general sequence service
//! (`GeneralSequenceService`, root "GeneralSequence", size 0x98, registered at 0x82fxxx). Both belong to the
//! UI manager's sequence code; the layout sequences (`mw_assets::sequence`) drive them while they play.
//!
//! **GeneralSequence.** Two bit sets: the service's own (+0x88, `IsTriggerSet[n]`, getter 0x82e950, set by
//! `FUN_00829d30` with name 0, cleared by `FUN_00829810`) and up to five named ones (`Triggers["NAME"]`, an object
//! lookup by name in a sorted table at +0xc / entries from +0x20, `FUN_0082fb00`; `Trigger.IsTriggerSet[n]`,
//! getter 0x82c650). Indices are 1-based: set / clear touch bit n - 1; the getter takes the 0-based position the
//! binding layer hands it. A name that is not in the table resolves to a null object (the read fails: `Null`).
//! `FUN_00829d30(name, n)`: name 0 sets the service's bit; a known name sets the bit; an unknown name is added
//! unless five are used already (then the set is dropped). Clearing the last bit of a named trigger removes its
//! entry (`FUN_0082ea50`).
//!
//! **Who sets triggers.** The `GeneralTriggerSequenceItem`s of the UI sequences (`mw_assets::sequence`
//! `ItemKind::GeneralTrigger { name, trigger }`: "BOOT" on the title, "TRANSITION" at 7.75 s of the event intro,
//! "MODS", "MWL"): the runtime calls [`UiSequences::set_trigger`] when the item fires and
//! [`UiSequences::clear_trigger`] when it ends; the HSM then reads them through `GeneralSequence.Triggers[...]`.
//! `LogicState::set_general_trigger` / `clear_general_trigger` are the entry points (the name is the binding's
//! `hash("BOOT")`; the item's `name` field is that hash of the trigger name).
//!
//! **UISequences.** `HideHudRequests` (+0x3254, the count of live HideHud items), `IsAnimationTriggerSet[n]`
//! (0x82bcb0: a bit set at +0x20), `IsActionHostUsed["NAME"]` (`FUN_0082fbe0`: byte +1 of the host's entry in the
//! table at +0x28, set by `FUN_00829b20`; an unknown name reads false), `UseDefaultHud` / `UseThermalHud` /
//! `UseNightHud` (0x82c690 / 0x82c720 / 0x82c7a0: the HUD mode is the first of the request counters at +8, +0x10,
//! +0x14, +0x18 that is nonzero (0, 2, 3, 4), else 5 when +0x1c is nonzero, else 1 = default; the getters compare
//! it with 1 / 2 / 3), and `Params[n].Text1..3` (16 `StringSequenceItemParam` of 0x320 bytes, valid byte at +0x4c;
//! an invalid one reads null). With no sequence playing every counter is 0: default HUD, nothing hidden.
//!
//! Model decisions: the named-trigger table is keyed by the binding's name hash (the original's getter looks up
//! `hash + 1`, its setter the value as given; the two agree through the callers' packing, which is not traced);
//! the HUD-mode counters are plain fields the runtime / sequence items fill.

use crate::binding::{Index, Lookup, Segment, Value};

use super::LogicState;

/// The most named triggers the service keeps (`FUN_00829d30`: a sixth is dropped).
pub const MAX_NAMED_TRIGGERS: usize = 5;

/// `StringSequenceItemParam` slots in `UISequenceService.Params`.
pub const PARAM_SLOTS: usize = 16;

/// The HUD modes the sequence service picks between (`FUN_0082c690`'s result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HudMode {
    /// A counter at +8 is nonzero: none of the three getters matches.
    Hidden,
    Default,
    Thermal,
    Night,
    /// +0x18 (4) or +0x1c (5): none of the three getters matches.
    Other,
}

/// The sequence services' state (`UISequenceService` and `GeneralSequenceService`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UiSequences {
    /// GeneralSequence +0x88: the service's own triggers (bit n - 1).
    pub general_bits: u64,
    /// GeneralSequence +0xc / +0x20: the named triggers (name hash, bits), in table order (sorted by name).
    pub named: Vec<(u32, u64)>,
    /// UISequences +0x3254: the HideHud requests.
    pub hide_hud_requests: i32,
    /// UISequences +0x20: the animation triggers (bit n).
    pub animation_bits: u64,
    /// UISequences +0x28 / +0x34: the action hosts by name hash and whether they were used.
    pub action_hosts: Vec<(u32, bool)>,
    /// UISequences +8, +0x10, +0x14, +0x18, +0x1c: the HUD-mode request counts.
    pub hud_requests: [u32; 5],
    /// UISequences +4 + 0x320 * n: the sequence string parameters (valid slots only).
    pub params: [Option<[String; 3]>; PARAM_SLOTS],
}

impl UiSequences {
    /// `FUN_00829d30(name, index)`: sets trigger `index` (1-based) of the named trigger (name 0 = the service's).
    pub fn set_trigger(&mut self, name: u32, index: u32) {
        let Some(bit) = index.checked_sub(1).filter(|b| *b < 64).map(|b| 1u64 << b) else { return };
        if name == 0 {
            self.general_bits |= bit;
        } else if let Some(e) = self.named.iter_mut().find(|e| e.0 == name) {
            e.1 |= bit;
        } else if self.named.len() < MAX_NAMED_TRIGGERS {
            self.named.push((name, bit));
            self.named.sort_by_key(|e| e.0);
        }
    }

    /// `FUN_00829810(name, index)`: clears the trigger; an emptied named trigger leaves the table (`FUN_0082ea50`).
    pub fn clear_trigger(&mut self, name: u32, index: u32) {
        let Some(bit) = index.checked_sub(1).filter(|b| *b < 64).map(|b| 1u64 << b) else { return };
        if name == 0 {
            self.general_bits &= !bit;
        } else if let Some(i) = self.named.iter().position(|e| e.0 == name) {
            self.named[i].1 &= !bit;
            if self.named[i].1 == 0 {
                self.named.remove(i);
            }
        }
    }

    /// `FUN_00829b20`: the action host was used.
    pub fn use_action_host(&mut self, name: u32) {
        match self.action_hosts.iter_mut().find(|e| e.0 == name) {
            Some(e) => e.1 = true,
            None => self.action_hosts.push((name, true)),
        }
    }

    /// 0x82c690: the HUD mode.
    pub fn hud_mode(&self) -> HudMode {
        let r = &self.hud_requests;
        if r[0] > 0 {
            HudMode::Hidden
        } else if r[1] > 0 {
            HudMode::Thermal
        } else if r[2] > 0 {
            HudMode::Night
        } else if r[3] > 0 || r[4] > 0 {
            HudMode::Other
        } else {
            HudMode::Default
        }
    }
}

impl LogicState {
    /// The runtime calls this when a `GeneralTriggerSequenceItem` fires (`name` = the item's name hash, `index`
    /// its 1-based trigger; name 0 = the service's own).
    pub fn set_general_trigger(&mut self, name: u32, index: u32) {
        self.sequences.set_trigger(name, index);
    }
    /// The runtime calls this when the item ends.
    pub fn clear_general_trigger(&mut self, name: u32, index: u32) {
        self.sequences.clear_trigger(name, index);
    }
}

fn bit_set(bits: u64, n: i32) -> Lookup {
    if !(0..64).contains(&n) {
        return Lookup::OutOfRange;
    }
    Lookup::Found(Value::Bool(bits & (1 << n) != 0))
}

/// `GeneralSequence.<member>[...]`.
pub fn general_lookup(s: &LogicState, members: &[Segment]) -> Lookup {
    let Some((first, rest)) = members.split_first() else { return Lookup::Found(Value::Object(true)) };
    let seq = &s.sequences;
    let is = |n: &str| first.hash == crate::binding::hash(n);
    if is("IsTriggerSet") {
        // Positions count from 1; the getter takes the 0-based bit.
        match first.index.pos() {
            Some(n) if n >= 1 => bit_set(seq.general_bits, n - 1),
            _ => Lookup::OutOfRange,
        }
    } else if is("Triggers") {
        let Index::Key { hash, .. } = &first.index else { return Lookup::Null };
        let Some(bits) = seq.named.iter().find(|e| e.0 == *hash).map(|e| e.1) else { return Lookup::Null };
        let Some((m, _)) = rest.split_first() else { return Lookup::Found(Value::Object(true)) };
        if m.hash != crate::binding::hash("IsTriggerSet") {
            return Lookup::Null;
        }
        match m.index.pos() {
            Some(n) if n >= 1 => bit_set(bits, n - 1),
            _ => Lookup::OutOfRange,
        }
    } else {
        Lookup::Null
    }
}

/// `UISequences.<member>[...]`.
pub fn ui_lookup(s: &LogicState, members: &[Segment]) -> Lookup {
    let Some((first, rest)) = members.split_first() else { return Lookup::Found(Value::Object(true)) };
    let seq = &s.sequences;
    let is = |n: &str| first.hash == crate::binding::hash(n);
    let b = |v: bool| Lookup::Found(Value::Bool(v));
    if is("HideHudRequests") {
        Lookup::Found(Value::Int(seq.hide_hud_requests))
    } else if is("IsAnimationTriggerSet") {
        match first.index.pos() {
            Some(n) if n >= 1 => bit_set(seq.animation_bits, n - 1),
            _ => Lookup::OutOfRange,
        }
    } else if is("IsActionHostUsed") {
        let Index::Key { hash, .. } = &first.index else { return Lookup::Null };
        b(seq.action_hosts.iter().any(|e| e.0 == *hash && e.1))
    } else if is("UseDefaultHud") {
        b(seq.hud_mode() == HudMode::Default)
    } else if is("UseThermalHud") {
        b(seq.hud_mode() == HudMode::Thermal)
    } else if is("UseNightHud") {
        b(seq.hud_mode() == HudMode::Night)
    } else if is("Params") {
        let Some(n) = first.index.pos() else { return Lookup::Null };
        if n < 1 || n as usize > PARAM_SLOTS {
            return Lookup::OutOfRange;
        }
        let Some(p) = &seq.params[n as usize - 1] else { return Lookup::Null };
        let Some((m, _)) = rest.split_first() else { return Lookup::Found(Value::Object(true)) };
        ["Text1", "Text2", "Text3"]
            .iter()
            .position(|t| m.hash == crate::binding::hash(t))
            .map_or(Lookup::Null, |i| Lookup::Found(Value::Str(p[i].clone())))
    } else {
        Lookup::Null
    }
}

#[cfg(test)]
mod tests {
    use super::super::LogicState;

    fn resolves(s: &LogicState, path: &str) -> bool {
        crate::binding::Binding::parse(path).unwrap().eval_bool(s).ok
    }
    use crate::binding::hash;

    #[test]
    fn general_triggers() {
        assert_eq!(hash("BOOT"), 1112493908);
        let mut s = LogicState::default();
        let boot = r#"GeneralSequence.Triggers["BOOT"].IsTriggerSet[1]"#;
        // An unknown name is a null object: the read fails.
        assert!(!resolves(&s, boot));
        s.set_general_trigger(hash("BOOT"), 1);
        assert_eq!(s.resolve_bool(boot), Some(true));
        assert_eq!(s.resolve_bool(r#"GeneralSequence.Triggers["BOOT"].IsTriggerSet[2]"#), Some(false));
        s.clear_general_trigger(hash("BOOT"), 1);
        assert!(!resolves(&s, boot), "the emptied trigger leaves the table");
        // The service's own bits.
        assert_eq!(s.resolve_bool("GeneralSequence.IsTriggerSet[7]"), Some(false));
        s.set_general_trigger(0, 7);
        assert_eq!(s.resolve_bool("GeneralSequence.IsTriggerSet[7]"), Some(true));
        // Five names at most.
        for n in ["A1", "A2", "A3", "A4", "A5", "A6"] {
            s.set_general_trigger(hash(n), 1);
        }
        assert_eq!(s.resolve_bool(r#"GeneralSequence.Triggers["A5"].IsTriggerSet[1]"#), Some(true));
        assert!(!resolves(&s, r#"GeneralSequence.Triggers["A6"].IsTriggerSet[1]"#));
    }

    #[test]
    fn ui_sequences() {
        let mut s = LogicState::default();
        assert_eq!(s.resolve_int("UISequences.HideHudRequests"), Some(0));
        assert_eq!(s.resolve_bool("UISequences.HideHudRequests(EQUALS:0)"), Some(true));
        assert_eq!(s.resolve_bool("UISequences.HideHudRequests(GTEQ:1)"), Some(false));
        assert_eq!(s.resolve_bool("UISequences.UseDefaultHud"), Some(true));
        assert_eq!(s.resolve_bool("UISequences.UseNightHud"), Some(false));
        s.sequences.hud_requests[2] = 1;
        assert_eq!(s.resolve_bool("UISequences.UseDefaultHud"), Some(false));
        assert_eq!(s.resolve_bool("UISequences.UseNightHud"), Some(true));
        s.sequences.hud_requests[1] = 1;
        assert_eq!(s.resolve_bool("UISequences.UseThermalHud"), Some(true));
        assert_eq!(s.resolve_bool("UISequences.UseNightHud"), Some(false));
        s.sequences.hide_hud_requests = 2;
        assert_eq!(s.resolve_bool("UISequences.HideHudRequests(GTEQ:1)"), Some(true));
        assert_eq!(s.resolve_bool(r#"UISequences.IsActionHostUsed["RETRY"](NOT)"#), Some(true));
        s.sequences.use_action_host(hash("RETRY"));
        assert_eq!(s.resolve_bool(r#"UISequences.IsActionHostUsed["RETRY"](NOT)"#), Some(false));
        assert_eq!(s.resolve_bool("UISequences.IsAnimationTriggerSet[3]"), Some(false));
        s.sequences.animation_bits = 1 << 2;
        assert_eq!(s.resolve_bool("UISequences.IsAnimationTriggerSet[3]"), Some(true));
        // Params: an invalid slot reads null.
        assert_eq!(s.resolve_string("UISequences.Params[1].Text1"), None);
        s.sequences.params[0] = Some(["a".into(), "".into(), "c".into()]);
        assert_eq!(s.resolve_string("UISequences.Params[1].Text1").as_deref(), Some("a"));
        assert_eq!(s.resolve_bool("UISequences.Params[1].Text2(IS_EMPTY)"), Some(true));
    }
}
