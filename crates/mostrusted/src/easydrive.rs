//! Easydrive, the game's in-drive d-pad menu (first version). The tree comes from the driving screen's data
//! (`mw_assets::easydrive`, research log 2026-10-06 session 5): tiers, their link / hook items and labels
//! from the language table. Link items open the tier their token names; the jack-spot tier (the one that
//! has a `JSMANUFACT` entry source) lists the catalogue's manufacturers, then their models, and drives the
//! chosen car through the garage (`GarageAction::Drive`). The RACES tier (`TO_FMN`, entries from the code's
//! `Widget.FMNEAREST` list) lists the offline races (game mode 730000) offered with the driven car (their
//! `AllowedVehicleList`'s cars; every race when the car has none), nearest start first (`FMNEAREST`,
//! `CalculateDistanceToEventByIdString`); choosing one opens its ACTIONS tier (`TO_FMN_OPT`: "SET
//! DESTINATION", inert, and "START RACE", which starts it through the event-loading card,
//! `event_card::PlayEvent`, as the map's START RACE does: model decision, research log 2026-10-10 left open
//! whether the original plays sequence 1014321 here too). Which events the code's list holds and its order are
//! our reading of the names (model decision). Other hook items and generated tiers are shown and inert.
//!
//! The look is the `EasyDrive` widget's menu layout (`MENU_LAYOUT`): an element stack whose header lists the
//! tier history, whose rows repeat the item layouts over `Widget.EasyDrive.Items` (the selected row's template
//! where `Widget.EasyDrive.SelectedIndex` equals the row) and whose footer holds the scroll arrows; drawn
//! through `UiScene` like the HUD, with the bindings in `lookup` filled from the menu state.
//!
//! Model decisions (not from the data or code yet): the rows shown at once (`WINDOW`), which of the widget's
//! three layouts shows when (code: only the menu layout, while open) and the keyboard keys. The data binds the `EasyDrive::*` actions to
//! the input mappings `easy_up` (previous), `easy_down` (next), `easy_left` (back a tier), `easy_right` (select;
//! `Hack_EasyRight` opens) and `cancel` (quick close); which keys those mappings are on PC is not decoded, so
//! here: the original's EASY_* bindings (d-pad; Num 4 / 6 / 8 / 2, input.rs), Esc / pad B to close, `E` opens (ours).
//!
//! **Availability** (the logic's, while this menu is still the runtime's own; research log 2026-11-19): the
//! Easydrive machines leave the open menu through `INTERRUPT` while `UISequences.HideHudRequests(GTEQ:1)` ("Run
//! Child - Exit When EasyDrive Unavailable" 1445505) and their prompt then waits for the HUD ("Wait Show EZ Drive"
//! 1445536) instead of for input: the menu closes and does not open while a sequence hides the HUD (an event's
//! intro). The `EASYDRIVE` control group (input group 6, also listed by `CAR`; `GameLogicState`, 0x6b1770) blocked
//! by an `HSB_DISCONGP` state ("Disable Car Controls and Easydrive" 2050378 / 1560571 / 1445348, `GROUP` = `CAR`:
//! the boot, the event intro) keeps the menu's input away.

use crate::platform::PollLock;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mw_assets::easydrive::{self as ed, EASYDRIVE_SCREEN};

use crate::Game;
use crate::race::RaceSetup;
use std::collections::HashMap;

use mw_assets::ui::UiScene;
use mw_bundle::ResourceId;
use mw_formats::binding::BindValue;

use crate::game_text;
use crate::garage::{Garage, GarageAction};
use crate::mods_menu::{ModCatalogue, ModSelection};
use crate::ui_quads::{AdditiveQuadMaterial, PoolNode, QuadNode, QuadPool};

pub struct EasyDrivePlugin {
    pub enabled: bool,
    /// Open at startup (`--show-easydrive`, for screenshots).
    pub open: bool,
    /// `--easydrive-path`: rows to enter one after the other once the car has loaded (headless checks).
    pub path: Vec<usize>,
}

