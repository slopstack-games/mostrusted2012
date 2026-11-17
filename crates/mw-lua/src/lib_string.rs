//! The string library (`lstrlib.c` 5.1.3): byte, char, find, format, gmatch (and the 5.0 name gfind), gsub,
//! len, lower, match, rep, reverse, sub, upper. Patterns are a port of its backtracking matcher; character
//! classes are the "C" locale's. Strings get the metatable `{__index = string}`.

use std::cell::Cell;
use std::rc::Rc;

use crate::lib_base::{arg_error, check_int, check_number, check_str, opt_int, tag_error};
use crate::number::{fmt_e, fmt_f, fmt_g, trunc_i32, trunc_u32};
use crate::value::{LuaStr, Value};
use crate::{Lua, LuaError, Result};

const L_ESC: u8 = b'%';
const SPECIALS: &[u8] = b"^$*+?.([%-";
const MAXCAPTURES: usize = 32;
const CAP_UNFINISHED: isize = -1;
const CAP_POSITION: isize = -2;

/// `posrelat`: negative positions count from the end.
fn posrelat(pos: i64, len: usize) -> i64 {
    let pos = if pos < 0 { pos + len as i64 + 1 } else { pos };
    pos.max(0)
}

fn s(v: &[u8]) -> Value {
    Value::Str(LuaStr::new(v))
}

// ---- the matcher ----

struct MatchState<'a> {
    src: &'a [u8],
    pat: &'a [u8],
    level: usize,
    capture: [(usize, isize); MAXCAPTURES],
}

fn match_class(c: u8, cl: u8) -> bool {
    let res = match cl.to_ascii_lowercase() {
        b'a' => c.is_ascii_alphabetic(),
        b'c' => c.is_ascii_control(),
        b'd' => c.is_ascii_digit(),
        b'l' => c.is_ascii_lowercase(),
        b'p' => c.is_ascii_punctuation(),
        b's' => crate::number::is_space(c),
        b'u' => c.is_ascii_uppercase(),
        b'w' => c.is_ascii_alphanumeric(),
        b'x' => c.is_ascii_hexdigit(),
        b'z' => c == 0,
        _ => return cl == c,
    };
    if cl.is_ascii_uppercase() { !res } else { res }
}

