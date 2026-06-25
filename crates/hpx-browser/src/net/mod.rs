//! Stealth HTTP client with cookie management, Accept-CH tracking, and
//! redirect following.
//!
//! Wraps `hpx::Client` as the underlying HTTP/1.1 + HTTP/2 transport with
//! BoringSSL TLS and browser-profile emulation. Higher-level browser
//! session concerns (cookies, Client Hints, H1-only host memory) live here.

pub mod blocklist;
pub mod cookies;
pub mod csp;
pub mod headers;
pub mod robots;
pub mod ssrf;

use std::{collections::HashMap, sync::Arc};

pub use cookies::CookieJar;
use tokio::sync::Mutex;
use url::Url;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("HTTP error: {0}")]
    Http(String),

    #[error("URL parse error: {0}")]
    Url(#[from] url::ParseError),

    #[error("Request failed: {0}")]
    Request(String),

    #[error("hpx client error: {0}")]
    Client(#[from] hpx::Error),
}

// ---------------------------------------------------------------------------
// TimingStats
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct TimingStats {
    pub dns_start_ms: f64,
    pub dns_end_ms: f64,
    pub connect_start_ms: f64,
    pub connect_end_ms: f64,
    pub tls_start_ms: f64,
    pub tls_end_ms: f64,
    pub request_start_ms: f64,
    pub response_start_ms: f64,
    pub response_end_ms: f64,
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Response {
    pub status: u16,
    pub status_text: String,
    pub headers: HashMap<String, String>,
    /// All Set-Cookie header values, preserved separately because HTTP
    /// responses can contain multiple Set-Cookie headers.
    pub set_cookies: Vec<String>,
    pub body: Vec<u8>,
    pub url: String,
    /// Whether this response taught the client Accept-CH for the first time.
    pub accept_ch_upgrade: bool,
    pub timings: TimingStats,
}

impl Response {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

// ---------------------------------------------------------------------------
// SharedSession — process-wide cookie jar + Accept-CH origins
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct SharedSession {
    pub cookies: Arc<Mutex<CookieJar>>,
    pub accept_ch: scc::HashSet<String>,
    pub h1_only_hosts: scc::HashSet<String>,
}

static SHARED_SESSION: std::sync::OnceLock<SharedSession> = std::sync::OnceLock::new();

/// Get the process-wide shared session.
pub fn shared_session() -> SharedSession {
    SHARED_SESSION
        .get_or_init(|| SharedSession {
            cookies: Arc::new(Mutex::new(CookieJar::new())),
            accept_ch: scc::HashSet::new(),
            h1_only_hosts: scc::HashSet::new(),
        })
        .clone()
}

// ---------------------------------------------------------------------------
// HttpClient
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct HttpClient {
    inner: hpx::Client,
    cookies: Arc<Mutex<CookieJar>>,
    accept_ch_origins: scc::HashSet<String>,
    h1_only_hosts: scc::HashSet<String>,
    browser_profile: hpx::BrowserProfile,
}

impl HttpClient {
    /// Create a new client with the given browser profile.
    pub fn new(browser_profile: hpx::BrowserProfile) -> Result<Self, NetError> {
        let inner = hpx::Client::builder()
            .build()
            .map_err(|e| NetError::Http(format!("failed to build hpx client: {e}")))?;

        Ok(Self {
            inner,
            cookies: Arc::new(Mutex::new(CookieJar::new())),
            accept_ch_origins: scc::HashSet::new(),
            h1_only_hosts: scc::HashSet::new(),
            browser_profile,
        })
    }

    /// Build a client that participates in the process-wide shared session.
    pub fn shared(browser_profile: hpx::BrowserProfile) -> Result<Self, NetError> {
        let s = shared_session();
        let inner = hpx::Client::builder()
            .build()
            .map_err(|e| NetError::Http(format!("failed to build hpx client: {e}")))?;

        Ok(Self {
            inner,
            cookies: s.cookies,
            accept_ch_origins: s.accept_ch,
            h1_only_hosts: s.h1_only_hosts,
            browser_profile,
        })
    }

    pub fn cookies(&self) -> Arc<Mutex<CookieJar>> {
        self.cookies.clone()
    }

    pub fn browser_profile(&self) -> &hpx::BrowserProfile {
        &self.browser_profile
    }

    /// Whether `host` has previously sent `Accept-CH`.
    pub fn has_accept_ch(&self, host: &str) -> bool {
        self.accept_ch_origins.contains_sync(host)
    }

    /// Learn Accept-CH from response headers. Returns `true` if this is a
    /// new origin that just opted in.
    fn learn_accept_ch(&self, host: &str, headers: &HashMap<String, String>) -> bool {
        let has_ch = headers.keys().any(|k| {
            let k = k.to_ascii_lowercase();
            k == "accept-ch" || k == "critical-ch"
        });
        if has_ch {
            return self.accept_ch_origins.insert_sync(host.to_string()).is_ok();
        }
        false
    }

    /// Snapshot all cookies for a URL.
    pub async fn cookies_for_url(&self, url: &Url) -> Option<String> {
        let jar = self.cookies.lock().await;
        jar.cookies_for(url)
    }

    /// Inject cookies from external sources (e.g., JS `document.cookie`).
    pub async fn inject_cookies(&self, url: &Url, cookies: &[String]) {
        let mut jar = self.cookies.lock().await;
        jar.set_cookies(url, cookies);
    }

    /// Set a single cookie from a raw Set-Cookie-style string.
    pub async fn set_cookie_str(&self, url: &Url, raw: &str) {
        let mut jar = self.cookies.lock().await;
        jar.set_cookies(url, &[raw.to_string()]);
    }

    /// Drop all cookies matching `target_domain`.
    pub async fn clear_cookies_for_domain(&self, target_domain: &str) {
        let mut jar = self.cookies.lock().await;
        jar.clear_for_domain(target_domain);
    }

    // ----- Request methods -----

    /// Perform a GET request.
    pub async fn get(&self, url: &str) -> Result<Response, NetError> {
        self.get_with_headers(url, &[]).await
    }

    /// GET with extra headers.
    pub async fn get_with_headers(
        &self,
        url: &str,
        extra_headers: &[(String, String)],
    ) -> Result<Response, NetError> {
        let parsed = Url::parse(url)?;
        let builder = self.inner.get(url).emulation(self.browser_profile);

        let builder = self
            .inject_request_headers(builder, &parsed, extra_headers)
            .await;
        let hpx_resp = builder.send().await?;
        self.process_response(hpx_resp, url, &parsed).await
    }

    /// Fetch-API-style GET with `accept: */*` semantics.
    pub async fn fetch_get(
        &self,
        url: &str,
        extra_headers: &[(String, String)],
        _origin: Option<&str>,
    ) -> Result<Response, NetError> {
        let parsed = Url::parse(url)?;
        let mut builder = self.inner.get(url).emulation(self.browser_profile);

        // Fetch-style: accept: */*, sec-fetch-dest: empty
        builder = builder.header("accept", "*/*");
        builder = builder.header("sec-fetch-mode", "cors");
        builder = builder.header("sec-fetch-dest", "empty");
        builder = builder.header("sec-fetch-site", "same-origin");

        builder = self
            .inject_request_headers(builder, &parsed, extra_headers)
            .await;
        let hpx_resp = builder.send().await?;
        self.process_response(hpx_resp, url, &parsed).await
    }

    /// Fetch-API-style POST with raw bytes.
    pub async fn fetch_post_bytes(
        &self,
        url: &str,
        body: &[u8],
        extra_headers: &[(String, String)],
        _origin: Option<&str>,
    ) -> Result<Response, NetError> {
        let parsed = Url::parse(url)?;
        let mut builder = self.inner.post(url).emulation(self.browser_profile);

        builder = builder.header("accept", "*/*");
        builder = builder.header("sec-fetch-mode", "cors");
        builder = builder.header("sec-fetch-dest", "empty");
        builder = builder.header("sec-fetch-site", "same-origin");

        builder = self
            .inject_request_headers(builder, &parsed, extra_headers)
            .await;
        let hpx_resp = builder.body(body.to_vec()).send().await?;
        self.process_response(hpx_resp, url, &parsed).await
    }

    /// Perform a POST request with a string body.
    pub async fn post(&self, url: &str, body: &str) -> Result<Response, NetError> {
        self.post_with_headers(url, body, &[]).await
    }

    /// POST with extra headers.
    pub async fn post_with_headers(
        &self,
        url: &str,
        body: &str,
        extra_headers: &[(String, String)],
    ) -> Result<Response, NetError> {
        self.post_bytes_with_headers(url, body.as_bytes(), extra_headers)
            .await
    }

    /// POST with raw bytes and extra headers.
    pub async fn post_bytes_with_headers(
        &self,
        url: &str,
        body: &[u8],
        extra_headers: &[(String, String)],
    ) -> Result<Response, NetError> {
        let parsed = Url::parse(url)?;
        let builder = self.inner.post(url).emulation(self.browser_profile);

        let builder = self
            .inject_request_headers(builder, &parsed, extra_headers)
            .await;
        let hpx_resp = builder.body(body.to_vec()).send().await?;
        self.process_response(hpx_resp, url, &parsed).await
    }

    /// GET with explicit redirect following.
    pub async fn get_follow(&self, url: &str, max_redirects: u8) -> Result<Response, NetError> {
        let mut current_url = url.to_string();
        for _ in 0..max_redirects {
            let resp = self.get(&current_url).await?;
            if matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
                if let Some(loc) = resp.headers.get("location") {
                    current_url = resolve_redirect(&current_url, loc)?;
                    continue;
                }
            }
            return Ok(resp);
        }
        self.get(&current_url).await
    }

    /// GET with extra headers and redirect following.
    pub async fn get_follow_with_headers(
        &self,
        url: &str,
        extra_headers: &[(String, String)],
        max_redirects: u8,
    ) -> Result<Response, NetError> {
        let mut current_url = url.to_string();
        for _ in 0..max_redirects {
            let resp = self.get_with_headers(&current_url, extra_headers).await?;
            if matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
                if let Some(loc) = resp.headers.get("location") {
                    current_url = resolve_redirect(&current_url, loc)?;
                    continue;
                }
            }
            return Ok(resp);
        }
        self.get_with_headers(&current_url, extra_headers).await
    }

    /// POST with redirect following. 307/308 preserve the body.
    pub async fn post_follow(
        &self,
        url: &str,
        body: &str,
        max_redirects: u8,
    ) -> Result<Response, NetError> {
        self.post_bytes_follow(url, body.as_bytes(), &[], max_redirects)
            .await
    }

    /// POST with raw bytes and redirect following.
    pub async fn post_bytes_follow(
        &self,
        url: &str,
        body: &[u8],
        extra_headers: &[(String, String)],
        max_redirects: u8,
    ) -> Result<Response, NetError> {
        let mut current_url = url.to_string();
        for _ in 0..max_redirects {
            let resp = self
                .post_bytes_with_headers(&current_url, body, extra_headers)
                .await?;

            if matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
                if let Some(loc) = resp.headers.get("location") {
                    let next_url = resolve_redirect(&current_url, loc)?;
                    if matches!(resp.status, 307 | 308) {
                        current_url = next_url;
                        continue;
                    }
                    // 301/302/303 on POST → switch to GET
                    return self
                        .get_follow(&next_url, max_redirects.saturating_sub(1))
                        .await;
                }
            }
            return Ok(resp);
        }
        self.post_bytes_with_headers(&current_url, body, extra_headers)
            .await
    }

    /// Pre-establish a connection to a host. hpx handles connection pooling
    /// internally, so this is a lightweight GET that warms the pool.
    pub async fn preconnect(&self, url: &str) -> Result<(), NetError> {
        // ponytail: hpx manages its own pool; a HEAD is the cheapest way to
        // establish a connection. If hpx ever exposes a dedicated preconnect,
        // switch to that.
        let _ = self
            .inner
            .head(url)
            .emulation(self.browser_profile)
            .send()
            .await;
        Ok(())
    }

    // ----- Internal helpers -----

    /// Inject cookies and extra headers into a request builder.
    async fn inject_request_headers(
        &self,
        mut builder: hpx::RequestBuilder,
        parsed: &Url,
        extra_headers: &[(String, String)],
    ) -> hpx::RequestBuilder {
        let cookie_str = {
            let jar = self.cookies.lock().await;
            jar.cookies_for(parsed)
        };

        if let Some(cs) = cookie_str {
            builder = builder.header("cookie", cs);
        }

        for (k, v) in extra_headers {
            if k.eq_ignore_ascii_case("host") || k.eq_ignore_ascii_case("connection") {
                continue;
            }
            builder = builder.header(k.as_str(), v.as_str());
        }

        builder
    }

    /// Convert an hpx Response into our Response type.
    async fn process_response(
        &self,
        hpx_resp: hpx::Response,
        url: &str,
        parsed: &Url,
    ) -> Result<Response, NetError> {
        let status = hpx_resp.status().as_u16();
        let status_text = hpx_resp
            .status()
            .canonical_reason()
            .unwrap_or("")
            .to_string();

        let mut headers = HashMap::new();
        let mut set_cookies = Vec::new();

        for (key, value) in hpx_resp.headers() {
            if let Ok(v) = value.to_str() {
                if key.as_str().eq_ignore_ascii_case("set-cookie") {
                    set_cookies.push(v.to_string());
                } else {
                    headers.insert(key.to_string(), v.to_string());
                }
            }
        }

        let body = hpx_resp
            .bytes()
            .await
            .map_err(|e| NetError::Http(format!("failed to read body: {e}")))?;

        // Learn Accept-CH
        let host = parsed.host_str().unwrap_or("");
        let upgrade = self.learn_accept_ch(host, &headers);

        // Store Set-Cookie
        if !set_cookies.is_empty() {
            let mut jar = self.cookies.lock().await;
            jar.set_cookies(parsed, &set_cookies);
        }

        Ok(Response {
            status,
            status_text,
            headers,
            set_cookies,
            body: body.to_vec(),
            url: url.to_string(),
            accept_ch_upgrade: upgrade,
            timings: TimingStats::default(),
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve a redirect Location header to an absolute URL.
fn resolve_redirect(current_url: &str, location: &str) -> Result<String, NetError> {
    let base = Url::parse(current_url).map_err(|e| NetError::Request(e.to_string()))?;
    let resolved = base.join(location).map_err(|e| {
        NetError::Request(format!(
            "redirect resolve: {e} (base={current_url}, loc={location})"
        ))
    })?;
    Ok(resolved.to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_creates_successfully() {
        let client = HttpClient::new(hpx::BrowserProfile::Chrome);
        assert!(client.is_ok());
    }

    #[test]
    fn shared_client_creates_successfully() {
        let client = HttpClient::shared(hpx::BrowserProfile::Chrome);
        assert!(client.is_ok());
    }

    #[test]
    fn redirect_resolve_handles_rfc3986_cases() {
        // Absolute
        assert_eq!(
            resolve_redirect("https://a.com/x", "https://b.com/y").unwrap(),
            "https://b.com/y"
        );
        // Root-relative
        assert_eq!(
            resolve_redirect("https://a.com/x/y", "/z").unwrap(),
            "https://a.com/z"
        );
        // Relative
        assert_eq!(
            resolve_redirect("https://a.com/x/y", "z.html").unwrap(),
            "https://a.com/x/z.html"
        );
        // Dot segments
        assert_eq!(
            resolve_redirect("https://a.com/x/y/", "../z.html").unwrap(),
            "https://a.com/x/z.html"
        );
        // Scheme-relative
        assert_eq!(
            resolve_redirect("https://a.com/x", "//b.com/y").unwrap(),
            "https://b.com/y"
        );
        // Query-only
        assert_eq!(
            resolve_redirect("https://a.com/x?old=1", "?new=2").unwrap(),
            "https://a.com/x?new=2"
        );
    }

    #[test]
    fn response_text_and_ok() {
        let resp = Response {
            status: 200,
            status_text: "OK".into(),
            headers: HashMap::new(),
            set_cookies: Vec::new(),
            body: b"Hello world".to_vec(),
            url: "https://example.com".into(),
            accept_ch_upgrade: false,
            timings: TimingStats::default(),
        };
        assert_eq!(resp.text(), "Hello world");
        assert!(resp.ok());
    }

    #[test]
    fn response_not_ok() {
        let resp = Response {
            status: 404,
            status_text: "Not Found".into(),
            headers: HashMap::new(),
            set_cookies: Vec::new(),
            body: vec![],
            url: "https://example.com/missing".into(),
            accept_ch_upgrade: false,
            timings: TimingStats::default(),
        };
        assert!(!resp.ok());
    }

    #[test]
    fn cookie_jar_set_and_get() {
        let mut jar = CookieJar::new();
        let url = Url::parse("https://example.com/path").unwrap();
        jar.set_cookies(&url, &["session=abc123; Path=/; Secure".to_string()]);
        assert_eq!(jar.cookie_count(), 1);
        let cookies = jar.cookies_for(&url);
        assert_eq!(cookies, Some("session=abc123".to_string()));
    }

    #[test]
    fn cookie_jar_domain_scope() {
        let mut jar = CookieJar::new();
        let url = Url::parse("https://sub.example.com").unwrap();
        jar.set_cookies(&url, &["token=xyz; Domain=example.com".to_string()]);
        // Parent domain cookie visible on subdomain
        assert_eq!(jar.cookie_count(), 1);
        let cookies = jar.cookies_for(&url);
        assert!(cookies.is_some());
        assert!(cookies.unwrap().contains("token=xyz"));
    }

    #[test]
    fn cookie_jar_cross_domain_reject() {
        let mut jar = CookieJar::new();
        let url = Url::parse("https://example.com").unwrap();
        jar.set_cookies(&url, &["evil=hack; Domain=evil.com".to_string()]);
        assert_eq!(jar.cookie_count(), 0);
    }

    #[test]
    fn cookie_jar_clear_for_domain() {
        let mut jar = CookieJar::new();
        let url = Url::parse("https://example.com").unwrap();
        jar.set_cookies(&url, &["a=1".to_string(), "b=2".to_string()]);
        assert_eq!(jar.cookie_count(), 2);
        jar.clear_for_domain("example.com");
        assert_eq!(jar.cookie_count(), 0);
    }

    #[test]
    fn accept_ch_starts_false_then_true() {
        let client = HttpClient::new(hpx::BrowserProfile::Chrome).unwrap();
        assert!(!client.has_accept_ch("example.com"));

        let mut headers = HashMap::new();
        headers.insert(
            "accept-ch".to_string(),
            "Sec-CH-UA-Full-Version-List".to_string(),
        );
        client.learn_accept_ch("example.com", &headers);

        assert!(client.has_accept_ch("example.com"));
        assert!(!client.has_accept_ch("other.com"));
    }

    #[test]
    fn accept_ch_case_insensitive() {
        let client = HttpClient::new(hpx::BrowserProfile::Chrome).unwrap();
        let mut headers = HashMap::new();
        headers.insert("Accept-CH".to_string(), "Sec-CH-UA-Arch".to_string());
        client.learn_accept_ch("site.example", &headers);
        assert!(client.has_accept_ch("site.example"));
    }

    #[test]
    fn response_without_accept_ch_does_not_upgrade() {
        let client = HttpClient::new(hpx::BrowserProfile::Chrome).unwrap();
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "text/html".to_string());
        client.learn_accept_ch("boring.example", &headers);
        assert!(!client.has_accept_ch("boring.example"));
    }

    #[tokio::test]
    #[ignore] // requires network
    async fn get_request() {
        let client = HttpClient::new(hpx::BrowserProfile::Chrome).unwrap();
        let resp = client.get("https://httpbin.org/get").await.unwrap();
        assert_eq!(resp.status, 200);
        assert!(resp.text().contains("httpbin"));
    }

    #[tokio::test]
    #[ignore] // requires network
    async fn post_request() {
        let client = HttpClient::new(hpx::BrowserProfile::Chrome).unwrap();
        let resp = client
            .post("https://httpbin.org/post", "hello")
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
        assert!(resp.text().contains("hello"));
    }

    #[tokio::test]
    #[ignore] // requires network
    async fn get_follow_redirects() {
        let client = HttpClient::new(hpx::BrowserProfile::Chrome).unwrap();
        let resp = client
            .get_follow("https://httpbin.org/redirect/2", 5)
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
    }
}
