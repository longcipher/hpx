//! HTTP Cookies

use std::{
    fmt,
    sync::{Arc, OnceLock},
    time::SystemTime,
};

use bytes::Bytes;
use cookie::{Cookie as RawCookie, CookieJar, Expiration, SameSite, time::OffsetDateTime};
use http::Uri;
use publicsuffix::{List as PslList, Psl};
use scc::HashMap as SccHashMap;

use crate::{IntoUri, error::Error, ext::UriExt, header::HeaderValue};

// The Mozilla Public Suffix List, bundled at compile time and parsed once on
// first use. Stored behind an `Arc` so every `Jar` shares a single parsed
// copy (~230 KB) instead of re-parsing per instance.
static PSL: OnceLock<Arc<PslList>> = OnceLock::new();

/// Returns a shared, lazily-initialized handle to the bundled Public Suffix
/// List. Parsing only happens once per process; subsequent callers receive an
/// `Arc::clone` (a single atomic refcount bump).
fn shared_psl() -> Arc<PslList> {
    PSL.get_or_init(|| {
        let list =
            PslList::from_bytes(include_bytes!("public_suffix_list.dat")).unwrap_or_default();
        Arc::new(list)
    })
    .clone()
}

/// Cookie header values in two forms.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Cookies {
    /// All cookies combined into one header (compressed).
    Compressed(HeaderValue),

    /// Each cookie sent as its own header (uncompressed).
    Uncompressed(Vec<HeaderValue>),

    /// No cookies.
    Empty,
}

/// Actions for a persistent cookie store providing session support.
pub trait CookieStore: Send + Sync {
    /// Store a set of Set-Cookie header values received from `uri`
    fn set_cookies(&self, cookie_headers: &mut dyn Iterator<Item = &HeaderValue>, uri: &Uri);

    /// Get any [`Cookies`] in the store for `uri`
    fn cookies(&self, uri: &Uri) -> Cookies;
}

/// Trait for converting types into a shared cookie store ([`Arc<dyn CookieStore>`]).
///
/// Implemented for any [`CookieStore`] type, [`Arc<T>`] where `T: CookieStore`, and [`Arc<dyn
/// CookieStore>`]. Enables ergonomic conversion to a trait object for use in APIs without manual
/// boxing.
pub trait IntoCookieStore {
    /// Converts the implementor into an [`Arc<dyn CookieStore>`].
    ///
    /// This method allows ergonomic conversion of concrete cookie stores, [`Arc<T>`], or
    /// existing [`Arc<dyn CookieStore>`] into a trait object suitable for APIs that expect
    /// a shared cookie store.
    fn into_cookie_store(self) -> Arc<dyn CookieStore>;
}

/// Trait for converting types into an owned cookie ([`Cookie<'static>`]).
pub trait IntoCookie {
    /// Converts the implementor into an optional owned [`Cookie<'static>`].
    fn into_cookie(self) -> Option<Cookie<'static>>;
}

/// A single HTTP cookie.
#[derive(Debug, Clone)]
pub struct Cookie<'a>(RawCookie<'a>);

/// A good default `CookieStore` implementation.
///
/// This is the implementation used when simply calling `cookie_store(true)`.
/// This type is exposed to allow creating one and filling it with some
/// existing cookies more easily, before creating a [`crate::Client`].
///
/// Uses `scc::HashMap` for lock-free reads with fine-grained entry-level
/// mutations, avoiding the full-map clone that `ArcSwap` requires on every
/// write.
///
/// Includes a bundled Mozilla Public Suffix List (PSL) so that cookies with a
/// `Domain` attribute pointing at a public suffix (e.g. `co.uk`,
/// `github.io`) are rejected per RFC 6265 Section 5.3 step 6, preventing
/// cross-registrable-domain cookie leakage.
pub struct Jar {
    compression: bool,
    store: Arc<SccHashMap<String, SccHashMap<String, CookieJar>>>,
    psl: Arc<PslList>,
}

// ===== impl CookieStore =====

impl_request_config_value!(Arc<dyn CookieStore>);

// ===== impl IntoCookieStore =====

impl IntoCookieStore for Arc<dyn CookieStore> {
    #[inline]
    fn into_cookie_store(self) -> Arc<dyn CookieStore> {
        self
    }
}

