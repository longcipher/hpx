//! html5ever integration — parse HTML strings into a [`Dom`].

use std::{borrow::Cow, cell::UnsafeCell, collections::HashMap};

use html5ever::{
    Attribute as H5Attribute, ExpandedName, QualName as H5QualName, local_name, ns, parse_document,
    tendril::TendrilSink,
    tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink},
};

use crate::dom::{Attribute, Dom, NodeData, NodeId, QualName};

/// Parse an HTML document string into a DOM tree.
pub fn parse_html(html: &str) -> Dom {
    let sink = DomTreeSink::new();
    parse_document(sink, Default::default())
        .from_utf8()
        .one(html.as_bytes())
}

// ---------------------------------------------------------------------------
// DomTreeSink — bridges html5ever's TreeSink to our Dom
// ---------------------------------------------------------------------------

/// `TreeSink` implementation that builds a DOM.
///
/// Uses `UnsafeCell` because html5ever's `TreeSink` trait takes `&self`
/// but tree building inherently requires mutation.
///
/// # Safety
///
/// `DomTreeSink` is **not `Sync`**. It is owned by a single parsing
/// thread for the lifetime of the parse. html5ever calls `TreeSink`
/// methods serially from that thread — never concurrently, never
/// reentrantly. References handed out by helpers are dropped before
/// the next callback runs.
#[allow(unsafe_code)]
pub struct DomTreeSink {
    dom: UnsafeCell<Dom>,
    quirks_mode: UnsafeCell<QuirksMode>,
    names: UnsafeCell<HashMap<NodeId, H5QualName>>,
}

#[allow(unsafe_code)]
impl DomTreeSink {
    pub fn new() -> Self {
        Self {
            dom: UnsafeCell::new(Dom::new()),
            quirks_mode: UnsafeCell::new(QuirksMode::NoQuirks),
            names: UnsafeCell::new(HashMap::new()),
        }
    }

    fn dom(&self) -> &Dom {
        // SAFETY: single-threaded parser, no concurrent or reentrant access.
        unsafe { &*self.dom.get() }
    }

    #[allow(
        clippy::mut_from_ref,
        reason = "single-threaded non-reentrant parser; &mut-from-&self is sound"
    )]
    fn dom_mut(&self) -> &mut Dom {
        // SAFETY: single-threaded parser, no concurrent or reentrant access.
        unsafe { &mut *self.dom.get() }
    }

    fn names(&self) -> &HashMap<NodeId, H5QualName> {
        // SAFETY: single-threaded parser.
        unsafe { &*self.names.get() }
    }

    #[allow(
        clippy::mut_from_ref,
        reason = "single-threaded non-reentrant parser; &mut-from-&self is sound"
    )]
    fn names_mut(&self) -> &mut HashMap<NodeId, H5QualName> {
        // SAFETY: single-threaded parser.
        unsafe { &mut *self.names.get() }
    }
}

impl Default for DomTreeSink {
    fn default() -> Self {
        Self::new()
    }
}

fn convert_qualname(name: &H5QualName) -> QualName {
    let ns_str = name.ns.to_string();
    let ns = if ns_str.is_empty() || ns_str == "http://www.w3.org/1999/xhtml" {
        None
    } else {
        Some(ns_str)
    };
    QualName {
        ns,
        local: name.local.to_string(),
    }
}

fn convert_attrs(attrs: Vec<H5Attribute>) -> Vec<Attribute> {
    attrs
        .into_iter()
        .map(|a| {
            let ns_str = a.name.ns.to_string();
            Attribute {
                name: QualName {
                    ns: if ns_str.is_empty() {
                        None
                    } else {
                        Some(ns_str)
                    },
                    local: a.name.local.to_string(),
                },
                value: a.value.to_string(),
            }
        })
        .collect()
}

#[allow(unsafe_code)]
impl TreeSink for DomTreeSink {
    type Handle = NodeId;
    type Output = Dom;
    type ElemName<'a> = ExpandedName<'a>;

