//! The jack spots of `HSMCARSWAPLO` ("Car Swap Logic", vtable 0xfdb4c8, 0x74d0 B; decomp.md "Jack spots: the parked
//! cars and the swap"): which parked cars stand, the prompt, the swap's seven phases, the spot's reset. Ported from
//! the code: update `FUN_00761d70` (with `FUN_007615f0` distances / prompt / input, `FUN_0075d290` the standing
//! rule, `FUN_00742400` eligibility, `FUN_00742210` "in the location", `FUN_0075d590` the swap's start,
//! `FUN_00759410` the nearest standing car, `FUN_0075d800` / `FUN_007596b0` the reset of the last spot).
//!
//! **What the runtime fills** ([`CarSwapInputs`]): the spots from the `CarSwapSpot` list (`FUN_00747aa0`: copied
//! into the logic's slots on enter and on a rebuild), what the engine reports about each parked car (its spawn
//! result and readiness), the player's car (box, position, speed, car id, colour, pursuit), the SWAP_CARS press and
//! the engine waits of the swap's phases (each field names its function). **What the logic decides**
//! ([`CarSwapState`]): each slot's state (none / requested / spawning / standing), distance, eligibility, the prompt
//! (GMM+0x2c70 / +0x2ca7), the swap's phase (+0x74a0), the slot data the swap changes (car and colour left on the
//! spot), the latch (+0x74c9), the last swap spot (+0x74b8), GMM+0x252f, input block 2 bit 7. **What the runtime
//! carries out**: [`CarSwapOut`].

use super::LogicState;
use super::state::Request;
use super::trigger_boxes::{CarBox, TriggerBox, V3, dist2, hits};

/// A car is requested from 20 m (`.data` 0x010c5cac = 400, squared) ...
pub const SPAWN_MIN_D2: f32 = 400.0;
/// ... to 300 m (`.data` 0x010c5ca4 = 90000; 0x010c5ca8 = 1e6 in the intro free drive).
pub const SPAWN_MAX_D2: f32 = 90_000.0;
pub const SPAWN_MAX_D2_INTRO: f32 = 1_000_000.0;
/// The location query's radius (`.rdata` 0x00fd6800: 300 m) and the nearest five keep their distance.
pub const QUERY_D2: f32 = 90_000.0;
pub const NEAREST: usize = 5;
/// The last swap spot is rebuilt from its data beyond 300 m (`.rdata` 0x00fd67e0 = 90000).
pub const RESET_D2: f32 = 90_000.0;
/// In the intro "in the spot" is within 22.5 m (`.rdata` 0x00fd6b70) of the location, and only this one counts.
pub const INTRO_RADIUS: f32 = 22.5;
pub const INTRO_LOCATION: u32 = 0x12_7eae;

/// A spot's data (`CarSwapSpot`, GAMEPLAY; slot +0x40..+0x70).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SpotData {
    /// +0x60 `Placement`: the location trigger and its box (`None`: not streamed).
    pub location: u32,
    pub location_box: Option<TriggerBox>,
    /// +0x64 `VehicleId`.
    pub vehicle: u32,
    /// +0x68 `0e3bcabe`: the parked car's trigger; +0x40 / +0x50 its position and direction (`FUN_0042f250`).
    pub car_trigger: u32,
    pub car_position: V3,
    pub car_direction: V3,
    /// +0x6c `e36e9d9d`: the second parking place (trigger, position, direction).
    pub second: Option<(u32, V3, V3)>,
    /// +0x70 `ColourId`.
    pub colour: u32,
}

/// What the engine reports about a slot's parked car.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ParkedReport {
    /// +0x80 / +0x88 after a spawn request (`FUN_0066bf90`): 0 pending, 3 done (with the handle), 4 failed.
    pub spawn_status: u32,
    pub handle: u32,
    /// `FUN_005733b0`(handle): the vehicle is ready (spawning → standing).
    pub ready: bool,
    /// `FUN_00573390`(handle): the vehicle exists (the nearest standing car).
    pub alive: bool,
    /// Its position (`FUN_00573420`).
    pub position: V3,
}

/// Per-slot profile answers (`FUN_00742400`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SpotProfile {
    /// `FUN_0068f190`(vehicle): the profile has the car (else the prompt offers the store).
    pub owned: bool,
    /// `FUN_00643c10`(pack) and not `FUN_006ec300`(vehicle): a pack car offered through the store.
    pub store: bool,
}

