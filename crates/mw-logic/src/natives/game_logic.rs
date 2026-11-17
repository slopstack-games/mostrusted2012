//! The game-logic object **G** (`*0x01177868`) as far as the HSM hooks read and write it directly, and the
//! `GameLogicConfig` fields they read. Offsets are G's unless stated; the managers G points at have their own
//! files (`game_modes.rs` GMM = G+0x2f3fc, `players.rs` PM = G+0x2f3dc, `progression.rs`).

/// Fields of G the hooks touch.
#[derive(Debug, Clone, Default)]
pub struct GameLogic {
    /// The event launcher at G+0x2f4c8 (`FUN_0063e380` sets it): +0x230 = G+0x2f6f8 the **pending event** (the
    /// event a respawn at the event start and the new-game intro check read), +0x234 = −1 on every set, +0x238
    /// a flag.
    pub pending_event: u32,
    /// Launcher +0x234.
    pub pending_event_aux: i32,
    /// Launcher +0x238.
    pub pending_event_flag: bool,
    /// Events listed at G+0x29204 (pairs of 8 B sorted by id, count G+0x2920c; `FUN_0068fa10`'s first pass
    /// takes only these). Filled by the runtime from the profile (what the list holds is open).
    pub listed_events: Vec<u32>,
    /// The car-control block set at G+0x2f0a4.
    pub input_blocks: InputBlocks,
    /// Matchmaking state (`FUN_00675470` clears it).
    pub matchmaking: Matchmaking,
    /// G+0x2a9e4: the online game's visibility (2 invite only, 3 joinable, 4 public).
    pub game_visibility: u32,
    /// G+0x2922c: set with every profile save request (`SaveProfile`, `FUN_0068fd50`, `FUN_006798a0`).
    pub save_pending: bool,
    /// `DAT_011a2470`: `ShutdownGame` sets it; the main loop quits.
    pub shutdown: bool,
    /// The loading-screen manager's request mask (UIM+0x2d204 = LSM+0x54, one bit per reason; bit 5 =
    /// `HSB_TOG_LS`; decomp.md "Boot, loading screens and the free-roam start").
    pub loading_screen_requests: u32,
    /// UIM+0x2d21c (cleared with bit 5 by `ToggleLoadingScreenOff`; meaning open).
    pub loading_screen_2d21c: u32,
    /// `DAT_01179de0`: the running `HSB_RUNNING` ("Show World") states.
    pub show_world_count: i32,
}

/// The block set at G+0x2f0a4 (`FUN_0083c930` clears a bit): a mask word per group at +group×4, a count byte per
/// group at +0x1c+group (decremented when a set bit is cleared), and at +0x24 (G+0x2f0c8) the intro's car-control
/// suspension time (`HACKSuspendCarControlForIntro` 3 s, `.rdata` 0x00f92eb0).
#[derive(Debug, Clone, Default)]
pub struct InputBlocks {
    pub masks: [u32; 7],
    pub counts: [u8; 7],
    pub suspend_time: f32,
}

impl InputBlocks {
    /// `FUN_0083d000`(bit) on group 0 (the inverse of [`clear`](Self::clear): sets the bit, counts it up).
    pub fn set(&mut self, group: usize, bit: u32) {
        let before = self.masks[group];
        self.masks[group] |= 1 << (bit & 31);
        if before != self.masks[group] {
            self.counts[group] = self.counts[group].wrapping_add(1);
        }
    }

    /// `FUN_0083c930`(group, bit).
    pub fn clear(&mut self, group: usize, bit: u32) {
        let before = self.masks[group];
        self.masks[group] &= !(1 << (bit & 31));
        if before != self.masks[group] {
            self.counts[group] = self.counts[group].wrapping_sub(1);
        }
    }
}

/// G+0x2f490..+0x2f708 (`FUN_00675470` zeroes all of it; `QuickJoinGameDesired` / `CreateGameDesired` set the
/// first and last).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Matchmaking {
    /// +0x2f490 (u16).
    pub quick_join: u16,
    /// +0x2f498, +0x2f49c, +0x2f4a0, +0x2f4a4.
    pub words_498: [u32; 4],
    /// +0x2f4a8.
    pub byte_4a8: u8,
    /// +0x2f4ac..+0x2f4c0.
    pub words_4ac: [u32; 6],
    /// +0x2f708.
    pub create_game: bool,
}

/// `GameLogicConfig` (`GLOBALCONFIG.BNDL`, the handle at `GlobalConfig` +0x24 = `DAT_010edac0`): the fields the
/// hooks read, filled by the runtime from the install (0 = not loaded).
#[derive(Debug, Clone, Default)]
pub struct LogicConfig {
    /// +0x60 `4c86472d` (390185): the game mode whose events show the escape sequence (`ShouldShowEscapeSequence`).
    pub escape_mode: u32,
    /// +0xa4 `DefaultRollout` (523000): the new game's car (`FUN_0068fd50`).
    pub default_rollout: u32,
    /// +0xd8 `1b3c1ddc` (390145): the free-drive game mode (`FUN_0068fa10`, the start position).
    pub freedrive_mode: u32,
    /// +0xfc `4b9afa9f` (1560354, "WELCOME TO FAIRHAVEN"): the intro event (`SetIntroEventNext`).
    pub intro_event: u32,
    /// +0x100 `814c73fc` (1909234): the event `IsIntroRace` compares the current one with.
    pub intro_race_event: u32,
    /// +0x130 `3d09a25a` (431133): the other free-drive mode `FUN_006e5ea0` accepts (the online one).
    pub online_freedrive_mode: u32,
}
