//! The 38 Lua 5.1 opcodes and the instruction layout (`lopcodes.h`): op 6 bits, A 8, C 9, B 9 (Bx = B:C
//! 18 bits, sBx = Bx - 131071). RK operands >= 256 index the constant table.

pub const OP_MOVE: u32 = 0;
pub const OP_LOADK: u32 = 1;
pub const OP_LOADBOOL: u32 = 2;
pub const OP_LOADNIL: u32 = 3;
pub const OP_GETUPVAL: u32 = 4;
pub const OP_GETGLOBAL: u32 = 5;
pub const OP_GETTABLE: u32 = 6;
pub const OP_SETGLOBAL: u32 = 7;
pub const OP_SETUPVAL: u32 = 8;
pub const OP_SETTABLE: u32 = 9;
pub const OP_NEWTABLE: u32 = 10;
pub const OP_SELF: u32 = 11;
pub const OP_ADD: u32 = 12;
pub const OP_SUB: u32 = 13;
pub const OP_MUL: u32 = 14;
pub const OP_DIV: u32 = 15;
pub const OP_MOD: u32 = 16;
pub const OP_POW: u32 = 17;
pub const OP_UNM: u32 = 18;
pub const OP_NOT: u32 = 19;
pub const OP_LEN: u32 = 20;
pub const OP_CONCAT: u32 = 21;
pub const OP_JMP: u32 = 22;
pub const OP_EQ: u32 = 23;
pub const OP_LT: u32 = 24;
pub const OP_LE: u32 = 25;
pub const OP_TEST: u32 = 26;
pub const OP_TESTSET: u32 = 27;
pub const OP_CALL: u32 = 28;
pub const OP_TAILCALL: u32 = 29;
pub const OP_RETURN: u32 = 30;
pub const OP_FORLOOP: u32 = 31;
pub const OP_FORPREP: u32 = 32;
pub const OP_TFORLOOP: u32 = 33;
pub const OP_SETLIST: u32 = 34;
pub const OP_CLOSE: u32 = 35;
pub const OP_CLOSURE: u32 = 36;
pub const OP_VARARG: u32 = 37;

pub const OP_NAMES: [&str; 38] = [
    "MOVE",
    "LOADK",
    "LOADBOOL",
    "LOADNIL",
    "GETUPVAL",
    "GETGLOBAL",
    "GETTABLE",
    "SETGLOBAL",
    "SETUPVAL",
    "SETTABLE",
    "NEWTABLE",
    "SELF",
    "ADD",
    "SUB",
    "MUL",
    "DIV",
    "MOD",
    "POW",
    "UNM",
    "NOT",
    "LEN",
    "CONCAT",
    "JMP",
    "EQ",
    "LT",
    "LE",
    "TEST",
    "TESTSET",
    "CALL",
    "TAILCALL",
    "RETURN",
    "FORLOOP",
    "FORPREP",
    "TFORLOOP",
    "SETLIST",
    "CLOSE",
    "CLOSURE",
    "VARARG",
];

pub const BITRK: usize = 256;
/// `LFIELDS_PER_FLUSH`: SETLIST block size.
pub const LFIELDS_PER_FLUSH: usize = 50;

#[inline]
pub fn op(i: u32) -> u32 {
    i & 0x3f
}
#[inline]
pub fn arg_a(i: u32) -> usize {
    ((i >> 6) & 0xff) as usize
}
#[inline]
pub fn arg_c(i: u32) -> usize {
    ((i >> 14) & 0x1ff) as usize
}
#[inline]
pub fn arg_b(i: u32) -> usize {
    ((i >> 23) & 0x1ff) as usize
}
#[inline]
pub fn arg_bx(i: u32) -> usize {
    (i >> 14) as usize
}
#[inline]
pub fn arg_sbx(i: u32) -> i64 {
    (i >> 14) as i64 - 131071
}
#[inline]
pub fn is_k(x: usize) -> bool {
    x & BITRK != 0
}

/// `testAMode`: the opcode writes register A (used by the error-message symbolic execution).
pub fn sets_a(o: u32) -> bool {
    !matches!(
        o,
        OP_SETGLOBAL
            | OP_SETUPVAL
            | OP_SETTABLE
            | OP_JMP
            | OP_EQ
            | OP_LT
            | OP_LE
            | OP_RETURN
            | OP_TFORLOOP
            | OP_SETLIST
            | OP_CLOSE
    )
}

/// `luaO_fb2int`: the "floating point byte" table sizes of NEWTABLE.
pub fn fb2int(x: usize) -> usize {
    let e = (x >> 3) & 31;
    if e == 0 { x } else { ((x & 7) + 8) << (e - 1) }
}
