#![allow(
    clippy::allow_attributes,
    clippy::expect_used,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used
)]
pub(crate) mod delay_server;
pub(crate) mod error;
pub(crate) mod layer;
pub(crate) mod server;

// TODO: remove once done converting to new support server?
#[allow(unused)]
pub(crate) static DEFAULT_USER_AGENT: &str =
    concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
