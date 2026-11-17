//! The game's JSON reader, as the original parses the `TextFile` (0x70) JSON resources (widget definitions in
//! `UI/SCREENS2`, see [`crate::widget`]) and the data definitions inside `HSMData`.
//!
//! Ported from the exe (docs/decomp.md "Widget definitions (JSON) and widget scripts"): lexer `FUN_009f69b0`,
//! value readers `FUN_009f7bc0` (string) / `FUN_009f7c40` (int) / `FUN_009f7ca0` (double) / `FUN_009f7d00` (bool) /
//! `FUN_009f7d60` (null), object `FUN_009f7990` + `FUN_009f8510`, array `FUN_009f8c70`, string lexer
//! `FUN_009f18d0`, number lexer `FUN_009f1aa0`, lookup `FUN_009f16b0`.
//!
//! - Value tags (`value + 4`): 0 object, 1 array, 2 string, 3 int, 4 double, 5 bool, 6 null ([`Json::tag`]).
//! - Numbers: the characters `-`, `+`, `0`–`9` are collected; a `.`, `e` or `E` makes the number a double
//!   (`strtod`), otherwise it is a 64-bit int (`_strtoi64`). So `0` is an int and `0.25` a double.
//! - Strings: escapes `\" \/ \\ \b \f \n \r \t \uXXXX` (the code point written as UTF-8); any other escape
//!   fails the parse.
//! - Objects keep their entries in file order, each with the key's hash ([`crate::genesys::field_hash`], the
//!   same CRC as GeneSys field names); lookups scan in order and return the first entry whose hash matches.
//! - Commas are skipped where they appear: a trailing comma before `}` / `]` is accepted.

use crate::genesys::field_hash;
use crate::{Error, Result};

/// A JSON value with the original's value tags.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    /// Tag 0: entries in file order.
    Object(Vec<(String, Json)>),
    /// Tag 1.
    Array(Vec<Json>),
    /// Tag 2.
    Str(String),
    /// Tag 3: numbers without `.`, `e` or `E`.
    Int(i64),
    /// Tag 4.
    Double(f64),
    /// Tag 5.
    Bool(bool),
    /// Tag 6.
    Null,
}

impl Json {
    /// Parses a document (one value; leading / trailing whitespace allowed).
    pub fn parse(text: &str) -> Result<Json> {
        let mut p = Parser { s: text.as_bytes(), i: 0 };
        let v = p.value(0)?;
        p.ws();
        if p.i < p.s.len() {
            return Err(p.err("text after the value"));
        }
        Ok(v)
    }

    /// Parses a `TextFile` resource's chunk: a u32 length, then the text.
    pub fn parse_text_file(data: &[u8]) -> Result<Json> {
        Self::parse(text_file(data)?)
    }

    /// The original's value tag (0 object … 6 null).
    pub fn tag(&self) -> u8 {
        match self {
            Json::Object(_) => 0,
            Json::Array(_) => 1,
            Json::Str(_) => 2,
            Json::Int(_) => 3,
            Json::Double(_) => 4,
            Json::Bool(_) => 5,
            Json::Null => 6,
        }
    }

    /// The first entry whose key hashes like `key` (`FUN_009f16b0`).
    pub fn get(&self, key: &str) -> Option<&Json> {
        self.get_hash(field_hash(key))
    }

    /// The first entry whose key hash is `hash`.
    pub fn get_hash(&self, hash: u32) -> Option<&Json> {
        match self {
            Json::Object(e) => e.iter().find(|(k, _)| field_hash(k) == hash).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn entries(&self) -> &[(String, Json)] {
        match self {
            Json::Object(e) => e,
            _ => &[],
        }
    }

    pub fn items(&self) -> &[Json] {
        match self {
            Json::Array(a) => a,
            _ => &[],
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        if let Json::Str(s) = self { Some(s) } else { None }
    }

    pub fn as_i64(&self) -> Option<i64> {
        if let Json::Int(i) = self { Some(*i) } else { None }
    }

    /// An int or a double.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Int(i) => Some(*i as f64),
            Json::Double(d) => Some(*d),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        if let Json::Bool(b) = self { Some(*b) } else { None }
    }

    /// The type name used in reports.
    pub fn kind_name(&self) -> &'static str {
        ["object", "array", "string", "int", "double", "bool", "null"][self.tag() as usize]
    }
}

/// The text of a `TextFile` (0x70) chunk: a u32 byte count, then the bytes (trailing NULs dropped).
pub fn text_file(data: &[u8]) -> Result<&str> {
    let len = data.get(..4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize).ok_or_else(short)?;
    let body = data.get(4..).ok_or_else(short)?;
    let body = &body[..len.min(body.len())];
    let end = body.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1);
    std::str::from_utf8(&body[..end]).map_err(|e| Error::Malformed(format!("text file: {e}")))
}

