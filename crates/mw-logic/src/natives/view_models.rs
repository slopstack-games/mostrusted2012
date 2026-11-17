//! The view models bindings read, resolved like the original's binding manager (docs/decomp.md "Native
//! registries", "Binding evaluator"). Natives resolve bindings through this (`ResolveBoolBinding`, ...), the HSM
//! host reads its conditions here, widgets bind to it.
//!
//! A path's root is looked up in two tables, in this order (`FUN_00a4ef40`):
//! 1. **Registered roots** (`FUN_00a4eb00` over the root hash table `FUN_00a520d0`, records of 0x14 at +0x12968 of
//!    the binding manager, filled by `FUN_00a4d2a0`(root, type, object, description) and emptied by
//!    `FUN_00a4d3f0`): the managers' view models (`Mode` = `GameModeManager`, `Players` = `PlayerManager`,
//!    `Signals` = `UISignalGeneratorManager`, ...: 99 registrations, listed in decomp.md). Their members are typed
//!    fields or getter functions of the manager. Here: [`Signals`] is ported (pure logic over the UI clock); the
//!    engine's roots are [`Provider`]s the runtime registers with [`ViewModels::register_root`].
//! 2. **Dynamic providers** (`FUN_00a58f70`, a sorted table of root hashes at +0x14984 with their objects at
//!    +0x14a50): the data definitions HSM states publish under their path while they run (`FUN_00a4ead0` →
//!    `FUN_00a58fd0` inserts or replaces by hash(path); `FUN_00a4de20` → `FUN_00a559b0` removes). A published
//!    definition answers through its vtable slot 1 (`FUN_00774610`): the JSON walk `FUN_009f61c0` ([`walk_json`]),
//!    then its binding adapters (the definition's `adapters` member, not ported).
//!
//! A registered root whose path does not resolve falls through to the dynamic table under the same hash, as in
//! the dispatcher.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::binding::{self, Binding, Host, Index, Lookup, Provider, Segment, Value};
use mw_formats::json::Json;

/// A registered root's view model, as the runtime provides it.
pub type RootProvider = Box<dyn Provider + Send + Sync>;

/// Every view model bindings can read.
#[derive(Default)]
pub struct ViewModels {
    /// Published data-definition view models by path, in publish order (one per root hash, as the original's
    /// table).
    pub published: Vec<(String, Json)>,
    /// The UI signal generator (`Signals`), a registered root of the original that is pure logic.
    pub signals: Signals,
    /// Engine roots registered by the runtime, by root hash (name kept for listings).
    roots: HashMap<u32, (String, RootProvider)>,
}

impl std::fmt::Debug for ViewModels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut roots: Vec<&str> = self.roots.values().map(|(n, _)| n.as_str()).collect();
        roots.sort();
        f.debug_struct("ViewModels")
            .field("published", &self.published)
            .field("signals", &self.signals)
            .field("roots", &roots)
            .finish()
    }
}

impl ViewModels {
    /// `ResolveBoolBinding` / an HSM condition (`FUN_0067f4e0`: inner-form parse, the bool visitor). `None` when
    /// the text does not parse; otherwise the visitor's answer, which for a path that does not resolve is false
    /// (true with `IS_EMPTY`), as in the original.
    pub fn resolve_bool(&self, binding: &str) -> Option<bool> {
        Binding::parse(binding).ok().map(|b| b.eval_bool(self).value)
    }
    /// `ResolveIntBinding` (`FUN_006794e0`, the int visitor): `None` when the text does not parse; an unresolved
    /// path gives 0 (1 with `IS_EMPTY`).
    pub fn resolve_int(&self, binding: &str) -> Option<i64> {
        Binding::parse(binding).ok().map(|b| b.eval_int(self).value as i64)
    }
    /// `ResolveDoubleBinding` (`FUN_00679550`, the double visitor): `None` when the text does not parse.
    pub fn resolve_double(&self, binding: &str) -> Option<f64> {
        Binding::parse(binding).ok().map(|b| b.eval_double(self).value)
    }
    /// `ResolveStringBinding` (`FUN_007e7cc0`, the to-text visitor): `None` when the text does not parse or the
    /// path does not resolve (the original then returns "").
    pub fn resolve_string(&self, binding: &str) -> Option<String> {
        let r = Binding::parse(binding).ok()?.eval_text(self);
        r.ok.then_some(r.value)
    }
    /// The HSM's string reads (`FUN_00a4f2e0` with the string visitor, `HSB_SEL_EX_B`, parameter values):
    /// `None` when the text does not parse or the path does not resolve.
    pub fn resolve_hsm_string(&self, binding: &str) -> Option<String> {
        let r = Binding::parse(binding).ok()?.eval_string(self);
        r.ok.then_some(r.value)
    }