impl<R> IntoCookieStore for Arc<R>
where
    R: CookieStore + 'static,
{
    #[inline]
    fn into_cookie_store(self) -> Arc<dyn CookieStore> {
        self
    }
}

impl<R> IntoCookieStore for R
where
    R: CookieStore + 'static,
{
    #[inline]
    fn into_cookie_store(self) -> Arc<dyn CookieStore> {
        Arc::new(self)
    }
}

// ===== impl IntoCookie =====

impl IntoCookie for Cookie<'_> {
    #[inline]
    fn into_cookie(self) -> Option<Cookie<'static>> {
        Some(self.into_owned())
    }
}

impl IntoCookie for RawCookie<'_> {
    #[inline]
    fn into_cookie(self) -> Option<Cookie<'static>> {
        Some(Cookie(self.into_owned()))
    }
}

impl IntoCookie for &str {
    #[inline]
    fn into_cookie(self) -> Option<Cookie<'static>> {
        RawCookie::parse(self).map(|c| Cookie(c.into_owned())).ok()
    }
}

// ===== impl Cookie =====

impl<'a> Cookie<'a> {
    pub(crate) fn parse(value: &'a HeaderValue) -> crate::Result<Self> {
        std::str::from_utf8(value.as_bytes())
            .map_err(cookie::ParseError::from)
            .and_then(cookie::Cookie::parse)
            .map_err(Error::decode)
            .map(Cookie)
    }

    /// The name of the cookie.
    #[inline]
    pub fn name(&self) -> &str {
        self.0.name()
    }

    /// The value of the cookie.
    #[inline]
    pub fn value(&self) -> &str {
        self.0.value()
    }

    /// Returns true if the 'HttpOnly' directive is enabled.
    #[inline]
    pub fn http_only(&self) -> bool {
        self.0.http_only().unwrap_or(false)
    }

    /// Returns true if the 'Secure' directive is enabled.
    #[inline]
    pub fn secure(&self) -> bool {
        self.0.secure().unwrap_or(false)
    }

    /// Returns true if  'SameSite' directive is 'Lax'.
    #[inline]
    pub fn same_site_lax(&self) -> bool {
        self.0.same_site() == Some(SameSite::Lax)
    }

    /// Returns true if  'SameSite' directive is 'Strict'.
    #[inline]
    pub fn same_site_strict(&self) -> bool {
        self.0.same_site() == Some(SameSite::Strict)
    }

    /// Returns the path directive of the cookie, if set.
    #[inline]
    pub fn path(&self) -> Option<&str> {
        self.0.path()
    }

    /// Returns the domain directive of the cookie, if set.
    #[inline]
    pub fn domain(&self) -> Option<&str> {
        self.0.domain()
    }

    /// Get the Max-Age information.
    #[inline]
    pub fn max_age(&self) -> Option<std::time::Duration> {
        self.0.max_age().and_then(|d| d.try_into().ok())
    }

    /// The cookie expiration time.
    #[inline]
    pub fn expires(&self) -> Option<SystemTime> {
        match self.0.expires() {
            Some(Expiration::DateTime(offset)) => Some(SystemTime::from(offset)),
            None | Some(Expiration::Session) => None,
        }
    }

    /// Converts `self` into a `Cookie` with a static lifetime with as few
    /// allocations as possible.
    #[inline]
    pub fn into_owned(self) -> Cookie<'static> {
        Cookie(self.0.into_owned())
    }
}

impl fmt::Display for Cookie<'_> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<'c> From<RawCookie<'c>> for Cookie<'c> {
    #[inline]
    fn from(cookie: RawCookie<'c>) -> Self {
        Cookie(cookie)
    }
}

impl<'c> From<Cookie<'c>> for RawCookie<'c> {
    #[inline]
    fn from(cookie: Cookie<'c>) -> Self {
        cookie.0
    }
}

// ===== impl Jar =====

impl Jar {
    /// Creates a new [`Jar`] with the specified compression setting.
    pub fn new(compression: bool) -> Self {
        Self {
            compression,
            store: Arc::new(SccHashMap::new()),
            psl: shared_psl(),
        }
    }