impl Plugin for EasyDrivePlugin {
    fn build(&self, app: &mut App) {
        let script = self.path.iter().rev().copied().collect();
        app.insert_resource(EasyDrive { enabled: self.enabled, open: self.open, show: self.open, script, ..default() })
            .add_systems(Startup, start_load)
            .add_systems(Update, (poll_load, navigate, redraw).chain());
    }
}

/// A tier with resolved labels.
#[derive(Debug, Clone)]
struct Tier {
    transition: String,
    title: String,
    /// (label, link token or empty for hooks).
    items: Vec<(String, String)>,
    /// The jack-spot tier: its entries are generated from the car catalogue.
    jack: bool,
}

#[derive(Debug, Clone)]
enum Page {
    Tier(usize),
    /// The jack tier's model list for one manufacturer.
    Models(String),
    /// "Customize car": a mod class's types (the `TO_OPT` tier, "MOD TYPE"), generated from the car's
    /// `PerformanceUpgrades` (mods_menu.rs).
    ModClass(usize),
    /// A mod type's two levels (the "MOD STANDARD" tier): (class, slot).
    ModType(usize, usize),
    /// A race's ACTIONS tier (`TO_FMN_OPT`): index into `EasyDrive::races`.
    RaceActions(usize),
}

#[derive(Resource, Default)]
pub struct EasyDrive {
    enabled: bool,
    pub open: bool,
    /// Opened at least once (the HUD's `Widget.Prompt.New` model, `hud.rs`).
    pub seen: bool,
    /// `--show-easydrive`: the menu the logic closed opens again once it may.
    reopen: bool,
    show: bool,
    tiers: Vec<Tier>,
    job: Option<Arc<Mutex<Option<Loaded>>>>,
    look: Option<Look>,
    /// Open pages, root first, with the selected row of each.
    stack: Vec<(Page, usize)>,
    dirty: bool,
    /// Closed by Esc this frame: the car menu must not take the same key press as "pause".
    pub just_closed: bool,
    /// Rows still to enter (`--easydrive-path`), last first.
    script: Vec<usize>,
    /// Offline races for the RACES tier.
    races: Vec<RaceItem>,
    /// "SET DESTINATION" / "START RACE" (the `TO_FMN_OPT` tier's RPC items, generated in code; labels from the
    /// black-list tier's hook items `SetWaypointToEventFromBinding` / `StartEventFromBinding`).
    race_actions: [String; 2],
}

impl EasyDrive {
    /// `Widget.EasyDrive.IsActive`: the menu is up.
    pub fn is_active(&self) -> bool {
        self.open && !self.stack.is_empty()
    }
}

/// An offline race Easydrive can start.
#[derive(Clone)]
struct RaceItem {
    setup: Arc<RaceSetup>,
    start: Vec3,
    /// Vehicle list ids of the event's allowed cars.
    vehicles: Vec<u32>,
}

/// Language ids of "SET DESTINATION" and "START RACE".
const RACE_ACTION_LABELS: [u32; 2] = [1_222_255, 1_222_256];

/// Where the player is and what they drive (for the RACES tier).
#[derive(Default)]
struct Here {
    position: Option<Vec3>,
    vehicle: Option<u32>,
}

impl Here {
    fn new(garage: &Garage, game: &Game, player: Option<&GlobalTransform>) -> Self {
        let bundle = match (&garage.current, &game.request) {
            (Some(c), _) => c.bundle.clone(),
            (None, crate::loader::Request::Drive { bundle, .. }) => bundle.clone(),
            _ => String::new(),
        };
        let vehicle = bundle
            .to_ascii_uppercase()
            .rsplit('/')
            .next()
            .and_then(|b| b.strip_prefix("VEH_"))
            .and_then(|b| b.split('_').next())
            .and_then(|id| id.parse().ok());
        Self { position: player.map(|t| t.translation()), vehicle }
    }
}

/// What the loader thread hands over: the tiers and the menu layout's look.
type Loaded = (Vec<Tier>, Option<Look>, Vec<RaceItem>, [String; 2]);

