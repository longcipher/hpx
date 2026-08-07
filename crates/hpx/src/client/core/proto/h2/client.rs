use std::{
    convert::Infallible,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll, ready},
    time::Duration,
};

use bytes::Bytes;
use futures_channel::{
    mpsc,
    mpsc::{Receiver, Sender},
    oneshot,
};
use futures_util::{
    future::{Either, FusedFuture},
    stream::{FusedStream, Stream},
};
use http::{Method, Request, Response, StatusCode};
use http_body::Body;
use http2::{
    SendStream,
    client::{Builder, Connection, ResponseFuture, SendRequest},
};
use pin_project_lite::pin_project;
use tokio::io::{AsyncRead, AsyncWrite};

use super::{
    H2Upgraded, PipeToSendStream, SendBuf, ping,
    ping::{Ponger, Recorder},
};
use crate::{
    client::core::{
        self, Error,
        body::{self, Incoming as IncomingBody},
        dispatch::{self, Callback, SendWhen, TrySendError},
        error::BoxError,
        proto::{Dispatched, headers},
        rt::{Sleep, Time, bounds::Http2ClientConnExec},
        upgrade::{self, Upgraded},
    },
    config::RequestConfig,
    header::OrigHeaderMap,
};

type ClientRx<B> = dispatch::Receiver<Request<B>, Response<IncomingBody>>;

///// An mpsc channel is used to help notify the `Connection` task when *all*
///// other handles to it have been dropped, so that it can shutdown.
type ConnDropRef = mpsc::Sender<Infallible>;

///// A oneshot channel watches the `Connection` task, and when it completes,
///// the "dispatch" task will be notified and can shutdown sooner.
type ConnEof = oneshot::Receiver<Infallible>;

#[expect(
    clippy::expect_used,
    reason = "ping_pong() returns Some when ping_config.is_enabled() at handshake time"
)]
pub(crate) async fn handshake<T, B, E>(
    io: T,
    req_rx: ClientRx<B>,
    builder: Builder,
    ping_config: ping::Config,
    mut exec: E,
    timer: Time,
    conn_id: u64,
) -> core::Result<ClientTask<B, E, T>>
where
    T: AsyncRead + AsyncWrite + Unpin,
    B: Body + 'static,
    B::Data: Send + 'static,
    E: Http2ClientConnExec<B, T> + Unpin,
    B::Error: Into<BoxError>,
{
    let (h2_tx, mut conn) = builder
        .handshake::<_, SendBuf<B::Data>>(io)
        .await
        .map_err(Error::new_h2)?;

    // An mpsc channel is used entirely to detect when the
    // 'Client' has been dropped. This is to get around a bug
    // in h2 where dropping all SendRequests won't notify a
    // parked Connection.
    let (conn_drop_ref, conn_drop_rx) = mpsc::channel(1);
    let (cancel_tx, conn_eof) = oneshot::channel();

    let (conn, ping) = if ping_config.is_enabled() {
        let pp = conn.ping_pong().expect("conn.ping_pong");
        let (recorder, ponger) = ping::channel(pp, ping_config, timer.clone());

        let conn: Conn<_, B> = Conn::new(ponger, conn);
        (Either::Left(conn), recorder)
    } else {
        (Either::Right(conn), ping::disabled())
    };
    let conn: ConnMapErr<T, B> = ConnMapErr {
        conn,
        is_terminated: false,
    };

    exec.execute_h2_future(H2ClientFuture::Task {
        task: ConnTask::new(conn, conn_drop_rx, cancel_tx),
    });

    let closed = ping.closed_notified();

    Ok(ClientTask {
        ping,
        conn_drop_ref,
        conn_eof,
        executor: exec,
        h2_tx,
        req_rx,
        fut_ctx: None,
        timer,
        closed,
        poll_ready_deadline: None,
        dispatch_watchdog: None,
        conn_id,
        marker: PhantomData,
    })
}

