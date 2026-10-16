//! The world's lights at night: street lamps, flood lights, signs (`LightInstanceList` + GeneSys
//! `Light_*`, `mw_assets::lights`) and the night level that the lit-window and lightmap shaders take.
//!
//! The loader sends each track unit's lights ([`crate::loader::Msg::Lights`]): those of its light lists and
//! those its objects carry (street lamps: a `WCVfxBehaviour` light at each locator of a locator group). Whenever the hour or the
//! set of lights changes, [`update_lights`] turns them into the constants of the game's light programs
//! (`deferred::GpuWorldLight`; the game's CPU side is `FUN_008b0940` for point lights, `FUN_008af930` for
//! cone lights and `FUN_008afee0` for spot lights, docs/decomp.md "World lights") and hands the list to
//! the lighting pass (`deferred.rs`), which draws one light volume per light.
//!
//! What switches a light on is traced (`mw_assets::lights::DayPhase::light_level`, the game's
//! `FUN_0089b400`): its definition's `OnDuringDay` / `OnDuringNight` flags against the environment's day flag
//! and night level, which ramps over `LightTransitionDuration` at both ends of the night (`FUN_008a8480`).
//!
//! Not drawn: line lights (66 instances; `LightBuffer_Line` not ported) and the game's own cap on lights per
//! frame (it keeps the nearest of a fixed-size list per type, `FUN_008dfa00`: the main view's spot and cone lists
//! keep 224 of 256, the point list 128 of 160, `.data` 0x010c6fd0 / 0x010c6f88 / 0x010c7060). The lights cars carry
//! are `car_lights.rs`'s. Flash patterns are not animated (those lights are steady). A knocked-over lamp's light goes
//! out (`prop_vfx.rs`: its "SetLightsOn" behaviour is switched off by the contact).

use std::sync::Arc;

use bevy::prelude::*;
use mw_assets::lights::{DayNightTransition, DayPhase, LightShape, WorldLight};

use crate::deferred::{GpuWorldLight, WorldLightList};
use crate::weather::{EnvironmentState, GAME_UNIT};

/// Every light of the loaded track units.
#[derive(Resource, Default)]
pub struct LightSources {
    pub lights: Vec<WorldLight>,
    pub transition: DayNightTransition,
    /// The list has grown since the last [`update_lights`].
    dirty: bool,
    /// The hour the list on the GPU was built for.
    shown_hour: Option<f32>,
}

impl LightSources {
    /// Take out the lights object definition `object` carries within `radius` of `near` (a knocked lamp's,
    /// `prop_vfx.rs`); [`Self::add`] puts them back.
    pub fn take_object_lights(&mut self, object: mw_bundle::ResourceId, near: Vec3, radius: f32) -> Vec<WorldLight> {
        let (taken, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.lights)
            .into_iter()
            .partition(|l| l.object == Some(object) && Vec3::from(l.position).distance(near) <= radius);
        self.lights = kept;
        self.dirty |= !taken.is_empty();
        taken
    }

    pub fn add(&mut self, lights: Vec<WorldLight>) {
        if !lights.is_empty() {
            self.lights.extend(lights);
            self.dirty = true;
        }
    }
}

/// `DynamicLighting_GlobalLightMapIntensities.x`, the global factor on the world shaders' lightmap, lit-window
/// and baked-light terms (`layered.wgsl`): the environment's night level (`DayPhase::night`). The game writes
/// the constant as (night, night or 1 while it rains, night, 0) in `FUN_008c2080`.
#[derive(Resource, Clone, Copy, PartialEq, Default)]
pub struct NightLevel(pub f32);

pub struct WorldLightsPlugin;

impl Plugin for WorldLightsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LightSources>()
            .init_resource::<NightLevel>()
            .add_systems(Startup, load_transition)
            .add_systems(Update, (update_lights, apply_night_level).chain());
    }
}

fn load_transition(game: Option<Res<crate::Game>>, mut sources: ResMut<LightSources>) {
    let Some(game) = game else { return };
    match game.store.day_night_transition() {
        Ok(t) => sources.transition = t,
        Err(e) => warn!("no day / night transition ({e}); using the game's values"),
    }
}

/// The game's light volumes come in 45 cone angles, 2° apart (`FUN_008ae860`, the table at renderer
/// +0xad0: entry i holds cos(i × 2°) and 1 / (1 − cos(i × 2°)); step `.rdata` 0x0100e760 = 0.0349 rad). A
/// light uses entry `angle / 2` rounded up (`FUN_00ede840` is the CRT ceil), at most 44 (`FUN_008b8840`): the edge is quantised outwards.
fn table_cos(angle_degrees: f32) -> f32 {
    let index = (angle_degrees * 0.5).ceil().clamp(0.0, 44.0);
    (index * 2.0).to_radians().cos()
}

