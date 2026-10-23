//! Proxy matchers
//!
//! This module contains different matchers to configure rules for when a proxy
//! should be used, and if so, with what arguments.
//!
//! A [`Matcher`] can be constructed either using environment variables, or
//! a [`Matcher::builder()`].
//!
//! Once constructed, the `Matcher` can be asked if it intercepts a `Uri` by
//! calling [`Matcher::intercept()`].
//!
//! An [`Intercept`] includes the destination for the proxy, and any parsed
//! authentication to be used.

use std::net::IpAddr;
#[cfg(unix)]
use std::{path::Path, sync::Arc};

use bytes::Bytes;
use http::{
    HeaderMap, Uri,
    header::HeaderValue,
    uri::{Authority, Scheme},
};
use ipnet::IpNet;
use percent_encoding::percent_decode_str;

use self::builder::IntoValue;
use super::{CustomProxy, Extra, Intercepted};
use crate::ext::UriExt;

/// A proxy matcher, usually built from environment variables.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Matcher {
    http: Option<Intercept>,
    https: Option<Intercept>,
    no: NoProxy,
    #[cfg(unix)]
    unix: Option<Arc<Path>>,
    custom: Option<CustomProxy>,
    /// A Proxy Auto-Config rule, evaluated for every request.
    #[cfg(feature = "pac")]
    pac: Option<crate::proxy::pac::PacRule>,
    /// Extra headers/credentials applied to proxies selected by a custom rule.
    extra: Extra,
}

/// A matched proxy,
///
/// This is returned by a matcher if a proxy should be used.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Intercept {
    uri: Uri,
    auth: Auth,
    extra: Extra,
}

/// A builder to create a [`Matcher`].
///
/// Construct with [`Matcher::builder()`].
#[derive(Default)]
pub(crate) struct Builder {
    pub(super) is_cgi: bool,
    pub(super) all: String,
    pub(super) http: String,
    pub(super) https: String,
    pub(super) no: String,
    #[cfg(unix)]
    pub(super) unix: Option<Arc<Path>>,
    pub(super) custom: Option<CustomProxy>,
    /// A Proxy Auto-Config rule, evaluated for every request.
    #[cfg(feature = "pac")]
    pub(super) pac: Option<crate::proxy::pac::PacRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Auth {
    Empty,
    Basic(HeaderValue),
    Raw(Bytes, Bytes),
}

/// A filter for proxy matchers.
///
/// This type is based off the `NO_PROXY` rules used by curl.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct NoProxy {
    ips: IpMatcher,
    domains: DomainMatcher,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct DomainMatcher(Vec<String>);

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct IpMatcher(Vec<Ip>);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Ip {
    Address(IpAddr),
    Network(IpNet),
}

// ===== impl Matcher =====

impl Matcher {
    /// Create a matcher from the environment or system.
    ///
    /// This checks the same environment variables as `from_env()`, and if not
    /// set, checks the system configuration for values for the OS.
    ///
    /// This constructor is always available, but if the `client-proxy-system`
    /// feature is enabled, it will check more configuration. Use this
    /// constructor if you want to allow users to optionally enable more, or
    /// use `from_env` if you do not want the values to change based on an
    /// enabled feature.
    pub(crate) fn from_system() -> Self {
        Builder::from_system().build(Extra::default())
    }

    /// Start a builder to configure a matcher.
    pub(crate) fn builder() -> Builder {
        Builder::default()
    }

    #[cfg(all(feature = "http3", feature = "socks"))]
    /// The SOCKS5 proxy endpoints this rule can select.
    ///
    /// A SOCKS5 proxy relays UDP, which is what QUIC needs, so the HTTP/3
    /// path uses the endpoints this method reports.
    pub(crate) fn socks5_endpoints(&self) -> Vec<&Uri> {
        let mut endpoints = Vec::new();
        for intercept in [&self.http, &self.https].into_iter().flatten() {
            if is_socks5(intercept.uri()) {
                endpoints.push(intercept.uri());
            }
        }
        endpoints
    }

