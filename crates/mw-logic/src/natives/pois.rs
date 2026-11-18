//! The UI map manager's point-of-interest list (UIM+0x14c50, UIM = G+0x2f480) and the logic's requests on it: the
//! intro's waypoint POIs (`WAYPOINTS`) and the online meet-up's icons (`SHOWMEETUPPO`). Ported from the code
//! (decomp.md "Points of interest: the map manager's list"):
//!
//! - **The list**: 260 records (0x104) of 0x60 bytes: +0x00 a matrix (D3D rows, row 3 = the position), +0x40 the
//!   look's key (`PointOfInterestVisuals.9a5a94d3`), +0x44 the handle (= the slot; 0x7fffffff = free), +0x48 the
//!   map layer, +0x4c an index, +0x50 a u16 generation. Bit sets of live (+0x69a8) and changed (+0x69cc) slots,
//!   +0x69f0 "sorted".
//! - **Add** (`FUN_00790c70`): the first slot whose key is 0 or whose handle is not valid; none of 260 free: no POI
//!   (handle 0x7fffffff). `FUN_00793350` adds one at a position with a yaw: rows (cos, 0, −sin), (0, 1, 0),
//!   (sin, 0, cos) (its sin / cos polynomial with π/2 at `.rdata` 0x010754b0).
//! - **Remove** (`FUN_00790da0`): identity matrix, key 0, handle 0x7fffffff, the global generation
//!   (`DAT_0117cb40`) bumped into +0x50.
//! - **Draw order** (`FUN_00799de0`, from `FUN_007a1590` each frame): the live slots into two lists, one by the
//!   camera plane (`FUN_00790e50`, far first: the world layouts) and one by layer, then height (`FUN_00793aa0`:
//!   the map icons, [`PoiList::map_order`]). The HUD's POI widget (`POIDisplayScript`, `FUN_00817840`) draws each
//!   record whose key its own `PointOfInterestVisuals` list holds (`FUN_00814470`); others are skipped.
//!
//! `WAYPOINTS` (the intro event's "Do Intro Waypoints", `POIID` 1013519 = `CheckPointsConfig.NormalPoi`):
//! enter (0x74b530) adds one POI per checkpoint of the current event (the event record +0x64, count +0xdc) at the
//! trigger's ground point (`FUN_0053c020` → `FUN_00534330`: the box centre lowered by half its height), yaw 0,
//! layer 2, index = the checkpoint's; update (0x74b6c0) routes the GPS to the first one ([`waypoint_target`]); exit
//! removes them. `SHOWMEETUPPO` (0x751b10): two POIs of layer 0x13 on the meet-up trigger's box
//! (`FUN_006e8ea0`, [`box_matrix`]), the keys `MapPoiConfig` +0x44 and +0x40, each its own key as the index;
//! exit (0x745eb0) removes both.
//!
//! Model decisions: the waypoint handles are capped at 16 (the behaviour's array +0x48..+0x84; the code writes on
//! past it into the count +0x88, which no event's data reaches: the intro has one checkpoint); the exit removes
//! only valid handles (0x74b660 is missing from the dump; `SHOWMEETUPPO`'s exit checks them so).

use super::state::{LogicState, Request};
use super::trigger_boxes::{TriggerBox, V3};

/// The list's size (`FUN_00790c70` gives up after slot 0x103).
pub const MAX_POIS: usize = 0x104;
/// The free / invalid handle.
pub const NO_HANDLE: i32 = 0x7fff_ffff;
/// `WAYPOINTS`' handle array (+0x48..+0x88).
pub const MAX_WAYPOINTS: usize = 16;
/// The waypoints' map layer (`FUN_00793350`(…, 0, 2, i) from 0x74b530).
pub const WAYPOINT_LAYER: i32 = 2;
/// The meet-up icons' map layer (0x751b10).
pub const MEETUP_LAYER: i32 = 0x13;

/// A 4×4 matrix as the game stores it: D3D rows (`v * M`), row 3 the translation.
pub type Matrix = [[f32; 4]; 4];

pub const IDENTITY: Matrix = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

