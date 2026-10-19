//! `MW_AUDIO_TAP=<file.wav>`: an offline render of the player car's engine bank and vehicle sound bank (tyres,
//! road, wind) in game time, for measuring their levels in muted and headless runs (`--test drift` runs at 4x real
//! time; the output device plays nothing). Its own players are fed the same parameters as the live ones, and each
//! frame renders the frame's game time: a two-channel 32-bit float .wav (0 = engine, 1 = vehicle sounds, both
//! after the source's soft limiter; their mix volumes are equal at the default settings, `settings::Channel`).
//! Next to it, `<file>.voices`: the game time at the first sample (`# start`), then every 0.25 s of game time the
//! vehicle bank's sounding wave voices (`seconds bank sequence index, sequence id, item, gain`; seconds of
//! `Time::elapsed`, as `MW_SFX_TRACE`). Loads the engine and vehicle banks also when the run is muted.

use std::io::{Seek, SeekFrom, Write};
use std::sync::Mutex;

use bevy::prelude::*;

pub fn on() -> bool {
    std::env::var_os("MW_AUDIO_TAP").is_some()
}

struct Tap {
    engine: Option<Box<mw_audio::sequence::Player>>,
    sfx: Option<Box<mw_audio::sfx_player::Player>>,
    file: Option<std::fs::File>,
    voices: Option<std::fs::File>,
    frames: u64,
    owed: f64,
    t: f64,
    next_dump: f64,
}

static TAP: Mutex<Option<Tap>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Tap) -> R) -> R {
    let mut g = TAP.lock().unwrap();
    let tap = g.get_or_insert_with(|| Tap {
        engine: None,
        sfx: None,
        file: None,
        voices: None,
        frames: 0,
        owed: 0.0,
        t: 0.0,
        next_dump: 0.0,
    });
    f(tap)
}

/// The player's engine bank (`audio.rs`).
pub fn add_engine(p: mw_audio::sequence::Player) {
    if on() {
        with(|t| t.engine = Some(Box::new(p)));
    }
}

/// The vehicle sound bank (`sfx.rs`).
pub fn add_sfx(p: mw_audio::sfx_player::Player) {
    if on() {
        with(|t| t.sfx = Some(Box::new(p)));
    }
}

pub struct AudioTapPlugin;

impl Plugin for AudioTapPlugin {
    fn build(&self, app: &mut App) {
        if on() {
            app.add_systems(PostUpdate, render);
        }
    }
}

const RATE: u32 = mw_audio::sfx_player::OUTPUT_RATE;

fn header(frames: u64) -> Vec<u8> {
    let data = (frames * 8) as u32;
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&RATE.to_le_bytes());
    h.extend_from_slice(&(RATE * 8).to_le_bytes());
    h.extend_from_slice(&8u16.to_le_bytes());
    h.extend_from_slice(&32u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data.to_le_bytes());
    h
}

fn render(time: Res<Time>) {
    let dt = time.delta_secs_f64();
    with(|tap| {
        if tap.engine.is_none() && tap.sfx.is_none() {
            return;
        }
        if tap.file.is_none() {
            let path = std::env::var_os("MW_AUDIO_TAP").unwrap();
            let mut vpath = path.clone();
            vpath.push(".voices");
            tap.voices = std::fs::File::create(vpath).ok();
            if let Some(f) = &mut tap.voices {
                _ = writeln!(f, "# start {:.4}", time.elapsed_secs_f64() - dt);
            }
            tap.file = std::fs::File::create(path).ok();
            if let Some(f) = &mut tap.file {
                _ = f.write_all(&header(0));
            }
        }
        tap.owed += dt * RATE as f64;
        let n = tap.owed.floor() as usize;
        tap.owed -= n as f64;
        let mut buf = Vec::with_capacity(n * 8);
        for _ in 0..n {
            let e = tap.engine.as_mut().map_or(0.0, |p| p.next_sample());
            let s = tap.sfx.as_mut().map_or(0.0, |p| p.next_sample());
            buf.extend_from_slice(&e.to_le_bytes());
            buf.extend_from_slice(&s.to_le_bytes());
        }
        tap.frames += n as u64;
        tap.t += dt;
        let frames = tap.frames;
        if let Some(f) = &mut tap.file {
            _ = f.write_all(&buf);
            _ = f.seek(SeekFrom::Start(0)).and_then(|_| f.write_all(&header(frames)));
            _ = f.seek(SeekFrom::End(0));
        }
        if tap.t >= tap.next_dump {
            tap.next_dump = tap.t + 0.25;
            let t = time.elapsed_secs_f64();
            if let (Some(p), Some(f)) = (&tap.sfx, &mut tap.voices) {
                for (si, id, item, gain) in p.voices() {
                    _ = writeln!(f, "{t:.3} {si} {id:016x} {item} {gain:.4}");
                }
            }
        }
    });
}
