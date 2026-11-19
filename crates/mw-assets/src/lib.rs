//! Game-wide resource access.
//!
//! Resources freely import resources from other bundles (a car's materials
//! use shaders from `SHADERS.BNDL` and textures from
//! `GLOBALTEXTUREDICTIONARY.BNDL`), so the engine needs one index over every
//! bundle in the install. [`AssetStore`] builds that index from the bundle
//! tables of contents (cheap: no resource data is read) and loads bundles on
//! demand.

pub mod backdrop;
pub mod buses;
pub mod camera;
pub mod car_anim;
pub mod car_lights;
pub mod car_sfx;
pub mod car_sound;
mod catalog;
pub mod coronas;
pub mod crash;
pub mod damage;
pub mod damage_vfx;
pub mod easydrive;
pub mod engine_driver;
pub mod engine_sound;
pub mod environment;
pub mod events;
pub mod frontend;
pub mod handling;
pub mod lights;
pub mod mods;
pub mod music;
pub mod packs;
pub mod paint;
pub mod police;
pub mod police_lights;
pub mod police_sound;
pub mod police_speech;
pub mod props;
mod scene;
pub mod scoring;
pub mod sequence;
pub mod sequence_fx;
pub mod sfx;
mod sfx_loader;
pub mod sound;
pub mod surfaces;
#[cfg(test)]
mod test_store;
pub mod traffic;
pub mod traffic_lamps;
pub mod ui;
pub mod ui_sounds;
pub mod vehicle_driver;
pub mod vfx;
pub mod wheel_layout;
pub mod widgets;
pub mod world_sfx;

pub use catalog::{DEFAULT_LANGUAGE, VehicleInfo};

pub use scene::{
    Groundcover, GroundcoverMaterial, IDENTITY, Placement, SceneMesh, WorldLayers, is_hidden_shader, trs_matrix,
};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mw_bundle::{Bundle, Resource, ResourceId, ResourceType};
use mw_formats as fmt;
use mw_gamedir::GameDir;
use rayon::prelude::*;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Bundle(#[from] mw_bundle::Error),
    #[error(transparent)]
    Format(#[from] mw_formats::Error),
    #[error("resource {0} not found in any bundle")]
    Missing(ResourceId),
    #[error("resource {id} is a {actual}, expected {expected}")]
    WrongType { id: ResourceId, expected: ResourceType, actual: ResourceType },
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy)]
struct Location {
    bundle: u32,
    entry: u32,
    type_id: u32,
}

/// Index of every resource in the game plus a cache of opened bundles.
pub struct AssetStore {
    game: GameDir,
    bundles: Vec<PathBuf>,
    index: HashMap<ResourceId, Vec<Location>>,
    open: Mutex<HashMap<u32, Arc<Bundle>>>,
    genesys_types: Mutex<HashMap<ResourceId, Option<fmt::genesys::Type>>>,
    buses: std::sync::OnceLock<buses::BusMix>,
}

impl AssetStore {
    /// Index every `.BNDL` in the install.
    pub fn new(game: GameDir) -> Result<Self> {
        let bundles = game.files_with_extension("BNDL");
        let tocs: Vec<_> = bundles
            .par_iter()
            .map(|rel| Bundle::read_toc_with(|offset, len| game.read_at(rel, offset, len)).map(|(_, e)| e))
            .collect::<std::result::Result<_, _>>()?;
        let mut index: HashMap<ResourceId, Vec<Location>> = HashMap::new();
        for (b, entries) in tocs.into_iter().enumerate() {
            for (e, entry) in entries.into_iter().enumerate() {
                index.entry(entry.id).or_default().push(Location {
                    bundle: b as u32,
                    entry: e as u32,
                    type_id: entry.type_id,
                });
            }
        }
        Ok(Self {
            game,
            bundles,
            index,
            open: Mutex::new(HashMap::new()),
            genesys_types: Mutex::new(HashMap::new()),
            buses: std::sync::OnceLock::new(),
        })
    }

    pub fn game(&self) -> &GameDir {
        &self.game
    }

    pub fn bundle_paths(&self) -> &[PathBuf] {
        &self.bundles
    }

    pub fn resource_count(&self) -> usize {
        self.index.len()
    }

    /// Open (or fetch from cache) a bundle by game-relative path.
    pub fn bundle(&self, relative: impl AsRef<Path>) -> Option<Arc<Bundle>> {
        let rel = relative.as_ref();
        let idx = self.bundles.iter().position(|p| paths_eq(p, rel))?;
        self.bundle_by_index(idx as u32).ok()
    }

    fn bundle_by_index(&self, idx: u32) -> Result<Arc<Bundle>> {
        if let Some(b) = self.open.lock().unwrap().get(&idx) {
            return Ok(b.clone());
        }
        let data = self.game.read(&self.bundles[idx as usize]).map_err(mw_bundle::Error::from)?;
        let b = Arc::new(Bundle::from_bytes(data)?);
        self.open.lock().unwrap().insert(idx, b.clone());
        Ok(b)
    }

    /// Drop all cached bundles.
    pub fn evict_all(&self) {
        self.open.lock().unwrap().clear();
    }

