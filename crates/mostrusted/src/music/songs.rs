//! The song manager (`UISongManager`, per-frame `FUN_007a4010`; docs/decomp.md "Music").

use crate::platform::PollLock;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mw_assets::music::{Song, SongList};

use super::{Decoded, MusicClip, MusicEnabled, Voice, VoiceKey, log_on, spawn_voice};

/// The free-drive game mode (`GameLogicConfig` `1b3c1ddc`); its `MusicPlaylist` is what "Freedrive Update" plays.
const FREEDRIVE_MODE: u32 = 390_145;
/// The new-game intro event's mode ("INTRO"): "Is Intro Race" plays list 2026345 instead (our reading of the
/// HSM function `IsIntroRace`: not traced).
const INTRO_MODE: u32 = 1_560_350;
const INTRO_SONG_LIST: u32 = 2_026_345;
/// Songs remembered against repeats (`FUN_00798a90`: a ring of 4 at +0xc8).
const HISTORY: usize = 4;
/// MUSIC_ADVANCE: a press released within this is a tap (`FUN_007a15c0`).
const TAP: f32 = 1.0;
/// "Mute World And Music" (sequence 1870238, the in-game menu): the music falls 1 → 0 over the first 0.056 of
/// its 5 s item (`MixerChannelSequenceItem` "Music UI", 3 keys). Coming back uses the same ramp (our choice: the
/// release was not traced).
const MENU_FADE: f32 = 0.28;

type Job<T> = Arc<Mutex<Option<T>>>;

/// The song manager.
///
/// - **What plays**: the game logic hands it a `SongList` (HSM `PLAYSONGLIST`): free roam's "Freedrive Update"
///   the free-drive mode's `MusicPlaylist` (1622455, the 42 licensed songs; `ALLOWPLAYOUT`: a song of another
///   list plays out first), events' "Play Default Game Mode Music" their mode's (1622455 for every offline mode,
///   so the same list keeps playing), the intro race 2026345. Nothing plays before the game: the title and the
///   boot transition have their own sequence waves.
/// - **First song** (state 7): the session's remembered position, else for a licensed list (`3fdcf508` and a
///   `FeedbackMessage`) the profile's `SongIndex`, else random; the list then plays from the next index.
/// - **Next song** (`FUN_0079d1e0`, `FUN_00798a90`): the next in order; a song played in the last 4 is passed
///   over, and the song's `bc7081c6` (f) biases by situation: in free drive (GMM+0x2e90) a song with f > 0 is
///   passed over when r ≤ f, in events a song with f < 0 when r ≤ −f (r uniform; the f > 0 songs are the rock
///   ones, f < 0 the electronic ones); a passed-over song swaps with a random later one, the last candidate is
///   taken.
/// - No crossfade: a song that ends starts the next the next frame; MUSIC_ADVANCE (M) skips on licensed lists
///   (`341d0a95`).
/// - The in-game menu (pause menu, map) fades the music out ("Mute World And Music") and back.
///
/// Model decisions: the play order is not shuffled (the default of profile option 5, shuffle, was not found),
/// the event's own list (event +0x68, read before the mode's) is not read, the RNG is seeded from the clock
/// like the game's (`FUN_00415fc0`, a timer).
#[derive(Resource)]
pub struct SongManager {
    lists: HashMap<u32, Arc<SongList>>,
    loading: Option<(u32, Job<Option<SongList>>)>,
    modes: Option<HashMap<u32, u32>>,
    modes_job: Option<Job<HashMap<u32, u32>>>,
    current: Option<Arc<SongList>>,
    order: Vec<usize>,
    index: usize,
    /// Per-session positions (`FUN_00792900`: list → next index + 1).
    session: HashMap<u32, usize>,
    history: VecDeque<u32>,
    rng: u64,
    /// The 8-deep delay line of uniform draws (+0x380).
    delay: [f32; 8],
    delay_at: usize,
    /// The song playing (list index) and its voice.
    song: Option<(usize, Entity)>,
    /// Picked, waiting to decode.
    pending: Option<usize>,
    advance_down: Option<f32>,
    menu_gain: f32,
    logged_menu_gain: f32,
}

impl Default for SongManager {
    fn default() -> Self {
        Self::with_seed(crate::platform::time_seed())
    }
}

impl SongManager {
    fn with_seed(seed: u64) -> Self {
        let mut m = Self {
            lists: HashMap::new(),
            loading: None,
            modes: None,
            modes_job: None,
            current: None,
            order: Vec::new(),
            index: 0,
            session: HashMap::new(),
            history: VecDeque::new(),
            rng: seed,
            delay: [0.0; 8],
            delay_at: 0,
            song: None,
            pending: None,
            advance_down: None,
            menu_gain: 1.0,
            logged_menu_gain: 1.0,
        };
        for _ in 0..8 {
            m.uniform();
        }
        m
    }

