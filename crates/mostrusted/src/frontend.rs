//! The start-up flow: title screen, the boot transition that covers the stream-in, the in-game loading screen,
//! the free-roam start and the "continue" profile (docs/decomp.md "Front end: boot, loading and the free-roam
//! start"; data in `mw_assets::frontend`).
//!
//! **The original** (game-logic HSM "Top State", read from `GAMELOGIC/GAMEMODES.BNDL`): the boot page
//! (screen 491012) skips the language select on PC and activates "Press Start" (widget group 491032), whose
//! `Background` widget plays the title sequence 1022357: it jumps to 28.1 s (not Japanese), plays the intro
//! movie 1689283, then loops the title movie 1689287 with the music; at 37.4 s it sets the general trigger
//! "BOOT", which shows "PRESS ANY KEY" and lets the "Press Start" behaviour wait for any key (armed on a
//! press, done on the release). Leaving the page plays the screen transition 1836020: black, a fade from
//! black, the car pictures cross-fading (sequence 1899171, `LoadingTipsConfig.TransitionImages`) and the
//! strip with the logo; it **holds at 4.05–4.25 s while the next screen is not active** (`Screen.IsActive(NOT)`)
//! and so covers the world loading, then plays out to 4.8 s. Meanwhile "Post Controller Bind Boot" loads the
//! world and "InGame" sets up the start point; the game-mode manager's world-loaded flag needs the world's
//! resources, the zone at the camera and its neighbours within 10 m resident, and traffic ready or 1.5 s.
//! In game, the loading screen (layout 108177: black, a spinner turning once a second) shows while
//! `Mode.ShowLoadingScreen`, i.e. while that flag is false; controls are disabled and sound muted meanwhile.
//!
//! **Here**: [`Phase::Title`] → [`Phase::Transition`] (holding while not [`FrontEnd::ready`]) → playing, for
//! plain runs (`mostrusted` with no mode flags) and `--front-end`; other interactive world drives show
//! [`Phase::Loading`] (108177) until ready. Headless runs and tests start in [`Phase::Playing`].
//!
//! Model decisions (not from code or data): the screen behind the title movie is the widget's white backdrop
//! drawn full screen, the movies are decoded by ffmpeg (`movie.rs`) and drawn full screen, the title music and
//! the transition's whooshes are not played (sound stays muted), "ready" uses the 1.5 s timeout instead of
//! checking traffic, the pictures are picked at random from the base game's 60 (four preloaded), the
//! `TransitionImages.ImageNReady` holds never wait (our pictures are decoded up front; their polarity was not
//! traced), and the profile (car, position) is our own file, saved every 10 s like the game's "Regular
//! Autosave" state.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use mw_assets::frontend as fe;
use mw_assets::sequence::{Controller, ItemKind, Sequence};
use mw_assets::ui::{DrawOptions, PlacedLayout, Quad, UiAnim, UiBlend, UiScene};
use mw_bundle::ResourceId;
use mw_formats::binding::{BindValue, Binding};

use crate::game_text::{self, GameFonts, LoadedFont};
use crate::movie::Movie;
use crate::ui_quads::{AdditiveQuadMaterial, PoolNode, QuadNode, QuadPool, show_quads};

/// The in-game loading screen layout (`UIConfig.eee932fa` / `a65d5d07` both list only this one).
pub const LOADING_LAYOUT: u32 = 108_177;
/// The car pictures preloaded for the transition.
const PICTURES: usize = 4;
/// Loading tips picked for a run (one per picture).
const TIPS: usize = 8;
/// UI shaders not ported: their elements are not drawn.
const UNPORTED_SHADERS: [&str; 1] = ["UIRoadRibbonShader"];
/// The world-loaded flag's traffic wait (`DAT_00fc5aac`).
const TRAFFIC_TIMEOUT: f32 = 1.5;
/// Radius around the camera whose neighbour zones must be resident (`DAT_00fc9340`).
pub const READY_RADIUS: f32 = 10.0;
/// A placement waits until its target is resident within max(this, 3 s of travel) (`FUN_00647e00`).
pub const PLACE_RADIUS: f32 = 200.0;
/// Seconds between profile saves (our "Regular Autosave").
const AUTOSAVE: f32 = 10.0;
/// `MW_FE_QUADS`: log the quads of the frame a front-end screenshot is taken.
static DUMP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Fake texture keys for the full-screen movie and backdrop quads.
const MOVIE_KEY: ResourceId = ResourceId(0x0f00_0000_0000_0001);