pin_project! {
    struct Conn<T, B>
    where
        B: Body,
    {
        #[pin]
        ponger: Ponger,
        #[pin]
        conn: Connection<T, SendBuf<<B as Body>::Data>>,
    }
}

impl<T, B> Conn<T, B>
where
    B: Body,
    T: AsyncRead + AsyncWrite + Unpin,
{
    const fn new(ponger: Ponger, conn: Connection<T, SendBuf<<B as Body>::Data>>) -> Self {
        Self { ponger, conn }
    }
}

impl<T, B> Future for Conn<T, B>
where
    B: Body,
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Output = Result<(), http2::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        match this.ponger.poll(cx) {
            Poll::Ready(ping::Ponged::SizeUpdate(wnd)) => {
                this.conn.set_target_window_size(wnd);
                this.conn.set_initial_window_size(wnd)?;
            }
            Poll::Ready(ping::Ponged::KeepAliveTimedOut) => {
                debug!("connection keep-alive timed out");
                this.ponger.mark_conn_closed();
                return Poll::Ready(Ok(()));
            }
            Poll::Pending => {}
        }

        let polled = Pin::new(&mut this.conn).poll(cx);
        if polled.is_ready() {
            // The h2 connection is done (GOAWAY, error, ...). Mark it closed
            // so tasks parked on in-flight requests are woken up and can fail
            // fast instead of hanging until the caller's request timeout.
            this.ponger.mark_conn_closed();
        }
        polled
    }
}

pin_project! {
    struct ConnMapErr<T, B>
    where
        B: Body,
        T: AsyncRead,
        T: AsyncWrite,
        T: Unpin,
    {
        #[pin]
        conn: Either<Conn<T, B>, Connection<T, SendBuf<<B as Body>::Data>>>,
        #[pin]
        is_terminated: bool,
    }
}

impl<T, B> Future for ConnMapErr<T, B>
where
    B: Body,
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Output = Result<(), ()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        if *this.is_terminated {
            return Poll::Pending;
        }
        let polled = this.conn.poll(cx);
        if polled.is_ready() {
            *this.is_terminated = true;
        }
        polled.map_err(|_e| {
            debug!(error = %_e, "connection error");
        })
    }
}

impl<T, B> FusedFuture for ConnMapErr<T, B>
where
    B: Body,
    T: AsyncRead + AsyncWrite + Unpin,
{
    fn is_terminated(&self) -> bool {
        self.is_terminated
    }
}

pin_project! {
    pub struct ConnTask<T, B>
    where
        B: Body,
        T: AsyncRead,
        T: AsyncWrite,
        T: Unpin,
    {
        #[pin]
        drop_rx: Receiver<Infallible>,
        #[pin]
        cancel_tx: Option<oneshot::Sender<Infallible>>,
        #[pin]
        conn: ConnMapErr<T, B>,
    }
}

impl<T, B> ConnTask<T, B>
where
    B: Body,
    T: AsyncRead + AsyncWrite + Unpin,
{
    const fn new(
        conn: ConnMapErr<T, B>,
        drop_rx: Receiver<Infallible>,
        cancel_tx: oneshot::Sender<Infallible>,
    ) -> Self {
        Self {
            drop_rx,
            cancel_tx: Some(cancel_tx),
            conn,
        }
    }
}

impl<T, B> Future for ConnTask<T, B>
where
    B: Body,
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Output = ();

    #[expect(
        clippy::expect_used,
        reason = "ConnTask Future state machine: cancel_tx is Some until first poll after drop"
    )]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        trace!(
            "ConnTask poll on thread {:?}",
            std::thread::current().name().unwrap_or("<unnamed>")
        );
        let mut this = self.project();

        if !this.conn.is_terminated() && Pin::new(&mut this.conn).poll(cx).is_ready() {
            // ok or err, the `conn` has finished.
            return Poll::Ready(());
        }

        if !this.drop_rx.is_terminated() && Pin::new(&mut this.drop_rx).poll_next(cx).is_ready() {
            // mpsc has been dropped, hopefully polling
            // the connection some more should start shutdown
            // and then close.
            trace!("send_request dropped, starting conn shutdown");
            drop(this.cancel_tx.take().expect("ConnTask Future polled twice"));
        }

        Poll::Pending
    }
}

