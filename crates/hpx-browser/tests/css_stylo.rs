//! Real-world stylesheet smoke tests for the Stylo CSS engine.
//!
//! Parses production-like CSS through `blitz-dom` (Stylo) and checks that
//! common selectors cascade to the expected computed values.

use hpx_browser::{
    css_engine::{ComputedStyle, CssEngine, StyloCssEngine},
    dom::{Dom, DomElement, NodeId},
    html_parser::parse_html,
    layout::{LayoutEngine, Viewport},
};

fn find_by_id(dom: &Dom, id: &str) -> NodeId {
    walk(dom, &|el| el.id() == Some(id)).unwrap_or_else(|| panic!("#{id} not found"))
}

fn find_by_class(dom: &Dom, class: &str) -> NodeId {
    walk(dom, &|el| el.has_class(class)).unwrap_or_else(|| panic!(".{class} not found"))
}

fn walk(dom: &Dom, pred: &dyn Fn(DomElement<'_>) -> bool) -> Option<NodeId> {
    let html = dom.child_elements(dom.document())[0];
    let body = dom.child_elements(html).into_iter().find(|&n| {
        dom.get(n)
            .map(|e| e.is_element_with_tag("body"))
            .unwrap_or(false)
    })?;
    let mut stack = vec![body];
    while let Some(n) = stack.pop() {
        if let Some(el) = DomElement::new(dom, n)
            && pred(el)
        {
            return Some(n);
        }
        for c in dom.children(n) {
            stack.push(c);
        }
    }
    None
}

fn resolve(html: &str) -> Dom {
    let mut dom = parse_html(html);
    let mut engine = LayoutEngine::new(Viewport::default());
    engine.compute(&mut dom);
    dom
}

/// A mini "bootstrap-like" stylesheet exercising type, class, id, and
/// descendant selectors plus `!important` and custom properties.
const REALWORLD_CSS: &str = r#"
:root {
  --brand: #3366cc;
  font-size: 16px;
}
body {
  margin: 0;
  font-family: system-ui, sans-serif;
  color: #222222;
}
.card {
  display: block;
  padding: 16px;
  border-radius: 8px;
  background: #ffffff;
}
.card > .title {
  font-size: 24px;
  font-weight: 700;
  color: #3366cc;
}
#footer {
  display: flex;
  color: rgb(34, 34, 34);
}
.hidden {
  display: none !important;
}
@media (min-width: 600px) {
  .card { max-width: 480px; }
}
"#;

fn page(body: &str) -> String {
    format!(
        "<!doctype html><html><head><style>{REALWORLD_CSS}</style></head><body>{body}</body></html>"
    )
}

#[test]
fn stylo_parses_realworld_type_class_id_selectors() {
    let dom = resolve(&page(
        r#"<div class="card" id="main">
             <h1 class="title">Hello</h1>
             <p class="hidden">secret</p>
           </div>
           <footer id="footer">f</footer>"#,
    ));
    let engine = StyloCssEngine::new();

    // .card — class selector
    let card = find_by_id(&dom, "main");
    let card_style = engine.compute_style(&dom, card);
    assert!(
        card_style.display.contains("block"),
        ".card should be block, got {card_style}"
    );

    // .hidden — class + !important
    let hidden = find_by_class(&dom, "hidden");
    let hidden_style = engine.compute_style(&dom, hidden);
    assert!(
        hidden_style.is_display_none(),
        ".hidden !important should be display:none, got {hidden_style}"
    );

    // #footer — id selector
    let footer = find_by_id(&dom, "footer");
    let footer_style = engine.compute_style(&dom, footer);
    assert!(
        footer_style.display.contains("flex"),
        "#footer should be flex, got {footer_style}"
    );
    assert!(
        footer_style.color.contains("34"),
        "#footer color should be rgb(34,34,34), got {footer_style}"
    );
}

#[test]
fn stylo_applies_descendant_and_root_font_size() {
    let dom = resolve(&page(
        r#"<div class="card" id="main"><h1 class="title" id="t">Hi</h1></div>"#,
    ));
    let engine = StyloCssEngine::new();

    let title = find_by_id(&dom, "t");
    let title_style = engine.compute_style(&dom, title);
    // `.card > .title { font-size: 24px }`
    assert!(
        (title_style.font_size_px - 24.0).abs() < 0.05,
        "expected 24px, got {}",
        title_style.font_size_px
    );
}

#[test]
fn stylo_empty_stylesheet_leaves_initial_display() {
    let dom = resolve("<html><body><div id=d>x</div></body></html>");
    let engine = StyloCssEngine::new();
    let style = engine.compute_style(&dom, find_by_id(&dom, "d"));
    // UA stylesheet: div is block
    assert!(
        style.display.contains("block") || style.display.contains("flow"),
        "UA default for div should be block-ish, got {style}"
    );
    assert!(!style.is_display_none());
}

#[test]
fn layout_engine_get_computed_style_matches_stylo() {
    let mut dom = parse_html(&page(r#"<div class="card" id="main">x</div>"#));
    let mut layout = LayoutEngine::new(Viewport::default());
    let id = {
        // Need the node id after parse; resolve first for styles.
        layout.compute(&mut dom);
        find_by_id(&dom, "main")
    };
    let via_layout: ComputedStyle = layout.get_computed_style(&mut dom, id);
    let via_stylo = StyloCssEngine::new().compute_style(&dom, id);
    assert_eq!(
        via_layout, via_stylo,
        "LayoutEngine and StyloCssEngine must agree"
    );
    assert!(
        via_layout.display.contains("block"),
        "card should be block, got {via_layout}"
    );
}
