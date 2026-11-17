//! Jack spots: the cars parked around the city that the player can swap into (docs/decomp.md "Jack spots:
//! the parked cars and the swap"). The game's `HSMBehaviourCarSwapLogic` ("Car Swap Logic" under the HSM's
//! "Freedrive States", script object `JackSpots`), traced from the decompiled code:
//!
//! - **Data** (`FUN_00747aa0`): one slot per `CarSwapSpot` (GAMEPLAY.BNDL, 183): `Placement` = the location
//!   box trigger, `VehicleId`, `ColourId`, `0e3bcabe` = the parked car's trigger (its position, its local Z
//!   the car's direction) and `e36e9d9d` = a second parking place.
//! - **Which cars stand** (`FUN_007615f0`, `FUN_007610c0`, `FUN_0075d290`): each frame the location triggers
//!   within 300 m of the player's car (`.rdata` 0x00fd6800) are sorted by distance and the nearest five keep
//!   their squared distance; every other slot gets FLT_MAX. A slot's car is requested while 400 ≤ d² ≤ 90000
//!   (`.data` 0x010c5cac / 0x010c5ca4: 20 m to 300 m, so it never appears next to the player), stays while
//!   d² ≤ 90000 once it stands, and is removed otherwise.
//! - **Prompt and swap** (`FUN_007615f0`): with the player's car in the slot's location trigger
//!   (`FUN_00742210`), its car standing, |speed| ≤ `GameLogicConfig.ff4aef6f` (70 mph) and no pursuit,
//!   `Mode.PromptString` is language string 955092 ("PRESS … TO DRIVE CAR", `GameLogicConfig.a1e83efa`); the
//!   action is SWAP_CARS (input state +0xc4 = action 14: E / pad Y) in state 2.
//! - **What the swap does** (`FUN_00761d70`, `FUN_00758f00`): the player's vehicle becomes the slot's car in
//!   the slot's colour and takes the parked car's pose at rest; the parked vehicle becomes the car the player
//!   came in (its model and colour go into the slot) and is put on the second parking place, the next time on
//!   the car trigger, alternating (without a second place: where the player's car was). Both triggers of the
//!   slot are marked found (`FUN_007059b0`), the slot is remembered as the last swap spot (player +0x784).
//!   No new swap until the player has left every prompt (+0x74c9). The slot goes back to its data once the
//!   player is more than 300 m from it (`FUN_0075d800`, `.rdata` 0x00fd67e0 = 90000), and all slots do when
//!   the driven car changes any other way (`FUN_00761d70`: `FUN_007592a0`, `FUN_00747aa0`).
//! - **Found** (`FUN_007059b0`, `FUN_006e44f0`): a location counts as found when the player's car enters its
//!   box while the slot's car stands (`JackSpots.IsSwapSpotVehicleSpawned`), in free roam.
//!
//! Model decisions (not from the game):
//! - "In the location trigger" is the car's origin inside the box (the game overlaps the car's box, its
//!   narrow phase was not read; the same decision as `freeroam.rs`).
//! - Every spot's car counts as available: the game hides spots by game pack (`FUN_00747aa0`) and offers the
//!   store instead of the swap for cars the profile has not unlocked (`FUN_0068f190`, prompt 2094166); neither
//!   the entitlements nor the Most Wanted unlocks exist here.
//! - The swap is immediate: the game first plays the target's manufacturer movie and a transition
//!   (`SwapTargetManufacturerMovie`, the config's sequences 995872 / 1016424) with the controls off.
//! - Parked cars are static: the body and wheel models with one box collider from the body meshes' bounds (the
//!   game's are physical cars that can be pushed), standing on the ground found below their trigger.
//! - The pursuit gate and the player flag +0x23f are not modelled (no police here); a crashed car cannot swap.
//! - The key shown in the prompt is the keyboard's (E), or Y with a pad connected.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use avian3d::prelude::*;
use bevy::camera::primitives::Aabb;
use bevy::prelude::*;
use mw_assets::AssetStore;
use mw_assets::frontend::SwapSpot;
use mw_assets::paint::VehiclePaint;
use mw_bundle::ResourceId;
use mw_formats::Trigger;

use crate::drive::{Car, CarInput, PendingCar, Player};
use crate::frontend::Collected;
use crate::garage::{Garage, GarageAction};
use crate::loader::{CarRequest, LoaderRequests, TrafficCarSpec, TrafficModelRequest, WheelSpec};
use crate::materials::GameMaterial;
use crate::props::GameLayer;
use crate::{Game, ModelParts};

