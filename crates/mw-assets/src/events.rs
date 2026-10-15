//! Offline events (races, speed runs, ambushes, ...): the career's `OfflineEvent`s joined with the world's
//! trigger volumes they name, the cars allowed in them and their opponents. Rules: docs/decomp.md ("Races");
//! format: docs/formats.md ("Events").
//!
//! Where the data lives:
//!
//! - `GAMELOGIC/GAMEMODES.BNDL`: one large GenesysObject (`0100000000045d5b`, untyped root) whose `Item` array
//!   holds every `OfflineEvent`, `OnlineEvent`, `GameMode`, `EventArena`, `RaceBalancingData`, ... as
//!   polymorphic structs. The events are found by their type name, not by a fixed resource ID.
//! - `GAMELOGIC/GAMEPLAY.BNDL`: `EventList`s (the career's list of event ids), `Gameplay.AllowedVehicleList`
//!   (which cars may enter an event, with their mods), `AIPlayerType` (opponent driving profiles).
//! - `HAWAII/TRIGGERS.BNDL` (`TriggerData`): the checkpoints are kind-0 **boxes** (gates about 2 m deep, the
//!   road's width wide, 36 m high; local Z is the gate's normal) and the start positions (`SpawnPosition`, each
//!   opponent's `Placement`) are kind-4 **points** with a unit direction.
//!
//! References between them are `CgsCore.UniqueId`s (u32), the same numbers as the trigger records' ids.
//!
//! | type | offset | field | meaning |
//! |------|--------|-------|---------|
//! | `OfflineEvent` (248 B; first 76 B = `Event`) | 0x18 | `655d3f5f` | the event's id (what `EventList`s hold) |
//! | | 0x1c | `GameMode` | id of a `GameMode` item: 730000 races, 955569 speed runs, 390185 ambushes, ... |
//! | | 0x64 | `Checkpoints` UniqueId[] | trigger box ids, in driving order; for circuits the last is the start line |
//! | | 0x84 | `Name` | language string id |
//! | | 0x9c | `SpawnPosition` | trigger point id: the player's start |
//! | | 0xa4 | `TypeName` | language string id of the event type ("Circuit", "Sprint", ...) |
//! | | 0xa8 / 0xac | `TargetSpeed` / `TargetTime` float[] | speed run / timed targets (3 tiers) |
//! | | 0xb8 | `c0756b63` `CustomChevron`[] | polylines (x[], y[], z[]) of the chevron barriers along the route |
//! | | 0xc0 | `1795ca2a` `AIPlayerInformation`[] | opponents: `AIPlayerType`, `Placement` (trigger point id), two ids |
//! | | 0xc4 | `d4b38a64` `AlternateRoutes`[] | `Checkpoint` + alternative checkpoint ids |
//! | | 0x5c | `AllowedVehicleList` | id of a `Gameplay.AllowedVehicleList` (GAMEPLAY.BNDL) |
//! | | 0xec / 0xf0 | `NitrousAllowed` / `TrafficEnabled` | bools |
//! | | 0xb0 / 0xf1 | `ce4961e7` / `90f52617` | traffic density base / use it (else the hour-of-day table) |
//! | | 0xf6 | `Laps` u8 | 1 for sprints, 2–3 for circuits |
//! | | 0x60 / 0x70 / 0x74 | `AROutro` / `EventIntro` / `EventOutro` | sequence UniqueIds (`mw_assets::sequence`) |
//! | | 0x78 / 0x80 | `FeedbackMessageGroup` / `IntroMessageGroup` | feedback group bundles (`EN_US/FEEDBACKGROUPS/<id>.BNDL`) |
//! | `GameMode` | 0x44 | `09d6656d` | crash reset delay, s (decomp.md "Crashes") |
//! | | 0x0c / 0x10 / 0x24 / 0x34 | `AROutro` / `CountdownSequence` / `Intro` / `ResultsSequence` | sequence UniqueIds |
//! | | 0x75 | `FeedbackGroupID` u8 | the game-mode feedback group (2 → 604180) |
//! | `Gameplay.AllowedVehicleList` | | `193ad61b` `VehicleAndMods`[] | `Vehicle` (vehicle list id), `Mods` u8[], `Difficulty` |