    /// The LCG step (`s = s × 0x5851f42d4c957f2d + 1`); the output is the high word before the step.
    fn next_u32(&mut self) -> u32 {
        let out = (self.rng >> 32) as u32;
        self.rng = self.rng.wrapping_mul(0x5851_f42d_4c95_7f2d).wrapping_add(1);
        out
    }

    /// A uniform [0, 1) from the manager's delay line (+0x380): each value was drawn 8 draws earlier, built from
    /// the generator's word as `bits >> 9 | 0x3f800000` − 1.
    fn uniform(&mut self) -> f32 {
        let out = self.delay[self.delay_at];
        let bits = self.next_u32() >> 9 | 0x3f80_0000;
        self.delay[self.delay_at] = f32::from_bits(bits) - 1.0;
        self.delay_at = (self.delay_at + 1) % 8;
        out
    }

    /// `FUN_00798a90`: may this song play now?
    fn acceptable(&mut self, song: &Song, free_drive: bool) -> bool {
        if self.history.contains(&song.id) {
            return false;
        }
        let f = song.bc7081c6;
        if f.abs() <= 1.19e-7 {
            return true;
        }
        let r = self.uniform();
        match (free_drive, f > 0.0) {
            (true, true) => r > f,
            (false, false) => r > -f,
            _ => true,
        }
    }

    /// `FUN_0079d1e0`: the next song's list index.
    fn next_song(&mut self, free_drive: bool) -> Option<usize> {
        let list = self.current.clone()?;
        let n = list.songs.len();
        if n == 0 {
            return None;
        }
        self.index += 1;
        if self.index >= n {
            self.order = (0..n).collect();
            self.index = 0;
        }
        let mut tries = n - self.index;
        while tries > 1 && !self.acceptable(&list.songs[self.order[self.index]], free_drive) {
            let j = self.index + (self.next_u32() as usize) % (n - self.index);
            self.order.swap(self.index, j);
            tries -= 1;
        }
        let pick = self.order[self.index];
        self.history.push_back(list.songs[pick].id);
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }
        Some(pick)
    }

    /// State 7: a newly started list's position; the next song is the one after it.
    fn start_list(&mut self, list: Arc<SongList>, profile: Option<&crate::frontend::Collected>) {
        let n = list.songs.len().max(1);
        let index = if let Some(&i) = self.session.get(&list.id) {
            i.saturating_sub(1)
        } else if list.flag_23 && list.feedback_message != 0 {
            profile.and_then(|p| p.song_lists.get(&list.id)).map_or(0, |&i| i as usize)
        } else {
            self.next_u32() as usize % n
        };
        self.index = if index >= n { 0 } else { index };
        self.order = (0..list.songs.len()).collect();
        self.current = Some(list);
    }

    /// `FUN_0079d0d0`: leaving a list remembers where it was (session table; the profile for licensed lists).
    fn leave_list(&mut self, profile: Option<&mut crate::frontend::Collected>) {
        let Some(list) = self.current.take() else { return };
        let n = list.songs.len().max(1);
        let next = (self.index + 1) % n;
        self.session.insert(list.id, next + 1);
        if list.flag_23
            && list.feedback_message != 0
            && let Some(p) = profile
        {
            p.song_lists.insert(list.id, next as u32);
            p.dirty = true;
        }
    }
}

/// `--hsm`: the song list the original's logic asked for (logic.rs), used instead of [`wanted_list`]'s hand-written
/// choice. `PLAYSONGLIST` enter (`FUN_0079f7b0`) sets `list` and `allow_playout` (`ALLOWPLAYOUT`); its exit with
/// `STOPONLEAVE` (`FUN_007908e0`) and the lists 0 ("Pause Music", "Clear Credits Songlist") set 0, which holds the
/// song playing and starts no new one, as list 0 does here. `free_drive` is GMM+0x2e90 (`IsFreedriveEvent`).
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct SongListRequest {
    pub list: u32,
    pub allow_playout: bool,
    pub free_drive: bool,
}

