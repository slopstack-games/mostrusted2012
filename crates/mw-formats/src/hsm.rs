//! `HSMData` (0x501, `Game::GameLogic::HSMData`): the game logic's hierarchical state machines (the boot /
//! front-end / game-mode flow in `GAMELOGIC/GAMEMODES.BNDL`, the cars' active-aero machines). Official structure
//! and field names from burnout.wiki "HSM Data/NFS Most Wanted" (a PS3 debug build); every field checked against
//! the data and the code that reads it (docs/formats.md "HSMData", the interpreter in docs/decomp.md "The HSM
//! interpreter"; ported in `mw_logic::hsm`). Offsets are into the resource's chunk 0; `ptr` = u32 offset.
//!
//! Header (0x24): u32 version (3), then (ptr, u32 count) pairs: +0x04 top-level states, +0x0C states, +0x14 data
//! definitions, +0x1C behaviour interfaces.
//!
//! | struct | size | layout |
//! |--------|------|--------|
//! | `HSMTopStateDefinition` | 12 | u32 definition id, u32 behaviour id, u32 instance id (what the code starts: GAMEMODES' boot = 20527 → "Top State" 21597) |
//! | `HSMStateDefinition` | 0x20 | +0 u32 behaviour id (`~crc32(code name)`), +4 u32 definition id, +8 ptr entry points, +0xC ptr child exit data, +0x10 ptr parameters, +0x14 ptr data definition (or 0), +0x18 ptr debug name, +0x1C u8 parameter count, +0x1D u8 fallback behaviour, +0x1E u8 entry point count, +0x1F u8 child count |
//! | `HSMStateEntryPoint` | 0x10 | u64 entry point id, u8 action, u8 spawn count, 2 pad, ptr spawn data |
//! | `HSMStateSpawnData` | 0x10 | u64 entry point id (the child is entered through it), u32 child definition id, ptr debug name |
//! | `HSMChildExitData` | 0x10 | u32 child definition id, ptr exit points, i32 exit point count, ptr debug name |
//! | `HSMChildExitPointData` | 0x18 | u64 parent exit point id, u64 child exit point id, u8 action, u8 spawn count, 2 pad, ptr spawn data |
//! | `HSMParameter` | 0x10 | u64 parameter id, ptr values (`char*[count]`, or `HSMStructureInstance[count]` for STRUCTURE), i16 type, i16 count |
//! | `HSMStructureInstance` | 8 | ptr parameters, i16 count, 2 pad |
//! | `HSMDataDefinition` | 0x14 | u32 id, ptr path (the view-model name it is published under, or 0), ptr data (JSON text), u32 size, ptr debug name |
//! | `HSMBehaviourInterface` | 0x20 | u64 behaviour id (packed code name), ptr name, ptr entry point ids (u64[]), ptr exit point ids (u64[]), ptr parameter defaults (`HSMParameter`[]), u32 parameter count, i16 entry count, i16 exit count |
//!
//! Actions: NOTHING 0, CHILD 1 (start the spawn list), EXIT 2 (exit the parent: through the parent exit point
//! id). Fallback behaviour (used when the behaviour id is not registered, `FUN_00775a60`): RUN_CHILDREN 0
//! (`HSB_RUNCHILD`), EXIT 1 and ASSERT_THEN_EXIT 2 (`HSB_PLCHLDR`, which exits at once through `EXIT`).
//! Parameter types: STRING 0, INT 1, FLOAT 2, ID 3, BOOL 4, ASSET_REFERENCE 5, STRUCTURE 6, INVALID 7; every
//! value is stored as text.
//!
//! Ids of entry / exit points and parameters are CgsIDs: names packed base 40, most significant first, padded to
//! 12 characters (`FUN_009ed220`: '-' 1, '/' 2, '0'–'9' 3–12, letters 13–38, '_' 39): `EXIT` = 0x684561a168000000.

use std::collections::HashMap;

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

/// The packed form of a name (inverse of [`key_name`]; `FUN_009ed220`: characters outside the alphabet count as
/// padding, names longer than 12 characters are cut).
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

/// A state's behaviour id from the behaviour's code name (`HSB_RUNCHILD` → 0x24c422b2): the JAMCRC (`~crc32`).
pub fn behaviour_id(code: &str) -> u32 {
    !mw_bundle::crc32(code.as_bytes())
}

/// `EHSMAction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Nothing,
    /// Start the spawn list as children of the state that owns the entry / exit point.
    Child,
    /// Exit the owning state (through the exit point's parent exit point id).
    Exit,
    Other(u8),
}

