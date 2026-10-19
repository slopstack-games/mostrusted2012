//! The game's per-view model skip mask (`Model` +0x19, docs/decomp.md "Model pass mask") on our views.
//!
//! Each world view of the original queues its objects with the mask bits it excludes (`FUN_008cffa0` →
//! `FUN_008dbd90`; the object walker `FUN_00538b00`): bit 1 the sun's shadow cascades, 2 the six cube-map
//! faces, 4 the main view, 8 the view at renderer +0x172e0 (the planar reflection: roads, 0x09, are not in it),
//! 0x10 an unidentified fifth view. Here:
//! - bit 4: not in the main camera. The 230 coarse ground stand-ins (mask 0x04) are drawn only in the sun's
//!   shadow cascades and the planar reflection, as in the game; drawn in the main view they cut through roads.
//! - bit 8: not in the planar reflection camera (`reflection.rs`, `--planar-reflection`).
//! - bit 1: casts no shadow (`NotShadowCaster`: the sun's cascades and the headlight's shadow map).
//! - bit 2: we render no cube maps (the environment maps come from the data), nothing to do.
//!
//! Views select by [`RenderLayers`]: layer 0 holds everything without bits 4 / 8 (seen by every view, as
//! before), [`MAIN_ONLY`] the models with bit 8, [`NOT_MAIN`] those with bit 4, [`SHADOW_ONLY`] those with both.
//! The main camera sees 0 and [`MAIN_ONLY`], the planar reflection 0 and [`NOT_MAIN`], the sun all four, spot
//! lights (the headlight's shadow map) 0 and [`MAIN_ONLY`]. A placement's root carries [`PassMask`]; its
//! mesh descendants get the layer and the shadow flag. `MW_ALL_PASSES=1` (loader) sends no masks: every model
//! in every view, the pre-2026-10-16 drawing.
use bevy::camera::visibility::{RenderLayers, VisibilitySystems};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;

/// Models drawn in the main view but not in the planar reflection (bit 8).
pub const MAIN_ONLY: usize = 1;
/// Models drawn in the reflection and the shadows but not in the main view (bit 4).
pub const NOT_MAIN: usize = 2;
/// Models drawn only in the shadow cascades (bits 4 and 8).
pub const SHADOW_ONLY: usize = 3;

/// A placement's `Model` +0x19 pass mask, on its root entity.
#[derive(Component, Clone, Copy, Debug)]
pub struct PassMask(pub u8);

impl PassMask {
    /// The bits this port applies: shadow, main, planar.
    pub const USED: u8 = mw_formats::Model::PASS_SHADOW | mw_formats::Model::PASS_MAIN | mw_formats::Model::PASS_PLANAR;

    fn layers(self) -> Option<RenderLayers> {
        let main = self.0 & mw_formats::Model::PASS_MAIN != 0;
        let planar = self.0 & mw_formats::Model::PASS_PLANAR != 0;
        match (main, planar) {
            (false, false) => None,
            (false, true) => Some(RenderLayers::layer(MAIN_ONLY)),
            (true, false) => Some(RenderLayers::layer(NOT_MAIN)),
            (true, true) => Some(RenderLayers::layer(SHADOW_ONLY)),
        }
    }

    fn casts_shadow(self) -> bool {
        self.0 & mw_formats::Model::PASS_SHADOW == 0
    }

    /// Whether any view we render draws the model (the main view, the shadow cascades or the planar
    /// reflection).
    pub fn drawn(mask: u8) -> bool {
        mask & mw_formats::Model::PASS_MAIN == 0
            || mask & mw_formats::Model::PASS_SHADOW == 0
            || mask & mw_formats::Model::PASS_PLANAR == 0
    }
}

pub struct PassMaskPlugin;

impl Plugin for PassMaskPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            (view_layers, apply)
                .before(VisibilitySystems::VisibilityPropagate)
                .before(VisibilitySystems::CheckVisibility),
        );
    }
}

/// The views' layers, as each camera / light appears.
fn view_layers(
    mut commands: Commands,
    cameras: Query<(Entity, Has<crate::reflection::PlanarReflectionCamera>), Added<Camera3d>>,
    suns: Query<Entity, Added<DirectionalLight>>,
    spots: Query<Entity, Added<SpotLight>>,
) {
    for (e, planar) in &cameras {
        let layers =
            if planar { RenderLayers::from_layers(&[0, NOT_MAIN]) } else { RenderLayers::from_layers(&[0, MAIN_ONLY]) };
        commands.entity(e).insert(layers);
    }
    for e in &suns {
        commands.entity(e).insert(RenderLayers::from_layers(&[0, MAIN_ONLY, NOT_MAIN, SHADOW_ONLY]));
    }
    for e in &spots {
        commands.entity(e).insert(RenderLayers::from_layers(&[0, MAIN_ONLY]));
    }
}

/// Hand a placement's mask down to its meshes (also those spawned later under it).
fn apply(
    mut commands: Commands,
    roots: Query<(Entity, &PassMask), Or<(Added<PassMask>, Changed<Children>)>>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
    mut totals: Local<[usize; 5]>,
) {
    let log = roots.iter().next().is_some() && std::env::var_os("MW_PASS_MASK_LOG").is_some();
    for (root, mask) in &roots {
        let layers = mask.layers();
        let casts = mask.casts_shadow();
        let layer = match (mask.0 & mw_formats::Model::PASS_MAIN != 0, mask.0 & mw_formats::Model::PASS_PLANAR != 0) {
            (false, false) => 0,
            (false, true) => MAIN_ONLY,
            (true, false) => NOT_MAIN,
            (true, true) => SHADOW_ONLY,
        };
        for e in std::iter::once(root).chain(children.iter_descendants(root)) {
            if !meshes.contains(e) {
                continue;
            }
            let mut ec = commands.entity(e);
            if let Some(l) = &layers {
                ec.insert(l.clone());
            }
            if !casts {
                ec.insert(NotShadowCaster);
                totals[4] += 1;
            }
            totals[layer] += 1;
        }
    }
    if log {
        info!(
            "pass masks: meshes on layer 0 / main only / not main / shadow only: {} / {} / {} / {}, no shadow {}",
            totals[0], totals[1], totals[2], totals[3], totals[4]
        );
    }
}
