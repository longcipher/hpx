#![allow(missing_docs)]
use hpx_browser::{
    dom::NodeId,
    html_parser::parse_html,
    page::Page,
    resource_loader::{ResourceType, extract_resource_urls, filter_by_block_types},
};

// ── Inline Script Execution ──────────────────────────────────────────────

#[cfg(feature = "v8")]
#[tokio::test]
async fn inline_scripts_set_globals() {
    let html = r#"<!DOCTYPE html><html><head>
        <script>window.x = 42;</script>
    </head><body></body></html>"#;
    let mut page = Page::from_html(html, false).await.unwrap();
    page.execute_inline_scripts().unwrap();
    let result = page.evaluate("window.x").unwrap();
    assert_eq!(result, "42", "inline script should set global");
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn inline_scripts_execute_in_order() {
    let html = r#"<!DOCTYPE html><html><head>
        <script>window.order = [];</script>
        <script>window.order.push(1);</script>
        <script>window.order.push(2);</script>
    </head><body></body></html>"#;
    let mut page = Page::from_html(html, false).await.unwrap();
    page.execute_inline_scripts().unwrap();
    let result = page.evaluate("window.order.join(',')").unwrap();
    assert_eq!(result, "1,2", "scripts should execute in document order");
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn inline_script_error_does_not_stop_others() {
    let html = r#"<!DOCTYPE html><html><head>
        <script>window.before = true;</script>
        <script>throw new Error('intentional');</script>
        <script>window.after = true;</script>
    </head><body></body></html>"#;
    let mut page = Page::from_html(html, false).await.unwrap();
    page.execute_inline_scripts().unwrap();
    assert_eq!(page.evaluate("window.before").unwrap(), "true");
    assert_eq!(page.evaluate("window.after").unwrap(), "true");
}

// ── External Script Execution ────────────────────────────────────────────

#[cfg(feature = "v8")]
#[tokio::test]
async fn external_scripts_loaded() {
    use hpx_browser::resource_loader::LoadedResource;

    let html = r#"<!DOCTYPE html><html><head></head><body></body></html>"#;
    let mut page = Page::from_html(html, false).await.unwrap();

    let scripts = vec![LoadedResource {
        url: "http://example.com/app.js".to_string(),
        resource_type: ResourceType::Script,
        content: "window.loaded = 'yes';".to_string(),
        content_type: Some("application/javascript".to_string()),
    }];

    page.execute_scripts(&scripts).unwrap();
    let result = page.evaluate("window.loaded").unwrap();
    assert_eq!(result, "yes", "external script should be executed");
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn inline_then_external_order() {
    use hpx_browser::resource_loader::LoadedResource;

    let html = r#"<!DOCTYPE html><html><head>
        <script>window.seq = ['inline'];</script>
    </head><body></body></html>"#;
    let mut page = Page::from_html(html, false).await.unwrap();

    let scripts = vec![LoadedResource {
        url: "http://example.com/ext.js".to_string(),
        resource_type: ResourceType::Script,
        content: "window.seq.push('external');".to_string(),
        content_type: Some("application/javascript".to_string()),
    }];

    page.execute_scripts(&scripts).unwrap();
    let result = page.evaluate("window.seq.join(',')").unwrap();
    assert_eq!(result, "inline,external", "inline runs before external");
}

// ── CSS Injection ────────────────────────────────────────────────────────

#[tokio::test]
async fn stylesheets_injected_into_head() {
    use hpx_browser::resource_loader::LoadedResource;

    let html = r#"<!DOCTYPE html><html><head></head><body></body></html>"#;
    let mut page = Page::from_html(html, false).await.unwrap();

    let styles = vec![LoadedResource {
        url: "http://example.com/style.css".to_string(),
        resource_type: ResourceType::Stylesheet,
        content: "body { margin: 0; }".to_string(),
        content_type: Some("text/css".to_string()),
    }];

    page.apply_stylesheets(&styles);

    let dom_html = page.dom().serialize_html(NodeId::DOCUMENT);
    assert!(dom_html.contains("<style>"), "should have <style> tag");
    assert!(dom_html.contains("margin"), "should contain CSS content");
}

#[tokio::test]
async fn multiple_stylesheets_all_injected() {
    use hpx_browser::resource_loader::LoadedResource;

    let html = r#"<!DOCTYPE html><html><head></head><body></body></html>"#;
    let mut page = Page::from_html(html, false).await.unwrap();

    let styles = vec![
        LoadedResource {
            url: "http://example.com/a.css".to_string(),
            resource_type: ResourceType::Stylesheet,
            content: "body { color: red; }".to_string(),
            content_type: Some("text/css".to_string()),
        },
        LoadedResource {
            url: "http://example.com/b.css".to_string(),
            resource_type: ResourceType::Stylesheet,
            content: "p { font-size: 16px; }".to_string(),
            content_type: Some("text/css".to_string()),
        },
    ];

    page.apply_stylesheets(&styles);

    let dom_html = page.dom().serialize_html(NodeId::DOCUMENT);
    assert!(
        dom_html.contains("color: red"),
        "first CSS should be injected"
    );
    assert!(
        dom_html.contains("font-size"),
        "second CSS should be injected"
    );
}

// ── Resource URL Extraction ──────────────────────────────────────────────

#[test]
fn extract_stylesheet_urls() {
    let dom = parse_html(
        r#"<html><head>
        <link rel="stylesheet" href="/style.css">
        </head><body></body></html>"#,
    );
    let urls = extract_resource_urls(&dom);
    assert_eq!(urls.len(), 1);
    assert_eq!(urls[0].resource_type, ResourceType::Stylesheet);
    assert_eq!(urls[0].url, "/style.css");
}

#[test]
fn extract_script_src_urls() {
    let dom = parse_html(
        r#"<html><head>
        <script src="/app.js"></script>
        </head><body></body></html>"#,
    );
    let urls = extract_resource_urls(&dom);
    assert_eq!(urls.len(), 1);
    assert_eq!(urls[0].resource_type, ResourceType::Script);
    assert_eq!(urls[0].url, "/app.js");
}

#[test]
fn extract_img_urls() {
    let dom = parse_html(r#"<html><body><img src="/photo.jpg"></body></html>"#);
    let urls = extract_resource_urls(&dom);
    assert_eq!(urls.len(), 1);
    assert_eq!(urls[0].resource_type, ResourceType::Image);
    assert_eq!(urls[0].url, "/photo.jpg");
}

#[test]
fn extract_all_resource_types() {
    let dom = parse_html(
        r#"<html><head>
        <link rel="stylesheet" href="/style.css">
        <script src="/app.js"></script>
        </head><body>
        <img src="/logo.png">
        <video src="/clip.mp4"></video>
        <audio src="/sound.mp3"></audio>
        </body></html>"#,
    );
    let urls = extract_resource_urls(&dom);
    let types: std::collections::HashSet<_> = urls.iter().map(|u| u.resource_type).collect();
    assert!(types.contains(&ResourceType::Stylesheet));
    assert!(types.contains(&ResourceType::Script));
    assert!(types.contains(&ResourceType::Image));
    assert!(types.contains(&ResourceType::Media));
}

#[test]
fn inline_scripts_not_extracted() {
    let dom = parse_html(
        r#"<html><head>
        <script>window.x = 1;</script>
        <script src="/ext.js"></script>
        </head><body></body></html>"#,
    );
    let urls = extract_resource_urls(&dom);
    assert_eq!(
        urls.len(),
        1,
        "inline scripts should not be in resource URLs"
    );
    assert_eq!(urls[0].url, "/ext.js");
}

#[test]
fn duplicate_urls_deduplicated() {
    let dom = parse_html(
        r#"<html><head>
        <link rel="stylesheet" href="/style.css">
        <link rel="stylesheet" href="/style.css">
        </head><body></body></html>"#,
    );
    let urls = extract_resource_urls(&dom);
    assert_eq!(urls.len(), 1, "duplicate URLs should be deduplicated");
}

// ── Resource Blocking ────────────────────────────────────────────────────

#[test]
fn block_images_filters_image_resources() {
    let dom = parse_html(
        r#"<html><body>
        <img src="/photo.jpg">
        <img src="/icon.png">
        </body></html>"#,
    );
    let urls = extract_resource_urls(&dom);
    let mut block = std::collections::HashSet::new();
    block.insert(ResourceType::Image);
    let filtered = filter_by_block_types(urls, &block);
    assert!(filtered.is_empty(), "all images should be blocked");
}

#[test]
fn block_images_preserves_scripts() {
    let dom = parse_html(
        r#"<html><head>
        <script src="/app.js"></script>
        </head><body>
        <img src="/photo.jpg">
        </body></html>"#,
    );
    let urls = extract_resource_urls(&dom);
    let mut block = std::collections::HashSet::new();
    block.insert(ResourceType::Image);
    let filtered = filter_by_block_types(urls, &block);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].resource_type, ResourceType::Script);
}

#[test]
fn block_multiple_types() {
    let dom = parse_html(
        r#"<html><head>
        <link rel="stylesheet" href="/style.css">
        <script src="/app.js"></script>
        </head><body>
        <img src="/photo.jpg">
        </body></html>"#,
    );
    let urls = extract_resource_urls(&dom);
    let mut block = std::collections::HashSet::new();
    block.insert(ResourceType::Image);
    block.insert(ResourceType::Stylesheet);
    let filtered = filter_by_block_types(urls, &block);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].resource_type, ResourceType::Script);
}

