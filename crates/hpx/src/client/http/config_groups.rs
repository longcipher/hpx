use std::{num::NonZeroU32, time::Duration};

use crate::{
    Proxy,
    client::{
        conn::TcpConnectOptions,
        layer::{recovery::Recoveries, timeout::TimeoutOptions},
    },
    redirect, retry,
    tls::{CertStore, Identity, KeyLog, TlsVersion},
};

/// HTTP protocol preference for grouped client configuration.
#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub enum HttpVersionPreference {
    /// Only negotiate HTTP/1.
    Http1,
    /// Only negotiate HTTP/2.
    Http2,
    /// Allow the client to negotiate the best supported protocol.
    #[default]
    All,
}

/// Reusable transport-layer settings for a [`crate::ClientBuilder`].
///
/// Applying a grouped config replaces the current transport group on the builder.
#[must_use]
#[derive(Clone)]
pub struct TransportConfigOptions {
    pub(crate) connect_timeout: Option<Duration>,
    pub(crate) connection_verbose: bool,
    pub(crate) tcp_nodelay: bool,
    pub(crate) tcp_reuse_address: bool,
    pub(crate) tcp_keepalive: Option<Duration>,
    pub(crate) tcp_keepalive_interval: Option<Duration>,
    pub(crate) tcp_keepalive_retries: Option<u32>,
    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    pub(crate) tcp_user_timeout: Option<Duration>,
    pub(crate) tcp_send_buffer_size: Option<usize>,
    pub(crate) tcp_recv_buffer_size: Option<usize>,
    pub(crate) tcp_happy_eyeballs_timeout: Option<Duration>,
    pub(crate) tcp_connect_options: TcpConnectOptions,
}

impl Default for TransportConfigOptions {
    fn default() -> Self {
        Self {
            connect_timeout: None,
            connection_verbose: false,
            tcp_nodelay: true,
            tcp_reuse_address: false,
            tcp_keepalive: Some(Duration::from_secs(15)),
            tcp_keepalive_interval: Some(Duration::from_secs(15)),
            tcp_keepalive_retries: Some(3),
            #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
            tcp_user_timeout: Some(Duration::from_secs(30)),
            tcp_send_buffer_size: None,
            tcp_recv_buffer_size: None,
            tcp_happy_eyeballs_timeout: Some(Duration::from_millis(300)),
            tcp_connect_options: TcpConnectOptions::default(),
        }
    }
}

impl TransportConfigOptions {
    /// Create a transport config with the client defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the connector timeout used for TCP and TLS establishment.
    pub fn connect_timeout<D>(mut self, timeout: D) -> Self
    where
        D: Into<Option<Duration>>,
    {
        self.connect_timeout = timeout.into();
        self
    }

    /// Enable or disable verbose transport logging.
    pub fn connection_verbose(mut self, verbose: bool) -> Self {
        self.connection_verbose = verbose;
        self
    }

    /// Set whether sockets use `TCP_NODELAY`.
    pub fn tcp_nodelay(mut self, enabled: bool) -> Self {
        self.tcp_nodelay = enabled;
        self
    }

    /// Set whether sockets use `SO_REUSEADDR`.
    pub fn tcp_reuse_address(mut self, enabled: bool) -> Self {
        self.tcp_reuse_address = enabled;
        self
    }

    /// Set the keepalive duration.
    pub fn tcp_keepalive<D>(mut self, value: D) -> Self
    where
        D: Into<Option<Duration>>,
    {
        self.tcp_keepalive = value.into();
        self
    }

    /// Set the keepalive interval.
    pub fn tcp_keepalive_interval<D>(mut self, value: D) -> Self
    where
        D: Into<Option<Duration>>,
    {
        self.tcp_keepalive_interval = value.into();
        self
    }

    /// Set the keepalive retry count.
    pub fn tcp_keepalive_retries<C>(mut self, retries: C) -> Self
    where
        C: Into<Option<u32>>,
    {
        self.tcp_keepalive_retries = retries.into();
        self
    }

