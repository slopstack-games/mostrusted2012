//! The interpreter: calls, the opcode loop (`lvm.c` `luaV_execute`), metamethod-aware operations and the
//! error texts of `ldebug.c` (variable names found by `symbexec` / `getobjname`).

use std::cell::RefCell;
use std::rc::Rc;

use crate::opcodes::*;
use crate::undump::{Proto, VARARG_ISVARARG, VARARG_NEEDSARG};
use crate::value::{Function, LuaClosure, LuaStr, UpRef, UpVal, Value};
use crate::{Frame, Lua, LuaError, MAX_CALLS, Result};

/// MAXTAGLOOP: longest `__index` / `__newindex` chain.
const MAX_TAG_LOOP: usize = 100;

fn first(v: Vec<Value>) -> Value {
    v.into_iter().next().unwrap_or_default()
}

/// `luaO_chunkid` for the message prefix.
pub(crate) fn chunk_id(source: &LuaStr) -> String {
    let s = source.to_string_lossy();
    if let Some(rest) = s.strip_prefix('=') {
        rest.chars().take(59).collect()
    } else if let Some(rest) = s.strip_prefix('@') {
        if rest.len() > 59 { format!("...{}", &rest[rest.len() - 56..]) } else { rest.to_string() }
    } else {
        let line = s.lines().next().unwrap_or("");
        let cut = line.len() < s.len() || line.len() > 45;
        let l: String = line.chars().take(45).collect();
        if cut { format!("[string \"{l}...\"]") } else { format!("[string \"{l}\"]") }
    }
}

/// `symbexec` restricted to finding the instruction that last set register `reg` before `lastpc`.
fn symbexec(p: &Proto, lastpc: usize, reg: usize) -> u32 {
    let code = &p.code;
    let mut last = code.len() - 1;
    let mut pc = 0;
    while pc < lastpc {
        let i = code[pc];
        let o = op(i);
        let a = arg_a(i);
        if sets_a(o) && a == reg {
            last = pc;
        }
        match o {
            OP_LOADNIL => {
                if a <= reg && reg <= arg_b(i) {
                    last = pc;
                }
            }
            OP_SELF => {
                if reg == a + 1 {
                    last = pc;
                }
            }
            OP_TFORLOOP => {
                if reg >= a + 2 {
                    last = pc;
                }
            }
            OP_FORLOOP | OP_FORPREP | OP_JMP => {
                let dest = pc as i64 + 1 + arg_sbx(i);
                if (pc as i64) < dest && dest <= lastpc as i64 {
                    pc = (pc as i64 + arg_sbx(i)) as usize;
                }
            }
            OP_CALL | OP_TAILCALL => {
                if reg >= a {
                    last = pc;
                }
            }
            OP_SETLIST => {
                if arg_c(i) == 0 {
                    pc += 1;
                }
            }
            OP_CLOSURE => {
                if let Some(np) = p.p.get(arg_bx(i)) {
                    pc += np.nups as usize;
                }
            }
            _ => {}
        }
        pc += 1;
    }
    code[last]
}

/// `luaF_getlocalname`: the name of the `n`-th (1-based) active local at `pc` (only in chunks with debug info).
fn local_name(p: &Proto, mut n: usize, pc: usize) -> Option<String> {
    for (name, start, end) in &p.local_names {
        if *start as usize > pc {
            break;
        }
        if pc < *end as usize {
            n -= 1;
            if n == 0 {
                return Some(name.to_string_lossy());
            }
        }
    }
    None
}

fn kname(p: &Proto, c: usize) -> String {
    if is_k(c)
        && let Some(Value::Str(s)) = p.k.get(c - BITRK)
    {
        return s.to_string_lossy();
    }
    "?".into()
}

