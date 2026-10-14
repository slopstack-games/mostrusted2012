//! Dirt, dust and scratches on the car's paint, as the original builds them up and draws them.
//!
//! **Build-up** (`FUN_009d19c0`, once per physics tick from the vehicle update `FUN_00904da0` →
//! `FUN_009d6e30`; docs/decomp.md "Vehicle paint, liveries and dirt"). The vehicle keeps two levels (0–1): dust
//! at vehicle +0x2c80 and dirt at +0x2c84. For each wheel on the ground whose surface has `VehicleEffects`
//! (`mw_assets::surfaces::VehicleEffects`, `Dirt` / `Dust` = (rate, time)):
//!
//! - t = clamp((|v| − start) / max(full − start, ε)), v the car's speed in mph (30 → 120 on most surfaces);
//! - dirt += max(wheel intensities, t) × dt / max(Dirt.time, dt) × Dirt.rate / 4;
//! - dust += t × dt / max(Dust.time, dt) × Dust.rate / 4;
//!
//! each clamped to 0..1 after every wheel. Negative rates (the two water surfaces) wash the car. The wheel
//! intensities are three smoothed per-wheel skid values (`FUN_009cef90`, not ported): here the skid sound's
//! lateral and spin ramps per wheel stand in (model decision, `tyre_marks::intensities`).
//!
//! **Drawing**: `FUN_00583290` puts dust × 2 and dirt × 3 in the render record, `FUN_008e9c00` multiplies them by
//! `DustColour.w` / `DirtColour.w` into `GenericVector4_A.xy` ([`shader_levels`]); `layered.wgsl` has the maths.
//! Scratches follow the car's zone damage: `FUN_00585030` fills a 3 × 5 grid (`gavDamageVolumeValues`) over
//! the car's bounding box from the eight zone levels × `VehicleInfo.76bfd668` (10 on most cars), capped at 1
//! ([`damage_grid`]); the paint's vertex program samples it at the bind-pose position.
//!
//! **Washing**: the game's repair (`FUN_0058f060`, a vtable method that also clears the damage) posts message
//! 0x12, which zeroes the levels (`FUN_00919240`); here the reset key does both (`damage.rs`). Message 0x13
//! sets them to one value (`FUN_00589770`); `--dirt` starts the player's car that way.

use bevy::camera::primitives::Aabb;
use bevy::prelude::*;

use crate::damage::CarDamage;
use crate::drive::{Car, Player};
use crate::materials::GameMaterial;

pub struct PaintWearPlugin;

impl Plugin for PaintWearPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedPostUpdate, build_up)
            .add_systems(Update, (attach_paint, clean_on_reset, write_materials).chain());
        if std::env::var_os("MW_DIRT_LOG").is_some() {
            app.add_systems(Update, log_levels);
        }
    }
}

/// The car's dust and dirt levels (vehicle +0x2c80 / +0x2c84, 0–1).
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct CarGrime {
    pub dust: f32,
    pub dirt: f32,
}

/// The car's own copies of its paint materials and what was last written into them.
#[derive(Component)]
struct CarPaint {
    materials: Vec<Handle<GameMaterial>>,
    /// gvDamageVolumeParam: (2 / width, 4 / length, −min.x × 2 / width, −min.z × 4 / length) of the body's
    /// bounding box in car space (`FUN_0057e840`).
    grid_param: Vec4,
    /// `VehicleInfo.76bfd668`: zone level → scratch weight scale.
    scratch_scale: f32,
    written: Option<([f32; 2], [f32; 15])>,
}

/// The car's scratch scale (`VehicleInfo.76bfd668`), inserted with the car by the loader when known.
#[derive(Component, Clone, Copy)]
pub struct ScratchScale(pub f32);

/// `GenericVector4_A.xy` from the car's dust and dirt levels: (dust × 2 × `DustColour.w`, dirt × 3 ×
/// `DirtColour.w`) (`FUN_00583290` scales the levels into the render record, `FUN_008e9c00` by the colours' w).
pub fn shader_levels(dust: f32, dirt: f32, dust_w: f32, dirt_w: f32) -> [f32; 2] {
    [dust * DUST_TO_RECORD * dust_w, dirt * DIRT_TO_RECORD * dirt_w]
}

