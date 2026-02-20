//! Emulation for different browsers.

#[macro_use]
mod macros;
pub mod chrome;
pub mod firefox;
pub mod okhttp;
pub mod opera;
pub mod safari;

#[cfg(feature = "emulation-compression")]
pub use hpx::header::ACCEPT_ENCODING;
pub use hpx::{
    Emulation,
    header::{ACCEPT, ACCEPT_LANGUAGE, HeaderMap, HeaderName, HeaderValue, USER_AGENT},
    http2::{
        Http2Options, Priorities, Priority, PseudoId, PseudoOrder, SettingId, SettingsOrder,
        StreamDependency, StreamId,
    },
    tls::{
        AlpnProtocol, AlpsProtocol, CertificateCompressionAlgorithm, ExtensionType, TlsOptions,
        TlsVersion,
    },
};
pub use typed_builder::TypedBuilder;

pub use crate::emulation::{EmulationOS, EmulationOption};
