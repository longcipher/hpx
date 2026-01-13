//! Lifecycle hooks for HTTP requests and responses.
//!
//! This module provides a way to add custom hooks that execute at different
//! stages of the request lifecycle, similar to popular HTTP client libraries
//! like Ky (Node.js) or httpx (Python).
//!
//! # Example
//!
//! ```rust,no_run
//! use std::sync::Arc;
//!
//! use hpx::hooks::{AfterResponseHook, BeforeRequestHook, Hooks};
//!
//! // Define a simple logging hook
//! struct LoggingHook;
//!
//! impl BeforeRequestHook for LoggingHook {
//!     fn on_request(&self, request: &mut http::Request<hpx::Body>) -> Result<(), hpx::Error> {
//!         println!("Request: {} {}", request.method(), request.uri());
//!         Ok(())
//!     }
//! }
//!
//! impl AfterResponseHook for LoggingHook {
//!     fn on_response(
//!         &self,
//!         status: http::StatusCode,
//!         headers: &http::HeaderMap,
//!     ) -> Result<(), hpx::Error> {
//!         println!("Response: {}", status);
//!         Ok(())
//!     }
//! }
//!
//! # async fn doc() -> hpx::Result<()> {
//! let logging_hook = Arc::new(LoggingHook);
//! let hooks = Hooks::builder()
//!     .before_request(logging_hook.clone())
//!     .after_response(logging_hook)
//!     .build();
//!
//! let client = hpx::Client::builder().hooks(hooks).build()?;
//! # Ok(())
//! # }
//! ```

use std::{
    sync::Arc,
    task::{Context, Poll},
};

use futures_util::{FutureExt, future::BoxFuture};
use http::{HeaderMap, Request, Response, StatusCode};
use tower::{Layer, Service};

use crate::{Body, Error};

/// A hook that is called before a request is sent.
pub trait BeforeRequestHook: Send + Sync {
    /// Called before the request is sent.
    ///
    /// This hook can modify the request, add headers, etc.
    /// Return an error to abort the request.
    fn on_request(&self, request: &mut Request<Body>) -> Result<(), Error>;
}

/// A hook that is called after a response is received.
///
/// Note: This hook receives only status and headers to be dyn-compatible.
/// For full response access, use a Tower middleware instead.
pub trait AfterResponseHook: Send + Sync {
    /// Called after the response is received.
    ///
    /// This hook can inspect the response status and headers.
    /// Return an error to propagate an error to the caller.
    fn on_response(&self, status: StatusCode, headers: &HeaderMap) -> Result<(), Error>;
}

/// A hook that is called when an error occurs.
pub trait OnErrorHook: Send + Sync {
    /// Called when an error occurs during the request.
    ///
    /// This hook can be used for logging or metrics.
    fn on_error(&self, error: &Error);
}

/// A collection of hooks that execute at different stages of the request lifecycle.
#[derive(Clone, Default)]
pub struct Hooks {
    /// Hooks executed before sending the request.
    pub(crate) before_request: Vec<Arc<dyn BeforeRequestHook>>,
    /// Hooks executed after receiving the response.
    pub(crate) after_response: Vec<Arc<dyn AfterResponseHook>>,
    /// Hooks executed when an error occurs.
    pub(crate) on_error: Vec<Arc<dyn OnErrorHook>>,
}

/// Builder for configuring [`Hooks`].
#[derive(Default)]
pub struct HooksBuilder {
    hooks: Hooks,
}

impl Hooks {
    /// Creates a new empty [`Hooks`] collection.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new [`HooksBuilder`] for configuring hooks.
    #[inline]
    pub fn builder() -> HooksBuilder {
        HooksBuilder::default()
    }

    /// Returns `true` if no hooks are configured.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.before_request.is_empty() && self.after_response.is_empty() && self.on_error.is_empty()
    }

    /// Runs all before-request hooks.
    pub(crate) fn run_before_request(&self, request: &mut Request<Body>) -> Result<(), Error> {
        for hook in &self.before_request {
            hook.on_request(request)?;
        }
        Ok(())
    }

    /// Runs all after-response hooks.
    pub(crate) fn run_after_response(
        &self,
        status: StatusCode,
        headers: &HeaderMap,
    ) -> Result<(), Error> {
        for hook in &self.after_response {
            hook.on_response(status, headers)?;
        }
        Ok(())
    }

    /// Runs all on-error hooks.
    pub(crate) fn run_on_error(&self, error: &Error) {
        for hook in &self.on_error {
            hook.on_error(error);
        }
    }
}

impl HooksBuilder {
    /// Adds a before-request hook.
    pub fn before_request<H>(mut self, hook: Arc<H>) -> Self
    where
        H: BeforeRequestHook + 'static,
    {
        self.hooks.before_request.push(hook);
        self
    }

    /// Adds an after-response hook.
    pub fn after_response<H>(mut self, hook: Arc<H>) -> Self
    where
        H: AfterResponseHook + 'static,
    {
        self.hooks.after_response.push(hook);
        self
    }

    /// Adds an on-error hook.
    pub fn on_error<H>(mut self, hook: Arc<H>) -> Self
    where
        H: OnErrorHook + 'static,
    {
        self.hooks.on_error.push(hook);
        self
    }

    /// Builds the [`Hooks`] collection.
    pub fn build(self) -> Hooks {
        self.hooks
    }
}

// Built-in hooks

/// A hook that logs request and response information.
#[derive(Clone, Default)]
pub struct LoggingHook {
    log_headers: bool,
}

impl LoggingHook {
    /// Creates a new logging hook.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enables logging of request and response headers.
    pub fn with_headers(mut self) -> Self {
        self.log_headers = true;
        self
    }
}