/// The offline races (game mode 730000) with a start and checkpoints.
fn load_races(store: &mw_assets::AssetStore, lang: &mw_formats::Language) -> Vec<RaceItem> {
    let events = match store.events() {
        Ok(e) => e,
        Err(e) => {
            warn!("Easydrive: no events ({e})");
            return Vec::new();
        }
    };
    events
        .events
        .iter()
        .filter(|e| e.game_mode == 730_000 && !e.checkpoints.is_empty())
        .filter_map(|e| {
            let start = Vec3::from_array(e.spawn?.position);
            let name = lang.get(e.name).map_or_else(|| format!("EVENT {}", e.id), str::to_owned);
            let setup =
                RaceSetup { event: e.clone(), game_mode: events.game_mode(e.game_mode).copied(), name, test: false };
            Some(RaceItem {
                setup: Arc::new(setup),
                start,
                vehicles: e.allowed_vehicles.iter().map(|v| v.vehicle).collect(),
            })
        })
        .collect()
}

/// The menu layout's scene and decoded textures (loader thread), then its Bevy side.
struct Look {
    scene: UiScene,
    images: Vec<(ResourceId, Image)>,
    textures: HashMap<ResourceId, (Handle<Image>, Vec2)>,
    root: Option<Entity>,
    pool: QuadPool,
}

/// The `EasyDrive` widget's menu layout in `EASYDRIVE_SCREEN` (UniqueId): the element stack at (3, 43). The
/// widget's other layouts are the prompt (1805584: d-pad icon, the EASYDRIVE strip, alert, "NEW" badge), drawn
/// above the open menu as in the original (`ref/ui-2026-10-19/easydrive_1680x720.png`; the widget is a
/// `Widget_ShowLayouts`, which shows all of its layouts), and an empty one (1713378). Closed, the `Prompt`
/// widget's copy of the prompt shows instead (`hud.rs`).
const MENU_LAYOUT: u32 = 1_773_380;
const PROMPT_LAYOUT: u32 = crate::hud::PROMPT_LAYOUT;

fn load_look(store: &mw_assets::AssetStore, lang: &mw_formats::Language) -> Option<Look> {
    let layouts = store
        .ui_screen_layouts(EASYDRIVE_SCREEN, Some(&["EasyDrive"]))
        .map_err(|e| warn!("Easydrive layouts unavailable: {e}"))
        .ok()?
        .into_iter()
        .filter(|l| l.layout.id == MENU_LAYOUT || l.layout.id == PROMPT_LAYOUT)
        .collect();
    let scene = store.ui_scene(layouts, lang);
    let images =
        scene.textures().into_iter().filter_map(|id| Some((id, game_text::decode_texture(store, id)?))).collect();
    Some(Look { scene, images, textures: HashMap::new(), root: None, pool: QuadPool::default() })
}

fn start_load(mut menu: ResMut<EasyDrive>, game: Res<Game>) {
    if !menu.enabled && !menu.open {
        return;
    }
    let job = Arc::new(Mutex::new(None));
    let (out, store) = (job.clone(), game.store.clone());
    crate::platform::thread::spawn(move || {
        let tiers = store
            .language(mw_assets::DEFAULT_LANGUAGE)
            .and_then(|lang| Ok((store.easydrive(EASYDRIVE_SCREEN)?, lang)))
            .map(|(m, lang)| {
                let actions = RACE_ACTION_LABELS.map(|id| lang.get(id).unwrap_or_default().to_string());
                (resolve(&m, &lang), load_look(&store, &lang), load_races(&store, &lang), actions)
            })
            .map_err(|e| warn!("Easydrive data unavailable: {e}"))
            .unwrap_or_default();
        *out.lock().unwrap() = Some(tiers);
    });
    menu.job = Some(job);
}

fn resolve(m: &ed::EasyDrive, lang: &mw_formats::Language) -> Vec<Tier> {
    m.tiers
        .iter()
        .map(|t| Tier {
            transition: t.transition.clone(),
            title: if t.title == 0 { String::new() } else { ed::label(lang, t.title) },
            items: t
                .lists
                .iter()
                .flat_map(|l| {
                    l.items.iter().map(|i| {
                        let link = if l.kind == "Link" { i.token.clone() } else { String::new() };
                        (ed::label(lang, i.name), link)
                    })
                })
                .collect(),
            jack: t.sources.iter().any(|s| s.item_type == "JSMANUFACT"),
        })
        .collect()
}

