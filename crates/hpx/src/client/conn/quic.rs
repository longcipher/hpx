//! HTTP/3 (QUIC) connector.
//!
//! This module is gated on the `http3` Cargo feature per [C-01]. It provides
//! [`QuicConnector`], a [`tower::Service<ConnectRequest>`] implementation that:
//!
//! 1. Resolves the request host via the existing `dns::Resolve` infrastructure
//!    (reused from the HTTP/1 + HTTP/2 connector).
//! 2. Establishes a QUIC connection via [`quinn::Endpoint::connect`].
//! 3. Wraps the resulting [`quinn::Connection`] in [`h3_quinn::Connection`].
//! 4. Builds an HTTP/3 client via [`h3::client::builder()`], applying the
//!    [`Http3Options`] (max field section size, grease, …).
//! 5. Spawns a driver task that polls `poll_close` and feeds `close_rx`.
//! 6. Returns an [`H3Connection`] carrier (`send_request`, `close_rx`,
//!    `idle_at`) to the caller.
//!
//! Scope note (T1.4):
//! - `H3Error` is defined in [`crate::error`] (T1.8) and re-exported here via
//!   `pub use crate::error::H3Error;`. The full 11-variant enum per §5.1.9 of
//!   the design lives there; this module consumes it for
//!   `QuicConnector::Service::Error`.
//! - `AltSvcCache` is **omitted** for Phase 1 (YAGNI; Phase 2 = T2.2). A
//!   `// TODO(T2.2): add alt_svc_cache` comment marks the future field.
//! - Pool integration is T1.7 scope. TLS config builder is T1.5 scope. The h3
//!   protocol layer (`proto/h3/client.rs`) is T1.9 scope.

// NOTE: Adapted from the design spec §5.1.5 to match the actual h3 0.0.8 API:
// - `h3::Error` does not exist in h3 0.0.8; `h3::client::builder().build()`
//   returns `Result<_, h3::error::ConnectionError>`. The `H3Error::Framing`
//   variant therefore carries `h3::error::ConnectionError` (not `h3::Error`).
// - `h3::client::SendRequest<h3_quinn::OpenStreams, bytes::Bytes>` matches the
//   design's `H3Connection.send_request` signature exactly.

use std::{
    fmt,
    future::Future,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

use bytes::Bytes;
use tower::Service;

use super::{Connected, Connection};
// Re-export `H3Error` from `crate::error` (T1.8) so that:
//   1. This module can use `H3Error` and its variants directly.
//   2. The existing `crate::client::core::http3` re-export chain
//      (`pub use crate::client::conn::quic::{H3Connection, H3Error,
//      QuicConnector}`) continues to resolve.
// The canonical definition (full 11-variant enum + `Other` catch-all per
// §5.1.9 of the design) lives in `crate::error`.
pub use crate::error::H3Error;
use crate::{
    client::{core::http3::Http3Options, http::ConnectRequest},
    dns::{self, Name, resolve},
};

/// Future returned by [`QuicConnector::call`].
type H3Future = Pin<Box<dyn Future<Output = Result<H3Connection, H3Error>> + Send>>;

/// HTTP/3 connection carrier type.
///
/// Returned by [`QuicConnector`] on a successful handshake. Stored in the
/// connection pool (T1.7 scope) and analogous to `h2::client::SendRequest`
/// for `Ver::Http2`.
pub struct H3Connection {
    /// `h3::client::SendRequest` for opening new request streams.
    pub send_request: h3::client::SendRequest<h3_quinn::OpenStreams, Bytes>,
    /// Receives the terminal `ConnectionError` when the driver task observes
    /// `poll_close` resolving.
    pub close_rx: tokio::sync::mpsc::Receiver<h3::error::ConnectionError>,
    /// Timestamp of the last activity on this connection; used by the pool's
    /// idle eviction logic (T1.7 scope).
    pub idle_at: Instant,
    /// Set to `true` by the driver task once `poll_close` resolves. Read by
    /// [`is_valid`](Self::is_valid) (called from `PoolClient::is_open`) to
    /// evict broken HTTP/3 connections from the pool. Using an `Arc<AtomicBool>`
    /// allows the driver task (which owns the h3 `Connection`) and the
    /// `H3Connection` carrier (which owns the `SendRequest`) to share the
    /// "broken" flag without either side holding a `&mut` to the other.
    pub is_broken: Arc<AtomicBool>,
    /// Set to `true` by the 0-RTT acceptance task once the QUIC handshake
    /// confirms that 0-RTT (early data) was accepted by the server. Using an
    /// `Arc<AtomicBool>` allows the background task (which awaits the
    /// `quinn::ZeroRttAccepted` future) and the `H3Connection` carrier to
    /// share the flag.
    pub used_0rtt: Arc<AtomicBool>,
}

impl fmt::Debug for H3Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("H3Connection")
            .field("send_request", &"<h3::client::SendRequest>")
            .field("close_rx", &self.close_rx)
            .field("idle_at", &self.idle_at)
            .field("is_broken", &self.is_broken.load(Ordering::Relaxed))
            .field("used_0rtt", &self.used_0rtt.load(Ordering::Relaxed))
            .finish()
    }
}