pin_project! {
    #[project = H2ClientFutureProject]
    pub enum H2ClientFuture<B, T>
    where
        B: http_body::Body,
        B: 'static,
        B::Error: Into<BoxError>,
        T: AsyncRead,
        T: AsyncWrite,
        T: Unpin,
    {
        Pipe {
            #[pin]
            pipe: PipeMap<B>,
        },
        Send {
            #[pin]
            send_when: SendWhen<B>,
        },
        Task {
            #[pin]
            task: ConnTask<T, B>,
        },
    }
}

impl<B, T> Future for H2ClientFuture<B, T>
where
    B: Body + 'static,
    B::Data: Send,
    B::Error: Into<BoxError>,
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> std::task::Poll<Self::Output> {
        let this = self.project();

        match this {
            H2ClientFutureProject::Pipe { pipe } => pipe.poll(cx),
            H2ClientFutureProject::Send { send_when } => send_when.poll(cx),
            H2ClientFutureProject::Task { task } => task.poll(cx),
        }
    }
}

struct FutCtx<B>
where
    B: Body,
{
    is_connect: bool,
    eos: bool,
    fut: ResponseFuture,
    body_tx: SendStream<SendBuf<B::Data>>,
    body: B,
    cb: Callback<Request<B>, Response<IncomingBody>>,
}

impl<B: Body> Unpin for FutCtx<B> {}

pub(crate) struct ClientTask<B, E, T>
where
    B: Body,
    E: Unpin,
{
    ping: ping::Recorder,
    conn_drop_ref: ConnDropRef,
    conn_eof: ConnEof,
    executor: E,
    h2_tx: SendRequest<SendBuf<B::Data>>,
    req_rx: ClientRx<B>,
    fut_ctx: Option<FutCtx<B>>,
    timer: Time,
    /// Resolves when the underlying h2 connection has ended. Polled alongside
    /// `poll_ready` so a dead connection wakes this task instead of parking it
    /// forever waiting for stream capacity that never comes.
    closed: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    /// Armed when `poll_ready` first parks. If the h2 channel stays unready
    /// past `POLL_READY_TIMEOUT`, the connection is stuck (pending stream can
    /// never open even though keep-alive pings still flow); fail the pending
    /// request fast so the pool can rebuild the connection instead of the
    /// request hanging until the caller's request timeout.
    poll_ready_deadline: Option<Pin<Box<dyn Sleep>>>,
    /// Watchdog that forces a periodic re-poll while this task is parked
    /// waiting for the next request. If the dispatch mpsc wake-up is ever lost
    /// (a request sits in the buffer while the task is never polled again —
    /// observed in the wild as a fixed ~15s stall on pooled h2 connections),
    /// this timer bounds the stall to `DISPATCH_WATCHDOG_INTERVAL`: when it
    /// fires the task wakes, `poll_recv` drains the buffered request, and the
    /// watchdog is re-armed. Idle connections merely wake once per interval
    /// for a no-op. Never tears down a healthy connection by itself.
    dispatch_watchdog: Option<Pin<Box<dyn Sleep>>>,
    /// Monotonically increasing identifier for this connection, used to
    /// correlate dispatch-side traces (try_send / poll / watchdog) with
    /// pool and connector logs when multiple connections to the same host
    /// are alive.
    conn_id: u64,
    marker: PhantomData<T>,
}

/// How long `poll_ready` may stay parked before the dispatch task gives up on
/// the h2 channel and lets the connection be torn down. Normal capacity
/// waits finish in milliseconds; anything near this bound means the
/// connection is wedged.
const POLL_READY_TIMEOUT: Duration = Duration::from_secs(3);

