//! Async stream wrapper that parses SSE events from an underlying byte stream.
//!
//! Vendored from [sse-core](https://github.com/PizzasBear/sse-rs) and adapted for hpx.

use std::{
    fmt,
    pin::Pin,
    sync::Arc,
    task::{self, Poll, ready},
};

use bytes::Buf;
use futures_util::{
    TryStream,
    stream::{FusedStream, Stream},
};
use pin_project_lite::pin_project;

use super::decode::{PayloadTooLargeError, SseDecoder, SseEvent};

pin_project! {
    /// An asynchronous stream wrapper that parses SSE events from an underlying byte stream.
    #[derive(Debug)]
    pub struct SseStream<T: TryStream> {
        #[pin]
        inner: Option<T>,
        buf: Option<T::Ok>,
        decoder: SseDecoder,
    }
}

/// Errors that can occur while reading from an [`SseStream`].
#[derive(Debug)]
pub enum SseStreamError<T> {
    /// A single field exceeded the configured byte limit.
    PayloadTooLarge(PayloadTooLargeError),
    /// An error propagated from the inner [`TryStream`].
    Inner(T),
}

impl<T: fmt::Display> fmt::Display for SseStreamError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PayloadTooLarge(e) => write!(f, "{e}"),
            Self::Inner(e) => write!(f, "{e}"),
        }
    }
}

impl<T: std::error::Error + 'static> std::error::Error for SseStreamError<T> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PayloadTooLarge(e) => Some(e),
            Self::Inner(e) => Some(e),
        }
    }
}

impl<T> From<T> for SseStreamError<T> {
    fn from(inner: T) -> Self {
        Self::Inner(inner)
    }
}

impl<T: TryStream> SseStream<T> {
    /// Creates a new, disconnected [`SseStream`].
    #[inline]
    #[must_use]
    pub fn disconnected() -> Self {
        Self::with_decoder(SseDecoder::new())
    }

    /// Creates a disconnected stream initialized with a custom decoder.
    #[inline]
    #[must_use]
    pub fn with_decoder(decoder: SseDecoder) -> Self {
        Self {
            inner: None,
            buf: None,
            decoder,
        }
    }

    /// Creates a new [`SseStream`] wrapping the provided inner stream.
    #[inline]
    #[must_use]
    pub fn new(inner: T) -> Self {
        let mut slf = Self::disconnected();
        slf.inner = Some(inner);
        slf
    }

    /// Consumes the stream and returns the underlying state-machine decoder.
    #[inline]
    pub fn take_decoder(self) -> SseDecoder {
        let Self { mut decoder, .. } = self;
        decoder.reconnect();
        decoder
    }

    /// Returns `true` if the stream is currently disconnected.
    #[inline]
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.is_none()
    }

    /// Returns the current `Last-Event-ID` parsed by the underlying decoder.
    #[inline]
    #[must_use]
    pub fn last_event_id(&self) -> Option<&Arc<str>> {
        self.decoder.last_event_id()
    }

    /// Disconnects the inner stream while retaining the underlying parser's state.
    #[inline]
    pub fn close(&mut self) {
        self.decoder.reconnect();
        self.clear_bufs();
    }

    /// Disconnects the stream and completely purges the underlying parser's state.
    #[inline]
    pub fn close_and_clear(&mut self) {
        self.decoder.clear();
        self.clear_bufs();
    }

    /// Disconnects the inner stream and explicitly overrides the `Last-Event-ID`.
    #[inline]
    pub fn close_with_id(&mut self, id: Option<Arc<str>>) {
        self.decoder.reconnect_with_id(id);
        self.clear_bufs();
    }

    /// Attaches a new inner stream to resume processing events.
    #[inline]
    pub fn attach(&mut self, inner: T) {
        self.close();
        self.inner = Some(inner);
    }

    /// Attaches a new inner stream and completely purges the underlying parser's state.
    #[inline]
    pub fn clear_and_attach(&mut self, inner: T) {
        self.close_and_clear();
        self.inner = Some(inner);
    }

    /// Attaches a new inner stream with an explicit `Last-Event-ID` override.
    #[inline]
    pub fn attach_with_id(&mut self, inner: T, id: Option<Arc<str>>) {
        self.close_with_id(id);
        self.inner = Some(inner);
    }

    #[inline]
    fn clear_bufs(&mut self) {
        self.inner = None;
        self.buf = None;
    }
}

impl<T: TryStream> Stream for SseStream<T>
where
    T::Ok: Buf,
{
    type Item = Result<SseEvent, SseStreamError<T::Error>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        let mut slf = self.project();

        let Some(mut inner) = slf.inner.as_mut().as_pin_mut() else {
            return Poll::Ready(None);
        };

        loop {
            if let Some(event) = (slf.buf.as_mut())
                .and_then(|buf| slf.decoder.next(buf))
                .transpose()
                .map_err(SseStreamError::PayloadTooLarge)?
            {
                return Poll::Ready(Some(Ok(event)));
            };

            *slf.buf = ready!(inner.as_mut().try_poll_next(cx)?);
            if slf.buf.is_none() {
                slf.inner.set(None);
                return Poll::Ready(None);
            }
        }
    }
}

impl<T: TryStream> FusedStream for SseStream<T>
where
    T::Ok: Buf,
{
    fn is_terminated(&self) -> bool {
        self.is_closed()
    }
}
