//!  TLS options configuration
//!
//! By default, a `Client` will make use of BoringSSL for TLS.
//!
//! - Various parts of TLS can also be configured or even disabled on the `ClientBuilder`.

pub(crate) mod conn;
mod keylog;
mod options;
mod x509;

#[cfg(feature = "boring-tls")]
pub(crate) mod boring;
#[cfg(all(feature = "openssl-tls", not(feature = "boring-tls")))]
pub(crate) mod openssl;
#[cfg(feature = "http3")]
pub mod quic;
#[cfg(all(feature = "rustls-tls", not(feature = "boring-tls")))]
pub(crate) mod rustls;

#[cfg(feature = "boring-tls")]
pub use ::boring::ssl::{CertificateCompressionAlgorithm, ExtensionType};

/// Placeholder type when using the OpenSSL backend.
///
/// Certificate compression is a BoringSSL-specific feature.
/// This type exists for API compatibility and has no effect.
#[cfg(all(feature = "openssl-tls", not(feature = "boring-tls")))]
#[derive(Debug, Clone, Copy)]
pub struct CertificateCompressionAlgorithm;
use bytes::Bytes;

pub use self::{
    keylog::KeyLog,
    options::{TlsOptions, TlsOptionsBuilder},
    x509::{CertStore, CertStoreBuilder, Certificate, Identity},
};

/// Http extension carrying extra TLS layer information.
/// Made available to clients on responses when `tls_info` is set.
#[derive(Debug, Clone)]
pub struct TlsInfo {
    pub(crate) peer_certificate: Option<Bytes>,
    pub(crate) peer_certificate_chain: Option<Vec<Bytes>>,
}

impl TlsInfo {
    /// Get the DER encoded leaf certificate of the peer.
    pub fn peer_certificate(&self) -> Option<&[u8]> {
        self.peer_certificate.as_deref()
    }

    /// Get the DER encoded certificate chain of the peer.
    ///
    /// This includes the leaf certificate on the client side.
    pub fn peer_certificate_chain(&self) -> Option<impl Iterator<Item = &[u8]>> {
        self.peer_certificate_chain
            .as_ref()
            .map(|v| v.iter().map(|b| b.as_ref()))
    }
}

#[cfg(any(feature = "boring-tls", feature = "openssl-tls"))]
use bytes::{BufMut, BytesMut};

/// A TLS protocol version.
///
/// Internally stored as the TLS wire encoding (e.g. TLS 1.2 = 0x0303).
/// Each TLS backend module provides `From`/`Into` conversions to its native
/// version type.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct TlsVersion(u16);

impl TlsVersion {
    /// Version 1.0 of the TLS protocol (wire value 0x0301).
    pub const TLS_1_0: TlsVersion = TlsVersion(0x0301);
    /// Version 1.1 of the TLS protocol (wire value 0x0302).
    pub const TLS_1_1: TlsVersion = TlsVersion(0x0302);
    /// Version 1.2 of the TLS protocol (wire value 0x0303).
    pub const TLS_1_2: TlsVersion = TlsVersion(0x0303);
    /// Version 1.3 of the TLS protocol (wire value 0x0304).
    pub const TLS_1_3: TlsVersion = TlsVersion(0x0304);

    /// Convert to the BoringSSL `SslVersion` native type.
    #[cfg(feature = "boring-tls")]
    #[allow(unsafe_code)]
    pub(crate) fn to_native_version(self) -> ::boring::ssl::SslVersion {
        match self.0 {
            0x0301 => ::boring::ssl::SslVersion::TLS1,
            0x0302 => ::boring::ssl::SslVersion::TLS1_1,
            0x0303 => ::boring::ssl::SslVersion::TLS1_2,
            0x0304 => ::boring::ssl::SslVersion::TLS1_3,
            raw => {
                // SAFETY: boring::ssl::SslVersion is repr(i16) and passthrough
                // is the only reasonable fallback for unknown versions.
                unsafe { std::mem::transmute::<i16, ::boring::ssl::SslVersion>(raw as i16) }
            }
        }
    }

