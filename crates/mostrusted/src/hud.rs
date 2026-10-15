//! The driving HUD, drawn from the game's own UI data: the free-roam screen's layouts
//! (`UI/SCREENS2/371670.BNDL`), the race HUD's (`UI/SCREENS2/604153.BNDL`), the countdown and results
//! sequences' layouts, their textures and bitmap fonts, with the bindings the layouts name filled from the car
//! and the race (`mw_assets::ui::UiScene::draw_with` turns them into quads; `ui_quads` shows them as Bevy UI
//! nodes). The minimap is the game's map-tile grid, composited on the CPU with the car's facing (local +Z) up,
//! with the points-of-interest icons over it (player, gates, racers, cops, body shops and jack spots within
//! 400 m; docs/decomp.md "Minimap orientation", "The world map's items and filters").
//!
//! The game decides which layouts a widget shows in code (widget adapters); here fixed sets stand in:
//! - free roam (always): speedometer + nitrous gauge, odometer, minimap frame, road / district name;
//! - a race (`race::RaceState` present, intro and racing): position / lap or distance left, race time,
//!   wrong way / checkpoint missed;
//! - the countdown sequence over the intro's last seconds, and the results sequence after the finish.
//!
//! Triggered behaviours (fades, slides, flashes keyed to a binding's changes) are animated (`UiAnim`).
//! F3 toggles the debug status text.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

use bevy::prelude::*;
use mw_assets::ui::{DrawOptions, FREE_ROAM_HUD, MapMosaic, PlacedLayout, Quad, SequenceLayout, UiAnim, UiScene};
use mw_bundle::ResourceId;
use mw_formats::binding::{BindValue, Binding, race_time};

use crate::game_text::{self, GameFonts, LoadedFont};
use crate::race::{RacePhase, RaceState, Racer, RacerProgress};
use crate::ui_quads::{AdditiveQuadMaterial, PoolNode, QuadNode, QuadPool, show_quads};

/// Layouts shown while driving in free roam (UniqueIds in `FREE_ROAM_HUD`): the first `Nitrous` widget's
/// speedometer + nitrous gauge (1670359) and odometer (1806698), and `AdditionalHUD`'s minimap frame
/// (1206246) and road / district name plates (1990196), and `HUD`'s Speed Points total (979928: shown when the
/// total changes, `freeroam.rs`). Pursuit and EasyDrive layouts need game modes we don't have yet. 604307 is
/// `EngineOffHud`'s prompt bar (`{Mode.PromptString}`, e.g. the jack spots' "PRESS … TO DRIVE CAR"), drawn
/// while [`HudState::prompt`] is set.
pub const FREE_ROAM_LAYOUTS: [u32; 6] = [1_206_246, 1_990_196, 1_670_359, 1_806_698, 979_928, 604_307];

/// Pursuit layouts of the free-roam screen (`AdditionalHUD`): 1213038, the HEAT / COOLDOWN bars beside the
/// minimap (the stepped level bar bound to `Players.LocalPlayer.HeatLevelInt` over 0–6 and the fill bound to
/// `Widget.HUD.Heat`), shown in a chase or cooldown, and 581357, the BUSTING bar
/// (`Players.LocalPlayer.StaticBustProgress`), shown while `Players.LocalPlayer.ShowStaticBustBar` holds
/// (`police.rs`; docs/decomp.md "Police and pursuits"). MODEL: which layouts the widget shows when is decided
/// by the widget's code, not read; `Widget.HUD.Heat` is taken as the progress through the heat level
/// (`FUN_006bdc50`).
pub const PURSUIT_LAYOUT: u32 = 1_213_038;
pub const BUSTING_LAYOUT: u32 = 581_357;

/// The offline race HUD screen: the HSM's "Race HUD" state ("Activate Race HUD Styles Widget Group") names
/// 604152 / 371726; this bundle is the screen whose widgets bind `DistanceToFinishManager`, `Event.Data.Laps`
/// and `Players.LocalPlayer.EventPosition` (research log 2026-10-09).
pub const RACE_HUD: &str = "UI/SCREENS2/604153.BNDL";

/// Race layouts of `RACE_HUD`: `AdditionalHUD` 1470173 (POSITION, LAP x / y on circuits, DISTANCE on
/// sprints), 1470152 (TIME, `Mode.Time`) and `HUD` 1106106 (WRONG WAY / CHECKPOINT MISSED banners).
pub const RACE_LAYOUTS: [u32; 3] = [1_470_173, 1_470_152, 1_106_106];

/// Countdown sequence: the online race mode's (GameMode 1019777) `4d1a6bfa`, a `Sequence` bound to
/// `Mode.IntroTime` whose layouts are "GET READY..." and the "RACE / STARTING IN... / {Mode.IntroTime(INT)}"
/// card with GO. The offline race mode (730000) has 0 in that field and in every other sequence field, and
/// the offline flow's countdown was not located: using this one is a model decision.
pub const COUNTDOWN_SEQUENCE: u32 = 275_829;
/// The HSM starts the countdown when `Mode.IntroTime(LTEQ:3)` holds ("Wait For Countdown Start (Exit When
/// Binding Is True)" → "Play countdown (Run Gamelogic Sequence)"); the sequence runs on its own clock from
/// there. Its items after 5 s (the "FALSE START" card at 8–11 s, for the online false-start check) are left out.
const COUNTDOWN_START_INTRO_TIME: f32 = 3.0;
const COUNTDOWN_LAST_ITEM_START: f32 = 5.0;

