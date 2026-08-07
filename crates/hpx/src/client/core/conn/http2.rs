//! HTTP/2 client connections

use std::{
    fmt,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll, ready},
    time::Duration,
};

use http::{Request, Response};
use http_body::Body;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{
    client::core::{
        Result,
        body::Incoming as IncomingBody,
        dispatch::{self, TrySendError},
        error::{BoxError, Error},
        proto::{self, h2::ping},
        rt::{ArcTimer, Time, Timer, bounds::Http2ClientConnExec},
    },
    http2::Http2Options,
};

/// Monotonically increasing per-process counter used to tag every h2
/// connection with a stable `conn_id` for trace correlation.
static CONN_SEQ: AtomicU64 = AtomicU64::new(1);

/// Upper bound on how long a request may wait for the connection's dispatch
/// task to produce a response before we treat the connection as stalled.
///
/// The dispatch oneshot resolves when response headers arrive. It must NOT be
/// a tight per-request deadline: a healthy server that is simply slow — e.g.
/// one that delays the connection preface or limits `max_concurrent_streams`
/// to 1, forcing queued requests to wait behind earlier ones — legitimately
/// resolves its oneshot well past 1 s. A tight timeout (1 s) therefore kills
/// requests to such servers and misclassifies normal backpressure as a stall.
///
/// If the `ClientTask` ever parks permanently (a stall observed in production
/// where a connection's watchdog timer and mpsc wakeups both stop being
/// delivered), an unbounded `rx.await` here would pin the request until the
/// outer request `Timeout` fires (~15 s). The `ClientTask` dispatch watchdog
/// (5 s interval) provides the primary fast-path detection; this timeout is a
/// final safety net that converts a never-resolving dispatch into a retryable
/// failure. 30 s leaves ample headroom for slow but healthy servers, at the
/// cost of a longer worst-case recovery for genuinely stalled connections
/// (previously ~1.3 s). Prefer keeping the user-facing request `Timeout` tight
/// so stalled connections are bounded by it in practice.
const DISPATCH_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

/// The sender side of an established connection.
pub(crate) struct SendRequest<B> {
    dispatch: dispatch::UnboundedSender<Request<B>, Response<IncomingBody>>,
    /// Identifier assigned when the connection is created; correlates
    /// dispatch-side traces with pool/connector logs across connections.
    conn_id: u64,
    /// Name of the thread that created the connection. Its dispatch task is
    /// spawned onto whatever tokio runtime was current at creation time, so a
    /// connection created on one runtime can stall if a request is later sent
    /// from a different runtime whose executor does not drive this task.
    created_on_thread: String,
}

impl<B> Clone for SendRequest<B> {
    fn clone(&self) -> Self {
        Self {
            dispatch: self.dispatch.clone(),
            conn_id: self.conn_id,
            created_on_thread: self.created_on_thread.clone(),
        }
    }
}

/// A future that processes all HTTP state for the IO object.
///
/// In most cases, this should just be spawned into an executor, so that it
/// can process incoming and outgoing messages, notice hangups, and the like.
#[must_use = "futures do nothing unless polled"]
pub(crate) struct Connection<T, B, E>
where
    T: AsyncRead + AsyncWrite + Unpin,
    B: Body + 'static,
    E: Http2ClientConnExec<B, T> + Unpin,
    B::Error: Into<BoxError>,
{
    inner: (PhantomData<T>, proto::h2::ClientTask<B, E, T>),
}

/// A builder to configure an HTTP connection.
///
/// After setting options, the builder is used to create a handshake future.
///
/// **Note**: The default values of options are *not considered stable*. They
/// are subject to change at any time.
#[derive(Clone)]
pub(crate) struct Builder<Ex> {
    exec: Ex,
    timer: Time,
    opts: Http2Options,
}

// ===== impl SendRequest

impl<B> SendRequest<B> {
    /// Polls to determine whether this sender can be used yet for a request.
    ///
    /// If the associated connection is closed, this returns an Error.
    pub(crate) fn poll_ready(&self, _cx: &mut Context<'_>) -> Poll<Result<()>> {
        if self.is_closed() {
            Poll::Ready(Err(Error::new_closed()))
        } else {
            Poll::Ready(Ok(()))
        }
    }

