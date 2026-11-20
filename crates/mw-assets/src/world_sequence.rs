//! Sequences that world objects play (docs/decomp.md "World-object sequences"): the `WCSequenceBehaviour`s of
//! track-unit objects (flashing road-work lights, the harbour crane, a hydrant's spray sound) and of the spike
//! strip's world object (`GAMELOGIC/WEAPONS`), and the items that act on the object that plays them.
//!
//! - **`WCSequenceBehaviour`** (type 0x3f412; handler vtable 0x00faa9cc, runtime built by `FUN_005b0610`, vtable
//!   0x00faf3f4): `Label` +0x0c, the behaviour base bools +0x18 `0228a2f8` (contacts switch it on), +0x19
//!   `713589ad` (world contacts switch it off), +0x1a `543527d0` (on from the start), +0x1c `Sequence` (`04bd97eb`),
//!   +0x24 `0ad467f1` (animated: the runtime carries a 60-bone palette, identity at creation, and tells the object
//!   it is animated through its vtable +0x20). Switching it on (slot 1, `FUN_005b7190`) starts the sequence with
//!   a context naming the object ("EntitySequenceItemParam", `FUN_00861820`: the object and the palette) and its
//!   id (vtable 0x00faf3c8); state +0x6c = 2. Its update (slot 6, `0x005a79f0`) ticks the sequence
//!   (`FUN_00a45220`) while state is 2; switching it off (slot 2, `FUN_005a79c0`) stops the sequence (state 3).
//!   A pair contact (slot 8, `0x005a79e0`) asks for "on" when +0x18 is set; a world contact (slot 9,
//!   `0x005b8810`) asks for "on" by +0x18 and "off" by +0x19.
//! - **`CollisionSequenceBehaviour`** (0x7357e; handler 0x00faaaa8, runtime `FUN_005a9170`, vtable 0x00faed94):
//!   three sequences, +0x1c `26b1191f`, +0x24 `cbe0bd7b`, +0x2c `Vehicle_Hit`. Pair contact `FUN_005b8620`: while
//!   the object's upward speed (its velocity against −(0, 1, 0)… `.rdata` 0x00fae850) stays below 6 (`.bss`
//!   0x0110d580, snap2), a contact whose value reaches 5 (`.bss` 0x0110e010) once the behaviour's timer (+0x20)
//!   is at least 0.6 s (`.bss` 0x0110df60) and whose other body is a vehicle (`FUN_00573360`) switches it on and
//!   plays `Vehicle_Hit` (`FUN_005b5480`); otherwise `FUN_005b7350` (the other two; not read).
//! - **`WCShowBehaviour`** (0x2ada): shows the object (the spike strip's "Show"); no other field.
//!
//! The items ([`EntityItem`]), each through the item loop `FUN_00a42b90` (vtable slots: decomp.md "Sequence
//! spawn items"); the owner object comes from the sequence's context:
//! - **`AddBehaviourSequenceItem`** (factory 0x00545e20, vtable 0x00faabc4): enter `0x00533880` adds the
//!   `Behaviour` (+0x24 handle) to the owner (`FUN_005be520` → `FUN_005be3b0`: the behaviour type's handler builds
//!   its runtime) and switches it on (`FUN_005be090` → the owner's manager +4); stop `FUN_00532150` switches it off
//!   (`FUN_005be0b0`, manager +8) and removes it (`FUN_005be0f0`, manager +0x10).
//! - **`BehaviourEnabledSequenceItem`** (0x00545dc0, vtable 0x00faab90): init `0x00539660` hashes `Label` (+0x24,
//!   `FUN_00a1d5c0`) into +0x1c; enter `0x00532180` switches the owner's behaviours of that label on, stop
//!   `0x00532190` off.
//! - **`BehaviourEnabledSequenceItemGameplay`** (0x75755 → 0x00861180, vtable 0x00ff1fec): the same label hash
//!   (init `0x00850fb0`; entity id +0x18 from the context at bind, `0x00850720`); enter `FUN_00854150` posts
//!   entity message 4 (switch on) with the id and the hash (`FUN_0084c5a0`); stop `0x00854190` posts message 6
//!   (switch off) only when `856325e8` (+0x2c) is set.
//! - **`AnimationSequenceItem`** (0x00861060, vtable 0x00ff1f4c): lead 1 s (`0x00861530`); enter `0x008614f0`
//!   sets state 3; update `0x008620a0` samples `Animation` (+0x24) at the item's local time into the owner's bone
//!   palette (`FUN_009ef7d0` / `FUN_009ef800` / `FUN_009ef970` / `FUN_009f2280`); stop `0x00861510` returns true.
//! - **`ApplyVehicleKickSequenceItem`** (0x5d7b0 → 0x00860760, vtable 0x00ff1a68): enter `FUN_008540d0` sends
//!   message 0x1d to the context player's car (player record +0xa0) with `405a1f8d` (+0x24) and `c07d2638` (+0x28)
//!   each splatted to a vector (`FUN_0085a550`).
//! - **`WaveSelectSequenceItem`** (0x0083aa60, vtable 0x00fef518; otherwise the wave item's): prepare
//!   `FUN_00836ea0` picks one id at random from each of `09768ebb` (+0xf4, count +0xfc) and `5512994f` (+0xf8,
//!   count +0xfe) with the sequence's RNG (multiplier 0x5851f42d4c957f2d) and loads them as wave resources
//!   (`FUN_00adaf70`, type 0xb) into +0xa8 / +0xb0, then plays as a `WaveSequenceItem` on its channels
//!   `1642b53c` (+0xe4) / `041d3e39` (+0xec).