use std::collections::HashMap;

use mw_bundle::{ResourceId, ResourceType};
use mw_formats::genesys::{Object, Value};
use mw_formats::{Trigger, TriggerShape};

use crate::{AssetStore, Error, Result};

mod h {
    use mw_formats::genesys::field_hash;

    pub const ITEM: u32 = field_hash("Item");
    pub const ID: u32 = 0x655d_3f5f;
    pub const GAME_MODE: u32 = field_hash("GameMode");
    pub const NAME: u32 = field_hash("Name");
    pub const DESCRIPTION: u32 = field_hash("Description");
    pub const TYPE_NAME: u32 = field_hash("TypeName");
    pub const CHECKPOINTS: u32 = field_hash("Checkpoints");
    pub const SPAWN_POSITION: u32 = field_hash("SpawnPosition");
    pub const LAPS: u32 = field_hash("Laps");
    pub const TARGET_SPEED: u32 = field_hash("TargetSpeed");
    pub const TARGET_TIME: u32 = field_hash("TargetTime");
    pub const ALLOWED_VEHICLE_LIST: u32 = field_hash("AllowedVehicleList");
    pub const NITROUS_ALLOWED: u32 = field_hash("NitrousAllowed");
    pub const TRAFFIC_ENABLED: u32 = field_hash("TrafficEnabled");
    pub const TRAFFIC_DENSITY: u32 = 0xce49_61e7;
    pub const TRAFFIC_DENSITY_FIXED: u32 = 0x90f5_2617;
    pub const AI_PLAYERS: u32 = 0x1795_ca2a;
    pub const AI_PLAYER_TYPE: u32 = field_hash("AIPlayerType");
    pub const PLACEMENT: u32 = field_hash("Placement");
    pub const AI_9B4E: u32 = 0x9b4e_35f7;
    pub const AI_2968: u32 = 0x2968_e77a;
    pub const CUSTOM_CHEVRONS: u32 = 0xc075_6b63;
    pub const CHEVRON_X: u32 = 0xc6bc_8827;
    pub const CHEVRON_Y: u32 = 0xc27d_9491;
    pub const CHEVRON_Z: u32 = 0xcf3e_b14b;
    pub const CHEVRON_FLAG: u32 = 0xf8cc_f73e;
    pub const ALTERNATE_ROUTES: u32 = 0xd4b3_8a64;
    pub const CHECKPOINT: u32 = field_hash("Checkpoint");
    pub const ALTERNATES: u32 = 0xfe6e_f8d8;
    pub const CRASH_RESET_DELAY: u32 = 0x09d6_656d;
    pub const INTRO_TIME: u32 = 0x15f0_7dbf;
    pub const TIMEOUT: u32 = 0xfe4f_2d8c;
    pub const GATE_MARKERS: u32 = 0xf112_3d7b;
    pub const RESPAWN_TO_RACER: u32 = 0xbc5e_a1ae;
    pub const FIRST_PLACE_ONLY: u32 = 0xc8f2_6580;
    pub const SKIP_PLAYER_CAR_AI: u32 = 0xc75c_9d90;
    pub const VEHICLES: u32 = 0x193a_d61b;
    pub const VEHICLE: u32 = field_hash("Vehicle");
    pub const MODS: u32 = field_hash("Mods");
    pub const DIFFICULTY: u32 = field_hash("Difficulty");
    pub const EVENT_LIST_ITEMS: u32 = 0xa1d5_23c0;
    // Sequences (UniqueIds of `Sequence`s in feedback groups; names cracked, see sequence.rs)
    pub const EVENT_INTRO: u32 = field_hash("EventIntro");
    pub const EVENT_OUTRO: u32 = field_hash("EventOutro");
    pub const AR_OUTRO: u32 = field_hash("AROutro");
    pub const FEEDBACK_MESSAGE_GROUP: u32 = field_hash("FeedbackMessageGroup");
    pub const INTRO_MESSAGE_GROUP: u32 = field_hash("IntroMessageGroup");
    pub const COUNTDOWN_SEQUENCE: u32 = field_hash("CountdownSequence");
    pub const MODE_INTRO: u32 = field_hash("Intro");
    pub const RESULTS_SEQUENCE: u32 = field_hash("ResultsSequence");
    pub const FEEDBACK_GROUP_ID: u32 = field_hash("FeedbackGroupID");
    pub const MUSIC_PLAYLIST: u32 = field_hash("MusicPlaylist");
}

