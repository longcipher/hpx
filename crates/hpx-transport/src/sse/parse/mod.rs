//! Inlined SSE parsing implementation.
//!
//! This module contains a self-contained Server-Sent Events parser based on the
//! [HTML Living Standard](https://html.spec.whatwg.org/multipage/server-sent-events.html).
//! It was originally derived from the `sseer` crate and inlined to eliminate
//! the external dependency.

pub(crate) mod constants;
pub(crate) mod errors;
pub mod event;
pub mod event_stream;
pub(crate) mod parser;

pub use errors::EventStreamError;
pub use event::Event;
pub use event_stream::EventStream;
