//! A WebSocket echo server implementation using yawc and hyper.
//! This server accepts WebSocket connections and echoes back any text or binary messages it receives.

use futures::{SinkExt, StreamExt};
use hpx_yawc::{OpCode, Options, WebSocket};
use http_body_util::Empty;
use hyper::{
    Request, Response,
    body::{Bytes, Incoming},
    server::conn::http1,
    service::service_fn,
};
use tokio::net::TcpListener;

/// Handles an individual WebSocket client connection by echoing back any received messages.
async fn handle_client(fut: hpx_yawc::UpgradeFut) -> hpx_yawc::Result<()> {
    let mut ws = fut.await?;

    while let Some(frame) = ws.next().await {
        match frame.opcode() {
            OpCode::Text | OpCode::Binary => {
                ws.send(frame).await?;
            }
            _ => {}
        }
    }

    Ok(())
}

async fn server_upgrade(mut req: Request<Incoming>) -> hpx_yawc::Result<Response<Empty<Bytes>>> {
    let (response, fut) = WebSocket::upgrade_with_options(
        &mut req,
        Options::default()
            .with_utf8()
            .with_backpressure_boundary(100 * 1024 * 1024)
            .with_max_payload_read(100 * 1024 * 1024)
            .with_max_read_buffer(200 * 1024 * 1024)
            .with_low_latency_compression(),
    )?;

    tokio::task::spawn(async move {
        if let Err(e) = handle_client(fut).await {
            tracing::error!("Error in websocket connection: {e}");
        }
    });

    Ok(response)
}

#[tokio::main]
async fn main() -> hpx_yawc::Result<()> {
    // console_subscriber::init();

    let listener = TcpListener::bind("0.0.0.0:9002").await?;
    tracing::debug!("Listening on {}", listener.local_addr().unwrap());

    loop {
        let (stream, _) = listener.accept().await?;
        let _ = stream.set_nodelay(true);

        tokio::spawn(async move {
            let io = hyper_util::rt::TokioIo::new(stream);
            let conn_fut = http1::Builder::new()
                .serve_connection(io, service_fn(server_upgrade))
                .with_upgrades();
            if let Err(e) = conn_fut.await {
                tracing::error!("An error occurred: {e:?}");
            }
        });
    }
}
