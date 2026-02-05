//! Protocol handler trait for exchange-agnostic WebSocket handling.
//!
//! The [`ProtocolHandler`] trait abstracts protocol-specific details like
//! message framing, authentication, and heartbeat handling.

use super::types::{MessageKind, RequestId, Topic};
use crate::error::TransportResult;

/// Message representation for WebSocket communication.
#[derive(Clone, Debug)]
pub enum WsMessage {
    /// Text message.
    Text(String),
    /// Binary message.
    Binary(Vec<u8>),
}

impl WsMessage {
    /// Create a text message.
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }

    /// Create a binary message.
    pub fn binary(data: impl Into<Vec<u8>>) -> Self {
        Self::Binary(data.into())
    }

    /// Get as text if this is a text message.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            Self::Binary(_) => None,
        }
    }

    /// Get as bytes regardless of message type.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Text(s) => s.as_bytes(),
            Self::Binary(b) => b,
        }
    }

    /// Check if this is a text message.
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text(_))
    }

    /// Check if this is a binary message.
    pub fn is_binary(&self) -> bool {
        matches!(self, Self::Binary(_))
    }
}

/// Trait for handling exchange-specific WebSocket protocols.
///
/// Implement this trait to add support for a new exchange. The trait
/// abstracts all protocol-specific details:
///
/// - Message classification and routing
/// - Authentication flow
/// - Subscription/unsubscription message building
/// - Heartbeat handling (ping/pong)
///
/// # Example
///
/// ```rust,ignore
/// struct MyExchangeHandler {
///     api_key: String,
///     api_secret: String,
/// }
///
/// impl ProtocolHandler for MyExchangeHandler {
///     fn classify_message(&self, message: &str) -> MessageKind {
///         // Parse JSON and determine message type
///         MessageKind::Update
///     }
///     // ... other methods
/// }
/// ```
pub trait ProtocolHandler: Send + Sync + 'static {
    // ========================
    // Connection Lifecycle
    // ========================

    /// Messages to send immediately after connection is established.
    ///
    /// Override this to send initial handshake or identification messages.
    /// Default returns empty vec (no initial messages).
    fn on_connect(&self) -> Vec<WsMessage> {
        Vec::new()
    }

    /// Build authentication message if required.
    ///
    /// Return `Some(message)` if authentication should be sent after connect,
    /// or `None` if no authentication is needed.
    fn build_auth_message(&self) -> Option<WsMessage> {
        None
    }

    /// Check if the given message indicates successful authentication.
    fn is_auth_success(&self, message: &str) -> bool {
        let _ = message;
        true
    }

    /// Check if the given message indicates authentication failure.
    fn is_auth_failure(&self, message: &str) -> bool {
        let _ = message;
        false
    }

    // ========================
    // Message Classification
    // ========================

    /// Classify an incoming message by type.
    ///
    /// This is the primary routing mechanism. Based on the returned
    /// [`MessageKind`], the actor will:
    /// - `Response`: Route to pending request via `extract_request_id`
    /// - `Update`: Route to subscribers via `extract_topic`
    /// - `System`: Handle ping/pong internally
    /// - `Control`: Log and process (subscription confirmations, etc.)
    /// - `Unknown`: Log warning and ignore
    fn classify_message(&self, message: &str) -> MessageKind;

    /// Extract request ID from a response message.
    ///
    /// Called when `classify_message` returns `MessageKind::Response`.
    fn extract_request_id(&self, message: &str) -> Option<RequestId>;

    /// Extract topic from an update message.
    ///
    /// Called when `classify_message` returns `MessageKind::Update`.
    fn extract_topic(&self, message: &str) -> Option<Topic>;

    // ========================
    // Message Building
    // ========================

    /// Build a subscription message for the given topics.
    ///
    /// The `request_id` can be embedded in the message for tracking
    /// subscription confirmations.
    fn build_subscribe(&self, topics: &[Topic], request_id: RequestId) -> WsMessage;

    /// Build an unsubscription message for the given topics.
    fn build_unsubscribe(&self, topics: &[Topic], request_id: RequestId) -> WsMessage;

    /// Build an application-level ping message.
    ///
    /// Return `None` to use WebSocket protocol-level pings instead.
    /// Some exchanges require periodic application-level ping messages
    /// (e.g., `{"op": "ping"}`).
    fn build_ping(&self) -> Option<WsMessage> {
        None
    }

    /// Build a response to a server ping.
    ///
    /// Some exchanges send application-level pings that require a matching pong.
    fn build_pong(&self, ping_data: &[u8]) -> Option<WsMessage> {
        let _ = ping_data;
        None
    }

    // ========================
    // Message Processing
    // ========================

    /// Decode binary message to string.
    ///
    /// Override this for exchanges that use compression (gzip, deflate)
    /// or custom binary formats.
    fn decode_binary(&self, data: &[u8]) -> TransportResult<String> {
        String::from_utf8(data.to_vec()).map_err(Into::into)
    }

    /// Check if the message is a server-initiated ping.
    fn is_server_ping(&self, message: &str) -> bool {
        let _ = message;
        false
    }

    /// Check if the message is a pong response.
    fn is_pong_response(&self, message: &str) -> bool {
        let _ = message;
        false
    }

    /// Check if the message indicates subscription success.
    fn is_subscription_success(&self, message: &str, topics: &[Topic]) -> bool {
        let _ = (message, topics);
        false
    }

    /// Check if the message indicates we should reconnect.
    ///
    /// Some exchanges send explicit "please reconnect" messages.
    fn should_reconnect(&self, message: &str) -> bool {
        let _ = message;
        false
    }

    // ========================
    // Request Building
    // ========================

    /// Generate a new request ID for outgoing requests.
    ///
    /// Default implementation uses ULID.
    fn generate_request_id(&self) -> RequestId {
        RequestId::new()
    }

    /// Inject or update request ID in a request message.
    ///
    /// Some protocols require embedding the request ID in the message payload.
    /// Return the modified message with the request ID embedded, or the original
    /// message if no modification is needed.
    fn inject_request_id(&self, message: WsMessage, request_id: &RequestId) -> WsMessage {
        let _ = request_id;
        message
    }
}