/// The light program constants of one light at a brightness level (0–1), or `None` when it is off or of a
/// kind that is not drawn.
pub fn gpu_light(light: &WorldLight, level: f32) -> Option<GpuWorldLight> {
    let def = &light.def;
    let radiance = Vec3::from(def.radiance()) * level;
    let max = radiance.max_element();
    if max <= 0.0 || def.attenuation_threshold <= 0.0 {
        return None;
    }
    // deferred_lightColour.w = threshold / max channel; the volume's radius is sqrt(max / threshold).
    let cutoff = def.attenuation_threshold / max;
    let range = (max / def.attenuation_threshold).sqrt();
    let (cos_edge, scale, cone) = match def.shape {
        LightShape::Point => (0.0, 0.0, 0.0),
        // deferred_lightPosition.w = cos(edge), deferred_lightDirection.w = 1 / (1 − cos(edge)).
        LightShape::Cone { angle } => {
            let c = table_cos(angle);
            (c, 1.0 / (1.0 - c).max(1e-6), 1.0)
        }
        // The spot's edge comes from the same kind of table; its inner angle is used exactly:
        // deferred_lightDirection.w = 1 / (cos(inner) − cos(edge)), or FLT_MAX (`.rdata` 0x0100ce20) when the
        // inner cone is not inside the edge.
        LightShape::Spot { angle, inner } => {
            let c = table_cos(angle);
            let ci = inner.abs().to_radians().cos();
            (c, if ci > c { 1.0 / (ci - c) } else { f32::MAX }, 1.0)
        }
        LightShape::Line { .. } | LightShape::Other => return None,
    };
    Some(GpuWorldLight {
        position: Vec3::from(light.position).extend(range),
        colour: (radiance * GAME_UNIT).extend(cutoff),
        direction: (-Vec3::from(light.direction)).extend(cos_edge),
        cone: Vec4::new(scale, cone, 0.0, 0.0),
    })
}

/// Rebuild the light list and the night level when the hour or the lights have changed.
fn update_lights(
    state: Option<Res<EnvironmentState>>,
    mut sources: ResMut<LightSources>,
    mut list: ResMut<WorldLightList>,
    mut night: ResMut<NightLevel>,
) {
    let Some(state) = state else { return };
    let hour = state.want.hour;
    if !sources.dirty && sources.shown_hour == Some(hour) {
        return;
    }
    sources.dirty = false;
    sources.shown_hour = Some(hour);
    let phase: DayPhase = sources.transition.phase(hour);
    if night.0 != phase.night {
        night.0 = phase.night;
    }
    // `MW_NO_WORLD_LIGHTS=1`: A/B switch for screenshots and benchmarks.
    let off = std::env::var_os("MW_NO_WORLD_LIGHTS").is_some();
    let lights: Vec<GpuWorldLight> = if off {
        Vec::new()
    } else {
        sources.lights.iter().filter_map(|l| gpu_light(l, phase.light_level(&l.def, l.stagger))).collect()
    };
    if !sources.lights.is_empty() {
        info!(
            "world lights at {hour:.2} h: {} of {} on (night level {:.2}{})",
            lights.len(),
            sources.lights.len(),
            phase.night,
            if phase.day { ", day" } else { "" }
        );
    }
    list.lights = Arc::new(lights);
    list.generation += 1;
}

/// Push the night level into the materials that read it (lit windows, illuminated signs), when it changes
/// and when materials have been added.
fn apply_night_level(
    night: Res<NightLevel>,
    loaded: Res<crate::Loaded>,
    mut materials: ResMut<Assets<crate::materials::GameMaterial>>,
    mut seen: Local<(usize, Option<f32>)>,
) {
    if *seen == (loaded.materials.len(), Some(night.0)) {
        return;
    }
    *seen = (loaded.materials.len(), Some(night.0));
    for (look, handle) in &loaded.materials {
        if look.uses_night()
            && materials.get(handle).is_some_and(|m| m.extension.params.night.x != night.0)
            && let Some(mut m) = materials.get_mut(handle)
        {
            m.extension.params.night.x = night.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mw_assets::lights::{FlashPattern, LightDef};
    use mw_bundle::ResourceId;

    fn light(shape: LightShape) -> WorldLight {
        WorldLight {
            def: Arc::new(LightDef {
                id: ResourceId(1),
                shape,
                colour: [1.0, 0.5, 0.25],
                intensity: 30.0,
                attenuation_threshold: 0.3,
                on_day: false,
                on_night: true,
                on_twilight: true,
                on_rain: false,
                on_wet: false,
                flash: FlashPattern::default(),
            }),
            position: [1.0, 2.0, 3.0],
            direction: [0.0, -1.0, 0.0],
            object: None,
            stagger: None,
        }
    }

    #[test]
    fn point_light_constants() {
        let g = gpu_light(&light(LightShape::Point), 1.0).unwrap();
        // Range sqrt(30 / 0.3) = 10 m; the shader's cutoff is threshold / max channel.
        assert!((g.position.w - 10.0).abs() < 1e-4);
        assert!((g.colour.w - 0.01).abs() < 1e-6);
        assert_eq!(g.cone.y, 0.0);
        assert!(gpu_light(&light(LightShape::Point), 0.0).is_none());
    }

    #[test]
    fn spot_cone_is_quantised_at_the_edge_only() {
        let g = gpu_light(&light(LightShape::Spot { angle: 61.033, inner: 11.989 }), 1.0).unwrap();
        assert!((g.direction.w - 62f32.to_radians().cos()).abs() < 1e-6);
        let scale = 1.0 / (11.989f32.to_radians().cos() - 62f32.to_radians().cos());
        assert!((g.cone.x - scale).abs() < 1e-4);
        assert_eq!(g.direction.truncate(), Vec3::Y);
    }
}
