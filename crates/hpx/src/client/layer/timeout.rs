//! Middleware for setting a timeout on the response.

mod body;
mod future;

use std::{
    task::{Context, Poll},
    time::Duration,
};

use http::{Request, Response};
use tower::{Layer, Service};

pub use self::body::TimeoutBody;
use self::future::{ResponseBodyTimeoutFuture, ResponseFuture};
use crate::{config::RequestConfig, error::BoxError};

/// Options for configuring granular, multi-phase timeouts.
///
/// Each field corresponds to a specific phase of the HTTP request lifecycle.
/// All timeouts are independent and can be set individually or in combination.
/// When multiple timeouts apply, the most restrictive one takes effect.
#[derive(Clone, Copy)]
pub struct TimeoutOptions {
    /// Timeout for the entire request lifecycle (end-to-end).
    /// Covers DNS resolution through response body completion.
    global: Option<Duration>,
    /// Timeout for a single call attempt when following redirects.
    /// Resets after each redirect.
    per_call: Option<Duration>,
    /// Timeout for DNS resolution.
    resolve: Option<Duration>,
    /// Timeout for TCP connection + TLS handshake.
    connect: Option<Duration>,
    /// Timeout for sending request headers (not body).
    send_request: Option<Duration>,
    /// Timeout for awaiting a `100 Continue` response.
    await_100: Option<Duration>,
    /// Timeout for sending the request body.
    send_body: Option<Duration>,
    /// Timeout for receiving response headers (not body).
    recv_response: Option<Duration>,
    /// Timeout for receiving the response body.
    recv_body: Option<Duration>,
    /// Backward compatibility alias for `global`.
    total_timeout: Option<Duration>,
    /// Backward compatibility alias for `recv_body`.
    read_timeout: Option<Duration>,
    /// Maximum size of HTTP response headers in bytes.
    /// `None` means no limit.
    max_response_header_size: Option<usize>,
}

impl Default for TimeoutOptions {
    fn default() -> Self {
        Self {
            global: None,
            per_call: None,
            resolve: None,
            connect: None,
            send_request: None,
            await_100: Some(Duration::from_secs(1)),
            send_body: None,
            recv_response: None,
            recv_body: None,
            total_timeout: None,
            read_timeout: None,
            max_response_header_size: Some(64 * 1024),
        }
    }
}

impl TimeoutOptions {
    /// Returns the effective global timeout (prefers `global` over `total_timeout`).
    #[inline]
    #[must_use]
    pub fn global_timeout(&self) -> Option<Duration> {
        self.global.or(self.total_timeout)
    }

    /// Returns the effective per-call timeout.
    #[inline]
    #[must_use]
    pub fn per_call_timeout(&self) -> Option<Duration> {
        self.per_call
    }

    /// Returns the DNS resolution timeout.
    #[inline]
    #[must_use]
    pub fn resolve_timeout(&self) -> Option<Duration> {
        self.resolve
    }

    /// Returns the connect (TCP + TLS) timeout.
    #[inline]
    #[must_use]
    pub fn connect_timeout(&self) -> Option<Duration> {
        self.connect
    }

    /// Returns the send-request-headers timeout.
    #[inline]
    #[must_use]
    pub fn send_request_timeout(&self) -> Option<Duration> {
        self.send_request
    }

    /// Returns the 100-continue await timeout.
    #[inline]
    #[must_use]
    pub fn await_100_timeout(&self) -> Option<Duration> {
        self.await_100
    }

    /// Returns the send-body timeout.
    #[inline]
    #[must_use]
    pub fn send_body_timeout(&self) -> Option<Duration> {
        self.send_body
    }

    /// Returns the receive-response-headers timeout.
    #[inline]
    #[must_use]
    pub fn recv_response_timeout(&self) -> Option<Duration> {
        self.recv_response
    }

    /// Returns the effective receive-body timeout (prefers `recv_body` over `read_timeout`).
    #[inline]
    #[must_use]
    pub fn recv_body_timeout(&self) -> Option<Duration> {
        self.recv_body.or(self.read_timeout)
    }

    // Setters

    /// Sets the global (end-to-end) timeout.
    #[inline]
    pub fn timeout_global(&mut self, timeout: Option<Duration>) -> &mut Self {
        self.global = timeout;
        self
    }

    /// Sets the per-call timeout (resets on redirect).
    #[inline]
    pub fn timeout_per_call(&mut self, timeout: Option<Duration>) -> &mut Self {
        self.per_call = timeout;
        self
    }

    /// Sets the DNS resolution timeout.
    #[inline]
    pub fn timeout_resolve(&mut self, timeout: Option<Duration>) -> &mut Self {
        self.resolve = timeout;
        self
    }

    /// Sets the connect (TCP + TLS handshake) timeout.
    #[inline]
    pub fn timeout_connect(&mut self, timeout: Option<Duration>) -> &mut Self {
        self.connect = timeout;
        self
    }

    /// Sets the send-request-headers timeout.
    #[inline]
    pub fn timeout_send_request(&mut self, timeout: Option<Duration>) -> &mut Self {
        self.send_request = timeout;
        self
    }

    /// Sets the 100-continue await timeout.
    #[inline]
    pub fn timeout_await_100(&mut self, timeout: Option<Duration>) -> &mut Self {
        self.await_100 = timeout;
        self
    }

