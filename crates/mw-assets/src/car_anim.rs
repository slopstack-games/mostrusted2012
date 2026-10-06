//! Animated body parts of a car (active aero: spoilers, air brakes, flaps, vents).
//!
//! The data (docs/formats.md, "Vehicle animations"): a car's `Genesys.Vehicles.AnimationStateBehaviour`
//! names an `HSMData` state machine (field `0501d88b`) and the `Animation` clips it plays (`Resources`). Each
//! clip moves a few bones of the car's damage skeleton between a retracted pose (frame 0) and a deployed one
//! (last frame) and plays at its own frame rate. The state machine's conditions read the variables
//! `speedinmph`, `brake`, `steering` and `gas`; its states name a clip by its resource id (in decimal).
//!
//! The HSM layout is only partly decoded: this module reads its strings. The common case, one clip and one
//! `speedinmph` condition without timers (911, LFA, SLS, SL65, Panamera, 918), is turned into a
//! [`SpeedRule`]: deploy at the condition's speed, retract below the other speed operand of the machine (or
//! the same speed if there is none). Other machines (Veyron, McLaren and Venom air brakes, Huayra flaps,
//! vents with timers, the Agera's `gas`) are listed but not run.

use mw_bundle::{Bundle, ResourceId, ResourceType};
use mw_formats::Animation;
use mw_formats::genesys::Value;

use crate::{AssetStore, Result};

/// `CgsCore.StringBase` at +12.
const LABEL: u32 = 0xd11fce34;
const TYPE_NAME: &str = "Genesys.Vehicles.AnimationStateBehaviour";
/// `CgsResource.Handle` at +28: the `HSMData`.
const HSM: u32 = 0x0501d88b;
/// `CgsResource.Handle[]` at +36: the clips.
const RESOURCES: u32 = 0x3f227e59;
/// Variables the state machines read (strings next to each other in the exe, `.rdata` 0x00fae7e0..).
pub const VARIABLES: [&str; 4] = ["speedinmph", "brake", "steering", "gas"];

/// Deploy / retract speeds of a one-clip speed machine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeedRule {
    /// Deploy at or above this speed (mph).
    pub deploy_mph: f32,
    /// Retract below this speed (mph).
    pub retract_mph: f32,
}

#[derive(Debug, Clone)]
pub struct AnimClip {
    pub id: ResourceId,
    pub animation: Animation,
}

impl AnimClip {
    /// Seconds from the first to the last frame at the clip's frame rate.
    pub fn duration(&self) -> f32 {
        (self.animation.frames.len().saturating_sub(1)) as f32 / self.animation.frame_rate.max(1e-3)
    }
}

#[derive(Debug, Clone)]
pub struct CarAnimations {
    /// The behaviour's label (e.g. `Porsche_911CarreraS_2012_SpoilerBehaviour`).
    pub label: String,
    pub hsm: Option<ResourceId>,
    pub clips: Vec<AnimClip>,
    /// Printable strings of the state machine (state / condition names, operands), in file order.
    pub hsm_strings: Vec<String>,
    /// Variables the state machine reads.
    pub variables: Vec<&'static str>,
    /// The speed rule, when the machine is the simple one-clip speed form.
    pub speed_rule: Option<SpeedRule>,
}

/// NUL-terminated printable strings of at least two characters.
fn strings(data: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, &b) in data.iter().enumerate() {
        if b == 0 {
            if i >= start + 2
                && let Ok(s) = std::str::from_utf8(&data[start..i])
                && s.bytes().all(|c| (0x20..0x7f).contains(&c))
            {
                out.push(s.trim().to_string());
            }
            start = i + 1;
        } else if !(0x20..0x7f).contains(&b) {
            start = i + 1;
        }
    }
    out
}

/// First number in a condition label such as `Speed above 70mph` or `Speed 74mph`.
fn label_speed(s: &str) -> Option<f32> {
    let digits: String = s.chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn speed_rule(strings: &[String], variables: &[&str], clips: usize) -> Option<SpeedRule> {
    if clips != 1 || variables != ["speedinmph"] || strings.iter().any(|s| s.starts_with("Wait")) {
        return None;
    }
    let deploy = strings.iter().filter(|s| s.starts_with("Speed")).find_map(|s| label_speed(s))?;
    // Bare operands below 1000 mph (larger numbers are clip ids).
    let retract = strings
        .iter()
        .filter_map(|s| s.parse::<f32>().ok())
        .filter(|&v| v < deploy && v >= 1.0 && v.fract() == 0.0)
        .fold(deploy, f32::min);
    Some(SpeedRule { deploy_mph: deploy, retract_mph: retract })
}

impl AssetStore {
    /// The animated body parts of a vehicle bundle (`_HI`), if it has an `AnimationStateBehaviour`.
    pub fn car_animations(&self, bundle: &Bundle) -> Result<Option<CarAnimations>> {
        for e in bundle.entries_of_type(ResourceType::GenesysObject) {
            let Ok(obj) = self.decode_genesys(&bundle.load(e)?) else { continue };
            if obj.type_name != TYPE_NAME {
                continue;
            }
            let hsm = match obj.get(HSM) {
                Some(Value::Handle(Some(id))) => Some(*id),
                _ => None,
            };
            let mut clips = Vec::new();
            if let Some(Value::Array(items)) = obj.get(RESOURCES) {
                for item in items {
                    if let Value::Handle(Some(id)) = item
                        && let Ok(res) = self.load(*id)
                        && let Ok(animation) = Animation::parse(&res)
                    {
                        clips.push(AnimClip { id: *id, animation });
                    }
                }
            }
            let hsm_strings = match hsm.map(|id| self.load(id)) {
                Some(Ok(res)) => strings(&res.chunks[0]),
                _ => Vec::new(),
            };
            let variables: Vec<&'static str> =
                VARIABLES.into_iter().filter(|v| hsm_strings.iter().any(|s| s == v)).collect();
            let speed_rule = speed_rule(&hsm_strings, &variables, clips.len());
            let label = match obj.get(LABEL) {
                Some(Value::Str(s)) => s.clone(),
                _ => String::new(),
            };
            return Ok(Some(CarAnimations { label, hsm, clips, hsm_strings, variables, speed_rule }));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_speed_machine() {
        let s: Vec<String> = ["Speed above 70mph", "Spoiler Anim", "904078", "40", "speedinmph", "False", "0.25", "70"]
            .map(String::from)
            .into();
        assert_eq!(speed_rule(&s, &["speedinmph"], 1), Some(SpeedRule { deploy_mph: 70.0, retract_mph: 40.0 }));
        assert_eq!(speed_rule(&s, &["speedinmph", "brake"], 1), None);
    }
}
