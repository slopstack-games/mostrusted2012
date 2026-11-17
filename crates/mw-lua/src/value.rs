//! Lua values: nil, booleans, numbers (doubles), byte strings, tables, functions and light userdata (the only
//! userdata the game's host pushes: the script object pointer given to `Construct`).

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::table::Table;
use crate::undump::Proto;
use crate::{Lua, LuaError};

/// A Lua value. Cloning is cheap (reference counted).
#[derive(Clone, Default)]
pub enum Value {
    #[default]
    Nil,
    Bool(bool),
    Number(f64),
    Str(LuaStr),
    Table(TableRef),
    Function(Function),
    /// `lua_pushlightuserdata`: an opaque host value (the original pushes the script object's address).
    LightUserdata(usize),
}

impl Value {
    /// Lua truthiness: only `nil` and `false` are false.
    pub fn truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }

    /// The name `type()` returns.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::Str(_) => "string",
            Value::Table(_) => "table",
            Value::Function(_) => "function",
            Value::LightUserdata(_) => "userdata",
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// The bytes of a string value (no number coercion).
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Str(s) => Some(s.as_bytes()),
            _ => None,
        }
    }

    /// The string value as UTF-8 (lossy), without number coercion.
    pub fn as_str_lossy(&self) -> Option<String> {
        self.as_bytes().map(|b| String::from_utf8_lossy(b).into_owned())
    }

    pub fn as_table(&self) -> Option<&TableRef> {
        match self {
            Value::Table(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_function(&self) -> Option<&Function> {
        match self {
            Value::Function(f) => Some(f),
            _ => None,
        }
    }

    /// Primitive equality (`rawequal`): no metamethods.
    pub fn raw_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Table(a), Value::Table(b)) => Rc::ptr_eq(&a.0, &b.0),
            (Value::Function(a), Value::Function(b)) => a.id() == b.id(),
            (Value::LightUserdata(a), Value::LightUserdata(b)) => a == b,
            _ => false,
        }
    }

    /// A short readable form for logs (strings quoted, numbers as Lua prints them).
    pub fn display(&self) -> String {
        match self {
            Value::Nil => "nil".into(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => crate::number::fmt_number(*n),
            Value::Str(s) => format!("{:?}", String::from_utf8_lossy(s.as_bytes())),
            Value::Table(t) => format!("table#{}", t.id()),
            Value::Function(f) => match f {
                Function::Native(n) => format!("native:{}", n.name),
                Function::Lua(c) => format!("function#{}", c.id),
            },
            Value::LightUserdata(p) => format!("userdata:{p:#x}"),
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display())
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}
impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Number(n)
    }
}
impl From<i32> for Value {
    fn from(n: i32) -> Self {
        Value::Number(n as f64)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(LuaStr::new(s.as_bytes()))
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Str(LuaStr::new(s.as_bytes()))
    }
}
impl From<&[u8]> for Value {
    fn from(s: &[u8]) -> Self {
        Value::Str(LuaStr::new(s))
    }
}
impl From<LuaStr> for Value {
    fn from(s: LuaStr) -> Self {
        Value::Str(s)
    }
}
impl From<TableRef> for Value {
    fn from(t: TableRef) -> Self {
        Value::Table(t)
    }
}
impl From<Function> for Value {
    fn from(f: Function) -> Self {
        Value::Function(f)
    }
}

/// An immutable Lua byte string with its PUC-Rio 5.1 hash (`luaS_hash`), which places it in a table's hash
/// part exactly where the original puts it, so `next` / `pairs` visit keys in the original's order.
#[derive(Clone)]
pub struct LuaStr(Rc<StrInner>);

struct StrInner {
    hash: u32,
    bytes: Box<[u8]>,
}

impl LuaStr {
    pub fn new(bytes: &[u8]) -> Self {
        LuaStr(Rc::new(StrInner { hash: str_hash(bytes), bytes: bytes.into() }))
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0.bytes
    }
    pub fn len(&self) -> usize {
        self.0.bytes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.bytes.is_empty()
    }
    pub fn hash(&self) -> u32 {
        self.0.hash
    }
    pub fn to_string_lossy(&self) -> String {
        String::from_utf8_lossy(&self.0.bytes).into_owned()
    }
}

impl PartialEq for LuaStr {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0) || (self.0.hash == other.0.hash && self.0.bytes == other.0.bytes)
    }
}

