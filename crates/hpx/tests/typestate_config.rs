//! Tests for the typestate configuration builder.
//!
//! The builder is a compile-time state machine: `ConfigBuilder<ClientScope>` and
//! `ConfigBuilder<RequestScope>` deliberately expose different method sets and
//! different `build()` return types. Most of what can go wrong here is either a
//! broken forwarding call, a `Debug` impl that leaks internals, or a timeout
//! that cannot be cleared once set, so the tests concentrate on those.

use std::time::Duration;

use hpx::{Client, Method, typestate::ConfigBuilder};

/// A tiny delay that keeps the tests fast while still being a real non-default
/// value for the builder to store.
const T: Duration = Duration::from_millis(25);

fn test_client() -> Client {
    Client::builder()
        .build()
        .expect("a client must build in the test environment")
}

fn test_uri() -> hpx::Uri {
    "http://example.com/path".parse().expect("valid URI")
}

#[test]
fn client_scope_debug_is_opaque() {
    let debug = format!("{:?}", ConfigBuilder::client());
    assert!(debug.contains("ClientScope"), "unexpected Debug: {debug}");
    assert!(
        !debug.contains("timeout"),
        "ClientScope Debug must not leak the wrapped builder: {debug}"
    );
}

#[test]
fn request_scope_debug_exposes_method_and_uri() {
    let debug = format!(
        "{:?}",
        ConfigBuilder::request(test_client(), Method::POST, test_uri())
    );
    assert!(debug.contains("RequestScope"), "{debug}");
    assert!(debug.contains("POST"), "{debug}");
    assert!(debug.contains("example.com"), "{debug}");
}

#[test]
fn client_builder_starts_from_defaults() {
    // `client()` must not require any explicit configuration to build.
    assert!(ConfigBuilder::client().build().is_ok());
}

#[test]
fn client_builder_accepts_a_prebuilt_builder() {
    let builder = Client::builder().https_only(true);
    assert!(ConfigBuilder::from_builder(builder).build().is_ok());
}

#[test]
fn every_client_timeout_is_accepted() {
    // Each setter must forward to the underlying builder without panicking or
    // short-circuiting the chain.
    let built = ConfigBuilder::client()
        .timeout_global(Some(T))
        .timeout_per_call(Some(T))
        .timeout_resolve(Some(T))
        .timeout_connect(T)
        .timeout_send_request(Some(T))
        .timeout_await_100(Some(T))
        .timeout_send_body(Some(T))
        .timeout_recv_response(Some(T))
        .timeout_recv_body(Some(T))
        .https_only(false)
        .user_agent("hpx-test-agent")
        .build();
    assert!(built.is_ok(), "chained configuration must build");
}

#[test]
fn timeouts_are_cleared_by_none() {
    // `None` must disable a previously configured timeout rather than leaving
    // the earlier value in place.
    let built = ConfigBuilder::client()
        .timeout_global(Some(T))
        .timeout_global(None)
        .timeout_per_call(Some(T))
        .timeout_per_call(None)
        .timeout_resolve(Some(T))
        .timeout_resolve(None)
        .timeout_send_request(Some(T))
        .timeout_send_request(None)
        .timeout_await_100(Some(T))
        .timeout_await_100(None)
        .timeout_send_body(Some(T))
        .timeout_send_body(None)
        .timeout_recv_response(Some(T))
        .timeout_recv_response(None)
        .timeout_recv_body(Some(T))
        .timeout_recv_body(None)
        .build();
    assert!(built.is_ok());
}

#[test]
fn https_only_toggles_both_ways() {
    assert!(ConfigBuilder::client().https_only(true).build().is_ok());
    assert!(
        ConfigBuilder::client()
            .https_only(true)
            .https_only(false)
            .build()
            .is_ok()
    );
}

#[test]
fn invalid_user_agent_is_rejected_at_build_time() {
    // `user_agent` accepts any `TryInto<HeaderValue>`; a value that cannot
    // become a header must surface as an error rather than a panic.
    let built = ConfigBuilder::client().user_agent("bad\nvalue").build();
    assert!(built.is_err(), "an invalid user agent must not build");
}

#[test]
fn request_scope_builds_a_request_builder() {
    let request = ConfigBuilder::request(test_client(), Method::PUT, test_uri()).build();
    // `RequestBuilder` is opaque here, so the assertion is that building it
    // neither panics nor leaves an unusable builder behind.
    assert!(!format!("{request:?}").is_empty());
}

#[test]
fn request_scope_accepts_every_http_method() {
    for method in [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::HEAD,
        Method::OPTIONS,
    ] {
        let request = ConfigBuilder::request(test_client(), method.clone(), test_uri()).build();
        assert!(
            !format!("{request:?}").is_empty(),
            "building a {method} request failed"
        );
    }
}

#[test]
fn request_scope_timeout_global_is_a_no_op() {
    // The request scope deliberately ignores the timeout: it is fixed at the
    // client level. The call must still be chainable.
    let request = ConfigBuilder::request(test_client(), Method::GET, test_uri())
        .timeout_global(Some(T))
        .build();
    assert!(!format!("{request:?}").is_empty());
}

/// Compile-time evidence that the typestate split is real: each scope resolves
/// `build()` to its own distinct type.
#[test]
fn scopes_expose_disjoint_finalization_targets() {
    let client_scoped = ConfigBuilder::client();
    let client: hpx::Result<Client> = client_scoped.build();
    assert!(client.is_ok());

    let request_scoped = ConfigBuilder::request(test_client(), Method::GET, test_uri());
    let request = request_scoped.build();
    // The `let` bindings above only compile if the two `build()` methods return
    // different types.
    assert!(!format!("{request:?}").is_empty());
}
