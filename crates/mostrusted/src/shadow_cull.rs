//! Per-cascade filtering of the key light's shadow casters, after Bevy's own culling
//! (`check_dir_light_mesh_visibility`) and before the lists are extracted to the render world.
//!
//! **Receiver-footprint cull** (default on, picture-neutral by construction): our lighting pass
//! (`game_lighting.wgsl` `game_shadow`) samples cascade i only for visible points whose light-space xy falls
//! inside that cascade, and the shadow is faded out completely at the far bound of the last cascade. So a
//! caster can only darken a pixel if its light-space xy footprint overlaps the light-space xy footprint of
//! the view frustum from `minimum_distance` to that far bound. Bevy instead keeps every caster inside the
//! cascade's square box, which bounds a sphere around the frustum slice (most of it to the sides of and
//! behind the camera). Each caster's world AABB (8 corners) and the view frustum's 8 corners are projected
//! with the cascade's `clip_from_world`; a caster whose xy rectangle misses the frustum's xy rectangle,
//! widened by [`FOOTPRINT_MARGIN_TEXELS`] texels for the 2x2 PCF tap and the receiver's normal offset, is
//! dropped from that cascade.
//!
//! Debug switches for attributing the shadow pass's cost (research log, session 5 round 6), read once from
//! the environment:
//! - `MW_SHADOW_CASCADE_OFF=0,2`: those cascades draw no casters.
//! - `MW_SHADOW_CLASS_OFF=alpha,world,backdrop,prop,car,other`: casters of those [`ShadowClass`]es are dropped
//!   from every cascade (`other`: meshes without a class, e.g. particles).
//! - `MW_SHADOW_FOOTPRINT=0`: turn off the receiver-footprint cull below (on by default).
//! - `MW_SHADOW_MIN_TEXELS=<k>`: a caster whose bounding sphere (the mesh AABB's half diagonal times the
//!   largest axis scale) has a diameter below `k` texels of a cascade is dropped from that cascade.
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::CascadesVisibleEntities;
use bevy::light::{CascadeShadowConfig, Cascades, DirectionalLightShadowMap, SimulationLightSystems};
use bevy::prelude::*;

/// A model's own shadow distance in metres (`Model` +0x0C, stored squared): see [`ShadowCull::model_ranges`].
#[derive(Component, Clone, Copy, Debug)]
pub struct ShadowRange(pub f32);

/// What kind of object a mesh entity belongs to (only used to attribute shadow cost).
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShadowClass {
    /// A world placement (instanced or merged static geometry), opaque material.
    World,
    /// A world placement with an alpha-tested material (trees, fences, foliage cards).
    Alpha,
    /// A backdrop placement (composed or LOD2-stripped backdrop variant).
    Backdrop,
    /// A placement with physics (`props.rs`).
    Prop,
    /// The player's car.
    Car,
}

#[derive(Resource, Default, Clone)]
pub struct ShadowCull {
    pub cascade_off: [bool; 4],
    pub class_off: Vec<ShadowClass>,
    pub other_off: bool,
    pub min_texels: f32,
    /// Drop a caster from every cascade while the camera is farther than its model's shadow distance from the
    /// centre of its bounding box (`MW_SHADOW_MODEL_RANGE=1`).
    pub model_ranges: bool,
    /// The receiver-footprint cull (module docs).
    pub footprint: bool,
}

impl ShadowCull {
    pub fn from_env() -> Self {
        let mut s = ShadowCull::default();
        if let Ok(v) = std::env::var("MW_SHADOW_CASCADE_OFF") {
            for i in v.split(',').filter_map(|x| x.trim().parse::<usize>().ok()) {
                if i < 4 {
                    s.cascade_off[i] = true;
                }
            }
        }
        if let Ok(v) = std::env::var("MW_SHADOW_CLASS_OFF") {
            for c in v.split(',') {
                match c.trim() {
                    "world" => s.class_off.push(ShadowClass::World),
                    "alpha" => s.class_off.push(ShadowClass::Alpha),
                    "backdrop" => s.class_off.push(ShadowClass::Backdrop),
                    "prop" => s.class_off.push(ShadowClass::Prop),
                    "car" => s.class_off.push(ShadowClass::Car),
                    "other" => s.other_off = true,
                    "" => {}
                    x => warn!("MW_SHADOW_CLASS_OFF: unknown class {x}"),
                }
            }
        }
        if let Some(k) = std::env::var("MW_SHADOW_MIN_TEXELS").ok().and_then(|v| v.parse().ok()) {
            s.min_texels = k;
        }
        s.footprint = std::env::var("MW_SHADOW_FOOTPRINT").map_or(true, |v| v != "0");
        s.model_ranges = std::env::var("MW_SHADOW_MODEL_RANGE").is_ok_and(|v| v != "0");
        s
    }

    fn active(&self) -> bool {
        self.cascade_off.iter().any(|&b| b)
            || !self.class_off.is_empty()
            || self.other_off
            || self.min_texels > 0.0
            || self.model_ranges
            || self.footprint
    }
}

