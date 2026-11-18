//! Smash-and-Grab scoring: `DOSMSHNGRB` ("HSM.DoSmashAndGrabScoring", factory 0x77e150, vtable 0xfda9c8, 0x1258 B
//! of instance state; decomp.md "Smash-and-Grab scoring"). Ported from the code: enter `FUN_0075b820`, enter update
//! `FUN_007612e0` (`FUN_00760b40` builds the item lists, `FUN_0075e9a0` / `FUN_0075bae0` place an item, `FUN_006e41e0`
//! hides the other billboards), update `FUN_00761450` (`FUN_00760dd0` items smashed by contact, `FUN_0075ba30`
//! billboards, `FUN_0074cf40` the award), exit update `FUN_0075ed20`, exit 0x73cf00; the binding root `SmashScoring`
//! (`FUN_007570f0`'s registrations and the getters 0x76a190, 0x76a1c0, 0x768980, 0x7689d0, 0x768a10).
//!
//! **What the runtime fills** ([`SmashGrabInputs`]): the event's trigger list (event +0x54, count +0xd2) with each
//! trigger's kind byte (record +0x10), the `SmashAndGrabConfig` items, the time limit (event +0xac), the medal
//! scores, the load / prop states, the clock flag, the local car's contacts with the items' props, and the last
//! billboard the collectables smashed in this mode (`DAT_010c5c08`, set by `FUN_006ed330` → `FUN_0073cef0` while
//! GMM+0x2e8f is set). The race clock is GMM+0x2d28 ([`GameModeManager::word_2d28`](super::game_modes)).
//!
//! **What the logic decides** ([`SmashGrabState`]): the item lists, the score (+0x1224), the end (time up or every
//! item taken: the place by medal, WIN or LOSE, `FUN_006b9d00`, GMM+0x2e78). **What the runtime carries out**: the
//! [`SmashGrabOut`] requests, [`Request::NitrousAward`] (10 per item) and [`Request::SpeedPoints`] (the score on
//! exit).

use super::LogicState;
use super::drifts::{Slot, finish, set_player_score};
use super::state::Request;
use crate::binding::{Lookup, Segment, Value};

/// A `SmashAndGrabConfig.GrabItem` (0x18 B): the map icon / prop key +0xc (`b6c42d2d`), `ScoringAction` +0x10,
/// `Score` +0x14.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrabItem {
    pub poi: u32,
    pub action: u32,
    pub score: u16,
}

/// `GameLogicConfig` +0x2b0 `SmashAndGrabConfig`: four items at +0xc / +0x24 / +0x3c / +0x54. The code uses the
/// first for the plain items (trigger kind 0x1f), the second for billboards, the third for the high-value items
/// (kind 0x21); the fourth is not read. Defaults: the install's `GLOBALCONFIG.BNDL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmashGrabConfig {
    pub items: [GrabItem; 4],
}

impl Default for SmashGrabConfig {
    fn default() -> Self {
        let g = |poi, action, score| GrabItem { poi, action, score };
        SmashGrabConfig {
            items: [
                g(2_210_071, 2_210_197, 50),
                g(2_210_109, 2_210_198, 250),
                g(2_210_110, 2_210_197, 150),
                g(2_210_111, 2_210_197, 1),
            ],
        }
    }
}

/// Trigger kinds (record +0x10) `FUN_00760b40` sorts the event's triggers by.
pub const KIND_ITEM: u8 = 0x1f;
pub const KIND_HIGH_VALUE: u8 = 0x21;
/// Billboards ('2', 'A', 'D'; 'J' and 'M' only with game pack 3, `FUN_00647680`(3)).
pub const BILLBOARD_KINDS: [u8; 3] = *b"2AD";
pub const PACK_BILLBOARD_KINDS: [u8; 2] = *b"JM";

