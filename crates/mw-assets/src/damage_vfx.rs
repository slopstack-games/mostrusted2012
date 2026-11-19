//! The car's damage sequences and their spot-effect items (docs/decomp.md "Damage sequences and their effects").
//!
//! Every car's `VehicleDamageBehaviour.DamageSequences` (+0x28, count +0x32; eight on every car) are started for
//! each race car by `FUN_005b6ce0` and run all the time. Each carries a `RaceCarEntityTimelineController`
//! (factory 0x0058f230, vtable 0x00fae1cc, registered with type id 0x1565d8 by `FUN_00590ce0`) whose time method
//! (`0x00828dd0`, vtable +4) replaces the sequence clock with a property of the race-car entity picked by its
//! `Property` (1..8 = the zone levels front +0xbd0, rear +0xbe0, left +0xbe8, right +0xbd8, front-left +0xbcc,
//! front-right +0xbd4, rear-left +0xbe4, rear-right +0xbdc; jump table at 0x00828f10): the level, capped at
//! [`LEVEL_CAP`] while the car is not wrecked, × [`AXIS_SCALE`] (`0x005a1b70` …). These are the same eight levels
//! the damage clips are posed from (`FUN_005b62f0`), so the axis is the zone's damage in percent.
//!
//! Items are active while `StartTime ≤ axis ≤ EndTime` (the item loop `FUN_00a42b90`; the item's look-ahead,
//! vtable +4, is the base 0). `VfxLocatorGroupVehicleSpotEffectSequenceItem` (factory 0x0086a7b0, vtable
//! 0x00ff30c4, 0xdc bytes):
//! - **start** (+8, `0x00862850`): binds `Intensity`; **play** (+0xc, `0x00862010`): its four locator slots
//!   (+0xb0, 8 bytes each: effect handle, state) are armed (state 0) only when the car's +0x1964 timer is 0 and
//!   its +0x1c2a flag set; otherwise state 2 and nothing plays until the item is entered again. The timer is set
//!   to 0.1 s (`.rdata` 0x00fad540) at the car's spawn (`FUN_00591350`) and counted down each frame
//!   (`FUN_00587120`): [`SPAWN_GATE`].
//! - **update** (+0x14, `0x00866390`): `Intensity` at the item's local time (`FUN_00862590`; with no curve and no
//!   binding: `Value` × the sequence's weight +0x40, which is 1, set by the player's init) and `FUN_00865f20`: per
//!   locator of `LocatorGroup` (`FUN_0057c4b0`, falling back to the first group) the transform car × locator ×
//!   (`LocalOffset`, `LocalRotation` in degrees; `FUN_008644e0`), then `FUN_00864ec0`: slot state 0 creates the
//!   effect (`FUN_0049f7c0`, the car's spot-effect parameter block, vtable +0x4c) unless one is still alive on the
//!   slot, puts the intensity into the record's velocity w (the effect's `Intensity`) and moves to state 1; state 1
//!   refreshes the live effect's parameters and intensity. An effect that ends while the item is active is not
//!   started again.
//! - **stop** (+0x24, `0x00862890`; exit +0x28 → +0x24): clears each live effect's emitting bit (record +0xe8
//!   bit 1) and invalidates the slot's state: the effect dies out on its own.
//!
//! `VfxSpotEffectSequenceItem` (factory 0x0086a6f0, vtable 0x00ff3054, 0xd0 bytes) is the same item without a
//! locator group: one effect (`873d9e9c`) at a transform from its binding (`97b735e6`, `FUN_00869010`), velocity
//! from the transform's movement (or a player car's, `FUN_00865310`); its stop (`0x00862950`) returns 0 while the
//! effect lives, so the item stays "exiting" until it has played out.

use mw_bundle::ResourceId;
use mw_formats::genesys::{Object, Value};
use mw_formats::{Locator, VehicleGraphicsSpec};

use crate::sequence::Modulated;
use crate::{AssetStore, Result};

/// The four effect slots of a locator-group item (+0xb0 .. +0xd0): the game indexes them by locator.
pub const LOCATOR_SLOTS: usize = 4;
/// A zone level is capped here while the car is not wrecked (`.rdata` 0x00f9cad4).
pub const LEVEL_CAP: f32 = 0.99;
/// Level → sequence axis (`.rdata` 0x00faba78).
pub const AXIS_SCALE: f32 = 100.0;
/// Items entered this soon after the car's spawn stay silent (vehicle +0x1964, `.rdata` 0x00fad540).
pub const SPAWN_GATE: f32 = 0.1;

