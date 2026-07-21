pub mod client;
mod config_groups;
pub mod future;

mod builder;

use std::{
    borrow::Cow,
    collections::HashMap,
    future::Future,
    net::SocketAddr,
    num::NonZeroU32,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use http::header::HeaderMap;
use tower::{
    retry::Retry,
    util::{BoxCloneSyncService, BoxCloneSyncServiceLayer, MapErr, Oneshot},
};

#[cfg(any(feature = "boring-tls", feature = "openssl-tls"))]
pub(crate) use self::client::extra::ConnectIdentity;
pub(crate) use self::client::{ConnectRequest, HttpClient, extra::ConnectExtra};
pub use self::config_groups::{
    HttpVersionPreference, PoolConfigOptions, ProtocolConfigOptions, ProxyConfigOptions,
    TlsConfigOptions, TransportConfigOptions,
};
use self::future::Pending;
#[cfg(any(
    feature = "gzip",
    feature = "zstd",
    feature = "brotli",
    feature = "deflate",
))]
use super::layer::decoder::AcceptEncoding;
#[cfg(feature = "ws-yawc")]
use super::ws::WebSocketRequestBuilder;
use super::{
    Body,
    conn::{BoxedConnectorLayer, Connector, TcpConnectOptions},
    core::body::Incoming,
    layer::{
        config::{ConfigService, TransportOptions},
        recovery::{Recoveries, ResponseRecovery},
        redirect::FollowRedirect,
        retry::RetryPolicy,
        timeout::{ResponseBodyTimeout, Timeout, TimeoutBody, TimeoutOptions},
    },
    request::{Request, RequestBuilder},
    response::Response,
};
#[cfg(feature = "http3")]
use crate::client::conn::alt_svc::{AltSvcCache, H3FailureTracker};
#[cfg(feature = "cookies")]
use crate::cookie;
#[cfg(feature = "http3")]
use crate::http3::{Http3Options, QuicConfig};
use crate::{
    IntoUri, Method, Proxy,
    dns::Resolve,
    error::{BoxError, Error},
    header::OrigHeaderMap,
    proxy::Matcher as ProxyMatcher,
    redirect::{self, FollowRedirectPolicy},
    retry,
    tls::{CertStore, Identity, KeyLog, TlsVersion},
};

/// Service type for cookie handling. Identity type when cookies feature is disabled.
#[cfg(not(feature = "cookies"))]
type CookieService<T> = T;

/// Service wrapper that handles cookie storage and injection.
#[cfg(feature = "cookies")]
type CookieService<T> = super::layer::cookie::CookieService<T>;

/// Decompression service type. Identity type when compression features are disabled.
#[cfg(not(any(
    feature = "gzip",
    feature = "zstd",
    feature = "brotli",
    feature = "deflate"
)))]
type Decompression<T> = T;

/// Service wrapper that handles response body decompression.
#[cfg(any(
    feature = "gzip",
    feature = "zstd",
    feature = "brotli",
    feature = "deflate"
))]
type Decompression<T> = super::layer::decoder::Decompression<T>;

/// Response body type with timeout and optional decompression.
#[cfg(any(
    feature = "gzip",
    feature = "zstd",
    feature = "brotli",
    feature = "deflate"
))]
pub(crate) type InnerResponseBody =
    TimeoutBody<tower_http::decompression::DecompressionBody<Incoming>>;

/// Response body type with timeout only (no compression features).
#[cfg(not(any(
    feature = "gzip",
    feature = "zstd",
    feature = "brotli",
    feature = "deflate"
)))]
pub(crate) type InnerResponseBody = TimeoutBody<Incoming>;

/// The complete HTTP client service stack before outer timeout decoration.
type BaseClientService = ResponseBodyTimeout<
    ConfigService<
        Decompression<
            Retry<
                RetryPolicy,
                FollowRedirect<
                    CookieService<
                        MapErr<HttpClient<Connector, Body>, fn(client::error::Error) -> BoxError>,
                    >,
                    FollowRedirectPolicy,
                >,
            >,
        >,
    >,
