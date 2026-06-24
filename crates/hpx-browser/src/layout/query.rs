use serde::Serialize;

use crate::layout::layout_unit::LayoutUnit;

#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct DOMRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl DOMRect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        let qx = LayoutUnit::from_f64_px(x).to_f64_px();
        let qy = LayoutUnit::from_f64_px(y).to_f64_px();
        let qw = LayoutUnit::from_f64_px(width).to_f64_px();
        let qh = LayoutUnit::from_f64_px(height).to_f64_px();
        Self {
            x: qx,
            y: qy,
            width: qw,
            height: qh,
            top: qy,
            right: qx + qw,
            bottom: qy + qh,
            left: qx,
        }
    }

    pub fn from_taffy_layout(layout: &taffy::Layout) -> Self {
        let x = LayoutUnit::from_taffy_f32(layout.location.x).to_f64_px();
        let y = LayoutUnit::from_taffy_f32(layout.location.y).to_f64_px();
        let w = LayoutUnit::from_taffy_f32(layout.size.width).to_f64_px();
        let h = LayoutUnit::from_taffy_f32(layout.size.height).to_f64_px();
        Self::new(x, y, w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dom_rect_new() {
        let r = DOMRect::new(10.0, 20.0, 100.0, 50.0);
        assert_eq!(r.x, 10.0);
        assert_eq!(r.y, 20.0);
        assert_eq!(r.width, 100.0);
        assert_eq!(r.height, 50.0);
        assert_eq!(r.top, 20.0);
        assert_eq!(r.right, 110.0);
        assert_eq!(r.bottom, 70.0);
        assert_eq!(r.left, 10.0);
    }

    #[test]
    fn dom_rect_quantized() {
        let r = DOMRect::new(1.3, 0.0, 200.0, 100.0);
        assert_eq!(r.x, 1.296875);
    }
}
