//! Body shops: the drive-through garages that repair and respray the car (docs/decomp.md "Body shops").
//!
//! They are the `TriggerData` boxes of kind 6 (22 in the base game). The HSM runs a "Do Body Shop" behaviour
//! (`DOBODYSHOP`, `FUN_0077b600`) in every driving game mode; its per-pass work is `FUN_00748ff0(this, player)`:
//!
//! - vehicle-manager message 0xc (`FUN_008446a0` → `FUN_00590cb0` → `FUN_0058bc60`): the car's colour becomes
//!   the **next entry of its palette**, cyclic (`VehicleColourPalette`: the `ColourGroups`' primary colours,
//!   or the plain paint list), not a random one; `FUN_0058edd0` then loads the paint's values. Offline only
//!   (the game-mode byte +0xe8 skips it);
//! - message 0x16 (`FUN_00844940` → `FUN_0058f060`): the repair, which zeroes the damage and, through physics
//!   message 0x12, the dust and dirt levels (decomp.md "Vehicle paint, liveries and dirt");
//! - `FUN_00658ad0(player, 100)` (`.rdata` 0x00fd2104): a full nitrous tank;
//! - stats 0x4a and 0x49 +1, the score events `ScoringConfig` +0x164 (955328) and +0x28 (955329), and the
//!   pursuit side (`FUN_00639b00`, the heat value at player +0x3d8 through `FUN_006b97e0`), not ported: there
//!   are no Speed Points or police here.
//!
//! Finding a shop is the collectable hit (`FUN_007059b0`, kind 6: the per-kind "found" bitset, language string
//! 1444768 "BODY SHOP FOUND").
//!
//! Model decisions (not from the game):
//! - **When a pass counts**: the behaviour's update (code between 0x0074921b and 0x00749570) is not in the
//!   decompiled set, so the rule here is the trigger system's usual one: once on entering the box (the car's
//!   origin inside it, as `freeroam.rs`), again after leaving it. A cooldown or a speed gate, if the game has
//!   one, is in that function.
//! - The handler resprays and repairs on every pass; the game's own tip (language string 1444769) says "repair
//!   your car, or spray it a different colour if it's undamaged". If that either / or exists it is decided in
//!   the same untraced function. Ours follows the handler.
//! - The respray reloads the car through the garage (as the car menu's paint choice does), which takes a
//!   moment; the nitrous tank is carried over, damage and dirt stay off.
//! - No effect or sound: none is named by the handler (the events above go to the scoring / HUD listeners).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use avian3d::prelude::*;
use bevy::prelude::*;
use mw_formats::{Collectable, Trigger, TriggerShape};

use crate::damage::CarDamage;
use crate::drive::{Car, CarInput, PendingCar, Player};
use crate::frontend::Collected;
use crate::garage::{Garage, GarageAction};
use crate::paint_wear::CarGrime;
use crate::props::GameLayer;
use crate::{Game, Loaded};

/// The nitrous a pass awards (`FUN_00748ff0`: `FUN_00658ad0(player, DAT_00fd2104)`, `.rdata` = 100.0).
const NITROUS_AWARD: f32 = 100.0;
/// "BODY SHOP FOUND" (language string; logged).
const FOUND_TEXT: &str = "BODY SHOP FOUND";

pub struct BodyShopPlugin {
    /// `--test bodyshop`: drive through the shop nearest the spawn, with before / after screenshots.
    pub test: Option<Option<PathBuf>>,
}

impl Plugin for BodyShopPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BodyShops>()
            .init_resource::<Collected>()
            .add_systems(Update, (read_palette, pass).chain())
            // After the new car's own start-up state (`--damage`, `--dirt`) has been applied.
            .add_systems(PostUpdate, finish_respray);
        if let Some(out) = &self.test {
            app.insert_resource(ShopTest { out: out.clone(), phase: 0, timer: 0.0, before: String::new(), paint: 0 })
                .add_systems(Update, run_test.before(pass));
        }
    }
}

type Palette = Vec<(usize, u32)>;

/// A respray in progress: the garage is loading the car in its next colour.
struct Respray {
    old_entity: Entity,
    tank: f32,
    started: f32,
}

#[derive(Resource, Default)]
pub struct BodyShops {
    loaded: bool,
    shops: Vec<Trigger>,
    inside: HashSet<u32>,
    /// The driven car's palette in the game's order (index into its paints, colour id), read on a thread
    /// when the car changes.
    palette: Option<(String, Palette)>,
    palette_job: Option<(String, Arc<Mutex<Option<Palette>>>)>,
    respray: Option<Respray>,
    /// Passes so far and the shop of the last one.
    pub passes: u32,
    pub last: Option<u32>,
}

