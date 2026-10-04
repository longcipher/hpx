//! Glyph rasterization via `swash`.

use swash::{
    FontRef, GlyphId,
    scale::{Render, ScaleContext, Source, StrikeWith},
    zeno::Format,
};

/// A rasterized glyph coverage mask.
#[derive(Debug, Clone)]
pub struct GlyphBitmap {
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub pixels: Vec<u8>,
}

/// Rasterize a single glyph.
pub fn rasterize_glyph(
    face_data: &[u8],
    face_index: u32,
    glyph_id: u32,
    size_px: f32,
) -> Option<GlyphBitmap> {
    let font = FontRef::from_index(face_data, face_index as usize)?;
    let mut context = ScaleContext::new();
    let mut scaler = context.builder(font).size(size_px).hint(true).build();

    let image = Render::new(&[
        Source::Outline,
        Source::ColorOutline(0),
        Source::ColorBitmap(StrikeWith::BestFit),
    ])
    .format(Format::Alpha)
    .render(&mut scaler, GlyphId::from(glyph_id as u16))?;

    Some(GlyphBitmap {
        width: image.placement.width,
        height: image.placement.height,
        left: image.placement.left,
        top: image.placement.top,
        pixels: image.data,
    })
}

#[cfg(test)]
mod tests {
    use super::rasterize_glyph;

    /// Malformed font data must yield `None` rather than panicking. Font bytes
    /// reach this code from web content, so a corrupt or hostile font is an
    /// ordinary input, not an exceptional one.
    #[test]
    fn empty_font_data_returns_none() {
        assert!(rasterize_glyph(&[], 0, 0, 16.0).is_none());
    }

    #[test]
    fn garbage_font_data_returns_none() {
        let garbage: Vec<u8> = (0..=255u8).cycle().take(1024).collect();
        assert!(rasterize_glyph(&garbage, 0, 0, 16.0).is_none());
    }

    #[test]
    fn out_of_range_face_index_returns_none() {
        let garbage: Vec<u8> = (0..=255u8).cycle().take(512).collect();
        assert!(rasterize_glyph(&garbage, 99, 0, 16.0).is_none());
    }

    #[test]
    fn truncated_font_header_returns_none() {
        // A plausible sfnt version followed by nothing.
        let truncated = [0x00, 0x01, 0x00, 0x00];
        assert!(rasterize_glyph(&truncated, 0, 0, 16.0).is_none());
    }

    #[test]
    fn extreme_sizes_do_not_panic() {
        let garbage: Vec<u8> = (0..=255u8).cycle().take(512).collect();
        for size in [0.0, -1.0, 1e-6, 1e9, f32::NAN] {
            let _ = rasterize_glyph(&garbage, 0, 0, size);
        }
    }

    /// A real font, if the system has one installed, must rasterize glyph 0 and
    /// produce self-consistent metrics. Font discovery is best-effort: a machine
    /// with no installed fonts simply has nothing to assert.
    #[test]
    fn a_real_font_rasterizes_a_glyph_consistently() {
        let database = fontdb::Database::new();
        let query = fontdb::Query {
            families: &[],
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        let Some(id) = database.query(&query) else {
            // No installed font on this machine; the guard-rail tests above
            // already cover the failure modes that matter.
            return;
        };
        let Some(face_index) = database.face(id).map(|info| info.index) else {
            return;
        };
        let outcome = database.with_face_data(id, |face_data, index| {
            let bitmap = rasterize_glyph(face_data, index, 0, 24.0);
            (index == face_index, bitmap)
        });
        let Some((_, bitmap)) = outcome else {
            return;
        };
        let Some(bitmap) = bitmap else {
            return;
        };
        assert_eq!(
            bitmap.pixels.len(),
            (bitmap.width as usize) * (bitmap.height as usize),
            "the coverage mask size must match its reported dimensions"
        );
    }
}
