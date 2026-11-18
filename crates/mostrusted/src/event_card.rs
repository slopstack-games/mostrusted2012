//! The event-loading card: what the original shows between choosing START RACE on the map and the event's
//! intro. Data: the map options group's (1222205) `ScreenTransition` for the exit `PLAY_EVENT` (1453803) plays
//! sequence 1014321 (8.5 s): a black shape (layout 26458), the conducted card sequence 1014274, and black
//! (layout 1689864) at 8.25–8.5 s; it waits at 0.15 s for the conducted sequence (`JumpMonitorTimelineController`
//! to 8.0 s; our reading of "monitor": the jump is taken once the conducted sequence has played out) and holds at
//! 8.017–8.233 s while `Screen.IsActive(NOT)`.
//!
//! The card (1014274, 8.5 s): black quad, the car pictures cross-fading (conducted 1899171, as in the boot
//! transition, `LoadingTipsConfig.TransitionImages`), the tip strip (1899089, `Transitions.Tip`), the header
//! strips (1675179), the event panel from 0.75 s (1675921: type, name, difficulty, top of speedwall; Most Wanted
//! events use 2041418: their name in the strip and the description large, `Event.Data.IsBlacklistEvent`), the
//! rewards (1014298, `Events.Loading.Reward` …), the Most Wanted list (conducted 2041453) and the speedwall
//! (conducted 1675879). It holds at 7.517–7.733 s while the next screen is not active and sets the general
//! trigger "TRANSITION" at 7.75 s, which the EventIntro context's "Screen Transition Voodoo" waits for before
//! "Run intro sequence": the event's intro starts under the card's last 0.75 s.
//!
//! Here: [`PlayEvent`] starts the card; once it covers the screen the race starts (`race::StartRace`, the
//! player's car moved to the start) with the event's intro and outro (`cutscene.rs`); "screen active" is the race
//! being ready (the opponents placed, the start area resident: `frontend::Residency`); "TRANSITION" releases
//! the intro. Model decisions: the race is started at 0.25 s of the card (the original loads the event behind
//! the transition; when exactly was not traced); the rewards, the Most Wanted list and the speedwall show
//! nothing (no progression or Autolog data), no transition sounds.
//!
//! Driven (`--hsm`, [`crate::race::RaceDriven`]): the card is the chosen menu's transition as above, but the race is
//! the logic's: logic.rs makes the choice the logic's event request and starts the race on the logic's respawn at
//! the event's start; "TRANSITION" is handed to the logic (its general trigger, [`EventCard::transition`]), whose
//! "Screen Transition Voodoo" then plays the intro. The clock the pause menu stopped runs again when the card
//! begins (the logic pauses it itself while the event loads).

use crate::platform::PollLock;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mw_assets::sequence::{ItemKind, Sequence};
use mw_assets::ui::{DrawOptions, PlacedLayout, Quad, UiAnim, UiBlend, UiScene};
use mw_bundle::ResourceId;
use mw_formats::binding::BindValue;

use crate::frontend::{SlotKey, binding_true, sequence_layouts, step};
use crate::game_text::{self, GameFonts, LoadedFont};
use crate::race::{RaceSetup, RaceState, StartRace};
use crate::ui_quads::{AdditiveQuadMaterial, PoolNode, QuadNode, QuadPool, show_quads};

/// The options group's `PLAY_EVENT` transition sequence.
pub const TRANSITION_SEQUENCE: u32 = 1_014_321;
/// Pictures preloaded for the card.
const PICTURES: usize = 3;
/// Card time at which the race is started (model decision, module doc).
const START_AT: f32 = 0.25;
/// The outer sequence's monitor jump: wait here for the card, then go on from `MONITOR_TO`.
const MONITOR_AT: f32 = 0.15;
const MONITOR_TO: f32 = 8.0;

/// Show the event-loading card for this event, then run it.
#[derive(Message, Clone)]
pub struct PlayEvent(pub Arc<RaceSetup>);

pub struct EventCardPlugin;

impl Plugin for EventCardPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<PlayEvent>()
            .init_resource::<EventCard>()
            .add_systems(Update, (begin, receive, advance, draw, shots).chain());
    }
}

