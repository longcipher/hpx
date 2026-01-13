//! Core transport abstractions and types.

use std::{
    collections::HashMap,
    fmt,
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::TransportResult;

/// Unique identifier for requests.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestId(Uuid);

impl RequestId {
    /// Generate a new random request ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a request ID from a UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the inner UUID.
    pub fn as_uuid(&self) -> &Uuid {
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

impl From<Uuid> for RequestId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl From<RequestId> for Uuid {
    fn from(id: RequestId) -> Self {
        id.0
    }
}

/// HTTP method enumeration.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
    Connect,
    Trace,
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Method::Get => write!(f, "GET"),
            Method::Post => write!(f, "POST"),
            Method::Put => write!(f, "PUT"),
            Method::Delete => write!(f, "DELETE"),
            Method::Patch => write!(f, "PATCH"),
            Method::Head => write!(f, "HEAD"),
            Method::Options => write!(f, "OPTIONS"),
            Method::Connect => write!(f, "CONNECT"),
            Method::Trace => write!(f, "TRACE"),
        }
    }
}

impl From<http::Method> for Method {
    fn from(method: http::Method) -> Self {
        match method {
            http::Method::GET => Method::Get,
            http::Method::POST => Method::Post,
            http::Method::PUT => Method::Put,
            http::Method::DELETE => Method::Delete,
            http::Method::PATCH => Method::Patch,
            http::Method::HEAD => Method::Head,
            http::Method::OPTIONS => Method::Options,
            http::Method::CONNECT => Method::Connect,
            http::Method::TRACE => Method::Trace,
            _ => Method::Get, // Default fallback
        }
    }
}

impl From<Method> for http::Method {
    fn from(method: Method) -> Self {
        match method {
            Method::Get => http::Method::GET,
            Method::Post => http::Method::POST,
            Method::Put => http::Method::PUT,
            Method::Delete => http::Method::DELETE,
            Method::Patch => http::Method::PATCH,
            Method::Head => http::Method::HEAD,
            Method::Options => http::Method::OPTIONS,
            Method::Connect => http::Method::CONNECT,
            Method::Trace => http::Method::TRACE,
        }
    }
}

/// Generic request type that can be used across different transport protocols.
#[derive(Debug, Clone)]
pub struct Request {
    /// Unique identifier for this request
    pub id: RequestId,

    /// HTTP method
    pub method: Method,

    /// Request URL or path
    pub url: String,

    /// Request headers
    pub headers: HashMap<String, String>,

    /// Query parameters
    pub query: Vec<(String, String)>,

    /// Request body
    pub body: Option<Bytes>,

    /// Request timeout
    pub timeout: Option<Duration>,

    /// Request creation timestamp
    pub created_at: SystemTime,

    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

impl Request {
    /// Create a new request.
    pub fn new(method: Method, url: impl Into<String>) -> Self {
        Self {
            id: RequestId::new(),
            method,
            url: url.into(),
            headers: HashMap::new(),
            query: Vec::new(),
            body: None,
            timeout: None,
            created_at: SystemTime::now(),
            metadata: HashMap::new(),
        }
    }

    /// Create a GET request.
    pub fn get(url: impl Into<String>) -> Self {
        Self::new(Method::Get, url)
    }

    /// Create a POST request.
    pub fn post(url: impl Into<String>) -> Self {
        Self::new(Method::Post, url)
    }

    /// Create a PUT request.
    pub fn put(url: impl Into<String>) -> Self {
        Self::new(Method::Put, url)
    }

    /// Create a DELETE request.
    pub fn delete(url: impl Into<String>) -> Self {
        Self::new(Method::Delete, url)
    }

    /// Add a header to the request.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    /// Add multiple headers to the request.
    pub fn headers<I, K, V>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (name, value) in headers {
            self.headers.insert(name.into(), value.into());
        }
        self
    }

    /// Add a query parameter to the request.
    pub fn query(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.query.push((name.into(), value.into()));
        self
    }

    /// Add multiple query parameters to the request.
    pub fn queries<I, K, V>(mut self, queries: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (name, value) in queries {
            self.query.push((name.into(), value.into()));
        }
        self
    }

