//! Link and asset extraction from HTML documents.
//!
//! Pure DOM-level helpers shared by frontends (CLI `scrape`, tests, future
//! service APIs). They parse the given HTML with [`crate::html_parser`] and
//! return `(attribute_value, label)` pairs:
//!
//! - [`extract_links`]: every `<a href>` as `(href, anchor_text)`.
//! - [`extract_assets`]: `<script src>`, `<link href>` and `<img src>` as
//!   `(url, tag_name)`.

use crate::dom::Dom;
// `parse_html` is the canonical entry point; re-exported here for readability.
use crate::html_parser;

/// Extract all hyperlink references from the HTML document.
///
/// Returns `(href, text)` pairs in document order. Anchors without an `href`
/// attribute are skipped; text content has surrounding whitespace preserved
/// as-is from the DOM.
#[must_use]
pub fn extract_links(html: &str) -> Vec<(String, String)> {
    let dom = crate::html_parser::parse_html(html);
    let anchors = dom.get_elements_by_tag_name(dom.document(), "a");
    let mut results = Vec::with_capacity(anchors.len());
    for &id in &anchors {
        let Some(node) = dom.get(id) else {
            continue;
        };
        let Some(elem) = node.as_element() else {
            continue;
        };
        let Some(href) = elem
            .attrs
            .iter()
            .find(|a| a.name.local == "href")
            .map(|a| a.value.as_str())
        else {
            continue;
        };
        let text = dom.text_content(id);
        results.push((href.to_string(), text));
    }
    results
}

/// Extract sub-resource asset URLs from the HTML document.
///
/// Returns `(url, kind)` pairs where `kind` is `"script"`, `"link"` or
/// `"img"`, in that tag order, document order within each tag.
#[must_use]
pub fn extract_assets(html: &str) -> Vec<(String, String)> {
    let dom = html_parser::parse_html(html);
    extract_assets_from_dom(&dom)
}

fn extract_assets_from_dom(dom: &Dom) -> Vec<(String, String)> {
    let mut results = Vec::new();

    for tag in ["script", "link", "img"] {
        for &id in &dom.get_elements_by_tag_name(dom.document(), tag) {
            let Some(node) = dom.get(id) else {
                continue;
            };
            let Some(elem) = node.as_element() else {
                continue;
            };
            let attr_name = if tag == "link" { "href" } else { "src" };
            if let Some(value) = elem
                .attrs
                .iter()
                .find(|a| a.name.local == attr_name)
                .map(|a| a.value.clone())
            {
                results.push((value, tag.to_string()));
            }
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_extract_href_and_text() {
        let html = r#"<html><body>
            <a href="https://a.example">Alpha</a>
            <a name="no-href">skipped</a>
            <a href="/rel">Beta</a>
        </body></html>"#;

        let links = extract_links(html);
        assert_eq!(
            links,
            vec![
                ("https://a.example".to_string(), "Alpha".to_string()),
                ("/rel".to_string(), "Beta".to_string()),
            ]
        );
    }

    #[test]
    fn assets_cover_script_link_img() {
        let html = r#"<html><head>
            <link rel="stylesheet" href="style.css">
            <script src="app.js"></script>
        </head><body><img src="logo.png"></body></html>"#;

        let assets = extract_assets(html);
        assert_eq!(
            assets,
            vec![
                ("app.js".to_string(), "script".to_string()),
                ("style.css".to_string(), "link".to_string()),
                ("logo.png".to_string(), "img".to_string()),
            ]
        );
    }

    #[test]
    fn empty_document_yields_nothing() {
        assert!(extract_links("<p>hi</p>").is_empty());
        assert!(extract_assets("<p>hi</p>").is_empty());
    }
}
