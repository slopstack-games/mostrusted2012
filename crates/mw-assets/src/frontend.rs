//! Front-end data: the boot / title screen, the loading screens, the pause menu and where free roam starts
//! (docs/decomp.md "Front end: boot, loading and the free-roam start").
//!
//! - **Flow** (the game-logic `HSMData` of `GAMELOGIC/GAMEMODES.BNDL`, node "Top State"): "Run PageRunner2
//!   Context - Boot Flow" runs screen 491012 ([`BOOT_SCREEN`]): language select (skipped on PC by its locale
//!   check), then "Press Start" = widget group 491032, whose `Background` widget plays [`TITLE_SEQUENCE`] and
//!   whose screen transition plays [`BOOT_TRANSITION_SEQUENCE`] when the page exits. "Post Controller Bind
//!   Boot" loads the world and the profile, "InGame" waits for its resources, sets up the start point (new or
//!   continued game) and drops the black background.
//! - **Loading screen**: the UI manager shows one of the `UIConfig` layout lists (`eee932fa` for the general
//!   reasons, `a65d5d07` for the HSM's "Toggle Loading Screen"; both hold layout 108177, black with a spinner)
//!   while a reason is pending; the world-loaded flag needs the world's resources, the zone at the camera and
//!   its neighbours within 10 m resident, and traffic ready or 1.5 s (decomp).
//! - **Start**: a new game starts at `GameLogicConfig.beab0bc2` (start trigger 1156487) in `DefaultRollout`'s
//!   car; a continued free roam at the last swap (jack) spot of the current car, else the nearest swap spot
//!   holding it, else that default start (`FUN_006d0180`). The pose is the trigger's, speed 0.

use std::collections::HashMap;

use mw_bundle::{ResourceId, ResourceType};
use mw_formats::genesys::{Object, Value, field_hash};
use mw_formats::{TriggerShape, ui::object_id};

use crate::events::{GAMEPLAY, items};
use crate::{AssetStore, Error, Result};

/// The boot screen (`BootScreen`): language select, press start, the profile-loading black screen.
pub const BOOT_SCREEN: &str = "UI/SCREENS2/491012.BNDL";
/// The press-start group's `Background` sequence: intro movie 1689283 from 28 s, the looping title movie
/// 1689287 from 34.4 s, the music, the "BOOT" trigger at 37.4 s that lets the press-start state wait for a
/// key, and the press-start layout (1274392) from 34.38 s. Jumps: 0 → 28.1 s unless Japanese, 100.88 →
/// 73.952 s (music loop).
pub const TITLE_SEQUENCE: u32 = 1_022_357;
/// The press-start widget's own layouts: the white backdrop shape (1689284) and the full-screen `MoviePlayer`
/// (108048) the title movies play on.
pub const TITLE_LAYOUTS: [u32; 2] = [1_689_284, 108_048];
/// The boot screen's transition (`ScreenTransition` 1836019, exit condition `Signals.True`): 4.8 s of black
/// quad, the car-picture sequence ([`TRANSITION_IMAGES_SEQUENCE`]) from 0.25 s, a fade from black and the
/// tip strip with the logo, whooshes.
pub const BOOT_TRANSITION_SEQUENCE: u32 = 1_836_020;
/// The car pictures: two image layers (`Transitions.Image1` / `Image2`) cross-fading every ~10 s, looping
/// at 20 s back to 0.1 s.
pub const TRANSITION_IMAGES_SEQUENCE: u32 = 1_899_171;
/// The pause menu screen (`PAUSE`, offline items `StdData`, in-event items `Data`).
pub const PAUSE_SCREEN: &str = "UI/SCREENS2/431082.BNDL";

/// A placement: trigger point position and unit direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub trigger: u32,
    pub position: [f32; 3],
    pub direction: [f32; 3],
}

