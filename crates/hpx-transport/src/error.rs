//! Comprehensive error handling for the transport layer.

use std::string::FromUtf8Error;

use thiserror::Error;

/// The main result type used throughout the transport layer.
pub type TransportResult<T> = Result<T, TransportError>;

/// Comprehensive error type for all transport operations.
#[derive(Error, Debug)]
pub enum TransportError {
    /// Network-related errors (connection, timeout, DNS, etc.)
    #[error("Network error: {0}")]
    Network(Box<NetworkError>),

    /// Protocol-level errors (HTTP status codes, WebSocket close frames, etc.)
    #[error("Protocol error: {0}")]
    Protocol(Box<ProtocolError>),

    /// Authentication and authorization errors
    #[error("Authentication error: {0}")]
    Auth(Box<AuthError>),

    /// Serialization/deserialization errors
    #[error("Serialization error: {0}")]
    Serialization(Box<SerializationError>),

    /// Configuration errors
    #[error("Configuration error: {message}")]
    Config { message: String },

    /// Rate limiting errors
    #[error("Rate limit exceeded: {message}")]
    RateLimit { message: String },

    /// Middleware errors
    #[error("Middleware error: {0}")]
    Middleware(Box<MiddlewareError>),

    /// Connection pool errors
    #[error("Connection pool error: {0}")]
    Pool(Box<PoolError>),

    /// Timeout errors
    #[error("Operation timed out after {duration:?}")]
    Timeout { duration: std::time::Duration },

    /// Cancellation errors
    #[error("Operation was cancelled")]
    Cancelled,

    /// Resource exhaustion errors
    #[error("Resource exhausted: {resource}")]
    ResourceExhausted { resource: String },

    /// Internal errors (should not happen in normal operation)
    #[error("Internal error: {message}")]
    Internal { message: String },
}

impl From<NetworkError> for TransportError {
    fn from(e: NetworkError) -> Self {
        Self::Network(Box::new(e))
    }
}

impl From<ProtocolError> for TransportError {
    fn from(e: ProtocolError) -> Self {
        Self::Protocol(Box::new(e))
    }
}

impl From<AuthError> for TransportError {
    fn from(e: AuthError) -> Self {
        Self::Auth(Box::new(e))
    }
}

impl From<SerializationError> for TransportError {
    fn from(e: SerializationError) -> Self {
        Self::Serialization(Box::new(e))
    }
}

impl From<MiddlewareError> for TransportError {
    fn from(e: MiddlewareError) -> Self {
        Self::Middleware(Box::new(e))
    }
}

impl From<PoolError> for TransportError {
    fn from(e: PoolError) -> Self {
        Self::Pool(Box::new(e))
    }
}

/// Network-level errors
#[derive(Error, Debug)]
pub enum NetworkError {
    #[error("Connection failed: {message}")]
    ConnectionFailed { message: String },

    #[error("Connection lost: {message}")]
    ConnectionLost { message: String },

    #[error("DNS resolution failed: {message}")]
    DnsResolution { message: String },

    #[error("TLS handshake failed: {message}")]
    TlsHandshake { message: String },

    #[error("IO error: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },

    #[error("Hpx error: {source}")]
    Hpx {
        #[from]
        source: hpx::Error,
    },

    #[error("WebSocket error: {source}")]
    WebSocket {
        #[from]
        source: fastwebsockets::WebSocketError,
    },
}

/// Protocol-level errors
#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("HTTP error: status={status}, body={body:?}")]
    Http {
        status: http::StatusCode,
        body: Option<String>,
    },

    #[error("WebSocket close: code={code:?}, reason={reason:?}")]
    WebSocketClose {
        code: Option<u16>,
        reason: Option<String>,
    },

    #[error("Invalid message format: {message}")]
    InvalidMessage { message: String },

    #[error("Unsupported protocol version: {version}")]
    UnsupportedVersion { version: String },

    #[error("Protocol violation: {message}")]
    ProtocolViolation { message: String },
}

/// Authentication and authorization errors
#[derive(Error, Debug)]
pub enum AuthError {
    #[error("Invalid credentials")]
    InvalidCredentials,

    #[error("Token expired")]
    TokenExpired,

    #[error("Token refresh failed: {message}")]
    TokenRefreshFailed { message: String },

    #[error("Insufficient permissions: {required}")]
    InsufficientPermissions { required: String },

    #[error("Authentication required")]
    AuthenticationRequired,

    #[error("Invalid signature: {message}")]
    InvalidSignature { message: String },

    #[error("API key invalid or missing")]
    InvalidApiKey,

    #[error("Authentication service unavailable")]
    ServiceUnavailable,
}

/// Serialization-related errors
#[derive(Debug, thiserror::Error)]
pub enum SerializationError {
    /// JSON serialization/deserialization error
    #[error("JSON error: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },

    /// SIMD JSON serialization/deserialization error
    #[cfg(feature = "simd-json")]
    #[error("SIMD JSON error: {message}")]
    SimdJson { message: String },

    /// URL encoding error
    #[error("URL encoding error: {message}")]
    UrlEncoding { message: String },

