//! The UI message manager's feedback queues (UIM+0x17e0; research log 2026-11-18 "the HUD feedback-message queue"):
//! the queues messages are posted to (`ShowFeedbackMessage`, `ShowForcedFeedBackMessage`, `PLAYFEEDBACK`, the song
//! popup, ...), and the per-frame update that plays the head message of each queue: its `LayoutStyle` layout on
//! the UI sequence player (a play of one layout with its visibility transition), its time, its exit.
//!
//! **The queues** (`FUN_007ba460` builds the list, `FUN_007b2a80` names each with "interruptible" = 1): Online (3
//! messages), Instructions, Hud (UIM+0x7230, the default), Camera (2), Autolog, SecondaryQueue, DamageQueue,
//! Notifications, Song (2), Transition (2), System (2), then eight "Message Sequence" queues (2); the others hold 7
//! (vtable 0xfbfc24 +0x14 = `0x6b2720`; 0xfbfc6c: 3; 0xfbfc48: 2). A queue is an ordered array: insert at a position
//! shifts up (`0x6b29f0`), remove shifts down (`0x6b2c10`). A name is found by exact compare in list order
//! (`FUN_007b2940`); an unknown name means the Hud queue (`FUN_007b6110`).
//!
//! **Posting** (`FUN_007b6280` through `FUN_007b8080`): the serial `DAT_0117d170` advances (0 after 0x7ffffffe); the
//! message is looked up by its UniqueId (+0x88) in the loaded feedback groups' `Messages` (`FUN_007985b0`: the 7 group
//! slots in order, state 3 only); not found, nothing is queued (the serial is still taken). The record
//! (`FUN_007b5e40`) keeps the definition, the group, the parts (the texts' `{name}` values), the serial (+0x330) and
//! the timer (+0x328) = `Time` (+0xc4) × 3e6 ticks (`.rdata` 0xf9f604), counted only when |Time| > 1.19e-7
//! (0xfe0fdc / 0xf91588; +0x3a4). `FUN_007b96b0` appends (position = count), `FUN_007b8100` puts it first.
//!
//! **The update** (`FUN_007bb160`, from the UI manager's update `FUN_007a4af0`): each queue in list order
//! (`FUN_007baec0`):
//! - the head's group no longer loaded (`FUN_00792590`): the queue is emptied (`FUN_007b2ba0`) and resumed;
//! - a suspended queue hides its sequence (`FUN_00791e60`(h, 0)) and does nothing else;
//! - an empty queue releases its sequence (`FUN_007b94c0`);
//! - else the head runs (`FUN_007bab20`): started once (`FUN_007b9ee0`: its sounds, movie, speech, camera, HUD mode
//!   and the `Transition_Message`, which empties the Transition queue and is appended to it); a timed head counts
//!   its timer down by the frame (a message that pauses with the game (+0xec) or follows a cutscene's clock (+0xf1
//!   with `CutsceneData.Cutscene`) only while the game runs, the latter by the game's step) and is done once the
//!   timer / 3 < 1 (+0x3a5). A done head is removed (`FUN_007b95c0`: its play released). Else its layout plays
//!   (`FUN_007bad00`): a `LayoutStyle` (+0x90, a `UILayout` id) other than the one playing starts (`FUN_007ba7d0`:
//!   with the `UILayoutInstanceParams` of +0x4c, `FUN_007a0df0`, else on its own, `FUN_00791840`; the draw layer
//!   from +0xd8: 0 → 11, 1 → 0, 2 → 19, 3 → 24), the play is shown every frame, paused while a pause-with-game
//!   message waits for the game, and once the play object is ready (`FUN_007b2d10`) its out length (+0x18) is the
//!   out time and, for `fffa9b53` (+0xe6), the timer grows by its in + out lengths (+0x14 / +0x18) and counts; when
//!   the timer falls below the out time the play goes out (`FUN_007b6240`: inactive (`FUN_00791d60`(h, 0)), state 3).
//! - Then, when a running queue's head is exclusive (+0xe8, the last such queue wins), every other interruptible
//!   queue is suspended (+0x25), once per change; with none, all resume.
//!
//! The runtime feeds the loaded groups' messages ([`FeedbackQueues::set_group`]) and, once a play's layout is
//! loaded, its in / out lengths ([`FeedbackQueues::sequences`]); it calls [`FeedbackQueues::update`] once per frame
//! and draws the queues' [`Play`]s with their head's texts.
//!
//! Model decisions: the message's name argument (hashed by `FUN_009ed180` into +0x3a0, the key of its sounds) is kept
//! as text; the timer counts the frame's real time (`FUN_0046ee00`'s clock) and the game's step for a cutscene
//! message; a movie (+0x94) counts as ready at once (`FUN_007900d0` not modelled: no movie plays); a full queue's
//! insert drops its last record (the original writes past the array); the group slots are the runtime's load order;
//! +0x334 (a record byte whose writer was not found) stays false; `FUN_007b95c0`'s exit sound plays only without a
//! sequence (its test of the play's time was not traced). Not modelled: the manager's pause counter
//! (`FUN_0046ee70`(3)), the display widgets the update steps after the queues (+0x10990 .. +0x12808: Gameplay, Hud,
//! HudSmall, NonInteractive, MostWantedList, Milestones, HudInstant, Nitrous, Commentary, Scoring), the HUD mode
//! write (+0x8c → UIM+0x31c08), the camera / mix / world-entity effects (`FUN_007b2c20`, `FUN_007b7e40`,
//! `FUN_007b9480`, `FUN_007b7f90`).

