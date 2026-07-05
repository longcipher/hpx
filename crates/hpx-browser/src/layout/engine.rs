use blitz_traits::shell::Viewport;

use crate::{
    dom::{Dom, NodeId},
    layout::query::DOMRect,
};

pub struct LayoutEngine {
    dirty: bool,
    viewport: Viewport,
}

impl LayoutEngine {
    pub fn new(viewport: crate::layout::Viewport) -> Self {
        let blitz_viewport = Viewport {
            window_size: (viewport.width as u32, viewport.height as u32),
            ..Default::default()
        };
        Self {
            dirty: true,
            viewport: blitz_viewport,
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn compute(&mut self, dom: &mut Dom) {
        let inner = dom.inner_mut();
        inner.set_viewport(self.viewport.clone());
        inner.resolve(0.0);
        self.dirty = false;
    }

    pub fn ensure_computed(&mut self, dom: &mut Dom) {
        if self.dirty {
            self.compute(dom);
        }
    }

    pub fn get_bounding_rect(&mut self, dom: &mut Dom, node_id: NodeId) -> DOMRect {
        self.ensure_computed(dom);
        let inner = dom.inner();
        let Some(node) = inner.get_node(node_id.0) else {
            return DOMRect::default();
        };
        let layout = &node.final_layout;
        let (abs_x, abs_y) = self.absolute_position(inner, node_id.0);
        DOMRect::new(
            abs_x as f64,
            abs_y as f64,
            layout.size.width as f64,
            layout.size.height as f64,
        )
    }

    pub fn get_computed_style(
        &mut self,
        _dom: &mut Dom,
        _node_id: NodeId,
    ) -> crate::dom::ElementData {
        crate::dom::ElementData {
            name: crate::dom::QualName::new(""),
            attrs: vec![],
            shadow_root: None,
        }
    }

    pub fn get_offset_width(&mut self, dom: &mut Dom, node_id: NodeId) -> f64 {
        self.ensure_computed(dom);
        self.node_size(dom, node_id).0
    }

    pub fn get_offset_height(&mut self, dom: &mut Dom, node_id: NodeId) -> f64 {
        self.ensure_computed(dom);
        self.node_size(dom, node_id).1
    }

    pub fn get_offset_top(&mut self, dom: &mut Dom, node_id: NodeId) -> f64 {
        self.ensure_computed(dom);
        self.node_position(dom, node_id).1
    }

    pub fn get_offset_left(&mut self, dom: &mut Dom, node_id: NodeId) -> f64 {
        self.ensure_computed(dom);
        self.node_position(dom, node_id).0
    }

    fn absolute_position(&self, inner: &blitz_dom::BaseDocument, node_id: usize) -> (f32, f32) {
        let mut x = 0.0f32;
        let mut y = 0.0f32;
        let mut current_id = node_id;
        while let Some(node) = inner.get_node(current_id) {
            x += node.final_layout.location.x;
            y += node.final_layout.location.y;
            match node.parent {
                Some(pid) => current_id = pid,
                None => break,
            }
        }
        (x, y)
    }

    fn node_size(&self, dom: &Dom, node_id: NodeId) -> (f64, f64) {
        let inner = dom.inner();
        let Some(node) = inner.get_node(node_id.0) else {
            return (0.0, 0.0);
        };
        (
            node.final_layout.size.width as f64,
            node.final_layout.size.height as f64,
        )
    }

    fn node_position(&self, dom: &Dom, node_id: NodeId) -> (f64, f64) {
        let inner = dom.inner();
        let Some(node) = inner.get_node(node_id.0) else {
            return (0.0, 0.0);
        };
        (
            node.final_layout.location.x as f64,
            node.final_layout.location.y as f64,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::{Attribute, Dom, QualName};

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
        let mut dom = make_dom_with_styled_div("width: 200px; height: 100px");
        let viewport = crate::layout::Viewport::new(1920.0, 1080.0);
        let mut engine = LayoutEngine::new(viewport);
        engine.compute(&mut dom);

        let html = dom.child_elements(NodeId::DOCUMENT)[0];
        let body = dom.child_elements(html)[0];
        let div = dom.child_elements(body)[0];

        let rect = engine.get_bounding_rect(&mut dom, div);
        assert!(
            rect.width >= 200.0,
            "width should be >= 200, got {}",
            rect.width
        );
    }

    #[test]
    fn dirty_tracking() {
        let mut dom = make_dom_with_styled_div("width: 100px");
        let viewport = crate::layout::Viewport::new(1920.0, 1080.0);
        let mut engine = LayoutEngine::new(viewport);

        assert!(engine.dirty);
        engine.compute(&mut dom);
        assert!(!engine.dirty);
        engine.mark_dirty();
        assert!(engine.dirty);
    }

    fn find_child_by_tag(dom: &Dom, parent: NodeId, tag: &str) -> Option<NodeId> {
        dom.child_elements(parent).into_iter().find(|&id| {
            dom.get(id)
                .and_then(|n| n.as_element().cloned())
                .is_some_and(|e| e.name.local == tag)
        })
    }

    fn find_child_by_class(dom: &Dom, parent: NodeId, class_part: &str) -> Option<NodeId> {
        dom.child_elements(parent).into_iter().find(|&id| {
            dom.get(id)
                .and_then(|n| n.as_element().cloned())
                .is_some_and(|e| {
                    e.attrs
                        .iter()
                        .any(|a| a.name.local == "class" && a.value.contains(class_part))
                })
        })
    }

    #[test]
    fn full_rendering_pipeline_html_parse_resolve_layout() {
        let html = r#"<!DOCTYPE html>
<html>
<body style="margin: 0; padding: 0;">
    <div style="display: flex; width: 800px; height: 600px;">
        <div style="width: 200px; height: 100%;">
            <p>Nav 1</p>
            <p>Nav 2</p>
        </div>
        <div style="flex-grow: 1; height: 100%;">
            <div style="width: 100%; height: 80px;">
                <h1>Page Title</h1>
            </div>
            <div style="padding: 20px;">
                <p>Hello World</p>
                <p>Second paragraph</p>
            </div>
        </div>
    </div>
</body>
</html>"#;

        let mut dom = crate::html_parser::parse_html(html);
        let viewport = crate::layout::Viewport::new(1920.0, 1080.0);
        let mut engine = LayoutEngine::new(viewport);
        engine.compute(&mut dom);

        let body = find_body(&dom);

        let container_id = dom.child_elements(body)[0];

        let container_rect = engine.get_bounding_rect(&mut dom, container_id);
        assert!(
            container_rect.width >= 790.0,
            "container width should be ~800px, got {}",
            container_rect.width
        );
        assert!(
            container_rect.height >= 590.0,
            "container height should be ~600px, got {}",
            container_rect.height
        );

        let children = dom.child_elements(container_id);
        assert!(
            children.len() >= 2,
            "container should have 2 flex children, got {}",
            children.len()
        );

        let sidebar_rect = engine.get_bounding_rect(&mut dom, children[0]);
        let main_rect = engine.get_bounding_rect(&mut dom, children[1]);

        assert!(
            sidebar_rect.width >= 190.0,
            "sidebar width should be ~200px, got {}",
            sidebar_rect.width
        );
        assert!(
            main_rect.width >= 500.0,
            "main width should fill remaining ~600px, got {}",
            main_rect.width
        );
        assert!(
            sidebar_rect.height >= 500.0,
            "sidebar height should be substantial, got {}",
            sidebar_rect.height
        );

        let main_children = dom.child_elements(children[1]);
        assert!(
            main_children.len() >= 2,
            "main should have header + content, got {}",
            main_children.len()
        );

        let header_rect = engine.get_bounding_rect(&mut dom, main_children[0]);
        assert!(
            header_rect.height >= 70.0,
            "header height should be ~80px, got {}",
            header_rect.height
        );

        let h1_id = dom.child_elements(main_children[0])[0];
        let h1_text = dom.text_content(h1_id);
        assert_eq!(h1_text, "Page Title");
        let h1_rect = engine.get_bounding_rect(&mut dom, h1_id);
        assert!(
            h1_rect.height >= 20.0,
            "h1 should have height, got {}",
            h1_rect.height
        );

        let content_id = main_children[1];
        let paragraphs = dom.get_elements_by_tag_name(content_id, "p");
        assert_eq!(paragraphs.len(), 2, "should have 2 paragraphs in content");
        let p1_text = dom.text_content(paragraphs[0]);
        let p2_text = dom.text_content(paragraphs[1]);
        assert_eq!(p1_text, "Hello World");
        assert_eq!(p2_text, "Second paragraph");
    }

    #[test]
    fn layout_with_flexbox_grow() {
        let html = r#"<!DOCTYPE html>
<html>
<body>
    <div style="display: flex; width: 600px; height: 200px;">
        <div style="width: 100px; height: 100px;">A</div>
        <div style="flex-grow: 1; height: 100px;">B</div>
        <div style="width: 150px; height: 100px;">C</div>
    </div>
</body>
</html>"#;

        let mut dom = crate::html_parser::parse_html(html);
        let viewport = crate::layout::Viewport::new(1920.0, 1080.0);
        let mut engine = LayoutEngine::new(viewport);
        engine.compute(&mut dom);

        let body = find_body(&dom);
        let body_children = dom.child_elements(body);

        let flex_div = body_children
            .iter()
            .find(|&&id| {
                dom.get(id)
                    .and_then(|n| n.as_element().cloned())
                    .is_some_and(|e| {
                        e.attrs
                            .iter()
                            .any(|a| a.name.local == "style" && a.value.contains("flex"))
                    })
            })
            .copied()
            .unwrap_or(body_children[0]);

        let flex_rect = engine.get_bounding_rect(&mut dom, flex_div);
        assert!(
            flex_rect.width >= 590.0,
            "flex container width should be ~600px, got {}",
            flex_rect.width
        );
        assert!(
            flex_rect.height >= 190.0,
            "flex container height should be ~200px, got {}",
            flex_rect.height
        );

        let children = dom.child_elements(flex_div);
        assert!(
            children.len() >= 3,
            "should have at least 3 flex children, got {}",
            children.len()
        );

        let a_rect = engine.get_bounding_rect(&mut dom, children[0]);
        let b_rect = engine.get_bounding_rect(&mut dom, children[1]);
        let c_rect = engine.get_bounding_rect(&mut dom, children[2]);

        assert!(
            a_rect.width >= 90.0,
            "A width should be ~100px, got {}",
            a_rect.width
        );
        assert!(
            c_rect.width >= 140.0,
            "C width should be ~150px, got {}",
            c_rect.width
        );
        assert!(
            b_rect.width >= 300.0,
            "B should fill remaining space ~350px, got {}",
            b_rect.width
        );
        assert!(
            b_rect.x > a_rect.x + a_rect.width - 1.0,
            "B should be to the right of A"
        );
        assert!(
            c_rect.x > b_rect.x + b_rect.width - 1.0,
            "C should be to the right of B"
        );
    }

    #[test]
    fn layout_style_block_resolves() {
        let html = r#"<!DOCTYPE html>
<html>
<head>
    <style>
        .flex-box { display: flex; width: 400px; height: 200px; }
        .child-a { width: 100px; }
        .child-b { flex-grow: 1; }
    </style>
</head>
<body>
    <div class="flex-box">
        <div class="child-a">A</div>
        <div class="child-b">B</div>
    </div>
</body>
</html>"#;

        let mut dom = crate::html_parser::parse_html(html);
        let viewport = crate::layout::Viewport::new(1920.0, 1080.0);
        let mut engine = LayoutEngine::new(viewport);
        engine.compute(&mut dom);

        let body = find_body(&dom);
        let body_children = dom.child_elements(body);
        let flex_div = body_children
            .iter()
            .find(|&&id| {
                dom.get(id)
                    .and_then(|n| n.as_element().cloned())
                    .is_some_and(|e| {
                        e.attrs
                            .iter()
                            .any(|a| a.name.local == "class" && a.value.contains("flex-box"))
                    })
            })
            .copied();

        if let Some(flex_id) = flex_div {
            let rect = engine.get_bounding_rect(&mut dom, flex_id);
            assert!(
                rect.width >= 390.0,
                "style block flex container width should be ~400px, got {}",
                rect.width
            );
        } else {
            panic!("should have flex-box element");
        }
    }

    #[test]
    fn debug_tree_structure() {
        let html = r#"<!DOCTYPE html>
<html>
<body>
    <div class="container">
        <div class="sidebar"><p>Nav</p></div>
        <div class="main"><h1>Title</h1></div>
    </div>
</body>
</html>"#;
        let mut dom = crate::html_parser::parse_html(html);
        let viewport = crate::layout::Viewport::new(1920.0, 1080.0);
        let mut engine = LayoutEngine::new(viewport);
        engine.compute(&mut dom);

        fn print_tree(dom: &Dom, id: NodeId, depth: usize) {
            let node = match dom.get(id) {
                Some(n) => n,
                None => return,
            };
            let indent = "  ".repeat(depth);
            let tag = node
                .as_element()
                .map(|e| {
                    format!(
                        "{} [class={:?}]",
                        e.name.local,
                        e.attrs
                            .iter()
                            .find(|a| a.name.local == "class")
                            .map(|a| &a.value)
                    )
                })
                .unwrap_or_else(|| format!("{:?}", node.data));
            let rect = dom
                .inner()
                .get_node(id.0)
                .map(|n| {
                    let l = &n.final_layout;
                    format!(
                        "({:.0},{:.0} {:.0}x{:.0})",
                        l.location.x, l.location.y, l.size.width, l.size.height
                    )
                })
                .unwrap_or_default();
            println!("{}{} {}", indent, tag, rect);
            for child_id in dom.children(id) {
                print_tree(dom, child_id, depth + 1);
            }
        }

        print_tree(&dom, NodeId::DOCUMENT, 0);
    }

    fn find_body(dom: &Dom) -> NodeId {
        let html = dom.child_elements(NodeId::DOCUMENT)[0];
        dom.child_elements(html)
            .iter()
            .find(|&&id| {
                dom.get(id)
                    .and_then(|n| n.as_element().cloned())
                    .is_some_and(|e| e.name.local == "body")
            })
            .copied()
            .unwrap_or(html)
    }

    #[test]
    fn layout_with_padding_and_margin() {
        let html = r#"<!DOCTYPE html>
<html>
<body>
    <div style="width: 300px; height: 200px; padding: 10px; margin: 20px;">
        <div style="width: 100%; height: 100%;"></div>
    </div>
</body>
</html>"#;

        let mut dom = crate::html_parser::parse_html(html);
        let viewport = crate::layout::Viewport::new(1920.0, 1080.0);
        let mut engine = LayoutEngine::new(viewport);
        engine.compute(&mut dom);

        let body = find_body(&dom);
        let body_children = dom.child_elements(body);
        let outer_div = body_children
            .iter()
            .find(|&&id| {
                dom.get(id)
                    .and_then(|n| n.as_element().cloned())
                    .is_some_and(|e| {
                        e.attrs
                            .iter()
                            .any(|a| a.name.local == "style" && a.value.contains("300px"))
                    })
            })
            .copied()
            .expect("should find div with 300px style");

        let outer_rect = engine.get_bounding_rect(&mut dom, outer_div);
        assert!(
            outer_rect.width > 0.0,
            "outer div should have non-zero width, got {}",
            outer_rect.width
        );
        assert!(
            outer_rect.height > 0.0,
            "outer div should have non-zero height, got {}",
            outer_rect.height
        );
    }
}