    /// Check if the destination should be intercepted by a proxy.
    ///
    /// If the proxy rules match the destination, a new `Uri` will be returned
    /// to connect to.
    pub(crate) fn intercept(&self, dst: &Uri) -> Option<Intercepted> {
        // if unix sockets are configured, check them first
        #[cfg(unix)]
        if let Some(unix) = &self.unix {
            return Some(Intercepted::Unix(unix.clone()));
        }

        // TODO(perf): don't need to check `no` if below doesn't match...
        if self.no.contains(dst.host()?) {
            return None;
        }

        // A Proxy Auto-Config rule decides on its own which proxy to use, so
        // it is evaluated before the scheme-specific rules.
        #[cfg(feature = "pac")]
        if let Some(pac) = &self.pac {
            let selected = pac.select(dst)?;
            return intercept_from_uri(&selected, &self.extra).map(Intercepted::Proxy);
        }

        // A custom rule decides on its own which scheme it applies to, so it
        // is evaluated before the scheme-specific rules.
        if let Some(custom) = &self.custom {
            let selected = custom.select(dst)?;
            return intercept_from_uri(&selected, &self.extra).map(Intercepted::Proxy);
        }

        if dst.is_http() {
            return self.http.clone().map(Intercepted::Proxy);
        }

        if dst.is_https() {
            return self.https.clone().map(Intercepted::Proxy);
        }

        None
    }
}

// ===== impl Intercept =====

#[cfg(all(feature = "http3", feature = "socks"))]
/// Returns true when the proxy URI selects a SOCKS5 proxy.
pub(crate) fn is_socks5(uri: &Uri) -> bool {
    matches!(uri.scheme_str(), Some("socks5" | "socks5h"))
}

impl Intercept {
    #[inline]
    pub(crate) const fn uri(&self) -> &Uri {
        &self.uri
    }

    pub(crate) const fn basic_auth(&self) -> Option<&HeaderValue> {
        if let Some(ref val) = self.extra.auth {
            return Some(val);
        }

        // Explicit credentials for a challenge-based scheme replace the
        // credentials that were embedded in the proxy URI: the scheme
        // decides how they are used, so the URI credentials must not be
        // turned into a preemptive `Basic` header.
        if self.extra.credentials.is_some() {
            return None;
        }

        if let Auth::Basic(ref val) = self.auth {
            Some(val)
        } else {
            None
        }
    }

    #[inline]
    pub(crate) const fn custom_headers(&self) -> Option<&HeaderMap> {
        self.extra.misc.as_ref()
    }

    /// The credentials used to answer a proxy authentication challenge.
    pub(crate) fn credentials(&self) -> Option<&crate::proxy::auth::ProxyCredentials> {
        self.extra.credentials.as_ref()
    }

    #[cfg(feature = "socks")]
    pub(crate) fn raw_auth(&self) -> Option<(Bytes, Bytes)> {
        if let Auth::Raw(ref u, ref p) = self.auth {
            Some((u.clone(), p.clone()))
        } else {
            None
        }
    }
}

// ===== impl Builder =====

impl Builder {
    fn from_env() -> Self {
        Self::from_env_source(&super::ProcessEnv)
    }

    /// Build the environment-driven rules from an environment source.
    pub(super) fn from_env_source(env: &dyn super::EnvSource) -> Self {
        Self {
            is_cgi: std::env::var_os("REQUEST_METHOD").is_some(),
            all: env.get(&["ALL_PROXY", "all_proxy"]).unwrap_or_default(),
            http: env.get(&["HTTP_PROXY", "http_proxy"]).unwrap_or_default(),
            https: env.get(&["HTTPS_PROXY", "https_proxy"]).unwrap_or_default(),
            no: env.get(&["NO_PROXY", "no_proxy"]).unwrap_or_default(),
            #[cfg(unix)]
            unix: None,
            custom: None,
            #[cfg(feature = "pac")]
            pac: None,
        }
    }

