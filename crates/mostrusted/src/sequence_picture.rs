//! The picture and environment items of the playing sequences (`mw_assets::sequence_picture`): the event intros
//! and outros (`cutscene.rs`) and the UI sequences (`ui_sequence.rs`) hand their clocks to [`SequencePicture`]
//! every frame ([`SequencePicture::play`]); this module starts, updates and ends their items and runs the PostFX
//! states they request (docs/decomp.md "Sequence PostFX states").
//!
//! **PostFX states.** A `PostFXSequenceItem` requests its state on the game logic's PostFX block (+0x2ef70) at its
//! start (`FUN_00863810` → `FUN_0083d850`: up to 3 requests, reference-counted; a full list drops its newest),
//! feeds it every frame (`FUN_00863960`: its `Intensity` curve as the amount when `090eb59d`, `82d1338f` as the
//! transition when `354b8d7c`, with mode 1 the vector (`1a62fa2c`, `1a62fa2d`, `1a62fa2c`, `1a62fa2c`)) and
//! releases it at its end (`FUN_00863de0` → `FUN_0083d9e0`). The PostFX manager (`FUN_008ecf80`) turns each
//! request into a slot after its base states (`FUN_008e88d0`; a new slot starts at the state's `Value`,
//! `FUN_008e48b0`) and blends every slot in order (`FUN_008eccb0`): the amount is the request's when set, else it
//! moves to the `ValueBinding` by `ChangeBehaviour` (`FUN_008e4970`); the colour cubes go to the two state slots of
//! the TintBlend (`FUN_008e4a90`), the keyframe's Bloom, Vignette, depth-of-field and General blocks are blended
//! towards by the amount (`FUN_008e9610`, `FUN_008e4b30`, `FUN_008e4f20`, `FUN_008e4d00`; the vector replaces the
//! vignette's centre). Carried out: the vignette and the bloom ([`SlotBlend`], folded over the base in
//! `tonemap.rs`) and the cubes ([`crate::tonemap::PostView::state_cube`], mixed in `post.wgsl` with the TintBlend's
//! weights). Logged only: the General block (the motion blur, not drawn) and the depth of field.
//!
//! **Other items.** `DisableWorldRenderingSequenceItem` clears the game logic's +0x2efb4 (`FUN_008619c0`) and sets
//! it back at the end when `c38f91c8` (`FUN_00863270`): kept in [`SequencePicture::world_rendering`], not drawn
//! (where the renderer reads it, frame state +0x6af7af from `FUN_0046f730`, was not followed).
//! `ImagePaletteSequenceItem` keeps a UI image palette requested (`FUN_00796160` / `FUN_007915b0`; our UI loads
//! its images by itself). `SetTimeOfDaySequenceItem` sets the hour to its `da6ac384` time (`FUN_00847870`: the
//! first two words of the environment manager's message 2) and puts back the time saved before it at its end
//! (`FUN_00850280`; the original saves it when the item is created, `FUN_00897a20`, ours at its start).
//! `EnvironmentTimelineSequenceItem` hands the environment its timeline (`FUN_00844060`, message 7) and takes it
//! back at its end (`0x0084fe90`, message 7 without one). Logged only: `SceneAnalysis`, `CameraShake`,
//! `SetVisionModeType`.
//!
//! MODEL: the bindings of the states (`582a5d55`, `b2bcd17a`, `ValueBinding`) and the mode-2 vector binding are not
//! evaluated (sequence states leave them empty, except `…7886`, `…f8127`, `…facb2`, `…18d981`: `Signals.GetValue[0]`);
//! with one the amount keeps its last value. The base states of `POSTFX.BNDL` other than the default are not run
//! (their bindings: cooldown, heat, nitrous, wreck, escape): the request slots blend over the default keyframe's
//! vignette and the environment's bloom, as the original's do in free roam. Items' `EnabledBinding`s are not
//! evaluated (as `cutscene.rs`). Message 7's flag and SetTimeOfDay's transition (`23198e19` > 0, the rate
//! `d6529487`) are not traced: the timeline and the hour change at once.
//!
//! `MW_SEQUENCE_PICTURE_LOG=1`: item starts / ends, requests and the blended blocks; `MW_NO_SEQUENCE_POSTFX=1`: the
//! states run but are not drawn (A/B).

use crate::platform::PollLock;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mw_assets::environment::{PostFxKeyframe, Vignette};
use mw_assets::sequence::{ItemKind, Sequence};
use mw_assets::sequence_picture::{PictureItem, PostFxItem, PostFxState};
use mw_bundle::ResourceId;