impl MatchState<'_> {
    fn err(&self, msg: &str) -> LuaError {
        LuaError::new(msg.to_string())
    }

    fn class_end(&self, mut p: usize) -> Result<usize> {
        let c = self.pat[p];
        p += 1;
        if c == L_ESC {
            if p >= self.pat.len() {
                return Err(self.err("malformed pattern (ends with '%')"));
            }
            return Ok(p + 1);
        }
        if c == b'[' {
            if p < self.pat.len() && self.pat[p] == b'^' {
                p += 1;
            }
            loop {
                if p >= self.pat.len() {
                    return Err(self.err("malformed pattern (missing ']')"));
                }
                let c = self.pat[p];
                p += 1;
                if c == L_ESC && p < self.pat.len() {
                    p += 1;
                }
                if p < self.pat.len() && self.pat[p] == b']' {
                    break;
                }
            }
            return Ok(p + 1);
        }
        Ok(p)
    }

    /// `matchbracketclass`: `p` at '[', `ec` at the closing ']'.
    fn match_bracket_class(&self, c: u8, mut p: usize, ec: usize) -> bool {
        let mut sig = true;
        if self.pat[p + 1] == b'^' {
            sig = false;
            p += 1;
        }
        loop {
            p += 1;
            if p >= ec {
                break;
            }
            if self.pat[p] == L_ESC {
                p += 1;
                if match_class(c, self.pat[p]) {
                    return sig;
                }
            } else if self.pat.get(p + 1) == Some(&b'-') && p + 2 < ec {
                p += 2;
                if self.pat[p - 2] <= c && c <= self.pat[p] {
                    return sig;
                }
            } else if self.pat[p] == c {
                return sig;
            }
        }
        !sig
    }

    fn single_match(&self, c: u8, p: usize, ep: usize) -> bool {
        match self.pat[p] {
            b'.' => true,
            L_ESC => match_class(c, self.pat[p + 1]),
            b'[' => self.match_bracket_class(c, p, ep - 1),
            x => x == c,
        }
    }

    fn match_balance(&self, s: usize, p: usize) -> Result<Option<usize>> {
        if p + 1 >= self.pat.len() {
            return Err(self.err("unbalanced pattern"));
        }
        if s >= self.src.len() || self.src[s] != self.pat[p] {
            return Ok(None);
        }
        let (b, e) = (self.pat[p], self.pat[p + 1]);
        let mut cont = 1;
        let mut s = s + 1;
        while s < self.src.len() {
            if self.src[s] == e {
                cont -= 1;
                if cont == 0 {
                    return Ok(Some(s + 1));
                }
            } else if self.src[s] == b {
                cont += 1;
            }
            s += 1;
        }
        Ok(None)
    }

    fn max_expand(&mut self, s: usize, p: usize, ep: usize) -> Result<Option<usize>> {
        let mut i = 0usize;
        while s + i < self.src.len() && self.single_match(self.src[s + i], p, ep) {
            i += 1;
        }
        loop {
            if let Some(r) = self.do_match(s + i, ep + 1)? {
                return Ok(Some(r));
            }
            if i == 0 {
                return Ok(None);
            }
            i -= 1;
        }
    }

    fn min_expand(&mut self, mut s: usize, p: usize, ep: usize) -> Result<Option<usize>> {
        loop {
            if let Some(r) = self.do_match(s, ep + 1)? {
                return Ok(Some(r));
            }
            if s < self.src.len() && self.single_match(self.src[s], p, ep) {
                s += 1;
            } else {
                return Ok(None);
            }
        }
    }

    fn start_capture(&mut self, s: usize, p: usize, what: isize) -> Result<Option<usize>> {
        if self.level >= MAXCAPTURES {
            return Err(self.err("too many captures"));
        }
        self.capture[self.level] = (s, what);
        self.level += 1;
        let r = self.do_match(s, p)?;
        if r.is_none() {
            self.level -= 1;
        }
        Ok(r)
    }

    fn end_capture(&mut self, s: usize, p: usize) -> Result<Option<usize>> {
        let l = self.capture_to_close()?;
        self.capture[l].1 = (s - self.capture[l].0) as isize;
        let r = self.do_match(s, p)?;
        if r.is_none() {
            self.capture[l].1 = CAP_UNFINISHED;
        }
        Ok(r)
    }

    fn capture_to_close(&self) -> Result<usize> {
        (0..self.level)
            .rev()
            .find(|&l| self.capture[l].1 == CAP_UNFINISHED)
            .ok_or_else(|| self.err("invalid pattern capture"))
    }

    fn check_capture(&self, l: u8) -> Result<usize> {
        let l = l as isize - b'1' as isize;
        if l < 0 || l as usize >= self.level || self.capture[l as usize].1 == CAP_UNFINISHED {
            return Err(self.err("invalid capture index"));
        }
        Ok(l as usize)
    }

    fn match_capture(&self, s: usize, l: u8) -> Result<Option<usize>> {
        let l = self.check_capture(l)?;
        let (start, len) = self.capture[l];
        let len = len as usize;
        if self.src.len() - s >= len && self.src[start..start + len] == self.src[s..s + len] {
            Ok(Some(s + len))
        } else {
            Ok(None)
        }
    }

    /// `match`: the end of the match of `pat[p..]` at `src[s..]`.
    fn do_match(&mut self, mut s: usize, mut p: usize) -> Result<Option<usize>> {
        loop {
            if p == self.pat.len() {
                return Ok(Some(s));
            }
            match self.pat[p] {
                b'(' => {
                    return if self.pat.get(p + 1) == Some(&b')') {
                        self.start_capture(s, p + 2, CAP_POSITION)
                    } else {
                        self.start_capture(s, p + 1, CAP_UNFINISHED)
                    };
                }
                b')' => return self.end_capture(s, p + 1),
                L_ESC if self.pat.get(p + 1) == Some(&b'b') => match self.match_balance(s, p + 2)? {
                    Some(ns) => {
                        s = ns;
                        p += 4;
                        continue;
                    }
                    None => return Ok(None),
                },
                L_ESC if self.pat.get(p + 1) == Some(&b'f') => {
                    p += 2;
                    if self.pat.get(p) != Some(&b'[') {
                        return Err(self.err("missing '[' after '%f' in pattern"));
                    }
                    let ep = self.class_end(p)?;
                    let prev = if s == 0 { 0 } else { self.src[s - 1] };
                    let cur = self.src.get(s).copied().unwrap_or(0);
                    if self.match_bracket_class(prev, p, ep - 1) || !self.match_bracket_class(cur, p, ep - 1) {
                        return Ok(None);
                    }
                    p = ep;
                    continue;
                }
                L_ESC if self.pat.get(p + 1).is_some_and(|c| c.is_ascii_digit()) => {
                    match self.match_capture(s, self.pat[p + 1])? {
                        Some(ns) => {
                            s = ns;
                            p += 2;
                            continue;
                        }
                        None => return Ok(None),
                    }
                }
                b'$' if p + 1 == self.pat.len() => {
                    return Ok(if s == self.src.len() { Some(s) } else { None });
                }
                _ => {}
            }
            // default
            let ep = self.class_end(p)?;
            let m = s < self.src.len() && self.single_match(self.src[s], p, ep);
            match self.pat.get(ep) {
                Some(b'?') => {
                    if m && let Some(r) = self.do_match(s + 1, ep + 1)? {
                        return Ok(Some(r));
                    }
                    p = ep + 1;
                    continue;
                }
                Some(b'*') => return self.max_expand(s, p, ep),
                Some(b'+') => return if m { self.max_expand(s + 1, p, ep) } else { Ok(None) },
                Some(b'-') => return self.min_expand(s, p, ep),
                _ => {
                    if !m {
                        return Ok(None);
                    }
                    s += 1;
                    p = ep;
                }
            }
        }
    }

    /// `push_onecapture`.
    fn capture_value(&self, i: usize, s: usize, e: usize) -> Result<Value> {
        if i >= self.level {
            if i == 0 {
                return Ok(Value::Str(LuaStr::new(&self.src[s..e])));
            }
            return Err(self.err("invalid capture index"));
        }
        let (start, len) = self.capture[i];
        match len {
            CAP_UNFINISHED => Err(self.err("unfinished capture")),
            CAP_POSITION => Ok(Value::Number((start + 1) as f64)),
            l => Ok(Value::Str(LuaStr::new(&self.src[start..start + l as usize]))),
        }
    }

    /// `push_captures`: all captures, or the whole match when there are none (`whole` false: none).
    fn captures(&self, s: usize, e: usize, whole: bool) -> Result<Vec<Value>> {
        let n = if self.level == 0 && whole { 1 } else { self.level };
        (0..n).map(|i| self.capture_value(i, s, e)).collect()
    }
}

