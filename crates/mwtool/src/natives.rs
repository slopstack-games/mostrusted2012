//! `mwtool natives`: the original's two native registries (`mw_logic::natives`): every HSM hook and Lua native
//! with its handler address, area and whether it is ported, then the counts per area.

use anyhow::{Result, bail};
use mw_logic::natives::{Area, Native, Registry};

pub fn run(area: Option<&str>, unported: bool, find: Option<&str>) -> Result<()> {
    let area = match area {
        Some(a) => match Area::parse(a) {
            Some(a) => Some(a),
            None => bail!("--area {a}: expected offline, online, platform, debug or unknown"),
        },
        None => None,
    };
    let find = find.map(str::to_lowercase);
    let r = Registry::original();
    let keep = |name: &str, a: Area, ported: bool| {
        area.is_none_or(|x| x == a)
            && !(unported && ported)
            && find.as_ref().is_none_or(|f| name.to_lowercase().contains(f.as_str()))
    };
    let mut shown = 0;
    let mut rows = |kind: &str, natives: Vec<(&str, u32, Area, bool)>| {
        for (name, handler, a, ported) in natives {
            if keep(name, a, ported) {
                println!(
                    "{kind:4} {name:<52} 0x{handler:08x}  {:<8} {}",
                    a.name(),
                    if ported { "ported" } else { "-" }
                );
                shown += 1;
            }
        }
    };
    fn list<F>(v: &[Native<F>]) -> Vec<(&'static str, u32, Area, bool)> {
        v.iter().map(|n| (n.name, n.handler, n.area, n.ported())).collect()
    }
    rows("hook", list(r.hooks()));
    rows("lua", list(r.lua_natives()));
    println!("{shown} shown");
    println!("{:<10} {:>14} {:>14}", "area", "hooks", "lua");
    let count = |v: &[(&str, u32, Area, bool)], a: Option<Area>| {
        let all = v.iter().filter(|n| a.is_none_or(|a| n.2 == a));
        let (n, p) = all.fold((0, 0), |(n, p), x| (n + 1, p + usize::from(x.3)));
        format!("{p:>4} / {n:<4}")
    };
    let (h, l) = (list(r.hooks()), list(r.lua_natives()));
    for a in Area::ALL {
        if h.iter().chain(&l).any(|n| n.2 == a) {
            println!("{:<10} {:>14} {:>14}", a.name(), count(&h, Some(a)), count(&l, Some(a)));
        }
    }
    println!("{:<10} {:>14} {:>14}   (ported / total)", "all", count(&h, None), count(&l, None));
    Ok(())
}
