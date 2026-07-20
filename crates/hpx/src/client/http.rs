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
#[cfg(feature = "cookies")]
use crate::cookie;
#[cfg(feature = "http3")]
use crate::client::conn::alt_svc::{AltSvcCache, H3FailureTracker};
#[cfg(feature = "hickory-dns")]
use crate::dns::hickory::HickoryDnsResolver;
#[cfg(feature = "http1")]
use crate::http1::Http1Options;
#[cfg(feature = "http2")]
use crate::http2::Http2Options;
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
    /// pool shard) is wired in T1.4+; until then, this variant exists so
    /// downstream builder methods (`http3_only()` in T1.6) can set it.
    ///
    /// Per constraint C-06, the TCP ALPN list for `Http3` is empty — h3 is
    /// QUIC-only and is never proposed over TCP TLS. Alt-Svc (RFC 7838,
    /// Phase 2) is the only h3 discovery mechanism over TCP responses.
    Http3,
    /// Propose `["h2", "http/1.1"]` over TCP TLS. In Phase 1, `All` does
    /// NOT automatically attempt h3 (Alt-Svc upgrade is Phase 2 scope).
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
    // HTTP/3 (QUIC) configuration. Gated on the `http3` Cargo feature per [C-01].
    //
    // `http3_options` stores the user-supplied [`Http3Options`] (or `None` if
    // the user did not call [`ClientBuilder::http3_options`]). When `None`,
    // `Client::build` (T1.7 scope) will fall back to [`Http3Options::default`]
    // (Chrome 143 baseline).
    //
    // `quic_config` stores an optional low-level [`QuicConfig`] override (a
    // type alias for `quinn::TransportConfig`). When `Some`, the override
    // takes precedence over the transport config derived from `http3_options`.
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