pub struct SequencePicturePlugin;

impl Plugin for SequencePicturePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SequencePicture>().add_systems(PostUpdate, (ui_sequences, run).chain());
    }
}

/// `MW_SEQUENCE_PICTURE_LOG=1`.
fn log_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MW_SEQUENCE_PICTURE_LOG").is_some())
}

/// The game logic's PostFX requests (+0x2ef70 + 0x50, 0x30 bytes each, count at +0xe0).
const MAX_REQUESTS: usize = 3;

#[derive(Debug, Clone, PartialEq)]
struct Request {
    state: u64,
    /// +0x18: when it was (re)requested (`FUN_0046ee00` + 8, a frame counter).
    stamp: u64,
    /// +0x20: references.
    refs: i32,
    /// +0x24 with flag +0x2c (`FUN_0083da30`): the amount.
    amount: Option<f32>,
    /// +0x28 with flag +0x2e (`FUN_0083daf0`): the transition.
    transition: Option<f32>,
    /// +0x0 with flag +0x2d (`FUN_0083da90`): the vector.
    vector: Option<[f32; 4]>,
}

/// The request list's operations, as the original keeps it.
#[derive(Debug, Default)]
struct Requests {
    list: Vec<Request>,
    frame: u64,
}

impl Requests {
    /// `FUN_0083d850`: a known state gains a reference and a new stamp; a new one is appended, after the newest
    /// entry is dropped (swapped with the last) when the list is full.
    fn request(&mut self, state: u64) {
        if let Some(r) = self.list.iter_mut().find(|r| r.state == state) {
            r.refs += 1;
            r.stamp = self.frame;
            return;
        }
        if self.list.len() == MAX_REQUESTS {
            let newest = (1..MAX_REQUESTS).fold(0, |k, i| if self.list[i].stamp > self.list[k].stamp { i } else { k });
            self.list.swap_remove(newest);
        }
        self.list.push(Request { state, stamp: self.frame, refs: 1, amount: None, transition: None, vector: None });
    }

    /// `FUN_0083d9e0` → `FUN_0084a7d0`: one reference less; at none the entry goes (the last moves into it).
    fn release(&mut self, state: u64) {
        let mut i = 0;
        while i < self.list.len() {
            if self.list[i].state == state {
                self.list[i].refs -= 1;
                if self.list[i].refs < 1 {
                    self.list.swap_remove(i);
                    continue;
                }
            }
            i += 1;
        }
    }

    fn get(&mut self, state: u64) -> Option<&mut Request> {
        self.list.iter_mut().find(|r| r.state == state)
    }
}

/// A state with what it blends, read on a worker thread.
struct LoadedState {
    state: PostFxState,
    keyframe: Option<PostFxKeyframe>,
    next: Option<PostFxKeyframe>,
    cubes: [Option<Arc<mw_formats::ColourCube>>; 2],
}

fn load_state(store: &mw_assets::AssetStore, id: u64) -> Option<LoadedState> {
    let state = PostFxState::load(store, ResourceId(id))
        .map_err(|e| warn!("sequence picture: PostFX state {id:016x}: {e}"))
        .ok()?;
    let keyframe = |k: Option<ResourceId>| k.and_then(|k| PostFxKeyframe::load(store, k).ok());
    let cube = |c: Option<ResourceId>| {
        c.and_then(|c| store.load(c).ok()).and_then(|r| mw_formats::ColourCube::parse(&r).ok()).map(Arc::new)
    };
    Some(LoadedState {
        keyframe: keyframe(state.keyframe),
        next: keyframe(state.next_keyframe),
        cubes: [cube(state.cubes[0].cube), cube(state.cubes[1].cube)],
        state,
    })
}

type Job<T> = Arc<Mutex<Option<T>>>;

/// One request slot of the PostFX manager (400 bytes from manager +0x10 + index × 400).
struct Slot {
    state: u64,
    /// +0x180: the amount (starts at the state's `Value`, `FUN_008e48b0`).
    amount: f32,
}

/// What a request slot blends over the base, in slot order (`tonemap.rs` folds them).
#[derive(Debug, Clone, Default)]
pub struct SlotBlend {
    /// `FUN_008e4b30`: the vignette blended towards with this weight.
    pub vignette: Option<(f32, Vignette)>,
    /// `FUN_008e9610`: the bloom words blended towards with this weight.
    pub bloom: Option<(f32, BloomWords)>,
}

