//! Writing and deleting members of a script's view-model JSON by path (`Crumbs[1].Description`), as the
//! original's JSON wrapper does: `FUN_009f8a00` (set), `FUN_009f8800` (delete), both through the parent walk
//! `FUN_009f8150`. Used by the `Set*Variable` / `DeleteVariable` natives on the script object's view model
//! (+0xb4).
//!
//! Rules (C, from those functions):
//! - a path containing any of `(){}` is refused (set); it is cut to 127 bytes;
//! - the path splits at its last `.`: the part before is walked from the root, creating objects on the way
//!   (set) or not (delete); the leaf must not contain `(){}.`;
//! - a segment `name[n]` indexes the array member `name`, **1-based** (`strtol`, base 10, of what is between the
//!   brackets, at most 15 characters; n ≤ 0 or no `]` is invalid); a missing array is created only for n = 1, an
//!   existing one grows only by one (n = length + 1); array elements on the way must be objects;
//! - members are found by the GeneSys field hash of their name (`FUN_009f1650`), first match; a set replaces the
//!   member's value in place or appends the member.
//!
//! Model decisions: an invalid index segment in the middle of a path fails the whole set (the original skips the
//! segment and stays at the current object); walking into a member that is not an object fails (the original
//! would treat the value as an object).

use mw_formats::genesys::field_hash;
use mw_formats::json::Json;

const MAX_PATH: usize = 127;

/// A path segment: `name` or `name[n]` (n 1-based).
enum Seg<'a> {
    Name(&'a str),
    Index(&'a str, usize),
}

/// `FUN_009f14f0`: the 1-based index of `name[n]`; `Err` = invalid.
fn seg(token: &str) -> Result<Seg<'_>, ()> {
    let Some(open) = token.find('[') else { return Ok(Seg::Name(token)) };
    let rest = &token[open + 1..];
    let close = rest.find(']').ok_or(())?;
    let digits: String = rest[..close].chars().take(15).collect();
    let n = strtol10(&digits);
    if n <= 0 {
        return Err(());
    }
    Ok(Seg::Index(&token[..open], n as usize))
}

/// C `strtol(s, NULL, 10)` (leading spaces, sign, digits; saturating).
fn strtol10(s: &str) -> i64 {
    let t = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let mut v: i64 = 0;
    for c in digits.bytes().take_while(u8::is_ascii_digit) {
        v = (v * 10 + (c - b'0') as i64).min(i32::MAX as i64 + 1);
    }
    let v = if neg { -v } else { v };
    v.clamp(i32::MIN as i64, i32::MAX as i64)
}

fn members(obj: &mut Json) -> Option<&mut Vec<(String, Json)>> {
    match obj {
        Json::Object(e) => Some(e),
        _ => None,
    }
}

fn find(entries: &[(String, Json)], name: &str) -> Option<usize> {
    let h = field_hash(name);
    entries.iter().position(|(k, _)| field_hash(k) == h)
}

/// `FUN_009f8150`: walk `prefix` (dot-separated) from `root`; `create` adds the missing objects / arrays.
fn walk<'a>(root: &'a mut Json, prefix: &str, create: bool) -> Option<&'a mut Json> {
    let mut cur = root;
    for token in prefix.split('.') {
        if token.is_empty() {
            return None;
        }
        let entries = members(cur)?;
        cur = match seg(token).ok()? {
            Seg::Name(name) => match find(entries, name) {
                Some(i) => &mut entries[i].1,
                None if create => {
                    entries.push((name.to_string(), Json::Object(Vec::new())));
                    &mut entries.last_mut().unwrap().1
                }
                None => return None,
            },
            Seg::Index(name, n) => match find(entries, name) {
                Some(i) => {
                    let Json::Array(items) = &mut entries[i].1 else { return None };
                    if n - 1 < items.len() {
                        let item = &mut items[n - 1];
                        if !matches!(item, Json::Object(_)) {
                            return None;
                        }
                        item
                    } else if n - 1 == items.len() && create {
                        items.push(Json::Object(Vec::new()));
                        items.last_mut().unwrap()
                    } else {
                        return None;
                    }
                }
                None if create && n == 1 => {
                    entries.push((name.to_string(), Json::Array(vec![Json::Object(Vec::new())])));
                    let Json::Array(items) = &mut entries.last_mut().unwrap().1 else { unreachable!() };
                    &mut items[0]
                }
                None => return None,
            },
        };
    }
    Some(cur)
}

