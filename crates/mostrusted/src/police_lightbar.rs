//! The light bar (docs/decomp.md "Police sirens, light bars and radio"; data `mw_assets::police_lights`): the
//! cop lights of effect group 3 while the car's bar is on ([`crate::police_fx::FlashCar::bar_on`]).
//!
//! - **The bar's glow** (`VehicleNFS13_..._LocalEmissive`, `materials.rs` kind 4): every frame the car's group-3
//!   coronas with an `Animation` are evaluated at the car's flash time, raised to the power 10 (`_CIpow`, 10.0
//!   at 0x00f9f698), those above 1e-4 kept, the strongest first; each of their locators whose +Z axis faces the
//!   camera becomes one of at most 16 local lights (position, intensity v¹⁰) of the bar's material
//!   (`FUN_00583290` → `FUN_0057fcf0`). The colour comes from the red / blue `Glass_Colourise` lenses over the
//!   white emission. Each car gets its own copy of the bar's material.
//! - **The bar's light on the world**: its group-3 `Light_Custom`s (blue 253a4 / red 253a5: intensity 35,
//!   threshold 0.05, ~26 m) at their locators, intensity × `a3de907e` × the animation's value (linear,
//!   `FUN_005818f0`), day and night (`da6ac384` = 0), through the port of the game's `LightBuffer_Cop`
//!   program (`game_lighting.wgsl`, cone.z = 1): diffuse only, faded towards the vertical ((1 − L̂.y)⁶ where the
//!   light is above the pixel, 1 + 0.75 L̂.y below).
//!
//! Not drawn: the coronas themselves (sprites with flares and glows, `FUN_004a5790`, not decoded), the grille
//! and rear-deck lamps' coronas (only through the bar's emissive where it has them). `MW_NO_LIGHTBAR=1` turns
//! this module off for A/B; `MW_COP_LIGHTS=1` forces every flashing car's bar on (screenshots).

use std::sync::Arc;

use bevy::prelude::*;
use mw_assets::lights::{LightShape, WorldLight};

use crate::deferred::{GpuWorldLight, WorldLightList};
use crate::materials::{GameMaterial, LOCAL_EMISSIVE_SLOTS, is_local_emissive};
use crate::police_fx::FlashCar;

pub struct LightBarPlugin;

impl Plugin for LightBarPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var_os("MW_NO_LIGHTBAR").is_some() {
            return;
        }
        app.init_resource::<BarLights>()
            .add_systems(Update, (own_materials, update_bars, shots).chain().after(crate::police_fx::cop_fx_state))
            .add_systems(PostUpdate, merge_lights);
    }
}

/// The exponent of the bar's local lights (`FUN_0057fcf0`, the double at 0x00f9f698).
const LOCAL_POWER: i32 = 10;
/// Local lights below this are left out (`FUN_0057fcf0`).
const LOCAL_MIN: f32 = 1e-4;

/// The car's own copies of its light-bar materials.
#[derive(Component)]
struct BarMaterials(Vec<Handle<GameMaterial>>);

/// The bars' world lights this frame, and the static list they are added to.
#[derive(Resource, Default)]
struct BarLights {
    lights: Vec<GpuWorldLight>,
    /// The world's own lights (`world_lights.rs`) as last seen, and the generation written here.
    base: Arc<Vec<GpuWorldLight>>,
    written: u64,
    had: bool,
}

fn forced() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("MW_COP_LIGHTS").is_some());
    *ON
}

/// Give each car with cop lights its own copy of every light-bar material on its body.
#[allow(clippy::type_complexity)]
fn own_materials(
    mut commands: Commands,
    cars: Query<(Entity, &Children), (With<FlashCar>, Without<BarMaterials>)>,
    mut parts: Query<&mut MeshMaterial3d<GameMaterial>>,
    mut materials: ResMut<Assets<GameMaterial>>,
) {
    for (car, children) in &cars {
        let mut own = Vec::new();
        let mut any = false;
        for c in children.iter() {
            let Ok(mut m) = parts.get_mut(c) else { continue };
            any = true;
            let Some(mat) = materials.get(&m.0) else { continue };
            if !is_local_emissive(mat) {
                continue;
            }
            let mut copy = mat.clone();
            for &i in &LOCAL_EMISSIVE_SLOTS {
                copy.extension.params.p[i] = Vec4::ZERO;
            }
            let h = materials.add(copy);
            m.0 = h.clone();
            own.push(h);
        }
        if any {
            commands.entity(car).insert(BarMaterials(own));
        }
    }
}

