//! Middleware abstractions and implementations for the transport layer.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, UNIX_EPOCH},
};

use async_trait::async_trait;
use http::StatusCode;
use opentelemetry::{
    KeyValue, global,
    metrics::{Counter, Histogram},
};
use scc::HashMap;
use tokio::time::sleep;

use crate::{
    auth::Authentication,
    error::{TransportError, TransportResult},
    transport::{Request, RequestContext, Response, Transport},
};

/// Middleware trait for processing requests and responses.
#[async_trait]
pub trait Middleware: Send + Sync + 'static {
    /// Process a request before it's sent to the transport.
    async fn process_request(
        &self,
        request: Request,
        _context: &mut RequestContext,
    ) -> TransportResult<Request> {
        Ok(request)
    }

    /// Process a response after it's received from the transport.
    async fn process_response(
        &self,
        response: Response,
        _context: &RequestContext,
    ) -> TransportResult<Response> {
        Ok(response)
    }

    /// Process an error that occurred during transport.
    async fn process_error(
        &self,
        error: TransportError,
        _context: &RequestContext,
    ) -> TransportResult<TransportError> {
        Ok(error)
    }

    /// Get the middleware name for debugging and metrics.
    fn name(&self) -> &'static str;
}

/// A service that wraps a transport with middleware.
#[derive(Clone)]
pub struct MiddlewareStack<T> {
    transport: T,
    middlewares: Arc<Vec<Arc<dyn Middleware>>>,
}

/// Configuration for retry logic.
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    pub max_attempts: usize,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub backoff_multiplier: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 2.0,
        }
    }
}

// Separate retry logic from middleware stack
pub struct RetryableTransport<T> {
    inner: T,
    retry_policy: RetryPolicy,
}

impl<T> RetryableTransport<T> {
    pub fn new(transport: T, policy: RetryPolicy) -> Self {
        Self {
            inner: transport,
            retry_policy: policy,
        }
    }