const GAME_MODES: &str = "GAMELOGIC/GAMEMODES.BNDL";
pub(crate) const GAMEPLAY: &str = "GAMELOGIC/GAMEPLAY.BNDL";

/// What kind of event, from its `GameMode` and `TypeName` (the type names' English texts: "Circuit",
/// "Sprint", "Speed Run", "Ambush", ...; see [`EventKind::from_ids`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EventKind {
    /// Game mode 730000 with several laps' worth of data (`TypeName` 979287): laps over a closed loop.
    Circuit,
    /// Game mode 730000, `TypeName` 979289: point to point.
    Sprint,
    /// Game mode 730000, `TypeName` 457003: a one-on-one race against a Most Wanted (blacklist) car.
    MostWanted,
    /// Game mode 955569 (`TypeName` 1472095): a point-to-point run against `TargetSpeed` (average speed).
    SpeedRun,
    /// Game mode 390185 (`TypeName` 1119989): escape the police from a spawn, no checkpoints.
    Ambush,
    /// Anything else (DLC modes, free-roam placeholders): the raw game mode id.
    Other(u32),
}

impl Default for EventKind {
    fn default() -> Self {
        Self::Other(0)
    }
}

impl EventKind {
    pub fn from_ids(game_mode: u32, type_name: u32) -> Self {
        match (game_mode, type_name) {
            (730000, 979287) => Self::Circuit,
            (730000, 979289) => Self::Sprint,
            (730000, 457003) => Self::MostWanted,
            (955569, _) => Self::SpeedRun,
            (390185, _) => Self::Ambush,
            (m, _) => Self::Other(m),
        }
    }

    /// Whether the event is a race against opponents over checkpoints.
    pub fn is_race(self) -> bool {
        matches!(self, Self::Circuit | Self::Sprint | Self::MostWanted)
    }

    pub fn label(self) -> String {
        match self {
            Self::Circuit => "circuit".into(),
            Self::Sprint => "sprint".into(),
            Self::MostWanted => "most-wanted".into(),
            Self::SpeedRun => "speed-run".into(),
            Self::Ambush => "ambush".into(),
            Self::Other(m) => format!("mode-{m}"),
        }
    }
}

/// A position with a heading: a trigger point (`SpawnPosition`, an opponent's `Placement`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StartPoint {
    pub id: u32,
    pub position: [f32; 3],
    /// Unit direction the car faces (world space; y is small: the road's slope).
    pub direction: [f32; 3],
}

/// A checkpoint gate: a trigger box. `size` is the full extent (x across the road, y up, z through the gate);
/// local Z (`rotation` applied to +Z) is the gate's normal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gate {
    pub id: u32,
    pub position: [f32; 3],
    /// Quaternion x, y, z, w.
    pub rotation: [f32; 4],
    pub size: [f32; 3],
}

impl Gate {
    fn rotate(&self, v: [f32; 3]) -> [f32; 3] {
        let [x, y, z, w] = self.rotation;
        let t = [2.0 * (y * v[2] - z * v[1]), 2.0 * (z * v[0] - x * v[2]), 2.0 * (x * v[1] - y * v[0])];
        [
            v[0] + w * t[0] + (y * t[2] - z * t[1]),
            v[1] + w * t[1] + (z * t[0] - x * t[2]),
            v[2] + w * t[2] + (x * t[1] - y * t[0]),
        ]
    }

    /// The gate's normal (local +Z in world space).
    pub fn normal(&self) -> [f32; 3] {
        self.rotate([0.0, 0.0, 1.0])
    }