/// `MW_CARD_SHOTS=<dir>`: screenshots of the card at 1.5 s, 4 s and 7 s of its clock, of the intro 3 s after the
/// card, then exit (headless checks).
fn shots(
    mut commands: Commands,
    card: Res<EventCard>,
    real: Res<Time<Real>>,
    mut state: Local<(usize, Option<f32>)>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(dir) = std::env::var_os("MW_CARD_SHOTS").map(std::path::PathBuf::from) else { return };
    let (step, done_at) = &mut *state;
    let mut shot = |name: &str| {
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{name}.png"));
        info!("event card: screenshot {}", path.display());
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(path));
    };
    let clock = card.card.as_ref().filter(|c| c.look.is_some()).map(|c| c.clock);
    const AT: [f32; 3] = [1.5, 4.0, 7.0];
    match (clock, *step) {
        (Some(t), s) if s < AT.len() && t >= AT[s] => {
            shot(&format!("card-{:.1}", AT[s]));
            *step += 1;
        }
        (None, s) if s > 0 && s <= AT.len() => {
            *done_at = Some(real.elapsed_secs());
            *step = AT.len() + 1;
        }
        (None, s) if s == AT.len() + 1 && done_at.is_some_and(|d| real.elapsed_secs() - d >= 3.0) => {
            shot("intro");
            *step += 1;
            *done_at = Some(real.elapsed_secs());
        }
        (None, s) if s == AT.len() + 2 && done_at.is_some_and(|d| real.elapsed_secs() - d >= 1.0) => {
            exit.write(AppExit::Success);
            *step += 1;
        }
        _ => {}
    }
}

#[derive(Resource, Default)]
pub struct EventCard {
    card: Option<Card>,
    job: Option<Arc<Mutex<Option<Loaded>>>>,
    root: Option<Entity>,
    pool: QuadPool,
    /// The card's "TRANSITION" general trigger item has passed (set until the next card).
    transition: bool,
}

impl EventCard {
    /// A card is showing (other screens and the pause menu stay closed).
    pub fn showing(&self) -> bool {
        self.card.is_some()
    }

    /// The card's general trigger "TRANSITION" has passed (`GeneralSequence.Triggers["TRANSITION"]`).
    pub fn transition(&self) -> bool {
        self.transition
    }
}

struct Card {
    setup: Arc<RaceSetup>,
    /// Outer sequence clock and the card's own (the conducted 1014274), and its conducted children's.
    outer: f32,
    clock: f32,
    children: HashMap<usize, f32>,
    look: Option<Look>,
    race_started: bool,
    /// "TRANSITION" set: the intro may start.
    released: bool,
    /// Picture slots for `Transitions.Image1` / `Image2` and the next picture to load.
    pictures: [usize; 2],
    next_picture: usize,
    /// The event has an intro (it holds the start until "TRANSITION").
    has_intro: bool,
}

struct Loaded {
    outer: Sequence,
    scene: UiScene,
    slots: HashMap<SlotKey, usize>,
    images: Vec<(ResourceId, Image)>,
    pictures: Vec<ResourceId>,
    fonts: Vec<(u32, mw_formats::Font)>,
    strings: HashMap<String, String>,
    intro: Option<Arc<Sequence>>,
    outro: Option<Arc<Sequence>>,
}

struct Look {
    outer: Sequence,
    scene: UiScene,
    slots: HashMap<SlotKey, usize>,
    textures: HashMap<ResourceId, (Handle<Image>, Vec2)>,
    pictures: Vec<ResourceId>,
    strings: HashMap<String, String>,
    anim: UiAnim,
}

/// The conducted card sequence inside the outer one.
fn card_of(outer: &Sequence) -> Option<(f32, &Sequence)> {
    outer.items.iter().find_map(|i| match &i.kind {
        ItemKind::Conductor(s) => Some((i.start, &**s)),
        _ => None,
    })
}

