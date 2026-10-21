//! Headless browser engine for hpx: HTML parsing, rendering, CDP, and canvas support.
//!
//! # Lint policy
//!
//! The workspace denies blanket `#[allow]` attributes, so this crate uses
//! `#[expect]` with an explicit reason wherever a lint group cannot be
//! satisfied yet. Every expectation is self-cleaning: once the findings are
//! gone the expectation becomes unfulfilled and the build fails, which forces
//! the suppression to be removed together with the fix.

#![deny(unsafe_code)]
// The public API is documented incrementally; the expectation is crate-scoped
// because the findings are spread over every module.
#![expect(
    missing_docs,
    reason = "public API documentation is written incrementally"
)]
#![expect(
    missing_debug_implementations,
    reason = "Debug impls are added together with the API documentation"
)]
#![expect(
    dead_code,
    reason = "the blitz-dom port still exposes helpers that are not wired up yet"
)]
#![expect(
    clippy::pedantic,
    reason = "pedantic findings are fixed module by module, not crate wide"
)]
#![expect(
    clippy::nursery,
    reason = "nursery findings are triaged per module as they surface"
)]
#![expect(
    clippy::cargo,
    reason = "dependency metadata lints are handled in a dedicated dependency pass"
)]
#![expect(
    clippy::style,
    reason = "style findings are fixed module by module, not crate wide"
)]
#![expect(
    rustdoc::invalid_html_tags,
    reason = "doc comments still contain unescaped generics from the port"
)]
#![cfg_attr(
    test,
    expect(
        clippy::assert_is_empty,
        clippy::cast_lossless,
        clippy::collapsible_if,
        clippy::explicit_iter_loop,
        clippy::expect_used,
        clippy::ignore_without_reason,
        clippy::ignored_unit_patterns,
        clippy::map_unwrap_or,
        clippy::needless_continue,
        clippy::needless_pass_by_ref_mut,
        clippy::needless_raw_string_hashes,
        clippy::panic,
        clippy::print_stdout,
        clippy::redundant_clone,
        clippy::unreadable_literal,
        clippy::unnested_or_patterns,
        clippy::use_self,
        clippy::unwrap_used,
        reason = "unit tests may panic, unwrap, and use test-only idioms"
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