/// `getobjname`: how register `reg` got its value at `pc` (global / field / upvalue / method / local).
pub(crate) fn obj_name(p: &Proto, pc: usize, reg: usize) -> Option<(&'static str, String)> {
    if let Some(n) = local_name(p, reg + 1, pc) {
        return Some(("local", n));
    }
    let i = symbexec(p, pc, reg);
    match op(i) {
        OP_GETGLOBAL => p.k.get(arg_bx(i)).and_then(|k| k.as_str_lossy()).map(|n| ("global", n)),
        OP_MOVE => {
            let (a, b) = (arg_a(i), arg_b(i));
            if b < a { obj_name(p, pc, b) } else { None }
        }
        OP_GETTABLE => Some(("field", kname(p, arg_c(i)))),
        OP_GETUPVAL => {
            let name = p.upvalue_names.get(arg_b(i)).map(|s| s.to_string_lossy()).unwrap_or_else(|| "?".into());
            Some(("upvalue", name))
        }
        OP_SELF => Some(("method", kname(p, arg_c(i)))),
        _ => None,
    }
}

fn arith_op(o: u32, a: f64, b: f64) -> f64 {
    match o {
        OP_ADD => a + b,
        OP_SUB => a - b,
        OP_MUL => a * b,
        OP_DIV => a / b,
        OP_MOD => a - (a / b).floor() * b,
        OP_POW => a.powf(b),
        _ => -a,
    }
}

fn arith_event(o: u32) -> &'static str {
    match o {
        OP_ADD => "__add",
        OP_SUB => "__sub",
        OP_MUL => "__mul",
        OP_DIV => "__div",
        OP_MOD => "__mod",
        OP_POW => "__pow",
        _ => "__unm",
    }
}

impl Lua {
    pub(crate) fn ensure(&mut self, n: usize) {
        if self.stack.len() < n {
            self.stack.resize(n, Value::Nil);
        }
    }

    /// The metatable field `event` of a value (raw), nil if none.
    pub(crate) fn metamethod(&self, v: &Value, event: &str) -> Value {
        let mt = match v {
            Value::Table(t) => t.metatable(),
            Value::Str(_) => self.string_mt.clone(),
            _ => None,
        };
        mt.map(|m| m.get_str(event)).unwrap_or_default()
    }

    /// `luaG_runerror`: the message with the current Lua function's `source:line:` (natives get none).
    pub(crate) fn rt_error(&self, msg: impl Into<String>) -> LuaError {
        let msg = msg.into();
        match self.frames.last() {
            Some(Frame { proto: Some(p), pc, .. }) => {
                let line = p.line_info.get(*pc).copied().unwrap_or(0);
                LuaError::plain(format!("{}:{}: {}", chunk_id(&p.source), line, msg))
            }
            _ => LuaError::plain(msg),
        }
    }

    /// `luaL_where(level)`: `source:line: ` of the function `level` calls up (0 = the running function), if
    /// it is Lua code with line info.
    pub(crate) fn where_(&self, level: usize) -> String {
        if level >= self.frames.len() {
            return String::new();
        }
        match &self.frames[self.frames.len() - 1 - level] {
            Frame { proto: Some(p), pc, .. } => {
                let line = p.line_info.get(*pc).copied().unwrap_or(0);
                if line > 0 { format!("{}:{}: ", chunk_id(&p.source), line) } else { String::new() }
            }
            _ => String::new(),
        }
    }

    fn var_info(&self, slot: usize) -> Option<(&'static str, String)> {
        let f = self.frames.last()?;
        let p = f.proto.as_ref()?;
        if slot < f.base || slot >= f.base + p.max_stack as usize {
            return None;
        }
        obj_name(p, f.pc, slot - f.base)
    }

    /// `luaG_typeerror`.
    pub(crate) fn type_error(&self, v: &Value, slot: Option<usize>, what: &str) -> LuaError {
        let t = v.type_name();
        match slot.and_then(|s| self.var_info(s)) {
            Some((kind, name)) => self.rt_error(format!("attempt to {what} {kind} '{name}' (a {t} value)")),
            None => self.rt_error(format!("attempt to {what} a {t} value")),
        }
    }

    fn order_error(&self, a: &Value, b: &Value) -> LuaError {
        let (t1, t2) = (a.type_name(), b.type_name());
        if t1 == t2 {
            self.rt_error(format!("attempt to compare two {t1} values"))
        } else {
            self.rt_error(format!("attempt to compare {t1} with {t2}"))
        }
    }

