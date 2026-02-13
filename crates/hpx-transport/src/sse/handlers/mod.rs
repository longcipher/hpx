//! Protocol handler implementations for common SSE patterns.
//!
//! This module provides ready-to-use handlers for common SSE protocols:
//!
//! - [`GenericSseHandler`]: Configurable handler for generic SSE streams

mod generic;

pub use generic::GenericSseHandler;
