//! Audio volume settings: master, engine, effects and music.
//!
//! F2 (or gamepad Select) opens a small overlay: Up/Down picks a row, Left/Right changes it in 5 % steps.
//! Changes apply at once to the playing sounds and are saved to `$XDG_CONFIG_HOME/mostrusted/settings.cfg`
//! (`~/.config/...`); `--volume master=0.8,music=0.3` overrides the saved values for one run.

use bevy::audio::{AudioSinkPlayback, Volume};
use bevy::prelude::*;
use std::path::PathBuf;

/// Which volume slider a sound follows; added to each sound entity next to its `AudioPlayer`.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Engine,
    Sfx,
    Music,
}

impl Channel {
    /// Mix level of the channel at 100 % (balanced by ear before the settings existed).
    fn base(self) -> f32 {
        match self {
            Channel::Engine => 0.6,
            Channel::Sfx => 0.6,
            Channel::Music => 0.35,
        }
    }
}

/// Volumes, 0..1 each (the overall gain of a sound is base × channel × master).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct AudioSettings {
    pub master: f32,
    pub engine: f32,
    pub sfx: f32,
    pub music: f32,
    /// Every sound muted for now (not saved): the game's `MUTE_SOUND` behaviour with reason `LoadingScreen`
    /// while the boot flow's screens and the loading screen show (`frontend.rs`).
    pub muted: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self { master: 1.0, engine: 1.0, sfx: 1.0, music: 0.6, muted: false }
    }
}

const ROWS: [&str; 4] = ["Master", "Engine", "Effects", "Music"];
const STEP: f32 = 0.05;

impl AudioSettings {
    fn slot(&mut self, row: usize) -> &mut f32 {
        match row {
            0 => &mut self.master,
            1 => &mut self.engine,
            2 => &mut self.sfx,
            _ => &mut self.music,
        }
    }

    fn get(&self, row: usize) -> f32 {
        [self.master, self.engine, self.sfx, self.music][row.min(3)]
    }

    /// Linear gain for a channel.
    pub fn gain(&self, channel: Channel) -> f32 {
        let v = match channel {
            Channel::Engine => self.engine,
            Channel::Sfx => self.sfx,
            Channel::Music => self.music,
        };
        if self.muted {
            return 0.0;
        }
        channel.base() * v * self.master
    }

    /// Apply `key=value` pairs (`master`, `engine`, `sfx`/`effects`, `music`; values 0..1 or percentages).
    pub fn apply(&mut self, text: &str) {
        for pair in text.split([',', '\n']) {
            let Some((k, v)) = pair.split_once('=') else { continue };
            let Ok(mut v) = v.trim().trim_end_matches('%').parse::<f32>() else { continue };
            if v > 1.0 {
                v /= 100.0;
            }
            let v = v.clamp(0.0, 1.0);
            match k.trim().to_ascii_lowercase().as_str() {
                "master" => self.master = v,
                "engine" => self.engine = v,
                "sfx" | "effects" => self.sfx = v,
                "music" => self.music = v,
                _ => {}
            }
        }
    }

    fn to_text(&self) -> String {
        format!("master={:.2}\nengine={:.2}\nsfx={:.2}\nmusic={:.2}\n", self.master, self.engine, self.sfx, self.music)
    }
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("mostrusted").join("settings.cfg"))
}

/// The saved settings, then `overrides` (the `--volume` flag) on top.
pub fn load(overrides: Option<&str>) -> AudioSettings {
    let mut s = AudioSettings::default();
    if let Some(text) = config_path().and_then(|p| std::fs::read_to_string(p).ok()) {
        s.apply(&text);
    }
    if let Some(o) = overrides {
        s.apply(o);
    }
    s
}

fn save(s: &AudioSettings) {
    let Some(path) = config_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, s.to_text()) {
        warn!("could not save {}: {e}", path.display());
    }
}

pub struct SettingsPlugin {
    pub settings: AudioSettings,
    /// Show the overlay at startup (`--show-settings`, for screenshots).
    pub open: bool,
}

#[derive(Resource, Default)]
pub struct SettingsMenu {
    pub open: bool,
    row: usize,
    dirty: bool,
}

#[derive(Component)]
struct SettingsRoot;

#[derive(Component)]
struct SettingsText;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.settings.clone())
            .insert_resource(SettingsMenu { open: self.open, ..default() })
            .add_systems(Startup, spawn_overlay)
            .add_systems(Update, (menu_input, refresh_overlay, apply_volumes, freeze_car_input).chain());
    }
}