    fn from_system() -> Self {
        #[cfg(any(
            all(target_os = "macos", feature = "system-proxy"),
            all(windows, feature = "system-proxy")
        ))]
        {
            let mut builder = Self::from_env();

            #[cfg(all(target_os = "macos", feature = "system-proxy"))]
            super::mac::with_system(&mut builder);

            #[cfg(all(windows, feature = "system-proxy"))]
            super::win::with_system(&mut builder);

            builder
        }

        #[cfg(not(any(
            all(target_os = "macos", feature = "system-proxy"),
            all(windows, feature = "system-proxy")
        )))]
        {
            Self::from_env()
        }
    }

    /// Set the target proxy for all destinations.
    pub(crate) fn all<S>(mut self, val: S) -> Self
    where
        S: IntoValue,
    {
        self.all = val.into_value();
        self
    }

    /// Set the target proxy for HTTP destinations.
    pub(crate) fn http<S>(mut self, val: S) -> Self
    where
        S: IntoValue,
    {
        self.http = val.into_value();
        self
    }

    /// Set the target proxy for HTTPS destinations.
    pub(crate) fn https<S>(mut self, val: S) -> Self
    where
        S: IntoValue,
    {
        self.https = val.into_value();
        self
    }

    /// Set the "no" proxy filter.
    ///
    /// The rules are as follows:
    /// * Entries are expected to be comma-separated (whitespace between entries is ignored)
    /// * IP addresses (both IPv4 and IPv6) are allowed, as are optional subnet masks (by adding
    ///   /size, for example "`192.168.1.0/24`").
    /// * An entry "`*`" matches all hostnames (this is the only wildcard allowed)
    /// * Any other entry is considered a domain name (and may contain a leading dot, for example
    ///   `google.com` and `.google.com` are equivalent) and would match both that domain AND all
    ///   subdomains.
    ///
    /// For example, if `"NO_PROXY=google.com, 192.168.1.0/24"` was set, all of the following would
    /// match (and therefore would bypass the proxy):
    /// * `http://google.com/`
    /// * `http://www.google.com/`
    /// * `http://192.168.1.42/`
    ///
    /// The URI `http://notgoogle.com/` would not match.
    pub(crate) fn no<S>(mut self, val: S) -> Self
    where
        S: IntoValue,
    {
        self.no = val.into_value();
        self
    }

    // / Set the unix socket target proxy for all destinations.
    #[cfg(unix)]
    pub(crate) fn unix<S>(mut self, val: S) -> Self
    where
        S: super::uds::IntoUnixSocket,
    {
        self.unix = Some(val.unix_socket());
        self
    }

    /// Set a custom proxy rule.
    ///
    /// The rule takes precedence over the scheme-specific `all`/`http`/`https`
    /// rules: it decides on its own which requests are proxied.
    pub(crate) fn custom(mut self, val: CustomProxy) -> Self {
        self.custom = Some(val);
        self
    }

    /// Set a Proxy Auto-Config rule.
    #[cfg(feature = "pac")]
    pub(crate) fn pac(mut self, rule: crate::proxy::pac::PacRule) -> Self {
        self.pac = Some(rule);
        self
    }

    /// Construct a [`Matcher`] using the configured values.
    pub(super) fn build(self, extra: Extra) -> Matcher {
        if self.is_cgi {
            return Matcher {
                http: None,
                https: None,
                no: NoProxy::empty(),
                #[cfg(unix)]
                unix: None,
                custom: None,
                #[cfg(feature = "pac")]
                pac: None,
                extra: Extra::default(),
            };
        }

        let mut all = parse_env_uri(&self.all);
        let mut http = parse_env_uri(&self.http);
        let mut https = parse_env_uri(&self.https);

        if let Some(http) = http.as_mut() {
            http.extra = extra.clone();
        }
        if let Some(https) = https.as_mut() {
            https.extra = extra.clone();
        }
        if (http.is_none() || https.is_none())
            && let Some(all) = all.as_mut()
        {
            all.extra = extra.clone();
        }

        Matcher {
            http: http.or_else(|| all.clone()),
            https: https.or(all),
            no: NoProxy::from_string(&self.no),
            #[cfg(unix)]
            unix: self.unix,
            custom: self.custom,
            #[cfg(feature = "pac")]
            pac: self.pac,
            extra,
        }
    }
}

