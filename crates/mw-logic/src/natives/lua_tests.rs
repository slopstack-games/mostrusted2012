//! One test per ported Lua native, called as the scripts do: a global of a Lua state with the registry installed
//! through [`LogicHost`], arguments as Lua values. Expected values are what the handlers do (argument
//! conversion, results, state), see the doc comments of each native.

use mw_formats::json::Json;
use mw_lua::{Lua, Value};

use super::lua_settings::{SettingsCategory, ValueSetting};
use super::{LogicHost, Request};

fn setup() -> (Lua, LogicHost) {
    let mut lua = Lua::new();
    let host = LogicHost::default();
    host.install_lua(&mut lua);
    (lua, host)
}

fn s(v: &str) -> Value {
    Value::from(v)
}

fn n(v: f64) -> Value {
    Value::Number(v)
}

fn call(lua: &mut Lua, name: &str, args: Vec<Value>) -> Vec<Value> {
    lua.call_global(name, args).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn num(r: &[Value]) -> f64 {
    r[0].as_number().unwrap()
}

fn text(r: &[Value]) -> Option<String> {
    r[0].as_str_lossy()
}

fn flag(r: &[Value]) -> bool {
    matches!(r[0], Value::Bool(true))
}

fn publish(host: &LogicHost, path: &str, json: &str) {
    host.state.borrow_mut().view_models.publish(path, Json::parse(json).unwrap());
}

#[test]
fn bindings() {
    let (mut lua, host) = setup();
    publish(&host, "Test", r#"{"B": true, "I": 7, "D": 2.5, "S": "hello"}"#);
    assert!(flag(&call(&mut lua, "ResolveBoolBinding", vec![s("Test.B")])));
    assert!(!flag(&call(&mut lua, "ResolveBoolBinding", vec![s("Test.Nope")])));
    assert_eq!(num(&call(&mut lua, "ResolveIntBinding", vec![s("Test.I")])), 7.0);
    assert_eq!(num(&call(&mut lua, "ResolveIntBinding", vec![s("Test.Nope")])), 0.0);
    assert_eq!(num(&call(&mut lua, "ResolveDoubleBinding", vec![s("Test.D")])), 2.5);
    assert_eq!(text(&call(&mut lua, "ResolveStringBinding", vec![s("Test.S")])).as_deref(), Some("hello"));
    assert_eq!(text(&call(&mut lua, "ResolveWideStringBinding", vec![s("Test.S")])).as_deref(), Some("hello"));
    // A nil argument reads as "<invalid>", which resolves to nothing.
    assert_eq!(text(&call(&mut lua, "ResolveStringBinding", vec![])).as_deref(), Some(""));
}

#[test]
fn exit_widget_group() {
    let (mut lua, host) = setup();
    call(&mut lua, "ExitWidgetGroup", vec![s("CANCEL")]);
    assert_eq!(host.state.borrow().screen.widget_group_exit, mw_formats::hsm::key("CANCEL"));
    // A `{binding}` argument is resolved first.
    publish(&host, "Test", r#"{"Next": "ACCEPT"}"#);
    call(&mut lua, "ExitWidgetGroup", vec![s("{Test.Next}")]);
    assert_eq!(host.state.borrow().screen.widget_group_exit, mw_formats::hsm::key("ACCEPT"));
}

#[test]
fn script_variables() {
    let (mut lua, host) = setup();
    let h = host.state.borrow_mut().widget_scripts.create("W", "Script");
    let u = Value::LightUserdata(h);
    call(&mut lua, "SetBoolVariable", vec![u.clone(), s("flag"), Value::Bool(true)]);
    call(&mut lua, "SetIntVariable", vec![u.clone(), s("count"), n(3.9)]);
    call(&mut lua, "SetDoubleVariable", vec![u.clone(), s("ratio"), n(0.5)]);
    call(&mut lua, "SetStringVariable", vec![u.clone(), s("text"), s("abc"), s("Group")]);
    call(&mut lua, "SetUniqueIdVariable", vec![u.clone(), s("id"), n(42.0)]);
    {
        let st = host.state.borrow();
        assert_eq!(st.view_models.resolve_bool("Widget.W.flag"), Some(true));
        assert_eq!(st.view_models.resolve_int("Widget.W.count"), Some(3));
        assert_eq!(st.view_models.resolve_double("Widget.W.ratio"), Some(0.5));
        assert_eq!(st.view_models.resolve_string("Widget.W.Group.text").as_deref(), Some("abc"));
        assert_eq!(st.view_models.resolve_int("Widget.W.id"), Some(42));
    }
    call(&mut lua, "DeleteVariable", vec![u, s("flag")]);
    assert_eq!(host.state.borrow().view_models.resolve_bool("Widget.W.flag"), Some(false));
    // No script object: nothing happens.
    call(&mut lua, "SetBoolVariable", vec![Value::Nil, s("x"), Value::Bool(true)]);
}

#[test]
fn rpc_modal_platform_print() {
    let (mut lua, host) = setup();
    call(&mut lua, "DoRpc", vec![s("W"), s("F"), s("a,b")]);
    assert_eq!(
        host.state.borrow().requests,
        [Request::WidgetRpc { widget: "W".into(), function: "F".into(), args: "a,b".into() }]
    );
    call(&mut lua, "SetModalMode", vec![Value::Bool(true)]);
    assert!(host.state.borrow().screen.modal_mode);
    assert_eq!(text(&call(&mut lua, "GetPlatformName", vec![])).as_deref(), Some("PC"));
    assert!(call(&mut lua, "print", vec![s("x")]).is_empty());
}

#[test]
fn smooth_step_and_language() {
    let (mut lua, host) = setup();
    assert_eq!(num(&call(&mut lua, "SmoothStep", vec![n(0.0), n(10.0), n(5.0)])), 0.5);
    assert_eq!(num(&call(&mut lua, "SmoothStep", vec![n(0.0), n(10.0), n(-5.0)])), 0.0);
    assert_eq!(num(&call(&mut lua, "SmoothStep", vec![n(0.0), n(10.0), n(50.0)])), 1.0);
    call(&mut lua, "SetLanguage", vec![n(2.0)]);
    assert_eq!(host.state.borrow().screen.language, 5);
    assert_eq!(host.state.borrow().requests, [Request::SetLanguage { code: 5 }]);
    assert_eq!(num(&call(&mut lua, "GetInitialLanguageSelectLanguage", vec![])), 2.0);
    // Out of range: language 1 (the first).
    call(&mut lua, "SetLanguage", vec![n(99.0)]);
    assert_eq!(host.state.borrow().screen.language, 1);
}

#[test]
fn conductors_and_sequences() {
    let (mut lua, host) = setup();
    assert!(flag(&call(&mut lua, "IsConductorIdle", vec![s("C")])));
    call(&mut lua, "CreateConductor", vec![s("C")]);
    call(&mut lua, "StartSequence", vec![s("Seq"), s("C")]);
    assert!(!flag(&call(&mut lua, "IsConductorIdle", vec![s("C")])));
    assert!(!flag(&call(&mut lua, "HasSequenceFinished", vec![s("Seq"), s("C")])));
    call(&mut lua, "StopSequence", vec![s("C")]);
    assert!(flag(&call(&mut lua, "HasSequenceFinished", vec![s("Seq"), s("C")])));
    call(&mut lua, "DestroyConductor", vec![s("C")]);
    let st = host.state.borrow();
    assert!(st.screen.conductors.is_empty());
    assert_eq!(st.requests.len(), 4);
    assert_eq!(st.requests[1], Request::StartSequence { conductor: "C".into(), sequence: "Seq".into() });
}

#[test]
fn dialogs() {
    let (mut lua, host) = setup();
    call(&mut lua, "RequestDialog", vec![n(7.0)]);
    call(&mut lua, "RequestDialog", vec![n(8.0)]);
    assert_eq!(num(&call(&mut lua, "GetDialogResult", vec![n(7.0)])), -1.0);
    host.state.borrow_mut().screen.dialogs.set_result(7, 2);
    assert_eq!(num(&call(&mut lua, "GetDialogResult", vec![n(7.0)])), 2.0);
    // Collected once.
    assert_eq!(num(&call(&mut lua, "GetDialogResult", vec![n(7.0)])), -1.0);
    call(&mut lua, "ClearDialog", vec![n(7.0)]);
    call(&mut lua, "ClearDialog", vec![n(8.0)]);
    call(&mut lua, "ClearDialog", vec![n(8.0)]);
    let st = host.state.borrow();
    assert!(st.requests.contains(&Request::CloseDialog { id: 7, last: false }));
    assert!(st.requests.contains(&Request::CloseDialog { id: 8, last: true }));
    assert_eq!(st.requests.iter().filter(|r| matches!(r, Request::CloseDialog { .. })).count(), 2);
}

#[test]
fn text_entry() {
    let (mut lua, host) = setup();
    call(&mut lua, "StartTextEntry", vec![s("T"), s("D"), s("init"), n(16.0), Value::Bool(false)]);
    assert_eq!(
        host.state.borrow().requests,
        [Request::StartTextEntry {
            title: "T".into(),
            description: "D".into(),
            text: "init".into(),
            max_len: 16,
            flags: 0
        }]
    );
    {
        let mut st = host.state.borrow_mut();
        st.text_entry.state = 2;
        st.text_entry.entered = "typed".into();
    }
    assert_eq!(num(&call(&mut lua, "GetTextEntryState", vec![])), 2.0);
    assert_eq!(text(&call(&mut lua, "GetEnteredText", vec![])).as_deref(), Some("typed"));
}

fn with_settings(host: &LogicHost) {
    let v =
        ValueSetting { value: 1, saved: 1, min: 0, max: 4, names: vec!["Low".into(), "High".into()], changed: false };
    host.state.borrow_mut().settings.insert(SettingsCategory { id: 3, settings: vec![Box::new(v)] });
}

#[test]
fn settings_values() {
    let (mut lua, host) = setup();
    with_settings(&host);
    let ci = || vec![n(3.0), n(0.0)];
    assert_eq!(num(&call(&mut lua, "GetSettingsValue", ci())), 1.0);
    assert_eq!(num(&call(&mut lua, "GetSettingsMinValue", ci())), 0.0);
    assert_eq!(num(&call(&mut lua, "GetSettingsMaxValue", ci())), 4.0);
    assert_eq!(num(&call(&mut lua, "GetNormalisedSettingsValue", ci())), 0.25);
    assert_eq!(text(&call(&mut lua, "GetSettingsValueName", ci())).as_deref(), Some("High"));
    assert!(flag(&call(&mut lua, "IsSettingValid", ci())));
    assert!(!flag(&call(&mut lua, "IsSettingValid", vec![n(3.0), n(5.0)])));
    assert_eq!(num(&call(&mut lua, "GetSettingsValue", vec![n(9.0), n(0.0)])), 0.0);
    call(&mut lua, "SetSettingsValue", vec![n(3.0), n(0.0), n(3.0)]);
    assert_eq!(num(&call(&mut lua, "GetSettingsValue", ci())), 3.0);
}

#[test]
fn settings_revert_save_action() {
    let (mut lua, host) = setup();
    with_settings(&host);
    let ci = || vec![n(3.0), n(0.0)];
    call(&mut lua, "SetSettingsValue", vec![n(3.0), n(0.0), n(4.0)]);
    call(&mut lua, "RevertSetting", ci());
    assert_eq!(num(&call(&mut lua, "GetSettingsValue", ci())), 1.0);
    call(&mut lua, "SetSettingsValue", vec![n(3.0), n(0.0), n(4.0)]);
    call(&mut lua, "RevertCategorySettings", vec![n(3.0)]);
    assert_eq!(num(&call(&mut lua, "GetSettingsValue", ci())), 1.0);
    call(&mut lua, "SetSettingsValue", vec![n(3.0), n(0.0), n(4.0)]);
    call(&mut lua, "RevertSettings", vec![]);
    assert_eq!(num(&call(&mut lua, "GetSettingsValue", ci())), 1.0);
    call(&mut lua, "SetSettingsValue", vec![n(3.0), n(0.0), n(4.0)]);
    call(&mut lua, "SaveSettings", vec![]);
    call(&mut lua, "RevertSettings", vec![]);
    assert_eq!(num(&call(&mut lua, "GetSettingsValue", ci())), 4.0);
    assert_eq!(host.state.borrow().requests, [Request::SaveSettings]);
    // A plain value setting has no action; the hooks must not fail.
    call(&mut lua, "ActionSetting", ci());
    call(&mut lua, "OnActionSettingSuccess", ci());
    call(&mut lua, "OnActionSettingFailure", ci());
    assert_eq!(host.state.borrow().requests.len(), 1);
}

#[test]
fn post_fx_settings() {
    let (mut lua, host) = setup();
    call(&mut lua, "RegisterPostFXSetting", vec![n(1.0), n(0.0), n(10.0), n(5.0), n(7.0)]);
    call(&mut lua, "ChangePostFXSetting", vec![n(7.0), n(3.0)]);
    assert_eq!(host.state.borrow().post_fx.settings[0].value, 8);
    call(&mut lua, "ChangePostFXSetting", vec![n(7.0), n(5.0)]);
    assert_eq!(host.state.borrow().post_fx.settings[0].value, 8, "above max: unchanged");
    call(&mut lua, "ChangePostFXSetting", vec![n(7.0), n(-9.0)]);
    assert_eq!(host.state.borrow().post_fx.settings[0].value, 8, "below min: unchanged");
    call(&mut lua, "ResetPostFXSettings", vec![]);
    assert!(host.state.borrow().post_fx.settings.is_empty());
}

#[test]
fn career_fields() {
    let (mut lua, host) = setup();
    call(&mut lua, "SetOfflineEventToPlay", vec![n(12.0)]);
    {
        let st = host.state.borrow();
        let g = &st.game;
        assert_eq!((g.offline_event_to_play, g.offline_event_234, g.offline_event_238), (12, -1, false));
    }
    call(&mut lua, "SetTargetOfflineEvent", vec![n(4.0)]);
    assert_eq!(host.state.borrow().ui_manager.target_offline_event, 4);
    call(&mut lua, "SetStoryMode", vec![Value::Bool(true)]);
    assert!(host.state.borrow().game_modes.story_mode);
    host.state.borrow_mut().challenges.challenge_vehicle_set = 9;
    assert_eq!(num(&call(&mut lua, "GetChallengeVehicleSet", vec![])), 9.0);
    call(&mut lua, "SelectDrivingTestListGroup", vec![n(2.0)]);
    assert_eq!(host.state.borrow().challenges.driving_test_list_group, 2);
    call(&mut lua, "SetRequestedEasyGuideContent", vec![n(6.0)]);
    assert_eq!(host.state.borrow().ui_manager.requested_easy_guide_content, 6);
    host.state.borrow_mut().ui_manager.viewing_easy_drive_pages = true;
    call(&mut lua, "StopViewingEasyDrivePages", vec![]);
    {
        let st = host.state.borrow();
        assert!(!st.ui_manager.viewing_easy_drive_pages && st.ui_manager.requested_easy_guide_content == 0);
    }
    assert_eq!(num(&call(&mut lua, "GetCurrentSafeHouse", vec![])), 0.0);
    for f in ["SetSelectedVehicleId", "SetSafeHouse", "SetTargetRouteFromSafehouse"] {
        assert!(call(&mut lua, f, vec![n(1.0)]).is_empty());
    }
    assert!(call(&mut lua, "ChangePerkOnCustomRollout", vec![n(1.0), n(2.0)]).is_empty());
    assert!(host.state.borrow().unported.is_empty());
}
