//! The map screen (screen 899991 "MainMap", the MAP tab), reached the original's way: the pause menu's MAP item
//! (`SHOW_MAP` → "Suppress Prefetched Screens -> Run Map Screen" → "Run PauseMap Screen" → "Show Main Map" →
//! "Run PageRunner2 Context - Main Map" (`DEF_PAGE_REF` 899991) → "Run Map Screen" with `NavTabName` MAP → "Run
//! Map Options": widget group 899993 (`MapControls`, `TabActions`) with the map layouts (1695072) and the map
//! title; `TO_OPTIONS` opens group 1222205 (`Options`), whose list for an event is the `Event` menu data
//! (1222202): "SET DESTINATION" (`Map:OnSetWaypoint`) and "START RACE" (`Event:OnJumpToEvent:PLAY_EVENT`).
//! Leaving the options group with `PLAY_EVENT` plays its `ScreenTransition` 1453803 (sequence 1014321, the
//! event-loading card: `event_card.rs`), which then starts the race.
//!
//! What is drawn, from the screen's data: the black backdrop (`BlackBackground` 491565), the city map (the
//! main-map image of the `POI` widget's map layout 1222145, texture 2195ca), the event icons (point-of-interest
//! look 1222133 = map layout 816139, the green diamond), the selected event's card (look 1135809 = layout
//! 1481372: event name, type, difficulty), the cursor with the option list (look 1135304 = layout 1470763:
//! rings, then an element stack over `Widget.Options.Items`), the player's arrow (look 1222137 = layout
//! 371789), the title header (`MapOverlay` 1206401, "MAP"), the navigation bar and tickers (`Tabs` 1470689,
//! 1470704) and the filter selector (`Filters` 1470726, "RACES").
//!
//! Measured on the original (2026-10-10 capture, `ref/frontend-2026-10-10/`): the RACES filter shows the
//! current car's events (green) and the Most Wanted events (red: their own look, item type 9);
//! the selection sits at (836, 360) of the 1280×720 frame and the map turns so that world +X points down and
//! +Z left (94.4°), 6.29 m per pixel: fitted to eight icons of the capture against the events' start points
//! (research log 2026-10-10). The game draws the map with a 3D camera (`UIMapCameraConfig`: look-at
//! `Widget.MapManager.SelectedPosition`, distance 7000, field of view and tilt from bindings); here it is flat.
//!
//! Controls, as the original's `LiveMapManager` code runs them (decomp.md "Menu input and the map screen's
//! controls", tuning from `MainMapConfig`): the arrows, the left stick and the d-pad push a cursor (the camera's
//! look-at); a short push (≤ 0.4 s) flicks to the next item that way, a longer one moves the cursor freely at
//! 1500 m/s after 0.1 s and snaps to the nearest item on release; the cursor then glides onto the selection
//! (0.8 s). Accept opens an event's options, cancel goes back from them; cancel on the map and start (Esc, pad
//! Start) return to the game (`TO_GAME`). No zoom or pan on PC (the zoom states are the Vita's touch zoom).
//!
//! What the map lists, as the `LiveMapManager` code builds it (decomp.md "The world map's items and filters"):
//! one filter at a time, from the offline filter widget (group 1135638: RACES, JACK SPOTS, RECOMMENDS, SPEED
//! CAMERAS, BILLBOARDS; `DefaultFilterIndex` 1 = RACES), switched with `trigger_left` / `right` (Z / A, LT / RT:
//! `OnPrevFilter` / `OnNextFilter`, `FUN_00818550`, wrapping, then a snap from the cursor). Each filter's items
//! (`FUN_008181b0`): RACES the events (`FUN_008154f0`; a Most Wanted race, `c8f26580` set, is item type 9 with
//! its own red icon), SPEED CAMERAS the `TriggerData` kinds '4' / 'G', BILLBOARDS '2' / 'A' / 'D' / 'J' / 'M',
//! JACK SPOTS the locations '<' / 'B' / 'E' / 'K' / 'N'. The icon and the card of each item type are the looks
//! the screen script's `MapItemTypes` name (`LiveMapManager_ScreenScript` 1135185); cops in a pursuit are
//! `AICopPOI` (the arrow that turns blue on `Players.PlayerBase[#].IsCop`, `FUN_00808290`).
//!
//! Model decisions: the camera is orthographic with the measured scale and turn (cursor moves along the screen's
//! axes, no foreshortening); the tabs (Page Up / Down, LB / RB) are inert; a failed
//! left flick snaps instead of opening the Most Wanted list;
//! no completion state (`ShowEventComplete` false); the Most Wanted list panel, the player's header (name,
//! points), the speedwall entries, the map's road glow (layouts 1222163 / 1222558) and the tabs' icons are left
//! out; SET DESTINATION routes there (`gps.rs`); events whose game mode we do not run (speed runs, ambushes)
//! cannot be started.
//! RECOMMENDS lists nothing (Autolog). Collectables are every trigger of their kinds (the code keeps those in
//! the world database's `Collect3` table, not read); jack spots are item type 7 (the `FUN_006fcf90` profile
//! list that makes them type 14 is not modelled), whose icon shows once the spot is found
//! (`JackSpots.IsSwapSpotFound[#]`), and spots not found are left out of the selectable items; collectables'
//! options are their menus (`SpeedCamera`, `Jump`, ...) with only SET DESTINATION wired; the chosen filter is
//! not saved to the profile (`MapFilterIndex`).

use crate::platform::PollLock;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mw_assets::ui::{DrawOptions, PlacedLayout, Quad, UiAnim, UiBlend, UiScene, map_filter, map_item};
use mw_bundle::ResourceId;
use mw_formats::binding::BindValue;

use crate::game_text::{self, GameFonts, LoadedFont};
use crate::race::RaceSetup;
use crate::ui_quads::{AdditiveQuadMaterial, PoolNode, QuadNode, QuadPool, show_quads};

/// The map screen's bundle.
pub const MAP_SCREEN: &str = "UI/SCREENS2/899991.BNDL";
/// The `POI` widget's map layout holding the city image (2000 × 2412 map units).
const MAP_IMAGE_LAYOUT: u32 = 1_222_145;
const BLACK_BACKGROUND: u32 = 491_565;
const TITLE: u32 = 1_206_401;
const TABS: [u32; 2] = [1_470_689, 1_470_704];
const FILTERS: u32 = 1_470_726;
/// Point-of-interest map looks (keys 1135497, 1135808, 1135303, 1135501). The event icon is the fallback of
/// the item types' icons (`MapItemTypes`).
const EVENT_ICON: u32 = 816_139;
const EVENT_CARD: u32 = 1_481_372;
/// The widget group of the offline map's filters (`Filters`, `Widget_MapFilters` 1135664).
const FILTER_GROUP: u32 = 1_135_638;
const CURSOR: u32 = 1_470_763;
const PLAYER_ICON: u32 = 371_789;
/// The `Event` menu data (options for an event).
const EVENT_OPTIONS: u32 = 1_222_202;
/// The collectables' option menus: the map screen's `Widget_MenuData` whose `Name` is the item type's
/// `DataSource` (`SpeedCamera`, `Jump`, `SecurityGate`, `Jackspot`, `BodyShop`); each starts with SET DESTINATION
/// (`Map:OnSetWaypoint`).
const POINT_OPTIONS: [(u16, u32); 6] = [
    (map_item::SPEED_CAMERA, 1_135_362),
    (map_item::BILLBOARD, 1_135_373),
    (map_item::SECURITY_GATE, 1_135_374),
    (map_item::JACK_SPOT, 1_135_375),
    (map_item::JACK_SPOT_OWNED, 1_135_375),
    (map_item::BODY_SHOP, 1_524_675),
];
/// The option that sets the destination (`FUN_0080eb60`).
const SET_WAYPOINT: &str = "Map:OnSetWaypoint";
/// "MAP", "RACES", "You are not connected to the Autolog Servers".
const TITLE_TEXT: u32 = 565_900;
const FILTER_TEXT: u32 = 1_135_274;
const OFFLINE_TICKER: u32 = 119_887;
/// Ink of the navigation bar (its labels' colour in layout 1470689, linear): the unselected tabs' icons.
const TAB_INK: f32 = 0.005;
/// The offline race game mode (the one `race.rs` runs).
pub const RACE_MODE: u32 = 730_000;

