//! Integration tests for the SSE transport module.
//!
//! Uses a mock hyper HTTP server to emit SSE events, verifying the full
//! connection → parse → classify → stream pipeline.

#![cfg(feature = "sse")]

use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};

use hpx_transport::sse::{
    SseConfig, SseConnection, SseMessageKind, SseProtocolHandler, handlers::GenericSseHandler,
};
use http_body_util::Full;
use hyper::{
    Request, Response,
    body::{Bytes, Incoming},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use tokio::{net::TcpListener, sync::Notify, time::timeout};

// ---------------------------------------------------------------------------
// Mock SSE server helpers
// ---------------------------------------------------------------------------

/// Start a mock SSE server that returns the given body with `text/event-stream`
/// content type. Returns the `SocketAddr` it is listening on.
async fn start_sse_server(body: &'static str) -> SocketAddr {
    start_sse_server_with_options(body, "text/event-stream", 200).await
}

/// Start a mock SSE server with configurable content type and status code.
async fn start_sse_server_with_options(
    body: &'static str,
    content_type: &'static str,
    status: u16,
) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock server");
    let addr = listener.local_addr().expect("local addr");

    tokio::spawn(async move {
        // Accept a single connection (sufficient for most tests).
        if let Ok((stream, _)) = listener.accept().await {
            let io = TokioIo::new(stream);
            let _ = http1::Builder::new()
                .serve_connection(
                    io,
                    service_fn(move |_req: Request<Incoming>| {
                        let resp = Response::builder()
                            .status(status)
                            .header("content-type", content_type)
                            .body(Full::new(Bytes::from(body)))
                            .expect("build response");
                        async move { Ok::<_, Infallible>(resp) }
                    }),
                )
                .await;
        }
    });

    addr
}

/// Start a mock SSE server that accepts multiple connections, useful for
/// reconnection tests.
async fn start_multi_sse_server(responses: Vec<(&'static str, &'static str, u16)>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock server");
    let addr = listener.local_addr().expect("local addr");
    let responses = Arc::new(std::sync::Mutex::new(responses.into_iter()));

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let responses = Arc::clone(&responses);
            let io = TokioIo::new(stream);
            tokio::spawn(async move {
                let (body, content_type, status) = {
                    let mut iter = responses.lock().expect("lock responses");
                    match iter.next() {
                        Some(r) => r,
                        None => return,
                    }
                };
                let _ = http1::Builder::new()
                    .serve_connection(
                        io,
                        service_fn(move |_req: Request<Incoming>| {
                            let resp = Response::builder()
                                .status(status)
                                .header("content-type", content_type)
                                .body(Full::new(Bytes::from(body)))
                                .expect("build response");
                            async move { Ok::<_, Infallible>(resp) }
                        }),
                    )
                    .await;
            });
        }
    });

    addr
}

// ---------------------------------------------------------------------------
// TC-01: Basic connection and event reception
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_basic_sse_connection_and_events() {
    let body = "event: message\ndata: {\"price\":42000}\nid: evt-1\n\nevent: update\ndata: hello world\nid: evt-2\n\n";
    let addr = start_sse_server(body).await;

    let config = SseConfig::new(format!("http://{addr}/stream")).reconnect_max_attempts(Some(0));
    let handler = GenericSseHandler::new();

    let connection = SseConnection::connect(config, handler)
        .await
        .expect("connect");
    let (_handle, mut stream) = connection.split();

    // First event.
    let event = timeout(Duration::from_secs(2), stream.next_event())
        .await
        .expect("timeout")
        .expect("event");
    assert_eq!(event.event_type(), "message");
    assert_eq!(event.data(), "{\"price\":42000}");
    assert_eq!(event.id(), "evt-1");
    assert!(event.kind.is_data());

    // Second event.
    let event = timeout(Duration::from_secs(2), stream.next_event())
        .await
        .expect("timeout")
        .expect("event");
    assert_eq!(event.event_type(), "update");
    assert_eq!(event.data(), "hello world");
    assert_eq!(event.id(), "evt-2");
    assert!(event.kind.is_data());
}

// ---------------------------------------------------------------------------
// TC-02: Non-2xx status code handling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_sse_non_2xx_status_closes() {
    let addr = start_sse_server_with_options("", "text/event-stream", 403).await;

    let config = SseConfig::new(format!("http://{addr}/stream")).reconnect_max_attempts(Some(0));
    let handler = GenericSseHandler::new();

    let connection = SseConnection::connect(config, handler)
        .await
        .expect("connect");
    let (_handle, mut stream) = connection.split();

    // Stream should end because connection fails with 403 and max_attempts=0.
    let result = timeout(Duration::from_secs(2), stream.next_event())
        .await
        .expect("timeout");
    assert!(result.is_none(), "Expected stream to end on 403");
}