    /// An HSM state with a data definition became active (`FUN_00a4ead0` → `FUN_00a58fd0`: a path already
    /// published is replaced).
    pub fn publish(&mut self, path: &str, data: Json) {
        let h = binding::hash(path);
        match self.published.iter_mut().find(|(p, _)| binding::hash(p) == h) {
            Some(slot) => *slot = (path.to_string(), data),
            None => self.published.push((path.to_string(), data)),
        }
    }
    /// ... and finished (`FUN_00a4de20` → `FUN_00a559b0`).
    pub fn retract(&mut self, path: &str) {
        let h = binding::hash(path);
        self.published.retain(|(p, _)| binding::hash(p) != h);
    }
    /// The published view model under this root name.
    pub fn published(&self, root: &str) -> Option<&Json> {
        self.published_hash(binding::hash(root))
    }
    fn published_hash(&self, h: u32) -> Option<&Json> {
        self.published.iter().find(|(p, _)| binding::hash(p) == h).map(|(_, j)| j)
    }

    /// Register an engine root (`FUN_00a4d2a0`); a root of the same name is replaced, as in the original.
    pub fn register_root(&mut self, name: &str, provider: impl Provider + Send + Sync + 'static) {
        self.roots.insert(binding::hash(name), (name.to_string(), Box::new(provider)));
    }
    /// `FUN_00a4d3f0`.
    pub fn unregister_root(&mut self, name: &str) {
        self.roots.remove(&binding::hash(name));
    }
    /// The registered engine roots' names (sorted), plus `Signals`.
    pub fn root_names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.roots.values().map(|(n, _)| n.as_str()).collect();
        v.push("Signals");
        v.sort();
        v
    }

    fn registered(&self, root: u32, members: &[Segment]) -> Option<Lookup> {
        if root == *SIGNALS {
            return Some(self.signals.lookup(members));
        }
        let (_, p) = self.roots.get(&root)?;
        Some(match p.lookup(members) {
            // `_length` on a list member the provider does not answer itself (the original registers `_length`
            // members on its list types).
            Lookup::Null => match members.split_last() {
                Some((last, rest)) if last.hash == *LENGTH => match p.lookup(rest) {
                    Lookup::Found(Value::List(n)) => Lookup::Found(Value::UInt(n)),
                    _ => Lookup::Null,
                },
                _ => Lookup::Null,
            },
            r => r,
        })
    }
}

impl Host for ViewModels {
    fn lookup(&self, path: &[Segment]) -> Lookup {
        let Some((root, members)) = path.split_first() else { return Lookup::Null };
        let fixed = self.registered(root.hash, members);
        if let Some(r @ (Lookup::Found(_) | Lookup::Unsupported)) = &fixed {
            return r.clone();
        }
        if let Some(json) = self.published_hash(root.hash) {
            return walk_json(json, path);
        }
        fixed.unwrap_or(Lookup::Null)
    }
}

static SIGNALS: std::sync::LazyLock<u32> = std::sync::LazyLock::new(|| binding::hash("Signals"));
/// `DAT_0118a094` = hash("_length").
static LENGTH: std::sync::LazyLock<u32> = std::sync::LazyLock::new(|| binding::hash("_length"));

/// The number a segment's index record holds (+0x58 of the binding): `[n]` as atol, `["key"]` as hash(key).
fn index_value(i: &Index) -> u32 {
    match i {
        Index::Pos(n) => *n as u32,
        Index::Key { hash, .. } => *hash,
    }
}

/// A published data definition's JSON read along a binding path (`FUN_009f61c0`, from the segment after the
/// root):
/// - object: the next segment's name looked up by hash (first match in file order); no segment left or no such
///   key → not resolved;
/// - array: `_length` as the next segment → the item count (u32); else the item at the *previous* segment's
///   index (1-based, default 1: `Items[2].Name` takes item 2, `Items` alone item 1), 0 or past the count → out of
///   range; the walk goes on with the item at the same segment;
/// - leaves: string, int (i64 slot), double, bool, null whatever segments follow.
///
/// Not ported: the vector read (an array at the last segment read by a vector visitor gives up to 4 numbers);
/// the binding module has no vector result type yet.
pub fn walk_json(json: &Json, path: &[Segment]) -> Lookup {
    let mut node = json;
    let mut i = 1;
    loop {
        match node {
            Json::Object(entries) => {
                let Some(seg) = path.get(i) else { return Lookup::Null };
                match entries.iter().find(|(k, _)| binding::hash(k) == seg.hash) {
                    Some((_, v)) => node = v,
                    None => return Lookup::Null,
                }
                i += 1;
            }
            Json::Array(items) => {
                if path.get(i).is_some_and(|s| s.hash == *LENGTH) {
                    return Lookup::Found(Value::UInt(items.len() as u32));
                }
                let idx = index_value(&path[i - 1].index).wrapping_sub(1) as usize;
                match items.get(idx) {
                    Some(item) => node = item,
                    None => return Lookup::OutOfRange,
                }
            }
            Json::Str(s) => return Lookup::Found(Value::Str(s.clone())),
            Json::Int(v) => return Lookup::Found(Value::Int64(*v)),
            Json::Double(v) => return Lookup::Found(Value::Double(*v)),
            Json::Bool(v) => return Lookup::Found(Value::Bool(*v)),
            Json::Null => return Lookup::Null,
        }
    }
}

