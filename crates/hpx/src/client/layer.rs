//! Middleware for the client.

#[cfg(feature = "auth")]
pub(crate) mod auth;
pub(crate) mod auto_header;
pub(crate) mod circuit_breaker;
pub(crate) mod config;
#[cfg(feature = "cookies")]
pub(crate) mod cookie;
#[cfg(any(
    feature = "gzip",
    feature = "zstd",
    feature = "brotli",
    feature = "deflate",
))]
pub(crate) mod decoder;
pub(crate) mod hooks;
pub(crate) mod recovery;
pub(crate) mod redirect;
pub(crate) mod retry;
pub(crate) mod timeout;
