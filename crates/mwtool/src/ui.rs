//! `mwtool ui`: inspect and preview the game's UI screens and layouts (see `mw_formats::ui`).

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use mw_assets::AssetStore;
use mw_assets::ui::{PlacedLayout, UiCache};
use mw_bundle::ResourceId;
use mw_formats::Language;
use mw_formats::ui::{Element, REFERENCE_SIZE};

/// One line per screen bundle: its widgets and how many layouts each places.
pub fn screens(store: &AssetStore) -> Result<()> {
    let mut paths: Vec<_> = store
        .bundle_paths()
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .filter(|p| p.to_ascii_uppercase().starts_with("UI/SCREENS2/") && !p.to_ascii_uppercase().contains("_TRANS"))
        .collect();
    paths.sort();
    for p in paths {
        match store.ui_screen(&p) {
            Ok(s) => {
                let widgets: Vec<String> =
                    s.widgets
                        .iter()
                        .map(|w| {
                            if w.layouts.is_empty() {
                                w.name.clone()
                            } else {
                                format!("{}({})", w.name, w.layouts.len())
                            }
                        })
                        .collect();
                println!("{p}  screen {}: {}", s.id, widgets.join(" "));
            }
            Err(e) => println!("{p}  ({e})"),
        }
    }
    Ok(())
}

fn text_of(lang: &Language, e: &Element) -> String {
    let Some(t) = &e.text else { return String::new() };
    match lang.get(t.string_id) {
        Some(s) => s.to_string(),
        None if !e.bindings.text.is_empty() => format!("{{{}}}", e.bindings.text),
        None => String::new(),
    }
}

/// Print a screen's widgets, layouts and elements.
pub fn show(store: &AssetStore, bundle: &str, widgets: Option<&[&str]>, lang: &Language) -> Result<()> {
    let screen = store.ui_screen(bundle)?;
    println!("screen {} ({} widgets)", screen.id, screen.widgets.len());
    for w in &screen.widgets {
        println!("  widget {:?} {} layer {} layouts {}", w.name, w.type_name, w.layer, w.layouts.len());
    }
    let mut cache = UiCache::default();
    for pl in store.ui_screen_layouts(bundle, widgets)? {
        print_layout(store, &mut cache, &pl, lang);
    }
    Ok(())
}

fn print_layout(store: &AssetStore, cache: &mut UiCache, pl: &PlacedLayout, lang: &Language) {
    println!("\n== widget {:?} layout {} ({} elements)", pl.widget, pl.layout.id, pl.layout.elements.len());
    if let Some(i) = &pl.instance {
        let t: Vec<String> = i
            .transforms
            .iter()
            .map(|t| {
                format!(
                    "{}{:?}{}",
                    ["?", "T", "R", "S"][t.kind.min(3) as usize],
                    t.value,
                    if t.binding.is_empty() { String::new() } else { format!(" <{}>", t.binding) }
                )
            })
            .collect();
        println!("   instance anchor {} [{}] {}", i.anchor_type, t.join(", "), i.timeline);
    }
    for e in &pl.layout.elements {
        print_element(store, cache, e, lang, 1);
    }
}

fn print_element(store: &AssetStore, cache: &mut UiCache, e: &Element, lang: &Language, depth: usize) {
    let pad = "   ".repeat(depth);
    let tl = e.top_left();
    let mut line = format!(
        "{pad}{:?} {} at ({:.0},{:.0}) size {:.0}x{:.0} anchor {} rot {}",
        e.kind, e.id, tl[0], tl[1], e.size[0], e.size[1], e.anchor, e.rotation
    );
    if let Some(t) = e.texture {
        line += &format!(" tex {t} uv {:?}", store.ui_element_uv(e));
    }
    if e.tint != [1.0; 4] {
        line += &format!(" tint {:?}", e.tint);
    }
    if e.opacity != 1.0 {
        line += &format!(" opacity {}", e.opacity);
    }
    if let Some(bg) = e.background {
        line += &format!(" bg {bg:?}");
    }
    if let Some(t) = &e.text {
        let style = t.style.and_then(|s| cache.style(store, s).cloned());
        line += &format!(" text {:?}", text_of(lang, e));
        if let Some(s) = style {
            line += &format!(" font {} size {} colour {:?} justify {}", s.font, s.size, s.colour, t.justification);
        }
    }
    println!("{line}");
    let b = &e.bindings;
    for (name, v) in [
        ("visible", &b.visibility),
        ("x", &b.position_x),
        ("y", &b.position_y),
        ("rot", &b.rotation),
        ("wscale", &b.width_scale),
        ("hscale", &b.height_scale),
        ("tint", &b.tint),
        ("opacity", &b.opacity),
        ("image", &b.image),
        ("text", &b.text),
        ("colour", &b.text_colour),
    ] {
        if !v.is_empty() {
            println!("{pad}   {name}: {v}");
        }
    }
    for t in &e.triggered {
        println!("{pad}   on {:?}: {} in / {} out", t.trigger, t.transition_in.len(), t.transition_out.len());
    }
    if let Some(s) = &e.sub_image {
        print_element(store, cache, s, lang, depth + 1);
    }
}

