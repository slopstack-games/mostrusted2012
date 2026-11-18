//! The suspension's force elements and the wheel's ground contact as the game's island solver runs them
//! (docs/decomp.md "Suspension, wheels, aerodynamics"). Each wheel is a point mass (the wheel body's data mass,
//! wheel record +0x50) tied to the chassis by one spring-damper element (built by `FUN_00933930`, element type 0)
//! between a chassis anchor (element +0x20) and the wheel centre, and held off the ground by the wheel
//! constraint's contact. Per 60 Hz tick the island step (`FUN_0093f5a0`) runs:
//!
//! 1. two contact passes (`FUN_00932190` → `FUN_009c5a90`): the wheel's velocity is raised along the ground
//!    normal by the predicted penetration / dt ([`contact_correction`]), and the strut stop moves the chassis
//!    ([`stop_excess`]);
//! 2. positions are integrated (`FUN_009324f0`: bodies and wheel centres, x += v dt);
//! 3. two sub-steps at dt / 2 (`FUN_0093f200`): the contact's impulse pass (`FUN_009c2670`: the wheel's approach to
//!    the ground is stopped, never pulled; the strut stop's share on the chassis), then the force elements
//!    (`FUN_00932f90`): every damper ([`damper_impulse`], `FUN_0092f160`), every spring ([`spring_impulse`],
//!    `FUN_0092f510`), the anti-roll bars (`FUN_0092ee70`), then gravity (`FUN_009282d0`, [`wheel_gravity`]);
//! 4. the tyres, once.
//!
//! Everything here is scalar along the element's line n = (wheel centre − anchor) / |…|, with the relative
//! velocity v_rel = (v_wheel − v_chassis(anchor)) · n: positive while the strut extends. An impulse λ on the
//! element goes +λ n to the wheel and −λ n to the chassis at the anchor, so it changes v_rel by λ (1/m_wheel +
//! W_chassis), W_chassis being the chassis's inverse effective mass at the anchor along n (`FUN_009e8230`).

/// `.rdata` 0x010245b0 (0.5, all lanes; read from NFS13.exe on disk): the damper's impulse factor in
/// `FUN_0092f160`.
pub const DAMPER_SCALE: f32 = 0.5;

/// The damper's impulse on its element in one sub-step (`FUN_0092f160`). `coef` is chosen by the caller from the
/// sign of `v_rel`: element +0x18 (`0c5f89b3` × c, rebound) while it is positive, +0x2c (c) otherwise. The impulse
/// is −coef × v_rel × [`DAMPER_SCALE`] × h, cut so that v_rel, predicted through `inv_mass` (the element's inverse
/// effective mass), does not change sign: past zero it stops at zero.
///
/// In the game the wheel's 1/m enters `inv_mass` and the wheel takes its share only where wheel record +0x0c
/// is not 1.0; +0x0c is the w of the wheel centre (a point, w = 1 in every snapshot), so the damper acts on the
/// chassis alone and `inv_mass` is the chassis's.
pub fn damper_impulse(v_rel: f32, coef: f32, inv_mass: f32, h: f32) -> f32 {
    let raw = -coef * v_rel * DAMPER_SCALE * h;
    let predicted = v_rel + raw * inv_mass;
    let target = if sign(predicted) == sign(v_rel) { predicted } else { 0.0 };
    (target - v_rel) / inv_mass.max(f32::EPSILON)
}

/// The spring's impulse on its element in one sub-step (`FUN_0092f510`): with x = |anchor → wheel| − rest
/// length (element +0x10; negative = compressed) and k (+0x14), the relative velocity it heads for is
/// v_rel − k x h × `inv_mass` (the chassis's inverse effective mass plus the wheel's 1/m: both take it). While
/// compressed (and wheel record +0x0c = 1, always) that is taken as it is; while extended it may not fall below
/// −x / dt (the pull back to the rest length is capped at what closes it in one full tick, `inv_dt` = context
/// +0x1e0), and at exactly the rest length it may not rise above 0. The ±1 of that cap is `.rdata` 0x01024570
/// (−1: max(−x, x) = |x|). The element accumulates λ × `inv_dt` in +0x3c (a force, for telemetry).
pub fn spring_impulse(v_rel: f32, x: f32, k: f32, inv_mass: f32, h: f32, inv_dt: f32) -> f32 {
    let free = v_rel - k * x * h * inv_mass;
    let cap = x.abs() * inv_dt;
    let target = if x < 0.0 {
        free
    } else if x > 0.0 {
        free.max(-cap)
    } else {
        free.min(cap)
    };
    (target - v_rel) / inv_mass.max(f32::EPSILON)
}

