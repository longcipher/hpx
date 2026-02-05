//! Custom Protocol Handler Example
//!
//! Demonstrates how to implement a custom ProtocolHandler for a specific exchange.
//!
//! Run with: `cargo run -p hpx-transport --example ws_custom_handler`

use hpx_transport::websocket::{MessageKind, ProtocolHandler, RequestId, Topic, WsMessage};

/// Example protocol handler for a hypothetical exchange.
///
/// This handler demonstrates all the methods you might need to implement
/// for a real exchange integration.
struct HypotheticalExchangeHandler {
    /// API key for authentication.
    api_key: String,
    /// API secret for signing.
    #[allow(dead_code)]
    api_secret: String,
}

impl HypotheticalExchangeHandler {
    fn new(api_key: String, api_secret: String) -> Self {
        Self {
            api_key,
            api_secret,
        }
    }

    fn parse_json(&self, message: &str) -> Option<serde_json::Value> {
        serde_json::from_str(message).ok()
    }
}

impl ProtocolHandler for HypotheticalExchangeHandler {
    // =========================================================================
    // Connection Lifecycle
    // =========================================================================

    fn on_connect(&self) -> Vec<WsMessage> {
        // Some exchanges require an initial identification message
        vec![WsMessage::text(
            serde_json::json!({
                "event": "connect",
                "api_key": self.api_key
            })
            .to_string(),
        )]
    }

    fn build_auth_message(&self) -> Option<WsMessage> {
        // Build HMAC signature (simplified)
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Some(WsMessage::text(
            serde_json::json!({
                "event": "auth",
                "api_key": self.api_key,
                "timestamp": timestamp,
                "signature": "hmac_signature_here"
            })
            .to_string(),
        ))
    }

    fn is_auth_success(&self, message: &str) -> bool {
        let json = self.parse_json(message);
        json.and_then(|j| j.get("event")?.as_str().map(|s| s == "auth_success"))
            .unwrap_or(false)
    }

    fn is_auth_failure(&self, message: &str) -> bool {
        let json = self.parse_json(message);
        json.and_then(|j| j.get("event")?.as_str().map(|s| s == "auth_failed"))
            .unwrap_or(false)
    }

    // =========================================================================
    // Message Classification
    // =========================================================================

    fn classify_message(&self, message: &str) -> MessageKind {
        let Some(json) = self.parse_json(message) else {
            return MessageKind::Unknown;
        };

        // Check event type
        if let Some(event) = json.get("event").and_then(|v| v.as_str()) {
            match event {
                "response" => return MessageKind::Response,
                "update" | "data" => return MessageKind::Update,
                "ping" | "pong" => return MessageKind::System,
                "subscribed" | "unsubscribed" => return MessageKind::Control,
                _ => {}
            }
        }

        // Fallback: has request_id = Response
        if json.get("request_id").is_some() {
            return MessageKind::Response;
        }

        // Fallback: has channel = Update
        if json.get("channel").is_some() {
            return MessageKind::Update;
        }

        MessageKind::Unknown
    }

    fn extract_request_id(&self, message: &str) -> Option<RequestId> {
        let json = self.parse_json(message)?;
        let id = json.get("request_id")?;

        match id {
            serde_json::Value::String(s) => Some(RequestId::from(s.clone())),
            serde_json::Value::Number(n) => Some(RequestId::from(n.to_string())),
            _ => None,
        }
    }

    fn extract_topic(&self, message: &str) -> Option<Topic> {
        let json = self.parse_json(message)?;
        let channel = json.get("channel")?.as_str()?;
        Some(Topic::new(channel))
    }

    // =========================================================================
    // Message Building
    // =========================================================================

    fn build_subscribe(&self, topics: &[Topic], request_id: RequestId) -> WsMessage {
        let channels: Vec<&str> = topics.iter().map(|t| t.as_str()).collect();

        WsMessage::text(
            serde_json::json!({
                "event": "subscribe",
                "request_id": request_id.as_str(),
                "channels": channels
            })
            .to_string(),
        )
    }

    fn build_unsubscribe(&self, topics: &[Topic], request_id: RequestId) -> WsMessage {
        let channels: Vec<&str> = topics.iter().map(|t| t.as_str()).collect();

        WsMessage::text(
            serde_json::json!({
                "event": "unsubscribe",
                "request_id": request_id.as_str(),
                "channels": channels
            })
            .to_string(),
        )
    }

    fn build_ping(&self) -> Option<WsMessage> {
        // This exchange uses application-level ping
        Some(WsMessage::text(
            serde_json::json!({"event": "ping"}).to_string(),
        ))
    }

    fn build_pong(&self, _ping_data: &[u8]) -> Option<WsMessage> {
        Some(WsMessage::text(
            serde_json::json!({"event": "pong"}).to_string(),
        ))
    }

    // =========================================================================
    // Message Processing
    // =========================================================================

    fn is_server_ping(&self, message: &str) -> bool {
        self.parse_json(message)
            .and_then(|j| j.get("event")?.as_str().map(|s| s == "ping"))
            .unwrap_or(false)
    }

    fn is_pong_response(&self, message: &str) -> bool {
        self.parse_json(message)
            .and_then(|j| j.get("event")?.as_str().map(|s| s == "pong"))
            .unwrap_or(false)
    }

    fn should_reconnect(&self, message: &str) -> bool {
        // Some exchanges send explicit reconnect requests
        self.parse_json(message)
            .and_then(|j| j.get("event")?.as_str().map(|s| s == "reconnect"))
            .unwrap_or(false)
    }
}

fn main() {
    println!("Custom Protocol Handler Example");
    println!("================================");
    println!();
    println!("This example shows how to implement ProtocolHandler");
    println!("for a specific exchange's WebSocket protocol.");
    println!();
    println!("Key methods to implement:");
    println!("  - classify_message(): Route messages correctly");
    println!("  - extract_request_id(): Correlate responses");
    println!("  - extract_topic(): Route updates to subscribers");
    println!("  - build_subscribe/unsubscribe(): Format protocol messages");
    println!();
    println!("Optional methods:");
    println!("  - on_connect(): Initial handshake messages");
    println!("  - build_auth_message(): Authentication");
    println!("  - build_ping/pong(): Application-level heartbeat");
    println!("  - decode_binary(): Decompress or decode binary data");
    println!();

    // Create handler instance for demonstration
    let handler =
        HypotheticalExchangeHandler::new("my-api-key".to_string(), "my-api-secret".to_string());

    // Test classification
    let test_cases = [
        (
            r#"{"event":"response","request_id":"1","data":{}}"#,
            "Response",
        ),
        (
            r#"{"event":"update","channel":"trades.BTC","data":[]}"#,
            "Update",
        ),
        (r#"{"event":"ping"}"#, "System"),
        (
            r#"{"event":"subscribed","channel":"trades.BTC"}"#,
            "Control",
        ),
        (r#"{"unknown":"data"}"#, "Unknown"),
    ];

    println!("Message classification examples:");
    for (msg, expected) in test_cases {
        let kind = handler.classify_message(msg);
        println!("  {expected:?} -> {kind:?}");
    }
}
