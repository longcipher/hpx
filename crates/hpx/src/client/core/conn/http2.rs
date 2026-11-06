//! HTTP/2 client connections

use std::{
    fmt,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
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
        dispatch::{self, RetryPromise, TrySendError},
        error::{BoxError, Error},
        proto::{self, h2::ping},
        rt::{ArcTimer, Time, Timer, bounds::Http2ClientConnExec},
    },
    http2::Http2Options,
};

/// Monotonically increasing per-process counter used to tag every h2
/// connection with a stable `conn_id` for trace correlation.
static CONN_SEQ: AtomicU64 = AtomicU64::new(1);

/// How long a request may wait on the connection's dispatch task without
/// that task being polled before we treat the connection as stalled.
///
/// The dispatch oneshot resolves when response headers arrive, so this must
/// NOT bound the wait itself: a healthy server that is simply slow — a long
/// poll, a slow upload, a time-to-first-byte over the bound, or queued
/// requests waiting behind `max_concurrent_streams` — legitimately resolves
/// its oneshot after any fixed deadline, and only the caller's own request
/// timeout may cut that short.
///
/// If the `ClientTask` ever parks permanently (a stall observed in production
/// where a connection's watchdog timer and mpsc wakeups both stop being
/// delivered), an unbounded `rx.await` here would pin the request until the
/// outer request `Timeout` fires (~15 s), or forever without one. A live
/// `ClientTask` is re-polled by its dispatch watchdog at least once a second
/// and counts every poll in `dispatch_polls`, so a whole window without a
/// single poll converts the never-resolving dispatch into a retryable
/// failure, while any poll starts a new window. The watchdog stays the
/// primary defence against lost wake-ups; this only bounds a task that is no
/// longer polled at all.
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
    /// Poll count of the connection's `ClientTask`, read while waiting for a
    /// response to tell a parked task from a slow server.
    dispatch_polls: Arc<AtomicU64>,
}