/// `.rdata` 0x00fac280 (2) and 0x00fabaf8 (3): `FUN_00583290`'s scales from the vehicle's levels to the render
/// record's words 0x28 / 0x29.
const DUST_TO_RECORD: f32 = 2.0;
const DIRT_TO_RECORD: f32 = 3.0;
/// `.rdata` 0x01026120: the build-up's per-wheel divisor (four wheels).
const WHEELS: f32 = 4.0;
/// `.rdata` 0x01025ab0: the smallest speed span of the ramp.
const MIN_SPAN: f32 = 1.192_092_9e-7;
/// `.rdata` 0x00f9cad4: zone levels are capped at this while the car is not wrecked (`FUN_00585030`).
const ZONE_CAP: f32 = 0.99;
const MS_TO_MPH: f32 = 2.236_936;

/// One tick of `FUN_009d19c0` for every car.
fn build_up(time: Res<Time>, mut cars: Query<(&Car, &mut CarGrime)>) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (car, mut grime) in &mut cars {
        let mph = car.speed.abs() * MS_TO_MPH;
        let (mut dust, mut dirt) = (grime.dust, grime.dirt);
        for (contact, (_ratio, angle)) in car.wheel_contacts().zip(car.wheel_slips()) {
            let Some(c) = contact else { continue };
            let Some(fx) = c.effects else { continue };
            let t = ((mph - fx.speed_start) / (fx.speed_full - fx.speed_start).max(MIN_SPAN)).clamp(0.0, 1.0);
            // Stand-in for the wheel's three smoothed skid values (`FUN_009cef90`).
            let (lat, spin) =
                crate::tyre_marks::intensities(angle, c.velocity.length() * MS_TO_MPH, c.tread_slip * MS_TO_MPH);
            let s = lat.max(spin).max(t);
            let (rate, secs) = fx.dirt;
            dirt = (dirt + s * dt / secs.max(dt) * rate / WHEELS).clamp(0.0, 1.0);
            let (rate, secs) = fx.dust;
            dust = (dust + t * dt / secs.max(dt) * rate / WHEELS).clamp(0.0, 1.0);
        }
        if (dust, dirt) != (grime.dust, grime.dirt) {
            grime.dust = dust;
            grime.dirt = dirt;
        }
    }
}