#[allow(clippy::too_many_arguments)]
fn begin(
    mut commands: Commands,
    mut card: ResMut<EventCard>,
    mut plays: MessageReader<PlayEvent>,
    game: Res<crate::Game>,
    race: Option<Res<RaceState>>,
    driven: Option<Res<crate::race::RaceDriven>>,
    mut time: ResMut<Time<Virtual>>,
    mut reasons: ResMut<crate::pause::PauseReasons>,
    world_logic: Option<NonSendMut<crate::world_logic::WorldLogic>>,
) {
    let Some(PlayEvent(setup)) = plays.read().last().cloned() else { return };
    // `FUN_006f3830` / `FUN_0074a600`: a DLC event whose pack is not owned is listed but not started (`dlc.rs`).
    if let Some(pack) = crate::dlc::event_locked(&setup.event) {
        warn!("event card: {} is part of the {pack}, not owned (DLC locked)", setup.name);
        if let Some(mut w) = world_logic {
            w.plain.event_refused();
        }
        // Started from the map: the menu's pause reason came with it.
        reasons.set(crate::pause::PauseReason::Menu, false, &mut time);
        return;
    }
    if card.card.is_some() || race.is_some() {
        warn!("event card: {} not started (a race is already running)", setup.name);
        return;
    }
    if card.root.is_none() {
        let node = Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        };
        card.root = Some(commands.spawn((node, Pickable::IGNORE, GlobalZIndex(25))).id());
    }
    info!("event card: {} ({})", setup.name, setup.event.id);
    card.transition = false;
    if driven.is_some() {
        // The pause menu's clock (driven: the logic pauses it while the event loads).
        reasons.set(crate::pause::PauseReason::Menu, false, &mut time);
    }
    let job = Arc::new(Mutex::new(None));
    let (out, store, s) = (job.clone(), game.store.clone(), setup.clone());
    crate::platform::thread::Builder::new()
        .name("mw-event-card".into())
        .spawn(move || {
            let t = crate::platform::Instant::now();
            let Ok(lang) = store.language(mw_assets::DEFAULT_LANGUAGE) else { return };
            let outer = match store.ui_sequence(TRANSITION_SEQUENCE) {
                Ok(o) => o,
                Err(e) => {
                    warn!("event card: sequence {TRANSITION_SEQUENCE}: {e}");
                    return;
                }
            };
            let mut wanted: Vec<(SlotKey, u32)> = Vec::new();
            sequence_layouts(&outer, &mut wanted);
            wanted.sort_by_key(|(_, layer)| *layer);
            wanted.dedup_by_key(|(k, _)| *k);
            let mut slots = HashMap::new();
            let mut layouts = Vec::new();
            for ((id, instance), _) in wanted {
                let Ok(mut layout) = store.ui_layout(mw_formats::ui::object_id(id)) else { continue };
                layout.elements.retain(|e| {
                    let shader = e.material.and_then(|m| store.ui_material_shader(m));
                    !shader.as_deref().is_some_and(|s| crate::frontend::UNPORTED_SHADERS.contains(&s))
                });
                let inst = instance.and_then(|i| store.ui_layout_instance(i).ok());
                slots.insert((id, instance), layouts.len());
                layouts.push(PlacedLayout {
                    widget_index: 0,
                    widget: String::new(),
                    widget_type: String::new(),
                    layout,
                    instance: inst,
                });
            }
            let mut scene = store.ui_scene(layouts, &lang);
            scene.letterbox = true;
            let mut images: Vec<(ResourceId, Image)> = scene
                .textures()
                .into_iter()
                .filter_map(|id| Some((id, game_text::decode_texture(&store, id)?)))
                .collect();
            let cfg = store.loading_config().unwrap_or_default();
            let mut seed = crate::frontend::clock_seed();
            let pictures = crate::frontend::pick_pictures(&store, &cfg, PICTURES, &mut seed)
                .into_iter()
                .map(|(id, img)| {
                    images.push((id, img));
                    id
                })
                .collect();
            let mut strings = HashMap::new();
            for (n, tip) in crate::frontend::pick_tips(&cfg, &lang, &mut seed).into_iter().enumerate() {
                strings.insert(format!("tip:{n}"), tip);
            }
            let text = |id: u32| lang.get(id).unwrap_or_default().to_uppercase();
            strings.insert("name".into(), s.name.clone());
            strings.insert("description".into(), text(s.event.description));
            strings.insert("type".into(), text(s.event.type_name));
            let (intro, outro) = store
                .event_sequences(&s.event, s.game_mode.as_ref())
                .map_err(|e| warn!("event card: cutscenes: {e}"))
                .unwrap_or((None, None));
            let fonts = scene.fonts.iter().map(|(&n, f)| (n, f.clone())).collect();
            info!("event card: {} layouts, {} textures in {:.0?}", scene.layouts.len(), images.len(), t.elapsed());
            *out.lock().unwrap() = Some(Loaded { outer, scene, slots, images, pictures, fonts, strings, intro, outro });
        })
        .expect("spawn event-card loader");
    card.job = Some(job);
    card.card = Some(Card {
        setup,
        outer: 0.0,
        clock: 0.0,
        children: HashMap::new(),
        look: None,
        race_started: false,
        released: false,
        pictures: [0, 1],
        next_picture: 2,
        has_intro: false,
    });
}

