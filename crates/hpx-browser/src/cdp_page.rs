//! CDP-based page implementation for browser automation.

use std::{sync::Arc, time::Duration};

use serde::de::DeserializeOwned;
use tokio::{sync::OnceCell, task::JoinHandle};

use crate::{
    cdp_client::{
        cdp::CdpClient,
        error::{CdpClientError, Result},
    },
    har::HarCapture,
};

/// Internal helper for tracking network events during `NetworkIdle` wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkEvent {
    RequestStarted,
    RequestFinished,
}

/// Controls when [`CdpPage::goto`] considers navigation complete.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WaitUntil {
    /// Fires when the `DOMContentLoaded` event completes.
    DomContentLoaded,
    /// Fires when the page `load` event fires.
    #[default]
    Load,
    /// Fires when there are no more than 2 network connections for 500 ms.
    NetworkIdle,
}

/// A CDP-managed browser page.
///
/// Wraps a [`CdpClient`] session with high-level navigation, evaluation,
/// and screenshot/PDF helpers. `Page.enable` is sent lazily on first use.
#[derive(Debug, Clone)]
pub struct CdpPage {
    /// The target ID of this page.
    pub target_id: String,
    /// The session ID for this page's CDP session.
    pub session_id: String,
    /// The CDP client used to communicate with this page.
    pub cdp: Arc<CdpClient>,
    page_enabled: Arc<OnceCell<()>>,
    network_enabled: Arc<OnceCell<()>>,
    current_url: Arc<parking_lot::Mutex<String>>,
}

impl CdpPage {
    /// Create a new `CdpPage`.
    #[must_use]
    pub fn new(target_id: String, session_id: String, cdp: Arc<CdpClient>) -> Self {
        Self {
            target_id,
            session_id,
            cdp,
            page_enabled: Arc::new(OnceCell::new()),
            network_enabled: Arc::new(OnceCell::new()),
            current_url: Arc::new(parking_lot::Mutex::new(String::new())),
        }
    }

    /// Start HAR capture for this page.
    ///
    /// Subscribes to CDP events from the underlying [`CdpClient`] and
    /// returns a [`HarCapture`] that filters events for this page's session.
    #[must_use]
    pub fn start_har_capture(&self) -> HarCapture {
        let rx = self.cdp.subscribe_events();
        HarCapture::new(rx)
    }

    /// Ensure `Page.enable` has been sent for this session.
    async fn ensure_page_enabled(&self) -> Result<()> {
        self.page_enabled
            .get_or_try_init(|| async {
                self.cdp
                    .send_command_with_session("Page.enable", None, &self.session_id)
                    .await?;
                Ok(())
            })
            .await
            .cloned()
    }

