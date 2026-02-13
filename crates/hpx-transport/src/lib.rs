//! # hpx-transport
//!
//! Exchange SDK toolkit for cryptocurrency trading applications.
//!
//! This crate builds on `hpx` to provide exchange-specific functionality:
//!
//! - **Authentication**: API key, HMAC signing, and custom auth strategies
//! - **WebSocket**: Single-task connection with `Connection`/`Handle`/`Stream` split API
//! - **SSE** *(feature `sse`)*: Server-Sent Events transport with auto-reconnection,
//!   protocol handlers, and `SseConnection`/`SseHandle`/`SseStream` split API
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
//! ## WebSocket Quick Start (Split API)
//!
//! ```rust,no_run
//! use hpx_transport::websocket::{Connection, Event, WsConfig, handlers::GenericJsonHandler};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = WsConfig::new("wss://api.exchange.com/ws");
//! let handler = GenericJsonHandler::new();
//!
//! let connection = Connection::connect_stream(config, handler).await?;
//! let (handle, mut stream) = connection.split();
//!
//! handle.subscribe("trades.BTC").await?;
//! while let Some(event) = stream.next().await {
//!     if let Event::Message(msg) = event {
//!         println!("Control/unknown message: {:?}", msg.kind);
//!     }
//! }
//! # Ok(())
//! # }
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
#[cfg(feature = "sse")]
pub mod sse;
pub mod typed;
pub mod websocket;

// Re-export commonly used types
pub use auth::{ApiKeyAuth, Authentication, BearerAuth, HmacAuth, NoAuth};
pub use error::{TransportError, TransportResult};
pub use exchange::{ExchangeClient, RestClient, RestConfig};
pub use rate_limit::RateLimiter;
pub use typed::{ApiError, TypedResponse};
pub use websocket::{
    // Core connection API
    Connection,
    ConnectionEpoch,
    ConnectionHandle,
    // Connection state
    ConnectionState,
    ConnectionStream,
    Event,
    MessageKind,
    // Protocol abstraction
    ProtocolHandler,
    RequestId,
    // Subscription guard
    SubscriptionGuard,
    Topic,
    // Core client types
    WsClient,
    WsConfig,
    WsMessage,
};