    /// Sets the send-body timeout.
    #[inline]
    pub fn timeout_send_body(&mut self, timeout: Option<Duration>) -> &mut Self {
        self.send_body = timeout;
        self
    }

    /// Sets the receive-response-headers timeout.
    #[inline]
    pub fn timeout_recv_response(&mut self, timeout: Option<Duration>) -> &mut Self {
        self.recv_response = timeout;
        self
    }

    /// Sets the receive-body timeout.
    #[inline]
    pub fn timeout_recv_body(&mut self, timeout: Option<Duration>) -> &mut Self {
        self.recv_body = timeout;
        self
    }

    // Backward-compatible aliases

    /// Sets the total timeout (alias for global, for backward compatibility).
    #[inline]
    pub fn total_timeout(&mut self, total_timeout: Duration) -> &mut Self {
        self.total_timeout = Some(total_timeout);
        self
    }

    /// Sets the read timeout (alias for recv_body, for backward compatibility).
    #[inline]
    pub fn read_timeout(&mut self, read_timeout: Duration) -> &mut Self {
        self.read_timeout = Some(read_timeout);
        self
    }

    /// Returns the max response header size.
    #[inline]
    #[must_use]
    pub fn max_response_header_size(&self) -> Option<usize> {
        self.max_response_header_size
    }

    /// Sets the max response header size in bytes.
    ///
    /// Default is 64KB. Set to `None` to disable the limit.
    #[inline]
    pub fn set_max_response_header_size(&mut self, size: Option<usize>) -> &mut Self {
        self.max_response_header_size = size;
        self
    }
}

impl_request_config_value!(TimeoutOptions);

/// [`Layer`] that applies a [`Timeout`] middleware to a service.
// This layer allows you to set a total timeout and a read timeout for requests.
#[derive(Clone)]
pub struct TimeoutLayer {
    timeout: RequestConfig<TimeoutOptions>,
}

impl TimeoutLayer {
    /// Create a new [`TimeoutLayer`].
    #[inline(always)]
    pub const fn new(options: TimeoutOptions) -> Self {
        TimeoutLayer {
            timeout: RequestConfig::new(Some(options)),
        }
    }
}

impl<S> Layer<S> for TimeoutLayer {
    type Service = Timeout<S>;

    #[inline(always)]
    fn layer(&self, service: S) -> Self::Service {
        Timeout {
            inner: service,
            timeout: self.timeout,
        }
    }
}

/// Middleware that applies total and per-read timeouts to a [`Service`] response body.
#[derive(Clone)]
pub struct Timeout<T> {
    inner: T,
    timeout: RequestConfig<TimeoutOptions>,
}

impl<ReqBody, ResBody, S> Service<Request<ReqBody>> for Timeout<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>, Error = BoxError>,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = ResponseFuture<S::Future>;

    #[inline(always)]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    #[inline(always)]
    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let (total_timeout, read_timeout) = fetch_timeout_options(&self.timeout, req.extensions());
        ResponseFuture {
            response: self.inner.call(req),
            total_timeout: total_timeout.map(tokio::time::sleep),
            read_timeout: read_timeout.map(tokio::time::sleep),
        }
    }
}

/// [`Layer`] that applies a [`ResponseBodyTimeout`] middleware to a service.
// This layer allows you to set a total timeout and a read timeout for the response body.
#[derive(Clone)]
pub struct ResponseBodyTimeoutLayer {
    timeout: RequestConfig<TimeoutOptions>,
}

impl ResponseBodyTimeoutLayer {
    /// Creates a new [`ResponseBodyTimeoutLayer`].
    #[inline(always)]
    pub const fn new(options: TimeoutOptions) -> Self {
        Self {
            timeout: RequestConfig::new(Some(options)),
        }
    }
}

impl<S> Layer<S> for ResponseBodyTimeoutLayer {
    type Service = ResponseBodyTimeout<S>;

    #[inline(always)]
    fn layer(&self, inner: S) -> Self::Service {
        ResponseBodyTimeout {
            inner,
            timeout: self.timeout,
        }
    }
}

/// Middleware that timeouts the response body of a request with a [`Service`] to a total timeout
/// and a read timeout.
#[derive(Clone)]
pub struct ResponseBodyTimeout<S> {
    inner: S,
    timeout: RequestConfig<TimeoutOptions>,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for ResponseBodyTimeout<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
{
    type Response = Response<TimeoutBody<ResBody>>;
    type Error = S::Error;
    type Future = ResponseBodyTimeoutFuture<S::Future>;

    #[inline(always)]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    #[inline(always)]
    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let (total_timeout, read_timeout) = fetch_timeout_options(&self.timeout, req.extensions());
        ResponseBodyTimeoutFuture {
            inner: self.inner.call(req),
            total_timeout,
            read_timeout,
        }
    }
}

fn fetch_timeout_options(
    opts: &RequestConfig<TimeoutOptions>,
    extensions: &http::Extensions,
) -> (Option<Duration>, Option<Duration>) {
    match (opts.as_ref(), opts.fetch(extensions)) {
        (Some(opts), Some(request_opts)) => (
            request_opts.global_timeout().or(opts.global_timeout()),
            request_opts
                .recv_body_timeout()
                .or(opts.recv_body_timeout()),
        ),
        (Some(opts), None) => (opts.global_timeout(), opts.recv_body_timeout()),
        (None, Some(opts)) => (opts.global_timeout(), opts.recv_body_timeout()),
        (None, None) => (None, None),
    }
}
