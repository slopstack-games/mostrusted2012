//! The widget-tree renderer: draws the widget groups the logic has active (`mw_logic::natives::pages`, published
//! by logic.rs after each HSM update as [`ActiveWidgetGroups`]), the way the original's widget screen manager
//! shows its screens' layouts: every layout of every widget of each active group, through the layout renderer
//! (`UiScene::draw_with`, `ui_quads.rs`). Which of a widget's layouts and elements show is the data's: the
//! layout instances' visibility bindings, their triggered timelines and the elements' visibility bindings
//! (research log 2026-11-18 "UI port (widget-tree renderer)"). The groups themselves are the HSM's: e.g. the HUD
//! screen 371621's `Nitrous` group follows `Players.LocalPlayer.CurrentNitrousType` ("Choose Nitrous" 1524930),
//! `AdditionalHUD`'s group 1695018 goes while Easydrive is open ("Run No EasyDrive HUD" 1524982), the `Prompt`
//! group 1135011 shows while the Easydrive prompt waits for input ("With EasyDrivePrompt Offline" 1135035).
//!
//! **Bindings** resolve through the runtime's HUD feed first ([`crate::hud::HudFeed`]: the car, `HudState`, the
//! pursuit, Easydrive), then the logic's view models (`ui_sequence::logic_lookup`: `Widget.*`, `Mode.*`,
//! `Players.*` the logic answers). The fitted nitrous mod goes to the logic as the local player's nitrous record
//! (`CurrentNitrousType`, [`crate::hud::nitrous_record`]).
//!
//! **Screens drawn here**: those in [`DRAWN_SCREENS`] (the HUD 371621); the other screens the logic pushes are
//! still drawn by their hand-written modules (front end, pause, Easydrive's menu) until they move here. Groups of
//! a drawn screen whose look another module still draws are left out ([`DRAWN_ELSEWHERE`]).
//!
//! **The minimap** (`Widget_PointOfInterest`, native `POIDisplayScript`, not ported): the `Map` element's quad
//! shows `hud.rs`'s stand-in (the map-tile mosaic and the point-of-interest icons, [`crate::hud::Minimap`]),
//! spliced in at the element's place in the draw order. The stand-in is drawn for the rectangle of the previous
//! frame's map element.
//!
//! Model decisions (not traced in the code): layouts draw in widget `LayoutLayer` order (0 below maps .. 6
//! overlay), then screen order of the widgets and their layouts, then activation order of screens (the order
//! across groups and screens was not traced); a screen whose layout instances bind `UIConfig.HudAspectCorrection`
//! is a HUD screen (the 16:9 HUD frame with its aspect correction), any other screen is letterboxed (the
//! original's letterbox belongs to a UI camera override, UI manager +0x2ecf0 / +0x2ecf4, whose writer was not
//! found); a HUD screen hides with the runtime's HUD (`HudSettings`: pause menu, hiding sequences, no car); an
//! instance's triggered timelines are applied as their `FADE` steps only (opacity of the whole layout over the
//! step; `MOVE` / `APPROACH` steps show settled), the visibility timeline's show transition is not played; a
//! timeline trigger the feed and the logic do not answer reads 0 before its modifiers (the speedwall panel 1470201
//! fades out on `Mode.HasSpeedwallTargetTime(NOT)`).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mw_assets::ui::{DrawOptions, PlacedLayout, Quad, UiAnim, UiScene};
use mw_bundle::ResourceId;
use mw_formats::binding::{BindValue, Binding};

use crate::game_text::{self, GameFonts, LoadedFont};
use crate::platform::PollLock;
use crate::ui_quads::{AdditiveQuadMaterial, PoolNode, QuadNode, QuadPool, show_quads};
use crate::ui_sequence::LogicView;

/// Screens this renderer draws (their hand-written drawing removed): the free-roam HUD "FreedriveHud" 371621 and
/// the offline race HUD 604153 (the race game mode's "Race HUD" page) and the intro's HUD 1560359 ("Intro HUD"
/// 1560357, the Intro game mode's page: its minimap shows the waypoint POIs, `poi.rs`).
pub const DRAWN_SCREENS: [u32; 3] = [371_621, 604_153, 1_560_359];

/// Widget groups of drawn screens whose look another module still draws: the `EasyDrive` widget's group 1135023
/// (the open menu and its prompt, `easydrive.rs`).
const DRAWN_ELSEWHERE: [u32; 1] = [1_135_023];

pub struct UiWidgetsPlugin;

impl Plugin for UiWidgetsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActiveWidgetGroups>()
            .init_resource::<WidgetScreens>()
            .add_systems(Update, feed_logic.before(crate::logic::LogicTick))
            .add_systems(Update, (load, draw).chain().after(crate::logic::LogicTick).after(crate::hud::HudDraw));
    }
}

