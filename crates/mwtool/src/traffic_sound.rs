//! `mwtool traffic-sound`: the traffic engine banks (`mw_assets::car_sound`), their RPM curves, and a render
//! of one through the game's distance falloff and doppler.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use mw_assets::AssetStore;
use mw_assets::car_sound::{TrafficEngine, TrafficEngineDriver, TrafficEngineInput, traffic_normalised_speed};
use mw_audio::sequence::{OUTPUT_RATE, Params, Player};
use mw_audio::spatial::{SpatialParams, SpatialRender};

pub fn run(store: &AssetStore, component: &str, out: Option<&Path>, passby_mph: f32) -> Result<()> {
    let data = store.traffic_data()?;
    let mut picked: Option<(u32, TrafficEngine)> = None;
    for v in &data.vehicles {
        if component != "all" && component != v.component.to_string() {
            continue;
        }
        let Some(e) = store.traffic_engine(v.component)? else {
            println!("{}: no engine sound", v.component);
            continue;
        };
        println!(
            "{} ({}): {} sequences, {} channels, top speed {} mph, idle {} rpm, master {:.3}",
            v.component,
            v.bundle,
            e.bank.sequences.len(),
            e.bank.channels.len(),
            e.top_speed_mph,
            e.idle_rpm,
            e.bank.master_gain
        );
        for s in &e.bank.sequences {
            println!("  sequence {:016x} {:?} {}..{}: {} items", s.id, s.binding, s.min, s.max, s.items.len());
        }
        let f = &e.spatial.falloff;
        println!(
            "  3D: falloff min {} m, max {} m, curve {}, gains {} / {}; focus {} / {}; doppler {:?}",
            f.min_distance,
            f.max_distance,
            f.curve,
            f.initial_gain,
            f.peak_gain,
            e.spatial.emitter_focus,
            e.spatial.listener_focus,
            e.spatial.doppler_speed
        );
        let params = Arc::new(Params::new(e.bank.param_names()));
        let mut d = TrafficEngineDriver::new(&e, params);
        let curve: Vec<String> = (0..=10)
            .map(|k| {
                let n = k as f32 / 10.0;
                d.update(&TrafficEngineInput { normalised_speed: n, ..Default::default() });
                format!("{:.0}", d.rpm())
            })
            .collect();
        println!("  rpm at normalised speed 0, 0.1 .. 1: {}", curve.join(" "));
        let gains: Vec<String> = [0.0, 5.0, 10.0, 20.0, 30.0, 50.0, 85.0, 120.0]
            .iter()
            .map(|&m| format!("{m} m {:.1} dB", 20.0 * e.spatial.gain(m, 0.0, 1.0).max(1e-6).log10()))
            .collect();
        println!("  gain: {}", gains.join(", "));
        if picked.is_none() {
            picked = Some((v.component, e));
        }
    }
    let Some(out) = out else { return Ok(()) };
    let (component, e) = picked.context("no traffic engine")?;
    let rate = OUTPUT_RATE as usize;
    let params = Arc::new(Params::new(e.bank.param_names()));
    let mut driver = TrafficEngineDriver::new(&e, params.clone());
    let mut player = Player::new(e.bank.clone(), params);
    player.enable_meter();
    let tick = rate / 60;
    let mut samples: Vec<i16> = Vec::new();
    // 1. A sweep, mono at full level: 0 → 1 → 0 normalised speed over 14 s.
    let sweep = 14.0;
    let mut rms_at = Vec::new();
    let mut acc = 0f64;
    for i in 0..(rate as f32 * sweep) as usize {
        let t = i as f32 / rate as f32;
        if i % tick == 0 {
            let n = if t < 7.0 { t / 7.0 } else { (sweep - t) / 7.0 };
            driver.update(&TrafficEngineInput { normalised_speed: n, random: 0.5, ..Default::default() });
        }
        let s = player.next_sample();
        acc += (s * s) as f64;
        if (i + 1) % rate == 0 {
            rms_at.push(format!("{:.0}s {:.1}", t.round(), 10.0 * (acc / rate as f64 + 1e-12).log10()));
            acc = 0.0;
        }
        let v = (s * 32767.0).clamp(-32768.0, 32767.0) as i16;
        samples.extend([v, v]);
    }
    println!("{component} sweep RMS dBFS per second: {}", rms_at.join(", "));
    // 2. A pass-by at `passby_mph`: the car drives along +x 4 m to the side of a listener facing +z... its
    // speed is constant, so its RPM holds; the falloff, focus and doppler of the bank's channel act on it.
    let v = passby_mph * 0.447_04;
    let n = traffic_normalised_speed(v, e.top_speed_mph);
    let sp = Arc::new(SpatialParams::default());
    let mut render = SpatialRender::new(sp.clone());
    let seconds = 200.0 / v.max(1.0);
    let mut lines = Vec::new();
    let (mut acc_l, mut acc_r, mut count) = (0f64, 0f64, 0usize);
    for i in 0..(rate as f32 * seconds) as usize {
        let t = i as f32 / rate as f32;
        let x = -100.0 + v * t;
        let (ex, ez) = (x, 4.0f32);
        let dist = (ex * ex + ez * ez).sqrt();
        if i % tick == 0 {
            driver.update(&TrafficEngineInput { normalised_speed: n, random: 0.5, ..Default::default() });
            let u = (ex / dist, ez / dist);
            // The listener looks along +z (towards the road), the car's forward is +x.
            let gain = e.spatial.gain(dist, u.0, u.1);
            let pitch = e.spatial.doppler(v * u.0);
            let pan = u.0;
            sp.set(gain, pan, pitch);
            if i % (rate / 4) < tick {
                lines.push(format!(
                    "  t {t:5.2} s  x {x:6.1} m  d {dist:5.1} m  gain {:6.1} dB  doppler {pitch:.3}  pan {pan:+.2}",
                    20.0 * gain.max(1e-6).log10()
                ));
            }
        }
        let (l, r) = render.frame(&mut || player.next_sample());
        acc_l += (l * l) as f64;
        acc_r += (r * r) as f64;
        count += 1;
        if count == rate / 4 {
            if let Some(last) = lines.last_mut() {
                last.push_str(&format!(
                    "  RMS L {:.1} R {:.1} dBFS",
                    10.0 * (acc_l / count as f64 + 1e-12).log10(),
                    10.0 * (acc_r / count as f64 + 1e-12).log10()
                ));
            }
            (acc_l, acc_r, count) = (0.0, 0.0, 0);
        }
        samples.push((l * 32767.0).clamp(-32768.0, 32767.0) as i16);
        samples.push((r * 32767.0).clamp(-32768.0, 32767.0) as i16);
    }
    println!(
        "{component} pass-by at {passby_mph} mph (normalised speed {n:.3}, rpm {:.0}):\n{}",
        driver.rpm(),
        lines.join("\n")
    );
    let pcm = mw_audio::Pcm { channels: 2, sample_rate: rate as u32, samples };
    std::fs::write(out, pcm.to_wav())?;
    println!("wrote {}", out.display());
    Ok(())
}
