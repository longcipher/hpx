//! Integration tests for the reconnecting SSE client (`hpx::sse`).
//!
//! Covers the connection lifecycle against a local mock SSE server:
//! happy path, status/content-type rejection, `Last-Event-ID` reconnection,
//! graceful close, and retry exhaustion.

#![cfg(feature = "sse")]

mod support;

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures_util::StreamExt;
use hpx::{
    Client,
    sse::{
        EventSource, RequestBuilderSseExt, SseClientError as SseError, SseEvent, SseRetryConfig,
    },
};
use support::server;

fn sse_body(events: String) -> hpx::Body {
    hpx::Body::wrap_stream(futures_util::stream::once(async move {
        Ok::<_, std::io::Error>(bytes::Bytes::from(events))
    }))
}

fn sse_response(events: &str) -> http::Response<hpx::Body> {
    let mut resp = http::Response::new(sse_body(events.to_string()));
    resp.headers_mut().insert(
        http::header::CONTENT_TYPE,
        http::header::HeaderValue::from_static("text/event-stream"),
    );
    resp
}

#[tokio::test]
async fn basic_connection_and_event_reception() {
    let server = server::http(move |_req| async move {
        sse_response("data: hello\n\nid: 2\nevent: tick\ndata: world\n\n")
    });

    let client = Client::builder().no_proxy().build().unwrap();
    let mut source = client
        .get(format!("http://{}/events", server.addr()))
        .into_event_source();

    // Open
    match source.next().await {
        Some(Ok(SseEvent::Open)) => {}
        other => panic!("expected Open, got {other:?}"),
    }

    // First message
    match source.next().await {
        Some(Ok(SseEvent::Message(msg))) => {
            assert_eq!(msg.data, "hello");
            assert_eq!(msg.event, "message");
        }
        other => panic!("expected hello message, got {other:?}"),
    }

    // Second named message with id
    match source.next().await {
        Some(Ok(SseEvent::Message(msg))) => {
            assert_eq!(msg.data, "world");
            assert_eq!(msg.event, "tick");
            assert_eq!(msg.last_event_id.as_deref(), Some("2"));
        }
        other => panic!("expected tick message, got {other:?}"),
    }

    source.close();
}

#[tokio::test]
async fn non_2xx_status_is_terminal_error() {
    let server = server::http(move |_req| async move {
        let mut resp = http::Response::new(hpx::Body::from("nope"));
        *resp.status_mut() = http::StatusCode::FORBIDDEN;
        resp
    });

    let client = Client::builder().no_proxy().build().unwrap();
    let mut source = client
        .get(format!("http://{}/events", server.addr()))
        .into_event_source();

    match source.next().await {
        Some(Err(SseError::Status(status))) => {
            assert_eq!(status, http::StatusCode::FORBIDDEN);
        }
        other => panic!("expected Status(403), got {other:?}"),
    }
}

#[tokio::test]
async fn invalid_content_type_is_rejected() {
    let server = server::http(move |_req| async move {
        let mut resp = http::Response::new(hpx::Body::from("not sse"));
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::header::HeaderValue::from_static("application/json"),
        );
        resp
    });

    let client = Client::builder().no_proxy().build().unwrap();
    let mut source = client
        .get(format!("http://{}/events", server.addr()))
        .into_event_source();

    match source.next().await {
        Some(Err(SseError::InvalidContentType)) => {}
        other => panic!("expected InvalidContentType, got {other:?}"),
    }
}

#[tokio::test]
async fn missing_content_type_is_rejected() {
    let server =
        server::http(
            move |_req| async move { http::Response::new(hpx::Body::from("data: x\n\n")) },
        );

    let client = Client::builder().no_proxy().build().unwrap();
    let mut source = client
        .get(format!("http://{}/events", server.addr()))
        .into_event_source();

    match source.next().await {
        Some(Err(SseError::MissingContentType)) => {}
        other => panic!("expected MissingContentType, got {other:?}"),
    }
}

#[tokio::test]
async fn graceful_close_ends_stream() {
    let server =
        server::http(move |_req| async move { sse_response("data: one\n\ndata: two\n\n") });

    let client = Client::builder().no_proxy().build().unwrap();
    let mut source = client
        .get(format!("http://{}/events", server.addr()))
        .into_event_source();

    assert!(matches!(source.next().await, Some(Ok(SseEvent::Open))));
    assert!(matches!(
        source.next().await,
        Some(Ok(SseEvent::Message(_)))
    ));

    source.close();
    assert!(source.next().await.is_none());
}

