//! Circuit breaker middleware for Tower services.
//!
//! Implements the circuit breaker pattern to prevent cascading failures when
//! a downstream service is unhealthy. When failures exceed a threshold, the
//! circuit "opens" and fast-fails all requests until a recovery timeout.
//!
//! # States
//!
//! - **Closed**: Normal operation. Failures are counted.
//! - **Open**: All requests fail fast. After `recovery_timeout`, transitions to Half-Open.
//! - **Half-Open**: A limited number of probe requests are allowed through.
//!   If they succeed, the circuit closes. If they fail, it opens again.
//!
//! # Example
//!
//! ```rust,no_run
//! use std::time::Duration;
//!
//! use hpx::circuit_breaker::{CircuitBreakerConfig, CircuitBreakerLayer};
//!
//! let client = hpx::Client::builder()
//!     .layer(CircuitBreakerLayer::new(
//!         CircuitBreakerConfig::new(5).recovery_timeout(Duration::from_secs(30)),
//!     ))
//!     .build()
//!     .unwrap();
//! ```

use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use http::{Request, Response};
use parking_lot::Mutex;
use tower::{Layer, Service};

use crate::Body;

/// State of the circuit breaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation. Requests pass through.
    Closed,
    /// Circuit is open. All requests fail fast.
    Open,
    /// Probing state. Limited requests allowed.
    HalfOpen,
}

/// Configuration for the circuit breaker.
#[derive(Clone, Debug)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before opening the circuit.
    pub failure_threshold: u32,
    /// How long to wait before transitioning from Open to Half-Open.
    pub recovery_timeout: Duration,
    /// Number of probe requests allowed in Half-Open state.
    pub half_open_max_probes: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self::new(5)
    }
}

impl CircuitBreakerConfig {
    /// Create a new config with the given failure threshold.
    pub fn new(failure_threshold: u32) -> Self {
        Self {
            failure_threshold,
            recovery_timeout: Duration::from_secs(30),
            half_open_max_probes: 1,
        }
    }

    /// Set the recovery timeout.
    pub fn recovery_timeout(mut self, timeout: Duration) -> Self {
        self.recovery_timeout = timeout;
        self
    }

    /// Set the number of probe requests in half-open state.
    pub fn half_open_max_probes(mut self, probes: u32) -> Self {
        self.half_open_max_probes = probes;
        self
    }
}

/// Internal state of the circuit breaker.
struct CircuitBreakerState {
    state: CircuitState,
    failure_count: u32,
    success_count: u32,
    last_failure_time: Option<Instant>,
}

impl CircuitBreakerState {
    fn new() -> Self {
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            success_count: 0,
            last_failure_time: None,
        }
    }

    fn should_allow_request(&mut self, config: &CircuitBreakerConfig) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if recovery timeout has elapsed
                if let Some(last_failure) = self.last_failure_time {
                    if last_failure.elapsed() >= config.recovery_timeout {
                        self.state = CircuitState::HalfOpen;
                        self.success_count = 0;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => self.success_count < config.half_open_max_probes,
        }
    }

    fn record_success(&mut self, config: &CircuitBreakerConfig) {
        match self.state {
            CircuitState::Closed => {
                self.failure_count = 0;
            }
            CircuitState::HalfOpen => {
                self.success_count += 1;
                if self.success_count >= config.half_open_max_probes {
                    self.state = CircuitState::Closed;
                    self.failure_count = 0;
                }
            }
            CircuitState::Open => {}
        }
    }

    fn record_failure(&mut self, config: &CircuitBreakerConfig) {
        self.last_failure_time = Some(Instant::now());
        match self.state {
            CircuitState::Closed => {
                self.failure_count += 1;
                if self.failure_count >= config.failure_threshold {
                    self.state = CircuitState::Open;
                }
            }
            CircuitState::HalfOpen => {
                self.state = CircuitState::Open;
            }
            CircuitState::Open => {}
        }
    }
}

/// Layer that adds circuit breaker functionality to a service.
#[derive(Clone)]
pub struct CircuitBreakerLayer {
    config: CircuitBreakerConfig,
}

