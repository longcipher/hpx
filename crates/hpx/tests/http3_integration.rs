#![allow(missing_docs)]
#![cfg(feature = "http3")]
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
use hpx::Client;
use http::Version;

type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Real-network HTTP/3 integration test against Cloudflare's trace endpoint.
///
/// Sends a GET request to `https://cloudflare.com/cdn-cgi/trace` using
/// `Client::builder().http3_only().build()` and asserts the response is
/// delivered over HTTP/3 with status 200 OK and the expected trace body.
///
/// This test is marked `#[ignore]` because it requires real network access.
/// Run manually with:
///
/// ```text
/// cargo test -p hpx --features http3 --test http3_integration \
///   http3_get_cloudflare_trace -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore]
async fn http3_get_cloudflare_trace() -> TestResult<()> {
    let client = Client::builder().no_proxy().http3_only().build()?;

    let response = client
        .get("https://cloudflare.com/cdn-cgi/trace")
        .send()
        .await?;

    assert_eq!(
        response.status(),
        http::StatusCode::OK,
        "expected 200 OK from Cloudflare trace endpoint"
    );

    assert_eq!(
        response.version(),
        Version::HTTP_3,
        "expected HTTP/3 version from Cloudflare trace endpoint"
    );

    let body = response.text().await?;
    assert!(
        body.contains("h="),
        "expected trace output to contain 'h=' (host header line), got: {body}"
    );

    Ok(())
}