/// Keep the driven car's palette at hand.
fn read_palette(mut shops: ResMut<BodyShops>, garage: Res<Garage>, game: Option<Res<Game>>) {
    let (Some(game), Some(current)) = (game, garage.current.as_ref()) else { return };
    if let Some((bundle, job)) = &shops.palette_job {
        let Some(palette) = job.lock().unwrap().take() else { return };
        shops.palette = Some((bundle.clone(), palette));
        shops.palette_job = None;
    }
    if shops.palette.as_ref().is_some_and(|(b, _)| *b == current.bundle) {
        return;
    }
    let job = Arc::new(Mutex::new(None));
    let (out, store, bundle) = (job.clone(), game.store.clone(), current.bundle.clone());
    std::thread::Builder::new()
        .name("mw-palette".into())
        .spawn(move || {
            let palette = store.bundle(&bundle).and_then(|b| store.vehicle_palette(&b).ok()).unwrap_or_default();
            *out.lock().unwrap() = Some(palette);
        })
        .expect("spawn palette thread");
    shops.palette_job = Some((current.bundle.clone(), job));
}

/// `FUN_0058bc60`: the palette entry after the current colour, cyclic; a colour that is not in the palette
/// stays.
fn next_paint(palette: &[(usize, u32)], current: usize) -> usize {
    match palette.iter().position(|(i, _)| *i == current) {
        Some(at) => palette[(at + 1) % palette.len()].0,
        None => current,
    }
}

#[allow(clippy::too_many_arguments)]
fn pass(
    mut shops: ResMut<BodyShops>,
    game: Option<Res<Game>>,
    mut collected: ResMut<Collected>,
    garage: Res<Garage>,
    mut actions: MessageWriter<GarageAction>,
    pending: Option<Res<PendingCar>>,
    time: Res<Time<Real>>,
    mut cars: Query<(Entity, &mut Car, &Transform, Option<&mut CarDamage>, Option<&mut CarGrime>), With<Player>>,
) {
    let Some(game) = game else { return };
    let shops = &mut *shops;
    if !shops.loaded {
        shops.loaded = true;
        match game.store.trigger_data() {
            Ok(data) => shops.shops = data.collectables(Collectable::BodyShop).copied().collect(),
            Err(e) => warn!("body shops: TriggerData not loaded: {e}"),
        }
        info!("body shops: {}", shops.shops.len());
    }
    let Some((entity, mut car, transform, damage, grime)) = cars.iter_mut().next() else { return };
    let p = transform.translation.to_array();
    let mut entered = None;
    for shop in &shops.shops {
        if !shop.contains(p) {
            shops.inside.remove(&shop.id);
        } else if shops.inside.insert(shop.id) {
            entered = Some(shop.id);
        }
    }
    let Some(id) = entered else { return };
    // The handler does nothing more for a car that is being replaced.
    if shops.respray.is_some() || pending.is_some() || garage.loading.is_some() {
        return;
    }
    let Some(current) = garage.current.as_ref() else { return };
    if collected.body_shops.insert(id) {
        collected.dirty = true;
        info!("{FOUND_TEXT}: {id} ({} found)", collected.body_shops.len());
    }
    // Message 0x16: repair and wash.
    let worst = damage.as_ref().map_or(0.0, |d| d.levels.iter().copied().fold(0.0, f32::max));
    if let Some(mut d) = damage {
        d.repair();
    }
    let before = grime.as_deref().copied().unwrap_or_default();
    if let Some(mut g) = grime {
        *g = CarGrime::default();
    }
    car.nitrous.add(NITROUS_AWARD);
    // Message 0xc: the next colour of the palette.
    let next = match &shops.palette {
        Some((bundle, palette)) if *bundle == current.bundle && !palette.is_empty() => {
            next_paint(palette, current.paint_index)
        }
        // The palette is not read yet (a pass right after a car change): the paints in their stored order.
        _ => (current.paint_index + 1) % current.paint_count.max(1),
    };
    shops.passes += 1;
    shops.last = Some(id);
    info!(
        "body shop {id}: repaired (worst zone {worst:.2}), washed (dust {:.2}, dirt {:.2}), nitrous full, paint {} -> {next} of {} at {:.0} mph",
        before.dust,
        before.dirt,
        current.paint_index,
        current.paint_count,
        car.speed / 0.447_04
    );
    if next != current.paint_index {
        let name = garage
            .catalogue
            .iter()
            .find(|c| c.bundle.eq_ignore_ascii_case(&current.bundle))
            .map_or_else(|| current.bundle.clone(), |c| c.name.clone());
        actions.write(GarageAction::Drive { name, bundle: current.bundle.clone(), paint_index: next });
        shops.respray = Some(Respray { old_entity: entity, tank: car.nitrous.tank, started: time.elapsed_secs() });
    }
}

