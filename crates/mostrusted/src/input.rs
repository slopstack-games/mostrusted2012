//! Control bindings: the original's default tables and its raw-input mapping, in one place.
//!
//! Sources (docs/decomp.md "Input and control bindings"):
//! - keyboard defaults: `FUN_00483470` (device type 2, Windows virtual-key codes);
//! - XInput pad defaults: `FUN_00482250` (device type 1, control slots 0..19);
//! - a binding stores `min`, `max` and `1 / (max − min)` (`FUN_00ac72a0`); its value is
//!   `max(0, (raw − min) / (max − min))` (`FUN_00ac6850`), so steering left is the stick's X with max −1;
//! - pad poll `FUN_004119c0`: sticks per axis, `|v| / 32768`, dead zone 0.2 (`.rdata` 0x00f9156c) rescaled to
//!   0..1, sign kept; triggers `byte / 255`, dead zone 0.1 (0x00f91570) rescaled; keys and buttons 0 / 1.
//!
//! Debug keys of our own (F1–F7, Tab car menu, R reset) stay outside these tables; none of them collides with a
//! driving default of the original (its Tab = leaderboard / spot, not used here).

use bevy::input::gamepad::{GamepadAxis, GamepadButton};
use bevy::input::keyboard::KeyCode;

/// Pad stick dead zone per axis (`FUN_004119c0`, this+0x140 = `.rdata` 0x00f9156c).
pub const STICK_DEAD_ZONE: f32 = 0.2;
/// Pad trigger dead zone (`FUN_004119c0`, this+0x144 = `.rdata` 0x00f91570).
pub const TRIGGER_DEAD_ZONE: f32 = 0.1;

/// The value of one binding (`FUN_00ac6850`): `max(0, (raw − min) × 1/(max − min))`, with the scale 1 when
/// `|max − min|` ≤ FLT_EPSILON (`FUN_00ac72a0`). No upper clamp: the device values are already in −1..1.
pub fn binding_value(raw: f32, min: f32, max: f32) -> f32 {
    let span = max - min;
    let scale = if span.abs() <= f32::EPSILON { 1.0 } else { 1.0 / span };
    ((raw - min) * scale).max(0.0)
}

/// Dead zone with rescale, as the pad poll applies it to each stick axis and trigger (`FUN_004119c0`):
/// `sign(v) × clamp((|v| − dz) / (1 − dz), 0, 1)`.
pub fn dead_zone(v: f32, dz: f32) -> f32 {
    let a = ((v.abs() - dz) / (1.0 - dz)).clamp(0.0, 1.0);
    a.copysign(v)
}

/// A device slot of the original: a key or a pad control, with its binding range.
#[derive(Clone, Copy)]
pub enum Control {
    Key(KeyCode),
    /// Pad button (A, X, LB, d-pad, …): raw 0 / 1, or the analog value Bevy reports.
    Button(GamepadButton),
    /// Pad stick axis after the per-axis dead zone; the binding's `max` picks the direction (±1).
    Axis(GamepadAxis, f32),
    /// Pad trigger after the trigger dead zone; Bevy reports it as a button or, on some pads, a Z axis.
    Trigger(GamepadButton, GamepadAxis),
}

/// The actions our runtime has, with the original's default bindings (keyboard `FUN_00483470`, pad
/// `FUN_00482250`; action ids are the indices of the name table at `.data` 0x010c3540).
pub mod defaults {
    use super::Control::{self, *};
    use super::{GamepadAxis as A, GamepadButton as B, KeyCode as K};