use std::collections::HashMap;

use super::state::Request;
use crate::binding::{Lookup, Segment, Value};

/// Ticks per second of the timers (`.rdata` 0xf9f604; 0xf9f608 = its inverse).
pub const TICKS_PER_SECOND: f64 = 3.0e6;
/// `DAT_0117d170` wraps to 0 here.
pub const SERIAL_WRAP: u32 = 0x7fff_ffff;
/// `FUN_007bb160`'s "no exclusive queue".
pub const NO_QUEUE: usize = 0x16;
/// The group slots `FUN_007985b0` scans.
pub const GROUP_SLOTS: usize = 7;
/// `Time` within ±this is untimed (0xfe0fdc / 0xf91588).
const TIME_EPSILON: f32 = 1.192_092_9e-7;

/// The queues `FUN_007ba460` registers, in list order, with their capacity.
pub const QUEUES: [(&str, usize); 19] = [
    ("Online", 3),
    ("Instructions", 7),
    ("Hud", 7),
    ("Camera", 2),
    ("Autolog", 7),
    ("SecondaryQueue", 7),
    ("DamageQueue", 7),
    ("Notifications", 7),
    ("Song", 2),
    ("Transition", 2),
    ("System", 2),
    ("Message Sequence", 2),
    ("Message Sequence", 2),
    ("Message Sequence", 2),
    ("Message Sequence", 2),
    ("Message Sequence", 2),
    ("Message Sequence", 2),
    ("Message Sequence", 2),
    ("Message Sequence", 2),
];
/// UIM+0x7230.
pub const HUD: usize = 2;
/// UIM+0xa6f0.
pub const SECONDARY: usize = 5;
/// UIM+0xf610 (the song popup's).
pub const SONG: usize = 8;
/// UIM+0x11190.
pub const TRANSITION: usize = 9;

/// A `FeedbackMessage` (GeneSys; offsets in its data) as far as the queues read it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MessageDef {
    /// +0x88 `655d3f5f`.
    pub id: u32,
    /// +0x90 `LayoutStyle`: the `UILayout` that shows it (`FUN_007ba7d0`).
    pub layout_style: u32,
    /// +0x4c `bca59513`: the `UILayoutInstanceParams` placing it (raw resource id; 0 = none).
    pub layout_instance: u64,
    /// The three texts (language ids): `Title` (+0xb0) → Text1, `0a899744` (+0xa8) → Text2, `0a899747` (+0xac) → Text3.
    pub texts: [u32; 3],
    /// +0xc4 `Time` (seconds; 0 = until removed or extended by the sequence).
    pub time: f32,
    /// +0xd8 `a0ba493a`: the draw layer choice.
    pub layer: u16,
    /// +0xe6 `fffa9b53`: the play's in + out lengths are added to the time.
    pub extend_by_sequence: bool,
    /// +0xe7 `13b2a760` (the queue's +0x7a; read by `FUN_007b8210`, not modelled).
    pub flag_e7: bool,
    /// +0xe8 `0cefee9e`: suspends the other queues while it is a running head.
    pub exclusive: bool,
    /// +0xec `2d000db9`: its timer and sequence pause while the game is paused.
    pub pause_with_game: bool,
    /// +0xf1 `f472d91c` with `CutsceneData.Cutscene` (+0x18) set: counts on the game clock.
    pub cutscene_clock: bool,
    /// +0xb8 `Transition_Message`.
    pub transition_message: u32,
    /// What the start / exit would play (`FUN_007b9ee0`, `FUN_007b6040`): not modelled, reported.
    pub effects: MessageEffects,
}

