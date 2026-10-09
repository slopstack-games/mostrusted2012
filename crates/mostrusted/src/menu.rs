//! In-game car menu (native Bevy UI): pick a car from the catalogue and a paint, then drive it in
//! place of the current car or below the free camera; or leave the car for the free camera.
//!
//! Toggle with Tab / F1 or the gamepad's Start / Select. Keyboard: Up/Down (PageUp/PageDown,
//! Home/End) select, Left/Right paint, Enter drive, F2 free camera, Esc close, typing filters the
//! list. Gamepad: D-pad or left stick select, D-pad left/right paint, LB/RB page, A drive, Y free
//! camera, B close. Mouse: wheel scrolls, click selects, clicking the selected row drives.
//! The actions themselves live in `garage.rs`.

use bevy::camera_controller::free_camera::FreeCameraState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;

use crate::Game;
use crate::garage::{CatalogueEntry, Garage, GarageAction};

pub struct MenuPlugin {
    /// Whether the menu can be opened (not in screenshot, test or vehicle-viewer runs).
    pub enabled: bool,
    /// Open it at startup (`--show-menu`, for screenshots of the menu).
    pub open: bool,
}

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CarMenu { open: self.open, enabled: self.enabled, preselect: true, ..default() })
            .add_systems(Startup, spawn_menu)
            .add_systems(Update, (toggle_menu, menu_input, refresh_menu, free_camera_focus).chain())
            .add_systems(Update, freeze_car_input.after(crate::drive::read_input));
    }
}

/// Visible rows of the car list.
const ROWS: usize = 12;
/// Paint swatches shown (cars have up to about 20 paints).
const SWATCHES: usize = 24;
/// Seconds a garage message stays on screen after the menu closes.
const TOAST_SECS: f32 = 4.0;

const ACCENT: Color = Color::srgb(1.0, 0.55, 0.1);
const PANEL: Color = Color::srgba(0.04, 0.05, 0.07, 0.93);
const TEXT: Color = Color::srgb(0.9, 0.9, 0.92);
const DIM: Color = Color::srgb(0.55, 0.57, 0.62);
const ROW_SELECTED: Color = Color::srgba(1.0, 0.55, 0.1, 0.35);
const ROW_HOVER: Color = Color::srgba(1.0, 1.0, 1.0, 0.08);
const BUTTON: Color = Color::srgba(1.0, 1.0, 1.0, 0.07);
const BUTTON_HOVER: Color = Color::srgba(1.0, 0.55, 0.1, 0.45);

#[derive(Resource, Default)]
pub struct CarMenu {
    pub open: bool,
    enabled: bool,
    filter: String,
    /// Position in the filtered list, and the first visible row.
    selected: usize,
    scroll: usize,
    paint: usize,
    /// Bundle of the selected car, to reset the paint when the selection changes.
    selected_bundle: Option<String>,
    /// Gamepad auto-repeat: held direction and seconds until it fires again.
    repeat: (i32, f32),
    /// Select the car being driven (with its paint) once it and the catalogue are known.
    preselect: bool,
}

#[derive(Component)]
struct MenuRoot;

/// The message box at the bottom (container, and its text).
#[derive(Component)]
struct Toast;

#[derive(Component)]
struct ToastText;

#[derive(Component, Clone, Copy, PartialEq, Debug)]
enum MenuButton {
    Row(usize),
    Swatch(usize),
    PaintPrev,
    PaintNext,
    Drive,
    Free,
    Close,
}

#[derive(Component, Clone, Copy, PartialEq)]
enum MenuText {
    Current,
    Search,
    Make(usize),
    Model(usize),
    Year(usize),
    Range,
    Paint,
    Status,
}

/// What an input asks the menu to do.
#[derive(Debug, Clone, Copy)]
enum Nav {
    Move(i32),
    Home,
    End,
    Select(usize),
    Scroll(i32),
    Paint(i32),
    SetPaint(usize),
    Activate,
    Free,
    Close,
}

fn font(size: f32) -> TextFont {
    TextFont { font_size: FontSize::Px(size), ..default() }
}

