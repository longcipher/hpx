//! Error types for CDP client operations.

/// Result type alias for CDP client operations.
pub type Result<T> = std::result::Result<T, CdpClientError>;

/// Errors that can occur during CDP client operations.
#[derive(Debug, thiserror::Error)]
pub enum CdpClientError {
    /// WebSocket connection or communication error.
    #[error("WebSocket error during {operation}: {message}")]
    WebSocket {
        /// The operation that failed.
        operation: String,
        /// Error message.
        message: String,
    },

    /// Failed to connect to the browser endpoint.
    #[error("connection to {endpoint} failed: {reason}")]
    ConnectionFailed {
        /// The endpoint that failed.
        endpoint: String,
        /// Why it failed.
        reason: String,
    },

    /// A CDP command returned an error.
    #[error("command '{command}' failed: {reason}")]
    CommandFailed {
        /// The command name that failed.
        command: String,
        /// Why it failed.
        reason: String,
    },

    /// A CDP protocol-level error response.
    #[error("CDP error {code} on {method}: {message}")]
    CdpError {
        /// CDP error code.
        code: i32,
        /// The method that triggered the error.
        method: String,
        /// Error message from CDP.
        message: String,
    },

    /// An operation timed out.
    #[error("timeout after {timeout_secs}s during {operation}")]
    Timeout {
        /// The operation that timed out.
        operation: String,
        /// Timeout duration in seconds.
        timeout_secs: u64,
    },

    /// Requested page was not found.
    #[error("page not found: {0}")]
    PageNotFound(String),

    /// Navigation to a URL failed.
    #[error("navigation to {url} failed: {reason}")]
    NavigationFailed {
        /// The URL that failed.
        url: String,
        /// Why it failed.
        reason: String,
    },

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl CdpClientError {
    /// Create a WebSocket error.
    #[must_use]
    pub fn websocket(operation: &str, message: &str) -> Self {
        Self::WebSocket {
            operation: operation.to_string(),
            message: message.to_string(),
        }
    }

    /// Create a connection failed error.
    #[must_use]
    pub fn connection_failed(endpoint: &str, reason: &str) -> Self {
        Self::ConnectionFailed {
            endpoint: endpoint.to_string(),
            reason: reason.to_string(),
        }
    }

    /// Create a command failed error.
    #[must_use]
    pub fn command_failed(command: &str, reason: &str) -> Self {
        Self::CommandFailed {
            command: command.to_string(),
            reason: reason.to_string(),
        }
    }

    /// Create a timeout error.
    #[must_use]
    pub fn timeout(operation: &str, timeout_secs: u64) -> Self {
        Self::Timeout {
            operation: operation.to_string(),
            timeout_secs,
        }
    }

    /// Create a navigation failed error.
    #[must_use]
    pub fn navigation_failed(url: &str, reason: &str) -> Self {
        Self::NavigationFailed {
            url: url.to_string(),
            reason: reason.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_error_display() {
        let err = CdpClientError::websocket("connect", "connection refused");
        let msg = err.to_string();
        assert!(msg.contains("WebSocket"));
        assert!(msg.contains("connect"));
        assert!(msg.contains("connection refused"));
    }

    #[test]
    fn connection_failed_display() {
        let err = CdpClientError::connection_failed("ws://localhost:9222", "refused");
        let msg = err.to_string();
        assert!(msg.contains("ws://localhost:9222"));
        assert!(msg.contains("refused"));
    }

    #[test]
    fn command_failed_display() {
        let err = CdpClientError::command_failed("Page.navigate", "invalid URL");
        let msg = err.to_string();
        assert!(msg.contains("Page.navigate"));
        assert!(msg.contains("invalid URL"));
    }

    #[test]
    fn cdp_error_display() {
        let err = CdpClientError::CdpError {
            code: -32601,
            method: "Unknown.method".to_string(),
            message: "not found".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("-32601"));
        assert!(msg.contains("Unknown.method"));
        assert!(msg.contains("not found"));
    }

    #[test]
    fn timeout_error_display() {
        let err = CdpClientError::timeout("load", 30);
        let msg = err.to_string();
        assert!(msg.contains("30"));
        assert!(msg.contains("load"));
    }

    #[test]
    fn page_not_found_display() {
        let err = CdpClientError::PageNotFound("tab-42".to_string());
        let msg = err.to_string();
        assert!(msg.contains("tab-42"));
    }

    #[test]
    fn navigation_failed_display() {
        let err =
            CdpClientError::navigation_failed("https://example.com", "net::ERR_NAME_NOT_RESOLVED");
        let msg = err.to_string();
        assert!(msg.contains("https://example.com"));
        assert!(msg.contains("net::ERR_NAME_NOT_RESOLVED"));
    }

    #[test]
    fn json_error_conversion() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid").unwrap_err();
        let err: CdpClientError = json_err.into();
        let msg = err.to_string();
        assert!(msg.contains("JSON"));
    }

    #[test]
    fn io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err: CdpClientError = io_err.into();
        let msg = err.to_string();
        assert!(msg.contains("I/O"));
        assert!(msg.contains("file missing"));
    }
}
