use std::collections::{HashMap, HashSet};

use crate::{
    css_cascade::ComputedStyle,
    css_parser::{self, ComponentValue, TokenKind},
    css_values::{
        property::{CssValue, PropertyId},
        types::{display as css_display, font as css_font, length as css_length},
    },
    dom::{Dom, NodeData, NodeId},
    layout::{
        query::DOMRect, resolve::ResolveContext, style_map::computed_to_taffy, viewport::Viewport,
    },
};

const LAYOUT_BUILD_LIMIT: usize = 100_000;

pub struct LayoutEngine {
    tree: taffy::TaffyTree,
    dom_to_taffy: HashMap<u32, taffy::NodeId>,
    viewport: Viewport,
    dirty: bool,
    root_taffy: Option<taffy::NodeId>,
}

impl LayoutEngine {
    pub fn new(viewport: Viewport) -> Self {
        Self {
            tree: taffy::TaffyTree::new(),
            dom_to_taffy: HashMap::new(),
            viewport,
            dirty: true,
            root_taffy: None,
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn compute(&mut self, dom: &Dom) {
        self.tree = taffy::TaffyTree::new();
        self.dom_to_taffy.clear();

        let ctx = ResolveContext {
            font_size: 16.0,
            root_font_size: 16.0,
            viewport_w: self.viewport.width,
            viewport_h: self.viewport.height,
        };

        let root = self.build_node(dom, NodeId::DOCUMENT, &ctx);
        self.root_taffy = root;

        if let Some(root_id) = self.root_taffy {
            let avail = taffy::Size {
                width: taffy::AvailableSpace::Definite(self.viewport.width),
                height: taffy::AvailableSpace::Definite(self.viewport.height),
            };
            self.tree.compute_layout(root_id, avail).ok();
        }

        self.dirty = false;
    }

    pub fn ensure_computed(&mut self, dom: &Dom) {
        if self.dirty {
            self.compute(dom);
        }
    }

    pub fn get_bounding_rect(&mut self, dom: &Dom, node_id: NodeId) -> DOMRect {
        self.ensure_computed(dom);

        let taffy_id = match self.dom_to_taffy.get(&node_id.to_raw()) {
            Some(id) => *id,
            None => return DOMRect::default(),
        };

        let layout = match self.tree.layout(taffy_id) {
            Ok(l) => *l,
            Err(_) => return DOMRect::default(),
        };

        let (abs_x, abs_y) = self.absolute_position(taffy_id);

        DOMRect::new(
            abs_x as f64,
            abs_y as f64,
            layout.size.width as f64,
            layout.size.height as f64,
        )
    }

    pub fn get_computed_style(&mut self, dom: &Dom, node_id: NodeId) -> ComputedStyle {
        let node = match dom.get(node_id) {
            Some(n) => n,
            None => return ComputedStyle::default(),
        };
        match &node.data {
            NodeData::Element(elem) => {
                let inline = self.parse_inline_style(elem);
                ComputedStyle::resolve(&inline, None)
            }
            _ => ComputedStyle::default(),
        }
    }

    pub fn get_offset_width(&mut self, dom: &Dom, node_id: NodeId) -> f64 {
        self.ensure_computed(dom);
        self.taffy_size(node_id).0
    }

    pub fn get_offset_height(&mut self, dom: &Dom, node_id: NodeId) -> f64 {
        self.ensure_computed(dom);
        self.taffy_size(node_id).1
    }

    pub fn get_offset_top(&mut self, dom: &Dom, node_id: NodeId) -> f64 {
        self.ensure_computed(dom);
        self.taffy_position(node_id).1
    }

    pub fn get_offset_left(&mut self, dom: &Dom, node_id: NodeId) -> f64 {
        self.ensure_computed(dom);
        self.taffy_position(node_id).0
    }

    // --- Internal ---

    fn build_node(
        &mut self,
        dom: &Dom,
        root: NodeId,
        ctx: &ResolveContext,
    ) -> Option<taffy::NodeId> {
        enum Work {
            Visit(NodeId),
            Finish(NodeId),
        }
        let mut stack: Vec<Work> = vec![Work::Visit(root)];
        let mut visited: HashSet<NodeId> = HashSet::with_capacity(64);
        let mut steps: usize = 0;
        while let Some(work) = stack.pop() {
            match work {
                Work::Visit(node_id) => {
                    if !visited.insert(node_id) {
                        continue;
                    }
                    steps += 1;
                    if steps > LAYOUT_BUILD_LIMIT {
                        panic!(
                            "Layout build cycle from {:?} — visited {} unique nodes",
                            root,
                            visited.len()
                        );
                    }
                    stack.push(Work::Finish(node_id));
                    let kids = dom.children(node_id);
                    for c in kids.into_iter().rev() {
                        stack.push(Work::Visit(c));
                    }
                }
                Work::Finish(node_id) => {
                    self.finish_node(dom, node_id, ctx);
                }
            }
        }
        self.dom_to_taffy.get(&root.to_raw()).copied()
    }

    fn finish_node(&mut self, dom: &Dom, node_id: NodeId, ctx: &ResolveContext) {
        let node = match dom.get(node_id) {
            Some(n) => n,
            None => return,
        };

        let children: Vec<taffy::NodeId> = dom
            .children(node_id)
            .into_iter()
            .filter_map(|cid| self.dom_to_taffy.get(&cid.to_raw()).copied())
            .collect();

        let taffy_id = match &node.data {
            NodeData::Document | NodeData::DocumentFragment => {
                let style = taffy::Style {
                    display: taffy::Display::Block,
                    size: taffy::Size {
                        width: taffy::Dimension::length(ctx.viewport_w),
                        height: taffy::Dimension::auto(),
                    },
                    ..Default::default()
                };
                match self.tree.new_with_children(style, &children) {
                    Ok(id) => id,
                    Err(_) => return,
                }
            }
            NodeData::Element(elem) => {
                let inline_style = self.parse_inline_style(elem);
                let computed = ComputedStyle::resolve(&inline_style, None);
                if let Some(CssValue::Display(css_display::Display::None)) =
                    computed.get(&PropertyId::Display)
                {
                    return;
                }
                let taffy_style = computed_to_taffy(&computed, ctx);
                match self.tree.new_with_children(taffy_style, &children) {
                    Ok(id) => id,
                    Err(_) => return,
                }
            }
            NodeData::Text(text) => {
                let char_count = text.chars().count() as f32;
                let width = char_count * ctx.font_size * 0.6;
                let height = ctx.font_size * 1.2;
                let style = taffy::Style {
                    size: taffy::Size {
                        width: taffy::Dimension::length(width),
                        height: taffy::Dimension::length(height),
                    },
                    ..Default::default()
                };
                match self.tree.new_leaf(style) {
                    Ok(id) => id,
                    Err(_) => return,
                }
            }
            _ => return,
        };
        self.dom_to_taffy.insert(node_id.to_raw(), taffy_id);
    }

    fn parse_inline_style(&self, elem: &crate::dom::ElementData) -> HashMap<PropertyId, CssValue> {
        let mut map = HashMap::new();
        let style_attr = elem.attrs.iter().find(|a| a.name.local == "style");
        let Some(attr) = style_attr else {
            return map;
        };
        let (decls, _errors) = css_parser::parse_declaration_list(&attr.value);
        for decl in &decls {
            let prop_id = PropertyId::from_name(decl.name);
            if let Some(value) = component_values_to_css(&prop_id, &decl.value) {
                map.insert(prop_id, value);
            }
        }
        map
    }

    fn absolute_position(&self, taffy_id: taffy::NodeId) -> (f32, f32) {
        let mut x = 0.0f32;
        let mut y = 0.0f32;
        let mut current = taffy_id;
        loop {
            if let Ok(layout) = self.tree.layout(current) {
                x += layout.location.x;
                y += layout.location.y;
            }
            match self.tree.parent(current) {
                Some(parent) => current = parent,
                None => break,
            }
        }
        (x, y)
    }

    fn taffy_size(&self, node_id: NodeId) -> (f64, f64) {
        match self.dom_to_taffy.get(&node_id.to_raw()) {
            Some(taffy_id) => match self.tree.layout(*taffy_id) {
                Ok(layout) => (
                    crate::layout::layout_unit::LayoutUnit::from_taffy_f32(layout.size.width)
                        .to_f64_px(),
                    crate::layout::layout_unit::LayoutUnit::from_taffy_f32(layout.size.height)
                        .to_f64_px(),
                ),
                Err(_) => (0.0, 0.0),
            },
            None => (0.0, 0.0),
        }
    }

    fn taffy_position(&self, node_id: NodeId) -> (f64, f64) {
        match self.dom_to_taffy.get(&node_id.to_raw()) {
            Some(taffy_id) => match self.tree.layout(*taffy_id) {
                Ok(layout) => (
                    crate::layout::layout_unit::LayoutUnit::from_taffy_f32(layout.location.x)
                        .to_f64_px(),
                    crate::layout::layout_unit::LayoutUnit::from_taffy_f32(layout.location.y)
                        .to_f64_px(),
                ),
                Err(_) => (0.0, 0.0),
            },
            None => (0.0, 0.0),
        }
    }
}

// --- Inline style token → CssValue conversion ---

fn first_token<'a, 'b>(values: &'b [ComponentValue<'a>]) -> Option<&'b ComponentValue<'a>> {
    values
        .iter()
        .find(|v| !matches!(v, ComponentValue::Token(t) if t.kind.is_whitespace()))
}

fn component_values_to_css(prop: &PropertyId, values: &[ComponentValue<'_>]) -> Option<CssValue> {
    let tok = first_token(values)?;
    match tok {
        ComponentValue::Token(t) => match &t.kind {
            TokenKind::Ident(ident) => parse_keyword(prop, ident),
            TokenKind::Dimension { value, unit, .. } => parse_dimension(prop, *value, unit),
            TokenKind::Number { value, .. } => parse_number(prop, *value),
            TokenKind::Percentage { value, .. } => parse_percentage(prop, *value),
            _ => None,
        },
        _ => None,
    }
}

fn parse_keyword(prop: &PropertyId, ident: &str) -> Option<CssValue> {
    let lower = ident.to_ascii_lowercase();
    match prop {
        PropertyId::Display => match lower.as_str() {
            "none" => Some(CssValue::Display(css_display::Display::None)),
            "block" => Some(CssValue::Display(css_display::Display::Block)),
            "inline" => Some(CssValue::Display(css_display::Display::Inline)),
            "inline-block" => Some(CssValue::Display(css_display::Display::InlineBlock)),
            "flex" => Some(CssValue::Display(css_display::Display::Flex)),
            "inline-flex" => Some(CssValue::Display(css_display::Display::InlineFlex)),
            "grid" => Some(CssValue::Display(css_display::Display::Grid)),
            "inline-grid" => Some(CssValue::Display(css_display::Display::InlineGrid)),
            "table" => Some(CssValue::Display(css_display::Display::Table)),
            "contents" => Some(CssValue::Display(css_display::Display::Contents)),
            "flow-root" => Some(CssValue::Display(css_display::Display::FlowRoot)),
            "list-item" => Some(CssValue::Display(css_display::Display::ListItem)),
            _ => None,
        },
        PropertyId::Position => match lower.as_str() {
            "static" => Some(CssValue::Position(css_display::Position::Static)),
            "relative" => Some(CssValue::Position(css_display::Position::Relative)),
            "absolute" => Some(CssValue::Position(css_display::Position::Absolute)),
            "fixed" => Some(CssValue::Position(css_display::Position::Fixed)),
            "sticky" => Some(CssValue::Position(css_display::Position::Sticky)),
            _ => None,
        },
        PropertyId::FlexDirection => match lower.as_str() {
            "row" => Some(CssValue::FlexDirection(css_display::FlexDirection::Row)),
            "row-reverse" => Some(CssValue::FlexDirection(
                css_display::FlexDirection::RowReverse,
            )),
            "column" => Some(CssValue::FlexDirection(css_display::FlexDirection::Column)),
            "column-reverse" => Some(CssValue::FlexDirection(
                css_display::FlexDirection::ColumnReverse,
            )),
            _ => None,
        },
        PropertyId::FlexWrap => match lower.as_str() {
            "nowrap" => Some(CssValue::FlexWrap(css_display::FlexWrap::Nowrap)),
            "wrap" => Some(CssValue::FlexWrap(css_display::FlexWrap::Wrap)),
            "wrap-reverse" => Some(CssValue::FlexWrap(css_display::FlexWrap::WrapReverse)),
            _ => None,
        },
        PropertyId::BoxSizing => match lower.as_str() {
            "content-box" => Some(CssValue::BoxSizing(css_display::BoxSizing::ContentBox)),
            "border-box" => Some(CssValue::BoxSizing(css_display::BoxSizing::BorderBox)),
            _ => None,
        },
        PropertyId::OverflowX | PropertyId::OverflowY => match lower.as_str() {
            "visible" => Some(CssValue::Overflow(css_display::Overflow::Visible)),
            "hidden" => Some(CssValue::Overflow(css_display::Overflow::Hidden)),
            "scroll" => Some(CssValue::Overflow(css_display::Overflow::Scroll)),
            "auto" => Some(CssValue::Overflow(css_display::Overflow::Auto)),
            "clip" => Some(CssValue::Overflow(css_display::Overflow::Clip)),
            _ => None,
        },
        PropertyId::Visibility => match lower.as_str() {
            "visible" => Some(CssValue::Visibility(css_display::Visibility::Visible)),
            "hidden" => Some(CssValue::Visibility(css_display::Visibility::Hidden)),
            "collapse" => Some(CssValue::Visibility(css_display::Visibility::Collapse)),
            _ => None,
        },
        PropertyId::AlignItems
        | PropertyId::AlignSelf
        | PropertyId::AlignContent
        | PropertyId::JustifyContent
        | PropertyId::JustifyItems
        | PropertyId::JustifySelf => match lower.as_str() {
            "normal" => Some(CssValue::Alignment(css_display::AlignmentValue::Normal)),
            "stretch" => Some(CssValue::Alignment(css_display::AlignmentValue::Stretch)),
            "center" => Some(CssValue::Alignment(css_display::AlignmentValue::Center)),
            "start" => Some(CssValue::Alignment(css_display::AlignmentValue::Start)),
            "end" => Some(CssValue::Alignment(css_display::AlignmentValue::End)),
            "flex-start" => Some(CssValue::Alignment(css_display::AlignmentValue::FlexStart)),
            "flex-end" => Some(CssValue::Alignment(css_display::AlignmentValue::FlexEnd)),
            "baseline" => Some(CssValue::Alignment(css_display::AlignmentValue::Baseline)),
            "space-between" => Some(CssValue::Alignment(
                css_display::AlignmentValue::SpaceBetween,
            )),
            "space-around" => Some(CssValue::Alignment(
                css_display::AlignmentValue::SpaceAround,
            )),
            "space-evenly" => Some(CssValue::Alignment(
                css_display::AlignmentValue::SpaceEvenly,
            )),
            _ => None,
        },
        PropertyId::Float => match lower.as_str() {
            "none" => Some(CssValue::Float(css_display::Float::None)),
            "left" => Some(CssValue::Float(css_display::Float::Left)),
            "right" => Some(CssValue::Float(css_display::Float::Right)),
            _ => None,
        },
        PropertyId::Clear => match lower.as_str() {
            "none" => Some(CssValue::Clear(css_display::Clear::None)),
            "left" => Some(CssValue::Clear(css_display::Clear::Left)),
            "right" => Some(CssValue::Clear(css_display::Clear::Right)),
            "both" => Some(CssValue::Clear(css_display::Clear::Both)),
            _ => None,
        },
        _ => None,
    }
}

fn length_from_unit(value: f64, unit: &str) -> Option<css_length::Length> {
    match unit.to_ascii_lowercase().as_str() {
        "px" => Some(css_length::Length::Px(value)),
        "em" => Some(css_length::Length::Em(value)),
        "rem" => Some(css_length::Length::Rem(value)),
        "vw" => Some(css_length::Length::Vw(value)),
        "vh" => Some(css_length::Length::Vh(value)),
        "vmin" => Some(css_length::Length::Vmin(value)),
        "vmax" => Some(css_length::Length::Vmax(value)),
        "cm" => Some(css_length::Length::Cm(value)),
        "mm" => Some(css_length::Length::Mm(value)),
        "in" => Some(css_length::Length::In(value)),
        "pt" => Some(css_length::Length::Pt(value)),
        "pc" => Some(css_length::Length::Pc(value)),
        "ch" => Some(css_length::Length::Ch(value)),
        "ex" => Some(css_length::Length::Ex(value)),
        _ => None,
    }
}

fn parse_dimension(prop: &PropertyId, value: f64, unit: &str) -> Option<CssValue> {
    let length = length_from_unit(value, unit)?;
    match prop {
        PropertyId::Width
        | PropertyId::Height
        | PropertyId::MinWidth
        | PropertyId::MinHeight
        | PropertyId::MaxWidth
        | PropertyId::MaxHeight => Some(CssValue::LengthPercentageAuto(
            css_length::LengthPercentageAuto::Length(length),
        )),
        PropertyId::MarginTop
        | PropertyId::MarginRight
        | PropertyId::MarginBottom
        | PropertyId::MarginLeft => Some(CssValue::LengthPercentageAuto(
            css_length::LengthPercentageAuto::Length(length),
        )),
        PropertyId::PaddingTop
        | PropertyId::PaddingRight
        | PropertyId::PaddingBottom
        | PropertyId::PaddingLeft => Some(CssValue::LengthPercentage(
            css_length::LengthPercentage::Length(length),
        )),
        PropertyId::BorderTopWidth
        | PropertyId::BorderRightWidth
        | PropertyId::BorderBottomWidth
        | PropertyId::BorderLeftWidth => Some(CssValue::Length(length)),
        PropertyId::FlexBasis => Some(CssValue::LengthPercentageAuto(
            css_length::LengthPercentageAuto::Length(length),
        )),
        PropertyId::RowGap | PropertyId::ColumnGap | PropertyId::Gap => Some(
            CssValue::LengthPercentage(css_length::LengthPercentage::Length(length)),
        ),
        PropertyId::FontSize => Some(CssValue::Length(length)),
        _ => None,
    }
}

fn parse_number(prop: &PropertyId, value: f64) -> Option<CssValue> {
    match prop {
        PropertyId::FlexGrow => Some(CssValue::Number(value)),
        PropertyId::FlexShrink => Some(CssValue::Number(value)),
        PropertyId::FlexBasis => {
            if value == 0.0 {
                Some(CssValue::LengthPercentageAuto(
                    css_length::LengthPercentageAuto::Length(css_length::Length::Zero),
                ))
            } else {
                None
            }
        }
        PropertyId::ZIndex => Some(CssValue::Integer(value as i32)),
        PropertyId::Opacity => Some(CssValue::Number(value)),
        PropertyId::FontWeight => {
            let weight = match value as u32 {
                700 => css_font::FontWeight::Bold,
                _ => css_font::FontWeight::Numeric(value),
            };
            Some(CssValue::FontWeight(weight))
        }
        _ => None,
    }
}

fn parse_percentage(prop: &PropertyId, value: f64) -> Option<CssValue> {
    match prop {
        PropertyId::Width
        | PropertyId::Height
        | PropertyId::MinWidth
        | PropertyId::MinHeight
        | PropertyId::MaxWidth
        | PropertyId::MaxHeight => Some(CssValue::LengthPercentageAuto(
            css_length::LengthPercentageAuto::Percentage(value),
        )),
        PropertyId::MarginTop
        | PropertyId::MarginRight
        | PropertyId::MarginBottom
        | PropertyId::MarginLeft => Some(CssValue::LengthPercentageAuto(
            css_length::LengthPercentageAuto::Percentage(value),
        )),
        PropertyId::PaddingTop
        | PropertyId::PaddingRight
        | PropertyId::PaddingBottom
        | PropertyId::PaddingLeft => Some(CssValue::LengthPercentage(
            css_length::LengthPercentage::Percentage(value),
        )),
        PropertyId::RowGap | PropertyId::ColumnGap | PropertyId::Gap => Some(
            CssValue::LengthPercentage(css_length::LengthPercentage::Percentage(value)),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::{Attribute, QualName};

    fn make_dom_with_styled_div(style: &str) -> Dom {
        let mut dom = Dom::new();
        let html = dom.create_element(QualName::new("html"), vec![]);
        dom.append_child(NodeId::DOCUMENT, html);
        let body = dom.create_element(QualName::new("body"), vec![]);
        dom.append_child(html, body);
        let div = dom.create_element(
            QualName::new("div"),
            vec![Attribute {
                name: QualName::new("style"),
                value: style.to_string(),
            }],
        );
        dom.append_child(body, div);
        dom
    }

    #[test]
    fn layout_basic_div() {
        let dom = make_dom_with_styled_div("width: 200px; height: 100px");
        let viewport = Viewport::new(1920.0, 1080.0);
        let mut engine = LayoutEngine::new(viewport);
        engine.compute(&dom);

        let html = dom.child_elements(NodeId::DOCUMENT)[0];
        let body = dom.child_elements(html)[0];
        let div = dom.child_elements(body)[0];

        let rect = engine.get_bounding_rect(&dom, div);
        assert!(
            rect.width >= 200.0,
            "width should be >= 200, got {}",
            rect.width
        );
        assert!(
            rect.height >= 100.0,
            "height should be >= 100, got {}",
            rect.height
        );
    }

    #[test]
    fn layout_text_node_has_size() {
        let mut dom = Dom::new();
        let html = dom.create_element(QualName::new("html"), vec![]);
        dom.append_child(NodeId::DOCUMENT, html);
        let body = dom.create_element(QualName::new("body"), vec![]);
        dom.append_child(html, body);
        let text = dom.create_text("Hello world".to_string());
        dom.append_child(body, text);

        let viewport = Viewport::new(1920.0, 1080.0);
        let mut engine = LayoutEngine::new(viewport);
        engine.compute(&dom);

        let (w, h) = engine.taffy_size(text);
        assert!(w > 0.0, "text width should be > 0, got {}", w);
        assert!(h > 0.0, "text height should be > 0, got {}", h);
    }

    #[test]
    fn layout_offset_width() {
        let dom = make_dom_with_styled_div("width: 300px; height: 150px");
        let viewport = Viewport::new(1920.0, 1080.0);
        let mut engine = LayoutEngine::new(viewport);

        let html = dom.child_elements(NodeId::DOCUMENT)[0];
        let body = dom.child_elements(html)[0];
        let div = dom.child_elements(body)[0];

        let w = engine.get_offset_width(&dom, div);
        assert!(w >= 300.0, "offsetWidth should be >= 300, got {}", w);
        let h = engine.get_offset_height(&dom, div);
        assert!(h >= 150.0, "offsetHeight should be >= 150, got {}", h);
    }

    #[test]
    fn dirty_tracking() {
        let dom = make_dom_with_styled_div("width: 100px");
        let viewport = Viewport::new(1920.0, 1080.0);
        let mut engine = LayoutEngine::new(viewport);

        assert!(engine.dirty);
        engine.compute(&dom);
        assert!(!engine.dirty);
        engine.mark_dirty();
        assert!(engine.dirty);
    }

    #[test]
    fn display_none_hides_node() {
        let dom = make_dom_with_styled_div("display: none");
        let viewport = Viewport::new(1920.0, 1080.0);
        let mut engine = LayoutEngine::new(viewport);
        engine.compute(&dom);

        let html = dom.child_elements(NodeId::DOCUMENT)[0];
        let body = dom.child_elements(html)[0];
        let div = dom.child_elements(body)[0];

        assert!(!engine.dom_to_taffy.contains_key(&div.to_raw()));
    }

    #[test]
    fn flex_layout_sizes() {
        let mut dom = Dom::new();
        let html = dom.create_element(QualName::new("html"), vec![]);
        dom.append_child(NodeId::DOCUMENT, html);
        let body = dom.create_element(QualName::new("body"), vec![]);
        dom.append_child(html, body);
        let flex_container = dom.create_element(
            QualName::new("div"),
            vec![Attribute {
                name: QualName::new("style"),
                value: "display: flex; width: 500px; height: 200px".to_string(),
            }],
        );
        dom.append_child(body, flex_container);
        let child1 = dom.create_element(
            QualName::new("div"),
            vec![Attribute {
                name: QualName::new("style"),
                value: "width: 100px; height: 50px".to_string(),
            }],
        );
        dom.append_child(flex_container, child1);
        let child2 = dom.create_element(
            QualName::new("div"),
            vec![Attribute {
                name: QualName::new("style"),
                value: "width: 150px; height: 60px".to_string(),
            }],
        );
        dom.append_child(flex_container, child2);

        let viewport = Viewport::new(1920.0, 1080.0);
        let mut engine = LayoutEngine::new(viewport);
        engine.compute(&dom);

        let w1 = engine.get_offset_width(&dom, child1);
        assert!(w1 >= 100.0, "child1 width should be >= 100, got {}", w1);

        let w2 = engine.get_offset_width(&dom, child2);
        assert!(w2 >= 150.0, "child2 width should be >= 150, got {}", w2);

        let x1 = engine.get_offset_left(&dom, child1);
        let x2 = engine.get_offset_left(&dom, child2);
        assert!(x2 > x1, "child2 should be to the right of child1");
    }

    #[test]
    fn get_computed_style_returns_resolved() {
        let dom = make_dom_with_styled_div("display: flex; width: 200px");
        let viewport = Viewport::new(1920.0, 1080.0);
        let mut engine = LayoutEngine::new(viewport);

        let html = dom.child_elements(NodeId::DOCUMENT)[0];
        let body = dom.child_elements(html)[0];
        let div = dom.child_elements(body)[0];

        let style = engine.get_computed_style(&dom, div);
        assert_eq!(
            style.get(&PropertyId::Display),
            Some(&CssValue::Display(css_display::Display::Flex))
        );
    }

    #[test]
    fn dom_rect_from_layout() {
        let layout = taffy::Layout::new();
        let rect = DOMRect::from_taffy_layout(&layout);
        assert_eq!(rect.width, 0.0);
    }

    #[test]
    fn margin_offset() {
        let mut dom = Dom::new();
        let html = dom.create_element(QualName::new("html"), vec![]);
        dom.append_child(NodeId::DOCUMENT, html);
        let body = dom.create_element(QualName::new("body"), vec![]);
        dom.append_child(html, body);
        let div = dom.create_element(
            QualName::new("div"),
            vec![Attribute {
                name: QualName::new("style"),
                value: "width: 100px; height: 50px; margin-left: 30px; margin-top: 20px"
                    .to_string(),
            }],
        );
        dom.append_child(body, div);

        let viewport = Viewport::new(1920.0, 1080.0);
        let mut engine = LayoutEngine::new(viewport);
        engine.compute(&dom);

        let x = engine.get_offset_left(&dom, div);
        let y = engine.get_offset_top(&dom, div);
        assert!(x >= 30.0, "offsetLeft should be >= 30, got {}", x);
        assert!(y >= 20.0, "offsetTop should be >= 20, got {}", y);
    }
}
