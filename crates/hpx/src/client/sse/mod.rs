//! Server-Sent Events (SSE) support for hpx.
//!
//! This module provides a reconnecting SSE client built on top of hpx's HTTP client.
//!
//! # Example
//!
//! ```rust,no_run
//! use futures_util::StreamExt;
//! use hpx::sse::{RequestBuilderSseExt, SseEvent};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut source = hpx::get("https://example.com/events").into_event_source();
//!
//! while let Some(event) = source.next().await {
//!     match event? {
//!         SseEvent::Open => println!("connected"),
//!         SseEvent::Message(msg) => println!("{}: {}", msg.event, msg.data),
//!         SseEvent::Error(err) => eprintln!("reconnecting: {err}"),
//!     }
//! }
//! # Ok(())
//! # }
//! ```

mod decode;
mod event_source;
mod retry;
mod stream;

pub use decode::{MessageEvent, PayloadTooLargeError, SseDecoder, SseEvent as DecoderEvent};
pub use event_source::{
    Error as SseClientError, EventSource, EventSourceBuilder, ReadyState, RequestBuilderSseExt,
    SseErrorEvent, SseEvent,
};
pub use retry::SseRetryConfig;
pub use stream::{SseStream, SseStreamError};
