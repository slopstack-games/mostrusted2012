//! GeneSys: Criterion's reflected data system. `GenesysType` (0x14) resources
//! describe types; `GenesysObject` (0x15) resources are binary instances.
//!
//! Most of the game's configuration lives here: vehicle physics
//! (`RaceCarPhysicalDefinition`, `Engine`, `Gearbox`, `Tyre`...), gameplay
//! rules, traffic, audio. Type names are stored as strings, field names only
//! as 32-bit hashes ([`field_hash`]). Names recovered by dictionary attack are
//! listed in `genesys_names.txt` ([`field_name`]).
//!
//! Type (chunk 0):
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x01 | u8   | kind (see [`Kind`]) |
//! | 0x02 | u16  | field count |
//! | 0x04 | ptr  | base type (import) |
//! | 0x10 | u64  | type hash |
//! | 0x18 | u32  | size |
//! | 0x20 | ptr  | → ptr → type name |
//! | 0x24 | 0x1C × n | fields |
//!
//! Field: `{type import, u32 type hash, u32 0, u32 name hash, u32 size,
//! u32 offset, u16 count, u16 flags}`. Flags: low byte = alignment; `0x100` =
//! pointer to a sub-object (in the same chunk, or an import = external
//! object); `0x1000` = the array holds pointers to elements; `0x800` = dynamic array (pointer; its length is one of the trailing
//! small unsigned fields, assigned in declaration order); `0x200` = fixed
//! inline array of `count`.
//!
//! Every struct begins with the `Genesys.Object` header: the type pointer at
//! +0 (an import), so a sub-object's type can be read at its own offset.

use mw_bundle::{ImportKind, Reader, Resource, ResourceId};

use crate::{Error, Result};

const CRC_POLY: u32 = 0x04c1_1db7;

const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = (i as u32) << 24;
        let mut k = 0;
        while k < 8 {
            c = if c & 0x8000_0000 != 0 { (c << 1) ^ CRC_POLY } else { c << 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

const CRC_TABLE: [u32; 256] = crc_table();

/// Hash of a GeneSys field name.
///
/// CRC-32 (polynomial 0x04C11DB7, MSB first, not reflected) computed with
/// the shift-register ("non-augmented") algorithm: the register starts as the
/// complement of the first four bytes, each further byte is shifted in at the
/// bottom while the byte shifted out is reduced through the table, and the
/// result is complemented. Names of four bytes therefore hash to themselves
/// (`Mass` = 0x4d617373). Found from nested type names (`Engine.Sound` is held
/// in a field named `Sound`), see docs/research-log.md (2026-10-04).
pub const fn field_hash(name: &str) -> u32 {
    let b = name.as_bytes();
    let mut h = FieldHasher::new();
    let mut i = 0;
    while i < b.len() {
        h = h.push(b[i]);
        i += 1;
    }
    h.finish()
}

/// Incremental [`field_hash`]: a hasher can be cloned after a common prefix
/// (used by `mwtool genesys-crack` to search name candidates quickly).
#[derive(Debug, Clone, Copy, Default)]
pub struct FieldHasher {
    reg: u32,
    len: usize,
}

impl FieldHasher {
    pub const fn new() -> Self {
        Self { reg: 0, len: 0 }
    }

    #[inline]
    pub const fn push(self, byte: u8) -> Self {
        let reg = if self.len < 4 {
            (self.reg << 8) | (byte ^ 0xff) as u32
        } else {
            ((self.reg << 8) | byte as u32) ^ CRC_TABLE[(self.reg >> 24) as usize]
        };
        Self { reg, len: self.len + 1 }
    }

    #[inline]
    pub fn push_all(mut self, bytes: &[u8]) -> Self {
        for &b in bytes {
            self = self.push(b);
        }
        self
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub const fn finish(&self) -> u32 {
        if self.len < 4 {
            // Shorter names: unverified (none occur in the install); packed like four-byte names.
            return !self.reg & (u32::MAX >> (8 * (4 - self.len)));
        }
        !self.reg
    }
}

/// Field names recovered from their hashes (one per line).
const NAMES: &str = include_str!("genesys_names.txt");

/// The name of a field hash, if it is known.
pub fn field_name(hash: u32) -> Option<&'static str> {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static MAP: OnceLock<HashMap<u32, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        NAMES
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|n| (field_hash(n), n))
            .collect()
    })
    .get(&hash)
    .copied()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Int,
    UInt,
    Float,
    Bool,
    Char,
    Enum,
    /// Compound value types: vectors, matrices, strings.
    Value,
    Struct,
    /// `CgsResource.Handle`: a resource ID.
    Handle,
    Unknown(u8),
}