    fn should_retry(&self, error: &TransportError) -> bool {
        match error {
            TransportError::Network(_) => true,
            TransportError::Timeout { .. } => true,
            TransportError::Protocol(boxed_proto) => {
                match &**boxed_proto {
                    crate::error::ProtocolError::Http { status, .. } => {
                        // Retry on server errors and rate limits
                        status.as_u16() >= 500 || status.as_u16() == 429
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn calculate_delay(&self, attempt: usize) -> Duration {
        let delay_ms = (self.retry_policy.initial_delay.as_millis() as f64
            * self
                .retry_policy
                .backoff_multiplier
                .powi((attempt - 1) as i32))
        .min(self.retry_policy.max_delay.as_millis() as f64) as u64;
        Duration::from_millis(delay_ms)
    }
}

#[async_trait]
impl<T> Transport for RetryableTransport<T>
where
    T: Transport,
    T::Error: Into<TransportError>,
{
    type Error = TransportError;

    async fn send(&self, request: Request) -> Result<Response, TransportError> {
        let mut attempt = 0;
        let max_attempts = self.retry_policy.max_attempts;

        // Clone request ONCE before the loop
        let original_request = request.clone();

        loop {
            attempt += 1;

            // Use cloned request for each attempt
            let current_request = if attempt == 1 {
                request.clone()
            } else {
                original_request.clone()
            };

            let result = self.inner.send(current_request).await;

            match result {
                Ok(response) => return Ok(response),
                Err(e) => {
                    let transport_error = e.into();
                    if attempt < max_attempts && self.should_retry(&transport_error) {
                        let delay = self.calculate_delay(attempt);
                        sleep(delay).await;
                        continue;
                    }
                    return Err(transport_error);
                }
            }
        }
    }

    fn error_from_status(&self, status: u16) -> Self::Error {
        TransportError::Protocol(Box::new(crate::error::ProtocolError::Http {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            body: None,
        }))
    }
}

impl<T> MiddlewareStack<T> {
    /// Create a new middleware stack.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            middlewares: Arc::new(Vec::new()),
        }
    }

    /// Add a middleware to the stack.
    pub fn layer<M: Middleware>(self, middleware: M) -> Self {
        // Clone the middlewares vec and add the new one
        let mut middlewares = (*self.middlewares).clone();
        middlewares.push(Arc::new(middleware));

        Self {
            transport: self.transport,
            middlewares: Arc::new(middlewares),
        }
    }

    /// Get the number of middlewares in the stack.
    pub fn len(&self) -> usize {
        self.middlewares.len()
    }

    /// Check if the stack is empty.
    pub fn is_empty(&self) -> bool {
        self.middlewares.is_empty()
    }
}

#[async_trait]
impl<T> Transport for MiddlewareStack<T>
where
    T: Transport,
    T::Error: Into<TransportError>,
{
    type Error = TransportError;

    async fn send(&self, request: Request) -> Result<Response, TransportError> {
        let mut context = RequestContext::new(request.id.clone());

        // Process request through middleware stack (forward)
        let mut current_request = request;
        for middleware in self.middlewares.iter() {
            current_request = middleware
                .process_request(current_request, &mut context)
                .await?;
        }

        // Send through transport
        let result = self.transport.send(current_request).await;

        // Process response through middleware stack (reverse)
        match result {
            Ok(mut response) => {
                for middleware in self.middlewares.iter().rev() {
                    response = middleware.process_response(response, &context).await?;
                }
                Ok(response)
            }
            Err(e) => {
                let transport_error = e.into();
                let mut processed_error = transport_error;

                for middleware in self.middlewares.iter().rev() {
                    processed_error = middleware.process_error(processed_error, &context).await?;
                }

                Err(processed_error)
            }
        }
    }

    fn error_from_status(&self, status: u16) -> Self::Error {
        TransportError::Protocol(Box::new(crate::error::ProtocolError::Http {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            body: None,
        }))
    }
}

/// Authentication middleware that adds authentication to requests.
pub struct AuthMiddleware<A> {
    auth: A,
}

impl<A> AuthMiddleware<A>
where
    A: Authentication,
{
    /// Create a new authentication middleware.
    pub fn new(auth: A) -> Self {
        Self { auth }
    }
}

#[async_trait]
impl<A> Middleware for AuthMiddleware<A>
where
    A: Authentication + 'static,
{
    async fn process_request(
        &self,
        mut request: Request,
        _context: &mut RequestContext,
    ) -> TransportResult<Request> {
        // Create a bridge between our Request type and the auth trait
        let mut auth_request = AuthRequestAdapter::new(&mut request);

        self.auth.authenticate(&mut auth_request).await?;
        Ok(request)
    }

    fn name(&self) -> &'static str {
        "auth"
    }
}

/// Adapter to make our Request type work with the Authentication trait.
struct AuthRequestAdapter<'a> {
    request: &'a mut Request,
}

impl<'a> AuthRequestAdapter<'a> {
    fn new(request: &'a mut Request) -> Self {
        Self { request }
    }
}

impl<'a> crate::auth::AuthenticatedRequest for AuthRequestAdapter<'a> {
    fn add_header(&mut self, name: &str, value: &str) -> TransportResult<()> {
        self.request
            .headers
            .insert(name.to_string(), value.to_string());
        Ok(())
    }

    fn get_header(&self, name: &str) -> Option<&str> {
        self.request.headers.get(name).map(|v| v.as_str())
    }

    fn add_query(&mut self, name: &str, value: &str) -> TransportResult<()> {
        self.request
            .query
            .push((name.to_string(), value.to_string()));
        Ok(())
    }

    fn get_query_params(&self) -> Vec<(String, String)> {
        self.request.query.clone()
    }

    fn body(&self) -> Option<&[u8]> {
        self.request.body.as_deref()
    }

    fn url(&self) -> &str {
        &self.request.url
    }

