//! The HSM interpreter: runs `HSMData` state machines (`mw_formats::hsm`) like the original's
//! `Game::GameLogic` HSM manager. Ported from the dump (docs/decomp.md "The HSM interpreter"); each rule below
//! names its function.
//!
//! **Machines** (`FUN_00775ed0` start, `FUN_00776010` per-frame update, `FUN_00775b30` free, `FUN_00775010`
//! exit request): up to 12 top-level machines run at once, each started from a top-level state by its instance
//! id (the boot: 20527 → "Top State" 21597, read live from G+0x2f710), entered through the state's first entry
//! point, with a serial number as its handle. Every frame each machine's root is ticked once, in slot order; a
//! root that has finished is freed and its (serial, exit point) goes into a ring of the last 16 results.
//!
//! **Behaviour instances** (`FUN_00775cd0` init, base vtable of 9 slots): a state runs as an instance of its
//! behaviour type, created by the registry from the state's behaviour id (`FUN_00775a60`); an unregistered id
//! falls back to the state's fallback behaviour (RUN_CHILDREN → `HSB_RUNCHILD`, EXIT / ASSERT_THEN_EXIT →
//! `HSB_PLCHLDR`; a top-level state always falls back to `HSB_PLCHLDR`). Its exit point starts as `EXITINVALID`.
//!
//! **Tick** (`FUN_00776470`, vtable slot 8), four phases:
//! - *Entering*: once, fire the entry point it was entered through (`FUN_007760b0`: CHILD starts the spawn list,
//!   EXIT requests its own exit), publish its data definition when that has a path, then `on_enter` (slot 0).
//!   Every tick `enter_update` (slot 1) until it returns true; then `on_running` (slot 2) and *Running* in the
//!   same tick. Children are not ticked while entering.
//! - *Running*: unless an exit was requested, `update_children` (`FUN_00776210`); a nonzero result requests the
//!   exit of the instance and its whole subtree (`FUN_00778f10`). Then *Exiting* in the same tick.
//! - *Exiting*: once `on_exit_begin` (slot 5); every tick the children are ticked and the finished ones removed
//!   (`FUN_007757f0`); when none is left, `exit_update` (slot 4) until true, then `on_exit` (slot 6), the data
//!   definition retracted, *Done*.
//! - `update_children` (`FUN_00776210`): the behaviour's `update` (slot 3) first; true → done (the children are
//!   not ticked in that call). Otherwise passes over the children (in start order): the first pass ticks all,
//!   later passes only those never ticked before (started by a transition in this call); each finished child is
//!   removed and its exit point looked up in the parent's child exit data (`FUN_00776100`): CHILD starts the
//!   spawns, EXIT requests the parent's exit through the parent exit point. Passes repeat while a child finished
//!   and the parent is not exiting, so chains of instant states run within one frame.
//! - Starting a child (`FUN_00775da0`): appended to the parent's children, entered through the spawn's entry
//!   point; a parent already exiting passes its exit request on at once.
//!
//! Behaviour types are a [`Registry`] of factories keyed by behaviour id, so the 268 types of the original
//! (`ORIGINAL_BEHAVIOURS`) can be ported one by one; [`Registry::structural`] holds the ported ones. The engine
//! side (bindings, hooks, view models, clocks) is the [`Host`].

use std::collections::HashMap;
use std::sync::Arc;

use mw_formats::hsm::{Action, Fallback, HsmData, Parameter, State, behaviour_id, key, key_name};
use mw_formats::json::Json;

/// An entry / exit point or parameter id (a packed name, `mw_formats::hsm::key`).
pub type Key = u64;

/// Exit point every instance starts with (`DAT_0117bd78`, read from the running game); also what a missing
/// hook returns (`FUN_007761a0`).
pub const EXITINVALID: Key = 0x6845_64dc_1b60_c5c0;
/// `EXIT` (`DAT_0117a970`).
pub const EXIT: Key = 0x6845_61a1_6800_0000;
/// Machines running at once (`FUN_00775ed0`: 12 slots of 0x30 bytes).
pub const MAX_MACHINES: usize = 12;
/// Results kept of finished machines (`FUN_00775b30`: a ring of 16).
pub const HISTORY: usize = 16;