/// The player's car.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SwapPlayer {
    /// `FUN_006ba2a0`: has a vehicle in the world.
    pub has_vehicle: bool,
    /// `FUN_006ba2f0`: its car is loaded.
    pub loaded: bool,
    pub car: CarBox,
    /// |speed| in mph (`FUN_006ba3e0`).
    pub speed_mph: f32,
    /// The police record's heat (G+0x2f410 + slot × 0x8b0 +0x874; above 0 no swap).
    pub heat: f32,
    /// `FUN_006ba290`: player +0x23f (crashed).
    pub crashed: bool,
    /// `FUN_006c2410`: in a pursuit.
    pub in_pursuit: bool,
    /// The player's car id (its vehicle record +0x30 through the player's vtable +0x18; `FUN_006ba320`).
    pub vehicle: u32,
    /// `FUN_006b9d80`: the profile's current car.
    pub profile_car: u32,
    /// `FUN_0083bed0`: its colour.
    pub colour: u32,
    /// G+0x25694 + vehicle.
    pub resetting: bool,
}

/// What the runtime supplies each frame.
#[derive(Debug, Clone, Default)]
pub struct CarSwapInputs {
    /// The `CarSwapSpot` list as `FUN_00747aa0` keeps it (spots of unowned game packs left out).
    pub spots: Vec<SpotData>,
    pub profile: Vec<SpotProfile>,
    pub parked: Vec<ParkedReport>,
    pub player: SwapPlayer,
    /// +0x74c8: the engine has started the logic (set outside the decompiled set).
    pub started: bool,
    /// Player 0's action 14 SWAP_CARS (input record +0xc4) in state 2.
    pub swap_pressed: bool,
    /// `FUN_006e0430`: the intro free drive; UI manager +0x2de98 == 0x1ab434 (its screen).
    pub intro: bool,
    pub intro_screen: bool,
    /// `DAT_010c5bb8`: a swap asked from Lua (a location id; Easydrive's `JackIt`), `DAT_010c5bb4`: the spot
    /// left out (0 = none).
    pub jack_it: u32,
    pub jack_it_excluded: u32,
    /// `DAT_01179ddc`: every slot is rebuilt.
    pub rebuild: bool,
    /// Phase 1: +0x74c4, the movie / transition covers the screen.
    pub covered: bool,
    /// Phase 2 without a standing car: `FUN_0073b520`.
    pub exchange_ready: bool,
    /// Phase 4: `FUN_0073b600` (placing still under way).
    pub placing: bool,
    /// Phase 6: `FUN_006b9430`.
    pub sequence_done: bool,
    /// Phase 7: `FUN_00573390` and `FUN_0057a010`(player's vehicle) and `FUN_0073b640`.
    pub vehicles_ready: bool,
    /// Phase 5: `FUN_006fa600`(`FUN_006fa850`(car trigger)): the car trigger was found before (B+0x74c7).
    pub car_trigger_found: bool,
    /// `GameLogicConfig` +0x284 (`ff4aef6f`, 70), +0x16c (`a1e83efa`, 955092), +0x168 (`PromptStore`, 2094166),
    /// +0x8c (the swap sequence, 995872).
    pub max_mph: f32,
    pub prompt_drive: u32,
    pub prompt_store: u32,
    pub swap_sequence: u32,
}

/// A slot (0xa0 B at B+0x50).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Slot {
    pub data: SpotData,
    /// +0x74 the parked vehicle's handle, +0x78 its state (0 none, 1 requested, 2 spawning, 3 standing).
    pub handle: Option<u32>,
    pub state: u8,
    /// +0x90: the squared distance (FLT_MAX unless among the nearest five within 300 m).
    pub d2: f32,
    /// +0x97: the parked car goes to the car trigger next (else the second place).
    pub second_used: bool,
    /// +0x98 eligible, +0x99 in range.
    pub eligible: bool,
    pub in_range: bool,
    /// +0xe4: the pursuit score event was sent; +0xe5 first use done; +0xe6 used before (`FUN_0075d590`).
    pub score_sent: bool,
    pub first_use_done: bool,
    pub used_before: bool,
}

/// The swap under way (+0x7420..+0x749c).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SwapRun {
    /// +0x7428 slot, +0x7458 its car trigger, +0x745c the parked vehicle (`None` = −1).
    pub slot: usize,
    pub car_trigger: u32,
    pub parked: Option<u32>,
}