fn poll_load(mut commands: Commands, mut menu: ResMut<EasyDrive>, mut images: ResMut<Assets<Image>>) {
    let Some((tiers, look, races, race_actions)) =
        menu.job.as_ref().and_then(|j| j.poll_lock().and_then(|mut s| s.take()))
    else {
        return;
    };
    menu.races = races;
    menu.race_actions = race_actions;
    menu.job = None;
    menu.look = look.map(|mut look| {
        for (id, img) in std::mem::take(&mut look.images) {
            let size = img.size().as_vec2();
            look.textures.insert(id, (images.add(img), size));
        }
        let node = Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        };
        look.root = Some(commands.spawn((node, Pickable::IGNORE, GlobalZIndex(5))).id());
        look
    });
    info!("Easydrive: {} tiers from {EASYDRIVE_SCREEN}", tiers.len());
    menu.tiers = tiers;
    if menu.open {
        open_root(&mut menu);
    }
}

fn open_root(menu: &mut EasyDrive) {
    menu.stack.clear();
    if let Some(i) = menu.tiers.iter().position(|t| t.transition == "TO_BASE") {
        menu.stack.push((Page::Tier(i), 0));
        menu.open = true;
        menu.seen = true;
    }
    menu.dirty = true;
}

/// What a row does when chosen.
enum Choice {
    Tier(usize),
    Make(String),
    Car(usize),
    ModClass(usize),
    ModType(usize, usize),
    /// Equip (class, Some((slot, category))) or return the class to stock.
    Mod(usize, Option<(usize, usize)>),
    Race(usize),
    StartRace(usize),
    Inert,
}

/// The rows of a page: (label, enabled, choice).
fn rows(
    menu: &EasyDrive,
    page: &Page,
    garage: &Garage,
    mods: &ModCatalogue,
    sel: &ModSelection,
    here: &Here,
) -> Vec<(String, Choice)> {
    // The equipped row's mark (ours; the game's row template has its own equipped state, not read yet).
    let mark = |on: bool, label: String| if on { format!("{label} - ON") } else { label };
    match page {
        // The mod-class tier: its classes come from the car (`MODCLASS` source), then the tier's own links.
        Page::Tier(i) if menu.tiers[*i].transition == "TO_MOD" => {
            let mut v: Vec<(String, Choice)> =
                mods.classes().into_iter().map(|(k, label)| (label.to_string(), Choice::ModClass(k))).collect();
            v.extend(menu.tiers[*i].items.iter().map(|(label, _)| (label.clone(), Choice::Inert)));
            v
        }
        Page::ModClass(k) => {
            let on = mods.equipped(sel, *k);
            let mut v = vec![(mark(on.is_none(), "STOCK".into()), Choice::Mod(*k, None))];
            v.extend(mods.class_slots(*k).into_iter().map(|s| {
                (mark(on.is_some_and(|(e, _)| e == s), mods.name(s, crate::mods_menu::BASE)), Choice::ModType(*k, s))
            }));
            v
        }
        Page::ModType(k, s) => {
            let on = mods.equipped(sel, *k);
            [crate::mods_menu::BASE, crate::mods_menu::PRO]
                .into_iter()
                .map(|c| (mark(on == Some((*s, c)), mods.name(*s, c)), Choice::Mod(*k, Some((*s, c)))))
                .collect()
        }
        Page::Tier(i) if menu.tiers[*i].transition == "TO_FMN" => {
            let offered = |r: &RaceItem| here.vehicle.is_some_and(|v| r.vehicles.contains(&v));
            let any = menu.races.iter().any(offered);
            let mut list: Vec<(f32, usize)> = menu
                .races
                .iter()
                .enumerate()
                .filter(|(_, r)| !any || offered(r))
                .map(|(k, r)| (here.position.map_or(0.0, |p| p.distance(r.start)), k))
                .collect();
            list.sort_by(|a, b| a.0.total_cmp(&b.0));
            list.into_iter().map(|(_, k)| (menu.races[k].setup.name.clone(), Choice::Race(k))).collect()
        }
        Page::RaceActions(k) => {
            vec![(menu.race_actions[0].clone(), Choice::Inert), (menu.race_actions[1].clone(), Choice::StartRace(*k))]
        }
        Page::Tier(i) if menu.tiers[*i].jack => {
            let mut makes: Vec<String> = garage.catalogue.iter().map(|c| c.make.clone()).collect();
            makes.dedup();
            makes.into_iter().map(|m| (m.clone(), Choice::Make(m))).collect()
        }
        Page::Tier(i) => menu.tiers[*i]
            .items
            .iter()
            .map(|(label, token)| {
                let target = menu.tiers.iter().position(|t| !token.is_empty() && t.transition == *token);
                (label.clone(), target.map_or(Choice::Inert, Choice::Tier))
            })
            .collect(),
        Page::Models(make) => garage
            .catalogue
            .iter()
            .enumerate()
            .filter(|(_, c)| c.make == *make)
            .map(|(i, c)| (c.model.clone(), Choice::Car(i)))
            .collect(),
    }
}

