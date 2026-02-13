//! Protocol handler trait for SSE exchange abstraction.
//!
//! Provides the [`SseProtocolHandler`] trait for exchange-specific SSE event
//! classification and lifecycle hooks.

use crate::{error::TransportError, sse::types::SseMessageKind};

/// Trait for handling exchange-specific SSE protocol details.
///
/// Implementors classify incoming SSE events, react to connection lifecycle
/// transitions, and decide whether reconnection should occur after errors.
///
/// SSE connections are read-only (server → client); the handler only observes
/// and classifies — it never sends messages back.
pub trait SseProtocolHandler: Send + Sync + 'static {
    /// Classify an incoming SSE event into a [`SseMessageKind`].
    ///
    /// Called for every event received from the stream. The classification
    /// determines how the event is routed and whether it is forwarded to the
    /// consumer.
    fn classify_event(&self, event: &super::parse::event::Event) -> SseMessageKind;

    /// Called when a connection is successfully established (or re-established).
    ///
    /// Default implementation does nothing.
    fn on_connect(&self) {}

    /// Called when a connection is lost or closed.
    ///
    /// Default implementation does nothing.
    fn on_disconnect(&self) {}

    /// Determine whether the connection should be retried after the given error.
    ///
    /// Default implementation returns `true` for all errors (always retry).
    fn should_retry(&self, _error: &TransportError) -> bool {
        true
    }
}
