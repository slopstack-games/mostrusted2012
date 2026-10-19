//! Control of the running original for clean captures (docs/probe.md "Capture helpers"): teleport, car swap,
//! scripted inputs tick by tick, fixed conditions, and a per-tick telemetry log.
//!
//! Two places where our code runs on the game's own threads:
//! - **The vehicle tick**: the race car's "Update Car" method (`FUN_00904da0`, decomp.md "Update loop") is reached
//!   through one vtable slot in `.rdata` (`0x01023760`). The probe swaps that slot for a thunk that calls
//!   [`on_tick`] with the vehicle and its time step and then jumps to the original method. Only the pointer
//!   changes (checked to hold `0x00904da0` first, restored on unload); no instruction of the game is rewritten.
//!   `FUN_0091c5c0` copies each car's control record into its control fields (`FUN_009aacc0`) before the world
//!   step runs the cars' updates, so values written to those fields here are the ones this tick uses.
//! - **The frame**: the free-drive behaviour "Car Swap Logic" (`HSMBehaviourCarSwapLogic`, decomp.md "Jack spots")
//!   is updated once per frame on the game-logic thread through its vtable slot `0x00fdb4d4` (`FUN_00761d70`, the
//!   swap's own state machine). The same kind of thunk runs [`on_frame`] there first: the commands that call game
//!   functions (car swap, repair, traffic off, the game's own placement) run where the jack spots call them.
//!   (The XInput poll is no use for this: it runs on the input thread, `FUN_0040f140`.)
//!
//! Commands come from `cmd.req` in the probe's output directory (written by `mwtool probe` / `tp` / `car` /
//! `conditions` / `capture --probe`); results go to `cmd.out`. A status snapshot is written to `status.txt`.

use super::*;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

/// The vtable slot holding the race car's per-tick update (`FUN_00904da0`), and what it must hold.
const TICK_SLOT: usize = 0x0102_3760;
const TICK_FN_UPDATE_CAR: usize = 0x0090_4da0;
/// The vtable slot of the "Car Swap Logic" behaviour's per-frame update (`FUN_00761d70`), and what it must hold.
const FRAME_SLOT: usize = 0x00fd_b4d4;
const FRAME_FN_CAR_SWAP_LOGIC: usize = 0x0076_1d70;
/// The game's placement with streaming (`FUN_00647e00`, thiscall on G + 0x25690: vehicle handle, position,
/// direction, speed in mph, 0, 0xf, 0, 0 as the trigger teleport `FUN_006e4770` calls it; `ret 0x20`; decomp.md
/// "Boot, loading screens": it waits until the target is resident, then places the car).
const FN_PLACE: usize = 0x0064_7e00;
const PLACE_MANAGER: usize = 0x25690;
/// `G` (decomp.md "Boot, loading screens"): the game object.
const GAME_PTR: usize = 0x0117_7868;
/// The swap of the player's car (`FUN_006c2020`, thiscall on the player: vehicle id, colour, rollout or 0),
/// as the jack spots' swap calls it (decomp.md "Jack spots").
const FN_SWAP_CAR: usize = 0x006c_2020;
/// Vehicle-manager message 0x16, the repair (`FUN_00844940`, stdcall: vehicle handle).
const FN_REPAIR: usize = 0x0084_4940;
/// Traffic message 0xf: density 0 and the traffic system restarts (`FUN_008462a0`, decomp.md "Traffic rules").
const FN_TRAFFIC_OFF: usize = 0x0084_62a0;

// Vehicle object offsets (decomp.md "Update loop", "Wheel placement"; research log 2026-10-10).
const V_BODY: usize = 0x1760;
const V_STEER: usize = 0x1bf0;
const V_THROTTLE: usize = 0x1bf4;
const V_BRAKE: usize = 0x1c00;
const V_NITROUS: usize = 0x1c20;
const V_HANDBRAKE: usize = 0x1c40;
const V_SPEED_MPH: usize = 0x1620;
const V_WHEEL0: usize = 0x870;
const WHEEL_STRIDE: usize = 0x1d0;
const W_RECORD: usize = 0x198;
const W_SHAPE: usize = 0x19c;
const SHAPE_RADIUS: usize = 0x40;
const W_MATRIX: usize = 0x60;
const R_CENTRE: usize = 0x00;
const R_VELOCITY: usize = 0x40;
const R_SPIN: usize = 0x2c;
const B_POS: usize = 0x30;
const B_VEL: usize = 0xb0;
const B_ANGVEL: usize = 0xc0;
const B_INV_INERTIA_WORLD: usize = 0x40;
const B_INV_INERTIA_LOCAL: usize = 0xe0;

pub static PLAYER_VEH: AtomicUsize = AtomicUsize::new(0);
static ORIG_TICK: AtomicUsize = AtomicUsize::new(0);
static ORIG_FRAME: AtomicUsize = AtomicUsize::new(0);
static HOOKED: AtomicBool = AtomicBool::new(false);
static FRAME_HOOKED: AtomicBool = AtomicBool::new(false);
static TICK_THREAD: AtomicU32 = AtomicU32::new(0);
static FRAME_THREAD: AtomicU32 = AtomicU32::new(0);
static FRAMES: AtomicU32 = AtomicU32::new(0);

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn VirtualProtect(addr: *mut c_void, len: usize, protect: u32, old: *mut u32) -> BOOL;
    fn GetCurrentThreadId() -> u32;
}
#[cfg(not(windows))]
unsafe fn VirtualProtect(_: *mut c_void, _: usize, _: u32, _: *mut u32) -> BOOL {
    0
}
#[cfg(not(windows))]
unsafe fn GetCurrentThreadId() -> u32 {
    0
}
const PAGE_READWRITE: u32 = 0x04;

