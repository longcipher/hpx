//! SSL support via OpenSSL.

#[macro_use]
mod macros;
mod cache;
mod ext;
mod service;

use std::{
    borrow::Cow,
    fmt::{self, Debug},
    io,
    pin::Pin,
    sync::{Arc, LazyLock},
    task::{Context, Poll},
};

use cache::{SessionCache, SessionKey};
use http::Uri;
use openssl::{
    error::ErrorStack,
    ex_data::Index,
    ssl::{Ssl, SslConnector, SslMethod, SslOptions, SslRef, SslSessionCacheMode},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_openssl::SslStream;
use tower::Service;

use self::ext::SslConnectorBuilderExt;
use crate::{
    Error,
    client::{ConnectIdentity, ConnectRequest, Connected, Connection},
    error::BoxError,
    tls::{AlpnProtocol, CertStore, Identity, KeyLog, TlsOptions, TlsVersion},
};

fn key_index() -> Result<Index<Ssl, SessionKey<ConnectIdentity>>, ErrorStack> {
    static IDX: LazyLock<Result<Index<Ssl, SessionKey<ConnectIdentity>>, ErrorStack>> =
        LazyLock::new(Ssl::new_ex_index);
    IDX.clone()
}

/// Builds for [`HandshakeConfig`].
pub struct HandshakeConfigBuilder {
    settings: HandshakeConfig,
}

/// Settings for [`TlsConnector`]
#[derive(Clone)]
pub struct HandshakeConfig {
    no_ticket: bool,
    verify_hostname: bool,
    tls_sni: bool,
    alpn_protocols: Option<Cow<'static, [AlpnProtocol]>>,
}

impl HandshakeConfigBuilder {
    /// Skips the session ticket.
    #[must_use]
    pub fn no_ticket(mut self, skip: bool) -> Self {
        self.settings.no_ticket = skip;
        self
    }

    /// Sets hostname verification.
    #[must_use]
    pub fn verify_hostname(mut self, verify: bool) -> Self {
        self.settings.verify_hostname = verify;
        self
    }

    /// Sets TLS SNI.
    #[must_use]
    pub fn tls_sni(mut self, sni: bool) -> Self {
        self.settings.tls_sni = sni;
        self
    }

    /// Sets ALPN protocols.
    #[must_use]
    pub fn alpn_protocols<P>(mut self, alpn_protocols: P) -> Self
    where
        P: Into<Option<Cow<'static, [AlpnProtocol]>>>,
    {
        self.settings.alpn_protocols = alpn_protocols.into();
        self
    }

    /// Builds the `HandshakeConfig`.
    #[must_use]
    pub fn build(self) -> HandshakeConfig {
        self.settings
    }
}

impl HandshakeConfig {
    /// Creates a new `HandshakeConfigBuilder`.
    pub fn builder() -> HandshakeConfigBuilder {
        HandshakeConfigBuilder {
            settings: HandshakeConfig::default(),
        }
    }
}

impl Default for HandshakeConfig {
    fn default() -> Self {
        Self {
            no_ticket: false,
            verify_hostname: true,
            tls_sni: true,
            alpn_protocols: None,
        }
    }
}

/// A Connector using OpenSSL to support `http` and `https` schemes.
#[derive(Clone)]
pub struct HttpsConnector<T> {
    http: T,
    inner: Inner,
}

#[derive(Clone)]
struct Inner {
    ssl: SslConnector,
    cache: Option<Arc<SessionCache<ConnectIdentity>>>,
    config: HandshakeConfig,
}

/// A builder for creating a `TlsConnector`.
#[derive(Clone)]
pub struct TlsConnectorBuilder {
    session_cache: Arc<SessionCache<ConnectIdentity>>,
    alpn_protocol: Option<AlpnProtocol>,
    max_version: Option<TlsVersion>,
    min_version: Option<TlsVersion>,
    tls_sni: bool,
    verify_hostname: bool,
    identity: Option<Identity>,
    cert_store: Option<CertStore>,
    cert_verification: bool,
    keylog: Option<KeyLog>,
}

/// A layer which wraps services in an `SslConnector`.
#[derive(Clone)]
pub struct TlsConnector {
    inner: Inner,
}

// ===== impl HttpsConnector =====

impl<S, T> HttpsConnector<S>
where
    S: Service<Uri, Response = T> + Send,
    S::Error: Into<BoxError>,
    S::Future: Unpin + Send + 'static,
    T: AsyncRead + AsyncWrite + Connection + Unpin + Debug + Sync + Send + 'static,
{
    /// Creates a new [`HttpsConnector`] with a given [`TlsConnector`].
    #[inline]
    pub fn with_connector(http: S, connector: TlsConnector) -> HttpsConnector<S> {
        HttpsConnector {
            http,
            inner: connector.inner,
        }
    }

    /// Disables ALPN negotiation.
    #[inline]
    pub fn no_alpn(&mut self) -> &mut Self {
        self.inner.config.alpn_protocols = None;
        self
    }
}

// ===== impl Inner =====

impl Inner {
    fn setup_ssl(&self, uri: Uri) -> Result<Ssl, BoxError> {
        let mut cfg = self.ssl.configure()?;
        cfg.set_use_server_name_indication(self.config.tls_sni);
        cfg.set_verify_hostname(self.config.verify_hostname);
        let host = uri.host().ok_or("URI missing host")?;
        let host = Self::normalize_host(host);
        let ssl = cfg.into_ssl(host)?;
        Ok(ssl)
    }

    fn setup_ssl2(&self, req: ConnectRequest) -> Result<Ssl, BoxError> {
        let mut cfg = self.ssl.configure()?;

        // Use server name indication
        cfg.set_use_server_name_indication(self.config.tls_sni);

        // Verify hostname
        cfg.set_verify_hostname(self.config.verify_hostname);

        // Set ALPN protocols
        if let Some(alpn) = req.extra().alpn_protocol() {
            // If ALPN is set in the request, it takes precedence over the connector configuration.
            cfg.set_alpn_protos(&alpn.encode())?;
        } else {
            // Default use the connector configuration.
            if let Some(ref alpn_values) = self.config.alpn_protocols {
                let encoded = AlpnProtocol::encode_sequence(alpn_values.as_ref());
                cfg.set_alpn_protos(&encoded)?;
            }
        }

        let uri = req.uri().clone();
        let host = uri.host().ok_or("URI missing host")?;
        let host = Self::normalize_host(host);

        if let Some(ref cache) = self.cache {
            let key = SessionKey(req.identify());

            // If the session cache is enabled, we try to retrieve the session
            // associated with the key. If it exists, we set it in the SSL configuration.
            if let Some(session) = cache.get(&key) {
                // Safety: `session` was obtained from a prior `set_new_session_callback`
                // invoked by OpenSSL on the same `SslContext`, so the session handle is
                // valid for the lifetime of the `SslConnectConfiguration`.
                #[allow(unsafe_code)]
                unsafe { cfg.set_session(&session) }?;
            }

            let idx = key_index()?;
            cfg.set_ex_data(idx, key);
        }

        let ssl = cfg.into_ssl(host)?;
        Ok(ssl)
    }

    /// If `host` is an IPv6 address, we must strip away the square brackets that surround
    /// it (otherwise, OpenSSL will fail to parse the host as an IP address, eventually
    /// causing the handshake to fail due a hostname verification error).
    fn normalize_host(host: &str) -> &str {
        if host.is_empty() {
            return host;
        }

        let last = host.len() - 1;
        let mut chars = host.chars();

        if let (Some('['), Some(']')) = (chars.next(), chars.last())
            && host[1..last].parse::<std::net::Ipv6Addr>().is_ok()
        {
            return &host[1..last];
        }

        host
    }
}

// ====== impl TlsConnectorBuilder =====

impl TlsConnectorBuilder {
    /// Sets the alpn protocol to be used.
    #[inline(always)]
    pub fn alpn_protocol(mut self, protocol: Option<AlpnProtocol>) -> Self {
        self.alpn_protocol = protocol;
        self
    }

    /// Sets the TLS keylog policy.
    #[inline(always)]
    pub fn keylog(mut self, keylog: Option<KeyLog>) -> Self {
        self.keylog = keylog;
        self
    }

    /// Sets the identity to be used for client certificate authentication.
    #[inline(always)]
    pub fn identity(mut self, identity: Option<Identity>) -> Self {
        self.identity = identity;
        self
    }

    /// Sets the certificate store used for TLS verification.
    #[inline(always)]
    pub fn cert_store<T>(mut self, cert_store: T) -> Self
    where
        T: Into<Option<CertStore>>,
    {
        self.cert_store = cert_store.into();
        self
    }

    /// Sets the certificate verification flag.
    #[inline(always)]
    pub fn cert_verification(mut self, enabled: bool) -> Self {
        self.cert_verification = enabled;
        self
    }

    /// Sets the minimum TLS version to use.
    #[inline(always)]
    pub fn min_version<T>(mut self, version: T) -> Self
    where
        T: Into<Option<TlsVersion>>,
    {
        self.min_version = version.into();
        self
    }

    /// Sets the maximum TLS version to use.
    #[inline(always)]
    pub fn max_version<T>(mut self, version: T) -> Self
    where
        T: Into<Option<TlsVersion>>,
    {
        self.max_version = version.into();
        self
    }

    /// Sets the Server Name Indication (SNI) flag.
    #[inline(always)]
    pub fn tls_sni(mut self, enabled: bool) -> Self {
        self.tls_sni = enabled;
        self
    }

    /// Sets the hostname verification flag.
    #[inline(always)]
    pub fn verify_hostname(mut self, enabled: bool) -> Self {
        self.verify_hostname = enabled;
        self
    }

    /// Build the `TlsConnector` with the provided configuration.
    pub fn build(&self, opts: &TlsOptions) -> crate::Result<TlsConnector> {
        // Replace the default configuration with the provided one
        let max_tls_version = opts.max_tls_version.or(self.max_version);
        let min_tls_version = opts.min_tls_version.or(self.min_version);
        let alpn_protocols = self
            .alpn_protocol
            .map(|proto| Cow::Owned(vec![proto]))
            .or_else(|| opts.alpn_protocols.clone());

        // Create the SslConnector with the provided options
        let mut connector = SslConnector::builder(SslMethod::tls())
            .map_err(Error::tls)?
            .configure_cert_store(self.cert_store.as_ref())?
            .set_cert_verification(self.cert_verification)?;

        // Set Identity
        if let Some(ref identity) = self.identity {
            identity.add_to_tls(&mut connector)?;
        }

        // Set minimum TLS version
        set_option_inner_try!(min_tls_version, connector, set_min_proto_version);

        // Set maximum TLS version
        set_option_inner_try!(max_tls_version, connector, set_max_proto_version);

        // Set TLS Session ticket options
        set_bool!(
            opts,
            !session_ticket,
            connector,
            set_options,
            SslOptions::NO_TICKET
        );

        // Set TLS No Renegotiation options
        set_bool!(
            opts,
            !renegotiation,
            connector,
            set_options,
            SslOptions::NO_RENEGOTIATION
        );

        // Set TLS curves list (openssl uses "groups" terminology)
        set_option_ref_try!(opts, curves_list, connector, set_groups_list);

        // Set TLS signature algorithms list
        set_option_ref_try!(opts, sigalgs_list, connector, set_sigalgs_list);

        // Set TLS cipher list
        set_option_ref_try!(opts, cipher_list, connector, set_cipher_list);

        // Set TLS keylog handler.
        if let Some(ref policy) = self.keylog {
            let handle = policy.clone().handle().map_err(Error::tls)?;
            connector.set_keylog_callback(move |_, line| {
                handle.write(line);
            });
        }

        // Create the handshake config with the default session cache capacity.
        let config = HandshakeConfig::builder()
            .no_ticket(opts.psk_skip_session_ticket)
            .alpn_protocols(alpn_protocols)
            .tls_sni(self.tls_sni)
            .verify_hostname(self.verify_hostname)
            .build();

        // If the session cache is disabled, we don't need to set up any callbacks.
        let cache = opts.pre_shared_key.then(|| {
            let cache = self.session_cache.clone();

            connector.set_session_cache_mode(SslSessionCacheMode::CLIENT);
            connector.set_new_session_callback({
                let cache = cache.clone();
                move |ssl: &mut SslRef, session| {
                    if let Ok(Some(key)) = key_index().map(|idx| ssl.ex_data(idx)) {
                        cache.insert(key.clone(), session);
                    }
                }
            });

            cache
        });

        Ok(TlsConnector {
            inner: Inner {
                ssl: connector.build(),
                cache,
                config,
            },
        })
    }
}

// ===== impl TlsConnector =====

impl TlsConnector {
    /// Creates a new `TlsConnectorBuilder` with the given configuration.
    pub fn builder() -> TlsConnectorBuilder {
        const DEFAULT_SESSION_CACHE_CAPACITY: usize = 8;
        TlsConnectorBuilder {
            session_cache: Arc::new(SessionCache::with_capacity(DEFAULT_SESSION_CACHE_CAPACITY)),
            alpn_protocol: None,
            min_version: None,
            max_version: None,
            identity: None,
            cert_store: None,
            cert_verification: true,
            tls_sni: true,
            verify_hostname: true,
            keylog: None,
        }
    }
}

/// A stream which may be wrapped with TLS.
pub enum MaybeHttpsStream<T> {
    /// A raw HTTP stream.
    Http(T),
    /// An SSL-wrapped HTTP stream.
    Https(SslStream<T>),
}

/// A connection that has been established with a TLS handshake.
pub struct EstablishedConn<IO> {
    io: IO,
    req: ConnectRequest,
}

// ===== impl MaybeHttpsStream =====

impl<T> MaybeHttpsStream<T> {
    /// Returns a reference to the underlying stream.
    #[inline]
    pub fn get_ref(&self) -> &T {
        match self {
            MaybeHttpsStream::Http(s) => s,
            MaybeHttpsStream::Https(s) => s.get_ref(),
        }
    }
}

impl<T> fmt::Debug for MaybeHttpsStream<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            MaybeHttpsStream::Http(..) => f.pad("Http(..)"),
            MaybeHttpsStream::Https(..) => f.pad("Https(..)"),
        }
    }
}

