//! `mwtool tune`: a live tuning menu for the running original, in the terminal next to the game. It lists
//! handling values by page, shows what the game holds right now, and changes them with a keypress so the effect
//! can be felt (or captured) at once. Every change is a data poke through [`memscan::Pokes`]: only writable,
//! non-executable mappings, the original remembered, everything restored on quit (docs/probe.md "Tier 0",
//! docs/reverse-engineering.md). Nothing here touches code.
//!
//! Two kinds of field (docs/probe.md "Tuning menu"):
//! - **global**: a `.bss` physics constant at `NFS13.exe+rva`, shared by every car; a poke persists across car
//!   changes. Each one is a constant our own handling code already cites with its address.
//! - **car**: a field of the player's vehicle object, reached through the vehicle table as in `handling.watch`:
//!   `q = [[NFS13.exe+0xcedb04]+4]`, `index = [q+0x55c48]`, `vehicle = [[q+0x53220] + 4 + 8·index]`. The chain is
//!   re-resolved every refresh; when the vehicle object changes (the player picked another car in the game), the
//!   old car's records are dropped without writing (its memory may be reused) and the set values are poked into
//!   the new car. The car itself can only be changed through the game: no memory write respawns a vehicle.
//!
//! Per-car GeneSys data (grip curves, gear ratios, drift behaviour) has no known pointer from the vehicle object
//! yet, so it is not on these pages; `mwtool handling <car>` prints it.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Attribute, Print, SetAttribute};
use crossterm::terminal::{self, ClearType};
use crossterm::{cursor, execute, queue};

use crate::memscan::{Pokes, Process, Ty};

/// `NFS13.exe` has no ASLR; used if the module cannot be found in the maps.
const IMAGE_BASE: u64 = 0x40_0000;
const VEHICLE_ROOT: u64 = 0xced_b04;
const TABLE_OFF: u64 = 0x53220;
const INDEX_OFF: u64 = 0x55c48;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Loc {
    /// `.bss` constant at `NFS13.exe + rva`.
    Global(u64),
    /// Offset on the player's vehicle object.
    Car(u64),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Safe to change: a wrong value changes the feel, nothing else.
    Tune,
    /// Writable, but recomputed by the game or of unknown effect; shown with a marker.
    Experiment,
    /// Read-only state, shown so a change's effect is visible.
    Watch,
}

pub struct Field {
    /// Stable name used in preset files.
    pub key: &'static str,
    pub label: &'static str,
    pub loc: Loc,
    pub ty: Ty,
    pub kind: Kind,
    pub step: f64,
    /// Where the address and meaning come from.
    pub source: &'static str,
}

pub struct Page {
    pub title: &'static str,
    pub note: &'static str,
    pub fields: &'static [Field],
}

macro_rules! field {
    ($key:literal, $label:literal, $loc:expr, $ty:ident, $kind:ident, $step:expr, $source:literal) => {
        Field { key: $key, label: $label, loc: $loc, ty: Ty::$ty, kind: Kind::$kind, step: $step, source: $source }
    };
}

use Loc::{Car, Global};