fn new_ms<'a>(src: &'a [u8], pat: &'a [u8]) -> MatchState<'a> {
    MatchState { src, pat, level: 0, capture: [(0, 0); MAXCAPTURES] }
}

/// `str_find_aux`.
fn find_aux(args: &[Value], fname: &str, find: bool) -> Result<Vec<Value>> {
    let src = check_str(args, 0, fname)?;
    let pat = check_str(args, 1, fname)?;
    let (sb, pb) = (src.as_bytes(), pat.as_bytes());
    let mut init = posrelat(opt_int(args, 2, fname, 1)?, sb.len()) - 1;
    if init < 0 {
        init = 0;
    } else if init as usize > sb.len() {
        init = sb.len() as i64;
    }
    let init = init as usize;
    let plain = args.get(3).is_some_and(|v| v.truthy());
    // The C code sees the pattern as a C string: specials after a NUL don't count.
    let cpat = &pb[..pb.iter().position(|&c| c == 0).unwrap_or(pb.len())];
    if find && (plain || !cpat.iter().any(|c| SPECIALS.contains(c))) {
        let found = if pb.is_empty() {
            Some(init)
        } else if pb.len() > sb.len() - init {
            None
        } else {
            sb[init..].windows(pb.len()).position(|w| w == pb).map(|i| i + init)
        };
        return Ok(match found {
            Some(i) => vec![Value::Number((i + 1) as f64), Value::Number((i + pb.len()) as f64)],
            None => vec![Value::Nil],
        });
    }
    let anchor = cpat.first() == Some(&b'^');
    let p0 = if anchor { 1 } else { 0 };
    let mut s1 = init;
    loop {
        let mut ms = new_ms(sb, cpat);
        if let Some(e) = ms.do_match(s1, p0)? {
            if find {
                let mut out = vec![Value::Number((s1 + 1) as f64), Value::Number(e as f64)];
                out.extend(ms.captures(0, 0, false)?);
                return Ok(out);
            }
            return ms.captures(s1, e, true);
        }
        s1 += 1;
        if s1 > sb.len() || anchor {
            break;
        }
    }
    Ok(vec![Value::Nil])
}