/// Results sequence: the online race mode's `ResultsSequence` (1019828): the RESULTS leaderboard (1335088, element
/// stacks over `ActiveGameMode.RaceLeaderboard`) and the summary (1645120: "YOU FINISHED 1ST", "TIME ...") from
/// 0.25 s to 8.25 s. The offline race mode's `ResultsSequence` is 0 (its results are the front end's
/// `ArcResults` screen 1377068): model decision as for the countdown.
pub const RESULTS_SEQUENCE: u32 = 1_019_828;
/// `582390`'s string palettes: `FinishPosition` "YOU FINISHED {...FinishPosition(WITHORDINAL)}" and
/// `FinishTime` "TIME {...LocalPlayerTime(TIME)}", what `Widget.HUD.SummaryHeadline` / `SummaryDetail` show.
const FINISH_POSITION_TEXT: u32 = 1_589_986;
const FINISH_TIME_TEXT: u32 = 1_693_016;

/// The free-roam collectables' points of interest come and go with the distance to the player's car
/// (`FUN_006ffd90` from `FUN_00702xxx`): added within 400 m (`.rdata` 0x00fcb044), removed beyond 450 m (`.data`
/// 0x010c5944, its initial value).
const POI_ADD_RANGE: f32 = 400.0;
const POI_REMOVE_RANGE: f32 = 450.0;

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
        app.insert_resource(HudSettings {
            enabled: self.enabled,
            metric: self.metric,
            debug_text: false,
            hidden_by_sequence: false,
        })
        .init_resource::<HudState>()
        .add_systems(Startup, start_loading)
        .add_systems(Update, (receive_hud, toggle_debug, track_car, track_race, draw_hud).chain());
    }
}

#[derive(Resource)]
pub struct HudSettings {
    pub enabled: bool,
    /// km/h and km (the game's `Language.IsMetric`).
    pub metric: bool,
    /// F3: the debug status text instead of / with the HUD.
    pub debug_text: bool,
    /// A sequence hides the HUD (`HideHudSequenceItem`, the crash camera).
    pub hidden_by_sequence: bool,
}

/// Game state the bindings read that the car doesn't keep.
#[derive(Resource, Default)]
pub struct HudState {
    /// Distance driven (m).
    pub odometer: f64,
    last_position: Option<Vec3>,
    /// Nitrous tank fill 0–1 and whether it burns (the driven car's `mw_handling::nitrous::Nitrous`).
    pub nitrous: f32,
    pub using_nitrous: bool,
    /// Seconds the results sequence has played (the player finished and no sequence hides the HUD).
    pub results_clock: Option<f32>,
    /// Speed Points, the collectables' counts and the message sequences they started (`freeroam.rs`).
    pub freeroam: crate::freeroam::FreeroamHud,
    /// `Mode.PromptString` (game-mode manager +0x2c70 with its flag +0x2ca7): the prompt bar's text, set each
    /// frame by whoever prompts (`jackspots.rs`); `None` hides the bar.
    pub prompt: Option<String>,
}

/// What a layout of the HUD scene belongs to.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Group {
    FreeRoam,
    Race,
    /// The heat bars / the BUSTING bar (`PURSUIT_LAYOUT`, `BUSTING_LAYOUT`).
    Pursuit,
    Busting,
    /// Item of the countdown / results sequence (index into `Hud::countdown` / `Hud::results`).
    Countdown(usize),
    Results(usize),
    /// Item of a scoring message sequence (index into `Hud::feedback`, then into its items).
    Feedback(usize, usize),
    /// A point-of-interest icon drawn on the minimap (the race HUD's `Widget_PointOfInterest` looks).
    MapIcon(MapIcon),
}

/// The minimap icons drawn: the `PointOfInterestVisuals` map layouts named by `mw_assets::ui::PoiKeys`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum MapIcon {
    Player,
    Gate,
    GatePreview,
    Finish,
    FinishPreview,
    Opponent,
    /// A cop: the opponent look with `Players.PlayerBase[#].IsCop` true (its cop elements).
    Cop,
    /// `CollectablesConfig` +0x10: a body shop.
    BodyShop,
    /// `CollectablesConfig` +0x48: a jack spot location (icon shown once found).
    JackSpot,
}

impl MapIcon {
    /// The icon whose layout is drawn (a cop is the opponent look).
    fn look(self) -> Self {
        if self == Self::Cop { Self::Opponent } else { self }
    }
}

/// A sequence item's timing, enabling binding and texts.
#[derive(Debug, Clone)]
struct SeqItem {
    start: f32,
    end: f32,
    enabled: String,
    texts: [u32; 3],
}

impl SeqItem {
    fn of(s: &SequenceLayout) -> Self {
        Self { start: s.start, end: s.end, enabled: s.enabled.clone(), texts: s.texts }
    }
}

struct HudAssets {
    scene: UiScene,
    groups: Vec<Group>,
    countdown: Vec<SeqItem>,
    results: Vec<SeqItem>,
    feedback: Vec<(u32, Vec<SeqItem>)>,
    lang: mw_formats::Language,
    car_names: HashMap<String, String>,
    images: Vec<(ResourceId, Image)>,
    fonts: Vec<(u32, mw_formats::Font)>,
    mosaic: Option<MapMosaic>,
    points: Vec<MapPoint>,
}

/// A free-roam collectable with a minimap icon (`FUN_006ffba0`): body shops (kind 6) and jack spot locations
/// ('<', 'B', 'E', 'K', 'N').
#[derive(Debug, Clone, Copy)]
struct MapPoint {
    icon: MapIcon,
    /// The trigger's id.
    id: u32,
    pos: Vec3,
}

#[derive(Resource)]
struct HudLoading(Mutex<Receiver<HudAssets>>);

