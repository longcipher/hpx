#[macro_use]
pub mod error;
mod exec;
pub mod extra;
mod lazy;
mod pool;
mod util;

use std::{
    future::Future,
    num::NonZeroU32,
    pin::Pin,
    sync::Arc,
    task::{self, Poll},
    time::Duration,
};

use bytes::Bytes;
use futures_util::future::{Either, FutureExt, TryFutureExt};
use http::{
    HeaderValue, Method, Request, Response, Uri, Version,
    header::{HOST, PROXY_AUTHORIZATION},
};
use http_body::Body;
use pool::Ver;
use tokio::io::{AsyncRead, AsyncWrite};
use tower::util::Oneshot;

use self::{
    error::{ClientConnectError, Error, ErrorKind, TrySendError},
    exec::Exec,
    extra::{ConnectExtra, ConnectIdentity},
    lazy::{Started as Lazy, lazy},
};
#[cfg(feature = "http3")]
use crate::client::conn::alt_svc::{AltSvcCache, H3FailureTracker, parse_alt_svc};
#[cfg(feature = "http3")]
use crate::client::conn::quic::H3Connection;
#[allow(unused_imports)]
use crate::client::core::conn::{self, TrySendError as ConnTrySendError};
#[cfg(feature = "http1")]
use crate::client::core::http1::Http1Options;
#[cfg(feature = "http2")]
use crate::client::core::http2::Http2Options;
use crate::{
    client::{
        conn::{Connected, Connection},
        core::{
            body::Incoming,
            rt::{ArcTimer, Executor, Timer},
        },
        layer::config::RequestOptions,
    },
    config::RequestConfig,
    error::BoxError,
    hash::{HASHER, HashMemo},
    tls::AlpnProtocol,
};

type BoxSendFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Parameters required to initiate a new connection.
///
/// [`ConnectRequest`] holds the target URI and all connection-specific options
/// (protocol, proxy, TCP/TLS settings) needed to establish a new network connection.
/// Used by connectors to drive the connection setup process.
#[must_use]
#[derive(Clone)]
pub struct ConnectRequest {
    uri: Uri,
    identifier: ConnectIdentity,
}

// ===== impl ConnectRequest =====

impl ConnectRequest {
    /// Create a new [`ConnectRequest`] with the given URI and identifier.
    ///
    /// `pub(crate)` so that connector-level tests (e.g. `QuicConnector` under
    /// `client/conn/quic.rs`) can synthesize a request without going through
    /// the full `HttpClient` pipeline.
    #[inline]
    pub(crate) fn new<T>(uri: Uri, identifier: T) -> ConnectRequest
    where
        T: Into<Option<RequestOptions>>,
    {
        ConnectRequest {
            uri: uri.clone(),
            identifier: Arc::new(HashMemo::with_hasher(
                ConnectExtra::new(uri, identifier),
                HASHER,
            )),
        }
    }

    /// Returns a reference to the [`Uri`].
    #[inline]
    pub fn uri(&self) -> &Uri {
        &self.uri
    }

    /// Returns a mutable reference to the [`Uri`].
    #[inline]
    pub fn uri_mut(&mut self) -> &mut Uri {
        &mut self.uri
    }

    /// Returns a unique [`ConnectIdentity`].
    #[inline]
    pub(crate) fn identify(&self) -> ConnectIdentity {
        self.identifier.clone()
    }

    /// Returns the [`ConnectExtra`] connection extra.
    #[inline]
    pub(crate) fn extra(&self) -> &ConnectExtra {
        self.identifier.as_ref().as_ref()
    }
}

/// A HttpClient to make outgoing HTTP requests.
///
/// `HttpClient` is cheap to clone and cloning is the recommended way to share a `HttpClient`. The
/// underlying connection pool will be reused.
#[must_use]
pub struct HttpClient<C, B> {
    config: Config,
    connector: C,
    exec: Exec,
    #[cfg(feature = "http1")]
    h1_builder: conn::http1::Builder,
    #[cfg(feature = "http2")]
    h2_builder: conn::http2::Builder<Exec>,
    /// HTTP/3 (QUIC) connector. `Some` when the `http3` Cargo feature is
    /// enabled AND the caller (typically `Client::build` or a test escape
    /// hatch) supplied a `QuicConnector`. When `Some` and the request's
    /// version resolves to `Ver::Http3`, `connect_to` routes through this
    /// connector instead of the TCP connector (`C`). When `None`, h3
    /// requests fail with `ErrorKind::UserUnsupportedVersion`.
    #[cfg(feature = "http3")]
    h3_connector: Option<crate::client::conn::quic::QuicConnector>,
    /// RFC 7838 Alt-Svc cache. Populated from `alt-svc` response headers
    /// on h2 (and later h1) responses. Used by the HTTP/3 upgrade path
    /// to discover QUIC endpoints for origins that advertise h3 support.
    #[cfg(feature = "http3")]
    alt_svc_cache: Arc<AltSvcCache>,
    /// Circuit breaker for HTTP/3 connection failures. Tracks per-authority
    /// QUIC handshake failures and blocks h3 retries for a cooldown period
    /// (default 60s). Shared across `HttpClient` clones via `Arc`.
    #[cfg(feature = "http3")]
    h3_failure_tracker: Arc<H3FailureTracker>,
    pool: pool::Pool<PoolClient<B>, ConnectIdentity>,
}

#[derive(Clone, Copy)]
struct Config {
    retry_canceled_requests: bool,
    set_host: bool,
    ver: Ver,
}

// ===== impl HttpClient =====

impl HttpClient<(), ()> {
    /// Create a builder to configure a new `HttpClient`.
    pub fn builder<E>(executor: E) -> Builder
    where
        E: Executor<BoxSendFuture> + Send + Sync + Clone + 'static,
    {
        Builder::new(executor)
    }
}