pub static PAGES: &[Page] = &[
    Page {
        title: "Grip",
        note: "The absolute grip scale has no single constant (it is N x the multipliers). Per-axle scales and \
               curves are per-car GeneSys data: `mwtool handling <car>`. The multipliers' writer is unknown: \
               hold (h) a value if it does not stick.",
        fields: &[
            field!("w0_grip", "wheel 0 grip mult", Car(0x9b0), F32, Experiment, 0.05, "handling.watch w0_grip0"),
            field!("w1_grip", "wheel 1 grip mult", Car(0xb80), F32, Experiment, 0.05, "handling.watch w1_grip0"),
            field!("w2_grip", "wheel 2 grip mult", Car(0xd50), F32, Experiment, 0.05, "handling.watch w2_grip0"),
            field!("w3_grip", "wheel 3 grip mult", Car(0xf20), F32, Experiment, 0.05, "handling.watch w3_grip0"),
            field!("w0_n", "wheel 0 load N", Car(0x970), F32, Watch, 0.0, "handling.watch w0_N"),
            field!("w1_n", "wheel 1 load N", Car(0xb40), F32, Watch, 0.0, "handling.watch w1_N"),
            field!("w2_n", "wheel 2 load N", Car(0xd10), F32, Watch, 0.0, "handling.watch w2_N"),
            field!("w3_n", "wheel 3 load N", Car(0xee0), F32, Watch, 0.0, "handling.watch w3_N"),
        ],
    },
    Page {
        title: "Drag/Aero",
        note: "Drag and lift coefficients and downforce are per-car GeneSys data (`mwtool handling <car>`).",
        fields: &[
            field!("drag_k", "drag scale K", Global(0xd872a0), F32, Tune, 0.05, "chassis.rs DRAG_K (F=-1/2 K C|v|v)"),
            field!("arb_scale", "anti-roll scale", Global(0xd85d70), F32, Tune, 5.0, "chassis.rs ARB_PER_DEGREE"),
            field!("gravity", "gravity g", Global(0xd87330), F32, Experiment, 0.25, "consts.txt; affects all physics"),
            field!("speed", "speed mph", Car(0x1620), F32, Watch, 0.0, "handling.watch speed_mph"),
        ],
    },
    Page {
        title: "Drift/HB",
        note: "Drift entry angles, yaw torques and handbrake strengths are per-car GeneSys data \
               (`mwtool genesys <bundle> --type DriftBehaviour`). The handbrake lock condition is not located.",
        fields: &[
            field!("side_cap", "drift side-force cap", Global(0xd87380), F32, Tune, 0.25, "drift.rs (game cap 5)"),
            field!("burnout_mph", "burnout speed scale", Global(0xd866f0), F32, Experiment, 1.0, "consts.txt, dump"),
            field!("drift_thr", "drift threshold", Car(0x2c10), F32, Watch, 0.0, "handling.watch; FLT_MAX off-drift"),
            field!("drift_scale", "drift scale", Car(0x2b20), F32, Watch, 0.0, "handling.watch"),
            field!("drifting", "drifting", Car(0x2ac4), U8, Watch, 0.0, "handling.watch"),
            field!("drift_yaw", "drift yaw torque", Car(0x2ad0), F32, Watch, 0.0, "handling.watch"),
            field!("handbrake", "handbrake", Car(0x2a40), U8, Watch, 0.0, "handling.watch handbrake_a"),
        ],
    },
    Page {
        title: "Drivetrain",
        note: "Gear ratios, shift rpm / speeds, idle and max rpm are per-car GeneSys data (`mwtool handling <car>`).",
        fields: &[
            field!("clutch_cap", "clutch cap N m", Global(0xd86f70), F32, Tune, 250.0, "drivetrain.rs CLUTCH_CAP"),
            field!("shift_declutch", "shift: declutch frac", Global(0xd86ee0), F32, Tune, 0.01, "drivetrain.rs"),
            field!("shift_neutral", "shift: neutral frac", Global(0xd86fd0), F32, Tune, 0.05, "drivetrain.rs"),
            field!("down_hyst", "downshift hyst mph", Global(0xd872e0), F32, Tune, 0.5, "drivetrain.rs"),
            field!("idle_rpm", "idle governor rpm", Global(0xd86f30), F32, Tune, 5.0, "drivetrain.rs"),
            field!("idle_gain", "idle governor gain", Global(0xd871c0), F32, Tune, 0.01, "drivetrain.rs"),
            field!("idle_thr", "idle governor max thr", Global(0xd873f0), F32, Tune, 0.05, "drivetrain.rs"),
            field!("rpm", "rpm", Car(0x1d10), F32, Watch, 0.0, "handling.watch"),
            field!("gear", "gear", Car(0x1d70), I32, Watch, 0.0, "handling.watch"),
            field!("clutch", "clutch", Car(0x1e20), F32, Watch, 0.0, "handling.watch"),
            field!("load", "engine load", Car(0x1f20), F32, Watch, 0.0, "handling.watch"),
        ],
    },
];

fn find_field(key: &str) -> Option<&'static Field> {
    PAGES.iter().flat_map(|p| p.fields).find(|f| f.key == key)
}

/// The player's vehicle as found through the table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CarRef {
    pub index: u32,
    pub object: u64,
}

fn plausible_ptr(v: u32) -> bool {
    (0x1_0000..0xffff_0000).contains(&v) && v != 0xcdcd_cdcd
}