    /// Set the request body.
    pub fn body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// Set the request body as JSON.
    #[allow(clippy::result_large_err)]
    #[allow(clippy::result_large_err)]
    pub fn json<T: Serialize>(mut self, data: &T) -> TransportResult<Self> {
        let json_str = serde_json::to_string(data)?;
        self.body = Some(json_str.into());
        self.headers
            .insert("Content-Type".to_string(), "application/json".to_string());
        Ok(self)
    }

    /// Set the request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Add metadata to the request.
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Get the age of this request.
    pub fn age(&self) -> Duration {
        self.created_at.elapsed().unwrap_or(Duration::ZERO)
    }

    /// Check if the request has timed out.
    pub fn is_timed_out(&self) -> bool {
        if let Some(timeout) = self.timeout {
            self.age() > timeout
        } else {
            false
        }
    }
}

/// Generic response type.
#[derive(Debug, Clone)]
pub struct Response {
    /// Request ID this response corresponds to
    pub request_id: RequestId,

    /// Response status code
    pub status: u16,

    /// Response headers
    pub headers: HashMap<String, String>,

    /// Response body
    pub body: Bytes,

    /// Response timestamp
    pub received_at: SystemTime,

    /// Round-trip time
    pub duration: Duration,

    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

impl Response {
    /// Create a new response.
    pub fn new(request_id: RequestId, status: u16, body: impl Into<Bytes>) -> Self {
        Self {
            request_id,
            status,
            headers: HashMap::new(),
            body: body.into(),
            received_at: SystemTime::now(),
            duration: Duration::ZERO,
            metadata: HashMap::new(),
        }
    }

    /// Check if the response indicates success.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Check if the response indicates a client error.
    pub fn is_client_error(&self) -> bool {
        (400..500).contains(&self.status)
    }

    /// Check if the response indicates a server error.
    pub fn is_server_error(&self) -> bool {
        (500..600).contains(&self.status)
    }

    /// Parse the response body as JSON.
    #[allow(clippy::result_large_err)]
    #[allow(clippy::result_large_err)]
    pub fn json<T: for<'de> Deserialize<'de>>(&self) -> TransportResult<T> {
        Ok(serde_json::from_slice(&self.body)?)
    }

    /// Get the response body as text.
    #[allow(clippy::result_large_err)]
    #[allow(clippy::result_large_err)]
    pub fn text(&self) -> TransportResult<String> {
        Ok(String::from_utf8(self.body.to_vec())?)
    }

    /// Get a header value.
    pub fn header(&self, name: &str) -> Option<&String> {
        self.headers.get(name)
    }

    /// Set the round-trip duration.
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Add metadata to the response.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

/// Core transport trait that defines the interface for sending requests.
#[async_trait]
pub trait Transport: Send + Sync + 'static {
    /// The error type for this transport.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Send a request and return a response.
    async fn send(&self, request: Request) -> Result<Response, Self::Error>;

    /// Check if the transport is healthy and ready to send requests.
    async fn health_check(&self) -> Result<(), Self::Error> {
        // Default implementation: try to send a simple request
        let request = Request::get("https://httpbin.org/get").timeout(Duration::from_secs(5));

        match self.send(request).await {
            Ok(response) => {
                if response.is_success() {
                    Ok(())
                } else {
                    Err(self.error_from_status(response.status))
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Create an error from an HTTP status code.
    fn error_from_status(&self, status: u16) -> Self::Error;

    /// Get transport-specific metrics.
    fn metrics(&self) -> TransportMetrics {
        TransportMetrics::default()
    }
}

/// Transport metrics for monitoring and observability.
#[derive(Debug, Clone, Default)]
pub struct TransportMetrics {
    /// Total number of requests sent
    pub requests_sent: u64,

    /// Total number of successful responses
    pub requests_successful: u64,

    /// Total number of failed requests
    pub requests_failed: u64,

    /// Average response time
    pub avg_response_time: Duration,

    /// Number of active connections
    pub active_connections: usize,

    /// Connection pool utilization
    pub pool_utilization: f64,

    /// Last error timestamp
    pub last_error_at: Option<SystemTime>,
}

impl TransportMetrics {
    /// Calculate the success rate.
    pub fn success_rate(&self) -> f64 {
        if self.requests_sent == 0 {
            0.0
        } else {
            self.requests_successful as f64 / self.requests_sent as f64
        }
    }

    /// Calculate the error rate.
    pub fn error_rate(&self) -> f64 {
        if self.requests_sent == 0 {
            0.0
        } else {
            self.requests_failed as f64 / self.requests_sent as f64
        }
    }
}

/// Request context that can be passed through middleware layers.
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// Request ID
    pub request_id: RequestId,

    /// Trace ID for distributed tracing
    pub trace_id: Option<String>,

    /// Span ID for distributed tracing
    pub span_id: Option<String>,

    /// User ID for authorization
    pub user_id: Option<String>,

    /// Request tags for filtering and routing
    pub tags: HashMap<String, String>,

    /// Request start time
    pub started_at: SystemTime,
}

impl RequestContext {
    /// Create a new request context.
    pub fn new(request_id: RequestId) -> Self {
        Self {
            request_id,
            trace_id: None,
            span_id: None,
            user_id: None,
            tags: HashMap::new(),
            started_at: SystemTime::now(),
        }
    }

    /// Add a tag to the context.
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key.into(), value.into());
        self
    }

    /// Set the trace ID.
    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }

