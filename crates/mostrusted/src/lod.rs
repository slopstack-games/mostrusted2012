//! The game's model LOD choice and draw distance for world placements (docs/decomp.md "Model LOD and draw
//! distance"; `FUN_008a02f0`, its view scale from `FUN_008b5fe0`).
//!
//! Per placement and frame the game measures the distance from the camera to the placed centre of the
//! model's LOD0 bounding sphere, takes off the sphere's radius (not below 0) and divides by the placement's
//! LOD scale and the view's LOD scale. Past the model's last LOD distance the placement is not drawn;
//! otherwise it draws the first LOD whose distance is not exceeded. The shadow pass draws it only while that
//! distance is also within the model's shadow distance.
//!
//! The view scale is `max(setting · 0.75 / tan(fov_h / 2), 1)` with `setting` the "LodScaling" display option:
//! 0 → 1, 1 → 1.5 (the game's own default on a machine with 3+ cores and a fast GPU, `FUN_00414b40`, and the
//! value in this install's `config.NFS13Save`), 2 → the pixel count over 1280×720, at most 1.5. Here the
//! setting is 1 unless `MW_LOD_SCALING=0|1|2` says otherwise; `MW_NO_LOD=1` draws every placement at LOD0
//! at any distance (the old behaviour, for A/B).
//!
//! Each placement's root entity carries [`ModelLod`]; its mesh children carry a LOD mask. Only the children's
//! [`Visibility`] is changed (the root's belongs to `props.rs`: a smashed compound hides its root), and only
//! when the chosen LOD changes.
use std::sync::Arc;

use bevy::camera::visibility::VisibilitySystems;
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use mw_bundle::ResourceId;

/// A model's LOD table (shared by its placements).
#[derive(Debug, Clone)]
pub struct LodTable {
    /// The LOD0 renderable's bounding sphere centre, model space.
    pub centre: Vec3,
    /// Its radius (the game does not scale it by the placement's matrix).
    pub radius: f32,
    /// LOD distances in metres (`Model` +0x08, stored squared), at most four.
    pub distances: Vec<f32>,
    /// The model's shadow distance in metres (`Model` +0x0C, stored squared).
    pub shadow: f32,
}

impl LodTable {
    pub fn from_model(model: &mw_formats::Model, sphere: [f32; 4]) -> Option<Self> {
        let n = model.lod_distances.len().min(4);
        (n > 0).then(|| Self {
            centre: Vec3::new(sphere[0], sphere[1], sphere[2]),
            radius: sphere[3],
            distances: model.lod_distances[..n].to_vec(),
            shadow: model.shadow_distance,
        })
    }

    /// `FUN_008a02f0`'s choice for an effective distance: `None` past the last LOD distance.
    fn lod(&self, d: f32) -> Option<u8> {
        if d > *self.distances.last()? {
            return None;
        }
        self.distances.iter().position(|&l| d <= l).map(|i| i as u8)
    }
}

/// One placement's LODs as the loader sends them.
#[derive(Debug, Clone)]
pub struct LodPlacement {
    pub table: Arc<LodTable>,
    /// The placement's own LOD scale (`InstanceList` +0x04; object lists after the instance id).
    pub scale: f32,
    /// The model key drawn at each LOD (index = LOD; neighbouring LODs may share a key; `None` = nothing
    /// drawable at that LOD). Empty: the placement's one model is drawn at every LOD (only the draw distance
    /// applies).
    pub levels: Vec<Option<ResourceId>>,
}

impl LodPlacement {
    /// The distinct model keys with the mask of LODs each is drawn at.
    pub fn level_masks(&self) -> Vec<(ResourceId, u8)> {
        let mut out: Vec<(ResourceId, u8)> = Vec::new();
        for (i, key) in self.levels.iter().enumerate() {
            let Some(key) = key else { continue };
            match out.iter_mut().find(|(k, _)| k == key) {
                Some((_, m)) => *m |= 1 << i,
                None => out.push((*key, 1 << i)),
            }
        }
        out
    }
}

/// Every LOD (the mask of a mesh drawn at all of them).
pub const ALL_LODS: u8 = 0xF;

const UNSET: u8 = 0xFF;
const CULLED: u8 = 0xFE;

/// A placement under the game's LOD rule (on the root entity).
#[derive(Component)]
pub struct ModelLod {
    table: Arc<LodTable>,
    scale: f32,
    /// Mesh children and the LODs each is drawn at.
    meshes: Vec<(Entity, u8)>,
    state: u8,
    shadow: Option<bool>,
}

impl ModelLod {
    pub fn new(table: Arc<LodTable>, scale: f32, meshes: Vec<(Entity, u8)>) -> Self {
        Self { table, scale, meshes, state: UNSET, shadow: None }
    }
}

/// On a LOD-managed mesh: whether the placement casts a shadow now (within the model's shadow distance), read
/// by `shadow_cull`.
#[derive(Component, Clone, Copy, Default)]
pub struct LodShadow(pub bool);

/// The view's LOD settings.
#[derive(Resource, Clone, Copy)]
pub struct LodSettings {
    pub enabled: bool,
    /// The "LodScaling" display option (0, 1, 2).
    pub scaling: u8,
}

