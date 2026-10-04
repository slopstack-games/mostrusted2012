//! Body forces outside the tyres, after the game's code (docs/decomp.md "Suspension, wheels,
//! aerodynamics"): per-axis aerodynamic drag with an over-speed ramp, lift (downforce), anti-roll bars,
//! extra gravity in the air and the take-off damping that keeps a car from tumbling off a kerb or jump.

use bevy::prelude::*;
use mw_assets::handling::VehicleHandling;

const MPH: f32 = 0.44704;
/// Drag scale K in F = −½·K·C·|v|·v: 1.2, read from the running game (`.bss` 0x011872a0; ≈ air density).
const DRAG_K: f32 = 1.2;
/// Anti-roll stiffness scale on the data values: 60, read from the running game (`.bss` 0x01185d70).
const ARB_PER_DEGREE: f32 = 60.0;
/// The per-tick damping fractions are per 60 Hz tick.
const DATA_HZ: f32 = 60.0;
/// Take-off roll damping fades in between these speeds (mph).
const TAKE_OFF_ROLL_SPEEDS: [f32; 2] = [5.0, 30.0];

/// `AerodynamicBehaviour` and `BodyMovementBehaviour` values for one car.
#[derive(Debug, Clone)]
pub struct ChassisParams {
    /// Drag coefficients along the body's side (X), top (Y) and front (Z) axes.
    pub drag: Vec3,
    /// Above `f1538c51` mph the front coefficient ramps to `b8786ac2` at `4aadb133` mph.
    pub overspeed_mph: [f32; 2],
    pub overspeed_drag: f32,
    /// `Lift_coefficient` × `ca386558` (negative = downforce), applied at body z = `b821de16`.
    pub lift: f32,
    pub lift_z: f32,
    /// `In_air_gravity_coefficient`: total gravity while a wheel is off the ground.
    pub in_air_gravity: f32,
    /// Anti-roll bars (front, rear): stiffness (N m per rad of roll) and damping ratio.
    pub anti_roll: [f32; 2],
    pub roll_damping: [f32; 2],
    /// Take-off: duration (s) and per-tick angular damping of pitch, yaw, and roll (low and high, blended by
    /// the roll rate between `Low_Roll_Velocity` and `High_Roll_Velocity`).
    pub take_off_time: f32,
    pub take_off_pitch: f32,
    pub take_off_yaw: f32,
    pub take_off_roll: [f32; 2],
    pub take_off_roll_rates: [f32; 2],
}

impl ChassisParams {
    pub fn new(h: &VehicleHandling) -> Self {
        let a = |name: &str, default: f32| h.param(&format!("Aerodynamics.{name}")).unwrap_or(default);
        let b = |name: &str, default: f32| h.param(&format!("Body_movement.{name}")).unwrap_or(default);
        let t = |name: &str, default: f32| b(&format!("Take_off_behaviour.{name}"), default);
        let over_lo = a("f1538c51", 200.0);
        Self {
            drag: Vec3::new(
                a("Side_drag_coefficient", 2.0),
                a("Top_drag_coefficient", 2.0),
                a("Front_drag_coefficient", 0.75),
            ),
            overspeed_mph: [over_lo, a("4aadb133", over_lo + 40.0).max(over_lo + 1.0)],
            overspeed_drag: a("b8786ac2", 1.5),
            lift: a("Lift_coefficient", -1.0) * a("ca386558", 0.5),
            lift_z: a("b821de16", -0.3),
            in_air_gravity: h.in_air_gravity.unwrap_or(1.0).clamp(1.0, 3.0),
            anti_roll: [b("782f05c1", 1200.0) * ARB_PER_DEGREE, b("2bb0f6a0", 1200.0) * ARB_PER_DEGREE],
            roll_damping: [b("Front_roll_damping", 1.2), b("Rear_roll_damping", 1.2)],
            take_off_time: t("123d2559", 0.35),
            take_off_pitch: t("8b456140", 0.3).clamp(0.0, 0.99),
            take_off_yaw: t("312bd67d", 0.15).clamp(0.0, 0.99),
            take_off_roll: [
                t("Low_Velocity_Roll_Damping", 0.15).clamp(0.0, 0.99),
                t("High_Velocity_Roll_Damping", 0.45).clamp(0.0, 0.99),
            ],
            take_off_roll_rates: [t("Low_Roll_Velocity", 0.075), t("High_Roll_Velocity", 2.0).max(0.1)],
        }
    }

    /// Aerodynamic drag (world-space force) on a body with rotation `rot` moving at `velocity`.
    pub fn drag_force(&self, rot: Quat, velocity: Vec3) -> Vec3 {
        let local = rot.inverse() * velocity;
        let speed = velocity.length();
        let mut c = self.drag;
        let [lo, hi] = self.overspeed_mph;
        let over = ((speed / MPH - lo) / (hi - lo)).clamp(0.0, 1.0);
        c.z += (self.overspeed_drag - c.z) * over;
        rot * (-0.5 * DRAG_K * c * speed * local)
    }

    /// Lift along the body's up axis (N, negative = down) and the body-space point it acts at (relative to the
    /// body origin, at the centre of mass height).
    pub fn lift(&self, rot: Quat, velocity: Vec3, centre_of_mass: Vec3) -> (f32, Vec3) {
        let forward = velocity.dot(rot * Vec3::Z).abs();
        (self.lift * forward * velocity.length(), Vec3::new(0.0, centre_of_mass.y, self.lift_z))
    }

    /// Per-axis multipliers (pitch about X, yaw about Y, roll about Z, body space) for the angular velocity
    /// during the take-off window after a wheel leaves the ground.
    pub fn take_off_damping(&self, local_omega: Vec3, mph: f32, dt: f32) -> Vec3 {
        let ticks = dt * DATA_HZ;
        let [r0, r1] = self.take_off_roll_rates;
        let t = ((local_omega.z.abs() - r0) / (r1 - r0).max(1e-3)).clamp(0.0, 1.0);
        let roll = self.take_off_roll[0] + (self.take_off_roll[1] - self.take_off_roll[0]) * t;
        let [s0, s1] = TAKE_OFF_ROLL_SPEEDS;
        let roll = roll * ((mph.abs() - s0) / (s1 - s0)).clamp(0.0, 1.0);
        Vec3::new(
            (1.0 - self.take_off_pitch).powf(ticks),
            (1.0 - self.take_off_yaw).powf(ticks),
            (1.0 - roll).powf(ticks),
        )
    }
}
