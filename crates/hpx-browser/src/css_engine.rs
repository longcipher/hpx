//! CSS engine abstraction — allows either a lightweight snapshot extractor
//! or the full Stylo cascade (via `blitz-dom`) to serve computed styles.
//!
//! `blitz-dom` embeds Servo's Stylo (`stylo_taffy`) as its cascade backend.
//! [`StyloCssEngine`] reads those computed values back out of the resolved
//! node tree. [`CustomCssEngine`] remains as a zero-cost placeholder for
//! tests and for callers that only need the trait shape.

use std::fmt;

/// Snapshot of a few longhands that CDP / layout queries need most often.
///
/// Values are rendered as strings so this module does not have to name
/// Stylo's concrete value types (they live in the `style` crate, pulled in
/// transitively by `blitz-dom`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ComputedStyle {
    /// `display` as a CSS keyword (`block`, `inline`, `flex`, …).
    pub display: String,
    /// `color` rendered as CSS (`rgb(…)` / `#rrggbb` / `transparent`).
    pub color: String,
    /// `font-size` in CSS pixels.
    pub font_size_px: f32,
    /// `visibility` keyword.
    pub visibility: String,
    /// `opacity` as 0.0–1.0.
    pub opacity: f32,
}

impl ComputedStyle {
    /// True when the element is not rendered (`display: none`).
    #[must_use]
    pub fn is_display_none(&self) -> bool {
        self.display.contains("none")
    }
}

impl fmt::Display for ComputedStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "display={}; color={}; font-size={}px; visibility={}; opacity={}",
            self.display, self.color, self.font_size_px, self.visibility, self.opacity
        )
    }
}

/// Computes styles for a single element given a document stylesheet.
pub trait CssEngine: Send + Sync + 'static {
    /// Engine name (`"custom"` / `"stylo"`).
    fn name(&self) -> &'static str;

    /// Compute the style for `node_id` inside `dom`.
    ///
    /// Returns [`ComputedStyle::default`] when the node is missing or has no
    /// resolved styles yet (callers should `resolve()` first).
    fn compute_style(&self, dom: &crate::dom::Dom, node_id: crate::dom::NodeId) -> ComputedStyle;
}

/// Placeholder engine — returns empty styles.
///
/// Useful as a baseline and for unit tests of the trait shape. Real cascade
/// lives in `blitz-dom` and is surfaced by [`StyloCssEngine`].
#[derive(Debug, Default, Clone, Copy)]
pub struct CustomCssEngine;

impl CustomCssEngine {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl CssEngine for CustomCssEngine {
    fn name(&self) -> &'static str {
        "custom"
    }

    fn compute_style(&self, _dom: &crate::dom::Dom, _node_id: crate::dom::NodeId) -> ComputedStyle {
        ComputedStyle::default()
    }
}

/// Stylo-backed engine — reads computed styles out of a resolved `blitz-dom`.
#[derive(Debug, Default, Clone, Copy)]
pub struct StyloCssEngine;

impl StyloCssEngine {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl CssEngine for StyloCssEngine {
    fn name(&self) -> &'static str {
        "stylo"
    }

    fn compute_style(&self, dom: &crate::dom::Dom, node_id: crate::dom::NodeId) -> ComputedStyle {
        extract_style(dom, node_id)
    }
}

/// Extract a [`ComputedStyle`] snapshot from the resolved Stylo node.
///
/// Requires `BaseDocument::resolve` to have run; otherwise the node has no
/// `primary_styles` and the snapshot is empty.
#[must_use]
pub fn extract_style(dom: &crate::dom::Dom, node_id: crate::dom::NodeId) -> ComputedStyle {
    let Some(node) = dom.inner().get_node(node_id.to_blitz()) else {
        return ComputedStyle::default();
    };
    let Some(styles) = node.primary_styles() else {
        return ComputedStyle::default();
    };

    // Stylo's `Display` is a bitfield (`display(514)`); render via
    // `outside()` / `inside()` enum variants instead.
    let display_obj = styles.clone_display();
    let display = format!(
        "{}/{}",
        format!("{:?}", display_obj.outside()).to_ascii_lowercase(),
        format!("{:?}", display_obj.inside()).to_ascii_lowercase()
    );
    let color = format!("{:?}", styles.clone_color());
    let font_size_px = styles.clone_font_size().used_size().px();
    // `clone_visibility` / `clone_opacity` exist on ComputedValues; fall back
    // safely if a future stylo rename drops them (we only read Debug here).
    let visibility = debug_lower(styles.clone_visibility());
    let opacity = styles.clone_opacity();

    ComputedStyle {
        display,
        color,
        font_size_px,
        visibility,
        opacity,
    }
}

fn debug_lower<T: fmt::Debug>(value: T) -> String {
    format!("{value:?}").to_ascii_lowercase()
}

/// Construct the default CSS engine (Stylo via blitz).
#[must_use]
pub fn default_css_engine() -> Box<dyn CssEngine> {
    Box::new(StyloCssEngine::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html_parser::parse_html;

    fn first_div(dom: &crate::dom::Dom) -> crate::dom::NodeId {
        let html = dom.child_elements(dom.document())[0];
        let body = dom
            .child_elements(html)
            .into_iter()
            .find(|&id| {
                dom.get(id)
                    .map(|n| n.is_element_with_tag("body"))
                    .unwrap_or(false)
            })
            .expect("body");
        dom.child_elements(body)[0]
    }

    #[test]
    fn custom_engine_is_named_and_empty() {
        let engine = CustomCssEngine::new();
        assert_eq!(engine.name(), "custom");
        let dom = parse_html("<div>x</div>");
        let style = engine.compute_style(&dom, crate::dom::NodeId(0));
        assert_eq!(style, ComputedStyle::default());
        assert!(!style.is_display_none());
    }

    #[test]
    fn stylo_engine_reads_display_block() {
        let mut dom = parse_html(
            r#"<html><body><div id="a" style="display:block; color: rgb(1, 2, 3); font-size: 20px">x</div></body></html>"#,
        );
        // Resolve so Stylo populates primary_styles.
        let mut engine = crate::layout::LayoutEngine::new(crate::layout::Viewport::default());
        engine.compute(&mut dom);

        let id = first_div(&dom);
        let style = StyloCssEngine::new().compute_style(&dom, id);
        assert_eq!(StyloCssEngine::new().name(), "stylo");
        assert!(
            style.display.contains("block"),
            "display should be block-like, got {}",
            style.display
        );
        assert!(
            style.display.contains("flow") || style.display.contains("flowroot"),
            "inside should be flow/flow-root, got {}",
            style.display
        );
        assert!(!style.is_display_none());
        assert!(
            (style.font_size_px - 20.0).abs() < 0.01,
            "font-size should be 20px, got {}",
            style.font_size_px
        );
        assert!(
            style.color.contains("1") && style.color.contains("2") && style.color.contains("3"),
            "color should carry rgb(1,2,3), got {}",
            style.color
        );
    }

    #[test]
    fn stylo_engine_reads_display_none() {
        let mut dom =
            parse_html(r#"<html><body><div style="display:none">hidden</div></body></html>"#);
        let mut engine = crate::layout::LayoutEngine::new(crate::layout::Viewport::default());
        engine.compute(&mut dom);
        let id = first_div(&dom);
        let style = StyloCssEngine::new().compute_style(&dom, id);
        assert!(
            style.display.contains("none"),
            "expected display:none, got {}",
            style.display
        );
        assert!(style.is_display_none());
    }

    #[test]
    fn default_engine_is_stylo() {
        assert_eq!(default_css_engine().name(), "stylo");
    }
}