/// What the front end starts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The press-start screen.
    Title,
    /// The boot screen's transition, holding over the world loading.
    Transition,
    /// The in-game loading screen (layout 108177).
    Loading,
    Playing,
}

pub struct FrontEndPlugin {
    pub start: Phase,
    /// `--auto-start <s>`: "press" a key this long after the prompt shows (headless runs).
    pub auto_start: Option<f32>,
    /// `--front-end-shots <dir>`: screenshots at each step, then exit.
    pub shots: Option<PathBuf>,
    /// Where to save the profile (interactive world drives, or `--profile`); `None` saves nothing.
    pub profile: Option<PathBuf>,
}

impl Plugin for FrontEndPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(FrontEnd {
            phase: self.start,
            clock: 0.0,
            started: Instant::now(),
            ready: false,
            ready_since: None,
            boot_trigger: false,
            armed: false,
            prompt_at: None,
            auto_start: self.auto_start,
            shots: self.shots.clone(),
            shot_step: 0,
            images_clock: None,
            pictures: [0, 1],
            next_picture: 2,
            profile: self.profile.clone(),
            autosave: 0.0,
            log: Vec::new(),
        })
        .add_systems(Startup, start_loading)
        .add_systems(
            Update,
            (
                receive,
                track_ready,
                advance,
                draw,
                (freeze_controls, mute).after(crate::drive::read_input),
                save_profile,
                take_shots,
            )
                .chain(),
        );
    }
}

/// Which track units (= `ZoneList` zones) have been streamed in.
#[derive(Resource, Default)]
pub struct Residency {
    zones: Option<mw_formats::ZoneList>,
    loaded: HashSet<u32>,
    /// Every unit the loader was asked for has been sent (`--units` runs never get the full set).
    pub all_sent: bool,
}

impl Residency {
    /// Residency for a run: without a streamed world (the test track, the car viewer) everything counts as
    /// resident.
    pub fn new(world: bool) -> Self {
        Self { all_sent: !world, ..Default::default() }
    }

    pub fn set_zones(&mut self, zones: mw_formats::ZoneList) {
        self.zones = Some(zones);
    }

    pub fn unit_loaded(&mut self, unit: u32) {
        self.loaded.insert(unit);
    }

    /// `Resident(p, r)` (`FUN_0054b540` → `FUN_0054b340`): the zone holding `p` is loaded and so is every
    /// neighbour zone within `r` of `p`. Without a zone list, or once the loader is done, everything is.
    pub fn resident(&self, p: Vec3, r: f32) -> bool {
        if self.all_sent {
            return true;
        }
        let Some(zones) = &self.zones else { return false };
        let Some(zone) = zones.nearest(p.x, p.z) else { return false };
        self.loaded.contains(&zone.id)
            && zone.neighbours.iter().all(|&(i, _)| {
                let n = &zones.zones[i];
                n.distance(p.x, p.z) > r || self.loaded.contains(&n.id)
            })
    }
}

#[derive(Resource)]
pub struct FrontEnd {
    pub phase: Phase,
    /// The current screen's sequence clock (s).
    clock: f32,
    started: Instant,
    /// The world-loaded flag (see the module doc).
    pub ready: bool,
    ready_since: Option<f32>,
    /// The title's "BOOT" trigger is set.
    boot_trigger: bool,
    /// "Press Start": a key went down after the trigger (done on its release).
    armed: bool,
    prompt_at: Option<f32>,
    auto_start: Option<f32>,
    shots: Option<PathBuf>,
    shot_step: u32,
    /// The car-picture sequence's own clock (started by the transition's conductor item).
    images_clock: Option<f32>,
    /// Picture shown in `Transitions.Image1` / `Image2` (indices into the preloaded ones).
    pictures: [usize; 2],
    next_picture: usize,
    profile: Option<PathBuf>,
    autosave: f32,
    /// Timing log (phase, seconds since start).
    log: Vec<(String, f32)>,
}

impl FrontEnd {
    pub fn playing(&self) -> bool {
        self.phase == Phase::Playing
    }

