use std::net::SocketAddr;

use axum::{Router, response::IntoResponse, routing::get};
use hpx_yawc::{Frame, HttpStream, IncomingUpgrade, Options, WebSocket, frame::OpCode};
use tokio::net::TcpListener;

async fn ws_handler(ws: IncomingUpgrade) -> impl IntoResponse {
    // Configure options based on what Autobahn tests need
    let options = get_server_options();

    let (response, fut) = ws.upgrade(options).unwrap();

    tokio::spawn(async move {
        if let Ok(websocket) = fut.await {
            handle_socket(websocket).await;
        }
    });

    response
}

fn get_server_options() -> Options {
    Options::default()
        .with_utf8()
        .with_max_payload_read(100 * 1024 * 1024)
        .with_max_read_buffer(200 * 1024 * 1024)
        .with_low_latency_compression()
}

async fn handle_socket(mut ws: WebSocket<HttpStream>) {
    use futures::{SinkExt, StreamExt};

    while let Some(msg) = ws.next().await {
        let (opcode, _is_fin, body) = msg.into_parts();

        // Echo back the message
        match opcode {
            OpCode::Text | OpCode::Binary => {
                if let Err(e) = ws.send(Frame::from((opcode, body))).await {
                    tracing::error!("Error sending message: {}", e);
                    break;
                }
            }
            _ => {}
        }
    }

    tracing::debug!("WebSocket connection closed");
}

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt::init();

    let app = Router::new().route("/", get(ws_handler));

    let addr = SocketAddr::from(([127, 0, 0, 1], 9002));
    tracing::info!("Autobahn test server listening on {}", addr);

    let listener = TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