/// Follow the `handling.watch` chain to the player's vehicle; `None` outside gameplay (menus, loading).
pub fn resolve_car(read_u32: impl Fn(u64) -> Option<u32>, exe: u64) -> Option<CarRef> {
    let root = read_u32(exe + VEHICLE_ROOT).filter(|&v| plausible_ptr(v))?;
    let q = read_u32(root as u64 + 4).filter(|&v| plausible_ptr(v))? as u64;
    let index = read_u32(q + INDEX_OFF).filter(|&i| i < 256)?;
    let table = read_u32(q + TABLE_OFF).filter(|&v| plausible_ptr(v))? as u64;
    let object = read_u32(table + 4 + 8 * index as u64).filter(|&v| plausible_ptr(v))? as u64;
    Some(CarRef { index, object })
}

struct Tuner<'a> {
    p: &'a Process,
    exe: u64,
    pokes: Pokes,
    /// Values set from the menu, re-applied to a new car and (if held) every refresh.
    set: Vec<(&'static Field, f64, bool)>,
    car: Option<CarRef>,
    page: usize,
    row: usize,
    status: String,
    input: Option<String>,
    preset: PathBuf,
}

impl<'a> Tuner<'a> {
    fn new(p: &'a Process, preset: PathBuf) -> Self {
        let exe = p.module_base("NFS13.exe").unwrap_or(IMAGE_BASE);
        Tuner {
            p,
            exe,
            pokes: Pokes::default(),
            set: Vec::new(),
            car: None,
            page: 0,
            row: 0,
            status: String::new(),
            input: None,
            preset,
        }
    }

    fn addr(&self, f: &Field) -> Option<u64> {
        match f.loc {
            Global(rva) => Some(self.exe + rva),
            Car(off) => self.car.map(|c| c.object + off),
        }
    }

    fn read(&self, f: &Field) -> Option<f64> {
        self.p.read_value(f.ty, self.addr(f)?)
    }