fn parse_env_uri(val: &str) -> Option<Intercept> {
    let uri = val.parse::<Uri>().ok()?;
    intercept_from_uri(&uri, &Extra::default())
}

/// Turn a proxy endpoint URI into an [`Intercept`].
///
/// The URI may carry `userinfo` (for example `http://user:pass@proxy:8080`);
/// it is split off and turned into the appropriate authentication scheme for
/// the proxy kind. SOCKS endpoints without an explicit port default to
/// `1080`.
fn intercept_from_uri(uri: &Uri, extra: &Extra) -> Option<Intercept> {
    let mut builder = Uri::builder();
    let mut is_httpish = false;
    let mut is_socks = false;
    let mut auth = Auth::Empty;

    builder = builder.scheme(if let Some(s) = uri.scheme() {
        if s == &Scheme::HTTP || s == &Scheme::HTTPS {
            is_httpish = true;
            s.clone()
        } else if matches!(s.as_str(), "socks4" | "socks4a" | "socks5" | "socks5h") {
            is_socks = true;
            s.clone()
        } else {
            // can't use this proxy scheme
            return None;
        }
    } else {
        // if no scheme provided, assume they meant 'http'
        is_httpish = true;
        Scheme::HTTP
    });

    let authority = {
        let authority = uri.authority()?;
        // default SOCKS port to 1080 if missing
        if is_socks && authority.port().is_none() {
            Authority::from_maybe_shared(Bytes::from(format!("{authority}:1080"))).ok()?
        } else {
            authority.clone()
        }
    };

    if let Some((userinfo, host_port)) = authority.as_str().rsplit_once('@') {
        let (user, pass) = match userinfo.split_once(':') {
            Some((user, pass)) => (user, Some(pass)),
            None => (userinfo, None),
        };

        let user = percent_decode_str(user).decode_utf8_lossy();
        let pass = pass.map(|pass| percent_decode_str(pass).decode_utf8_lossy());
        if is_httpish {
            auth = Auth::Basic(crate::util::basic_auth(&user, pass.as_deref()));
        } else {
            auth = Auth::Raw(
                Bytes::from(user.into_owned()),
                Bytes::from(pass.map_or_else(String::new, std::borrow::Cow::into_owned)),
            );
        }
        builder = builder.authority(host_port);
    } else {
        builder = builder.authority(authority);
    }

    // removing any path, but we MUST specify one or the builder errors
    builder = builder.path_and_query("/");

    Some(Intercept {
        auth,
        extra: extra.clone(),
        uri: builder.build().ok()?,
    })
}

