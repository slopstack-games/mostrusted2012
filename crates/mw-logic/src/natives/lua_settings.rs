//! Lua natives of the options screens: the settings manager SM = (G+0x2f3e0)+0x4210 (G = `*0x01177868`, reached
//! from the natives as WM+0x2020) and the post-FX list PFX = (G+0x2f3e0)+0x5ac8.
//!
//! SM keeps its categories sorted by id (ids at +0x8d8, count +0x8d4, category objects at +0x8f0; lookup
//! `FUN_0087eb90`, a lower bound). Every native goes category (by id) → vtable slot 4 (`setting(index)`) →
//! one virtual of the setting; [`Setting`] is that vtable, slot by slot. The concrete setting classes (audio
//! levels, display modes, controls, ...) are the original's own C++ and not ported yet; [`ValueSetting`] is a
//! plain value with a range (model decision, to be replaced class by class). A category id that is not
//! registered dereferences null in the original; here the getters answer 0 / "" and the setters do nothing.

use mw_lua::{Lua, Value};

use super::lua_args::{fit, ftol32, int, push_str, push_uint, tonumber};
use super::{LogicState, Registry, Request};

type R = mw_lua::Result<Vec<Value>>;

/// A setting's vtable (slots counted from 0; byte offsets as in the dump).
pub trait Setting: std::fmt::Debug {
    /// Slot 2 (+0x08): the value as 0..1, for sliders (`GetNormalisedSettingsValue`).
    fn normalised(&self) -> f32;
    /// Slot 3 (+0x0c).
    fn value(&self) -> u32;
    /// Slot 4 (+0x10): the value's display text.
    fn value_name(&self) -> String;
    /// Slot 5 (+0x14).
    fn set_value(&mut self, v: u32);
    /// Slot 6 (+0x18).
    fn max(&self) -> u32;
    /// Slot 7 (+0x1c).
    fn min(&self) -> u32;
    /// Slot 8 (+0x20): keep the current value (`SaveSettings`; the manager then clears the setting's byte +0x14).
    fn commit(&mut self);
    /// Slot 9 (+0x24): back to the kept value.
    fn revert(&mut self);
    /// Slot 10 (+0x28): `ActionSetting` (e.g. start a key binding, apply a display mode).
    fn action(&mut self) -> Option<Request>;
    /// Slot 11 (+0x2c).
    fn on_action_success(&mut self);
    /// Slot 12 (+0x30).
    fn on_action_failure(&mut self);
}

/// A plain value in [min, max] with optional display names (model decision: stands in for the original's
/// setting classes until they are ported).
#[derive(Debug, Clone, Default)]
pub struct ValueSetting {
    pub value: u32,
    pub saved: u32,
    pub min: u32,
    pub max: u32,
    /// Display text per value from `min`; empty: the number.
    pub names: Vec<String>,
    /// The byte +0x14 `SaveSettings` clears.
    pub changed: bool,
}

impl Setting for ValueSetting {
    fn normalised(&self) -> f32 {
        if self.max > self.min { (self.value.saturating_sub(self.min)) as f32 / (self.max - self.min) as f32 } else { 0.0 }
    }
    fn value(&self) -> u32 {
        self.value
    }
    fn value_name(&self) -> String {
        let i = self.value.wrapping_sub(self.min) as usize;
        self.names.get(i).cloned().unwrap_or_else(|| self.value.to_string())
    }
    fn set_value(&mut self, v: u32) {
        self.value = v;
        self.changed = true;
    }
    fn max(&self) -> u32 {
        self.max
    }
    fn min(&self) -> u32 {
        self.min
    }
    fn commit(&mut self) {
        self.saved = self.value;
        self.changed = false;
    }
    fn revert(&mut self) {
        self.value = self.saved;
        self.changed = false;
    }
    fn action(&mut self) -> Option<Request> {
        None
    }
    fn on_action_success(&mut self) {}
    fn on_action_failure(&mut self) {}
}

/// A settings category (its vtable: slot 2 `is_valid(index)`, slot 4 `setting(index)`, slot 5 after a save,
/// slots 7 / 9 the begin / end of its settings).
#[derive(Debug, Default)]
pub struct SettingsCategory {
    pub id: i32,
    pub settings: Vec<Box<dyn Setting>>,
}

/// The settings manager SM.
#[derive(Debug, Default)]
pub struct SettingsManager {
    /// +0x8d4 / +0x8d8 / +0x8f0, sorted by id.
    pub categories: Vec<SettingsCategory>,
}

impl SettingsManager {
    /// Add (or replace) a category, keeping the order by id.
    pub fn insert(&mut self, category: SettingsCategory) {
        match self.categories.binary_search_by_key(&category.id, |c| c.id) {
            Ok(i) => self.categories[i] = category,
            Err(i) => self.categories.insert(i, category),
        }
    }