    fn field(&self) -> &'static Field {
        let fields = PAGES[self.page].fields;
        &fields[self.row.min(fields.len() - 1)]
    }

    /// Re-resolve the car; on a change, drop the old car's records and poke the set values into the new one.
    fn refresh(&mut self) {
        let car = resolve_car(|a| self.p.read_u32(a), self.exe);
        if car != self.car {
            if let Some(old) = self.car {
                for (f, _, _) in &self.set {
                    if let Car(off) = f.loc {
                        self.pokes.forget(old.object + off);
                    }
                }
            }
            self.car = car;
            if let Some(c) = car {
                let n = self.apply_set(|f| matches!(f.loc, Car(_)));
                self.status = format!("car: index {} object {:#010x}; re-applied {n} value(s)", c.index, c.object);
            }
        }
        // Held values: put them back if the game changed them.
        let held: Vec<(&Field, f64)> = self.set.iter().filter(|s| s.2).map(|s| (s.0, s.1)).collect();
        for (f, v) in held {
            if self.read(f).is_some_and(|cur| (cur - v).abs() > 1e-6 * v.abs().max(1.0)) {
                let _ = self.poke_raw(f, v);
            }
        }
    }

    fn apply_set(&mut self, which: impl Fn(&Field) -> bool) -> usize {
        let todo: Vec<(&Field, f64)> = self.set.iter().filter(|s| which(s.0)).map(|s| (s.0, s.1)).collect();
        todo.into_iter().filter(|&(f, v)| self.poke_raw(f, v).is_ok()).count()
    }

    fn poke_raw(&mut self, f: &Field, value: f64) -> Result<f64> {
        let Some(addr) = self.addr(f) else { bail!("{}: no car (not in gameplay?)", f.label) };
        self.pokes.poke(self.p, addr, f.ty, value)
    }

    fn set_value(&mut self, f: &'static Field, value: f64) {
        if f.kind == Kind::Watch {
            self.status = format!("{} is read-only", f.label);
            return;
        }
        match self.poke_raw(f, value) {
            Ok(before) => {
                match self.set.iter_mut().find(|s| s.0.key == f.key) {
                    Some(s) => s.1 = value,
                    None => self.set.push((f, value, false)),
                }
                self.status = format!("{}: {before} -> {value}", f.label);
            }
            Err(e) => self.status = format!("{e:#}"),
        }
    }

    fn nudge(&mut self, mul: f64) {
        let f = self.field();
        match self.read(f) {
            Some(cur) => {
                let v = cur + f.step * mul;
                // Round away float noise so repeated steps land on clean values.
                let q = (f.step * mul.abs().min(1.0)).abs().max(1e-6) / 100.0;
                self.set_value(f, if f.ty == Ty::F32 { (v / q).round() * q } else { v.round() });
            }
            None => self.status = format!("{}: cannot read", f.label),
        }
    }

    fn restore_field(&mut self) {
        let f = self.field();
        self.set.retain(|s| s.0.key != f.key);
        match self.addr(f) {
            Some(a) if self.pokes.restore(self.p, Some(a)) > 0 => self.status = format!("{} restored", f.label),
            _ => self.status = format!("{} was not changed", f.label),
        }
    }

    fn restore_all(&mut self) -> usize {
        self.set.clear();
        self.pokes.restore(self.p, None)
    }

    fn toggle_hold(&mut self) {
        let f = self.field();
        if f.kind == Kind::Watch {
            self.status = format!("{} is read-only", f.label);
            return;
        }
        if !self.set.iter().any(|s| s.0.key == f.key) {
            match self.read(f) {
                Some(v) => self.set.push((f, v, false)),
                None => return,
            }
        }
        if let Some(s) = self.set.iter_mut().find(|s| s.0.key == f.key) {
            s.2 = !s.2;
            self.status = format!("{}: {}", f.label, if s.2 { "held (re-applied every refresh)" } else { "not held" });
        }
    }

    fn save_preset(&mut self) -> Result<()> {
        if let Some(dir) = self.preset.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut s = String::from("# mwtool tune preset: key = value [hold]\n");
        for (f, v, hold) in &self.set {
            s += &format!("{} = {v}{}\n", f.key, if *hold { " hold" } else { "" });
        }
        std::fs::write(&self.preset, s).with_context(|| format!("writing {}", self.preset.display()))?;
        self.status = format!("saved {} value(s) to {}", self.set.len(), self.preset.display());
        Ok(())
    }

    fn load_preset(&mut self) -> Result<()> {
        let entries = parse_preset(&std::fs::read_to_string(&self.preset)?)?;
        let n = entries.len();
        for (f, v, hold) in entries {
            self.set_value(f, v);
            if let Some(s) = self.set.iter_mut().find(|s| s.0.key == f.key) {
                s.2 = hold;
            }
        }
        self.status = format!("loaded {n} value(s) from {}", self.preset.display());
        Ok(())
    }

    fn key(&mut self, k: KeyEvent) -> bool {
        if let Some(buf) = &mut self.input {
            match k.code {
                KeyCode::Enter => {
                    let text = std::mem::take(buf);
                    self.input = None;
                    match text.trim().parse::<f64>() {
                        Ok(v) => self.set_value(self.field(), v),
                        Err(_) => self.status = format!("not a number: {text}"),
                    }
                }
                KeyCode::Esc => self.input = None,
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Char(c) if c.is_ascii_digit() || "-.eE+".contains(c) => buf.push(c),
                _ => {}
            }
            return true;
        }
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        let n_rows = PAGES[self.page].fields.len();
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => return false,
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return false,
            KeyCode::Tab => self.goto_page((self.page + 1) % PAGES.len()),
            KeyCode::BackTab => self.goto_page((self.page + PAGES.len() - 1) % PAGES.len()),
            KeyCode::Char(c @ '1'..='9') => self.goto_page(c as usize - '1' as usize),
            KeyCode::Up | KeyCode::Char('k') => self.row = (self.row + n_rows - 1) % n_rows,
            KeyCode::Down | KeyCode::Char('j') => self.row = (self.row + 1) % n_rows,
            KeyCode::Left => self.nudge(if shift { -10.0 } else { -1.0 }),
            KeyCode::Right => self.nudge(if shift { 10.0 } else { 1.0 }),
            KeyCode::PageDown => self.nudge(-10.0),
            KeyCode::PageUp => self.nudge(10.0),
            KeyCode::Char(',') => self.nudge(-0.1),
            KeyCode::Char('.') => self.nudge(0.1),
            KeyCode::Enter | KeyCode::Char('=') => self.input = Some(String::new()),
            KeyCode::Char('h') => self.toggle_hold(),
            KeyCode::Char('r') => self.restore_field(),
            KeyCode::Char('R') => {
                let n = self.restore_all();
                self.status = format!("restored {n} value(s)");
            }
            KeyCode::Char('s') => {
                if let Err(e) = self.save_preset() {
                    self.status = format!("{e:#}");
                }
            }
            KeyCode::Char('L') => {
                if let Err(e) = self.load_preset() {
                    self.status = format!("{e:#}");
                }
            }
            _ => {}
        }
        true
    }

    fn goto_page(&mut self, page: usize) {
        if page < PAGES.len() {
            self.page = page;
            self.row = 0;
        }
    }

    /// One line per field: marker, label, live value, original if changed, flags, address and source.
    fn row_text(&self, f: &Field) -> String {
        let addr = self.addr(f);
        let live = self.read(f).map_or("--".to_string(), fmt_value);
        let orig =
            addr.and_then(|a| self.pokes.original(a)).map_or(String::new(), |o| format!("(was {})", fmt_value(o)));
        let held = self.set.iter().any(|s| s.0.key == f.key && s.2);
        let flags = match f.kind {
            Kind::Tune => "",
            Kind::Experiment => "exp",
            Kind::Watch => "ro",
        };
        let place = match f.loc {
            Global(rva) => format!("NFS13.exe+{rva:#x}"),
            Car(off) => format!("car+{off:#x}"),
        };
        format!(
            "{:<24}{:>12} {:<14}{:<5}{:<4} {:<18} {}",
            f.label,
            live,
            orig,
            if held { "HOLD" } else { "" },
            flags,
            place,
            f.source
        )
    }

    fn draw(&self, out: &mut impl Write) -> Result<()> {
        let (w, h) = terminal::size().unwrap_or((120, 40));
        let w = w as usize;
        let mut lines: Vec<(String, bool)> = Vec::new();
        let car = match self.car {
            Some(c) => format!("car index {} at {:#010x}", c.index, c.object),
            None => "no car (menus / loading): car fields unavailable".into(),
        };
        lines.push((format!("mwtool tune  pid {}  {car}  {} poke(s)", self.p.pid, self.pokes.list().count()), true));
        let tabs: Vec<String> =
            PAGES
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    if i == self.page { format!("[{} {}]", i + 1, p.title) } else { format!(" {} {} ", i + 1, p.title) }
                })
                .collect();
        lines.push((tabs.join(" "), false));
        lines.push((String::new(), false));
        let page = &PAGES[self.page];
        for (i, f) in page.fields.iter().enumerate() {
            let sel = i == self.row;
            lines.push((format!("{} {}", if sel { ">" } else { " " }, self.row_text(f)), sel));
        }
        lines.push((String::new(), false));
        for chunk in wrap(page.note, w.saturating_sub(2).max(20)) {
            lines.push((chunk, false));
        }
        lines.push((String::new(), false));
        lines.push((
            "up/down select  left/right step (shift x10, , . x0.1)  enter type a value  h hold  r restore  \
             R restore all  s save / L load preset  tab page  q quit (restores everything)"
                .into(),
            false,
        ));
        match &self.input {
            Some(buf) => lines.push((format!("{} = {buf}_", self.field().label), true)),
            None => lines.push((self.status.clone(), false)),
        }
        queue!(out, cursor::MoveTo(0, 0))?;
        for (i, (text, bold)) in lines.iter().take(h as usize).enumerate() {
            let text: String = text.chars().take(w).collect();
            queue!(out, cursor::MoveTo(0, i as u16))?;
            if *bold {
                queue!(out, SetAttribute(Attribute::Reverse), Print(&text), SetAttribute(Attribute::Reset))?;
            } else {
                queue!(out, Print(&text))?;
            }
            queue!(out, terminal::Clear(ClearType::UntilNewLine))?;
        }
        queue!(out, terminal::Clear(ClearType::FromCursorDown))?;
        out.flush()?;
        Ok(())
    }
}

