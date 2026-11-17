//! `mwtool widgets`: the screens' JSON widget definitions and their scripts (`mw_assets::widgets`).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result, bail};
use mw_assets::AssetStore;
use mw_assets::widgets::{ScreenGraph, ScriptTarget};
use mw_bundle::{ResourceId, ResourceType};
use mw_formats::widget::{NATIVE_SCREEN_SCRIPTS, WidgetDef};

pub struct Options<'a> {
    pub screen: Option<&'a str>,
    pub json: Option<&'a str>,
    pub scripts: bool,
}

pub fn run(store: &AssetStore, o: &Options) -> Result<()> {
    if let Some(j) = o.json {
        return print_json(store, j);
    }
    if let Some(s) = o.screen {
        let bundle = screen_bundle(store, s)?;
        let g = store.ui_screen_graph(&bundle)?;
        print_screen(&g);
        return Ok(());
    }
    let graphs: Vec<ScreenGraph> =
        store.ui_screen_bundles().iter().map(|b| store.ui_screen_graph(b)).collect::<mw_assets::Result<_>>()?;
    if o.scripts {
        print_scripts(store, &graphs);
        return Ok(());
    }
    survey(store, &graphs)
}

/// A bundle path, or a screen number (`899991` → `UI/SCREENS2/899991.BNDL`).
fn screen_bundle(store: &AssetStore, s: &str) -> Result<String> {
    if s.contains('/') {
        return Ok(s.to_string());
    }
    let want = format!("{}/{s}.BNDL", mw_assets::widgets::SCREENS_DIR).to_ascii_uppercase();
    store
        .ui_screen_bundles()
        .into_iter()
        .find(|b| b.to_ascii_uppercase() == want)
        .with_context(|| format!("no screen bundle {want}"))
}

fn print_screen(g: &ScreenGraph) {
    println!("{}  screen {} {:?}: {} widgets", g.bundle, g.screen, g.screen_name, g.widgets.len());
    let mut group = None;
    for w in &g.widgets {
        if group != Some(w.group) {
            println!("group {}", w.group);
            group = Some(w.group);
        }
        let (layer, modal, extra) = match &w.def {
            Ok(d) => (
                d.layout_layer.as_ref().map_or("-", |l| l.name()).to_string(),
                d.modal_mode.as_ref().map_or("-", |m| m.name()).to_string(),
                format!(
                    "layouts {} actions {} adapters {} timers {}{}",
                    d.layouts.len(),
                    d.actions.len(),
                    d.adapters.len(),
                    d.timers.len(),
                    if d.unknown_keys.is_empty() { String::new() } else { format!(" untyped {:?}", d.unknown_keys) }
                ),
            ),
            Err(e) => ("?".into(), "?".into(), format!("JSON: {e}")),
        };
        println!("  {:<34} {:<18} {:>8}  {}  {layer} {modal}  {extra}", w.type_name, w.name, w.id, w.json);
        if let Ok(d) = &w.def {
            for a in &d.actions {
                println!(
                    "      action {:>8} {}{}{}{}",
                    a.definition,
                    if a.in_lua { "lua " } else { "native " },
                    a.callback,
                    if a.args.is_empty() { String::new() } else { format!("({})", a.args) },
                    if a.active_binding.is_empty() { String::new() } else { format!("  if {}", a.active_binding) }
                );
            }
        }
        for s in &w.scripts {
            println!("      script {:<20} {}", s.name, target(&s.target));
        }
    }
    if !g.orphan_json.is_empty() {
        println!("JSON without a widget: {:?}", g.orphan_json);
    }
}

fn target(t: &ScriptTarget) -> String {
    match t {
        ScriptTarget::Lua { chunk, found } => format!("lua chunk {chunk}{}", if *found { "" } else { " (MISSING)" }),
        ScriptTarget::Native(n) => format!("native (factory {:08x}, {} bytes)", n.factory, n.size),
        ScriptTarget::Unresolved => "UNRESOLVED".into(),
    }
}

