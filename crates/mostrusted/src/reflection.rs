//! The planar reflection wet roads show (`planarReflectionSampler` in the road shader).
//!
//! The game renders the scene mirrored in a horizontal plane (`planarReflectionViewProjectionModified`,
//! a CPU-set matrix) and the road's wet path samples it at the road pixel's mirrored projection, shifted
//! by the normal's deviation (`WetScaleParams.y`), at mip roughness × `WetScaleParams.z`, scaled by
//! env-BRDF × wet amount. Here a second camera renders the mirrored view into a half-resolution HDR image
//! through the same deferred pipeline (game G-buffer + lighting pass, no game tonemap):
//! - the plane is the ground under the camera (a ray cast down onto the collision mesh, eased);
//! - geometry below the plane is discarded in the reflection view (`PLANAR_REFLECTION_VIEW` in
//!   `layered.wgsl`; the game's "Modified" matrix is probably an oblique near plane doing the same);
//! - the plane height reaches the shaders through the view's `color_grading.hue`, which nothing reads
//!   while Bevy's tonemapping is off (`Tonemapping::None`; the game tonemap in `tonemap.rs` replaces it);
//! - the reflection camera has a `NormalPrepass` (the main camera doesn't), which is the view key bit
//!   `Layered::specialize` uses to compile the clip in for that view only;
//! - no mip chain: the road shader blurs with a few taps by roughness instead (provisional).

use bevy::camera::RenderTarget;
use bevy::camera::visibility::VisibilitySystems;
use bevy::core_pipeline::prepass::{DeferredPrepass, DepthPrepass, NormalPrepass};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::Skybox;
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::ColorGrading;
use bevy::transform::TransformSystems;

use crate::materials::GameEnvironment;

/// Marks the mirrored camera (also in the render world, where `tonemap.rs` skips it).
#[derive(Component, Clone, Copy, ExtractComponent)]
pub struct PlanarReflectionCamera;

/// The reflection plane's height (world y).
#[derive(Resource)]
pub struct PlanarReflection {
    height: Option<f32>,
}

/// What the reflection camera copies from the main camera when it is spawned.
type MainView = (Entity, &'static Projection, Option<&'static Skybox>, Option<&'static bevy::camera::Exposure>);

pub struct PlanarReflectionPlugin {
    pub enabled: bool,
}

impl Plugin for PlanarReflectionPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<PlanarReflectionCamera>::default());
        if self.enabled {
            app.add_systems(PostStartup, spawn).add_systems(
                PostUpdate,
                follow.after(TransformSystems::Propagate).before(VisibilitySystems::UpdateFrusta),
            );
        }
    }
}

/// Half the window, in physical pixels.
fn target_size(window: Option<&Window>) -> UVec2 {
    let s = window.map_or(UVec2::new(1280, 720), |w| w.physical_size());
    (s / 2).max(UVec2::new(64, 64))
}

fn spawn(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    env: Option<ResMut<GameEnvironment>>,
    windows: Query<&Window>,
    main: Query<MainView, crate::MainCamera>,
) {
    let Some((main, projection, skybox, exposure)) = main.iter().next() else { return };
    let size = target_size(windows.iter().next());
    let mut image = Image::new_target_texture(size.x, size.y, TextureFormat::Rgba16Float, None);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        ..ImageSamplerDescriptor::linear()
    });
    let image = images.add(image);
    if let Some(mut env) = env {
        env.planar = Some(image.clone());
    }
    let mut e = commands.spawn((
        Camera3d::default(),
        Camera { order: -1, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
        RenderTarget::Image(image.into()),
        DepthPrepass,
        DeferredPrepass,
        // The view key bit that selects the clip below the plane (`Layered::specialize`).
        NormalPrepass,
        bevy::camera::Hdr,
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        projection.clone(),
        ColorGrading::default(),
        PlanarReflectionCamera,
    ));
    if crate::indirect_drawing_off() {
        e.insert(bevy::render::view::NoIndirectDrawing);
    }
    if let Some(exposure) = exposure {
        e.insert(*exposure);
    }
    if let Some(skybox) = skybox {
        e.insert(skybox.clone());
    }
    commands.entity(main).insert(ColorGrading::default());
    commands.insert_resource(PlanarReflection { height: None });
}

/// Mirror the main camera in the ground plane under it, every frame after transforms are final.
#[allow(clippy::type_complexity)]
fn follow(
    time: Res<Time>,
    spatial: avian3d::prelude::SpatialQuery,
    state: Option<ResMut<PlanarReflection>>,
    mut main: Query<
        (&GlobalTransform, &Projection, &mut ColorGrading),
        (With<Camera3d>, Without<PlanarReflectionCamera>),
    >,
    mut mirror: Query<
        (&mut Transform, &mut GlobalTransform, &mut Projection, &mut ColorGrading),
        With<PlanarReflectionCamera>,
    >,
) {
    let Some(mut state) = state else { return };
    let Some((cam, projection, mut grading)) = main.iter_mut().next() else { return };
    let eye = cam.translation();
    // The ground under the camera: the plane the game would mirror in (not the car or a prop).
    let ground = avian3d::prelude::SpatialQueryFilter::default().with_mask(crate::props::GameLayer::World);
    let hit = spatial.cast_ray(eye, Dir3::NEG_Y, 500.0, true, &ground).map(|h| eye.y - h.distance);
    let target = hit.or(state.height).unwrap_or(eye.y - 1.5);
    let h = match state.height {
        // Ease towards the new ground over ~0.1 s (curbs and bumps would make the reflection jump).
        Some(h) if (target - h).abs() < 20.0 => h + (target - h) * (time.delta_secs() * 10.0).min(1.0),
        _ => target,
    };
    state.height = Some(h);
    grading.global.hue = h;
    let Ok((mut t, mut g, mut p, mut mg)) = mirror.single_mut() else { return };
    // Reflection across y = h: (x, y, z) → (x, 2h − y, z), applied after the camera's own transform.
    let m = Mat4::from_cols(Vec4::X, Vec4::NEG_Y, Vec4::Z, Vec4::new(0.0, 2.0 * h, 0.0, 1.0));
    let mirrored = m * cam.to_matrix();
    *g = GlobalTransform::from(mirrored);
    *t = Transform::from_matrix(mirrored);
    *p = projection.clone();
    mg.global.hue = h;
}
