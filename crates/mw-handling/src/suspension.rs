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

/// The strut joint's position pass (`FUN_00928cd0` → `FUN_009c6230` → `FUN_009c4050`, or `FUN_009c42d0` when the
/// joint's +0x80 is set: the same with the wheel on an arc about the arm pivot +0x60), once per contact pass after
/// the wheel constraints. The joint is the strut object (vehicle +0x10f0 + 8i): +0x40 top, +0x50 bottom in the
/// chassis frame. The wheel record's centre predicted for the end of the tick (centre + v dt) is projected onto the
/// strut predicted the same way (`FUN_009eab00`: chassis point + its velocity × dt), the projection clamped to the
/// segment; what lies beyond (and anything across the strut) is removed in one go: the impulse err / dt through
/// the effective mass (the chassis's at the wheel centre along err, `FUN_009e8230`, plus the wheel's 1/m) goes to
/// the chassis at the wheel centre and against the wheel. `y_pred` is the predicted position along the strut
/// relative to the rest position, `bottom` / `top` the strut's ends (negative / positive); returns the part of it
/// beyond them (positive above the top), 0 within. The game skips errors at or below FLT_EPSILON (`.rdata`
/// 0x010258a0).
pub fn strut_position_error(y_pred: f32, bottom: f32, top: f32) -> f32 {
    let err = if y_pred > top {
        y_pred - top
    } else if y_pred < bottom {
        y_pred - bottom
    } else {
        0.0
    };
    if err.abs() > f32::EPSILON { err } else { 0.0 }
}

/// The strut joint's velocity pass (`FUN_00928fb0` → `FUN_009c62c0` → `FUN_009c3ae0`, or `FUN_009c3d70` for the
/// arm), in each sub-step before the wheel contacts' impulse pass: the wheel's velocity relative to the chassis
/// at its centre loses everything across the strut, and along it (`s`, positive towards the top) what would
/// carry it past an end it is at: with the joint's position parameter `t` (+0x30: 0 at the bottom, 1 at the top)
/// at 1 or more only `min(s, 0)` is kept, at 0 or less only `max(s, 0)`. Returns the part of `s` to remove (the
/// impulse goes as in [`strut_position_error`], without the 1 / dt).
pub fn strut_velocity_error(s: f32, t: f32) -> f32 {
    let allowed = if t >= 1.0 {
        s.min(0.0)
    } else if t <= 0.0 {
        s.max(0.0)
    } else {
        s
    };
    let err = s - allowed;
    if err.abs() > f32::EPSILON { err } else { 0.0 }
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

    #[test]
    fn strut_joint_limits_only_past_the_ends() {
        assert_eq!(strut_position_error(0.02, -0.09, 0.05), 0.0);
        assert!((strut_position_error(0.08, -0.09, 0.05) - 0.03).abs() < 1e-6);
        assert!((strut_position_error(-0.12, -0.09, 0.05) + 0.03).abs() < 1e-6);
        // At the top only the motion further up goes; at the bottom only further down; between, nothing.
        assert_eq!(strut_velocity_error(1.5, 1.0), 1.5);
        assert_eq!(strut_velocity_error(-1.5, 1.0), 0.0);
        assert_eq!(strut_velocity_error(-1.5, 0.0), -1.5);
        assert_eq!(strut_velocity_error(1.5, 0.5), 0.0);
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
