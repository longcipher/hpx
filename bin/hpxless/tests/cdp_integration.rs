//! CDP integration tests for hpxless.
//!
//! Tests the CDP protocol end-to-end: server lifecycle, navigation,
//! evaluation, events, and multi-page support.

#![allow(clippy::print_stdout)]
#![allow(clippy::print_stderr)]
#![allow(missing_docs)]

use futures_util::{SinkExt, StreamExt};
use hpx_browser::protocol::CdpServer;
use hpx_yawc::{Frame, WebSocket, frame::OpCode};
use serde_json::json;

struct CdpClient {
    ws: hpx_yawc::TcpWebSocket,
    next_id: u64,
    events: Vec<serde_json::Value>,
}

impl CdpClient {
    async fn connect(port: u16) -> Self {
        let url: url::Url = format!("ws://127.0.0.1:{port}").parse().unwrap();
        let ws = WebSocket::connect(url).await.expect("connect to CDP");
        Self {
            ws,
            next_id: 1,
            events: Vec::new(),
        }
    }

    async fn send(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"id": id, "method": method, "params": params});
        self.ws
            .send(Frame::text(msg.to_string()))
            .await
            .expect(&format!("send {method}"));

        while let Some(frame) = self.ws.next().await {
            if frame.opcode() == OpCode::Text {
                let text = frame.as_str();
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
                    if value.get("id").is_none() {
                        self.events.push(value);
                        continue;
                    }
                    if value.get("id").and_then(serde_json::Value::as_u64) == Some(id) {
                        return value;
                    }
                }
            }
        }
        panic!("no {method} response received");
    }

    fn take_events(&mut self) -> Vec<serde_json::Value> {
        std::mem::take(&mut self.events)
    }
}

// ── Server Lifecycle ─────────────────────────────────────────────────────

#[tokio::test]
async fn server_starts_on_ephemeral_port() {
    let server = CdpServer::start_ephemeral("").unwrap();
    assert!(server.port() > 0, "port should be nonzero");
    drop(server);
}

#[tokio::test]
async fn server_starts_with_html() {
    let server = CdpServer::start("<html><body>hello</body></html>", 0, false, None).unwrap();
    assert!(server.port() > 0);
    drop(server);
}

#[tokio::test]
async fn server_starts_with_stealth() {
    let server = CdpServer::start("", 0, true, None).unwrap();
    assert!(server.port() > 0);
    drop(server);
}

// ── Browser.getVersion ───────────────────────────────────────────────────

#[tokio::test]
async fn browser_get_version() {
    let server = CdpServer::start("", 0, false, None).unwrap();
    let mut client = CdpClient::connect(server.port()).await;

    let resp = client.send("Browser.getVersion", json!({})).await;
    let result = resp.get("result").expect("result field");
    assert!(
        result.get("protocolVersion").is_some(),
        "should have protocolVersion"
    );
    assert!(result.get("product").is_some(), "should have product");
}

// ── Runtime.evaluate ─────────────────────────────────────────────────────

#[tokio::test]
async fn runtime_evaluate_simple_expression() {
    let server = CdpServer::start("", 0, false, None).unwrap();
    let mut client = CdpClient::connect(server.port()).await;

    let resp = client
        .send(
            "Runtime.evaluate",
            json!({"expression": "1 + 2", "returnByValue": true}),
        )
        .await;
    let value = resp
        .get("result")
        .and_then(|r| r.get("result"))
        .and_then(|r| r.get("value"))
        .and_then(serde_json::Value::as_i64);
    assert_eq!(value, Some(3), "1+2 should equal 3");
}

#[tokio::test]
async fn runtime_evaluate_string() {
    let server = CdpServer::start("", 0, false, None).unwrap();
    let mut client = CdpClient::connect(server.port()).await;

    let resp = client
        .send(
            "Runtime.evaluate",
            json!({"expression": "'hello ' + 'world'", "returnByValue": true}),
        )
        .await;
    let value = resp
        .get("result")
        .and_then(|r| r.get("result"))
        .and_then(|r| r.get("value"))
        .and_then(serde_json::Value::as_str);
    assert_eq!(value, Some("hello world"));
}

#[tokio::test]
async fn runtime_evaluate_boolean() {
    let server = CdpServer::start("", 0, false, None).unwrap();
    let mut client = CdpClient::connect(server.port()).await;

    let resp = client
        .send(
            "Runtime.evaluate",
            json!({"expression": "true", "returnByValue": true}),
        )
        .await;
    let value = resp
        .get("result")
        .and_then(|r| r.get("result"))
        .and_then(|r| r.get("value"))
        .and_then(serde_json::Value::as_bool);
    assert_eq!(value, Some(true));
}

// ── Page.navigate + Lifecycle Events ─────────────────────────────────────

#[tokio::test]
async fn page_navigate_with_data_url() {
    let server = CdpServer::start("", 0, false, None).unwrap();
    let mut client = CdpClient::connect(server.port()).await;

    // Enable Page domain for lifecycle events
    client.send("Page.enable", json!({})).await;

    let resp = client
        .send(
            "Page.navigate",
            json!({"url": "data:text/html,<h1>Hello</h1>"}),
        )
        .await;

    // Should get a frameId
    let frame_id = resp.get("result").and_then(|r| r.get("frameId"));
    assert!(frame_id.is_some(), "should return frameId");

    // Should receive lifecycle events
    let events = client.take_events();
    let event_methods: Vec<_> = events
        .iter()
        .map(|e| {
            e.get("method")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
        })
        .collect();

    assert!(
        event_methods.contains(&"Page.frameNavigated"),
        "should receive Page.frameNavigated, got: {event_methods:?}"
    );
    assert!(
        event_methods.contains(&"Page.loadEventFired"),
        "should receive Page.loadEventFired, got: {event_methods:?}"
    );
}