    /// The gate's across-road axis (local +X in world space).
    pub fn across(&self) -> [f32; 3] {
        self.rotate([1.0, 0.0, 0.0])
    }

    /// As the trigger record it came from (for [`Trigger::contains`]).
    pub fn trigger(&self) -> Trigger {
        Trigger {
            position: self.position,
            id: self.id,
            kind: 0,
            flags: 0,
            shape: TriggerShape::Box { rotation: self.rotation, size: self.size },
        }
    }
}

/// An opponent slot (`OfflineEvent.AIPlayerInformation`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Opponent {
    /// `AIPlayerType` id (GAMEPLAY.BNDL): the driver profile.
    pub ai_type: u32,
    /// Where the opponent starts (`Placement`, a trigger point), if it resolves.
    pub start: Option<StartPoint>,
    pub placement_id: u32,
    /// `9b4e35f7` and `2968e77a`: ids into GAMEPLAY.BNDL (`2968e77a` matches `ColourId`s of paint data: the
    /// opponent's colour, inferred); `9b4e35f7` unknown (the opponent's car?).
    pub unknown_9b4e: u32,
    pub colour: u32,
}

/// A chevron barrier line (`CustomChevron`): points along which the route's arrow boards stand.
#[derive(Debug, Clone, PartialEq)]
pub struct Chevron {
    pub id: u32,
    pub points: Vec<[f32; 3]>,
    /// `f8ccf73e` (unknown flag).
    pub flag: bool,
}

/// A car allowed in an event (`Gameplay.AllowedVehicleList.VehicleAndMods`).
#[derive(Debug, Clone, PartialEq)]
pub struct AllowedVehicle {
    /// Vehicle list id (`VEHICLES/VEH_<id>_HI.BNDL`).
    pub vehicle: u32,
    /// `Mods` values (per-slot levels, meaning open).
    pub mods: Vec<i64>,
    pub difficulty: i64,
    /// The AI rubber band's pace multiplier when the player drives this car.
    pub ai_pace: f32,
}

/// `OfflineEvent.AlternateRoutes`: a checkpoint that may be replaced by others.
#[derive(Debug, Clone, PartialEq)]
pub struct AlternateRoute {
    pub checkpoint: u32,
    pub alternatives: Vec<u32>,
}

/// One offline event, resolved against the world's triggers.
#[derive(Debug, Clone, Default)]
pub struct Event {
    pub id: u32,
    pub kind: EventKind,
    pub game_mode: u32,
    pub type_name: u32,
    /// Language string ids.
    pub name: u32,
    pub description: u32,
    pub laps: u8,
    /// `RaceBalancingData` and `RaceBalancingProfile` unique ids (+0x94 / +0x98; GeneSys resource id =
    /// 0x0100000000000000 | id): the AI rubber band (`ai.rs`). 0 = none.
    pub balancing: (u32, u32),
    pub spawn: Option<StartPoint>,
    pub spawn_id: u32,
    pub checkpoints: Vec<Gate>,
    /// Checkpoint ids that did not resolve to a trigger box (should be empty).
    pub missing_checkpoints: Vec<u32>,
    pub opponents: Vec<Opponent>,
    pub chevrons: Vec<Chevron>,
    pub alternate_routes: Vec<AlternateRoute>,
    pub allowed_vehicles: Vec<AllowedVehicle>,
    pub target_speed: Vec<f32>,
    pub target_time: Vec<f32>,
    pub nitrous_allowed: bool,
    /// `TrafficEnabled` (+0xf0): off, the event clears the traffic (message 0xf, `FUN_0070caa0`).
    pub traffic_enabled: bool,
    /// `ce4961e7` (+0xb0): the traffic density base for the event (`world+0xa2b74`, multiplies every flow's
    /// `Density`; message 4 from `FUN_0070caa0`). 1 in free drive, 0.1–1 in races.
    pub traffic_density: f32,
    /// `90f52617` (+0xf1): use `traffic_density`; when clear, the density follows the hour of day
    /// (`TrafficSystemConfig` +0x0c, re-sent when the hour changes, `FUN_006e6380`). Set in all but two events.
    pub traffic_density_fixed: bool,
    /// `c8f26580` (+0xe9): the event is won only in first place (else in the top three; `FUN_006b9d00`).
    /// Set on the Most Wanted races.
    pub first_place_only: bool,
    /// `c75c9d90` (+0xee): an opponent slot whose car is the player's own car is skipped (`FUN_00686f30`).
    pub skip_player_car_ai: bool,
    /// `EventIntro` (+0x70): UniqueId of the intro `Sequence` (in `intro_message_group`), 0 = none.
    pub event_intro: u32,
    /// `EventOutro` (+0x74): the outro `Sequence` played after the finish (in the game mode's feedback group).
    pub event_outro: u32,
    /// `AROutro` (+0x60): the Autolog-recommendations outro (the same sequence as `EventOutro` in every event).
    pub ar_outro: u32,
    /// `FeedbackMessageGroup` (+0x78) and `IntroMessageGroup` (+0x80): `EN_US/FEEDBACKGROUPS/<id>.BNDL`.
    pub feedback_message_group: u32,
    pub intro_message_group: u32,
}