/// A `CarSwapSpot` (jack spot): its location box, the car parked there and that car's pose.
#[derive(Debug, Clone, PartialEq)]
pub struct SwapSpot {
    pub id: u32,
    /// `VehicleList` id of the parked car.
    pub vehicle: u32,
    /// `Placement`: the jack-spot location trigger (a box).
    pub location: u32,
    /// `0e3bcabe`: the car's trigger point.
    pub car: Option<Pose>,
    /// `ColourId` (+0x10): the parked car's paint, a `VehiclePaint` UniqueId of the car's palette
    /// (`FUN_00747aa0` copies it into the swap slot, `FUN_0075d290` spawns the car with it).
    pub colour: u32,
    /// `e36e9d9d` (+0x1c): the spot's second parking place, where the car the player arrived in is left
    /// (`FUN_00761d70` state 3 alternates between it and the car trigger); `None` when the id is no trigger.
    pub second: Option<Pose>,
}

/// Where and in what free roam starts.
#[derive(Debug, Clone, Default)]
pub struct StartConfig {
    /// `GameLogicConfig.DefaultRollout`'s `Vehicle` (`VehicleList` id).
    pub default_vehicle: Option<u32>,
    /// `GameLogicConfig.beab0bc2`: the default start point.
    pub default_start: Option<Pose>,
    pub swap_spots: Vec<SwapSpot>,
    /// `GameLogicConfig.ff4aef6f` (+0x284, 70): the speed in mph up to which a parked car can be swapped
    /// into (`FUN_007615f0`).
    pub swap_max_mph: Option<f32>,
}

impl StartConfig {
    /// The continued free-roam start for `vehicle` from `near` (`FUN_006d0180` → `FUN_006c1840`): the
    /// nearest swap spot holding the car (the game tries the discovered spots first and, before that, the
    /// profile's last swap spot when it held this car; we keep no discovery), else the default start.
    pub fn continue_pose(&self, vehicle: u32, near: [f32; 3]) -> Option<Pose> {
        let d = |p: &Pose| {
            let v = [p.position[0] - near[0], p.position[1] - near[1], p.position[2] - near[2]];
            v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
        };
        self.swap_spots
            .iter()
            .filter(|s| s.vehicle == vehicle)
            .filter_map(|s| s.car)
            .min_by(|a, b| d(a).total_cmp(&d(b)))
            .or(self.default_start)
    }
}

/// The loading screens' data (`UI/UICONFIG.BNDL`).
#[derive(Debug, Clone, Default)]
pub struct LoadingConfig {
    /// `UIConfig.eee932fa` (+0x80): layouts for the general loading reasons (108177).
    pub layouts: Vec<u32>,
    /// `UIConfig.a65d5d07` (+0x74): layouts for the HSM's "Toggle Loading Screen" (108177).
    pub toggle_layouts: Vec<u32>,
    /// `LoadingTipsConfig.TransitionImages` of the base game (`GamePack` 0): streamed pictures
    /// (`UI/IMAGES/STREAMED/<id>.BNDL`, one texture each).
    pub images: Vec<u32>,
    /// That entry's `f9ebff2a` (10): seconds per picture (our reading).
    pub image_seconds: f32,
    /// Tips (language ids) of the offline categories (`LoadingTipsConfig` lists except `Autolog` and `Online`).
    pub tips: Vec<u32>,
}

fn uint(o: &Object, hash: u32) -> u32 {
    match o.get(hash) {
        Some(Value::UInt(u)) => *u as u32,
        Some(Value::Int(i)) => *i as u32,
        Some(Value::Float(f)) => *f as u32,
        _ => 0,
    }
}

