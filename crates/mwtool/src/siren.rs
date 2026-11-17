//! `mwtool siren`: the cops' siren sets (`mw_assets::police_sound`), their waves and channels, and a render of
//! one set's voice through the tone changes (`SirenPlayer`), plus the flash lights of a cop car
//! (`mw_assets::police_lights`).

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use mw_assets::AssetStore;
use mw_assets::police_sound::{SirenPlayer, SirenTone};

const RATE: u32 = 48_000;

pub fn run(store: &AssetStore, set: usize, out: Option<&Path>, car: Option<&str>) -> Result<()> {
    let sounds = store.cop_sounds().context("no CopSoundSet in VEHICLES/WAVES.BNDL")?;
    for (name, list) in [("efd7715f", &sounds.sets_a), ("9f467b30", &sounds.sets_b)] {
        println!("list {name}: {} sets", list.len());
        for s in list.iter() {
            println!(
                "  set {:016x}: channel gain {:.3}, doppler {:?}, {} blips, burst {}, flag {}",
                s.id.0,
                s.gain,
                s.spatial.doppler_speed,
                s.blips.len(),
                s.burst.as_ref().map_or("-".into(), |b| format!(
                    "{:016x} ({:.2} s, gain {}, fade out {} s)",
                    b.id.0, b.length, b.gain, b.fade_out
                )),
                s.flag
            );
            for (i, l) in s.levels.iter().enumerate() {
                let w: Vec<String> = l
                    .iter()
                    .map(|c| {
                        format!(
                            "{:.2} s{}",
                            c.samples.len() as f32 / c.rate as f32,
                            if c.looping { " loop" } else { "" }
                        )
                    })
                    .collect();
                println!("    level {}: {}", i + 1, w.join(", "));
            }
        }
    }
    for (name, c) in [("competitor", &sounds.competitor), ("player", &sounds.player), ("roadblock", &sounds.roadblock)]
    {
        match c {
            Some(c) => println!(
                "channel {name}: gain {:.3}, voices {}, doppler {:?}, focus {:.2} / {:.2}, falloff min {} max {} m",
                c.gain,
                c.voices,
                c.spatial.doppler_speed,
                c.spatial.emitter_focus,
                c.spatial.listener_focus,
                c.spatial.falloff.min_distance,
                c.spatial.falloff.max_distance
            ),
            None => println!("channel {name}: not found"),
        }
    }
    if let Some(car) = car {
        lights(store, car)?;
    }
    let Some(out) = out else { return Ok(()) };
    let s = sounds.sets_b.get(set).or(sounds.sets_b.first()).context("no siren set")?.clone();
    // 1 s off, 4 s each level, 0.8 s blip, 4 s level 1, 1 s off.
    let script: [(f32, SirenTone); 7] = [
        (1.0, SirenTone::Off),
        (4.0, SirenTone::Level(1)),
        (4.0, SirenTone::Level(2)),
        (4.0, SirenTone::Level(3)),
        (0.8, SirenTone::Blip),
        (4.0, SirenTone::Level(1)),
        (1.0, SirenTone::Off),
    ];
    let mut p = SirenPlayer::new(Arc::clone(&s), 0, RATE);
    let mut samples: Vec<i16> = Vec::new();
    let mut t = 0.0;
    for (len, tone) in script {
        println!("{t:5.1} s: {tone:?}");
        for _ in 0..(len * RATE as f32) as usize {
            samples.push((p.next_sample(tone) * 32767.0 * 0.5).clamp(-32768.0, 32767.0) as i16);
        }
        t += len;
    }
    let pcm = mw_audio::Pcm { samples, channels: 1, sample_rate: RATE };
    std::fs::write(out, pcm.to_wav())?;
    println!("set {:016x}: {:.1} s (× 0.5) -> {}", s.id.0, t, out.display());
    Ok(())
}

