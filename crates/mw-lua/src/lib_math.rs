//! The math library (`lmathlib.c` 5.1.3, with the 5.0 name `mod` for `fmod`). `random` uses the MSVC CRT's
//! `rand()` (LCG 214013 / 2531011, 15-bit results); the game's CRT shares that state with its C++ code, so the
//! sequence here is the algorithm, not the game's draw order.

use crate::lib_base::{arg_error, check_int, check_number};
use crate::value::Value;
use crate::{Lua, LuaError, Result};

const RAND_MAX: u32 = 0x7fff;

fn one(lua: &mut Lua, t: &crate::TableRef, name: &str, f: fn(f64) -> f64) {
    lua.register_in(t, name, move |_, name, args| Ok(vec![Value::Number(f(check_number(&args, 0, name)?))]));
}

/// C `frexp`.
fn frexp(x: f64) -> (f64, i32) {
    if x == 0.0 || !x.is_finite() {
        return (x, 0);
    }
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    if exp == 0 {
        // Subnormal: scale up first.
        let (m, e) = frexp(x * f64::from_bits(0x4350_0000_0000_0000)); // 2^54
        return (m, e - 54);
    }
    let m = f64::from_bits((bits & !(0x7ff << 52)) | (1022 << 52));
    (m, exp - 1022)
}

pub(crate) fn rand(lua: &mut Lua) -> u32 {
    lua.rand_state = lua.rand_state.wrapping_mul(214013).wrapping_add(2531011);
    (lua.rand_state >> 16) & RAND_MAX
}

pub(crate) fn open(lua: &mut Lua) {
    let t = lua.create_table();
    one(lua, &t, "abs", f64::abs);
    one(lua, &t, "acos", f64::acos);
    one(lua, &t, "asin", f64::asin);
    one(lua, &t, "atan", f64::atan);
    one(lua, &t, "ceil", f64::ceil);
    one(lua, &t, "cos", f64::cos);
    one(lua, &t, "cosh", f64::cosh);
    one(lua, &t, "exp", f64::exp);
    one(lua, &t, "floor", f64::floor);
    one(lua, &t, "log", f64::ln);
    one(lua, &t, "log10", f64::log10);
    one(lua, &t, "sin", f64::sin);
    one(lua, &t, "sinh", f64::sinh);
    one(lua, &t, "sqrt", f64::sqrt);
    one(lua, &t, "tan", f64::tan);
    one(lua, &t, "tanh", f64::tanh);
    one(lua, &t, "deg", |x| x / (std::f64::consts::PI / 180.0));
    one(lua, &t, "rad", |x| x * (std::f64::consts::PI / 180.0));
    lua.register_in(&t, "atan2", |_, name, args| {
        Ok(vec![Value::Number(check_number(&args, 0, name)?.atan2(check_number(&args, 1, name)?))])
    });
    let fmod = |_: &mut Lua, name: &str, args: Vec<Value>| -> Result<Vec<Value>> {
        Ok(vec![Value::Number(check_number(&args, 0, name)? % check_number(&args, 1, name)?)])
    };
    lua.register_in(&t, "fmod", fmod);
    lua.register_in(&t, "mod", fmod);
    lua.register_in(&t, "pow", |_, name, args| {
        Ok(vec![Value::Number(check_number(&args, 0, name)?.powf(check_number(&args, 1, name)?))])
    });
    lua.register_in(&t, "modf", |_, name, args| {
        let x = check_number(&args, 0, name)?;
        let ip = x.trunc();
        let fp = if x.is_infinite() { 0.0 } else { x - ip };
        Ok(vec![Value::Number(ip), Value::Number(fp)])
    });
    lua.register_in(&t, "frexp", |_, name, args| {
        let (m, e) = frexp(check_number(&args, 0, name)?);
        Ok(vec![Value::Number(m), Value::Number(e as f64)])
    });
    lua.register_in(&t, "ldexp", |_, name, args| {
        let m = check_number(&args, 0, name)?;
        let e = check_int(&args, 1, name)?;
        Ok(vec![Value::Number(m * 2f64.powi(e as i32))])
    });
    lua.register_in(&t, "min", |_, name, args| {
        let mut m = check_number(&args, 0, name)?;
        for i in 1..args.len() {
            let d = check_number(&args, i, name)?;
            if d < m {
                m = d;
            }
        }
        Ok(vec![Value::Number(m)])
    });
    lua.register_in(&t, "max", |_, name, args| {
        let mut m = check_number(&args, 0, name)?;
        for i in 1..args.len() {
            let d = check_number(&args, i, name)?;
            if d > m {
                m = d;
            }
        }
        Ok(vec![Value::Number(m)])
    });
    lua.register_in(&t, "random", |lua, name, args| {
        let r = (rand(lua) % RAND_MAX) as f64 / RAND_MAX as f64;
        match args.len() {
            0 => Ok(vec![Value::Number(r)]),
            1 => {
                let u = check_int(&args, 0, name)?;
                if u < 1 {
                    return Err(arg_error(name, 0, "interval is empty"));
                }
                Ok(vec![Value::Number((r * u as f64).floor() + 1.0)])
            }
            2 => {
                let l = check_int(&args, 0, name)?;
                let u = check_int(&args, 1, name)?;
                if l > u {
                    return Err(arg_error(name, 1, "interval is empty"));
                }
                Ok(vec![Value::Number((r * (u - l + 1) as f64).floor() + l as f64)])
            }
            _ => Err(LuaError::new("wrong number of arguments")),
        }
    });
    lua.register_in(&t, "randomseed", |lua, name, args| {
        lua.rand_state = check_int(&args, 0, name)? as u32;
        Ok(vec![])
    });
    t.set_str("pi", std::f64::consts::PI);
    t.set_str("huge", f64::INFINITY);
    lua.set_global("math", t);
}

#[cfg(test)]
mod tests {
    #[test]
    fn frexp_matches_c() {
        assert_eq!(super::frexp(8.0), (0.5, 4));
        assert_eq!(super::frexp(-3.0), (-0.75, 2));
        assert_eq!(super::frexp(0.1).1, -3);
    }
}