/// The behaviour types the original registers in its game-logic registry (factories 0x776000–0x780000, by name;
/// `scratch/scripting/hsm_behaviours_exe.json` of the research agent). Ids are [`behaviour_id`] of the names.
pub const ORIGINAL_BEHAVIOURS: &[&str] = &[
    "ACCT_INVITE",
    "ACTIVATESPOT",
    "ACTI_WDGT_GR",
    "ALLOWSIMPAUS",
    "BDELAY",
    "BINDCONTROLL",
    "BINDINGTIMED",
    "BINGAME",
    "BPAUSED",
    "BPLAYIT",
    "BROWSER_IN",
    "BUNITTEST",
    "BWAIT4GFX",
    "CACHESCORE",
    "CALCULATEMAT",
    "CALLHSMHOOK",
    "CALLHSMHOOKS",
    "CARCOLCORRCT",
    "CARJACKERACT",
    "CHANGELICENS",
    "CHANGEVSYNCS",
    "CHECK4THANKS",
    "CHECKPTS",
    "CHECK_ENT",
    "CHKPNTSONLIN",
    "CLEARUIPOSTF",
    "CLRSELEVENT",
    "COLLECTABLES",
    "CONSMUNLOCKS",
    "COSTSTATESC",
    "CRASHESCAPE",
    "CREATEPLAYGR",
    "DBAUTOSELWD",
    "DBGAUTOBIND",
    "DBGSTARTFRID",
    "DEBUGSTRTEVT",
    "DEBUGUNLOCKS",
    "DEL_PR_SESS",
    "DISABLETIME",
    "DISABLEWORLD",
    "DISCONNECTFR",
    "DOBLACKLISTS",
    "DOBODYSHOP",
    "DOONEVTSTRT",
    "DOONLINEEVEN",
    "DOSESSIONBES",
    "DOSMSHNGRB",
    "DWNLDPROG",
    "ENABLEALERTS",
    "ENABLETIME",
    "ENABLETRIGS",
    "ENDGAME",
    "ENDSETTINGSS",
    "ENUM_ENTMNTS",
    "EVENTSTARTS",
    "EXITBINDTRUE",
    "EXITWHENGAME",
    "FREEZE_CAR",
    "GAMEERROR",
    "GAMEMODE",
    "GAMEMODEINTR",
    "GCVALIDATEGA",
    "GETSPEEDWALL",
    "HAS_INV_ACCT",
    "HIDECHEV",
    "HORN_PRESS",
    "HOSTEZGUIDE",
    "HOSTSETGAMEM",
    "HSB_B_NOT_0",
    "HSB_DEBSKIP",
    "HSB_DISCONGP",
    "HSB_EX_BI_CH",
    "HSB_HDLAYOUT",
    "HSB_ME_CLEAR",
    "HSB_ME_ITEM",
    "HSB_NET_CRE",
    "HSB_NET_JOI",
    "HSB_NET_LEA",
    "HSB_NET_QJO",
    "HSB_NOSCORES",
    "HSB_P1BIND",
    "HSB_PLAYMOV",
    "HSB_PLAYSEQ",
    "HSB_PLCHLDR",
    "HSB_RBLAYOUT",
    "HSB_RILAYOUT",
    "HSB_RLAYOUTM",
    "HSB_RNLAYOUT",
    "HSB_RUNCHILD",
    "HSB_RUNMENL",
    "HSB_RUNNING",
    "HSB_SEL_EX_B",
    "HSB_STRSCRPT",
    "HSB_TOG_LS",
    "HSB_UI_DEB",
    "HSB_WAIT4IN",
    "HSB_WAIT4LS",
    "HSB_WAIT4NL",
    "HSM.CHECKFOR",
    "HSM.DOSPEEDT",
    "HSM.ENABLEDE",
    "HSM.INPUTDET",
    "HSM.NEEDTOTE",
    "HSM.PROGRESS",
    "HSM.SELECTEA",
    "HSM.STARTPHO",
    "HSM.UPLOADON",
    "HSMCARSWAPLO",
    "HSMDAYTRIPPE",
    "HSMDODRIFTS",
    "HSMHOOKEXIT",
    "HSMPAUSEINTR",
    "HSMSHAKEDOWN",
    "HSM_B_TRUE",
    "HSM_KEYB_ENT",
    "HSM_PLAYING",
    "HSM_SERV_OP",
    "INVITEFAILED",
    "IN_ON_GAME",
    "ISAUTOLOBBY",
    "ISAUTOTESTIN",
    "ISGAMEADMIN",
    "ISGAMEMODERU",
    "ISINVITEPEND",
    "ISNEWPROFILE",
    "IsPRCtxtOpen",
    "JOINEDLEFT",
    "JOINPARTYUID",
    "JOINPLAYGROU",
    "KILLCAMSEQ",
    "LAUNCH",
    "LEAVEPLAYGRO",
    "LIMITEDEDSP",
    "LIST_INVITE",
    "LOADLUA",
    "LOADPROFILE",
    "LOADSPEECHEV",
    "LOADSTRSET",
    "LOADWEAPONLI",
    "LOADWORLD",
    "LOAD_ALL",
    "LOAD_FDBK_GR",
    "LOAD_FM",
    "LOBBY",
    "LOCALECHECK",
    "MARKEDMANLOG",
    "MINIMAPICONS",
    "MINIMAPPURSU",
    "MMPICONSONL",
    "MODEENDCHECK",
    "MUTEALLRUMBL",
    "MUTE_SOUND",
    "NEWLUASTATE",
    "ONLINECARSWA",
    "ONLINESCORIN",
    "ONLNSCRACT",
    "OVERRIDEPAGE",
    "PAUSEAI",
    "PAUSEAUTOLOG",
    "PAUSEMSGS",
    "PAUSESIM",
    "PHOTO_COMPL",
    "PHOTO_INPUT",
    "PHOTO_LISTEN",
    "PICKNEXTEVEN",
    "PICKNEXTFDE",
    "PLAYAUTOLOGS",
    "PLAYFEEDBACK",
    "PLAYSAUTOLGS",
    "PLAYSONGLIST",
    "PREFETCHSCRE",
    "PRELOADCOPS",
    "PREP_INVITE",
    "RANDOM_EXIT",
    "RCDONEVNTWNR",
    "REBOOT",
    "REBOOT_REQ",
    "RECONNECT AL",
    "REFRESHENT",
    "REQTRAFRSTRT",
    "RESETGAME",
    "RESETPRCNTXT",
    "RFRSHALMANIF",
    "ROLLOUTACHVM",
    "RUNAMBSEQ",
    "RUNCHALL",
    "RUNGAMELOGIC",
    "RUNGAMEMODE",
    "RUNHUDSUBNET",
    "RUNKILLCAM",
    "RUNLUASCRIPT",
    "RUNPRCONTEXT",
    "RUNSTOREENUM",
    "RUNSUBNET",
    "RUN_PG_RNNR2",
    "SELECT_PROF",
    "SEND_INVITE",
    "SETBLACKBACK",
    "SETENDREASON",
    "SETENDSTRING",
    "SETFAILLOGIN",
    "SETPENDINGON",
    "SETSIMPAUSES",
    "SETSPAWNPOS",
    "SETSUCCESSLO",
    "SETUPRESULTS",
    "SET_GAME_ERR",
    "SET_LAY_GRP",
    "SET_LAY_SET",
    "SHOULDSHOWCA",
    "SHOWBUSTINGS",
    "SHOWCARSELEC",
    "SHOWDIALOG",
    "SHOWMAINMAP",
    "SHOWMEETUPPO",
    "SHOWPENDINGO",
    "SHOWVITASYST",
    "SHOW_DIALOG",
    "SHOW_UNL_SEQ",
    "SIGNINSCROLL",
    "SIGNIN_CNTRL",
    "SI_ADD_CNTRL",
    "SI_CNT_VIS",
    "SI_DISP_DATE",
    "SI_DISP_LIST",
    "SI_EDT_CHK",
    "SI_EDT_EMPT",
    "SI_FIRST_PRT",
    "SI_SHOW_ERR",
    "SI_STRT_FLOW",
    "SNDINTROTIME",
    "STARTCARSELE",
    "STOPMOVIE",
    "SUPPRESSPREF",
    "SWAPGAMEMODE",
    "SYNCGAMESTAT",
    "SYNCPROFILEW",
    "SYNCRESSTART",
    "SYNCTIME",
    "SYSERROR",
    "TAKEPHOTO",
    "TAKEPPIC",
    "TELEPORTTOME",
    "TGL_SI_B_VAL",
    "TPORTTOEVNT",
    "TRYTOPLAYSPE",
    "UNLOAD_FM",
    "UPLDCHALSTE",
    "UPLDCONCOURS",
    "UPLOADRESULT",
    "VALID_CNX_MP",
    "WAIT4ALLSYN",
    "WAIT4FEEDBK",
    "WAIT4TIME",
    "WAITCHILDREN",
    "WAITFORANYCO",
    "WAITFORCHASE",
    "WAITFORHOST",
    "WAITFORMODEC",
    "WAITFORVEHIC",
    "WAITSYSMENU",
    "WAIT_BUST",
    "WAYPOINTS",
    "WEAPONSOK",
    "WRITESESSION",
    "WRITETODATAS",
    "WRT_PR_SESSN",
    "WipePRSessn",
];