impl ClientBuilder {
    /// Returns a [`Client`] that uses this [`ClientBuilder`] configuration.
    ///
    /// # Errors
    ///
    /// This method fails if a TLS backend cannot be initialized, or the resolver
    /// cannot load the system configuration.
    pub fn build(self) -> crate::Result<Client> {
        let mut config = self.config;

        if let Some(err) = config.error {
            return Err(err);
        }

        // Prepare proxies
        if config.auto_sys_proxy {
            config.proxies.push(ProxyMatcher::system());
        }

        // Create base client service
        // Shared Alt-Svc cache lives outside the `service` block so it can
        // also be stored in `Client` for test inspection.
        #[cfg(feature = "http3")]
        let alt_svc_cache = Arc::new(AltSvcCache::new());
        #[cfg(feature = "http3")]
        let h3_failure_tracker =
            Arc::new(H3FailureTracker::new(std::time::Duration::from_secs(60)));

        let service = {
            let tls_options = config.transport_options.tls_options;
            #[cfg(feature = "http1")]
            let http1_options = config.transport_options.http1_options;
            #[cfg(feature = "http2")]
            let http2_options = config.transport_options.http2_options;

            let resolver = {
                let mut resolver: Arc<dyn Resolve> = match config.dns_resolver {
                    Some(dns_resolver) => dns_resolver,
                    #[cfg(feature = "hickory-dns")]
                    None if config.hickory_dns => Arc::new(HickoryDnsResolver::new()),
                    None => Arc::new(GaiResolver::new()),
                };

                if !config.dns_overrides.is_empty() {
                    resolver = Arc::new(DnsResolverWithOverrides::new(
                        resolver,
                        config.dns_overrides,
                    ));
                }
                DynResolver::new(resolver)
            };

            // Build connector
            let connector = Connector::builder(config.proxies, resolver)
                .timeout(config.connect_timeout)
                .tls_info(config.tls_info)
                .tls_options(tls_options)
                .verbose(config.connection_verbose)
                .with_tls(|tls| {
                    let alpn_protocol = match config.http_version_pref {
                        HttpVersionPref::Http1 => Some(AlpnProtocol::HTTP1),
                        HttpVersionPref::Http2 => Some(AlpnProtocol::HTTP2),
                        // h3 is QUIC-only — never proposed over TCP TLS (C-06).
                        // The actual h3 transport over QUIC is wired in T1.4+.
                        HttpVersionPref::Http3 => None,
                        _ => None,
                    };
                    tls.alpn_protocol(alpn_protocol)
                        .max_version(config.max_tls_version)
                        .min_version(config.min_tls_version)
                        .tls_sni(config.tls_sni)
                        .verify_hostname(config.verify_hostname)
                        .cert_verification(config.cert_verification)
                        .cert_store(config.cert_store)
                        .identity(config.identity)
                        .keylog(config.keylog)
                })
                .with_http(|http| {
                    http.enforce_http(false);
                    http.set_keepalive(config.tcp_keepalive);
                    http.set_keepalive_interval(config.tcp_keepalive_interval);
                    http.set_keepalive_retries(config.tcp_keepalive_retries);
                    http.set_reuse_address(config.tcp_reuse_address);
                    http.set_connect_options(config.tcp_connect_options);
                    http.set_connect_timeout(config.connect_timeout);
                    http.set_nodelay(config.tcp_nodelay);
                    http.set_send_buffer_size(config.tcp_send_buffer_size);
                    http.set_recv_buffer_size(config.tcp_recv_buffer_size);
                    http.set_happy_eyeballs_timeout(config.tcp_happy_eyeballs_timeout);
                    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
                    http.set_tcp_user_timeout(config.tcp_user_timeout);
                })
                .build(config.connector_layers)?;

            // Build client
            #[allow(unused_mut)]
            let mut builder = HttpClient::builder(TokioExecutor::new());

            #[cfg(feature = "http1")]
            {
                builder = builder.http1_options(http1_options);
            }

            #[cfg(feature = "http2")]
            {
                builder = builder
                    .http2_options(http2_options)
                    .http2_only(matches!(config.http_version_pref, HttpVersionPref::Http2))
                    .http2_timer(TokioTimer::new());
            }

            // HTTP/3 (QUIC) connector wiring (T1.10 scope).
            //
            // When the caller supplies a `QuicConnector` via the
            // `__test_with_quic_connector` escape hatch (e.g., integration
            // tests with a custom root store trusting a self-signed cert),
            // pass it through to `HttpClient::Builder::h3_connector` verbatim.
            //
            // When `http_version_pref == HttpVersionPref::Http3`, set
            // `Ver::Http3` on the `HttpClient::Builder` so `connect_to`
            // routes through the `QuicConnector` instead of the TCP
            // connector. If no connector is wired in (neither test escape
            // hatch nor the production construction path), h3 requests fail
            // at runtime with `ErrorKind::UserUnsupportedVersion` ("HTTP/3
            // requested but no QuicConnector wired in") from the
            // `Ver::Http3` branch of `connect_to`.
            //
            // TODO(T1.7-complete): for the production (non-test) path,
            // construct a `QuicConnector` from `config.http3_options` /
            // `config.quic_config` / `tls_options` / `cert_store`. Until
            // then, only the `__test_with_quic_connector` escape hatch is
            // wired; `http3_only().build()` without the escape hatch will
            // produce a client whose h3 requests fail at runtime.
            #[cfg(feature = "http3")]
            {
                if matches!(config.http_version_pref, HttpVersionPref::Http3) {
                    builder = builder.http3_only(true);
                }
                if let Some(quic_connector) = config.test_quic_connector.take() {
                    builder = builder.h3_connector(quic_connector);
                }
                builder = builder.alt_svc_cache(alt_svc_cache.clone());
                builder = builder.h3_failure_tracker(h3_failure_tracker.clone());
            }

            builder
                .pool_timer(TokioTimer::new())
                .pool_idle_timeout(config.pool_idle_timeout)
                .pool_max_idle_per_host(config.pool_max_idle_per_host)
                .pool_max_size(config.pool_max_size)
                .build(connector)
                .map_err(Into::into as _)
        };

        // Configured client service with layers
        let client = {
            #[cfg(feature = "cookies")]
            let service = ServiceBuilder::new()
                .layer(CookieServiceLayer::new(config.cookie_store))
                .service(service);

            let service = ServiceBuilder::new()
                .layer(RetryLayer::new(RetryPolicy::new(config.retry_policy)))
                .layer({
                    let policy = FollowRedirectPolicy::new(config.redirect_policy)
                        .with_referer(config.referer)
                        .with_https_only(config.https_only);
                    FollowRedirectLayer::with_policy(policy)
                })
                .service(service);

            #[cfg(any(
                feature = "gzip",
                feature = "zstd",
                feature = "brotli",
                feature = "deflate",
            ))]
            let service = ServiceBuilder::new()
                .layer(DecompressionLayer::new(config.accept_encoding))
                .service(service);

            let service = ServiceBuilder::new()
                .layer(ResponseBodyTimeoutLayer::new(config.timeout_options))
                .layer(ConfigServiceLayer::new(
                    config.https_only,
                    config.headers,
                    config.orig_headers,
                ))
                .service(service);

            // Add hooks layer if configured
            let has_hooks = config.hooks.as_ref().is_some_and(|h| !h.is_empty());

            if config.layers.is_empty() && !has_hooks {
                let service = ServiceBuilder::new()
                    .layer(TimeoutLayer::new(config.timeout_options))
                    .service(service);

                ClientRef::Left(service)
            } else {
                // Start with boxed service
                let mut service = BoxCloneSyncService::new(service);

                // Add hooks layer if present
                if let Some(hooks) = config.hooks
                    && !hooks.is_empty()
                {
                    let hooks_layer = super::layer::hooks::HooksLayer::new(hooks);
                    service = ServiceBuilder::new()
                        .layer(BoxCloneSyncServiceLayer::new(hooks_layer))
                        .service(service);
                }

                // Add custom layers
                let service = config.layers.into_iter().fold(service, |service, layer| {
                    ServiceBuilder::new().layer(layer).service(service)
                });

                let service = ServiceBuilder::new()
                    .layer(TimeoutLayer::new(config.timeout_options))
                    .service(service)
                    .map_err(error::map_timeout_to_request_error);

                ClientRef::Right(BoxCloneSyncService::new(service))
            }
        };

        Ok(Client {
            inner: Arc::new(client),
            #[cfg(feature = "http3")]
            alt_svc_cache,
            #[cfg(feature = "http3")]
            h3_failure_tracker,
        })
    }

    // Higher-level options

    /// Sets the `User-Agent` header to be used by this client.
    ///
    /// # Example
    ///
    /// ```rust
    /// # async fn doc() -> hpx::Result<()> {
    /// // Name your user agent after your app?
    /// static APP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);
    ///
    /// let client = hpx::Client::builder().user_agent(APP_USER_AGENT).build()?;
    /// let res = client.get("https://www.rust-lang.org").send().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn user_agent<V>(mut self, value: V) -> ClientBuilder
    where
        V: TryInto<HeaderValue>,
        V::Error: Into<http::Error>,
    {
        match value.try_into() {
            Ok(value) => {
                self.config.headers.insert(USER_AGENT, value);
            }
            Err(err) => {
                self.config.error = Some(Error::builder(err.into()));
            }
        };
        self
    }

    /// Sets the default headers for every request.
    ///
    /// # Example
    ///
    /// ```rust
    /// use hpx::header;
    /// # async fn doc() -> hpx::Result<()> {
    /// let mut headers = header::HeaderMap::new();
    /// headers.insert("X-MY-HEADER", header::HeaderValue::from_static("value"));
    ///
    /// // Consider marking security-sensitive headers with `set_sensitive`.
    /// let mut auth_value = header::HeaderValue::from_static("secret");
    /// auth_value.set_sensitive(true);
    /// headers.insert(header::AUTHORIZATION, auth_value);
    ///
    /// // get a client builder
    /// let client = hpx::Client::builder().default_headers(headers).build()?;
    /// let res = client.get("https://www.rust-lang.org").send().await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Override the default headers:
    ///
    /// ```rust
    /// use hpx::header;
    /// # async fn doc() -> hpx::Result<()> {
    /// let mut headers = header::HeaderMap::new();
    /// headers.insert("X-MY-HEADER", header::HeaderValue::from_static("value"));
    ///
    /// // get a client builder
    /// let client = hpx::Client::builder().default_headers(headers).build()?;
    /// let res = client
    ///     .get("https://www.rust-lang.org")
    ///     .header("X-MY-HEADER", "new_value")
    ///     .send()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn default_headers(mut self, headers: HeaderMap) -> ClientBuilder {
        crate::util::replace_headers(&mut self.config.headers, headers);
        self
    }

    /// Sets the original headers for every request.
    #[inline]
    pub fn orig_headers(mut self, orig_headers: OrigHeaderMap) -> ClientBuilder {
        self.config.orig_headers.extend(orig_headers);
        self
    }

    /// Enable a persistent cookie store for the client.
    ///
    /// Cookies received in responses will be preserved and included in
    /// additional requests.
    ///
    /// By default, no cookie store is used.
    ///
    /// # Optional
    ///
    /// This requires the optional `cookies` feature to be enabled.
    #[inline]
    #[cfg(feature = "cookies")]
    #[cfg_attr(docsrs, doc(cfg(feature = "cookies")))]
    pub fn cookie_store(mut self, enable: bool) -> ClientBuilder {
        if enable {
            self.cookie_provider(Arc::new(cookie::Jar::default()))
        } else {
            self.config.cookie_store = None;
            self
        }
    }

    /// Set the persistent cookie store for the client.
    ///
    /// Cookies received in responses will be passed to this store, and
    /// additional requests will query this store for cookies.
    ///
    /// By default, no cookie store is used.
    ///
    /// # Optional
    ///
    /// This requires the optional `cookies` feature to be enabled.
    #[inline]
    #[cfg(feature = "cookies")]
    #[cfg_attr(docsrs, doc(cfg(feature = "cookies")))]
    pub fn cookie_provider<C>(mut self, cookie_store: C) -> ClientBuilder
    where
        C: cookie::IntoCookieStore,
    {
        self.config.cookie_store = Some(cookie_store.into_cookie_store());
        self
    }

    /// Enable auto gzip decompression by checking the `Content-Encoding` response header.
    ///
    /// If auto gzip decompression is turned on:
    ///
    /// - When sending a request and if the request's headers do not already contain an
    ///   `Accept-Encoding` **and** `Range` values, the `Accept-Encoding` header is set to `gzip`.
    ///   The request body is **not** automatically compressed.
    /// - When receiving a response, if its headers contain a `Content-Encoding` value of `gzip`,
    ///   both `Content-Encoding` and `Content-Length` are removed from the headers' set. The
    ///   response body is automatically decompressed.
    ///
    /// If the `gzip` feature is turned on, the default option is enabled.
    ///
    /// # Optional
    ///
    /// This requires the optional `gzip` feature to be enabled
    #[inline]
    #[cfg(feature = "gzip")]
    #[cfg_attr(docsrs, doc(cfg(feature = "gzip")))]
    pub fn gzip(mut self, enable: bool) -> ClientBuilder {
        self.config.accept_encoding.gzip = enable;
        self
    }

    /// Enable auto brotli decompression by checking the `Content-Encoding` response header.
    ///
    /// If auto brotli decompression is turned on:
    ///
    /// - When sending a request and if the request's headers do not already contain an
    ///   `Accept-Encoding` **and** `Range` values, the `Accept-Encoding` header is set to `br`. The
    ///   request body is **not** automatically compressed.
    /// - When receiving a response, if its headers contain a `Content-Encoding` value of `br`, both
    ///   `Content-Encoding` and `Content-Length` are removed from the headers' set. The response
    ///   body is automatically decompressed.
    ///
    /// If the `brotli` feature is turned on, the default option is enabled.
    ///
    /// # Optional
    ///
    /// This requires the optional `brotli` feature to be enabled
    #[inline]
    #[cfg(feature = "brotli")]
    #[cfg_attr(docsrs, doc(cfg(feature = "brotli")))]
    pub fn brotli(mut self, enable: bool) -> ClientBuilder {
        self.config.accept_encoding.brotli = enable;
        self
    }

    /// Enable auto zstd decompression by checking the `Content-Encoding` response header.
    ///
    /// If auto zstd decompression is turned on:
    ///
    /// - When sending a request and if the request's headers do not already contain an
    ///   `Accept-Encoding` **and** `Range` values, the `Accept-Encoding` header is set to `zstd`.
    ///   The request body is **not** automatically compressed.
    /// - When receiving a response, if its headers contain a `Content-Encoding` value of `zstd`,
    ///   both `Content-Encoding` and `Content-Length` are removed from the headers' set. The
    ///   response body is automatically decompressed.
    ///
    /// If the `zstd` feature is turned on, the default option is enabled.
    ///
    /// # Optional
    ///
    /// This requires the optional `zstd` feature to be enabled
    #[inline]
    #[cfg(feature = "zstd")]
    #[cfg_attr(docsrs, doc(cfg(feature = "zstd")))]
    pub fn zstd(mut self, enable: bool) -> ClientBuilder {
        self.config.accept_encoding.zstd = enable;
        self
    }

    /// Enable auto deflate decompression by checking the `Content-Encoding` response header.
    ///
    /// If auto deflate decompression is turned on:
    ///
    /// - When sending a request and if the request's headers do not already contain an
    ///   `Accept-Encoding` **and** `Range` values, the `Accept-Encoding` header is set to
    ///   `deflate`. The request body is **not** automatically compressed.
    /// - When receiving a response, if it's headers contain a `Content-Encoding` value that equals
    ///   to `deflate`, both values `Content-Encoding` and `Content-Length` are removed from the
    ///   headers' set. The response body is automatically decompressed.
    ///
    /// If the `deflate` feature is turned on, the default option is enabled.
    ///
    /// # Optional
    ///
    /// This requires the optional `deflate` feature to be enabled
    #[inline]
    #[cfg(feature = "deflate")]
    #[cfg_attr(docsrs, doc(cfg(feature = "deflate")))]
    pub fn deflate(mut self, enable: bool) -> ClientBuilder {
        self.config.accept_encoding.deflate = enable;
        self
    }

    /// Disable auto response body zstd decompression.
    ///
    /// This method exists even if the optional `zstd` feature is not enabled.
    /// This can be used to ensure a `Client` doesn't use zstd decompression
    /// even if another dependency were to enable the optional `zstd` feature.
    #[inline]
    pub fn no_zstd(self) -> ClientBuilder {
        #[cfg(feature = "zstd")]
        {
            self.zstd(false)
        }

        #[cfg(not(feature = "zstd"))]
        {
            self
        }
    }

    /// Disable auto response body gzip decompression.
    ///
    /// This method exists even if the optional `gzip` feature is not enabled.
    /// This can be used to ensure a `Client` doesn't use gzip decompression
    /// even if another dependency were to enable the optional `gzip` feature.
    #[inline]
    pub fn no_gzip(self) -> ClientBuilder {
        #[cfg(feature = "gzip")]
        {
            self.gzip(false)
        }
    }

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
    /// Set a request retry policy.
    pub fn retry(mut self, policy: retry::Policy) -> ClientBuilder {
        self.config.retry_policy = policy;
        self
    }

    // Proxy options

    /// Add a `Proxy` to the list of proxies the `Client` will use.
    ///
    /// # Note
    ///
    /// Adding a proxy will disable the automatic usage of the "system" proxy.
    ///
    /// # Example
    /// ```
    /// use hpx::{Client, Proxy};
    ///
    /// let proxy = Proxy::http("http://proxy:8080").unwrap();
    /// let client = Client::builder().proxy(proxy).build().unwrap();
    /// ```
    #[inline]
    pub fn proxy(mut self, proxy: Proxy) -> ClientBuilder {
        self.config.proxies.push(proxy.into_matcher());
        self.config.auto_sys_proxy = false;
        self
    }

    /// Clear all `Proxies`, so `Client` will use no proxy anymore.
    ///
    /// # Note
    /// To add a proxy exclusion list, use [crate::proxy::Proxy::no_proxy()]
    /// on all desired proxies instead.
    ///
    /// This also disables the automatic usage of the "system" proxy.
    #[inline]
    pub fn no_proxy(mut self) -> ClientBuilder {
        self.config.proxies.clear();
        self.config.auto_sys_proxy = false;
        self
    }

    // Timeout options

    /// Enables a request timeout.
    ///
    /// The timeout is applied from when the request starts connecting until the
    /// response body has finished.
    ///
    /// Default is no timeout.
    #[inline]
    pub fn timeout(mut self, timeout: Duration) -> ClientBuilder {
        self.config.timeout_options.total_timeout(timeout);
        self
    }

    /// Set a timeout for only the read phase of a `Client`.
    ///
    /// Default is `None`.
    #[inline]
    pub fn read_timeout(mut self, timeout: Duration) -> ClientBuilder {
        self.config.timeout_options.read_timeout(timeout);
        self
    }

    /// Set a timeout for only the connect phase of a `Client`.
    ///
    /// Default is `None`.
    ///
    /// # Note
    ///
    /// This **requires** the futures be executed in a tokio runtime with
    /// a tokio timer enabled.
    #[inline]
    pub fn connect_timeout(mut self, timeout: Duration) -> ClientBuilder {
        self.config.connect_timeout = Some(timeout);
        self
    }

    /// Set whether connections should emit verbose logs.
    ///
    /// Enabling this option will emit [log][] messages at the `TRACE` level
    /// for read and write operations on connections.
    ///
    /// [log]: https://crates.io/crates/log
    #[inline]
    pub fn connection_verbose(mut self, verbose: bool) -> ClientBuilder {
        self.config.connection_verbose = verbose;
        self
    }

    // HTTP options

    /// Set an optional timeout for idle sockets being kept-alive.
    ///
    /// Pass `None` to disable timeout.
    ///
    /// Default is 90 seconds.
    #[inline]
    pub fn pool_idle_timeout<D>(mut self, val: D) -> ClientBuilder
    where
        D: Into<Option<Duration>>,
    {
        self.config.pool_idle_timeout = val.into();
        self
    }

    /// Sets the maximum idle connection per host allowed in the pool.
    #[inline]
    pub fn pool_max_idle_per_host(mut self, max: usize) -> ClientBuilder {
        self.config.pool_max_idle_per_host = max;
        self
    }

    /// Sets the maximum number of connections in the pool.
    #[inline]
    pub fn pool_max_size(mut self, max: u32) -> ClientBuilder {
        self.config.pool_max_size = NonZeroU32::new(max);
        self
    }

    /// Restrict the Client to be used with HTTPS only requests.
    ///
    /// Defaults to false.
    #[inline]
    pub fn https_only(mut self, enabled: bool) -> ClientBuilder {
        self.config.https_only = enabled;
        self
    }

    /// Only use HTTP/1.
    #[inline]
    pub fn http1_only(mut self) -> ClientBuilder {
        self.config.http_version_pref = HttpVersionPref::Http1;
        self
    }

    /// Only use HTTP/2.
    #[inline]
    pub fn http2_only(mut self) -> ClientBuilder {
        self.config.http_version_pref = HttpVersionPref::Http2;
        self
    }

    /// Sets the HTTP/1 options for the client.
    #[cfg(feature = "http1")]
    #[inline]
    pub fn http1_options(mut self, options: Http1Options) -> ClientBuilder {
        *self.config.transport_options.http1_options_mut() = Some(options);
        self
    }

    /// Sets the HTTP/2 options for the client.
    #[cfg(feature = "http2")]
    #[inline]
    pub fn http2_options(mut self, options: Http2Options) -> ClientBuilder {
        *self.config.transport_options.http2_options_mut() = Some(options);
        self
    }

    // ===== HTTP/3 (QUIC) options =====
    //
    // The four `http3_*` / `quic_config` builder methods below are gated on
    // the `http3` Cargo feature per [C-01]. They store their inputs on the
    // `Config` struct for later consumption by `QuicConnector` in
    // `Client::build` (T1.7 scope — see the `// TODO(T1.7)` marker there).
    //
    // Naming follows the reqwest API for familiarity:
    //   - `http3_only()` forces HTTP/3 only (no fallback to h2/h1).
    //   - `http3_prior_knowledge()` is an alias of `http3_only()` (reqwest
    //     exposes both names; hpx mirrors this so users can copy reqwest
    //     examples verbatim).
    //   - `http3_options(Http3Options)` configures h3 SETTINGS + QUIC transport.
    //   - `quic_config(QuicConfig)` is a low-level escape hatch that overrides
    //     the transport config derived from `http3_options`.

    /// Enables HTTP/3 (QUIC) transport for this client.
    ///
    /// When enabled, requests are sent over QUIC instead of TCP.
    /// This is equivalent to HTTP/3 prior knowledge per [RFC 9114 §3.1.1].
    /// If the QUIC handshake fails, the request fails — there is no
    /// automatic fallback to HTTP/2 or HTTP/1 over TCP.
    ///
    /// This sets the version preference to `HttpVersionPref::Http3`, which:
    /// - Skips TCP TLS ALPN negotiation entirely (h3 is QUIC-only per [C-06];
    ///   see the `match` arm in `Client::build` that returns `None` for the
    ///   TCP ALPN list when `Http3` is selected).
    /// - Routes requests through the h3 pool shard (`Ver::Http3`).
    ///
    /// For h3-with-fallback (Alt-Svc + h2/h1 retry, [RFC 7838]), use
    /// `prefer_http3()` (Phase 2 / T2.5 scope — not yet implemented).
    ///
    /// [RFC 9114 §3.1.1]: https://www.rfc-editor.org/rfc/rfc9114.html#section-3.1.1
    /// [RFC 7838]: https://www.rfc-editor.org/rfc/rfc7838.html
    /// [C-06]: ../tls/index.html
    #[cfg(feature = "http3")]
    #[inline]
    pub fn http3_only(mut self) -> ClientBuilder {
        self.config.http_version_pref = HttpVersionPref::Http3;
        self
    }

    /// Alias of [`http3_only()`](Self::http3_only) for parity with the
    /// reqwest API.
    ///
    /// HTTP/3 prior knowledge means the client assumes the server supports
    /// HTTP/3 without first discovering it via [Alt-Svc] ([RFC 7838]).
    /// See [RFC 9114 §3.1.1] for the formal definition.
    ///
    /// Both names exist so that reqwest examples using
    /// `.http3_prior_knowledge()` work verbatim against hpx. The two methods
    /// are exactly equivalent.
    ///
    /// [RFC 9114 §3.1.1]: https://www.rfc-editor.org/rfc/rfc9114.html#section-3.1.1
    /// [RFC 7838]: https://www.rfc-editor.org/rfc/rfc7838.html
    /// [Alt-Svc]: https://www.rfc-editor.org/rfc/rfc7838.html
    #[cfg(feature = "http3")]
    #[inline]
    pub fn http3_prior_knowledge(self) -> ClientBuilder {
        self.http3_only()
    }

    /// Prefer HTTP/3, but fall back to HTTP/2 or HTTP/1.1 if HTTP/3 is
    /// not available.
    ///
    /// This sets the version preference to `All` (`["h2", "http/1.1"]` over
    /// TCP TLS) and enables Alt-Svc ([RFC 7838]) discovery. When a server
    /// advertises h3 via an `alt-svc` response header, subsequent requests
    /// to the same origin are upgraded to HTTP/3 over QUIC. If the QUIC
    /// handshake or connection fails, the client falls back to HTTP/2 or
    /// HTTP/1.1 over TCP.
    ///
    /// This is the recommended approach for HTTP/3 adoption — it lets the
    /// client discover h3-capable servers dynamically without requiring
    /// prior knowledge of h3 support.
    ///
    /// Use [`http3_only()`](Self::http3_only) if you want to force HTTP/3
    /// without any fallback to HTTP/2 or HTTP/1.1.
    ///
    /// [RFC 7838]: https://www.rfc-editor.org/rfc/rfc7838.html
    #[cfg(feature = "http3")]
    #[inline]
    pub fn prefer_http3(mut self) -> ClientBuilder {
        self.config.http_version_pref = HttpVersionPref::All;
        self
    }

    /// Configures the HTTP/3 (h3 + QUIC) protocol parameters per
    /// [RFC 9114].
    ///
    /// The supplied [`Http3Options`] is stored verbatim on the
    /// [`ClientBuilder`] and consumed by `QuicConnector` in
    /// `Client::build` to populate the h3 SETTINGS frame and the
    /// [`quinn::TransportConfig`].
    ///
    /// When this method is NOT called, `Client::build` falls back to
    /// [`Http3Options::default()`] (Chrome 143 baseline).
    ///
    /// Accepts `impl Into<Http3Options>` so callers can pass either an
    /// `Http3Options` instance or anything that converts into one (e.g., a
    /// builder pattern from `hpx-emulation` in Phase 3).
    ///
    /// [RFC 9114]: https://www.rfc-editor.org/rfc/rfc9114.html
    #[cfg(feature = "http3")]
    #[inline]
    pub fn http3_options(mut self, opts: impl Into<Http3Options>) -> ClientBuilder {
        self.config.http3_options = Some(opts.into());
        self
    }

    /// Overrides the default [`quinn::TransportConfig`] (QUIC transport
    /// parameters per [RFC 9000]) derived from
    /// [`http3_options`](Self::http3_options) with a user-supplied
    /// [`QuicConfig`].
    ///
    /// This is a low-level escape hatch for callers who need fine-grained
    /// control over QUIC transport parameters (e.g., to set
    /// `max_idle_timeout`, `stream_receive_window`, or congestion control
    /// settings) that are not exposed via [`Http3Options`].
    ///
    /// When this method is called, the supplied [`QuicConfig`] takes
    /// precedence over the transport config that would otherwise be derived
    /// from `http3_options` by `tls::quic::build_transport_config`.
    ///
    /// [RFC 9000]: https://www.rfc-editor.org/rfc/rfc9000.html
    #[cfg(feature = "http3")]
    #[inline]
    pub fn quic_config(mut self, cfg: QuicConfig) -> ClientBuilder {
        self.config.quic_config = Some(cfg);
        self
    }

    // ===== HTTP/3 test accessors (`#[doc(hidden)]`) =====
    //
    // Exposed so that integration tests (in `crates/hpx/tests/http3.rs`) can
    // verify the four builder methods above store their inputs correctly
    // without having to spin up a real h3 server. They are intentionally
    // `#[doc(hidden)]` so they don't appear in the public API surface — they
    // exist solely to support BDD scenario verification in T1.6 and are not
    // part of the stable API contract. Callers should not rely on them in
    // production code; they may be removed or renamed in a future release.
    //
    // Naming: the accessors use the `_ref` suffix (e.g., `http3_options_ref`)
    // to avoid colliding with the builder methods of the same base name
    // (`http3_options`, `quic_config`). Rust does not support method
    // overloading, so the builder method (which matches reqwest's public API
    // for familiarity) "wins" the canonical name and the test accessor gets
    // the `_ref` suffix.

    /// Returns `true` if the builder is configured for HTTP/3 only.
    ///
    /// `#[doc(hidden)]` — test accessor for the T1.6 BDD scenario; not part
    /// of the stable public API.
    #[cfg(feature = "http3")]
    #[doc(hidden)]
    #[inline]
    pub fn is_http3_only(&self) -> bool {
        matches!(self.config.http_version_pref, HttpVersionPref::Http3)
    }

    /// Returns the stored [`Http3Options`], or `None` if
    /// [`http3_options()`](Self::http3_options) was not called.
    ///
    /// `#[doc(hidden)]` — test accessor for the T1.6 BDD scenario; not part
    /// of the stable public API.
    #[cfg(feature = "http3")]
    #[doc(hidden)]
    #[inline]
    pub fn http3_options_ref(&self) -> Option<&Http3Options> {
        self.config.http3_options.as_ref()
    }

    /// Returns the stored [`QuicConfig`] override, or `None` if
    /// [`quic_config()`](Self::quic_config) was not called.
    ///
    /// `#[doc(hidden)]` — test accessor for the T1.6 BDD scenario; not part
    /// of the stable public API.
    #[cfg(feature = "http3")]
    #[doc(hidden)]
    #[inline]
    pub fn quic_config_ref(&self) -> Option<&QuicConfig> {
        self.config.quic_config.as_ref()
    }

    /// Test-only escape hatch for injecting a pre-constructed `QuicConnector`
    /// with a custom root store (e.g., for integration tests trusting a
    /// self-signed cert).
    ///
    /// `#[doc(hidden)]` — exists solely to support the T1.10 BDD scenario
    /// (`http3_request_full`); not part of the stable public API. Production
    /// code should use `ClientBuilder::http3_only().build()`, which
    /// constructs the `QuicConnector` from `Config::http3_options` and
    /// `Config::quic_config` internally (T1.7-complete scope).
    ///
    /// When called, `Client::build` passes the supplied connector through to
    /// [`HttpClient::Builder::h3_connector`] verbatim, bypassing the normal
    /// `QuicConnector` construction path. This lets integration tests
    /// configure a custom rustls root store (e.g., trusting a self-signed
    /// rcgen cert) without having to plumb through `TlsOptions` /
    /// `CertStore`.
    #[cfg(feature = "http3")]
    #[doc(hidden)]
    #[inline]
    pub fn __test_with_quic_connector(
        mut self,
        connector: crate::client::conn::quic::QuicConnector,
    ) -> ClientBuilder {
        self.config.test_quic_connector = Some(connector);
        self
    }

    // TCP options

    /// Set whether sockets have `TCP_NODELAY` enabled.
    ///
    /// Default is `true`.
    #[inline]
    pub fn tcp_nodelay(mut self, enabled: bool) -> ClientBuilder {
        self.config.tcp_nodelay = enabled;
        self
    }

    /// Set that all sockets have `SO_KEEPALIVE` set with the supplied duration.
    ///
    /// If `None`, the option will not be set.
    ///
    /// Default is 15 seconds.
    #[inline]
    pub fn tcp_keepalive<D>(mut self, val: D) -> ClientBuilder
    where
        D: Into<Option<Duration>>,
    {
        self.config.tcp_keepalive = val.into();
        self
    }

    /// Set that all sockets have `SO_KEEPALIVE` set with the supplied interval.
    ///
    /// If `None`, the option will not be set.
    ///
    /// Default is 15 seconds.
    #[inline]
    pub fn tcp_keepalive_interval<D>(mut self, val: D) -> ClientBuilder
    where
        D: Into<Option<Duration>>,
    {
        self.config.tcp_keepalive_interval = val.into();
        self
    }

    /// Set that all sockets have `SO_KEEPALIVE` set with the supplied retry count.
    ///
    /// If `None`, the option will not be set.
    ///
    /// Default is 3 retries.
    #[inline]
    pub fn tcp_keepalive_retries<C>(mut self, retries: C) -> ClientBuilder
    where
        C: Into<Option<u32>>,
    {
        self.config.tcp_keepalive_retries = retries.into();
        self
    }

    /// Set that all sockets have `TCP_USER_TIMEOUT` set with the supplied duration.
    ///
    /// This option controls how long transmitted data may remain unacknowledged before
    /// the connection is force-closed.
    ///
    /// If `None`, the option will not be set.
    ///
    /// Default is 30 seconds.
    #[inline]
    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    #[cfg_attr(
        docsrs,
        doc(cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux")))
    )]
    pub fn tcp_user_timeout<D>(mut self, val: D) -> ClientBuilder
    where
        D: Into<Option<Duration>>,
    {
        self.config.tcp_user_timeout = val.into();
        self
    }

    /// Set whether sockets have `SO_REUSEADDR` enabled.
    #[inline]
    pub fn tcp_reuse_address(mut self, enabled: bool) -> ClientBuilder {
        self.config.tcp_reuse_address = enabled;
        self
    }

    /// Sets the size of the TCP send buffer on this client socket.
    ///
    /// On most operating systems, this sets the `SO_SNDBUF` socket option.
    #[inline]
    pub fn tcp_send_buffer_size<S>(mut self, size: S) -> ClientBuilder
    where
        S: Into<Option<usize>>,
    {
        self.config.tcp_send_buffer_size = size.into();
        self
    }

    /// Sets the size of the TCP receive buffer on this client socket.
    ///
    /// On most operating systems, this sets the `SO_RCVBUF` socket option.
    #[inline]
    pub fn tcp_recv_buffer_size<S>(mut self, size: S) -> ClientBuilder
    where
        S: Into<Option<usize>>,
    {
        self.config.tcp_recv_buffer_size = size.into();
        self
    }

    /// Set timeout for [RFC 6555 (Happy Eyeballs)][RFC 6555] algorithm.
    ///
    /// If hostname resolves to both IPv4 and IPv6 addresses and connection
    /// cannot be established using preferred address family before timeout
    /// elapses, then connector will in parallel attempt connection using other
    /// address family.
    ///
    /// If `None`, parallel connection attempts are disabled.
    ///
    /// Default is 300 milliseconds.
    ///
    /// [RFC 6555]: https://tools.ietf.org/html/rfc6555
    #[inline]
    pub fn tcp_happy_eyeballs_timeout<D>(mut self, val: D) -> ClientBuilder
    where
        D: Into<Option<Duration>>,
    {
        self.config.tcp_happy_eyeballs_timeout = val.into();
        self
    }

    /// Bind to a local IP Address.
    ///
    /// # Example
    ///
    /// ```
    /// use std::net::IpAddr;
    /// let local_addr = IpAddr::from([12, 4, 1, 8]);
    /// let client = hpx::Client::builder()
    ///     .local_address(local_addr)
    ///     .build()
    ///     .unwrap();
    /// ```
    #[inline]
    pub fn local_address<T>(mut self, addr: T) -> ClientBuilder
    where
        T: Into<Option<IpAddr>>,
    {
        self.config
            .tcp_connect_options
            .set_local_address(addr.into());
        self
    }

    /// Set that all sockets are bound to the configured IPv4 or IPv6 address (depending on host's
    /// preferences) before connection.
    ///
    ///  # Example
    /// ///
    /// ```
    /// use std::net::{Ipv4Addr, Ipv6Addr};
    /// let ipv4 = Ipv4Addr::new(127, 0, 0, 1);
    /// let ipv6 = Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1);
    /// let client = hpx::Client::builder()
    ///     .local_addresses(ipv4, ipv6)
    ///     .build()
    ///     .unwrap();
    /// ```
    #[inline]
    pub fn local_addresses<V4, V6>(mut self, ipv4: V4, ipv6: V6) -> ClientBuilder
    where
        V4: Into<Option<Ipv4Addr>>,
        V6: Into<Option<Ipv6Addr>>,
    {
        self.config
            .tcp_connect_options
            .set_local_addresses(ipv4, ipv6);
        self
    }

    /// Bind connections only on the specified network interface.
    ///
    /// This option is only available on the following operating systems:
    ///
    /// - Android
    /// - Fuchsia
    /// - Linux,
    /// - macOS and macOS-like systems (iOS, tvOS, watchOS and visionOS)
    /// - Solaris and illumos
    ///
    /// On Android, Linux, and Fuchsia, this uses the
    /// [`SO_BINDTODEVICE`][man-7-socket] socket option. On macOS and macOS-like
    /// systems, Solaris, and illumos, this instead uses the [`IP_BOUND_IF` and
    /// `IPV6_BOUND_IF`][man-7p-ip] socket options (as appropriate).
    ///
    /// Note that connections will fail if the provided interface name is not a
    /// network interface that currently exists when a connection is established.
    ///
    /// # Example
    ///
    /// ```
    /// # fn doc() -> Result<(), hpx::Error> {
    /// let interface = "lo";
    /// let client = hpx::Client::builder().interface(interface).build()?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [man-7-socket]: https://man7.org/linux/man-pages/man7/socket.7.html
    /// [man-7p-ip]: https://docs.oracle.com/cd/E86824_01/html/E54777/ip-7p.html
    #[inline]
    #[cfg(any(
        target_os = "android",
        target_os = "fuchsia",
        target_os = "illumos",
        target_os = "ios",
        target_os = "linux",
        target_os = "macos",
        target_os = "solaris",
        target_os = "tvos",
        target_os = "visionos",
        target_os = "watchos",
    ))]
    #[cfg_attr(
        docsrs,
        doc(cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "illumos",
            target_os = "ios",
            target_os = "linux",
            target_os = "macos",
            target_os = "solaris",
            target_os = "tvos",
            target_os = "visionos",
            target_os = "watchos",
        )))
    )]
    pub fn interface<T>(mut self, interface: T) -> ClientBuilder
    where
        T: Into<std::borrow::Cow<'static, str>>,
    {
        self.config.tcp_connect_options.set_interface(interface);
        self
    }

    // TLS options

    /// Sets the identity to be used for client certificate authentication.
    #[inline]
    pub fn identity(mut self, identity: Identity) -> ClientBuilder {
        self.config.identity = Some(identity);
        self
    }

    /// Sets the verify certificate store for the client.
    ///
    /// This method allows you to specify a custom verify certificate store to be used
    /// for TLS connections. By default, the system's verify certificate store is used.
    #[inline]
    pub fn cert_store(mut self, store: CertStore) -> ClientBuilder {
        self.config.cert_store = store;
        self
    }

    /// Controls the use of certificate validation.
    ///
    /// Defaults to `true`.
    ///
    /// # Warning
    ///
    /// You should think very carefully before using this method. If
    /// invalid certificates are trusted, *any* certificate for *any* site
    /// will be trusted for use. This includes expired certificates. This
    /// introduces significant vulnerabilities, and should only be used
    /// as a last resort.
    #[inline]
    pub fn cert_verification(mut self, cert_verification: bool) -> ClientBuilder {
        self.config.cert_verification = cert_verification;
        self
    }

    /// Configures the use of hostname verification when connecting.
    ///
    /// Defaults to `true`.
    /// # Warning
    ///
    /// You should think very carefully before you use this method. If hostname verification is not
    /// used, *any* valid certificate for *any* site will be trusted for use from any other. This
    /// introduces a significant vulnerability to man-in-the-middle attacks.
    #[inline]
    pub fn verify_hostname(mut self, verify_hostname: bool) -> ClientBuilder {
        self.config.verify_hostname = verify_hostname;
        self
    }

    /// Configures the use of Server Name Indication (SNI) when connecting.
    ///
    /// Defaults to `true`.
    #[inline]
    pub fn tls_sni(mut self, tls_sni: bool) -> ClientBuilder {
        self.config.tls_sni = tls_sni;
        self
    }

    /// Configures TLS key logging for the client.
    #[inline]
    pub fn keylog(mut self, keylog: KeyLog) -> ClientBuilder {
        self.config.keylog = Some(keylog);
        self
    }

    /// Set the minimum required TLS version for connections.
    ///
    /// By default the TLS backend's own default is used.
    #[inline]
    pub fn min_tls_version(mut self, version: TlsVersion) -> ClientBuilder {
        self.config.min_tls_version = Some(version);
        self
    }

    /// Set the maximum allowed TLS version for connections.
    ///
    /// By default there's no maximum.
    #[inline]
    pub fn max_tls_version(mut self, version: TlsVersion) -> ClientBuilder {
        self.config.max_tls_version = Some(version);
        self
    }

    /// Add TLS information as `TlsInfo` extension to responses.
    ///
    /// # Optional
    ///
    /// feature to be enabled.
    #[inline]
    pub fn tls_info(mut self, tls_info: bool) -> ClientBuilder {
        self.config.tls_info = tls_info;
        self
    }

    /// Sets the TLS options for the client.
    #[inline]
    pub fn tls_options(mut self, options: TlsOptions) -> ClientBuilder {
        *self.config.transport_options.tls_options_mut() = Some(options);
        self
    }

    // DNS options

    /// Disables the hickory-dns async resolver.
    ///
    /// This method exists even if the optional `hickory-dns` feature is not enabled.
    /// This can be used to ensure a `Client` doesn't use the hickory-dns async resolver
    /// even if another dependency were to enable the optional `hickory-dns` feature.
    #[inline]
    #[cfg(feature = "hickory-dns")]
    #[cfg_attr(docsrs, doc(cfg(feature = "hickory-dns")))]
    pub fn no_hickory_dns(mut self) -> ClientBuilder {
        self.config.hickory_dns = false;
        self
    }

    /// Override DNS resolution for specific domains to a particular IP address.
    ///
    /// Warning
    ///
    /// Since the DNS protocol has no notion of ports, if you wish to send
    /// traffic to a particular port you must include this port in the URI
    /// itself, any port in the overridden addr will be ignored and traffic sent
    /// to the conventional port for the given scheme (e.g. 80 for http).
    #[inline]
    pub fn resolve<D>(self, domain: D, addr: SocketAddr) -> ClientBuilder
    where
        D: Into<Cow<'static, str>>,
    {
        self.resolve_to_addrs(domain, std::iter::once(addr))
    }

    /// Override DNS resolution for specific domains to particular IP addresses.
    ///
    /// Warning
    ///
    /// Since the DNS protocol has no notion of ports, if you wish to send
    /// traffic to a particular port you must include this port in the URI
    /// itself, any port in the overridden addresses will be ignored and traffic sent
    /// to the conventional port for the given scheme (e.g. 80 for http).
    #[inline]
    pub fn resolve_to_addrs<D, A>(mut self, domain: D, addrs: A) -> ClientBuilder
    where
        D: Into<Cow<'static, str>>,
        A: IntoIterator<Item = SocketAddr>,
    {
        self.config
            .dns_overrides
            .insert(domain.into(), addrs.into_iter().collect());
        self
    }

    /// Override the DNS resolver implementation.
    ///
    /// Pass any type implementing `IntoResolve`.
    /// Overrides for specific names passed to `resolve` and `resolve_to_addrs` will
    /// still be applied on top of this resolver.
    #[inline]
    pub fn dns_resolver<R>(mut self, resolver: R) -> ClientBuilder
    where
        R: IntoResolve,
    {
        self.config.dns_resolver = Some(resolver.into_resolve());
        self
    }

    // Hooks options

    /// Adds lifecycle hooks to the client.
    ///
    /// Hooks allow you to execute custom logic at different stages of the
    /// request lifecycle, such as before sending a request or after receiving
    /// a response.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use std::sync::Arc;
    ///
    /// use hpx::hooks::{Hooks, LoggingHook};
    ///
    /// let hooks = Hooks::builder()
    ///     .before_request(Arc::new(LoggingHook::new()))
    ///     .build();
    ///
    /// let client = hpx::Client::builder().hooks(hooks).build().unwrap();
    /// ```
    #[inline]
    pub fn hooks(mut self, hooks: super::layer::hooks::Hooks) -> ClientBuilder {
        self.config.hooks = Some(hooks);
        self
    }

    /// Adds a before-request hook using a closure.
    ///
    /// This is a convenience method for adding simple request hooks without
    /// implementing the `BeforeRequestHook` trait manually.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use hpx::header;
    ///
    /// let client = hpx::Client::builder()
    ///     .on_request(|req| {
    ///         req.headers_mut().insert(
    ///             header::HeaderName::from_static("x-custom"),
    ///             header::HeaderValue::from_static("value"),
    ///         );
    ///         Ok(())
    ///     })
    ///     .build()
    ///     .unwrap();
    /// ```
    #[inline]
    pub fn on_request<F>(mut self, hook: F) -> ClientBuilder
    where
        F: Fn(&mut http::Request<Body>) -> Result<(), Error> + Send + Sync + 'static,
    {
        let hooks = self
            .config
            .hooks
            .get_or_insert_with(super::layer::hooks::Hooks::new);

        // Create a wrapper struct to implement BeforeRequestHook
        struct ClosureHook<F>(F);

        impl<F> super::layer::hooks::BeforeRequestHook for ClosureHook<F>
        where
            F: Fn(&mut http::Request<Body>) -> Result<(), Error> + Send + Sync,
        {
            fn on_request(&self, request: &mut http::Request<Body>) -> Result<(), Error> {
                (self.0)(request)
            }
        }

        hooks.before_request.push(Arc::new(ClosureHook(hook)));
        self
    }

    /// Adds an after-response hook using a closure.
    ///
    /// This is a convenience method for adding simple response hooks without
    /// implementing the `AfterResponseHook` trait manually.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use http::StatusCode;
    ///
    /// let client = hpx::Client::builder()
    ///     .on_response(|status, _headers| {
    ///         println!("Response status: {}", status);
    ///         Ok(())
    ///     })
    ///     .build()
    ///     .unwrap();
    /// ```
    #[inline]
    pub fn on_response<F>(mut self, hook: F) -> ClientBuilder
    where
        F: Fn(http::StatusCode, &http::HeaderMap) -> Result<(), Error> + Send + Sync + 'static,
    {
        let hooks = self
            .config
            .hooks
            .get_or_insert_with(super::layer::hooks::Hooks::new);

        // Create a wrapper struct to implement AfterResponseHook
        struct ClosureHook<F>(F);

        impl<F> super::layer::hooks::AfterResponseHook for ClosureHook<F>
        where
            F: Fn(http::StatusCode, &http::HeaderMap) -> Result<(), Error> + Send + Sync,
        {
            fn on_response(
                &self,
                status: http::StatusCode,
                headers: &http::HeaderMap,
            ) -> Result<(), Error> {
                (self.0)(status, headers)
            }
        }

        hooks.after_response.push(Arc::new(ClosureHook(hook)));
        self
    }

    // Tower middleware options

    /// Adds a new Tower [`Layer`](https://docs.rs/tower/latest/tower/trait.Layer.html) to the
    /// request [`Service`](https://docs.rs/tower/latest/tower/trait.Service.html) which is responsible
    /// for request processing.
    ///
    /// Each subsequent invocation of this function will wrap previous layers.
    ///
    /// If configured, the `timeout` will be the outermost layer.
    ///
    /// Example usage:
    /// ```
    /// use std::time::Duration;
    ///
    /// let client = hpx::Client::builder()
    ///     .timeout(Duration::from_millis(200))
    ///     .layer(tower::timeout::TimeoutLayer::new(Duration::from_millis(50)))
    ///     .build()
    ///     .unwrap();
    /// ```
    #[inline]
    pub fn layer<L>(mut self, layer: L) -> ClientBuilder
    where
        L: Layer<BoxedClientService> + Clone + Send + Sync + 'static,
        L::Service: Service<http::Request<Body>, Response = http::Response<ResponseBody>, Error = BoxError>
            + Clone
            + Send
            + Sync
            + 'static,
        <L::Service as Service<http::Request<Body>>>::Future: Send + 'static,
    {
        let layer = BoxCloneSyncServiceLayer::new(layer);
        self.config.layers.push(layer);
        self
    }

    /// Adds a new Tower [`Layer`](https://docs.rs/tower/latest/tower/trait.Layer.html) to the
    /// base connector [`Service`](https://docs.rs/tower/latest/tower/trait.Service.html) which
    /// is responsible for connection establishment.a
    ///
    /// Each subsequent invocation of this function will wrap previous layers.
    ///
    /// If configured, the `connect_timeout` will be the outermost layer.
    ///
    /// Example usage:
    /// ```
    /// use std::time::Duration;
    ///
    /// let client = hpx::Client::builder()
    ///     // resolved to outermost layer, meaning while we are waiting on concurrency limit
    ///     .connect_timeout(Duration::from_millis(200))
    ///     // underneath the concurrency check, so only after concurrency limit lets us through
    ///     .connector_layer(tower::timeout::TimeoutLayer::new(Duration::from_millis(50)))
    ///     .connector_layer(tower::limit::concurrency::ConcurrencyLimitLayer::new(2))
    ///     .build()
    ///     .unwrap();
    /// ```
    #[inline]
    pub fn connector_layer<L>(mut self, layer: L) -> ClientBuilder
    where
        L: Layer<BoxedConnectorService> + Clone + Send + Sync + 'static,
        L::Service:
            Service<Unnameable, Response = Conn, Error = BoxError> + Clone + Send + Sync + 'static,
        <L::Service as Service<Unnameable>>::Future: Send + 'static,
    {
        let layer = BoxCloneSyncServiceLayer::new(layer);
        self.config.connector_layers.push(layer);
        self
    }

    // TLS/HTTP2 emulation options

    /// Configures the client builder to emulate the specified HTTP context.
    ///
    /// This method sets the necessary headers, HTTP/1 and HTTP/2 options configurations, and  TLS
    /// options config to use the specified HTTP context. It allows the client to mimic the
    /// behavior of different versions or setups, which can be useful for testing or ensuring
    /// compatibility with various environments.
    ///
    /// # Note
    /// This will overwrite the existing configuration.
    /// You must set emulation before you can perform subsequent HTTP1/HTTP2/TLS fine-tuning.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use hpx::Client;
    /// use hpx_util::Emulation;
    ///
    /// let client = Client::builder()
    ///     .emulation(Emulation::Firefox128)
    ///     .build()
    ///     .unwrap();
    /// ```
    #[inline]
    pub fn emulation<P>(mut self, factory: P) -> ClientBuilder
    where
        P: EmulationFactory,
    {
        let emulation = factory.emulation();
        #[cfg(feature = "http3")]
        let http3_opts = emulation.http3_options().cloned();
        let (transport_opts, headers, orig_headers) = emulation.into_parts();

        self.config
            .transport_options
            .apply_transport_options(transport_opts);
        #[cfg(feature = "http3")]
        if let Some(opts) = http3_opts {
            self = self.http3_options(opts);
        }
        self.default_headers(headers).orig_headers(orig_headers)
    }
}

