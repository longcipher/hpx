//! Integration tests for the hooks system.

mod support;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use hpx::{
    Client,
    hooks::{AfterResponseHook, BeforeRequestHook, Hooks, LoggingHook, OnErrorHook},
};
use http::{HeaderMap, StatusCode};
use support::server;

/// A hook that counts the number of requests.
struct RequestCounterHook {
    count: Arc<AtomicUsize>,
}

impl BeforeRequestHook for RequestCounterHook {
    fn on_request(&self, _request: &mut http::Request<hpx::Body>) -> Result<(), hpx::Error> {
        self.count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// A hook that counts the number of responses.
struct ResponseCounterHook {
    count: Arc<AtomicUsize>,
}

impl AfterResponseHook for ResponseCounterHook {
    fn on_response(&self, _status: StatusCode, _headers: &HeaderMap) -> Result<(), hpx::Error> {
        self.count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// A hook that counts errors.
#[allow(dead_code)]
struct ErrorCounterHook {
    count: Arc<AtomicUsize>,
}

impl OnErrorHook for ErrorCounterHook {
    fn on_error(&self, _error: &hpx::Error) {
        self.count.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn test_before_request_hook_executes() {
    let _ = pretty_env_logger::try_init();

    let server = server::http(move |_req| async move { http::Response::default() });

    let request_count = Arc::new(AtomicUsize::new(0));
    let hook = Arc::new(RequestCounterHook {
        count: request_count.clone(),
    });

    let hooks = Hooks::builder().before_request(hook).build();

    let client = Client::builder().hooks(hooks).build().unwrap();

    let url = format!("http://{}", server.addr());
    let resp = client.get(&url).send().await.unwrap();

    assert_eq!(resp.status(), 200);
    assert_eq!(request_count.load(Ordering::SeqCst), 1);

    // Second request should increment the counter
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(request_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_after_response_hook_executes() {
    let _ = pretty_env_logger::try_init();

    let server = server::http(move |_req| async move { http::Response::default() });

    let response_count = Arc::new(AtomicUsize::new(0));
    let hook = Arc::new(ResponseCounterHook {
        count: response_count.clone(),
    });

    let hooks = Hooks::builder().after_response(hook).build();

    let client = Client::builder().hooks(hooks).build().unwrap();

    let url = format!("http://{}", server.addr());
    let resp = client.get(&url).send().await.unwrap();

    assert_eq!(resp.status(), 200);
    assert_eq!(response_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_on_request_closure() {
    let _ = pretty_env_logger::try_init();

    let server = server::http(move |req| async move {
        // Check that our custom header was added
        if req.headers().contains_key("x-custom-header") {
            http::Response::default()
        } else {
            http::Response::builder()
                .status(http::StatusCode::BAD_REQUEST)
                .body(Default::default())
                .unwrap()
        }
    });

    let client = Client::builder()
        .on_request(|req| {
            req.headers_mut().insert(
                http::header::HeaderName::from_static("x-custom-header"),
                http::HeaderValue::from_static("test-value"),
            );
            Ok(())
        })
        .build()
        .unwrap();

    let url = format!("http://{}", server.addr());
    let resp = client.get(&url).send().await.unwrap();

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_on_response_closure() {
    let _ = pretty_env_logger::try_init();

    let response_count = Arc::new(AtomicUsize::new(0));
    let count_clone = response_count.clone();

    let server = server::http(move |_req| async move { http::Response::default() });

    let client = Client::builder()
        .on_response(move |status, _headers| {
            assert!(status.is_success());
            count_clone.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .build()
        .unwrap();

    let url = format!("http://{}", server.addr());
    let resp = client.get(&url).send().await.unwrap();

    assert_eq!(resp.status(), 200);
    assert_eq!(response_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_logging_hook() {
    let _ = pretty_env_logger::try_init();

    let server = server::http(move |_req| async move { http::Response::default() });

    let hooks = Hooks::builder()
        .before_request(Arc::new(LoggingHook::new().with_headers()))
        .after_response(Arc::new(LoggingHook::new().with_headers()))
        .build();

    let client = Client::builder().hooks(hooks).build().unwrap();

    let url = format!("http://{}", server.addr());
    let resp = client.get(&url).send().await.unwrap();

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_multiple_hooks() {
    let _ = pretty_env_logger::try_init();

    let server = server::http(move |_req| async move { http::Response::default() });

    let request_count = Arc::new(AtomicUsize::new(0));
    let response_count = Arc::new(AtomicUsize::new(0));

    let req_hook = Arc::new(RequestCounterHook {
        count: request_count.clone(),
    });
    let resp_hook = Arc::new(ResponseCounterHook {
        count: response_count.clone(),
    });

    let hooks = Hooks::builder()
        .before_request(req_hook.clone())
        .before_request(req_hook) // Add twice to verify multiple hooks
        .after_response(resp_hook)
        .build();

    let client = Client::builder().hooks(hooks).build().unwrap();

    let url = format!("http://{}", server.addr());
    let resp = client.get(&url).send().await.unwrap();

    assert_eq!(resp.status(), 200);
    // Two before_request hooks should have incremented twice
    assert_eq!(request_count.load(Ordering::SeqCst), 2);
    assert_eq!(response_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_header_injection_hook() {
    let _ = pretty_env_logger::try_init();

    let server = server::http(move |req| async move {
        // Check that our custom header was added
        let auth = req
            .headers()
            .get(http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        if auth == Some("Bearer test-token") {
            http::Response::default()
        } else {
            http::Response::builder()
                .status(http::StatusCode::UNAUTHORIZED)
                .body(Default::default())
                .unwrap()
        }
    });

    use hpx::hooks::HeaderInjectionHook;

    let hook = Arc::new(HeaderInjectionHook::single(
        http::header::AUTHORIZATION,
        http::HeaderValue::from_static("Bearer test-token"),
    ));

    let hooks = Hooks::builder().before_request(hook).build();

    let client = Client::builder().hooks(hooks).build().unwrap();

    let url = format!("http://{}", server.addr());
    let resp = client.get(&url).send().await.unwrap();

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_request_id_hook() {
    let _ = pretty_env_logger::try_init();

    let server = server::http(move |req| async move {
        // Check that request ID was added
        if req.headers().contains_key("x-request-id") {
            http::Response::default()
        } else {
            http::Response::builder()
                .status(http::StatusCode::BAD_REQUEST)
                .body(Default::default())
                .unwrap()
        }
    });

    use hpx::hooks::RequestIdHook;

    let hooks = Hooks::builder()
        .before_request(Arc::new(RequestIdHook::new()))
        .build();

    let client = Client::builder().hooks(hooks).build().unwrap();

    let url = format!("http://{}", server.addr());
    let resp = client.get(&url).send().await.unwrap();

    assert_eq!(resp.status(), 200);
}
