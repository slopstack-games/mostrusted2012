//! The lights cars cast on the world (docs/decomp.md "Car lights on the world"; data `mw_assets::car_lights`).
//!
//! **Which cars**: the game's 16 race-car slots, i.e. the player, the AI racers and the cops (`FUN_00588da0` calls
//! `FUN_005818f0` for each). Traffic and parked cars cast no light (`FUN_005fd190` only queues their coronas).
//!
//! **Per car** (`FUN_005818f0`): every `VehicleVfxBehaviour.Lights` entry whose `Group` bit is in the car's effect
//! mask (entity +0x3c8, `FUN_0057a5e0`: groups 0, 4, 6, 7 for a race car, 1 while braking, 2 in reverse, 3 the
//! cop bar; group 7 is dropped while the brake is on), at each locator of its group:
//! - the **viewed car** (`FUN_00588da0`: the local player's slot, param 12) skips group 6 and is the only car
//!   whose `Light_ProjectedTexture`s are drawn (the headlight projector, group 4: `LightBuffer_ProjectedTexture`);
//!   its lights skip the day / night gate;
//! - every other car takes the gate `da6ac384` through `FUN_0089b400` (0 always, 1 = 1 − night, 2 = night) and
//!   lights its headlights with the `Light_Spot` of group 6;
//! - intensity = the definition's `Intensity` × `a3de907e` × (the headlight level +0x3d4 for groups 4, 6 and 17,
//!   else the light level +0x3cc × the entry's `Animation`; both 1 on a running car), colour = `Colour` × the entry's
//!   colour; cone, point and projected lights go out when the car's damage at the lamp's corner reaches the
//!   global threshold (`FUN_0049fda0`: 0.01 × 100 → any damage; the viewed car's front corners never);
//! - spot / cone / point lights go through the world lights' programs (`FUN_008cebf0` / `FUN_008ce9c0` /
//!   `FUN_008ce920`, the same constants as `world_lights::gpu_light`), projected ones through `FUN_008cf0c0` /
//!   `FUN_008b1860` ([`projector_constants`]).
//!
//! **Headlight shadows** (the original's `HeadlightShadows` display option, on in this machine's config): the game
//! then renders a shadow map from the viewed car's projector every frame (`FUN_008cfe20` → `FUN_005837e0`,
//! `FUN_008b7a20`) and draws the projector with the diffuse-only shadowed program; here a shadow-only Bevy spot
//! light at the projector supplies the map ([`HeadlightShadowCaster`]). `MW_HEADLIGHT_SHADOWS=0` = the option off
//! (the specular program, no shadow).
//!
//! The group-3 light-bar lights are `police_lightbar.rs`'s. `MW_NO_CAR_LIGHTS=1` turns this module off (A/B);
//! `MW_CAR_LIGHTS_LOG=1` logs the lights submitted and the headlight shadow map's casters once a second.

use std::sync::Arc;

use bevy::image::{CompressedImageFormatSupport, CompressedImageFormats, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use mw_assets::car_lights::{CarLightSource, Projector};
use mw_assets::lights::{LightShape, WorldLight};

use crate::deferred::{DynamicLights, GpuProjectedLight};
use crate::drive::{Car, CarControls, Player};
use crate::weather::GAME_UNIT;

/// A race car's light sources (`VehicleVfxBehaviour.Lights`).
#[derive(Component, Clone)]
pub struct CarLights(pub Arc<Vec<CarLightSource>>);

pub struct CarLightsPlugin;

impl Plugin for CarLightsPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var_os("MW_NO_CAR_LIGHTS").is_some() {
            return;
        }
        app.init_resource::<Cookie>()
            .init_resource::<ShadowCasterPlacement>()
            .add_systems(Startup, spawn_shadow_caster)
            .add_systems(Update, load_cookie)
            .add_systems(
                PostUpdate,
                (submit, place_shadow_caster)
                    .chain()
                    .after(bevy::transform::TransformSystems::Propagate)
                    .before(bevy::light::SimulationLightSystems::UpdateBounds)
                    .before(bevy::light::SimulationLightSystems::UpdateLightFrusta),
            )
            .add_systems(
                PostUpdate,
                cull_headlight_casters.after(bevy::light::SimulationLightSystems::CheckLightVisibility),
            );
    }
}