/// Squared distances of the parked cars' rule (`FUN_0075d290`): request from 20 m (`.data` 0x010c5cac = 400)
/// to 300 m (`.data` 0x010c5ca4 = 90000); the same 90000 resets the last swapped slot (`.rdata` 0x00fd67e0).
const SPAWN_MIN_D2: f32 = 400.0;
const SPAWN_MAX_D2: f32 = 90_000.0;
/// Radius of the location-trigger query around the player's car (`.rdata` 0x00fd6800, `FUN_007610c0`) and how
/// many of the nearest keep their distance (`FUN_007615f0`).
const QUERY_RADIUS: f32 = 300.0;
const NEAREST: usize = 5;
/// `GameLogicConfig.ff4aef6f` (+0x284) when the data cannot be read.
const DEFAULT_MAX_MPH: f32 = 70.0;
/// `GameLogicConfig.a1e83efa` (+0x16c): the prompt's language string ("PRESS {…SWAP_CARS…} TO DRIVE CAR").
const PROMPT_TEXT: u32 = 955_092;
/// "JACK SPOT FOUND" (language string, logged; the HUD message is the free-roam messages' work).
const FOUND_TEXT: u32 = 1_444_754;
/// Loader model keys of parked cars (`TrafficModelRequest::vehicle`): above every traffic vehicle index.
pub const MODEL_KEY_BASE: usize = 1 << 24;
/// Seconds to wait for the loader before a swap is given up (ours).
const SWAP_TIMEOUT: f32 = 30.0;

pub struct JackSpotsPlugin {
    /// `--test jackspot`: swap at the spot nearest the spawn, with before / after screenshots.
    pub test: Option<Option<PathBuf>>,
}

impl Plugin for JackSpotsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<JackSpots>()
            .init_resource::<ParkedModels>()
            .init_resource::<Collected>()
            .add_systems(Startup, start_loading)
            .add_systems(Update, (receive, track_cars, fit_boxes, swap).chain().after(crate::drive::read_input));
        // `MW_RACE_TEST_JACK=1` with `--test race`: the same swap once the race is over (`race.rs`).
        let test = self.test.clone().or_else(|| std::env::var_os("MW_RACE_TEST_JACK").map(|_| None));
        if let Some(out) = test {
            app.insert_resource(JackTest { out, phase: 0, timer: 0.0, spot: None, old: String::new() })
                .add_systems(Update, run_test.before(receive));
        }
    }
}

/// Which paint of a car: a colour id of its palette (the spot's `ColourId`) or an index into its paints (the
/// car the player left behind).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Colour {
    Id(u32),
    Index(usize),
}

/// A parked car's render parts in car space.
struct ParkedModel {
    body: ModelParts,
    wheels: Vec<(WheelSpec, Vec<ModelParts>)>,
}

enum Parked {
    /// The game's state 0.
    None,
    /// Requested (states 1 / 2): the paint is being read, then the loader builds the model.
    Loading { serial: usize },
    /// The model is here, the ground under the pose is not (the track unit has not streamed in).
    Ready { model: ParkedModel },
    /// State 3: `at` is the car's origin on the ground.
    Spawned { root: Entity, collider: Entity, at: Vec3, heading: f32 },
}

struct Spot {
    data: SwapSpot,
    location: Trigger,
    /// The slot's content: the data's car, or the car the player left here.
    bundle: Option<String>,
    colour: Colour,
    /// Index of `colour` in the car's paints, known once its model was requested.
    paint_index: usize,
    /// Where the slot's car stands (trigger centre or a point on the road) and faces.
    position: Vec3,
    direction: Vec3,
    /// Slot +0x97: the next car left here goes on the car trigger instead of the second place.
    second_used: bool,
    /// The car the player left: stands at once (the game turns the parked vehicle into it).
    force: bool,
    d2: f32,
    parked: Parked,
}

impl Spot {
    fn new(data: SwapSpot, location: Trigger, bundle: Option<String>) -> Self {
        let (position, direction) = data.car.map_or((Vec3::from(location.position), Vec3::Z), |p| {
            (Vec3::from(p.position), Vec3::from(p.direction).normalize_or(Vec3::Z))
        });
        let colour = Colour::Id(data.colour);
        Self {
            data,
            location,
            bundle,
            colour,
            paint_index: 0,
            position,
            direction,
            second_used: false,
            force: false,
            d2: f32::MAX,
            parked: Parked::None,
        }
    }

    /// `FUN_007596b0`: back to the data's car on its own trigger (the caller removes the parked car).
    fn reset(&mut self, bundle: Option<String>) {
        *self = Self::new(self.data.clone(), self.location, bundle);
    }
}

/// A swap in progress: the loader is building the car the player asked for.
struct Swap {
    spot: usize,
    target: String,
    old_entity: Entity,
    old_bundle: String,
    old_paint: usize,
    old_pose: (Vec3, Vec3),
    visuals: Entity,
    started: f32,
}

type PaintAnswer = (usize, Option<(VehiclePaint, usize)>);

#[derive(Resource, Default)]
pub struct JackSpots {
    init: Option<Mutex<Receiver<Option<Init>>>>,
    spots: Vec<Spot>,
    /// Home bundle of each spot (the data's car), for resets.
    homes: Vec<Option<String>>,
    max_mph: f32,
    prompt_text: String,
    found_text: String,
    /// The prompt shows this frame (`DAT_01179ddd`).
    pub prompt: bool,
    /// +0x74c9: a swap happened and the player has not left the prompts since.
    latch: bool,
    /// +0x74b8: the slot last swapped at.
    last_swapped: Option<usize>,
    /// +0x74ac: the car the slots were built for.
    known_car: Option<String>,
    swap: Option<Swap>,
    /// Paint lookups (helper threads) and model requests in flight: serial → spot.
    paints: Option<(Sender<PaintAnswer>, Mutex<Receiver<PaintAnswer>>)>,
    serials: HashMap<usize, usize>,
    next_serial: usize,
    /// Location boxes the player's car is in (entry detection for "found").
    inside: Vec<bool>,
    /// A swap key press from the test.
    press: bool,
    /// Completed swaps (test / log).
    pub swaps: u32,
}