/// What the runtime supplies (see the module doc).
#[derive(Debug, Clone)]
pub struct SmashGrabInputs {
    pub config: SmashGrabConfig,
    /// The event's triggers (`FUN_006e66a0` → +0x54, count +0xd2) and each one's kind (`FUN_0053c020` → +0x10).
    pub event_triggers: Vec<(u32, u8)>,
    /// `FUN_00647680`(3): game pack 3 is available (its 'J' / 'M' billboards count).
    pub pack3: bool,
    /// The event's time limit (event +0xac → the first float), seconds.
    pub time_limit: f32,
    /// `FUN_006f1900`(GMM, event, 0 / 1 / 2): gold, silver, bronze (read on enter; compared unsigned).
    pub medals: [i32; 3],
    /// The resource loader's state +0x11c4 is 2 or 3 (busy): the enter update waits.
    pub loading: bool,
    /// Every item's prop record has state 3 (+0): the enter update waits for it.
    pub props_ready: bool,
    /// The game clock's running flag (`FUN_0046ee10` +0x1c).
    pub clock_running: bool,
    /// The local vehicle has a simulation record (world +0x55c48 + vehicle × 4 > −1).
    pub in_sim: bool,
    /// The local car's contacts this frame (world +0x5df4c / +0x5df5c, entry +100) that are an item's prop: the
    /// item's trigger id, one per contact, in contact order.
    pub contacts: Vec<u32>,
    /// `DAT_010c5c08`: the billboard trigger last smashed while GMM+0x2e8f is set (−1 = none; the logic resets it).
    pub last_billboard: i32,
}

impl Default for SmashGrabInputs {
    fn default() -> Self {
        SmashGrabInputs {
            config: SmashGrabConfig::default(),
            event_triggers: Vec::new(),
            pack3: false,
            time_limit: 0.0,
            medals: [-1; 3],
            loading: false,
            props_ready: true,
            clock_running: true,
            in_sim: true,
            contacts: Vec::new(),
            last_billboard: -1,
        }
    }
}

/// The end of the update: WIN with a medal, LOSE without.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmashGrabEnd {
    Win,
    Lose,
}

/// The behaviour's state (the logic decides it).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SmashGrabState {
    /// Between enter and exit: the root `SmashScoring` is registered.
    pub active: bool,
    /// +0x1208: the enter update's phase (3 = entered); +0x120c the exit update's.
    pub phase: u32,
    pub exit_phase: u32,
    /// +0x1224 `Score`.
    pub score: u32,
    /// +0x1248 `ItemWorth`, +0x124c `BillboardWorth`, +0x1250 `HighValueWorth` (copies of +0x123c..+0x1244).
    pub item_worth: u32,
    pub billboard_worth: u32,
    pub high_value_worth: u32,
    /// +0x1210 / +0x1214 / +0x1218: gold, silver, bronze.
    pub medals: [u32; 3],
    /// +0x11f0: the time limit in clock ticks (3e6 per second); +0x1200: a tenth of it (`TimeToNextClock`).
    pub time_limit: i64,
    pub next_clock: i64,
    /// The sorted maps +0x50 (plain items, values +0x260), +0xf88 (high-value items, +0xfe0) and +0xe90
    /// (billboards, icons +0xee4): trigger ids.
    pub items: Vec<u32>,
    pub high_value: Vec<u32>,
    pub billboards: Vec<u32>,
    /// +0x1254 `LastItemBillboard`, +0x1255 `JustHitBillboard`.
    pub last_item_billboard: bool,
    pub just_hit_billboard: bool,
    /// The update's outcome (exit `WIN` / `LOSE`).
    pub end: Option<SmashGrabEnd>,
}

/// The smash-and-grab scoring: runtime inputs and the logic's state.
#[derive(Debug, Clone, Default)]
pub struct SmashGrab {
    pub inputs: SmashGrabInputs,
    pub state: SmashGrabState,
}

/// Clock ticks per second (`.rdata` 0x00f9f604; its reciprocal `.rdata` 0x00f9f608 is `0x34b2f4fc`).
pub const TICKS: f32 = 3.0e6;
/// The nitrous an item gives (`DAT_00fa929c`, through `FUN_00658ad0`).
pub const ITEM_NITROUS: f32 = 10.0;

