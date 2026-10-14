// TODO: progressively remove these blanket clippy allows as the crate matures
#![allow(clippy::pedantic)]
#![allow(clippy::nursery)]
#![allow(clippy::cargo)]
#![allow(clippy::style)]
#![allow(clippy::allow_attributes)]
#![allow(clippy::panic)]
// TODO: remove once dom.rs blitz-dom API errors are resolved and full build passes;
// then fix dead code warnings with targeted #[allow(dead_code)] on specific items
#![allow(dead_code)]
#![allow(missing_docs)]
#![allow(missing_debug_implementations)]
#![allow(rustdoc::missing_crate_level_docs)]
#![allow(rustdoc::invalid_html_tags)]
#![deny(unsafe_code)]
// Unit tests are allowed to panic, unwrap, and print.
// Unit tests are allowed to panic, unwrap, and print.
// Unit tests are allowed to panic, unwrap, and print.
#![cfg_attr(
    test,
    allow(
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
        unfulfilled_lint_expectations,
    )
)]
pub mod challenge;
pub mod css_engine;
pub mod dom;
pub mod extract;
pub mod html_parser;
pub mod iframe;
pub mod layout;
pub mod markdown;
pub mod net;
pub mod page;
pub mod parallel;
pub mod pool;
pub mod resource_loader;
pub mod stealth;
pub mod tls;
pub mod utils;

#[cfg(feature = "v8")]
pub mod js_runtime;

#[cfg(feature = "v8")]
pub mod event_loop;

#[cfg(feature = "canvas")]
pub mod canvas;

#[cfg(feature = "workers")]
pub mod workers;

#[cfg(feature = "cdp")]
pub mod protocol;

#[cfg(feature = "cdp-client")]
pub mod cdp_client;

#[cfg(feature = "cdp-client")]
pub mod chrome;

#[cfg(feature = "cdp-client")]
pub mod cdp_page;

#[cfg(feature = "cdp-client")]
pub mod locator;

#[cfg(feature = "cdp-client")]
pub mod har;
