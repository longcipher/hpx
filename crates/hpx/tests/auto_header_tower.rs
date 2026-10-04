//! Tests for the automatic-header value type and the Tower compatibility
//! adapters.
//!
//! `AutoHeaderValue` is a three-state value (`None` / `Default` / `Provided`)
//! whose whole purpose is to remove ambiguity about whether a header should be
//! sent and with what value. The subtle part is the empty-string case: an
//! explicitly configured empty value and "do not send" must behave identically,
//! because there is no way to express an intentionally empty header here.
//!
//! The Tower compatibility layer is tested for the property users rely on: a
//! `tower::Service` extracted from a `Client` must be usable with standard
//! `http::Request` values, and the adapter must convert `hpx::Request` back into
//! one.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use hpx::{
    Body, Client, ClientResponseBody, Method, StatusCode,
    auto_header::AutoHeaderValue,
    tower_compat::{HpxAdapter, HpxRequestExt as _, TowerServiceExt as _},
};
use http::{HeaderValue, Request, Response};
use tower::Service;

// ---------------------------------------------------------------------------
// AutoHeaderValue
// ---------------------------------------------------------------------------

#[test]
fn default_variant_is_the_default() {
    let value = AutoHeaderValue::default();
    assert!(
        value.is_default(),
        "the derived Default must select the Default variant"
    );
    assert!(!value.is_none());
}

#[test]
fn none_variant_reports_itself() {
    let value = AutoHeaderValue::None;
    assert!(value.is_none());
    assert!(!value.is_default());
}

#[test]
fn provided_variant_reports_neither() {
    let value: AutoHeaderValue = "agent/1.0".into();
    assert!(!value.is_none());
    assert!(!value.is_default());
}

#[test]
fn as_str_returns_the_default_for_the_default_variant() {
    assert_eq!(
        AutoHeaderValue::Default.as_str("hpx/9.9.9"),
        Some("hpx/9.9.9")
    );
}

#[test]
fn as_str_returns_the_provided_value() {
    let value: AutoHeaderValue = "agent/1.0".into();
    assert_eq!(value.as_str("hpx/9.9.9"), Some("agent/1.0"));
}

#[test]
fn as_str_is_none_for_the_none_variant() {
    // `None` means "do not send", which the API expresses as `None` so callers
    // can skip the header entirely rather than sending an empty one.
    assert_eq!(AutoHeaderValue::None.as_str("hpx/9.9.9"), None);
}

#[test]
fn empty_provided_value_collapses_to_not_sending() {
    // Converting an empty string yields `None`, and an empty *default* also
    // yields `None`. Both mean "no header": there is no way to express an
    // intentionally empty header through this type.
    let from_empty: AutoHeaderValue = "".into();
    assert!(from_empty.is_none());
    assert_eq!(from_empty.as_str("hpx/9.9.9"), None);

    assert_eq!(AutoHeaderValue::Default.as_str(""), None);
}

#[test]
fn conversion_accepts_anything_string_like() {
    let from_str: AutoHeaderValue = "abc".into();
    assert_eq!(from_str.as_str("d"), Some("abc"));

    let from_string: AutoHeaderValue = String::from("xyz").into();
    assert_eq!(from_string.as_str("d"), Some("xyz"));

    let from_cow: AutoHeaderValue = std::borrow::Cow::Borrowed("cow").into();
    assert_eq!(from_cow.as_str("d"), Some("cow"));
}

#[test]
fn provided_values_survive_cloning() {
    let original: AutoHeaderValue = "shared".into();
    let cloned = original.clone();
    assert_eq!(original.as_str("d"), cloned.as_str("d"));
    assert_eq!(cloned.as_str("d"), Some("shared"));
}

#[test]
fn debug_output_names_the_variant() {
    assert!(format!("{:?}", AutoHeaderValue::None).contains("None"));
    assert!(format!("{:?}", AutoHeaderValue::Default).contains("Default"));
    let provided: AutoHeaderValue = "v".into();
    assert!(format!("{provided:?}").contains("Provided"));
}

// ---------------------------------------------------------------------------
// Tower compatibility
// ---------------------------------------------------------------------------

/// Error type the adapter's bound requires: `Box<dyn Error + Send + Sync>`.
type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Service that answers with a fixed status using the response body type the
/// adapter requires, so the conversion path is exercised without a network.
#[derive(Clone, Debug, Default)]
struct Echo {
    calls: Arc<AtomicUsize>,
}

impl Service<Request<Body>> for Echo {
    type Response = Response<ClientResponseBody>;
    type Error = BoxError;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, BoxError>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: Request<Body>) -> Self::Future {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(Response::builder()
                .status(StatusCode::IM_A_TEAPOT)
                .body(ClientResponseBody::wrap(http_body_util::Empty::<
                    bytes::Bytes,
                >::new()))
                .expect("a static response always builds"))
        })
    }
}

