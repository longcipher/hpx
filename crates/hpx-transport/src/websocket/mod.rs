//! Advanced WebSocket functionality with dual interaction patterns.
//!
//! This module provides a robust WebSocket client implementation with support for:
//!
//! - **Request-Response Pattern**: Send requests and await correlated responses with
//!   automatic timeout handling
//! - **Subscription Pattern**: Subscribe to topics and receive updates via broadcast channels
//! - **Auto-Reconnection**: Exponential backoff with jitter for reliable connections
//! - **Lock-Free Concurrency**: High-performance stores using `scc::HashMap`
//! - **Protocol Abstraction**: Exchange-agnostic via the [`ProtocolHandler`] trait
//!
//! # Architecture
//!
//! The WebSocket system uses an actor-based architecture:
//!
//! ```text
//! ┌─────────────┐     ┌─────────────────┐     ┌──────────────┐
//! │  WsClient   │────▶│ ConnectionActor │────▶│   Exchange   │
//! │  (Clone)    │     │   (Background)  │     │   Server     │
//! └─────────────┘     └───────┬─────────┘     └──────────────┘
//!                             │
//!          ┌──────────────────┴────────────────────┐
//!          ▼                                       ▼
//! ┌─────────────────────┐             ┌────────────────────────┐
//! │ PendingRequestStore │             │   SubscriptionStore    │
//! │   (scc::HashMap)    │             │    (scc::HashMap)      │
//! └─────────────────────┘             └────────────────────────┘
//! ```
//!
//! # Quick Start
//!
//! ## Request-Response Example
//!
//! ```rust,ignore
//! use hpx_transport::websocket::{WsClient, WsConfig};
//! use hpx_transport::websocket::handlers::GenericJsonHandler;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Configure the WebSocket connection
//!     let config = WsConfig::new("wss://api.exchange.com/ws")
//!         .ping_interval(std::time::Duration::from_secs(30))
//!         .request_timeout(std::time::Duration::from_secs(10));
//!
//!     // Create a protocol handler
//!     let handler = GenericJsonHandler::new();
//!
//!     // Connect
//!     let client = WsClient::connect(config, handler).await?;
//!
//!     // Send a request and await response
//!     let response: serde_json::Value = client.request(&serde_json::json!({
//!         "method": "getOrderBook",
//!         "params": {"symbol": "BTCUSD"}
//!     })).await?;
//!
//!     println!("Response: {:?}", response);
//!     Ok(())
//! }
//! ```
//!
//! ## Subscription Example
//!
//! ```rust,ignore
//! use hpx_transport::websocket::{WsClient, WsConfig, Topic};
//! use hpx_transport::websocket::handlers::GenericJsonHandler;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = WsConfig::new("wss://api.exchange.com/ws");
//!     let handler = GenericJsonHandler::new();
//!     let client = WsClient::connect(config, handler).await?;
//!
//!     // Subscribe to a topic
//!     let mut rx = client.subscribe("orderbook.BTC").await?;
//!
//!     // Receive updates
//!     while let Ok(msg) = rx.recv().await {
//!         println!("Update: {:?}", msg);
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! # Creating a Custom Protocol Handler
//!
//! Implement [`ProtocolHandler`] for your exchange's specific protocol:
//!
//! ```rust
//! use hpx_transport::websocket::{MessageKind, ProtocolHandler, RequestId, Topic, WsMessage};
//!
//! struct MyExchangeHandler;
//!
//! impl ProtocolHandler for MyExchangeHandler {
//!     fn classify_message(&self, message: &str) -> MessageKind {
//!         // Parse and classify the message
//!         if message.contains("\"type\":\"response\"") {
//!             MessageKind::Response
//!         } else if message.contains("\"type\":\"update\"") {
//!             MessageKind::Update
//!         } else {
//!             MessageKind::Unknown
//!         }
//!     }
//!
//!     fn extract_request_id(&self, message: &str) -> Option<RequestId> {
//!         // Extract ID from response
//!         None // Simplified
//!     }
//!
//!     fn extract_topic(&self, message: &str) -> Option<Topic> {
//!         // Extract topic from update
//!         None // Simplified
//!     }
//!
//!     fn build_subscribe(&self, topics: &[Topic], request_id: RequestId) -> WsMessage {
//!         WsMessage::text(format!(r#"{{"op":"subscribe","id":"{}"}}"#, request_id))
//!     }
//!
//!     fn build_unsubscribe(&self, topics: &[Topic], request_id: RequestId) -> WsMessage {
//!         WsMessage::text(format!(r#"{{"op":"unsubscribe","id":"{}"}}"#, request_id))
//!     }
//! }
//! ```
//!
//! # Configuration
//!
//! Use [`WsConfig`] to customize connection behavior:
//!
//! | Setting | Default | Description |
//! |---------|---------|-------------|
//! | `ping_interval` | 30s | Interval between heartbeat pings |
//! | `pong_timeout` | 10s | Max wait for pong response |
//! | `request_timeout` | 30s | Default request-response timeout |
//! | `reconnect_initial_delay` | 1s | Initial reconnection delay |
//! | `reconnect_max_delay` | 60s | Maximum reconnection delay |
//! | `max_pending_requests` | 1000 | Maximum concurrent pending requests |
//!
//! # Thread Safety
//!
//! All types in this module are designed for concurrent use:
//!
//! - [`WsClient`] is `Clone` and can be shared across tasks
//! - [`PendingRequestStore`] uses lock-free `scc::HashMap`
//! - [`SubscriptionStore`] uses lock-free `scc::HashMap`
//!
//! # Error Handling
//!
//! WebSocket operations return [`TransportResult<T>`](crate::error::TransportResult).
//! Common error cases:
//!
//! - [`TransportError::RequestTimeout`](crate::error::TransportError::RequestTimeout) - Request timed out
//! - [`TransportError::ConnectionClosed`](crate::error::TransportError::ConnectionClosed) - Connection lost
//! - [`TransportError::CapacityExceeded`](crate::error::TransportError::CapacityExceeded) - Too many pending requests
//!
//! # Module Structure
//!
//! - `config`: WebSocket connection configuration
//! - `types`: Core type definitions (RequestId, Topic, MessageKind)
//! - `protocol`: Protocol handler trait for exchange abstraction
//! - [`handlers`]: Ready-to-use protocol handler implementations
//! - `actor`: Connection actor for WebSocket lifecycle management
//! - `client`: Actor-based WebSocket client implementation
//! - `pending`: Lock-free pending request management
//! - `subscription`: Lock-free subscription management

mod actor;
mod client;
mod config;
pub mod connection;
pub mod handlers;
mod pending;
mod protocol;
mod subscription;
mod types;
mod ws_client;

// Re-export config types
// Re-export actor types
pub use actor::{ActorCommand, ConnectionActor, ConnectionState};
// Re-export client types
pub use client::{ExchangeHandler, WebSocketConfig, WebSocketHandle};
pub use config::WsConfig;
// Re-export new connection API types
pub use connection::{Connection, ConnectionEpoch, ConnectionHandle, ConnectionStream, Event};
// Re-export state management stores
pub use pending::PendingRequestStore;
// Re-export protocol types
pub use protocol::{ProtocolHandler, WsMessage};
pub use subscription::SubscriptionStore;
// Re-export core types
pub use types::{MessageKind, RequestId, Topic};
pub use ws_client::WsClient;
