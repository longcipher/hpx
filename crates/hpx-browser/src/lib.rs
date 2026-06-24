#![deny(unsafe_code)]

pub mod challenge;
pub mod css_cascade;
pub mod css_parser;
pub mod css_selectors;
pub mod css_values;
pub mod dom;
pub mod host;
pub mod html_parser;
pub mod iframe;
pub mod layout;
pub mod net;
pub mod page;
pub mod parallel;
pub mod pool;
pub mod stealth;
pub mod tls;

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

#[cfg(all(test, feature = "proptest"))]
mod css_property_tests;
