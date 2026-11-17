//! More behaviour types of the free-drive mode machines (`GAMELOGIC/GAMEMODES.BNDL`): rumble and message pauses,
//! the song list, weapons, minimap icons, the profile sync, the chevron, AI pause and the body-shop watcher.
//! Engine effects are [`Request`]s, engine state [`Query`]s (both name the original's code).

use super::{Query, ask, ask_bool, param_int, param_true};
use crate::hsm::{Behaviour, Cx, EXIT, Registry};
use crate::natives::Request;

pub fn register(r: &mut Registry) {
    r.register_fn("MUTEALLRUMBL", |_| MuteRumble::default());
    r.register_fn("PAUSEMSGS", |_| PauseMessages::default());
    r.register_fn("PLAYSONGLIST", |_| PlaySongList);
    r.register_fn("WEAPONSOK", |_| WeaponsOk);
    r.register_fn("MINIMAPICONS", |_| MinimapIcons);
    r.register_fn("SYNCPROFILEW", |_| SyncProfileWrite::default());
    r.register_fn("HIDECHEV", |_| HideChevron);
    r.register_fn("PAUSEAI", |_| PauseAi);
    r.register_fn("DOBODYSHOP", |_| DoBodyShop);
}

/// The rumble mute reasons (`PTR_s_Unknown_010c5c10`, 6 entries).
pub const RUMBLE_REASONS: [&str; 6] = ["Unknown", "LoadingScreen", "CarSelect", "Pause", "Autolog", "Settings"];

/// "Mute All Rumble" (`MUTEALLRUMBL`, factory 0x77cb40, vtable 0xfd9f24, 0x50 B): enter (0x73e170) maps the
/// `MUTEREASON` parameter (compared without case with entries 1..5 of the table, else 0) into +0x48 and sets
/// that reason's rumble mute; exit (0x73e1f0) clears it. Runs its children; never exits by itself.
#[derive(Default)]
pub struct MuteRumble {
    reason: u8,
}

impl Behaviour for MuteRumble {
    fn on_enter(&mut self, cx: &mut Cx) {
        let name = cx.param_text("MUTEREASON");
        self.reason =
            (1..RUMBLE_REASONS.len()).find(|&i| RUMBLE_REASONS[i].eq_ignore_ascii_case(&name)).unwrap_or(0) as u8;
        cx.host.request(Request::MuteRumble { reason: self.reason, on: true });
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::MuteRumble { reason: self.reason, on: false });
    }
}

/// `PAUSEMSGS` flag bits in [`Request::PauseMessages`]: the boolean parameters that read "true" (`_stricmp`),
/// each with the instance byte that holds it.
pub const PAUSE_HUD: u16 = 1; // PAUSEHUD +0x9d
pub const PAUSE_HUD_ONLY: u16 = 1 << 1; // PAUSEHUDONLY +0x9e
pub const PAUSE_HUD_SMALL: u16 = 1 << 2; // PAUSEHUDSML +0x9f
pub const PAUSE_MWL_SEQS: u16 = 1 << 3; // PAUSEMWLSEQS +0xa3
pub const PAUSE_MILESTONES: u16 = 1 << 4; // PAUSEMILESTN +0xa4
pub const PAUSE_NIS: u16 = 1 << 5; // PAUSENIS +0xa1
pub const PAUSE_SPEECH: u16 = 1 << 6; // PAUSESPEECH +0xa2
pub const PAUSE_FORCE: u16 = 1 << 7; // FORCEPAUSE +0x9c
pub const PAUSE_DEFER_HUD: u16 = 1 << 8; // DEFERHUD +0xa5
pub const PAUSE_INSTANT: u16 = 1 << 9; // PAUSEINSTANT +0xa0

