//! Debug detector for sudden brightness jumps ("flashbangs"): `MW_FLASH_LOG=1` (log to stderr) or
//! `MW_FLASH_LOG=<file>` (also one line per frame to the file). Not part of the game.
//!
//! After the tonemap, `flash_probe.wgsl` reduces the frame onto a 32 × 32 grid (mean displayed luma, count of
//! non-finite HDR pixels, brightest HDR luma, adapted exposure), which is read back every frame. A frame whose
//! mean displayed luma jumps well above the last second's median is logged as `FLASH` with the camera, the
//! environment and the exposure state; `MW_FLASH_SHOTS=<dir>` also saves a screenshot of it (at most 20).

use std::collections::VecDeque;
use std::io::Write;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

/// Edge of the probe grid; must match `GRID` in flash_probe.wgsl.
pub const GRID: u32 = 32;

pub fn enabled() -> bool {
    std::env::var_os("MW_FLASH_LOG").is_some()
}

/// The probe's target (read back each frame).
#[derive(Resource, Clone, ExtractResource)]
pub struct FlashProbe(pub Handle<Image>);

pub struct FlashProbePlugin;

impl Plugin for FlashProbePlugin {
    fn build(&self, app: &mut App) {
        if !enabled() {
            return;
        }
        app.add_plugins(ExtractResourcePlugin::<FlashProbe>::default()).add_systems(Startup, setup);
    }
}

#[derive(Default)]
struct History {
    frame: u64,
    means: VecDeque<f32>,
    file: Option<std::fs::File>,
    shots: u32,
    last_flash: u64,
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let mut image = Image::new_fill(
        Extent3d { width: GRID, height: GRID, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0; 16],
        TextureFormat::Rgba32Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage |=
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC | TextureUsages::TEXTURE_BINDING;
    let handle = images.add(image);
    commands.insert_resource(FlashProbe(handle.clone()));
    let mut history = History::default();
    if let Some(path) = std::env::var_os("MW_FLASH_LOG").filter(|p| p != "1") {
        match std::fs::File::create(&path) {
            Ok(mut f) => {
                let _ = writeln!(
                    f,
                    "# frame time dt mean white_cells nonfinite peak_hdr exposure min max bias cam_x cam_y cam_z \
                     fwd_x fwd_y fwd_z timeline hour"
                );
                history.file = Some(f);
            }
            Err(e) => warn!("MW_FLASH_LOG {}: {e}", path.to_string_lossy()),
        }
    }
    let history = std::sync::Mutex::new(history);
    commands.spawn(Readback::texture(handle)).observe(
        move |ev: On<ReadbackComplete>,
              mut commands: Commands,
              time: Res<Time>,
              camera: Query<&GlobalTransform, crate::MainCamera>,
              env: Option<Res<crate::weather::EnvironmentState>>,
              tonemap: Option<Res<crate::tonemap::GameTonemap>>| {
            let Ok(mut h) = history.lock() else { return };
            let floats: Vec<f32> = ev.data.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
            let cells = (GRID * GRID) as usize;
            if floats.len() < cells * 4 {
                return;
            }
            let mut mean = 0.0;
            let mut white = 0;
            let mut bad = 0.0;
            let mut peak = 0.0f32;
            for c in floats.as_chunks::<4>().0.iter().take(cells) {
                mean += c[0];
                white += usize::from(c[0] > 0.8);
                bad += c[1];
                peak = peak.max(c[2]);
            }
            mean /= cells as f32;
            let exposure = floats[3];
            h.frame += 1;
            let frame = h.frame;
            let (pos, fwd) =
                camera.iter().next().map_or((Vec3::ZERO, Vec3::ZERO), |t| (t.translation(), t.forward().as_vec3()));
            let (timeline, hour) = env.as_ref().map_or((0, 0.0), |e| (e.want.timeline.0 & 0xFF_FFFF, e.want.hour));
            let (min, max, bias) =
                tonemap.as_ref().map_or((0.0, 0.0, 0.0), |t| (t.settings.min, t.settings.max, t.settings.bias));
            let t = time.elapsed_secs();
            let dt = time.delta_secs();
            let line = format!(
                "{frame} {t:.3} {dt:.4} {mean:.4} {white} {bad} {peak:.3} {exposure:.3} {min:.2} {max:.2} {bias:.2} \
                 {:.1} {:.1} {:.1} {:.2} {:.2} {:.2} {timeline:x} {hour:.2}",
                pos.x, pos.y, pos.z, fwd.x, fwd.y, fwd.z
            );
            if let Some(f) = h.file.as_mut() {
                let _ = writeln!(f, "{line}");
            }
            if bad > 0.0 {
                warn!("FLASH-PROBE nonfinite: {line}");
            }
            let mut sorted: Vec<f32> = h.means.iter().copied().collect();
            sorted.sort_by(f32::total_cmp);
            if sorted.len() >= 20 {
                let median = sorted[sorted.len() / 2];
                if mean > median + 0.12 && mean > median * 1.5 {
                    warn!("FLASH mean {mean:.3} (median {median:.3}): {line}");
                    if let Some(dir) = std::env::var_os("MW_FLASH_SHOTS")
                        && h.shots < 20
                        && frame > h.last_flash + 30
                    {
                        h.shots += 1;
                        let path = std::path::Path::new(&dir).join(format!("flash_{frame}.png"));
                        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
                    }
                    h.last_flash = frame;
                }
            }
            h.means.push_back(mean);
            if h.means.len() > 60 {
                h.means.pop_front();
            }
        },
    );
}