/// The behaviour's state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CarSwapState {
    pub slots: Vec<Slot>,
    /// +0x74a0.
    pub phase: u8,
    pub swap: SwapRun,
    /// +0x74a8: the player's colour before the swap (the spot keeps it).
    pub old_colour: u32,
    /// +0x74ac: the player's car last frame; +0x74b0 the use counter; +0x74b8 the last swap spot; +0x74bc the
    /// eligibility round robin.
    pub last_vehicle: u32,
    pub uses: u32,
    pub last_spot: Option<usize>,
    pub round_robin: usize,
    /// +0x74c5 in a pursuit last frame; +0x74c6 the slots are kept on a car change; +0x74c9 the latch; +0x74ca
    /// input blocked; +0x74cb / +0x74cc the intro screen and its passing.
    pub was_in_pursuit: bool,
    pub keep_slots: bool,
    pub latch: bool,
    pub input_blocked: bool,
    pub intro_screen: bool,
    pub intro_passed: bool,
    /// `DAT_010c5bbc`: the nearest standing car's handle.
    pub nearest: Option<u32>,
}

/// The logic's car-swap manager.
#[derive(Debug, Clone, Default)]
pub struct CarSwap {
    pub inputs: CarSwapInputs,
    pub state: CarSwapState,
}

/// What `HSMCARSWAPLO` asks the engine to do.
#[derive(Debug, Clone, PartialEq)]
pub enum CarSwapOut {
    /// `FUN_0066bf90`: spawn the slot's parked car at rest (vehicle, colour, position, direction).
    Spawn { slot: usize, vehicle: u32, colour: u32, position: V3, direction: V3 },
    /// `FUN_00758b00`: the parked car goes (handle when it had one).
    Remove { slot: usize, handle: Option<u32> },
    /// `FUN_0071c8e0`: preload the nearest spot's car.
    Preload { vehicle: u32 },
    /// `FUN_0075d590`: the swap starts (the config's sequences +0x8c / +0x90, message 0x1f(1); on a spot's first
    /// use `FUN_0083c220`(car), the heat bump `FUN_006bdac0`, `FUN_00703dc0`, `FUN_007357f0`(0)).
    Begin { slot: usize, first_use: bool },
    /// `FUN_00748090`: the store for a car the profile lacks.
    OpenStore { slot: usize },
    /// Phase 1 (`FUN_00679670`, `FUN_0067f700`; message 0x1c for +0x742c): the screen is covered.
    Covered,
    /// Phase 2 (`FUN_00758f00`): the player's vehicle becomes `vehicle` in `colour` (`FUN_006c2020`, repair
    /// message 0x18, the profile's rollout `FUN_00730ec0`); the parked one (if any) the car the player came in
    /// (`FUN_00844580`), and both exchange their physical state (message 0x17 twice).
    Exchange { slot: usize, vehicle: u32, colour: u32, parked: Option<u32>, old_vehicle: u32 },
    /// Phase 3 (`FUN_00647e00` twice, `FUN_00673770`): the player's vehicle onto the parked car's pose, the parked
    /// one onto `parked_pose`.
    Place { player_pose: (V3, V3), parked: Option<u32>, parked_pose: (V3, V3) },
    /// Phase 5: the swap sequence (+0x8c, not in the intro) and `FUN_007355b0`(1, 1).
    Sequence { sequence: Option<u32> },
    /// Phase 6: `FUN_007059b0`(player, trigger, 0, 0, 1): the trigger counts as found.
    Found { trigger: u32 },
    /// Phase 6 in a pursuit: `FUN_00639b60` on G+0x2a890, and on the spot's first time `ScoringConfig` +0x40
    /// (955227 "CAR JACK USED").
    PursuitSwap { score: bool },
    /// Phase 7: messages 0x18 for both vehicles, the colours exchanged (`FUN_00844660`), `FUN_00679670`,
    /// `FUN_007683f0`(slot).
    Finish { player_colour: u32, parked: Option<u32>, parked_colour: u32 },
    /// `FUN_007426e0`: a pursuit started.
    PursuitStarted,
    /// `FUN_00759410`: the nearest standing car (its sequence `GameLogicConfig` +0x104; `None` = none).
    Nearest { handle: Option<u32> },
}

fn out(s: &mut LogicState, o: CarSwapOut) {
    s.requests.push(Request::CarSwap(o));
}

/// The `HSMCARSWAPLO` requests the logic carries out (from [`super::LogicHost::apply`]).
pub fn apply(s: &mut LogicState, r: &Request) {
    match r {
        Request::CarSwapEnter => enter(s),
        Request::CarSwapStep => update(s),
        Request::CarSwapExit => exit(s),
        _ => {}
    }
}