/// How often the dispatch watchdog re-arms while the task is parked waiting
/// for the next request. Bounds any lost-wake-up stall on the request queue.
/// A tokio timer wake is independent of the mpsc wake, so a request that
/// would otherwise sit in the buffer for the full caller timeout (~15s) is
/// drained within this interval instead.
const DISPATCH_WATCHDOG_INTERVAL: Duration = Duration::from_secs(1);

pin_project! {
    pub struct PipeMap<S>
    where
        S: Body,
    {
        #[pin]
        pipe: PipeToSendStream<S>,
        #[pin]
        conn_drop_ref: Option<Sender<Infallible>>,
        #[pin]
        ping: Option<Recorder>,
        closed: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    }
}

impl<B> Future for PipeMap<B>
where
    B: http_body::Body,
    B::Error: Into<BoxError>,
{
    type Output = ();

    #[expect(
        clippy::expect_used,
        reason = "PipeMap Future state machine: conn_drop_ref and ping are Some until completion"
    )]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> std::task::Poll<Self::Output> {
        let mut this = self.project();

        // Stop early once the underlying h2 connection has ended instead of
        // parking the body pipe on a dead connection.
        if let Some(ref ping) = *this.ping
            && (ping.is_conn_closed()
                || this
                    .closed
                    .as_mut()
                    .is_some_and(|closed| closed.as_mut().poll(cx).is_ready()))
        {
            drop(this.conn_drop_ref.take().expect("Future polled twice"));
            drop(this.ping.take().expect("Future polled twice"));
            return Poll::Ready(());
        }

        match Pin::new(&mut this.pipe).poll(cx) {
            Poll::Ready(result) => {
                if let Err(_e) = result {
                    debug!("client request body error: {}", _e);
                }
                drop(this.conn_drop_ref.take().expect("Future polled twice"));
                drop(this.ping.take().expect("Future polled twice"));
                return Poll::Ready(());
            }
            Poll::Pending => (),
        }
        Poll::Pending
    }
}

impl<B, E, T> ClientTask<B, E, T>
where
    B: Body + 'static + Unpin,
    B::Data: Send,
    E: Http2ClientConnExec<B, T> + Unpin,
    B::Error: Into<BoxError>,
    T: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_pipe(&mut self, f: FutCtx<B>, cx: &mut Context<'_>) {
        let ping = self.ping.clone();

        let send_stream = if f.is_connect {
            Some(f.body_tx)
        } else {
            if !f.eos {
                let mut pipe = PipeToSendStream::new(f.body, f.body_tx);

                // eagerly see if the body pipe is ready and
                // can thus skip allocating in the executor
                match Pin::new(&mut pipe).poll(cx) {
                    Poll::Ready(_) => (),
                    Poll::Pending => {
                        let conn_drop_ref = self.conn_drop_ref.clone();
                        // keep the ping recorder's knowledge of an
                        // "open stream" alive while this body is
                        // still sending...
                        let pipe = PipeMap {
                            pipe,
                            conn_drop_ref: Some(conn_drop_ref),
                            ping: Some(ping.clone()),
                            closed: ping.closed_notified(),
                        };
                        // Clear send task
                        self.executor
                            .execute_h2_future(H2ClientFuture::Pipe { pipe });
                    }
                }
            }

            None
        };

        self.executor.execute_h2_future(H2ClientFuture::Send {
            send_when: SendWhen {
                when: ResponseFutMap {
                    fut: f.fut,
                    ping: Some(ping.clone()),
                    send_stream: Some(send_stream),
                    closed: ping.closed_notified(),
                },
                call_back: Some(f.cb),
            },
        });
    }

    /// Returns true if the connection task (ConnTask) has ended. The oneshot
    /// resolves with an error when the ConnTask's `cancel_tx` is dropped.
    fn conn_is_eof(&mut self, cx: &mut Context<'_>) -> bool {
        match Pin::new(&mut self.conn_eof).poll(cx) {
            Poll::Ready(Ok(never)) => match never {},
            Poll::Ready(Err(_)) => true,
            Poll::Pending => false,
        }
    }

    /// Arms the dispatch watchdog if it is not already armed, and reports
    /// whether it just fired. The watchdog forces this task to be re-polled
    /// at least once per `DISPATCH_WATCHDOG_INTERVAL` while parked in *any*
    /// Pending branch. If a wake-up from the dispatch mpsc or the h2 channel
    /// is ever lost (observed in the wild as a request sitting in the buffer
    /// for the full caller timeout), the timer wake breaks the stall and the
    /// caller's `continue` re-runs the full loop (poll_ready + poll_recv).
    fn dispatch_watchdog_fired(&mut self, cx: &mut Context<'_>) -> bool {
        if self.dispatch_watchdog.is_none() {
            self.dispatch_watchdog = Some(self.timer.sleep(DISPATCH_WATCHDOG_INTERVAL));
            trace!("dispatch watchdog armed conn_id={}", self.conn_id);
        }
        if self
            .dispatch_watchdog
            .as_mut()
            .is_some_and(|sleep| sleep.as_mut().poll(cx).is_ready())
        {
            trace!(
                "dispatch watchdog fired conn_id={}; re-arming",
                self.conn_id
            );
            self.dispatch_watchdog = Some(self.timer.sleep(DISPATCH_WATCHDOG_INTERVAL));
            true
        } else {
            false
        }
    }
}

