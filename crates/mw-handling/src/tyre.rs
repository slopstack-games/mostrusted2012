//! Tyre force limits after the game's code (docs/decomp.md, "Tyres").
//!
//! The game's tyre is a velocity constraint: each tick the contact takes whatever force stops it sliding
//! (sideways, and between the tread and the road), capped at μ × load. The grip curves only set those caps:
//! the lateral coefficient by slip angle, the longitudinal one by slip ratio, each blending from its "pure"
//! curve to a "slip" curve as the tyre also slips the other way. There is no friction circle.

use mw_assets::handling::TyreAxle;

/// Slip ratio κ = (ωr − v) / v while braking (the tread slower than the road), (ωr − v) / ωr while driving;
/// clamped to ±10.
pub fn slip_ratio(rim_speed: f32, road_speed: f32) -> f32 {
    let (w, v) = (rim_speed, road_speed);
    if w.abs() < 1e-3 && v.abs() < 1e-3 {
        return 0.0;
    }
    let k = if w.abs() < v.abs() { (w - v) / v.abs().max(1e-3) } else { (w - v) / w.abs().max(1e-3) };
    k.clamp(-10.0, 10.0)
}

/// (longitudinal, lateral) force coefficients (magnitudes) at slip ratio `kappa` and slip angle `alpha_deg`,
/// times the camber grip factor at `camber_deg`.
pub fn coefficients(t: &TyreAxle, kappa: f32, alpha_deg: f32, camber_deg: f32) -> (f32, f32) {
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
    let (p, s) = (t.pure_lateral.at(alpha_deg).abs(), slip_lat.at(alpha_deg).abs());
    let mu_lat = p + (s - p) * b;

    let [c0, c1, floor] = t.camber;
    let camber = 1.0 + (floor - 1.0) * ((camber_deg.abs() - c0) / (c1 - c0).max(1e-3)).clamp(0.0, 1.0);
    (mu_long * camber, mu_lat * camber)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slip_ratio_signs() {
        assert_eq!(slip_ratio(0.0, 0.0), 0.0);
        assert!((slip_ratio(0.0, 20.0) + 1.0).abs() < 1e-6); // locked
        assert!((slip_ratio(20.0, 10.0) - 0.5).abs() < 1e-6); // spinning
        assert!(slip_ratio(30.0, 0.0) > 0.99); // burnout
    }
}
