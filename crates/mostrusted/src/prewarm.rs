//! Pipeline pre-warm at the end of loading.
//!
//! Bevy specialises a mesh entity's pipeline per view (the main camera, its deferred prepass, each shadow
//! cascade) the first time the entity is visible in that view, and keeps the result while the entity lives
//! (`SpecializedMaterialPipelineCache`, `specialize_shadows`: entries go only when the entity or its material
//! changes). So a camera cut to a new area pays for every entity it has not seen yet in one frame: the race's
//! intro shots and the grid cost one 50–190 ms frame each, mostly `specialize_shadows` (22 ms) and
//! `queue_shadows` (11 ms) in the trace (research log 2026-10-10).
//!
//! When loading is done (`Loaded::done`) the cameras' frusta are opened and every shadow caster is put in every
//! cascade for [`FRAMES`] frames, so everything loaded so far is specialised (and its pipelines compiled) in one
//! long frame (~110–190 ms) instead of one hitch per first sight. It runs only where that frame is hidden: in a
//! race before its intro starts (`cutscene.rs` waits for it) and while the front end's loading screen or
//! transition still covers the world; not in plain free roam, where `Loaded::done` arrives while driving and
//! there was no start hitch to remove. Later arrivals (traffic, streamed units) are specialised as before,
//! when first seen. `MW_NO_PREWARM=1` turns it off (A/B).

use std::time::Instant;

use bevy::camera::primitives::Frustum;
use bevy::camera::visibility::{CascadesVisibleEntities, VisibilitySystems};
use bevy::light::NotShadowCaster;
use bevy::math::primitives::HalfSpace;
use bevy::prelude::*;

/// Frames with everything visible: the first specialises what has its render assets, the next ones pick up
/// what was pending (materials / meshes still uploading) and compile.
const FRAMES: u32 = 3;

pub struct PrewarmPlugin;

impl Plugin for PrewarmPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var_os("MW_NO_PREWARM").is_some() {
            return;
        }
        app.init_resource::<Prewarm>().add_systems(Last, advance).add_systems(
            PostUpdate,
            (
                open_frusta.after(VisibilitySystems::UpdateFrusta).before(VisibilitySystems::CheckVisibility),
                fill_cascades.after(crate::shadow_cull::cull_casters),
            ),
        );
    }
}

#[derive(Resource, Default)]
pub struct Prewarm {
    /// Frames of pre-warm left once started.
    left: Option<u32>,
    started: Option<Instant>,
}

impl Prewarm {
    fn active(&self) -> bool {
        self.left.is_some_and(|n| n > 0)
    }

    /// Not run yet or running (a race's intro waits for it).
    pub fn pending(&self) -> bool {
        self.left != Some(0)
    }
}

fn advance(
    mut p: ResMut<Prewarm>,
    loaded: Res<crate::Loaded>,
    frames: Res<bevy::diagnostic::FrameCount>,
    race: Option<Res<crate::race::RaceState>>,
    front: Option<Res<crate::frontend::FrontEnd>>,
) {
    let hidden = race.is_some() || front.is_some_and(|f| f.phase != crate::frontend::Phase::Playing);
    match p.left {
        None if loaded.done && !hidden => p.left = Some(0),
        None if loaded.done => {
            p.left = Some(FRAMES);
            p.started = Some(Instant::now());
            info!("prewarm: start at frame {}", frames.0);
        }
        Some(n) if n > 0 => {
            p.left = Some(n - 1);
            if n == 1 {
                info!("prewarm: done in {:.0?}", p.started.map(|t| t.elapsed()).unwrap_or_default());
            }
        }
        _ => {}
    }
}

/// All-pass frusta on the 3D cameras (a half-space with a zero normal and d = 1 contains every point).
fn open_frusta(p: Res<Prewarm>, mut cameras: Query<&mut Frustum, With<Camera3d>>) {
    if !p.active() {
        return;
    }
    for mut f in &mut cameras {
        f.half_spaces = [HalfSpace::new(Vec4::new(0.0, 0.0, 0.0, 1.0)); 6];
    }
}

/// Every visible shadow caster in every cascade of every light.
#[allow(clippy::type_complexity)]
fn fill_cascades(
    p: Res<Prewarm>,
    mut lights: Query<&mut CascadesVisibleEntities>,
    casters: Query<(Entity, &InheritedVisibility), (With<Mesh3d>, Without<NotShadowCaster>)>,
) {
    if !p.active() {
        return;
    }
    let mut all: Vec<Entity> = casters.iter().filter(|(_, v)| v.get()).map(|(e, _)| e).collect();
    all.sort_unstable();
    for mut visible in &mut lights {
        for lists in visible.entities.values_mut() {
            for list in lists.iter_mut() {
                list.entities.clone_from(&all);
            }
        }
    }
}
