//! The binding path language, parsed and evaluated like the original's binding system (docs/decomp.md
//! "Binding evaluator"). Bindings name a value of a native view model: `Mode.ShowLoadingScreen`,
//! `Players.LocalPlayer.Lap`, `Signals.GetSineWave[7500](MULT:8, MINUS:4)`,
//! `GeneralSequence.Triggers["BOOT"].IsTriggerSet[1]`. They appear in the JSON widget files, the Lua scripts
//! (`ResolveBoolBinding("...")`), HSM parameters (`BINDING`) and GeneSys `*Binding` fields.
//!
//! Parse once, evaluate many times. There are two parse forms, as in the original:
//! - [`Binding::parse`] (`FUN_00a50650`, 358 call sites): every segment alike, the root too. The HSM
//!   (`FUN_0076f330`, `FUN_007716c0`, `FUN_00771ea0`), Lua's `ResolveBoolBinding` / `ResolveIntBinding` /
//!   `ResolveDoubleBinding` / `ResolveStringBinding` (`FUN_0067f4e0`, `FUN_006794e0`, `FUN_00679550`,
//!   `FUN_007e7cc0`), most of the UI and the `{...}` splices use it. A failed parse gives false / 0 / "".
//! - [`Binding::parse_rooted`] (`FUN_00a503e0`, 7 call sites, e.g. `FUN_00851c80`, `FUN_008539b0` and the
//!   `{binding}` string values): the root is the text before the first `.`, taken as is, and must have a member.
//!
//! Both splice `{binding}`s in op arguments when they parse (`FUN_00a50230` → `FUN_00a4fe80`), with the values
//! of that moment: [`Binding::compile`] does that with a host, the plain parsers leave them as warnings.
//!
//! Evaluation goes through a [`Host`] that resolves paths to [`Value`]s, with the result type the caller wants:
//! [`Binding::eval_bool`], [`eval_int`](Binding::eval_int), [`eval_float`](Binding::eval_float),
//! [`eval_double`](Binding::eval_double), [`eval_string`](Binding::eval_string),
//! [`eval_text`](Binding::eval_text). As in the original, *which operators apply depends on the result type*
//! (the original converts the value with one visitor per result type, each handling the operators it knows).
//!
//! # Grammar (`FUN_00a503e0` compile, `FUN_00a48ec0` tokenizer, `FUN_00a50230` ops, `FUN_00a4a440` index)
//!
//! ```text
//! binding  = root "." segment { "." segment }          at most 6 segments, the text at most 127 bytes
//! root     = text up to the first "."                  hashed as is (no index or ops; inner form: a segment)
//! segment  = name [ "[" index "]" ] [ "(" ops ")" ]
//! index    = integer (C atol) | '"' key '"'            default 1; 1-based for property arrays
//! ops      = token { "," token }                       at most 5 per binding, spaces before a token skipped
//! token    = NAME [ ":" operand ]                      upper-cased first, at most 23 bytes, `{binding}` spliced
//! ```
//!
//! - Segments are split at `.` unless a `(` comes first: from a `(` on, the rest of the text is one segment
//!   (the tokenizer's paren counter starts at 1 and counts the opening paren again, so it never returns to
//!   0), so ops end a binding and `.` in their arguments is safe. Text after the first `)` is ignored.
//! - A token without `:` is its own operand (`NOT` → name `NOT`, operand `NOT`). Every token becomes an op
//!   record {op code or none, hash(operand), atof(operand) as f32}. Unknown names are kept: they are the text
//!   formatters' flags (`SPEED`, `DIST`, `TIME`, `PCT`, ...) and the positional arguments of `BETWEEN` /
//!   `LERP` (`BETWEEN,5,140`, `LERP,MINVALUE:0,MAXVALUE:6,MINRANGE:0,MAXRANGE:1`).
//! - Operators (`FUN_00a4da00` registers them in this order, the op code is the index): ADD MINUS MULT DIV
//!   POW FLOOR_TO CEIL_TO ROUND_TO AXES MOD MIN MAX EQUALS GTEQ LTEQ BETWEEN HASHEQUALS NOT UPPERCASE POSTFIX
//!   IS_EMPTY LERP.
//!
//! # Semantics by result type (the visitors; addresses in docs/decomp.md)
//!
//! - **Arithmetic** (ADD..MAX, `FUN_00a4b940` f64, `FUN_00a4bb10` f32, `FUN_00a4bca0` i64), all records in order:
//!   `MIN:a` clamps from above (`a < v → a`), `MAX:a` from below, `FLOOR_TO` / `CEIL_TO` / `ROUND_TO` step
//!   through a truncating int division, `MOD` = `v − trunc(v/a)·a`. The i64 form knows only ADD MINUS MULT DIV
//!   MOD MIN MAX.
//! - **Comparisons** (`FUN_00a4bea0`, on f32): the *first* EQUALS / GTEQ / LTEQ / BETWEEN / HASHEQUALS decides.
//!   EQUALS = |a − v| < 2⁻¹⁶; GTEQ = v ≥ a; LTEQ = v ≤ a; BETWEEN = a < v < b (strict) with a, b the *next two*
//!   records; HASHEQUALS = the operand's hash (as a number) equals v.
//! - **bool**: comparisons on numbers (ints and floats after the arithmetic), otherwise non-zero; a string with
//!   EQUALS compares hash(upper(first 23 bytes)) with the operand's hash, otherwise non-empty; then one NOT or
//!   IS_EMPTY inverts. A missing value is false, and only IS_EMPTY inverts it.
//! - **int**: value truncated, then the f64 arithmetic. **float / double**: value, LERP, then the arithmetic.
//!   A string `{Other.Binding}` is resolved as that binding in the float result only.
//! - **string** (HSM parameters, `FUN_00771ea0`): numbers as `%.2f` / `%d`, bools `TRUE` / `FALSE`, strings with
//!   UPPERCASE and POSTFIX; no arithmetic. **text** (Lua `ResolveStringBinding`, nested keys): arithmetic and
//!   comparisons (`true` / `false`), `%.2f` / `%d`.
//!
//! The text formatters (`SPEED`, `TIME`, `DIST`, `WITHORDINAL`, ...) belong to the text widget and are not
//! applied here; [`Binding::flags`] lists them for it.

use std::collections::HashMap;

/// Most op records a binding holds (`+0x3c` count after five 12-byte records).
pub const MAX_OPS: usize = 5;
/// Most path segments (root included; hash slots `+0x40..+0x58`).
pub const MAX_SEGMENTS: usize = 6;
/// The compile buffer: longer text is cut (`_strncat` into 128 bytes).
pub const MAX_TEXT: usize = 127;
/// An op token's buffer (24 bytes).
pub const MAX_TOKEN: usize = 23;

/// EQUALS / HASHEQUALS tolerance (`.rdata` 0x0102b880 = 2⁻¹⁶).
const EQUALS_EPS: f32 = 1.0 / 65536.0;
/// A double is true when |v| > 2⁻²³ (`.rdata` 0x0102c1a8 / 0x0102c1b0).
const DOUBLE_ZERO: f64 = 1.0 / 8_388_608.0;

