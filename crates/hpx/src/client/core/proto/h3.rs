//! HTTP/3 protocol layer.
//!
//! Mirrors the structure of [`super::h2`] and is gated on the `http3` Cargo
//! feature per [C-01]. Provides [`ConnTask`], the dispatch loop that drives
//! the h3 connection's `SendRequest` and correlates requests with responses
//! via the existing [`crate::client::core::dispatch`] channel.
//!
//! Scope (T1.9):
//! - [`ConnTask`] consumes an [`H3Connection`](crate::client::conn::quic::H3Connection)'s
//!   `send_request` and `close_rx` and runs the per-request lifecycle
//!   (send_request → finish → recv_response → drain body) on spawned tasks.
//! - Request body sending is limited to bodyless requests in T1.9; the full
//!   body-sending loop (send_data, send_trailers, Content-Length framing) is
//!   T1.11 scope.
//! - Extended CONNECT (RFC 9220) and Alt-Svc discovery are out of scope
//!   (Phase 2).
//!
//! T1.9 note: [`ConnTask`] is not yet re-exported at the module level because
//! nothing in T1.9 consumes it via `proto::h3::ConnTask`. The re-export
//! (`pub(crate) use self::client::ConnTask;`) will be added in T1.10 when the
//! pool/client pipeline wiring references it, mirroring
//! `proto::h2::ClientTask`.
pub(crate) mod client;
pub(crate) mod dispatch;
