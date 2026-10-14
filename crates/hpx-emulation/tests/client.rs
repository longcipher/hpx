//! hpx-emulation test/example crate.

#![cfg(not(target_arch = "wasm32"))]
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
mod support;

use hpx::Client;
use hpx_emulation::{Emulation, EmulationOS, EmulationOption};
use support::server;

#[tokio::test]
async fn test_client_emulation_device() {
    let server = server::http(move |req| async move {
        for (name, value) in req.headers() {
            if name == "user-agent" {
                assert_eq!(
                    value,
                    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36"
                );
            }
            if name == "sec-ch-ua" {
                assert_eq!(
                    value,
                    r#""Not(A:Brand";v="99", "Google Chrome";v="133", "Chromium";v="133""#
                );
            }
            if name == "sec-ch-ua-mobile" {
                assert_eq!(value, "?0");
            }
            if name == "sec-ch-ua-platform" {
                assert_eq!(value, "\"Linux\"");
            }
        }
        http::Response::default()
    });

    let url = format!("http://{}/ua", server.addr());
    let res = Client::builder()
        .emulation(
            EmulationOption::builder()
                .emulation(Emulation::Chrome133)
                .emulation_os(EmulationOS::Linux)
                .skip_http2(true)
                .build(),
        )
        .build()
        .expect("Unable to build client")
        .get(&url)
        .send()
        .await
        .expect("request");

    assert_eq!(res.status(), hpx::StatusCode::OK);
}
