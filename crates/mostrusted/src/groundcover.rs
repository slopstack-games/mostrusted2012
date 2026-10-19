//! Which groundcover groups are drawn (`loader::Msg::Groundcover` spawns one entity per group).
//!
//! Two tests, both from the game:
//!
//! - **Resident collections.** The game draws groundcover from a list of 64 collection slots
//!   (`FUN_00eb9f50` walks them; `FUN_008d6900` copies the list). A unit lists the collections for its own
//!   ground and for the far hills it sees, placed on *its* backdrop tier of those hills. The slots hold the
//!   **current zone's** collections only: when the streaming zone changes, `FUN_005573f0` removes the old
//!   zone's collections (`FUN_0043ac80`) and adds the new zone's (`FUN_0043ac10`: first free of the 64
//!   `{collection, offset}` slots, count at +0x200), in the same step that swaps the zone's backdrop
//!   (`zone_view.rs`). So only the collections the streaming zone lists are drawn. (Until 2026-10-19 the
//!   zone's `ZoneList` neighbours' collections were drawn too, a model decision: they stand on the
//!   neighbours' backdrop tiers, which the camera zone does not draw, and float: `mwtool placement-check
//!   --floating`, summed over all camera zones, sprites within 300 m more than 1 m above the drawn ground:
//!   397,849 with the neighbours' collections, 3,053 with the zone's own.)
//! - **Distance.** A group is drawn while the camera is within 125 x its radius of its box
//!   (`FUN_00eb9500`). The game also drops sprites behind the camera plane or beyond their own range and
//!   shrinks them near it (`GroundcoverInstance::fade`); `groundcover.wgsl` does that per sprite on the GPU,
//!   with the port of the `Groundcover` vertex program (camera-facing about world up, wind sway).

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use mw_bundle::ResourceId;

/// A groundcover group's collection, box and draw distance.
#[derive(Component)]
pub struct GroundcoverRange {
    pub collection: ResourceId,
    pub min: Vec3,
    pub max: Vec3,
    pub range: f32,
}

/// The zones and which units list which collections (`loader::Msg::GroundcoverListing`).
#[derive(Resource, Default)]
pub struct GroundcoverResidency {
    zones: Option<mw_formats::ZoneList>,
    /// Collections listed per track unit.
    listing: HashMap<u32, Vec<ResourceId>>,
    /// The zone the resident set was built for, and the set.
    current: Option<u32>,
    resident: HashSet<ResourceId>,
    dirty: bool,
}

impl GroundcoverResidency {
    pub fn set_zones(&mut self, zones: mw_formats::ZoneList) {
        self.zones = Some(zones);
        self.dirty = true;
    }

    pub fn add_listing(&mut self, unit: u32, collections: Vec<ResourceId>) {
        self.listing.insert(unit, collections);
        self.dirty = true;
    }

    /// Rebuild the resident set when the camera changed zone or listings arrived. `None` when no zone list
    /// is known (every collection is drawn).
    fn update(&mut self, eye: Vec3, streaming: Option<u32>) -> Option<&HashSet<ResourceId>> {
        let zones = self.zones.as_ref()?;
        let zone = streaming.or_else(|| zones.nearest(eye.x, eye.z).map(|z| z.id))?;
        if self.dirty || self.current != Some(zone) {
            self.dirty = false;
            self.current = Some(zone);
            self.resident = self.listing.get(&zone).into_iter().flatten().copied().collect();
        }
        Some(&self.resident)
    }
}

/// Show the groups of resident collections within their draw distance.
pub fn groundcover_visibility(
    camera: Query<&GlobalTransform, crate::MainCamera>,
    mut residency: ResMut<GroundcoverResidency>,
    zone_view: Res<crate::zone_view::ZoneView>,
    mut groups: Query<(&GroundcoverRange, &mut Visibility)>,
) {
    let Some(eye) = camera.iter().next().map(|t| t.translation()) else { return };
    // The streaming zone (`zone_view.rs`: the player's car's, else the camera's).
    let resident = residency.update(eye, zone_view.current());
    for (g, mut vis) in &mut groups {
        let d = eye.clamp(g.min, g.max).distance(eye);
        let shown = d <= g.range && resident.is_none_or(|r| r.contains(&g.collection));
        vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
}