fn uints(v: Option<&Value>) -> Vec<u32> {
    match v {
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

fn roots(store: &AssetStore, bundle: &str) -> Result<Vec<Object>> {
    let b = store.bundle(bundle).ok_or(Error::Missing(ResourceId::NULL))?;
    let mut out = Vec::new();
    for e in b.entries_of_type(ResourceType::GenesysObject) {
        if let Ok(o) = store.decode_genesys(&b.load(e)?) {
            out.push(o);
        }
    }
    Ok(out)
}

/// Every struct of `type_name` under `o` (fields, arrays, nested structs).
fn collect<'a>(o: &'a Object, type_name: &str, out: &mut Vec<&'a Object>) {
    fn walk<'a>(v: &'a Value, name: &str, out: &mut Vec<&'a Object>) {
        match v {
            Value::Struct(o) => collect(o, name, out),
            Value::Array(a) => a.iter().for_each(|v| walk(v, name, out)),
            _ => {}
        }
    }
    if o.type_name == type_name {
        out.push(o);
    }
    for (_, v) in &o.fields {
        walk(v, type_name, out);
    }
}

mod h {
    use mw_formats::genesys::field_hash;
    pub const ID: u32 = 0x655d_3f5f;
    pub const DEFAULT_ROLLOUT: u32 = field_hash("DefaultRollout");
    pub const DEFAULT_START: u32 = 0xbeab_0bc2;
    pub const VEHICLE: u32 = field_hash("Vehicle");
    pub const VEHICLE_ID: u32 = field_hash("VehicleId");
    pub const PLACEMENT: u32 = field_hash("Placement");
    pub const SWAP_CAR: u32 = 0x0e3b_cabe;
    pub const COLOUR_ID: u32 = field_hash("ColourId");
    pub const SWAP_SECOND: u32 = 0xe36e_9d9d;
    pub const SWAP_MAX_MPH: u32 = 0xff4a_ef6f;
    pub const GENERAL_LAYOUTS: u32 = 0xeee9_32fa;
    pub const TOGGLE_LAYOUTS: u32 = 0xa65d_5d07;
    pub const TRANSITION_IMAGES: u32 = field_hash("TransitionImages");
    pub const GAME_PACK: u32 = field_hash("GamePack");
    pub const IMAGES: u32 = field_hash("Images");
    pub const IMAGE_SECONDS: u32 = 0xf9eb_ff2a;
    pub const AUTOLOG: u32 = field_hash("Autolog");
    pub const ONLINE: u32 = field_hash("Online");
    pub const TERMS: u32 = field_hash("Terms");
    pub const NAME: u32 = field_hash("Name");
    /// `ComplianceTerms.Term`'s PC string (the other three: PS3 `21901f63`, Vita `90ac0fb2`, X360 `0797e772`;
    /// told apart by "PRESS ANY KEY" / "PRESS START BUTTON" / "TOUCH TO START" / "PRESS START").
    pub const TERM_PC: u32 = 0x042a_361e;
    pub const ITEMS: u32 = field_hash("Items");
    pub const VALUE: u32 = field_hash("Value");
    pub const DISPLAY_STRING: u32 = field_hash("DisplayString");
}

/// The keyboard's button texts (`UIConfig.ControllerSettings.ButtonText`, the PC platform `f12bac41`, device
/// 2 = keyboard): `UIListItems` of `Value` (key name) → `DisplayString`.
const KEYBOARD_BUTTON_TEXT: ResourceId = ResourceId(0x0100_0000_0001_bbaa);
/// `UIListItems` 113577: the keyboard keys' button prompts (boxed key names, glyphs).
const KEYBOARD_BUTTON_PROMPT: ResourceId = ResourceId(0x0100_0000_0001_bba9);

impl AssetStore {
    /// The game logic's state machines (`HSMData` of `GAMELOGIC/GAMEMODES.BNDL`, or of another bundle).
    pub fn hsm_data(&self, bundle: &str) -> Result<mw_formats::hsm::HsmData> {
        let b = self.bundle(bundle).ok_or(Error::Missing(ResourceId::NULL))?;
        let e = b.entries_of_type(ResourceType::HSMData).next().ok_or(Error::Missing(ResourceId::NULL))?;
        Ok(mw_formats::hsm::HsmData::parse(&b.load(e)?)?)
    }