#[derive(Resource)]
struct Hud {
    scene: UiScene,
    groups: Vec<Group>,
    countdown: Vec<SeqItem>,
    results: Vec<SeqItem>,
    /// The collectables' message sequences by UniqueId (`freeroam::hud_sequences`): the "+SP" line, the speed
    /// camera's plate, the security gate's speedwall panel.
    feedback: Vec<(u32, Vec<SeqItem>)>,
    lang: mw_formats::Language,
    /// Car names by bundle stem (`VEH_1085007_HI`): AI racers are named after their car's bundle.
    car_names: HashMap<String, String>,
    anim: UiAnim,
    textures: HashMap<ResourceId, (Handle<Image>, Vec2)>,
    root: Entity,
    pool: QuadPool,
    map: Option<MapView>,
    points: Vec<MapPoint>,
    /// Indices into `points` with a point of interest now (within range, `FUN_006ffd90`).
    active: std::collections::BTreeSet<usize>,
}

struct MapView {
    mosaic: MapMosaic,
    image: Handle<Image>,
    size: UVec2,
    frame: u32,
}

/// The layouts of `screen` named in `ids`, each once (screens repeat layouts across widget variants: the five
/// `Nitrous` widgets all place the odometer).
fn screen_layouts(store: &mw_assets::AssetStore, screen: &str, ids: &[u32]) -> Vec<PlacedLayout> {
    let mut out: Vec<PlacedLayout> = Vec::new();
    match store.ui_screen_layouts(screen, None) {
        Ok(all) => {
            for l in all {
                if ids.contains(&l.layout.id) && !out.iter().any(|o| o.layout.id == l.layout.id) {
                    out.push(l);
                }
            }
        }
        Err(e) => warn!("HUD: no screen {screen} ({e})"),
    }
    out
}

/// Build the HUD scene on a thread: decode the screens' layouts, the sequences', styles, fonts and textures.
fn start_loading(mut commands: Commands, game: Res<crate::Game>, settings: Res<HudSettings>) {
    if !settings.enabled {
        return;
    }
    let store = game.store.clone();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let t = std::time::Instant::now();
        let lang = match store.language(mw_assets::DEFAULT_LANGUAGE) {
            Ok(l) => l,
            Err(e) => {
                warn!("HUD: no language table ({e})");
                return;
            }
        };
        let mut layouts = Vec::new();
        let mut groups = Vec::new();
        for l in screen_layouts(&store, FREE_ROAM_HUD, &FREE_ROAM_LAYOUTS) {
            layouts.push(l);
            groups.push(Group::FreeRoam);
        }
        for l in screen_layouts(&store, FREE_ROAM_HUD, &[PURSUIT_LAYOUT, BUSTING_LAYOUT]) {
            groups.push(if l.layout.id == PURSUIT_LAYOUT { Group::Pursuit } else { Group::Busting });
            layouts.push(l);
        }
        for l in screen_layouts(&store, RACE_HUD, &RACE_LAYOUTS) {
            layouts.push(l);
            groups.push(Group::Race);
        }
        // Minimap icons: the map layout of each point-of-interest look the race code names. Their instance only
        // translates to the `Pos` binding (the point on the map), which `draw_hud` does.
        match (store.poi_keys(), store.ui_poi_visuals(RACE_HUD)) {
            (Ok(keys), Ok(pois)) => {
                let wanted = [
                    (MapIcon::Player, keys.player),
                    (MapIcon::Gate, keys.gate),
                    (MapIcon::GatePreview, keys.gate_preview),
                    (MapIcon::Finish, keys.finish),
                    (MapIcon::FinishPreview, keys.finish_preview),
                    (MapIcon::Opponent, keys.opponent),
                    (MapIcon::BodyShop, keys.body_shop),
                    (MapIcon::JackSpot, keys.jack_spot),
                ];
                for (icon, key) in wanted {
                    match pois.iter().find(|p| p.key == key).and_then(|p| p.map.clone()) {
                        Some(mut l) => {
                            l.instance = None;
                            layouts.push(l);
                            groups.push(Group::MapIcon(icon));
                        }
                        None => warn!("HUD: no map icon for {icon:?} (point of interest {key})"),
                    }
                }
            }
            (k, p) => warn!("HUD: no point-of-interest looks ({:?} / {:?})", k.err(), p.err()),
        }
        let sequence = |id: u32| {
            store.ui_sequence_layouts(id).unwrap_or_else(|e| {
                warn!("HUD: no sequence {id} ({e})");
                Vec::new()
            })
        };
        let mut countdown = Vec::new();
        for s in sequence(COUNTDOWN_SEQUENCE).into_iter().filter(|s| s.start < COUNTDOWN_LAST_ITEM_START) {
            groups.push(Group::Countdown(countdown.len()));
            countdown.push(SeqItem::of(&s));
            layouts.push(s.layout);
        }
        let mut results = Vec::new();
        for s in sequence(RESULTS_SEQUENCE) {
            groups.push(Group::Results(results.len()));
            results.push(SeqItem::of(&s));
            layouts.push(s.layout);
        }
        let mut feedback = Vec::new();
        for id in store.scoring().map(|s| crate::freeroam::hud_sequences(&s)).unwrap_or_default() {
            let mut items = Vec::new();
            for s in sequence(id) {
                groups.push(Group::Feedback(feedback.len(), items.len()));
                items.push(SeqItem::of(&s));
                layouts.push(s.layout);
            }
            feedback.push((id, items));
        }
        let scene = store.ui_scene(layouts, &lang);
        let car_names = store
            .vehicles(&lang)
            .unwrap_or_default()
            .into_iter()
            .filter(|v| v.name != "?")
            .flat_map(|v| {
                let name = v.name.to_uppercase();
                [(format!("VEH_{}_HI", v.id), name.clone()), (format!("VEH_{}_LO", v.id), name)]
            })
            .collect();
        let images =
            scene.textures().into_iter().filter_map(|id| Some((id, game_text::decode_texture(&store, id)?))).collect();
        let fonts = scene.fonts.iter().map(|(&n, f)| (n, f.clone())).collect();
        let mosaic = store.ui_map_grid().ok().map(|g| MapMosaic::build(&store, g));
        let points = store
            .trigger_data()
            .map(|td| {
                td.triggers
                    .iter()
                    .filter_map(|t| {
                        let icon = match t.kind {
                            6 => MapIcon::BodyShop,
                            b'<' | b'B' | b'E' | b'K' | b'N' => MapIcon::JackSpot,
                            _ => return None,
                        };
                        Some(MapPoint { icon, id: t.id, pos: Vec3::from(t.position) })
                    })
                    .collect()
            })
            .unwrap_or_default();
        info!(
            "HUD: {} layouts ({} countdown, {} results items), {} fonts, minimap {} in {:.0?}",
            scene.layouts.len(),
            countdown.len(),
            results.len(),
            scene.fonts.len(),
            mosaic.is_some(),
            t.elapsed()
        );
        let _ = tx.send(HudAssets {
            scene,
            groups,
            countdown,
            results,
            feedback,
            lang,
            car_names,
            images,
            fonts,
            mosaic,
            points,
        });
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
    commands.insert_resource(Hud {
        scene: assets.scene,
        groups: assets.groups,
        countdown: assets.countdown,
        results: assets.results,
        feedback: assets.feedback,
        lang: assets.lang,
        car_names: assets.car_names,
        anim: UiAnim::default(),
        textures,
        root,
        pool: QuadPool::default(),
        map,
        points: assets.points,
        active: Default::default(),
    });
}

