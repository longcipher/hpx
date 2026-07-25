//! HTTP Client protocol implementation and low level utilities.

mod common;
mod dispatch;
mod error;
pub(crate) use self::error::BoxError;
mod proto;

pub(super) mod body;
pub(super) mod conn;
pub(crate) mod ext;
#[cfg(feature = "http1")]
pub mod http1;
#[cfg(feature = "http2")]
pub mod http2;
#[cfg(feature = "http3")]
pub mod http3;
pub(super) mod rt;
pub(super) mod upgrade;

pub(crate) use self::error::{Error, Result};
