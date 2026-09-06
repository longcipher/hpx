//! Emulation for different browsers.

#[macro_use]
mod macros;
pub(crate) mod chrome;
pub(crate) mod firefox;
pub(crate) mod okhttp;
pub(crate) mod opera;
pub(crate) mod safari;

pub(crate) use bon::Builder;
pub(crate) use chrome::tls::tls_fingerprint_from_preset;
#[cfg(feature = "emulation-compression")]
pub(crate) use hpx::header::ACCEPT_ENCODING;
pub(crate) use hpx::{
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

pub(crate) use crate::emulation::{EmulationOS, EmulationOption};
