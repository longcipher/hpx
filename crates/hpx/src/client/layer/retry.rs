//! Middleware for retrying requests.

mod classify;
mod scope;

use std::{error::Error as StdError, future::Ready, sync::Arc, time::Duration};

use http::{Request, Response};
use tower::retry::{
    Policy,
    budget::{Budget, TpsBudget},
};

pub(crate) use self::{
    classify::{Action, Classifier, ClassifyFn, ReqRep},
    scope::{ScopeFn, Scoped},
};
use crate::{Body, ClientResponseBody, error::BoxError, retry};

/// A retry policy for HTTP requests.
#[derive(Clone)]
pub(crate) struct RetryPolicy {
    budget: Option<Arc<TpsBudget>>,
    classifier: Classifier,
    max_retries_per_request: u32,
    retry_cnt: u32,
    scope: Scoped,
}

impl RetryPolicy {
    /// Create a new `RetryPolicy`.
    #[inline]
    pub(crate) fn new(policy: retry::Policy) -> Self {
        Self {
            budget: policy
                .budget
                .map(|budget| Arc::new(TpsBudget::new(Duration::from_secs(10), 10, budget))),
            classifier: policy.classifier,
            max_retries_per_request: policy.max_retries_per_request,
            retry_cnt: 0,
            scope: policy.scope,
        }
    }
}

type Req = Request<Body>;

type Res = Response<ClientResponseBody>;

pub(crate) fn clone_http_request(req: &Req) -> Option<Req> {
    let body = req.body().try_clone()?;
    let mut new = http::Request::new(body);
    *new.method_mut() = req.method().clone();
    *new.uri_mut() = req.uri().clone();
    *new.version_mut() = req.version();
    *new.headers_mut() = req.headers().clone();
    *new.extensions_mut() = req.extensions().clone();
    Some(new)
}

impl Policy<Req, Res, BoxError> for RetryPolicy {
    type Future = Ready<()>;

    fn retry(&mut self, req: &mut Req, result: &mut Result<Res, BoxError>) -> Option<Self::Future> {
        match self.classifier.classify(req, result) {
            Action::Success => {
                trace!(
                    "Request successful, no retry needed: {} {}",
                    req.method(),
                    req.uri()
                );

                if let Some(ref budget) = self.budget {
                    budget.deposit();
                    trace!("Token deposited back to retry budget");
                }
                None
            }
            Action::Retryable => {
                trace!(
                    "Retrying request ({} attempts so far): {} {} - {}",
                    self.retry_cnt,
                    req.method(),
                    req.uri(),
                    match result {
                        Ok(res) => format!("HTTP {}", res.status()),
                        Err(e) => format!("Error: {}", e),
                    }
                );

                Some(std::future::ready(()))
            }
        }
    }

    fn clone_request(&mut self, req: &Req) -> Option<Req> {
        if self.retry_cnt > 0 && !self.scope.applies_to(req) {
            trace!("not in scope, not retrying");
            return None;
        }

        if self.retry_cnt >= self.max_retries_per_request {
            trace!("max_retries_per_request hit");
            return None;
        }

        // Withdraw budget token only when we're about to actually retry.
        if !self.budget.as_ref().is_none_or(|b| b.withdraw()) {
            debug!(
                "Request is retryable but retry budget exhausted: {} {}",
                req.method(),
                req.uri()
            );
            return None;
        }

        self.retry_cnt += 1;
        if let Some(cloned) = clone_http_request(req) {
            Some(cloned)
        } else {
            // Clone failed — deposit the token back.
            if let Some(ref budget) = self.budget {
                budget.deposit();
            }
            None
        }
    }
}

