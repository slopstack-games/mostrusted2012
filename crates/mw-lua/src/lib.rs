//! A Lua 5.1 virtual machine for the game's UI scripts: the 115 precompiled chunks (`TextFile` resources in
//! `UI/SCREENS2`) that the original runs on PUC-Rio Lua 5.1.3 (docs/research-log.md, 2026-11-17).
//!
//! Pure Rust, no compiler (everything ships as bytecode), no threads: it builds for wasm32-unknown-unknown.
//! What follows the original:
//! - the bytecode format and the 38 opcodes ([`undump`], [`opcodes`]), a 32-bit build's chunks (`size_t` 4);
//! - table layout and `next` order (a port of `ltable.c` with `luaS_hash`, [`table`]);
//! - number formatting and parsing through the game's MSVC C runtime ([`number`]);
//! - the base, string, math and table libraries of 5.1.3 (message texts included); string patterns are a port
//!   of `lstrlib.c`'s matcher;
//! - runtime error messages (`?:0: attempt to call global 'X' (a nil value)`: the chunks carry no debug
//!   info, so the source is `?` and the line 0), variable names recovered by the same symbolic execution;
//! - host functions: each native is a global closure that receives its registered name, like the original's
//!   C closures with the name as upvalue (`FUN_009f86d0` / `FUN_009f7e50`).
//!
//! Model decisions (no source in the original): tables and functions hash and print by a per-state counter
//! instead of their address; the call depth limit is 200 nested calls (LUAI_MAXCCALLS; the original allows
//! deeper Lua-to-Lua recursion); memory is reference counted with the game's collector stopped
//! (`lua_gc(STOP)` in `FUN_009f86d0`) standing for "never collect" (cycles are kept until the state is
//! dropped). Not provided: coroutines, `io`, `package`, `load*` / `dofile`, `getfenv` / `setfenv` (one shared
//! environment), `string.dump`; the shipped chunks use none of them.

mod lib_base;
mod lib_math;
mod lib_string;
mod lib_table;
pub mod number;
pub mod opcodes;
pub mod table;
pub mod undump;
pub mod value;
mod vm;

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

pub use undump::Proto;
pub use value::{Function, LuaStr, Native, NativeFn, TableRef, Value};

use value::{LuaClosure, UpRef, UpVal};

/// A Lua error: the error value (usually a message string) and the functions it passed through, innermost
/// first, as the callers named them (`global 'Foo'`, `field 'Construct'`) plus what the host called.
#[derive(Clone)]
pub struct LuaError {
    pub value: Value,
    pub traceback: Vec<String>,
    /// Raised like `luaL_error` by a native: the caller's `source:line: ` is still to be prefixed.
    pub(crate) needs_where: bool,
}

impl LuaError {
    /// An error raised by a native the way `luaL_error` does: the position of the Lua code that called the
    /// native is prefixed (only with debug info; the game's chunks have none).
    pub fn new(msg: impl Into<String>) -> Self {
        LuaError { value: Value::from(msg.into()), traceback: Vec::new(), needs_where: true }
    }

    /// An error message used as is (no position added).
    pub fn plain(msg: impl Into<String>) -> Self {
        LuaError { value: Value::from(msg.into()), traceback: Vec::new(), needs_where: false }
    }

    /// An error carrying any value (`error(t)`).
    pub fn with_value(value: Value) -> Self {
        LuaError { value, traceback: Vec::new(), needs_where: false }
    }

    /// The error value as text (what `lua.c` would print).
    pub fn message(&self) -> String {
        match &self.value {
            Value::Str(s) => s.to_string_lossy(),
            Value::Number(n) => number::fmt_number(*n),
            v => format!("(error object is a {} value)", v.type_name()),
        }
    }

    /// Add the outermost frame's name (what the host called).
    pub fn context(mut self, what: impl Into<String>) -> Self {
        self.traceback.push(what.into());
        self
    }
}

impl fmt::Display for LuaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())?;
        if !self.traceback.is_empty() {
            write!(f, " (in {})", self.traceback.join(" <- "))?;
        }
        Ok(())
    }
}

impl fmt::Debug for LuaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for LuaError {}

pub type Result<T> = std::result::Result<T, LuaError>;

/// Where `print` output goes.
pub(crate) type PrintHook = Rc<dyn Fn(&str)>;

/// One active call: a Lua function (its prototype, register base and current instruction) or a native.
pub(crate) struct Frame {
    pub(crate) proto: Option<Rc<Proto>>,
    pub(crate) base: usize,
    pub(crate) pc: usize,
}

/// Maximum nesting of calls (LUAI_MAXCCALLS of 5.1).
pub(crate) const MAX_CALLS: usize = 200;

