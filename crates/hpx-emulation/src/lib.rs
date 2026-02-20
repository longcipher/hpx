#![deny(unused)]
#![deny(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(test, deny(warnings))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
//! # hpx-emulation
//!
//! Browser emulation utilities for hpx.
//!
//! This crate provides browser emulation capabilities for the `hpx` HTTP client:
//!
//! - **Emulation**: Browser emulation profiles (TLS fingerprinting, HTTP/2 settings).

use hpx as _;
#[cfg(feature = "emulation-serde")]
use serde as _;

#[cfg(feature = "emulation")]
pub mod emulation;

#[cfg(feature = "emulation")]
pub use self::emulation::{Emulation, EmulationOS, EmulationOption};
