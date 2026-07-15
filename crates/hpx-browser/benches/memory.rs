use criterion::{Criterion, criterion_group, criterion_main};
use hpx_browser::html_parser::parse_html;

fn current_rss_bytes() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // RUSAGE_SELF = 0 on both macOS and Linux
    unsafe {
        if libc::getrusage(0, usage.as_mut_ptr()) == 0 {
            let usage = usage.assume_init();
            // ru_maxrss is in bytes on macOS, KB on Linux
            #[cfg(target_os = "macos")]
            return usage.ru_maxrss as u64;
            #[cfg(target_os = "linux")]
            return usage.ru_maxrss as u64 * 1024;
        }
    }
    0
}

fn bench_memory_100_navigations(c: &mut Criterion) {
    let html = include_str!("test_page.html");
    let initial_rss = current_rss_bytes();

    c.bench_function("memory_100_navigations", |b| {
        b.iter(|| {
            let url = "http://example.com";
            // Simulate 100 navigations: create Page, reload_html 100 times, drop
            let engine = hpx_browser::host::EngineHandle::new();
            let mut page = hpx_browser::page::Page::new(engine);
            for _ in 0..100 {
                page.reload_html(html, url);
            }
            drop(page);
        });
    });

    let final_rss = current_rss_bytes();
    let delta = final_rss.saturating_sub(initial_rss);
    // Report RSS growth (should be < 64MB = 67_108_864 bytes)
    eprintln!(
        "RSS delta: {delta} bytes ({:.1} MB)",
        delta as f64 / 1_048_576.0
    );
}

criterion_group!(benches, bench_memory_100_navigations);
criterion_main!(benches);
