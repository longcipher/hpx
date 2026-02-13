//! SSE connection configuration.

use std::time::Duration;

/// Configuration for SSE connections.
///
/// Follows the same builder pattern as
/// [`WsConfig`](crate::websocket::WsConfig), providing sensible defaults and
/// chainable setter methods.
#[derive(Clone, Debug)]
pub struct SseConfig {
    /// SSE endpoint URL.
    pub url: String,
    /// HTTP method (usually GET, some APIs use POST).
    pub method: http::Method,
    /// Additional HTTP headers to include with every SSE request.
    pub headers: http::HeaderMap,
    /// Optional request body (for POST-based SSE).
    pub body: Option<Vec<u8>>,
    /// Connection timeout.
    pub connect_timeout: Duration,
    /// Initial delay before the first reconnection attempt.
    pub reconnect_initial_delay: Duration,
    /// Maximum delay between reconnection attempts.
    pub reconnect_max_delay: Duration,
    /// Backoff multiplier for reconnection delays.
    pub reconnect_backoff_factor: f64,
    /// Maximum number of reconnection attempts (None = infinite).
    pub reconnect_max_attempts: Option<u32>,
    /// Random jitter factor (0.0–1.0) for reconnection delays.
    pub reconnect_jitter: f64,
    /// Whether to send auth headers on connect/reconnect.
    pub auth_on_connect: bool,
    /// Capacity of the event channel.
    pub event_channel_capacity: usize,
    /// Capacity of the command channel.
    pub command_channel_capacity: usize,
}

impl Default for SseConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            method: http::Method::GET,
            headers: http::HeaderMap::new(),
            body: None,
            connect_timeout: Duration::from_secs(10),
            reconnect_initial_delay: Duration::from_secs(1),
            reconnect_max_delay: Duration::from_secs(60),
            reconnect_backoff_factor: 2.0,
            reconnect_max_attempts: None,
            reconnect_jitter: 0.1,
            auth_on_connect: false,
            event_channel_capacity: 256,
            command_channel_capacity: 64,
        }
    }
}