/// Drop casters past their model's shadow distance from the headlight's shadow map, as `shadow_cull.rs` does for
/// the key light's cascades (`lod::LodShadow`). **Model decision**: the game's own culling for its headlight
/// shadow passes (`SortShadowHeadlightWorld` / `SortShadowHeadlightCarsMisc`) is not traced.
fn cull_headlight_casters(
    mut caster: Query<&mut bevy::camera::visibility::VisibleMeshEntities, With<HeadlightShadowCaster>>,
    lod: Query<&crate::lod::LodShadow>,
) {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| crate::lod::LodSettings::from_env().enabled);
    if !*ON {
        return;
    }
    for mut visible in &mut caster {
        visible.entities.retain(|&e| !lod.get(e).is_ok_and(|s| !s.0));
    }
}

/// `HeadlightShadows` (the original's display option, on in this machine's `config.NFS13Save`; `FUN_008cfe20`
/// renders the viewed car's headlight shadow map when it is on, `+0x1cc23`): `MW_HEADLIGHT_SHADOWS=0` turns it off
/// (the unshadowed program with the specular term).
fn headlight_shadows() -> bool {
    static ON: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var("MW_HEADLIGHT_SHADOWS").as_deref() != Ok("0"));
    *ON
}

/// The headlight's shadow map: a Bevy spot light that only casts shadows (black, intensity 0), placed at the viewed
/// car's projector each frame and looked up by `game_lighting.wgsl`'s `headlight_shadow`. The game renders its own
/// (`FUN_008b7a20`, passes `SortShadowHeadlightWorld` / `SortShadowHeadlightCarsMisc`) with the light's projection;
/// Bevy's is square, 2 × the wider half-angle, with its default biases (**model decision**).
#[derive(Component)]
struct HeadlightShadowCaster;

/// Where the shadow caster goes this frame: (position, beam, up, half-angle, far).
#[derive(Resource, Default)]
struct ShadowCasterPlacement(Option<(Vec3, Vec3, Vec3, f32, f32)>);

fn spawn_shadow_caster(mut commands: Commands) {
    commands.spawn((
        HeadlightShadowCaster,
        SpotLight { color: Color::BLACK, intensity: 0.0, shadow_maps_enabled: false, ..default() },
        Transform::default(),
    ));
}

#[allow(clippy::type_complexity)]
fn place_shadow_caster(
    time: Res<Time>,
    placement: Res<ShadowCasterPlacement>,
    mut caster: Query<
        (&mut SpotLight, &mut Transform, &mut GlobalTransform, Option<&bevy::camera::visibility::VisibleMeshEntities>),
        With<HeadlightShadowCaster>,
    >,
    mut log: Local<f32>,
) {
    let Ok((mut light, mut transform, mut global, visible)) = caster.single_mut() else { return };
    *log -= time.delta_secs();
    if std::env::var_os("MW_CAR_LIGHTS_LOG").is_some() && *log <= 0.0 {
        *log = 1.0;
        info!(
            "car lights: headlight shadow map {}, {} casters",
            if light.shadow_maps_enabled { "on" } else { "off" },
            visible.map_or(0, |v| v.entities.len())
        );
    }
    match placement.0 {
        Some((pos, beam, up, half, far)) if headlight_shadows() => {
            let t = Transform::from_translation(pos).looking_to(beam, up);
            *transform = t;
            // A root entity: its global transform is its transform (set now, so the light's frustum and visible
            // meshes this frame follow the car).
            *global = GlobalTransform::from(t);
            let angle = half.min(std::f32::consts::FRAC_PI_2 - 0.01);
            if !light.shadow_maps_enabled || light.outer_angle != angle || light.range != far {
                light.shadow_maps_enabled = true;
                light.outer_angle = angle;
                light.inner_angle = angle;
                light.range = far;
            }
        }
        _ => {
            if light.shadow_maps_enabled {
                light.shadow_maps_enabled = false;
            }
        }
    }
}

