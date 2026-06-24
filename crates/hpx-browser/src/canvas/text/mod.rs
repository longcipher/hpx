//! Font stack for Canvas 2D text rendering.

pub mod font_database;
pub mod font_shorthand;
pub mod raster;
pub mod shaper;

pub use font_database::FontDatabase;
pub use font_shorthand::ParsedFont;
pub use raster::GlyphBitmap;
pub use shaper::ShapedRun;

/// Full 13-field `TextMetrics` object.
#[derive(Debug, Clone, PartialEq)]
pub struct TextMetrics {
    pub width: f32,
    pub actual_bounding_box_left: f32,
    pub actual_bounding_box_right: f32,
    pub actual_bounding_box_ascent: f32,
    pub actual_bounding_box_descent: f32,
    pub font_bounding_box_ascent: f32,
    pub font_bounding_box_descent: f32,
    pub em_height_ascent: f32,
    pub em_height_descent: f32,
    pub hanging_baseline: f32,
    pub alphabetic_baseline: f32,
    pub ideographic_baseline: f32,
}

impl TextMetrics {
    pub fn zero() -> Self {
        Self {
            width: 0.0,
            actual_bounding_box_left: 0.0,
            actual_bounding_box_right: 0.0,
            actual_bounding_box_ascent: 0.0,
            actual_bounding_box_descent: 0.0,
            font_bounding_box_ascent: 0.0,
            font_bounding_box_descent: 0.0,
            em_height_ascent: 0.0,
            em_height_descent: 0.0,
            hanging_baseline: 0.0,
            alphabetic_baseline: 0.0,
            ideographic_baseline: 0.0,
        }
    }
}

fn resolve_face(font: &ParsedFont, os_name: &str) -> Option<(&'static [u8], u32)> {
    let db = FontDatabase::get();
    let id = db.query_chain(&font.families, font.weight, font.italic, os_name)?;
    db.face_data(id)
}

/// Measure text metrics.
pub fn measure_text_metrics(text: &str, font: &ParsedFont, os_name: &str) -> TextMetrics {
    if text.is_empty() {
        return TextMetrics::zero();
    }
    let Some((data, idx)) = resolve_face(font, os_name) else {
        return TextMetrics::zero();
    };
    let run = shaper::shape(text, data, idx, font.size_px);

    let em_ascent = font.size_px * 0.8;
    let em_descent = font.size_px * 0.2;

    TextMetrics {
        width: run.width,
        actual_bounding_box_left: -run.bbox_left.min(0.0),
        actual_bounding_box_right: run.bbox_right.max(run.width),
        actual_bounding_box_ascent: run.bbox_ascent,
        actual_bounding_box_descent: run.bbox_descent,
        font_bounding_box_ascent: run.ascent,
        font_bounding_box_descent: run.descent,
        em_height_ascent: em_ascent,
        em_height_descent: em_descent,
        hanging_baseline: run.ascent * 0.8,
        alphabetic_baseline: 0.0,
        ideographic_baseline: -run.descent * 0.5,
    }
}

/// Measure text width only.
pub fn measure_text_width(text: &str, font: &ParsedFont, os_name: &str) -> f64 {
    measure_text_metrics(text, font, os_name).width as f64
}

/// One blitted glyph ready for compositing.
pub struct PlacedGlyph {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub coverage: Vec<u8>,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub alpha: f32,
}

/// Resolve font face + shape text, returning face bytes, TTC index, and shaped run.
pub fn shape_run(
    text: &str,
    font: &ParsedFont,
    os_name: &str,
) -> Option<(&'static [u8], u32, ShapedRun)> {
    if text.is_empty() {
        return None;
    }
    let (data, idx) = resolve_face(font, os_name)?;
    Some((data, idx, shaper::shape(text, data, idx, font.size_px)))
}

/// Shape + rasterize text, producing placed alpha masks.
pub fn rasterize_text(
    text: &str,
    x: f32,
    y: f32,
    font: &ParsedFont,
    r: u8,
    g: u8,
    b: u8,
    alpha: f32,
    os_name: &str,
) -> Vec<PlacedGlyph> {
    if text.is_empty() {
        return Vec::new();
    }
    let Some((data, idx)) = resolve_face(font, os_name) else {
        return Vec::new();
    };
    let run = shaper::shape(text, data, idx, font.size_px);

    let mut out = Vec::with_capacity(run.glyphs.len());
    let mut cursor_x = x;
    for glyph in &run.glyphs {
        if let Some(bitmap) = raster::rasterize_glyph(data, idx, glyph.glyph_id, font.size_px) {
            let draw_x = cursor_x + glyph.x_offset + bitmap.left as f32;
            let draw_y = y - glyph.y_offset - bitmap.top as f32;
            out.push(PlacedGlyph {
                x: draw_x.round() as i32,
                y: draw_y.round() as i32,
                width: bitmap.width,
                height: bitmap.height,
                coverage: bitmap.pixels,
                r,
                g,
                b,
                alpha,
            });
        }
        cursor_x += glyph.x_advance;
    }
    out
}

