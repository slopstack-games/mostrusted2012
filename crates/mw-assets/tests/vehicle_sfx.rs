//! The vehicle sound bank (tyres, road, wind, impacts) builds from the game data and responds to its inputs:
//! skids grow with slip, impacts are heard. Skipped when no game install is found.

use std::sync::Arc;

use mw_assets::AssetStore;
use mw_assets::vehicle_driver::{TyreSoundParams, VehicleDriver, VehicleInput, WheelSlip};
use mw_audio::sfx_player::{OUTPUT_RATE, Params, Player};
use mw_gamedir::GameDir;

struct Rig {
    driver: VehicleDriver,
    player: Player,
}

impl Rig {
    /// Run `seconds` with this input (64 Hz updates); RMS of the second half in dBFS.
    fn run(&mut self, seconds: f32, input: &VehicleInput) -> f32 {
        let rate = OUTPUT_RATE as usize;
        let n = (seconds * rate as f32) as usize;
        let mut sum = 0f64;
        for i in 0..n {
            if i % (rate / 64) == 0 {
                self.driver.update(1.0 / 64.0, input);
            }
            let s = self.player.next_sample();
            assert!(s.is_finite());
            if i >= n / 2 {
                sum += (s * s) as f64;
            }
        }
        10.0 * ((sum / (n - n / 2) as f64) + 1e-12).log10() as f32
    }
}

/// `slip_angle` in degrees, `slip_speed` tread minus ground (m/s), `brake` pedal.
fn cruise(mph: f32, slip_angle: f32, slip_speed: f32, brake: f32) -> VehicleInput {
    VehicleInput { speed_mph: mph, skid_wheel: Some(WheelSlip { slip_angle, slip_speed }), brake, handbrake: false }
}

/// The surface list's road asphalt (`mwtool surfaces`: index 8, `SurfacePhysics` 283001) and the stock tyre class.
const ROAD_ASPHALT: usize = 8;
const STOCK_TYRES: u8 = 2;

#[test]
fn vehicle_sounds_follow_the_physics() {
    let game = match GameDir::locate(None) {
        Ok(g) => g,
        Err(e) => return eprintln!("skipped: no game install ({e})"),
    };
    let store = AssetStore::new(game).expect("index the install");
    let (bank, surfaces) = store.vehicle_sfx_bank().expect("vehicle bank");
    assert!(bank.sequences.len() > 20, "{} sequences", bank.sequences.len());
    // Every wave decodes to more than its 47-sample PCM prefix (32 kHz streams used to stop there and loop
    // as a buzz: the "horn" in the tyre screech).
    let shortest = bank
        .sequences
        .iter()
        .flat_map(|s| &s.items)
        .filter_map(|i| match &i.kind {
            mw_audio::sfx_player::ItemKind::Wave { waves, .. } => Some(waves),
            _ => None,
        })
        .flatten()
        .map(|c| c.samples.len() as f32 / c.rate as f32)
        .fold(f32::MAX, f32::min);
    assert!(shortest > 0.05, "shortest wave {shortest} s");
    let tyre = store.tyre_sound_params().expect("TyreSoundParams");
    assert_eq!(tyre, TyreSoundParams::default(), "the built-in copy matches the data");
    let params = Arc::new(Params::new(bank.param_names()));
    // All four wheels on road asphalt (surface 8, the city's roads) with stock tyres (class 2): the game plays that
    // set's tyre and road sequences (`FUN_0054a5d0`; the runtime ramps the gate, `mostrusted::sfx`).
    let set = surfaces.pick(Some(ROAD_ASPHALT), STOCK_TYRES).expect("road asphalt has a sound set");
    params.set(&mw_assets::sfx::surface_gate(set), 1.0);
    let mut rig = Rig { driver: VehicleDriver::new(params.clone()), player: Player::new(bank, params) };

    let base = rig.run(1.5, &cruise(50.0, 0.0, 0.0, 0.0));
    // Slip angles 0°, 10°, 20°, 35° at 50 mph (lateral intensity 0, 0.25, 0.5, 0.87 in the game's ramp).
    let v = 50.0 * 0.447_04;
    let skids: Vec<f32> =
        [0.0f32, 10.0, 20.0, 35.0].iter().map(|&deg| rig.run(1.0, &cruise(50.0, deg, 0.0, 0.0))).collect();
    let locked = rig.run(1.0, &cruise(50.0, 0.0, -v, 1.0));
    let after = rig.run(1.5, &cruise(50.0, 0.0, 0.0, 0.0));
    rig.driver.impact(15.0);
    let impact = rig.run(0.3, &cruise(50.0, 0.0, 0.0, 0.0));
    eprintln!(
        "road {base:.1} dB, skid by slip angle {skids:.1?}, locked {locked:.1}, after {after:.1}, impact {impact:.1}"
    );
    assert!((skids[0] - base).abs() < 3.0, "no skid without slip");
    // The drift sequence swaps layers across the intensity range rather than getting louder throughout.
    assert!(skids[1..].iter().all(|&s| s > base + 6.0), "skids over {base}: {skids:?}");
    assert!(locked > base + 3.0, "locked wheels screech");
    assert!((after - base).abs() < 3.0, "skids stop");
    assert!(impact > base + 6.0, "impacts are heard");
}
