//! The DLC lock (user decision 2026-11-17): the base game is owned, **every DLC pack is not** (the Steam depot
//! ships all packs, so the files cannot show what the player bought; the base game's ownership is the retail check,
//! `mw_gamedir::GameDir::ownership`). The rules are the original's, ported in `mw_logic::natives::entitlements`
//! and specified per system in docs/decomp.md "DLC and entitlements":
//! - **cars** (`FUN_0073c180`: selectable only when its pack is owned, `FUN_00647780`): `--drive`, the garage, a
//!   saved profile's car and car swaps refuse a DLC car (it stays listed); [`car_locked`];
//! - **jack spots** (`FUN_007615f0`): a DLC car still stands at its spot, but the prompt is `PromptStore` (2094166,
//!   "… TO GO TO STORE") and the swap button opens the store instead (no store offline: nothing happens);
//! - **events** (`FUN_006f3830` / `FUN_0074a600`): a DLC event stays listed and is not started;
//! - **mods** (`FUN_00678e60`): game slots 11 (drift tyres) and 13 (jump nitrous) need Terminal Velocity;
//! - **the airport** (Terminal Velocity): free drive shows the barrier chevron (`FUN_006e68d0`, `CheckPointsConfig`
//!   `e74de7e2`), within 50 m of it the prompt `GameLogicConfig` +0x58 (2211224) (`FUN_006cfd40`), and a car in the
//!   airport arena (zones 36–57) is put back on the free-roam start, which is what the original does
//!   (docs/probe.md "Limits": placing, teleporting or driving into it puts the car back; the code behind it was not
//!   found).
//!
//! Ownership is fixed for a run, so the state is process-wide ([`init`] in `run`, read anywhere, also by the loader
//! thread).

use std::collections::HashMap;
use std::sync::OnceLock;

use bevy::prelude::*;
use mw_assets::AssetStore;
use mw_assets::events::Chevron;
use mw_logic::natives::entitlements::Entitlements;

use crate::drive::{Car, Player};

/// `GameLogicConfig` +0x168 `PromptStore`: "PRESS … TO GO TO STORE" (jack spots of cars the profile lacks).
const PROMPT_STORE: u32 = 2_094_166;
/// `GameLogicConfig` +0x58 `edbc4945`: the prompt at the airport barrier.
const PROMPT_AIRPORT: u32 = 2_211_224;
/// `FUN_006cfd40`: the prompt shows within 50 m of a barrier's centre (`.rdata` 0x00fc4fa0 = 2500, squared).
const BARRIER_PROMPT_RANGE: f32 = 50.0;
/// The airport arena's zones (docs/probe.md "Limits", observed in the original).
const AIRPORT_ZONES: std::ops::RangeInclusive<u32> = 36..=57;

pub struct Dlc {
    pub entitlements: Entitlements,
    /// Pack id → its display name.
    pack_names: HashMap<u32, String>,
    store_prompt: String,
    airport_prompt: String,
    barriers: Vec<Chevron>,
    zones: Option<mw_formats::ZoneList>,
    /// The free-roam start (`GameLogicConfig.beab0bc2`): position, heading.
    start: Option<(Vec3, f32)>,
}

static DLC: OnceLock<Dlc> = OnceLock::new();

/// Read the packs, the cars' packs and what the lock shows; nothing is owned.
pub fn init(store: &AssetStore) {
    let entitlements = mw_logic::natives::entitlements::load(store).unwrap_or_else(|e| {
        warn!("DLC: no pack data ({e}); nothing is locked");
        Entitlements::default()
    });
    let lang = store.language(mw_assets::DEFAULT_LANGUAGE).ok();
    let text = |id: u32| lang.as_ref().and_then(|l| l.strings.get(&id).cloned());
    let pack_names = store
        .pack_data()
        .map(|p| p.game_packs.iter().filter_map(|g| Some((g.id, text(g.display_name)?))).collect())
        .unwrap_or_default();
    let key = store.keyboard_button_text("KEY_E").and_then(text).unwrap_or_else(|| "E".into());
    let prompt = |id: u32, fallback: &str| {
        mw_formats::ui::substitute(&text(id).unwrap_or_else(|| fallback.into()), |expr| {
            expr.starts_with("Controls.ButtonPrompt").then(|| key.clone())
        })
    };
    let start = store.start_config().ok().and_then(|c| c.default_start).map(|p| {
        let d = p.direction;
        (Vec3::from_array(p.position), d[0].atan2(d[2]))
    });
    let dlc = Dlc {
        entitlements,
        pack_names,
        store_prompt: prompt(PROMPT_STORE, "PRESS {Controls.ButtonPrompt[\"SWAP_CARS\"]} TO GO TO STORE"),
        airport_prompt: prompt(PROMPT_AIRPORT, "AIRPORT REQUIRES THE TERMINAL VELOCITY PACK"),
        barriers: store.route_markers().map(|m| m.barriers).unwrap_or_default(),
        zones: store.zone_list().ok(),
        start,
    };
    let locked = dlc.entitlements.game_packs.iter().filter(|p| p.release > 0 && !dlc.entitlements.pack_owned(p.id));
    info!("DLC: {} packs not owned (offline rule: no DLC is owned)", locked.count());
    let _ = DLC.set(dlc);
}