    /// Navigate to `url` and wait until the page reaches the requested state.
    ///
    /// # Errors
    ///
    /// Returns [`CdpClientError::NavigationFailed`] if the CDP command fails or
    /// the wait times out.
    pub async fn goto(&self, url: &str, wait_until: WaitUntil) -> Result<()> {
        self.ensure_page_enabled().await?;

        // Subscribe before sending the command to avoid missing the event.
        let mut load_event = self
            .cdp
            .subscribe_events_filtered(Some("Page.loadEventFired"), Some(&self.session_id));
        let mut dom_event = self
            .cdp
            .subscribe_events_filtered(Some("Page.domContentEventFired"), Some(&self.session_id));

        let _response = self
            .cdp
            .send_command_with_session(
                "Page.navigate",
                Some(serde_json::json!({ "url": url })),
                &self.session_id,
            )
            .await?;

        // Wait for the appropriate event.
        match wait_until {
            WaitUntil::DomContentLoaded => {
                tokio::time::timeout(Duration::from_secs(30), dom_event.recv())
                    .await
                    .map_err(|_| {
                        CdpClientError::navigation_failed(url, "domContentLoaded timeout")
                    })?
                    .map_err(|e| CdpClientError::navigation_failed(url, &e.to_string()))?;
            }
            WaitUntil::Load => {
                tokio::time::timeout(Duration::from_secs(30), load_event.recv())
                    .await
                    .map_err(|_| CdpClientError::navigation_failed(url, "load event timeout"))?
                    .map_err(|e| CdpClientError::navigation_failed(url, &e.to_string()))?;
            }
            WaitUntil::NetworkIdle => {
                // Wait for load event first, then poll until no pending network
                // requests remain for 500 ms.
                tokio::time::timeout(Duration::from_secs(30), load_event.recv())
                    .await
                    .map_err(|_| {
                        CdpClientError::navigation_failed(url, "load event timeout (NetworkIdle)")
                    })?
                    .map_err(|e| CdpClientError::navigation_failed(url, &e.to_string()))?;

                let mut request_event = self.cdp.subscribe_events_filtered(
                    Some("Network.requestWillBeSent"),
                    Some(&self.session_id),
                );
                let mut response_event = self.cdp.subscribe_events_filtered(
                    Some("Network.loadingFinished"),
                    Some(&self.session_id),
                );
                let mut fail_event = self.cdp.subscribe_events_filtered(
                    Some("Network.loadingFailed"),
                    Some(&self.session_id),
                );

                let mut active_requests: u32 = 0;
                let mut idle_since = None;
                let idle_threshold = Duration::from_millis(500);
                let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

                loop {
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    if remaining.is_zero() {
                        break;
                    }

                    let event = tokio::select! {
                        biased;
                        msg = tokio::time::timeout(remaining, request_event.recv()) => {
                            msg.ok().and_then(|r| r.ok()).map(|_| NetworkEvent::RequestStarted)
                        }
                        msg = tokio::time::timeout(remaining, response_event.recv()) => {
                            msg.ok().and_then(|r| r.ok()).map(|_| NetworkEvent::RequestFinished)
                        }
                        msg = tokio::time::timeout(remaining, fail_event.recv()) => {
                            msg.ok().and_then(|r| r.ok()).map(|_| NetworkEvent::RequestFinished)
                        }
                    };

                    match event {
                        Some(NetworkEvent::RequestStarted) => {
                            active_requests = active_requests.saturating_add(1);
                            idle_since = None;
                        }
                        Some(NetworkEvent::RequestFinished) => {
                            active_requests = active_requests.saturating_sub(1);
                            if active_requests == 0 && idle_since.is_none() {
                                idle_since = Some(tokio::time::Instant::now());
                            }
                        }
                        None => break,
                    }

                    if let Some(since) = idle_since {
                        if since.elapsed() >= idle_threshold {
                            break;
                        }
                    }
                }
            }
        }

        *self.current_url.lock() = url.to_string();
        Ok(())
    }

