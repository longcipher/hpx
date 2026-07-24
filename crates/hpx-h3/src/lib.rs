//! HTTP/3 client and server.
//!
//! This crate is a fork of [`hyperium/h3`](https://github.com/hyperium/h3) vendored
//! into the hpx workspace with RFC 9220 WebSocket support. It tracks upstream
//! closely; lint configuration lives in `Cargo.toml` to keep the source diff
//! minimal.

// Lint configuration lives in Cargo.toml [lints.clippy] / [lints.rust].
// Per-module overrides (e.g. `#[allow(dead_code)]` on private internals) are
// kept directly on those modules below.

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
