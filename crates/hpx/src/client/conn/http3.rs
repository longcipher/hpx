//! HTTP/3 connection adapter.
//!
//! Re-exports [`H3Connection`] from the [`quic`](super::quic) module and
//! provides adapter logic so that `H3Connection` is usable wherever the
//! [`Connection`](super::Connection) trait is expected — e.g. by the
//! dispatcher in `client::http::client`.

#[expect(unused_imports)]
pub(crate) use super::quic::H3Connection;
