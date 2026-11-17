//! Trigger volumes and car boxes as the checkpoint, event-start and car-swap logic reads them: a world trigger box
//! of `TriggerData` (formats.md "TriggerData") and the overlap rule of the trigger manager's per-vehicle hit list
//! (decomp.md "Trigger hits: streaming, overlap and firing"). The original asks `FUN_00537190(vehicle, trigger id,
//! kind)`: is the trigger in the vehicle's hit list, which the overlap job (`FUN_00536a10`) fills through
//! `FUN_00536f70`: spheres always, boxes with flags bit 0 clear always, boxes with bit 0 set only when
//! `dot(vehicle vector, box local Z) ≥ 0` (`DAT_00faa600` = 0.0; a one-way trigger).
//!
//! Model decision (the job's narrow phase was not read): the car's box against the trigger box by separating axes,
//! the car's box centred on its position with the half extents the runtime gives (as `race.rs` /
//! `freeroam.rs` test today). The runtime fills the boxes; no engine physics is needed.

/// A 3-vector (game coordinates: metres, Y up).
pub type V3 = [f32; 3];

pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Squared distance.
pub fn dist2(a: V3, b: V3) -> f32 {
    let d = sub(a, b);
    dot(d, d)
}

/// `v` rotated by the unit quaternion `q` (x, y, z, w).
pub fn rotate(q: [f32; 4], v: V3) -> V3 {
    let [x, y, z, w] = q;
    let t = scale(cross([x, y, z], v), 2.0);
    add(add(v, scale(t, w)), cross([x, y, z], t))
}

/// The identity rotation's rows (local X, Y, Z in world space).
pub const AXES: [V3; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// A trigger box of the world's `TriggerData` (record: id +0xc, kind +0x10, shape +0x11, rotation quaternion +0x14,
/// size +0x24 = the full extents; `FUN_00539e50` builds the shape with half extents = size × 0.5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TriggerBox {
    /// The trigger id (record +0xc).
    pub id: u32,
    /// The box centre (its translation).
    pub centre: V3,
    /// Its rotation's rows: local X (across), Y (up), Z (the box's direction, `FUN_0042f250`).
    pub axes: [V3; 3],
    /// The full extents along the local axes.
    pub size: V3,
    /// Flags bit 0: counts only when crossed along +Z (`FUN_00536f70`).
    pub one_way: bool,
}

impl Default for TriggerBox {
    fn default() -> Self {
        TriggerBox { id: 0, centre: [0.0; 3], axes: AXES, size: [0.0; 3], one_way: false }
    }
}

impl TriggerBox {
    /// A box from its record: centre, rotation quaternion (x, y, z, w) and full size.
    pub fn from_quat(id: u32, centre: V3, q: [f32; 4], size: V3, one_way: bool) -> Self {
        TriggerBox { id, centre, axes: AXES.map(|a| rotate(q, a)), size, one_way }
    }

    /// The route entry point of a checkpoint (`FUN_00645290`): the centre lowered by half the height.
    pub fn ground(&self) -> V3 {
        sub(self.centre, scale(self.axes[1], self.size[1] * 0.5))
    }

    /// The box's direction (local Z, `FUN_0042f250` for a box).
    pub fn direction(&self) -> V3 {
        self.axes[2]
    }
}

/// A vehicle's box as the overlap job queries it (`FUN_00536a10`): centre, rotation rows, half extents, and the
/// vector the one-way test reads (vehicle +0x100 or the body's +0xb0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CarBox {
    pub centre: V3,
    pub axes: [V3; 3],
    pub half: V3,
    pub forward: V3,
}

impl Default for CarBox {
    fn default() -> Self {
        CarBox { centre: [0.0; 3], axes: AXES, half: [1.0, 0.7, 2.3], forward: [0.0, 0.0, 1.0] }
    }
}

impl CarBox {
    /// A car standing level at `centre`, heading along `forward` (y ignored), with `half` extents.
    pub fn level(centre: V3, forward: V3, half: V3) -> Self {
        let f = [forward[0], 0.0, forward[2]];
        let len = dot(f, f).sqrt();
        let z = if len > 1e-6 { scale(f, 1.0 / len) } else { [0.0, 0.0, 1.0] };
        let y = [0.0, 1.0, 0.0];
        let x = cross(y, z);
        CarBox { centre, axes: [x, y, z], half, forward: z }
    }
}

/// `FUN_00537190(vehicle, trigger)`: the trigger is in the vehicle's hit list this frame (`FUN_00536f70`'s rule
/// over the overlap test).
pub fn hits(car: &CarBox, trigger: &TriggerBox) -> bool {
    if trigger.one_way && dot(car.forward, trigger.axes[2]) < 0.0 {
        return false;
    }
    overlaps(car, trigger)
}

/// Box against box by separating axes (model: the job's narrow phase was not read).
pub fn overlaps(car: &CarBox, trigger: &TriggerBox) -> bool {
    let a = trigger.axes;
    let b = car.axes;
    let ha = scale(trigger.size, 0.5);
    let hb = car.half;
    let d = sub(car.centre, trigger.centre);
    let mut axes: Vec<V3> = a.iter().chain(b.iter()).copied().collect();
    for x in &a {
        for y in &b {
            let c = cross(*x, *y);
            let l2 = dot(c, c);
            if l2 > 1e-8 {
                axes.push(scale(c, 1.0 / l2.sqrt()));
            }
        }
    }
    axes.iter().all(|l| {
        let ra = ha[0] * dot(a[0], *l).abs() + ha[1] * dot(a[1], *l).abs() + ha[2] * dot(a[2], *l).abs();
        let rb = hb[0] * dot(b[0], *l).abs() + hb[1] * dot(b[1], *l).abs() + hb[2] * dot(b[2], *l).abs();
        dot(d, *l).abs() <= ra + rb
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_overlap_and_one_way() {
        let gate = TriggerBox { id: 7, centre: [0.0, 2.0, 10.0], size: [20.0, 4.0, 2.0], ..Default::default() };
        let mut car = CarBox::level([0.0, 0.7, 0.0], [0.0, 0.0, 1.0], [1.0, 0.7, 2.3]);
        assert!(!hits(&car, &gate));
        car.centre[2] = 8.0;
        assert!(hits(&car, &gate));
        assert_eq!(gate.ground(), [0.0, 0.0, 10.0]);
        let one_way = TriggerBox { one_way: true, ..gate };
        assert!(hits(&car, &one_way));
        let back = CarBox::level(car.centre, [0.0, 0.0, -1.0], car.half);
        assert!(!hits(&back, &one_way));
        assert!(hits(&back, &gate));
    }

    #[test]
    fn box_from_quaternion() {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        // A quarter turn about +Y: local Z points along world +X.
        let b = TriggerBox::from_quat(1, [0.0; 3], [0.0, h, 0.0, h], [2.0, 2.0, 2.0], false);
        let close = |a: V3, b: V3| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5);
        assert!(close(b.axes[2], [1.0, 0.0, 0.0]), "{:?}", b.axes);
        assert!(close(b.axes[0], [0.0, 0.0, -1.0]), "{:?}", b.axes);
        assert!(close(b.axes[1], [0.0, 1.0, 0.0]));
    }
}
