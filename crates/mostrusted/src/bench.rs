//! `--bench <seconds>`: frame-time benchmark for the headless runs.
//!
//! After the world has finished loading (`Loaded::done`) and a warm-up of `WARMUP` seconds, records the
//! frame time from Bevy's `FrameTimeDiagnosticsPlugin` for the requested time, then prints one
//! `BENCH RESULT` line and exits. Present mode is unthrottled (`AutoNoVsync`, set in `main`). With
//! `MW_BENCH_PASSES=1` the render passes' CPU / GPU times (Bevy's `RenderDiagnosticsPlugin`, timestamp
//! queries on Vulkan) are printed as `BENCH PASS` lines too; that plugin adds queries to every pass,
//! so it is off for the headline numbers.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;

use crate::Loaded;

/// Render-world CPU time (ns) since the bench started: extract .. bind groups (prepare + queue, including
/// the swapchain acquire, which waits on the GPU), and the `Render` set (graph encoding + submit + present).
static RENDER_PREPARE_NS: AtomicU64 = AtomicU64::new(0);
static RENDER_GRAPH_NS: AtomicU64 = AtomicU64::new(0);
static RENDER_FRAMES: AtomicU64 = AtomicU64::new(0);
static RENDER_MEASURE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Resource, Default)]
struct RenderMarks {
    begin: Option<Instant>,
    graph: Option<Instant>,
}

fn add_ns(a: &AtomicU64, since: Option<Instant>) {
    if let Some(t) = since {
        a.fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }
}

/// Seconds after loading before measuring (GPU uploads, pipeline compiles, the car settling).
const WARMUP: f32 = 4.0;

pub struct BenchPlugin {
    pub seconds: f32,
    pub label: String,
}

#[derive(Resource)]
struct Bench {
    seconds: f32,
    label: String,
    since_done: Option<Instant>,
    start: Option<Instant>,
    frames_ms: Vec<f64>,
    /// Main-world schedule time (First .. Last) per measured frame.
    main_ms: Vec<f64>,
    main_start: Option<Instant>,
    finished: bool,
    /// Mesh and image asset counts when measuring started (loading should be over by then).
    assets_at_start: (usize, usize),
}

impl Plugin for BenchPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<FrameTimeDiagnosticsPlugin>() {
            app.add_plugins(FrameTimeDiagnosticsPlugin::default());
        }
        let passes = std::env::var_os("MW_BENCH_PASSES").is_some();
        if passes {
            app.add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
        }
        app.insert_resource(Bench {
            seconds: self.seconds,
            label: self.label.clone(),
            since_done: None,
            start: None,
            frames_ms: Vec::new(),
            main_ms: Vec::new(),
            main_start: None,
            finished: false,
            assets_at_start: (0, 0),
        });
        use bevy::render::{Render, RenderApp, RenderSystems};
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render
                .init_resource::<RenderMarks>()
                .add_systems(
                    Render,
                    (|mut m: ResMut<RenderMarks>| m.begin = Some(Instant::now()))
                        .in_set(RenderSystems::ExtractCommands),
                )
                .add_systems(
                    Render,
                    (|mut m: ResMut<RenderMarks>| m.graph = Some(Instant::now()))
                        .after(RenderSystems::PrepareBindGroups)
                        .before(RenderSystems::Render),
                )
                .add_systems(
                    Render,
                    (|m: Res<RenderMarks>| {
                        if RENDER_MEASURE.load(Ordering::Relaxed) {
                            add_ns(&RENDER_GRAPH_NS, m.graph);
                            if let (Some(b), Some(g)) = (m.begin, m.graph) {
                                RENDER_PREPARE_NS.fetch_add((g - b).as_nanos() as u64, Ordering::Relaxed);
                            }
                            RENDER_FRAMES.fetch_add(1, Ordering::Relaxed);
                        }
                    })
                    .after(RenderSystems::Render)
                    .before(RenderSystems::Cleanup),
                );
        }
        app.add_systems(First, |mut b: ResMut<Bench>| b.main_start = Some(Instant::now()))
            .add_systems(Last, |b: Res<Bench>| {
                RENDER_MEASURE.store(b.start.is_some() && !b.finished, Ordering::Relaxed)
            })
            .add_systems(Last, (record, report).chain());
    }
}

fn record(
    mut bench: ResMut<Bench>,
    loaded: Res<Loaded>,
    store: Res<DiagnosticsStore>,
    meshes: Res<Assets<Mesh>>,
    images: Res<Assets<Image>>,
) {
    let main_ms = bench.main_start.map(|t| t.elapsed().as_secs_f64() * 1000.0);
    if !loaded.done || bench.finished {
        return;
    }
    let now = Instant::now();
    let since = *bench.since_done.get_or_insert(now);
    if since.elapsed().as_secs_f32() < WARMUP {
        return;
    }
    let start = *bench.start.get_or_insert(now);
    if start == now {
        bench.assets_at_start = (meshes.len(), images.len());
        return; // the first frame's time includes the warm-up's last frame
    }
    if let Some(ft) = store.get(&FrameTimeDiagnosticsPlugin::FRAME_TIME).and_then(|d| d.value()) {
        bench.frames_ms.push(ft);
    }
    if let Some(m) = main_ms {
        bench.main_ms.push(m);
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<f64>().ok())
        })
        .map_or(0.0, |kb| kb / 1024.0)
}