/// The logic's active widget groups as (screen, group), in activation order (`Pages::active_screen_groups`),
/// set by logic.rs after the HSM update.
#[derive(Resource, Default)]
pub struct ActiveWidgetGroups {
    pub list: Vec<(u32, u32)>,
}

impl ActiveWidgetGroups {
    pub fn set(&mut self, list: Vec<(u32, u32)>) {
        if self.list != list {
            self.list = list;
        }
    }
}

/// One screen's look: all its widgets' layouts in draw order, each with its widget's group.
struct ScreenLook {
    scene: UiScene,
    /// Per layout of `scene`: the widget group it belongs to.
    groups: Vec<u32>,
    textures: HashMap<ResourceId, (Handle<Image>, Vec2)>,
    anim: UiAnim,
    /// Per (layout, timeline): the trigger's value and when it last changed (`None`: never changed, settled).
    timelines: HashMap<(usize, usize), (bool, Option<f64>)>,
}

struct LoadedScreen {
    scene: UiScene,
    groups: Vec<u32>,
    images: Vec<(ResourceId, Image)>,
    fonts: Vec<(u32, mw_formats::Font)>,
}

#[derive(Resource, Default)]
struct WidgetScreens {
    looks: HashMap<u32, ScreenLook>,
    jobs: HashMap<u32, Arc<Mutex<Option<Option<LoadedScreen>>>>>,
    root: Option<Entity>,
    pool: QuadPool,
    /// The minimap stand-in's textures merged in (`hud::Minimap::generation`).
    minimap_generation: u32,
    extra_textures: HashMap<ResourceId, (Handle<Image>, Vec2)>,
}

/// The screen's widgets' layouts sorted into draw order (module doc), with their groups.
fn load_screen(store: &mw_assets::AssetStore, id: u32) -> Option<LoadedScreen> {
    let bundle = format!("UI/SCREENS2/{id}.BNDL");
    let lang = store.language(mw_assets::DEFAULT_LANGUAGE).ok()?;
    let screen = store.ui_screen(&bundle).map_err(|e| warn!("ui widgets: screen {id}: {e}")).ok()?;
    let mut placed: Vec<(u64, usize, usize, PlacedLayout)> = store
        .ui_screen_layouts(&bundle, None)
        .map_err(|e| warn!("ui widgets: screen {id} layouts: {e}"))
        .ok()?
        .into_iter()
        .enumerate()
        .map(|(n, l)| (screen.widgets.get(l.widget_index).map_or(0, |w| w.layer), l.widget_index, n, l))
        .collect();
    placed.sort_by_key(|p| (p.0, p.1, p.2));
    let hud = placed.iter().any(|p| {
        p.3.instance
            .as_ref()
            .is_some_and(|i| i.transforms.iter().any(|t| t.binding.contains("UIConfig.HudAspectCorrection")))
    });
    let mut groups = Vec::new();
    let mut layouts = Vec::new();
    for (_, wi, _, mut l) in placed {
        l.layout.elements.retain(|e| {
            let shader = e.material.and_then(|m| store.ui_material_shader(m));
            !shader.as_deref().is_some_and(|s| crate::frontend::UNPORTED_SHADERS.contains(&s))
        });
        groups.push(screen.widgets.get(wi).map_or(0, |w| w.group));
        layouts.push(l);
    }
    let mut scene = store.ui_scene(layouts, &lang);
    scene.letterbox = !hud;
    let images = scene.textures().into_iter().filter_map(|t| Some((t, game_text::decode_texture(store, t)?))).collect();
    let fonts = scene.fonts.iter().map(|(&n, f)| (n, f.clone())).collect();
    info!("ui widgets: screen {id}: {} layouts, hud frame {hud}", scene.layouts.len());
    Some(LoadedScreen { scene, groups, images, fonts })
}