/// CRC-32 (MSB first, polynomial 0x04C11DB7) table, `.rdata` 0x010e5a10.
const CRC_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = (i as u32) << 24;
        let mut k = 0;
        while k < 8 {
            c = if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04c1_1db7 } else { c << 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

/// The name hash the binding system uses everywhere (`FUN_009ed180` → `FUN_00d6a410`): 0 for "", the GeneSys
/// field hash for 4+ bytes, and for 1–3 bytes a variant that starts from the first byte, shifts every byte in
/// (the first one again) and reduces through the table by the register's *low* byte.
pub fn hash(s: &str) -> u32 {
    hash_bytes(s.as_bytes())
}

pub fn hash_bytes(b: &[u8]) -> u32 {
    match b.len() {
        0 => 0,
        1..=3 => {
            let mut r = b[0] as u32;
            for &c in b {
                r = ((r << 8) | c as u32) ^ CRC_TABLE[(r & 0xff) as usize];
            }
            r
        }
        _ => {
            let mut r = !u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
            for &c in &b[4..] {
                r = ((r << 8) | c as u32) ^ CRC_TABLE[(r >> 24) as usize];
            }
            !r
        }
    }
}

/// The operators, numbered like the original's table (`FUN_00a4da00`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum OpCode {
    Add = 0,
    Minus,
    Mult,
    Div,
    Pow,
    FloorTo,
    CeilTo,
    RoundTo,
    Axes,
    Mod,
    Min,
    Max,
    Equals,
    GtEq,
    LtEq,
    Between,
    HashEquals,
    Not,
    Uppercase,
    Postfix,
    IsEmpty,
    Lerp,
}

impl OpCode {
    pub const ALL: [OpCode; 22] = [
        Self::Add,
        Self::Minus,
        Self::Mult,
        Self::Div,
        Self::Pow,
        Self::FloorTo,
        Self::CeilTo,
        Self::RoundTo,
        Self::Axes,
        Self::Mod,
        Self::Min,
        Self::Max,
        Self::Equals,
        Self::GtEq,
        Self::LtEq,
        Self::Between,
        Self::HashEquals,
        Self::Not,
        Self::Uppercase,
        Self::Postfix,
        Self::IsEmpty,
        Self::Lerp,
    ];

    pub fn name(self) -> &'static str {
        [
            "ADD",
            "MINUS",
            "MULT",
            "DIV",
            "POW",
            "FLOOR_TO",
            "CEIL_TO",
            "ROUND_TO",
            "AXES",
            "MOD",
            "MIN",
            "MAX",
            "EQUALS",
            "GTEQ",
            "LTEQ",
            "BETWEEN",
            "HASHEQUALS",
            "NOT",
            "UPPERCASE",
            "POSTFIX",
            "IS_EMPTY",
            "LERP",
        ][self as usize]
    }

    /// Lookup by name hash, like `FUN_00a4a2b0` (the token is already upper case).
    pub fn from_hash(h: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|op| hash(op.name()) == h)
    }
}

/// The text formatter flags the UI text widget reads from the op list (`.rdata` 0x00f9e484..0x00f9e690); not
/// operators of the evaluator, listed so tools can tell them from typos.
pub const FORMAT_FLAGS: &[&str] = &[
    "TIME",
    "DATE",
    "DIST",
    "WEIGHT",
    "INT",
    "SPEED",
    "SPEED_NOWRD",
    "CUR",
    "PRICE",
    "PCT",
    "YEAR",
    "GEAR",
    "ASORDINAL",
    "WITHORDINAL",
    "RAW",
    "BOUNTY",
    "SPEEDPOINTS",
    "HOURS",
    "HOURS_STRING",
    "MED",
    "SPLIT",
    "SECS",
    "SECS_HUN",
    "SECS_HUN_NWD",
    "HRS_MINS_SECS",
    "MINS_SECS",
    "OFDAY",
    "SERVER",
    "LONG",
    "LONG_ABBR",
    "LONG_FULLWORD",
    "LONG_ROUNDED",
    "ROUNDED",
    "ROUNDED_NOWORD",
    "LONG_FLOOR",
    "FULLWORD",
    "NOWORD",
    "LONG_NOWORD",
    "LONG_NOWORD_FLOOR",
    "AUTO",
    "AUTO_LONG",
    "AUTO_ROUNDED",
    "AUTO_LONG_ROUNDED",
    "DECIMAL",
    "POS",
    "DIGITS",
    "SCREEN",
    "HSVLERP",
    "PASSWORD",
    "PRODUCTKEY",
    "PLUS",
];

/// One op record.
#[derive(Debug, Clone, PartialEq)]
pub struct Op {
    /// The operator, or `None` for a formatter flag / positional argument / unknown name.
    pub code: Option<OpCode>,
    /// The (upper-cased) name part of the token.
    pub name: String,
    /// The operand text (the whole token when it has no `:`).
    pub operand_text: String,
    /// `hash(operand_text)`.
    pub operand_hash: u32,
    /// `atof(operand_text)` as f32 (0 when it is not a number).
    pub operand: f32,
}

impl Op {
    fn parse(token: &str) -> Self {
        let (name, operand_text) = match token.find(':') {
            Some(i) => (&token[..i], &token[i + 1..]),
            None => (token, token),
        };
        Op {
            code: OpCode::from_hash(hash(name)),
            name: name.to_string(),
            operand_text: operand_text.to_string(),
            operand_hash: hash(operand_text),
            operand: atof(operand_text) as f32,
        }
    }
}

/// A segment's `[...]` index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Index {
    /// `[n]` (C `atol`), 1 without brackets, for an empty `[]`, a content over 32 bytes or an unclosed quote.
    /// Property arrays are 1-based (0 or past the count is "out of range"); providers such as `Signals` take it
    /// as an argument (`GetSineWave[7500]`).
    Pos(i32),
    /// `["text"]`: a keyed lookup by `hash(text)` (case kept).
    Key { hash: u32, text: String },
}

impl Index {
    pub fn pos(&self) -> Option<i32> {
        match self {
            Index::Pos(n) => Some(*n),
            Index::Key { .. } => None,
        }
    }
}

/// One path segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub name: String,
    pub hash: u32,
    pub index: Index,
}

impl Segment {
    fn new(name: &str, index: Index) -> Self {
        Segment { name: name.to_string(), hash: hash(name), index }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("empty binding")]
    Empty,
    /// The compile form needs `Root.Member` (no `.`: the original leaves the binding invalid).
    #[error("no member after the root")]
    NoMember,
    #[error("more than {MAX_SEGMENTS} segments")]
    TooManySegments,
    /// More than [`MAX_OPS`] tokens in all (the original's record array would overflow).
    #[error("more than {MAX_OPS} op tokens")]
    TooManyOps,
}

/// Things the original accepts silently but that probably do not do what the author meant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// The text was longer than [`MAX_TEXT`] bytes and was cut.
    TextCut,
    /// An op token was longer than [`MAX_TOKEN`] bytes and was cut.
    TokenCut(String),
    /// Text after the `)` of a segment's ops (`A.B(NOT).C`: `.C` is ignored).
    IgnoredAfterOps(String),
    /// A token that is neither an operator, a formatter flag, nor a number / LERP argument.
    UnknownOp(String),
    /// `BETWEEN:a:b` (colon form): BETWEEN reads its bounds from the next two records, so this is always false.
    BetweenColonForm,
    /// An op token still holds `{...}` (use [`Binding::compile`] to splice it).
    Splice(String),
    /// An operator that takes an operand written without `:` (`LTEQ,50`): its operand is its own name, i.e.
    /// 0, and the next token is a separate record (EQUALS / GTEQ would read it only if the operand hashed to
    /// their op code).
    NoOperand(String),
}