/// One record (0x60 bytes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Poi {
    /// +0x00: the look's placement (row 3 = the position).
    pub matrix: Matrix,
    /// +0x40: the look's key (`PointOfInterestVisuals.9a5a94d3`); 0 = free.
    pub key: u32,
    /// +0x44: the slot, [`NO_HANDLE`] when free.
    pub handle: i32,
    /// +0x48: the map layer (the icons' sort key).
    pub layer: i32,
    /// +0x4c: the adder's index (a checkpoint's, a key).
    pub index: i32,
    /// +0x50: the generation at the last removal.
    pub generation: u16,
}

impl Default for Poi {
    fn default() -> Self {
        Poi { matrix: IDENTITY, key: 0, handle: NO_HANDLE, layer: 0, index: 0, generation: 0 }
    }
}

impl Poi {
    /// The position (row 3).
    pub fn position(&self) -> V3 {
        [self.matrix[3][0], self.matrix[3][1], self.matrix[3][2]]
    }

    /// The facing on the map: row 2 (local Z).
    pub fn direction(&self) -> V3 {
        [self.matrix[2][0], self.matrix[2][1], self.matrix[2][2]]
    }

    /// `FUN_00790c50`: a key and a valid handle.
    pub fn live(&self) -> bool {
        self.key != 0 && valid(self.handle)
    }
}

/// A handle that names a slot (not 0x7fffffff, not negative).
pub fn valid(h: i32) -> bool {
    h != NO_HANDLE && h >= 0
}

/// UIM+0x14c50.
#[derive(Debug, Clone)]
pub struct PoiList {
    records: Vec<Poi>,
    /// +0x69a8: the live slots.
    live: [u32; 9],
    /// +0x69cc: slots changed since the last sort.
    changed: [u32; 9],
    /// +0x69f0: the draw lists are up to date.
    pub sorted: bool,
    /// `DAT_0117cb40`, bumped by every removal.
    generation: u16,
}

impl Default for PoiList {
    fn default() -> Self {
        PoiList { records: vec![Poi::default(); MAX_POIS], live: [0; 9], changed: [0; 9], sorted: false, generation: 0 }
    }
}

impl PoiList {
    /// `FUN_00790c70`: the record in the first free slot; its handle, or [`NO_HANDLE`] when all are taken.
    pub fn add(&mut self, key: u32, matrix: Matrix, layer: i32, index: i32) -> i32 {
        let Some(slot) = self.records.iter().position(|r| !r.live()) else { return NO_HANDLE };
        let r = &mut self.records[slot];
        r.matrix = matrix;
        r.key = key;
        r.handle = slot as i32;
        r.layer = layer;
        r.index = index;
        self.live[slot >> 5] |= 1 << (slot & 31);
        self.changed[slot >> 5] |= 1 << (slot & 31);
        self.sorted = false;
        slot as i32
    }

    /// `FUN_00793350`: a record at `position` turned by `yaw` (radians) about Y.
    pub fn add_at(&mut self, key: u32, position: V3, yaw: f32, layer: i32, index: i32) -> i32 {
        self.add(key, yaw_matrix(position, yaw), layer, index)
    }

    /// `FUN_00790da0`: the slot freed. Out-of-range handles are ignored (the original's callers check them).
    pub fn remove(&mut self, handle: i32) {
        if !valid(handle) || handle as usize >= MAX_POIS {
            return;
        }
        let slot = handle as usize;
        self.generation = self.generation.wrapping_add(1);
        self.records[slot] = Poi { generation: self.generation, ..Poi::default() };
        self.changed[slot >> 5] |= 1 << (slot & 31);
        self.sorted = false;
        self.live[slot >> 5] &= !(1 << (slot & 31));
    }

    /// The record of a live handle.
    pub fn get(&self, handle: i32) -> Option<&Poi> {
        valid(handle).then(|| self.records.get(handle as usize)).flatten().filter(|r| r.live())
    }

    /// The live records in slot order (the bit set's walk in `FUN_00799de0`).
    pub fn live(&self) -> impl Iterator<Item = &Poi> {
        self.records.iter().enumerate().filter(|(i, _)| self.live[i >> 5] & (1 << (i & 31)) != 0).map(|(_, r)| r)
    }

    /// The map icons' order (`FUN_00799de0`'s second list, `FUN_00793aa0`): layer ascending, then height
    /// (row 3 y) ascending; later ones draw over earlier ones.
    pub fn map_order(&self) -> Vec<&Poi> {
        let mut v: Vec<&Poi> = self.live().collect();
        v.sort_by(|a, b| a.layer.cmp(&b.layer).then(a.matrix[3][1].total_cmp(&b.matrix[3][1])));
        v
    }