impl Event {
    /// The route as a polyline: the start, then every gate centre in order (for a circuit the last gate is
    /// the start line, so one lap is this polyline).
    pub fn route(&self) -> Vec<[f32; 3]> {
        self.spawn.iter().map(|s| s.position).chain(self.checkpoints.iter().map(|g| g.position)).collect()
    }

    /// The car the event is first offered with (the first entry of its allowed vehicle list).
    pub fn default_vehicle(&self) -> Option<u32> {
        self.allowed_vehicles.first().map(|v| v.vehicle)
    }
}

/// A `GameMode` item.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameMode {
    pub id: u32,
    pub name: u32,
    /// `09d6656d` (+0x44): seconds before a crashed car is reset (`FUN_006d39d0`).
    pub crash_reset_delay: f32,
    /// `15f07dbf` (+0x48): the intro before go, s (`FUN_0070caa0`, `FUN_006e2640`); the 3-2-1 countdown is its
    /// last 3 s. 10 for races and ambushes, 0 for speed runs.
    pub intro_time: f32,
    /// `fe4f2d8c` (+0x50): the event's timeout, s (armed by `FUN_006f0f80`; 900 for races).
    pub timeout: f32,
    /// `f1123d7b` (+0x6e): the player's gate markers are shown (`FUN_007545b0`).
    pub gate_markers: bool,
    /// `bc5ea1ae` (+0x70): without a checkpoint list a respawn faces the nearest racer (`FUN_006c9fb0`).
    pub respawn_to_racer: bool,
    /// `AROutro` (+0x0c), `CountdownSequence` (+0x10), `Intro` (+0x24) and `ResultsSequence` (+0x34): sequence
    /// UniqueIds. Offline modes have no countdown, intro or results sequence (the online modes do).
    pub ar_outro: u32,
    pub countdown_sequence: u32,
    pub intro_sequence: u32,
    pub results_sequence: u32,
    /// `FeedbackGroupID` (+0x75): which game-mode feedback group is loaded (2 → 604180, 1 → 604179).
    pub feedback_group_id: u8,
    /// `MusicPlaylist`: the `SongList` the HSM's "Play Default Game Mode Music" (`PLAYSONGLIST`, `USEDEFAULT`)
    /// plays: 1622455 on every offline mode (`mw_assets::music`).
    pub music_playlist: u32,
}

/// Every offline event plus the game modes and the career's event lists.
#[derive(Debug, Clone, Default)]
pub struct Events {
    pub events: Vec<Event>,
    pub game_modes: Vec<GameMode>,
    /// `EventList`s (GAMEPLAY.BNDL): (list id, event ids).
    pub event_lists: Vec<(u32, Vec<u32>)>,
}

impl Events {
    pub fn get(&self, id: u32) -> Option<&Event> {
        self.events.iter().find(|e| e.id == id)
    }

    pub fn game_mode(&self, id: u32) -> Option<&GameMode> {
        self.game_modes.iter().find(|m| m.id == id)
    }
}