    /// STEERLEFT (0): Left arrow; left stick X, max −1.
    pub const STEER_LEFT: &[Control] = &[Key(K::ArrowLeft), Axis(A::LeftStickX, -1.0)];
    /// STEERRIGHT (1): Right arrow; left stick X, max +1.
    pub const STEER_RIGHT: &[Control] = &[Key(K::ArrowRight), Axis(A::LeftStickX, 1.0)];
    /// ACCELERATE (2): A; right trigger.
    pub const ACCELERATE: &[Control] = &[Key(K::KeyA), Trigger(B::RightTrigger2, A::RightZ)];
    /// BRAKE (3): Z; left trigger.
    pub const BRAKE: &[Control] = &[Key(K::KeyZ), Trigger(B::LeftTrigger2, A::LeftZ)];
    /// HANDBRAKE (4): Space; X (pad slot 18).
    pub const HANDBRAKE: &[Control] = &[Key(K::Space), Button(B::West)];
    /// HORN (5): W; B (pad slot 17). Held: the player's horn (`player_sfx.rs`).
    pub const HORN: &[Control] = &[Key(K::KeyW), Button(B::East)];
    /// NITROUS (32): Left Shift; A (decomp.md, "Input").
    pub const NITROUS: &[Control] = &[Key(K::ShiftLeft), Button(B::South)];
    /// CHANGECAMERA (6): C; LB (pad slot 12).
    pub const CHANGE_CAMERA: &[Control] = &[Key(K::KeyC), Button(B::LeftTrigger)];
    /// ROTATECAMERALEFT (7): Delete; right stick X, max −1.
    pub const CAMERA_LEFT: &[Control] = &[Key(K::Delete), Axis(A::RightStickX, -1.0)];
    /// ROTATECAMERARIGHT (8): Page Down; right stick X, max +1.
    pub const CAMERA_RIGHT: &[Control] = &[Key(K::PageDown), Axis(A::RightStickX, 1.0)];
    /// ROTATECAMERAUP (9): Home; right stick Y, max +1.
    pub const CAMERA_UP: &[Control] = &[Key(K::Home), Axis(A::RightStickY, 1.0)];
    /// ROTATECAMERADOWN (10): End; right stick Y, max −1. LOOK_BACK (37) has no default binding in either
    /// table, so looking back is this action held fully (our camera's rule, camera.rs).
    pub const CAMERA_DOWN: &[Control] = &[Key(K::End), Axis(A::RightStickY, -1.0)];
    /// PAUSE (20): Escape; Start (slot 15). Opens our car menu. CHANGE_VEHICLE (31) is Back (slot 14), which
    /// opens the same menu.
    pub const PAUSE: &[Control] = &[Key(K::Escape), Button(B::Start)];
    pub const CHANGE_VEHICLE: &[Control] = &[Button(B::Select)];
    /// SKIP_CUTSCENE (34): Escape; Back (slot 14). While a sequence plays it takes these keys (`cutscene.rs`).
    pub const SKIP_CUTSCENE: &[Control] = &[Key(K::Escape), Button(B::Select)];
    /// EASY_LEFT / RIGHT / UP / DOWN (69–72): Num 4 / 6 / 8 / 2; d-pad (the pad bindings' min is 0.8, a digital
    /// d-pad reads 0 or 1 either way). Left = back a tier, right = select, up / down = previous / next
    /// (`EasyDrive_ScreenScript`'s `easy_*` mappings).
    pub const EASY_LEFT: &[Control] = &[Key(K::Numpad4), Button(B::DPadLeft)];
    pub const EASY_RIGHT: &[Control] = &[Key(K::Numpad6), Button(B::DPadRight)];
    pub const EASY_UP: &[Control] = &[Key(K::Numpad8), Button(B::DPadUp)];
    pub const EASY_DOWN: &[Control] = &[Key(K::Numpad2), Button(B::DPadDown)];
}

/// Raw device state the bindings read: keys and the pads.
pub struct Devices<'a> {
    pub keys: &'a bevy::input::ButtonInput<KeyCode>,
    pub pads: &'a [&'a bevy::input::gamepad::Gamepad],
}