/// `Signals`, the UI signal generator (`UISignalGeneratorManager`, object of 0x70 bytes; constructor and type
/// registration `FUN_0079c500`, per-frame update `FUN_007903c0` from the UI manager's update). Members (field
/// offset or getter; every getter takes the segment's index − 1, `FUN_00a55dd0`):
///
/// | member | type | value |
/// |---|---|---|
/// | `Zero` / `One` (+0x30 / +0x34) | float | 0 / 1 |
/// | `True` / `False` (+0x38 / +0x39) | bool | |
/// | `SlowRotateCW` (+0x3c) | float | fmod(t, 1) × 360 |
/// | `SlowRotateCCW` (+0x40) | float | 360 − `SlowRotateCW` |
/// | `Bounce` (+0x44) | float | \|sin(fmod(t, 1) × 2π)\| (x87) |
/// | `3SecondSquareWave` (+0x48) | float | trunc(10 t) mod 30 > 15 |
/// | `10SecondSquareWave` (+0x4c) | float | trunc(t) mod 10 > 5 |
/// | `SquareWave` (+0x50) | float | trunc(10 t) mod 10 > 5 |
/// | `SawWave` (+0x54) | float | (f32) t − trunc(t) |
/// | `SineWavePos` (+0x58) | float | (sinf(saw × 2π) + 1) × 0.5 |
/// | `GetSawWave[n]` (`FUN_007a7100`, n ≤ 100000) | float | (trunc(1000 t) mod n) / n |
/// | `GetSawWaveInv[n]` (`FUN_007a7190`) | float | 1 − that |
/// | `GetSineWave[n]` (0x7a71f0 → `FUN_00790590`) | float | 0.5 + 0.5 sin(2π t / ((n − 1) · 0.001)), SSE polynomial |
/// | `GetMetronome[n]` (0x7a7150) | float | 1 when counter mod (n − 1) = 0 |
/// | `GetValue[n]` (0x7a7210) | float | n |
/// | `GetIntValue[n]` (0x7a7230, any n ≥ 1) | i32 | n |
/// | `GetRandomNumber[n]` (`FUN_007a7240`) | i32 | high word of the LCG state mod n, then a step |
///
/// t (+0x60) is the UI clock's tick count (i64 at +8 of `*0x010edacc` → +4 → +0x18) × 1/3e6 (`.rdata`
/// 0x00faecb0) in seconds; the counter (+0x68) is the u32 at +0 of the same clock. The random state (+0x20) is a
/// 64-bit LCG (× 0x5851f42d4c957f2d + 1) seeded with 0x31613979fb637ec0 and stepped 8 times by the constructor.
/// `GetSineWave[1]` and `GetMetronome[1]` divide by zero in the original (unused by the data); here they give NaN
/// and 0.
#[derive(Debug)]
pub struct Signals {
    /// +0x60, seconds.
    pub time: f64,
    /// +0x68.
    pub counter: u32,
    fields: [f32; 8],
    rng: AtomicU64,
}

const LCG_MUL: u64 = 0x5851_f42d_4c95_7f2d;

impl Default for Signals {
    fn default() -> Self {
        let mut s = 0x3161_3979_fb63_7ec0u64;
        for _ in 0..8 {
            s = s.wrapping_mul(LCG_MUL).wrapping_add(1);
        }
        Signals { time: 0.0, counter: 0, fields: [0.0; 8], rng: AtomicU64::new(s) }
    }
}

/// Field names from +0x3c on, in offset order.
const SIGNAL_FIELDS: [&str; 8] = [
    "SlowRotateCW",
    "SlowRotateCCW",
    "Bounce",
    "3SecondSquareWave",
    "10SecondSquareWave",
    "SquareWave",
    "SawWave",
    "SineWavePos",
];

