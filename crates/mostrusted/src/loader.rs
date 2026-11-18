//! Background asset streaming.
//!
//! A worker thread reads bundles from the user's game install, decodes
//! geometry and textures, converts them into Bevy assets and hands them to
//! the main thread over a channel. Track units are streamed nearest-first
//! from the focus point, much like the original game streams its world.

use crate::platform::PollLock;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use bevy::prelude::*;
use mw_assets::AssetStore;
use mw_assets::backdrop::{BackdropSelector, track_unit_number};
use mw_bundle::{ResourceId, ResourceType};

use crate::convert;
use crate::materials::{Blend, InstanceLook, Look, TexKey};

/// What to load.
#[derive(Debug, Clone)]
pub enum Request {
    /// Stream world track units nearest-first from `focus` (default: the
    /// city centre). `max_units: None` loads all of them.
    World { max_units: Option<usize>, focus: Option<Vec3> },
    /// Load every model of a vehicle bundle at the origin.
    Vehicle { bundle: String },
    /// Load a drivable car, then stream the world around `spawn`.
    /// `world: false` loads only the car (for the test track), plus `props` placed on it.
    Drive { bundle: String, spawn: Vec3, heading: f32, max_units: Option<usize>, world: bool, props: Vec<TestProp> },
}

/// A world object placed by hand (the `--test smash` track).
#[derive(Debug, Clone)]
pub struct TestProp {
    pub object: ResourceId,
    pub kind: mw_assets::props::ObjectKind,
    pub position: Vec3,
    /// Yaw in radians.
    pub yaw: f32,
}

/// A car to load while the session runs (the in-game car menu, `--swap-to`). The world is not
/// reloaded: the loader answers with `Msg::Car` (or `Msg::CarFailed`) between track units.
#[derive(Debug, Clone)]
pub struct CarRequest {
    pub bundle: String,
    /// Which of the car's `VehiclePaint` options to use (clamped to the ones it has).
    pub paint_index: usize,
    /// A traffic car's model instead of a drivable car (`traffic.rs`): answered with `Msg::TrafficCar`.
    pub traffic: Option<TrafficModelRequest>,
    /// An opponent's slot (`ai.rs`): the car comes back with [`CarSpec::opponent`] set instead of going to the garage.
    pub opponent: Option<usize>,
    /// Not a car: a world object placed at runtime (a roadblock's cones and barriers, `roadblock.rs`), answered with
    /// `Msg::Prop` carrying the request's tag; `bundle` is unused.
    pub prop: Option<(TestProp, u32)>,
}

/// A traffic car model in one paint (traffic cars keep their paints outside their bundle).
#[derive(Debug, Clone)]
pub struct TrafficModelRequest {
    /// Index into `TrafficData::vehicles` and into that type's paints, echoed back.
    pub vehicle: usize,
    pub paint: usize,
    pub colour: Option<mw_assets::paint::VehiclePaint>,
}

/// A loaded traffic car model: body parts and wheels in car space.
#[derive(Debug, Clone)]
pub struct TrafficCarSpec {
    pub vehicle: usize,
    pub paint: usize,
    pub body: Vec<ResourceId>,
    pub wheels: Vec<WheelSpec>,
    /// Height of the model origin above the road (`VehicleInfo.509931cd` translation y): the body and wheels
    /// are drawn this much above the lane.
    pub ride_height: f32,
    /// The car's coronas (`VehicleVfxBehaviour.Coronas`, `coronas.rs`).
    pub coronas: Arc<Vec<mw_assets::coronas::CarCorona>>,
}

/// Sends [`CarRequest`]s to the loader thread.
#[derive(Resource)]
pub struct LoaderRequests(pub Sender<CarRequest>);

/// A wheel of a drivable car, in car space.
#[derive(Debug, Clone)]
pub struct WheelSpec {
    pub position: Vec3,
    pub rotation: Quat,
    pub mirror: bool,
    pub front: bool,
    /// Part models and whether each one spins with the wheel.
    pub parts: Vec<(ResourceId, bool)>,
}

/// Everything needed to spawn a drivable car.
#[derive(Debug, Clone)]
pub struct CarSpec {
    /// The vehicle bundle it came from, and its paint (index, number of paints the car has).
    pub bundle: String,
    pub paint_index: usize,
    pub paint_count: usize,
    /// All of the car's paints (`VehiclePaint`, in the order `paint_index` counts), for a respray in place
    /// (`paint_wear::Respray`).
    pub paints: Arc<Vec<mw_assets::paint::VehiclePaint>>,
    pub body: Vec<ResourceId>,
    pub wheels: Vec<WheelSpec>,
    /// Handling parameters from the car's GeneSys data.
    pub handling: mw_assets::handling::VehicleHandling,
    /// Performance mods (`mods_menu.rs`); `handling` is the stock car.
    pub mod_catalogue: crate::mods_menu::ModCatalogue,
    /// Engine sound (`AssetStore::engine`): the car's sequence bank, or Ginsu layers for cars without one.
    pub engine: Option<mw_assets::engine_sound::EngineVoice>,
    /// An AI opponent's engine (`<id>_COMPETITORSOUND`, `mw_assets::car_sound`) and how it sits in the world;
    /// loaded instead of [`CarSpec::engine`] for opponents.
    pub competitor_engine: Option<Arc<mw_assets::car_sound::CompetitorEngine>>,
    /// Deformation data; the body meshes carry joint attributes when this is set (`damage.rs`).
    pub damage: Option<Arc<mw_assets::damage::VehicleDamage>>,
    pub impact_graph: mw_assets::damage::ImpactGraph,
    /// `VehicleVfxBehaviour` spot effects with their locators (exhaust flames, smoke, ...; `exhaust.rs`).
    pub spot_effects: Vec<mw_assets::vfx::VehicleSpotEffect>,
    /// The cop lights and siren flag (`mw_assets::police_lights`; `police_fx.rs`), when the car has any.
    pub flash_lights: Option<Arc<mw_assets::police_lights::CarFlashLights>>,
    /// The car's coronas (`VehicleVfxBehaviour.Coronas`, `coronas.rs`).
    pub coronas: Arc<Vec<mw_assets::coronas::CarCorona>>,
    /// The light sources the car carries (`VehicleVfxBehaviour.Lights`, `car_lights.rs`).
    pub light_sources: Arc<Vec<mw_assets::car_lights::CarLightSource>>,
    /// Set for an AI opponent's car (its slot in `ai::Opponents`); `None` for the player's.
    pub opponent: Option<usize>,
    /// The vehicle list's top speed (entry +0x3C, mph): the nitrous HighSpeed refill's reference.
    pub top_speed_mph: f32,
    /// `VehicleInfo.76bfd668`: zone damage → paint scratch weight (`paint_wear.rs`).
    pub scratch_scale: f32,
}

/// The vehicle id in a `VEHICLES/VEH_<id>_HI.BNDL` path.
fn bundle_vehicle_id(bundle: &str) -> Option<u32> {
    let name = bundle.rsplit('/').next()?.to_ascii_uppercase();
    name.strip_prefix("VEH_")?.split('_').next()?.parse().ok()
}

#[derive(Clone)]
pub struct Part {
    pub mesh: Mesh,
    pub look: Look,
}

pub enum Msg {
    /// Where the camera should start.
    Focus(Vec3),
    Texture(TexKey, Image),
    Model(ResourceId, Vec<Part>),
    /// A model's own shadow distance (`Model` +0x0C, metres), sent before its `Model` when it is finite.
    ShadowRange(ResourceId, f32),
    /// A model's pass-mask bits that keep it out of some views (`pass_mask.rs`), sent before its `Model`.
    PassMask(ResourceId, u8),
    Instance {
        model: ResourceId,
        transform: Mat4,
        /// World placements: the game's LODs and draw distance (`lod.rs`).
        lod: Option<crate::lod::LodPlacement>,
    },
    /// The streaming zones (`ZoneList`), for the groundcover residency (`groundcover.rs`).
    Zones(mw_formats::ZoneList),
    /// The camera zones that show the `Instance`s that follow (`zone_view.rs`); `None` ends the tagging.
    ZoneTag(Option<crate::zone_view::ZoneTag>),
    /// A track unit (= zone) has been sent completely: its collision, placements and groundcover
    /// (`frontend::Residency`).
    UnitLoaded(u32),
    /// What follows (up to [`Msg::UnitLoaded`]) belongs to this track unit: it goes when the unit is unloaded
    /// (`streaming.rs`).
    UnitBegin(u32),
    /// The instances that follow are shared by several units (a backdrop placement, by its key): they go when
    /// no resident unit lists it any more. `None` ends the shared run.
    StreamKey(Option<u64>),
    /// A track unit left the stream set (`streaming.rs`): its entities go, and the models, textures, shared
    /// backdrop instances and groundcover collections no resident unit uses any more are dropped.
    UnitUnloaded(Box<Unloaded>),
    /// The world is streamed around the focus (not loaded once): `Done` then means the first stream set is in.
    Streaming,
    /// The groundcover collections a track unit lists (also those already sent for another unit).
    GroundcoverListing {
        unit: u32,
        collections: Vec<ResourceId>,
    },
    /// One groundcover group (a model at the origin), drawn while the camera is within `range` of its box
    /// and a unit around the camera lists `collection`.
    Groundcover {
        model: ResourceId,
        collection: ResourceId,
        min: Vec3,
        max: Vec3,
        range: f32,
        /// World bounds of every sprite turned any way and swaying (the mesh holds only base points).
        bounds: (Vec3, Vec3),
    },
    /// A prop / compound / dynamic object with physics (`props.rs`).
    Prop {
        model: ResourceId,
        transform: Mat4,
        physics: Arc<crate::props::PropPhysics>,
        /// Compound objects: the model's geometry per part (its skeleton bone; `None` where a part has none),
        /// then the triangles of bones no part owns.
        pieces: Option<(Vec<Option<ResourceId>>, Option<ResourceId>)>,
        /// The game's LODs and draw distance (`lod.rs`).
        lod: Option<crate::lod::LodPlacement>,
        /// Compounds: `model` is the standing model (`CompoundObject` +0x24), not the pieces' (+0x04).
        standing: bool,
        /// Billboard compounds: the model they stand as once found (+0x28) and its LODs.
        found: Option<(ResourceId, Option<crate::lod::LodPlacement>)>,
        /// A runtime placement's tag ([`CarRequest::prop`]): not part of any track unit.
        tag: Option<u32>,
    },
    /// Static world collision (one per `PolygonSoupList`) and the collision tag of each of its triangles.
    Collider(avian3d::prelude::Collider, Arc<Vec<u32>>),
    /// The game's collision surface list (`mw_assets::surfaces`).
    Surfaces(Vec<mw_assets::surfaces::SurfaceInfo>),
    /// A track unit's lights (`LightInstanceList`; `world_lights.rs`).
    Lights(Vec<mw_assets::lights::WorldLight>),
    /// How cars' contacts with knocked props are solved (`mw_assets::props::DynamicResponses`).
    PropResponses(mw_assets::props::DynamicResponses),
    Car(Box<CarSpec>),
    /// A traffic car model ([`CarRequest::traffic`]).
    TrafficCar(Box<TrafficCarSpec>),
    /// A runtime [`CarRequest`] could not be loaded.
    CarFailed(String),
    Progress(String),
    Done,
}

