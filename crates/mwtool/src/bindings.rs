//! `mwtool bindings`: every binding path in the install (JSON widget files, Lua string constants, HSM
//! parameters, GeneSys string fields), parsed with `mw_logic::binding` (docs/decomp.md "Binding evaluator").
//! The summary counts distinct texts per source, parse errors and warnings, roots and op tokens.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use mw_assets::AssetStore;
use mw_bundle::ResourceType;
use mw_formats::genesys::{self, Value as GValue};
use mw_logic::binding::{Binding, ParseError, Warning, qualify};

pub struct Options<'a> {
    /// Only texts containing this.
    pub find: Option<&'a str>,
    /// Print every binding with its parse.
    pub parse: bool,
    /// Skip the GeneSys scan (the slow part).
    pub no_genesys: bool,
    /// Write `source \t key \t text \t result` for every binding here.
    pub out: Option<&'a Path>,
}

/// Where a binding text was found: (source, key / field / parameter name).
type Origins = BTreeMap<String, BTreeSet<(&'static str, String)>>;

/// Does `s` have the shape of a binding (`Root.Member...`, optional `~`, indices, ops)? A relative path
/// (no `.`) only counts under a binding key (`relative`).
fn looks_like_binding(s: &str, relative: bool) -> bool {
    let t = s.strip_prefix('{').and_then(|t| t.strip_suffix('}')).unwrap_or(s);
    let t = t.strip_prefix('~').unwrap_or(t);
    let Some(first) = t.chars().next() else { return false };
    if !(first.is_ascii_alphabetic() || first == '_' || first == '{') || t.len() > 160 {
        return false;
    }
    let head = t.split('(').next().unwrap_or(t);
    if !head.chars().all(|c| c.is_ascii_alphanumeric() || "_.[]\"#{}%~".contains(c)) {
        return false;
    }
    if t.contains('(') && !t.ends_with(')') {
        return false;
    }
    let last = head.rsplit('.').next().unwrap_or("");
    const EXT: &[&str] = &["lua", "json", "png", "dds", "bnd", "bndl", "wav", "txt", "xml", "dat", "bin", "exe", "com"];
    if EXT.contains(&last.to_ascii_lowercase().as_str()) {
        return false;
    }
    head.contains('.') && !head.ends_with('.') && !head.contains("..") || relative
}

fn is_binding_key(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    (k.contains("binding") && !k.contains("mode"))
        || k == "path"
        || k == "source"
        || k == "operand"
        || k == "datasource"
}

/// The string values of a JSON text with the key they belong to (array items inherit the array's key).
fn json_strings(text: &str) -> Vec<(String, String)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut keys: Vec<String> = vec![String::new()];
    let mut last: Option<String> = None;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                let mut s = Vec::new();
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' && i + 1 < b.len() {
                        i += 1;
                        s.push(match b[i] {
                            b'n' => b'\n',
                            b't' => b'\t',
                            c => c,
                        });
                    } else {
                        s.push(b[i]);
                    }
                    i += 1;
                }
                let s = String::from_utf8_lossy(&s).into_owned();
                // A key if a ':' follows.
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < b.len() && b[j] == b':' {
                    last = Some(s);
                } else {
                    let key = last.take().unwrap_or_else(|| keys.last().cloned().unwrap_or_default());
                    out.push((key, s));
                }
            }
            b'[' | b'{' => keys.push(last.take().unwrap_or_else(|| keys.last().cloned().unwrap_or_default())),
            b']' | b'}' => {
                keys.pop();
                if keys.is_empty() {
                    keys.push(String::new());
                }
                last = None;
            }
            b',' => last = None,
            _ => {}
        }
        i += 1;
    }
    out
}

