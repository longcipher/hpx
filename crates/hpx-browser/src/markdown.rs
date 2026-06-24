use crate::dom::{Dom, NodeData, NodeId};

/// Convert HTML to Markdown.
pub fn html_to_markdown(html: &str) -> String {
    let dom = crate::html_parser::parse_html(html);
    let md = dom_to_markdown(&dom, NodeId::DOCUMENT, 0);
    md.trim().to_string()
}

fn dom_to_markdown(dom: &Dom, node_id: NodeId, depth: usize) -> String {
    let node = match dom.get(node_id) {
        Some(n) => n,
        None => return String::new(),
    };

    match &node.data {
        NodeData::Text(text) => return text.clone(),
        NodeData::Comment(_)
        | NodeData::DocumentType { .. }
        | NodeData::ProcessingInstruction { .. } => {
            return String::new();
        }
        NodeData::Document | NodeData::DocumentFragment | NodeData::ShadowRoot { .. } => {
            return children_md(dom, node_id, depth);
        }
        NodeData::Element(el) => {
            let tag = el.name.local.as_str();
            let children = children_md(dom, node_id, depth);

            return match tag {
                "h1" => format!("# {}\n\n", children.trim()),
                "h2" => format!("## {}\n\n", children.trim()),
                "h3" => format!("### {}\n\n", children.trim()),
                "h4" => format!("#### {}\n\n", children.trim()),
                "h5" => format!("##### {}\n\n", children.trim()),
                "h6" => format!("###### {}\n\n", children.trim()),
                "p" => format!("{}\n\n", children.trim()),
                "br" => "\n".to_string(),
                "hr" => "---\n\n".to_string(),
                "strong" | "b" => format!("**{}**", children.trim()),
                "em" | "i" => format!("*{}*", children.trim()),
                "code" => format!("`{}`", children.trim()),
                "a" => {
                    let href = el
                        .attrs
                        .iter()
                        .find(|a| a.name.local == "href")
                        .map(|a| a.value.as_str())
                        .unwrap_or("");
                    format!("[{}]({})", children.trim(), href)
                }
                "img" => {
                    let src = el
                        .attrs
                        .iter()
                        .find(|a| a.name.local == "src")
                        .map(|a| a.value.as_str())
                        .unwrap_or("");
                    let alt = el
                        .attrs
                        .iter()
                        .find(|a| a.name.local == "alt")
                        .map(|a| a.value.as_str())
                        .unwrap_or("");
                    format!("![{}]({})", alt, src)
                }
                "ul" => list_md(dom, node_id, false, depth),
                "ol" => list_md(dom, node_id, true, depth),
                "li" => children, // parent handles prefix
                "blockquote" => {
                    let indented = children
                        .trim()
                        .lines()
                        .map(|l| format!("> {}", l))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!("{}\n\n", indented)
                }
                "pre" => {
                    let code_text = extract_text(dom, node_id);
                    format!("```\n{}\n```\n\n", code_text.trim())
                }
                "table" => table_md(dom, node_id),
                "thead" | "tbody" | "tfoot" | "tr" | "th" | "td" => children, /* handled by table_md */
                _ => children,
            };
        }
    }
}

fn children_md(dom: &Dom, parent: NodeId, depth: usize) -> String {
    let mut out = String::new();
    let mut current = dom.get(parent).and_then(|n| n.first_child);
    while let Some(id) = current {
        out.push_str(&dom_to_markdown(dom, id, depth));
        current = dom.get(id).and_then(|n| n.next_sibling);
    }
    out
}

fn extract_text(dom: &Dom, node_id: NodeId) -> String {
    let node = match dom.get(node_id) {
        Some(n) => n,
        None => return String::new(),
    };
    match &node.data {
        NodeData::Text(t) => return t.clone(),
        _ => {}
    }
    let mut out = String::new();
    let mut current = node.first_child;
    while let Some(id) = current {
        out.push_str(&extract_text(dom, id));
        current = dom.get(id).and_then(|n| n.next_sibling);
    }
    out
}

fn list_md(dom: &Dom, list_id: NodeId, ordered: bool, depth: usize) -> String {
    let mut out = String::new();
    let mut idx: usize = 1;
    let mut current = dom.get(list_id).and_then(|n| n.first_child);
    while let Some(id) = current {
        if let Some(node) = dom.get(id) {
            if let NodeData::Element(el) = &node.data {
                if el.name.local == "li" {
                    let content = children_md(dom, id, depth + 1);
                    let prefix = if ordered {
                        format!("{}. ", idx)
                    } else {
                        "- ".to_string()
                    };
                    out.push_str(&format!("{}{}\n", prefix, content.trim()));
                    if ordered {
                        idx += 1;
                    }
                }
            }
        }
        current = dom.get(id).and_then(|n| n.next_sibling);
    }
    format!("{}\n", out)
}

