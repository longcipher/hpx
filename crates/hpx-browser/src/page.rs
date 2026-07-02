//! Browser page abstraction with challenge-aware navigation.

use std::time::{Duration, Instant};

#[cfg(feature = "v8")]
use crate::js_runtime::runtime::BrowserJsRuntime;
use crate::{
    challenge::{ChallengeVerdict, EngineClass, engine_classify},
    dom::Dom,
    host::EngineHandle,
    net::{HttpClient, RedirectPolicy},
    stealth::StealthProfile,
};

/// Default navigation budget.
const DEFAULT_NAV_BUDGET: Duration = Duration::from_secs(15);
/// Default max iterations for challenge retry loops.
const DEFAULT_MAX_ITERATIONS: u8 = 3;

/// A browser page/tab.
pub struct Page {
    engine: EngineHandle,
    dom: Dom,
    url: String,
    title: String,
    html: String,
    challenge_class: EngineClass,
    profile: Option<StealthProfile>,
    stealth: bool,
}

impl std::fmt::Debug for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Page")
            .field("url", &self.url)
            .field("title", &self.title)
            .field("stealth", &self.stealth)
            .field("challenge_class", &self.challenge_class)
            .field("profile", &self.profile.is_some())
            .finish()
    }
}

impl Page {
    pub fn new(engine: EngineHandle) -> Self {
        Self {
            engine,
            dom: Dom::new(),
            url: "about:blank".to_string(),
            title: String::new(),
            html: String::new(),
            challenge_class: EngineClass {
                tag: "L3-RENDERED",
                verdict: ChallengeVerdict::Pass,
                len: 0,
            },
            profile: None,
            stealth: false,
        }
    }

    /// Create a page from raw HTML (no network).
    pub async fn from_html(html: &str, stealth: bool) -> Result<Self, PageError> {
        let dom = crate::html_parser::parse_html(html);
        let title = extract_title(html);
        let challenge_class = engine_classify(html);
        Ok(Self {
            engine: EngineHandle::new(),
            dom,
            url: "about:blank".to_string(),
            title,
            html: html.to_string(),
            challenge_class,
            profile: None,
            stealth,
        })
    }

    /// Create a page with profile and URL (no network).
    pub async fn with_profile(
        html: &str,
        url: &str,
        _profile: StealthProfile,
    ) -> Result<Self, PageError> {
        let dom = crate::html_parser::parse_html(html);
        let title = extract_title(html);
        let challenge_class = engine_classify(html);
        Ok(Self {
            engine: EngineHandle::new(),
            dom,
            url: url.to_string(),
            title,
            html: html.to_string(),
            challenge_class,
            profile: None,
            stealth: true,
        })
    }

    /// Reload the page with new HTML (reuses V8 isolate in v8 mode).
    pub fn reload_html(&mut self, html: &str, url: &str) {
        self.dom = crate::html_parser::parse_html(html);
        self.url = url.to_string();
        self.html = html.to_string();
        self.title = extract_title(html);
        self.challenge_class = engine_classify(html);
    }

    /// Navigate to a URL with challenge-aware retry loop.
    ///
    /// Fetch → classify → if challenge detected, retry up to `max_iterations`.
    /// Uses 15s budget by default.
    pub async fn navigate(&mut self, url: &str) -> Result<(), PageError> {
        // ponytail: always Chrome profile; per-profile routing via tls_impersonate
        let client = HttpClient::new(hpx::BrowserProfile::Chrome).map_err(PageError::Net)?;
        self.navigate_inner(url, &client, DEFAULT_MAX_ITERATIONS, DEFAULT_NAV_BUDGET)
            .await
    }

    /// Navigate with a custom solver list.
    ///
    /// Same as `navigate()` but accepts external challenge solvers.
    pub async fn navigate_with_solvers(
        &mut self,
        url: &str,
        solvers: &[&dyn crate::challenge::ChallengeSolver],
    ) -> Result<(), PageError> {
        let client = HttpClient::new(hpx::BrowserProfile::Chrome).map_err(PageError::Net)?;
        self.navigate_with_solvers_inner(
            url,
            &client,
            solvers,
            DEFAULT_MAX_ITERATIONS,
            DEFAULT_NAV_BUDGET,
        )
        .await
    }