/// Starts the loads of drawn screens that have an active group, takes in the finished ones.
fn load(
    mut screens: ResMut<WidgetScreens>,
    active: Res<ActiveWidgetGroups>,
    game: Res<crate::Game>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<GameFonts>,
) {
    let WidgetScreens { looks, jobs, .. } = &mut *screens;
    for &(screen, _) in &active.list {
        if !DRAWN_SCREENS.contains(&screen) || looks.contains_key(&screen) || jobs.contains_key(&screen) {
            continue;
        }
        let slot = Arc::new(Mutex::new(None));
        let (out, store) = (slot.clone(), game.store.clone());
        let spawned = crate::platform::thread::Builder::new().name("mw-widgets".into()).spawn(move || {
            *out.lock().unwrap() = Some(load_screen(&store, screen));
        });
        if spawned.is_ok() {
            jobs.insert(screen, slot);
        }
    }
    let done: Vec<u32> =
        jobs.iter().filter(|(_, j)| j.poll_lock().is_some_and(|g| g.is_some())).map(|(&k, _)| k).collect();
    for id in done {
        let Some(job) = jobs.remove(&id) else { continue };
        let Some(Some(loaded)) = job.lock().unwrap().take() else {
            // Failed: an empty look, so it is not tried again every frame.
            looks.insert(id, empty_look());
            continue;
        };
        let textures: HashMap<_, _> = loaded
            .images
            .into_iter()
            .map(|(t, img)| {
                let size = img.size().as_vec2();
                (t, (images.add(img), size))
            })
            .collect();
        for (number, font) in loaded.fonts {
            if let (false, Some(atlas)) =
                (fonts.fonts.contains_key(&number), font.texture.and_then(|t| textures.get(&t)))
            {
                fonts.fonts.insert(number, LoadedFont { font: Arc::new(font), atlas: atlas.0.clone() });
            }
        }
        looks.insert(
            id,
            ScreenLook {
                scene: loaded.scene,
                groups: loaded.groups,
                textures,
                anim: UiAnim::default(),
                timelines: HashMap::new(),
            },
        );
    }
}

fn empty_look() -> ScreenLook {
    ScreenLook {
        scene: UiScene::default(),
        groups: Vec::new(),
        textures: HashMap::new(),
        anim: UiAnim::default(),
        timelines: HashMap::new(),
    }
}

/// The engine state the HUD's groups wait on, set on the logic's local player every frame before the logic's
/// update: the nitrous record from the fitted mod (`CurrentNitrousType`, the HSM's "Choose Nitrous" 1524930), the
/// player's car existing (`FUN_00573390`, `vehicle_alive`) and its engine running (`IsEngineOn`, `FUN_00572c20`:
/// `ignition::EngineStart` past cranking; "Some HUD Elements Visible" 1014536 waits in "Wait For Engine On"
/// 979873 before it activates the HUD groups).
fn feed_logic(
    view: Option<NonSend<LogicView>>,
    mods: Option<Res<crate::mods_menu::ModSelection>>,
    catalogue: Option<Res<crate::mods_menu::ModCatalogue>>,
    car: Query<Option<&crate::ignition::EngineStart>, (With<crate::drive::Car>, With<crate::drive::Player>)>,
) {
    let Some(view) = view else { return };
    let record = match (&mods, &catalogue) {
        (Some(m), Some(c)) => crate::hud::nitrous_record(m, c),
        _ => (0, 0),
    };
    let car = car.single().ok();
    let mut st = view.0.borrow_mut();
    // `Camera.IsCockpitCam` ("Select HUD"'s "Is using cockpit cam" 371574 picks the internal or the third-person
    // HUDs; unanswered it exits through nothing and no HUD runs). Model decision: false, the runtime has no
    // cockpit camera (its bonnet / bumper views are not the cockpit); published as a view model stand-in for the
    // camera manager's binding root, once.
    if st.view_models.published("Camera").is_none() {
        let camera = mw_formats::json::Json::Object(vec![("IsCockpitCam".into(), mw_formats::json::Json::Bool(false))]);
        st.view_models.publish("Camera", camera);
    }
    let Some(p) = st.players.local_player_mut() else { return };
    p.nitrous = Some(record);
    p.vehicle_alive = car.is_some() && p.vehicle != -1;
    // A car without start-up states (test runs) runs.
    p.engine_on = car.is_some_and(|e| e.is_none_or(|e| e.running()));
}