    pub fn category_mut(&mut self, id: i32) -> Option<&mut SettingsCategory> {
        let i = self.categories.binary_search_by_key(&id, |c| c.id).ok()?;
        Some(&mut self.categories[i])
    }

    /// Category `id` → slot 4 `setting(index)`.
    pub fn setting_mut(&mut self, id: i32, index: i32) -> Option<&mut Box<dyn Setting>> {
        let c = self.category_mut(id)?;
        usize::try_from(index).ok().and_then(|i| c.settings.get_mut(i))
    }
}

/// One post-FX setting (0x40-byte records from PFX+4, count +0x3c4; `FUN_0086ece0`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PostFxSetting {
    /// +0x28 (`RegisterPostFXSetting` argument 1).
    pub id: i32,
    /// +0x2c.
    pub min: i32,
    /// +0x30.
    pub max: i32,
    /// +0x34.
    pub value: i32,
    /// +0x38: the registered value.
    pub default: i32,
    /// +0x3c: what `ChangePostFXSetting` looks the record up by; 3 takes its value from the renderer.
    pub key: i32,
}

/// The post-FX list PFX.
#[derive(Debug, Default)]
pub struct PostFxSettings {
    pub settings: Vec<PostFxSetting>,
    /// The renderer's float that key 3 starts from (`FUN_00897ac0` → `FUN_008e0e30`: the render settings'
    /// +0x1400); the runtime keeps it current.
    pub render_value: f32,
}

fn cat_index(args: &[Value]) -> (i32, i32) {
    (int(args, 1), int(args, 2))
}

/// `GetSettingsValue(category, index)` (0x007eac70 → `FUN_0087ae60` → slot 3), pushed unsigned.
fn get_settings_value(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (c, i) = cat_index(&args);
    Ok(vec![push_uint(state.settings.setting_mut(c, i).map_or(0, |s| s.value()))])
}

/// `GetSettingsMinValue(category, index)` (0x007eae60 → `FUN_0087b050` → slot 7), unsigned.
fn get_settings_min_value(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (c, i) = cat_index(&args);
    Ok(vec![push_uint(state.settings.setting_mut(c, i).map_or(0, |s| s.min()))])
}

/// `GetSettingsMaxValue(category, index)` (0x007eaee0 → `FUN_0087b0c0` → slot 6), unsigned.
fn get_settings_max_value(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (c, i) = cat_index(&args);
    Ok(vec![push_uint(state.settings.setting_mut(c, i).map_or(0, |s| s.max()))])
}

/// `GetNormalisedSettingsValue(category, index)` (0x007ead80 → `FUN_0087af40` → slot 2, a float).
fn get_normalised_settings_value(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (c, i) = cat_index(&args);
    let v = state.settings.setting_mut(c, i).map_or(0.0, |s| s.normalised());
    Ok(vec![Value::Number(v as f64)])
}

/// `GetSettingsValueName(category, index)` (0x007eacf0 → `FUN_0087aed0` → slot 4, copied into 0x3ff bytes).
fn get_settings_value_name(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (c, i) = cat_index(&args);
    let s = state.settings.setting_mut(c, i).map(|s| s.value_name()).unwrap_or_default();
    Ok(vec![push_str(fit(&s, 0x3ff))])
}

/// `IsSettingValid(category, index)` (0x007eadf0 → `FUN_0087afb0`): false for an unknown category, else the
/// category's slot 2 (here: the index names a setting).
fn is_setting_valid(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (c, i) = cat_index(&args);
    Ok(vec![Value::Bool(state.settings.setting_mut(c, i).is_some())])
}

/// `SetSettingsValue(category, index, value)` (0x007eaf60 → `FUN_0087b130` → slot 5).
fn set_settings_value(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (c, i) = cat_index(&args);
    let v = int(&args, 3) as u32;
    if let Some(s) = state.settings.setting_mut(c, i) {
        s.set_value(v);
    }
    Ok(vec![])
}

/// `ActionSetting(category, index)` (0x007eafc0 → `FUN_0087b1a0` → slot 10).
fn action_setting(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (c, i) = cat_index(&args);
    if let Some(req) = state.settings.setting_mut(c, i).and_then(|s| s.action()) {
        state.requests.push(req);
    }
    Ok(vec![])
}

/// `OnActionSettingSuccess(category, index)` (0x007eb010 → `FUN_0087b210` → slot 11).
fn on_action_setting_success(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (c, i) = cat_index(&args);
    if let Some(s) = state.settings.setting_mut(c, i) {
        s.on_action_success();
    }
    Ok(vec![])
}