>;

/// The complete HTTP client service stack with all middleware layers.
pub type ClientService = Timeout<ResponseRecovery<BaseClientService>>;

/// Hooks-enabled client service path that remains statically dispatched.
type HookedClientService =
    Timeout<super::layer::hooks::HooksService<ResponseRecovery<BaseClientService>>>;

/// Type-erased client service for dynamic middleware composition.
pub type BoxedClientService =
    BoxCloneSyncService<http::Request<Body>, http::Response<super::ClientResponseBody>, BoxError>;

/// Layer type for wrapping boxed client services with additional middleware.
type BoxedClientLayer = BoxCloneSyncServiceLayer<
    BoxedClientService,
    http::Request<Body>,
    http::Response<super::ClientResponseBody>,
    BoxError,
>;

/// Client reference type that can be either a typed service path or a boxed service.
///
/// This enum replaces the previous nested `Either` type for better readability
/// and debug output.
pub(crate) enum ClientRef {
    /// Standard service path (no hooks, no custom layers).
    Standard(ClientService),
    /// Hooks-enabled service path (hooks but no custom layers).
    Hooked(HookedClientService),
    /// Type-erased service path (custom layers, with or without hooks).
    Boxed(BoxedClientService),
}

impl Clone for ClientRef {
    fn clone(&self) -> Self {
        match self {
            Self::Standard(s) => Self::Standard(s.clone()),
            Self::Hooked(s) => Self::Hooked(s.clone()),
            Self::Boxed(s) => Self::Boxed(s.clone()),
        }
    }
}

impl tower::Service<http::Request<Body>> for ClientRef {
    type Response = http::Response<super::ClientResponseBody>;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self {
            Self::Standard(s) => s.poll_ready(cx),
            Self::Hooked(s) => s.poll_ready(cx),
            Self::Boxed(s) => s.poll_ready(cx),
        }
    }

    fn call(&mut self, req: http::Request<Body>) -> Self::Future {
        match self {
            Self::Standard(s) => Box::pin(s.call(req)),
            Self::Hooked(s) => Box::pin(s.call(req)),
            Self::Boxed(s) => Box::pin(s.call(req)),
        }
    }
}

/// An [`Client`] to make Requests with.
///
/// The Client has various configuration values to tweak, but the defaults
/// are set to what is usually the most commonly desired value. To configure a
/// [`Client`], use [`Client::builder()`].
///
/// The [`Client`] holds a connection pool internally, so it is advised that
/// you create one and **reuse** it.
///
/// You do **not** have to wrap the [`Client`] in an [`Rc`] or [`Arc`] to **reuse** it,
/// because it already uses an [`Arc`] internally.
///
/// [`Rc`]: std::rc::Rc
pub struct Client {
    inner: Arc<ClientRef>,
    /// RFC 7838 Alt-Svc cache shared with the inner `HttpClient`.
    /// Populated from `alt-svc` response headers; used by the HTTP/3
    /// upgrade path to discover QUIC endpoints.
    #[cfg(feature = "http3")]
    alt_svc_cache: Arc<AltSvcCache>,
    /// Circuit breaker for HTTP/3 connection failures. Tracks per-authority
    /// QUIC handshake failures and blocks h3 retries for a cooldown period
    /// (default 60s). Shared across `Client` clones via `Arc`.
    #[cfg(feature = "http3")]
    h3_failure_tracker: Arc<H3FailureTracker>,
}

impl Clone for Client {
    fn clone(&self) -> Self {
        Client {
            inner: self.inner.clone(),
            #[cfg(feature = "http3")]
            alt_svc_cache: self.alt_svc_cache.clone(),
            #[cfg(feature = "http3")]
            h3_failure_tracker: self.h3_failure_tracker.clone(),
        }
    }
}

/// A [`ClientBuilder`] can be used to create a [`Client`] with custom configuration.
#[must_use]
pub struct ClientBuilder {
    config: CoreConfig,
}