    /// Waits until the dispatcher is ready
    ///
    /// If the associated connection is closed, this returns an Error.
    pub(crate) async fn ready(&self) -> Result<()> {
        std::future::poll_fn(|cx| self.poll_ready(cx)).await
    }

    /// Checks if the connection is currently ready to send a request.
    ///
    /// # Note
    ///
    /// This is mostly a hint. Due to inherent latency of networks, it is
    /// possible that even after checking this is ready, sending a request
    /// may still fail because the connection was closed in the meantime.
    pub(crate) fn is_ready(&self) -> bool {
        self.dispatch.is_ready()
    }

    /// Checks if the connection side has been closed.
    pub(crate) fn is_closed(&self) -> bool {
        self.dispatch.is_closed()
    }
}

impl<B> SendRequest<B>
where
    B: Body + 'static,
{
    /// Sends a `Request` on the associated connection.
    ///
    /// Returns a future that if successful, yields the `Response`.
    ///
    /// # Error
    ///
    /// If there was an error before trying to serialize the request to the
    /// connection, the message will be returned as part of this error.
    #[expect(clippy::result_large_err)]
    pub(crate) fn try_send_request(
        &self,
        req: Request<B>,
    ) -> impl Future<Output = std::result::Result<Response<IncomingBody>, TrySendError<Request<B>>>>
    {
        let sent = self.dispatch.try_send(req);
        match &sent {
            Ok(_) => trace!("h2 dispatch try_send ok conn_id={}", self.conn_id),
            Err(_) => trace!(
                "h2 dispatch try_send failed conn_id={}; channel closed",
                self.conn_id
            ),
        }
        let conn_id = self.conn_id;
        let created_on_thread = self.created_on_thread.clone();
        async move {
            match sent {
                Ok(rx) => match tokio::time::timeout(DISPATCH_RESPONSE_TIMEOUT, rx).await {
                    Ok(Ok(Ok(res))) => Ok(res),
                    Ok(Ok(Err(err))) => Err(err),
                    // The dispatch task dropped its callback sender without
                    // returning a result. This indicates a bug in the
                    // connection lifecycle; surface it as a closed-connection
                    // error rather than panicking.
                    Ok(Err(_)) => Err(TrySendError {
                        error: Error::new_closed().with("dispatch dropped without result"),
                        message: None,
                    }),
                    // The connection's dispatch task never answered within the
                    // bound. The request was queued but may never be written
                    // (stalled ClientTask), so surface a transport timeout with
                    // no message: the retry layer will retry idempotent methods
                    // on a fresh connection and the pool poisons this one.
                    Err(_) => {
                        warn!(
                            "h2 dispatch response timed out conn_id={}; connection stalled (created on {:?}, requested from {:?})",
                            conn_id,
                            created_on_thread,
                            std::thread::current().name().unwrap_or("<unnamed>")
                        );
                        Err(TrySendError {
                            error: Error::new_io(std::io::Error::new(
                                std::io::ErrorKind::TimedOut,
                                "dispatch wait timed out",
                            )),
                            message: None,
                        })
                    }
                },
                Err(req) => {
                    debug!("connection was not ready");
                    let error = Error::new_canceled().with("connection was not ready");
                    Err(TrySendError {
                        error,
                        message: Some(req),
                    })
                }
            }
        }
    }
}

impl<B> fmt::Debug for SendRequest<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SendRequest").finish()
    }
}

// ===== impl Connection

impl<T, B, E> fmt::Debug for Connection<T, B, E>
where
    T: AsyncRead + AsyncWrite + fmt::Debug + 'static + Unpin,
    B: Body + 'static,
    E: Http2ClientConnExec<B, T> + Unpin,
    B::Error: Into<BoxError>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection").finish()
    }
}

impl<T, B, E> Future for Connection<T, B, E>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
    B: Body + 'static + Unpin,
    B::Data: Send,
    B::Error: Into<BoxError>,
    E: Http2ClientConnExec<B, T> + Unpin,
{
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match ready!(Pin::new(&mut self.inner.1).poll(cx))? {
            proto::Dispatched::Shutdown => Poll::Ready(Ok(())),
            proto::Dispatched::Upgrade(_pending) => unreachable!("http2 cannot upgrade"),
        }
    }
}

// ===== impl Builder

