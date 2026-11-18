//! The map manager's point-of-interest list on the runtime's side. The list itself and the logic's requests on it
//! (`WAYPOINTS`, `SHOWMEETUPPO`) are `mw_logic::natives::pois` (decomp.md "Points of interest: the map manager's
//! list"); here: the copy the HUD and the world map draw ([`Pois`], refreshed by `logic.rs` every frame in the
//! map order of `FUN_00799de0`), the meet-up trigger's box read on demand, and the GPS target of
//! `RouteToFirstWaypointPoi` (`logic.rs` → `logic_effects::Effect::RouteTo`).
//!
//! Each screen's POI widget (`POIDisplayScript`, `FUN_00817840` → `FUN_008162a0`) draws a record only when its own
//! `PointOfInterestVisuals` list holds the record's key (`FUN_00814470`): the HUD's minimap (`hud.rs`) draws the
//! looks of the free-roam and race HUD screens, the world map (`map_screen.rs`) the map screen's. So the intro's
//! waypoints (key 1013519, a HUD look only) show on the minimap and not on the world map, the online meet-up's
//! (1013503 / 1013504, map-screen looks) on the world map.
//!
//! Model decisions: the POIs that the runtime's own systems draw directly (the race gates, the event starts, the
//! free-roam collectables, the racers and cops) do not go through this list yet; the records' world layouts
//! (`FUN_00816520`, sorted by the camera plane) are not drawn.

use bevy::prelude::*;
use mw_logic::natives::LogicState;
use mw_logic::natives::pois::PoiList;
use mw_logic::natives::trigger_boxes::TriggerBox;

pub struct PoiPlugin;

impl Plugin for PoiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pois>();
    }
}

/// One live record as drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoiIcon {
    /// The look's key (`PointOfInterestVisuals.9a5a94d3`).
    pub key: u32,
    /// Row 3 of the record's matrix.
    pub position: Vec3,
    /// Row 2 (the icon's facing on the map).
    pub direction: Vec3,
}

/// The live records in the map order (layer, then height: `FUN_00793aa0`).
#[derive(Resource, Default, Debug)]
pub struct Pois {
    pub icons: Vec<PoiIcon>,
}

impl Pois {
    /// Copy the logic's list (`logic.rs`, after the machine's update).
    pub fn sync(&mut self, list: &PoiList) {
        let icons: Vec<PoiIcon> = list
            .map_order()
            .into_iter()
            .map(|p| PoiIcon { key: p.key, position: Vec3::from(p.position()), direction: Vec3::from(p.direction()) })
            .collect();
        if icons != self.icons {
            self.icons = icons;
        }
    }
}

/// `SHOWMEETUPPO`'s trigger (`FUN_006e8ea0` resolves it with `FUN_0053c020`): its box from the world's
/// `TriggerData`, added to the logic's boxes before the request is applied. Online only (the offline game never
/// asks).
pub fn provide_trigger(st: &mut LogicState, store: &mw_assets::AssetStore, id: u32) {
    if st.pois.trigger_boxes.iter().any(|b| b.id == id) {
        return;
    }
    let found = store.trigger_data().ok().and_then(|td| {
        td.triggers.iter().find(|t| t.id == id).and_then(|t| match t.shape {
            mw_formats::TriggerShape::Box { rotation, size } => {
                Some(TriggerBox::from_quat(t.id, t.position, rotation, size, t.flags & 1 != 0))
            }
            _ => None,
        })
    });
    match found {
        Some(b) => st.pois.trigger_boxes.push(b),
        None => warn!("POI: meet-up trigger {id} is not a box of the world's triggers"),
    }
}
