//! The game's speech: police radio chatter, race and pursuit speech (docs/decomp.md "Radio"; data
//! `mw_assets::police_speech`, the manager `mw_audio::speech`).
//!
//! - **The manager** (`mw_audio::speech::SpeechManager`, G+0x2c3c0) is shared ([`SpeechLink`]) between this module
//!   and the logic (`logic.rs`): the HSM's "Try to play speech" (`TRYTOPLAYSPE`) asks it through
//!   `Query::SpeechLoadState` / `TryPlaySpeech` / `SpeechPlaying`, the Lua `PlaySpeech` and the hook / native
//!   `StopSpeech` reach it as requests, and the HSM "Speech Manager" 983342 (chase / cooldown / race chatter every
//!   5 s) runs as data in the logic. While the logic runs it also steps the manager on the game clock and
//!   evaluates the events' conditions through its own bindings (`Mode.*`, `Signals.GetRandomNumber[n]`, ...) after
//!   the runtime's facts ([`SpeechFacts`]); with `--no-hsm` this module does both, on its facts alone.
//! - **Triggers** (the scoring actions' `GameplayTrigger`s, `FUN_006f9670` → `FUN_006e0a90`: the first event of
//!   the list whose conditions hold is requested; research log 2026-10-16): chase started, resumed, heat level
//!   up 1–6, cooldown started, escaped, a cop taken down, and (inferred) busted. They are C++ in the original, so
//!   they are raised here from the pursuit's state and queued for the manager.
//! - **Voices**: each line the manager says is decoded on a thread and played at its `Character.AudioSettings`
//!   mixer channel's gain (0.562 for the radio voices; the radio sound is in the recordings); a stopped event's
//!   voices end.
//!
//! MODEL: the facts' getters are the runtime's pursuit state (`IsHiddenFromCops` = the cops do not see the player,
//! `Health` 1, `DistanceToClosestChasingCop` from the cop cars in chase mode); bindings neither the facts nor the
//! logic resolve read false; without the logic, `GetRandomNumber[n]` draws from a local LCG and the mode time is
//! the time since start. Not ported: the radio clicks and static bed of `AudioSettings`, the loudhailer's 3D
//! placement, the speech volume setting (lines play on the effects volume). `MW_SPEECH_LOG=1` logs requests and
//! lines.

use crate::platform::PollLock;
use std::sync::{Arc, Mutex};

use avian3d::prelude::LinearVelocity;
use bevy::audio::Volume;
use bevy::prelude::*;
use mw_assets::police_speech::SpeechData;
use mw_audio::speech::{SpeechManager, SpeechOut, SpeechWorld};

use crate::audio::PcmAudio;
use crate::car_impacts::AiCrash;
use crate::drive::Player;
use crate::police::{Cop, CopMode, Police, PursuitHud};
use crate::pursuit::PursuitState;

pub struct PoliceSpeechPlugin;

impl Plugin for PoliceSpeechPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpeechLink>()
            .init_resource::<Speech>()
            .add_systems(Update, (load, facts, triggers, step, play).chain());
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

/// The game state the speech conditions and the speech HSM read (written each frame by [`facts`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct SpeechFacts {
    pub in_chase: bool,
    pub in_cooldown: bool,
    pub heat_level: u8,
    pub heat_progress: f32,
    pub cops_chasing: u32,
    pub speed_mph: f32,
    pub max_level: u8,
    pub time_til_evaded: f32,
    pub hidden: bool,
    /// Metres to the nearest cop in chase mode (`f32::MAX` without one).
    pub closest_chasing_cop: f32,
    pub has_car: bool,
    /// Set by the logic: a cutscene (NIS) plays; GMM+0x2d28 in minutes (else the time since start).
    pub in_cutscene: bool,
    pub mode_minutes: f32,
    /// Seconds since start (the mode clock's stand-in without the logic).
    pub now: f32,
}

impl SpeechFacts {
    /// A getter by its binding's last name.
    fn value(&self, name: &str) -> Option<f32> {
        let b = |x: bool| f32::from(u8::from(x));
        Some(match name {
            "IsInChase" => b(self.in_chase),
            "IsInCooldown" => b(self.in_cooldown),
            "IsInChaseOrCooldown" => b(self.in_chase || self.in_cooldown),
            "HeatLevelInt" => self.heat_level as f32,
            "HeatProgress" => self.heat_progress,
            "CopsInChase" => self.cops_chasing as f32,
            "GetPlayerSpeed" => self.speed_mph,
            "MaxHeatLevelReachedInChase" => self.max_level as f32,
            "TimeTilEvaded" => self.time_til_evaded,
            "IsHiddenFromCops" => b(self.hidden),
            "DistanceToClosestChasingCop" => self.closest_chasing_cop,
            "Health" => 1.0,
            _ => return None,
        })
    }

