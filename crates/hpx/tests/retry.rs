mod support;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bytes::Bytes;
use hpx::{Body, Client};
use http_body_util::Full;
use support::server;

#[tokio::test]
async fn retries_apply_in_scope() {
    let _ = pretty_env_logger::try_init();

    let cnt = Arc::new(AtomicUsize::new(0));
    let server = server::http(move |_req| {
        let cnt = cnt.clone();
        async move {
            if cnt.fetch_add(1, Ordering::Relaxed) == 0 {
                // first req is bad
                http::Response::builder()
                    .status(http::StatusCode::SERVICE_UNAVAILABLE)
                    .body(Default::default())
                    .unwrap()
            } else {
                http::Response::default()
            }
        }
    });

    let scope = server.addr().ip().to_string();
    let policy = hpx::retry::Policy::for_host(scope).classify_fn(|req_rep| {
        if req_rep.status() == Some(http::StatusCode::SERVICE_UNAVAILABLE) {
            req_rep.retryable()
        } else {
            req_rep.success()
        }
    });

    let url = format!("http://{}", server.addr());
    let resp = Client::builder()
        .retry(policy)
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn status_recovery_retries_payment_required_once() {
    let _ = pretty_env_logger::try_init();

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_assert = attempts.clone();
    let server = server::http(move |req| {
        let attempts = attempts.clone();
        async move {
            let attempt = attempts.fetch_add(1, Ordering::Relaxed);

            if attempt == 0 {
                return http::Response::builder()
                    .status(http::StatusCode::PAYMENT_REQUIRED)
                    .body(Body::from("payment-required"))
                    .unwrap();
            }

            let paid = req
                .headers()
                .get("x-payment")
                .and_then(|value| value.to_str().ok());

            if paid == Some("ok") {
                http::Response::default()
            } else {
                http::Response::builder()
                    .status(http::StatusCode::BAD_REQUEST)
                    .body(Body::from("missing-payment"))
                    .unwrap()
            }
        }
    });

    let client = Client::builder()
        .on_status(http::StatusCode::PAYMENT_REQUIRED, |ctx| async move {
            assert_eq!(ctx.status(), http::StatusCode::PAYMENT_REQUIRED);
            assert_eq!(ctx.body().as_ref(), b"payment-required");

            let mut request = ctx
                .into_original_request()
                .expect("request body should be replayable");
            request.headers_mut().insert(
                http::header::HeaderName::from_static("x-payment"),
                http::HeaderValue::from_static("ok"),
            );
            Ok(Some(request))
        })
        .build()
        .unwrap();

    let url = format!("http://{}", server.addr());
    let response = client.post(url).body("payload").send().await.unwrap();

    assert_eq!(response.status(), http::StatusCode::OK);
    assert_eq!(attempts_for_assert.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn status_recovery_skips_non_replayable_bodies() {
    let _ = pretty_env_logger::try_init();

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_assert = attempts.clone();
    let server = server::http(move |_req| {
        let attempts = attempts.clone();
        async move {
            attempts.fetch_add(1, Ordering::Relaxed);
            http::Response::builder()
                .status(http::StatusCode::PAYMENT_REQUIRED)
                .body(Body::from("payment-required"))
                .unwrap()
        }
    });

    let client = Client::builder()
        .on_status(http::StatusCode::PAYMENT_REQUIRED, |ctx| async move {
            assert!(ctx.into_original_request().is_none());
            Ok(None)
        })
        .build()
        .unwrap();

    let url = format!("http://{}", server.addr());
    let response = client
        .post(url)
        .body(Body::wrap(Full::new(Bytes::from_static(b"payload"))))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), http::StatusCode::PAYMENT_REQUIRED);
    assert_eq!(response.text().await.unwrap(), "payment-required");
    assert_eq!(attempts_for_assert.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn status_recovery_skips_oversized_bodies() {
    let _ = pretty_env_logger::try_init();

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_assert = attempts.clone();
    let oversized_body = "x".repeat(70_000);
    let expected_len = oversized_body.len();

    let server = server::http(move |_req| {
        let attempts = attempts.clone();
        let oversized_body = oversized_body.clone();
        async move {
            attempts.fetch_add(1, Ordering::Relaxed);
            http::Response::builder()
                .status(http::StatusCode::PAYMENT_REQUIRED)
                .body(Body::from(oversized_body))
                .unwrap()
        }
    });

    let client = Client::builder()
        .on_status(http::StatusCode::PAYMENT_REQUIRED, |_ctx| async move {
            panic!("oversized bodies should bypass recovery buffering")
        })
        .build()
        .unwrap();

    let url = format!("http://{}", server.addr());
    let response = client.get(url).send().await.unwrap();

    assert_eq!(response.status(), http::StatusCode::PAYMENT_REQUIRED);
    assert_eq!(response.text().await.unwrap().len(), expected_len);
    assert_eq!(attempts_for_assert.load(Ordering::Relaxed), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_retries_have_a_limit() {
    let _ = pretty_env_logger::try_init();

    let server = server::http_with_config(
        move |req| async move {
            assert_eq!(req.version(), http::Version::HTTP_2);
            // refused forever
            Err(http2::Error::from(http2::Reason::REFUSED_STREAM))
        },
        |_| {},
    );

    let client = Client::builder().http2_only().build().unwrap();

    let url = format!("http://{}", server.addr());

    let _err = client.get(url).send().await.unwrap_err();
}

// NOTE: using the default "current_thread" runtime here would cause the test to
// fail, because the only thread would block until `panic_rx` receives a
// notification while the client needs to be driven to get the graceful shutdown
// done.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn highly_concurrent_requests_to_http2_server_with_low_max_concurrent_streams() {
    let client = Client::builder().http2_only().no_proxy().build().unwrap();

    let server = server::http_with_config(
        move |req| async move {
            assert_eq!(req.version(), http::Version::HTTP_2);
            Ok::<_, std::convert::Infallible>(http::Response::default())
        },
        |builder| {
            builder.http2().max_concurrent_streams(1);
        },
    );

    let url = format!("http://{}", server.addr());

    let futs = (0..100).map(|_| {
        let client = client.clone();
        let url = url.clone();
        async move {
            let res = client.get(&url).send().await.unwrap();
            assert_eq!(res.status(), hpx::StatusCode::OK);
        }
    });
    futures_util::future::join_all(futs).await;
}

#[tokio::test]
async fn highly_concurrent_requests_to_slow_http2_server_with_low_max_concurrent_streams() {
    use support::delay_server;

    let client = Client::builder().http2_only().no_proxy().build().unwrap();

    let server = delay_server::Server::new(
        move |req| async move {
            assert_eq!(req.version(), http::Version::HTTP_2);
            http::Response::default()
        },
        |http| {
            http.http2().max_concurrent_streams(1);
        },
        std::time::Duration::from_secs(2),
    )
    .await;

    let url = format!("http://{}", server.addr());

    let futs = (0..100).map(|_| {
        let client = client.clone();
        let url = url.clone();
        async move {
            let res = client.get(&url).send().await.unwrap();
            assert_eq!(res.status(), hpx::StatusCode::OK);
        }
    });
    futures_util::future::join_all(futs).await;

    server.shutdown().await;
}