/// `mwtool speech`: the police radio's speech events (`mw_assets::police_speech`); with an event id, its items,
/// and with `--out <dir>` every line of it decoded to `<wave>.wav`.
pub fn speech(store: &AssetStore, event: Option<u32>, out: Option<&Path>) -> Result<()> {
    let d = store.speech_data()?;
    println!(
        "{} speech events, {} waves in the container ({})",
        d.events.len(),
        d.waves.len(),
        d.container.as_ref().map_or("missing".into(), |(_, p)| p.clone())
    );
    let mut ids: Vec<&u32> = d.events.keys().collect();
    ids.sort();
    for id in ids {
        let e = &d.events[id];
        if event.is_some_and(|x| x != *id) {
            continue;
        }
        let lines: usize = e
            .items
            .iter()
            .map(|i| match i {
                mw_assets::police_speech::SpeechItem::Lines { lines, .. } => lines.len(),
                _ => 0,
            })
            .sum();
        println!(
            "{id}: priority {} delay {} period {} min, max {}, {} items, {} lines{}{}",
            e.priority,
            e.delay,
            e.period,
            e.max_plays,
            e.items.len(),
            lines,
            if e.conditions.is_empty() {
                String::new()
            } else {
                format!(", if {}", e.conditions.join(if e.all_conditions { " & " } else { " | " }))
            },
            if e.one_off { ", one-off" } else { "" }
        );
        if event.is_none() {
            continue;
        }
        for it in &e.items {
            match it {
                mw_assets::police_speech::SpeechItem::Lines { start, probability, lines } => {
                    println!("  @{start} p {probability}: {} lines", lines.len());
                    if let Some(dir) = out {
                        std::fs::create_dir_all(dir)?;
                        for l in lines {
                            let Some(pcm) = d.line_pcm(l.wave) else {
                                println!("    {}: not decoded", l.wave);
                                continue;
                            };
                            let path = dir.join(format!("{}.wav", l.wave));
                            std::fs::write(&path, pcm.to_wav())?;
                            println!(
                                "    {} ({:.2} s, gain {:.3}) -> {}",
                                l.wave,
                                pcm.samples.len() as f32 / pcm.sample_rate as f32 / pcm.channels.max(1) as f32,
                                l.gain,
                                path.display()
                            );
                        }
                    }
                }
                mw_assets::police_speech::SpeechItem::Events { start, events } => {
                    println!("  @{start} event of {events:?}")
                }
            }
        }
    }
    Ok(())
}

fn lights(store: &AssetStore, car: &str) -> Result<()> {
    let v = store.find_vehicle(car)?.context("no such car")?;
    let bundle = v.hi_bundle.context("no bundle")?;
    let b = store.bundle(&bundle).context("bundle not found")?;
    let e = b.entries_of_type(mw_bundle::ResourceType::VehicleGraphicsSpec).next().context("no VehicleGraphicsSpec")?;
    let spec = mw_formats::VehicleGraphicsSpec::parse(&b.load(e)?)?;
    let l = store.car_flash_lights(&b, &spec);
    println!(
        "{} ({bundle}): siren {}, {} coronas, {} lights in group 3",
        v.name,
        l.has_siren,
        l.coronas.len(),
        l.lights.len()
    );
    for c in &l.coronas {
        let frames: String = (0..32)
            .map(|k| match c.animation.as_ref().map(|a| a.value(k as f32 / 8.0)) {
                Some(x) if x > 0.66 => '#',
                Some(x) if x > 0.33 => '+',
                Some(x) if x > 0.0 => '.',
                Some(_) => ' ',
                None => '-',
            })
            .collect();
        println!(
            "  corona gate {} colour {:.2?} bright {} size {:?} {} locators [{frames}]",
            c.gate,
            &c.colour[..3],
            c.brightness,
            c.size,
            c.locators.len()
        );
    }
    for li in &l.lights {
        println!(
            "  light {:016x} colour {:.2?} intensity {} threshold {} gate {} {} locators at {:?}",
            li.def.id.0,
            li.def.colour,
            li.def.intensity,
            li.def.attenuation_threshold,
            li.gate,
            li.locators.len(),
            li.locators.first().map(|l| l.position)
        );
    }
    Ok(())
}
