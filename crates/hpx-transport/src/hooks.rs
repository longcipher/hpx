//! Lifecycle hooks for request/response processing.
//!
//! This module provides a hooks system similar to Ky's hooks API, allowing users
//! to intercept and modify requests and responses at various stages of the HTTP lifecycle.
//!
//! # Example
//!
//! ```rust,ignore
//! use hpx_transport::hooks::{Hooks, BeforeRequestHook, AfterResponseHook};
//!
//! struct LoggingHook;
//!
//! #[async_trait]
//! impl BeforeRequestHook for LoggingHook {
//!     async fn on_request(&self, request: &mut Request) -> Result<(), HookError> {
//!         println!("Sending request to: {}", request.url);
//!         Ok(())
//!     }
//! }
//! ```

use std::{fmt, sync::Arc, time::Duration};

use async_trait::async_trait;

use crate::transport::{Request, Response};

/// Error type for hook operations.
#[derive(Debug, thiserror::Error)]
pub enum HookError {
    /// Hook execution failed with a message.
    #[error("Hook failed: {message}")]
    Failed { message: String },

    /// Hook was cancelled.
    #[error("Hook cancelled: {reason}")]
    Cancelled { reason: String },

    /// Hook timed out.
    #[error("Hook timed out after {duration:?}")]
    Timeout { duration: Duration },

    /// Custom error from hook implementation.
    #[error("Hook error: {0}")]
    Custom(Box<dyn std::error::Error + Send + Sync>),
}

impl HookError {
    /// Create a new failed hook error.
    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed {
            message: message.into(),
        }
    }

    /// Create a new cancelled hook error.
    pub fn cancelled(reason: impl Into<String>) -> Self {
        Self::Cancelled {
            reason: reason.into(),
        }
    }

    /// Create a custom hook error.
    pub fn custom<E: std::error::Error + Send + Sync + 'static>(error: E) -> Self {
        Self::Custom(Box::new(error))
    }
}

/// Hook executed before a request is sent.
#[async_trait]
pub trait BeforeRequestHook: Send + Sync {
    /// Called before a request is sent. Can modify the request.
    async fn on_request(&self, request: &mut Request) -> Result<(), HookError>;

    /// Get the hook name for debugging.
    fn name(&self) -> &'static str {
        "before_request"
    }
}

/// Hook executed after a response is received.
#[async_trait]
pub trait AfterResponseHook: Send + Sync {
    /// Called after a response is received. Can modify the response.
    async fn on_response(
        &self,
        request: &Request,
        response: &mut Response,
    ) -> Result<(), HookError>;

    /// Get the hook name for debugging.
    fn name(&self) -> &'static str {
        "after_response"
    }
}

/// Hook executed before a retry attempt.
#[async_trait]
pub trait BeforeRetryHook: Send + Sync {
    /// Called before a retry attempt. Return `false` to cancel the retry.
    async fn on_retry(
        &self,
        request: &Request,
        error: &crate::error::TransportError,
        attempt: usize,
        delay: Duration,
    ) -> Result<bool, HookError>;

    /// Get the hook name for debugging.
    fn name(&self) -> &'static str {
        "before_retry"
    }
}

/// Hook executed before a redirect is followed.
#[async_trait]
pub trait BeforeRedirectHook: Send + Sync {
    /// Called before a redirect is followed. Return `false` to stop redirecting.
    async fn on_redirect(
        &self,
        request: &Request,
        response: &Response,
        redirect_url: &str,
    ) -> Result<bool, HookError>;

    /// Get the hook name for debugging.
    fn name(&self) -> &'static str {
        "before_redirect"
    }
}

/// Hook executed when an error occurs.
#[async_trait]
pub trait OnErrorHook: Send + Sync {
    /// Called when an error occurs during request processing.
    async fn on_error(
        &self,
        request: &Request,
        error: &crate::error::TransportError,
    ) -> Result<(), HookError>;

    /// Get the hook name for debugging.
    fn name(&self) -> &'static str {
        "on_error"
    }
}