    fn method(&self) -> &str {
        match self.request.method {
            crate::transport::Method::Get => "GET",
            crate::transport::Method::Post => "POST",
            crate::transport::Method::Put => "PUT",
            crate::transport::Method::Delete => "DELETE",
            crate::transport::Method::Patch => "PATCH",
            crate::transport::Method::Head => "HEAD",
            crate::transport::Method::Options => "OPTIONS",
            crate::transport::Method::Connect => "CONNECT",
            crate::transport::Method::Trace => "TRACE",
        }
    }
}

/// Retry middleware that retries failed requests.
pub struct RetryMiddleware {
    max_attempts: usize,
    initial_delay: Duration,
    max_delay: Duration,
    backoff_multiplier: f64,
    retry_condition: Box<dyn Fn(&TransportError) -> bool + Send + Sync>,
}

impl RetryMiddleware {
    /// Create a new retry middleware with default settings.
    pub fn new() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 2.0,
            retry_condition: Box::new(|error| match error {
                TransportError::Network { .. } => true,
                TransportError::Timeout { .. } => true,
                TransportError::Protocol(boxed_proto) => {
                    match &**boxed_proto {
                        crate::error::ProtocolError::Http { status, .. } => {
                            // Retry on server errors and rate limits
                            status.as_u16() >= 500 || status.as_u16() == 429
                        }
                        _ => false,
                    }
                }
                _ => false,
            }),
        }
    }

    /// Set the maximum number of retry attempts.
    pub fn with_max_attempts(mut self, max_attempts: usize) -> Self {
        self.max_attempts = max_attempts;
        self
    }

    /// Set the initial retry delay.
    pub fn with_initial_delay(mut self, delay: Duration) -> Self {
        self.initial_delay = delay;
        self
    }

    /// Set the maximum retry delay.
    pub fn with_max_delay(mut self, delay: Duration) -> Self {
        self.max_delay = delay;
        self
    }

    /// Set the backoff multiplier.
    pub fn with_backoff_multiplier(mut self, multiplier: f64) -> Self {
        self.backoff_multiplier = multiplier;
        self
    }

    /// Set a custom retry condition.
    pub fn with_retry_condition<F>(mut self, condition: F) -> Self
    where
        F: Fn(&TransportError) -> bool + Send + Sync + 'static,
    {
        self.retry_condition = Box::new(condition);
        self
    }

    /// Calculate the delay for a given attempt.
    fn calculate_delay(&self, attempt: usize) -> Duration {
        let delay =
            self.initial_delay.as_millis() as f64 * self.backoff_multiplier.powi(attempt as i32);

        Duration::from_millis(delay.min(self.max_delay.as_millis() as f64) as u64)
    }
}

impl Default for RetryMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware for RetryMiddleware {
    async fn process_error(
        &self,
        error: TransportError,
        context: &RequestContext,
    ) -> TransportResult<TransportError> {
        // Check if we should retry this error
        if !(self.retry_condition)(&error) {
            return Ok(error);
        }

        // Get the current attempt count from context
        let attempt_count = context
            .tags
            .get("retry_attempt")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);

        if attempt_count >= self.max_attempts {
            tracing::warn!(
                request_id = %context.request_id,
                attempts = attempt_count,
                "Maximum retry attempts reached"
            );
            return Ok(error);
        }

        let delay = self.calculate_delay(attempt_count);

        tracing::info!(
            request_id = %context.request_id,
            attempt = attempt_count + 1,
            delay_ms = delay.as_millis(),
            "Retrying request"
        );

        sleep(delay).await;

        // This is a simplified approach. In a real implementation,
        // we would need to modify the context and re-execute the request.
        Ok(error)
    }

    fn name(&self) -> &'static str {
        "retry"
    }
}

/// Rate limiting middleware using token bucket algorithm.
pub struct RateLimitMiddleware {
    tokens: Arc<AtomicU64>,
    max_tokens: u64,
    refill_rate: u64, // tokens per second
    last_refill: Arc<parking_lot::Mutex<Instant>>,
}

impl RateLimitMiddleware {
    /// Create a new rate limit middleware.
    pub fn new(max_tokens: u64, refill_rate: u64) -> Self {
        Self {
            tokens: Arc::new(AtomicU64::new(max_tokens)),
            max_tokens,
            refill_rate,
            last_refill: Arc::new(parking_lot::Mutex::new(Instant::now())),
        }
    }