/// A message's sounds, movie, speech and camera (`FUN_007b9ee0`, `FUN_007b6040`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MessageEffects {
    /// +0xb4 `5fc73ed7` (sound kind 8 at the start).
    pub start_sound: u32,
    /// +0xbc `0b910820` (at the exit).
    pub exit_sound: u32,
    /// +0xa4 `StreamedSound` (kind 0xd).
    pub streamed_sound: u32,
    /// +0x94 `Movie`, +0x98 `Movie_Sound`.
    pub movie: u32,
    pub movie_sound: u32,
    /// +0x9c `Speech` (first entry; kind 100).
    pub speech: u32,
    /// +0x84 `CameraTake`.
    pub camera_take: u32,
    /// +0x8c `056664c3` (UIM+0x31c08 while it shows).
    pub hud_mode: u32,
}

/// A part of a message's texts: `{name}` in a text reads its value (`FUN_007c22d0`).
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub name: String,
    pub value: PartValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PartValue {
    /// `FUN_007c1bd0` / `FUN_007c1c00`.
    Number(f32),
    /// A gamer tag (`FUN_007c1aa0`: the player's name) or a string (`FUN_007c21b0`).
    Text(String),
    /// `FUN_007c2000`: a language id.
    StringId(i32),
}

/// A queued message (the 0x3b0-byte record `FUN_007b5e40` fills).
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub def: MessageDef,
    /// +0x320: the group it was found in.
    pub group: i64,
    /// The name argument (`FUN_009ed180` hashes it into +0x3a0).
    pub name: String,
    pub parts: Vec<Part>,
    /// +0x330.
    pub serial: u32,
    /// +0x328 (ticks).
    pub timer: i64,
    /// +0x3a4.
    pub timed: bool,
    /// +0x3a5.
    pub done: bool,
    /// +0x3a6.
    pub started: bool,
    /// +0x3a7: a `LayoutStyle` whose sequence is not ready yet.
    pub waiting_sequence: bool,
    /// +0x3a8: waits for its movie.
    pub waiting_movie: bool,
    /// +0x334.
    pub held: bool,
    /// Host-filled: the record's three text buffers (+0, +0x108, +0x210; `FUN_007c3b70`: the language string of
    /// `def.texts[i]` with the parts' `{name}`s filled), once `texts_ready`.
    pub texts: [String; 3],
    pub texts_ready: bool,
}

/// A queue's layout play (+0x28 handle, +0x2c state, +0x30 out time).
#[derive(Debug, Clone, PartialEq)]
pub struct Play {
    /// The layout (`LayoutStyle`).
    pub sequence: u32,
    /// Its `UILayoutInstanceParams` (raw resource id, 0 = none).
    pub instance: u64,
    /// The draw layer (11, 0, 19, 24).
    pub layer: u32,
    /// Distinct per start (the runtime restarts the sequence's clock on a new handle).
    pub handle: u32,
    /// `FUN_00791e60`: shown.
    pub visible: bool,
    /// `FUN_00791d60`: playing (false: paused, or gone out).
    pub active: bool,
    /// 1 playing, 2 paused, 3 out.
    pub state: u8,
    /// The out time in ticks (`None` = the sentinel i64::MIN: not yet read from the ready sequence).
    pub out: Option<i64>,
}

/// One message queue.
#[derive(Debug, Clone, PartialEq)]
pub struct Queue {
    /// +0x1c.
    pub name: &'static str,
    /// vtable +0x14.
    pub capacity: usize,
    /// +0x24.
    pub interruptible: bool,
    /// +0x25.
    pub suspended: bool,
    pub messages: Vec<Message>,
    pub play: Option<Play>,
    /// +0x7a.
    pub flag_7a: bool,
}