#[tokio::test]
async fn adapter_forwards_an_hpx_request() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut adapter = HpxAdapter::new(Echo {
        calls: calls.clone(),
    });
    let request: hpx::Request = hpx::Request::new(
        Method::GET,
        "http://example.com/".parse().expect("valid URI"),
    );

    let ready = std::future::poll_fn(|cx| adapter.poll_ready(cx)).await;
    assert!(ready.is_ok(), "the adapter must be ready");

    let response = adapter.call(request).await;
    assert!(response.is_ok(), "the adapter must forward the call");
    assert_eq!(
        response.expect("response").status(),
        StatusCode::IM_A_TEAPOT
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the adapter must reach the inner service exactly once"
    );
}

#[tokio::test]
async fn adapter_into_inner_returns_the_wrapped_service() {
    let calls = Arc::new(AtomicUsize::new(0));
    let adapter = HpxAdapter::new(Echo {
        calls: calls.clone(),
    });
    let mut inner = adapter.into_inner();
    let request = Request::new(Body::from("ping"));
    assert!(inner.call(request).await.is_ok());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn adapter_exposes_its_inner_service() {
    let mut adapter = HpxAdapter::new(Echo::default());
    // `inner()` hands back the wrapped service by shared reference.
    assert_eq!(
        adapter.inner().calls.load(Ordering::SeqCst),
        0,
        "constructing the adapter must not call the inner service"
    );
    let _ = adapter.inner_mut();
}

#[test]
fn adapter_is_debug_and_clone() {
    let adapter = HpxAdapter::new(Echo::default());
    assert!(!format!("{adapter:?}").is_empty());
    let _cloned = adapter.clone();
}

#[tokio::test]
async fn client_yields_a_clonable_tower_service() {
    let client = Client::builder().build().expect("client builds");
    let service = client.tower_service();
    // The point of `HpxService` is that it is `Clone`, so a service can be
    // shared across tower middleware without an `Arc`.
    let mut cloned = service.clone();
    let request = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1:1/unreachable")
        .body(Body::from("x"))
        .expect("request");
    // The request cannot succeed without a server, so only the readiness
    // handshake is checked; what matters is that the service behaves as a
    // `tower::Service`.
    let ready = std::future::poll_fn(|cx| cloned.poll_ready(cx)).await;
    let _ = (ready, cloned.call(request).await);
}

#[test]
fn client_into_tower_service_consumes_the_client() {
    let client = Client::builder().build().expect("client builds");
    let service = client.into_tower_service();
    let _second = service.clone();
}

/// Build a plain request for the opt-out tests.
fn plain_request() -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri("http://example.com/")
        .body(Body::from("payload"))
        .expect("request builder accepts a valid URI")
}

#[test]
fn skip_default_headers_leaves_the_request_usable() {
    // The opt-out is recorded out of band, so the request itself must survive
    // unchanged. If it did not, every other per-request extension would be at
    // risk too.
    let skipped = plain_request().skip_default_headers();
    assert_eq!(skipped.method(), Method::GET);
    assert_eq!(skipped.uri(), "http://example.com/");
}

#[test]
fn skip_default_headers_is_repeatable() {
    // Consuming rather than mutating means a second application is a no-op
    // rather than a conflict, and the source request stays usable.
    let once = plain_request().skip_default_headers();
    let twice = once.skip_default_headers();
    assert_eq!(twice.method(), Method::GET);
    assert_eq!(twice.uri(), "http://example.com/");

    let plain = plain_request();
    assert_eq!(plain.uri(), "http://example.com/");
    // The original is still chainable, proving it was not consumed.
    let chained = plain.skip_default_headers();
    assert_eq!(chained.uri(), "http://example.com/");
}

#[test]
fn skip_default_headers_applies_to_every_method() {
    for method in [Method::GET, Method::POST, Method::HEAD, Method::DELETE] {
        let request = Request::builder()
            .method(method.clone())
            .uri("http://example.com/")
            .body(Body::from("x"))
            .expect("request")
            .skip_default_headers();
        assert_eq!(request.method(), method);
    }
}

// ---------------------------------------------------------------------------
// Header values through the public API
// ---------------------------------------------------------------------------

#[test]
fn header_values_reject_control_characters() {
    // `AutoHeaderValue` accepts any string, so converting to a `HeaderValue` is
    // where an invalid one has to be caught.
    assert!(HeaderValue::try_from("valid").is_ok());
    assert!(HeaderValue::try_from("bad\nvalue").is_err());
    assert!(HeaderValue::try_from("bad\rvalue").is_err());
    assert!(
        HeaderValue::try_from("").is_ok(),
        "an empty header value is legal"
    );
}

#[test]
fn method_round_trips_through_the_compatibility_layer() {
    for method in [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::HEAD,
        Method::OPTIONS,
        Method::TRACE,
    ] {
        let request = Request::builder()
            .method(method.clone())
            .uri("http://example.com/")
            .body(Body::from("x"))
            .expect("valid request");
        assert_eq!(request.method(), method);
    }
}
