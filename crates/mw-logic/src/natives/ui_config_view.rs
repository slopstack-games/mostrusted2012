//! The `UIConfig` binding root: `UIConfigManager` (`FUN_007a03c0`, root "UIConfig", type size 0x41c; the manager
//! is the UI configuration object, the registration sits next to its constructor). Members the data reads:
//!
//! - fields: `IsHighDef` (+0x418, the constructor sets 1), `NumberOfMonitors` (+0x414, 1), `Platform` (+0x208, a
//!   string pointer the constructor sets to the `.rdata` string "PC" at 0xfde864), `IsPlatformPC` (+0x20c, 1),
//!   `IsPlatformX360` / `PS3` / `VITA` (+0x20d / 0x20e / 0x20f, 0). The PC build is the only platform this port
//!   runs, so these are the constructor's values.
//! - language getters (UI language id at manager +0xb4, 0 until the text database tells the UI; the same codes the
//!   language menu uses, `lua_widgets::LANGUAGE_BY_INDEX`): `IsEnglish` (0x78ec50: 1 or 2), `IsGerman` (0x78ecc0:
//!   6), `IsRussian` (0x78ece0: 7), `IsJapanese` (0x78eca0: 0xb), `IsTraditionalChinese` (0x78ec80: 0xc),
//!   `IsArabic` (0x78ed00: 0xe).
//! - `ShouldShowSubtitleOptions` (0x7a9df0) and `IsLeftToRightLanguage` (0x7a9e20): language 0 answers true;
//!   otherwise they read byte +0x35 / +0x32 of the language's record in the UI's language table (`FUN_00794f30`),
//!   which is not ported. Model decision: `IsLeftToRightLanguage` is false only for Arabic (0xe) and the subtitle
//!   options are shown.
//!
//! `HudAspectCorrection.*` and `Data.*` are not answered here: they come from the UI data (`mw_assets::ui`,
//! published definitions).

use crate::binding::{Lookup, Segment, Value};

use super::LogicState;

/// The UI language ids (`UIConfig` +0xb4).
mod lang {
    pub const GERMAN: u32 = 6;
    pub const RUSSIAN: u32 = 7;
    pub const JAPANESE: u32 = 0xb;
    pub const TRADITIONAL_CHINESE: u32 = 0xc;
    pub const ARABIC: u32 = 0xe;
}

/// `UIConfig.<member>`.
pub fn lookup(s: &LogicState, members: &[Segment]) -> Lookup {
    let Some((first, rest)) = members.split_first() else { return Lookup::Found(Value::Object(true)) };
    let l = s.screen.language;
    let b = |v: bool| Lookup::Found(Value::Bool(v));
    let is = |n: &str| first.hash == crate::binding::hash(n);
    if !rest.is_empty() {
        return Lookup::Null;
    }
    if is("Platform") {
        Lookup::Found(Value::Str("PC".into()))
    } else if is("IsPlatformPC") || is("IsHighDef") {
        b(true)
    } else if is("IsPlatformX360") || is("IsPlatformPS3") || is("IsPlatformVITA") {
        b(false)
    } else if is("NumberOfMonitors") {
        Lookup::Found(Value::Int(1))
    } else if is("IsEnglish") {
        b(l == 1 || l == 2)
    } else if is("IsGerman") {
        b(l == lang::GERMAN)
    } else if is("IsRussian") {
        b(l == lang::RUSSIAN)
    } else if is("IsJapanese") {
        b(l == lang::JAPANESE)
    } else if is("IsTraditionalChinese") {
        b(l == lang::TRADITIONAL_CHINESE)
    } else if is("IsArabic") {
        b(l == lang::ARABIC)
    } else if is("ShouldShowSubtitleOptions") {
        b(true)
    } else if is("IsLeftToRightLanguage") {
        b(l != lang::ARABIC)
    } else {
        Lookup::Null
    }
}

#[cfg(test)]
mod tests {
    use super::super::LogicState;

    fn resolves(s: &LogicState, path: &str) -> bool {
        crate::binding::Binding::parse(path).unwrap().eval_bool(s).ok
    }

    #[test]
    fn ui_config_root() {
        let mut s = LogicState::default();
        assert_eq!(s.resolve_string("UIConfig.Platform").as_deref(), Some("PC"));
        assert_eq!(s.resolve_bool("UIConfig.Platform(EQUALS:PC)"), Some(true));
        assert_eq!(s.resolve_bool("UIConfig.Platform(EQUALS:X360)"), Some(false));
        assert_eq!(s.resolve_bool("UIConfig.IsPlatformPC"), Some(true));
        assert_eq!(s.resolve_bool("UIConfig.IsPlatformX360(NOT)"), Some(true));
        assert_eq!(s.resolve_bool("UIConfig.IsHighDef"), Some(true));
        assert_eq!(s.resolve_bool("UIConfig.IsEnglish"), Some(false), "language 0 until the text database sets it");
        s.screen.language = 1;
        assert_eq!(s.resolve_bool("UIConfig.IsEnglish"), Some(true));
        assert_eq!(s.resolve_bool("UIConfig.IsLeftToRightLanguage"), Some(true));
        s.screen.language = 0xe;
        assert_eq!(s.resolve_bool("UIConfig.IsArabic"), Some(true));
        assert_eq!(s.resolve_bool("UIConfig.IsLeftToRightLanguage(NOT)"), Some(true));
        // Unregistered members fall through to published data (none here).
        assert!(!resolves(&s, "UIConfig.NoSuchMember"));
    }
}