/// A Lua state.
pub struct Lua {
    pub(crate) stack: Vec<Value>,
    /// `L->top` after an instruction with a variable number of results (CALL C=0, VARARG B=0).
    pub(crate) top: usize,
    pub(crate) open_upvals: Vec<UpRef>,
    pub(crate) frames: Vec<Frame>,
    pub(crate) globals: TableRef,
    pub(crate) string_mt: Option<TableRef>,
    next_id: u32,
    /// MSVC `rand()` state (`math.random`).
    pub(crate) rand_state: u32,
    pub(crate) print_hook: Option<PrintHook>,
}

impl Default for Lua {
    fn default() -> Self {
        Self::new()
    }
}

impl Lua {
    /// A state with the libraries the game opens (`FUN_009f86d0`): base, table, string, math. (`io` and
    /// `package` are opened there too but are not provided; no shipped chunk uses them.)
    pub fn new() -> Self {
        let mut lua = Self::bare();
        lib_base::open(&mut lua);
        lib_string::open(&mut lua);
        lib_math::open(&mut lua);
        lib_table::open(&mut lua);
        lua
    }

    /// A state with no libraries.
    pub fn bare() -> Self {
        let globals = TableRef::new(1, 0, 0);
        Lua {
            stack: Vec::new(),
            top: 0,
            open_upvals: Vec::new(),
            frames: Vec::new(),
            globals,
            string_mt: None,
            next_id: 2,
            rand_state: 1,
            print_hook: None,
        }
    }

    pub(crate) fn next_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    /// A new empty table.
    pub fn create_table(&mut self) -> TableRef {
        self.new_table(0, 0)
    }

    pub(crate) fn new_table(&mut self, narray: usize, nhash: usize) -> TableRef {
        let id = self.next_id();
        TableRef::new(id, narray, nhash)
    }

    /// The global table.
    pub fn globals(&self) -> TableRef {
        self.globals.clone()
    }

    /// Raw read of a global.
    pub fn global(&self, name: &str) -> Value {
        self.globals.get_str(name)
    }

    /// Raw write of a global.
    pub fn set_global(&mut self, name: &str, v: impl Into<Value>) {
        self.globals.set_str(name, v);
    }

    /// Where `print` output goes (default: dropped, like the game's stdout).
    pub fn set_print(&mut self, f: impl Fn(&str) + 'static) {
        self.print_hook = Some(Rc::new(f));
    }

    /// A host function value. The handler gets the state, `name` and the arguments.
    pub fn create_function(
        &mut self,
        name: &str,
        f: impl Fn(&mut Lua, &str, Vec<Value>) -> Result<Vec<Value>> + 'static,
    ) -> Function {
        let id = self.next_id();
        Function::Native(Rc::new(Native { id, name: name.into(), f: Rc::new(f) }))
    }

    /// Register a host function as the global `name` (the original's native registry).
    pub fn register(&mut self, name: &str, f: impl Fn(&mut Lua, &str, Vec<Value>) -> Result<Vec<Value>> + 'static) {
        let func = self.create_function(name, f);
        self.set_global(name, func);
    }

    /// Register a host function as `table[name]`.
    pub fn register_in(
        &mut self,
        table: &TableRef,
        name: &str,
        f: impl Fn(&mut Lua, &str, Vec<Value>) -> Result<Vec<Value>> + 'static,
    ) {
        let func = self.create_function(name, f);
        table.set_str(name, func);
    }

    /// Load a precompiled chunk; returns its main function (not run yet).
    pub fn load(&mut self, chunk: &[u8]) -> Result<Function> {
        let proto = Rc::new(undump::undump(chunk)?);
        let upvals =
            (0..proto.nups).map(|_| Rc::new(RefCell::new(UpVal::Closed(Value::Nil))) as UpRef).collect::<Vec<_>>();
        let id = self.next_id();
        Ok(Function::Lua(Rc::new(LuaClosure { id, proto, upvals })))
    }

    /// Call a value with arguments; returns all results. On an error the state is unwound to where it was.
    pub fn call(&mut self, f: &Value, args: Vec<Value>) -> Result<Vec<Value>> {
        let at = self.stack.len();
        let nframes = self.frames.len();
        let saved_top = self.top;
        self.stack.push(f.clone());
        let nargs = args.len();
        self.stack.extend(args);
        let r = self.call_at(at, nargs, -1);
        self.top = saved_top;
        match r {
            Ok(n) => {
                let out: Vec<Value> = self.stack.drain(at..at + n).collect();
                self.stack.truncate(at);
                Ok(out)
            }
            Err(e) => {
                self.close_upvals(at);
                self.frames.truncate(nframes);
                self.stack.truncate(at);
                Err(e)
            }
        }
    }

    /// Call the global function `name`.
    pub fn call_global(&mut self, name: &str, args: Vec<Value>) -> Result<Vec<Value>> {
        let f = self.global(name);
        self.call(&f, args).map_err(|e| e.context(name.to_string()))
    }