/// The HTTP version preference for the client.
#[repr(u8)]
#[derive(Clone, Debug)]
enum HttpVersionPref {
    Http1,
    Http2,
    /// HTTP/3 over QUIC. The actual h3 transport path (QuicConnector + h3
    /// pool shard) is wired in `ClientBuilder::build`; until then, this
    /// variant exists so downstream builder methods (`http3_only()`) can
    /// set it.
    ///
    /// Per constraint C-06, the TCP ALPN list for `Http3` is empty — h3 is
    /// QUIC-only and is never proposed over TCP TLS. Alt-Svc (RFC 7838,
    /// Phase 2) is the only h3 discovery mechanism over TCP responses.
    #[cfg_attr(not(feature = "http3"), allow(dead_code))]
    Http3,
    /// Propose `["h2", "http/1.1"]` over TCP TLS. `All` does NOT
    /// automatically attempt h3 (Alt-Svc upgrade is Phase 2 scope).
    /// The TCP ALPN list does NOT include `h3` (C-06).
    All,
}

/// Transport-layer configuration.
#[derive(Clone)]
struct TransportConfig {
    connect_timeout: Option<Duration>,
    connection_verbose: bool,
    transport_options: TransportOptions,
    tcp_nodelay: bool,
    tcp_reuse_address: bool,
    tcp_keepalive: Option<Duration>,
    tcp_keepalive_interval: Option<Duration>,
    tcp_keepalive_retries: Option<u32>,
    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    tcp_user_timeout: Option<Duration>,
    tcp_send_buffer_size: Option<usize>,
    tcp_recv_buffer_size: Option<usize>,
    tcp_happy_eyeballs_timeout: Option<Duration>,
    tcp_connect_options: TcpConnectOptions,
}

/// Connection pool configuration
#[derive(Clone)]
struct PoolConfig {
    idle_timeout: Option<Duration>,
    max_idle_per_host: usize,
    max_size: Option<NonZeroU32>,
}

/// TLS configuration
#[derive(Clone)]
struct TlsConfig {
    keylog: Option<KeyLog>,
    tls_info: bool,
    tls_sni: bool,
    verify_hostname: bool,
    identity: Option<Identity>,
    cert_store: CertStore,
    cert_verification: bool,
    min_version: Option<TlsVersion>,
    max_version: Option<TlsVersion>,
}

/// HTTP protocol configuration
#[derive(Clone)]
struct ProtocolConfig {
    http_version_pref: HttpVersionPref,
    https_only: bool,
    retry_policy: retry::Policy,
    redirect_policy: redirect::Policy,
    referer: bool,
    timeout_options: TimeoutOptions,
    recoveries: Recoveries,
}

/// Proxy configuration
#[derive(Clone)]
struct ProxyConfig {
    proxies: Vec<ProxyMatcher>,
    auto_sys_proxy: bool,
}

/// DNS configuration
#[derive(Clone)]
struct DnsConfig {
    #[cfg(feature = "hickory-dns")]
    hickory_dns: bool,
    dns_overrides: HashMap<Cow<'static, str>, Vec<SocketAddr>>,
    dns_resolver: Option<Arc<dyn Resolve>>,
}

/// Middleware and hooks configuration
#[derive(Clone)]
struct MiddlewareConfig {
    #[cfg(any(
        feature = "gzip",
        feature = "zstd",
        feature = "brotli",
        feature = "deflate",
    ))]
    accept_encoding: AcceptEncoding,
    #[cfg(feature = "cookies")]
    cookie_store: Option<Arc<dyn cookie::CookieStore>>,
    layers: Vec<BoxedClientLayer>,
    connector_layers: Vec<BoxedConnectorLayer>,
    hooks: Option<super::layer::hooks::Hooks>,
}

