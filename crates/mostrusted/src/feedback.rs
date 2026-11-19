//! The runtime side of the UI message manager's feedback queues (`mw_logic::natives::feedback_queue`, which holds
//! the queues, posts and the per-frame rules; research log 2026-11-18 "the HUD feedback-message queue"):
//! - **feeds** the queues: the loaded feedback groups' `Messages` (logic.rs hands `logic_loads.rs`'s loads over,
//!   [`message_def`]), the plays' lengths once their layout is loaded, and the song popup (`FUN_007a1ce0`: when a song of
//!   a list with a `FeedbackMessage` starts, the Song queue is emptied (`FUN_007b35c0`) and the message appended
//!   with the parts `TrackName`, `AlbumName`, `Artist`);
//! - **runs** the update ([`FeedbackQueues::update`](mw_logic::natives::feedback_queue::FeedbackQueues::update),
//!   `FUN_007bb160`) once a frame after the logic's tick;
//! - **plays** each queue's layout (`LayoutStyle`, a `UILayout`, placed by the message's `UILayoutInstanceParams`)
//!   through `ui_sequence.rs` as a one-item sequence on the play's layer: shown while the play is visible and active,
//!   faded in by the instance's visibility timeline (`FADE(0, 400, IN, EASEINOUT)` on the song popup) from each
//!   activation; its layout reads the head message's texts as `UISequences.Params[#].Text1..3` (each a language
//!   string whose `{name}` reads the message part of that name, `FUN_007c22d0`).
//!
//! Model decisions (the play object's class was not found): a play is ready once its layout is loaded; its in
//! length (+0x14) is the visibility timeline's `FADE` (delay + duration), its out length (+0x18) 0 (the hide
//! timeline `0b603df7` is empty on the messages read): an inactive play (gone out, or paused with the game) is not
//! drawn; the fade is linear (its easing not applied, as `ui_widgets.rs`); a text's `{…}` that names no part stays
//! empty (the original's fallback for those was not traced); `AlbumName` is empty (the song's album id, `c95dc017`, is 0 on the
//! licensed songs); the popup's fallback message (`DAT_0117cc2c` when group `DAT_0117cd24` is loaded) is not modelled.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mw_assets::sequence::{FeedbackMessageInfo, Item, ItemKind, Sequence};
use mw_bundle::ResourceId;
use mw_formats::binding::{BindValue, Binding};
use mw_logic::natives::feedback_queue::{Message, MessageDef, MessageEffects, Part, PartValue, SONG, SequenceLengths};

use crate::ui_sequence::{LogicView, Shown, UiSequencePlayer};

pub struct FeedbackPlugin;

/// The song popup's elements show on `Widget.HudSelect.ShowMusicChyron`, a getter of the `Widget_HudSelection`
/// widget's native view model, which was not found (decomp.md "UI renderer"; neither its name nor its hash is in the
/// exe or the live snapshot): read as true while a feedback play shows (model decision).
const SHOW_MUSIC_CHYRON: &str = "Widget.HudSelect.ShowMusicChyron";

/// The queues' update and the plays handed to the sequence player.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct FeedbackStep;

impl Plugin for FeedbackPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FeedbackPlays>()
            .add_systems(Update, step.in_set(FeedbackStep).after(crate::logic::LogicTick));
    }
}

/// The queues' view of a `FeedbackMessage`.
pub fn message_def(i: &FeedbackMessageInfo) -> MessageDef {
    MessageDef {
        id: i.id,
        layout_style: i.layout_style,
        layout_instance: i.layout_instance.map_or(0, |r| r.0),
        texts: i.texts,
        time: i.time,
        layer: i.layer,
        extend_by_sequence: i.extend_by_sequence,
        flag_e7: i.flag_e7,
        exclusive: i.exclusive,
        pause_with_game: i.pause_with_game,
        cutscene_clock: i.cutscene_clock,
        transition_message: i.transition_message,
        effects: MessageEffects {
            start_sound: i.start_sound,
            exit_sound: i.exit_sound,
            streamed_sound: i.streamed_sound,
            movie: i.movie,
            movie_sound: i.movie_sound,
            speech: i.speech,
            camera_take: i.camera_take,
            hud_mode: i.hud_mode,
        },
    }
}

type Job<T> = Arc<Mutex<Option<T>>>;

/// The plays' layouts and clocks.
#[derive(Resource, Default)]
pub struct FeedbackPlays {
    /// Layouts by UniqueId (`None`: not found).
    layouts: HashMap<u32, Option<Arc<LayoutPlay>>>,
    jobs: HashMap<u32, Job<Option<LayoutPlay>>>,
    /// Per queue: its play's clock.
    clocks: HashMap<usize, PlayClock>,
    lang: Option<Arc<mw_formats::Language>>,
    lang_job: Option<Job<Option<mw_formats::Language>>>,
    /// The last song start seen.
    song_serial: u32,
}

fn spawn<T: Send + 'static>(name: &str, f: impl FnOnce() -> T + Send + 'static) -> Job<T> {
    let job = Arc::new(Mutex::new(None));
    let out = job.clone();
    let _ = crate::platform::thread::Builder::new().name(name.into()).spawn(move || {
        let v = f();
        *out.lock().unwrap() = Some(v);
    });
    job
}