/// F3 shows the debug status text (hidden while the HUD is up).
fn toggle_debug(
    keys: Res<ButtonInput<KeyCode>>,
    mut settings: ResMut<HudSettings>,
    hud: Option<Res<Hud>>,
    car: Query<(), (With<crate::drive::Car>, With<crate::drive::Player>)>,
    mut status: Query<&mut Visibility, With<crate::StatusText>>,
) {
    if keys.just_pressed(KeyCode::F3) {
        settings.debug_text = !settings.debug_text;
    }
    let hud_up = settings.enabled && hud.is_some() && !car.is_empty();
    let show = (settings.debug_text || !hud_up) && !settings.hidden_by_sequence;
    for mut v in &mut status {
        let want = if show { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    }
}

fn track_car(
    mut state: ResMut<HudState>,
    car: Query<(&GlobalTransform, &crate::drive::Car), With<crate::drive::Player>>,
) {
    let Ok((t, car)) = car.single() else {
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
    state.nitrous = car.nitrous.fill();
    state.using_nitrous = car.nitrous.burning;
}

/// The results sequence's clock: runs once the player has finished and nothing hides the HUD (the event
/// outro's `HideHudSequenceItem` comes first in the original).
fn track_race(mut state: ResMut<HudState>, race: Option<Res<RaceState>>, settings: Res<HudSettings>, time: Res<Time>) {
    let finished = race.is_some_and(|r| r.phase == RacePhase::Finished);
    state.results_clock = match (finished, state.results_clock) {
        (false, _) => None,
        (true, None) if settings.hidden_by_sequence => None,
        (true, None) => {
            info!("HUD: results sequence starts at {:.2} s game time", time.elapsed_secs());
            Some(0.0)
        }
        (true, Some(t)) => Some(t + time.delta_secs()),
    };
}

/// One racer's row of `ActiveGameMode.RaceLeaderboard`.
struct BoardRow {
    name: String,
    finished: Option<f32>,
    local: bool,
}

/// The race as the bindings see it this frame.
struct RaceView<'a> {
    race: &'a RaceState,
    player: Option<&'a RacerProgress>,
    board: Vec<BoardRow>,
    mode_name: String,
}

/// `Mode.IntroTime` (`FUN_006e2670`): (start − now + 1 s) in whole seconds; after go it is 0 and below.
fn intro_time(race: &RaceState) -> f32 {
    match race.phase {
        RacePhase::Waiting => race.intro + 1.0,
        RacePhase::Intro => (race.intro_left() + 1.0).trunc(),
        _ => (1.0 - race.clock).trunc(),
    }
}

/// The countdown sequence's clock: seconds since `Mode.IntroTime(LTEQ:3)` became true.
fn countdown_clock(race: &RaceState) -> Option<f32> {
    match race.phase {
        RacePhase::Intro if race.intro_left() < COUNTDOWN_START_INTRO_TIME => {
            Some(COUNTDOWN_START_INTRO_TIME - race.intro_left())
        }
        RacePhase::Racing | RacePhase::Finished => Some(COUNTDOWN_START_INTRO_TIME + race.clock),
        _ => None,
    }
}

/// `ActiveGameMode.RaceLeaderboard[i].Field`: as a whole path (cell fields) or split by the binding parser at
/// the `[` (template bindings: path `ActiveGameMode.RaceLeaderboard`, argument `i].Field`).
fn board_field<'p>(path: &'p str, arg: Option<&'p str>) -> Option<(usize, &'p str)> {
    const LIST: &str = "ActiveGameMode.RaceLeaderboard";
    let rest = match path.strip_prefix(LIST)? {
        "" => arg?,
        r => r.strip_prefix('[')?,
    };
    let (index, field) = rest.split_once(']')?;
    Some((index.trim().parse().ok()?, field.trim_start_matches('.')))
}

