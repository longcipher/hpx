//! Tests for the request-delay middleware.
//!
//! The delay layers are `Tower::Layer` adapters, so the interesting behaviour is
//! in three places: the jitter range arithmetic, the conditional predicate, and
//! whether the wrapped service is actually reached. Timing is asserted loosely —
//! a floor for "did it wait" and a generous ceiling for "did it wait too long" —
//! because exact elapsed time is flaky on a loaded machine. What must hold
//! exactly is that a delay layer never *skips* the delay and never delays a
//! request its predicate rejects.

use std::{
    convert::Infallible,
    future::{Future, poll_fn},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant},
};

use futures::FutureExt as _;
use hpx::{
    Client,
    delay::{DelayLayer, DelayLayerWith, JitterDelayLayer, JitterDelayLayerWith},
};
use http::{Request, Response};
use tower::{Layer, Service};

/// Marker the tests attach to a request to drive the conditional predicate.
#[derive(Clone, Copy)]
struct ShouldDelay(bool);

/// Counts how many requests reached the inner service.
#[derive(Clone, Default)]
struct Counter(Arc<Mutex<usize>>);

impl Counter {
    fn count(&self) -> usize {
        *self.0.lock().expect("counter lock is never poisoned")
    }
}

/// A service that answers immediately.
#[derive(Clone)]
struct Echo {
    counter: Counter,
}

impl Service<Request<()>> for Echo {
    type Response = Response<()>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<()>) -> Self::Future {
        assert!(
            req.extensions().get::<ShouldDelay>().is_some(),
            "every request in this suite carries the marker"
        );
        *self
            .counter
            .0
            .lock()
            .expect("counter lock is never poisoned") += 1;
        Box::pin(async { Ok(Response::new(())) })
    }
}

fn counter() -> Counter {
    Counter::default()
}

/// Build a marked request carrying the predicate verdict.
fn marked(should_delay: bool) -> Request<()> {
    let mut req = Request::new(());
    req.extensions_mut().insert(ShouldDelay(should_delay));
    req
}

fn predicate(req: &Request<()>) -> bool {
    req.extensions()
        .get::<ShouldDelay>()
        .expect("marker present")
        .0
}

// ---------------------------------------------------------------------------
// Fixed delay
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fixed_delay_layer_delays_every_request() {
    let count = counter();
    let delay = Duration::from_millis(120);
    let mut service = DelayLayer::new(delay).layer(Echo {
        counter: count.clone(),
    });

    for _ in 0..2 {
        let started = Instant::now();
        let _ = service.call(marked(true)).await;
        assert!(
            started.elapsed() >= delay,
            "request finished in {:?}, expected at least {delay:?}",
            started.elapsed()
        );
    }

    assert_eq!(count.count(), 2, "the inner service must be reached");
}

#[tokio::test]
async fn zero_delay_is_a_pass_through() {
    let count = counter();
    let mut service = DelayLayer::new(Duration::ZERO).layer(Echo {
        counter: count.clone(),
    });

    let started = Instant::now();
    let _ = service.call(marked(true)).await;
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "a zero delay must not wait, waited {:?}",
        started.elapsed()
    );
    assert_eq!(count.count(), 1);
}

#[tokio::test]
async fn inner_service_error_is_propagated() {
    // The layer must not swallow the inner service's result.
    #[derive(Clone)]
    struct Failing;
    impl Service<Request<()>> for Failing {
        type Response = Response<()>;
        type Error = &'static str;
        type Future = std::future::Ready<Result<Self::Response, Self::Error>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _req: Request<()>) -> Self::Future {
            std::future::ready(Err("boom"))
        }
    }

    let mut service = DelayLayer::new(Duration::from_millis(1)).layer(Failing);
    let result = service.call(marked(true)).await;
    assert!(
        result
            .expect_err("the inner service must surface its error")
            .to_string()
            .contains("boom"),
        "the layer must not swallow the inner service's error"
    );
}

// ---------------------------------------------------------------------------
// Conditional fixed delay
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conditional_delay_layer_applies_only_to_matching_requests() {
    let count = counter();
    let delay = Duration::from_millis(120);
    let mut service = DelayLayer::new(delay).when(predicate).layer(Echo {
        counter: count.clone(),
    });

    // Matching request: delayed.
    let started = Instant::now();
    let _ = service.call(marked(true)).await;
    assert!(
        started.elapsed() >= delay,
        "matching request was not delayed"
    );

    // Non-matching request: passes straight through.
    let started = Instant::now();
    let _ = service.call(marked(false)).await;
    assert!(
        started.elapsed() < delay,
        "non-matching request was delayed for {:?}",
        started.elapsed()
    );

    assert_eq!(count.count(), 2, "both requests must reach the service");
}

