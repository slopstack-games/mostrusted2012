//! `Font` (0x30): a bitmap font, one per `UI/FONTS/<id>.BNDL` next to its glyph atlas (a BC3 `Texture`,
//! glyphs in the alpha channel, colour white). UI text styles name fonts by that bundle number.
//!
//! Header (chunk 0):
//!
//! | off   | type     | field |
//! |-------|----------|-------|
//! | 0x00  | u32      | version (10) |
//! | 0x04  | u32      | data size (resource size − 16) |
//! | 0x08  | f32      | atlas width / em: the em is `width / this` atlas pixels |
//! | 0x0C  | f32      | atlas height / em (same em) |
//! | 0x14  | f32      | 0.71–0.82 (ascent / em?) |
//! | 0x18  | f32      | 0.66–0.80 (cap height / em?) |
//! | 0x1C  | u32      | glyph count |
//! | 0x20  | u32      | offset of the glyph records (32 bytes each) |
//! | 0x24  | u32      | offset of the code table (u16 per glyph, UTF-16 code unit) |
//! | 0x28  | u16[129] | hash buckets: glyphs with `code & 127 == b` are `bucket[b]..bucket[b + 1]` |
//! | 0x12C | u32      | 1 (texture count?) |
//! | 0x130 | ptr      | atlas texture (import) |
//! | 0x148 | u32      | point size the atlas was generated at (text of size `s` scales glyphs by `s / this`) |
//! | 0x14C | char[]   | name (the bundle number) |
//! | 0x1CC | char[]   | style ("Regular", "B5HelvRoman") |
//!
//! Glyph record: `f32 u, v, width, height` (normalised atlas rectangle), `f32 x offset` (fraction of the atlas
//! width), `f32 y offset` (fraction of the atlas height, from the line top: glyphs of one line share their
//! baseline at `y offset + height`), `f32 advance` (fraction of the atlas width), `u32 flags` (`0x2xxxx` drawn,
//! `0x40000` empty). Empty glyphs (space, control codes, the private-use `U+F0xx` button codes) have `u = -1`;
//! drawn glyphs can start a texel outside the atlas (`u = -1/256`).
//! Private-use `U+E1xx` codes are drawn icons (SP, controller buttons) embedded in strings.

use std::collections::HashMap;

use mw_bundle::{Reader, Resource, ResourceId};

use crate::{Error, Result};

/// One glyph, in atlas pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    /// Atlas rectangle (x, y, width, height); `None` for glyphs that only advance.
    pub rect: Option<[f32; 4]>,
    /// Offset of the rectangle from the pen position (x) and the line top (y).
    pub offset: [f32; 2],
    pub advance: f32,
    pub flags: u32,
}

#[derive(Debug, Clone)]
pub struct Font {
    pub id: ResourceId,
    pub name: String,
    pub style: String,
    pub texture: Option<ResourceId>,
    /// Atlas size in pixels, from the em ratios and the em (the texture header has the same).
    pub atlas_size: [f32; 2],
    /// Point size the atlas was generated at: text of size `s` scales glyphs by `s / point_size`.
    pub point_size: f32,
    /// Atlas width / the ratio at 0x08: within ±12 % of the point size (line height?).
    pub em: f32,
    pub metrics: [f32; 2],
    pub glyphs: HashMap<char, Glyph>,
}

impl Font {
    /// Parse a Font. The atlas size isn't stored in the font (only `size / em`), so it comes from the texture.
    pub fn parse(res: &Resource, atlas_size: [u32; 2]) -> Result<Self> {
        let data = res.header();
        let r = Reader(data);
        let version = r.u32(0)?;
        if version != 10 {
            return Err(Error::Unsupported(format!("font version {version}")));
        }
        let (w, h) = (atlas_size[0] as f32, atlas_size[1] as f32);
        let ratio = r.f32(0x08)?;
        if ratio.is_nan() || ratio <= 0.0 {
            return Err(Error::Malformed("font em ratio".into()));
        }
        let em = w / ratio;
        let n = r.u32(0x1C)? as usize;
        let glyph_at = r.u32(0x20)? as usize;
        let codes_at = r.u32(0x24)? as usize;
        let mut glyphs = HashMap::with_capacity(n);
        for i in 0..n {
            let code = r.u16(codes_at + i * 2)?;
            let g = glyph_at + i * 32;
            let f = |k: usize| r.f32(g + k * 4);
            let (u, v, gw, gh) = (f(0)?, f(1)?, f(2)?, f(3)?);
            let glyph = Glyph {
                rect: (u > -0.5 && gw > 0.0 && gh > 0.0).then_some([u * w, v * h, gw * w, gh * h]),
                offset: [f(4)? * w, f(5)? * h],
                advance: f(6)? * w,
                flags: r.u32(g + 28)?,
            };
            // Surrogate halves can't be chars; the fonts have none.
            if let Some(c) = char::from_u32(code as u32) {
                glyphs.insert(c, glyph);
            }
        }
        let cstr = |o: usize| {
            let s = data.get(o..).unwrap_or_default();
            String::from_utf8_lossy(&s[..s.iter().position(|&b| b == 0).unwrap_or(s.len()).min(0x80)]).into_owned()
        };
        Ok(Self {
            id: res.id,
            point_size: match r.u32(0x148)? {
                0 => em,
                n => n as f32,
            },
            name: cstr(0x14C),
            style: cstr(0x1CC),
            texture: res.import_at(r.u32(0x130)?),
            atlas_size: [w, h],
            em,
            metrics: [r.f32(0x14)?, r.f32(0x18)?],
            glyphs,
        })
    }