impl Signals {
    /// One frame of `FUN_007903c0`: `ticks` = the UI clock's i64 (3,000,000 per second), `counter` = its u32.
    pub fn update(&mut self, ticks: i64, counter: u32) {
        let t = ticks as f64 * f64::from_bits(0x3e96_5e9f_80f2_9212);
        self.time = t;
        self.counter = counter;
        let two_pi = f32::from_bits(0x40c9_0fdb);
        // fmod(t, 1) on the x87 stack (kept extended for the sine), stored as f32 for the rotations.
        let m = t % 1.0;
        let cw = m as f32 * 360.0;
        let ten = (t * 10.0) as i32;
        let whole = t as i32;
        let saw = t as f32 - whole as f32;
        self.fields = [
            cw,
            360.0 - cw,
            (m * two_pi as f64).sin().abs() as f32,
            if ten % 30 > 15 { 1.0 } else { 0.0 },
            if whole % 10 > 5 { 1.0 } else { 0.0 },
            if ten % 10 > 5 { 1.0 } else { 0.0 },
            saw,
            ((saw * two_pi).sin() + 1.0) * 0.5,
        ];
    }

    /// The members after the root (one segment: a field or an indexed getter).
    pub fn lookup(&self, members: &[Segment]) -> Lookup {
        let [m] = members else { return Lookup::Null };
        let n = match &m.index {
            Index::Pos(n) => *n as u32,
            Index::Key { hash, .. } => *hash,
        };
        // Index 0, or past the member's count (1 for fields, 100000 for most getters): out of range.
        let count = match m.name.as_str() {
            "GetIntValue" => u32::MAX,
            n if n.starts_with("Get") => 100_000,
            _ => 1,
        };
        if n == 0 || n > count {
            return Lookup::OutOfRange;
        }
        let i = n - 1;
        let f = |v: f32| Lookup::Found(Value::Float(v));
        match m.name.as_str() {
            "Zero" => f(0.0),
            "One" => f(1.0),
            "True" => Lookup::Found(Value::Bool(true)),
            "False" => Lookup::Found(Value::Bool(false)),
            "GetSawWave" => f(self.saw(n)),
            "GetSawWaveInv" => f(1.0 - self.saw(n)),
            "GetSineWave" => f(self.sine(i)),
            "GetMetronome" => f(if i != 0 && (self.counter as i32) % (i as i32) == 0 { 1.0 } else { 0.0 }),
            "GetValue" => f(n as i32 as f32),
            "GetIntValue" => Lookup::Found(Value::Int(n as i32)),
            "GetRandomNumber" => {
                let s = self.rng.load(Ordering::Relaxed);
                self.rng.store(s.wrapping_mul(LCG_MUL).wrapping_add(1), Ordering::Relaxed);
                Lookup::Found(Value::Int(((s >> 32) as u32 % n) as i32))
            }
            name => match SIGNAL_FIELDS.iter().position(|&f| f == name) {
                Some(k) => f(self.fields[k]),
                None => Lookup::Null,
            },
        }
    }

    /// `FUN_007a7100`: trunc(t × 1000) mod n, over n, in f32.
    fn saw(&self, n: u32) -> f32 {
        let ms = (self.time * 1000.0) as i64;
        (ms % n as i64) as f32 / n as i32 as f32
    }