impl Drop for Tuner<'_> {
    fn drop(&mut self) {
        let n = self.restore_all();
        if n > 0 {
            eprintln!("restored {n} poked value(s)");
        }
    }
}

fn fmt_value(v: f64) -> String {
    if v.abs() >= 1e7 {
        format!("{v:.3e}")
    } else if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.4}")
    }
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for word in text.split_whitespace() {
        let last = lines.last_mut().unwrap();
        if !last.is_empty() && last.len() + 1 + word.len() > width {
            lines.push(word.to_string());
        } else {
            if !last.is_empty() {
                last.push(' ');
            }
            last.push_str(word);
        }
    }
    lines
}

/// Preset lines: `key = value` with an optional trailing `hold`; `#` starts a comment.
pub fn parse_preset(text: &str) -> Result<Vec<(&'static Field, f64, bool)>> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (key, rest) =
            line.split_once('=').with_context(|| format!("preset line {}: expected key = value", n + 1))?;
        let mut words = rest.split_whitespace();
        let value: f64 = words
            .next()
            .with_context(|| format!("preset line {}: no value", n + 1))?
            .parse()
            .with_context(|| format!("preset line {}: bad value", n + 1))?;
        let hold = words.next() == Some("hold");
        let f =
            find_field(key.trim()).with_context(|| format!("preset line {}: unknown field {}", n + 1, key.trim()))?;
        if f.kind == Kind::Watch {
            bail!("preset line {}: {} is read-only", n + 1, f.key);
        }
        out.push((f, value, hold));
    }
    Ok(out)
}