impl Action {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Action::Nothing,
            1 => Action::Child,
            2 => Action::Exit,
            v => Action::Other(v),
        }
    }
}

/// `EFallbackBehaviour`: what runs when a state's behaviour id is not registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    RunChildren,
    Exit,
    AssertThenExit,
    Other(u8),
}

impl Fallback {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Fallback::RunChildren,
            1 => Fallback::Exit,
            2 => Fallback::AssertThenExit,
            v => Fallback::Other(v),
        }
    }
}

/// `EHSMParamTypes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    String,
    Int,
    Float,
    Id,
    Bool,
    AssetReference,
    Structure,
    Invalid,
    Other(i16),
}

impl ParamType {
    fn from_i16(v: i16) -> Self {
        match v {
            0 => ParamType::String,
            1 => ParamType::Int,
            2 => ParamType::Float,
            3 => ParamType::Id,
            4 => ParamType::Bool,
            5 => ParamType::AssetReference,
            6 => ParamType::Structure,
            7 => ParamType::Invalid,
            v => ParamType::Other(v),
        }
    }
}

/// `HSMParameter`: every value is text (`"15"`, `"True"`, `"491032"`); a STRUCTURE holds parameter lists instead.
#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    pub id: u64,
    pub kind: ParamType,
    pub values: Vec<String>,
    /// STRUCTURE: one parameter list per instance.
    pub structures: Vec<Vec<Parameter>>,
}

impl Parameter {
    /// The first value (what the behaviours read: `value_ptr[0]`), "" when there is none.
    pub fn text(&self) -> &str {
        self.values.first().map_or("", String::as_str)
    }
}

/// `HSMStateSpawnData`: start `state` through its entry point `entry_point`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spawn {
    pub entry_point: u64,
    pub state: u32,
}

/// `HSMStateEntryPoint`: what entering the state through `id` does (most states: `ENTER` → start children).
#[derive(Debug, Clone, PartialEq)]
pub struct EntryPoint {
    pub id: u64,
    pub action: Action,
    pub spawns: Vec<Spawn>,
}

/// `HSMChildExitPointData`: when the child exits through `child_exit`, do `action` (and for EXIT, exit the parent
/// through `parent_exit`).
#[derive(Debug, Clone, PartialEq)]
pub struct ChildExitPoint {
    pub parent_exit: u64,
    pub child_exit: u64,
    pub action: Action,
    pub spawns: Vec<Spawn>,
}

/// `HSMChildExitData`: one child the state may run, with its exit points' handling.
#[derive(Debug, Clone, PartialEq)]
pub struct ChildExits {
    pub state: u32,
    pub exits: Vec<ChildExitPoint>,
}

/// `HSMStateDefinition` (a node).
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    /// `~crc32(code name)` ([`behaviour_id`]).
    pub behaviour: u32,
    pub id: u32,
    pub name: String,
    pub fallback: Fallback,
    pub entry_points: Vec<EntryPoint>,
    pub children: Vec<ChildExits>,
    pub parameters: Vec<Parameter>,
    /// Index into [`HsmData::data_definitions`].
    pub data_definition: Option<usize>,
}

impl State {
    /// The children started when entered through `ENTER`.
    pub fn initial(&self) -> impl Iterator<Item = u32> + '_ {
        self.entry_points.iter().filter(|e| e.id == key("ENTER")).flat_map(|e| e.spawns.iter().map(|s| s.state))
    }

    pub fn parameter_by_id(&self, id: u64) -> Option<&Parameter> {
        self.parameters.iter().find(|p| p.id == id)
    }

    pub fn parameter(&self, name: &str) -> Option<&str> {
        self.parameter_by_id(key(name)).map(Parameter::text)
    }

    pub fn child(&self, state: u32) -> Option<&ChildExits> {
        self.children.iter().find(|c| c.state == state)
    }
}

/// `HSMDataDefinition`: a JSON block. With a `path` it is a view model published under that name while a state
/// using it is active (`SafeForDialogs {"CanShow": true}`); without, it holds a hook's arguments
/// (`{"Params": {"NewGame": true}}`).
#[derive(Debug, Clone, PartialEq)]
pub struct DataDefinition {
    pub id: u32,
    pub path: String,
    pub name: String,
    pub json: String,
}

/// `HSMBehaviourInterface`: a behaviour type's entry / exit points and parameters with their defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct BehaviourInterface {
    /// Packed code name (`HSB_RUNCHILD`); a state's behaviour id is [`behaviour_id`] of [`key_name`] of it.
    pub id: u64,
    pub name: String,
    pub entry_points: Vec<u64>,
    pub exit_points: Vec<u64>,
    pub parameters: Vec<Parameter>,
}