impl Kind {
    fn from_raw(v: u8) -> Self {
        match v {
            0 => Self::Int,
            1 => Self::UInt,
            2 => Self::Float,
            3 => Self::Bool,
            4 => Self::Char,
            5 => Self::Enum,
            6 => Self::Value,
            7 => Self::Struct,
            8 => Self::Handle,
            o => Self::Unknown(o),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name_hash: u32,
    pub ty: Option<ResourceId>,
    pub offset: u32,
    pub size: u32,
    pub count: u16,
    pub flags: u16,
}

impl Field {
    pub fn is_pointer(&self) -> bool {
        self.flags & 0x100 != 0
    }
    pub fn is_array(&self) -> bool {
        self.flags & 0x800 != 0
    }
    pub fn is_fixed_array(&self) -> bool {
        self.flags & 0x200 != 0
    }
    /// Array whose elements are pointers to sub-objects (flag `0x1000`).
    pub fn is_pointer_array(&self) -> bool {
        self.flags & 0x1000 != 0
    }
}

#[derive(Debug, Clone)]
pub struct Type {
    pub id: ResourceId,
    pub name: String,
    pub kind: Kind,
    pub size: u32,
    pub base: Option<ResourceId>,
    pub fields: Vec<Field>,
}

impl Type {
    pub fn parse(res: &Resource) -> Result<Self> {
        let data = res.header();
        let r = Reader(data);
        let kind = Kind::from_raw(r.u8(1)?);
        let n = r.u16(2)? as usize;
        let name_ptr = r.u32(r.u32(0x20)? as usize)? as usize;
        let name = data
            .get(name_ptr..)
            .map(|s| String::from_utf8_lossy(&s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]).into_owned())
            .unwrap_or_default();
        let mut fields = Vec::new();
        if kind == Kind::Struct {
            for i in 0..n {
                let o = 0x24 + i * 0x1C;
                fields.push(Field {
                    ty: res.import_at(o as u32),
                    name_hash: r.u32(o + 0x0C)?,
                    size: r.u32(o + 0x10)?,
                    offset: r.u32(o + 0x14)?,
                    count: r.u16(o + 0x18)?,
                    flags: r.u16(o + 0x1A)?,
                });
            }
        }
        Ok(Self { id: res.id, name, kind, size: r.u32(0x18)?, base: res.import_at(0x04), fields })
    }

    /// For each array field (by index into `fields`), the index of the
    /// field holding its length.
    pub fn array_counts(&self, types: &dyn Fn(ResourceId) -> Option<Type>) -> Vec<(usize, Option<usize>)> {
        // A derived type starts with its base type's fields, whose counts sit in the base's block
        // (WaveSelectSequenceItem extends WaveSequenceItem).
        let base = self.base.filter(|&b| b != self.id).and_then(types).filter(|b| {
            !b.fields.is_empty()
                && b.fields.len() <= self.fields.len()
                && b.fields.iter().zip(&self.fields).all(|(x, y)| x.name_hash == y.name_hash && x.offset == y.offset)
        });
        let first = base.as_ref().map_or(0, |b| b.fields.len());
        let mut out = base.map(|b| b.array_counts(types)).unwrap_or_default();
        let mut arrays: Vec<usize> = (first..self.fields.len()).filter(|&i| self.fields[i].is_array()).collect();
        let Some(&last) = arrays.iter().max() else { return out };
        let last_ptr =
            (first..self.fields.len()).filter(|&i| self.fields[i].is_array() || self.fields[i].is_pointer()).max();
        let start = last_ptr.unwrap_or(last) + 1;
        let counts: Vec<usize> = (start..self.fields.len())
            .filter(|&i| {
                let f = &self.fields[i];
                !f.is_array()
                    && !f.is_pointer()
                    && f.size <= 2
                    && f.ty.and_then(types).is_some_and(|t| matches!(t.kind, Kind::UInt | Kind::Int))
            })
            .collect();
        let mut counts = counts;
        // Pairs the general rule gets wrong, verified on the data: a WaveSequenceItem's `Waves` handles are
        // counted by its second trailing u8 (`bc514b0c`, also the `Waves` count of SpeedSoundSet layers, where
        // the array pointers' spacing confirms it), the ModulatingDoubleValue array by the first.
        for &(array, count) in KNOWN_COUNTS {
            let a = arrays.iter().position(|&i| self.fields[i].name_hash == array);
            let c = counts.iter().position(|&i| self.fields[i].name_hash == count);
            if let (Some(a), Some(c)) = (a, c) {
                out.push((arrays.remove(a), Some(counts.remove(c))));
            }
        }
        // Counts follow the arrays ordered by alignment (descending), then
        // offset: verified on RigidBody, whose 1-byte-aligned bool array is
        // laid out first but counted last.
        arrays.sort_by_key(|&i| (std::cmp::Reverse(self.fields[i].flags & 0xFF), self.fields[i].offset));
        out.extend(arrays.iter().enumerate().map(|(k, &a)| (a, counts.get(k).copied())));
        out
    }
}

/// (array field, count field) name hashes that override the positional rule in [`Type::array_counts`].
const KNOWN_COUNTS: &[(u32, u32)] = &[(field_hash("Waves"), 0xbc51_4b0c)];

/// A decoded GeneSys value.
#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    UInt(u64),
    Float(f32),
    Bool(bool),
    Vector([f32; 4]),
    Matrix([[f32; 4]; 4]),
    Handle(Option<ResourceId>),
    /// Reference to another GenesysObject resource.
    Ref(ResourceId),
    Struct(Object),
    Array(Vec<Value>),
    Null,
    /// `CgsCore.StringBase`: `i32` offset from the field to the text, `u32` length including the NUL.
    /// The sound data's bindings are such strings (`"LocalPlayer.GetPlayerSpeed"`).
    Str(String),
    Raw(Vec<u8>),
}

