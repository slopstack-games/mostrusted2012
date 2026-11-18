//! Differential tests against the reference implementation: each script is compiled with the host's
//! `luac5.1`, run by `mw_lua`, and its printed output compared with `lua5.1` running the source. Skipped when
//! the host has no Lua 5.1. Cases avoid what legitimately differs from the host build: exponent formatting
//! (the game's MSVC CRT prints `e+020`, glibc `e+20`; covered by unit tests), address-keyed tables, and
//! non-integral integer arguments (MSVC x86 rounds, x86-64 builds truncate).

use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

use mw_lua::{Lua, Value};

fn have_host_lua() -> bool {
    // In the temp dir: `luac -v` may write an empty `luac.out`.
    let tmp = env!("CARGO_TARGET_TMPDIR");
    Command::new("luac5.1").arg("-v").current_dir(tmp).output().is_ok()
        && Command::new("lua5.1").arg("-v").output().is_ok()
}

fn run(name: &str, src: &str) -> Option<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("mw-lua-diff").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t.lua"), src).unwrap();
    let c = Command::new("luac5.1").args(["-o", "t.luac", "t.lua"]).current_dir(&dir).output().unwrap();
    assert!(c.status.success(), "{name}: luac failed: {}", String::from_utf8_lossy(&c.stderr));
    let r = Command::new("lua5.1").arg("t.lua").current_dir(&dir).output().unwrap();
    let mut expected = String::from_utf8_lossy(&r.stdout).into_owned();
    if !r.status.success() {
        let err = String::from_utf8_lossy(&r.stderr);
        let first = err.lines().next().unwrap_or("").trim_start_matches("lua5.1: ").to_string();
        expected.push_str(&format!("ERROR: {first}\n"));
    }

    let chunk = std::fs::read(dir.join("t.luac")).unwrap();
    let out = Rc::new(RefCell::new(String::new()));
    let mut lua = Lua::new();
    let o = out.clone();
    lua.set_print(move |s| {
        o.borrow_mut().push_str(s);
        o.borrow_mut().push('\n');
    });
    let main = lua.load(&chunk).expect("load");
    if let Err(e) = lua.call(&Value::Function(main), vec![]) {
        out.borrow_mut().push_str(&format!("ERROR: {}\n", e.message()));
    }
    let ours = out.borrow().clone();
    Some((expected, ours))
}

fn check(name: &str, src: &str) {
    let Some((expected, ours)) = run(name, src) else { return };
    if expected != ours {
        let mut msg = format!("{name}: output differs\n");
        for (i, (e, o)) in expected.lines().zip(ours.lines()).enumerate() {
            if e != o {
                msg.push_str(&format!("line {}:\n  lua5.1: {e}\n  mw-lua: {o}\n", i + 1));
            }
        }
        msg.push_str(&format!("({} vs {} lines)", expected.lines().count(), ours.lines().count()));
        panic!("{msg}");
    }
}

macro_rules! case {
    ($name:ident, $src:expr) => {
        #[test]
        fn $name() {
            if !have_host_lua() {
                eprintln!("skipped: no host lua5.1 / luac5.1");
                return;
            }
            check(stringify!($name), $src);
        }
    };
}

case!(
    arithmetic_and_numbers,
    r#"
print(1, 1.5, -0.25, 1/3, 100, 123456789012, 2^31, -2^31, 0.1 + 0.2, 100 / 3)
print(10 % 3, -10 % 3, 10 % -3, 5.5 % 2, 2^0.5, -2^2)
"#
);

case!(
    coercion,
    r#"
print("10" + 5, "0x10" + 0, "1e2" * 2, 10 .. 20, 1.5 .. "", -0.5 .. "|")
print(tostring(nil), tostring(true), type(print), type({}), type("x"), type(2))
print(tonumber("  12  "), tonumber("12a"), tonumber("ff", 16), tonumber("777", 8), tonumber("z", 36), tonumber(""), tonumber("0x"), tonumber(" 0x1F "))
print(math.floor(3.7), math.ceil(3.2), math.max(1, 5, 3), math.min(4, 2, 8), math.abs(-3), math.huge > 1e308, math.pi, math.sqrt(16), math.fmod(7, 3), math.modf(3.7))
print(1e100 == 1e100, 3 == 3.0, "3" == 3, 0 == -0)
"#
);

