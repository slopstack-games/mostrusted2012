//! Where a wheel's cylinder meets the ground (its contact record).
//!
//! The game's collision detection meets each wheel body's cylinder (wheel object +0x19c: the body's
//! `CylinderVolumes` radius and half length, its axis along the axle) with the world and hands the wheel its
//! contacts (`FUN_00923bb0` → `FUN_009ab3b0` → `FUN_009dd300`: wheel object +0x1a0 / +0x1a4, 0x40 bytes each: the
//! point on the cylinder, the world point, the normal). The wheel takes the first (`FUN_009e04a0` / `FUN_009e06d0`)
//! unless its normal points against the chassis's up axis; its constraint keeps the world point, the normal and the
//! cylinder point's offset from the wheel centre (`FUN_0095b4d0`, `FUN_009c5a90`). In snapshot snap1 (XKR on a flat
//! road) each wheel has one contact, the cylinder point on the rim of its lower end. So on flat ground the contact
//! is the cylinder's lowest point and the face's normal; on a kerb's edge it is the edge, with the normal from the
//! edge towards the axle, and the wheel rides up over √(2 r h) where a ray under the centre meets the step at once.
//!
//! The narrow phase itself runs in a job the dump lacks. This one finds, per triangle, how high the wheel's centre
//! must stand on its strut (along `up`) for the cylinder to rest on the triangle, by cutting the cylinder into
//! slices across the axle (circles in planes normal to the axle) and solving each circle against the triangle's
//! cut (a segment) exactly. The caller takes the highest over all nearby triangles: the ground the cylinder meets
//! first coming down the strut.
//!
//! The normal is the triangle's own, also where the cylinder touches the triangle's edge: the original's wheel
//! contacts read live (`ref/kerb_drop2`, F-150 Raptor at 31–34 mph over two 0.10–0.13 m steps up and a 0.11 m
//! drop) keep a vertical normal (y ≥ 0.995) through every step while the contact point moves up or down the edge
//! over a tick or two (one tick on a step up read 0.044 m of the 0.10 m), so the wheel is lifted by the edge's
//! growing depth in the cylinder rather than pushed back along a radial normal. A radial normal in ours doubled
//! the pitch and roll rates over those steps against the capture.

/// Slices across the cylinder's length, from one rim to the other (an odd count: the middle one is the centre).
const SLICES: usize = 9;
/// Triangles whose normal is within this of the axle (walls along the wheel) are not met by the slices.
const AXIAL_NORMAL_MAX: f64 = 0.7;
/// Heights within this (m) count as a tie.
const TIE: f64 = 1e-4;

type V = [f64; 3];

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V, s: f64) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn normalize(a: V) -> Option<V> {
    let l = dot(a, a).sqrt();
    (l > 1e-12).then(|| scale(a, 1.0 / l))
}

/// The wheel's cylinder, in coordinates relative to a reference point on its strut (the caller's origin).
#[derive(Debug, Clone, Copy)]
pub struct Wheel {
    /// The axle (unit).
    pub axis: V,
    /// The strut's direction, up (unit, normal to the axle): the wheel moves along it.
    pub up: V,
    pub radius: f64,
    pub half_length: f64,
}

/// The cylinder resting on a triangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rest {
    /// Where the wheel's centre stands along `up` (from the origin) when it touches.
    pub height: f64,
    /// The touching point (on the triangle; from the origin).
    pub point: V,
    /// The contact normal, out of the ground towards the wheel (unit).
    pub normal: V,
    /// The touching point's offset from the wheel's centre (the contact record's +0x20).
    pub offset: V,
    /// On a triangle edge rather than inside its face.
    pub edge: bool,
    /// The caller's tag of the triangle (which collider it belongs to).
    pub tag: u64,
}

impl Rest {
    /// The normal's component along the wheel's `up`.
    pub fn normal_up(&self, wheel: &Wheel) -> f64 {
        dot(self.normal, wheel.up)
    }
}

/// The ground under the cylinder slice by slice: per slice the highest rest over the triangles added.
#[derive(Debug, Clone)]
pub struct Profile {
    wheel: Wheel,
    slices: [Option<Rest>; SLICES],
    /// Ground above this (along `up`, from the origin) is not under the wheel; rests below `lowest` are out of reach.
    highest: f64,
    lowest: f64,
}

impl Profile {
    pub fn new(wheel: Wheel) -> Self {
        Self { wheel, slices: [None; SLICES], highest: f64::INFINITY, lowest: f64::NEG_INFINITY }
    }