impl NoProxy {
    const fn empty() -> Self {
        Self {
            ips: IpMatcher(Vec::new()),
            domains: DomainMatcher(Vec::new()),
        }
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
    /// For example, if `"NO_PROXY=google.com, 192.168.1.0/24"` was set, all of the following would
    /// match (and therefore would bypass the proxy):
    /// * `http://google.com/`
    /// * `http://www.google.com/`
    /// * `http://192.168.1.42/`
    ///
    /// The URI `http://notgoogle.com/` would not match.
    pub(crate) fn from_string(no_proxy_list: &str) -> Self {
        let mut ips = Vec::new();
        let mut domains = Vec::new();
        let parts = no_proxy_list.split(',').map(str::trim);
        for part in parts {
            match part.parse::<IpNet>() {
                // If we can parse an IP net or address, then use it, otherwise, assume it is a
                // domain
                Ok(ip) => ips.push(Ip::Network(ip)),
                Err(_) => match part.parse::<IpAddr>() {
                    Ok(addr) => ips.push(Ip::Address(addr)),
                    Err(_) => {
                        if !part.trim().is_empty() {
                            domains.push(part.to_owned());
                        }
                    }
                },
            }
        }
        Self {
            ips: IpMatcher(ips),
            domains: DomainMatcher(domains),
        }
    }

    /// Return true if this matches the host (domain or IP).
    pub(crate) fn contains(&self, host: &str) -> bool {
        // According to RFC3986, raw IPv6 hosts will be wrapped in []. So we need to strip those off
        // the end in order to parse correctly
        let host = if host.starts_with('[') {
            let x: &[_] = &['[', ']'];
            host.trim_matches(x)
        } else {
            host
        };
        match host.parse::<IpAddr>() {
            // If we can parse an IP addr, then use it, otherwise, assume it is a domain
            Ok(ip) => self.ips.contains(ip),
            Err(_) => self.domains.contains(host),
        }
    }
}

impl IpMatcher {
    fn contains(&self, addr: IpAddr) -> bool {
        for ip in &self.0 {
            match ip {
                Ip::Address(address) => {
                    if &addr == address {
                        return true;
                    }
                }
                Ip::Network(net) => {
                    if net.contains(&addr) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

impl DomainMatcher {
    // The following links may be useful to understand the origin of these rules:
    // * https://curl.se/libcurl/c/CURLOPT_NOPROXY.html
    // * https://github.com/curl/curl/issues/1208
    fn contains(&self, domain: &str) -> bool {
        let domain_len = domain.len();
        for d in &self.0 {
            // The `<local>` token (used by the Windows proxy override list)
            // matches every hostname that does not contain a dot.
            if d.eq_ignore_ascii_case("<local>") {
                if !domain.contains('.') {
                    return true;
                }
                continue;
            }

            if d.eq_ignore_ascii_case(domain)
                || d.strip_prefix('.')
                    .is_some_and(|s| s.eq_ignore_ascii_case(domain))
            {
                return true;
            } else if domain
                .get(domain_len.saturating_sub(d.len())..)
                .is_some_and(|s| s.eq_ignore_ascii_case(d))
            {
                if d.starts_with('.') {
                    // If the first character of d is a dot, that means the first character of
                    // domain must also be a dot, so we are looking at a
                    // subdomain of d and that matches
                    return true;
                } else if domain
                    .as_bytes()
                    .get(domain_len.saturating_sub(d.len() + 1))
                    == Some(&b'.')
                {
                    // Given that d is a prefix of domain, if the prior character in domain is a dot
                    // then that means we must be matching a subdomain of d, and that matches
                    return true;
                }
            } else if d == "*" {
                return true;
            }
        }
        false
    }
}

mod builder {
    /// A type that can used as a `Builder` value.
    ///
    /// Private and sealed, only visible in docs.
    pub(crate) trait IntoValue {
        #[doc(hidden)]
        fn into_value(self) -> String;
    }

    impl IntoValue for String {
        #[doc(hidden)]
        fn into_value(self) -> String {
            self
        }
    }

    impl IntoValue for &String {
        #[doc(hidden)]
        fn into_value(self) -> String {
            self.into()
        }
    }

    impl IntoValue for &str {
        #[doc(hidden)]
        fn into_value(self) -> String {
            self.into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_domain_matcher() {
        let domains = vec![".foo.bar".into(), "bar.foo".into()];
        let matcher = DomainMatcher(domains);

        // domains match with leading `.`
        assert!(matcher.contains("foo.bar"));
        assert!(matcher.contains("FOO.BAR"));
        // subdomains match with leading `.`
        assert!(matcher.contains("www.foo.bar"));
        assert!(matcher.contains("WWW.FOO.BAR"));

        // domains match with no leading `.`
        assert!(matcher.contains("bar.foo"));
        assert!(matcher.contains("Bar.foo"));
        // subdomains match with no leading `.`
        assert!(matcher.contains("www.bar.foo"));
        assert!(matcher.contains("WWW.BAR.FOO"));

        // non-subdomain string prefixes don't match
        assert!(!matcher.contains("notfoo.bar"));
        assert!(!matcher.contains("notbar.foo"));
    }

    #[test]
    fn test_no_proxy_wildcard() {
        let no_proxy = NoProxy::from_string("*");
        assert!(no_proxy.contains("any.where"));
    }

    #[test]
    fn test_no_proxy_ip_ranges() {
        let no_proxy =
            NoProxy::from_string(".foo.bar, bar.baz,10.42.1.1/24,::1,10.124.7.8,2001::/17");

        let should_not_match = [
            // random uri, not in no_proxy
            "hyper.rs",
            // make sure that random non-subdomain string prefixes don't match
            "notfoo.bar",
            // make sure that random non-subdomain string prefixes don't match
            "notbar.baz",
            // ipv4 address out of range
            "10.43.1.1",
            // ipv4 address out of range
            "10.124.7.7",
            // ipv6 address out of range
            "[ffff:db8:a0b:12f0::1]",
            // ipv6 address out of range
            "[2005:db8:a0b:12f0::1]",
        ];

        for host in &should_not_match {
            assert!(!no_proxy.contains(host), "should not contain {host:?}");
        }

        let should_match = [
            // make sure subdomains (with leading .) match
            "hello.foo.bar",
            // make sure exact matches (without leading .) match (also makes sure spaces between
            // entries work)
            "bar.baz",
            // make sure subdomains (without leading . in no_proxy) match
            "foo.bar.baz",
            // make sure subdomains (without leading . in no_proxy) match - this differs from cURL
            "foo.bar",
            // ipv4 address match within range
            "10.42.1.100",
            // ipv6 address exact match
            "[::1]",
            // ipv6 address match within range
            "[2001:db8:a0b:12f0::1]",
            // ipv4 address exact match
            "10.124.7.8",
        ];

        for host in &should_match {
            assert!(no_proxy.contains(host), "should contain {host:?}");
        }
    }

    macro_rules! p {
        ($($n:ident = $v:expr,)*) => ({Builder {
            $($n: $v.into(),)*
            ..Builder::default()
        }.build(Extra::default())});
    }

    fn intercept(p: &Matcher, u: &str) -> Intercept {
        match p.intercept(&u.parse().unwrap()).unwrap() {
            Intercepted::Proxy(intercept) => intercept,
            Intercepted::Unix(path) => {
                unreachable!("should not intercept unix socket: {path:?}")
            }
        }
    }

    #[test]
    fn test_all_proxy() {
        let p = p! {
            all = "http://om.nom",
        };

        assert_eq!("http://om.nom", intercept(&p, "http://example.com").uri());

        assert_eq!("http://om.nom", intercept(&p, "https://example.com").uri());
    }

    #[test]
    fn test_specific_overrides_all() {
        let p = p! {
            all = "http://no.pe",
            http = "http://y.ep",
        };

        assert_eq!("http://no.pe", intercept(&p, "https://example.com").uri());

        // the http rule is "more specific" than the all rule
        assert_eq!("http://y.ep", intercept(&p, "http://example.com").uri());
    }

    #[test]
    fn test_parse_no_scheme_defaults_to_http() {
        let p = p! {
            https = "y.ep",
            http = "127.0.0.1:8887",
        };

        assert_eq!(intercept(&p, "https://example.local").uri(), "http://y.ep");
        assert_eq!(
            intercept(&p, "http://example.local").uri(),
            "http://127.0.0.1:8887"
        );
    }

    #[test]
    fn test_parse_http_auth() {
        let p = p! {
            all = "http://Aladdin:opensesame@y.ep",
        };

        let proxy = intercept(&p, "https://example.local");
        assert_eq!(proxy.uri(), "http://y.ep");
        assert_eq!(
            proxy.basic_auth().expect("basic_auth"),
            "Basic QWxhZGRpbjpvcGVuc2VzYW1l"
        );
    }

    #[test]
    fn test_parse_http_auth_without_password() {
        let p = p! {
            all = "http://Aladdin@y.ep",
        };
        let proxy = intercept(&p, "https://example.local");
        assert_eq!(proxy.uri(), "http://y.ep");
        assert_eq!(
            proxy.basic_auth().expect("basic_auth"),
            "Basic QWxhZGRpbjo="
        );
    }

    #[test]
    fn test_parse_http_auth_without_scheme() {
        let p = p! {
            all = "Aladdin:opensesame@y.ep",
        };

        let proxy = intercept(&p, "https://example.local");
        assert_eq!(proxy.uri(), "http://y.ep");
        assert_eq!(
            proxy.basic_auth().expect("basic_auth"),
            "Basic QWxhZGRpbjpvcGVuc2VzYW1l"
        );
    }

    #[test]
    fn test_dont_parse_http_when_is_cgi() {
        let mut builder = Matcher::builder();
        builder.is_cgi = true;
        builder.http = "http://never.gonna.let.you.go".into();
        let m = builder.build(Extra::default());

        assert!(m.intercept(&"http://rick.roll".parse().unwrap()).is_none());
    }

    fn test_parse_socks(uri: &str) {
        let p = p! {
            all = uri,
        };

        let proxy = intercept(&p, "https://example.local");
        assert_eq!(proxy.uri(), uri);
    }

    #[test]
    fn test_parse_socks4() {
        test_parse_socks("socks4://localhost:8887");
        test_parse_socks("socks4a://localhost:8887");
    }

    #[test]
    fn test_parse_socks5() {
        test_parse_socks("socks5://localhost:8887");
        test_parse_socks("socks5h://localhost:8887");
    }

    #[test]
    fn test_domain_matcher_case_insensitive() {
        let domains = vec![".foo.bar".into()];
        let matcher = DomainMatcher(domains);

        assert!(matcher.contains("foo.bar"));
        assert!(matcher.contains("FOO.BAR"));
        assert!(matcher.contains("Foo.Bar"));

        assert!(matcher.contains("www.foo.bar"));
        assert!(matcher.contains("WWW.FOO.BAR"));
        assert!(matcher.contains("Www.Foo.Bar"));
    }

    #[test]
    fn test_no_proxy_case_insensitive() {
        let p = p! {
            all = "http://proxy.local",
            no = ".example.com",
        };

        // should bypass proxy (case insensitive match)
        assert!(
            p.intercept(&"http://example.com".parse().unwrap())
                .is_none()
        );
        assert!(
            p.intercept(&"http://EXAMPLE.COM".parse().unwrap())
                .is_none()
        );
        assert!(
            p.intercept(&"http://Example.com".parse().unwrap())
                .is_none()
        );

        // subdomain should bypass proxy (case insensitive match)
        assert!(
            p.intercept(&"http://www.example.com".parse().unwrap())
                .is_none()
        );
        assert!(
            p.intercept(&"http://WWW.EXAMPLE.COM".parse().unwrap())
                .is_none()
        );
        assert!(
            p.intercept(&"http://Www.Example.Com".parse().unwrap())
                .is_none()
        );
    }

    #[test]
    fn test_no_proxy_local_token_matches_dotless_hostnames() {
        let no_proxy = NoProxy::from_string("<local>");

        assert!(no_proxy.contains("localhost"));
        assert!(no_proxy.contains("myserver"));
        assert!(no_proxy.contains("LOCALHOST"));

        // Hostnames with a dot are not "local".
        assert!(!no_proxy.contains("example.com"));
        assert!(!no_proxy.contains("www.example.com"));
    }

    #[test]
    fn test_no_proxy_local_token_in_matcher() {
        let p = p! {
            all = "http://proxy.local",
            no = "<local>",
        };

        assert!(
            p.intercept(&"http://localhost".parse().unwrap()).is_none(),
            "dotless hostname should bypass the proxy"
        );
        assert!(
            p.intercept(&"http://example.com".parse().unwrap())
                .is_some(),
            "hostname with a dot should still be proxied"
        );
    }

    /// The credentials and custom headers configured on a proxy must reach
    /// the rule that is actually used, including the catch-all rule.
    #[test]
    fn test_extra_is_applied_to_the_catch_all_rule() {
        let mut headers = HeaderMap::new();
        headers.insert("x-proxy-tag", HeaderValue::from_static("catch-all"));

        let extra = Extra {
            auth: Some(HeaderValue::from_static("Basic dXNlcjpwYXNz")),
            credentials: None,
            misc: Some(headers.clone()),
        };

        let mut builder = Matcher::builder();
        builder.all = "http://all.proxy:3128".to_owned();
        let m = builder.build(extra);

        let got = intercept(&m, "https://example.local");
        assert_eq!(got.basic_auth().unwrap(), "Basic dXNlcjpwYXNz");
        assert_eq!(got.custom_headers().unwrap(), &headers);
    }

    /// A SOCKS proxy carries the credentials as raw user/password bytes
    /// rather than a `Proxy-Authorization` header.
    #[cfg(feature = "socks")]
    #[test]
    fn test_socks_proxy_keeps_raw_credentials() {
        let m = p! {
            all = "socks5://Aladdin:opensesame@proxy.local:1080",
        };

        let proxy = intercept(&m, "https://example.local");
        let (user, pass) = proxy.raw_auth().expect("socks credentials");
        assert_eq!(user.as_ref(), b"Aladdin");
        assert_eq!(pass.as_ref(), b"opensesame");
        // A SOCKS proxy does not use an HTTP authorization header.
        assert!(proxy.basic_auth().is_none());
    }

    /// An HTTP proxy turns the userinfo into a `Proxy-Authorization` header.
    #[cfg(feature = "socks")]
    #[test]
    fn test_http_proxy_uses_basic_auth() {
        let m = p! {
            all = "http://Aladdin:opensesame@proxy.local:8080",
        };

        let proxy = intercept(&m, "https://example.local");
        assert!(proxy.basic_auth().is_some());
        assert!(proxy.raw_auth().is_none());
    }

    /// The catch-all rule is the fallback for the schemes that have no
    /// dedicated rule, so it must carry the credentials even when only one
    /// of the two schemes has its own rule.
    #[test]
    fn test_extra_reaches_the_catch_all_rule_when_one_scheme_has_its_own() {
        let extra = Extra {
            auth: Some(HeaderValue::from_static("Basic dXNlcjpwYXNz")),
            credentials: None,
            misc: None,
        };

        let mut builder = Matcher::builder();
        builder.http = "http://http.proxy:8080".to_owned();
        builder.all = "http://all.proxy:3128".to_owned();
        let m = builder.build(extra);

        // HTTP uses its own rule.
        let http = intercept(&m, "http://example.local");
        assert_eq!(http.uri(), "http://http.proxy:8080/");
        assert!(http.basic_auth().is_some());

        // HTTPS falls back to the catch-all rule, which must carry the
        // credentials as well.
        let https = intercept(&m, "https://example.local");
        assert_eq!(https.uri(), "http://all.proxy:3128/");
        assert_eq!(https.basic_auth().unwrap(), "Basic dXNlcjpwYXNz");
    }

    /// The builder accepts the usual string types for the rule values.
    #[test]
    fn test_builder_accepts_string_types() {
        let owned = String::from("http://owned.proxy:8080");
        let m = Matcher::builder().all(&owned).build(Extra::default());

        assert_eq!(
            intercept(&m, "http://example.local").uri(),
            "http://owned.proxy:8080/"
        );
    }

    /// The environment-driven rules are read through the environment source.
    #[test]
    fn test_builder_reads_the_environment_source() {
        struct TestEnv;

        impl super::super::EnvSource for TestEnv {
            fn get(&self, names: &[&str]) -> Option<String> {
                match names {
                    ["ALL_PROXY", ..] => Some("http://all.proxy:3128".to_owned()),
                    ["NO_PROXY", ..] => Some("direct.local".to_owned()),
                    _ => None,
                }
            }
        }

        let builder = Builder::from_env_source(&TestEnv);
        assert_eq!(builder.all, "http://all.proxy:3128");
        assert_eq!(builder.no, "direct.local");
        assert!(builder.http.is_empty());
        assert!(builder.https.is_empty());

        let m = builder.build(Extra::default());
        assert_eq!(
            intercept(&m, "http://example.local").uri(),
            "http://all.proxy:3128/"
        );
        assert!(
            m.intercept(&"http://direct.local".parse().unwrap())
                .is_none()
        );
    }
}
