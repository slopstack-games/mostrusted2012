//! Police radio chatter in pursuits (docs/decomp.md "Police sirens, light bars and radio"; data
//! `mw_assets::police_speech`): the game's speech manager for the pursuit events.
//!
//! - **One speech at a time, no queue** (`FUN_008408a0` → `FUN_008406a0`): a request names events in order; the
//!   first whose conditions hold (all, or any by its flag) and that is eligible (fewer plays than its maximum,
//!   `Period` minutes since it last ended, not a one-off heard before; `FUN_0083d270`) starts if nothing is
//!   playing, or interrupts what plays when its `Priority` number is lower (0 highest); otherwise the request
//!   is dropped.
//! - **An event** waits its `Delay`, then plays its items at their `StartTime`: a line item says one line picked
//!   uniformly over all its lines and containers (`FUN_0083c5a0`), with its probability; an event item plays one
//!   of its sub-events from there. The voice's level is its `Character.AudioSettings` mixer channel's gain
//!   (0.562 for the radio voices); the radio sound is in the recordings.
//! - **Triggers** (the scoring actions' `GameplayTrigger`s, `FUN_006f9670`; research log 2026-10-16): chase
//!   started, resumed, heat level up 1–6, cooldown started, escaped, a cop taken down, and (inferred) busted.
//! - **Chatter** (HSM 983342 "Speech Manager"): in a chase, every 5 s one of commentary 1186587 / backup 1378016
//!   / request update 1871411 / player direction 1378788 by `GetRandomNumber[4]` (within 25 m of a chasing cop:
//!   commentary, direction, or the in-car loudhailer 1186559 twice as often); in the cooldown every 5 s the search
//!   instructions 1871531.
//!
//! Conditions read our game state by the binding's last name (`IsInChase`, `HeatLevelInt`, `CopsInChase`,
//! `GetPlayerSpeed`, `GetRandomNumber[n]`, ...). MODEL: bindings we do not have read 0 (`Mode.IsRaceEvent`,
//! `GameWorld.Compass`, `WasInChaseWithThisCar`, ...). Not ported: the narrator's one-off lines, the radio clicks
//! and static bed of `AudioSettings`, vocabulary slots (car model / colour lines; an event made only of them is
//! passed over), the loudhailer's 3D placement,
//! cutscene behaviour, the speech volume setting (lines play on the effects volume). `MW_SPEECH_LOG=1` logs
//! requests and lines.

use crate::platform::PollLock;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use avian3d::prelude::LinearVelocity;
use bevy::audio::Volume;
use bevy::prelude::*;
use mw_assets::police_speech::{SpeechData, SpeechEvent, SpeechItem};

use crate::audio::PcmAudio;
use crate::car_impacts::AiCrash;
use crate::drive::Player;
use crate::police::{Cop, CopMode, Police, PursuitHud};
use crate::pursuit::PursuitState;

pub struct PoliceSpeechPlugin;

impl Plugin for PoliceSpeechPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Speech>().add_systems(Update, (load, triggers, chatter, play).chain());
    }
}

/// Chase started (`ScoringConfig` +0x50's trigger: the first eligible), then the +0x48 variant's.
const CHASE_STARTED: &[u32] = &[1871814, 1186394, 1186396, 1656883, 1186352, 1186379];
const CHASE_RESUMED: &[u32] = &[1186477];
const COOLDOWN_STARTED: &[u32] = &[1186436];
const ESCAPED: &[u32] = &[1186439];
/// Inferred: priority 0, play-once events no trigger was found for (research log 2026-10-16).
const BUSTED: &[u32] = &[1186445, 1186447];
/// Heat level up to 1..=6 (`ScoringConfig` +0xe8 / +0xec triggers).
const HEAT_UP: [&[u32]; 6] = [
    &[1186591],
    &[1186607, 1186591],
    &[1656990, 1186591],
    &[1186563, 1186593],
    &[1186570, 1186595],
    &[1186807, 1186595],
];
/// A cop taken down (the five TAKEDOWN actions' trigger 1378651): within 2 s of the chase's start, else later.
const TAKEDOWN_EARLY: &[u32] = &[1186381, 1186388, 1186386, 1186390];
const TAKEDOWN: &[u32] = &[1186421, 1186424, 1186787, 1186413, 1186416];
/// HSM 983342: the chase chatter by `GetRandomNumber[4]`, far and within 25 m of a chasing cop.
const CHATTER_FAR: [u32; 4] = [1186587, 1378016, 1871411, 1378788];
const CHATTER_CLOSE: [u32; 4] = [1186587, 1378788, 1186559, 1186559];
const CHATTER_CLOSE_RANGE: f32 = 25.0;
const COOLDOWN_CHATTER: u32 = 1871531;
/// HSM 983363 / 983365 "Wait For Time" (s).
const CHATTER_WAIT: f32 = 5.0;