/// Race cars' effect mask (`FUN_0057a5e0`: entity +0x1a8c set on every race car → 0xd1 = groups 0, 4, 6, 7).
const RACE_MASK: u32 = 0xd1;
/// The groups lit at the headlight level (+0x3d4) instead of the light level (`FUN_005818f0`: 4, 6, 17).
const HEADLIGHT_GROUPS: [u8; 3] = [4, 6, 17];
/// The viewed car's headlight `Light_Spot` group, skipped for it (`FUN_005818f0`).
const SPOT_HEADLIGHT_GROUP: u8 = 6;
/// The tail light group, dropped while the brake light (group 1) is on (`FUN_005818f0`: render instance +0x1930).
const TAIL_GROUP: u8 = 7;
/// The cop bar's group: `police_lightbar.rs` draws it.
const COP_GROUP: u8 = 3;
/// Lights at or below this are not submitted (`.rdata` 0x00fab7ac).
const MIN_INTENSITY: f32 = 1.192_092_9e-7;
/// The projected light's near plane (`FUN_008b1860`: `.rdata` 0x00f9f660).
const NEAR: f32 = 0.1;
/// `FUN_0049fda0`: lamps within (−0.6, 0.1) m of the car's middle along its length are never damaged out
/// (`.rdata` 0x00fa3010, 0x00f9f660); lamps within 0.1 m of the centre line take half of each side.
const DAMAGE_REAR_Z: f32 = -0.6;
const DAMAGE_FRONT_Z: f32 = 0.1;
const DAMAGE_CENTRE_X: f32 = 0.1;
/// The global damage level at which a lamp goes out: the global object's +0x94 (0.01 in `re/out/live/snap1`)
/// × 100 + 0.5 (`.rdata` 0x00fa300c, 0x00f8a810), as a byte, against the zone level × 100 + 0.5 rounded.
const DAMAGE_THRESHOLD: f32 = 0.01;

/// The projector cookie (`Light_ProjectedTexture.Material`'s texture; every car's is the same, `mwtool car-lights
/// all`).
#[derive(Resource, Default)]
struct Cookie {
    requested: Option<mw_bundle::ResourceId>,
    job: Option<std::sync::Mutex<std::sync::mpsc::Receiver<Option<Image>>>>,
    image: Option<Handle<Image>>,
}

fn load_cookie(
    game: Option<Res<crate::Game>>,
    support: Option<Res<CompressedImageFormatSupport>>,
    cars: Query<&CarLights, Added<CarLights>>,
    mut cookie: ResMut<Cookie>,
    mut images: ResMut<Assets<Image>>,
) {
    if let Some(job) = &cookie.job {
        let got = job.lock().unwrap().try_recv();
        match got {
            Ok(image) => {
                cookie.image = image.map(|i| images.add(i));
                cookie.job = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(_) => cookie.job = None,
        }
    }
    if cookie.requested.is_some() {
        return;
    }
    let (Some(game), Some(support)) = (game, support) else { return };
    let Some(texture) = cars.iter().flat_map(|c| c.0.iter()).find_map(|l| l.projector.as_ref()?.texture) else {
        return;
    };
    cookie.requested = Some(texture);
    let bc = support.0.contains(CompressedImageFormats::BC);
    let store = game.store.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // Model decision: the cookie's texels are used as stored (`BC1_UNORM`, no sRGB decode), like the sky's
        // cloud textures (research log 2026-10-07).
        let image = store.texture(texture).ok().and_then(|t| crate::convert::image(&t, false, bc)).map(|mut i| {
            i.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
            i
        });
        let _ = tx.send(image);
    });
    cookie.job = Some(std::sync::Mutex::new(rx));
}

/// Rows of the locator's rotation as the game expands the quaternion (`FUN_005818f0`; row-vector convention):
/// x, y, z axes in car space.
fn locator_axes(q: [f32; 4]) -> [Vec3; 3] {
    let [x, y, z, w] = q;
    [
        Vec3::new(1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + w * z), 2.0 * (x * z - w * y)),
        Vec3::new(2.0 * (x * y - w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + w * x)),
        Vec3::new(2.0 * (x * z + w * y), 2.0 * (y * z - w * x), 1.0 - 2.0 * (x * x + y * y)),
    ]
}

