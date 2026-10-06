//! Easydrive, the game's in-drive d-pad menu (first version). The tree comes from the driving screen's data
//! (`mw_assets::easydrive`, research log 2026-10-06 session 5): tiers, their link / hook items and labels
//! from the language table. Link items open the tier their token names; the jack-spot tier (the one that
//! has a `JSMANUFACT` entry source) lists the catalogue's manufacturers, then their models, and drives the
//! chosen car through the garage (`GarageAction::Drive`). Hook items and tiers whose entries the game's code
//! generates are shown and inert.
//!
//! Model decisions (not from the data or code yet): the panel's place and look (the game's layouts
//! `01000000001b0f44` & co. are not read yet) and the keyboard keys. The data binds the `EasyDrive::*` actions to
//! the input mappings `easy_up` (previous), `easy_down` (next), `easy_left` (back a tier), `easy_right` (select;
//! `Hack_EasyRight` opens) and `cancel` (quick close); which keys those mappings are on PC is not decoded, so
//! here: the original's EASY_* bindings (d-pad; Num 4 / 6 / 8 / 2, input.rs), Esc / pad B to close, `E` opens (ours).

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mw_assets::easydrive::{self as ed, EASYDRIVE_SCREEN};

use crate::Game;
use crate::game_text::{BODY_FONT, GameText, TITLE_FONT};
use crate::garage::{Garage, GarageAction};

pub struct EasyDrivePlugin {
    pub enabled: bool,
    /// Open at startup (`--show-easydrive`, for screenshots).
    pub open: bool,
}

impl Plugin for EasyDrivePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(EasyDrive { enabled: self.enabled, open: self.open, ..default() })
            .add_systems(Startup, (start_load, spawn_panel))
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
}

#[derive(Resource, Default)]
pub struct EasyDrive {
    enabled: bool,
    pub open: bool,
    tiers: Vec<Tier>,
    job: Option<Arc<Mutex<Option<Vec<Tier>>>>>,
    /// Open pages, root first, with the selected row of each.
    stack: Vec<(Page, usize)>,
    dirty: bool,
    /// Closed by Esc this frame: the car menu must not take the same key press as "pause".
    pub just_closed: bool,
}

#[derive(Component)]
struct Panel;

fn start_load(mut menu: ResMut<EasyDrive>, game: Res<Game>) {
    if !menu.enabled && !menu.open {
        return;
    }
    let job = Arc::new(Mutex::new(None));
    let (out, store) = (job.clone(), game.store.clone());
    std::thread::spawn(move || {
        let tiers = store
            .language(mw_assets::DEFAULT_LANGUAGE)
            .and_then(|lang| Ok((store.easydrive(EASYDRIVE_SCREEN)?, lang)))
            .map(|(m, lang)| resolve(&m, &lang))
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

fn poll_load(mut menu: ResMut<EasyDrive>) {
    let Some(tiers) = menu.job.as_ref().and_then(|j| j.lock().unwrap().take()) else { return };
    menu.job = None;
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
    }
    menu.dirty = true;
}

/// What a row does when chosen.
enum Choice {
    Tier(usize),
    Make(String),
    Car(usize),
    Inert,
}

/// The rows of a page: (label, enabled, choice).
fn rows(menu: &EasyDrive, page: &Page, garage: &Garage) -> Vec<(String, Choice)> {
    match page {
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

fn navigate(
    mut menu: ResMut<EasyDrive>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    garage: Res<Garage>,
    mut actions: MessageWriter<GarageAction>,
) {
    if !menu.enabled || menu.tiers.is_empty() {
        return;
    }
    // The original's EASY_* actions (Num 4 / 6 / 8 / 2, d-pad; input.rs). EASY_OPEN has no default binding in the
    // original's tables; opening with "right" is the console layout, `E` is ours for keyboards without a numpad.
    use crate::input::defaults as act;
    let pad_list: Vec<&Gamepad> = pads.iter().collect();
    let dev = crate::input::Devices { keys: &keys, pads: &pad_list };
    let (up, down) = (dev.just_pressed(act::EASY_UP), dev.just_pressed(act::EASY_DOWN));
    let (left, right) = (dev.just_pressed(act::EASY_LEFT), dev.just_pressed(act::EASY_RIGHT));
    menu.just_closed = false;
    if !menu.open {
        if keys.just_pressed(KeyCode::KeyE) || right {
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
    let list = rows(&menu, &page, &garage);
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
            Some(Choice::Inert) | None => {}
        }
        menu.dirty = true;
    }
}

fn spawn_panel(mut commands: Commands) {
    commands.spawn((
        Panel,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(4.0),
            top: Val::Percent(28.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            padding: UiRect::all(Val::Px(8.0)),
            display: Display::None,
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
        GlobalZIndex(5),
    ));
}

/// Rows shown at once (model decision; the script's `Widget.EasyDrive.Menu.NumItemsShown` is set by code).
const WINDOW: usize = 8;

fn redraw(
    mut commands: Commands,
    mut menu: ResMut<EasyDrive>,
    garage: Res<Garage>,
    mut panel: Query<(Entity, &mut Node), With<Panel>>,
) {
    if !menu.dirty && !garage.is_changed() {
        return;
    }
    menu.dirty = false;
    let Ok((entity, mut node)) = panel.single_mut() else { return };
    commands.entity(entity).despawn_related::<Children>();
    let Some((page, sel)) = menu.stack.last().cloned().filter(|_| menu.open) else {
        node.display = Display::None;
        return;
    };
    node.display = Display::Flex;
    let title = match &page {
        Page::Tier(i) => menu.tiers[*i].title.clone(),
        Page::Models(make) => make.clone(),
    };
    let list = rows(&menu, &page, &garage);
    let first = sel.saturating_sub(WINDOW - 1).min(list.len().saturating_sub(WINDOW));
    commands.entity(entity).with_children(|p| {
        if !title.is_empty() {
            p.spawn(GameText::new(title.to_uppercase(), 30.0, Color::WHITE).with_font(TITLE_FONT));
        }
        for (i, (label, choice)) in list.iter().enumerate().skip(first).take(WINDOW) {
            let selected = i == sel;
            let color = match (selected, choice) {
                (true, _) => Color::BLACK,
                (false, Choice::Inert) => Color::srgb(0.55, 0.55, 0.58),
                _ => Color::WHITE,
            };
            let back = if selected { Color::srgb(1.0, 0.55, 0.1) } else { Color::NONE };
            p.spawn((Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(2.0)), ..default() }, BackgroundColor(back)))
                .with_children(|r| {
                    r.spawn(GameText::new(label.to_uppercase(), 22.0, color).with_font(BODY_FONT));
                });
        }
    });
}
