//! The `Event` binding root: the event data manager (`FUN_006dfbc0` registers the type "EventDataManager", 0x7ec
//! bytes, and the root "Event" on it; `FUN_006e6bf0` registers it again on the *other* of the GMM's two event data
//! managers when a new event's data is swapped in, so the root always names the loaded event). Fields:
//!
//! | Member | Offset | Type | Meaning |
//! |---|---|---|---|
//! | `Data` (`.rdata` 0x00fc5894) | +0xf4 | object | "The Genesys object": the event's GeneSys record |
//! | `GameMode` | +0xec | object | "The Game Mode Genesys object": the event's `GameMode` record |
//! | `HasDescription2` | +0xe9 | bool | "Workaround for Genesys binding eval not quite working" |
//!
//! `Event.Data.<field>` (`EventIntro`, `EventOutro`, `Laps`, `IsBlacklistEvent`, `TargetScore`, `Name2`,
//! `FeedbackMessageGroup`, `IntroMessageGroup`, `DemoMode`, ...) is a GeneSys field of the event by its field hash
//! (the binding hash of a name of 4+ bytes), exactly like [`Mode.Data`](super::mode_view): the events' scalar
//! fields come from the install ([`EventInfo::data`](super::game_modes::EventInfo)) and the current event is the
//! GMM's (+0x2ce0). `Event.GameMode.<field>` is the same for the event's `GameMode` object (the current mode's
//! record). The other `Event.*` members the data reads (`Id`, `Name2`, `IsRecommends`, `CustomItems`,
//! `Action.Options`, `speedwall.Items`, ...) are not registered members: they are data definitions the
//! HSM / UI publish under "Event" and fall through to [`ViewModels`](super::ViewModels).
//!
//! Model decisions: `HasDescription2` (set by the event-description code, not traced) is false; the loaded event is
//! taken to be the current one (+0x2ce0) once its data is in (the original swaps managers in `FUN_006e6bf0`, which
//! the logic keeps as [`GameModeManager::event_data_ready`](super::game_modes::GameModeManager)).

use crate::binding::{Lookup, Segment, Value, hash};

use super::LogicState;

/// `Event.<member>[...]`.
pub fn lookup(s: &LogicState, members: &[Segment]) -> Lookup {
    let Some((first, rest)) = members.split_first() else { return Lookup::Found(Value::Object(true)) };
    let gm = &s.game_modes;
    let is = |n: &str| first.hash == hash(n);
    if is("Data") {
        if gm.current_event == 0 {
            return Lookup::Null;
        }
        let Some(e) = gm.events.iter().find(|e| e.id == gm.current_event) else { return Lookup::Null };
        match rest.first() {
            None => Lookup::Found(Value::Object(true)),
            Some(f) => e.data.iter().find(|(h, _)| *h == f.hash).map(|(_, v)| v.clone()).into(),
        }
    } else if is("GameMode") {
        match (super::mode_view::current_mode(s), rest.first()) {
            (None, _) => Lookup::Null,
            (Some(_), None) => Lookup::Found(Value::Object(true)),
            (Some(m), Some(f)) => m.data.iter().find(|(h, _)| *h == f.hash).map(|(_, v)| v.clone()).into(),
        }
    } else if is("HasDescription2") {
        Lookup::Found(Value::Bool(false))
    } else {
        Lookup::Null
    }
}

#[cfg(test)]
mod tests {
    use super::super::LogicState;
    use super::super::game_modes::EventInfo;
    use super::super::mode_machine::ModeInfo;
    use crate::binding::{Value, hash};

    fn resolves(s: &LogicState, path: &str) -> bool {
        crate::binding::Binding::parse(path).unwrap().eval_int(s).ok
    }

    #[test]
    fn event_root() {
        let mut s = LogicState::default();
        // No event: the object is null.
        assert!(!resolves(&s, "Event.Data.EventIntro"));
        s.game_modes.events = vec![EventInfo {
            id: 462174,
            data: vec![(hash("EventIntro"), Value::UInt(1234)), (hash("Laps"), Value::UInt64(3))],
            ..Default::default()
        }];
        s.game_modes.modes =
            vec![ModeInfo { id: 5, data: vec![(hash("Online"), Value::Bool(false))], ..Default::default() }];
        s.game_modes.current_event = 462174;
        s.game_modes.current_mode = 5;
        assert_eq!(s.resolve_int("Event.Data.EventIntro"), Some(1234));
        assert_eq!(s.resolve_int("Event.Data.Laps"), Some(3));
        assert!(!resolves(&s, "Event.Data.Nothing"));
        assert_eq!(s.resolve_bool("Event.GameMode.Online"), Some(false));
        assert_eq!(s.resolve_bool("Event.HasDescription2"), Some(false));
        // Another event is not the loaded one.
        s.game_modes.current_event = 1;
        assert!(!resolves(&s, "Event.Data.EventIntro"));
        // A member that is not registered falls through to the published definition.
        s.view_models.publish("Event", mw_formats::json::Json::parse(r#"{"Id": 9}"#).unwrap());
        assert_eq!(s.resolve_int("Event.Id"), Some(9));
    }
}