impl JackSpots {
    /// Easydrive's own `E` key yields to the swap while the prompt shows.
    pub fn prompting(&self) -> bool {
        self.prompt
    }
}

/// Parked car models from the loader (`Msg::TrafficCar` with a key from [`MODEL_KEY_BASE`] up).
#[derive(Resource, Default)]
pub struct ParkedModels {
    arrived: Vec<(usize, ParkedModel)>,
}

impl ParkedModels {
    pub fn arrived(&mut self, spec: &TrafficCarSpec, parts: &HashMap<ResourceId, ModelParts>) {
        let get = |m: &ResourceId| parts.get(m).cloned().unwrap_or_default();
        let model = ParkedModel {
            body: spec.body.iter().flat_map(get).collect(),
            wheels: spec.wheels.iter().map(|w| (w.clone(), w.parts.iter().map(|(m, _)| get(m)).collect())).collect(),
        };
        self.arrived.push((spec.vehicle - MODEL_KEY_BASE, model));
    }
}

struct Init {
    spots: Vec<(SwapSpot, Trigger, Option<String>)>,
    max_mph: f32,
    prompt_text: String,
    found_text: String,
}

/// The `VehicleList` id's bundle (`VEHICLES/VEH_<id>_HI.BNDL`).
fn vehicle_bundle(id: u32) -> String {
    format!("VEHICLES/VEH_{id}_HI.BNDL")
}

fn start_loading(game: Res<Game>, mut jack: ResMut<JackSpots>, pads: Query<&Gamepad>) {
    if !matches!(game.request, crate::loader::Request::Drive { world: true, .. }) {
        return;
    }
    let (tx, rx) = channel();
    let store = game.store.clone();
    let pad = pads.iter().next().is_some();
    crate::platform::thread::Builder::new()
        .name("mw-jackspots".into())
        .spawn(move || {
            let _ = tx.send(read_data(&store, pad));
        })
        .expect("spawn jack spot thread");
    jack.init = Some(Mutex::new(rx));
    jack.paints = Some({
        let (tx, rx) = channel();
        (tx, Mutex::new(rx))
    });
}

fn read_data(store: &AssetStore, pad: bool) -> Option<Init> {
    let cfg = store.start_config().map_err(|e| warn!("jack spots: no swap spot data ({e})")).ok()?;
    let triggers = store.trigger_data().map_err(|e| warn!("jack spots: no TriggerData ({e})")).ok()?;
    let by_id: HashMap<u32, &Trigger> = triggers.triggers.iter().map(|t| (t.id, t)).collect();
    let mut bundles: HashMap<u32, Option<String>> = HashMap::new();
    let mut spots = Vec::new();
    for s in &cfg.swap_spots {
        let Some(location) = by_id.get(&s.location) else { continue };
        let bundle = bundles
            .entry(s.vehicle)
            .or_insert_with(|| Some(vehicle_bundle(s.vehicle)).filter(|b| store.bundle(b).is_some()))
            .clone();
        spots.push((s.clone(), **location, bundle));
    }
    let lang = store.language(mw_assets::DEFAULT_LANGUAGE).ok();
    let text = |id: u32| lang.as_ref().and_then(|l| l.strings.get(&id).cloned());
    // The keyboard's key text from the UI config (`KEY_E`: SWAP_CARS' default key, `FUN_00483470`); the pad's
    // is the Y button (`FUN_00482250`).
    let key = if pad {
        "Y".to_string()
    } else {
        store.keyboard_button_text("KEY_E").and_then(text).unwrap_or_else(|| "E".into())
    };
    let prompt_text = mw_formats::ui::substitute(
        &text(PROMPT_TEXT).unwrap_or_else(|| "PRESS {Controls.ButtonPrompt[\"SWAP_CARS\"]} TO DRIVE CAR".into()),
        |expr| expr.starts_with("Controls.ButtonPrompt").then(|| key.clone()),
    );
    Some(Init {
        spots,
        max_mph: cfg.swap_max_mph.unwrap_or(DEFAULT_MAX_MPH),
        prompt_text,
        found_text: text(FOUND_TEXT).unwrap_or_else(|| "JACK SPOT FOUND".into()),
    })
}

