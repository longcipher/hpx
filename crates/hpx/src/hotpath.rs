//! Hotpath profiling integration for the hpx HTTP and WebSocket clients.
//!
//! The `hotpath` Cargo feature (off by default) links the
//! [hotpath](https://hotpath.rs) profiler and activates per-request timing in
//! this module plus `#[hotpath::measure]` annotations spread across the
//! request pipeline. With the feature disabled every entry point in this
//! module is a zero-overhead passthrough, so the API can stay available in all
//! builds.
//!
//! # HTTP: `HotpathLayer`
//!
//! Add [`HotpathLayer`] to a client stack to time every request and aggregate
//! per-endpoint statistics (count, errors, status codes, total duration):
//!
//! ```no_run
//! use hpx::hotpath::HotpathLayer;
//!
//! let client = hpx::Client::builder()
//!     .layer(HotpathLayer::new())
//!     .build()?;
//! # Ok::<(), hpx::Error>(())
//! ```
//!
//! Each request is wrapped in `hotpath::future!` (visible in hotpath's
//! `futures` report section, labelled with the normalized endpoint), and the
//! request lifecycle is also aggregated by [`snapshot()`] keyed on the
//! normalized endpoint (`GET host/path` with identifier-like path segments
//! collapsed to `{id}`). Transport errors and `>= 400` responses count as
//! errors, mirroring hotpath's own reqwest middleware.
//!
//! # WebSocket: message timing and throughput
//!
//! With the `hotpath` feature enabled, the WebSocket handshake and every
//! send/receive/close call in the `hpx::ws` module is measured via
//! `#[hotpath::measure]`, and cumulative payload bytes are tracked through
//! [`record_ws_bytes_sent`] / [`record_ws_bytes_recv`] into hotpath gauges
//! (`hpx_ws_sent_bytes`, `hpx_ws_recv_bytes`), visible in hotpath's `debug`
//! report section.

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use http::{Method, Uri};
use tower::{BoxError, Layer, Service};

#[cfg(feature = "hotpath")]
use std::{collections::HashMap, sync::OnceLock};

#[cfg(feature = "hotpath")]
use parking_lot::Mutex;

use crate::{Body, ClientResponseBody};

/// Per-endpoint statistics aggregated by [`HotpathLayer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointStat {
    /// Normalized endpoint key, e.g. `GET 127.0.0.1:8080/users/{id}`.
    pub endpoint: String,
    /// Total number of requests to this endpoint.
    pub count: u64,
    /// Requests that failed with a transport error or a `>= 400` response.
    pub error_count: u64,
    /// Total time spent on requests to this endpoint (start to response headers).
    pub total: Duration,
    /// Response status codes observed for this endpoint (status, count).
    pub statuses: Vec<(u16, u64)>,
}

impl EndpointStat {
    /// Average request duration for this endpoint.
    #[must_use]
    pub fn avg(&self) -> Duration {
        self.total.checked_div(self.count.max(1) as u32).unwrap_or_default()
    }
}

/// A Tower [`Layer`] that times every request and records per-endpoint stats.
///
/// With the `hotpath` feature enabled each request future is wrapped in
/// `hotpath::future!` (labelled with the normalized endpoint) and the outcome
/// is fed into the aggregator behind [`snapshot()`]. Without the feature the
/// layer forwards requests unchanged with no measurable overhead.
#[derive(Debug, Clone, Default)]
pub struct HotpathLayer {
    /// Optional label prefixing every recorded endpoint key, letting multiple
    /// clients be told apart in the same report.
    label: Option<String>,
}

impl HotpathLayer {
    /// Creates a new [`HotpathLayer`].
    #[must_use]
    pub const fn new() -> Self {
        Self { label: None }
    }

    /// Creates a [`HotpathLayer`] whose endpoint keys are prefixed with `label`.
    ///
    /// Use a distinct label per client to separate their traffic in the report.
    #[must_use]
    pub fn with_label(label: impl Into<String>) -> Self {
        Self {
            label: Some(label.into()),
        }
    }
}

impl<S> Layer<S> for HotpathLayer {
    type Service = HotpathService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        HotpathService {
            inner,
            label: self.label.clone(),
        }
    }
}

/// The [`Service`] produced by [`HotpathLayer`].
#[derive(Debug, Clone)]
pub struct HotpathService<S> {
    inner: S,
    label: Option<String>,
}

impl<S> Service<http::Request<Body>> for HotpathService<S>
where
    S: Service<http::Request<Body>, Response = http::Response<ClientResponseBody>, Error = BoxError>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = http::Response<ClientResponseBody>;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<Body>) -> Self::Future {
        let key = normalize_endpoint(&endpoint_key(req.method(), req.uri()));
        let key = match &self.label {
            Some(label) => format!("{label}: {key}"),
            None => key,
        };
        let fut = instrument_future(self.inner.call(req), key.clone());
        Box::pin(async move {
            let start = std::time::Instant::now();
            let result = fut.await;
            record_endpoint(
                &key,
                start.elapsed(),
                result.as_ref().ok().map(|r| r.status().as_u16()),
                result.is_err(),
            );
            result
        })
    }
}

