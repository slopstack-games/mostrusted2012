//! The bust camera: the camera part of the busted sequence (1014946, `mw_assets::crash::BustCamera`), which HSM
//! 457517 "Do Pursuits" plays when the pursuit ends BUSTED (docs/decomp.md "Bust camera"). The data: three
//! `DynamicCameraSequenceItem`s (0–2 s "Heli Cam", 2–4 s "LineScene", 4–7 s "Heli Cam" moving in from "Heli Cam
//! Move"), each a `Camera.DynamicCamera` whose eye is a `SAS.Analyser[…]` transform of the sequence's
//! `SceneAnalysis` (aliases try their analysers in order), looking at `Players.LocalPlayer.GetTransform` with a
//! `CameraTransformLerpLookAt` tracking; the HUD hidden 0–13 s and the letterbox 0–12 s.
//!
//! Model decisions (the process-node and scene-analyser code is not in the dump; docs/handling.md "Bust camera"):
//! a line-test analyser's point is its bound analyser's point plus `bffd3e8f` metres along its `7c2809fc` rotation
//! (yaw then pitch, in the bound analyser's frame), and the test fails (the alias tries its next analyser) when the
//! world lies in between (`TestWorld`); a nearest-race-car analyser is the player's car with its offset in the car's
//! frame; a road runner walks the road graph `Offset.z` metres from the player; the tracking is the crash camera's
//! (`TrackingSpeed` per 60 Hz tick); the eye of the moving shot lerps from its `SLerpFromBinding` over the item; the
//! FOV stays the gameplay camera's (`CameraFOVZoom` not read); the collider node is not run; the sequence's movie,
//! world-rendering cut and camera take after 6.5 s are not played (the gameplay camera shows from 7 s).
//!
//! `MW_BUST_CAMERA=<s>[,<t>]` starts the camera <s> seconds after launch and holds its clock at <t> (screenshots).

use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;
use mw_assets::crash::{Analyser, BustCamera};

use crate::drive::{Car, Player};

/// The crash camera's tracking model: `TrackingSpeed` is the share of the aim error closed per 60 Hz tick.
const TICK: f32 = 1.0 / 60.0;

#[derive(Resource, Default)]
pub struct BustCam {
    data: Option<Arc<BustCamera>>,
    /// Seconds into the sequence, while it plays.
    t: Option<f32>,
    aim: Option<Vec3>,
    shot: Option<usize>,
    /// `MW_BUST_CAMERA`: when to start, and the sequence time to hold.
    demo_at: Option<f32>,
    demo_hold: Option<f32>,
}

impl BustCam {
    /// The pursuit ended busted: the sequence starts.
    pub fn start(&mut self) {
        if self.data.is_some() {
            self.t = Some(0.0);
            self.aim = None;
            self.shot = None;
        }
    }
}

pub fn load_bust_camera(game: Res<crate::Game>, mut cam: ResMut<BustCam>) {
    match game.store.bust_camera() {
        Ok(c) => {
            info!("bust camera: {} shots, {} analysers", c.shots.len(), c.analysers.len());
            cam.data = Some(Arc::new(c));
        }
        Err(e) => warn!("bust camera: no data ({e})"),
    }
    if let Ok(v) = std::env::var("MW_BUST_CAMERA") {
        let mut it = v.split(',').map(|x| x.trim().parse::<f32>().ok());
        cam.demo_at = it.next().flatten();
        cam.demo_hold = it.next().flatten();
    }
}

struct Scene<'a> {
    data: &'a BustCamera,
    car: &'a Transform,
    graph: Option<&'a mw_formats::AiData>,
    /// Whether the world lies on a segment (origin, direction, length).
    blocked: &'a dyn Fn(Vec3, Dir3, f32) -> bool,
}