impl Queue {
    pub fn full(&self) -> bool {
        self.messages.len() == self.capacity
    }

    /// `FUN_007aeb10`: a message with `serial` is queued.
    pub fn queued(&self, serial: u32) -> bool {
        self.messages.iter().any(|m| m.serial == serial)
    }

    /// `FUN_007b3410` / `FUN_007b35c0` / `FUN_007b2ba0`: every message dropped (the play stays until the next
    /// update releases it).
    pub fn clear(&mut self) {
        self.messages.clear();
    }

    /// `FUN_007b3380`: the head dropped.
    pub fn drop_head(&mut self) {
        if !self.messages.is_empty() {
            self.messages.remove(0);
        }
    }
}

/// A loaded feedback group's messages, in its `Messages` order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadedGroup {
    pub id: i64,
    pub messages: Vec<MessageDef>,
}

/// A play's lengths once its layout is loaded (the play object's +0x14 / +0x18: in and out transitions, seconds).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SequenceLengths {
    pub in_len: f32,
    pub out_len: f32,
}

/// The manager (UIM+0x17e0).
#[derive(Debug, Clone, PartialEq)]
pub struct FeedbackQueues {
    pub queues: Vec<Queue>,
    /// `DAT_0117d170`.
    pub serial: u32,
    /// +0x13458.
    pub exclusive: usize,
    /// Host-fed: the loaded groups (the group table's slots in state 3).
    pub groups: Vec<LoadedGroup>,
    /// Host-fed: the layouts loaded for plays, by `LayoutStyle`.
    pub sequences: HashMap<u32, SequenceLengths>,
    next_handle: u32,
}

impl Default for FeedbackQueues {
    fn default() -> Self {
        let queues = QUEUES
            .iter()
            .map(|&(name, capacity)| Queue {
                name,
                capacity,
                interruptible: true,
                suspended: false,
                messages: Vec::new(),
                play: None,
                flag_7a: false,
            })
            .collect();
        Self { queues, serial: 0, exclusive: NO_QUEUE, groups: Vec::new(), sequences: HashMap::new(), next_handle: 0 }
    }
}

fn ticks(seconds: f32) -> i64 {
    // `FUN_00eda646` (ftol): truncation.
    (seconds as f64 * TICKS_PER_SECOND) as i64
}

impl FeedbackQueues {
    /// `FUN_007b2940`: the first queue named `name`.
    pub fn find_queue(&self, name: &str) -> Option<usize> {
        self.queues.iter().position(|q| q.name == name)
    }

    /// `FUN_007b6110`: the named queue, else the Hud queue.
    pub fn queue_or_hud(&self, name: &str) -> usize {
        self.find_queue(name).unwrap_or(HUD)
    }

    /// A group finished loading (`Some`, its messages) or was released (`None`).
    pub fn set_group(&mut self, id: i64, messages: Option<Vec<MessageDef>>) {
        self.groups.retain(|g| g.id != id);
        if let Some(messages) = messages
            && self.groups.len() < GROUP_SLOTS
        {
            self.groups.push(LoadedGroup { id, messages });
        }
    }

    /// `FUN_00792590`.
    pub fn group_loaded(&self, id: i64) -> bool {
        self.groups.iter().any(|g| g.id == id)
    }

    /// `FUN_007985b0`: the message `id` and its group.
    pub fn find(&self, id: u32) -> Option<(&MessageDef, i64)> {
        self.groups.iter().find_map(|g| g.messages.iter().find(|m| m.id == id).map(|m| (m, g.id)))
    }

