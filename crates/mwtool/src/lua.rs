//! `mwtool lua`: the UI's Lua scripts run by `mw_lua` on the logic host (`mw_logic::natives::LogicHost`).
//!
//! Every precompiled chunk (`TextFile` resources in `UI/SCREENS2`) is loaded, its main function run and its
//! script table's `Construct` called with a light userdata, as the game's `FUN_007ec680` does; `--call`
//! adds callbacks. The natives a script reads (`GETGLOBAL` names it does not define) are the original's Lua
//! registry (`mw_logic::natives::table`; `--natives <file>` replaces the list, one name per line): a ported one
//! runs on the logic host's shared state, an unported one is a stub; both log their name and arguments. A name
//! the registry lacks stays nil (or the library function), as in the game; a listed name replaces a library
//! function (`print`).
//!
//! Stub results are a model, not the game's: `Resolve{Int,Double}Binding` and names with `Count` / `Index` /
//! `Num` give 0, `ResolveBoolBinding` and `Is` / `Has` / `Can` / `Should` / `Did` names false, string
//! resolvers "", everything else nothing (nil).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use anyhow::{Context, Result};
use mw_assets::AssetStore;
use mw_bundle::{Bundle, ResourceId, ResourceType};
use mw_logic::natives::LogicHost;
use mw_lua::{Lua, Value};

/// One unique chunk: id, script name, the chunk bytes, and the bundles it is in.
pub struct Chunk {
    pub id: ResourceId,
    pub name: String,
    pub bytes: Vec<u8>,
    pub copies: usize,
}

/// Every Lua chunk in `UI/SCREENS2`, unique by id, sorted by name.
pub fn chunks(store: &AssetStore) -> Result<Vec<Chunk>> {
    let mut by_id: BTreeMap<ResourceId, Chunk> = BTreeMap::new();
    for rel in store.bundle_paths() {
        let p = rel.to_string_lossy().replace('\\', "/").to_ascii_uppercase();
        if !p.contains("UI/SCREENS2/") {
            continue;
        }
        let data = store.game().read(rel).with_context(|| format!("read {}", rel.display()))?;
        let b = Bundle::from_bytes(data)?;
        for e in b.entries.iter().filter(|e| e.resource_type() == ResourceType::TextFile) {
            if let Some(c) = by_id.get_mut(&e.id) {
                c.copies += 1;
                continue;
            }
            let r = b.load(e)?;
            let Some(chunk) = mw_lua::textfile_chunk(r.header()) else { continue };
            let name = mw_lua::chunk_script_name(chunk).unwrap_or_else(|| format!("{:016x}", e.id.0));
            by_id.insert(e.id, Chunk { id: e.id, name, bytes: chunk.to_vec(), copies: 1 });
        }
    }
    let mut v: Vec<Chunk> = by_id.into_values().collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(v)
}

fn stub_result(name: &str) -> Vec<Value> {
    let v = match name {
        "ResolveIntBinding" | "ResolveDoubleBinding" => Value::Number(0.0),
        "ResolveBoolBinding" => Value::Bool(false),
        "ResolveStringBinding" | "ResolveWideStringBinding" => Value::from(""),
        n if ["Is", "Has", "Can", "Should", "Did"].iter().any(|p| n.starts_with(p)) => Value::Bool(false),
        n if ["Count", "Index", "Num"].iter().any(|p| n.contains(p)) => Value::Number(0.0),
        _ => return vec![],
    };
    vec![v]
}

/// What running one script did.
pub struct Report {
    pub name: String,
    /// Native name -> (calls, arguments of the first call).
    pub natives: BTreeMap<String, (usize, String)>,
    /// Every native call in order (name, arguments), and base-library `print` lines as `(base print)`.
    pub trace: Vec<(String, String)>,
    /// Errors, each with the step that raised it (`main`, `Construct`, a callback name).
    pub errors: Vec<String>,
    /// Natives the chunk reads but that had no stub (not in `--natives`).
    pub missing: Vec<String>,
    /// Natives the chunk reads that got a stub or the ported function (called or not).
    pub stubbed: Vec<String>,
    /// Of those, the ported ones.
    pub ported: Vec<String>,
}

