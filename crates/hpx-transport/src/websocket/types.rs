//! Core type definitions for WebSocket protocol handling.

use std::fmt;

/// Unique identifier for request-response correlation.
/// Uses ULID for lexicographically sortable, unique IDs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RequestId(String);

impl RequestId {
    /// Generate a new unique request ID using ULID.
    pub fn new() -> Self {
        Self(ulid::Ulid::new().to_string())
    }

    /// Create from an existing string.
    pub fn from_string(s: String) -> Self {
        Self(s)
    }

    /// Get the inner string value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for RequestId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for RequestId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Topic identifier for subscription routing.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Topic(String);

impl Topic {
    /// Create a new topic from a string.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Get the inner string value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Topic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for Topic {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for Topic {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Classification of incoming WebSocket messages.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MessageKind {
    /// Response to a specific request (has request_id).
    Response,
    /// Push update for a subscription (has topic).
    Update,
    /// System-level message (ping, pong, heartbeat).
    System,
    /// Control message (subscription confirmations, errors).
    Control,
    /// Unrecognized message type.
    Unknown,
}

impl MessageKind {
    /// Returns true if this is a response that should resolve a pending request.
    pub fn is_response(&self) -> bool {
        matches!(self, Self::Response)
    }

    /// Returns true if this is an update to be published to subscribers.
    pub fn is_update(&self) -> bool {
        matches!(self, Self::Update)
    }

    /// Returns true if this is a system-level message.
    pub fn is_system(&self) -> bool {
        matches!(self, Self::System)
    }

    /// Returns true if this is a control message.
    pub fn is_control(&self) -> bool {
        matches!(self, Self::Control)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_id_uniqueness() {
        let id1 = RequestId::new();
        let id2 = RequestId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_request_id_from_string() {
        let id = RequestId::from_string("test-id".to_string());
        assert_eq!(id.as_str(), "test-id");
    }

    #[test]
    fn test_request_id_display() {
        let id = RequestId::from_string("my-request-123".to_string());
        assert_eq!(format!("{}", id), "my-request-123");
    }

    #[test]
    fn test_request_id_from_str() {
        let id: RequestId = "test-id".into();
        assert_eq!(id.as_str(), "test-id");
    }

    #[test]
    fn test_request_id_default() {
        let id1 = RequestId::default();
        let id2 = RequestId::default();
        assert_ne!(id1, id2); // Each default is unique
    }

    #[test]
    fn test_topic_creation() {
        let topic = Topic::new("orderbook.BTC");
        assert_eq!(topic.as_str(), "orderbook.BTC");
    }

    #[test]
    fn test_topic_from_str() {
        let topic: Topic = "trades.ETH".into();
        assert_eq!(topic.to_string(), "trades.ETH");
    }

    #[test]
    fn test_topic_display() {
        let topic = Topic::new("depth.BTCUSDT");
        assert_eq!(format!("{}", topic), "depth.BTCUSDT");
    }

    #[test]
    fn test_topic_from_string() {
        let topic: Topic = String::from("kline.1m.ETHUSDT").into();
        assert_eq!(topic.as_str(), "kline.1m.ETHUSDT");
    }

    #[test]
    fn test_message_kind_is_response() {
        assert!(MessageKind::Response.is_response());
        assert!(!MessageKind::Update.is_response());
        assert!(!MessageKind::System.is_response());
        assert!(!MessageKind::Control.is_response());
        assert!(!MessageKind::Unknown.is_response());
    }

    #[test]
    fn test_message_kind_is_update() {
        assert!(!MessageKind::Response.is_update());
        assert!(MessageKind::Update.is_update());
        assert!(!MessageKind::System.is_update());
        assert!(!MessageKind::Control.is_update());
        assert!(!MessageKind::Unknown.is_update());
    }

    #[test]
    fn test_message_kind_is_system() {
        assert!(!MessageKind::Response.is_system());
        assert!(!MessageKind::Update.is_system());
        assert!(MessageKind::System.is_system());
        assert!(!MessageKind::Control.is_system());
        assert!(!MessageKind::Unknown.is_system());
    }

    #[test]
    fn test_message_kind_is_control() {
        assert!(!MessageKind::Response.is_control());
        assert!(!MessageKind::Update.is_control());
        assert!(!MessageKind::System.is_control());
        assert!(MessageKind::Control.is_control());
        assert!(!MessageKind::Unknown.is_control());
    }
}
