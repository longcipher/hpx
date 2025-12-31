#![allow(unused)]
use std::{
    borrow::Cow,
    fmt::{self, Debug},
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use http::Uri;
use rustls::{
    ClientConfig, DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_rustls::{TlsConnector as RustlsConnector, client::TlsStream};
use tower::Service;

use crate::{
    client::{ConnectRequest, Connected, Connection},
    error::BoxError,
    tls::{AlpnProtocol, AlpsProtocol, CertStore, Identity, KeyLog, TlsOptions, TlsVersion},
};

/// Builds for [`HandshakeConfig`].
pub struct HandshakeConfigBuilder {
    settings: HandshakeConfig,
}

/// Settings for [`TlsConnector`]
#[derive(Clone, Default)]
pub struct HandshakeConfig {
    verify_hostname: bool,
    alpn_protocols: Option<Vec<Vec<u8>>>,
}

impl HandshakeConfigBuilder {
    /// Skips the session ticket.
    pub fn no_ticket(self, _skip: bool) -> Self {
        self
    }

    /// Enables or disables ECH grease.
    pub fn enable_ech_grease(self, _enable: bool) -> Self {
        self
    }

    /// Sets hostname verification.
    pub fn verify_hostname(mut self, verify: bool) -> Self {
        self.settings.verify_hostname = verify;
        self
    }

    /// Sets TLS SNI.
    pub fn tls_sni(self, _sni: bool) -> Self {
        self
    }

    /// Sets ALPN protocols.
    pub fn alpn_protocols<P>(mut self, alpn_protocols: P) -> Self
    where
        P: Into<Option<Cow<'static, [AlpnProtocol]>>>,
    {
        if let Some(protos) = alpn_protocols.into() {
            self.settings.alpn_protocols =
                Some(protos.iter().map(|p| p.encode().to_vec()).collect());
        } else {
            self.settings.alpn_protocols = None;
        }
        self
    }

    /// Sets ALPS protocol.
    pub fn alps_protocols<P>(self, _alps_protocols: P) -> Self
    where
        P: Into<Option<Cow<'static, [AlpsProtocol]>>>,
    {
        self
    }

    /// Sets ALPS new codepoint usage.
    pub fn alps_use_new_codepoint(self, _use_new: bool) -> Self {
        self
    }

    /// Sets random AES hardware override.
    pub fn random_aes_hw_override(self, _override_: bool) -> Self {
        self
    }

    /// Builds the `HandshakeConfig`.
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

/// A builder for creating a `TlsConnector`.
#[derive(Clone)]
pub struct TlsConnectorBuilder {
    alpn_protocol: Option<AlpnProtocol>,
    min_version: Option<TlsVersion>,
    max_version: Option<TlsVersion>,
    identity: Option<Identity>,
    cert_store: Option<CertStore>,
    cert_verification: bool,
    tls_sni: bool,
    verify_hostname: bool,
    keylog: Option<KeyLog>,
}

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
        let root_store = if let Some(store) = &self.cert_store {
            (*store.0).clone()
        } else {
            let mut root_store = rustls::RootCertStore::empty();
            root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            root_store
        };

        // ALPN
        let alpn_protocols = self
            .alpn_protocol
            .map(|proto| vec![proto.encode().to_vec()])
            .or_else(|| {
                opts.alpn_protocols
                    .as_ref()
                    .map(|protos| protos.iter().map(|p| p.encode().to_vec()).collect())
            });

        let create_config = |alpn: Option<Vec<Vec<u8>>>| {
            let mut config = ClientConfig::builder()
                .with_root_certificates(root_store.clone())
                .with_no_client_auth();

            if !self.cert_verification {
                config
                    .dangerous()
                    .set_certificate_verifier(Arc::new(NoVerifier));
            }

            if let Some(protos) = alpn {
                config.alpn_protocols = protos;
            }

            Arc::new(RustlsConnector::from(Arc::new(config)))
        };

        let connector = create_config(alpn_protocols);
        let connector_no_alpn = create_config(None);
        let connector_h2 = create_config(Some(vec![AlpnProtocol::HTTP2.encode().to_vec()]));
        let connector_http1 = create_config(Some(vec![AlpnProtocol::HTTP1.encode().to_vec()]));

        // TODO: Handle other options like min/max version, keylog, etc.

        Ok(TlsConnector {
            connector,
            connector_h2,
            connector_http1,
            connector_no_alpn,
            config: HandshakeConfig::default(), // TODO: populate from builder
        })
    }
}

/// A layer which wraps services in an `SslConnector`.
#[derive(Clone)]
pub struct TlsConnector {
    connector: Arc<RustlsConnector>,
    connector_h2: Arc<RustlsConnector>,
    connector_http1: Arc<RustlsConnector>,
    connector_no_alpn: Arc<RustlsConnector>,
    config: HandshakeConfig,
}

impl TlsConnector {
    /// Creates a new `TlsConnectorBuilder` with the given configuration.
    pub fn builder() -> TlsConnectorBuilder {
        TlsConnectorBuilder {
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

/// A Connector using Rustls to support `http` and `https` schemes.
#[derive(Clone)]
pub struct HttpsConnector<T> {
    http: T,
    connector: Arc<RustlsConnector>,
    connector_h2: Arc<RustlsConnector>,
    connector_http1: Arc<RustlsConnector>,
    connector_no_alpn: Arc<RustlsConnector>,
    config: HandshakeConfig,
    forced_no_alpn: bool,
}

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
            connector: connector.connector,
            connector_h2: connector.connector_h2,
            connector_http1: connector.connector_http1,
            connector_no_alpn: connector.connector_no_alpn,
            config: connector.config,
            forced_no_alpn: false,
        }
    }

    /// Disables ALPN negotiation.
    #[inline]
    pub fn no_alpn(&mut self) -> &mut Self {
        self.config.alpn_protocols = None;
        self.forced_no_alpn = true;
        self
    }
}

impl<S, T> Service<Uri> for HttpsConnector<S>
where
    S: Service<Uri, Response = T> + Send,
    S::Error: Into<BoxError>,
    S::Future: Unpin + Send + 'static,
    T: AsyncRead + AsyncWrite + Connection + Unpin + Debug + Sync + Send + 'static,
{
    type Response = MaybeHttpsStream<T>;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.http.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let connect = self.http.call(uri.clone());
        let connector = self.connector.clone();
        // let config = self.config.clone();

        Box::pin(async move {
            let conn = connect.await.map_err(Into::into)?;

            // Perform TLS handshake
            let domain = uri.host().ok_or("URI missing host")?;
            let server_name = rustls_pki_types::ServerName::try_from(domain)
                .map_err(|e| Box::new(e) as BoxError)?;

            let stream = connector
                .connect(server_name.to_owned(), conn)
                .await
                .map_err(|e| Box::new(e) as BoxError)?;

            Ok(MaybeHttpsStream::Https(stream))
        })
    }
}