/// The loaded card: textures and fonts to Bevy; the event's cutscenes to `cutscene.rs`.
fn receive(
    mut commands: Commands,
    mut card: ResMut<EventCard>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<GameFonts>,
) {
    let Some(loaded) = card.job.as_ref().and_then(|j| j.poll_lock().and_then(|mut s| s.take())) else { return };
    card.job = None;
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
    let has_intro = loaded.intro.is_some();
    crate::cutscene::run_event_cutscenes(&mut commands, loaded.intro, loaded.outro);
    if let Some(c) = card.card.as_mut() {
        c.has_intro = has_intro;
        c.look = Some(Look {
            outer: loaded.outer,
            scene: loaded.scene,
            slots: loaded.slots,
            textures,
            pictures: loaded.pictures,
            strings: loaded.strings,
            anim: UiAnim::default(),
        });
    }
}

/// The bindings the card reads.
fn lookup(path: &str, arg: Option<&str>, card: &Card, look: &Look, ready: bool) -> Option<BindValue> {
    let st = |k: &str| BindValue::Text(look.strings.get(k).cloned().unwrap_or_default());
    let e = &card.setup.event;
    let most_wanted = matches!(e.kind, mw_assets::events::EventKind::MostWanted);
    Some(match path {
        "Screen.IsActive" => ready.into(),
        "Screen.TransitionsLoaded" => true.into(),
        "UIMessages.NISStalled" | "System.IsSimulationPausedForStreaming" | "InviteState.ProcessingInvite" => {
            false.into()
        }
        "TransitionImages.Image1Ready" | "TransitionImages.Image2Ready" => false.into(),
        "Transitions.Image1" | "Transitions.Image2" => {
            let slot = if path.ends_with('1') { 0 } else { 1 };
            let tex = look.pictures.get(card.pictures[slot] % look.pictures.len().max(1))?;
            BindValue::Text(format!("{:016x}", tex.0))
        }
        "Transitions.Tip" => st(&format!("tip:{}", card.next_picture.saturating_sub(2) % crate::frontend::TIPS)),
        "Events.Loading.Name" => st("name"),
        "Events.Loading.Description" => st("description"),
        "Events.Loading.EventTypeName" => st("type"),
        "Events.Loading.Difficulty" => (e.allowed_vehicles.first().map_or(0, |v| v.difficulty) as f64).into(),
        "Event.Data.IsBlacklistEvent" => most_wanted.into(),
        "Events.Loading.IsDLC" | "Events.Current.IsDLC" => false.into(),
        // Rewards, the speedwall and the Most Wanted list: no progression or Autolog data (module doc).
        "Events.Loading.Reward" | "Events.Loading.SecondReward" | "Events.Loading.ThirdReward" => 0.0.into(),
        "Events.Loading.TargetSpeed" | "Events.Current.TargetSpeed" | "Events.Loading.TargetScore" => 0.0.into(),
        "TopOfSpeedwall.CurrentEvent.No1Gamertag" => BindValue::Text(String::new()),
        "SequenceMwl.NumItemsShown" | "ModeVM.Event.speedwall.Items._length" => 0.0.into(),
        "Mode.IsBiggerBetter"
        | "Mode.IsScoreBasedEvent"
        | "Mode.IsBiggerBetterAndNotScoreBasedEvent"
        | "Mode.RecommendsEventActive"
        | "Players.LocalPlayer.HasFinishedRace"
        | "Net.IsConnectedToServer" => false.into(),
        // The layout's `TextN` (0 = none).
        "UISequences.Params" => BindValue::Text(String::new()),
        "Signals.True" => true.into(),
        "Signals.False" => false.into(),
        _ => {
            let _ = arg;
            return None;
        }
    })
}