case!(
    strings,
    r#"
local s = "Hello World from Lua"
print(string.len(s), #s, s:upper(), s:lower(), s:rep(2), ("ab"):rep(0))
print(string.sub(s, 1, 5), string.sub(s, -3), string.sub(s, 7), string.sub(s, 0), string.sub(s, 5, 2), string.sub(s, -100, 3))
print(string.find(s, "World"), string.find(s, "o", 6), string.find(s, "%s(%w+)"), string.find(s, "xyz"), string.find(s, ".", 1, true))
print(string.find(s, ""), string.find(s, "", 100), string.find(s, "Lua$"), string.find(s, "^Hello"), string.find(s, "^World"))
print(string.match(s, "(%a+) (%a+)"), string.match("key=val", "(%w+)=(%w+)"), string.match("  trim  ", "^%s*(.-)%s*$"), string.match("abc", "()b()"))
print(string.gsub(s, "o", "0"))
print(string.gsub(s, "(%w+)", "<%1>"))
print(string.gsub("abc", "", "-"))
print(string.gsub("hello world", "%w+", "%0 %0", 1))
print(string.gsub("$name is $age", "%$(%w+)", {name="Bob", age=42}))
print(string.gsub("abc", "%w", function(c) return c:byte() end))
print(string.gsub("abc", "%w", function(c) if c == "b" then return false end return "x" end))
print(string.gsub("a,b,,c", ",", ";"), string.gsub("path/to/file", "/", "\\"))
print(string.byte("ABC"), string.byte("ABC", 2), string.byte("ABC", 1, -1), string.char(72, 105), string.reverse("abc"))
for w in string.gmatch("one two  three", "%a+") do print(w) end
for k, v in string.gmatch("a=1, b=2", "(%w+)=(%w+)") do print(k, v) end
for p in string.gmatch("abc", "") do print(p) end
print(string.find("a.b", ".", 1, true), string.find("[x]", "[%[]"), string.match("2024-01-05", "(%d+)-(%d+)-(%d+)"))
print(string.find("THE (quick) fox", "%((%a+)%)"), string.match("f(a(b)c)d", "%b()"), string.find("THE fox", "%f[%a]%a+", 2))
print(string.match("hello", "(h)(e)(l)(l)(o)"), string.match("aaa", "a-"), string.match("aaa", "a-$"), string.match("x = 10", "(%w+)%s*=%s*(%d+)"))
print(string.find("abc", "[a-c]+"), string.find("abc", "[^a]+"), string.match("a1b2c3", "%d"), string.match("  x", "%S"), string.match("abab", "(ab)%1"))
print(string.gsub("hello", "l+", "%%"), string.match("A-Z", "[A%-Z]+"), string.match("x]y", "[]x]+"))
print(("x"):rep(3), ("%d items"):format(5), ("CamelCase"):gsub("%u", function(c) return "_" .. c:lower() end))
"#
);

case!(
    format,
    r#"
print(string.format("%d %5d %-5d| %05d %x %X %o %c %%", 42, 42, 42, -42, 255, 255, 8, 65))
print(string.format("%5.2f %.3f %10.4f %-8.1f|", 3.14159, 2/3, -1.5, 9.99))
print(string.format("%g %g %g %g %.3g %G", 0.0001, 100000, 0.5, 123456, 3.14159, 1.5))
print(string.format("%s|%10s|%-10s|%.2s|%s|%s", "abc", "abc", "abc", "abc", 12, 1.5))
print(string.format("%q", 'he said "hi"\n\\ \r end'))
print(string.format("%5.1f|%-6d|%+d|% d|%#x|%#o|%.3d|%3c|", 2.25, 7, 5, 5, 255, 8, 7, 66))
print(string.format("[%10.3s]", "abcdef"), string.format("%-+6d|", 3), string.format("%.0f %.0f %.0f", 0.5, 1.5, 2.5))
"#
);

case!(
    table_order,
    r#"
local t = {}
for i = 1, 10 do t[i] = i * i end
t.x = 1; t.y = 2; t.name = "n"; t[100] = "h"; t[1.5] = "f"; t[true] = "b"; t[-1] = "neg"; t[false] = "nb"
local keys = {}
for k, v in pairs(t) do keys[#keys+1] = tostring(k) .. "=" .. tostring(v) end
print(table.concat(keys, ","))
local u = {Construct=1, SetConfigurationItem=2, OnActivated=3, OnDeactivating=4, PreInputUpdate=5, OnAction=6, OnBack=7, OnUp=8, OnDown=9, Destruct=10}
local ks = {} for k in pairs(u) do ks[#ks+1] = k end print(table.concat(ks, " "))
u.OnUp = nil; u.OnLeft = 11; u.OnUp = 12; u.Extra1 = 1; u.Extra2 = 2; u.Extra3 = 3; u.Extra4 = 4; u.Extra5 = 5; u.Extra6 = 6; u.Extra7 = 7
ks = {} for k in pairs(u) do ks[#ks+1] = k end print(table.concat(ks, " "))
local v = {}
for i = 1, 40 do v["k" .. i] = i end
for i = 1, 40, 3 do v["k" .. i] = nil end
for i = 20, 1, -1 do v[i] = i end
ks = {} for k in pairs(v) do ks[#ks+1] = tostring(k) end print(table.concat(ks, " "))
local w = {10, 20, 30, nil, 50, x = 1, [7] = 70}
ks = {} for k, val in pairs(w) do ks[#ks+1] = tostring(k) .. ":" .. tostring(val) end print(table.concat(ks, " "), #w)
local h = {1, 2, nil, 4} print(#h)
local g = {} g[1] = 1 g[2] = 2 g[4] = 4 print(#g)
for i, val in ipairs({1, 2, nil, 4}) do print(i, val) end
print(next({}), next({5}), type(next))
local m = {} for i = 1, 5 do m[i * 2] = i end
ks = {} for k, val in pairs(m) do ks[#ks+1] = k .. "=" .. val end print(table.concat(ks, " "))
"#
);

case!(
    table_library,
    r#"
local t = {} for i = 1, 10 do t[i] = i * i end
print(#t, select('#', 1, 2, 3), select(2, "a", "b", "c"), select(-1, "a", "b"))
table.insert(t, 1, 0); table.insert(t, "end"); print(table.concat(t, ",", 1, 5), #t, table.remove(t), table.remove(t, 1), #t)
local arr = {5, 2, 8, 1, 9, 3, 7, 4, 6, 10, 15, 12, 11, 14, 13}
table.sort(arr) print(table.concat(arr, " "))
table.sort(arr, function(a, b) return a > b end) print(table.concat(arr, " "))
local words = {"banana", "apple", "Cherry", "date", "apple", "Banana", "cherry"}
table.sort(words) print(table.concat(words, " "))
local recs = {} for i = 1, 30 do recs[i] = {k = (i * 7) % 5, id = i} end
table.sort(recs, function(a, b) return a.k < b.k end)
local ids = {} for i, r in ipairs(recs) do ids[i] = r.id end print(table.concat(ids, " "))
print(table.maxn({1, 2, [10] = 3}), table.getn({1, 2, 3}), unpack({1, 2, 3}))
print(unpack({1, 2, 3}, 2), unpack({1, 2, 3}, 2, 5))
print(table.concat({}), table.concat({1, 2.5, "x"}, "-"), table.remove({}), #{n = 1})
local q = {"a", "b", "c"} table.insert(q, 2, "X") print(table.concat(q, ""))
table.foreachi({"p", "q"}, print)
"#
);

case!(
    closures_and_varargs,
    r#"
local function counter() local c = 0 return function() c = c + 1 return c end end
local c1, c2 = counter(), counter()
print(c1(), c1(), c2(), c1())
local function va(...) local a, b = ... return select('#', ...), a, b, ... end
print(va(1, nil, 3))
print(va())
local fns = {} for i = 1, 3 do fns[i] = function() return i end end
print(fns[1](), fns[2](), fns[3]())
local function shared() local x = 0 local function inc() x = x + 1 end local function get() return x end return inc, get end
local inc, get = shared() inc() inc() print(get())
local function rec(n) if n == 0 then return 0 end return n + rec(n - 1) end print(rec(50))
local function tail(n, acc) if n == 0 then return acc end return tail(n - 1, acc + n) end print(tail(100, 0))
local function multi() return 1, 2, 3 end
print(multi(), (multi()), #{multi()}, multi())
local t = {multi(), multi()} print(#t)
local function old(...) return arg and arg.n end print(old(1, 2))
local sum = 0 for i = 10, 1, -2 do sum = sum + i end print(sum)
for i = 1, 3 do for j = 1, 2 do if j == 2 then break end print(i, j) end end
local n = 0 while n < 5 do n = n + 1 end repeat n = n - 2 until n < 0 print(n)
for i = 1, 0 do print("never") end for i = 1.5, 3 do print(i) end
local a, b, c = (function() return 1 end)() print(a, b, c)
local x, y = 1 print(x, y)
print(not nil, not 0, nil and 1, false or "d", 1 and 2, nil or false)
"#
);

case!(
    errors,
    r#"
print(pcall(error, "msg"))
print(select('#', pcall(error, {})), pcall(function() error("lvl", 0) end))
print(pcall(function() error("with pos") end))
print(pcall(function() error("lvl2", 2) end))
print(pcall(function() local x = nil; return x.y end))
print(pcall(function() return undefinedglobal() end))
print(pcall(function() local t = {} return t.a.b end))
print(pcall(function() local t = {} t.f() end))
print(pcall(function() return 1 + {} end))
print(pcall(function() return "a" < 1 end))
print(pcall(function() return {} < {} end))
print(pcall(function() return #nil end))
print(pcall(function() return string.sub() end))
print(pcall(function() return string.rep("x", "y") end))
print(pcall(function() return ("x"):bad() end))
print(pcall(function() local t = setmetatable({}, {__index = function(t, k) error("idx " .. k) end}) return t.zz end))
print(pcall(function() return nil .. "x" end))
print(pcall((function() local up = nil return function() return up.x end end)()))
print(pcall(function() for i = "a", 2 do end end))
print(pcall(function() t = nil; t[nil] = 1 end))
print(pcall(function() local t = {} t[nil] = 1 end))
print(xpcall(function() error("E") end, function(m) return "handled: " .. m end))
print(pcall(assert, false), pcall(assert, nil, "custom"), pcall(assert, 1, 2))
error("top level")
"#
);

case!(
    metatables,
    r#"
local mt = {__index = function(t, k) return k .. "!" end, __add = function(a, b) return 42 end, __tostring = function() return "OBJ" end,
  __call = function(self, x) return x * 2 end, __eq = function() return true end, __lt = function() return true end,
  __le = function() return false end, __concat = function(a, b) return "cat" end, __unm = function() return "neg" end}
local o = setmetatable({}, mt)
print(o.foo, o + 1, 1 + o, tostring(o), o(21), o .. "x", "x" .. o, -o, o < o, o <= o)
local o2 = setmetatable({}, mt)
print(o == o2, rawequal(o, o2), o ~= o2)
local proxy = setmetatable({}, {__newindex = function(t, k, v) rawset(t, k, v * 10) end})
proxy.a = 5 print(proxy.a)
local base = {greet = function() return "hi" end}
local derived = setmetatable({}, {__index = base})
print(derived.greet(), rawget(derived, "greet"), getmetatable(derived).__index == base)
local prot = setmetatable({}, {__metatable = "locked"}) print(getmetatable(prot), pcall(setmetatable, prot, {}))
print(getmetatable("abc").__index == string, ("abc"):len())
local chain = setmetatable({}, {__index = setmetatable({mid = 1}, {__index = {deep = 2}})}) print(chain.mid, chain.deep, chain.none)
"#
);

case!(
    regressions,
    r#"
local function tc(n) if n > 0 then return tc(n - 1) end return "done", n end
print(tc(10000))
local function even(n) if n == 0 then return true end return odd(n - 1) end
function odd(n) if n == 0 then return false end return even(n - 1) end
print(even(5001))
print(pcall(error, 42), pcall(error, 42, 2), pcall(function() error(42) end), pcall(function() error(42, 0) end))
print(type(select(2, pcall(function() error(42) end))), type(select(2, pcall(error, 42))))
print(xpcall(function() error("x") end, function(m) error("y") end))
print(xpcall(function() error("x") end, 5))
print(xpcall(function() error("x") end, function(m) return "h:" .. m end))
print(pcall(function() return ("%d"):format("x") end))
print(pcall(function() return ("x"):rep() end))
print(pcall(function() return string.rep() end))
print(pcall(function() local f = string.rep return f("x") end))
print(pcall(string.rep), pcall(pcall), pcall(setmetatable, 1))
print(pcall(function() return string.rep(nil, 1) end))
local t = {f = string.sub}
print(pcall(function() return t.f() end))
"#
);

/// Number formatting of the game's MSVC runtime (not comparable with a glibc host).
#[test]
fn msvc_number_formatting() {
    use mw_lua::number::*;
    assert_eq!(fmt_number(2f64.powi(53)), "9.007199254741e+015");
    assert_eq!(fmt_number(1e20), "1e+020");
    assert_eq!(fmt_number(-1e-10), "-1e-010");
    assert_eq!(fmt_e(12345.678, 6, false, false), "1.234568e+004");
    assert_eq!(fmt_g(1e20, 6, true, false), "1E+020");
    assert_eq!(fmt_number(1.0 / 0.0), "1.#INF");
    assert_eq!(fmt_number(-1.0 / 0.0), "-1.#INF");
}