impl<Ex> Builder<Ex>
where
    Ex: Clone,
{
    /// Creates a new connection builder.
    #[inline]
    pub(crate) fn new(exec: Ex) -> Self {
        Self {
            exec,
            timer: Time::Empty,
            opts: Default::default(),
        }
    }

    /// Provide a timer to execute background HTTP2 tasks.
    #[inline]
    pub(crate) fn timer<M>(&mut self, timer: M)
    where
        M: Timer + Send + Sync + 'static,
    {
        self.timer = Time::Timer(ArcTimer::new(timer));
    }

    /// Provide a options configuration for the HTTP/2 connection.
    #[inline]
    pub(crate) fn options(&mut self, opts: Http2Options) {
        self.opts = opts;
    }

    /// Constructs a connection with the configured options and IO.
    ///
    /// Note, if [`Connection`] is not `await`-ed, [`SendRequest`] will
    /// do nothing.
    pub(crate) async fn handshake<T, B>(
        self,
        io: T,
    ) -> Result<(SendRequest<B>, Connection<T, B, Ex>)>
    where
        T: AsyncRead + AsyncWrite + Unpin,
        B: Body + 'static,
        B::Data: Send,
        B::Error: Into<BoxError>,
        Ex: Http2ClientConnExec<B, T> + Unpin,
    {
        trace!("client handshake HTTP/2");

        // Crate the HTTP/2 client with the provided options.
        let builder = {
            let mut builder = http2::client::Builder::default();
            builder
                .initial_max_send_streams(self.opts.initial_max_send_streams)
                .initial_window_size(self.opts.initial_window_size)
                .initial_connection_window_size(self.opts.initial_conn_window_size)
                .max_send_buffer_size(self.opts.max_send_buffer_size);
            if let Some(id) = self.opts.initial_stream_id {
                builder.initial_stream_id(id);
            }
            if let Some(max) = self.opts.max_pending_accept_reset_streams {
                builder.max_pending_accept_reset_streams(max);
            }
            if let Some(max) = self.opts.max_concurrent_reset_streams {
                builder.max_concurrent_reset_streams(max);
            }
            if let Some(max) = self.opts.max_concurrent_streams {
                builder.max_concurrent_streams(max);
            }
            if let Some(max) = self.opts.max_header_list_size {
                builder.max_header_list_size(max);
            }
            if let Some(opt) = self.opts.enable_push {
                builder.enable_push(opt);
            }
            if let Some(max) = self.opts.max_frame_size {
                builder.max_frame_size(max);
            }
            if let Some(max) = self.opts.header_table_size {
                builder.header_table_size(max);
            }
            if let Some(v) = self.opts.enable_connect_protocol {
                builder.enable_connect_protocol(v);
            }
            if let Some(v) = self.opts.no_rfc7540_priorities {
                builder.no_rfc7540_priorities(v);
            }
            if let Some(order) = self.opts.settings_order {
                builder.settings_order(order);
            }
            if let Some(experimental_settings) = self.opts.experimental_settings {
                builder.experimental_settings(experimental_settings);
            }
            if let Some(stream_dependency) = self.opts.headers_stream_dependency {
                builder.headers_stream_dependency(stream_dependency);
            }
            if let Some(order) = self.opts.headers_pseudo_order {
                builder.headers_pseudo_order(order);
            }
            if let Some(priority) = self.opts.priorities {
                builder.priorities(priority);
            }

            builder
        };

        // Create the ping configuration for the connection.
        let ping_config = ping::Config::new(
            self.opts.adaptive_window,
            self.opts.initial_window_size,
            self.opts.keep_alive_interval,
            self.opts.keep_alive_timeout,
            self.opts.keep_alive_while_idle,
        );

        let (tx, rx) = dispatch::channel();
        let conn_id = CONN_SEQ.fetch_add(1, Ordering::Relaxed);
        let created_on_thread = std::thread::current()
            .name()
            .unwrap_or("<unnamed>")
            .to_string();
        trace!(
            "h2 connection created conn_id={} on {:?}",
            conn_id, created_on_thread
        );
        let h2 = proto::h2::client::handshake(
            io,
            rx,
            builder,
            ping_config,
            self.exec,
            self.timer,
            conn_id,
        )
        .await?;
        Ok((
            SendRequest {
                dispatch: tx.unbound(),
                conn_id,
                created_on_thread,
            },
            Connection {
                inner: (PhantomData, h2),
            },
        ))
    }
}
