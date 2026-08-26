#![allow(missing_docs)]
#![cfg(feature = "ws-yawc")]

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
        .force_http2()
        .protocols(["chat"])
        .emulation(BrowserProfile::Firefox)
        .send()
        .await;

    let err = result.expect_err("unsupported yawc options should not be accepted");
    let message = err.to_string();

    assert!(err.is_upgrade());
    assert!(message.contains("accept_key"));
    assert!(message.contains("force_http2"));
    assert!(message.contains("protocols"));
    assert!(message.contains("emulation"));
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
