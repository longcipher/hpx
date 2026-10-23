pub(crate) mod auth;
#[cfg(all(target_os = "macos", feature = "system-proxy"))]
mod mac;
#[cfg(feature = "pac")]
pub(crate) mod pac;
#[cfg(any(windows, target_os = "macos", test))]
mod sysproxy;
#[cfg(unix)]
mod uds;
#[cfg(all(windows, feature = "system-proxy"))]
mod win;

pub(crate) mod matcher;

#[cfg(unix)]
use std::path::Path;
use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

pub use auth::ProxyAuthMethod;
use auth::ProxyCredentials;
use http::{HeaderMap, Uri, header::HeaderValue};

use crate::{IntoUri, ext::UriExt};

// # Internals
//
// This module is a couple pieces:
//
// - The public builder API
// - The internal built types that our Connector knows how to use.
//
// The user creates a builder (`hpx::Proxy`), and configures any extras.
// Once that type is passed to the `ClientBuilder`, we convert it into the
// built matcher types, making use of `core`'s matchers.

/// Configuration of a proxy that a `Client` should pass requests to.
///
/// A `Proxy` has a couple pieces to it:
///
/// - a URI of how to talk to the proxy
/// - rules on what `Client` requests should be directed to the proxy
///
/// For instance, let's look at `Proxy::http`:
///
/// ```rust
/// # fn run() -> Result<(), Box<dyn std::error::Error>> {
/// let proxy = hpx::Proxy::http("https://secure.example")?;
/// # Ok(())
/// # }
/// ```
///
/// This proxy will intercept all HTTP requests, and make use of the proxy
/// at `https://secure.example`. A request to `http://hyper.rs` will talk
/// to your proxy. A request to `https://hyper.rs` will not.
///
/// Multiple `Proxy` rules can be configured for a `Client`. The `Client` will
/// check each `Proxy` in the order it was added. This could mean that a
/// `Proxy` added first with eager intercept rules, such as `Proxy::all`,
/// would prevent a `Proxy` later in the list from ever working, so take care.
///
/// By enabling the `"socks"` feature it is possible to use a socks proxy:
/// ```rust
/// # fn run() -> Result<(), Box<dyn std::error::Error>> {
/// let proxy = hpx::Proxy::http("socks5://192.168.1.1:9000")?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Proxy {
    extra: Extra,
    scheme: ProxyScheme,
    exclusions: Option<NoProxy>,
}

/// A configuration for filtering out requests that shouldn't be proxied
#[derive(Clone, Debug, Default)]
pub struct NoProxy {
    inner: String,
}

// ===== Internal =====

#[expect(clippy::large_enum_variant)]
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Intercepted {
    Proxy(matcher::Intercept),
    #[cfg(unix)]
    Unix(Arc<Path>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Matcher {
    inner: Box<matcher::Matcher>,
}

#[derive(Clone, Debug)]
enum ProxyScheme {
    All(Uri),
    Http(Uri),
    Https(Uri),
    #[cfg(unix)]
    Unix(Arc<Path>),
    Custom(CustomProxy),
    /// A Proxy Auto-Config script, evaluated for every request.
    #[cfg(feature = "pac")]
    Pac(Arc<pac::PacRule>),
}

/// The type of a user-supplied proxy selector.
///
/// The selector receives the destination URI of the request and returns the
/// proxy endpoint to use for it, or `None` to send the request directly.
type CustomProxyFn = dyn Fn(&Uri) -> Option<Uri> + Send + Sync;

/// A closure-backed proxy rule.
///
/// The closure receives the destination URI of the request being sent and
/// returns the proxy endpoint that should be used for it, or `None` to send
/// the request directly.
///
/// `CustomProxy` is compared and hashed by the identity of the closure
/// instance rather than structurally. This keeps the connection-pool key
/// well defined for closure-backed proxies: two separately created custom
/// proxies never share pooled connections.
#[derive(Clone)]
pub(crate) struct CustomProxy {
    /// Identity token, used for `Hash`/`Eq`.
    token: Arc<()>,
    /// The user-supplied selector.
    func: Arc<CustomProxyFn>,
}

impl CustomProxy {
    /// Select the proxy endpoint for `dst`, if any.
    pub(crate) fn select(&self, dst: &Uri) -> Option<Uri> {
        (self.func)(dst)
    }
}

impl std::fmt::Debug for CustomProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomProxy").finish_non_exhaustive()
    }
}

impl Hash for CustomProxy {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Hash by the identity of the closure instance. Two `CustomProxy`
        // values created by different [`Proxy::custom`] calls are never
        // equal, and clones of the same one always are.
        (Arc::as_ptr(&self.token) as usize).hash(state);
    }
}

impl PartialEq for CustomProxy {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.token, &other.token)
    }
}

impl Eq for CustomProxy {}

/// A value that can be used as a proxy endpoint.
///
/// This trait is implemented for string types and for [`http::Uri`]. It is
/// used by [`Proxy::custom`] so that a selector closure may return whichever
/// representation is most convenient.
pub trait IntoProxy: IntoProxySealed {}

impl IntoProxy for &str {}
impl IntoProxy for String {}
impl IntoProxy for &String {}
impl IntoProxy for Uri {}
impl IntoProxy for &Uri {}

/// Sealed conversion trait backing [`IntoProxy`].
pub trait IntoProxySealed {
    /// Convert the value into a proxy endpoint, or `None` when the value is
    /// not a usable proxy URI.
    #[doc(hidden)]
    fn into_proxy(self) -> Option<Uri>;
}

impl IntoProxySealed for &str {
    fn into_proxy(self) -> Option<Uri> {
        let trimmed = self.trim();
        if trimmed.is_empty() {
            return None;
        }

        match Uri::try_from(trimmed) {
            // Absolute form: the scheme is authoritative.
            Ok(uri) if uri.scheme().is_some() => Some(uri),
            // Authority-only form (for example `proxy.local:8080`): treat it
            // as a plain HTTP proxy.
            Ok(uri) => uri.authority().and_then(|authority| {
                Uri::builder()
                    .scheme("http")
                    .authority(authority.clone())
                    .path_and_query("/")
                    .build()
                    .ok()
            }),
            Err(_) => None,
        }
    }
}

impl IntoProxySealed for String {
    fn into_proxy(self) -> Option<Uri> {
        self.as_str().into_proxy()
    }
}