/// Wraps the inner request future with `hotpath::future!` when profiling is
/// enabled; identity otherwise.
#[cfg(feature = "hotpath")]
fn instrument_future<F, T>(fut: F, label: String) -> impl Future<Output = T> + Send + 'static
where
    F: Future<Output = T> + Send + 'static,
{
    hotpath::future!(fut, label = label)
}

/// Feature-off counterpart of [`instrument_future`]: pure passthrough.
#[cfg(not(feature = "hotpath"))]
fn instrument_future<F, T>(fut: F, _label: String) -> impl Future<Output = T> + Send + 'static
where
    F: Future<Output = T> + Send + 'static,
{
    fut
}

#[cfg(feature = "hotpath")]
#[derive(Debug, Default)]
struct EndpointEntry {
    count: u64,
    error_count: u64,
    total_nanos: u64,
    statuses: HashMap<u16, u64>,
}

#[cfg(feature = "hotpath")]
static ENDPOINTS: OnceLock<Mutex<HashMap<String, EndpointEntry>>> = OnceLock::new();

#[cfg(feature = "hotpath")]
fn endpoints() -> &'static Mutex<HashMap<String, EndpointEntry>> {
    ENDPOINTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Records one completed request against the aggregator.
///
/// A request is counted as an error when it failed at the transport level or
/// the response status was `>= 400` (same semantics as hotpath's own HTTP
/// section).
#[cfg(feature = "hotpath")]
fn record_endpoint(endpoint: &str, duration: Duration, status: Option<u16>, transport_error: bool) {
    let is_error = transport_error || status.is_some_and(|s| s >= 400);
    let mut guard = endpoints().lock();
    let entry = guard.entry(endpoint.to_string()).or_default();
    entry.count += 1;
    entry.total_nanos += duration.as_nanos() as u64;
    if is_error {
        entry.error_count += 1;
    }
    if let Some(status) = status {
        *entry.statuses.entry(status).or_insert(0) += 1;
    }
}

/// Feature-off counterpart of [`record_endpoint`]: no-op.
#[cfg(not(feature = "hotpath"))]
fn record_endpoint(
    _endpoint: &str,
    _duration: Duration,
    _status: Option<u16>,
    _transport_error: bool,
) {
}

/// Returns the aggregated per-endpoint statistics, slowest endpoint first.
///
/// Without the `hotpath` feature this always returns an empty vector.
#[must_use]
pub fn snapshot() -> Vec<EndpointStat> {
    #[cfg(not(feature = "hotpath"))]
    {
        return Vec::new();
    }
    #[cfg(feature = "hotpath")]
    {
        let guard = endpoints().lock();
        let mut stats: Vec<EndpointStat> = guard
            .iter()
            .map(|(endpoint, entry)| EndpointStat {
                endpoint: endpoint.clone(),
                count: entry.count,
                error_count: entry.error_count,
                total: Duration::from_nanos(entry.total_nanos),
                statuses: {
                    let mut statuses: Vec<(u16, u64)> = entry
                        .statuses
                        .iter()
                        .map(|(&status, &count)| (status, count))
                        .collect();
                    statuses.sort_unstable();
                    statuses
                },
            })
            .collect();
        stats.sort_by(|a, b| {
            b.total
                .cmp(&a.total)
                .then_with(|| b.count.cmp(&a.count))
                .then_with(|| a.endpoint.cmp(&b.endpoint))
        });
        stats
    }
}

/// Records `bytes` sent on a WebSocket connection into the `hpx_ws_sent_bytes`
/// hotpath gauge (only active with the `hotpath` feature).
#[inline]
pub fn record_ws_bytes_sent(bytes: usize) {
    #[cfg(feature = "hotpath")]
    {
        hotpath::gauge!("hpx_ws_sent_bytes").inc(bytes);
    }
    #[cfg(not(feature = "hotpath"))]
    {
        let _ = bytes;
    }
}

/// Records `bytes` received on a WebSocket connection into the
/// `hpx_ws_recv_bytes` hotpath gauge (only active with the `hotpath` feature).
#[inline]
pub fn record_ws_bytes_recv(bytes: usize) {
    #[cfg(feature = "hotpath")]
    {
        hotpath::gauge!("hpx_ws_recv_bytes").inc(bytes);
    }
    #[cfg(not(feature = "hotpath"))]
    {
        let _ = bytes;
    }
}

