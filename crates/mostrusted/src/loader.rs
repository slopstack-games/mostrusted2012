//! Background asset streaming.
//!
//! A worker thread reads bundles from the user's game install, decodes
//! geometry and textures, converts them into Bevy assets and hands them to
//! the main thread over a channel. Track units are streamed nearest-first
//! from the focus point, much like the original game streams its world.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use bevy::prelude::*;
use mw_assets::AssetStore;
use mw_bundle::{ResourceId, ResourceType};

use crate::convert;
use crate::materials::{Look, TexKey};

/// What to load.
#[derive(Debug, Clone)]
pub enum Request {
    /// Stream world track units nearest-first from `focus` (default: the
    /// city centre). `max_units: None` loads all of them.
    World { max_units: Option<usize>, focus: Option<Vec3> },
    /// Load every model of a vehicle bundle at the origin.
    Vehicle { bundle: String },
    /// Load a drivable car, then stream the world around `spawn`.
    /// `world: false` loads only the car (for the test track).
    Drive { bundle: String, spawn: Vec3, heading: f32, max_units: Option<usize>, world: bool },
}

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
    pub body: Vec<ResourceId>,
    pub wheels: Vec<WheelSpec>,
    /// Handling parameters from the car's GeneSys data.
    pub handling: mw_assets::handling::VehicleHandling,
}

pub struct Part {
    pub mesh: Mesh,
    pub look: Look,
}

pub enum Msg {
    /// Where the camera should start.
    Focus(Vec3),
    Texture(TexKey, Image),
    Model(ResourceId, Vec<Part>),
    Instance {
        model: ResourceId,
        transform: Mat4,
    },
    /// Static world collision (one per track unit).
    Collider(avian3d::prelude::Collider),
    Car(CarSpec),
    Progress(String),
    Done,
}

/// Debug filters for world placements.
#[derive(Debug, Clone, Default)]
pub struct WorldFilter {
    /// Skip InstanceList backdrop entries.
    pub no_backdrop: bool,
    /// Skip these models entirely.
    pub exclude: HashSet<ResourceId>,
    /// Keep backdrop LOD2 stand-ins even though detailed geometry is loaded.
    pub keep_lod_proxies: bool,
}

#[derive(Resource)]
pub struct LoaderChannel(pub std::sync::Mutex<Receiver<Msg>>);

pub fn spawn(store: Arc<AssetStore>, request: Request, bc_supported: bool, filter: WorldFilter) -> LoaderChannel {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("mw-loader".into())
        .spawn(move || {
            let mut w = Worker {
                store,
                tx,
                bc_supported,
                models: HashSet::new(),
                textures: HashSet::new(),
                failed_textures: HashSet::new(),
                filter,
                empty_models: HashSet::new(),
            };
            if let Err(e) = w.run(request) {
                let _ = w.tx.send(Msg::Progress(format!("loader error: {e:#}")));
            }
            let _ = w.tx.send(Msg::Done);
        })
        .expect("spawn loader thread");
    LoaderChannel(std::sync::Mutex::new(rx))
}

struct Worker {
    store: Arc<AssetStore>,
    tx: Sender<Msg>,
    bc_supported: bool,
    models: HashSet<ResourceId>,
    textures: HashSet<TexKey>,
    failed_textures: HashSet<TexKey>,
    filter: WorldFilter,
    /// Models (or stripped variants) with nothing drawable.
    empty_models: HashSet<ResourceId>,
}

impl Worker {
    fn send(&self, m: Msg) -> anyhow::Result<()> {
        self.tx.send(m).map_err(|_| anyhow::anyhow!("main thread gone"))
    }

    fn run(&mut self, request: Request) -> anyhow::Result<()> {
        match request {
            Request::Vehicle { bundle } => {
                let b = self.store.bundle(&bundle).ok_or_else(|| anyhow::anyhow!("bundle not found: {bundle}"))?;
                self.send(Msg::Focus(Vec3::ZERO))?;
                let placements = match self.store.vehicle_placements(&b)? {
                    Some(p) => p,
                    None => b
                        .entries_of_type(ResourceType::Model)
                        .map(|e| mw_assets::Placement { model: e.id, transform: mw_assets::IDENTITY, backdrop: false })
                        .collect(),
                };
                for p in placements {
                    if self.model(p.model)? {
                        self.send(Msg::Instance { model: p.model, transform: game_matrix(&p.transform) })?;
                    }
                }
                self.send(Msg::Progress(format!("loaded {bundle}")))?;
            }
            Request::World { max_units, focus } => self.world(max_units, focus)?,
            Request::Drive { bundle, spawn, max_units, world, .. } => {
                self.car(&bundle)?;
                if world {
                    self.world(max_units, Some(spawn))?;
                }
            }
        }
        Ok(())
    }

