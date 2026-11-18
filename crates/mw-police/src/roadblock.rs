//! Roadblocks: where a roadblock's items stand, ported from the roadblock manager (world +0x2f484; docs/decomp.md
//! "Roadblocks"). The coordinator's task ([`crate::coordinator::RoadblockTask`]) picks a trigger region on the
//! player's route; `FUN_005226b0` posts world message 7 with the region's ground point, its z axis and its width;
//! `FUN_006ab390` → `FUN_0083f030` fills a record and `FUN_00841040` adds it (one at a time: a new record is taken
//! only while none stands). The manager's update (`FUN_00847af0`) then turns the record's direction away from the
//! player (state 0), lays out its layers (`FUN_00844ff0` → `FUN_00841710`, one layer side per frame), spawns the
//! items (`FUN_00846c60`: world objects `FUN_00841390`, AI cop cars `FUN_00845270`) and removes it once the player
//! is beyond `GameLogicConfig` +0x280 (`FUN_008414f0`).

use glam::Vec3;
use mw_assets::police::{RoadblockData, RoadblockItem, RoadblockLayer};

/// `FUN_00845270`: at most 8 cars per roadblock (record +0x9b0).
pub const MAX_CARS: usize = 8;
/// The ground probe of each item (`FUN_00841710`, queued on `DAT_010edb40`): from 4 m above (`.rdata` 0x00ff0390)
/// to 10 m below (0x00ff03a0) the item's point.
pub const PROBE_UP: f32 = 4.0;
pub const PROBE_DOWN: f32 = 10.0;
/// `FUN_008414f0`: the removal distance outside a pursuit is `GameLogicConfig` +0x280 × 0.8 (`.rdata` 0x00feff94).
pub const REMOVAL_OUTSIDE_PURSUIT: f32 = 0.8;

/// The road the layout follows (`FUN_0083fad0` on the AI road graph).
pub trait RoadWalk {
    /// From the road segment nearest `from`, oriented along `dir`, `distance` metres on from the point of that
    /// segment nearest `from`: the point and the direction of the segment it lies on.
    fn walk(&self, from: Vec3, dir: Vec3, distance: f32) -> Option<(Vec3, Vec3)>;
}

/// One item to place: what (`RoadBlockLayer.Item` +0x10), where (before the ground probe), the layer's direction
/// (item record +0x50) and the item's angle in degrees (+0x0c).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub object: u32,
    pub position: Vec3,
    pub dir: Vec3,
    pub angle: f32,
}

/// `FUN_00847af0` state 0: the record's direction is turned to point away from the player.
pub fn facing_away(position: Vec3, dir: Vec3, player: Vec3) -> Vec3 {
    if (position - player).dot(dir) < 0.0 { -dir } else { dir }
}

/// `FUN_008414f0`: the distance beyond which the roadblock goes. `in_pursuit`: `FUN_006c2410` (the pursuit state is
/// chase or cooldown; the level's +0x112 flag is not read).
pub fn removal_distance(data: &RoadblockData, in_pursuit: bool) -> f32 {
    if in_pursuit { data.removal_distance } else { data.removal_distance * REMOVAL_OUTSIDE_PURSUIT }
}

/// `FUN_008414f0` (disassembly 0x8415ce–0x8416ff): the roadblock goes once it lies behind the player (the car's
/// forward axis, `FUN_006ba380` → matrix row 2, against the player-to-roadblock vector: ≤ 0, `.rdata` 0x00ff03b0)
/// and farther than [`removal_distance`]. While it lies ahead within 150 m (0x00fb69cc = 150²) the manager queues a
/// line-of-sight ray to it instead (its result, record +0xfe5, is not read here).
pub fn passed(position: Vec3, player: Vec3, player_forward: Vec3, removal: f32) -> bool {
    let to = position - player;
    player_forward.dot(to) <= 0.0 && to.length_squared() > removal * removal
}

/// `FUN_00841710`: one layer across the road at `centre` facing `dir`, `width` wide (the trigger box's x size),
/// shifted `lateral` metres to the side (0 for every caller).
pub fn layer(layer: &RoadblockLayer, centre: Vec3, dir: Vec3, lateral: f32, width: f32, out: &mut Vec<Placed>) {
    // side = normalize(dir × (0, 1, 0)) (`.rdata` 0x00ff02c0).
    let side = dir.cross(Vec3::Y).normalize_or_zero();
    let c = centre + side * lateral;
    let half2 = (width * 0.5) * (width * 0.5);
    let place = |item: &RoadblockItem, at: Vec3, out: &mut Vec<Placed>| {
        out.push(Placed { object: item.object, position: at, dir, angle: item.angle })
    };
    // State 0: the centre item.
    if layer.centre.object != 0 {
        place(&layer.centre, c, out);
    }
    // States 1 and 2: the left items (last to first) towards −side, the right ones towards +side. The first step is
    // `FirstDistance`, then the k-th item stands (k + 1) × `Distance` out; a side ends at the first item whose outer
    // half-step reaches past half the width.
    for (sign, items) in [(-1.0f32, &layer.left), (1.0, &layer.right)] {
        let n = items.len();
        for k in 0..n {
            let item = if sign < 0.0 { &items[n - 1 - k] } else { &items[k] };
            if item.object == 0 {
                continue;
            }
            let step = if k == 0 { layer.first_distance } else { layer.distance };
            let at = c + side * (sign * (k as f32 + 1.0) * step);
            let far = at + side * (sign * 0.5 * step);
            if far.distance_squared(c) > half2 {
                break;
            }
            place(item, at, out);
        }
    }
}