#[tokio::test]
async fn page_navigate_with_network_events() {
    let server = CdpServer::start("", 0, false, None).unwrap();
    let mut client = CdpClient::connect(server.port()).await;

    // Enable both Page and Network domains
    client.send("Page.enable", json!({})).await;
    client.send("Network.enable", json!({})).await;

    client
        .send(
            "Page.navigate",
            json!({"url": "data:text/html,<p>test</p>"}),
        )
        .await;

    let events = client.take_events();
    let event_methods: Vec<_> = events
        .iter()
        .map(|e| {
            e.get("method")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
        })
        .collect();

    assert!(
        event_methods.contains(&"Network.requestWillBeSent"),
        "should receive Network.requestWillBeSent, got: {event_methods:?}"
    );
    assert!(
        event_methods.contains(&"Network.responseReceived"),
        "should receive Network.responseReceived, got: {event_methods:?}"
    );
}

// ── DOM.getDocument ──────────────────────────────────────────────────────

#[tokio::test]
async fn dom_get_document() {
    let server = CdpServer::start("<html><body>test</body></html>", 0, false, None).unwrap();
    let mut client = CdpClient::connect(server.port()).await;

    client.send("DOM.enable", json!({})).await;
    let resp = client.send("DOM.getDocument", json!({})).await;

    let root = resp.get("result").and_then(|r| r.get("root"));
    assert!(root.is_some(), "should return root node");
}

// ── Multiple Sequential Connections ───────────────────────────────────────
// Note: V8 isolates are !Send, so true concurrent connections across threads
// will crash. The CDP server uses spawn_local (same thread). Testing sequential
// connections verifies the server handles multiple accept cycles.

#[tokio::test]
async fn multiple_sequential_connections() {
    let server = CdpServer::start("", 0, false, None).unwrap();
    let port = server.port();

    for i in 0..5 {
        let mut client = CdpClient::connect(port).await;
        let resp = client
            .send(
                "Runtime.evaluate",
                json!({"expression": format!("{i} * 10"), "returnByValue": true}),
            )
            .await;
        let value = resp
            .get("result")
            .and_then(|r| r.get("result"))
            .and_then(|r| r.get("value"))
            .and_then(serde_json::Value::as_i64);
        assert_eq!(value, Some(i * 10));
    }
}

// ── Unknown Method Error ─────────────────────────────────────────────────

#[tokio::test]
async fn unknown_method_returns_error() {
    let server = CdpServer::start("", 0, false, None).unwrap();
    let mut client = CdpClient::connect(server.port()).await;

    let resp = client.send("FakeMethod.doesNotExist", json!({})).await;
    let error = resp.get("error");
    assert!(error.is_some(), "unknown method should return error");
    let code = error
        .and_then(|e| e.get("code"))
        .and_then(serde_json::Value::as_i64);
    assert_eq!(code, Some(-32601), "should return method not found error");
}

// ── Page.enable + DOM.enable ─────────────────────────────────────────────

#[tokio::test]
async fn page_enable_returns_ok() {
    let server = CdpServer::start("", 0, false, None).unwrap();
    let mut client = CdpClient::connect(server.port()).await;

    let resp = client.send("Page.enable", json!({})).await;
    assert!(resp.get("result").is_some(), "Page.enable should succeed");
}

#[tokio::test]
async fn network_enable_returns_ok() {
    let server = CdpServer::start("", 0, false, None).unwrap();
    let mut client = CdpClient::connect(server.port()).await;

    let resp = client.send("Network.enable", json!({})).await;
    assert!(
        resp.get("result").is_some(),
        "Network.enable should succeed"
    );
}

// ── Runtime.evaluate with Initial HTML ───────────────────────────────────

#[tokio::test]
async fn evaluate_after_initial_html() {
    let html = r#"<html><body><p>Hello</p></body></html>"#;
    let server = CdpServer::start(html, 0, false, None).unwrap();
    let mut client = CdpClient::connect(server.port()).await;

    // Use document.body.textContent since querySelector is not yet implemented
    let resp = client
        .send(
            "Runtime.evaluate",
            json!({"expression": "document.body.textContent.trim()", "returnByValue": true}),
        )
        .await;
    let value = resp
        .get("result")
        .and_then(|r| r.get("result"))
        .and_then(|r| r.get("value"))
        .and_then(serde_json::Value::as_str);
    assert_eq!(value, Some("Hello"), "should read DOM content");
}

// ── HTTP Endpoints ───────────────────────────────────────────────────────

#[tokio::test]
async fn http_hello_endpoint() {
    let server = CdpServer::start("", 0, false, None).unwrap();
    let port = server.port();

    let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .expect("connect");
    tokio::io::AsyncWriteExt::write_all(
        &mut stream,
        b"GET /hello HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    )
    .await
    .unwrap();

    let mut buf = vec![0u8; 4096];
    let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
        .await
        .unwrap();
    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(response.contains("200 OK"), "should return 200: {response}");
}

#[tokio::test]
async fn http_json_version_endpoint() {
    let server = CdpServer::start("", 0, false, None).unwrap();
    let port = server.port();

    let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .expect("connect");
    tokio::io::AsyncWriteExt::write_all(
        &mut stream,
        b"GET /json/version HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    )
    .await
    .unwrap();

    let mut buf = vec![0u8; 4096];
    let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
        .await
        .unwrap();
    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(response.contains("200 OK"), "should return 200: {response}");
}