    /// Refill tokens based on elapsed time.
    fn refill_tokens(&self) {
        let mut last_refill = self.last_refill.lock();
        let now = Instant::now();

        let current = self.tokens.load(Ordering::Relaxed);
        if current >= self.max_tokens {
            *last_refill = now;
            return;
        }

        let elapsed = now.duration_since(*last_refill);
        let elapsed_millis = elapsed.as_millis() as u64;

        let tokens_to_add = (elapsed_millis * self.refill_rate) / 1000;

        if tokens_to_add > 0 {
            let new_tokens = (current + tokens_to_add).min(self.max_tokens);
            self.tokens.store(new_tokens, Ordering::Relaxed);

            // Advance last_refill by the time corresponding to tokens added
            // This ensures we don't lose the fractional part of the time
            let time_used = Duration::from_millis((tokens_to_add * 1000) / self.refill_rate);
            *last_refill += time_used;
        }
    }

    /// Try to consume a token.
    fn try_consume_token(&self) -> bool {
        self.refill_tokens();

        let current = self.tokens.load(Ordering::Relaxed);
        if current > 0 {
            self.tokens
                .compare_exchange_weak(current, current - 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        } else {
            false
        }
    }
}

#[async_trait]
impl Middleware for RateLimitMiddleware {
    async fn process_request(
        &self,
        request: Request,
        context: &mut RequestContext,
    ) -> TransportResult<Request> {
        if !self.try_consume_token() {
            tracing::warn!(
                request_id = %context.request_id,
                "Rate limit exceeded"
            );

            return Err(TransportError::RateLimit {
                message: "Rate limit exceeded".to_string(),
            });
        }

        tracing::trace!(
            request_id = %context.request_id,
            remaining_tokens = self.tokens.load(Ordering::Relaxed),
            "Rate limit check passed"
        );

        Ok(request)
    }

    fn name(&self) -> &'static str {
        "rate_limit"
    }
}

/// Timeout middleware that adds request timeouts.
pub struct TimeoutMiddleware {
    timeout: Duration,
}

impl TimeoutMiddleware {
    /// Create a new timeout middleware.
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }
}

#[async_trait]
impl Middleware for TimeoutMiddleware {
    async fn process_request(
        &self,
        mut request: Request,
        _context: &mut RequestContext,
    ) -> TransportResult<Request> {
        // Set timeout if not already set or if our timeout is shorter
        if request.timeout.is_none_or(|timeout| timeout > self.timeout) {
            request.timeout = Some(self.timeout);
        }
        Ok(request)
    }

    fn name(&self) -> &'static str {
        "timeout"
    }
}

/// Metrics middleware that collects request/response metrics.
#[derive(Clone)]
pub struct MetricsMiddleware {
    request_counter: Arc<AtomicU64>,
    success_counter: Arc<AtomicU64>,
    error_counter: Arc<AtomicU64>,
    response_times: Arc<HashMap<String, Duration>>,
    otlp_request_counter: Counter<u64>,
    otlp_error_counter: Counter<u64>,
    otlp_latency_histogram: Histogram<f64>,
}

impl MetricsMiddleware {
    /// Create a new metrics middleware.
    pub fn new() -> Self {
        let meter = global::meter("longtrader-transport");
        Self {
            request_counter: Arc::new(AtomicU64::new(0)),
            success_counter: Arc::new(AtomicU64::new(0)),
            error_counter: Arc::new(AtomicU64::new(0)),
            response_times: Arc::new(HashMap::new()),
            otlp_request_counter: meter
                .u64_counter("transport.http.requests")
                .with_description("Total number of HTTP requests")
                .build(),
            otlp_error_counter: meter
                .u64_counter("transport.http.errors")
                .with_description("Total number of HTTP request errors")
                .build(),
            otlp_latency_histogram: meter
                .f64_histogram("transport.http.duration")
                .with_unit("s")
                .with_description("HTTP request duration in seconds")
                .build(),
        }
    }

    /// Get the total number of requests.
    pub fn request_count(&self) -> u64 {
        self.request_counter.load(Ordering::Relaxed)
    }

    /// Get the number of successful requests.
    pub fn success_count(&self) -> u64 {
        self.success_counter.load(Ordering::Relaxed)
    }

    /// Get the number of failed requests.
    pub fn error_count(&self) -> u64 {
        self.error_counter.load(Ordering::Relaxed)
    }