impl BeforeRequestHook for LoggingHook {
    fn on_request(&self, request: &mut Request<Body>) -> Result<(), Error> {
        #[cfg(feature = "tracing")]
        {
            tracing::debug!(
                method = %request.method(),
                uri = %request.uri(),
                "Sending request"
            );
            if self.log_headers {
                for (name, value) in request.headers() {
                    tracing::trace!(header = %name, value = ?value);
                }
            }
        }
        #[cfg(not(feature = "tracing"))]
        {
            let _ = (request, self.log_headers);
        }
        Ok(())
    }
}

impl AfterResponseHook for LoggingHook {
    fn on_response(&self, status: StatusCode, headers: &HeaderMap) -> Result<(), Error> {
        #[cfg(feature = "tracing")]
        {
            tracing::debug!(
                status = %status,
                "Received response"
            );
            if self.log_headers {
                for (name, value) in headers {
                    tracing::trace!(header = %name, value = ?value);
                }
            }
        }
        #[cfg(not(feature = "tracing"))]
        {
            let _ = (status, headers, self.log_headers);
        }
        Ok(())
    }
}

/// A hook that injects headers into every request.
pub struct HeaderInjectionHook {
    headers: HeaderMap,
}

impl HeaderInjectionHook {
    /// Creates a new header injection hook with the given headers.
    pub fn new(headers: HeaderMap) -> Self {
        Self { headers }
    }

    /// Creates a new header injection hook with a single header.
    pub fn single(name: http::header::HeaderName, value: http::HeaderValue) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(name, value);
        Self { headers }
    }
}

impl BeforeRequestHook for HeaderInjectionHook {
    fn on_request(&self, request: &mut Request<Body>) -> Result<(), Error> {
        for (name, value) in &self.headers {
            request.headers_mut().insert(name.clone(), value.clone());
        }
        Ok(())
    }
}

/// A hook that adds a unique request ID to each request.
pub struct RequestIdHook {
    header_name: http::header::HeaderName,
}

impl Default for RequestIdHook {
    fn default() -> Self {
        Self {
            header_name: http::header::HeaderName::from_static("x-request-id"),
        }
    }
}

impl RequestIdHook {
    /// Creates a new request ID hook.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a custom header name for the request ID.
    pub fn with_header_name(mut self, name: http::header::HeaderName) -> Self {
        self.header_name = name;
        self
    }
}

impl BeforeRequestHook for RequestIdHook {
    fn on_request(&self, request: &mut Request<Body>) -> Result<(), Error> {
        // Generate a simple unique ID based on timestamp and random value
        use std::time::{SystemTime, UNIX_EPOCH};

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);

        let id = format!("{:x}", timestamp);
        if let Ok(value) = http::HeaderValue::from_str(&id) {
            request
                .headers_mut()
                .insert(self.header_name.clone(), value);
        }
        Ok(())
    }
}

// Tower Layer and Service implementation

/// A Tower layer that wraps services with hooks.
#[derive(Clone)]
pub struct HooksLayer {
    hooks: Arc<Hooks>,
}

impl HooksLayer {
    /// Creates a new hooks layer.
    pub fn new(hooks: Hooks) -> Self {
        Self {
            hooks: Arc::new(hooks),
        }
    }
}

impl<S> Layer<S> for HooksLayer {
    type Service = HooksService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        HooksService {
            inner,
            hooks: self.hooks.clone(),
        }
    }
}

/// A Tower service that executes hooks before and after requests.
#[derive(Clone)]
pub struct HooksService<S> {
    inner: S,
    hooks: Arc<Hooks>,
}

impl<S, ResBody> Service<Request<Body>> for HooksService<S>
where
    S: Service<Request<Body>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Error: Into<crate::error::BoxError> + Send,
    S::Future: Send,
    ResBody: Send + 'static,
{
    type Response = Response<ResBody>;
    type Error = crate::error::BoxError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        let hooks = self.hooks.clone();
        let mut inner = self.inner.clone();

        // Run before-request hooks
        if let Err(e) = hooks.run_before_request(&mut req) {
            hooks.run_on_error(&e);
            return futures_util::future::err(e.into()).boxed();
        }

        async move {
            match inner.call(req).await {
                Ok(response) => {
                    // Run after-response hooks
                    if let Err(e) = hooks.run_after_response(response.status(), response.headers())
                    {
                        hooks.run_on_error(&e);
                        return Err(e.into());
                    }
                    Ok(response)
                }
                Err(e) => {
                    let boxed_error: crate::error::BoxError = e.into();
                    let error = Error::request(boxed_error);
                    hooks.run_on_error(&error);
                    Err(error.into())
                }
            }
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hooks_builder() {
        let hooks = Hooks::builder().build();
        assert!(hooks.is_empty());
    }

    #[test]
    fn test_logging_hook() {
        let hook = LoggingHook::new().with_headers();
        assert!(hook.log_headers);
    }

    #[test]
    fn test_header_injection_hook() {
        use http::header;

        let hook = HeaderInjectionHook::single(
            header::AUTHORIZATION,
            header::HeaderValue::from_static("Bearer token"),
        );

        let mut req = Request::builder()
            .uri("https://example.com")
            .body(Body::empty())
            .unwrap();

        hook.on_request(&mut req).unwrap();
        assert!(req.headers().contains_key(header::AUTHORIZATION));
    }

    #[test]
    fn test_request_id_hook() {
        let hook = RequestIdHook::new();

        let mut req = Request::builder()
            .uri("https://example.com")
            .body(Body::empty())
            .unwrap();

        hook.on_request(&mut req).unwrap();
        assert!(req.headers().contains_key("x-request-id"));
    }
}
