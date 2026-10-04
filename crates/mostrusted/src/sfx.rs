//! The player car's tyre, road, wind, impact and scrape sounds, from the game's sound data
//! (`mw_assets::sfx`): one sequence bank rendered by `mw_audio::sfx_player::Player`, driven by the wheels'
//! ground contacts (`Car::wheel_contacts`) and the body's contacts (`particles::ImpactEvent`).

use std::num::NonZero;
use std::sync::{Arc, Mutex};

use bevy::audio::{AddAudioSource, Decodable, Sample, Source, Volume};
use bevy::prelude::*;
use mw_assets::vehicle_driver::{VehicleDriver, VehicleInput, WheelSlip};
use mw_audio::sfx_player::{Bank, OUTPUT_RATE, Params, Player};

use crate::drive::{Car, CarInput};
use crate::particles::ImpactEvent;

pub struct SfxPlugin;

impl Plugin for SfxPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<SfxAudio>()
            .init_resource::<SfxState>()
            .add_systems(Update, (load_bank, update_params).chain());
    }
}

#[derive(Asset, TypePath, Clone)]
pub struct SfxAudio {
    bank: Bank,
    params: Arc<Params>,
}

pub struct SfxSource(Box<Player>);

impl Iterator for SfxSource {
    type Item = Sample;
    fn next(&mut self) -> Option<Sample> {
        Some(self.0.next_sample())
    }
}

impl Source for SfxSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> NonZero<u16> {
        NonZero::new(1).unwrap()
    }
    fn sample_rate(&self) -> NonZero<u32> {
        NonZero::new(OUTPUT_RATE).unwrap()
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

impl Decodable for SfxAudio {
    type Decoder = SfxSource;
    fn decoder(&self) -> SfxSource {
        let mut p = Player::new(self.bank.clone(), self.params.clone());
        p.dry = std::env::var_os("MW_AUDIO_DRY").is_some();
        SfxSource(Box::new(p))
    }
}

#[derive(Resource, Default)]
struct SfxState {
    /// The bank, built on a worker thread (it decodes a few hundred waves).
    building: Option<Arc<Mutex<Option<Bank>>>>,
    started: bool,
    driver: Option<VehicleDriver>,
    last_impact: f32,
    scrape: f32,
    /// Time with no wheel on the ground (s).
    air_time: f32,
}

/// Closing speed (m/s) that counts as an impact, and the minimum time between impact sounds.
const IMPACT_SPEED: f32 = 2.0;
const IMPACT_COOLDOWN: f32 = 0.2;

fn load_bank(
    mut commands: Commands,
    mut state: ResMut<SfxState>,
    mut assets: ResMut<Assets<SfxAudio>>,
    game: Res<crate::Game>,
) {
    if state.driver.is_some() || (game.mute && std::env::var_os("MW_SFX_TRACE").is_none()) {
        return;
    }
    if !state.started {
        state.started = true;
        let slot = Arc::new(Mutex::new(None));
        let out = slot.clone();
        let store = game.store.clone();
        std::thread::spawn(move || {
            let bank = store.vehicle_sfx_bank().unwrap_or_else(|e| {
                warn!("vehicle sounds: {e}");
                Bank::default()
            });
            *out.lock().unwrap() = Some(bank);
        });
        state.building = Some(slot);
        return;
    }
    let Some(slot) = state.building.clone() else { return };
    let Some(bank) = slot.lock().unwrap().take() else { return };
    state.building = None;
    info!("vehicle sounds: {} sequences, {} channels", bank.sequences.len(), bank.channels.len());
    let params = Arc::new(Params::new(bank.param_names()));
    state.driver = Some(VehicleDriver::new(params.clone()));
    let handle = assets.add(SfxAudio { bank, params });
    // ONCE: a live source never ends; LOOP would buffer it 0.74 s at a time (see audio.rs).
    // Starts silent; `settings::apply_volumes` sets the effects volume once the sink exists.
    commands.spawn((
        AudioPlayer::<SfxAudio>(handle),
        PlaybackSettings::ONCE.with_volume(Volume::Linear(0.0)),
        crate::settings::Channel::Sfx,
    ));
}

fn update_params(
    time: Res<Time>,
    mut state: ResMut<SfxState>,
    cars: Query<(Entity, &Car, &GlobalTransform)>,
    input: Res<CarInput>,
    mut impacts: MessageReader<ImpactEvent>,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let state = &mut *state;
    let Some(driver) = state.driver.as_mut() else {
        impacts.clear();
        return;
    };
    let car = cars.single().ok();
    // Body contacts this frame: the hardest closing speed and the fastest slide.
    let (mut closing, mut sliding) = (0f32, 0f32);
    for e in impacts.read() {
        if car.is_some_and(|(c, ..)| c == e.car) && e.impulse > 0.0 {
            closing = closing.max(e.closing_speed);
            sliding = sliding.max(e.sliding_speed);
        }
    }
    if closing > IMPACT_SPEED && now - state.last_impact > IMPACT_COOLDOWN {
        state.last_impact = now;
        driver.impact(closing);
    }
    // Scrapes hold briefly between contact frames.
    state.scrape = if sliding > 0.0 { sliding } else { (state.scrape - dt * 40.0).max(0.0) };
    let mut i = VehicleInput { brake: input.brake, scrape_speed: state.scrape, ..default() };
    let mut landing = 0f32;
    if let Some((_, car, transform)) = car {
        i.speed_mph = car.speed.abs() / 0.447_04;
        let forward = transform.rotation() * Vec3::Z;
        for c in car.wheel_contacts().flatten() {
            let speed = c.velocity.length();
            // Split the patch's slip along the car's heading (positive: locked or sliding forward, negative:
            // spinning) and across it. At a standstill only wheelspin slips (a burnout).
            let fwd = forward.reject_from(c.normal).normalize_or_zero();
            let (long, lateral) = if speed < 1.0 {
                (-c.slip.length(), 0.0)
            } else {
                let long = c.slip.dot(fwd);
                (long, (c.slip - fwd * long).length())
            };
            i.wheels.push(WheelSlip { speed, lateral, longitudinal: long });
            landing = landing.max(-c.velocity.dot(c.normal));
        }
    }
    // Landings: wheels back on the ground after a jump thump like a body impact at the touchdown speed.
    let grounded = i.wheels.len();
    if grounded == 0 {
        state.air_time += dt;
    } else {
        if state.air_time > 0.25 && grounded >= 2 && landing > IMPACT_SPEED && now - state.last_impact > IMPACT_COOLDOWN
        {
            state.last_impact = now;
            driver.impact(landing);
        }
        state.air_time = 0.0;
    }
    driver.update(dt, &i);
    trace(now, driver.last(), closing);
}

/// `MW_SFX_TRACE=<file>`: one line per frame, `seconds mph lateral spin braking scrape closing` (skid
/// intensities 0..1, speeds m/s).
fn trace(t: f32, v: [f32; 5], closing: f32) {
    use std::io::Write;
    static FILE: std::sync::OnceLock<Option<Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    let file = FILE.get_or_init(|| std::fs::File::create(std::env::var_os("MW_SFX_TRACE")?).ok().map(Mutex::new));
    if let Some(f) = file {
        _ = writeln!(
            f.lock().unwrap(),
            "{t:.4} {:.1} {:.3} {:.3} {:.3} {:.2} {closing:.2}",
            v[0],
            v[1],
            v[2],
            v[3],
            v[4]
        );
    }
}
