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
//! current car's events (green) and the Most Wanted events (red when beaten, the icon's `MAP_EVENT_DONE` step);
//! the selection sits at (836, 360) of the 1280×720 frame and the map turns so that world +X points down and
//! +Z left (94.4°), 6.29 m per pixel: fitted to eight icons of the capture against the events' start points
//! (research log 2026-10-10). The game draws the map with a 3D camera (`UIMapCameraConfig`: look-at
//! `Widget.MapManager.SelectedPosition`, distance 7000, field of view and tilt from bindings); here it is flat.
//!
//! Model decisions: the camera is orthographic with the measured scale and turn and eases to the selection
//! (rate ours); arrows pick the nearest item in that direction (the `LiveMapManager` script was not decoded);
//! no completion state (every event shows green); the Most Wanted list panel, the player's header (name,
//! points), the speedwall entries, the map's road glow (layouts 1222163 / 1222558) and the tabs' icons are left
//! out; SET DESTINATION is inactive, as are events whose game mode we do not run (speed runs, ambushes).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mw_assets::ui::{DrawOptions, PlacedLayout, Quad, UiAnim, UiBlend, UiScene};
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
/// Point-of-interest map looks (keys 1135497, 1135808, 1135303, 1135501).
const EVENT_ICON: u32 = 816_139;
const EVENT_CARD: u32 = 1_481_372;
const CURSOR: u32 = 1_470_763;
const PLAYER_ICON: u32 = 371_789;
/// The `Event` menu data (options for an event).
const EVENT_OPTIONS: u32 = 1_222_202;
/// "MAP", "RACES", "You are not connected to the Autolog Servers".
const TITLE_TEXT: u32 = 565_900;
const FILTER_TEXT: u32 = 1_135_274;
const OFFLINE_TICKER: u32 = 119_887;
/// The offline race game mode (the one `race.rs` runs).
const RACE_MODE: u32 = 730_000;

/// World rectangle (x, z, width, depth) the main-map image covers: registered against the `UIMapGrid` tiles,
/// whose placement is data (research log 2026-10-10).
const MAP_WORLD: [f32; 4] = [-4000.0, -4581.0, 7990.0, 9987.0];
/// Where the selection sits in the 1280×720 frame, the map's turn (degrees, clockwise on screen: world +X
/// points this far from screen-right) and its scale (metres per reference pixel); measured (module doc).
const ANCHOR: Vec2 = Vec2::new(836.0, 360.0);
const TURN_DEG: f32 = 94.4;
const METRES_PER_PX: f32 = 6.29;
/// Camera easing towards the selection (1/s; ours).
const EASE: f32 = 8.0;

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
}

impl Plugin for MapScreenPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MapScreen {
            enabled: self.enabled,
            open_at_start: self.open || self.start.is_some(),
            select: self.start.or(self.select),
            options_at_start: self.options || self.start.is_some(),
            start_at_start: self.start.is_some(),
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

#[derive(Clone)]
struct OptionItem {
    action: String,
    label: String,
}

#[derive(Resource, Default)]
pub struct MapScreen {
    pub enabled: bool,
    pub open: bool,
    /// Closed this frame (other menus must not take the same key).
    pub just_closed: bool,
    items: Vec<MapItem>,
    /// The events shown (indices into `items`) for the current car.
    shown: Vec<usize>,
    /// Selected entry of `shown`; `None` = the player.
    selected: Option<usize>,
    /// The options list is open, with this row selected.
    options: Option<usize>,
    option_items: Vec<OptionItem>,
    /// Camera look-at (world x, z).
    centre: Vec2,
    player: Vec2,
    strings: HashMap<&'static str, String>,
    job: Option<Arc<Mutex<Option<Loaded>>>>,
    look: Option<Look>,
    /// The virtual clock was running when the map opened.
    unpause: bool,
    /// Re-read the player's position and car (just opened).
    refresh: bool,
    open_at_start: bool,
    select: Option<u32>,
    options_at_start: bool,
    start_at_start: bool,
}

impl MapScreen {
    /// Open the map (from the pause menu): the player selected, the clock stopped; `unpause`: restart the clock
    /// when the map closes back to the game.
    pub fn open(&mut self, time: &mut Time<Virtual>, unpause: bool) {
        if self.look.is_none() {
            return;
        }
        self.open = true;
        self.selected = None;
        self.options = None;
        self.refresh = true;
        self.unpause = unpause;
        time.pause();
        if let Some(l) = self.look.as_mut() {
            l.anim.reset();
        }
    }

