//! Middleware abstractions and implementations for the transport layer.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use http::StatusCode;
use opentelemetry::{
    KeyValue, global,
    metrics::{Counter, Histogram},
};
use rand::Rng;
use scc::HashMap;
use tokio::time::sleep;

use crate::{
    auth::Authentication,
    error::{TransportError, TransportResult},
    transport::{Method, Request, RequestContext, Response, Transport},
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

/// Retry strategy for calculating delay between attempts.
#[derive(Clone, Debug)]
pub enum RetryStrategy {
    /// Exponential backoff: `base * multiplier^attempt`
    Exponential {
        /// Base delay for the first retry.
        base: Duration,
        /// Maximum delay cap.
        max: Duration,
        /// Multiplier for each attempt (default 2.0).
        multiplier: f64,
    },
    /// Linear backoff: `base + (increment * attempt)`
    Linear {
        /// Base delay.
        base: Duration,
        /// Increment per attempt.
        increment: Duration,
        /// Maximum delay cap.
        max: Duration,
    },
    /// Constant delay between retries.
    Constant {
        /// Fixed delay between attempts.
        delay: Duration,
    },
    /// Decorrelated jitter (AWS-style): `min(cap, random_between(base, prev * 3))`
    /// This provides better distribution than exponential jitter.
    DecorrelatedJitter {
        /// Base delay for the first retry.
        base: Duration,
        /// Maximum delay cap.
        max: Duration,
    },
    /// Full jitter: `random(0, min(cap, base * 2^attempt))`
    FullJitter {
        /// Base delay for the first retry.
        base: Duration,
        /// Maximum delay cap.
        max: Duration,
    },
}

impl Default for RetryStrategy {
    fn default() -> Self {
        Self::DecorrelatedJitter {
            base: Duration::from_millis(100),
            max: Duration::from_secs(60),
        }
    }
}

impl RetryStrategy {
    /// Calculate delay for a given attempt using the configured strategy.
    ///
    /// # Arguments
    /// * `attempt` - The current attempt number (1-indexed)
    /// * `prev_delay` - The previous delay (used for decorrelated jitter)
    pub fn calculate_delay(&self, attempt: usize, prev_delay: Option<Duration>) -> Duration {
        match self {
            Self::Exponential {
                base,
                max,
                multiplier,
            } => {
                let delay_ms = (base.as_millis() as f64 * multiplier.powi((attempt - 1) as i32))
                    .min(max.as_millis() as f64) as u64;
                Duration::from_millis(delay_ms)
            }
            Self::Linear {
                base,
                increment,
                max,
            } => {
                let delay = *base + (*increment * (attempt - 1) as u32);
                delay.min(*max)
            }
            Self::Constant { delay } => *delay,
            Self::DecorrelatedJitter { base, max } => {
                let base_ms = base.as_millis() as f64;
                let cap_ms = max.as_millis() as f64;
                let prev_ms = prev_delay.map(|d| d.as_millis() as f64).unwrap_or(base_ms);

                // Decorrelated Jitter: sleep = min(cap, random_between(base, sleep * 3))
                let upper = (prev_ms * 3.0).min(cap_ms);
                let jittered = if upper > base_ms {
                    rand::rng().random_range(base_ms..=upper)
                } else {
                    base_ms
                };
                Duration::from_millis(jittered.min(cap_ms) as u64)
            }
            Self::FullJitter { base, max } => {
                let cap_ms = max.as_millis() as f64;
                let exp_delay_ms =
                    (base.as_millis() as f64 * 2.0f64.powi((attempt - 1) as i32)).min(cap_ms);
                let jittered = rand::rng().random_range(0.0..=exp_delay_ms);
                Duration::from_millis(jittered as u64)
            }
        }
    }
}

/// Configuration for retry logic with enhanced capabilities.
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts (including the initial request).
    pub max_attempts: usize,
    /// Retry strategy for calculating delays.
    pub strategy: RetryStrategy,
    /// HTTP status codes that should trigger a retry.
    pub retryable_status_codes: Vec<u16>,
    /// Whether to respect `Retry-After` header in responses.
    pub respect_retry_after: bool,
    /// Maximum delay to honor from `Retry-After` header (prevents abuse).
    pub max_retry_after: Duration,
    /// Whether to only retry idempotent HTTP methods.
    pub idempotent_only: bool,
    /// Per-method retry configuration (overrides default max_attempts).
    pub method_config: std::collections::HashMap<Method, MethodRetryConfig>,
    /// Retry budget configuration to prevent retry storms.
    pub budget: Option<RetryBudget>,
    /// Legacy fields for backward compatibility
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub backoff_multiplier: f64,
}