/// Adapter for `ttf_parser::OutlineBuilder` → `Path2D`.
struct Path2DOutlineBuilder<'a> {
    path: &'a mut crate::canvas::path::Path2D,
    origin_x: f32,
    origin_y: f32,
    scale: f32,
}

impl Path2DOutlineBuilder<'_> {
    fn map(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.origin_x + x * self.scale,
            self.origin_y - y * self.scale,
        )
    }
}

impl rustybuzz::ttf_parser::OutlineBuilder for Path2DOutlineBuilder<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (mx, my) = self.map(x, y);
        self.path.move_to(mx, my);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (mx, my) = self.map(x, y);
        self.path.line_to(mx, my);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (cpx, cpy) = self.map(x1, y1);
        let (mx, my) = self.map(x, y);
        self.path.quadratic_curve_to(cpx, cpy, mx, my);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (c1x, c1y) = self.map(x1, y1);
        let (c2x, c2y) = self.map(x2, y2);
        let (mx, my) = self.map(x, y);
        self.path.bezier_curve_to(c1x, c1y, c2x, c2y, mx, my);
    }
    fn close(&mut self) {
        self.path.close_path();
    }
}

/// Append text outlines to a path for `strokeText`.
pub fn append_text_outline_to_path(
    path: &mut crate::canvas::path::Path2D,
    text: &str,
    x: f32,
    y: f32,
    font: &ParsedFont,
    os_name: &str,
) -> bool {
    if text.is_empty() {
        return false;
    }
    let Some((data, idx)) = resolve_face(font, os_name) else {
        return false;
    };
    let Ok(ttf_face) = rustybuzz::ttf_parser::Face::parse(data, idx) else {
        return false;
    };
    let upem = ttf_face.units_per_em() as f32;
    if upem <= 0.0 {
        return false;
    }
    let scale = font.size_px / upem;
    let run = shaper::shape(text, data, idx, font.size_px);

    let mut any_outline = false;
    let mut cursor_x = x;
    for glyph in &run.glyphs {
        let glyph_origin_x = cursor_x + glyph.x_offset;
        let glyph_origin_y = y - glyph.y_offset;
        let mut builder = Path2DOutlineBuilder {
            path,
            origin_x: glyph_origin_x,
            origin_y: glyph_origin_y,
            scale,
        };
        if ttf_face
            .outline_glyph(
                rustybuzz::ttf_parser::GlyphId(glyph.glyph_id as u16),
                &mut builder,
            )
            .is_some()
        {
            any_outline = true;
        }
        cursor_x += glyph.x_advance;
    }
    any_outline
}

/// Composite a glyph onto a premultiplied-RGBA pixel buffer.
pub fn composite_glyph(
    glyph: &PlacedGlyph,
    pixels: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
) {
    for gy in 0..glyph.height {
        for gx in 0..glyph.width {
            let px = glyph.x + gx as i32;
            let py = glyph.y + gy as i32;
            if px < 0 || py < 0 || px >= canvas_width as i32 || py >= canvas_height as i32 {
                continue;
            }

            let cov_idx = (gy * glyph.width + gx) as usize;
            if cov_idx >= glyph.coverage.len() {
                continue;
            }
            let cov = glyph.coverage[cov_idx] as f32 / 255.0;
            if cov < 0.001 {
                continue;
            }

            let a = cov * glyph.alpha;
            if a <= 0.0 {
                continue;
            }

            let dst_idx = ((py as u32 * canvas_width + px as u32) * 4) as usize;
            if dst_idx + 3 >= pixels.len() {
                continue;
            }

            let src_r = glyph.r as f32 * a;
            let src_g = glyph.g as f32 * a;
            let src_b = glyph.b as f32 * a;
            let src_a = 255.0 * a;

            let dst_r = pixels[dst_idx] as f32;
            let dst_g = pixels[dst_idx + 1] as f32;
            let dst_b = pixels[dst_idx + 2] as f32;
            let dst_a = pixels[dst_idx + 3] as f32;

            let inv_src_a = 1.0 - a;
            let out_r = (src_r + dst_r * inv_src_a).min(255.0);
            let out_g = (src_g + dst_g * inv_src_a).min(255.0);
            let out_b = (src_b + dst_b * inv_src_a).min(255.0);
            let out_a = (src_a + dst_a * inv_src_a).min(255.0);

            pixels[dst_idx] = out_r as u8;
            pixels[dst_idx + 1] = out_g as u8;
            pixels[dst_idx + 2] = out_b as u8;
            pixels[dst_idx + 3] = out_a as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measure_hello_world() {
        let font = ParsedFont::parse("16px Arial").expect("should parse");
        let metrics = measure_text_metrics("Hello, World!", &font, "Linux");
        assert!(metrics.width > 30.0 && metrics.width < 200.0);
        assert!(metrics.font_bounding_box_ascent > 10.0);
    }

    #[test]
    fn different_sizes_different_widths() {
        let small = ParsedFont::parse("10px Arial").expect("should parse");
        let big = ParsedFont::parse("40px Arial").expect("should parse");
        let w1 = measure_text_width("Hello", &small, "Linux");
        let w2 = measure_text_width("Hello", &big, "Linux");
        assert!(w2 > w1 * 3.0);
    }
}