impl<S, T> Service<ConnectRequest> for HttpsConnector<S>
where
    S: Service<Uri, Response = T> + Send,
    S::Error: Into<BoxError>,
    S::Future: Unpin + Send + 'static,
    T: AsyncRead + AsyncWrite + Connection + Unpin + Debug + Sync + Send + 'static,
{
    type Response = MaybeHttpsStream<T>;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.http.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: ConnectRequest) -> Self::Future {
        let is_https = req.uri().scheme_str() == Some("https");
        let uri = req.uri().clone();

        if !is_https {
            let connect = self.http.call(uri);
            return Box::pin(async move {
                let conn = connect.await.map_err(Into::into)?;
                Ok(MaybeHttpsStream::Http(conn))
            });
        }

        let connect = self.http.call(uri.clone());

        let connector = if self.forced_no_alpn {
            self.connector_no_alpn.clone()
        } else if let Some(alpn) = req.extra().alpn_protocol() {
            if alpn == AlpnProtocol::HTTP2 {
                self.connector_h2.clone()
            } else if alpn == AlpnProtocol::HTTP1 {
                self.connector_http1.clone()
            } else {
                self.connector.clone()
            }
        } else {
            self.connector.clone()
        };

        Box::pin(async move {
            let conn = connect.await.map_err(Into::into)?;

            // Perform TLS handshake
            let domain = uri.host().ok_or("URI missing host")?;
            let server_name = rustls_pki_types::ServerName::try_from(domain)
                .map_err(|e| Box::new(e) as BoxError)?;

            let stream = connector
                .connect(server_name.to_owned(), conn)
                .await
                .map_err(|e| Box::new(e) as BoxError)?;

            Ok(MaybeHttpsStream::Https(stream))
        })
    }
}

/// A stream which may be wrapped with TLS.
pub enum MaybeHttpsStream<T> {
    /// A raw HTTP stream.
    Http(T),
    /// An SSL-wrapped HTTP stream.
    Https(TlsStream<T>),
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
            MaybeHttpsStream::Https(s) => s.get_ref().0,
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
                let (io, session) = s.get_ref();
                let mut connected = io.connected();

                if session.alpn_protocol() == Some(b"h2") {
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

    pub fn into_parts(self) -> (IO, ConnectRequest) {
        (self.io, self.req)
    }
}

impl<S, T> Service<EstablishedConn<T>> for HttpsConnector<S>
where
    S: Service<Uri> + Send,
    S::Error: Into<BoxError>,
    S::Future: Unpin + Send + 'static,
    T: AsyncRead + AsyncWrite + Connection + Unpin + Debug + Sync + Send + 'static,
{
    type Response = MaybeHttpsStream<T>;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: EstablishedConn<T>) -> Self::Future {
        let (conn, connect_req) = req.into_parts();
        let uri = connect_req.uri().clone();
        let connector = self.connector.clone();

        Box::pin(async move {
            let domain = uri.host().ok_or("URI missing host")?;
            let server_name = rustls_pki_types::ServerName::try_from(domain)
                .map_err(|e| Box::new(e) as BoxError)?;

            let stream = connector
                .connect(server_name.to_owned(), conn)
                .await
                .map_err(|e| Box::new(e) as BoxError)?;

            Ok(MaybeHttpsStream::Https(stream))
        })
    }
}

#[derive(Debug)]
struct NoVerifier;

impl ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA1,
            SignatureScheme::ECDSA_SHA1_Legacy,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
            SignatureScheme::ED448,
        ]
    }
}
