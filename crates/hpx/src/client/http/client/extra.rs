use std::sync::Arc;

use http::{Uri, Version};

use crate::{
    client::{
        conn::TcpConnectOptions,
        layer::config::{ProxySelection, RequestOptions, TransportOptions},
    },
    hash::HashMemo,
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

/// Strips path/query from a URI so it can be used as a connection-pool key.
///
/// Keeps only `scheme` and `authority` (`host[:port]`) so that every request
/// to the same origin shares the pooled connection(s) — critical for HTTP/2
/// multiplexing and for avoiding per-path connection re-establishment on
/// HTTP/1. Falls back to the original URI if the scheme/authority cannot be
/// preserved (e.g. non-http schemes such as Unix sockets).
#[inline]
#[cfg_attr(feature = "hotpath", hotpath::measure)]
fn normalize_pool_key_uri(uri: &Uri) -> Uri {
    // Fast path: `HttpClient::request` already builds the pool key from a
    // canonical `scheme://authority/` URI (see `util::normalize_uri`), so the
    // overwhelmingly common case is already normalized. `Uri::from_parts`
    // re-validates and re-allocates the serialized `ByteStr`, which is pure
    // per-request waste — skip it when there is nothing to rewrite.
    if matches!(uri.path_and_query(), Some(path) if path.as_str() == "/") {
        return uri.clone();
    }

    // `Uri::from_parts` requires a path when scheme/authority is present, so
    // normalize every origin to a single canonical "/" path — the key then
    // collapses all paths/queries on the same host into one pooled entry.
    let mut parts = uri.clone().into_parts();
    parts.path_and_query = Some(http::uri::PathAndQuery::from_static("/"));
    match Uri::from_parts(parts) {
        Ok(normalized) => normalized,
        Err(_) => uri.clone(),
    }
}

impl ConnectExtra {
    /// Create a new [`ConnectExtra`] with the given URI and extra.
    #[inline]
    #[cfg_attr(feature = "hotpath", hotpath::measure(impl_type = "ConnectExtra"))]
    pub(super) fn new<T>(uri: Uri, extra: T) -> Self
    where
        T: Into<Option<RequestOptions>>,
    {
        // Low-latency optimization: normalize the connection-pool key to
        // scheme + authority only (strip path/query). Without this, every
        // distinct REST path/query produces a different pool key and forces a
        // brand-new DNS + TCP + TLS connection, which dominates TTFB for
        // short-lived API calls. The full URI (with path) is retained by
        // `ConnectRequest::uri` for the actual request routing; here it is
        // only used for hashing/sharing pooled connections.
        let uri = normalize_pool_key_uri(&uri);
        Self {
            uri,
            extra: extra.into(),
        }
    }

    /// Return the negotiated [`AlpnProtocol`].
    pub(crate) fn alpn_protocol(&self) -> Option<AlpnProtocol> {
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

    /// Return the proxy selection for this connection.
    #[inline]
    pub(crate) fn proxy_selection(&self) -> &ProxySelection {
        self.extra
            .as_ref()
            .map_or(&ProxySelection::Inherit, RequestOptions::proxy)
    }

    /// Return a reference to the [`TlsOptions`].
    #[inline]
    pub(crate) fn tls_options(&self) -> Option<&TlsOptions> {
        self.extra
            .as_ref()
            .map(RequestOptions::transport_opts)
            .and_then(TransportOptions::tls_options)
    }

    /// Return a reference to the [`TcpConnectOptions`].
    #[inline]
    pub(crate) fn tcp_options(&self) -> Option<&TcpConnectOptions> {
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

    #[test]
    fn different_paths_on_same_origin_share_pool_identity() {
        // Pool key is normalized to scheme+authority: distinct paths/queries on
        // the same host must yield the same identity so the connection is reused.
        let a: Uri = "https://example.com/v1/order/place".parse().unwrap();
        let b: Uri = "https://example.com/v1/account/balance?x=1"
            .parse()
            .unwrap();
        let c: Uri = "https://example.com".parse().unwrap();

        let ia = ConnectExtra::new(a, None::<RequestOptions>);
        let ib = ConnectExtra::new(b, None::<RequestOptions>);
        let ic = ConnectExtra::new(c, None::<RequestOptions>);
        assert_eq!(ia, ib);
        assert_eq!(ia, ic);
    }

    #[test]
    fn different_origins_produce_different_identity() {
        let a: Uri = "https://api.example.com/v1/order/place".parse().unwrap();
        let b: Uri = "https://alt.example.com/v1/order/place".parse().unwrap();

        let ia = ConnectExtra::new(a, None::<RequestOptions>);
        let ib = ConnectExtra::new(b, None::<RequestOptions>);
        assert_ne!(ia, ib);
    }

    /// Two requests that select different proxies must not share a pooled
    /// connection: the connection is established through a different peer.
    #[test]
    fn different_proxy_selections_produce_different_identity() {
        let uri: Uri = "https://example.com".parse().unwrap();

        let direct = RequestOptions::default();
        assert_eq!(direct.proxy(), &ProxySelection::Inherit);

        let mut proxied = RequestOptions::default();
        proxied
            .proxy_mut()
            .clone_from(&ProxySelection::Rule(Arc::new(
                crate::Proxy::all("http://proxy.local:8080")
                    .unwrap()
                    .into_matcher(),
            )));

        let id = ConnectExtra::new(uri.clone(), None::<RequestOptions>);
        let ip = ConnectExtra::new(uri, Some(proxied));

        assert_ne!(id, ip);
        assert_ne!(hash_of(&id), hash_of(&ip));
    }

    /// The same custom proxy rule produces the same pool identity, so the
    /// connection is reused across requests.
    #[test]
    fn the_same_custom_proxy_produces_the_same_identity() {
        let uri: Uri = "https://example.com".parse().unwrap();

        // A single proxy instance, cloned: clones share the identity of the
        // closure, so they hash and compare equal.
        let proxy = crate::Proxy::custom(|_: &http::Uri| Some("http://proxy.local:8080"));
        let matcher = Arc::new(proxy.clone().into_matcher());

        let build = || {
            let mut options = RequestOptions::default();
            options
                .proxy_mut()
                .clone_from(&ProxySelection::Rule(matcher.clone()));
            options
        };

        let a = ConnectExtra::new(uri.clone(), Some(build()));
        let b = ConnectExtra::new(uri, Some(build()));

        assert_eq!(a, b);
        assert_eq!(hash_of(&a), hash_of(&b));
    }
}
