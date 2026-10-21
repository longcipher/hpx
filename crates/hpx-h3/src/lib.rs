//! HTTP/3 client and server.
//!
//! This crate is a fork of [`hyperium/h3`](https://github.com/hyperium/h3) vendored
//! into the hpx workspace. It adds the RFC 9220 `:protocol` pseudo-header
//! constant (`Protocol::WEBSOCKET`); the WebSocket-over-HTTP/3 implementation
//! itself lives in the `hpx` crate. The fork tracks upstream closely; lint
//! configuration lives in `Cargo.toml` to keep the source diff minimal.

// Lint configuration lives in Cargo.toml [lints.clippy] / [lints.rust].
// Per-module overrides (e.g. `#[allow(dead_code)]` on private internals) are
// kept directly on those modules below.

// Unit tests are allowed to panic, print, and use loose assertions. Production
// code stays under the stricter Cargo.toml lint table.
#![cfg_attr(
    test,
    allow(
        clippy::allow_attributes,
        clippy::assert_is_empty,
        clippy::cloned_ref_to_slice_refs,
        clippy::print_stdout,
        clippy::should_panic_without_expect,
        clippy::unimplemented,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic
    )
)]

pub mod client;

mod config;
pub mod ext;
pub mod quic;

pub mod server;

mod buf;

mod shared_state;

#[cfg(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes")]
pub use shared_state::{ConnectionState, SharedState};

pub mod error;

#[cfg(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes")]
#[allow(missing_docs)]
pub mod connection;
#[cfg(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes")]
#[allow(missing_docs)]
pub mod frame;
#[cfg(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes")]
#[allow(missing_docs)]
pub mod proto;
#[cfg(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes")]
#[allow(dead_code, missing_docs)]
pub mod qpack;
#[cfg(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes")]
#[allow(missing_docs)]
pub mod stream;
#[cfg(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes")]
#[allow(missing_docs)]
pub mod webtransport;

#[cfg(not(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes"))]
mod connection;
#[cfg(not(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes"))]
mod frame;
#[cfg(not(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes"))]
mod proto;
#[cfg(not(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes"))]
#[allow(dead_code)]
mod qpack;
#[cfg(not(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes"))]
mod stream;
#[cfg(not(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes"))]
mod webtransport;

/// Quinn QUIC transport backend (merged from hpx-h3-quinn).
#[cfg(feature = "quinn")]
pub mod quinn;
