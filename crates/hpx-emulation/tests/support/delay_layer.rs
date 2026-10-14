#![allow(
    clippy::allow_attributes,
    clippy::assert_is_empty,
    clippy::borrow_as_ptr,
    clippy::cast_lossless,
    clippy::collection_is_never_read,
    clippy::collapsible_if,
    clippy::duration_suboptimal_units,
    clippy::explicit_iter_loop,
    clippy::expect_fun_call,
    clippy::expect_used,
    clippy::ignore_without_reason,
    clippy::ignored_unit_patterns,
    clippy::items_after_test_module,
    clippy::iter_kv_map,
    clippy::iter_with_drain,
    clippy::let_underscore_future,
    clippy::manual_div_ceil,
    clippy::manual_string_new,
    clippy::map_unwrap_or,
    clippy::needless_borrows_for_generic_args,
    clippy::needless_continue,
    clippy::needless_pass_by_ref_mut,
    clippy::needless_raw_string_hashes,
    clippy::nonminimal_bool,
    clippy::panic,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::redundant_async_block,
    clippy::redundant_clone,
    clippy::redundant_pattern_matching,
    clippy::struct_field_names,
    clippy::type_complexity,
    clippy::unchecked_time_subtraction,
    clippy::unreadable_literal,
    clippy::unnecessary_cast,
    clippy::unnecessary_trailing_comma,
    clippy::unnested_or_patterns,
    clippy::useless_borrows_in_formatting,
    clippy::useless_vec,
    clippy::use_self,
    clippy::unwrap_used,
    missing_docs,
    unfulfilled_lint_expectations
)]
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use pin_project_lite::pin_project;
use tokio::time::Sleep;
use tower::{BoxError, Layer, Service};

/// This tower layer injects an arbitrary delay before calling downstream layers.
#[derive(Clone)]
pub struct DelayLayer {
    delay: Duration,
}

impl DelayLayer {
    #[allow(unused)]
    pub const fn new(delay: Duration) -> Self {
        DelayLayer { delay }
    }
}

impl<S> Layer<S> for DelayLayer {
    type Service = Delay<S>;
    fn layer(&self, service: S) -> Self::Service {
        Delay::new(service, self.delay)
    }
}

impl std::fmt::Debug for DelayLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DelayLayer")
            .field("delay", &self.delay)
            .finish()
    }
}

/// This tower service injects an arbitrary delay before calling downstream layers.
#[derive(Debug, Clone)]
pub struct Delay<S> {
    inner: S,
    delay: Duration,
}
impl<S> Delay<S> {
    pub fn new(inner: S, delay: Duration) -> Self {
        Delay { inner, delay }
    }
}

impl<S, Request> Service<Request> for Delay<S>
where
    S: Service<Request>,
    S::Error: Into<BoxError>,
{
    type Response = S::Response;

    type Error = BoxError;

    type Future = ResponseFuture<S::Future>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        match self.inner.poll_ready(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(r) => Poll::Ready(r.map_err(Into::into)),
        }
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let response = self.inner.call(req);
        let sleep = tokio::time::sleep(self.delay);

        ResponseFuture::new(response, sleep)
    }
}

// `Delay` response future
pin_project! {
    #[derive(Debug)]
    pub struct ResponseFuture<S> {
        #[pin]
        response: S,
        #[pin]
        sleep: Sleep,
    }
}

impl<S> ResponseFuture<S> {
    pub(crate) fn new(response: S, sleep: Sleep) -> Self {
        ResponseFuture { response, sleep }
    }
}

impl<F, S, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<S, E>>,
    E: Into<BoxError>,
{
    type Output = Result<S, BoxError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        // First poll the sleep until complete
        match this.sleep.poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(_) => {}
        }

        // Then poll the inner future
        match this.response.poll(cx) {
            Poll::Ready(v) => Poll::Ready(v.map_err(Into::into)),
            Poll::Pending => Poll::Pending,
        }
    }
}