fn update_bars(
    time: Res<Time>,
    camera: Query<&GlobalTransform, crate::MainCamera>,
    cars: Query<(&FlashCar, &GlobalTransform, Option<&BarMaterials>)>,
    mut materials: ResMut<Assets<GameMaterial>>,
    mut bar_lights: ResMut<BarLights>,
) {
    let eye = camera.iter().next().map_or(Vec3::ZERO, |c| c.translation());
    bar_lights.lights.clear();
    for (f, t, mats) in &cars {
        let on = f.bar_on() || forced();
        let clock = time.elapsed_secs() * f.rate;
        let affine = t.affine();
        // The bar's local lights.
        let mut local: Vec<Vec4> = Vec::with_capacity(16);
        if on {
            let mut lit: Vec<(f32, &mw_assets::police_lights::FlashCorona)> = f
                .lights
                .coronas
                .iter()
                .filter_map(|c| Some((c.animation.as_ref()?.value(clock).powi(LOCAL_POWER), c)))
                .filter(|(v, _)| *v > LOCAL_MIN)
                .collect();
            lit.sort_by(|a, b| b.0.total_cmp(&a.0));
            'fill: for (v, c) in lit {
                for l in &c.locators {
                    if local.len() >= 16 {
                        break 'fill;
                    }
                    let p = affine.transform_point3(Vec3::from(l.position));
                    let q = Quat::from_array(l.rotation);
                    let axis = affine.transform_vector3(q * Vec3::Z);
                    if axis.dot(eye - p) >= 0.0 {
                        local.push(p.extend(v));
                    }
                }
            }
        }
        if let Some(mats) = mats {
            for h in &mats.0 {
                let Some(mut m) = materials.get_mut(h) else { continue };
                for (k, &i) in LOCAL_EMISSIVE_SLOTS.iter().enumerate() {
                    m.extension.params.p[i] = local.get(k).copied().unwrap_or(Vec4::ZERO);
                }
            }
        }
        // The bar's lights on the world.
        if on {
            for l in &f.lights.lights {
                let level = l.scale * l.animation.as_ref().map_or(1.0, |a| a.value(clock));
                if level <= 0.0 {
                    continue;
                }
                let mut def = l.def.clone();
                def.shape = LightShape::Point;
                let def = Arc::new(def);
                for loc in &l.locators {
                    let p = affine.transform_point3(Vec3::from(loc.position));
                    let w = WorldLight {
                        def: def.clone(),
                        position: p.into(),
                        direction: [0.0, -1.0, 0.0],
                        object: None,
                        stagger: None,
                    };
                    if let Some(mut g) = crate::world_lights::gpu_light(&w, level) {
                        g.cone.z = 1.0;
                        bar_lights.lights.push(g);
                    }
                }
            }
        }
    }
}

/// `MW_LIGHTBAR_SHOTS=<dir>`: from 2 s after a bar first shows, 16 screenshots 1/8 s of game time apart
/// (`bar_00.png` …), each logged with the flash values of the first car's animated coronas.
fn shots(mut commands: Commands, time: Res<Time>, cars: Query<&FlashCar>, mut state: Local<(Option<f32>, u32)>) {
    use bevy::render::view::screenshot::{Screenshot, save_to_disk};
    let Some(dir) = std::env::var_os("MW_LIGHTBAR_SHOTS") else { return };
    let Some(f) = cars.iter().find(|f| f.bar_on() || forced()) else { return };
    let now = time.elapsed_secs();
    let start = *state.0.get_or_insert(now + 2.0);
    let k = state.1;
    if k >= 16 || now < start + k as f32 * 0.125 {
        return;
    }
    let clock = now * f.rate;
    let values: Vec<String> = f
        .lights
        .coronas
        .iter()
        .filter_map(|c| c.animation.as_ref().map(|a| format!("{:.2}", a.value(clock))))
        .collect();
    info!("lightbar shot {k}: t {now:.3} flash clock {clock:.3}: {}", values.join(" "));
    let path = std::path::Path::new(&dir).join(format!("bar_{k:02}.png"));
    commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    state.1 += 1;
}

/// Add the bars' lights to the world's light list (kept by `world_lights.rs`, rebuilt there when the hour
/// changes).
fn merge_lights(mut bar: ResMut<BarLights>, mut list: ResMut<WorldLightList>) {
    if list.generation != bar.written {
        bar.base = list.lights.clone();
    }
    if bar.lights.is_empty() && !bar.had {
        bar.written = list.generation;
        return;
    }
    let mut all = Vec::with_capacity(bar.base.len() + bar.lights.len());
    all.extend_from_slice(&bar.base);
    all.extend_from_slice(&bar.lights);
    list.lights = Arc::new(all);
    list.generation += 1;
    bar.written = list.generation;
    bar.had = !bar.lights.is_empty();
}
