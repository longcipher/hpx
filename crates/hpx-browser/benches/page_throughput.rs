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
use hpx_browser::html_parser::parse_html;

fn bench_parse_html(c: &mut Criterion) {
    let small = "<html><head><title>t</title></head><body><p>hello</p></body></html>";
    let medium = format!(
        "<html><head><title>test</title></head><body>{}</body></html>",
        "<p>line</p>".repeat(500)
    );
    let large = format!(
        "<html><head><title>test</title></head><body>{}</body></html>",
        "<div class=\"item\"><span>content</span><a href=\"#\">link</a></div>".repeat(2000)
    );

    let mut group = c.benchmark_group("parse_html");
    for (name, html) in [
        ("small", small),
        ("medium", medium.as_str()),
        ("large", large.as_str()),
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(name), html, |b, html| {
            b.iter(|| parse_html(html));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_parse_html);
criterion_main!(benches);
