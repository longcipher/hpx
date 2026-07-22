//! HTTP/3 connection task.
//!
//! Mirrors [`crate::client::core::proto::h2::client`] but adapted to the h3
//! 0.0.8 API. The h3 `SendRequest` is `Clone` and its `send_request` is an
//! async fn (vs h2's `poll_ready` + `send_request` sync pair), so per-request
//! driving is offloaded to spawned tasks rather than polled inline.

use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use bytes::{Buf, Bytes};
use http::{Request, Response};
use http_body::Body;
use tokio::sync::mpsc;

use crate::{
    client::core::{
        self, Error,
        body::{DecodedLength, Incoming as IncomingBody},
        dispatch::{self, TrySendError},
        error::BoxError,
        proto::{Dispatched, headers},
    },
    error::H3Error,
};

/// Dispatch channel receiver type for h3.
///
/// Mirrors `proto::h2::client::ClientRx<B>` but lives under the h3 module.
#[allow(dead_code)] // HTTP/3 dispatch scaffolding; wired in by future tasks.
pub(crate) type ClientRx<B> = dispatch::Receiver<Request<B>, Response<IncomingBody>>;

/// HTTP/3 connection dispatch task.
///
/// Drives the h3 connection's [`hpx_h3::client::SendRequest`] by polling the
/// dispatch channel for incoming requests and spawning per-request tasks
/// that run the h3 request/response lifecycle (send_request → finish →
/// recv_response → drain body).
///
/// Unlike h2's `ConnTask` (which drives the h2 `Connection` future to
/// completion), the h3 connection-driver future is spawned separately by
/// [`QuicConnector::call`](crate::client::conn::quic::QuicConnector) and
/// feeds the terminal [`hpx_h3::error::ConnectionError`] to `close_rx`.
/// [`ConnTask`] polls `close_rx` to detect connection closure and shut down
/// the dispatch loop.
///
/// Per-request driving is offloaded to `tokio::spawn` because h3's
/// `send_request` is an `async fn` (not a sync `poll_ready` + `send_request`
/// pair like h2). Each spawned task owns a `Clone` of the `SendRequest` and
/// runs the full request lifecycle independently.
#[allow(dead_code)] // HTTP/3 dispatch scaffolding; wired in by future tasks.
pub(crate) struct ConnTask<B>
where
    B: Body,
    B: 'static,
    B: Unpin,
    B: Send,
    B::Data: Send,
{
    /// Cloneable h3 `SendRequest` for opening new request streams.
    send_request: hpx_h3::client::SendRequest<hpx_h3::quinn::OpenStreams, Bytes>,
    /// Dispatch channel receiver — receives `(Request<B>, Callback)` pairs.
    req_rx: ClientRx<B>,
    /// Receives the terminal `ConnectionError` when the driver task
    /// observes `poll_close` resolving.
    close_rx: mpsc::Receiver<hpx_h3::error::ConnectionError>,
}

impl<B> ConnTask<B>
where
    B: Body + 'static + Unpin + Send,
    B::Data: Send + 'static + Into<Bytes>,
    B::Error: Into<BoxError> + Send + Sync + 'static,
{
    /// Construct a new [`ConnTask`] from the components of an
    /// [`H3Connection`](crate::client::conn::quic::H3Connection).
    ///
    /// The caller is responsible for spawning this future (typically via
    /// `tokio::spawn`) so it runs concurrently with the connection-driver
    /// task that feeds `close_rx`.
    #[allow(dead_code)] // HTTP/3 dispatch scaffolding; wired in by future tasks.
    pub(crate) fn new(
        send_request: hpx_h3::client::SendRequest<hpx_h3::quinn::OpenStreams, Bytes>,
        req_rx: ClientRx<B>,
        close_rx: mpsc::Receiver<hpx_h3::error::ConnectionError>,
    ) -> Self {
        Self {
            send_request,
            req_rx,
            close_rx,
        }
    }
}

