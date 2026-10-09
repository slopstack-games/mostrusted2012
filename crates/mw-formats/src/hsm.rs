//! `HSMData` (0x501): the game logic's hierarchical state machines (the boot / front-end / game-mode flow in
//! `GAMELOGIC/GAMEMODES.BNDL`, the cars' active-aero machines). Layout from the data, checked against the
//! code that reads it (docs/formats.md "HSMData"; the behaviour registry and node lookup in docs/decomp.md
//! "Front end"). Offsets are into the resource's chunk 0.
//!
//! | off | type | field |
//! |-----|------|-------|
//! | 0x00 | u32 | version (3) |
//! | 0x04 / 0x08 | ptr, u32 | entry points: 12 B {u32 node id, u32 behaviour code, u32 key} (36 in GAMEMODES) |
//! | 0x0C / 0x10 | ptr, u32 | nodes (32 B each; 2706 in GAMEMODES) |
//! | 0x14 / 0x18 | ptr, u32 | not read by the interpreter |
//! | 0x1C / 0x20 | ptr, u32 | behaviour types: 32 B {u64 key, ptr name, ...} (199) |
//!
//! Node (32 B; found by id): +0 u32 behaviour code (CRC-32 of the behaviour's code name, `~crc32(name)`:
//! `HSB_RUNCHILD` 0x24c422b2, `LOADWORLD` 0xac59d95a, `HSB_TOG_LS` 0xe6d37d97), +4 u32 id, +8 ptr events,
//! +0xC ptr children, +0x10 ptr parameters, +0x14 ptr inline Lua chunk (or 0), +0x18 ptr name, +0x1C u8
//! parameter count, +0x1D u8 kind, +0x1E u8 event count, +0x1F u8 child count.
//! - Event (16 B): u64 exit code, u8 action (1 = start the listed nodes), u8 count, ptr → count × 16 B
//!   {u64 key, u32 node id, ptr name}. A node's `ENTER` event lists the children it starts with.
//! - Child (16 B): u32 child id, ptr transitions, u32 transition count, ptr name. Transition (24 B): u64 key,
//!   u64 exit code (what the child finished with), u8 action, u8 count, ptr → targets (16 B as above).
//! - Parameter (16 B): u64 key, ptr → u32 ptr → the value as text, u16 length, u16 1.
//!
//! Keys and exit codes are names packed base 40, most significant first, padded to 12 characters
//! (`FUN_009ed220`: '-' 1, '/' 2, '0'–'9' 3–12, letters 13–38, '_' 39): `EXIT` = 0x684561a168000000.

use mw_bundle::{Reader, Resource};

use crate::Result;

/// A packed name (see the module doc) as text.
pub fn key_name(mut v: u64) -> String {
    const ALPHABET: &[u8; 40] = b"\0-/0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_";
    let mut digits = [0u8; 12];
    for d in digits.iter_mut().rev() {
        *d = (v % 40) as u8;
        v /= 40;
    }
    digits.iter().take_while(|&&d| d != 0).map(|&d| ALPHABET[d as usize] as char).collect()
}

/// The packed form of a name (inverse of [`key_name`]).
pub fn key(name: &str) -> u64 {
    let mut v = 0u64;
    let bytes: Vec<u8> = name.bytes().collect();
    for i in 0..12 {
        let c = bytes.get(i).map_or(0, |&c| match c.to_ascii_uppercase() {
            b'-' => 1,
            b'/' => 2,
            c @ b'0'..=b'9' => (c - b'0') as u64 + 3,
            c @ b'A'..=b'Z' => (c - b'A') as u64 + 13,
            b'_' => 39,
            _ => 0,
        });
        v = v.wrapping_mul(40).wrapping_add(c);
    }
    v
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    /// Exit code of the child it reacts to (`EXIT`, `TRUE` / `FALSE` for conditions, ...).
    pub exit: u64,
    pub action: u8,
    /// Nodes started.
    pub targets: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Child {
    pub id: u32,
    pub transitions: Vec<Transition>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub code: u64,
    pub action: u8,
    pub nodes: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub code: u32,
    pub id: u32,
    pub name: String,
    pub kind: u8,
    pub events: Vec<Event>,
    pub children: Vec<Child>,
    /// (key, value text).
    pub parameters: Vec<(u64, String)>,
    pub has_lua: bool,
}

impl Node {
    /// The children started on entry (its `ENTER` event).
    pub fn initial(&self) -> impl Iterator<Item = u32> + '_ {
        self.events.iter().filter(|e| e.code == key("ENTER")).flat_map(|e| e.nodes.iter().copied())
    }

    pub fn parameter(&self, name: &str) -> Option<&str> {
        let k = key(name);
        self.parameters.iter().find(|(p, _)| *p == k).map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, Default)]