#[test]
fn empty_block_list_keeps_all() {
    let dom = parse_html(
        r#"<html><head>
        <link rel="stylesheet" href="/style.css">
        </head><body>
        <img src="/photo.jpg">
        </body></html>"#,
    );
    let urls = extract_resource_urls(&dom);
    let block = std::collections::HashSet::new();
    let filtered = filter_by_block_types(urls.clone(), &block);
    assert_eq!(filtered.len(), urls.len(), "empty block should keep all");
}

// ── DOM Operations ───────────────────────────────────────────────────────

#[tokio::test]
async fn page_title_extraction() {
    let html = r#"<!DOCTYPE html><html><head><title>My Page</title></head><body></body></html>"#;
    let page = Page::from_html(html, false).await.unwrap();
    assert_eq!(page.title(), "My Page");
}

#[tokio::test]
async fn page_url_set_on_reload() {
    let mut page = Page::from_html("", false).await.unwrap();
    page.reload_html("<html></html>", "https://example.com/test");
    assert_eq!(page.url(), "https://example.com/test");
}

#[tokio::test]
async fn collect_inline_scripts_finds_scripts() {
    let html = r#"<!DOCTYPE html><html><head>
        <script>var a = 1;</script>
        <script src="/ext.js"></script>
        <script>var b = 2;</script>
    </head><body></body></html>"#;
    let page = Page::from_html(html, false).await.unwrap();
    let scripts = page.collect_inline_scripts();
    assert_eq!(scripts.len(), 2, "should find 2 inline scripts");
    assert_eq!(scripts[0], "var a = 1;");
    assert_eq!(scripts[1], "var b = 2;");
}

#[tokio::test]
async fn reload_html_updates_dom() {
    let mut page = Page::from_html("<html><body>old</body></html>", false)
        .await
        .unwrap();
    assert!(page.content().contains("old"));

    page.reload_html("<html><body>new</body></html>", "http://example.com");
    assert!(page.content().contains("new"));
    assert!(!page.content().contains("old"));
}

// ── Challenge Detection ──────────────────────────────────────────────────

#[tokio::test]
async fn clean_page_verdict_pass() {
    let mut html = String::from("<html><body>");
    for _ in 0..400 {
        html.push_str("<p>Substantial content for testing challenge detection.</p>");
    }
    html.push_str("</body></html>");
    let page = Page::from_html(&html, false).await.unwrap();
    assert_eq!(
        page.challenge_verdict(),
        hpx_browser::challenge::ChallengeVerdict::Pass
    );
}

#[tokio::test]
async fn cloudflare_challenge_detected() {
    let html = r#"<!DOCTYPE html><html><head><title>Just a moment...</title></head>
    <body><script>window._cf_chl_opt={cvId:'3',cType:'managed'};</script>
    Checking your browser...</body></html>"#;
    let page = Page::from_html(html, false).await.unwrap();
    assert!(page.challenge_verdict().is_challenge());
}