fn receive(mut jack: ResMut<JackSpots>) {
    let Some(init) = jack.init.as_ref().and_then(|rx| rx.lock().unwrap().try_recv().ok()) else { return };
    jack.init = None;
    let Some(init) = init else { return };
    let with_second = init.spots.iter().filter(|s| s.0.second.is_some()).count();
    let without_car = init.spots.iter().filter(|s| s.2.is_none()).count();
    info!(
        "jack spots: {} ({} with a second place, {} without a car bundle), swap up to {} mph, prompt {:?}",
        init.spots.len(),
        with_second,
        without_car,
        init.max_mph,
        init.prompt_text
    );
    if std::env::var_os("MW_JACKSPOT_LOG").is_some() {
        for (s, ..) in &init.spots {
            if let (Some(c), Some(o)) = (s.car, s.second) {
                let f = Vec3::from(c.direction);
                let d = Vec3::from(o.position) - Vec3::from(c.position);
                let turn = f.dot(Vec3::from(o.direction));
                info!(
                    "jack spot {}: second place {} is {:.1} m ahead, {:.1} m to the side, {:.1} m up, facing dot {turn:.2}",
                    s.id,
                    o.trigger,
                    d.dot(f),
                    d.dot(Vec3::Y.cross(f)),
                    d.y
                );
            }
        }
    }
    jack.homes = init.spots.iter().map(|s| s.2.clone()).collect();
    jack.inside = vec![false; init.spots.len()];
    jack.spots = init.spots.into_iter().map(|(s, t, b)| Spot::new(s, t, b)).collect();
    jack.max_mph = init.max_mph;
    jack.prompt_text = init.prompt_text;
    jack.found_text = init.found_text;
}

/// The rotation of a car standing on a surface with normal `up`, facing `direction` (local +Z forward).
fn standing_rotation(direction: Vec3, up: Vec3) -> Quat {
    let forward = (direction - up * direction.dot(up)).normalize_or(Vec3::Z);
    Quat::from_mat3(&Mat3::from_cols(up.cross(forward), up, forward))
}

fn remove(commands: &mut Commands, parked: &mut Parked) {
    if let Parked::Spawned { root, .. } = parked {
        commands.entity(*root).despawn();
    }
    *parked = Parked::None;
}