/// The code name of a behaviour id among [`ORIGINAL_BEHAVIOURS`].
pub fn original_code(id: u32) -> Option<&'static str> {
    ORIGINAL_BEHAVIOURS.iter().copied().find(|c| behaviour_id(c) == id)
}

/// The frame's time steps. The behaviours read two clocks of the timer manager (`*0x010edacc` + 4): `frame` =
/// the one at +0x18 (step +0x10 × rate +0x14: `BDELAY`, `WAIT4TIME`), `game` = the one at +0x38, `None` while its
/// running flag (+0x1c) is clear (`BINDINGTIMED`). Which engine clocks these are is open (decomp.md).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Clock {
    pub frame: f32,
    pub game: Option<f32>,
}

impl Clock {
    pub fn both(dt: f32) -> Self {
        Clock { frame: dt, game: Some(dt) }
    }
}

/// What the interpreter reports to the host (logging, tests).
#[derive(Debug, Clone, PartialEq)]
pub enum Trace {
    /// A machine started (`serial`) from top-level state `state`.
    Start { serial: u32, state: u32 },
    /// A state entered (`depth` 0 = a machine's root) through `entry`, run as `behaviour` (its code name, or
    /// the fallback used).
    Enter { depth: usize, state: u32, name: String, entry: Key, behaviour: String },
    /// A state finished through exit point `exit`.
    Exit { depth: usize, state: u32, name: String, exit: Key },
    /// A hook called (`CALLHSMHOOK` and friends) and what it returned.
    Hook { state: u32, name: String, result: Key },
    /// A finished machine (`serial`) and its exit point.
    Finish { serial: u32, exit: Key },
    /// Data the original would crash on (a child exit point with no handling, an entry point the state lacks,
    /// a spawn of an unknown state); the interpreter carries on.
    Missing { state: u32, what: String },
}