// --- small memory helpers (addresses are validated by the caller or by `readable`) -----------------------

fn rf(a: usize) -> f32 {
    // SAFETY: callers pass addresses inside objects checked readable when they were resolved.
    unsafe { std::ptr::read_volatile(a as *const f32) }
}
fn wf(a: usize, v: f32) {
    // SAFETY: as `rf`; the fields written are plain data of live game objects.
    unsafe { std::ptr::write_volatile(a as *mut f32, v) }
}
fn ru(a: usize) -> u32 {
    // SAFETY: as `rf`.
    unsafe { std::ptr::read_volatile(a as *const u32) }
}
fn wu(a: usize, v: u32) {
    // SAFETY: as `wf`.
    unsafe { std::ptr::write_volatile(a as *mut u32, v) }
}
fn v3(a: usize) -> [f32; 3] {
    [rf(a), rf(a + 4), rf(a + 8)]
}
fn w3(a: usize, v: [f32; 3]) {
    wf(a, v[0]);
    wf(a + 4, v[1]);
    wf(a + 8, v[2]);
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

/// The player's vehicle through the `handling.watch` chain, every pointer checked.
fn resolve_player_vehicle() -> Option<usize> {
    let q = follow(IMAGE_BASE + 0xcedb04, &[0x4])?;
    let q = read_u32(q)? as usize;
    let index = read_u32(q + 0x55c48)? as usize;
    if index >= 16 * 8 {
        return None;
    }
    let table = read_u32(q + 0x53220)? as usize;
    let veh = read_u32(table + 4 + 8 * index)? as usize;
    (veh > 0x1_0000 && readable(veh, 0x3100)).then_some(veh)
}

/// The local player object and its vehicle handle (`FUN_00774650` / `FUN_006bf330`: players = [G+0x2f3dc],
/// player = players + 0x10 + [players+0xa6b4] × 0x7f0; handle at player +0xa0).
fn player_object() -> Option<(usize, u32)> {
    let g = read_u32(GAME_PTR)? as usize;
    let players = read_u32(g + 0x2f3dc)? as usize;
    let index = read_u32(players + 0xa6b4)? as usize;
    if index > 8 {
        return None;
    }
    let player = players + 0x10 + index * 0x7f0;
    Some((player, read_u32(player + 0xa0)?))
}

/// Body, wheel objects and wheel records of a vehicle, all checked readable.
struct CarParts {
    body: usize,
    wheels: [(usize, usize, f32); 4],
}

fn car_parts(veh: usize) -> Option<CarParts> {
    let body = read_u32(veh + V_BODY)? as usize;
    if !readable(body, 0x100) {
        return None;
    }
    let mut wheels = [(0, 0, 0.0); 4];
    for (i, w) in wheels.iter_mut().enumerate() {
        let wo = veh + V_WHEEL0 + i * WHEEL_STRIDE;
        let rec = read_u32(wo + W_RECORD)? as usize;
        let shape = read_u32(wo + W_SHAPE)? as usize;
        if !readable(rec, 0x80) || !readable(shape + SHAPE_RADIUS, 4) {
            return None;
        }
        *w = (wo, rec, rf(shape + SHAPE_RADIUS));
    }
    Some(CarParts { body, wheels })
}

/// Ground height under the car estimated from its wheel records: mean of (centre y − radius).
fn ground_under(parts: &CarParts) -> f32 {
    parts.wheels.iter().map(|&(_, rec, r)| rf(rec + R_CENTRE + 4) - r).sum::<f32>() / 4.0
}

// --- commands ------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Teleport {
    pos: [f32; 3],
    heading_deg: f32,
    speed: f32,
    /// `pos[1]` is the ground height (the body goes as high above it as it is now above the ground under its
    /// wheels) rather than the body's own height.
    ground: bool,
}

#[derive(Debug, Clone, Copy)]
enum Ty {
    F32,
    I32,
    U8,
}

#[derive(Debug, Clone, Copy)]
enum FrameCmd {
    Car(u32),
    Repair,
    TrafficOff,
    /// The game's placement: position (x, ground y, z), heading in degrees, speed in mph.
    Place([f32; 3], f32, f32),
}

struct Hold {
    addr: usize,
    ty: Ty,
    value: f64,
}

/// One column of the per-tick log.
struct Col {
    /// The address, for a column that does not depend on pointers that may change while logging.
    addr: Option<usize>,
    /// For a chain through the player's vehicle (`NFS13.exe+0xcedb04 0x4 0x53220 <slot> off ...`): the vehicle
    /// field's offset and the further pointer hops, followed every tick (contact lists move).
    via_vehicle: Option<(usize, Vec<usize>)>,
    ty: Ty,
    /// The field is one of the vehicle's control fields: logged as the value the previous tick used.
    input: bool,
}

struct Script {
    /// (t, steer, throttle, brake, handbrake, nitrous)
    rows: Vec<[f32; 6]>,
    start: Option<f64>,
    end: f64,
}

