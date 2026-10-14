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
use bevy::prelude::{Query, Real, Res, ResMut, Resource, Time};

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
    /// SWAP_CARS (14): E; Y (slot 19): the jack spots' "PRESS … TO DRIVE CAR" (`jackspots.rs`). The original's
    /// mouse button 1 for it is left out (the right button looks around here).
    pub const SWAP_CARS: &[Control] = &[Key(K::KeyE), Button(B::North)];
    /// EASY_LEFT / RIGHT / UP / DOWN (69–72): Num 4 / 6 / 8 / 2; d-pad (the pad bindings' min is 0.8, a digital
    /// d-pad reads 0 or 1 either way). Left = back a tier, right = select, up / down = previous / next
    /// (`EasyDrive_ScreenScript`'s `easy_*` mappings).
    pub const EASY_LEFT: &[Control] = &[Key(K::Numpad4), Button(B::DPadLeft)];
    pub const EASY_RIGHT: &[Control] = &[Key(K::Numpad6), Button(B::DPadRight)];
    pub const EASY_UP: &[Control] = &[Key(K::Numpad8), Button(B::DPadUp)];
    pub const EASY_DOWN: &[Control] = &[Key(K::Numpad2), Button(B::DPadDown)];
}

/// The Autolog menu actions (ids 42–61) the front-end screens' user actions name by input mapping (`menu_left`,
/// `accept`, …; the name → id table at `.rdata` 0x00fe58c0, lookup `FUN_007dbc30` / `FUN_007e7910`), with the
/// original's default bindings (keyboard `FUN_00483470`, pad `FUN_00482250`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuAction {
    Left,
    Right,
    Up,
    Down,
    Accept,
    Cancel,
    Settings,
    Alt,
    TabLeft,
    TabRight,
    TriggerLeft,
    TriggerRight,
    Start,
}

impl MenuAction {
    pub const ALL: [MenuAction; 13] = [
        Self::Left,
        Self::Right,
        Self::Up,
        Self::Down,
        Self::Accept,
        Self::Cancel,
        Self::Settings,
        Self::Alt,
        Self::TabLeft,
        Self::TabRight,
        Self::TriggerLeft,
        Self::TriggerRight,
        Self::Start,
    ];

    /// The action's bindings and, for the four directions, those of its `_ALT` twin (46–49: the d-pad).
    pub fn bindings(self) -> (&'static [Control], &'static [Control]) {
        use Control::*;
        use GamepadAxis as A;
        use GamepadButton as B;
        use KeyCode as K;
        match self {
            // AUTOLOG_MENU_LEFT / RIGHT (42 / 43): arrows, left stick X (max −1 / +1); _ALT: d-pad.
            Self::Left => (&[Key(K::ArrowLeft), Axis(A::LeftStickX, -1.0)], &[Button(B::DPadLeft)]),
            Self::Right => (&[Key(K::ArrowRight), Axis(A::LeftStickX, 1.0)], &[Button(B::DPadRight)]),
            // AUTOLOG_MENU_UP / DOWN (44 / 45): arrows, left stick Y (max +1 / −1); _ALT: d-pad.
            Self::Up => (&[Key(K::ArrowUp), Axis(A::LeftStickY, 1.0)], &[Button(B::DPadUp)]),
            Self::Down => (&[Key(K::ArrowDown), Axis(A::LeftStickY, -1.0)], &[Button(B::DPadDown)]),
            // AUTOLOG_ACCEPT (52): Enter (VK_RETURN, both keys); A.
            Self::Accept => (&[Key(K::Enter), Key(K::NumpadEnter), Button(B::South)], &[]),
            // AUTOLOG_CANCEL (53): Backspace; B.
            Self::Cancel => (&[Key(K::Backspace), Button(B::East)], &[]),
            // AUTOLOG_SETTINGS (54): Right Ctrl; Y. AUTOLOG_ALT (55): E; X.
            Self::Settings => (&[Key(K::ControlRight), Button(B::North)], &[]),
            Self::Alt => (&[Key(K::KeyE), Button(B::West)], &[]),
            // AUTOLOG_TAB_LEFT / RIGHT (56 / 57): Page Up / Down; LB / RB.
            Self::TabLeft => (&[Key(K::PageUp), Button(B::LeftTrigger)], &[]),
            Self::TabRight => (&[Key(K::PageDown), Button(B::RightTrigger)], &[]),
            // AUTOLOG_TRIGGER_LEFT / RIGHT (58 / 59): Z / A; LT / RT.
            Self::TriggerLeft => (&[Key(K::KeyZ), Trigger(B::LeftTrigger2, A::LeftZ)], &[]),
            Self::TriggerRight => (&[Key(K::KeyA), Trigger(B::RightTrigger2, A::RightZ)], &[]),
            // AUTOLOG_START (61): Escape; Start.
            Self::Start => (&[Key(K::Escape), Button(B::Start)], &[]),
        }
    }

    fn directional(self) -> bool {
        matches!(self, Self::Left | Self::Right | Self::Up | Self::Down)
    }
}