/// The wheel contact's correction in one contact pass (`FUN_009c5a90`): `d` is the penetration of the ground into
/// the tyre predicted for the end of the tick (contact point + v dt against the ground point + its velocity dt,
/// along the normal). Below zero nothing happens (the record stays inactive); otherwise the record is activated
/// (`FUN_009c52d0` sets its w to 1) and the wheel's velocity along the normal rises by d / dt, so the integration
/// that follows puts it on the ground. Returns that change, or `None` when the contact does not touch.
pub fn contact_correction(d: f32, dt: f32) -> Option<f32> {
    (d >= 0.0).then(|| d / dt)
}

/// The wheel contact's impulse pass (`FUN_009c2670`), one sub-step: the velocity change along the normal that stops
/// the wheel approaching the ground (`v_n` < 0), never one that pulls it down. The target is (bias + ground
/// velocity · n) × constraint +0x0c, and +0x0c is 0 in the snapshots, so it is 0. The record accumulates
/// mass × the change (its normal's w, copied to constraint +0x3c): the ground's normal impulse.
pub fn contact_impulse(v_n: f32) -> f32 {
    (-v_n).max(0.0)
}

/// The strut stop's slack is at most this (`.bss` 0x01186bf0, 0.1 in all lanes, read in the snapshots).
pub const STOP_SLACK_MAX: f32 = 0.1;
/// The chassis takes at least this share of the stop's predicted penetration (`.bss` 0x01186b10, 0.1, read in the
/// snapshots).
pub const STOP_MIN_SHARE: f32 = 0.1;

/// The strut stop in a contact pass (`FUN_009c5a90` with +0x4ac set): `d` is the penetration of the ground into a
/// tyre centred at the strut's end (+0x460), predicted for the end of the tick; `slack` is the tyre's sidewall
/// (+0x470). Returns (excess, share): the chassis is moved at the strut end along the normal by excess / dt
/// through its effective mass, and the impulse passes then stop its closing speed there × share (+0x474, kept
/// while +0x4ae is set). `None` when the ground does not reach past the strut end.
pub fn stop_excess(d: f32, slack: f32) -> Option<(f32, f32)> {
    if d <= 0.0 {
        return None;
    }
    let excess = (d - d.min(slack.min(STOP_SLACK_MAX))).max(0.0).max(d * STOP_MIN_SHARE);
    Some((excess, (excess / d.max(f32::EPSILON)).clamp(0.0, 1.0)))
}

/// A point or direction (x, y, z).
pub type V3 = [f32; 3];

/// `.rdata` 0x01025930 (0.0174533 in all lanes, read from NFS13.exe on disk): degrees to radians in the strut
/// joint's builder `FUN_009c3650`.
pub const DEG_TO_RAD: f32 = 0.017_453_3;

/// The strut joint (decomp.md "Strut joint"): the strut object at vehicle +0x10f0 + 8i, a world joint (element
/// type 3) between the chassis and the wheel record that holds the wheel on its strut. Points are in the chassis
/// frame: +0x40 top, +0x50 bottom, +0x60 the arm's pivot; +0x80 the arm flag (the wheel moves on an arc about the
/// pivot instead of along the strut).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrutJoint {
    pub top: V3,
    pub bottom: V3,
    pub pivot: V3,
    pub arm: bool,
}