impl LodSettings {
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var("MW_NO_LOD").map_or(true, |v| v == "0"),
            scaling: std::env::var("MW_LOD_SCALING").ok().and_then(|v| v.parse().ok()).unwrap_or(1).min(2),
        }
    }

    /// `FUN_008b5fe0`: the view's LOD scale for a horizontal FOV (radians) and the render target's pixels.
    pub fn view_scale(&self, fov_h: f32, pixels: Vec2) -> f32 {
        let setting = match self.scaling {
            0 => 1.0,
            1 => 1.5,
            _ => (pixels.x * pixels.y / (1280.0 * 720.0)).min(1.5),
        };
        (setting * 0.75 / (fov_h * 0.5).tan()).max(1.0)
    }
}

pub struct LodPlugin;

impl Plugin for LodPlugin {
    fn build(&self, app: &mut App) {
        let settings = LodSettings::from_env();
        info!("model LODs: {} (LodScaling {})", if settings.enabled { "on" } else { "off" }, settings.scaling);
        app.insert_resource(settings).add_systems(
            PostUpdate,
            choose_lods.after(TransformSystems::Propagate).before(VisibilitySystems::VisibilityPropagate),
        );
    }
}

fn choose_lods(
    settings: Res<LodSettings>,
    camera: Query<(&GlobalTransform, &Projection, &Camera), crate::MainCamera>,
    mut placements: Query<(&GlobalTransform, &mut ModelLod)>,
    mut meshes: Query<(&mut Visibility, &mut LodShadow)>,
    mut log: Local<Option<LodLog>>,
) {
    let log = log.get_or_insert_with(|| LodLog {
        on: std::env::var("MW_LOD_LOG").is_ok_and(|v| v != "0"),
        ..Default::default()
    });
    let start = log.on.then(std::time::Instant::now);
    let Some((eye_t, projection, cam)) = camera.iter().find(|(_, _, c)| c.is_active) else { return };
    let eye = eye_t.translation();
    let fov_h = match projection {
        Projection::Perspective(p) => crate::camera::game_fov(p).to_radians(),
        _ => 90f32.to_radians(),
    };
    let pixels = cam.physical_viewport_size().map_or(Vec2::new(1280.0, 720.0), |s| s.as_vec2());
    let view = if settings.enabled { settings.view_scale(fov_h, pixels) } else { f32::INFINITY };
    for (gt, lod) in &mut placements {
        let t = &lod.table;
        let d = ((gt.transform_point(t.centre).distance(eye) - t.radius).max(0.0)) / (lod.scale * view);
        let (state, shadow) = match t.lod(d) {
            Some(l) => (l, d <= t.shadow),
            None => (CULLED, false),
        };
        if log.on {
            log.count(state, shadow);
        }
        if state == lod.state && Some(shadow) == lod.shadow {
            continue;
        }
        let lod = lod.into_inner();
        for &(e, mask) in &lod.meshes {
            let Ok((mut vis, mut cast)) = meshes.get_mut(e) else { continue };
            let shown = state != CULLED && mask & 1 << state != 0;
            vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
            cast.set_if_neq(LodShadow(shadow));
        }
        lod.state = state;
        lod.shadow = Some(shadow);
    }
    if let Some(start) = start {
        log.finish(start.elapsed().as_secs_f64(), view);
    }
}

/// `MW_LOD_LOG=1`: placements per LOD, culled and shadow-casting, and the time [`choose_lods`] takes, every
/// 2 s.
#[derive(Default)]
struct LodLog {
    on: bool,
    lods: [u32; 4],
    culled: u32,
    shadows: u32,
    frames: u32,
    seconds: f64,
    since: Option<std::time::Instant>,
}

impl LodLog {
    fn count(&mut self, state: u8, shadow: bool) {
        match state {
            CULLED => self.culled += 1,
            l => self.lods[l as usize & 3] += 1,
        }
        self.shadows += shadow as u32;
    }

    fn finish(&mut self, seconds: f64, view: f32) {
        self.frames += 1;
        self.seconds += seconds;
        let since = *self.since.get_or_insert_with(std::time::Instant::now);
        if since.elapsed().as_secs_f32() >= 2.0 {
            let n = self.frames.max(1);
            info!(
                "LOD view scale {view:.3}: per frame LOD0..3 {:?}, culled {}, casting {}; choose_lods {:.3} ms",
                self.lods.map(|c| c / n),
                self.culled / n,
                self.shadows / n,
                self.seconds * 1e3 / n as f64
            );
            *self = LodLog { on: true, ..Default::default() };
        }
    }
}

impl PartialEq for LodShadow {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

/// The meshes to spawn for a placement: (model key, parts, mask of the LODs they are drawn at). Without LOD
/// levels: the placement's one model at every LOD.
pub fn mesh_sets(
    lod: Option<&LodPlacement>,
    lod0: (ResourceId, &crate::ModelParts),
    models: &std::collections::HashMap<ResourceId, crate::ModelParts>,
) -> Vec<(ResourceId, crate::ModelParts, u8)> {
    match lod {
        Some(l) if !l.levels.is_empty() => {
            l.level_masks().into_iter().filter_map(|(k, m)| Some((k, models.get(&k)?.clone(), m))).collect()
        }
        _ => vec![(lod0.0, lod0.1.clone(), ALL_LODS)],
    }
}

/// A LOD-managed mesh's visibility before [`choose_lods`] first runs: LOD0's meshes shown.
pub fn initial_visibility(mask: u8) -> Visibility {
    if mask & 1 != 0 { Visibility::Inherited } else { Visibility::Hidden }
}
