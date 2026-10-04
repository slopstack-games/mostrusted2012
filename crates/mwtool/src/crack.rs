//! `mwtool genesys-crack`: dictionary search for GeneSys field names.
//!
//! Field names are stored only as [`field_hash`](mw_formats::genesys::field_hash)
//! values. This builds candidate names from word lists (one list per word
//! position, the last one repeating) in the game's naming styles and prints
//! every candidate whose hash matches a field. A 32-bit hash collides about
//! once per 4 billion candidates per target, so large searches print false
//! positives: keep vocabularies small and specific, restrict targets with
//! `--type`, and check each hit against the field's type and values before
//! adding it to `genesys_names.txt`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Context, Result};
use mw_assets::AssetStore;
use mw_bundle::ResourceType;
use mw_formats::genesys::{FieldHasher, field_name};
use rayon::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Style {
    /// `First_second_third` (handling data, most designer-facing fields).
    Snake,
    /// `First_Second_Third`.
    TitleSnake,
    /// `FirstSecondThird` (code-facing fields).
    Camel,
}

pub struct Options {
    pub words: Vec<PathBuf>,
    pub max_words: usize,
    pub styles: Vec<Style>,
    pub type_filter: Option<String>,
    pub include_named: bool,
}

pub fn run(store: &AssetStore, opt: &Options) -> Result<()> {
    // Targets: field hashes of every matching type, with the types using them.
    let mut targets: HashMap<u32, Vec<String>> = HashMap::new();
    let mut seen = HashSet::new();
    for rel in store.bundle_paths().to_vec() {
        let Some(b) = store.bundle(&rel) else { continue };
        for e in b.entries_of_type(ResourceType::GenesysType) {
            if !seen.insert(e.id) {
                continue;
            }
            let Some(t) = store.genesys_type(e.id) else { continue };
            if opt.type_filter.as_ref().is_some_and(|f| !t.name.contains(f.as_str())) {
                continue;
            }
            for f in &t.fields {
                if opt.include_named || field_name(f.name_hash).is_none() {
                    targets.entry(f.name_hash).or_default().push(t.name.clone());
                }
            }
        }
        store.evict_all();
    }
    eprintln!("{} target hashes", targets.len());

    let vocabs: Vec<Vec<String>> = opt
        .words
        .iter()
        .map(|p| {
            let text = std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
            let mut v: Vec<String> = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_string)
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            v.sort();
            Ok(v)
        })
        .collect::<Result<_>>()?;
    anyhow::ensure!(!vocabs.is_empty(), "give at least one --words file");

    let mut found: HashMap<u32, Vec<String>> = HashMap::new();
    for &style in &opt.styles {
        let levels: Vec<Vec<Vec<u8>>> = (0..opt.max_words)
            .map(|i| vocabs[i.min(vocabs.len() - 1)].iter().map(|w| styled(w, i == 0, style).into_bytes()).collect())
            .collect();
        let hits: Vec<(u32, String)> = levels[0]
            .par_iter()
            .enumerate()
            .flat_map_iter(|(i, first)| {
                let mut out = Vec::new();
                let mut path = vec![i];
                search(&levels, &targets, FieldHasher::new().push_all(first), &mut path, &mut out);
                out
            })
            .collect();
        for (h, name) in hits {
            let e = found.entry(h).or_default();
            if !e.contains(&name) {
                e.push(name);
            }
        }
    }
    let mut rows: Vec<_> = found.into_iter().collect();
    rows.sort();
    for (h, names) in rows {
        let mut types = targets[&h].clone();
        types.dedup();
        println!("{h:08x}\t{}\t{}", names.join(" "), types.join(","));
    }
    Ok(())
}

fn search(
    levels: &[Vec<Vec<u8>>],
    targets: &HashMap<u32, Vec<String>>,
    hasher: FieldHasher,
    path: &mut Vec<usize>,
    out: &mut Vec<(u32, String)>,
) {
    // Names of four bytes or fewer are stored verbatim; they need no search.
    if hasher.len() > 4 && targets.contains_key(&hasher.finish()) {
        let name = path.iter().enumerate().map(|(l, &i)| String::from_utf8_lossy(&levels[l][i])).collect();
        out.push((hasher.finish(), name));
    }
    let depth = path.len();
    if depth < levels.len() {
        for (i, w) in levels[depth].iter().enumerate() {
            path.push(i);
            search(levels, targets, hasher.push_all(w), path, out);
            path.pop();
        }
    }
}

/// A word as it appears at a position in a name. Words containing capitals
/// (acronyms like `RPM`) are kept as written.
fn styled(word: &str, first: bool, style: Style) -> String {
    let w = if word.chars().any(char::is_uppercase) { word.to_string() } else { word.to_lowercase() };
    let cap = |w: &str| {
        let mut c = w.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    };
    match (style, first) {
        (_, true) => cap(&w),
        (Style::Snake, false) => format!("_{w}"),
        (Style::TitleSnake, false) => format!("_{}", cap(&w)),
        (Style::Camel, false) => cap(&w),
    }
}
