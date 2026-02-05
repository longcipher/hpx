//! WebSocket Request-Response Example
//!
//! Demonstrates the request-response pattern with the WebSocket client.
//!
//! Run with: `cargo run -p hpx-transport --example ws_request_response`

use hpx_transport::websocket::{MessageKind, ProtocolHandler, RequestId, Topic, WsMessage};
use serde::{Deserialize, Serialize};

/// Example request type.
#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct OrderBookRequest {
    method: String,
    symbol: String,
}

/// Example response type.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OrderBookResponse {
    symbol: String,
    bids: Vec<(String, String)>,
    asks: Vec<(String, String)>,
}

/// Custom handler for demonstration (echoes back with request_id).
struct DemoHandler;

impl ProtocolHandler for DemoHandler {
    fn classify_message(&self, message: &str) -> MessageKind {
        if message.contains("\"req_id\"") {
            MessageKind::Response
        } else {
            MessageKind::Unknown
        }
    }

    fn extract_request_id(&self, message: &str) -> Option<RequestId> {
        // Simple extraction for demo
        let json: serde_json::Value = serde_json::from_str(message).ok()?;
        let id = json.get("req_id")?.as_str()?;
        Some(RequestId::from(id.to_string()))
    }

    fn extract_topic(&self, _message: &str) -> Option<Topic> {
        None
    }

    fn build_subscribe(&self, topics: &[Topic], request_id: RequestId) -> WsMessage {
        let args: Vec<&str> = topics.iter().map(|t| t.as_str()).collect();
        WsMessage::text(
            serde_json::json!({
                "op": "subscribe",
                "req_id": request_id.as_str(),
                "args": args
            })
            .to_string(),
        )
    }

    fn build_unsubscribe(&self, topics: &[Topic], request_id: RequestId) -> WsMessage {
        let args: Vec<&str> = topics.iter().map(|t| t.as_str()).collect();
        WsMessage::text(
            serde_json::json!({
                "op": "unsubscribe",
                "req_id": request_id.as_str(),
                "args": args
            })
            .to_string(),
        )
    }
}

fn main() {
    // Instantiate handler to prove it compiles
    let _handler = DemoHandler;

    // This is a demonstration that shows the API usage.
    // In a real application, you would connect to an actual WebSocket server.
    println!("WebSocket Request-Response Example");
    println!("===================================");
    println!();
    println!("This example demonstrates the request-response pattern.");
    println!();
    println!("Usage:");
    println!("  1. Create a WsConfig with your WebSocket URL");
    println!("  2. Implement ProtocolHandler for your exchange");
    println!("  3. Connect using WsClient::connect()");
    println!("  4. Use client.request() for typed request/response");
    println!();
    println!("Example code:");
    println!();
    println!(
        r##"
    let config = WsConfig::new("wss://api.exchange.com/ws")
        .request_timeout(Duration::from_secs(10));

    let handler = GenericJsonHandler::new();
    let client = WsClient::connect(config, handler).await?;

    // Typed request
    let response: MyResponse = client.request(&MyRequest {{
        method: "getOrderBook".into(),
        symbol: "BTCUSD".into(),
    }}).await?;

    // Request with custom timeout
    let response: Value = client.request_with_timeout(
        &request,
        Some(Duration::from_secs(5))
    ).await?;

    // Raw request
    let raw_response = client.request_raw(
        WsMessage::text(r#"{{"action":"ping"}}"#)
    ).await?;
    "##
    );
}