    // ---- calls ----

    /// Call `stack[func]` with the `nargs` values above it. Results end up at `stack[func..]` (exactly
    /// `nresults` of them unless it is -1); the stack is cut after them. Returns the result count.
    pub(crate) fn call_at(&mut self, func: usize, nargs: usize, nresults: i32) -> Result<usize> {
        if self.frames.len() >= MAX_CALLS {
            return Err(self.rt_error("stack overflow"));
        }
        let n = match self.stack[func].clone() {
            Value::Function(Function::Lua(cl)) => self.call_lua(cl, func, nargs)?,
            Value::Function(Function::Native(nf)) => {
                let args = self.stack[func + 1..func + 1 + nargs].to_vec();
                self.frames.push(Frame { proto: None, base: func + 1, pc: 0 });
                let mut r = (nf.f)(self, &nf.name, args);
                if let Err(e) = &mut r
                    && e.needs_where
                {
                    e.needs_where = false;
                    let w = self.where_(1);
                    if let (false, Value::Str(m)) = (w.is_empty(), &e.value) {
                        let mut b = w.into_bytes();
                        b.extend_from_slice(m.as_bytes());
                        e.value = Value::Str(LuaStr::new(&b));
                    }
                }
                self.frames.pop();
                let r = r?;
                let n = r.len();
                self.ensure(func + n);
                for (i, v) in r.into_iter().enumerate() {
                    self.stack[func + i] = v;
                }
                n
            }
            other => {
                let tm = self.metamethod(&other, "__call");
                if !matches!(tm, Value::Function(_)) {
                    return Err(self.type_error(&other, Some(func), "call"));
                }
                self.ensure(func + nargs + 2);
                for i in (func..=func + nargs).rev() {
                    self.stack[i + 1] = self.stack[i].clone();
                }
                self.stack[func] = tm;
                return self.call_at(func, nargs + 1, nresults);
            }
        };
        let n = if nresults >= 0 {
            let want = nresults as usize;
            self.ensure(func + want);
            for i in n..want {
                self.stack[func + i] = Value::Nil;
            }
            want
        } else {
            n
        };
        self.stack.truncate(func + n);
        Ok(n)
    }

    /// `luaD_precall` for a Lua function (with `adjust_varargs`), run it, move the results to `func`.
    fn call_lua(&mut self, cl: Rc<LuaClosure>, func: usize, nargs: usize) -> Result<usize> {
        let p = cl.proto.clone();
        let np = p.num_params as usize;
        let ms = p.max_stack as usize;
        let base;
        if p.is_vararg & VARARG_ISVARARG == 0 {
            base = func + 1;
            self.ensure(base + ms);
            for i in nargs.min(np)..ms {
                self.stack[base + i] = Value::Nil;
            }
        } else {
            let actual = nargs.max(np);
            self.ensure(func + 1 + actual);
            for i in nargs..np {
                self.stack[func + 1 + i] = Value::Nil;
            }
            let htab = if p.is_vararg & VARARG_NEEDSARG != 0 {
                let nextra = actual - np;
                let t = self.new_table(nextra, 1);
                for i in 0..nextra {
                    let v = self.stack[func + 1 + np + i].clone();
                    t.0.t.borrow_mut().set_int(i as i64 + 1, v);
                }
                t.set_str("n", nextra as f64);
                Some(t)
            } else {
                None
            };
            base = func + 1 + actual;
            self.ensure(base + ms + 1);
            for i in 0..np {
                let v = std::mem::take(&mut self.stack[func + 1 + i]);
                self.stack[base + i] = v;
            }
            let mut free = np;
            if let Some(t) = htab {
                self.stack[base + np] = Value::Table(t);
                free += 1;
            }
            for i in free..ms {
                self.stack[base + i] = Value::Nil;
            }
        }
        self.frames.push(Frame { proto: Some(p), base, pc: 0 });
        let r = self.execute(&cl, func, base);
        self.frames.pop();
        let (first, n) = r?;
        for i in 0..n {
            let v = std::mem::take(&mut self.stack[first + i]);
            self.stack[func + i] = v;
        }
        Ok(n)
    }