struct TickLog {
    path: String,
    cols: Vec<Col>,
    prev_inputs: Vec<f64>,
    lines: Vec<String>,
    start: Option<f64>,
    end: Option<f64>,
    veh: usize,
    ticks: u64,
}

type Reply = std::sync::Arc<Mutex<Option<String>>>;

#[derive(Default)]
struct State {
    tick: u64,
    sim_t: f64,
    teleport: Option<(Teleport, Reply)>,
    frame_cmds: VecDeque<(FrameCmd, Reply)>,
    holds: Vec<Hold>,
    script: Option<Script>,
    log: Option<TickLog>,
    last_inputs: [f32; 5],
    last_dt: f64,
    /// Hold the script and the log until the car is within 1.5 m (horizontally) of this point: a run that starts
    /// with a placement starts on the first tick after the car was put there.
    arm: Option<[f32; 3]>,
    /// A placement was requested since the last `arm` (the arm only fires after one).
    placed: bool,
    script_done: Option<String>,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.get_or_init(|| Mutex::new(State::default())).lock().unwrap_or_else(|e| e.into_inner())
}

// --- the vtable hook ------------------------------------------------------------------------------------

/// Thunk installed in the vtable slot: saves every register and the flags, calls `on_tick(this, dt)`, restores
/// them and continues in the original method with the untouched stack (thiscall: `this` in ECX, the time step
/// pointer as the first stack argument).
#[cfg(all(windows, target_arch = "x86"))]
#[unsafe(naked)]
unsafe extern "C" fn tick_thunk() {
    core::arch::naked_asm!(
        "pushad",
        "pushfd",
        "mov eax, [esp + 40]",
        "push eax",
        "push ecx",
        "call {hook}",
        "add esp, 8",
        "popfd",
        "popad",
        "jmp dword ptr [{orig}]",
        hook = sym on_tick,
        orig = sym ORIG_TICK,
    )
}

/// Thunk for the frame slot: nothing to pass on (`this` in ECX only).
#[cfg(all(windows, target_arch = "x86"))]
#[unsafe(naked)]
unsafe extern "C" fn frame_thunk() {
    core::arch::naked_asm!(
        "pushad",
        "pushfd",
        "call {hook}",
        "popfd",
        "popad",
        "jmp dword ptr [{orig}]",
        hook = sym on_frame,
        orig = sym ORIG_FRAME,
    )
}

fn swap_slot(slot: usize, new: usize) -> Result<usize, String> {
    let mut old = 0u32;
    // SAFETY: changes the protection of the page of the game's `.rdata` holding the slot, writes one pointer, and
    // puts the protection back.
    unsafe {
        if VirtualProtect(slot as *mut c_void, 4, PAGE_READWRITE, &mut old) == 0 {
            return Err("VirtualProtect failed".into());
        }
        let prev = std::ptr::read_volatile(slot as *const usize);
        std::ptr::write_volatile(slot as *mut usize, new);
        let mut tmp = 0u32;
        VirtualProtect(slot as *mut c_void, 4, old, &mut tmp);
        Ok(prev)
    }
}

/// Install a vtable hook once its slot holds the expected method (after the wrapper has unpacked the game).
#[allow(dead_code)]
fn install(slot: usize, expect: usize, thunk: usize, orig: &AtomicUsize, flag: &AtomicBool) -> Option<String> {
    if flag.load(Ordering::Acquire) {
        return None;
    }
    let cur = read_u32(slot)? as usize;
    if cur != expect {
        return None;
    }
    orig.store(cur, Ordering::Release);
    Some(match swap_slot(slot, thunk) {
        Ok(_) => {
            flag.store(true, Ordering::Release);
            format!("hook installed: slot {slot:#010x} {cur:#010x} -> thunk")
        }
        Err(e) => format!("hook NOT installed at {slot:#010x}: {e}"),
    })
}

pub fn install_hooks() -> Vec<String> {
    #[allow(unused_mut)]
    let mut out = Vec::new();
    #[cfg(all(windows, target_arch = "x86"))]
    {
        out.extend(install(TICK_SLOT, TICK_FN_UPDATE_CAR, tick_thunk as *const () as usize, &ORIG_TICK, &HOOKED));
        out.extend(install(
            FRAME_SLOT,
            FRAME_FN_CAR_SWAP_LOGIC,
            frame_thunk as *const () as usize,
            &ORIG_FRAME,
            &FRAME_HOOKED,
        ));
    }
    out
}

pub fn uninstall_hooks() {
    if HOOKED.swap(false, Ordering::AcqRel) {
        let _ = swap_slot(TICK_SLOT, ORIG_TICK.load(Ordering::Acquire));
    }
    if FRAME_HOOKED.swap(false, Ordering::AcqRel) {
        let _ = swap_slot(FRAME_SLOT, ORIG_FRAME.load(Ordering::Acquire));
    }
}

