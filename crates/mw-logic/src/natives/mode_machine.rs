//! The game-mode manager (GMM, G+0x2f3fc) running a mode's HSM machine: `RUNGAMEMODE` asks for the event with
//! [`Request::StartGameMode`] (`FUN_00701cb0`, its logic fields in [`start_game_mode`]); the GMM's per-frame update
//! (`FUN_0070d580`, its machine part in [`update`]) starts the mode's top-level state once the event's data and the
//! mode's weapon list are loaded. The original calls the HSM manager's update (`FUN_00776010`) and then the GMM's in
//! the same frame (`FUN_006abc10`), so whoever owns the [`Hsm`] calls [`update`] right after [`Hsm::update`]; the
//! interpreter's host contract is unchanged (Requests out, Queries in).
//!
//! Free drive: the boot's `SetNewGame` makes the free-drive event the pending one (G+0x2f6f8, `FUN_0068fa10`: 390703
//! "FAIRHAVEN", mode 390145), "Run Game Mode" (390011) starts it, and the GMM starts the mode's instance 390147
//! "Freedrive Game Mode" (`GameMode` +0x1c; read live in both snapshots: GMM+0x2cfc = its serial).

use std::cell::RefCell;

use super::{LogicState, Request};
use crate::hsm::{Host, Hsm};

/// A `GameMode` item (GAMEMODES.BNDL) as the GMM uses it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModeInfo {
    /// +0x18: its id (what an event's +0x1c names).
    pub id: u32,
    /// +0x1c `0501d88b`: the instance id of its top-level HSM state.
    pub hsm: u32,
    /// +0x40 `WeaponList`: set on the local player before the machine starts.
    pub weapon_list: u32,
    /// +0x20 `8e9585cb`: the HUD's top-level instance (`Mode.HudHsm`).
    pub hud_hsm: u32,
    /// The `GameMode` object's scalar fields by field hash, what `Mode.Data.<field>` reads (the binding hash of a
    /// name of 4+ bytes is the GeneSys field hash): [`super::mode_view::data_fields`].
    pub data: Vec<(u32, crate::binding::Value)>,
}

/// `FUN_006e5ea0`(event): the event's mode is a free-drive mode (`GameLogicConfig` +0xd8 or +0x130).
pub fn is_freedrive_event(s: &LogicState, event: u32) -> bool {
    let mode = event_mode(s, event);
    mode == s.config.freedrive_mode || mode == s.config.online_freedrive_mode
}

/// `FUN_006e5dc0`(event) +0x1c: the event's game mode; an unknown event gives the null handle (`DAT_01179528`),
/// read here as mode 0.
pub fn event_mode(s: &LogicState, event: u32) -> u32 {
    s.game_modes.events.iter().find(|e| e.id == event).map_or(0, |e| e.mode)
}

/// `FUN_00701cb0`(event) on the GMM, the fields the game logic reads. Without `HasTransitionedEvents` (+0x2e7f) the
/// current event (+0x2ce0) becomes the pending one (G+0x2f6f8; `RUNGAMEMODE` passes that same value), the mode
/// handle (+0x2cf4) its event's mode, the machine serial (+0x2cfc) −1, and `IsFreedriveEvent` (+0x2e90) is
/// `FUN_006e5ea0`. Not ported: the event data manager's own start (`FUN_006f06d0`), the profile and player
/// set-up calls (`FUN_006a0cf0`, `FUN_0065ff60`, `FUN_00844fb0`, `FUN_006a16a0`), the timers and the other flags
/// (+0x2e8b..+0x2e93 compare the event with `GameLogicConfig` ids), the trailing calls (`FUN_006f89f0`,
/// `FUN_006d1380`, `FUN_006bb6a0`, `FUN_008432d0`, `FUN_006835c0`, `FUN_00843380`, `FUN_0071d1e0`): the runtime's
/// [`Request::StartGameMode`].
pub fn start_game_mode(s: &mut LogicState, event: u32) {
    if s.game_modes.has_transitioned_events {
        return;
    }
    let pending = s.game.pending_event;
    s.game_modes.current_event = pending;
    s.game_modes.current_mode = event_mode(s, event);
    s.game_modes.machine = None;
    s.game_modes.freedrive_event = is_freedrive_event(s, event);
}

/// `FUN_006dfe00`: an event runs (+0x2ce0 ≠ 0) and its data is loaded ((GMM+4)+0xdc == 4).
pub fn mode_started(s: &LogicState) -> bool {
    s.game_modes.current_event != 0 && s.game_modes.event_data_ready
}

/// `FUN_0070d580`, the machine part: with no machine, a started mode (`FUN_006dfe00`) and an offline event, set the
/// mode's weapon list on the local player (`FUN_006bb720`) and, once it is loaded (`FUN_006bb6f0`), start the
/// mode's top-level instance (`FUN_00775ed0`(the HSMData of (GMM+4)+0x104, `GameMode` +0x1c, GMM+0x2d00)) and keep
/// its serial (+0x2cfc; −1 when the instance does not exist, tried again next frame). Not ported: online events
/// (`FUN_006ded90` / `FUN_004c1ec0` decide; held here), the mode intro set-up after the start (`FUN_0070caa0`), the
/// running mode's upkeep (`FUN_007090f0`) and the end of the event once its machine finished (`FUN_0070b1e0`),
/// and the event data managers' own updates at the top of the function.
pub fn update(state: &RefCell<LogicState>, hsm: &mut Hsm, host: &mut dyn Host) {
    let instance = {
        let mut s = state.borrow_mut();
        if s.game_modes.machine.is_some() || !mode_started(&s) || s.game_modes.online_event {
            return;
        }
        let current = s.game_modes.current_mode;
        let Some((hsm, weapon_list)) =
            s.game_modes.modes.iter().find(|m| m.id == current).map(|m| (m.hsm, m.weapon_list))
        else {
            return;
        };
        s.game_modes.weapon_list = weapon_list;
        if s.game_modes.weapon_list_loaded != weapon_list {
            return;
        }
        hsm
    };
    let serial = hsm.start(instance, host);
    state.borrow_mut().game_modes.machine = serial;
}

/// What the logic host does with a request itself before the runtime gets it (the parts of the original's call
/// that only touch logic state).
pub fn apply(s: &mut LogicState, r: &Request) {
    match r {
        Request::StartGameMode { event } => start_game_mode(s, *event),
        Request::AdjustShowWorldCount { delta } => s.game.show_world_count += delta,
        Request::SetInputBlock { group, bit, set: true } if (*group as usize) < s.game.input_blocks.masks.len() => {
            s.game.input_blocks.set(*group as usize, *bit)
        }
        Request::SetInputBlock { group, bit, set: false } if (*group as usize) < s.game.input_blocks.masks.len() => {
            s.game.input_blocks.clear(*group as usize, *bit)
        }
        _ => {}
    }
}