/// RGBA float canvas (straight alpha), reference-frame sized times `scale`.
pub struct Canvas {
    pub w: u32,
    pub h: u32,
    pub scale: f32,
    px: Vec<[f32; 4]>,
}

impl Canvas {
    pub fn new(scale: f32, background: [f32; 4]) -> Self {
        let (w, h) = ((REFERENCE_SIZE[0] * scale) as u32, (REFERENCE_SIZE[1] * scale) as u32);
        Self { w, h, scale, px: vec![background; (w * h) as usize] }
    }

    fn blend(&mut self, x: u32, y: u32, c: [f32; 4]) {
        let p = &mut self.px[(y * self.w + x) as usize];
        let a = c[3].clamp(0.0, 1.0);
        for i in 0..3 {
            p[i] = p[i] * (1.0 - a) + c[i] * a;
        }
        p[3] = p[3] + a * (1.0 - p[3]);
    }

    /// Fill a rotated rectangle (reference pixels; rotation in degrees about `pivot`), colour from `shade(u, v)`
    /// with u, v in 0..1 across the rectangle.
    pub fn fill(
        &mut self,
        rect: [f32; 4],
        rotation: f32,
        pivot: [f32; 2],
        mut shade: impl FnMut(f32, f32) -> [f32; 4],
    ) {
        let s = self.scale;
        let (sin, cos) = rotation.to_radians().sin_cos();
        let corners = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]].map(|[u, v]| {
            let (x, y) = (rect[0] + u * rect[2] - pivot[0], rect[1] + v * rect[3] - pivot[1]);
            [(pivot[0] + x * cos - y * sin) * s, (pivot[1] + x * sin + y * cos) * s]
        });
        let minx = corners.iter().map(|c| c[0]).fold(f32::MAX, f32::min).floor().max(0.0) as u32;
        let maxx = corners.iter().map(|c| c[0]).fold(f32::MIN, f32::max).ceil().min(self.w as f32) as u32;
        let miny = corners.iter().map(|c| c[1]).fold(f32::MAX, f32::min).floor().max(0.0) as u32;
        let maxy = corners.iter().map(|c| c[1]).fold(f32::MIN, f32::max).ceil().min(self.h as f32) as u32;
        if rect[2] <= 0.0 || rect[3] <= 0.0 {
            return;
        }
        for y in miny..maxy {
            for x in minx..maxx {
                // Back-rotate the pixel centre into the rectangle.
                let (px, py) = ((x as f32 + 0.5) / s - pivot[0], (y as f32 + 0.5) / s - pivot[1]);
                let (rx, ry) = (pivot[0] + px * cos + py * sin, pivot[1] - px * sin + py * cos);
                let (u, v) = ((rx - rect[0]) / rect[2], (ry - rect[1]) / rect[3]);
                if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
                    let c = shade(u, v);
                    if c[3] > 0.0 {
                        self.blend(x, y, c);
                    }
                }
            }
        }
    }

    pub fn save(&self, out: &Path) -> Result<()> {
        let bytes: Vec<u8> = self
            .px
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], p[3]].map(|c| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8))
            .collect();
        image::RgbaImage::from_raw(self.w, self.h, bytes).context("canvas size")?.save(out)?;
        Ok(())
    }
}

/// Decoded RGBA8 image.
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

impl Image {
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let x = ((u * self.w as f32) as i64).clamp(0, self.w as i64 - 1) as usize;
        let y = ((v * self.h as f32) as i64).clamp(0, self.h as i64 - 1) as usize;
        let i = (y * self.w as usize + x) * 4;
        [0, 1, 2, 3].map(|k| self.px[i + k] as f32 / 255.0)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// Options of `mwtool ui render`.
pub struct RenderOptions<'a> {
    pub widgets: Option<&'a [&'a str]>,
    /// Only these layouts (UniqueIds).
    pub layouts: &'a [u32],
    pub scale: f32,
    /// Binding values (`path=value`); text values unless they parse as numbers.
    pub values: &'a [String],
    /// Unknown bindings evaluate to 1 (everything visible, scales 1) instead of hiding their elements.
    pub all: bool,
    /// Draw the minimap centred on world `x, z` facing `heading` degrees.
    pub map: Option<[f32; 3]>,
}

