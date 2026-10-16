//! Tyre smoke and tyre mark intensities: the game's per-wheel ramps over `TyreVfxBehaviour`
//! (docs/decomp.md "Tyre marks and tyre smoke").
//!
//! `FUN_009cef90` runs once per wheel and physics tick (from `FUN_009d6e30`, after the dirt build-up) on a
//! 0x30-byte record per wheel at vehicle +0x2c90: lanes 0..2 follow the axle's `Smoke` block, lanes 4, 5 and 7
//! its `Skid_Marks` block (the smoke and the marks read them back: `FUN_00589e20`). Each block has three ramps:
//!
//! - **spin** (`Long_Spin`): s = |tread − ground speed| in mph (wheel +0xe4 − +0xe0, × 2.2369);
//! - **lateral** (`Lat_Slip`): |α| in degrees (wheel +0x114), scaled by a window on the car's speed in mph;
//! - **lock** (`64222c3f`): the same s, but only while the wheel's +0x108 is below −0.1, where the spin ramp
//!   only counts at or above it (`.data` 0x011870d0, read in the snapshot image). +0x108 is read here as the
//!   slip ratio κ (inferred: the sign test splits wheelspin from locked wheels; the skid sound treats it the
//!   same way).
//!
//! target = clamp((x − start) / max(full − start, 2⁻²³))^`Curve_Power` × scale, and the value moves towards it
//! by `rise` (rising) or `fall` (falling) × dt × 60 (`.data` 0x01187200) of the gap each tick.

use mw_assets::handling::{TyreVfxBlock, TyreVfxRamp};

/// m/s → mph (`.data` 0x0118a060, 2.236936).
pub const MS_TO_MPH: f32 = 2.236_936_3;
/// The κ below which a wheel counts as locked (lock ramp) rather than spinning (spin ramp): `.data` 0x011870d0.
pub const LOCK_SLIP: f32 = -0.1;
/// The smoothing rates are per 60 Hz tick (`.data` 0x01187200 = 60).
const TICKS_PER_SECOND: f32 = 60.0;
/// The smallest ramp span (`.data` 0x01188270 = 2⁻²³).
const MIN_SPAN: f32 = 1.192_092_9e-7;

/// One wheel's state as the ramps see it.
#[derive(Debug, Clone, Copy, Default)]
pub struct WheelInput {
    /// |tread speed − ground speed| (m/s).
    pub tread_slip: f32,
    /// Slip angle (degrees, either sign).
    pub slip_deg: f32,
    /// The car's speed (m/s, either sign).
    pub speed: f32,
    /// Slip ratio κ.
    pub kappa: f32,
}

/// The three smoothed values of one block.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Ramps {
    pub spin: f32,
    pub lat: f32,
    pub lock: f32,
}

impl Ramps {
    pub fn max(&self) -> f32 {
        self.spin.max(self.lat).max(self.lock)
    }

    /// One tick of `FUN_009cef90` for one block.
    pub fn step(&mut self, b: &TyreVfxBlock, w: &WheelInput, dt: f32) {
        let t = targets(b, w);
        let ticks = dt * TICKS_PER_SECOND;
        let smooth = |v: &mut f32, target: f32, r: &TyreVfxRamp| {
            let rate = if *v < target { r.rise } else { r.fall };
            *v += rate * ticks * (target - *v);
        };
        smooth(&mut self.spin, t.spin, &b.spin);
        smooth(&mut self.lat, t.lat, &b.lat);
        smooth(&mut self.lock, t.lock, &b.lock);
    }
}

fn unit(x: f32, start: f32, full: f32) -> f32 {
    ((x - start) / (full - start).max(MIN_SPAN)).clamp(0.0, 1.0)
}

/// What the ramps head for this tick (before the smoothing).
pub fn targets(b: &TyreVfxBlock, w: &WheelInput) -> Ramps {
    let s = w.tread_slip.abs() * MS_TO_MPH;
    let v = w.speed.abs() * MS_TO_MPH;
    let alpha = w.slip_deg.abs();
    let spin =
        if w.kappa < LOCK_SLIP { 0.0 } else { unit(s, b.spin.start, b.spin.full).powf(b.spin.power) * b.spin.scale };
    let lock =
        if w.kappa < LOCK_SLIP { unit(s, b.lock.start, b.lock.full).powf(b.lock.power) * b.lock.scale } else { 0.0 };
    let lat =
        unit(alpha, b.lat.start, b.lat.full).powf(b.lat.power) * unit(v, b.lat.speed[0], b.lat.speed[1]) * b.lat.scale;
    Ramps { spin, lat, lock }
}

/// The smoke is on while any of the wheel's three smoke ramps is above this (`.rdata` 0x00fad910 = 0.01,
/// `FUN_00589e20`).
pub const SMOKE_ON: f32 = 0.01;

#[cfg(test)]
mod tests {
    use super::*;

    /// The 911's front `Skid_Marks` block (`…68fe4`, `mwtool genesys VEHICLES/VEH_1085007_HI.BNDL --type
    /// TyreVfxBehaviour`).
    fn skid() -> TyreVfxBlock {
        TyreVfxBlock {
            lat: TyreVfxRamp {
                power: 0.35,
                start: 23.5,
                full: 60.0,
                scale: 0.45,
                rise: 0.2,
                fall: 0.1,
                speed: [0.0, 50.0],
            },
            spin: TyreVfxRamp {
                power: 1.25,
                start: 0.25,
                full: 50.0,
                scale: 1.0,
                rise: 0.2,
                fall: 0.075,
                speed: [0.0; 2],
            },
            lock: TyreVfxRamp {
                power: 0.15,
                start: 0.1,
                full: 10.0,
                scale: 0.8,
                rise: 0.01,
                fall: 0.2,
                speed: [0.0; 2],
            },
        }
    }

    #[test]
    fn lateral_needs_slip_and_speed() {
        let b = skid();
        let still = targets(&b, &WheelInput { slip_deg: 40.0, ..Default::default() });
        assert_eq!(still.lat, 0.0);
        let t = targets(&b, &WheelInput { slip_deg: 41.75, speed: 50.0 / MS_TO_MPH, ..Default::default() });
        // (18.25 / 36.5)^0.35 × 1 × 0.45
        assert!((t.lat - 0.5f32.powf(0.35) * 0.45).abs() < 1e-5, "{t:?}");
        let small = targets(&b, &WheelInput { slip_deg: 10.0, speed: 30.0, ..Default::default() });
        assert_eq!(small.lat, 0.0);
    }

    #[test]
    fn spin_and_lock_split_on_kappa() {
        let b = skid();
        let spin = targets(&b, &WheelInput { tread_slip: 10.0, kappa: 0.5, ..Default::default() });
        assert!(spin.spin > 0.0 && spin.lock == 0.0);
        let lock = targets(&b, &WheelInput { tread_slip: 10.0, kappa: -1.0, ..Default::default() });
        assert!(lock.lock > 0.0 && lock.spin == 0.0);
    }

    #[test]
    fn smoothing_rises_and_falls_at_its_rates() {
        let b = skid();
        let w = WheelInput { slip_deg: 60.0, speed: 30.0, ..Default::default() };
        let mut r = Ramps::default();
        r.step(&b, &w, 1.0 / 60.0);
        // One tick at rise 0.2 towards 0.45.
        assert!((r.lat - 0.09).abs() < 1e-5, "{r:?}");
        r.step(&b, &WheelInput::default(), 1.0 / 60.0);
        assert!((r.lat - 0.081).abs() < 1e-5, "{r:?}");
    }
}