impl<T> Connection for MaybeHttpsStream<T>
where
    T: Connection,
{
    fn connected(&self) -> Connected {
        match self {
            MaybeHttpsStream::Http(s) => s.connected(),
            MaybeHttpsStream::Https(s) => {
                let mut connected = s.get_ref().connected();

                if s.ssl().selected_alpn_protocol() == Some(b"h2") {
                    connected = connected.negotiated_h2();
                }

                connected
            }
        }
    }
}

impl<T> AsyncRead for MaybeHttpsStream<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.as_mut().get_mut() {
            MaybeHttpsStream::Http(inner) => Pin::new(inner).poll_read(cx, buf),
            MaybeHttpsStream::Https(inner) => Pin::new(inner).poll_read(cx, buf),
        }
    }
}

impl<T> AsyncWrite for MaybeHttpsStream<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.as_mut().get_mut() {
            MaybeHttpsStream::Http(inner) => Pin::new(inner).poll_write(ctx, buf),
            MaybeHttpsStream::Https(inner) => Pin::new(inner).poll_write(ctx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.as_mut().get_mut() {
            MaybeHttpsStream::Http(inner) => Pin::new(inner).poll_flush(ctx),
            MaybeHttpsStream::Https(inner) => Pin::new(inner).poll_flush(ctx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.as_mut().get_mut() {
            MaybeHttpsStream::Http(inner) => Pin::new(inner).poll_shutdown(ctx),
            MaybeHttpsStream::Https(inner) => Pin::new(inner).poll_shutdown(ctx),
        }
    }
}

// ===== impl EstablishedConn =====

impl<IO> EstablishedConn<IO> {
    /// Creates a new [`EstablishedConn`].
    #[inline]
    pub fn new(io: IO, req: ConnectRequest) -> EstablishedConn<IO> {
        EstablishedConn { io, req }
    }
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, pin::Pin};

    use bytes::Bytes;
    use http_body_util::Full;
    use hyper::{Response, server::conn::http1, service::service_fn};
    use hyper_util::rt::TokioIo;
    use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod, SslVerifyMode};
    use tokio::net::TcpListener;

