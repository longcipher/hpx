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
use hpx_browser::tls::DeviceFingerprint;

fn bench_fingerprint_build(c: &mut Criterion) {
    // ponytail: actual TLS handshake requires a live server; benchmarking
    // fingerprint construction as a proxy for per-connection overhead.
    let mut group = c.benchmark_group("tls");
    group.bench_function("fingerprint_chrome", |b| {
        b.iter(DeviceFingerprint::chrome_147);
    });
    group.bench_function("fingerprint_safari", |b| {
        b.iter(DeviceFingerprint::safari_ios_18);
    });
    group.bench_function("fingerprint_firefox", |b| {
        b.iter(DeviceFingerprint::firefox_135);
    });
    group.bench_function("to_tls_options", |b| {
        let fp = DeviceFingerprint::chrome_147();
        b.iter(|| fp.to_tls_options());
    });
    group.finish();
}

criterion_group!(benches, bench_fingerprint_build);
criterion_main!(benches);
