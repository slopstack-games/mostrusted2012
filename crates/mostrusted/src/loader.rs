//! Background asset streaming.
//!
//! A worker thread reads bundles from the user's game install, decodes
//! geometry and textures, converts them into Bevy assets and hands them to
//! the main thread over a channel. Track units are streamed nearest-first
//! from the focus point, much like the original game streams its world.

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
    pub body: Vec<ResourceId>,
    pub wheels: Vec<WheelSpec>,
    /// Handling parameters from the car's GeneSys data.
    pub handling: mw_assets::handling::VehicleHandling,
    /// Performance mods (`mods_menu.rs`); `handling` is the stock car.
    pub mod_catalogue: crate::mods_menu::ModCatalogue,
    /// Engine sound (`AssetStore::engine`): the car's sequence bank, or Ginsu layers for cars without one.
    pub engine: Option<mw_assets::engine_sound::EngineVoice>,
    /// Deformation data; the body meshes carry joint attributes when this is set (`damage.rs`).
    pub damage: Option<Arc<mw_assets::damage::VehicleDamage>>,
    pub impact_graph: mw_assets::damage::ImpactGraph,
    /// `VehicleVfxBehaviour` spot effects with their locators (exhaust flames, smoke, ...; `exhaust.rs`).
    pub spot_effects: Vec<mw_assets::vfx::VehicleSpotEffect>,
    /// Set for an AI opponent's car (its slot in `ai::Opponents`); `None` for the player's.
    pub opponent: Option<usize>,
    /// The vehicle list's top speed (entry +0x3C, mph): the nitrous HighSpeed refill's reference.
    pub top_speed_mph: f32,
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
    Instance {
        model: ResourceId,
        transform: Mat4,
    },
    /// One groundcover group (a model at the origin), drawn while the camera is within `range` of its box.
    Groundcover {
        model: ResourceId,
        min: Vec3,
        max: Vec3,
        range: f32,
    },
    /// A prop / compound / dynamic object with physics (`props.rs`).
    Prop {
        model: ResourceId,
        transform: Mat4,
        physics: Arc<crate::props::PropPhysics>,
    },
    /// Static world collision (one per `PolygonSoupList`) and the collision tag of each of its triangles.
    Collider(avian3d::prelude::Collider, Arc<Vec<u32>>),
    /// The game's collision surface list (`mw_assets::surfaces`).
    Surfaces(Vec<mw_assets::surfaces::SurfaceInfo>),
    Car(Box<CarSpec>),
    /// A traffic car model ([`CarRequest::traffic`]).
    TrafficCar(Box<TrafficCarSpec>),
    /// A runtime [`CarRequest`] could not be loaded.
    CarFailed(String),
    Progress(String),
    Done,
}

/// Which `InstanceList` backdrop entries to draw (see `mw_assets::backdrop`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BackdropMode {
    /// One backdrop per place: a backdrop triangle is drawn where the zone owning its position lists its
    /// model, i.e. what the game shows there when the player stands in that zone.
    #[default]
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
    /// Draw props, compounds and dynamic objects without collision (no `--prop-physics`).
    pub no_prop_physics: bool,
    /// Draw every static world placement as its own entity (the default: without `--merge-static`)
    /// instead of merging a track unit's single-use static placements per material.
    pub no_merge: bool,
}

#[derive(Resource)]
pub struct LoaderChannel(pub std::sync::Mutex<Receiver<Msg>>);

pub fn spawn(
    store: Arc<AssetStore>,
    request: Request,
    bc_supported: bool,
    filter: WorldFilter,
) -> (LoaderChannel, LoaderRequests) {
    let (tx, rx) = channel();
    let (req_tx, req_rx) = channel();
    std::thread::Builder::new()
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
                opponent: None,
                backdrop: None,
                backdrop_sent: HashSet::new(),
                prop_physics: HashMap::new(),
            };
            if let Err(e) = w.run(request) {
                let _ = w.tx.send(Msg::Progress(format!("loader error: {e:#}")));
            }
            let _ = w.tx.send(Msg::Done);
            // Then serve runtime car requests until the main thread drops its sender.
            while let Ok(req) = w.requests.recv() {
                if w.car_request(req).is_err() {
                    break;
                }
            }
        })
        .expect("spawn loader thread");
    (LoaderChannel(std::sync::Mutex::new(rx)), LoaderRequests(req_tx))
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
    /// The opponent slot of the car being loaded (`CarRequest::opponent`).
    opponent: Option<usize>,
    /// Backdrop composition for [`BackdropMode::Composed`].
    backdrop: Option<BackdropSelector>,
    /// Backdrop instances already sent (the same backdrop model is listed by many units).
    backdrop_sent: HashSet<(ResourceId, [u32; 16])>,
    /// Physics per object definition (shared by its placements).
    prop_physics: HashMap<(ResourceId, mw_assets::props::ObjectKind), Option<Arc<crate::props::PropPhysics>>>,
}