#[tokio::test]
async fn conditional_delay_layer_can_be_disabled_entirely() {
    let count = counter();
    let delay = Duration::from_millis(300);
    let mut service = DelayLayer::new(delay).when(predicate).layer(Echo {
        counter: count.clone(),
    });

    let started = Instant::now();
    let _ = service.call(marked(false)).await;
    assert!(
        started.elapsed() < delay,
        "a never-matching predicate must not delay"
    );
    assert_eq!(count.count(), 1);
}

// ---------------------------------------------------------------------------
// Jittered delay
// ---------------------------------------------------------------------------

#[tokio::test]
async fn jittered_delay_stays_inside_the_configured_range() {
    let count = counter();
    let base = Duration::from_millis(40);
    let pct = 0.5;
    // A generous ceiling. The point is that jitter never escapes the documented
    // `base * (1 + pct)` bound, which a missing clamp or an inverted range
    // would break.
    let ceiling = base.mul_f64(1.0 + pct) + Duration::from_millis(120);

    let mut service = JitterDelayLayer::new(base, pct).layer(Echo {
        counter: count.clone(),
    });

    for _ in 0..5 {
        let started = Instant::now();
        let _ = service.call(marked(true)).await;
        assert!(
            started.elapsed() <= ceiling,
            "jittered delay {:?} exceeded the ceiling {ceiling:?}",
            started.elapsed()
        );
    }

    assert_eq!(count.count(), 5);
}

#[tokio::test]
async fn jitter_is_actually_randomised() {
    // With a non-zero percentage, repeated draws must not all be identical.
    // A `pct` that is silently dropped to zero would make every delay equal.
    let count = counter();
    let base = Duration::from_millis(20);
    let mut service = JitterDelayLayer::new(base, 0.9).layer(Echo {
        counter: count.clone(),
    });

    let mut durations = Vec::new();
    for _ in 0..8 {
        let started = Instant::now();
        let _ = service.call(marked(true)).await;
        durations.push(started.elapsed());
    }

    // Coarse buckets: real timing jitter would make exact comparison flaky, but
    // a completely fixed delay would land every sample in one bucket.
    let buckets: std::collections::BTreeSet<u64> = durations
        .iter()
        .map(|d| u64::try_from(d.as_millis() / 5).expect("a delay in milliseconds fits in u64"))
        .collect();
    assert!(
        buckets.len() > 1,
        "every jittered delay landed in the same 5ms bucket: {durations:?}"
    );
    assert_eq!(count.count(), 8);
}

#[tokio::test]
async fn zero_pct_jitter_is_deterministic() {
    // `pct = 0.0` collapses the range to a single point, so every delay must
    // equal the base.
    let count = counter();
    let base = Duration::from_millis(20);
    let mut service = JitterDelayLayer::new(base, 0.0).layer(Echo {
        counter: count.clone(),
    });

    let started = Instant::now();
    let _ = service.call(marked(true)).await;
    assert!(
        started.elapsed() >= base,
        "a zero-percent jitter must still apply the full base delay, waited {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn full_pct_jitter_still_terminates() {
    // `pct = 1.0` gives `low = 0`, so the draw must not be able to produce a
    // zero-width range (the `low >= high` early return) or a panic.
    let count = counter();
    let mut service = JitterDelayLayer::new(Duration::from_millis(2), 1.0).layer(Echo {
        counter: count.clone(),
    });

    for _ in 0..10 {
        let _ = service.call(marked(true)).await;
    }
    assert_eq!(count.count(), 10);
}

#[tokio::test]
async fn conditional_jitter_delay_applies_only_to_matching_requests() {
    let count = counter();
    let base = Duration::from_millis(60);
    let mut service = JitterDelayLayer::new(base, 0.1)
        .when(predicate)
        .layer(Echo {
            counter: count.clone(),
        });

    let started = Instant::now();
    let _ = service.call(marked(true)).await;
    assert!(
        started.elapsed() >= Duration::from_millis(30),
        "matching jittered request was not delayed, waited {:?}",
        started.elapsed()
    );

    let started = Instant::now();
    let _ = service.call(marked(false)).await;
    assert!(
        started.elapsed() < base,
        "non-matching jittered request waited {:?}",
        started.elapsed()
    );

    assert_eq!(count.count(), 2);
}

// ---------------------------------------------------------------------------
// Construction, clamping, and types
// ---------------------------------------------------------------------------

#[tokio::test]
async fn jitter_percentage_is_clamped_into_the_unit_range() {
    // `JitterDelayLayer::new` clamps `pct` to [0, 1]. Values outside that range
    // — including NaN — must still produce a usable service rather than an
    // inverted range or a panic.
    for pct in [
        -1.0,
        -0.001,
        0.0,
        0.5,
        1.0,
        1.001,
        5.0,
        f64::NAN,
        f64::INFINITY,
    ] {
        let mut service =
            JitterDelayLayer::new(Duration::from_millis(1), pct).layer(Echo { counter: counter() });
        let started = Instant::now();
        let _ = service.call(marked(true)).await;
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "pct={pct} produced an unbounded delay of {:?}",
            started.elapsed()
        );
    }
}

