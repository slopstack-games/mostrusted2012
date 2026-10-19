//! What the world shows around the streaming zone, as the game swaps it (docs/decomp.md "Streaming zone and
//! backdrop swap").
//!
//! The game streams by `ZoneList` zone. `FUN_00549fb0` builds the stream list for the focus: the zone holding it
//! (`FUN_00548ab0`), then **all** of that zone's neighbours (either neighbour flag), ranked by a second vector's
//! direction. `FUN_005573f0` requests those units and, when the current zone changes and its unit is loaded,
//! removes the old zone's backdrop instances from the culling structure (`FUN_00556a60`) and inserts the new
//! zone's (`FUN_00556980` walks the `InstanceList` entries from the always-loaded count to the end, through
//! `FUN_00553810`); in the same step it swaps the groundcover collection slots (`FUN_0043ac80` /
//! `FUN_0043ac10`, `groundcover.rs`).
//!
//! So with the focus in zone Z the game draws the main geometry of Z and its neighbours and only Z's backdrop.
//! Here every unit is loaded at once; the loader tags each placement's root ([`ZoneTag`]) and this module shows
//! it by that rule: a unit's main placements while the unit is Z or a neighbour of Z, a backdrop model while Z
//! lists it. Before (`--composed-backdrop`) one backdrop was composed per place from the zone owning it, which
//! drops the ground a neighbour's view shows over a zone whose own view never sees it: the hill over the tunnel
//! on the road west to the airfield is drawn only by the backdrop of the zone outside the tunnel, so its trees
//! and groundcover floated (research log 2026-10-19).
//!
//! A zone's backdrop is what can be seen from the ground in that zone. A free camera high above the ground sees
//! past it (holes to the horizon), so without a player car and with the camera more than [`AERIAL`] above the
//! collision under it, the composed backdrop and every unit's main geometry are shown instead.
//!
//! Model decisions: the focus is the player's car, else the camera (the caller of `FUN_005586e0` passes a
//! position and a direction, not traced; the direction's use, prefetch order, does not matter when all is
//! loaded); outside every zone polygon the nearest zone counts (the game keeps the last zone it found); the
//! switch is immediate (the game waits for the new zone's unit to be loaded); props with physics (`props.rs`)
//! and collision stay everywhere; the aerial fallback and its height are ours.

use std::collections::HashSet;
use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;

/// A free camera this far above the collision under it (or with none within [`AERIAL_PROBE`]) sees the
/// composed view (model decision; the game's focus is always a car on the ground).
pub const AERIAL: f32 = 25.0;
const AERIAL_PROBE: f32 = 2000.0;

/// Which streaming zones show a placement root.
#[derive(Component, Clone, Debug)]
pub enum ZoneTag {
    /// A track unit's main placement: shown while the unit is the focus zone or one of its neighbours.
    Unit(u32),
    /// A backdrop model, whole: shown while the focus zone is one of the units listing it.
    Backdrop(Arc<[u32]>),
    /// A backdrop model cut down to the per-place composition (`mw_assets::backdrop`): the aerial view.
    Composed,
}

#[derive(Resource, Default)]
pub struct ZoneView {
    zones: Option<mw_formats::ZoneList>,
    /// The focus zone and the units streamed with it.
    current: Option<u32>,
    resident: HashSet<u32>,
    aerial: bool,
}

impl ZoneView {
    pub fn set_zones(&mut self, zones: mw_formats::ZoneList) {
        self.zones = Some(zones);
        self.current = None;
    }

    /// The focus zone (`None` before the zones are known).
    pub fn current(&self) -> Option<u32> {
        self.current
    }

    /// Track the focus zone and the aerial state; true when either changed.
    fn update(&mut self, focus: Vec3, aerial: bool) -> bool {
        let Some(zones) = &self.zones else { return false };
        let Some(zone) = zones.nearest(focus.x, focus.z) else { return false };
        if self.current == Some(zone.id) && self.aerial == aerial {
            return false;
        }
        self.current = Some(zone.id);
        self.aerial = aerial;
        self.resident =
            std::iter::once(zone.id).chain(zone.neighbours.iter().map(|&(i, _)| zones.zones[i].id)).collect();
        true
    }

    fn shows(&self, tag: &ZoneTag) -> bool {
        let Some(current) = self.current.filter(|_| !self.aerial) else {
            return !matches!(tag, ZoneTag::Backdrop(_));
        };
        match tag {
            ZoneTag::Unit(u) => self.resident.contains(u),
            ZoneTag::Backdrop(units) => units.contains(&current),
            ZoneTag::Composed => false,
        }
    }
}

/// Show the tagged roots the focus zone draws; runs over all of them when the zone changes, otherwise only over
/// newly spawned ones.
pub fn zone_view_visibility(
    camera: Query<&GlobalTransform, crate::MainCamera>,
    player: Query<&GlobalTransform, (With<crate::drive::Player>, With<crate::drive::Car>)>,
    spatial: SpatialQuery,
    mut view: ResMut<ZoneView>,
    mut roots: ParamSet<(Query<(&ZoneTag, &mut Visibility)>, Query<(&ZoneTag, &mut Visibility), Added<ZoneTag>>)>,
) {
    if view.zones.is_none() {
        return;
    }
    let car = player.iter().next().map(|t| t.translation());
    let Some(focus) = car.or_else(|| camera.iter().next().map(|t| t.translation())) else { return };
    let aerial = car.is_none() && {
        let filter = SpatialQueryFilter::from_mask(crate::props::GameLayer::World);
        spatial.cast_ray(focus, Dir3::NEG_Y, AERIAL_PROBE, true, &filter).is_none_or(|hit| hit.distance > AERIAL)
    };
    let apply = |tag: &ZoneTag, vis: &mut Visibility, view: &ZoneView| {
        let want = if view.shows(tag) { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
    };
    if view.update(focus, aerial) {
        if let Some(z) = view.current {
            info!(
                "streaming zone {z}{}: {} units",
                if view.aerial { " (aerial view)" } else { "" },
                view.resident.len()
            );
        }
        for (tag, mut vis) in &mut roots.p0() {
            apply(tag, &mut vis, &view);
        }
    } else {
        for (tag, mut vis) in &mut roots.p1() {
            apply(tag, &mut vis, &view);
        }
    }
}
