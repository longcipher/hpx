//! Typed client wrapper providing ergonomic API patterns.
//!
//! This module provides typed response wrappers and API error handling similar to
//! Python httpx and Node.js Ky patterns.
//!
//! # Example
//!
//! ```rust,ignore
//! use hpx_transport::typed::{TypedResponse, ApiError};
//!
//! #[derive(Deserialize)]
//! struct User {
//!     id: u64,
//!     name: String,
//! }
//!
//! let response: TypedResponse<User> = client.fetch_typed("/users/1").await?;
//! println!("User: {}, Latency: {:?}", response.data.name, response.latency);
//! ```

use std::{collections::HashMap, fmt, time::Duration};

use bytes::Bytes;
use http::StatusCode;
use serde::{Deserialize, de::DeserializeOwned};

use crate::error::TransportError;

/// A typed response wrapper that includes both the deserialized data and metadata.
#[derive(Debug, Clone)]
pub struct TypedResponse<T> {
    /// The deserialized response data.
    pub data: T,
    /// HTTP status code of the response.
    pub status: StatusCode,
    /// Response headers.
    pub headers: HashMap<String, String>,
    /// Round-trip latency.
    pub latency: Duration,
    /// Raw body bytes (if preserved).
    raw_body: Option<Bytes>,
}

impl<T> TypedResponse<T> {
    /// Create a new typed response.
    pub fn new(
        data: T,
        status: StatusCode,
        headers: HashMap<String, String>,
        latency: Duration,
    ) -> Self {
        Self {
            data,
            status,
            headers,
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

    /// Get a header value by name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(|s| s.as_str())
    }

    /// Map the response data to a new type.
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> TypedResponse<U> {
        TypedResponse {
            data: f(self.data),
            status: self.status,
            headers: self.headers,
            latency: self.latency,
            raw_body: self.raw_body,
        }
    }
}

/// Trait for API error types that can be deserialized from error responses.
///
/// Implement this trait for your API's error type to enable automatic error parsing.
///
/// # Example
///
/// ```rust,ignore
/// #[derive(Deserialize, Debug)]
/// struct MyApiError {
///     code: String,
///     message: String,
/// }
///
/// impl ApiError for MyApiError {
///     fn from_response(status: StatusCode, body: &[u8]) -> Option<Self> {
///         serde_json::from_slice(body).ok()
///     }
///     
///     fn error_message(&self) -> &str {
///         &self.message
///     }
/// }
/// ```
pub trait ApiError: Sized + fmt::Debug {
    /// Try to parse an error from the response status and body.
    fn from_response(status: StatusCode, body: &[u8]) -> Option<Self>;

    /// Get the error message for display.
    fn error_message(&self) -> &str;

    /// Get the error code if available.
    fn error_code(&self) -> Option<&str> {
        None
    }
}

/// A generic API error that can deserialize common error response formats.
#[derive(Debug, Clone, Deserialize)]
pub struct GenericApiError {
    /// Error message.
    #[serde(alias = "msg", alias = "error", alias = "detail")]
    pub message: String,
    /// Error code.
    #[serde(alias = "code", alias = "error_code")]
    pub code: Option<String>,
    /// HTTP status code.
    #[serde(skip)]
    pub status: Option<StatusCode>,
}

impl ApiError for GenericApiError {
    fn from_response(status: StatusCode, body: &[u8]) -> Option<Self> {
        let mut error: GenericApiError = serde_json::from_slice(body).ok()?;
        error.status = Some(status);
        Some(error)
    }

    fn error_message(&self) -> &str {
        &self.message
    }