/// Which parked cars stand: distances, requests, removal, and building the cars whose model has arrived.
#[allow(clippy::too_many_arguments)]
fn track_cars(
    mut commands: Commands,
    mut jack: ResMut<JackSpots>,
    mut models: ResMut<ParkedModels>,
    game: Res<Game>,
    garage: Res<Garage>,
    requests: Option<Res<LoaderRequests>>,
    player: Query<&Transform, (With<Car>, With<Player>)>,
    focus: Option<Res<crate::loader::StreamFocus>>,
    spatial: SpatialQuery,
) {
    let jack = &mut *jack;
    if jack.spots.is_empty() {
        return;
    }
    // The driven car changed outside a swap (menu, Easydrive): every slot back to its data (`FUN_00761d70`).
    // A respray keeps the car (`bodyshop.rs`).
    let current = garage.current.as_ref().map(|c| c.bundle.clone());
    if jack.swap.is_none() && current != jack.known_car {
        if jack.known_car.is_some() && current.is_some() {
            for (spot, home) in jack.spots.iter_mut().zip(&jack.homes) {
                remove(&mut commands, &mut spot.parked);
                spot.reset(home.clone());
            }
            jack.serials.clear();
            jack.last_swapped = None;
        }
        jack.known_car = current;
    }
    let Some(p) = player.iter().next().map(|t| t.translation) else { return };

    // `FUN_007610c0`: the location triggers within 300 m, nearest first; the first five keep their distance.
    let mut near: Vec<(usize, f32)> = jack
        .spots
        .iter()
        .enumerate()
        .map(|(i, s)| (i, Vec3::from(s.location.position).distance_squared(p)))
        .filter(|&(_, d2)| d2 <= QUERY_RADIUS * QUERY_RADIUS)
        .collect();
    near.sort_by(|a, b| a.1.total_cmp(&b.1));
    near.truncate(NEAREST);
    for s in &mut jack.spots {
        s.d2 = f32::MAX;
    }
    for &(i, d2) in &near {
        jack.spots[i].d2 = d2;
    }

    // `FUN_0075d800`: the slot last swapped at goes back to its data 300 m away.
    if let Some(i) = jack.last_swapped
        && jack.swap.is_none()
    {
        let home = jack.spots[i].data.car.map_or(jack.spots[i].position, |c| Vec3::from(c.position));
        if home.distance_squared(p) > SPAWN_MAX_D2 {
            let s = &mut jack.spots[i];
            remove(&mut commands, &mut s.parked);
            s.reset(jack.homes[i].clone());
            jack.last_swapped = None;
        }
    }

    // `FUN_0075d290` per slot.
    let swapping = jack.swap.as_ref().map(|s| s.spot);
    for i in 0..jack.spots.len() {
        let s = &mut jack.spots[i];
        let standing = matches!(s.parked, Parked::Spawned { .. });
        // A car still loading when the player comes within 20 m is dropped again, as in the game (only a
        // standing car, state 3, is kept inside the ring).
        let wanted = s.bundle.is_some()
            && (s.force || (s.d2 <= SPAWN_MAX_D2 && s.d2 >= SPAWN_MIN_D2) || (standing && s.d2 <= SPAWN_MAX_D2));
        if !wanted {
            if !matches!(s.parked, Parked::None) && swapping != Some(i) {
                remove(&mut commands, &mut s.parked);
                jack.serials.retain(|_, spot| *spot != i);
            }
            continue;
        }
        if matches!(s.parked, Parked::None) && swapping != Some(i) {
            let (Some(bundle), Some((tx, _))) = (s.bundle.clone(), jack.paints.as_ref()) else { continue };
            let serial = jack.next_serial;
            jack.next_serial += 1;
            jack.serials.insert(serial, i);
            s.parked = Parked::Loading { serial };
            let (store, tx, colour) = (game.store.clone(), tx.clone(), s.colour);
            crate::platform::thread::Builder::new()
                .name("mw-jackspot-paint".into())
                .spawn(move || {
                    let _ = tx.send((serial, read_paint(&store, &bundle, colour)));
                })
                .expect("spawn jack spot paint thread");
        }
    }

    // Paints read: ask the loader for the model in that paint.
    let answers: Vec<PaintAnswer> =
        jack.paints.as_ref().map(|(_, rx)| rx.lock().unwrap().try_iter().collect()).unwrap_or_default();
    for (serial, answer) in answers {
        let Some(&i) = jack.serials.get(&serial) else { continue };
        let s = &mut jack.spots[i];
        if !matches!(s.parked, Parked::Loading { serial: n } if n == serial) {
            continue;
        }
        let (Some(bundle), Some(requests)) = (s.bundle.clone(), requests.as_ref()) else { continue };
        s.paint_index = answer.map_or(0, |(_, index)| index);
        let _ = requests.0.send(CarRequest {
            bundle,
            paint_index: 0,
            traffic: Some(TrafficModelRequest {
                vehicle: MODEL_KEY_BASE + serial,
                paint: 0,
                colour: answer.map(|(paint, _)| paint),
            }),
            opponent: None,
        });
    }
    for (serial, model) in models.arrived.drain(..) {
        // The parked car holds its meshes and materials itself: the loader can forget them, so they go with the
        // car (`streaming.rs`; before, every parked car's model stayed loaded).
        if let Some(f) = &focus {
            f.release(crate::loader::PARKED_OWNER | serial as u32);
        }
        let Some(i) = jack.serials.remove(&serial) else { continue };
        let s = &mut jack.spots[i];
        if matches!(s.parked, Parked::Loading { serial: n } if n == serial) {
            s.parked = Parked::Ready { model };
        }
    }

    // Stand the cars whose ground is there.
    let ground = SpatialQueryFilter::from_mask(GameLayer::World);
    for s in &mut jack.spots {
        if !matches!(s.parked, Parked::Ready { .. }) {
            continue;
        }
        // The trigger's centre is about 1.5 m above the road; a car left here has its origin on the road.
        let from = s.position + Vec3::Y * 1.0;
        let Some(hit) = spatial.cast_ray(from, Dir3::NEG_Y, 8.0, true, &ground) else { continue };
        let Parked::Ready { model } = std::mem::replace(&mut s.parked, Parked::None) else { continue };
        let at = from - Vec3::Y * hit.distance;
        let rotation = standing_rotation(s.direction, hit.normal);
        let root = commands
            .spawn((
                Name::new("jack spot car"),
                Transform::from_translation(at).with_rotation(rotation),
                Visibility::default(),
                RigidBody::Static,
            ))
            .id();
        // MODEL: one static box around the body meshes (the game's parked cars are physical vehicles).
        // A stand-in size until Bevy has the meshes' bounds (`fit_boxes`).
        let (lo, hi) = (Vec3::new(-0.95, 0.0, -2.2), Vec3::new(0.95, 1.3, 2.2));
        let size = hi - lo;
        let collider = commands
            .spawn((
                Collider::cuboid(size.x, size.y, size.z),
                Transform::from_translation((lo + hi) * 0.5),
                CollisionLayers::new(GameLayer::World, LayerMask::ALL),
                ChildOf(root),
            ))
            .id();
        commands.entity(root).insert(ParkedBox { collider });
        let part = |commands: &mut Commands, parts: &ModelParts, parent: Entity| {
            for (mesh, material) in parts {
                commands.spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d::<GameMaterial>(material.clone()),
                    crate::shadow_cull::ShadowClass::Car,
                    ChildOf(parent),
                ));
            }
        };
        part(&mut commands, &model.body, root);
        for (w, parts) in &model.wheels {
            let mirror = if w.mirror { Vec3::new(-1.0, 1.0, 1.0) } else { Vec3::ONE };
            let wheel = commands
                .spawn((
                    Transform::from_translation(w.position).with_rotation(w.rotation).with_scale(mirror),
                    Visibility::default(),
                    ChildOf(root),
                ))
                .id();
            for p in parts {
                part(&mut commands, p, wheel);
            }
        }
        let heading = s.direction.x.atan2(s.direction.z);
        info!(
            "jack spot {}: {} (paint {}) parked at {at:.1}, {:.0} m away",
            s.data.id,
            s.bundle.as_deref().unwrap_or("?"),
            s.paint_index,
            s.d2.sqrt()
        );
        s.parked = Parked::Spawned { root, collider, at, heading };
    }
}

/// A parked car whose box collider still has the stand-in size.
#[derive(Component)]
struct ParkedBox {
    collider: Entity,
}