impl<B> Future for ConnTask<B>
where
    B: Body + 'static + Unpin + Send,
    B::Data: Send + 'static + Into<Bytes>,
    B::Error: Into<BoxError> + Send + Sync + 'static,
{
    type Output = core::Result<Dispatched>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // All fields are `Unpin` (SendRequest, dispatch::Receiver,
        // mpsc::Receiver are all Unpin), so ConnTask is auto-Unpin and
        // Pin::get_mut is safe.
        let this = self.get_mut();

        // 1. Check close_rx for connection closure (non-blocking).
        //    The driver task sends the terminal ConnectionError when
        //    `poll_close` resolves; the channel closes if the driver is
        //    dropped. Either way, the dispatch loop should shut down.
        match this.close_rx.poll_recv(cx) {
            Poll::Ready(Some(_err)) => {
                trace!("h3 connection closed by driver task");
                return Poll::Ready(Ok(Dispatched::Shutdown));
            }
            Poll::Ready(None) => {
                trace!("h3 driver task dropped close_tx");
                return Poll::Ready(Ok(Dispatched::Shutdown));
            }
            Poll::Pending => {}
        }

        // 2. Poll req_rx for new requests.
        match this.req_rx.poll_recv(cx) {
            Poll::Ready(Some((req, cb))) => {
                if cb.is_canceled() {
                    trace!("h3 request callback is canceled");
                    return Poll::Pending;
                }

                // Clone the SendRequest for the per-request task. h3's
                // SendRequest is Clone (via h3-quinn's OpenStreams: Clone),
                // which is the basis for the pool's Shared reservation
                // semantics for Ver::Http3 (C-05).
                let mut send_request = this.send_request.clone();
                // Spawn the per-request lifecycle on the tokio runtime.
                // The task owns its own clone of SendRequest and the
                // request/callback, driving send_request → finish →
                // recv_response → drain body independently.
                let join_handle = tokio::spawn(async move {
                    let result = drive_request(&mut send_request, req).await;
                    // Convert `(Error, Option<Request<B>>)` to
                    // `TrySendError<Request<B>>` to match `Callback::send`'s
                    // expected `Result<U, TrySendError<T>>` signature.
                    let cb_result =
                        result.map_err(|(error, message)| TrySendError { error, message });
                    cb.send(cb_result);
                });
                // Detach the JoinHandle; the per-request task completes
                // independently. `#[must_use]` consumed by `drop(...)`.
                drop(join_handle);

                Poll::Pending
            }
            Poll::Ready(None) => {
                trace!("h3 dispatch::Sender dropped");
                Poll::Ready(Ok(Dispatched::Shutdown))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Drive a single h3 request through the full lifecycle.
///
/// Performs the following steps:
/// 1. Strip connection headers (RFC 9110 §7.6.1) and set `Content-Length`
///    when the body size is known and the method has defined payload
///    semantics.
/// 2. Convert `Request<B>` to `Request<()>` (h3 requires `Request<()>`).
/// 3. Call `send_request.send_request(req).await` to open a new bidirectional
///    stream and send the request headers.
/// 4. Send the body via `stream.send_data(chunk).await` in a loop, polling
///    the body for frames via `poll_frame`. Trailers are ignored (Phase 1).
/// 5. Call `stream.finish()` to half-close the send direction.
/// 6. Call `stream.recv_response().await` to receive the response headers.
/// 7. Collect the response body via `stream.recv_data()` until `None`, then
///    deliver it via an `IncomingBody` channel so the caller can read it
///    through the standard `Body` API.
///
/// For CONNECT (Extended CONNECT) requests (RFC 9220):
/// - Steps 4-5 are skipped (CONNECT has no body; `finish` is called without
///   data to half-close the send direction).
/// - Step 7 is skipped; after a 200 OK response, the stream becomes a
///   bidirectional WebSocket data channel. The [`RequestStream`] is stored
///   in the response extensions as [`H3WebSocket`] so the caller can
///   extract it for WebSocket communication.
///
/// Any [`hpx_h3::error::StreamError`] is mapped to [`H3Error`] (and then to
/// [`Error`] via the `From<H3Error>` impl) per §5.1.9 of the design.
#[allow(clippy::result_large_err)]
pub(crate) async fn drive_request<B>(
    send_request: &mut hpx_h3::client::SendRequest<hpx_h3::quinn::OpenStreams, Bytes>,
    req: Request<B>,
) -> Result<Response<IncomingBody>, (Error, Option<Request<B>>)>
where
    B: Body + Send + Unpin,
    B::Data: Into<Bytes>,
    B::Error: Into<BoxError>,
{
    // 1. Split the request into parts. Strip connection headers (RFC 9110
    //    §7.6.1) and set Content-Length when the body size is known and the
    //    method has defined payload semantics (mirrors h2's logic at
    //    proto/h2/client.rs:559-566).
    let (head, mut body) = req.into_parts();
    let mut req = http::Request::from_parts(head, ());

    let is_connect = req.method() == http::Method::CONNECT;

    headers::strip_connection_headers(req.headers_mut(), true);
    if !is_connect
        && let Some(len) = body.size_hint().exact()
        && (len != 0 || headers::method_has_defined_payload_semantics(req.method()))
    {
        headers::set_content_length_if_missing(req.headers_mut(), len);
    }

    // 2. Open a new bidirectional stream and send the request headers.
    let mut stream = send_request
        .send_request(req)
        .await
        .map_err(|e| (stream_error_to_error(e), None::<Request<B>>))?;

    // 3. Send the request body via send_data, then call finish.
    //    Trailers are ignored (Phase 1 scope).
    //    For CONNECT (Extended CONNECT), skip body sending AND finish —
    //    CONNECT requests have no body, and the stream must stay open
    //    for bidirectional WebSocket communication after the response.
    if !is_connect {
        super::dispatch::send_request_body(&mut stream, &mut body).await?;
    }

    // 4. Receive the response headers. Handle graceful STOP_SENDING
    //    (H3_NO_ERROR) and non-graceful stream errors.
    let response = match super::dispatch::handle_response(&mut stream).await? {
        super::dispatch::RecvResponseResult::EarlyReturn(response) => return Ok(response),
        super::dispatch::RecvResponseResult::Success(response) => response,
    };

    // 5. For CONNECT (Extended CONNECT), do NOT collect the response body.
    //    After a 200 OK, the stream becomes a bidirectional WebSocket data
    //    channel. Store the stream in the response extensions so the caller
    //    can extract it via [`H3WebSocket::from_response`].
    if is_connect {
        let (body_tx, body_rx) =
            IncomingBody::new_channel(DecodedLength::CHUNKED, /* wanter = */ false);
        drop(body_tx); // Empty body — no data chunks for CONNECT.
        let mut response = response.map(|_| body_rx);
        response.extensions_mut().insert(H3WebSocket {
            inner: Arc::new(tokio::sync::Mutex::new(H3WebSocketInner {
                stream,
                recv_buf: Vec::new(),
            })),
            read_timeout: None,
            write_timeout: None,
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
        });
        return Ok(response);
    }

    // 5. Collect the response body and deliver it through an
    //    `IncomingBody` channel.
    let response = super::dispatch::collect_response_body(&mut stream, response).await?;
    Ok(response)
}

/// Map an [`hpx_h3::error::StreamError`] to an [`Error`] (the dispatch-level
/// `crate::client::core::Error`) via [`H3Error`].
///
/// Per §5.1.9 of the design, [`H3Error`] is the canonical HTTP/3 error type
/// (with `From<H3Error> for crate::error::Error` mapping each variant to the
/// appropriate `Kind`). The dispatch channel, however, carries
/// `crate::client::core::Error` (a separate type from `crate::error::Error`).
/// We bridge this by:
/// 1. Constructing the appropriate [`H3Error`] variant from the
///    [`hpx_h3::error::StreamError`].
/// 2. Wrapping it via [`Error::new_body`] (the `crate::client::core::Error`
///    constructor that accepts any `StdError + Send + Sync` cause). The
///    `Kind::Body` mapping is conservative because stream errors most
///    commonly occur during body transfer; the original [`H3Error`] is
///    preserved in the cause chain for callers that need the precise variant.
///
/// Variant mapping (where accessible — h3 0.0.8 marks several
/// `StreamError` variants as `#[non_exhaustive]`/private):
/// - `StreamError::StreamError { code, .. }` → `H3Error::StreamReset { code, stream_id: 0 }`
///   (stream_id is unknown at the mapping site; we use 0 as a sentinel
///   because h3 0.0.8's `RequestStream` does not expose the stream id).
/// - `StreamError::RemoteTerminate { code, .. }` → `H3Error::StreamReset { code, stream_id: 0 }`.
/// - `StreamError::HeaderTooBig { .. }` → `H3Error::MaxConcurrentStreamsExceeded`
///   (placeholder; the design does not have a `HeaderTooBig` variant, so we
///   reuse `MaxConcurrentStreamsExceeded` as the closest `Kind::Request`-
///   mapped variant).
/// - All other variants (including `ConnectionError`, `RemoteClosing`,
///   `Undefined`, which are private/non-exhaustive in h3 0.0.8 and cannot
///   be matched directly from outside the crate) → `H3Error::Other` with
///   the original `StreamError` boxed as the cause.
pub(crate) fn stream_error_to_error(err: hpx_h3::error::StreamError) -> Error {
    Error::new_body(stream_error_to_h3_error(err))
}

pub(crate) fn stream_error_to_h3_error(err: hpx_h3::error::StreamError) -> H3Error {
    match err {
        hpx_h3::error::StreamError::StreamError { code, .. } => H3Error::StreamReset {
            code: code.value(),
            stream_id: 0,
        },
        hpx_h3::error::StreamError::RemoteTerminate { code, .. } => H3Error::StreamReset {
            code: code.value(),
            stream_id: 0,
        },
        hpx_h3::error::StreamError::HeaderTooBig { .. } => H3Error::MaxConcurrentStreamsExceeded,
        other => H3Error::Other(Box::new(other)),
    }
}

/// A WebSocket message received from or to be sent over the h3 stream.
///
/// Variants correspond to WebSocket frame opcodes per RFC 6455 §5.2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsMessage {
    /// A text frame (opcode 0x1).
    Text(String),
    /// A binary frame (opcode 0x2).
    Binary(Vec<u8>),
    /// A close frame (opcode 0x8). Contains an optional status code and reason.
    Close {
        /// Optional close status code per RFC 6455 §7.4.
        code: Option<u16>,
        /// Optional close reason string.
        reason: Option<String>,
    },
    /// A ping frame (opcode 0x9).
    Ping(Vec<u8>),
    /// A pong frame (opcode 0xA).
    Pong(Vec<u8>),
}

/// Error type for WebSocket-over-h3 operations.
///
/// Wraps both underlying h3 stream errors and WebSocket framing errors.
#[derive(Debug)]
pub enum WsError {
    /// An error from the underlying h3 stream.
    Stream(hpx_h3::error::StreamError),
    /// The operation timed out.
    Timeout,
    /// The frame payload exceeds the configured maximum size.
    FrameTooLarge,
    /// The frame payload is malformed or invalid.
    FrameMalformed(&'static str),
}

impl std::fmt::Display for WsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WsError::Stream(e) => write!(f, "h3 stream error: {e}"),
            WsError::Timeout => write!(f, "WebSocket operation timed out"),
            WsError::FrameTooLarge => write!(f, "WebSocket frame payload exceeds max size"),
            WsError::FrameMalformed(reason) => write!(f, "WebSocket frame malformed: {reason}"),
        }
    }
}