/// The larger root of a d² + b d + c = 0, as `FUN_008a0a20` solves it (0 when there is none).
fn far_root(a: f32, b: f32, c: f32) -> f32 {
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return 0.0;
    }
    let s = disc.sqrt();
    if b.abs() <= 1e-12 {
        if a.abs() <= 1e-12 {
            return 0.0;
        }
        return ((-b + s) / (2.0 * a)).max((-b - s) / (2.0 * a));
    }
    let q = -0.5 * (b + b.signum() * s);
    let r1 = if a.abs() > 1e-12 { q / a } else { 0.0 };
    let r2 = if q.abs() > 1e-12 { c / q } else { 0.0 };
    r1.max(r2)
}

/// The constants of a projected light (`FUN_008cf0c0` → `FUN_008b1860`) at world position `pos` with frame
/// (`x`, `y`, `z` = the beam), the locator's local rotation `local` (its Rxx / Ryy set the angles), colour × intensity
/// `radiance` (game units) and the definition's threshold.
///
/// - Half-angles: atan(Rxx) × `FovScale.x`, atan(Ryy) × `FovScale.y` (`FUN_005818f0`: CRT `atanf` 0x00edc7b2 of
///   the locator rotation's diagonal, × 2 × 57.2958 × FovScale, then × 0.0174533 × 0.5 in `FUN_008cf0c0`), so 45° ×
///   FovScale for every car's (unrotated) projector locator; the projection scales x and y by 1 / tan(half-angle).
/// - Depth: z from `NEAR` to where colour × attenuation falls to the threshold t: the larger root of
///   a t d² + b t d + (c t − max(rgb)) = 0 (`FUN_008a0a20`); the GL-style projection z' = z (f + n) / (f − n) −
///   2 f n / (f − n), w = z (`.rdata` 0x00f9d488 = −2).
pub fn projector_constants(
    p: &Projector,
    pos: Vec3,
    axes: [Vec3; 3],
    local: [Vec3; 3],
    radiance: Vec3,
    threshold: f32,
) -> Option<(GpuProjectedLight, f32, f32)> {
    let m = radiance.max_element();
    if m <= MIN_INTENSITY {
        return None;
    }
    let [a, b, c] = p.attenuation;
    let t = threshold;
    let far = far_root(a * t, b * t, c * t - m);
    if far <= NEAR {
        return None;
    }
    let hx = local[0].x.atan() * p.fov_scale[0];
    let hy = local[1].y.atan() * p.fov_scale[1];
    let (sx, sy) = (1.0 / hx.tan(), 1.0 / hy.tan());
    if !sx.is_finite() || !sy.is_finite() {
        return None;
    }
    let view =
        Mat4::from_cols(axes[0].extend(0.0), axes[1].extend(0.0), axes[2].extend(0.0), pos.extend(1.0)).inverse();
    let proj = Mat4::from_cols(
        Vec4::new(sx, 0.0, 0.0, 0.0),
        Vec4::new(0.0, sy, 0.0, 0.0),
        Vec4::new(0.0, 0.0, (far + NEAR) / (far - NEAR), 1.0),
        Vec4::new(0.0, 0.0, -2.0 * far * NEAR / (far - NEAR), 0.0),
    );
    let light = GpuProjectedLight {
        clip_from_world: proj * view,
        position: pos.extend(1.0),
        colour: (radiance * GAME_UNIT).extend(0.0),
        attenuation: Vec4::new(a, b, c, 0.0),
        window: Vec4::from_array(p.window),
        params: Vec4::ZERO,
    };
    Some((light, hx.abs().max(hy.abs()), far))
}

