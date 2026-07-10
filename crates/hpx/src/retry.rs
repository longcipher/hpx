//! Retry requests
//!
//! A `Client` has the ability to retry requests, by sending additional copies
//! to the server if a response is considered retryable.
//!
//! The [`Policy`] makes it easier to configure what requests to retry, along
//! with including best practices by default, such as a retry budget.
//!
//! # Defaults
//!
//! The default retry behavior of a `Client` is to only retry requests where an
//! error or low-level protocol NACK is encountered that is known to be safe to
//! retry. Note however that providing a specific retry policy will override
//! the default, and you will need to explicitly include that behavior.
//!
//! All policies default to including a retry budget that permits 20% extra
//! requests to be sent.
//!
//! # Scoped
//!
//! A client's retry policy is scoped. That means that the policy doesn't
//! apply to all requests, but only those within a user-defined scope.
//!
//! Since all policies include a budget by default, it doesn't make sense to
//! apply it on _all_ requests. Rather, the retry history applied by a budget
//! should likely only be applied to the same host.
//!
//! # Classifiers
//!
//! A retry policy needs to be configured with a classifier that determines
//! if a request should be retried. Knowledge of the destination server's
//! behavior is required to make a safe classifier. **Requests should not be
//! retried** if the server cannot safely handle the same request twice, or if
//! it causes side effects.
//!
//! Some common properties to check include if the request method is
//! idempotent, or if the response status code indicates a transient error.

use std::sync::Arc;

use http::Request;

use crate::{
    Body,
    client::layer::retry::{Action, Classifier, ClassifyFn, ReqRep, ScopeFn, Scoped},
};

/// A retry policy.
#[derive(Clone)]
pub struct Policy {
    pub(crate) budget: Option<f32>,
    pub(crate) classifier: Classifier,
    pub(crate) max_retries_per_request: u32,
    pub(crate) scope: Scoped,
}

impl Policy {
    /// Create a retry policy that will never retry any request.
    ///
    /// This is useful for disabling the `Client`s default behavior of retrying
    /// protocol nacks.
    #[inline]
    pub fn never() -> Policy {
        Self::scoped(|_| false).no_budget()
    }

    /// Create a retry policy scoped to requests for a specific host.
    ///
    /// This is a convenience method that creates a retry policy which only applies
    /// to requests targeting the specified host. Requests to other hosts will not
    /// be retried under this policy.
    ///
    /// # Arguments
    /// * `host` - The hostname to match against request URIs (e.g., "api.example.com")
    ///
    /// # Example
    /// ```rust
    /// use hpx::retry::Policy;
    ///
    /// // Only retry requests to rust-lang.org
    /// let policy = Policy::for_host("rust-lang.org");
    /// ```
    #[inline]
    pub fn for_host<S>(host: S) -> Policy
    where
        S: for<'a> PartialEq<&'a str> + Send + Sync + 'static,
    {
        Self::scoped(move |req| {
            req.uri()
                .host()
                .is_some_and(|request_host| host == request_host)
        })
    }

    /// Create a scoped retry policy.
    ///
    /// For a more convenient constructor, see [`Policy::for_host()`].
    #[inline]
    fn scoped<F>(func: F) -> Policy
    where
        F: Fn(&Request<Body>) -> bool + Send + Sync + 'static,
    {
        Self {
            budget: Some(0.2),
            classifier: Classifier::Never,
            max_retries_per_request: 2,
            scope: Scoped::Dyn(Arc::new(ScopeFn(func))),
        }
    }

    /// Set no retry budget.
    ///
    /// Sets that no budget will be enforced. This could also be considered
    /// to be an infinite budget.
    ///
    /// This is NOT recommended. Disabling the budget can make your system more
    /// susceptible to retry storms.
    #[inline]
    pub fn no_budget(mut self) -> Self {
        self.budget = None;
        self
    }

    /// Sets the max extra load the budget will allow.
    ///
    /// Think of the amount of requests your client generates, and how much
    /// load that puts on the server. This option configures as a percentage
    /// how much extra load is allowed via retries.
    ///
    /// For example, if you send 1,000 requests per second, setting a maximum
    /// extra load value of `0.3` would allow 300 more requests per second
    /// in retries. A value of `2.5` would allow 2,500 more requests.
    ///
    /// # Panics
    ///
    /// The `extra_percent` value must be within reasonable values for a
    /// percentage. This method will panic if it is less than `0.0`, or greater
    /// than `1000.0`.
    #[inline]
    pub fn max_extra_load(mut self, extra_percent: f32) -> Self {
        assert!(extra_percent >= 0.0);
        assert!(extra_percent <= 1000.0);
        self.budget = Some(extra_percent);
        self
    }

    /// Set the max retries allowed per request.
    ///
    /// For each logical (initial) request, only retry up to `max` times.
    ///
    /// This value is used in combination with a token budget that is applied
    /// to all requests. Even if the budget would allow more requests, this
    /// limit will prevent. Likewise, the budget may prevent retrying up to
    /// `max` times. This setting prevents a single request from consuming
    /// the entire budget.
    ///
    /// Default is currently 2 retries.
    #[inline]
    pub fn max_retries_per_request(mut self, max: u32) -> Self {
        self.max_retries_per_request = max;
        self
    }