/// Fill `{binding}`s of a language string with `lookup` (the summary's palette strings, sequence texts).
fn resolve(template: &str, metric: bool, lookup: &mut dyn FnMut(&str, Option<&str>) -> Option<BindValue>) -> String {
    mw_formats::ui::substitute(template, |expr| {
        let b = Binding::parse(expr)?;
        let v = b.eval(lookup)?;
        Some(b.format_in(&v, Some(metric)))
    })
}

impl RaceView<'_> {
    /// Values for the race layouts' bindings (`None`: not a race binding).
    fn lookup(&self, path: &str, arg: Option<&str>, metric: bool) -> Option<BindValue> {
        let race = self.race;
        if let Some((i, field)) = board_field(path, arg) {
            let row = self.board.get(i)?;
            return Some(match field {
                "Name" => row.name.clone().into(),
                // DNF shows for 0.
                "FinalTime" => row.finished.map_or(BindValue::Num(0.0), |t| race_time(t as f64).into()),
                "LocalPlayer" => row.local.into(),
                // Not modelled (speed points, takedowns, wrecks): empty labels.
                "Score" | "Score1" | "Score2" => String::new().into(),
                _ => false.into(),
            });
        }
        Some(match path {
            // 0 until the race has placed the racers: nothing shown.
            "Players.LocalPlayer.EventPosition" => (Some(self.player?.position).filter(|&p| p > 0)? as f64).into(),
            "Players.LocalPlayer.Lap" => (self.player?.lap.clamp(1, race.laps) as f64).into(),
            "Event.Data.Laps" => (race.laps as f64).into(),
            "Players.LocalPlayer.DistanceLeft" => (self.player?.to_finish.max(0.0) as f64).into(),
            "Mode.Time" => race_time(race.clock as f64).into(),
            "DistanceToFinishManager.IsGoingWrongWay" => {
                (race.phase == RacePhase::Racing && self.player.is_some_and(|p| p.wrong_way)).into()
            }
            "DistanceToFinishManager.HasMissedCheckpoint" => {
                (race.phase == RacePhase::Racing && self.player.is_some_and(|p| p.missed_checkpoint)).into()
            }
            "Mode.IntroTime" => (intro_time(race) as f64).into(),
            "Mode.IsRaceEvent" => true.into(),
            "Mode.DisplayTimeout" | "Mode.IsFreedriveEvent" | "Mode.Data.Online" => false.into(),
            "Net.NumPlayersInGame" => (self.board.len() as f64).into(),
            // Row numbers of the leaderboard (`~Signals.GetIntValue[#]`, `#` = the 0-based row).
            "Signals.GetIntValue" => (arg?.trim().parse::<f64>().ok()? + 1.0).into(),
            "Event.GameMode.Name" | "Widget.HUD.GameMode.Name" => self.mode_name.clone().into(),
            "Widget.FinishPosition.FinishPosition" => (self.player?.position as f64).into(),
            "Widget.FinishTime.LocalPlayerTime" => (self.player?.finished? as f64).into(),
            // Speed points are not modelled.
            "Widget.HUD.LocalPlayerScore" => 0.0.into(),
            _ => {
                let _ = metric;
                return None;
            }
        })
    }
}

/// The race's gate icons for the player's target route entry `target`, as `FUN_00759860` registers the
/// points of interest: the target gate (the finish look when it is the last entry) and, while the target is
/// not the last entry, the gate after it (the finish's preview look when that one is last). Each at the gate's
/// ground point, facing its normal.
fn gate_icons(race: &RaceState, target: usize) -> Vec<(MapIcon, Vec3, Vec3)> {
    use crate::gate_marker::{GateLook, shown_gates};
    shown_gates(race, target)
        .into_iter()
        .map(|(look, gate)| {
            let icon = match look {
                GateLook::Gate => MapIcon::Gate,
                GateLook::GatePreview => MapIcon::GatePreview,
                GateLook::Finish => MapIcon::Finish,
                GateLook::FinishPreview => MapIcon::FinishPreview,
            };
            (icon, gate.ground(), gate.normal())
        })
        .collect()
}