impl H3Connection {
    /// Returns `true` if this connection is still usable, `false` otherwise.
    ///
    /// A connection is considered unusable if either:
    /// - The driver task has observed `poll_close` resolving (the underlying
    ///   QUIC transport has closed, possibly with an error). The driver task
    ///   sets `is_broken = true` in that case.
    /// - The connection has been idle for longer than `idle_timeout` (the
    ///   caller controls this threshold; the pool's `Expiration` is the
    ///   primary idle-eviction mechanism, but `is_valid` exposes the same
    ///   check for callers that need it).
    ///
    /// Passing `Duration::MAX` skips the idle check; this is what
    /// `PoolClient::is_open` does, because the pool's `Expiration` already
    /// handles idle eviction separately.
    ///
    /// This method takes `&self` (not `&mut self`) so it can be called from
    /// `PoolClient::is_open(&self)`. The "broken" signal is shared via the
    /// interior-mutable `Arc<AtomicBool>`; the `close_rx` channel (which
    /// would require `&mut self` to call `try_recv` on) is left untouched
    /// for the actual error-retrieval path, which is T1.9 scope.
    pub fn is_valid(&self, idle_timeout: Duration) -> bool {
        if self.is_broken.load(Ordering::Acquire) {
            return false;
        }
        if idle_timeout == Duration::MAX {
            return true;
        }
        // Avoid `Instant::elapsed` to avoid issues like rust-lang/rust#86470.
        Instant::now().saturating_duration_since(self.idle_at) <= idle_timeout
    }

    /// Returns `true` if this connection was established using 0-RTT (early data).
    ///
    /// 0-RTT allows a client to send data immediately on a resumed connection,
    /// skipping the full TLS handshake. This is a best-effort optimization:
    /// if the server rejects 0-RTT, the connection still works via the full
    /// handshake. The flag is set by a background task that awaits the
    /// `quinn::ZeroRttAccepted` future; it may be `false` initially and
    /// transition to `true` once the handshake completes and 0-RTT is confirmed.
    #[must_use]
    pub fn used_0rtt(&self) -> bool {
        self.used_0rtt.load(Ordering::Acquire)
    }
}

impl Connection for H3Connection {
    fn connected(&self) -> Connected {
        Connected::new()
    }
}

/// HTTP/3 (QUIC) connector implementing `tower::Service<ConnectRequest>`.
///
/// Per [C-04], `QuicConnector` composes with the existing connector layer by
/// implementing the same `tower::Service<ConnectRequest>` trait as
/// `HttpConnector` and `TlsConnector`.
#[derive(Clone)]
pub struct QuicConnector {
    /// The `quinn::Endpoint` used to initiate outgoing QUIC connections.
    endpoint: quinn::Endpoint,
    /// QUIC transport configuration (`Arc`-shared, replaceable per DIP).
    transport_config: Arc<quinn::TransportConfig>,
    /// rustls client configuration for QUIC TLS (`Arc`-shared, replaceable).
    tls_config: Arc<rustls::ClientConfig>,
    /// HTTP/3 protocol options (max field section size, grease, …).
    h3_options: Http3Options,
    // TODO(T2.2): add alt_svc_cache: Arc<AltSvcCache> (Phase 2; omitted per YAGNI).
}