/// The `EASYDRIVE` control group's input group (the input manager's table, `mw_logic::natives::game_logic_view`).
const EASYDRIVE_INPUT_GROUP: usize = 6;

#[allow(clippy::too_many_arguments)]
fn navigate(
    mut menu: ResMut<EasyDrive>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    garage: Res<Garage>,
    mods: Res<ModCatalogue>,
    mut mod_sel: ResMut<ModSelection>,
    mut actions: MessageWriter<GarageAction>,
    mut race_start: MessageWriter<crate::event_card::PlayEvent>,
    game: Res<Game>,
    player: Query<&GlobalTransform, (With<crate::drive::Car>, With<crate::drive::Player>)>,
    front: Option<Res<crate::frontend::FrontEnd>>,
    jack: Option<Res<crate::jackspots::JackSpots>>,
    screens: (Option<Res<crate::map_screen::MapScreen>>, Option<Res<crate::event_card::EventCard>>),
    view: Option<NonSend<crate::ui_sequence::LogicView>>,
) {
    // The logic's say (module doc "Availability"): the menu closes while a sequence hides the HUD, and the
    // `EASYDRIVE` control group's block keeps its input away.
    if let Some(view) = &view {
        let st = view.0.borrow();
        let hidden = st.resolve_bool("UISequences.HideHudRequests(GTEQ:1)") == Some(true);
        let blocked = st.game.input_blocks.counts[EASYDRIVE_INPUT_GROUP] != 0;
        drop(st);
        if hidden && menu.open {
            info!("Easydrive: closed, a sequence hides the HUD");
            menu.stack.clear();
            menu.open = false;
            menu.just_closed = true;
            menu.dirty = true;
            menu.reopen = menu.show;
        }
        if hidden || blocked {
            return;
        }
        if std::mem::take(&mut menu.reopen) && !menu.tiers.is_empty() {
            open_root(&mut menu);
        }
    }
    // Not over the front end's screens ("Disable Car Control and Easydrive"), the map or the event card.
    let (map, card) = screens;
    if !menu.enabled
        || menu.tiers.is_empty()
        || front.is_some_and(|f| !f.playing())
        || map.is_some_and(|m| m.open || m.just_closed)
        || card.is_some_and(|c| c.showing())
    {
        return;
    }
    // `E` is the original's SWAP_CARS key: beside a parked car it swaps (`jackspots.rs`).
    let swap_key = jack.is_some_and(|j| j.prompting());
    // The original's EASY_* actions (Num 4 / 6 / 8 / 2, d-pad; input.rs). EASY_OPEN has no default binding in the
    // original's tables; opening with "right" is the console layout, `E` is ours for keyboards without a numpad.
    use crate::input::defaults as act;
    let pad_list: Vec<&Gamepad> = pads.iter().collect();
    let dev = crate::input::Devices { keys: &keys, pads: &pad_list };
    let (up, down) = (dev.just_pressed(act::EASY_UP), dev.just_pressed(act::EASY_DOWN));
    let (left, mut right) = (dev.just_pressed(act::EASY_LEFT), dev.just_pressed(act::EASY_RIGHT));
    menu.just_closed = false;
    // `--easydrive-path`: select the next scripted row and enter it, one per frame, once the car's mods are known.
    if menu.open
        && !menu.stack.is_empty()
        && !mods.mods.is_empty()
        && let Some(row) = menu.script.pop()
    {
        menu.stack.last_mut().unwrap().1 = row;
        right = true;
    }
    if !menu.open {
        if (keys.just_pressed(KeyCode::KeyE) && !swap_key) || right {
            info!("Easydrive: opened ({})", if right { "EASY_RIGHT" } else { "E" });
            open_root(&mut menu);
        }
        return;
    }
    // `Hack_EasyDriveQuit` = the `cancel` mapping: EasyDrive::QuickClose.
    if keys.just_pressed(KeyCode::Escape) || pad_list.iter().any(|p| p.just_pressed(GamepadButton::East)) {
        menu.stack.clear();
        menu.open = false;
        menu.just_closed = true;
        menu.dirty = true;
        return;
    }
    let Some((page, sel)) = menu.stack.last().cloned() else { return };
    let here = Here::new(&garage, &game, player.single().ok());
    let list = rows(&menu, &page, &garage, &mods, &mod_sel, &here);
    if up || down {
        let n = list.len().max(1);
        let sel = if up { (sel + n - 1) % n } else { (sel + 1) % n };
        menu.stack.last_mut().unwrap().1 = sel;
        menu.dirty = true;
    } else if left {
        menu.stack.pop();
        menu.open = !menu.stack.is_empty();
        menu.dirty = true;
    } else if right {
        match list.into_iter().nth(sel).map(|(_, c)| c) {
            Some(Choice::Tier(t)) => menu.stack.push((Page::Tier(t), 0)),
            Some(Choice::Make(m)) => menu.stack.push((Page::Models(m), 0)),
            Some(Choice::Car(i)) => {
                let c = &garage.catalogue[i];
                actions.write(GarageAction::Drive { name: c.name.clone(), bundle: c.bundle.clone(), paint_index: 0 });
                menu.stack.clear();
                menu.open = false;
            }
            Some(Choice::ModClass(k)) => menu.stack.push((Page::ModClass(k), 0)),
            Some(Choice::ModType(k, s)) => menu.stack.push((Page::ModType(k, s), 0)),
            // The game applies a mod while driving; mods_menu::apply_live picks the change up.
            Some(Choice::Mod(k, choice)) => {
                mods.equip(&mut mod_sel, k, choice);
                if matches!(page, Page::ModType(..)) {
                    menu.stack.pop();
                }
            }
            Some(Choice::Race(k)) => menu.stack.push((Page::RaceActions(k), 0)),
            Some(Choice::StartRace(k)) => {
                race_start.write(crate::event_card::PlayEvent(menu.races[k].setup.clone()));
                menu.stack.clear();
                menu.open = false;
            }
            Some(Choice::Inert) | None => {}
        }
        menu.dirty = true;
    }
}