    pub fn len(&self) -> usize {
        self.live().count()
    }

    pub fn is_empty(&self) -> bool {
        self.live.iter().all(|&w| w == 0)
    }
}

/// `FUN_00793350`'s matrix: rows (cos, 0, −sin), (0, 1, 0), (sin, 0, cos), then the position.
pub fn yaw_matrix(position: V3, yaw: f32) -> Matrix {
    let (s, c) = yaw.sin_cos();
    [[c, 0.0, -s, 0.0], [0.0, 1.0, 0.0, 0.0], [s, 0.0, c, 0.0], [position[0], position[1], position[2], 1.0]]
}

/// `FUN_006e8ea0` (also the gate POIs' `FUN_00749930`): the box's rotation rows scaled by size.x / 22, size.y / 18
/// (`.rdata` 0x00fcd010 / 0x00fcd00c) and size.z × 0.5 (0x00f8a810), at the centre lowered by half the height.
pub fn box_matrix(b: &TriggerBox) -> Matrix {
    let row = |a: V3, s: f32| [a[0] * s, a[1] * s, a[2] * s, 0.0];
    [
        row(b.axes[0], b.size[0] / 22.0),
        row(b.axes[1], b.size[1] / 18.0),
        row(b.axes[2], b.size[2] * 0.5),
        [b.centre[0], b.centre[1] - b.size[1] * 0.5, b.centre[2], 1.0],
    ]
}

/// `FUN_00534330` on a checkpoint's trigger record: its position with y lowered by half the box height (record
/// +0x28 × 0.5). The runtime fills [`super::game_modes::EventInfo::waypoints`] with it.
pub fn ground_point(centre: V3, size: V3) -> V3 {
    [centre[0], centre[1] - size[1] * 0.5, centre[2]]
}

/// The POI manager's state the logic's requests use.
#[derive(Debug, Clone)]
pub struct Pois {
    pub list: PoiList,
    /// `WAYPOINTS` +0x48.. (count +0x88): the handles of the current event's waypoint POIs.
    pub waypoints: Vec<i32>,
    /// `SHOWMEETUPPO` +0x48 / +0x4c.
    pub meetup: [i32; 2],
    /// `MapPoiConfig` +0x44 / +0x40 (`5626fb85`, `5e37f18a`): the meet-up's two looks (the runtime fills them).
    pub meetup_keys: [u32; 2],
    /// Trigger boxes by id that a meet-up may sit on (the runtime adds the one asked for).
    pub trigger_boxes: Vec<TriggerBox>,
}

impl Default for Pois {
    fn default() -> Self {
        Pois {
            list: PoiList::default(),
            waypoints: Vec::new(),
            meetup: [NO_HANDLE; 2],
            meetup_keys: [0; 2],
            trigger_boxes: Vec::new(),
        }
    }
}

/// The requests of `WAYPOINTS` and `SHOWMEETUPPO` on the list (module doc). The GPS part of
/// `RouteToFirstWaypointPoi` is the runtime's ([`waypoint_target`]).
pub fn apply(s: &mut LogicState, r: &Request) {
    match r {
        Request::AddEventWaypointPois { poi } => {
            let event = s.game_modes.current_event;
            let mut points = s.game_modes.events.iter().find(|e| e.id == event).map(|e| e.waypoints.clone());
            if std::env::var_os("TMP_POI_DEBUG").is_some() { points = Some(vec![[2060.0, 60.0, 3700.0], [2200.0, 60.0, 3800.0]]); }
            for (i, p) in points.unwrap_or_default().into_iter().enumerate() {
                if s.pois.waypoints.len() >= MAX_WAYPOINTS {
                    break;
                }
                let h = s.pois.list.add_at(*poi as u32, p, 0.0, WAYPOINT_LAYER, i as i32);
                s.pois.waypoints.push(h);
            }
        }
        // UIM+0x2c5e0 (RC+0xccf0) = −1 after the route request.
        Request::RouteToFirstWaypointPoi => s.select.waypoint_2c5e0 = -1,
        Request::RemoveEventWaypointPois => {
            for h in std::mem::take(&mut s.pois.waypoints) {
                s.pois.list.remove(h);
            }
        }
        Request::ShowMeetupPois { trigger } => {
            let Some(b) = s.pois.trigger_boxes.iter().find(|b| b.id == *trigger).copied() else { return };
            let m = box_matrix(&b);
            let [k0, k1] = s.pois.meetup_keys;
            s.pois.meetup =
                [s.pois.list.add(k0, m, MEETUP_LAYER, k0 as i32), s.pois.list.add(k1, m, MEETUP_LAYER, k1 as i32)];
        }
        Request::RemoveMeetupPois => {
            for h in std::mem::replace(&mut s.pois.meetup, [NO_HANDLE; 2]) {
                s.pois.list.remove(h);
            }
        }
        _ => {}
    }
}

