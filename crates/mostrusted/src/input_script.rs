//! Scripted keys and pad input for headless checks of the menus (`MW_INPUT_SCRIPT`).
//!
//! The script is a `;`-separated list of `<seconds> <command> [<argument>]` steps. The clock starts on the first
//! frame the pause menu or the map screen is open and the loading screen is gone (real time, as those screens
//! run with the game clock paused); with `MW_INPUT_SCRIPT_START=playing` as soon as the game is playing.
//! Commands: `key <KeyCode>` / `keyup <KeyCode>` (held until released: `ArrowLeft`, `Enter`, `Backspace`,
//! `Escape`, `PageUp`, `KeyZ`, …), `pad <button>` / `padup <button>` on a virtual gamepad (`South`, `East`,
//! `North`, `West`, `DPadUp`, `DPadDown`, `DPadLeft`, `DPadRight`, `Start`, `Select`, `LeftTrigger`,
//! `RightTrigger`, `LeftTrigger2`, `RightTrigger2`), `lx` / `ly` / `rx` / `ry <value>` (stick axes, −1..1, up
//! positive), `shot <path>` (screenshot) and `quit`. The virtual pad sends the raw gamepad messages a real pad
//! sends, so the whole path (bindings, dead zones, `MenuInput`) is exercised. Example:
//! `MW_INPUT_SCRIPT="0.5 lx 1; 1.4 lx 0; 2.5 shot ref/m.png; 2.6 pad South; 3.5 shot ref/o.png; 4 quit"`.

use bevy::input::InputSystems;
use bevy::input::gamepad::{
    GamepadAxis, GamepadButton, GamepadConnection, GamepadConnectionEvent, RawGamepadAxisChangedEvent,
    RawGamepadButtonChangedEvent, RawGamepadEvent,
};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

#[derive(Clone, Debug)]
enum Cmd {
    Key(KeyCode, bool),
    Pad(GamepadButton, f32),
    Axis(GamepadAxis, f32),
    Shot(String),
    Quit,
}

#[derive(Resource, Default)]
struct InputScript {
    steps: Vec<(f32, Cmd)>,
    next: usize,
    /// Seconds since the menus opened (`None` until then).
    clock: Option<f32>,
    pad: Option<Entity>,
    keys: Vec<(KeyCode, bool)>,
}

fn key_code(name: &str) -> Option<KeyCode> {
    use KeyCode as K;
    Some(match name {
        "ArrowLeft" => K::ArrowLeft,
        "ArrowRight" => K::ArrowRight,
        "ArrowUp" => K::ArrowUp,
        "ArrowDown" => K::ArrowDown,
        "Enter" => K::Enter,
        "NumpadEnter" => K::NumpadEnter,
        "Backspace" => K::Backspace,
        "Escape" => K::Escape,
        "PageUp" => K::PageUp,
        "PageDown" => K::PageDown,
        "KeyZ" => K::KeyZ,
        "KeyA" => K::KeyA,
        "KeyE" => K::KeyE,
        "KeyM" => K::KeyM,
        "ControlRight" => K::ControlRight,
        "Numpad2" => K::Numpad2,
        "Numpad4" => K::Numpad4,
        "Numpad6" => K::Numpad6,
        "Numpad8" => K::Numpad8,
        // The driving keys (input.rs `defaults`): handbrake, nitrous, camera, reset, Tab.
        "Space" => K::Space,
        "Tab" => K::Tab,
        "F1" => K::F1,
        "KeyR" => K::KeyR,
        "KeyC" => K::KeyC,
        "AltLeft" => K::AltLeft,
        "ShiftLeft" => K::ShiftLeft,
        _ => return None,
    })
}

fn button(name: &str) -> Option<GamepadButton> {
    use GamepadButton as B;
    Some(match name {
        "South" => B::South,
        "East" => B::East,
        "North" => B::North,
        "West" => B::West,
        "DPadUp" => B::DPadUp,
        "DPadDown" => B::DPadDown,
        "DPadLeft" => B::DPadLeft,
        "DPadRight" => B::DPadRight,
        "Start" => B::Start,
        "Select" => B::Select,
        "LeftTrigger" => B::LeftTrigger,
        "RightTrigger" => B::RightTrigger,
        "LeftTrigger2" => B::LeftTrigger2,
        "RightTrigger2" => B::RightTrigger2,
        _ => return None,
    })
}