    /// Only ground no higher than `highest` and rests no lower than `lowest` count.
    pub fn with_limits(mut self, highest: f64, lowest: f64) -> Self {
        (self.highest, self.lowest) = (highest, lowest);
        self
    }

    fn t(&self, k: usize) -> f64 {
        self.wheel.half_length * (2.0 * k as f64 / (SLICES - 1) as f64 - 1.0)
    }

    /// Meets one triangle (vertices relative to the origin) with every slice. Triangles running along the axle
    /// (walls beside the tyre) are not met.
    pub fn add(&mut self, tri: &[V; 3], tag: u64) {
        let (a, v, r) = (self.wheel.axis, self.wheel.up, self.wheel.radius);
        let w = cross(v, a);
        let Some(face) = normalize(cross(sub(tri[1], tri[0]), sub(tri[2], tri[0]))) else { return };
        if dot(face, a).abs() > AXIAL_NORMAL_MAX {
            return;
        }
        let face = if dot(face, v) < 0.0 { scale(face, -1.0) } else { face };
        for k in 0..SLICES {
            let t = self.t(k);
            let Some(cut) = slice(tri, a, t) else { continue };
            let p: [(f64, f64); 2] = cut.map(|q| (dot(q, w), dot(q, v)));
            let Some((x, y, f, interior)) = circle_on_segment(p[0], p[1], r) else { continue };
            if y > self.highest || f < self.lowest {
                continue;
            }
            // A tie (a kerb's top and its upright face meeting at their shared edge) goes to the more upward normal.
            if self.slices[k]
                .is_some_and(|b| f < b.height - TIE || (f < b.height + TIE && dot(face, v) <= dot(b.normal, v)))
            {
                continue;
            }
            let point = add(scale(a, t), add(scale(w, x), scale(v, y)));
            self.slices[k] =
                Some(Rest { height: f, point, normal: face, offset: sub(point, scale(v, f)), edge: !interior, tag });
        }
    }

    /// The contact: the highest slice's rest (a tie to the slice nearest the middle), the ground the cylinder
    /// meets first coming down its strut. A kerb's edge under the outer part of the tyre lifts the whole wheel; a
    /// penetration-depth contact would push such a wheel aside instead, but ours only moves along `up`, and passing
    /// such edges over until the overlap outgrew the lift (tried) made the lift come at once later: worse against
    /// the captures (handling.md "Model decisions").
    pub fn resolve(&self) -> Option<Rest> {
        let mid = SLICES / 2;
        let mut out: Option<(usize, Rest)> = None;
        for (k, r) in self.slices.iter().enumerate() {
            let Some(r) = *r else { continue };
            let better = out.is_none_or(|(j, b)| {
                r.height > b.height + TIE || (r.height > b.height - TIE && k.abs_diff(mid) < j.abs_diff(mid))
            });
            if better {
                out = Some((k, r));
            }
        }
        out.map(|(_, r)| r)
    }
}

/// The cylinder centred on the `up` line through the origin resting on one triangle (vertices relative to the
/// origin), or `None` when it cannot touch it (the triangle is beyond the cylinder's reach across or along the
/// axle, or runs along the axle like a wall beside the tyre).
pub fn rest_on(wheel: &Wheel, tri: &[V; 3]) -> Option<Rest> {
    let mut p = Profile::new(*wheel);
    p.add(tri, 0);
    p.resolve()
}

/// The segment where the plane through `a t` normal to `a` cuts the triangle.
fn slice(tri: &[V; 3], a: V, t: f64) -> Option<[V; 2]> {
    let d = tri.map(|p| dot(p, a) - t);
    if d.iter().all(|&x| x.abs() < 1e-9) {
        return None;
    }
    let mut pts: Vec<V> = Vec::with_capacity(3);
    for i in 0..3 {
        let j = (i + 1) % 3;
        if d[i].abs() < 1e-9 {
            pts.push(tri[i]);
        } else if d[i] * d[j] < 0.0 {
            let k = d[i] / (d[i] - d[j]);
            pts.push(add(tri[i], scale(sub(tri[j], tri[i]), k)));
        }
    }
    match pts.len() {
        0 => None,
        1 => Some([pts[0], pts[0]]),
        _ => Some([pts[0], pts[1]]),
    }
}