/// Size each parked car's box to its body meshes' bounds once Bevy has computed them (the body meshes are the
/// root's own children, in car space).
fn fit_boxes(
    mut commands: Commands,
    cars: Query<(Entity, &ParkedBox, &Children)>,
    bodies: Query<Option<&Aabb>, With<Mesh3d>>,
) {
    'cars: for (root, parked, children) in &cars {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for kid in children.iter() {
            match bodies.get(kid) {
                Ok(Some(b)) => {
                    lo = lo.min(Vec3::from(b.min()));
                    hi = hi.max(Vec3::from(b.max()));
                }
                Ok(None) => continue 'cars,
                Err(_) => {}
            }
        }
        commands.entity(root).remove::<ParkedBox>();
        if lo.x > hi.x {
            continue;
        }
        let size = (hi - lo).max(Vec3::splat(0.2));
        if let Ok(mut collider) = commands.get_entity(parked.collider) {
            collider
                .try_insert((Collider::cuboid(size.x, size.y, size.z), Transform::from_translation((lo + hi) * 0.5)));
        }
    }
}

/// The paint a colour of the car's palette stands for, and its index in the car's paints (`FUN_0058b9a0`: an
/// id that is not in the palette gives the palette's first entry).
fn read_paint(store: &AssetStore, bundle: &str, colour: Colour) -> Option<(VehiclePaint, usize)> {
    let b = store.bundle(bundle)?;
    let paints = store.vehicle_paints(&b).ok()?;
    let index = match colour {
        Colour::Index(i) => i,
        Colour::Id(id) => {
            let palette = store.vehicle_palette(&b).unwrap_or_default();
            palette.iter().find(|(_, c)| *c == id).or(palette.first()).map_or(0, |(i, _)| *i)
        }
    };
    let index = index.min(paints.len().checked_sub(1)?);
    Some((paints[index], index))
}

/// The prompt, the swap key and the swap itself; "found" bookkeeping.
#[allow(clippy::too_many_arguments)]
fn swap(
    mut commands: Commands,
    mut jack: ResMut<JackSpots>,
    mut garage: ResMut<Garage>,
    mut collected: ResMut<Collected>,
    mut actions: MessageWriter<GarageAction>,
    mut scoring: MessageWriter<crate::freeroam::ScoreAction>,
    mut hud: Option<ResMut<crate::hud::HudState>>,
    input: Res<CarInput>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    time: Res<Time<Real>>,
    race: Option<Res<crate::race::RaceState>>,
    pending: Option<Res<PendingCar>>,
    player: Query<(Entity, &Car, &Transform), With<Player>>,
) {
    let jack = &mut *jack;
    jack.prompt = false;
    let pressed = std::mem::take(&mut jack.press) || {
        let pads: Vec<&Gamepad> = pads.iter().collect();
        crate::input::Devices { keys: &keys, pads: &pads }.just_pressed(crate::input::defaults::SWAP_CARS)
    };
    let now = time.elapsed_secs();
    let mut show = None;
    if let Some(swap) = jack.swap.as_ref() {
        // The loader's car has replaced the one the player came in (`garage::place_arrived_car`).
        let arrived = player.iter().next().filter(|(e, ..)| *e != swap.old_entity);
        let failed = garage.loading.is_none()
            && pending.is_none()
            && arrived.is_none()
            && garage.place_at.is_some()
            && now - swap.started > 1.0;
        if let Some((_, _, t)) = arrived {
            let swap = jack.swap.take().expect("checked");
            commands.entity(swap.visuals).despawn();
            let s = &mut jack.spots[swap.spot];
            // `FUN_00761d70` state 3: where the car the player came in goes.
            let (position, direction) = match (s.data.second, s.data.car) {
                (Some(second), _) if !s.second_used => {
                    s.second_used = true;
                    (Vec3::from(second.position), Vec3::from(second.direction))
                }
                (Some(_), Some(car)) => {
                    s.second_used = false;
                    (Vec3::from(car.position), Vec3::from(car.direction))
                }
                _ => swap.old_pose,
            };
            // `FUN_00758f00`: the slot now holds that car in its colour.
            s.bundle = Some(swap.old_bundle.clone());
            s.colour = Colour::Index(swap.old_paint);
            s.position = position;
            s.direction = direction.normalize_or(Vec3::Z);
            s.force = true;
            s.parked = Parked::None;
            // State 6: both triggers found, the slot is the last swap spot.
            collected.jack_spots.insert(s.data.location);
            // The car's first find scores `CarSwapDiscovered` (350 SP in the data; decomp.md "Speed Points").
            if let Some(car) = s.data.car
                && collected.jack_cars.insert(car.trigger)
            {
                scoring.write(crate::freeroam::ScoreAction(mw_assets::scoring::action::CAR_SWAP_DISCOVERED));
            }
            collected.swap_spot = Some(s.data.location);
            collected.dirty = true;
            jack.last_swapped = Some(swap.spot);
            jack.known_car = Some(swap.target.clone());
            jack.latch = true;
            jack.swaps += 1;
            info!(
                "jack spot {}: now driving {} at {:.1}; {} left at {position:.1}",
                s.data.id, swap.target, t.translation, swap.old_bundle
            );
        } else if failed || now - swap.started > SWAP_TIMEOUT {
            let swap = jack.swap.take().expect("checked");
            warn!("jack spot {}: the swap to {} did not happen", jack.spots[swap.spot].data.id, swap.target);
            commands.entity(swap.visuals).despawn();
            garage.place_at = None;
            jack.spots[swap.spot].parked = Parked::None;
        }
    } else if let Some((entity, car, t)) = player.iter().next()
        && !jack.spots.is_empty()
    {
        let p = t.translation.to_array();
        let mph = car.speed.abs() / 0.447_04;
        // `FUN_007615f0`'s gates (the pursuit one is not modelled); jack spot triggers fire in free roam only
        // (`FUN_00707650`: game-state byte +0x2e90).
        let free_roam = race.is_none();
        let can = free_roam && !input.crashed && mph <= jack.max_mph;
        let mut near_any = false;
        for i in 0..jack.spots.len() {
            let s = &jack.spots[i];
            let inside = s.d2 < f32::MAX && s.location.contains(p);
            let entered = !std::mem::replace(&mut jack.inside[i], inside) && inside;
            // The car the player left stands for the game from the moment of the swap (ours is still loading):
            // the swap latch holds meanwhile.
            near_any |= inside && can && s.force;
            let Parked::Spawned { root, collider, at, heading } = s.parked else { continue };
            if !inside {
                continue;
            }
            // `FUN_007059b0`: the location counts once its car stands.
            if (entered || !collected.jack_spots.contains(&s.data.location))
                && free_roam
                && collected.jack_spots.insert(s.data.location)
            {
                collected.dirty = true;
                info!("{}: jack spot {} ({} found)", jack.found_text, s.data.id, collected.jack_spots.len());
            }
            if !can {
                continue;
            }
            near_any = true;
            if jack.latch {
                continue;
            }
            show = Some(jack.prompt_text.clone());
            jack.prompt = true;
            let (Some(target), Some(current)) = (s.bundle.clone(), garage.current.clone()) else { continue };
            if !pressed || garage.loading.is_some() || pending.is_some() {
                continue;
            }
            let name = garage
                .catalogue
                .iter()
                .find(|c| c.bundle.eq_ignore_ascii_case(&target))
                .map_or_else(|| target.clone(), |c| c.name.clone());
            info!("jack spot {}: swapping {} for {name} at {mph:.0} mph", s.data.id, current.bundle);
            // The player's car takes the parked car's pose (`FUN_00742530` / `FUN_00742580`), at rest; the parked
            // car's box goes now so the new car lands on the road, its picture when the new car is there.
            commands.entity(collider).despawn();
            garage.place_at = Some((at, heading));
            actions.write(GarageAction::Drive { name, bundle: target.clone(), paint_index: s.paint_index });
            jack.swap = Some(Swap {
                spot: i,
                target,
                old_entity: entity,
                old_bundle: current.bundle,
                old_paint: current.paint_index,
                old_pose: (t.translation, t.rotation * Vec3::Z),
                visuals: root,
                started: now,
            });
            jack.prompt = false;
            show = None;
            break;
        }
        if jack.latch && !near_any {
            jack.latch = false;
        }
    }
    if let Some(hud) = hud.as_mut()
        && hud.prompt != show
    {
        hud.prompt = show;
    }
}