type Job = Arc<Mutex<Option<anyhow::Result<SpeechData>>>>;
type LineJob = Arc<Mutex<Option<Option<mw_audio::Pcm>>>>;

struct Playing {
    event: u32,
    priority: u8,
    t: f32,
    /// Items still to start: (time on this event's clock, item).
    items: Vec<(f32, SpeechItem)>,
    /// Lines being decoded, voices playing, and when the last line ends.
    jobs: Vec<LineJob>,
    voices: Vec<Entity>,
    end: f32,
}

#[derive(Resource, Default)]
struct Speech {
    data: Option<Arc<SpeechData>>,
    job: Option<Job>,
    playing: Option<Playing>,
    plays: HashMap<u32, u32>,
    /// Game time each event last ended.
    ended: HashMap<u32, f32>,
    heard: HashSet<u32>,
    rng: u64,
    chatter: f32,
    previous: Option<(PursuitState, u8)>,
}

fn log() -> bool {
    static LOG: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("MW_SPEECH_LOG").is_some());
    *LOG
}

impl Speech {
    fn random(&mut self, n: u32) -> u32 {
        self.rng = self.rng.wrapping_mul(0x5851_f42d_4c95_7f2d).wrapping_add(1);
        ((self.rng >> 33) as u32) % n.max(1)
    }

    /// A condition string against the game state (`Expr` form: base plus an optional test).
    fn condition(&mut self, cond: &str, state: &State) -> bool {
        let Some(e) = mw_audio::sfx_player::Expr::parse(cond) else { return true };
        let name = e.base.rsplit('.').next().unwrap_or("");
        let v = if let Some(n) = name.strip_prefix("GetRandomNumber[").and_then(|s| s.strip_suffix(']')) {
            self.random(n.parse().unwrap_or(1)) as f32
        } else {
            state.value(name)
        };
        e.apply(v) > 0.5
    }

    fn eligible(&mut self, ev: &SpeechEvent, state: &State) -> bool {
        // MODEL: an event made only of vocabulary slots (car model / colour lines, not ported) says nothing here,
        // so the request goes on to the next event of its list.
        let says_something = ev.items.iter().any(|i| match i {
            SpeechItem::Lines { lines, .. } => !lines.is_empty(),
            SpeechItem::Events { .. } => true,
        });
        if !says_something {
            return false;
        }
        if ev.max_plays > 0 && self.plays.get(&ev.id).copied().unwrap_or(0) >= ev.max_plays as u32 {
            return false;
        }
        if ev.one_off && self.heard.contains(&ev.id) {
            return false;
        }
        if let Some(&t) = self.ended.get(&ev.id)
            && state.now - t < ev.period * 60.0
        {
            return false;
        }
        if ev.conditions.is_empty() {
            return true;
        }
        let results: Vec<bool> = ev.conditions.iter().map(|c| self.condition(c, state)).collect();
        if ev.all_conditions { results.iter().all(|&b| b) } else { results.iter().any(|&b| b) }
    }

    /// `FUN_008408a0`: the first eligible event of `list` starts, unless a more important one plays.
    fn request(&mut self, commands: &mut Commands, list: &[u32], state: &State, why: &str) -> bool {
        let Some(data) = self.data.clone() else { return false };
        for id in list {
            let Some(ev) = data.events.get(id) else { continue };
            if !self.eligible(ev, state) {
                continue;
            }
            if let Some(p) = &self.playing {
                if ev.priority >= p.priority {
                    if log() {
                        info!(
                            "speech: {why}: {id} dropped ({} plays, priority {} ≥ {})",
                            p.event, ev.priority, p.priority
                        );
                    }
                    return false;
                }
                self.stop(commands, state.now);
            }
            if log() {
                info!(
                    "speech: {why}: event {id} (priority {}, delay {} s, {} items)",
                    ev.priority,
                    ev.delay,
                    ev.items.len()
                );
            }
            self.playing = Some(Playing {
                event: *id,
                priority: ev.priority,
                t: 0.0,
                items: ev.items.iter().map(|i| (ev.delay + i.start(), i.clone())).collect(),
                jobs: Vec::new(),
                voices: Vec::new(),
                end: 0.0,
            });
            *self.plays.entry(*id).or_default() += 1;
            self.heard.insert(*id);
            return true;
        }
        if log() {
            info!("speech: {why}: none of {list:?} eligible");
        }
        false
    }

    fn stop(&mut self, commands: &mut Commands, now: f32) {
        if let Some(p) = self.playing.take() {
            for v in p.voices {
                commands.entity(v).try_despawn();
            }
            self.ended.insert(p.event, now);
        }
    }
}

