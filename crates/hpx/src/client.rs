mod body;
#[doc(hidden)]
pub mod conn;
mod core;
mod emulation;
mod http;
mod request;
mod response;
pub mod tower_compat;

pub mod layer;
#[cfg(feature = "multipart")]
pub mod multipart;
#[cfg(feature = "sse")]
pub mod sse;
#[cfg(feature = "ws-yawc")]
pub mod ws;

#[allow(unused_imports)]
pub(crate) use self::conn::{Connected, Connection};
#[cfg(feature = "http1")]
pub use self::core::http1;
#[cfg(feature = "http2")]
pub use self::core::http2;
#[cfg(any(feature = "boring-tls", feature = "openssl-tls"))]
pub(crate) use self::http::ConnectIdentity;
pub use self::{
    body::{AsSendBody, Body, ClientResponseBody},
    conn::HttpInfo,
    core::upgrade::Upgraded,
    emulation::{BrowserProfile, Emulation, EmulationBuilder, EmulationFactory},
    http::{Client, ClientBuilder},
    request::{Request, RequestBuilder},
    response::Response,
};
pub(crate) use self::{
    core::{Error as CoreError, ext},
    http::{ConnectRequest, client::error::Error},
};