/// A parsed binding.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    /// The text as given.
    pub text: String,
    pub segments: Vec<Segment>,
    pub ops: Vec<Op>,
    pub warnings: Vec<Warning>,
}

/// A host value. `Object` stands for a property that holds an object or pointer (true when non-null),
/// `List(n)` for a list object with `n` items (its `_length`).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i32),
    UInt(u32),
    Int64(i64),
    UInt64(u64),
    Float(f32),
    Double(f64),
    Str(String),
    Object(bool),
    List(u32),
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}
impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Int(v)
    }
}
impl From<u32> for Value {
    fn from(v: u32) -> Self {
        Value::UInt(v)
    }
}
impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::Float(v)
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Double(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Str(v.to_string())
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Str(v)
    }
}

/// What a path lookup gives (the original's dispatcher `FUN_00a4ef40`).
#[derive(Debug, Clone, PartialEq)]
pub enum Lookup {
    Found(Value),
    /// Unknown root / member, or a null object on the way (visitor slot 25).
    Null,
    /// Index 0 or past the array's count (slot 26).
    OutOfRange,
    /// A property type the visitors cannot read (slot 27).
    Unsupported,
}

impl From<Option<Value>> for Lookup {
    fn from(v: Option<Value>) -> Self {
        v.map_or(Lookup::Null, Lookup::Found)
    }
}

/// Resolves a path (`segments[0]` is the root) to a value.
pub trait Host {
    fn lookup(&self, path: &[Segment]) -> Lookup;
}

impl<F: Fn(&[Segment]) -> Lookup> Host for F {
    fn lookup(&self, path: &[Segment]) -> Lookup {
        self(path)
    }
}

/// One root's view model (`Mode`, `Players`, `Signals`, ...): resolves the members after the root.
pub trait Provider {
    fn lookup(&self, members: &[Segment]) -> Lookup;
}

impl<F: Fn(&[Segment]) -> Lookup> Provider for F {
    fn lookup(&self, members: &[Segment]) -> Lookup {
        self(members)
    }
}

/// A [`Host`] made of root providers keyed by the root's hash, like the original's root table
/// (`FUN_00a520d0`, then the dynamic providers `FUN_00a58f70`). A trailing `_length` the provider does not
/// answer is the item count of a [`Value::List`] at the path before it (the original registers `_length`
/// members on its list types).
#[derive(Default)]
pub struct Roots<'a> {
    roots: HashMap<u32, Box<dyn Provider + 'a>>,
}

impl<'a> Roots<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, root: &str, provider: impl Provider + 'a) -> &mut Self {
        self.roots.insert(hash(root), Box::new(provider));
        self
    }
}

impl Host for Roots<'_> {
    fn lookup(&self, path: &[Segment]) -> Lookup {
        let Some((root, members)) = path.split_first() else { return Lookup::Null };
        let Some(p) = self.roots.get(&root.hash) else { return Lookup::Null };
        match p.lookup(members) {
            Lookup::Null => match members.split_last() {
                Some((last, rest)) if last.name == "_length" => match p.lookup(rest) {
                    Lookup::Found(Value::List(n)) => Lookup::Found(Value::UInt(n)),
                    _ => Lookup::Null,
                },
                _ => Lookup::Null,
            },
            r => r,
        }
    }
}

/// An evaluated value and whether the path resolved (the original's error flag `+0x24`, cleared).
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved<T> {
    pub value: T,
    pub ok: bool,
}

/// C `atof` on a prefix: spaces, sign, digits, `.`, exponent; 0 when there is no number.
pub fn atof(s: &str) -> f64 {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || (b'\t'..=b'\r').contains(&b[i])) {
        i += 1;
    }
    let start = i;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let digits = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut mantissa = i > digits;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let frac = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        mantissa |= i > frac;
    }
    if !mantissa {
        return 0.0;
    }
    let mut end = i;
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let e = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > e {
            end = j;
        }
    }
    s[start..end].trim_end_matches('.').parse().unwrap_or_else(|_| s[start..end].parse().unwrap_or(0.0))
}

/// C `atol` (spaces, sign, digits; wraps like a 32-bit long).
pub fn atol(s: &str) -> i32 {
    let s = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut v: i32 = 0;
    for c in digits.bytes().take_while(u8::is_ascii_digit) {
        v = v.wrapping_mul(10).wrapping_add((c - b'0') as i32);
    }
    if neg { v.wrapping_neg() } else { v }
}

/// x86 `cvttsd2si`: truncation, `i32::MIN` for NaN or out of range.
fn cvtt(v: f64) -> i32 {
    if v.is_nan() || v >= 2_147_483_648.0 || v < -2_147_483_648.0 { i32::MIN } else { v as i32 }
}

/// `__ftol2` to 64 bits: truncation, `i64::MIN` for NaN or out of range.
fn ftol64(v: f32) -> i64 {
    let v = v as f64;
    let lim = 2f64.powi(63);
    if v.is_nan() || v >= lim || v < -lim { i64::MIN } else { v as i64 }
}

/// Cut `s` to at most `n` bytes (on a char boundary).
fn cut(s: &str, n: usize) -> &str {
    if s.len() <= n {
        return s;
    }
    let mut i = n;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    &s[..i]
}

fn upper(s: &str) -> String {
    s.to_ascii_uppercase()
}

/// Which parser (module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// `FUN_00a50650`: every segment alike.
    Inner,
    /// `FUN_00a503e0`: root before the first `.` as is, a member needed.
    Rooted,
}

