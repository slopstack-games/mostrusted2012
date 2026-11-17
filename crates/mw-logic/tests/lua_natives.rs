//! The ported Lua natives driven by the game's own UI chunks (`UI/SCREENS2`). Skipped without an install.

use std::collections::BTreeMap;

use mw_assets::AssetStore;
use mw_bundle::{Bundle, ResourceType};
use mw_gamedir::GameDir;
use mw_logic::natives::LogicHost;
use mw_lua::{Lua, Value};

fn chunks() -> Option<BTreeMap<String, Vec<u8>>> {
    let game = match GameDir::locate(None) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipped: no game install ({e})");
            return None;
        }
    };
    let store = AssetStore::new(game).expect("index the install");
    let mut out = BTreeMap::new();
    for rel in store.bundle_paths() {
        if !rel.to_string_lossy().replace('\\', "/").to_ascii_uppercase().contains("UI/SCREENS2/") {
            continue;
        }
        let b = Bundle::from_bytes(store.game().read(rel).unwrap()).unwrap();
        for e in b.entries.iter().filter(|e| e.resource_type() == ResourceType::TextFile) {
            let r = b.load(e).unwrap();
            if let Some(c) = mw_lua::textfile_chunk(r.header()) {
                out.insert(mw_lua::chunk_script_name(c).unwrap(), c.to_vec());
            }
        }
    }
    Some(out)
}

/// A script's chunk run in a Lua state with the registry's natives installed through [`LogicHost`].
fn host_with(chunk: &[u8]) -> (Lua, LogicHost) {
    let mut lua = Lua::new();
    let host = LogicHost::default();
    host.install_lua(&mut lua);
    let main = Value::Function(lua.load(chunk).unwrap());
    lua.call(&main, vec![]).unwrap();
    (lua, host)
}

#[test]
fn view_legal_callbacks_exit_the_widget_group() {
    let Some(all) = chunks() else { return };
    let (mut lua, host) = host_with(&all["ViewLegal"]);
    let h = host.state.borrow_mut().widget_scripts.create("ViewLegal", "ViewLegal");
    lua.call_field("ViewLegal", "Construct", vec![Value::LightUserdata(h)]).unwrap();
    // OnCancel: ExitWidgetGroup("CANCEL").
    assert!(lua.call_field("ViewLegal", "OnCancel", vec![]).unwrap().is_some());
    {
        let st = host.state.borrow();
        assert_eq!(st.screen.widget_group_exit, mw_formats::hsm::key("CANCEL"));
        assert!(st.unported.iter().all(|u| u == "lua SignInFinished"), "{:?}", st.unported);
    }
    // OnAction: ResolveStringBinding(nil) reads "<invalid>", which resolves to nothing; the script then calls
    // a chunk-level local that another callback sets (decomp.md: errors without that context are expected), so
    // the error is the script's, not a native's; the binding read happened and the exit code is unchanged.
    let _ = lua.call_field("ViewLegal", "OnAction", vec![]);
    let st = host.state.borrow();
    assert_eq!(st.screen.widget_group_exit, mw_formats::hsm::key("CANCEL"));
    assert!(st.unported.iter().all(|u| u == "lua SignInFinished"));
}

#[test]
fn breadcrumb_keeps_its_variables() {
    let Some(all) = chunks() else { return };
    let (mut lua, host) = host_with(&all["Breadcrumb"]);
    let handle = host.state.borrow_mut().widget_scripts.create("Breadcrumbs", "Breadcrumb");
    lua.call_field("Breadcrumb", "Construct", vec![Value::LightUserdata(handle)]).unwrap();
    for (f, a) in [("OnActivated", vec![]), ("PushBreadcrumb", vec![Value::from("A"), Value::from("B")])] {
        lua.call_field("Breadcrumb", f, a).unwrap();
    }
    let st = host.state.borrow();
    let vm = &st.widget_scripts.get(handle).unwrap().view_model;
    assert!(!vm.entries().is_empty(), "the script set view-model variables: {vm:?}");
    assert!(st.view_models.published("Widget").is_some(), "republished as the Widget root");
    assert!(st.unported.is_empty(), "{:?}", st.unported);
}