pin_project! {
    pub(crate) struct ResponseFutMap<B>
    where
        B: Body,
        B: 'static,
    {
        #[pin]
        fut: ResponseFuture,
        #[pin]
        ping: Option<Recorder>,
        #[pin]
        send_stream: Option<Option<SendStream<SendBuf<<B as Body>::Data>>>>,
        closed: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    }
}

impl<B> Future for ResponseFutMap<B>
where
    B: Body + 'static,
    B::Data: Send,
{
    type Output = Result<Response<body::Incoming>, (Error, Option<Request<B>>)>;

    #[expect(
        clippy::expect_used,
        reason = "ResponseFutMap state machine: ping and send_stream are Some until completion"
    )]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        // Fail fast once the underlying h2 connection has ended: without this
        // the response future parks until the caller's request timeout (e.g.
        // 15s) even though the connection task is gone for good.
        if let Some(ref ping) = *this.ping
            && (ping.is_conn_closed()
                || this
                    .closed
                    .as_mut()
                    .is_some_and(|closed| closed.as_mut().poll(cx).is_ready()))
        {
            return Poll::Ready(Err((
                Error::new_h2(::http2::Reason::NO_ERROR.into()),
                None::<Request<B>>,
            )));
        }

        let result = ready!(this.fut.poll(cx));

        let ping = this.ping.take().expect("Future polled twice");
        let send_stream = this.send_stream.take().expect("Future polled twice");

        match result {
            Ok(res) => {
                // record that we got the response headers
                ping.record_non_data();

                let content_length = headers::content_length_parse_all(res.headers());
                if let (Some(mut send_stream), StatusCode::OK) = (send_stream, res.status()) {
                    if content_length.is_some_and(|len| len != 0) {
                        warn!("h2 connect response with non-zero body not supported");

                        send_stream.send_reset(http2::Reason::INTERNAL_ERROR);
                        return Poll::Ready(Err((
                            Error::new_h2(http2::Reason::INTERNAL_ERROR.into()),
                            None::<Request<B>>,
                        )));
                    }
                    let (parts, recv_stream) = res.into_parts();
                    let mut res = Response::from_parts(parts, IncomingBody::empty());

                    let (pending, on_upgrade) = upgrade::pending();
                    let io = H2Upgraded {
                        ping,
                        send_stream,
                        recv_stream,
                        buf: Bytes::new(),
                    };
                    let upgraded = Upgraded::new(io, Bytes::new());

                    pending.fulfill(upgraded);
                    res.extensions_mut().insert(on_upgrade);

                    Poll::Ready(Ok(res))
                } else {
                    let res = res.map(|stream| {
                        let ping = ping.for_stream(&stream);
                        IncomingBody::h2(stream, content_length.into(), ping)
                    });
                    Poll::Ready(Ok(res))
                }
            }
            Err(err) => {
                ping.ensure_not_timed_out().map_err(|e| (e, None))?;

                debug!("client response error: {}", err);
                Poll::Ready(Err((Error::new_h2(err), None::<Request<B>>)))
            }
        }
    }
}

