//! The `SignInState` binding root: `SignInViewModel` (`FUN_00888d40`, root "SignInState", type size 0x3148); its
//! `Params` member is the `SignInParamsViewModel` (VM +8, size 0x1590). The HSM reads these params:
//!
//! | Member | Source | Offline |
//! |---|---|---|
//! | `IsChatRestricted` | getter 0x889bf0 stores 0 | false |
//! | `IsMultiplayerAllowed` | getter 0x889be0 stores 1 | true |
//! | `IsAccountDeactivated` | 0x881610: error code VM +0x135c == 0x1b (0 after the reset) | false |
//! | `AuthenticatedWithBlaze` | 0x889c10 -> 0x4c2070 -> 0xa82f20: the Blaze connection (net module +0x56b0, +0x358) is null | false |
//! | `IsReadyForToS` | 0x881650: byte +0x1589, written by the ToS-memory message only | false |
//! | `AccountCreated`, `FirstUseInTitle`, `EMailHasEAAccount`, `EAInfo`, `ShareInfo`, `ShareUsage`, `NeedToReviewTofS`, `RememberMe`, `IsErrorCodeAvailable` | plain bytes (+0x136c, 0x1367, 0x136b, 0x1368, 0x1369, 0x136a, 0x1366, 0x1364, 0x136d), cleared by the VM reset | false |
//! | `EMail`, `Password`, `Persona`, `ProductKey`, `UnlockKey` | string buffers (e.g. +0x414), empty before sign-in | "" |
//! | `TofSLineCount` / `PrivacyLineCount` | words (+0x1370 ...) | 0 |
//!
//! Model decision: no sign-in flow is ported (the player is never signed in), so the VM keeps its reset values.
//! `Error` is not a member of the root (the data's `SignInState.Error` DATACONTEXT does not resolve).

use crate::binding::{Lookup, Segment, Value};

use super::LogicState;

const FALSE_MEMBERS: &[&str] = &[
    "IsChatRestricted",
    "IsAllUgcBlocked",
    "IsAccountDeactivated",
    "AuthenticatedWithBlaze",
    "IsReadyForToS",
    "AccountCreated",
    "FirstUseInTitle",
    "EMailHasEAAccount",
    "EAInfo",
    "ShareInfo",
    "ShareUsage",
    "NeedToReviewTofS",
    "RememberMe",
    "IsErrorCodeAvailable",
];

/// `SignInState.<member>[...]`.
pub fn lookup(_s: &LogicState, members: &[Segment]) -> Lookup {
    let Some((first, rest)) = members.split_first() else { return Lookup::Found(Value::Object(true)) };
    if first.hash != crate::binding::hash("Params") {
        return Lookup::Null;
    }
    let Some((m, _)) = rest.split_first() else { return Lookup::Found(Value::Object(true)) };
    let is = |n: &str| m.hash == crate::binding::hash(n);
    if is("IsMultiplayerAllowed") {
        Lookup::Found(Value::Bool(true))
    } else if FALSE_MEMBERS.iter().any(|n| is(n)) {
        Lookup::Found(Value::Bool(false))
    } else if ["EMail", "Password", "Persona", "ProductKey", "UnlockKey"].iter().any(|n| is(n)) {
        Lookup::Found(Value::Str(String::new()))
    } else if is("TofSLineCount") || is("PrivacyLineCount") {
        Lookup::Found(Value::Int(0))
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
    fn sign_in_root() {
        let s = LogicState::default();
        assert_eq!(s.resolve_bool("SignInState.Params.IsChatRestricted"), Some(false));
        assert_eq!(s.resolve_bool("SignInState.Params.IsMultiplayerAllowed"), Some(true));
        assert_eq!(s.resolve_bool("SignInState.Params.AuthenticatedWithBlaze"), Some(false));
        assert_eq!(s.resolve_bool("SignInState.Params.IsAccountDeactivated"), Some(false));
        assert_eq!(s.resolve_bool("SignInState.Params.FirstUseInTitle"), Some(false));
        assert_eq!(s.resolve_int("SignInState.Params.PrivacyLineCount(ADD:2)"), Some(2));
        assert!(!resolves(&s, "SignInState.Error"));
    }
}
