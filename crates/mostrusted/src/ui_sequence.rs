//! The general UI sequence player: plays any UI sequence (`mw_assets::sequence::Sequence`) the way `frontend.rs`
//! plays the title: its layouts drawn through the layout renderer (`UiScene::draw_with`, `ui_quads.rs`), black
//! quads, movies (`movie.rs`), wave items (`music.rs`'s [`crate::music::SequenceWaves`]), ICE camera takes on the
//! gameplay camera, HideHud items, and its conducted sequences on their own clocks with their jump controllers.
//!
//! **Who plays through it.** Under `--hsm` the logic's own sequences (logic.rs [`Playback`](crate::logic)):
//! conductors' `StartSequence` other than the title and `RUNGAMELOGIC` `PlaySequence` other than the event's intro /
//! outro, e.g. the continue's game recap 1689073 (24 s). The logic keeps the clock (its jumps are what the HSM waits
//! on): every frame it hands the player what plays and where ([`UiSequencePlayer::show`]); the player loads each
//! sequence's look on a thread, steps the conducted children itself and draws. Bindings (items' `enabled`,
//! conducted jumps, the layouts' elements) resolve through the logic's view models (`LogicState`, the [`LogicView`]
//! the logic registers), else they do not resolve. The title / boot transition (`frontend.rs`) and the event card
//! (`event_card.rs`) keep their own players (their pictures, monitor jump and hand-overs are specific to them).
//!
//! **The recap 1689073**: the ICE take 1689072 "Game" relative to the player's car (`Player` space, no bindings;
//! its `GAME_CAM_BLEND` goes 0 → 1 over 13.68–20.35 s, back to the gameplay camera), HideHud 0–18.9 s, layout
//! 1689155 (layer 6, 0–19.1 s), 1689182 (layer 1, unless `Event.Data.DemoMode`), the conducted 1532921 (layout
//! 1532919) and 275950 at 15.9 s (layouts 979577 / 984699 / 1695598 by their bindings, stingers), and the music
//! bed `01000000001b0a3e` (0–21.4 s, type 4).
//!
//! Model decisions (as the other two players): the layouts draw in the letterboxed 1280 × 720 frame with its black
//! bars, a movie fills that frame under the layouts, `UISequences.Params[n].TextN` reads "" (no text parameter),
//! elements of unported shaders (`frontend::UNPORTED_SHADERS`) are left out. Not played: `SetUICamera`,
//! `AdapterHost`, `ImagePaletteSequenceItem`, the bus mixer channels (the waves play at their own gain),
//! `RaceCarEffectSequenceItem`, `ListenerSequenceItem`, the recap's `EventTrigger` (what the logic does with it
//! was not traced), streamed images (no `Transitions.Image` slots here), the take's depth of field and motion blur.

use crate::platform::PollLock;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mw_assets::sequence::{ItemKind, Sequence};
use mw_assets::ui::{DrawOptions, PlacedLayout, Quad, UiAnim, UiBlend, UiScene};
use mw_bundle::ResourceId;
use mw_formats::binding::BindValue;
use mw_formats::camera_take::{Bindings, Frame};

use crate::frontend::{SlotKey, UNPORTED_SHADERS, binding_true, sequence_layouts, step};
use crate::game_text::{self, GameFonts, LoadedFont};
use crate::movie::Movie;
use crate::ui_quads::{AdditiveQuadMaterial, PoolNode, QuadNode, QuadPool, show_quads};

/// Fake texture key for a sequence's movie quad.
const MOVIE_KEY: ResourceId = ResourceId(0x0f00_0000_0000_0002);

pub struct UiSequencePlugin;

impl Plugin for UiSequencePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiSequencePlayer>().add_systems(
            Update,
            (receive, advance, camera.after(crate::camera::GameplayCamera), waves, draw, shots)
                .chain()
                .after(crate::logic::LogicTick),
        );
    }
}

