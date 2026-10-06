//! Text in the game's own bitmap fonts (`UI/FONTS`, `mw_formats::Font`) as Bevy UI: a [`GameText`] node
//! gets one image child per glyph, cut from the font's atlas, and sizes itself to the text. Bevy's text
//! needs vector fonts and its default font is ASCII-only; the game's fonts cover accented names.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use mw_assets::AssetStore;
use mw_formats::Font;

/// Regular weight with the widest Latin coverage (`UI/FONTS/1847555.BNDL`: Central European, Cyrillic).
pub const BODY_FONT: u32 = 1_847_555;
/// Bold (`UI/FONTS/1206239.BNDL`), the HUD's label font.
pub const TITLE_FONT: u32 = 1_206_239;

/// Line height relative to the text size (room for descenders below the em box).
const LINE_HEIGHT: f32 = 1.25;

pub struct LoadedFont {
    pub font: Arc<Font>,
    pub atlas: Handle<Image>,
}

/// Fonts by bundle number. The menu's fonts load at startup; the HUD adds the ones its styles use.
#[derive(Resource, Default)]
pub struct GameFonts {
    pub fonts: HashMap<u32, LoadedFont>,
}

impl GameFonts {
    pub fn get(&self, number: u32) -> Option<&LoadedFont> {
        self.fonts.get(&number).or_else(|| self.fonts.get(&BODY_FONT))
    }
}

/// A line of text in a game font; size in logical pixels (the font's point size scales to it).
#[derive(Component, Clone, Debug, PartialEq)]
#[require(Node)]
pub struct GameText {
    pub text: String,
    pub size: f32,
    pub color: Color,
    pub font: u32,
}

// Builders for game-font text outside the HUD (the car menu); not used yet (paused UI session).
#[allow(dead_code)]
impl GameText {
    pub fn new(text: impl Into<String>, size: f32, color: Color) -> Self {
        Self { text: text.into(), size, color, font: BODY_FONT }
    }

    pub fn with_font(mut self, font: u32) -> Self {
        self.font = font;
        self
    }
}

#[derive(Component)]
struct GameGlyph;

pub struct GameTextPlugin;

impl Plugin for GameTextPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameFonts>()
            .add_systems(Startup, load_menu_fonts)
            // Before the stack system too (it is not ordered against Prepare): glyphs spawned after it keep stack
            // index 0 for the frame and draw under their own panel, which hid every text that changes each frame.
            .add_systems(
                PostUpdate,
                layout_game_text.before(bevy::ui::UiSystems::Prepare).before(bevy::ui::UiSystems::Stack),
            );
    }
}

/// An RGBA8 sRGB image for UI use (clamped, linear filtering).
pub fn ui_image(width: u32, height: u32, rgba: Vec<u8>) -> Image {
    let mut img = Image::new(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    img
}

/// Decode a game texture for the UI.
pub fn decode_texture(store: &AssetStore, id: mw_bundle::ResourceId) -> Option<Image> {
    let t = store.texture(id).ok()?;
    let (w, h, px) = t.decode_rgba8(0)?;
    Some(ui_image(w, h, px))
}

/// Load a font and its atlas.
pub fn load_font(store: &AssetStore, number: u32) -> Option<(Font, Image)> {
    let font = store.ui_font(number).map_err(|e| warn!("UI font {number}: {e}")).ok()?;
    let atlas = decode_texture(store, font.texture?)?;
    Some((font, atlas))
}

fn load_menu_fonts(game: Res<crate::Game>, mut fonts: ResMut<GameFonts>, mut images: ResMut<Assets<Image>>) {
    for number in [BODY_FONT, TITLE_FONT] {
        if let Some((font, atlas)) = load_font(&game.store, number) {
            fonts.fonts.insert(number, LoadedFont { font: Arc::new(font), atlas: images.add(atlas) });
        }
    }
}

/// Rebuild the glyphs of changed texts (all of them when fonts arrive).
fn layout_game_text(
    mut commands: Commands,
    fonts: Res<GameFonts>,
    mut texts: Query<(Entity, Ref<GameText>, &mut Node, Option<&Children>)>,
    glyphs: Query<(), With<GameGlyph>>,
) {
    let all = fonts.is_changed();
    for (entity, text, mut node, children) in &mut texts {
        if !all && !text.is_changed() {
            continue;
        }
        for child in children.into_iter().flatten() {
            if glyphs.contains(*child) {
                commands.entity(*child).despawn();
            }
        }
        let height = (text.size * LINE_HEIGHT).ceil();
        let Some(f) = fonts.get(text.font) else {
            // No game font (missing install files): Bevy's default font.
            commands.spawn((
                GameGlyph,
                Text::new(text.text.clone()),
                TextFont { font_size: bevy::text::FontSize::Px(text.size), ..default() },
                TextColor(text.color),
                ChildOf(entity),
            ));
            continue;
        };
        let (quads, width) = f.font.layout(&text.text, text.size, 0.0);
        let (w, h) = (Val::Px(width.ceil()), Val::Px(height));
        if node.width != w || node.height != h {
            node.width = w;
            node.height = h;
        }
        for q in quads {
            let a = q.atlas;
            commands.spawn((
                GameGlyph,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(q.rect[0]),
                    top: Val::Px(q.rect[1]),
                    width: Val::Px(q.rect[2]),
                    height: Val::Px(q.rect[3]),
                    ..default()
                },
                ImageNode {
                    image: f.atlas.clone(),
                    color: text.color,
                    rect: Some(Rect::new(a[0], a[1], a[0] + a[2], a[1] + a[3])),
                    ..default()
                },
                Pickable::IGNORE,
                ChildOf(entity),
            ));
        }
    }
}