    /// `FUN_007b6280` on `queue`: takes the next serial and queues message `id` at `pos` (`None` = appended,
    /// `FUN_007b96b0`; `Some(0)` = first, `FUN_007b8100`) when a loaded group has it. Returns the serial.
    pub fn post(&mut self, queue: usize, pos: Option<usize>, id: u32, name: &str, parts: Vec<Part>) -> u32 {
        self.serial = self.serial.wrapping_add(1);
        if self.serial >= SERIAL_WRAP {
            self.serial = 0;
        }
        let serial = self.serial;
        let Some((def, group)) = self.find(id).map(|(d, g)| (d.clone(), g)) else { return serial };
        let timed = def.time > TIME_EPSILON || def.time < -TIME_EPSILON;
        let message = Message {
            timer: ticks(def.time),
            waiting_sequence: def.layout_style != 0,
            def,
            group,
            name: name.to_string(),
            parts,
            serial,
            timed,
            done: false,
            started: false,
            waiting_movie: false,
            held: false,
            texts: Default::default(),
            texts_ready: false,
        };
        let Some(q) = self.queues.get_mut(queue) else { return serial };
        let at = pos.unwrap_or(q.messages.len()).min(q.messages.len());
        q.messages.insert(at, message);
        q.messages.truncate(q.capacity);
        serial
    }

    /// `FUN_007bb160`: one frame of every queue. `dt` is the frame's real time, `game_dt` the game's step (None
    /// while the game is paused).
    pub fn update(&mut self, dt: f32, game_dt: Option<f32>, requests: &mut Vec<Request>) {
        let mut exclusive = NO_QUEUE;
        for i in 0..self.queues.len() {
            self.update_queue(i, dt, game_dt, requests);
            let q = &self.queues[i];
            if !q.suspended && q.messages.first().is_some_and(|m| m.def.exclusive) {
                exclusive = i;
            }
        }
        if exclusive != NO_QUEUE {
            if self.exclusive != exclusive {
                for (j, q) in self.queues.iter_mut().enumerate() {
                    if j != exclusive && q.interruptible {
                        q.suspended = true;
                    }
                }
                self.exclusive = exclusive;
            }
        } else if self.exclusive != NO_QUEUE {
            for q in &mut self.queues {
                q.suspended = false;
            }
            self.exclusive = NO_QUEUE;
        }
    }

    /// `FUN_007baec0`.
    fn update_queue(&mut self, i: usize, dt: f32, game_dt: Option<f32>, requests: &mut Vec<Request>) {
        if let Some(group) = self.queues[i].messages.first().map(|m| m.group)
            && !self.group_loaded(group)
        {
            let q = &mut self.queues[i];
            q.suspended = false;
            q.clear();
        }
        let q = &mut self.queues[i];
        if q.suspended {
            if let Some(p) = &mut q.play {
                p.visible = false;
            }
            return;
        }
        if q.messages.is_empty() {
            // `FUN_007b94c0`.
            q.play = None;
            return;
        }
        if !q.messages[0].started && !self.start(i, requests) {
            return;
        }
        let ready = |s: &HashMap<u32, SequenceLengths>, id| s.contains_key(&id);
        let sequences = &self.sequences;
        let q = &mut self.queues[i];
        let m = &mut q.messages[0];
        if m.waiting_sequence && ready(sequences, m.def.layout_style) {
            m.waiting_sequence = false;
        }
        // `FUN_007bab20`: the timer.
        if m.timed && !((m.def.pause_with_game || m.def.cutscene_clock) && game_dt.is_none()) {
            if m.waiting_movie {
                m.waiting_movie = false;
            }
            let step = if m.def.cutscene_clock { game_dt.unwrap_or(0.0) } else { dt };
            m.timer -= ticks(step);
            m.done = m.timer / 3 < 1;
        }
        q.flag_7a = m.def.flag_e7;
        if m.done {
            // `FUN_007b95c0`.
            if q.play.take().is_none() && m.def.effects.exit_sound != 0 {
                requests.push(Request::FeedbackMessageSound { queue: q.name.into(), sound: m.def.effects.exit_sound });
            }
            q.messages.remove(0);
            return;
        }
        self.play_sequence(i, game_dt.is_some());
    }

    /// `FUN_007b9ee0`: the head starts. False when the start emptied the queue (its transition message went to it).
    fn start(&mut self, i: usize, requests: &mut Vec<Request>) -> bool {
        let q = &mut self.queues[i];
        let m = &mut q.messages[0];
        m.started = true;
        if m.def.effects.movie != 0 {
            m.waiting_movie = true;
        }
        let e = &m.def.effects;
        if [e.start_sound, e.streamed_sound, e.movie, e.movie_sound, e.speech, e.camera_take, e.hud_mode]
            .iter()
            .any(|&v| v != 0)
        {
            requests.push(Request::FeedbackMessageEffects { queue: q.name.into(), id: m.def.id, effects: e.clone() });
        }
        let transition = m.def.transition_message;
        if transition != 0 {
            self.queues[TRANSITION].clear();
            self.post(TRANSITION, None, transition, "", Vec::new());
            if i == TRANSITION {
                return false;
            }
        }
        true
    }

