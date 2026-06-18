//! WebSocket Upgrade
//!
//! This module provides WebSocket support using the hpx-yawc backend.

#[cfg(feature = "json")]
mod json;
pub mod message;

// Re-export message types at module level for internal use by json.rs
pub use message::{Message, Utf8Bytes};

#[cfg(feature = "ws-yawc")]
mod backend_yawc;
#[cfg(feature = "ws-yawc")]
pub use backend_yawc::*;