fn table_md(dom: &Dom, table_id: NodeId) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    collect_table_rows(dom, table_id, &mut rows);
    if rows.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    // header row
    if let Some(header) = rows.first() {
        out.push_str("| ");
        out.push_str(&header.join(" | "));
        out.push_str(" |\n");
        // separator
        out.push_str("| ");
        out.push_str(&header.iter().map(|_| "---").collect::<Vec<_>>().join(" | "));
        out.push_str(" |\n");
    }
    // body rows
    for row in rows.iter().skip(1) {
        out.push_str("| ");
        out.push_str(&row.join(" | "));
        out.push_str(" |\n");
    }
    format!("{}\n", out)
}

fn collect_table_rows(dom: &Dom, node_id: NodeId, rows: &mut Vec<Vec<String>>) {
    let node = match dom.get(node_id) {
        Some(n) => n,
        None => return,
    };
    if let NodeData::Element(el) = &node.data {
        if el.name.local == "tr" {
            let mut cells = Vec::new();
            let mut child = node.first_child;
            while let Some(id) = child {
                if let Some(cnode) = dom.get(id) {
                    if let NodeData::Element(cel) = &cnode.data {
                        if cel.name.local == "td" || cel.name.local == "th" {
                            cells.push(extract_text(dom, id).trim().to_string());
                        }
                    }
                }
                child = cnode_next(dom, id);
            }
            rows.push(cells);
            return;
        }
    }
    // recurse into children
    let mut child = node.first_child;
    while let Some(id) = child {
        collect_table_rows(dom, id, rows);
        child = cnode_next(dom, id);
    }
}

fn cnode_next(dom: &Dom, id: NodeId) -> Option<NodeId> {
    dom.get(id).and_then(|n| n.next_sibling)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_headings() {
        let html = "<html><body><h1>Hello</h1><h2>World</h2><h3>Sub</h3></body></html>";
        let md = html_to_markdown(html);
        assert!(md.contains("# Hello"), "got: {}", md);
        assert!(md.contains("## World"), "got: {}", md);
        assert!(md.contains("### Sub"), "got: {}", md);
    }

    #[test]
    fn test_paragraph() {
        let html = "<html><body><p>First paragraph.</p><p>Second paragraph.</p></body></html>";
        let md = html_to_markdown(html);
        assert!(md.contains("First paragraph."), "got: {}", md);
        assert!(md.contains("Second paragraph."), "got: {}", md);
    }

    #[test]
    fn test_links() {
        let html = r#"<html><body><a href="https://example.com">Click here</a></body></html>"#;
        let md = html_to_markdown(html);
        assert!(
            md.contains("[Click here](https://example.com)"),
            "got: {}",
            md
        );
    }

    #[test]
    fn test_images() {
        let html = r#"<html><body><img src="pic.png" alt="A picture"></body></html>"#;
        let md = html_to_markdown(html);
        assert!(md.contains("![A picture](pic.png)"), "got: {}", md);
    }

    #[test]
    fn test_bold_italic() {
        let html = "<html><body><strong>Bold</strong> and <em>italic</em></body></html>";
        let md = html_to_markdown(html);
        assert!(md.contains("**Bold**"), "got: {}", md);
        assert!(md.contains("*italic*"), "got: {}", md);
    }

    #[test]
    fn test_inline_code() {
        let html = "<html><body><code>foo()</code></body></html>";
        let md = html_to_markdown(html);
        assert!(md.contains("`foo()`"), "got: {}", md);
    }

    #[test]
    fn test_code_block() {
        let html = "<html><body><pre><code>let x = 1;</code></pre></body></html>";
        let md = html_to_markdown(html);
        assert!(md.contains("```\nlet x = 1;\n```"), "got: {}", md);
    }

    #[test]
    fn test_unordered_list() {
        let html = "<html><body><ul><li>One</li><li>Two</li></ul></body></html>";
        let md = html_to_markdown(html);
        assert!(md.contains("- One"), "got: {}", md);
        assert!(md.contains("- Two"), "got: {}", md);
    }

    #[test]
    fn test_ordered_list() {
        let html = "<html><body><ol><li>First</li><li>Second</li></ol></body></html>";
        let md = html_to_markdown(html);
        assert!(md.contains("1. First"), "got: {}", md);
        assert!(md.contains("2. Second"), "got: {}", md);
    }

    #[test]
    fn test_blockquote() {
        let html = "<html><body><blockquote><p>Wise words.</p></blockquote></body></html>";
        let md = html_to_markdown(html);
        assert!(md.contains("> Wise words."), "got: {}", md);
    }

    #[test]
    fn test_hr_and_br() {
        let html = "<html><body>Line<br>Two<hr>After</body></html>";
        let md = html_to_markdown(html);
        assert!(md.contains("---"), "got: {}", md);
    }

    #[test]
    fn test_table() {
        let html = r#"<html><body><table>
            <thead><tr><th>Name</th><th>Age</th></tr></thead>
            <tbody><tr><td>Alice</td><td>30</td></tr><tr><td>Bob</td><td>25</td></tr></tbody>
        </table></body></html>"#;
        let md = html_to_markdown(html);
        assert!(md.contains("| Name | Age |"), "got: {}", md);
        assert!(md.contains("| --- | --- |"), "got: {}", md);
        assert!(md.contains("| Alice | 30 |"), "got: {}", md);
        assert!(md.contains("| Bob | 25 |"), "got: {}", md);
    }
}
