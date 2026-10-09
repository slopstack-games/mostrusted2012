//! Movies (`UI/MOVIES/<id>.VP6`, EA `MVhd` containers with VP6 video) for the front end's `MovieSequenceItem`s,
//! decoded by an installed `ffmpeg` (model decision: no VP6 decoder of our own yet, PLAN M7 "decode through
//! ffmpeg or skip"). Without ffmpeg, or for movies it cannot decode (several of the short 1.5 s clips fail
//! with "Invalid data"), the screen shows what lies under the movie (the title's white backdrop).
//!
//! A thread runs `ffmpeg -i <movie> -f rawvideo -pix_fmt rgba` at a reduced size and hands frames over a
//! bounded channel; [`Movie::update`] shows the frame for the movie's clock (30 fps, the VP6 streams' rate).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

use bevy::prelude::*;

/// Decode size (the movies are 1280×720; half keeps the CPU cost low).
const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;
/// The VP6 streams' frame rate (`ffprobe`: 30000/1000 for every title movie).
const FPS: f32 = 30.0;

pub struct Movie {
    pub id: u32,
    rx: std::sync::Mutex<Receiver<Vec<u8>>>,
    child: Option<Child>,
    pub image: Handle<Image>,
    /// Frames shown so far.
    shown: u32,
    /// A frame has been shown (the image holds a picture).
    pub ready: bool,
    pub ended: bool,
    looping: bool,
    path: PathBuf,
}

fn spawn(path: &Path) -> Option<(Child, Receiver<Vec<u8>>)> {
    // A clean environment: the inherited one can exceed the exec limit (E2BIG seen in some sessions).
    let mut child = Command::new("ffmpeg")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()))
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-vf", &format!("scale={WIDTH}:{HEIGHT}"), "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| warn!("ffmpeg for {}: {e}", path.display()))
        .ok()?;
    let mut out = child.stdout.take()?;
    let (tx, rx): (SyncSender<Vec<u8>>, _) = sync_channel(8);
    std::thread::Builder::new()
        .name("mw-movie".into())
        .spawn(move || {
            let size = (WIDTH * HEIGHT * 4) as usize;
            loop {
                let mut frame = vec![0u8; size];
                if out.read_exact(&mut frame).is_err() {
                    return;
                }
                // Blocks while the main thread is behind; ends when the movie is dropped.
                let mut f = frame;
                loop {
                    match tx.try_send(f) {
                        Ok(()) => break,
                        Err(TrySendError::Full(back)) => {
                            f = back;
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                        Err(TrySendError::Disconnected(_)) => return,
                    }
                }
            }
        })
        .ok()?;
    Some((child, rx))
}

impl Movie {
    /// Start decoding `UI/MOVIES/<id>.VP6`; `None` when the file or ffmpeg is missing.
    pub fn open(game: &mw_gamedir::GameDir, id: u32, looping: bool, images: &mut Assets<Image>) -> Option<Self> {
        let path = game.path(&format!("UI/MOVIES/{id}.VP6"))?;
        let (child, rx) = spawn(&path)?;
        let image = images.add(crate::game_text::ui_image(WIDTH, HEIGHT, vec![0; (WIDTH * HEIGHT * 4) as usize]));
        Some(Self {
            id,
            rx: std::sync::Mutex::new(rx),
            child: Some(child),
            image,
            shown: 0,
            ready: false,
            ended: false,
            looping,
            path,
        })
    }

    /// Show the frame for `seconds` into the movie (frames are only skipped forward).
    pub fn update(&mut self, seconds: f32, images: &mut Assets<Image>) {
        let want = (seconds.max(0.0) * FPS) as u32 + 1;
        let mut latest = None;
        while self.shown < want {
            let r = self.rx.get_mut().unwrap().try_recv();
            match r {
                Ok(f) => {
                    latest = Some(f);
                    self.shown += 1;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if self.looping && self.shown > 0 {
                        // Start over; the clock keeps running, so restart the frame count with it.
                        if let Some((child, rx)) = spawn(&self.path) {
                            self.stop();
                            self.child = Some(child);
                            self.rx = std::sync::Mutex::new(rx);
                            self.shown = want.saturating_sub(1);
                            continue;
                        }
                    }
                    self.ended = true;
                    break;
                }
            }
        }
        if let Some(f) = latest
            && let Some(mut img) = images.get_mut(&self.image)
        {
            *img = crate::game_text::ui_image(WIDTH, HEIGHT, f);
            self.ready = true;
        }
    }

    fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for Movie {
    fn drop(&mut self) {
        self.stop();
    }
}