impl Binding {
    /// Parse like `FUN_00a50650` (HSM, Lua, most of the UI): `{...}` in op tokens is left in place
    /// (a [`Warning::Splice`]).
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        Self::parse_with(text, true, None)
    }

    /// Parse like `FUN_00a503e0`: the root is the text before the first `.`, taken as is; no `.` is an error.
    pub fn parse_rooted(text: &str) -> Result<Self, ParseError> {
        Self::parse_with(text, false, None)
    }

    /// Parse in either form, splicing `{binding}` in op tokens with the bound value's text (`FUN_00a4fe80`,
    /// run when the original parses the binding, so the value is the one at that time).
    pub fn compile(text: &str, form: Form, host: &dyn Host) -> Result<Self, ParseError> {
        Self::parse_with(text, form == Form::Inner, Some(host))
    }

    fn parse_with(text: &str, inner: bool, host: Option<&dyn Host>) -> Result<Self, ParseError> {
        let mut warnings = Vec::new();
        let s = cut(text, MAX_TEXT);
        if s.len() < text.len() {
            warnings.push(Warning::TextCut);
        }
        if s.is_empty() {
            return Err(ParseError::Empty);
        }
        let mut segments = Vec::new();
        let mut ops = Vec::new();
        // `pos` is the index of the delimiter before the next segment (-1: the start).
        let mut pos: isize = -1;
        if !inner {
            let dot = s.find('.').ok_or(ParseError::NoMember)?;
            segments.push(Segment::new(&s[..dot], Index::Pos(1)));
            pos = dot as isize;
        }
        loop {
            let start = (pos + 1) as usize;
            if start >= s.len() {
                break;
            }
            let rest = &s[start..];
            // FUN_00a48ec0: a `(` before the next `.` makes the rest of the text one segment.
            let seg = match (rest.find('('), rest.find('.')) {
                (Some(p), Some(d)) if p < d => rest,
                (_, Some(d)) => &rest[..d],
                (_, None) => rest,
            };
            pos = if seg.len() == rest.len() { s.len() as isize } else { (start + seg.len()) as isize };
            if seg.is_empty() {
                break;
            }
            if segments.len() >= MAX_SEGMENTS {
                return Err(ParseError::TooManySegments);
            }
            let (name, index) = parse_segment(seg, &mut ops, &mut warnings, host)?;
            segments.push(Segment::new(name, index));
        }
        if segments.is_empty() {
            return Err(ParseError::Empty);
        }
        // Formatter flags and the positional records after LERP (4) / BETWEEN (2) are legitimate unknown names
        // (the exe holds no `MINVALUE` string: LERP's arguments are read by position).
        for (i, op) in ops.iter().enumerate() {
            let bare = op.operand_text == op.name;
            match op.code {
                Some(OpCode::Between) if !bare => warnings.push(Warning::BetweenColonForm),
                Some(OpCode::Between | OpCode::Lerp | OpCode::Not | OpCode::Uppercase | OpCode::Postfix)
                | Some(OpCode::IsEmpty) => {}
                Some(_) if bare => warnings.push(Warning::NoOperand(op.name.clone())),
                _ => {}
            }
            let positional = ops[..i].iter().rev().enumerate().any(|(back, o)| match o.code {
                Some(OpCode::Lerp) => back < 4,
                Some(OpCode::Between) => back < 2,
                _ => false,
            });
            if op.code.is_none() && !positional && !FORMAT_FLAGS.contains(&op.name.as_str()) && !op.name.contains('{') {
                warnings.push(Warning::UnknownOp(op.name.clone()));
            }
        }
        Ok(Binding { text: text.to_string(), segments, ops, warnings })
    }

    /// The op record with this code, first one.
    pub fn op(&self, code: OpCode) -> Option<(usize, &Op)> {
        self.ops.iter().enumerate().find(|(_, o)| o.code == Some(code))
    }

    pub fn has(&self, code: OpCode) -> bool {
        self.op(code).is_some()
    }

    /// Tokens that are not operators (formatter flags such as `SPEED`, positional arguments), in order.
    pub fn flags(&self) -> impl Iterator<Item = &Op> {
        self.ops.iter().filter(|o| o.code.is_none())
    }

    /// The vector lanes a scalar result is written to (`AXES:XZ`, `FUN_00a4c070` with the mask table at
    /// `.bss` 0x0118a840: X Y Z W XY XZ XW YZ YW ZW XYZ XYW XZW YZW); all four without AXES or for another name.
    pub fn axes(&self) -> [bool; 4] {
        const NAMES: [&str; 14] = ["X", "Y", "Z", "W", "XY", "XZ", "XW", "YZ", "YW", "ZW", "XYZ", "XYW", "XZW", "YZW"];
        let Some((_, op)) = self.op(OpCode::Axes) else { return [true; 4] };
        match NAMES.iter().find(|n| hash(n) == op.operand_hash) {
            Some(n) => b"XYZW".map(|c| n.as_bytes().contains(&c)),
            None => [true; 4],
        }
    }

    fn lookup(&self, host: &dyn Host) -> Lookup {
        host.lookup(&self.segments)
    }

    fn rec(&self, i: usize) -> f32 {
        // Records past the count hold whatever an earlier compile left there; 0 here (model decision).
        self.ops.get(i).map_or(0.0, |o| o.operand)
    }

    /// The comparison operators on an f32 (`FUN_00a4bea0`); `None` when the binding has none.
    pub fn compare(&self, v: f32) -> Option<bool> {
        for (i, op) in self.ops.iter().enumerate() {
            let own = op.operand;
            // EQUALS / GTEQ read the next record when the operand hashes to their own op code (as the code does).
            let next_if = |code: OpCode| if op.operand_hash == code as u32 { self.rec(i + 1) } else { own };
            return Some(match op.code {
                Some(OpCode::Equals) => (next_if(OpCode::Equals) - v).abs() < EQUALS_EPS,
                Some(OpCode::GtEq) => v >= next_if(OpCode::GtEq),
                Some(OpCode::LtEq) => own >= v,
                Some(OpCode::Between) => v > self.rec(i + 1) && self.rec(i + 2) > v,
                Some(OpCode::HashEquals) => ((op.operand_hash as f64) - v as f64).abs() < EQUALS_EPS as f64,
                _ => continue,
            });
        }
        None
    }

    /// One NOT, else one IS_EMPTY, inverts a bool result (`FUN_00a4ca40`).
    fn invert(&self, b: bool) -> bool {
        if self.has(OpCode::Not) || self.has(OpCode::IsEmpty) { !b } else { b }
    }

    /// The arithmetic operators on an f64 (`FUN_00a4b940`).
    pub fn arith_f64(&self, mut v: f64) -> f64 {
        for op in &self.ops {
            let a = op.operand as f64;
            v = match op.code {
                Some(OpCode::Add) => v + a,
                Some(OpCode::Minus) => v - a,
                Some(OpCode::Mult) => v * a,
                Some(OpCode::Div) => v / a,
                Some(OpCode::Pow) => v.powf(a),
                Some(OpCode::FloorTo) => cvtt(v / a) as f64 * a,
                Some(OpCode::CeilTo) => {
                    let i = cvtt(v / a);
                    let i = if v - i as f64 * a > 0.0 { i.wrapping_add(1) } else { i };
                    i as f64 * a
                }
                Some(OpCode::RoundTo) => {
                    let i = cvtt(v / a);
                    if a * 0.5 <= v - i as f64 * a { i.wrapping_add(1) as f64 * a } else { i as f64 * a }
                }
                Some(OpCode::Mod) => v - cvtt(v / a) as f64 * a,
                Some(OpCode::Min) if a < v => a,
                Some(OpCode::Max) if v <= a => a,
                _ => v,
            };
        }
        v
    }

    /// The arithmetic operators on an f32 (`FUN_00a4bb10`).
    pub fn arith_f32(&self, mut v: f32) -> f32 {
        for op in &self.ops {
            let a = op.operand;
            let ct = |x: f32| cvtt(x as f64);
            v = match op.code {
                Some(OpCode::Add) => v + a,
                Some(OpCode::Minus) => v - a,
                Some(OpCode::Mult) => v * a,
                Some(OpCode::Div) => v / a,
                Some(OpCode::Pow) => v.powf(a),
                Some(OpCode::FloorTo) => ct(v / a) as f32 * a,
                Some(OpCode::CeilTo) => {
                    let i = ct(v / a);
                    let i = if v - i as f32 * a > 0.0 { i.wrapping_add(1) } else { i };
                    i as f32 * a
                }
                Some(OpCode::RoundTo) => {
                    let i = ct(v / a);
                    if a * 0.5 <= v - i as f32 * a { i.wrapping_add(1) as f32 * a } else { i as f32 * a }
                }
                Some(OpCode::Mod) => v - ct(v / a) as f32 * a,
                Some(OpCode::Min) if a < v => a,
                Some(OpCode::Max) if v <= a => a,
                _ => v,
            };
        }
        v
    }

    /// The arithmetic operators on an i64 (`FUN_00a4bca0`): ADD MINUS MULT DIV MOD MIN MAX with the operand
    /// truncated to an integer. Division by 0 leaves the value (the original faults; model decision).
    pub fn arith_i64(&self, mut v: i64) -> i64 {
        for op in &self.ops {
            let a = ftol64(op.operand);
            v = match op.code {
                Some(OpCode::Add) => v.wrapping_add(a),
                Some(OpCode::Minus) => v.wrapping_sub(a),
                Some(OpCode::Mult) => v.wrapping_mul(a),
                Some(OpCode::Div) if a != 0 => v.wrapping_div(a),
                Some(OpCode::Mod) if a != 0 => v.wrapping_sub(v.wrapping_div(a).wrapping_mul(a)),
                Some(OpCode::Min) if a <= v => a,
                Some(OpCode::Max) if v <= a => a,
                _ => v,
            };
        }
        v
    }

    /// LERP (`FUN_00a4b5d0`): with the first LERP record at k, the next four records' operands are
    /// a, b (value range) and c, d (output range): t = clamp((v − a) / (b − a), 0, 1), result c + (d − c)·t.
    pub fn lerp(&self, v: f32) -> f32 {
        let Some((k, _)) = self.op(OpCode::Lerp) else { return v };
        let (a, b, c, d) = (self.rec(k + 1), self.rec(k + 2), self.rec(k + 3), self.rec(k + 4));
        // `t < 0 → 0, 1 <= t → 1`; NaN stays NaN.
        let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
        (d - c) * t + c
    }

    /// As a bool (the bool visitor, vtable 0x00f98d10; HSM conditions, `ResolveBoolBinding`).
    pub fn eval_bool(&self, host: &dyn Host) -> Resolved<bool> {
        let ops = !self.ops.is_empty();
        let v = match self.lookup(host) {
            Lookup::Found(v) => v,
            _ => return Resolved { value: self.has(OpCode::IsEmpty), ok: false },
        };
        let b = match v {
            Value::Bool(b) => b,
            Value::Double(d) => self.bool_double(d, ops),
            Value::Float(f) => self.bool_double(if ops { self.arith_f32(f) } else { f } as f64, ops),
            Value::Int(i) => {
                let cmp = ops.then(|| self.compare(cvtt(self.arith_f64(i as f64)) as f32)).flatten();
                cmp.unwrap_or(i != 0)
            }
            Value::UInt(u) => self.bool_i64(u as i64, ops),
            Value::Int64(i) => self.bool_i64(i, ops),
            Value::UInt64(u) => self.bool_i64(u as i64, ops),
            Value::Str(s) => match self.op(OpCode::Equals).filter(|_| ops) {
                Some((_, eq)) => hash(&upper(cut(&s, MAX_TOKEN))) == eq.operand_hash,
                None => !s.is_empty(),
            },
            Value::Object(nonnull) => nonnull,
            Value::List(_) => true,
        };
        Resolved { value: self.invert(b), ok: true }
    }

    fn bool_double(&self, d: f64, ops: bool) -> bool {
        let cmp = ops.then(|| self.compare(d as f32)).flatten();
        // `!(-2⁻²³ <= d <= 2⁻²³)`: NaN is true.
        cmp.unwrap_or(!(-DOUBLE_ZERO..=DOUBLE_ZERO).contains(&d))
    }

    fn bool_i64(&self, i: i64, ops: bool) -> bool {
        let cmp = ops.then(|| self.compare(self.arith_i64(i) as f32)).flatten();
        cmp.unwrap_or(i != 0)
    }

    /// As an int (the int visitor, vtable 0x00fb68a0, then the f64 arithmetic: `FUN_00a4f220`).
    pub fn eval_int(&self, host: &dyn Host) -> Resolved<i32> {
        let (v, ok) = match self.lookup(host) {
            Lookup::Found(v) => (
                match v {
                    Value::Bool(b) => b as i32,
                    Value::Int(i) => i,
                    Value::UInt(u) => u as i32,
                    Value::Int64(i) => i as i32,
                    Value::UInt64(u) => u as i32,
                    Value::Float(f) => cvtt(f as f64),
                    Value::Double(d) => cvtt(d),
                    // The string and object slots leave the result as it was (0).
                    Value::Str(_) | Value::Object(_) | Value::List(_) => 0,
                },
                true,
            ),
            // Only the null slot (`FUN_00a4d970`) knows IS_EMPTY.
            Lookup::Null => (self.has(OpCode::IsEmpty) as i32, false),
            _ => (0, false),
        };
        let v = if self.ops.is_empty() { v } else { cvtt(self.arith_f64(v as f64)) };
        Resolved { value: v, ok }
    }

    /// As an f32 (the float visitor at vtable 0x00fb0e00, then the f32 arithmetic: `FUN_00a4f1d0`).
    pub fn eval_float(&self, host: &dyn Host) -> Resolved<f32> {
        let (v, ok) = self.float_value(host, 0);
        Resolved { value: self.arith_f32(v), ok }
    }

    fn float_value(&self, host: &dyn Host, depth: u32) -> (f32, bool) {
        match self.lookup(host) {
            Lookup::Found(v) => match v {
                Value::Bool(b) => (b as u8 as f32, true),
                Value::Int(i) => (self.lerp(i as f32), true),
                Value::UInt(u) => (self.lerp(u as f32), true),
                Value::Int64(i) => (self.lerp(i as f32), true),
                Value::UInt64(u) => (self.lerp(u as f32), true),
                Value::Float(f) => (self.lerp(f), true),
                Value::Double(d) => (self.lerp(d as f32), true),
                // `FUN_00a51020`: a string `{Path}` is that binding (`FUN_00a50f40`; nesting capped at 8, model).
                Value::Str(s) => {
                    let inner = s.strip_prefix('{').filter(|_| depth < 8).map(|r| cut(r, r.len().saturating_sub(1)));
                    match inner.and_then(|i| Binding::parse_rooted(i).ok()) {
                        Some(b) => (b.float_value(host, depth + 1).0, true),
                        None => ((!s.is_empty()) as u8 as f32, true),
                    }
                }
                Value::Object(_) | Value::List(_) => (0.0, false),
            },
            _ => (0.0, false),
        }
    }

    /// As an f64 (the double visitor, vtable 0x00fbcef0, then the f64 arithmetic: `FUN_00a4f1a0`).
    pub fn eval_double(&self, host: &dyn Host) -> Resolved<f64> {
        let (v, ok) = match self.lookup(host) {
            Lookup::Found(v) => match v {
                Value::Bool(b) => (b as u8 as f64, true),
                Value::Str(s) => ((!s.is_empty()) as u8 as f64, true),
                Value::Object(_) | Value::List(_) => (0.0, false),
                Value::Int(i) => (self.lerp_f64(i as f64), true),
                Value::UInt(u) => (self.lerp_f64(u as f64), true),
                Value::Int64(i) => (self.lerp_f64(i as f64), true),
                Value::UInt64(u) => (self.lerp_f64(u as f64), true),
                Value::Float(f) => (self.lerp_f64(f as f64), true),
                Value::Double(d) => (self.lerp_f64(d), true),
            },
            _ => (0.0, false),
        };
        Resolved { value: self.arith_f64(v), ok }
    }

    /// `FUN_00a4b530`: LERP on a double, computed in f32; unchanged without LERP.
    fn lerp_f64(&self, v: f64) -> f64 {
        if self.has(OpCode::Lerp) { self.lerp(v as f32) as f64 } else { v }
    }

    /// As a string (the string visitor, vtable 0x00fd8368; the HSM's parameter values, `FUN_00771ea0`): no
    /// arithmetic.
    pub fn eval_string(&self, host: &dyn Host) -> Resolved<String> {
        let v = match self.lookup(host) {
            Lookup::Found(v) => v,
            Lookup::Null => return Resolved { value: "<NULL>".into(), ok: false },
            Lookup::OutOfRange => return Resolved { value: "<INVALID ARRAY INDEX>".into(), ok: false },
            Lookup::Unsupported => return Resolved { value: "<INVALID PROPERTY BINDING TYPE>".into(), ok: false },
        };
        let s = match v {
            Value::Bool(b) => if b { "TRUE" } else { "FALSE" }.to_string(),
            Value::Double(d) => format!("{:.2}", d as f32),
            Value::Float(f) => format!("{f:.2}"),
            Value::Int(i) => i.to_string(),
            Value::UInt(u) => u.to_string(),
            Value::Int64(i) => i.to_string(),
            Value::UInt64(u) => u.to_string(),
            Value::Str(s) => {
                let mut s = if self.has(OpCode::Uppercase) { upper(&s) } else { s };
                if self.has(OpCode::Postfix) {
                    // `FUN_00a4cae0` appends the literal text at 0x0102bbb4.
                    s.push_str(":POSTFIX");
                }
                s
            }
            Value::Object(_) | Value::List(_) => {
                return Resolved { value: "<INVALID PROPERTY BINDING TYPE>".into(), ok: false };
            }
        };
        Resolved { value: s, ok: true }
    }

    /// As text (the to-text visitor, vtable 0x00fb6830; Lua `ResolveStringBinding` via `FUN_007e7cc0`, the
    /// nested keys of `FUN_00a4f410`): arithmetic, then a comparison gives `true` / `false`, else `%.2f` / `%d`;
    /// "" when missing.
    pub fn eval_text(&self, host: &dyn Host) -> Resolved<String> {
        let v = match self.lookup(host) {
            Lookup::Found(v) => v,
            _ => return Resolved { value: String::new(), ok: false },
        };
        let tf = |b: bool| if b { "true" } else { "false" }.to_string();
        let s = match v {
            Value::Bool(b) => tf(b),
            Value::Double(d) => {
                let f = self.arith_f64(d) as f32;
                self.compare(f).map_or_else(|| format!("{f:.2}"), tf)
            }
            Value::Float(f) => {
                let f = self.arith_f32(f);
                self.compare(f).map_or_else(|| format!("{f:.2}"), tf)
            }
            Value::Int(i) => {
                let i = cvtt(self.arith_f64(i as f64));
                self.compare(i as f32).map_or_else(|| i.to_string(), tf)
            }
            Value::Int64(i) => {
                let i = self.arith_i64(i);
                self.compare(i as f32).map_or_else(|| i.to_string(), tf)
            }
            Value::UInt(u) => u.to_string(),
            Value::UInt64(u) => u.to_string(),
            Value::Str(s) => s,
            Value::Object(_) | Value::List(_) => return Resolved { value: String::new(), ok: false },
        };
        Resolved { value: s, ok: true }
    }

    /// The text a `{binding}` splice inserts (the splice visitor, vtable 0x0102c0f0): no operators, `%.2f`,
    /// `%d`, `true` / `false`; "" when missing.
    pub fn eval_splice(&self, host: &dyn Host) -> String {
        match self.lookup(host) {
            Lookup::Found(v) => match v {
                Value::Bool(b) => if b { "true" } else { "false" }.to_string(),
                Value::Double(d) => format!("{:.2}", d as f32),
                Value::Float(f) => format!("{f:.2}"),
                Value::Int(i) => i.to_string(),
                Value::UInt(u) => (u as i32).to_string(),
                Value::Int64(i) => i.to_string(),
                Value::UInt64(u) => (u as i64).to_string(),
                Value::Str(s) => s,
                Value::Object(_) | Value::List(_) => String::new(),
            },
            _ => String::new(),
        }
    }
}

