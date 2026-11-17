//! Number <-> string conversions as the original's C runtime does them.
//!
//! The exe links Lua 5.1.3 against a statically linked pre-2015 MSVC CRT (its `.rdata` has the `1#INF`,
//! `1#QNAN`, `1#IND` strings of that CRT and Lua's `"%.14g"` at 0x01084474), so:
//! - `tostring(n)` / concatenation use `%.14g` with MSVC's three-digit exponents (`1e+020`) and its spellings
//!   of infinities / NaNs (`1.#INF`, `-1.#INF`, `-1.#IND` for the default NaN of 0/0, `1.#QNAN` otherwise);
//! - `lua_tointeger` (luaconf.h's `lua_number2integer`, an x87 `fistp` on MSVC x86) rounds to nearest-even,
//!   an out-of-range value gives the "integer indefinite" 0x80000000;
//! - the C casts `(long)n` of `string.format("%d")` truncate, out of range gives 0x80000000;
//! - `luaO_str2d` is `strtod` (decimal only in that CRT: no hex floats, no `inf` / `nan`) with a `strtoul`
//!   fallback for `0x` integers, trailing white space allowed.

/// `%.14g` (LUAI_NUMFFORMAT): what `tostring` gives for a number.
pub fn fmt_number(n: f64) -> String {
    fmt_g(n, 14, false, false)
}

/// MSVC's spelling of a non-finite value (without width / precision handling).
pub fn fmt_nonfinite(x: f64) -> Option<String> {
    if x.is_infinite() {
        Some(if x < 0.0 { "-1.#INF".into() } else { "1.#INF".into() })
    } else if x.is_nan() {
        let bits = x.to_bits();
        let neg = bits >> 63 != 0;
        // The x86 default NaN (0/0, inf-inf, ...) is negative with an all-zero payload: "indefinite".
        if neg && bits & 0x0007_ffff_ffff_ffff == 0 {
            Some("-1.#IND".into())
        } else {
            Some(format!("{}1.#QNAN", if neg { "-" } else { "" }))
        }
    } else {
        None
    }
}

fn exp_suffix(e: i32, upper: bool) -> String {
    format!("{}{}{:03}", if upper { 'E' } else { 'e' }, if e < 0 { '-' } else { '+' }, e.unsigned_abs())
}

fn strip_zeros(s: &mut String) {
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
}

/// `%.<prec>e` of |x| split into mantissa digits and decimal exponent.
fn sci(x: f64, prec: usize) -> (String, i32) {
    let s = format!("{:.*e}", prec, x.abs());
    let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
    (m.to_string(), e.parse().unwrap_or(0))
}

/// `%e` / `%E` (sign included for negative values, no `+` / space flags).
pub fn fmt_e(x: f64, prec: usize, upper: bool, alt: bool) -> String {
    if let Some(s) = fmt_nonfinite(x) {
        return s;
    }
    let (mut m, e) = sci(x, prec);
    if alt && prec == 0 {
        m.push('.');
    }
    format!("{}{}{}", if x.is_sign_negative() { "-" } else { "" }, m, exp_suffix(e, upper))
}

/// `%f`.
pub fn fmt_f(x: f64, prec: usize, alt: bool) -> String {
    if let Some(s) = fmt_nonfinite(x) {
        return s;
    }
    let mut s = format!("{:.*}", prec, x.abs());
    if alt && prec == 0 {
        s.push('.');
    }
    format!("{}{}", if x.is_sign_negative() { "-" } else { "" }, s)
}

/// `%g` / `%G`: `prec` significant digits, exponent form when the exponent is < -4 or >= prec, trailing
/// zeros removed unless `#`.
pub fn fmt_g(x: f64, prec: usize, upper: bool, alt: bool) -> String {
    if let Some(s) = fmt_nonfinite(x) {
        return s;
    }
    let p = prec.max(1);
    let sign = if x.is_sign_negative() { "-" } else { "" };
    let (m, e) = if x == 0.0 { ("0".to_string(), 0) } else { sci(x, p - 1) };
    if e < -4 || e >= p as i32 {
        let mut m = m;
        if !alt {
            strip_zeros(&mut m);
        }
        format!("{sign}{m}{}", exp_suffix(e, upper))
    } else {
        let mut s = format!("{:.*}", (p as i32 - 1 - e).max(0) as usize, x.abs());
        if !alt {
            strip_zeros(&mut s);
        }
        format!("{sign}{s}")
    }
}

/// `lua_number2integer` on MSVC x86 (`fistp`, round to nearest-even; out of range: 0x80000000).
pub fn number2integer(n: f64) -> i32 {
    let r = n.round_ties_even();
    if r.is_nan() || r < i32::MIN as f64 || r > i32::MAX as f64 { i32::MIN } else { r as i32 }
}

/// A C `(long)` cast of a double (truncation; out of range: 0x80000000).
pub fn trunc_i32(n: f64) -> i32 {
    let t = n.trunc();
    if t.is_nan() || t < i32::MIN as f64 || t > i32::MAX as f64 { i32::MIN } else { t as i32 }
}