    fn mark(&mut self, what: &str) {
        let t = self.started.elapsed().as_secs_f32();
        info!("FRONTEND {what} at {t:.2} s");
        self.log.push((what.to_string(), t));
    }
}

/// A layout the screens use: its scene index per (layout, instance).
type SlotKey = (u32, Option<ResourceId>);

struct Loaded {
    scene: UiScene,
    slots: HashMap<SlotKey, usize>,
    /// Draw order: scene layouts sorted by layer (built that way).
    title: Option<Sequence>,
    transition: Option<Sequence>,
    images: Vec<(ResourceId, Image)>,
    pictures: Vec<ResourceId>,
    fonts: Vec<(u32, mw_formats::Font)>,
    strings: HashMap<String, String>,
}

#[derive(Resource)]
struct Loading(Mutex<Receiver<Loaded>>);

#[derive(Resource)]
struct Screens {
    scene: UiScene,
    slots: HashMap<SlotKey, usize>,
    title: Option<Sequence>,
    transition: Option<Sequence>,
    textures: HashMap<ResourceId, (Handle<Image>, Vec2)>,
    pictures: Vec<ResourceId>,
    strings: HashMap<String, String>,
    anim: UiAnim,
    root: Entity,
    pool: QuadPool,
    movie: Option<Movie>,
    failed_movie: Option<u32>,
}

/// Layouts of a sequence (recursing into conductors) with their layers.
fn sequence_layouts(seq: &Sequence, out: &mut Vec<(SlotKey, u32)>) {
    for i in &seq.items {
        match &i.kind {
            ItemKind::Layout { layout: Some(l), layer, instance, .. } => {
                out.push((((l.0 & 0xffff_ffff) as u32, *instance), *layer))
            }
            ItemKind::Conductor(s) => sequence_layouts(s, out),
            _ => {}
        }
    }
}