mod h {
    pub const ID: u32 = 0x655d_3f5f;
    pub const START_TIME: u32 = 0x6c82_122b;
    pub const END_TIME: u32 = 0xddb9_c614;
    pub const LOCAL_OFFSET: u32 = 0xd1ae_e4c2;
    pub const LOCAL_ROTATION: u32 = 0x92ff_a255;
    pub const INTENSITY: u32 = 0xf672_6e73;
    /// Unnamed string (+0x88): the transform binding of the plain spot item; empty in every locator-group item.
    pub const TRANSFORM_BINDING: u32 = 0x97b7_35e6;
    /// The effect handle: +0x90 on the locator-group item (`50635a40`), the same place on the plain one
    /// (`873d9e9c`).
    pub const LOCATOR_EFFECT: u32 = 0x5063_5a40;
    pub const SPOT_EFFECT: u32 = 0x873d_9e9c;
    pub const LOCATOR_GROUP: u32 = 0xf73f_e1c1;
    pub const SEQUENCE_ITEMS: u32 = mw_formats::genesys::field_hash("SequenceItems");
    pub const TIMELINE_CONTROLLERS: u32 = mw_formats::genesys::field_hash("TimelineControllers");
    pub const PROPERTY: u32 = mw_formats::genesys::field_hash("Property");
    pub const DAMAGE_SEQUENCES: u32 = 0xb3de_577d;
    pub const DAMAGE_SEQUENCE: u32 = 0xc44c_a7f1;
}

/// A `VfxLocatorGroupVehicleSpotEffectSequenceItem` or `VfxSpotEffectSequenceItem`.
#[derive(Debug, Clone, PartialEq)]
pub struct SpotItem {
    pub unique_id: u32,
    pub start: f32,
    pub end: f32,
    /// `LocalOffset` (m) and `LocalRotation` (degrees) in the locator's frame. Zero in every item of the data.
    pub offset: [f32; 3],
    pub rotation: [f32; 3],
    pub intensity: Modulated,
    /// `97b735e6`: a transform binding (empty in every locator-group item).
    pub transform_binding: String,
    pub effect: Option<ResourceId>,
    /// The `VehicleGraphicsSpec` locator group (`None`: a plain `VfxSpotEffectSequenceItem`).
    pub locator_group: Option<u32>,
}

fn num(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f32),
        Value::UInt(u) => Some(*u as f32),
        _ => None,
    }
}

fn uint(o: &Object, hash: u32) -> u32 {
    match o.get(hash) {
        Some(Value::UInt(u)) => *u as u32,
        Some(Value::Int(i)) => *i as u32,
        _ => 0,
    }
}

fn handle(o: &Object, hash: u32) -> Option<ResourceId> {
    match o.get(hash)? {
        Value::Handle(h) => *h,
        Value::Ref(r) => Some(*r),
        _ => None,
    }
}

fn vec3(o: &Object, hash: u32) -> [f32; 3] {
    match o.get(hash) {
        Some(Value::Vector(v)) => [v[0], v[1], v[2]],
        _ => [0.0; 3],
    }
}

fn short(o: &Object) -> &str {
    let n = o.type_name.strip_prefix("Genesys.Gen.").unwrap_or(&o.type_name);
    n.strip_prefix("SequenceItems.").unwrap_or(n)
}

impl SpotItem {
    /// Read either spot-effect item (`None` for any other type).
    pub fn read(o: &Object) -> Option<Self> {
        let (effect, locator_group) = match short(o) {
            "VfxLocatorGroupVehicleSpotEffectSequenceItem" => {
                (handle(o, h::LOCATOR_EFFECT), Some(uint(o, h::LOCATOR_GROUP)))
            }
            "VfxSpotEffectSequenceItem" => (handle(o, h::SPOT_EFFECT), None),
            _ => return None,
        };
        Some(Self {
            unique_id: uint(o, h::ID),
            start: num(o.get(h::START_TIME)).unwrap_or(0.0),
            end: num(o.get(h::END_TIME)).unwrap_or(0.0),
            offset: vec3(o, h::LOCAL_OFFSET),
            rotation: vec3(o, h::LOCAL_ROTATION),
            intensity: match o.get(h::INTENSITY) {
                Some(Value::Struct(s)) => Modulated::read(s),
                _ => Modulated::constant(1.0),
            },
            transform_binding: match o.get(h::TRANSFORM_BINDING) {
                Some(Value::Str(s)) => s.clone(),
                _ => String::new(),
            },
            effect,
            locator_group,
        })
    }