/// `HSMTopStateDefinition`: a machine the code can start by instance id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopState {
    pub definition: u32,
    pub behaviour: u32,
    pub instance: u32,
}

#[derive(Debug, Clone, Default)]
pub struct HsmData {
    pub version: u32,
    pub top_states: Vec<TopState>,
    pub states: Vec<State>,
    pub data_definitions: Vec<DataDefinition>,
    pub behaviours: Vec<BehaviourInterface>,
    index: HashMap<u32, usize>,
}

fn cstr(r: &Reader, o: usize) -> String {
    if o == 0 {
        return String::new();
    }
    let mut out = Vec::new();
    let mut i = o;
    while let Ok(b) = r.u8(i) {
        if b == 0 || out.len() > 1 << 16 {
            break;
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn spawns(r: &Reader, ptr: usize, n: usize) -> Result<Vec<Spawn>> {
    (0..n).map(|k| Ok(Spawn { entry_point: r.u64(ptr + 16 * k)?, state: r.u32(ptr + 16 * k + 8)? })).collect()
}

fn parameters(r: &Reader, ptr: usize, n: usize, depth: usize) -> Result<Vec<Parameter>> {
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let p = ptr + 16 * k;
        let (id, values_ptr, kind, count) = (r.u64(p)?, r.u32(p + 8)? as usize, r.i16(p + 12)?, r.i16(p + 14)?);
        let kind = ParamType::from_i16(kind);
        let count = count.max(0) as usize;
        let mut param = Parameter { id, kind, values: Vec::new(), structures: Vec::new() };
        if values_ptr != 0 {
            if kind == ParamType::Structure {
                if depth < 8 {
                    for j in 0..count {
                        let s = values_ptr + 8 * j;
                        let (sp, sn) = (r.u32(s)? as usize, r.i16(s + 4)?.max(0) as usize);
                        param.structures.push(parameters(r, sp, sn, depth + 1)?);
                    }
                }
            } else {
                for j in 0..count {
                    param.values.push(cstr(r, r.u32(values_ptr + 4 * j)? as usize));
                }
            }
        }
        out.push(param);
    }
    Ok(out)
}

impl HsmData {
    pub fn parse(res: &Resource) -> Result<Self> {
        Self::parse_bytes(res.header())
    }

    pub fn parse_bytes(bytes: &[u8]) -> Result<Self> {
        let r = Reader(bytes);
        let mut out = HsmData { version: r.u32(0)?, ..Default::default() };
        let (tops, n_tops) = (r.u32(0x04)? as usize, r.u32(0x08)? as usize);
        for i in 0..n_tops.min(4096) {
            let [definition, behaviour, instance] = r.u32_array::<3>(tops + 12 * i)?;
            out.top_states.push(TopState { definition, behaviour, instance });
        }
        let (defs, n_defs) = (r.u32(0x14)? as usize, r.u32(0x18)? as usize);
        let mut def_index = HashMap::new();
        for i in 0..n_defs.min(1 << 16) {
            let o = defs + 0x14 * i;
            let [id, path, data, size, name] = r.u32_array::<5>(o)?;
            let json = if data == 0 { Vec::new() } else { r.bytes(data as usize, size as usize)?.to_vec() };
            def_index.insert(o as u32, out.data_definitions.len());
            out.data_definitions.push(DataDefinition {
                id,
                path: cstr(&r, path as usize),
                name: cstr(&r, name as usize),
                json: String::from_utf8_lossy(&json).trim_end_matches('\0').to_owned(),
            });
        }
        let (states, n_states) = (r.u32(0x0c)? as usize, r.u32(0x10)? as usize);
        for i in 0..n_states.min(1 << 16) {
            let o = states + 32 * i;
            let [behaviour, id, entries, children, params, data_def, name] = r.u32_array::<7>(o)?;
            let (n_params, fallback, n_entries, n_children) =
                (r.u8(o + 0x1c)?, r.u8(o + 0x1d)?, r.u8(o + 0x1e)?, r.u8(o + 0x1f)?);
            let mut state = State {
                behaviour,
                id,
                name: cstr(&r, name as usize),
                fallback: Fallback::from_u8(fallback),
                entry_points: Vec::new(),
                children: Vec::new(),
                parameters: parameters(&r, params as usize, n_params as usize, 0)?,
                data_definition: if data_def == 0 { None } else { def_index.get(&data_def).copied() },
            };
            for k in 0..n_entries as usize {
                let e = entries as usize + 16 * k;
                state.entry_points.push(EntryPoint {
                    id: r.u64(e)?,
                    action: Action::from_u8(r.u8(e + 8)?),
                    spawns: spawns(&r, r.u32(e + 12)? as usize, r.u8(e + 9)? as usize)?,
                });
            }
            for k in 0..n_children as usize {
                let c = children as usize + 16 * k;
                let (child, eptr, en) = (r.u32(c)?, r.u32(c + 4)? as usize, r.u32(c + 8)? as usize);
                let mut exits = Vec::new();
                for j in 0..en.min(256) {
                    let t = eptr + 24 * j;
                    exits.push(ChildExitPoint {
                        parent_exit: r.u64(t)?,
                        child_exit: r.u64(t + 8)?,
                        action: Action::from_u8(r.u8(t + 16)?),
                        spawns: spawns(&r, r.u32(t + 20)? as usize, r.u8(t + 17)? as usize)?,
                    });
                }
                state.children.push(ChildExits { state: child, exits });
            }
            out.index.entry(id).or_insert(out.states.len());
            out.states.push(state);
        }
        let (types, n_types) = (r.u32(0x1c)? as usize, r.u32(0x20)? as usize);
        for i in 0..n_types.min(4096) {
            let o = types + 32 * i;
            let (entry_ptr, exit_ptr, param_ptr) =
                (r.u32(o + 0x0c)? as usize, r.u32(o + 0x10)? as usize, r.u32(o + 0x14)? as usize);
            let (n_params, n_entries, n_exits) =
                (r.u32(o + 0x18)? as usize, r.i16(o + 0x1c)?.max(0) as usize, r.i16(o + 0x1e)?.max(0) as usize);
            out.behaviours.push(BehaviourInterface {
                id: r.u64(o)?,
                name: cstr(&r, r.u32(o + 8)? as usize),
                entry_points: (0..n_entries)
                    .map(|k| r.u64(entry_ptr + 8 * k))
                    .collect::<std::result::Result<_, _>>()?,
                exit_points: (0..n_exits).map(|k| r.u64(exit_ptr + 8 * k)).collect::<std::result::Result<_, _>>()?,
                parameters: parameters(&r, param_ptr, n_params.min(256), 0)?,
            });
        }
        Ok(out)
    }

    /// Data built in code (tests, tools): indexes the states.
    pub fn from_parts(
        top_states: Vec<TopState>,
        states: Vec<State>,
        data_definitions: Vec<DataDefinition>,
        behaviours: Vec<BehaviourInterface>,
    ) -> Self {
        let mut index = HashMap::new();
        for (i, s) in states.iter().enumerate() {
            index.entry(s.id).or_insert(i);
        }
        HsmData { version: 3, top_states, states, data_definitions, behaviours, index }
    }

    /// A state by definition id (the first one with it, like the code's lookup).
    pub fn state(&self, id: u32) -> Option<&State> {
        self.index.get(&id).map(|&i| &self.states[i])
    }

    /// The index of a state in [`HsmData::states`].
    pub fn state_index(&self, id: u32) -> Option<usize> {
        self.index.get(&id).copied()
    }

    /// The top-level state started under `instance` (`FUN_00775ed0`).
    pub fn top_state(&self, instance: u32) -> Option<&TopState> {
        self.top_states.iter().find(|t| t.instance == instance)
    }

    /// The behaviour interface whose code name hashes to a state's behaviour id.
    pub fn behaviour(&self, behaviour: u32) -> Option<&BehaviourInterface> {
        self.behaviours.iter().find(|b| behaviour_id(&key_name(b.id)) == behaviour)
    }

    /// The code name of a behaviour id (`HSB_RUNCHILD`), when the interface table has it.
    pub fn behaviour_code(&self, behaviour: u32) -> Option<String> {
        self.behaviour(behaviour).map(|b| key_name(b.id))
    }

    pub fn data_definition(&self, state: &State) -> Option<&DataDefinition> {
        state.data_definition.and_then(|i| self.data_definitions.get(i))
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
        // The exit code a behaviour starts with (`DAT_0117bd78`, read from the running game).
        assert_eq!(key_name(0x6845_64dc_1b60_c5c0), "EXITINVALID");
    }

    #[test]
    fn behaviour_ids() {
        assert_eq!(behaviour_id("HSB_RUNCHILD"), 0x24c4_22b2);
        assert_eq!(behaviour_id("LOADWORLD"), 0xac59_d95a);
        assert_eq!(behaviour_id("HSB_TOG_LS"), 0xe6d3_7d97);
    }
}