/// The 15 `gavDamageVolumeValues` (`FUN_00585030`), row by row from the car's back (min z) to its front, each
/// row right (min x) to left: back and front rows twice, the middle row (right side, the smaller of the two
/// sides, left side). Each value is min(zone level (≤ 0.99) × `scale`, 1). Zone order on the vehicle (+0x112c
/// …): front-left, front, front-right, right, rear-right, rear, rear-left, left; matched to our clips by name.
pub fn damage_grid(levels: impl Iterator<Item = (&'static str, f32)>, scale: f32) -> [f32; 15] {
    damage_grid_raw(levels).map(|v| (v * scale).min(1.0))
}

/// [`damage_grid`] before the scale: each cell's zone level (≤ 0.99). `FUN_00585030` compares these with the
/// cells' glass thresholds (`player_sfx.rs`).
pub fn damage_grid_raw(levels: impl Iterator<Item = (&'static str, f32)>) -> [f32; 15] {
    let mut z = [0.0f32; 8];
    for (name, l) in levels {
        let i = match name {
            "front-left" => 0,
            "front" => 1,
            "front-right" => 2,
            "right" => 3,
            "rear-right" => 4,
            "rear" => 5,
            "rear-left" => 6,
            "left" => 7,
            _ => continue,
        };
        z[i] = l.min(ZONE_CAP);
    }
    let [fl, f, fr, r, rr, b, rl, l] = z;
    [rr, b, rl, rr, b, rl, r, l.min(r), l, fr, f, fl, fr, f, fl]
}

/// Give every new car its grime and its own copies of its paint materials.
#[allow(clippy::type_complexity)]
fn attach_paint(
    mut commands: Commands,
    cars: Query<(Entity, Option<&Player>, Option<&ScratchScale>), (With<Car>, Without<CarPaint>)>,
    children: Query<&Children>,
    parts: Query<(Option<&Aabb>, &MeshMaterial3d<GameMaterial>), With<Mesh3d>>,
    mut materials: ResMut<Assets<GameMaterial>>,
) {
    'cars: for (car, player, scale) in &cars {
        let Ok(kids) = children.get(car) else { continue };
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        let mut copies: Vec<(Handle<GameMaterial>, Handle<GameMaterial>)> = Vec::new();
        let mut targets = Vec::new();
        for kid in kids.iter() {
            let Ok((aabb, mat)) = parts.get(kid) else { continue };
            // The body's bounding box (the game's: its model's mesh boxes, `FUN_0057c120`). The body meshes sit
            // at the car's origin; their boxes appear once Bevy has computed them.
            let Some(aabb) = aabb else { continue 'cars };
            lo = lo.min(Vec3::from(aabb.min()));
            hi = hi.max(Vec3::from(aabb.max()));
            let Some(m) = materials.get(&mat.0) else { continue };
            if m.extension.params.mode != 6 || m.extension.params.p[19].w < 0.5 {
                continue;
            }
            let m = m.clone();
            let copy = match copies.iter().find(|(src, _)| *src == mat.0) {
                Some((_, c)) => c.clone(),
                None => {
                    let c = materials.add(m);
                    copies.push((mat.0.clone(), c.clone()));
                    c
                }
            };
            targets.push((kid, copy));
        }
        if lo.x > hi.x {
            continue;
        }
        for (kid, copy) in targets {
            commands.entity(kid).insert(MeshMaterial3d(copy));
        }
        let (w, l) = ((hi.x - lo.x).max(0.1), (hi.z - lo.z).max(0.1));
        let grid_param = Vec4::new(2.0 / w, 4.0 / l, -lo.x * 2.0 / w, -lo.z * 4.0 / l);
        let start = if player.is_some() {
            let [dirt, dust] = crate::materials::dirt_level();
            CarGrime { dust, dirt }
        } else {
            CarGrime::default()
        };
        commands.entity(car).insert((
            start,
            CarPaint {
                materials: copies.into_iter().map(|(_, c)| c).collect(),
                grid_param,
                scratch_scale: scale.map_or(10.0, |s| s.0),
                written: None,
            },
        ));
    }
}

/// The game's repair washes the car too (message 0x12 next to the damage reset in `FUN_0058f060`).
fn clean_on_reset(keys: Res<ButtonInput<KeyCode>>, mut cars: Query<&mut CarGrime>) {
    if keys.just_pressed(KeyCode::KeyR) {
        for mut g in &mut cars {
            *g = CarGrime::default();
        }
    }
}

/// Push each car's levels and damage grid into its paint materials when they changed visibly.
fn write_materials(
    mut cars: Query<(&CarGrime, &mut CarPaint, Option<&CarDamage>)>,
    mut materials: ResMut<Assets<GameMaterial>>,
) {
    for (grime, mut paint, damage) in &mut cars {
        let grid = damage.map_or([0.0; 15], |d| damage_grid(d.zone_levels(), paint.scratch_scale));
        let levels = [grime.dust, grime.dirt];
        let close = |a: &[f32], b: &[f32]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1.0 / 512.0);
        if paint.written.as_ref().is_some_and(|(l, g)| close(l, &levels) && close(g, &grid)) {
            continue;
        }
        paint.written = Some((levels, grid));
        for h in &paint.materials {
            let Some(mut m) = materials.get_mut(h) else { continue };
            let p = &mut m.extension.params.p;
            let a = shader_levels(grime.dust, grime.dirt, p[10].w, p[11].w);
            p[9].x = a[0];
            p[9].y = a[1];
            for (i, v) in grid.iter().enumerate() {
                p[14 + i / 4][i % 4] = *v;
            }
            p[18] = paint.grid_param;
        }
    }
}

/// `MW_DIRT_LOG=1`: the player's levels once a second.
fn log_levels(time: Res<Time>, cars: Query<(&Car, &CarGrime), With<Player>>, mut last: Local<f32>) {
    let now = time.elapsed_secs();
    if now - *last < 1.0 {
        return;
    }
    *last = now;
    for (car, g) in &cars {
        info!("grime: dust {:.4} dirt {:.4} at {:.0} mph", g.dust, g.dirt, car.speed.abs() * MS_TO_MPH);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_puts_zones_where_the_shader_reads_them() {
        let g = damage_grid([("front", 0.05), ("left", 0.2)].into_iter(), 10.0);
        // Rows 3 and 4 (front): (front-right, front, front-left).
        assert_eq!(&g[9..12], &[0.0, 0.5, 0.0]);
        assert_eq!(&g[12..15], &[0.0, 0.5, 0.0]);
        // Middle row: (right, min(left, right), left).
        assert_eq!(&g[6..9], &[0.0, 0.0, 1.0]);
        assert!(g[..6].iter().all(|v| *v == 0.0));
    }
}
