//! Hand-assembled chunks (no host Lua needed): malformed code is a load or run error, never a panic or a
//! huge allocation, and tail calls run in constant depth.

use mw_lua::opcodes::*;
use mw_lua::{Lua, Value};

fn abc(o: u32, a: u32, b: u32, c: u32) -> u32 {
    o | (a << 6) | (c << 14) | (b << 23)
}
fn abx(o: u32, a: u32, bx: u32) -> u32 {
    o | (a << 6) | (bx << 14)
}
fn asbx(o: u32, a: u32, sbx: i32) -> u32 {
    abx(o, a, (sbx + 131071) as u32)
}
const K: u32 = BITRK as u32;

enum Const {
    Nil,
    Num(f64),
    Str(&'static str),
}

struct Func {
    params: u8,
    vararg: u8,
    max_stack: u8,
    code: Vec<u32>,
    k: Vec<Const>,
    protos: Vec<Func>,
}

fn int(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn func(out: &mut Vec<u8>, f: &Func) {
    int(out, 0); // no source
    int(out, 0);
    int(out, 0);
    out.extend_from_slice(&[0, f.params, f.vararg, f.max_stack]);
    int(out, f.code.len() as u32);
    for &i in &f.code {
        int(out, i);
    }
    int(out, f.k.len() as u32);
    for k in &f.k {
        match k {
            Const::Nil => out.push(0),
            Const::Num(n) => {
                out.push(3);
                out.extend_from_slice(&n.to_le_bytes());
            }
            Const::Str(s) => {
                out.push(4);
                int(out, s.len() as u32 + 1);
                out.extend_from_slice(s.as_bytes());
                out.push(0);
            }
        }
    }
    int(out, f.protos.len() as u32);
    for p in &f.protos {
        func(out, p);
    }
    int(out, 0);
    int(out, 0);
    int(out, 0);
}

fn chunk(f: &Func) -> Vec<u8> {
    let mut out = b"\x1bLua\x51\x00\x01\x04\x04\x04\x08\x00".to_vec();
    func(&mut out, f);
    out
}

fn main_fn(code: Vec<u32>, k: Vec<Const>) -> Func {
    Func { params: 0, vararg: 2, max_stack: 4, code, k, protos: vec![] }
}

/// Load and run; the error message of whichever step failed.
fn run(f: &Func) -> Result<Vec<Value>, String> {
    let mut lua = Lua::new();
    let main = lua.load(&chunk(f)).map_err(|e| format!("load: {}", e.message()))?;
    lua.call(&Value::Function(main), vec![]).map_err(|e| e.message())
}

#[test]
fn verifier_rejects_unchecked_code() {
    // VARARG in a function that is not vararg (luaG_checkcode).
    let mut f = main_fn(vec![abc(OP_VARARG, 0, 2, 0), abc(OP_RETURN, 0, 1, 0)], vec![]);
    f.vararg = 0;
    assert!(run(&f).unwrap_err().starts_with("load:"));
    // A comparison whose next instruction is not the JMP it takes its offset from.
    let f = main_fn(vec![abc(OP_EQ, 1, K, K), abc(OP_LOADNIL, 0, 0, 0), abc(OP_RETURN, 0, 1, 0)], vec![Const::Nil]);
    assert!(run(&f).unwrap_err().starts_with("load:"));
    // A jump to a SETLIST block number (here a word that would jump far before the code).
    let f = main_fn(
        vec![
            asbx(OP_JMP, 0, 2),
            abc(OP_NEWTABLE, 0, 0, 0),
            abc(OP_SETLIST, 0, 1, 0),
            asbx(OP_JMP, 0, -1000),
            abc(OP_RETURN, 0, 1, 0),
        ],
        vec![],
    );
    assert!(run(&f).unwrap_err().starts_with("load:"));
    // An open call whose values nothing takes.
    let f = main_fn(vec![abc(OP_CALL, 0, 1, 0), abc(OP_LOADNIL, 0, 0, 0), abc(OP_RETURN, 0, 1, 0)], vec![]);
    assert!(run(&f).unwrap_err().starts_with("load:"));
}

#[test]
fn malformed_operands_are_errors() {
    // B = 0 (values up to top) with no open call before it.
    let f = main_fn(vec![abc(OP_LOADNIL, 0, 1, 0), abc(OP_CALL, 0, 0, 1), abc(OP_RETURN, 0, 1, 0)], vec![]);
    assert!(run(&f).unwrap_err().ends_with(": bad code"));
    // Table sizes past MAXASIZE: an error, not a multi-GB allocation.
    let f = main_fn(vec![abc(OP_NEWTABLE, 0, 28 << 3, 0), abc(OP_RETURN, 0, 1, 0)], vec![]);
    assert!(run(&f).unwrap_err().ends_with(": not enough memory"));
    let f = main_fn(vec![abc(OP_NEWTABLE, 0, 0, 28 << 3), abc(OP_RETURN, 0, 1, 0)], vec![]);
    assert!(run(&f).unwrap_err().ends_with(": table overflow"));
    // SETLIST with a block number word of 0.
    let f = main_fn(
        vec![abc(OP_NEWTABLE, 0, 0, 0), abx(OP_LOADK, 1, 0), abc(OP_SETLIST, 0, 1, 0), 0, abc(OP_RETURN, 0, 1, 0)],
        vec![Const::Num(1.0)],
    );
    assert!(run(&f).unwrap_err().ends_with(": not enough memory"));
}

#[test]
fn string_rep_too_big_is_an_error() {
    let mut lua = Lua::new();
    let r = lua.call_field("string", "rep", vec![Value::from("abc"), Value::Number(1e9)]);
    assert_eq!(r.err().map(|e| e.message()).as_deref(), Some("not enough memory"));
}

#[test]
fn tail_calls_do_not_nest() {
    // function f(n) if n == 0 then return n end return f(n - 1) end; return f(10000)
    let f = Func {
        params: 1,
        vararg: 0,
        max_stack: 3,
        code: vec![
            abc(OP_EQ, 1, 0, K),
            asbx(OP_JMP, 0, 4),
            abx(OP_GETGLOBAL, 1, 1),
            abc(OP_SUB, 2, 0, K + 2),
            abc(OP_TAILCALL, 1, 2, 0),
            abc(OP_RETURN, 1, 0, 0),
            abc(OP_RETURN, 0, 2, 0),
            abc(OP_RETURN, 0, 1, 0),
        ],
        k: vec![Const::Num(0.0), Const::Str("f"), Const::Num(1.0)],
        protos: vec![],
    };
    let mut main = main_fn(
        vec![
            abx(OP_CLOSURE, 0, 0),
            abx(OP_SETGLOBAL, 0, 0),
            abx(OP_GETGLOBAL, 0, 0),
            abx(OP_LOADK, 1, 1),
            abc(OP_CALL, 0, 2, 2),
            abc(OP_RETURN, 0, 2, 0),
            abc(OP_RETURN, 0, 1, 0),
        ],
        vec![Const::Str("f"), Const::Num(10000.0)],
    );
    main.protos.push(f);
    let r = run(&main).unwrap();
    assert!(matches!(r.as_slice(), [Value::Number(n)] if *n == 0.0), "{:?}", r.len());
}
