//! Generic SSE protocol handler implementation.
//!
//! [`GenericSseHandler`] classifies events based on simple heuristics:
//! - Events with non-empty data → [`SseMessageKind::Data`]
//! - Events with only a retry field → [`SseMessageKind::Retry`]
//! - Everything else (comments, empty data, etc.) → [`SseMessageKind::System`]

use crate::sse::{protocol::SseProtocolHandler, types::SseMessageKind};

/// A generic SSE handler that classifies events using simple heuristics.
///
/// Suitable for most SSE streams where no exchange-specific classification is
/// needed. Data events are forwarded, empty events are treated as system
/// heartbeats.
#[derive(Clone, Debug, Default)]
pub struct GenericSseHandler;

impl GenericSseHandler {
    /// Create a new generic SSE handler.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl SseProtocolHandler for GenericSseHandler {
    fn classify_event(&self, event: &crate::sse::parse::event::Event) -> SseMessageKind {
        // Retry directives
        if event.retry.is_some() && event.data.is_empty() {
            return SseMessageKind::Retry;
        }

        // Data events (non-empty data payload)
        if !event.data.is_empty() {
            return SseMessageKind::Data;
        }

        // Heartbeat / system events (empty data, no retry)
        SseMessageKind::System
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bytes_utils::Str;

    use super::*;
    use crate::sse::parse::event::Event;

    fn make_event(event: &str, data: &str, id: &str, retry: Option<Duration>) -> Event {
        Event {
            event: Str::from(event),
            data: Str::from(data),
            id: Str::from(id),
            retry,
        }
    }

    #[test]
    fn test_classify_data_event() {
        let handler = GenericSseHandler::new();
        let event = make_event("message", "{\"price\":42000}", "evt-1", None);
        assert_eq!(handler.classify_event(&event), SseMessageKind::Data);
    }

    #[test]
    fn test_classify_retry_event() {
        let handler = GenericSseHandler::new();
        let event = make_event("", "", "", Some(Duration::from_secs(5)));
        assert_eq!(handler.classify_event(&event), SseMessageKind::Retry);
    }

    #[test]
    fn test_classify_system_event() {
        let handler = GenericSseHandler::new();
        let event = make_event("", "", "", None);
        assert_eq!(handler.classify_event(&event), SseMessageKind::System);
    }

    #[test]
    fn test_classify_data_with_retry() {
        let handler = GenericSseHandler::new();
        // Event has both data and retry — data takes precedence
        let event = make_event("message", "hello", "", Some(Duration::from_secs(3)));
        assert_eq!(handler.classify_event(&event), SseMessageKind::Data);
    }

    #[test]
    fn test_classify_named_event_no_data() {
        let handler = GenericSseHandler::new();
        // Named event type but no data → system
        let event = make_event("ping", "", "", None);
        assert_eq!(handler.classify_event(&event), SseMessageKind::System);
    }

    #[test]
    fn test_default_lifecycle_hooks() {
        let handler = GenericSseHandler::new();
        // These should not panic
        handler.on_connect();
        handler.on_disconnect();
    }

    #[test]
    fn test_default_should_retry() {
        let handler = GenericSseHandler::new();
        let err = crate::error::TransportError::sse_stream_ended();
        assert!(handler.should_retry(&err));
    }
}