extern "C" fn on_tick(this: usize, dt: *const f32) {
    if this == 0 || this != PLAYER_VEH.load(Ordering::Acquire) {
        return;
    }
    TICK_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Relaxed);
    let dt = if dt.is_null() { 1.0 / 60.0 } else { rf(dt as usize) as f64 };
    let mut s = state();
    s.tick += 1;
    if let Some([x, z, mph]) = s.arm.filter(|_| s.placed) {
        let b = read_u32(this + V_BODY).map_or(0, |b| b as usize);
        let near = b != 0 && {
            let p = v3(b + B_POS);
            (p[0] - x).hypot(p[2] - z) < 1.5 && (mph < 0.0 || (rf(this + V_SPEED_MPH) - mph).abs() < 0.5)
        };
        if near {
            s.arm = None;
        }
    }
    let armed = s.arm.is_some();
    if !armed {
        tick_log(&mut s, this);
    }
    if let Some((tp, reply)) = s.teleport.take() {
        let msg = teleport(this, tp, s.tick);
        *reply.lock().unwrap_or_else(|e| e.into_inner()) = Some(msg);
    }
    for h in &s.holds {
        write_value(h.addr, h.ty, h.value);
    }
    let sim_t = s.sim_t;
    let mut finished = None;
    if let Some(script) = s.script.as_mut().filter(|_| !armed) {
        let start = *script.start.get_or_insert(sim_t);
        let t = (sim_t - start) as f32;
        if (t as f64) < script.end {
            let i = script.rows.partition_point(|r| r[0] <= t + 1e-4).saturating_sub(1);
            let r = script.rows[i];
            let inputs = [r[1], r[2], r[3], r[4], r[5]];
            apply_inputs(this, inputs);
            s.last_inputs = inputs;
        } else {
            finished = Some(format!("script done at tick {} (t {:.4} s)", s.tick, t));
        }
    }
    if let Some(msg) = finished {
        s.script = None;
        s.script_done = Some(msg);
    }
    if let Some(log) = s.log.as_mut() {
        log.prev_inputs = log
            .cols
            .iter()
            .map(|c| if c.input { col_addr(c, this).map_or(f64::NAN, |a| read_value(a, c.ty)) } else { 0.0 })
            .collect();
    }
    s.sim_t += dt;
    s.last_dt = dt;
}

/// Where a log column is this tick (pointer hops checked readable).
fn col_addr(c: &Col, veh: usize) -> Option<usize> {
    match &c.via_vehicle {
        Some((off, rest)) if rest.is_empty() => Some(veh + off),
        Some((off, rest)) => follow(veh + off, rest).filter(|&a| readable(a, 4)),
        None => c.addr,
    }
}

fn apply_inputs(veh: usize, [steer, throttle, brake, handbrake, nitrous]: [f32; 5]) {
    wf(veh + V_STEER, steer.clamp(-1.0, 1.0));
    wf(veh + V_THROTTLE, throttle.clamp(0.0, 1.0));
    for lane in 0..4 {
        wf(veh + V_BRAKE + 4 * lane, brake.clamp(0.0, 1.0));
        wf(veh + V_NITROUS + 4 * lane, nitrous.clamp(0.0, 1.0));
        wu(veh + V_HANDBRAKE + 4 * lane, if handbrake > 0.5 { u32::MAX } else { 0 });
    }
}

fn read_value(addr: usize, ty: Ty) -> f64 {
    match ty {
        Ty::F32 => rf(addr) as f64,
        Ty::I32 => ru(addr) as i32 as f64,
        // SAFETY: as `rf`.
        Ty::U8 => (unsafe { std::ptr::read_volatile(addr as *const u8) }) as f64,
    }
}

fn write_value(addr: usize, ty: Ty, v: f64) {
    match ty {
        Ty::F32 => wf(addr, v as f32),
        Ty::I32 => wu(addr, v as i32 as u32),
        // SAFETY: as `wf`.
        Ty::U8 => unsafe { std::ptr::write_volatile(addr as *mut u8, v as u8) },
    }
}

/// Log the state the previous tick left (with the inputs it used), before this tick changes anything.
fn tick_log(s: &mut State, veh: usize) {
    let sim_t = s.sim_t;
    let tick = s.tick;
    let last_dt = s.last_dt;
    let Some(log) = s.log.as_mut() else { return };
    if log.veh != veh {
        return;
    }
    let start = *log.start.get_or_insert(sim_t);
    let t = sim_t - start;
    if log.end.is_some_and(|e| t > e + 1e-6) {
        return;
    }
    let mut line = format!("{t:.5},{tick},{last_dt:.6}");
    let have_prev = log.prev_inputs.len() == log.cols.len();
    for (i, c) in log.cols.iter().enumerate() {
        let v = if c.input && have_prev {
            log.prev_inputs[i]
        } else {
            match col_addr(c, veh) {
                Some(a) => read_value(a, c.ty),
                None => f64::NAN,
            }
        };
        let _ = write!(line, ",{v}");
    }
    log.lines.push(line);
    log.ticks += 1;
}