/// `FUN_0049fda0`: 1, or 0 when the car's damage at the lamp's corner has reached the threshold (½ per side for
/// a lamp on the centre line). `local` is the lamp's position in car space (+x = the car's left, +z = forward);
/// `zones` = (front-left, front-right, rear-left, rear-right) levels, the viewed car's front ones 0.
fn damage_factor(local: Vec3, zones: [f32; 4]) -> f32 {
    if local.z > DAMAGE_REAR_Z && local.z < DAMAGE_FRONT_Z {
        return 1.0;
    }
    // x87 `fistp` rounds to the nearest even integer: an undamaged corner gives 0.
    let byte = |level: f32| (level.min(0.99) * 100.0 + 0.5).round_ties_even();
    let threshold = (DAMAGE_THRESHOLD * 100.0 + 0.5).trunc();
    let (left, right) = if local.z > 0.0 { (zones[0], zones[1]) } else { (zones[2], zones[3]) };
    let lit = |level: f32| byte(level) < threshold;
    if local.x.abs() < DAMAGE_CENTRE_X {
        return 0.5 * f32::from(u8::from(lit(left))) + 0.5 * f32::from(u8::from(lit(right)));
    }
    f32::from(u8::from(lit(if local.x > 0.0 { left } else { right })))
}

#[allow(clippy::type_complexity)]
fn submit(
    time: Res<Time>,
    night: Option<Res<crate::world_lights::NightLevel>>,
    cookie: Res<Cookie>,
    cars: Query<(
        &GlobalTransform,
        &CarLights,
        Option<&Car>,
        Option<&CarControls>,
        Has<Player>,
        Option<&crate::damage::CarDamage>,
        &InheritedVisibility,
    )>,
    mut out: ResMut<DynamicLights>,
    mut placement: ResMut<ShadowCasterPlacement>,
    mut log: Local<f32>,
) {
    placement.0 = None;
    let night = night.map_or(0.0, |n| n.0);
    out.cookie = cookie.image.clone();
    *log -= time.delta_secs();
    let logging = std::env::var_os("MW_CAR_LIGHTS_LOG").is_some() && *log <= 0.0;
    if logging {
        *log = 1.0;
    }
    let clock = time.elapsed_secs();
    let (spots_before, projected_before) = (out.lights.len(), out.projected.len());
    for (t, lights, car, controls, viewed, damage, vis) in &cars {
        if !vis.get() {
            continue;
        }
        let braking = controls.is_some_and(|k| k.brake > 0.0);
        let mut mask = RACE_MASK;
        if braking {
            mask |= 1 << 1;
            mask &= !(1 << TAIL_GROUP);
        }
        if car.is_some_and(|c| c.gear < 0) {
            mask |= 1 << 2;
        }
        let mut zones = [0.0f32; 4];
        if let Some(d) = damage {
            for (name, level) in d.zone_levels() {
                match name {
                    "front-left" if !viewed => zones[0] = level,
                    "front-right" if !viewed => zones[1] = level,
                    "rear-left" => zones[2] = level,
                    "rear-right" => zones[3] = level,
                    _ => {}
                }
            }
        }
        let affine = t.affine();
        if logging {
            info!(
                "car lights: car at {:.1} viewed {viewed} mask {mask:#x} sources {:?}",
                t.translation(),
                lights.0.iter().map(|l| (l.group, l.projector.is_some(), l.locators.len())).collect::<Vec<_>>()
            );
        }
        for l in lights.0.iter() {
            if l.group == COP_GROUP || l.group >= 32 || mask & (1 << l.group) == 0 {
                continue;
            }
            if viewed && l.group == SPOT_HEADLIGHT_GROUP {
                continue;
            }
            if l.projector.is_some() && !viewed {
                continue;
            }
            let headlight = HEADLIGHT_GROUPS.contains(&l.group);
            // The light level (+0x3cc) path takes the entry's animation; the headlight level (+0x3d4) does not.
            let mut level = l.scale;
            if !headlight && l.projector.is_none() {
                level *= l.animation.as_ref().map_or(1.0, |a| a.value(clock));
            }
            if !viewed {
                level *= match l.gate {
                    1 => 1.0 - night,
                    2 => night,
                    _ => 1.0,
                };
            }
            if level <= 0.0 {
                continue;
            }
            let colour = Vec3::from(l.def.colour) * Vec3::new(l.colour[0], l.colour[1], l.colour[2]);
            for loc in &l.locators {
                let local_pos = Vec3::from(loc.position);
                let local = locator_axes(loc.rotation);
                let pos = affine.transform_point3(local_pos);
                let axes = local.map(|a| affine.transform_vector3(a).normalize_or_zero());
                let damaged = match l.def.shape {
                    LightShape::Spot { .. } => 1.0,
                    _ => damage_factor(local_pos, zones),
                };
                let intensity = l.def.intensity * level * damaged;
                if intensity <= MIN_INTENSITY {
                    continue;
                }
                if let Some(p) = &l.projector {
                    if let Some((mut g, half, far)) =
                        projector_constants(p, pos, axes, local, colour * intensity, l.def.attenuation_threshold)
                    {
                        // The headlight shadow map is the viewed car's projector's (`FUN_005837e0`).
                        if headlight_shadows() && placement.0.is_none() {
                            placement.0 = Some((pos, axes[2], axes[1], half, far));
                            g.params.x = 1.0;
                        }
                        out.projected.push(g);
                    }
                    continue;
                }
                let w = WorldLight {
                    def: l.def.clone(),
                    position: pos.into(),
                    direction: axes[2].into(),
                    object: None,
                    stagger: None,
                };
                // `gpu_light` takes Colour × Intensity × level: the entry's colour goes into the level per channel.
                if let Some(mut g) = crate::world_lights::gpu_light(&w, level * damaged) {
                    let tint = Vec3::new(l.colour[0], l.colour[1], l.colour[2]);
                    let rgb = g.colour.truncate() * tint;
                    let max_game = rgb.max_element() / GAME_UNIT;
                    if max_game <= MIN_INTENSITY {
                        continue;
                    }
                    g.colour = rgb.extend(l.def.attenuation_threshold / max_game);
                    g.position.w = (max_game / l.def.attenuation_threshold.max(1e-3)).sqrt();
                    out.lights.push(g);
                }
            }
        }
    }
    if logging {
        info!(
            "car lights: {} spot / cone / point, {} projected (night {night:.2}, cookie {})",
            out.lights.len() - spots_before,
            out.projected.len() - projected_before,
            if out.cookie.is_some() { "loaded" } else { "pending" }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headlight_reaches_its_threshold_at_the_far_plane() {
        // The 911's group-4 projector: intensity 11.5, threshold 0.05, attenuation (0.001, 0.25, 10).
        let (a, b, c, m, t) = (0.001f32, 0.25f32, 10.0f32, 11.5f32, 0.05f32);
        let far = far_root(a * t, b * t, c * t - m);
        let att = 1.0 / (a * far * far + b * far + c);
        assert!((m * att - t).abs() < 1e-4, "far {far}");
        assert!((far - 360.0).abs() < 1.0);
    }

    #[test]
    fn projector_frustum() {
        let p = Projector {
            fov_scale: [1.23, 0.85],
            attenuation: [0.001, 0.25, 10.0],
            window: [-1.0, -1.0, 1.0, 0.098],
            material: None,
            texture: None,
        };
        let axes = [Vec3::X, Vec3::Y, Vec3::Z];
        let (g, half, _) = projector_constants(&p, Vec3::ZERO, axes, axes, Vec3::splat(11.5), 0.05).unwrap();
        assert!((half - (45f32 * 1.23).to_radians()).abs() < 1e-4);
        // A point on the beam's edge: 45° × 1.23 off the axis horizontally.
        let h = (45f32 * 1.23).to_radians();
        let c = g.clip_from_world * Vec4::new(10.0 * h.tan(), 0.0, 10.0, 1.0);
        assert!((c.x / c.w - 1.0).abs() < 1e-4);
        // The near and far planes map to −1 and 1.
        let near = g.clip_from_world * Vec4::new(0.0, 0.0, NEAR, 1.0);
        assert!((near.z / near.w + 1.0).abs() < 1e-4);
    }

    #[test]
    fn lamps_go_out_with_corner_damage() {
        let front_left = Vec3::new(0.6, 0.7, 2.0);
        assert_eq!(damage_factor(front_left, [0.0; 4]), 1.0);
        assert_eq!(damage_factor(front_left, [0.2, 0.0, 0.0, 0.0]), 0.0);
        assert_eq!(damage_factor(Vec3::new(0.0, 0.7, -2.2), [0.0, 0.0, 0.3, 0.0]), 0.5);
        assert_eq!(damage_factor(Vec3::new(0.6, 0.7, -0.3), [1.0; 4]), 1.0);
    }
}