    fn error_code(&self) -> Option<&str> {
        self.code.as_deref()
    }
}

impl fmt::Display for GenericApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(code) = &self.code {
            write!(f, "[{}] {}", code, self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for GenericApiError {}

/// Error type for typed API operations.
#[derive(Debug, thiserror::Error)]
pub enum TypedApiError<E: ApiError> {
    /// Transport-level error (network, timeout, etc.)
    #[error("Transport error: {0}")]
    Transport(#[from] TransportError),

    /// API returned an error response that was successfully parsed.
    #[error("API error (status {status}): {error:?}")]
    Api {
        /// HTTP status code.
        status: StatusCode,
        /// Parsed error response.
        error: E,
    },

    /// API returned an error response that couldn't be parsed.
    #[error("HTTP error (status {status}): {body}")]
    Http {
        /// HTTP status code.
        status: StatusCode,
        /// Raw body as string.
        body: String,
    },

    /// JSON deserialization error.
    #[error("Deserialization error: {message}")]
    Deserialization {
        /// Error message.
        message: String,
        /// Raw body for debugging.
        body: Option<String>,
    },
}

impl<E: ApiError> TypedApiError<E> {
    /// Create an API error from status and parsed error.
    pub fn api_error(status: StatusCode, error: E) -> Self {
        Self::Api { status, error }
    }

    /// Create an HTTP error from status and body.
    pub fn http_error(status: StatusCode, body: impl Into<String>) -> Self {
        Self::Http {
            status,
            body: body.into(),
        }
    }

    /// Create a deserialization error.
    pub fn deserialization_error(message: impl Into<String>, body: Option<String>) -> Self {
        Self::Deserialization {
            message: message.into(),
            body,
        }
    }

    /// Check if this is an API error (4xx/5xx with parseable body).
    pub fn is_api_error(&self) -> bool {
        matches!(self, Self::Api { .. })
    }

    /// Check if this is a transport error.
    pub fn is_transport_error(&self) -> bool {
        matches!(self, Self::Transport(_))
    }

    /// Get the HTTP status code if available.
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            Self::Api { status, .. } => Some(*status),
            Self::Http { status, .. } => Some(*status),
            _ => None,
        }
    }
}

/// Result type for typed API operations.
pub type TypedResult<T, E> = Result<T, TypedApiError<E>>;

/// Extension trait for parsing typed responses.
pub trait TypedResponseExt {
    /// Parse the response as a typed value, with custom error type.
    fn typed<T, E>(self) -> TypedResult<TypedResponse<T>, E>
    where
        T: DeserializeOwned,
        E: ApiError;

    /// Parse the response as a typed value, using generic error type.
    fn typed_generic<T>(self) -> TypedResult<TypedResponse<T>, GenericApiError>
    where
        T: DeserializeOwned;
}

impl TypedResponseExt for crate::transport::Response {
    fn typed<T, E>(self) -> TypedResult<TypedResponse<T>, E>
    where
        T: DeserializeOwned,
        E: ApiError,
    {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

        // Check for error responses
        if !status.is_success() {
            // Try to parse as API error
            if let Some(api_error) = E::from_response(status, &self.body) {
                return Err(TypedApiError::api_error(status, api_error));
            }

            // Fall back to HTTP error
            let body_str = String::from_utf8_lossy(&self.body).to_string();
            return Err(TypedApiError::http_error(status, body_str));
        }

        // Parse successful response
        #[cfg(not(feature = "simd-json"))]
        let data: T = serde_json::from_slice(&self.body).map_err(|e| {
            TypedApiError::deserialization_error(
                e.to_string(),
                Some(String::from_utf8_lossy(&self.body).to_string()),
            )
        })?;

        #[cfg(feature = "simd-json")]
        let data: T = {
            let mut bytes_vec = self.body.to_vec();
            simd_json::from_slice(&mut bytes_vec).map_err(|e| {
                TypedApiError::deserialization_error(
                    e.to_string(),
                    Some(String::from_utf8_lossy(&self.body).to_string()),
                )
            })?
        };

        Ok(TypedResponse::new(data, status, self.headers, self.duration).with_raw_body(self.body))
    }

    fn typed_generic<T>(self) -> TypedResult<TypedResponse<T>, GenericApiError>
    where
        T: DeserializeOwned,
    {
        self.typed()
    }
}

/// Configuration for per-request retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    pub max_attempts: usize,
    /// Delay between retries.
    pub delay: Duration,
    /// Whether to use exponential backoff.
    pub exponential_backoff: bool,
    /// Maximum delay cap for exponential backoff.
    pub max_delay: Option<Duration>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            delay: Duration::from_millis(500),
            exponential_backoff: true,
            max_delay: Some(Duration::from_secs(30)),
        }
    }
}

impl RetryConfig {
    /// Create a new retry config with the given max attempts.
    pub fn new(max_attempts: usize) -> Self {
        Self {
            max_attempts,
            ..Default::default()
        }
    }

    /// Disable retries.
    pub fn none() -> Self {
        Self {
            max_attempts: 0,
            ..Default::default()
        }
    }

