//! The driving HUD, drawn from the game's own UI data: the free-roam screen's layouts
//! (`UI/SCREENS2/371670.BNDL`), its textures and bitmap fonts, with the bindings the layouts name filled
//! from the car (`mw_assets::ui::UiScene::draw` turns them into quads; this module shows the quads as Bevy UI
//! nodes). The minimap is the game's map-tile grid, composited on the CPU heading-up.
//!
//! The game decides which layouts a widget shows in code (widget adapters); here a fixed free-roam set
//! stands in: speedometer + nitrous gauge, odometer, minimap frame, road / district name. F3 toggles the
//! debug status text.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

use bevy::prelude::*;
use mw_assets::ui::{FREE_ROAM_HUD, MapMosaic, UiScene};
use mw_bundle::ResourceId;
use mw_formats::binding::BindValue;

use crate::game_text::{self, GameFonts, LoadedFont};

/// Layouts shown while driving in free roam (UniqueIds in `FREE_ROAM_HUD`): the first `Nitrous` widget's
/// speedometer + nitrous gauge (1670359) and odometer (1806698), and `AdditionalHUD`'s minimap frame
/// (1206246) and road / district name plates (1990196). Pursuit, event and EasyDrive layouts need game
/// modes we don't have yet.
pub const FREE_ROAM_LAYOUTS: [u32; 4] = [1_206_246, 1_990_196, 1_670_359, 1_806_698];

/// Minimap scale in metres per reference pixel (1280×720 frame): the map tiles' own resolution
/// (256 px per 1036 m tile). The game's map camera (`UIMapCameraConfig`: distance 600 m bound to
/// `Mode.MinimapDistance`, FOV 90°) isn't reproduced yet.
pub const MINIMAP_METRES_PER_PIXEL: f32 = 4.0;

pub struct HudPlugin {
    /// Show the HUD (not in vehicle-viewer or test runs).
    pub enabled: bool,
    pub metric: bool,
}

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(HudSettings { enabled: self.enabled, metric: self.metric, debug_text: false })
            .init_resource::<HudState>()
            .add_systems(Startup, start_loading)
            .add_systems(Update, (receive_hud, toggle_debug, track_car, draw_hud).chain());
    }
}

#[derive(Resource)]
pub struct HudSettings {
    pub enabled: bool,
    /// km/h and km (the game's `Language.IsMetric`).
    pub metric: bool,
    /// F3: the debug status text instead of / with the HUD.
    pub debug_text: bool,
}

/// Game state the bindings read that the car doesn't keep.
#[derive(Resource, Default)]
pub struct HudState {
    /// Distance driven (m).
    pub odometer: f64,
    last_position: Option<Vec3>,
    /// 0–1. Nitrous isn't simulated yet: the gauge shows a full tank.
    pub nitrous: f32,
}

struct HudAssets {
    scene: UiScene,
    images: Vec<(ResourceId, Image)>,
    fonts: Vec<(u32, mw_formats::Font)>,
    mosaic: Option<MapMosaic>,
}

#[derive(Resource)]
struct HudLoading(Mutex<Receiver<HudAssets>>);

#[derive(Resource)]
struct Hud {
    scene: UiScene,
    textures: HashMap<ResourceId, (Handle<Image>, Vec2)>,
    root: Entity,
    pool: Vec<Entity>,
    map: Option<MapView>,
}

struct MapView {
    mosaic: MapMosaic,
    image: Handle<Image>,
    size: UVec2,
    frame: u32,
}

/// Build the HUD scene on a thread: decode the screen's layouts, styles, fonts and textures.
fn start_loading(mut commands: Commands, game: Res<crate::Game>, settings: Res<HudSettings>) {
    if !settings.enabled {
        return;
    }
    let store = game.store.clone();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let t = std::time::Instant::now();
        let layouts = match store.ui_screen_layouts(FREE_ROAM_HUD, None) {
            Ok(l) => l.into_iter().filter(|l| FREE_ROAM_LAYOUTS.contains(&l.layout.id)).collect(),
            Err(e) => {
                warn!("HUD: no screen {FREE_ROAM_HUD} ({e})");
                return;
            }
        };
        let lang = match store.language(mw_assets::DEFAULT_LANGUAGE) {
            Ok(l) => l,
            Err(e) => {
                warn!("HUD: no language table ({e})");
                return;
            }
        };
        let scene = store.ui_scene(layouts, &lang);
        let images =
            scene.textures().into_iter().filter_map(|id| Some((id, game_text::decode_texture(&store, id)?))).collect();
        let fonts = scene.fonts.iter().map(|(&n, f)| (n, f.clone())).collect();
        let mosaic = store.ui_map_grid().ok().map(|g| MapMosaic::build(&store, g));
        info!(
            "HUD: {} layouts, {} fonts, minimap {} in {:.0?}",
            scene.layouts.len(),
            scene.fonts.len(),
            mosaic.is_some(),
            t.elapsed()
        );
        let _ = tx.send(HudAssets { scene, images, fonts, mosaic });
    });
    commands.insert_resource(HudLoading(Mutex::new(rx)));
}

