//! The pause menu (screen 431082, `PAUSE`), drawn from the game's layout with the game's offline items.
//!
//! Data: the `Menu` widget's layout 431094 (the frame, the red "PAUSE" title and an element stack over
//! `Widget.Menu.Items` whose selected row uses the template `Widget.Menu.SelectedIndex(EQUALS:#)`); the
//! offline item list is the `StdData` `Widget_MenuData` (431091): RESUME "CONTINUE", MWL "DRIVER DETAILS", MAP,
//! MPLAYER "MULTIPLAYER", FRIENDS, TO_SETTINGS "SETTINGS", STORE. The widget's actions: previous / next, the
//! current item's `Action` through `ExitGroupVia`, and RESUME on cancel. Controls: the original's PAUSE (Esc,
//! pad Start) opens it; the widget's user actions (decomp.md "Menu input and the map screen's controls") move with
//! `menu_up` / `menu_down` (arrows, left stick, d-pad; auto-repeat 0.2 s then every 0.1 s), choose with `accept`
//! (Enter, A) and resume with `cancel` (Backspace, B) or `start` (Esc, Start).
//! The game pauses (`Game Paused Behaviour`): our virtual clock stops while it is open.
//!
//! What works here: CONTINUE; MAP (`SHOW_MAP`) opens the map screen 899991 (`map_screen.rs`; until that is
//! loaded, a list of the offline races that starts the chosen one); SETTINGS opens our volume settings.
//! DRIVER DETAILS, MULTIPLAYER, FRIENDS and STORE are online / Autolog screens: shown inactive. Model decisions:
//! the inactive rows' look (the row template's `Active` field), the race list page, the list wrapping around.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mw_assets::frontend::PAUSE_SCREEN;
use mw_assets::ui::{DrawOptions, UiAnim, UiScene};
use mw_bundle::ResourceId;
use mw_formats::binding::BindValue;

use crate::game_text::{self, GameFonts, LoadedFont};
use crate::race::RaceSetup;
use crate::ui_quads::{AdditiveQuadMaterial, PoolNode, QuadNode, QuadPool, show_quads};

/// The `Menu` widget's frame layout.
const MENU_LAYOUT: u32 = 431_094;
/// The offline items (`StdData`).
const STD_DATA: u32 = 431_091;
/// Rows shown at once (the stack's `Widget.Menu.Menu.NumItemsShown` is set by code; model decision).
const WINDOW: usize = 9;

pub struct PausePlugin {
    pub enabled: bool,
    pub open: bool,
    /// Open on the MAP page (`--pause-map`, for screenshots).
    pub map: bool,
}

impl Plugin for PausePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PauseMenu { enabled: self.enabled, open: self.open, open_map: self.map, ..default() })
            .add_systems(Startup, start_load)
            .add_systems(Update, (poll_load, navigate, draw).chain());
    }
}

#[derive(Clone)]
struct Item {
    action: String,
    label: String,
    active: bool,
}

#[derive(Clone)]
enum Page {
    Root,
    /// The offline races (MAP).
    Races,
}

#[derive(Resource, Default)]
pub struct PauseMenu {
    pub enabled: bool,
    pub open: bool,
    /// Closed this frame (other menus must not take the same Esc).
    pub just_closed: bool,
    items: Vec<Item>,
    races: Vec<Arc<RaceSetup>>,
    page: Option<Page>,
    selected: usize,
    job: Option<Arc<Mutex<Option<Loaded>>>>,
    look: Option<Look>,
    /// The virtual clock was running when the menu opened.
    unpause: bool,
    open_map: bool,
}

struct Loaded {
    items: Vec<Item>,
    races: Vec<Arc<RaceSetup>>,
    scene: UiScene,
    images: Vec<(ResourceId, Image)>,
    fonts: Vec<(u32, mw_formats::Font)>,
}

struct Look {
    scene: UiScene,
    textures: HashMap<ResourceId, (Handle<Image>, Vec2)>,
    root: Entity,
    pool: QuadPool,
    anim: UiAnim,
}