impl StrutJoint {
    /// The builder `FUN_009c3650` (from `FUN_009b40c0` through `FUN_00933790`), from the strut's untilted ends
    /// (the wheel's x and z, y = `caac331c` + `b867a203` / + `7fc0dc41`) and the suspension constraint's `Angle`
    /// (degrees, × `DEG_TO_RAD`), arm length `c2a87807` and arm flag `b1c680d5`. With s the side (the sign of the
    /// top's x: 1, −1, or 0 on the centre line) and a the angle, the top moves out by ½ Δy sin a × s and the bottom
    /// in by as much (the top sits outboard of the bottom), and the pivot sits the arm's length from the strut's
    /// middle, perpendicular to it, towards the car's centre: middle + (−s cos a × arm, sin a × arm, 0). Checked
    /// against the 40 strut joints of snap1 (XKR front: top 0.8018 / bottom 0.7882, pivot (−1.1974, 0.0198) from
    /// x 0.795, y −0.077 / −0.232, Angle 5, arm 2; traffic with Angle 0, arm 1: the pivot 1 m inside the middle).
    pub fn build(top: V3, bottom: V3, angle: f32, arm_length: f32, arm: bool) -> Self {
        let a = angle * DEG_TO_RAD;
        let s = sign(top[0]);
        let lean = 0.5 * (top[1] - bottom[1]) * a.sin() * s;
        let top = [top[0] + lean, top[1], top[2]];
        let bottom = [bottom[0] - lean, bottom[1], bottom[2]];
        let mid: V3 = std::array::from_fn(|i| (top[i] - bottom[i]) * 0.5 + bottom[i]);
        let pivot = [mid[0] - a.cos() * arm_length * s, mid[1] + a.sin() * arm_length, mid[2]];
        Self { top, bottom, pivot, arm }
    }

    /// Where a wheel at `wheel` (chassis frame) stands along the strut: its projection, 0 at the bottom and 1 at
    /// the top (unclamped). The joint's +0x30 holds such a parameter for the velocity pass; its writer was not found
    /// (handling.md "Model decisions").
    pub fn parameter(&self, wheel: V3) -> f32 {
        let a = sub(self.top, self.bottom);
        let len2 = dot(a, a);
        if len2 > 0.0 { dot(a, sub(wheel, self.bottom)) / len2 } else { 0.0 }
    }

    /// The direction the wheel may move in at `wheel` (chassis frame, unit): along the strut (`FUN_009c3ae0`:
    /// (top − bottom) / |…|), or for the arm the arc's tangent (`FUN_009c3d70`: with d = wheel − pivot and
    /// A = top − bottom, (d × A) × d normalised, A without its part along d).
    pub fn axis(&self, wheel: V3) -> V3 {
        let a = sub(self.top, self.bottom);
        if !self.arm {
            return normalize(a);
        }
        let d = sub(wheel, self.pivot);
        normalize(cross(cross(d, a), d))
    }

    /// The position pass (`FUN_00928cd0` → `FUN_009c6230` → `FUN_009c4050`, or `FUN_009c42d0` for the arm), once
    /// per contact pass after the wheel constraints. Everything predicted for the end of the tick: `wheel` the
    /// wheel record's centre + v dt, `top`, `bottom`, `pivot` the joint's points in the world + the chassis's velocity
    /// there × dt (`FUN_009eab00`). The wheel's projection onto the strut, t = (top − bottom) · (wheel − bottom) /
    /// |top − bottom|², is clamped to 0..1 (q = bottom + t (top − bottom)); the target is q, or for the arm the point
    /// on the line pivot → q at the radius lerped by t between the bottom's and the top's distance from the pivot
    /// (taken in the chassis frame from the joint itself). Returns the error wheel − target, zero when no component
    /// exceeds FLT_EPSILON (`.rdata` 0x010258a0, `movmskps` over x, y, z). The caller removes it in one go: the
    /// impulse err / dt through the effective mass along err (the chassis's at the wheel centre, `FUN_009e8230`,
    /// plus the wheel's 1/m +0x58) goes to the chassis at the wheel centre (`FUN_009ca290` + `FUN_00907840`) and
    /// against the wheel.
    pub fn position_error(&self, wheel: V3, top: V3, bottom: V3, pivot: V3) -> V3 {
        let a = sub(top, bottom);
        let len2 = dot(a, a);
        let t = if len2 > 0.0 { (dot(a, sub(wheel, bottom)) / len2).clamp(0.0, 1.0) } else { 0.0 };
        let q = add(bottom, scale(a, t));
        let target = if self.arm {
            let r_bottom = length(sub(self.bottom, self.pivot));
            let r_top = length(sub(self.top, self.pivot));
            add(pivot, scale(normalize(sub(q, pivot)), (r_top - r_bottom) * t + r_bottom))
        } else {
            q
        };
        gate(sub(wheel, target))
    }