    /// Convert to the OpenSSL `SslVersion` native type.
    #[cfg(all(feature = "openssl-tls", not(feature = "boring-tls")))]
    pub(crate) fn to_native_version(self) -> ::openssl::ssl::SslVersion {
        match self.0 {
            0x0301 => ::openssl::ssl::SslVersion::TLS1,
            0x0302 => ::openssl::ssl::SslVersion::TLS1_1,
            0x0303 => ::openssl::ssl::SslVersion::TLS1_2,
            0x0304 => ::openssl::ssl::SslVersion::TLS1_3,
            raw => ::openssl::ssl::SslVersion::from_raw(raw as i32),
        }
    }
}

/// A TLS ALPN protocol.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct AlpnProtocol(&'static [u8]);

impl AlpnProtocol {
    /// Prefer HTTP/1.1
    pub const HTTP1: AlpnProtocol = AlpnProtocol(b"http/1.1");

    /// Prefer HTTP/2
    pub const HTTP2: AlpnProtocol = AlpnProtocol(b"h2");

    /// Prefer HTTP/3
    pub const HTTP3: AlpnProtocol = AlpnProtocol(b"h3");

    /// Create a new [`AlpnProtocol`] from a static byte slice.
    #[inline]
    pub const fn new(value: &'static [u8]) -> Self {
        AlpnProtocol(value)
    }

    /// Returns the raw protocol name bytes (e.g. `b"h2"`, `b"http/1.1"`).
    ///
    /// This is the format expected by rustls's `ClientConfig::alpn_protocols`.
    #[inline]
    pub fn as_wire_bytes(&self) -> &'static [u8] {
        self.0
    }

    /// Encode a single protocol in TLS wire format (length-prefixed).
    ///
    /// This is the format expected by BoringSSL's and OpenSSL's `set_alpn_protos()`.
    #[cfg(any(feature = "boring-tls", feature = "openssl-tls"))]
    #[inline]
    fn encode(self) -> Bytes {
        Self::encode_sequence(std::iter::once(&self))
    }

    /// Encode a sequence of protocols in TLS wire format (each length-prefixed).
    ///
    /// This is the format expected by BoringSSL's and OpenSSL's `set_alpn_protos()`.
    #[cfg(any(feature = "boring-tls", feature = "openssl-tls"))]
    fn encode_sequence<'a, I>(items: I) -> Bytes
    where
        I: IntoIterator<Item = &'a AlpnProtocol>,
    {
        let mut buf = BytesMut::new();
        for item in items {
            buf.put_u8(item.0.len() as u8);
            buf.extend_from_slice(item.0);
        }
        buf.freeze()
    }
}

/// A TLS ALPS protocol.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct AlpsProtocol(&'static [u8]);

impl AlpsProtocol {
    /// Prefer HTTP/1.1
    pub const HTTP1: AlpsProtocol = AlpsProtocol(b"http/1.1");

    /// Prefer HTTP/2
    pub const HTTP2: AlpsProtocol = AlpsProtocol(b"h2");

