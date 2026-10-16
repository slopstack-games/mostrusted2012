//! Sirens and light bars: the race-car entity's siren / light-bar state (docs/decomp.md "Police sirens, light
//! bars and radio"), what drives it, and the flash clock. The siren voice is `police_audio.rs`, the bar's
//! emissive and its light `police_lightbar.rs`, the radio `police_speech.rs`.
//!
//! - **State** ([`FlashCar`], on every car whose bundle has cop lights or a siren): the siren level (entity
//!   +0x186c: 0 off, 1–3 = `Sirens[level − 1]`), the blip timer (+0x185c), the light-bar flag (+0x1c27), and the
//!   two requests the game posts to a car (+0x1950 siren: 1 → level 0, 2–4 → level 1–3, 5 → blip for 0.8 s;
//!   +0x1954 lights: 1 off, 2 on), consumed once per update (`FUN_0058d770`). While the car is wrecked or
//!   crashing the bar is off and the siren stops (`FUN_0058d770` / `FUN_0058bdc0`).
//! - **Who requests what**: `SetSirenState(vehicle, on)` (message 0xd, `FUN_00844300` → `FUN_00572d50`) posts
//!   lights on/off and siren level 1/off together; a car with a siren (`VehicleComponent.54eb210e`) driven by a
//!   person turns the horn into the siren switch: each press steps off → level 1 + lights → 2 → 3 → off + lights
//!   off (`FUN_00587720`), and the horn does not sound. What switches the AI cops' sirens on in a pursuit was not
//!   found in the code (research log 2026-10-16): [`cop_fx_state`] is our rule (MODEL, below).
//! - **Flash clock**: game time × the car's rate `frac(slot × 0.39 + 0.5) × 0.2 + 0.9` (entity +0x3e4).
//!
//! The interface to `police.rs` / `pursuit.rs` is [`cop_fx_state`] alone: it reads each cop's [`CopMode`] and the
//! pursuit's state and calls [`FlashCar::set_siren_state`]. `MW_SIREN_LOG=1` logs every change.

use std::sync::Arc;

use bevy::prelude::*;
use mw_assets::police_lights::{CarFlashLights, flash_rate};
use mw_assets::police_sound::{BLIP_SECONDS, SirenTone};

use crate::drive::Player;
use crate::police::{Cop, CopMode, Police, SLOT_BASE};
use crate::pursuit::PursuitState;

pub struct PoliceFxPlugin;

impl Plugin for PoliceFxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerSiren>()
            .init_resource::<SlotCounter>()
            .add_systems(Update, (assign_slots, cop_fx_state, player_siren_switch, apply_requests).chain())
            .add_plugins((
                crate::police_audio::PoliceAudioPlugin,
                crate::police_lightbar::LightBarPlugin,
                crate::police_speech::PoliceSpeechPlugin,
            ));
    }
}

/// A car's siren and light bar.
#[derive(Component, Debug)]
pub struct FlashCar {
    pub lights: Arc<CarFlashLights>,
    /// Entity +0x186c: 0 off, 1–3.
    pub siren_level: u8,
    /// Entity +0x185c: seconds of blipping left.
    pub blip: f32,
    /// Entity +0x1c27: the light bar (effect group 3).
    pub light_bar: bool,
    /// Wrecked or crashing: no bar, the siren stops.
    pub disabled: bool,
    /// The car's slot (the wave per level, the flash rate) and flash clock rate.
    pub slot: usize,
    pub rate: f32,
    /// Requests +0x1950 / +0x1954 (0 = none).
    siren_request: u8,
    lights_request: u8,
    assigned: bool,
}

impl FlashCar {
    pub fn new(lights: Arc<CarFlashLights>) -> Self {
        Self {
            lights,
            siren_level: 0,
            blip: 0.0,
            light_bar: false,
            disabled: false,
            slot: 0,
            rate: 1.0,
            siren_request: 0,
            lights_request: 0,
            assigned: false,
        }
    }

    /// `SetSirenState(vehicle, on)`: message 0xd with both bytes = `on` (`FUN_00572d50`): lights on / off and
    /// siren level 1 / off.
    pub fn set_siren_state(&mut self, on: bool) {
        self.siren_request = if on { 2 } else { 1 };
        self.lights_request = if on { 2 } else { 1 };
    }

    /// The light bar shows (effect group 3): requested and not wrecked.
    pub fn bar_on(&self) -> bool {
        self.light_bar && !self.disabled
    }

    /// Siren and bar are wanted (for the cop rule: posts only on a change, as the game's script would).
    fn on(&self) -> bool {
        self.siren_level > 0 || self.light_bar
    }

    /// What the siren voice plays (`FUN_0058bdc0`): blips while the blip timer runs, else the level's tone,
    /// nothing while wrecked.
    pub fn tone(&self) -> SirenTone {
        if self.blip > 0.0 {
            SirenTone::Blip
        } else if self.disabled || self.siren_level == 0 {
            SirenTone::Off
        } else {
            SirenTone::Level(self.siren_level)
        }
    }
}