/// A message part's value for `{expr}` (`FUN_007c22d0`: the part named like the expression's path).
fn part_text(expr: &str, parts: &[Part], lang: Option<&mw_formats::Language>) -> Option<String> {
    let b = Binding::parse(expr)?;
    let p = parts.iter().find(|p| p.name == b.path)?;
    let v = match &p.value {
        PartValue::Number(n) => BindValue::Num(f64::from(*n)),
        PartValue::Text(t) => BindValue::Text(t.clone()),
        PartValue::StringId(id) => BindValue::Text(lang?.get(*id as u32).unwrap_or("").to_string()),
    };
    Some(b.format_in(&v, None))
}

/// The head message's three texts (the record's buffers, `FUN_007c3b70`).
fn texts(m: &Message, lang: Option<&mw_formats::Language>) -> [String; 3] {
    m.def.texts.map(|id| {
        let Some(t) = lang.and_then(|l| l.get(id)) else { return String::new() };
        mw_formats::ui::substitute(t, |expr| Some(part_text(expr, &m.parts, lang).unwrap_or_default()))
    })
}

#[allow(clippy::too_many_arguments)]
fn step(
    view: Option<NonSend<LogicView>>,
    game: Res<crate::Game>,
    real: Res<Time<Real>>,
    virt: Res<Time<Virtual>>,
    songs: Option<Res<crate::music::SongManager>>,
    mut plays: ResMut<FeedbackPlays>,
    player: Option<ResMut<UiSequencePlayer>>,
    mut commands: Commands,
    mut shot: Local<bool>,
) {
    let Some(view) = view else { return };
    let dt = real.delta_secs().min(virt.max_delta().as_secs_f32());
    let game_dt = (!virt.is_paused()).then(|| virt.delta_secs());
    let FeedbackPlays { layouts, jobs, clocks, lang, lang_job, song_serial } = &mut *plays;
    if lang.is_none() && lang_job.is_none() {
        let store = game.store.clone();
        *lang_job = Some(spawn("mw-feedback-text", move || store.language(mw_assets::DEFAULT_LANGUAGE).ok()));
    }
    if let Some(got) = lang_job.as_ref().and_then(|j| j.lock().unwrap().take()) {
        *lang_job = None;
        *lang = got.map(Arc::new);
    }
    let lang = lang.as_deref();
    let state = &view.0;
    // The layouts loaded for plays: ready, with their lengths.
    let done: Vec<(u32, Option<LayoutPlay>)> =
        jobs.iter().filter_map(|(&id, j)| j.lock().unwrap().take().map(|s| (id, s))).collect();
    for (id, play) in done {
        jobs.remove(&id);
        match &play {
            Some(p) => {
                let lengths = SequenceLengths { in_len: p.fade_in.0 + p.fade_in.1, out_len: 0.0 };
                state.borrow_mut().feedback_queues.sequences.insert(id, lengths);
            }
            None => warn!("feedback: no layout {id}"),
        }
        layouts.insert(id, play.map(Arc::new));
    }
    // The song popup (`FUN_007a1ce0`).
    if let Some(s) = songs.and_then(|m| m.last_start())
        && s.serial != *song_serial
    {
        *song_serial = s.serial;
        if s.message != 0 {
            let parts = vec![
                Part { name: "TrackName".into(), value: PartValue::StringId(s.title as i32) },
                Part { name: "AlbumName".into(), value: PartValue::Text(String::new()) },
                Part { name: "Artist".into(), value: PartValue::StringId(s.artist as i32) },
            ];
            let st = &mut *state.borrow_mut();
            let f = &mut st.feedback_queues;
            f.queues[SONG].clear(&mut st.layer_sets.set);
            let serial = f.post(SONG, None, s.message, "", parts);
            let queued = f.queues[SONG].queued(serial);
            info!("feedback: song popup {} (serial {serial}, queued {queued})", s.message);
        }
    }
    {
        let st = &mut *state.borrow_mut();
        let before = st.layer_sets.set;
        st.feedback_queues.update(dt, game_dt, &mut st.layer_sets.set, &mut st.requests);
        if st.layer_sets.set != before {
            info!("feedback: layer set {before} -> {} (a message's HudMode)", st.layer_sets.set);
        }
        // The UI manager's layer sets (`FUN_007a2400`; `mw_logic::natives::layer_set`): no list is ever loaded on
        // PC, so this does nothing and no layer is shown or hidden by the requested set.
        for op in st.layer_sets.update(dt, &|_| 0.0) {
            info!("layer set: {op:?}");
        }
        // The records' texts (`FUN_007c3b70` at the post), once the language is in.
        if lang.is_some() {
            for m in st.feedback_queues.queues.iter_mut().flat_map(|q| q.messages.iter_mut()) {
                if !m.texts_ready {
                    m.texts = texts(m, lang);
                    m.texts_ready = true;
                }
            }
        }
    }
    let st = state.borrow();
    let mut shown = Vec::new();
    let mut live = HashSet::new();
    for (qi, q) in st.feedback_queues.queues.iter().enumerate() {
        let Some(p) = &q.play else { continue };
        live.insert(qi);
        let c = clocks.entry(qi).or_insert(PlayClock { handle: p.handle, since_active: 0.0, active: true });
        if c.handle != p.handle {
            *c = PlayClock { handle: p.handle, since_active: 0.0, active: true };
            info!("feedback: queue {} shows layout {} (layer {})", q.name, p.sequence, p.layer);
        }
        if p.active && !c.active {
            c.since_active = 0.0;
        }
        c.active = p.active;
        let Some(play) = (match layouts.get(&p.sequence) {
            Some(l) => l.clone(),
            None => {
                if !jobs.contains_key(&p.sequence) {
                    let (store, id, instance, layer) = (game.store.clone(), p.sequence, p.instance, p.layer);
                    jobs.insert(id, spawn("mw-feedback-layout", move || load(&store, id, instance, layer)));
                }
                None
            }
        }) else {
            continue;
        };
        if !p.active {
            continue;
        }
        c.since_active += dt;
        if !p.visible {
            continue;
        }
        let (delay, duration) = play.fade_in;
        let opacity = if duration > 0.0 { ((c.since_active - delay) / duration).clamp(0.0, 1.0) } else { 1.0 };
        let texts = q.messages.first().map(|m| m.texts.clone());
        let key = format!("feedback {} {}", q.name, p.handle);
        let bindings = vec![(SHOW_MUSIC_CHYRON, BindValue::Num(1.0))];
        shown.push(Shown { key, seq: play.seq.clone(), clock: c.since_active, clocks: None, texts, opacity, bindings });
    }
    clocks.retain(|k, _| live.contains(k));
    // `MW_FEEDBACK_SHOT=<png>`: a screenshot once a play has shown for 1.5 s (headless checks).
    if !*shot
        && let Some(path) = std::env::var_os("MW_FEEDBACK_SHOT")
        && shown.iter().any(|s| s.clock >= 1.5)
    {
        *shot = true;
        info!("feedback: screenshot {}", path.to_string_lossy());
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(std::path::PathBuf::from(path)));
    }
    if let Some(mut player) = player {
        player.show_feedback(shown);
    }
}

