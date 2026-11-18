//! The engine's start-up states: off, cranking, running (docs/decomp.md "Engine start and stop").
//!
//! The game's gearbox state machine (`FUN_009da340`, state at vehicle +0x1bd8) has two start-up states before
//! its driving ones: 0 = off (`FUN_009ce3d0`) and 1 = cranking (`FUN_009cd7a0`).
//! - **Off → cranking**: once the engine has been off for 0.5 s (`.bss` 0x01186ef0, set from `.rdata`
//!   0x01026050 by its initialiser), the throttle or the brake pedal (or the control block's flag lanes)
//!   start it (`FUN_009ce3d0`).
//! - **Cranking**: the engine sound starts `Ignition_Sequence` (`FUN_0058c930`); the state moves to 2 (running)
//!   when that sequence posts effect property 7 (the engine-start flag +0x1be5, `FUN_008f5490`) or has ended
//!   (+0x1be4, from the sound's +0x1c8; `FUN_00900120`). Per car the catch comes 0.25..1.5 s into the sequence.
//!   The start-up states never run the driving states, so the car does not drive.
//! - **EngineOn** (vehicle +0x1ef0, the engine buses' gain binding): +0.05 per tick towards 1 while running,
//!   −0.05 towards 0 in the start-up states (`FUN_009b3030`, `.bss` 0x011866c0 from `.rdata` 0x01025370).
//!
//! Model decisions (marked): the player's car spawns cranking in play (the game's spawn state comes from the
//! player slot, `FUN_00575540` / entity +0x1a84, not traced to its source), running in `--test` / `--replay`
//! runs; IGNITION (action 17: I, L3) turns a running engine off and starts a stopped one (the action's reader
//! was not found; ENGINE_START / ENGINE_OFF, 39 / 40, have no default binding). While off the physics engine
//! keeps idling silently (the original's engine model in state 0 was not read).

use bevy::prelude::*;

use crate::drive::{Car, CarInput, Player};

/// IGNITION (action 17): I; L3 (pad slot 4). `input.rs` "Input and control bindings".
pub const IGNITION: &[crate::input::Control] =
    &[crate::input::Control::Key(KeyCode::KeyI), crate::input::Control::Button(GamepadButton::LeftThumb)];

/// Seconds an engine stays off before the pedals can start it (`.bss` 0x01186ef0 = 0.5).
const RESTART_DELAY: f32 = 0.5;
/// `EngineOn` change per physics tick (`.bss` 0x011866c0 = 0.05).
const ENGINE_ON_STEP: f32 = 0.05;

pub struct IgnitionPlugin {
    /// The player's car spawns with its engine cranking (play) rather than running (tests, replays).
    pub crank_at_spawn: bool,
}

impl Plugin for IgnitionPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CrankAtSpawn(self.crank_at_spawn))
            .add_systems(Update, ignition_input)
            .add_systems(FixedUpdate, engine_states.before(crate::drive::car_physics));
    }
}

#[derive(Resource)]
pub struct CrankAtSpawn(pub bool);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineState {
    /// Gearbox state 0.
    Off,
    /// Gearbox state 1: the ignition sequence plays.
    Cranking,
    /// Gearbox states 2 and up.
    Running,
}

/// The player's engine start-up state.
#[derive(Component, Debug)]
pub struct EngineStart {
    pub state: EngineState,
    /// Seconds in the current state.
    pub timer: f32,
    /// `EngineOn` (0..1).
    pub on: f32,
    /// Starts of the cranking state so far: the engine sound starts the ignition sequence for each new one.
    pub cranks: u32,
    /// The ignition sequence's timing (`EngineBank::ignition`): seconds to the catch.
    catch: Option<f32>,
    /// IGNITION pressed since the last tick.
    toggle: bool,
}

impl EngineStart {
    /// `ignition`: the car's ignition-sequence timing (`None`: no sequence, the engine catches at once).
    pub fn new(crank: bool, ignition: Option<(f32, f32)>) -> Self {
        Self {
            state: if crank { EngineState::Cranking } else { EngineState::Running },
            timer: 0.0,
            on: if crank { 0.0 } else { 1.0 },
            cranks: u32::from(crank),
            catch: ignition.map(|(catch, length)| catch.min(length)),
            toggle: false,
        }
    }

    /// `SetEngineState` (`FUN_008444a0`): the engine is started or stopped as the ignition button would.
    pub fn set_running(&mut self, on: bool) {
        if on == (self.state == EngineState::Off) {
            self.toggle = true;
        }
    }

    pub fn running(&self) -> bool {
        self.state == EngineState::Running
    }
}

fn ignition_input(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    menu: Option<Res<crate::menu::CarMenu>>,
    pause: Option<Res<crate::pause::PauseMenu>>,
    mut cars: Query<&mut EngineStart, With<Player>>,
) {
    if menu.is_some_and(|m| m.open) || pause.is_some_and(|p| p.open) {
        return;
    }
    let pads: Vec<&Gamepad> = pads.iter().collect();
    let dev = crate::input::Devices { keys: &keys, pads: &pads };
    if dev.just_pressed(IGNITION) {
        for mut e in &mut cars {
            e.toggle = true;
        }
    }
}

/// One physics tick of the start-up states, before the car's physics reads the player's controls.
fn engine_states(time: Res<Time>, mut input: ResMut<CarInput>, mut cars: Query<&mut EngineStart, With<Car>>) {
    let dt = time.delta_secs();
    for mut e in &mut cars {
        e.timer += dt;
        let toggle = std::mem::take(&mut e.toggle);
        match e.state {
            EngineState::Running if toggle => {
                e.state = EngineState::Off;
                e.timer = 0.0;
                info!("engine off");
            }
            EngineState::Off if e.timer >= RESTART_DELAY && (toggle || input.throttle > 0.0 || input.brake > 0.0) => {
                e.state = EngineState::Cranking;
                e.timer = 0.0;
                e.cranks += 1;
                info!("engine cranking");
            }
            EngineState::Cranking if e.catch.is_none_or(|c| e.timer >= c) => {
                e.state = EngineState::Running;
                e.timer = 0.0;
                info!("engine running");
            }
            _ => {}
        }
        let target = if e.running() { 1.0 } else { 0.0 };
        e.on = if target > e.on { (e.on + ENGINE_ON_STEP).min(1.0) } else { (e.on - ENGINE_ON_STEP).max(0.0) };
        if !e.running() && input.scripted.is_none() {
            // The start-up states never reach the gearbox's driving states, so the clutch stays open; our
            // drivetrain has no such states, so the throttle is held at 0 instead (model).
            input.throttle = 0.0;
        }
    }
}