/// Builds the raw `METHOD host[:port]/path` pre-key for a request.
fn endpoint_key(method: &Method, uri: &Uri) -> String {
    let host = uri.host().unwrap_or("");
    match uri.port_u16() {
        Some(port) => format!("{method} {host}:{port}{}", uri.path()),
        None => format!("{method} {host}{}", uri.path()),
    }
}

/// Normalizes a raw `METHOD host[:port]/path` key by collapsing
/// identifier-like path segments (all digits, UUIDs, and long hex strings)
/// into `{id}`, so parameter-varied requests to the same route merge into a
/// single bucket. Numeric hosts are left untouched.
fn normalize_endpoint(endpoint: &str) -> String {
    let Some(slash) = endpoint.find('/') else {
        return endpoint.to_string();
    };
    let (prefix, path) = endpoint.split_at(slash);
    let normalized_path = path
        .split('/')
        .map(|segment| {
            if is_id_segment(segment) {
                "{id}"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    format!("{prefix}{normalized_path}")
}

fn is_id_segment(segment: &str) -> bool {
    !segment.is_empty()
        && (segment.bytes().all(|b| b.is_ascii_digit())
            || is_uuid(segment)
            || is_long_hex(segment))
}

fn is_uuid(segment: &str) -> bool {
    const HYPHENS: [usize; 4] = [8, 13, 18, 23];
    let bytes = segment.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (i, &b) in bytes.iter().enumerate() {
        if HYPHENS.contains(&i) {
            if b != b'-' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

fn is_long_hex(segment: &str) -> bool {
    segment.len() >= 16 && segment.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_numeric_segments() {
        assert_eq!(
            normalize_endpoint("GET api.example.com/users/1"),
            normalize_endpoint("GET api.example.com/users/42"),
        );
        assert_eq!(
            normalize_endpoint("GET api.example.com/users/1"),
            "GET api.example.com/users/{id}",
        );
    }

    #[test]
    fn merges_uuid_segments() {
        assert_eq!(
            normalize_endpoint("GET api.example.com/jobs/550e8400-e29b-41d4-a716-446655440000"),
            "GET api.example.com/jobs/{id}",
        );
    }

    #[test]
    fn merges_long_hex_segments() {
        assert_eq!(
            normalize_endpoint("GET api.example.com/blobs/deadbeefdeadbeef"),
            "GET api.example.com/blobs/{id}",
        );
    }

    #[test]
    fn keeps_words_and_short_hex() {
        assert_eq!(
            normalize_endpoint("GET api.example.com/cafe/abc123x"),
            "GET api.example.com/cafe/abc123x",
        );
    }

    #[test]
    fn keeps_numeric_host_untouched() {
        assert_eq!(
            normalize_endpoint("GET 127.0.0.1:8080/users/5"),
            "GET 127.0.0.1:8080/users/{id}",
        );
    }

    #[test]
    fn nested_ids() {
        assert_eq!(
            normalize_endpoint("POST api.example.com/users/12/posts/34/comments"),
            "POST api.example.com/users/{id}/posts/{id}/comments",
        );
    }

    #[test]
    fn root_path() {
        assert_eq!(normalize_endpoint("GET api.example.com/"), "GET api.example.com/");
    }

    #[test]
    fn no_path_passthrough() {
        assert_eq!(normalize_endpoint("GET nowhere"), "GET nowhere");
    }

    #[test]
    fn endpoint_key_builds_pre_key() {
        let uri: Uri = "http://127.0.0.1:8080/users/7?q=1".parse().unwrap();
        assert_eq!(
            endpoint_key(&Method::GET, &uri),
            "GET 127.0.0.1:8080/users/7",
        );
    }

    #[test]
    fn endpoint_key_normalizes_ids() {
        let uri: Uri = "https://api.example.com/users/7".parse().unwrap();
        assert_eq!(
            normalize_endpoint(&endpoint_key(&Method::GET, &uri)),
            "GET api.example.com/users/{id}",
        );
    }

    #[cfg(feature = "hotpath")]
    #[test]
    fn record_and_snapshot_aggregate() {
        record_endpoint("GET test.local/a", Duration::from_millis(10), Some(200), false);
        record_endpoint("GET test.local/a", Duration::from_millis(20), Some(500), false);
        record_endpoint("GET test.local/a", Duration::from_millis(30), None, true);

        let stats = snapshot();
        let entry = stats
            .iter()
            .find(|s| s.endpoint == "GET test.local/a")
            .expect("endpoint recorded");
        assert_eq!(entry.count, 3);
        assert_eq!(entry.error_count, 2);
        assert_eq!(entry.total, Duration::from_millis(60));
        assert_eq!(
            entry.statuses,
            vec![(200, 1), (500, 1)],
        );
    }

    #[cfg(not(feature = "hotpath"))]
    #[test]
    fn snapshot_is_empty_without_feature() {
        assert!(snapshot().is_empty());
    }
}