/// Margin around the view frustum's light-space footprint, in shadow-map texels (PCF tap + normal bias).
const FOOTPRINT_MARGIN_TEXELS: f32 = 4.0;

pub struct ShadowCullPlugin;

impl Plugin for ShadowCullPlugin {
    fn build(&self, app: &mut App) {
        let cfg = ShadowCull::from_env();
        if cfg.active() {
            info!(
                "shadow cull: cascades off {:?}, classes off {:?} (other {}), min {} texels, model ranges {}",
                cfg.cascade_off, cfg.class_off, cfg.other_off, cfg.min_texels, cfg.model_ranges
            );
        }
        app.insert_resource(cfg)
            .add_systems(PostUpdate, cull_casters.after(SimulationLightSystems::CheckLightVisibility));
    }
}

/// The xy bounding rectangle of points projected by an orthographic `clip_from_world`.
fn clip_rect(clip_from_world: &Mat4, points: impl Iterator<Item = Vec3>) -> (Vec2, Vec2) {
    points.fold((Vec2::MAX, Vec2::MIN), |(lo, hi), p| {
        let q = clip_from_world.project_point3(p).truncate();
        (lo.min(q), hi.max(q))
    })
}

fn cull_casters(
    cfg: Res<ShadowCull>,
    shadow_map: Res<DirectionalLightShadowMap>,
    mut lights: Query<(&mut CascadesVisibleEntities, &Cascades, &CascadeShadowConfig)>,
    casters: Query<(Option<&ShadowClass>, Option<&Aabb>, &GlobalTransform, Option<&ShadowRange>)>,
    views: Query<(&GlobalTransform, Option<&Projection>)>,
) {
    if !cfg.active() {
        return;
    }
    let shadow_map_size = shadow_map.size;
    for (mut visible, cascades, config) in &mut lights {
        for (view, lists) in visible.entities.iter_mut() {
            let Some(cs) = cascades.cascades.get(view) else { continue };
            let view_tf = views.get(*view).ok();
            let eye = view_tf.map_or(Vec3::ZERO, |(t, _)| t.translation());
            // The view frustum over the whole shadow range, in world space.
            let frustum: Option<Vec<Vec3>> = match (cfg.footprint, view_tf, config.bounds.last()) {
                (true, Some((t, Some(projection))), Some(&far)) => Some(
                    projection
                        .get_frustum_corners(-config.minimum_distance, -far)
                        .iter()
                        .map(|c| t.transform_point(Vec3::from(*c)))
                        .collect(),
                ),
                _ => None,
            };
            for (i, list) in lists.iter_mut().enumerate() {
                if cfg.cascade_off.get(i).copied().unwrap_or(false) {
                    list.entities.clear();
                    continue;
                }
                let min_diameter = cfg.min_texels * cs.get(i).map_or(0.0, |c| c.texel_size);
                // The frustum's light-space xy rectangle in this cascade's clip space, widened by the margin.
                let footprint = frustum.as_ref().zip(cs.get(i)).map(|(corners, c)| {
                    let (lo, hi) = clip_rect(&c.clip_from_world, corners.iter().copied());
                    let m = FOOTPRINT_MARGIN_TEXELS * 2.0 / shadow_map_size as f32;
                    (lo - m, hi + m, c.clip_from_world)
                });
                list.entities.retain(|&e| {
                    let Ok((class, aabb, gt, range)) = casters.get(e) else { return true };
                    match class {
                        Some(c) if cfg.class_off.contains(c) => return false,
                        None if cfg.other_off => return false,
                        _ => {}
                    }
                    if cfg.model_ranges
                        && let (Some(range), Some(aabb)) = (range, aabb)
                        && gt.transform_point(Vec3::from(aabb.center)).distance_squared(eye) > range.0 * range.0
                    {
                        return false;
                    }
                    if let (Some((lo, hi, clip_from_world)), Some(aabb)) = (&footprint, aabb) {
                        let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
                        let corners = (0..8).map(|k| {
                            let s = Vec3::new(
                                if k & 1 == 0 { -1.0 } else { 1.0 },
                                if k & 2 == 0 { -1.0 } else { 1.0 },
                                if k & 4 == 0 { -1.0 } else { 1.0 },
                            );
                            gt.transform_point(c + h * s)
                        });
                        let (clo, chi) = clip_rect(clip_from_world, corners);
                        if chi.x < lo.x || clo.x > hi.x || chi.y < lo.y || clo.y > hi.y {
                            return false;
                        }
                    }
                    if min_diameter > 0.0
                        && let Some(aabb) = aabb
                    {
                        let m = gt.affine().matrix3;
                        let scale = m.x_axis.length().max(m.y_axis.length()).max(m.z_axis.length());
                        let diameter = 2.0 * Vec3::from(aabb.half_extents).length() * scale;
                        return diameter >= min_diameter;
                    }
                    true
                });
            }
        }
    }
}