use mw_bundle::ResourceId;
use mw_formats::genesys::{Object, Value, field_hash};

use crate::AssetStore;

/// `WCSequenceBehaviour` type id.
pub const WC_SEQUENCE_BEHAVIOUR: u32 = 0x3f412;
/// `CollisionSequenceBehaviour` type id.
pub const COLLISION_SEQUENCE_BEHAVIOUR: u32 = 0x7357e;
/// `WCShowBehaviour` type id.
pub const WC_SHOW_BEHAVIOUR: u32 = 0x2ada;

/// `CollisionSequenceBehaviour`: a vehicle contact plays `Vehicle_Hit` only while the object's upward speed is
/// below this (`.bss` 0x0110d580, read in snap2) …
pub const COLLISION_MAX_RISE: f32 = 6.0;
/// … the contact's value reaches this (`.bss` 0x0110e010) …
pub const COLLISION_MIN_CONTACT: f32 = 5.0;
/// … and the behaviour's timer has run this long (s, `.bss` 0x0110df60).
pub const COLLISION_REARM: f32 = 0.6;

/// A `WCSequenceBehaviour` of an object or one of its compound parts.
#[derive(Debug, Clone, PartialEq)]
pub struct WcSequence {
    pub object: ResourceId,
    pub label: String,
    /// +0x18 `0228a2f8`: contacts switch it on.
    pub on_by_contact: bool,
    /// +0x19 `713589ad`: a world contact switches it off.
    pub off_by_contact: bool,
    /// +0x1a `543527d0`: on from the start.
    pub at_start: bool,
    /// +0x1c: the `Sequence`.
    pub sequence: Option<ResourceId>,
    /// +0x24 `0ad467f1`: the runtime carries a bone palette its animation items write.
    pub animated: bool,
    /// The compound part it belongs to (`None`: the object's own).
    pub part: Option<usize>,
}

/// A `CollisionSequenceBehaviour`.
#[derive(Debug, Clone, PartialEq)]
pub struct WcCollisionSequence {
    pub object: ResourceId,
    pub part: Option<usize>,
    /// +0x1c `26b1191f`, +0x24 `cbe0bd7b` (played by `FUN_005b7350`, not read).
    pub other: [Option<ResourceId>; 2],
    /// +0x2c `Vehicle_Hit`.
    pub vehicle_hit: Option<ResourceId>,
}

/// The sequence behaviours of one object definition.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObjectSequences {
    pub sequences: Vec<WcSequence>,
    pub collisions: Vec<WcCollisionSequence>,
}

impl ObjectSequences {
    pub fn is_empty(&self) -> bool {
        self.sequences.is_empty() && self.collisions.is_empty()
    }
}

/// What a world-object item does to the object that plays its sequence (see the module docs).
#[derive(Debug, Clone, PartialEq)]
pub enum EntityItem {
    /// `AddBehaviourSequenceItem`: the behaviour is added and switched on for the item's span.
    AddBehaviour { behaviour: Option<ResourceId> },
    /// `BehaviourEnabledSequenceItem` (`gameplay` false: off again at the stop) and
    /// `BehaviourEnabledSequenceItemGameplay` (off at the stop only with `off_at_stop`, `856325e8`).
    BehaviourEnabled { label: String, gameplay: bool, off_at_stop: bool },
    /// `AnimationSequenceItem`: the clip drives the owner's bones over the item's span.
    Animation { animation: Option<ResourceId> },
    /// `ApplyVehicleKickSequenceItem`: car message 0x1d with `405a1f8d` and `c07d2638`.
    VehicleKick { a: f32, b: f32 },
    /// `WaveSelectSequenceItem`: one wave id of each list, picked at random.
    WaveSelect { first: Vec<u32>, second: Vec<u32>, channels: [Option<ResourceId>; 2] },
}

