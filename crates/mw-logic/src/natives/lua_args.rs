//! Reading a Lua native's arguments the way the original's handlers read the Lua stack (Lua 5.1 `lapi.c` as
//! linked into NFS13.exe: `lua_gettop` 0x00ebfca0, `lua_tolstring` 0x00ec0170, `lua_tonumber` 0x00ec00c0,
//! `lua_toboolean` 0x00ec0140, `lua_touserdata` 0x00ec0280; results with `lua_pushnumber` 0x00ec0360,
//! `lua_pushboolean` 0x00ec0560, `lua_pushstring` 0x00ec03f0). `lua_gettop` is the argument count
//! (`args.len()`); a missing argument reads as `nil`.

use mw_lua::{Lua, LuaStr, Value};

/// `"<invalid>"` (`.rdata` 0x00fbd9c0): what every handler uses when `lua_tolstring` returns NULL.
pub const INVALID: &str = "<invalid>";

fn arg(args: &[Value], i: usize) -> &Value {
    args.get(i - 1).unwrap_or(&Value::Nil)
}

/// `lua_tolstring(L, i, NULL)` read as a C string: strings and numbers (`"%.14g"`), up to the first NUL; `None`
/// (NULL) for anything else.
pub fn tolstring(args: &[Value], i: usize) -> Option<String> {
    Lua::tolstring(arg(args, i)).map(|s: LuaStr| {
        let b = s.as_bytes();
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        String::from_utf8_lossy(&b[..end]).into_owned()
    })
}

/// `lua_tolstring`, `"<invalid>"` when NULL: the pattern of every string argument in the handlers.
pub fn string_or_invalid(args: &[Value], i: usize) -> String {
    tolstring(args, i).unwrap_or_else(|| INVALID.to_string())
}

/// `lua_tonumber`: 0 when the value is not a number or a string that converts.
pub fn tonumber(args: &[Value], i: usize) -> f64 {
    Lua::tonumber(arg(args, i)).unwrap_or(0.0)
}

/// `lua_toboolean`.
pub fn toboolean(args: &[Value], i: usize) -> bool {
    arg(args, i).truthy()
}

/// `lua_touserdata`: the light userdata's value (the original: the pointer); `None` (NULL) for anything else.
pub fn touserdata(args: &[Value], i: usize) -> Option<usize> {
    match arg(args, i) {
        Value::LightUserdata(p) => Some(*p),
        _ => None,
    }
}

/// MSVC `_ftol2` (0x00eda610, its x87 path 0x00eda646): a double truncated toward zero to 64 bits; NaN and out
/// of range give 0x8000000000000000. The handlers keep the low 32 bits ([`ftol32`]); `(int)` casts compiled as
/// `fistp` with the truncating control word (`| 0xc00`) give the same.
pub fn ftol(n: f64) -> i64 {
    let t = n.trunc();
    if t.is_nan() || t < i64::MIN as f64 || t >= i64::MAX as f64 { i64::MIN } else { t as i64 }
}

/// [`ftol`] kept to 32 bits (`int` / `unsigned` arguments).
pub fn ftol32(n: f64) -> i32 {
    ftol(n) as i32
}

/// `lua_tonumber` then `_ftol2` to 32 bits: every integer argument of the handlers.
pub fn int(args: &[Value], i: usize) -> i32 {
    ftol32(tonumber(args, i))
}

/// `lua_tonumber` then `fistp` with the default control word (round to nearest, ties to even; the decompiler
/// shows `ROUND(x)`), to 32 bits: the arguments the handlers read with a plain `(int)(longlong)ROUND(...)`
/// (`ChangePostFXSetting` key, `StartTextEntry` flag bit). NaN and out of range: the 64-bit `fistp` gives
/// 0x8000000000000000, whose low 32 bits are 0.
pub fn round_int(args: &[Value], i: usize) -> i32 {
    let r = tonumber(args, i).round_ties_even();
    if r.is_nan() || r.abs() >= 9.2e18 { 0 } else { r as i64 as i32 }
}

/// `lua_pushnumber((double)i)` of a signed int.
pub fn push_int(i: i32) -> Value {
    Value::Number(i as f64)
}

/// `lua_pushnumber` of an unsigned int (`fild` + 2^32 when negative, `.rdata` 0x00fbd4b0).
pub fn push_uint(u: u32) -> Value {
    Value::Number(u as f64)
}

/// `lua_pushstring`.
pub fn push_str(s: &str) -> Value {
    Value::Str(LuaStr::new(s.as_bytes()))
}

/// Truncate to what fits a C buffer of `size` bytes with its NUL (`strncat` / `strncpy` into `char[size]`), at a
/// character boundary.
pub fn fit(s: &str, size: usize) -> &str {
    if s.len() < size {
        return s;
    }
    let mut end = size.saturating_sub(1);
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_reads() {
        let args = vec![Value::Number(2.9), push_str("x\0y"), Value::Bool(false)];
        assert_eq!(int(&args, 1), 2);
        assert_eq!(ftol32(-2.9), -2);
        assert_eq!(ftol32(f64::NAN), 0);
        assert_eq!(round_int(&[Value::Number(2.5)], 1), 2);
        assert_eq!(round_int(&[Value::Number(3.5)], 1), 4);
        assert_eq!(round_int(&[Value::Number(-0.6)], 1), -1);
        assert_eq!(ftol32(4294967295.0), -1);
        assert_eq!(tolstring(&args, 1).as_deref(), Some("2.9"));
        assert_eq!(tolstring(&args, 2).as_deref(), Some("x"));
        assert_eq!(string_or_invalid(&args, 3), INVALID);
        assert_eq!(string_or_invalid(&args, 9), INVALID);
        assert_eq!(tonumber(&[push_str(" 0x10 ")], 1), 16.0);
        assert!(!toboolean(&args, 3) && toboolean(&args, 1) && !toboolean(&args, 4));
        assert_eq!(fit("abcdef", 4), "abc");
        assert_eq!(push_uint(u32::MAX).as_number(), Some(4294967295.0));
    }
}
