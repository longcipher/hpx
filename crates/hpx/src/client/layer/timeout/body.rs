use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, ready},
    time::Duration,
};

use http_body::Body;
use pin_project_lite::pin_project;
use tokio::time::{Instant, Sleep, sleep, sleep_until};

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
    /// The timer runs only while the inner body is pending, so it measures
    /// how long a read waits on the server, never the caller's own time
    /// between reads. If a single read waits longer than the specified
    /// duration, an error is returned.
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
        // Whether `sleep` is counting down for the current read.
        armed: bool,
        #[pin]
        body: B,
    }
}

/// ==== impl TimeoutBody ====
impl<B> TimeoutBody<B> {
    /// Creates a new [`TimeoutBody`] with no timeout.
    pub(crate) fn new(deadline: Option<Instant>, read_timeout: Option<Duration>, body: B) -> Self {
        let deadline = deadline.map(sleep_until).map(Box::pin);
        match (deadline, read_timeout) {
            (Some(total_timeout), Some(read_timeout)) => Self::CombinedTimeout {
                body: TotalTimeoutBody {
                    timeout: total_timeout,
                    body: ReadTimeoutBody {
                        timeout: read_timeout,
                        sleep: Box::pin(sleep(read_timeout)),
                        armed: false,
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
                    armed: false,
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

        // Poll the body before the timer, and run the timer only while the
        // body is pending: the read timeout measures waiting on the server,
        // so a frame that is already available is returned however long the
        // caller took to ask for it.
        if let Poll::Ready(frame) = this.body.poll_frame(cx) {
            // The next pending read starts a fresh countdown.
            *this.armed = false;
            return Poll::Ready(frame.map(|frame| frame.map_err(Into::into)));
        }

        if !*this.armed {
            // Reuse the timer allocation via `reset`. A timeout too large to
            // represent never fires, so the timer stays unarmed.
            let Some(deadline) = Instant::now().checked_add(*this.timeout) else {
                return Poll::Pending;
            };
            this.sleep.as_mut().reset(deadline);
            *this.armed = true;
        }

        if this.sleep.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Some(Err(Box::new(TimedOut))));
        }
        Poll::Pending
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
            armed: false,
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
            armed: false,
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
        let mut body = Box::pin(TimeoutBody::new(
            None,
            Some(Duration::from_millis(50)),
            inner,
        ));

        for _ in 0..3 {
            let frame = futures_util::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await;
            assert!(frame.is_some());
            assert!(frame.unwrap().is_ok());
        }
        let end = futures_util::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await;
        assert!(end.is_none());
    }

    #[tokio::test]
    async fn read_timeout_body_ignores_caller_time_between_reads() {
        // The read timeout measures waiting on the server. A frame that is
        // already available must be returned however long the caller spent
        // on its own work before asking for it, both before the first read
        // and between reads.
        let inner = ImmediateBody { remaining: 2 };
        let mut body = Box::pin(TimeoutBody::new(
            None,
            Some(Duration::from_millis(20)),
            inner,
        ));

        for _ in 0..2 {
            sleep(Duration::from_millis(60)).await;
            let frame = futures_util::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await;
            assert!(frame.is_some());
            assert!(
                frame.unwrap().is_ok(),
                "an available frame must not time out"
            );
        }
    }
}
