//! Lua tables, a port of PUC-Rio 5.1.3 `ltable.c`: an array part for keys `1..n` and a hash part of
//! power-of-two size with chained scatter (Brent's variation, free slots taken from the end). The layout
//! decisions (main positions, `luaS_hash`, `hashnum`, the rehash size computation, re-insertion order) are
//! the original's, so `next` / `pairs` visit string, number and boolean keys in the same order as the game.
//! Tables and functions used as keys hash by a state-unique id where the original hashes their address.

use crate::value::Value;

/// `MAXBITS` in ltable.c 5.1: the array part holds at most 2^26 entries.
const MAXBITS: usize = 26;
const MAXASIZE: i64 = 1 << MAXBITS;

#[derive(Default)]
struct Node {
    key: Value,
    val: Value,
    next: Option<usize>,
}

#[derive(Clone, Copy)]
enum Slot {
    Array(usize),
    Node(usize),
}

pub struct Table {
    array: Vec<Value>,
    /// Empty = the shared `dummynode` of the original (size 1, never written).
    node: Vec<Node>,
    lastfree: usize,
    pub(crate) metatable: Option<crate::value::TableRef>,
}

/// `lua_number2int` + `luai_numeq` as used by `arrayindex` / `luaH_get`: the integer a number is equal to,
/// if any (32-bit `int`).
fn as_int(n: f64) -> Option<i64> {
    if n.fract() == 0.0 && n >= i32::MIN as f64 && n <= i32::MAX as f64 { Some(n as i64) } else { None }
}

/// `ceillog2` (luaO_log2(x - 1) + 1).
fn ceil_log2(x: usize) -> usize {
    if x <= 1 { 0 } else { (usize::BITS - (x - 1).leading_zeros()) as usize }
}

impl Table {
    /// `luaH_new(narray, nhash)`.
    pub fn new(narray: usize, nhash: usize) -> Self {
        let mut t = Table { array: vec![Value::Nil; narray], node: Vec::new(), lastfree: 0, metatable: None };
        t.set_node_vector(nhash);
        t
    }

    fn set_node_vector(&mut self, size: usize) {
        if size == 0 {
            self.node = Vec::new();
            self.lastfree = 0;
        } else {
            let size = 1usize << ceil_log2(size);
            self.node = (0..size).map(|_| Node::default()).collect();
            self.lastfree = size;
        }
    }

    /// `mainposition` (never called on the dummy node).
    fn main_position(&self, key: &Value) -> usize {
        let size = self.node.len() as u32;
        let hashmod = |h: u32| (h % ((size - 1) | 1)) as usize;
        match key {
            Value::Number(n) => {
                if *n == 0.0 {
                    0
                } else {
                    let b = n.to_bits();
                    hashmod((b as u32).wrapping_add((b >> 32) as u32))
                }
            }
            Value::Str(s) => (s.hash() & (size - 1)) as usize,
            Value::Bool(b) => (*b as u32 & (size - 1)) as usize,
            Value::LightUserdata(p) => hashmod(*p as u32),
            Value::Table(t) => hashmod(t.id()),
            Value::Function(f) => hashmod(f.id()),
            Value::Nil => 0,
        }
    }

    fn find_node(&self, key: &Value) -> Option<usize> {
        if self.node.is_empty() {
            return None;
        }
        let mut i = self.main_position(key);
        loop {
            if self.node[i].key.raw_eq(key) {
                return Some(i);
            }
            i = self.node[i].next?;
        }
    }

    /// `luaH_get` (the slot, even when it holds nil: a dead key keeps its node).
    fn find(&self, key: &Value) -> Option<Slot> {
        match key {
            Value::Nil => None,
            Value::Number(n) => {
                if let Some(k) = as_int(*n)
                    && k >= 1
                    && (k as usize) <= self.array.len()
                {
                    return Some(Slot::Array(k as usize - 1));
                }
                self.find_node(key).map(Slot::Node)
            }
            _ => self.find_node(key).map(Slot::Node),
        }
    }

    fn read(&self, s: Slot) -> &Value {
        match s {
            Slot::Array(i) => &self.array[i],
            Slot::Node(i) => &self.node[i].val,
        }
    }

    fn write(&mut self, s: Slot, v: Value) {
        match s {
            Slot::Array(i) => self.array[i] = v,
            Slot::Node(i) => self.node[i].val = v,
        }
    }