/// The engine side of the interpreter. Every method has a neutral default so a host implements what it has.
pub trait Host {
    /// A binding's value as a bool (`FUN_00a4f160`); `None` = not resolved.
    fn binding_bool(&mut self, _path: &str) -> Option<bool> {
        None
    }
    /// A binding's value as text (`FUN_00a4f2e0`, at most 12 characters in the original's buffer).
    fn binding_string(&mut self, _path: &str) -> Option<String> {
        None
    }
    /// A binding's value as an integer (`FUN_00a4f200`).
    fn binding_int(&mut self, _path: &str) -> Option<i64> {
        None
    }
    /// Calls an HSM hook (the registry at manager+0x360, `FUN_007761a0`, keyed by the name's hash) with the
    /// state's data-definition member as arguments; returns the exit point, `None` = no such hook
    /// (→ [`EXITINVALID`]).
    fn hook(&mut self, _name: &str, _args: Option<&Json>) -> Option<Key> {
        None
    }
    /// Whether the graphics are prepared (`FUN_00897960` on `*0x010edafc`; `BWAIT4GFX` waits for it).
    fn graphics_prepared(&mut self) -> bool {
        true
    }
    /// Whether the loading screen shows: the loading-screen manager's request mask (UI manager +0x2d204,
    /// decomp.md "Boot, loading screens") is nonzero and the byte at +0x2d200 is clear (its meaning is open);
    /// `HSB_WAIT4LS` / `HSB_WAIT4NL` wait on it.
    fn loading_screen_showing(&mut self) -> bool {
        false
    }
    /// A state with a data definition that has a path became active: publish the view model (`FUN_00a4ead0`).
    fn publish(&mut self, _path: &str, _data: &Json) {}
    /// ... and its state finished (`FUN_00a4de20`).
    fn retract(&mut self, _path: &str) {}
    fn trace(&mut self, _event: &Trace) {}
}

/// A host without an engine: no bindings resolve, no hooks exist.
pub struct NullHost;
impl Host for NullHost {}

/// One behaviour type: the original's 9-slot vtable (slot 8 is the shared tick, [`Hsm`]). Defaults are the
/// base class's (0x55cd90..0x55ce00): nothing, enter / exit at once, `update` keeps running.
pub trait Behaviour: Send + Sync {
    /// Slot 0, once when entered (after the entry point's spawns).
    fn on_enter(&mut self, _cx: &mut Cx) {}
    /// Slot 1, every tick while entering; true = entered.
    fn enter_update(&mut self, _cx: &mut Cx) -> bool {
        true
    }
    /// Slot 2, once when running.
    fn on_running(&mut self, _cx: &mut Cx) {}
    /// Slot 3, every tick while running, before the children; true = exit (with [`Cx::set_exit`]).
    fn update(&mut self, _cx: &mut Cx) -> bool {
        false
    }
    /// Slot 4, every tick while exiting once the children are gone; true = finished.
    fn exit_update(&mut self, _cx: &mut Cx) -> bool {
        true
    }
    /// Slot 5, once when the exit starts.
    fn on_exit_begin(&mut self, _cx: &mut Cx) {}
    /// Slot 6, once when finished.
    fn on_exit(&mut self, _cx: &mut Cx) {}
}

/// A behaviour factory: gets the state it will run.
pub type Factory = Arc<dyn Fn(&State) -> Box<dyn Behaviour> + Send + Sync>;

/// Behaviour types by behaviour id (`~crc32(code name)`).
#[derive(Clone, Default)]
pub struct Registry {
    factories: HashMap<u32, (String, Factory)>,
}

impl Registry {
    pub fn register(&mut self, code: &str, factory: Factory) {
        self.register_id(behaviour_id(code), code, factory);
    }

    /// Registers under a behaviour id directly (for ids whose code name is not known).
    pub fn register_id(&mut self, behaviour: u32, code: &str, factory: Factory) {
        self.factories.insert(behaviour, (code.to_owned(), factory));
    }

    /// A new instance of the behaviour registered for `state`'s behaviour id.
    pub fn create_for(&self, state: &State) -> Option<Box<dyn Behaviour>> {
        self.create(state.behaviour, state).map(|(_, b)| b)
    }

    pub fn register_fn<B: Behaviour + 'static>(&mut self, code: &str, f: fn(&State) -> B) {
        self.register(code, Arc::new(move |s| Box::new(f(s))));
    }

    pub fn contains(&self, behaviour: u32) -> bool {
        self.factories.contains_key(&behaviour)
    }

    pub fn code(&self, behaviour: u32) -> Option<&str> {
        self.factories.get(&behaviour).map(|(c, _)| c.as_str())
    }

    fn create(&self, behaviour: u32, state: &State) -> Option<(String, Box<dyn Behaviour>)> {
        self.factories.get(&behaviour).map(|(c, f)| (c.clone(), f(state)))
    }

    /// The ported behaviour types (see [`behaviours`]).
    pub fn structural() -> Self {
        let mut r = Registry::default();
        behaviours::register(&mut r);
        r
    }
}