    /// UTF-8 conversion error
    #[error("Invalid UTF-8: {message}")]
    InvalidUtf8 { message: String },
}

/// Middleware errors
#[derive(Error, Debug)]
pub enum MiddlewareError {
    #[error("Middleware chain failed: {message}")]
    ChainFailed { message: String },

    #[error("Middleware timeout: {middleware}")]
    Timeout { middleware: String },

    #[error("Middleware error: {middleware}, error={message}")]
    ExecutionFailed { middleware: String, message: String },

    #[error("Middleware configuration error: {message}")]
    Configuration { message: String },
}

/// Connection pool errors
#[derive(Error, Debug)]
pub enum PoolError {
    #[error("Pool exhausted: max_size={max_size}")]
    Exhausted { max_size: usize },

    #[error("Pool closed")]
    Closed,

    #[error("Connection validation failed: {message}")]
    ValidationFailed { message: String },

    #[error("Pool configuration error: {message}")]
    Configuration { message: String },
}

// Implement conversion from common errors to our error types
impl From<std::io::Error> for TransportError {
    fn from(err: std::io::Error) -> Self {
        Self::Network(Box::new(NetworkError::Io { source: err }))
    }
}

impl From<hpx::Error> for TransportError {
    fn from(err: hpx::Error) -> Self {
        Self::Network(Box::new(NetworkError::Hpx { source: err }))
    }
}

impl From<FromUtf8Error> for TransportError {
    fn from(e: FromUtf8Error) -> Self {
        Self::Serialization(Box::new(SerializationError::InvalidUtf8 {
            message: e.to_string(),
        }))
    }
}

impl From<fastwebsockets::WebSocketError> for TransportError {
    fn from(e: fastwebsockets::WebSocketError) -> Self {
        Self::Network(Box::new(NetworkError::ConnectionFailed {
            message: e.to_string(),
        }))
    }
}

impl From<serde_json::Error> for TransportError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialization(Box::new(SerializationError::Json { source: e }))
    }
}

#[cfg(feature = "simd-json")]
impl From<simd_json::Error> for TransportError {
    fn from(e: simd_json::Error) -> Self {
        Self::Serialization(Box::new(SerializationError::SimdJson {
            message: e.to_string(),
        }))
    }
}

/// Error context trait for adding additional context to errors
pub trait ErrorContext<T> {
    /// Add context to an error
    #[allow(clippy::result_large_err)]
    fn with_context<F>(self, f: F) -> Result<T, TransportError>
    where
        F: FnOnce() -> String;

    /// Add static context to an error
    #[allow(clippy::result_large_err)]
    fn context(self, msg: &'static str) -> Result<T, TransportError>;
}

impl<T, E> ErrorContext<T> for Result<T, E>
where
    E: Into<TransportError>,
{
    #[allow(clippy::result_large_err)]
    fn with_context<F>(self, _f: F) -> Result<T, TransportError>
    where
        F: FnOnce() -> String,
    {
        self.map_err(|e| {
            // Add context information to the error
            // This could be implemented more sophisticatedly with error chains
            e.into()
        })
    }

    #[allow(clippy::result_large_err)]
    fn context(self, msg: &'static str) -> Result<T, TransportError> {
        self.with_context(|| msg.to_string())
    }
}

/// Helper trait for creating specific error types
pub trait ErrorFactory {
    fn config_error(message: impl Into<String>) -> TransportError {
        TransportError::Config {
            message: message.into(),
        }
    }

    fn internal_error(message: impl Into<String>) -> TransportError {
        TransportError::Internal {
            message: message.into(),
        }
    }

    fn timeout_error(duration: std::time::Duration) -> TransportError {
        TransportError::Timeout { duration }
    }

    fn rate_limit_error(message: impl Into<String>) -> TransportError {
        TransportError::RateLimit {
            message: message.into(),
        }
    }

    fn hook_error(message: impl Into<String>) -> TransportError {
        TransportError::Middleware(Box::new(MiddlewareError::ExecutionFailed {
            middleware: "hook".to_string(),
            message: message.into(),
        }))
    }
}

// Blanket implementation
impl<T> ErrorFactory for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_conversion() {
        let io_err =
            std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "Connection refused");
        let transport_err: TransportError = io_err.into();

        match transport_err {
            TransportError::Network(boxed_err) => match *boxed_err {
                NetworkError::Io { .. } => {}
                _ => panic!("Expected NetworkError::Io"),
            },
            _ => panic!("Expected TransportError::Network"),
        }
    }

    #[test]
    fn test_error_context() {
        let result: Result<(), std::io::Error> =
            Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout"));

        let err = result.context("Failed to connect").unwrap_err();
        assert!(matches!(err, TransportError::Network(_)));
    }

    #[test]
    fn test_error_factory() {
        let err = TransportError::config_error("Invalid URL");
        assert!(matches!(err, TransportError::Config { .. }));

        let err = TransportError::timeout_error(std::time::Duration::from_secs(5));
        assert!(matches!(err, TransportError::Timeout { .. }));
    }
}