/// Rows shown at once (model decision: the script's `Widget.EasyDrive.Menu.NumItemsShown` is set by code; the
/// footer's scroll arrows show from `Length >= 11`, which suggests ten).
const WINDOW: usize = 10;

/// Values for the bindings the menu layout uses (`Widget.EasyDrive.*`; inside a row the item's fields arrive as
/// `Widget.EasyDrive.Items[<row>].<field>` or `Widget.EasyDrive.CurrentItem.<field>`).
fn lookup(path: &str, arg: Option<&str>, history: &[String], names: &[String], sel: usize) -> Option<BindValue> {
    let item_field = |name: Option<&String>, f: &str| -> Option<BindValue> {
        Some(match f {
            "Name" => BindValue::Text(name?.to_uppercase()),
            // `Shutdown(IS_EMPTY)` picks the plain name label over the backed "shut down" one.
            "Shutdown" => BindValue::Text(String::new()),
            "Locked" | "Unavailable" | "New" | "PositionNotNull" => false.into(),
            _ => return None,
        })
    };
    if let Some(f) = path.strip_prefix("Widget.EasyDrive.CurrentItem.") {
        return item_field(names.get(sel), f);
    }
    if let Some(rest) = path.strip_prefix("Widget.EasyDrive.Items[") {
        let (i, f) = rest.split_once("].")?;
        return item_field(names.get(i.parse::<usize>().ok()?), f);
    }
    Some(match path {
        "Widget.EasyDrive.Items._length" | "Widget.EasyDrive.EasyDrive.Length" => (names.len() as f64).into(),
        "Widget.EasyDrive.SelectedIndex" => (sel as f64).into(),
        "Widget.EasyDrive.EasyDrive.TierHistoryLength" => (history.len() as f64).into(),
        // `TierHistory[#].Name`: the argument arrives as `<index>].Name`.
        "Widget.EasyDrive.EasyDrive.TierHistory" => {
            let i = arg?.split(']').next()?.parse::<usize>().ok()?;
            BindValue::Text(history.get(i)?.to_uppercase())
        }
        "Signals.False" => false.into(),
        // The prompt's alert and NEW badge: Autolog alerts are not modelled; the badge belongs to the `Prompt`
        // widget, whose group is not active while the menu is open (model decision).
        "Recommends.ShowAlert" | "Widget.Prompt.New" => false.into(),
        // Item types: the special row templates (paint, nearest event, recommends, friends) are not ours yet.
        p if p.starts_with("Widget.") && p.ends_with(".IsActive") => false.into(),
        _ => return None,
    })
}