    /// `FUN_007bad00`.
    fn play_sequence(&mut self, i: usize, game_running: bool) {
        let q = &mut self.queues[i];
        let m = &mut q.messages[0];
        let want = m.def.layout_style;
        if want != 0 && q.play.as_ref().map_or(0, |p| p.sequence) != want {
            // `FUN_007ba7d0`.
            self.next_handle = self.next_handle.wrapping_add(1);
            let layer = match m.def.layer {
                0 => 11,
                2 => 19,
                3 => 24,
                _ => 0,
            };
            q.play = Some(Play {
                sequence: want,
                instance: m.def.layout_instance,
                layer,
                handle: self.next_handle,
                visible: true,
                active: true,
                state: 1,
                out: None,
            });
        }
        let Some(p) = &mut q.play else { return };
        p.visible = true;
        if !m.held && (!m.def.pause_with_game || game_running) {
            if p.state == 2 {
                p.active = true;
                p.state = 1;
            }
        } else if p.state == 1 {
            p.active = false;
            p.state = 2;
        }
        match p.out {
            None => {
                // `FUN_007b2d10`.
                if let Some(l) = self.sequences.get(&p.sequence) {
                    p.out = Some(ticks(l.out_len));
                    if m.def.extend_by_sequence {
                        m.timer += ticks(l.in_len) + ticks(l.out_len);
                        m.timed = true;
                    }
                }
            }
            Some(out) => {
                let (t, o) = (m.timer as f32 / TICKS_PER_SECOND as f32, out as f32 / TICKS_PER_SECOND as f32);
                if t < o && t > 0.0 {
                    // `FUN_007b6240`.
                    p.active = false;
                    p.out = Some(0);
                    p.state = 3;
                }
            }
        }
    }

    /// Carries out the queue requests of the hooks and behaviours: `ClearHudMessageQueues` (`FUN_0067f8c0`:
    /// `FUN_007b2ba0` on the Hud and SecondaryQueue queues), `ClearAllFeedbackQueues` /
    /// `ClearAllNonBackgroundFeedbackQueues` (`FUN_007b3410` on the Hud queue; their sequence-manager half is not
    /// modelled), `PLAYFEEDBACK`'s exit (`FUN_007b3410` on its queue). False for any other request.
    pub fn apply(&mut self, r: &Request) -> bool {
        match r {
            Request::ClearHudMessageQueues => {
                self.queues[HUD].clear();
                self.queues[SECONDARY].clear();
            }
            Request::ClearFeedbackQueues { .. } => self.queues[HUD].clear(),
            Request::ClearFeedbackQueue { queue } => {
                if let Some(q) = self.find_queue(queue) {
                    self.queues[q].clear();
                }
            }
            _ => return false,
        }
        true
    }

    /// `PLAYFEEDBACK`'s post (`FUN_007b96b0`(queue, message, *0x10c5c34, 0, 0, 0x10c5c70)): appended to the named
    /// queue; the serial, the behaviour's handle. The name argument (the string at `*0x10c5c34`) is read as "".
    pub fn post_named(&mut self, queue: &str, message: i64) -> u64 {
        let q = self.queue_or_hud(queue);
        u64::from(self.post(q, None, message as u32, "", Vec::new()))
    }

    /// The queues' plays to show, with their head message (the texts' source), in list order.
    pub fn plays(&self) -> impl Iterator<Item = (&Queue, &Play, Option<&Message>)> {
        self.queues.iter().filter_map(|q| Some((q, q.play.as_ref().filter(|p| p.visible)?, q.messages.first())))
    }
}

/// `UIMessages` members naming a queue's current message, and the queue (model decision: the root's getters were
/// not found; the names the layouts bind are matched to the queues by meaning).
const UI_MESSAGES: [(&str, usize); 5] =
    [("HUD", HUD), ("Songs", SONG), ("System", 10), ("Autolog", 4), ("Notifications", 7)];