/// Draw the minimap's icons: each icon layout centred on its point's place on the map (`Pos`), turned by its
/// heading relative to the map (`Rot`, degrees clockwise; the player's arrow points up). The map is centred on
/// the player, `facing` up, `metres_per_px` per logical pixel. Model decisions: icons are drawn at their
/// layouts' reference size × the HUD scale, and icons whose centre is off the map are left out (the POI
/// widget's clamping, `EnableClamping` / `IsClamped`, is code that was not traced).
#[allow(clippy::too_many_arguments)]
fn map_icons(
    scene: &UiScene,
    groups: &[Group],
    anim: &mut UiAnim,
    t: f64,
    screen: [f32; 2],
    map: &Quad,
    centre: Vec3,
    facing: Vec3,
    metres_per_px: f32,
    icons: &[(MapIcon, Vec3, Vec3)],
    base: &mut dyn FnMut(&str, Option<&str>) -> Option<BindValue>,
) -> Vec<Quad> {
    let up = Vec2::new(facing.x, facing.z).normalize_or(Vec2::Y);
    // Screen right of the heading-up map (as `MapMosaic::render`).
    let right = Vec2::new(-up.y, up.x) * mw_assets::ui::MAP_RIGHT_SIGN;
    let map_centre = Vec2::new(map.rect[0] + map.rect[2] * 0.5, map.rect[1] + map.rect[3] * 0.5);
    // Where a layout's origin lands when drawn without an instance (`Placement`: the reference frame centred).
    let s = (screen[0] / 1280.0).min(screen[1] / 720.0);
    let origin = Vec2::new(screen[0] * 0.5 - 640.0 * s, screen[1] * 0.5 - 360.0 * s);
    let mut out = Vec::new();
    for &(icon, pos, dir) in icons {
        let d = Vec2::new(pos.x - centre.x, pos.z - centre.z);
        let at = map_centre + Vec2::new(d.dot(right), -d.dot(up)) / metres_per_px;
        if at.x < map.rect[0]
            || at.y < map.rect[1]
            || at.x > map.rect[0] + map.rect[2]
            || at.y > map.rect[1] + map.rect[3]
        {
            continue;
        }
        let g = Vec2::new(dir.x, dir.z);
        let rot = g.dot(right).atan2(g.dot(up)).to_degrees();
        let mut lookup = |path: &str, arg: Option<&str>| -> Option<BindValue> {
            match path {
                "Rot" => Some((rot as f64).into()),
                "Net.IsInOnlineGame" => Some(false.into()),
                // `Players.PlayerBase[#].IsCop` / `.InChase`: the look's cop elements for cops.
                "Players.PlayerBase" => Some((icon == MapIcon::Cop).into()),
                // Only found jack spots are listed (`icons`).
                "JackSpots.IsSwapSpotFound" => Some((icon == MapIcon::JackSpot).into()),
                _ => base(path, arg),
            }
        };
        let look = icon.look();
        let mut options = DrawOptions::new(
            Some(Box::new(move |i| groups.get(i).is_some_and(|&g| g == Group::MapIcon(look)))),
            Some(&mut *anim),
            t,
        );
        let offset = at - origin;
        for mut q in scene.draw_with(screen, &mut lookup, &mut options) {
            q.rect[0] += offset.x;
            q.rect[1] += offset.y;
            out.push(q);
        }
    }
    out
}

/// The car's facing on the ground plane: its local +Z (the game's vehicle matrix row 2, which
/// `FUN_0083c030` returns for the binding below). Not Bevy's `forward()`, which is local −Z.
fn car_facing(xf: &GlobalTransform) -> Vec3 {
    xf.back().as_vec3()
}

/// `Players.LocalPlayer.GetPlayerDirection` ("2d Direction of the car in degrees", `FUN_006db960` →
/// `FUN_006be9b0`): −atan2(f.x, f.z) in degrees of the facing f.
fn player_direction(xf: &GlobalTransform) -> f32 {
    let f = car_facing(xf);
    -f.x.atan2(f.z).to_degrees()
}

/// Values for the bindings the free-roam layouts use.
fn lookup(
    path: &str,
    arg: Option<&str>,
    car: Option<(&crate::drive::Car, &GlobalTransform)>,
    state: &HudState,
    pursuit: &crate::police::PursuitHud,
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
        "Players.LocalPlayer.NitrousState.UsingNitrous" => state.using_nitrous.into(),
        "Players.LocalPlayer.NitrousState.InOverdrive" => false.into(),
        "Widget.Nitrous.ShowOdometer" => true.into(),
        // Metres: `DIST,LONG` prints km or miles.
        "Widget.Nitrous.Odometer" => state.odometer.into(),
        "Signals.GetSineWave" => {
            let period = arg.and_then(|a| a.parse::<f64>().ok()).unwrap_or(1000.0) / 1000.0;
            (0.5 + 0.5 * (t / period.max(0.001) * std::f64::consts::TAU).sin()).into()
        }
        "Players.LocalPlayer.GetPlayerDirection" => (player_direction(xf?) as f64).into(),
        // Speed Points and collectables (`freeroam.rs`). The total's label (layout 979928) shows while the
        // widget `SPVisible` is visible: 8 s at the start and after each change (HSM "SP Visibility", 1695370).
        // Its backing is triggered by `MostWantedListManager.LocalPlayerSpeedPoints` (steps: white, invisible
        // after 8 s).
        // Model decisions: that trigger is read as "the total just changed" (true for 0.1 s after a change; the
        // game's trigger evaluation was not traced), and the speedwalls have no rows (they list Autolog
        // friends).
        "ScoringViewModel.CurrentSpeedPoints" => (state.freeroam.speed_points as f64).into(),
        "MostWantedListManager.LocalPlayerSpeedPoints" => state.freeroam.sp_changed.is_some_and(|c| t - c < 0.1).into(),
        "SPVisible.IsVisible" => state.freeroam.sp_visible_until.is_some_and(|until| t < until).into(),
        "LocalPlayer.IsInChaseOrCooldown" | "Mode.Data.Online" => false.into(),
        // `(SPEED, DECIMAL)`: mph, or km/h with metric units (model decision, as the other code-side formatters).
        "Collectables.LastCameraSpeed" => {
            (state.freeroam.last_camera_mph as f64 * if metric { 1.609_34 } else { 1.0 }).into()
        }
        "Collectables.SpeedwallColFound" => (state.freeroam.wall_found as f64).into(),
        "Collectables.SpeedwallColAmt" => (state.freeroam.wall_total as f64).into(),
        "Collectables.SpeedCamsFoundFromSpeedwall" => (state.freeroam.cameras_found as f64).into(),
        "Collectables.SpeedCamAmountFromSpeedwall" => (state.freeroam.cameras_total as f64).into(),
        "CollectablesVM.CameraSpeedwall.ShowMiniWall" | "Camera.IsInternalDrivingCam" => false.into(),
        "CollectablesVM.Collectables.speedwall.Items._length" => 0.0.into(),
        // An empty text is false: the bar's elements are visible on this binding.
        "Mode.PromptString" => state.prompt.clone().unwrap_or_default().into(),
        "Players.LocalPlayer.IsInChase" => pursuit.in_chase.into(),
        "Players.LocalPlayer.IsInCooldown" => pursuit.in_cooldown.into(),
        "Players.LocalPlayer.IsInChaseOrCooldown" => (pursuit.in_chase || pursuit.in_cooldown).into(),
        "Players.LocalPlayer.HeatLevelInt" => (pursuit.heat_level as f64).into(),
        "Widget.HUD.Heat" => (pursuit.heat_progress as f64).into(),
        "Players.LocalPlayer.StaticBustProgress" => (pursuit.bust_progress as f64).into(),
        "Players.LocalPlayer.Jammed" | "Players.LocalPlayer.IsInTunnel" => false.into(),
        _ => return None,
    })
}