/// Enter (`FUN_00758ba0` → `FUN_00747aa0`): the slots from the data.
pub fn enter(s: &mut LogicState) {
    s.car_swap.state = CarSwapState { last_vehicle: s.car_swap.inputs.player.vehicle, ..Default::default() };
    build(s);
}

/// `FUN_00747aa0`: one slot per spot, from its data.
fn build(s: &mut LogicState) {
    s.car_swap.state.slots =
        s.car_swap.inputs.spots.iter().map(|&data| Slot { data, d2: f32::MAX, ..Default::default() }).collect();
}

/// `FUN_007592a0`: every parked car goes.
fn remove_all(s: &mut LogicState) {
    for slot in 0..s.car_swap.state.slots.len() {
        if s.car_swap.state.slots[slot].state != 0 {
            remove(s, slot);
        }
    }
}

fn remove(s: &mut LogicState, slot: usize) {
    let sl = &mut s.car_swap.state.slots[slot];
    let handle = sl.handle.take();
    sl.state = 0;
    if s.car_swap.state.nearest.is_some() && s.car_swap.state.nearest == handle {
        s.car_swap.state.nearest = None;
    }
    out(s, CarSwapOut::Remove { slot, handle });
}

/// Exit (`FUN_0075d510`): the cars go, GMM+0x252f cleared, input block 2 bit 7 released.
pub fn exit(s: &mut LogicState) {
    remove_all(s);
    s.game_modes.swap_in_progress = false;
    if s.car_swap.state.input_blocked {
        set_block(s, false);
    }
}

fn set_block(s: &mut LogicState, set: bool) {
    s.car_swap.state.input_blocked = set;
    if set {
        s.game.input_blocks.set(2, 7);
    } else {
        s.game.input_blocks.clear(2, 7);
    }
    s.requests.push(Request::SetInputBlock { group: 2, bit: 7, set });
}

/// The update (`FUN_00761d70`); never exits by itself.
pub fn update(s: &mut LogicState) {
    let i = s.car_swap.inputs.clone();
    if !i.started {
        return;
    }
    if i.intro {
        if s.car_swap.state.intro_screen && !i.intro_screen {
            s.car_swap.state.intro_passed = true;
        }
        s.car_swap.state.intro_screen = i.intro_screen;
    }
    let active = s.car_swap.state.phase != 0;
    if active != s.car_swap.state.input_blocked {
        set_block(s, active);
    }
    if i.player.in_pursuit && !s.car_swap.state.was_in_pursuit {
        out(s, CarSwapOut::PursuitStarted);
    }
    let p = i.player;
    if p.profile_car != s.car_swap.state.last_vehicle && p.profile_car == p.vehicle && !s.car_swap.state.keep_slots {
        remove_all(s);
        build(s);
    }
    match s.car_swap.state.phase {
        0 => prompt_and_input(s),
        1 => {
            if i.covered {
                out(s, CarSwapOut::Covered);
                s.car_swap.state.phase = 2;
            }
        }
        2 => {
            let slot = s.car_swap.state.swap.slot;
            standing(s, slot);
            let sl = s.car_swap.state.slots[slot];
            if sl.state == 3 {
                s.car_swap.state.swap.parked = sl.handle;
            } else if !i.exchange_ready {
                return finish_frame(s);
            }
            exchange(s);
            s.car_swap.state.phase = 3;
        }
        3 => {
            let slot = s.car_swap.state.swap.slot;
            let sl = s.car_swap.state.slots[slot];
            // The player's car onto the parked car's pose (`FUN_00742530` / `FUN_00742580`); the parked one onto
            // the second place the first time, the car trigger the next; without a second place where the
            // player's car was.
            let player_pose = (sl.data.car_position, sl.data.car_direction);
            let parked_pose = match sl.data.second {
                Some((_, pos, dir)) if !sl.second_used => {
                    s.car_swap.state.slots[slot].second_used = true;
                    (pos, dir)
                }
                Some(_) => {
                    s.car_swap.state.slots[slot].second_used = false;
                    (sl.data.car_position, sl.data.car_direction)
                }
                None => (p.car.centre, p.car.forward),
            };
            let parked = s.car_swap.state.swap.parked;
            out(s, CarSwapOut::Place { player_pose, parked, parked_pose });
            s.car_swap.state.phase = 4;
        }
        4 => {
            let busy = p.resetting || !p.loaded;
            if !busy && !i.placing {
                s.car_swap.state.phase = 5;
            }
        }
        5 => {
            let sequence = (i.swap_sequence != 0 && !i.intro).then_some(i.swap_sequence);
            out(s, CarSwapOut::Sequence { sequence });
            s.car_swap.state.phase = 6;
        }
        6 => {
            if i.sequence_done {
                let run = s.car_swap.state.swap;
                out(s, CarSwapOut::Found { trigger: run.car_trigger });
                let location = s.car_swap.state.slots[run.slot].data.location;
                out(s, CarSwapOut::Found { trigger: location });
                if p.in_pursuit {
                    let score = !s.car_swap.state.slots[run.slot].score_sent;
                    s.car_swap.state.slots[run.slot].score_sent = true;
                    out(s, CarSwapOut::PursuitSwap { score });
                }
                s.car_swap.state.phase = 7;
            }
        }
        _ => {
            if i.vehicles_ready {
                let st = &mut s.car_swap.state;
                let slot = st.swap.slot;
                let player_colour = st.slots[slot].data.colour;
                let parked_colour = st.old_colour;
                st.slots[slot].data.colour = parked_colour;
                st.latch = true;
                st.phase = 0;
                let parked = st.swap.parked;
                out(s, CarSwapOut::Finish { player_colour, parked, parked_colour });
            }
        }
    }
    finish_frame(s);
}

