//! Core type definitions for SSE event handling.

use std::fmt;

/// Classification of incoming SSE events (analogous to
/// [`websocket::MessageKind`](crate::websocket::MessageKind)).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SseMessageKind {
    /// A data event (has event type + data payload).
    Data,
    /// System/control event (heartbeat, ping comments).
    System,
    /// Retry directive from server.
    Retry,
    /// Unknown/unclassified event.
    Unknown,
}

impl SseMessageKind {
    /// Returns true if this is a data event.
    pub fn is_data(&self) -> bool {
        matches!(self, Self::Data)
    }

    /// Returns true if this is a system-level event.
    pub fn is_system(&self) -> bool {
        matches!(self, Self::System)
    }

    /// Returns true if this is a retry directive.
    pub fn is_retry(&self) -> bool {
        matches!(self, Self::Retry)
    }

    /// Returns true if this is an unknown/unclassified event.
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }
}

impl fmt::Display for SseMessageKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Data => write!(f, "Data"),
            Self::System => write!(f, "System"),
            Self::Retry => write!(f, "Retry"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Wrapper around [`super::parse::event::Event`] with classification metadata.
#[derive(Clone, Debug)]
pub struct SseEvent {
    /// The raw SSE event.
    pub raw: super::parse::event::Event,
    /// Classified message kind.
    pub kind: SseMessageKind,
}

impl SseEvent {
    /// Create a new SSE event with the given raw event and kind.
    pub fn new(raw: super::parse::event::Event, kind: SseMessageKind) -> Self {
        Self { raw, kind }
    }

    /// Convenience accessor for the event's data field.
    pub fn data(&self) -> &str {
        &self.raw.data
    }

    /// Convenience accessor for the event type field.
    pub fn event_type(&self) -> &str {
        &self.raw.event
    }

    /// Convenience accessor for the event ID field.
    pub fn id(&self) -> &str {
        &self.raw.id
    }

    /// Convenience accessor for the retry duration, if present.
    pub fn retry(&self) -> Option<std::time::Duration> {
        self.raw.retry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_kind_is_data() {
        assert!(SseMessageKind::Data.is_data());
        assert!(!SseMessageKind::System.is_data());
        assert!(!SseMessageKind::Retry.is_data());
        assert!(!SseMessageKind::Unknown.is_data());
    }

    #[test]
    fn test_message_kind_is_system() {
        assert!(!SseMessageKind::Data.is_system());
        assert!(SseMessageKind::System.is_system());
        assert!(!SseMessageKind::Retry.is_system());
        assert!(!SseMessageKind::Unknown.is_system());
    }

    #[test]
    fn test_message_kind_is_retry() {
        assert!(!SseMessageKind::Data.is_retry());
        assert!(!SseMessageKind::System.is_retry());
        assert!(SseMessageKind::Retry.is_retry());
        assert!(!SseMessageKind::Unknown.is_retry());
    }

    #[test]
    fn test_message_kind_is_unknown() {
        assert!(!SseMessageKind::Data.is_unknown());
        assert!(!SseMessageKind::System.is_unknown());
        assert!(!SseMessageKind::Retry.is_unknown());
        assert!(SseMessageKind::Unknown.is_unknown());
    }

    #[test]
    fn test_message_kind_display() {
        assert_eq!(SseMessageKind::Data.to_string(), "Data");
        assert_eq!(SseMessageKind::System.to_string(), "System");
        assert_eq!(SseMessageKind::Retry.to_string(), "Retry");
        assert_eq!(SseMessageKind::Unknown.to_string(), "Unknown");
    }

    #[test]
    fn test_sse_event_accessors() {
        use std::time::Duration;

        use bytes_utils::Str;

        let raw = crate::sse::parse::event::Event {
            event: Str::from("message"),
            data: Str::from("{\"price\": 42000}"),
            id: Str::from("evt-123"),
            retry: Some(Duration::from_secs(5)),
        };

        let event = SseEvent::new(raw, SseMessageKind::Data);

        assert_eq!(event.event_type(), "message");
        assert_eq!(event.data(), "{\"price\": 42000}");
        assert_eq!(event.id(), "evt-123");
        assert_eq!(event.retry(), Some(Duration::from_secs(5)));
        assert!(event.kind.is_data());
    }

    #[test]
    fn test_sse_event_no_retry() {
        use bytes_utils::Str;

        let raw = crate::sse::parse::event::Event {
            event: Str::from("update"),
            data: Str::from("hello"),
            id: Str::from(""),
            retry: None,
        };

        let event = SseEvent::new(raw, SseMessageKind::System);

        assert_eq!(event.event_type(), "update");
        assert_eq!(event.data(), "hello");
        assert!(event.id().is_empty());
        assert!(event.retry().is_none());
        assert!(event.kind.is_system());
    }
}