/// World rectangle (x, z, width, depth) the main-map image covers: registered against the `UIMapGrid` tiles,
/// whose placement is data (research log 2026-10-10).
const MAP_WORLD: [f32; 4] = [-4000.0, -4581.0, 7990.0, 9987.0];
/// Where the selection sits in the 1280×720 frame, the map's turn (degrees, clockwise on screen: world +X
/// points this far from screen-right) and its scale (metres per reference pixel); measured (module doc).
const ANCHOR: Vec2 = Vec2::new(836.0, 360.0);
const TURN_DEG: f32 = 94.4;
const METRES_PER_PX: f32 = 6.29;
/// The main map's navigation tuning (`MainMapConfig` 1135320, `UI/UICONFIG.BNDL`), read by the
/// `LiveMapManager` code (research log 2026-10-14). Field offsets are the code's.
const MAIN_MAP_CONFIG: u64 = 0x0100_0000_0011_52d8;

/// `MainMapConfig` values the navigation uses; `Default` holds the shipped data (used if the record does not load).
#[derive(Clone, Copy, Debug)]
struct MapConfig {
    /// +0x10 / +0x20: the cursor's world box (max / min x, z).
    cursor_max: Vec2,
    cursor_min: Vec2,
    /// +0x58: snap radius on release (screen heights).
    snap_radius: f32,
    /// +0x5c: the cursor's glide to the selection (s).
    glide_time: f32,
    /// +0x60: input dead zone (length of (right − left, up − down)).
    dead_zone: f32,
    /// +0x64 / +0x68: flick cones (degrees), the narrow one first.
    flick_cone: [f32; 2],
    /// +0x6c: a release within this hold time (s) is a flick.
    flick_time: f32,
    /// +0x70: a flick needs at least this input length.
    flick_min: f32,
    /// +0x74: cursor speed (m/s at full input).
    speed: f32,
    /// +0x78: hold this long (s) before the cursor moves.
    move_delay: f32,
    /// +0x8c, +0x90, +0x94: how far off screen a flick target may be (screen widths / heights).
    flick_margin_y: f32,
    flick_margin_x: f32,
    flick_margin_y2: f32,
}

impl Default for MapConfig {
    fn default() -> Self {
        Self {
            cursor_max: Vec2::new(2500.0, 5040.0),
            cursor_min: Vec2::new(-3500.0, -4319.0),
            snap_radius: 0.4,
            glide_time: 0.8,
            dead_zone: 0.4,
            flick_cone: [30.0, 45.0],
            flick_time: 0.4,
            flick_min: 0.1,
            speed: 1500.0,
            move_delay: 0.1,
            flick_margin_y: 1.0,
            flick_margin_x: 1.0,
            flick_margin_y2: 0.0,
        }
    }
}

impl MapConfig {
    fn load(store: &mw_assets::AssetStore) -> Self {
        use mw_formats::genesys::Value;
        let mut c = Self::default();
        let obj = match store.genesys_object(ResourceId(MAIN_MAP_CONFIG)) {
            Ok(o) => o,
            Err(e) => {
                warn!("map screen: MainMapConfig: {e}; using the shipped values");
                return c;
            }
        };
        let f = |h: u32| match obj.get(h) {
            Some(Value::Float(v)) => Some(*v),
            _ => None,
        };
        let v = |h: u32| match obj.get(h) {
            Some(Value::Vector(v)) => Some(Vec2::new(v[0], v[2])),
            _ => None,
        };
        let set = |dst: &mut f32, h: u32| {
            if let Some(x) = f(h) {
                *dst = x;
            }
        };
        if let (Some(max), Some(min)) = (v(0xf0fc_3707), v(0x104f_8c6f)) {
            c.cursor_max = max;
            c.cursor_min = min;
        }
        set(&mut c.snap_radius, 0x33aa_95af);
        set(&mut c.glide_time, 0xd7ea_64ad);
        set(&mut c.dead_zone, 0xa8ab_14f7);
        set(&mut c.flick_cone[0], 0x20df_81d4);
        set(&mut c.flick_cone[1], 0xd1e7_9229);
        set(&mut c.flick_time, 0xba1a_0da6);
        set(&mut c.flick_min, 0x2e0e_62f3);
        set(&mut c.speed, 0x528e_1731);
        set(&mut c.move_delay, 0xd1a5_d10d);
        set(&mut c.flick_margin_y, 0x3fa7_9c23);
        set(&mut c.flick_margin_x, 0x3dfe_660a);
        set(&mut c.flick_margin_y2, 0x3c5e_6456);
        c
    }
}

pub struct MapScreenPlugin {
    pub enabled: bool,
    /// Open at startup (`--show-map`, `--show-pause --pause-map`).
    pub open: bool,
    /// `--map-select <event id>`: select this event when the map opens.
    pub select: Option<u32>,
    /// `--map-options`: open the selection's options.
    pub options: bool,
    /// `--map-start <event id>`: select the event and choose START RACE.
    pub start: Option<u32>,
    /// `--map-waypoint <event id>`: select the event and choose SET DESTINATION (the map stays open with `open`).
    pub waypoint: Option<u32>,
}

impl Plugin for MapScreenPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MapScreen {
            enabled: self.enabled,
            open_at_start: self.open || self.start.is_some() || self.waypoint.is_some(),
            select: self.start.or(self.waypoint).or(self.select),
            options_at_start: self.options || self.start.is_some() || self.waypoint.is_some(),
            start_at_start: self.start.is_some(),
            waypoint_at_start: self.waypoint.is_some(),
            close_after_waypoint: !self.open,
            ..default()
        })
        .add_systems(Startup, start_load)
        .add_systems(Update, (poll_load, open_at_start, navigate, draw).chain().before(crate::pause::navigate));
    }
}

/// An event on the map.
#[derive(Clone)]
struct MapItem {
    setup: Arc<RaceSetup>,
    /// Start point (world x, z).
    pos: Vec2,
    /// The event's type name ("CIRCUIT RACE", "MOST WANTED").
    type_name: String,
    /// Route length (m): start to the last gate, all laps.
    length: f32,
    /// Vehicle list ids that may enter (empty: any).
    vehicles: Vec<u32>,
    /// `AllowedVehicle.difficulty` per vehicle (1 easy, 2 medium, 3 hard).
    difficulty: Vec<(u32, i64)>,
    most_wanted: bool,
    /// We run its game mode.
    playable: bool,
}

impl MapItem {
    /// `MapItemTypes[9]` for an offline event with `c8f26580` (+0xe9) set, else `[1]` (`FUN_008154f0`).
    fn item_type(&self) -> u16 {
        if self.setup.event.first_place_only { map_item::MOST_WANTED_EVENT } else { map_item::EVENT }
    }
}

/// A collectable on the map: a `TriggerData` record of a kind a filter lists (`FUN_00817500`).
#[derive(Clone, Debug)]
struct MapPoint {
    item_type: u16,
    /// The trigger's id.
    id: u32,
    pos: Vec2,
    /// The trigger's position (the destination SET DESTINATION routes to, `FUN_008091d0`).
    world: Vec3,
}

/// An entry of the current filter's list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Entry {
    /// An event (index into `items`).
    Event(usize),
    /// A collectable (index into `points`).
    Point(usize),
}

#[derive(Clone)]
struct OptionItem {
    action: String,
    label: String,
}

/// What the map has selected (`LiveMapManager` +0x2610: one of its items, or none while the cursor is free).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sel {
    /// The local player's item (`MapItemType` 0, `LocalPlayer`).
    Player,
    /// An entry of `shown`.
    Item(usize),
}

/// The cursor's navigation state (`LiveMapManager` fields, research log 2026-10-14).
#[derive(Default)]
struct Nav {
    /// +0x2660: the cursor when the input left the dead zone (a flick starts from here).
    start: Vec2,
    /// +0x26a4 / +0x26a8 / +0x26ac: last frame's input (right − left, up − down) and its squared length.
    prev: Vec2,
    prev_len2: f32,
    /// +0x26b0: time the input has been out of the dead zone.
    held: f32,
    /// +0x26b4: the glide's clock.
    glide: f32,
    /// +0x26d6: the cursor was moved by hand and has not snapped to an item since.
    moving: bool,
}