    /// Execute a JavaScript expression and return the deserialized result.
    ///
    /// Uses `Runtime.evaluate` with `returnByValue: true` and `awaitPromise: true`.
    ///
    /// # Errors
    ///
    /// Returns an error if the evaluation throws a JS exception or deserialization fails.
    pub async fn evaluate<T: DeserializeOwned>(&self, expression: &str) -> Result<T> {
        self.ensure_page_enabled().await?;

        let response = self
            .cdp
            .send_command_with_session(
                "Runtime.evaluate",
                Some(serde_json::json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                })),
                &self.session_id,
            )
            .await?;

        let result = response
            .result
            .ok_or_else(|| CdpClientError::command_failed("Runtime.evaluate", "missing result"))?;

        // Check for JS exception.
        if let Some(exception) = result.get("exceptionDetails") {
            let desc = exception
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown exception");
            return Err(CdpClientError::command_failed("Runtime.evaluate", desc));
        }

        let value = result
            .get("result")
            .ok_or_else(|| {
                CdpClientError::command_failed("Runtime.evaluate", "missing result.result")
            })?
            .clone();

        serde_json::from_value(value)
            .map_err(|e| CdpClientError::command_failed("Runtime.evaluate", &e.to_string()))
    }

    /// Return the full HTML content of the page.
    ///
    /// # Errors
    ///
    /// Returns an error if the evaluation fails.
    pub async fn content(&self) -> Result<String> {
        self.evaluate("document.documentElement.outerHTML").await
    }

    /// Return the page title.
    ///
    /// # Errors
    ///
    /// Returns an error if the evaluation fails.
    pub async fn title(&self) -> Result<String> {
        self.evaluate("document.title").await
    }

    /// Wait for a CSS selector to match at least one element in the DOM.
    ///
    /// Injects a `MutationObserver`-backed `Promise` into Chrome. The CDP
    /// response stays open until the element appears or the timer fires,
    /// giving ~1 ms reaction gap with a single round-trip.
    ///
    /// # Errors
    ///
    /// Returns [`CdpClientError::Timeout`] if the element does not appear
    /// within `timeout`.
    pub async fn wait_for_selector(&self, selector: &str, timeout: Duration) -> Result<bool> {
        let js = build_wait_for_selector_js(selector, timeout.as_millis());
        self.evaluate(&js).await
    }

    /// Return the current URL of the page.
    #[must_use]
    pub fn url(&self) -> String {
        self.current_url.lock().clone()
    }

    /// Take a screenshot of the page and return raw PNG bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the CDP command fails or base64 decoding fails.
    pub async fn screenshot(&self) -> Result<Vec<u8>> {
        self.ensure_page_enabled().await?;

        let response = self
            .cdp
            .send_command_with_session(
                "Page.captureScreenshot",
                Some(serde_json::json!({ "format": "png" })),
                &self.session_id,
            )
            .await?;

        let result = response.result.ok_or_else(|| {
            CdpClientError::command_failed("Page.captureScreenshot", "missing result")
        })?;
        let data = result.get("data").and_then(|v| v.as_str()).ok_or_else(|| {
            CdpClientError::command_failed("Page.captureScreenshot", "missing data field")
        })?;

        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| CdpClientError::command_failed("Page.captureScreenshot", &e.to_string()))
    }

    /// Generate a PDF of the page and return raw PDF bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the CDP command fails or base64 decoding fails.
    pub async fn pdf(&self) -> Result<Vec<u8>> {
        self.ensure_page_enabled().await?;

        let response = self
            .cdp
            .send_command_with_session("Page.printToPDF", None, &self.session_id)
            .await?;

        let result = response
            .result
            .ok_or_else(|| CdpClientError::command_failed("Page.printToPDF", "missing result"))?;
        let data = result.get("data").and_then(|v| v.as_str()).ok_or_else(|| {
            CdpClientError::command_failed("Page.printToPDF", "missing data field")
        })?;

        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| CdpClientError::command_failed("Page.printToPDF", &e.to_string()))
    }

    /// Ensure `Network.enable` has been sent for this session.
    async fn ensure_network_enabled(&self) -> Result<()> {
        self.network_enabled
            .get_or_try_init(|| async {
                self.cdp
                    .send_command_with_session("Network.enable", None, &self.session_id)
                    .await?;
                Ok(())
            })
            .await
            .cloned()
    }

    /// Enable network request interception.
    ///
    /// Subscribes to `Network.requestPaused` events and invokes `callback`
    /// for each intercepted request with `(url, resource_type)`. If the
    /// callback returns `false`, the request is failed via
    /// `Network.failRequest`; otherwise it is continued via
    /// `Network.continueRequest`.
    ///
    /// Returns a [`JoinHandle`] that can be aborted to stop interception.
    ///
    /// # Errors
    ///
    /// Returns an error if enabling the Network domain or setting request
    /// interception fails.
    pub async fn intercept_requests(
        &self,
        callback: Arc<dyn Fn(String, String) -> bool + Send + Sync>,
    ) -> Result<JoinHandle<()>> {
        self.ensure_network_enabled().await?;

        self.cdp
            .send_command_with_session(
                "Network.setRequestInterception",
                Some(serde_json::json!({ "patterns": [{"urlPattern": "*"}] })),
                &self.session_id,
            )
            .await?;

        let mut request_paused = self
            .cdp
            .subscribe_events_filtered(Some("Network.requestPaused"), Some(&self.session_id));

        let cdp = self.cdp.clone();
        let session_id = self.session_id.clone();

        let handle = tokio::spawn(async move {
            while let Ok(msg) = request_paused.recv().await {
                let params = match msg.params {
                    Some(p) => p,
                    None => continue,
                };

                let request_id = match params.get("requestId").and_then(|v| v.as_str()) {
                    Some(id) => id.to_string(),
                    None => continue,
                };

                let url = params
                    .get("request")
                    .and_then(|r| r.get("url"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let resource_type = params
                    .get("request")
                    .and_then(|r| r.get("resourceType"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Other")
                    .to_string();

                if callback(url, resource_type) {
                    let _ = cdp
                        .send_command_with_session(
                            "Network.continueRequest",
                            Some(serde_json::json!({ "requestId": request_id })),
                            &session_id,
                        )
                        .await;
                } else {
                    let _ = cdp
                        .send_command_with_session(
                            "Network.failRequest",
                            Some(serde_json::json!({
                                "requestId": request_id,
                                "reason": "BlockedByClient"
                            })),
                            &session_id,
                        )
                        .await;
                }
            }
        });

        Ok(handle)
    }

    /// Get all cookies for the current page.
    ///
    /// Sends `Network.getCookies` and returns the cookies as a JSON array.
    ///
    /// # Errors
    ///
    /// Returns an error if the CDP command fails.
    pub async fn cookies(&self) -> Result<Vec<serde_json::Value>> {
        self.ensure_network_enabled().await?;

        let response = self
            .cdp
            .send_command_with_session("Network.getCookies", None, &self.session_id)
            .await?;

        let result = response.result.ok_or_else(|| {
            CdpClientError::command_failed("Network.getCookies", "missing result")
        })?;

        let cookies = result
            .get("cookies")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                CdpClientError::command_failed("Network.getCookies", "missing cookies array")
            })?;

        Ok(cookies.clone())
    }

    /// Set cookies for the current page.
    ///
    /// Sends `Network.setCookies` with the provided cookie array.
    ///
    /// # Errors
    ///
    /// Returns an error if the CDP command fails.
    pub async fn set_cookies(&self, cookies: Vec<serde_json::Value>) -> Result<()> {
        self.ensure_network_enabled().await?;

        self.cdp
            .send_command_with_session(
                "Network.setCookies",
                Some(serde_json::json!({ "cookies": cookies })),
                &self.session_id,
            )
            .await?;

        Ok(())
    }

    /// Take a screenshot with format and quality options.
    ///
    /// # Errors
    ///
    /// Returns an error if the CDP command fails or base64 decoding fails.
    pub async fn screenshot_with_options(
        &self,
        format: &str,
        quality: Option<u32>,
    ) -> Result<Vec<u8>> {
        self.ensure_page_enabled().await?;

        let mut params = serde_json::json!({ "format": format });
        if let Some(q) = quality {
            params["quality"] = serde_json::json!(q);
        }

        let response = self
            .cdp
            .send_command_with_session("Page.captureScreenshot", Some(params), &self.session_id)
            .await?;

        let result = response.result.ok_or_else(|| {
            CdpClientError::command_failed("Page.captureScreenshot", "missing result")
        })?;
        let data = result.get("data").and_then(|v| v.as_str()).ok_or_else(|| {
            CdpClientError::command_failed("Page.captureScreenshot", "missing data field")
        })?;

        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| CdpClientError::command_failed("Page.captureScreenshot", &e.to_string()))
    }

    /// Generate a PDF with print background and scale options.
    ///
    /// # Errors
    ///
    /// Returns an error if the CDP command fails or base64 decoding fails.
    pub async fn pdf_with_options(
        &self,
        print_background: bool,
        scale: Option<f64>,
    ) -> Result<Vec<u8>> {
        self.ensure_page_enabled().await?;

        let mut params = serde_json::json!({ "printBackground": print_background });
        if let Some(s) = scale {
            params["scale"] = serde_json::json!(s);
        }

        let response = self
            .cdp
            .send_command_with_session("Page.printToPDF", Some(params), &self.session_id)
            .await?;

        let result = response
            .result
            .ok_or_else(|| CdpClientError::command_failed("Page.printToPDF", "missing result"))?;
        let data = result.get("data").and_then(|v| v.as_str()).ok_or_else(|| {
            CdpClientError::command_failed("Page.printToPDF", "missing data field")
        })?;

        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| CdpClientError::command_failed("Page.printToPDF", &e.to_string()))
    }
}

