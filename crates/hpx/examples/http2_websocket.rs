//! Run websocket server
//!
//! ```not_rust
//! git clone https://github.com/tokio-rs/axum && cd axum
//! cargo run -p example-websockets-http2
//! ```

use futures_util::TryStreamExt;
use hpx::{header, ws::message::Message};

#[tokio::main]
async fn main() -> hpx::Result<()> {
    let resp = hpx::websocket("wss://127.0.0.1:3000/ws")
        .header(header::USER_AGENT, env!("CARGO_PKG_NAME"))
        .send()
        .await?;

    let websocket = resp.into_websocket().await?;

    let (mut tx, mut rx) = websocket.split();

    tokio::spawn(async move {
        for i in 1..11 {
            if let Err(err) = tx.send(Message::text(format!("Hello, World! #{i}"))).await {
                eprintln!("failed to send message: {err}");
            }
        }
    });

    while let Some(message) = rx.try_next().await? {
        if let Message::Text(text) = message {
            println!("received: {text}");
        }
    }

    Ok(())
}