/// The list the game logic asks for now (0 = none), whether a song of another list may play out first, and the
/// free-drive flag (GMM+0x2e90).
fn wanted_list(
    front: Option<&crate::frontend::FrontEnd>,
    race: Option<&crate::race::RaceState>,
    modes: &HashMap<u32, u32>,
) -> (u32, bool, bool) {
    if front.is_some_and(|f| !f.playing()) {
        return (0, false, true);
    }
    match race.and_then(|r| r.game_mode.as_ref()) {
        Some(m) if m.id == INTRO_MODE => (INTRO_SONG_LIST, false, false),
        // "Play Default Game Mode Music": USEDEFAULT, no playout.
        Some(m) => (m.music_playlist, false, false),
        // "Freedrive Update" → "Play Freedrive Song List": USEDEFAULT, ALLOWPLAYOUT.
        None => (modes.get(&FREEDRIVE_MODE).copied().unwrap_or(0), true, true),
    }
}

/// Start `job` on a worker unless running; take its result when done.
fn poll<T: Send + 'static>(job: &mut Option<Job<T>>, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let slot = job.get_or_insert_with(|| {
        let slot = Arc::new(Mutex::new(None));
        let out = slot.clone();
        crate::platform::thread::spawn(move || *out.lock().unwrap() = Some(f()));
        slot
    });
    let done = slot.poll_lock().and_then(|mut s| s.take());
    if done.is_some() {
        *job = None;
    }
    done
}