    /// A `Players.LocalPlayer.*` / `LocalPlayer.*` / `DrivingFacts.*` binding with its test, when it is one of ours.
    pub fn binding(&self, path: &str) -> Option<bool> {
        let e = mw_audio::sfx_player::Expr::parse(path)?;
        let (root, name) = e.base.rsplit_once('.')?;
        if !matches!(root, "Players.LocalPlayer" | "LocalPlayer" | "DrivingFacts") {
            return None;
        }
        Some(e.apply(self.value(name)?) > 0.5)
    }
}

/// The manager and what it shares with the logic.
#[derive(Default)]
pub struct SpeechCore {
    /// `None` while the events load.
    pub manager: Option<SpeechManager>,
    /// The bundle did not load (`Query::SpeechLoadState` answers 1).
    pub failed: bool,
    pub facts: SpeechFacts,
    /// Gameplay triggers waiting for the manager: the event list and why.
    triggers: Vec<(&'static [u32], &'static str)>,
    /// What the manager said, for [`play`].
    out: Vec<SpeechOut>,
    /// The logic stepped the manager this frame.
    pub logic_frame: bool,
    rng: u64,
}

/// The conditions: the facts, then the logic's bindings.
struct World<'a, 'l> {
    facts: &'a SpeechFacts,
    logic: Option<&'a mut (dyn FnMut(&str) -> Option<bool> + 'l)>,
    rng: &'a mut u64,
}

/// The logic's bindings (`LogicHost::binding_bool`), when the logic runs.
pub type LogicBindings<'a, 'l> = Option<&'a mut (dyn FnMut(&str) -> Option<bool> + 'l)>;

impl SpeechWorld for World<'_, '_> {
    fn condition(&mut self, binding: &str) -> bool {
        if let Some(b) = self.facts.binding(binding) {
            return b;
        }
        if let Some(l) = self.logic.as_mut() {
            return l(binding).unwrap_or(false);
        }
        // Without the logic: `Signals.GetRandomNumber[n]` on a local LCG (`FUN_007a7240`'s form).
        let Some(e) = mw_audio::sfx_player::Expr::parse(binding) else { return false };
        let Some(n) = e.base.strip_prefix("Signals.GetRandomNumber[").and_then(|s| s.strip_suffix(']')) else {
            return false;
        };
        let s = *self.rng;
        *self.rng = s.wrapping_mul(0x5851_f42d_4c95_7f2d).wrapping_add(1);
        e.apply(((s >> 32) as u32 % n.parse::<u32>().unwrap_or(1).max(1)) as f32) > 0.5
    }
    fn mode_minutes(&self) -> f32 {
        if self.logic.is_some() { self.facts.mode_minutes } else { self.facts.now / 60.0 }
    }
    fn has_car(&self) -> bool {
        self.facts.has_car
    }
    fn in_cutscene(&self) -> bool {
        self.facts.in_cutscene
    }
}

pub(crate) fn log() -> bool {
    static LOG: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("MW_SPEECH_LOG").is_some());
    *LOG
}

impl SpeechCore {
    fn world<'a, 'l>(facts: &'a SpeechFacts, rng: &'a mut u64, logic: LogicBindings<'a, 'l>) -> World<'a, 'l> {
        World { facts, logic, rng }
    }

    /// `FUN_008408a0`(id, args): the manager's answer ([`mw_audio::speech::NOT_LOADED`] while loading).
    pub fn request(&mut self, id: u32, args: [i32; 3], logic: LogicBindings<'_, '_>) -> u32 {
        let Self { manager, facts, rng, .. } = self;
        let Some(m) = manager.as_mut() else { return mw_audio::speech::NOT_LOADED };
        let r = m.request(id, false, args, &mut Self::world(facts, rng, logic));
        if log() {
            info!("speech: request {id}: {r:#x}");
        }
        r
    }

    /// `StopSpeech`.
    pub fn stop(&mut self) {
        if let Some(m) = self.manager.as_mut() {
            m.stop();
        }
    }