/// Lua 5.1 string constants (type 4, 32-bit size including the NUL) in a chunk.
fn lua_strings(chunk: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 5 < chunk.len() {
        if chunk[i] == 4 {
            let n = u32::from_le_bytes(chunk[i + 1..i + 5].try_into().unwrap()) as usize;
            if (2..=200).contains(&n) && i + 5 + n <= chunk.len() {
                let s = &chunk[i + 5..i + 5 + n];
                if s[n - 1] == 0 && s[..n - 1].iter().all(|&c| (0x20..0x7f).contains(&c)) {
                    out.push(String::from_utf8_lossy(&s[..n - 1]).into_owned());
                    i += 5 + n;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

fn genesys_strings(v: &GValue, field: &str, out: &mut Vec<(String, String)>) {
    match v {
        GValue::Str(s) => out.push((field.to_string(), s.clone())),
        GValue::Struct(o) => {
            for (h, v) in &o.fields {
                let name = genesys::field_name(*h).map_or_else(|| format!("{h:08x}"), str::to_string);
                genesys_strings(v, &name, out);
            }
        }
        GValue::Array(a) => a.iter().for_each(|v| genesys_strings(v, field, out)),
        _ => {}
    }
}

pub fn run(store: &AssetStore, opt: &Options) -> Result<()> {
    let mut found: Origins = BTreeMap::new();
    let mut add = |src: &'static str, key: &str, text: &str| {
        let relative = is_binding_key(key);
        if looks_like_binding(text, relative) {
            found.entry(text.to_string()).or_default().insert((src, key.to_string()));
        }
    };
    let mut n_json = 0;
    let mut n_lua = 0;
    let mut n_hsm = 0;
    let mut n_gs = 0;
    for rel in store.bundle_paths().to_vec() {
        let Some(b) = store.bundle(&rel) else { continue };
        for e in b.entries_of_type(ResourceType::TextFile) {
            let Ok(res) = b.load(e) else { continue };
            let h = res.header();
            if h.len() < 8 {
                continue;
            }
            let n = (u32::from_le_bytes(h[..4].try_into().unwrap()) as usize).min(h.len() - 4);
            let body = &h[4..4 + n];
            if body.starts_with(b"\x1bLua") {
                n_lua += 1;
                for s in lua_strings(body) {
                    add("lua", "", &s);
                }
            } else {
                n_json += 1;
                for (k, s) in json_strings(&String::from_utf8_lossy(body)) {
                    add("json", &k, &s);
                }
            }
        }
        for e in b.entries_of_type(ResourceType::HSMData) {
            let Ok(data) =
                b.load(e).map_err(anyhow::Error::from).and_then(|r| Ok(mw_formats::hsm::HsmData::parse(&r)?))
            else {
                continue;
            };
            n_hsm += 1;
            for node in &data.nodes {
                for (k, v) in &node.parameters {
                    add("hsm", &mw_formats::hsm::key_name(*k), v);
                }
            }
        }
        if !opt.no_genesys {
            for e in b.entries_of_type(ResourceType::GenesysObject) {
                let Ok(res) = b.load(e) else { continue };
                let Ok(obj) = store.decode_genesys(&res) else { continue };
                n_gs += 1;
                let mut strs = Vec::new();
                genesys_strings(&GValue::Struct(obj), "", &mut strs);
                for (k, s) in strs {
                    add("genesys", &k, &s);
                }
            }
        }
        store.evict_all();
    }
    println!("scanned {n_json} JSON files, {n_lua} Lua chunks, {n_hsm} HSMData, {n_gs} GeneSys objects");

    let mut out = match opt.out {
        Some(p) => Some(std::io::BufWriter::new(std::fs::File::create(p)?)),
        None => None,
    };
    let mut per_source: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut errors: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut warnings: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut roots: BTreeMap<String, usize> = BTreeMap::new();
    let mut tokens: BTreeMap<String, usize> = BTreeMap::new();
    let (mut relative, mut spliced, mut fragments) = (0, 0, 0);
    for (text, origins) in &found {
        if opt.find.is_some_and(|f| !text.contains(f)) {
            continue;
        }
        // Lua builds some paths by concatenation: `"Rollouts.AvailablePerks["` .. i .. `"]"`.
        if origins.iter().all(|(s, _)| *s == "lua") && (text.ends_with(['[', '.']) || text.starts_with(['.', ']'])) {
            fragments += 1;
            continue;
        }
        // A whole value `{Path}` is a binding the value stands for (`FUN_00a50f40`).
        let text_inner = match text.strip_prefix('{').and_then(|t| t.strip_suffix('}')) {
            Some(inner) => {
                spliced += 1;
                inner
            }
            None => text.as_str(),
        };
        // As the widget code does before compiling: `~` = absolute, `#` = the item index; a path without a
        // root is relative to the widget's base (a stand-in base here).
        let abs = text_inner.strip_prefix('~');
        let text = &text_inner.to_string();
        let full = match abs {
            Some(a) => qualify("", a, Some(1)),
            None if !text.split('(').next().unwrap_or("").contains('.') => {
                relative += 1;
                qualify("Item", text, Some(1))
            }
            None => qualify("", text, Some(1)),
        };
        // The HSM and Lua's Resolve*Binding natives use `FUN_00a50650` (`parse`); the stricter `FUN_00a503e0`
        // (`parse_rooted`) for the rest, whose callers are not all known.
        let inner_only = origins.iter().all(|(s, _)| matches!(*s, "lua" | "hsm"));
        let parsed = if inner_only { Binding::parse(&full) } else { Binding::parse_rooted(&full) };
        for (src, _) in origins {
            let e = per_source.entry(src).or_default();
            e.0 += 1;
            e.1 += parsed.is_err() as usize;
        }
        let result = match &parsed {
            Ok(b) => {
                *roots.entry(b.segments[0].name.clone()).or_default() += 1;
                for op in &b.ops {
                    *tokens.entry(op.name.clone()).or_default() += 1;
                }
                for w in &b.warnings {
                    let kind = match w {
                        Warning::TextCut => "text over 127 bytes".to_string(),
                        Warning::TokenCut(_) => "op token over 23 bytes".to_string(),
                        Warning::IgnoredAfterOps(_) => "text after the ops".to_string(),
                        Warning::UnknownOp(n) => format!("unknown op token {n}"),
                        Warning::BetweenColonForm => "BETWEEN:a:b (always false)".to_string(),
                        Warning::Splice(_) => "{binding} splice in an op".to_string(),
                        Warning::NoOperand(n) => format!("{n} without an operand (compares with 0)"),
                    };
                    warnings.entry(kind).or_default().push(text.clone());
                }
                format!("{} segments, {} ops", b.segments.len(), b.ops.len())
            }
            Err(e) => {
                let kind = match e {
                    ParseError::NoMember => "no member".to_string(),
                    e => e.to_string(),
                };
                errors.entry(kind).or_default().push(text.clone());
                format!("error: {e}")
            }
        };
        let src = origins.iter().map(|(s, k)| format!("{s}:{k}")).collect::<Vec<_>>().join(",");
        if opt.parse {
            println!("{text}\t[{src}]\t{result}");
            if let Ok(b) = &parsed {
                for s in &b.segments {
                    println!("    {} {:08x} {:?}", s.name, s.hash, s.index);
                }
                for o in &b.ops {
                    println!(
                        "    op {:?} {} operand {:?} = {} ({:08x})",
                        o.code, o.name, o.operand_text, o.operand, o.operand_hash
                    );
                }
            }
        }
        if let Some(w) = out.as_mut() {
            writeln!(w, "{src}\t{}\t{result}", text.replace(['\t', '\n'], " "))?;
        }
    }
    println!(
        "{} distinct binding texts ({relative} relative to a widget base, {spliced} whole-value {{binding}}, \
         {fragments} Lua concatenation fragments skipped)",
        found.len()
    );
    for (src, (n, bad)) in &per_source {
        println!("  {src:8} {n:6} texts, {bad} parse errors");
    }
    for (kind, list) in &errors {
        println!("error {kind}: {} (e.g. {:?})", list.len(), &list[..list.len().min(3)]);
    }
    for (kind, list) in &warnings {
        println!("warning {kind}: {} (e.g. {:?})", list.len(), &list[..list.len().min(3)]);
    }
    let mut r: Vec<_> = roots.into_iter().collect();
    r.sort_by_key(|x| std::cmp::Reverse(x.1));
    println!(
        "{} roots: {}",
        r.len(),
        r.iter().take(40).map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", ")
    );
    let mut t: Vec<_> = tokens.into_iter().collect();
    t.sort_by_key(|x| std::cmp::Reverse(x.1));
    println!("op tokens: {}", t.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", "));
    Ok(())
}
