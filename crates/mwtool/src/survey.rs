//! `mwtool survey`: statistics for every 4-byte field of a resource type's
//! header across every instance in the install. The fastest way to separate
//! constants, counts, pointers, import slots, floats and enums in an unknown
//! struct.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Result;
use mw_bundle::{Bundle, ResourceType};
use mw_gamedir::GameDir;
use rayon::prelude::*;

pub struct Options {
    pub ty: ResourceType,
    /// Number of header bytes to report.
    pub len: usize,
    /// Max instances sampled.
    pub limit: usize,
    /// Survey chunk 1 instead of chunk 0.
    pub body: bool,
}

#[derive(Default)]
struct Field {
    present: usize,
    zero: usize,
    pointer: usize,
    import: usize,
    float: usize,
    values: HashMap<u32, usize>,
}

pub fn run(game: &GameDir, opt: &Options) -> Result<()> {
    let bundles = game.files_with_extension("BNDL");
    let samples: Mutex<Vec<(Vec<u8>, Vec<u32>)>> = Mutex::default();
    bundles.par_iter().for_each(|rel| {
        if samples.lock().unwrap().len() >= opt.limit {
            return;
        }
        let Ok(b) = Bundle::open(game.root().join(rel)) else { return };
        for e in b.entries_of_type(opt.ty) {
            let Ok(res) = b.load(e) else { continue };
            let imports = res.imports.iter().map(|i| i.offset).collect();
            let data = if opt.body { res.chunks[1].clone() } else { res.chunks[0].clone() };
            let mut s = samples.lock().unwrap();
            if s.len() >= opt.limit {
                return;
            }
            s.push((data, imports));
        }
    });
    let samples = samples.into_inner().unwrap();
    if samples.is_empty() {
        println!("no {} resources found", opt.ty);
        return Ok(());
    }
    let sizes: Vec<usize> = samples.iter().map(|(d, _)| d.len()).collect();
    println!(
        "{} instances of {}, chunk{} size min {:#x} max {:#x}",
        samples.len(),
        opt.ty,
        opt.body as u8,
        sizes.iter().min().unwrap(),
        sizes.iter().max().unwrap()
    );
    println!(
        "{:>6} {:>6} {:>5} {:>5} {:>5} {:>5} {:>6}  top values",
        "off", "have", "zero", "ptr", "imp", "flt", "uniq"
    );

    for off in (0..opt.len).step_by(4) {
        let mut f = Field::default();
        for (data, imports) in &samples {
            let Some(b) = data.get(off..off + 4) else { continue };
            let v = u32::from_le_bytes(b.try_into().unwrap());
            f.present += 1;
            if v == 0 {
                f.zero += 1;
            }
            if imports.contains(&(off as u32)) {
                f.import += 1;
            } else if v != 0 && (v as usize) < data.len() && v % 4 == 0 && v as usize >= off.min(0x10) {
                f.pointer += 1;
            }
            let fl = f32::from_bits(v);
            if v != 0 && fl.is_finite() && (1e-4..1e7).contains(&fl.abs()) {
                f.float += 1;
            }
            *f.values.entry(v).or_default() += 1;
        }
        if f.present == 0 {
            break;
        }
        let mut top: Vec<_> = f.values.iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let pct = |n: usize| (n * 100 / f.present).to_string();
        let shown: Vec<String> = top.iter().take(5).map(|(v, n)| format!("{v:#x}×{n}")).collect();
        println!(
            "{off:#6x} {:>6} {:>5} {:>5} {:>5} {:>5} {:>6}  {}{}",
            f.present,
            pct(f.zero),
            pct(f.pointer),
            pct(f.import),
            pct(f.float),
            f.values.len(),
            shown.join(" "),
            hint(&f, &top)
        );
    }
    Ok(())
}

/// A short guess at what the field is.
fn hint(f: &Field, top: &[(&u32, &usize)]) -> String {
    let n = f.present;
    if f.values.len() == 1 {
        return "   [const]".into();
    }
    if f.import * 10 >= n * 9 {
        return "   [import]".into();
    }
    if f.pointer * 10 >= n * 8 {
        return "   [ptr]".into();
    }
    if f.float * 10 >= (n - f.zero) * 9 && f.zero < n {
        return "   [f32]".into();
    }
    let small = top.iter().all(|(v, _)| **v < 0x10000);
    if small && f.values.len() <= 16 {
        return "   [enum/flags]".into();
    }
    if small {
        return "   [count/index]".into();
    }
    String::new()
}