/// Mock raw HTTP/1.1 SSE server that records `Last-Event-ID` per connection
/// and can serve a configurable number of events before closing.
fn spawn_raw_sse_server(
    events_per_conn: &'static str,
    max_connections: usize,
) -> (
    std::net::SocketAddr,
    Arc<AtomicUsize>,
    Arc<Vec<std::sync::Mutex<Option<String>>>>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let hits = Arc::new(AtomicUsize::new(0));
    let last_ids = Arc::new(
        (0..max_connections)
            .map(|_| std::sync::Mutex::new(None))
            .collect::<Vec<_>>(),
    );
    let hits_clone = hits.clone();
    let last_ids_clone = last_ids.clone();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let mut handles = Vec::new();
            while hits_clone.load(Ordering::SeqCst) < max_connections {
                let accepted = tokio::time::timeout(
                    Duration::from_millis(200),
                    listener.accept(),
                )
                .await;
                let Ok(Ok((mut sock, _))) = accepted else {
                    continue;
                };
                let n = hits_clone.fetch_add(1, Ordering::SeqCst);
                if n >= max_connections {
                    break;
                }
                let last_ids_clone = last_ids_clone.clone();
                handles.push(tokio::spawn(async move {
                    // Read request head.
                    let mut buf = vec![0u8; 8192];
                    let mut read = 0;
                    loop {
                        match sock.read(&mut buf[read..]).await {
                            Ok(0) => break,
                            Ok(m) => {
                                read += m;
                                if read >= buf.len() || buf[..read].windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                            }
                            Err(_) => return,
                        }
                    }
                    let head = String::from_utf8_lossy(&buf[..read]).to_string();
                    let last_event_id = head
                        .lines()
                        .find_map(|line| {
                            let lower = line.to_ascii_lowercase();
                            lower
                                .strip_prefix("last-event-id:")
                                .map(|v| v.trim().to_string())
                        })
                        .filter(|s| !s.is_empty());
                    if let Some(slot) = last_ids_clone.get(n) {
                        *slot.lock().unwrap() = last_event_id;
                    }

                    let body = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{events_per_conn}"
                    );
                    let _ = sock.write_all(body.as_bytes()).await;
                    let _ = sock.flush().await;
                    let _ = sock.shutdown().await;
                }));
            }
            for h in handles {
                let _ = h.await;
            }
        });
    });

    (addr, hits, last_ids)
}

#[tokio::test]
async fn reconnect_sends_last_event_id() {
    // First connection: emit id=42 then close. Second connection: any body.
    let (addr, hits, last_ids) = spawn_raw_sse_server("id: 42\ndata: first\n\n", 2);

    let client = Client::builder().no_proxy().build().unwrap();
    let mut source = EventSource::builder(client.get(format!("http://{addr}/events")))
        .retry_config(SseRetryConfig {
            max_retries: 3,
            max_backoff_ms: 50,
            min_sleep_ms: 10,
            backoff_multiplier: 1.0,
            jitter: false,
        })
        .initial_reconnection_time(Duration::from_millis(10))
        .build();

    assert!(matches!(source.next().await, Some(Ok(SseEvent::Open))));
    assert!(matches!(
        source.next().await,
        Some(Ok(SseEvent::Message(_)))
    ));

    // Connection drops → Error(Eof) then reconnect. Keep polling: the
    // EventSource only progresses while the stream is polled.
    let mut saw_reconnect = false;
    for _ in 0..50 {
        match source.next().await {
            Some(Ok(SseEvent::Error(_))) => {
                saw_reconnect = true;
                // Keep polling until the second connection is observed.
                while hits.load(Ordering::SeqCst) < 2 {
                    match source.next().await {
                        Some(Ok(_)) | Some(Err(_)) => continue,
                        None => break,
                    }
                }
                break;
            }
            Some(Ok(_)) => continue,
            Some(Err(e)) => panic!("unexpected error before reconnect: {e}"),
            None => break,
        }
    }
    assert!(saw_reconnect, "expected a reconnect error event");
    assert!(
        hits.load(Ordering::SeqCst) >= 2,
        "expected a second connection (hits={})",
        hits.load(Ordering::SeqCst)
    );

    let recorded = last_ids[1].lock().unwrap().clone();
    assert_eq!(
        recorded.as_deref(),
        Some("42"),
        "second request must carry Last-Event-ID"
    );

    source.close();
}

#[tokio::test]
async fn max_reconnect_attempts_is_terminal() {
    // Server always closes immediately after headers, no events.
    let (addr, _, _) = spawn_raw_sse_server("", 8);

    let client = Client::builder().no_proxy().build().unwrap();
    let mut source = EventSource::builder(client.get(format!("http://{addr}/events")))
        .retry_config(SseRetryConfig {
            max_retries: 2,
            max_backoff_ms: 20,
            min_sleep_ms: 5,
            backoff_multiplier: 1.0,
            jitter: false,
        })
        .initial_reconnection_time(Duration::from_millis(5))
        .build();

    let mut errors = 0u32;
    let mut terminal = false;
    for _ in 0..30 {
        match source.next().await {
            Some(Ok(SseEvent::Error(_))) => errors += 1,
            Some(Err(SseError::Timeout(_, _))) => {
                terminal = true;
                break;
            }
            Some(Ok(SseEvent::Open)) => {}
            Some(Ok(_)) => {}
            None => break,
            Some(Err(e)) => panic!("unexpected error: {e}"),
        }
    }
    assert!(
        terminal,
        "expected Timeout after max attempts (errors={errors})"
    );
}

#[tokio::test]
async fn no_content_ends_stream() {
    let server = server::http(move |_req| async move {
        let mut resp = http::Response::new(hpx::Body::from(""));
        *resp.status_mut() = http::StatusCode::NO_CONTENT;
        resp
    });

    let client = Client::builder().no_proxy().build().unwrap();
    let mut source = client
        .get(format!("http://{}/events", server.addr()))
        .into_event_source();

    assert!(source.next().await.is_none());
}
