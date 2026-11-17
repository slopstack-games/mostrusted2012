//! The original's AI racer driving, ported from the decompiled code (docs/decomp.md "AI racers (ported)"),
//! engine-free and wasm-safe.
//!
//! Per fixed tick the game runs, for every AI driver, `FUN_004e75f0`: clear the driver's command block, run its
//! state's Update (state 8, the racer, `FUN_0050da90`; state 11, the crash recovery, `FUN_004eeba0`), then the
//! reset timer. `AIPlayer::FillInput` (0x006c0890) copies the command block into the player's input record and
//! `FUN_009aacc0` writes the vehicle's control record from it (+0x1bf0 steering, +0x1bf4 throttle, +0x1c00 brake,
//! +0x1c40 handbrake mask, +0x1c20 nitrous): [`Driver::tick`] → [`Command`] → [`InputRecord`] →
//! [`VehicleControls`].
//!
//! Conventions are the game's: the ground plane is (x, z) of the world (metres, Y up), +X is to the left of a car
//! facing +Z, headings are unit (x, z) vectors, **steering is positive to the right**, speeds are m/s
//! (the game's mph constants × 0.44704, `.data` 0x010c4284).
//!
//! What the runtime computes and hands in (world queries, the path, the race tracker) is listed on
//! [`RacerInput`] and in decomp.md "AI racers (ported)": "Runtime wiring".

mod config;
mod path;
mod pid;
mod racer;
mod rubber_band;

pub use config::AiConfig;
pub use path::{Path, PathNode, braking_distance, corner_speeds};
pub use pid::Pid;
pub use racer::{
    AiRandom, CarState, Command, Driver, InputRecord, Interaction, RacerInput, Recovery, State, TickOutput,
    VehicleControls,
};
pub use rubber_band::{RubberBand, RubberBandManager, RubberBandSlot};

/// mph → m/s (`.data` 0x010c4284 / 0x010c417c / 0x010c51b4 = 0.44704).
pub const MPH: f32 = 0.44704;
/// `FLT_MAX` (`.rdata` 0x00fa88c4 / 0x00fa7d54): "no target speed".
pub const NO_SPEED: f32 = f32::MAX;

/// A ground-plane vector (world x, z).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct V2 {
    pub x: f32,
    pub z: f32,
}

impl V2 {
    pub const fn new(x: f32, z: f32) -> Self {
        Self { x, z }
    }
    pub fn dot(self, o: V2) -> f32 {
        self.x * o.x + self.z * o.z
    }
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
    /// Unit vector; zero stays zero.
    pub fn normalize(self) -> V2 {
        let l = self.length();
        if l > 0.0 { self * (1.0 / l) } else { self }
    }
    /// The car's right for a heading `self` ((−h.z, h.x): +X is the left of a car facing +Z).
    pub fn right(self) -> V2 {
        V2::new(-self.z, self.x)
    }
    /// The angle the game takes with `atan2(z, x)` (`FUN_004e4eb0`, `FUN_004e6a10`).
    pub fn angle(self) -> f32 {
        self.z.atan2(self.x)
    }
}

impl std::ops::Add for V2 {
    type Output = V2;
    fn add(self, o: V2) -> V2 {
        V2::new(self.x + o.x, self.z + o.z)
    }
}
impl std::ops::Sub for V2 {
    type Output = V2;
    fn sub(self, o: V2) -> V2 {
        V2::new(self.x - o.x, self.z - o.z)
    }
}
impl std::ops::Mul<f32> for V2 {
    type Output = V2;
    fn mul(self, s: f32) -> V2 {
        V2::new(self.x * s, self.z * s)
    }
}

/// Wrap an angle difference into [−π, π] the game's way (one ±2π step, `.rdata` 0x00fa7d64 / 0x00fa7d68 /
/// 0x00fa8534).
pub fn wrap_angle(mut a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    if a > PI {
        a -= TAU;
    }
    if a < -PI {
        a += TAU;
    }
    a
}
