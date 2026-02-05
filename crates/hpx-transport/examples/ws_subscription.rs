//! WebSocket Subscription Example
//!
//! Demonstrates the subscription pattern with the WebSocket client.
//!
//! Run with: `cargo run -p hpx-transport --example ws_subscription`

use hpx_transport::websocket::{Topic, WsConfig};

fn main() {
    // Create instances to demonstrate API usage
    let _config = WsConfig::new("wss://api.exchange.com/ws");
    let _topic = Topic::new("orderbook.BTC");

    println!("WebSocket Subscription Example");
    println!("===============================");
    println!();
    println!("This example demonstrates the subscription pattern.");
    println!();
    println!("Usage:");
    println!("  1. Connect to a WebSocket server");
    println!("  2. Subscribe to one or more topics");
    println!("  3. Receive updates via the broadcast receiver");
    println!("  4. Unsubscribe when done");
    println!();
    println!("Example code:");
    println!();
    println!(
        r#"
    let config = WsConfig::new("wss://api.exchange.com/ws");
    let handler = GenericJsonHandler::new();
    let client = WsClient::connect(config, handler).await?;

    // Subscribe to a single topic
    let mut orderbook_rx = client.subscribe("orderbook.BTC").await?;

    // Subscribe to multiple topics
    let receivers = client.subscribe_many([
        "trades.BTC",
        "trades.ETH",
        "ticker.BTC",
    ]).await?;

    // Process updates
    loop {{
        tokio::select! {{
            Ok(msg) = orderbook_rx.recv() => {{
                println!("Orderbook update: {{:?}}", msg);
            }}
            // Handle other receivers...
        }}
    }}

    // Unsubscribe
    client.unsubscribe("orderbook.BTC").await?;

    // Topics that are still subscribed
    println!("Active topics: {{:?}}", client.subscribed_topics());
    "#
    );
}