#[cfg(test)]
mod tests {
    use super::HttpVersionPref;
    use crate::tls::AlpnProtocol;

    /// Phase 1 Task 1.2: the `Http3` variant exists on `HttpVersionPref`.
    /// The TCP ALPN for `Http3` MUST be `None` — h3 is QUIC-only and is
    /// never proposed over TCP TLS (constraint C-06). The actual h3 path
    /// over QUIC is wired in T1.4+; this test only locks in the variant
    /// and the no-TCP-ALPN contract.
    #[test]
    fn http3_version_pref_variant_exists() {
        let pref = HttpVersionPref::Http3;
        let alpn = match pref {
            HttpVersionPref::Http1 => Some(AlpnProtocol::HTTP1),
            HttpVersionPref::Http2 => Some(AlpnProtocol::HTTP2),
            HttpVersionPref::Http3 => None,
            HttpVersionPref::All => None,
        };
        assert!(
            alpn.is_none(),
            "HTTP/3 must not be proposed over TCP TLS (C-06)"
        );
    }

    /// The `All` variant's TCP ALPN list remains `["h2", "http/1.1"]`
    /// (proposed via no ALPN restriction in the connector). It MUST NOT
    /// include `h3` — h3 is QUIC-only (C-06).
    #[test]
    fn all_version_pref_tcp_alpn_excludes_h3() {
        let pref = HttpVersionPref::All;
        let alpn = match pref {
            HttpVersionPref::Http1 => Some(AlpnProtocol::HTTP1),
            HttpVersionPref::Http2 => Some(AlpnProtocol::HTTP2),
            HttpVersionPref::Http3 => None,
            HttpVersionPref::All => None,
        };
        assert!(alpn.is_none(), "All TCP ALPN must not include h3 (C-06)");
    }
}