/// The resprayed car has replaced the old one: it is the same car, so it keeps the tank and stays repaired and
/// clean (the garage spawns a fresh car, with the run's `--damage` / `--dirt` start values).
fn finish_respray(
    mut shops: ResMut<BodyShops>,
    time: Res<Time<Real>>,
    mut cars: Query<(Entity, &mut Car, Option<&mut CarDamage>, Option<&mut CarGrime>), With<Player>>,
) {
    let Some(respray) = shops.respray.as_ref() else { return };
    if time.elapsed_secs() - respray.started > 60.0 {
        warn!("body shop: the resprayed car did not arrive");
        shops.respray = None;
        return;
    }
    let Some((entity, mut car, damage, grime)) = cars.iter_mut().next() else { return };
    if entity == respray.old_entity {
        return;
    }
    // Both arrive a frame or two after the car (`damage::attach_damage`, `paint_wear::attach_paint`).
    let (Some(mut damage), Some(mut grime)) = (damage, grime) else { return };
    car.nitrous.tank = respray.tank;
    damage.repair();
    *grime = CarGrime::default();
    shops.respray = None;
}

// ---------------------------------------------------------------------------------------------
// `--test bodyshop`: drive through the shop nearest the spawn point; screenshots before and after.

#[derive(Resource)]
struct ShopTest {
    out: Option<PathBuf>,
    phase: u8,
    timer: f32,
    before: String,
    paint: usize,
}

impl ShopTest {
    fn shoot(&self, commands: &mut Commands, suffix: &str) {
        let Some(out) = &self.out else { return };
        let stem = out.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let ext = out.extension().map_or("png".into(), |e| e.to_string_lossy().to_string());
        let path = out.with_file_name(format!("{stem}-{suffix}.{ext}"));
        info!("TEST bodyshop: screenshot {}", path.display());
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(path));
    }
}