    /// Set the delay between retries.
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    /// Enable exponential backoff with optional max delay.
    pub fn with_exponential_backoff(mut self, max_delay: Option<Duration>) -> Self {
        self.exponential_backoff = true;
        self.max_delay = max_delay;
        self
    }

    /// Use constant delay (no backoff).
    pub fn with_constant_delay(mut self) -> Self {
        self.exponential_backoff = false;
        self
    }
}

/// Builder for typed request configuration.
pub struct TypedRequestBuilder<'a, E: ApiError> {
    /// Request timeout override.
    pub timeout: Option<Duration>,
    /// Retry count override.
    pub retries: Option<usize>,
    /// Full retry configuration.
    pub retry_config: Option<RetryConfig>,
    /// Custom headers for this request.
    pub headers: HashMap<String, String>,
    /// Phantom data for error type.
    _error: std::marker::PhantomData<&'a E>,
}

impl<'a, E: ApiError> Default for TypedRequestBuilder<'a, E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, E: ApiError> TypedRequestBuilder<'a, E> {
    /// Create a new typed request builder.
    pub fn new() -> Self {
        Self {
            timeout: None,
            retries: None,
            retry_config: None,
            headers: HashMap::new(),
            _error: std::marker::PhantomData,
        }
    }

    /// Set the request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set the retry count (simple configuration).
    ///
    /// For more control over retry behavior, use [`retry()`](Self::retry).
    pub fn retries(mut self, retries: usize) -> Self {
        self.retries = Some(retries);
        self
    }

    /// Set the full retry configuration.
    ///
    /// This provides more control than [`retries()`](Self::retries).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use hpx_transport::typed::RetryConfig;
    /// use std::time::Duration;
    ///
    /// let request = client
    ///     .get("/api/data")
    ///     .retry(RetryConfig::new(5)
    ///         .with_delay(Duration::from_secs(1))
    ///         .with_exponential_backoff(Some(Duration::from_secs(30))))
    ///     .fetch_json()
    ///     .await?;
    /// ```
    pub fn retry(mut self, config: RetryConfig) -> Self {
        self.retry_config = Some(config);
        self
    }

    /// Disable retries for this request.
    pub fn no_retry(mut self) -> Self {
        self.retry_config = Some(RetryConfig::none());
        self
    }

    /// Add a header to the request.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    /// Add a bearer auth token.
    pub fn bearer_auth(self, token: impl Into<String>) -> Self {
        self.header("Authorization", format!("Bearer {}", token.into()))
    }

    /// Add basic auth credentials.
    pub fn basic_auth(self, username: impl AsRef<str>, password: Option<impl AsRef<str>>) -> Self {
        use base64::Engine;
        let credentials = match password {
            Some(pass) => format!("{}:{}", username.as_ref(), pass.as_ref()),
            None => format!("{}:", username.as_ref()),
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
        self.header("Authorization", format!("Basic {}", encoded))
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_typed_response() {
        let response: TypedResponse<String> = TypedResponse::new(
            "test".to_string(),
            StatusCode::OK,
            HashMap::new(),
            Duration::from_millis(100),
        );

        assert!(response.is_success());
        assert_eq!(response.data, "test");
        assert_eq!(response.latency, Duration::from_millis(100));
    }

    #[test]
    fn test_typed_response_map() {
        let response: TypedResponse<i32> = TypedResponse::new(
            42,
            StatusCode::OK,
            HashMap::new(),
            Duration::from_millis(100),
        );

        let mapped = response.map(|n| n.to_string());
        assert_eq!(mapped.data, "42");
    }

    #[test]
    fn test_generic_api_error_deserialize() {
        let json = r#"{"message": "Not found", "code": "NOT_FOUND"}"#;
        let error: GenericApiError = serde_json::from_str(json).unwrap();

        assert_eq!(error.message, "Not found");
        assert_eq!(error.code, Some("NOT_FOUND".to_string()));
    }

    #[test]
    fn test_generic_api_error_alias() {
        // Test with 'error' alias
        let json = r#"{"error": "Something went wrong"}"#;
        let error: GenericApiError = serde_json::from_str(json).unwrap();
        assert_eq!(error.message, "Something went wrong");

        // Test with 'msg' alias
        let json = r#"{"msg": "Another error"}"#;
        let error: GenericApiError = serde_json::from_str(json).unwrap();
        assert_eq!(error.message, "Another error");
    }
}