/// `FUN_00844ff0`: every item of a roadblock definition at `position` (the ground point of the trigger box) facing
/// `dir` (away from the player), `width` wide. The centre layer stands at the point; the definition's back layers
/// (+0x10) follow the road towards the player, the k-th `(k + 1) × LayerDistance` back (`FUN_0083fad0` along −dir,
/// laid out along the walked direction), the front ones (+0x0c, empty in the data) away from it.
pub fn layout(
    data: &RoadblockData,
    definition: u32,
    position: Vec3,
    dir: Vec3,
    width: f32,
    road: &impl RoadWalk,
) -> Vec<Placed> {
    let mut out = Vec::new();
    let Some(def) = data.definitions.get(&definition) else { return out };
    if let Some(l) = data.layers.get(&def.centre) {
        layer(l, position, dir, 0.0, width, &mut out);
    }
    for (rows, along) in [(&def.back, -dir), (&def.front, dir)] {
        for (k, id) in rows.iter().enumerate() {
            let Some(l) = data.layers.get(id) else { continue };
            let Some((at, walked)) = road.walk(position, along, (k as f32 + 1.0) * def.layer_distance) else {
                continue;
            };
            layer(l, at, walked, 0.0, width, &mut out);
        }
    }
    out
}

/// `FUN_0083f120`: an item's frame (left, up, forward rows, the game's car axes) from the layer's direction, the
/// ground normal at its point and its angle: left = −dir, forward = left × up, both turned by the angle about up.
pub fn item_frame(dir: Vec3, up: Vec3, angle_deg: f32) -> (Vec3, Vec3, Vec3) {
    let left = -dir;
    let forward = left.cross(up).normalize_or_zero();
    // `.rdata` 0x00fef66c: degrees to radians; the polynomial pair gives cos (at a) and sin (at a − π/2 through
    // cos, 0x010754b0).
    let (s, c) = angle_deg.to_radians().sin_cos();
    (left * c - forward * s, up, left * s + forward * c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mw_assets::police::{RoadblockDefinition, RoadblockItem};

    struct Straight;
    impl RoadWalk for Straight {
        fn walk(&self, from: Vec3, dir: Vec3, distance: f32) -> Option<(Vec3, Vec3)> {
            Some((from + dir * distance, dir))
        }
    }

    fn item(object: u32, angle: f32) -> RoadblockItem {
        RoadblockItem { object, angle }
    }

    #[test]
    fn a_layer_fills_the_width() {
        // 18433: cars 6 m apart; a 20 m wide box takes the centre car and one each side.
        let l = RoadblockLayer {
            id: 1,
            centre: item(7, 15.0),
            left: vec![item(7, 0.0), item(7, 180.0), item(7, 0.0), item(7, 175.0)],
            right: vec![item(7, 0.0), item(7, 5.0), item(7, 10.0)],
            distance: 6.0,
            first_distance: 6.0,
        };
        let mut out = Vec::new();
        layer(&l, Vec3::ZERO, Vec3::Z, 0.0, 20.0, &mut out);
        assert_eq!(out.len(), 3);
        // dir +Z → side = (−1, 0, 0); the left side takes the last item first.
        assert_eq!(out[1].position, Vec3::new(6.0, 0.0, 0.0));
        assert_eq!(out[1].angle, 175.0);
        assert_eq!(out[2].position, Vec3::new(-6.0, 0.0, 0.0));
        assert_eq!(out[2].angle, 0.0);
        // Gaps (object 0) are skipped without ending the side.
        let gaps = RoadblockLayer { left: vec![item(5, 0.0), item(0, 0.0), item(5, 0.0)], distance: 2.0, ..l };
        let mut out = Vec::new();
        layer(&gaps, Vec3::ZERO, Vec3::Z, 0.0, 20.0, &mut out);
        assert_eq!(out.iter().filter(|p| p.object == 5).count(), 2);
    }

    #[test]
    fn back_layers_follow_the_road_towards_the_player() {
        let mut data = RoadblockData::default();
        data.layers.insert(1, RoadblockLayer { id: 1, centre: item(9, 0.0), ..Default::default() });
        data.layers.insert(2, RoadblockLayer { id: 2, centre: item(8, 0.0), ..Default::default() });
        data.definitions.insert(
            10,
            RoadblockDefinition { id: 10, centre: 1, back: vec![2, 2], layer_distance: 3.0, ..Default::default() },
        );
        let out = layout(&data, 10, Vec3::ZERO, Vec3::Z, 10.0, &Straight);
        let z: Vec<f32> = out.iter().map(|p| p.position.z).collect();
        assert_eq!(z, vec![0.0, -3.0, -6.0]);
        assert_eq!(out[1].dir, -Vec3::Z);
    }

    #[test]
    fn frame() {
        // Angle 0: the item's forward lies across the road; 90° turns it along the road.
        let (l, _, f) = item_frame(Vec3::Z, Vec3::Y, 0.0);
        assert!((l - -Vec3::Z).length() < 1e-6 && (f - Vec3::X).length() < 1e-6);
        let (_, _, f) = item_frame(Vec3::Z, Vec3::Y, 90.0);
        assert!((f - -Vec3::Z).length() < 1e-5, "{f}");
        assert_eq!(facing_away(Vec3::Z * 10.0, -Vec3::Z, Vec3::ZERO), Vec3::Z);
    }
}
