//! `MW_FRAME_LOG=<file>`: one line per frame with where the time went, to find hitches.
//!
//! Main world: the time of each main schedule (First, PreUpdate, the fixed-step loop, Update, PostUpdate,
//! Last), measured by marker schedules inserted between them, and the gap from the end of `Last` to the next
//! `First` (extraction into the render world, waiting for the render thread, window events). Render world (the
//! frame finished most recently on the render thread): extract commands .. meshes / asset uploads, views and
//! pipeline specialisation, queue + sort, prepare (batching, buffers, bind groups) and the graph (encode,
//! submit, present). Plus the fixed steps run, the loader messages applied and their time, entity and asset
//! counts. Columns are named in the header line. Costs nothing when the variable is unset (nothing is added).

use crate::platform::Instant;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use bevy::app::MainScheduleOrder;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;

/// Loader messages applied this frame and the time spent (`receive_assets`), set by `main.rs`.
pub static LOADER_MSGS: AtomicU64 = AtomicU64::new(0);
pub static LOADER_NS: AtomicU64 = AtomicU64::new(0);

/// Render-world segment times (ns) of the last finished render frame.
static RENDER: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct AfterFirst;
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct AfterPreUpdate;
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct AfterFixed;
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct AfterUpdate;
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct AfterPostUpdate;

/// Marks: First start, after First, PreUpdate, fixed loop, Update, PostUpdate; then Last end.
#[derive(Resource)]
struct Marks {
    t: [Option<Instant>; 6],
    last_end: Option<Instant>,
    prev_first: Option<Instant>,
    start: Instant,
    frame: u64,
    fixed_steps: u32,
    out: Mutex<std::io::BufWriter<std::fs::File>>,
}

#[derive(Resource, Default)]
struct RenderMarks([Option<Instant>; 6]);

pub struct FrameLogPlugin;

impl Plugin for FrameLogPlugin {
    fn build(&self, app: &mut App) {
        let Some(path) = std::env::var_os("MW_FRAME_LOG") else { return };
        let file = match std::fs::File::create(&path) {
            Ok(f) => f,
            Err(e) => {
                warn!("MW_FRAME_LOG {}: {e}", path.to_string_lossy());
                return;
            }
        };
        let mut out = std::io::BufWriter::new(file);
        let _ = writeln!(
            out,
            "frame t dt first pre fixed update post last gap r_assets r_spec r_queue r_prepare r_graph steps \
             loader_msgs loader_ms entities meshes images materials"
        );
        app.insert_resource(Marks {
            t: [None; 6],
            last_end: None,
            prev_first: None,
            start: Instant::now(),
            frame: 0,
            fixed_steps: 0,
            out: Mutex::new(out),
        });
        {
            let mut order = app.world_mut().resource_mut::<MainScheduleOrder>();
            order.insert_after(First, AfterFirst);
            order.insert_after(PreUpdate, AfterPreUpdate);
            order.insert_after(RunFixedMainLoop, AfterFixed);
            order.insert_after(Update, AfterUpdate);
            order.insert_after(PostUpdate, AfterPostUpdate);
        }
        fn mark<const I: usize>(mut m: ResMut<Marks>) {
            m.t[I] = Some(Instant::now());
        }
        app.add_systems(First, mark::<0>.before(bevy::time::TimeSystems))
            .add_systems(AfterFirst, mark::<1>)
            .add_systems(AfterPreUpdate, mark::<2>)
            .add_systems(AfterFixed, mark::<3>)
            .add_systems(AfterUpdate, mark::<4>)
            .add_systems(AfterPostUpdate, mark::<5>)
            .add_systems(FixedFirst, |mut m: ResMut<Marks>| m.fixed_steps += 1)
            .add_systems(Last, write_line.after(bevy::window::exit_on_all_closed));

        use bevy::render::{Render, RenderApp, RenderSystems as S};
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            fn rmark<const I: usize>(mut m: ResMut<RenderMarks>) {
                m.0[I] = Some(Instant::now());
            }
            render
                .init_resource::<RenderMarks>()
                .add_systems(Render, rmark::<0>.in_set(S::ExtractCommands))
                .add_systems(Render, rmark::<1>.after(S::PrepareMeshes).before(S::CreateViews))
                .add_systems(Render, rmark::<2>.after(S::PrepareViews).before(S::Queue))
                .add_systems(Render, rmark::<3>.after(S::PhaseSort).before(S::Prepare))
                .add_systems(Render, rmark::<4>.after(S::Prepare).before(S::Render))
                .add_systems(
                    Render,
                    (|m: Res<RenderMarks>| {
                        let now = Instant::now();
                        #[allow(clippy::needless_range_loop)]
                        for i in 0..5 {
                            let (Some(a), b) = (m.0[i], if i < 4 { m.0[i + 1] } else { Some(now) }) else {
                                continue;
                            };
                            let ns = b.map_or(0, |b| b.saturating_duration_since(a).as_nanos() as u64);
                            RENDER[i].store(ns, Ordering::Relaxed);
                        }
                    })
                    .after(S::Render)
                    .before(S::Cleanup),
                );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn write_line(
    mut m: ResMut<Marks>,
    archetypes: &bevy::ecs::archetype::Archetypes,
    meshes: Res<Assets<Mesh>>,
    images: Res<Assets<Image>>,
    materials: Res<Assets<crate::materials::GameMaterial>>,
) {
    let now = Instant::now();
    let ms = |a: Option<Instant>, b: Option<Instant>| match (a, b) {
        (Some(a), Some(b)) => b.saturating_duration_since(a).as_secs_f64() * 1000.0,
        _ => 0.0,
    };
    let t = m.t;
    let dt = ms(m.prev_first, t[0]);
    let gap = ms(m.last_end, t[0]);
    let r: Vec<f64> = RENDER.iter().map(|a| a.load(Ordering::Relaxed) as f64 / 1e6).collect();
    let line = format!(
        "{} {:.3} {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} {} {} {:.2} {} {} {} {}",
        m.frame,
        t[0].map_or(0.0, |t| t.duration_since(m.start).as_secs_f64()),
        dt,
        ms(t[0], t[1]),
        ms(t[1], t[2]),
        ms(t[2], t[3]),
        ms(t[3], t[4]),
        ms(t[4], t[5]),
        ms(t[5], Some(now)),
        gap,
        r[0],
        r[1],
        r[2],
        r[3],
        r[4],
        m.fixed_steps,
        LOADER_MSGS.swap(0, Ordering::Relaxed),
        LOADER_NS.swap(0, Ordering::Relaxed) as f64 / 1e6,
        archetypes.iter().map(|a| a.len() as usize).sum::<usize>(),
        meshes.len(),
        images.len(),
        materials.len(),
    );
    if let Ok(mut out) = m.out.lock() {
        let _ = writeln!(out, "{line}");
        if m.frame.is_multiple_of(300) {
            let _ = out.flush();
        }
    }
    m.frame += 1;
    m.fixed_steps = 0;
    m.prev_first = t[0];
    m.last_end = Some(Instant::now());
}

impl Drop for Marks {
    fn drop(&mut self) {
        if let Ok(mut out) = self.out.lock() {
            let _ = out.flush();
        }
    }
}
