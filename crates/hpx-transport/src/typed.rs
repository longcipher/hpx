//! Typed response wrapper for ergonomic API patterns.
//!
//! This module provides typed response wrappers that include both the
//! deserialized data and metadata like latency and status codes.

use std::time::Duration;

use bytes::Bytes;
use http::StatusCode;
use serde::de::DeserializeOwned;

/// A typed response wrapper that includes both the deserialized data and metadata.
#[derive(Debug, Clone)]
pub struct TypedResponse<T> {
    /// The deserialized response data.
    pub data: T,
    /// HTTP status code of the response.
    pub status: StatusCode,
    /// Round-trip latency.
    pub latency: Duration,
    /// Raw body bytes (if preserved).
    raw_body: Option<Bytes>,
}

impl<T> TypedResponse<T> {
    /// Create a new typed response.
    pub fn new(data: T, status: StatusCode, latency: Duration) -> Self {
        Self {
            data,
            status,
            latency,
            raw_body: None,
        }
    }

    /// Create a typed response with raw body preserved.
    pub fn with_raw_body(mut self, body: Bytes) -> Self {
        self.raw_body = Some(body);
        self
    }

    /// Get the raw body if preserved.
    pub fn raw_body(&self) -> Option<&Bytes> {
        self.raw_body.as_ref()
    }

    /// Check if the response indicates success.
    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }

    /// Check if the response is a client error.
    pub fn is_client_error(&self) -> bool {
        self.status.is_client_error()
    }

    /// Check if the response is a server error.
    pub fn is_server_error(&self) -> bool {
        self.status.is_server_error()
    }

    /// Map the response data to a new type.
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> TypedResponse<U> {
        TypedResponse {
            data: f(self.data),
            status: self.status,
            latency: self.latency,
            raw_body: self.raw_body,
        }
    }

    /// Consume the response and return just the data.
    pub fn into_data(self) -> T {
        self.data
    }
}

/// Trait for API error types that can be deserialized from error responses.
pub trait ApiError: DeserializeOwned + std::error::Error + Send + Sync {
    /// Try to parse an error from a response body.
    fn from_response(status: StatusCode, body: &[u8]) -> Option<Self>;
}

/// Result type for typed API responses.
pub type TypedResult<T, E = crate::error::TransportError> = Result<TypedResponse<T>, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_typed_response() {
        let response = TypedResponse::new("hello", StatusCode::OK, Duration::from_millis(100));
        assert!(response.is_success());
        assert_eq!(response.latency.as_millis(), 100);
    }

    #[test]
    fn test_map() {
        let response = TypedResponse::new(
            "hello".to_string(),
            StatusCode::OK,
            Duration::from_millis(100),
        );
        let mapped = response.map(|s| s.len());
        assert_eq!(mapped.data, 5);
    }
}
