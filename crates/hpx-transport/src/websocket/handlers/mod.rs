//! Protocol handler implementations for common WebSocket patterns.
//!
//! This module provides ready-to-use handlers for common protocols:
//!
//! - [`GenericJsonHandler`]: Configurable handler for JSON-based protocols
//! - [`JsonRpcHandler`]: JSON-RPC 2.0 compliant handler

mod generic_json;
mod jsonrpc;

pub use generic_json::{GenericJsonConfig, GenericJsonHandler};
pub use jsonrpc::JsonRpcHandler;