/// Move the car: rows, position, velocity and angular velocity of the chassis body, and each wheel record's
/// centre, velocity and spin (and the wheel matrices), by the same rigid motion.
fn teleport(veh: usize, tp: Teleport, tick: u64) -> String {
    let Some(parts) = car_parts(veh) else { return "error: car parts not readable".into() };
    let b = parts.body;
    let old_rows = [v3(b), v3(b + 0x10), v3(b + 0x20)];
    let p_old = v3(b + B_POS);
    let det = dot(cross(old_rows[0], old_rows[1]), old_rows[2]);
    let h = tp.heading_deg.to_radians();
    let f = [h.sin(), 0.0, h.cos()];
    let u = [0.0, 1.0, 0.0];
    let r = cross(u, f);
    let r = if det < 0.0 { scale(r, -1.0) } else { r };
    let new_rows = [r, u, f];
    let ground = ground_under(&parts);
    let y = if tp.ground { tp.pos[1] + (p_old[1] - ground) } else { tp.pos[1] };
    let p_new = [tp.pos[0], y, tp.pos[2]];
    let rot = |v: [f32; 3]| {
        let l = [dot(v, old_rows[0]), dot(v, old_rows[1]), dot(v, old_rows[2])];
        add(add(scale(new_rows[0], l[0]), scale(new_rows[1], l[1])), scale(new_rows[2], l[2]))
    };
    let point = |x: [f32; 3]| add(p_new, rot(sub(x, p_old)));
    // World inverse inertia = Mᵀ diag(local) M, if the stored one has that form (checked on the old rows).
    let local = v3(b + B_INV_INERTIA_LOCAL);
    let world_of = |m: &[[f32; 3]; 3]| {
        let mut out = [[0.0f32; 3]; 3];
        for (i, row) in out.iter_mut().enumerate() {
            for (j, e) in row.iter_mut().enumerate() {
                *e = (0..3).map(|k| m[k][i] * local[k] * m[k][j]).sum();
            }
        }
        out
    };
    let stored = [v3(b + B_INV_INERTIA_WORLD), v3(b + B_INV_INERTIA_WORLD + 0x10), v3(b + B_INV_INERTIA_WORLD + 0x20)];
    let predicted = world_of(&old_rows);
    let mut err = 0.0f32;
    let mut mag = 0.0f32;
    for i in 0..3 {
        for j in 0..3 {
            err = err.max((stored[i][j] - predicted[i][j]).abs());
            mag = mag.max(stored[i][j].abs());
        }
    }
    let inertia_ok = mag > 0.0 && err <= 0.01 * mag;
    for (k, row) in new_rows.iter().enumerate() {
        w3(b + 0x10 * k, *row);
    }
    w3(b + B_POS, p_new);
    let v = scale(f, tp.speed);
    w3(b + B_VEL, v);
    w3(b + B_ANGVEL, [0.0; 3]);
    if inertia_ok {
        let w = world_of(&new_rows);
        for (k, row) in w.iter().enumerate() {
            w3(b + B_INV_INERTIA_WORLD + 0x10 * k, *row);
        }
    }
    for &(wo, rec, radius) in &parts.wheels {
        w3(rec + R_CENTRE, point(v3(rec + R_CENTRE)));
        w3(rec + R_VELOCITY, v);
        wf(rec + R_SPIN, if radius > 0.05 { tp.speed / radius } else { 0.0 });
        for k in 0..3 {
            let a = wo + W_MATRIX + 0x10 * k;
            w3(a, rot(v3(a)));
        }
        let a = wo + W_MATRIX + 0x30;
        w3(a, point(v3(a)));
    }
    format!(
        "ok teleported at tick {} from ({:.2}, {:.2}, {:.2}) to ({:.2}, {:.2}, {:.2}) heading {:.1} speed {:.2} m/s; \
         ground under wheels was {ground:.3}; world inverse inertia {} (max err {err:.3e} of {mag:.3e})",
        tick,
        p_old[0],
        p_old[1],
        p_old[2],
        p_new[0],
        p_new[1],
        p_new[2],
        tp.heading_deg,
        tp.speed,
        if inertia_ok { "rotated" } else { "left as is (not of the expected form)" }
    )
}

// --- the frame hook ---------------------------------------------------------------------------------------

/// Runs on the game-logic thread before the "Car Swap Logic" behaviour's update, once per frame in free drive.
extern "C" fn on_frame() {
    FRAME_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Relaxed);
    FRAMES.fetch_add(1, Ordering::Relaxed);
    let cmd = state().frame_cmds.pop_front();
    let Some((cmd, reply)) = cmd else { return };
    let place = matches!(cmd, FrameCmd::Place(..));
    let msg = run_frame_cmd(cmd);
    if place {
        state().placed = true;
    }
    *reply.lock().unwrap_or_else(|e| e.into_inner()) = Some(msg);
}

#[cfg(all(windows, target_arch = "x86"))]
fn run_frame_cmd(cmd: FrameCmd) -> String {
    let Some((player, handle)) = player_object() else { return "error: no player object".into() };
    match cmd {
        FrameCmd::Car(id) => {
            // SAFETY: the game's own swap (`FUN_006c2020`, `ret 0xc`), called as the jack spot's swap calls it,
            // on the game's thread, with a vehicle id from the vehicle list and colour 0 (the car's default).
            let f: extern "thiscall" fn(usize, u32, u32, usize) = unsafe { std::mem::transmute(FN_SWAP_CAR) };
            f(player, id, 0, 0);
            format!("ok car swap to {id} requested (player {player:#x}, old handle {handle:#x})")
        }
        FrameCmd::Repair => {
            // SAFETY: `FUN_00844940(handle)`, stdcall (`ret 4`), posts the vehicle manager's repair message.
            let f: extern "system" fn(u32) = unsafe { std::mem::transmute(FN_REPAIR) };
            f(handle);
            format!("ok repair posted for handle {handle:#x}")
        }
        FrameCmd::Place(pos, heading, mph) => {
            let Some(g) = read_u32(GAME_PTR) else { return "error: no game object".into() };
            let h = heading.to_radians();
            let p = [pos[0], pos[1], pos[2], 1.0f32];
            let d = [h.sin(), 0.0, h.cos(), 0.0f32];
            // SAFETY: `FUN_00647e00` called as `FUN_006e4770` calls it (thiscall, eight stack arguments, `ret 0x20`),
            // on the game-logic thread; the vectors outlive the call (it copies them).
            let f: extern "thiscall" fn(usize, u32, *const f32, *const f32, f32, u32, u32, u32, u32) -> u8 =
                unsafe { std::mem::transmute(FN_PLACE) };
            let r = f(g as usize + PLACE_MANAGER, handle, p.as_ptr(), d.as_ptr(), mph, 0, 0xf, 0, 0);
            format!("ok placement requested (handle {handle:#x}, returned {r})")
        }
        FrameCmd::TrafficOff => {
            // SAFETY: `FUN_008462a0()`, no arguments, plain `ret`: posts traffic message 0xf.
            let f: extern "C" fn() = unsafe { std::mem::transmute(FN_TRAFFIC_OFF) };
            f();
            "ok traffic off posted".into()
        }
    }
}