    // ---- upvalues ----

    fn find_upval(&mut self, idx: usize) -> UpRef {
        for u in &self.open_upvals {
            if matches!(*u.borrow(), UpVal::Open(i) if i == idx) {
                return u.clone();
            }
        }
        let u = Rc::new(RefCell::new(UpVal::Open(idx)));
        self.open_upvals.push(u.clone());
        u
    }

    /// `luaF_close`: close every open upvalue at or above `level`.
    pub(crate) fn close_upvals(&mut self, level: usize) {
        let stack = &self.stack;
        self.open_upvals.retain(|u| {
            let mut b = u.borrow_mut();
            if let UpVal::Open(i) = *b
                && i >= level
            {
                *b = UpVal::Closed(stack.get(i).cloned().unwrap_or_default());
                return false;
            }
            true
        });
    }

    fn get_upval(&self, u: &UpRef) -> Value {
        match &*u.borrow() {
            UpVal::Open(i) => self.stack[*i].clone(),
            UpVal::Closed(v) => v.clone(),
        }
    }

    fn set_upval(&mut self, u: &UpRef, v: Value) {
        let mut b = u.borrow_mut();
        match &mut *b {
            UpVal::Open(i) => self.stack[*i] = v,
            UpVal::Closed(c) => *c = v,
        }
    }

    // ---- metamethod-aware operations ----

    /// `luaV_gettable`. `slot` is the stack slot `t` came from (for the error message).
    pub(crate) fn gettable(&mut self, t: &Value, key: &Value, mut slot: Option<usize>) -> Result<Value> {
        let mut t = t.clone();
        for _ in 0..MAX_TAG_LOOP {
            let tm = if let Value::Table(tt) = &t {
                let v = tt.get(key);
                if !v.is_nil() {
                    return Ok(v);
                }
                let tm = tt.metatable().map(|m| m.get_str("__index")).unwrap_or_default();
                if tm.is_nil() {
                    return Ok(Value::Nil);
                }
                tm
            } else {
                let tm = self.metamethod(&t, "__index");
                if tm.is_nil() {
                    return Err(self.type_error(&t, slot, "index"));
                }
                tm
            };
            if let Value::Function(_) = tm {
                return Ok(first(self.call(&tm, vec![t, key.clone()])?));
            }
            t = tm;
            slot = None;
        }
        Err(self.rt_error("loop in gettable"))
    }

    /// `luaV_settable`.
    pub(crate) fn settable(&mut self, t: &Value, key: Value, val: Value, mut slot: Option<usize>) -> Result<()> {
        let mut t = t.clone();
        for _ in 0..MAX_TAG_LOOP {
            let tm = if let Value::Table(tt) = &t {
                let tm = tt.metatable().map(|m| m.get_str("__newindex")).unwrap_or_default();
                // luaH_set creates the key first; the metamethod only runs for an absent (nil) value.
                let stored =
                    tt.0.t
                        .borrow_mut()
                        .set_unless_newindex(key.clone(), val.clone(), !tm.is_nil())
                        .map_err(|m| self.rt_error(m))?;
                if stored {
                    return Ok(());
                }
                tm
            } else {
                let tm = self.metamethod(&t, "__newindex");
                if tm.is_nil() {
                    return Err(self.type_error(&t, slot, "index"));
                }
                tm
            };
            if let Value::Function(_) = tm {
                self.call(&tm, vec![t, key, val])?;
                return Ok(());
            }
            t = tm;
            slot = None;
        }
        Err(self.rt_error("loop in settable"))
    }

    fn arith(&mut self, o: u32, a: &Value, b: &Value, sa: Option<usize>, sb: Option<usize>) -> Result<Value> {
        if let (Some(x), Some(y)) = (Lua::tonumber(a), Lua::tonumber(b)) {
            return Ok(Value::Number(arith_op(o, x, y)));
        }
        let ev = arith_event(o);
        let mut tm = self.metamethod(a, ev);
        if tm.is_nil() {
            tm = self.metamethod(b, ev);
        }
        if tm.is_nil() {
            return Err(if Lua::tonumber(a).is_none() {
                self.type_error(a, sa, "perform arithmetic on")
            } else {
                self.type_error(b, sb, "perform arithmetic on")
            });
        }
        Ok(first(self.call(&tm, vec![a.clone(), b.clone()])?))
    }

