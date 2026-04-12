//! Response recovery support for the HTTP client.

use std::{
    sync::Arc,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_util::{FutureExt, future::BoxFuture};
use http::{HeaderMap, Request, Response, StatusCode};
use http_body::Body as _;
use http_body_util::BodyExt;
use tower::{Layer, Service};

use super::retry::clone_http_request;
use crate::{Body, ClientResponseBody, Error, client::http::InnerResponseBody, error::BoxError};

/// A hook that can recover from a specific status response by optionally returning a replay request.
pub trait OnStatusHook: Send + Sync {
    /// Inspect the buffered response and optionally return a request to replay.
    fn on_status(
        &self,
        context: StatusRecoveryContext,
    ) -> BoxFuture<'static, Result<Option<Request<Body>>, Error>>;
}

/// A buffered response recovery context.
#[non_exhaustive]
pub struct StatusRecoveryContext {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
    original_request: Option<Request<Body>>,
}

impl StatusRecoveryContext {
    pub(crate) fn new(
        status: StatusCode,
        headers: HeaderMap,
        body: Bytes,
        original_request: Option<Request<Body>>,
    ) -> Self {
        Self {
            status,
            headers,
            body,
            original_request,
        }
    }

    /// Returns the response status being recovered.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns the response headers being recovered.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Returns the buffered response body.
    pub fn body(&self) -> &Bytes {
        &self.body
    }

    /// Consume the context and return a replayable copy of the original request, if available.
    pub fn into_original_request(self) -> Option<Request<Body>> {
        self.original_request
    }
}

#[derive(Clone)]
struct StatusRecoveryEntry {
    status: StatusCode,
    hook: Arc<dyn OnStatusHook>,
}

/// A collection of configured response recoveries.
#[derive(Clone, Default)]
pub struct Recoveries {
    hooks: Vec<StatusRecoveryEntry>,
    max_body_bytes: usize,
}

impl Recoveries {
    /// Creates an empty recovery collection.
    #[inline]
    pub fn new() -> Self {
        Self {
            hooks: Vec::new(),
            max_body_bytes: 64 * 1024,
        }
    }

    /// Creates a builder for status recovery hooks.
    #[inline]
    pub fn builder() -> RecoveriesBuilder {
        RecoveriesBuilder::default()
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    fn hook_for(&self, status: StatusCode) -> Option<Arc<dyn OnStatusHook>> {
        self.hooks
            .iter()
            .rev()
            .find(|entry| entry.status == status)
            .map(|entry| entry.hook.clone())
    }

    pub(crate) fn push_hook(&mut self, status: StatusCode, hook: Arc<dyn OnStatusHook>) {
        self.hooks.push(StatusRecoveryEntry { status, hook });
    }

    fn max_body_bytes(&self) -> usize {
        self.max_body_bytes
    }
}

/// Builder for configuring status-based recoveries.
#[derive(Default)]
pub struct RecoveriesBuilder {
    recoveries: Recoveries,
}

impl RecoveriesBuilder {
    /// Adds a recovery hook for a specific status code.
    pub fn on_status<H>(mut self, status: StatusCode, hook: Arc<H>) -> Self
    where
        H: OnStatusHook + 'static,
    {
        self.recoveries
            .hooks
            .push(StatusRecoveryEntry { status, hook });
        self
    }

    /// Sets the maximum response body size that will be buffered for recovery.
    pub fn max_body_bytes(mut self, max_body_bytes: usize) -> Self {
        self.recoveries.max_body_bytes = max_body_bytes;
        self
    }

    /// Builds the configured recoveries.
    pub fn build(self) -> Recoveries {
        self.recoveries
    }
}

/// Layer that performs response recovery and erases response bodies to [`ClientResponseBody`].
#[derive(Clone)]
pub(crate) struct ResponseRecoveryLayer {
    recoveries: Arc<Recoveries>,
}

impl ResponseRecoveryLayer {
    /// Creates a new response recovery layer.
    pub fn new(recoveries: Recoveries) -> Self {
        Self {
            recoveries: Arc::new(recoveries),
        }
    }
}

impl<S> Layer<S> for ResponseRecoveryLayer {
    type Service = ResponseRecovery<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ResponseRecovery {
            inner,
            recoveries: self.recoveries.clone(),
        }
    }
}

/// Service that applies response recovery before exposing responses to downstream middleware.
#[derive(Clone)]
pub(crate) struct ResponseRecovery<S> {
    inner: S,
    recoveries: Arc<Recoveries>,
}

impl<S> Service<Request<Body>> for ResponseRecovery<S>
where
    S: Service<Request<Body>, Response = Response<InnerResponseBody>, Error = BoxError>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = Response<ClientResponseBody>;
    type Error = BoxError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let replayable_request = if self.recoveries.is_empty() {
            None
        } else {
            clone_http_request(&req)
        };
        let recoveries = self.recoveries.clone();
        let mut inner = self.inner.clone();

        async move {
            let response = inner.call(req).await?;
            let Some(hook) = recoveries.hook_for(response.status()) else {
                return Ok(response.map(ClientResponseBody::from_inner_response));
            };

            let body_limit = recoveries.max_body_bytes() as u64;
            let upper = response.body().size_hint().upper();
            if upper.is_none_or(|upper| upper > body_limit) {
                return Ok(response.map(ClientResponseBody::from_inner_response));
            }

            let (parts, body) = response.into_parts();
            let collected = BodyExt::collect(body).await?;
            let bytes = collected.to_bytes();
            let context = StatusRecoveryContext::new(
                parts.status,
                parts.headers.clone(),
                bytes.clone(),
                replayable_request,
            );

            if let Some(request) = hook
                .on_status(context)
                .await
                .map_err(Into::<BoxError>::into)?
            {
                let response = inner.call(request).await?;
                return Ok(response.map(ClientResponseBody::from_inner_response));
            }

            Ok(Response::from_parts(parts, ClientResponseBody::from(bytes)))
        }
        .boxed()
    }
}
