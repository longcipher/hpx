#![allow(missing_docs)]
#![cfg(feature = "ws-yawc")]
#![allow(
    clippy::allow_attributes,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used
)]

mod support;

use hpx::{BrowserProfile, Client};
use sha1::Digest as _;
use support::server;
use tokio::io::AsyncWriteExt;

/// RFC 6455 §4.2.2: a 101 response MUST echo `Sec-WebSocket-Accept`
/// computed as `base64(SHA1(client_key + magic GUID))`; the yawc client
/// validates it during the handshake.
fn websocket_accept_for(request_text: &str) -> String {
    let key = request_text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("sec-websocket-key")
                .then(|| value.trim().to_string())
        })
        .expect("upgrade request should carry sec-websocket-key");

    let mut sha1 = sha1::Sha1::new();
    sha1.update(key.as_bytes());
    sha1.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11"); // magic GUID
    base64_simd::STANDARD.encode_to_string(&sha1.finalize()[..])
}

async fn reply_to_ws_upgrade(stream: &mut tokio::net::TcpStream, request_text: &str) {
    let accept = websocket_accept_for(request_text);
    let response = format!(
        "\
        HTTP/1.1 101 Switching Protocols\r\n\
        Connection: upgrade\r\n\
        Upgrade: websocket\r\n\
        Sec-WebSocket-Accept: {accept}\r\n\
        \r\n"
    );

    stream.write_all(response.as_bytes()).await.unwrap();
}

#[tokio::test]
async fn unsupported_yawc_builder_options_fail_explicitly() {
    let server = server::low_level_with_response(|request, stream| {
        let request_text = std::str::from_utf8(request).expect("request should be valid utf-8");
        let accept = websocket_accept_for(request_text);
        Box::new(async move {
            // The client rejects the unsupported builder options before the
            // handshake completes, so the response body is never read; still
            // answer with a well-formed 101 for realism.
            let response = format!(
                "\
                HTTP/1.1 101 Switching Protocols\r\n\
                Connection: upgrade\r\n\
                Upgrade: websocket\r\n\
                Sec-WebSocket-Accept: {accept}\r\n\
                \r\n"
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        })
    });

    let result = Client::new()
        .websocket(format!("ws://{}", server.addr()))
        .accept_key("client-supplied-key")
        .emulation(BrowserProfile::Firefox)
        .send()
        .await;

    let err = result.expect_err("unsupported yawc options should not be accepted");
    let message = err.to_string();

    assert!(err.is_upgrade());
    assert!(message.contains("accept_key"));
    assert!(message.contains("emulation"));
}

/// RFC 6455 §4.1: the client offers subprotocols in the
/// `Sec-WebSocket-Protocol` request header, the server selects one of them
/// and echoes it back, and the client exposes the negotiated value.
#[tokio::test]
async fn yawc_negotiates_subprotocol() {
    let server = server::low_level_with_response(|request, stream| {
        let request_text = std::str::from_utf8(request).expect("request should be valid utf-8");
        assert!(
            request_text.contains("sec-websocket-protocol: chat, superchat"),
            "the offered subprotocols must be sent in preference order: {request_text}"
        );

        let accept = websocket_accept_for(request_text);
        Box::new(async move {
            let response = format!(
                "\
                HTTP/1.1 101 Switching Protocols\r\n\
                Connection: upgrade\r\n\
                Upgrade: websocket\r\n\
                Sec-WebSocket-Accept: {accept}\r\n\
                Sec-WebSocket-Protocol: superchat\r\n\
                \r\n"
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        })
    });

    let response = Client::new()
        .websocket(format!("ws://{}", server.addr()))
        .protocols(["chat", "superchat"])
        .send()
        .await
        .expect("websocket handshake should succeed");

    assert_eq!(response.protocol(), Some("superchat"));

    let ws = response
        .into_websocket()
        .await
        .expect("websocket should upgrade cleanly");

    assert_eq!(ws.protocol(), Some("superchat"));
}

/// A server that selects a subprotocol the client did not offer must fail
/// the connection (RFC 6455 §4.1).
#[tokio::test]
async fn yawc_rejects_unoffered_subprotocol() {
    let server = server::low_level_with_response(|request, stream| {
        let request_text = std::str::from_utf8(request).expect("request should be valid utf-8");
        let accept = websocket_accept_for(request_text);
        Box::new(async move {
            let response = format!(
                "\
                HTTP/1.1 101 Switching Protocols\r\n\
                Connection: upgrade\r\n\
                Upgrade: websocket\r\n\
                Sec-WebSocket-Accept: {accept}\r\n\
                Sec-WebSocket-Protocol: not-offered\r\n\
                \r\n"
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        })
    });

    let err = Client::new()
        .websocket(format!("ws://{}", server.addr()))
        .protocols(["chat"])
        .send()
        .await
        .expect_err("an unoffered subprotocol must fail the handshake");

    assert!(
        err.to_string().contains("Sec-WebSocket-Protocol"),
        "unexpected error: {err}"
    );
}

/// A WebSocket connection can be established through an HTTP proxy with a
/// CONNECT tunnel.
#[tokio::test]
async fn yawc_websocket_through_http_proxy() {
    let server = server::low_level_with_response(|request, stream| {
        let request_text = std::str::from_utf8(request).expect("request should be valid utf-8");
        Box::new(async move {
            if request_text.starts_with("CONNECT ") {
                assert!(
                    request_text.contains("CONNECT hyper.rs.local:80"),
                    "unexpected CONNECT request: {request_text}"
                );
                stream
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .await
                    .unwrap();
            } else {
                let accept = websocket_accept_for(request_text);
                let response = format!(
                    "\
                    HTTP/1.1 101 Switching Protocols\r\n\
                    Connection: upgrade\r\n\
                    Upgrade: websocket\r\n\
                    Sec-WebSocket-Accept: {accept}\r\n\
                    \r\n"
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        })
    });

    let proxy = format!("http://{}", server.addr());

    let response = Client::new()
        .websocket("ws://hyper.rs.local/")
        .proxy(hpx::Proxy::http(&proxy).unwrap())
        .send()
        .await
        .expect("websocket handshake through the proxy should succeed");

    let _ws = response
        .into_websocket()
        .await
        .expect("websocket should upgrade cleanly");
}

#[tokio::test]
async fn yawc_send_preserves_request_headers() {
    let server = server::low_level_with_response(|request, stream| {
        let request_text = std::str::from_utf8(request).expect("request should be valid utf-8");
        assert!(request_text.contains("authorization: Bearer secret-token"));
        assert!(request_text.contains("x-test-header: present"));

        Box::new(async move {
            reply_to_ws_upgrade(stream, request_text).await;
        })
    });

    let response = Client::new()
        .websocket(format!("ws://{}", server.addr()))
        .bearer_auth("secret-token")
        .header("x-test-header", "present")
        .send()
        .await
        .expect("websocket handshake should succeed");

    let _ws = response
        .into_websocket()
        .await
        .expect("websocket should upgrade cleanly");
}