impl fmt::Debug for LuaStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.to_string_lossy())
    }
}

/// `luaS_hash` (lstring.c, 5.1.3): seeded with the length (a 32-bit `size_t` in the original), long strings
/// hash every `(len >> 5) + 1`-th byte from the end.
pub fn str_hash(s: &[u8]) -> u32 {
    let l = s.len();
    let mut h = l as u32;
    let step = (l >> 5) + 1;
    let mut l1 = l;
    while l1 >= step {
        h ^= (h << 5).wrapping_add(h >> 2).wrapping_add(s[l1 - 1] as u32);
        l1 -= step;
    }
    h
}

/// A reference to a table. Each table has a state-unique id, used where the original hashes or prints its
/// address (deterministic here, an address there).
#[derive(Clone)]
pub struct TableRef(pub(crate) Rc<TableBox>);

pub(crate) struct TableBox {
    pub(crate) id: u32,
    pub(crate) t: RefCell<Table>,
}

impl TableRef {
    pub(crate) fn new(id: u32, narray: usize, nhash: usize) -> Self {
        TableRef(Rc::new(TableBox { id, t: RefCell::new(Table::new(narray, nhash)) }))
    }
    pub fn id(&self) -> u32 {
        self.0.id
    }
    /// Raw read (`rawget`).
    pub fn get(&self, key: &Value) -> Value {
        self.0.t.borrow().get(key)
    }
    /// Raw read of a string key.
    pub fn get_str(&self, key: &str) -> Value {
        self.get(&Value::from(key))
    }
    /// Raw write (`rawset`). Fails for a nil or NaN key.
    pub fn set(&self, key: Value, val: Value) -> Result<(), LuaError> {
        self.0.t.borrow_mut().set(key, val).map_err(LuaError::plain)
    }
    /// Raw write of a string key.
    pub fn set_str(&self, key: &str, val: impl Into<Value>) {
        let _ = self.set(Value::from(key), val.into());
    }
    /// `#t` (`luaH_getn`).
    pub fn len(&self) -> usize {
        self.0.t.borrow().getn()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// `next(t, key)`, in the original's order. `Err` for a key not in the table.
    pub fn next(&self, key: &Value) -> Result<Option<(Value, Value)>, LuaError> {
        self.0.t.borrow().next(key).map_err(|_| LuaError::new("invalid key to 'next'"))
    }
    /// All key / value pairs in `next` order.
    pub fn pairs(&self) -> Vec<(Value, Value)> {
        let t = self.0.t.borrow();
        let mut out = Vec::new();
        let mut k = Value::Nil;
        while let Ok(Some((nk, v))) = t.next(&k) {
            out.push((nk.clone(), v));
            k = nk;
        }
        out
    }
    pub fn metatable(&self) -> Option<TableRef> {
        self.0.t.borrow().metatable.clone()
    }
    pub fn set_metatable(&self, mt: Option<TableRef>) {
        self.0.t.borrow_mut().metatable = mt;
    }
}

impl fmt::Debug for TableRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "table#{}", self.0.id)
    }
}

/// A host function: called with the state, its registered name (the original registers every native as a C
/// closure whose one upvalue is its name) and the arguments; returns the results.
pub type NativeFn = dyn Fn(&mut Lua, &str, Vec<Value>) -> Result<Vec<Value>, LuaError>;

pub struct Native {
    pub(crate) id: u32,
    pub name: Rc<str>,
    pub(crate) f: Rc<NativeFn>,
}

pub struct LuaClosure {
    pub(crate) id: u32,
    pub(crate) proto: Rc<Proto>,
    pub(crate) upvals: Vec<UpRef>,
}

pub(crate) type UpRef = Rc<RefCell<UpVal>>;

/// An upvalue: open while the variable still lives in a stack slot, closed (holding the value) after.
pub(crate) enum UpVal {
    Open(usize),
    Closed(Value),
}

#[derive(Clone)]
pub enum Function {
    Lua(Rc<LuaClosure>),
    Native(Rc<Native>),
}

impl Function {
    pub fn id(&self) -> u32 {
        match self {
            Function::Lua(c) => c.id,
            Function::Native(n) => n.id,
        }
    }
    /// The registered name of a native, `None` for a Lua function.
    pub fn native_name(&self) -> Option<&str> {
        match self {
            Function::Native(n) => Some(&n.name),
            Function::Lua(_) => None,
        }
    }
}