/// Layered root configuration for [`ClientBuilder`].
struct CoreConfig {
    headers: HeaderMap,
    orig_headers: OrigHeaderMap,
    transport: TransportConfig,
    pool: PoolConfig,
    tls: TlsConfig,
    protocol: ProtocolConfig,
    proxy: ProxyConfig,
    dns: DnsConfig,
    middleware: MiddlewareConfig,
    /// Deferred user-agent value that hasn't been validated yet.
    /// Converted to HeaderValue at build time so the builder stays infallible.
    pending_user_agent: Option<Cow<'static, str>>,
    /// HTTP/3 (QUIC) configuration. Gated on the `http3` Cargo feature.
    ///
    /// `http3_options` stores the user-supplied [`Http3Options`] (or `None`
    /// if the user did not call [`ClientBuilder::http3_options`]). When
    /// `None`, `Client::build` falls back to [`Http3Options::default`].
    ///
    /// `quic_config` stores an optional low-level [`QuicConfig`] override (a
    /// type alias for `quinn::TransportConfig`). When `Some`, the override
    /// takes precedence over the transport config derived from `http3_options`.
    #[cfg(feature = "http3")]
    http3_options: Option<Http3Options>,
    #[cfg(feature = "http3")]
    quic_config: Option<QuicConfig>,
    /// Test-only escape hatch for injecting a pre-constructed `QuicConnector`
    /// (e.g., with a custom root store trusting a self-signed cert). When
    /// `Some`, `Client::build` passes it through to
    /// [`HttpClient::Builder::h3_connector`] verbatim, bypassing the normal
    /// `QuicConnector` construction from `http3_options` / `quic_config`.
    /// `#[doc(hidden)]` escape hatch — not part of the stable public API.
    #[cfg(feature = "http3")]
    test_quic_connector: Option<crate::client::conn::quic::QuicConnector>,
}

impl CoreConfig {
    fn sync_connect_timeout(&mut self) {
        self.protocol
            .timeout_options
            .timeout_connect(self.transport.connect_timeout);
    }
}

impl From<HttpVersionPreference> for HttpVersionPref {
    fn from(value: HttpVersionPreference) -> Self {
        match value {
            HttpVersionPreference::Http1 => Self::Http1,
            HttpVersionPreference::Http2 => Self::Http2,
            HttpVersionPreference::All => Self::All,
        }
    }
}

impl TransportConfig {
    fn with_transport_options(mut self, transport_options: TransportOptions) -> Self {
        self.transport_options = transport_options;
        self
    }
}

#[allow(deprecated)]
impl From<TransportConfigOptions> for TransportConfig {
    fn from(value: TransportConfigOptions) -> Self {
        Self {
            connect_timeout: value.connect_timeout,
            connection_verbose: value.connection_verbose,
            transport_options: TransportOptions::default(),
            tcp_nodelay: value.tcp_nodelay,
            tcp_reuse_address: value.tcp_reuse_address,
            tcp_keepalive: value.tcp_keepalive,
            tcp_keepalive_interval: value.tcp_keepalive_interval,
            tcp_keepalive_retries: value.tcp_keepalive_retries,
            #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
            tcp_user_timeout: value.tcp_user_timeout,
            tcp_send_buffer_size: value.tcp_send_buffer_size,
            tcp_recv_buffer_size: value.tcp_recv_buffer_size,
            tcp_happy_eyeballs_timeout: value.tcp_happy_eyeballs_timeout,
            tcp_connect_options: value.tcp_connect_options,
        }
    }
}

#[allow(deprecated)]
impl From<PoolConfigOptions> for PoolConfig {
    fn from(value: PoolConfigOptions) -> Self {
        Self {
            idle_timeout: value.idle_timeout,
            max_idle_per_host: value.max_idle_per_host,
            max_size: value.max_size,
        }
    }
}

#[allow(deprecated)]
impl From<TlsConfigOptions> for TlsConfig {
    fn from(value: TlsConfigOptions) -> Self {
        Self {
            keylog: value.keylog,
            tls_info: value.tls_info,
            tls_sni: value.tls_sni,
            verify_hostname: value.verify_hostname,
            identity: value.identity,
            cert_store: value.cert_store,
            cert_verification: value.cert_verification,
            min_version: value.min_version,
            max_version: value.max_version,
        }
    }
}