impl Scene<'_> {
    /// The first of a binding's analysers that resolves.
    fn binding(&self, binding: &str, depth: u32) -> Option<Transform> {
        if depth > 4 {
            return None;
        }
        self.data.candidates(binding).into_iter().find_map(|a| self.analyser(a, depth + 1))
    }

    fn analyser(&self, a: &Analyser, depth: u32) -> Option<Transform> {
        let offset = Vec3::from(a.translation);
        let kind = a.kind.rsplit('.').next().unwrap_or("");
        match kind {
            "NearestRaceCarSceneAnalyser" => {
                Some(Transform { translation: self.car.transform_point(offset), ..*self.car })
            }
            "LineTestSceneAnalyser" => {
                let from = self.binding(&a.binding, depth)?;
                let [pitch, yaw, _] = a.line_rotation;
                let dir = from.rotation
                    * Quat::from_rotation_y(yaw.to_radians())
                    * Quat::from_rotation_x(pitch.to_radians())
                    * Vec3::Z;
                let to = from.translation + dir * a.length;
                if a.test_world && (self.blocked)(from.translation, Dir3::new(dir).ok()?, a.length) {
                    return None;
                }
                Some(Transform::from_translation(to + from.rotation * offset))
            }
            "RoadRunnerSceneAnalyser" => {
                let graph = self.graph?;
                let f = self.car.rotation * Vec3::Z;
                let back = offset.z < 0.0;
                let dir = if back { -f } else { f };
                let (mut at, mut along) = (self.car.translation, dir);
                crate::police::walk(graph, self.car.translation, Vec2::new(dir.x, dir.z), |n, d, _| {
                    let p = Vec3::from(graph.nodes[n].position);
                    along = (p - at).normalize_or(along);
                    at = p;
                    d < offset.z.abs()
                });
                let side = Vec3::Y.cross(along).normalize_or_zero();
                Some(Transform::from_translation(at + side * offset.x + Vec3::Y * offset.y))
            }
            _ => None,
        }
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn run_bust_camera(
    real: Res<Time<Real>>,
    mut cam: ResMut<BustCam>,
    police: Res<crate::police::Police>,
    mut hud: ResMut<crate::hud::HudSettings>,
    mut letterbox: ResMut<crate::cutscene::LetterboxBars>,
    spatial: SpatialQuery,
    car: Query<&Transform, (With<Player>, With<Car>, Without<Camera3d>)>,
    mut camera: Query<&mut Transform, crate::camera::DrivingCamera>,
) {
    if let Some(at) = cam.demo_at
        && real.elapsed_secs() >= at
    {
        cam.demo_at = None;
        cam.start();
    }
    let (Some(data), Some(t)) = (cam.data.clone(), cam.t) else { return };
    let t = (t + real.delta_secs()).min(cam.demo_hold.unwrap_or(f32::MAX));
    if t >= data.length {
        cam.t = None;
        hud.hidden_by_sequence = false;
        return;
    }
    cam.t = Some(t);
    hud.hidden_by_sequence = data.hide_hud.is_some_and(|(a, b)| t >= a && t < b);
    if let Some(lb) = data.letterbox
        && let Some(out) = lb.state(t)
    {
        letterbox.show(lb.bars, out);
    }
    let Ok(car) = car.single() else { return };
    let Some((index, shot)) = data.shot_at(t) else {
        cam.shot = None;
        return;
    };
    let world = SpatialQueryFilter::default().with_mask(crate::props::GameLayer::World);
    let blocked = |from: Vec3, dir: Dir3, length: f32| spatial.cast_ray(from, dir, length, true, &world).is_some();
    let scene = Scene { data: &data, car, graph: police.graph(), blocked: &blocked };
    let Some(mut eye) = scene.binding(&shot.eye_binding, 0).map(|e| e.translation) else { return };
    if !shot.slerp_binding.is_empty()
        && let Some(from) = scene.binding(&shot.slerp_binding, 0)
    {
        let u = ((t - shot.start) / (shot.end - shot.start).max(1e-3)).clamp(0.0, 1.0);
        eye = from.translation.lerp(eye, u);
    }
    let target = car.translation;
    if cam.shot != Some(index) {
        info!(
            "bust camera: shot {index} at {t:.2} s, eye {eye:.1?} ({}), car {:.1?}",
            shot.eye_binding, car.translation
        );
        cam.shot = Some(index);
        cam.aim = Some(target);
    }
    let speed = shot.tracking.map_or(1.0, |t| t[0]).clamp(0.0, 1.0);
    let ticks = real.delta_secs() / TICK;
    let aim = cam.aim.unwrap_or(target);
    let aim = aim + (target - aim) * (1.0 - (1.0 - speed).powf(ticks));
    cam.aim = Some(aim);
    if let Ok(mut c) = camera.single_mut() {
        *c = Transform::from_translation(eye).looking_at(aim, Vec3::Y);
    }
}