/// One segment (`FUN_00a50230` ops, then `FUN_00a4a440` index, then the name hashed); returns the name.
fn parse_segment<'s>(
    seg: &'s str,
    ops: &mut Vec<Op>,
    warnings: &mut Vec<Warning>,
    host: Option<&dyn Host>,
) -> Result<(&'s str, Index), ParseError> {
    let mut name = seg;
    // The search for `(` starts at index 1, as `FUN_009ee250(c, 0)` does.
    if let Some(open) = seg.get(1..).and_then(|r| r.find('(')).map(|i| i + 1)
        && let Some(close) = seg[open + 1..].find(')').map(|i| i + open + 1)
    {
        let inside = &seg[open + 1..close];
        if close + 1 < seg.len() {
            warnings.push(Warning::IgnoredAfterOps(seg[close + 1..].to_string()));
        }
        let b = inside.as_bytes();
        let mut i = 0;
        let mut count = 0;
        while i < b.len() && count < MAX_OPS {
            while b[i] == b' ' && i + 1 < b.len() {
                i += 1;
            }
            let end = inside[i..].find(',').map_or(b.len(), |e| e + i);
            let raw = &inside[i..end];
            let mut token = cut(raw, MAX_TOKEN).to_string();
            if token.len() < raw.len() {
                warnings.push(Warning::TokenCut(raw.to_string()));
            }
            if token.contains('{') {
                match host {
                    Some(h) => token = splice(&token, h),
                    None => warnings.push(Warning::Splice(token.clone())),
                }
            }
            if ops.len() >= MAX_OPS {
                return Err(ParseError::TooManyOps);
            }
            ops.push(Op::parse(&upper(&token)));
            count += 1;
            if end + 1 >= b.len() {
                break;
            }
            i = end + 1;
        }
        name = &seg[..open];
    }
    // Index (`FUN_00a4a440`): from the first `[`, content up to the first `]` after it.
    let mut index = Index::Pos(1);
    if let Some(open) = name.find('[') {
        let content = name[open + 1..].find(']').map(|c| &name[open + 1..open + 1 + c]);
        if let Some(content) = content.filter(|c| !c.is_empty() && c.len() <= 0x20) {
            index = match content.find('"') {
                None => Index::Pos(atol(content)),
                Some(q) => match content[q + 1..].find('"') {
                    Some(e) => {
                        let text = &content[q + 1..q + 1 + e];
                        Index::Key { hash: hash(text), text: text.to_string() }
                    }
                    None => Index::Pos(1),
                },
            };
        }
        name = &name[..open];
    }
    Ok((name, index))
}