/// An instance handle (index into the arena).
pub type InstId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Entering,
    Running,
    Exiting,
    Done,
}

struct Instance {
    /// Index into `HsmData::states`.
    state: usize,
    /// The entry point it was entered through (the spawn's, or a root's first).
    entry: Key,
    exit: Key,
    parent: Option<InstId>,
    children: Vec<InstId>,
    phase: Phase,
    /// +0x40: exit requested (set on the whole subtree).
    exit_requested: bool,
    /// +0x41: entered (the entry point fired, `on_enter` called).
    entered: bool,
    /// +0x42: `on_exit_begin` called.
    exit_begun: bool,
    /// +0x43: ticked at least once.
    ticked: bool,
    /// +0x38: the data definition's JSON (an own copy per instance, like the original's).
    data: Option<Json>,
    behaviour: Option<Box<dyn Behaviour>>,
    code: String,
}

struct Machine {
    serial: u32,
    root: InstId,
    instance: u32,
}

/// The interpreter: a manager of up to [`MAX_MACHINES`] machines over one `HSMData`.
pub struct Hsm {
    data: Arc<HsmData>,
    json: Vec<Option<Json>>,
    registry: Registry,
    arena: Vec<Option<Instance>>,
    free: Vec<InstId>,
    machines: [Option<Machine>; MAX_MACHINES],
    serial: u32,
    history: [(u32, Key); HISTORY],
    history_next: usize,
    /// Guard against a cycle of instant states (the original would hang): passes per `update_children` call.
    pub max_passes: usize,
}

impl Hsm {
    pub fn new(data: Arc<HsmData>, registry: Registry) -> Self {
        // The engine's JSON reader (`FUN_009f8fc0`): key order kept, int / double tags.
        let json = data.data_definitions.iter().map(|d| Json::parse(&d.json).ok()).collect();
        Hsm {
            data,
            json,
            registry,
            arena: Vec::new(),
            free: Vec::new(),
            machines: Default::default(),
            serial: 0,
            history: [(u32::MAX, EXITINVALID); HISTORY],
            history_next: 0,
            max_passes: 10_000,
        }
    }

    pub fn data(&self) -> &Arc<HsmData> {
        &self.data
    }

    pub fn registry_mut(&mut self) -> &mut Registry {
        &mut self.registry
    }

    /// Starts the top-level state of `instance` (`FUN_00775ed0`); its serial, or `None` without that
    /// instance or a free slot.
    pub fn start(&mut self, instance: u32, host: &mut dyn Host) -> Option<u32> {
        let top = *self.data.top_state(instance)?;
        let slot = self.machines.iter().position(Option::is_none)?;
        let state = self.data.state_index(top.definition)?;
        let entry = self.data.states[state].entry_points.first().map_or(0, |e| e.id);
        // A top-level state's unregistered behaviour falls back to HSB_PLCHLDR, whatever its fallback byte.
        let id = self.create(state, entry, None, Fallback::Exit);
        let serial = self.serial;
        self.serial = self.serial.wrapping_add(1);
        self.machines[slot] = Some(Machine { serial, root: id, instance });
        host.trace(&Trace::Start { serial, state: top.definition });
        Some(serial)
    }

    /// Ticks every machine once, in slot order, and frees the finished ones (`FUN_00776010`).
    pub fn update(&mut self, host: &mut dyn Host, clock: Clock) {
        for slot in 0..MAX_MACHINES {
            let Some(root) = self.machines[slot].as_ref().map(|m| m.root) else { continue };
            self.tick(root, host, clock);
            if self.inst(root).phase == Phase::Done {
                self.finish_machine(slot, host);
            }
        }
    }

    /// Ticks one machine (`FUN_00776050`).
    pub fn update_machine(&mut self, serial: u32, host: &mut dyn Host, clock: Clock) {
        let Some(slot) = self.slot(serial) else { return };
        let root = self.machines[slot].as_ref().unwrap().root;
        self.tick(root, host, clock);
        if self.inst(root).phase == Phase::Done {
            self.finish_machine(slot, host);
        }
    }

    /// Asks a machine to exit (`FUN_00775010`).
    pub fn request_exit(&mut self, serial: u32) {
        if let Some(slot) = self.slot(serial) {
            let root = self.machines[slot].as_ref().unwrap().root;
            self.request_subtree_exit(root);
        }
    }

    pub fn is_running(&self, serial: u32) -> bool {
        self.slot(serial).is_some()
    }

    /// A finished machine's exit point, while it is in the ring of the last 16 (`FUN_00774fe0`).
    pub fn result(&self, serial: u32) -> Option<Key> {
        self.history.iter().find(|(s, _)| *s == serial).map(|&(_, e)| e)
    }

    /// The running machines: (serial, top-state instance id).
    pub fn machines(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.machines.iter().flatten().map(|m| (m.serial, m.instance))
    }

