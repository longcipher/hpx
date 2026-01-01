#![deny(unused)]
#![deny(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(test, deny(warnings))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
//! # hpx-util
//!
//! Utilities for hpx.
//!
//! This crate provides utility modules for the `hpx` HTTP client, including:
//!
//! - **Emulation**: Browser emulation capabilities (TLS fingerprinting, HTTP/2 settings).
//! - **Tower**: Middleware layers for `tower` services (e.g., delay, jitter).

#[cfg(feature = "emulation")]
pub mod emulation;
pub mod tower;

#[cfg(feature = "emulation")]
pub use self::emulation::{Emulation, EmulationOS, EmulationOption};