    /// Clone this [`Jar`], sharing storage but enabling compression.
    pub fn compressed(self: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self {
            compression: true,
            store: self.store.clone(),
            psl: self.psl.clone(),
        })
    }

    /// Clone this [`Jar`], sharing storage but disabling compression.
    pub fn uncompressed(self: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self {
            compression: false,
            store: self.store.clone(),
            psl: self.psl.clone(),
        })
    }

    /// Get a cookie by name for a given Uri.
    ///
    /// Returns the cookie with the specified name for the domain and path
    /// derived from the given Uri, if it exists.
    ///
    /// # Example
    /// ```
    /// use hpx::cookie::Jar;
    /// let jar = Jar::default();
    /// jar.add_cookie_str(
    ///     "foo=bar; Path=/foo; Domain=example.com",
    ///     "http://example.com/foo",
    /// );
    /// let cookie = jar.get("foo", "http://example.com/foo").unwrap();
    /// assert_eq!(cookie.value(), "bar");
    /// ```
    pub fn get<U: IntoUri>(&self, name: &str, uri: U) -> Option<Cookie<'static>> {
        let uri = uri.into_uri().ok()?;
        let host = uri.host()?;
        let path = uri.path();
        let path_entry = self.store.get_sync(host)?;
        let cookie_jar = path_entry.get_sync(path)?;
        cookie_jar
            .get()
            .get(name)
            .map(|c| Cookie(c.clone().into_owned()))
    }

    /// Get all cookies in this jar.
    ///
    /// Returns an iterator over all cookies currently stored in the jar,
    /// regardless of domain or path.
    ///
    /// # Example
    /// ```
    /// use hpx::cookie::Jar;
    /// let jar = Jar::default();
    /// jar.add_cookie_str("foo=bar; Domain=example.com", "http://example.com");
    /// for cookie in jar.get_all() {
    ///     println!("{}={}", cookie.name(), cookie.value());
    /// }
    /// ```
    pub fn get_all(&self) -> Vec<Cookie<'static>> {
        let mut cookies = Vec::new();
        self.store.iter_sync(|_, path_map| {
            path_map.iter_sync(|_, cookie_jar| {
                for cookie in cookie_jar.iter() {
                    cookies.push(Cookie(cookie.clone().into_owned()));
                }
                true
            });
            true
        });
        cookies
    }

    /// Add a cookie str to this jar.
    ///
    /// # Example
    ///
    /// ```
    /// use hpx::cookie::Jar;
    ///
    /// let cookie = "foo=bar; Domain=yolo.local";
    /// let jar = Jar::default();
    /// jar.add_cookie_str(cookie, "https://yolo.local");
    /// ```
    pub fn add_cookie_str<U: IntoUri>(&self, cookie: &str, uri: U) {
        self.add(cookie, uri);
    }

    /// Add a cookie to this jar.
    ///
    /// The cookie's domain and path attributes are used if present, otherwise
    /// the domain and path are derived from the provided Uri. A cookie whose
    /// `Domain` attribute the Uri's host does not domain-match is ignored.
    ///
    /// # Example
    /// ```
    /// use cookie::CookieBuilder;
    /// use hpx::cookie::Jar;
    /// let jar = Jar::default();
    /// let cookie = CookieBuilder::new("foo", "bar")
    ///     .domain("example.com")
    ///     .path("/")
    ///     .build();
    /// jar.add(cookie, "http://example.com");
    ///
    /// // You can also use a string directly
    /// jar.add("foo=bar; Domain=example.com; Path=/", "http://example.com");
    /// ```
    pub fn add<C, U>(&self, cookie: C, uri: U)
    where
        C: IntoCookie,
        U: IntoUri,
    {
        let Some(cookie) = cookie.into_cookie() else {
            return;
        };
        // Single storage path, so the domain and expiry rules in
        // `add_cookie` cannot drift between the two entry points.
        self.add_cookie(cookie, uri);
    }

    /// Add a cookie to this jar (legacy API).
    ///
    /// The cookie's domain and path attributes are used if present, otherwise
    /// the domain and path are derived from the provided Uri. A cookie whose
    /// `Domain` attribute the Uri's host does not domain-match is ignored.
    ///
    /// # Example
    /// ```
    /// use cookie::CookieBuilder;
    /// use hpx::cookie::Jar;
    /// let jar = Jar::default();
    /// let cookie = CookieBuilder::new("foo", "bar")
    ///     .domain("example.com")
    ///     .path("/")
    ///     .build();
    /// jar.add_cookie(cookie, "http://example.com");
    /// ```
    #[expect(clippy::unwrap_or_default)]
    pub fn add_cookie<C, U>(&self, cookie: C, uri: U)
    where
        C: Into<RawCookie<'static>>,
        U: IntoUri,
    {
        let mut cookie: RawCookie<'static> = cookie.into();
        let Ok(uri) = uri.into_uri() else { return };
        let host = uri.host();

        // RFC 6265 Section 5.3 step 3: Max-Age takes precedence over Expires.
        // Store it as an absolute Expires, because the read path only checks
        // Expires; a Max-Age-only cookie would otherwise never expire.
        let expired = match cookie.max_age() {
            Some(max_age) if max_age.is_positive() => {
                cookie.set_expires(OffsetDateTime::now_utc().saturating_add(max_age));
                false
            }
            Some(_) => true,
            None => cookie
                .expires_datetime()
                .is_some_and(|dt| dt <= SystemTime::now()),
        };

        // Public Suffix List check (RFC 6265 Section 5.3 step 5): reject
        // cookies whose `Domain` attribute is itself a public suffix (e.g.
        // `co.uk`, `github.io`) to prevent cross-registrable-domain cookie
        // leakage.
        if let Some(d) = cookie.domain().map(normalize_domain)
            && is_public_suffix(&self.psl, d)
        {
            if host != Some(d) {
                return;
            }
            // Exception: when the cookie's domain equals the request host AND
            // the host is itself a public suffix, allow the cookie but make it
            // host-only by dropping the attribute.
            cookie.unset_domain();
        }

        let domain: &str = match cookie.domain().map(normalize_domain) {
            // RFC 6265 Section 5.3 step 6: a `Domain` attribute is honoured
            // only when the request host domain-matches it. Otherwise any site
            // could plant cookies for another one (`Domain=bank.com` sent by
            // attacker.com).
            Some(d) if host.is_some_and(|h| domain_match(h, d)) => d,
            Some(_) => return,
            // No `Domain` attribute: a host-only cookie. It keeps
            // `domain() == None`, which `cookies()` uses as the host-only flag.
            None => host.unwrap_or_default(),
        };
        let path = cookie.path().unwrap_or_else(|| normalize_path(&uri));

        self.store
            .entry_sync(domain.to_owned())
            .or_insert_with(SccHashMap::new)
            .get()
            .entry_sync(path.to_owned())
            .or_insert_with(CookieJar::default)
            .get_mut()
            .remove(cookie.name().to_owned());

        if !expired {
            self.store
                .entry_sync(domain.to_owned())
                .or_insert_with(SccHashMap::new)
                .get()
                .entry_sync(path.to_owned())
                .or_insert_with(CookieJar::default)
                .get_mut()
                .add(cookie);
        }
    }

    /// Remove a cookie by name for a given Uri.
    ///
    /// Removes the cookie with the specified name for the domain and path
    /// derived from the given Uri, if it exists.
    ///
    /// # Example
    /// ```
    /// use hpx::cookie::Jar;
    /// let jar = Jar::default();
    /// jar.add_cookie_str(
    ///     "foo=bar; Path=/foo; Domain=example.com",
    ///     "http://example.com/foo",
    /// );
    /// assert!(jar.get("foo", "http://example.com/foo").is_some());
    /// jar.remove("foo", "http://example.com/foo");
    /// assert!(jar.get("foo", "http://example.com/foo").is_none());
    /// ```
    #[expect(clippy::unwrap_or_default)]
    pub fn remove<C, U>(&self, cookie: C, uri: U)
    where
        C: Into<RawCookie<'static>>,
        U: IntoUri,
    {
        let cookie_raw = cookie.into();
        let Ok(uri) = uri.into_uri() else { return };
        let Some(host) = uri.host() else { return };
        let path = uri.path().to_owned();

        self.store
            .entry_sync(host.to_owned())
            .or_insert_with(SccHashMap::new)
            .get()
            .entry_sync(path)
            .or_insert_with(CookieJar::default)
            .get_mut()
            .remove(cookie_raw.name().to_owned());
    }

    /// Clear all cookies from this jar.
    ///
    /// Removes all cookies from the jar, leaving it empty.
    ///
    /// # Example
    /// ```
    /// use hpx::cookie::Jar;
    /// let jar = Jar::default();
    /// jar.add_cookie_str("foo=bar; Domain=example.com", "http://example.com");
    /// assert_eq!(jar.get_all().len(), 1);
    /// jar.clear();
    /// assert_eq!(jar.get_all().len(), 0);
    /// ```
    pub fn clear(&self) {
        self.store.clear_sync();
    }
}