fn print_json(store: &AssetStore, s: &str) -> Result<()> {
    // `<name>_<id>` or a resource id.
    let id = match s.rsplit_once('_').and_then(|(n, i)| Some((n, i.trim_end_matches(".json").parse::<u32>().ok()?))) {
        Some((n, i)) if !s.chars().all(|c| c.is_ascii_hexdigit()) => mw_formats::widget::json_resource_id(n, i),
        _ => ResourceId::parse(s).context("expected <Name>_<UniqueId> or a resource id")?,
    };
    let j = store.ui_widget_json(id)?;
    // The widget type, when a screen names this JSON.
    let mut ty = None;
    for b in store.ui_screen_bundles() {
        if let Ok(g) = store.ui_screen_graph(&b)
            && let Some(w) = g.widgets.iter().find(|w| w.json == id)
        {
            ty = Some(w.type_name.clone());
            break;
        }
    }
    println!("{id} type {}", ty.as_deref().unwrap_or("?"));
    let d = WidgetDef::parse(&j, ty.as_deref())?;
    println!("{d:#?}");
    Ok(())
}

fn print_scripts(store: &AssetStore, graphs: &[ScreenGraph]) {
    // name → (target, uses, screens)
    let mut by_name: BTreeMap<String, (String, usize, BTreeSet<String>, Option<String>)> = BTreeMap::new();
    for g in graphs {
        for (_, s) in g.scripts() {
            let e = by_name.entry(s.name.clone()).or_insert_with(|| (target(&s.target), 0, BTreeSet::new(), None));
            e.1 += 1;
            e.2.insert(g.bundle.trim_start_matches("UI/SCREENS2/").trim_end_matches(".BNDL").to_string());
            if e.3.is_none() {
                e.3 = s.object.as_ref().map(|o| o.type_name.clone());
            }
        }
    }
    for (name, (t, n, screens, obj)) in &by_name {
        let list: Vec<_> = screens.iter().take(6).cloned().collect();
        println!(
            "{name:<20} {t:<44} x{n:<3} object {:<34} screens {}{}",
            obj.as_deref().unwrap_or("-"),
            list.join(" "),
            if screens.len() > 6 { " ..." } else { "" }
        );
    }
    let used: HashSet<&str> = by_name.keys().map(String::as_str).collect();
    let unused: Vec<_> = NATIVE_SCREEN_SCRIPTS.iter().filter(|n| !used.contains(n.name)).map(|n| n.name).collect();
    println!("native screen scripts no shipped widget uses: {unused:?}");
    let _ = store;
}

