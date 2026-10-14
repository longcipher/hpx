//! Integration tests for SSRF protection at the HttpClient boundary.
//!
//! Spins up a local HTTP server on 127.0.0.1 and verifies that:
//! - Default HttpClient rejects loopback requests (SSRF)
//! - HttpClient with `allow_private_network(true)` connects successfully

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
use axum::{Router, routing::get};
use hpx_browser::net::{HttpClient, RedirectPolicy};
use tokio::net::TcpListener;

/// Start a minimal HTTP server on a random loopback port, return its URL.
async fn start_test_server() -> String {
    let app = Router::new().route("/", get(|| async { "ok" }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn ssrf_blocks_loopback_by_default() {
    let server_url = start_test_server().await;
    let client = HttpClient::new(hpx::BrowserProfile::Chrome).unwrap();

    let result = client
        .request("GET", &server_url, None, &[], RedirectPolicy::Manual)
        .await;

    assert!(result.is_err(), "SSRF should block loopback by default");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("SSRF"),
        "error should be SSRF rejection, got: {err}"
    );
}

#[tokio::test]
async fn ssrf_allows_loopback_when_permitted() {
    let server_url = start_test_server().await;
    let client = HttpClient::new(hpx::BrowserProfile::Chrome)
        .unwrap()
        .allow_private_network(true);

    let result = client
        .request("GET", &server_url, None, &[], RedirectPolicy::Manual)
        .await;

    assert!(
        result.is_ok(),
        "should connect to loopback when allow_private_network is true: {:?}",
        result.err()
    );
    let resp = result.unwrap();
    assert!(resp.ok(), "server should return success status");
}
