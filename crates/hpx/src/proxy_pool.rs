//! Proxy pool middleware for rotating proxies across requests.
//!
//! This module provides a shared proxy pool that can be attached to [`crate::ClientBuilder`]
//! to select proxies per request using configurable strategies.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use futures_util::{FutureExt, future::BoxFuture};
use http::{Request, Response, StatusCode};
use rand::RngExt;
use tower::{Layer, Service};

use crate::{
    Body, Error, Proxy, client::layer::config::RequestOptions, config::RequestConfig,
    error::BoxError, proxy::Matcher,
};

/// Strategy used when selecting a proxy from a [`ProxyPool`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProxyPoolStrategy {
    /// Select a random proxy for every request.
    RandomPerRequest,
    /// Keep using the same proxy until a failure is observed, then switch to the next proxy.
    #[default]
    StickyFailover,
}

/// Builder for creating a [`ProxyPool`].
#[derive(Default)]
pub struct ProxyPoolBuilder {
    proxies: Vec<Proxy>,
    strategy: ProxyPoolStrategy,
}

/// Shared proxy pool state used by middleware.
#[derive(Clone, Debug)]
pub struct ProxyPool {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    strategy: ProxyPoolStrategy,
    matchers: Vec<Matcher>,
    sticky_index: AtomicUsize,
}

#[derive(Clone)]
pub(crate) struct ProxyPoolLayer {
    pool: ProxyPool,
}

#[derive(Clone)]
pub(crate) struct ProxyPoolService<S> {
    inner: S,
    pool: ProxyPool,
}

#[derive(Clone)]
struct Selection {
    index: usize,
    matcher: Matcher,
}

// ===== impl ProxyPoolBuilder =====

impl ProxyPoolBuilder {
    /// Create a new proxy pool builder.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the selection strategy.
    #[inline]
    pub fn strategy(mut self, strategy: ProxyPoolStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Add a proxy to the pool.
    #[inline]
    pub fn proxy(mut self, proxy: Proxy) -> Self {
        self.proxies.push(proxy);
        self
    }

    /// Add multiple proxies to the pool.
    #[inline]
    pub fn proxies<I>(mut self, proxies: I) -> Self
    where
        I: IntoIterator<Item = Proxy>,
    {
        self.proxies.extend(proxies);
        self
    }

    /// Build the proxy pool.
    #[inline]
    pub fn build(self) -> crate::Result<ProxyPool> {
        ProxyPool::with_strategy(self.proxies, self.strategy)
    }
}

// ===== impl ProxyPool =====

impl ProxyPool {
    /// Create a proxy pool with the default strategy ([`ProxyPoolStrategy::StickyFailover`]).
    #[inline]
    pub fn new(proxies: Vec<Proxy>) -> crate::Result<Self> {
        Self::with_strategy(proxies, ProxyPoolStrategy::StickyFailover)
    }

    /// Create a proxy pool with the specified strategy.
    pub fn with_strategy(proxies: Vec<Proxy>, strategy: ProxyPoolStrategy) -> crate::Result<Self> {
        let matchers: Vec<Matcher> = proxies.into_iter().map(Proxy::into_matcher).collect();

        if matchers.is_empty() {
            return Err(Error::builder("proxy pool cannot be empty"));
        }

        Ok(Self {
            inner: Arc::new(Inner {
                strategy,
                matchers,
                sticky_index: AtomicUsize::new(0),
            }),
        })
    }

    /// Create a builder for constructing a proxy pool.
    #[inline]
    pub fn builder() -> ProxyPoolBuilder {
        ProxyPoolBuilder::new()
    }

    /// Return the proxy selection strategy.
    #[inline]
    pub fn strategy(&self) -> ProxyPoolStrategy {
        self.inner.strategy
    }

    /// Return the number of proxies in this pool.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.matchers.len()
    }