/// Per-method retry configuration.
#[derive(Clone, Debug)]
pub struct MethodRetryConfig {
    /// Maximum retry attempts for this method.
    pub max_attempts: usize,
    /// Whether this method is retryable at all.
    pub enabled: bool,
}

impl Default for MethodRetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            enabled: true,
        }
    }
}

/// Retry budget to prevent retry storms.
///
/// The budget uses a token bucket algorithm where:
/// - Successful requests add tokens to the bucket
/// - Retries consume tokens from the bucket
/// - If the bucket is empty, retries are not allowed
#[derive(Clone, Debug)]
pub struct RetryBudget {
    /// Maximum tokens in the bucket.
    max_tokens: u64,
    /// Current token count (shared via Arc for concurrent access).
    tokens: std::sync::Arc<AtomicU64>,
    /// Tokens added per successful request.
    tokens_per_success: u64,
    /// Tokens consumed per retry.
    tokens_per_retry: u64,
    /// Maximum percentage of requests that can be retries (0.0 - 1.0).
    max_retry_ratio: f64,
}

impl RetryBudget {
    /// Create a new retry budget with default settings.
    ///
    /// Default: 10 max tokens, 1 token per success, 5 tokens per retry, 20% max retry ratio.
    pub fn new() -> Self {
        Self {
            max_tokens: 10,
            tokens: std::sync::Arc::new(AtomicU64::new(10)),
            tokens_per_success: 1,
            tokens_per_retry: 5,
            max_retry_ratio: 0.2,
        }
    }

    /// Create a retry budget with custom settings.
    pub fn custom(
        max_tokens: u64,
        tokens_per_success: u64,
        tokens_per_retry: u64,
        max_retry_ratio: f64,
    ) -> Self {
        Self {
            max_tokens,
            tokens: std::sync::Arc::new(AtomicU64::new(max_tokens)),
            tokens_per_success,
            tokens_per_retry,
            max_retry_ratio: max_retry_ratio.clamp(0.0, 1.0),
        }
    }