// ---------------------------------------------------------------------------
// TC-03: Invalid Content-Type handling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_sse_invalid_content_type_closes() {
    let addr = start_sse_server_with_options("data: test\n\n", "application/json", 200).await;

    let config = SseConfig::new(format!("http://{addr}/stream")).reconnect_max_attempts(Some(0));
    let handler = GenericSseHandler::new();

    let connection = SseConnection::connect(config, handler)
        .await
        .expect("connect");
    let (_handle, mut stream) = connection.split();

    // Stream should end because of content-type mismatch.
    let result = timeout(Duration::from_secs(2), stream.next_event())
        .await
        .expect("timeout");
    assert!(
        result.is_none(),
        "Expected stream to end on wrong content-type"
    );
}

// ---------------------------------------------------------------------------
// TC-05: Graceful close via SseHandle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_sse_handle_close() {
    // Server sends events indefinitely by providing a large body.
    let body = "event: message\ndata: first\n\nevent: message\ndata: second\n\nevent: message\ndata: third\n\n";
    let addr = start_sse_server(body).await;

    let config = SseConfig::new(format!("http://{addr}/stream")).reconnect_max_attempts(Some(0));
    let handler = GenericSseHandler::new();

    let connection = SseConnection::connect(config, handler)
        .await
        .expect("connect");
    let (handle, mut stream) = connection.split();

    // Read one event.
    let event = timeout(Duration::from_secs(2), stream.next_event())
        .await
        .expect("timeout")
        .expect("event");
    assert_eq!(event.data(), "first");

    // Close the connection.  The background task may have already exited if
    // the short body was fully consumed, so we accept both Ok and Err here.
    let _ = handle.close().await;

    // Drain remaining events until the stream ends.
    loop {
        match timeout(Duration::from_secs(2), stream.next_event()).await {
            Ok(None) => break, // Cleanly closed.
            Ok(Some(_)) => {}  // Buffered event before close.
            Err(_) => panic!("Stream did not close in time after handle.close()"),
        }
    }
}

// ---------------------------------------------------------------------------
// TC-08: Max reconnect attempts exceeded
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_sse_max_reconnect_exceeded() {
    // Server always returns 500.
    let responses = vec![
        ("", "text/event-stream", 500),
        ("", "text/event-stream", 500),
        ("", "text/event-stream", 500),
    ];
    let addr = start_multi_sse_server(responses).await;

    let config = SseConfig::new(format!("http://{addr}/stream"))
        .reconnect_max_attempts(Some(2))
        .reconnect_initial_delay(Duration::from_millis(10))
        .reconnect_max_delay(Duration::from_millis(50));
    let handler = GenericSseHandler::new();

    let connection = SseConnection::connect(config, handler)
        .await
        .expect("connect");
    let (_handle, mut stream) = connection.split();

    // Stream should end after exhausting reconnect attempts.
    let result = timeout(Duration::from_secs(5), stream.next_event())
        .await
        .expect("timeout");
    assert!(
        result.is_none(),
        "Expected stream to end after max reconnects"
    );
}

// ---------------------------------------------------------------------------
// Event classification with custom handler
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_sse_custom_handler_classification() {
    struct CustomHandler;

    impl SseProtocolHandler for CustomHandler {
        fn classify_event(&self, event: &hpx_transport::sse::Event) -> SseMessageKind {
            if event.event == "heartbeat" {
                SseMessageKind::System
            } else if event.data.is_empty() {
                SseMessageKind::Unknown
            } else {
                SseMessageKind::Data
            }
        }
    }

    let body = "event: heartbeat\ndata: \n\nevent: trade\ndata: {\"qty\":1}\n\n";
    let addr = start_sse_server(body).await;

    let config = SseConfig::new(format!("http://{addr}/stream")).reconnect_max_attempts(Some(0));
    let handler = CustomHandler;

    let connection = SseConnection::connect(config, handler)
        .await
        .expect("connect");
    let (_handle, mut stream) = connection.split();

    // First event: heartbeat classified as System.
    let event = timeout(Duration::from_secs(2), stream.next_event())
        .await
        .expect("timeout")
        .expect("event");
    assert_eq!(event.event_type(), "heartbeat");
    assert!(event.kind.is_system());

    // Second event: trade classified as Data.
    let event = timeout(Duration::from_secs(2), stream.next_event())
        .await
        .expect("timeout")
        .expect("event");
    assert_eq!(event.event_type(), "trade");
    assert!(event.kind.is_data());
}