    fn finish(self) -> Self::Output {
        self.dom.into_inner()
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> NodeId {
        NodeId::DOCUMENT
    }

    fn elem_name<'a>(&'a self, target: &'a NodeId) -> ExpandedName<'a> {
        if let Some(qn) = self.names().get(target) {
            ExpandedName {
                ns: &qn.ns,
                local: &qn.local,
            }
        } else {
            static NS: html5ever::Namespace = ns!(html);
            static LOCAL: html5ever::LocalName = local_name!("");
            ExpandedName {
                ns: &NS,
                local: &LOCAL,
            }
        }
    }

    fn create_element(
        &self,
        name: H5QualName,
        attrs: Vec<H5Attribute>,
        _flags: ElementFlags,
    ) -> NodeId {
        let id = self
            .dom_mut()
            .create_element(convert_qualname(&name), convert_attrs(attrs));
        self.names_mut().insert(id, name);
        id
    }

    fn create_comment(&self, text: html5ever::tendril::StrTendril) -> NodeId {
        self.dom_mut().create_comment(text.to_string())
    }

    fn create_pi(
        &self,
        target: html5ever::tendril::StrTendril,
        data: html5ever::tendril::StrTendril,
    ) -> NodeId {
        self.dom_mut()
            .allocate_pi(target.to_string(), data.to_string())
    }

    fn append(&self, parent: &NodeId, child: NodeOrText<NodeId>) {
        let dom = self.dom_mut();
        match child {
            NodeOrText::AppendNode(node_id) => {
                dom.append_child(*parent, node_id);
            }
            NodeOrText::AppendText(text) => {
                if let Some(last_child) = dom.get(*parent).and_then(|n| n.last_child) {
                    if let Some(node) = dom.get_mut(last_child) {
                        if let NodeData::Text(ref mut existing) = node.data {
                            existing.push_str(&text);
                            return;
                        }
                    }
                }
                let text_id = dom.create_text(text.to_string());
                dom.append_child(*parent, text_id);
            }
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &NodeId,
        prev_element: &NodeId,
        child: NodeOrText<NodeId>,
    ) {
        let has_parent = self.dom().get(*element).and_then(|n| n.parent).is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        name: html5ever::tendril::StrTendril,
        public_id: html5ever::tendril::StrTendril,
        system_id: html5ever::tendril::StrTendril,
    ) {
        let dom = self.dom_mut();
        let doctype = dom.create_doctype(
            name.to_string(),
            public_id.to_string(),
            system_id.to_string(),
        );
        dom.append_child(NodeId::DOCUMENT, doctype);
    }

    fn get_template_contents(&self, target: &NodeId) -> NodeId {
        *target
    }

    fn same_node(&self, x: &NodeId, y: &NodeId) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        // SAFETY: single-threaded parser, no concurrent access.
        unsafe {
            *self.quirks_mode.get() = mode;
        }
    }

    fn append_before_sibling(&self, sibling: &NodeId, child: NodeOrText<NodeId>) {
        let dom = self.dom_mut();
        let parent = match dom.get(*sibling).and_then(|n| n.parent) {
            Some(p) => p,
            None => return,
        };
        match child {
            NodeOrText::AppendNode(node_id) => {
                dom.insert_before(parent, node_id, *sibling);
            }
            NodeOrText::AppendText(text) => {
                let text_id = dom.create_text(text.to_string());
                dom.insert_before(parent, text_id, *sibling);
            }
        }
    }

    fn add_attrs_if_missing(&self, target: &NodeId, attrs: Vec<H5Attribute>) {
        let dom = self.dom_mut();
        if let Some(node) = dom.get_mut(*target) {
            if let Some(elem) = node.as_element_mut() {
                for attr in convert_attrs(attrs) {
                    if !elem.attrs.iter().any(|a| a.name == attr.name) {
                        elem.attrs.push(attr);
                    }
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &NodeId) {
        self.dom_mut().detach(*target);
    }

    fn reparent_children(&self, node: &NodeId, new_parent: &NodeId) {
        self.dom_mut().reparent_children(*node, *new_parent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{css_selectors::Element, dom::DomElement};

    #[test]
    fn parse_basic_html() {
        let dom = parse_html("<html><body><h1>Hello</h1></body></html>");
        let children = dom.children(NodeId::DOCUMENT);
        assert!(!children.is_empty(), "Document should have children");
    }

    #[test]
    fn parse_has_html_element() {
        let dom = parse_html("<html><head></head><body><p>Test</p></body></html>");
        let doc_children = dom.child_elements(NodeId::DOCUMENT);
        assert!(!doc_children.is_empty());

        let html_el = DomElement::new(&dom, doc_children[0]).unwrap();
        assert_eq!(html_el.local_name(), "html");
    }

    #[test]
    fn parse_text_content() {
        let dom = parse_html("<html><body><p>Hello world</p></body></html>");
        let html = dom.child_elements(NodeId::DOCUMENT)[0];
        let body = dom
            .child_elements(html)
            .into_iter()
            .find(|&id| {
                dom.get(id)
                    .and_then(|n| n.as_element())
                    .is_some_and(|e| e.name.local == "body")
            })
            .unwrap();
        let p = dom.child_elements(body)[0];
        assert_eq!(dom.text_content(p), "Hello world");
    }

    #[test]
    fn parse_attributes() {
        let dom = parse_html("<div id=\"main\" class=\"container\">test</div>");
        let html = dom.child_elements(NodeId::DOCUMENT)[0];
        let body = dom
            .child_elements(html)
            .into_iter()
            .find(|&id| {
                dom.get(id)
                    .and_then(|n| n.as_element())
                    .is_some_and(|e| e.name.local == "body")
            })
            .unwrap();
        let div = dom.child_elements(body)[0];
        let el = DomElement::new(&dom, div).unwrap();

        assert_eq!(el.id(), Some("main"));
        assert!(el.has_class("container"));
    }

    #[test]
    fn parse_nested_structure() {
        let dom = parse_html("<html><body><div><span>a</span><span>b</span></div></body></html>");
        let html = dom.child_elements(NodeId::DOCUMENT)[0];
        let body = dom
            .child_elements(html)
            .into_iter()
            .find(|&id| {
                dom.get(id)
                    .and_then(|n| n.as_element())
                    .is_some_and(|e| e.name.local == "body")
            })
            .unwrap();

        let div = dom
            .child_elements(body)
            .into_iter()
            .find(|&id| {
                dom.get(id)
                    .and_then(|n| n.as_element())
                    .is_some_and(|e| e.name.local == "div")
            })
            .unwrap();

        let spans = dom.child_elements(div);
        assert!(!spans.is_empty(), "expected at least 1 span");
        assert_eq!(dom.text_content(div), "ab");
    }

    // BDD: "Parse simple HTML" scenario via html5ever
    #[test]
    fn bdd_parse_simple_html_via_parser() {
        let dom = parse_html("<html><body><h1>Hello</h1></body></html>");
        assert!(dom.get(NodeId::DOCUMENT).is_some());

        let html = dom.child_elements(NodeId::DOCUMENT)[0];
        let body = dom
            .child_elements(html)
            .into_iter()
            .find(|&id| {
                dom.get(id)
                    .and_then(|n| n.as_element())
                    .is_some_and(|e| e.name.local == "body")
            })
            .unwrap();
        let h1 = dom
            .child_elements(body)
            .into_iter()
            .find(|&id| {
                dom.get(id)
                    .and_then(|n| n.as_element())
                    .is_some_and(|e| e.name.local == "h1")
            })
            .unwrap();

        assert_eq!(dom.text_content(h1), "Hello");
    }

    // BDD: "Query elements by selector" scenario via html5ever
    #[test]
    fn bdd_query_elements_via_parser() {
        let dom = parse_html("<div class='content'><p>First</p><p>Second</p></div>");
        let ps = dom.get_elements_by_tag_name(NodeId::DOCUMENT, "p");
        assert_eq!(ps.len(), 2);
        assert_eq!(dom.text_content(ps[0]), "First");
    }

    // BDD: "Mutate DOM tree" scenario via html5ever
    #[test]
    fn bdd_mutate_dom_via_parser() {
        let dom = parse_html("<div><span>Old</span></div>");
        let html = dom.child_elements(NodeId::DOCUMENT)[0];
        let body = dom
            .child_elements(html)
            .into_iter()
            .find(|&id| {
                dom.get(id)
                    .and_then(|n| n.as_element())
                    .is_some_and(|e| e.name.local == "body")
            })
            .unwrap();
        let div = dom
            .child_elements(body)
            .into_iter()
            .find(|&id| {
                dom.get(id)
                    .and_then(|n| n.as_element())
                    .is_some_and(|e| e.name.local == "div")
            })
            .unwrap();

        // Mutate: need a mutable Dom
        let mut dom = dom;
        let p = dom.create_element(QualName::new("p"), vec![]);
        let text = dom.create_text("New".to_string());
        dom.append_child(div, p);
        dom.append_child(p, text);

        assert_eq!(dom.children(div).len(), 2);
    }
}