impl SseConfig {
    /// Create a new SSE configuration with the given URL.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Default::default()
        }
    }

    /// Set the HTTP method (e.g., `POST` for POST-based SSE).
    #[must_use]
    pub fn method(mut self, method: http::Method) -> Self {
        self.method = method;
        self
    }

    /// Set additional HTTP headers.
    #[must_use]
    pub fn headers(mut self, headers: http::HeaderMap) -> Self {
        self.headers = headers;
        self
    }

    /// Set the request body (for POST-based SSE).
    #[must_use]
    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = Some(body);
        self
    }

    /// Set the connection timeout.
    #[must_use]
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Set the initial reconnection delay.
    #[must_use]
    pub fn reconnect_initial_delay(mut self, delay: Duration) -> Self {
        self.reconnect_initial_delay = delay;
        self
    }

    /// Set the maximum reconnection delay.
    #[must_use]
    pub fn reconnect_max_delay(mut self, delay: Duration) -> Self {
        self.reconnect_max_delay = delay;
        self
    }

    /// Set the reconnection backoff factor.
    #[must_use]
    pub fn reconnect_backoff_factor(mut self, factor: f64) -> Self {
        self.reconnect_backoff_factor = factor;
        self
    }

    /// Set the maximum reconnection attempts.
    #[must_use]
    pub fn reconnect_max_attempts(mut self, attempts: Option<u32>) -> Self {
        self.reconnect_max_attempts = attempts;
        self
    }

    /// Set the reconnection jitter factor.
    #[must_use]
    pub fn reconnect_jitter(mut self, jitter: f64) -> Self {
        self.reconnect_jitter = jitter;
        self
    }

    /// Set whether to authenticate on connect.
    #[must_use]
    pub fn auth_on_connect(mut self, auth: bool) -> Self {
        self.auth_on_connect = auth;
        self
    }

    /// Set the event channel capacity.
    #[must_use]
    pub fn event_channel_capacity(mut self, capacity: usize) -> Self {
        self.event_channel_capacity = capacity;
        self
    }

    /// Set the command channel capacity.
    #[must_use]
    pub fn command_channel_capacity(mut self, capacity: usize) -> Self {
        self.command_channel_capacity = capacity;
        self
    }

    /// Validate the configuration.
    ///
    /// # Errors
    ///
    /// Returns an error message string if any field has an invalid value.
    pub fn validate(&self) -> Result<(), String> {
        if self.url.is_empty() {
            return Err("URL cannot be empty".to_string());
        }
        if self.reconnect_backoff_factor < 1.0 {
            return Err("Backoff factor must be >= 1.0".to_string());
        }
        if !(0.0..=1.0).contains(&self.reconnect_jitter) {
            return Err("Jitter must be between 0.0 and 1.0".to_string());
        }
        if self.event_channel_capacity == 0 {
            return Err("Event channel capacity must be > 0".to_string());
        }
        if self.command_channel_capacity == 0 {
            return Err("Command channel capacity must be > 0".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = SseConfig::default();
        assert!(config.url.is_empty());
        assert_eq!(config.method, http::Method::GET);
        assert!(config.headers.is_empty());
        assert!(config.body.is_none());
        assert_eq!(config.connect_timeout, Duration::from_secs(10));
        assert_eq!(config.reconnect_initial_delay, Duration::from_secs(1));
        assert_eq!(config.reconnect_max_delay, Duration::from_secs(60));
        assert_eq!(config.reconnect_backoff_factor, 2.0);
        assert!(config.reconnect_max_attempts.is_none());
        assert_eq!(config.reconnect_jitter, 0.1);
        assert!(!config.auth_on_connect);
        assert_eq!(config.event_channel_capacity, 256);
        assert_eq!(config.command_channel_capacity, 64);
    }

    #[test]
    fn test_new_sets_url() {
        let config = SseConfig::new("https://api.example.com/stream");
        assert_eq!(config.url, "https://api.example.com/stream");
        // Other fields should be defaults
        assert_eq!(config.method, http::Method::GET);
        assert_eq!(config.reconnect_backoff_factor, 2.0);
    }

    #[test]
    fn test_builder_pattern() {
        let config = SseConfig::new("https://api.example.com/stream")
            .method(http::Method::POST)
            .body(b"subscribe".to_vec())
            .connect_timeout(Duration::from_secs(30))
            .reconnect_max_attempts(Some(5))
            .auth_on_connect(true);

        assert_eq!(config.url, "https://api.example.com/stream");
        assert_eq!(config.method, http::Method::POST);
        assert_eq!(config.body.as_deref(), Some(b"subscribe".as_slice()));
        assert_eq!(config.connect_timeout, Duration::from_secs(30));
        assert_eq!(config.reconnect_max_attempts, Some(5));
        assert!(config.auth_on_connect);
    }

    #[test]
    fn test_all_builder_methods() {
        let mut headers = http::HeaderMap::new();
        headers.insert("X-Api-Key", "test-key".parse().expect("valid header value"));

        let config = SseConfig::new("https://api.exchange.com/v1/stream")
            .method(http::Method::POST)
            .headers(headers)
            .body(b"{\"channels\":[\"trades\"]}".to_vec())
            .connect_timeout(Duration::from_secs(15))
            .reconnect_initial_delay(Duration::from_millis(500))
            .reconnect_max_delay(Duration::from_secs(120))
            .reconnect_backoff_factor(1.5)
            .reconnect_max_attempts(Some(10))
            .reconnect_jitter(0.2)
            .auth_on_connect(true)
            .event_channel_capacity(512)
            .command_channel_capacity(128);

        assert_eq!(config.url, "https://api.exchange.com/v1/stream");
        assert_eq!(config.method, http::Method::POST);
        assert_eq!(
            config
                .headers
                .get("X-Api-Key")
                .map(|v| v.to_str().expect("valid str")),
            Some("test-key")
        );
        assert_eq!(
            config.body.as_deref(),
            Some(b"{\"channels\":[\"trades\"]}".as_slice())
        );
        assert_eq!(config.connect_timeout, Duration::from_secs(15));
        assert_eq!(config.reconnect_initial_delay, Duration::from_millis(500));
        assert_eq!(config.reconnect_max_delay, Duration::from_secs(120));
        assert_eq!(config.reconnect_backoff_factor, 1.5);
        assert_eq!(config.reconnect_max_attempts, Some(10));
        assert_eq!(config.reconnect_jitter, 0.2);
        assert!(config.auth_on_connect);
        assert_eq!(config.event_channel_capacity, 512);
        assert_eq!(config.command_channel_capacity, 128);
    }

    #[test]
    fn test_validation_empty_url() {
        let config = SseConfig::default();
        let result = config.validate();
        assert!(result.is_err());
        assert_eq!(result.expect_err("should fail"), "URL cannot be empty");
    }

    #[test]
    fn test_validation_invalid_backoff() {
        let config = SseConfig::new("https://example.com").reconnect_backoff_factor(0.5);
        let result = config.validate();
        assert!(result.is_err());
        assert_eq!(
            result.expect_err("should fail"),
            "Backoff factor must be >= 1.0"
        );
    }

    #[test]
    fn test_validation_invalid_jitter_high() {
        let config = SseConfig::new("https://example.com").reconnect_jitter(1.5);
        let result = config.validate();
        assert!(result.is_err());
        assert_eq!(
            result.expect_err("should fail"),
            "Jitter must be between 0.0 and 1.0"
        );
    }

    #[test]
    fn test_validation_invalid_jitter_negative() {
        let config = SseConfig::new("https://example.com").reconnect_jitter(-0.1);
        let result = config.validate();
        assert!(result.is_err());
        assert_eq!(
            result.expect_err("should fail"),
            "Jitter must be between 0.0 and 1.0"
        );
    }

    #[test]
    fn test_validation_zero_event_channel() {
        let config = SseConfig::new("https://example.com").event_channel_capacity(0);
        let result = config.validate();
        assert!(result.is_err());
        assert_eq!(
            result.expect_err("should fail"),
            "Event channel capacity must be > 0"
        );
    }

    #[test]
    fn test_validation_zero_command_channel() {
        let config = SseConfig::new("https://example.com").command_channel_capacity(0);
        let result = config.validate();
        assert!(result.is_err());
        assert_eq!(
            result.expect_err("should fail"),
            "Command channel capacity must be > 0"
        );
    }

    #[test]
    fn test_validation_valid_config() {
        let config = SseConfig::new("https://example.com");
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validation_boundary_jitter() {
        // Jitter at exact boundaries (0.0 and 1.0) should be valid
        let config_zero = SseConfig::new("https://example.com").reconnect_jitter(0.0);
        assert!(config_zero.validate().is_ok());

        let config_one = SseConfig::new("https://example.com").reconnect_jitter(1.0);
        assert!(config_one.validate().is_ok());
    }

    #[test]
    fn test_validation_boundary_backoff() {
        // Backoff factor at exact boundary (1.0) should be valid
        let config = SseConfig::new("https://example.com").reconnect_backoff_factor(1.0);
        assert!(config.validate().is_ok());
    }
}
