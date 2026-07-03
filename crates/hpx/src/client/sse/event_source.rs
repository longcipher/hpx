//! A reconnecting stream of Server-Sent Events built on hpx.

use std::{
    fmt,
    future::Future,
    num::NonZeroUsize,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, ready},
    time::Duration,
};

use bytes::Bytes;
use futures_util::stream::Stream;
use tokio::time::{Instant, Sleep, sleep};

use super::{
    decode::{MessageEvent, PayloadTooLargeError, SseDecoder, SseEvent as SseEventCore},
    retry::SseRetryConfig,
    stream::{SseStream, SseStreamError},
};
use crate::{
    Error as HpxError, RequestBuilder,
    header::{HeaderName, HeaderValue},
};

type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, crate::Error>> + Send>>;
type ConnectFuture = Pin<Box<dyn Future<Output = crate::Result<crate::Response>> + Send>>;

/// Errors that can occur during the lifecycle of an [`EventSource`] connection.
#[derive(Debug)]
pub enum Error {
    /// The server responded with a non-200 HTTP status code.
    Status(http::StatusCode),
    /// The server's response lacked the `text/event-stream` Content-Type.
    InvalidContentType,
    /// The server's response did not contain a Content-Type header.
    MissingContentType,
    /// The client exhausted all retry attempts without successfully reconnecting.
    Timeout(u32, SseErrorEvent),
    /// The server sent an event payload that exceeded the configured buffer limit.
    PayloadTooLarge(PayloadTooLargeError),
    /// An underlying hpx transport error.
    Transport(HpxError),
    /// The `Last-Event-ID` cannot be converted to a valid HTTP header.
    InvalidLastEventId(http::header::InvalidHeaderValue),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(s) => write!(f, "unexpected HTTP status code: {s}"),
            Self::InvalidContentType => write!(f, "invalid response HTTP Content-Type"),
            Self::MissingContentType => write!(f, "response HTTP Content-Type missing"),
            Self::Timeout(attempts, cause) => {
                write!(f, "couldn't reconnect in {attempts} attempts: {cause}")
            }
            Self::PayloadTooLarge(_) => write!(f, "server sent an oversized payload"),
            Self::Transport(e) => write!(f, "{e}"),
            Self::InvalidLastEventId(e) => write!(f, "invalid Last-Event-ID header: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PayloadTooLarge(e) => Some(e),
            Self::Transport(e) => Some(e),
            Self::InvalidLastEventId(e) => Some(e),
            _ => None,
        }
    }
}

impl From<HpxError> for Error {
    fn from(e: HpxError) -> Self {
        Self::Transport(e)
    }
}

/// The connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReadyState {
    /// The connection has not yet been established, or it was closed and the client is reconnecting.
    Connecting = 0,
    /// The connection is open and ready to receive events.
    Open = 1,
    /// The connection is permanently closed and will not reconnect.
    Closed = 2,
}

enum State {
    Disconnected,
    Connecting(ConnectFuture),
    Open,
    Sleeping(Pin<Box<Sleep>>),
    Closed,
}

impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected => f.write_str("Disconnected"),
            Self::Connecting(_) => f.write_str("Connecting(_)"),
            Self::Open => f.write_str("Open"),
            Self::Sleeping(fut) => f.debug_tuple("Sleeping").field(fut).finish(),
            Self::Closed => f.write_str("Closed"),
        }
    }
}

/// Transient errors that cause the stream to drop and trigger an automatic reconnection.
#[derive(Debug)]
pub enum SseErrorEvent {
    /// The server gracefully closed the TCP connection (EOF) while the stream was active.
    Eof,
    /// The server responded with an HTTP status code designated as retryable.
    Http(http::StatusCode),
    /// A network-level error occurred.
    Network(HpxError),
}

impl fmt::Display for SseErrorEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Eof => write!(f, "server cleanly closed the connection (EOF)"),
            Self::Http(s) => write!(f, "transient HTTP error: {s}"),
            Self::Network(e) => write!(f, "network or transport error: {e}"),
        }
    }
}

impl std::error::Error for SseErrorEvent {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Network(e) => Some(e),
            _ => None,
        }
    }
}