    /// Type of a resource without loading it.
    pub fn type_of(&self, id: ResourceId) -> Option<ResourceType> {
        self.index.get(&id).and_then(|l| l.first()).map(|l| ResourceType::from_id(l.type_id))
    }

    /// Bundles that contain a resource.
    pub fn locations(&self, id: ResourceId) -> Vec<&Path> {
        self.index
            .get(&id)
            .map(|ls| ls.iter().map(|l| self.bundles[l.bundle as usize].as_path()).collect())
            .unwrap_or_default()
    }

    /// Load a resource, preferring a copy in an already-open bundle.
    pub fn load(&self, id: ResourceId) -> Result<Resource> {
        let locs = self.index.get(&id).ok_or(Error::Missing(id))?;
        let loc = {
            let open = self.open.lock().unwrap();
            *locs.iter().find(|l| open.contains_key(&l.bundle)).unwrap_or(&locs[0])
        };
        let bundle = self.bundle_by_index(loc.bundle)?;
        Ok(bundle.load(&bundle.entries[loc.entry as usize])?)
    }

    fn load_typed(&self, id: ResourceId, expected: ResourceType) -> Result<Resource> {
        let res = self.load(id)?;
        let actual = res.resource_type();
        if actual != expected {
            return Err(Error::WrongType { id, expected, actual });
        }
        Ok(res)
    }

    pub fn texture(&self, id: ResourceId) -> Result<fmt::Texture> {
        Ok(fmt::Texture::parse(&self.load_typed(id, ResourceType::Texture)?)?)
    }
    pub fn material(&self, id: ResourceId) -> Result<fmt::Material> {
        Ok(fmt::Material::parse(&self.load_typed(id, ResourceType::Material)?)?)
    }
    pub fn sampler_state(&self, id: ResourceId) -> Result<fmt::SamplerState> {
        Ok(fmt::SamplerState::parse(&self.load_typed(id, ResourceType::SamplerState)?)?)
    }
    pub fn shader(&self, id: ResourceId) -> Result<fmt::Shader> {
        Ok(fmt::Shader::parse(&self.load_typed(id, ResourceType::Shader)?)?)
    }
    pub fn vertex_descriptor(&self, id: ResourceId) -> Result<fmt::VertexDescriptor> {
        Ok(fmt::VertexDescriptor::parse(&self.load_typed(id, ResourceType::VertexDescriptor)?)?)
    }
    pub fn renderable(&self, id: ResourceId) -> Result<fmt::Renderable> {
        Ok(fmt::Renderable::parse(&self.load_typed(id, ResourceType::Renderable)?)?)
    }
    pub fn model(&self, id: ResourceId) -> Result<fmt::Model> {
        Ok(fmt::Model::parse(&self.load_typed(id, ResourceType::Model)?)?)
    }
    pub fn vehicle_graphics_spec(&self, id: ResourceId) -> Result<fmt::VehicleGraphicsSpec> {
        Ok(fmt::VehicleGraphicsSpec::parse(&self.load_typed(id, ResourceType::VehicleGraphicsSpec)?)?)
    }
    /// A GeneSys type definition (cached; types are shared across bundles).
    pub fn genesys_type(&self, id: ResourceId) -> Option<fmt::genesys::Type> {
        if let Some(t) = self.genesys_types.lock().unwrap().get(&id) {
            return t.clone();
        }
        let t = self.load_typed(id, ResourceType::GenesysType).ok().and_then(|r| fmt::genesys::Type::parse(&r).ok());
        self.genesys_types.lock().unwrap().insert(id, t.clone());
        t
    }

    /// Decode a GenesysObject resource with types resolved across the game.
    /// The game's bus mix (loaded once).
    pub fn bus_mix(&self) -> &buses::BusMix {
        self.buses.get_or_init(|| buses::BusMix::load(self))
    }

    pub fn genesys_object(&self, id: ResourceId) -> Result<fmt::genesys::Object> {
        let res = self.load_typed(id, ResourceType::GenesysObject)?;
        self.decode_genesys(&res)
    }

    pub fn decode_genesys(&self, res: &Resource) -> Result<fmt::genesys::Object> {
        let types = |t: ResourceId| self.genesys_type(t);
        Ok(fmt::genesys::Decoder::new(res, &types).root()?)
    }

    pub fn instance_list(&self, id: ResourceId) -> Result<fmt::InstanceList> {
        Ok(fmt::InstanceList::parse(&self.load_typed(id, ResourceType::InstanceList)?)?)
    }

    /// Resolve the vertex layout a material's shader expects.
    pub fn layout_for_material(&self, material: &fmt::Material) -> Result<(fmt::Shader, fmt::VertexDescriptor)> {
        let shader_id = material.shader.ok_or(Error::Missing(ResourceId::NULL))?;
        let shader = self.shader(shader_id)?;
        let vd = self.vertex_descriptor(shader.vertex_descriptor.ok_or(Error::Missing(shader_id))?)?;
        Ok((shader, vd))
    }
}

fn paths_eq(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    norm(a) == norm(b)
}
