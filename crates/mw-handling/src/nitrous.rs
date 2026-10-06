//! Nitrous: the per-tick physics of `FUN_009d2aa0` (update-loop step 5) and the tank behind its input
//! (docs/decomp.md "Nitrous and upgrades", docs/handling.md "Nitrous").
//!
//! The physics step reads the nitrous amount (vehicle controls +0x1c20, four lanes, all the same) and the mode
//! (+0x1c60) and writes a four-lane multiplier on the nitrous component (+0x10..+0x1c, default 1.0 from
//! `.rdata` 0x01026060):
//! - mode 0 (ordinary nitrous): multiplier = lerp(1, `Engine.Nitrous_strength.e9de413e`, amount)
//!   (1.9 on the 911).
//! - mode 1 (punch): multiplier = the strength; with a wheel down, a forward force
//!   `(1 − clamp((mph − Punch.5cb7c5aa) / (top mph − Punch.5cb7c5aa), 0, 1)) · amount · Punch.0798d257 · k · mass`
//!   (k = `.bss` 0x0118a018 = 0.44704, read live 2026-10-06: mph → m/s, so `0798d257` is an acceleration in
//!   mph per second; the 911's 40 is 17.9 m/s²).
//! - mode 3 (jump, airborne only): the same force with the `Jump` fields, multiplier lerp(1, `Jump.e9de413e`).
//!
//! The gearbox sees nitrous too (`FUN_009d4c20`, drivetrain.rs): upshifts on rpm alone and the top gear unlocked.
//! The tank lives in gameplay code that was not traced; its rates come from the car's `NitrousParameters`
//! (`NitrousUsage.053c5a6a` per second out of `Restrictions.c441f502` = 100), see `NitrousParams`.

use mw_assets::handling::VehicleHandling;

/// Tank size in the units of `NitrousUsage.053c5a6a` (`NitrousParameters.Restrictions.c441f502` = 100 in the
/// instances where it is not 1; which of the two the tank uses is not traced).
pub const TANK: f32 = 100.0;
/// Burn per second of the stock nitrous (`NitrousUsage.053c5a6a` = 24 in the first variant of each car; the
/// others carry 18 and 16: the nitrous upgrade's longer-lasting levels).
pub const STOCK_BURN: f32 = 24.0;
/// Fraction of the tank needed to start a burn (`NitrousUsage.7ebf362a` = 0.1 in every instance; its use as a
/// start threshold is inferred, the gameplay code was not traced).
pub const MIN_START: f32 = 0.1;

/// The car's nitrous data.
#[derive(Debug, Clone, Copy)]
pub struct NitrousParams {
    /// `Engine.Nitrous_strength.e9de413e` (engine +0x118 → +0x3c): the multiplier at full nitrous.
    pub strength: f32,
    /// `Nitrous_strength.Punch` {force `0798d257`, fade start mph `5cb7c5aa`} (+0x30, +0x34).
    pub punch: [f32; 2],
    /// `Nitrous_strength.Jump` {force `0798d257`, multiplier `e9de413e`, fade start mph `b959e02f`} (+0x18..+0x20).
    pub jump: [f32; 3],
    /// Tank burned per second (`NitrousUsage.053c5a6a`).
    pub burn: f32,
    /// The amount written to the controls while burning (`NitrousUsage.845c3879` = 1 everywhere).
    pub power: f32,
}

impl NitrousParams {
    pub fn new(h: &VehicleHandling) -> Self {
        let g = |path: &str, d: f32| h.param(&format!("Engine.Nitrous_strength.{path}")).unwrap_or(d);
        Self {
            strength: g("e9de413e", 1.0),
            punch: [g("Punch.0798d257", 0.0), g("Punch.5cb7c5aa", 0.0)],
            jump: [g("Jump.0798d257", 0.0), g("Jump.e9de413e", 1.0), g("Jump.b959e02f", 0.0)],
            burn: STOCK_BURN,
            power: 1.0,
        }
    }

    /// Mode 0 of `FUN_009d2aa0`: the component's multiplier at nitrous `amount` (0–1).
    pub fn multiplier(&self, amount: f32) -> f32 {
        if amount <= 0.0 { 1.0 } else { 1.0 + (self.strength - 1.0) * amount }
    }

    /// Modes 1 / 3: the forward acceleration (m/s², to be multiplied by the mass) of the punch or jump force for
    /// its `0798d257` value in mph per second (`FUN_009d2aa0`, `PUNCH_SCALE`). Not run yet (only mode 0 is).
    pub fn force_accel(mph_per_s: f32, fade: f32, amount: f32) -> f32 {
        fade * amount * mph_per_s * PUNCH_SCALE
    }

    /// Modes 1 / 3: the speed fade of the punch and jump forces, `1 − clamp((mph − from) / (top − from), 0, 1)`.
    pub fn fade(mph: f32, from: f32, top_mph: f32) -> f32 {
        1.0 - ((mph - from) / (top_mph - from)).clamp(0.0, 1.0)
    }
}

/// mph → m/s on the punch and jump forces (`.bss` 0x0118a018 = 0.44704, read live 2026-10-06).
pub const PUNCH_SCALE: f32 = 0.44704;

/// The tank and the burn state.
#[derive(Debug, Clone, Copy)]
pub struct Nitrous {
    /// 0..=`TANK`.
    pub tank: f32,
    pub burning: bool,
}

impl Default for Nitrous {
    fn default() -> Self {
        Self { tank: TANK, burning: false }
    }
}

impl Nitrous {
    /// One tick: burn while `held` and fuel remains; a burn starts only above `MIN_START` of the tank.
    /// Returns the amount for the controls (+0x1c20), 0 when not burning.
    pub fn step(&mut self, p: &NitrousParams, held: bool, dt: f32) -> f32 {
        self.burning = held && self.tank > 0.0 && (self.burning || self.tank >= MIN_START * TANK);
        if !self.burning {
            return 0.0;
        }
        self.tank = (self.tank - p.burn * dt).max(0.0);
        p.power
    }

    /// Tank fill, 0–1 (the HUD's `NitrousState.NitrousAmount`).
    pub fn fill(&self) -> f32 {
        self.tank / TANK
    }

    /// Add to the tank (refills from driving; the amounts per event are in `NitrousParameters`, not traced).
    pub fn add(&mut self, units: f32) {
        self.tank = (self.tank + units).clamp(0.0, TANK);
    }
}