#[cfg(not(all(windows, target_arch = "x86")))]
fn run_frame_cmd(_: FrameCmd) -> String {
    "error: not the 32-bit Windows build".into()
}

// --- parsing and the command file ------------------------------------------------------------------------

fn unix_to_wine(p: &str) -> String {
    if p.starts_with('/') { format!("Z:{}", p.replace('/', "\\")) } else { p.to_string() }
}

fn parse_ty(s: &str) -> Option<Ty> {
    match s {
        "f32" => Some(Ty::F32),
        "i32" | "u32" => Some(Ty::I32),
        "u8" => Some(Ty::U8),
        _ => None,
    }
}

/// `base off off ...` in the watch convention, every pointer checked; returns (final address, the address the
/// last offset was added to).
fn resolve_chain(words: &[&str]) -> Option<(usize, usize)> {
    let nums: Option<Vec<usize>> = words.iter().map(|w| parse_addr(w)).collect();
    let nums = nums?;
    let (&base, offs) = nums.split_first()?;
    let mut addr = base;
    let mut parent = base;
    for off in offs {
        parent = read_u32(addr)? as usize;
        if parent < 0x1_0000 {
            return None;
        }
        addr = parent + off;
    }
    Some((addr, parent))
}

fn load_script(path: &str) -> Result<Script, String> {
    let text = std::fs::read_to_string(unix_to_wine(path)).map_err(|e| format!("{path}: {e}"))?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#'));
    let header: Vec<String> = lines.next().unwrap_or("").split(',').map(|h| h.trim().to_lowercase()).collect();
    let col = |n: &str| header.iter().position(|h| h == n);
    let t = col("t").ok_or("inputs need a `t` column")?;
    let cols = ["steer", "throttle", "brake", "handbrake", "nitrous"].map(col);
    let mut rows = Vec::new();
    for l in lines {
        let f: Vec<f32> = l.split(',').map(|x| x.trim().parse().unwrap_or(0.0)).collect();
        let get = |c: Option<usize>| c.and_then(|c| f.get(c).copied()).unwrap_or(0.0);
        rows.push([get(Some(t)), get(cols[0]), get(cols[1]), get(cols[2]), get(cols[3]), get(cols[4])]);
    }
    if rows.is_empty() {
        return Err("no input rows".into());
    }
    rows.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let end = rows.last().map_or(0.0, |r| r[0]) as f64;
    Ok(Script { rows, start: None, end })
}

fn load_log(watch: &str, out: &str, veh: usize) -> Result<TickLog, String> {
    let text = std::fs::read_to_string(unix_to_wine(watch)).map_err(|e| format!("{watch}: {e}"))?;
    let mut names = Vec::new();
    let mut cols = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let w: Vec<&str> = line.split_whitespace().collect();
        if w.len() < 3 {
            continue;
        }
        let Some(ty) = parse_ty(w[1]) else { continue };
        let nums: Option<Vec<usize>> = w[2..].iter().map(|x| parse_addr(x)).collect();
        let Some(nums) = nums else { return Err(format!("{}: bad number", w[0])) };
        let via_vehicle = (nums.len() >= 5 && nums[0] == IMAGE_BASE + 0xcedb04 && nums[1] == 4 && nums[2] == 0x53220)
            .then(|| (nums[4], nums[5..].to_vec()));
        let addr =
            if via_vehicle.is_some() { None } else { resolve_chain(&w[2..]).map(|r| r.0).filter(|&a| readable(a, 4)) };
        let input = via_vehicle
            .as_ref()
            .is_some_and(|(off, rest)| rest.is_empty() && (V_STEER..V_HANDBRAKE + 0x30).contains(off));
        names.push(w[0].to_string());
        cols.push(Col { addr, via_vehicle, ty, input });
    }
    let header = format!("t,tick,dt,{}", names.join(","));
    Ok(TickLog {
        path: unix_to_wine(out),
        cols,
        prev_inputs: Vec::new(),
        lines: vec![header],
        start: None,
        end: None,
        veh,
        ticks: 0,
    })
}