fn state_line(car: &Car, damage: Option<&CarDamage>, grime: Option<&CarGrime>, garage: &Garage) -> String {
    let worst = damage.map_or(f32::NAN, |d| d.levels.iter().copied().fold(0.0, f32::max));
    let zones = damage.map_or(0, |d| d.levels.iter().filter(|l| **l > 0.0).count());
    let g = grime.copied().unwrap_or(CarGrime { dust: f32::NAN, dirt: f32::NAN });
    let paint = garage.current.as_ref().map_or((0, 0), |c| (c.paint_index, c.paint_count));
    format!(
        "damage worst {worst:.2} ({zones} zones), dust {:.2}, dirt {:.2}, paint {} of {}, nitrous {:.0}",
        g.dust, g.dirt, paint.0, paint.1, car.nitrous.tank
    )
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run_test(
    mut commands: Commands,
    mut test: ResMut<ShopTest>,
    shops: Res<BodyShops>,
    mut input: ResMut<CarInput>,
    time: Res<Time<Real>>,
    garage: Res<Garage>,
    collected: Res<Collected>,
    loaded: Res<Loaded>,
    spatial: SpatialQuery,
    mut player: Query<
        (
            Entity,
            &Car,
            &mut Transform,
            &mut LinearVelocity,
            &mut AngularVelocity,
            Option<&CarDamage>,
            Option<&CarGrime>,
        ),
        With<Player>,
    >,
    mut exit: MessageWriter<AppExit>,
) {
    test.timer += time.delta_secs();
    let fail = |what: &str, exit: &mut MessageWriter<AppExit>| {
        println!("TEST RESULT bodyshop FAILED: {what}");
        exit.write(AppExit::Success);
    };
    let Ok((entity, car, mut t, mut v, mut w, damage, grime)) = player.single_mut() else {
        if test.timer > 120.0 && shops.respray.is_none() {
            fail("no player car", &mut exit);
        }
        return;
    };
    match test.phase {
        // Wait for the world around the car, then line the car up with the nearest shop.
        0 => {
            input.scripted = Some([0.0, 0.0, 0.0, 1.0]);
            if !shops.loaded || !loaded.done || damage.is_none() || grime.is_none() || test.timer < 4.0 {
                return;
            }
            let p = t.translation;
            let Some(shop) = shops
                .shops
                .iter()
                .min_by(|a, b| Vec3::from(a.position).distance(p).total_cmp(&Vec3::from(b.position).distance(p)))
            else {
                return fail("no body shops in the data", &mut exit);
            };
            let centre = Vec3::from(shop.position);
            let TriggerShape::Box { rotation: [x, y, z, qw], size } = shop.shape else {
                return fail("the shop is not a box", &mut exit);
            };
            let rotation = Quat::from_xyzw(x, y, z, qw);
            let filter = SpatialQueryFilter::from_mask(GameLayer::World);
            // Approach along one of the box's horizontal axes: the first with road before the box and a free
            // line into it.
            let mut start = None;
            for (axis, half) in [(Vec3::X, size[0] * 0.5), (Vec3::Z, size[2] * 0.5)] {
                for sign in [1.0, -1.0] {
                    let dir = (rotation * axis * -sign).with_y(0.0).normalize_or(Vec3::Z);
                    let from = centre - dir * (half + 14.0) + Vec3::Y * 3.0;
                    let Some(hit) = spatial.cast_ray(from, Dir3::NEG_Y, 12.0, true, &filter) else { continue };
                    let ground = from - Vec3::Y * hit.distance;
                    let eye = ground + Vec3::Y * 0.8;
                    let Ok(d) = Dir3::new(dir) else { continue };
                    if spatial.cast_ray(eye, d, half + 14.0, true, &filter).is_none() && start.is_none() {
                        start = Some((ground, dir));
                    }
                }
            }
            let Some((ground, dir)) = start else {
                return fail(
                    &format!(
                        "no free approach to shop {} at {centre:.0} ({:.0} m from the spawn)",
                        shop.id,
                        centre.distance(p)
                    ),
                    &mut exit,
                );
            };
            println!(
                "TEST RESULT bodyshop shop {} at {:.1} {:.1} {:.1} box {:.1}x{:.1}x{:.1}, approach from {:.1} {:.1} {:.1}",
                shop.id, centre.x, centre.y, centre.z, size[0], size[1], size[2], ground.x, ground.y, ground.z
            );
            t.translation = ground + Vec3::Y * 0.3;
            t.rotation = Quat::from_rotation_y(dir.x.atan2(dir.z));
            v.0 = Vec3::ZERO;
            w.0 = Vec3::ZERO;
            commands.entity(entity).insert((Position(t.translation), Rotation(t.rotation)));
            test.phase = 1;
            test.timer = 0.0;
        }
        // Settle, record the state, screenshot.
        1 => {
            input.scripted = Some([0.0, 0.0, 0.0, 1.0]);
            if test.timer < 2.5 {
                return;
            }
            test.before = state_line(car, damage, grime, &garage);
            test.paint = garage.current.as_ref().map_or(0, |c| c.paint_index);
            println!("TEST RESULT bodyshop before: {}", test.before);
            test.shoot(&mut commands, "before");
            test.phase = 2;
            test.timer = 0.0;
        }
        // Drive through.
        2 => {
            input.scripted = Some([if car.speed < 9.0 { 0.6 } else { 0.0 }, 0.0, 0.0, 0.0]);
            if shops.passes > 0 {
                println!(
                    "TEST RESULT bodyshop pass: shop {:?} at {:.1} mph after {:.1} s",
                    shops.last,
                    car.speed / 0.447_04,
                    test.timer
                );
                test.phase = 3;
                test.timer = 0.0;
            } else if test.timer > 20.0 {
                fail(&format!("no pass after 20 s (car at {:.1})", t.translation), &mut exit);
            }
        }
        // Stop, wait for the resprayed car.
        3 => {
            input.scripted = Some([0.0, 1.0, 0.0, 0.0]);
            if shops.respray.is_some() || damage.is_none() || grime.is_none() || test.timer < 4.0 {
                if test.timer > 70.0 {
                    fail("the resprayed car did not arrive", &mut exit);
                }
                return;
            }
            let after = state_line(car, damage, grime, &garage);
            let paint = garage.current.as_ref().map_or(0, |c| c.paint_index);
            let clean = damage.is_some_and(|d| d.levels.iter().all(|l| *l == 0.0))
                && grime.is_some_and(|g| g.dust < 0.05 && g.dirt < 0.05);
            println!("TEST RESULT bodyshop after: {after}");
            println!(
                "TEST RESULT bodyshop {}: repaired and washed {clean}, resprayed {} ({} -> {paint}), shops found {:?}",
                if clean && paint != test.paint { "PASSED" } else { "FAILED" },
                paint != test.paint,
                test.paint,
                collected.body_shops
            );
            test.shoot(&mut commands, "after");
            test.phase = 4;
            test.timer = 0.0;
        }
        _ => {
            input.scripted = Some([0.0, 1.0, 0.0, 0.0]);
            if test.timer > 1.5 {
                exit.write(AppExit::Success);
            }
        }
    }
}