/// The bloom block's words the post chain uses (`FUN_008e51b0` order): 0–2 `Colour`, 4 `ff564907` (B.w), 9–11
/// `c9f605e2`, `a3d336c1`, `cf7d9668` (the level weights).
#[derive(Debug, Clone, Copy, Default)]
pub struct BloomWords {
    pub colour: [f32; 3],
    pub intensity: f32,
    pub weights: [f32; 3],
}

impl BloomWords {
    fn of(k: &PostFxKeyframe) -> Self {
        let b = &k.bloom;
        Self {
            colour: [b.colour[0], b.colour[1], b.colour[2]],
            intensity: b.threshold,
            weights: [b.c9f605e2, b.a3d336c1, b.cf7d9668],
        }
    }

    pub fn lerp(&self, o: &Self, t: f32) -> Self {
        Self {
            colour: lerp3(self.colour, o.colour, t),
            intensity: lerp(self.intensity, o.intensity, t),
            weights: lerp3(self.weights, o.weights, t),
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    (b - a) * t + a
}

fn lerp2(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t)]
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)]
}

/// `FUN_008ee430` over the vignette block (`FUN_008e2660` order: `Sharpness`, inner, outer, centre, `Scale`, …).
pub fn lerp_vignette(a: &Vignette, b: &Vignette, t: f32) -> Vignette {
    Vignette {
        centre: lerp2(a.centre, b.centre, t),
        scale: lerp2(a.scale, b.scale, t),
        inner: lerp3(a.inner, b.inner, t),
        outer: lerp3(a.outer, b.outer, t),
        sharpness: lerp(a.sharpness, b.sharpness, t),
    }
}

/// One playing sequence's picture items.
struct Run {
    /// (item index, start, end, item).
    items: Vec<(usize, f32, f32, PictureItem)>,
    clock: f32,
    /// Items started and not ended yet.
    active: Vec<bool>,
    seen: bool,
}

#[derive(Resource)]
pub struct SequencePicture {
    runs: HashMap<String, Run>,
    requests: Requests,
    slots: Vec<Slot>,
    states: HashMap<u64, Option<Arc<LoadedState>>>,
    jobs: HashMap<u64, Job<Option<LoadedState>>>,
    cube_images: HashMap<u64, Handle<Image>>,
    /// The blended pair of state cubes (ids, weight ratio) and its image.
    blended_cube: Option<((u64, u64, u8), Handle<Image>)>,
    /// Game logic +0x2efb4 (`DisableWorldRenderingSequenceItem`).
    pub world_rendering: bool,
    /// UI image palettes requested (id, references; up to 4, `FUN_00796160`).
    palettes: Vec<(u32, i32)>,
    /// The hour / timeline in place before a SetTimeOfDay / EnvironmentTimeline item (by run and item).
    saved_hour: HashMap<(String, usize), f32>,
    saved_timeline: HashMap<(String, usize), (ResourceId, bool)>,
    logged: std::collections::HashSet<String>,
}

impl Default for SequencePicture {
    fn default() -> Self {
        Self {
            runs: HashMap::new(),
            requests: Requests::default(),
            slots: Vec::new(),
            states: HashMap::new(),
            jobs: HashMap::new(),
            cube_images: HashMap::new(),
            blended_cube: None,
            world_rendering: true,
            palettes: Vec::new(),
            saved_hour: HashMap::new(),
            saved_timeline: HashMap::new(),
            logged: Default::default(),
        }
    }
}

impl SequencePicture {
    /// A sequence plays at `clock` this frame (`key` stays the same while it plays; one not played in a frame
    /// ends its items).
    pub fn play(&mut self, key: &str, seq: &Sequence, clock: f32) {
        let run = self.runs.entry(key.to_string()).or_insert_with(|| {
            let items: Vec<_> = seq
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, it)| match &it.kind {
                    ItemKind::Picture(p) => Some((i, it.start, it.end, p.clone())),
                    _ => None,
                })
                .collect();
            Run { active: vec![false; items.len()], items, clock, seen: true }
        });
        run.clock = clock;
        run.seen = true;
    }
}