/// `add_s`: a replacement string with `%0`..`%9` and `%x` escapes.
fn add_s(ms: &MatchState, out: &mut Vec<u8>, s: usize, e: usize, news: &[u8]) -> Result<()> {
    let mut i = 0;
    while i < news.len() {
        if news[i] != L_ESC {
            out.push(news[i]);
        } else {
            i += 1;
            let c = news.get(i).copied().unwrap_or(0);
            if !c.is_ascii_digit() {
                if i < news.len() {
                    out.push(c);
                } else {
                    out.push(0);
                }
            } else if c == b'0' {
                out.extend_from_slice(&ms.src[s..e]);
            } else {
                let v = ms.capture_value((c - b'1') as usize, s, e)?;
                match Lua::tolstring(&v) {
                    Some(st) => out.extend_from_slice(st.as_bytes()),
                    None => return Err(LuaError::new("invalid capture index")),
                }
            }
        }
        i += 1;
    }
    Ok(())
}

fn gsub(lua: &mut Lua, fname: &str, args: Vec<Value>) -> Result<Vec<Value>> {
    let src = check_str(&args, 0, fname)?;
    let pat = check_str(&args, 1, fname)?;
    let (sb, pb) = (src.as_bytes(), pat.as_bytes());
    let max_s = opt_int(&args, 3, fname, sb.len() as i64 + 1)?;
    let pb = &pb[..pb.iter().position(|&c| c == 0).unwrap_or(pb.len())];
    let anchor = pb.first() == Some(&b'^');
    let p0 = if anchor { 1 } else { 0 };
    let repl = args.get(2).cloned().unwrap_or_default();
    if !matches!(repl, Value::Number(_) | Value::Str(_) | Value::Table(_) | Value::Function(_)) {
        return Err(arg_error(fname, 2, "string/function/table expected"));
    }
    let mut out = Vec::new();
    let mut n = 0i64;
    let mut src_i = 0usize;
    while n < max_s {
        let mut ms = new_ms(sb, pb);
        let e = ms.do_match(src_i, p0)?;
        if let Some(e) = e {
            n += 1;
            // add_value
            let value = match &repl {
                Value::Number(_) | Value::Str(_) => {
                    let news = Lua::tolstring(&repl).unwrap();
                    add_s(&ms, &mut out, src_i, e, news.as_bytes())?;
                    None
                }
                Value::Function(_) => {
                    let caps = ms.captures(src_i, e, true)?;
                    Some(lua.call(&repl, caps)?.into_iter().next().unwrap_or_default())
                }
                _ => {
                    let key = ms.capture_value(0, src_i, e)?;
                    Some(lua.index(&repl, &key)?)
                }
            };
            if let Some(v) = value {
                match &v {
                    Value::Nil | Value::Bool(false) => out.extend_from_slice(&sb[src_i..e]),
                    Value::Str(_) | Value::Number(_) => out.extend_from_slice(Lua::tolstring(&v).unwrap().as_bytes()),
                    other => {
                        return Err(LuaError::new(format!("invalid replacement value (a {})", other.type_name())));
                    }
                }
            }
            if e > src_i {
                src_i = e;
            } else if src_i < sb.len() {
                out.push(sb[src_i]);
                src_i += 1;
            } else {
                break;
            }
        } else if src_i < sb.len() {
            out.push(sb[src_i]);
            src_i += 1;
        } else {
            break;
        }
        if anchor {
            break;
        }
    }
    out.extend_from_slice(&sb[src_i..]);
    Ok(vec![s(&out), Value::Number(n as f64)])
}