/// `OnActionSettingFailure(category, index)` (0x007eb060 → `FUN_0087b280` → slot 12).
fn on_action_setting_failure(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (c, i) = cat_index(&args);
    if let Some(s) = state.settings.setting_mut(c, i) {
        s.on_action_failure();
    }
    Ok(vec![])
}

/// `RevertSetting(category, index)` (0x007eb0b0 → `FUN_0087b2f0` → slot 9).
fn revert_setting(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (c, i) = cat_index(&args);
    if let Some(s) = state.settings.setting_mut(c, i) {
        s.revert();
    }
    Ok(vec![])
}

/// `RevertCategorySettings(category)` (0x007eb100 → `FUN_0087b360`: slot 9 of each of its settings).
fn revert_category_settings(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    if let Some(c) = state.settings.category_mut(int(&args, 1)) {
        c.settings.iter_mut().for_each(|s| s.revert());
    }
    Ok(vec![])
}

/// `RevertSettings()` (0x007eb1d0 → `FUN_0086fec0`: slot 9 of every setting of every category).
fn revert_settings(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    for c in &mut state.settings.categories {
        c.settings.iter_mut().for_each(|s| s.revert());
    }
    Ok(vec![])
}

/// `SaveSettings()` (0x007eb1a0 → `FUN_0086fe20`): slot 8 of every setting (and its byte +0x14 cleared), the
/// category's slot 5, then `FUN_0071c1c0`, `FUN_0046efe0`, `FUN_00414e50`: [`Request::SaveSettings`].
fn save_settings(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    for c in &mut state.settings.categories {
        c.settings.iter_mut().for_each(|s| s.commit());
    }
    state.requests.push(Request::SaveSettings);
    Ok(vec![])
}

/// `RegisterPostFXSetting(id, min, max, value, key)` (0x007e9ff0 → `FUN_0086ece0`): argument 1 through `_ftol2`,
/// 2–5 through `(int)` casts (truncating). Appends a record whose value and default are `value`; key 3 starts at
/// `(int)(renderer float × max)` (max as unsigned; `fistp` rounding to nearest).
fn register_post_fx_setting(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = int(&args, 1);
    let min = ftol32(tonumber(&args, 2));
    let max = ftol32(tonumber(&args, 3));
    let value = ftol32(tonumber(&args, 4));
    let key = ftol32(tonumber(&args, 5));
    let mut s = PostFxSetting { id, min, max, value, default: value, key };
    if key == 3 {
        let v = (state.post_fx.render_value as f64 * max as u32 as f64).round_ties_even();
        s.value = if v.is_nan() || v.abs() >= 9.2e18 { 0 } else { v as i64 as i32 };
    }
    state.post_fx.settings.push(s);
    Ok(vec![])
}

/// `ChangePostFXSetting(key, delta)` (0x007e9f80 → `FUN_00872500`): the first record with that key; the value
/// + delta is kept unless it goes above max (delta > 0) or below min (delta ≤ 0). An unknown key (a null
/// dereference in the original) does nothing.
fn change_post_fx_setting(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let key = ftol32(tonumber(&args, 1));
    let delta = int(&args, 2);
    if let Some(s) = state.post_fx.settings.iter_mut().find(|s| s.key == key) {
        let v = s.value.wrapping_add(delta);
        let ok = if delta < 1 { v >= s.min } else { v <= s.max };
        if ok {
            s.value = v;
        }
    }
    Ok(vec![])
}

/// `ResetPostFXSettings()` (0x007ea0f0 → `FUN_00872550`): every record cleared, the list emptied.
fn reset_post_fx_settings(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.post_fx.settings.clear();
    Ok(vec![])
}

pub fn register(r: &mut Registry) {
    r.set_lua("GetSettingsValue", get_settings_value);
    r.set_lua("GetSettingsMinValue", get_settings_min_value);
    r.set_lua("GetSettingsMaxValue", get_settings_max_value);
    r.set_lua("GetNormalisedSettingsValue", get_normalised_settings_value);
    r.set_lua("GetSettingsValueName", get_settings_value_name);
    r.set_lua("IsSettingValid", is_setting_valid);
    r.set_lua("SetSettingsValue", set_settings_value);
    r.set_lua("ActionSetting", action_setting);
    r.set_lua("OnActionSettingSuccess", on_action_setting_success);
    r.set_lua("OnActionSettingFailure", on_action_setting_failure);
    r.set_lua("RevertSetting", revert_setting);
    r.set_lua("RevertCategorySettings", revert_category_settings);
    r.set_lua("RevertSettings", revert_settings);
    r.set_lua("SaveSettings", save_settings);
    r.set_lua("RegisterPostFXSetting", register_post_fx_setting);
    r.set_lua("ChangePostFXSetting", change_post_fx_setting);
    r.set_lua("ResetPostFXSettings", reset_post_fx_settings);
}
