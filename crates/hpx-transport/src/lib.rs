//! # hpx-transport
//!
//! Exchange SDK toolkit for cryptocurrency trading applications.
//!
//! This crate builds on `hpx` to provide exchange-specific functionality:
//!
//! - **Authentication**: API key, HMAC signing, and custom auth strategies
//! - **WebSocket**: Actor-based WebSocket with automatic reconnection
//! - **Typed Responses**: Generic response wrapper with metadata
//! - **Rate Limiting**: Token bucket rate limiter
//! - **Metrics**: OpenTelemetry metrics integration
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use hpx_transport::{
//!     auth::ApiKeyAuth,
//!     exchange::{ExchangeClient, RestClient, RestConfig},
//! };
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config =
//!         RestConfig::new("https://api.example.com").timeout(std::time::Duration::from_secs(30));
//!
//!     let auth = ApiKeyAuth::header("X-API-Key", "my-api-key");
//!     let client = RestClient::new(config, auth)?;
//!
//!     // Use the client...
//!     Ok(())
//! }
//! ```
//!
//! ## Rate Limiting Example
//!
//! ```rust
//! use hpx_transport::rate_limit::RateLimiter;
//!
//! let limiter = RateLimiter::new();
//! limiter.add_limit("orders", 10, 1.0); // 10 capacity, 1/sec refill
//!
//! if limiter.try_acquire("orders") {
//!     println!("Request allowed");
//! }
//! ```

pub mod auth;
pub mod error;
pub mod exchange;
pub mod metrics;
pub mod rate_limit;
pub mod typed;
pub mod websocket;

// Re-export commonly used types
pub use auth::{ApiKeyAuth, Authentication, BearerAuth, HmacAuth, NoAuth};
pub use error::{TransportError, TransportResult};
pub use exchange::{ExchangeClient, RestClient, RestConfig};
pub use rate_limit::RateLimiter;
pub use typed::{ApiError, TypedResponse};
pub use websocket::{
    // Connection state
    ConnectionState,
    // Backward compatibility (legacy types)
    ExchangeHandler,
    MessageKind,
    // Protocol abstraction
    ProtocolHandler,
    RequestId,
    Topic,
    WebSocketConfig,
    WebSocketHandle,
    // Core client types
    WsClient,
    WsConfig,
    WsMessage,
};
