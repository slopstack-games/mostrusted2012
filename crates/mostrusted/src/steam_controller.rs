//! The Steam Controller (2026) read straight from its HID interface, as a Bevy gamepad.
//!
//! The controller is no game controller to the OS: over USB, Bluetooth or its wireless puck it shows up as a
//! mouse, a keyboard and a vendor-defined HID interface, so gilrs never lists it. Only Steam Input turns it into
//! an XInput pad, and only for games started through Steam. This module reads the vendor interface on a thread
//! and writes the same raw gamepad messages `bevy_gilrs` does, so the bindings of `input.rs` and every
//! `Query<&Gamepad>` see it as one more pad.
//!
//! Protocol (from SDL's `SDL_hidapi_steam_triton.c` and `steam/controller_structs.h`, read for the layout):
//! - devices: vendor 0x28DE; product 0x1302 (USB), 0x1303 (Bluetooth), 0x1304 / 0x1305 (wireless pucks, whose
//!   interfaces 2..=5 are one controller slot each);
//! - input reports 0x42 / 0x45 / 0x47, little endian after the report id: sequence `u8`, buttons `u32`, left and
//!   right trigger `i16` (0..32767), left stick X / Y and right stick X / Y `i16` (Y up is positive); the
//!   trackpads and the IMU follow and are not used here;
//! - puck reports 0x79 / 0x46: byte 1 = 2 when a controller connects to the slot, 1 when it leaves;
//! - feature report 1, message 0x87 (set settings), setting 9 = 0: turns the firmware's own mouse / keyboard
//!   emulation ("lizard mode") off. A watchdog in the controller turns it back on, so it is sent every 3 s.
//!
//! `MW_STEAM_CONTROLLER=0` turns the reader off, `=1` forces it on. By default it is off when Steam Input already
//! provides a virtual pad for this process (`SteamVirtualGamepadInfo` is set), which would double every input.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use bevy::input::InputSystems;
use bevy::input::gamepad::{
    GamepadAxis, GamepadButton, GamepadConnection, GamepadConnectionEvent, RawGamepadAxisChangedEvent,
    RawGamepadButtonChangedEvent, RawGamepadEvent,
};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use hidapi::{HidApi, HidDevice};

const VENDOR_VALVE: u16 = 0x28de;
const PRODUCT_WIRED: u16 = 0x1302;
const PRODUCT_BLE: u16 = 0x1303;
/// The wireless pucks: four controller slots on interfaces 2..=5.
const PRODUCT_PUCKS: [u16; 2] = [0x1304, 0x1305];
const PUCK_INTERFACES: std::ops::RangeInclusive<i32> = 2..=5;

const REPORT_STATE: u8 = 0x42;
const REPORT_STATE_BLE: u8 = 0x45;
const REPORT_STATE_TIMESTAMP: u8 = 0x47;
const REPORT_WIRELESS_STATUS_X: u8 = 0x46;
const REPORT_WIRELESS_STATUS: u8 = 0x79;
const WIRELESS_DISCONNECT: u8 = 1;
const WIRELESS_CONNECT: u8 = 2;

/// Feature report 1: `ID_SET_SETTINGS_VALUES` (0x87), 3 payload bytes, `SETTING_LIZARD_MODE` (9) = 0.
const LIZARD_MODE_OFF: [u8; 64] = {
    let mut report = [0; 64];
    report[0] = 1;
    report[1] = 0x87;
    report[2] = 3;
    report[3] = 9;
    report
};
const LIZARD_INTERVAL: Duration = Duration::from_secs(3);
const SCAN_INTERVAL: Duration = Duration::from_secs(2);