/// First auto-repeat after this long held, then every `REPEAT_NEXT`. The code (`FUN_0047c360`) reads 0.2 s
/// (`.rdata` 0x00f9fb7c) for the first and 0.1 s (0x00f9fb78) for the next ones, but the original's pause menu
/// repeats every 0.2 s throughout (2026-10-14: Down held 0.45 / 0.55 / 1.0 / 1.25 s moved 3 / 3 / 6 / 7 rows, as
/// presses at 0, 0.2, 0.4, … s): the measurement is used (why the 0.1 s step does not show is open).
pub const REPEAT_FIRST: f32 = 0.2;
pub const REPEAT_NEXT: f32 = 0.2;
/// A directional menu action fires (pressed or repeated) only above this value (`FUN_007f50e0`, actions 0x2a–0x31).
pub const DIRECTION_FIRE_MIN: f32 = 0.05;
/// Which of an action and its `_ALT` twin is read (`FUN_0047a3f0`): `TABLE[main state × 4 + alt state]` is the
/// combined state (`.rdata` 0x00f9fab0; states idle 0, held 1, pressed 2, released 3); the main action is read
/// when it has that state, else the twin.
const STATE_PRIORITY: [u8; 16] = [0, 1, 2, 3, 1, 1, 1, 1, 2, 1, 2, 2, 3, 1, 2, 3];

/// One action's value over two frames and its auto-repeat timer (the binding value pair of `FUN_00ac6850`).
#[derive(Clone, Copy, Default)]
struct Track {
    value: f32,
    prev: f32,
    timer: f32,
    repeated: bool,
    repeat: bool,
}

impl Track {
    /// idle 0, held 1, pressed 2, released 3.
    fn state(&self) -> u8 {
        match (self.prev > 0.0, self.value > 0.0) {
            (false, false) => 0,
            (true, true) => 1,
            (false, true) => 2,
            (true, false) => 3,
        }
    }

    fn update(&mut self, v: f32, dt: f32) {
        self.prev = self.value;
        self.value = v;
        self.repeat = false;
        match self.state() {
            1 | 2 => {
                self.timer += dt;
                let wait = if self.repeated { REPEAT_NEXT } else { REPEAT_FIRST };
                if self.timer >= wait {
                    self.timer = 0.0;
                    self.repeat = true;
                    self.repeated = true;
                }
            }
            3 => {
                self.timer = 0.0;
                self.repeated = false;
            }
            _ => {}
        }
    }
}

/// The menu actions this frame, as the front-end screens read them (`menu_screens` systems run after
/// `update_menu_input`).
#[derive(Resource, Default)]
pub struct MenuInput {
    tracks: [(Track, Track); 13],
}

impl MenuInput {
    fn resolved(&self, a: MenuAction) -> &Track {
        let (main, alt) = &self.tracks[a as usize];
        let combined = STATE_PRIORITY[(main.state() * 4 + alt.state()) as usize];
        if main.state() == combined { main } else { alt }
    }

    /// The action's value (analogue: the stick's, keys and buttons 0 / 1).
    pub fn value(&self, a: MenuAction) -> f32 {
        self.resolved(a).value
    }

    /// Pressed this frame (a plain user action fires on this).
    pub fn pressed(&self, a: MenuAction) -> bool {
        self.resolved(a).state() == 2
    }

    /// Pressed or auto-repeated this frame (a user action with the repeat flag, `2da735e3`, fires on this).
    pub fn fired(&self, a: MenuAction) -> bool {
        let t = self.resolved(a);
        (t.state() == 2 || t.repeat) && (!a.directional() || t.value > DIRECTION_FIRE_MIN)
    }
}

/// Read the menu actions from the keys and pads (real time: menus run while the game clock is paused).
pub fn update_menu_input(
    mut menu: ResMut<MenuInput>,
    keys: Res<bevy::input::ButtonInput<KeyCode>>,
    pads: Query<&bevy::input::gamepad::Gamepad>,
    time: Res<Time<Real>>,
) {
    let pad_list: Vec<&bevy::input::gamepad::Gamepad> = pads.iter().collect();
    let dev = Devices { keys: &keys, pads: &pad_list };
    let dt = time.delta_secs();
    for a in MenuAction::ALL {
        let (main, alt) = a.bindings();
        let (m, t) = &mut menu.tracks[a as usize];
        m.update(dev.value(main), dt);
        t.update(dev.value(alt), dt);
    }
}

/// Reads the menu actions each frame, after Bevy's input systems (and after the Steam Controller's and the input
/// script's raw events, which come before them).
pub struct MenuInputPlugin;

impl bevy::app::Plugin for MenuInputPlugin {
    fn build(&self, app: &mut bevy::app::App) {
        use bevy::prelude::IntoScheduleConfigs;
        app.init_resource::<MenuInput>()
            .add_systems(bevy::app::PreUpdate, update_menu_input.after(bevy::input::InputSystems));
    }
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
    fn menu_repeat_timing() {
        // Held: fires on the press, then every 0.2 s (measured on the original); reset on release.
        let mut t = Track::default();
        let mut fired = Vec::new();
        for i in 0..10 {
            t.update(1.0, 0.0625);
            if t.state() == 2 || t.repeat {
                fired.push(i);
            }
        }
        assert_eq!(fired, vec![0, 3, 7]);
        t.update(0.0, 0.01);
        assert_eq!(t.state(), 3);
        assert!(!t.repeated && t.timer == 0.0);
    }

    #[test]
    fn menu_alt_twin_is_read_when_main_is_idle() {
        // The d-pad (MENU_*_ALT) drives menu_up when the stick / arrow is idle (FUN_0047a3f0).
        let mut m = MenuInput::default();
        let (main, alt) = &mut m.tracks[MenuAction::Up as usize];
        main.update(0.0, 0.01);
        alt.update(1.0, 0.01);
        assert!(m.pressed(MenuAction::Up) && m.fired(MenuAction::Up));
        assert_eq!(m.value(MenuAction::Up), 1.0);
        // A stick barely past its dead zone presses but does not fire a direction (> 0.05 needed).
        let mut m = MenuInput::default();
        m.tracks[MenuAction::Down as usize].0.update(0.03, 0.01);
        assert!(m.pressed(MenuAction::Down) && !m.fired(MenuAction::Down));
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
