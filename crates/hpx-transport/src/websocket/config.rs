//! WebSocket connection configuration.

use std::time::Duration;

use crate::reconnect::BackoffConfig;

/// Configuration for WebSocket connections.
#[derive(Clone, Debug)]
pub struct WsConfig {
    // URL
    /// WebSocket endpoint URL.
    pub url: String,

    // Reconnection settings
    /// Initial delay before first reconnection attempt.
    pub reconnect_initial_delay: Duration,
    /// Maximum delay between reconnection attempts.
    pub reconnect_max_delay: Duration,
    /// Backoff multiplier for reconnection delays.
    pub reconnect_backoff_factor: f64,
    /// Maximum number of reconnection attempts (None = infinite).
    pub reconnect_max_attempts: Option<u32>,
    /// Random jitter factor (0.0-1.0) for reconnection delays.
    pub reconnect_jitter: f64,

    // Heartbeat settings
    /// Interval between ping messages.
    pub ping_interval: Duration,
    /// Maximum time to wait for pong response.
    pub pong_timeout: Duration,
    /// Use WebSocket protocol-level ping frames (vs application-level).
    pub use_websocket_ping: bool,

    // Request handling
    /// Default timeout for request-response operations.
    pub request_timeout: Duration,
    /// Maximum number of pending requests.
    pub max_pending_requests: usize,
    /// Interval for cleaning up stale pending requests.
    pub pending_cleanup_interval: Duration,

    // Channels
    /// Capacity of subscription broadcast channels.
    pub subscription_channel_capacity: usize,
    /// Capacity of command channel.
    pub command_channel_capacity: usize,
    /// Capacity of event channel.
    pub event_channel_capacity: usize,

    // Connection
    /// Timeout for initial connection.
    pub connect_timeout: Duration,
    /// Whether to send auth message on connect.
    pub auth_on_connect: bool,
    /// Maximum message size in bytes.
    pub max_message_size: usize,
}

impl Default for WsConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            reconnect_initial_delay: Duration::from_secs(1),
            reconnect_max_delay: Duration::from_secs(60),
            reconnect_backoff_factor: 2.0,
            reconnect_max_attempts: None,
            reconnect_jitter: 0.1,
            ping_interval: Duration::from_secs(30),
            pong_timeout: Duration::from_secs(10),
            use_websocket_ping: true,
            request_timeout: Duration::from_secs(30),
            max_pending_requests: 1000,
            pending_cleanup_interval: Duration::from_secs(5),
            subscription_channel_capacity: 256,
            command_channel_capacity: 64,
            event_channel_capacity: 256,
            connect_timeout: Duration::from_secs(10),
            auth_on_connect: false,
            max_message_size: 16 * 1024 * 1024, // 16 MB
        }
    }
}

