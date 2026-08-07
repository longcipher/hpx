#![allow(clippy::pedantic)]

//! Regression test: a request that hits the request-level timeout must be
//! retried for idempotent methods (GET). The tower `Retry` layer must wrap the
//! outer request `Timeout` so a timed-out attempt can still be retried with a
//! fresh timeout budget.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use hpx::Client;

#[tokio::test]
async fn timeout_retries_for_idempotent_get() {
    // Blackhole server: accepts TCP connections, counts them, but never
    // responds — so the client hits the request timeout on every attempt.
    let conn_count = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    {
        let conn_count = conn_count.clone();
        tokio::spawn(async move {
            let mut conns = Vec::new();
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                conn_count.fetch_add(1, Ordering::Relaxed);
                // Keep the socket open but never write a response.
                conns.push(stream);
            }
        });
    }

    let url = format!("http://{addr}/");
    let err = Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap_err();

    assert!(err.is_timeout(), "expected timeout error, got: {err}");
    // Default retry policy: max_retries_per_request = 2 → up to 3 attempts.
    let attempts = conn_count.load(Ordering::Relaxed);
    assert!(
        attempts >= 2,
        "timed-out GET should be retried; expected >=2 connection attempts, got {attempts}"
    );
}
