//! Precompiled chunk loader, a port of `lundump.c` (Lua 5.1).
//!
//! Header (12 bytes): `"\x1bLua"`, version `0x51`, format 0, endianness 1 (little), `sizeof(int)` 4,
//! `sizeof(size_t)`, `sizeof(Instruction)` 4, `sizeof(lua_Number)` 8, integral flag 0. The game's chunks are
//! from a 32-bit build (`size_t` = 4); 8 is accepted too so that chunks from a 64-bit `luac5.1` load in tests.
//!
//! Function (recursive): source (string), line defined, last line defined (int), upvalue count, parameter
//! count, vararg flags, max stack size (bytes), code (int count + u32 each), constants (int count; each a type
//! byte: 0 nil, 1 boolean + byte, 3 number (f64), 4 string), nested functions (int count), then debug info:
//! line info (int count + ints), locals (int count; name, start pc, end pc), upvalue names (int count +
//! strings). Strings are a `size_t` length that counts a trailing NUL (0 = no string). The shipped chunks
//! carry no debug info (stripped); a missing source is the parent's, `"=?"` at the top.

use std::rc::Rc;

use crate::LuaError;
use crate::value::{LuaStr, Value};

/// A function prototype.
pub struct Proto {
    pub source: LuaStr,
    pub line_defined: i32,
    pub nups: u8,
    pub num_params: u8,
    pub is_vararg: u8,
    pub max_stack: u8,
    pub code: Vec<u32>,
    pub k: Vec<Value>,
    pub p: Vec<Rc<Proto>>,
    pub line_info: Vec<i32>,
    pub upvalue_names: Vec<LuaStr>,
    pub local_names: Vec<(LuaStr, i32, i32)>,
}

pub const VARARG_HASARG: u8 = 1;
pub const VARARG_ISVARARG: u8 = 2;
pub const VARARG_NEEDSARG: u8 = 4;

struct Loader<'a> {
    data: &'a [u8],
    pos: usize,
    size_t: usize,
}

fn bad(what: &str) -> LuaError {
    LuaError::plain(format!("binary string: {what} in precompiled chunk"))
}