/// Every screen: definitions parse, untyped keys per widget type, scripts resolve, JSON and GeneSys agree.
fn survey(store: &AssetStore, graphs: &[ScreenGraph]) -> Result<()> {
    let mut defs: HashMap<ResourceId, (&str, &str)> = HashMap::new();
    let mut errors = Vec::new();
    let mut untyped: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
    let mut layer_map: BTreeMap<(String, u64), usize> = BTreeMap::new();
    let mut modal_map: BTreeMap<(String, u64), usize> = BTreeMap::new();
    let (mut lua, mut native, mut unresolved, mut missing_chunk, mut lua_asset_mismatch) = (0, 0, 0, 0, 0);
    let mut chunks_used = HashSet::new();
    let mut orphans = Vec::new();
    for g in graphs {
        println!(
            "{:<28} screen {:>8} {:<20} widgets {:>3} scripts {:>3} orphan json {}",
            g.bundle,
            g.screen,
            g.screen_name,
            g.widgets.len(),
            g.scripts().count(),
            g.orphan_json.len()
        );
        orphans.extend(g.orphan_json.iter().map(|id| (*id, g.bundle.as_str())));
        for w in &g.widgets {
            match &w.def {
                Ok(d) => {
                    defs.insert(w.json, (w.type_name.as_str(), w.name.as_str()));
                    for k in &d.unknown_keys {
                        *untyped.entry(w.type_name.as_str()).or_default().entry(k.as_str()).or_default() += 1;
                    }
                    if let (Some(l), Some(v)) = (&d.layout_layer, w.layout_layer) {
                        *layer_map.entry((l.name().to_string(), v)).or_default() += 1;
                    }
                    if let (Some(m), Some(v)) = (&d.modal_mode, w.modal_mode) {
                        *modal_map.entry((m.name().to_string(), v)).or_default() += 1;
                    }
                    for (i, s) in w.scripts.iter().enumerate() {
                        match &s.target {
                            ScriptTarget::Lua { chunk, found } => {
                                lua += 1;
                                chunks_used.insert(*chunk);
                                if !found {
                                    missing_chunk += 1;
                                    println!("  missing chunk: {} {} {}", w.name, s.name, chunk);
                                }
                            }
                            ScriptTarget::Native(_) => native += 1,
                            ScriptTarget::Unresolved => {
                                unresolved += 1;
                                println!("  unresolved script: {} {}", w.name, s.name);
                            }
                        }
                        let json_asset = d.scripts.get(i).and_then(|s| s.lua_asset);
                        if let Some(obj) = &s.object
                            && obj.lua_asset.filter(|&a| a != 0) != json_asset
                        {
                            lua_asset_mismatch += 1;
                            println!(
                                "  LuaAsset differs: {} {}: object {:?} json {:?}",
                                w.name, s.name, obj.lua_asset, json_asset
                            );
                        }
                    }
                }
                Err(e) => errors.push(format!("{} {} {}: {e}", g.bundle, w.name, w.json)),
            }
        }
    }
    // Every JSON TextFile in the install: named by a screen's widget or not.
    let mut all_json = BTreeMap::new();
    let mut all_lua = HashSet::new();
    for path in store.bundle_paths() {
        let p = path.to_string_lossy().replace('\\', "/");
        let Some(b) = store.bundle(&p) else { continue };
        for e in b.entries_of_type(ResourceType::TextFile) {
            let res = b.load(e)?;
            if res.header().get(4..8) == Some(b"\x1bLua") {
                all_lua.insert(e.id);
            } else {
                all_json.entry(e.id).or_insert(p.clone());
            }
        }
    }
    let unnamed: Vec<_> = all_json.iter().filter(|(id, _)| !defs.contains_key(id)).collect();
    println!();
    println!(
        "{} unique JSON TextFiles in the install; {} named by a screen widget and parsed, {} errors",
        all_json.len(),
        defs.len(),
        errors.len()
    );
    for e in &errors {
        println!("  error: {e}");
    }
    // The others: reached through a sequence's `AdapterHost` item (by handle); they parse as definitions
    // without a widget type (common fields only: Name + Adapters).
    let mut hosts: HashMap<ResourceId, String> = HashMap::new();
    for bundle in unnamed.iter().map(|(_, b)| b.as_str()).collect::<BTreeSet<_>>() {
        for h in store.ui_adapter_hosts(bundle)? {
            hosts.entry(h.json).or_insert(format!("sequence {} item {} {:?}", h.sequence, h.id, h.name));
        }
    }
    let mut unnamed_bad = 0;
    for (id, bundle) in &unnamed {
        let def =
            store.ui_widget_json(**id).ok().and_then(|j| WidgetDef::parse(&j, Some("SequenceItems.AdapterHost")).ok());
        let host = hosts.get(*id);
        match (def, host) {
            (Some(d), Some(h)) if d.unknown_keys.is_empty() => {
                println!("  {id} ({bundle}) \"{}\": adapters {}, from {h}", d.name, d.adapters.len())
            }
            _ => {
                unnamed_bad += 1;
                println!("  {id} ({bundle}): no widget or adapter host, or it does not parse");
            }
        }
    }
    println!(
        "{} JSON files are not a screen widget's: adapter hosts of sequences ({unnamed_bad} unexplained)",
        unnamed.len()
    );
    println!("keys left untyped (in WidgetDef::fields only), per widget type:");
    for (t, keys) in &untyped {
        let k: Vec<_> = keys.iter().map(|(k, n)| format!("{k}×{n}")).collect();
        println!("  {t}: {}", k.join(" "));
    }
    println!("LayoutLayer JSON name -> GeneSys value: {layer_map:?}");
    println!("ModalMode JSON name -> GeneSys value: {modal_map:?}");
    println!(
        "scripts: {lua} Lua ({} distinct chunks of {} in the install, {missing_chunk} missing), {native} native, \
         {unresolved} unresolved; ScreenScript LuaAsset vs JSON mismatches: {lua_asset_mismatch}",
        chunks_used.len(),
        all_lua.len()
    );
    if !errors.is_empty() || unresolved > 0 || missing_chunk > 0 || unnamed_bad > 0 {
        bail!("widget check failed");
    }
    Ok(())
}