/// What `DOSMSHNGRB` asks the engine to do.
#[derive(Debug, Clone, PartialEq)]
pub enum SmashGrabOut {
    /// Enter update phase 0: load `SmashAndGrabTrigger_1?ID=2210010` (string 0x00fd3b00) into +0x11e8.
    LoadItemModel,
    /// `FUN_0075e9a0`: the map icon `poi_key` on the trigger (`FUN_00793350`), the trigger switched on (message
    /// {1, 0, index, on} through `FUN_007160b0`), and the item prop on the trigger's position, upright on the ground
    /// a ±5 m ray finds (`FUN_004b3b20`, `.rdata` 0x00fd67f0), facing the trigger's local Z.
    Item {
        trigger: u32,
        poi_key: u32,
    },
    /// The item is gone (smashed or the exit): icon removed (`FUN_00790da0`), trigger off (message {1, 0, index,
    /// off} on `DAT_010edb48`), prop released (`FUN_00843e60`).
    ItemRemove {
        trigger: u32,
    },
    /// `FUN_00793350`(`poi_key`, the billboard's position, 0, 0, id): a billboard's map icon.
    BillboardPoi {
        trigger: u32,
        poi_key: u32,
    },
    BillboardPoiRemove {
        trigger: u32,
    },
    /// `FUN_006e41e0`: every billboard-kind trigger ('2' 'A' 'D' 'J' 'M') not in `keep` is hidden (the event
    /// data manager's list +0x1194, count +0x22c4, at most 1100).
    HideOtherBillboards {
        keep: Vec<u32>,
    },
    /// `FUN_006e4120`: the hidden billboards back.
    RestoreBillboards,
    /// `FUN_0076a950`({0x15, 0}): message 0x15 posted after the hide and after the restore (queue not traced).
    Message15,
    /// `FUN_006a8050`: the item's `ScoringAction`.
    Score {
        action: u32,
    },
}

fn out(s: &mut LogicState, o: SmashGrabOut) {
    s.requests.push(Request::SmashGrab(o));
}

/// The slots of `DOSMSHNGRB` (called from [`super::LogicHost::apply`]).
pub fn apply(s: &mut LogicState, r: &Request) {
    match r {
        Request::SmashGrabScoring(Slot::Enter) => enter(s),
        Request::SmashGrabScoring(Slot::EnterUpdate) => {
            enter_update(s);
        }
        Request::SmashGrabScoring(Slot::Update) => {
            update(s);
        }
        Request::SmashGrabScoring(Slot::ExitUpdate) => {
            exit_update(s);
        }
        Request::SmashGrabScoring(Slot::Exit) => leave(s),
        _ => {}
    }
}

/// Enter (`FUN_0075b820`): score and lists empty, the time limit in ticks (`FUN_00eda646`: truncation) and a tenth
/// of it, the medals, the worths from the config items, `DAT_010c5c08` = −1, the root registered.
pub fn enter(s: &mut LogicState) {
    let i = &s.smash_grab.inputs;
    let c = i.config;
    let limit = (f64::from(i.time_limit) * f64::from(TICKS)) as i64;
    let tenth = (limit as f64 * f64::from(f32::from_bits(0x34b2_f4fc)) * f64::from(0.1f32) * f64::from(TICKS)) as i64;
    s.smash_grab.state = SmashGrabState {
        active: true,
        medals: i.medals.map(|m| m as u32),
        time_limit: limit,
        next_clock: tenth,
        item_worth: u32::from(c.items[0].score),
        billboard_worth: u32::from(c.items[1].score),
        high_value_worth: u32::from(c.items[2].score),
        ..Default::default()
    };
    s.smash_grab.inputs.last_billboard = -1;
}

/// `FUN_0076c2b0` / `FUN_0076c3b0` / `FUN_0076c340`: find-or-insert in a map sorted by id.
fn insert(list: &mut Vec<u32>, id: u32) {
    if let Err(at) = list.binary_search(&id) {
        list.insert(at, id);
    }
}

/// The enter update (`FUN_007612e0`); true = entered. Phase 0 loads the item model; 1 (once loaded) builds the
/// lists, places the items and billboard icons, hides the other billboards; 2 waits for every prop.
pub fn enter_update(s: &mut LogicState) -> bool {
    if s.smash_grab.state.phase == 0 {
        out(s, SmashGrabOut::LoadItemModel);
        s.smash_grab.state.phase = 1;
    }
    if s.smash_grab.state.phase == 1 {
        if s.smash_grab.inputs.loading {
            return false;
        }
        build(s);
        let keep = s.smash_grab.state.billboards.clone();
        out(s, SmashGrabOut::HideOtherBillboards { keep });
        out(s, SmashGrabOut::Message15);
        s.smash_grab.state.phase = 2;
    }
    if s.smash_grab.state.phase == 2 {
        if !s.smash_grab.inputs.props_ready {
            return false;
        }
        s.smash_grab.state.phase = 3;
    }
    s.smash_grab.state.phase == 3
}