    /// Set the span ID.
    pub fn with_span_id(mut self, span_id: impl Into<String>) -> Self {
        self.span_id = Some(span_id.into());
        self
    }

    /// Set the user ID.
    pub fn with_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    /// Get the elapsed time since request start.
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed().unwrap_or(Duration::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_creation() {
        let request = Request::get("https://api.example.com/test")
            .header("Accept", "application/json")
            .query("limit", "10")
            .timeout(Duration::from_secs(30));

        assert_eq!(request.method, Method::Get);
        assert_eq!(request.url, "https://api.example.com/test");
        assert_eq!(
            request.headers.get("Accept"),
            Some(&"application/json".to_string())
        );
        assert_eq!(
            request
                .query
                .iter()
                .find(|(k, _)| k == "limit")
                .map(|(_, v)| v),
            Some(&"10".to_string())
        );
        assert_eq!(request.timeout, Some(Duration::from_secs(30)));
    }

    #[test]
    fn test_request_json() {
        #[derive(Serialize)]
        struct TestData {
            name: String,
            value: i32,
        }

        let data = TestData {
            name: "test".to_string(),
            value: 42,
        };

        let request = Request::post("https://api.example.com/test")
            .json(&data)
            .unwrap();

        assert!(request.body.is_some());
        assert_eq!(
            request.headers.get("Content-Type"),
            Some(&"application/json".to_string())
        );
    }

    #[test]
    fn test_response_status_checks() {
        let response = Response::new(RequestId::new(), 200, "OK");
        assert!(response.is_success());
        assert!(!response.is_client_error());
        assert!(!response.is_server_error());

        let response = Response::new(RequestId::new(), 404, "Not Found");
        assert!(!response.is_success());
        assert!(response.is_client_error());
        assert!(!response.is_server_error());

        let response = Response::new(RequestId::new(), 500, "Internal Server Error");
        assert!(!response.is_success());
        assert!(!response.is_client_error());
        assert!(response.is_server_error());
    }

    #[test]
    fn test_transport_metrics() {
        let metrics = TransportMetrics {
            requests_sent: 100,
            requests_successful: 95,
            requests_failed: 5,
            ..Default::default()
        };

        assert_eq!(metrics.success_rate(), 0.95);
        assert_eq!(metrics.error_rate(), 0.05);
    }

    #[test]
    fn test_request_context() {
        let request_id = RequestId::new();
        let context = RequestContext::new(request_id.clone())
            .with_trace_id("trace-123")
            .with_span_id("span-456")
            .with_user_id("user-789")
            .with_tag("service", "api");

        assert_eq!(context.request_id, request_id);
        assert_eq!(context.trace_id, Some("trace-123".to_string()));
        assert_eq!(context.span_id, Some("span-456".to_string()));
        assert_eq!(context.user_id, Some("user-789".to_string()));
        assert_eq!(context.tags.get("service"), Some(&"api".to_string()));
    }

    #[test]
    fn test_method_conversion() {
        let http_method = http::Method::POST;
        let method: Method = http_method.into();
        assert_eq!(method, Method::Post);

        let http_method: http::Method = method.into();
        assert_eq!(http_method, http::Method::POST);
    }
}
