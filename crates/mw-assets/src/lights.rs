//! World lights: the `LightInstanceList` placements of a track unit with their GeneSys `Light_*`
//! definitions, and the day / night switch of `EnvironmentSettings.DayNightTransition`.
//!
//! How the game uses them (docs/decomp.md "World lights"): each frame the lights in range are submitted to
//! the renderer by type (`FUN_0054ade0`) with their position (the instance matrix's translation), the
//! matrix's z axis as the beam direction and `Colour × intensity`; the renderer turns them into the
//! `LightBuffer_Point` / `_Cone` / `_Spot` constants (`FUN_008b0940`, `FUN_008af930`, `FUN_008afee0`).
//!
//! Street lamps are not in the light lists: they are compound objects (and a few world objects) whose
//! `WCVfxBehaviour` ("SetLightsOn") names a light definition and a locator group of the object
//! ([`AssetStore::object_lights`]); one light sits at each locator of the group.

use std::collections::HashMap;
use std::sync::Arc;

use mw_bundle::{Bundle, ResourceId, ResourceType};
use mw_formats as fmt;
use mw_formats::Locator;
use mw_formats::genesys::{Object, Value, field_hash};

use crate::{AssetStore, Result};

/// What a light definition is, by its GeneSys type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LightShape {
    /// `Light_Point`: all directions.
    Point,
    /// `Light_Cone`: `ConeAngle` (degrees from the axis); the light falls off linearly in the cosine from
    /// the axis to the edge.
    Cone { angle: f32 },
    /// `Light_Spot`: `ConeAngle` (the edge) and `ConeInnerAngle` (full brightness inside), degrees from the
    /// axis.
    Spot { angle: f32, inner: f32 },
    /// `Light_Line`: a segment of `HalfLength` metres either way along the axis (not drawn yet).
    Line { half_length: f32 },
    /// `Light_Custom`, `Light_ProjectedTexture` (not drawn).
    Other,
}

impl LightShape {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Point => "point",
            Self::Cone { .. } => "cone",
            Self::Spot { .. } => "spot",
            Self::Line { .. } => "line",
            Self::Other => "other",
        }
    }
}

/// `Light_Base.FlashPattern`. `Type` 0 on steady lights (most definitions), 2 on blinking ones (beacons,
/// signs), 1 once. How the game evaluates it is not traced.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FlashPattern {
    pub kind: u32,
    pub duration: f32,
    pub frequency: f32,
    pub offset: f32,
    /// `3ac4f0ce` (0–1, unnamed).
    pub unknown_3ac4f0ce: f32,
}

/// A GeneSys `Light_Point` / `Light_Cone` / `Light_Spot` / `Light_Line` (all extend `Light_Base`).
#[derive(Debug, Clone, PartialEq)]
pub struct LightDef {
    pub id: ResourceId,
    pub shape: LightShape,
    /// `Colour` (linear rgb, 0–1).
    pub colour: [f32; 3],
    /// `Intensity`: scales the colour (the game's HDR units at 1 m: the light shaders multiply by 1 / d²).
    pub intensity: f32,
    /// `AttenuationThreshold`: the brightness below which the light is cut off. The shaders compute
    /// `saturate(1 / d² − threshold / max(colour × intensity))`, so the light ends at
    /// `sqrt(max(colour × intensity) / threshold)` metres ([`LightDef::range`]).
    pub attenuation_threshold: f32,
    /// `OnDuringDay` / `OnDuringNight` / `OnDuringTwilight` / `OnDuringRain` / `OnDuringWet`.
    pub on_day: bool,
    pub on_night: bool,
    pub on_twilight: bool,
    pub on_rain: bool,
    pub on_wet: bool,
    pub flash: FlashPattern,
}

fn float(o: &Object, hash: u32) -> Option<f32> {
    match o.get(hash)? {
        Value::Float(v) => Some(*v),
        Value::Int(v) => Some(*v as f32),
        Value::UInt(v) => Some(*v as f32),
        _ => None,
    }
}