/// Collection of hooks for the HTTP client lifecycle.
#[derive(Default)]
pub struct Hooks {
    /// Hooks executed before each request.
    pub before_request: Vec<Arc<dyn BeforeRequestHook>>,
    /// Hooks executed after each response.
    pub after_response: Vec<Arc<dyn AfterResponseHook>>,
    /// Hooks executed before each retry attempt.
    pub before_retry: Vec<Arc<dyn BeforeRetryHook>>,
    /// Hooks executed before following a redirect.
    pub before_redirect: Vec<Arc<dyn BeforeRedirectHook>>,
    /// Hooks executed when an error occurs.
    pub on_error: Vec<Arc<dyn OnErrorHook>>,
}

impl fmt::Debug for Hooks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hooks")
            .field("before_request_count", &self.before_request.len())
            .field("after_response_count", &self.after_response.len())
            .field("before_retry_count", &self.before_retry.len())
            .field("before_redirect_count", &self.before_redirect.len())
            .field("on_error_count", &self.on_error.len())
            .finish()
    }
}

impl Clone for Hooks {
    fn clone(&self) -> Self {
        Self {
            before_request: self.before_request.clone(),
            after_response: self.after_response.clone(),
            before_retry: self.before_retry.clone(),
            before_redirect: self.before_redirect.clone(),
            on_error: self.on_error.clone(),
        }
    }
}