    /// The queued triggers (each: the first event whose conditions hold, `FUN_006e0a90`), then one manager frame.
    pub fn step(&mut self, dt: f32, mut logic: LogicBindings<'_, '_>) {
        let Self { manager, facts, rng, triggers, out, .. } = self;
        let Some(m) = manager.as_mut() else { return };
        for (list, why) in std::mem::take(triggers) {
            let mut w = Self::world(facts, rng, logic.as_mut().map(|l| &mut **l as _));
            let pick = list.iter().copied().find(|&id| m.conditions_hold(id, &mut w));
            let r = pick.map(|id| m.request(id, false, [-1, -1, 0], &mut w));
            if log() {
                info!("speech: {why}: {pick:?} of {list:?} → {r:?}");
            }
        }
        out.extend(m.update(dt, &mut Self::world(facts, rng, logic)));
    }
}

/// The manager shared with the logic.
#[derive(Resource, Clone, Default)]
pub struct SpeechLink(pub Arc<Mutex<SpeechCore>>);

type Job = Arc<Mutex<Option<anyhow::Result<SpeechData>>>>;
type LineJob = Arc<Mutex<Option<Option<mw_audio::Pcm>>>>;

#[derive(Resource, Default)]
struct Speech {
    data: Option<Arc<SpeechData>>,
    job: Option<Job>,
    /// Lines decoding and voices playing, by event.
    jobs: Vec<(u32, LineJob)>,
    voices: Vec<(u32, Entity)>,
    previous: Option<(PursuitState, u8)>,
}

fn load(game: Option<Res<crate::Game>>, mut speech: ResMut<Speech>, link: Res<SpeechLink>) {
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
        }
        Some(job) => {
            let Some(done) = job.poll_lock().and_then(|mut s| s.take()) else { return };
            let mut core = link.0.lock().unwrap();
            match done {
                Ok(d) => {
                    info!("speech: {} events, {} waves", d.events.len(), d.waves.len());
                    core.manager = Some(SpeechManager::new(d.events.values().cloned()));
                    speech.data = Some(Arc::new(d));
                }
                Err(e) => {
                    warn!("speech: no speech events ({e})");
                    core.failed = true;
                    speech.data = Some(Arc::new(SpeechData::default()));
                }
            }
            core.rng = 0x853c_49e6_748f_ea9b;
        }
    }
}

/// The facts the conditions read.
#[allow(clippy::type_complexity)]
fn facts(
    time: Res<Time>,
    police: Option<Res<Police>>,
    hud: Option<Res<PursuitHud>>,
    link: Res<SpeechLink>,
    player: Query<(&Transform, &LinearVelocity), With<Player>>,
    cops: Query<(&Transform, &Cop), Without<Player>>,
) {
    let mut core = link.0.lock().unwrap();
    let f = &mut core.facts;
    f.now = time.elapsed_secs();
    let car = player.single().ok();
    f.has_car = car.is_some();
    f.speed_mph = car.map_or(0.0, |(_, v)| v.0.length() / 0.447_04);
    let (Some(police), Some(hud)) = (police, hud) else {
        f.in_chase = false;
        f.in_cooldown = false;
        f.closest_chasing_cop = f32::MAX;
        return;
    };
    let p = &police.pursuit;
    f.in_chase = p.state == PursuitState::Chase;
    f.in_cooldown = p.state == PursuitState::Cooldown;
    f.heat_level = hud.heat_level;
    f.heat_progress = hud.heat_progress;
    f.cops_chasing = p.cops_chasing;
    f.max_level = p.max_level;
    f.time_til_evaded = p.time_til_evaded;
    f.hidden = !p.sighted;
    f.closest_chasing_cop = car.map_or(f32::MAX, |(pt, _)| {
        cops.iter()
            .filter(|(_, c)| c.mode() == CopMode::Chase)
            .map(|(t, _)| t.translation.distance(pt.translation))
            .fold(f32::MAX, f32::min)
    });
}