    pub fn ready(&self) -> bool {
        self.look.is_some()
    }

    /// Close; returns whether the clock is to be restarted.
    fn close(&mut self) -> bool {
        self.open = false;
        self.options = None;
        self.just_closed = true;
        std::mem::take(&mut self.unpause)
    }

    fn selected_item(&self) -> Option<&MapItem> {
        self.selected.and_then(|s| self.shown.get(s)).and_then(|&i| self.items.get(i))
    }

    /// The selection's world position (the player when nothing is selected).
    fn target(&self) -> Vec2 {
        self.selected_item().map_or(self.player, |i| i.pos)
    }
}

struct Loaded {
    items: Vec<MapItem>,
    options: Vec<OptionItem>,
    strings: HashMap<&'static str, String>,
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
    std::thread::Builder::new()
        .name("mw-map-screen".into())
        .spawn(move || {
            let t = std::time::Instant::now();
            let Ok(lang) = store.language(mw_assets::DEFAULT_LANGUAGE) else { return };
            let text = |id: u32| lang.get(id).unwrap_or_default().to_string();
            let items = load_items(&store, &lang);
            let options = store
                .menu_items(MAP_SCREEN, EVENT_OPTIONS)
                .map_err(|e| warn!("map screen: event options: {e}"))
                .unwrap_or_default()
                .into_iter()
                .map(|(action, name, _)| OptionItem { action, label: text(name).to_uppercase() })
                .collect();
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
            // The layouts drawn, in draw order.
            let ids = [BLACK_BACKGROUND, EVENT_ICON, PLAYER_ICON, EVENT_CARD, CURSOR, TITLE, TABS[0], TABS[1], FILTERS];
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
            let scene = store.ui_scene(layouts, &lang);
            let mut images: Vec<(ResourceId, Image)> = scene
                .textures()
                .into_iter()
                .chain(map_texture)
                .filter_map(|id| Some((id, game_text::decode_texture(&store, id)?)))
                .collect();
            images.dedup_by_key(|(id, _)| *id);
            let fonts = scene.fonts.iter().map(|(&n, f)| (n, f.clone())).collect();
            info!("map screen: {} events, {} textures in {:.0?}", items.len(), images.len(), t.elapsed());
            *out.lock().unwrap() = Some(Loaded { items, options, strings, scene, map_texture, images, fonts });
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
    let Some(loaded) = map.job.as_ref().and_then(|j| j.lock().unwrap().take()) else { return };
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
    map.option_items = loaded.options;
    map.strings = loaded.strings;
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
fn open_at_start(
    mut map: ResMut<MapScreen>,
    mut time: ResMut<Time<Virtual>>,
    car: Query<&GlobalTransform, (With<crate::drive::Car>, With<crate::drive::Player>)>,
    garage: Res<crate::garage::Garage>,
    mut play: MessageWriter<crate::event_card::PlayEvent>,
) {
    if map.refresh
        && let Ok(xf) = car.single()
    {
        map.refresh = false;
        let p = xf.translation();
        map.player = Vec2::new(p.x, p.z);
        map.centre = map.player;
        let vehicle = garage.current.as_ref().and_then(|c| crate::frontend::vehicle_id(&c.bundle));
        map.shown = shown_items(&map.items, vehicle);
    }
    if !map.open_at_start || map.look.is_none() {
        return;
    }
    let Ok(xf) = car.single() else { return };
    map.open_at_start = false;
    let p = xf.translation();
    map.player = Vec2::new(p.x, p.z);
    let vehicle = garage.current.as_ref().and_then(|c| crate::frontend::vehicle_id(&c.bundle));
    map.shown = shown_items(&map.items, vehicle);
    let unpause = !time.is_paused();
    map.open(&mut time, unpause);
    map.refresh = false;
    map.centre = map.player;
    if let Some(id) = map.select {
        // A selected event that the car's list does not hold is shown too (headless checks).
        let k = match map.items.iter().position(|i| i.setup.event.id == id) {
            Some(k) => k,
            None => {
                warn!("map screen: no event {id}");
                return;
            }
        };
        let s = map.shown.iter().position(|&i| i == k).unwrap_or_else(|| {
            map.shown.push(k);
            map.shown.len() - 1
        });
        map.selected = Some(s);
        map.centre = map.items[k].pos;
    }
    if map.options_at_start && map.selected.is_some() {
        map.options = Some(0);
    }
    if map.start_at_start
        && let Some(item) = map.selected_item().cloned()
    {
        map.close();
        play.write(crate::event_card::PlayEvent(item.setup.clone()));
    }
}

/// Screen position (reference pixels) of a world point with the camera at `centre`.
fn to_screen(world: Vec2, centre: Vec2) -> Vec2 {
    let d = (world - centre) / METRES_PER_PX;
    let (s, c) = TURN_DEG.to_radians().sin_cos();
    ANCHOR + Vec2::new(c * d.x - s * d.y, s * d.x + c * d.y)
}

/// The shown item nearest to the selection in a screen direction (within ±60°), for the arrows.
fn step_selection(map: &MapScreen, dir: Vec2) -> Option<Option<usize>> {
    let from = to_screen(map.target(), map.centre);
    let mut best: Option<(f32, Option<usize>)> = None;
    let candidates = map
        .shown
        .iter()
        .enumerate()
        .map(|(s, &i)| (Some(s), map.items[i].pos))
        .chain(std::iter::once((None, map.player)));
    for (s, pos) in candidates {
        if s == map.selected {
            continue;
        }
        let d = to_screen(pos, map.centre) - from;
        let along = d.dot(dir);
        if along <= 1.0 || along < d.length() * 0.5 {
            continue;
        }
        // Prefer close items near the direction.
        let score = d.length() * (2.0 - along / d.length());
        if best.is_none_or(|(b, _)| score < b) {
            best = Some((score, s));
        }
    }
    best.map(|b| b.1)
}

#[allow(clippy::too_many_arguments)]
fn navigate(
    mut map: ResMut<MapScreen>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut time: ResMut<Time<Virtual>>,
    real: Res<Time<Real>>,
    mut play: MessageWriter<crate::event_card::PlayEvent>,
    mut pause: ResMut<crate::pause::PauseMenu>,
) {
    map.just_closed = false;
    if !map.open {
        return;
    }
    let dt = real.delta_secs().min(0.1);
    let target = map.target();
    let k = 1.0 - (-EASE * dt).exp();
    map.centre = map.centre.lerp(target, k);
    let pad = |b: GamepadButton| pads.iter().any(|p| p.just_pressed(b));
    let up = keys.just_pressed(KeyCode::ArrowUp) || pad(GamepadButton::DPadUp);
    let down = keys.just_pressed(KeyCode::ArrowDown) || pad(GamepadButton::DPadDown);
    let left = keys.just_pressed(KeyCode::ArrowLeft) || pad(GamepadButton::DPadLeft);
    let right = keys.just_pressed(KeyCode::ArrowRight) || pad(GamepadButton::DPadRight);
    let accept =
        keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter) || pad(GamepadButton::South);
    let back = keys.just_pressed(KeyCode::Backspace) || pad(GamepadButton::East);
    // DRIVE (Esc / pad Start): back to the game.
    let drive = keys.just_pressed(KeyCode::Escape) || pad(GamepadButton::Start);
    if drive {
        if map.close() {
            time.unpause();
        }
        return;
    }
    if let Some(row) = map.options {
        let n = map.option_items.len().max(1);
        if back {
            map.options = None;
        } else if up {
            map.options = Some((row + n - 1) % n);
        } else if down {
            map.options = Some((row + 1) % n);
        } else if accept {
            let action = map.option_items.get(row).map(|o| o.action.clone()).unwrap_or_default();
            let playable = map.selected_item().is_some_and(|i| i.playable);
            if action.ends_with(":PLAY_EVENT") && playable {
                let setup = map.selected_item().map(|i| i.setup.clone());
                // The event card restarts the clock when the race starts.
                map.close();
                if let Some(setup) = setup {
                    info!("map screen: START RACE {} ({})", setup.name, setup.event.id);
                    play.write(crate::event_card::PlayEvent(setup));
                }
            }
        }
        return;
    }
    if back {
        // Back to the pause menu ("NAV_BACK").
        let unpause = map.close();
        pause.reopen = Some(unpause);
        return;
    }
    for (pressed, dir) in [(up, Vec2::NEG_Y), (down, Vec2::Y), (left, Vec2::NEG_X), (right, Vec2::X)] {
        if pressed && let Some(next) = step_selection(&map, dir) {
            map.selected = next;
        }
    }
    if accept && map.selected.is_some() {
        map.options = Some(0);
    }
}

/// Values for the screen's bindings. `item` is the event the layout being drawn belongs to.
#[allow(clippy::too_many_arguments)]
fn lookup(
    path: &str,
    arg: Option<&str>,
    map: &MapScreen,
    item: Option<&MapItem>,
    selected: bool,
    vehicle: Option<u32>,
) -> Option<BindValue> {
    let s = |k: &str| BindValue::Text(map.strings.get(k).cloned().unwrap_or_default());
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
        "Widget.MapManager.LiveMapManager.FilterName" => s("filter"),
        "Widget.MapControls.ShowTunePrompt" | "Widget.Awards.AwardsBackActive" | "Widget.Awards.ShowProfileOpt" => {
            false.into()
        }
        "UIConfig.IsArabic" | "Net.IsInOnlineGame" => false.into(),
        // Navigation tabs: the MAP tab selected among six (their icons and names are not decoded but MAP's).
        "Widget.Tabs.TabList._length" => 6.0.into(),
        "Widget.Tabs.SelectedIndex" => 1.0.into(),
        p if p.starts_with("Widget.Tabs.TabList[") => {
            let (i, f) = p["Widget.Tabs.TabList[".len()..].split_once("].")?;
            match f {
                "Name" if i == "1" => s("title"),
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
) {
    let Ok(window) = windows.single() else { return };
    let screen = [window.width(), window.height()];
    let open = map.open;
    let vehicle = garage.current.as_ref().and_then(|c| crate::frontend::vehicle_id(&c.bundle));
    let player_facing = car.single().ok().map(|xf| xf.back().as_vec3());
    let Some(mut look) = map.look.take() else { return };
    let mut quads: Vec<Quad> = Vec::new();
    if open {
        // The reference frame centred on the window, scaled to fit (as `Placement` places layouts).
        let k = (screen[0] / 1280.0).min(screen[1] / 720.0);
        let origin = Vec2::new(screen[0] * 0.5 - 640.0 * k, screen[1] * 0.5 - 360.0 * k);
        let to_px = |p: Vec2| origin + p * k;
        let now = time.elapsed_secs_f64();
        let m = &*map;
        // One layout drawn at a screen point (reference pixels) for an item.
        let draw_at = |look: &mut Look,
                       out: &mut Vec<Quad>,
                       id: u32,
                       at: Option<Vec2>,
                       item: Option<&MapItem>,
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
            let centre = to_px(to_screen(Vec2::new(x0 + w * 0.5, z0 + h * 0.5), m.centre));
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
        let visible = |p: Vec2| p.x > -40.0 && p.x < 1320.0 && p.y > -40.0 && p.y < 760.0;
        for (s, &i) in m.shown.iter().enumerate() {
            let item = &m.items[i];
            let p = to_screen(item.pos, m.centre);
            if visible(p) {
                draw_at(&mut look, &mut quads, EVENT_ICON, Some(p), Some(item), m.selected == Some(s), &none);
            }
        }
        // The player's arrow, turned with the car (`Rot`, degrees clockwise on the map).
        let rot = player_facing.map_or(0.0, |f| {
            let d = to_screen(m.player + Vec2::new(f.x, f.z), m.centre) - to_screen(m.player, m.centre);
            d.x.atan2(-d.y).to_degrees()
        });
        let player_at = to_screen(m.player, m.centre);
        let rot_lookup = move |p: &str| (p == "Rot").then(|| BindValue::Num(rot as f64));
        draw_at(&mut look, &mut quads, PLAYER_ICON, Some(player_at), None, false, &rot_lookup);
        let target = to_screen(m.target(), m.centre);
        if let Some(item) = m.selected_item() {
            draw_at(&mut look, &mut quads, EVENT_CARD, Some(target), Some(item), true, &none);
        }
        draw_at(&mut look, &mut quads, CURSOR, Some(target), m.selected_item(), true, &none);
        for id in [TITLE, TABS[0], TABS[1], FILTERS] {
            draw_at(&mut look, &mut quads, id, None, None, false, &none);
        }
    } else {
        look.anim.reset();
    }
    let Look { root, pool, textures, .. } = &mut look;
    show_quads(&mut commands, *root, pool, &quads, textures, None, &mut nodes, &mut materials);
    map.look = Some(look);
}