#[allow(deprecated)]
impl From<ProtocolConfigOptions> for ProtocolConfig {
    fn from(value: ProtocolConfigOptions) -> Self {
        Self {
            http_version_pref: value.http_version_preference.into(),
            https_only: value.https_only,
            retry_policy: value.retry_policy,
            redirect_policy: value.redirect_policy,
            referer: value.referer,
            timeout_options: value.timeout_options,
            recoveries: value.recoveries,
        }
    }
}

#[allow(deprecated)]
impl From<ProxyConfigOptions> for ProxyConfig {
    fn from(value: ProxyConfigOptions) -> Self {
        Self {
            proxies: value.proxies.into_iter().map(Proxy::into_matcher).collect(),
            auto_sys_proxy: value.auto_system_proxy,
        }
    }
}

// ===== impl Client =====

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    /// Constructs a new [`Client`].
    ///
    /// # Panics
    ///
    /// This method panics if a TLS backend cannot be initialized, or the resolver
    /// cannot load the system configuration.
    ///
    /// Use [`Client::builder()`] if you wish to handle the failure as an [`Error`]
    /// instead of panicking.
    #[inline]
    pub fn new() -> Client {
        Client::builder().build().expect(
            "Client::new() failed to build — use Client::builder().build() for error handling",
        )
    }

    /// Creates a [`ClientBuilder`] to configure a [`Client`].
    pub fn builder() -> ClientBuilder {
        ClientBuilder {
            config: CoreConfig {
                headers: HeaderMap::new(),
                orig_headers: OrigHeaderMap::new(),
                transport: TransportConfig {
                    connect_timeout: None,
                    connection_verbose: false,
                    transport_options: TransportOptions::default(),
                    tcp_nodelay: true,
                    tcp_reuse_address: false,
                    tcp_keepalive: Some(Duration::from_secs(15)),
                    tcp_keepalive_interval: Some(Duration::from_secs(15)),
                    tcp_keepalive_retries: Some(3),
                    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
                    tcp_user_timeout: Some(Duration::from_secs(30)),
                    tcp_connect_options: TcpConnectOptions::default(),
                    tcp_send_buffer_size: None,
                    tcp_recv_buffer_size: None,
                    tcp_happy_eyeballs_timeout: Some(Duration::from_millis(300)),
                },
                pool: PoolConfig {
                    idle_timeout: Some(Duration::from_secs(90)),
                    max_idle_per_host: usize::MAX,
                    max_size: None,
                },
                tls: TlsConfig {
                    keylog: None,
                    tls_info: false,
                    tls_sni: true,
                    verify_hostname: true,
                    identity: None,
                    cert_store: CertStore::default(),
                    cert_verification: true,
                    min_version: None,
                    max_version: None,
                },
                protocol: ProtocolConfig {
                    http_version_pref: HttpVersionPref::All,
                    https_only: false,
                    retry_policy: retry::Policy::default(),
                    redirect_policy: redirect::Policy::none(),
                    referer: true,
                    timeout_options: TimeoutOptions::default(),
                    recoveries: Recoveries::new(),
                },
                proxy: ProxyConfig {
                    proxies: Vec::new(),
                    auto_sys_proxy: true,
                },
                dns: DnsConfig {
                    #[cfg(feature = "hickory-dns")]
                    hickory_dns: cfg!(feature = "hickory-dns"),
                    dns_overrides: HashMap::new(),
                    dns_resolver: None,
                },
                middleware: MiddlewareConfig {
                    #[cfg(any(
                        feature = "gzip",
                        feature = "zstd",
                        feature = "brotli",
                        feature = "deflate",
                    ))]
                    accept_encoding: AcceptEncoding::default(),
                    #[cfg(feature = "cookies")]
                    cookie_store: None,
                    layers: Vec::new(),
                    connector_layers: Vec::new(),
                    hooks: None,
                },
                pending_user_agent: None,
                #[cfg(feature = "http3")]
                http3_options: None,
                #[cfg(feature = "http3")]
                quic_config: None,
                #[cfg(feature = "http3")]
                test_quic_connector: None,
            },
        }
    }

    /// Convenience method to make a `GET` request to a URI.
    ///
    /// # Errors
    ///
    /// This method fails whenever the supplied `Uri` cannot be parsed.
    #[inline]
    pub fn get<U: IntoUri>(&self, uri: U) -> RequestBuilder {
        self.request(Method::GET, uri)
    }

    /// Convenience method to make a `POST` request to a URI.
    ///
    /// # Errors
    ///
    /// This method fails whenever the supplied `Uri` cannot be parsed.
    #[inline]
    pub fn post<U: IntoUri>(&self, uri: U) -> RequestBuilder {
        self.request(Method::POST, uri)
    }

    /// Convenience method to make a `PUT` request to a URI.
    ///
    /// # Errors
    ///
    /// This method fails whenever the supplied `Uri` cannot be parsed.
    #[inline]
    pub fn put<U: IntoUri>(&self, uri: U) -> RequestBuilder {
        self.request(Method::PUT, uri)
    }

    /// Convenience method to make a `PATCH` request to a URI.
    ///
    /// # Errors
    ///
    /// This method fails whenever the supplied `Uri` cannot be parsed.
    #[inline]
    pub fn patch<U: IntoUri>(&self, uri: U) -> RequestBuilder {
        self.request(Method::PATCH, uri)
    }

    /// Convenience method to make a `DELETE` request to a URI.
    ///
    /// # Errors
    ///
    /// This method fails whenever the supplied `Uri` cannot be parsed.
    #[inline]
    pub fn delete<U: IntoUri>(&self, uri: U) -> RequestBuilder {
        self.request(Method::DELETE, uri)
    }

    /// Convenience method to make a `HEAD` request to a URI.
    ///
    /// # Errors
    ///
    /// This method fails whenever the supplied `Uri` cannot be parsed.
    #[inline]
    pub fn head<U: IntoUri>(&self, uri: U) -> RequestBuilder {
        self.request(Method::HEAD, uri)
    }

    /// Convenience method to make a `OPTIONS` request to a URI.
    ///
    /// # Errors
    ///
    /// This method fails whenever the supplied `Uri` cannot be parsed.
    #[inline]
    pub fn options<U: IntoUri>(&self, uri: U) -> RequestBuilder {
        self.request(Method::OPTIONS, uri)
    }

    /// Start building a `Request` with the `Method` and `Uri`.
    ///
    /// Returns a `RequestBuilder`, which will allow setting headers and
    /// the request body before sending.
    ///
    /// # Errors
    ///
    /// This method fails whenever the supplied `Uri` cannot be parsed.
    pub fn request<U: IntoUri>(&self, method: Method, uri: U) -> RequestBuilder {
        let req = uri.into_uri().map(move |uri| Request::new(method, uri));
        RequestBuilder::new(self.clone(), req)
    }

    /// Upgrades the [`RequestBuilder`] to perform a
    /// websocket handshake. This returns a wrapped type, so you must do
    /// this after you set up your request, and just before you send the
    /// request.
    #[inline]
    #[cfg(feature = "ws-yawc")]
    #[cfg_attr(docsrs, doc(cfg(feature = "ws-yawc")))]
    pub fn websocket<U: IntoUri>(&self, uri: U) -> WebSocketRequestBuilder {
        WebSocketRequestBuilder::new(self.request(Method::GET, uri))
    }

    /// Executes a `Request`.
    ///
    /// A `Request` can be built manually with `Request::new()` or obtained
    /// from a RequestBuilder with `RequestBuilder::build()`.
    ///
    /// You should prefer to use the `RequestBuilder` and
    /// `RequestBuilder::send()`.
    ///
    /// # Errors
    ///
    /// This method fails if there was an error while sending request,
    /// redirect loop was detected or redirect limit was exhausted.
    pub fn execute(&self, request: Request) -> Pending {
        let req = http::Request::<Body>::from(request);
        // Prepare the future request by ensuring we use the exact same Service instance
        // for both poll_ready and call.
        let uri = req.uri().clone();
        let fut = Oneshot::new(self.inner.as_ref().clone(), req);
        Pending::request(uri, fut)
    }

    /// Consume the client and return the inner tower::Service.
    pub(crate) fn into_inner(self) -> ClientRef {
        Arc::unwrap_or_clone(self.inner)
    }

    /// Get a clone of the inner tower::Service.
    pub(crate) fn clone_inner(&self) -> ClientRef {
        self.inner.as_ref().clone()
    }

    /// Test helper: checks whether the Alt-Svc cache has a non-expired
    /// entry for the given authority `(host, port)`.
    ///
    /// `#[doc(hidden)]` — exposed for integration tests only
    /// (`crates/hpx/tests/http3.rs`). Not part of the stable public API.
    #[cfg(feature = "http3")]
    #[doc(hidden)]
    pub async fn __test_alt_svc_cache_has_entry(&self, host: &str, port: u16) -> bool {
        self.alt_svc_cache
            .get(&(host.to_string(), port))
            .await
            .is_some()
    }

    /// Test helper: checks whether the H3 failure tracker has blocked
    /// the given authority `(host, port)`.
    ///
    /// `#[doc(hidden)]` — exposed for integration tests only
    /// (`crates/hpx/tests/http3.rs`). Not part of the stable public API.
    #[cfg(feature = "http3")]
    #[doc(hidden)]
    pub async fn __test_h3_failure_tracker_is_blocked(&self, host: &str, port: u16) -> bool {
        self.h3_failure_tracker
            .is_blocked(&(host.to_string(), port))
            .await
    }
}