fn wait_reply(reply: &Reply, secs: f32) -> String {
    let t0 = Instant::now();
    while t0.elapsed().as_secs_f32() < secs {
        if let Some(m) = reply.lock().unwrap_or_else(|e| e.into_inner()).take() {
            return m;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    "error: timed out (is the game paused, in a menu or loading?)".into()
}

/// One command line; returns its result line.
fn run_command(line: &str) -> String {
    let w: Vec<&str> = line.split_whitespace().collect();
    let num = |i: usize| w.get(i).and_then(|x| x.parse::<f32>().ok());
    match w.first().copied().unwrap_or("") {
        "tp" | "tpabs" => {
            let (Some(x), Some(y), Some(z), Some(h)) = (num(1), num(2), num(3), num(4)) else {
                return "error: tp x y z heading_deg [speed_m_s]".into();
            };
            if !HOOKED.load(Ordering::Acquire) {
                return "error: the tick hook is not installed".into();
            }
            let tp = Teleport { pos: [x, y, z], heading_deg: h, speed: num(5).unwrap_or(0.0), ground: w[0] == "tp" };
            let reply: Reply = Default::default();
            state().teleport = Some((tp, reply.clone()));
            wait_reply(&reply, 10.0)
        }
        "car" | "repair" | "notraffic" | "place" => {
            if !FRAME_HOOKED.load(Ordering::Acquire) {
                return "error: the frame hook is not installed (it runs in free drive only)".into();
            }
            let cmd = match w[0] {
                "place" => match (num(1), num(2), num(3), num(4)) {
                    (Some(x), Some(y), Some(z), Some(h)) => FrameCmd::Place([x, y, z], h, num(5).unwrap_or(0.0)),
                    _ => return "error: place x y z heading_deg [mph]".into(),
                },
                "car" => match w.get(1).and_then(|x| x.parse().ok()) {
                    Some(id) => FrameCmd::Car(id),
                    None => return "error: car <vehicle id>".into(),
                },
                "repair" => FrameCmd::Repair,
                _ => FrameCmd::TrafficOff,
            };
            let reply: Reply = Default::default();
            state().frame_cmds.push_back((cmd, reply.clone()));
            wait_reply(&reply, 10.0)
        }
        "poke" | "hold" => {
            // poke <type> <value> <base> [off ...]
            let (Some(ty), Some(value)) = (w.get(1).and_then(|t| parse_ty(t)), w.get(2).and_then(|v| v.parse().ok()))
            else {
                return "error: poke|hold <f32|i32|u8> <value> <base> [offset ...]".into();
            };
            let Some((addr, _)) = resolve_chain(&w[3..]) else { return "error: chain did not resolve".into() };
            if !readable(addr, 4) {
                return format!("error: {addr:#x} not readable");
            }
            let was = read_value(addr, ty);
            write_value(addr, ty, value);
            if w[0] == "hold" {
                let mut s = state();
                s.holds.retain(|h| h.addr != addr);
                s.holds.push(Hold { addr, ty, value });
            }
            format!("ok {} {addr:#010x} = {value} (was {was})", w[0])
        }
        "arm" => match (num(1), num(2)) {
            (Some(x), Some(z)) => {
                let mut s = state();
                s.arm = Some([x, z, num(3).unwrap_or(-1.0)]);
                s.placed = false;
                format!("ok the next script / log waits for the car at ({x}, {z})")
            }
            _ => "error: arm x z [mph]".into(),
        },
        "release" => {
            let n = std::mem::take(&mut state().holds).len();
            format!("ok released {n} hold(s)")
        }
        "peek" => {
            let Some(ty) = w.get(1).and_then(|t| parse_ty(t)) else {
                return "error: peek <type> <base> [off ...]".into();
            };
            match resolve_chain(&w[2..]) {
                Some((addr, _)) if readable(addr, 4) => format!("ok {addr:#010x} = {}", read_value(addr, ty)),
                _ => "error: not readable".into(),
            }
        }
        "script" | "log" => {
            // script <inputs.csv> [<watch> <out.csv>]   |   log <watch> <out.csv> [seconds]
            let veh = PLAYER_VEH.load(Ordering::Acquire);
            if veh == 0 {
                return "error: no player vehicle".into();
            }
            let (script, watch, out) = if w[0] == "script" {
                let Some(p) = w.get(1) else { return "error: script <inputs.csv> [<watch> <out.csv>]".into() };
                match load_script(p) {
                    Ok(s) => (Some(s), w.get(2), w.get(3)),
                    Err(e) => return format!("error: {e}"),
                }
            } else {
                (None, w.get(1), w.get(2))
            };
            let log = match (watch, out) {
                (Some(watch), Some(out)) => match load_log(watch, out, veh) {
                    Ok(mut l) => {
                        l.end = script.as_ref().map(|s| s.end).or_else(|| num(3).map(|x| x as f64));
                        Some(l)
                    }
                    Err(e) => return format!("error: {e}"),
                },
                _ => None,
            };
            let mut s = state();
            let msg = format!(
                "ok {}{} starts at the next tick ({})",
                script.as_ref().map_or(String::new(), |sc| format!(
                    "script of {} rows, {:.3} s",
                    sc.rows.len(),
                    sc.end
                )),
                log.as_ref().map_or(String::new(), |l| format!(", logging {} columns", l.cols.len())),
                s.tick
            );
            s.script_done = None;
            s.script = script;
            if let Some(old) = s.log.take() {
                flush_log(old, true);
            }
            s.log = log;
            msg
        }
        "stop" => {
            let mut s = state();
            s.script = None;
            if let Some(l) = s.log.take() {
                flush_log(l, true);
            }
            "ok stopped".into()
        }
        "status" => status_text(),
        "" => String::new(),
        other => format!("error: unknown command {other}"),
    }
}

fn flush_log(mut log: TickLog, last: bool) {
    let mut text = std::mem::take(&mut log.lines).join("\n");
    text.push('\n');
    let _ = std::fs::write(&log.path, text);
    if last {
        let _ = std::fs::write(format!("{}.done", log.path), format!("{} ticks\n", log.ticks));
    }
}

fn status_text() -> String {
    let veh = PLAYER_VEH.load(Ordering::Acquire);
    let s = state();
    let mut out = format!(
        "vehicle {veh:#x}\ntick_hook {}\nframe_hook {}\ntick {}\nsim_t {:.4}\nframes {}\ntick_thread {}\nframe_thread {}\n",
        HOOKED.load(Ordering::Acquire),
        FRAME_HOOKED.load(Ordering::Acquire),
        s.tick,
        s.sim_t,
        FRAMES.load(Ordering::Relaxed),
        TICK_THREAD.load(Ordering::Relaxed),
        FRAME_THREAD.load(Ordering::Relaxed)
    );
    if let Some(p) = (veh != 0).then(|| car_parts(veh)).flatten() {
        let b = p.body;
        let pos = v3(b + B_POS);
        let f = v3(b + 0x20);
        let v = v3(b + B_VEL);
        let _ = write!(
            out,
            "pos {:.3} {:.3} {:.3}\nfwd {:.4} {:.4} {:.4}\nheading_deg {:.2}\nvel {:.3} {:.3} {:.3}\nspeed_mph {:.2}\nground {:.3}\n",
            pos[0],
            pos[1],
            pos[2],
            f[0],
            f[1],
            f[2],
            f[0].atan2(f[2]).to_degrees(),
            v[0],
            v[1],
            v[2],
            rf(veh + V_SPEED_MPH),
            ground_under(&p)
        );
        for (i, &(_, rec, r)) in p.wheels.iter().enumerate() {
            let c = v3(rec + R_CENTRE);
            let _ = writeln!(out, "wheel{i} {:.3} {:.3} {:.3} r {r:.3} spin {:.2}", c[0], c[1], c[2], rf(rec + R_SPIN));
        }
    }
    if let Some((player, handle)) = player_object() {
        let _ = writeln!(out, "player {player:#x} handle {handle:#x}");
    }
    let _ = writeln!(
        out,
        "script {}\nlog {}\nholds {}",
        s.script.as_ref().map_or("none".to_string(), |sc| format!("running ({} rows, {:.3} s)", sc.rows.len(), sc.end)),
        s.log.as_ref().map_or("none".to_string(), |l| format!("{} rows", l.ticks)),
        s.holds.len()
    );
    if let Some(d) = &s.script_done {
        let _ = writeln!(out, "script_done {d}");
    }
    out
}

/// Called every ~50 ms from the probe's own thread.
pub fn service(dir: &str) {
    if let Some(veh) = resolve_player_vehicle() {
        PLAYER_VEH.store(veh, Ordering::Release);
    } else {
        PLAYER_VEH.store(0, Ordering::Release);
    }
    for msg in install_hooks() {
        log(&msg);
    }
    // A finished script ends its log too.
    {
        let mut s = state();
        let done = s.script.is_none() && s.script_done.is_some();
        let log_over =
            s.log.as_ref().is_some_and(|l| l.end.is_some_and(|e| s.sim_t - l.start.unwrap_or(s.sim_t) > e + 0.05));
        if ((done && s.log.as_ref().is_some_and(|l| l.end.is_some())) || log_over)
            && let Some(l) = s.log.take()
        {
            let n = l.ticks;
            flush_log(l, true);
            drop(s);
            log(&format!("log finished: {n} rows"));
        }
    }
    let req = format!("{dir}\\cmd.req");
    let Ok(text) = std::fs::read_to_string(&req) else { return };
    let _ = std::fs::remove_file(&req);
    let mut lines = text.lines();
    let id = lines.next().unwrap_or("").trim().to_string();
    let mut out = format!("{id}\n");
    for line in lines.map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let r = run_command(line);
        log(&format!("cmd `{line}`: {}", r.lines().next().unwrap_or("")));
        let _ = writeln!(out, "{r}");
    }
    out.push_str("end\n");
    let tmp = format!("{dir}\\cmd.out.tmp");
    if std::fs::write(&tmp, &out).is_ok() {
        let _ = std::fs::rename(&tmp, format!("{dir}\\cmd.out"));
    }
}

pub fn write_status(dir: &str) {
    let tmp = format!("{dir}\\status.tmp");
    if std::fs::write(&tmp, status_text()).is_ok() {
        let _ = std::fs::rename(&tmp, format!("{dir}\\status.txt"));
    }
}

/// Flush a running log every so often so a crash leaves most of it on disk.
pub fn flush_running() {
    let mut s = state();
    if let Some(l) = s.log.as_mut()
        && l.lines.len() > 1
    {
        let mut text = l.lines.join("\n");
        text.push('\n');
        let _ = std::fs::write(&l.path, text);
    }
}