    /// `FUN_00790590`(index − 1): the angle in f32, then the SSE sine (range reduction by −1/2π and the
    /// 1.5·2²³ rounding trick, an odd polynomial of degree 19).
    fn sine(&self, period_ms: u32) -> f32 {
        let x = (f32::from_bits(0x40c9_0fdb) / (period_ms as f32 * f32::from_bits(0x3a83_126f))) * self.time as f32;
        let a = (x - f32::from_bits(0x3fc9_0fdb)).abs() * f32::from_bits(0xbe22_f983);
        let magic = f32::from_bits(0x4b40_0000);
        let r = (((a - magic) + magic) - a) - 0.5;
        let u = r.abs() - 0.25;
        let u2 = u * u;
        let u5 = u2 * u2 * u;
        let u7 = u2 * u5;
        let u9 = u2 * u7;
        let u11 = u2 * u9;
        let u13 = u2 * u11;
        let u15 = u2 * u13;
        let u17 = u2 * u15;
        let c = |b: u32| f32::from_bits(b);
        let p = c(0xc225_5de7) * u2 * u
            + c(0x40c9_0fdb) * u
            + c(0x42a3_35e3) * u5
            + c(0xc299_6966) * u7
            + c(0x4228_3c1a) * u9
            + c(0xc171_83a8) * u11
            + c(0x4074_7a1a) * u13
            + c(0xbf37_d6dd) * u15
            + c(0x3dd5_7619) * u17
            + u2 * u17 * c(0xbc45_2021);
        (0.5 + p as f64 * 0.5) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_json_paths() {
        let mut vm = ViewModels::default();
        let j = Json::parse(r#"{"CanShow": true, "N": 3, "D": 0.5, "S": "x", "L": [{"A": 7}, {"A": 9}], "Z": null}"#)
            .unwrap();
        vm.publish("SafeForDialogs", j);
        assert_eq!(vm.resolve_bool("SafeForDialogs.CanShow"), Some(true));
        assert_eq!(vm.resolve_bool("SafeForDialogs.CanShow(NOT)"), Some(false));
        assert_eq!(vm.resolve_int("SafeForDialogs.N"), Some(3));
        assert_eq!(vm.resolve_double("SafeForDialogs.D"), Some(0.5));
        assert_eq!(vm.resolve_string("SafeForDialogs.S"), Some("x".into()));
        assert_eq!(vm.resolve_int("SafeForDialogs.L._length"), Some(2));
        assert_eq!(vm.resolve_int("SafeForDialogs.L[2].A"), Some(9));
        // No index: item 1; index 0 / past the end: out of range.
        assert_eq!(vm.resolve_int("SafeForDialogs.L.A"), Some(7));
        assert_eq!(vm.resolve_string("SafeForDialogs.L[3].A"), None);
        assert_eq!(vm.resolve_bool("SafeForDialogs.Z(IS_EMPTY)"), Some(true));
        assert_eq!(vm.resolve_bool("SafeForDialogs.Missing"), Some(false));
        assert_eq!(vm.resolve_hsm_string("SafeForDialogs.CanShow"), Some("TRUE".into()));
        vm.retract("SafeForDialogs");
        assert_eq!(vm.resolve_bool("SafeForDialogs.CanShow"), Some(false));
        assert_eq!(vm.resolve_bool("SafeForDialogs.CanShow(IS_EMPTY)"), Some(true));
    }

    #[test]
    fn signals() {
        let mut vm = ViewModels::default();
        assert_eq!(vm.resolve_int("Signals.GetIntValue[12]"), Some(12));
        assert_eq!(vm.resolve_int("Signals.GetIntValue"), Some(1));
        assert_eq!(vm.resolve_bool("Signals.True"), Some(true));
        assert_eq!(vm.resolve_bool("Signals.One(EQUALS:1)"), Some(true));
        assert_eq!(vm.resolve_double("Signals.Zero"), Some(0.0));
        // 2.75 s: saw over 11 s = 2750 / 11000 = 0.25.
        vm.signals.update(2_750_000 * 3, 0);
        assert!((vm.resolve_double("Signals.GetSawWave[11000]").unwrap() - 0.25).abs() < 1e-3);
        assert_eq!(vm.resolve_bool("Signals.GetSawWave[11000](LTEQ:0.5)"), Some(true));
        assert!((vm.resolve_double("Signals.SawWave").unwrap() - 0.75).abs() < 1e-6);
        assert!((vm.resolve_double("Signals.SlowRotateCW").unwrap() - 270.0).abs() < 1e-3);
        // A quarter of a 1.001 s period after t = 0.25025 s: the sine's top.
        vm.signals.update((0.250_25 * 3e6) as i64, 0);
        assert!((vm.resolve_double("Signals.GetSineWave[1002]").unwrap() - 1.0).abs() < 1e-4);
        let r: Vec<i64> = (0..4).map(|_| vm.resolve_int("Signals.GetRandomNumber[4]").unwrap()).collect();
        assert!(r.iter().all(|v| (0..4).contains(v)));
        assert_eq!(vm.resolve_string("Signals.GetValue[0]"), None);
    }

    #[test]
    fn registered_roots_first() {
        let mut vm = ViewModels::default();
        vm.register_root("Mode", |m: &[Segment]| match m {
            [s] if s.name == "ShowLoadingScreen" => Lookup::Found(Value::Bool(true)),
            [s] if s.name == "Racers" => Lookup::Found(Value::List(3)),
            _ => Lookup::Null,
        });
        assert_eq!(vm.resolve_bool("Mode.ShowLoadingScreen"), Some(true));
        assert_eq!(vm.resolve_int("Mode.Racers._length"), Some(3));
        assert_eq!(vm.resolve_bool("Mode.Other"), Some(false));
        assert_eq!(vm.root_names(), ["Mode", "Signals"]);
    }
}