impl CookieStore for Jar {
    fn set_cookies(&self, cookie_headers: &mut dyn Iterator<Item = &HeaderValue>, uri: &Uri) {
        let cookies = cookie_headers
            .map(Cookie::parse)
            .filter_map(Result::ok)
            .map(|cookie| cookie.0.into_owned());

        for cookie in cookies {
            self.add_cookie(cookie, uri);
        }
    }

    fn cookies(&self, uri: &Uri) -> Cookies {
        let host = match uri.host() {
            Some(h) => h,
            None => return Cookies::Empty,
        };

        if self.compression {
            let mut cookies = String::new();
            self.store.iter_sync(|domain, path_map| {
                if !domain_match(host, domain) {
                    return true;
                }
                // Host-only cookies (stored without a `Domain` attribute) go
                // to the exact host that set them, never to its subdomains
                // (RFC 6265 Section 5.4 step 1).
                let exact_host = host == domain.as_str();
                path_map.iter_sync(|path, cookie_jar| {
                    if !path_match(uri.path(), path) {
                        return true;
                    }
                    for cookie in cookie_jar.iter() {
                        if cookie.domain().is_none() && !exact_host {
                            continue;
                        }
                        if cookie.secure() == Some(true) && uri.is_http() {
                            continue;
                        }
                        if cookie
                            .expires_datetime()
                            .is_some_and(|dt| dt <= SystemTime::now())
                        {
                            continue;
                        }
                        if !cookies.is_empty() {
                            cookies.push_str("; ");
                        }
                        cookies.push_str(cookie.name());
                        cookies.push('=');
                        cookies.push_str(cookie.value());
                    }
                    true
                });
                true
            });

            if cookies.is_empty() {
                return Cookies::Empty;
            }

            HeaderValue::from_maybe_shared(Bytes::from(cookies))
                .map_or(Cookies::Empty, Cookies::Compressed)
        } else {
            let mut cookie_strs = Vec::new();
            self.store.iter_sync(|domain, path_map| {
                if !domain_match(host, domain) {
                    return true;
                }
                // Host-only cookies match the exact host only; see above.
                let exact_host = host == domain.as_str();
                path_map.iter_sync(|path, cookie_jar| {
                    if !path_match(uri.path(), path) {
                        return true;
                    }
                    for cookie in cookie_jar.iter() {
                        if cookie.domain().is_none() && !exact_host {
                            continue;
                        }
                        if cookie.secure() == Some(true) && uri.is_http() {
                            continue;
                        }
                        if cookie
                            .expires_datetime()
                            .is_some_and(|dt| dt <= SystemTime::now())
                        {
                            continue;
                        }
                        let name = cookie.name();
                        let value = cookie.value();
                        let mut s = String::with_capacity(name.len() + 1 + value.len());
                        s.push_str(name);
                        s.push('=');
                        s.push_str(value);
                        cookie_strs.push(s);
                    }
                    true
                });
                true
            });

            let cookies = cookie_strs
                .into_iter()
                .filter_map(|s| HeaderValue::from_maybe_shared(Bytes::from(s)).ok())
                .collect();

            Cookies::Uncompressed(cookies)
        }
    }
}