impl LightDef {
    pub fn from_object(id: ResourceId, o: &Object) -> Option<Self> {
        let h = field_hash;
        let f = |name: &str| float(o, h(name)).unwrap_or(0.0);
        let b = |name: &str| matches!(o.get(h(name)), Some(Value::Bool(true)));
        let shape = match o.type_name.as_str() {
            "Genesys.Gen.Light_Point" => LightShape::Point,
            "Genesys.Gen.Light_Cone" => LightShape::Cone { angle: f("ConeAngle") },
            "Genesys.Gen.Light_Spot" => LightShape::Spot { angle: f("ConeAngle"), inner: f("ConeInnerAngle") },
            "Genesys.Gen.Light_Line" => LightShape::Line { half_length: f("HalfLength") },
            "Genesys.Gen.Light_Custom" | "Genesys.Gen.Light_ProjectedTexture" => LightShape::Other,
            _ => return None,
        };
        let colour = match o.get(h("Colour")) {
            Some(Value::Vector(v)) => [v[0], v[1], v[2]],
            _ => [1.0; 3],
        };
        let flash = match o.get(h("FlashPattern")) {
            Some(Value::Struct(s)) => FlashPattern {
                kind: float(s, h("Type")).unwrap_or(0.0) as u32,
                duration: float(s, h("Duration")).unwrap_or(0.0),
                frequency: float(s, h("Frequency")).unwrap_or(0.0),
                offset: float(s, h("Offset")).unwrap_or(0.0),
                unknown_3ac4f0ce: float(s, 0x3ac4_f0ce).unwrap_or(0.0),
            },
            _ => FlashPattern::default(),
        };
        Some(Self {
            id,
            shape,
            colour,
            intensity: f("Intensity"),
            attenuation_threshold: f("AttenuationThreshold"),
            on_day: b("OnDuringDay"),
            on_night: b("OnDuringNight"),
            on_twilight: b("OnDuringTwilight"),
            on_rain: b("OnDuringRain"),
            on_wet: b("OnDuringWet"),
            flash,
        })
    }

    /// `Colour × Intensity`: the light's colour in the game's HDR units at 1 m.
    pub fn radiance(&self) -> [f32; 3] {
        self.colour.map(|c| c * self.intensity)
    }

    /// Where the light ends (metres): `sqrt(max channel / threshold)`, the radius the game scales the
    /// light's volume mesh by (`FUN_008b0940`). A threshold below 0.001 is replaced by a renderer default
    /// in the game (`FUN_008ce920`; a runtime value, not read: the data's lowest threshold is 0.05).
    pub fn range(&self) -> f32 {
        let m = self.radiance().into_iter().fold(0.0, f32::max);
        (m / self.attenuation_threshold.max(1e-3)).sqrt()
    }
}

/// A light placed in the world.
#[derive(Debug, Clone)]
pub struct WorldLight {
    pub def: Arc<LightDef>,
    pub position: [f32; 3],
    /// The instance matrix's z axis (row 2), normalised: the beam direction of cone and spot lights, the
    /// axis of line lights.
    pub direction: [f32; 3],
    /// Set for a light that an object carries (a street lamp's): the object's definition.
    pub object: Option<ResourceId>,
}

/// A light an object carries: its definition and where it sits on the object.
#[derive(Debug, Clone)]
pub struct ObjectLight {
    pub def: Arc<LightDef>,
    pub locator: Locator,
}

/// The z axis of a locator's rotation (quaternion x, y, z, w).
fn locator_axis(q: [f32; 4]) -> [f32; 3] {
    let [x, y, z, w] = q;
    [2.0 * (x * z + w * y), 2.0 * (y * z - w * x), 1.0 - 2.0 * (x * x + y * y)]
}

fn normalised(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    v.map(|c| c / len)
}

/// `EnvironmentSettings.DayNightTransition`: when the world's lights (and the shadows) change between
/// their day and night states. Hours on the 24-hour clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DayNightTransition {
    /// `1ab46d30` (8.5) and `0e2be130` (20): read as the hours the night ends and starts, from their
    /// values, their place between `LightTransitionOffset` and `ShadowTransitionDuration` in the name-sorted
    /// field order, and `SunMoonPath`'s moon hours (20 to 8). Names not cracked.
    pub night_end: f32,
    pub night_start: f32,
    /// `LightTransitionDuration` (1) and `LightTransitionOffset` (0), hours.
    pub light_duration: f32,
    pub light_offset: f32,
    /// `ShadowTransitionDuration` (1) and `ShadowTransitionOffset` (0), hours.
    pub shadow_duration: f32,
    pub shadow_offset: f32,
}