    /// Try to consume tokens for a retry. Returns true if retry is allowed.
    pub fn try_acquire(&self) -> bool {
        let current = self.tokens.load(Ordering::Relaxed);
        if current >= self.tokens_per_retry {
            // Use compare_exchange for thread-safe token consumption
            self.tokens
                .compare_exchange_weak(
                    current,
                    current.saturating_sub(self.tokens_per_retry),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_ok()
        } else {
            false
        }
    }

    /// Add tokens for a successful request.
    pub fn record_success(&self) {
        let current = self.tokens.load(Ordering::Relaxed);
        let new_value = (current + self.tokens_per_success).min(self.max_tokens);
        self.tokens.store(new_value, Ordering::Relaxed);
    }

    /// Get current token count.
    pub fn available_tokens(&self) -> u64 {
        self.tokens.load(Ordering::Relaxed)
    }

    /// Get the maximum retry ratio.
    pub fn max_retry_ratio(&self) -> f64 {
        self.max_retry_ratio
    }
}

impl Default for RetryBudget {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            strategy: RetryStrategy::default(),
            retryable_status_codes: vec![
                429, // Too Many Requests
                500, // Internal Server Error
                502, // Bad Gateway
                503, // Service Unavailable
                504, // Gateway Timeout
            ],
            respect_retry_after: true,
            max_retry_after: Duration::from_secs(120), // Cap at 2 minutes
            idempotent_only: true,
            method_config: std::collections::HashMap::new(),
            budget: Some(RetryBudget::default()),
            // Legacy fields
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryPolicy {
    /// Create a new retry policy with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum number of retry attempts.
    pub fn with_max_attempts(mut self, max_attempts: usize) -> Self {
        self.max_attempts = max_attempts;
        self
    }

    /// Set the retry strategy.
    pub fn with_strategy(mut self, strategy: RetryStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Add a retryable status code.
    pub fn with_retryable_status(mut self, status: u16) -> Self {
        if !self.retryable_status_codes.contains(&status) {
            self.retryable_status_codes.push(status);
        }
        self
    }

    /// Set whether to respect `Retry-After` header.
    pub fn with_respect_retry_after(mut self, respect: bool) -> Self {
        self.respect_retry_after = respect;
        self
    }

    /// Set the maximum delay to honor from `Retry-After` header.
    pub fn with_max_retry_after(mut self, max: Duration) -> Self {
        self.max_retry_after = max;
        self
    }

    /// Set whether to only retry idempotent methods.
    pub fn with_idempotent_only(mut self, idempotent_only: bool) -> Self {
        self.idempotent_only = idempotent_only;
        self
    }

    /// Configure retry behavior for a specific HTTP method.
    pub fn with_method_config(mut self, method: Method, config: MethodRetryConfig) -> Self {
        self.method_config.insert(method, config);
        self
    }

    /// Set the retry budget.
    pub fn with_budget(mut self, budget: RetryBudget) -> Self {
        self.budget = Some(budget);
        self
    }

    /// Disable retry budget.
    pub fn no_budget(mut self) -> Self {
        self.budget = None;
        self
    }

    /// Get the max attempts for a specific method.
    pub fn max_attempts_for_method(&self, method: &Method) -> usize {
        self.method_config
            .get(method)
            .map(|c| c.max_attempts)
            .unwrap_or(self.max_attempts)
    }

    /// Check if retries are enabled for a specific method.
    pub fn is_retryable_method(&self, method: &Method) -> bool {
        self.method_config
            .get(method)
            .map(|c| c.enabled)
            .unwrap_or(true)
    }

    /// Check if an HTTP method is considered idempotent.
    pub fn is_idempotent(method: &Method) -> bool {
        matches!(
            method,
            Method::Get | Method::Head | Method::Put | Method::Delete | Method::Options
        )
    }
}

// Separate retry logic from middleware stack
pub struct RetryableTransport<T> {
    inner: T,
    retry_policy: RetryPolicy,
}

impl<T> RetryableTransport<T> {
    /// Create a new retryable transport with the given policy.
    pub fn new(transport: T, policy: RetryPolicy) -> Self {
        Self {
            inner: transport,
            retry_policy: policy,
        }
    }

    /// Check if the error or response should trigger a retry.
    fn should_retry(&self, error: &TransportError, method: &Method) -> bool {
        // Check idempotency first if required
        if self.retry_policy.idempotent_only && !RetryPolicy::is_idempotent(method) {
            return false;
        }

        match error {
            TransportError::Network(network_err) => {
                // DNS failures and connection issues are usually transient
                matches!(
                    &**network_err,
                    crate::error::NetworkError::ConnectionFailed { .. }
                        | crate::error::NetworkError::ConnectionLost { .. }
                        | crate::error::NetworkError::DnsResolution { .. }
                        | crate::error::NetworkError::Io { .. }
                )
            }
            TransportError::Timeout { .. } => true,
            TransportError::Protocol(boxed_proto) => {
                match &**boxed_proto {
                    crate::error::ProtocolError::Http { status, .. } => {
                        // Check if status code is in the retryable list
                        self.retry_policy
                            .retryable_status_codes
                            .contains(&status.as_u16())
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Parse `Retry-After` header from an error response.
    /// Returns the delay to wait, or None if not present/parseable.
    fn parse_retry_after(error: &TransportError) -> Option<Duration> {
        // Extract Retry-After from error if it contains response headers
        // This is a simplified implementation - in practice we'd need access to response headers
        if let TransportError::Protocol(boxed_proto) = error
            && let crate::error::ProtocolError::Http {
                body: Some(body_str),
                ..
            } = &**boxed_proto
        {
            // Check for embedded Retry-After (simplified)
            if let Some(retry_after) = Self::extract_retry_after_from_body(body_str) {
                return Some(retry_after);
            }
        }
        None
    }

    /// Extract Retry-After value from response body or headers.
    fn extract_retry_after_from_body(body: &str) -> Option<Duration> {
        // First try to parse as seconds
        if let Ok(seconds) = body.trim().parse::<u64>() {
            return Some(Duration::from_secs(seconds));
        }

        // Try to parse as HTTP-date using httpdate crate
        if let Ok(datetime) = httpdate::parse_http_date(body.trim()) {
            let now = SystemTime::now();
            if let Ok(duration) = datetime.duration_since(now) {
                return Some(duration);
            }
        }

        None
    }

    /// Calculate delay for a retry attempt.
    fn calculate_delay(&self, attempt: usize, prev_delay: Option<Duration>) -> Duration {
        self.retry_policy
            .strategy
            .calculate_delay(attempt, prev_delay)
    }

    /// Calculate delay considering Retry-After header.
    fn calculate_delay_with_retry_after(
        &self,
        attempt: usize,
        prev_delay: Option<Duration>,
        error: &TransportError,
    ) -> Duration {
        // Check for Retry-After header first
        if self.retry_policy.respect_retry_after
            && let Some(retry_after) = Self::parse_retry_after(error)
        {
            // Cap the Retry-After delay
            let capped = retry_after.min(self.retry_policy.max_retry_after);
            tracing::debug!(
                retry_after_secs = retry_after.as_secs(),
                capped_secs = capped.as_secs(),
                "Using Retry-After header for delay"
            );
            return capped;
        }

        // Fall back to strategy-based delay
        self.calculate_delay(attempt, prev_delay)
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
        let method = request.method.clone();

        // Check if retries are enabled for this method
        if !self.retry_policy.is_retryable_method(&method) {
            return self.inner.send(request).await.map_err(|e| e.into());
        }

        // Get max attempts for this specific method
        let max_attempts = self.retry_policy.max_attempts_for_method(&method);

        // Clone request ONCE before the loop
        let original_request = request.clone();
        let mut prev_delay: Option<Duration> = None;

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
                Ok(response) => {
                    // Record success for retry budget
                    if let Some(budget) = &self.retry_policy.budget {
                        budget.record_success();
                    }
                    return Ok(response);
                }
                Err(e) => {
                    let transport_error = e.into();

                    // Check if we should retry
                    let should_retry =
                        attempt < max_attempts && self.should_retry(&transport_error, &method);

                    // Check retry budget if configured
                    let budget_allows = if should_retry {
                        self.retry_policy
                            .budget
                            .as_ref()
                            .map(|b| b.try_acquire())
                            .unwrap_or(true)
                    } else {
                        false
                    };

                    if should_retry && budget_allows {
                        let delay = self.calculate_delay_with_retry_after(
                            attempt,
                            prev_delay,
                            &transport_error,
                        );
                        prev_delay = Some(delay);

                        tracing::info!(
                            attempt = attempt,
                            max_attempts = max_attempts,
                            delay_ms = delay.as_millis(),
                            error = %transport_error,
                            budget_tokens = self.retry_policy.budget.as_ref().map(|b| b.available_tokens()),
                            "Retrying request"
                        );

                        sleep(delay).await;
                        continue;
                    }

                    if should_retry && !budget_allows {
                        tracing::warn!(
                            attempt = attempt,
                            error = %transport_error,
                            "Retry budget exhausted, not retrying"
                        );
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
