//! Every car's engine bank builds from its `EngineSound2` data and renders a rev sweep without blowing
//! up: finite, not pinned at the limiter, audible. Skipped when no game install is found.

use std::sync::Arc;

use mw_assets::engine_driver::{EngineDriver, EngineInput};
use mw_assets::engine_sound::EngineVoice;
use mw_assets::{AssetStore, DEFAULT_LANGUAGE};
use mw_audio::sequence::{OUTPUT_RATE, Params, Player};
use mw_gamedir::GameDir;

#[test]
fn every_engine_bank_renders() {
    let game = match GameDir::locate(None) {
        Ok(g) => g,
        Err(e) => return eprintln!("skipped: no game install ({e})"),
    };
    let store = AssetStore::new(game).expect("index the install");
    let lang = store.language(DEFAULT_LANGUAGE).expect("language");
    let mut seen = std::collections::HashSet::new();
    let (mut banks, mut layers) = (0, 0);
    for v in store.vehicles(&lang).expect("vehicle list") {
        let Some(path) = v.hi_bundle.filter(|p| seen.insert(p.clone())) else { continue };
        let b = store.bundle(&path).expect("bundle");
        let Some(voice) = store.engine(&b).expect("engine") else { continue };
        let EngineVoice::Bank(eb) = voice else {
            layers += 1;
            continue;
        };
        banks += 1;
        let params = Arc::new(Params::new(eb.bank.param_names()));
        let mut driver = EngineDriver::new(&eb, params.clone());
        let mut player = Player::new(eb.bank.clone(), params);
        let rate = OUTPUT_RATE as usize;
        let (mut peak, mut sum, mut pinned) = (0f32, 0f64, 0usize);
        // 1.5 s: idle, then a full-throttle rev to the redline.
        let n = rate * 3 / 2;
        for i in 0..n {
            if i % (rate / 64) == 0 {
                let t = i as f32 / rate as f32;
                let rpm = if t < 0.5 { 1000.0 } else { 1000.0 + 6000.0 * (t - 0.5) };
                let input = EngineInput {
                    rpm,
                    throttle: if t < 0.5 { 0.0 } else { 1.0 },
                    gear: 1,
                    speed_mph: 0.0,
                    idle_rpm: 1000.0,
                    redline_rpm: 7000.0,
                    top_speed_mph: 180.0,
                };
                driver.update(1.0 / 64.0, &input);
            }
            let s = player.next_sample();
            assert!(s.is_finite(), "{}: non-finite sample", v.name);
            peak = peak.max(s.abs());
            sum += (s * s) as f64;
            pinned += usize::from(s.abs() > 0.98);
        }
        let rms = (sum / n as f64).sqrt();
        assert!(rms > 1e-3, "{}: silent (rms {rms})", v.name);
        eprintln!(
            "{:<40} rms {:6.1} dBFS peak {peak:.3} limiter {:.2}%",
            v.name,
            20.0 * rms.log10(),
            pinned as f64 * 100.0 / n as f64
        );
        assert!(pinned * 50 < n, "{}: {} samples at the limiter", v.name, pinned);
    }
    eprintln!("{banks} engine banks rendered, {layers} cars fell back to Ginsu layers");
    assert!(banks > 50, "only {banks} engine banks");
}