impl Default for DayNightTransition {
    /// The values of the game's one `EnvironmentSettings` object.
    fn default() -> Self {
        Self {
            night_end: 8.5,
            night_start: 20.0,
            light_duration: 1.0,
            light_offset: 0.0,
            shadow_duration: 1.0,
            shadow_offset: 0.0,
        }
    }
}

/// Day, night or the change between them, as the lights' `OnDuring…` flags name them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DayPhase {
    /// 0 by day, 1 at night, in between during a light transition.
    pub night: f32,
    /// Inside a light transition.
    pub twilight: bool,
}

impl DayNightTransition {
    pub fn from_settings(settings: &Object) -> Self {
        let d = Self::default();
        let Some(Value::Struct(s)) = settings.get(field_hash("DayNightTransition")) else { return d };
        let f = |hash: u32, or: f32| float(s, hash).unwrap_or(or);
        Self {
            night_end: f(0x1ab4_6d30, d.night_end),
            night_start: f(0x0e2b_e130, d.night_start),
            light_duration: f(field_hash("LightTransitionDuration"), d.light_duration),
            light_offset: f(field_hash("LightTransitionOffset"), d.light_offset),
            shadow_duration: f(field_hash("ShadowTransitionDuration"), d.shadow_duration),
            shadow_offset: f(field_hash("ShadowTransitionOffset"), d.shadow_offset),
        }
    }

    /// The phase at an hour (0–24).
    ///
    /// **Model decision** (the code that reads these fields was not found in the dump; docs/decomp.md
    /// "World lights"): the lights' transition is a linear ramp of `LightTransitionDuration` hours that
    /// starts `LightTransitionOffset` hours after each of the two boundary hours, so with the data's values
    /// the lights come on from 20:00 to 21:00 and go off from 08:30 to 09:30.
    pub fn phase(&self, hour: f32) -> DayPhase {
        let hour = hour.rem_euclid(24.0);
        let d = self.light_duration.max(1e-3);
        // Hours since each boundary, wrapping; the one passed most recently decides.
        let since = |boundary: f32| (hour - boundary - self.light_offset).rem_euclid(24.0);
        let (up, down) = (since(self.night_start), since(self.night_end));
        if up <= down {
            DayPhase { night: (up / d).min(1.0), twilight: up < d }
        } else {
            DayPhase { night: 1.0 - (down / d).min(1.0), twilight: down < d }
        }
    }
}

impl DayPhase {
    /// How bright a light with these `OnDuring…` flags is (0–1).
    ///
    /// **Model decision**, as [`DayNightTransition::phase`]: outside a transition a light is on or off by
    /// `OnDuringDay` / `OnDuringNight`; inside one, `OnDuringTwilight` holds a light that is on at either
    /// end fully on, and a light without it follows the ramp between its day and night states.
    pub fn light_level(&self, def: &LightDef) -> f32 {
        let (day, night) = (def.on_day as u8 as f32, def.on_night as u8 as f32);
        if self.twilight && def.on_twilight {
            return day.max(night);
        }
        day + (night - day) * self.night
    }
}

impl AssetStore {
    pub fn day_night_transition(&self) -> Result<DayNightTransition> {
        let settings = self.decode_genesys(&self.load(crate::environment::SETTINGS)?)?;
        Ok(DayNightTransition::from_settings(&settings))
    }

    /// The lights an object definition carries through its `WCVfxBehaviour`s (the object's own and, for a
    /// compound object, its parts'): each `Lights` entry is a light definition and a `LocatorGroup`, looked
    /// up among the part's locator groups and then the object's; one light per locator.
    ///
    /// Not modelled: what switches such a behaviour ("SetLightsOn") on and off in the game (the lights
    /// are taken as on, subject to their definition's `OnDuring…` flags), and the bone pose of a part's own
    /// locators (taken as object space; the street lamps' groups are object-level).
    pub fn object_lights(&self, def: &fmt::ObjectDef) -> Vec<ObjectLight> {
        let mut out = Vec::new();
        let no_part: &[fmt::LocatorGroup] = &[];
        let parts =
            def.compound_parts.iter().enumerate().map(|(i, p)| {
                (p.behaviours.as_slice(), def.part_locator_groups.get(i).map_or(no_part, |g| g.as_slice()))
            });
        for (behaviours, own) in std::iter::once((def.behaviours.as_slice(), no_part)).chain(parts) {
            for b in behaviours.iter().filter(|b| b.type_id == fmt::WC_VFX_BEHAVIOUR) {
                let Some(o) = b.object.and_then(|id| self.genesys_object(id).ok()) else { continue };
                let Some(Value::Array(lights)) = o.get(field_hash("Lights")) else { continue };
                for l in lights {
                    let Value::Struct(l) = l else { continue };
                    let Some(Value::Handle(Some(id))) = l.get(field_hash("LightDefinition")) else { continue };
                    let group = float(l, field_hash("LocatorGroup")).unwrap_or(0.0) as u32;
                    let Some(light) = self.genesys_object(*id).ok().and_then(|o| LightDef::from_object(*id, &o)) else {
                        continue;
                    };
                    let light = Arc::new(light);
                    let Some(group) = own.iter().chain(&def.locator_groups).find(|g| g.id == group) else { continue };
                    out.extend(group.locators.iter().map(|&locator| ObjectLight { def: light.clone(), locator }));
                }
            }
        }
        out
    }