/// "Pause Messages" (`PAUSEMSGS`, factory 0x77bb50, vtable 0xfd97d0, 0xa8 B): enter (0x74e890) finds the
/// message queues named by the `QUEUES` list (at most 8), pauses them (+0x25 = 1), reads the ten boolean
/// parameters above and counts the HUD message categories they name up; update (0x74edd0): with `FORCEPAUSE`
/// every found queue still unpaused is paused again; exit (0x74ee10) resumes the queues and counts the
/// categories down. Runs its children. The effects are [`Request::PauseMessages`] (the counter bytes are listed
/// there); the original keeps the found queue pointers, here the host looks the names up again.
#[derive(Default)]
pub struct PauseMessages {
    queues: Vec<String>,
    flags: u16,
}

impl Behaviour for PauseMessages {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.queues = cx.param("QUEUES").map_or_else(Vec::new, |p| p.values.iter().take(8).cloned().collect());
        const PARAMS: [(&str, u16); 10] = [
            ("PAUSEHUD", PAUSE_HUD),
            ("PAUSEHUDONLY", PAUSE_HUD_ONLY),
            ("PAUSEHUDSML", PAUSE_HUD_SMALL),
            ("PAUSEMWLSEQS", PAUSE_MWL_SEQS),
            ("PAUSEMILESTN", PAUSE_MILESTONES),
            ("PAUSENIS", PAUSE_NIS),
            ("PAUSESPEECH", PAUSE_SPEECH),
            ("FORCEPAUSE", PAUSE_FORCE),
            ("DEFERHUD", PAUSE_DEFER_HUD),
            ("PAUSEINSTANT", PAUSE_INSTANT),
        ];
        self.flags = PARAMS.iter().filter(|(name, _)| param_true(cx, name)).fold(0, |f, (_, bit)| f | bit);
        cx.host.request(Request::PauseMessages { queues: self.queues.clone(), flags: self.flags, on: true });
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if self.flags & PAUSE_FORCE != 0 {
            cx.host.request(Request::ForcePauseMessageQueues { queues: self.queues.clone() });
        }
        false
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::PauseMessages { queues: self.queues.clone(), flags: self.flags, on: false });
    }
}

/// "Play Song List" (`PLAYSONGLIST`, factory 0x77c610, vtable 0xfd9cdc, 0x48 B): enter (0x757e90) starts the
/// song list. The list (0x74f540): `SONGLIST` (atoi), else unless `USEDEFAULT` is "true" the event's own list
/// ([`Query::CurrentEventSongList`]), else the event's default list ([`Query::DefaultEventSongList`]). Update
/// (0x744b50): with `STOPONLEAVE` "true" it keeps running, else it exits through `EXIT` at once; exit begin
/// (0x744b00) stops the list when `STOPONLEAVE` is "true".
pub struct PlaySongList;

impl Behaviour for PlaySongList {
    fn on_enter(&mut self, cx: &mut Cx) {
        let mut list = param_int(cx, "SONGLIST");
        if list == 0 && !param_true(cx, "USEDEFAULT") {
            list = ask(cx, Query::CurrentEventSongList) as i64;
        }
        if list == 0 {
            list = ask(cx, Query::DefaultEventSongList) as i64;
        }
        let allow_playout = param_true(cx, "ALLOWPLAYOUT");
        cx.host.request(Request::PlaySongList { list, allow_playout });
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if param_true(cx, "STOPONLEAVE") {
            return false;
        }
        cx.set_exit(EXIT);
        true
    }

    fn on_exit_begin(&mut self, cx: &mut Cx) {
        if param_true(cx, "STOPONLEAVE") {
            cx.host.request(Request::StopSongList);
        }
    }
}

/// "Weapons OK" (`WEAPONSOK`, factory 0x77d8a0, vtable 0xfda5d8, 0x48 B): enter (0x753ad0) sets the object
/// manager's weapons-allowed byte: always for an online event (`FUN_006dfed0`), else from the event record's
/// byte +0xf2; update (0x741ba0) 0, runs its children; exit begin (0x741bb0) clears the byte and, unless
/// GMM+0x2e80 is set, resets the players' weapon state (`FUN_006c8e90`).
pub struct WeaponsOk;