/// Print every field once (no terminal UI): for scripts, and to check the addresses resolve.
fn print_all(t: &Tuner) {
    match t.car {
        Some(c) => println!("car index {} at {:#010x}", c.index, c.object),
        None => println!("no car (menus / loading)"),
    }
    for page in PAGES {
        println!("[{}]", page.title);
        for f in page.fields {
            println!("  {:<16}{}", f.key, t.row_text(f));
        }
    }
}

pub fn run(p: &Process, preset: &Path, apply: bool, print: bool) -> Result<()> {
    let mut t = Tuner::new(p, preset.to_path_buf());
    t.refresh();
    if print {
        print_all(&t);
        return Ok(());
    }
    if apply {
        t.load_preset()?;
    }
    let mut out = std::io::stdout();
    terminal::enable_raw_mode()?;
    execute!(out, terminal::EnterAlternateScreen, cursor::Hide)?;
    let result = (|| -> Result<()> {
        let mut last = Instant::now() - Duration::from_secs(1);
        loop {
            if last.elapsed() >= Duration::from_millis(100) {
                t.refresh();
                last = Instant::now();
            }
            t.draw(&mut out)?;
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(k) = event::read()?
                && k.kind == KeyEventKind::Press
                && !t.key(k)
            {
                return Ok(());
            }
        }
    })();
    let _ = execute!(out, cursor::Show, terminal::LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn keys_unique_and_steps_set() {
        let mut seen = HashSet::new();
        for f in PAGES.iter().flat_map(|p| p.fields) {
            assert!(seen.insert(f.key), "duplicate key {}", f.key);
            assert!(f.kind == Kind::Watch || f.step > 0.0, "{} needs a step", f.key);
        }
    }

    #[test]
    fn car_chain_follows_handling_watch() {
        // [[exe+root]+4] = q; index at q+0x55c48; table at [q+0x53220]; vehicle at [table + 4 + 8*index].
        let exe = IMAGE_BASE;
        let mut mem = HashMap::new();
        mem.insert(exe + VEHICLE_ROOT, 0x0100_0000u32);
        mem.insert(0x0100_0004, 0x0200_0000);
        mem.insert(0x0200_0000 + INDEX_OFF, 48);
        mem.insert(0x0200_0000 + TABLE_OFF, 0x0300_0000);
        mem.insert(0x0300_0000 + 4 + 8 * 48, 0x0400_0000);
        let car = resolve_car(|a| mem.get(&a).copied(), exe);
        assert_eq!(car, Some(CarRef { index: 48, object: 0x0400_0000 }));
        // Uninitialised memory (0xCD fill) is not a car.
        mem.insert(0x0300_0000 + 4 + 8 * 48, 0xcdcd_cdcd);
        assert_eq!(resolve_car(|a| mem.get(&a).copied(), exe), None);
    }

    #[test]
    fn preset_round_trip() {
        let p = parse_preset("# c\ndrag_k = 1.3\nw0_grip = 0.9 hold\n").unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!((p[0].0.key, p[0].1, p[0].2), ("drag_k", 1.3, false));
        assert_eq!((p[1].0.key, p[1].1, p[1].2), ("w0_grip", 0.9, true));
        assert!(parse_preset("rpm = 3000").is_err()); // read-only
        assert!(parse_preset("nope = 1").is_err());
    }
}