impl Default for Jar {
    fn default() -> Self {
        Self::new(true)
    }
}

// Jar needs fmt::Debug since the store is ArcSwap now
impl fmt::Debug for Jar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Jar")
            .field("compression", &self.compression)
            .finish_non_exhaustive()
    }
}

const DEFAULT_PATH: &str = "/";

/// Returns `true` when `domain` is a public suffix (e.g. `com`, `co.uk`,
/// `github.io`). Uses the bundled Mozilla Public Suffix List.
fn is_public_suffix(psl: &PslList, domain: &str) -> bool {
    psl.suffix(domain.as_bytes()).is_some_and(|s| s == domain)
}

/// Determines if the given `host` matches the cookie `domain` according to
/// [RFC 6265 section 5.1.3](https://datatracker.ietf.org/doc/html/rfc6265#section-5.1.3).
///
/// - Returns true if the host and domain are identical.
/// - Returns true if the host is a subdomain of the domain (host ends with ".domain").
/// - Returns false otherwise.
fn domain_match(host: &str, domain: &str) -> bool {
    if domain.is_empty() {
        return false;
    }
    if host == domain {
        return true;
    }
    host.len() > domain.len()
        && host.as_bytes()[host.len() - domain.len() - 1] == b'.'
        && host.ends_with(domain)
}