/// The logic's binding host (`LogicState`), registered by logic.rs under `--hsm`.
pub struct LogicView(pub Rc<RefCell<mw_logic::natives::LogicState>>);

/// A sequence to show this frame: who plays it (a key stable while it plays), the sequence, its clock.
pub struct Shown {
    pub key: String,
    pub seq: Arc<Sequence>,
    pub clock: f32,
}

#[derive(Resource, Default)]
pub struct UiSequencePlayer {
    /// What the owner wants shown now ([`UiSequencePlayer::show`]).
    wanted: Vec<Shown>,
    playing: Vec<Playing>,
    root: Option<Entity>,
    pool: QuadPool,
    /// HideHud items are live (the HUD was hidden by the player).
    hid_hud: bool,
}

impl UiSequencePlayer {
    /// The sequences that play now (replaces the last frame's list; one left out stops).
    pub fn show(&mut self, wanted: Vec<Shown>) {
        self.wanted = wanted;
    }
}

struct Playing {
    key: String,
    seq: Arc<Sequence>,
    clock: f32,
    /// Conducted children by item index: their own clock from the item's start.
    children: HashMap<usize, f32>,
    job: Option<Arc<Mutex<Option<Loaded>>>>,
    look: Option<Look>,
    movie: Option<Movie>,
    failed_movie: Option<u32>,
    /// Per camera item: when its take started (frozen bindings are sampled then).
    take_started: HashSet<usize>,
    /// Screenshots taken (`MW_UI_SEQUENCE_SHOTS`).
    shots_taken: u32,
}

struct Loaded {
    scene: UiScene,
    slots: HashMap<SlotKey, usize>,
    images: Vec<(ResourceId, Image)>,
    fonts: Vec<(u32, mw_formats::Font)>,
}

struct Look {
    scene: UiScene,
    slots: HashMap<SlotKey, usize>,
    textures: HashMap<ResourceId, (Handle<Image>, Vec2)>,
    anim: UiAnim,
}

