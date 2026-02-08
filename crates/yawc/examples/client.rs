/// Example WebSocket client that connects to Bybit's public trade stream
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use hpx_yawc::{Frame, OpCode, Options, WebSocket};
use tokio::time::interval;

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Connect to the WebSocket server with fast compression enabled
    let mut client = WebSocket::connect("wss://stream.bybit.com/v5/public/linear".parse().unwrap())
        .with_options(Options::default().with_high_compression())
        .await
        .expect("connection");

    // JSON-formatted subscription request
    let text = r#"{
        "req_id": "1",
        "op": "subscribe",
        "args": [
            "publicTrade.BTCUSDT"
        ]
    }"#;

    // Send subscription request
    let _ = client.send(Frame::text(text)).await;

    // Set up an interval to send pings every 3 seconds
    let mut ival = interval(Duration::from_secs(3));

    loop {
        tokio::select! {
            // Send a ping on each tick
            _ = ival.tick() => {
                tracing::debug!("Tick");
                let _ = client.send(Frame::ping("idk")).await;
            }
            // Handle incoming frames
            frame = client.next() => {
                if frame.is_none() {
                    tracing::debug!("Disconnected");
                    break;
                }

                let frame = frame.unwrap();
                let (opcode, _is_fin, body) = frame.into_parts();
                match opcode {
                    OpCode::Text => {
                        let text = std::str::from_utf8(&body).expect("utf8");
                        tracing::info!("{text}");
                        let _: serde_json::Value = serde_json::from_str(text).expect("serde");
                    }
                    OpCode::Pong => {
                        let data = std::str::from_utf8(&body).unwrap();
                        tracing::debug!("Pong: {data}");
                    }
                    _ => {}
                }
            }
        }
    }
}