/// Button bits of the state report and the pad button each one is.
const BUTTONS: [(u32, GamepadButton); 15] = [
    (0x0000_0001, GamepadButton::South),
    (0x0000_0002, GamepadButton::East),
    (0x0000_0004, GamepadButton::West),
    (0x0000_0008, GamepadButton::North),
    (0x0000_0020, GamepadButton::RightThumb),
    (0x0000_0040, GamepadButton::Start),
    (0x0000_0200, GamepadButton::RightTrigger),
    (0x0000_0400, GamepadButton::DPadDown),
    (0x0000_0800, GamepadButton::DPadRight),
    (0x0000_1000, GamepadButton::DPadLeft),
    (0x0000_2000, GamepadButton::DPadUp),
    (0x0000_4000, GamepadButton::Select),
    (0x0000_8000, GamepadButton::LeftThumb),
    (0x0001_0000, GamepadButton::Mode),
    (0x0008_0000, GamepadButton::LeftTrigger),
];

/// The part of a state report the game uses.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct PadState {
    buttons: u32,
    /// Left, right: 0..1.
    triggers: [f32; 2],
    /// Left X, left Y, right X, right Y: −1..1, up and right positive.
    sticks: [f32; 4],
}

/// Decode a state report (report id first). `None` for other reports and short ones.
fn parse_state(report: &[u8]) -> Option<PadState> {
    if !matches!(*report.first()?, REPORT_STATE | REPORT_STATE_BLE | REPORT_STATE_TIMESTAMP) || report.len() < 18 {
        return None;
    }
    let word = |at: usize| f32::from(i16::from_le_bytes([report[at], report[at + 1]]));
    let axis = |at: usize| (word(at) / 32767.0).clamp(-1.0, 1.0);
    Some(PadState {
        buttons: u32::from_le_bytes([report[2], report[3], report[4], report[5]])
            & BUTTONS.iter().fold(0, |m, b| m | b.0),
        triggers: [axis(6).max(0.0), axis(8).max(0.0)],
        sticks: [axis(10), axis(12), axis(14), axis(16)],
    })
}

/// Whether a HID interface is a controller (slot) of ours.
fn is_controller(product: u16, interface: i32, usage_page: u16) -> bool {
    // Where the OS splits an interface by collection (Windows), only the vendor-defined one carries the reports.
    let vendor_page = usage_page == 0 || usage_page >= 0xff00;
    let interface_ok = if PRODUCT_PUCKS.contains(&product) {
        PUCK_INTERFACES.contains(&interface)
    } else {
        product == PRODUCT_WIRED || product == PRODUCT_BLE
    };
    interface_ok && vendor_page
}

enum Msg {
    Connected(u32, u16),
    Disconnected(u32),
    State(u32, PadState),
}

struct OpenPad {
    id: u32,
    path: std::ffi::CString,
    product: u16,
    dev: HidDevice,
    connected: bool,
    last: PadState,
    lizard_sent: Option<Instant>,
}

impl OpenPad {
    /// Read every pending report. `Err` when the device is gone or the game is closing.
    fn poll(&mut self, tx: &Sender<Msg>) -> Result<(), ()> {
        if self.connected && self.lizard_sent.is_none_or(|t| t.elapsed() >= LIZARD_INTERVAL) {
            // Best effort: with Steam running it does this itself.
            let _ = self.dev.send_feature_report(&LIZARD_MODE_OFF);
            self.lizard_sent = Some(Instant::now());
        }
        let mut report = [0u8; 64];
        loop {
            let n = self.dev.read(&mut report).map_err(|_| ())?;
            if n == 0 {
                return Ok(());
            }
            let report = &report[..n];
            if let Some(state) = parse_state(report) {
                if !self.connected {
                    self.connected = true;
                    tx.send(Msg::Connected(self.id, self.product)).map_err(|_| ())?;
                }
                if state != self.last {
                    self.last = state;
                    tx.send(Msg::State(self.id, state)).map_err(|_| ())?;
                }
            } else if matches!(report[0], REPORT_WIRELESS_STATUS | REPORT_WIRELESS_STATUS_X) && n >= 2 {
                match report[1] {
                    WIRELESS_CONNECT if !self.connected => {
                        self.connected = true;
                        tx.send(Msg::Connected(self.id, self.product)).map_err(|_| ())?;
                    }
                    WIRELESS_DISCONNECT if self.connected => self.disconnect(tx)?,
                    _ => {}
                }
            }
        }
    }