/// The sequence's layouts (with its conducted ones), their textures and fonts.
fn load(store: &mw_assets::AssetStore, seq: &Sequence) -> Option<Loaded> {
    let lang = store.language(mw_assets::DEFAULT_LANGUAGE).ok()?;
    let mut wanted: Vec<(SlotKey, u32)> = Vec::new();
    sequence_layouts(seq, &mut wanted);
    // Sorted by layer (stable: item order within a layer), so the scene draws bottom-up.
    wanted.sort_by_key(|(_, layer)| *layer);
    let mut seen = HashSet::new();
    wanted.retain(|(k, _)| seen.insert(*k));
    let mut slots = HashMap::new();
    let mut layouts = Vec::new();
    for ((id, instance), _) in wanted {
        let Ok(mut layout) = store.ui_layout(mw_formats::ui::object_id(id)) else {
            warn!("ui sequence {}: no layout {id}", seq.unique_id);
            continue;
        };
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
    let mut scene = store.ui_scene(layouts, &lang);
    scene.letterbox = true;
    let images =
        scene.textures().into_iter().filter_map(|id| Some((id, game_text::decode_texture(store, id)?))).collect();
    let fonts = scene.fonts.iter().map(|(&n, f)| (n, f.clone())).collect();
    Some(Loaded { scene, slots, images, fonts })
}

/// A UI binding read through the logic's view models: the path (with its `[...]` argument) looked up in
/// `LogicState`'s roots, the value handed to the layout's binding evaluator. `UISequences.Params` reads "" (module
/// doc).
fn logic_lookup(view: Option<&LogicView>, path: &str, arg: Option<&str>) -> Option<BindValue> {
    use mw_logic::binding::{Binding, Host, Lookup, Value};
    if path == "UISequences.Params" {
        return Some(BindValue::Text(String::new()));
    }
    let view = view?;
    let text = match arg {
        Some(a) => format!("{path}[{a}]"),
        None => path.to_string(),
    };
    let b = Binding::parse(&text).ok()?;
    let Lookup::Found(v) = view.0.borrow().lookup(&b.segments) else {
        // Logged once per binding: what the logic's view models do not answer yet.
        thread_local!(static MISSED: RefCell<HashSet<String>> = RefCell::new(HashSet::new()));
        if MISSED.with(|m| m.borrow_mut().insert(text.clone())) {
            info!("ui sequence: binding {text} does not resolve");
        }
        return None;
    };
    Some(match v {
        Value::Bool(b) | Value::Object(b) => b.into(),
        Value::Int(n) => (n as f64).into(),
        Value::UInt(n) | Value::List(n) => (n as f64).into(),
        Value::Int64(n) => (n as f64).into(),
        Value::UInt64(n) => (n as f64).into(),
        Value::Float(f) => (f as f64).into(),
        Value::Double(f) => f.into(),
        Value::Str(s) => BindValue::Text(s),
    })
}

/// An item's `enabled` binding holds (empty = always).
fn enabled(view: Option<&LogicView>, b: &str) -> bool {
    b.is_empty() || binding_true(b, &mut |p, a| logic_lookup(view, p, a))
}

/// Starts and stops the playing sequences as the owner asks, starts their looks' loads.
fn advance(
    mut player: ResMut<UiSequencePlayer>,
    game: Res<crate::Game>,
    real: Res<Time<Real>>,
    view: Option<NonSend<LogicView>>,
    mut images: ResMut<Assets<Image>>,
) {
    let view = view.as_deref();
    let dt = real.delta_secs().min(0.1);
    let UiSequencePlayer { wanted, playing, .. } = &mut *player;
    playing.retain(|p| {
        let keep = wanted.iter().any(|w| w.key == p.key && w.seq.unique_id == p.seq.unique_id);
        if !keep {
            info!("ui sequence: {} of {} stops at {:.2} s", p.seq.unique_id, p.key, p.clock);
        }
        keep
    });
    for w in wanted.iter() {
        let p = match playing.iter_mut().find(|p| p.key == w.key && p.seq.unique_id == w.seq.unique_id) {
            Some(p) => p,
            None => {
                info!("ui sequence: {} of {} starts ({:.2} s long)", w.seq.unique_id, w.key, w.seq.length);
                let job = Arc::new(Mutex::new(None));
                let (out, store, seq) = (job.clone(), game.store.clone(), w.seq.clone());
                let _ = crate::platform::thread::Builder::new().name("mw-ui-sequence".into()).spawn(move || {
                    let t = crate::platform::Instant::now();
                    let loaded = load(&store, &seq);
                    if let Some(l) = &loaded {
                        info!(
                            "ui sequence {}: {} layouts, {} textures in {:.0?}",
                            seq.unique_id,
                            l.scene.layouts.len(),
                            l.images.len(),
                            t.elapsed()
                        );
                    }
                    *out.lock().unwrap() = loaded;
                });
                playing.push(Playing {
                    key: w.key.clone(),
                    seq: w.seq.clone(),
                    clock: w.clock,
                    children: HashMap::new(),
                    job: Some(job),
                    look: None,
                    movie: None,
                    failed_movie: None,
                    take_started: HashSet::new(),
                    shots_taken: 0,
                });
                playing.last_mut().unwrap()
            }
        };
        p.clock = w.clock;
        let clock = p.clock;
        // Conducted children: their own clock (with their jumps) from the item's start while the item runs.
        for (k, i) in p.seq.items.iter().enumerate() {
            let ItemKind::Conductor(child) = &i.kind else { continue };
            if clock < i.start || clock >= i.end {
                p.children.remove(&k);
                continue;
            }
            let t = p.children.get(&k).copied();
            let (n, _) = step(child, t.unwrap_or(clock - i.start), if t.is_some() { dt } else { 0.0 }, &mut |b| {
                binding_true(b, &mut |p, a| logic_lookup(view, p, a))
            });
            p.children.insert(k, n.min(child.length));
        }
        // The movie item active now.
        let active = p.seq.items.iter().find_map(|i| match i.kind {
            ItemKind::Movie { movie, looping } if clock >= i.start && clock < i.end => Some((movie, looping, i.start)),
            _ => None,
        });
        match active {
            Some((id, looping, start)) => {
                if p.movie.as_ref().is_none_or(|m| m.id != id) && p.failed_movie != Some(id) {
                    p.movie = Movie::open(game.store.game(), id, looping, &mut images);
                    if p.movie.is_none() {
                        warn!("ui sequence: movie {id} not playable (no ffmpeg?)");
                        p.failed_movie = Some(id);
                    }
                }
                if let Some(m) = p.movie.as_mut() {
                    m.update(clock - start, &mut images);
                }
            }
            None => p.movie = None,
        }
    }
}

/// Hands the finished looks to the playing sequences (textures and fonts to the main world).
fn receive(
    mut commands: Commands,
    mut player: ResMut<UiSequencePlayer>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<GameFonts>,
) {
    if player.root.is_none() {
        let node = Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        };
        // Under the front end's screens (20): a transition covers what plays.
        player.root = Some(commands.spawn((node, Pickable::IGNORE, GlobalZIndex(19))).id());
    }
    for p in &mut player.playing {
        let Some(loaded) = p.job.as_ref().and_then(|j| j.poll_lock().and_then(|mut s| s.take())) else { continue };
        p.job = None;
        let textures: HashMap<ResourceId, (Handle<Image>, Vec2)> = loaded
            .images
            .into_iter()
            .map(|(id, img)| {
                let size = img.size().as_vec2();
                (id, (images.add(img), size))
            })
            .collect();
        for (number, font) in loaded.fonts {
            if let (false, Some(atlas)) =
                (fonts.fonts.contains_key(&number), font.texture.and_then(|t| textures.get(&t)))
            {
                fonts.fonts.insert(number, LoadedFont { font: Arc::new(font), atlas: atlas.0.clone() });
            }
        }
        p.look = Some(Look { scene: loaded.scene, slots: loaded.slots, textures, anim: UiAnim::default() });
    }
}