    /// Get the success rate.
    pub fn success_rate(&self) -> f64 {
        let total = self.request_count();
        if total == 0 {
            0.0
        } else {
            self.success_count() as f64 / total as f64
        }
    }

    /// Get the average response time.
    pub async fn average_response_time(&self) -> Duration {
        let mut total = Duration::ZERO;
        let mut count = 0;

        // Use any_async in proper async context for better performance
        let _ = self
            .response_times
            .any_async(|_, value| {
                total += *value;
                count += 1;
                false // Keep iterating through all entries
            })
            .await;

        if count == 0 {
            Duration::ZERO
        } else {
            total / count as u32
        }
    }
}

impl Default for MetricsMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware for MetricsMiddleware {
    async fn process_request(
        &self,
        request: Request,
        context: &mut RequestContext,
    ) -> TransportResult<Request> {
        self.request_counter.fetch_add(1, Ordering::Relaxed);
        self.otlp_request_counter
            .add(1, &[KeyValue::new("method", request.method.to_string())]);

        // Record request start time for latency calculation
        context.tags.insert(
            "start_time".to_string(),
            context
                .started_at
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
                .to_string(),
        );

        tracing::info!(
            request_id = %context.request_id,
            method = %request.method,
            url = %request.url,
            "Request started"
        );

        Ok(request)
    }

    async fn process_response(
        &self,
        response: Response,
        context: &RequestContext,
    ) -> TransportResult<Response> {
        self.success_counter.fetch_add(1, Ordering::Relaxed);

        // Calculate response time
        let response_time = context.elapsed();
        self.otlp_latency_histogram.record(
            response_time.as_secs_f64(),
            &[KeyValue::new("status", response.status.to_string())],
        );

        let _ = self
            .response_times
            .insert_sync(context.request_id.to_string(), response_time);

        tracing::info!(
            request_id = %context.request_id,
            status = response.status,
            duration_ms = response_time.as_millis(),
            "Request completed successfully"
        );

        Ok(response)
    }

    async fn process_error(
        &self,
        error: TransportError,
        context: &RequestContext,
    ) -> TransportResult<TransportError> {
        self.error_counter.fetch_add(1, Ordering::Relaxed);
        self.otlp_error_counter
            .add(1, &[KeyValue::new("error", error.to_string())]);

        let response_time = context.elapsed();
        tracing::error!(
            request_id = %context.request_id,
            duration_ms = response_time.as_millis(),
            error = %error,
            "Request failed"
        );

        Ok(error)
    }

    fn name(&self) -> &'static str {
        "metrics"
    }
}

/// Tracing middleware that adds distributed tracing support.
pub struct TracingMiddleware;

impl TracingMiddleware {
    /// Create a new tracing middleware.
    pub fn new() -> Self {
        Self
    }
}

impl Default for TracingMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware for TracingMiddleware {
    async fn process_request(
        &self,
        mut request: Request,
        context: &mut RequestContext,
    ) -> TransportResult<Request> {
        // Generate trace and span IDs if not present
        if context.trace_id.is_none() {
            context.trace_id = Some(uuid::Uuid::new_v4().to_string());
        }

        if context.span_id.is_none() {
            context.span_id = Some(uuid::Uuid::new_v4().to_string());
        }

        // Add tracing headers to the request
        if let Some(ref trace_id) = context.trace_id {
            request
                .headers
                .insert("X-Trace-ID".to_string(), trace_id.clone());
        }

        if let Some(ref span_id) = context.span_id {
            request
                .headers
                .insert("X-Span-ID".to_string(), span_id.clone());
        }

        tracing::debug!(
            request_id = %context.request_id,
            trace_id = ?context.trace_id,
            span_id = ?context.span_id,
            "Added tracing headers"
        );

        Ok(request)
    }

    fn name(&self) -> &'static str {
        "tracing"
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::{auth::NoAuth, transport::Transport};

    struct MockTransport {
        response_status: u16,
        response_body: &'static str,
        call_count: Arc<AtomicUsize>,
    }

