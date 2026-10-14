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
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use hpx_browser::challenge::engine_classify;

fn bench_engine_classify(c: &mut Criterion) {
    let clean = "<html><head><title>OK</title></head><body><p>Normal page</p></body></html>";
    let cf_challenge =
        "<html><body><div id=\"cf-browser-verification\">just a moment</div></body></html>";
    let thin = "<html><body>x</body></html>";
    let large_clean = format!(
        "<html><body>{}</body></html>",
        "<p>content</p>".repeat(10_000)
    );
    let large_challenge = format!(
        "<html><body>{}<div>just a moment</div>{}</body></html>",
        "<!--padding-->".repeat(5000),
        "<!--more-->".repeat(5000),
    );

    let mut group = c.benchmark_group("engine_classify");
    for (name, body) in [
        ("clean", clean.to_string()),
        ("cf_challenge", cf_challenge.to_string()),
        ("thin_body", thin.to_string()),
        ("large_clean", large_clean),
        ("large_challenge", large_challenge),
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(name), &body, |b, body| {
            b.iter(|| engine_classify(body));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_engine_classify);
criterion_main!(benches);