    /// Set the Linux-family `TCP_USER_TIMEOUT`.
    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    #[cfg_attr(
        docsrs,
        doc(cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux")))
    )]
    pub fn tcp_user_timeout<D>(mut self, value: D) -> Self
    where
        D: Into<Option<Duration>>,
    {
        self.tcp_user_timeout = value.into();
        self
    }

    /// Set the TCP send buffer size.
    pub fn tcp_send_buffer_size<S>(mut self, size: S) -> Self
    where
        S: Into<Option<usize>>,
    {
        self.tcp_send_buffer_size = size.into();
        self
    }

    /// Set the TCP receive buffer size.
    pub fn tcp_recv_buffer_size<S>(mut self, size: S) -> Self
    where
        S: Into<Option<usize>>,
    {
        self.tcp_recv_buffer_size = size.into();
        self
    }

    /// Set the Happy Eyeballs fallback timeout.
    pub fn tcp_happy_eyeballs_timeout<D>(mut self, value: D) -> Self
    where
        D: Into<Option<Duration>>,
    {
        self.tcp_happy_eyeballs_timeout = value.into();
        self
    }

    /// Replace the low-level TCP connect options.
    pub fn tcp_connect_options(mut self, options: TcpConnectOptions) -> Self {
        self.tcp_connect_options = options;
        self
    }
}

/// Reusable connection-pool settings for a [`crate::ClientBuilder`].
#[must_use]
#[derive(Clone)]
pub struct PoolConfigOptions {
    pub(crate) idle_timeout: Option<Duration>,
    pub(crate) max_idle_per_host: usize,
    pub(crate) max_size: Option<NonZeroU32>,
}

impl Default for PoolConfigOptions {
    fn default() -> Self {
        Self {
            idle_timeout: Some(Duration::from_secs(90)),
            max_idle_per_host: usize::MAX,
            max_size: None,
        }
    }
}

impl PoolConfigOptions {
    /// Create a pool config with the client defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the idle timeout for pooled connections.
    pub fn idle_timeout<D>(mut self, value: D) -> Self
    where
        D: Into<Option<Duration>>,
    {
        self.idle_timeout = value.into();
        self
    }

    /// Set the maximum idle connections per host.
    pub fn max_idle_per_host(mut self, max: usize) -> Self {
        self.max_idle_per_host = max;
        self
    }

    /// Set the maximum pool size.
    pub fn max_size(mut self, max: Option<u32>) -> Self {
        self.max_size = max.and_then(NonZeroU32::new);
        self
    }
}

/// Reusable TLS settings for a [`crate::ClientBuilder`].
#[must_use]
#[derive(Clone)]
pub struct TlsConfigOptions {
    pub(crate) keylog: Option<KeyLog>,
    pub(crate) tls_info: bool,
    pub(crate) tls_sni: bool,
    pub(crate) verify_hostname: bool,
    pub(crate) identity: Option<Identity>,
    pub(crate) cert_store: CertStore,
    pub(crate) cert_verification: bool,
    pub(crate) min_version: Option<TlsVersion>,
    pub(crate) max_version: Option<TlsVersion>,
}

impl Default for TlsConfigOptions {
    fn default() -> Self {
        Self {
            keylog: None,
            tls_info: false,
            tls_sni: true,
            verify_hostname: true,
            identity: None,
            cert_store: CertStore::default(),
            cert_verification: true,
            min_version: None,
            max_version: None,
        }
    }
}

impl TlsConfigOptions {
    /// Create a TLS config with the client defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the client identity used for mutual TLS.
    pub fn identity(mut self, identity: Identity) -> Self {
        self.identity = Some(identity);
        self
    }

    /// Replace the certificate store.
    pub fn cert_store(mut self, store: CertStore) -> Self {
        self.cert_store = store;
        self
    }

    /// Enable or disable certificate verification.
    pub fn cert_verification(mut self, enabled: bool) -> Self {
        self.cert_verification = enabled;
        self
    }

    /// Enable or disable hostname verification.
    pub fn verify_hostname(mut self, enabled: bool) -> Self {
        self.verify_hostname = enabled;
        self
    }

    /// Enable or disable SNI.
    pub fn tls_sni(mut self, enabled: bool) -> Self {
        self.tls_sni = enabled;
        self
    }