    fn car(&mut self, bundle: &str) -> anyhow::Result<()> {
        let b = self.store.bundle(bundle).ok_or_else(|| anyhow::anyhow!("bundle not found: {bundle}"))?;
        let e = b
            .entries_of_type(ResourceType::VehicleGraphicsSpec)
            .next()
            .ok_or_else(|| anyhow::anyhow!("{bundle} has no VehicleGraphicsSpec"))?;
        let spec = mw_formats::VehicleGraphicsSpec::parse(&b.load(e)?)?;
        let handling = mw_assets::handling::VehicleHandling::from_bundle(&self.store, &b).unwrap_or_default();
        let mut car = CarSpec { body: Vec::new(), wheels: Vec::new(), handling };
        for &m in spec.parts.iter().flatten() {
            if self.model(m)? {
                car.body.push(m);
            }
        }
        // Classify wheels by position, not name: every car faces +Z, but some
        // specs label their axles or sides the other way round (the Veyron's
        // "Front" wheels are at the back).
        let mid_z = spec.wheels.iter().map(|w| w.position[2]).sum::<f32>() / spec.wheels.len().max(1) as f32;
        for w in &spec.wheels {
            let mut parts = Vec::new();
            for p in &w.parts {
                if let Some(m) = p.model
                    && self.model(m)?
                {
                    parts.push((m, p.spins));
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
        self.send(Msg::Car(car))?;
        self.store.evict_all();
        Ok(())
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
            let verts = verts.into_iter().map(Vec3::from).collect();
            self.send(Msg::Collider(Collider::trimesh_with_config(verts, tris, TrimeshFlags::FIX_INTERNAL_EDGES)))?;
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

        let total = max_units.unwrap_or(placed.len()).min(placed.len());
        for (i, (name, placements, _)) in placed.into_iter().take(total).enumerate() {
            self.send(Msg::Progress(format!("streaming {name} ({}/{total})", i + 1)))?;
            if let Some(b) = self.store.bundle(&name) {
                self.collision(&b)?;
            }
            for p in placements {
                if (p.backdrop && self.filter.no_backdrop) || self.filter.exclude.contains(&p.model) {
                    continue;
                }
                // Backdrop entries mix needed world-space geometry (terrain,
                // water, horizon) with LOD2 stand-ins for buildings whose
                // detailed versions are also loaded; drawing both overlaps, so
                // backdrop placements use a variant without the LOD2 meshes.
                let strip = p.backdrop && !self.filter.keep_lod_proxies;
                if let Some(model) = self.model_variant(p.model, strip)? {
                    self.send(Msg::Instance { model, transform: game_matrix(&p.transform) })?;
                }
            }
            self.store.evict_all();
        }
        self.send(Msg::Progress(format!("world loaded: {total} track units")))?;
        Ok(())
    }

    /// Ensure a model (and its textures) has been sent. Returns false if the
    /// model has nothing drawable.
    fn model(&mut self, id: ResourceId) -> anyhow::Result<bool> {
        Ok(self.model_variant(id, false)?.is_some())
    }

    /// Load a model, optionally without its LOD2 stand-in meshes. Returns the
    /// key the main thread knows it by (the stripped variant gets a synthetic
    /// ID with bit 62 set), or `None` if nothing is drawable.
    fn model_variant(&mut self, id: ResourceId, strip_lod2: bool) -> anyhow::Result<Option<ResourceId>> {
        let key = if strip_lod2 { ResourceId(id.0 | 1 << 62) } else { id };
        if self.models.contains(&key) {
            return Ok(Some(key));
        }
        if self.empty_models.contains(&key) {
            return Ok(None);
        }
        let meshes = match self.store.model_meshes(id, 0) {
            Ok((m, _errors)) => m,
            Err(_) => {
                self.empty_models.insert(key);
                return Ok(None);
            }
        };
        let mut parts = Vec::new();
        for sm in meshes {
            if strip_lod2 && sm.shader_name.contains("LOD2") {
                continue;
            }
            let name = sm.shader_name.to_ascii_lowercase();
            if skip_shader(&name) {
                continue;
            }
            let mut look = Look::from_scene_mesh(&sm);
            for key in look.textures.into_iter().flatten() {
                self.texture(key)?;
            }
            look.retain_textures(|k| self.textures.contains(k));
            let Some(mesh) = convert::mesh(&sm.mesh, look.uv_b) else { continue };
            parts.push(Part { mesh, look });
        }
        if parts.is_empty() {
            self.empty_models.insert(key);
            return Ok(None);
        }
        self.models.insert(key);
        self.send(Msg::Model(key, parts))?;
        Ok(Some(key))
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

/// Shaders that are not meant to be drawn as plain geometry.
fn skip_shader(name: &str) -> bool {
    ["shadow", "occlusion", "occluder", "invisible", "collision"].iter().any(|k| name.contains(k))
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