    fn disconnect(&mut self, tx: &Sender<Msg>) -> Result<(), ()> {
        self.connected = false;
        self.last = PadState::default();
        self.lizard_sent = None;
        tx.send(Msg::Disconnected(self.id)).map_err(|_| ())
    }
}

/// The reader thread: finds controllers (also ones plugged in later), reads them and reports changes.
fn reader(tx: Sender<Msg>) {
    let mut api = match HidApi::new() {
        Ok(api) => api,
        Err(e) => {
            warn!("Steam Controller: HID is not available: {e}");
            return;
        }
    };
    let mut pads: Vec<OpenPad> = Vec::new();
    let mut next_id = 0;
    let mut scanned: Option<Instant> = None;
    loop {
        if scanned.is_none_or(|t| t.elapsed() >= SCAN_INTERVAL) {
            if scanned.is_some() {
                let _ = api.refresh_devices();
            }
            scanned = Some(Instant::now());
            for info in api.device_list() {
                if info.vendor_id() != VENDOR_VALVE
                    || !is_controller(info.product_id(), info.interface_number(), info.usage_page())
                    || pads.iter().any(|p| p.path.as_c_str() == info.path())
                {
                    continue;
                }
                let Ok(dev) = info.open_device(&api) else { continue };
                if dev.set_blocking_mode(false).is_err() {
                    continue;
                }
                pads.push(OpenPad {
                    id: next_id,
                    path: info.path().to_owned(),
                    product: info.product_id(),
                    dev,
                    connected: false,
                    last: PadState::default(),
                    lizard_sent: None,
                });
                next_id += 1;
            }
        }
        let mut closing = false;
        pads.retain_mut(|pad| {
            if pad.poll(&tx).is_ok() {
                return true;
            }
            // Unplugged (or the game is closing: then the send fails too).
            closing |= pad.connected && pad.disconnect(&tx).is_err();
            false
        });
        if closing {
            return;
        }
        // Reports come every 4 ms; without a controller only the scan runs.
        std::thread::sleep(if pads.is_empty() { Duration::from_millis(500) } else { Duration::from_millis(2) });
    }
}

#[derive(Resource)]
struct SteamControllers {
    rx: Mutex<Receiver<Msg>>,
    /// Reader id → gamepad entity and the state last passed on.
    pads: HashMap<u32, (Entity, PadState)>,
}

/// Pass the reader's changes on as raw gamepad messages, as `bevy_gilrs` does for its pads.
fn forward_events(
    mut commands: Commands,
    mut controllers: ResMut<SteamControllers>,
    mut events: MessageWriter<RawGamepadEvent>,
    mut connection_events: MessageWriter<GamepadConnectionEvent>,
    mut button_events: MessageWriter<RawGamepadButtonChangedEvent>,
    mut axis_events: MessageWriter<RawGamepadAxisChangedEvent>,
) {
    let SteamControllers { rx, pads } = &mut *controllers;
    let Ok(rx) = rx.get_mut() else { return };
    for msg in rx.try_iter() {
        match msg {
            Msg::Connected(id, product) => {
                let (entity, _) = *pads.entry(id).or_insert_with(|| (commands.spawn_empty().id(), PadState::default()));
                info!(
                    "Steam Controller connected (product {product:04x}). If Steam is running and the game was not started through it, its desktop layout also sends keys and mouse moves: see the README."
                );
                let event = GamepadConnectionEvent::new(
                    entity,
                    GamepadConnection::Connected {
                        name: "Steam Controller".into(),
                        vendor_id: Some(VENDOR_VALVE),
                        product_id: Some(product),
                    },
                );
                events.write(event.clone().into());
                connection_events.write(event);
            }
            Msg::Disconnected(id) => {
                let Some((entity, state)) = pads.get_mut(&id) else { continue };
                *state = PadState::default();
                let event = GamepadConnectionEvent::new(*entity, GamepadConnection::Disconnected);
                events.write(event.clone().into());
                connection_events.write(event);
            }
            Msg::State(id, new) => {
                let Some((entity, old)) = pads.get_mut(&id) else { continue };
                let entity = *entity;
                let mut button = |button, value: f32| {
                    let event = RawGamepadButtonChangedEvent::new(entity, button, value);
                    events.write(event.into());
                    button_events.write(event);
                };
                for (bit, b) in BUTTONS {
                    if (new.buttons ^ old.buttons) & bit != 0 {
                        button(b, f32::from(u8::from(new.buttons & bit != 0)));
                    }
                }
                for (i, b) in [GamepadButton::LeftTrigger2, GamepadButton::RightTrigger2].into_iter().enumerate() {
                    if new.triggers[i] != old.triggers[i] {
                        button(b, new.triggers[i]);
                    }
                }
                let axes = [
                    GamepadAxis::LeftStickX,
                    GamepadAxis::LeftStickY,
                    GamepadAxis::RightStickX,
                    GamepadAxis::RightStickY,
                ];
                for (i, a) in axes.into_iter().enumerate() {
                    if new.sticks[i] != old.sticks[i] {
                        let event = RawGamepadAxisChangedEvent::new(entity, a, new.sticks[i]);
                        events.write(event.into());
                        axis_events.write(event);
                    }
                }
                *old = new;
            }
        }
    }
}