impl IntoProxySealed for &String {
    fn into_proxy(self) -> Option<Uri> {
        self.as_str().into_proxy()
    }
}

impl IntoProxySealed for Uri {
    fn into_proxy(self) -> Option<Uri> {
        Some(self)
    }
}

impl IntoProxySealed for &Uri {
    fn into_proxy(self) -> Option<Uri> {
        Some(self.clone())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Extra {
    /// The preemptive `Proxy-Authorization` header value, if any.
    auth: Option<HeaderValue>,
    /// The credentials used to answer a proxy authentication challenge.
    credentials: Option<ProxyCredentials>,
    misc: Option<HeaderMap>,
}

// ===== impl Proxy =====

impl Proxy {
    /// Proxy all HTTP traffic to the passed URI.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate hpx;
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = hpx::Client::builder()
    ///     .proxy(hpx::Proxy::http("https://my.prox")?)
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// # fn main() {}
    /// ```
    pub fn http<U: IntoUri>(uri: U) -> crate::Result<Self> {
        uri.into_uri().map(ProxyScheme::Http).map(Self::new)
    }

    /// Proxy all HTTPS traffic to the passed URI.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate hpx;
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = hpx::Client::builder()
    ///     .proxy(hpx::Proxy::https("https://example.prox:4545")?)
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// # fn main() {}
    /// ```
    pub fn https<U: IntoUri>(uri: U) -> crate::Result<Self> {
        uri.into_uri().map(ProxyScheme::Https).map(Self::new)
    }

    /// Proxy **all** traffic to the passed URI.
    ///
    /// "All" refers to `https` and `http` URIs. Other schemes are not
    /// recognized by hpx.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate hpx;
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = hpx::Client::builder()
    ///     .proxy(hpx::Proxy::all("http://pro.xy")?)
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// # fn main() {}
    /// ```
    pub fn all<U: IntoUri>(uri: U) -> crate::Result<Self> {
        uri.into_uri().map(ProxyScheme::All).map(Self::new)
    }

    /// Provide a custom function to determine which traffic goes through which proxy.
    ///
    /// The closure is invoked with the URI of the request that is about to be
    /// sent and returns the proxy endpoint to use for it. Returning `None`
    /// sends the request directly, without a proxy.
    ///
    /// This is the most flexible proxy rule: it can route by host, by port,
    /// by path, by any property of the request URI.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate hpx;
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = hpx::Client::builder()
    ///     .proxy(hpx::Proxy::custom(move |uri: &http::Uri| {
    ///         if uri.host() == Some("internal.example") {
    ///             Some("http://proxy.internal:8080")
    ///         } else {
    ///             None
    ///         }
    ///     }))
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// # fn main() {}
    /// ```
    ///
    /// # Note
    ///
    /// The closure is invoked once per request that reaches the connection
    /// layer. It must be cheap and must not block.
    pub fn custom<F, U>(fun: F) -> Self
    where
        F: Fn(&Uri) -> Option<U> + Send + Sync + 'static,
        U: IntoProxy,
    {
        let func = move |uri: &Uri| fun(uri).and_then(IntoProxySealed::into_proxy);

        Self::new(ProxyScheme::Custom(CustomProxy {
            token: Arc::new(()),
            func: Arc::new(func),
        }))
    }

    /// Build a proxy from the standard environment variables.
    ///
    /// The following variables are consulted, in this order:
    ///
    /// * `ALL_PROXY` / `all_proxy` — used for every request.
    /// * `HTTP_PROXY` / `http_proxy` — used for plain HTTP requests.
    /// * `HTTPS_PROXY` / `https_proxy` — used for HTTPS requests.
    ///
    /// `NO_PROXY` / `no_proxy` is applied as the exclusion list of the
    /// returned proxy.
    ///
    /// Returns `None` when none of the variables is set, when the value
    /// cannot be parsed as a proxy endpoint, or when the process is running
    /// as a CGI script (in which case `REQUEST_METHOD` is present and
    /// proxying would break the script).
    ///
    /// # Note
    ///
    /// This returns a **single** rule. When `HTTP_PROXY` and `HTTPS_PROXY`
    /// are both set, use [`crate::ClientBuilder::proxies_from_env`] to
    /// configure a client with the full precedence rules.
    ///
    /// # Example
    /// ```
    /// # extern crate hpx;
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// if let Some(proxy) = hpx::Proxy::from_env() {
    ///     let client = hpx::Client::builder().proxy(proxy).build()?;
    /// }
    /// # Ok(())
    /// # }
    /// # fn main() {}
    /// ```
    pub fn from_env() -> Option<Self> {
        // A CGI script must never be proxied: the `REQUEST_METHOD`
        // environment variable is set by the web server and proxying would
        // break the script.
        if std::env::var_os("REQUEST_METHOD").is_some() {
            return None;
        }

        Self::from_env_source(&ProcessEnv)
    }

    /// Build a proxy rule from an environment source.
    fn from_env_source(env: &impl EnvSource) -> Option<Self> {
        let http = env.get(&["HTTP_PROXY", "http_proxy"]);
        let https = env.get(&["HTTPS_PROXY", "https_proxy"]);
        let all = env.get(&["ALL_PROXY", "all_proxy"]);
        let no = env.get(&["NO_PROXY", "no_proxy"]);

        Self::from_env_values(
            http.as_deref(),
            https.as_deref(),
            all.as_deref(),
            no.as_deref(),
        )
    }

    /// Build a proxy rule from already-resolved environment values.
    ///
    /// This is the pure core of [`Proxy::from_env`]; it is separated out so
    /// the precedence rules can be tested without mutating the process
    /// environment.
    fn from_env_values(
        http: Option<&str>,
        https: Option<&str>,
        all: Option<&str>,
        no: Option<&str>,
    ) -> Option<Self> {
        let no_proxy = no.map(|raw| NoProxy::from_string(raw).unwrap_or_default());

        if let Some(endpoint) = all {
            return Self::all(normalize_endpoint(endpoint))
                .ok()
                .map(|p| p.no_proxy(no_proxy));
        }

        // Only one of the scheme-specific variables is set: express the
        // rule with the matching scheme so it only intercepts the traffic it
        // is meant for.
        match (http, https) {
            (Some(http), None) => Self::http(normalize_endpoint(http))
                .ok()
                .map(|p| p.no_proxy(no_proxy)),
            (None, Some(https)) => Self::https(normalize_endpoint(https))
                .ok()
                .map(|p| p.no_proxy(no_proxy)),
            // Both or neither are set: `ALL_PROXY` handled above covers the
            // "both" case, and "neither" means there is nothing to do.
            _ => None,
        }
    }

    /// Build every proxy rule described by the standard environment variables.
    ///
    /// The rules are returned in precedence order: the HTTP rule first, then
    /// the HTTPS rule, then the catch-all rule. A client that is given these
    /// rules in this order routes plain HTTP through `HTTP_PROXY`, HTTPS
    /// through `HTTPS_PROXY`, and falls back to `ALL_PROXY` for whatever is
    /// left. `NO_PROXY` is applied to every rule.
    pub(crate) fn from_env_all() -> Vec<Self> {
        // A CGI script must never be proxied.
        if std::env::var_os("REQUEST_METHOD").is_some() {
            return Vec::new();
        }

        Self::from_env_all_source(&ProcessEnv)
    }

    /// Build every proxy rule from an environment source.
    fn from_env_all_source(env: &impl EnvSource) -> Vec<Self> {
        let http = env.get(&["HTTP_PROXY", "http_proxy"]);
        let https = env.get(&["HTTPS_PROXY", "https_proxy"]);
        let all = env.get(&["ALL_PROXY", "all_proxy"]);
        let no = env.get(&["NO_PROXY", "no_proxy"]);

        Self::from_env_all_values(
            http.as_deref(),
            https.as_deref(),
            all.as_deref(),
            no.as_deref(),
        )
    }

    /// Build every proxy rule from already-resolved environment values.
    fn from_env_all_values(
        http: Option<&str>,
        https: Option<&str>,
        all: Option<&str>,
        no: Option<&str>,
    ) -> Vec<Self> {
        let no_proxy = no.map(|raw| NoProxy::from_string(raw).unwrap_or_default());
        let mut proxies = Vec::new();

        if let Some(endpoint) = http
            && let Ok(proxy) = Self::http(normalize_endpoint(endpoint))
        {
            proxies.push(proxy.no_proxy(no_proxy.clone()));
        }

        if let Some(endpoint) = https
            && let Ok(proxy) = Self::https(normalize_endpoint(endpoint))
        {
            proxies.push(proxy.no_proxy(no_proxy.clone()));
        }

        if let Some(endpoint) = all
            && let Ok(proxy) = Self::all(normalize_endpoint(endpoint))
        {
            proxies.push(proxy.no_proxy(no_proxy));
        }

        proxies
    }

    /// Proxy all traffic through a Proxy Auto-Config (PAC) script.
    ///
    /// The script must define a `FindProxyForURL(url, host)` function that
    /// returns the proxies to use, for example
    /// `"PROXY proxy.local:8080; DIRECT"`. The script is evaluated for
    /// every request, because the result can depend on the URL.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate hpx;
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let script = r#"
    ///     function FindProxyForURL(url, host) {
    ///         if (dnsDomainIs(host, ".example.com")) {
    ///             return "PROXY proxy.example.com:8080";
    ///         }
    ///         return "DIRECT";
    ///     }
    /// "#;
    /// let proxy = hpx::Proxy::pac(script)?;
    /// # Ok(())
    /// # }
    /// # fn main() {}
    /// ```
    #[cfg(feature = "pac")]
    pub fn pac(script: &str) -> crate::Result<Self> {
        let rule = pac::PacRule::new(script)
            .map_err(|err| crate::Error::builder(format!("invalid PAC script: {err}")))?;
        Ok(Self::new(ProxyScheme::Pac(Arc::new(rule))))
    }

    /// Proxy all traffic to the passed Unix Domain Socket path.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate hpx;
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = hpx::Client::builder()
    ///     .proxy(hpx::Proxy::unix("/var/run/docker.sock")?)
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// # fn main() {}
    /// ```
    #[cfg(unix)]
    pub fn unix<P: uds::IntoUnixSocket>(unix: P) -> crate::Result<Self> {
        Ok(Self::new(ProxyScheme::Unix(unix.unix_socket())))
    }

    const fn new(scheme: ProxyScheme) -> Self {
        Self {
            extra: Extra {
                auth: None,
                credentials: None,
                misc: None,
            },
            scheme,
            exclusions: None,
        }
    }

    /// Set the `Proxy-Authorization` header using Basic auth.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate hpx;
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let proxy = hpx::Proxy::https("http://localhost:1234")?.basic_auth("Aladdin", "open sesame");
    /// # Ok(())
    /// # }
    /// # fn main() {}
    /// ```
    pub fn basic_auth(self, username: &str, password: &str) -> Self {
        self.auth(username, password, ProxyAuthMethod::Basic)
    }

    /// Set the credentials used to authenticate with the proxy.
    ///
    /// The authentication scheme decides how the credentials are used:
    ///
    /// * [`ProxyAuthMethod::Basic`] sends the credentials preemptively in
    ///   every `Proxy-Authorization` header.
    /// * [`ProxyAuthMethod::Digest`] answers a `407 Proxy Authentication
    ///   Required` challenge with a `Digest` response (RFC 7616).
    /// * [`ProxyAuthMethod::Ntlm`] answers a `407` challenge with the
    ///   NTLM handshake.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate hpx;
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// use hpx::{Proxy, ProxyAuthMethod};
    ///
    /// let proxy = Proxy::https("http://localhost:1234")?.auth(
    ///     "Aladdin",
    ///     "open sesame",
    ///     ProxyAuthMethod::Digest,
    /// );
    /// # Ok(())
    /// # }
    /// # fn main() {}
    /// ```
    pub fn auth(mut self, username: &str, password: &str, method: ProxyAuthMethod) -> Self {
        self.extra.credentials = Some(ProxyCredentials::new(method, username, password));

        // Basic authentication is sent preemptively, so the header value can
        // be prepared up front.
        if method == ProxyAuthMethod::Basic {
            self.extra.auth = Some(auth::basic_value(username, password));
        }

        // The userinfo is kept so that SOCKS proxies, which authenticate
        // with the raw credentials, keep working.
        match self.scheme {
            ProxyScheme::All(ref mut uri)
            | ProxyScheme::Http(ref mut uri)
            | ProxyScheme::Https(ref mut uri) => {
                uri.set_userinfo(username, Some(password));
            }
            #[cfg(unix)]
            ProxyScheme::Unix(_) => {}
            ProxyScheme::Custom(_) => {}
            #[cfg(feature = "pac")]
            ProxyScheme::Pac(_) => {}
        }

        self
    }

    /// Set the `Proxy-Authorization` header to a specified value.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate hpx;
    /// # use hpx::header::*;
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let proxy = hpx::Proxy::https("http://localhost:1234")?
    ///     .custom_http_auth(HeaderValue::from_static("justletmeinalreadyplease"));
    /// # Ok(())
    /// # }
    /// # fn main() {}
    /// ```
    pub fn custom_http_auth(mut self, header_value: HeaderValue) -> Self {
        self.extra.auth = Some(header_value);
        self
    }

    /// Adds a Custom Headers to Proxy
    /// Adds custom headers to this Proxy
    ///
    /// # Example
    /// ```
    /// # extern crate hpx;
    /// # use hpx::header::*;
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut headers = HeaderMap::new();
    /// headers.insert(USER_AGENT, "hpx".parse().unwrap());
    /// let proxy = hpx::Proxy::https("http://localhost:1234")?.custom_http_headers(headers);
    /// # Ok(())
    /// # }
    /// # fn main() {}
    /// ```
    pub fn custom_http_headers(mut self, headers: HeaderMap) -> Self {
        match self.scheme {
            ProxyScheme::All(_) | ProxyScheme::Http(_) | ProxyScheme::Https(_) => {
                self.extra.misc = Some(headers);
            }
            // A custom rule selects the proxy endpoint per request, so the
            // headers are applied to whichever endpoint the rule returns.
            ProxyScheme::Custom(_) => {
                self.extra.misc = Some(headers);
            }
            #[cfg(unix)]
            ProxyScheme::Unix(_) => {
                // For Unix sockets, we don't set custom headers.
                // This is a no-op, but keeps the API consistent.
            }
            #[cfg(feature = "pac")]
            ProxyScheme::Pac(_) => {
                // A PAC script selects the proxy per request, so the
                // headers are applied to whichever proxy it selects.
            }
        }

        self
    }

    /// Adds a `No Proxy` exclusion list to this Proxy
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate hpx;
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let proxy = hpx::Proxy::https("http://localhost:1234")?
    ///     .no_proxy(hpx::NoProxy::from_string("direct.tld, sub.direct2.tld"));
    /// # Ok(())
    /// # }
    /// # fn main() {}
    /// ```
    pub fn no_proxy(mut self, no_proxy: Option<NoProxy>) -> Self {
        self.exclusions = no_proxy;
        self
    }

    pub(crate) fn into_matcher(self) -> Matcher {
        let Self {
            scheme,
            extra,
            exclusions,
        } = self;

        let no_proxy = exclusions.as_ref().map_or("", |n| n.inner.as_ref());

        let inner = match scheme {
            ProxyScheme::All(uri) => matcher::Matcher::builder()
                .all(uri.to_string())
                .no(no_proxy)
                .build(extra),
            ProxyScheme::Http(uri) => matcher::Matcher::builder()
                .http(uri.to_string())
                .no(no_proxy)
                .build(extra),
            ProxyScheme::Https(uri) => matcher::Matcher::builder()
                .https(uri.to_string())
                .no(no_proxy)
                .build(extra),
            #[cfg(unix)]
            ProxyScheme::Unix(unix) => matcher::Matcher::builder()
                .unix(unix)
                .no(no_proxy)
                .build(extra),
            ProxyScheme::Custom(custom) => matcher::Matcher::builder()
                .custom(custom)
                .no(no_proxy)
                .build(extra),
            #[cfg(feature = "pac")]
            ProxyScheme::Pac(rule) => matcher::Matcher::builder()
                .pac((*rule).clone())
                .no(no_proxy)
                .build(extra),
        };

        Matcher {
            inner: Box::new(inner),
        }
    }
}

// ===== impl NoProxy =====

/// Read the first environment variable in `names` that is set to a non-empty value.
fn first_env(names: &[&str]) -> Option<String> {
    for name in names {
        if let Ok(value) = std::env::var(name) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    None
}

/// A source of environment variables.
///
/// The environment-driven proxy rules read their values through this trait so
/// the precedence rules can be tested without mutating the process
/// environment, which would race with the other tests in the same binary.
pub(crate) trait EnvSource {
    /// Returns the first non-empty value among `names`.
    fn get(&self, names: &[&str]) -> Option<String>;
}

/// The real process environment.
pub(crate) struct ProcessEnv;

impl EnvSource for ProcessEnv {
    fn get(&self, names: &[&str]) -> Option<String> {
        first_env(names)
    }
}

/// Normalize a proxy endpoint that may omit the scheme.
///
/// Proxy environment variables commonly carry `host:port` without a scheme;
/// such values mean a plain HTTP proxy. A value that contains `://` already
/// carries an explicit scheme (`http://`, `https://`, `socks5://`, ...) and
/// is left untouched.
pub(super) fn normalize_endpoint(endpoint: &str) -> String {
    let trimmed = endpoint.trim();
    if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("http://{trimmed}")
    }
}

impl NoProxy {
    /// Returns a new no-proxy configuration based on environment variables (or `None` if no
    /// variables are set) see [self::NoProxy::from_string()] for the string format
    pub fn from_env() -> Option<Self> {
        Self::from_env_source(&ProcessEnv)
    }

    /// Build a no-proxy list from an environment source.
    fn from_env_source(env: &impl EnvSource) -> Option<Self> {
        let raw = env.get(&["NO_PROXY", "no_proxy"])?;

        // Per the docs, this returns `None` if no environment variable is set. We can only reach
        // here if an env var is set, so we return `Some(NoProxy::default)` if `from_string`
        // returns None, which occurs with an empty string.
        Some(Self::from_string(&raw).unwrap_or_default())
    }

    /// Returns a new no-proxy configuration based on a `no_proxy` string (or `None` if no variables
    /// are set)
    /// The rules are as follows:
    /// * The environment variable `NO_PROXY` is checked, if it is not set, `no_proxy` is checked
    /// * If neither environment variable is set, `None` is returned
    /// * Entries are expected to be comma-separated (whitespace between entries is ignored)
    /// * IP addresses (both IPv4 and IPv6) are allowed, as are optional subnet masks (by adding
    ///   /size, for example "`192.168.1.0/24`").
    /// * An entry "`*`" matches all hostnames (this is the only wildcard allowed)
    /// * Any other entry is considered a domain name (and may contain a leading dot, for example
    ///   `google.com` and `.google.com` are equivalent) and would match both that domain AND all
    ///   subdomains.
    ///
    /// For example, if `"NO_PROXY=google.com, 192.168.1.0/24"` was set, all the following would
    /// match (and therefore would bypass the proxy):
    /// * `http://google.com/`
    /// * `http://www.google.com/`
    /// * `http://192.168.1.42/`
    ///
    /// The URI `http://notgoogle.com/` would not match.
    pub fn from_string(no_proxy_list: &str) -> Option<Self> {
        Some(Self {
            inner: no_proxy_list.into(),
        })
    }
}

// ===== impl Matcher =====

impl Matcher {
    pub(crate) fn system() -> Self {
        Self {
            inner: Box::new(matcher::Matcher::from_system()),
        }
    }

    /// Intercept the given destination URI, returning the intercepted
    /// proxy configuration if there is a match.
    #[inline]
    pub(crate) fn intercept(&self, dst: &Uri) -> Option<Intercepted> {
        self.inner.intercept(dst)
    }

    /// The SOCKS5 proxy endpoints this rule can select.
    pub(crate) fn socks5_endpoints(&self) -> Vec<&Uri> {
        self.inner.socks5_endpoints()
    }
}

// ===== impl Extra =====

impl Hash for Extra {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.auth.hash(state);
        self.credentials.hash(state);
        if let Some(ref misc) = self.misc {
            for (k, v) in misc {
                k.as_str().hash(state);
                v.as_bytes().hash(state);
            }
        } else {
            1u8.hash(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed environment used to exercise the env-driven rules without
    /// mutating the process environment.
    struct TestEnv {
        values: Vec<(&'static str, &'static str)>,
    }

    impl TestEnv {
        fn new(values: &[(&'static str, &'static str)]) -> Self {
            Self {
                values: values.to_vec(),
            }
        }
    }

    impl EnvSource for TestEnv {
        fn get(&self, names: &[&str]) -> Option<String> {
            for name in names {
                if let Some((_, value)) = self.values.iter().find(|(key, _)| key == name) {
                    return Some((*value).to_owned());
                }
            }
            None
        }
    }

    fn uri(s: &str) -> Uri {
        s.parse().unwrap()
    }

    fn intercept(p: &Matcher, s: &Uri) -> matcher::Intercept {
        match p.intercept(s).unwrap() {
            Intercepted::Proxy(proxy) => proxy,
            #[cfg(unix)]
            Intercepted::Unix(_) => {
                unreachable!("intercepted_port should only be called with a Proxy matcher")
            }
        }
    }

    fn intercepted_uri(p: &Matcher, s: &str) -> Uri {
        match p.intercept(&s.parse().unwrap()).unwrap() {
            Intercepted::Proxy(proxy) => proxy.uri().clone(),
            #[cfg(unix)]
            Intercepted::Unix(_) => {
                unreachable!("intercepted_uri should only be called with a Proxy matcher")
            }
        }
    }

    #[test]
    fn test_http() {
        let target = "http://example.domain/";
        let p = Proxy::http(target).unwrap().into_matcher();

        let http = "http://hyper.rs";
        let other = "https://hyper.rs";

        assert_eq!(intercepted_uri(&p, http), target);
        assert!(p.intercept(&uri(other)).is_none());
    }

    #[test]
    fn test_https() {
        let target = "http://example.domain/";
        let p = Proxy::https(target).unwrap().into_matcher();

        let http = "http://hyper.rs";
        let other = "https://hyper.rs";

        assert!(p.intercept(&uri(http)).is_none());
        assert_eq!(intercepted_uri(&p, other), target);
    }

    #[test]
    fn test_all() {
        let target = "http://example.domain/";
        let p = Proxy::all(target).unwrap().into_matcher();

        let http = "http://hyper.rs";
        let https = "https://hyper.rs";
        // no longer supported
        // let other = "x-youve-never-heard-of-me-mr-proxy://hyper.rs";

        assert_eq!(intercepted_uri(&p, http), target);
        assert_eq!(intercepted_uri(&p, https), target);
        // assert_eq!(intercepted_uri(&p, other), target);
    }

    #[test]
    fn test_standard_with_custom_auth_header() {
        let target = "http://example.domain/";
        let p = Proxy::all(target)
            .unwrap()
            .custom_http_auth(http::HeaderValue::from_static("testme"))
            .into_matcher();

        let got = intercept(&p, &uri("http://anywhere.local"));
        let auth = got.basic_auth().unwrap();
        assert_eq!(auth, "testme");
    }

    #[test]
    fn test_maybe_has_http_auth() {
        let uri = uri("http://example.domain/");

        let m = Proxy::all("https://letme:in@yo.local")
            .unwrap()
            .into_matcher();

        let got = intercept(&m, &uri);
        assert!(got.basic_auth().is_some(), "https forwards");

        let m = Proxy::all("http://letme:in@yo.local")
            .unwrap()
            .into_matcher();

        let got = intercept(&m, &uri);
        assert!(got.basic_auth().is_some(), "http forwards");
    }

    #[test]
    fn test_maybe_has_http_custom_headers() {
        let uri = uri("http://example.domain/");

        let mut headers = HeaderMap::new();
        headers.insert("x-custom-header", HeaderValue::from_static("custom-value"));

        let m = Proxy::all("https://yo.local")
            .unwrap()
            .custom_http_headers(headers.clone())
            .into_matcher();

        match m.intercept(&uri).unwrap() {
            Intercepted::Proxy(proxy) => {
                let got_headers = proxy.custom_headers().unwrap();
                assert_eq!(got_headers, &headers, "https forwards");
            }
            #[cfg(unix)]
            Intercepted::Unix(_) => {
                unreachable!("Expected a Proxy Intercepted");
            }
        }

        let m = Proxy::all("http://yo.local")
            .unwrap()
            .custom_http_headers(headers.clone())
            .into_matcher();

        match m.intercept(&uri).unwrap() {
            Intercepted::Proxy(proxy) => {
                let got_headers = proxy.custom_headers().unwrap();
                assert_eq!(got_headers, &headers, "http forwards");
            }
            #[cfg(unix)]
            Intercepted::Unix(_) => {
                unreachable!("Expected a Proxy Intercepted");
            }
        }
    }

    fn test_socks_proxy_default_port(uri: &str, url2: &str, port: u16) {
        let m = Proxy::all(uri).unwrap().into_matcher();

        let http = "http://hyper.rs";
        let https = "https://hyper.rs";

        assert_eq!(intercepted_uri(&m, http).port_u16(), Some(1080));
        assert_eq!(intercepted_uri(&m, https).port_u16(), Some(1080));

        // custom port
        let m = Proxy::all(url2).unwrap().into_matcher();

        assert_eq!(intercepted_uri(&m, http).port_u16(), Some(port));
        assert_eq!(intercepted_uri(&m, https).port_u16(), Some(port));
    }

    #[test]
    fn test_socks4_proxy_default_port() {
        test_socks_proxy_default_port("socks4://example.com", "socks4://example.com:1234", 1234);
        test_socks_proxy_default_port("socks4a://example.com", "socks4a://example.com:1234", 1234);
    }

    #[test]
    fn test_socks5_proxy_default_port() {
        test_socks_proxy_default_port("socks5://example.com", "socks5://example.com:1234", 1234);
        test_socks_proxy_default_port("socks5h://example.com", "socks5h://example.com:1234", 1234);
    }

    #[test]
    fn test_no_proxy_excludes_matching_url() {
        let target = "http://proxy.example/";
        let no_proxy = NoProxy::from_string("excluded.local, .example.org").unwrap();

        let p = Proxy::all(target)
            .unwrap()
            .no_proxy(Some(no_proxy))
            .into_matcher();

        // Should intercept: non-excluded URL
        assert!(p.intercept(&uri("http://allowed.com")).is_some());

        // Should NOT intercept: excluded URL
        assert!(p.intercept(&uri("http://excluded.local")).is_none());

        // Should NOT intercept: excluded domain (.example.org matches subdomains)
        assert!(p.intercept(&uri("http://sub.example.org")).is_none());
    }

    #[test]
    fn test_basic_auth_sets_proxy_url_userinfo() {
        let m = Proxy::all("http://proxy.local:8080")
            .unwrap()
            .basic_auth("user", "pass")
            .into_matcher();

        let got = intercept(&m, &uri("http://hyper.rs"));
        assert!(got.basic_auth().is_some(), "basic auth should be set");
    }

    #[test]
    fn test_custom_auth_header_value() {
        let auth_val = http::HeaderValue::from_static("Bearer proxy-token-123");

        let m = Proxy::http("http://proxy.local")
            .unwrap()
            .custom_http_auth(auth_val)
            .into_matcher();

        let got = intercept(&m, &uri("http://hyper.rs"));
        let got_auth = got.basic_auth().unwrap();
        assert_eq!(got_auth, "Bearer proxy-token-123");
    }

    #[test]
    fn test_http_scheme_only_intercepts_http() {
        let target = "http://http-proxy.local/";
        let p = Proxy::http(target).unwrap().into_matcher();

        // HTTP URL should be intercepted
        assert!(p.intercept(&uri("http://api.example.com")).is_some());

        // HTTPS URL should NOT be intercepted by HTTP proxy
        assert!(p.intercept(&uri("https://api.example.com")).is_none());
    }

    #[test]
    fn test_https_scheme_only_intercepts_https() {
        let target = "http://https-proxy.local/";
        let p = Proxy::https(target).unwrap().into_matcher();

        // HTTP URL should NOT be intercepted by HTTPS proxy
        assert!(p.intercept(&uri("http://api.example.com")).is_none());

        // HTTPS URL should be intercepted
        assert!(p.intercept(&uri("https://api.example.com")).is_some());
    }

    #[test]
    fn test_all_scheme_intercepts_both_http_and_https() {
        let target = "http://universal-proxy.local/";
        let p = Proxy::all(target).unwrap().into_matcher();

        assert!(p.intercept(&uri("http://api.example.com")).is_some());
        assert!(p.intercept(&uri("https://api.example.com")).is_some());
    }

    #[test]
    fn test_custom_selects_proxy_per_request() {
        let p = Proxy::custom(|uri: &Uri| {
            (uri.host() == Some("proxied.example")).then_some("http://proxy.local:8080")
        })
        .into_matcher();

        assert_eq!(
            intercepted_uri(&p, "http://proxied.example/path"),
            "http://proxy.local:8080/"
        );
        assert_eq!(
            intercepted_uri(&p, "https://proxied.example/path"),
            "http://proxy.local:8080/"
        );

        // Not selected -> no proxy at all.
        assert!(p.intercept(&uri("http://direct.example")).is_none());
    }

    #[test]
    fn test_custom_applies_auth_and_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-proxy-tag", HeaderValue::from_static("custom"));

        let p = Proxy::custom(|_: &Uri| Some("http://user:pass@proxy.local:8080"))
            .basic_auth("Aladdin", "open sesame")
            .custom_http_headers(headers.clone())
            .into_matcher();

        let got = intercept(&p, &uri("http://anywhere.local"));
        assert_eq!(
            got.basic_auth().unwrap(),
            "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
        assert_eq!(got.custom_headers().unwrap(), &headers);
    }

    #[test]
    fn test_custom_honours_no_proxy_exclusions() {
        let p = Proxy::custom(|_: &Uri| Some("http://proxy.local:8080"))
            .no_proxy(Some(NoProxy::from_string("excluded.local").unwrap()))
            .into_matcher();

        assert!(p.intercept(&uri("http://excluded.local")).is_none());
        assert!(p.intercept(&uri("http://allowed.local")).is_some());
    }

    #[test]
    fn test_custom_supports_socks_scheme() {
        let p = Proxy::custom(|_: &Uri| Some("socks5://proxy.local")).into_matcher();

        // SOCKS endpoints default to port 1080.
        assert_eq!(
            intercepted_uri(&p, "https://anywhere.local").port_u16(),
            Some(1080)
        );
    }

    #[test]
    fn test_custom_ignores_unusable_endpoints() {
        let p = Proxy::custom(|_: &Uri| Some("ftp://proxy.local:21")).into_matcher();

        // Unsupported proxy schemes are dropped, so the request goes direct.
        assert!(p.intercept(&uri("http://anywhere.local")).is_none());
    }

    #[test]
    fn test_custom_matchers_are_not_equal() {
        let a = Proxy::custom(|_: &Uri| Some("http://proxy.local")).into_matcher();
        let b = Proxy::custom(|_: &Uri| Some("http://proxy.local")).into_matcher();

        assert_ne!(a, b);

        let c = a.clone();
        assert_eq!(a, c);
    }

    #[test]
    fn test_custom_accepts_authority_only_form() {
        let p = Proxy::custom(|_: &Uri| Some("proxy.local:8080")).into_matcher();

        assert_eq!(
            intercepted_uri(&p, "http://anywhere.local"),
            "http://proxy.local:8080/"
        );
    }

    #[test]
    fn test_from_env_values_all_proxy_wins() {
        let p = Proxy::from_env_values(
            Some("http://http.proxy:8080"),
            Some("http://https.proxy:8443"),
            Some("http://all.proxy:3128"),
            None,
        )
        .unwrap()
        .into_matcher();

        assert_eq!(
            intercepted_uri(&p, "http://anywhere.local"),
            "http://all.proxy:3128/"
        );
        assert_eq!(
            intercepted_uri(&p, "https://anywhere.local"),
            "http://all.proxy:3128/"
        );
    }

    #[test]
    fn test_from_env_values_scheme_specific() {
        let http = Proxy::from_env_values(Some("http://http.proxy:8080"), None, None, None)
            .unwrap()
            .into_matcher();
        assert_eq!(
            intercepted_uri(&http, "http://anywhere.local"),
            "http://http.proxy:8080/"
        );
        assert!(http.intercept(&uri("https://anywhere.local")).is_none());

        let https = Proxy::from_env_values(None, Some("http://https.proxy:8443"), None, None)
            .unwrap()
            .into_matcher();
        assert!(https.intercept(&uri("http://anywhere.local")).is_none());
        assert_eq!(
            intercepted_uri(&https, "https://anywhere.local"),
            "http://https.proxy:8443/"
        );
    }

    #[test]
    fn test_from_env_values_without_any_variable() {
        assert!(Proxy::from_env_values(None, None, None, None).is_none());
    }

    #[test]
    fn test_from_env_values_applies_no_proxy() {
        let p = Proxy::from_env_values(
            None,
            None,
            Some("http://all.proxy:3128"),
            Some("direct.local"),
        )
        .unwrap()
        .into_matcher();

        assert!(p.intercept(&uri("http://direct.local")).is_none());
        assert!(p.intercept(&uri("http://other.local")).is_some());
    }

    #[test]
    fn test_from_env_values_endpoint_without_scheme() {
        let p = Proxy::from_env_values(None, None, Some("all.proxy:3128"), None)
            .unwrap()
            .into_matcher();

        assert_eq!(
            intercepted_uri(&p, "http://anywhere.local"),
            "http://all.proxy:3128/"
        );
    }

    #[test]
    fn test_from_env_all_values_precedence_order() {
        let proxies = Proxy::from_env_all_values(
            Some("http://http.proxy:8080"),
            Some("http://https.proxy:8443"),
            Some("http://all.proxy:3128"),
            Some("direct.local"),
        );
        assert_eq!(proxies.len(), 3);

        // The client-level proxy list is consulted in order, so the first
        // rule that matches a URI wins.
        let matchers: Vec<_> = proxies.into_iter().map(Proxy::into_matcher).collect();

        let mut selected = None;
        for matcher in &matchers {
            if let Some(intercepted) = matcher.intercept(&uri("http://anywhere.local")) {
                selected = Some(intercepted);
                break;
            }
        }
        match selected {
            Some(Intercepted::Proxy(intercept)) => {
                assert_eq!(intercept.uri(), "http://http.proxy:8080/");
            }
            _ => panic!("expected an http proxy interception"),
        }

        let mut selected = None;
        for matcher in &matchers {
            if let Some(intercepted) = matcher.intercept(&uri("https://anywhere.local")) {
                selected = Some(intercepted);
                break;
            }
        }
        match selected {
            Some(Intercepted::Proxy(intercept)) => {
                assert_eq!(intercept.uri(), "http://https.proxy:8443/");
            }
            _ => panic!("expected an https proxy interception"),
        }
    }

    #[test]
    fn test_from_env_all_values_without_any_variable() {
        assert!(Proxy::from_env_all_values(None, None, None, None).is_empty());
    }

    #[test]
    fn test_from_env_source_reads_the_standard_variables() {
        let env = TestEnv::new(&[
            ("HTTP_PROXY", "http://http.proxy:8080"),
            ("HTTPS_PROXY", "http://https.proxy:8443"),
            ("ALL_PROXY", "http://all.proxy:3128"),
            ("NO_PROXY", "direct.local"),
        ]);

        let p = Proxy::from_env_source(&env).unwrap().into_matcher();

        // `ALL_PROXY` takes precedence for the single-rule constructor.
        assert_eq!(
            intercepted_uri(&p, "http://anywhere.local"),
            "http://all.proxy:3128/"
        );
    }

    #[test]
    fn test_from_env_source_prefers_the_lowercase_variable() {
        let env = TestEnv::new(&[("http_proxy", "http://lower.proxy:8080")]);

        let p = Proxy::from_env_source(&env).unwrap().into_matcher();
        assert_eq!(
            intercepted_uri(&p, "http://anywhere.local"),
            "http://lower.proxy:8080/"
        );
    }

    #[test]
    fn test_from_env_source_without_any_variable() {
        let env = TestEnv::new(&[]);
        assert!(Proxy::from_env_source(&env).is_none());
        assert!(Proxy::from_env_all_source(&env).is_empty());
    }

    #[test]
    fn test_from_env_all_source_collects_every_rule() {
        let env = TestEnv::new(&[
            ("HTTP_PROXY", "http://http.proxy:8080"),
            ("HTTPS_PROXY", "http://https.proxy:8443"),
            ("ALL_PROXY", "http://all.proxy:3128"),
            ("NO_PROXY", "direct.local"),
        ]);

        let proxies = Proxy::from_env_all_source(&env);
        assert_eq!(proxies.len(), 3);

        let matchers: Vec<_> = proxies.into_iter().map(Proxy::into_matcher).collect();

        // NO_PROXY is applied to every rule.
        for matcher in &matchers {
            assert!(matcher.intercept(&uri("http://direct.local")).is_none());
        }
    }

    #[test]
    fn test_no_proxy_from_env_source() {
        let env = TestEnv::new(&[("NO_PROXY", "excluded.local")]);
        let no_proxy = NoProxy::from_env_source(&env).unwrap();

        let p = Proxy::all("http://proxy.local")
            .unwrap()
            .no_proxy(Some(no_proxy))
            .into_matcher();

        assert!(p.intercept(&uri("http://excluded.local")).is_none());
        assert!(p.intercept(&uri("http://allowed.local")).is_some());
    }

    #[test]
    fn test_no_proxy_from_env_source_without_a_variable() {
        let env = TestEnv::new(&[]);
        assert!(NoProxy::from_env_source(&env).is_none());
    }

    #[test]
    fn test_into_proxy_accepts_uris_and_strings() {
        // A closure may return any `IntoProxy` value.
        let p = Proxy::custom(|_: &Uri| Some(uri("http://proxy.local:8080"))).into_matcher();
        assert_eq!(
            intercepted_uri(&p, "http://anywhere.local"),
            "http://proxy.local:8080/"
        );

        let p = Proxy::custom(|_: &Uri| Some("http://proxy.local:8080".to_owned())).into_matcher();
        assert_eq!(
            intercepted_uri(&p, "http://anywhere.local"),
            "http://proxy.local:8080/"
        );

        // The remaining `IntoProxy` implementations.
        let owned = "http://proxy.local:8080".to_owned();
        assert_eq!(
            IntoProxySealed::into_proxy(&owned),
            Some(uri("http://proxy.local:8080"))
        );
        assert_eq!(
            IntoProxySealed::into_proxy(uri("http://proxy.local:8080")),
            Some(uri("http://proxy.local:8080"))
        );
        assert_eq!(
            IntoProxySealed::into_proxy(&uri("http://proxy.local:8080")),
            Some(uri("http://proxy.local:8080"))
        );
    }

    #[test]
    fn test_into_proxy_rejects_empty_and_invalid_values() {
        assert!(IntoProxySealed::into_proxy("").is_none());
        assert!(IntoProxySealed::into_proxy("   ").is_none());
        assert!(IntoProxySealed::into_proxy("http://").is_none());
    }

    #[test]
    fn test_custom_proxy_debug_is_stable() {
        let p = Proxy::custom(|_: &Uri| Some("http://proxy.local"));
        let rendered = format!("{p:?}");
        assert!(
            rendered.contains("CustomProxy"),
            "unexpected debug output: {rendered}"
        );
    }

    /// Two different custom proxies must not share a connection-pool entry,
    /// so they must hash differently.
    #[test]
    fn test_custom_proxies_hash_differently() {
        use std::hash::{Hash, Hasher};

        fn hash_of(matcher: &Matcher) -> u64 {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            matcher.hash(&mut hasher);
            hasher.finish()
        }

        let a = Proxy::custom(|_: &Uri| Some("http://proxy-a.local"));
        let b = Proxy::custom(|_: &Uri| Some("http://proxy-b.local"));

        let matcher_a = a.clone().into_matcher();
        let matcher_b = b.into_matcher();

        assert_ne!(hash_of(&matcher_a), hash_of(&matcher_b));

        // A clone of the same proxy hashes identically.
        assert_eq!(hash_of(&matcher_a), hash_of(&a.into_matcher()));
    }

    /// Proxies that carry different credentials or headers must not share a
    /// connection-pool entry.
    #[test]
    fn test_proxies_with_different_credentials_hash_differently() {
        use std::hash::{Hash, Hasher};

        fn hash_of(matcher: &Matcher) -> u64 {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            matcher.hash(&mut hasher);
            hasher.finish()
        }

        let plain = Proxy::all("http://proxy.local:8080").unwrap();
        let with_auth = plain.clone().basic_auth("Aladdin", "open sesame");
        let with_headers = plain.clone().custom_http_headers({
            let mut headers = HeaderMap::new();
            headers.insert("x-proxy-tag", HeaderValue::from_static("tag"));
            headers
        });

        let plain = plain.into_matcher();
        let with_auth = with_auth.into_matcher();
        let with_headers = with_headers.into_matcher();

        assert_ne!(hash_of(&plain), hash_of(&with_auth));
        assert_ne!(hash_of(&plain), hash_of(&with_headers));
        assert_ne!(hash_of(&with_auth), hash_of(&with_headers));
    }

    /// An endpoint without a scheme is treated as a plain HTTP proxy.
    #[test]
    fn test_into_proxy_defaults_to_http_for_authority_only_values() {
        assert_eq!(
            IntoProxySealed::into_proxy("proxy.local:8080"),
            Some(uri("http://proxy.local:8080/"))
        );
        assert_eq!(
            IntoProxySealed::into_proxy("socks5://proxy.local"),
            Some(uri("socks5://proxy.local/"))
        );
    }

    /// The public constructors read the real process environment.
    ///
    /// The environment is mutated for the duration of the test and restored
    /// afterwards. The lock keeps the test from running concurrently with
    /// itself; no other test in this binary depends on the proxy environment
    /// variables.
    #[test]
    #[expect(
        unsafe_code,
        reason = "the test mutates the process environment to exercise the env-driven rules"
    )]
    fn test_from_env_reads_the_process_environment() {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

        const NAMES: [&str; 8] = [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
            "NO_PROXY",
            "no_proxy",
        ];

        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());

        let saved: Vec<(String, Option<String>)> = NAMES
            .iter()
            .map(|name| ((*name).to_owned(), std::env::var(name).ok()))
            .collect();

        for name in NAMES {
            // SAFETY: the lock serializes the tests in this binary that
            // touch the proxy environment variables, and the values are
            // restored before the test returns.
            unsafe { std::env::remove_var(name) };
        }

        // SAFETY: see above.
        unsafe { std::env::set_var("HTTP_PROXY", "http://env.proxy:8080") };
        // SAFETY: see above.
        unsafe { std::env::set_var("NO_PROXY", "direct.local") };

        let proxy = Proxy::from_env().expect("HTTP_PROXY should produce a proxy rule");
        let matcher = proxy.into_matcher();
        assert_eq!(
            intercepted_uri(&matcher, "http://anywhere.local"),
            "http://env.proxy:8080/"
        );
        assert!(matcher.intercept(&uri("http://direct.local")).is_none());

        let all = Proxy::from_env_all();
        assert_eq!(all.len(), 1);

        let no_proxy = NoProxy::from_env().expect("NO_PROXY should produce a no-proxy list");
        assert!(no_proxy.inner.contains("direct.local"));

        for (name, value) in saved {
            match value {
                // SAFETY: see above.
                Some(value) => unsafe { std::env::set_var(&name, value) },
                // SAFETY: see above.
                None => unsafe { std::env::remove_var(&name) },
            }
        }
    }
}