impl WsConfig {
    /// Create a new configuration with the given URL.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Default::default()
        }
    }

    /// Set the reconnection initial delay.
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

    /// Set the ping interval.
    #[must_use]
    pub fn ping_interval(mut self, interval: Duration) -> Self {
        self.ping_interval = interval;
        self
    }

    /// Set the pong timeout.
    #[must_use]
    pub fn pong_timeout(mut self, timeout: Duration) -> Self {
        self.pong_timeout = timeout;
        self
    }

    /// Set whether to use WebSocket protocol-level pings.
    #[must_use]
    pub fn use_websocket_ping(mut self, use_ws_ping: bool) -> Self {
        self.use_websocket_ping = use_ws_ping;
        self
    }

    /// Set the request timeout.
    #[must_use]
    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Set the maximum pending requests.
    #[must_use]
    pub fn max_pending_requests(mut self, max: usize) -> Self {
        self.max_pending_requests = max;
        self
    }

    /// Set the connection timeout.
    #[must_use]
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Set the event channel capacity.
    #[must_use]
    pub fn event_channel_capacity(mut self, capacity: usize) -> Self {
        self.event_channel_capacity = capacity;
        self
    }

    /// Set whether to authenticate on connect.
    #[must_use]
    pub fn auth_on_connect(mut self, auth: bool) -> Self {
        self.auth_on_connect = auth;
        self
    }

    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.url.is_empty() {
            return Err("URL cannot be empty".to_string());
        }
        BackoffConfig {
            initial_delay: self.reconnect_initial_delay,
            max_delay: self.reconnect_max_delay,
            factor: self.reconnect_backoff_factor,
            jitter: self.reconnect_jitter,
        }
        .validate()?;
        if self.ping_interval.is_zero() {
            return Err("Ping interval must be > 0".to_string());
        }
        if self.pong_timeout.is_zero() {
            return Err("Pong timeout must be > 0".to_string());
        }
        if self.request_timeout.is_zero() {
            return Err("Request timeout must be > 0".to_string());
        }
        if self.pending_cleanup_interval.is_zero() {
            return Err("Pending cleanup interval must be > 0".to_string());
        }
        if self.connect_timeout.is_zero() {
            return Err("Connect timeout must be > 0".to_string());
        }
        if self.max_pending_requests == 0 {
            return Err("Max pending requests must be > 0".to_string());
        }
        if self.subscription_channel_capacity == 0 {
            return Err("Subscription channel capacity must be > 0".to_string());
        }
        if self.command_channel_capacity == 0 {
            return Err("Command channel capacity must be > 0".to_string());
        }
        if self.event_channel_capacity == 0 {
            return Err("Event channel capacity must be > 0".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = WsConfig::default();
        assert!(config.url.is_empty());
        assert_eq!(config.reconnect_initial_delay, Duration::from_secs(1));
        assert_eq!(config.reconnect_max_delay, Duration::from_secs(60));
        assert_eq!(config.reconnect_backoff_factor, 2.0);
        assert!(config.reconnect_max_attempts.is_none());
        assert_eq!(config.ping_interval, Duration::from_secs(30));
        assert_eq!(config.pong_timeout, Duration::from_secs(10));
        assert!(config.use_websocket_ping);
        assert_eq!(config.request_timeout, Duration::from_secs(30));
        assert_eq!(config.max_pending_requests, 1000);
        assert_eq!(config.event_channel_capacity, 256);
        assert_eq!(config.connect_timeout, Duration::from_secs(10));
        assert!(!config.auth_on_connect);
        assert_eq!(config.max_message_size, 16 * 1024 * 1024);
    }

    #[test]
    fn test_builder_pattern() {
        let config = WsConfig::new("wss://example.com")
            .ping_interval(Duration::from_secs(15))
            .request_timeout(Duration::from_secs(60))
            .reconnect_max_attempts(Some(5))
            .auth_on_connect(true);

        assert_eq!(config.url, "wss://example.com");
        assert_eq!(config.ping_interval, Duration::from_secs(15));
        assert_eq!(config.request_timeout, Duration::from_secs(60));
        assert_eq!(config.reconnect_max_attempts, Some(5));
        assert!(config.auth_on_connect);
    }

    #[test]
    fn test_validation_empty_url() {
        let config = WsConfig::default();
        let result = config.validate();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "URL cannot be empty");
    }

    #[test]
    fn test_validation_invalid_backoff() {
        let config = WsConfig::new("wss://example.com").reconnect_backoff_factor(0.5);
        let result = config.validate();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Backoff factor must be >= 1.0");
    }

    #[test]
    fn test_validation_valid_config() {
        let config = WsConfig::new("wss://example.com");
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_all_builder_methods() {
        let config = WsConfig::new("wss://test.com")
            .reconnect_initial_delay(Duration::from_millis(500))
            .reconnect_max_delay(Duration::from_secs(120))
            .reconnect_backoff_factor(1.5)
            .reconnect_max_attempts(Some(10))
            .ping_interval(Duration::from_secs(20))
            .pong_timeout(Duration::from_secs(5))
            .use_websocket_ping(false)
            .request_timeout(Duration::from_secs(45))
            .max_pending_requests(500)
            .event_channel_capacity(128)
            .connect_timeout(Duration::from_secs(15))
            .auth_on_connect(true);

        assert_eq!(config.url, "wss://test.com");
        assert_eq!(config.reconnect_initial_delay, Duration::from_millis(500));
        assert_eq!(config.reconnect_max_delay, Duration::from_secs(120));
        assert_eq!(config.reconnect_backoff_factor, 1.5);
        assert_eq!(config.reconnect_max_attempts, Some(10));
        assert_eq!(config.ping_interval, Duration::from_secs(20));
        assert_eq!(config.pong_timeout, Duration::from_secs(5));
        assert!(!config.use_websocket_ping);
        assert_eq!(config.request_timeout, Duration::from_secs(45));
        assert_eq!(config.max_pending_requests, 500);
        assert_eq!(config.event_channel_capacity, 128);
        assert_eq!(config.connect_timeout, Duration::from_secs(15));
        assert!(config.auth_on_connect);
    }

    #[test]
    fn test_validation_zero_command_channel_capacity() {
        let mut config = WsConfig::new("wss://test.com");
        config.command_channel_capacity = 0;
        let result = config.validate();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Command channel capacity must be > 0");
    }

    #[test]
    fn test_validation_zero_ping_interval() {
        let mut config = WsConfig::new("wss://test.com");
        config.ping_interval = Duration::ZERO;
        let result = config.validate();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Ping interval must be > 0");
    }
}