/// High-level events emitted by the [`EventSource`] stream.
#[derive(Debug)]
pub enum SseEvent {
    /// Emitted when the underlying HTTP connection is successfully established.
    Open,
    /// A parsed message event from the server.
    Message(MessageEvent),
    /// Emitted when the connection drops but the client is actively attempting to reconnect.
    Error(SseErrorEvent),
}

impl SseEvent {
    /// Returns the underlying [`MessageEvent`] if this is a standard message.
    pub fn into_message(self) -> Option<MessageEvent> {
        match self {
            Self::Message(msg) => Some(msg),
            Self::Open | Self::Error(_) => None,
        }
    }

    /// Returns a reference to the underlying [`MessageEvent`] if this is a standard message.
    pub fn as_message(&self) -> Option<&MessageEvent> {
        match self {
            Self::Message(msg) => Some(msg),
            Self::Open | Self::Error(_) => None,
        }
    }
}

impl From<MessageEvent> for SseEvent {
    fn from(event: MessageEvent) -> Self {
        Self::Message(event)
    }
}

/// A builder for configuring an [`EventSource`] connection.
pub struct EventSourceBuilder {
    req: RequestBuilder,
    retry_config: SseRetryConfig,
    reconnection_time_ms: u32,
    max_payload_size: Option<NonZeroUsize>,
    last_event_id: Option<Arc<str>>,
    retry_transient_errors: bool,
    successful_connection_threshold: Duration,
}

impl EventSourceBuilder {
    /// Creates a new builder wrapping the given [`RequestBuilder`].
    #[must_use]
    pub fn new(req: RequestBuilder) -> Self {
        Self {
            req,
            reconnection_time_ms: 3000,
            retry_config: SseRetryConfig::new(),
            max_payload_size: None,
            last_event_id: None,
            retry_transient_errors: false,
            successful_connection_threshold: Duration::from_secs(5),
        }
    }

    /// Applies a custom retry configuration for automatic reconnections.
    #[inline]
    #[must_use]
    pub fn retry_config(mut self, retry_config: SseRetryConfig) -> Self {
        self.retry_config = retry_config;
        self
    }

    /// Sets the base delay to wait before attempting to reconnect.
    ///
    /// This delay may be overridden by the server using `retry` events.
    #[inline]
    #[must_use]
    pub fn initial_reconnection_time(mut self, reconnection_time: Duration) -> Self {
        self.reconnection_time_ms = reconnection_time
            .as_millis()
            .try_into()
            .expect("reconnection time too long");
        self
    }

    /// Configures the maximum allowed byte size for a single event payload.
    #[inline]
    #[must_use]
    pub fn max_payload_size(mut self, max_payload_size: NonZeroUsize) -> Self {
        self.max_payload_size = Some(max_payload_size);
        self
    }

    /// Sets the initial `Last-Event-ID` to send with the first connection request.
    #[inline]
    #[must_use]
    pub fn last_event_id(mut self, id: impl Into<Arc<str>>) -> Self {
        self.last_event_id = Some(id.into());
        self
    }

    /// Enables automatic retries for transient HTTP status codes (408, 429, 502, 503, 504).
    #[inline]
    #[must_use]
    pub fn retry_transient_errors(mut self, retry: bool) -> Self {
        self.retry_transient_errors = retry;
        self
    }

    /// Sets the minimum duration a connection must remain open to reset the backoff counter.
    #[inline]
    #[must_use]
    pub fn successful_connection_threshold(mut self, threshold: Duration) -> Self {
        self.successful_connection_threshold = threshold;
        self
    }

    /// Consumes the builder and returns the configured [`EventSource`].
    #[must_use]
    pub fn build(self) -> EventSource {
        let mut decoder = match self.max_payload_size {
            Some(max_payload_size) => SseDecoder::with_limit(max_payload_size),
            None => SseDecoder::new(),
        };
        decoder.reconnect_with_id(self.last_event_id);

        EventSource {
            req: self.req,
            reconnection_time_ms: self.reconnection_time_ms,
            connection_attempt: 0,
            connected_since: None,
            retry_config: self.retry_config,
            retry_transient_errors: self.retry_transient_errors,
            successful_connection_threshold: self.successful_connection_threshold,
            stream: SseStream::with_decoder(decoder),
            state: State::Disconnected,
        }
    }
}

