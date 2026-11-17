//! Window-size switches for checking the UI at other sizes headless (`tools/hl` with `MW_HL_SIZE=WxH`):
//! `MW_WINDOW=WxH` sizes the window at startup, `MW_RESIZE_AT=<frame>,WxH` resizes it at that frame (a runtime
//! resize, as a player dragging the window). Sizes are logical pixels.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

pub struct WindowTestPlugin;

impl Plugin for WindowTestPlugin {
    fn build(&self, app: &mut App) {
        let mut steps = Vec::new();
        if let Some(s) = std::env::var("MW_WINDOW").ok().and_then(|v| size(&v)) {
            steps.push((0, s));
        }
        if let Some((f, s)) = std::env::var("MW_RESIZE_AT").ok().and_then(|v| {
            let (f, s) = v.split_once(',')?;
            Some((f.trim().parse::<u32>().ok()?, size(s)?))
        }) {
            steps.push((f, s));
        }
        if !steps.is_empty() {
            app.insert_resource(Resizes { steps, frame: 0 }).add_systems(Update, resize);
        }
    }
}

fn size(v: &str) -> Option<Vec2> {
    let (w, h) = v.trim().split_once(['x', 'X'])?;
    Some(Vec2::new(w.parse().ok()?, h.parse().ok()?))
}

#[derive(Resource)]
struct Resizes {
    steps: Vec<(u32, Vec2)>,
    frame: u32,
}

fn resize(mut r: ResMut<Resizes>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let frame = r.frame;
    r.frame += 1;
    let Ok(mut window) = windows.single_mut() else { return };
    for &(_, s) in r.steps.iter().filter(|(f, _)| *f == frame) {
        info!("window: resizing to {}x{} at frame {frame}", s.x, s.y);
        window.resolution.set(s.x, s.y);
    }
}