#[allow(clippy::too_many_arguments)]
fn draw_hud(
    mut commands: Commands,
    hud: Option<ResMut<Hud>>,
    settings: Res<HudSettings>,
    state: Res<HudState>,
    car: Query<(&crate::drive::Car, &GlobalTransform), With<crate::drive::Player>>,
    race: Option<Res<RaceState>>,
    racers: Query<(&Racer, &RacerProgress)>,
    racer_xf: Query<(&Racer, &GlobalTransform)>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    time: Res<Time>,
    mut nodes: Query<PoolNode, With<QuadNode>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<AdditiveQuadMaterial>>,
    mut root_vis: Query<&mut Visibility, Without<QuadNode>>,
    pause: Option<Res<crate::pause::PauseMenu>>,
    (pursuit, collected): (Res<crate::police::PursuitHud>, Option<Res<crate::frontend::Collected>>),
) {
    let Some(mut hud) = hud else { return };
    let car = car.single().ok();
    // The pause menu covers the game (its own screen); the HUD goes while it is open.
    let paused = pause.is_some_and(|p| p.open);
    let visible = settings.enabled && !settings.hidden_by_sequence && car.is_some() && !paused;
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

    let Hud { scene, groups, countdown, results, feedback, lang, car_names, anim, map, points, active, .. } = &mut *hud;
    scene.metric = Some(metric);
    let view = race.as_deref().map(|race| {
        let player = racers.iter().find(|(r, _)| r.player).map(|(_, p)| p);
        // Finished racers by finish time, then the rest by position.
        let mut rows: Vec<_> = racers.iter().collect();
        rows.sort_by(|a, b| match (a.1.finished, b.1.finished) {
            (Some(x), Some(y)) => x.total_cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.1.position.cmp(&b.1.position),
        });
        let board = rows
            .into_iter()
            .map(|(r, p)| BoardRow {
                // Model decision: the car's name (the original's racer names were not located).
                name: car_names.get(&r.name).cloned().unwrap_or_else(|| r.name.to_uppercase()),
                finished: p.finished,
                local: r.player,
            })
            .collect();
        let mode_name = race.game_mode.and_then(|m| lang.get(m.name)).unwrap_or("RACE").to_uppercase();
        RaceView { race, player, board, mode_name }
    });
    let race_hud = view.as_ref().is_some_and(|v| matches!(v.race.phase, RacePhase::Intro | RacePhase::Racing));

    let mut base = |path: &str, arg: Option<&str>| -> Option<BindValue> {
        view.as_ref()
            .and_then(|v| v.lookup(path, arg, metric))
            .or_else(|| lookup(path, arg, car, &state, &pursuit, metric, t))
    };
    let mut draw = |only: &dyn Fn(Group) -> bool, lookup: &mut dyn FnMut(&str, Option<&str>) -> Option<BindValue>| {
        let groups = &*groups;
        let mut options =
            DrawOptions::new(Some(Box::new(move |i| groups.get(i).is_some_and(|&g| only(g)))), Some(&mut *anim), t);
        scene.draw_with(screen, lookup, &mut options)
    };
    let mut quads: Vec<Quad> = draw(
        &|g| {
            g == Group::FreeRoam
                || (g == Group::Race && race_hud)
                || (g == Group::Pursuit && (pursuit.in_chase || pursuit.in_cooldown))
                || (g == Group::Busting && pursuit.show_bust_bar)
        },
        &mut base,
    );

    // Sequence items: their texts come in as `UISequences.Params[#].TextN` (the parser splits the path at
    // the `[`: argument `#].TextN`).
    let mut sequence = |items: &[SeqItem], clock: f32, group: fn(usize) -> Group, quads: &mut Vec<Quad>| {
        for (i, item) in items.iter().enumerate() {
            if clock < item.start || clock >= item.end {
                continue;
            }
            if !item.enabled.is_empty()
                && !Binding::parse(&item.enabled).and_then(|b| b.eval(&mut base)).is_some_and(|v| v.truthy())
            {
                continue;
            }
            let texts = item.texts.map(|id| lang.get(id).map(|s| resolve(s, metric, &mut base)).unwrap_or_default());
            let mut seq_lookup = |path: &str, arg: Option<&str>| -> Option<BindValue> {
                if path == "UISequences.Params" {
                    let n = arg?.rsplit("Text").next()?.trim().parse::<usize>().ok()?;
                    return Some(texts.get(n.checked_sub(1)?)?.clone().into());
                }
                match path {
                    "Widget.HUD.SummaryHeadline" => {
                        Some(resolve(lang.get(FINISH_POSITION_TEXT)?, metric, &mut base).into())
                    }
                    "Widget.HUD.SummaryDetail" => Some(resolve(lang.get(FINISH_TIME_TEXT)?, metric, &mut base).into()),
                    _ => base(path, arg),
                }
            };
            let g = group(i);
            quads.extend(draw(&|x| x == g, &mut seq_lookup));
        }
    };
    if let Some(clock) = view.as_ref().and_then(|v| countdown_clock(v.race)) {
        sequence(countdown, clock, Group::Countdown, &mut quads);
    }
    if let Some(clock) = state.results_clock {
        sequence(results, clock, Group::Results, &mut quads);
    }
    // The collectables' messages: each playing sequence's items in their time range; a scoring action's title
    // replaces the item's `Text1`.
    for playing in &state.freeroam.playing {
        let Some(si) = feedback.iter().position(|(id, _)| *id == playing.sequence) else { continue };
        for (i, item) in feedback[si].1.iter().enumerate() {
            if playing.clock < item.start || playing.clock >= item.end {
                continue;
            }
            if !item.enabled.is_empty()
                && !Binding::parse(&item.enabled).and_then(|b| b.eval(&mut base)).is_some_and(|v| v.truthy())
            {
                continue;
            }
            let mut texts =
                item.texts.map(|id| lang.get(id).map(|s| resolve(s, metric, &mut base)).unwrap_or_default());
            if let Some(t1) = &playing.text1 {
                texts[0] = t1.clone();
            }
            let mut seq_lookup = |path: &str, arg: Option<&str>| -> Option<BindValue> {
                if path == "UISequences.Params" {
                    let n = arg?.rsplit("Text").next()?.trim().parse::<usize>().ok()?;
                    return Some(texts.get(n.checked_sub(1)?)?.clone().into());
                }
                base(path, arg)
            };
            quads.extend(draw(&|x| x == Group::Feedback(si, i), &mut seq_lookup));
        }
    }

    // The minimap: re-render the heading-up view into its image (every other frame). The map camera looks at
    // the player with `LookAtRotation` = `GetPlayerDirection` (the POI widget's `UIMapCameraConfig`): the car's
    // facing (local +Z) is up.
    let mut map_image = None;
    let map_quad = quads.iter().position(|q| q.map);
    if let (Some(qi), Some(map), Some((_, xf))) = (map_quad, map.as_mut(), car) {
        let q = quads[qi].clone();
        let scale = window.scale_factor();
        let size = UVec2::new((q.rect[2] * scale).round().max(1.0) as u32, (q.rect[3] * scale).round().max(1.0) as u32);
        let p = xf.translation();
        let f = car_facing(xf);
        // Metres per logical pixel: the map's reference scale over the HUD's scale.
        let hud_scale = (screen[0] / 1280.0).min(screen[1] / 720.0);
        let metres_per_px = MINIMAP_METRES_PER_PIXEL / hud_scale;
        map.frame += 1;
        if size != map.size || map.frame % 2 == 0 {
            let mut px = vec![0u8; (size.x * size.y * 4) as usize];
            map.mosaic.render(p.x, p.z, [f.x, f.z], metres_per_px / scale, size.x, size.y, &mut px);
            map.size = size;
            if let Some(mut img) = images.get_mut(&map.image) {
                *img = game_text::ui_image(size.x, size.y, px);
            }
        }
        map_image = Some(map.image.clone());

        // Icons over the map: the player, the race's target and next gates, the other racers.
        let mut icons = vec![(MapIcon::Player, p, f)];
        if let Some(v) = view.as_ref().filter(|v| matches!(v.race.phase, RacePhase::Intro | RacePhase::Racing)) {
            icons.extend(gate_icons(v.race, v.player.map_or(1, |p| p.target())));
        }
        icons.extend(
            racer_xf
                .iter()
                .filter(|(r, _)| !r.player)
                .map(|(_, x)| (MapIcon::Opponent, x.translation(), car_facing(x))),
        );
        // Cops: the same look (`MapPoiConfig.5bfb3553`), whose cop elements show on `IsCop`.
        icons.extend(pursuit.cops.iter().map(|&(p, f)| (MapIcon::Cop, p, f)));
        // Body shops and jack spots near the car (`FUN_006ffd90`): a point of interest within 400 m, removed
        // beyond 450 m. A jack spot's icon is visible once found (`JackSpots.IsSwapSpotFound[#]`). Model
        // decisions: every jack spot counts as one whose car is not yet in the profile's list (`FUN_006ed400`,
        // `FUN_006fcf90`), and the packs' spots use the base look.
        for (i, pt) in points.iter().enumerate() {
            let d2 = pt.pos.distance_squared(p);
            if d2 < POI_ADD_RANGE * POI_ADD_RANGE {
                active.insert(i);
            } else if d2 > POI_REMOVE_RANGE * POI_REMOVE_RANGE {
                active.remove(&i);
            }
        }
        let found = |id: u32| collected.as_ref().is_some_and(|c| c.jack_spots.contains(&id));
        icons.extend(
            active
                .iter()
                .map(|&i| points[i])
                .filter(|pt| pt.icon != MapIcon::JackSpot || found(pt.id))
                .map(|pt| (pt.icon, pt.pos, Vec3::Z)),
        );
        let icon_quads = map_icons(scene, groups, anim, t, screen, &q, p, f, metres_per_px, &icons, &mut base);
        quads.splice(qi + 1..qi + 1, icon_quads);
    }

    let Hud { root, pool, textures, .. } = &mut *hud;
    show_quads(&mut commands, *root, pool, &quads, textures, map_image, &mut nodes, &mut materials);
}