/// Determines if the request path matches the cookie path according to
/// [RFC 6265 section 5.1.4](https://datatracker.ietf.org/doc/html/rfc6265#section-5.1.4).
///
/// - Returns true if the request path and cookie path are identical.
/// - Returns true if the request path starts with the cookie path, and
///   - the cookie path ends with '/', or
///   - the next character in the request path after the cookie path is '/'.
/// - Returns false otherwise.
fn path_match(req_path: &str, cookie_path: &str) -> bool {
    req_path == cookie_path
        || req_path.starts_with(cookie_path)
            && (cookie_path.ends_with(DEFAULT_PATH)
                || req_path[cookie_path.len()..].starts_with(DEFAULT_PATH))
}

/// Normalizes a domain by stripping any port information.
///
/// According to [RFC 6265 section 5.2.3](https://datatracker.ietf.org/doc/html/rfc6265#section-5.2.3),
/// the domain attribute of a cookie must not include a port. If a port is present (non-standard),
/// it will be ignored for domain matching purposes.
fn normalize_domain(domain: &str) -> &str {
    domain.split(':').next().unwrap_or(domain)
}

/// Computes the normalized default path for a cookie as specified in
/// [RFC 6265 section 5.1.4](https://datatracker.ietf.org/doc/html/rfc6265#section-5.1.4).
///
/// This function normalizes the path for a cookie, ensuring it matches
/// browser and server expectations for default cookie scope.
fn normalize_path(uri: &Uri) -> &str {
    let path = uri.path();
    if !path.starts_with(DEFAULT_PATH) {
        return DEFAULT_PATH;
    }
    if let Some(pos) = path.rfind(DEFAULT_PATH) {
        if pos == 0 {
            return DEFAULT_PATH;
        }
        return &path[..pos];
    }
    DEFAULT_PATH
}

#[cfg(test)]
#[cfg(feature = "cookies")]
mod tests {
    use std::sync::Arc;

    use http::Uri;

    use crate::{
        cookie::{CookieStore, Cookies, Jar},
        header::HeaderValue,
    };