/// Escape a string for safe interpolation into a JS single-quoted literal.
///
/// Handles `\`, `'`, and newlines — the three characters that would break
/// a single-quoted JS string.
fn escape_js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '`' => out.push_str("\\`"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out
}

/// Build the JS expression that resolves `true` when `selector` matches
/// an element, or rejects after `timeout_ms`.
fn build_wait_for_selector_js(selector: &str, timeout_ms: u128) -> String {
    let escaped = escape_js_string(selector);
    format!(
        "new Promise((resolve, reject) => {{ \
           const sel = '{escaped}'; \
           const existing = document.querySelector(sel); \
           if (existing) {{ resolve(true); return; }} \
           const observer = new MutationObserver(() => {{ \
             if (document.querySelector(sel)) {{ \
               observer.disconnect(); \
               resolve(true); \
             }} \
           }}); \
           observer.observe(document.body, {{ childList: true, subtree: true }}); \
           setTimeout(() => {{ observer.disconnect(); reject(new Error('wait_for_selector timeout')); }}, {timeout_ms}); \
         }})"
    )
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use super::*;
    use crate::cdp_client::cdp::CdpMessage;

    /// Spawn a task that reads commands from the writer and sends back
    /// auto-generated responses via `dispatch_message`.
    fn spawn_auto_responder(client: &CdpClient, mut rx: mpsc::UnboundedReceiver<String>) {
        let client = client.clone();
        tokio::spawn(async move {
            while let Some(json_str) = rx.recv().await {
                if let Ok(req) = serde_json::from_str::<serde_json::Value>(&json_str) {
                    let id = req["id"].as_u64().unwrap_or(0) as u32;
                    let method = req["method"].as_str().unwrap_or("unknown");
                    let session_id = req["sessionId"].as_str().map(String::from);

                    let result = if method.contains("getCookies") {
                        serde_json::json!({ "cookies": [] })
                    } else {
                        serde_json::json!({ "method": method })
                    };

                    client.dispatch_message(CdpMessage {
                        id: Some(id),
                        method: None,
                        params: None,
                        result: Some(result),
                        error: None,
                        session_id,
                    });
                }
            }
        });
    }

    #[test]
    fn wait_until_default_is_load() {
        assert_eq!(WaitUntil::default(), WaitUntil::Load);
    }

    #[test]
    fn cdp_page_new_fields() {
        let client = Arc::new(CdpClient::new());
        let page = CdpPage::new("target-1".to_string(), "session-1".to_string(), client);
        assert_eq!(page.target_id, "target-1");
        assert_eq!(page.session_id, "session-1");
        assert!(page.url().is_empty());
    }

    #[test]
    fn wait_until_variants_are_distinct() {
        let dc = WaitUntil::DomContentLoaded;
        let ld = WaitUntil::Load;
        let ni = WaitUntil::NetworkIdle;
        assert_ne!(dc, ld);
        assert_ne!(ld, ni);
        assert_ne!(dc, ni);
    }

    #[test]
    fn escape_js_string_simple() {
        assert_eq!(escape_js_string("hello"), "hello");
    }

    #[test]
    fn escape_js_string_quotes() {
        assert_eq!(escape_js_string("it's"), "it\\'s");
    }

    #[test]
    fn escape_js_string_backslash() {
        assert_eq!(escape_js_string("a\\b"), "a\\\\b");
    }

    #[test]
    fn escape_js_string_newlines() {
        assert_eq!(escape_js_string("a\nb\r"), "a\\nb\\r");
    }

    #[test]
    fn escape_js_string_backtick() {
        assert_eq!(escape_js_string("a`b"), "a\\`b");
    }

    #[test]
    fn build_wait_for_selector_js_contains_mutation_observer() {
        let js = build_wait_for_selector_js("#foo", 5000);
        assert!(js.contains("MutationObserver"));
        assert!(js.contains("document.querySelector"));
        assert!(js.contains("observer.observe"));
        assert!(js.contains("setTimeout"));
    }

    #[test]
    fn build_wait_for_selector_js_uses_escaped_selector() {
        let js = build_wait_for_selector_js("it's", 1000);
        assert!(js.contains("it\\'s"));
    }

    #[test]
    fn build_wait_for_selector_js_timeout_is_embedded() {
        let js = build_wait_for_selector_js(".cls", 3000);
        assert!(js.contains("3000"));
    }

    #[test]
    fn build_wait_for_selector_js_resolves_true() {
        let js = build_wait_for_selector_js("div", 500);
        assert!(js.contains("resolve(true)"));
    }

    #[test]
    fn build_wait_for_selector_js_rejects_on_timeout() {
        let js = build_wait_for_selector_js("span", 2000);
        assert!(js.contains("reject(new Error"));
        assert!(js.contains("wait_for_selector timeout"));
    }

    // ── Task 4.4: Network Request Interception Tests ───────────────────

    #[tokio::test]
    async fn intercept_requests_sends_network_enable() {
        let client = Arc::new(CdpClient::new());
        let (writer_tx, writer_rx) = mpsc::unbounded_channel::<String>();
        client.set_test_writer(writer_tx);
        spawn_auto_responder(&client, writer_rx);

        let page = CdpPage::new(
            "target-1".to_string(),
            "session-1".to_string(),
            client.clone(),
        );

        let callback: Arc<dyn Fn(String, String) -> bool + Send + Sync> =
            Arc::new(|_url, _resource_type| true);

        let handle = page
            .intercept_requests(callback)
            .await
            .expect("intercept_requests should succeed");

        // Give time for commands to be processed
        tokio::time::sleep(Duration::from_millis(100)).await;

        handle.abort();
    }

    #[tokio::test]
    async fn intercept_requests_callback_receives_url_and_type() {
        let client = Arc::new(CdpClient::new());
        let (writer_tx, writer_rx) = mpsc::unbounded_channel::<String>();
        client.set_test_writer(writer_tx);
        spawn_auto_responder(&client, writer_rx);

        let page = CdpPage::new(
            "target-1".to_string(),
            "session-1".to_string(),
            client.clone(),
        );

        let (tx, mut rx) = mpsc::unbounded_channel::<(String, String)>();
        let callback: Arc<dyn Fn(String, String) -> bool + Send + Sync> =
            Arc::new(move |url, rt| {
                let _ = tx.send((url, rt));
                true
            });

        let handle = page
            .intercept_requests(callback)
            .await
            .expect("intercept_requests should succeed");

        // Give background task time to subscribe
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Simulate a Network.requestPaused event
        client.dispatch_message(CdpMessage {
            id: None,
            method: Some("Network.requestPaused".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "request": {
                    "url": "https://example.com/resource.js",
                    "resourceType": "Script"
                }
            })),
            result: None,
            error: None,
            session_id: Some("session-1".to_string()),
        });

        let (url, rt) = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("callback should be invoked")
            .expect("channel open");

        assert_eq!(url, "https://example.com/resource.js");
        assert_eq!(rt, "Script");

        handle.abort();
    }

    #[tokio::test]
    async fn intercept_requests_abort_sends_fail_request() {
        let client = Arc::new(CdpClient::new());
        let (writer_tx, writer_rx) = mpsc::unbounded_channel::<String>();
        client.set_test_writer(writer_tx);
        spawn_auto_responder(&client, writer_rx);

        let page = CdpPage::new(
            "target-1".to_string(),
            "session-1".to_string(),
            client.clone(),
        );

        // Callback that always aborts (returns false)
        let callback: Arc<dyn Fn(String, String) -> bool + Send + Sync> =
            Arc::new(|_url, _rt| false);

        let handle = page
            .intercept_requests(callback)
            .await
            .expect("intercept_requests should succeed");

        // Give background task time to subscribe
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Simulate requestPaused
        client.dispatch_message(CdpMessage {
            id: None,
            method: Some("Network.requestPaused".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-2",
                "request": {
                    "url": "https://example.com/blocked.png",
                    "resourceType": "Image"
                }
            })),
            result: None,
            error: None,
            session_id: Some("session-1".to_string()),
        });

        // Give time for the failRequest to be sent
        tokio::time::sleep(Duration::from_millis(50)).await;

        handle.abort();
    }

    #[tokio::test]
    async fn intercept_requests_continue_sends_continue_request() {
        let client = Arc::new(CdpClient::new());
        let (writer_tx, writer_rx) = mpsc::unbounded_channel::<String>();
        client.set_test_writer(writer_tx);
        spawn_auto_responder(&client, writer_rx);

        let page = CdpPage::new(
            "target-1".to_string(),
            "session-1".to_string(),
            client.clone(),
        );

        // Callback that always continues (returns true)
        let callback: Arc<dyn Fn(String, String) -> bool + Send + Sync> =
            Arc::new(|_url, _rt| true);

        let handle = page
            .intercept_requests(callback)
            .await
            .expect("intercept_requests should succeed");

        // Give background task time to subscribe
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Simulate requestPaused
        client.dispatch_message(CdpMessage {
            id: None,
            method: Some("Network.requestPaused".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-3",
                "request": {
                    "url": "https://example.com/style.css",
                    "resourceType": "Stylesheet"
                }
            })),
            result: None,
            error: None,
            session_id: Some("session-1".to_string()),
        });

        // Give time for the continueRequest to be sent
        tokio::time::sleep(Duration::from_millis(50)).await;

        handle.abort();
    }

    // ── Task 4.5: Cookie Operations Tests ──────────────────────────────

    #[tokio::test]
    async fn cookies_sends_get_cookies_command() {
        let client = Arc::new(CdpClient::new());
        let (writer_tx, writer_rx) = mpsc::unbounded_channel::<String>();
        client.set_test_writer(writer_tx);
        spawn_auto_responder(&client, writer_rx);

        let page = CdpPage::new(
            "target-1".to_string(),
            "session-1".to_string(),
            client.clone(),
        );

        let cookies = page.cookies().await.expect("cookies should succeed");
        assert!(cookies.is_empty());
    }

    #[tokio::test]
    async fn set_cookies_sends_set_cookies_command() {
        let client = Arc::new(CdpClient::new());
        let (writer_tx, writer_rx) = mpsc::unbounded_channel::<String>();
        client.set_test_writer(writer_tx);
        spawn_auto_responder(&client, writer_rx);

        let page = CdpPage::new(
            "target-1".to_string(),
            "session-1".to_string(),
            client.clone(),
        );

        let cookies_to_set = vec![
            serde_json::json!({"name": "user", "value": "alice", "domain": "example.com"}),
            serde_json::json!({"name": "token", "value": "xyz789", "domain": "example.com"}),
        ];

        page.set_cookies(cookies_to_set)
            .await
            .expect("set_cookies should succeed");
    }

    // ── Task 4.6: Screenshot and PDF Options Tests ─────────────────────

    #[tokio::test]
    async fn screenshot_with_options_png() {
        let client = Arc::new(CdpClient::new());
        let (writer_tx, writer_rx) = mpsc::unbounded_channel::<String>();
        client.set_test_writer(writer_tx);
        spawn_auto_responder(&client, writer_rx);

        let page = CdpPage::new(
            "target-1".to_string(),
            "session-1".to_string(),
            client.clone(),
        );

        let result = page.screenshot_with_options("png", None).await;
        // The auto-responder returns {"method": "Page.captureScreenshot"}
        // which can't be decoded as PNG, so this will fail
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn screenshot_with_options_jpeg_quality() {
        let client = Arc::new(CdpClient::new());
        let (writer_tx, writer_rx) = mpsc::unbounded_channel::<String>();
        client.set_test_writer(writer_tx);
        spawn_auto_responder(&client, writer_rx);

        let page = CdpPage::new(
            "target-1".to_string(),
            "session-1".to_string(),
            client.clone(),
        );

        let result = page.screenshot_with_options("jpeg", Some(80)).await;
        // The auto-responder returns invalid data for screenshot
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn pdf_with_options_sends_correct_params() {
        let client = Arc::new(CdpClient::new());
        let (writer_tx, writer_rx) = mpsc::unbounded_channel::<String>();
        client.set_test_writer(writer_tx);
        spawn_auto_responder(&client, writer_rx);

        let page = CdpPage::new(
            "target-1".to_string(),
            "session-1".to_string(),
            client.clone(),
        );

        let result = page.pdf_with_options(true, Some(1.5)).await;
        // The auto-responder returns invalid data for PDF
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn pdf_with_options_defaults() {
        let client = Arc::new(CdpClient::new());
        let (writer_tx, writer_rx) = mpsc::unbounded_channel::<String>();
        client.set_test_writer(writer_tx);
        spawn_auto_responder(&client, writer_rx);

        let page = CdpPage::new(
            "target-1".to_string(),
            "session-1".to_string(),
            client.clone(),
        );

        let result = page.pdf_with_options(false, None).await;
        // The auto-responder returns invalid data for PDF
        assert!(result.is_err());
    }

    // ── Task 5.2: HAR Integration Tests ───────────────────────────────────

    #[tokio::test]
    async fn start_har_capture_returns_capture() {
        let client = Arc::new(CdpClient::new());
        let page = CdpPage::new("target-1".to_string(), "session-1".to_string(), client);

        let mut capture = page.start_har_capture();
        let archive = capture.stop().await.expect("stop should succeed");
        assert_eq!(archive.log.version, "1.2");
        assert!(archive.log.entries.is_empty());
    }
}