/// What the conditions read.
struct State {
    now: f32,
    in_chase: bool,
    in_cooldown: bool,
    heat_level: u8,
    heat_progress: f32,
    cops_chasing: u32,
    speed_mph: f32,
    max_level: u8,
    time_til_evaded: f32,
}

impl State {
    fn value(&self, name: &str) -> f32 {
        let b = |x: bool| f32::from(u8::from(x));
        match name {
            "IsInChase" => b(self.in_chase),
            "IsInCooldown" => b(self.in_cooldown),
            "IsInChaseOrCooldown" => b(self.in_chase || self.in_cooldown),
            "HeatLevelInt" => self.heat_level as f32,
            "HeatProgress" => self.heat_progress,
            "CopsInChase" => self.cops_chasing as f32,
            "GetPlayerSpeed" => self.speed_mph,
            "MaxHeatLevelReachedInChase" => self.max_level as f32,
            "TimeTilEvaded" => self.time_til_evaded,
            "Health" => 1.0,
            // MODEL: everything else (race events, the compass, the car history, ...) reads 0.
            _ => 0.0,
        }
    }
}

fn load(game: Option<Res<crate::Game>>, mut speech: ResMut<Speech>) {
    if speech.data.is_some() {
        return;
    }
    let Some(game) = game else { return };
    match &speech.job {
        None => {
            let job: Job = Arc::new(Mutex::new(None));
            let out = job.clone();
            let store = game.store.clone();
            crate::platform::thread::spawn(move || {
                let d = store.speech_data().map_err(anyhow::Error::from);
                *out.lock().unwrap() = Some(d);
            });
            speech.job = Some(job);
            speech.rng = 0x853c_49e6_748f_ea9b;
        }
        Some(job) => {
            let Some(done) = job.poll_lock().and_then(|mut s| s.take()) else { return };
            match done {
                Ok(d) => {
                    info!("speech: {} events, {} waves", d.events.len(), d.waves.len());
                    speech.data = Some(Arc::new(d));
                }
                Err(e) => {
                    warn!("speech: no police radio ({e})");
                    speech.data = Some(Arc::new(SpeechData::default()));
                }
            }
        }
    }
}

fn state(time: &Time, police: &Police, hud: &PursuitHud, speed_mph: f32) -> State {
    let p = &police.pursuit;
    State {
        now: time.elapsed_secs(),
        in_chase: p.state == PursuitState::Chase,
        in_cooldown: p.state == PursuitState::Cooldown,
        heat_level: hud.heat_level,
        heat_progress: hud.heat_progress,
        cops_chasing: p.cops_chasing,
        speed_mph,
        max_level: p.max_level,
        time_til_evaded: p.time_til_evaded,
    }
}

/// The pursuit's events → speech requests.
#[allow(clippy::type_complexity)]
fn triggers(
    mut commands: Commands,
    time: Res<Time>,
    police: Option<Res<Police>>,
    hud: Option<Res<PursuitHud>>,
    mut speech: ResMut<Speech>,
    player: Query<&LinearVelocity, With<Player>>,
    taken_down: Query<(), (Added<AiCrash>, With<Cop>)>,
) {
    let (Some(police), Some(hud)) = (police, hud) else { return };
    if speech.data.is_none() {
        return;
    }
    let speed = player.single().map_or(0.0, |v| v.0.length() / 0.447_04);
    let s = state(&time, &police, &hud, speed);
    let now = (police.pursuit.state, hud.heat_level);
    let Some((was, level)) = speech.previous.replace(now) else { return };
    let started = |a: PursuitState, b: PursuitState| was == a && now.0 == b;
    if was != now.0 {
        match now.0 {
            PursuitState::Chase if was == PursuitState::Cooldown => {
                speech.request(&mut commands, CHASE_RESUMED, &s, "chase resumed");
            }
            PursuitState::Chase => {
                speech.request(&mut commands, CHASE_STARTED, &s, "chase started");
            }
            PursuitState::Cooldown => {
                speech.request(&mut commands, COOLDOWN_STARTED, &s, "cooldown");
            }
            PursuitState::Escaped => {
                speech.request(&mut commands, ESCAPED, &s, "escaped");
            }
            PursuitState::Busted => {
                speech.request(&mut commands, BUSTED, &s, "busted");
            }
            PursuitState::None => {}
        }
        speech.chatter = 0.0;
    } else if now.1 > level && (1..=6).contains(&now.1) && !started(PursuitState::None, PursuitState::Chase) {
        speech.request(&mut commands, HEAT_UP[now.1 as usize - 1], &s, "heat level up");
    }
    if s.in_chase && !taken_down.is_empty() {
        let list = if police.pursuit.chase_time <= 2.0 { TAKEDOWN_EARLY } else { TAKEDOWN };
        speech.request(&mut commands, list, &s, "cop taken down");
    }
}