impl std::error::Error for WsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WsError::Stream(e) => Some(e),
            _ => None,
        }
    }
}

impl From<hpx_h3::error::StreamError> for WsError {
    fn from(e: hpx_h3::error::StreamError) -> Self {
        WsError::Stream(e)
    }
}

/// Default maximum WebSocket frame payload size (1 MB).
const DEFAULT_MAX_FRAME_SIZE: usize = 1024 * 1024;

/// Internal state shared across clones of [`H3WebSocket`].
struct H3WebSocketInner {
    /// The underlying h3 request stream.
    stream: hpx_h3::client::RequestStream<hpx_h3::quinn::BidiStream<Bytes>, Bytes>,
    /// Buffer for accumulating partial WebSocket frame data from `recv_data()`.
    recv_buf: Vec<u8>,
}

/// A WebSocket-over-h3 transport backed by an h3 [`RequestStream`].
///
/// After an Extended CONNECT (RFC 9220) handshake, the h3 request stream
/// becomes a bidirectional WebSocket data channel. This struct wraps the
/// underlying [`hpx_h3::client::RequestStream`] and provides `send_text`,
/// `send_binary`, `send_close`, `send_ping`, `send_pong`, and `recv`
/// methods for exchanging properly framed WebSocket messages per RFC 6455.
///
/// Client-to-server frames are automatically masked with a random 4-byte key
/// as required by RFC 6455 §5.3. Server-to-client frames are unmasked.
///
/// The struct is stored in the response [`http::Extensions`] by
/// [`drive_request`] when the request method is `CONNECT`. Callers can
/// extract it via [`H3WebSocket::from_response`].
#[derive(Clone)]
pub struct H3WebSocket {
    /// Shared inner state (stream + receive buffer).
    inner: Arc<tokio::sync::Mutex<H3WebSocketInner>>,
    /// Read timeout for `recv()`. `None` means no timeout.
    read_timeout: Option<std::time::Duration>,
    /// Write timeout for `send_*()` methods. `None` means no timeout.
    write_timeout: Option<std::time::Duration>,
    /// Maximum allowed frame payload size in bytes. Frames exceeding this
    /// are rejected to prevent memory exhaustion.
    max_frame_size: usize,
}