    /// Active at sequence axis `t` (the item loop's range test, both ends included).
    pub fn active(&self, t: f32) -> bool {
        self.start <= t && t <= self.end
    }

    /// `Intensity` at `local` seconds into the item (`FUN_00862590`) for a sequence of weight `weight` (+0x40;
    /// 1 for the damage sequences). Without keys and binding: `Value × weight`. MODEL: with keys the curve is
    /// read through [`Modulated::at`] over the item's length (no item in the data has keys or a binding).
    pub fn intensity_at(&self, local: f32, weight: f32) -> f32 {
        if self.intensity.curve.is_empty() {
            return self.intensity.value * weight;
        }
        let length = self.end - self.start;
        self.intensity.at(if length == 0.0 { 0.0 } else { local / length })
    }
}

/// One damage sequence: the zone property its controller reads and its locator-group effect items.
#[derive(Debug, Clone)]
pub struct DamageSequence {
    pub id: ResourceId,
    /// `RaceCarEntityTimelineController.Property` (1..8 = zone clip 0..7; 0 = another entity value, not ported).
    pub property: u16,
    pub items: Vec<DamageVfxItem>,
}

/// A locator-group item with the locators of its group.
#[derive(Debug, Clone)]
pub struct DamageVfxItem {
    pub item: SpotItem,
    /// At most [`LOCATOR_SLOTS`].
    pub locators: Vec<Locator>,
}

impl DamageSequence {
    /// The zone clip whose level drives the sequence (`None` for property 0).
    pub fn zone(&self) -> Option<usize> {
        (1..=8).contains(&self.property).then(|| self.property as usize - 1)
    }
}

/// The sequence axis for a zone level (0..1): the controller's property getter (`0x005a1b70` …).
pub fn axis(level: f32, wrecked: bool) -> f32 {
    let level = if wrecked { level } else { level.min(LEVEL_CAP) };
    level * AXIS_SCALE
}

/// What an item's slots are doing (`FUN_00864ec0`'s state +0xb4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Slot {
    /// Stopped (the stop's −1).
    #[default]
    Idle,
    /// Armed by play: start an effect on the next update unless one lives on the slot.
    Armed,
    /// Started: refresh it while it lives.
    Playing,
    /// Play found the car not ready (the spawn gate): nothing until the item is entered again.
    Off,
}

/// An item's run state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ItemState {
    pub active: bool,
    pub slots: [Slot; LOCATOR_SLOTS],
}

/// What a tick asks of the effect side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Idle,
    /// The item is (still) active: run the update over its slots.
    Update,
    /// The item left its range: stop every slot's effect.
    Stop,
}

impl ItemState {
    /// One pass of the item loop at axis `t`; `ready` is the play gate (spawn timer run out, +0x1c2a set).
    pub fn tick(&mut self, item: &SpotItem, t: f32, ready: bool) -> Step {
        let now = item.active(t);
        match (self.active, now) {
            (false, true) => {
                self.active = true;
                self.slots = [if ready { Slot::Armed } else { Slot::Off }; LOCATOR_SLOTS];
                Step::Update
            }
            (true, true) => Step::Update,
            (true, false) => {
                self.active = false;
                self.slots = [Slot::Idle; LOCATOR_SLOTS];
                Step::Stop
            }
            (false, false) => Step::Idle,
        }
    }
}