impl<B> Clone for SendRequest<B> {
    fn clone(&self) -> Self {
        Self {
            dispatch: self.dispatch.clone(),
            conn_id: self.conn_id,
            created_on_thread: self.created_on_thread.clone(),
            dispatch_polls: self.dispatch_polls.clone(),
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
        let dispatch_polls = self.dispatch_polls.clone();
        async move {
            match sent {
                Ok(rx) => {
                    await_dispatch(
                        rx,
                        &dispatch_polls,
                        DISPATCH_RESPONSE_TIMEOUT,
                        conn_id,
                        &created_on_thread,
                    )
                    .await
                }
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

/// Waits for the dispatch task's answer to a queued request.
///
/// Fails only after a whole `window` in which the connection's `ClientTask`
/// was never polled (see `DISPATCH_RESPONSE_TIMEOUT`); on a live connection a
/// slow response is awaited for as long as the caller lets it.
#[expect(
    clippy::result_large_err,
    reason = "TrySendError hands an unsent request back to the caller, as in try_send_request"
)]
async fn await_dispatch<B>(
    mut rx: RetryPromise<Request<B>, Response<IncomingBody>>,
    dispatch_polls: &AtomicU64,
    window: Duration,
    conn_id: u64,
    created_on_thread: &str,
) -> std::result::Result<Response<IncomingBody>, TrySendError<Request<B>>> {
    loop {
        let polls_before = dispatch_polls.load(Ordering::Relaxed);
        match tokio::time::timeout(window, &mut rx).await {
            Ok(Ok(Ok(res))) => return Ok(res),
            Ok(Ok(Err(err))) => return Err(err),
            // The dispatch task dropped its callback sender without
            // returning a result. This indicates a bug in the
            // connection lifecycle; surface it as a closed-connection
            // error rather than panicking.
            Ok(Err(_)) => {
                return Err(TrySendError {
                    error: Error::new_closed().with("dispatch dropped without result"),
                    message: None,
                });
            }
            // The dispatch task was polled during the window: it is alive and
            // the server is merely slow, so keep waiting.
            Err(_) if dispatch_polls.load(Ordering::Relaxed) != polls_before => {}
            // The connection's dispatch task was not polled once in the
            // window. The request was queued but may never be written
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
                return Err(TrySendError {
                    error: Error::new_io(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "dispatch wait timed out",
                    )),
                    message: None,
                });
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
        let dispatch_polls = h2.dispatch_polls();
        Ok((
            SendRequest {
                dispatch: tx.unbound(),
                conn_id,
                created_on_thread,
                dispatch_polls,
            },
            Connection {
                inner: (PhantomData, h2),
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use bytes::Bytes;
    use http::StatusCode;
    use http_body_util::Empty;
    use tokio::{
        io::{DuplexStream, duplex},
        sync::oneshot,
        task::JoinHandle,
    };

    use super::*;
    use crate::client::core::{
        proto::h2::client::H2ClientFuture,
        rt::{Executor, Sleep, TokioTimer},
    };

    /// Spawns connection tasks on the test's own runtime.
    #[derive(Clone)]
    struct TestExec;

    impl<F> Executor<F> for TestExec
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        fn execute(&self, fut: F) {
            tokio::spawn(fut);
        }
    }

    type ParkedTask = Pin<Box<dyn Future<Output = ()> + Send>>;

    /// Spawns every connection future except the h2 connection task, which
    /// it keeps alive but never polls: a new stream then stays pending open
    /// forever although its stream slot is free, i.e. a wedged connection.
    #[derive(Clone, Default)]
    struct WedgedConnExec(Arc<parking_lot::Mutex<Vec<ParkedTask>>>);

    impl<B, T> Executor<H2ClientFuture<B, T>> for WedgedConnExec
    where
        B: Body + 'static,
        B::Error: Into<BoxError>,
        T: AsyncRead + AsyncWrite + Unpin,
        H2ClientFuture<B, T>: Future<Output = ()> + Send + 'static,
    {
        fn execute(&self, fut: H2ClientFuture<B, T>) {
            if matches!(fut, H2ClientFuture::Task { .. }) {
                self.0.lock().push(Box::pin(fut));
            } else {
                tokio::spawn(fut);
            }
        }
    }

    /// Divides every relative sleep the connection asks for, so the dispatch
    /// watchdog (1 s) and `POLL_READY_TIMEOUT` (3 s) elapse in milliseconds.
    const TIME_SCALE: u32 = 20;

    struct FastTimer;

    impl Timer for FastTimer {
        fn sleep(&self, duration: Duration) -> Pin<Box<dyn Sleep>> {
            TokioTimer::new().sleep(duration / TIME_SCALE)
        }

        fn sleep_until(&self, deadline: Instant) -> Pin<Box<dyn Sleep>> {
            TokioTimer::new().sleep_until(deadline)
        }
    }

    /// Serves h2 on `io`, answering the n-th request after `delays[n]` (and
    /// any later one at once).
    fn serve(
        io: DuplexStream,
        max_concurrent_streams: Option<u32>,
        delays: Vec<Duration>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut builder = http2::server::Builder::new();
            if let Some(max) = max_concurrent_streams {
                builder.max_concurrent_streams(max);
            }
            let mut conn = builder
                .handshake::<_, Bytes>(io)
                .await
                .expect("server handshake");
            let mut delays = delays.into_iter();
            while let Some(accepted) = conn.accept().await {
                let Ok((_req, mut respond)) = accepted else {
                    break;
                };
                let delay = delays.next().unwrap_or_default();
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    let _ = respond.send_response(Response::new(()), true);
                });
            }
        })
    }

    /// Opens a client connection with keep-alive pings off, so only the
    /// timers under test run, and spawns its dispatch task.
    async fn connect(io: DuplexStream, mut opts: Http2Options) -> SendRequest<Empty<Bytes>> {
        opts.keep_alive_interval = None;
        let mut builder = Builder::new(TestExec);
        builder.timer(FastTimer);
        builder.options(opts);
        let (sender, conn) = builder
            .handshake::<_, Empty<Bytes>>(io)
            .await
            .expect("client handshake");
        tokio::spawn(conn);
        sender
    }

    /// A response slower than the dispatch window still arrives while the
    /// connection's dispatch task is alive: the window bounds a stalled
    /// task, not the server's time to response headers.
    #[tokio::test]
    async fn slow_response_on_live_connection_outlasts_dispatch_window() {
        // Five scaled watchdog intervals per window; the response takes four
        // windows.
        let window = Duration::from_millis(250);
        let (client_io, server_io) = duplex(64 * 1024);
        let _server = serve(server_io, None, vec![window * 4]);
        let sender = connect(client_io, Http2Options::default()).await;

        let rx = sender
            .dispatch
            .try_send(Request::new(Empty::new()))
            .expect("dispatch channel open");
        let res = await_dispatch(rx, &sender.dispatch_polls, window, sender.conn_id, "test")
            .await
            .expect("slow response");

        assert_eq!(res.status(), StatusCode::OK);
    }

    /// A dispatch task that is never polled again (lost wake-ups, or a
    /// runtime that stopped driving it) still fails the waiting request after
    /// one window, as a transport timeout the retry layer and pool act on.
    #[tokio::test]
    async fn stalled_dispatch_task_fails_request_after_one_window() {
        let (_tx, rx) = oneshot::channel::<
            std::result::Result<Response<IncomingBody>, TrySendError<Request<()>>>,
        >();
        let polls = AtomicU64::new(0);

        let err = await_dispatch(rx, &polls, Duration::from_millis(20), 0, "test")
            .await
            .expect_err("a stalled dispatch task must time out");

        assert!(err.message.is_none());
        let io = err
            .error
            .find_source::<std::io::Error>()
            .expect("io error source");
        assert_eq!(io.kind(), std::io::ErrorKind::TimedOut);
    }

    /// A request queued behind SETTINGS_MAX_CONCURRENT_STREAMS waits for a
    /// free stream slot instead of being failed as a wedged connection.
    #[tokio::test]
    async fn request_queued_behind_stream_limit_is_not_failed() {
        // Four times `POLL_READY_TIMEOUT` once `FastTimer` scales it.
        let slow = Duration::from_millis(600);
        let (client_io, server_io) = duplex(64 * 1024);
        let _server = serve(server_io, Some(1), vec![slow]);
        // Match the server's limit before its SETTINGS arrive, so the second
        // request waits for a slot instead of being refused.
        let opts = Http2Options {
            initial_max_send_streams: 1,
            ..Http2Options::default()
        };
        let sender = connect(client_io, opts).await;

        let first = sender.try_send_request(Request::new(Empty::new()));
        let second = sender.try_send_request(Request::new(Empty::new()));
        let (first, second) = tokio::join!(first, second);

        assert_eq!(first.expect("first response").status(), StatusCode::OK);
        assert_eq!(second.expect("second response").status(), StatusCode::OK);
    }

    /// A stream that stays pending open while its stream slot is free still
    /// marks the connection as wedged and fails the request fast.
    #[tokio::test]
    async fn pending_stream_with_free_slot_still_fails_fast() {
        let (client_io, _server_io) = duplex(64 * 1024);
        let mut builder = Builder::new(WedgedConnExec::default());
        builder.timer(FastTimer);
        builder.options(Http2Options {
            // A single slot, which the pending stream alone does not fill.
            initial_max_send_streams: 1,
            keep_alive_interval: None,
            ..Http2Options::default()
        });
        let (sender, conn) = builder
            .handshake::<_, Empty<Bytes>>(client_io)
            .await
            .expect("client handshake");
        tokio::spawn(conn);

        // Far past the scaled `POLL_READY_TIMEOUT` (150 ms), far below
        // `DISPATCH_RESPONSE_TIMEOUT`.
        let res = tokio::time::timeout(
            Duration::from_secs(2),
            sender.try_send_request(Request::new(Empty::new())),
        )
        .await
        .expect("a wedged connection must fail the request fast");

        assert!(res.is_err());
    }
}
