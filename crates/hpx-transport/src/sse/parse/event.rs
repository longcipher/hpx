//! Representation of SSE events based on the
//! [HTML Living Standard](https://html.spec.whatwg.org/multipage/server-sent-events.html).

use core::time::Duration;

use bytes_utils::Str;

/// Event with immutable fields from an
/// [`EventStream`](super::event_stream::EventStream).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Event {
    /// The event type field (defaults to `"message"` when unspecified).
    pub event: Str,
    /// The data payload.
    pub data: Str,
    /// The last event ID.
    pub id: Str,
    /// Optional retry duration (milliseconds) advertised by the server.
    pub retry: Option<Duration>,
}