/// The `UIMessages` root's queue members: `UIMessages.<Queue>.CurrentMessage.Text` / `SubText1` / `SubText2`, the
/// head message's three texts (Text1..3 of its record; "" while the queue is empty). The song popup's scrolling
/// text binds `{UIMessages.Songs.CurrentMessage.SubText2}  {UIMessages.Songs.CurrentMessage.Text}` (artist, title).
/// Other members (`NISRunning`, `NitrousList`, `Colour`, `Status`) read null here.
pub fn lookup(s: &super::LogicState, members: &[Segment]) -> Lookup {
    let Some((queue, rest)) = members.split_first() else { return Lookup::Found(Value::Object(true)) };
    let Some(&(_, q)) = UI_MESSAGES.iter().find(|(n, _)| queue.hash == crate::binding::hash(n)) else {
        return Lookup::Null;
    };
    let Some((current, rest)) = rest.split_first() else { return Lookup::Found(Value::Object(true)) };
    if current.hash != crate::binding::hash("CurrentMessage") {
        return Lookup::Null;
    }
    let Some((field, _)) = rest.split_first() else { return Lookup::Found(Value::Object(true)) };
    let Some(i) = ["Text", "SubText1", "SubText2"].iter().position(|t| field.hash == crate::binding::hash(t)) else {
        return Lookup::Null;
    };
    let head = s.feedback_queues.queues.get(q).and_then(|q| q.messages.first());
    Lookup::Found(Value::Str(head.map(|m| m.texts[i].clone()).unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: u32, time: f32) -> MessageDef {
        MessageDef { id, layout_style: id + 1, time, ..Default::default() }
    }

    fn with_group(messages: Vec<MessageDef>) -> FeedbackQueues {
        let mut f = FeedbackQueues::default();
        f.set_group(7, Some(messages));
        f
    }

    #[test]
    fn post_and_order() {
        let mut f = with_group(vec![def(10, 2.0), def(20, 0.0)]);
        assert_eq!(f.post(HUD, None, 10, "a", vec![]), 1);
        assert_eq!(f.post(HUD, Some(0), 20, "b", vec![]), 2);
        // Not found: the serial is taken, nothing queued.
        assert_eq!(f.post(HUD, None, 99, "c", vec![]), 3);
        let q = &f.queues[HUD];
        assert_eq!(q.messages.iter().map(|m| m.def.id).collect::<Vec<_>>(), [20, 10]);
        assert!(q.messages[1].timed && !q.messages[0].timed);
        assert_eq!(q.messages[1].timer, 6_000_000);
        assert!(q.queued(1) && !q.queued(3));
        assert_eq!(f.queue_or_hud("Song"), SONG);
        assert_eq!(f.queue_or_hud("nope"), HUD);
        f.serial = SERIAL_WRAP - 1;
        assert_eq!(f.post(HUD, None, 10, "", vec![]), 0);
    }

    #[test]
    fn timed_head_plays_and_leaves() {
        let mut f = with_group(vec![def(10, 0.5), def(20, 1.0)]);
        f.post(HUD, None, 10, "", vec![]);
        f.post(HUD, None, 20, "", vec![]);
        let mut reqs = Vec::new();
        f.update(0.25, Some(0.25), &mut reqs);
        let p = f.queues[HUD].play.clone().unwrap();
        assert_eq!((p.sequence, p.layer, p.state, p.visible), (11, 11, 1, true));
        f.update(0.25, Some(0.25), &mut reqs);
        // 0.5 s counted: done and removed; the play goes with it.
        assert_eq!(f.queues[HUD].messages.len(), 1);
        assert!(f.queues[HUD].play.is_none());
        f.update(0.1, None, &mut reqs);
        assert_eq!(f.queues[HUD].play.as_ref().unwrap().sequence, 21);
        // Its group released: the queue empties, then its play goes.
        f.set_group(7, None);
        f.update(0.1, None, &mut reqs);
        assert!(f.queues[HUD].messages.is_empty());
        f.update(0.1, None, &mut reqs);
        assert!(f.queues[HUD].play.is_none());
    }

    #[test]
    fn sequence_extends_and_goes_out() {
        let mut d = def(10, 0.0);
        d.extend_by_sequence = true;
        let mut f = with_group(vec![d]);
        f.post(SONG, None, 10, "", vec![]);
        let mut reqs = Vec::new();
        f.update(0.1, Some(0.1), &mut reqs);
        assert!(!f.queues[SONG].messages[0].timed);
        f.sequences.insert(11, SequenceLengths { in_len: 1.0, out_len: 0.5 });
        f.update(0.1, Some(0.1), &mut reqs);
        let m = &f.queues[SONG].messages[0];
        assert!(m.timed && !m.waiting_sequence);
        assert_eq!(m.timer, 4_500_000);
        assert_eq!(f.queues[SONG].play.as_ref().unwrap().out, Some(1_500_000));
        // 1.5 s → 0.4 s left, below the out time: the play goes out.
        f.update(1.1, Some(1.1), &mut reqs);
        let p = f.queues[SONG].play.as_ref().unwrap();
        assert_eq!((p.state, p.active), (3, false));
    }

    #[test]
    fn ui_messages_root() {
        let mut s = super::super::LogicState::default();
        s.feedback_queues.set_group(1, Some(vec![def(10, 4.0)]));
        assert_eq!(s.resolve_string("UIMessages.Songs.CurrentMessage.Text").as_deref(), Some(""));
        s.feedback_queues.post(SONG, None, 10, "", vec![]);
        s.feedback_queues.queues[SONG].messages[0].texts = ["Title".into(), "".into(), "Artist".into()];
        assert_eq!(s.resolve_string("UIMessages.Songs.CurrentMessage.Text").as_deref(), Some("Title"));
        assert_eq!(s.resolve_string("UIMessages.Songs.CurrentMessage.SubText2").as_deref(), Some("Artist"));
        assert_eq!(s.resolve_bool("UIMessages.Songs.CurrentMessage.SubText1(IS_EMPTY)"), Some(true));
    }

    #[test]
    fn exclusive_suspends_others() {
        let mut d = def(10, 1.0);
        d.exclusive = true;
        let mut f = with_group(vec![d, def(20, 5.0)]);
        f.post(HUD, None, 20, "", vec![]);
        f.post(SONG, None, 10, "", vec![]);
        let mut reqs = Vec::new();
        f.update(0.1, Some(0.1), &mut reqs);
        assert!(f.queues[HUD].suspended && !f.queues[SONG].suspended);
        f.update(0.1, Some(0.1), &mut reqs);
        assert!(!f.queues[HUD].play.as_ref().unwrap().visible);
        let t = f.queues[HUD].messages[0].timer;
        f.update(1.0, Some(1.0), &mut reqs);
        // The exclusive one is done: the others resume; the suspended timer did not count.
        assert!(!f.queues[HUD].suspended);
        assert_eq!(f.queues[HUD].messages[0].timer, t);
        assert!(f.apply(&Request::ClearHudMessageQueues));
        assert!(f.queues[HUD].messages.is_empty());
        assert_eq!(f.post_named("System", 20), 3);
        assert!(f.queues[f.find_queue("System").unwrap()].queued(3));
        assert!(f.apply(&Request::ClearFeedbackQueue { queue: "System".into() }));
        assert!(f.queues.iter().all(|q| q.messages.is_empty()));
    }

    #[test]
    fn transition_message_and_pause() {
        let mut d = def(10, 1.0);
        d.transition_message = 20;
        d.pause_with_game = true;
        let mut f = with_group(vec![d, def(20, 1.0)]);
        f.post(HUD, None, 10, "", vec![]);
        let mut reqs = Vec::new();
        f.update(0.5, None, &mut reqs);
        assert_eq!(f.queues[TRANSITION].messages[0].def.id, 20);
        // Paused game: the pause-with-game timer holds, its play pauses.
        assert_eq!(f.queues[HUD].messages[0].timer, 3_000_000);
        assert_eq!(f.queues[HUD].play.as_ref().unwrap().state, 2);
        f.update(0.5, Some(0.5), &mut reqs);
        assert_eq!(f.queues[HUD].messages[0].timer, 1_500_000);
        assert_eq!(f.queues[HUD].play.as_ref().unwrap().state, 1);
    }
}