    /// The atlas texture import, readable before the atlas size is known.
    pub fn texture_of(res: &Resource) -> Option<ResourceId> {
        let r = Reader(res.header());
        res.import_at(r.u32(0x130).ok()?)
    }

    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs.get(&c)
    }

    /// Lay out one line of text at `size` (em, output units). Returns the quads (atlas rectangle, output
    /// rectangle relative to the pen start and line top) and the advance width. Unknown characters use `?`.
    ///
    /// Key boxes: the button prompts are `U+E047` (left cap), `U+E048` (fill), the key's name, `U+E049` (right
    /// cap), e.g. "ESC" for Esc (`UIListItems` 113577). The fill is stretched under the name, which is marked
    /// [`GlyphQuad::key`]: 1 for the box, 2 for the name (drawn in the box's ink).
    pub fn layout(&self, text: &str, size: f32, spacing: f32) -> (Vec<GlyphQuad>, f32) {
        const FILL: char = '\u{e048}';
        const RIGHT: char = '\u{e049}';
        const PROMPTS: char = '\u{e000}';
        const PROMPTS_END: char = '\u{e0ff}';
        let scale = size / self.point_size;
        let mut pen = 0.0;
        let mut quads: Vec<GlyphQuad> = Vec::new();
        // An open key box: where its fill goes in `quads` and the pen there.
        let mut key_box: Option<(usize, f32)> = None;
        for c in text.chars() {
            if c == FILL && self.glyph(FILL).is_some() {
                key_box = Some((quads.len(), pen));
                continue;
            }
            if c == RIGHT
                && let Some((at, start)) = key_box.take()
                && let Some(fill) = self.glyph(FILL)
                && let Some(rect) = fill.rect
            {
                let natural = fill.advance * scale;
                let width = (pen - start).max(natural);
                // A name narrower than the box sits in its middle.
                let shift = (width - (pen - start)) * 0.5;
                for q in &mut quads[at..] {
                    q.rect[0] += shift;
                }
                quads.insert(
                    at,
                    GlyphQuad {
                        atlas: rect,
                        rect: [start + fill.offset[0] * scale, fill.offset[1] * scale, width, rect[3] * scale],
                        key: 1,
                    },
                );
                pen = start + width;
            }
            let Some(g) = self.glyph(c).or_else(|| self.glyph('?')) else { continue };
            if let Some(rect) = g.rect {
                quads.push(GlyphQuad {
                    atlas: rect,
                    rect: [pen + g.offset[0] * scale, g.offset[1] * scale, rect[2] * scale, rect[3] * scale],
                    // Prompt glyphs (the private-use U+E0xx range: box caps, the Enter key) are light.
                    key: if key_box.is_some() {
                        2
                    } else if (PROMPTS..=PROMPTS_END).contains(&c) {
                        1
                    } else {
                        0
                    },
                });
            }
            pen += g.advance * scale + spacing;
        }
        (quads, pen)
    }
}

/// A positioned glyph: atlas rectangle (pixels) and output rectangle (x, y, width, height).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphQuad {
    pub atlas: [f32; 4],
    pub rect: [f32; 4],
    /// Key boxes (see [`Font::layout`]): 1 = part of the box, 2 = the key's name in it, 0 = text.
    pub key: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_scales_by_em() {
        let mut glyphs = HashMap::new();
        glyphs.insert('A', Glyph { rect: Some([0.0, 0.0, 10.0, 20.0]), offset: [1.0, 2.0], advance: 12.0, flags: 0 });
        glyphs.insert(' ', Glyph { rect: None, offset: [0.0, 0.0], advance: 6.0, flags: 0 });
        let font = Font {
            id: ResourceId(0),
            name: String::new(),
            style: String::new(),
            texture: None,
            atlas_size: [256.0, 256.0],
            point_size: 20.0,
            em: 19.0,
            metrics: [0.8, 0.7],
            glyphs,
        };
        let (quads, width) = font.layout("A A", 10.0, 0.0);
        assert_eq!(quads.len(), 2);
        assert_eq!(width, 15.0);
        assert_eq!(quads[1].rect, [9.5, 1.0, 5.0, 10.0]);
    }
}
