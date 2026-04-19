#![cfg(all(feature = "ws-yawc", not(feature = "ws-fastwebsockets")))]

mod support;

use hpx::{BrowserProfile, Client};
use support::server;
use tokio::io::AsyncWriteExt;

async fn reply_to_ws_upgrade(stream: &mut tokio::net::TcpStream) {
    let response = "\
        HTTP/1.1 101 Switching Protocols\r\n\
        Connection: upgrade\r\n\
        Upgrade: websocket\r\n\
        \r\n";

    stream.write_all(response.as_bytes()).await.unwrap();
}

#[tokio::test]
async fn unsupported_yawc_builder_options_fail_explicitly() {
    let server = server::low_level_with_response(|_request, stream| {
        Box::new(async move {
            reply_to_ws_upgrade(stream).await;
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
            reply_to_ws_upgrade(stream).await;
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