    /// Returns `true` if the pool contains no proxies.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.matchers.is_empty()
    }

    /// Returns `true` if the HTTP status should be treated as a proxy-failure signal.
    #[inline]
    pub fn is_failure_status(status: StatusCode) -> bool {
        status == StatusCode::PROXY_AUTHENTICATION_REQUIRED
            || status == StatusCode::TOO_MANY_REQUESTS
            || status.is_server_error()
    }

    #[inline]
    pub(crate) fn layer(&self) -> ProxyPoolLayer {
        ProxyPoolLayer { pool: self.clone() }
    }

    fn select(&self) -> Selection {
        let len = self.inner.matchers.len();
        let index = match self.inner.strategy {
            ProxyPoolStrategy::RandomPerRequest => {
                let mut rng = rand::rng();
                rng.random_range(0..len)
            }
            ProxyPoolStrategy::StickyFailover => {
                self.inner.sticky_index.load(Ordering::Relaxed) % len
            }
        };

        Selection {
            index,
            matcher: self.inner.matchers[index].clone(),
        }
    }

    fn record_status(&self, selected_index: usize, status: StatusCode) {
        if Self::is_failure_status(status) {
            self.record_failure(selected_index);
        }
    }

    fn record_error(&self, selected_index: usize, _error: &BoxError) {
        self.record_failure(selected_index);
    }

    fn record_failure(&self, selected_index: usize) {
        if self.inner.strategy != ProxyPoolStrategy::StickyFailover {
            return;
        }

        let len = self.inner.matchers.len();
        if len <= 1 {
            return;
        }

        let next = (selected_index + 1) % len;
        let _ = self.inner.sticky_index.compare_exchange(
            selected_index,
            next,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
    }
}

// ===== impl ProxyPoolLayer =====

impl ProxyPoolLayer {
    #[inline]
    pub(crate) fn new(pool: ProxyPool) -> Self {
        Self { pool }
    }
}

impl<S> Layer<S> for ProxyPoolLayer {
    type Service = ProxyPoolService<S>;

    #[inline]
    fn layer(&self, inner: S) -> Self::Service {
        ProxyPoolService {
            inner,
            pool: self.pool.clone(),
        }
    }
}

// ===== impl ProxyPoolService =====

impl<S, ResBody> Service<Request<Body>> for ProxyPoolService<S>
where
    S: Service<Request<Body>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Error: Into<BoxError> + Send,
    S::Future: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = Response<ResBody>;
    type Error = BoxError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        let selected = self.pool.select();

        RequestConfig::<RequestOptions>::get_mut(req.extensions_mut())
            .get_or_insert_default()
            .proxy_matcher_mut()
            .replace(selected.matcher.clone());

        let pool = self.pool.clone();
        let mut inner = self.inner.clone();

        async move {
            match inner.call(req).await {
                Ok(response) => {
                    pool.record_status(selected.index, response.status());
                    Ok(response)
                }
                Err(error) => {
                    let boxed_error: BoxError = error.into();
                    pool.record_error(selected.index, &boxed_error);
                    Err(boxed_error)
                }
            }
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use http::StatusCode;

    use super::*;

    fn make_pool(strategy: ProxyPoolStrategy) -> ProxyPool {
        ProxyPool::with_strategy(
            vec![
                Proxy::all("http://proxy-a:8080").expect("proxy a should parse"),
                Proxy::all("http://proxy-b:8080").expect("proxy b should parse"),
                Proxy::all("http://proxy-c:8080").expect("proxy c should parse"),
            ],
            strategy,
        )
        .expect("pool should be non-empty")
    }

    #[test]
    fn sticky_failover_switches_only_after_failure() {
        let pool = make_pool(ProxyPoolStrategy::StickyFailover);

        assert_eq!(pool.select().index, 0);

        pool.record_status(0, StatusCode::OK);
        assert_eq!(pool.select().index, 0);

        pool.record_status(0, StatusCode::BAD_GATEWAY);
        assert_eq!(pool.select().index, 1);

        pool.record_status(1, StatusCode::OK);
        assert_eq!(pool.select().index, 1);

        pool.record_status(1, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(pool.select().index, 2);
    }

    #[test]
    fn random_strategy_does_not_advance_sticky_cursor_on_failure() {
        let pool = make_pool(ProxyPoolStrategy::RandomPerRequest);

        pool.record_status(0, StatusCode::BAD_GATEWAY);

        assert_eq!(pool.inner.sticky_index.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn failure_status_classifier_matches_policy() {
        assert!(ProxyPool::is_failure_status(
            StatusCode::PROXY_AUTHENTICATION_REQUIRED
        ));
        assert!(ProxyPool::is_failure_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(ProxyPool::is_failure_status(
            StatusCode::SERVICE_UNAVAILABLE
        ));
        assert!(!ProxyPool::is_failure_status(StatusCode::BAD_REQUEST));
        assert!(!ProxyPool::is_failure_status(StatusCode::OK));
    }

    #[test]
    fn empty_pool_is_rejected() {
        let result = ProxyPool::with_strategy(Vec::new(), ProxyPoolStrategy::StickyFailover);
        assert!(result.is_err());
    }
}
