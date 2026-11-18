//! The base library (`lbaselib.c` 5.1.3) without coroutines, chunk loading and environments, plus the
//! argument checks of `lauxlib.c` the libraries share.

use crate::number::{fmt_number, number2integer, strtoul};
use crate::value::{LuaStr, TableRef, Value};
use crate::{Lua, LuaError, Result};

// ---- lauxlib argument checks (messages as luaL_argerror / luaL_typerror) ----

pub(crate) fn arg_error(fname: &str, n: usize, extra: &str) -> LuaError {
    let mut e = LuaError::new(format!("bad argument #{} to '{}' ({})", n + 1, fname, extra));
    e.arg = Some((n + 1, extra.to_string()));
    e
}

pub(crate) fn type_name_of(args: &[Value], n: usize) -> &'static str {
    args.get(n).map(|v| v.type_name()).unwrap_or("no value")
}

pub(crate) fn tag_error(args: &[Value], n: usize, fname: &str, expected: &str) -> LuaError {
    arg_error(fname, n, &format!("{expected} expected, got {}", type_name_of(args, n)))
}

pub(crate) fn is_none_or_nil(args: &[Value], n: usize) -> bool {
    args.get(n).is_none_or(|v| v.is_nil())
}

pub(crate) fn check_any(args: &[Value], n: usize, fname: &str) -> Result<()> {
    if n >= args.len() { Err(arg_error(fname, n, "value expected")) } else { Ok(()) }
}

pub(crate) fn check_number(args: &[Value], n: usize, fname: &str) -> Result<f64> {
    args.get(n).and_then(Lua::tonumber).ok_or_else(|| tag_error(args, n, fname, "number"))
}

/// `luaL_checkinteger` / `luaL_checkint`: `lua_tointeger` (MSVC rounding) of a number or numeric string.
pub(crate) fn check_int(args: &[Value], n: usize, fname: &str) -> Result<i64> {
    Ok(number2integer(check_number(args, n, fname)?) as i64)
}

pub(crate) fn opt_int(args: &[Value], n: usize, fname: &str, def: i64) -> Result<i64> {
    if is_none_or_nil(args, n) { Ok(def) } else { check_int(args, n, fname) }
}

pub(crate) fn check_str(args: &[Value], n: usize, fname: &str) -> Result<LuaStr> {
    args.get(n).and_then(Lua::tolstring).ok_or_else(|| tag_error(args, n, fname, "string"))
}

pub(crate) fn opt_str(args: &[Value], n: usize, fname: &str, def: &str) -> Result<LuaStr> {
    if is_none_or_nil(args, n) { Ok(LuaStr::new(def.as_bytes())) } else { check_str(args, n, fname) }
}

pub(crate) fn check_table(args: &[Value], n: usize, fname: &str) -> Result<TableRef> {
    match args.get(n) {
        Some(Value::Table(t)) => Ok(t.clone()),
        _ => Err(tag_error(args, n, fname, "table")),
    }
}

// ---- tostring ----

/// `luaB_tostring`: `__tostring`, numbers as `%.14g`, others as `type: address` (MSVC `%p`: 8 hex digits).
pub(crate) fn tostring(lua: &mut Lua, v: &Value) -> Result<LuaStr> {
    let tm = lua.metamethod(v, "__tostring");
    if !tm.is_nil() {
        let r = lua.call(&tm, vec![v.clone()])?;
        return Ok(match r.into_iter().next() {
            Some(Value::Str(s)) => s,
            Some(Value::Number(n)) => LuaStr::new(fmt_number(n).as_bytes()),
            _ => LuaStr::new(b""),
        });
    }
    Ok(match v {
        Value::Number(n) => LuaStr::new(fmt_number(*n).as_bytes()),
        Value::Str(s) => s.clone(),
        Value::Bool(b) => LuaStr::new(if *b { b"true" } else { b"false" }),
        Value::Nil => LuaStr::new(b"nil"),
        Value::Table(t) => LuaStr::new(format!("table: {:08X}", t.id()).as_bytes()),
        Value::Function(f) => LuaStr::new(format!("function: {:08X}", f.id()).as_bytes()),
        Value::LightUserdata(p) => LuaStr::new(format!("userdata: {:08X}", *p as u32).as_bytes()),
    })
}