fn start_loading(mut commands: Commands, game: Res<crate::Game>, front: Res<FrontEnd>) {
    if front.phase == Phase::Playing {
        return;
    }
    let store = game.store.clone();
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("mw-frontend".into())
        .spawn(move || {
            let t = Instant::now();
            let Ok(lang) = store.language(mw_assets::DEFAULT_LANGUAGE) else { return };
            let title = store.ui_sequence(fe::TITLE_SEQUENCE).map_err(|e| warn!("title sequence: {e}")).ok();
            let transition =
                store.ui_sequence(fe::BOOT_TRANSITION_SEQUENCE).map_err(|e| warn!("boot transition: {e}")).ok();
            let mut wanted: Vec<(SlotKey, u32)> = Vec::new();
            for s in title.iter().chain(&transition) {
                sequence_layouts(s, &mut wanted);
            }
            wanted.push(((LOADING_LAYOUT, None), 0x17));
            // Sorted by layer (stable: item order within a layer), so the scene draws bottom-up.
            wanted.sort_by_key(|(_, layer)| *layer);
            wanted.dedup_by_key(|(k, _)| *k);
            let mut slots = HashMap::new();
            let mut layouts = Vec::new();
            for ((id, instance), _) in wanted {
                let Ok(mut layout) = store.ui_layout(mw_formats::ui::object_id(id)) else {
                    warn!("front end: no layout {id}");
                    continue;
                };
                // Elements drawn by shaders we have not ported are left out (model decision): the transition's
                // `UIRoadRibbonShader` light streaks (layout 1014312) would otherwise show their raw stripe mask.
                layout.elements.retain(|e| {
                    let shader = e.material.and_then(|m| store.ui_material_shader(m));
                    !shader.as_deref().is_some_and(|s| UNPORTED_SHADERS.contains(&s))
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
            let scene = store.ui_scene(layouts, &lang);
            let mut images: Vec<(ResourceId, Image)> = scene
                .textures()
                .into_iter()
                .filter_map(|id| Some((id, game_text::decode_texture(&store, id)?)))
                .collect();
            // Car pictures: a random few of the base game's (`Signals.GetIntValue[1]` picks them in the game).
            let cfg = store.loading_config().unwrap_or_default();
            let mut pool = cfg.images.clone();
            let mut seed =
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
            let mut pictures = Vec::new();
            while pictures.len() < PICTURES && !pool.is_empty() {
                seed = seed.wrapping_mul(0x5851_f42d_4c95_7f2d).wrapping_add(1442695040888963407);
                let k = (seed >> 33) as usize % pool.len();
                let id = pool.swap_remove(k);
                if let Some(tex) = store.streamed_image(id)
                    && let Some(img) = game_text::decode_texture(&store, tex)
                {
                    images.push((tex, img));
                    pictures.push(tex);
                }
            }
            let fonts = scene.fonts.iter().map(|(&n, f)| (n, f.clone())).collect();
            let text = |id: Option<u32>| id.and_then(|i| lang.get(i)).unwrap_or_default().to_string();
            let mut strings = HashMap::new();
            for term in ["Press", "PressStart", "AutoSaveWarning"] {
                strings.insert(format!("term:{term}"), text(store.compliance_term(term)));
            }
            strings.insert("key:AUTOLOG_ACCEPT".into(), text(store.keyboard_button_text("KEY_RETURN")));
            // Loading tips for the strip's `Transitions.Tip` (the offline categories, a random order; how the
            // game's `GameTipsViewModel` picks them was not traced).
            let mut tips = cfg.tips.clone();
            let mut n = 0;
            while !tips.is_empty() && n < TIPS {
                seed = seed.wrapping_mul(0x5851_f42d_4c95_7f2d).wrapping_add(1442695040888963407);
                let id = tips.swap_remove((seed >> 33) as usize % tips.len());
                if let Some(s) = lang.get(id) {
                    strings.insert(format!("tip:{n}"), s.to_uppercase());
                    n += 1;
                }
            }
            info!(
                "front end: {} layouts, {} textures, {} pictures in {:.0?}",
                scene.layouts.len(),
                images.len(),
                pictures.len(),
                t.elapsed()
            );
            let _ = tx.send(Loaded { scene, slots, title, transition, images, pictures, fonts, strings });
        })
        .expect("spawn front-end loader");
    commands.insert_resource(Loading(Mutex::new(rx)));
}

fn receive(
    mut commands: Commands,
    loading: Option<Res<Loading>>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<GameFonts>,
    mut front: ResMut<FrontEnd>,
) {
    let Some(loading) = loading else { return };
    let Ok(a) = loading.0.lock().unwrap().try_recv() else { return };
    commands.remove_resource::<Loading>();
    let textures: HashMap<ResourceId, (Handle<Image>, Vec2)> = a
        .images
        .into_iter()
        .map(|(id, img)| {
            let size = img.size().as_vec2();
            (id, (images.add(img), size))
        })
        .collect();
    for (number, font) in a.fonts {
        if let (false, Some(atlas)) = (fonts.fonts.contains_key(&number), font.texture.and_then(|t| textures.get(&t))) {
            fonts.fonts.insert(number, LoadedFont { font: std::sync::Arc::new(font), atlas: atlas.0.clone() });
        }
    }
    let node = Node {
        position_type: PositionType::Absolute,
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        ..default()
    };
    let root = commands.spawn((node, Pickable::IGNORE, GlobalZIndex(20))).id();
    commands.insert_resource(Screens {
        scene: a.scene,
        slots: a.slots,
        title: a.title,
        transition: a.transition,
        textures,
        pictures: a.pictures,
        strings: a.strings,
        anim: UiAnim::default(),
        root,
        pool: QuadPool::default(),
        movie: None,
        failed_movie: None,
    });
    front.clock = 0.0;
    front.mark("screens loaded");
}

/// The world-loaded flag: the player's car is placed, the zone at it and its neighbours within 10 m are
/// resident, then 1.5 s (the traffic timeout) have passed.
fn track_ready(
    mut front: ResMut<FrontEnd>,
    residency: Res<Residency>,
    car: Query<&GlobalTransform, (With<crate::drive::Car>, With<crate::drive::Player>)>,
    time: Res<Time<Real>>,
) {
    if front.ready {
        return;
    }
    let t = time.elapsed_secs();
    let here = car.single().ok().map(|x| x.translation());
    match (here.is_some_and(|p| residency.resident(p, READY_RADIUS)), front.ready_since) {
        (true, None) => {
            front.ready_since = Some(t);
            front.mark("car placed and area resident");
        }
        (true, Some(since)) if t - since >= TRAFFIC_TIMEOUT => {
            front.ready = true;
            front.mark("world loaded");
        }
        (false, _) => front.ready_since = None,
        _ => {}
    }
}

/// Clock with the sequence's jump controllers: crossing a jump's trigger time while its binding holds (empty =
/// always) jumps to its destination. Returns the new clock and whether it jumped.
fn step(seq: &Sequence, clock: f32, dt: f32, holds: &mut dyn FnMut(&str) -> bool) -> (f32, bool) {
    let next = clock + dt;
    for c in &seq.controllers {
        if let Controller::Jump { trigger_time, destination, enabled, .. } = c
            && clock <= *trigger_time
            && *trigger_time < next
            && (enabled.is_empty() || holds(enabled))
        {
            return (*destination, true);
        }
    }
    (next, false)
}

fn binding_true(b: &str, lookup: &mut dyn FnMut(&str, Option<&str>) -> Option<BindValue>) -> bool {
    Binding::parse(b).and_then(|b| b.eval(lookup)).is_some_and(|v| v.truthy())
}

/// The bindings the boot screens read.
fn lookup(
    path: &str,
    arg: Option<&str>,
    front: &FrontEnd,
    strings: &HashMap<String, String>,
    pictures: &[ResourceId],
    pad: bool,
) -> Option<BindValue> {
    let quoted = || arg.map(|a| a.split('"').nth(1).unwrap_or(a).to_string());
    Some(match path {
        "Language.IsJapanese" | "EntitlementManager.ShowDirectDownloadInfo" => false.into(),
        "DemoBuild.Name" => BindValue::Text(String::new()),
        "GeneralSequence.Triggers" => front.boot_trigger.into(),
        "Controls.IsXInputControllerConnected" => pad.into(),
        "ComplianceTerm.Value" => strings.get(&format!("term:{}", quoted()?))?.clone().into(),
        // The keyboard's text for the action's key (Enter for AUTOLOG_ACCEPT, the default keyboard table).
        "Controls.ButtonPrompt" => strings.get(&format!("key:{}", quoted()?))?.clone().into(),
        "Screen.TransitionsLoaded" => true.into(),
        "Screen.IsActive" => front.ready.into(),
        "TransitionImages.Image1Ready" | "TransitionImages.Image2Ready" => false.into(),
        "Transitions.Image1" | "Transitions.Image2" => {
            let slot = if path.ends_with('1') { 0 } else { 1 };
            let tex = pictures.get(front.pictures[slot] % pictures.len().max(1))?;
            BindValue::Text(format!("{:016x}", tex.0))
        }
        // The strip's tip: the next one with each new picture.
        "Transitions.Tip" => {
            strings.get(&format!("tip:{}", front.next_picture.saturating_sub(2) % TIPS))?.clone().into()
        }
        "Signals.True" => true.into(),
        "Signals.False" => false.into(),
        _ => return None,
    })
}

/// Advance the phase clocks; title → transition on a key, transition → playing once ready and played out.
#[allow(clippy::too_many_arguments)]
fn advance(
    mut front: ResMut<FrontEnd>,
    screens: Option<ResMut<Screens>>,
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    game: Res<crate::Game>,
    mut images: ResMut<Assets<Image>>,
) {
    let dt = time.delta_secs().min(0.1);
    if front.phase == Phase::Loading && front.ready {
        front.phase = Phase::Playing;
        front.mark("playing (loading screen off)");
        return;
    }
    let Some(mut screens) = screens else { return };
    let pad = pads.iter().next().is_some();
    match front.phase {
        Phase::Title => {
            let Some(seq) = screens.title.clone() else {
                front.phase = Phase::Transition;
                return;
            };
            let (strings, pictures) = (screens.strings.clone(), screens.pictures.clone());
            let clock = front.clock;
            let (next, _) = {
                let fr = &*front;
                step(&seq, clock, dt, &mut |b| binding_true(b, &mut |p, a| lookup(p, a, fr, &strings, &pictures, pad)))
            };
            front.clock = next;
            // The general trigger item ("BOOT") sets its trigger when its start passes.
            for i in &seq.items {
                if let ItemKind::GeneralTrigger { .. } = i.kind
                    && next >= i.start
                    && !front.boot_trigger
                {
                    front.boot_trigger = true;
                    front.prompt_at = Some(time.elapsed_secs());
                    front.mark("title: PRESS ANY KEY shows");
                }
            }
            // The movie item active now (the intro, then the looping title movie).
            let active = seq.items.iter().find_map(|i| match i.kind {
                ItemKind::Movie { movie, looping } if next >= i.start && next < i.end => {
                    Some((movie, looping, i.start))
                }
                _ => None,
            });
            match active {
                Some((id, looping, start)) => {
                    if screens.movie.as_ref().is_none_or(|m| m.id != id) && screens.failed_movie != Some(id) {
                        screens.movie = Movie::open(game.store.game(), id, looping, &mut images);
                        if screens.movie.is_none() {
                            warn!("movie {id}: not playable (no ffmpeg?)");
                            screens.failed_movie = Some(id);
                        }
                    }
                    if let Some(m) = screens.movie.as_mut() {
                        m.update(next - start, &mut images);
                    }
                }
                None => screens.movie = None,
            }
            // "Press Start" (`WAITFORANYCO`): armed by any key or button going down, done on its release.
            if front.boot_trigger {
                let any_down =
                    keys.get_pressed().next().is_some() || pads.iter().any(|p| p.get_pressed().next().is_some());
                let auto = front.auto_start.zip(front.prompt_at).is_some_and(|(a, at)| time.elapsed_secs() - at >= a);
                if any_down {
                    front.armed = true;
                } else if front.armed || auto {
                    front.phase = Phase::Transition;
                    front.clock = 0.0;
                    screens.movie = None;
                    screens.anim.reset();
                    front.mark("title: key pressed, transition starts");
                }
            }
        }
        Phase::Transition => {
            let Some(seq) = screens.transition.clone() else {
                if front.ready {
                    front.phase = Phase::Playing;
                }
                return;
            };
            let (strings, pictures) = (screens.strings.clone(), screens.pictures.clone());
            let clock = front.clock;
            let (next, jumped) = {
                let fr = &*front;
                step(&seq, clock, dt, &mut |b| binding_true(b, &mut |p, a| lookup(p, a, fr, &strings, &pictures, pad)))
            };
            if jumped && next < clock && clock > 4.0 && front.log.last().is_none_or(|l| l.0 != "transition: holding") {
                front.mark("transition: holding");
            }
            front.clock = next;
            // The conductor's nested sequence (the car pictures) runs on its own clock from the item's start.
            for i in &seq.items {
                let ItemKind::Conductor(child) = &i.kind else { continue };
                if next < i.start {
                    continue;
                }
                let c = front.images_clock.unwrap_or(0.0);
                let (n, j) = {
                    let fr = &*front;
                    step(child, c, if front.images_clock.is_some() { dt } else { 0.0 }, &mut |b| {
                        binding_true(b, &mut |p, a| lookup(p, a, fr, &strings, &pictures, pad))
                    })
                };
                // A streamed image item that (re)starts loads the next picture into its slot.
                for it in &child.items {
                    if let ItemKind::StreamedImage { target, .. } = &it.kind {
                        let starts = (c < it.start && n >= it.start) || (j && n >= it.start && n - it.start < 0.5);
                        if starts && front.images_clock.is_some() {
                            let slot = if target.ends_with('1') { 0 } else { 1 };
                            front.pictures[slot] = front.next_picture;
                            front.next_picture += 1;
                        }
                    }
                }
                if j {
                    // A jump restarts the layers: their fades play again.
                    screens.anim.reset();
                }
                front.images_clock = Some(n);
            }
            if next >= seq.length {
                front.phase = Phase::Playing;
                front.mark("transition done: playing");
            }
        }
        Phase::Loading | Phase::Playing => {}
    }
}

/// Draw the current screen.
#[allow(clippy::too_many_arguments)]
fn draw(
    mut commands: Commands,
    front: Res<FrontEnd>,
    screens: Option<ResMut<Screens>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    pads: Query<&Gamepad>,
    time: Res<Time<Real>>,
    mut nodes: Query<PoolNode, With<QuadNode>>,
    mut materials: ResMut<Assets<AdditiveQuadMaterial>>,
) {
    let Some(mut screens) = screens else { return };
    let Ok(window) = windows.single() else { return };
    let screen = [window.width(), window.height()];
    let pad = pads.iter().next().is_some();
    let full = |colour: [f32; 4], texture: Option<ResourceId>| Quad {
        texture,
        uv: [0.0, 0.0, 1.0, 1.0],
        rect: [0.0, 0.0, screen[0], screen[1]],
        rotation: 0.0,
        colour,
        blend: UiBlend::Alpha,
        map: false,
    };
    // (scene slot, sequence clock) of each layout to draw, and the base quads under them.
    let mut base: Vec<Quad> = Vec::new();
    let mut active: Vec<usize> = Vec::new();
    let mut texts: HashMap<usize, [u32; 3]> = HashMap::new();
    let Screens { scene, slots, title, transition, textures, pictures, strings, anim, root, pool, movie, .. } =
        &mut *screens;
    let mut add_items = |seq: &Sequence, clock: f32, active: &mut Vec<usize>, base: &mut Vec<Quad>| {
        for i in seq.items.iter().filter(|i| clock >= i.start && clock < i.end) {
            if !i.enabled.is_empty()
                && !binding_true(&i.enabled, &mut |p, a| lookup(p, a, &front, strings, pictures, pad))
            {
                continue;
            }
            match &i.kind {
                ItemKind::Layout { layout: Some(l), instance, texts: t, .. } => {
                    if let Some(&s) = slots.get(&((l.0 & 0xffff_ffff) as u32, *instance)) {
                        active.push(s);
                        texts.insert(s, *t);
                    }
                }
                ItemKind::BlackQuad => base.push(full([0.0, 0.0, 0.0, 1.0], None)),
                _ => {}
            }
        }
    };
    match front.phase {
        Phase::Title => {
            // The press-start widget's own layouts: the white backdrop, the movie over it (full screen).
            base.push(full([1.0; 4], None));
            if let Some(m) = movie.as_ref().filter(|m| m.ready) {
                textures.insert(MOVIE_KEY, (m.image.clone(), Vec2::new(640.0, 360.0)));
                base.push(full([1.0; 4], Some(MOVIE_KEY)));
            }
            if let Some(seq) = title.as_ref() {
                add_items(seq, front.clock, &mut active, &mut base);
            }
        }
        Phase::Transition => {
            if let Some(seq) = transition.as_ref() {
                add_items(seq, front.clock, &mut active, &mut base);
                if let Some(c) = front.images_clock {
                    for i in seq.items.iter().filter(|i| front.clock >= i.start && front.clock < i.end) {
                        if let ItemKind::Conductor(child) = &i.kind {
                            add_items(child, c, &mut active, &mut base);
                        }
                    }
                }
            }
        }
        Phase::Loading => {
            if let Some(&s) = slots.get(&(LOADING_LAYOUT, None)) {
                active.push(s);
            }
        }
        Phase::Playing => {}
    }
    let now = time.elapsed_secs_f64();
    let mut quads = base;
    if !active.is_empty() {
        let set: HashSet<usize> = active.iter().copied().collect();
        let mut options = DrawOptions::new(Some(Box::new(move |i| set.contains(&i))), Some(anim), now);
        let mut lk = |p: &str, a: Option<&str>| -> Option<BindValue> {
            if p == "UISequences.Params" {
                // The layout's `TextN` (0 = none: the "empty" variant of the label shows).
                return Some(BindValue::Text(String::new()));
            }
            lookup(p, a, &front, strings, pictures, pad)
        };
        quads.extend(scene.draw_with(screen, &mut lk, &mut options));
    }
    if std::env::var_os("MW_FE_QUADS").is_some() && DUMP.swap(false, std::sync::atomic::Ordering::Relaxed) {
        info!("front end {:?} clock {:.2}: screen {screen:?}", front.phase, front.clock);
        for q in &quads {
            info!("  quad {:?} rect {:?} rot {} colour {:?}", q.texture, q.rect, q.rotation, q.colour);
        }
    }
    show_quads(&mut commands, *root, pool, &quads, textures, None, &mut nodes, &mut materials);
}

/// Controls are disabled while a front-end screen shows ("Disable Car Control and Easydrive").
fn freeze_controls(front: Res<FrontEnd>, mut input: ResMut<crate::drive::CarInput>) {
    if !front.playing() {
        input.throttle = 0.0;
        input.brake = 0.0;
        input.steer = 0.0;
        input.handbrake = false;
        input.nitrous = false;
    }
}

/// "Mute All Sound" (`MUTE_SOUND`, reason `LoadingScreen`) while a front-end screen shows.
fn mute(front: Res<FrontEnd>, settings: Option<ResMut<crate::settings::AudioSettings>>) {
    let Some(mut s) = settings else { return };
    let want = !front.playing();
    if s.muted != want {
        s.muted = want;
    }
}

/// The saved profile: the car and where it was (`~/.config/mostrusted/profile.cfg`).
#[derive(Debug, Clone, Default)]
pub struct Profile {
    pub bundle: String,
    pub position: Option<Vec3>,
}

pub fn default_profile_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("mostrusted").join("profile.cfg"))
}

pub fn load_profile(path: &std::path::Path) -> Option<Profile> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut p = Profile::default();
    for line in text.lines() {
        match line.split_once('=') {
            Some(("bundle", v)) => p.bundle = v.trim().to_string(),
            Some(("position", v)) => {
                let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if let [x, y, z] = n[..] {
                    p.position = Some(Vec3::new(x, y, z));
                }
            }
            _ => {}
        }
    }
    (!p.bundle.is_empty()).then_some(p)
}