    /// `get_compTM`: the metamethod both values share (for `__eq` / `__lt` / `__le`).
    fn comp_tm(&self, a: &Value, b: &Value, event: &str) -> Value {
        let t1 = self.metamethod(a, event);
        if t1.is_nil() {
            return t1;
        }
        let t2 = self.metamethod(b, event);
        if t1.raw_eq(&t2) { t1 } else { Value::Nil }
    }

    /// `luaV_equalval` (`==`).
    pub(crate) fn equals(&mut self, a: &Value, b: &Value) -> Result<bool> {
        match (a, b) {
            (Value::Table(x), Value::Table(y)) => {
                if Rc::ptr_eq(&x.0, &y.0) {
                    return Ok(true);
                }
                let tm = self.comp_tm(a, b, "__eq");
                if tm.is_nil() {
                    return Ok(false);
                }
                Ok(first(self.call(&tm, vec![a.clone(), b.clone()])?).truthy())
            }
            _ => Ok(a.raw_eq(b)),
        }
    }

    fn order_tm(&mut self, a: &Value, b: &Value, event: &str) -> Result<Option<bool>> {
        let t1 = self.metamethod(a, event);
        if t1.is_nil() {
            return Ok(None);
        }
        let t2 = self.metamethod(b, event);
        if !t1.raw_eq(&t2) {
            return Ok(None);
        }
        Ok(Some(first(self.call(&t1, vec![a.clone(), b.clone()])?).truthy()))
    }