    pub fn get(&self, key: &Value) -> Value {
        self.find(key).map(|s| self.read(s).clone()).unwrap_or_default()
    }

    pub fn get_int(&self, k: i64) -> Value {
        self.get(&Value::Number(k as f64))
    }

    /// `luaH_set` + assignment. Like the original, assigning nil to a missing key still creates the key.
    pub fn set(&mut self, key: Value, val: Value) -> Result<(), &'static str> {
        match &key {
            Value::Nil => return Err("table index is nil"),
            Value::Number(n) if n.is_nan() => return Err("table index is NaN"),
            _ => {}
        }
        let slot = self.slot_for(key);
        self.write(slot, val);
        Ok(())
    }

    /// `luaV_settable`'s table step: create the key (like `luaH_set`), then store unless the value there is
    /// nil and the table has a `__newindex` metamethod. Returns whether it stored.
    pub fn set_unless_newindex(&mut self, key: Value, val: Value, has_newindex: bool) -> Result<bool, &'static str> {
        match &key {
            Value::Nil => return Err("table index is nil"),
            Value::Number(n) if n.is_nan() => return Err("table index is NaN"),
            _ => {}
        }
        let slot = self.slot_for(key);
        if !self.read(slot).is_nil() || !has_newindex {
            self.write(slot, val);
            return Ok(true);
        }
        Ok(false)
    }

    pub fn set_int(&mut self, k: i64, val: Value) {
        let slot = self.slot_for(Value::Number(k as f64));
        self.write(slot, val);
    }

    fn slot_for(&mut self, key: Value) -> Slot {
        match self.find(&key) {
            Some(s) => s,
            None => self.new_key(key),
        }
    }

    /// `getfreepos`: scan down from `lastfree` for a node with a nil key.
    fn get_free_pos(&mut self) -> Option<usize> {
        while self.lastfree > 0 {
            self.lastfree -= 1;
            if self.node[self.lastfree].key.is_nil() {
                return Some(self.lastfree);
            }
        }
        None
    }

    /// `newkey`: insert a key that is not present.
    fn new_key(&mut self, key: Value) -> Slot {
        let mut mp = if self.node.is_empty() { usize::MAX } else { self.main_position(&key) };
        if mp == usize::MAX || !self.node[mp].val.is_nil() {
            let Some(n) = self.get_free_pos() else {
                self.rehash(&key);
                return self.slot_for(key);
            };
            let othern = self.main_position(&self.node[mp].key);
            if othern != mp {
                // The colliding node is out of its main position: move it to the free slot.
                let mut o = othern;
                while self.node[o].next != Some(mp) {
                    o = self.node[o].next.expect("broken hash chain");
                }
                self.node[o].next = Some(n);
                let moved = std::mem::take(&mut self.node[mp]);
                self.node[n] = moved;
                self.node[mp].next = None;
                self.node[mp].val = Value::Nil;
            } else {
                // The colliding node is in its own main position: the new key goes to the free slot.
                self.node[n].next = self.node[mp].next;
                self.node[mp].next = Some(n);
                mp = n;
            }
        }
        self.node[mp].key = key;
        Slot::Node(mp)
    }

    fn count_int(key: &Value, nums: &mut [usize; MAXBITS + 1]) -> usize {
        if let Value::Number(n) = key
            && let Some(k) = as_int(*n)
            && k > 0
            && k <= MAXASIZE
        {
            nums[ceil_log2(k as usize)] += 1;
            return 1;
        }
        0
    }

    fn num_use_array(&self, nums: &mut [usize; MAXBITS + 1]) -> usize {
        let mut ause = 0;
        let mut i = 1usize;
        let mut ttlg = 1usize;
        for slot in nums.iter_mut() {
            let mut lc = 0;
            let mut lim = ttlg;
            if lim > self.array.len() {
                lim = self.array.len();
                if i > lim {
                    break;
                }
            }
            while i <= lim {
                if !self.array[i - 1].is_nil() {
                    lc += 1;
                }
                i += 1;
            }
            *slot += lc;
            ause += lc;
            ttlg *= 2;
        }
        ause
    }

    fn num_use_hash(&self, nums: &mut [usize; MAXBITS + 1], nasize: &mut usize) -> usize {
        let mut total = 0;
        let mut ause = 0;
        for n in self.node.iter().rev() {
            if !n.val.is_nil() {
                ause += Self::count_int(&n.key, nums);
                total += 1;
            }
        }
        *nasize += ause;
        total
    }

    fn compute_sizes(nums: &[usize; MAXBITS + 1], narray: &mut usize) -> usize {
        let mut a = 0;
        let mut na = 0;
        let mut n = 0;
        let mut twotoi = 1usize;
        let mut i = 0;
        while twotoi / 2 < *narray && i <= MAXBITS {
            if nums[i] > 0 {
                a += nums[i];
                if a > twotoi / 2 {
                    n = twotoi;
                    na = a;
                }
            }
            if a == *narray {
                break;
            }
            i += 1;
            twotoi *= 2;
        }
        *narray = n;
        na
    }

    fn rehash(&mut self, extra: &Value) {
        let mut nums = [0usize; MAXBITS + 1];
        let mut nasize = self.num_use_array(&mut nums);
        let mut total = nasize;
        total += self.num_use_hash(&mut nums, &mut nasize);
        nasize += Self::count_int(extra, &mut nums);
        total += 1;
        let na = Self::compute_sizes(&nums, &mut nasize);
        self.resize(nasize, total - na);
    }

    /// `resize` (luaH_resize): grow / shrink the array, rebuild the hash part, re-insert the vanishing array
    /// slice, then the old nodes from the last to the first.
    fn resize(&mut self, nasize: usize, nhsize: usize) {
        let oldasize = self.array.len();
        let old = std::mem::take(&mut self.node);
        if nasize > oldasize {
            self.array.resize(nasize, Value::Nil);
        }
        self.set_node_vector(nhsize);
        if nasize < oldasize {
            let vanishing: Vec<Value> = self.array.drain(nasize..).collect();
            for (j, v) in vanishing.into_iter().enumerate() {
                if !v.is_nil() {
                    self.set_int((nasize + j + 1) as i64, v);
                }
            }
        }
        for n in old.into_iter().rev() {
            if !n.val.is_nil() {
                let slot = self.slot_for(n.key);
                self.write(slot, n.val);
            }
        }
    }

    /// `luaH_resizearray`, used by SETLIST.
    pub fn resize_array(&mut self, nasize: usize) {
        let nsize = self.node.len();
        self.resize(nasize, nsize);
    }

    pub fn array_size(&self) -> usize {
        self.array.len()
    }

    /// `luaH_getn`: a border of the table (binary search in the array part, unbound search in the hash).
    pub fn getn(&self) -> usize {
        let mut j = self.array.len();
        if j > 0 && self.array[j - 1].is_nil() {
            let mut i = 0;
            while j - i > 1 {
                let m = (i + j) / 2;
                if self.array[m - 1].is_nil() {
                    j = m;
                } else {
                    i = m;
                }
            }
            return i;
        }
        if self.node.is_empty() {
            return j;
        }
        // unbound_search
        let mut i = j;
        j += 1;
        while !self.get_int(j as i64).is_nil() {
            i = j;
            j *= 2;
            if j > i32::MAX as usize {
                let mut i = 1;
                while !self.get_int(i).is_nil() {
                    i += 1;
                }
                return (i - 1) as usize;
            }
        }
        while j - i > 1 {
            let m = (i + j) / 2;
            if self.get_int(m as i64).is_nil() {
                j = m;
            } else {
                i = m;
            }
        }
        i
    }

    /// `luaH_next`: the pair after `key` (nil = first), array part first, then the nodes in slot order.
    /// `Err` for a key that is not in the table.
    #[allow(clippy::result_unit_err)]
    pub fn next(&self, key: &Value) -> Result<Option<(Value, Value)>, ()> {
        let start = match key {
            Value::Nil => 0,
            Value::Number(n) if as_int(*n).is_some_and(|k| k >= 1 && (k as usize) <= self.array.len()) => *n as usize,
            _ => self.find_node(key).ok_or(())? + self.array.len() + 1,
        };
        let mut i = start;
        while i < self.array.len() {
            if !self.array[i].is_nil() {
                return Ok(Some((Value::Number((i + 1) as f64), self.array[i].clone())));
            }
            i += 1;
        }
        let mut j = i - self.array.len();
        while j < self.node.len() {
            let n = &self.node[j];
            if !n.val.is_nil() {
                return Ok(Some((n.key.clone(), n.val.clone())));
            }
            j += 1;
        }
        Ok(None)
    }
}