/// The `VehicleList` id of a car bundle (`VEHICLES/VEH_<id>_HI.BNDL`).
pub fn vehicle_id(bundle: &str) -> Option<u32> {
    bundle.to_ascii_uppercase().rsplit('/').next()?.strip_prefix("VEH_")?.split('_').next()?.parse().ok()
}

fn save_profile(
    mut front: ResMut<FrontEnd>,
    time: Res<Time<Real>>,
    garage: Res<crate::garage::Garage>,
    car: Query<&GlobalTransform, (With<crate::drive::Car>, With<crate::drive::Player>)>,
) {
    if front.profile.is_none() || !front.playing() {
        return;
    }
    let t = time.elapsed_secs();
    if t - front.autosave < AUTOSAVE {
        return;
    }
    front.autosave = t;
    let (Some(current), Ok(xf)) = (garage.current.as_ref(), car.single()) else { return };
    let Some(path) = front.profile.clone() else { return };
    let p = xf.translation();
    let text = format!("bundle={}\nposition={:.2},{:.2},{:.2}\n", current.bundle, p.x, p.y, p.z);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, text) {
        warn!("could not save {}: {e}", path.display());
    }
}

/// `--front-end-shots <dir>`: title with the prompt, the transition early and holding, then the first seconds of
/// play; then exit, printing the timing log.
fn take_shots(mut commands: Commands, mut front: ResMut<FrontEnd>, mut exit: MessageWriter<AppExit>) {
    let Some(dir) = front.shots.clone() else { return };
    if front.shot_step == 0 {
        let _ = std::fs::create_dir_all(&dir);
    }
    let step = front.shot_step;
    let last = front.log.last().map(|l| l.1).unwrap_or(0.0);
    let now = front.started.elapsed().as_secs_f32();
    let shot = |name: &str, commands: &mut Commands| {
        let path = dir.join(format!("{name}.png"));
        info!("FRONTEND screenshot {}", path.display());
        DUMP.store(true, std::sync::atomic::Ordering::Relaxed);
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    };
    let has = |what: &str| front.log.iter().any(|l| l.0.starts_with(what));
    match step {
        // The prompt has been up 0.5 s (use an --auto-start above that).
        0 if has("title: PRESS ANY KEY") && now - last >= 0.5 => {
            shot("1-title", &mut commands);
            front.shot_step = 1;
        }
        1 if front.phase == Phase::Transition && front.clock >= 1.5 => {
            shot("2-transition", &mut commands);
            front.shot_step = 2;
        }
        2 if front.phase == Phase::Transition && has("transition: holding") => {
            shot("3-transition-holding", &mut commands);
            front.shot_step = 3;
        }
        2 | 3 if front.phase == Phase::Playing => front.shot_step = 4,
        0..=2 if front.phase == Phase::Loading && has("screens loaded") && now - last >= 0.5 => {
            shot("2-loading", &mut commands);
            front.shot_step = 3;
        }
        4 if front.log.iter().rev().find(|l| l.0.contains("playing")).is_some_and(|l| now - l.1 >= 2.0) => {
            shot("4-playing", &mut commands);
            front.shot_step = 5;
            front.mark("done");
        }
        5 if now - last >= 1.0 => {
            for (what, at) in &front.log {
                println!("FRONTEND TIMING {at:7.2} s  {what}");
            }
            exit.write(AppExit::Success);
            front.shot_step = 6;
        }
        _ => {}
    }
}