fn short() -> Error {
    Error::Malformed("text file: too short".into())
}

/// Nesting deeper than this is refused (the original's parser keeps fixed-size stacks per level).
const MAX_DEPTH: usize = 64;

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, what: &str) -> Error {
        let line = self.s[..self.i.min(self.s.len())].iter().filter(|&&b| b == b'\n').count() + 1;
        Error::Malformed(format!("json line {line}: {what}"))
    }

    /// C `isspace`: space, \t, \n, \v, \f, \r.
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }

    fn value(&mut self, depth: usize) -> Result<Json> {
        if depth > MAX_DEPTH {
            return Err(self.err("nested too deep"));
        }
        match self.peek() {
            Some(b'{') => {
                self.i += 1;
                let mut entries = Vec::new();
                loop {
                    match self.peek() {
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Object(entries));
                        }
                        Some(b'"') => {
                            self.i += 1;
                            let k = self.string()?;
                            if self.peek() != Some(b':') {
                                return Err(self.err("expected ':'"));
                            }
                            self.i += 1;
                            let v = self.value(depth + 1)?;
                            entries.push((k, v));
                            if self.peek() == Some(b',') {
                                self.i += 1;
                            }
                        }
                        _ => return Err(self.err("expected a key or '}'")),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    if self.peek() == Some(b']') {
                        self.i += 1;
                        return Ok(Json::Array(items));
                    }
                    items.push(self.value(depth + 1)?);
                    if self.peek() == Some(b',') {
                        self.i += 1;
                    }
                }
            }
            Some(b'"') => {
                self.i += 1;
                self.string().map(Json::Str)
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(b't') => self.word("true", Json::Bool(true)),
            Some(b'f') => self.word("false", Json::Bool(false)),
            Some(b'n') => self.word("null", Json::Null),
            _ => Err(self.err("expected a value")),
        }
    }

    fn word(&mut self, w: &str, v: Json) -> Result<Json> {
        if self.s[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Ok(v)
        } else {
            Err(self.err("bad literal"))
        }
    }

    /// `FUN_009f1aa0`.
    fn number(&mut self) -> Result<Json> {
        let start = self.i;
        let mut int = true;
        while let Some(&c) = self.s.get(self.i) {
            match c {
                b'-' | b'+' | b'0'..=b'9' => {}
                b'.' | b'e' | b'E' => int = false,
                _ => break,
            }
            self.i += 1;
        }
        let t = std::str::from_utf8(&self.s[start..self.i]).unwrap_or_default();
        if int {
            // `_strtoi64` reads the longest valid prefix.
            let end = t.char_indices().skip(1).find(|&(_, c)| !c.is_ascii_digit()).map_or(t.len(), |(i, _)| i);
            Ok(Json::Int(t[..end].parse().unwrap_or(0)))
        } else {
            t.parse().map(Json::Double).map_err(|_| self.err("bad number"))
        }
    }

    /// `FUN_009f18d0`, after the opening quote.
    fn string(&mut self) -> Result<String> {
        let mut out = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.i) else { return Err(self.err("unterminated string")) };
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.s.get(self.i) else { return Err(self.err("unterminated string")) };
                    self.i += 1;
                    match e {
                        b'"' | b'/' | b'\\' => out.push(e),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let hex = self.s.get(self.i..self.i + 4).ok_or_else(|| self.err("short \\u"))?;
                            let cp = u32::from_str_radix(std::str::from_utf8(hex).unwrap_or(""), 16).unwrap_or(0);
                            self.i += 4;
                            let ch = char::from_u32(cp).unwrap_or('\u{fffd}');
                            out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
                _ => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| self.err("string is not UTF-8"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_and_order() {
        let j = Json::parse("{\"b\":\t0, \"a\": 0.25, \"c\": [true, null, \"x\\u00e9\",], \"b\": 1,}").unwrap();
        let e = j.entries();
        assert_eq!(e[0], ("b".into(), Json::Int(0)));
        assert_eq!(e[1], ("a".into(), Json::Double(0.25)));
        assert_eq!(e[2].1.items(), &[Json::Bool(true), Json::Null, Json::Str("xé".into())]);
        // The first entry wins.
        assert_eq!(j.get("b"), Some(&Json::Int(0)));
        assert_eq!(Json::parse("-60").unwrap(), Json::Int(-60));
        assert_eq!(Json::parse("1e3").unwrap(), Json::Double(1000.0));
        assert!(Json::parse("{\"a\" 1}").is_err());
        assert!(Json::parse("\"\\q\"").is_err());
    }

    #[test]
    fn text_file_prefix() {
        let mut d = 7u32.to_le_bytes().to_vec();
        d.extend_from_slice(b"{\"a\":1}\0\0");
        assert_eq!(Json::parse_text_file(&d).unwrap().get("a"), Some(&Json::Int(1)));
    }
}