    /// The active states of a machine, depth first: (depth, state id, phase).
    pub fn active(&self, serial: u32) -> Vec<(usize, u32, Phase)> {
        let mut out = Vec::new();
        if let Some(slot) = self.slot(serial) {
            let mut stack = vec![(0, self.machines[slot].as_ref().unwrap().root)];
            while let Some((d, id)) = stack.pop() {
                let i = self.inst(id);
                out.push((d, self.data.states[i.state].id, i.phase));
                stack.extend(i.children.iter().rev().map(|&c| (d + 1, c)));
            }
        }
        out
    }

    fn slot(&self, serial: u32) -> Option<usize> {
        self.machines.iter().position(|m| m.as_ref().is_some_and(|m| m.serial == serial))
    }

    fn finish_machine(&mut self, slot: usize, host: &mut dyn Host) {
        let m = self.machines[slot].take().unwrap();
        let exit = self.inst(m.root).exit;
        self.history[self.history_next] = (m.serial, exit);
        self.history_next = (self.history_next + 1) % HISTORY;
        self.release(m.root);
        host.trace(&Trace::Finish { serial: m.serial, exit });
    }

    fn inst(&self, id: InstId) -> &Instance {
        self.arena[id].as_ref().expect("live instance")
    }

    fn inst_mut(&mut self, id: InstId) -> &mut Instance {
        self.arena[id].as_mut().expect("live instance")
    }

    fn depth(&self, mut id: InstId) -> usize {
        let mut d = 0;
        while let Some(p) = self.inst(id).parent {
            d += 1;
            id = p;
        }
        d
    }

    /// Creates an instance of `state` (`FUN_00775a60` + `FUN_00775cd0`).
    fn create(&mut self, state: usize, entry: Key, parent: Option<InstId>, fallback: Fallback) -> InstId {
        let def = &self.data.states[state];
        let (code, behaviour) = match self.registry.create(def.behaviour, def) {
            Some(b) => b,
            None => {
                let code = match fallback {
                    Fallback::Exit | Fallback::AssertThenExit => "HSB_PLCHLDR",
                    _ => "HSB_RUNCHILD",
                };
                let name = original_code(def.behaviour).map_or(format!("{:08x}", def.behaviour), str::to_owned);
                let b: Box<dyn Behaviour> = if code == "HSB_PLCHLDR" {
                    Box::new(behaviours::Placeholder)
                } else {
                    Box::new(behaviours::RunChild)
                };
                (format!("{name}→{code}"), b)
            }
        };
        let data = def.data_definition.and_then(|i| self.json.get(i).cloned().flatten());
        let inst = Instance {
            state,
            entry,
            exit: EXITINVALID,
            parent,
            children: Vec::new(),
            phase: Phase::Entering,
            exit_requested: false,
            entered: false,
            exit_begun: false,
            ticked: false,
            data,
            behaviour: Some(behaviour),
            code,
        };
        match self.free.pop() {
            Some(id) => {
                self.arena[id] = Some(inst);
                id
            }
            None => {
                self.arena.push(Some(inst));
                self.arena.len() - 1
            }
        }
    }

    fn release(&mut self, id: InstId) {
        let children = std::mem::take(&mut self.inst_mut(id).children);
        for c in children {
            self.release(c);
        }
        self.arena[id] = None;
        self.free.push(id);
    }

    /// `FUN_00778f10`: flags the instance and its subtree to exit.
    fn request_subtree_exit(&mut self, id: InstId) {
        let i = self.inst_mut(id);
        i.exit_requested = true;
        let children = i.children.clone();
        for c in children {
            self.request_subtree_exit(c);
        }
    }

    /// Starts a child through a spawn (`FUN_00775da0`).
    fn spawn(&mut self, parent: InstId, entry: Key, state_id: u32, host: &mut dyn Host) {
        let Some(state) = self.data.state_index(state_id) else {
            let p = self.data.states[self.inst(parent).state].id;
            host.trace(&Trace::Missing { state: p, what: format!("spawn of unknown state {state_id}") });
            return;
        };
        let fallback = self.data.states[state].fallback;
        let id = self.create(state, entry, Some(parent), fallback);
        self.inst_mut(parent).children.push(id);
        if self.inst(parent).exit_requested {
            self.request_subtree_exit(id);
        }
    }

    /// `FUN_00775e70`: what an entry or exit point's action does to `id`.
    fn act(&mut self, id: InstId, action: Action, spawns: &[mw_formats::hsm::Spawn], host: &mut dyn Host) {
        match action {
            Action::Child => {
                for s in spawns {
                    self.spawn(id, s.entry_point, s.state, host);
                }
            }
            Action::Exit => self.request_subtree_exit(id),
            _ => {}
        }
    }

    /// Calls one vtable slot with the behaviour taken out of the arena.
    fn call<R>(
        &mut self,
        id: InstId,
        host: &mut dyn Host,
        clock: Clock,
        f: impl FnOnce(&mut dyn Behaviour, &mut Cx) -> R,
    ) -> R {
        let mut b = self.inst_mut(id).behaviour.take().expect("behaviour present");
        let data = self.data.clone();
        let r = {
            let mut cx = Cx { hsm: self, host, id, clock, data };
            f(b.as_mut(), &mut cx)
        };
        self.inst_mut(id).behaviour = Some(b);
        r
    }