impl fmt::Debug for QuicConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuicConnector")
            .field("endpoint", &self.endpoint)
            .field("transport_config", &self.transport_config)
            .field("tls_config", &self.tls_config)
            .field("h3_options", &self.h3_options)
            .finish_non_exhaustive()
    }
}

impl QuicConnector {
    /// Construct a new `QuicConnector` from the provided QUIC endpoint, transport
    /// config, TLS config, and h3 protocol options.
    ///
    /// All inputs are `Arc`-shared and replaceable (DIP per §5.1.5).
    #[must_use]
    pub fn new(
        endpoint: quinn::Endpoint,
        transport_config: Arc<quinn::TransportConfig>,
        tls_config: Arc<rustls::ClientConfig>,
        h3_options: Http3Options,
    ) -> Self {
        Self {
            endpoint,
            transport_config,
            tls_config,
            h3_options,
        }
    }

    /// Drive the h3 connection to completion, polling `poll_close` and feeding
    /// the terminal `ConnectionError` to `close_tx`.
    ///
    /// Spawned as a background task by `Service::call` after a successful
    /// handshake. The `close_tx` may be dropped (pool eviction) before the
    /// connection closes; in that case the send is silently discarded.
    ///
    /// When `poll_close` resolves, `is_broken` is set to `true` so that
    /// [`H3Connection::is_valid`] returns `false` for any subsequent pool
    /// checkout. This is the primary mechanism by which the pool learns that
    /// an HTTP/3 connection has died: the driver task owns the h3
    /// `Connection` (the only `poll_close` consumer), and the `H3Connection`
    /// carrier owns the `SendRequest`. They share the "broken" signal via
    /// the `Arc<AtomicBool>`.
    ///
    /// If `max_idle_timeout` is `Some(d)`, the function will initiate a
    /// graceful shutdown after `d` of inactivity: it sends a GOAWAY frame
    /// via [`h3::client::Connection::shutdown`], then waits for
    /// `poll_close` to resolve. If `max_idle_timeout` is `None`, the
    /// function waits indefinitely for `poll_close` (the QUIC-level idle
    /// timeout, if set on the transport config, still applies).
    async fn drive_connection(
        mut conn: h3::client::Connection<h3_quinn::Connection, Bytes>,
        close_tx: tokio::sync::mpsc::Sender<h3::error::ConnectionError>,
        is_broken: Arc<AtomicBool>,
        max_idle_timeout: Option<Duration>,
    ) {
        // Signal the broken state before the channel send so that
        // `is_valid` returns `false` even if the pool checks before the
        // receiver consumes the terminal error.
        let err = match max_idle_timeout {
            Some(idle_dur) => {
                tokio::select! {
                    err = futures_util::future::poll_fn(|cx| conn.poll_close(cx)) => {
                        // Connection closed naturally (peer closed or error).
                        err
                    }
                    () = tokio::time::sleep(idle_dur) => {
                        // Idle timeout expired — initiate graceful shutdown.
                        // Send GOAWAY (RFC 9114 §3.3) so the peer can
                        // reliably determine which requests were processed.
                        trace!("h3 idle timeout expired, sending GOAWAY");
                        let _ = conn.shutdown(0).await;
                        // Wait for the connection to finish closing after
                        // GOAWAY has been sent and all streams complete.
                        futures_util::future::poll_fn(|cx| conn.poll_close(cx)).await
                    }
                }
            }
            None => {
                // No application-level idle timeout; wait indefinitely.
                futures_util::future::poll_fn(|cx| conn.poll_close(cx)).await
            }
        };
        is_broken.store(true, Ordering::Release);
        // Receiver may have been dropped (pool evicted the connection); ignore.
        let _ = close_tx.send(err).await;
    }
}