/// Opacity of a `FADE(delay, duration, IN|OUT, ...)` step `since` seconds after it started (1 without one).
fn fade(timeline: &str, since: Option<f64>) -> f32 {
    let Some(start) = timeline.find("FADE") else { return 1.0 };
    let rest = &timeline[start + 4..];
    let Some(args) = rest.trim_start().strip_prefix('(').and_then(|r| r.split(')').next()) else { return 1.0 };
    let parts: Vec<&str> = args.split(',').map(str::trim).collect();
    let num = |i: usize| parts.get(i).and_then(|p| p.parse::<f64>().ok()).unwrap_or(0.0);
    let (delay, duration) = (num(0) / 1000.0, num(1) / 1000.0);
    let p = match since {
        None => 1.0,
        Some(s) if duration <= 0.0 => f64::from(s >= delay),
        Some(s) => ((s - delay) / duration).clamp(0.0, 1.0),
    } as f32;
    match parts.get(2).copied() {
        Some("IN") => p,
        Some("OUT") => 1.0 - p,
        _ => 1.0,
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    mut commands: Commands,
    mut screens: ResMut<WidgetScreens>,
    active: Res<ActiveWidgetGroups>,
    feed: Option<Res<crate::hud::HudFeed>>,
    mut minimap: Option<ResMut<crate::hud::Minimap>>,
    view: Option<NonSend<LogicView>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    time: Res<Time<Real>>,
    mut nodes: Query<PoolNode, With<QuadNode>>,
    mut materials: ResMut<Assets<AdditiveQuadMaterial>>,
) {
    let view = view.as_deref();
    let root = *screens.root.get_or_insert_with(|| {
        commands
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
            .id()
    });
    let Ok(window) = windows.single() else { return };
    let screen = [window.width(), window.height()];
    let now = time.elapsed_secs_f64();
    let hud_visible = feed.as_ref().is_some_and(|f| f.visible);
    let metric = feed.as_ref().is_none_or(|f| f.metric);
    let mut lookup = |path: &str, arg: Option<&str>| -> Option<BindValue> {
        feed.as_ref().and_then(|f| f.lookup(path, arg)).or_else(|| crate::ui_sequence::logic_lookup(view, path, arg))
    };
    // The minimap stand-in's textures (its icons), merged once per change.
    let WidgetScreens { looks, pool, minimap_generation, extra_textures, .. } = &mut *screens;
    if let Some(m) = minimap.as_ref()
        && m.generation != *minimap_generation
    {
        *minimap_generation = m.generation;
        extra_textures.extend(m.textures.iter().map(|(k, v)| (*k, v.clone())));
    }
    let mut quads: Vec<Quad> = Vec::new();
    let mut map_quad = None;
    let mut drawn_screens: Vec<u32> = Vec::new();
    for &(id, _) in &active.list {
        if !drawn_screens.contains(&id) {
            drawn_screens.push(id);
        }
    }
    for id in drawn_screens {
        let Some(look) = looks.get_mut(&id) else { continue };
        if !look.scene.letterbox && !hud_visible {
            continue;
        }
        let groups: HashSet<u32> =
            active.list.iter().filter(|(s, g)| *s == id && !DRAWN_ELSEWHERE.contains(g)).map(|&(_, g)| g).collect();
        look.scene.metric = Some(metric);
        let ScreenLook { scene, groups: layout_groups, anim, timelines, .. } = look;
        for (index, pl) in scene.layouts.iter().enumerate() {
            if !layout_groups.get(index).is_some_and(|g| groups.contains(g)) {
                continue;
            }
            // The instance's triggered timelines: the `FADE` of the one playing (module doc).
            let mut opacity = 1.0;
            for (k, tl) in pl.instance.as_ref().map_or(&[][..], |i| &i.timelines[..]).iter().enumerate() {
                // An unanswered trigger value counts as 0 before its modifiers (`(NOT)` makes it true).
                let mut or_zero = |p: &str, a: Option<&str>| lookup(p, a).or(Some(BindValue::Num(0.0)));
                let on = Binding::parse(&tl.trigger).and_then(|b| b.eval(&mut or_zero)).is_some_and(|v| v.truthy());
                let entry = timelines.entry((index, k)).or_insert((on, None));
                if entry.0 != on {
                    *entry = (on, Some(now));
                }
                let since = entry.1.map(|t| now - t);
                opacity *= fade(if on { &tl.on } else { &tl.off }, since);
            }
            if opacity <= 0.0 {
                continue;
            }
            let mut options = DrawOptions::new(Some(Box::new(move |i| i == index)), Some(&mut *anim), now);
            for mut q in scene.draw_with(screen, &mut lookup, &mut options) {
                q.colour[3] *= opacity;
                if q.map {
                    crate::hud::minimap_quad(&mut q);
                    map_quad = Some(q.clone());
                    quads.push(q);
                    // The stand-in's icons over the map tiles.
                    if let Some(m) = minimap.as_ref() {
                        quads.extend(m.icons.iter().cloned());
                    }
                    continue;
                }
                quads.push(q);
            }
        }
    }
    if let Some(m) = minimap.as_mut() {
        m.quad = map_quad;
    }
    let mut textures: HashMap<ResourceId, (Handle<Image>, Vec2)> = extra_textures.clone();
    for look in looks.values() {
        textures.extend(look.textures.iter().map(|(k, v)| (*k, v.clone())));
    }
    let map_image = minimap.as_ref().and_then(|m| m.image.clone());
    show_quads(&mut commands, root, pool, &quads, &textures, map_image, &mut nodes, &mut materials);
}

#[cfg(test)]
mod tests {
    use super::fade;

    #[test]
    fn fade_steps() {
        assert_eq!(fade("FADE(0,200,OUT,EASEINOUT)", None), 0.0);
        assert_eq!(fade("APPROACH (0, 400, -400, 0, EASEIN), FADE(0, 200, IN, EASEINOUT)", None), 1.0);
        assert!((fade("FADE(0, 1000, IN)", Some(0.5)) - 0.5).abs() < 1e-6);
        assert_eq!(fade("MOVE(0,0,0,27)", Some(0.0)), 1.0);
    }
}
