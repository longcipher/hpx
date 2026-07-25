//! Lower-level client connection API.
//!
//! The types in this module are to provide a lower-level API based around a
//! single connection. Connecting to a host, pooling connections, and the like
//! are not handled at this level. This module provides the building blocks to
//! customize those things externally.

#[cfg(feature = "http1")]
pub(crate) mod http1;
#[cfg(feature = "http2")]
pub(crate) mod http2;

pub(crate) use super::dispatch::TrySendError;