/// `FUN_00a4fe80`: replace each `{inner}` (nesting counted) with the inner binding's splice text, in the
/// token's 23-byte buffer.
fn splice(token: &str, host: &dyn Host) -> String {
    let mut t = token.to_string();
    for _ in 0..8 {
        let Some(open) = t.find('{') else { break };
        let mut depth = 0;
        let mut close = None;
        for (i, c) in t[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else { break };
        let text = Binding::parse(&t[open + 1..close]).map(|b| b.eval_splice(host)).unwrap_or_default();
        t = cut(&format!("{}{}{}", &t[..open], text, &t[close + 1..]), MAX_TOKEN).to_string();
    }
    t
}

/// Qualify a widget's binding path against the widget's base path (`FUN_007cb300`): `{parent}` is the base
/// (prefixed `~`), a leading `~` makes the path absolute (the `~` is dropped), any other path is
/// `base.path`; then `#` becomes the list item index (the first `#` for 0–9, every `#` from 10 on). The
/// widget's `%name%` variables (`FUN_007c83f0`) are not substituted here.
pub fn qualify(base: &str, path: &str, item: Option<i32>) -> String {
    let mut out = if base.is_empty() || path.is_empty() {
        path.to_string()
    } else if path == "{parent}" {
        format!("~{base}")
    } else if let Some(abs) = path.strip_prefix('~') {
        abs.to_string()
    } else {
        format!("{base}.{path}")
    };
    if let Some(i) = item.filter(|&i| i != -1) {
        out =
            if (0..10).contains(&i) { out.replacen('#', &i.to_string(), 1) } else { out.replace('#', &i.to_string()) };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(v: Value) -> impl Fn(&[Segment]) -> Lookup {
        move |_: &[Segment]| Lookup::Found(v.clone())
    }

    fn b(s: &str) -> Binding {
        Binding::parse_rooted(s).unwrap()
    }

    #[test]
    fn hashes_match_the_exe() {
        // The AXES mask table (.bss 0x0118a840) holds these hashes.
        assert_eq!(hash("X"), 0x5e9f_1ee7);
        assert_eq!(hash("Y"), 0x5a5e_0251);
        assert_eq!(hash("XY"), 0x44f0_9125);
        assert_eq!(hash("XZ"), 0x44f0_9126);
        assert_eq!(hash("XYZ"), 0x7f77_f8d1);
        assert_eq!(hash("YZW"), 0xbec4_1af8);
        assert_eq!(hash(""), 0);
        assert_eq!(hash("Mass"), 0x4d61_7373);
        assert_eq!(OpCode::from_hash(hash("BETWEEN")), Some(OpCode::Between));
        assert_eq!(OpCode::from_hash(hash("MINVALUE")), None);
    }

    #[test]
    fn parses_paths() {
        let x = b("GeneralSequence.Triggers[\"BOOT\"].IsTriggerSet[2]");
        assert_eq!(x.segments.len(), 3);
        assert_eq!(x.segments[1].name, "Triggers");
        assert_eq!(x.segments[1].index, Index::Key { hash: hash("BOOT"), text: "BOOT".into() });
        assert_eq!(x.segments[2].index, Index::Pos(2));
        assert_eq!(x.segments[0].index, Index::Pos(1));
        let s = b("Signals.GetSineWave[7500](MULT:8, MINUS:4)");
        assert_eq!(s.segments[1].name, "GetSineWave");
        assert_eq!(s.segments[1].index, Index::Pos(7500));
        assert_eq!(s.ops.len(), 2);
        assert_eq!((s.ops[1].code, s.ops[1].operand), (Some(OpCode::Minus), 4.0));
        // A `(` ends the splitting: the dot in 0.98 is safe, `.C` after the ops is ignored.
        let g = b("A.B(GTEQ:0.98).C");
        assert_eq!(g.segments.len(), 2);
        assert_eq!(g.ops[0].operand, 0.98);
        assert!(matches!(g.warnings[0], Warning::IgnoredAfterOps(_)));
        // Tokens are upper-cased; a bare token is its own operand.
        let n = b("Screen.Session.NavTabName(equals:mplayer, not)");
        assert_eq!(n.ops[0].operand_hash, hash("MPLAYER"));
        assert_eq!(n.ops[1].code, Some(OpCode::Not));
        assert_eq!(n.ops[1].operand_hash, hash("NOT"));
        assert_eq!(Binding::parse_rooted("Mode"), Err(ParseError::NoMember));
        assert_eq!(Binding::parse_rooted(""), Err(ParseError::Empty));
        assert_eq!(Binding::parse_rooted("A.B.C.D.E.F.G"), Err(ParseError::TooManySegments));
        assert_eq!(Binding::parse_rooted("A.B(NOT,NOT,NOT,NOT,NOT,NOT)").unwrap().ops.len(), 5);
        // The rooted form keeps the root text as is; the inner form parses it like a member.
        assert_eq!(Binding::parse_rooted("A[2].B").unwrap().segments[0].name, "A[2]");
        assert_eq!(Binding::parse("A[2].B").unwrap().segments[0].index, Index::Pos(2));
        let inner = Binding::parse("Foo[2](NOT)").unwrap();
        assert_eq!((inner.segments[0].name.as_str(), &inner.segments[0].index), ("Foo", &Index::Pos(2)));
        assert_eq!(inner.ops[0].code, Some(OpCode::Not));
        assert_eq!(Binding::parse("A.B.C.D.E.F.G"), Err(ParseError::TooManySegments));
        assert!(b("X.Y(BETWEEN:5:140)").warnings.contains(&Warning::BetweenColonForm));
        assert!(b("X.Y(BETWEEN,5,140,NOT)").warnings.is_empty());
        assert!(b("X.Y(SPEED,DECIMAL)").warnings.is_empty());
        assert!(matches!(b("X.Y(FROB)").warnings[0], Warning::UnknownOp(_)));
    }

    #[test]
    fn comparisons() {
        let t = |s: &str, v: Value| b(s).eval_bool(&one(v)).value;
        assert!(t("Mode.IntroTime(LTEQ:3)", Value::Int(3)));
        assert!(!t("Mode.IntroTime(LTEQ:3)", Value::Int(4)));
        assert!(t("A.B(GTEQ:0.98)", Value::Float(0.98)));
        assert!(t("A.B(EQUALS:1)", Value::Double(1.000001)));
        assert!(!t("A.B(EQUALS:1)", Value::Double(1.0001)));
        // BETWEEN is strict and reads the next two records.
        assert!(t("A.DistanceToCamera(BETWEEN,5,140)", Value::Float(6.0)));
        assert!(!t("A.DistanceToCamera(BETWEEN,5,140)", Value::Float(5.0)));
        assert!(t("A.DistanceToCamera(BETWEEN,5,140,NOT)", Value::Float(5.0)));
        assert!(!t("A.DistanceToCamera(BETWEEN:5:140)", Value::Float(6.0)));
        // The first comparison decides.
        assert!(t("A.B(GTEQ:1,LTEQ:0)", Value::Int(2)));
        // HASHEQUALS compares the value with the operand's hash, but the value is an f32 by then, so a hash
        // needing more than 24 bits never matches (as in the original; no shipped binding uses it).
        let h = hash("BOOT");
        let exact = ((h as f32) as f64 - h as f64).abs() < 1e-5;
        assert_eq!(t("A.B(HASHEQUALS:BOOT)", Value::Double(h as f64)), exact);
        assert!(!t("A.B(HASHEQUALS:BOOT)", Value::UInt(h ^ 0x100)));
        // Arithmetic first for ints and floats.
        assert!(t("A.B(MINUS:4,LTEQ:0)", Value::Int(4)));
        assert!(t("A.B(ADD:1,EQUALS:2)", Value::Float(1.0)));
        // ... but not for doubles.
        assert!(!t("A.B(ADD:1,EQUALS:2)", Value::Double(1.0)));
    }

    #[test]
    fn bool_conversions() {
        let t = |s: &str, v: Value| b(s).eval_bool(&one(v));
        assert!(t("A.B", Value::Int(2)).value);
        assert!(!t("A.B", Value::Double(1e-9)).value);
        assert!(t("A.B", Value::Double(1e-6)).value);
        assert!(!t("A.B(NOT)", Value::Bool(true)).value);
        assert!(t("A.B(NOT,NOT)", Value::Bool(false)).value, "one inversion only");
        assert!(t("A.B(IS_EMPTY)", Value::Str(String::new())).value);
        assert!(!t("A.B(IS_EMPTY)", Value::Str("x".into())).value);
        assert!(t("A.B(EQUALS:mplayer)", Value::Str("MPlayer".into())).value);
        assert!(!t("A.B(EQUALS:MPLAYER, NOT)", Value::Str("mplayer".into())).value);
        // The int's own value decides the non-zero test, not the arithmetic result.
        assert!(t("A.B(MINUS:1)", Value::Int(1)).value);
        // Missing: false; NOT does not invert it, IS_EMPTY does.
        let none = |_: &[Segment]| Lookup::Null;
        assert_eq!(b("A.B(NOT)").eval_bool(&none), Resolved { value: false, ok: false });
        assert_eq!(b("A.B(IS_EMPTY)").eval_bool(&none), Resolved { value: true, ok: false });
    }

    #[test]
    fn arithmetic() {
        let x = b("A.B(MULT:246,ADD:893)");
        assert_eq!(x.arith_f64(0.5), 1016.0);
        assert_eq!(b("A.B(CEIL_TO:0.25)").arith_f64(0.3), 0.5);
        assert_eq!(b("A.B(CEIL_TO:0.25)").arith_f64(0.5), 0.5);
        assert_eq!(b("A.B(FLOOR_TO:10)").arith_f64(-15.0), -10.0, "truncates toward 0");
        assert_eq!(b("A.B(ROUND_TO:10)").arith_f64(15.0), 20.0);
        assert_eq!(b("A.B(ROUND_TO:10)").arith_f64(14.0), 10.0);
        assert_eq!(b("A.B(MOD:3)").arith_f64(7.5), 1.5);
        assert_eq!(b("A.B(MIN:10)").arith_f64(12.0), 10.0);
        assert_eq!(b("A.B(MAX:2)").arith_f64(1.0), 2.0);
        assert_eq!(b("A.B(POW:2)").arith_f32(3.0), 9.0);
        assert_eq!(b("A.B(DIV:2,MOD:4)").arith_i64(15), 3);
        assert_eq!(b("A.B(MIN:10)").arith_i64(12), 10);
        // POW is not an i64 operator.
        assert_eq!(b("A.B(POW:2)").arith_i64(3), 3);
        // int: truncated after the arithmetic.
        assert_eq!(b("A.B(MULT:0.5)").eval_int(&one(Value::Int(3))).value, 1);
        assert_eq!(b("A.B(MIN:10)").eval_float(&one(Value::Float(12.0))).value, 10.0);
    }

    #[test]
    fn lerp() {
        let l = b("Players.LocalPlayer.HeatLevelInt(LERP,MINVALUE:0,MAXVALUE:6,MINRANGE:0.0,MAXRANGE:1.0)");
        assert!(l.warnings.is_empty());
        assert_eq!(l.eval_float(&one(Value::Int(3))).value, 0.5);
        assert_eq!(l.eval_float(&one(Value::Int(9))).value, 1.0);
        assert_eq!(l.eval_double(&one(Value::Int(-1))).value, 0.0);
        // No LERP in the int result.
        assert_eq!(l.eval_int(&one(Value::Int(3))).value, 3);
    }

    #[test]
    fn strings_and_splices() {
        let s = |t: &str, v: Value| b(t).eval_string(&one(v)).value;
        assert_eq!(s("A.B", Value::Bool(true)), "TRUE");
        assert_eq!(s("A.B(MULT:2)", Value::Float(1.5)), "1.50");
        assert_eq!(s("A.B(UPPERCASE)", Value::Str("abc".into())), "ABC");
        assert_eq!(b("A.B").eval_string(&|_: &[Segment]| Lookup::OutOfRange).value, "<INVALID ARRAY INDEX>");
        assert_eq!(b("A.B(MULT:2,GTEQ:3)").eval_text(&one(Value::Float(1.5))).value, "true");
        let host = |p: &[Segment]| match p[0].name.as_str() {
            "Limit" => Lookup::Found(Value::Int(3)),
            _ => Lookup::Found(Value::Int(4)),
        };
        let c = Binding::compile("A.B(GTEQ:{Limit.X})", Form::Inner, &host).unwrap();
        assert_eq!(c.ops[0].operand, 3.0);
        assert!(c.eval_bool(&host).value);
        // A float result resolves a `{binding}` string.
        let nested = |p: &[Segment]| match p[0].name.as_str() {
            "A" => Lookup::Found(Value::Str("{C.D}".into())),
            _ => Lookup::Found(Value::Float(2.5)),
        };
        assert_eq!(b("A.B").eval_float(&nested).value, 2.5);
        assert_eq!(b("A.B").eval_double(&nested).value, 1.0, "the double result does not");
    }

    #[test]
    fn roots_and_length() {
        let mut roots = Roots::new();
        roots.insert("Widget", |m: &[Segment]| match m.iter().map(|s| s.name.as_str()).collect::<Vec<_>>()[..] {
            ["Menu", "Items"] => Lookup::Found(Value::List(4)),
            _ => Lookup::Null,
        });
        assert!(b("Widget.Menu.Items._length(GTEQ:4)").eval_bool(&roots).value);
        assert_eq!(b("Widget.Menu.Items._length").eval_int(&roots).value, 4);
        assert!(!b("Other.X").eval_bool(&roots).ok);
        assert_eq!(b("X.Y(AXES:XZ)").axes(), [true, false, true, false]);
        assert_eq!(b("X.Y").axes(), [true; 4]);
    }

    #[test]
    fn qualifies() {
        assert_eq!(qualify("Widget.List.Items[3]", "Name", None), "Widget.List.Items[3].Name");
        assert_eq!(qualify("Widget.List", "~Signals.GetIntValue[#]", Some(2)), "Signals.GetIntValue[2]");
        assert_eq!(qualify("Widget.List", "{parent}", None), "~Widget.List");
        assert_eq!(qualify("", "Widget.Tab.Tabs[#].Items", Some(12)), "Widget.Tab.Tabs[12].Items");
    }

    #[test]
    fn c_numbers() {
        assert_eq!(atof("0.98"), 0.98);
        assert_eq!(atof("5:140"), 5.0);
        assert_eq!(atof("NOT"), 0.0);
        assert_eq!(atof("-3."), -3.0);
        assert_eq!(atof("1e3x"), 1000.0);
        assert_eq!(atol(" 42]"), 42);
        assert_eq!(atol("#"), 0);
    }
}