fn receive_hud(
    mut commands: Commands,
    loading: Option<Res<HudLoading>>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<GameFonts>,
) {
    let Some(loading) = loading else { return };
    let Ok(assets) = loading.0.lock().unwrap().try_recv() else { return };
    commands.remove_resource::<HudLoading>();
    let textures = assets
        .images
        .into_iter()
        .map(|(id, img)| {
            let size = img.size().as_vec2();
            (id, (images.add(img), size))
        })
        .collect::<HashMap<_, _>>();
    for (number, font) in assets.fonts {
        if let (false, Some(atlas)) = (fonts.fonts.contains_key(&number), font.texture.and_then(|t| textures.get(&t))) {
            fonts.fonts.insert(number, LoadedFont { font: std::sync::Arc::new(font), atlas: atlas.0.clone() });
        }
    }
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            Pickable::IGNORE,
            GlobalZIndex(3),
        ))
        .id();
    let map = assets.mosaic.map(|mosaic| MapView {
        mosaic,
        image: images.add(game_text::ui_image(1, 1, vec![0; 4])),
        size: UVec2::ONE,
        frame: 0,
    });
    commands.insert_resource(Hud { scene: assets.scene, textures, root, pool: Vec::new(), map });
}

/// F3 shows the debug status text (hidden while the HUD is up).
fn toggle_debug(
    keys: Res<ButtonInput<KeyCode>>,
    mut settings: ResMut<HudSettings>,
    hud: Option<Res<Hud>>,
    car: Query<(), With<crate::drive::Car>>,
    mut status: Query<&mut Visibility, With<crate::StatusText>>,
) {
    if keys.just_pressed(KeyCode::F3) {
        settings.debug_text = !settings.debug_text;
    }
    let hud_up = settings.enabled && hud.is_some() && !car.is_empty();
    let show = settings.debug_text || !hud_up;
    for mut v in &mut status {
        let want = if show { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    }
}

fn track_car(mut state: ResMut<HudState>, car: Query<&GlobalTransform, With<crate::drive::Car>>) {
    let Ok(t) = car.single() else {
        state.last_position = None;
        return;
    };
    let p = t.translation();
    if let Some(last) = state.last_position {
        let d = p.distance(last) as f64;
        // Ignore teleports (resets, car swaps).
        if d < 50.0 {
            state.odometer += d;
        }
    }
    state.last_position = Some(p);
    state.nitrous = 1.0;
}

/// Values for the bindings the free-roam layouts use.
fn lookup(
    path: &str,
    arg: Option<&str>,
    car: Option<(&crate::drive::Car, &GlobalTransform)>,
    state: &HudState,
    metric: bool,
    t: f64,
) -> Option<BindValue> {
    const MPH: f64 = 2.236_936;
    let (car, xf) = car.unzip();
    Some(match path {
        // Miles per hour: the layouts convert (`MULT:1.60934` for km/h).
        "Players.GetLocalCompetitor.GetPlayerSpeed" => (car?.speed as f64 * MPH).into(),
        "Language.IsMetric" => metric.into(),
        "Players.LocalPlayer.NitrousState.NitrousAmount" => (state.nitrous as f64).into(),
        "Players.LocalPlayer.NitrousState.UsingNitrous" | "Players.LocalPlayer.NitrousState.InOverdrive" => {
            false.into()
        }
        "Widget.Nitrous.ShowOdometer" => true.into(),
        "Widget.Nitrous.Odometer" => (state.odometer / if metric { 1000.0 } else { 1609.344 }).into(),
        "Signals.GetSineWave" => {
            let period = arg.and_then(|a| a.parse::<f64>().ok()).unwrap_or(1000.0) / 1000.0;
            (0.5 + 0.5 * (t / period.max(0.001) * std::f64::consts::TAU).sin()).into()
        }
        "Players.LocalPlayer.GetPlayerDirection" => {
            let f = xf?.forward();
            (f.x.atan2(f.z).to_degrees() as f64).into()
        }
        "Players.LocalPlayer.IsInChase"
        | "Players.LocalPlayer.IsInCooldown"
        | "Players.LocalPlayer.IsInChaseOrCooldown"
        | "Players.LocalPlayer.Jammed"
        | "Players.LocalPlayer.IsInTunnel" => false.into(),
        _ => return None,
    })
}

type PoolNode<'a> = (&'a mut Node, &'a mut ImageNode, &'a mut UiTransform, &'a mut Visibility);

#[allow(clippy::too_many_arguments)]
fn draw_hud(
    mut commands: Commands,
    hud: Option<ResMut<Hud>>,
    settings: Res<HudSettings>,
    state: Res<HudState>,
    car: Query<(&crate::drive::Car, &GlobalTransform)>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    time: Res<Time>,
    mut nodes: Query<PoolNode>,
    mut images: ResMut<Assets<Image>>,
    mut root_vis: Query<&mut Visibility, Without<ImageNode>>,
) {
    let Some(mut hud) = hud else { return };
    let car = car.single().ok();
    let visible = settings.enabled && car.is_some();
    if let Ok(mut v) = root_vis.get_mut(hud.root) {
        let want = if visible { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    }
    if !visible {
        return;
    }
    let Ok(window) = windows.single() else { return };
    let screen = [window.width(), window.height()];
    let t = time.elapsed_secs_f64();
    let metric = settings.metric;
    let quads = hud.scene.draw(screen, &mut |path, arg| lookup(path, arg, car, &state, metric, t));

    // The minimap: re-render the heading-up view into its image (every other frame).
    let mut map_image = None;
    if let (Some(q), Some(map), Some((_, xf))) = (quads.iter().find(|q| q.map), hud.map.as_mut(), car) {
        let scale = window.scale_factor();
        let size = UVec2::new((q.rect[2] * scale).round().max(1.0) as u32, (q.rect[3] * scale).round().max(1.0) as u32);
        map.frame += 1;
        if size != map.size || map.frame % 2 == 0 {
            let p = xf.translation();
            let f = xf.forward();
            // Metres per physical pixel: the reference scale over the HUD's pixel scale.
            let ref_scale = (screen[0] / 1280.0).min(screen[1] / 720.0) * scale;
            let mut px = vec![0u8; (size.x * size.y * 4) as usize];
            map.mosaic.render(p.x, p.z, [f.x, f.z], MINIMAP_METRES_PER_PIXEL / ref_scale, size.x, size.y, &mut px);
            map.size = size;
            if let Some(mut img) = images.get_mut(&map.image) {
                *img = game_text::ui_image(size.x, size.y, px);
            }
        }
        map_image = Some(map.image.clone());
    }

    // Grow the pool of quad nodes as needed.
    while hud.pool.len() < quads.len() {
        let e = commands
            .spawn((
                Node { position_type: PositionType::Absolute, ..default() },
                ImageNode::default(),
                UiTransform::default(),
                Visibility::Hidden,
                Pickable::IGNORE,
                ChildOf(hud.root),
            ))
            .id();
        hud.pool.push(e);
    }
    for (i, &e) in hud.pool.iter().enumerate() {
        let Ok((mut node, mut image, mut transform, mut vis)) = nodes.get_mut(e) else { continue };
        let Some(q) = quads.get(i) else {
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
            continue;
        };
        let (handle, rect, flip) = match (q.map, q.texture.and_then(|t| hud.textures.get(&t))) {
            (true, _) => (map_image.clone().unwrap_or_default(), None, (false, false)),
            (false, Some((h, size))) => {
                let [u0, v0, u1, v1] = q.uv;
                let r = Rect::new(u0.min(u1) * size.x, v0.min(v1) * size.y, u0.max(u1) * size.x, v0.max(v1) * size.y);
                (h.clone(), Some(r), (u0 > u1, v0 > v1))
            }
            (false, None) if q.texture.is_some() => {
                *vis = Visibility::Hidden;
                continue;
            }
            // Solid colour: Bevy's default image is one white pixel.
            (false, None) => (Handle::default(), None, (false, false)),
        };
        let color = Color::linear_rgba(q.colour[0], q.colour[1], q.colour[2], q.colour[3]);
        if image.image != handle
            || image.color != color
            || image.rect != rect
            || image.flip_x != flip.0
            || image.flip_y != flip.1
        {
            *image = ImageNode { image: handle, color, rect, flip_x: flip.0, flip_y: flip.1, ..default() };
        }
        let (left, top, w, h) = (Val::Px(q.rect[0]), Val::Px(q.rect[1]), Val::Px(q.rect[2]), Val::Px(q.rect[3]));
        if node.left != left || node.top != top || node.width != w || node.height != h {
            node.left = left;
            node.top = top;
            node.width = w;
            node.height = h;
        }
        let rot = Rot2::degrees(q.rotation);
        if transform.rotation != rot {
            transform.rotation = rot;
        }
        if *vis != Visibility::Inherited {
            *vis = Visibility::Inherited;
        }
    }
}
