use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use parley::{
    Alignment, AlignmentOptions, FontContext, LayoutContext,
    fontique::Blob,
    layout::PositionedLayoutItem,
    style::{FontFamily, StyleProperty},
};

#[derive(Debug, Clone)]
pub struct Glyph {
    pub glyph_id: u32,
    pub cluster: u32,
    pub x_advance: f32,
    pub y_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
}

#[derive(Debug, Clone)]
pub struct ShapedRun {
    pub glyphs: Vec<Glyph>,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub bbox_left: f32,
    pub bbox_right: f32,
    pub bbox_ascent: f32,
    pub bbox_descent: f32,
}

impl ShapedRun {
    pub fn empty() -> Self {
        Self {
            glyphs: Vec::new(),
            width: 0.0,
            ascent: 0.0,
            descent: 0.0,
            bbox_left: 0.0,
            bbox_right: 0.0,
            bbox_ascent: 0.0,
            bbox_descent: 0.0,
        }
    }
}

struct ParleyState {
    font_cx: Mutex<FontContext>,
    layout_cx: Mutex<LayoutContext>,
    family_cache: Mutex<HashMap<usize, String>>,
}

static PARLEY: OnceLock<ParleyState> = OnceLock::new();

fn parley_state() -> &'static ParleyState {
    PARLEY.get_or_init(|| ParleyState {
        font_cx: Mutex::new(FontContext::new()),
        layout_cx: Mutex::new(LayoutContext::new()),
        family_cache: Mutex::new(HashMap::new()),
    })
}

fn ensure_font_registered(state: &ParleyState, face_data: &[u8]) -> String {
    let key = face_data.as_ptr() as usize;
    {
        let cache = state.family_cache.lock().unwrap();
        if let Some(name) = cache.get(&key) {
            return name.clone();
        }
    }
    let mut font_cx = state.font_cx.lock().unwrap();
    let blob = Blob::from(face_data.to_vec());
    let results = font_cx.collection.register_fonts(blob, None);
    let name = results
        .first()
        .and_then(|(fid, _)| font_cx.collection.family_name(*fid))
        .unwrap_or("sans-serif")
        .to_string();
    drop(font_cx);
    let mut cache = state.family_cache.lock().unwrap();
    cache.insert(key, name.clone());
    name
}

pub fn shape(text: &str, face_data: &[u8], _face_index: u32, size_px: f32) -> ShapedRun {
    if text.is_empty() {
        return ShapedRun::empty();
    }

    let state = parley_state();
    let family_name = ensure_font_registered(state, face_data);

    let mut font_cx = state.font_cx.lock().unwrap();
    let mut layout_cx = state.layout_cx.lock().unwrap();

    let mut builder = layout_cx.ranged_builder(&mut font_cx, text, 1.0, true);
    builder.push_default(StyleProperty::FontFamily(FontFamily::named(&family_name)));
    builder.push_default(StyleProperty::FontSize(size_px));

    let mut layout = builder.build(text);
    layout.break_all_lines(Some(f32::INFINITY));
    layout.align(Alignment::Start, AlignmentOptions::default());

    let mut glyphs = Vec::new();
    let mut cursor_x = 0.0_f32;
    let mut bbox_left = f32::INFINITY;
    let mut bbox_right = f32::NEG_INFINITY;
    let mut bbox_ascent = f32::NEG_INFINITY;
    let mut bbox_descent = f32::NEG_INFINITY;
    let mut ascent = 0.0_f32;
    let mut descent = 0.0_f32;

    for line in layout.lines() {
        let line_metrics = line.metrics();
        if line_metrics.ascent > ascent {
            ascent = line_metrics.ascent;
        }
        if line_metrics.descent > descent {
            descent = line_metrics.descent;
        }

        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                for g in glyph_run.positioned_glyphs() {
                    let x_advance = g.advance;
                    let x_offset = g.x - cursor_x;
                    let y_offset = g.y;

                    let glyph_left = cursor_x + x_offset;
                    let glyph_right = glyph_left + x_advance;
                    if glyph_left < bbox_left {
                        bbox_left = glyph_left;
                    }
                    if glyph_right > bbox_right {
                        bbox_right = glyph_right;
                    }
                    if -y_offset > bbox_ascent {
                        bbox_ascent = -y_offset;
                    }
                    if y_offset > bbox_descent {
                        bbox_descent = y_offset;
                    }

                    glyphs.push(Glyph {
                        glyph_id: g.id,
                        cluster: 0,
                        x_advance,
                        y_advance: 0.0,
                        x_offset,
                        y_offset,
                    });
                    cursor_x += x_advance;
                }
            }
        }
    }

    let width = cursor_x;

    if glyphs.is_empty() {
        bbox_left = 0.0;
        bbox_right = 0.0;
        bbox_ascent = 0.0;
        bbox_descent = 0.0;
    }

    ShapedRun {
        glyphs,
        width,
        ascent,
        descent,
        bbox_left,
        bbox_right,
        bbox_ascent,
        bbox_descent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::text::font_database::FontDatabase;

    fn arial_face() -> (&'static [u8], u32) {
        let db = FontDatabase::get();
        let id = db.query("Arial", 400, false, "Linux").expect("Arial");
        db.face_data(id).expect("face data")
    }

    #[test]
    fn empty_text_returns_empty_run() {
        let (data, idx) = arial_face();
        let run = shape("", data, idx, 14.0);
        assert!(run.glyphs.is_empty());
        assert_eq!(run.width, 0.0);
    }

    #[test]
    fn hello_world_has_nonzero_width() {
        let (data, idx) = arial_face();
        let run = shape("Hello, World!", data, idx, 14.0);
        assert!(!run.glyphs.is_empty());
        assert!(run.width > 0.0);
    }

    #[test]
    fn size_scales_width_linearly() {
        let (data, idx) = arial_face();
        let short = shape("Hello", data, idx, 14.0);
        let tall = shape("Hello", data, idx, 28.0);
        let ratio = tall.width / short.width;
        assert!((ratio - 2.0).abs() < 0.05);
    }
}