fn cut(path: &str) -> &str {
    super::lua_args::fit(path, MAX_PATH + 1)
}

/// Split at the last `.` into (parent object, leaf); the parent is walked with `create`.
fn parent<'a, 'p>(root: &'a mut Json, path: &'p str, create: bool) -> Option<(&'a mut Json, &'p str)> {
    match path.rfind('.') {
        Some(dot) => Some((walk(root, &path[..dot], create)?, &path[dot + 1..])),
        None => Some((root, path)),
    }
}

/// `FUN_009f8a00`: set `path` to `value`; false when the path is refused or cannot be reached.
pub fn set(root: &mut Json, path: &str, value: Json) -> bool {
    let path = cut(path);
    if path.contains(['(', ')', '{', '}']) {
        return false;
    }
    let Some((obj, leaf)) = parent(root, path, true) else { return false };
    if leaf.contains(['(', ')', '{', '}', '.']) {
        return false;
    }
    let Some(entries) = members(obj) else { return false };
    match seg(leaf) {
        Ok(Seg::Name(name)) => {
            match find(entries, name) {
                Some(i) => entries[i].1 = value,
                None => entries.push((name.to_string(), value)),
            }
            true
        }
        Ok(Seg::Index(name, n)) => {
            let i = match find(entries, name) {
                Some(i) => i,
                None if n == 1 => {
                    entries.push((name.to_string(), Json::Array(Vec::new())));
                    entries.len() - 1
                }
                None => return false,
            };
            let Json::Array(items) = &mut entries[i].1 else { return false };
            if n - 1 < items.len() {
                items[n - 1] = value;
                true
            } else if n <= items.len() + 1 {
                items.push(value);
                true
            } else {
                false
            }
        }
        Err(()) => false,
    }
}

/// `FUN_009f8800`: delete the member (or the 1-based array element) at `path`.
pub fn delete(root: &mut Json, path: &str) -> bool {
    let path = cut(path);
    let Some((obj, leaf)) = parent(root, path, false) else { return false };
    if leaf.contains(['(', ')', '{', '}', '.']) {
        return false;
    }
    let Some(entries) = members(obj) else { return false };
    match seg(leaf) {
        Ok(Seg::Name(name)) => {
            if let Some(i) = find(entries, name) {
                entries.remove(i);
            }
            true
        }
        Ok(Seg::Index(name, n)) => match find(entries, name).map(|i| &mut entries[i].1) {
            Some(Json::Array(items)) if n - 1 < items.len() => {
                items.remove(n - 1);
                true
            }
            _ => false,
        },
        Err(()) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        let mut vm = Json::Object(Vec::new());
        assert!(set(&mut vm, "Playing", Json::Bool(true)));
        assert!(set(&mut vm, "Playing", Json::Bool(false)));
        assert!(set(&mut vm, "Crumbs[1].Description", Json::Str("A".into())));
        assert!(set(&mut vm, "Crumbs[2].Description", Json::Str("B".into())));
        assert!(!set(&mut vm, "Crumbs[4].Description", Json::Str("D".into())));
        assert!(!set(&mut vm, "List[2]", Json::Int(1)));
        assert!(set(&mut vm, "List[1]", Json::Int(1)));
        assert!(set(&mut vm, "List[2]", Json::Int(2)));
        assert!(set(&mut vm, "List[1]", Json::Int(3)));
        assert!(!set(&mut vm, "List[0]", Json::Int(3)));
        assert!(!set(&mut vm, "A{b}", Json::Int(3)));
        assert!(set(&mut vm, "Deep.Er.Value", Json::Double(0.5)));
        assert_eq!(
            vm,
            Json::parse(
                r#"{"Playing": false, "Crumbs": [{"Description": "A"}, {"Description": "B"}], "List": [3, 2],
                    "Deep": {"Er": {"Value": 0.5}}}"#
            )
            .unwrap()
        );
        assert!(delete(&mut vm, "Crumbs[1]"));
        assert!(delete(&mut vm, "Deep.Er"));
        assert!(delete(&mut vm, "Nothing"));
        assert!(!delete(&mut vm, "No.Thing"));
        assert!(!delete(&mut vm, "List[3]"));
        assert_eq!(
            vm,
            Json::parse(r#"{"Playing": false, "Crumbs": [{"Description": "B"}], "List": [3, 2], "Deep": {}}"#)
                .unwrap()
        );
    }
}