/// Render a screen's layouts through [`mw_assets::ui::UiScene::draw`] (the same path as the game's HUD).
pub fn render(store: &AssetStore, bundle: &str, lang: &Language, out: &Path, o: &RenderOptions) -> Result<()> {
    use mw_assets::ui::UiBlend;
    use mw_formats::binding::BindValue;
    let mut layouts = store.ui_screen_layouts(bundle, o.widgets)?;
    if !o.layouts.is_empty() {
        layouts.retain(|l| o.layouts.contains(&l.layout.id));
    }
    let n = layouts.len();
    let scene = store.ui_scene(layouts, lang);
    let values: HashMap<String, BindValue> = o
        .values
        .iter()
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| {
            let value = v.parse::<f64>().map(BindValue::Num).unwrap_or_else(|_| BindValue::Text(v.to_string()));
            (k.to_string(), value)
        })
        .collect();
    let mut lookup = |path: &str, arg: Option<&str>| -> Option<BindValue> {
        if let Some(v) = values.get(path) {
            return Some(v.clone());
        }
        if path == "Signals.GetSineWave" {
            return Some(BindValue::Num(0.5));
        }
        let _ = arg;
        o.all.then_some(BindValue::Num(1.0))
    };
    let mut canvas = Canvas::new(o.scale, [0.18, 0.2, 0.24, 1.0]);
    let screen = [canvas.w as f32, canvas.h as f32];
    let quads = scene.draw(screen, &mut lookup);
    let mut images: HashMap<ResourceId, Option<Image>> = HashMap::new();
    let mosaic = o.map.and_then(|_| store.ui_map_grid().ok()).map(|g| mw_assets::ui::MapMosaic::build(store, g));
    for q in &quads {
        // The canvas is in reference pixels × scale; quads are already in canvas pixels.
        let r = [q.rect[0] / o.scale, q.rect[1] / o.scale, q.rect[2] / o.scale, q.rect[3] / o.scale];
        let pivot = [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5];
        let c = [linear_to_srgb(q.colour[0]), linear_to_srgb(q.colour[1]), linear_to_srgb(q.colour[2]), q.colour[3]];
        let additive = q.blend == UiBlend::Additive;
        let shade = |s: [f32; 4]| -> [f32; 4] {
            let px = [s[0] * c[0], s[1] * c[1], s[2] * c[2], s[3] * c[3]];
            // Additive: fake it by scaling alpha with brightness.
            if additive { [px[0], px[1], px[2], px[3] * (px[0] + px[1] + px[2]) / 3.0] } else { px }
        };
        if q.map {
            let (Some(m), Some([x, z, heading])) = (&mosaic, o.map) else { continue };
            let (w, h) = (q.rect[2].max(1.0) as u32, q.rect[3].max(1.0) as u32);
            let mut px = vec![0u8; (w * h * 4) as usize];
            let f = [heading.to_radians().sin(), heading.to_radians().cos()];
            m.render(x, z, f, MINIMAP_METRES_PER_PIXEL / o.scale, w, h, &mut px);
            let img = Image { w, h, px };
            canvas.fill(r, 0.0, pivot, |u, v| {
                let s = img.sample(u, v);
                [s[0], s[1], s[2], 0.62]
            });
            continue;
        }
        match q.texture {
            None => canvas.fill(r, q.rotation, pivot, |_, _| shade([1.0; 4])),
            Some(id) => {
                let img = images.entry(id).or_insert_with(|| {
                    let t = store.texture(id).ok()?;
                    let (w, h, px) = t.decode_rgba8(0)?;
                    Some(Image { w, h, px })
                });
                let Some(img) = img.as_ref() else { continue };
                let uv = q.uv;
                canvas.fill(r, q.rotation, pivot, |u, v| {
                    shade(img.sample(uv[0] + (uv[2] - uv[0]) * u, uv[1] + (uv[3] - uv[1]) * v))
                });
            }
        }
    }
    canvas.save(out)?;
    println!("{n} layouts, {} quads -> {}", quads.len(), out.display());
    Ok(())
}

/// Minimap scale (metres per reference pixel): the map tiles' own resolution (256 px per 1036 m tile),
/// pending the game's map camera (`UIMapCameraConfig`: distance 600, FOV 90°).
pub const MINIMAP_METRES_PER_PIXEL: f32 = 4.0;

/// Render a sample string in a UI font.
pub fn font_sample(store: &AssetStore, number: u32, text: &str, size: f32, out: &Path) -> Result<()> {
    let font = store.ui_font(number)?;
    println!(
        "font {} {:?} {:?}: {} glyphs, atlas {:?}, point size {}, em {:.2} px, metrics {:?}",
        number,
        font.name,
        font.style,
        font.glyphs.len(),
        font.atlas_size,
        font.point_size,
        font.em,
        font.metrics
    );
    let atlas = store.texture(font.texture.context("font without atlas")?)?;
    let (aw, ah, px) = atlas.decode_rgba8(0).context("atlas decode")?;
    let img = Image { w: aw, h: ah, px };
    let mut canvas = Canvas::new(1.0, [0.1, 0.1, 0.12, 1.0]);
    let mut y = 10.0;
    for line in text.split('\n') {
        let (quads, width) = font.layout(line, size, 0.0);
        // Baseline guide and em box.
        canvas.fill([10.0, y, width, size], 0.0, [0.0, 0.0], |_, _| [0.3, 0.3, 0.5, 0.5]);
        for q in &quads {
            let r = [10.0 + q.rect[0], y + q.rect[1], q.rect[2], q.rect[3]];
            canvas.fill(r, 0.0, [0.0, 0.0], |u, v| {
                let c =
                    img.sample((q.atlas[0] + u * q.atlas[2]) / aw as f32, (q.atlas[1] + v * q.atlas[3]) / ah as f32);
                [c[0], c[1], c[2], c[3]]
            });
        }
        y += size * 1.4;
    }
    canvas.save(out)?;
    Ok(())
}