    use crate::{
        Client,
        tls::{CertStore, Identity},
    };

    const CA_CERT_PEM: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/mtls/ca.crt"
    ));
    const CLIENT_CERT_PEM: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/mtls/client.crt"
    ));
    const CLIENT_KEY_PEM: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/mtls/client.key"
    ));
    const SERVER_CERT_PATH: &str =
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mtls/server.crt");
    const SERVER_KEY_PATH: &str =
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mtls/server.key");
    const CA_CERT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mtls/ca.crt");

    fn tls_acceptor() -> SslAcceptor {
        let mut acceptor = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
        acceptor
            .set_certificate_chain_file(SERVER_CERT_PATH)
            .unwrap();
        acceptor
            .set_private_key_file(SERVER_KEY_PATH, SslFiletype::PEM)
            .unwrap();
        acceptor.set_ca_file(CA_CERT_PATH).unwrap();
        acceptor.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
        acceptor.check_private_key().unwrap();
        acceptor.build()
    }

    #[tokio::test]
    async fn combined_pem_identity_authenticates_with_mutual_tls() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let acceptor = tls_acceptor();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ssl = openssl::ssl::Ssl::new(acceptor.context()).unwrap();
            let stream = tokio_openssl::SslStream::new(ssl, stream).unwrap();
            let mut stream = stream;
            Pin::new(&mut stream).accept().await.unwrap();
            let service = service_fn(|_request| async {
                let mut response = Response::new(Full::new(Bytes::from_static(b"mtls-ok")));
                response.headers_mut().insert(
                    http::header::CONNECTION,
                    http::HeaderValue::from_static("close"),
                );
                Ok::<_, Infallible>(response)
            });

            http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await
                .unwrap();
        });

        let mut pem = CLIENT_CERT_PEM.to_vec();
        pem.extend_from_slice(CLIENT_KEY_PEM);

        let cert_store = CertStore::builder()
            .add_pem_cert(CA_CERT_PEM)
            .build()
            .unwrap();
        let identity = Identity::from_pem(&pem).unwrap();
        let client = Client::builder()
            .no_proxy()
            .cert_store(cert_store)
            .identity(identity)
            .build()
            .unwrap();

        let response = client
            .get(format!("https://localhost:{}/", addr.port()))
            .send()
            .await
            .unwrap();

        assert_eq!(response.text().await.unwrap(), "mtls-ok");
        server.await.unwrap();
    }
}