/// `FUN_00760b40`: the event's triggers by kind into the three maps, then the items placed (plain ones with the
/// first config item's key, high-value ones with the third's) and the billboards' icons (the second's key).
fn build(s: &mut LogicState) {
    let i = &s.smash_grab.inputs;
    let (pack3, c) = (i.pack3, i.config);
    let triggers = i.event_triggers.clone();
    let st = &mut s.smash_grab.state;
    for (id, kind) in triggers {
        match kind {
            KIND_ITEM => insert(&mut st.items, id),
            KIND_HIGH_VALUE => insert(&mut st.high_value, id),
            k if BILLBOARD_KINDS.contains(&k) => insert(&mut st.billboards, id),
            k if PACK_BILLBOARD_KINDS.contains(&k) && pack3 => insert(&mut st.billboards, id),
            _ => {}
        }
    }
    let (items, high, boards) = (st.items.clone(), st.high_value.clone(), st.billboards.clone());
    for trigger in items {
        out(s, SmashGrabOut::Item { trigger, poi_key: c.items[0].poi });
    }
    for trigger in high {
        out(s, SmashGrabOut::Item { trigger, poi_key: c.items[2].poi });
    }
    for trigger in boards {
        out(s, SmashGrabOut::BillboardPoi { trigger, poi_key: c.items[1].poi });
    }
}

fn clock(s: &LogicState) -> i64 {
    (u64::from(s.game_modes.word_2d28) | (u64::from(s.game_modes.word_2d2c) << 32)) as i64
}

/// `FUN_0074cf40`: the action, 10 nitrous, the worth added to the score.
fn award(s: &mut LogicState, worth: u32, action: u32) {
    out(s, SmashGrabOut::Score { action });
    if let Some(local) = s.players.local {
        s.requests.push(Request::NitrousAward { slot: local as i32, amount: ITEM_NITROUS });
    }
    let st = &mut s.smash_grab.state;
    st.score = st.score.wrapping_add(worth);
}

/// The update (`FUN_00761450`); true = finished (the exit in [`SmashGrabState::end`]). With the clock and the mode
/// timer running: the items the car touches and the billboard smashed; then, when the time is up or every list is
/// empty, the place by medal (1–3 WIN, else 4, GMM+0x2e78 cleared, LOSE) and the final data.
pub fn update(s: &mut LogicState) -> bool {
    s.smash_grab.state.just_hit_billboard = false;
    if !s.smash_grab.inputs.clock_running || !s.game_modes.timer_running {
        return false;
    }
    let racing = s.players.local_player().is_some_and(|p| p.vehicle_alive && !p.wrecked);
    if racing && s.smash_grab.inputs.in_sim {
        contacts(s);
        billboard(s);
    }
    let st = &s.smash_grab.state;
    let empty = st.items.is_empty() && st.high_value.is_empty() && st.billboards.is_empty();
    if st.time_limit > clock(s) && !empty {
        return false;
    }
    let score = st.score;
    let [gold, silver, bronze] = st.medals;
    let place = if score >= gold {
        1
    } else if score >= silver {
        2
    } else if score >= bronze {
        3
    } else {
        4
    };
    if place == 4 {
        s.game_modes.won = false;
    }
    finish(s, place);
    s.smash_grab.state.end = Some(if place < 4 { SmashGrabEnd::Win } else { SmashGrabEnd::Lose });
    true
}

/// `FUN_00760dd0`: the first contact that is a plain item's prop (then a high-value one's) takes it: removed from
/// the world and its map, awarded (`ItemWorth` / `HighValueWorth`), `LastItemBillboard` cleared. One per frame.
fn contacts(s: &mut LogicState) {
    let c = s.smash_grab.inputs.config;
    let contacts = std::mem::take(&mut s.smash_grab.inputs.contacts);
    for &trigger in &contacts {
        let st = &mut s.smash_grab.state;
        let (worth, action) = if let Ok(at) = st.items.binary_search(&trigger) {
            st.items.remove(at);
            (st.item_worth, c.items[0].action)
        } else if let Ok(at) = st.high_value.binary_search(&trigger) {
            st.high_value.remove(at);
            (st.high_value_worth, c.items[2].action)
        } else {
            continue;
        };
        out(s, SmashGrabOut::ItemRemove { trigger });
        award(s, worth, action);
        s.smash_grab.state.last_item_billboard = false;
        break;
    }
    s.smash_grab.inputs.contacts = contacts;
}