impl tower::Service<Request> for Client {
    type Response = Response;
    type Error = Error;
    type Future = Pending;

    #[inline(always)]
    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    #[inline(always)]
    fn call(&mut self, req: Request) -> Self::Future {
        self.execute(req)
    }
}

impl tower::Service<Request> for &'_ Client {
    type Response = Response;
    type Error = Error;
    type Future = Pending;

    #[inline(always)]
    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    #[inline(always)]
    fn call(&mut self, req: Request) -> Self::Future {
        self.execute(req)
    }
}

// ===== impl ClientBuilder =====

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use super::*;

    struct NoopBeforeRequestHook;

    impl super::super::layer::hooks::BeforeRequestHook for NoopBeforeRequestHook {
        fn on_request(&self, _request: &mut http::Request<Body>) -> Result<(), Error> {
            Ok(())
        }
    }

    #[test]
    fn hooks_only_client_keeps_typed_service_path() {
        let hooks = super::super::layer::hooks::Hooks::builder()
            .before_request(Arc::new(NoopBeforeRequestHook))
            .build();

        let client = Client::builder().hooks(hooks).build().unwrap();

        assert!(matches!(client.into_inner(), ClientRef::Hooked(_)));
    }

    #[test]
    #[allow(deprecated)]
    fn transport_config_options_override_transport_defaults() {
        let connect_timeout = Duration::from_secs(3);
        let builder = Client::builder().transport_config(
            TransportConfigOptions::new()
                .connect_timeout(Some(connect_timeout))
                .connection_verbose(true)
                .tcp_nodelay(false)
                .tcp_reuse_address(true),
        );

        assert_eq!(
            builder.config.transport.connect_timeout,
            Some(connect_timeout)
        );
        assert!(builder.config.transport.connection_verbose);
        assert!(!builder.config.transport.tcp_nodelay);
        assert!(builder.config.transport.tcp_reuse_address);
        assert_eq!(
            builder.config.protocol.timeout_options.connect_timeout(),
            Some(connect_timeout)
        );
    }

    #[test]
    fn transport_builder_methods_mutate_nested_transport_group() {
        let connect_timeout = Duration::from_secs(7);

        let builder = Client::builder()
            .connect_timeout(connect_timeout)
            .connection_verbose(true);

        assert_eq!(
            builder.config.transport.connect_timeout,
            Some(connect_timeout)
        );
        assert!(builder.config.transport.connection_verbose);
        assert_eq!(
            builder.config.protocol.timeout_options.connect_timeout(),
            Some(connect_timeout)
        );
    }

    #[test]
    #[allow(deprecated)]
    fn reusable_protocol_config_can_be_applied_to_multiple_builders() {
        let protocol = ProtocolConfigOptions::new().https_only(true).referer(false);

        let builder_a = Client::builder().protocol_config(protocol.clone());
        let builder_b = Client::builder().protocol_config(protocol);

        assert!(builder_a.config.protocol.https_only);
        assert!(!builder_a.config.protocol.referer);
        assert!(builder_b.config.protocol.https_only);
        assert!(!builder_b.config.protocol.referer);
    }

    #[test]
    #[allow(deprecated)]
    fn protocol_config_preserves_transport_connect_timeout() {
        let connect_timeout = Duration::from_secs(11);

        let builder = Client::builder()
            .connect_timeout(connect_timeout)
            .protocol_config(ProtocolConfigOptions::new().https_only(true));

        assert_eq!(
            builder.config.transport.connect_timeout,
            Some(connect_timeout)
        );
        assert_eq!(
            builder.config.protocol.timeout_options.connect_timeout(),
            Some(connect_timeout)
        );
    }

    #[test]
    fn max_retries_per_request_updates_retry_policy() {
        let builder = Client::builder().max_retries_per_request(3);

        assert_eq!(
            builder.config.protocol.retry_policy.max_retries_per_request,
            3
        );
    }

    #[cfg(feature = "http1")]
    #[test]
    #[allow(deprecated)]
    fn transport_config_preserves_existing_http1_transport_options() {
        let builder = Client::builder()
            .max_poll_iterations(7)
            .transport_config(TransportConfigOptions::new().tcp_nodelay(false));

        let options = builder
            .config
            .transport
            .transport_options
            .http1_options
            .unwrap();

        assert_eq!(options.h1_max_poll_iterations, Some(7));
        assert!(!builder.config.transport.tcp_nodelay);
    }

    #[cfg(feature = "http1")]
    #[test]
    fn max_poll_iterations_updates_http1_options() {
        let builder = Client::builder().max_poll_iterations(7);
        let options = builder
            .config
            .transport
            .transport_options
            .http1_options
            .unwrap();

        assert_eq!(options.h1_max_poll_iterations, Some(7));
    }
}