    /// How the game calls a widget script's callback (`FUN_009ef230` + `FUN_009ef2e0`): look up the global
    /// table `table`, take its field `func` (both with metamethods, like `lua_getfield`), and call it if it is
    /// a function. `Ok(None)` when it is not a function (the game then skips the call). The game runs the
    /// call under `lua_pcall` and drops the error message; this returns it, named `table.func`.
    pub fn call_field(&mut self, table: &str, func: &str, args: Vec<Value>) -> Result<Option<Vec<Value>>> {
        let what = format!("{table}.{func}");
        let t = self.global(table);
        let f = self.index(&t, &Value::from(func)).map_err(|e| e.context(what.clone()))?;
        if !matches!(f, Value::Function(_)) {
            return Ok(None);
        }
        self.call(&f, args).map(Some).map_err(|e| e.context(what))
    }

    /// `t[k]` with metamethods (`lua_gettable`).
    pub fn index(&mut self, t: &Value, k: &Value) -> Result<Value> {
        self.gettable(t, k, None)
    }

    /// `t[k] = v` with metamethods (`lua_settable`).
    pub fn set_index(&mut self, t: &Value, k: Value, v: Value) -> Result<()> {
        self.settable(t, k, v, None)
    }

    /// `tostring(v)` (`__tostring` honoured).
    pub fn tostring(&mut self, v: &Value) -> Result<LuaStr> {
        lib_base::tostring(self, v)
    }

    /// `luaV_tonumber`: a number, or a string that converts to one.
    pub fn tonumber(v: &Value) -> Option<f64> {
        match v {
            Value::Number(n) => Some(*n),
            Value::Str(s) => number::str2d(s.as_bytes()),
            _ => None,
        }
    }

    /// The bytes of a string or a number converted like `lua_tolstring`.
    pub fn tolstring(v: &Value) -> Option<LuaStr> {
        match v {
            Value::Str(s) => Some(s.clone()),
            Value::Number(n) => Some(LuaStr::new(number::fmt_number(*n).as_bytes())),
            _ => None,
        }
    }
}

/// The Lua chunk inside a `TextFile` resource: a little-endian u32 length, then the bytes. `None` when the
/// text is not a precompiled chunk (the JSON files share the type).
pub fn textfile_chunk(data: &[u8]) -> Option<&[u8]> {
    let len = u32::from_le_bytes(data.get(..4)?.try_into().ok()?) as usize;
    let body = data.get(4..)?;
    let body = &body[..len.min(body.len())];
    body.starts_with(b"\x1bLua").then_some(body)
}

/// The script's name as its main chunk declares it: the first global the main function assigns a new table
/// to (`NEWTABLE` then `SETGLOBAL`); every shipped script starts so, and its resource id is the CRC-32 of the
/// lower-cased `"<name>.lua"`.
pub fn chunk_script_name(chunk: &[u8]) -> Option<String> {
    use opcodes::*;
    let p = undump::undump(chunk).ok()?;
    let code = &p.code;
    for w in code.windows(2) {
        if op(w[0]) == OP_NEWTABLE && op(w[1]) == OP_SETGLOBAL && arg_a(w[0]) == arg_a(w[1]) {
            return p.k.get(arg_bx(w[1]))?.as_str_lossy();
        }
    }
    None
}

/// Every global name a chunk reads (`GETGLOBAL`) and writes (`SETGLOBAL`), over all its functions.
pub fn chunk_globals(chunk: &[u8]) -> Result<(Vec<String>, Vec<String>)> {
    use opcodes::*;
    fn walk(p: &Proto, get: &mut Vec<String>, set: &mut Vec<String>) {
        for &i in &p.code {
            let list = match op(i) {
                OP_GETGLOBAL => &mut *get,
                OP_SETGLOBAL => &mut *set,
                _ => continue,
            };
            if let Some(name) = p.k.get(arg_bx(i)).and_then(|k| k.as_str_lossy())
                && !list.contains(&name)
            {
                list.push(name);
            }
        }
        for c in &p.p {
            walk(c, get, set);
        }
    }
    let p = undump::undump(chunk)?;
    let (mut get, mut set) = (Vec::new(), Vec::new());
    walk(&p, &mut get, &mut set);
    Ok((get, set))
}

/// Opcode histogram of a chunk (all functions).
pub fn chunk_opcodes(chunk: &[u8]) -> Result<[usize; 38]> {
    fn walk(p: &Proto, h: &mut [usize; 38]) {
        for &i in &p.code {
            h[opcodes::op(i) as usize] += 1;
        }
        for c in &p.p {
            walk(c, h);
        }
    }
    let p = undump::undump(chunk)?;
    let mut h = [0; 38];
    walk(&p, &mut h);
    Ok(h)
}