/// What [`Msg::UnitUnloaded`] frees.
#[derive(Debug, Default)]
pub struct Unloaded {
    pub unit: u32,
    pub models: Vec<ResourceId>,
    pub textures: Vec<TexKey>,
    /// Shared backdrop instances ([`Msg::StreamKey`]).
    pub backdrops: Vec<u64>,
    /// Groundcover collections (`Msg::Groundcover::collection`).
    pub groundcover: Vec<ResourceId>,
}

/// The streaming focus the main thread hands the loader: the camera's position and view direction (the game's
/// `FUN_00558740` passes the camera manager's +0x140 / +0x130 to `FUN_005586e0`; `streaming.rs`).
/// Also carries owners whose models the main thread no longer needs ([`StreamFocus::release`]).
#[derive(Resource, Clone)]
pub struct StreamFocus {
    focus: Arc<std::sync::Mutex<Option<(Vec3, Vec3)>>>,
    releases: std::sync::mpsc::Sender<u32>,
    /// Read by the loader thread only.
    released: Arc<std::sync::Mutex<std::sync::mpsc::Receiver<u32>>>,
}

impl Default for StreamFocus {
    fn default() -> Self {
        let (releases, released) = std::sync::mpsc::channel();
        Self { focus: Default::default(), releases, released: Arc::new(std::sync::Mutex::new(released)) }
    }
}

impl StreamFocus {
    pub fn set(&self, at: Vec3, direction: Vec3) {
        // Polled: the main thread must not block (a browser traps there); a busy lock skips one frame's focus.
        if let Some(mut f) = self.focus.poll_lock() {
            *f = Some((at, direction));
        }
    }

    fn get(&self) -> Option<(Vec3, Vec3)> {
        self.focus.lock().ok().and_then(|f| *f)
    }

    /// Forget what `owner` uses (a parked car's models: [`PARKED_OWNER`] | its serial), as for an unloaded unit.
    pub fn release(&self, owner: u32) {
        let _ = self.releases.send(owner);
    }

    fn take_releases(&self) -> Vec<u32> {
        self.released.lock().map(|r| r.try_iter().collect()).unwrap_or_default()
    }
}

/// Owner ids of jack spots' parked cars (`jackspots.rs`): this bit and the request's serial. Track units are
/// small numbers.
pub const PARKED_OWNER: u32 = 1 << 31;

/// Which `InstanceList` backdrop entries to draw (see `mw_assets::backdrop`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BackdropMode {
    /// As the game: every backdrop model whole, shown while the camera's zone lists it, and each unit's main
    /// placements while the unit is that zone or a neighbour (`zone_view.rs`).
    #[default]
    CameraZone,
    /// One backdrop per place: a backdrop triangle is drawn where the zone owning its position lists its
    /// model, i.e. what the game shows there when the player stands in that zone; every unit's main
    /// placements always (`--composed-backdrop`).
    Composed,
    /// Every unit's backdrop, unfiltered (all LOD tiers on top of each other; debug).
    All,
    /// Only this track unit's backdrop, as the game draws it while the player is in that unit (debug).
    Zone(u32),
    /// No backdrop (debug).
    None,
}

/// Most vertices in one merged static mesh (`Worker::send_merged`).
const MERGE_VERTICES: usize = 1 << 18;

/// Debug filters for world placements.
#[derive(Debug, Clone, Default)]
pub struct WorldFilter {
    /// Which backdrop entries to draw.
    pub backdrop: BackdropMode,
    /// Skip groundcover (grass and plant sprites).
    pub no_groundcover: bool,
    /// Skip these models entirely.
    pub exclude: HashSet<ResourceId>,
    /// Keep backdrop LOD2 stand-ins in the `All` / `Zone` backdrop modes.
    pub keep_lod_proxies: bool,
    /// Which of the car's `VehiclePaint` options to paint it with.
    pub paint_index: usize,
    /// Draw props, compounds and dynamic objects without collision (`--no-prop-physics`).
    pub no_prop_physics: bool,
    /// Draw every static world placement as its own entity (the default: without `--merge-static`)
    /// instead of merging a track unit's single-use static placements per material.
    pub no_merge: bool,
    /// `--load-all`: load every track unit once and keep it (the behaviour before streaming), instead of
    /// streaming the focus zone and its neighbours (`streaming.rs`). `--units N` also loads once (N units).
    pub load_all: bool,
}

#[derive(Resource)]
pub struct LoaderChannel(pub std::sync::Mutex<Receiver<Msg>>);