    /// `FUN_00776470` (vtable slot 8).
    fn tick(&mut self, id: InstId, host: &mut dyn Host, clock: Clock) {
        let data = self.data.clone();
        let phase = self.inst(id).phase;
        if phase == Phase::Entering {
            if !self.inst(id).entered {
                // FUN_007760b0: the entry point it was entered through.
                let (state, entry) = (self.inst(id).state, self.inst(id).entry);
                let def = &data.states[state];
                let depth = self.depth(id);
                host.trace(&Trace::Enter {
                    depth,
                    state: def.id,
                    name: def.name.clone(),
                    entry,
                    behaviour: self.inst(id).code.clone(),
                });
                if !def.entry_points.is_empty() {
                    match def.entry_points.iter().find(|e| e.id == entry) {
                        Some(e) => self.act(id, e.action, &e.spawns, host),
                        None => host.trace(&Trace::Missing {
                            state: def.id,
                            what: format!("no entry point {}", key_name(entry)),
                        }),
                    }
                }
                if let Some(d) = data.data_definition(def).filter(|d| !d.path.is_empty())
                    && let Some(v) = &self.inst(id).data
                {
                    host.publish(&d.path, v);
                }
                self.call(id, host, clock, |b, cx| b.on_enter(cx));
                self.inst_mut(id).entered = true;
            }
            if !self.call(id, host, clock, |b, cx| b.enter_update(cx)) {
                self.inst_mut(id).ticked = true;
                return;
            }
            self.call(id, host, clock, |b, cx| b.on_running(cx));
            self.inst_mut(id).phase = Phase::Running;
        }
        if self.inst(id).phase == Phase::Running {
            if !self.inst(id).exit_requested {
                if !self.update_children(id, host, clock) {
                    self.inst_mut(id).ticked = true;
                    return;
                }
                self.request_subtree_exit(id);
            }
            self.inst_mut(id).phase = Phase::Exiting;
        }
        if self.inst(id).phase == Phase::Exiting {
            if !self.inst(id).exit_begun {
                self.call(id, host, clock, |b, cx| b.on_exit_begin(cx));
                self.inst_mut(id).exit_begun = true;
            }
            if self.tick_exiting_children(id, host, clock) && self.call(id, host, clock, |b, cx| b.exit_update(cx)) {
                self.call(id, host, clock, |b, cx| b.on_exit(cx));
                let def = &data.states[self.inst(id).state];
                if self.inst(id).entered
                    && let Some(d) = data.data_definition(def).filter(|d| !d.path.is_empty())
                {
                    host.retract(&d.path);
                }
                self.inst_mut(id).phase = Phase::Done;
                host.trace(&Trace::Exit {
                    depth: self.depth(id),
                    state: def.id,
                    name: def.name.clone(),
                    exit: self.inst(id).exit,
                });
            }
        }
        self.inst_mut(id).ticked = true;
    }

    /// `FUN_007757f0`: ticks the children of an exiting instance, removes the finished ones; true when none
    /// is left.
    fn tick_exiting_children(&mut self, id: InstId, host: &mut dyn Host, clock: Clock) -> bool {
        let mut all = true;
        for c in self.inst(id).children.clone() {
            self.tick(c, host, clock);
            if self.inst(c).phase == Phase::Done {
                self.inst_mut(id).children.retain(|&x| x != c);
                self.release(c);
            } else {
                all = false;
            }
        }
        all
    }

    /// `FUN_00776210`: true = the instance exits.
    fn update_children(&mut self, id: InstId, host: &mut dyn Host, clock: Clock) -> bool {
        if self.call(id, host, clock, |b, cx| b.update(cx)) {
            return true;
        }
        let mut exiting = false;
        let mut later = false;
        for _ in 0..self.max_passes {
            let mut finished = false;
            for c in self.inst(id).children.clone() {
                if !later || !self.inst(c).ticked {
                    self.tick(c, host, clock);
                    if self.inst(c).phase == Phase::Done {
                        finished = true;
                    }
                }
            }
            if !finished {
                return exiting;
            }
            for c in self.inst(id).children.clone() {
                if self.inst(c).phase != Phase::Done {
                    continue;
                }
                let (child_state, child_exit) = (self.data.states[self.inst(c).state].id, self.inst(c).exit);
                self.inst_mut(id).children.retain(|&x| x != c);
                self.release(c);
                if self.child_exited(id, child_state, child_exit, host) {
                    exiting = true;
                }
            }
            if exiting {
                return true;
            }
            later = true;
        }
        let state = self.data.states[self.inst(id).state].id;
        host.trace(&Trace::Missing { state, what: "pass limit reached (a cycle of instant states)".into() });
        exiting
    }

    /// `FUN_00776100`: a child finished through `exit`; true when the parent now exits.
    fn child_exited(&mut self, id: InstId, child: u32, exit: Key, host: &mut dyn Host) -> bool {
        let data = self.data.clone();
        let def = &data.states[self.inst(id).state];
        let Some(point) = def.child(child).and_then(|c| c.exits.iter().find(|e| e.child_exit == exit)) else {
            host.trace(&Trace::Missing {
                state: def.id,
                what: format!("child {child} exited through {} with no handling", key_name(exit)),
            });
            return false;
        };
        self.act(id, point.action, &point.spawns, host);
        if point.action == Action::Exit {
            self.inst_mut(id).exit = point.parent_exit;
            return true;
        }
        false
    }
}