pub struct HsmData {
    /// (node id, behaviour code, key).
    pub entries: Vec<(u32, u32, u64)>,
    pub nodes: Vec<Node>,
    /// Behaviour types: (key, name).
    pub behaviours: Vec<(u64, String)>,
}

fn cstr(r: &Reader, o: usize) -> String {
    if o == 0 {
        return String::new();
    }
    let mut out = Vec::new();
    let mut i = o;
    while let Ok(b) = r.u8(i) {
        if b == 0 || out.len() > 4096 {
            break;
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn targets(r: &Reader, ptr: usize, n: usize) -> Result<Vec<u32>> {
    (0..n).map(|k| r.u32(ptr + 16 * k + 8).map_err(Into::into)).collect()
}

impl HsmData {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let mut out = HsmData::default();
        let (entries, n_entries) = (r.u32(0x04)? as usize, r.u32(0x08)? as usize);
        for i in 0..n_entries.min(4096) {
            let o = entries + 12 * i;
            out.entries.push((r.u32(o)?, r.u32(o + 4)?, r.u32(o + 8)? as u64));
        }
        let (nodes, n_nodes) = (r.u32(0x0c)? as usize, r.u32(0x10)? as usize);
        for i in 0..n_nodes.min(1 << 16) {
            let o = nodes + 32 * i;
            let [code, id, events, children, params, lua, name] = r.u32_array::<7>(o)?;
            let (n_params, kind, n_events, n_children) =
                (r.u8(o + 0x1c)?, r.u8(o + 0x1d)?, r.u8(o + 0x1e)?, r.u8(o + 0x1f)?);
            let mut node = Node {
                code,
                id,
                name: cstr(&r, name as usize),
                kind,
                events: Vec::new(),
                children: Vec::new(),
                parameters: Vec::new(),
                has_lua: lua != 0,
            };
            for k in 0..n_events as usize {
                let e = events as usize + 16 * k;
                let count = r.u8(e + 9)? as usize;
                node.events.push(Event {
                    code: r.u64(e)?,
                    action: r.u8(e + 8)?,
                    nodes: targets(&r, r.u32(e + 12)? as usize, count)?,
                });
            }
            for k in 0..n_children as usize {
                let c = children as usize + 16 * k;
                let (child, tptr, tn) = (r.u32(c)?, r.u32(c + 4)? as usize, r.u32(c + 8)? as usize);
                let mut transitions = Vec::new();
                for j in 0..tn.min(64) {
                    let t = tptr + 24 * j;
                    let count = r.u8(t + 17)? as usize;
                    transitions.push(Transition {
                        exit: r.u64(t + 8)?,
                        action: r.u8(t + 16)?,
                        targets: targets(&r, r.u32(t + 20)? as usize, count)?,
                    });
                }
                node.children.push(Child { id: child, transitions });
            }
            for k in 0..n_params as usize {
                let p = params as usize + 16 * k;
                let value = r.u32(p + 8)? as usize;
                let text = if value == 0 { String::new() } else { cstr(&r, r.u32(value)? as usize) };
                node.parameters.push((r.u64(p)?, text));
            }
            out.nodes.push(node);
        }
        let (types, n_types) = (r.u32(0x1c)? as usize, r.u32(0x20)? as usize);
        for i in 0..n_types.min(4096) {
            let o = types + 32 * i;
            out.behaviours.push((r.u64(o)?, cstr(&r, r.u32(o + 8)? as usize)));
        }
        Ok(out)
    }

    pub fn node(&self, id: u32) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_names() {
        assert_eq!(key("EXIT"), 0x6845_61a1_6800_0000);
        assert_eq!(key("ENTER"), 0x66da_c3dd_d9c0_0000);
        assert_eq!(key_name(0x66da_c3fc_e24d_aa00), "ENTERTIME");
        assert_eq!(key_name(key("HSB_RUNCHILD")), "HSB_RUNCHILD");
    }
}