fn label(o: &Object, hash: u32) -> String {
    match o.get(hash) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn flag(o: &Object, hash: u32) -> bool {
    matches!(o.get(hash), Some(Value::Bool(true)))
}

fn handle(o: &Object, hash: u32) -> Option<ResourceId> {
    match o.get(hash)? {
        Value::Handle(h) => *h,
        Value::Ref(id) => Some(*id),
        _ => None,
    }
}

fn float(o: &Object, hash: u32) -> f32 {
    match o.get(hash) {
        Some(Value::Float(f)) => *f,
        _ => 0.0,
    }
}

fn ids(o: &Object, hash: u32) -> Vec<u32> {
    match o.get(hash) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| match v {
                Value::UInt(u) => Some(*u as u32),
                Value::Int(i) => Some(*i as u32),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

const LABEL: u32 = 0xd11f_ce34;

impl EntityItem {
    /// The item type's short name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::AddBehaviour { .. } => "AddBehaviourSequenceItem",
            Self::BehaviourEnabled { gameplay: false, .. } => "BehaviourEnabledSequenceItem",
            Self::BehaviourEnabled { gameplay: true, .. } => "BehaviourEnabledSequenceItemGameplay",
            Self::Animation { .. } => "AnimationSequenceItem",
            Self::VehicleKick { .. } => "ApplyVehicleKickSequenceItem",
            Self::WaveSelect { .. } => "WaveSelectSequenceItem",
        }
    }

    /// The item, when `kind` (the type's short name) is one of these.
    pub fn parse(kind: &str, o: &Object) -> Option<Self> {
        Some(match kind {
            "AddBehaviourSequenceItem" => Self::AddBehaviour { behaviour: handle(o, 0xd407_e3b8) },
            "BehaviourEnabledSequenceItem" => {
                Self::BehaviourEnabled { label: label(o, LABEL), gameplay: false, off_at_stop: true }
            }
            "BehaviourEnabledSequenceItemGameplay" => {
                Self::BehaviourEnabled { label: label(o, LABEL), gameplay: true, off_at_stop: flag(o, 0x8563_25e8) }
            }
            "AnimationSequenceItem" => Self::Animation { animation: handle(o, 0x1769_3e5e) },
            "ApplyVehicleKickSequenceItem" => Self::VehicleKick { a: float(o, 0x405a_1f8d), b: float(o, 0xc07d_2638) },
            "WaveSelectSequenceItem" => Self::WaveSelect {
                first: ids(o, 0x0976_8ebb),
                second: ids(o, 0x5512_994f),
                channels: [handle(o, 0x1642_b53c), handle(o, 0x041d_3e39)],
            },
            _ => return None,
        })
    }
}

impl AssetStore {
    /// The `WCSequenceBehaviour`s and `CollisionSequenceBehaviour`s of an object and its compound parts; ones that
    /// fail to decode are left out.
    pub fn object_sequences(&self, def: &mw_formats::ObjectDef) -> ObjectSequences {
        let mut out = ObjectSequences::default();
        let own = def.behaviours.iter().map(|b| (None, b));
        let parts =
            def.compound_parts.iter().enumerate().flat_map(|(i, p)| p.behaviours.iter().map(move |b| (Some(i), b)));
        for (part, b) in own.chain(parts) {
            let Some(id) = b.object else { continue };
            match b.type_id {
                WC_SEQUENCE_BEHAVIOUR => {
                    let Ok(o) = self.genesys_object(id) else { continue };
                    out.sequences.push(WcSequence {
                        object: id,
                        label: label(&o, field_hash("Label")),
                        on_by_contact: flag(&o, 0x0228_a2f8),
                        off_by_contact: flag(&o, 0x7135_89ad),
                        at_start: flag(&o, 0x5435_27d0),
                        sequence: handle(&o, field_hash("Sequence")),
                        animated: flag(&o, 0x0ad4_67f1),
                        part,
                    });
                }
                COLLISION_SEQUENCE_BEHAVIOUR => {
                    let Ok(o) = self.genesys_object(id) else { continue };
                    out.collisions.push(WcCollisionSequence {
                        object: id,
                        part,
                        other: [handle(&o, 0x26b1_191f), handle(&o, 0xcbe0_bd7b)],
                        vehicle_hit: handle(&o, field_hash("Vehicle_Hit")),
                    });
                }
                _ => {}
            }
        }
        out
    }
}