/// A play's layout as a one-item sequence (the `LayoutSequenceItem` the sequence player draws) and its fade in.
pub struct LayoutPlay {
    seq: Arc<Sequence>,
    /// The visibility timeline's `FADE(delay, duration, IN)` (seconds).
    fade_in: (f32, f32),
}

/// A play's clock: its handle, the time since it last became active.
struct PlayClock {
    handle: u32,
    since_active: f32,
    active: bool,
}

/// The layout `id` with its instance params (`FUN_007a0df0` / `FUN_00791840`), on `layer`.
fn load(store: &mw_assets::AssetStore, id: u32, instance: u64, layer: u32) -> Option<LayoutPlay> {
    let rid = mw_formats::ui::object_id(id);
    store.ui_layout(rid).ok()?;
    let instance = (instance != 0).then_some(ResourceId(instance));
    let timeline = instance.and_then(|i| store.ui_layout_instance(i).ok()).map(|i| i.timeline).unwrap_or_default();
    let item = Item {
        unique_id: id,
        start: 0.0,
        end: f32::MAX,
        kind: ItemKind::Layout { layout: Some(rid), layer, texts: [0; 3], instance },
        depth: 0,
        enabled: String::new(),
        clock: 0,
    };
    let clocks = vec![mw_assets::sequence::ClockNode::root(Vec::new(), f32::MAX)];
    let seq = Sequence { id: rid, unique_id: id, items: vec![item], controllers: Vec::new(), length: f32::MAX, clocks };
    Some(LayoutPlay { seq: Arc::new(seq), fade_in: fade_in(&timeline) })
}

/// `FADE(delay, duration, IN, …)` of a timeline, in seconds ((0, 0) without one).
fn fade_in(timeline: &str) -> (f32, f32) {
    let Some(start) = timeline.find("FADE") else { return (0.0, 0.0) };
    let Some(args) = timeline[start + 4..].trim_start().strip_prefix('(').and_then(|r| r.split(')').next()) else {
        return (0.0, 0.0);
    };
    let parts: Vec<&str> = args.split(',').map(str::trim).collect();
    if parts.get(2) != Some(&"IN") {
        return (0.0, 0.0);
    }
    let num = |i: usize| parts.get(i).and_then(|p| p.parse::<f32>().ok()).unwrap_or(0.0) / 1000.0;
    (num(0), num(1))
}

#[cfg(test)]
mod tests {
    #[test]
    fn fade_in() {
        assert_eq!(super::fade_in("FADE(0, 400, IN, EASEINOUT)"), (0.0, 0.4));
        assert_eq!(super::fade_in("APPROACH (0, 300, 0, -100, EASIN), FADE(100, 200, IN, EASEINOUT)"), (0.1, 0.2));
        assert_eq!(super::fade_in("FADE(0,300,OUT)"), (0.0, 0.0));
    }
}