/// Whether to read the controller ourselves (module doc: `MW_STEAM_CONTROLLER`, Steam Input's virtual pad).
fn enabled() -> bool {
    match std::env::var("MW_STEAM_CONTROLLER").as_deref() {
        Ok("0") => false,
        Ok(_) => true,
        Err(_) => std::env::var_os("SteamVirtualGamepadInfo").is_none_or(|v| v.is_empty()),
    }
}

pub struct SteamControllerPlugin;

impl Plugin for SteamControllerPlugin {
    fn build(&self, app: &mut App) {
        if !enabled() {
            return;
        }
        let (tx, rx) = channel();
        if std::thread::Builder::new().name("steam-controller".into()).spawn(move || reader(tx)).is_err() {
            return;
        }
        app.insert_resource(SteamControllers { rx: Mutex::new(rx), pads: HashMap::default() })
            .add_systems(PreUpdate, forward_events.before(InputSystems));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state report: id, sequence, buttons, triggers, sticks, then the trackpads and IMU as zeros.
    fn report(id: u8, buttons: u32, values: [i16; 6]) -> Vec<u8> {
        let mut r = vec![id, 7];
        r.extend(buttons.to_le_bytes());
        for v in values {
            r.extend(v.to_le_bytes());
        }
        r.resize(46, 0);
        r
    }

    #[test]
    fn state_report_fields() {
        let s = parse_state(&report(REPORT_STATE, 0x0008_0005, [32767, 0, -32767, 16384, 0, -32768])).unwrap();
        assert_eq!(s.buttons, 0x0008_0005);
        assert_eq!(s.triggers, [1.0, 0.0]);
        assert_eq!(s.sticks[0], -1.0);
        assert!((s.sticks[1] - 0.5).abs() < 1e-4);
        assert_eq!(s.sticks[3], -1.0);
        assert!(parse_state(&report(REPORT_STATE_TIMESTAMP, 0, [0; 6])).is_some());
    }

    #[test]
    fn touch_bits_do_not_count_as_changes() {
        // Stick / trackpad / grip touch sensors (bits 20.., 24.., 28..) flicker while holding the controller.
        let s = parse_state(&report(REPORT_STATE, 0x3310_0000, [0; 6])).unwrap();
        assert_eq!(s, PadState::default());
    }

    #[test]
    fn other_reports_are_skipped() {
        assert!(parse_state(&[REPORT_WIRELESS_STATUS, WIRELESS_CONNECT]).is_none());
        assert!(parse_state(&[REPORT_STATE, 0, 0]).is_none());
        assert!(parse_state(&[]).is_none());
    }

    #[test]
    fn puck_slots_and_collections() {
        assert!(is_controller(0x1304, 2, 0xff00));
        assert!(is_controller(0x1304, 5, 0));
        assert!(!is_controller(0x1304, 6, 0xff00));
        assert!(!is_controller(0x1304, 3, 0x0001));
        assert!(is_controller(PRODUCT_WIRED, 0, 0xff00));
        assert!(!is_controller(0x1142, 2, 0xff00));
    }
}
