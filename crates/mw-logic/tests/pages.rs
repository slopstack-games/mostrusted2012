//! Page contexts and widget groups (`natives::pages`) on the shipped boot screen 491012: pushing the boot page and
//! activating "Press Start" (491032) runs the `Background` widget's `PlaySequence` script, which plays its
//! sequence `Widget.Background.Sequence` = the title sequence 1022357 on the conductor "PressStart"; deactivating
//! stops it. Skipped without an install.

use mw_assets::AssetStore;
use mw_gamedir::GameDir;
use mw_logic::hsm::Host;
use mw_logic::hsm::behaviours::Query;
use mw_logic::natives::{LogicHost, Request, pages};

#[test]
fn boot_page_press_start() {
    let game = match GameDir::locate(None) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipped: no game install ({e})");
            return;
        }
    };
    let store = AssetStore::new(game).expect("index the install");
    let mut host = LogicHost::default();
    host.request(Request::PushPageContext { name: "boot".into(), page: "491012".into() });
    assert_eq!(host.query(&Query::PageContextExists { name: "boot".into() }), Some(1));
    assert_eq!(host.query(&Query::PageReady), Some(0), "not ready before the screen is loaded");
    assert_eq!(host.pages.wanted, [491012]);
    host.pages.provide(pages::load_screen(&store, 491012).expect("boot screen"));
    assert_eq!(host.query(&Query::PageReady), Some(1));

    host.request(Request::ActivateWidgetGroup { group: 491032 });
    assert!(host.pages.errors.is_empty(), "{:?}", host.pages.errors);
    let reqs = std::mem::take(&mut host.state.borrow_mut().requests);
    let started = reqs.iter().find_map(|r| match r {
        Request::StartSequence { conductor, sequence } => Some((conductor.clone(), sequence.clone())),
        _ => None,
    });
    let (conductor, sequence) = started.expect("PlaySequence starts its sequence");
    assert_eq!(conductor, "PressStart");
    assert_eq!(host.state.borrow().resolve_int(&sequence), Some(1_022_357));

    host.request(Request::DeactivateWidgetGroup { group: 491032, instant: false });
    let reqs = std::mem::take(&mut host.state.borrow_mut().requests);
    assert!(reqs.contains(&Request::StopSequence { conductor: "PressStart".into() }), "{reqs:?}");
    assert!(host.pages.active_groups().is_empty());
}

/// Widget input on the boot screen's calibration group 1532543: `accept` (52) fires the native
/// `ExitGroupVia(NEXT)`, `cancel` (53) `ExitGroupVia(BACK)`; `menu_right` (43, repeat flag) reaches the Lua
/// `VisualSetup.IncrementSetting`; nothing fires without input.
#[test]
fn calibration_actions() {
    use mw_formats::hsm::key;
    use mw_logic::natives::pages::ActionInput;
    let Ok(game) = GameDir::locate(None) else {
        eprintln!("skipped: no game install");
        return;
    };
    let store = AssetStore::new(game).expect("index the install");
    let mut host = LogicHost::default();
    host.request(Request::PushPageContext { name: "boot".into(), page: "491012".into() });
    host.pages.provide(pages::load_screen(&store, 491012).expect("boot screen"));
    host.request(Request::ActivateWidgetGroup { group: 1_532_543 });
    assert!(host.pages.errors.is_empty(), "{:?}", host.pages.errors);
    let press =
        |id: u8| move |a: u8| ActionInput { pressed: a == id, repeat: false, value: if a == id { 1.0 } else { 0.0 } };
    host.input(&|_| ActionInput::default());
    assert_eq!(host.state.borrow().screen.widget_group_exit, 0);
    host.input(&press(43));
    assert!(host.pages.errors.is_empty(), "{:?}", host.pages.errors);
    host.input(&press(52));
    assert_eq!(host.state.borrow().screen.widget_group_exit, key("NEXT"));
    host.request(Request::ClearWidgetExit);
    host.input(&press(53));
    assert_eq!(host.state.borrow().screen.widget_group_exit, key("BACK"));
}
