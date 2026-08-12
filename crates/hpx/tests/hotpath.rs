#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration tests exercise error paths with unwrap/panic"
)]
#![allow(missing_docs)]

//! Integration tests for hotpath profiling support: the `HotpathLayer`
//! middleware and the WebSocket handshake/message instrumentation must work
//! when the `hotpath` feature is enabled, and the client must behave normally
//! with and without it.

mod support;

use axum::{
    Router,
    extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
};
use futures_util::StreamExt;
use hpx::{Client, hotpath::HotpathLayer, ws::message::Message};
use support::server;
use tokio::net::TcpListener;

async fn start_ws_server() -> String {
    let app = Router::new().route("/ws", get(ws_handler));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("ws://{addr}/ws")
}

async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(echo_ws)
}

async fn echo_ws(mut ws: WebSocket) {
    while let Some(Ok(msg)) = ws.recv().await {
        match msg {
            WsMessage::Text(text) => {
                let _ = ws.send(WsMessage::Text(text)).await;
            }
            WsMessage::Binary(data) => {
                let _ = ws.send(WsMessage::Binary(data)).await;
            }
            _ => {}
        }
    }
}

fn find_stat<'a>(
    stats: &'a [hpx::hotpath::EndpointStat],
    endpoint: &str,
) -> Option<&'a hpx::hotpath::EndpointStat> {
    stats.iter().find(|s| s.endpoint == endpoint)
}

#[tokio::test]
async fn hotpath_layer_records_normalized_endpoint_stats() {
    let server = server::http(move |req| async move {
        match req.uri().path() {
            "/ok" => http::Response::builder()
                .status(200)
                .body(hpx::Body::from("ok"))
                .unwrap(),
            "/users/1" | "/users/2" => http::Response::builder()
                .status(200)
                .body(hpx::Body::from("user"))
                .unwrap(),
            "/missing" => http::Response::builder()
                .status(404)
                .body(hpx::Body::from("nope"))
                .unwrap(),
            _ => http::Response::builder()
                .status(500)
                .body(hpx::Body::default())
                .unwrap(),
        }
    });

    let client = Client::builder()
        .layer(HotpathLayer::new())
        .build()
        .unwrap();
    let base = format!("http://{}", server.addr());

    for id in 1..=2 {
        let resp = client
            .get(format!("{base}/users/{id}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), hpx::StatusCode::OK);
    }
    let resp = client.get(format!("{base}/ok")).send().await.unwrap();
    assert_eq!(resp.status(), hpx::StatusCode::OK);
    let resp = client.get(format!("{base}/missing")).send().await.unwrap();
    assert_eq!(resp.status(), hpx::StatusCode::NOT_FOUND);

    let stats = hpx::hotpath::snapshot();
    let users = find_stat(&stats, &format!("GET {}/users/{{id}}", server.addr()))
        .expect("normalized users endpoint recorded");
    assert_eq!(users.count, 2);
    assert_eq!(users.error_count, 0);
    assert_eq!(users.statuses, vec![(200, 2)]);

    let ok = find_stat(&stats, &format!("GET {}/ok", server.addr())).expect("ok endpoint recorded");
    assert_eq!(ok.count, 1);
    assert_eq!(ok.statuses, vec![(200, 1)]);

    let missing = find_stat(&stats, &format!("GET {}/missing", server.addr()))
        .expect("missing endpoint recorded");
    assert_eq!(missing.count, 1);
    // 404 counts as an error, mirroring hotpath's own HTTP semantics.
    assert_eq!(missing.error_count, 1);
    assert_eq!(missing.statuses, vec![(404, 1)]);
}

#[tokio::test]
async fn hotpath_layer_label_prefixes_endpoint_keys() {
    let server = server::http(|_req| async move {
        http::Response::builder()
            .status(200)
            .body(hpx::Body::from("ok"))
            .unwrap()
    });

    let client = Client::builder()
        .layer(HotpathLayer::with_label("bench"))
        .build()
        .unwrap();

    let resp = client
        .get(format!("http://{}/ok", server.addr()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), hpx::StatusCode::OK);

    let stats = hpx::hotpath::snapshot();
    let entry = find_stat(&stats, &format!("bench: GET {}/ok", server.addr()))
        .expect("labeled endpoint recorded");
    assert_eq!(entry.count, 1);
}

#[tokio::test]
async fn client_without_layer_works_when_hotpath_feature_is_enabled() {
    let server = server::http(|_req| async move {
        http::Response::builder()
            .status(200)
            .body(hpx::Body::from("hello"))
            .unwrap()
    });

    // No HotpathLayer: exercises the plain client path under the feature,
    // including the `#[hotpath::measure]` response body instrumentation.
    let client = Client::new();
    let resp = client
        .get(format!("http://{}/hello", server.addr()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), hpx::StatusCode::OK);
    let body = resp.bytes().await.unwrap();
    assert_eq!(&body[..], b"hello");
}

#[tokio::test]
async fn websocket_handshake_and_echo_work_with_hotpath_feature() {
    let ws_url = start_ws_server().await;

    let resp = hpx::websocket(&ws_url).send().await.unwrap();
    let ws = resp.into_websocket().await.unwrap();
    let (mut tx, mut rx) = ws.split();

    for i in 1..=3 {
        let text = format!("ping {i}");
        tx.send(Message::text(text.clone())).await.unwrap();
        let echo = rx.next().await.expect("echo message");
        let echo = echo.unwrap();
        match echo {
            Message::Text(echoed) => assert_eq!(echoed.as_str(), text.as_str()),
            other => panic!("expected text echo, got {other:?}"),
        }
    }
    tx.close(hpx::ws::message::CloseCode::NORMAL, "done")
        .await
        .unwrap();
}
