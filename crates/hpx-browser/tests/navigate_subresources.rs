//! hpx-browser test/example crate.

#![allow(missing_docs)]
#![allow(
    clippy::allow_attributes,
    clippy::assert_is_empty,
    clippy::borrow_as_ptr,
    clippy::cast_lossless,
    clippy::collection_is_never_read,
    clippy::collapsible_if,
    clippy::duration_suboptimal_units,
    clippy::explicit_iter_loop,
    clippy::expect_fun_call,
    clippy::expect_used,
    clippy::ignore_without_reason,
    clippy::ignored_unit_patterns,
    clippy::items_after_test_module,
    clippy::iter_kv_map,
    clippy::iter_with_drain,
    clippy::let_underscore_future,
    clippy::manual_div_ceil,
    clippy::manual_string_new,
    clippy::map_unwrap_or,
    clippy::needless_borrows_for_generic_args,
    clippy::needless_continue,
    clippy::needless_pass_by_ref_mut,
    clippy::needless_raw_string_hashes,
    clippy::nonminimal_bool,
    clippy::panic,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::redundant_async_block,
    clippy::redundant_clone,
    clippy::redundant_pattern_matching,
    clippy::struct_field_names,
    clippy::type_complexity,
    clippy::unchecked_time_subtraction,
    clippy::unreadable_literal,
    clippy::unnecessary_cast,
    clippy::unnecessary_trailing_comma,
    clippy::unnested_or_patterns,
    clippy::useless_borrows_in_formatting,
    clippy::useless_vec,
    clippy::use_self,
    clippy::unwrap_used,
    unfulfilled_lint_expectations
)]
use hpx_browser::{
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
    let dom_html = page.dom().serialize_html(page.dom().document());
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