fn gmatch(lua: &mut Lua, fname: &str, args: Vec<Value>) -> Result<Vec<Value>> {
    let src = check_str(&args, 0, fname)?;
    let pat = check_str(&args, 1, fname)?;
    let pos = Rc::new(Cell::new(0usize));
    let f = lua.create_function("gmatch_aux", move |_, _, _| {
        let (sb, pb) = (src.as_bytes(), pat.as_bytes());
        let mut s1 = pos.get();
        while s1 <= sb.len() {
            let mut ms = new_ms(sb, pb);
            if let Some(e) = ms.do_match(s1, 0)? {
                // An empty match advances one position.
                pos.set(if e == s1 { e + 1 } else { e });
                return ms.captures(s1, e, true);
            }
            s1 += 1;
        }
        pos.set(s1);
        Ok(vec![])
    });
    Ok(vec![Value::Function(f)])
}

// ---- format ----

struct Spec {
    left: bool,
    plus: bool,
    space: bool,
    alt: bool,
    zero: bool,
    width: usize,
    prec: Option<usize>,
}

fn pad(spec: &Spec, body: Vec<u8>, numeric: bool) -> Vec<u8> {
    if body.len() >= spec.width {
        return body;
    }
    let fill = spec.width - body.len();
    if spec.left {
        let mut b = body;
        b.extend(std::iter::repeat_n(b' ', fill));
        b
    } else if spec.zero && numeric {
        // Zeros go after the sign / 0x prefix.
        let mut split = 0;
        if matches!(body.first(), Some(b'-' | b'+' | b' ')) {
            split = 1;
        }
        if body[split..].starts_with(b"0x") || body[split..].starts_with(b"0X") {
            split += 2;
        }
        let mut b = body[..split].to_vec();
        b.extend(std::iter::repeat_n(b'0', fill));
        b.extend_from_slice(&body[split..]);
        b
    } else {
        let mut b = vec![b' '; fill];
        b.extend(body);
        b
    }
}

fn sign_prefix(spec: &Spec, neg: bool) -> &'static str {
    if neg {
        "-"
    } else if spec.plus {
        "+"
    } else if spec.space {
        " "
    } else {
        ""
    }
}