fn parse(script: &str) -> Vec<(f32, Cmd)> {
    let mut steps = Vec::new();
    for step in script.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let mut it = step.split_whitespace();
        let (Some(t), Some(cmd)) = (it.next().and_then(|t| t.parse::<f32>().ok()), it.next()) else {
            warn!("input script: bad step {step:?}");
            continue;
        };
        let arg = it.next().unwrap_or("");
        let num = arg.parse::<f32>().ok();
        let c = match cmd {
            "key" | "keyup" => key_code(arg).map(|k| Cmd::Key(k, cmd == "key")),
            "pad" => button(arg).map(|b| Cmd::Pad(b, 1.0)),
            "padup" => button(arg).map(|b| Cmd::Pad(b, 0.0)),
            "lx" => num.map(|v| Cmd::Axis(GamepadAxis::LeftStickX, v)),
            "ly" => num.map(|v| Cmd::Axis(GamepadAxis::LeftStickY, v)),
            "rx" => num.map(|v| Cmd::Axis(GamepadAxis::RightStickX, v)),
            "ry" => num.map(|v| Cmd::Axis(GamepadAxis::RightStickY, v)),
            "shot" => Some(Cmd::Shot(arg.to_string())),
            "quit" => Some(Cmd::Quit),
            _ => None,
        };
        match c {
            Some(c) => steps.push((t, c)),
            None => warn!("input script: bad step {step:?}"),
        }
    }
    steps.sort_by(|a, b| a.0.total_cmp(&b.0));
    steps
}

/// Before Bevy's input systems: run the clock, send the virtual pad's messages, queue key changes.
#[allow(clippy::too_many_arguments)]
fn run_script(
    mut commands: Commands,
    mut script: ResMut<InputScript>,
    time: Res<Time<Real>>,
    pause: Option<Res<crate::pause::PauseMenu>>,
    map: Option<Res<crate::map_screen::MapScreen>>,
    front: Option<Res<crate::frontend::FrontEnd>>,
    mut events: MessageWriter<RawGamepadEvent>,
    mut connections: MessageWriter<GamepadConnectionEvent>,
    mut buttons: MessageWriter<RawGamepadButtonChangedEvent>,
    mut axes: MessageWriter<RawGamepadAxisChangedEvent>,
    mut exit: MessageWriter<AppExit>,
) {
    let pad = match script.pad {
        Some(p) => p,
        None => {
            let p = commands.spawn_empty().id();
            let event = GamepadConnectionEvent::new(
                p,
                GamepadConnection::Connected { name: "Input script pad".into(), vendor_id: None, product_id: None },
            );
            events.write(event.clone().into());
            connections.write(event);
            script.pad = Some(p);
            return;
        }
    };
    // Open and on screen (not under the loading screen); `MW_INPUT_SCRIPT_START=playing`: as soon as the game is
    // playing (free roam, the HUD's widget actions).
    let playing = front.is_none_or(|f| f.playing());
    let menus = pause.is_some_and(|p| p.open) || map.is_some_and(|m| m.open);
    let open = playing && (menus || std::env::var("MW_INPUT_SCRIPT_START").is_ok_and(|v| v == "playing"));
    let now = match script.clock {
        Some(t) => t + time.delta_secs(),
        None if open => 0.0,
        None => return,
    };
    script.clock = Some(now);
    while let Some((t, cmd)) = script.steps.get(script.next).cloned() {
        if t > now {
            break;
        }
        script.next += 1;
        info!("input script {now:.2}s: {cmd:?}");
        match cmd {
            Cmd::Key(k, down) => script.keys.push((k, down)),
            Cmd::Pad(b, v) => {
                let e = RawGamepadButtonChangedEvent::new(pad, b, v);
                events.write(e.into());
                buttons.write(e);
            }
            Cmd::Axis(a, v) => {
                let e = RawGamepadAxisChangedEvent::new(pad, a, v);
                events.write(e.into());
                axes.write(e);
            }
            Cmd::Shot(path) => {
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
            }
            Cmd::Quit => {
                exit.write(AppExit::Success);
            }
        }
    }
}

/// After Bevy's keyboard system: press / release the queued keys (seen as pressed this frame).
fn apply_keys(mut script: ResMut<InputScript>, mut keys: ResMut<ButtonInput<KeyCode>>) {
    for (k, down) in script.keys.drain(..) {
        if down {
            keys.press(k);
        } else {
            keys.release(k);
        }
    }
}

pub struct InputScriptPlugin;

impl Plugin for InputScriptPlugin {
    fn build(&self, app: &mut App) {
        let Ok(text) = std::env::var("MW_INPUT_SCRIPT") else { return };
        let steps = parse(&text);
        info!("input script: {} steps", steps.len());
        app.insert_resource(InputScript { steps, ..default() })
            .add_systems(PreUpdate, run_script.before(InputSystems))
            .add_systems(PreUpdate, apply_keys.after(InputSystems).before(crate::input::update_menu_input));
    }
}