    /// `luaV_lessthan`.
    pub(crate) fn less_than(&mut self, a: &Value, b: &Value) -> Result<bool> {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => Ok(x < y),
            (Value::Str(x), Value::Str(y)) => Ok(x.as_bytes() < y.as_bytes()),
            _ if a.type_name() != b.type_name() => Err(self.order_error(a, b)),
            _ => match self.order_tm(a, b, "__lt")? {
                Some(r) => Ok(r),
                None => Err(self.order_error(a, b)),
            },
        }
    }

    /// `lessequal`.
    fn less_equal(&mut self, a: &Value, b: &Value) -> Result<bool> {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => Ok(x <= y),
            (Value::Str(x), Value::Str(y)) => Ok(x.as_bytes() <= y.as_bytes()),
            _ if a.type_name() != b.type_name() => Err(self.order_error(a, b)),
            _ => {
                if let Some(r) = self.order_tm(a, b, "__le")? {
                    return Ok(r);
                }
                match self.order_tm(b, a, "__lt")? {
                    Some(r) => Ok(!r),
                    None => Err(self.order_error(a, b)),
                }
            }
        }
    }

    /// `luaV_concat` over registers `first..=last` (absolute slots); numbers are converted in place.
    fn concat(&mut self, first_slot: usize, last: usize) -> Result<()> {
        let mut top = last + 1;
        let mut total = last - first_slot + 1;
        let is_sn = |v: &Value| matches!(v, Value::Str(_) | Value::Number(_));
        loop {
            let mut n = 2;
            let x = self.stack[top - 2].clone();
            let y = self.stack[top - 1].clone();
            if !is_sn(&x) || !is_sn(&y) {
                let mut tm = self.metamethod(&x, "__concat");
                if tm.is_nil() {
                    tm = self.metamethod(&y, "__concat");
                }
                if tm.is_nil() {
                    return Err(if is_sn(&x) {
                        self.type_error(&y, Some(top - 1), "concatenate")
                    } else {
                        self.type_error(&x, Some(top - 2), "concatenate")
                    });
                }
                let r = first(self.call(&tm, vec![x, y])?);
                self.stack[top - 2] = r;
            } else {
                let ys = Lua::tolstring(&y).unwrap();
                self.stack[top - 1] = Value::Str(ys.clone());
                if ys.is_empty() {
                    let xs = Lua::tolstring(&x).unwrap();
                    self.stack[top - 2] = Value::Str(xs);
                } else {
                    n = 1;
                    while n < total && is_sn(&self.stack[top - n - 1]) {
                        n += 1;
                    }
                    let mut buf = Vec::new();
                    for i in (top - n)..top {
                        let s = Lua::tolstring(&self.stack[i]).unwrap();
                        buf.extend_from_slice(s.as_bytes());
                    }
                    self.stack[top - n] = Value::Str(LuaStr::new(&buf));
                }
            }
            total -= n - 1;
            top -= n - 1;
            if total <= 1 {
                return Ok(());
            }
        }
    }

    /// The name a call site used for the function in register `a` (for tracebacks).
    fn callee_name(p: &Proto, pc: usize, a: usize) -> String {
        match obj_name(p, pc, a) {
            Some((kind, name)) => format!("{kind} '{name}'"),
            None => "?".into(),
        }
    }

    // ---- the interpreter loop ----

    /// Run a Lua function whose frame is set up; returns where its results are and how many.
    fn execute(&mut self, cl: &Rc<LuaClosure>, func: usize, base: usize) -> Result<(usize, usize)> {
        let p = cl.proto.clone();
        let k = &p.k;
        let code = &p.code;
        let ms = p.max_stack as usize;
        let fi = self.frames.len() - 1;
        let mut pc = 0usize;
        macro_rules! r {
            ($x:expr) => {
                self.stack[base + $x]
            };
        }
        macro_rules! rk {
            ($x:expr) => {
                if is_k($x) { k[$x - BITRK].clone() } else { self.stack[base + $x].clone() }
            };
        }
        macro_rules! rk_slot {
            ($x:expr) => {
                if is_k($x) { None } else { Some(base + $x) }
            };
        }
        loop {
            let i = code[pc];
            self.frames[fi].pc = pc;
            pc += 1;
            let a = arg_a(i);
            let ra = base + a;
            match op(i) {
                OP_MOVE => r!(a) = r!(arg_b(i)).clone(),
                OP_LOADK => r!(a) = k[arg_bx(i)].clone(),
                OP_LOADBOOL => {
                    r!(a) = Value::Bool(arg_b(i) != 0);
                    if arg_c(i) != 0 {
                        pc += 1;
                    }
                }
                OP_LOADNIL => {
                    for x in a..=arg_b(i) {
                        r!(x) = Value::Nil;
                    }
                }
                OP_GETUPVAL => r!(a) = self.get_upval(&cl.upvals[arg_b(i)]),
                OP_GETGLOBAL => {
                    let g = Value::Table(self.globals.clone());
                    let v = self.gettable(&g, &k[arg_bx(i)], None)?;
                    r!(a) = v;
                }
                OP_GETTABLE => {
                    let b = arg_b(i);
                    let t = r!(b).clone();
                    let v = self.gettable(&t, &rk!(arg_c(i)), Some(base + b))?;
                    r!(a) = v;
                }
                OP_SETGLOBAL => {
                    let g = Value::Table(self.globals.clone());
                    let v = r!(a).clone();
                    self.settable(&g, k[arg_bx(i)].clone(), v, None)?;
                }
                OP_SETUPVAL => {
                    let v = r!(a).clone();
                    self.set_upval(&cl.upvals[arg_b(i)], v);
                }
                OP_SETTABLE => {
                    let t = r!(a).clone();
                    self.settable(&t, rk!(arg_b(i)), rk!(arg_c(i)), Some(ra))?;
                }
                OP_NEWTABLE => {
                    let t = self.new_table(fb2int(arg_b(i)), fb2int(arg_c(i)));
                    r!(a) = Value::Table(t);
                }
                OP_SELF => {
                    let b = arg_b(i);
                    let t = r!(b).clone();
                    r!(a + 1) = t.clone();
                    let v = self.gettable(&t, &rk!(arg_c(i)), Some(base + b))?;
                    r!(a) = v;
                }
                o @ (OP_ADD | OP_SUB | OP_MUL | OP_DIV | OP_MOD | OP_POW) => {
                    let (b, c) = (arg_b(i), arg_c(i));
                    let (x, y) = (rk!(b), rk!(c));
                    r!(a) = match (&x, &y) {
                        (Value::Number(x), Value::Number(y)) => Value::Number(arith_op(o, *x, *y)),
                        _ => self.arith(o, &x, &y, rk_slot!(b), rk_slot!(c))?,
                    };
                }
                OP_UNM => {
                    let b = arg_b(i);
                    let x = r!(b).clone();
                    r!(a) = match x {
                        Value::Number(n) => Value::Number(-n),
                        _ => self.arith(OP_UNM, &x, &x, Some(base + b), Some(base + b))?,
                    };
                }
                OP_NOT => r!(a) = Value::Bool(!r!(arg_b(i)).truthy()),
                OP_LEN => {
                    let b = arg_b(i);
                    let x = r!(b).clone();
                    r!(a) = match &x {
                        Value::Str(s) => Value::Number(s.len() as f64),
                        Value::Table(t) => Value::Number(t.len() as f64),
                        _ => {
                            let tm = self.metamethod(&x, "__len");
                            if tm.is_nil() {
                                return Err(self.type_error(&x, Some(base + b), "get length of"));
                            }
                            first(self.call(&tm, vec![x.clone(), Value::Nil])?)
                        }
                    };
                }
                OP_CONCAT => {
                    let (b, c) = (arg_b(i), arg_c(i));
                    self.concat(base + b, base + c)?;
                    r!(a) = r!(b).clone();
                }
                OP_JMP => pc = (pc as i64 + arg_sbx(i)) as usize,
                o @ (OP_EQ | OP_LT | OP_LE) => {
                    let (x, y) = (rk!(arg_b(i)), rk!(arg_c(i)));
                    let res = match o {
                        OP_EQ => self.equals(&x, &y)?,
                        OP_LT => self.less_than(&x, &y)?,
                        _ => self.less_equal(&x, &y)?,
                    };
                    if res == (a != 0) {
                        pc = (pc as i64 + arg_sbx(code[pc])) as usize;
                    }
                    pc += 1;
                }
                OP_TEST => {
                    if r!(a).truthy() == (arg_c(i) != 0) {
                        pc = (pc as i64 + arg_sbx(code[pc])) as usize;
                    }
                    pc += 1;
                }
                OP_TESTSET => {
                    let v = r!(arg_b(i)).clone();
                    if v.truthy() == (arg_c(i) != 0) {
                        r!(a) = v;
                        pc = (pc as i64 + arg_sbx(code[pc])) as usize;
                    }
                    pc += 1;
                }
                OP_CALL => {
                    let (b, c) = (arg_b(i), arg_c(i));
                    let nargs = if b != 0 { b - 1 } else { self.top - ra - 1 };
                    let is_fn = matches!(self.stack[ra], Value::Function(_));
                    let n = match self.call_at(ra, nargs, c as i32 - 1) {
                        Ok(n) => n,
                        Err(mut e) => {
                            if is_fn {
                                e.traceback.push(Self::callee_name(&p, pc - 1, a));
                            }
                            return Err(e);
                        }
                    };
                    if c == 0 {
                        self.top = ra + n;
                    }
                    self.ensure(base + ms);
                }
                OP_TAILCALL => {
                    let b = arg_b(i);
                    let nargs = if b != 0 { b - 1 } else { self.top - ra - 1 };
                    self.close_upvals(base);
                    let is_fn = matches!(self.stack[ra], Value::Function(_));
                    return match self.call_at(ra, nargs, -1) {
                        Ok(n) => Ok((ra, n)),
                        Err(mut e) => {
                            if is_fn {
                                e.traceback.push(Self::callee_name(&p, pc - 1, a));
                            }
                            Err(e)
                        }
                    };
                }
                OP_RETURN => {
                    let b = arg_b(i);
                    let n = if b != 0 { b - 1 } else { self.top - ra };
                    self.close_upvals(base);
                    return Ok((ra, n));
                }
                OP_FORLOOP => {
                    let num = |v: &Value| v.as_number().unwrap_or(f64::NAN);
                    let step = num(&r!(a + 2));
                    let idx = num(&r!(a)) + step;
                    let limit = num(&r!(a + 1));
                    if if 0.0 < step { idx <= limit } else { limit <= idx } {
                        pc = (pc as i64 + arg_sbx(i)) as usize;
                        r!(a) = Value::Number(idx);
                        r!(a + 3) = Value::Number(idx);
                    }
                }
                OP_FORPREP => {
                    let Some(init) = Lua::tonumber(&r!(a)) else {
                        return Err(self.rt_error("'for' initial value must be a number"));
                    };
                    let Some(limit) = Lua::tonumber(&r!(a + 1)) else {
                        return Err(self.rt_error("'for' limit must be a number"));
                    };
                    let Some(step) = Lua::tonumber(&r!(a + 2)) else {
                        return Err(self.rt_error("'for' step must be a number"));
                    };
                    r!(a + 1) = Value::Number(limit);
                    r!(a + 2) = Value::Number(step);
                    r!(a) = Value::Number(init - step);
                    pc = (pc as i64 + arg_sbx(i)) as usize;
                }
                OP_TFORLOOP => {
                    let cb = ra + 3;
                    self.ensure(cb + 3);
                    self.stack[cb + 2] = r!(a + 2).clone();
                    self.stack[cb + 1] = r!(a + 1).clone();
                    self.stack[cb] = r!(a).clone();
                    let is_fn = matches!(self.stack[cb], Value::Function(_));
                    if let Err(mut e) = self.call_at(cb, 2, arg_c(i) as i32) {
                        if is_fn {
                            e.traceback.push("for iterator".into());
                        }
                        return Err(e);
                    }
                    self.ensure(base + ms);
                    if !self.stack[cb].is_nil() {
                        self.stack[cb - 1] = self.stack[cb].clone();
                        pc = (pc as i64 + arg_sbx(code[pc])) as usize;
                    }
                    pc += 1;
                }
                OP_SETLIST => {
                    let mut n = arg_b(i);
                    if n == 0 {
                        n = self.top - ra - 1;
                    }
                    let mut c = arg_c(i);
                    if c == 0 {
                        c = code[pc] as usize;
                        pc += 1;
                    }
                    let Value::Table(t) = r!(a).clone() else {
                        return Err(self.rt_error("SETLIST on a non-table"));
                    };
                    let mut last = (c - 1) * LFIELDS_PER_FLUSH + n;
                    let mut tb = t.0.t.borrow_mut();
                    if last > tb.array_size() {
                        tb.resize_array(last);
                    }
                    while n > 0 {
                        tb.set_int(last as i64, self.stack[ra + n].clone());
                        last -= 1;
                        n -= 1;
                    }
                }
                OP_CLOSE => self.close_upvals(ra),
                OP_CLOSURE => {
                    let np = p.p[arg_bx(i)].clone();
                    let mut ups = Vec::with_capacity(np.nups as usize);
                    for _ in 0..np.nups {
                        let u = code[pc];
                        pc += 1;
                        if op(u) == OP_GETUPVAL {
                            ups.push(cl.upvals[arg_b(u)].clone());
                        } else {
                            ups.push(self.find_upval(base + arg_b(u)));
                        }
                    }
                    let id = self.next_id();
                    r!(a) = Value::Function(Function::Lua(Rc::new(LuaClosure { id, proto: np, upvals: ups })));
                }
                OP_VARARG => {
                    let nvar = base - func - 1 - p.num_params as usize;
                    let b = arg_b(i);
                    let want = if b == 0 {
                        self.ensure(ra + nvar);
                        self.top = ra + nvar;
                        nvar
                    } else {
                        b - 1
                    };
                    for j in 0..want {
                        self.stack[ra + j] = if j < nvar { self.stack[base - nvar + j].clone() } else { Value::Nil };
                    }
                }
                o => return Err(self.rt_error(format!("bad opcode {o}"))),
            }
        }
    }
}
