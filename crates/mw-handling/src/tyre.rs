//! Tyre force limits after the game's code (docs/decomp.md, "Tyres").
//!
//! The game's tyre is a velocity constraint: each tick the contact takes whatever force stops it sliding
//! (sideways, and between the tread and the road), capped at μ × N × factors (`FUN_009c5da0`). The grip curves
//! only set those caps: the lateral coefficient by slip angle, the longitudinal one by slip ratio, each
//! blending from its "pure" curve to a "slip" curve as the tyre also slips the other way. There is no friction
//! circle. N is not the suspension force: it is the wheel's static load plus half its axle's share of the
//! downforce ([`grip_load`]).

use mw_assets::handling::TyreAxle;

/// Below this rim speed (m/s) κ takes the braking form whatever the road speed (`FUN_009ba3e0`, `.bss`
/// 0x01186af0, read live).
const BRAKE_FORM_RIM: f32 = 0.01;

/// Slip ratio κ = (ωr − v) / v while braking (the tread slower than the road, or |ωr| < 0.01), (ωr − v) / ωr while
/// driving; clamped to ±10 (`FUN_009ba3e0`). Both divisions are by the signed speed, so a locked wheel brakes
/// (κ < 0) and a spinning one drives (κ > 0) backwards as well as forwards.
pub fn slip_ratio(rim_speed: f32, road_speed: f32) -> f32 {
    let (w, v) = (rim_speed, road_speed);
    if w.abs() < 1e-3 && v.abs() < 1e-3 {
        return 0.0;
    }
    // Ours: a divisor under 1 mm/s is held at that, keeping its sign.
    let guard = |x: f32| if x.abs() < 1e-3 { 1e-3f32.copysign(x) } else { x };
    let k = if w.abs() < BRAKE_FORM_RIM || w.abs() < v.abs() { (w - v) / guard(v) } else { (w - v) / guard(w) };
    k.clamp(-10.0, 10.0)
}

/// The normal load the game's tyre caps use (`FUN_00905910`, `FUN_009cfaf0`; confirmed on the XKR reference
/// run, where it never moved with braking or acceleration): the wheel's static load `static_load` (N, the
/// chassis weight on that corner at rest) plus half of `axle_downforce` (N, this axle's share of
/// |`Lift_coefficient`| × |v_forward| × |v|, without the `ca386558` factor that scales the force on the body).
pub fn grip_load(static_load: f32, axle_downforce: f32) -> f32 {
    static_load + 0.5 * axle_downforce.max(0.0)
}

/// (longitudinal, lateral) force coefficients (magnitudes) at slip ratio `kappa` and slip angle `alpha_deg`,
/// times the camber grip factor at `camber_deg`. `load_ratio` is the static load over the grip load
/// ([`grip_load`]); the game scales the lateral curve's exponent by it.
pub fn coefficients(t: &TyreAxle, kappa: f32, alpha_deg: f32, camber_deg: f32, load_ratio: f32) -> (f32, f32) {
    let braking = kappa < 0.0;
    let (pure_long, slip_long, long_blend) = if braking {
        (&t.brake_pure_long, &t.brake_slip_long, &t.brake_long_blend)
    } else {
        (&t.traction_pure_long, &t.traction_slip_long, &t.traction_long_blend)
    };
    let b = long_blend.at(alpha_deg.abs());
    let (p, s) = (pure_long.at(kappa).abs(), slip_long.at(kappa).abs());
    let mu_long = p + (s - p) * b;

    let (slip_lat, lat_blend) = if braking {
        (&t.brake_slip_lateral, &t.brake_lateral_blend)
    } else {
        (&t.traction_slip_lateral, &t.traction_lateral_blend)
    };
    let b = lat_blend.at(kappa.abs());
    let (p, s) = (t.pure_lateral.at_load(alpha_deg, load_ratio).abs(), slip_lat.at_load(alpha_deg, load_ratio).abs());
    let mu_lat = p + (s - p) * b;

    let [c0, c1, floor] = t.camber;
    let camber = 1.0 + (floor - 1.0) * ((camber_deg.abs() - c0) / (c1 - c0).max(1e-3)).clamp(0.0, 1.0);
    (mu_long * camber, mu_lat * camber)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grip_load_adds_half_the_axle_downforce() {
        // XKR reference run at 28.5 m/s: |Lift| 1 → 812 N in all, 42 % on the front axle.
        let front_axle = 0.4212 * 812.0;
        assert!((grip_load(4576.0, front_axle) - (4576.0 + 171.0)).abs() < 1.0);
        assert_eq!(grip_load(4576.0, -50.0), 4576.0); // lift never lowers the grip load
    }

    #[test]
    fn slip_ratio_signs() {
        assert_eq!(slip_ratio(0.0, 0.0), 0.0);
        assert!((slip_ratio(0.0, 20.0) + 1.0).abs() < 1e-6); // locked
        assert!((slip_ratio(20.0, 10.0) - 0.5).abs() < 1e-6); // spinning
        assert!(slip_ratio(30.0, 0.0) > 0.99); // burnout
    }

    #[test]
    fn slip_ratio_signs_in_reverse() {
        // Rolling backwards: a locked wheel brakes (κ = −1), a wheel spinning backwards faster than the road drives
        // (κ > 0), as forwards: the game divides by the signed v or ωr (`FUN_009ba3e0`).
        assert!((slip_ratio(0.0, -20.0) + 1.0).abs() < 1e-6);
        assert!((slip_ratio(-10.0, -20.0) + 0.5).abs() < 1e-6);
        assert!((slip_ratio(-20.0, -10.0) - 0.5).abs() < 1e-6);
        assert!(slip_ratio(-30.0, 0.0) > 0.99); // reverse burnout
    }

    #[test]
    fn slip_ratio_braking_form_below_a_hundredth() {
        // |ωr| < 0.01 m/s takes the braking form (ωr − v) / v even when the rim is faster than the road.
        assert!((slip_ratio(0.008, 0.004) - 1.0).abs() < 1e-4);
    }
}