/// A circle of radius `r` coming down a vertical line (x = 0) onto the segment p0–p1 (2D, y up): the highest centre
/// height at which it touches, maximising f(x) = y(x) + √(r² − x²) over the part of the segment within |x| ≤ r.
/// Returns (x, y) of the touching point, f, and whether the point is inside the segment (a tangent contact on its
/// face) rather than at one of its ends.
fn circle_on_segment(p0: (f64, f64), p1: (f64, f64), r: f64) -> Option<(f64, f64, f64, bool)> {
    let ((x0, y0), (x1, y1)) = if p0.0 <= p1.0 { (p0, p1) } else { (p1, p0) };
    let (lo, hi) = (x0.max(-r), x1.min(r));
    if lo > hi {
        return None;
    }
    let root = |x: f64| (r * r - x * x).max(0.0).sqrt();
    if x1 - x0 < 1e-9 {
        // Along the line of descent: its top end.
        let y = y0.max(y1);
        return Some((x0, y, y + root(x0), false));
    }
    let m = (y1 - y0) / (x1 - x0);
    let y_at = |x: f64| y0 + m * (x - x0);
    // f is concave: its maximum is where the circle's tangent is parallel to the segment, else at a bound.
    let tangent = m * r / (1.0 + m * m).sqrt();
    let (x, interior) = if tangent > lo && tangent < hi {
        (tangent, true)
    } else if tangent <= lo {
        (lo, false)
    } else {
        (hi, false)
    };
    let y = y_at(x);
    Some((x, y, y + root(x), interior))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wheel() -> Wheel {
        Wheel { axis: [1.0, 0.0, 0.0], up: [0.0, 1.0, 0.0], radius: 0.34, half_length: 0.12 }
    }

    /// A big flat triangle under the wheel: the centre rests one radius above it, face normal, middle slice.
    #[test]
    fn flat_ground() {
        let tri = [[-5.0, -1.0, -5.0], [5.0, -1.0, -5.0], [0.0, -1.0, 5.0]];
        let r = rest_on(&wheel(), &tri).unwrap();
        assert!((r.height - (-1.0 + 0.34)).abs() < 1e-9, "{r:?}");
        assert!(!r.edge && (r.normal[1] - 1.0).abs() < 1e-9);
        assert!((r.offset[1] + 0.34).abs() < 1e-9 && r.offset[0].abs() < 1e-9 && r.offset[2].abs() < 1e-9);
    }

    /// A kerb's top face (y = h, starting at z = z0 ahead): the wheel meets its edge with a radial normal and
    /// rides up over √(2 r h) − the distance across the circle's reach.
    #[test]
    fn kerb_edge() {
        let (h, z0) = (0.168, 0.2);
        let tri = [[-5.0, h, z0], [5.0, h, z0], [0.0, h, z0 + 5.0]];
        let r = rest_on(&wheel(), &tri).unwrap();
        let want = h + (0.34f64 * 0.34 - z0 * z0).sqrt();
        assert!((r.height - want).abs() < 1e-9 && r.edge, "{r:?}");
        // The normal is the top face's (the original keeps it vertical on a kerb's edge).
        assert!((r.normal[1] - 1.0).abs() < 1e-9);
        // Out of reach ahead: nothing.
        let far = [[-5.0, h, 0.35], [5.0, h, 0.35], [0.0, h, 5.0]];
        assert!(rest_on(&wheel(), &far).is_none());
    }

    /// Ground tilted across the axle: the contact is on the lower rim with the face's normal.
    #[test]
    fn cambered_ground() {
        let k: f64 = 0.05; // y = -1 + k x
        let tri = [[-5.0, -1.0 - 5.0 * k, -5.0], [5.0, -1.0 + 5.0 * k, -5.0], [0.0, -1.0, 5.0]];
        let r = rest_on(&wheel(), &tri).unwrap();
        assert!((r.point[0] - 0.12).abs() < 1e-9, "{r:?}");
        assert!((r.height - (-1.0 + k * 0.12 + 0.34)).abs() < 1e-9);
        assert!(!r.edge && r.normal[0] < 0.0);
    }

    /// A wall along the wheel is not met; a sloped face ahead is met where the circle's tangent matches it.
    #[test]
    fn wall_and_slope() {
        let wall = [[0.1, -1.0, -5.0], [0.1, 1.0, -5.0], [0.1, 0.0, 5.0]];
        assert!(rest_on(&wheel(), &wall).is_none());
        // y = -1 + 0.2 z: the tangent point is at z = 0.2 r / √1.04.
        let slope = [[-5.0, -2.0, -5.0], [5.0, -2.0, -5.0], [0.0, 0.0, 5.0]];
        let r = rest_on(&wheel(), &slope).unwrap();
        assert!(!r.edge && (r.point[2] - 0.2 * 0.34 / 1.04f64.sqrt()).abs() < 1e-9, "{r:?}");
    }
}