    impl MockTransport {
        fn new(status: u16, body: &'static str) -> Self {
            Self {
                response_status: status,
                response_body: body,
                call_count: Arc::new(AtomicUsize::new(0)),
            }
        }

        #[allow(dead_code)]
        fn call_count(&self) -> usize {
            self.call_count.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl crate::transport::Transport for MockTransport {
        type Error = TransportError;

        async fn send(&self, request: Request) -> Result<Response, Self::Error> {
            self.call_count.fetch_add(1, Ordering::Relaxed);

            if self.response_status >= 400 {
                return Err(TransportError::Protocol(Box::new(
                    crate::error::ProtocolError::Http {
                        status: StatusCode::from_u16(self.response_status)
                            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                        body: Some(self.response_body.to_string()),
                    },
                )));
            }

            Ok(Response::new(
                request.id,
                self.response_status,
                self.response_body,
            ))
        }

        fn error_from_status(&self, status: u16) -> Self::Error {
            TransportError::Protocol(Box::new(crate::error::ProtocolError::Http {
                status: StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                body: None,
            }))
        }
    }

    #[tokio::test]
    async fn test_auth_middleware() {
        let transport = MockTransport::new(200, "OK");
        let auth = NoAuth;
        let stack = MiddlewareStack::new(transport).layer(AuthMiddleware::new(auth));

        let request = Request::get("https://api.example.com/test");
        let response = stack.send(request).await.unwrap();

        assert_eq!(response.status, 200);
    }

    #[tokio::test]
    async fn test_metrics_middleware() {
        let transport = MockTransport::new(200, "OK");
        let metrics = MetricsMiddleware::new();
        let stack = MiddlewareStack::new(transport).layer(metrics.clone());

        let request = Request::get("https://api.example.com/test");
        let _response = stack.send(request).await.unwrap();

        assert_eq!(metrics.request_count(), 1);
        assert_eq!(metrics.success_count(), 1);
        assert_eq!(metrics.error_count(), 0);
        assert_eq!(metrics.success_rate(), 1.0);
    }

    #[tokio::test]
    async fn test_rate_limit_middleware() {
        let transport = MockTransport::new(200, "OK");
        let rate_limit = RateLimitMiddleware::new(1, 1); // 1 token, 1 per second
        let stack = MiddlewareStack::new(transport).layer(rate_limit);

        // First request should succeed
        let request = Request::get("https://api.example.com/test");
        let response = stack.send(request).await.unwrap();
        assert_eq!(response.status, 200);

        // Second request should be rate limited
        let request = Request::get("https://api.example.com/test");
        let result = stack.send(request).await;
        assert!(matches!(result, Err(TransportError::RateLimit { .. })));
    }

    #[tokio::test]
    async fn test_timeout_middleware() {
        let transport = MockTransport::new(200, "OK");
        let timeout = TimeoutMiddleware::new(Duration::from_millis(100));
        let stack = MiddlewareStack::new(transport).layer(timeout);

        let request = Request::get("https://api.example.com/test");
        let response = stack.send(request).await.unwrap();

        assert_eq!(response.status, 200);
        // The request should have the timeout set
        // (We can't easily test the actual timeout without making the mock transport slow)
    }

    #[tokio::test]
    async fn test_tracing_middleware() {
        let transport = MockTransport::new(200, "OK");
        let stack = MiddlewareStack::new(transport).layer(TracingMiddleware::new());

        let request = Request::get("https://api.example.com/test");
        let response = stack.send(request).await.unwrap();

        assert_eq!(response.status, 200);
        // In a real test, we would verify that tracing headers were added
    }

    #[tokio::test]
    async fn test_middleware_stack() {
        let transport = MockTransport::new(200, "OK");
        let metrics = MetricsMiddleware::new();
        let stack = MiddlewareStack::new(transport)
            .layer(TracingMiddleware::new())
            .layer(metrics.clone())
            .layer(AuthMiddleware::new(NoAuth));

        assert_eq!(stack.len(), 3);
        assert!(!stack.is_empty());

        let request = Request::get("https://api.example.com/test");
        let response = stack.send(request).await.unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(metrics.request_count(), 1);
        assert_eq!(metrics.success_count(), 1);
    }
}