fn start_load(mut menu: ResMut<PauseMenu>, game: Res<crate::Game>) {
    if !menu.enabled {
        return;
    }
    let job = Arc::new(Mutex::new(None));
    let (out, store) = (job.clone(), game.store.clone());
    std::thread::spawn(move || {
        let Ok(lang) = store.language(mw_assets::DEFAULT_LANGUAGE) else { return };
        let text = |id: u32| lang.get(id).unwrap_or_default().to_uppercase();
        // Online / Autolog destinations are inactive offline (ours).
        let supported = ["RESUME", "MAP", "TO_SETTINGS"];
        let items = store
            .menu_items(PAUSE_SCREEN, STD_DATA)
            .map_err(|e| warn!("pause menu items: {e}"))
            .unwrap_or_default()
            .into_iter()
            .map(|(action, name, active)| Item {
                label: text(name),
                active: active && supported.contains(&action.as_str()),
                action,
            })
            .collect();
        let races = store
            .events()
            .map(|events| {
                events
                    .events
                    .iter()
                    .filter(|e| e.game_mode == 730_000 && !e.checkpoints.is_empty() && e.spawn.is_some())
                    .map(|e| {
                        Arc::new(RaceSetup {
                            event: e.clone(),
                            game_mode: events.game_mode(e.game_mode).copied(),
                            name: lang.get(e.name).map_or_else(|| format!("EVENT {}", e.id), str::to_uppercase),
                            test: false,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let layouts = store
            .ui_screen_layouts(PAUSE_SCREEN, Some(&["Menu"]))
            .unwrap_or_default()
            .into_iter()
            .filter(|l| l.layout.id == MENU_LAYOUT)
            .take(1)
            .collect();
        let mut scene = store.ui_scene(layouts, &lang);
        // A menu screen: letterboxed like the original (`UiFrame::letterbox`).
        scene.letterbox = true;
        let images =
            scene.textures().into_iter().filter_map(|id| Some((id, game_text::decode_texture(&store, id)?))).collect();
        let fonts = scene.fonts.iter().map(|(&n, f)| (n, f.clone())).collect();
        *out.lock().unwrap() = Some(Loaded { items, races, scene, images, fonts });
    });
    menu.job = Some(job);
}

fn poll_load(
    mut commands: Commands,
    mut menu: ResMut<PauseMenu>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<GameFonts>,
) {
    let Some(loaded) = menu.job.as_ref().and_then(|j| j.lock().unwrap().take()) else { return };
    menu.job = None;
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
    let root = commands.spawn((node, Pickable::IGNORE, GlobalZIndex(15))).id();
    info!("pause menu: {} items, {} races", loaded.items.len(), loaded.races.len());
    menu.items = loaded.items;
    menu.races = loaded.races;
    menu.look = Some(Look { scene: loaded.scene, textures, root, pool: QuadPool::default(), anim: UiAnim::default() });
    if menu.open {
        menu.open = false;
        open(&mut menu, None);
        if menu.open_map {
            menu.page = Some(Page::Races);
        }
    }
}

fn open(menu: &mut PauseMenu, time: Option<&mut Time<Virtual>>) {
    menu.open = true;
    menu.page = Some(Page::Root);
    menu.selected = 0;
    if let Some(t) = time {
        menu.unpause = !t.is_paused();
        t.pause();
    }
}

fn close(menu: &mut PauseMenu, time: &mut Time<Virtual>) {
    menu.open = false;
    menu.page = None;
    menu.just_closed = true;
    if std::mem::take(&mut menu.unpause) {
        time.unpause();
    }
}

/// The rows of the current page: (label, active).
fn rows(menu: &PauseMenu) -> Vec<(String, bool)> {
    match menu.page {
        Some(Page::Races) => menu.races.iter().map(|r| (r.name.clone(), true)).collect(),
        _ => menu.items.iter().map(|i| (i.label.clone(), i.active)).collect(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn navigate(
    mut menu: ResMut<PauseMenu>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut time: ResMut<Time<Virtual>>,
    mut settings: ResMut<crate::settings::SettingsMenu>,
    mut race_start: MessageWriter<crate::event_card::PlayEvent>,
    front: Option<Res<crate::frontend::FrontEnd>>,
    easydrive: Option<Res<crate::easydrive::EasyDrive>>,
    cutscene: Option<Res<crate::cutscene::CutsceneState>>,
    mut map: Option<ResMut<crate::map_screen::MapScreen>>,
    card: Option<Res<crate::event_card::EventCard>>,
    input: Res<crate::input::MenuInput>,
) {
    menu.just_closed = false;
    if !menu.enabled || menu.look.is_none() {
        return;
    }
    if map.as_ref().is_some_and(|m| m.open || m.just_closed) || card.is_some_and(|c| c.showing()) {
        return;
    }
    use crate::input::defaults as act;
    let pad_list: Vec<&Gamepad> = pads.iter().collect();
    let dev = crate::input::Devices { keys: &keys, pads: &pad_list };
    if !menu.open {
        // PAUSE (Esc / Start) while driving; not over the front end's screens, a cutscene or open Easydrive.
        let busy = front.is_some_and(|f| !f.playing())
            || cutscene.is_some_and(|c| c.playing())
            || easydrive.is_some_and(|e| e.open || e.just_closed)
            || settings.open;
        if dev.just_pressed(act::PAUSE) && !busy {
            open(&mut menu, Some(&mut time));
        }
        return;
    }
    // The `Menu` widget's user actions (431082): menu_up / menu_down (repeating) → previous / next, accept → the
    // item's action, cancel and start → RESUME. Directions read the arrows, the left stick and the d-pad.
    use crate::input::MenuAction as M;
    let n = rows(&menu).len().max(1);
    let back = input.pressed(M::Cancel);
    if input.pressed(M::Start) || (back && matches!(menu.page, Some(Page::Root))) {
        close(&mut menu, &mut time);
        return;
    }
    if back {
        menu.page = Some(Page::Root);
        menu.selected = menu.items.iter().position(|i| i.action == "MAP").unwrap_or(0);
        return;
    }
    if input.fired(M::Up) {
        menu.selected = (menu.selected + n - 1) % n;
    }
    if input.fired(M::Down) {
        menu.selected = (menu.selected + 1) % n;
    }
    if input.pressed(M::Accept) {
        match menu.page.clone() {
            Some(Page::Races) => {
                if let Some(r) = menu.races.get(menu.selected).cloned() {
                    close(&mut menu, &mut time);
                    race_start.write(crate::event_card::PlayEvent(r));
                }
            }
            _ => match menu.items.get(menu.selected).filter(|i| i.active).map(|i| i.action.clone()).as_deref() {
                Some("RESUME") => close(&mut menu, &mut time),
                // `SHOW_MAP`: the map screen (899991); our race list while it is not loaded.
                Some("MAP") => match map.as_mut().filter(|m| m.ready()) {
                    Some(m) => {
                        let unpause = std::mem::take(&mut menu.unpause);
                        menu.open = false;
                        menu.page = None;
                        menu.just_closed = true;
                        m.open(&mut time, unpause);
                    }
                    None => {
                        menu.page = Some(Page::Races);
                        menu.selected = 0;
                    }
                },
                Some("TO_SETTINGS") => {
                    close(&mut menu, &mut time);
                    settings.open = true;
                }
                _ => {}
            },
        }
    }
}

/// Values for the frame's bindings (`Widget.Menu.*`; inside a row the item's fields arrive as
/// `Widget.Menu.Items[<row>].<field>`).
fn lookup(path: &str, arg: Option<&str>, rows: &[(String, bool)], selected: usize) -> Option<BindValue> {
    let field = |row: Option<&(String, bool)>, f: &str| -> Option<BindValue> {
        let (label, active) = row?;
        Some(match f {
            "Name" | "Label" | "DisplayName" => BindValue::Text(label.clone()),
            "Active" => (*active).into(),
            "Action" => BindValue::Text(String::new()),
            _ => return None,
        })
    };
    if let Some(f) = path.strip_prefix("Widget.Menu.CurrentItem.") {
        return field(rows.get(selected), f);
    }
    if let Some(rest) = path.strip_prefix("Widget.Menu.Items[") {
        let (i, f) = rest.split_once("].")?;
        return field(rows.get(i.parse::<usize>().ok()?), f);
    }
    if path == "Widget.Menu.Items" {
        // `Widget.Menu.Items[#parent_index#].Action(...)`: the argument carries the index and the field.
        let a = arg?;
        let (i, f) = a.split_once("].")?;
        return field(rows.get(i.trim().parse::<usize>().ok()?), f);
    }
    Some(match path {
        "Widget.Menu.Items._length" | "Widget.Menu.Menu.Length" => (rows.len() as f64).into(),
        "Widget.Menu.Menu.NumItemsShown" => (rows.len().min(WINDOW) as f64).into(),
        "Widget.Menu.SelectedIndex" => (selected as f64).into(),
        "Widget.Menu.Menu.CanSelectPrevious" | "Widget.Menu.Menu.CanSelectNext" => true.into(),
        "Mode.IsInGameIntro" => false.into(),
        "Signals.True" => true.into(),
        "Signals.False" => false.into(),
        _ => return None,
    })
}

#[allow(clippy::too_many_arguments)]
fn draw(
    mut commands: Commands,
    mut menu: ResMut<PauseMenu>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    time: Res<Time<Real>>,
    mut nodes: Query<PoolNode, With<QuadNode>>,
    mut materials: ResMut<Assets<AdditiveQuadMaterial>>,
) {
    let Ok(window) = windows.single() else { return };
    let all = rows(&menu);
    let first = menu.selected.saturating_sub(WINDOW - 1).min(all.len().saturating_sub(WINDOW));
    let shown: Vec<(String, bool)> = all.iter().skip(first).take(WINDOW).cloned().collect();
    let sel = menu.selected - first;
    let open = menu.open;
    let Some(look) = menu.look.as_mut() else { return };
    let quads = if open {
        let mut options = DrawOptions::new(None, Some(&mut look.anim), time.elapsed_secs_f64());
        let mut misses = Vec::new();
        let q = look.scene.draw_with(
            [window.width(), window.height()],
            &mut |p, a| {
                let v = lookup(p, a, &shown, sel);
                if v.is_none() {
                    misses.push(format!("{p}[{a:?}]"));
                }
                v
            },
            &mut options,
        );
        if std::env::var_os("MW_PAUSE_BINDINGS").is_some() {
            misses.sort();
            misses.dedup();
            info!("pause menu: unknown bindings {misses:?}");
        }
        let mut q = q;
        // The black bars outside the letterboxed frame (`FUN_006240b0`).
        let screen = [window.width(), window.height()];
        q.extend(mw_assets::ui::UiFrame::letterbox(screen).bars(screen));
        q
    } else {
        look.anim.reset();
        Vec::new()
    };
    let Look { root, pool, textures, .. } = look;
    show_quads(&mut commands, *root, pool, &quads, textures, None, &mut nodes, &mut materials);
}
