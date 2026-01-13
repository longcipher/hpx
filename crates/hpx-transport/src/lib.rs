//! # LongTrader Transport
//!
//! High-performance, extensible transport layer for cryptocurrency trading applications.
//!
//! This crate provides a unified abstraction over HTTP and WebSocket communications
//! with support for middleware, connection pooling, metrics, and comprehensive error handling.
//!
//! ## Features
//!
//! - **Unified Transport Abstraction**: Common interface for HTTP and WebSocket
//! - **Middleware Support**: Pluggable middleware for authentication, retry, rate limiting
//! - **Connection Management**: Automatic connection pooling and health monitoring
//! - **High Performance**: Zero-copy operations and async-first design
//! - **Observability**: Built-in metrics, tracing, and structured logging
//! - **Type Safety**: Strongly typed APIs with comprehensive error handling
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use hpx_transport::{
//!     auth::NoAuth,
//!     http::{HttpClient, HttpConfig},
//! };
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = HttpConfig::builder("https://api.example.com")
//!         .timeout(std::time::Duration::from_secs(30))
//!         .build()?;
//!
//!     let client = HttpClient::new(config, NoAuth)?;
//!
//!     // Use the client...
//!     Ok(())
//! }
//! ```

pub mod auth;
pub mod error;
pub mod hooks;
pub mod metrics;
pub mod middleware;
pub mod transport;
pub mod typed;

#[cfg(feature = "http")]
pub mod http;

#[cfg(feature = "websocket")]
pub mod websocket;

// Re-export commonly used types
pub use error::{TransportError, TransportResult};
pub use hooks::{
    AfterResponseHook, BeforeRedirectHook, BeforeRequestHook, BeforeRetryHook, HeaderInjectionHook,
    HookError, Hooks, LoggingHook, OnErrorHook, RequestIdHook,
};
#[cfg(feature = "http")]
pub use http::{
    HttpClient, HttpConfig, RequestHandler, RestClient, RestClientBuilder, StreamingResponse,
    StreamingResponseBuilder,
};
pub use transport::{Request, Response, Transport};
pub use typed::{
    ApiError, GenericApiError, TypedApiError, TypedResponse, TypedResponseExt, TypedResult,
};
#[cfg(feature = "websocket")]
pub use websocket::{ExchangeHandler, WebSocketClient, WebSocketConfig};