/// A reconnecting stream of Server-Sent Events.
pub struct EventSource {
    req: RequestBuilder,
    reconnection_time_ms: u32,
    connection_attempt: u32,
    connected_since: Option<Instant>,
    retry_config: SseRetryConfig,
    retry_transient_errors: bool,
    successful_connection_threshold: Duration,
    stream: SseStream<ByteStream>,
    state: State,
}

impl fmt::Debug for EventSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventSource")
            .field("reconnection_time_ms", &self.reconnection_time_ms)
            .field("connection_attempt", &self.connection_attempt)
            .field("retry_config", &self.retry_config)
            .field("retry_transient_errors", &self.retry_transient_errors)
            .field("state", &self.state)
            .field(
                "stream.last_event_id()",
                &self.stream.last_event_id().map(|id| &**id),
            )
            .field("stream.is_closed()", &self.stream.is_closed())
            .finish_non_exhaustive()
    }
}

impl EventSource {
    /// Creates a new [`EventSource`] from the given request with default configurations.
    #[must_use]
    pub fn new(req: RequestBuilder) -> Self {
        Self::builder(req).build()
    }

    /// Creates a builder to customize the [`EventSource`] before connecting.
    #[must_use]
    pub fn builder(req: RequestBuilder) -> EventSourceBuilder {
        EventSourceBuilder::new(req)
    }

    /// Closes the underlying SSE connection.
    pub fn close(&mut self) {
        self.stream.close();
        self.state = State::Closed;
    }

    /// Returns the current connection state.
    #[inline]
    #[must_use]
    pub fn ready_state(&self) -> ReadyState {
        match &self.state {
            State::Disconnected | State::Connecting(_) | State::Sleeping(_) => {
                ReadyState::Connecting
            }
            State::Open => ReadyState::Open,
            State::Closed => ReadyState::Closed,
        }
    }

    /// Returns the most recently received `Last-Event-ID`, if any.
    #[inline]
    #[must_use]
    pub fn last_event_id(&self) -> Option<&Arc<str>> {
        self.stream.last_event_id()
    }

    /// Terminates the current connection and immediately attempts to reconnect.
    #[inline]
    pub fn force_reconnect(&mut self) {
        self.stream.close();
        self.connection_attempt = 0;
        self.state = State::Disconnected;
    }

    /// Terminates the current connection and immediately attempts to reconnect,
    /// explicitly overriding the `Last-Event-ID` sent to the server.
    #[inline]
    pub fn force_reconnect_with_id(&mut self, id: Option<Arc<str>>) {
        self.stream.close_with_id(id);
        self.connection_attempt = 0;
        self.state = State::Disconnected;
    }

    fn go_to_sleep(&mut self, cause: SseErrorEvent) -> Result<SseEvent, Error> {
        if let Some(connected_since) = self.connected_since.take()
            && self.successful_connection_threshold <= connected_since.elapsed()
        {
            self.connection_attempt = 0;
        }

        let wait_dur = self
            .retry_config
            .calculate_backoff(self.reconnection_time_ms, self.connection_attempt);
        self.connection_attempt += 1;
        if let Some(dur) = wait_dur {
            self.stream.close();
            self.state = State::Sleeping(Box::pin(sleep(dur)));
            Ok(SseEvent::Error(cause))
        } else {
            self.close();
            Err(Error::Timeout(self.connection_attempt, cause))
        }
    }
}

impl Stream for EventSource {
    type Item = Result<SseEvent, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let slf = &mut *self;

