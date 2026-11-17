//! The game's shipped UI chunks (`TextFile` resources in `UI/SCREENS2`). Skipped without an install.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use mw_assets::AssetStore;
use mw_bundle::{Bundle, ResourceId, ResourceType};
use mw_gamedir::GameDir;
use mw_lua::{Lua, Value};

fn chunks() -> Option<BTreeMap<String, (ResourceId, Vec<u8>)>> {
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
                let name = mw_lua::chunk_script_name(c).expect("script name");
                out.insert(name, (e.id, c.to_vec()));
            }
        }
    }
    Some(out)
}

type Log = Rc<RefCell<Vec<String>>>;

/// A state with a logging stub for every global the chunk reads that neither it nor the libraries define.
fn state(chunk: &[u8], result: impl Fn(&str, &[Value]) -> Vec<Value> + 'static) -> (Lua, Log) {
    let mut lua = Lua::new();
    let log: Log = Rc::default();
    let result = Rc::new(result);
    let (get, set) = mw_lua::chunk_globals(chunk).unwrap();
    for g in get.iter().filter(|g| !set.contains(g)) {
        if !lua.global(g).is_nil() {
            continue;
        }
        let (l, r) = (log.clone(), result.clone());
        lua.register(g, move |_, name, args| {
            let a: Vec<String> = args.iter().map(|v| v.display()).collect();
            l.borrow_mut().push(format!("{name}({})", a.join(", ")));
            Ok(r(name, &args))
        });
    }
    (lua, log)
}

#[test]
fn every_chunk_loads_and_constructs() {
    let Some(all) = chunks() else { return };
    assert_eq!(all.len(), 115, "unique Lua chunks in UI/SCREENS2");
    let mut failures = Vec::new();
    for (name, (id, bytes)) in &all {
        assert_eq!(ResourceId::from_name(&format!("{name}.lua")), *id, "{name}: id = CRC-32 of the name");
        let (mut lua, _) = state(bytes, |name, _| match name {
            "ResolveIntBinding" | "ResolveDoubleBinding" => vec![Value::Number(0.0)],
            "ResolveBoolBinding" => vec![Value::Bool(false)],
            "ResolveStringBinding" | "ResolveWideStringBinding" => vec![Value::from("")],
            _ => vec![],
        });
        let main = Value::Function(lua.load(bytes).unwrap());
        if let Err(e) = lua.call(&main, vec![]) {
            failures.push(format!("{name} main: {e}"));
            continue;
        }
        assert!(lua.global(name).as_table().is_some(), "{name}: the script table");
        if let Err(e) = lua.call_field(name, "Construct", vec![Value::LightUserdata(0x1000)]) {
            failures.push(format!("{name}.Construct: {e}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// ViewLegal (the legal / terms screen): OnAction resolves the action binding and runs the matching closure.
#[test]
fn view_legal_actions() {
    let Some(all) = chunks() else { return };
    let (_, bytes) = &all["ViewLegal"];
    for (action, exit) in [("ViewTOS", "\"TOS\""), ("ViewPP", "\"PP\"")] {
        let action = action.to_string();
        let (mut lua, log) =
            state(
                bytes,
                move |name, _| {
                    if name == "ResolveStringBinding" { vec![Value::from(action.as_str())] } else { vec![] }
                },
            );
        let main = Value::Function(lua.load(bytes).unwrap());
        lua.call(&main, vec![]).unwrap();
        let r = lua.call_field("ViewLegal", "Construct", vec![Value::LightUserdata(0x1000)]).unwrap().unwrap();
        assert!(matches!(r[..], [Value::Bool(true)]), "Construct returns true");
        let r = lua.call_field("ViewLegal", "OnAction", vec![Value::from("Widget.Action")]).unwrap().unwrap();
        assert!(matches!(r[..], [Value::Bool(true)]));
        assert_eq!(
            *log.borrow(),
            vec!["ResolveStringBinding(\"Widget.Action\")".to_string(), format!("ExitWidgetGroup({exit})")]
        );
    }
    // A missing callback is skipped, like the game's isfunction check.
    let (mut lua, _) = state(bytes, |_, _| vec![]);
    let main = Value::Function(lua.load(bytes).unwrap());
    lua.call(&main, vec![]).unwrap();
    assert!(lua.call_field("ViewLegal", "NoSuchCallback", vec![]).unwrap().is_none());
}

/// Errors name the failing function and the native the script expected (the game would drop the message).
#[test]
fn errors_name_the_function() {
    let Some(all) = chunks() else { return };
    let (_, bytes) = &all["ViewLegal"];
    let mut lua = Lua::new();
    let main = Value::Function(lua.load(bytes).unwrap());
    lua.call(&main, vec![]).unwrap();
    let e = lua.call_field("ViewLegal", "OnAction", vec![Value::from("x")]).unwrap_err();
    assert_eq!(e.message(), "?:0: attempt to call global 'ResolveStringBinding' (a nil value)");
    assert_eq!(e.traceback, vec!["ViewLegal.OnAction".to_string()]);
}