/// The update's tail: GMM+0x252f, the nearest standing car, the last spot's reset, +0x74ac, +0x74c5.
fn finish_frame(s: &mut LogicState) {
    s.game_modes.swap_in_progress = s.car_swap.state.phase != 0;
    nearest_standing(s);
    reset_last_spot(s);
    s.car_swap.state.last_vehicle = s.car_swap.inputs.player.vehicle;
    s.car_swap.state.was_in_pursuit = s.car_swap.inputs.player.in_pursuit;
}

/// `FUN_00758f00` (phase 2): the player's old colour kept; the player's car becomes the slot's; the slot's car id
/// becomes the player's old car when a parked vehicle stood there.
fn exchange(s: &mut LogicState) {
    let p = s.car_swap.inputs.player;
    let st = &mut s.car_swap.state;
    st.old_colour = p.colour;
    let slot = st.swap.slot;
    let data = st.slots[slot].data;
    let parked = st.swap.parked;
    if parked.is_some() {
        st.slots[slot].data.vehicle = p.vehicle;
    }
    out(s, CarSwapOut::Exchange { slot, vehicle: data.vehicle, colour: data.colour, parked, old_vehicle: p.vehicle });
}

/// `FUN_0075d590`: the swap of `slot` starts (phase 1).
fn begin(s: &mut LogicState, slot: usize) {
    let st = &mut s.car_swap.state;
    st.keep_slots = false;
    let sl = &mut st.slots[slot];
    if !sl.used_before {
        if st.uses == 0 {
            sl.used_before = true;
        }
        st.uses += 1;
    } else {
        sl.used_before = false;
        st.keep_slots = true;
        st.uses = 0;
    }
    let first_use = !sl.first_use_done;
    sl.first_use_done = true;
    st.swap = SwapRun { slot, car_trigger: sl.data.car_trigger, parked: None };
    st.phase = 1;
    out(s, CarSwapOut::Begin { slot, first_use });
}

/// `FUN_00742400`: the slot has a car, the profile has it (or it is a store car), and it is not the spot left out;
/// in the intro only location 0x127eae.
fn eligibility(s: &mut LogicState, slot: usize) {
    let i = &s.car_swap.inputs;
    let sl = &s.car_swap.state.slots[slot];
    let excluded = i.jack_it_excluded != 0
        && (sl.data.car_trigger == i.jack_it_excluded || sl.data.location == i.jack_it_excluded);
    let pr = i.profile.get(slot).copied().unwrap_or_default();
    let mut ok = pr.owned || pr.store;
    if i.intro {
        ok = sl.data.location == INTRO_LOCATION;
    }
    let eligible = !excluded && sl.data.vehicle != 0 && ok;
    s.car_swap.state.slots[slot].eligible = eligible;
}