impl AssetStore {
    /// The damage sequences of a `VehicleDamageBehaviour` (`DamageSequences`) with their locator-group effect
    /// items, the locators taken from `spec`. Sequences without such items are left out.
    pub fn damage_sequences(&self, behaviour: &Object, spec: &VehicleGraphicsSpec) -> Vec<DamageSequence> {
        let Some(Value::Array(list)) = behaviour.get(h::DAMAGE_SEQUENCES) else { return Vec::new() };
        let mut out = Vec::new();
        for v in list {
            let Value::Struct(s) = v else { continue };
            let Some(id) = handle(s, h::DAMAGE_SEQUENCE) else { continue };
            let Ok(seq) = self.genesys_object(id) else { continue };
            let property = match seq.get(h::TIMELINE_CONTROLLERS) {
                Some(Value::Array(a)) => a.iter().find_map(|c| match c {
                    Value::Struct(c) if short(c).ends_with("RaceCarEntityTimelineController") => {
                        Some(uint(c, h::PROPERTY) as u16)
                    }
                    _ => None,
                }),
                _ => None,
            };
            let Some(property) = property else { continue };
            let items: Vec<DamageVfxItem> = match seq.get(h::SEQUENCE_ITEMS) {
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(|v| match v {
                        Value::Struct(o) => SpotItem::read(o),
                        _ => None,
                    })
                    .filter_map(|item| {
                        // `FUN_0057c4b0` → `FUN_0042b870`: a linear search falling back to the first group.
                        let group = spec.locator_group(item.locator_group?).or(spec.locator_groups.first());
                        let mut locators = group.map(|g| g.locators.clone()).unwrap_or_default();
                        locators.truncate(LOCATOR_SLOTS);
                        Some(DamageVfxItem { item, locators })
                    })
                    .collect(),
                _ => Vec::new(),
            };
            if !items.is_empty() {
                out.push(DamageSequence { id, property, items });
            }
        }
        out
    }

    /// [`Self::damage_sequences`] of a car bundle (its `VehicleGraphicsSpec` and `VehicleDamageBehaviour`).
    pub fn vehicle_damage_sequences(&self, bundle: &mw_bundle::Bundle) -> Result<Vec<DamageSequence>> {
        use mw_bundle::ResourceType;
        let Some(entry) = bundle.entries_of_type(ResourceType::VehicleGraphicsSpec).next() else {
            return Ok(Vec::new());
        };
        let spec = VehicleGraphicsSpec::parse(&bundle.load(entry)?)?;
        for e in bundle.entries_of_type(ResourceType::GenesysObject) {
            let Ok(o) = self.decode_genesys(&bundle.load(e)?) else { continue };
            if o.type_name == "Genesys.Gen.VehicleDamageBehaviour" {
                return Ok(self.damage_sequences(&o, &spec));
            }
        }
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(start: f32, end: f32) -> SpotItem {
        SpotItem {
            unique_id: 1,
            start,
            end,
            offset: [0.0; 3],
            rotation: [0.0; 3],
            intensity: Modulated::constant(1.0),
            transform_binding: String::new(),
            effect: None,
            locator_group: Some(7),
        }
    }

    #[test]
    fn axis_caps_unless_wrecked() {
        assert_eq!(axis(0.5, false), 50.0);
        assert_eq!(axis(1.0, false), 99.0);
        assert_eq!(axis(1.0, true), 100.0);
    }

    #[test]
    fn items_enter_and_leave_their_range() {
        let it = item(40.0, 100.0);
        let mut s = ItemState::default();
        assert_eq!(s.tick(&it, 39.0, true), Step::Idle);
        assert_eq!(s.tick(&it, 40.0, true), Step::Update);
        assert_eq!(s.slots[0], Slot::Armed);
        assert_eq!(s.tick(&it, 60.0, true), Step::Update);
        assert_eq!(s.tick(&it, 0.0, true), Step::Stop);
        assert_eq!(s.slots[0], Slot::Idle);
        // Entered during the spawn gate: off until entered again.
        assert_eq!(s.tick(&it, 50.0, false), Step::Update);
        assert_eq!(s.slots[0], Slot::Off);
        // The full-damage items need a wreck: 99 never reaches [100, 100].
        let full = item(100.0, 100.0);
        let mut s = ItemState::default();
        assert_eq!(s.tick(&full, axis(1.0, false), true), Step::Idle);
        assert_eq!(s.tick(&full, axis(1.0, true), true), Step::Update);
    }

    #[test]
    fn damage_sequences_of_the_911() {
        let Ok(game) = mw_gamedir::GameDir::locate(None) else {
            eprintln!("no game install found; skipping");
            return;
        };
        let store = AssetStore::new(game).unwrap();
        let Some(b) = store.bundle("VEHICLES/VEH_1085007_HI.BNDL") else { return };
        let seqs = store.vehicle_damage_sequences(&b).unwrap();
        // Eight zones, properties 1..8 in order; the rear (the 911's engine) smokes from 40 %.
        assert_eq!(seqs.iter().map(|s| s.property).collect::<Vec<_>>(), (1..=8).collect::<Vec<u16>>());
        let rear = &seqs[1];
        assert!(rear.items.iter().any(|i| i.item.start == 40.0 && !i.locators.is_empty()));
    }
}