impl Devices<'_> {
    /// Raw value of one control (before the binding's range).
    fn raw(&self, c: Control) -> f32 {
        let pads = self.pads.iter();
        match c {
            Control::Key(k) => f32::from(u8::from(self.keys.pressed(k))),
            Control::Button(b) => {
                pads.map(|p| p.get(b).unwrap_or(0.0).max(f32::from(u8::from(p.pressed(b))))).fold(0.0, f32::max)
            }
            Control::Axis(a, dir) => {
                // The axis value whose sign matches the binding direction, largest across pads.
                pads.map(|p| dead_zone(p.get(a).unwrap_or(0.0), STICK_DEAD_ZONE))
                    .fold(0.0, |m: f32, v| if v * dir > m * dir { v } else { m })
            }
            Control::Trigger(b, a) => pads
                .map(|p| {
                    let v = p.get(b).unwrap_or(0.0).max(p.get(a).unwrap_or(0.0));
                    dead_zone(v.clamp(0.0, 1.0), TRIGGER_DEAD_ZONE)
                })
                .fold(0.0, f32::max),
        }
    }

    /// Value of an action: its largest binding value (`FUN_00ac6850` per binding, range 0..`max`).
    pub fn value(&self, action: &[Control]) -> f32 {
        action
            .iter()
            .map(|&c| match c {
                Control::Axis(_, dir) => binding_value(self.raw(c), 0.0, dir),
                _ => binding_value(self.raw(c), 0.0, 1.0),
            })
            .fold(0.0, f32::max)
    }

    pub fn held(&self, action: &[Control]) -> bool {
        self.value(action) > 0.0
    }

    /// Pressed this frame (keys and pad buttons; analog axes are not edge-triggered here).
    pub fn just_pressed(&self, action: &[Control]) -> bool {
        action.iter().any(|&c| match c {
            Control::Key(k) => self.keys.just_pressed(k),
            Control::Button(b) | Control::Trigger(b, _) => self.pads.iter().any(|p| p.just_pressed(b)),
            Control::Axis(..) => false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steering_bindings_split_the_stick() {
        // STEERLEFT = stick X with max −1, STEERRIGHT = max +1 (FUN_00482250); steer = right − left.
        for x in [-1.0, -0.5, 0.0, 0.3, 1.0] {
            let steer = binding_value(x, 0.0, 1.0) - binding_value(x, 0.0, -1.0);
            assert!((steer - x).abs() < 1e-6, "{x} -> {steer}");
        }
        assert_eq!(binding_value(-0.4, 0.0, 1.0), 0.0);
        assert!((binding_value(-0.4, 0.0, -1.0) - 0.4).abs() < 1e-6);
    }

    #[test]
    fn keys_are_digital() {
        assert_eq!(binding_value(1.0, 0.0, 1.0), 1.0);
        assert_eq!(binding_value(0.0, 0.0, 1.0), 0.0);
    }

    #[test]
    fn easydrive_dpad_threshold() {
        // EASY_* use min 0.8 (.rdata 0x00fa1db8), max 1: a pressed d-pad gives 1, a half value 0.
        assert!((binding_value(1.0, 0.8, 1.0) - 1.0).abs() < 1e-5);
        assert_eq!(binding_value(0.5, 0.8, 1.0), 0.0);
    }

    #[test]
    fn degenerate_range_has_unit_scale() {
        assert!((binding_value(0.7, 0.5, 0.5) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn pad_dead_zones() {
        assert_eq!(dead_zone(0.15, STICK_DEAD_ZONE), 0.0);
        assert!((dead_zone(0.6, STICK_DEAD_ZONE) - 0.5).abs() < 1e-6);
        assert!((dead_zone(-0.6, STICK_DEAD_ZONE) + 0.5).abs() < 1e-6);
        assert_eq!(dead_zone(1.0, STICK_DEAD_ZONE), 1.0);
        assert_eq!(dead_zone(0.05, TRIGGER_DEAD_ZONE), 0.0);
        assert!((dead_zone(0.55, TRIGGER_DEAD_ZONE) - 0.5).abs() < 1e-6);
    }
}
