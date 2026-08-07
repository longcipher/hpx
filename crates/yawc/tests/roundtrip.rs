//! End-to-end WebSocket echo roundtrip exercising the hyper-backed
//! `HttpStream` read/write paths on both the server and client side.
//!
//! Without this test the `AsyncRead`/`AsyncWrite` implementations of
//! `HttpStream` are never exercised, so mutations that return fixed
//! `Poll::Ready` values (e.g. writing zero bytes) would go undetected.

use futures::{SinkExt, StreamExt};
use hpx_yawc::{Frame, OpCode, Options, UpgradeFut, WebSocket};
use hyper::{Request, body::Incoming, server::conn::http1, service::service_fn};
use tokio::net::TcpListener;

async fn echo_connection(fut: UpgradeFut) -> hpx_yawc::Result<()> {
    let mut ws = fut.await?;
    while let Some(frame) = ws.next().await {
        if matches!(frame.opcode(), OpCode::Text | OpCode::Binary) {
            ws.send(frame).await?;
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_echo_roundtrip_via_httpstream() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let _ = stream.set_nodelay(true);
            let io = hyper_util::rt::TokioIo::new(stream);
            tokio::spawn(async move {
                let service = service_fn(|mut req: Request<Incoming>| async move {
                    let (response, fut) =
                        WebSocket::upgrade_with_options(&mut req, Options::default())?;
                    tokio::spawn(async move {
                        let _ = echo_connection(fut).await;
                    });
                    Ok::<_, hpx_yawc::WebSocketError>(response)
                });
                let connection = http1::Builder::new()
                    .serve_connection(io, service)
                    .with_upgrades();
                if let Err(e) = connection.await {
                    eprintln!("websocket server error: {e}");
                }
            });
        }
    });

    let mut client = WebSocket::connect(format!("ws://{addr}/").parse().unwrap())
        .await
        .unwrap();

    let payload = b"hello httpstream roundtrip".to_vec();
    client.send(Frame::binary(payload.clone())).await.unwrap();
    let echo = tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
        .await
        .expect("echo frame timed out")
        .expect("echo frame");
    assert_eq!(echo.opcode(), OpCode::Binary);
    assert_eq!(echo.payload(), payload.as_slice());

    let _ = client.close().await;
    server.abort();
}
