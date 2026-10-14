#![deny(unused)]
#![deny(unsafe_code)]
#![allow(clippy::extra_unused_lifetimes)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(test, deny(warnings))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
//! # hpx-emulation
//!
//! Browser emulation utilities for hpx.
//!
//! This crate provides browser emulation capabilities for the `hpx` HTTP client:
//!
//! - **Emulation**: Browser emulation profiles (TLS fingerprinting, HTTP/2 settings).
//! - **Fingerprint**: Structured browser fingerprint types for comparison, testing,
//!   and custom profile creation.

// Unit tests are allowed to panic, unwrap, and print.
// Unit tests are allowed to panic, unwrap, and print.
// Unit tests are allowed to panic, unwrap, and print.
#![cfg_attr(
    test,
    allow(
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
        unfulfilled_lint_expectations,
    )
)]
use hpx as _;
#[cfg(feature = "emulation-serde")]
use serde as _;

/// Browser emulation profiles (TLS fingerprinting, HTTP/2 settings).
#[cfg(feature = "emulation")]
pub mod emulation;

#[cfg(feature = "emulation")]
pub use self::emulation::{Emulation, EmulationOS, EmulationOption};

/// Structured browser fingerprint types.
///
/// Provides type-safe, comparable representations of browser TLS/HTTP2/header
/// fingerprints, along with utilities for header composition, fingerprint
/// diffing, and named TLS presets.
pub mod fingerprint;