impl<C, B> HttpClient<C, B>
where
    C: tower::Service<ConnectRequest> + Clone + Send + Sync + 'static,
    C::Response: AsyncRead + AsyncWrite + Connection + Unpin + Send + 'static,
    C::Error: Into<BoxError>,
    C::Future: Unpin + Send + 'static,
    B: Body + Send + 'static + Unpin,
    B::Data: Send + Into<Bytes>,
    B::Error: Into<BoxError>,
{
    /// Send a constructed `Request` using this `HttpClient`.
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn request(&self, mut req: Request<B>) -> ResponseFuture {
        let is_http_connect = req.method() == Method::CONNECT;
        // Validate HTTP version early
        match req.version() {
            Version::HTTP_10 if is_http_connect => {
                warn!("CONNECT is not allowed for HTTP/1.0");
                return ResponseFuture::new(futures_util::future::err(Error::new_kind(
                    ErrorKind::UserUnsupportedRequestMethod,
                )));
            }
            Version::HTTP_10 | Version::HTTP_11 | Version::HTTP_2 => {}
            // HTTP/3 is allowed when the `http3` Cargo feature is enabled.
            // The actual routing to the QUIC connector happens in
            // `connect_to` (Ver::Http3 branch); if no `QuicConnector` is
            // wired in, `try_send_request` returns
            // `ErrorKind::UserUnsupportedVersion`.
            #[cfg(feature = "http3")]
            Version::HTTP_3 => {}
            // completely unsupported HTTP version (like HTTP/0.9)!
            _unsupported => {
                warn!("Request has unsupported version: {:?}", _unsupported);
                return ResponseFuture::new(futures_util::future::err(Error::new_kind(
                    ErrorKind::UserUnsupportedVersion,
                )));
            }
        };

        // Extract and normalize URI
        let uri = match util::normalize_uri(&mut req, is_http_connect) {
            Ok(uri) => uri,
            Err(err) => return ResponseFuture::new(futures_util::future::err(err)),
        };

        #[allow(unused_mut)]
        let mut this = self.clone();

        // Extract per-request options from the request extensions and apply them to the client
        // builder. This allows each request to override HTTP/1 and HTTP/2 options as
        // needed.
        let options = RequestConfig::<RequestOptions>::remove(req.extensions_mut());

        // Apply HTTP/1 and HTTP/2 options if provided
        #[allow(unused_variables)]
        if let Some(opts) = options.as_ref().map(RequestOptions::transport_opts) {
            #[cfg(feature = "http1")]
            if let Some(opts) = opts.http1_options() {
                this.h1_builder.options(opts.clone());
            }

            #[cfg(feature = "http2")]
            if let Some(opts) = opts.http2_options() {
                this.h2_builder.options(opts.clone());
            }
        }

        let connect_req = ConnectRequest::new(uri, options);
        ResponseFuture::new(this.send_request(req, connect_req))
    }

    async fn send_request(
        self,
        mut req: Request<B>,
        connect_req: ConnectRequest,
    ) -> Result<Response<Incoming>, Error> {
        let uri = req.uri().clone();

        // Track whether we've already attempted the h3 alt-svc upgrade
        // so we don't retry it on every loop iteration.
        #[cfg(feature = "http3")]
        let mut h3_attempted = false;

        loop {
            // Check Alt-Svc cache for HTTP/3 upgrade opportunity on the
            // first iteration of the loop. When the cache has a fresh h3
            // entry for the authority, try h3 first. If the h3 attempt
            // fails, fall through to the h2/h1 path below.
            #[cfg(feature = "http3")]
            {
                if !h3_attempted && self.config.ver == Ver::Auto && self.h3_connector.is_some() {
                    h3_attempted = true;
                    if let Some(host) = connect_req.uri().host() {
                        let port = connect_req.uri().port_u16().unwrap_or(443);
                        let authority = (host.to_string(), port);

                        // Circuit breaker: skip h3 if this authority had a
                        // recent QUIC failure (still in cooldown).
                        if self.h3_failure_tracker.is_blocked(&authority).await {
                            trace!(
                                "h3 circuit breaker: skipping h3 for {:?} (in cooldown)",
                                authority
                            );
                            // Fall through to h2/h1 below.
                        } else if let Some(entries) = self.alt_svc_cache.get(&authority).await {
                            let h3_entry = entries.iter().find(|e| e.protocol == "h3");
                            if let Some(h3_entry) = h3_entry {
                                let mut h3_connect_req = connect_req.clone();
                                let alt_svc_host = if h3_entry.host.is_empty() {
                                    host.to_string()
                                } else {
                                    h3_entry.host.clone()
                                };
                                let alt_svc_port = h3_entry.port;
                                let alt_svc_uri =
                                    format!("https://{}:{}", alt_svc_host, alt_svc_port);
                                if let Ok(new_uri) = alt_svc_uri.parse::<Uri>() {
                                    *h3_connect_req.uri_mut() = new_uri;
                                }

                                // Bypass the connection pool: call
                                // connect_h3 directly to create a fresh
                                // h3 connection instead of going through
                                // try_send_request which could return a
                                // cached h2 connection.
                                if let Some(h3_connector) = self.h3_connector.clone() {
                                    let pool = self.pool.clone();
                                    let lazy_conn =
                                        Self::connect_h3(Some(h3_connector), pool, h3_connect_req);
                                    match lazy_conn.await {
                                        Ok(mut pool_client) => {
                                            match pool_client.try_send_request(req).await {
                                                Ok(resp) => {
                                                    // Capture
                                                    // Alt-Svc
                                                    // headers
                                                    // from the
                                                    // h3
                                                    // response.
                                                    if let Some(host) = connect_req.uri().host() {
                                                        let port = connect_req
                                                            .uri()
                                                            .port_u16()
                                                            .unwrap_or(443);
                                                        if let Some(alt_svc_value) = resp
                                                            .headers()
                                                            .get(
                                                            http::header::HeaderName::from_static(
                                                                "alt-svc",
                                                            ),
                                                        ) && let Some(entries) =
                                                            parse_alt_svc(alt_svc_value)
                                                        {
                                                            self.alt_svc_cache
                                                                .insert(
                                                                    (host.to_string(), port),
                                                                    entries,
                                                                )
                                                                .await;
                                                        }
                                                    }
                                                    // Clear the circuit breaker
                                                    // on successful h3
                                                    // connection.
                                                    self.h3_failure_tracker.clear(&authority).await;
                                                    return Ok(resp);
                                                }
                                                Err(mut err) => {
                                                    if let Some(retry_req) = err.take_message() {
                                                        trace!(
                                                            "h3 alt-svc upgrade retryable, falling back to h2/h1"
                                                        );
                                                        // Record the
                                                        // failure so
                                                        // subsequent
                                                        // requests skip
                                                        // h3 for this
                                                        // authority.
                                                        self.h3_failure_tracker
                                                            .record_failure(authority.clone())
                                                            .await;
                                                        req = retry_req;
                                                        *req.uri_mut() = uri.clone();
                                                        // Fall
                                                        // through
                                                        // to
                                                        // h2/h1
                                                        // below.
                                                    } else {
                                                        let err = err.into_error();
                                                        trace!(
                                                            "h3 alt-svc upgrade failed irrecoverably (reason={:?})",
                                                            err
                                                        );
                                                        // Record the
                                                        // failure so
                                                        // subsequent
                                                        // requests skip
                                                        // h3 for this
                                                        // authority.
                                                        self.h3_failure_tracker
                                                            .record_failure(authority.clone())
                                                            .await;
                                                        return Err(Error::new(
                                                            ErrorKind::SendRequest,
                                                            err,
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                        Err(_err) => {
                                            trace!("h3 alt-svc upgrade failed (reason={:?})", _err);
                                            // Record the failure so
                                            // subsequent requests skip
                                            // h3 for this authority.
                                            self.h3_failure_tracker
                                                .record_failure(authority.clone())
                                                .await;
                                            // Fall through to h2/h1.
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            req = match self.try_send_request(req, connect_req.clone()).await {
                Ok(resp) => {
                    // Capture Alt-Svc headers for HTTP/3 upgrade discovery.
                    #[cfg(feature = "http3")]
                    {
                        if let Some(host) = connect_req.uri().host() {
                            let port = connect_req.uri().port_u16().unwrap_or(443);
                            if let Some(alt_svc_value) = resp
                                .headers()
                                .get(http::header::HeaderName::from_static("alt-svc"))
                                && let Some(entries) = parse_alt_svc(alt_svc_value)
                            {
                                self.alt_svc_cache
                                    .insert((host.to_string(), port), entries)
                                    .await;
                            }
                        }
                    }
                    return Ok(resp);
                }
                Err(TrySendError::Nope(err)) => return Err(err),
                Err(TrySendError::Retryable {
                    mut req,
                    error,
                    connection_reused,
                }) => {
                    if !self.config.retry_canceled_requests || !connection_reused {
                        // if client disabled, don't retry
                        // a fresh connection means we definitely can't retry
                        return Err(error);
                    }

                    trace!(
                        "unstarted request canceled, trying again (reason={:?})",
                        error
                    );
                    *req.uri_mut() = uri.clone();
                    req
                }
            }
        }
    }

    #[allow(clippy::result_large_err)]
    async fn try_send_request(
        &self,
        mut req: Request<B>,
        connect_req: ConnectRequest,
    ) -> Result<Response<Incoming>, TrySendError<B>> {
        let mut pooled = self
            .connection_for(connect_req)
            .await
            // `connection_for` already retries checkout errors, so if
            // it returns an error, there's not much else to retry
            .map_err(TrySendError::Nope)?;

        if pooled.is_http1() {
            if req.version() == Version::HTTP_2 {
                warn!("Connection is HTTP/1, but request requires HTTP/2");
                return Err(TrySendError::Nope(
                    Error::new_kind(ErrorKind::UserUnsupportedVersion)
                        .with_connect_info(pooled.conn_info.clone()),
                ));
            }

            if self.config.set_host {
                let uri = req.uri().clone();
                req.headers_mut().entry(HOST).or_insert_with(|| {
                    let hostname = uri.host().expect("authority implies host");
                    if let Some(port) = util::get_non_default_port(&uri) {
                        let s = format!("{hostname}:{port}");
                        HeaderValue::from_maybe_shared(Bytes::from(s))
                    } else {
                        HeaderValue::from_str(hostname)
                    }
                    .expect("uri host is valid header value")
                });
            }

            // CONNECT always sends authority-form, so check it first...
            if req.method() == Method::CONNECT {
                util::authority_form(req.uri_mut());
            } else if pooled.conn_info.is_proxied() {
                if let Some(auth) = pooled.conn_info.proxy_auth() {
                    req.headers_mut()
                        .entry(PROXY_AUTHORIZATION)
                        .or_insert_with(|| auth.clone());
                }

                if let Some(headers) = pooled.conn_info.proxy_headers() {
                    crate::util::replace_headers(req.headers_mut(), headers.clone());
                }

                util::absolute_form(req.uri_mut());
            } else {
                util::origin_form(req.uri_mut());
            }
        } else if req.method() == Method::CONNECT && !pooled.is_http2() {
            util::authority_form(req.uri_mut());
        }

        let mut res = match pooled.try_send_request(req).await {
            Ok(res) => res,
            Err(mut err) => {
                return if let Some(req) = err.take_message() {
                    Err(TrySendError::Retryable {
                        connection_reused: pooled.is_reused(),
                        error: Error::new(ErrorKind::Canceled, err.into_error())
                            .with_connect_info(pooled.conn_info.clone()),
                        req,
                    })
                } else {
                    Err(TrySendError::Nope(
                        Error::new(ErrorKind::SendRequest, err.into_error())
                            .with_connect_info(pooled.conn_info.clone()),
                    ))
                };
            }
        };

        // If the Connector included 'extra' info, add to Response...
        pooled.conn_info.set_extras(res.extensions_mut());

        // If pooled is HTTP/2, we can toss this reference immediately.
        //
        // when pooled is dropped, it will try to insert back into the
        // pool. To delay that, spawn a future that completes once the
        // sender is ready again.
        //
        // This *should* only be once the related `Connection` has polled
        // for a new request to start.
        //
        // It won't be ready if there is a body to stream.
        if pooled.is_http2() || !pooled.is_pool_enabled() || pooled.is_ready() {
            drop(pooled);
        } else {
            let on_idle = std::future::poll_fn(move |cx| pooled.poll_ready(cx)).map(|_| ());
            self.exec.execute(on_idle);
        }

        Ok(res)
    }

    async fn connection_for(
        &self,
        req: ConnectRequest,
    ) -> Result<pool::Pooled<PoolClient<B>, ConnectIdentity>, Error> {
        loop {
            match self.one_connection_for(req.clone()).await {
                Ok(pooled) => return Ok(pooled),
                Err(ClientConnectError::Normal(err)) => return Err(err),
                Err(ClientConnectError::CheckoutIsClosed(reason)) => {
                    if !self.config.retry_canceled_requests {
                        return Err(Error::new(ErrorKind::Connect, reason));
                    }

                    trace!(
                        "unstarted request canceled, trying again (reason={:?})",
                        reason,
                    );
                    continue;
                }
            };
        }
    }

    async fn one_connection_for(
        &self,
        req: ConnectRequest,
    ) -> Result<pool::Pooled<PoolClient<B>, ConnectIdentity>, ClientConnectError> {
        // Return a single connection if pooling is not enabled
        if !self.pool.is_enabled() {
            return self
                .connect_to(req)
                .await
                .map_err(ClientConnectError::Normal);
        }

        // This actually races 2 different futures to try to get a ready
        // connection the fastest, and to reduce connection churn.
        //
        // - If the pool has an idle connection waiting, that's used immediately.
        // - Otherwise, the Connector is asked to start connecting to the destination Uri.
        // - Meanwhile, the pool Checkout is watching to see if any other request finishes and tries
        //   to insert an idle connection.
        // - If a new connection is started, but the Checkout wins after (an idle connection became
        //   available first), the started connection future is spawned into the runtime to
        //   complete, and then be inserted into the pool as an idle connection.
        let checkout = self.pool.checkout(req.identify());
        let connect = self.connect_to(req);
        let is_ver_h2 = self.config.ver == Ver::Http2;

        // The order of the `select` is depended on below...

        match futures_util::future::select(checkout, connect).await {
            // Checkout won, connect future may have been started or not.
            //
            // If it has, let it finish and insert back into the pool,
            // so as to not waste the socket...
            Either::Left((Ok(checked_out), connecting)) => {
                // This depends on the `select` above having the correct
                // order, such that if the checkout future were ready
                // immediately, the connect future will never have been
                // started.
                //
                // If it *wasn't* ready yet, then the connect future will
                // have been started...
                if connecting.started() {
                    let bg = connecting
                        .map_err(|_err| {
                            trace!("background connect error: {}", _err);
                        })
                        .map(|_pooled| {
                            // dropping here should just place it in
                            // the Pool for us...
                        });
                    // An execute error here isn't important, we're just trying
                    // to prevent a waste of a socket...
                    self.exec.execute(bg);
                }
                Ok(checked_out)
            }
            // Connect won, checkout can just be dropped.
            Either::Right((Ok(connected), _checkout)) => Ok(connected),
            // Either checkout or connect could get canceled:
            //
            // 1. Connect is canceled if this is HTTP/2 and there is an outstanding HTTP/2
            //    connecting task.
            // 2. Checkout is canceled if the pool cannot deliver an idle connection reliably.
            //
            // In both cases, we should just wait for the other future.
            Either::Left((Err(err), connecting)) => {
                if err.is_canceled() {
                    connecting.await.map_err(ClientConnectError::Normal)
                } else {
                    Err(ClientConnectError::Normal(Error::new(
                        ErrorKind::Connect,
                        err,
                    )))
                }
            }
            Either::Right((Err(err), checkout)) => {
                if err.is_canceled() {
                    checkout.await.map_err(move |err| {
                        if is_ver_h2 && err.is_canceled() {
                            ClientConnectError::CheckoutIsClosed(err)
                        } else {
                            ClientConnectError::Normal(Error::new(ErrorKind::Connect, err))
                        }
                    })
                } else {
                    Err(ClientConnectError::Normal(err))
                }
            }
        }
    }

    fn connect_to(
        &self,
        req: ConnectRequest,
    ) -> impl Lazy<Output = Result<pool::Pooled<PoolClient<B>, ConnectIdentity>, Error>>
    + Send
    + Unpin
    + 'static {
        #[allow(unused_variables)]
        let executor = self.exec.clone();
        let pool = self.pool.clone();

        #[cfg(feature = "http1")]
        let h1_builder = self.h1_builder.clone();
        #[cfg(feature = "http2")]
        let h2_builder = self.h2_builder.clone();
        let ver = match req.extra().alpn_protocol() {
            Some(AlpnProtocol::HTTP2) => Ver::Http2,
            _ => self.config.ver,
        };
        let is_ver_h2 = ver == Ver::Http2;
        let connector = self.connector.clone();
        // Capture the optional `QuicConnector` for the Ver::Http3 branch.
        // When `None` (no connector wired in) and `ver == Ver::Http3`, the
        // request fails with `ErrorKind::UserUnsupportedVersion` from the
        // h3 branch below.
        #[cfg(feature = "http3")]
        let h3_connector = self.h3_connector.clone();
        lazy(move || {
            // ===== HTTP/3 (QUIC) branch =====
            //
            // When `Ver::Http3` is selected, route through the QUIC
            // connector instead of the TCP connector. The QUIC handshake
            // produces an `H3Connection` carrier that is stored in the
            // pool as `PoolTx::Http3`. Subsequent `try_send_request` calls
            // on the pooled connection drive the h3 request lifecycle via
            // `proto::h3::drive_request`.
            //
            // The h3 routing decision (including the
            // `UserUnsupportedVersion` fallback when no `QuicConnector` is
            // wired in) is encapsulated in [`Self::connect_h3`], which
            // returns a type-erased boxed future matching the closure's
            // return type. The TCP branch below produces the same boxed
            // future type, so the closure has a single concrete return
            // type without needing a per-branch cast at the call site.
            #[cfg(feature = "http3")]
            if ver == Ver::Http3 {
                return Self::connect_h3(h3_connector, pool, req);
            }

            // ===== TCP (HTTP/1 + HTTP/2) branch =====
            //
            // Try to take a "connecting lock".
            //
            // If the pool_key is for HTTP/2, and there is already a
            // connection being established, then this can't take a
            // second lock. The "connect_to" future is Canceled.
            let connecting = match pool.connecting(req.identify(), ver) {
                Some(lock) => lock,
                None => {
                    let canceled = Error::new_kind(ErrorKind::Canceled);
                    // HTTP/2 connection in progress.
                    return Box::pin(futures_util::future::err(canceled))
                        as Pin<
                            Box<
                                dyn Future<
                                        Output = Result<
                                            pool::Pooled<PoolClient<B>, ConnectIdentity>,
                                            Error,
                                        >,
                                    > + Send
                                    + 'static,
                            >,
                        >;
                }
            };
            Box::pin(
                Oneshot::new(connector, req)
                    .map_err(|src| Error::new(ErrorKind::Connect, src))
                    .and_then(move |io| {
                        let connected = io.connected();
                        // If ALPN is h2 and we aren't http2_only already,
                        // then we need to convert our pool checkout into
                        #[allow(unused_variables)]
                        let connecting = if connected.is_negotiated_h2() && !is_ver_h2 {
                            match connecting.alpn_h2(&pool) {
                                Some(lock) => {
                                    trace!("ALPN negotiated h2, updating pool");
                                    lock
                                }
                                None => {
                                    // Another connection has already upgraded,
                                    // the pool checkout should finish up for us.
                                    let canceled =Error::new(ErrorKind::Canceled, "ALPN upgraded to HTTP/2");
                                    return Either::Right(futures_util::future::err(canceled));
                                }
                            }
                        } else {
                            connecting
                        };

                        let is_h2 = is_ver_h2 || connected.is_negotiated_h2();

                        Either::Left(Box::pin(async move {
                            let _ = (&pool, &connected, &connecting);
                            #[allow(unused_variables)]
                            let tx = if is_h2 {
                                #[cfg(feature = "http2")]
                                {
                                    let (mut tx, conn) =
                                        h2_builder.handshake(io).await.map_err(Error::tx)?;

                                    trace!(
                                        "http2 handshake complete, spawning background dispatcher task"
                                    );
                                    executor.execute(
                                        conn.map_err(|_e| debug!("client connection error: {}", _e))
                                            .map(|_| ()),
                                    );

                                    // Wait for 'conn' to ready up before we
                                    // declare this tx as usable
                                    tx.ready().await.map_err(Error::tx)?;
                                    PoolTx::Http2(tx)
                                }
                                #[cfg(not(feature = "http2"))]
                                {
                                    return Err(Error::new(ErrorKind::UserUnsupportedVersion, "HTTP/2 not supported"));
                                }
                            } else {
                                #[cfg(feature = "http1")]
                                {
                                    // Perform the HTTP/1.1 handshake on the provided I/O stream. More actions
                                    // Uses the h1_builder to establish a connection, returning a sender (tx) for requests
                                    // and a connection task (conn) that manages the connection lifecycle.
                                    let (mut tx, conn) =
                                        h1_builder.handshake(io).await.map_err(Error::tx)?;

                                    // Log that the HTTP/1.1 handshake has completed successfully.
                                    // This indicates the connection is established and ready for request processing.
                                    trace!(
                                        "http1 handshake complete, spawning background dispatcher task"
                                    );

                                    // Create a oneshot channel to communicate errors from the connection task.
                                    // err_tx sends errors from the connection task, and err_rx receives them
                                    // to correlate connection failures with request readiness errors.
                                    let (err_tx, err_rx) = tokio::sync::oneshot::channel();
                                    // Spawn the connection task in the background using the executor.
                                    // The task manages the HTTP/1.1 connection, including upgrades (e.g., WebSocket).
                                    // Errors are sent via err_tx to ensure they can be checked if the sender (tx) fails.
                                    executor.execute(
                                        conn.with_upgrades()
                                                .map_err(|e| {
                                                // Log the connection error at debug level for diagnostic purposes.
                                                debug!("client connection error: {:?}", e);
                                                // Log that the error is being sent to the error channel.
                                                trace!("sending connection error to error channel");
                                                // Send the error via the oneshot channel, ignoring send failures
                                                // (e.g., if the receiver is dropped, which is handled later).
                                                let _ = err_tx.send(e);
                                            })
                                            .map(|_| ()),
                                    );

                                    // Log that the client is waiting for the connection to be ready.
                                    // Readiness indicates the sender (tx) can accept a request without blocking. More actions
                                    trace!("waiting for connection to be ready");

                                    // Check if the sender is ready to accept a request.
                                    // This ensures the connection is fully established before proceeding.
                                    // Wait for 'conn' to ready up before we
                                    // declare this tx as usable
                                    match tx.ready().await {
                                        // If ready, the connection is usable for sending requests.
                                        Ok(_) => {
                                            // Log that the connection is ready for use.
                                            trace!("connection is ready");
                                            // Drop the error receiver, as it’s no longer needed since the sender is ready.
                                            // This prevents waiting for errors that won’t occur in a successful case.
                                            drop(err_rx);
                                            // Wrap the sender in PoolTx::Http1 for use in the connection pool.
                                            PoolTx::Http1(tx)
                                        }
                                        // If the sender fails with a closed channel error, check for a specific connection error.
                                        // This distinguishes between a vague ChannelClosed error and an actual connection failure.
                                        Err(e) if e.is_closed() => {
                                            // Log that the channel is closed, indicating a potential connection issue.
                                            trace!("connection channel closed, checking for connection error");
                                            // Check the oneshot channel for a specific error from the connection task.
                                            match err_rx.await {
                                                // If an error was received, it’s a specific connection failure.
                                                Ok(err) => {
                                                     // Log the specific connection error for diagnostics.
                                                    trace!("received connection error: {:?}", err);
                                                    // Return the error wrapped in Error::tx to propagate it.
                                                    return Err(Error::tx(err));
                                                }
                                                // If the error channel is closed, no specific error was sent.
                                                // Fall back to the vague ChannelClosed error.
                                                Err(_) => {
                                                    // Log that the error channel is closed, indicating no specific error.
                                                    trace!("error channel closed, returning the vague ChannelClosed error");
                                                    // Return the original error wrapped in Error::tx.
                                                    return Err(Error::tx(e));
                                                }
                                            }
                                        }
                                        // For other errors (e.g., timeout, I/O issues), propagate them directly.
                                        // These are not ChannelClosed errors and don’t require error channel checks.
                                        Err(e) => {
                                            // Log the specific readiness failure for diagnostics.
                                            trace!("connection readiness failed: {:?}", e);
                                            // Return the error wrapped in Error::tx to propagate it.
                                            return Err(Error::tx(e));
                                        }
                                    }
                                }
                                #[cfg(not(feature = "http1"))]
                                {
                                    return Err(Error::new(ErrorKind::UserUnsupportedVersion, "HTTP/1 not supported"));
                                }
                            };

                            #[allow(unreachable_code)]
                            Ok(pool.pooled(
                                connecting,
                                PoolClient {
                                    conn_info: connected,
                                    tx,
                                    _marker: std::marker::PhantomData,
                                },
                            ))
                        }))
                    }),
            )
                as Pin<
                    Box<
                        dyn Future<
                            Output = Result<
                                pool::Pooled<PoolClient<B>, ConnectIdentity>,
                                Error,
                            >,
                        > + Send
                        + 'static,
                    >,
                >
        })
    }

    /// Drive the HTTP/3 (QUIC) connect path and return a pooled connection.
    ///
    /// This is the h3 routing helper extracted from [`Self::connect_to`].
    /// It is invoked when the request's resolved version is `Ver::Http3`.
    ///
    /// Behavior:
    /// - If `h3_connector` is `Some`, acquire the pool's "connecting" lock
    ///   for `Ver::Http3` (dedups concurrent connect attempts per C-05),
    ///   drive the [`QuicConnector`] via [`Oneshot`] to obtain an
    ///   [`H3Connection`], wrap it as `PoolTx::Http3`, and insert into the
    ///   pool.
    /// - If `h3_connector` is `None`, fail fast with
    ///   `ErrorKind::UserUnsupportedVersion`. This is the same error
    ///   returned by the request-validation match in [`Self::request`] when
    ///   the `http3` feature is disabled, so callers see a consistent error
    ///   regardless of which guard fires first.
    ///
    /// The returned boxed future's type matches the TCP branch's boxed
    /// future type in [`Self::connect_to`], so the `lazy` closure has a
    /// single concrete return type without a per-branch cast at the call
    /// site.
    #[cfg(feature = "http3")]
    #[allow(clippy::type_complexity)]
    fn connect_h3(
        h3_connector: Option<crate::client::conn::quic::QuicConnector>,
        pool: pool::Pool<PoolClient<B>, ConnectIdentity>,
        req: ConnectRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<pool::Pooled<PoolClient<B>, ConnectIdentity>, Error>>
                + Send
                + 'static,
        >,
    > {
        match h3_connector {
            Some(h3_connector) => Box::pin(async move {
                // Acquire the pool's "connecting" lock for Ver::Http3.
                // The pool's `connecting` method dedups concurrent connect
                // attempts for HTTP/2 and HTTP/3 (C-05).
                let connecting = match pool.connecting(req.identify(), Ver::Http3) {
                    Some(lock) => lock,
                    None => {
                        return Err(Error::new_kind(ErrorKind::Canceled));
                    }
                };
                // Drive the QuicConnector to obtain an H3Connection.
                // `Oneshot::new` consumes the connector and request and
                // resolves to `Result<H3Connection, H3Error>`.
                let h3_conn = Oneshot::new(h3_connector, req)
                    .await
                    .map_err(|src| Error::new(ErrorKind::Connect, src))?;
                // Wrap in a PoolClient and insert into the pool.
                // `Connected::new()` is sufficient for h3 — the response
                // version is set by h3 itself (HTTP_3), not derived from
                // `Connected::alpn` (which has no H3 variant).
                Ok(pool.pooled(
                    connecting,
                    PoolClient {
                        conn_info: Connected::new(),
                        tx: PoolTx::Http3(h3_conn),
                        _marker: std::marker::PhantomData,
                    },
                ))
            }),
            None => Box::pin(futures_util::future::err(Error::new(
                ErrorKind::UserUnsupportedVersion,
                "HTTP/3 requested but no QuicConnector wired in",
            ))),
        }
    }
}

// ===== Alt-Svc cache accessors =====

impl<C, B> HttpClient<C, B> {
    /// Returns a reference to the [`AltSvcCache`].
    #[cfg(feature = "http3")]
    #[allow(dead_code)] // HTTP/3 Alt-Svc accessor; wired in by future tasks.
    pub(crate) fn alt_svc_cache(&self) -> &Arc<AltSvcCache> {
        &self.alt_svc_cache
    }

    /// Test helper: checks whether the Alt-Svc cache has a non-expired
    /// entry for the given authority `(host, port)`.
    ///
    /// `#[doc(hidden)]` — exposed for integration tests only
    /// (`crates/hpx/tests/http3.rs`). Not part of the stable public API.
    #[cfg(feature = "http3")]
    #[doc(hidden)]
    pub async fn __test_alt_svc_cache_has_entry(&self, host: &str, port: u16) -> bool {
        self.alt_svc_cache
            .get(&(host.to_string(), port))
            .await
            .is_some()
    }

    /// Test helper: checks whether the H3 failure tracker has blocked
    /// the given authority `(host, port)`.
    ///
    /// `#[doc(hidden)]` — exposed for integration tests only
    /// (`crates/hpx/tests/http3.rs`). Not part of the stable public API.
    #[cfg(feature = "http3")]
    #[doc(hidden)]
    pub async fn __test_h3_failure_tracker_is_blocked(&self, host: &str, port: u16) -> bool {
        self.h3_failure_tracker
            .is_blocked(&(host.to_string(), port))
            .await
    }
}

impl<C, B> tower::Service<Request<B>> for HttpClient<C, B>
where
    C: tower::Service<ConnectRequest> + Clone + Send + Sync + 'static,
    C::Response: AsyncRead + AsyncWrite + Connection + Unpin + Send + 'static,
    C::Error: Into<BoxError>,
    C::Future: Unpin + Send + 'static,
    B: Body + Send + 'static + Unpin,
    B::Data: Send + Into<Bytes>,
    B::Error: Into<BoxError>,
{
    type Response = Response<Incoming>;
    type Error = Error;
    type Future = ResponseFuture;

    fn poll_ready(&mut self, _: &mut task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        self.request(req)
    }
}

impl<C, B> tower::Service<Request<B>> for &'_ HttpClient<C, B>
where
    C: tower::Service<ConnectRequest> + Clone + Send + Sync + 'static,
    C::Response: AsyncRead + AsyncWrite + Connection + Unpin + Send + 'static,
    C::Error: Into<BoxError>,
    C::Future: Unpin + Send + 'static,
    B: Body + Send + 'static + Unpin,
    B::Data: Send + Into<Bytes>,
    B::Error: Into<BoxError>,
{
    type Response = Response<Incoming>;
    type Error = Error;
    type Future = ResponseFuture;

    fn poll_ready(&mut self, _: &mut task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        self.request(req)
    }
}

impl<C: Clone, B> Clone for HttpClient<C, B> {
    fn clone(&self) -> HttpClient<C, B> {
        HttpClient {
            config: self.config,
            exec: self.exec.clone(),
            #[cfg(feature = "http1")]
            h1_builder: self.h1_builder.clone(),
            #[cfg(feature = "http2")]
            h2_builder: self.h2_builder.clone(),
            connector: self.connector.clone(),
            #[cfg(feature = "http3")]
            h3_connector: self.h3_connector.clone(),
            #[cfg(feature = "http3")]
            alt_svc_cache: self.alt_svc_cache.clone(),
            #[cfg(feature = "http3")]
            h3_failure_tracker: self.h3_failure_tracker.clone(),
            pool: self.pool.clone(),
        }
    }
}

/// A pooled HTTP connection that can send requests
struct PoolClient<B> {
    conn_info: Connected,
    tx: PoolTx<B>,
    _marker: std::marker::PhantomData<B>,
}

enum PoolTx<B> {
    #[cfg(feature = "http1")]
    Http1(conn::http1::SendRequest<B>),
    #[cfg(feature = "http2")]
    Http2(conn::http2::SendRequest<B>),
    /// HTTP/3 connection carrier. Gated on the `http3` Cargo feature per [C-01].
    /// Uses `Shared` reservation semantics (one connection per authority serves
    /// many streams), mirroring `Http2` (C-05). `H3Connection` does not carry
    /// the `B` type parameter (it uses `Bytes` internally via
    /// `hpx_h3::client::SendRequest<hpx_h3_quinn::OpenStreams, Bytes>`); the `B` on
    /// `PoolTx<B>` is preserved for the `Http1`/`Http2` variants and the
    /// `None` fallback.
    #[cfg(feature = "http3")]
    Http3(H3Connection),
    #[cfg(not(any(feature = "http1", feature = "http2")))]
    None(std::marker::PhantomData<B>),
}

// ===== impl PoolClient =====

impl<B> PoolClient<B> {
    #[allow(unused_variables)]
    fn poll_ready(&mut self, cx: &mut task::Context<'_>) -> Poll<Result<(), Error>> {
        match self.tx {
            #[cfg(feature = "http1")]
            PoolTx::Http1(ref mut tx) => tx.poll_ready(cx).map_err(Error::closed),
            #[cfg(feature = "http2")]
            PoolTx::Http2(_) => Poll::Ready(Ok(())),
            // HTTP/3 connections are always ready: `H3Connection::send_request`
            // is `Clone` and multiplexes streams over a single QUIC connection
            // (no per-stream readiness gate). Pool validity is checked via
            // `is_open` -> `H3Connection::is_valid`.
            #[cfg(feature = "http3")]
            PoolTx::Http3(_) => Poll::Ready(Ok(())),
            #[cfg(not(any(feature = "http1", feature = "http2")))]
            PoolTx::None(_) => Poll::Ready(Ok(())),
        }
    }

    fn is_http1(&self) -> bool {
        !self.is_http2() && !self.is_http3()
    }

    fn is_http2(&self) -> bool {
        match self.tx {
            #[cfg(feature = "http1")]
            PoolTx::Http1(_) => false,
            #[cfg(feature = "http2")]
            PoolTx::Http2(_) => true,
            #[cfg(feature = "http3")]
            PoolTx::Http3(_) => false,
            #[cfg(not(any(feature = "http1", feature = "http2")))]
            PoolTx::None(_) => false,
        }
    }

    /// Returns `true` if this pooled connection is an HTTP/3 (QUIC) connection.
    /// Mirrors `is_http2()`; used by `can_share()` to apply `Shared` reservation
    /// semantics for both HTTP/2 and HTTP/3 (C-05).
    fn is_http3(&self) -> bool {
        match self.tx {
            #[cfg(feature = "http1")]
            PoolTx::Http1(_) => false,
            #[cfg(feature = "http2")]
            PoolTx::Http2(_) => false,
            #[cfg(feature = "http3")]
            PoolTx::Http3(_) => true,
            #[cfg(not(any(feature = "http1", feature = "http2")))]
            PoolTx::None(_) => false,
        }
    }

    fn is_poisoned(&self) -> bool {
        self.conn_info.poisoned()
    }

    fn is_ready(&self) -> bool {
        match self.tx {
            #[cfg(feature = "http1")]
            PoolTx::Http1(ref tx) => tx.is_ready(),
            #[cfg(feature = "http2")]
            PoolTx::Http2(ref tx) => tx.is_ready(),
            // `H3Connection` is always "ready" in the h2 sense: stream
            // multiplexing means there's no per-connection readiness gate.
            // Pool validity is checked separately via `is_open`.
            #[cfg(feature = "http3")]
            PoolTx::Http3(_) => true,
            #[cfg(not(any(feature = "http1", feature = "http2")))]
            PoolTx::None(_) => true,
        }
    }
}

impl<B: Body + 'static> PoolClient<B> {
    #[allow(clippy::result_large_err)]
    fn try_send_request(
        &mut self,
        req: Request<B>,
    ) -> impl Future<Output = Result<Response<Incoming>, ConnTrySendError<Request<B>>>>
    where
        B: Send + Unpin,
        B::Data: Into<Bytes> + Send,
        B::Error: Into<crate::client::core::BoxError>,
    {
        // When `http3` is enabled, all match arms must agree on a single
        // future type. The h1/h2 arms are wrapped in an outer `Either::Left`
        // (which itself may contain an inner `Either` for the h1/h2 split),
        // and the h3 arm returns `Either::Right(std::future::ready(Err(...)))`
        // with a typed error indicating h3 request driving is T1.9 scope.
        //
        // When `http3` is disabled, the original h1/h2/`None` matching is
        // preserved unchanged (no extra `Either` wrapping, no h3 arm).
        #[cfg(feature = "http3")]
        {
            match self.tx {
                #[cfg(feature = "http1")]
                PoolTx::Http1(ref mut tx) => {
                    #[cfg(feature = "http2")]
                    {
                        Either::Left(Either::Left(tx.try_send_request(req)))
                    }
                    #[cfg(not(feature = "http2"))]
                    {
                        Either::Left(tx.try_send_request(req))
                    }
                }
                #[cfg(feature = "http2")]
                PoolTx::Http2(ref mut tx) => {
                    #[cfg(feature = "http1")]
                    {
                        Either::Left(Either::Right(tx.try_send_request(req)))
                    }
                    #[cfg(not(feature = "http1"))]
                    {
                        Either::Left(tx.try_send_request(req))
                    }
                }
                PoolTx::Http3(ref conn) => {
                    // Clone the `SendRequest` (h3-quinn's `OpenStreams: Clone`)
                    // and drive the request lifecycle in a boxed future. The
                    // cast to `Pin<Box<dyn Future<...> + Send>>` unifies the
                    // h3 branch's future type with the h1/h2 branches' future
                    // type (`Either::Left(...)`) so the match has a single
                    // concrete return type.
                    let mut send_request = conn.send_request.clone();
                    #[cfg(any(feature = "http1", feature = "http2"))]
                    {
                        Either::Right(Box::pin(async move {
                            crate::client::core::http3::drive_request(&mut send_request, req)
                                .await
                                .map_err(|(error, message)| ConnTrySendError { error, message })
                        })
                            as Pin<
                                Box<
                                    dyn Future<
                                            Output = Result<
                                                Response<Incoming>,
                                                ConnTrySendError<Request<B>>,
                                            >,
                                        > + Send,
                                >,
                            >)
                    }
                    #[cfg(not(any(feature = "http1", feature = "http2")))]
                    {
                        Box::pin(async move {
                            crate::client::core::http3::drive_request(&mut send_request, req)
                                .await
                                .map_err(|(error, message)| ConnTrySendError { error, message })
                        })
                            as Pin<
                                Box<
                                    dyn Future<
                                            Output = Result<
                                                Response<Incoming>,
                                                ConnTrySendError<Request<B>>,
                                            >,
                                        > + Send,
                                >,
                            >
                    }
                }
                #[cfg(not(any(feature = "http1", feature = "http2")))]
                PoolTx::None(_) => {
                    #[cfg(any(feature = "http1", feature = "http2"))]
                    {
                        Either::Right(std::future::ready(Err(ConnTrySendError {
                            error: crate::client::core::Error::new_user_service(Error::new(
                                ErrorKind::UserUnsupportedVersion,
                                "No HTTP version enabled",
                            )),
                            message: Some(req),
                        })))
                    }
                    #[cfg(not(any(feature = "http1", feature = "http2")))]
                    {
                        Box::pin(std::future::ready(Err(ConnTrySendError {
                            error: crate::client::core::Error::new_user_service(Error::new(
                                ErrorKind::UserUnsupportedVersion,
                                "No HTTP version enabled",
                            )),
                            message: Some(req),
                        })))
                            as Pin<
                                Box<
                                    dyn Future<
                                            Output = Result<
                                                Response<Incoming>,
                                                ConnTrySendError<Request<B>>,
                                            >,
                                        > + Send,
                                >,
                            >
                    }
                }
            }
        }
        #[cfg(not(feature = "http3"))]
        {
            match self.tx {
                #[cfg(feature = "http1")]
                PoolTx::Http1(ref mut tx) => {
                    #[cfg(feature = "http2")]
                    {
                        Either::Left(tx.try_send_request(req))
                    }
                    #[cfg(not(feature = "http2"))]
                    {
                        tx.try_send_request(req)
                    }
                }
                #[cfg(feature = "http2")]
                PoolTx::Http2(ref mut tx) => {
                    #[cfg(feature = "http1")]
                    {
                        Either::Right(tx.try_send_request(req))
                    }
                    #[cfg(not(feature = "http1"))]
                    {
                        tx.try_send_request(req)
                    }
                }
                #[cfg(not(any(feature = "http1", feature = "http2")))]
                PoolTx::None(_) => std::future::ready(Err(ConnTrySendError {
                    error: crate::client::core::Error::new_user_service(Error::new(
                        ErrorKind::UserUnsupportedVersion,
                        "No HTTP version enabled",
                    )),
                    message: Some(req),
                })),
            }
        }
    }
}

impl<B> pool::Poolable for PoolClient<B>
where
    B: Send + 'static + Unpin,
{
    fn is_open(&self) -> bool {
        if self.is_poisoned() {
            return false;
        }
        // For HTTP/3, delegate to `H3Connection::is_valid` which checks the
        // `is_broken` flag (set by the driver task when `poll_close` resolves)
        // and the idle timeout. `Duration::MAX` skips the idle check because
        // the pool's `Expiration` handles idle eviction separately. For other
        // variants, fall back to `is_ready()` (existing behavior).
        #[cfg(feature = "http3")]
        {
            if let PoolTx::Http3(ref conn) = self.tx {
                return conn.is_valid(Duration::MAX);
            }
        }
        self.is_ready()
    }

    fn reserve(self) -> pool::Reservation<Self> {
        match self.tx {
            #[cfg(feature = "http1")]
            PoolTx::Http1(tx) => pool::Reservation::Unique(PoolClient {
                conn_info: self.conn_info,
                tx: PoolTx::Http1(tx),
                _marker: std::marker::PhantomData,
            }),
            #[cfg(feature = "http2")]
            PoolTx::Http2(tx) => {
                let b = PoolClient {
                    conn_info: self.conn_info.clone(),
                    tx: PoolTx::Http2(tx.clone()),
                    _marker: std::marker::PhantomData,
                };
                let a = PoolClient {
                    conn_info: self.conn_info,
                    tx: PoolTx::Http2(tx),
                    _marker: std::marker::PhantomData,
                };
                pool::Reservation::Shared(a, b)
            }
            // HTTP/3 uses `Shared` reservation semantics (C-05): one QUIC
            // connection per authority serves many concurrent streams. We
            // clone the `send_request` (h3-quinn's `OpenStreams: Clone`),
            // `is_broken` (Arc<AtomicBool>), and `idle_at` (Copy) into a
            // second `H3Connection`. The original `close_rx` stays with
            // copy `a` (the pool's retained copy); copy `b` (returned to
            // the caller) gets a closed dummy channel (sender dropped
            // immediately). This is correct for T1.7 because `is_valid`
            // only reads `is_broken`, not `close_rx`; the terminal error
            // retrieval via `close_rx` is T1.9 scope.
            #[cfg(feature = "http3")]
            PoolTx::Http3(conn) => {
                let b = PoolClient {
                    conn_info: self.conn_info.clone(),
                    tx: PoolTx::Http3(H3Connection {
                        send_request: conn.send_request.clone(),
                        close_rx: {
                            let (close_tx, close_rx) = tokio::sync::mpsc::channel(1);
                            drop(close_tx);
                            close_rx
                        },
                        idle_at: conn.idle_at,
                        is_broken: conn.is_broken.clone(),
                        used_0rtt: conn.used_0rtt.clone(),
                    }),
                    _marker: std::marker::PhantomData,
                };
                let a = PoolClient {
                    conn_info: self.conn_info,
                    tx: PoolTx::Http3(conn),
                    _marker: std::marker::PhantomData,
                };
                pool::Reservation::Shared(a, b)
            }
            #[cfg(not(any(feature = "http1", feature = "http2")))]
            PoolTx::None(_) => pool::Reservation::Unique(PoolClient {
                conn_info: self.conn_info,
                tx: PoolTx::None(std::marker::PhantomData),
                _marker: std::marker::PhantomData,
            }),
        }
    }

    fn can_share(&self) -> bool {
        // HTTP/2 and HTTP/3 both support stream multiplexing over a single
        // connection, so both use `Shared` reservation semantics (C-05).
        self.is_http2() || self.is_http3()
    }
}

/// A `Future` that will resolve to an HTTP Response.
#[must_use = "futures do nothing unless polled"]
pub struct ResponseFuture {
    inner: Pin<Box<dyn Future<Output = Result<Response<Incoming>, Error>> + Send>>,
}

// ===== impl ResponseFuture =====

impl ResponseFuture {
    #[inline]
    pub(super) fn new<F>(value: F) -> ResponseFuture
    where
        F: Future<Output = Result<Response<Incoming>, Error>> + Send + 'static,
    {
        ResponseFuture {
            inner: Box::pin(value),
        }
    }
}

impl Future for ResponseFuture {
    type Output = Result<Response<Incoming>, Error>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        self.inner.as_mut().poll(cx)
    }
}

/// A builder to configure a new [`HttpClient`].
#[derive(Clone)]
pub struct Builder {
    config: Config,
    exec: Exec,

    #[cfg(feature = "http1")]
    h1_builder: conn::http1::Builder,
    #[cfg(feature = "http2")]
    h2_builder: conn::http2::Builder<Exec>,
    /// HTTP/3 (QUIC) connector. `Some` when the caller (typically
    /// `Client::build` or a test escape hatch) supplied a `QuicConnector`.
    /// Passed through to `HttpClient` verbatim by `Builder::build`.
    #[cfg(feature = "http3")]
    h3_connector: Option<crate::client::conn::quic::QuicConnector>,
    /// Pre-built Alt-Svc cache. When `Some`, used directly instead of
    /// creating a new one. Set via `Builder::alt_svc_cache` by
    /// `ClientBuilder::build` so the cache can be shared with `Client`.
    #[cfg(feature = "http3")]
    alt_svc_cache: Option<Arc<AltSvcCache>>,
    /// Pre-built H3 failure tracker. When `Some`, used directly instead
    /// of creating a new one with the default 60s cooldown.
    #[cfg(feature = "http3")]
    h3_failure_tracker: Option<Arc<H3FailureTracker>>,
    pool_config: pool::Config,
    pool_timer: Option<ArcTimer>,
}

// ===== impl Builder =====

impl Builder {
    /// Construct a new Builder.
    pub fn new<E>(executor: E) -> Self
    where
        E: Executor<BoxSendFuture> + Send + Sync + Clone + 'static,
    {
        let exec = Exec::new(executor);
        Self {
            config: Config {
                retry_canceled_requests: true,
                set_host: true,
                ver: Ver::Auto,
            },
            exec: exec.clone(),

            #[cfg(feature = "http1")]
            h1_builder: conn::http1::Builder::new(),
            #[cfg(feature = "http2")]
            h2_builder: conn::http2::Builder::new(exec),
            #[cfg(feature = "http3")]
            h3_connector: None,
            #[cfg(feature = "http3")]
            alt_svc_cache: None,
            #[cfg(feature = "http3")]
            h3_failure_tracker: None,
            pool_config: pool::Config {
                idle_timeout: Some(Duration::from_secs(90)),
                max_idle_per_host: usize::MAX,
                max_pool_size: None,
            },
            pool_timer: None,
        }
    }
    /// Set an optional timeout for idle sockets being kept-alive.
    /// A `Timer` is required for this to take effect. See `Builder::pool_timer`
    ///
    /// Pass `None` to disable timeout.
    ///
    /// Default is 90 seconds.
    #[inline]
    pub fn pool_idle_timeout<D>(mut self, val: D) -> Self
    where
        D: Into<Option<Duration>>,
    {
        self.pool_config.idle_timeout = val.into();
        self
    }

    /// Sets the maximum idle connection per host allowed in the pool.
    ///
    /// Default is `usize::MAX` (no limit).
    #[inline]
    pub fn pool_max_idle_per_host(mut self, max_idle: usize) -> Self {
        self.pool_config.max_idle_per_host = max_idle;
        self
    }

    /// Sets the maximum number of connections in the pool.
    ///
    /// Default is `None` (no limit).
    #[inline]
    pub fn pool_max_size(mut self, max_size: impl Into<Option<NonZeroU32>>) -> Self {
        self.pool_config.max_pool_size = max_size.into();
        self
    }

    /// Set whether the connection **must** use HTTP/2.
    ///
    /// The destination must either allow HTTP2 Prior Knowledge, or the
    /// `Connect` should be configured to do use ALPN to upgrade to `h2`
    /// as part of the connection process. This will not make the `HttpClient`
    /// utilize ALPN by itself.
    ///
    /// Note that setting this to true prevents HTTP/1 from being allowed.
    ///
    /// Default is false.
    #[inline]
    pub fn http2_only(mut self, val: bool) -> Self {
        self.config.ver = if val { Ver::Http2 } else { Ver::Auto };
        self
    }

    /// Set whether the connection **must** use HTTP/3 over QUIC.
    ///
    /// When `true`, requests are routed through the configured `QuicConnector`
    /// (set via [`Builder::h3_connector`]) instead of the TCP connector, and
    /// `connect_to` takes the `Ver::Http3` branch. When `false`, the version
    /// falls back to `Ver::Auto` (TCP ALPN-based selection between HTTP/1 and
    /// HTTP/2).
    ///
    /// When `true` but no `QuicConnector` is wired in, h3 requests fail at
    /// runtime with `ErrorKind::UserUnsupportedVersion` ("HTTP/3 requested
    /// but no QuicConnector wired in") from the `Ver::Http3` branch of
    /// `connect_to`.
    ///
    /// Default is false.
    #[cfg(feature = "http3")]
    #[inline]
    pub fn http3_only(mut self, val: bool) -> Self {
        self.config.ver = if val { Ver::Http3 } else { Ver::Auto };
        self
    }

    /// Provide a timer to be used for http2
    ///
    /// See the documentation of [`http2::client::Builder::timer`] for more
    /// details.
    ///
    /// [`http2::client::Builder::timer`]: https://docs.rs/http2/latest/http2/client/struct.Builder.html#method.timer
    #[cfg(feature = "http2")]
    #[inline]
    pub fn http2_timer<M>(mut self, timer: M) -> Self
    where
        M: Timer + Send + Sync + 'static,
    {
        self.h2_builder.timer(timer);
        self
    }

    /// Provide a configuration for HTTP/1.
    #[cfg(feature = "http1")]
    #[inline]
    pub fn http1_options<O>(mut self, opts: O) -> Self
    where
        O: Into<Option<Http1Options>>,
    {
        if let Some(opts) = opts.into() {
            self.h1_builder.options(opts);
        }

        self
    }

    /// Provide a configuration for HTTP/2.
    #[cfg(feature = "http2")]
    #[inline]
    pub fn http2_options<O>(mut self, opts: O) -> Self
    where
        O: Into<Option<Http2Options>>,
    {
        if let Some(opts) = opts.into() {
            self.h2_builder.options(opts);
        }
        self
    }

    /// Provide a timer to be used for timeouts and intervals in connection pools.
    #[inline]
    pub fn pool_timer<M>(mut self, timer: M) -> Self
    where
        M: Timer + Clone + Send + Sync + 'static,
    {
        self.pool_timer = Some(ArcTimer::new(timer));
        self
    }

    /// Set whether to retry requests that get disrupted before ever starting
    /// to write.
    ///
    /// This means a request that is queued, and gets given an idle, reused
    /// connection, and then encounters an error immediately as the idle
    /// connection was found to be unusable.
    ///
    /// When this is set to `false`, the related `ResponseFuture` would instead
    /// resolve to an `Error::Cancel`.
    ///
    /// Default is `true`.
    #[inline]
    pub fn retry_canceled_requests(mut self, val: bool) -> Self {
        self.config.retry_canceled_requests = val;
        self
    }

    /// Set whether to automatically add the `Host` header to requests.
    ///
    /// If true, and a request does not include a `Host` header, one will be
    /// added automatically, derived from the authority of the `Uri`.
    ///
    /// Default is `true`.
    #[inline]
    pub fn set_host(mut self, val: bool) -> Self {
        self.config.set_host = val;
        self
    }

    /// Inject a pre-constructed `QuicConnector` for HTTP/3 (QUIC) requests.
    ///
    /// When `Some` and `Ver::Http3` is selected (via `http3_only()` or
    /// `http3_prior_knowledge()`), `connect_to` routes through this
    /// connector instead of the TCP connector. When `None`, h3 requests
    /// fail with `ErrorKind::UserUnsupportedVersion` at the
    /// `try_send_request` layer.
    ///
    /// This is the integration point used by `Client::build` (which
    /// constructs a `QuicConnector` from `Config::http3_options` /
    /// `Config::quic_config` when `http_version_pref == Http3`) and by
    /// the `__test_with_quic_connector` escape hatch on `ClientBuilder`.
    #[cfg(feature = "http3")]
    #[inline]
    pub(crate) fn h3_connector(
        mut self,
        connector: crate::client::conn::quic::QuicConnector,
    ) -> Self {
        self.h3_connector = Some(connector);
        self
    }

    /// Inject a pre-built `Arc<AltSvcCache>` for sharing between
    /// `HttpClient` and `Client`.
    #[cfg(feature = "http3")]
    #[inline]
    pub(crate) fn alt_svc_cache(mut self, cache: Arc<AltSvcCache>) -> Self {
        self.alt_svc_cache = Some(cache);
        self
    }

    /// Inject a pre-built `Arc<H3FailureTracker>` for sharing between
    /// `HttpClient` and `Client`.
    #[cfg(feature = "http3")]
    #[inline]
    pub(crate) fn h3_failure_tracker(mut self, tracker: Arc<H3FailureTracker>) -> Self {
        self.h3_failure_tracker = Some(tracker);
        self
    }

    /// Combine the configuration of this builder with a connector to create a `HttpClient`.
    pub fn build<C, B>(self, connector: C) -> HttpClient<C, B>
    where
        C: tower::Service<ConnectRequest> + Clone + Send + Sync + 'static,
        C::Response: AsyncRead + AsyncWrite + Connection + Unpin + Send + 'static,
        C::Error: Into<BoxError>,
        C::Future: Unpin + Send + 'static,
        B: Body + Send,
        B::Data: Send,
    {
        let exec = self.exec.clone();
        let timer = self.pool_timer.clone();
        HttpClient {
            config: self.config,
            exec: exec.clone(),

            #[cfg(feature = "http1")]
            h1_builder: self.h1_builder,
            #[cfg(feature = "http2")]
            h2_builder: self.h2_builder,
            #[cfg(feature = "http3")]
            h3_connector: self.h3_connector,
            #[cfg(feature = "http3")]
            alt_svc_cache: self
                .alt_svc_cache
                .unwrap_or_else(|| Arc::new(AltSvcCache::new())),
            #[cfg(feature = "http3")]
            h3_failure_tracker: self
                .h3_failure_tracker
                .unwrap_or_else(|| Arc::new(H3FailureTracker::new(Duration::from_secs(60)))),
            connector,
            pool: pool::Pool::new(self.pool_config, exec, timer),
        }
    }
}