#[derive(Resource, Default)]
pub struct MapScreen {
    pub enabled: bool,
    pub open: bool,
    /// Closed this frame (other menus must not take the same key).
    pub just_closed: bool,
    items: Vec<MapItem>,
    /// Every collectable a filter can list.
    points: Vec<MapPoint>,
    /// The current filter's list (`FUN_008181b0`).
    shown: Vec<Entry>,
    /// The filters (`FilterType`, name) and the current one (`LiveMapManager` +0x26c4, 1-based there).
    filters: Vec<(u16, String)>,
    filter: usize,
    /// The driven car (`VehicleList` id), for the RACES filter.
    vehicle: Option<u32>,
    /// Jack spot locations found (`JackSpots.IsSwapSpotFound`).
    found: std::collections::BTreeSet<u32>,
    /// Cops (position, facing) while a pursuit runs (`FUN_00808290`: `AICopPOI`).
    cops: Vec<(Vec2, Vec3)>,
    /// Icon / card layout per item type (`MapItemTypes` → the screen's point-of-interest looks), the cops' icon.
    icons: HashMap<u16, u32>,
    cards: HashMap<u16, u32>,
    cop_icon: Option<u32>,
    /// Every map look of the screen's POI widget by key: the map manager's POI list is drawn with them (`poi.rs`).
    poi_looks: HashMap<u32, u32>,
    /// `CollectablesName`: the collectables' names by item type.
    point_names: HashMap<u16, String>,
    selected: Option<Sel>,
    /// The options list is open, with this row selected.
    options: Option<usize>,
    /// The open list's options, and every item type's (`MapItemTypes[t].DataSource`'s menu).
    option_items: Vec<OptionItem>,
    menus: HashMap<u16, Vec<OptionItem>>,
    /// The cursor (world x, z; +0x2630): `Widget.MapManager.SelectedPosition`, the map camera's look-at.
    cursor: Vec2,
    nav: Nav,
    config: MapConfig,
    player: Vec2,
    strings: HashMap<&'static str, String>,
    /// The navigation tabs (`Widget_NavigationTabs.Tabs`): icon texture (16 hex digits) and name.
    tabs: Vec<(String, String)>,
    job: Option<Arc<Mutex<Option<Loaded>>>>,
    look: Option<Look>,
    /// Re-read the player's position and car (just opened).
    refresh: bool,
    open_at_start: bool,
    select: Option<u32>,
    options_at_start: bool,
    start_at_start: bool,
    waypoint_at_start: bool,
    close_after_waypoint: bool,
}

impl MapScreen {
    /// Open the map (from the pause menu): the player selected, the clock stopped (the menu's pause reason, kept
    /// from the pause menu or taken here; released when the map closes back to the game).
    pub fn open(&mut self, reasons: &mut crate::pause::PauseReasons, time: &mut Time<Virtual>) {
        if self.look.is_none() {
            return;
        }
        self.open = true;
        self.selected = Some(Sel::Player);
        self.options = None;
        self.nav = Nav::default();
        self.refresh = true;
        reasons.set(crate::pause::PauseReason::Menu, true, time);
        if let Some(l) = self.look.as_mut() {
            l.anim.reset();
        }
    }

    pub fn ready(&self) -> bool {
        self.look.is_some()
    }

    /// Close; returns whether the clock is to be restarted.
    fn close(&mut self) {
        self.open = false;
        self.options = None;
        self.just_closed = true;
    }

    /// The selected event.
    fn selected_item(&self) -> Option<&MapItem> {
        match self.selected_entry()? {
            Entry::Event(i) => self.items.get(i),
            Entry::Point(_) => None,
        }
    }

    fn selected_entry(&self) -> Option<Entry> {
        match self.selected {
            Some(Sel::Item(s)) => self.shown.get(s).copied(),
            _ => None,
        }
    }

    fn entry_pos(&self, e: Entry) -> Vec2 {
        match e {
            Entry::Event(i) => self.items[i].pos,
            Entry::Point(i) => self.points[i].pos,
        }
    }

    /// Open the selected item's options (`TO_OPTIONS`), if its item type has a menu.
    fn open_options(&mut self) -> bool {
        let Some(items) = self.selected_entry().and_then(|e| self.menus.get(&self.entry_type(e))) else {
            return false;
        };
        self.option_items = items.clone();
        self.options = Some(0);
        true
    }

    /// `Map:OnSetWaypoint` (`FUN_0080eb60`): an event routes to its start (`FUN_00796ff0`), a collectable to its
    /// trigger (`FUN_008091d0`).
    fn waypoint(&self) -> Option<crate::gps::SetDestination> {
        Some(match self.selected_entry()? {
            Entry::Event(i) => {
                let e = &self.items[i].setup.event;
                crate::gps::SetDestination { position: Vec3::from(e.spawn?.position), event: Some(e.id) }
            }
            Entry::Point(i) => crate::gps::SetDestination { position: self.points[i].world, event: None },
        })
    }

    fn entry_type(&self, e: Entry) -> u16 {
        match e {
            Entry::Event(i) => self.items[i].item_type(),
            Entry::Point(i) => self.points[i].item_type,
        }
    }

    fn sel_pos(&self, sel: Sel) -> Vec2 {
        match sel {
            Sel::Player => self.player,
            Sel::Item(s) => self.entry_pos(self.shown[s]),
        }
    }

    /// Every selectable item: the player and the entries shown.
    fn candidates(&self) -> impl Iterator<Item = (Sel, Vec2)> + '_ {
        std::iter::once((Sel::Player, self.player))
            .chain(self.shown.iter().enumerate().map(|(s, &e)| (Sel::Item(s), self.entry_pos(e))))
    }

    /// The current filter's type (`FilterType`); RACES before the filters are loaded.
    fn filter_type(&self) -> u16 {
        self.filters.get(self.filter).map_or(map_filter::RACES, |f| f.0)
    }

    /// Rebuild the list for the current filter (`FUN_008181b0`).
    fn refresh_list(&mut self) {
        self.shown = match self.filter_type() {
            map_filter::RACES => shown_items(&self.items, self.vehicle).into_iter().map(Entry::Event).collect(),
            t => self
                .points
                .iter()
                .enumerate()
                .filter(|(_, p)| point_filter(p.item_type) == Some(t))
                // MODEL: a jack spot's icon is visible once found; hidden spots are not selectable.
                .filter(|(_, p)| p.item_type != map_item::JACK_SPOT || self.found.contains(&p.id))
                .map(|(i, _)| Entry::Point(i))
                .collect(),
        };
    }

    /// `OnPrevFilter` / `OnNextFilter` (`FUN_00818550`): the next filter, wrapping; the list for it; a snap
    /// from the cursor.
    fn step_filter(&mut self, forward: bool) {
        let n = self.filters.len();
        if n == 0 {
            return;
        }
        self.filter = if forward { (self.filter + 1) % n } else { (self.filter + n - 1) % n };
        self.refresh_list();
        self.selected = snap(self);
        self.nav.glide = 0.0;
        self.nav.moving = self.selected.is_none();
        if let Some(l) = self.look.as_mut() {
            l.anim.reset();
        }
    }
}

/// The filter that lists a collectable item type (`FUN_008181b0`).
fn point_filter(item_type: u16) -> Option<u16> {
    Some(match item_type {
        map_item::SPEED_CAMERA => map_filter::SPEED_CAMERAS,
        map_item::BILLBOARD => map_filter::BILLBOARDS,
        map_item::SECURITY_GATE => map_filter::SECURITY_GATES,
        map_item::JACK_SPOT | map_item::JACK_SPOT_OWNED => map_filter::JACK_SPOTS,
        map_item::BODY_SHOP => map_filter::BODY_SHOPS,
        _ => return None,
    })
}

/// The map item type of a `TriggerData` kind (`FUN_008181b0` → `FUN_00817500`): speed cameras '4' / 'G',
/// billboards '2' / 'A' / 'D' / 'J' / 'M', security gates '0', jack spot locations '<' and the packs' 'B' /
/// 'E' / 'K' / 'N' (item types 15–18 there, whose looks are the packs'; ours: the base look), body shops 6.
fn point_type(kind: u8) -> Option<u16> {
    Some(match kind {
        b'4' | b'G' => map_item::SPEED_CAMERA,
        b'2' | b'A' | b'D' | b'J' | b'M' => map_item::BILLBOARD,
        b'0' => map_item::SECURITY_GATE,
        b'<' | b'B' | b'E' | b'K' | b'N' => map_item::JACK_SPOT,
        6 => map_item::BODY_SHOP,
        _ => return None,
    })
}

struct Loaded {
    config: MapConfig,
    items: Vec<MapItem>,
    points: Vec<MapPoint>,
    filters: Vec<(u16, String)>,
    icons: HashMap<u16, u32>,
    cards: HashMap<u16, u32>,
    cop_icon: Option<u32>,
    poi_looks: HashMap<u32, u32>,
    point_names: HashMap<u16, String>,
    menus: HashMap<u16, Vec<OptionItem>>,
    strings: HashMap<&'static str, String>,
    tabs: Vec<(String, String)>,
    scene: UiScene,
    map_texture: Option<ResourceId>,
    images: Vec<(ResourceId, Image)>,
    fonts: Vec<(u32, mw_formats::Font)>,
}