/// A decoded struct instance.
#[derive(Debug, Clone)]
pub struct Object {
    pub type_name: String,
    pub type_id: ResourceId,
    /// (field name hash, value) in declaration order.
    pub fields: Vec<(u32, Value)>,
}

impl Object {
    /// Field by name hash.
    pub fn get(&self, hash: u32) -> Option<&Value> {
        self.fields.iter().find(|(h, _)| *h == hash).map(|(_, v)| v)
    }

    /// The n-th float field (declaration order), handy while names are unknown.
    pub fn floats(&self) -> Vec<f32> {
        self.fields.iter().filter_map(|(_, v)| if let Value::Float(f) = v { Some(*f) } else { None }).collect()
    }

    /// First nested struct (direct field, pointer or array element) of the given type name.
    pub fn find(&self, type_name: &str) -> Option<&Object> {
        fn walk<'a>(v: &'a Value, name: &str) -> Option<&'a Object> {
            match v {
                Value::Struct(o) if o.type_name == name => Some(o),
                Value::Struct(o) => o.find(name),
                Value::Array(a) => a.iter().find_map(|v| walk(v, name)),
                _ => None,
            }
        }
        self.fields.iter().find_map(|(_, v)| walk(v, type_name))
    }
}

/// Decoder for one GenesysObject resource.
pub struct Decoder<'a> {
    res: &'a Resource,
    types: &'a dyn Fn(ResourceId) -> Option<Type>,
    depth: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(res: &'a Resource, types: &'a dyn Fn(ResourceId) -> Option<Type>) -> Self {
        Self { res, types, depth: 0 }
    }

    /// Decode the root object (type from the import at offset 0).
    pub fn root(&mut self) -> Result<Object> {
        let ty = self.res.import_at(0).ok_or_else(|| Error::Malformed("GenesysObject without type".into()))?;
        self.object(ty, 0)
    }

    fn object(&mut self, ty: ResourceId, at: usize) -> Result<Object> {
        let t = (self.types)(ty).ok_or_else(|| Error::Malformed(format!("unknown GeneSys type {ty}")))?;
        if self.depth > 32 {
            return Err(Error::Malformed("GeneSys nesting too deep".into()));
        }
        self.depth += 1;
        let counts = t.array_counts(self.types);
        let r = Reader(self.res.header());
        let mut fields = Vec::with_capacity(t.fields.len());
        for (i, f) in t.fields.iter().enumerate() {
            let o = at + f.offset as usize;
            let value = if f.is_array() {
                let ptr = r.u32(o)? as usize;
                let ptr_null = ptr == 0;
                let n = match counts.iter().find(|(a, _)| *a == i).and_then(|(_, c)| *c) {
                    Some(ci) => {
                        let cf = &t.fields[ci];
                        let co = at + cf.offset as usize;
                        if cf.size == 1 { r.u8(co)? as usize } else { r.u16(co)? as usize }
                    }
                    None => 0,
                };
                let n = if ptr_null { 0 } else { n };
                let elem = f.ty.and_then(self.types);
                let handles = elem.as_ref().is_some_and(|e| e.kind == Kind::Handle) && !f.is_pointer_array();
                let n = if !ptr_null && handles { self.handle_array_len(&t, at, ptr, n) } else { n };
                let stride = elem.as_ref().map_or(f.size, |e| e.size.max(1)) as usize;
                let stride = align(stride, (f.flags & 0xFF).max(1) as usize);
                let mut items = Vec::with_capacity(n.min(4096));
                for k in 0..n.min(4096) {
                    if f.is_pointer_array() {
                        let p = r.u32(ptr + k * 4)? as usize;
                        items.push(match (p, self.res.import_at(p as u32)) {
                            (0, _) => Value::Null,
                            (_, Some(sub)) => Value::Struct(self.object(sub, p)?),
                            _ => self.value(f, elem.as_ref(), p)?,
                        });
                    } else {
                        items.push(self.value(f, elem.as_ref(), ptr + k * stride)?);
                    }
                }
                Value::Array(items)
            } else if f.is_pointer() {
                match self.res.imports.iter().find(|imp| imp.offset as usize == o) {
                    Some(imp) => Value::Ref(imp.id),
                    None => {
                        let ptr = r.u32(o)? as usize;
                        match (ptr, self.res.import_at(ptr as u32)) {
                            (0, _) => Value::Null,
                            (_, Some(sub)) => Value::Struct(self.object(sub, ptr)?),
                            _ => Value::Raw(r.bytes(ptr, f.size as usize).map(<[u8]>::to_vec).unwrap_or_default()),
                        }
                    }
                }
            } else if f.is_fixed_array() && f.count > 1 {
                let elem = f.ty.and_then(self.types);
                let stride = elem.as_ref().map_or(f.size, |e| e.size.max(1)) as usize;
                let mut items = Vec::with_capacity(f.count as usize);
                for k in 0..f.count as usize {
                    items.push(self.value(f, elem.as_ref(), o + k * stride)?);
                }
                Value::Array(items)
            } else {
                let elem = f.ty.and_then(self.types);
                self.value(f, elem.as_ref(), o)?
            };
            fields.push((f.name_hash, value));
        }
        self.depth -= 1;
        Ok(Object { type_name: t.name, type_id: ty, fields })
    }

    /// Length of a handle array, checked against the import table. The count fields of some types are not
    /// matched to their arrays by the positional rule (EngineSound2's twelve handle lists), but handle arrays
    /// are laid out back to back: when every 8-byte slot up to the object's next array holds a handle import,
    /// that run is the array; otherwise the count is only clamped so it doesn't run into the next array.
    fn handle_array_len(&self, t: &Type, at: usize, ptr: usize, n: usize) -> usize {
        let r = Reader(self.res.header());
        let next = t
            .fields
            .iter()
            .filter(|f| f.is_array())
            .filter_map(|f| r.u32(at + f.offset as usize).ok().map(|p| p as usize))
            .filter(|&p| p > ptr)
            .min();
        let Some(next) = next else { return n };
        let slots = (next - ptr) / 8;
        let is_handle =
            |o: usize| self.res.imports.iter().any(|i| i.offset as usize == o && i.kind == ImportKind::Handle);
        if slots > 0 && slots <= 4096 && (0..slots).all(|k| is_handle(ptr + k * 8)) { slots } else { n.min(slots) }
    }

    fn value(&mut self, f: &Field, ty: Option<&Type>, o: usize) -> Result<Value> {
        let r = Reader(self.res.header());
        let Some(t) = ty else {
            return Ok(Value::Raw(r.bytes(o, f.size as usize).map(<[u8]>::to_vec).unwrap_or_default()));
        };
        Ok(match t.kind {
            Kind::Float => Value::Float(r.f32(o)?),
            Kind::Bool => Value::Bool(r.u8(o)? != 0),
            Kind::Int | Kind::Enum => Value::Int(match t.size {
                1 => r.u8(o)? as i8 as i64,
                2 => r.i16(o)? as i64,
                8 => r.u64(o)? as i64,
                _ => r.i32(o)? as i64,
            }),
            Kind::UInt | Kind::Char => Value::UInt(match t.size {
                1 => r.u8(o)? as u64,
                2 => r.u16(o)? as u64,
                8 => r.u64(o)?,
                _ => r.u32(o)? as u64,
            }),
            Kind::Handle => Value::Handle(
                self.res
                    .imports
                    .iter()
                    .find(|i| i.offset as usize == o && i.kind == ImportKind::Handle)
                    .map(|i| i.id)
                    .or_else(|| r.u64(o).ok().filter(|&v| v != 0).map(ResourceId)),
            ),
            Kind::Struct => Value::Struct(self.object(t.id, o)?),
            Kind::Value if t.name.contains("Matrix44") => {
                let mut m = [[0.0; 4]; 4];
                for (i, row) in m.iter_mut().enumerate() {
                    *row = r.f32_array(o + i * 16)?;
                }
                Value::Matrix(m)
            }
            Kind::Value if t.name.contains("Vector") => Value::Vector(r.f32_array(o)?),
            Kind::Value if t.name == "CgsCore.StringBase" => {
                let (rel, len) = (r.i32(o)?, r.u32(o + 4)? as usize);
                let at = (o as i64 + i64::from(rel)) as usize;
                match r.bytes(at, len).ok().filter(|_| len > 0 && rel != 0) {
                    Some(b) => {
                        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
                        Value::Str(String::from_utf8_lossy(&b[..end]).into_owned())
                    }
                    None => Value::Str(String::new()),
                }
            }
            _ => Value::Raw(r.bytes(o, t.size as usize).map(<[u8]>::to_vec).unwrap_or_default()),
        })
    }
}

fn align(v: usize, a: usize) -> usize {
    v.div_ceil(a) * a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_hashes() {
        // Pairs observed in the install (types and objects).
        assert_eq!(field_hash("Mass"), 0x4d617373);
        assert_eq!(field_hash("Sound"), 0xab56a3f1);
        assert_eq!(field_hash("Power_curve"), 0xa1a3b0a2);
        assert_eq!(field_hash("Take_off_behaviour"), 0xc63c4447);
        assert_eq!(field_hash("Normal_quality_engine"), 0xbf36eecf);
    }

    #[test]
    fn names_table_has_no_collisions() {
        let mut seen = std::collections::HashMap::new();
        for n in NAMES.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            if let Some(prev) = seen.insert(field_hash(n), n) {
                panic!("{n} and {prev} share a hash");
            }
        }
        assert_eq!(field_name(0xab56a3f1), Some("Sound"));
    }
}
