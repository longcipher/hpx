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
