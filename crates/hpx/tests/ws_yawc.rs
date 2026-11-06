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
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

/// Encodes one unmasked server-to-client frame (RFC 6455 §5.2).
fn server_frame(fin: bool, opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![if fin { 0x80 | opcode } else { opcode }];
    match payload.len() {
        len @ 0..=125 => frame.push(len as u8),
        len @ 126..=0xFFFF => {
            frame.push(126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        }
        len => {
            frame.push(127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(payload);
    frame
}

/// Starts a server that completes the WebSocket upgrade, writes `frames`
/// verbatim and, when `eof` is set, shuts down its write half. It then
/// drains the connection until the client goes away, so the support server
/// never parses the client's frames as a new HTTP request.
fn raw_ws_server(frames: Vec<u8>, eof: bool) -> server::Server {
    server::low_level_with_response(move |request, stream| {
        let request_text = std::str::from_utf8(request).expect("request should be valid utf-8");
        let frames = frames.clone();
        Box::new(async move {
            reply_to_ws_upgrade(stream, request_text).await;
            stream.write_all(&frames).await.unwrap();
            if eof {
                stream.shutdown().await.unwrap();
            }
            let mut drained = Vec::new();
            let _ = stream.read_to_end(&mut drained).await;
        })
    })
}

async fn connect(server: &server::Server) -> hpx::ws::WebSocket {
    Client::new()
        .websocket(format!("ws://{}", server.addr()))
        .send()
        .await
        .expect("websocket handshake should succeed")
        .into_websocket()
        .await
        .expect("websocket should upgrade cleanly")
}

/// Returns the yawc error behind a failed WebSocket read.
fn yawc_error(err: &hpx::Error) -> &hpx_yawc::WebSocketError {
    assert!(err.is_upgrade(), "unexpected error kind: {err:?}");
    std::error::Error::source(err)
        .and_then(|source| source.downcast_ref::<hpx_yawc::WebSocketError>())
        .unwrap_or_else(|| panic!("expected a yawc error source: {err:?}"))
}

/// RFC 6455 §8.1: a text message that is not valid UTF-8 fails the read; it
/// must not reach the caller as a lossily repaired string.
#[tokio::test]
async fn yawc_recv_rejects_invalid_utf8_text() {
    // 0xC3 0x28 is an invalid two-byte sequence.
    let server = raw_ws_server(server_frame(true, 0x1, b"\xC3\x28"), false);
    let mut ws = connect(&server).await;

    let read = ws.recv().await;
    let Some(Err(err)) = &read else {
        panic!("invalid UTF-8 must fail the read, got {read:?}");
    };
    assert!(
        matches!(yawc_error(err), hpx_yawc::WebSocketError::InvalidUTF8),
        "{err:?}"
    );
}

/// A read error reaches the caller as `Some(Err(_))` on every read path.
/// yawc's own `Stream` impl reports it as `None`, which reads as a clean end
/// of stream.
#[tokio::test]
async fn yawc_read_paths_surface_protocol_errors() {
    use futures_util::StreamExt as _;

    // Opcode 0x3 is reserved (RFC 6455 §5.2).
    let server = raw_ws_server(server_frame(true, 0x3, b""), false);

    let mut ws = connect(&server).await;
    let unsplit = ws.recv().await;

    let (_write, mut read) = connect(&server).await.split();
    let split = read.recv().await;

    let (_write, mut read) = connect(&server).await.split();
    let stream = read.next().await;

    for (path, read) in [
        ("WebSocket::recv", unsplit),
        ("WebSocketRead::recv", split),
        ("Stream for WebSocketRead", stream),
    ] {
        let Some(Err(err)) = &read else {
            panic!("{path}: a read error must not look like the end of the stream, got {read:?}");
        };
        assert!(
            matches!(yawc_error(err), hpx_yawc::WebSocketError::InvalidOpCode(3)),
            "{path}: {err:?}"
        );
    }
}

/// A clean end of stream still reads as `None`: after the peer's close
/// frame, and at EOF.
#[tokio::test]
async fn yawc_recv_returns_none_after_close_and_eof() {
    let close_frame = server_frame(true, 0x8, &1000u16.to_be_bytes());
    let server = raw_ws_server(close_frame, false);
    let mut ws = connect(&server).await;

    let message = ws
        .recv()
        .await
        .expect("close frame expected")
        .expect("a close frame is not an error");
    let hpx::ws::Message::Close(Some(close)) = &message else {
        panic!("expected a close frame, got {message:?}");
    };
    assert_eq!(close.code, hpx::ws::message::CloseCode::NORMAL);
    let next = ws.recv().await;
    assert!(
        next.is_none(),
        "the stream must end after the close frame: {next:?}"
    );

    let server = raw_ws_server(Vec::new(), true);
    let mut ws = connect(&server).await;
    let next = ws.recv().await;
    assert!(next.is_none(), "EOF must end the stream: {next:?}");
}

/// `max_message_size` caps whole messages, not single frames: a message
/// reassembled from fragments that each fit the limit is still rejected
/// once its total exceeds it.
#[tokio::test]
async fn yawc_max_message_size_caps_fragmented_messages() {
    const LIMIT: usize = 64 * 1024;

    // Two binary messages, each sent as two fragments under LIMIT: the
    // first totals 48 KiB, the second 120 KiB.
    let mut frames = Vec::new();
    for half in [24 * 1024, 60 * 1024] {
        frames.extend(server_frame(false, 0x2, &b"a".repeat(half)));
        frames.extend(server_frame(true, 0x0, &b"b".repeat(half)));
    }
    let server = raw_ws_server(frames, false);

    let mut ws = Client::new()
        .websocket(format!("ws://{}", server.addr()))
        .max_message_size(LIMIT)
        .send()
        .await
        .expect("websocket handshake should succeed")
        .into_websocket()
        .await
        .expect("websocket should upgrade cleanly");

    let message = ws
        .recv()
        .await
        .expect("message expected")
        .expect("a 48 KiB message fits the 64 KiB limit");
    assert_eq!(message.into_data().len(), 48 * 1024);

    let read = ws.recv().await;
    let Some(Err(err)) = &read else {
        panic!("a 120 KiB message must exceed the 64 KiB limit, got {read:?}");
    };
    assert!(
        matches!(yawc_error(err), hpx_yawc::WebSocketError::FrameTooLarge),
        "{err:?}"
    );
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
