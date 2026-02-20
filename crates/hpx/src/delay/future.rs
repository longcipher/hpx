use std::{
    pin::Pin,
    task::{Context, Poll, ready},
};

use pin_project_lite::pin_project;
use tokio::time::Sleep;
use tower::{BoxError, Service};

pin_project! {
    /// Response future for delay middleware.
    #[derive(Debug)]
    #[project = ResponseFutureProj]
    pub enum ResponseFuture<S, Req>
    where
        S: Service<Req>,
    {
        /// Waiting for the configured delay before issuing the request.
        Delaying {
            #[pin]
            sleep: Sleep,
            service: Option<S>,
            request: Option<Req>,
        },
        /// Request has been dispatched to inner service.
        Calling {
            #[pin]
            response: S::Future,
        },
    }
}

impl<S, Req> ResponseFuture<S, Req>
where
    S: Service<Req>,
{
    #[inline]
    pub(crate) fn new(service: S, request: Req, sleep: Sleep) -> Self {
        Self::Delaying {
            sleep,
            service: Some(service),
            request: Some(request),
        }
    }
}

impl<S, Req> Future for ResponseFuture<S, Req>
where
    S: Service<Req>,
    S::Error: Into<BoxError>,
{
    type Output = Result<S::Response, BoxError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.as_mut().project();

        loop {
            match this {
                ResponseFutureProj::Delaying {
                    sleep,
                    service,
                    request,
                } => {
                    ready!(sleep.poll(cx));
                    let mut inner = service
                        .take()
                        .expect("delay future polled after service taken");
                    let req = request
                        .take()
                        .expect("delay future polled after request taken");
                    let response = inner.call(req);
                    self.set(Self::Calling { response });
                    this = self.as_mut().project();
                }
                ResponseFutureProj::Calling { response } => {
                    return response.poll(cx).map_err(Into::into);
                }
            }
        }
    }
}