pub fn get() -> Option<&'static Dlc> {
    DLC.get()
}

/// Why a car can't be driven: its DLC pack's name, `None` when it can (an unknown car or no data: allowed).
pub fn car_locked(bundle: &str) -> Option<String> {
    let d = get()?;
    let id = crate::frontend::vehicle_id(bundle)?;
    let pack = d.entitlements.vehicle_pack(id);
    (d.entitlements.is_dlc_pack(pack) && !d.entitlements.pack_owned(pack)).then(|| d.pack_name(pack))
}

/// Why an event can't be started: its DLC pack's name, `None` when it can.
pub fn event_locked(event: &mw_assets::events::Event) -> Option<String> {
    let d = get()?;
    (!d.entitlements.pack_content_available(event.game_pack)).then(|| d.pack_name(event.game_pack))
}

/// A mod field (`mods_menu` selections are (field, category)) the lock refuses: game slots 11 and 13 without
/// Terminal Velocity.
pub fn mod_field_locked(field: usize) -> bool {
    let Some(d) = get() else { return false };
    mw_assets::mods::GAME_SLOT_FIELD
        .iter()
        .enumerate()
        .any(|(slot, f)| *f == Some(field) && !d.entitlements.mod_slot_available(slot as i32))
}

/// The jack spots' prompt for a car the player does not own.
pub fn store_prompt() -> Option<&'static str> {
    get().map(|d| d.store_prompt.as_str())
}

impl Dlc {
    fn pack_name(&self, pack: u32) -> String {
        self.pack_names.get(&pack).cloned().unwrap_or_else(|| format!("DLC pack {pack}"))
    }
}

pub struct DlcPlugin {
    /// Free roam is running (the barrier and the airport rule apply there).
    pub free_roam: bool,
}

impl Plugin for DlcPlugin {
    fn build(&self, app: &mut App) {
        if !self.free_roam {
            return;
        }
        app.add_systems(Startup, spawn_barriers).add_systems(Update, airport.after(crate::jackspots::swap));
    }
}

/// The barrier chevrons, drawn as the race markers are (`chevron.rs`), without the airport's pack.
fn spawn_barriers(
    mut commands: Commands,
    game: Res<crate::Game>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<crate::chevron::ChevronMaterial>>,
) {
    let Some(d) = get() else { return };
    if d.entitlements.airport_pack_owned() || d.barriers.is_empty() {
        return;
    }
    crate::chevron::spawn_markers(&mut commands, &game.store, &d.barriers, &mut meshes, &mut images, &mut materials);
    info!("DLC: airport barrier ({} chevron lines)", d.barriers.len());
}

/// The player car's pose and velocities.
type PlayerBody = (
    &'static mut Transform,
    &'static mut avian3d::prelude::LinearVelocity,
    &'static mut avian3d::prelude::AngularVelocity,
);

/// `FUN_006cfd40` offline: the prompt near the barrier; a car in the airport arena goes back to the free-roam start.
fn airport(
    mut cars: Query<PlayerBody, (With<Car>, With<Player>)>,
    mut hud: Option<ResMut<crate::hud::HudState>>,
    mut shown: Local<bool>,
) {
    let Some(d) = get() else { return };
    if d.entitlements.airport_pack_owned() {
        return;
    }
    let Ok((mut t, mut lin, mut ang)) = cars.single_mut() else { return };
    let p = t.translation;
    let near = d.barriers.iter().any(|c| {
        let (lo, hi) = c.points.iter().fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), q| {
            ([lo[0].min(q[0]), lo[1].min(q[2])], [hi[0].max(q[0]), hi[1].max(q[2])])
        });
        let centre = Vec2::new((lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5);
        centre.distance_squared(Vec2::new(p.x, p.z)) < BARRIER_PROMPT_RANGE * BARRIER_PROMPT_RANGE
    });
    if let Some(hud) = hud.as_mut() {
        // Only where no jack spot prompts (they run first).
        if near && hud.prompt.is_none() {
            hud.prompt = Some(d.airport_prompt.clone());
            *shown = true;
        } else if !near && *shown {
            if hud.prompt.as_deref() == Some(d.airport_prompt.as_str()) {
                hud.prompt = None;
            }
            *shown = false;
        }
    }
    let inside = d.zones.as_ref().and_then(|z| z.zone_at(p.x, p.z)).is_some_and(|z| AIRPORT_ZONES.contains(&z.id));
    if let (true, Some((start, heading))) = (inside, d.start) {
        info!("DLC: the airport needs the Terminal Velocity pack: back to the free-roam start");
        t.translation = start;
        t.rotation = Quat::from_rotation_y(heading);
        lin.0 = Vec3::ZERO;
        ang.0 = Vec3::ZERO;
    }
}