fn spawn_overlay(mut commands: Commands, menu: Res<SettingsMenu>) {
    commands
        .spawn((
            SettingsRoot,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(24.0),
                top: Val::Px(24.0),
                padding: UiRect::all(Val::Px(14.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.85)),
            if menu.open { Visibility::Inherited } else { Visibility::Hidden },
        ))
        .with_children(|p| {
            p.spawn((
                SettingsText,
                Text::new(""),
                TextFont { font_size: FontSize::Px(18.0), ..default() },
                TextColor(Color::WHITE),
            ));
        });
}

fn menu_input(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut menu: ResMut<SettingsMenu>,
    mut settings: ResMut<AudioSettings>,
) {
    let pad = |b: GamepadButton| pads.iter().any(|p| p.just_pressed(b));
    if keys.just_pressed(KeyCode::F2) || pad(GamepadButton::Select) {
        menu.open = !menu.open;
        if !menu.open && std::mem::take(&mut menu.dirty) {
            save(&settings);
        }
    }
    if !menu.open {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        menu.open = false;
        if std::mem::take(&mut menu.dirty) {
            save(&settings);
        }
        return;
    }
    if keys.just_pressed(KeyCode::ArrowUp) || pad(GamepadButton::DPadUp) {
        menu.row = (menu.row + ROWS.len() - 1) % ROWS.len();
    }
    if keys.just_pressed(KeyCode::ArrowDown) || pad(GamepadButton::DPadDown) {
        menu.row = (menu.row + 1) % ROWS.len();
    }
    let delta = if keys.just_pressed(KeyCode::ArrowRight) || pad(GamepadButton::DPadRight) {
        STEP
    } else if keys.just_pressed(KeyCode::ArrowLeft) || pad(GamepadButton::DPadLeft) {
        -STEP
    } else {
        0.0
    };
    if delta != 0.0 {
        let row = menu.row;
        let v = settings.slot(row);
        *v = ((*v + delta) / STEP).round() * STEP;
        *v = v.clamp(0.0, 1.0);
        menu.dirty = true;
        // Save as we go too, so a closed window keeps the change.
        save(&settings);
    }
}

fn refresh_overlay(
    menu: Res<SettingsMenu>,
    settings: Res<AudioSettings>,
    mut roots: Query<&mut Visibility, With<SettingsRoot>>,
    mut texts: Query<&mut Text, With<SettingsText>>,
) {
    if !menu.is_changed() && !settings.is_changed() {
        return;
    }
    for mut v in &mut roots {
        *v = if menu.open { Visibility::Inherited } else { Visibility::Hidden };
    }
    let mut out = String::from("Audio  (F2 / Select closes)\n\n");
    for (i, name) in ROWS.iter().enumerate() {
        let v = settings.get(i);
        let filled = (v * 20.0).round() as usize;
        let cursor = if i == menu.row { ">" } else { " " };
        out += &format!("{cursor} {name:<8} [{}{}] {:>3.0}%\n", "#".repeat(filled), "-".repeat(20 - filled), v * 100.0);
    }
    out += "\nUp/Down: choose   Left/Right: change";
    for mut t in &mut texts {
        t.0.clone_from(&out);
    }
}

/// Keep every playing sound at its channel's volume (new sounds and setting changes).
fn apply_volumes(settings: Res<AudioSettings>, mut sinks: Query<(&Channel, &mut AudioSink)>) {
    for (channel, mut sink) in &mut sinks {
        if settings.is_changed() || sink.is_added() {
            sink.set_volume(Volume::Linear(settings.gain(*channel)));
        }
    }
}

/// The car coasts while the settings are open (the arrow keys drive it otherwise).
fn freeze_car_input(menu: Res<SettingsMenu>, mut input: ResMut<crate::drive::CarInput>) {
    if menu.open {
        input.throttle = 0.0;
        input.brake = 0.0;
        input.steer = 0.0;
        input.handbrake = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pairs_and_percentages() {
        let mut s = AudioSettings::default();
        s.apply("master=0.5, music=30%,effects=2\nengine=bogus");
        assert_eq!(s.master, 0.5);
        assert!((s.music - 0.3).abs() < 1e-6);
        assert_eq!(s.sfx, 0.02);
        assert_eq!(s.engine, 1.0);
        let mut t = AudioSettings::default();
        t.apply(&s.to_text());
        assert_eq!(s, t);
    }
}