/// What a behaviour sees of its instance, the interpreter and the host.
pub struct Cx<'a> {
    hsm: &'a mut Hsm,
    pub host: &'a mut dyn Host,
    id: InstId,
    pub clock: Clock,
    data: Arc<HsmData>,
}

impl Cx<'_> {
    pub fn state(&self) -> &State {
        &self.data.states[self.hsm.inst(self.id).state]
    }

    pub fn data(&self) -> &HsmData {
        &self.data
    }

    /// A parameter by name (`FUN_00774890`: the packed name, all 64 bits compared).
    pub fn param(&self, name: &str) -> Option<&Parameter> {
        self.state().parameter_by_id(key(name))
    }

    /// A parameter by its exact id (some ids carry characters after a NUL, `behaviours::BINDING_PATH`).
    pub fn param_id(&self, id: Key) -> Option<&Parameter> {
        self.state().parameter_by_id(id)
    }

    /// A parameter's first value, "" when the state lacks it.
    pub fn param_text(&self, name: &str) -> String {
        self.param(name).map_or(String::new(), |p| p.text().to_owned())
    }

    pub fn param_id_text(&self, id: Key) -> String {
        self.param_id(id).map_or(String::new(), |p| p.text().to_owned())
    }

    /// A float parameter (`atof`, 0 when missing or not a number).
    pub fn param_f32(&self, name: &str) -> f32 {
        atof(&self.param_text(name)) as f32
    }

    /// The entry point the state was entered through.
    pub fn entry_point(&self) -> Key {
        self.hsm.inst(self.id).entry
    }

    pub fn exit(&self) -> Key {
        self.hsm.inst(self.id).exit
    }

    pub fn set_exit(&mut self, exit: Key) {
        self.hsm.inst_mut(self.id).exit = exit;
    }

    pub fn has_children(&self) -> bool {
        !self.hsm.inst(self.id).children.is_empty()
    }

    /// A member of the state's data definition (`FUN_009f77a0` on the parsed JSON root).
    pub fn data_member(&self, name: &str) -> Option<Json> {
        self.hsm.inst(self.id).data.as_ref()?.get(name).cloned()
    }

    /// Calls a hook with a data-definition member as its arguments (`FUN_00776440` / `FUN_007761a0`).
    pub fn call_hook(&mut self, name: &str, args_member: &str) -> Key {
        let args = self.data_member(args_member);
        let result = self.host.hook(name, args.as_ref()).unwrap_or(EXITINVALID);
        let state = self.state().id;
        self.host.trace(&Trace::Hook { state, name: name.to_owned(), result });
        result
    }

    /// Whether the parent handles this exit point of the state (`FUN_00774790`: an exit point with an action
    /// other than NOTHING in the parent's child exit data).
    pub fn parent_handles(&self, exit: Key) -> bool {
        let Some(parent) = self.hsm.inst(self.id).parent else { return false };
        let pdef = &self.data.states[self.hsm.inst(parent).state];
        pdef.child(self.state().id)
            .is_some_and(|c| c.exits.iter().any(|e| e.child_exit == exit && e.action != Action::Nothing))
    }

    /// Starts another machine (`RUNSUBNET`).
    pub fn start_machine(&mut self, instance: u32) -> Option<u32> {
        self.hsm.start(instance, self.host)
    }

    pub fn machine_running(&self, serial: u32) -> bool {
        self.hsm.is_running(serial)
    }

    pub fn machine_exit_requested(&self, serial: u32) -> bool {
        self.hsm.slot(serial).is_some_and(|s| {
            let root = self.hsm.machines[s].as_ref().unwrap().root;
            self.hsm.inst(root).exit_requested
        })
    }

    pub fn machine_result(&self, serial: u32) -> Option<Key> {
        self.hsm.result(serial)
    }

    pub fn request_machine_exit(&mut self, serial: u32) {
        self.hsm.request_exit(serial)
    }
}

/// C `atof` on the leading number of a string (the parameters' float values).
pub fn atof(s: &str) -> f64 {
    let s = s.trim_start();
    let mut end = 0;
    let b = s.as_bytes();
    let mut seen_digit = false;
    let mut seen_dot = false;
    let mut seen_e = false;
    while end < b.len() {
        let c = b[end];
        match c {
            b'0'..=b'9' => seen_digit = true,
            b'+' | b'-' if end == 0 || matches!(b[end - 1], b'e' | b'E') => {}
            b'.' if !seen_dot && !seen_e => seen_dot = true,
            b'e' | b'E' if seen_digit && !seen_e => seen_e = true,
            _ => break,
        }
        end += 1;
    }
    let mut t = &s[..end];
    while !t.is_empty() && t.parse::<f64>().is_err() {
        t = &t[..t.len() - 1];
    }
    t.parse().unwrap_or(0.0)
}

pub mod behaviours;

#[cfg(test)]
mod tests;