    /// Provide a classifier to determine if a request should be retried.
    ///
    /// # Example
    ///
    /// ```rust
    /// # fn with_policy(policy: hpx::retry::Policy) -> hpx::retry::Policy {
    /// policy.classify_fn(|req_rep| match (req_rep.method(), req_rep.status()) {
    ///     (&http::Method::GET, Some(http::StatusCode::SERVICE_UNAVAILABLE)) => req_rep.retryable(),
    ///     _ => req_rep.success(),
    /// })
    /// # }
    /// ```
    #[inline]
    pub fn classify_fn<F>(mut self, func: F) -> Self
    where
        F: Fn(ReqRep<'_>) -> Action + Send + Sync + 'static,
    {
        self.classifier = Classifier::Dyn(Arc::new(ClassifyFn(func)));
        self
    }
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            budget: None,
            classifier: Classifier::ProtocolNacks,
            max_retries_per_request: 2,
            scope: Scoped::Unscoped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Policy;
    use crate::client::layer::retry::{Classifier, Scoped};

    #[test]
    fn default_policy_is_protocol_nacks_unscoped() {
        let policy = Policy::default();
        assert!(
            matches!(policy.classifier, Classifier::ProtocolNacks),
            "default classifier should be ProtocolNacks"
        );
        assert!(
            matches!(policy.scope, Scoped::Unscoped),
            "default scope should be Unscoped"
        );
        assert!(policy.budget.is_none(), "default budget should be None");
        assert_eq!(policy.max_retries_per_request, 2);
    }

    #[test]
    fn never_policy_has_never_classifier_no_budget() {
        let policy = Policy::never();
        assert!(
            matches!(policy.classifier, Classifier::Never),
            "never() classifier should be Never"
        );
        assert!(policy.budget.is_none(), "never() budget should be None");
    }

    #[test]
    fn for_host_creates_scoped_policy() {
        let policy = Policy::for_host("example.com");
        assert!(
            matches!(policy.scope, Scoped::Dyn(_)),
            "for_host should create a scoped policy"
        );
        assert!(
            matches!(policy.classifier, Classifier::Never),
            "for_host classifier should be Never by default"
        );
    }

    #[test]
    fn classify_fn_sets_custom_classifier() {
        let policy = Policy::never().classify_fn(|req_rep| {
            if req_rep.method() == &http::Method::POST {
                req_rep.retryable()
            } else {
                req_rep.success()
            }
        });

        assert!(
            matches!(policy.classifier, Classifier::Dyn(_)),
            "classify_fn should set a Dyn classifier"
        );
    }

    #[test]
    fn max_retries_per_request_configuration() {
        let policy = Policy::never().max_retries_per_request(5);
        assert_eq!(policy.max_retries_per_request, 5);
    }

    #[test]
    fn budget_configuration() {
        let base = Policy::for_host("example.com");

        let no_budget = base.clone().no_budget();
        assert!(no_budget.budget.is_none());

        let with_budget = base.max_extra_load(0.5);
        assert_eq!(with_budget.budget, Some(0.5));
    }

    #[test]
    fn combined_configuration() {
        let policy = Policy::for_host("api.example.com")
            .classify_fn(|req_rep| match req_rep.status() {
                Some(s) if s.is_server_error() => req_rep.retryable(),
                _ => req_rep.success(),
            })
            .max_retries_per_request(3)
            .max_extra_load(0.3);

        assert!(matches!(policy.classifier, Classifier::Dyn(_)));
        assert!(matches!(policy.scope, Scoped::Dyn(_)));
        assert_eq!(policy.max_retries_per_request, 3);
        assert_eq!(policy.budget, Some(0.3));
    }

    // -----------------------------------------------------------------------
    // Proptest: retry budget invariants
    // -----------------------------------------------------------------------
    mod proptests {
        use proptest::prelude::*;

        use super::Policy;

        proptest! {
            /// Budget value set via max_extra_load must round-trip correctly
            /// for all values in the valid range [0.0, 1000.0].
            #[test]
            fn max_extra_load_round_trip(budget in 0.0f32..=1000.0) {
                let policy = Policy::for_host("example.com").max_extra_load(budget);
                prop_assert_eq!(policy.budget, Some(budget));
            }

            /// no_budget always clears a previously set budget.
            #[test]
            fn no_budget_clears_any_budget(budget in 0.0f32..=1000.0) {
                let policy = Policy::for_host("example.com")
                    .max_extra_load(budget)
                    .no_budget();
                prop_assert!(policy.budget.is_none());
            }
        }

        #[test]
        #[should_panic(expected = "assertion failed")]
        fn max_extra_load_panics_on_negative() {
            Policy::for_host("example.com").max_extra_load(-0.1);
        }

        #[test]
        #[should_panic(expected = "assertion failed")]
        fn max_extra_load_panics_on_over_limit() {
            Policy::for_host("example.com").max_extra_load(1000.1);
        }

        #[test]
        fn max_retries_never_negative() {
            // max_retries_per_request takes u32, so it's compile-time safe.
            // Verify the default is reasonable.
            let policy = Policy::default();
            assert!(policy.max_retries_per_request > 0);

            // Verify a high value is preserved.
            let policy = Policy::never().max_retries_per_request(100);
            assert_eq!(policy.max_retries_per_request, 100);
        }
    }
}