pub fn spawn(
    store: Arc<AssetStore>,
    request: Request,
    bc_supported: bool,
    filter: WorldFilter,
) -> (LoaderChannel, LoaderRequests, StreamFocus) {
    let (tx, rx) = channel();
    let (req_tx, req_rx) = channel();
    let focus = StreamFocus::default();
    let worker_focus = focus.clone();
    crate::platform::thread::Builder::new()
        .name("mw-loader".into())
        .spawn(move || {
            let mut w = Worker {
                store,
                tx,
                bc_supported,
                models: HashSet::new(),
                parts: HashMap::new(),
                merged: 0,
                textures: HashSet::new(),
                failed_textures: HashSet::new(),
                filter,
                empty_models: HashSet::new(),
                shadow_ranges: HashMap::new(),
                groundcover_done: HashSet::new(),
                paint: None,
                paint_count: 0,
                paintable: HashMap::new(),
                requests: req_rx,
                skin_bones: None,
                unskinned: false,
                opponent: None,
                prop_tag: None,
                backdrop: None,
                backdrop_sent: HashSet::new(),
                prop_physics: HashMap::new(),
                lod_tables: HashMap::new(),
                standing_models: HashMap::new(),
                lods: crate::lod::LodSettings::from_env().enabled,
                main_view: HashMap::new(),
                pass_masks: HashMap::new(),
                all_passes: std::env::var_os("MW_ALL_PASSES").is_some(),
                standin_shadows: std::env::var("MW_STANDIN_SHADOWS").is_ok_and(|v| v != "0"),
                owner: None,
                refs: HashMap::new(),
                global: UnitRefs::default(),
                model_textures: HashMap::new(),
                gc_models: HashMap::new(),
                index: None,
                stream: None,
                focus: worker_focus,
                pending_release: HashSet::new(),
            };
            if let Err(e) = w.run(request) {
                let _ = w.tx.send(Msg::Progress(format!("loader error: {e:#}")));
            }
            let _ = w.tx.send(Msg::Done);
            // Then serve runtime car requests until the main thread drops its sender and, while streaming, keep
            // the stream set around the focus loaded (one unit at a time, car requests in between).
            loop {
                if w.process_releases().is_err() {
                    break;
                }
                match w.stream_step() {
                    Ok(true) => {
                        if w.poll_requests().is_err() {
                            break;
                        }
                        continue;
                    }
                    Ok(false) => {}
                    Err(e) => {
                        let _ = w.tx.send(Msg::Progress(format!("streaming error: {e:#}")));
                        break;
                    }
                }
                match crate::platform::recv_timeout(&w.requests, std::time::Duration::from_millis(STREAM_POLL_MS)) {
                    Ok(req) => {
                        if w.car_request(req).is_err() {
                            break;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .expect("spawn loader thread");
    (LoaderChannel(std::sync::Mutex::new(rx)), LoaderRequests(req_tx), focus)
}

/// How often the idle loader looks at the focus (ms). The game rebuilds its stream list every frame
/// (`FUN_00558740`); this only bounds how late a zone change is seen.
const STREAM_POLL_MS: u64 = 20;

/// A shared backdrop placement: its model and transform bits.
type BackdropKey = (ResourceId, [u32; 16]);

fn backdrop_key(model: ResourceId, transform: &[[f32; 4]; 4]) -> BackdropKey {
    (model, std::array::from_fn(|i| transform[i / 4][i % 4].to_bits()))
}

/// The key the main thread knows a shared backdrop placement by ([`Msg::StreamKey`]).
fn backdrop_hash(key: &BackdropKey) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    h.finish()
}

/// What a track unit (or the global set: cars, test props) uses of what the loader has sent.
#[derive(Default)]
struct UnitRefs {
    models: HashSet<ResourceId>,
    textures: HashSet<TexKey>,
    backdrops: HashSet<BackdropKey>,
    groundcover: HashSet<ResourceId>,
}

/// Every track unit's placements, read once (`Worker::index_world`).
struct WorldIndex {
    /// Bundle name and placements per track unit number.
    units: HashMap<u32, (String, Vec<mw_assets::Placement>)>,
    /// Units nearest-first from the first focus (the load-once modes).
    order: Vec<u32>,
    zones: Option<mw_formats::ZoneList>,
    /// The units listing each backdrop placement ([`BackdropMode::CameraZone`]).
    listing: HashMap<BackdropKey, Vec<u32>>,
    camera_zone: bool,
}

/// The streamer's state (`streaming.rs`): the units loaded now and the zone of the last focus inside one.
struct StreamState {
    resident: HashSet<u32>,
    last_zone: Option<usize>,
    /// The focus until the main thread sends one (the spawn point or the `--camera` pose).
    initial: Vec3,
}

struct Worker {
    store: Arc<AssetStore>,
    tx: Sender<Msg>,
    bc_supported: bool,
    /// Models sent to the main thread.
    models: HashSet<ResourceId>,
    /// Decoded models not (yet) sent: the current track unit's merge candidates.
    parts: HashMap<ResourceId, Vec<Part>>,
    /// Merged meshes sent so far (their synthetic model keys count up).
    merged: u64,
    textures: HashSet<TexKey>,
    failed_textures: HashSet<TexKey>,
    filter: WorldFilter,
    /// Models (or stripped variants) with nothing drawable.
    empty_models: HashSet<ResourceId>,
    /// Model shadow distances by variant key, sent with the model (`Msg::ShadowRange`).
    shadow_ranges: HashMap<ResourceId, f32>,
    /// Groundcover collections already sent (they are shared between track units).
    groundcover_done: HashSet<ResourceId>,
    /// Paint of the car being loaded, applied to its body-paint meshes, and how many the car has.
    paint: Option<mw_assets::paint::VehiclePaint>,
    paint_count: usize,
    /// Models known to have body-paint meshes (their key includes the paint) or not.
    paintable: HashMap<ResourceId, bool>,
    /// Runtime car requests, served between track units and after the initial request.
    requests: Receiver<CarRequest>,
    /// While loading a drivable car's body: its skeleton size, so meshes get joint attributes.
    skin_bones: Option<usize>,
    /// While loading a jack spot's parked car (`jackspots.rs`): a drivable car's model drawn without its damage
    /// skeleton, so its meshes are their own variant (no joint attributes), apart from the drivable car's.
    unskinned: bool,
    /// The opponent slot of the car being loaded (`CarRequest::opponent`).
    opponent: Option<usize>,
    /// The tag of the runtime placement being sent ([`CarRequest::prop`]).
    prop_tag: Option<u32>,
    /// Backdrop composition for [`BackdropMode::Composed`].
    backdrop: Option<BackdropSelector>,
    /// Backdrop instances already sent (the same backdrop model is listed by many units).
    backdrop_sent: HashSet<(ResourceId, [u32; 16])>,
    /// Physics per object definition (shared by its placements).
    prop_physics: HashMap<(ResourceId, mw_assets::props::ObjectKind), Option<Arc<crate::props::PropPhysics>>>,
    /// LOD table per model (`lod.rs`), `None` for models without one.
    lod_tables: HashMap<ResourceId, Option<Arc<crate::lod::LodTable>>>,
    /// Models' `Model` +0x19 pass masks ([`Self::pass_mask`]; 0 with `MW_ALL_PASSES=1`).
    main_view: HashMap<ResourceId, u8>,
    /// Pass masks by variant key, sent with the model (`Msg::PassMask`, `pass_mask.rs`).
    pass_masks: HashMap<ResourceId, u8>,
    /// `MW_ALL_PASSES=1`: also draw models the main view skips (A/B).
    all_passes: bool,
    /// `MW_STANDIN_SHADOWS=1`: the ground stand-ins' SHADOW states cast shadows (`pass_mask.rs`).
    standin_shadows: bool,
    /// Per compound definition: the models it draws while standing (see [`Self::compound_standing`]).
    standing_models: HashMap<ResourceId, StandingModels>,
    /// Apply the game's LOD rule to world placements (off with `MW_NO_LOD=1`).
    lods: bool,
    /// The track unit being sent (`None`: cars, test props; never unloaded).
    owner: Option<u32>,
    /// What each resident track unit uses, and what the rest uses.
    refs: HashMap<u32, UnitRefs>,
    global: UnitRefs,
    /// Textures per model key sent (to keep a shared model's textures with it).
    model_textures: HashMap<ResourceId, Vec<TexKey>>,
    /// Model keys per groundcover collection sent.
    gc_models: HashMap<ResourceId, Vec<ResourceId>>,
    index: Option<WorldIndex>,
    /// Set while the world is streamed around the focus.
    stream: Option<StreamState>,
    focus: StreamFocus,
    /// Owners released before their request was served (released right after it).
    pending_release: HashSet<u32>,
}

/// The models a compound definition draws while it stands (`Loader::compound_standing`): `CompoundObject`
/// +0x24 and +0x28.
#[derive(Clone, Copy, Default)]
struct StandingModels {
    advert: Option<ResourceId>,
    found: Option<ResourceId>,
}

/// How a model is turned into drawable parts.
#[derive(Clone, Copy, Default)]
struct Variant {
    /// Drop meshes whose shader is a LOD2 stand-in.
    strip_lod2: bool,
    /// Keep only the backdrop triangles the zone composition draws, for a placement with this transform.
    compose_backdrop: Option<[[f32; 4]; 4]>,
    /// Keep only the triangles skinned to these skeleton bones (bit mask): one part of a compound object.
    bones: Option<u64>,
    /// Which LOD state's renderable (0 = LOD0, `lod.rs`).
    lod: u8,
}

impl Worker {
    fn send(&self, m: Msg) -> anyhow::Result<()> {
        self.tx.send(m).map_err(|_| anyhow::anyhow!("main thread gone"))
    }

    fn run(&mut self, request: Request) -> anyhow::Result<()> {
        match request {
            Request::Vehicle { bundle } => {
                let b = self.store.bundle(&bundle).ok_or_else(|| anyhow::anyhow!("bundle not found: {bundle}"))?;
                self.select_paint(&b);
                self.send(Msg::Focus(Vec3::ZERO))?;
                let placements = match self.store.vehicle_placements(&b)? {
                    Some(p) => p,
                    None => b
                        .entries_of_type(ResourceType::Model)
                        .map(|e| mw_assets::Placement {
                            model: e.id,
                            transform: mw_assets::IDENTITY,
                            backdrop: false,
                            overrides: None,
                            object: None,
                            lod_scale: 1.0,
                        })
                        .collect(),
                };
                for p in placements {
                    if let Some(model) = self.model_variant(p.model, Variant::default(), InstanceLook::default())? {
                        self.send(Msg::Instance { model, transform: game_matrix(&p.transform), lod: None })?;
                    }
                }
                self.send(Msg::Progress(format!("loaded {bundle}")))?;
            }
            Request::World { max_units, focus } => {
                self.surfaces()?;
                self.world(max_units, focus)?
            }
            Request::Drive { bundle, spawn, max_units, world, props, .. } => {
                // The surface list first: the tyres read it from the car's first physics tick.
                self.surfaces()?;
                self.car(&bundle)?;
                for p in props {
                    self.test_prop(&p)?;
                }
                if world {
                    self.world(max_units, Some(spawn))?;
                }
            }
        }
        Ok(())
    }

    /// Load a car asked for at runtime. Load errors go back to the main thread; only a closed
    /// channel is an error here.
    fn car_request(&mut self, req: CarRequest) -> anyhow::Result<()> {
        // Cars are kept (never unloaded with a unit).
        let owner = self.owner.take();
        let r = self.car_request_global(req);
        self.owner = owner;
        r
    }

    fn car_request_global(&mut self, req: CarRequest) -> anyhow::Result<()> {
        if let Some((prop, tag)) = &req.prop {
            self.prop_tag = Some(*tag);
            let placed = self.test_prop(prop);
            self.prop_tag = None;
            if let Err(e) = placed {
                self.send(Msg::Progress(format!("runtime object {}: {e:#}", prop.object)))?;
            }
            return Ok(());
        }
        if let Some(t) = &req.traffic {
            // A jack spot's parked car is its own owner, released when the car goes (`jackspots.rs`).
            let parked =
                t.vehicle.checked_sub(crate::jackspots::MODEL_KEY_BASE).map(|serial| PARKED_OWNER | serial as u32);
            if let Some(o) = parked {
                self.refs.entry(o).or_default();
            }
            self.owner = parked;
            let loaded = self.traffic_car(&req.bundle, t);
            self.owner = None;
            if let Err(e) = loaded {
                self.send(Msg::Progress(format!("traffic car {}: {e:#}", req.bundle)))?;
            }
            if let Some(o) = parked
                && self.pending_release.remove(&o)
            {
                self.unload_unit(o)?;
            }
            return Ok(());
        }
        self.filter.paint_index = req.paint_index;
        self.opponent = req.opponent;
        let loaded = self.car(&req.bundle);
        self.opponent = None;
        if let Err(e) = loaded {
            self.send(Msg::CarFailed(format!("{}: {e:#}", req.bundle)))?;
        }
        Ok(())
    }

    /// Forget what released owners used (`StreamFocus::release`).
    fn process_releases(&mut self) -> anyhow::Result<()> {
        for owner in self.focus.take_releases() {
            if self.refs.contains_key(&owner) {
                self.unload_unit(owner)?;
            } else {
                self.pending_release.insert(owner);
            }
        }
        Ok(())
    }

    /// Serve any runtime car requests that are waiting (called between track units).
    fn poll_requests(&mut self) -> anyhow::Result<()> {
        self.process_releases()?;
        loop {
            match self.requests.try_recv() {
                Ok(req) => self.car_request(req)?,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return Ok(()),
            }
        }
    }

    fn select_paint(&mut self, b: &mw_bundle::Bundle) -> Vec<mw_assets::paint::VehiclePaint> {
        let paints = self.store.vehicle_paints(b).unwrap_or_default();
        self.paint_count = paints.len();
        self.paint = paints.get(self.filter.paint_index.min(paints.len().saturating_sub(1))).copied();
        if !paints.is_empty() {
            let _ = self.send(Msg::Progress(format!(
                "paint {} of {} (--paint-index)",
                self.filter.paint_index.min(paints.len() - 1),
                paints.len()
            )));
        }
        paints
    }

    /// A `NitrousParameters` variant's rules (`mw_handling::nitrous`).
    fn nitrous_rules(&self, id: &str) -> Option<mw_handling::nitrous::NitrousRules> {
        let f = mw_assets::mods::nitrous_parameters(&self.store, id)?;
        Some(mw_handling::nitrous::NitrousRules::from_fields(|b, k| f.get(&(b, k)).copied()))
    }

    fn car(&mut self, bundle: &str) -> anyhow::Result<()> {
        let b = self.store.bundle(bundle).ok_or_else(|| anyhow::anyhow!("bundle not found: {bundle}"))?;
        let paints = Arc::new(self.select_paint(&b));
        let e = b
            .entries_of_type(ResourceType::VehicleGraphicsSpec)
            .next()
            .ok_or_else(|| anyhow::anyhow!("{bundle} has no VehicleGraphicsSpec"))?;
        let mut spec = mw_formats::VehicleGraphicsSpec::parse(&b.load(e)?)?;
        let handling = mw_assets::handling::VehicleHandling::from_bundle(&self.store, &b).unwrap_or_default();
        // The wheels go where the game's vehicle build puts its wheel bodies (`mw_assets::wheel_layout`,
        // `FUN_009b40c0`), already in `handling.wheels`; the spec's positions are placeholders on many cars.
        let positions: Vec<[f32; 3]> = handling.wheels.iter().map(|w| w.position).collect();
        mw_assets::wheel_layout::place_spec_wheels(&mut spec, &positions);
        // An opponent plays its competitor engine (`car_audio.rs`); the player's bank is not built for it.
        let competitor_engine = self
            .opponent
            .and_then(|_| bundle_vehicle_id(bundle))
            .and_then(|id| self.store.competitor_engine(id).ok().flatten())
            .map(Arc::new);
        let engine = if self.opponent.is_some() { None } else { self.store.engine(&b).unwrap_or_default() };
        let spot_effects = self.store.vehicle_spot_effects(&b, &spec).unwrap_or_default();
        let flash_lights = Some(self.store.car_flash_lights(&b, &spec)).filter(|l| !l.is_empty()).map(Arc::new);
        let coronas = Arc::new(self.store.car_coronas(&b, &spec));
        let damage = match self.store.vehicle_damage(&b) {
            Ok(d) => d.map(Arc::new),
            Err(e) => {
                self.send(Msg::Progress(format!("no damage model: {e}")))?;
                None
            }
        };
        let impact_graph = self.store.impact_graph();
        self.skin_bones = damage.as_ref().map(|d| d.skeleton.bones.len());
        // Performance mods (`mods_menu.rs`): the catalogue, the nitrous variants' burn rates, the slot names.
        let mods = mw_assets::mods::performance_upgrades(&self.store, &b).unwrap_or_default();
        let lang = self.store.language(mw_assets::DEFAULT_LANGUAGE).ok();
        let mod_catalogue = crate::mods_menu::ModCatalogue {
            stock: handling.clone(),
            nitrous: mods
                .iter()
                .flatten()
                .filter_map(|c| c.nitrous.clone())
                .filter_map(|id| Some((id.clone(), self.nitrous_rules(&id)?)))
                .collect(),
            default_nitrous: mw_assets::mods::default_nitrous(&self.store, &b)
                .and_then(|(_offline, online)| self.nitrous_rules(&online))
                .unwrap_or_default(),
            names: mods
                .iter()
                .map(|cat| {
                    cat.iter()
                        .map(|c| {
                            let name = c.name.and_then(|id| lang.as_ref()?.get(id as u32)).unwrap_or_default();
                            if name.is_empty() { c.kind.clone() } else { name.to_string() }
                        })
                        .collect()
                })
                .collect(),
            mods,
        };
        let mut car = CarSpec {
            mod_catalogue,
            bundle: bundle.to_string(),
            paint_index: self.filter.paint_index.min(self.paint_count.saturating_sub(1)),
            paint_count: self.paint_count,
            paints,
            body: Vec::new(),
            wheels: Vec::new(),
            handling,
            engine,
            competitor_engine,
            damage,
            impact_graph,
            spot_effects,
            flash_lights,
            coronas,
            light_sources: Arc::new(self.store.car_light_sources(&b, &spec)),
            opponent: self.opponent,
            top_speed_mph: bundle_vehicle_id(bundle)
                .and_then(|id| self.store.vehicle_entry(id).ok().flatten())
                .map_or(0.0, |v| v.top_speed),
            scratch_scale: self.store.scratch_scale(&b).unwrap_or(10.0),
        };
        // Parts are listed by the key the main thread knows them by (paint variants differ).
        for &m in spec.parts.iter().flatten() {
            if let Some(key) = self.model_variant(m, Variant::default(), InstanceLook::default())? {
                car.body.push(key);
            }
        }
        self.skin_bones = None;
        // Classify wheels by position, not name: every car faces +Z, but some
        // specs label their axles or sides the other way round (the Veyron's
        // "Front" wheels are at the back).
        let mid_z = spec.wheels.iter().map(|w| w.position[2]).sum::<f32>() / spec.wheels.len().max(1) as f32;
        for w in &spec.wheels {
            let mut parts = Vec::new();
            for p in &w.parts {
                if let Some(m) = p.model
                    && let Some(key) = self.model_variant(m, Variant::default(), InstanceLook::default())?
                {
                    parts.push((key, p.spins));
                }
            }
            let [x, y, z, qw] = [w.rotation[0], w.rotation[1], w.rotation[2], w.rotation[3]];
            car.wheels.push(WheelSpec {
                position: Vec3::from(w.position),
                rotation: Quat::from_xyzw(x, y, z, qw),
                mirror: w.position[0] < 0.0,
                front: w.position[2] > mid_z,
                parts,
            });
        }
        self.send(Msg::Car(Box::new(car)))?;
        self.store.evict_all();
        Ok(())
    }

    /// A traffic car's body and wheel models in the given paint (no handling, sound or damage: the lane
    /// follower is kinematic, `traffic.rs`).
    fn traffic_car(&mut self, bundle: &str, req: &TrafficModelRequest) -> anyhow::Result<()> {
        let b = self.store.bundle(bundle).ok_or_else(|| anyhow::anyhow!("bundle not found: {bundle}"))?;
        let e = b
            .entries_of_type(ResourceType::VehicleGraphicsSpec)
            .next()
            .ok_or_else(|| anyhow::anyhow!("{bundle} has no VehicleGraphicsSpec"))?;
        let mut spec = mw_formats::VehicleGraphicsSpec::parse(&b.load(e)?)?;
        // The kinematic traffic pose (`FUN_005ee7f0`): the axles' positions with the front-left radius, in the
        // frame `VehicleInfo.509931cd`, whose translation lifts the model above the road (`wheel_layout`).
        let layout = self.store.vehicle_wheel_layout(&b);
        if let Some(l) = &layout {
            mw_assets::wheel_layout::place_spec_wheels(&mut spec, &l.traffic_positions());
        }
        let ride_height = layout.as_ref().map_or(0.0, |l| l.traffic_ride_height());
        let coronas = Arc::new(self.store.car_coronas(&b, &spec));
        let saved = (self.paint, self.paint_count);
        self.paint = req.colour;
        self.unskinned = req.vehicle >= crate::jackspots::MODEL_KEY_BASE;
        let mut body = Vec::new();
        for &m in spec.parts.iter().flatten() {
            if let Some(key) = self.model_variant(m, Variant::default(), InstanceLook::default())? {
                body.push(key);
            }
        }
        let mid_z = spec.wheels.iter().map(|w| w.position[2]).sum::<f32>() / spec.wheels.len().max(1) as f32;
        let mut wheels = Vec::new();
        for w in &spec.wheels {
            let mut parts = Vec::new();
            for p in &w.parts {
                if let Some(m) = p.model
                    && let Some(key) = self.model_variant(m, Variant::default(), InstanceLook::default())?
                {
                    parts.push((key, p.spins));
                }
            }
            let [x, y, z, qw] = [w.rotation[0], w.rotation[1], w.rotation[2], w.rotation[3]];
            wheels.push(WheelSpec {
                position: Vec3::from(w.position),
                rotation: Quat::from_xyzw(x, y, z, qw),
                mirror: w.position[0] < 0.0,
                front: w.position[2] > mid_z,
                parts,
            });
        }
        (self.paint, self.paint_count) = saved;
        self.unskinned = false;
        self.send(Msg::TrafficCar(Box::new(TrafficCarSpec {
            vehicle: req.vehicle,
            paint: req.paint,
            body,
            wheels,
            ride_height,
            coronas,
        })))
    }

    /// Static collision for a track unit, built off the main thread.
    fn collision(&self, bundle: &mw_bundle::Bundle) -> anyhow::Result<()> {
        use avian3d::prelude::{Collider, TrimeshFlags};
        for e in bundle.entries_of_type(ResourceType::PolygonSoupList) {
            let list = mw_formats::PolygonSoupList::parse(&bundle.load(e)?)?;
            let (verts, tris) = list.triangles();
            if tris.is_empty() {
                continue;
            }
            let tags = Arc::new(list.triangle_tags());
            let verts = verts.into_iter().map(Vec3::from).collect();
            let collider = Collider::trimesh_with_config(verts, tris, TrimeshFlags::FIX_INTERNAL_EDGES);
            self.send(Msg::Collider(collider, tags))?;
        }
        Ok(())
    }

    fn surfaces(&self) -> anyhow::Result<()> {
        match self.store.surfaces() {
            Ok(list) => self.send(Msg::Surfaces(list))?,
            Err(e) => self.send(Msg::Progress(format!("no surface list: {e}")))?,
        }
        match self.store.dynamic_responses() {
            Ok(r) => self.send(Msg::PropResponses(r)),
            Err(e) => self.send(Msg::Progress(format!("no collision responses (using the data's values): {e}"))),
        }
    }

    /// Physics of an object definition, decoded once per definition.
    fn prop_physics(
        &mut self,
        object: ResourceId,
        kind: mw_assets::props::ObjectKind,
    ) -> Option<Arc<crate::props::PropPhysics>> {
        if self.filter.no_prop_physics {
            return None;
        }
        self.prop_physics
            .entry((object, kind))
            .or_insert_with(|| {
                let p = self.store.object_physics(object, kind).ok()?;
                crate::props::PropPhysics::new(&p).map(Arc::new)
            })
            .clone()
    }

    /// The models a compound definition draws while it stands (`FUN_005396d0` at stream-in, `FUN_00533a00`
    /// later; the draw passes `FUN_00538b00` / `FUN_00550020` draw the object's +0x10): with the object's flag
    /// +0x10f clear, `CompoundObject` +0x24 (`advert`), with it set +0x28 (`found`), either falling back to
    /// +0x04 when empty. +0x10f is whether the instance's id (= its trigger id) is in the save's list of found
    /// billboards (`FUN_00554230`'s last argument, a search of the list `FUN_006a0400` returns; that list is
    /// built from the profile's `Collect3` "TriggerID" table, billboard kinds only). Once broken (+0x10d,
    /// `FUN_00532430`) the object draws +0x04, the model skinned to the parts.
    fn compound_standing(&mut self, object: ResourceId) -> StandingModels {
        let store = &self.store;
        *self.standing_models.entry(object).or_insert_with(|| {
            let Ok(def) = store.object_def(object, mw_assets::props::ObjectKind::Compound) else {
                return StandingModels::default();
            };
            let slot = |i: usize| def.models.get(i).copied().flatten();
            StandingModels { advert: slot(1), found: slot(2) }
        })
    }

    /// Send a placement: with physics when it is a world object that has some. A compound object's model is
    /// also sent split by skeleton bone, one piece per part, for when it breaks up (`props.rs`), and its
    /// standing models ([`Self::compound_standing`]) as what it draws until then.
    fn send_placement(
        &mut self,
        model: ResourceId,
        p: &mw_assets::Placement,
        variant: Variant,
        look: InstanceLook,
    ) -> anyhow::Result<()> {
        let transform = game_matrix(&p.transform);
        let physics = p.object.and_then(|(object, kind)| self.prop_physics(object, kind));
        let standing = match p.object {
            Some((object, mw_assets::props::ObjectKind::Compound)) => self.compound_standing(object),
            _ => StandingModels::default(),
        };
        // A model of its own and its LODs, for a standing model other than the pieces' (`p.model`, +0x04).
        let mut standing_model = |m: Option<ResourceId>| -> anyhow::Result<Option<(ResourceId, _)>> {
            let Some(m) = m.filter(|&m| m != p.model) else { return Ok(None) };
            let Some(key) = self.model_variant(m, variant, look)? else { return Ok(None) };
            let at = mw_assets::Placement { model: m, ..p.clone() };
            Ok(Some((key, self.placement_lods(&at, key, variant, look, true)?)))
        };
        let advert = standing_model(standing.advert)?;
        let found = standing_model(standing.found)?;
        // Compounds that can break and stand as their pieces draw LOD0's at every LOD (only the draw distance
        // applies).
        let breakable = physics.as_ref().is_some_and(|ph| ph.compound.is_some());
        let (model, lod, standing) = match advert {
            Some((key, lod)) => (key, lod, true),
            _ => (model, self.placement_lods(p, model, variant, look, !breakable)?, false),
        };
        let Some(physics) = physics else {
            return self.send(Msg::Instance { model, transform, lod });
        };
        let found = found.filter(|_| standing);
        let pieces = match &physics.compound {
            Some(c) => {
                let owned = c.parts.iter().filter(|p| p.bone_index < 64).fold(0u64, |m, p| m | 1 << p.bone_index);
                let mut parts = Vec::with_capacity(c.parts.len());
                for part in &c.parts {
                    let bones = (part.bone_index < 64).then(|| 1u64 << part.bone_index);
                    parts.push(match bones {
                        Some(mask) => self.model_variant(p.model, Variant { bones: Some(mask), ..variant }, look)?,
                        None => None,
                    });
                }
                let rest = self.model_variant(p.model, Variant { bones: Some(!owned), ..variant }, look)?;
                Some((parts, rest))
            }
            None => None,
        };
        self.send(Msg::Prop { model, transform, physics, pieces, lod, standing, found, tag: self.prop_tag })
    }

    /// A model's LOD table (`lod.rs`): its LOD distances, shadow distance and LOD0 bounding sphere.
    fn lod_table(&mut self, id: ResourceId) -> Option<Arc<crate::lod::LodTable>> {
        let store = &self.store;
        self.lod_tables
            .entry(id)
            .or_insert_with(|| {
                let model = store.model(id).ok()?;
                let sphere = store.renderable(model.renderable_for_state(0)?).ok()?.bounding_sphere;
                crate::lod::LodTable::from_model(&model, sphere).map(Arc::new)
            })
            .clone()
    }

    /// The LODs of a world placement (not backdrops): the model's table and, with `levels`, the model key
    /// drawn at each LOD (`lod0` = the key of the LOD0 variant already sent; other LODs are decoded the same
    /// way from their own renderables and sent).
    fn placement_lods(
        &mut self,
        p: &mw_assets::Placement,
        lod0: ResourceId,
        variant: Variant,
        look: InstanceLook,
        levels: bool,
    ) -> anyhow::Result<Option<crate::lod::LodPlacement>> {
        if !self.lods || p.backdrop || variant.compose_backdrop.is_some() || variant.bones.is_some() {
            return Ok(None);
        }
        let Some(table) = self.lod_table(p.model) else { return Ok(None) };
        let mut keys = Vec::new();
        if levels {
            let model = self.store.model(p.model)?;
            let mut prev = (model.renderable_for_state(0), Some(lod0));
            keys.push(Some(lod0));
            for l in 1..table.distances.len() {
                let r = model.renderable_for_state(l);
                if r.is_some() && r != prev.0 {
                    prev = (r, self.model_variant(p.model, Variant { lod: l as u8, ..variant }, look)?);
                }
                keys.push(prev.1);
            }
        }
        Ok(Some(crate::lod::LodPlacement { table, scale: p.lod_scale, levels: keys }))
    }

    /// Place one world object by hand (test track).
    fn test_prop(&mut self, t: &TestProp) -> anyhow::Result<()> {
        let def = self.store.object_def(t.object, t.kind)?;
        let model = def.model().ok_or_else(|| anyhow::anyhow!("object {} has no model", t.object))?;
        let rot = Mat4::from_rotation_y(t.yaw);
        let m = Mat4::from_translation(t.position) * rot;
        // Back to the game's row-major layout (rows = axes, row 3 = translation).
        let cols = m.to_cols_array_2d();
        let placement = mw_assets::Placement {
            model,
            transform: cols,
            backdrop: false,
            overrides: None,
            object: Some((t.object, t.kind)),
            lod_scale: 1.0,
        };
        if let Some(key) = self.model_variant(model, Variant::default(), InstanceLook::default())? {
            self.send_placement(key, &placement, Variant::default(), InstanceLook::default())?;
        }
        Ok(())
    }

    fn world(&mut self, max_units: Option<usize>, focus: Option<Vec3>) -> anyhow::Result<()> {
        let focus = self.index_world(focus)?;
        let Some(index) = &self.index else { return Ok(()) };
        // Streamed as the game does (`streaming.rs`) unless `--units N` or `--load-all` asks for a fixed set, or
        // there is no zone list to stream by.
        let streaming = max_units.is_none() && !self.filter.load_all && index.zones.is_some();
        let fixed: Vec<u32> = if streaming {
            index.units.keys().copied().collect()
        } else {
            index.order.iter().take(max_units.unwrap_or(usize::MAX)).copied().collect()
        };
        self.build_listing(&fixed);
        // The composition fills a zone's area from the nearest loaded zone's view: while streaming, as for the
        // full map (the composed cut is drawn only in the aerial view, `zone_view.rs`).
        if let Some(sel) = &mut self.backdrop {
            sel.set_loaded(fixed.iter().copied());
        }
        if streaming {
            self.send(Msg::Streaming)?;
            self.stream = Some(StreamState { resident: HashSet::new(), last_zone: None, initial: focus });
            // The first stream set, then `Done`.
            while self.stream_step()? {
                self.poll_requests()?;
            }
            let n = self.stream.as_ref().map_or(0, |s| s.resident.len());
            self.send(Msg::Progress(format!("first stream set loaded: {n} track units")))?;
            return Ok(());
        }
        let total = fixed.len();
        for (i, &unit) in fixed.iter().enumerate() {
            self.poll_requests()?;
            self.send(Msg::Progress(format!("streaming TRK_UNIT{unit} ({}/{total})", i + 1)))?;
            self.load_unit(unit)?;
        }
        self.send(Msg::Progress(format!("world loaded: {total} track units")))?;
        Ok(())
    }

    /// Read every unit's placements (cheap) to find the city centre and the units' order nearest-first from the
    /// focus; send the focus and the zone list. Returns the focus.
    fn index_world(&mut self, focus: Option<Vec3>) -> anyhow::Result<Vec3> {
        let names: Vec<String> = self
            .store
            .bundle_paths()
            .iter()
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")))
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let mut placed = Vec::with_capacity(names.len());
        for name in names {
            let Some(b) = self.store.bundle(&name) else { continue };
            let p = self.store.bundle_placements(&b)?;
            let centre = median_translation(&p);
            self.store.evict_all();
            match track_unit_number(&name) {
                Some(unit) => placed.push((unit, name, p, centre)),
                None => self.send(Msg::Progress(format!("{name}: no unit number, not loaded")))?,
            }
        }
        let mut centres: Vec<Vec3> = placed.iter().filter_map(|(_, _, _, c)| *c).collect();
        let focus = focus.or_else(|| median_vec(&mut centres)).unwrap_or(Vec3::ZERO);
        self.send(Msg::Focus(focus))?;
        let zones = self.store.zone_list().ok();
        if let Some(zones) = &zones {
            self.send(Msg::Zones(zones.clone()))?;
        }
        // Nearest zone first (the game streams by `ZoneList` zone: the zone holding the focus, then its
        // neighbours; docs/decomp.md "Front end"), units without a zone by their placements' median.
        let zone_distance = |unit: u32| -> Option<f32> {
            Some(zones.as_ref()?.zones.iter().find(|z| z.id == unit)?.distance(focus.x, focus.z))
        };
        placed.sort_by(|a, b| {
            let d = |unit: u32, c: &Option<Vec3>| {
                zone_distance(unit).unwrap_or_else(|| c.map_or(f32::MAX, |c| c.distance(focus)))
            };
            d(a.0, &a.3).total_cmp(&d(b.0, &b.3))
        });

        // Backdrop composition needs every unit's backdrop list, also of units not streamed.
        if matches!(self.filter.backdrop, BackdropMode::Composed | BackdropMode::CameraZone) {
            self.send(Msg::Progress("indexing backdrops".into()))?;
            let listings = placed.iter().flat_map(|(unit, _, placements, _)| {
                placements.iter().filter(|p| p.backdrop).map(move |p| (*unit, p.model, &p.transform))
            });
            match self.store.backdrop_selector(listings) {
                Ok(sel) => self.backdrop = Some(sel),
                Err(e) => {
                    self.send(Msg::Progress(format!("no zone list ({e}): drawing every backdrop")))?;
                }
            }
        }
        let camera_zone = self.filter.backdrop == BackdropMode::CameraZone && zones.is_some();
        let order = placed.iter().map(|(unit, ..)| *unit).collect();
        let units = placed.into_iter().map(|(unit, name, p, _)| (unit, (name, p))).collect();
        self.index = Some(WorldIndex { units, order, zones, listing: HashMap::new(), camera_zone });
        Ok(focus)
    }

    /// Camera-zone mode: the units of `units` listing each backdrop placement.
    fn build_listing(&mut self, units: &[u32]) {
        let Some(index) = &mut self.index else { return };
        if !index.camera_zone {
            return;
        }
        let mut listing: HashMap<BackdropKey, Vec<u32>> = HashMap::new();
        for unit in units {
            let Some((_, placements)) = index.units.get(unit) else { continue };
            for p in placements.iter().filter(|p| p.backdrop) {
                let units = listing.entry(backdrop_key(p.model, &p.transform)).or_default();
                if !units.contains(unit) {
                    units.push(*unit);
                }
            }
        }
        index.listing = listing;
    }

    /// One step of the streamer (`streaming.rs`, the game's `FUN_00549fb0` stream list and `FUN_0053acf0`
    /// streamer): build the stream list for the focus, unload every resident unit not on it, then load the
    /// missing unit of highest priority. True when a unit was loaded (call again).
    fn stream_step(&mut self) -> anyhow::Result<bool> {
        let (Some(state), Some(index)) = (&self.stream, &self.index) else { return Ok(false) };
        let Some(zones) = &index.zones else { return Ok(false) };
        let (pos, dir) = self.focus.get().unwrap_or((state.initial, Vec3::ZERO));
        let Some(list) = crate::streaming::stream_list(zones, pos, dir, state.last_zone) else { return Ok(false) };
        // Evict first: the game's streamer unloads every slot not requested before it picks one to load
        // (`FUN_00537c80`).
        let gone: Vec<u32> =
            state.resident.iter().filter(|u| !list.units.iter().any(|(w, _)| w == *u)).copied().collect();
        let next = list
            .units
            .iter()
            .filter(|(u, _)| !state.resident.contains(u) && index.units.contains_key(u))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|&(u, _)| u);
        let focus_unit = zones.zones.get(list.zone).map_or(0, |z| z.id);
        if let Some(state) = &mut self.stream {
            state.last_zone = Some(list.zone);
        }
        for unit in gone {
            if let Some(state) = &mut self.stream {
                state.resident.remove(&unit);
            }
            self.unload_unit(unit)?;
        }
        let Some(unit) = next else { return Ok(false) };
        if let Some(state) = &mut self.stream {
            state.resident.insert(unit);
        }
        let t = crate::platform::Instant::now();
        self.load_unit(unit)?;
        self.send(Msg::Progress(format!(
            "streamed TRK_UNIT{unit} in {:.0} ms (focus zone {focus_unit})",
            t.elapsed().as_secs_f32() * 1000.0,
        )))?;
        Ok(true)
    }

    /// Send one track unit: its collision, lights, groundcover and placements, owned by the unit.
    fn load_unit(&mut self, unit: u32) -> anyhow::Result<()> {
        let Some(index) = &mut self.index else { return Ok(()) };
        let Some((name, placements)) = index.units.remove(&unit) else { return Ok(()) };
        self.owner = Some(unit);
        self.refs.entry(unit).or_default();
        let sent = self.send(Msg::UnitBegin(unit)).and_then(|_| self.send_unit(unit, &name, &placements));
        self.owner = None;
        self.parts.clear();
        self.store.evict_all();
        if let Some(index) = &mut self.index {
            index.units.insert(unit, (name, placements));
        }
        sent?;
        self.send(Msg::UnitLoaded(unit))
    }

    /// Drop a track unit: the main thread despawns its entities, and the models, textures, shared backdrop
    /// instances and groundcover collections no other resident unit (nor a car) uses are forgotten here, so
    /// they are sent again when needed.
    fn unload_unit(&mut self, unit: u32) -> anyhow::Result<()> {
        let Some(refs) = self.refs.remove(&unit) else { return Ok(()) };
        let mut models = HashSet::new();
        let mut textures = HashSet::new();
        let mut backdrops = HashSet::new();
        let mut groundcover = HashSet::new();
        for r in self.refs.values().chain(std::iter::once(&self.global)) {
            models.extend(r.models.iter().copied());
            textures.extend(r.textures.iter().copied());
            backdrops.extend(r.backdrops.iter().copied());
            groundcover.extend(r.groundcover.iter().copied());
        }
        let mut out = Unloaded { unit, ..Default::default() };
        for k in refs.models.into_iter().filter(|k| !models.contains(k)) {
            self.model_textures.remove(&k);
            if self.models.remove(&k) {
                out.models.push(k);
            }
        }
        for t in refs.textures.into_iter().filter(|t| !textures.contains(t)) {
            if self.textures.remove(&t) {
                out.textures.push(t);
            }
        }
        for b in refs.backdrops.into_iter().filter(|b| !backdrops.contains(b)) {
            if self.backdrop_sent.remove(&b) {
                out.backdrops.push(backdrop_hash(&b));
            }
        }
        for c in refs.groundcover.into_iter().filter(|c| !groundcover.contains(c)) {
            if self.groundcover_done.remove(&c) {
                self.gc_models.remove(&c);
                out.groundcover.push(c);
            }
        }
        let line = format!(
            "unloaded TRK_UNIT{unit}: {} models, {} textures, {} backdrops, {} groundcover freed; {} models, {} \
             textures sent ({} / {} in use)",
            out.models.len(),
            out.textures.len(),
            out.backdrops.len(),
            out.groundcover.len(),
            self.models.len(),
            self.textures.len(),
            models.len(),
            textures.len()
        );
        self.send(Msg::UnitUnloaded(Box::new(out)))?;
        self.send(Msg::Progress(line))
    }

    /// The refs of the unit being sent, or the global ones.
    fn owner_refs(&mut self) -> &mut UnitRefs {
        match self.owner {
            Some(u) => self.refs.entry(u).or_default(),
            None => &mut self.global,
        }
    }

    /// The owner uses model `key` (and so its textures).
    fn note_model(&mut self, key: ResourceId) {
        let textures = self.model_textures.get(&key).cloned();
        let refs = self.owner_refs();
        if refs.models.insert(key)
            && let Some(t) = textures
        {
            refs.textures.extend(t);
        }
    }

    /// The owner uses groundcover collection `id` (and its group models, when sent).
    fn note_groundcover(&mut self, id: ResourceId) {
        self.owner_refs().groundcover.insert(id);
        for key in self.gc_models.get(&id).cloned().unwrap_or_default() {
            self.note_model(key);
        }
    }

    fn send_unit(&mut self, unit: u32, name: &str, placements: &[mw_assets::Placement]) -> anyhow::Result<()> {
        let camera_zone = self.index.as_ref().is_some_and(|i| i.camera_zone);
        if let Some(b) = self.store.bundle(name) {
            self.collision(&b)?;
            match self.store.bundle_lights(&b) {
                Ok(lights) if !lights.is_empty() => self.send(Msg::Lights(lights))?,
                Ok(_) => {}
                Err(e) => self.send(Msg::Progress(format!("no lights for {name}: {e}")))?,
            }
            if !self.filter.no_groundcover {
                self.groundcover(&b, Some(unit))?;
            }
        }
        let unit = Some(unit);
        let unit_tag = |u: Option<u32>| Msg::ZoneTag(u.filter(|_| camera_zone).map(crate::zone_view::ZoneTag::Unit));
        self.send(unit_tag(unit))?;
        let mut statics = Vec::new();
        // A shared backdrop instance's key is open ([`Msg::StreamKey`]) until the next placement.
        let mut key_open = false;
        for p in placements {
            if key_open {
                self.send(Msg::StreamKey(None))?;
                key_open = false;
            }
            if self.filter.exclude.contains(&p.model) {
                continue;
            }
            let mut variant = Variant::default();
            if p.backdrop {
                match self.filter.backdrop {
                    BackdropMode::None => continue,
                    BackdropMode::Zone(z) if unit != Some(z) => continue,
                    BackdropMode::Composed if self.backdrop.is_some() => {
                        // Many units list the same backdrop model: compose and send it once.
                        let key = backdrop_key(p.model, &p.transform);
                        self.owner_refs().backdrops.insert(key);
                        if !self.backdrop_sent.insert(key) {
                            continue;
                        }
                        self.send(Msg::StreamKey(Some(backdrop_hash(&key))))?;
                        key_open = true;
                        variant.compose_backdrop = Some(p.transform);
                    }
                    // Sent once, whole, shown while the camera's zone lists it.
                    BackdropMode::CameraZone if camera_zone => {
                        let key = backdrop_key(p.model, &p.transform);
                        self.owner_refs().backdrops.insert(key);
                        if !self.backdrop_sent.insert(key) {
                            continue;
                        }
                        self.send(Msg::StreamKey(Some(backdrop_hash(&key))))?;
                        key_open = true;
                        let units = self.index.as_ref().and_then(|i| i.listing.get(&key).cloned()).unwrap_or_default();
                        let look = InstanceLook::from_placement(p);
                        self.send(Msg::ZoneTag(Some(crate::zone_view::ZoneTag::Backdrop(units.into()))))?;
                        if let Some(model) = self.model_variant(p.model, variant, look)? {
                            self.send_placement(model, p, variant, look)?;
                        }
                        // The composed cut for a free camera high above the ground (`zone_view::AERIAL`).
                        if self.backdrop.is_some() {
                            let composed = Variant { compose_backdrop: Some(p.transform), ..variant };
                            self.send(Msg::ZoneTag(Some(crate::zone_view::ZoneTag::Composed)))?;
                            if let Some(model) = self.model_variant(p.model, composed, look)? {
                                self.send_placement(model, p, composed, look)?;
                            }
                        }
                        self.send(unit_tag(unit))?;
                        continue;
                    }
                    // Unfiltered backdrops mix needed world-space geometry (terrain, water, horizon)
                    // with LOD2 stand-ins for buildings whose detailed versions are also loaded.
                    _ => variant.strip_lod2 = !self.filter.keep_lod_proxies,
                }
            }
            // Static placements (no physics, not a backdrop) are merged per material below.
            let physics = p.object.and_then(|(object, kind)| self.prop_physics(object, kind));
            // Models whose pass mask excludes every view we render are not drawn; the others go only into
            // the views that draw them (`pass_mask.rs`). Placements with physics keep their model (model
            // decision: no collision-only body yet).
            let mask = self.pass_mask(p.model);
            if physics.is_none() && !crate::pass_mask::PassMask::drawn(mask) {
                continue;
            }
            // The coarse ground stand-ins (mask 0x04) are drawn in the shadow cascades and the planar
            // reflection only. The shadow pass draws a model's SHADOW state when it has one
            // (`FUN_008a02f0`): for these a `WorldShadow` mesh, lower than the tile (the tile itself would
            // shade the roads it cuts through), placed as a shadow-only instance under the draw distance.
            // Model decision: off unless `MW_STANDIN_SHADOWS=1` (those meshes still stand up to 3 m over
            // 3,100 m² of driven road, `mwtool terrain-check --only shadow`, and put whole carriageways in
            // shadow at a low sun; the shadow views' eye for the LOD / shadow-distance test is not traced).
            if physics.is_none() && mask & mw_formats::Model::PASS_MAIN != 0 {
                let look = InstanceLook::from_placement(p);
                let shadow = Variant { lod: mw_formats::Model::STATE_SHADOW as u8, ..variant };
                if self.standin_shadows
                    && mask & mw_formats::Model::PASS_SHADOW == 0
                    && let Some(key) = self.model_variant(p.model, shadow, look)?
                {
                    let lod = self.placement_lods(p, key, shadow, look, false)?;
                    self.send(Msg::Instance { model: key, transform: game_matrix(&p.transform), lod })?;
                }
                if mask & mw_formats::Model::PASS_PLANAR != 0 {
                    continue;
                }
            }
            if !self.filter.no_merge && !p.backdrop && physics.is_none() && mask == 0 {
                // A compound without physics only ever stands: its standing model (`compound_standing`).
                let model = match p.object {
                    Some((object, mw_assets::props::ObjectKind::Compound)) => {
                        self.compound_standing(object).advert.unwrap_or(p.model)
                    }
                    _ => p.model,
                };
                if let Some(model) = self.model_parts(model, variant, InstanceLook::from_placement(p), true)? {
                    statics.push((model, game_matrix(&p.transform)));
                }
                continue;
            }
            let look = InstanceLook::from_placement(p);
            if let Some(model) = self.model_variant(p.model, variant, look)? {
                self.send_placement(model, p, variant, look)?;
            }
        }
        if key_open {
            self.send(Msg::StreamKey(None))?;
        }
        self.send_merged(std::mem::take(&mut statics))?;
        self.send(Msg::ZoneTag(None))?;
        Ok(())
    }

    /// Whether the game's main view draws `model`: its pass mask (`Model` +0x19) has no main-view bit
    /// (`mw_formats::Model::PASS_MAIN`; the main view's query excludes `0x4000000`, docs/decomp.md "Model pass
    /// mask"). Always true with `MW_ALL_PASSES=1`.
    /// The bits of a model's pass mask that this port applies (`pass_mask.rs`); 0 with `MW_ALL_PASSES=1`.
    fn pass_mask(&mut self, model: ResourceId) -> u8 {
        if self.all_passes {
            return 0;
        }
        let store = &self.store;
        *self
            .main_view
            .entry(model)
            .or_insert_with(|| store.model(model).map_or(0, |m| m.pass_mask & crate::pass_mask::PassMask::USED))
    }

    /// Send a track unit's static placements: models placed once in the unit whose parts can all share a
    /// draw are merged per material (and vertex layout) into world-space meshes of at most
    /// [`MERGE_VERTICES`] vertices, drawn at the origin. Repeated models and models with a part that cannot
    /// be merged (blended, painted, an unknown attribute) are placed as before.
    fn send_merged(&mut self, statics: Vec<(ResourceId, Mat4)>) -> anyhow::Result<()> {
        let mergeable = |p: &Part| {
            matches!(p.look.blend, Blend::Opaque | Blend::Mask)
                && !p.look.paint
                && convert::merge_signature(&p.mesh).is_some()
        };
        // (material, attribute layout) -> the (model, part, world matrix) triples to merge.
        type Placed = (ResourceId, usize, Mat4);
        let mut groups: HashMap<(Look, u8), Vec<Placed>> = HashMap::new();
        let mut order = Vec::new();
        // Models placed more than once already share a draw (one bin per mesh and material): merging them
        // would only copy their vertices (the full map then ran out of GPU memory).
        let mut uses: HashMap<ResourceId, usize> = HashMap::new();
        for (key, _) in &statics {
            *uses.entry(*key).or_default() += 1;
        }
        for (key, transform) in statics {
            let all = uses[&key] == 1 && self.parts.get(&key).is_some_and(|parts| parts.iter().all(mergeable));
            if !all {
                self.send_model(key)?;
                self.send(Msg::Instance { model: key, transform, lod: None })?;
                continue;
            }
            for (i, part) in self.parts[&key].iter().enumerate() {
                let g = (part.look, convert::merge_signature(&part.mesh).unwrap_or(0));
                groups.entry(g).or_insert_with(|| {
                    order.push(g);
                    Vec::new()
                });
                groups.get_mut(&g).unwrap().push((key, i, transform));
            }
        }
        for g in order {
            let members = &groups[&g];
            let mut start = 0;
            while start < members.len() {
                let mut end = start;
                let mut verts = 0;
                while end < members.len() {
                    let (key, i, _) = members[end];
                    let n = self.parts[&key][i].mesh.count_vertices();
                    if end > start && verts + n > MERGE_VERTICES {
                        break;
                    }
                    verts += n;
                    end += 1;
                }
                let meshes: Vec<_> =
                    members[start..end].iter().map(|&(key, i, t)| (&self.parts[&key][i].mesh, t)).collect();
                let mesh = convert::merge(&meshes);
                self.merged += 1;
                let key = ResourceId(0x7000_0000_0000_0000 | self.merged);
                self.models.insert(key);
                self.model_textures.insert(key, g.0.all_textures().collect());
                self.note_model(key);
                self.send(Msg::Model(key, vec![Part { mesh, look: g.0 }]))?;
                self.send(Msg::Instance { model: key, transform: Mat4::IDENTITY, lod: None })?;
                start = end;
            }
        }
        Ok(())
    }

    /// Send the groundcover of every collection in a bundle that has not been sent yet: one merged
    /// mesh per collection and sprite class, drawn as an ordinary model at the origin.
    fn groundcover(&mut self, b: &mw_bundle::Bundle, unit: Option<u32>) -> anyhow::Result<()> {
        let listed: Vec<ResourceId> = b.entries_of_type(ResourceType::GroundcoverCollection).map(|e| e.id).collect();
        if let Some(unit) = unit {
            self.send(Msg::GroundcoverListing { unit, collections: listed.clone() })?;
        }
        for &id in &listed {
            self.note_groundcover(id);
        }
        let ids: Vec<ResourceId> = listed.into_iter().filter(|id| self.groundcover_done.insert(*id)).collect();
        for id in ids {
            let Ok(gc) = self.store.groundcover(id) else { continue };
            for (m, mat) in gc.materials.iter().enumerate() {
                let Some(tex) = mat.texture else { continue };
                // A stand-in scene mesh gives the usual alpha-tested, double-sided look.
                let sm = mw_assets::SceneMesh {
                    renderable: id,
                    mesh_index: m,
                    material: None,
                    shader_name: "Groundcover_alphatest_doublesided".into(),
                    diffuse: Some(tex),
                    normal_map: None,
                    textures: Vec::new(),
                    sampler_states: Vec::new(),
                    parameters: vec![
                        ("amount".into(), [mat.wind[0], 0.0, 0.0, 0.0]),
                        ("speed".into(), [mat.wind[1], 0.0, 0.0, 0.0]),
                    ],
                    mesh: mw_formats::DecodedMesh::default(),
                };
                let mut look = Look::from_scene_mesh(&sm);
                for key in look.all_textures().collect::<Vec<_>>() {
                    self.texture(key)?;
                }
                look.retain_textures(|k| self.textures.contains(k));
                // One model per group: the game culls whole groups by distance (`FUN_00eb9500`).
                let only_e: Option<u32> = std::env::var("MW_GC_E").ok().and_then(|v| v.parse().ok());
                for (gi, (g, range)) in gc
                    .groups
                    .iter()
                    .enumerate()
                    .filter(|(_, (g, _))| g.material == m && only_e.is_none_or(|e| e == g.shift))
                {
                    let mine = &gc.instances[range.clone()];
                    let Some(mesh) = groundcover_mesh(mine, &mat.regions) else { continue };
                    let bounds = groundcover_bounds(mine, mat.wind[0]);
                    let key = ResourceId(0x6000_0000_0000_0000 | (gi as u64) << 32 | id.0 & 0xFFFF_FFFF);
                    self.models.insert(key);
                    self.model_textures.insert(key, look.all_textures().collect());
                    self.gc_models.entry(id).or_default().push(key);
                    self.note_model(key);
                    self.send(Msg::Model(key, vec![Part { mesh, look }]))?;
                    self.send(Msg::Groundcover {
                        model: key,
                        collection: id,
                        min: Vec3::from(g.min),
                        max: Vec3::from(g.max),
                        range: g.radius * mw_formats::groundcover::DRAW_DISTANCE_SCALE,
                        bounds,
                    })?;
                }
            }
        }
        Ok(())
    }

    /// Load a model as a [`Variant`] (without its LOD2 stand-in meshes, or cut down to the composed
    /// backdrop) and with a placement's tint / AO. Returns the key the main thread knows it by (the
    /// stripped variant gets a synthetic ID with bit 62 set, per-instance variants a hash with bit 61 set,
    /// composed backdrops a hash with bit 63 set), or `None` if nothing is drawable.
    fn model_variant(
        &mut self,
        id: ResourceId,
        variant: Variant,
        inst: InstanceLook,
    ) -> anyhow::Result<Option<ResourceId>> {
        let key = self.model_parts(id, variant, inst, false)?;
        if let Some(key) = key {
            self.send_model(key)?;
        }
        Ok(key)
    }

    /// Send a decoded model to the main thread (once).
    fn send_model(&mut self, key: ResourceId) -> anyhow::Result<()> {
        if let Some(parts) = self.parts.remove(&key)
            && self.models.insert(key)
        {
            if let Some(&d) = self.shadow_ranges.get(&key) {
                self.send(Msg::ShadowRange(key, d))?;
            }
            if let Some(&m) = self.pass_masks.get(&key) {
                self.send(Msg::PassMask(key, m))?;
            }
            self.send(Msg::Model(key, parts))?;
        }
        Ok(())
    }

    /// Decode a model variant into [`Self::parts`] (see [`Self::model_variant`]) unless it is there or,
    /// without `need_parts`, already sent. The unit being sent (or the global set) then uses it.
    fn model_parts(
        &mut self,
        id: ResourceId,
        variant: Variant,
        inst: InstanceLook,
        need_parts: bool,
    ) -> anyhow::Result<Option<ResourceId>> {
        let key = self.model_parts_inner(id, variant, inst, need_parts)?;
        if let Some(key) = key {
            self.note_model(key);
        }
        Ok(key)
    }

    fn model_parts_inner(
        &mut self,
        id: ResourceId,
        variant: Variant,
        inst: InstanceLook,
        need_parts: bool,
    ) -> anyhow::Result<Option<ResourceId>> {
        use std::hash::{Hash, Hasher};
        let mut key = if variant.strip_lod2 { ResourceId(id.0 | 1 << 62) } else { id };
        if !inst.is_default() {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (key, inst).hash(&mut h);
            key = ResourceId(h.finish() & !(7 << 61) | 1 << 61);
        }
        if let Some(t) = &variant.compose_backdrop {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (key, t.as_flattened().iter().map(|v| v.to_bits()).collect::<Vec<_>>()).hash(&mut h);
            key = ResourceId(h.finish() | 1 << 63);
        }
        if let Some(mask) = variant.bones {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (key, mask).hash(&mut h);
            key = ResourceId(h.finish() & !(0xf << 59) | 1 << 59);
        }
        if variant.lod != 0 {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (key, "lod", variant.lod).hash(&mut h);
            key = ResourceId(h.finish() & ((1 << 58) - 1) | 1 << 58);
        }
        // Body-paint models are baked with the current paint, so each paint is its own variant
        // (bit 60, a hash of the ID and the paint's values).
        if self.paintable.get(&id) == Some(&true) {
            key = self.paint_key(key);
        }
        if self.unskinned {
            key = ResourceId(key.0 ^ (1 << 58));
        }
        if self.parts.contains_key(&key) || !need_parts && self.models.contains(&key) {
            return Ok(Some(key));
        }
        if self.empty_models.contains(&key) {
            return Ok(None);
        }
        if let Ok(m) = self.store.model(id)
            && m.shadow_distance.is_finite()
            && m.shadow_distance < 1e6
        {
            self.shadow_ranges.insert(key, m.shadow_distance);
        }
        let mut mask = self.pass_mask(id);
        // A stand-in's SHADOW state is its only shadow caster and is drawn in no other view; its LODs cast no
        // shadow (`pass_mask.rs`; one stand-in has no SHADOW state: no caster, model decision).
        if mask & mw_formats::Model::PASS_MAIN != 0 {
            if variant.lod as usize == mw_formats::Model::STATE_SHADOW {
                mask = mw_formats::Model::PASS_MAIN | mw_formats::Model::PASS_PLANAR;
            } else {
                mask |= mw_formats::Model::PASS_SHADOW;
            }
        }
        if mask != 0 {
            self.pass_masks.insert(key, mask);
        }
        let meshes = match variant.lod {
            0 => self.store.model_meshes(id, 0).map_err(anyhow::Error::from),
            l => self
                .store
                .model(id)
                .map_err(anyhow::Error::from)
                .and_then(|m| m.renderable_for_state(l as usize).ok_or_else(|| anyhow::anyhow!("no LOD {l}")))
                .and_then(|r| Ok(self.store.renderable_meshes(r)?)),
        };
        let meshes = match meshes {
            Ok((m, _errors)) => m,
            Err(_) => {
                self.empty_models.insert(key);
                return Ok(None);
            }
        };
        let mut parts = Vec::new();
        for mut sm in meshes {
            if variant.strip_lod2 && sm.shader_name.contains("LOD2") {
                continue;
            }
            if let (Some(t), Some(sel)) = (&variant.compose_backdrop, &self.backdrop) {
                match sel.filter_mesh(id, sm.mesh_index, &sm.mesh, t) {
                    Some(kept) => sm.mesh = kept,
                    None => continue,
                }
            }
            // The SHADOW state's own meshes (`WorldShadow_*`) are kept for the shadow-only instances.
            let shadow_mesh =
                variant.lod as usize == mw_formats::Model::STATE_SHADOW && sm.shader_name.starts_with("WorldShadow");
            if mw_assets::is_hidden_shader(&sm.shader_name) && !shadow_mesh {
                continue;
            }
            if let Some(mask) = variant.bones {
                let m = &sm.mesh;
                let in_mask = |v: u32| m.dominant_bone(v as usize).is_some_and(|b| b < 64 && mask & 1 << b != 0);
                match m.subset(|t| in_mask(t[0])) {
                    Some(kept) => sm.mesh = kept,
                    None => continue,
                }
            }
            // `InstanceDiffuse` shaders (the found billboard's picture, `World_SmashedBillboard_InstanceDiffuse`)
            // read their diffuse texture (`DiffuseSampler`, t0) from the placement, not the material.
            if let Some(t) = inst.diffuse
                && sm.shader_name.contains("InstanceDiffuse")
                && sm.texture("DiffuseTextureSampler").is_none()
            {
                sm.textures.push(("DiffuseTextureSampler".into(), t));
                sm.diffuse = Some(t);
            }
            let mut look = Look::from_scene_mesh(&sm).with_instance(&inst).with_paint(self.paint.as_ref());
            for key in look.all_textures().collect::<Vec<_>>() {
                self.texture(key)?;
            }
            look.retain_textures(|k| self.textures.contains(k));
            let Some(mut mesh) = convert::mesh(&sm.mesh, look.uv_b, look.vertex_colour) else { continue };
            if look.wet_road() {
                convert::add_road_wetness(&mut mesh, &sm.mesh);
            }
            if look.paint {
                convert::add_paint_grid(&mut mesh, &sm.mesh, look.effects_in_uv0());
            }
            if let Some(bones) = self.skin_bones {
                convert::add_skin(&mut mesh, &sm.mesh, bones);
            }
            parts.push(Part { mesh, look });
        }
        if parts.is_empty() {
            self.empty_models.insert(key);
            return Ok(None);
        }
        let paintable = parts.iter().any(|p| p.look.paint);
        if paintable && self.paintable.insert(id, true).is_none() {
            key = self.paint_key(key);
        }
        self.paintable.entry(id).or_insert(false);
        let textures: HashSet<TexKey> = parts.iter().flat_map(|p| p.look.all_textures()).collect();
        self.model_textures.insert(key, textures.into_iter().collect());
        self.parts.insert(key, parts);
        Ok(Some(key))
    }

    fn paint_key(&self, key: ResourceId) -> ResourceId {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut h);
        if let Some(p) = &self.paint {
            for v in p.diffuse.iter().chain(&p.specular).chain(&p.pearlescent) {
                v.to_bits().hash(&mut h);
            }
            for v in [p.specular_roughness, p.clearcoat_specular, p.clearcoat_fresnel, p.clearcoat_roughness] {
                v.to_bits().hash(&mut h);
            }
            p.clearcoat.hash(&mut h);
        }
        ResourceId(h.finish() & !(7 << 60) | 1 << 60)
    }

    fn texture(&mut self, key: TexKey) -> anyhow::Result<()> {
        self.owner_refs().textures.insert(key);
        if self.textures.contains(&key) || self.failed_textures.contains(&key) {
            return Ok(());
        }
        let (id, srgb) = key;
        match self.store.texture(id).ok().and_then(|t| convert::image(&t, srgb, self.bc_supported)) {
            Some(img) => {
                self.textures.insert(key);
                self.send(Msg::Texture(key, img))?
            }
            None => {
                self.failed_textures.insert(key);
            }
        }
        Ok(())
    }
}

/// Build one mesh for a group's instances: four vertices per sprite, all at the instance position, that
/// `groundcover.wgsl` (the port of the game's `Groundcover` vertex program) expands into a quad turned
/// about world up to face the camera, shrunk and dropped with distance as the game's CPU builder does
/// (`FUN_00eb9500`). The vertices carry what the game's builder writes (POSITION, NORMAL, the size bytes
/// and corner as [`crate::materials::ATTRIBUTE_GROUNDCOVER_SPRITE`], the colour) with the atlas cell
/// already applied to the texture coordinate (`region.xy + region.zw × (u, 1 − v)`, as the program does).
fn groundcover_mesh(mine: &[mw_formats::GroundcoverInstance], regions: &[[f32; 4]]) -> Option<Mesh> {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    if mine.is_empty() {
        return None;
    }
    let n = mine.len() * 4;
    let mut pos = Vec::with_capacity(n);
    let mut nor = Vec::with_capacity(n);
    let mut uv = Vec::with_capacity(n);
    let mut col = Vec::with_capacity(n);
    let mut sprite = Vec::with_capacity(n);
    let mut idx = Vec::with_capacity(mine.len() * 6);
    for inst in mine {
        // The program indexes `GroundcoverTextureRegions[round(A × 255)]`: 16 entries; a read past the
        // constant buffer gives zeros in D3D10.
        let [ox, oy, sx, sy] = regions.get(inst.region as usize).copied().unwrap_or([0.0; 4]);
        // The program outputs sqrt(2 tint) and the G-buffer stores sqrt(albedo): albedo = 2 tint (times the
        // texel; the layered shader clamps the product, as the 8-bit target does).
        let tint = inst.tint.map(|t| t as f32 / 255.0 * 2.0);
        let colour = [tint[0], tint[1], tint[2], 1.0];
        let scale = inst.size_scale();
        let base = pos.len() as u32;
        // Corners (u, v) = (0,0) (1,0) (1,1) (0,1): u across, v up from the base.
        for (c, (u, v)) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].into_iter().enumerate() {
            pos.push(inst.position);
            nor.push(inst.normal);
            uv.push([ox + sx * u, oy + sy * (1.0 - v)]);
            col.push(colour);
            sprite.push([inst.width as f32, inst.height as f32, scale, c as f32]);
        }
        idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nor);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    mesh.insert_attribute(crate::materials::ATTRIBUTE_GROUNDCOVER_SPRITE, sprite);
    mesh.insert_indices(Indices::U32(idx));
    Some(mesh)
}

/// World bounds of a group's sprites at full size, turned any way about world up and swayed by up to
/// `amount` metres at the top (`groundcover.wgsl`).
fn groundcover_bounds(mine: &[mw_formats::GroundcoverInstance], amount: f32) -> (Vec3, Vec3) {
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for inst in mine {
        let (w, h) = inst.size_m();
        let p = Vec3::from(inst.position);
        let side = w.abs() * 0.5 + amount.abs();
        lo = lo.min(p - Vec3::new(side, h.min(0.0), side));
        hi = hi.max(p + Vec3::new(side, h.max(0.0), side));
    }
    (lo, hi)
}

/// Game matrices are D3D row-major (`v * M`, translation in row 3). Some
/// writers leave 1.0 in the w column, so it is normalized here.
pub fn game_matrix(m: &[[f32; 4]; 4]) -> Mat4 {
    let col = |r: usize, w: f32| Vec4::new(m[r][0], m[r][1], m[r][2], w);
    Mat4::from_cols(col(0, 0.0), col(1, 0.0), col(2, 0.0), col(3, 1.0))
}

fn median_translation(p: &[mw_assets::Placement]) -> Option<Vec3> {
    let mut v: Vec<Vec3> =
        p.iter().map(|p| Vec3::new(p.transform[3][0], p.transform[3][1], p.transform[3][2])).collect();
    median_vec(&mut v)
}

fn median_vec(v: &mut [Vec3]) -> Option<Vec3> {
    if v.is_empty() {
        return None;
    }
    let mid = v.len() / 2;
    let mut axis = |f: fn(&Vec3) -> f32| {
        v.sort_by(|a, b| f(a).total_cmp(&f(b)));
        f(&v[mid])
    };
    Some(Vec3::new(axis(|p| p.x), axis(|p| p.y), axis(|p| p.z)))
}
