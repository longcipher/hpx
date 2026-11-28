#![deny(unused)]
#![deny(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
// Unit tests are allowed to panic, unwrap, and print.
#![cfg_attr(
    test,
    allow(
        clippy::allow_attributes,
        clippy::assert_is_empty,
        clippy::collection_is_never_read,
        clippy::expect_used,
        clippy::panic,
        clippy::print_stderr,
        clippy::print_stdout,
        clippy::redundant_clone,
        clippy::unwrap_used,
        clippy::useless_vec,
        missing_docs,
        unfulfilled_lint_expectations
    )
)]

//! Exchange transport governance for HFT clients.
//!
//! `hpx-transport` is deliberately transport-agnostic: it contains no sockets,
//! no TLS and no JSON. It is the deterministic policy layer that sits between
//! an exchange adapter and `hpx`'s REST/WebSocket clients:
//!
//! - [`rate_limit`] — three independent quota pools (IP request weight,
//!   account order rate, WebSocket frame rate) with server-response
//!   calibration so local clock drift cannot cause `429`/`418` bans.
//! - [`auth`] — per-message dynamic signing hooks for venues that require a
//!   fresh short-lived JWT for every subscription (Coinbase uses 120 s).
//! - [`book`] — a snapshot + buffered-delta alignment state machine that
//!   replays deltas by sequence id after a reconnect.
//!
//! All clocks are injectable through [`rate_limit::Clock`] and all state is
//! expressed with atomics, one short-lived mutex per quota window, or plain
//! containers, so every behaviour is unit testable without a network.
//!
//! These are standalone crate features, not `hpx` feature flags, and are
//! therefore **not** part of the `hft` or `stealth` presets.

#[cfg(feature = "auth")]
#[cfg_attr(docsrs, doc(cfg(feature = "auth")))]
pub mod auth;

#[cfg(feature = "book")]
#[cfg_attr(docsrs, doc(cfg(feature = "book")))]
pub mod book;

#[cfg(feature = "rate-limit")]
#[cfg_attr(docsrs, doc(cfg(feature = "rate-limit")))]
pub mod rate_limit;