    /// Enable TLS info extensions on responses.
    pub fn tls_info(mut self, enabled: bool) -> Self {
        self.tls_info = enabled;
        self
    }

    /// Configure TLS key logging.
    pub fn keylog(mut self, keylog: KeyLog) -> Self {
        self.keylog = Some(keylog);
        self
    }

    /// Set the minimum TLS version.
    pub fn min_tls_version(mut self, version: TlsVersion) -> Self {
        self.min_version = Some(version);
        self
    }

    /// Set the maximum TLS version.
    pub fn max_tls_version(mut self, version: TlsVersion) -> Self {
        self.max_version = Some(version);
        self
    }
}

/// Reusable HTTP protocol settings for a [`crate::ClientBuilder`].
#[must_use]
#[derive(Clone)]
pub struct ProtocolConfigOptions {
    pub(crate) http_version_preference: HttpVersionPreference,
    pub(crate) https_only: bool,
    pub(crate) retry_policy: retry::Policy,
    pub(crate) redirect_policy: redirect::Policy,
    pub(crate) referer: bool,
    pub(crate) timeout_options: TimeoutOptions,
    pub(crate) recoveries: Recoveries,
}

impl Default for ProtocolConfigOptions {
    fn default() -> Self {
        Self {
            http_version_preference: HttpVersionPreference::All,
            https_only: false,
            retry_policy: retry::Policy::default(),
            redirect_policy: redirect::Policy::none(),
            referer: true,
            timeout_options: TimeoutOptions::default(),
            recoveries: Recoveries::new(),
        }
    }
}

impl ProtocolConfigOptions {
    /// Create a protocol config with the client defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the preferred HTTP version.
    pub fn http_version_preference(mut self, preference: HttpVersionPreference) -> Self {
        self.http_version_preference = preference;
        self
    }

    /// Restrict the client to HTTPS requests only.
    pub fn https_only(mut self, enabled: bool) -> Self {
        self.https_only = enabled;
        self
    }

    /// Set the retry policy.
    pub fn retry_policy(mut self, policy: retry::Policy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Set the redirect policy.
    pub fn redirect_policy(mut self, policy: redirect::Policy) -> Self {
        self.redirect_policy = policy;
        self
    }

    /// Enable or disable automatic referer propagation.
    pub fn referer(mut self, enabled: bool) -> Self {
        self.referer = enabled;
        self
    }

    /// Replace the grouped timeout settings.
    pub fn timeout_options(mut self, options: TimeoutOptions) -> Self {
        self.timeout_options = options;
        self
    }

    /// Replace the grouped recovery hooks.
    pub fn recoveries(mut self, recoveries: Recoveries) -> Self {
        self.recoveries = recoveries;
        self
    }
}

/// Reusable proxy settings for a [`crate::ClientBuilder`].
#[must_use]
#[derive(Clone)]
pub struct ProxyConfigOptions {
    pub(crate) proxies: Vec<Proxy>,
    pub(crate) auto_system_proxy: bool,
}

impl Default for ProxyConfigOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl ProxyConfigOptions {
    /// Create a proxy config with the client defaults.
    pub fn new() -> Self {
        Self {
            proxies: Vec::new(),
            auto_system_proxy: true,
        }
    }

    /// Enable or disable automatic system proxy detection.
    pub fn system_proxy(mut self, enabled: bool) -> Self {
        self.auto_system_proxy = enabled;
        self
    }

    /// Add a proxy rule.
    pub fn proxy(mut self, proxy: Proxy) -> Self {
        self.proxies.push(proxy);
        self.auto_system_proxy = false;
        self
    }

    /// Replace the proxy list.
    pub fn proxies<I>(mut self, proxies: I) -> Self
    where
        I: IntoIterator<Item = Proxy>,
    {
        self.proxies = proxies.into_iter().collect();
        self.auto_system_proxy = false;
        self
    }

    /// Clear all proxy rules and disable system proxy detection.
    pub fn no_proxy(mut self) -> Self {
        self.proxies.clear();
        self.auto_system_proxy = false;
        self
    }
}