/// The UI sequences (`ui_sequence.rs`) and their conducted children.
fn ui_sequences(player: Option<Res<crate::ui_sequence::UiSequencePlayer>>, mut picture: ResMut<SequencePicture>) {
    let Some(player) = player else { return };
    for (key, seq, clock) in player.clocks() {
        picture.play(&key, seq, clock);
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    game: Option<Res<crate::Game>>,
    mut picture: ResMut<SequencePicture>,
    post: Option<ResMut<crate::tonemap::PostView>>,
    env: Option<ResMut<crate::weather::EnvironmentState>>,
    mut images: ResMut<Assets<Image>>,
) {
    let (Some(game), Some(mut post)) = (game, post) else { return };
    let mut env = env;
    let pic = &mut *picture;
    pic.requests.frame += 1;

    // Item starts, updates and ends (the sequence's item loop `FUN_00a42b90`: start slot 3, update slot 5, end
    // slot 9).
    let keys: Vec<String> = pic.runs.keys().cloned().collect();
    for key in keys {
        let Some(mut run) = pic.runs.remove(&key) else { continue };
        let gone = !run.seen;
        for k in 0..run.items.len() {
            let (index, start, end) = (run.items[k].0, run.items[k].1, run.items[k].2);
            let inside = !gone && run.clock >= start && run.clock < end;
            let item = run.items[k].3.clone();
            if inside && !run.active[k] {
                run.active[k] = true;
                start_item(pic, &key, index, &item, env.as_deref_mut());
            }
            if inside {
                let u = (run.clock - start) / (end - start).max(1e-6);
                update_item(pic, &item, u);
            } else if run.active[k] {
                run.active[k] = false;
                end_item(pic, &key, index, &item, env.as_deref_mut());
            }
        }
        if !gone {
            run.seen = false;
            pic.runs.insert(key, run);
        } else if log_on() {
            info!("sequence picture: {key} stops");
        }
    }

    // Load the requested states.
    for r in &pic.requests.list {
        if pic.states.contains_key(&r.state) || pic.jobs.contains_key(&r.state) {
            continue;
        }
        let job: Job<Option<LoadedState>> = Arc::new(Mutex::new(None));
        let out = job.clone();
        let (store, id) = (game.store.clone(), r.state);
        crate::platform::thread::spawn(move || *out.lock().unwrap() = Some(load_state(&store, id)));
        pic.jobs.insert(r.state, job);
    }
    let done: Vec<u64> =
        pic.jobs.iter().filter(|(_, j)| j.poll_lock().is_some_and(|s| s.is_some())).map(|(s, _)| *s).collect();
    for s in done {
        let loaded = pic.jobs.remove(&s).and_then(|j| j.poll_lock().and_then(|mut s| s.take())).flatten();
        pic.states.insert(s, loaded.map(Arc::new));
    }

    // The manager's request slots (`FUN_008e88d0`): one per request, in its order; a new one starts at the
    // state's `Value`.
    let mut slots = Vec::with_capacity(pic.requests.list.len());
    for r in &pic.requests.list {
        let amount = match pic.slots.iter().position(|s| s.state == r.state) {
            Some(i) => pic.slots.swap_remove(i).amount,
            None => match pic.states.get(&r.state) {
                Some(Some(l)) => l.state.value,
                _ => continue,
            },
        };
        slots.push(Slot { state: r.state, amount });
    }
    pic.slots = slots;

    // `FUN_008eccb0` per slot.
    let mut blends = Vec::new();
    // The TintBlend's two state slots (`FUN_008ecf80` +0x12e8…+0x12fc): (cube, weight).
    let mut cubes: [(Option<u64>, f32); 2] = [(None, 0.0); 2];
    for slot in &mut pic.slots {
        let Some(Some(l)) = pic.states.get(&slot.state).cloned() else { continue };
        let Some(r) = pic.requests.list.iter().find(|r| r.state == slot.state).cloned() else { continue };
        let s = &l.state;
        // The amount: the request's, else towards the (unevaluated) `ValueBinding` (`FUN_008e4970`).
        if let Some(a) = r.amount {
            slot.amount = a;
        }
        let amount = slot.amount;
        // The transition, only with a second keyframe.
        let transition = if s.next_keyframe.is_some() { r.transition.unwrap_or(0.0) } else { 0.0 };
        // Colour cubes (`FUN_008e4a90`), with the state enabled (its binding not evaluated: on).
        for k in 0..2 {
            if s.cubes[k].cube.is_some() {
                cubes[k] = (s.cubes[k].cube.map(|c| c.0), s.cube_modifier.apply(amount));
            } else if s.cubes[k].clear {
                cubes[k].1 = 0.0;
            }
        }
        let mut blend = SlotBlend::default();
        if s.bloom
            && let Some(a) = &l.keyframe
        {
            let w = s.bloom_modifier.apply(amount);
            if w > 0.0 {
                let mut words = BloomWords::of(a);
                if transition > 0.0
                    && let Some(b) = &l.next
                {
                    words = words.lerp(&BloomWords::of(b), transition);
                }
                blend.bloom = Some((w, words));
            }
        }
        if s.vignette
            && let Some(a) = &l.keyframe
        {
            let w = s.vignette_modifier.apply(amount);
            if w > 0.0 {
                let mut v = a.vignette;
                if transition > 0.0
                    && let Some(b) = &l.next
                {
                    v = lerp_vignette(&v, &b.vignette, transition);
                }
                if let Some(vec) = r.vector {
                    v.centre = [vec[0], vec[1]];
                }
                blend.vignette = Some((w, v));
            }
        }
        if log_on() && pic.requests.frame.is_multiple_of(30) {
            let general = (s.general, l.keyframe.map(|k| k.general));
            info!(
                "sequence picture: state {:x} amount {amount:.3} transition {transition:.3} vignette {:?} bloom {:?} \
                 cubes {cubes:?}; not drawn: general {general:?} ({}), depth of field {}",
                slot.state & 0xFF_FFFF,
                blend.vignette.map(|v| v.0),
                blend.bloom.map(|b| b.0),
                s.general_modifier.apply(amount),
                s.dof
            );
        }
        blends.push(blend);
    }
    // `MW_NO_SEQUENCE_POSTFX=1`: the states run but are not drawn (A/B).
    let off = std::env::var_os("MW_NO_SEQUENCE_POSTFX").is_some();
    post.post_fx_slots = if off { Vec::new() } else { blends };

    // The state cubes as one texture with the pair's summed weight (the TintBlend's weighted sum is linear in
    // the entries, so (Σ w C) / Σ w sampled once equals the slots sampled apart).
    let live: Vec<(u64, f32)> = cubes.iter().filter_map(|(c, w)| c.filter(|_| *w > 0.0).map(|c| (c, *w))).collect();
    let cube_of = |pic: &SequencePicture, id: u64| {
        pic.states.values().flatten().find_map(|l| {
            l.state
                .cubes
                .iter()
                .zip(&l.cubes)
                .find(|(s, _)| s.cube.map(|c| c.0) == Some(id))
                .and_then(|(_, c)| c.clone())
        })
    };
    let (handle, weight) = match live.as_slice() {
        [] => (None, 0.0),
        [(id, w)] => {
            let h = match pic.cube_images.get(id) {
                Some(h) => Some(h.clone()),
                None => cube_of(pic, *id).map(|c| {
                    let h = images.add(crate::tonemap::colour_cube_image(&c));
                    pic.cube_images.insert(*id, h.clone());
                    h
                }),
            };
            (h, *w)
        }
        [(a, wa), (b, wb), ..] => {
            let sum = wa + wb;
            let ratio = (wa / sum * 255.0).round() as u8;
            let key = (*a, *b, ratio);
            let h = match &pic.blended_cube {
                Some((k, h)) if *k == key => Some(h.clone()),
                _ => {
                    let image = crate::tonemap::blend_colour_cubes(&[
                        (cube_of(pic, *a).map(|c| (*c).clone()), wa / sum),
                        (cube_of(pic, *b).map(|c| (*c).clone()), wb / sum),
                    ]);
                    image.map(|i| {
                        let h = match &pic.blended_cube {
                            Some((_, h)) => {
                                let _ = images.insert(h, i);
                                h.clone()
                            }
                            None => images.add(i),
                        };
                        pic.blended_cube = Some((key, h.clone()));
                        h
                    })
                }
            };
            (h, sum)
        }
    };
    let handle = handle.filter(|_| !off);
    if post.state_cube != handle {
        post.state_cube = handle;
    }
    post.state_cube_weight = if post.state_cube.is_some() { weight } else { 0.0 };
}

fn start_item(
    pic: &mut SequencePicture,
    key: &str,
    index: usize,
    item: &PictureItem,
    env: Option<&mut crate::weather::EnvironmentState>,
) {
    if log_on() {
        info!("sequence picture: {key} item {index} {} starts", item.name());
    }
    match item {
        // `FUN_00863810`: request the state.
        PictureItem::PostFx(f) => {
            if let Some(s) = f.state {
                pic.requests.request(s.0);
            }
            if f.mode == 2 && pic.logged.insert(format!("binding {}", f.vector_binding)) {
                info!("sequence picture: PostFX vector binding {:?} not evaluated", f.vector_binding);
            }
        }
        // `FUN_008619c0`.
        PictureItem::DisableWorldRendering { .. } => pic.world_rendering = false,
        // `0x00827df0` → `FUN_00796160`.
        PictureItem::ImagePalette { palette } => {
            if let Some(p) = pic.palettes.iter_mut().find(|p| p.0 == *palette) {
                p.1 += 1;
            } else if pic.palettes.len() < 4 {
                pic.palettes.push((*palette, 1));
            }
        }
        PictureItem::SetTimeOfDay(t) => {
            if let Some(env) = env {
                pic.saved_hour.insert((key.to_string(), index), env.want.hour);
                env.want.hour = t.time1.seconds() / 3600.0;
                if log_on() {
                    info!("sequence picture: time of day {:.3} h (flags {:#x})", env.want.hour, t.flags);
                }
            }
        }
        PictureItem::EnvironmentTimeline { timeline: Some(tl) } => {
            if let Some(env) = env {
                pic.saved_timeline.insert((key.to_string(), index), (env.want.timeline, env.follow));
                env.want.timeline = *tl;
                env.follow = false;
            }
        }
        PictureItem::EnvironmentTimeline { timeline: None } => {}
        other => {
            if pic.logged.insert(other.name().to_string()) {
                info!("sequence picture: {} items are not ported", other.name());
            }
        }
    }
}

/// `FUN_00863960` (the other items' updates do nothing we carry out).
fn update_item(pic: &mut SequencePicture, item: &PictureItem, u: f32) {
    let PictureItem::PostFx(f) = item else { return };
    let Some(state) = f.state else { return };
    let PostFxItem { intensity, transition, vector, mode, set_intensity, set_transition, .. } = &**f;
    let Some(r) = pic.requests.get(state.0) else { return };
    if *set_intensity {
        r.amount = Some(intensity.at(u));
    }
    if *set_transition {
        r.transition = Some(transition.at(u));
    }
    if *mode == 1 {
        let (x, y) = (vector[0].at(u), vector[1].at(u));
        r.vector = Some([x, y, x, x]);
    }
}

fn end_item(
    pic: &mut SequencePicture,
    key: &str,
    index: usize,
    item: &PictureItem,
    env: Option<&mut crate::weather::EnvironmentState>,
) {
    if log_on() {
        info!("sequence picture: {key} item {index} {} ends", item.name());
    }
    match item {
        // `FUN_00863de0`.
        PictureItem::PostFx(f) => {
            if let Some(s) = f.state {
                pic.requests.release(s.0);
            }
        }
        // `FUN_00863270`.
        PictureItem::DisableWorldRendering { restore } => {
            if *restore {
                pic.world_rendering = true;
            }
        }
        // `FUN_00824bc0` → `FUN_007915b0`.
        PictureItem::ImagePalette { palette } => {
            if let Some(i) = pic.palettes.iter().position(|p| p.0 == *palette) {
                pic.palettes[i].1 -= 1;
                if pic.palettes[i].1 < 1 {
                    pic.palettes.remove(i);
                }
            }
        }
        // `FUN_00850280`.
        PictureItem::SetTimeOfDay(_) => {
            if let (Some(env), Some(h)) = (env, pic.saved_hour.remove(&(key.to_string(), index))) {
                env.want.hour = h;
            }
        }
        // `0x0084fe90`.
        PictureItem::EnvironmentTimeline { .. } => {
            if let (Some(env), Some((tl, follow))) = (env, pic.saved_timeline.remove(&(key.to_string(), index))) {
                env.want.timeline = tl;
                env.follow = follow;
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_count_references_and_drop_the_newest_when_full() {
        let mut r = Requests::default();
        for s in [1, 2, 3] {
            r.frame += 1;
            r.request(s);
        }
        r.request(2);
        assert_eq!(r.get(2).map(|x| x.refs), Some(2));
        r.frame += 1;
        r.request(4);
        // 2 was re-stamped last: the newest goes, the last entry moves into its place.
        assert_eq!(r.list.iter().map(|x| x.state).collect::<Vec<_>>(), vec![1, 3, 4]);
        r.release(4);
        assert_eq!(r.list.iter().map(|x| x.state).collect::<Vec<_>>(), vec![1, 3]);
    }
}
