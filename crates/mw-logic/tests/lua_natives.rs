//! The ported Lua natives driven by the game's own UI chunks (`UI/SCREENS2`). Skipped without an install.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use mw_assets::AssetStore;
use mw_bundle::{Bundle, ResourceType};
use mw_gamedir::GameDir;
use mw_logic::natives::{LogicState, Registry};
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

type Shared = Rc<RefCell<LogicState>>;

/// A Lua state with every ported native of the registry bound as a global (unported ones are left nil).
fn state_with_natives(chunk: &[u8]) -> (Lua, Shared) {
    let mut lua = Lua::new();
    let state: Shared = Rc::default();
    let registry = Registry::original();
    for n in registry.lua_natives() {
        if let Some(f) = n.f {
            let s = state.clone();
            lua.register(n.name, move |lua, _, args| f(&mut s.borrow_mut(), lua, args));
        }
    }
    let main = Value::Function(lua.load(chunk).unwrap());
    lua.call(&main, vec![]).unwrap();
    (lua, state)
}

fn keys(lua: &Lua, table: &str) -> Vec<String> {
    let mut k: Vec<String> =
        lua.global(table).as_table().unwrap().pairs().iter().map(|(k, _)| k.display()).collect();
    k.sort();
    k
}

#[test]
fn breadcrumb_keeps_its_variables() {
    let Some(all) = chunks() else { return };
    let (mut lua, state) = state_with_natives(&all["Breadcrumb"]);
    eprintln!("{:?}", keys(&lua, "Breadcrumb"));
    for n in ["LanguageSelect", "ModalToggle", "EditSettings", "Alert", "PlaySequence"] { let (l, _) = state_with_natives(&all[n]); eprintln!("{n} {:?}", keys(&l, n)); }
    let handle = state.borrow_mut().widget_scripts.create("Breadcrumbs", "Breadcrumb");
    lua.call_field("Breadcrumb", "Construct", vec![Value::LightUserdata(handle)]).unwrap();
    for (f, a) in [("OnActivated", vec![]), ("PushBreadcrumb", vec![Value::from("A"), Value::from("B")]), ("PushBreadcrumb", vec![Value::from("C")]), ("PopBreadcrumbs", vec![Value::Number(1.0)])] {
        let r = lua.call_field("Breadcrumb", f, a);
        eprintln!("{f} {r:?} {:?}", state.borrow().widget_scripts.get(handle).unwrap().view_model);
    }
}
