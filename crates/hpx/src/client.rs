mod body;
#[doc(hidden)]
pub(crate) mod conn;
mod core;
mod emulation;
mod http;
mod request;
mod response;
pub(crate) mod tower_compat;

pub(crate) mod layer;
#[cfg(feature = "multipart")]
pub mod multipart;
#[cfg(feature = "sse")]
pub mod sse;
#[cfg(feature = "ws-yawc")]
pub mod ws;

#[cfg(feature = "http1")]
pub use self::core::http1;
#[cfg(feature = "http2")]
pub use self::core::http2;
#[cfg(feature = "http3")]
pub use self::core::http3;
#[cfg(any(feature = "boring-tls", feature = "openssl-tls"))]
pub(crate) use self::http::ConnectIdentity;
pub use self::{
    body::{AsSendBody, Body, ClientResponseBody},
    core::upgrade::Upgraded,
    emulation::{BrowserProfile, Emulation, EmulationBuilder, EmulationFactory},
    http::{Client, ClientBuilder},
    request::{Request, RequestBuilder},
    response::Response,
};
pub(crate) use self::{
    conn::{Connected, Connection},
    core::{Error as CoreError, ext},
    http::{ConnectRequest, client::error::Error},
};