fn format(_lua: &mut Lua, fname: &str, args: Vec<Value>) -> Result<Vec<Value>> {
    let fmt = check_str(&args, 0, fname)?;
    let f = fmt.as_bytes();
    let mut out = Vec::new();
    let mut arg = 0usize;
    let mut i = 0;
    while i < f.len() {
        if f[i] != b'%' {
            out.push(f[i]);
            i += 1;
            continue;
        }
        i += 1;
        if f.get(i) == Some(&b'%') {
            out.push(b'%');
            i += 1;
            continue;
        }
        arg += 1;
        if arg >= args.len() {
            return Err(arg_error(fname, arg, "no value"));
        }
        let mut spec = Spec { left: false, plus: false, space: false, alt: false, zero: false, width: 0, prec: None };
        let start = i;
        while let Some(&c) = f.get(i) {
            match c {
                b'-' => spec.left = true,
                b'+' => spec.plus = true,
                b' ' => spec.space = true,
                b'#' => spec.alt = true,
                b'0' => spec.zero = true,
                _ => break,
            }
            i += 1;
        }
        if i - start >= 6 {
            return Err(LuaError::new("invalid format (repeated flags)"));
        }
        let digits = |i: &mut usize| {
            let mut v = 0;
            for _ in 0..2 {
                if let Some(c) = f.get(*i).filter(|c| c.is_ascii_digit()) {
                    v = v * 10 + (c - b'0') as usize;
                    *i += 1;
                }
            }
            v
        };
        spec.width = digits(&mut i);
        if f.get(i) == Some(&b'.') {
            i += 1;
            spec.prec = Some(digits(&mut i));
        }
        if f.get(i).is_some_and(|c| c.is_ascii_digit()) {
            return Err(LuaError::new("invalid format (width or precision too long)"));
        }
        let Some(&conv) = f.get(i) else {
            return Err(LuaError::new("invalid option '%' to 'format'"));
        };
        i += 1;
        let a = &args[arg];
        let body: Vec<u8> = match conv {
            b'c' => {
                let n = check_number(&args, arg, fname)?;
                pad(&spec, vec![trunc_i32(n) as u8], false)
            }
            b'd' | b'i' => {
                let n = trunc_i32(check_number(&args, arg, fname)?) as i64;
                let mut digits = n.unsigned_abs().to_string();
                if let Some(p) = spec.prec {
                    if p == 0 && n == 0 {
                        digits.clear();
                    }
                    while digits.len() < p {
                        digits.insert(0, '0');
                    }
                    spec.zero = false;
                }
                pad(&spec, format!("{}{}", sign_prefix(&spec, n < 0), digits).into_bytes(), true)
            }
            b'o' | b'u' | b'x' | b'X' => {
                let n = trunc_u32(check_number(&args, arg, fname)?);
                let mut digits = match conv {
                    b'o' => format!("{n:o}"),
                    b'u' => n.to_string(),
                    b'x' => format!("{n:x}"),
                    _ => format!("{n:X}"),
                };
                if let Some(p) = spec.prec {
                    if p == 0 && n == 0 {
                        digits.clear();
                    }
                    while digits.len() < p {
                        digits.insert(0, '0');
                    }
                    spec.zero = false;
                }
                if spec.alt && n != 0 {
                    match conv {
                        b'o' if !digits.starts_with('0') => digits.insert(0, '0'),
                        b'x' => digits.insert_str(0, "0x"),
                        b'X' => digits.insert_str(0, "0X"),
                        _ => {}
                    }
                }
                pad(&spec, digits.into_bytes(), true)
            }
            b'e' | b'E' | b'f' | b'g' | b'G' => {
                let n = check_number(&args, arg, fname)?;
                let p = spec.prec.unwrap_or(6);
                let body = match conv {
                    b'e' | b'E' => fmt_e(n, p, conv == b'E', spec.alt),
                    b'f' => fmt_f(n, p, spec.alt),
                    _ => fmt_g(n, p, conv == b'G', spec.alt),
                };
                let (neg, mag) = match body.strip_prefix('-') {
                    Some(m) => (true, m.to_string()),
                    None => (false, body),
                };
                let numeric = n.is_finite();
                pad(&spec, format!("{}{}", sign_prefix(&spec, neg), mag).into_bytes(), numeric)
            }
            b'q' => {
                let st = check_str(&args, arg, fname)?;
                let mut q = vec![b'"'];
                for &c in st.as_bytes() {
                    match c {
                        b'"' | b'\\' | b'\n' => {
                            q.push(b'\\');
                            q.push(c);
                        }
                        b'\r' => q.extend_from_slice(b"\\r"),
                        0 => q.extend_from_slice(b"\\000"),
                        _ => q.push(c),
                    }
                }
                q.push(b'"');
                q
            }
            b's' => {
                let st = match Lua::tolstring(a) {
                    Some(st) => st,
                    None => return Err(tag_error(&args, arg, fname, "string")),
                };
                let b = st.as_bytes();
                if spec.prec.is_none() && b.len() >= 100 {
                    b.to_vec()
                } else {
                    let b = &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())];
                    let b = &b[..spec.prec.map_or(b.len(), |p| p.min(b.len()))];
                    pad(&spec, b.to_vec(), false)
                }
            }
            c => {
                return Err(LuaError::new(format!("invalid option '%{}' to 'format'", c as char)));
            }
        };
        out.extend(body);
    }

    Ok(vec![s(&out)])
}