impl std::fmt::Debug for H3WebSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("H3WebSocket")
            .field("read_timeout", &self.read_timeout)
            .field("write_timeout", &self.write_timeout)
            .field("max_frame_size", &self.max_frame_size)
            .finish()
    }
}

impl H3WebSocket {
    /// Create a new [`H3WebSocket`] from an h3 request stream.
    ///
    /// This is used by integration tests that construct the h3 connection
    /// directly without going through [`drive_request`]. Normal usage should
    /// use [`H3WebSocket::from_response`] instead.
    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn new(
        stream: hpx_h3::client::RequestStream<hpx_h3::quinn::BidiStream<Bytes>, Bytes>,
    ) -> Self {
        H3WebSocket {
            inner: Arc::new(tokio::sync::Mutex::new(H3WebSocketInner {
                stream,
                recv_buf: Vec::new(),
            })),
            read_timeout: None,
            write_timeout: None,
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
        }
    }

    /// Extract an [`H3WebSocket`] from a response after a successful
    /// Extended CONNECT handshake.
    ///
    /// Returns `None` if the response was not produced by an Extended
    /// CONNECT request (i.e., [`drive_request`] did not insert an
    /// `H3WebSocket` into the extensions).
    #[inline]
    pub fn from_response<B>(response: &mut http::Response<B>) -> Option<Self> {
        response.extensions_mut().remove::<H3WebSocket>()
    }