/// `FUN_0075d290`: the parked car's state machine (requested → spawning on the engine's answer → standing when
/// ready), then wanted = eligible and (20–300 m, or standing and within 300 m): a wanted car with none is
/// requested, an unwanted one removed.
fn standing(s: &mut LogicState, slot: usize) {
    let report = s.car_swap.inputs.parked.get(slot).copied().unwrap_or_default();
    let intro = s.car_swap.inputs.intro;
    let rebuild = s.car_swap.inputs.rebuild;
    let sl = &mut s.car_swap.state.slots[slot];
    match sl.state {
        1 => match report.spawn_status {
            3 => {
                sl.handle = Some(report.handle);
                sl.state = 2;
            }
            4 => sl.state = 0,
            _ => return,
        },
        2 if report.ready => sl.state = 3,
        _ => {}
    }
    let r = if intro { SPAWN_MAX_D2_INTRO } else { SPAWN_MAX_D2 };
    let d2 = sl.d2;
    let stays = sl.state == 3 && d2 <= r;
    let in_range = (d2 <= r && d2 >= SPAWN_MIN_D2) || stays;
    let was_in_range = sl.in_range;
    sl.in_range = in_range;
    let mut wanted = false;
    if in_range {
        if !sl.eligible && !was_in_range {
            eligibility(s, slot);
        }
        wanted = s.car_swap.state.slots[slot].eligible && !rebuild;
    }
    let sl = s.car_swap.state.slots[slot];
    if wanted {
        if sl.state == 0 {
            out(
                s,
                CarSwapOut::Spawn {
                    slot,
                    vehicle: sl.data.vehicle,
                    colour: sl.data.colour,
                    position: sl.data.car_position,
                    direction: sl.data.car_direction,
                },
            );
            s.car_swap.state.slots[slot].state = 1;
        }
    } else if sl.state != 0 {
        remove(s, slot);
    }
}

/// "In the spot" (`FUN_00742210`): the location trigger in the player's hit list (in the intro within 22.5 m of
/// the location; a spot without a location: of the car's place).
fn in_spot(s: &LogicState, slot: usize) -> bool {
    let i = &s.car_swap.inputs;
    let d = s.car_swap.state.slots[slot].data;
    if d.location == 0 {
        return dist2(i.player.car.centre, d.car_position) <= INTRO_RADIUS * INTRO_RADIUS;
    }
    if !i.intro {
        return d.location_box.is_some_and(|b| hits(&i.player.car, &b));
    }
    let at = d.location_box.map_or(d.car_position, |b| b.centre);
    dist2(i.player.car.centre, at) <= INTRO_RADIUS * INTRO_RADIUS
}