/// Determines whether the given error/method pair is safe to retry.
///
/// The default classifier retries:
/// - HTTP/2 GOAWAY (graceful shutdown) and REFUSED_STREAM resets (RFC 9113);
/// - connection-establishment failures — the request was never delivered to
///   the server, so retrying is safe for every method;
/// - transport timeouts and connection resets, but only for idempotent
///   methods (GET/HEAD/PUT/DELETE/OPTIONS/TRACE) — after a timeout/reset the
///   server may or may not have processed the request, so retrying a
///   non-idempotent method could cause duplicate side effects (e.g. a
///   re-placed order).
fn is_retryable_error(err: &(dyn StdError + 'static), method: &http::Method) -> bool {
    let mut source = Some(err);

    while let Some(err) = source {
        #[cfg(feature = "http2")]
        if let Some(h2_err) = err.downcast_ref::<http2::Error>() {
            // They sent us a graceful shutdown, try with a new connection!
            if h2_err.is_go_away()
                && h2_err.is_remote()
                && h2_err.reason() == Some(http2::Reason::NO_ERROR)
            {
                return true;
            }

            // REFUSED_STREAM was sent from the server, which is safe to retry.
            // https://www.rfc-editor.org/rfc/rfc9113.html#section-8.7-3.2
            if h2_err.is_reset()
                && h2_err.is_remote()
                && h2_err.reason() == Some(http2::Reason::REFUSED_STREAM)
            {
                return true;
            }
        }

        // Connection-establishment failures: the request was never delivered
        // to the server, so retrying is safe regardless of the method.
        if err
            .downcast_ref::<crate::Error>()
            .is_some_and(|e| e.is_connect())
            || err
                .downcast_ref::<crate::client::Error>()
                .is_some_and(|e| e.is_connect())
            || matches!(
                err.downcast_ref::<std::io::Error>().map(|e| e.kind()),
                Some(std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::ConnectionAborted)
            )
        {
            return true;
        }

        // Timeout / connection reset on a (likely reused) connection. The
        // server may or may not have processed the request — only retry
        // idempotent methods to avoid duplicate side effects.
        if method_is_idempotent(method) && (is_timeout_error(err) || is_connection_reset_error(err))
        {
            return true;
        }

        source = err.source();
    }

    false
}

/// True for methods that a server may execute multiple times without
/// additional side effects.
fn method_is_idempotent(method: &http::Method) -> bool {
    matches!(
        *method,
        http::Method::GET
            | http::Method::HEAD
            | http::Method::PUT
            | http::Method::DELETE
            | http::Method::OPTIONS
            | http::Method::TRACE
    )
}

/// True if the error is a timeout, checking the current layer of the source
/// chain. Mirrors [`crate::Error::is_timeout`] so the retry classifier
/// recognizes the same conditions as the public error API. This includes the
/// `Error::request(TimedOut)` shape produced by the outer request `Timeout`
/// layer, so a timed-out attempt can be retried.
fn is_timeout_error(err: &(dyn StdError + 'static)) -> bool {
    err.is::<crate::error::TimedOut>()
        || err
            .downcast_ref::<crate::Error>()
            .is_some_and(|e| e.is_timeout())
        || err
            .downcast_ref::<crate::client::CoreError>()
            .is_some_and(|e| e.is_timeout())
        || err
            .downcast_ref::<std::io::Error>()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::TimedOut)
}

/// True if the error is a connection reset (peer closed the socket abruptly).
fn is_connection_reset_error(err: &(dyn StdError + 'static)) -> bool {
    err.downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.kind() == std::io::ErrorKind::ConnectionReset)
}

#[cfg(test)]
mod tests {
    use std::error::Error as StdError;

    use http::Request;
    use http_body_util::BodyExt;
    use tower::retry::Policy as _;

    use super::{RetryPolicy, is_retryable_error};
    use crate::{Body, retry};

    fn make_retry_policy(max_retries: u32) -> RetryPolicy {
        let policy = retry::Policy {
            budget: Some(0.2),
            classifier: crate::client::layer::retry::Classifier::Never,
            max_retries_per_request: max_retries,
            scope: crate::client::layer::retry::Scoped::Unscoped,
        };
        RetryPolicy::new(policy)
    }

    #[test]
    fn budget_preserved_when_clone_fails() {
        let mut policy = make_retry_policy(2);
        // Create a request with a non-clonable body (streaming/boxed).
        let body: Body = http_body_util::Empty::new()
            .map_err(|e| -> crate::error::BoxError { e.into() })
            .boxed()
            .into();
        let req = Request::new(body);

        // clone_request should return None because the body is not clonable.
        let result = policy.clone_request(&req);
        assert!(result.is_none(), "non-clonable body should return None");

        // Verify the budget still has tokens by cloning a clonable request.
        let clonable_req = Request::new(Body::from("hello"));
        let result2 = policy.clone_request(&clonable_req);
        assert!(
            result2.is_some(),
            "budget should still have tokens after failed clone"
        );
    }

    // -----------------------------------------------------------------------
    // Default classifier: network-level retry classification
    // -----------------------------------------------------------------------

    fn io_err(kind: std::io::ErrorKind) -> Box<dyn StdError + Send + Sync> {
        Box::new(std::io::Error::new(kind, "synthetic io error"))
    }

    /// Coerce a concrete error into a trait object for `is_retryable_error`.
    fn to_dyn<'a>(err: &'a (dyn StdError + 'static)) -> &'a (dyn StdError + 'static) {
        err
    }

    #[test]
    fn connect_failure_retryable_for_all_methods() {
        let err = crate::Error::request(io_err(std::io::ErrorKind::ConnectionRefused));
        // Connection establishment failed: the request was never delivered,
        // so retrying is safe even for state-changing methods.
        assert!(is_retryable_error(to_dyn(&err), &http::Method::GET));
        assert!(is_retryable_error(to_dyn(&err), &http::Method::POST));

        let aborted = crate::Error::request(io_err(std::io::ErrorKind::ConnectionAborted));
        assert!(is_retryable_error(to_dyn(&aborted), &http::Method::GET));
        assert!(is_retryable_error(to_dyn(&aborted), &http::Method::POST));
    }

    #[test]
    fn timeout_retryable_only_for_idempotent_methods() {
        let err = crate::Error::request(io_err(std::io::ErrorKind::TimedOut));
        assert!(is_retryable_error(to_dyn(&err), &http::Method::GET));
        assert!(is_retryable_error(to_dyn(&err), &http::Method::DELETE));
        // A timeout leaves ambiguous whether the server processed the request;
        // never retry non-idempotent methods (e.g. order placement).
        assert!(!is_retryable_error(to_dyn(&err), &http::Method::POST));
        assert!(!is_retryable_error(to_dyn(&err), &http::Method::PATCH));
    }

    #[test]
    fn connection_reset_retryable_only_for_idempotent_methods() {
        let err = crate::Error::request(io_err(std::io::ErrorKind::ConnectionReset));
        assert!(is_retryable_error(to_dyn(&err), &http::Method::GET));
        assert!(is_retryable_error(to_dyn(&err), &http::Method::PUT));
        assert!(!is_retryable_error(to_dyn(&err), &http::Method::POST));
    }

    #[test]
    fn timeout_detected_deep_in_source_chain() {
        // io::Error(ConnectionReset) -> crate::Error -> io::Error(other) wrap.
        let inner = crate::Error::request(io_err(std::io::ErrorKind::TimedOut));
        let wrapped = std::io::Error::other(inner);
        assert!(is_retryable_error(
            &wrapped as &(dyn StdError + 'static),
            &http::Method::GET,
        ));
        assert!(!is_retryable_error(
            &wrapped as &(dyn StdError + 'static),
            &http::Method::POST,
        ));
    }

    #[test]
    fn public_error_request_timedout_retryable_for_idempotent_methods() {
        // The public `Error::request(TimedOut)` shape produced by the outer
        // request `Timeout` layer must be recognized by the classifier so a
        // timed-out attempt can be retried.
        let err = crate::Error::request(crate::error::TimedOut);
        assert!(is_retryable_error(to_dyn(&err), &http::Method::GET));
        assert!(is_retryable_error(to_dyn(&err), &http::Method::DELETE));
        assert!(!is_retryable_error(to_dyn(&err), &http::Method::POST));
        assert!(!is_retryable_error(to_dyn(&err), &http::Method::PATCH));
    }

    #[test]
    fn unrelated_errors_are_not_retryable() {
        let err = crate::Error::request(io_err(std::io::ErrorKind::Other));
        assert!(!is_retryable_error(to_dyn(&err), &http::Method::GET));
        assert!(!is_retryable_error(to_dyn(&err), &http::Method::POST));
    }
}
