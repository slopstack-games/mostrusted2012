//! The profile's progression as far as the HSM hooks read and write it: the progression database (G+0x2f454
//! +0x4964, `FUN_0069fce0`; tables queried by name) and the profile manager at G+0x14c08 (PPM).
//!
//! The `last_played` table: one row per small key, a 64-bit `value` (`FUN_00732650`(key) reads it, −1 when there
//! is no row; `FUN_0072b120`(key, value) writes it and sets the database's changed flag +4). Keys the hooks use:
//! [`LAST_EVENT`] (1), [`INTRO_PLAYED`] (0xb), [`SAVED_POSITION`] (0xd).

use std::collections::BTreeMap;

/// `last_played` key 1: the last event played (`RetryLastEvent`, `FixUpRecommendationRetry`).
pub const LAST_EVENT: u8 = 1;
/// `last_played` key 0xb: the new-game intro was played (1 by `SetNewGameIntroCompleted`, 0 by
/// `SetIntroEventNext`; `ShouldPlayNewGameIntro` wants < 1, `IsIntroRace` exactly 0).
pub const INTRO_PLAYED: u8 = 0xb;
/// `last_played` key 0xd: read by `FUN_0068fd50` on a continued game for the saved position (`FUN_00847780`).
pub const SAVED_POSITION: u8 = 0xd;

/// The progression state the hooks touch.
#[derive(Debug, Clone, Default)]
pub struct Progression {
    /// The `last_played` table.
    pub last_played: BTreeMap<u8, i64>,
    /// Database +4: a write changed a row.
    pub changed: bool,
    /// (PPM+0xa0)+0x4964 ≠ 0: the progression database is loaded (`FUN_006e3910`).
    pub loaded: bool,
    /// PPM+0x22f (`FUN_006e0ef0`): a profile save is requested.
    pub save_requested: bool,
    /// `FUN_0083bd20`: `FUN_00571130()`+0x2108 > 0 (an engine count, read by `IsProgressionReady`; the runtime sets
    /// it; what it counts is open).
    pub world_count_ready: bool,
    /// (G+0x14e40)+0x2fe6: the collectables manager is ready (`IsProgressionReady`; the runtime sets it).
    pub collectables_ready: bool,
}

impl Progression {
    /// `FUN_00732650`(key).
    pub fn last_played(&self, key: u8) -> i64 {
        self.last_played.get(&key).copied().unwrap_or(-1)
    }

    /// `FUN_0072b120`(key, value): writes the row; the changed flag is set when a row was written (the original
    /// keeps it set once set).
    pub fn set_last_played(&mut self, key: u8, value: i64) {
        self.last_played.insert(key, value);
        self.changed = true;
    }

    /// `FUN_006e0ef0` on PPM.
    pub fn request_save(&mut self) {
        self.save_requested = true;
    }
}