fn spawn_menu(mut commands: Commands, menu: Res<CarMenu>) {
    let label = |text: &str, size: f32, color: Color| (Text::new(text), font(size), TextColor(color));
    let root = commands
        .spawn((
            MenuRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                display: if menu.open { Display::Flex } else { Display::None },
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.25)),
            GlobalZIndex(10),
        ))
        .id();
    let panel = commands
        .spawn((
            Node {
                width: Val::Px(640.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(16.0)),
                row_gap: Val::Px(6.0),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(PANEL),
            BorderColor::all(ACCENT),
            ChildOf(root),
        ))
        .id();
    // Title and the car being driven.
    let title = commands
        .spawn((
            Node { justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::End, ..default() },
            ChildOf(panel),
        ))
        .id();
    commands.spawn((label("CAR SELECT", 24.0, ACCENT), ChildOf(title)));
    commands.spawn((label("", 14.0, DIM), MenuText::Current, ChildOf(title)));
    commands.spawn((label("", 16.0, TEXT), MenuText::Search, ChildOf(panel)));

    // The list: make, model, year.
    let list = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(1.0),
                padding: UiRect::vertical(Val::Px(4.0)),
                border: UiRect::vertical(Val::Px(1.0)),
                ..default()
            },
            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.12)),
            ChildOf(panel),
        ))
        .id();
    for i in 0..ROWS {
        let row = commands
            .spawn((
                Button,
                MenuButton::Row(i),
                Node {
                    height: Val::Px(24.0),
                    align_items: AlignItems::Center,
                    padding: UiRect::horizontal(Val::Px(8.0)),
                    border_radius: BorderRadius::all(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(Color::NONE),
                ChildOf(list),
            ))
            .id();
        // Columns are sized by wrapper nodes (a sized text node draws its glyphs offset).
        let column = |commands: &mut Commands, node: Node| commands.spawn((node, ChildOf(row))).id();
        let make = column(&mut commands, Node { width: Val::Px(170.0), ..default() });
        let model = column(&mut commands, Node { flex_grow: 1.0, ..default() });
        commands.spawn((label("", 16.0, DIM), MenuText::Make(i), ChildOf(make)));
        commands.spawn((label("", 16.0, TEXT), MenuText::Model(i), ChildOf(model)));
        commands.spawn((label("", 16.0, DIM), MenuText::Year(i), ChildOf(row)));
    }
    commands.spawn((label("", 12.0, DIM), MenuText::Range, ChildOf(panel)));

    // Paint: arrows, "n / m", swatches.
    let paint = commands
        .spawn((Node { align_items: AlignItems::Center, column_gap: Val::Px(6.0), ..default() }, ChildOf(panel)))
        .id();
    let small_button = |commands: &mut Commands, text: &str, button: MenuButton, parent: Entity| {
        commands
            .spawn((
                Button,
                button,
                Node {
                    padding: UiRect::axes(Val::Px(8.0), Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(BUTTON),
                ChildOf(parent),
            ))
            .with_child(label(text, 16.0, TEXT));
    };
    small_button(&mut commands, "<", MenuButton::PaintPrev, paint);
    let paint_label = commands
        .spawn((Node { width: Val::Px(120.0), justify_content: JustifyContent::Center, ..default() }, ChildOf(paint)))
        .id();
    commands.spawn((label("", 16.0, TEXT), MenuText::Paint, ChildOf(paint_label)));
    small_button(&mut commands, ">", MenuButton::PaintNext, paint);
    let swatches = commands
        .spawn((
            Node { column_gap: Val::Px(4.0), row_gap: Val::Px(4.0), flex_wrap: FlexWrap::Wrap, ..default() },
            ChildOf(panel),
        ))
        .id();
    for i in 0..SWATCHES {
        commands.spawn((
            Button,
            MenuButton::Swatch(i),
            Node {
                width: Val::Px(20.0),
                height: Val::Px(20.0),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(Val::Px(3.0)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::BLACK),
            BorderColor::all(Color::NONE),
            ChildOf(swatches),
        ));
    }

    // Actions.
    let actions = commands
        .spawn((Node { column_gap: Val::Px(8.0), margin: UiRect::top(Val::Px(6.0)), ..default() }, ChildOf(panel)))
        .id();
    for (text, button) in [
        ("Drive (Enter/A)", MenuButton::Drive),
        ("Free camera (F2/Y)", MenuButton::Free),
        ("Close (Tab/B)", MenuButton::Close),
    ] {
        commands
            .spawn((
                Button,
                button,
                Node {
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                    border_radius: BorderRadius::all(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(BUTTON),
                ChildOf(actions),
            ))
            .with_child(label(text, 15.0, TEXT));
    }
    commands.spawn((label("", 13.0, ACCENT), MenuText::Status, ChildOf(panel)));
    commands.spawn((
        label(
            "Up/Down select, Left/Right paint, type to search, wheel scrolls. Gamepad: D-pad, LB/RB page.",
            12.0,
            DIM,
        ),
        ChildOf(panel),
    ));

    // Loading / result message while the menu is closed.
    let toast = commands
        .spawn((
            Toast,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(24.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                display: Display::None,
                ..default()
            },
            GlobalZIndex(9),
        ))
        .id();
    commands
        .spawn((
            Node {
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(PANEL),
            ChildOf(toast),
        ))
        .with_child((ToastText, label("", 16.0, TEXT)));
}

/// Tab / F1 / Start / Select open and close the menu.
#[allow(clippy::too_many_arguments)]
fn toggle_menu(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut menu: ResMut<CarMenu>,
    garage: Res<Garage>,
    easydrive: Option<Res<crate::easydrive::EasyDrive>>,
    cutscene: Option<Res<crate::cutscene::CutsceneState>>,
    pause: Option<Res<crate::pause::PauseMenu>>,
    front: Option<Res<crate::frontend::FrontEnd>>,
) {
    // Esc and Back are SKIP_CUTSCENE while a sequence plays; nothing opens over the front end's screens.
    if !menu.enabled || cutscene.is_some_and(|c| c.playing()) || front.is_some_and(|f| !f.playing()) {
        return;
    }
    // With the game's pause menu (`pause.rs`), PAUSE (Esc / Start) is its key; Tab / F1 / pad Back stay ours.
    let pause_menu = pause.as_ref().is_some_and(|p| p.enabled);
    if pause.is_some_and(|p| p.open || p.just_closed) {
        return;
    }
    // Esc belongs to Easydrive while that is open (it closes it).
    let easy_esc = keys.just_pressed(KeyCode::Escape) && easydrive.is_some_and(|e| e.open || e.just_closed);
    // Ours: Tab / F1. The original's PAUSE (Esc / Start) and CHANGE_VEHICLE (pad Back) also open it (input.rs);
    // Esc only opens, since the open menu closes on Esc itself.
    use crate::input::defaults as act;
    let pad_list: Vec<&Gamepad> = pads.iter().collect();
    let dev = crate::input::Devices { keys: &keys, pads: &pad_list };
    let pressed = keys.any_just_pressed([KeyCode::Tab, KeyCode::F1])
        || (dev.just_pressed(act::PAUSE)
            && !pause_menu
            && !easy_esc
            && !(menu.open && keys.just_pressed(KeyCode::Escape)))
        || dev.just_pressed(act::CHANGE_VEHICLE);
    if !pressed {
        return;
    }
    menu.open = !menu.open;
    if menu.open {
        menu.filter.clear();
        menu.selected = menu.selected.min(garage.catalogue.len().saturating_sub(1));
        menu.preselect = true;
    }
}

/// Start at the car being driven, with its paint.
fn preselect_current(menu: &mut CarMenu, garage: &Garage) {
    let Some(current) = &garage.current else { return };
    let Some(at) = garage.catalogue.iter().position(|e| e.bundle == current.bundle) else { return };
    menu.preselect = false;
    if !menu.filter.is_empty() {
        return;
    }
    menu.selected = at;
    menu.selected_bundle = Some(current.bundle.clone());
    menu.paint = current.paint_index;
    menu.scroll = at.saturating_sub(ROWS / 2).min(garage.catalogue.len().saturating_sub(ROWS));
}

/// Indices into the catalogue matching every word of the filter.
fn filtered(catalogue: &[CatalogueEntry], filter: &str) -> Vec<usize> {
    let words: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
    catalogue
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            let hay = format!("{} {}", e.name, e.year).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .map(|(i, _)| i)
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn menu_input(
    mut menu: ResMut<CarMenu>,
    mut garage: ResMut<Garage>,
    game: Res<Game>,
    mut keyboard: MessageReader<KeyboardInput>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    scroll: Res<AccumulatedMouseScroll>,
    buttons: Query<(&Interaction, &MenuButton), Changed<Interaction>>,
    time: Res<Time<Real>>,
    mut actions: MessageWriter<GarageAction>,
) {
    let typed: Vec<KeyboardInput> = keyboard.read().cloned().collect();
    if !menu.open {
        menu.repeat = (0, 0.0);
        return;
    }
    if menu.preselect {
        preselect_current(&mut menu, &garage);
    }
    // `--show-menu` in a screenshot run: shown, but deaf to input so the capture is deterministic.
    if !menu.enabled {
        return;
    }
    let mut navs = Vec::new();
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let mut filter_changed = false;
    for k in typed.iter().filter(|k| k.state.is_pressed()) {
        match &k.logical_key {
            Key::ArrowUp => navs.push(Nav::Move(-1)),
            Key::ArrowDown => navs.push(Nav::Move(1)),
            Key::PageUp => navs.push(Nav::Move(-(ROWS as i32))),
            Key::PageDown => navs.push(Nav::Move(ROWS as i32)),
            Key::Home => navs.push(Nav::Home),
            Key::End => navs.push(Nav::End),
            Key::ArrowLeft => navs.push(Nav::Paint(-1)),
            Key::ArrowRight => navs.push(Nav::Paint(1)),
            Key::Enter => navs.push(Nav::Activate),
            Key::F2 => navs.push(Nav::Free),
            Key::Escape => navs.push(Nav::Close),
            Key::Backspace => filter_changed |= menu.filter.pop().is_some(),
            Key::Space if !menu.filter.is_empty() => {
                menu.filter.push(' ');
                filter_changed = true;
            }
            Key::Character(s) if !ctrl => {
                for c in s.chars().filter(|c| c.is_ascii_graphic()) {
                    if menu.filter.len() < 32 {
                        menu.filter.push(c);
                        filter_changed = true;
                    }
                }
            }
            _ => {}
        }
    }
    if filter_changed {
        menu.selected = 0;
        menu.scroll = 0;
    }

    // Gamepad: buttons once, directions with auto-repeat.
    let mut dir = 0;
    for pad in &pads {
        let pressed = |b| pad.just_pressed(b);
        if pressed(GamepadButton::South) {
            navs.push(Nav::Activate);
        }
        if pressed(GamepadButton::North) {
            navs.push(Nav::Free);
        }
        if pressed(GamepadButton::East) {
            navs.push(Nav::Close);
        }
        if pressed(GamepadButton::DPadLeft) {
            navs.push(Nav::Paint(-1));
        }
        if pressed(GamepadButton::DPadRight) {
            navs.push(Nav::Paint(1));
        }
        if pressed(GamepadButton::LeftTrigger) {
            navs.push(Nav::Move(-(ROWS as i32)));
        }
        if pressed(GamepadButton::RightTrigger) {
            navs.push(Nav::Move(ROWS as i32));
        }
        let stick = pad.get(GamepadAxis::LeftStickY).unwrap_or(0.0);
        if pad.pressed(GamepadButton::DPadUp) || stick > 0.6 {
            dir = -1;
        } else if pad.pressed(GamepadButton::DPadDown) || stick < -0.6 {
            dir = 1;
        }
    }
    if dir == 0 {
        menu.repeat = (0, 0.0);
    } else if menu.repeat.0 != dir {
        menu.repeat = (dir, 0.35);
        navs.push(Nav::Move(dir));
    } else {
        menu.repeat.1 -= time.delta_secs();
        if menu.repeat.1 <= 0.0 {
            menu.repeat.1 = 0.07;
            navs.push(Nav::Move(dir));
        }
    }

    // Mouse.
    let lines = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 20.0,
    };
    if lines.abs() > 0.01 {
        navs.push(Nav::Scroll((-lines * 3.0).round() as i32));
    }
    for (interaction, button) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        navs.push(match *button {
            MenuButton::Row(i) => Nav::Select(menu.scroll + i),
            MenuButton::Swatch(i) => Nav::SetPaint(i),
            MenuButton::PaintPrev => Nav::Paint(-1),
            MenuButton::PaintNext => Nav::Paint(1),
            MenuButton::Drive => Nav::Activate,
            MenuButton::Free => Nav::Free,
            MenuButton::Close => Nav::Close,
        });
    }
    if navs.is_empty() && !filter_changed {
        return;
    }
    menu.preselect = false;

    let list = filtered(&garage.catalogue, &menu.filter);
    let last = list.len().saturating_sub(1);
    for nav in navs {
        match nav {
            Nav::Move(d) => menu.selected = (menu.selected as i64 + d as i64).clamp(0, last as i64) as usize,
            Nav::Home => menu.selected = 0,
            Nav::End => menu.selected = last,
            // Clicking the selected row drives it.
            Nav::Select(i) if i == menu.selected && i < list.len() => activate(&mut menu, &garage, &list, &mut actions),
            Nav::Select(i) if i < list.len() => menu.selected = i,
            Nav::Select(_) => {}
            Nav::Scroll(d) => {
                let max = list.len().saturating_sub(ROWS);
                menu.scroll = (menu.scroll as i64 + d as i64).clamp(0, max as i64) as usize;
                continue; // the wheel scrolls without moving the selection
            }
            Nav::Paint(d) => menu.paint = (menu.paint as i64 + d as i64).max(0) as usize,
            Nav::SetPaint(i) => menu.paint = i,
            Nav::Activate if !list.is_empty() => activate(&mut menu, &garage, &list, &mut actions),
            Nav::Activate => {}
            Nav::Free => {
                actions.write(GarageAction::FreeCamera);
                menu.open = false;
            }
            Nav::Close => menu.open = false,
        }
        // Keep the selection visible.
        if menu.selected < menu.scroll {
            menu.scroll = menu.selected;
        } else if menu.selected >= menu.scroll + ROWS {
            menu.scroll = menu.selected + 1 - ROWS;
        }
    }
    // A different car starts at its first paint; clamp to the paints it has.
    let bundle = list.get(menu.selected).map(|&i| garage.catalogue[i].bundle.clone());
    if bundle != menu.selected_bundle {
        menu.selected_bundle = bundle.clone();
        menu.paint = 0;
    }
    if let Some(b) = bundle
        && let Some(paints) = garage.paints(&b, &game.store)
    {
        menu.paint = menu.paint.min(paints.len().saturating_sub(1));
    }
}

fn activate(menu: &mut CarMenu, garage: &Garage, list: &[usize], actions: &mut MessageWriter<GarageAction>) {
    let Some(e) = list.get(menu.selected).map(|&i| &garage.catalogue[i]) else { return };
    actions.write(GarageAction::Drive { name: e.name.clone(), bundle: e.bundle.clone(), paint_index: menu.paint });
    menu.open = false;
}

fn set_text(text: &mut Text, value: String) {
    if text.0 != value {
        text.0 = value;
    }
}

type ButtonQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static MenuButton,
        &'static Interaction,
        &'static mut BackgroundColor,
        Option<&'static mut BorderColor>,
        &'static mut Node,
    ),
    (Without<MenuRoot>, Without<Toast>),
>;

/// Show the menu's state.
#[allow(clippy::too_many_arguments)]
fn refresh_menu(
    menu: Res<CarMenu>,
    mut garage: ResMut<Garage>,
    game: Res<Game>,
    time: Res<Time>,
    mut root: Query<&mut Node, (With<MenuRoot>, Without<Toast>)>,
    mut toast: Query<&mut Node, (With<Toast>, Without<MenuRoot>)>,
    mut toast_text: Query<&mut Text, (With<ToastText>, Without<MenuText>)>,
    mut texts: Query<(&MenuText, &mut Text), Without<ToastText>>,
    mut buttons: ButtonQuery,
) {
    if let (Ok(mut node), Ok(mut text)) = (toast.single_mut(), toast_text.single_mut()) {
        let recent = time.elapsed_secs() - garage.status_time < TOAST_SECS;
        let message = match &garage.loading {
            Some(name) => Some(format!("Loading {name}...")),
            None if recent && !garage.status.is_empty() => Some(garage.status.clone()),
            None => None,
        };
        let display = if message.is_some() && !menu.open { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        if let Some(m) = message {
            set_text(&mut text, m);
        }
    }
    let Ok(mut root) = root.single_mut() else { return };
    let display = if menu.open { Display::Flex } else { Display::None };
    if root.display != display {
        root.display = display;
    }
    if !menu.open {
        return;
    }

    let list = filtered(&garage.catalogue, &menu.filter);
    let selected = list.get(menu.selected).map(|&i| garage.catalogue[i].clone());
    let paints: Option<Vec<[f32; 3]>> =
        selected.as_ref().and_then(|e| garage.paints(&e.bundle, &game.store).map(<[_]>::to_vec));
    let current = garage.current.as_ref().map(|c| {
        let name = garage.catalogue.iter().find(|e| e.bundle == c.bundle).map_or(c.bundle.as_str(), |e| &e.name);
        format!("driving {name}, paint {} of {}", c.paint_index + 1, c.paint_count.max(1))
    });
    for (which, mut text) in &mut texts {
        let row = |i: usize| list.get(menu.scroll + i).map(|&c| &garage.catalogue[c]);
        let value = match *which {
            MenuText::Current => current.clone().unwrap_or_else(|| "free camera".into()),
            MenuText::Search if menu.filter.is_empty() => "Search: (type a name)".into(),
            MenuText::Search => format!("Search: {}_", menu.filter),
            MenuText::Make(i) => row(i).map_or(String::new(), |e| e.make.clone()),
            MenuText::Model(i) => row(i).map_or(String::new(), |e| e.model.clone()),
            MenuText::Year(i) => row(i).map_or(String::new(), |e| e.year.to_string()),
            MenuText::Range if garage.catalogue.is_empty() => "reading the car list...".into(),
            MenuText::Range if list.is_empty() => "no car matches".into(),
            MenuText::Range => {
                format!("{}-{} of {} cars", menu.scroll + 1, (menu.scroll + ROWS).min(list.len()), list.len())
            }
            MenuText::Paint => match &paints {
                Some(p) if p.is_empty() => "Paint: default".into(),
                Some(p) => format!("Paint {} / {}", menu.paint + 1, p.len()),
                None => "Paint: ...".into(),
            },
            MenuText::Status => match &garage.loading {
                Some(name) => format!("Loading {name}..."),
                None => garage.status.clone(),
            },
        };
        set_text(&mut text, value);
    }
    for (button, interaction, mut bg, border, mut node) in &mut buttons {
        // Not in screenshot runs (`--show-menu`), where the desktop's cursor may sit over the window.
        let hovered = menu.enabled && *interaction != Interaction::None;
        let (colour, border_colour, display) = match *button {
            MenuButton::Row(i) => {
                let idx = menu.scroll + i;
                let colour = if idx == menu.selected && idx < list.len() {
                    ROW_SELECTED
                } else if hovered && idx < list.len() {
                    ROW_HOVER
                } else {
                    Color::NONE
                };
                (colour, None, Display::Flex)
            }
            MenuButton::Swatch(i) => match paints.as_ref().and_then(|p| p.get(i)) {
                Some(&[r, g, b]) => {
                    let ring = if i == menu.paint {
                        ACCENT
                    } else if hovered {
                        TEXT
                    } else {
                        Color::srgba(1.0, 1.0, 1.0, 0.2)
                    };
                    (Color::linear_rgb(r, g, b), Some(ring), Display::Flex)
                }
                None => (Color::NONE, None, Display::None),
            },
            _ => (if hovered { BUTTON_HOVER } else { BUTTON }, None, Display::Flex),
        };
        if bg.0 != colour {
            bg.0 = colour;
        }
        if let (Some(mut border), Some(c)) = (border, border_colour)
            && border.top != c
        {
            *border = BorderColor::all(c);
        }
        if node.display != display {
            node.display = display;
        }
    }
}

/// The free camera ignores input while the menu is open (its WASD would fight the search box, and
/// it releases the mouse).
fn free_camera_focus(menu: Res<CarMenu>, mut cams: Query<&mut FreeCameraState>) {
    for mut state in &mut cams {
        if state.enabled == menu.open {
            state.enabled = !menu.open;
        }
    }
}

/// The car coasts while the menu is open.
fn freeze_car_input(menu: Res<CarMenu>, mut input: ResMut<crate::drive::CarInput>) {
    if menu.open {
        input.throttle = 0.0;
        input.brake = 0.0;
        input.steer = 0.0;
        input.handbrake = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, year: u16) -> CatalogueEntry {
        CatalogueEntry {
            id: 0,
            make: String::new(),
            model: String::new(),
            name: name.into(),
            year,
            bundle: String::new(),
        }
    }

    #[test]
    fn filter_words() {
        let cars = [entry("Porsche 911 GT2", 2011), entry("Porsche 918 Spyder", 2013), entry("Ford GT", 2006)];
        assert_eq!(filtered(&cars, ""), vec![0, 1, 2]);
        assert_eq!(filtered(&cars, "porsche 911"), vec![0]);
        assert_eq!(filtered(&cars, "gt"), vec![0, 2]);
        assert_eq!(filtered(&cars, "2013"), vec![1]);
    }
}