fn uint(v: Option<&Value>) -> u32 {
    match v {
        Some(Value::UInt(u)) => *u as u32,
        Some(Value::Int(i)) => *i as u32,
        _ => 0,
    }
}

fn int(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::UInt(u)) => *u as i64,
        Some(Value::Int(i)) => *i,
        _ => 0,
    }
}

fn float(v: Option<&Value>) -> f32 {
    if let Some(Value::Float(f)) = v { *f } else { 0.0 }
}

fn boolean(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Bool(true)))
}

fn array(v: Option<&Value>) -> &[Value] {
    if let Some(Value::Array(a)) = v { a } else { &[] }
}

fn uints(v: Option<&Value>) -> Vec<u32> {
    array(v).iter().map(|x| uint(Some(x))).collect()
}

fn floats(v: Option<&Value>) -> Vec<f32> {
    array(v).iter().map(|x| float(Some(x))).collect()
}

fn structs(v: Option<&Value>) -> impl Iterator<Item = &Object> {
    array(v).iter().filter_map(|x| if let Value::Struct(o) = x { Some(o) } else { None })
}

/// Every struct in the `Item` arrays of a bundle's objects, by type name (without `Genesys.Gen.`).
pub(crate) fn items(store: &AssetStore, bundle: &str) -> Result<Vec<Object>> {
    let b = store.bundle(bundle).ok_or(Error::Missing(ResourceId::NULL))?;
    let mut out = Vec::new();
    for e in b.entries_of_type(ResourceType::GenesysObject) {
        let Ok(root) = store.decode_genesys(&b.load(e)?) else { continue };
        if let Some(Value::Array(a)) = root.get(h::ITEM) {
            out.extend(a.iter().filter_map(|v| if let Value::Struct(o) = v { Some(o.clone()) } else { None }));
        } else {
            out.push(root);
        }
    }
    Ok(out)
}

fn short(o: &Object) -> &str {
    o.type_name.strip_prefix("Genesys.Gen.").unwrap_or(&o.type_name)
}