struct Look {
    scene: UiScene,
    /// Scene index of each layout above.
    slots: HashMap<u32, usize>,
    map_texture: Option<ResourceId>,
    textures: HashMap<ResourceId, (Handle<Image>, Vec2)>,
    root: Entity,
    pool: QuadPool,
    anim: UiAnim,
}

fn start_load(mut map: ResMut<MapScreen>, game: Res<crate::Game>) {
    if !map.enabled {
        return;
    }
    let job = Arc::new(Mutex::new(None));
    let (out, store) = (job.clone(), game.store.clone());
    crate::platform::thread::Builder::new()
        .name("mw-map-screen".into())
        .spawn(move || {
            let t = crate::platform::Instant::now();
            let Ok(lang) = store.language(mw_assets::DEFAULT_LANGUAGE) else { return };
            let text = |id: u32| lang.get(id).unwrap_or_default().to_string();
            let items = load_items(&store, &lang);
            let config = MapConfig::load(&store);
            let menu = |id: u32| -> Vec<OptionItem> {
                store
                    .menu_items(MAP_SCREEN, id)
                    .map_err(|e| warn!("map screen: options {id}: {e}"))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(action, name, _)| OptionItem { action, label: text(name).to_uppercase() })
                    .collect()
            };
            let mut menus: HashMap<u16, Vec<OptionItem>> = POINT_OPTIONS.iter().map(|&(t, id)| (t, menu(id))).collect();
            let events = menu(EVENT_OPTIONS);
            menus.insert(map_item::EVENT, events.clone());
            menus.insert(map_item::MOST_WANTED_EVENT, events);
            // The keyboard's button prompts (the original's default keys for the actions: MENU_ACCEPT Enter,
            // DRIVE Esc, the filters Z / A, the tabs Page Up / Down; decomp.md "Input").
            let key = |k: &str| store.keyboard_button_prompt(k).map(text).unwrap_or_default();
            let strings: HashMap<&'static str, String> = [
                ("title", text(TITLE_TEXT)),
                ("filter", text(FILTER_TEXT)),
                ("ticker", text(OFFLINE_TICKER)),
                ("accept", key("KEY_RETURN")),
                ("escape", key("KEY_ESCAPE")),
                ("z", key("KEY_Z")),
                ("a", key("KEY_A")),
                ("prior", key("KEY_PRIOR")),
                ("next", key("KEY_NEXT")),
            ]
            .into_iter()
            .collect();
            // The item types' icons and cards (`MapItemTypes` keys → the screen's point-of-interest looks), the
            // filters and the collectables.
            let manager = store.map_manager_config().map_err(|e| warn!("map screen: map item types: {e}")).ok();
            let pois =
                store.ui_poi_visuals(MAP_SCREEN).map_err(|e| warn!("map screen: looks: {e}")).unwrap_or_default();
            let look = |key: u32| pois.iter().find(|p| p.key == key).and_then(|p| p.map.as_ref()).map(|m| m.layout.id);
            let mut icons = HashMap::new();
            let mut cards = HashMap::new();
            let mut point_names = HashMap::new();
            let mut cop_icon = None;
            if let Some(m) = &manager {
                for t in &m.item_types {
                    if let Some(l) = look(t.icon) {
                        icons.insert(t.item_type, l);
                    }
                    if let Some(l) = look(t.card) {
                        cards.insert(t.item_type, l);
                    }
                }
                for &(t, name) in &m.collectable_names {
                    point_names.insert(t, text(name).to_uppercase());
                }
                cop_icon = look(m.ai_cop_poi);
            }
            let poi_looks: HashMap<u32, u32> =
                pois.iter().filter_map(|p| Some((p.key, p.map.as_ref()?.layout.id))).collect();
            // Events keep the card measured on the capture (look 1135809; research log 2026-10-10).
            cards.insert(map_item::EVENT, EVENT_CARD);
            cards.insert(map_item::MOST_WANTED_EVENT, EVENT_CARD);
            let filters: Vec<(u16, String)> = store
                .map_filters(MAP_SCREEN, FILTER_GROUP)
                .map_err(|e| warn!("map screen: filters: {e}"))
                .unwrap_or_default()
                .into_iter()
                .map(|f| (f.filter_type, text(f.name).to_uppercase()))
                .collect();
            let points: Vec<MapPoint> = store
                .trigger_data()
                .map(|td| {
                    td.triggers
                        .iter()
                        .filter_map(|t| {
                            Some(MapPoint {
                                item_type: point_type(t.kind)?,
                                id: t.id,
                                pos: Vec2::new(t.position[0], t.position[2]),
                                world: Vec3::from(t.position),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            // The layouts drawn, in draw order.
            let mut ids = vec![BLACK_BACKGROUND, EVENT_ICON];
            let add = |ids: &mut Vec<u32>, id: u32| {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            };
            for &l in icons.values().chain(cop_icon.iter()).chain(poi_looks.values()) {
                add(&mut ids, l);
            }
            add(&mut ids, PLAYER_ICON);
            for &l in cards.values() {
                add(&mut ids, l);
            }
            for id in [CURSOR, TITLE, TABS[0], TABS[1], FILTERS] {
                add(&mut ids, id);
            }
            let mut layouts = Vec::new();
            for id in ids {
                match store.ui_layout(mw_formats::ui::object_id(id)) {
                    Ok(layout) => layouts.push(PlacedLayout {
                        widget_index: 0,
                        widget: String::new(),
                        widget_type: String::new(),
                        layout,
                        instance: None,
                    }),
                    Err(e) => warn!("map screen: layout {id}: {e}"),
                }
            }
            let map_texture = store
                .ui_layout(mw_formats::ui::object_id(MAP_IMAGE_LAYOUT))
                .ok()
                .and_then(|l| l.elements.iter().find_map(|e| e.texture));
            let mut scene = store.ui_scene(layouts, &lang);
            scene.letterbox = true;
            let nav = store.navigation_tabs(MAP_SCREEN).map_err(|e| warn!("map screen: tabs: {e}")).unwrap_or_default();
            let tabs: Vec<(String, String)> = nav
                .iter()
                .map(|t| {
                    let image = t.image.map(|i| format!("{:016x}", i.0)).unwrap_or_default();
                    (image, lang.get(t.name).unwrap_or_default().to_uppercase())
                })
                .collect();
            let mut images: Vec<(ResourceId, Image)> = scene
                .textures()
                .into_iter()
                .chain(map_texture)
                .chain(nav.iter().filter_map(|t| t.image))
                .filter_map(|id| Some((id, game_text::decode_texture(&store, id)?)))
                .collect();
            images.dedup_by_key(|(id, _)| *id);
            let fonts = scene.fonts.iter().map(|(&n, f)| (n, f.clone())).collect();
            info!(
                "map screen: {} events, {} collectables, {} filters, {} item looks, {} textures in {:.0?}",
                items.len(),
                points.len(),
                filters.len(),
                icons.len(),
                images.len(),
                t.elapsed()
            );
            *out.lock().unwrap() = Some(Loaded {
                config,
                items,
                points,
                filters,
                icons,
                cards,
                cop_icon,
                poi_looks,
                point_names,
                menus,
                strings,
                tabs,
                scene,
                map_texture,
                images,
                fonts,
            });
        })
        .expect("spawn map-screen loader");
    map.job = Some(job);
}

/// Every offline event with a start point.
fn load_items(store: &mw_assets::AssetStore, lang: &mw_formats::Language) -> Vec<MapItem> {
    let Ok(events) = store.events() else { return Vec::new() };
    events
        .events
        .iter()
        .filter_map(|e| {
            let start = e.spawn?.position;
            let name = lang.get(e.name).map_or_else(|| format!("EVENT {}", e.id), str::to_uppercase);
            let type_name = lang.get(e.type_name).map(str::to_uppercase).unwrap_or_else(|| e.kind.label());
            let mut length = 0.0;
            let mut last = Vec2::new(start[0], start[2]);
            let laps = e.laps.max(1) as usize;
            for g in e.checkpoints.iter().cycle().take(e.checkpoints.len() * laps) {
                let p = Vec2::new(g.position[0], g.position[2]);
                length += p.distance(last);
                last = p;
            }
            let game_mode = events.game_mode(e.game_mode).copied();
            Some(MapItem {
                pos: Vec2::new(start[0], start[2]),
                type_name,
                length,
                vehicles: e.allowed_vehicles.iter().map(|v| v.vehicle).collect(),
                difficulty: e.allowed_vehicles.iter().map(|v| (v.vehicle, v.difficulty)).collect(),
                most_wanted: matches!(e.kind, mw_assets::events::EventKind::MostWanted),
                playable: e.game_mode == RACE_MODE && !e.checkpoints.is_empty(),
                setup: Arc::new(RaceSetup { event: e.clone(), game_mode, name, test: false }),
            })
        })
        .collect()
}

fn poll_load(
    mut commands: Commands,
    mut map: ResMut<MapScreen>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<GameFonts>,
) {
    let Some(loaded) = map.job.as_ref().and_then(|j| j.poll_lock().and_then(|mut s| s.take())) else { return };
    map.job = None;
    let textures: HashMap<ResourceId, (Handle<Image>, Vec2)> = loaded
        .images
        .into_iter()
        .map(|(id, img)| {
            let size = img.size().as_vec2();
            (id, (images.add(img), size))
        })
        .collect();
    for (number, font) in loaded.fonts {
        if let (false, Some(atlas)) = (fonts.fonts.contains_key(&number), font.texture.and_then(|t| textures.get(&t))) {
            fonts.fonts.insert(number, LoadedFont { font: Arc::new(font), atlas: atlas.0.clone() });
        }
    }
    let node = Node {
        position_type: PositionType::Absolute,
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        ..default()
    };
    let root = commands.spawn((node, Pickable::IGNORE, GlobalZIndex(16))).id();
    let slots = loaded.scene.layouts.iter().enumerate().map(|(i, l)| (l.layout.id, i)).collect();
    map.items = loaded.items;
    map.points = loaded.points;
    map.filters = loaded.filters;
    // `DefaultFilterIndex` 1 (1-based): the first, RACES.
    map.filter = 0;
    map.icons = loaded.icons;
    map.cards = loaded.cards;
    map.cop_icon = loaded.cop_icon;
    map.poi_looks = loaded.poi_looks;
    map.point_names = loaded.point_names;
    map.config = loaded.config;
    if std::env::var_os("MW_MAP_LOG").is_some() {
        info!("map screen: {:?}", map.config);
    }
    map.menus = loaded.menus;
    map.strings = loaded.strings;
    map.tabs = loaded.tabs;
    map.look = Some(Look {
        scene: loaded.scene,
        slots,
        map_texture: loaded.map_texture,
        textures,
        root,
        pool: QuadPool::default(),
        anim: UiAnim::default(),
    });
}

/// The events the RACES filter shows for the driven car: its own events and the Most Wanted races.
fn shown_items(items: &[MapItem], vehicle: Option<u32>) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, i)| i.most_wanted || vehicle.is_some_and(|v| i.vehicles.contains(&v)))
        .map(|(k, _)| k)
        .collect()
}

/// The player's position and the events shown for the driven car, when the map opens; then
/// `--show-map` / `--map-select` / `--map-options` / `--map-start` once the map and the player's car are in.
#[allow(clippy::too_many_arguments)]
fn open_at_start(
    mut map: ResMut<MapScreen>,
    mut time: ResMut<Time<Virtual>>,
    mut reasons: ResMut<crate::pause::PauseReasons>,
    car: Query<&GlobalTransform, (With<crate::drive::Car>, With<crate::drive::Player>)>,
    garage: Res<crate::garage::Garage>,
    collected: Option<Res<crate::frontend::Collected>>,
    pursuit: Option<Res<crate::police::PursuitHud>>,
    mut play: MessageWriter<crate::event_card::PlayEvent>,
    mut waypoint: MessageWriter<crate::gps::SetDestination>,
) {
    // What the lists depend on: the driven car, the jack spots found, the cops of a pursuit.
    let sync = |map: &mut MapScreen| {
        map.vehicle = garage.current.as_ref().and_then(|c| crate::frontend::vehicle_id(&c.bundle));
        map.found = collected.as_ref().map(|c| c.jack_spots.clone()).unwrap_or_default();
        map.cops = pursuit
            .as_ref()
            .filter(|p| p.in_chase || p.in_cooldown)
            .map(|p| p.cops.iter().map(|&(pos, f)| (Vec2::new(pos.x, pos.z), f)).collect())
            .unwrap_or_default();
        map.refresh_list();
    };
    if map.refresh
        && let Ok(xf) = car.single()
    {
        map.refresh = false;
        let p = xf.translation();
        map.player = Vec2::new(p.x, p.z);
        map.cursor = map.player;
        sync(&mut map);
    }
    if !map.open_at_start || map.look.is_none() {
        return;
    }
    let Ok(xf) = car.single() else { return };
    map.open_at_start = false;
    let p = xf.translation();
    map.player = Vec2::new(p.x, p.z);
    sync(&mut map);
    map.open(&mut reasons, &mut time);
    map.refresh = false;
    map.cursor = map.player;
    if let Some(id) = map.select {
        // A selected event that the car's list does not hold is shown too (headless checks).
        let k = match map.items.iter().position(|i| i.setup.event.id == id) {
            Some(k) => k,
            None => {
                warn!("map screen: no event {id}");
                return;
            }
        };
        let s = map.shown.iter().position(|&e| e == Entry::Event(k)).unwrap_or_else(|| {
            map.shown.push(Entry::Event(k));
            map.shown.len() - 1
        });
        map.selected = Some(Sel::Item(s));
        map.cursor = map.items[k].pos;
    }
    if map.options_at_start {
        map.open_options();
    }
    if map.waypoint_at_start {
        if let Some(row) = map.option_items.iter().position(|o| o.action == SET_WAYPOINT) {
            map.options = Some(row);
        }
        if let Some(set) = map.waypoint() {
            info!("map screen: SET DESTINATION {}", set.position);
            waypoint.write(set);
        }
        if map.close_after_waypoint {
            map.close();
            reasons.set(crate::pause::PauseReason::Menu, false, &mut time);
        }
    }
    if map.start_at_start
        && let Some(item) = map.selected_item().cloned()
    {
        map.close();
        play.write(crate::event_card::PlayEvent(item.setup.clone()));
    }
}

/// Screen position (reference pixels) of a world point with the camera looking at `centre`.
fn to_screen(world: Vec2, centre: Vec2) -> Vec2 {
    let d = (world - centre) / METRES_PER_PX;
    let (s, c) = TURN_DEG.to_radians().sin_cos();
    ANCHOR + Vec2::new(c * d.x - s * d.y, s * d.x + c * d.y)
}

/// World direction of a screen offset (reference pixels): the inverse of `to_screen`'s turn and scale.
fn screen_to_world(d: Vec2) -> Vec2 {
    let (s, c) = TURN_DEG.to_radians().sin_cos();
    Vec2::new(c * d.x + s * d.y, -s * d.x + c * d.y) * METRES_PER_PX
}

/// The game's projected screen position (`FUN_00803fc0`: x right and y up, 0..1 across the frame).
fn projected(world: Vec2, centre: Vec2) -> Vec2 {
    let p = to_screen(world, centre);
    Vec2::new(p.x / 1280.0, 1.0 - p.y / 720.0)
}

/// The frame's width over its height (the view's aspect, view +0x2f3c8).
const ASPECT: f32 = 1280.0 / 720.0;

/// The ease the cursor glides with (`FUN_00630980`): linear with slope 1.25 between 0.2 and 0.8 of the way,
/// quadratic ends, symmetric about (0.5, 0.5).
fn glide_ease(x: f32) -> f32 {
    if x < 0.5 {
        return 1.0 - glide_ease(1.0 - x);
    }
    if x > 0.8 { 0.875 + 0.625 * (6.25 * (1.0 - x) + 1.0) * (x - 0.8) } else { 1.25 * (x - 0.2) + 0.125 }
}

/// On release: the item nearest the cursor on screen within the snap radius (`FUN_00813e60`).
fn snap(map: &MapScreen) -> Option<Sel> {
    let c = projected(map.cursor, map.cursor);
    let r2 = map.config.snap_radius * map.config.snap_radius;
    map.candidates()
        .map(|(sel, pos)| {
            let p = projected(pos, map.cursor);
            (sel, (ASPECT * (c.x - p.x)).powi(2) + (c.y - p.y).powi(2))
        })
        .filter(|&(_, d2)| d2 <= r2)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(sel, _)| sel)
}