#[tokio::test]
async fn conditional_jitter_layer_clamps_too() {
    for pct in [-2.0, 0.25, 3.0, f64::NAN] {
        let mut service = JitterDelayLayer::new(Duration::from_millis(1), pct)
            .when(predicate)
            .layer(Echo { counter: counter() });
        let _ = service.call(marked(true)).await;
        let _ = service.call(marked(false)).await;
    }
}

#[test]
fn layers_are_debug() {
    // `Debug` is derived on all four layers. The conditional variants inherit it
    // from their predicate, so a `Debug` predicate type is used here — a
    // closure deliberately cannot be, which is why this test uses `bool`.
    let _ = format!("{:?}", DelayLayer::new(Duration::from_millis(1)));
    let _ = format!("{:?}", DelayLayerWith::new(Duration::from_millis(1), true));
    let _ = format!("{:?}", JitterDelayLayer::new(Duration::from_millis(1), 0.5));
    let _ = format!(
        "{:?}",
        JitterDelayLayerWith::new(Duration::from_millis(1), 0.5, true)
    );
}

#[test]
fn layer_service_types_are_associated_correctly() {
    // Compile-time evidence that each layer names the right service type.
    fn assert_delay_layer<S: Clone>(inner: S) {
        let _: <DelayLayer as Layer<S>>::Service =
            DelayLayer::new(Duration::ZERO).layer(inner.clone());
        let _: <DelayLayerWith<bool> as Layer<S>>::Service =
            DelayLayerWith::new(Duration::ZERO, true).layer(inner.clone());
        let _: <JitterDelayLayer as Layer<S>>::Service =
            JitterDelayLayer::new(Duration::ZERO, 0.0).layer(inner.clone());
        let _: <JitterDelayLayerWith<bool> as Layer<S>>::Service =
            JitterDelayLayerWith::new(Duration::ZERO, 0.0, true).layer(inner);
    }
    assert_delay_layer(());
}

#[tokio::test]
async fn layer_poll_ready_is_forwarded() {
    // `poll_ready` must reach the inner service rather than short-circuiting,
    // otherwise back-pressure from the inner service is lost.
    #[derive(Clone, Default)]
    struct Counting {
        polls: Arc<Mutex<usize>>,
    }

    impl Service<Request<()>> for Counting {
        type Response = Response<()>;
        type Error = Infallible;
        type Future = std::future::Ready<Result<Self::Response, Infallible>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
            *self.polls.lock().expect("polls lock") += 1;
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _req: Request<()>) -> Self::Future {
            std::future::ready(Ok(Response::new(())))
        }
    }

    let inner = Counting::default();
    let polls = inner.polls.clone();
    let mut service = JitterDelayLayer::new(Duration::ZERO, 0.0).layer(inner);

    poll_fn(|cx| service.poll_ready(cx))
        .now_or_never()
        .expect("poll_ready must resolve immediately")
        .expect("poll_ready must succeed");

    assert_eq!(*polls.lock().expect("polls lock"), 1);
}

#[test]
fn delay_layers_can_be_stacked_on_a_client() {
    // Two unconditional layers plus a conditional one: the builder must accept
    // them in sequence and still produce a working client.
    let client = Client::builder()
        .layer(DelayLayer::new(Duration::from_millis(1)))
        .layer(JitterDelayLayer::new(Duration::from_millis(1), 0.1))
        .layer(DelayLayer::new(Duration::ZERO).when(|_: &http::Request<hpx::Body>| true))
        .build();
    assert!(client.is_ok(), "stacked delay layers must build");
}