impl Hooks {
    /// Create a new empty hooks collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a before-request hook.
    pub fn before_request<H: BeforeRequestHook + 'static>(mut self, hook: H) -> Self {
        self.before_request.push(Arc::new(hook));
        self
    }

    /// Add an after-response hook.
    pub fn after_response<H: AfterResponseHook + 'static>(mut self, hook: H) -> Self {
        self.after_response.push(Arc::new(hook));
        self
    }

    /// Add a before-retry hook.
    pub fn before_retry<H: BeforeRetryHook + 'static>(mut self, hook: H) -> Self {
        self.before_retry.push(Arc::new(hook));
        self
    }

    /// Add a before-redirect hook.
    pub fn before_redirect<H: BeforeRedirectHook + 'static>(mut self, hook: H) -> Self {
        self.before_redirect.push(Arc::new(hook));
        self
    }

    /// Add an on-error hook.
    pub fn on_error<H: OnErrorHook + 'static>(mut self, hook: H) -> Self {
        self.on_error.push(Arc::new(hook));
        self
    }

    /// Check if there are any hooks registered.
    pub fn is_empty(&self) -> bool {
        self.before_request.is_empty()
            && self.after_response.is_empty()
            && self.before_retry.is_empty()
            && self.before_redirect.is_empty()
            && self.on_error.is_empty()
    }

    /// Execute all before-request hooks.
    pub async fn run_before_request(&self, request: &mut Request) -> Result<(), HookError> {
        for hook in &self.before_request {
            tracing::trace!(hook = hook.name(), "Running before_request hook");
            hook.on_request(request).await?;
        }
        Ok(())
    }

    /// Execute all after-response hooks.
    pub async fn run_after_response(
        &self,
        request: &Request,
        response: &mut Response,
    ) -> Result<(), HookError> {
        for hook in &self.after_response {
            tracing::trace!(hook = hook.name(), "Running after_response hook");
            hook.on_response(request, response).await?;
        }
        Ok(())
    }

    /// Execute all before-retry hooks. Returns `true` if retry should proceed.
    pub async fn run_before_retry(
        &self,
        request: &Request,
        error: &crate::error::TransportError,
        attempt: usize,
        delay: Duration,
    ) -> Result<bool, HookError> {
        for hook in &self.before_retry {
            tracing::trace!(hook = hook.name(), attempt, "Running before_retry hook");
            if !hook.on_retry(request, error, attempt, delay).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Execute all before-redirect hooks. Returns `true` if redirect should proceed.
    pub async fn run_before_redirect(
        &self,
        request: &Request,
        response: &Response,
        redirect_url: &str,
    ) -> Result<bool, HookError> {
        for hook in &self.before_redirect {
            tracing::trace!(
                hook = hook.name(),
                redirect_url,
                "Running before_redirect hook"
            );
            if !hook.on_redirect(request, response, redirect_url).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Execute all on-error hooks.
    pub async fn run_on_error(
        &self,
        request: &Request,
        error: &crate::error::TransportError,
    ) -> Result<(), HookError> {
        for hook in &self.on_error {
            tracing::trace!(hook = hook.name(), %error, "Running on_error hook");
            hook.on_error(request, error).await?;
        }
        Ok(())
    }
}

// ============================================================================
// Convenient Hook Implementations
// ============================================================================

/// A hook that logs request information.
pub struct LoggingHook {
    level: tracing::Level,
}

impl LoggingHook {
    /// Create a new logging hook with the specified log level.
    pub fn new(level: tracing::Level) -> Self {
        Self { level }
    }

    /// Create a logging hook at INFO level.
    pub fn info() -> Self {
        Self::new(tracing::Level::INFO)
    }

    /// Create a logging hook at DEBUG level.
    pub fn debug() -> Self {
        Self::new(tracing::Level::DEBUG)
    }
}

impl Default for LoggingHook {
    fn default() -> Self {
        Self::info()
    }
}

#[async_trait]
impl BeforeRequestHook for LoggingHook {
    async fn on_request(&self, request: &mut Request) -> Result<(), HookError> {
        match self.level {
            tracing::Level::ERROR => {
                tracing::error!(method = %request.method, url = %request.url, "Sending request");
            }
            tracing::Level::WARN => {
                tracing::warn!(method = %request.method, url = %request.url, "Sending request");
            }
            tracing::Level::INFO => {
                tracing::info!(method = %request.method, url = %request.url, "Sending request");
            }
            tracing::Level::DEBUG => {
                tracing::debug!(method = %request.method, url = %request.url, headers = ?request.headers, "Sending request");
            }
            tracing::Level::TRACE => {
                tracing::trace!(method = %request.method, url = %request.url, headers = ?request.headers, body = ?request.body, "Sending request");
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "logging"
    }
}

#[async_trait]
impl AfterResponseHook for LoggingHook {
    async fn on_response(
        &self,
        request: &Request,
        response: &mut Response,
    ) -> Result<(), HookError> {
        match self.level {
            tracing::Level::ERROR => {
                tracing::error!(
                    method = %request.method,
                    url = %request.url,
                    status = response.status,
                    duration_ms = response.duration.as_millis(),
                    "Received response"
                );
            }
            tracing::Level::WARN => {
                tracing::warn!(
                    method = %request.method,
                    url = %request.url,
                    status = response.status,
                    duration_ms = response.duration.as_millis(),
                    "Received response"
                );
            }
            tracing::Level::INFO => {
                tracing::info!(
                    method = %request.method,
                    url = %request.url,
                    status = response.status,
                    duration_ms = response.duration.as_millis(),
                    "Received response"
                );
            }
            tracing::Level::DEBUG => {
                tracing::debug!(
                    method = %request.method,
                    url = %request.url,
                    status = response.status,
                    duration_ms = response.duration.as_millis(),
                    headers = ?response.headers,
                    "Received response"
                );
            }
            tracing::Level::TRACE => {
                tracing::trace!(
                    method = %request.method,
                    url = %request.url,
                    status = response.status,
                    duration_ms = response.duration.as_millis(),
                    headers = ?response.headers,
                    body_len = response.body.len(),
                    "Received response"
                );
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "logging"
    }
}

/// A hook that injects headers into requests.
pub struct HeaderInjectionHook {
    headers: Vec<(String, String)>,
}

impl HeaderInjectionHook {
    /// Create a new header injection hook.
    pub fn new() -> Self {
        Self {
            headers: Vec::new(),
        }
    }

    /// Add a header to inject.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Add multiple headers to inject.
    pub fn headers<I, K, V>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (name, value) in headers {
            self.headers.push((name.into(), value.into()));
        }
        self
    }
}

impl Default for HeaderInjectionHook {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BeforeRequestHook for HeaderInjectionHook {
    async fn on_request(&self, request: &mut Request) -> Result<(), HookError> {
        for (name, value) in &self.headers {
            request.headers.insert(name.clone(), value.clone());
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "header_injection"
    }
}

/// A hook that adds a unique request ID to each request.
pub struct RequestIdHook {
    header_name: String,
}

impl RequestIdHook {
    /// Create a new request ID hook with the default header name "X-Request-ID".
    pub fn new() -> Self {
        Self {
            header_name: "X-Request-ID".to_string(),
        }
    }

    /// Create a request ID hook with a custom header name.
    pub fn with_header_name(header_name: impl Into<String>) -> Self {
        Self {
            header_name: header_name.into(),
        }
    }
}

impl Default for RequestIdHook {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BeforeRequestHook for RequestIdHook {
    async fn on_request(&self, request: &mut Request) -> Result<(), HookError> {
        request
            .headers
            .insert(self.header_name.clone(), request.id.to_string());
        Ok(())
    }

    fn name(&self) -> &'static str {
        "request_id"
    }
}

/// A hook for signing requests (placeholder for custom signing logic).
pub struct SignRequestHook<F>
where
    F: Fn(&mut Request) -> Result<(), HookError> + Send + Sync,
{
    sign_fn: F,
}

impl<F> SignRequestHook<F>
where
    F: Fn(&mut Request) -> Result<(), HookError> + Send + Sync,
{
    /// Create a new sign request hook with the given signing function.
    pub fn new(sign_fn: F) -> Self {
        Self { sign_fn }
    }
}

#[async_trait]
impl<F> BeforeRequestHook for SignRequestHook<F>
where
    F: Fn(&mut Request) -> Result<(), HookError> + Send + Sync,
{
    async fn on_request(&self, request: &mut Request) -> Result<(), HookError> {
        (self.sign_fn)(request)
    }

    fn name(&self) -> &'static str {
        "sign_request"
    }
}

/// A closure-based before-request hook for convenience.
pub struct FnBeforeRequestHook<F>
where
    F: Fn(&mut Request) -> Result<(), HookError> + Send + Sync,
{
    f: F,
    name: &'static str,
}

impl<F> FnBeforeRequestHook<F>
where
    F: Fn(&mut Request) -> Result<(), HookError> + Send + Sync,
{
    /// Create a new function-based hook.
    pub fn new(name: &'static str, f: F) -> Self {
        Self { f, name }
    }
}

#[async_trait]
impl<F> BeforeRequestHook for FnBeforeRequestHook<F>
where
    F: Fn(&mut Request) -> Result<(), HookError> + Send + Sync,
{
    async fn on_request(&self, request: &mut Request) -> Result<(), HookError> {
        (self.f)(request)
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

/// A closure-based after-response hook for convenience.
pub struct FnAfterResponseHook<F>
where
    F: Fn(&Request, &mut Response) -> Result<(), HookError> + Send + Sync,
{
    f: F,
    name: &'static str,
}

impl<F> FnAfterResponseHook<F>
where
    F: Fn(&Request, &mut Response) -> Result<(), HookError> + Send + Sync,
{
    /// Create a new function-based hook.
    pub fn new(name: &'static str, f: F) -> Self {
        Self { f, name }
    }
}

#[async_trait]
impl<F> AfterResponseHook for FnAfterResponseHook<F>
where
    F: Fn(&Request, &mut Response) -> Result<(), HookError> + Send + Sync,
{
    async fn on_response(
        &self,
        request: &Request,
        response: &mut Response,
    ) -> Result<(), HookError> {
        (self.f)(request, response)
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_hooks_builder() {
        let hooks = Hooks::new()
            .before_request(LoggingHook::info())
            .after_response(LoggingHook::info())
            .before_request(RequestIdHook::new());

        assert_eq!(hooks.before_request.len(), 2);
        assert_eq!(hooks.after_response.len(), 1);
        assert!(!hooks.is_empty());
    }

    #[tokio::test]
    async fn test_header_injection_hook() {
        let hook = HeaderInjectionHook::new()
            .header("X-Custom", "value")
            .header("Authorization", "Bearer token");

        let mut request = Request::get("https://example.com/api");
        hook.on_request(&mut request).await.unwrap();

        assert_eq!(request.headers.get("X-Custom"), Some(&"value".to_string()));
        assert_eq!(
            request.headers.get("Authorization"),
            Some(&"Bearer token".to_string())
        );
    }

    #[tokio::test]
    async fn test_request_id_hook() {
        let hook = RequestIdHook::new();
        let mut request = Request::get("https://example.com/api");
        let request_id = request.id.to_string();

        hook.on_request(&mut request).await.unwrap();

        assert_eq!(request.headers.get("X-Request-ID"), Some(&request_id));
    }

    #[tokio::test]
    async fn test_run_before_request_hooks() {
        let hooks =
            Hooks::new().before_request(HeaderInjectionHook::new().header("X-Test", "test-value"));

        let mut request = Request::get("https://example.com/api");
        hooks.run_before_request(&mut request).await.unwrap();

        assert_eq!(
            request.headers.get("X-Test"),
            Some(&"test-value".to_string())
        );
    }
}