/// How a model is turned into drawable parts.
#[derive(Clone, Copy, Default)]
struct Variant {
    /// Drop meshes whose shader is a LOD2 stand-in.
    strip_lod2: bool,
    /// Keep only the backdrop triangles the zone composition draws, for a placement with this transform.
    compose_backdrop: Option<[[f32; 4]; 4]>,
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
                        })
                        .collect(),
                };
                for p in placements {
                    if let Some(model) = self.model_variant(p.model, Variant::default(), InstanceLook::default())? {
                        self.send(Msg::Instance { model, transform: game_matrix(&p.transform) })?;
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
        if let Some(t) = &req.traffic {
            if let Err(e) = self.traffic_car(&req.bundle, t) {
                self.send(Msg::Progress(format!("traffic car {}: {e:#}", req.bundle)))?;
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

    /// Serve any runtime car requests that are waiting (called between track units).
    fn poll_requests(&mut self) -> anyhow::Result<()> {
        loop {
            match self.requests.try_recv() {
                Ok(req) => self.car_request(req)?,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return Ok(()),
            }
        }
    }

    fn select_paint(&mut self, b: &mw_bundle::Bundle) {
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
    }

    /// A `NitrousParameters` variant's rules (`mw_handling::nitrous`).
    fn nitrous_rules(&self, id: &str) -> Option<mw_handling::nitrous::NitrousRules> {
        let f = mw_assets::mods::nitrous_parameters(&self.store, id)?;
        Some(mw_handling::nitrous::NitrousRules::from_fields(|b, k| f.get(&(b, k)).copied()))
    }

    fn car(&mut self, bundle: &str) -> anyhow::Result<()> {
        let b = self.store.bundle(bundle).ok_or_else(|| anyhow::anyhow!("bundle not found: {bundle}"))?;
        self.select_paint(&b);
        let e = b
            .entries_of_type(ResourceType::VehicleGraphicsSpec)
            .next()
            .ok_or_else(|| anyhow::anyhow!("{bundle} has no VehicleGraphicsSpec"))?;
        let mut spec = mw_formats::VehicleGraphicsSpec::parse(&b.load(e)?)?;
        let mut handling = mw_assets::handling::VehicleHandling::from_bundle(&self.store, &b).unwrap_or_default();
        // Broken wheel data (the Venom's front axle): move the axle onto the body's arches, for the
        // physics wheels as well (`mw_assets::wheel_arches`).
        for (old, new) in self.store.repair_wheel_layout(&mut spec) {
            self.send(Msg::Progress(format!("{bundle}: axle at z {old:.3} moved to the body's arch at z {new:.3}")))?;
            for w in handling.wheels.iter_mut().filter(|w| (w.position[2] - old).abs() < 0.05) {
                w.position[2] += new - old;
            }
        }
        let engine = self.store.engine(&b).unwrap_or_default();
        let spot_effects = self.store.vehicle_spot_effects(&b, &spec).unwrap_or_default();
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
            body: Vec::new(),
            wheels: Vec::new(),
            handling,
            engine,
            damage,
            impact_graph,
            spot_effects,
            opponent: self.opponent,
            top_speed_mph: bundle_vehicle_id(bundle)
                .and_then(|id| self.store.vehicle_entry(id).ok().flatten())
                .map_or(0.0, |v| v.top_speed),
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
        self.store.repair_wheel_layout(&mut spec);
        let saved = (self.paint, self.paint_count);
        self.paint = req.colour;
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
        self.send(Msg::TrafficCar(Box::new(TrafficCarSpec { vehicle: req.vehicle, paint: req.paint, body, wheels })))
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
            Ok(list) => self.send(Msg::Surfaces(list)),
            Err(e) => self.send(Msg::Progress(format!("no surface list: {e}"))),
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

    /// Send a placement: with physics when it is a world object that has some.
    fn send_placement(&mut self, model: ResourceId, p: &mw_assets::Placement) -> anyhow::Result<()> {
        let transform = game_matrix(&p.transform);
        match p.object.and_then(|(object, kind)| self.prop_physics(object, kind)) {
            Some(physics) => self.send(Msg::Prop { model, transform, physics }),
            None => self.send(Msg::Instance { model, transform }),
        }
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
        };
        if let Some(key) = self.model_variant(model, Variant::default(), InstanceLook::default())? {
            self.send_placement(key, &placement)?;
        }
        Ok(())
    }

    fn world(&mut self, max_units: Option<usize>, focus: Option<Vec3>) -> anyhow::Result<()> {
        let units: Vec<String> = self
            .store
            .bundle_paths()
            .iter()
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")))
            .map(|p| p.to_string_lossy().into_owned())
            .collect();

        // Read every unit's placements first (cheap) to find the city centre
        // and order units nearest-first.
        let mut placed = Vec::with_capacity(units.len());
        for name in &units {
            let Some(b) = self.store.bundle(name) else { continue };
            let p = self.store.bundle_placements(&b)?;
            let centre = median_translation(&p);
            placed.push((name.clone(), p, centre));
            self.store.evict_all();
        }
        let mut centres: Vec<Vec3> = placed.iter().filter_map(|(_, _, c)| *c).collect();
        let focus = focus.or_else(|| median_vec(&mut centres)).unwrap_or(Vec3::ZERO);
        self.send(Msg::Focus(focus))?;
        placed.sort_by(|a, b| {
            let d = |c: &Option<Vec3>| c.map_or(f32::MAX, |c| c.distance_squared(focus));
            d(&a.2).total_cmp(&d(&b.2))
        });

        // Backdrop composition needs every unit's backdrop list, also of units not streamed.
        if self.filter.backdrop == BackdropMode::Composed {
            self.send(Msg::Progress("indexing backdrops".into()))?;
            let listings = placed.iter().flat_map(|(name, placements, _)| {
                let unit = track_unit_number(name);
                placements.iter().filter(|p| p.backdrop).filter_map(move |p| Some((unit?, p.model, &p.transform)))
            });
            match self.store.backdrop_selector(listings) {
                Ok(sel) => self.backdrop = Some(sel),
                Err(e) => {
                    self.send(Msg::Progress(format!("no zone list ({e}): drawing every backdrop")))?;
                }
            }
        }

        let total = max_units.unwrap_or(placed.len()).min(placed.len());
        if let Some(sel) = &mut self.backdrop {
            sel.set_loaded(placed.iter().take(total).filter_map(|(name, _, _)| track_unit_number(name)));
        }
        for (i, (name, placements, _)) in placed.into_iter().take(total).enumerate() {
            self.poll_requests()?;
            self.send(Msg::Progress(format!("streaming {name} ({}/{total})", i + 1)))?;
            if let Some(b) = self.store.bundle(&name) {
                self.collision(&b)?;
                if !self.filter.no_groundcover {
                    self.groundcover(&b)?;
                }
            }
            let unit = track_unit_number(&name);
            let mut statics = Vec::new();
            for p in placements {
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
                            let bits = std::array::from_fn(|i| p.transform[i / 4][i % 4].to_bits());
                            if !self.backdrop_sent.insert((p.model, bits)) {
                                continue;
                            }
                            variant.compose_backdrop = Some(p.transform);
                        }
                        // Unfiltered backdrops mix needed world-space geometry (terrain, water, horizon)
                        // with LOD2 stand-ins for buildings whose detailed versions are also loaded.
                        _ => variant.strip_lod2 = !self.filter.keep_lod_proxies,
                    }
                }
                // Static placements (no physics, not a backdrop) are merged per material below.
                let physics = p.object.and_then(|(object, kind)| self.prop_physics(object, kind));
                if !self.filter.no_merge && !p.backdrop && physics.is_none() {
                    if let Some(model) = self.model_parts(p.model, variant, InstanceLook::from_placement(&p), true)? {
                        statics.push((model, game_matrix(&p.transform)));
                    }
                    continue;
                }
                if let Some(model) = self.model_variant(p.model, variant, InstanceLook::from_placement(&p))? {
                    self.send_placement(model, &p)?;
                }
            }
            self.send_merged(std::mem::take(&mut statics))?;
            self.parts.clear();
            self.store.evict_all();
        }
        self.send(Msg::Progress(format!("world loaded: {total} track units")))?;
        Ok(())
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
                self.send(Msg::Instance { model: key, transform })?;
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
                self.send(Msg::Model(key, vec![Part { mesh, look: g.0 }]))?;
                self.send(Msg::Instance { model: key, transform: Mat4::IDENTITY })?;
                start = end;
            }
        }
        Ok(())
    }

    /// Send the groundcover of every collection in a bundle that has not been sent yet: one merged
    /// mesh per collection and sprite class, drawn as an ordinary model at the origin.
    fn groundcover(&mut self, b: &mw_bundle::Bundle) -> anyhow::Result<()> {
        let ids: Vec<ResourceId> = b
            .entries_of_type(ResourceType::GroundcoverCollection)
            .map(|e| e.id)
            .filter(|id| self.groundcover_done.insert(*id))
            .collect();
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
                    parameters: Vec::new(),
                    mesh: mw_formats::DecodedMesh::default(),
                };
                let mut look = Look::from_scene_mesh(&sm);
                for key in look.all_textures().collect::<Vec<_>>() {
                    self.texture(key)?;
                }
                look.retain_textures(|k| self.textures.contains(k));
                // One model per group: the game culls whole groups by distance (`FUN_00eb9500`).
                for (gi, (g, range)) in gc.groups.iter().enumerate().filter(|(_, (g, _))| g.material == m) {
                    let Some(mesh) = groundcover_mesh(&gc.instances[range.clone()], &mat.regions) else { continue };
                    let key = ResourceId(0x6000_0000_0000_0000 | (gi as u64) << 32 | id.0 & 0xFFFF_FFFF);
                    self.send(Msg::Model(key, vec![Part { mesh, look }]))?;
                    self.send(Msg::Groundcover {
                        model: key,
                        min: Vec3::from(g.min),
                        max: Vec3::from(g.max),
                        range: g.radius * mw_formats::groundcover::DRAW_DISTANCE_SCALE,
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
            self.send(Msg::Model(key, parts))?;
        }
        Ok(())
    }

    /// Decode a model variant into [`Self::parts`] (see [`Self::model_variant`]) unless it is there or,
    /// without `need_parts`, already sent.
    fn model_parts(
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
        // Body-paint models are baked with the current paint, so each paint is its own variant
        // (bit 60, a hash of the ID and the paint's values).
        if self.paintable.get(&id) == Some(&true) {
            key = self.paint_key(key);
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
        let meshes = match self.store.model_meshes(id, 0) {
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
            if mw_assets::is_hidden_shader(&sm.shader_name) {
                continue;
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

/// Build one mesh for a group's instances. The game's `Groundcover` vertex program turns each instance into
/// one quad that turns about the vertical to face the camera (right = perpendicular to the horizontal
/// direction from the camera, up = world up), `width x (u - 0.5)` across and `height x v` up from the
/// instance position. Without a vertex-shader path here, two crossed upright quads of that size stand in
/// for the turning one (model decision; `GroundcoverInstance::size_m` for the size).
fn groundcover_mesh(mine: &[mw_formats::GroundcoverInstance], regions: &[[f32; 4]]) -> Option<Mesh> {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    if mine.is_empty() {
        return None;
    }
    let quads = mine.len() * 2;
    let mut pos = Vec::with_capacity(quads * 4);
    let mut nor = Vec::with_capacity(quads * 4);
    let mut uv = Vec::with_capacity(quads * 4);
    let mut col = Vec::with_capacity(quads * 4);
    let mut idx = Vec::with_capacity(quads * 6);
    for inst in mine {
        let region = regions.get(inst.region as usize).copied().filter(|r| r[2] > 0.0 && r[3] > 0.0);
        let [ox, oy, sx, sy] = region.unwrap_or([0.0, 0.0, 1.0, 1.0]);
        // The shader outputs sqrt(2 * tint) and the g-buffer stores sqrt(albedo): albedo = 2 * tint (times
        // the texel; the layered shader clamps the product, as the 8-bit target does).
        let tint = inst.tint.map(|t| t as f32 / 255.0 * 2.0);
        let colour = [tint[0], tint[1], tint[2], 1.0];
        let (width, height) = inst.size_m();
        // The vertex program's normal (TEXCOORD1): normalize(sign(w) right (u - 0.5) + up v - 0.1 d) +
        // max(8 - height, 0.75) (ground normal + (0, 0.25, 0)), d = horizontal direction from the camera;
        // the pixel program normalises it. The camera term is left out for the static quads.
        let ground = (Vec3::from(inst.normal) + Vec3::new(0.0, 0.25, 0.0)) * (8.0 - height).max(0.75);
        let sign = if width > 0.0 {
            1.0
        } else if width < 0.0 {
            -1.0
        } else {
            0.0
        };
        for q in 0..2 {
            let a = q as f32 * std::f32::consts::FRAC_PI_2;
            let right = Vec3::new(a.cos(), 0.0, a.sin());
            let n = |u: f32, v: f32| {
                ((sign * right * (u - 0.5) + Vec3::Y * v).normalize_or_zero() + ground).normalize_or(Vec3::Y).to_array()
            };
            let (dx, dz) = (a.cos() * width * 0.5, a.sin() * width * 0.5);
            let base = pos.len() as u32;
            let p = inst.position;
            let top = p[1] + height;
            pos.extend([
                [p[0] - dx, p[1], p[2] - dz],
                [p[0] + dx, p[1], p[2] + dz],
                [p[0] + dx, top, p[2] + dz],
                [p[0] - dx, top, p[2] - dz],
            ]);
            nor.extend([n(0.0, 0.0), n(1.0, 0.0), n(1.0, 1.0), n(0.0, 1.0)]);
            uv.extend([[ox, oy + sy], [ox + sx, oy + sy], [ox + sx, oy], [ox, oy]]);
            col.extend([colour; 4]);
            idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nor);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    mesh.insert_indices(Indices::U32(idx));
    Some(mesh)
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