        loop {
            match &mut slf.state {
                State::Disconnected => {
                    let Some(mut req) = slf.req.try_clone() else {
                        slf.close();
                        return Poll::Ready(Some(Err(Error::Transport(crate::Error::builder(
                            "request builder could not be cloned",
                        )))));
                    };

                    if let Some(last_event_id) = slf.stream.last_event_id() {
                        match HeaderValue::from_str(last_event_id) {
                            Ok(val) => {
                                req = req.header(HeaderName::from_static("last-event-id"), val);
                            }
                            Err(err) => {
                                slf.close();
                                return Poll::Ready(Some(Err(Error::InvalidLastEventId(err))));
                            }
                        }
                    }

                    let fut = Box::pin(async move {
                        req.header(
                            http::header::ACCEPT,
                            HeaderValue::from_static("text/event-stream"),
                        )
                        .header(
                            http::header::CACHE_CONTROL,
                            HeaderValue::from_static("no-store"),
                        )
                        .send()
                        .await
                    });
                    slf.state = State::Connecting(fut);
                }

                State::Connecting(fut) => match ready!(fut.as_mut().poll(cx)) {
                    Ok(res) => {
                        let status = res.status();

                        if status == http::StatusCode::NO_CONTENT {
                            slf.close();
                            return Poll::Ready(None);
                        }

                        let is_transient_error = matches!(
                            status,
                            http::StatusCode::REQUEST_TIMEOUT
                                | http::StatusCode::TOO_MANY_REQUESTS
                                | http::StatusCode::BAD_GATEWAY
                                | http::StatusCode::SERVICE_UNAVAILABLE
                                | http::StatusCode::GATEWAY_TIMEOUT
                        );

                        if slf.retry_transient_errors && is_transient_error {
                            return Poll::Ready(Some(slf.go_to_sleep(SseErrorEvent::Http(status))));
                        } else if status != http::StatusCode::OK {
                            slf.close();
                            return Poll::Ready(Some(Err(Error::Status(status))));
                        }

                        let content_type = res
                            .headers()
                            .get(http::header::CONTENT_TYPE)
                            .map(|v| v.as_bytes());

                        let Some(content_type) = content_type else {
                            slf.close();
                            return Poll::Ready(Some(Err(Error::MissingContentType)));
                        };

                        const MIME_EVENT_STREAM: &[u8] = b"text/event-stream";
                        if !(content_type.starts_with(MIME_EVENT_STREAM)
                            && matches!(
                                content_type.get(MIME_EVENT_STREAM.len()),
                                None | Some(b';' | b' ' | b'\t')
                            ))
                        {
                            slf.close();
                            return Poll::Ready(Some(Err(Error::InvalidContentType)));
                        }

                        slf.state = State::Open;
                        slf.connected_since = Some(Instant::now());
                        slf.stream.attach(Box::pin(res.bytes_stream()));

                        return Poll::Ready(Some(Ok(SseEvent::Open)));
                    }
                    Err(err) => {
                        return Poll::Ready(Some(slf.go_to_sleep(SseErrorEvent::Network(err))));
                    }
                },

                State::Open => match ready!(Pin::new(&mut slf.stream).poll_next(cx)) {
                    Some(Ok(raw_event)) => match raw_event {
                        SseEventCore::Retry(ms) => slf.reconnection_time_ms = ms,
                        SseEventCore::Message(event) => {
                            return Poll::Ready(Some(Ok(event.into())));
                        }
                    },
                    Some(Err(SseStreamError::PayloadTooLarge(err))) => {
                        slf.close();
                        return Poll::Ready(Some(Err(Error::PayloadTooLarge(err))));
                    }
                    Some(Err(SseStreamError::Inner(err))) => {
                        return Poll::Ready(Some(slf.go_to_sleep(SseErrorEvent::Network(err))));
                    }
                    None => {
                        return Poll::Ready(Some(slf.go_to_sleep(SseErrorEvent::Eof)));
                    }
                },

                State::Sleeping(sleep_fut) => {
                    ready!(sleep_fut.as_mut().poll(cx));
                    slf.state = State::Disconnected;
                }

                State::Closed => return Poll::Ready(None),
            }
        }
    }
}

/// Extension trait for [`RequestBuilder`] to ergonomically create SSE streams.
pub trait RequestBuilderSseExt {
    /// Converts this request builder into an active [`EventSource`] with default settings.
    fn into_event_source(self) -> EventSource;
    /// Converts this request builder into an [`EventSourceBuilder`] for further configuration.
    fn into_event_source_builder(self) -> EventSourceBuilder;
}

impl RequestBuilderSseExt for RequestBuilder {
    fn into_event_source(self) -> EventSource {
        EventSource::new(self)
    }

    fn into_event_source_builder(self) -> EventSourceBuilder {
        EventSourceBuilder::new(self)
    }
}