/// A C `(unsigned long)` cast of a double (via a 64-bit conversion, keeping the low 32 bits).
pub fn trunc_u32(n: f64) -> u32 {
    let t = n.trunc();
    if t.is_nan() || t < i64::MIN as f64 || t > i64::MAX as f64 { 0 } else { t as i64 as u32 }
}

/// C `isspace` in the "C" locale.
pub fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `strtod`: the value and the index after the parsed text, `None` when nothing was converted.
pub fn strtod(s: &[u8]) -> Option<(f64, usize)> {
    let mut i = 0;
    while i < s.len() && is_space(s[i]) {
        i += 1;
    }
    let mut text = String::new();
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        text.push(s[i] as char);
        i += 1;
    }
    let mut digits = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        text.push(s[i] as char);
        i += 1;
        digits += 1;
    }
    if i < s.len() && s[i] == b'.' {
        text.push('.');
        i += 1;
        while i < s.len() && s[i].is_ascii_digit() {
            text.push(s[i] as char);
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if i < s.len() && (s[i] == b'e' || s[i] == b'E') {
        let mut j = i + 1;
        let mut exp = String::from("e");
        if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
            exp.push(s[j] as char);
            j += 1;
        }
        let start = j;
        while j < s.len() && s[j].is_ascii_digit() {
            exp.push(s[j] as char);
            j += 1;
        }
        if j > start {
            text.push_str(&exp);
            i = j;
        }
    }
    if text.ends_with('.') {
        text.push('0');
    }
    if text.starts_with('.') || text.starts_with("-.") || text.starts_with("+.") {
        text = text.replacen('.', "0.", 1);
    }
    let v: f64 = text.parse().ok()?;
    Some((v, i))
}

/// `strtoul`: value (saturating at u32::MAX, negated for a leading '-') and end index; end 0 when no digits.
pub fn strtoul(s: &[u8], base: u32) -> (u32, usize) {
    let mut i = 0;
    while i < s.len() && is_space(s[i]) {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    if base == 16 && i + 1 < s.len() && s[i] == b'0' && (s[i + 1] == b'x' || s[i + 1] == b'X') {
        // Only a prefix if a hex digit follows.
        if i + 2 < s.len() && s[i + 2].is_ascii_hexdigit() {
            i += 2;
        }
    }
    let start = i;
    let mut v: u64 = 0;
    let mut overflow = false;
    while i < s.len() {
        let Some(d) = (s[i] as char).to_digit(36).filter(|&d| d < base) else { break };
        v = v * base as u64 + d as u64;
        if v > u32::MAX as u64 {
            overflow = true;
            v = u32::MAX as u64;
        }
        i += 1;
    }
    if i == start {
        return (0, 0);
    }
    let v = if overflow {
        u32::MAX
    } else if neg {
        (v as u32).wrapping_neg()
    } else {
        v as u32
    };
    (v, i)
}

/// `luaO_str2d`: a string's numeric value for arithmetic coercion and `tonumber`.
pub fn str2d(s: &[u8]) -> Option<f64> {
    // The C string ends at the first NUL.
    let s = &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())];
    let (mut v, mut end) = strtod(s)?;
    if end < s.len() && (s[end] == b'x' || s[end] == b'X') {
        let (u, e) = strtoul(s, 16);
        v = u as f64;
        end = e;
    }
    while end < s.len() && is_space(s[end]) {
        end += 1;
    }
    if end == s.len() { Some(v) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g14() {
        assert_eq!(fmt_number(1.0), "1");
        assert_eq!(fmt_number(-0.5), "-0.5");
        assert_eq!(fmt_number(0.1), "0.1");
        assert_eq!(fmt_number(1.0 / 3.0), "0.33333333333333");
        assert_eq!(fmt_number(100000000000000.0), "1e+014");
        assert_eq!(fmt_number(12345678901234.0), "12345678901234");
        assert_eq!(fmt_number(1e-5), "1e-005");
        assert_eq!(fmt_number(0.0001), "0.0001");
        assert_eq!(fmt_number(f64::INFINITY), "1.#INF");
        assert_eq!(fmt_number(f64::from_bits(0xfff8_0000_0000_0000)), "-1.#IND");
        assert_eq!(fmt_number(2.5e-7), "2.5e-007");
    }

    #[test]
    fn str2d_cases() {
        assert_eq!(str2d(b" 10 "), Some(10.0));
        assert_eq!(str2d(b"0x1A"), Some(26.0));
        assert_eq!(str2d(b"1e2"), Some(100.0));
        assert_eq!(str2d(b".5"), Some(0.5));
        assert_eq!(str2d(b"5."), Some(5.0));
        assert_eq!(str2d(b"1e"), None);
        assert_eq!(str2d(b"0x"), None);
        assert_eq!(str2d(b"abc"), None);
        assert_eq!(str2d(b""), None);
        assert_eq!(str2d(b"inf"), None);
    }

    #[test]
    fn integer_conversions() {
        assert_eq!(number2integer(1.5), 2);
        assert_eq!(number2integer(2.5), 2);
        assert_eq!(number2integer(-1.5), -2);
        assert_eq!(number2integer(1e10), i32::MIN);
        assert_eq!(trunc_i32(-1.7), -1);
        assert_eq!(trunc_u32(-1.0), u32::MAX);
    }
}