    /// Set the read timeout for `recv()` operations.
    ///
    /// When set, `recv()` will return an error if no data is received within
    /// the specified duration.
    #[allow(dead_code)]
    pub fn set_read_timeout(&mut self, timeout: Option<std::time::Duration>) {
        self.read_timeout = timeout;
    }

    /// Set the write timeout for `send_*()` operations.
    ///
    /// When set, `send_*()` methods will return an error if the data cannot
    /// be sent within the specified duration.
    #[allow(dead_code)]
    pub fn set_write_timeout(&mut self, timeout: Option<std::time::Duration>) {
        self.write_timeout = timeout;
    }

    /// Set the maximum allowed WebSocket frame payload size.
    ///
    /// Frames exceeding this limit will be rejected with an error.
    /// Default is 1 MB.
    #[allow(dead_code)]
    pub fn set_max_frame_size(&mut self, max_size: usize) {
        self.max_frame_size = max_size;
    }

    /// Send a WebSocket text frame (opcode 0x1).
    #[allow(dead_code)]
    pub async fn send_text(&mut self, data: &str) -> Result<(), WsError> {
        self.send_frame(0x1, data.as_bytes()).await
    }

    /// Send a WebSocket binary frame (opcode 0x2).
    #[allow(dead_code)]
    pub async fn send_binary(&mut self, data: &[u8]) -> Result<(), WsError> {
        self.send_frame(0x2, data).await
    }

    /// Send a WebSocket close frame (opcode 0x8).
    ///
    /// If `code` is provided, a 2-byte status code is prepended to the payload
    /// per RFC 6455 §5.5.1.
    #[allow(dead_code)]
    pub async fn send_close(
        &mut self,
        code: Option<u16>,
        reason: Option<&str>,
    ) -> Result<(), WsError> {
        let payload = build_close_payload(code, reason);
        self.send_frame(0x8, &payload).await
    }

    /// Send a WebSocket ping frame (opcode 0x9).
    #[allow(dead_code)]
    pub async fn send_ping(&mut self, data: &[u8]) -> Result<(), WsError> {
        self.send_frame(0x9, data).await
    }

    /// Send a WebSocket pong frame (opcode 0xA).
    #[allow(dead_code)]
    pub async fn send_pong(&mut self, data: &[u8]) -> Result<(), WsError> {
        self.send_frame(0xA, data).await
    }

    /// Receive and parse the next WebSocket frame from the stream.
    ///
    /// Returns `Ok(None)` when the stream has been closed cleanly (no more
    /// data). Returns an error if the frame is malformed, exceeds
    /// `max_frame_size`, or the read timeout expires.
    #[allow(dead_code)]
    pub async fn recv(&mut self) -> Result<Option<WsMessage>, WsError> {
        let mut inner = self.inner.lock().await;

        loop {
            // Try to parse a frame from the accumulated buffer.
            if let Some(msg) = Self::try_parse_frame(&mut inner.recv_buf, self.max_frame_size)? {
                // Auto-respond to ping with pong per RFC 6455 §5.5.3.
                if let WsMessage::Ping(ref data) = msg {
                    drop(inner);
                    self.send_pong(data).await?;
                    inner = self.inner.lock().await;
                    continue;
                }
                return Ok(Some(msg));
            }

            // Need more data — read from the stream.
            let chunk = if let Some(timeout) = self.read_timeout {
                match tokio::time::timeout(timeout, inner.stream.recv_data()).await {
                    Ok(Ok(Some(data))) => {
                        let mut data = data;
                        let len = data.remaining();
                        data.copy_to_bytes(len)
                    }
                    Ok(Ok(None)) => {
                        // Stream closed cleanly. If there's buffered data,
                        // it's an incomplete frame.
                        return Ok(None);
                    }
                    Ok(Err(e)) => return Err(WsError::Stream(e)),
                    Err(_elapsed) => return Err(WsError::Timeout),
                }
            } else {
                match inner.stream.recv_data().await {
                    Ok(Some(data)) => {
                        let mut data = data;
                        let len = data.remaining();
                        data.copy_to_bytes(len)
                    }
                    Ok(None) => return Ok(None),
                    Err(e) => return Err(WsError::Stream(e)),
                }
            };

            inner.recv_buf.extend_from_slice(&chunk);
        }
    }

    /// Finish the send direction of the stream.
    #[allow(dead_code)]
    pub async fn finish(&mut self) -> Result<(), WsError> {
        Ok(self.inner.lock().await.stream.finish().await?)
    }

    // ── private helpers ──