/// The sequences' camera items (ICE takes) on the gameplay camera, and their HideHud items.
#[allow(clippy::type_complexity)]
fn camera(
    mut player: ResMut<UiSequencePlayer>,
    view: Option<NonSend<LogicView>>,
    car: Query<&Transform, (With<crate::drive::Car>, With<crate::drive::Player>, Without<Camera3d>)>,
    mut cam: Query<(&mut Transform, &mut Projection), crate::camera::DrivingCamera>,
    mut hud: ResMut<crate::hud::HudSettings>,
    mut snap: ResMut<crate::camera::SnapCamera>,
) {
    let view = view.as_deref();
    let subject = car.single().ok().map_or(Frame::IDENTITY, |t| crate::cutscene::frame_of(*t));
    let mut hide_hud = 0;
    let mut out: Option<(Transform, f32, f32)> = None;
    for p in &mut player.playing {
        let t = p.clock;
        for (index, item) in p.seq.items.iter().enumerate() {
            if !(t >= item.start && t < item.end) || !enabled(view, &item.enabled) {
                p.take_started.remove(&index);
                continue;
            }
            match &item.kind {
                ItemKind::HideHud => hide_hud += 1,
                ItemKind::Camera(shots) => {
                    let Some((shot, take)) = shots.iter().find_map(|s| Some((s, s.take_data.as_ref()?))) else {
                        continue;
                    };
                    if p.take_started.insert(index) {
                        snap.0 = true;
                    }
                    // Bound transforms (`...GetTransform`): the player's car (the recap's take has none).
                    let avatars: Vec<Frame> = shot.bindings.iter().map(|_| subject).collect();
                    let c = take.camera(t - item.start, &Bindings { subject, avatars: &avatars });
                    let (x, y, z) = (Vec3::from(c.frame.x), Vec3::from(c.frame.y), Vec3::from(c.frame.z));
                    let rotation = Quat::from_mat3(&Mat3::from_cols(-x, y, -z)).normalize();
                    let v = Transform { translation: Vec3::from(c.eye), rotation, scale: Vec3::ONE };
                    out = Some((v, c.fov_degrees, c.game_camera_blend.clamp(0.0, 1.0)));
                }
                _ => {}
            }
        }
    }
    // `UISequences.HideHudRequests` (+0x3254): the live HideHud items.
    if let Some(v) = view {
        v.0.borrow_mut().sequences.hide_hud_requests = hide_hud;
    }
    if (hide_hud > 0) != player.hid_hud {
        player.hid_hud = hide_hud > 0;
        hud.hidden_by_sequence = player.hid_hud;
    }
    // The take overrides the gameplay camera, blended back to it by the take's GAME_CAM_BLEND (as cutscene.rs).
    let (Some((v, fov, blend)), Ok((mut cam, mut projection))) = (out, cam.single_mut()) else { return };
    let gameplay_fov = match &*projection {
        Projection::Perspective(pp) => crate::camera::game_fov(pp),
        _ => 60.0,
    };
    *cam = Transform {
        translation: v.translation.lerp(cam.translation, blend),
        rotation: v.rotation.slerp(cam.rotation, blend),
        scale: Vec3::ONE,
    };
    if let Projection::Perspective(pp) = &mut *projection {
        pp.fov = crate::camera::projection_fov(fov + (gameplay_fov - fov) * blend, pp.aspect_ratio);
    }
}