/// HSM 983342: every 5 s in a chase or cooldown, one chatter event.
#[allow(clippy::type_complexity)]
fn chatter(
    mut commands: Commands,
    time: Res<Time>,
    police: Option<Res<Police>>,
    hud: Option<Res<PursuitHud>>,
    mut speech: ResMut<Speech>,
    player: Query<(&Transform, &LinearVelocity), With<Player>>,
    cops: Query<(&Transform, &Cop), Without<Player>>,
) {
    let (Some(police), Some(hud)) = (police, hud) else { return };
    if speech.data.is_none() || speech.playing.is_some() {
        return;
    }
    let Ok((pt, pv)) = player.single() else { return };
    let s = state(&time, &police, &hud, pv.0.length() / 0.447_04);
    if !s.in_chase && !s.in_cooldown {
        speech.chatter = 0.0;
        return;
    }
    speech.chatter += time.delta_secs();
    if speech.chatter < CHATTER_WAIT {
        return;
    }
    speech.chatter = 0.0;
    if s.in_cooldown {
        speech.request(&mut commands, &[COOLDOWN_CHATTER], &s, "cooldown chatter");
        return;
    }
    let close = cops
        .iter()
        .filter(|(_, c)| c.mode() == CopMode::Chase)
        .any(|(t, _)| t.translation.distance(pt.translation) <= CHATTER_CLOSE_RANGE);
    let pick = speech.random(4) as usize;
    let id = if close { CHATTER_CLOSE[pick] } else { CHATTER_FAR[pick] };
    speech.request(&mut commands, &[id], &s, "chase chatter");
}

/// The playing event's clock: start its lines, end it when they are done.
fn play(
    mut commands: Commands,
    time: Res<Time>,
    mut speech: ResMut<Speech>,
    mut assets: ResMut<Assets<PcmAudio>>,
    settings: Option<Res<crate::settings::AudioSettings>>,
) {
    let speech = &mut *speech;
    let Some(data) = speech.data.clone() else { return };
    let now = time.elapsed_secs();
    let Some(mut p) = speech.playing.take() else { return };
    p.t += time.delta_secs();
    // Items whose time has come.
    let mut due = Vec::new();
    p.items.retain(|(t, it)| {
        if *t <= p.t {
            due.push((*t, it.clone()));
            false
        } else {
            true
        }
    });
    for (t, item) in due {
        match item {
            SpeechItem::Lines { probability, lines, .. } => {
                if lines.is_empty() || (speech.random(1000) as f32) / 1000.0 >= probability {
                    continue;
                }
                let line = lines[speech.random(lines.len() as u32) as usize];
                let job: LineJob = Arc::new(Mutex::new(None));
                let out = job.clone();
                let d = data.clone();
                crate::platform::thread::spawn(move || {
                    let pcm = d.line_pcm(line.wave).map(|mut pcm| {
                        for s in &mut pcm.samples {
                            *s = (*s as f32 * line.gain) as i16;
                        }
                        pcm
                    });
                    *out.lock().unwrap() = Some(pcm);
                });
                if log() {
                    info!("speech: event {} line {} at {t:.1} s", p.event, line.wave);
                }
                p.jobs.push(job);
                // Until the line has decoded, the event lasts at least a moment past its start.
                p.end = p.end.max(t + 0.5);
            }
            SpeechItem::Events { events, .. } => {
                if events.is_empty() {
                    continue;
                }
                let id = events[speech.random(events.len() as u32) as usize];
                if let Some(ev) = data.events.get(&id) {
                    for i in &ev.items {
                        p.items.push((p.t + ev.delay + i.start(), i.clone()));
                    }
                }
            }
        }
    }
    // Lines that have decoded start now.
    let vol = settings.as_ref().map_or(0.6, |s| s.gain(crate::settings::Channel::Sfx));
    p.jobs.retain(|job| {
        let Some(done) = job.poll_lock().and_then(|mut s| s.take()) else { return true };
        if let Some(pcm) = done {
            let secs = pcm.samples.len() as f32 / pcm.sample_rate.max(1) as f32 / pcm.channels.max(1) as f32;
            p.end = p.end.max(p.t + secs);
            let h = assets.add(PcmAudio { pcm: Arc::new(pcm) });
            let e = commands
                .spawn((
                    AudioPlayer::<PcmAudio>(h),
                    PlaybackSettings::DESPAWN.with_volume(Volume::Linear(vol)),
                    crate::settings::Channel::Sfx,
                ))
                .id();
            p.voices.push(e);
        }
        false
    });
    if p.items.is_empty() && p.jobs.is_empty() && p.t >= p.end {
        if log() {
            info!("speech: event {} ended after {:.1} s", p.event, p.t);
        }
        speech.ended.insert(p.event, now);
        return;
    }
    speech.playing = Some(p);
}