/// `FUN_0075ba30`: the billboard the collectables just smashed, when it is one of the event's: its icon removed,
/// `LastItemBillboard` and `JustHitBillboard` set, `BillboardWorth` awarded, removed from the map.
fn billboard(s: &mut LogicState) {
    let last = s.smash_grab.inputs.last_billboard;
    if last < 1 {
        return;
    }
    let trigger = last as u32;
    let Ok(at) = s.smash_grab.state.billboards.binary_search(&trigger) else { return };
    s.smash_grab.inputs.last_billboard = -1;
    out(s, SmashGrabOut::BillboardPoiRemove { trigger });
    let st = &mut s.smash_grab.state;
    st.last_item_billboard = true;
    st.just_hit_billboard = true;
    let worth = st.billboard_worth;
    let action = s.smash_grab.inputs.config.items[1].action;
    award(s, worth, action);
    s.smash_grab.state.billboards.remove(at);
}

/// The exit update (`FUN_0075ed20`); true = finished. Phase 0 removes every item (prop, icon, trigger) and the
/// billboard icons; 1 restores the hidden billboards and posts message 0x15; 2 finishes.
pub fn exit_update(s: &mut LogicState) -> bool {
    match s.smash_grab.state.exit_phase {
        0 => {
            let st = &s.smash_grab.state;
            let gone: Vec<u32> = st.items.iter().chain(&st.high_value).copied().collect();
            for trigger in gone {
                out(s, SmashGrabOut::ItemRemove { trigger });
            }
            let boards = std::mem::take(&mut s.smash_grab.state.billboards);
            for trigger in boards {
                out(s, SmashGrabOut::BillboardPoiRemove { trigger });
            }
            s.smash_grab.state.exit_phase = 1;
            false
        }
        1 => {
            out(s, SmashGrabOut::RestoreBillboards);
            out(s, SmashGrabOut::Message15);
            s.smash_grab.state.exit_phase = 2;
            false
        }
        _ => true,
    }
}

/// The exit (0x73cf00): the root unregistered, the player's score, the score to the Speed Points total when the
/// end reason is 0 / 1 (`FUN_00734f60`(score, 0, car)), then score 0 and `DAT_010c5c08` = −1.
pub fn leave(s: &mut LogicState) {
    s.smash_grab.state.active = false;
    let score = s.smash_grab.state.score;
    set_player_score(s, score as i32);
    let reason = s.game_modes.end_reason & 0xffff;
    if score > 0 && (reason == 1 || reason == 0) {
        let car = s.players.local_player().map_or(0, |p| p.vehicle_id);
        s.requests.push(Request::SpeedPoints { sp: score as i32, bucket: 0, car });
    }
    s.smash_grab.state.score = 0;
    s.smash_grab.inputs.last_billboard = -1;
}

/// The query answers of the behaviour: entered, the update's end (0 none, 1 WIN, 2 LOSE), the exit update done.
pub fn entered(s: &LogicState) -> bool {
    s.smash_grab.state.phase == 3
}

pub fn end_code(s: &LogicState) -> u64 {
    match s.smash_grab.state.end {
        None => 0,
        Some(SmashGrabEnd::Win) => 1,
        Some(SmashGrabEnd::Lose) => 2,
    }
}

pub fn cleared(s: &LogicState) -> bool {
    s.smash_grab.state.exit_phase >= 2
}

/// 0x768980: 1 gold, 2 silver, 3 bronze, 0 none (unsigned compares).
fn medal(st: &SmashGrabState) -> u32 {
    let [gold, silver, bronze] = st.medals;
    if st.score >= gold {
        1
    } else if st.score >= silver {
        2
    } else if st.score >= bronze {
        3
    } else {
        0
    }
}