// ---------------------------------------------------------------------------------------------
// `--test jackspot`: swap at the spot nearest the spawn point; screenshots before and after.

#[derive(Resource)]
struct JackTest {
    out: Option<PathBuf>,
    phase: u8,
    timer: f32,
    spot: Option<usize>,
    old: String,
}

impl JackTest {
    fn shoot(&self, commands: &mut Commands, suffix: &str) {
        let Some(out) = &self.out else { return };
        let stem = out.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let ext = out.extension().map_or("png".into(), |e| e.to_string_lossy().to_string());
        let path = out.with_file_name(format!("{stem}-{suffix}.{ext}"));
        info!("TEST jackspot: screenshot {}", path.display());
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(path));
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run_test(
    mut commands: Commands,
    mut test: ResMut<JackTest>,
    mut jack: ResMut<JackSpots>,
    mut input: ResMut<CarInput>,
    time: Res<Time<Real>>,
    garage: Res<Garage>,
    collected: Res<Collected>,
    spatial: SpatialQuery,
    mut player: Query<(Entity, &Car, &mut Transform, &mut LinearVelocity, &mut AngularVelocity), With<Player>>,
    race: Option<Res<crate::race::RaceState>>,
    mut exit: MessageWriter<AppExit>,
) {
    // After a race (`MW_RACE_TEST_JACK`): start once the event is over and free roam is back.
    if race.is_some() {
        test.timer = 0.0;
        return;
    }
    test.timer += time.delta_secs();
    input.scripted = Some([0.0, 0.0, 0.0, 1.0]);
    let fail = |what: &str, exit: &mut MessageWriter<AppExit>| {
        println!("TEST RESULT jackspot FAILED: {what}");
        exit.write(AppExit::Success);
    };
    let Ok((entity, car, mut t, mut v, mut w)) = player.single_mut() else {
        if test.timer > 120.0 {
            fail("no player car", &mut exit);
        }
        return;
    };
    match test.phase {
        // The nearest spot's car has to stand: the spawn point must be 20 m to 300 m from it.
        0 => {
            if jack.spots.is_empty() || test.timer < 3.0 {
                return;
            }
            let p = t.translation;
            let nearest = (0..jack.spots.len())
                .min_by(|&a, &b| jack.spots[a].d2.total_cmp(&jack.spots[b].d2))
                .filter(|&i| jack.spots[i].d2 < f32::MAX);
            let Some(i) = nearest else {
                return fail(&format!("no jack spot within 300 m of {p:.0}"), &mut exit);
            };
            let s = &jack.spots[i];
            let Parked::Spawned { at, heading, .. } = s.parked else {
                if test.timer > 90.0 {
                    fail(
                        &format!("the car of spot {} did not appear ({:.0} m away)", s.data.id, s.d2.sqrt()),
                        &mut exit,
                    );
                }
                return;
            };
            println!(
                "TEST RESULT jackspot spot {} location {} vehicle {} colour {} -> {} paint {} parked at {:.1} {:.1} {:.1}, {:.0} m from the player",
                s.data.id,
                s.data.location,
                s.data.vehicle,
                s.data.colour,
                s.bundle.as_deref().unwrap_or("?"),
                s.paint_index,
                at.x,
                at.y,
                at.z,
                s.d2.sqrt()
            );
            // Pull up beside the parked car, inside the location box.
            let forward = Vec3::new(heading.sin(), 0.0, heading.cos());
            let right = Vec3::Y.cross(forward);
            let filter = SpatialQueryFilter::from_mask(GameLayer::World);
            let place = [right * 3.2, right * -3.2, forward * -7.0, forward * 7.0, right * 4.5, right * -4.5]
                .into_iter()
                .map(|o| at + o)
                .filter(|q| s.location.contains((*q + Vec3::Y * 0.5).to_array()))
                .find_map(|q| {
                    let from = q + Vec3::Y * 1.5;
                    let hit = spatial.cast_ray(from, Dir3::NEG_Y, 4.0, true, &filter)?;
                    // On the same level as the parked car, with room above.
                    let ground = from - Vec3::Y * hit.distance;
                    ((ground.y - at.y).abs() < 0.6).then_some(ground)
                });
            let Some(place) = place else {
                return fail(&format!("no free place beside the car of spot {}", s.data.id), &mut exit);
            };
            t.translation = place + Vec3::Y * 0.3;
            t.rotation = Quat::from_rotation_y(heading);
            v.0 = Vec3::ZERO;
            w.0 = Vec3::ZERO;
            commands.entity(entity).insert((Position(t.translation), Rotation(t.rotation)));
            test.old = garage.current.as_ref().map(|c| c.bundle.clone()).unwrap_or_default();
            test.spot = Some(i);
            test.phase = 1;
            test.timer = 0.0;
        }
        // Settled beside it: the prompt shows.
        1 => {
            if test.timer < 3.0 {
                return;
            }
            let i = test.spot.expect("set in phase 0");
            if !jack.prompt {
                if test.timer > 15.0 {
                    let s = &jack.spots[i];
                    fail(
                        &format!(
                            "no prompt beside the car of spot {} (in the box: {}, {:.1} mph)",
                            s.data.id,
                            s.location.contains(t.translation.to_array()),
                            car.speed / 0.447_04
                        ),
                        &mut exit,
                    );
                }
                return;
            }
            println!(
                "TEST RESULT jackspot prompt {:?} in {} at {:.1} mph (limit {} mph)",
                jack.prompt_text,
                test.old,
                car.speed.abs() / 0.447_04,
                jack.max_mph
            );
            test.shoot(&mut commands, "before");
            test.phase = 2;
            test.timer = 0.0;
        }
        2 => {
            if test.timer > 0.5 {
                jack.press = true;
                test.phase = 3;
                test.timer = 0.0;
            }
        }
        3 => {
            if jack.swaps == 0 {
                if test.timer > 60.0 {
                    fail("the swap did not finish", &mut exit);
                }
                return;
            }
            let i = test.spot.expect("set in phase 0");
            let s = &jack.spots[i];
            // The car left behind stands again.
            if !matches!(s.parked, Parked::Spawned { .. }) && test.timer < 40.0 {
                return;
            }
            test.phase = 4;
            test.timer = 0.0;
        }
        4 => {
            if test.timer < 3.0 {
                return;
            }
            let i = test.spot.expect("set in phase 0");
            let s = &jack.spots[i];
            let now = garage.current.as_ref().map(|c| (c.bundle.clone(), c.paint_index));
            let left = match s.parked {
                Parked::Spawned { at, .. } => format!("parked at {:.1} {:.1} {:.1}", at.x, at.y, at.z),
                _ => "NOT standing".to_string(),
            };
            let p = t.translation;
            println!(
                "TEST RESULT jackspot swapped: driving {:?} at {:.1} {:.1} {:.1} ({:.1} mph); {} (paint {:?}) {left}; prompt {} (latched {})",
                now,
                p.x,
                p.y,
                p.z,
                car.speed / 0.447_04,
                s.bundle.as_deref().unwrap_or("?"),
                s.colour,
                jack.prompt,
                jack.latch
            );
            println!(
                "TEST RESULT jackspot found: {} locations {:?}, {} cars {:?}, last swap spot {:?}",
                collected.jack_spots.len(),
                collected.jack_spots,
                collected.jack_cars.len(),
                collected.jack_cars,
                collected.swap_spot
            );
            test.shoot(&mut commands, "after");
            test.phase = 5;
            test.timer = 0.0;
        }
        _ => {
            if test.timer > 1.5 {
                exit.write(AppExit::Success);
            }
        }
    }
}
