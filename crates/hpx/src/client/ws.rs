//! WebSocket Upgrade
//!
//! This module provides WebSocket support with switchable backends:
//! - `ws-yawc` (default): Uses the hpx-yawc WebSocket library
//! - `ws-fastwebsockets`: Uses the fastwebsockets library

#[cfg(feature = "json")]
mod json;
pub mod message;

// Re-export message types at module level for internal use by json.rs
pub use message::{Message, Utf8Bytes};

#[cfg(feature = "ws-fastwebsockets")]
mod backend_fastwebsockets;
#[cfg(feature = "ws-fastwebsockets")]
pub use backend_fastwebsockets::*;

#[cfg(all(feature = "ws-yawc", not(feature = "ws-fastwebsockets")))]
mod backend_yawc;
#[cfg(all(feature = "ws-yawc", not(feature = "ws-fastwebsockets")))]
pub use backend_yawc::*;
// Suppress unused extern crate warning when both ws backends are enabled
// (ws-fastwebsockets takes priority, so the yawc backend module is excluded)
#[cfg(all(feature = "ws-yawc", feature = "ws-fastwebsockets"))]
use hpx_yawc as _;