/// The race is ready to be shown: started, opponents placed, its start area resident.
fn race_ready(race: Option<&RaceState>, loaded: &crate::Loaded, residency: &crate::frontend::Residency) -> bool {
    race.is_some_and(|r| {
        !r.hold
            && !r.placing
            && loaded.done
            && residency.resident(r.player_start.translation, crate::frontend::READY_RADIUS)
    })
}

#[allow(clippy::too_many_arguments)]
fn advance(
    mut card: ResMut<EventCard>,
    real: Res<Time<Real>>,
    mut time: ResMut<Time<Virtual>>,
    mut reasons: ResMut<crate::pause::PauseReasons>,
    race: Option<ResMut<RaceState>>,
    loaded: Res<crate::Loaded>,
    residency: Res<crate::frontend::Residency>,
    mut start: MessageWriter<StartRace>,
    mut cutscenes: ResMut<crate::cutscene::CutsceneState>,
    driven: Option<Res<crate::race::RaceDriven>>,
) {
    let card = &mut *card;
    let Some(c) = card.card.as_mut() else { return };
    let dt = real.delta_secs().min(0.1);
    let ready = race_ready(race.as_deref(), &loaded, &residency);
    // Hold the intro until "TRANSITION".
    cutscenes.hold_intro = !c.released;
    let Some(look) = c.look.take() else { return };
    // Driven: the logic starts the race (logic.rs, on its respawn at the event's start).
    if !c.race_started && c.clock >= START_AT && driven.is_none() {
        c.race_started = true;
        reasons.set(crate::pause::PauseReason::Menu, false, &mut time);
        start.write(StartRace(c.setup.clone()));
    }
    if let Some(mut r) = race
        && !c.released
        && c.has_intro
    {
        // The intro holds the start from the beginning (`hold_for_intro` for `--race`).
        r.cutscene = true;
    }
    let Some(seq) = card_of(&look.outer).map(|(_, q)| q.clone()) else {
        card.card = None;
        return;
    };
    // The card's own clock with its jumps (the hold while the race is not ready).
    let (next, _) = step(&seq, c.clock, dt, &mut |b| binding_true(b, &mut |p, a| lookup(p, a, c, &look, ready)));
    let before = c.clock;
    c.clock = next.min(seq.length);
    // "TRANSITION" (the general trigger item) releases the intro.
    for i in &seq.items {
        if let ItemKind::GeneralTrigger { .. } = i.kind
            && before < i.start
            && c.clock >= i.start
            && !c.released
        {
            c.released = true;
            card.transition = true;
            cutscenes.hold_intro = false;
            info!("event card: TRANSITION at {:.2} s", c.clock);
        }
    }
    // Conducted children of the card, each on its own clock from its item's start.
    for (k, i) in seq.items.iter().enumerate() {
        let ItemKind::Conductor(child) = &i.kind else { continue };
        if c.clock < i.start {
            continue;
        }
        let t = c.children.get(&k).copied();
        let (n, jumped) = step(child, t.unwrap_or(0.0), if t.is_some() { dt } else { 0.0 }, &mut |b| {
            binding_true(b, &mut |p, a| lookup(p, a, c, &look, ready))
        });
        // The picture sequence: a streamed image item that (re)starts loads the next picture into its slot.
        for it in &child.items {
            if let ItemKind::StreamedImage { target, .. } = &it.kind {
                let prev = t.unwrap_or(0.0);
                let starts = (prev < it.start && n >= it.start) || (jumped && n >= it.start && n - it.start < 0.5);
                if starts && t.is_some() {
                    let slot = if target.ends_with('1') { 0 } else { 1 };
                    c.pictures[slot] = c.next_picture;
                    c.next_picture += 1;
                }
            }
        }
        c.children.insert(k, n);
    }
    // The outer sequence: its monitor waits for the card, then the hold and the closing black.
    let card_done = c.clock >= seq.length;
    if c.outer < MONITOR_AT {
        c.outer = (c.outer + dt).min(MONITOR_AT);
    } else if card_done && c.outer < MONITOR_TO {
        c.outer = MONITOR_TO;
    } else if c.outer >= MONITOR_TO {
        let (n, _) =
            step(&look.outer, c.outer, dt, &mut |b| binding_true(b, &mut |p, a| lookup(p, a, c, &look, ready)));
        c.outer = n;
    }
    let finished = c.outer >= look.outer.length;
    c.look = Some(look);
    if finished {
        info!("event card: done");
        cutscenes.hold_intro = false;
        card.card = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    mut commands: Commands,
    mut card: ResMut<EventCard>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    time: Res<Time<Real>>,
    race: Option<Res<RaceState>>,
    loaded: Res<crate::Loaded>,
    residency: Res<crate::frontend::Residency>,
    mut nodes: Query<PoolNode, With<QuadNode>>,
    mut materials: ResMut<Assets<AdditiveQuadMaterial>>,
) {
    let Some(root) = card.root else { return };
    let Ok(window) = windows.single() else { return };
    let screen = [window.width(), window.height()];
    let ready = race_ready(race.as_deref(), &loaded, &residency);
    let full = |colour: [f32; 4]| Quad {
        texture: None,
        uv: [0.0, 0.0, 1.0, 1.0],
        rect: [0.0, 0.0, screen[0], screen[1]],
        rotation: 0.0,
        colour,
        blend: UiBlend::Alpha,
        map: false,
    };
    let EventCard { card: state, pool, .. } = &mut *card;
    let empty = HashMap::new();
    let Some(c) = state.as_mut() else {
        show_quads(&mut commands, root, pool, &[], &empty, None, &mut nodes, &mut materials);
        return;
    };
    let Some(mut look) = c.look.take() else {
        // Loading: black (the transition starts black).
        let black = [full([0.0, 0.0, 0.0, 1.0])];
        show_quads(&mut commands, root, pool, &black, &empty, None, &mut nodes, &mut materials);
        return;
    };
    let mut quads = Vec::new();
    let mut active: Vec<usize> = Vec::new();
    {
        let c = &*c;
        let look = &look;
        let add = |seq: &Sequence, clock: f32, active: &mut Vec<usize>, quads: &mut Vec<Quad>| {
            for i in seq.items.iter().filter(|i| clock >= i.start && clock < i.end) {
                if !i.enabled.is_empty() && !binding_true(&i.enabled, &mut |p, a| lookup(p, a, c, look, ready)) {
                    continue;
                }
                match &i.kind {
                    ItemKind::Layout { layout: Some(l), instance, .. } => {
                        if let Some(&s) = look.slots.get(&((l.0 & 0xffff_ffff) as u32, *instance)) {
                            active.push(s);
                        }
                    }
                    ItemKind::BlackQuad => quads.push(full([0.0, 0.0, 0.0, 1.0])),
                    _ => {}
                }
            }
        };
        // The outer sequence's own layouts (its conductor is drawn on the card's clock below).
        for i in look.outer.items.iter().filter(|i| c.outer >= i.start && c.outer < i.end) {
            if let ItemKind::Layout { layout: Some(l), instance, .. } = &i.kind
                && let Some(&s) = look.slots.get(&((l.0 & 0xffff_ffff) as u32, *instance))
            {
                active.push(s);
            }
        }
        if c.outer < MONITOR_TO
            && let Some((_, seq)) = card_of(&look.outer)
        {
            add(seq, c.clock, &mut active, &mut quads);
            for (k, i) in seq.items.iter().enumerate() {
                if let (ItemKind::Conductor(child), Some(&t)) = (&i.kind, c.children.get(&k))
                    && c.clock >= i.start
                    && c.clock < i.end
                {
                    add(child, t, &mut active, &mut quads);
                }
            }
        }
    }
    let set: HashSet<usize> = active.iter().copied().collect();
    let mut anim = std::mem::take(&mut look.anim);
    {
        let mut options =
            DrawOptions::new(Some(Box::new(move |i| set.contains(&i))), Some(&mut anim), time.elapsed_secs_f64());
        let mut lk = |p: &str, a: Option<&str>| lookup(p, a, c, &look, ready);
        quads.extend(look.scene.draw_with(screen, &mut lk, &mut options));
    }
    look.anim = anim;
    // The black bars outside the letterboxed frame (`FUN_006240b0`).
    quads.extend(mw_assets::ui::UiFrame::letterbox(screen).bars(screen));
    show_quads(&mut commands, root, pool, &quads, &look.textures, None, &mut nodes, &mut materials);
    c.look = Some(look);
}
