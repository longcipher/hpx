//! Tower-native compatibility layer for hpx.
//!
//! This module exposes hpx's internal `tower::Service` for direct composition
//! with the broader tower ecosystem, bridging the gap between hpx's convenience
//! API (`hpx::Request` / `hpx::Response`) and standard `http::Request<Body>` /
//! `http::Response<ClientResponseBody>` types.

use std::task::{Context, Poll};

use http::Request;
use tower::util::BoxCloneSyncService;

use super::{Body, Client, ClientResponseBody};
use crate::{Request as HpxRequest, error::BoxError};

/// Per-request configuration for hpx, stored in `http::Extensions`.
///
/// Internal convenience wrapper. Users should use `HpxRequestExt` methods
/// instead of inserting this directly.
#[derive(Debug, Clone, Default)]
pub(crate) struct HpxConfig {
    /// If true, skip client default headers for this request.
    pub(crate) skip_default_headers: bool,
}

/// Extension trait for `http::Request<Body>` providing hpx-specific convenience methods.
///
/// This bridges the gap between standard HTTP request types and hpx's per-request
/// configuration system. All configuration is stored in `http::Extensions`,
/// maintaining full compatibility with the tower ecosystem.
///
/// # Example
///
/// ```ignore
/// use hpx::{Body, tower_compat::HpxRequestExt};
///
/// let req = http::Request::builder()
///     .uri("https://example.com")
///     .body(Body::empty())
///     .unwrap()
///     .skip_default_headers();
/// ```
pub trait HpxRequestExt {
    /// Skip hpx client default headers for this request.
    fn skip_default_headers(self) -> Self;
}

impl HpxRequestExt for Request<Body> {
    fn skip_default_headers(mut self) -> Self {
        // Use the same RequestConfig<DefaultHeaders> mechanism as RequestBuilder.
        // This ensures ConfigService reads the value correctly from extensions.
        use crate::{client::layer::config::DefaultHeaders, config::RequestConfig};
        RequestConfig::<DefaultHeaders>::get_mut(self.extensions_mut()).replace(false);
        self
    }
}

/// The type-erased tower service returned by `TowerServiceExt`.
///
/// This is `BoxCloneSyncService<http::Request<Body>, http::Response<ClientResponseBody>, BoxError>`,
/// compatible with any tower middleware.
pub type HpxService =
    BoxCloneSyncService<Request<Body>, http::Response<ClientResponseBody>, BoxError>;

/// Extension trait exposing hpx's inner tower::Service for direct composition.
///
/// hpx's internal middleware stack operates on standard `http::Request<Body>` and
/// `http::Response<ClientResponseBody>` types. This trait lets you extract that service
/// and compose it with any tower middleware — rate limiters, concurrency limiters,
/// tracing, circuit breakers, and more.
///
/// # Example
///
/// ```ignore
/// use tower::{Service, ServiceBuilder, ServiceExt};
/// use tower::limit::ConcurrencyLimitLayer;
/// use hpx::{Body, tower_compat::TowerServiceExt};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let client = hpx::Client::builder().build()?;
///
/// // Extract the inner service and wrap it with standard tower middleware
/// let service = client.into_tower_service();
/// let service = ServiceBuilder::new()
///     .layer(ConcurrencyLimitLayer::new(100))
///     .service(service);
///
/// // Use standard http::Request — no hpx wrapper types needed
/// let req = http::Request::get("https://example.com")
///     .body(Body::empty())?;
/// let resp = service.oneshot(req).await?;
/// # Ok(())
/// # }
/// ```
pub trait TowerServiceExt {
    /// Consume the client and return the inner tower::Service.
    ///
    /// The returned service implements `tower::Service<http::Request<Body>>`
    /// with `Response = http::Response<ClientResponseBody>`.
    fn into_tower_service(self) -> HpxService;

    /// Get a clone of the inner tower::Service without consuming the client.
    fn tower_service(&self) -> HpxService;
}

impl TowerServiceExt for Client {
    fn into_tower_service(self) -> HpxService {
        BoxCloneSyncService::new(self.into_inner())
    }

    fn tower_service(&self) -> HpxService {
        BoxCloneSyncService::new(self.clone_inner())
    }
}

// ── HpxAdapter ─────────────────────────────────────────────────

/// Adapter that lets any `Service<http::Request<Body>>` accept `hpx::Request`.
///
/// Use this when you have a standard tower service (or composed middleware stack)
/// and want to feed it `hpx::Request` values. The adapter converts `hpx::Request`
/// to `http::Request<Body>` via `Into`, preserving all extensions.
///
/// **Note:** The inner service must produce `http::Response<ClientResponseBody>`.
/// For services with different response bodies, add a `.map_response()` layer
/// before adapting.
///
/// # Example
///
/// ```ignore
/// use tower::{Service, ServiceBuilder, ServiceExt};
/// use tower::limit::ConcurrencyLimitLayer;
/// use hpx::{Body, tower_compat::{HpxAdapter, TowerServiceExt}};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let client = hpx::Client::builder().build()?;
///
/// // Build a standard tower service stack
/// let inner = client.into_tower_service();
/// let adapted = HpxAdapter::new(inner);
///
/// // Now `adapted` accepts hpx::Request, while inner works on http::Request<Body>
/// let req = hpx::Request::new(http::Method::GET, "https://example.com".parse()?);
/// let resp = adapted.oneshot(req).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct HpxAdapter<S> {
    inner: S,
}

impl<S> HpxAdapter<S> {
    /// Wrap a service that accepts `http::Request<Body>` so it accepts `hpx::Request`.
    pub fn new(inner: S) -> Self {
        HpxAdapter { inner }
    }

    /// Consume the adapter and return the inner service.
    pub fn into_inner(self) -> S {
        self.inner
    }

    /// Get a reference to the inner service.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// Get a mutable reference to the inner service.
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }
}

impl<S> tower::Service<HpxRequest> for HpxAdapter<S>
where
    S: tower::Service<
            Request<Body>,
            Response = http::Response<ClientResponseBody>,
            Error = BoxError,
        > + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = http::Response<ClientResponseBody>;
    type Error = BoxError;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: HpxRequest) -> Self::Future {
        // hpx::Request implements Into<http::Request<Body>>
        self.inner.call(req.into())
    }
}
