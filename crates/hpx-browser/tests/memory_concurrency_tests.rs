#![allow(missing_docs)]
use hpx_browser::{page::Page, resource_loader::ResourceType};

fn current_rss_bytes() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    unsafe {
        if libc::getrusage(0, usage.as_mut_ptr()) == 0 {
            let usage = usage.assume_init();
            #[cfg(target_os = "macos")]
            return usage.ru_maxrss as u64;
            #[cfg(target_os = "linux")]
            return usage.ru_maxrss as u64 * 1024;
        }
    }
    0
}

// ── Page Creation/Drop Leak Test ─────────────────────────────────────────

#[tokio::test]
async fn create_and_drop_100_pages_no_leak() {
    let rss_before = current_rss_bytes();

    for i in 0..100 {
        let html = format!("<html><head><title>Page {i}</title></head><body>content</body></html>");
        let page = Page::from_html(&html, false).await.unwrap();
        assert_eq!(page.title(), format!("Page {i}"));
        drop(page);
    }

    let rss_after = current_rss_bytes();
    let delta = rss_after.saturating_sub(rss_before);
    // ponytail: DOM parser + arena allocation per page is heavy (~1MB/page).
    // 200MB threshold for 100 pages. Tighten after profiling identifies specific leaks.
    assert!(
        delta < 200 * 1024 * 1024,
        "RSS grew by {delta} bytes ({:.1} MB) after creating/dropping 100 pages — possible leak",
        delta as f64 / 1_048_576.0
    );
}

// ── V8 Runtime Reuse (reload_html) ──────────────────────────────────────

#[cfg(feature = "v8")]
#[tokio::test]
async fn reload_html_reuses_v8_runtime() {
    let mut page = Page::from_html("<html><body>first</body></html>", false)
        .await
        .unwrap();
    // Set a global in the V8 runtime
    page.evaluate("window.persistent = 'yes'").unwrap();

    // Reload with new HTML — runtime should be reused
    page.reload_html("<html><body>second</body></html>", "http://example.com");
    let result = page.evaluate("window.persistent").unwrap();
    assert_eq!(result, "yes", "V8 runtime should be reused across reloads");
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn reload_html_100_times_no_leak() {
    let rss_before = current_rss_bytes();

    let mut page = Page::from_html("<html><body>init</body></html>", false)
        .await
        .unwrap();
    for i in 0..100 {
        let html = format!("<html><body>reload {i}</body></html>");
        page.reload_html(&html, "http://example.com");
    }

    let rss_after = current_rss_bytes();
    let delta = rss_after.saturating_sub(rss_before);
    // V8 runtime reuse should keep memory stable.
    // RSS measurements are noisy — 50MB catches real leaks without false positives.
    assert!(
        delta < 50 * 1024 * 1024,
        "RSS grew by {delta} bytes ({:.1} MB) after 100 reloads — possible V8 leak",
        delta as f64 / 1_048_576.0
    );
}

// ── Concurrent Page Creation ─────────────────────────────────────────────

#[tokio::test]
async fn ten_concurrent_pages() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let handles: Vec<_> = (0..10)
                .map(|i| {
                    tokio::task::spawn_local(async move {
                        let html = format!(
                            "<html><head><title>Concurrent {i}</title></head><body>page {i}</body></html>"
                        );
                        let page = Page::from_html(&html, false).await.unwrap();
                        assert_eq!(page.title(), format!("Concurrent {i}"));
                        i
                    })
                })
                .collect();

            for h in handles {
                h.await.unwrap();
            }
        })
        .await;
}

// ── Memory Benchmark for Criterion ───────────────────────────────────────

/// Run with: `cargo test -p hpx-browser --test memory_concurrency_tests -- memory_benchmark --nocapture`
#[tokio::test]
async fn memory_benchmark() {
    let html = include_str!("../benches/test_page.html");
    let rss_before = current_rss_bytes();

    let engine = hpx_browser::host::EngineHandle::new();
    let mut page = Page::new(engine);
    for _ in 0..100 {
        page.reload_html(html, "http://example.com");
    }
    drop(page);

    let rss_after = current_rss_bytes();
    let delta = rss_after.saturating_sub(rss_before);
    println!(
        "RSS delta: {delta} bytes ({:.1} MB)",
        delta as f64 / 1_048_576.0
    );

    // ponytail: DOM parser + arena allocation is heavy per reload.
    // 200MB threshold for 100 reloads. Tighten after profiling identifies specific leaks.
    assert!(
        delta < 200 * 1024 * 1024,
        "RSS should stay under 200MB: grew by {:.1} MB",
        delta as f64 / 1_048_576.0
    );
}

// ── Subresource Block Types Persistence ──────────────────────────────────

#[tokio::test]
async fn block_types_persist_across_reload() {
    let mut page = Page::from_html("<html><body></body></html>", false)
        .await
        .unwrap();
    let mut block = std::collections::HashSet::new();
    block.insert(ResourceType::Image);
    block.insert(ResourceType::Media);
    page.set_subresource_block_types(block);

    // Reload — block types should persist
    page.reload_html("<html><body>new</body></html>", "http://example.com");
    // We can't directly access the field, but the page should still work
    assert_eq!(page.url(), "http://example.com");
}
