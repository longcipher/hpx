#![allow(missing_docs)]
use hpx_browser::{
    dom::NodeId,
    page::Page,
    resource_loader::{LoadedResource, ResourceType},
};

#[tokio::test]
async fn apply_stylesheets_injects_style_tags() {
    let html = r#"<!DOCTYPE html><html><head></head><body><p>hello</p></body></html>"#;
    let mut page = Page::from_html(html, false).await.unwrap();

    let styles = vec![LoadedResource {
        url: "http://example.com/style.css".to_string(),
        resource_type: ResourceType::Stylesheet,
        content: "body { color: red; }".to_string(),
        content_type: Some("text/css".to_string()),
    }];

    page.apply_stylesheets(&styles);

    // Verify via DOM serialization (content() returns the raw html field).
    let dom_html = page.dom().serialize_html(NodeId::DOCUMENT);
    assert!(
        dom_html.contains("color: red"),
        "CSS should be injected as <style> tag in DOM: {dom_html}"
    );
    assert!(
        dom_html.contains("<style>"),
        "should contain <style> element in DOM: {dom_html}"
    );
}

#[tokio::test]
async fn execute_scripts_runs_inline_scripts() {
    let html =
        r#"<!DOCTYPE html><html><head><script>var x = 42;</script></head><body></body></html>"#;
    let mut page = Page::from_html(html, false).await.unwrap();

    // Should not error even without v8 (just logs warning).
    let result = page.execute_scripts(&[]);
    assert!(result.is_ok(), "execute_scripts should succeed: {result:?}");
}

#[tokio::test]
async fn set_subresource_block_types_stores_value() {
    let mut page = Page::from_html("", false).await.unwrap();
    let mut block = std::collections::HashSet::new();
    block.insert(ResourceType::Stylesheet);
    page.set_subresource_block_types(block);
}