    /// The velocity pass (`FUN_00928fb0` → `FUN_009c62c0` → `FUN_009c3ae0`, or `FUN_009c3d70` for the arm), in
    /// each sub-step before the wheel contacts' impulse pass. `rel` is the wheel record's velocity minus the
    /// chassis's at the wheel centre (`FUN_0059eb20`), `axis` [`StrutJoint::axis`] in the world, `t` the joint's
    /// position parameter (+0x30). Everything across the axis goes, and along it (s = rel · axis, positive towards
    /// the top) what would carry the wheel past an end it is at: with t ≥ 1 (`.rdata` 0x01025830) only min(s, 0) is
    /// kept, with t ≤ 0 only max(s, 0). Returns the velocity error rel − kept × axis (zero under FLT_EPSILON as in
    /// [`StrutJoint::position_error`]); the impulse goes the same way, without the 1 / dt.
    pub fn velocity_error(rel: V3, axis: V3, t: f32) -> V3 {
        let s = dot(rel, axis);
        let kept = if t >= 1.0 { s.min(0.0) } else { s };
        let kept = if t <= 0.0 { kept.max(0.0) } else { kept };
        gate(sub(rel, scale(axis, kept)))
    }
}

fn gate(v: V3) -> V3 {
    if v.iter().all(|c| c.abs() <= f32::EPSILON) { [0.0; 3] } else { v }
}

fn add(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] + b[i])
}

fn sub(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] - b[i])
}

fn scale(a: V3, k: f32) -> V3 {
    a.map(|c| c * k)
}

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn length(a: V3) -> f32 {
    dot(a, a).sqrt()
}

/// Unit vector, zero for a zero one (ours: the game's rsqrt has no guard there).
fn normalize(a: V3) -> V3 {
    let l = length(a);
    if l > 0.0 { scale(a, 1.0 / l) } else { [0.0; 3] }
}

/// Gravity on a wheel record in one sub-step (`FUN_009282d0`, the wheels' loop): its velocity gains
/// (0, 1, 0) (`.rdata` 0x01024590) × 9.81 (`.bss` 0x01185f30, read in the snapshots) × h × h, both factors the
/// sub-step's dt: a world-up change of 0.0007 m/s per sub-step at 60 Hz. The chassis's loop applies its own
/// gravity (body +0x9c, −9.81) × h; the wheels get no other gravity (only the in-air extra, chassis.rs).
pub fn wheel_gravity(h: f32) -> f32 {
    WHEEL_GRAVITY * h * h
}

/// `.bss` 0x01185f30 (9.81 in all lanes, read in the snapshots).
pub const WHEEL_GRAVITY: f32 = 9.81;

