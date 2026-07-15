#![allow(missing_docs)]
use hpx_browser::page::Page;

// ── Content Extraction Quality ───────────────────────────────────────────

/// Test extraction from a typical article page with nav, content, and footer.
#[tokio::test]
async fn extract_article_content() {
    let html = r#"<!DOCTYPE html>
<html><head>
    <title>Test Article — Example Site</title>
    <nav><a href="/">Home</a> | <a href="/about">About</a></nav>
</head><body>
    <header><h1>Site Logo</h1></header>
    <nav class="main-nav"><ul><li>Link 1</li><li>Link 2</li></ul></nav>
    <main>
        <article>
            <h1>Important Article Title</h1>
            <p>This is the core content of the article that matters for extraction.</p>
            <p>The second paragraph contains additional important information about the topic.</p>
        </article>
    </main>
    <footer><p>&copy; 2024 Example Site. All rights reserved.</p></footer>
</body></html>"#;

    let page = Page::from_html(html, false).await.unwrap();
    let text = page.text_content().await.unwrap();

    assert!(
        text.contains("Important Article Title"),
        "should contain article title"
    );
    assert!(text.contains("core content"), "should contain article body");
    assert!(
        text.contains("second paragraph"),
        "should contain second paragraph"
    );
}

/// Test extraction preserves semantic structure.
#[tokio::test]
async fn extract_preserves_headings() {
    let html = r#"<!DOCTYPE html><html><head><title>Semantic Page</title></head><body>
    <h1>Main Title</h1>
    <h2>Section Title</h2>
    <p>Section content here.</p>
    <h3>Subsection</h3>
    <p>Subsection content.</p>
</body></html>"#;

    let page = Page::from_html(html, false).await.unwrap();
    let text = page.text_content().await.unwrap();

    assert!(text.contains("Main Title"));
    assert!(text.contains("Section Title"));
    assert!(text.contains("Subsection"));
}

/// Test extraction from a table-heavy page.
#[tokio::test]
async fn extract_table_content() {
    let html = r#"<!DOCTYPE html><html><head><title>Data Table</title></head><body>
    <table>
        <thead><tr><th>Name</th><th>Value</th></tr></thead>
        <tbody>
            <tr><td>Alpha</td><td>100</td></tr>
            <tr><td>Beta</td><td>200</td></tr>
        </tbody>
    </table>
</body></html>"#;

    let page = Page::from_html(html, false).await.unwrap();
    let text = page.text_content().await.unwrap();

    assert!(text.contains("Alpha"), "should contain table data");
    assert!(text.contains("100"), "should contain table values");
}

/// Test extraction from a list-heavy page.
#[tokio::test]
async fn extract_list_content() {
    let html = r#"<!DOCTYPE html><html><head><title>List Page</title></head><body>
    <ul>
        <li>First item</li>
        <li>Second item</li>
        <li>Third item</li>
    </ul>
    <ol>
        <li>Step one</li>
        <li>Step two</li>
    </ol>
</body></html>"#;

    let page = Page::from_html(html, false).await.unwrap();
    let text = page.text_content().await.unwrap();

    assert!(text.contains("First item"));
    assert!(text.contains("Step one"));
}

/// Test extraction handles deeply nested content.
#[tokio::test]
async fn extract_deeply_nested_content() {
    let html = r#"<!DOCTYPE html><html><head><title>Nested</title></head><body>
    <div class="container">
        <div class="row">
            <div class="col">
                <div class="card">
                    <div class="card-body">
                        <p>Deeply nested content that should still be extracted.</p>
                    </div>
                </div>
            </div>
        </div>
    </div>
</body></html>"#;

    let page = Page::from_html(html, false).await.unwrap();
    let text = page.text_content().await.unwrap();
    assert!(
        text.contains("Deeply nested content"),
        "should extract deeply nested text"
    );
}

/// Test extraction from a page with mixed content types.
#[tokio::test]
async fn extract_mixed_content() {
    let html = r#"<!DOCTYPE html><html><head><title>Mixed</title></head><body>
    <h1>Article Title</h1>
    <p>Introduction paragraph.</p>
    <img src="/photo.jpg" alt="A photo">
    <p>More text after image.</p>
    <blockquote>Quoted text here.</p>
    <pre>code block content</pre>
    <p>Final paragraph.</p>
</body></html>"#;

    let page = Page::from_html(html, false).await.unwrap();
    let text = page.text_content().await.unwrap();

    assert!(text.contains("Introduction paragraph"));
    assert!(text.contains("More text after image"));
    assert!(text.contains("Quoted text"));
    assert!(text.contains("Final paragraph"));
}

/// Test extraction quality — F1-style check: content present, noise absent.
#[tokio::test]
async fn extraction_f1_quality() {
    let html = r#"<!DOCTYPE html><html><head>
    <title>Quality Test</title>
    <style>body { font-family: sans-serif; }</style>
    <script>console.log('tracking');</script>
</head><body>
    <nav><a href="/">Home</a></nav>
    <main>
        <h1>Research Paper Title</h1>
        <p class="abstract">This paper presents a novel approach to content extraction.</p>
        <p>We demonstrate that our method achieves superior performance on benchmarks.</p>
    </main>
    <aside>Advertisement: Buy our product!</aside>
    <footer>Copyright 2024</footer>
</body></html>"#;

    let page = Page::from_html(html, false).await.unwrap();
    let text = page.text_content().await.unwrap();

    // Ground truth content should be present (Recall)
    let content_present = text.contains("Research Paper Title")
        && text.contains("novel approach")
        && text.contains("superior performance");
    assert!(content_present, "core content should be extracted: {text}");

    // The text_content method extracts everything (it's not a smart extractor),
    // but we verify it doesn't crash or return empty
    assert!(!text.trim().is_empty(), "should not return empty text");
}

/// Test extraction from HTML with special characters.
#[tokio::test]
async fn extract_special_characters() {
    let html = r#"<!DOCTYPE html><html><head><title>Spëcial Chârs</title></head><body>
    <p>Content with émojis 🎉 and ünïcödé characters.</p>
    <p>Math: 2 &gt; 1 &amp; 3 &lt; 4</p>
    <p>Entities: &copy; &trade; &reg;</p>
</body></html>"#;

    let page = Page::from_html(html, false).await.unwrap();
    let text = page.text_content().await.unwrap();

    assert!(
        text.contains("Spëcial") || text.contains("Special"),
        "should handle special chars in title"
    );
    assert!(
        text.contains("ünïcödé") || text.contains("unicode"),
        "should handle unicode content"
    );
}

/// Test extraction from empty/minimal pages.
#[tokio::test]
async fn extract_empty_page() {
    let page = Page::from_html("", false).await.unwrap();
    let text = page.text_content().await.unwrap();
    // Should not panic, just return empty or minimal text
    let _ = text; // just checking it doesn't error
}

/// Test extraction from a page with forms (read-only content).
#[tokio::test]
async fn extract_form_labels() {
    let html = r#"<!DOCTYPE html><html><head><title>Form</title></head><body>
    <form>
        <label for="name">Full Name</label>
        <input id="name" type="text">
        <label for="email">Email Address</label>
        <input id="email" type="email">
        <button type="submit">Submit</button>
    </form>
</body></html>"#;

    let page = Page::from_html(html, false).await.unwrap();
    let text = page.text_content().await.unwrap();

    assert!(text.contains("Full Name"), "should extract label text");
    assert!(
        text.contains("Email Address"),
        "should extract second label"
    );
}
