//! Text shaping via `rustybuzz`.

use rustybuzz::{Face as RustybuzzFace, UnicodeBuffer};

/// A single glyph emitted by the shaper.
#[derive(Debug, Clone)]
pub struct Glyph {
    pub glyph_id: u32,
    pub cluster: u32,
    pub x_advance: f32,
    pub y_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
}

/// A shaped run of glyphs.
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

/// Shape `text` using the given font face at `size_px`.
pub fn shape(text: &str, face_data: &[u8], face_index: u32, size_px: f32) -> ShapedRun {
    if text.is_empty() {
        return ShapedRun::empty();
    }
    let Some(face) = RustybuzzFace::from_slice(face_data, face_index) else {
        return ShapedRun::empty();
    };
    let upem = face.units_per_em() as f32;
    if upem <= 0.0 {
        return ShapedRun::empty();
    }
    let scale = size_px / upem;

    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();

    let glyph_buffer = rustybuzz::shape(&face, &[], buffer);
    let infos = glyph_buffer.glyph_infos();
    let positions = glyph_buffer.glyph_positions();

    let mut glyphs = Vec::with_capacity(infos.len());
    let mut cursor_x = 0.0_f32;
    let mut total_advance = 0.0_f32;
    let mut bbox_left = f32::INFINITY;
    let mut bbox_right = f32::NEG_INFINITY;
    let mut bbox_ascent = f32::NEG_INFINITY;
    let mut bbox_descent = f32::NEG_INFINITY;
    let mut any_glyph_bbox = false;

    for (info, pos) in infos.iter().zip(positions.iter()) {
        let x_advance = pos.x_advance as f32 * scale;
        let y_advance = pos.y_advance as f32 * scale;
        let x_offset = pos.x_offset as f32 * scale;
        let y_offset = pos.y_offset as f32 * scale;

        if let Some(em_bbox) =
            face.glyph_bounding_box(rustybuzz::ttf_parser::GlyphId(info.glyph_id as u16))
        {
            any_glyph_bbox = true;
            let glyph_left = cursor_x + x_offset + em_bbox.x_min as f32 * scale;
            let glyph_right = cursor_x + x_offset + em_bbox.x_max as f32 * scale;
            let glyph_top = (em_bbox.y_max as f32 * scale) - y_offset;
            let glyph_bot = (em_bbox.y_min as f32 * scale) - y_offset;
            if glyph_left < bbox_left {
                bbox_left = glyph_left;
            }
            if glyph_right > bbox_right {
                bbox_right = glyph_right;
            }
            if glyph_top > bbox_ascent {
                bbox_ascent = glyph_top;
            }
            if -glyph_bot > bbox_descent {
                bbox_descent = -glyph_bot;
            }
        }

        glyphs.push(Glyph {
            glyph_id: info.glyph_id,
            cluster: info.cluster,
            x_advance,
            y_advance,
            x_offset,
            y_offset,
        });
        cursor_x += x_advance;
        total_advance += x_advance;
    }

    let ascender = face.ascender() as f32 * scale;
    let descender = face.descender() as f32 * scale;

    if !any_glyph_bbox {
        bbox_left = 0.0;
        bbox_right = total_advance;
        bbox_ascent = ascender;
        bbox_descent = -descender;
    }

    ShapedRun {
        glyphs,
        width: total_advance,
        ascent: ascender,
        descent: -descender,
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