/// A flick (a short push): the item in the input's direction from where the push started, within the 30° cone,
/// else the 45° one, on or near the screen; of those the nearest in the world (`FUN_00814f60`).
fn flick(map: &MapScreen, from: Vec2, dir: Vec2) -> Option<Sel> {
    let c = &map.config;
    let p0 = projected(from, map.cursor);
    let d = dir.normalize_or_zero();
    let in_window = |p: Vec2| {
        let my = c.flick_margin_y2 + c.flick_margin_y;
        (-c.flick_margin_x..=c.flick_margin_x + 1.0).contains(&p.x) && (-my..=my + 1.0).contains(&p.y)
    };
    for cone in c.flick_cone {
        let cos = cone.to_radians().cos();
        let best = map
            .candidates()
            .filter(|&(sel, _)| Some(sel) != map.selected)
            .filter(|&(_, pos)| {
                let p = projected(pos, map.cursor);
                in_window(p) && Vec2::new(ASPECT * (p.x - p0.x), p.y - p0.y).normalize_or_zero().dot(d) > cos
            })
            .min_by(|a, b| a.1.distance_squared(from).total_cmp(&b.1.distance_squared(from)));
        if let Some((sel, _)) = best {
            return Some(sel);
        }
    }
    None
}

/// One frame of the map controls (`FUN_008170d0` with the `OnMenuLeft` / `Right` / `Up` / `Down` values of
/// `FUN_00803ce0`, then the glide `FUN_00807a60`). `input` = (right − left, up − down).
fn nav_update(map: &mut MapScreen, input: Vec2, dt: f32) {
    let c = map.config;
    let dz2 = c.dead_zone * c.dead_zone;
    let len2 = input.length_squared();
    if len2 <= dz2 {
        if dz2 < map.nav.prev_len2 {
            // Released: a short push flicks to the next item that way, anything else snaps to the nearest.
            let is_flick = map.nav.prev_len2 > c.flick_min * c.flick_min && map.nav.held <= c.flick_time;
            let picked = is_flick.then(|| flick(map, map.nav.start, map.nav.prev)).flatten().or_else(|| snap(map));
            // A flick left with nothing there opens the Most Wanted list (`TO_MWL`; that panel is not built).
            if let Some(sel) = picked {
                map.selected = Some(sel);
                map.nav.glide = 0.0;
                map.nav.moving = false;
            }
        }
        map.nav.held = 0.0;
    } else {
        if map.nav.prev_len2 <= dz2 {
            map.nav.start = map.cursor;
        }
        map.nav.held += dt;
        if map.nav.held > c.move_delay {
            // Screen direction (x right, y up) at `speed` m/s per unit of input, kept in the cursor's box.
            let step = screen_to_world(Vec2::new(input.x, -input.y)) / METRES_PER_PX * c.speed * dt;
            map.cursor = (map.cursor + step).clamp(c.cursor_min, c.cursor_max);
            map.selected = None;
            map.nav.moving = true;
        }
    }
    map.nav.prev = input;
    map.nav.prev_len2 = len2;
    // The glide onto the selection.
    if !map.nav.moving
        && let Some(sel) = map.selected
    {
        let target = map.sel_pos(sel);
        map.nav.glide += dt;
        if map.nav.glide < c.glide_time {
            map.cursor += glide_ease(map.nav.glide / c.glide_time) * (target - map.cursor);
        } else {
            map.cursor = target;
            map.nav.glide = c.glide_time;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn navigate(
    mut map: ResMut<MapScreen>,
    menu: Res<crate::input::MenuInput>,
    mut time: ResMut<Time<Virtual>>,
    mut reasons: ResMut<crate::pause::PauseReasons>,
    real: Res<Time<Real>>,
    mut play: MessageWriter<crate::event_card::PlayEvent>,
    mut waypoint: MessageWriter<crate::gps::SetDestination>,
) {
    use crate::input::MenuAction as A;
    map.just_closed = false;
    if !map.open {
        return;
    }
    let dt = real.delta_secs().min(0.1);
    // The navigation tabs' `Drive` (start) and the map group's `Back_MapScreen` (cancel) both exit `TO_GAME`;
    // in the options group cancel goes `BACK` to the map (899991's widget groups 1607001, 899993, 1222205).
    let drive = menu.pressed(A::Start) || (map.options.is_none() && menu.pressed(A::Cancel));
    if drive {
        map.close();
        reasons.set(crate::pause::PauseReason::Menu, false, &mut time);
        return;
    }
    if let Some(row) = map.options {
        let n = map.option_items.len().max(1);
        // `SubMenuUp` / `Down` (repeating) → `OnPrevious` / `OnNext`, bounded by `CanSelectPrevious` / `Next`.
        if menu.pressed(A::Cancel) {
            map.options = None;
        } else if menu.fired(A::Up) {
            map.options = Some(row.saturating_sub(1));
        } else if menu.fired(A::Down) {
            map.options = Some((row + 1).min(n - 1));
        } else if menu.pressed(A::Accept) {
            let action = map.option_items.get(row).map(|o| o.action.clone()).unwrap_or_default();
            let playable = map.selected_item().is_some_and(|i| i.playable);
            // No exit token follows `Map:OnSetWaypoint`: the options stay open.
            if action == SET_WAYPOINT
                && let Some(set) = map.waypoint()
            {
                info!("map screen: SET DESTINATION {}", set.position);
                waypoint.write(set);
            } else if action.ends_with(":PLAY_EVENT") && playable {
                let setup = map.selected_item().map(|i| i.setup.clone());
                // The event card restarts the clock when the race starts.
                map.close();
                if let Some(setup) = setup {
                    info!("map screen: START RACE {} ({})", setup.name, setup.event.id);
                    play.write(crate::event_card::PlayEvent(setup));
                }
            }
        }
        // `MapControls` is inactive: its analogue actions read nothing.
        nav_update(&mut map, Vec2::ZERO, dt);
        return;
    }
    // The filters group (1135638): `trigger_left` / `right` → `MapManager::OnPrevFilter` / `OnNextFilter`.
    if menu.pressed(A::TriggerLeft) || menu.pressed(A::TriggerRight) {
        map.step_filter(menu.pressed(A::TriggerRight));
        if std::env::var_os("MW_MAP_LOG").is_some() {
            info!("map: filter {:?}, {} items", map.filters.get(map.filter), map.shown.len());
        }
    }
    // `MainMapNav_*` (analogue, every frame): menu_left / right / up / down (arrows, left stick, d-pad).
    let input = Vec2::new(menu.value(A::Right) - menu.value(A::Left), menu.value(A::Up) - menu.value(A::Down));
    let before = (map.selected, map.nav.moving);
    nav_update(&mut map, input, dt);
    if std::env::var_os("MW_MAP_LOG").is_some() && before != (map.selected, map.nav.moving) {
        let name = map.selected_item().map(|i| i.setup.name.clone());
        info!(
            "map: {:?} {:?} moving {} cursor ({:.0}, {:.0})",
            map.selected, name, map.nav.moving, map.cursor.x, map.cursor.y
        );
    }
    // `Options` (accept) → `TO_OPTIONS`: the selected item type's menu.
    if menu.pressed(A::Accept) {
        map.open_options();
    }
}

/// Values for the screen's bindings. `entry` is the item the layout being drawn belongs to.
#[allow(clippy::too_many_arguments)]
fn lookup(
    path: &str,
    arg: Option<&str>,
    map: &MapScreen,
    entry: Option<Entry>,
    selected: bool,
    vehicle: Option<u32>,
) -> Option<BindValue> {
    let s = |k: &str| BindValue::Text(map.strings.get(k).cloned().unwrap_or_default());
    let item = match entry {
        Some(Entry::Event(i)) => map.items.get(i),
        _ => None,
    };
    let point = match entry {
        Some(Entry::Point(i)) => map.points.get(i),
        _ => None,
    };
    let field = arg.and_then(|a| a.split_once("].")).map(|(_, f)| f);
    Some(match path {
        "Widget.Menu.Title" => s("title"),
        "Widget.MainMapState.Ticker1" => s("ticker"),
        "Widget.MainMapState.Ticker2" => BindValue::Text(String::new()),
        "Widget.Tabs.Action.Drive" => {
            BindValue::Text(format!("DRIVE {}", map.strings.get("escape").map_or("", |s| s.as_str())))
        }
        "Widget.Tabs.Action.NavTabLeft" => s("prior"),
        "Widget.Tabs.Action.NavTabRight" => s("next"),
        "Controls.IsActiveControllerKeyboard" => true.into(),
        "Widget.Filters.IsActive" => (map.options.is_none()).into(),
        "Widget.Filters.Action.MapFilterLeft" => s("z"),
        "Widget.Filters.Action.MapFilterRight" => s("a"),
        "Widget.MapManager.LiveMapManager.FilterName" => match map.filters.get(map.filter) {
            Some((_, name)) => BindValue::Text(name.clone()),
            None => s("filter"),
        },
        "Widget.MapControls.ShowTunePrompt" | "Widget.Awards.AwardsBackActive" | "Widget.Awards.ShowProfileOpt" => {
            false.into()
        }
        "UIConfig.IsArabic" | "Net.IsInOnlineGame" => false.into(),
        // Navigation tabs (`Widget_NavigationTabs.Tabs`): MWL, MAP, MPLAYER, FRIENDS, SETTINGS, STORE, with MAP
        // selected; each cell shows its tab's icon (`Image`), the selected one its name too.
        "Widget.Tabs.TabList._length" => (map.tabs.len().max(2) as f64).into(),
        "Widget.Tabs.SelectedIndex" => 1.0.into(),
        p if p.starts_with("Widget.Tabs.TabList[") => {
            let (i, f) = p["Widget.Tabs.TabList[".len()..].split_once("].")?;
            let tab = i.parse::<usize>().ok().and_then(|i| map.tabs.get(i));
            match f {
                "Name" if i == "1" && tab.is_none() => s("title"),
                "Name" => BindValue::Text(tab?.1.clone()),
                "Image" => BindValue::Text(tab?.0.clone()),
                "Items._length" => 0.0.into(),
                _ => return None,
            }
        }
        // The cursor: rings while browsing, the option list once open.
        "Widget.MapControls.IsActive" => (map.options.is_none()).into(),
        "Widget.MapControls.SelectedAction" => {
            if map.options.is_some() {
                BindValue::Text(String::new())
            } else {
                BindValue::Text("SELECT".into())
            }
        }
        "Widget.Action.MenuAccept" | "Widget.Options.Action.SubMenuAccept" | "Controls.ButtonPrompt" => s("accept"),
        "Widget.Options.Menu.NumItemsShown" => {
            (if map.options.is_some() { map.option_items.len() as f64 } else { 0.0 }).into()
        }
        "Widget.Options.Menu.SelectedIndex" => (map.options.unwrap_or(usize::MAX) as f64).into(),
        // The list's top edge above the pin: rows of 21 px ending 17 px above it (measured on the capture).
        // While the options group is inactive its list (and the list's footer) is not shown: moved off screen
        // (ours; the game does not run the group's widgets then).
        "Widget.Options.MainMapOptions.OptionHeight" => match map.options {
            Some(_) => (-17.0 - 21.0 * map.option_items.len() as f64).into(),
            None => (-10_000.0).into(),
        },
        p if p.starts_with("Widget.Options.Items[") => {
            let (i, f) = p["Widget.Options.Items[".len()..].split_once("].")?;
            let o = map.option_items.get(i.parse::<usize>().ok()?)?;
            match f {
                "Name" => BindValue::Text(o.label.clone()),
                _ => return None,
            }
        }
        // The item the layout is drawn for: `#` is its index, so only its own selection test matches.
        "LiveMapManager.SelectedIndex" => {
            BindValue::Text(if selected && map.options.is_none() { "#" } else { "-" }.into())
        }
        "LiveMapManager.ShowEventComplete" => false.into(),
        "LiveMapManager.Items" | "Widget.MapManager.Items" if point.is_some() => {
            let p = point?;
            match field? {
                "Name" => BindValue::Text(map.point_names.get(&p.item_type).cloned().unwrap_or_default()),
                "EventType" | "EventTypeWithDistance" => BindValue::Text(String::new()),
                "ScreenDistance" => (if selected { 1.0 } else { 0.0 }).into(),
                "IsActive" => true.into(),
                _ => return None,
            }
        }
        // The jack spot icon's visibility (`JackSpots.IsSwapSpotFound[#]`).
        "JackSpots.IsSwapSpotFound" => point.is_some_and(|p| map.found.contains(&p.id)).into(),
        "LiveMapManager.Items" | "Widget.MapManager.Items" => {
            let it = item?;
            match field? {
                "Name" => BindValue::Text(it.setup.name.clone()),
                "EventType" => BindValue::Text(it.type_name.clone()),
                "EventTypeWithDistance" => BindValue::Text(format!("{:.1} KM", it.length / 1000.0)),
                "ScreenDistance" => (if selected { 1.0 } else { 0.0 }).into(),
                "IsActive" => true.into(),
                _ => return None,
            }
        }
        "Events.Loading.Difficulty" => {
            let it = item?;
            let d = vehicle
                .and_then(|v| it.difficulty.iter().find(|(id, _)| *id == v))
                .or(it.difficulty.first())
                .map_or(0, |d| d.1);
            (d as f64).into()
        }
        // Speed-point rewards and the player's profile are not modelled.
        // Unknown: the label stays empty.
        "Widget.Header.HeaderData.Bounty" => return None,
        "Widget.Options.IsActive" => map.options.is_some().into(),
        "Widget.Header.HeaderData.Position" | "Widget.Header.ProfilePic" | "MostWantedListManager.LocalUserPoints" => {
            BindValue::Text(String::new())
        }
        "Players.PlayerBase" => false.into(),
        "Signals.True" => true.into(),
        "Signals.False" => false.into(),
        _ => return None,
    })
}

#[allow(clippy::too_many_arguments)]
fn draw(
    mut commands: Commands,
    mut map: ResMut<MapScreen>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    time: Res<Time<Real>>,
    garage: Res<crate::garage::Garage>,
    car: Query<&GlobalTransform, (With<crate::drive::Car>, With<crate::drive::Player>)>,
    mut nodes: Query<PoolNode, With<QuadNode>>,
    mut materials: ResMut<Assets<AdditiveQuadMaterial>>,
    gps: Res<crate::gps::Gps>,
    pois: Res<crate::poi::Pois>,
) {
    let Ok(window) = windows.single() else { return };
    let screen = [window.width(), window.height()];
    let open = map.open;
    let route = gps.ahead();
    let vehicle = garage.current.as_ref().and_then(|c| crate::frontend::vehicle_id(&c.bundle));
    let player_facing = car.single().ok().map(|xf| xf.back().as_vec3());
    let Some(mut look) = map.look.take() else { return };
    let mut quads: Vec<Quad> = Vec::new();
    if open {
        // The reference frame on the window as the game places its UI (`mw_assets::ui::UiFrame`).
        let frame = mw_assets::ui::UiFrame::letterbox(screen);
        let (k, origin) = (frame.scale, Vec2::from_array(frame.origin));
        let to_px = |p: Vec2| origin + p * k;
        let now = time.elapsed_secs_f64();
        let m = &*map;
        // One layout drawn at a screen point (reference pixels) for an item.
        let draw_at = |look: &mut Look,
                       out: &mut Vec<Quad>,
                       id: u32,
                       at: Option<Vec2>,
                       item: Option<Entry>,
                       selected: bool,
                       extra: &dyn Fn(&str) -> Option<BindValue>| {
            let Some(&s) = look.slots.get(&id) else { return };
            let mut options = DrawOptions::new(Some(Box::new(move |i| i == s)), Some(&mut look.anim), now);
            let mut lk = |p: &str, a: Option<&str>| extra(p).or_else(|| lookup(p, a, m, item, selected, vehicle));
            let offset = at.map_or(Vec2::ZERO, |a| a * k);
            for mut q in look.scene.draw_with(screen, &mut lk, &mut options) {
                q.rect[0] += offset.x;
                q.rect[1] += offset.y;
                out.push(q);
            }
        };
        let none = |_: &str| None;
        draw_at(&mut look, &mut quads, BLACK_BACKGROUND, None, None, false, &none);
        // The city image, turned and scaled about the camera.
        if let Some(tex) = look.map_texture {
            let [x0, z0, w, h] = MAP_WORLD;
            let centre = to_px(to_screen(Vec2::new(x0 + w * 0.5, z0 + h * 0.5), m.cursor));
            let size = Vec2::new(w, h) / METRES_PER_PX * k;
            quads.push(Quad {
                texture: Some(tex),
                uv: [0.0, 0.0, 1.0, 1.0],
                rect: [centre.x - size.x * 0.5, centre.y - size.y * 0.5, size.x, size.y],
                rotation: TURN_DEG,
                colour: [1.0; 4],
                blend: UiBlend::Alpha,
                map: false,
            });
        }
        // The SET DESTINATION route (`gps.rs`): a band of the road's width along the line, at least 3 px.
        for s in route.windows(2) {
            let (a, b) = (
                to_screen(Vec2::new(s[0].pos.x, s[0].pos.z), m.cursor),
                to_screen(Vec2::new(s[1].pos.x, s[1].pos.z), m.cursor),
            );
            let len = a.distance(b);
            if len < 1e-3 {
                continue;
            }
            let width = (0.5 * (s[0].width + s[1].width) / METRES_PER_PX).max(3.0) * k;
            let c = to_px((a + b) * 0.5);
            // Overlap the joints by the band's width.
            let l = len * k + width;
            quads.push(Quad {
                texture: None,
                uv: [0.0, 0.0, 1.0, 1.0],
                rect: [c.x - l * 0.5, c.y - width * 0.5, l, width],
                rotation: (b.y - a.y).atan2(b.x - a.x).to_degrees(),
                colour: crate::gps::LINE_COLOUR,
                blend: UiBlend::Alpha,
                map: false,
            });
        }
        let visible = |p: Vec2| p.x > -40.0 && p.x < 1320.0 && p.y > -40.0 && p.y < 760.0;
        // The current filter's items, each with its item type's icon (`MapItemTypes`).
        for (s, &e) in m.shown.iter().enumerate() {
            let p = to_screen(m.entry_pos(e), m.cursor);
            if visible(p) {
                let icon = m.icons.get(&m.entry_type(e)).copied().unwrap_or(EVENT_ICON);
                draw_at(&mut look, &mut quads, icon, Some(p), Some(e), m.selected == Some(Sel::Item(s)), &none);
            }
        }
        // Cops in a pursuit (`AICopPOI`, the arrow's cop variant on `Players.PlayerBase[#].IsCop`), turned with
        // their cars.
        if let Some(icon) = m.cop_icon {
            for &(pos, f) in &m.cops {
                let at = to_screen(pos, m.cursor);
                if !visible(at) {
                    continue;
                }
                let d = to_screen(pos + Vec2::new(f.x, f.z), m.cursor) - at;
                let rot = d.x.atan2(-d.y).to_degrees();
                let cop = move |p: &str| match p {
                    "Rot" => Some(BindValue::Num(rot as f64)),
                    "Players.PlayerBase" => Some(true.into()),
                    _ => None,
                };
                draw_at(&mut look, &mut quads, icon, Some(at), None, false, &cop);
            }
        }
        // The map manager's POI list (`poi.rs`) in its map order: each record whose key is one of this screen's
        // looks (`FUN_008162a0` / `FUN_00814470`; the intro's waypoints are not, the online meet-up's are), turned by
        // its matrix's row 2.
        for poi in &pois.icons {
            let Some(&icon) = m.poi_looks.get(&poi.key) else { continue };
            let pos = Vec2::new(poi.position.x, poi.position.z);
            let at = to_screen(pos, m.cursor);
            if !visible(at) {
                continue;
            }
            let d = to_screen(pos + Vec2::new(poi.direction.x, poi.direction.z), m.cursor) - at;
            let rot = d.x.atan2(-d.y).to_degrees();
            let turn = move |p: &str| (p == "Rot").then_some(BindValue::Num(rot as f64));
            draw_at(&mut look, &mut quads, icon, Some(at), None, false, &turn);
        }
        // The player's arrow, turned with the car (`Rot`, degrees clockwise on the map).
        let rot = player_facing.map_or(0.0, |f| {
            let d = to_screen(m.player + Vec2::new(f.x, f.z), m.cursor) - to_screen(m.player, m.cursor);
            d.x.atan2(-d.y).to_degrees()
        });
        let player_at = to_screen(m.player, m.cursor);
        let rot_lookup = move |p: &str| (p == "Rot").then_some(BindValue::Num(rot as f64));
        draw_at(&mut look, &mut quads, PLAYER_ICON, Some(player_at), None, false, &rot_lookup);
        // The selected item's card (its item type's, `MapItemTypes`) sits on its icon; the cursor's rings on the
        // cursor (the camera's look-at).
        let selected = m.selected_entry();
        if let Some(e) = selected
            && let Some(&card) = m.cards.get(&m.entry_type(e))
        {
            draw_at(&mut look, &mut quads, card, Some(to_screen(m.entry_pos(e), m.cursor)), Some(e), true, &none);
        }
        draw_at(&mut look, &mut quads, CURSOR, Some(ANCHOR), selected, true, &none);
        for id in [TITLE, TABS[0], TABS[1], FILTERS] {
            draw_at(&mut look, &mut quads, id, None, None, false, &none);
        }
        // The other tabs' icons are dark on their light cells in the original (`o7.png`), the selected one white
        // on red; the icons are white silhouettes and their elements carry no tint. Model decision: the bar's
        // own ink (its labels' colour 0.005, layout 1470689) for the tabs not selected (the code that colours
        // them was not traced).
        let dark: Vec<ResourceId> = m
            .tabs
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != 1)
            .filter_map(|(_, (hex, _))| ResourceId::parse(hex))
            .collect();
        for q in quads.iter_mut().filter(|q| q.texture.is_some_and(|t| dark.contains(&t))) {
            q.colour[..3].copy_from_slice(&[TAB_INK; 3]);
        }
        // The black bars outside the letterboxed frame (`FUN_006240b0`).
        quads.extend(frame.bars(screen));
    } else {
        look.anim.reset();
    }
    let Look { root, pool, textures, .. } = &mut look;
    show_quads(&mut commands, *root, pool, &quads, textures, None, &mut nodes, &mut materials);
    map.look = Some(look);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glide_ease_shape() {
        // FUN_00630980: 0 → 0, 0.5 → 0.5, 1 → 1, linear (slope 1.25) between 0.2 and 0.8, continuous at the joins.
        assert!(glide_ease(0.0).abs() < 1e-6);
        assert!((glide_ease(0.5) - 0.5).abs() < 1e-6);
        assert!((glide_ease(1.0) - 1.0).abs() < 1e-6);
        assert!((glide_ease(0.8) - 0.875).abs() < 1e-6);
        assert!((glide_ease(0.8001) - glide_ease(0.7999)).abs() < 1e-3);
        assert!((glide_ease(0.2) - 0.125).abs() < 1e-5);
    }

    #[test]
    fn screen_directions_round_trip() {
        let c = Vec2::new(100.0, -200.0);
        for d in [Vec2::X, Vec2::Y, Vec2::new(-3.0, 7.0)] {
            let w = c + screen_to_world(d);
            assert!((to_screen(w, c) - ANCHOR - d).length() < 1e-3, "{d}");
        }
    }

    fn map_with(events: &[Vec2]) -> MapScreen {
        let mut m = MapScreen { config: MapConfig::default(), ..default() };
        for (k, &pos) in events.iter().enumerate() {
            let setup =
                Arc::new(RaceSetup { event: Default::default(), game_mode: None, name: format!("E{k}"), test: false });
            m.items.push(MapItem {
                setup,
                pos,
                type_name: String::new(),
                length: 0.0,
                vehicles: Vec::new(),
                difficulty: Vec::new(),
                most_wanted: false,
                playable: false,
            });
            m.shown.push(Entry::Event(k));
        }
        m.selected = Some(Sel::Player);
        m
    }

    #[test]
    fn tap_flicks_hold_moves_then_snaps() {
        // Events 100 px right and 100 px up of the player on screen.
        let right = screen_to_world(Vec2::new(100.0, 0.0));
        let up = screen_to_world(Vec2::new(0.0, -100.0));
        let mut m = map_with(&[right, up]);
        let dt = 1.0 / 60.0;
        // A 50 ms tap right: no movement (move delay 0.1 s), a flick on release.
        for _ in 0..3 {
            nav_update(&mut m, Vec2::X, dt);
        }
        assert_eq!(m.cursor, Vec2::ZERO);
        nav_update(&mut m, Vec2::ZERO, dt);
        assert_eq!(m.selected, Some(Sel::Item(0)));
        // The cursor glides there within the glide time.
        for _ in 0..60 {
            nav_update(&mut m, Vec2::ZERO, dt);
        }
        assert!((m.cursor - right).length() < 1e-3);
        // Held up for a second: free cursor (nothing selected) moving at the configured speed.
        for _ in 0..60 {
            nav_update(&mut m, Vec2::Y, dt);
        }
        assert_eq!(m.selected, None);
        let moved = (m.cursor - right).length();
        assert!((moved - 1500.0 * (1.0 - 0.1)).abs() < 60.0, "{moved}");
        // Released after the flick time: snaps to the nearest item within 0.4 screen heights.
        nav_update(&mut m, Vec2::ZERO, dt);
        assert!(m.selected.is_some());
    }
}
