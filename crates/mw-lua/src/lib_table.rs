//! The table library (`ltablib.c` 5.1.3): concat, foreach, foreachi, getn, insert, maxn, remove, setn, sort.
//! `sort` is the same quicksort, so equal keys and inconsistent comparators end up as in the game.

use crate::lib_base::{arg_error, check_int, check_table, is_none_or_nil, opt_int, opt_str, tag_error};
use crate::value::{TableRef, Value};
use crate::{Lua, LuaError, Result};

fn geti(t: &TableRef, i: i64) -> Value {
    t.0.t.borrow().get_int(i)
}

fn seti(t: &TableRef, i: i64, v: Value) {
    t.0.t.borrow_mut().set_int(i, v);
}

fn sort_lt(lua: &mut Lua, comp: &Value, a: &Value, b: &Value) -> Result<bool> {
    if comp.is_nil() {
        lua.less_than(a, b)
    } else {
        Ok(lua.call(comp, vec![a.clone(), b.clone()])?.into_iter().next().unwrap_or_default().truthy())
    }
}

fn sort_error() -> LuaError {
    LuaError::new("invalid order function for sorting")
}

/// `auxsort`.
fn auxsort(lua: &mut Lua, t: &TableRef, comp: &Value, mut l: i64, mut u: i64) -> Result<()> {
    while l < u {
        let (al, au) = (geti(t, l), geti(t, u));
        if sort_lt(lua, comp, &au, &al)? {
            seti(t, l, au);
            seti(t, u, al);
        }
        if u - l == 1 {
            break;
        }
        let mut i = (l + u) / 2;
        let (ai, al) = (geti(t, i), geti(t, l));
        if sort_lt(lua, comp, &ai, &al)? {
            seti(t, i, al);
            seti(t, l, ai);
        } else {
            let au = geti(t, u);
            if sort_lt(lua, comp, &au, &ai)? {
                seti(t, i, au);
                seti(t, u, ai);
            }
        }
        if u - l == 2 {
            break;
        }
        let p = geti(t, i);
        let au1 = geti(t, u - 1);
        seti(t, i, au1);
        seti(t, u - 1, p.clone());
        i = l;
        let mut j = u - 1;
        loop {
            loop {
                i += 1;
                let ai = geti(t, i);
                if !sort_lt(lua, comp, &ai, &p)? {
                    break;
                }
                if i > u {
                    return Err(sort_error());
                }
            }
            loop {
                j -= 1;
                let aj = geti(t, j);
                if !sort_lt(lua, comp, &p, &aj)? {
                    break;
                }
                if j < l {
                    return Err(sort_error());
                }
            }
            if j < i {
                break;
            }
            let (ai, aj) = (geti(t, i), geti(t, j));
            seti(t, i, aj);
            seti(t, j, ai);
        }
        let (au1, ai) = (geti(t, u - 1), geti(t, i));
        seti(t, u - 1, ai);
        seti(t, i, au1);
        let (nj, ni);
        if i - l < u - i {
            nj = l;
            ni = i - 1;
            l = i + 1;
        } else {
            nj = i + 1;
            ni = u;
            u = i - 1;
        }
        auxsort(lua, t, comp, nj, ni)?;
    }
    Ok(())
}

pub(crate) fn open(lua: &mut Lua) {
    let t = lua.create_table();
    lua.register_in(&t, "concat", |_, name, args| {
        let sep = opt_str(&args, 1, name, "")?;
        let tb = check_table(&args, 0, name)?;
        let i = opt_int(&args, 2, name, 1)?;
        let last = if is_none_or_nil(&args, 3) { tb.len() as i64 } else { check_int(&args, 3, name)? };
        let mut out = Vec::new();
        let mut k = i;
        while k <= last {
            let v = geti(&tb, k);
            let Some(s) = Lua::tolstring(&v) else {
                return Err(arg_error(name, 0, "table contains non-strings"));
            };
            out.extend_from_slice(s.as_bytes());
            if k != last {
                out.extend_from_slice(sep.as_bytes());
            }
            k += 1;
        }
        Ok(vec![Value::from(out.as_slice())])
    });
    lua.register_in(&t, "foreach", |lua, name, args| {
        let tb = check_table(&args, 0, name)?;
        let f = args.get(1).cloned().unwrap_or_default();
        if !matches!(f, Value::Function(_)) {
            return Err(tag_error(&args, 1, name, "function"));
        }
        let mut k = Value::Nil;
        while let Some((nk, v)) = tb.next(&k)? {
            let r = lua.call(&f, vec![nk.clone(), v])?.into_iter().next().unwrap_or_default();
            if !r.is_nil() {
                return Ok(vec![r]);
            }
            k = nk;
        }
        Ok(vec![])
    });
    lua.register_in(&t, "foreachi", |lua, name, args| {
        let tb = check_table(&args, 0, name)?;
        let f = args.get(1).cloned().unwrap_or_default();
        if !matches!(f, Value::Function(_)) {
            return Err(tag_error(&args, 1, name, "function"));
        }
        let n = tb.len() as i64;
        for i in 1..=n {
            let r = lua.call(&f, vec![Value::Number(i as f64), geti(&tb, i)])?.into_iter().next().unwrap_or_default();
            if !r.is_nil() {
                return Ok(vec![r]);
            }
        }
        Ok(vec![])
    });
    lua.register_in(&t, "getn", |_, name, args| Ok(vec![Value::Number(check_table(&args, 0, name)?.len() as f64)]));
    lua.register_in(&t, "maxn", |_, name, args| {
        let tb = check_table(&args, 0, name)?;
        let mut max = 0.0f64;
        for (k, _) in tb.pairs() {
            if let Value::Number(n) = k
                && n > max
            {
                max = n;
            }
        }
        Ok(vec![Value::Number(max)])
    });
    lua.register_in(&t, "insert", |_, name, args| {
        let tb = check_table(&args, 0, name)?;
        let mut e = tb.len() as i64 + 1;
        let pos = match args.len() {
            2 => e,
            3 => {
                let pos = check_int(&args, 1, name)?;
                if pos > e {
                    e = pos;
                }
                let mut i = e;
                while i > pos {
                    let v = geti(&tb, i - 1);
                    seti(&tb, i, v);
                    i -= 1;
                }
                pos
            }
            _ => return Err(LuaError::new("wrong number of arguments to 'insert'")),
        };
        seti(&tb, pos, args[args.len() - 1].clone());
        Ok(vec![])
    });
    lua.register_in(&t, "remove", |_, name, args| {
        let tb = check_table(&args, 0, name)?;
        let e = tb.len() as i64;
        let mut pos = opt_int(&args, 1, name, e)?;
        if e == 0 {
            return Ok(vec![]);
        }
        let r = geti(&tb, pos);
        while pos < e {
            let v = geti(&tb, pos + 1);
            seti(&tb, pos, v);
            pos += 1;
        }
        seti(&tb, e, Value::Nil);
        Ok(vec![r])
    });
    lua.register_in(&t, "setn", |_, _, _| Err(LuaError::new("'setn' is obsolete")));
    lua.register_in(&t, "sort", |lua, name, args| {
        let tb = check_table(&args, 0, name)?;
        let n = tb.len() as i64;
        let comp = args.get(1).cloned().unwrap_or_default();
        if !comp.is_nil() && !matches!(comp, Value::Function(_)) {
            return Err(tag_error(&args, 1, name, "function"));
        }
        auxsort(lua, &tb, &comp, 1, n)?;
        Ok(vec![])
    });
    lua.set_global("table", t);
}