/// `FUN_007615f0` (phase 0): distances (the nearest five locations within 300 m), each slot's standing rule, the
/// prompt and the SWAP_CARS press, the latch, the Lua swap, one slot's eligibility per frame.
fn prompt_and_input(s: &mut LogicState) {
    let i = s.car_swap.inputs.clone();
    let p = i.player;
    if !p.has_vehicle {
        return;
    }
    let mut allowed = p.heat <= 0.0 && !p.crashed;
    if i.intro && !s.car_swap.state.intro_passed {
        allowed = false;
    }
    // `FUN_007610c0`: the locations within 300 m, nearest first; the nearest five keep their distance.
    let mut near: Vec<(f32, usize)> = s
        .car_swap
        .state
        .slots
        .iter()
        .enumerate()
        .filter_map(|(k, sl)| {
            let at = sl.data.location_box.map(|b| b.centre)?;
            let d2 = dist2(at, p.car.centre);
            (d2 <= QUERY_D2).then_some((d2, k))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.truncate(NEAREST);
    let mut prompt = None;
    let mut in_any = false;
    for slot in 0..s.car_swap.state.slots.len() {
        let d2 = near.iter().position(|&(_, k)| k == slot).map_or(f32::MAX, |n| {
            if n == 0 {
                out(s, CarSwapOut::Preload { vehicle: s.car_swap.state.slots[slot].data.vehicle });
            }
            near[n].0
        });
        s.car_swap.state.slots[slot].d2 = d2;
        standing(s, slot);
        let standing_car = s.car_swap.state.slots[slot].state == 3;
        if !(allowed && p.speed_mph.abs() <= i.max_mph && standing_car && in_spot(s, slot)) {
            continue;
        }
        in_any = true;
        if s.car_swap.state.latch {
            continue;
        }
        if i.intro && p.profile_car != p.vehicle {
            continue;
        }
        let owned = i.profile.get(slot).is_some_and(|pr| pr.owned);
        if i.swap_pressed {
            if owned {
                begin(s, slot);
                allowed = false;
                let last = s.car_swap.state.last_spot.replace(slot);
                if let Some(last) = last.filter(|&l| l != slot) {
                    reset_spot(s, last);
                }
            } else {
                out(s, CarSwapOut::OpenStore { slot });
            }
        } else {
            prompt = Some(if owned { i.prompt_drive } else { i.prompt_store });
        }
    }
    if let Some(text) = prompt {
        s.game_modes.prompt = Some(text);
    }
    if s.car_swap.state.latch && !in_any {
        s.car_swap.state.latch = false;
    }
    if i.rebuild {
        remove_all(s);
        build(s);
    }
    if allowed && i.jack_it > 0 && !s.game_modes.event_start_held {
        let found = s.car_swap.state.slots.iter().position(|sl| sl.data.location == i.jack_it);
        if let Some(slot) = found {
            if i.profile.get(slot).is_some_and(|pr| pr.owned) {
                begin(s, slot);
                for k in 0..s.car_swap.state.slots.len() {
                    eligibility(s, k);
                }
            } else {
                out(s, CarSwapOut::OpenStore { slot });
            }
        }
    }
    let n = s.car_swap.state.slots.len();
    if n > 0 {
        let k = s.car_swap.state.round_robin;
        if k < n {
            eligibility(s, k);
        }
        s.car_swap.state.round_robin = (k + 1) % n;
    }
}

/// `FUN_007596b0`: the spot's car goes and the slot is rebuilt from its data.
fn reset_spot(s: &mut LogicState, slot: usize) {
    if s.car_swap.state.slots[slot].state == 3 {
        remove(s, slot);
    }
    if let Some(&data) = s.car_swap.inputs.spots.get(slot) {
        s.car_swap.state.slots[slot] = Slot { data, d2: f32::MAX, ..Default::default() };
    }
}

/// `FUN_0075d800`: beyond 300 m of the last swap spot's car place, it is rebuilt.
fn reset_last_spot(s: &mut LogicState) {
    let Some(last) = s.car_swap.state.last_spot else { return };
    let Some(sl) = s.car_swap.state.slots.get(last) else { return };
    if dist2(sl.data.car_position, s.car_swap.inputs.player.car.centre) > RESET_D2 {
        reset_spot(s, last);
        s.car_swap.state.last_spot = None;
    }
}

/// `FUN_00759410`: the nearest standing car (its handle changes → the request).
fn nearest_standing(s: &mut LogicState) {
    let i = &s.car_swap.inputs;
    if !i.player.loaded {
        return;
    }
    let mut best: Option<(f32, u32)> = None;
    for (k, sl) in s.car_swap.state.slots.iter().enumerate() {
        let r = i.parked.get(k).copied().unwrap_or_default();
        let Some(h) = sl.handle.filter(|_| sl.state == 3 && r.alive) else { continue };
        let d2 = dist2(r.position, i.player.car.centre);
        if best.is_none_or(|b| d2 < b.0) {
            best = Some((d2, h));
        }
    }
    let handle = best.map(|b| b.1);
    if handle != s.car_swap.state.nearest {
        s.car_swap.state.nearest = handle;
        out(s, CarSwapOut::Nearest { handle });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot_box(id: u32, x: f32) -> TriggerBox {
        TriggerBox { id, centre: [x, 1.0, 0.0], size: [8.0, 2.0, 8.0], ..Default::default() }
    }

    fn world() -> LogicState {
        let mut s = LogicState::default();
        let i = &mut s.car_swap.inputs;
        i.spots = vec![SpotData {
            location: 500,
            location_box: Some(spot_box(500, 100.0)),
            vehicle: 42,
            car_trigger: 501,
            car_position: [100.0, 0.0, 3.0],
            car_direction: [0.0, 0.0, 1.0],
            second: Some((502, [103.0, 0.0, 3.0], [0.0, 0.0, 1.0])),
            colour: 9,
        }];
        i.profile = vec![SpotProfile { owned: true, store: false }];
        i.parked = vec![ParkedReport::default()];
        i.started = true;
        i.max_mph = 70.0;
        i.prompt_drive = 955_092;
        i.prompt_store = 2_094_166;
        i.swap_sequence = 995_872;
        i.player = SwapPlayer {
            has_vehicle: true,
            loaded: true,
            car: CarBox::level([0.0, 0.7, 0.0], [1.0, 0.0, 0.0], [1.0, 0.7, 2.3]),
            vehicle: 7,
            profile_car: 7,
            colour: 3,
            ..Default::default()
        };
        apply(&mut s, &Request::CarSwapEnter);
        s
    }

    fn outs(s: &mut LogicState) -> Vec<CarSwapOut> {
        std::mem::take(&mut s.requests)
            .into_iter()
            .filter_map(|r| if let Request::CarSwap(o) = r { Some(o) } else { None })
            .collect()
    }

    fn step(s: &mut LogicState) {
        s.game_modes.prompt = None;
        apply(s, &Request::CarSwapStep);
    }

    #[test]
    fn parked_car_stands_between_20_and_300_m() {
        let mut s = world();
        step(&mut s);
        let o = outs(&mut s);
        assert!(o.iter().any(|o| matches!(o, CarSwapOut::Spawn { slot: 0, vehicle: 42, colour: 9, .. })), "{o:?}");
        assert_eq!(s.car_swap.state.slots[0].state, 1);
        s.car_swap.inputs.parked[0] = ParkedReport { spawn_status: 3, handle: 77, ..Default::default() };
        step(&mut s);
        assert_eq!(s.car_swap.state.slots[0].state, 2);
        s.car_swap.inputs.parked[0].ready = true;
        step(&mut s);
        assert_eq!(s.car_swap.state.slots[0].state, 3);
        // Far away (> 300 m) it goes.
        s.car_swap.inputs.player.car.centre = [500.0, 0.7, 0.0];
        step(&mut s);
        assert_eq!(s.car_swap.state.slots[0].state, 0);
        assert!(outs(&mut s).contains(&CarSwapOut::Remove { slot: 0, handle: Some(77) }));
    }

    #[test]
    fn swap_runs_its_seven_phases() {
        let mut s = world();
        s.car_swap.inputs.parked[0] =
            ParkedReport { spawn_status: 3, handle: 77, ready: true, alive: true, ..Default::default() };
        for _ in 0..3 {
            step(&mut s);
        }
        assert_eq!(s.car_swap.state.slots[0].state, 3);
        // Into the location: the prompt.
        s.car_swap.inputs.player.car.centre = [100.0, 0.7, -2.0];
        step(&mut s);
        assert_eq!(s.game_modes.prompt, Some(955_092));
        s.car_swap.inputs.swap_pressed = true;
        step(&mut s);
        s.car_swap.inputs.swap_pressed = false;
        assert_eq!(s.car_swap.state.phase, 1);
        assert!(outs(&mut s).contains(&CarSwapOut::Begin { slot: 0, first_use: true }));
        step(&mut s);
        assert!(s.game_modes.swap_in_progress && s.game.input_blocks.masks[2] & (1 << 7) != 0);
        assert_eq!(s.car_swap.state.phase, 1, "waits for the movie");
        s.car_swap.inputs.covered = true;
        step(&mut s);
        step(&mut s);
        assert_eq!(s.car_swap.state.phase, 3);
        let o = outs(&mut s);
        assert!(o.contains(&CarSwapOut::Exchange {
            slot: 0,
            vehicle: 42,
            colour: 9,
            parked: Some(77),
            old_vehicle: 7
        }));
        assert_eq!(s.car_swap.state.slots[0].data.vehicle, 7, "the spot keeps the player's old car");
        step(&mut s);
        let o = outs(&mut s);
        let pose = ([103.0, 0.0, 3.0], [0.0, 0.0, 1.0]);
        assert!(o.contains(&CarSwapOut::Place {
            player_pose: ([100.0, 0.0, 3.0], [0.0, 0.0, 1.0]),
            parked: Some(77),
            parked_pose: pose
        }));
        step(&mut s);
        step(&mut s);
        assert_eq!(s.car_swap.state.phase, 6);
        s.car_swap.inputs.sequence_done = true;
        step(&mut s);
        let o = outs(&mut s);
        assert!(o.contains(&CarSwapOut::Found { trigger: 501 }) && o.contains(&CarSwapOut::Found { trigger: 500 }));
        assert!(!o.iter().any(|o| matches!(o, CarSwapOut::PursuitSwap { .. })), "no pursuit, no score");
        s.car_swap.inputs.vehicles_ready = true;
        step(&mut s);
        assert_eq!(s.car_swap.state.phase, 0);
        assert!(outs(&mut s).contains(&CarSwapOut::Finish { player_colour: 9, parked: Some(77), parked_colour: 3 }));
        assert_eq!(s.car_swap.state.slots[0].data.colour, 3);
        // The latch: no prompt until the player has left the spot.
        step(&mut s);
        assert!(s.car_swap.state.latch && s.game_modes.prompt.is_none());
        assert!(!s.game_modes.swap_in_progress && s.game.input_blocks.masks[2] & (1 << 7) == 0);
        s.car_swap.inputs.player.car.centre = [60.0, 0.7, 0.0];
        step(&mut s);
        assert!(!s.car_swap.state.latch);
    }
}
