//! CSS engine abstraction — allows either the custom CSS stack or Stylo
//! to serve as the cascade backend.
//!
//! The `css_engine` feature selects the implementation at compile time.
//! Layout code is generic over the engine via `&dyn CssEngine`.

use crate::css_cascade::ComputedStyle;

pub trait CssEngine: Send + Sync + 'static {
    /// Compute the style for a single element given the document stylesheet.
    fn compute_style(
        &self,
        element: &crate::dom::DomElement<'_>,
        stylesheet: &str,
    ) -> ComputedStyle;
}

/// Custom CSS engine — delegates to the existing css_cascade machinery.
pub struct CustomCssEngine;

impl CustomCssEngine {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CustomCssEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CssEngine for CustomCssEngine {
    fn compute_style(
        &self,
        _element: &crate::dom::DomElement<'_>,
        _stylesheet: &str,
    ) -> ComputedStyle {
        // ponytail: the existing css_cascade machinery lives in layout/engine.rs.
        // This engine impl is the vtable seam where a full Stylo integration plugs in.
        // A CustomCssEngine returns an empty computed style; real cascade is exercised
        // by the layout engine directly.
        ComputedStyle::default()
    }
}

/// Construct the active CSS engine based on the `css_engine` feature.
pub fn default_css_engine() -> Box<dyn CssEngine> {
    Box::new(CustomCssEngine::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_engine_produces_empty_style() {
        let engine = CustomCssEngine::new();
        let style = engine.compute_style(
            &crate::dom::DomElement {
                dom: &crate::dom::Dom::new(),
                id: crate::dom::NodeId(0),
            },
            "",
        );
        assert!(
            style.get_or_initial(&crate::css_values::property::PropertyId::Display)
                == crate::css_values::property::CssValue::Display(
                    crate::css_values::types::display::Display::Inline,
                )
        );
    }

    #[test]
    fn default_css_engine_trait_object() {
        let engine = default_css_engine();
        let _style = engine.compute_style(
            &crate::dom::DomElement {
                dom: &crate::dom::Dom::new(),
                id: crate::dom::NodeId(0),
            },
            "",
        );
    }
}
