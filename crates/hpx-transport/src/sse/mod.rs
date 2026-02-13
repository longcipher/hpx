//! Server-Sent Events (SSE) transport for exchange data streaming.
//!
//! This module provides an SSE-based transport implementation with support for:
//!
//! - **Event Streaming**: Receive real-time updates via SSE connections using
//!   the inlined SSE parser for spec-compliant SSE parsing.
//! - **Auto-Reconnection**: Reliable connections with configurable exponential
//!   backoff, `Last-Event-ID` tracking, and maximum attempt limits.
//! - **Protocol Abstraction**: Exchange-agnostic via the [`SseProtocolHandler`]
//!   trait — classify events, hook into lifecycle events, and control retry
//!   behaviour per-exchange.
//! - **Authentication**: Optional request signing via
//!   [`Authentication`](crate::auth::Authentication) on every connection and
//!   reconnection.
//! - **Connection/Handle/Stream Split**: Mirrors the WebSocket module's
//!   [`Connection`](crate::websocket::Connection) pattern — spawn a background
//!   task, then interact via clone-able [`SseHandle`] and consumable
//!   [`SseStream`].
//!
//! # Architecture
//!
//! ```text
//! SseConnection::connect(config, handler)
//!   └─ spawns background task ──► tokio::spawn(sse_connection_driver)
//!        │                              │
//!        ├── SseHandle ◄─── mpsc ◄──────┤  (commands: Close, Reconnect)
//!        │                              │
//!        └── SseStream ◄─── mpsc ◄──────┘  (SseEvent items)
//! ```
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use std::time::Duration;
//!
//! use hpx_transport::sse::{
//!     SseConfig, SseConnection, SseMessageKind, handlers::GenericSseHandler,
//! };
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = SseConfig::new("https://api.exchange.com/v1/stream")
//!     .connect_timeout(Duration::from_secs(10))
//!     .reconnect_max_attempts(Some(5));
//!
//! let handler = GenericSseHandler::new();
//! let connection = SseConnection::connect(config, handler).await?;
//! let (handle, mut stream) = connection.split();
//!
//! while let Some(event) = stream.next_event().await {
//!     match event.kind {
//!         SseMessageKind::Data => {
//!             println!("type={} data={}", event.event_type(), event.data());
//!         }
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Module Index
//!
//! | Module | Description |
//! |--------|-------------|
//! | `config` | [`SseConfig`] builder for connection settings |
//! | [`connection`] | [`SseConnection`], [`SseHandle`], [`SseStream`] |
//! | `protocol` | [`SseProtocolHandler`] trait |
//! | `types` | [`SseEvent`], [`SseMessageKind`] |
//! | [`handlers`] | Ready-to-use handlers ([`GenericSseHandler`](handlers::GenericSseHandler)) |

mod config;
pub mod connection;
pub mod handlers;
pub mod parse;
mod protocol;
mod types;

// Re-export config types
pub use config::SseConfig;
// Re-export connection types
pub use connection::{SseCommand, SseConnection, SseConnectionState, SseHandle, SseStream};
// Re-export inlined SSE parser types
pub use parse::{Event, EventStream, EventStreamError};
// Re-export protocol types
pub use protocol::SseProtocolHandler;
// Re-export core types
pub use types::{SseEvent, SseMessageKind};
