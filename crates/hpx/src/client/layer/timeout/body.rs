use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, ready},
    time::Duration,
};

use http_body::Body;
use pin_project_lite::pin_project;
use tokio::time::{Sleep, sleep};

use crate::{
    Error,
    error::{BoxError, TimedOut},
};

pin_project! {
    /// A wrapper body that applies timeout strategies to an inner HTTP body.
    #[project = TimeoutBodyProj]
    pub enum TimeoutBody<B> {
        Plain {
            #[pin]
            body: B,
        },
        TotalTimeout {
            #[pin]
            body: TotalTimeoutBody<B>,
        },
        ReadTimeout {
            #[pin]
            body: ReadTimeoutBody<B>
        },
        CombinedTimeout {
            #[pin]
            body: TotalTimeoutBody<ReadTimeoutBody<B>>,
        }
    }
}

pin_project! {
    /// A body wrapper that enforces a total timeout for the entire stream.
    ///
    /// The timeout applies to the whole body: if the deadline is reached before
    /// the body is fully read, an error is returned. The timer does **not** reset
    /// between chunks.
    pub struct TotalTimeoutBody<B> {
        #[pin]
        body: B,
        timeout: Pin<Box<Sleep>>,
    }
}

pin_project! {
    /// A body wrapper that enforces a timeout for each read operation.
    ///
    /// The timeout resets after every successful read. If a single read
    /// takes longer than the specified duration, an error is returned.
    /// The timer is held and reused via `Sleep::reset` (same shape as
    /// [`TotalTimeoutBody`]) instead of allocating a new timer per chunk.
    pub struct ReadTimeoutBody<B> {
        timeout: Duration,
        // NOTE: deliberately NOT `#[pin]` — like `TotalTimeoutBody::timeout`,
        // the field is already a `Pin<Box<Sleep>>`, so projection yields
        // `&mut Pin<Box<Sleep>>` and `.as_mut()` recovers `Pin<&mut Sleep>`
        // for `poll`/`reset`. Marking it `#[pin]` would double-pin and
        // break method resolution.
        sleep: Pin<Box<Sleep>>,
        #[pin]
        body: B,
    }
}

/// ==== impl TimeoutBody ====
impl<B> TimeoutBody<B> {
    /// Creates a new [`TimeoutBody`] with no timeout.
    pub(crate) fn new(deadline: Option<Duration>, read_timeout: Option<Duration>, body: B) -> Self {
        let deadline = deadline.map(sleep).map(Box::pin);
        match (deadline, read_timeout) {
            (Some(total_timeout), Some(read_timeout)) => Self::CombinedTimeout {
                body: TotalTimeoutBody {
                    timeout: total_timeout,
                    body: ReadTimeoutBody {
                        timeout: read_timeout,
                        sleep: Box::pin(sleep(read_timeout)),
                        body,
                    },
                },
            },
            (Some(timeout), None) => Self::TotalTimeout {
                body: TotalTimeoutBody { body, timeout },
            },
            (None, Some(timeout)) => Self::ReadTimeout {
                body: ReadTimeoutBody {
                    timeout,
                    sleep: Box::pin(sleep(timeout)),
                    body,
                },
            },
            (None, None) => Self::Plain { body },
        }
    }
}

impl<B> Body for TimeoutBody<B>
where
    B: Body,
    B::Error: Into<BoxError>,
{
    type Data = B::Data;
    type Error = BoxError;

    #[inline]
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        match self.project() {
            TimeoutBodyProj::TotalTimeout { body } => body.poll_frame(cx),
            TimeoutBodyProj::ReadTimeout { body } => body.poll_frame(cx),
            TimeoutBodyProj::CombinedTimeout { body } => body.poll_frame(cx),
            TimeoutBodyProj::Plain { body } => poll_and_map_body(body, cx),
        }
    }

    #[inline]
    fn size_hint(&self) -> http_body::SizeHint {
        match self {
            Self::TotalTimeout { body } => body.size_hint(),
            Self::ReadTimeout { body } => body.size_hint(),
            Self::CombinedTimeout { body } => body.size_hint(),
            Self::Plain { body } => body.size_hint(),
        }
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        match self {
            Self::TotalTimeout { body } => body.is_end_stream(),
            Self::ReadTimeout { body } => body.is_end_stream(),
            Self::CombinedTimeout { body } => body.is_end_stream(),
            Self::Plain { body } => body.is_end_stream(),
        }
    }
}