#[allow(clippy::too_many_arguments)]
fn redraw(
    mut commands: Commands,
    mut menu: ResMut<EasyDrive>,
    garage: Res<Garage>,
    mods: Res<ModCatalogue>,
    mod_sel: Res<ModSelection>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut nodes: Query<PoolNode, With<QuadNode>>,
    mut materials: ResMut<Assets<AdditiveQuadMaterial>>,
    game: Res<Game>,
    player: Query<&GlobalTransform, (With<crate::drive::Car>, With<crate::drive::Player>)>,
    mut resized: MessageReader<bevy::window::WindowResized>,
) {
    // A resized window places the layout anew (the UI follows the window's current size).
    let resized = resized.read().count() > 0;
    if !menu.dirty && !garage.is_changed() && !mod_sel.is_changed() && !resized {
        return;
    }
    let Ok(window) = windows.single() else { return };
    menu.dirty = false;
    let page = menu.stack.last().cloned().filter(|_| menu.open);
    let (history, names, sel) = match &page {
        Some((page, sel)) => {
            let here = Here::new(&garage, &game, player.single().ok());
            let list = rows(&menu, page, &garage, &mods, &mod_sel, &here);
            let first = sel.saturating_sub(WINDOW - 1).min(list.len().saturating_sub(WINDOW));
            let names = list.into_iter().skip(first).take(WINDOW).map(|(l, _)| l).collect::<Vec<_>>();
            let history = menu
                .stack
                .iter()
                .map(|(p, _)| match p {
                    Page::Tier(i) => menu.tiers[*i].title.clone(),
                    Page::Models(make) => make.clone(),
                    // The titles of the game's `TO_OPT` tier and of the unnamed tier after it.
                    Page::ModClass(_) => "MOD TYPE".into(),
                    Page::ModType(..) => "MOD STANDARD".into(),
                    Page::RaceActions(_) => menu
                        .tiers
                        .iter()
                        .find(|t| t.transition == "TO_FMN_OPT")
                        .map_or_else(String::new, |t| t.title.clone()),
                })
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>();
            (history, names, sel - first)
        }
        None => Default::default(),
    };
    let open = page.is_some();
    let Some(look) = menu.look.as_mut() else { return };
    let Some(root) = look.root else { return };
    let quads = if open {
        look.scene.draw([window.width(), window.height()], &mut |p, a| lookup(p, a, &history, &names, sel))
    } else {
        Vec::new()
    };
    let Look { pool, textures, .. } = look;
    crate::ui_quads::show_quads(&mut commands, root, pool, &quads, textures, None, &mut nodes, &mut materials);
}