impl CircuitBreakerLayer {
    /// Create a new circuit breaker layer with the given config.
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self { config }
    }
}

impl<S> Layer<S> for CircuitBreakerLayer {
    type Service = CircuitBreakerService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CircuitBreakerService {
            inner,
            config: self.config.clone(),
            state: Arc::new(Mutex::new(CircuitBreakerState::new())),
        }
    }
}

/// Error returned when the circuit is open.
#[derive(Debug)]
pub struct CircuitOpenError {
    /// How long until the circuit might transition to half-open.
    pub retry_after: Duration,
}

impl std::fmt::Display for CircuitOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "circuit breaker is open, retry after {:?}",
            self.retry_after
        )
    }
}

impl std::error::Error for CircuitOpenError {}

/// Tower service with circuit breaker functionality.
#[derive(Clone)]
pub struct CircuitBreakerService<S> {
    inner: S,
    config: CircuitBreakerConfig,
    state: Arc<Mutex<CircuitBreakerState>>,
}

type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;

impl<S, ResBody> Service<Request<Body>> for CircuitBreakerService<S>
where
    S: Service<Request<Body>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Error: Into<crate::error::BoxError> + Send,
    S::Future: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = Response<ResBody>;
    type Error = crate::error::BoxError;
    type Future = BoxFut<Result<Response<ResBody>, crate::error::BoxError>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        // Check circuit state
        {
            let mut state = self.state.lock();
            if !state.should_allow_request(&self.config) {
                let retry_after = self.config.recovery_timeout;
                trace!(
                    "Circuit breaker is open for {} {}, failing fast (retry after {:?})",
                    req.method(),
                    req.uri(),
                    retry_after
                );
                return Box::pin(async move { Err(CircuitOpenError { retry_after }.into()) });
            }
        }

        trace!("Circuit breaker request: {} {}", req.method(), req.uri());

        let mut inner = self.inner.clone();
        let state = self.state.clone();
        let config = self.config.clone();

        Box::pin(async move {
            let result = inner.call(req).await;
            match &result {
                Ok(response) => {
                    let status = response.status();
                    if status.is_server_error() {
                        trace!("Circuit breaker recording server error: {}", status);
                        state.lock().record_failure(&config);
                    } else {
                        state.lock().record_success(&config);
                    }
                }
                Err(_) => {
                    state.lock().record_failure(&config);
                }
            }
            result.map_err(Into::into)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_breaker_config_defaults() {
        let config = CircuitBreakerConfig::new(3);
        assert_eq!(config.failure_threshold, 3);
        assert_eq!(config.recovery_timeout, Duration::from_secs(30));
        assert_eq!(config.half_open_max_probes, 1);
    }

    #[test]
    fn test_circuit_breaker_state_closed() {
        let config = CircuitBreakerConfig::new(3);
        let mut state = CircuitBreakerState::new();

        assert!(state.should_allow_request(&config));
        assert_eq!(state.state, CircuitState::Closed);

        state.record_failure(&config);
        assert_eq!(state.state, CircuitState::Closed);
        assert_eq!(state.failure_count, 1);

        state.record_failure(&config);
        assert_eq!(state.failure_count, 2);

        state.record_failure(&config);
        assert_eq!(state.state, CircuitState::Open);
    }

    #[test]
    fn test_circuit_breaker_state_success_resets() {
        let config = CircuitBreakerConfig::new(3);
        let mut state = CircuitBreakerState::new();

        state.record_failure(&config);
        state.record_failure(&config);
        assert_eq!(state.failure_count, 2);

        state.record_success(&config);
        assert_eq!(state.failure_count, 0);
        assert_eq!(state.state, CircuitState::Closed);
    }

    #[test]
    fn test_circuit_breaker_open_blocks_requests() {
        let config = CircuitBreakerConfig::new(2);
        let mut state = CircuitBreakerState::new();

        state.record_failure(&config);
        state.record_failure(&config);
        assert_eq!(state.state, CircuitState::Open);
        assert!(!state.should_allow_request(&config));
    }
}