    /// Warm navigation — reuse existing page state, fetch new URL.
    ///
    /// Faster than cold `navigate()` because it skips profile setup.
    pub async fn navigate_warm(&mut self, url: &str) -> Result<(), PageError> {
        let client = HttpClient::new(hpx::BrowserProfile::Chrome).map_err(PageError::Net)?;
        let resp = client
            .request("GET", url, None, &[], RedirectPolicy::Follow(10))
            .await
            .map_err(PageError::Net)?;
        let html = resp.text();
        let resp_url = resp.url.clone();

        self.reload_html(&html, &resp_url);
        Ok(())
    }

    /// Core navigate loop with budget and cookie-diff retry.
    async fn navigate_inner(
        &mut self,
        url: &str,
        client: &HttpClient,
        max_iterations: u8,
        budget: Duration,
    ) -> Result<(), PageError> {
        self.navigate_with_solvers_inner(url, client, &[], max_iterations, budget)
            .await
    }

    /// Core navigate loop with solver support.
    async fn navigate_with_solvers_inner(
        &mut self,
        url: &str,
        client: &HttpClient,
        solvers: &[&dyn crate::challenge::ChallengeSolver],
        max_iterations: u8,
        budget: Duration,
    ) -> Result<(), PageError> {
        let t0 = Instant::now();
        let iterations = max_iterations.max(1);

        let resp = client
            .request("GET", url, None, &[], RedirectPolicy::Follow(10))
            .await
            .map_err(PageError::Net)?;
        let mut current_html = resp.text();
        let mut current_url = resp.url.clone();
        let mut cookies_before = cookie_snapshot(client, &current_url).await;

        for iter in 0..iterations {
            if t0.elapsed() >= budget {
                tracing::warn!(
                    iter,
                    elapsed_ms = t0.elapsed().as_millis(),
                    "navigate budget exhausted"
                );
                break;
            }

            self.reload_html(&current_html, &current_url);

            let challenge = engine_classify(&current_html);

            // Clean page — no challenge markers, return immediately.
            if !challenge.verdict.is_challenge() {
                return Ok(());
            }

            // Try registered solvers.
            let kind = tag_to_kind(challenge.tag);
            let mut any_solved = false;
            for solver in solvers {
                if !solver.can_handle(&kind) {
                    continue;
                }
                if matches!(
                    solver.solve(&kind, self).await,
                    crate::challenge::SolveOutcome::Solved
                ) {
                    any_solved = true;
                }
            }

            if any_solved {
                // Re-fetch after solver ran.
                let resp = client
                    .request("GET", &current_url, None, &[], RedirectPolicy::Follow(10))
                    .await
                    .map_err(PageError::Net)?;
                current_html = resp.text();
                current_url = resp.url.clone();
                cookies_before = cookie_snapshot(client, &current_url).await;
                continue;
            }

            // Cookie-diff retry: if cookies changed during this iteration,
            // the challenge script may have self-solved.
            if iter + 1 < iterations {
                let cookies_after = cookie_snapshot(client, &current_url).await;
                if cookies_after != cookies_before && !cookies_after.is_empty() {
                    tracing::info!(iter, "cookie delta detected — retrying navigation");
                    let resp = client
                        .request("GET", &current_url, None, &[], RedirectPolicy::Follow(10))
                        .await
                        .map_err(PageError::Net)?;
                    current_html = resp.text();
                    current_url = resp.url.clone();
                    cookies_before = cookie_snapshot(client, &current_url).await;
                    continue;
                }
            }

            // Challenge still present, no solver helped, no cookie change.
            break;
        }

        Ok(())
    }

    pub async fn evaluate_async(&mut self, _script: &str) -> Result<serde_json::Value, PageError> {
        Err(PageError::Evaluation(
            "evaluate_async requires v8 feature".into(),
        ))
    }

    /// Synchronous evaluate — DOM-level only without v8.
    pub fn evaluate(&mut self, _script: &str) -> Result<String, PageError> {
        Ok("undefined".to_string())
    }

    pub async fn title_async(&self) -> Result<String, PageError> {
        Ok(self.title.clone())
    }

    /// Synchronous title.
    pub fn title(&self) -> String {
        self.title.clone()
    }

    /// Current URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Whether stealth globals are enabled for this page.
    pub fn stealth(&self) -> bool {
        self.stealth
    }

    /// Apply a stealth profile's fields as JS globals and run page init.
    ///
    /// This sets `navigator.userAgent`, `navigator.platform`, screen
    /// dimensions, GPU info, and other fingerprint globals from the
    /// profile, then calls `__hpx_init()` to wire them into the
    /// JavaScript environment.
    #[cfg(feature = "v8")]
    pub fn set_profile(&mut self, profile: StealthProfile) {
        let mut rt = BrowserJsRuntime::new(crate::dom::Dom::new());
        rt.set_user_agent(&profile.user_agent);
        rt.set_platform(&profile.platform, &profile.os_name, &profile.os_version);
        rt.set_stealth(true);
        rt.run_page_init();
        self.profile = Some(profile);
    }