impl Service<ConnectRequest> for QuicConnector {
    type Response = H3Connection;
    type Error = H3Error;
    type Future = H3Future;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Per the behavioral contract: the endpoint is always ready.
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: ConnectRequest) -> Self::Future {
        let endpoint = self.endpoint.clone();
        let transport_config = self.transport_config.clone();
        let tls_config = self.tls_config.clone();
        let h3_options = self.h3_options.clone();

        Box::pin(async move {
            // 1. Extract host and port from the request URI.
            let uri = req.uri().clone();
            let host = uri
                .host()
                .ok_or_else(|| -> H3Error { H3Error::Other("URI missing host".into()) })?;
            let host = host.trim_start_matches('[').trim_end_matches(']');
            let port = uri.port_u16().unwrap_or(443);

            // 2. Resolve host via DNS (or use literal IP directly).
            let addrs: Vec<SocketAddr> = if let Ok(ip) = IpAddr::from_str(host) {
                vec![SocketAddr::new(ip, port)]
            } else {
                // Reuse the existing DNS infrastructure. The connector does not
                // own a `DynResolver` in T1.4 scope; DNS resolution is delegated
                // to the upstream connector layer in T1.7. For Phase 1, use the
                // default `GaiResolver`.
                let mut resolver = dns::GaiResolver::new();
                let name = Name::new(host.into());
                let resolved = resolve(&mut resolver, name)
                    .await
                    .map_err(|e| H3Error::Other(Box::new(e)))?;
                resolved
                    .map(|mut addr| {
                        addr.set_port(port);
                        addr
                    })
                    .collect()
            };

            if addrs.is_empty() {
                return Err(H3Error::Other("no addresses resolved".into()));
            }

            // 3. Build QuicClientConfig + quinn::ClientConfig from the rustls config.
            let quic_client_config = quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)
                .map_err(|e| H3Error::Other(Box::new(e)))?;
            let mut client_config = quinn::ClientConfig::new(Arc::new(quic_client_config));
            client_config.transport_config(transport_config);

            // 4. Try each resolved address; surface the last error if all fail.
            let mut last_err: Option<H3Error> = None;
            for addr in &addrs {
                let connecting = match endpoint.connect_with(client_config.clone(), *addr, host) {
                    Ok(c) => c,
                    Err(source) => {
                        last_err = Some(H3Error::Other(Box::new(source)));
                        continue;
                    }
                };

                // 4a. Attempt 0-RTT (early data). If the TLS config has
                //     `enable_early_data = true` and a cached session ticket is
                //     available, `into_0rtt()` returns `Ok((conn, accepted))`.
                //     The `conn` can be used immediately; the `accepted` future
                //     resolves when the handshake completes, indicating whether
                //     0-RTT was actually accepted by the server.
                //     If `into_0rtt()` returns `Err(connecting)`, 0-RTT is not
                //     possible — fall back to the full handshake via `.await`.
                let (quinn_conn, used_0rtt) = match connecting.into_0rtt() {
                    Ok((conn, accepted)) => {
                        let used = Arc::new(AtomicBool::new(false));
                        let used_clone = used.clone();
                        // Spawn a background task to await the
                        // `ZeroRttAccepted` future. When it resolves, the
                        // flag is set to `true` if 0-RTT was accepted, or
                        // stays `false` if the server rejected it.
                        // `JoinHandle` is `#[must_use]`; explicitly detach
                        // via drop.
                        drop(tokio::spawn(async move {
                            used_clone.store(accepted.await, Ordering::Release);
                        }));
                        (conn, used)
                    }
                    Err(connecting) => {
                        // 0-RTT not possible; perform full TLS handshake.
                        match connecting.await {
                            Ok(quinn_conn) => (quinn_conn, Arc::new(AtomicBool::new(false))),
                            Err(source) => {
                                last_err = Some(H3Error::Handshake { source });
                                continue;
                            }
                        }
                    }
                };

                // 5. Wrap quinn::Connection in h3_quinn::Connection.
                let h3_quinn_conn = h3_quinn::Connection::new(quinn_conn);

                // 6. Build h3 client via h3::client::builder().
                let mut builder = h3::client::builder();
                if let Some(max_field_section_size) = h3_options.max_field_section_size {
                    builder.max_field_section_size(max_field_section_size);
                }
                builder.send_grease(h3_options.send_grease);
                builder.enable_extended_connect(h3_options.enable_connect_protocol);
                let (h3_conn, send_request) = builder
                    .build(h3_quinn_conn)
                    .await
                    .map_err(|e| H3Error::Framing { source: e })?;

                // 7. Spawn driver task polling poll_close and feeding close_rx.
                let (close_tx, close_rx) =
                    tokio::sync::mpsc::channel::<h3::error::ConnectionError>(1);
                // `Arc<AtomicBool>` shared between the driver task and
                // the `H3Connection` carrier; the driver sets it to
                // `true` when `poll_close` resolves, and
                // `is_valid` reads it on pool checkout.
                let is_broken = Arc::new(AtomicBool::new(false));
                // `JoinHandle` is `#[must_use]`; explicitly detach via drop.
                drop(tokio::spawn(Self::drive_connection(
                    h3_conn,
                    close_tx,
                    is_broken.clone(),
                    h3_options.max_idle_timeout,
                )));

                // 8. Return H3Connection.
                return Ok(H3Connection {
                    send_request,
                    close_rx,
                    idle_at: Instant::now(),
                    is_broken,
                    used_0rtt,
                });
            }

            // `addrs` is guaranteed non-empty (checked above), so `last_err`
            // is always `Some` here. Use a safe match to avoid `unwrap`.
            match last_err {
                Some(err) => Err(err),
                None => Err(H3Error::Other("no addresses available".into())),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        net::SocketAddr,
        sync::Arc,
        task::{Context, Poll},
        time::Duration,
    };

    use http::Uri;
    use quinn::{Endpoint, TransportConfig};
    use rustls::ClientConfig;
    use tower::Service;

    use super::*;
    use crate::client::{core::http3::Http3Options, http::ConnectRequest};

    type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

    /// Build a rustls `ClientConfig` that trusts nothing — sufficient for the
    /// unit tests below, which never reach the certificate verification step.
    ///
    /// Uses `builder_with_provider` with an explicit `ring` provider because
    /// enabling the `http3` feature pulls in both `aws-lc-rs` (via rustls
    /// `default`) and `ring` (via hpx's explicit feature), making
    /// `ClientConfig::builder()` panic with "Could not automatically determine
    /// the process-level CryptoProvider".
    fn make_test_tls_config() -> TestResult<Arc<ClientConfig>> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                format!("bad protocol versions: {e}").into()
            })?
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        Ok(Arc::new(config))
    }

    /// Bind a client-side `quinn::Endpoint` to an ephemeral loopback port.
    ///
    /// `quinn::Endpoint::client` requires a tokio runtime, so callers must
    /// invoke this from within a `#[tokio::test]` (or `#[tokio::main]`) context.
    fn make_test_endpoint() -> TestResult<Endpoint> {
        let addr: SocketAddr =
            "127.0.0.1:0"
                .parse()
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                    format!("bad addr: {e}").into()
                })?;
        Ok(Endpoint::client(addr)?)
    }

    /// (a) `QuicConnector::new(...)` constructs without panicking.
    ///
    /// Marked `#[tokio::test]` because `quinn::Endpoint::client` requires a
    /// tokio runtime (it spawns an I/O driver task).
    #[tokio::test]
    async fn quic_connector_new_constructs() -> TestResult<()> {
        let endpoint = make_test_endpoint()?;
        let transport_config = Arc::new(TransportConfig::default());
        let tls_config = make_test_tls_config()?;
        let h3_options = Http3Options::default();

        let _connector = QuicConnector::new(endpoint, transport_config, tls_config, h3_options);
        Ok(())
    }

    /// (b) `poll_ready` returns `Poll::Ready(Ok(()))` per the behavioral contract.
    ///
    /// Marked `#[tokio::test]` because `quinn::Endpoint::client` requires a
    /// tokio runtime. The `poll_ready` call itself is synchronous.
    #[tokio::test]
    async fn quic_connector_poll_ready_returns_ok() -> TestResult<()> {
        let endpoint = make_test_endpoint()?;
        let transport_config = Arc::new(TransportConfig::default());
        let tls_config = make_test_tls_config()?;
        let h3_options = Http3Options::default();

        let mut connector = QuicConnector::new(endpoint, transport_config, tls_config, h3_options);

        let waker = futures_util::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let poll = <QuicConnector as Service<ConnectRequest>>::poll_ready(&mut connector, &mut cx);
        match poll {
            Poll::Ready(Ok(())) => Ok(()),
            other => Err(format!("expected Poll::Ready(Ok(())), got {other:?}").into()),
        }
    }

    /// (c) `H3Error::Handshake { source }` Display output contains the expected
    /// prefix per the minimal `H3Error` definition.
    #[test]
    fn h3_error_handshake_display() -> TestResult<()> {
        // `quinn::ConnectionError::TimedOut` is a stable variant suitable for
        // constructing the error without an actual QUIC handshake.
        let err = H3Error::Handshake {
            source: quinn::ConnectionError::TimedOut,
        };
        let s = format!("{err}");
        assert!(
            s.contains("QUIC handshake failed"),
            "expected Display to mention 'QUIC handshake failed', got: {s}"
        );
        assert!(
            s.contains("TimedOut"),
            "expected Display to mention the underlying 'TimedOut', got: {s}"
        );
        Ok(())
    }

    /// (d) Calling `call` with an invalid (closed) endpoint returns an
    /// `H3Error` rather than panicking. The exact variant depends on
    /// `quinn::Endpoint::connect`'s failure mode when the endpoint has been
    /// closed — it returns `quinn::ConnectError::EndpointStopping`, which we
    /// surface as `H3Error::Other`.
    #[tokio::test]
    async fn quic_connector_call_with_closed_endpoint_returns_error() -> TestResult<()> {
        let endpoint = make_test_endpoint()?;
        // Close the endpoint so `endpoint.connect(...)` returns
        // `ConnectError::EndpointStopping` immediately. `quinn::Endpoint::close`
        // takes `(error_code: VarInt, reason: &[u8])` per its public API.
        endpoint.close(quinn::VarInt::from(0u32), &[]);

        let transport_config = Arc::new(TransportConfig::default());
        let tls_config = make_test_tls_config()?;
        let h3_options = Http3Options::default();

        let mut connector = QuicConnector::new(endpoint, transport_config, tls_config, h3_options);

        // Construct a `ConnectRequest` pointing at a loopback address.
        let uri: Uri = "https://127.0.0.1:443".parse()?;
        let req = ConnectRequest::new(uri, None);

        // `poll_ready` must be `Ok` per the behavioral contract.
        let waker = futures_util::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let poll = <QuicConnector as Service<ConnectRequest>>::poll_ready(&mut connector, &mut cx);
        match poll {
            Poll::Ready(Ok(())) => {}
            other => return Err(format!("poll_ready should be Ok, got {other:?}").into()),
        }

        // `call` must return an error (not panic). The exact variant is
        // `Other` because `quinn::ConnectError` is not a `ConnectionError`.
        let fut = connector.call(req);
        // Bound the wait so a regression that hangs the future fails the test
        // instead of stalling CI.
        let result = tokio::time::timeout(Duration::from_secs(2), fut).await;
        let conn_result = match result {
            Ok(r) => r,
            Err(_) => return Err("call future should resolve within 2s".into()),
        };
        assert!(
            conn_result.is_err(),
            "expected an error from call() with a closed endpoint"
        );
        // We do not assert the exact variant (Handshake vs Other) to keep the
        // test robust against `quinn::ConnectError` vs `ConnectionError`
        // mapping decisions; the key invariant is "no panic, returns Err".
        Ok(())
    }
}
