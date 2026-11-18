//! A ray down onto the world's collision that also works while the game clock is paused.
//!
//! avian enters a new collider into its query trees with the AABB its physics step computes, and the step does not
//! run while `Time<Virtual>` is paused. The logic pauses the sim around an event's start (`PAUSESIM`, the offline
//! pause states) while the start area streams in and the car and its opponents are placed, so a spatial-query ray
//! sees none of the ground that arrived during the pause. While paused, a ray that misses is cast again against
//! each world collider's own shape through its transform (no tree). The game itself places on residency alone
//! (`FUN_0064b290`); the ground test is ours, so a car is never put down before the ground under it exists. It
//! holds only while the sim runs: the start area's collision streams around the car, which is still elsewhere
//! until it is placed, so while paused the callers place on residency alone as the game does (nothing falls before
//! the sim resumes, and `streaming::streaming_stall` pauses it again while the car's surroundings are not in).

use avian3d::parry::query::Ray;
use avian3d::prelude::*;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::props::GameLayer;

#[derive(SystemParam)]
pub struct GroundRay<'w, 's> {
    spatial: SpatialQuery<'w, 's>,
    sim: Res<'w, Time<Virtual>>,
    colliders: Query<'w, 's, (&'static Collider, Option<&'static GlobalTransform>, Option<&'static CollisionLayers>)>,
}

impl GroundRay<'_, '_> {
    /// The game clock is paused (the logic's `PAUSESIM` / offline pause states, the pause menu, a streaming stall).
    pub fn paused(&self) -> bool {
        self.sim.is_paused()
    }

    /// The distance from `origin` straight down to the world's collision, within `max`.
    pub fn down(&self, origin: Vec3, max: f32) -> Option<f32> {
        let world = SpatialQueryFilter::default().with_mask(GameLayer::World);
        if let Some(hit) = self.spatial.cast_ray(origin, Dir3::NEG_Y, max, true, &world) {
            return Some(hit.distance);
        }
        if !self.paused() {
            return None;
        }
        let mut best: Option<f32> = None;
        for (collider, transform, layers) in &self.colliders {
            // No layers: world geometry (`GameLayer`'s default).
            if layers.is_some_and(|l| !l.memberships.has_all(GameLayer::World)) {
                continue;
            }
            // In the collider's own space (scale included, so the unscaled shape); the ray parameter stays in
            // world metres because the map is affine and the world direction is a unit vector. The world's
            // collision has no transform (its triangles are in world space, `loader.rs`).
            let inverse = transform.map_or(bevy::math::Affine3A::IDENTITY, |t| t.affine().inverse());
            let o = inverse.transform_point3(origin);
            let d = inverse.transform_vector3(Vec3::NEG_Y);
            let ray = Ray::new(o.to_array().into(), d.to_array().into());
            if let Some(t) = collider.shape().cast_local_ray(&ray, max, true) {
                best = Some(best.map_or(t, |b| b.min(t)));
            }
        }
        best
    }
}