impl Loader<'_> {
    fn block(&mut self, n: usize) -> Result<&[u8], LuaError> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len()).ok_or_else(|| bad("unexpected end"))?;
        let b = &self.data[self.pos..end];
        self.pos = end;
        Ok(b)
    }
    fn byte(&mut self) -> Result<u8, LuaError> {
        Ok(self.block(1)?[0])
    }
    fn int(&mut self) -> Result<i32, LuaError> {
        let b = self.block(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn count(&mut self) -> Result<usize, LuaError> {
        let n = self.int()?;
        if n < 0 || n as usize > self.data.len() {
            return Err(bad("bad integer"));
        }
        Ok(n as usize)
    }
    fn size(&mut self) -> Result<usize, LuaError> {
        let n = if self.size_t == 4 {
            self.int()? as u32 as u64
        } else {
            let b = self.block(8)?;
            u64::from_le_bytes(b.try_into().unwrap())
        };
        usize::try_from(n).ok().filter(|&n| n <= self.data.len()).ok_or_else(|| bad("bad size"))
    }
    fn number(&mut self) -> Result<f64, LuaError> {
        let b = self.block(8)?;
        Ok(f64::from_le_bytes(b.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<Option<LuaStr>, LuaError> {
        let n = self.size()?;
        if n == 0 {
            return Ok(None);
        }
        let b = self.block(n)?;
        Ok(Some(LuaStr::new(&b[..n - 1])))
    }

    fn function(&mut self, parent_source: &LuaStr, depth: usize) -> Result<Proto, LuaError> {
        if depth > 200 {
            return Err(bad("code too deep"));
        }
        let source = self.string()?.unwrap_or_else(|| parent_source.clone());
        let line_defined = self.int()?;
        let _last_line = self.int()?;
        let nups = self.byte()?;
        let num_params = self.byte()?;
        let is_vararg = self.byte()?;
        let max_stack = self.byte()?;
        let n = self.count()?;
        let mut code = Vec::with_capacity(n);
        for _ in 0..n {
            code.push(self.int()? as u32);
        }
        let n = self.count()?;
        let mut k = Vec::with_capacity(n);
        for _ in 0..n {
            k.push(match self.byte()? {
                0 => Value::Nil,
                1 => Value::Bool(self.byte()? != 0),
                3 => Value::Number(self.number()?),
                4 => Value::Str(self.string()?.unwrap_or_else(|| LuaStr::new(b""))),
                _ => return Err(bad("bad constant")),
            });
        }
        let n = self.count()?;
        let mut p = Vec::with_capacity(n);
        for _ in 0..n {
            p.push(Rc::new(self.function(&source, depth + 1)?));
        }
        let n = self.count()?;
        let mut line_info = Vec::with_capacity(n);
        for _ in 0..n {
            line_info.push(self.int()?);
        }
        let n = self.count()?;
        let mut local_names = Vec::with_capacity(n);
        for _ in 0..n {
            let name = self.string()?.unwrap_or_else(|| LuaStr::new(b""));
            local_names.push((name, self.int()?, self.int()?));
        }
        let n = self.count()?;
        let mut upvalue_names = Vec::with_capacity(n);
        for _ in 0..n {
            upvalue_names.push(self.string()?.unwrap_or_else(|| LuaStr::new(b"")));
        }
        let proto = Proto {
            source,
            line_defined,
            nups,
            num_params,
            is_vararg,
            max_stack,
            code,
            k,
            p,
            line_info,
            upvalue_names,
            local_names,
        };
        check(&proto)?;
        Ok(proto)
    }
}

/// The bounds `luaG_checkcode` guarantees and the interpreter relies on: register, constant, upvalue and
/// prototype operands in range, jumps inside the code, the code ending in RETURN. Not the full verifier.
fn check(p: &Proto) -> Result<(), LuaError> {
    use crate::opcodes::*;
    let ms = p.max_stack as usize;
    let n = p.code.len();
    if n == 0 || op(p.code[n - 1]) != OP_RETURN {
        return Err(bad("bad code"));
    }
    let reg = |r: usize| if r < ms { Ok(()) } else { Err(bad("bad code")) };
    let rk =
        |x: usize| if is_k(x) { if x - BITRK < p.k.len() { Ok(()) } else { Err(bad("bad code")) } } else { reg(x) };
    let mut pc = 0;
    while pc < n {
        let i = p.code[pc];
        let o = op(i);
        if o > OP_VARARG {
            return Err(bad("bad opcode"));
        }
        let (a, b, c, bx) = (arg_a(i), arg_b(i), arg_c(i), arg_bx(i));
        let jump = |off: i64| {
            let d = pc as i64 + 1 + off;
            if d >= 0 && (d as usize) < n { Ok(()) } else { Err(bad("bad jump")) }
        };
        match o {
            OP_MOVE | OP_UNM | OP_NOT | OP_LEN => {
                reg(a)?;
                reg(b)?
            }
            OP_LOADK | OP_GETGLOBAL | OP_SETGLOBAL => {
                reg(a)?;
                if bx >= p.k.len() || (o != OP_LOADK && !matches!(p.k[bx], Value::Str(_))) {
                    return Err(bad("bad constant index"));
                }
            }
            OP_LOADBOOL => {
                reg(a)?;
                if c != 0 {
                    jump(1)?
                }
            }
            OP_LOADNIL => {
                reg(a)?;
                reg(b)?
            }
            OP_GETUPVAL | OP_SETUPVAL => {
                reg(a)?;
                if b >= p.nups as usize {
                    return Err(bad("bad upvalue"));
                }
            }
            OP_GETTABLE | OP_SELF => {
                reg(a + if o == OP_SELF { 1 } else { 0 })?;
                reg(b)?;
                rk(c)?
            }
            OP_SETTABLE | OP_ADD | OP_SUB | OP_MUL | OP_DIV | OP_MOD | OP_POW | OP_EQ | OP_LT | OP_LE => {
                if o == OP_SETTABLE || o <= OP_POW {
                    reg(a)?;
                }
                rk(b)?;
                rk(c)?;
                if matches!(o, OP_EQ | OP_LT | OP_LE) {
                    jump(1)?
                }
            }
            OP_NEWTABLE => reg(a)?,
            OP_CONCAT => {
                reg(a)?;
                reg(c)?;
                if b >= c {
                    return Err(bad("bad code"));
                }
            }
            OP_JMP | OP_FORLOOP | OP_FORPREP => {
                if o != OP_JMP {
                    reg(a + 3)?
                }
                jump(arg_sbx(i))?
            }
            OP_TEST | OP_TESTSET | OP_TFORLOOP => {
                reg(a)?;
                if o == OP_TESTSET {
                    reg(b)?
                }
                if o == OP_TFORLOOP {
                    reg(a + 2 + c.max(1))?;
                }
                if pc + 1 >= n || op(p.code[pc + 1]) != OP_JMP {
                    return Err(bad("bad code"));
                }
            }
            OP_CALL | OP_TAILCALL => {
                reg(a)?;
                if b > 0 {
                    reg(a + b - 1)?
                }
            }
            OP_RETURN => {
                if b > 1 {
                    reg(a + b - 2)?
                }
            }
            OP_SETLIST => {
                if b > 0 {
                    reg(a + b)?
                }
                if c == 0 {
                    pc += 1;
                }
            }
            OP_CLOSE => {}
            OP_CLOSURE => {
                reg(a)?;
                let np = p.p.get(bx).ok_or_else(|| bad("bad function index"))?;
                for j in 0..np.nups as usize {
                    let u = *p.code.get(pc + 1 + j).ok_or_else(|| bad("bad code"))?;
                    match op(u) {
                        OP_MOVE => reg(arg_b(u))?,
                        OP_GETUPVAL => {
                            if arg_b(u) >= p.nups as usize {
                                return Err(bad("bad upvalue"));
                            }
                        }
                        _ => return Err(bad("bad closure")),
                    }
                }
                pc += np.nups as usize;
            }
            OP_VARARG => {
                reg(a)?;
                if b > 1 {
                    reg(a + b - 2)?
                }
            }
            _ => {}
        }
        pc += 1;
    }
    Ok(())
}

/// Load a precompiled chunk; `name` is only used in load error messages (the original's `S.name`).
pub fn undump(data: &[u8]) -> Result<Proto, LuaError> {
    if data.len() < 12 || &data[..4] != b"\x1bLua" {
        return Err(bad("bad header"));
    }
    let h = &data[4..12];
    if h[0] != 0x51 {
        return Err(bad("version mismatch"));
    }
    if h[1] != 0 || h[2] != 1 || h[3] != 4 || !(h[4] == 4 || h[4] == 8) || h[5] != 4 || h[6] != 8 || h[7] != 0 {
        return Err(bad("bad header"));
    }
    let mut l = Loader { data, pos: 12, size_t: h[4] as usize };
    l.function(&LuaStr::new(b"=?"), 0)
}