/// The wave items active now (`music.rs` plays them).
fn waves(
    player: Res<UiSequencePlayer>,
    view: Option<NonSend<LogicView>>,
    music: Option<ResMut<crate::music::SequenceWaves>>,
) {
    let Some(mut music) = music else { return };
    let view = view.as_deref();
    let mut active = Vec::new();
    let mut upcoming = Vec::new();
    for p in &player.playing {
        let mut add = |seq: &Sequence, clock: f32| {
            for i in &seq.items {
                let ItemKind::Wave(w) = &i.kind else { continue };
                upcoming.extend(w.waves.iter().copied());
                if clock >= i.start && clock < i.end && enabled(view, &i.enabled) {
                    active.push((i.unique_id, w.clone(), clock - i.start));
                }
            }
        };
        add(&p.seq, p.clock);
        for (&k, &t) in &p.children {
            if let ItemKind::Conductor(child) = &p.seq.items[k].kind {
                add(child, t);
            }
        }
    }
    music.player = active;
    music.player_upcoming = upcoming;
}

/// Draws the playing sequences' layouts (bottom-up by layer within each sequence).
#[allow(clippy::too_many_arguments)]
fn draw(
    mut commands: Commands,
    mut player: ResMut<UiSequencePlayer>,
    view: Option<NonSend<LogicView>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    time: Res<Time<Real>>,
    mut nodes: Query<PoolNode, With<QuadNode>>,
    mut materials: ResMut<Assets<AdditiveQuadMaterial>>,
) {
    let view = view.as_deref();
    let Some(root) = player.root else { return };
    let Ok(window) = windows.single() else { return };
    let screen = [window.width(), window.height()];
    let full = |colour: [f32; 4], texture: Option<ResourceId>| Quad {
        texture,
        uv: [0.0, 0.0, 1.0, 1.0],
        rect: [0.0, 0.0, screen[0], screen[1]],
        rotation: 0.0,
        colour,
        blend: UiBlend::Alpha,
        map: false,
    };
    let now = time.elapsed_secs_f64();
    let mut quads: Vec<Quad> = Vec::new();
    let mut textures: HashMap<ResourceId, (Handle<Image>, Vec2)> = HashMap::new();
    let UiSequencePlayer { playing, pool, .. } = &mut *player;
    for p in playing.iter_mut() {
        let Some(look) = p.look.as_mut() else { continue };
        let mut active: Vec<usize> = Vec::new();
        let mut add = |seq: &Sequence, clock: f32, quads: &mut Vec<Quad>| {
            for i in seq.items.iter().filter(|i| clock >= i.start && clock < i.end) {
                if !enabled(view, &i.enabled) {
                    continue;
                }
                match &i.kind {
                    ItemKind::Layout { layout: Some(l), instance, .. } => {
                        if let Some(&s) = look.slots.get(&((l.0 & 0xffff_ffff) as u32, *instance)) {
                            active.push(s);
                        }
                    }
                    ItemKind::BlackQuad => quads.push(full([0.0, 0.0, 0.0, 1.0], None)),
                    _ => {}
                }
            }
        };
        if let Some(m) = p.movie.as_ref().filter(|m| m.ready) {
            textures.insert(MOVIE_KEY, (m.image.clone(), Vec2::new(640.0, 360.0)));
            let rect = mw_assets::ui::UiFrame::letterbox(screen).rect();
            quads.push(Quad { rect, ..full([1.0; 4], Some(MOVIE_KEY)) });
        }
        add(&p.seq, p.clock, &mut quads);
        let mut children: Vec<(usize, f32)> = p.children.iter().map(|(&k, &t)| (k, t)).collect();
        children.sort_by_key(|c| c.0);
        for (k, t) in children {
            if let ItemKind::Conductor(child) = &p.seq.items[k].kind {
                add(child, t, &mut quads);
            }
        }
        if !active.is_empty() {
            let set: HashSet<usize> = active.into_iter().collect();
            let mut options = DrawOptions::new(Some(Box::new(move |i| set.contains(&i))), Some(&mut look.anim), now);
            let mut lk = |path: &str, a: Option<&str>| logic_lookup(view, path, a);
            quads.extend(look.scene.draw_with(screen, &mut lk, &mut options));
        }
        textures.extend(look.textures.iter().map(|(k, v)| (*k, v.clone())));
    }
    // The black bars outside the letterboxed frame (`FUN_006240b0`).
    if !quads.is_empty() {
        quads.extend(mw_assets::ui::UiFrame::letterbox(screen).bars(screen));
    }
    show_quads(&mut commands, root, pool, &quads, &textures, None, &mut nodes, &mut materials);
}