fn ret(v: impl Into<Value>) -> Result<Vec<Value>> {
    Ok(vec![v.into()])
}

pub(crate) fn open(lua: &mut Lua) {
    let g = lua.globals();
    g.set_str("_G", g.clone());
    g.set_str("_VERSION", "Lua 5.1");

    lua.register("assert", |_, _, args| {
        if args.first().is_some_and(|v| v.truthy()) {
            return Ok(args);
        }
        if args.is_empty() {
            return Err(arg_error("assert", 0, "value expected"));
        }
        let msg = opt_str(&args, 1, "assert", "assertion failed!")?;
        Err(LuaError::with_value(Value::Str(msg)))
    });
    lua.register("collectgarbage", |_, name, args| {
        let opt = opt_str(&args, 0, name, "collect")?;
        match opt.as_bytes() {
            b"count" => ret(0.0),
            b"step" => ret(true),
            b"stop" | b"restart" | b"collect" | b"setpause" | b"setstepmul" => ret(0.0),
            o => Err(arg_error(name, 0, &format!("invalid option '{}'", String::from_utf8_lossy(o)))),
        }
    });
    lua.register("error", |lua, _, args| {
        let level = opt_int(&args, 1, "error", 1)?;
        let v = args.into_iter().next().unwrap_or_default();
        // `lua_isstring` is also true for a number, which the concatenation turns into a string.
        if let Some(s) = Lua::tolstring(&v)
            && level > 0
        {
            // Level 1 is the function that called error (frame below this native).
            let w = lua.where_(level as usize);
            let mut b = w.into_bytes();
            b.extend_from_slice(s.as_bytes());
            return Err(LuaError::with_value(Value::Str(LuaStr::new(&b))));
        }
        Err(LuaError::with_value(v))
    });
    lua.register("gcinfo", |_, _, _| ret(0.0));
    lua.register("getmetatable", |lua, name, args| {
        check_any(&args, 0, name)?;
        let mt = match &args[0] {
            Value::Table(t) => t.metatable(),
            Value::Str(_) => lua.string_mt.clone(),
            _ => None,
        };
        let Some(mt) = mt else { return ret(Value::Nil) };
        let protected = mt.get_str("__metatable");
        if protected.is_nil() { ret(mt) } else { ret(protected) }
    });
    let ipairs_aux = lua.create_function("ipairs_aux", |_, name, args| {
        let i = check_int(&args, 1, name)? + 1;
        let t = check_table(&args, 0, name)?;
        let v = t.0.t.borrow().get_int(i);
        if v.is_nil() { Ok(vec![]) } else { Ok(vec![Value::Number(i as f64), v]) }
    });
    lua.register("ipairs", move |_, name, args| {
        check_any(&args, 0, name)?;
        Ok(vec![Value::Function(ipairs_aux.clone()), args[0].clone(), Value::Number(0.0)])
    });
    let next = lua.create_function("next", |_, name, args| {
        let t = check_table(&args, 0, name)?;
        let k = args.get(1).cloned().unwrap_or_default();
        match t.next(&k)? {
            Some((k, v)) => Ok(vec![k, v]),
            None => Ok(vec![Value::Nil]),
        }
    });
    lua.set_global("next", next.clone());
    lua.register("pairs", move |_, name, args| {
        let t = check_table(&args, 0, name)?;
        Ok(vec![Value::Function(next.clone()), Value::Table(t), Value::Nil])
    });
    lua.register("pcall", |lua, name, mut args| {
        check_any(&args, 0, name)?;
        let f = args.remove(0);
        match lua.call(&f, args) {
            Ok(mut r) => {
                r.insert(0, Value::Bool(true));
                Ok(r)
            }
            Err(e) => Ok(vec![Value::Bool(false), e.value]),
        }
    });
    lua.register("xpcall", |lua, name, args| {
        check_any(&args, 1, name)?;
        let f = args[0].clone();
        let handler = args[1].clone();
        match lua.call(&f, vec![]) {
            Ok(mut r) => {
                r.insert(0, Value::Bool(true));
                Ok(r)
            }
            Err(e) => {
                // luaG_errormsg: a handler that is not a function, or that errors itself (it is called again
                // on its own error until the C stack overflows), ends in LUA_ERRERR.
                let h = match handler {
                    Value::Function(_) => lua.call(&handler, vec![e.value]).ok(),
                    _ => None,
                };
                let h = h.map(|r| r.into_iter().next().unwrap_or_default());
                Ok(vec![Value::Bool(false), h.unwrap_or_else(|| Value::from("error in error handling"))])
            }
        }
    });
    lua.register("print", |lua, _, args| {
        let ts = lua.global("tostring");
        let mut out = Vec::new();
        for (i, v) in args.iter().enumerate() {
            let r = lua.call(&ts, vec![v.clone()])?;
            let Some(Value::Str(s)) = r.into_iter().next() else {
                return Err(LuaError::new("'tostring' must return a string to 'print'"));
            };
            if i > 0 {
                out.push(b'\t');
            }
            out.extend_from_slice(s.as_bytes());
        }
        if let Some(h) = lua.print_hook.clone() {
            h(&String::from_utf8_lossy(&out));
        }
        Ok(vec![])
    });
    lua.register("rawequal", |_, name, args| {
        check_any(&args, 0, name)?;
        check_any(&args, 1, name)?;
        ret(args[0].raw_eq(&args[1]))
    });
    lua.register("rawget", |_, name, args| {
        let t = check_table(&args, 0, name)?;
        check_any(&args, 1, name)?;
        ret(t.get(&args[1]))
    });
    lua.register("rawset", |_, name, args| {
        let t = check_table(&args, 0, name)?;
        check_any(&args, 1, name)?;
        check_any(&args, 2, name)?;
        t.set(args[1].clone(), args[2].clone())?;
        ret(t)
    });
    lua.register("select", |_, name, args| {
        let n = args.len() as i64;
        if let Some(Value::Str(s)) = args.first()
            && s.as_bytes().first() == Some(&b'#')
        {
            return ret((n - 1) as f64);
        }
        let mut i = check_int(&args, 0, name)?;
        if i < 0 {
            i += n;
        } else if i > n {
            i = n;
        }
        if i < 1 {
            return Err(arg_error(name, 0, "index out of range"));
        }
        Ok(args[i as usize..].to_vec())
    });
    lua.register("setmetatable", |_, name, args| {
        let t = check_table(&args, 0, name)?;
        let mt = match args.get(1) {
            Some(Value::Nil) => None,
            Some(Value::Table(m)) => Some(m.clone()),
            _ => return Err(arg_error(name, 1, "nil or table expected")),
        };
        if t.metatable().is_some_and(|m| !m.get_str("__metatable").is_nil()) {
            return Err(LuaError::new("cannot change a protected metatable"));
        }
        t.set_metatable(mt);
        ret(t)
    });
    lua.register("tonumber", |_, name, args| {
        let base = opt_int(&args, 1, name, 10)?;
        if base == 10 {
            check_any(&args, 0, name)?;
            return ret(Lua::tonumber(&args[0]).map(Value::Number).unwrap_or_default());
        }
        let s = check_str(&args, 0, name)?;
        if !(2..=36).contains(&base) {
            return Err(arg_error(name, 1, "base out of range"));
        }
        let b = s.as_bytes();
        let b = &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())];
        let (v, mut end) = strtoul(b, base as u32);
        if end != 0 {
            while end < b.len() && crate::number::is_space(b[end]) {
                end += 1;
            }
            if end == b.len() {
                return ret(v as f64);
            }
        }
        ret(Value::Nil)
    });
    lua.register("tostring", |lua, name, args| {
        check_any(&args, 0, name)?;
        Ok(vec![Value::Str(tostring(lua, &args[0])?)])
    });
    lua.register("type", |_, name, args| {
        check_any(&args, 0, name)?;
        ret(args[0].type_name())
    });
    lua.register("unpack", |_, name, args| {
        let t = check_table(&args, 0, name)?;
        let i = opt_int(&args, 1, name, 1)?;
        let e = if is_none_or_nil(&args, 2) { t.len() as i64 } else { check_int(&args, 2, name)? };
        if i > e {
            return Ok(vec![]);
        }
        let n = e - i + 1;
        if n <= 0 || n >= 8000 {
            return Err(LuaError::new("too many results to unpack"));
        }
        let tb = t.0.t.borrow();
        Ok((i..=e).map(|k| tb.get_int(k)).collect())
    });
}