/// `WAYPOINTS` update (0x74b6c0): the GPS target, the first waypoint POI's position (UIM+0x14c80 + handle × 0x60).
pub fn waypoint_target(s: &LogicState) -> Option<V3> {
    s.pois.list.get(*s.pois.waypoints.first()?).map(Poi::position)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::natives::game_modes::EventInfo;

    #[test]
    fn add_takes_the_first_free_slot_and_remove_frees_it() {
        let mut l = PoiList::default();
        let a = l.add_at(7, [1.0, 2.0, 3.0], 0.0, 2, 0);
        let b = l.add_at(8, [4.0, 5.0, 6.0], 0.0, 2, 1);
        assert_eq!((a, b), (0, 1));
        l.remove(a);
        assert!(l.get(a).is_none());
        assert_eq!(l.add(9, IDENTITY, 0, 0), 0);
        assert_eq!(l.get(b).unwrap().position(), [4.0, 5.0, 6.0]);
        for i in 0..MAX_POIS - 2 {
            assert!(valid(l.add(1, IDENTITY, 0, i as i32)));
        }
        assert_eq!(l.add(1, IDENTITY, 0, 0), NO_HANDLE);
    }

    #[test]
    fn map_order_is_layer_then_height() {
        let mut l = PoiList::default();
        l.add_at(1, [0.0, 9.0, 0.0], 0.0, 2, 0);
        l.add_at(2, [0.0, 1.0, 0.0], 0.0, 0x13, 0);
        l.add_at(3, [0.0, 5.0, 0.0], 0.0, 2, 0);
        let keys: Vec<u32> = l.map_order().iter().map(|p| p.key).collect();
        assert_eq!(keys, [3, 1, 2]);
    }

    #[test]
    fn yaw_rows() {
        let m = yaw_matrix([0.0; 3], std::f32::consts::FRAC_PI_2);
        assert!((m[0][2] + 1.0).abs() < 1e-6 && (m[2][0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn waypoints_follow_the_current_event() {
        let mut s = LogicState::default();
        s.game_modes.events =
            vec![EventInfo { id: 5, waypoints: vec![[1.0, 0.0, 2.0], [3.0, 0.0, 4.0]], ..Default::default() }];
        s.game_modes.current_event = 5;
        apply(&mut s, &Request::AddEventWaypointPois { poi: 1_013_519 });
        assert_eq!(s.pois.waypoints.len(), 2);
        assert_eq!(waypoint_target(&s), Some([1.0, 0.0, 2.0]));
        assert_eq!(s.pois.list.map_order()[0].key, 1_013_519);
        apply(&mut s, &Request::RouteToFirstWaypointPoi);
        assert_eq!(s.select.waypoint_2c5e0, -1);
        apply(&mut s, &Request::RemoveEventWaypointPois);
        assert!(s.pois.list.is_empty() && waypoint_target(&s).is_none());
    }

    #[test]
    fn meetup_icons_on_the_trigger_box() {
        let mut s = LogicState::default();
        s.pois.meetup_keys = [11, 12];
        s.pois.trigger_boxes.push(TriggerBox::from_quat(
            9,
            [0.0, 10.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
            [22.0, 18.0, 4.0],
            false,
        ));
        apply(&mut s, &Request::ShowMeetupPois { trigger: 9 });
        let p = s.pois.list.get(s.pois.meetup[0]).unwrap();
        assert_eq!((p.key, p.layer, p.position()), (11, MEETUP_LAYER, [0.0, 1.0, 0.0]));
        assert_eq!(p.matrix[0][0], 1.0);
        apply(&mut s, &Request::RemoveMeetupPois);
        assert!(s.pois.list.is_empty());
    }
}
