// TODO: progressively remove these blanket clippy allows as the crate matures
#![allow(clippy::pedantic)]
#![allow(clippy::nursery)]
#![allow(clippy::cargo)]
#![allow(clippy::style)]
#![allow(clippy::allow_attributes)]
#![allow(clippy::panic)]
// TODO: remove once dom.rs blitz-dom API errors are resolved and full build passes;
// then fix dead code warnings with targeted #[allow(dead_code)] on specific items
#![allow(dead_code)]
#![allow(missing_docs)]
#![allow(missing_debug_implementations)]
#![allow(rustdoc::missing_crate_level_docs)]
#![allow(rustdoc::invalid_html_tags)]
#![deny(unsafe_code)]

pub mod challenge;
pub mod dom;
pub mod extract;
pub mod html_parser;
pub mod iframe;
pub mod layout;
pub mod markdown;
pub mod net;
pub mod page;
pub mod parallel;
pub mod pool;
pub mod resource_loader;
pub mod stealth;
pub mod tls;
pub mod utils;

#[cfg(feature = "v8")]
pub mod js_runtime;

#[cfg(feature = "v8")]
pub mod event_loop;

#[cfg(feature = "canvas")]
pub mod canvas;

#[cfg(feature = "workers")]
pub mod workers;

#[cfg(feature = "cdp")]
pub mod protocol;

#[cfg(feature = "cdp-client")]
pub mod cdp_client;

#[cfg(feature = "cdp-client")]
pub mod chrome;

#[cfg(feature = "cdp-client")]
pub mod cdp_page;

#[cfg(feature = "cdp-client")]
pub mod locator;

#[cfg(feature = "cdp-client")]
pub mod har;