impl Behaviour for WeaponsOk {
    fn on_enter(&mut self, cx: &mut Cx) {
        let allowed = ask_bool(cx, Query::OnlineEvent) || ask_bool(cx, Query::EventWeaponsAllowed);
        cx.host.request(Request::WeaponsAllowed(allowed));
    }

    fn on_exit_begin(&mut self, cx: &mut Cx) {
        cx.host.request(Request::WeaponsAllowed(false));
        if !ask_bool(cx, Query::GmmFlag2e80) {
            cx.host.request(Request::ResetPlayerWeapons);
        }
    }
}

/// "Minimap Icons" (`MINIMAPICONS`, factory 0x77c480, vtable 0xfd9c28, 0x48 B): only the update (0x7579b0)
/// does anything: every frame [`Request::UpdateMinimapIcons`]; never exits.
pub struct MinimapIcons;

impl Behaviour for MinimapIcons {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.host.request(Request::UpdateMinimapIcons);
        false
    }
}

/// "Sync Profile Write" (`SYNCPROFILEW`, factory 0x77fed0, vtable 0xfdc194, 0x728 B; base constructor
/// 0x4c8640): enter (0x7462e0) saves G+0x2ee5c (+0xd0), reads `SYNCDDONLY` (+0xd1), sets state +0xd4 = 0 and
/// allocates two buffers (0x1400 / 0x4000 B), freed on exit (0x741090). The update (0x763a00) is a 16-state
/// machine over the online profile service; state 0 with no connection (`FUN_004c2050`) goes to state 15
/// (returning 0), whose next update restores the flag (saved 0: G+0x2ee5c = 1) and exits through `EXIT`.
/// **Model decision:** the connected states 1..14 (the sync itself) are not ported; connected is treated like
/// offline.
#[derive(Default)]
pub struct SyncProfileWrite {
    saved_flag: bool,
    state: u32,
}

impl Behaviour for SyncProfileWrite {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.saved_flag = ask_bool(cx, Query::ProfileSyncFlag);
        self.state = 0;
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if self.state == 0 {
            let _connected = ask_bool(cx, Query::ProfileServiceConnected);
            self.state = 0xf;
            return false;
        }
        if !self.saved_flag {
            cx.host.request(Request::SetProfileSyncFlag);
        }
        cx.set_exit(EXIT);
        true
    }
}

/// "Hide Chevron" (`HIDECHEV`, factory 0x77df60, vtable 0xfda8f0, 0x48 B): hides the HUD chevron on enter
/// (0x75bbf0) and shows it on exit (0x75bc20); runs its children (update 0x73d0a0 = 0).
pub struct HideChevron;

impl Behaviour for HideChevron {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::HideChevron { hide: true });
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::HideChevron { hide: false });
    }
}

/// "Pause AI" (`PAUSEAI`, factory 0x77ade0, vtable 0xfd922c, 0x48 B): pauses the AI on enter (0x73e450) and
/// resumes it on exit (0x73e4c0); runs its children (update 0x73e4b0 = 0).
pub struct PauseAi;

impl Behaviour for PauseAi {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::PauseAi { paused: true });
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::PauseAi { paused: false });
    }
}

/// "Do Body Shop" (`DOBODYSHOP`, factory 0x77b5d0, vtable 0xfd9554, 0x90 B): enter (0x742bf0) resets its state
/// (24 per-player flags +0x48.. = 1, the 4-entry timed list's count +0x80 and +0x84 = 0, the byte 0x01179dde =
/// 0: instance state, kept by the host with the request). The update (0x749220), every frame, walks the 24
/// player slots (G+0x2f3dc +0xa5d0), keeps the timed list (ids with a countdown by the frame time) and the
/// per-slot flags, and applies the local player's changed vehicle (`FUN_0069fce0` / `FUN_00729f20`); it never
/// exits. Ported as [`Request::UpdateBodyShop`] (the work is engine state); its inner reads (`FUN_005372f0` /
/// `FUN_005378d0`, the player's zone list) are not ported.
pub struct DoBodyShop;

impl Behaviour for DoBodyShop {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.host.request(Request::UpdateBodyShop);
        false
    }
}