    /// The free-roam start data (GLOBALCONFIG's `GameLogicConfig`, GAMEPLAY's rollouts and swap spots,
    /// TRIGGERS' points).
    pub fn start_config(&self) -> Result<StartConfig> {
        let triggers = self.trigger_data()?;
        let by_id: HashMap<u32, _> = triggers.triggers.iter().map(|t| (t.id, t)).collect();
        let pose = |id: u32| -> Option<Pose> {
            let t = by_id.get(&id)?;
            match t.shape {
                TriggerShape::Point { direction } => Some(Pose { trigger: id, position: t.position, direction }),
                // Swap-spot cars are boxes around the parked car (length along local +Z): `FUN_0042f250` gives a
                // trigger's direction, a point's vector or a box's local Z.
                TriggerShape::Box { rotation: [x, y, z, w], .. } => Some(Pose {
                    trigger: id,
                    position: t.position,
                    direction: [2.0 * (x * z + w * y), 2.0 * (y * z - w * x), 1.0 - 2.0 * (x * x + y * y)],
                }),
                TriggerShape::Sphere { .. } => None,
            }
        };
        let mut cfg = StartConfig::default();
        let mut rollout = 0;
        for root in roots(self, "GLOBALCONFIG.BNDL")? {
            if let Some(g) = root
                .find("Genesys.Gen.GameLogicConfig")
                .or(Some(&root).filter(|r| r.type_name == "Genesys.Gen.GameLogicConfig"))
            {
                rollout = uint(g, h::DEFAULT_ROLLOUT);
                cfg.default_start = pose(uint(g, h::DEFAULT_START));
                cfg.swap_max_mph = match g.get(h::SWAP_MAX_MPH) {
                    Some(Value::Float(f)) => Some(*f),
                    _ => None,
                };
            }
        }
        for o in items(self, GAMEPLAY)? {
            let mut found = Vec::new();
            collect(&o, "Genesys.Gen.Rollout", &mut found);
            if let Some(r) = found.into_iter().find(|r| uint(r, h::ID) == rollout) {
                cfg.default_vehicle = Some(uint(r, h::VEHICLE));
            }
            let mut spots = Vec::new();
            collect(&o, "Genesys.Gen.CarSwapSpot", &mut spots);
            cfg.swap_spots.extend(spots.into_iter().map(|s| SwapSpot {
                id: uint(s, h::ID),
                vehicle: uint(s, h::VEHICLE_ID),
                location: uint(s, h::PLACEMENT),
                car: pose(uint(s, h::SWAP_CAR)),
                colour: uint(s, h::COLOUR_ID),
                second: pose(uint(s, h::SWAP_SECOND)),
            }));
        }
        Ok(cfg)
    }

    /// The loading screens' layouts, pictures and tips (`UI/UICONFIG.BNDL`).
    pub fn loading_config(&self) -> Result<LoadingConfig> {
        let mut cfg = LoadingConfig { image_seconds: 10.0, ..Default::default() };
        for root in roots(self, "UI/UICONFIG.BNDL")? {
            if root.type_name == "Genesys.Gen.UIConfig" {
                cfg.layouts = uints(root.get(h::GENERAL_LAYOUTS));
                cfg.toggle_layouts = uints(root.get(h::TOGGLE_LAYOUTS));
            }
            if root.type_name == "Genesys.Gen.UI.LoadingTipsConfig" {
                for (hash, v) in &root.fields {
                    match (*hash, v) {
                        (h::TRANSITION_IMAGES, Value::Array(a)) => {
                            for t in a {
                                if let Value::Struct(t) = t
                                    && uint(t, h::GAME_PACK) == 0
                                {
                                    cfg.images = uints(t.get(h::IMAGES));
                                    if let Some(Value::Float(s)) = t.get(h::IMAGE_SECONDS) {
                                        cfg.image_seconds = *s;
                                    } else if uint(t, h::IMAGE_SECONDS) > 0 {
                                        cfg.image_seconds = uint(t, h::IMAGE_SECONDS) as f32;
                                    }
                                }
                            }
                        }
                        (h::AUTOLOG | h::ONLINE | h::ID, _) => {}
                        (_, Value::Array(_)) => cfg.tips.extend(uints(Some(v))),
                        _ => {}
                    }
                }
            }
        }
        Ok(cfg)
    }