#[allow(clippy::too_many_arguments)]
fn report(
    mut bench: ResMut<Bench>,
    store: Res<DiagnosticsStore>,
    entities: Query<Entity>,
    meshes: Query<(&ViewVisibility, &InheritedVisibility), With<Mesh3d>>,
    adapter: Option<Res<RenderAdapterInfo>>,
    cascades: Query<&bevy::camera::visibility::CascadesVisibleEntities>,
    texels: Query<&bevy::light::Cascades>,
    loaded: Res<Loaded>,
    mesh_assets: Res<Assets<Mesh>>,
    image_assets: Res<Assets<Image>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(start) = bench.start else { return };
    if bench.finished || start.elapsed().as_secs_f32() < bench.seconds {
        return;
    }
    bench.finished = true;
    let mut ft = bench.frames_ms.clone();
    ft.sort_by(f64::total_cmp);
    let n = ft.len().max(1);
    let mean = ft.iter().sum::<f64>() / n as f64;
    // 1 % low: the mean of the slowest 1 % of frames, as frame time.
    let worst = (n / 100).max(1);
    let low1 = ft.iter().rev().take(worst).sum::<f64>() / worst as f64;
    let main_mean = bench.main_ms.iter().sum::<f64>() / bench.main_ms.len().max(1) as f64;
    let (visible, mesh_entities) = meshes.iter().fold((0, 0), |(v, m), (vv, _)| (v + usize::from(vv.get()), m + 1));
    let rf = RENDER_FRAMES.load(Ordering::Relaxed).max(1) as f64;
    let prep = RENDER_PREPARE_NS.load(Ordering::Relaxed) as f64 / rf / 1e6;
    let graph = RENDER_GRAPH_NS.load(Ordering::Relaxed) as f64 / rf / 1e6;
    let casters: Vec<String> = cascades
        .iter()
        .flat_map(|c| c.entities.values())
        .map(|v| v.iter().map(|c| c.entities.len().to_string()).collect::<Vec<_>>().join("/"))
        .collect();
    let texel: Vec<String> = texels
        .iter()
        .flat_map(|c| c.cascades.values())
        .map(|v| v.iter().map(|c| format!("{:.3}", c.texel_size)).collect::<Vec<_>>().join("/"))
        .collect();
    let window_system = if std::env::var_os("WAYLAND_DISPLAY").is_some() { "Wayland, vsync-paced" } else { "X11" };
    let gpu = adapter.map_or("unknown".to_string(), |a| format!("{} ({:?}, {window_system})", a.name, a.backend));
    println!(
        "BENCH RESULT {}: {} frames in {:.1} s, frame mean {:.2} ms, median {:.2} ms, 1% low {:.2} ms \
         ({:.0} fps mean), main-world schedule {:.2} ms, render world {:.2} ms prepare + {:.2} ms graph, \
         shadow casters per cascade {} (texel {} m), {} entities, {} mesh entities ({} visible), \
         {} instances, meshes {} -> {}, images {} -> {} during the run, RSS {:.0} MB, GPU {}",
        bench.label,
        ft.len(),
        start.elapsed().as_secs_f32(),
        mean,
        percentile(&ft, 0.5),
        low1,
        1000.0 / mean,
        main_mean,
        prep,
        graph,
        casters.join(" "),
        texel.join(" "),
        entities.iter().count(),
        mesh_entities,
        visible,
        loaded.instances,
        bench.assets_at_start.0,
        mesh_assets.len(),
        bench.assets_at_start.1,
        image_assets.len(),
        rss_mb(),
        gpu,
    );
    // Render pass timings (only with MW_BENCH_PASSES, from RenderDiagnosticsPlugin).
    let mut lines: Vec<(String, f64)> = store
        .iter()
        .filter(|d| d.path().as_str().starts_with("render/"))
        .filter_map(|d| Some((d.path().as_str().to_string(), d.average()?)))
        .collect();
    lines.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, v) in lines {
        println!("BENCH PASS {path} {v:.3}");
    }
    exit.write(AppExit::Success);
}

/// `--no-shadows`: the key light without shadow maps (attribution runs only).
pub fn disable_shadows(mut lights: Query<&mut DirectionalLight, Changed<DirectionalLight>>) {
    for mut l in &mut lights {
        if l.shadow_maps_enabled {
            l.shadow_maps_enabled = false;
        }
    }
}
