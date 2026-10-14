#![allow(dead_code)]
#![allow(unreachable_pub)]
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
pub mod delay_layer;
pub mod delay_server;
pub mod server;

use std::{sync::LazyLock, time::Duration};

use hpx::Client;
use tokio::sync::Semaphore;

// TODO: remove once done converting to new support server?
#[allow(unused)]
pub static DEFAULT_USER_AGENT: &str =
    concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

pub static TEST_SEMAPHORE: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(1));

pub static CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .connect_timeout(Duration::from_secs(60))
        .build()
        .unwrap()
});

#[allow(unused_macros)]
macro_rules! test_emulation {
    ($test_name:ident, $emulation:expr, $ja4:expr, $akamai_hash:expr) => {
        #[tokio::test]
        async fn $test_name() {
            if std::env::var("HPX_NETWORK_TESTS").is_err() {
                eprintln!(
                    "skipping {}: set HPX_NETWORK_TESTS=1 to run network emulation tests",
                    stringify!($test_name)
                );
                return;
            }
            let _permit = crate::support::TEST_SEMAPHORE.acquire().await.unwrap();

            let resp = crate::support::CLIENT
                .get("https://tls.peet.ws/api/all")
                .emulation($emulation)
                .send()
                .await
                .unwrap();

            assert_eq!(resp.status(), hpx::StatusCode::OK);
            let content = resp.text().await.unwrap();

            let conditional = $ja4.iter().any(|&ja4| content.contains(ja4));
            if !conditional {
                println!("{}", content);
            }
            assert!(conditional);

            let conditional = content.contains($akamai_hash);
            if !conditional {
                println!("{}", content);
            }
            assert!(conditional);

            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }
    };
}
