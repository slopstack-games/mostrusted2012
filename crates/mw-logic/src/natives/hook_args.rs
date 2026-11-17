//! Reading an HSM hook's arguments as the original's handlers do: the data-definition member the behaviour passes
//! (`Params`, `EnterParams`, ...) is a JSON object; a handler looks one member up by its name hash
//! (`FUN_009f16b0`, the hash a 32-bit constant in `.bss`) and checks its value tag (2 string, 3 int, 5 bool;
//! `mw_formats::json`). On a missing member or a wrong tag the handler reads through a null pointer (the data
//! never does that on the paths it runs); here that reads as the type's zero value (model decision, as the HSM
//! interpreter's missing records).

use mw_formats::json::Json;

/// `lpcValue` (`DAT_01177acc` = 0xc6d9cf16): the string argument of most hooks.
pub const LPC_VALUE: u32 = 0xc6d9_cf16;
/// `liEventId` (`DAT_01177a44`).
pub const LI_EVENT_ID: u32 = 0xf3c7_b312;
/// `Won` (`DAT_01177ad8`, `SetGameModeWon`).
pub const WON: u32 = 0xf3d6_25b7;
/// `StoryMode` (`DAT_01177d44`).
pub const STORY_MODE: u32 = 0x4e5f_12d2;
/// `NewGame` (`DAT_01177a70`).
pub const NEW_GAME: u32 = 0x7bec_b68d;
/// `lbEnable` (`DAT_01177e1c`, `SetAllowLoadingScreen`).
pub const LB_ENABLE: u32 = 0xefa3_4ec0;
/// `Retry` (`DAT_01177ad4`, `SetRetryAtEnd`).
pub const RETRY: u32 = 0xa596_a25b;
/// `lbSeen` (`DAT_01177af4`, `SetEasyDriveOptionSeen`).
pub const LB_SEEN: u32 = 0xa783_3b84;
/// `liId` (`DAT_01177b34`, `SetVehicleAndRolloutFromSaveGameByVehicleID`).
pub const LI_ID: u32 = 0x6c69_4964;
/// The bool member `SetValidProgressionData` reads (`DAT_01177a6c`; name not found).
pub const VALID_PROGRESSION_DATA: u32 = 0xce68_ae4f;

/// `FUN_009f16b0` + the tag check: the member whose name hashes to `hash`, if it has the value tag `tag`.
pub fn member(args: Option<&Json>, hash: u32, tag: u8) -> Option<&Json> {
    args?.get_hash(hash).filter(|v| v.tag() == tag)
}

/// A string member (tag 2); "" when missing (model decision, see the module doc).
pub fn string(args: Option<&Json>, hash: u32) -> &str {
    member(args, hash, 2).and_then(Json::as_str).unwrap_or("")
}

/// An int member (tag 3), as the handlers read it: the low 32 bits.
pub fn int(args: Option<&Json>, hash: u32) -> i32 {
    member(args, hash, 3).and_then(Json::as_i64).unwrap_or(0) as i32
}

/// A bool member (tag 5).
pub fn boolean(args: Option<&Json>, hash: u32) -> bool {
    member(args, hash, 5).and_then(Json::as_bool).unwrap_or(false)
}

/// `strtol(s, NULL, 10)` (`FUN_00d751e0(s, 0, 10)`, the MSVC CRT): leading white space, an optional sign, decimal
/// digits; 0 when there are none; clamped to the 32-bit range on overflow.
pub fn strtol10(s: &str) -> i32 {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let negative = match b.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut v: i64 = 0;
    let mut overflow = false;
    while let Some(d @ b'0'..=b'9') = b.get(i).copied() {
        v = v * 10 + (d - b'0') as i64;
        if v > u32::MAX as i64 {
            overflow = true;
            v = u32::MAX as i64;
        }
        i += 1;
    }
    if negative {
        if overflow || v > -(i32::MIN as i64) { i32::MIN } else { (-v) as i32 }
    } else if overflow || v > i32::MAX as i64 {
        i32::MAX
    } else {
        v as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mw_formats::genesys::field_hash;

    #[test]
    fn member_hashes_are_the_names() {
        assert_eq!(field_hash("lpcValue"), LPC_VALUE);
        assert_eq!(field_hash("liEventId"), LI_EVENT_ID);
        assert_eq!(field_hash("Won"), WON);
        assert_eq!(field_hash("StoryMode"), STORY_MODE);
        assert_eq!(field_hash("NewGame"), NEW_GAME);
        assert_eq!(field_hash("lbEnable"), LB_ENABLE);
        assert_eq!(field_hash("Retry"), RETRY);
        assert_eq!(field_hash("lbSeen"), LB_SEEN);
        assert_eq!(field_hash("liId"), LI_ID);
    }

    #[test]
    fn reads_members_by_tag() {
        let j = Json::parse(r#"{ "Won": true, "lpcValue": "42", "liId": 7 }"#).unwrap();
        assert!(boolean(Some(&j), WON));
        assert_eq!(string(Some(&j), LPC_VALUE), "42");
        assert_eq!(int(Some(&j), LI_ID), 7);
        // Wrong tag / missing: the zero value.
        assert_eq!(int(Some(&j), LPC_VALUE), 0);
        assert!(!boolean(None, WON));
    }

    #[test]
    fn strtol_like_the_crt() {
        assert_eq!(strtol10("  -12abc"), -12);
        assert_eq!(strtol10("+7"), 7);
        assert_eq!(strtol10("x1"), 0);
        assert_eq!(strtol10("99999999999"), i32::MAX);
        assert_eq!(strtol10("-99999999999"), i32::MIN);
    }
}
