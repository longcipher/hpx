use std::sync::Arc;

use http::{Uri, Version};

use crate::{
    client::{
        conn::TcpConnectOptions,
        layer::config::{RequestOptions, TransportOptions},
    },
    hash::HashMemo,
    proxy::Matcher as ProxyMacher,
    tls::{AlpnProtocol, TlsOptions},
};

/// Unique identity for a reusable connection.
pub(crate) type ConnectIdentity = Arc<HashMemo<ConnectExtra>>;

/// Metadata describing a reusable network connection.
///
/// [`ConnectExtra`] holds connection-specific parameters such as the target URI, ALPN protocol,
/// proxy settings, and optional TCP/TLS options. Used for connection
#[must_use]
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub(crate) struct ConnectExtra {
    uri: Uri,
    extra: Option<RequestOptions>,
}

impl ConnectExtra {
    /// Create a new [`ConnectExtra`] with the given URI and extra.
    #[inline]
    pub(super) fn new<T>(uri: Uri, extra: T) -> Self
    where
        T: Into<Option<RequestOptions>>,
    {
        Self {
            uri,
            extra: extra.into(),
        }
    }

    /// Return the negotiated [`AlpnProtocol`].
    pub fn alpn_protocol(&self) -> Option<AlpnProtocol> {
        match self
            .extra
            .as_ref()
            .and_then(RequestOptions::enforced_version)
        {
            Some(Version::HTTP_11 | Version::HTTP_10 | Version::HTTP_09) => {
                Some(AlpnProtocol::HTTP1)
            }
            Some(Version::HTTP_2) => Some(AlpnProtocol::HTTP2),
            _ => None,
        }
    }

    /// Return a reference to the [`ProxyMacher`].
    #[inline]
    pub fn proxy_matcher(&self) -> Option<&Arc<ProxyMacher>> {
        self.extra.as_ref().and_then(RequestOptions::proxy_matcher)
    }

    /// Return a reference to the [`TlsOptions`].
    #[inline]
    pub fn tls_options(&self) -> Option<&TlsOptions> {
        self.extra
            .as_ref()
            .map(RequestOptions::transport_opts)
            .and_then(TransportOptions::tls_options)
    }

    /// Return a reference to the [`TcpConnectOptions`].
    #[inline]
    pub fn tcp_options(&self) -> Option<&TcpConnectOptions> {
        self.extra.as_ref().map(RequestOptions::tcp_connect_opts)
    }
}

#[cfg(test)]
mod tests {
    use std::hash::Hash;

    use super::*;
    use crate::{hash::HASHER, tls::TlsOptions};

    fn hash_of<T: Hash>(v: &T) -> u64 {
        HASHER.hash_one(v)
    }

    #[test]
    fn same_uri_same_options_produce_same_identity() {
        let uri: Uri = "https://example.com".parse().unwrap();
        let a = ConnectExtra::new(uri.clone(), None::<RequestOptions>);
        let b = ConnectExtra::new(uri, None::<RequestOptions>);
        assert_eq!(hash_of(&a), hash_of(&b));
        assert_eq!(a, b);
    }

    #[test]
    fn same_uri_different_tls_options_produce_different_identity() {
        let uri: Uri = "https://example.com".parse().unwrap();

        let mut opts_a = RequestOptions::default();
        opts_a.transport_opts_mut().tls_options = Some(
            TlsOptions::builder()
                .cipher_list("ECDHE-RSA-AES128-GCM-SHA256")
                .build(),
        );

        let mut opts_b = RequestOptions::default();
        opts_b.transport_opts_mut().tls_options = Some(
            TlsOptions::builder()
                .cipher_list("ECDHE-RSA-AES256-GCM-SHA384")
                .build(),
        );

        let a = ConnectExtra::new(uri.clone(), Some(opts_a));
        let b = ConnectExtra::new(uri, Some(opts_b));
        assert_ne!(hash_of(&a), hash_of(&b));
        assert_ne!(a, b);
    }
}