    /// The lights of a bundle: those its `LightInstanceList`s place, and those its compound and dynamic
    /// objects carry ([`Self::object_lights`]) at each placement. Instances whose definition is missing or
    /// not a light are skipped.
    pub fn bundle_lights(&self, bundle: &Bundle) -> Result<Vec<WorldLight>> {
        let mut defs: HashMap<ResourceId, Option<Arc<LightDef>>> = HashMap::new();
        let mut out = Vec::new();
        type ParseDef = fn(&mw_bundle::Resource) -> fmt::Result<fmt::ObjectDef>;
        let carriers: [(ResourceType, ParseDef); 2] = [
            (ResourceType::CompoundInstanceList, fmt::ObjectDef::parse_compound),
            (ResourceType::DynamicInstanceList, fmt::ObjectDef::parse_world_object),
        ];
        let mut carried: HashMap<ResourceId, Vec<ObjectLight>> = HashMap::new();
        for (ty, parse) in carriers {
            for e in bundle.entries_of_type(ty) {
                let list = fmt::ObjectInstanceList::parse(&bundle.load(e)?)?;
                for inst in list.instances {
                    let Some(object) = inst.object else { continue };
                    let lights = carried.entry(object).or_insert_with(|| {
                        let def = self.load(object).ok().and_then(|res| parse(&res).ok());
                        def.map_or(Vec::new(), |d| self.object_lights(&d))
                    });
                    let t = &inst.transform;
                    for l in lights.iter() {
                        let p = l.locator.position;
                        let a = locator_axis(l.locator.rotation);
                        // Row vectors: v × M.
                        let at = |c: usize| p[0] * t[0][c] + p[1] * t[1][c] + p[2] * t[2][c] + t[3][c];
                        let dir = |c: usize| a[0] * t[0][c] + a[1] * t[1][c] + a[2] * t[2][c];
                        out.push(WorldLight {
                            def: l.def.clone(),
                            position: [at(0), at(1), at(2)],
                            direction: normalised([dir(0), dir(1), dir(2)]),
                            object: Some(object),
                        });
                    }
                }
            }
        }
        for e in bundle.entries_of_type(ResourceType::LightInstanceList) {
            let list = fmt::ObjectInstanceList::parse(&bundle.load(e)?)?;
            for inst in list.instances {
                let Some(id) = inst.object else { continue };
                let def = defs.entry(id).or_insert_with(|| {
                    let o = self.genesys_object(id).ok()?;
                    LightDef::from_object(id, &o).map(Arc::new)
                });
                let Some(def) = def.clone() else { continue };
                let t = &inst.transform;
                out.push(WorldLight {
                    def,
                    position: [t[3][0], t[3][1], t[3][2]],
                    direction: normalised([t[2][0], t[2][1], t[2][2]]),
                    object: None,
                });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_follows_the_boundaries() {
        let t = DayNightTransition::default();
        assert_eq!(t.phase(13.0), DayPhase { night: 0.0, twilight: false });
        assert_eq!(t.phase(1.5), DayPhase { night: 1.0, twilight: false });
        assert_eq!(t.phase(23.0), DayPhase { night: 1.0, twilight: false });
        let dusk = t.phase(20.5);
        assert!(dusk.twilight && (dusk.night - 0.5).abs() < 1e-5);
        let dawn = t.phase(8.75);
        assert!(dawn.twilight && (dawn.night - 0.75).abs() < 1e-5);
        assert_eq!(t.phase(21.0), DayPhase { night: 1.0, twilight: false });
    }
}