pub(crate) fn open(lua: &mut Lua) {
    let t = lua.create_table();
    lua.register_in(&t, "byte", |_, name, args| {
        let st = check_str(&args, 0, name)?;
        let l = st.len();
        let posi = posrelat(opt_int(&args, 1, name, 1)?, l);
        let pose = posrelat(opt_int(&args, 2, name, posi)?, l);
        let posi = posi.max(1);
        let pose = pose.min(l as i64);
        if posi > pose {
            return Ok(vec![]);
        }
        Ok(st.as_bytes()[posi as usize - 1..pose as usize].iter().map(|&c| Value::Number(c as f64)).collect())
    });
    lua.register_in(&t, "char", |_, name, args| {
        let mut out = Vec::with_capacity(args.len());
        for i in 0..args.len() {
            let c = check_int(&args, i, name)?;
            if !(0..=255).contains(&c) {
                return Err(arg_error(name, i, "invalid value"));
            }
            out.push(c as u8);
        }
        Ok(vec![s(&out)])
    });
    lua.register_in(&t, "find", |_, name, args| find_aux(&args, name, true));
    lua.register_in(&t, "match", |_, name, args| find_aux(&args, name, false));
    lua.register_in(&t, "format", format);
    lua.register_in(&t, "gmatch", gmatch);
    lua.register_in(&t, "gfind", gmatch);
    lua.register_in(&t, "gsub", gsub);
    lua.register_in(&t, "len", |_, name, args| Ok(vec![Value::Number(check_str(&args, 0, name)?.len() as f64)]));
    lua.register_in(&t, "lower", |_, name, args| {
        Ok(vec![s(&check_str(&args, 0, name)?.as_bytes().to_ascii_lowercase())])
    });
    lua.register_in(&t, "upper", |_, name, args| {
        Ok(vec![s(&check_str(&args, 0, name)?.as_bytes().to_ascii_uppercase())])
    });
    lua.register_in(&t, "rep", |_, name, args| {
        let st = check_str(&args, 0, name)?;
        let n = check_int(&args, 1, name)?;
        Ok(vec![s(&st.as_bytes().repeat(n.max(0) as usize))])
    });
    lua.register_in(&t, "reverse", |_, name, args| {
        let mut b = check_str(&args, 0, name)?.as_bytes().to_vec();
        b.reverse();
        Ok(vec![s(&b)])
    });
    lua.register_in(&t, "sub", |_, name, args| {
        let st = check_str(&args, 0, name)?;
        let l = st.len();
        let start = posrelat(check_int(&args, 1, name)?, l).max(1);
        let end = posrelat(opt_int(&args, 2, name, -1)?, l).min(l as i64);
        if start <= end { Ok(vec![s(&st.as_bytes()[start as usize - 1..end as usize])]) } else { Ok(vec![s(b"")]) }
    });
    lua.set_global("string", t.clone());
    let mt = lua.create_table();
    mt.set_str("__index", t);
    lua.string_mt = Some(mt);
}
