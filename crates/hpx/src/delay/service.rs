use std::{
    task::{Context, Poll},
    time::Duration,
};

use tower::{BoxError, Service};

use super::{future::ResponseFuture, jittered_duration};

/// A Tower [`Service`] that introduces a fixed delay before each request.
#[derive(Debug, Clone)]
pub struct Delay<S> {
    inner: S,
    delay: Duration,
}

/// A Tower [`Service`] that conditionally applies fixed delay based on a predicate.
///
/// Requests that match the predicate will have the delay applied;
/// other requests pass through immediately.
#[derive(Clone, Debug)]
pub struct DelayWith<S, P> {
    inner: Delay<S>,
    predicate: P,
}

/// A Tower [`Service`] that applies jittered delay to requests.
///
/// This service wraps an inner service and introduces a random delay
/// (within a configured range) before each request.
#[derive(Clone, Debug)]
pub struct JitterDelay<S> {
    inner: S,
    base: Duration,
    pct: f64,
}

/// A Tower [`Service`] that conditionally applies jittered delay based on a predicate.
///
/// Requests that match the predicate will have a jittered delay applied;
/// other requests pass through immediately.
#[derive(Clone, Debug)]
pub struct JitterDelayWith<S, P> {
    inner: JitterDelay<S>,
    predicate: P,
}

// ===== impl Delay =====

impl<S> Delay<S> {
    /// Create a new [`Delay`] service wrapping the given inner service
    #[inline]
    pub fn new(inner: S, delay: Duration) -> Self {
        Delay { inner, delay }
    }
}

impl<S, Request> Service<Request> for Delay<S>
where
    S: Service<Request> + Clone,
    S::Error: Into<BoxError>,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = ResponseFuture<S, Request>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let sleep = tokio::time::sleep(self.delay);
        ResponseFuture::new(self.inner.clone(), req, sleep)
    }
}

// ===== impl DelayWith =====

impl<S, P> DelayWith<S, P> {
    /// Creates a new [`DelayWith`].
    #[inline]
    pub fn new(inner: S, delay: Duration, predicate: P) -> Self {
        Self {
            inner: Delay::new(inner, delay),
            predicate,
        }
    }
}

impl<S, Req, P> Service<Req> for DelayWith<S, P>
where
    S: Service<Req> + Clone,
    S::Error: Into<BoxError>,
    P: Fn(&Req) -> bool,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = ResponseFuture<S, Req>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Req) -> Self::Future {
        let delay = if (self.predicate)(&req) {
            self.inner.delay
        } else {
            Duration::ZERO
        };
        ResponseFuture::new(self.inner.inner.clone(), req, tokio::time::sleep(delay))
    }
}

// ===== impl JitterDelay =====

impl<S> JitterDelay<S> {
    /// Creates a new [`JitterDelay`].
    #[inline]
    pub fn new(inner: S, base: Duration, pct: f64) -> Self {
        Self {
            inner,
            base,
            pct: pct.clamp(0.0, 1.0),
        }
    }
}

impl<S, Req> Service<Req> for JitterDelay<S>
where
    S: Service<Req> + Clone,
    S::Error: Into<BoxError>,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = ResponseFuture<S, Req>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Req) -> Self::Future {
        let delay = jittered_duration(self.base, self.pct);
        let sleep = tokio::time::sleep(delay);
        ResponseFuture::new(self.inner.clone(), req, sleep)
    }
}

// ===== impl JitterDelayWith =====

impl<S, P> JitterDelayWith<S, P> {
    /// Creates a new [`JitterDelayWith`].
    #[inline]
    pub fn new(inner: S, base: Duration, pct: f64, predicate: P) -> Self {
        Self {
            inner: JitterDelay::new(inner, base, pct),
            predicate,
        }
    }
}

impl<S, Req, P> Service<Req> for JitterDelayWith<S, P>
where
    S: Service<Req> + Clone,
    S::Error: Into<BoxError>,
    P: Fn(&Req) -> bool,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = ResponseFuture<S, Req>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Req) -> Self::Future {
        let delay = if (self.predicate)(&req) {
            jittered_duration(self.inner.base, self.inner.pct)
        } else {
            Duration::ZERO
        };

        ResponseFuture::new(self.inner.inner.clone(), req, tokio::time::sleep(delay))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Context, Poll},
        time::Duration,
    };

    use tower::Service;

    use super::Delay;

    #[derive(Clone)]
    struct SideEffectService {
        calls: Arc<AtomicUsize>,
    }

    impl Service<()> for SideEffectService {
        type Response = ();
        type Error = Infallible;
        type Future = std::future::Ready<Result<(), Infallible>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _req: ()) -> Self::Future {
            self.calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn test_delay_invokes_inner_service_after_sleep() {
        let calls = Arc::new(AtomicUsize::new(0));
        let inner = SideEffectService {
            calls: Arc::clone(&calls),
        };
        let mut delayed = Delay::new(inner, Duration::from_millis(25));
        let started = tokio::time::Instant::now();

        let fut = delayed.call(());
        tokio::pin!(fut);
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        // Initial poll should not invoke the inner service yet.
        assert!(matches!(futures_util::poll!(fut.as_mut()), Poll::Pending));
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let _ = fut.await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(started.elapsed() >= Duration::from_millis(25));
    }
}
