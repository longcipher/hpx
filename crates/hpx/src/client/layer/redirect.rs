//! Middleware for following redirections.

mod future;
mod policy;

use std::{
    mem,
    task::{Context, Poll},
};

use futures_util::future::Either;
use http::{Request, Response};
use http_body::Body;
use tower::{Layer, Service};

use self::future::ResponseFuture;
pub(crate) use self::policy::{Action, Attempt, Policy};
use crate::error::BoxError;

enum BodyRepr<B> {
    Some(B),
    Empty,
    None,
}

impl<B> BodyRepr<B>
where
    B: Body + Default,
{
    fn take(&mut self) -> Option<B> {
        match mem::replace(self, Self::None) {
            Self::Some(body) => Some(body),
            Self::Empty => {
                *self = Self::Empty;
                Some(B::default())
            }
            Self::None => None,
        }
    }

    fn try_clone_from<P, E>(&mut self, body: &B, policy: &P)
    where
        P: Policy<B, E>,
    {
        match self {
            Self::Some(_) | Self::Empty => {}
            Self::None => {
                if body.size_hint().exact() == Some(0) {
                    *self = Self::Some(B::default());
                } else if let Some(cloned) = policy.clone_body(body) {
                    *self = Self::Some(cloned);
                }
            }
        }
    }
}

/// [`Layer`] for retrying requests with a [`Service`] to follow redirection responses.
#[derive(Clone, Copy, Default)]
pub(crate) struct FollowRedirectLayer<P> {
    policy: P,
}

impl<P> FollowRedirectLayer<P> {
    /// Create a new [`FollowRedirectLayer`] with the given redirection [`Policy`].
    #[inline]
    pub(crate) const fn with_policy(policy: P) -> Self {
        Self { policy }
    }
}

impl<S, P> Layer<S> for FollowRedirectLayer<P>
where
    S: Clone,
    P: Clone,
{
    type Service = FollowRedirect<S, P>;

    #[inline]
    fn layer(&self, inner: S) -> Self::Service {
        FollowRedirect::with_policy(inner, self.policy.clone())
    }
}

/// Middleware that retries requests with a [`Service`] to follow redirection responses.
#[derive(Clone, Copy)]
pub(crate) struct FollowRedirect<S, P> {
    inner: S,
    policy: P,
}

impl<S, P> FollowRedirect<S, P>
where
    P: Clone,
{
    /// Create a new [`FollowRedirect`] with the given redirection [`Policy`].
    #[inline]
    pub(crate) const fn with_policy(inner: S, policy: P) -> Self {
        Self { inner, policy }
    }
}

impl<ReqBody, ResBody, S, P> Service<Request<ReqBody>> for FollowRedirect<S, P>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone,
    S::Error: From<BoxError>,
    P: Policy<ReqBody, S::Error> + Clone,
    ReqBody: Body + Default,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = ResponseFuture<S, ReqBody, P>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        if self.policy.follow_redirects(&mut req) {
            let service = self.inner.clone();
            let mut service = mem::replace(&mut self.inner, service);
            let mut policy = self.policy.clone();

            let mut body_repr = BodyRepr::None;
            body_repr.try_clone_from(req.body(), &policy);
            policy.on_request(&mut req);

            let (parts, body) = req.into_parts();
            let req = Request::from_parts(parts.clone(), body);
            ResponseFuture::Redirect {
                future: Either::Left(service.call(req)),
                pending_future: None,
                service,
                policy,
                parts,
                body_repr,
            }
        } else {
            ResponseFuture::Direct {
                future: self.inner.call(req),
            }
        }
    }
}