    /// Build and send a WebSocket frame with the given opcode and payload.
    /// Client frames are masked per RFC 6455 §5.3.
    async fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), WsError> {
        let frame = build_ws_frame(opcode, payload);
        let frame_bytes = Bytes::from(frame);

        let mut stream = self.inner.lock().await;
        let send_result = if let Some(timeout) = self.write_timeout {
            tokio::time::timeout(timeout, stream.stream.send_data(frame_bytes))
                .await
                .map_err(|_elapsed| WsError::Timeout)?
        } else {
            stream.stream.send_data(frame_bytes).await
        };
        Ok(send_result?)
    }

    /// Try to parse a complete WebSocket frame from the buffer.
    ///
    /// On success, returns `Ok(Some(message))` and consumes the frame bytes
    /// from the buffer. Returns `Ok(None)` if more data is needed. Returns
    /// `Err` if the frame is malformed or exceeds `max_frame_size`.
    fn try_parse_frame(
        buf: &mut Vec<u8>,
        max_frame_size: usize,
    ) -> Result<Option<WsMessage>, WsError> {
        if buf.len() < 2 {
            return Ok(None);
        }

        let byte0 = buf[0];
        let byte1 = buf[1];
        let _fin = byte0 & 0x80 != 0;
        let opcode = byte0 & 0x0F;
        let masked = byte1 & 0x80 != 0;
        let mut payload_len = (byte1 & 0x7F) as u64;

        // Determine header size.
        let mut header_len = 2usize;
        if payload_len == 126 {
            if buf.len() < 4 {
                return Ok(None);
            }
            payload_len = u16::from_be_bytes([buf[2], buf[3]]) as u64;
            header_len = 4;
        } else if payload_len == 127 {
            if buf.len() < 10 {
                return Ok(None);
            }
            payload_len = u64::from_be_bytes([
                buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8], buf[9],
            ]);
            header_len = 10;
        }

        // Mask key is present for client-to-server frames.
        // Server-to-client frames must NOT be masked (RFC 6455 §5.1).
        if masked {
            header_len += 4;
        }

        // Check if we have the full frame.
        let payload_len_usize = match usize::try_from(payload_len) {
            Ok(n) => n,
            Err(_) => return Err(WsError::FrameMalformed("payload length overflow")),
        };

        if payload_len_usize > max_frame_size {
            return Err(WsError::FrameTooLarge);
        }

        let total_len = header_len + payload_len_usize;
        if buf.len() < total_len {
            return Ok(None);
        }

        // Extract payload, applying mask if needed.
        let mut payload: Vec<u8> = buf[header_len..total_len].to_vec();
        if masked {
            let mask_key = [
                buf[header_len - 4],
                buf[header_len - 3],
                buf[header_len - 2],
                buf[header_len - 1],
            ];
            for (i, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask_key[i % 4];
            }
        }

        // Remove the frame from the buffer.
        buf.drain(..total_len);

        // Build the message from opcode.
        let msg = match opcode {
            0x1 => {
                let text = String::from_utf8(payload)
                    .map_err(|_| WsError::FrameMalformed("text frame contains invalid UTF-8"))?;
                WsMessage::Text(text)
            }
            0x2 => WsMessage::Binary(payload),
            0x8 => {
                let (code, reason) = parse_close_payload(&payload);
                WsMessage::Close { code, reason }
            }
            0x9 => WsMessage::Ping(payload),
            0xA => WsMessage::Pong(payload),
            _ => return Err(WsError::FrameMalformed("unknown opcode")),
        };

        Ok(Some(msg))
    }
}

// ── WebSocket framing helpers (free functions) ──

/// Build a complete WebSocket frame (client → server, masked).
///
/// Returns the raw bytes of the frame: header + mask key + masked payload.
fn build_ws_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let fin_and_opcode = 0x80 | (opcode & 0x0F); // FIN=1
    let mask_key = generate_mask_key();
    let len = payload.len();

    let mut frame = Vec::with_capacity(2 + 4 + len + 8); // 8 = max extended length
    frame.push(fin_and_opcode);

    // Payload length with MASK bit set.
    if len < 126 {
        frame.push(0x80 | len as u8);
    } else if len <= 0xFFFF {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }

    // Mask key.
    frame.extend_from_slice(&mask_key);

    // Masked payload.
    for (i, byte) in payload.iter().enumerate() {
        frame.push(byte ^ mask_key[i % 4]);
    }

    frame
}

/// Generate a random 4-byte mask key for client-to-server frames.
///
/// Uses `std::time::SystemTime` for entropy. WebSocket masking is not
/// cryptographic — it only needs to be unpredictable enough to thwart
/// cache-poisoning attacks (RFC 6455 §10.3).
fn generate_mask_key() -> [u8; 4] {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // Mix with a simple xorshift to get more entropy from the timestamp.
    let mut x = nanos as u64;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x.to_ne_bytes()[..4]
        .try_into()
        .unwrap_or([0x37, 0xfa, 0x21, 0x3d])
}

/// Build the payload for a close frame from an optional code and reason.
fn build_close_payload(code: Option<u16>, reason: Option<&str>) -> Vec<u8> {
    let mut payload = Vec::new();
    if let Some(c) = code {
        payload.extend_from_slice(&c.to_be_bytes());
    }
    if let Some(r) = reason {
        payload.extend_from_slice(r.as_bytes());
    }
    payload
}

