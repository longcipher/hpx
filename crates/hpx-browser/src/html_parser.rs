use blitz_dom::{BaseDocument, DocumentConfig, StyleThreading};
use blitz_html::HtmlDocument;

use crate::dom::Dom;

pub fn parse_html(html: &str) -> Dom {
    let mut config = DocumentConfig::default();
    config.style_threading = StyleThreading::Sequential;
    config.base_url = Some("http://localhost".to_string());
    let html_doc = HtmlDocument::from_html(html, config);
    let base: BaseDocument = html_doc.into();
    Dom::from_base(base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::{DomElement, NodeId, QualName};

    fn find_child_by_tag(dom: &Dom, parent: NodeId, tag: &str) -> NodeId {
        dom.child_elements(parent)
            .into_iter()
            .find(|&id| {
                dom.get(id)
                    .map(|n| n.is_element_with_tag(tag))
                    .unwrap_or(false)
            })
            .unwrap()
    }

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
        let body = find_child_by_tag(&dom, html, "body");
        let p = dom.child_elements(body)[0];
        assert_eq!(dom.text_content(p), "Hello world");
    }

    #[test]
    fn parse_attributes() {
        let dom = parse_html("<div id=\"main\" class=\"container\">test</div>");
        let html = dom.child_elements(NodeId::DOCUMENT)[0];
        let body = find_child_by_tag(&dom, html, "body");
        let div = dom.child_elements(body)[0];
        let el = DomElement::new(&dom, div).unwrap();

        assert_eq!(el.id(), Some("main"));
        assert!(el.has_class("container"));
    }

    #[test]
    fn parse_nested_structure() {
        let dom = parse_html("<html><body><div><span>a</span><span>b</span></div></body></html>");
        let html = dom.child_elements(NodeId::DOCUMENT)[0];
        let body = find_child_by_tag(&dom, html, "body");
        let div = find_child_by_tag(&dom, body, "div");

        let spans = dom.child_elements(div);
        assert!(!spans.is_empty(), "expected at least 1 span");
        assert_eq!(dom.text_content(div), "ab");
    }

    #[test]
    fn bdd_parse_simple_html_via_parser() {
        let dom = parse_html("<html><body><h1>Hello</h1></body></html>");
        assert!(dom.get(NodeId::DOCUMENT).is_some());

        let html = dom.child_elements(NodeId::DOCUMENT)[0];
        let body = find_child_by_tag(&dom, html, "body");
        let h1 = find_child_by_tag(&dom, body, "h1");

        assert_eq!(dom.text_content(h1), "Hello");
    }

    #[test]
    fn bdd_query_elements_via_parser() {
        let dom = parse_html("<div class='content'><p>First</p><p>Second</p></div>");
        let ps = dom.get_elements_by_tag_name(NodeId::DOCUMENT, "p");
        assert_eq!(ps.len(), 2);
        assert_eq!(dom.text_content(ps[0]), "First");
    }

    #[test]
    fn bdd_mutate_dom_via_parser() {
        let dom = parse_html("<div><span>Old</span></div>");
        let html = dom.child_elements(NodeId::DOCUMENT)[0];
        let body = find_child_by_tag(&dom, html, "body");
        let div = find_child_by_tag(&dom, body, "div");

        let mut dom = dom;
        let p = dom.create_element(QualName::new("p"), vec![]);
        let text = dom.create_text("New".to_string());
        dom.append_child(div, p);
        dom.append_child(p, text);

        assert_eq!(dom.children(div).len(), 2);
    }
}