/// `MW_UI_SEQUENCE_SHOTS=<dir>`: screenshots of each playing sequence at 1, 4, 8, 12, 16 and 20 s of its clock,
/// exit after the last (headless checks).
fn shots(
    mut commands: Commands,
    mut player: ResMut<UiSequencePlayer>,
    mut done: Local<Option<u32>>,
    mut exit: MessageWriter<AppExit>,
) {
    const AT: [f32; 6] = [1.0, 4.0, 8.0, 12.0, 16.0, 20.0];
    let Some(dir) = std::env::var_os("MW_UI_SEQUENCE_SHOTS").map(std::path::PathBuf::from) else { return };
    // Exit 30 frames after a sequence's last shot.
    if let Some(n) = done.as_mut() {
        *n += 1;
        if *n == 30 {
            exit.write(AppExit::Success);
        }
        return;
    }
    for p in &mut player.playing {
        let Some(&at) = AT.get(p.shots_taken as usize) else { continue };
        if p.clock < at || p.look.is_none() {
            continue;
        }
        p.shots_taken += 1;
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{}-{:02}s.png", p.seq.unique_id, at as u32));
        info!("ui sequence: screenshot {} at {:.2} s", path.display(), p.clock);
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(path));
        if p.shots_taken as usize == AT.len() {
            *done = Some(0);
        }
    }
}