    /// Page HTML content.
    pub fn content(&self) -> String {
        self.html.clone()
    }

    pub async fn text_content(&self) -> Result<String, PageError> {
        Ok(self.dom.text_content(crate::dom::NodeId::DOCUMENT))
    }

    pub async fn text_of(&self, _selector: &str) -> Result<String, PageError> {
        Ok(String::new())
    }

    /// Synchronous element check.
    pub fn has_element(&self, _selector: &str) -> bool {
        false
    }

    /// Challenge classification result.
    pub fn challenge_verdict(&self) -> ChallengeVerdict {
        self.challenge_class.verdict
    }

    /// Full challenge classification.
    pub fn engine_class(&self) -> &EngineClass {
        &self.challenge_class
    }

    pub fn dom(&self) -> &Dom {
        &self.dom
    }
}

/// Map an `engine_classify` tag to a `ChallengeKind` for solver dispatch.
fn tag_to_kind(tag: &'static str) -> crate::challenge::ChallengeKind {
    let (vendor, sub_kind): (&'static str, &'static str) = if tag.starts_with("cf-") {
        ("cloudflare", tag)
    } else if tag.starts_with("AWS-WAF") {
        ("aws-waf", tag)
    } else if tag.eq_ignore_ascii_case("datadome") {
        ("datadome", tag)
    } else if tag.starts_with("akamai") {
        ("akamai", tag)
    } else if tag.starts_with("px-") || tag.starts_with("PXC") {
        ("perimeterx", tag)
    } else if tag.starts_with("kasada") {
        ("kasada", tag)
    } else if tag.starts_with("sec-cpt") {
        ("sec-cpt", tag)
    } else if tag.starts_with("hcaptcha") {
        ("hcaptcha", tag)
    } else {
        ("unknown", tag)
    };
    crate::challenge::ChallengeKind::new(vendor, sub_kind)
}

/// Snapshot cookie jar for a URL (empty string if none).
async fn cookie_snapshot(client: &HttpClient, url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        client.cookies_for_url(&parsed).await.unwrap_or_default()
    } else {
        String::new()
    }
}

/// Extract <title> from HTML (cheap string scan, no full parse).
fn extract_title(html: &str) -> String {
    let lower = html.to_lowercase();
    if let Some(start) = lower.find("<title") {
        let after_tag = &html[start..];
        if let Some(gt) = after_tag.find('>') {
            let content = &after_tag[gt + 1..];
            if let Some(end) = content.to_lowercase().find("</title>") {
                return content[..end].trim().to_string();
            }
        }
    }
    String::new()
}

