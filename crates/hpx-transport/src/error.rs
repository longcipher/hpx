//! Simplified error handling for the transport layer.

use std::string::FromUtf8Error;

use thiserror::Error;

/// The main result type used throughout the transport layer.
pub type TransportResult<T> = Result<T, TransportError>;

/// Comprehensive error type for all transport operations.
#[derive(Error, Debug)]
pub enum TransportError {
    /// HTTP request errors (wraps hpx::Error)
    #[error("HTTP error: {0}")]
    Http(#[from] hpx::Error),

    /// Authentication and authorization errors
    #[error("Authentication error: {message}")]
    Auth { message: String },

    /// Serialization/deserialization errors
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// API error response
    #[error("API error: status={status}, body={body}")]
    Api {
        status: http::StatusCode,
        body: String,
    },

    /// Rate limiting errors
    #[error("Rate limit exceeded: retry after {retry_after:?}")]
    RateLimit {
        retry_after: Option<std::time::Duration>,
    },

    /// WebSocket errors
    #[error("WebSocket error: {message}")]
    WebSocket { message: String },

    /// Configuration errors
    #[error("Configuration error: {message}")]
    Config { message: String },

    /// Timeout errors
    #[error("Operation timed out after {duration:?}")]
    Timeout { duration: std::time::Duration },

    /// Internal errors (should not happen in normal operation)
    #[error("Internal error: {message}")]
    Internal { message: String },

    /// Request timed out waiting for response.
    #[error("Request timed out after {duration:?}, request_id={request_id}")]
    RequestTimeout {
        duration: std::time::Duration,
        request_id: String,
    },

    /// Subscription operation failed.
    #[error("Subscription failed for topic '{topic}': {message}")]
    SubscriptionFailed { topic: String, message: String },

    /// Maximum reconnection attempts exceeded.
    #[error("Max reconnection attempts exceeded ({attempts})")]
    MaxReconnectAttempts { attempts: u32 },

    /// Protocol-level error from the exchange.
    #[error("Protocol error: {message}")]
    ProtocolError { message: String },

    /// Capacity limit exceeded.
    #[error("Capacity exceeded: {message}")]
    CapacityExceeded { message: String },

    /// Connection was closed.
    #[error("Connection closed: {}", reason.as_deref().unwrap_or("unknown reason"))]
    ConnectionClosed { reason: Option<String> },

    /// SSE: server returned an unexpected HTTP status.
    #[error("SSE invalid status: {status}")]
    SseInvalidStatus { status: http::StatusCode },

    /// SSE: server returned an unexpected content type.
    #[error("SSE invalid content type: {content_type}")]
    SseInvalidContentType { content_type: String },

    /// SSE: failed to parse an event from the stream.
    #[error("SSE parse error: {message}")]
    SseParse { message: String },

    /// SSE: the event stream ended unexpectedly.
    #[error("SSE stream ended")]
    SseStreamEnded,
}

impl From<FromUtf8Error> for TransportError {
    fn from(e: FromUtf8Error) -> Self {
        Self::Serialization(serde_json::Error::io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            e.to_string(),
        )))
    }
}

#[cfg(feature = "ws-fastwebsockets")]
impl From<fastwebsockets::WebSocketError> for TransportError {
    fn from(e: fastwebsockets::WebSocketError) -> Self {
        Self::WebSocket {
            message: e.to_string(),
        }
    }
}

impl TransportError {
    /// Create a configuration error.
    pub fn config(message: impl Into<String>) -> Self {
        Self::Config {
            message: message.into(),
        }
    }

    /// Create an authentication error.
    pub fn auth(message: impl Into<String>) -> Self {
        Self::Auth {
            message: message.into(),
        }
    }

    /// Create an internal error.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// Create a WebSocket error.
    pub fn websocket(message: impl Into<String>) -> Self {
        Self::WebSocket {
            message: message.into(),
        }
    }

    /// Create a timeout error.
    pub fn timeout(duration: std::time::Duration) -> Self {
        Self::Timeout { duration }
    }

    /// Create a rate limit error.
    pub fn rate_limit(retry_after: Option<std::time::Duration>) -> Self {
        Self::RateLimit { retry_after }
    }

    /// Create an API error.
    pub fn api(status: http::StatusCode, body: impl Into<String>) -> Self {
        Self::Api {
            status,
            body: body.into(),
        }
    }

    /// Create a request timeout error.
    pub fn request_timeout(duration: std::time::Duration, request_id: impl Into<String>) -> Self {
        Self::RequestTimeout {
            duration,
            request_id: request_id.into(),
        }
    }

    /// Create a subscription failed error.
    pub fn subscription_failed(topic: impl Into<String>, message: impl Into<String>) -> Self {
        Self::SubscriptionFailed {
            topic: topic.into(),
            message: message.into(),
        }
    }

    /// Create a max reconnect attempts error.
    pub fn max_reconnect_attempts(attempts: u32) -> Self {
        Self::MaxReconnectAttempts { attempts }
    }

    /// Create a protocol error.
    pub fn protocol_error(message: impl Into<String>) -> Self {
        Self::ProtocolError {
            message: message.into(),
        }
    }

    /// Create a capacity exceeded error.
    pub fn capacity_exceeded(message: impl Into<String>) -> Self {
        Self::CapacityExceeded {
            message: message.into(),
        }
    }

    /// Create a connection closed error.
    pub fn connection_closed(reason: Option<String>) -> Self {
        Self::ConnectionClosed { reason }
    }

    /// Create an SSE invalid status error.
    pub fn sse_invalid_status(status: http::StatusCode) -> Self {
        Self::SseInvalidStatus { status }
    }

    /// Create an SSE invalid content type error.
    pub fn sse_invalid_content_type(content_type: impl Into<String>) -> Self {
        Self::SseInvalidContentType {
            content_type: content_type.into(),
        }
    }

    /// Create an SSE parse error.
    pub fn sse_parse(message: impl Into<String>) -> Self {
        Self::SseParse {
            message: message.into(),
        }
    }

    /// Create an SSE stream ended error.
    pub fn sse_stream_ended() -> Self {
        Self::SseStreamEnded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let err = TransportError::config("Invalid URL");
        assert!(matches!(err, TransportError::Config { .. }));

        let err = TransportError::timeout(std::time::Duration::from_secs(5));
        assert!(matches!(err, TransportError::Timeout { .. }));

        let err = TransportError::auth("Invalid API key");
        assert!(matches!(err, TransportError::Auth { .. }));
    }

    #[test]
    fn test_sse_invalid_status() {
        let err = TransportError::sse_invalid_status(http::StatusCode::FORBIDDEN);
        assert!(matches!(
            err,
            TransportError::SseInvalidStatus { status } if status == http::StatusCode::FORBIDDEN
        ));
        assert_eq!(err.to_string(), "SSE invalid status: 403 Forbidden");
    }

    #[test]
    fn test_sse_invalid_content_type() {
        let err = TransportError::sse_invalid_content_type("application/json");
        assert!(matches!(err, TransportError::SseInvalidContentType { .. }));
        assert_eq!(
            err.to_string(),
            "SSE invalid content type: application/json"
        );
    }

    #[test]
    fn test_sse_parse() {
        let err = TransportError::sse_parse("unexpected EOF");
        assert!(matches!(err, TransportError::SseParse { .. }));
        assert_eq!(err.to_string(), "SSE parse error: unexpected EOF");
    }

    #[test]
    fn test_sse_stream_ended() {
        let err = TransportError::sse_stream_ended();
        assert!(matches!(err, TransportError::SseStreamEnded));
        assert_eq!(err.to_string(), "SSE stream ended");
    }
}