impl AssetStore {
    /// Every `OfflineEvent` with its checkpoints, start, opponents and cars resolved.
    pub fn events(&self) -> Result<Events> {
        let triggers = self.trigger_data()?;
        let by_id: HashMap<u32, &Trigger> = triggers.triggers.iter().map(|t| (t.id, t)).collect();
        let point = |id: u32| -> Option<StartPoint> {
            match by_id.get(&id)?.shape {
                TriggerShape::Point { direction } => Some(StartPoint { id, position: by_id[&id].position, direction }),
                _ => None,
            }
        };
        let gate = |id: u32| -> Option<Gate> {
            let t = by_id.get(&id)?;
            match t.shape {
                TriggerShape::Box { rotation, size } => Some(Gate { id, position: t.position, rotation, size }),
                _ => None,
            }
        };

        let gameplay = items(self, GAMEPLAY)?;
        let mut allowed: HashMap<u32, Vec<AllowedVehicle>> = HashMap::new();
        let mut event_lists = Vec::new();
        for o in &gameplay {
            match short(o) {
                "Gameplay.AllowedVehicleList" => {
                    let list = structs(o.get(h::VEHICLES))
                        .map(|v| AllowedVehicle {
                            vehicle: uint(v.get(h::VEHICLE)),
                            mods: array(v.get(h::MODS)).iter().map(|m| int(Some(m))).collect(),
                            difficulty: int(v.get(h::DIFFICULTY)),
                            // `a17a5e72` (VehicleAndMods +0x10): the AI pace multiplier when the player drives
                            // this car (`FUN_0063a0f0`; 1.0 by default).
                            ai_pace: match v.get(0xa17a5e72) {
                                Some(Value::Float(f)) if *f > 0.0 => *f,
                                _ => 1.0,
                            },
                        })
                        .collect();
                    allowed.insert(uint(o.get(h::ID)), list);
                }
                "EventList" => event_lists.push((uint(o.get(h::ID)), uints(o.get(h::EVENT_LIST_ITEMS)))),
                _ => {}
            }
        }

        let mut out = Events { event_lists, ..Default::default() };
        for o in items(self, GAME_MODES)? {
            match short(&o) {
                "GameMode" => out.game_modes.push(GameMode {
                    id: uint(o.get(h::ID)),
                    name: uint(o.get(h::NAME)),
                    crash_reset_delay: float(o.get(h::CRASH_RESET_DELAY)),
                    intro_time: float(o.get(h::INTRO_TIME)),
                    timeout: float(o.get(h::TIMEOUT)),
                    gate_markers: boolean(o.get(h::GATE_MARKERS)),
                    respawn_to_racer: boolean(o.get(h::RESPAWN_TO_RACER)),
                    ar_outro: uint(o.get(h::AR_OUTRO)),
                    countdown_sequence: uint(o.get(h::COUNTDOWN_SEQUENCE)),
                    intro_sequence: uint(o.get(h::MODE_INTRO)),
                    results_sequence: uint(o.get(h::RESULTS_SEQUENCE)),
                    feedback_group_id: uint(o.get(h::FEEDBACK_GROUP_ID)) as u8,
                    music_playlist: uint(o.get(h::MUSIC_PLAYLIST)),
                }),
                "OfflineEvent" => {
                    let game_mode = uint(o.get(h::GAME_MODE));
                    let type_name = uint(o.get(h::TYPE_NAME));
                    let ids = uints(o.get(h::CHECKPOINTS));
                    let checkpoints: Vec<Gate> = ids.iter().filter_map(|&id| gate(id)).collect();
                    let missing_checkpoints = ids.iter().copied().filter(|&id| gate(id).is_none()).collect();
                    let spawn_id = uint(o.get(h::SPAWN_POSITION));
                    out.events.push(Event {
                        id: uint(o.get(h::ID)),
                        kind: EventKind::from_ids(game_mode, type_name),
                        game_mode,
                        type_name,
                        name: uint(o.get(h::NAME)),
                        description: uint(o.get(h::DESCRIPTION)),
                        laps: uint(o.get(h::LAPS)) as u8,
                        // +0x94 / +0x98: the rubber band (`FUN_0070caa0` → `FUN_0063a0f0`).
                        balancing: (uint(o.get(0xcc1a2667)), uint(o.get(0x1c6e495f))),
                        spawn: point(spawn_id),
                        spawn_id,
                        checkpoints,
                        missing_checkpoints,
                        opponents: structs(o.get(h::AI_PLAYERS))
                            .map(|a| {
                                let placement_id = uint(a.get(h::PLACEMENT));
                                Opponent {
                                    ai_type: uint(a.get(h::AI_PLAYER_TYPE)),
                                    start: point(placement_id),
                                    placement_id,
                                    unknown_9b4e: uint(a.get(h::AI_9B4E)),
                                    colour: uint(a.get(h::AI_2968)),
                                }
                            })
                            .collect(),
                        chevrons: structs(o.get(h::CUSTOM_CHEVRONS))
                            .map(|c| {
                                let (x, y, z) = (
                                    floats(c.get(h::CHEVRON_X)),
                                    floats(c.get(h::CHEVRON_Y)),
                                    floats(c.get(h::CHEVRON_Z)),
                                );
                                Chevron {
                                    id: uint(c.get(h::ID)),
                                    points: x.iter().zip(&y).zip(&z).map(|((x, y), z)| [*x, *y, *z]).collect(),
                                    flag: boolean(c.get(h::CHEVRON_FLAG)),
                                }
                            })
                            .collect(),
                        alternate_routes: structs(o.get(h::ALTERNATE_ROUTES))
                            .map(|a| AlternateRoute {
                                checkpoint: uint(a.get(h::CHECKPOINT)),
                                alternatives: uints(a.get(h::ALTERNATES)),
                            })
                            .collect(),
                        allowed_vehicles: allowed
                            .get(&uint(o.get(h::ALLOWED_VEHICLE_LIST)))
                            .cloned()
                            .unwrap_or_default(),
                        target_speed: floats(o.get(h::TARGET_SPEED)),
                        target_time: floats(o.get(h::TARGET_TIME)),
                        nitrous_allowed: boolean(o.get(h::NITROUS_ALLOWED)),
                        first_place_only: boolean(o.get(h::FIRST_PLACE_ONLY)),
                        skip_player_car_ai: boolean(o.get(h::SKIP_PLAYER_CAR_AI)),
                        traffic_enabled: boolean(o.get(h::TRAFFIC_ENABLED)),
                        traffic_density: float(o.get(h::TRAFFIC_DENSITY)),
                        traffic_density_fixed: boolean(o.get(h::TRAFFIC_DENSITY_FIXED)),
                        event_intro: uint(o.get(h::EVENT_INTRO)),
                        event_outro: uint(o.get(h::EVENT_OUTRO)),
                        ar_outro: uint(o.get(h::AR_OUTRO)),
                        feedback_message_group: uint(o.get(h::FEEDBACK_MESSAGE_GROUP)),
                        intro_message_group: uint(o.get(h::INTRO_MESSAGE_GROUP)),
                    });
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// The materials the race route is marked with.
    pub fn route_markers(&self) -> Result<RouteMarkers> {
        let b = self.bundle("GLOBALCONFIG.BNDL").ok_or(Error::Missing(ResourceId::NULL))?;
        let mut config = None;
        for e in b.entries_of_type(ResourceType::GenesysObject) {
            if let Ok(root) = self.decode_genesys(&b.load(e)?)
                && let Some(c) = root.find("Genesys.Gen.CheckPointsConfig")
            {
                config = Some(c.clone());
                break;
            }
        }
        let c = config.ok_or(Error::Missing(ResourceId::NULL))?;
        let look = |hash: u32| -> Result<ChevronLook> {
            let id = ResourceId(0x0100_0000_0000_0000 | uint(c.get(hash)) as u64);
            let m = self.material(id)?;
            let param = |h: u32| m.parameters.iter().find(|p| p.name_hash == h).and_then(|p| p.value);
            Ok(ChevronLook {
                material: id,
                texture: m.samplers.first().and_then(|s| s.texture),
                colour: param(MATERIAL_DIFFUSE).unwrap_or([1.0; 4]),
                speed: param(SPEED).map_or(0.0, |v| v[0]),
            })
        };
        Ok(RouteMarkers {
            arrows: look(0x53a7_6c21)?,
            block: look(0xbeb3_01a3)?,
            poi: [uint(c.get(0x9c90_086c)), uint(c.get(mw_formats::genesys::field_hash("NormalPoi")))],
        })
    }
}

/// Shader parameter name hashes (`mwtool shaders` prints them).
const MATERIAL_DIFFUSE: u32 = 0xf6e2_7caa;
const SPEED: u32 = 0xf0d9_0109;

/// One of the two route-marker looks: a material of `HAWAII/GLOBALRESOURCES.BNDL` drawn on strips the game
/// builds along chevron lines (its shaders take positions and UVs only, no model).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChevronLook {
    pub material: ResourceId,
    /// The material's one texture (white; the arrows / crosses are in its alpha).
    pub texture: Option<ResourceId>,
    /// `materialDiffuse` (game colour units).
    pub colour: [f32; 4],
    /// `speed`: the arrows scroll by `frac(Time × speed)` texture widths (shader `Chevron`); 0 for the block.
    pub speed: f32,
}

/// The route markers (`GameLogicConfig.CheckpointConfig`, a `CheckPointsConfig` in GLOBALCONFIG.BNDL).
#[derive(Debug, Clone)]
pub struct RouteMarkers {
    /// Field `53a76c21` → material 0x253d6, shader `Chevron`: scrolling arrows.
    pub arrows: ChevronLook,
    /// Field `beb301a3` → material 0x298ce, shader `ChevronBlockRoad`: the red crosses closing side roads.
    pub block: ChevronLook,
    /// `9c90086c` and `NormalPoi`: map icon ids (POIs), for the map / HUD.
    pub poi: [u32; 2],
}