/// −1, 0 or 1, as the game compares signs (`FUN_0092f160`: 1.0 where > 0, −1.0 where < 0, else 0).
fn sign(x: f32) -> f32 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: V3, b: V3, eps: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < eps)
    }

    #[test]
    fn strut_joint_built_as_in_snap1() {
        // XKR front right (snap1, joint 0x07ba1f00): x 0.795 (model 0.7949), top −0.077, bottom −0.232 (body
        // frame), Angle 5, arm 2, flag set.
        let j = StrutJoint::build([0.795, -0.077, 1.291], [0.795, -0.232, 1.291], 5.0, 2.0, true);
        assert!(close(j.top, [0.8018, -0.077, 1.291], 2e-4));
        assert!(close(j.bottom, [0.7882, -0.232, 1.291], 2e-4));
        assert!(close(j.pivot, [-1.1974, 0.0198, 1.291], 2e-4));
        // The left side mirrors.
        let l = StrutJoint::build([-0.795, -0.077, 1.291], [-0.795, -0.232, 1.291], 5.0, 2.0, true);
        assert!(close(l.top, [-0.8018, -0.077, 1.291], 2e-4) && close(l.pivot, [1.1974, 0.0198, 1.291], 2e-4));
        // A traffic strut (Angle 0, arm 1): straight, the pivot 1 m inside its middle.
        let t = StrutJoint::build([0.785, -0.1953, 1.365], [0.785, -0.3703, 1.365], 0.0, 1.0, false);
        assert!(close(t.pivot, [-0.215, -0.2828, 1.365], 1e-4));
    }

    #[test]
    fn strut_joint_holds_the_wheel_on_its_arc() {
        let j = StrutJoint::build([0.8, 0.1, 0.0], [0.8, -0.1, 0.0], 5.0, 2.0, true);
        // A wheel on the arc at mid travel: no error; moved 1 cm sideways: the error is that 1 cm.
        let on = j.position_error(j.bottom, j.top, j.bottom, j.pivot);
        assert_eq!(on, [0.0; 3]);
        let off = j.position_error(add(j.bottom, [0.01, 0.0, 0.0]), j.top, j.bottom, j.pivot);
        assert!(close(off, [0.01, 0.0, 0.0], 1e-3));
        // Above the top: pulled back to it.
        let above = j.position_error(add(j.top, [0.0, 0.05, 0.0]), j.top, j.bottom, j.pivot);
        assert!(above[1] > 0.04);
        // The arc's tangent is (nearly) the strut's direction, square to the pivot.
        let axis = j.axis(j.bottom);
        assert!(dot(axis, sub(j.bottom, j.pivot)).abs() < 1e-5 && axis[1] > 0.99);
    }

    #[test]
    fn strut_joint_velocity_limits_only_past_the_ends() {
        let up = [0.0, 1.0, 0.0];
        // Across always goes; along, between the ends nothing.
        assert_eq!(StrutJoint::velocity_error([0.3, 1.5, 0.0], up, 0.5), [0.3, 0.0, 0.0]);
        // At the top only the motion further up goes; at the bottom only further down.
        assert_eq!(StrutJoint::velocity_error([0.0, 1.5, 0.0], up, 1.0), [0.0, 1.5, 0.0]);
        assert_eq!(StrutJoint::velocity_error([0.0, -1.5, 0.0], up, 1.0), [0.0; 3]);
        assert_eq!(StrutJoint::velocity_error([0.0, -1.5, 0.0], up, 0.0), [0.0, -1.5, 0.0]);
    }

    #[test]
    fn damper_never_reverses_the_strut() {
        // A light chassis share and a strong damper would overshoot: the impulse stops at v_rel = 0.
        let lam = damper_impulse(2.0, 1.0e5, 1.0 / 400.0, 1.0 / 120.0);
        assert!((2.0 + lam / 400.0).abs() < 1e-5);
        // A soft one takes −c v × 0.5 h.
        let lam = damper_impulse(-1.0, 3000.0, 1.0 / 400.0, 1.0 / 120.0);
        assert!((lam - 3000.0 * 0.5 / 120.0).abs() < 1e-3);
    }

    #[test]
    fn spring_holds_its_load_when_compressed() {
        // XKR front (snap1): k 41603, compressed 0.1646 m: the wheel gets k|x|h / m = 2.28 m/s per sub-step, as its
        // record's normal velocity read in the snapshot (−2.281).
        let inv_mass = 1.0 / 25.0 + 1.0 / 450.0;
        let lam = spring_impulse(0.0, -0.1646, 41603.0, inv_mass, 1.0 / 120.0, 60.0);
        assert!((lam - 41603.0 * 0.1646 / 120.0).abs() < 1e-2);
        assert!((lam / 25.0 - 2.283).abs() < 1e-2);
    }

    #[test]
    fn extended_spring_is_capped() {
        // Extended 1 cm, already closing at 1 m/s: the pull may not drive v_rel below −x / dt = −0.6 m/s, so the
        // element pushes back.
        let inv_mass = 1.0 / 25.0;
        let lam = spring_impulse(-1.0, 0.01, 41603.0, inv_mass, 1.0 / 120.0, 60.0);
        assert!((-1.0 + lam * inv_mass + 0.6).abs() < 1e-4);
    }

    #[test]
    fn stop_takes_a_tenth_within_the_sidewall() {
        assert_eq!(stop_excess(-0.01, 0.07), None);
        let (e, s) = stop_excess(0.05, 0.07).unwrap();
        assert!((e - 0.005).abs() < 1e-6 && (s - 0.1).abs() < 1e-6);
        let (e, _) = stop_excess(0.12, 0.07).unwrap();
        assert!((e - 0.05).abs() < 1e-6);
    }
}