#[allow(clippy::too_many_arguments)]
pub(super) fn song_manager(
    mut commands: Commands,
    enabled: Res<MusicEnabled>,
    mut m: ResMut<SongManager>,
    game: Res<crate::Game>,
    mut decoded: ResMut<Decoded>,
    mut clips: ResMut<Assets<MusicClip>>,
    time: Res<Time<Real>>,
    virtual_time: Res<Time<Virtual>>,
    (front, race): (Option<Res<crate::frontend::FrontEnd>>, Option<Res<crate::race::RaceState>>),
    pause: Option<Res<crate::pause::PauseMenu>>,
    map: Option<Res<crate::map_screen::MapScreen>>,
    mut collected: Option<ResMut<crate::frontend::Collected>>,
    request: Option<Res<SongListRequest>>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut voices: Query<&mut Voice>,
) {
    if !enabled.0 {
        return;
    }
    if m.modes.is_none() {
        let store = game.store.clone();
        let mut job = m.modes_job.take();
        let done = poll(&mut job, move || {
            store.events().map(|e| e.game_modes.iter().map(|g| (g.id, g.music_playlist)).collect()).unwrap_or_default()
        });
        m.modes_job = job;
        match done {
            Some(modes) => m.modes = Some(modes),
            None => return,
        }
    }
    let (want, playout, free_drive) = match request.as_deref() {
        Some(r) => (r.list, r.allow_playout, r.free_drive),
        None => wanted_list(front.as_deref(), race.as_deref(), m.modes.as_ref().unwrap()),
    };
    // The in-game menu's mixer sequence.
    let menu = pause.is_some_and(|p| p.open) || map.is_some_and(|s| s.open);
    let dt = time.delta_secs().min(0.1);
    m.menu_gain = (m.menu_gain + if menu { -dt } else { dt } / MENU_FADE).clamp(0.0, 1.0);
    let menu_gain = m.menu_gain;
    if log_on() && (menu_gain == 0.0 || menu_gain == 1.0) && m.logged_menu_gain != menu_gain {
        info!("music: in-game menu level {menu_gain}");
    }
    m.logged_menu_gain = menu_gain;
    if let Some((_, e)) = m.song
        && let Ok(mut v) = voices.get_mut(e)
    {
        v.duck = menu_gain;
    }
    // A requested list loads on a worker.
    if want != 0 && !m.lists.contains_key(&want) {
        if m.loading.as_ref().is_none_or(|(id, _)| *id != want) {
            m.loading = None;
        }
        let store = game.store.clone();
        let mut job = m.loading.take().map(|(_, j)| j);
        let done =
            poll(&mut job, move || store.song_list(want).inspect_err(|e| warn!("music: song list {want}: {e}")).ok());
        m.loading = job.map(|j| (want, j));
        if let Some(Some(list)) = done {
            m.lists.insert(want, Arc::new(list));
        }
    }
    if m.song.is_some_and(|(_, e)| voices.get(e).is_err()) {
        m.song = None;
    }
    // MUSIC_ADVANCE: a tap.
    let pad_list: Vec<&Gamepad> = pads.iter().collect();
    let dev = crate::input::Devices { keys: &keys, pads: &pad_list };
    let now = time.elapsed_secs();
    let mut skip = false;
    if dev.held(crate::input::defaults::MUSIC_ADVANCE) {
        if m.advance_down.is_none() {
            m.advance_down = Some(now);
        }
    } else if let Some(t) = m.advance_down.take() {
        skip = now - t < TAP && !menu && m.current.as_ref().is_some_and(|l| l.flag_22);
    }
    // A different list: switch now, or once the song has played out (ALLOWPLAYOUT).
    let current_id = m.current.as_ref().map(|l| l.id);
    if want != 0
        && current_id != Some(want)
        && (m.song.is_none() || !playout)
        && let Some(list) = m.lists.get(&want).cloned()
    {
        if let Some((_, e)) = m.song.take() {
            commands.entity(e).despawn();
        }
        m.pending = None;
        m.leave_list(collected.as_deref_mut());
        if log_on() {
            info!("music: song list {} ({} songs)", list.id, list.songs.len());
        }
        m.start_list(list, collected.as_deref());
    }
    if want == 0 || m.current.as_ref().map(|l| l.id) != Some(want) {
        // Paused (list 0) or waiting for the new list: the song playing is held, no new one starts.
        return;
    }
    if skip && let Some((_, e)) = m.song.take() {
        if log_on() {
            info!("music: MUSIC_ADVANCE skips the song");
        }
        commands.entity(e).despawn();
    }
    // While the game clock is paused (the pause menu) no new song starts (`FUN_007a1750`: game clock + 0x1c; the
    // `SetSongListSequenceItem` flag that allows it is not used here).
    if m.song.is_some() || virtual_time.is_paused() {
        return;
    }
    // The next song: picked once, started when decoded.
    let pick = match m.pending {
        Some(p) => p,
        None => {
            let Some(p) = m.next_song(free_drive) else { return };
            m.pending = Some(p);
            p
        }
    };
    let Some(list) = m.current.clone() else { return };
    let song = &list.songs[pick];
    let Some((pcm, gain, _)) = decoded.get(&game, song.wave) else { return };
    m.pending = None;
    if log_on() {
        info!(
            "music: song {} (list {} index {pick}, f {:.3}, free drive {free_drive}): {:.0} s",
            song.id,
            list.id,
            song.bc7081c6,
            pcm.duration_secs()
        );
    }
    let e = spawn_voice(
        &mut commands,
        &mut clips,
        MusicClip { pcm, start: 0, looping: false },
        gain,
        (0.0, 0.0),
        VoiceKey::Song(song.wave),
    );
    m.song = Some((pick, e));
    // Only the playing song stays decoded (songs are 40–60 MB of PCM).
    decoded.retain(&[song.wave]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(fs: &[f32]) -> Arc<SongList> {
        Arc::new(SongList {
            id: 1,
            feedback_message: 0,
            songs: fs
                .iter()
                .enumerate()
                .map(|(i, &f)| Song {
                    id: i as u32 + 100,
                    wave: mw_bundle::ResourceId(i as u64),
                    artist: 0,
                    title: 0,
                    bc7081c6: f,
                })
                .collect(),
            flag_22: true,
            flag_23: false,
            url: String::new(),
        })
    }

    #[test]
    fn every_song_once_per_pass_biased_order() {
        // Half rock (f = 0.95), half electronic (f = -0.95). A pass plays every song once (a passed-over song
        // only moves later); the bias decides the order: free drive puts the electronic songs first, events
        // the rock ones.
        let fs: Vec<f32> = (0..20).map(|i| if i % 2 == 0 { 0.95 } else { -0.95 }).collect();
        for free_drive in [true, false] {
            let mut m = SongManager::with_seed(12345);
            m.start_list(list(&fs), None);
            let mut last: VecDeque<usize> = VecDeque::new();
            let (mut rock_early, mut early) = (0, 0);
            let mut seen = std::collections::HashSet::new();
            for pass in 0..20 {
                for _ in 0..20 {
                    let p = m.next_song(free_drive).unwrap();
                    // `index` is the position in the pass (0 = a new pass began with this song).
                    let k = m.index;
                    if k == 0 {
                        seen.clear();
                    }
                    assert!(!last.contains(&p), "repeat within 4");
                    last.push_back(p);
                    if last.len() > 4 {
                        last.pop_front();
                    }
                    if pass > 0 {
                        assert!(seen.insert(p), "a song twice in one pass");
                    }
                    if pass > 0 && k < 8 {
                        early += 1;
                        rock_early += usize::from(fs[p] > 0.0);
                    }
                }
            }
            let share = rock_early as f32 / early as f32;
            assert_eq!(share < 0.3, free_drive, "free drive {free_drive}: rock share {share} of the first 8");
            assert_eq!(share > 0.7, !free_drive, "free drive {free_drive}: rock share {share} of the first 8");
        }
    }
}