#[inline]
fn poll_and_map_body<B>(
    body: Pin<&mut B>,
    cx: &mut Context<'_>,
) -> Poll<Option<Result<http_body::Frame<B::Data>, BoxError>>>
where
    B: Body,
    B::Error: Into<BoxError>,
{
    Poll::Ready(ready!(body.poll_frame(cx)).map(|opt| opt.map_err(Error::body).map_err(Into::into)))
}

// ==== impl TotalTimeoutBody ====
impl<B> Body for TotalTimeoutBody<B>
where
    B: Body,
    B::Error: Into<BoxError>,
{
    type Data = B::Data;
    type Error = BoxError;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let this = self.project();
        if this.timeout.as_mut().poll(cx) == Poll::Ready(()) {
            return Poll::Ready(Some(Err(Error::body(TimedOut).into())));
        }
        poll_and_map_body(this.body, cx)
    }

    #[inline]
    fn size_hint(&self) -> http_body::SizeHint {
        self.body.size_hint()
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }
}

/// ==== impl ReadTimeoutBody ====
impl<B> Body for ReadTimeoutBody<B>
where
    B: Body,
    B::Error: Into<BoxError>,
{
    type Data = B::Data;
    type Error = BoxError;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let this = self.project();

        // Error if the per-read timeout has expired. The timer is reused
        // across chunks via `reset` instead of allocating a new `Sleep`.
        if this.sleep.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Some(Err(Box::new(TimedOut))));
        }

        // Poll the actual body
        match ready!(this.body.poll_frame(cx)) {
            Some(Ok(frame)) => {
                // Reset deadline for the next read, reusing the timer allocation.
                // Dropping the body (end of stream) cancels the timer.
                let deadline = tokio::time::Instant::now() + *this.timeout;
                this.sleep.as_mut().reset(deadline);
                Poll::Ready(Some(Ok(frame)))
            }
            Some(Err(err)) => Poll::Ready(Some(Err(err.into()))),
            None => Poll::Ready(None),
        }
    }

    #[inline]
    fn size_hint(&self) -> http_body::SizeHint {
        self.body.size_hint()
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        pin::Pin,
        task::{Context, Poll},
        time::Duration,
    };

    use bytes::Bytes;
    use http_body::{Body, Frame};
    use tokio::time::sleep;

    use super::{ReadTimeoutBody, TimeoutBody};
    use crate::error::TimedOut;

    struct ImmediateBody {
        remaining: usize,
    }

    impl Body for ImmediateBody {
        type Data = Bytes;
        type Error = std::convert::Infallible;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
            if self.remaining == 0 {
                Poll::Ready(None)
            } else {
                self.remaining -= 1;
                Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"chunk")))))
            }
        }
    }

    struct StalledBody;

    impl Body for StalledBody {
        type Data = Bytes;
        type Error = std::convert::Infallible;

        fn poll_frame(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
            Poll::Pending
        }
    }

    #[tokio::test]
    async fn read_timeout_body_delivers_fast_stream_without_false_trigger() {
        let inner = ImmediateBody { remaining: 2 };
        let mut body = Box::pin(ReadTimeoutBody {
            timeout: Duration::from_millis(100),
            sleep: Box::pin(sleep(Duration::from_millis(100))),
            body: inner,
        });

        for _ in 0..2 {
            let frame = futures_util::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await;
            assert!(frame.is_some());
            assert!(frame.unwrap().is_ok());
        }
        let end = futures_util::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await;
        assert!(end.is_none());
    }

    #[tokio::test]
    async fn read_timeout_body_still_triggers_on_stalled_stream() {
        let mut body = Box::pin(ReadTimeoutBody {
            timeout: Duration::from_millis(10),
            sleep: Box::pin(sleep(Duration::from_millis(10))),
            body: StalledBody,
        });

        let frame = tokio::time::timeout(
            Duration::from_secs(5),
            futures_util::future::poll_fn(|cx| body.as_mut().poll_frame(cx)),
        )
        .await
        .expect("stalled read must resolve via read timeout");
        let err = frame.expect("timeout returns an error frame").unwrap_err();
        assert!(
            err.downcast_ref::<TimedOut>().is_some(),
            "expected TimedOut, got {err:?}"
        );
    }

    #[tokio::test]
    async fn read_timeout_body_resets_deadline_between_chunks() {
        // Timeout wrapper built through the shared constructor must also
        // deliver a fast stream without false triggers.
        let inner = ImmediateBody { remaining: 3 };
        let mut body = Box::pin(TimeoutBody::new(None, Some(Duration::from_millis(50)), inner));

        for _ in 0..3 {
            let frame = futures_util::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await;
            assert!(frame.is_some());
            assert!(frame.unwrap().is_ok());
        }
        let end = futures_util::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await;
        assert!(end.is_none());
    }
}