    /// `ComplianceTerm.Value["<name>"]`: the PC string id of a compliance term ("Press", "PressStart").
    pub fn compliance_term(&self, name: &str) -> Option<u32> {
        for root in roots(self, "UI/UICONFIG.BNDL").ok()? {
            let Some(Value::Array(terms)) = root.get(h::TERMS) else { continue };
            for t in terms {
                if let Value::Struct(t) = t
                    && matches!(t.get(h::NAME), Some(Value::Str(n)) if n == name)
                {
                    return Some(uint(t, h::TERM_PC)).filter(|&s| s != 0);
                }
            }
        }
        None
    }

    /// The keyboard's text for a key (`KEY_RETURN` → "RETURN"): a language id.
    pub fn keyboard_button_text(&self, key: &str) -> Option<u32> {
        let o = self.genesys_object(KEYBOARD_BUTTON_TEXT).ok()?;
        let Some(Value::Array(items)) = o.get(h::ITEMS) else { return None };
        items.iter().find_map(|i| match i {
            Value::Struct(i) if matches!(i.get(h::VALUE), Some(Value::Str(v)) if v == key) => {
                Some(uint(i, h::DISPLAY_STRING))
            }
            _ => None,
        })
    }

    /// The keyboard's button prompt for a key (`KEY_ESCAPE` → "ESC" in a key box, `KEY_RETURN` → the Enter glyph):
    /// a language id from the `UIListItems` 113577 (the boxed form; 113578 holds the plain names).
    pub fn keyboard_button_prompt(&self, key: &str) -> Option<u32> {
        let o = self.genesys_object(KEYBOARD_BUTTON_PROMPT).ok()?;
        let Some(Value::Array(items)) = o.get(h::ITEMS) else { return None };
        items.iter().find_map(|i| match i {
            Value::Struct(i) if matches!(i.get(h::VALUE), Some(Value::Str(v)) if v == key) => {
                Some(uint(i, h::DISPLAY_STRING))
            }
            _ => None,
        })
    }

    /// The texture of a streamed UI picture (`UI/IMAGES/STREAMED/<id>.BNDL`; the texture's id is the
    /// picture's id).
    pub fn streamed_image(&self, id: u32) -> Option<ResourceId> {
        let b = self.bundle(format!("UI/IMAGES/STREAMED/{id}.BNDL"))?;
        let e = b.entries_of_type(ResourceType::Texture).next()?;
        Some(e.id)
    }

    /// A `Widget_MenuData`'s custom items of a screen: (action token, name language id, active) per item, for
    /// the widget with this name and UniqueId (screens repeat widget names).
    pub fn menu_items(&self, screen: &str, widget_id: u32) -> Result<Vec<(String, u32, bool)>> {
        let o = self.ui_screen_object(screen)?;
        let mut widgets = Vec::new();
        collect(&o, "Genesys.Gen.Widget_MenuData", &mut widgets);
        let w =
            widgets.into_iter().find(|w| uint(w, h::ID) == widget_id).ok_or(Error::Missing(object_id(widget_id)))?;
        let mut out = Vec::new();
        let mut entries = Vec::new();
        collect(w, "Genesys.Gen.Widget_MenuData.Item", &mut entries);
        for e in entries {
            let action = match e.get(field_hash("Action")) {
                Some(Value::Str(s)) => s.clone(),
                _ => String::new(),
            };
            let active = !matches!(e.get(field_hash("Active")), Some(Value::Bool(false)));
            out.push((action, uint(e, h::NAME), active));
        }
        Ok(out)
    }
}