/// The root `SmashScoring` while the behaviour runs.
pub fn lookup(s: &LogicState, members: &[Segment]) -> Lookup {
    let st = &s.smash_grab.state;
    if !st.active {
        return Lookup::Null;
    }
    let Some((first, _)) = members.split_first() else { return Lookup::Found(Value::Object(true)) };
    let is = |n: &str| first.hash == crate::binding::hash(n);
    let [gold, silver, bronze] = st.medals;
    let v = if is("Score") {
        Value::UInt(st.score)
    } else if is("LastItemBillboard") {
        Value::Bool(st.last_item_billboard)
    } else if is("JustHitBillboard") {
        Value::Bool(st.just_hit_billboard)
    } else if is("ItemWorth") {
        Value::UInt(st.item_worth)
    } else if is("BillboardWorth") {
        Value::UInt(st.billboard_worth)
    } else if is("HighValueWorth") {
        Value::UInt(st.high_value_worth)
    } else if is("TimeToNextClock") {
        // 0x76a1c0: +0x1200 − the race clock (ticks).
        Value::Int64(st.next_clock.wrapping_sub(clock(s)))
    } else if is("TimeRemaining") {
        // 0x76a190: +0x11f0 − the race clock (ticks).
        Value::Int64(st.time_limit.wrapping_sub(clock(s)))
    } else if is("CurrentMedalIndex") {
        Value::UInt(medal(st))
    } else if is("TargetMedalIndex") {
        // 0x7689d0: below bronze 3, below silver 2, else 1.
        Value::UInt(if st.score < bronze {
            3
        } else if st.score < silver {
            2
        } else {
            1
        })
    } else if is("NextTargetScore") {
        // 0x768a10: the next medal's score (gold once silver is reached).
        Value::UInt(if st.score < bronze {
            bronze
        } else if st.score < silver {
            silver
        } else {
            gold
        })
    } else {
        return Lookup::Null;
    };
    Lookup::Found(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::natives::players::Player;

    fn state() -> LogicState {
        let mut s = LogicState::default();
        s.players.local = Some(0);
        s.players.slots[0] = Some(Player { vehicle: 1, vehicle_alive: true, vehicle_id: 911, ..Default::default() });
        s.game_modes.timer_running = true;
        s.game_modes.won = true;
        let i = &mut s.smash_grab.inputs;
        i.event_triggers =
            vec![(30, KIND_ITEM), (10, KIND_ITEM), (20, KIND_HIGH_VALUE), (40, b'A'), (50, b'J'), (60, 7)];
        i.time_limit = 120.0;
        i.medals = [400, 250, 100];
        s
    }

    fn outs(s: &mut LogicState) -> Vec<SmashGrabOut> {
        std::mem::take(&mut s.requests)
            .into_iter()
            .filter_map(|r| if let Request::SmashGrab(o) = r { Some(o) } else { None })
            .collect()
    }

    fn entered() -> LogicState {
        let mut s = state();
        apply(&mut s, &Request::SmashGrabScoring(Slot::Enter));
        s.smash_grab.inputs.props_ready = false;
        assert!(!enter_update(&mut s), "waits for the props");
        s.smash_grab.inputs.props_ready = true;
        assert!(enter_update(&mut s));
        s
    }

    #[test]
    fn lists_by_kind_and_placement() {
        let mut s = entered();
        let st = &s.smash_grab.state;
        assert_eq!((&st.items[..], &st.high_value[..], &st.billboards[..]), (&[10, 30][..], &[20][..], &[40][..]));
        assert_eq!(st.time_limit, 360_000_000);
        assert_eq!(st.next_clock, 36_000_000);
        let o = outs(&mut s);
        assert_eq!(o[0], SmashGrabOut::LoadItemModel);
        assert!(o.contains(&SmashGrabOut::Item { trigger: 10, poi_key: 2_210_071 }));
        assert!(o.contains(&SmashGrabOut::Item { trigger: 20, poi_key: 2_210_110 }));
        assert!(o.contains(&SmashGrabOut::BillboardPoi { trigger: 40, poi_key: 2_210_109 }));
        assert!(o.contains(&SmashGrabOut::HideOtherBillboards { keep: vec![40] }));
        assert_eq!(s.resolve_int("SmashScoring.ItemWorth"), Some(50));
        assert_eq!(s.resolve_int("SmashScoring.HighValueWorth"), Some(150));
        assert_eq!(s.resolve_int("SmashScoring.NextTargetScore"), Some(100));
        // With game pack 3 the 'J' billboard counts too.
        let mut s = state();
        s.smash_grab.inputs.pack3 = true;
        apply(&mut s, &Request::SmashGrabScoring(Slot::Enter));
        enter_update(&mut s);
        assert_eq!(s.smash_grab.state.billboards, [40, 50]);
    }

    #[test]
    fn smashing_items_and_billboards_then_all_taken_wins() {
        let mut s = entered();
        outs(&mut s);
        // Two contacts in one frame: only the first item counts.
        s.smash_grab.inputs.contacts = vec![99, 20, 10];
        assert!(!update(&mut s));
        assert_eq!(s.smash_grab.state.score, 150);
        assert_eq!(s.smash_grab.state.high_value, [] as [u32; 0]);
        let o = outs(&mut s);
        assert_eq!(o, [SmashGrabOut::ItemRemove { trigger: 20 }, SmashGrabOut::Score { action: 2_210_197 }]);
        assert!(s.requests.is_empty());
        assert!(!update(&mut s));
        assert_eq!(s.smash_grab.state.score, 200);
        // The collectables smash billboard 40: worth 250, the billboard flags.
        s.requests.clear();
        s.smash_grab.inputs.contacts.clear();
        s.smash_grab.inputs.last_billboard = 40;
        assert!(!update(&mut s));
        assert_eq!(s.smash_grab.state.score, 450);
        assert!(s.smash_grab.state.just_hit_billboard && s.smash_grab.state.last_item_billboard);
        assert_eq!(s.smash_grab.inputs.last_billboard, -1);
        assert!(s.requests.contains(&Request::NitrousAward { slot: 0, amount: ITEM_NITROUS }));
        assert!(outs(&mut s).contains(&SmashGrabOut::Score { action: 2_210_198 }));
        assert_eq!(s.resolve_int("SmashScoring.CurrentMedalIndex"), Some(1));
        // The last item: every list is empty, gold: WIN.
        s.smash_grab.inputs.contacts = vec![30];
        s.game_modes.word_2d28 = 3_000_000 * 60;
        assert!(update(&mut s));
        assert_eq!(s.smash_grab.state.end, Some(SmashGrabEnd::Win));
        assert!(!s.smash_grab.state.just_hit_billboard, "cleared every frame");
        assert_eq!(s.players.local_player().unwrap().final_5b8, 1);
        assert!(s.game_modes.won);
        assert_eq!(s.resolve_int("SmashScoring.TimeRemaining"), Some(180_000_000));
    }

    #[test]
    fn time_up_without_a_medal_loses_and_exit_clears() {
        let mut s = entered();
        s.smash_grab.inputs.contacts = vec![10];
        update(&mut s);
        s.game_modes.word_2d28 = 360_000_000;
        assert!(update(&mut s));
        assert_eq!(s.smash_grab.state.end, Some(SmashGrabEnd::Lose));
        assert!(!s.game_modes.won);
        assert_eq!(s.players.local_player().unwrap().final_5b8, 4);
        outs(&mut s);
        assert!(!exit_update(&mut s));
        let o = outs(&mut s);
        assert!(
            o.contains(&SmashGrabOut::ItemRemove { trigger: 30 })
                && o.contains(&SmashGrabOut::ItemRemove { trigger: 20 })
        );
        assert!(o.contains(&SmashGrabOut::BillboardPoiRemove { trigger: 40 }));
        assert!(!exit_update(&mut s));
        assert_eq!(outs(&mut s), [SmashGrabOut::RestoreBillboards, SmashGrabOut::Message15]);
        assert!(exit_update(&mut s));
        apply(&mut s, &Request::SmashGrabScoring(Slot::Exit));
        assert!(s.requests.contains(&Request::SpeedPoints { sp: 50, bucket: 0, car: 911 }));
        let p = s.players.local_player().unwrap();
        assert_eq!((p.score, p.score_348), (50, 50));
        assert_eq!(s.smash_grab.state.score, 0);
        assert_eq!(lookup(&s, &[]), Lookup::Null);
    }
}