/// Parse a close frame payload into an optional code and reason.
fn parse_close_payload(payload: &[u8]) -> (Option<u16>, Option<String>) {
    let code = if payload.len() >= 2 {
        Some(u16::from_be_bytes([payload[0], payload[1]]))
    } else {
        None
    };
    let reason = if payload.len() > 2 {
        String::from_utf8(payload[2..].to_vec()).ok()
    } else {
        None
    };
    (code, reason)
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, sync::Arc, time::Duration};

    use bytes::Bytes;
    use quinn::{Endpoint, ServerConfig, crypto::rustls::QuicServerConfig};
    use rustls::{
        RootCertStore, ServerConfig as RustlsServerConfig,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    };
    use tower::Service;

    use super::*;
    use crate::{
        Body,
        client::{
            conn::quic::{H3Connection, QuicConnector},
            core::{body::Incoming as IncomingBody, dispatch, http3::Http3Options},
            http::ConnectRequest,
        },
    };

    type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

    /// BDD Scenario: `ConnTask` drives a bodyless GET request through a real
    /// local h3 server and returns the response.
    ///
    /// Verifies the T1.9 behavioral contract: `ConnTask` polls the dispatch
    /// channel, spawns a per-request task that drives `send_request` →
    /// `finish` → `recv_response` → drain body, and fulfills the callback
    /// with the response.
    ///
    /// The test stands up a local h3 server (rcgen + rustls + quinn +
    /// hpx_h3::server), builds a `QuicConnector` to obtain an `H3Connection`,
    /// constructs a `ConnTask` from the connection's `send_request` +
    /// `close_rx` + a dispatch `Receiver`, spawns the `ConnTask`, sends a
    /// GET request through the dispatch `Sender`, and awaits the response
    /// via the `RetryPromise`.
    ///
    /// Scope (T1.9): exercises the bodyless GET path only. Request body
    /// sending, response body streaming, and full Client pipeline wiring
    /// are T1.10/T1.11 scope.
    #[tokio::test]
    async fn conn_task_drives_request_through_real_h3_server() -> TestResult<()> {
        // 1. Generate a self-signed certificate for the test server.
        let certified_key = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()])?;
        let cert_der: CertificateDer<'static> = certified_key.cert.der().clone();
        let key_der: PrivateKeyDer<'static> =
            PrivatePkcs8KeyDer::from(certified_key.signing_key.serialize_der()).into();

        // 2. Build the server-side rustls `ServerConfig` with ALPN `[b"h3"]`.
        //    Uses `builder_with_provider` with an explicit `ring` provider to
        //    avoid the CryptoProvider ambiguity between `aws-lc-rs` (rustls
        //    default) and `ring` (hpx explicit feature).
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut server_rustls_config = RustlsServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                format!("bad protocol versions: {e}").into()
            })?
            .with_no_client_auth()
            .with_single_cert(vec![cert_der.clone()], key_der)?;
        server_rustls_config.alpn_protocols = vec![b"h3".to_vec()];

        // 3. Build the quinn server endpoint bound to an ephemeral loopback port.
        let server_quic_config = QuicServerConfig::try_from(Arc::new(server_rustls_config))?;
        let server_config = ServerConfig::with_crypto(Arc::new(server_quic_config));
        let server_endpoint = Endpoint::server(server_config, "127.0.0.1:0".parse()?)?;
        let server_addr: SocketAddr = server_endpoint.local_addr()?;

        // 4. Spawn the server task. For each accepted QUIC connection, spawn
        //    a per-connection h3 driver task that accepts requests and
        //    responds with 200 OK + no body.
        let server_task = tokio::spawn(async move {
            while let Some(incoming) = server_endpoint.accept().await {
                match incoming.await {
                    Ok(quinn_conn) => {
                        tokio::spawn(async move {
                            let h3_quinn_conn = hpx_h3::quinn::Connection::new(quinn_conn);
                            let mut h3_conn: hpx_h3::server::Connection<
                                hpx_h3::quinn::Connection,
                                Bytes,
                            > = match hpx_h3::server::Connection::new(h3_quinn_conn).await {
                                Ok(c) => c,
                                Err(_) => return,
                            };
                            loop {
                                match h3_conn.accept().await {
                                    Ok(Some(resolver)) => {
                                        let (_req, mut stream) =
                                            match resolver.resolve_request().await {
                                                Ok(parts) => parts,
                                                Err(_) => continue,
                                            };
                                        let resp = match http::Response::builder()
                                            .status(http::StatusCode::OK)
                                            .body(())
                                        {
                                            Ok(r) => r,
                                            Err(_) => continue,
                                        };
                                        if stream.send_response(resp).await.is_err() {
                                            continue;
                                        }
                                        // Drain the request body so the
                                        // underlying quinn::RecvStream is
                                        // marked all_data_read before drop.
                                        while matches!(stream.recv_data().await, Ok(Some(_))) {}
                                        let _ = stream.finish().await;
                                    }
                                    Ok(None) => break,
                                    Err(_) => break,
                                }
                            }
                        });
                    }
                    Err(_) => break,
                }
            }
        });
        // Detach the server task; it is cancelled when the test runtime drops.
        let _ = server_task;

        // 5. Build the client-side rustls `ClientConfig` trusting the
        //    self-signed cert.
        let mut root_store = RootCertStore::empty();
        root_store.add(cert_der.clone())?;
        let client_provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut client_tls_config = rustls::ClientConfig::builder_with_provider(client_provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                format!("bad protocol versions: {e}").into()
            })?
            .with_root_certificates(root_store)
            .with_no_client_auth();
        client_tls_config.alpn_protocols = vec![b"h3".to_vec()];
        let tls_config: Arc<rustls::ClientConfig> = Arc::new(client_tls_config);

        // 6. Build the client `quinn::Endpoint` and a default transport config.
        let client_addr: SocketAddr = "127.0.0.1:0".parse()?;
        let client_endpoint = Endpoint::client(client_addr)?;
        let transport_config = Arc::new(quinn::TransportConfig::default());

        // 7. Construct the `QuicConnector` directly (bypasses `Client::build`,
        //    which is T1.10 scope).
        let mut connector = QuicConnector::new(
            client_endpoint,
            transport_config,
            tls_config,
            Http3Options::default(),
        );

        // 8. Call `QuicConnector::call` to obtain an `H3Connection`.
        let uri: http::Uri = format!("https://127.0.0.1:{}/", server_addr.port()).parse()?;
        let connect_req = ConnectRequest::new(uri, None);
        let waker = futures_util::task::noop_waker();
        let mut cx = std::task::Context::from_waker(&waker);
        match connector.poll_ready(&mut cx) {
            std::task::Poll::Ready(Ok(())) => {}
            other => return Err(format!("poll_ready should be Ok, got {other:?}").into()),
        }
        let h3_conn: H3Connection =
            tokio::time::timeout(Duration::from_secs(5), connector.call(connect_req))
                .await
                .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> {
                    "QuicConnector::call should resolve within 5s".into()
                })??;

        // 9. Build the dispatch channel for `Request<Body>` →
        //    `Response<IncomingBody>`.
        let (mut tx, rx) = dispatch::channel::<Request<Body>, Response<IncomingBody>>();

        // 10. Construct `ConnTask` from the `H3Connection`'s `send_request`
        //     + `close_rx` + the dispatch `Receiver`.
        let conn_task: ConnTask<Body> = ConnTask::new(h3_conn.send_request, rx, h3_conn.close_rx);

        // 11. Spawn `ConnTask` on the tokio runtime. It runs concurrently
        //     with the per-request tasks it spawns.
        let task_handle = tokio::spawn(async move { conn_task.await });

        // 12. Send a GET request through the dispatch `Sender`. The first
        //     message is always allowed by `dispatch::Sender::can_send`
        //     (buffered_once=false), so `try_send` succeeds even before the
        //     receiver is polled.
        let req = Request::get(format!("https://127.0.0.1:{}/", server_addr.port()))
            .body(Body::empty())?;
        let promise =
            tx.try_send(req)
                .map_err(|_req| -> Box<dyn std::error::Error + Send + Sync> {
                    "dispatch::Sender::try_send failed".into()
                })?;

        // 13. Await the response via the `RetryPromise` (a
        //     `oneshot::Receiver<Result<Response, TrySendError>>`). Bounded
        //     by a 5s timeout to keep the test from hanging on a regression.
        let response_result = tokio::time::timeout(Duration::from_secs(5), promise)
            .await
            .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> {
                "response promise should resolve within 5s".into()
            })?
            .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> {
                "oneshot receiver error".into()
            })?;
        let response =
            response_result.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                format!("dispatch returned error: {e:?}").into()
            })?;

        // 14. Assert the response status is 200 OK and the version is HTTP/3.
        assert_eq!(
            response.status(),
            http::StatusCode::OK,
            "expected 200 OK from ConnTask-driven request"
        );
        assert_eq!(
            response.version(),
            http::Version::HTTP_3,
            "expected HTTP/3 version from ConnTask-driven request"
        );

        // 15. Drop the dispatch `Sender` to shut down `ConnTask`
        //     (`req_rx.poll_recv` returns `None` → `Dispatched::Shutdown`).
        drop(tx);

        // 16. Wait for `ConnTask` to complete (bounded by 2s timeout).
        let _ = tokio::time::timeout(Duration::from_secs(2), task_handle).await;

        Ok(())
    }
}