/// The player's car has a siren (its horn is the siren switch; `player_sfx.rs` keeps the horn quiet).
#[derive(Resource, Default)]
pub struct PlayerSiren(pub bool);

#[derive(Resource, Default)]
struct SlotCounter(usize);

pub(crate) fn siren_log() -> bool {
    static LOG: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("MW_SIREN_LOG").is_some());
    *LOG
}

/// Gives every new flashing car its slot: a cop its cop slot, any other car the next free number.
fn assign_slots(mut counter: ResMut<SlotCounter>, mut cars: Query<(&mut FlashCar, Option<&Cop>)>) {
    for (mut f, cop) in &mut cars {
        if f.assigned {
            continue;
        }
        f.slot = match cop {
            Some(c) => c.slot() % SLOT_BASE,
            None => {
                counter.0 += 1;
                counter.0 - 1
            }
        };
        f.rate = flash_rate(f.slot);
        f.assigned = true;
    }
}

/// THE INTERFACE to the police code: the cops' sirens and bars from what each cop does.
///
/// MODEL (research log 2026-10-16: no code was found that switches an AI cop's siren in a pursuit; the
/// paths found are the horn, `SetSirenState` and cutscene cars): a cop that chases or searches (`CopMode::Chase` /
/// `Search`) while a pursuit or its cooldown runs has `SetSirenState(on)`; a patrolling cop, or any cop once the
/// pursuit is over, has it off. A crashed cop (`AiCrash`) is "wrecked": bar off, siren stopped.
pub fn cop_fx_state(
    police: Option<Res<Police>>,
    mut cops: Query<(&Cop, &mut FlashCar, Has<crate::car_impacts::AiCrash>), Without<Player>>,
) {
    let pursuit = police.as_ref().map(|p| p.pursuit.state).unwrap_or_default();
    let pursuit_on = matches!(pursuit, PursuitState::Chase | PursuitState::Cooldown | PursuitState::Busted);
    for (cop, mut f, crashed) in &mut cops {
        let want = pursuit_on && matches!(cop.mode(), CopMode::Chase | CopMode::Search);
        if want != f.on() && f.siren_request == 0 {
            if siren_log() {
                info!("siren: cop {} {:?} -> SetSirenState({want})", cop.slot(), cop.mode());
            }
            f.set_siren_state(want);
        }
        f.disabled = crashed;
    }
}

/// `FUN_00587720`: a car with a siren driven by the player steps its siren with the horn.
fn player_siren_switch(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut player_siren: ResMut<PlayerSiren>,
    mut held_before: Local<bool>,
    mut cars: Query<&mut FlashCar, With<Player>>,
) {
    let Ok(mut f) = cars.single_mut() else {
        player_siren.0 = false;
        return;
    };
    player_siren.0 = f.lights.has_siren;
    if !f.lights.has_siren {
        return;
    }
    let pads: Vec<&Gamepad> = pads.iter().collect();
    let dev = crate::input::Devices { keys: &keys, pads: &pads };
    // `MW_SIREN_PRESS=t0,t1,...`: press the horn for 0.1 s at these game times (headless checks).
    let now = time.elapsed_secs();
    let scripted = scripted_presses().iter().any(|&t| now >= t && now < t + 0.1);
    let held = dev.held(crate::input::defaults::HORN) || scripted;
    if held && !*held_before && !f.disabled {
        // off → 1 (+ lights), 1 → 2, 2 → 3, 3 → off (− lights).
        let (siren, lights) = match f.siren_level {
            0 => (2, 2),
            1 => (3, 2),
            2 => (4, 2),
            _ => (1, 1),
        };
        f.siren_request = siren;
        f.lights_request = lights;
    }
    *held_before = held;
}

fn scripted_presses() -> &'static [f32] {
    static AT: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    AT.get_or_init(|| {
        std::env::var("MW_SIREN_PRESS")
            .map(|s| s.split(',').filter_map(|t| t.trim().parse().ok()).collect())
            .unwrap_or_default()
    })
}

/// `FUN_0058d770`: consume the requests, count the blip down, keep the bar off while wrecked.
fn apply_requests(time: Res<Time>, mut cars: Query<(Entity, &mut FlashCar)>) {
    let dt = time.delta_secs();
    for (e, mut f) in &mut cars {
        let before = (f.siren_level, f.bar_on(), f.blip > 0.0);
        match f.siren_request {
            1 => f.siren_level = 0,
            2..=4 => f.siren_level = f.siren_request - 1,
            5 => f.blip = BLIP_SECONDS,
            _ => {}
        }
        match f.lights_request {
            1 => f.light_bar = false,
            2 => f.light_bar = true,
            _ => {}
        }
        f.siren_request = 0;
        f.lights_request = 0;
        f.blip = (f.blip - dt).max(0.0);
        let bar = f.light_bar && !f.disabled;
        if siren_log() && before != (f.siren_level, bar, f.blip > 0.0) {
            info!("siren: {e:?} slot {} level {} bar {} blip {:.2}", f.slot, f.siren_level, bar, f.blip);
        }
    }
}