impl<B, E, T> Future for ClientTask<B, E, T>
where
    B: Body + 'static + Unpin,
    B::Data: Send,
    B::Error: Into<BoxError>,
    E: Http2ClientConnExec<B, T> + Unpin,
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Output = core::Result<Dispatched>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            trace!(
                "client dispatch poll conn_id={}; fut_ctx={}; wd={}; prd={}; thread={:?}",
                self.conn_id,
                self.fut_ctx.is_some(),
                self.dispatch_watchdog.is_some(),
                self.poll_ready_deadline.is_some(),
                std::thread::current().name().unwrap_or("<unnamed>")
            );
            match self.h2_tx.poll_ready(cx) {
                Poll::Ready(Ok(())) => {
                    self.poll_ready_deadline = None;
                }
                Poll::Ready(Err(err)) => {
                    self.poll_ready_deadline = None;
                    self.ping.ensure_not_timed_out()?;
                    return if err.reason() == Some(::http2::Reason::NO_ERROR) {
                        trace!("connection gracefully shutdown");
                        Poll::Ready(Ok(Dispatched::Shutdown))
                    } else {
                        Poll::Ready(Err(Error::new_h2(err)))
                    };
                }
                Poll::Pending => {
                    // Diagnose why poll_ready is parked: whether the h2
                    // connection is at its stream-capacity limit, has no
                    // active streams at all (pending open can never succeed),
                    // or the keep-alive/connection already marked closed.
                    trace!(
                        "h2 poll_ready pending; num_active_streams={}, max_concurrent_send_streams={}, conn_closed={}",
                        self.h2_tx.num_active_streams(),
                        self.h2_tx.current_max_send_streams(),
                        self.ping.is_conn_closed(),
                    );
                    // The connection task may have ended (keep-alive timeout,
                    // GOAWAY, I/O error) while this task waited for the h2
                    // channel to become ready. Poll the EOF signal so we
                    // don't park forever on a dead connection that the pool
                    // may keep handing requests to.
                    if self.conn_is_eof(cx) {
                        trace!(
                            "connection task closed while awaiting readiness; closing dispatch task"
                        );
                        return Poll::Ready(Ok(Dispatched::Shutdown));
                    }
                    // The connection was explicitly marked closed (keep-alive
                    // timeout, GOAWAY, I/O error) even though the ConnTask may
                    // still be draining the socket. Fail fast instead of
                    // parking on stream capacity that will never arrive.
                    if self.ping.is_conn_closed()
                        || self
                            .closed
                            .as_mut()
                            .is_some_and(|closed| closed.as_mut().poll(cx).is_ready())
                    {
                        trace!(
                            "connection marked closed while awaiting readiness; closing dispatch task"
                        );
                        return Poll::Ready(Ok(Dispatched::Shutdown));
                    }
                    // Last-resort guard: the h2 channel must accept a request
                    // within a bounded time. If it stays parked past the
                    // deadline the connection is wedged (pending stream can
                    // never open) even though keep-alive pings still flow;
                    // tear the connection down so the pool rebuilds it.
                    if self.poll_ready_deadline.is_none() {
                        self.poll_ready_deadline = Some(self.timer.sleep(POLL_READY_TIMEOUT));
                    }
                    if self
                        .poll_ready_deadline
                        .as_mut()
                        .is_some_and(|sleep| sleep.as_mut().poll(cx).is_ready())
                    {
                        trace!("h2 channel unready for too long; closing dispatch task");
                        return Poll::Ready(Ok(Dispatched::Shutdown));
                    }
                    // Bound any lost wake-up on the h2 channel: re-run the
                    // full loop every interval so a request queued in the
                    // buffer is drained as soon as the channel frees up.
                    if self.dispatch_watchdog_fired(cx) {
                        continue;
                    }
                    return Poll::Pending;
                }
            }

            // If we were waiting on pending open
            // continue where we left off.
            if let Some(f) = self.fut_ctx.take() {
                self.poll_pipe(f, cx);
                continue;
            }

            match self.req_rx.poll_recv(cx) {
                Poll::Ready(Some((req, cb))) => {
                    trace!("client dispatch received request conn_id={}", self.conn_id);
                    // Check that future hasn't been canceled already
                    if cb.is_canceled() {
                        trace!("request callback is canceled");
                        continue;
                    }
                    let (head, body) = req.into_parts();
                    let mut req = ::http::Request::from_parts(head, ());
                    headers::strip_connection_headers(req.headers_mut(), true);
                    if let Some(len) = body.size_hint().exact()
                        && (len != 0 || headers::method_has_defined_payload_semantics(req.method()))
                    {
                        headers::set_content_length_if_missing(req.headers_mut(), len);
                    }

                    // Sort headers if we have the original headers
                    if let Some(orig_headers) =
                        RequestConfig::<OrigHeaderMap>::remove(req.extensions_mut())
                    {
                        orig_headers.sort_headers(req.headers_mut());
                    }

                    let is_connect = req.method() == Method::CONNECT;
                    let eos = body.is_end_stream();

                    if is_connect
                        && headers::content_length_parse_all(req.headers())
                            .is_some_and(|len| len != 0)
                    {
                        debug!("h2 connect request with non-zero body not supported");
                        cb.send(Err(TrySendError {
                            error: Error::new_user_invalid_connect(),
                            message: None,
                        }));
                        continue;
                    }

                    trace!(
                        "client dispatch calling send_request conn_id={}",
                        self.conn_id
                    );
                    let (fut, body_tx) = match self.h2_tx.send_request(req, !is_connect && eos) {
                        Ok(ok) => ok,
                        Err(err) => {
                            debug!("client send request error: {}", err);
                            cb.send(Err(TrySendError {
                                error: Error::new_h2(err),
                                message: None,
                            }));
                            continue;
                        }
                    };

                    let f = FutCtx {
                        is_connect,
                        eos,
                        fut,
                        body_tx,
                        body,
                        cb,
                    };

                    // Check poll_ready() again.
                    // If the call to send_request() resulted in the new stream being pending open
                    // we have to wait for the open to complete before accepting new requests.
                    match self.h2_tx.poll_ready(cx) {
                        Poll::Pending => {
                            // Don't wait indefinitely for the stream to open:
                            // if the connection task has died (keep-alive
                            // timeout, GOAWAY, I/O error) the stream will
                            // never open and nothing will wake this task.
                            // Fail the pending request immediately so the
                            // caller can retry on a fresh connection instead
                            // of hanging until its request timeout.
                            if self.conn_is_eof(cx) {
                                trace!(
                                    "connection task closed while awaiting stream open; failing pending request"
                                );
                                f.cb.send(Err(TrySendError {
                                    error: Error::new_h2(::http2::Reason::NO_ERROR.into()),
                                    message: None,
                                }));
                                return Poll::Ready(Ok(Dispatched::Shutdown));
                            }
                            // The connection may have been explicitly marked
                            // closed (keep-alive timeout, GOAWAY, I/O error)
                            // while the ConnTask is still draining the socket.
                            // The pending stream will never open; fail the
                            // request fast instead of parking until the
                            // caller's request timeout.
                            if self.ping.is_conn_closed()
                                || self
                                    .closed
                                    .as_mut()
                                    .is_some_and(|closed| closed.as_mut().poll(cx).is_ready())
                            {
                                trace!(
                                    "connection marked closed while awaiting stream open; failing pending request"
                                );
                                f.cb.send(Err(TrySendError {
                                    error: Error::new_h2(::http2::Reason::NO_ERROR.into()),
                                    message: None,
                                }));
                                return Poll::Ready(Ok(Dispatched::Shutdown));
                            }
                            // Last-resort guard for a wedged h2 channel: even
                            // when keep-alive pings still flow, a pending
                            // stream may never open (observed in the wild).
                            // Bounded wait, then fail the pending request and
                            // let the pool rebuild the connection.
                            if self.poll_ready_deadline.is_none() {
                                self.poll_ready_deadline =
                                    Some(self.timer.sleep(POLL_READY_TIMEOUT));
                            }
                            if self
                                .poll_ready_deadline
                                .as_mut()
                                .is_some_and(|sleep| sleep.as_mut().poll(cx).is_ready())
                            {
                                trace!(
                                    "h2 stream pending open for too long; failing pending request"
                                );
                                f.cb.send(Err(TrySendError {
                                    error: Error::new_h2(::http2::Reason::NO_ERROR.into()),
                                    message: None,
                                }));
                                return Poll::Ready(Ok(Dispatched::Shutdown));
                            }
                            // Save Context
                            self.fut_ctx = Some(f);
                            // Bound any lost wake-up while the stream is
                            // pending open: re-run the full loop every
                            // interval. If the stream opened in the meantime
                            // (but its wake-up was lost), poll_ready now
                            // returns Ready and the response is polled below;
                            // if it is still pending, the deadline above
                            // eventually fails the request and rebuilds the
                            // connection.
                            if self.dispatch_watchdog_fired(cx) {
                                continue;
                            }
                            return Poll::Pending;
                        }
                        Poll::Ready(Ok(())) => (),
                        Poll::Ready(Err(err)) => {
                            f.cb.send(Err(TrySendError {
                                error: Error::new_h2(err),
                                message: None,
                            }));
                            continue;
                        }
                    }
                    self.poll_pipe(f, cx);
                }

                Poll::Ready(None) => {
                    trace!("client::dispatch::Sender dropped");
                    return Poll::Ready(Ok(Dispatched::Shutdown));
                }

                Poll::Pending => {
                    // Watchdog against lost wake-ups: if a request is queued
                    // but this task is never woken (observed in the wild:
                    // try_send succeeds while the dispatch task parks
                    // forever), the request would sit in the buffer until the
                    // caller's request timeout (~15s). A tokio timer wake is
                    // independent of the mpsc wake, so polling it guarantees
                    // this task re-runs at least every DISPATCH_WATCHDOG_INTERVAL
                    // and poll_recv drains any buffered request. Idle
                    // connections only wake once per interval for a no-op.
                    if self.dispatch_watchdog_fired(cx) {
                        // Re-enter the loop so `poll_ready` and `poll_recv`
                        // are polled again: a request sitting in the buffer
                        // (whose mpsc wake was lost) is now drained and
                        // dispatched instead of staying queued until the
                        // caller's timeout.
                        continue;
                    }
                    match ready!(Pin::new(&mut self.conn_eof).poll(cx)) {
                        // As of Rust 1.82, this pattern is no longer needed, and emits a warning.
                        // But we cannot remove it as long as MSRV is less than that.
                        Ok(never) => match never {},
                        Err(_conn_is_eof) => {
                            trace!("connection task is closed, closing dispatch task");
                            return Poll::Ready(Ok(Dispatched::Shutdown));
                        }
                    }
                }
            }
        }
    }
}