    fn header_values<'a>(
        values: &'a [HeaderValue],
    ) -> Box<dyn Iterator<Item = &'a HeaderValue> + 'a> {
        Box::new(values.iter())
    }

    #[test]
    fn set_cookies_and_retrieve() {
        let jar = Jar::default();
        let uri: Uri = "http://example.com/".parse().unwrap();

        let set_cookie: HeaderValue = HeaderValue::from_static("session=abc123; Path=/");

        jar.set_cookies(&mut header_values(&[set_cookie]), &uri);

        match jar.cookies(&uri) {
            Cookies::Compressed(val) => {
                let s = val.to_str().unwrap();
                assert!(s.contains("session=abc123"));
            }
            other => panic!("expected Cookies::Compressed, got {other:?}"),
        }
    }

    #[test]
    fn domain_scoping() {
        let jar = Jar::default();
        let example_uri: Uri = "http://example.com/".parse().unwrap();
        let other_uri: Uri = "http://other.com/".parse().unwrap();

        let set_cookie: HeaderValue = HeaderValue::from_static("token=xyz; Domain=example.com");

        jar.set_cookies(&mut header_values(&[set_cookie]), &example_uri);

        assert!(
            !matches!(jar.cookies(&example_uri), Cookies::Empty),
            "cookie should be returned for example.com"
        );

        assert!(
            matches!(jar.cookies(&other_uri), Cookies::Empty),
            "cookie should NOT be returned for other.com"
        );
    }

    #[test]
    fn path_scoping() {
        let jar = Jar::default();
        let base_uri: Uri = "http://example.com/".parse().unwrap();
        let app_uri: Uri = "http://example.com/app/page".parse().unwrap();
        let other_uri: Uri = "http://example.com/other".parse().unwrap();

        let set_cookie: HeaderValue = HeaderValue::from_static("nav=open; Path=/app");

        jar.set_cookies(&mut header_values(&[set_cookie]), &base_uri);

        assert!(
            !matches!(jar.cookies(&app_uri), Cookies::Empty),
            "cookie should be returned for /app/page"
        );

        assert!(
            matches!(jar.cookies(&other_uri), Cookies::Empty),
            "cookie should NOT be returned for /other"
        );
    }

    #[test]
    fn expired_cookie_not_returned() {
        let jar = Jar::default();
        let uri: Uri = "http://example.com/".parse().unwrap();

        let set_cookie: HeaderValue =
            HeaderValue::from_static("stale=old; Expires=Wed, 21 Oct 2015 07:28:00 GMT");

        jar.set_cookies(&mut header_values(&[set_cookie]), &uri);

        assert!(
            matches!(jar.cookies(&uri), Cookies::Empty),
            "expired cookie should not be returned"
        );
    }

    #[tokio::test]
    async fn concurrent_insert_read_no_panic() {
        let jar = Arc::new(Jar::default());

        let mut handles = Vec::new();

        for i in 0..20u32 {
            let jar = Arc::clone(&jar);
            handles.push(tokio::task::spawn(async move {
                let uri: Uri = format!("http://host{i}.example/").parse().unwrap();
                let cookie_val = format!("key{i}=val{i}; Path=/");
                let hv = HeaderValue::from_str(&cookie_val).unwrap();

                jar.set_cookies(&mut header_values(&[hv]), &uri);

                let _ = jar.cookies(&uri);
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }
    }

    #[test]
    fn empty_set_cookie_adds_no_cookies() {
        let jar = Jar::default();
        let uri: Uri = "http://example.com/".parse().unwrap();

        let empty: HeaderValue = HeaderValue::from_static("");
        jar.set_cookies(&mut header_values(&[empty]), &uri);

        assert!(matches!(jar.cookies(&uri), Cookies::Empty));
    }

    #[test]
    fn multiple_set_cookie_headers_all_parsed() {
        let jar = Jar::default();
        let uri: Uri = "http://example.com/".parse().unwrap();

        let cookie1: HeaderValue = HeaderValue::from_static("a=1; Path=/");
        let cookie2: HeaderValue = HeaderValue::from_static("b=2; Path=/");

        jar.set_cookies(&mut header_values(&[cookie1, cookie2]), &uri);

        match jar.cookies(&uri) {
            Cookies::Compressed(val) => {
                let s = val.to_str().unwrap();
                assert!(s.contains("a=1"));
                assert!(s.contains("b=2"));
            }
            other => panic!("expected Cookies::Compressed with two cookies, got {other:?}"),
        }
    }

    #[test]
    fn domain_attribute_not_matching_request_host_is_rejected() {
        let jar = Jar::default();
        let attacker: Uri = "http://attacker.com/".parse().unwrap();
        let bank: Uri = "http://bank.com/".parse().unwrap();

        let set_cookie = HeaderValue::from_static("sid=evil; Domain=bank.com");
        jar.set_cookies(&mut header_values(&[set_cookie]), &attacker);

        assert!(
            matches!(jar.cookies(&bank), Cookies::Empty),
            "attacker.com must not set cookies for bank.com"
        );

        // `add` shares the same storage path.
        jar.add("sid=evil; Domain=bank.com", "http://attacker.com/");
        assert!(jar.get_all().is_empty());
    }

    #[test]
    fn domain_attribute_for_parent_domain_reaches_subdomains() {
        let jar = Jar::default();
        let www: Uri = "http://www.example.com/".parse().unwrap();
        let api: Uri = "http://api.example.com/".parse().unwrap();

        let set_cookie = HeaderValue::from_static("sid=ok; Domain=example.com");
        jar.set_cookies(&mut header_values(&[set_cookie]), &www);

        assert!(
            !matches!(jar.cookies(&api), Cookies::Empty),
            "a Domain cookie for a parent domain should reach sibling subdomains"
        );
    }

    #[test]
    fn host_only_cookie_not_sent_to_subdomain() {
        let jar = Arc::new(Jar::default());
        let host: Uri = "http://example.com/".parse().unwrap();
        let sub: Uri = "http://sub.example.com/".parse().unwrap();

        let set_cookie = HeaderValue::from_static("sid=abc");
        jar.set_cookies(&mut header_values(&[set_cookie]), &host);

        assert!(!matches!(jar.cookies(&host), Cookies::Empty));
        assert!(
            matches!(jar.cookies(&sub), Cookies::Empty),
            "a host-only cookie must not reach subdomains"
        );

        let uncompressed = jar.uncompressed();
        assert!(matches!(uncompressed.cookies(&host), Cookies::Uncompressed(v) if v.len() == 1));
        assert!(matches!(uncompressed.cookies(&sub), Cookies::Uncompressed(v) if v.is_empty()));
    }

    #[test]
    fn public_suffix_domain_equal_to_host_is_host_only() {
        let jar = Jar::default();
        let suffix_host: Uri = "http://co.uk/".parse().unwrap();
        let below: Uri = "http://example.co.uk/".parse().unwrap();

        let set_cookie = HeaderValue::from_static("sid=abc; Domain=co.uk");
        jar.set_cookies(&mut header_values(&[set_cookie]), &suffix_host);

        assert!(!matches!(jar.cookies(&suffix_host), Cookies::Empty));
        assert!(
            matches!(jar.cookies(&below), Cookies::Empty),
            "a public-suffix Domain cookie must stay host-only"
        );
    }

    #[test]
    fn max_age_takes_precedence_over_past_expires() {
        let jar = Jar::default();
        let uri: Uri = "http://example.com/".parse().unwrap();

        let set_cookie = HeaderValue::from_static(
            "sid=abc; Expires=Wed, 21 Oct 2015 07:28:00 GMT; Max-Age=3600",
        );
        jar.set_cookies(&mut header_values(&[set_cookie]), &uri);

        assert!(
            !matches!(jar.cookies(&uri), Cookies::Empty),
            "a positive Max-Age overrides a past Expires"
        );
    }

    #[test]
    fn zero_max_age_deletes_stored_cookie() {
        let jar = Jar::default();
        let uri: Uri = "http://example.com/".parse().unwrap();

        let set_cookie = HeaderValue::from_static("sid=abc");
        jar.set_cookies(&mut header_values(&[set_cookie]), &uri);
        assert_eq!(jar.get_all().len(), 1);

        let delete = HeaderValue::from_static("sid=abc; Max-Age=0");
        jar.set_cookies(&mut header_values(&[delete]), &uri);
        assert!(jar.get_all().is_empty());
    }

    #[test]
    fn max_age_is_stored_as_absolute_expiry() {
        use std::time::{Duration, SystemTime};

        let jar = Jar::default();
        let uri: Uri = "http://example.com/".parse().unwrap();

        let before = SystemTime::now();
        let set_cookie = HeaderValue::from_static("sid=abc; Max-Age=100");
        jar.set_cookies(&mut header_values(&[set_cookie]), &uri);
        let after = SystemTime::now();

        let expires = jar
            .get("sid", "http://example.com/")
            .and_then(|cookie| cookie.expires())
            .expect("Max-Age should be stored as an absolute expiry");
        assert!(expires >= before + Duration::from_secs(100));
        assert!(expires <= after + Duration::from_secs(100));
    }

    #[test]
    fn secure_httponly_samesite_attributes_parsed() {
        use crate::cookie::Cookie;

        let hv: HeaderValue =
            HeaderValue::from_static("securecookie=val; Secure; HttpOnly; SameSite=Strict");

        let cookie = Cookie::parse(&hv).expect("cookie should parse successfully");

        assert!(cookie.secure(), "Secure attribute should be set");
        assert!(cookie.http_only(), "HttpOnly attribute should be set");
        assert!(cookie.same_site_strict(), "SameSite should be Strict");
        assert!(!cookie.same_site_lax(), "SameSite should not be Lax");
    }
}