/// Run a chunk's main function, `Construct`, then `calls` (`all` = every function of the script table), on a
/// fresh logic host (one Lua state per widget in the game).
pub fn run(chunk: &Chunk, natives: Option<&HashSet<String>>, calls: &[String]) -> Result<Report> {
    let host = LogicHost::default();
    let mut lua = Lua::new();
    let log: Rc<RefCell<Vec<(String, String)>>> = Rc::default();
    let (get, set) = mw_lua::chunk_globals(&chunk.bytes).map_err(lerr)?;
    let mut missing = Vec::new();
    let mut stubbed = Vec::new();
    let mut ported = Vec::new();
    for g in &get {
        // The game registers its natives after opening the libraries, so a listed native replaces a library
        // function of the same name (`print` is one: a native that reads its argument and does nothing).
        let listed = natives.map_or_else(|| host.registry.lua(g).is_some(), |n| n.contains(g));
        if set.contains(g) || (!listed && !lua.global(g).is_nil()) {
            continue;
        }
        if !listed {
            missing.push(g.clone());
            continue;
        }
        stubbed.push(g.clone());
        let l = log.clone();
        let real = host.registry.lua(g).filter(|n| n.ported()).and_then(|_| host.lua_native(g));
        if real.is_some() {
            ported.push(g.clone());
        }
        lua.register(g, move |lua, name, args| {
            let a: Vec<String> = args.iter().map(|v| v.display()).collect();
            l.borrow_mut().push((name.to_string(), a.join(", ")));
            match &real {
                Some(f) => f(lua, name, args),
                None => Ok(stub_result(name)),
            }
        });
    }
    let l = log.clone();
    lua.set_print(move |s| l.borrow_mut().push(("(base print)".into(), s.to_string())));

    let mut errors = Vec::new();
    let main = Value::Function(lua.load(&chunk.bytes).map_err(lerr)?);
    if let Err(e) = lua.call(&main, vec![]) {
        errors.push(format!("main: {e}"));
    }
    // The game passes the script object's address (lua_pushlightuserdata).
    let this = Value::LightUserdata(0x1000_0000);
    match lua.call_field(&chunk.name, "Construct", vec![this.clone()]) {
        Ok(_) => {}
        Err(e) => errors.push(format!("Construct: {e}")),
    }
    let mut targets: Vec<String> = Vec::new();
    for c in calls {
        if c == "all" {
            if let Some(t) = lua.global(&chunk.name).as_table() {
                for (k, v) in t.pairs() {
                    if let (Some(k), Value::Function(_)) = (k.as_str_lossy(), &v)
                        && k != "Construct"
                    {
                        targets.push(k);
                    }
                }
            }
        } else {
            targets.push(c.clone());
        }
    }
    for f in &targets {
        if let Err(e) = lua.call_field(&chunk.name, f, vec![]) {
            errors.push(format!("{f}: {e}"));
        }
    }

    let trace = log.borrow().clone();
    let mut natives_called: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for (n, a) in trace.iter().filter(|(n, _)| n != "(base print)") {
        natives_called.entry(n.clone()).or_insert((0, a.clone())).0 += 1;
    }
    Ok(Report { name: chunk.name.clone(), natives: natives_called, trace, errors, missing, stubbed, ported })
}

fn read_natives(path: &Path) -> Result<HashSet<String>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    Ok(text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('#')).collect())
}

pub fn cmd(
    store: &AssetStore,
    list: bool,
    run_one: Option<&str>,
    calls: &[String],
    trace: bool,
    natives: Option<&Path>,
    out: Option<&Path>,
) -> Result<()> {
    let all = chunks(store)?;
    if list {
        let mut bad = 0;
        for c in &all {
            let crc_ok = ResourceId::from_name(&format!("{}.lua", c.name)) == c.id;
            bad += usize::from(!crc_ok);
            let (get, _) = mw_lua::chunk_globals(&c.bytes).map_err(lerr)?;
            let ops: usize = mw_lua::chunk_opcodes(&c.bytes).map_err(lerr)?.iter().sum();
            println!(
                "{:016x}  {:<18} {:6} bytes {:5} instr {:3} globals read  x{}{}",
                c.id.0,
                c.name,
                c.bytes.len(),
                ops,
                get.len(),
                c.copies,
                if crc_ok { "" } else { "  (name does not hash to the id)" }
            );
        }
        println!("{} chunks, {} names not matching their id", all.len(), bad);
        return Ok(());
    }
    let natives = natives.map(read_natives).transpose()?;
    let selected: Vec<&Chunk> = match run_one {
        Some(n) => {
            let found: Vec<&Chunk> = all
                .iter()
                .filter(|c| c.name.eq_ignore_ascii_case(n) || format!("{:016x}", c.id.0) == n.to_lowercase())
                .collect();
            anyhow::ensure!(!found.is_empty(), "no Lua chunk named {n}");
            found
        }
        None => all.iter().collect(),
    };
    // One row per native a script reads: calls in this run (0 = only on paths not run), or "nil" when
    // the name is not in the registry given with --natives.
    let mut tsv = String::from("script\tnative\tcalls\tfirst_args\n");
    let mut union: BTreeMap<String, usize> = BTreeMap::new();
    let mut failed = 0;
    for c in &selected {
        let r = run(c, natives.as_ref(), calls)?;
        if trace || run_one.is_some() {
            println!("== {} ({:016x})", r.name, c.id.0);
            if trace {
                for (n, a) in &r.trace {
                    println!("  {n}({a})");
                }
            }
            for (n, (count, a)) in &r.natives {
                println!("  native {n} x{count}  first: ({a})");
            }
            if !r.missing.is_empty() {
                println!("  not registered (nil): {}", r.missing.join(", "));
            }
            if !r.ported.is_empty() {
                println!("  ported natives: {}", r.ported.join(", "));
            }
            for e in &r.errors {
                println!("  ERROR {e}");
            }
        } else {
            let names: Vec<String> =
                r.natives.iter().map(|(n, (k, _))| if *k > 1 { format!("{n}x{k}") } else { n.clone() }).collect();
            println!(
                "{:<18} {}  {}",
                r.name,
                if r.errors.is_empty() { "ok " } else { "ERR" },
                if names.is_empty() { "-".into() } else { names.join(" ") }
            );
            for e in &r.errors {
                println!("    {e}");
            }
        }
        failed += usize::from(!r.errors.is_empty());
        for n in &r.stubbed {
            let (k, a) = r.natives.get(n).cloned().unwrap_or_default();
            if k > 0 {
                *union.entry(n.clone()).or_default() += 1;
            }
            tsv.push_str(&format!("{}\t{n}\t{k}\t{}\n", r.name, a.replace(['\t', '\n'], " ")));
        }
        for n in &r.missing {
            tsv.push_str(&format!("{}\t{n}\tnil\t\n", r.name));
        }
    }
    println!("{} scripts run, {} with errors; {} distinct natives called", selected.len(), failed, union.len());
    if let Some(out) = out {
        std::fs::write(out, tsv).with_context(|| format!("write {}", out.display()))?;
        println!("wrote {}", out.display());
    }
    Ok(())
}

/// `LuaError` holds Lua values (not `Send`); keep its text.
fn lerr(e: mw_lua::LuaError) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}