#[derive(Debug, thiserror::Error)]
pub enum PageError {
    #[error("navigation failed: {0}")]
    Navigation(String),
    #[error("evaluation failed: {0}")]
    Evaluation(String),
    #[error("element not found")]
    ElementNotFound,
    #[error("page not loaded")]
    NotLoaded,
    #[error("network error: {0}")]
    Net(#[from] crate::net::NetError),
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── BDD Scenario 1: Navigate to clean page ──────────────────────────

    #[tokio::test]
    async fn bdd_navigate_to_clean_page() {
        let mut body = String::from("Hello World. ");
        // Push past THIN_BODY_MAX_BYTES (1000) and THIN_SHELL_MAX_BYTES (15KB)
        for _ in 0..500 {
            body.push_str("This is real rendered content for the test page. ");
        }
        let html = format!(
            r#"<!DOCTYPE html>
<html>
<head><title>Test Page</title></head>
<body>{body}</body>
</html>"#
        );
        let page = Page::from_html(&html, false).await.unwrap();

        assert_eq!(page.title(), "Test Page");
        assert!(page.content().contains("Hello World"));
        assert_eq!(page.challenge_verdict(), ChallengeVerdict::Pass);
    }

    // ── BDD Scenario 2: Navigate with challenge detection ────────────────

    #[tokio::test]
    async fn bdd_navigate_with_challenge_detection() {
        // Simulate a Cloudflare challenge response
        let html = r#"<!DOCTYPE html>
<html>
<head><title>Just a moment...</title></head>
<body>
<script>window._cf_chl_opt={cvId:'3',cType:'managed'};</script>
Checking your browser before accessing the site...
</body>
</html>"#;
        let page = Page::from_html(html, false).await.unwrap();

        assert_eq!(page.challenge_verdict(), ChallengeVerdict::EdgeBlock);
        assert!(page.challenge_verdict().is_challenge());
    }

    // ── BDD Scenario 3: ChallengeIncomplete for large managed shell ──────

    #[tokio::test]
    async fn bdd_challenge_incomplete_verdict() {
        let mut html = String::from(
            r#"<html><head><title>Just a moment...</title></head><body>
            <script>window._cf_chl_opt={cvId:'3',cType:'managed'};</script>"#,
        );
        for _ in 0..2000 {
            html.push_str("<div>cf challenge orchestrator shell padding</div>");
        }
        html.push_str("</body></html>");
        assert!(html.len() >= 50_000);

        let page = Page::from_html(&html, false).await.unwrap();
        assert_eq!(
            page.challenge_verdict(),
            ChallengeVerdict::ChallengeIncomplete
        );
        assert!(page.challenge_verdict().is_challenge());
    }

    // ── BDD: Clean page with substantial content passes ──────────────────

    #[tokio::test]
    async fn bdd_clean_page_passes() {
        let mut html = String::from("<html><body>");
        for _ in 0..400 {
            html.push_str("<p>Normal rendered content paragraph with enough text.</p>");
        }
        html.push_str("</body></html>");
        assert!(html.len() >= 15_000);

        let page = Page::from_html(&html, false).await.unwrap();
        assert_eq!(page.challenge_verdict(), ChallengeVerdict::Pass);
        assert!(!page.challenge_verdict().is_challenge());
    }

    // ── BDD: Warm reuse reloads HTML ─────────────────────────────────────

    #[tokio::test]
    async fn bdd_warm_reuse_reloads_html() {
        let html1 =
            r#"<!DOCTYPE html><html><head><title>First</title></head><body>Page One</body></html>"#;
        let html2 = r#"<!DOCTYPE html><html><head><title>Second</title></head><body>Page Two</body></html>"#;

        let mut page = Page::from_html(html1, false).await.unwrap();
        assert_eq!(page.title(), "First");
        assert!(page.content().contains("Page One"));

        // Warm reuse: reload with new HTML
        page.reload_html(html2, "https://example.com/second");
        assert_eq!(page.title(), "Second");
        assert!(page.content().contains("Page Two"));
        assert_eq!(page.url(), "https://example.com/second");
    }

    // ── BDD: Thin body is RenderIncomplete ───────────────────────────────

    #[tokio::test]
    async fn bdd_thin_body_render_incomplete() {
        let html = "<html><body>tiny</body></html>";
        let page = Page::from_html(html, false).await.unwrap();
        assert_eq!(page.challenge_verdict(), ChallengeVerdict::RenderIncomplete);
        assert!(!page.challenge_verdict().is_challenge());
    }

    // ── BDD: DataDome interstitial detected ──────────────────────────────

    #[tokio::test]
    async fn bdd_datadome_interstitial() {
        let html = r#"<script src="https://geo.captcha-delivery.com/captcha.js"></script>
<div id="ddcaptchaencoded">encoded_payload</div>"#;
        let page = Page::from_html(html, false).await.unwrap();
        assert!(page.challenge_verdict().is_challenge());
    }

    // ── BDD: AWS-WAF challenge detected ──────────────────────────────────

    #[tokio::test]
    async fn bdd_awswaf_challenge() {
        let html = r#"<html><body>
<script>window.gokuProps={key:'a',context:'b',iv:'c'};</script>
<script>window.awsWafCookieDomainList=["example.com"];</script>
<script src="https://x.token.awswaf.com/challenge.js"></script>
<script>AwsWafIntegration.checkForceRefresh();</script>
</body></html>"#;
        let page = Page::from_html(html, false).await.unwrap();
        assert!(page.challenge_verdict().is_challenge());
    }

    // ── extract_title tests ──────────────────────────────────────────────

    #[test]
    fn extract_title_basic() {
        assert_eq!(
            extract_title("<html><head><title>Hello</title></head></html>"),
            "Hello"
        );
    }

    #[test]
    fn extract_title_empty() {
        assert_eq!(extract_title("<html><body></body></html>"), "");
    }

    #[test]
    fn extract_title_case_insensitive() {
        assert_eq!(
            extract_title("<HTML><HEAD><TITLE>Test</TITLE></HEAD></HTML>"),
            "Test"
        );
    }
}