/// Phase 1 Task 1.2: the `Http3` variant exists on `HttpVersionPref`.
/// The TCP ALPN for `Http3` MUST be `None` — h3 is QUIC-only and is
/// never proposed over TCP TLS (constraint C-06). The actual h3 path
/// over QUIC is wired in `ClientBuilder::build`.
#[cfg(test)]
mod http3_version_pref_tests {
    use super::HttpVersionPref;

    #[test]
    fn http3_version_pref_variant_exists() {
        let pref = HttpVersionPref::Http3;
        // Ensure the variant is constructible and matches exhaustively.
        let _alpn: Option<&'static str> = match pref {
            HttpVersionPref::Http1 => Some("http/1.1"),
            HttpVersionPref::Http2 => Some("h2"),
            HttpVersionPref::Http3 => None,
            HttpVersionPref::All => None,
        };
    }

    /// The TCP ALPN list for `All` MUST NOT include `h3` — h3 is
    /// QUIC-only (C-06).
    #[test]
    fn all_version_pref_tcp_alpn_excludes_h3() {
        let alpn: Option<&'static str> = match HttpVersionPref::All {
            HttpVersionPref::Http1 => Some("http/1.1"),
            HttpVersionPref::Http2 => Some("h2"),
            HttpVersionPref::Http3 => None,
            HttpVersionPref::All => None,
        };
        assert!(alpn.is_none(), "All TCP ALPN must not include h3 (C-06)");
    }
}