    /// Prefer HTTP/3
    pub const HTTP3: AlpsProtocol = AlpsProtocol(b"h3");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(feature = "boring-tls", feature = "openssl-tls"))]
    #[test]
    fn alpn_protocol_encode() {
        let alpn = AlpnProtocol::encode_sequence(&[AlpnProtocol::HTTP1, AlpnProtocol::HTTP2]);
        assert_eq!(alpn, Bytes::from_static(b"\x08http/1.1\x02h2"));

        let alpn = AlpnProtocol::encode_sequence(&[AlpnProtocol::HTTP3]);
        assert_eq!(alpn, Bytes::from_static(b"\x02h3"));

        let alpn = AlpnProtocol::encode_sequence(&[AlpnProtocol::HTTP1, AlpnProtocol::HTTP3]);
        assert_eq!(alpn, Bytes::from_static(b"\x08http/1.1\x02h3"));

        let alpn = AlpnProtocol::encode_sequence(&[AlpnProtocol::HTTP2, AlpnProtocol::HTTP3]);
        assert_eq!(alpn, Bytes::from_static(b"\x02h2\x02h3"));

        let alpn = AlpnProtocol::encode_sequence(&[
            AlpnProtocol::HTTP1,
            AlpnProtocol::HTTP2,
            AlpnProtocol::HTTP3,
        ]);
        assert_eq!(alpn, Bytes::from_static(b"\x08http/1.1\x02h2\x02h3"));
    }

    #[cfg(any(feature = "boring-tls", feature = "openssl-tls"))]
    #[test]
    fn alpn_protocol_encode_single() {
        let alpn = AlpnProtocol::HTTP1.encode();
        assert_eq!(alpn, b"\x08http/1.1".as_ref());

        let alpn = AlpnProtocol::HTTP2.encode();
        assert_eq!(alpn, b"\x02h2".as_ref());

        let alpn = AlpnProtocol::HTTP3.encode();
        assert_eq!(alpn, b"\x02h3".as_ref());
    }

    #[test]
    fn alpn_protocol_wire_bytes() {
        assert_eq!(AlpnProtocol::HTTP1.as_wire_bytes(), b"http/1.1");
        assert_eq!(AlpnProtocol::HTTP2.as_wire_bytes(), b"h2");
        assert_eq!(AlpnProtocol::HTTP3.as_wire_bytes(), b"h3");
    }

    #[test]
    fn alpn_protocol_custom() {
        let custom = AlpnProtocol::new(b"spdy/3.1");
        assert_eq!(custom.as_wire_bytes(), b"spdy/3.1");
    }

    #[test]
    fn alpn_protocol_eq_and_hash() {
        use std::collections::HashSet;

        assert_eq!(AlpnProtocol::HTTP1, AlpnProtocol::HTTP1);
        assert_ne!(AlpnProtocol::HTTP1, AlpnProtocol::HTTP2);

        let mut set = HashSet::new();
        set.insert(AlpnProtocol::HTTP1);
        set.insert(AlpnProtocol::HTTP2);
        set.insert(AlpnProtocol::HTTP3);
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn tls_version_constants_exist() {
        // Verify TLS version constants are distinct
        assert_ne!(TlsVersion::TLS_1_0, TlsVersion::TLS_1_1);
        assert_ne!(TlsVersion::TLS_1_1, TlsVersion::TLS_1_2);
        assert_ne!(TlsVersion::TLS_1_2, TlsVersion::TLS_1_3);
    }

    #[test]
    fn tls_version_copy_clone() {
        let v = TlsVersion::TLS_1_3;
        let v2 = v;
        assert_eq!(v, v2);
        let v3 = v.clone();
        assert_eq!(v, v3);
    }

    #[test]
    fn tls_version_debug() {
        let debug = format!("{:?}", TlsVersion::TLS_1_2);
        assert!(!debug.is_empty());
    }

    #[test]
    fn tls_info_no_certificate() {
        let info = TlsInfo {
            peer_certificate: None,
            peer_certificate_chain: None,
        };
        assert!(info.peer_certificate().is_none());
        assert!(info.peer_certificate_chain().is_none());
    }

    #[test]
    fn tls_info_with_certificate() {
        let cert = bytes::Bytes::from_static(&[0x30, 0x82, 0x01, 0x02]);
        let info = TlsInfo {
            peer_certificate: Some(cert.clone()),
            peer_certificate_chain: Some(vec![cert.clone()]),
        };
        assert_eq!(info.peer_certificate(), Some(&[0x30, 0x82, 0x01, 0x02][..]));
        let chain: Vec<&[u8]> = info.peer_certificate_chain().unwrap().collect();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0], &[0x30, 0x82, 0x01, 0x02][..]);
    }

    #[test]
    fn tls_info_clone() {
        let info = TlsInfo {
            peer_certificate: Some(bytes::Bytes::from_static(&[1, 2, 3])),
            peer_certificate_chain: None,
        };
        let info2 = info.clone();
        assert_eq!(info.peer_certificate(), info2.peer_certificate());
    }

    #[test]
    fn alps_protocol_constants() {
        assert_eq!(AlpsProtocol::HTTP1, AlpsProtocol(b"http/1.1"));
        assert_eq!(AlpsProtocol::HTTP2, AlpsProtocol(b"h2"));
        assert_eq!(AlpsProtocol::HTTP3, AlpsProtocol(b"h3"));
    }
}
