//! Which groundcover groups are drawn (`loader::Msg::Groundcover` spawns one entity per group).
//!
//! Two tests, both from the game:
//!
//! - **Resident collections.** The game draws groundcover from a list of 64 collection slots
//!   (`FUN_00eb9f50` walks them; `FUN_008d6900` copies the list), filled from the track units it has loaded.
//!   A unit lists the collections for its own ground and for the far hills it sees, placed on *its*
//!   backdrop tier of those hills. Here every unit is loaded at once, so a collection listed only by units
//!   kilometres away would stand on a terrain tier that the composed backdrop (`mw_assets::backdrop`) does
//!   not draw at that place: it floats ("floating grass", CONTINENTAL DRIFT start, 2026-10-09). So only the
//!   collections listed by the camera's zone and its `ZoneList` neighbours are drawn: the streaming set
//!   the PVS describes. That set holds at most 63 distinct collections in every zone of the map (`mwtool
//!   placement-check --owners`), which fits the 64 slots; the residency rule itself (which units the game
//!   keeps loaded) is not traced in the code (model decision).
//! - **Distance.** A group is drawn while the camera is within 125 x its radius of its box
//!   (`FUN_00eb9500`). The game also drops and shrinks single sprites near their own range
//!   (`GroundcoverInstance::fade`); merged group meshes cannot do that.

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
    fn update(&mut self, eye: Vec3) -> Option<&HashSet<ResourceId>> {
        let zones = self.zones.as_ref()?;
        let zone = zones.nearest(eye.x, eye.z)?;
        if self.dirty || self.current != Some(zone.id) {
            self.dirty = false;
            self.current = Some(zone.id);
            let units = std::iter::once(zone.id).chain(zone.neighbours.iter().map(|&(i, _)| zones.zones[i].id));
            self.resident = units.flat_map(|u| self.listing.get(&u).into_iter().flatten().copied()).collect();
        }
        Some(&self.resident)
    }
}

/// Show the groups of resident collections within their draw distance.
pub fn groundcover_visibility(
    camera: Query<&GlobalTransform, crate::MainCamera>,
    mut residency: ResMut<GroundcoverResidency>,
    mut groups: Query<(&GroundcoverRange, &mut Visibility)>,
) {
    let Some(eye) = camera.iter().next().map(|t| t.translation()) else { return };
    let resident = residency.update(eye);
    for (g, mut vis) in &mut groups {
        let d = eye.clamp(g.min, g.max).distance(eye);
        let shown = d <= g.range && resident.is_none_or(|r| r.contains(&g.collection));
        vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
}