/// The pursuit's gameplay triggers → queued speech requests.
fn triggers(
    police: Option<Res<Police>>,
    hud: Option<Res<PursuitHud>>,
    mut speech: ResMut<Speech>,
    link: Res<SpeechLink>,
    taken_down: Query<(), (Added<AiCrash>, With<Cop>)>,
) {
    let (Some(police), Some(hud)) = (police, hud) else { return };
    let now = (police.pursuit.state, hud.heat_level);
    let Some((was, level)) = speech.previous.replace(now) else { return };
    let mut core = link.0.lock().unwrap();
    if core.manager.is_none() {
        return;
    }
    let t = &mut core.triggers;
    if was != now.0 {
        match now.0 {
            PursuitState::Chase if was == PursuitState::Cooldown => t.push((CHASE_RESUMED, "chase resumed")),
            PursuitState::Chase => t.push((CHASE_STARTED, "chase started")),
            PursuitState::Cooldown => t.push((COOLDOWN_STARTED, "cooldown")),
            PursuitState::Escaped => t.push((ESCAPED, "escaped")),
            PursuitState::Busted => t.push((BUSTED, "busted")),
            PursuitState::None => {}
        }
    } else if now.1 > level && (1..=6).contains(&now.1) {
        t.push((HEAT_UP[now.1 as usize - 1], "heat level up"));
    }
    if now.0 == PursuitState::Chase && !taken_down.is_empty() {
        let list = if police.pursuit.chase_time <= 2.0 { TAKEDOWN_EARLY } else { TAKEDOWN };
        t.push((list, "cop taken down"));
    }
}

/// Without the logic, the manager's frame here (the logic steps it otherwise, `logic.rs`).
fn step(time: Res<Time>, link: Res<SpeechLink>) {
    let mut core = link.0.lock().unwrap();
    if std::mem::take(&mut core.logic_frame) {
        return;
    }
    core.step(time.delta_secs(), None);
}

/// The manager's lines → voices; a stopped event's voices end.
fn play(
    mut commands: Commands,
    mut speech: ResMut<Speech>,
    link: Res<SpeechLink>,
    mut assets: ResMut<Assets<PcmAudio>>,
    settings: Option<Res<crate::settings::AudioSettings>>,
) {
    let out = std::mem::take(&mut link.0.lock().unwrap().out);
    let speech = &mut *speech;
    let Some(data) = speech.data.clone() else { return };
    for o in out {
        match o {
            SpeechOut::Say { event, line } => {
                let job: LineJob = Arc::new(Mutex::new(None));
                let done = job.clone();
                let d = data.clone();
                crate::platform::thread::spawn(move || {
                    let pcm = d.line_pcm(line.wave).map(|mut pcm| {
                        for s in &mut pcm.samples {
                            *s = (*s as f32 * line.gain) as i16;
                        }
                        pcm
                    });
                    *done.lock().unwrap() = Some(pcm);
                });
                if log() {
                    info!("speech: event {event} line {} ({:.2} s)", line.wave, line.duration);
                }
                speech.jobs.push((event, job));
            }
            SpeechOut::Stop { event } => {
                speech.jobs.retain(|(e, _)| *e != event);
                speech.voices.retain(|&(e, v)| {
                    if e == event {
                        commands.entity(v).try_despawn();
                    }
                    e != event
                });
            }
        }
    }
    let vol = settings.as_ref().map_or(0.6, |s| s.gain(crate::settings::Channel::Sfx));
    let mut started = Vec::new();
    speech.jobs.retain(|(event, job)| {
        let Some(done) = job.poll_lock().and_then(|mut s| s.take()) else { return true };
        if let Some(pcm) = done {
            if log() {
                // The voice's level as it is handed to the mixer (after the channel gain), before the volume.
                let n = pcm.samples.len().max(1) as f32;
                let rms = (pcm.samples.iter().map(|&x| (x as f32 / 32768.0).powi(2)).sum::<f32>() / n).sqrt();
                let peak = pcm.samples.iter().map(|&x| (x as f32 / 32768.0).abs()).fold(0.0, f32::max);
                let secs = n / pcm.sample_rate.max(1) as f32 / pcm.channels.max(1) as f32;
                info!("speech: voice for event {event}: {secs:.2} s, rms {rms:.3}, peak {peak:.3}, volume {vol:.2}");
            }
            let h = assets.add(PcmAudio { pcm: Arc::new(pcm) });
            let e = commands
                .spawn((
                    AudioPlayer::<PcmAudio>(h),
                    PlaybackSettings::DESPAWN.with_volume(Volume::Linear(vol)),
                    crate::settings::Channel::Sfx,
                ))
                .id();
            started.push((*event, e));
        }
        false
    });
    speech.voices.extend(started);
    // Voices despawn themselves when they end.
    speech.voices.retain(|&(_, v)| commands.get_entity(v).is_ok());
}
