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
    missing_docs,
    unfulfilled_lint_expectations
)]
use criterion::{Criterion, criterion_group, criterion_main};

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
            let mut page = hpx_browser::page::Page::new();
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
