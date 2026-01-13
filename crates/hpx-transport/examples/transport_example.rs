//! Example demonstrating the new longtrader-transport architecture
//!
//! This example shows how to use both HTTP and WebSocket clients
//! with authentication, middleware, and error handling.

use std::time::Duration;

use hpx::ws::message::Message;
use hpx_transport::{
    Request,
    auth::{ApiKeyAuth, BearerAuth, NoAuth},
    error::TransportResult,
    http::{HttpClient, HttpConfig},
    websocket::{ExchangeHandler, WebSocketClient, WebSocketConfig},
};
use serde_json::json;

#[tokio::main]
async fn main() -> TransportResult<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    println!("🚀 LongTrader Transport Examples");

    // Example 1: HTTP Client with API Key Authentication
    println!("\n📡 Example 1: HTTP Client with API Key Auth");
    http_example().await?;

    // Example 2: HTTP Client with Bearer Token Authentication
    println!("\n🔐 Example 2: HTTP Client with Bearer Token Auth");
    http_bearer_example().await?;

    // Example 3: HTTP Client with Middleware
    println!("\n⚙️  Example 3: HTTP Client with Middleware");
    http_middleware_example().await?;

    // Example 4: WebSocket Client
    println!("\n🌐 Example 4: WebSocket Client");
    websocket_example().await?;

    // Example 5: WebSocket Client with Auth
    println!("\n🔐 Example 5: WebSocket Client with Auth");
    ws_auth_example().await?;

    println!("\n✅ All examples completed successfully!");
    Ok(())
}

/// Example using HTTP client with API key authentication
async fn http_example() -> TransportResult<()> {
    let config = HttpConfig::builder("https://httpbin.org")
        .timeout(Duration::from_secs(10))
        .user_agent("longtrader-transport-example/1.0")
        .build()?;

    let auth = ApiKeyAuth::header("X-API-Key", "demo-api-key");
    let client = HttpClient::new(config, auth)?;

    // Build a GET request
    let request = Request::get("https://httpbin.org/get?symbol=BTCUSDT");

    println!("  📤 Sending GET request to /get");

    match client.send_raw(request).await {
        Ok(response) => {
            println!("  ✅ Response status: {}", response.status);
            println!("  📊 Response size: {} bytes", response.body.len());

            // Parse JSON response
            if let Ok(json_value) = serde_json::from_slice::<serde_json::Value>(&response.body) {
                if let Some(args) = json_value.get("args") {
                    println!("  🔍 Query parameters: {}", args);
                }
                if let Some(headers) = json_value.get("headers")
                    && let Some(api_key) = headers.get("X-Api-Key")
                {
                    println!("  🔑 API Key header received: {}", api_key);
                }
            }
        }
        Err(e) => {
            println!("  ❌ Request failed: {}", e);
        }
    }

    Ok(())
}

/// Example using HTTP client with Bearer token authentication
async fn http_bearer_example() -> TransportResult<()> {
    let config = HttpConfig::builder("https://httpbin.org")
        .timeout(Duration::from_secs(10))
        .build()?;

    let auth = BearerAuth::new("demo-bearer-token");
    let client = HttpClient::new(config, auth)?;

    // Build a POST request with JSON body
    let body = json!({
        "symbol": "ETHUSDT",
        "side": "buy",
        "quantity": 0.1
    });

    let request = Request::post("https://httpbin.org/post")
        .header("Content-Type", "application/json")
        .body(serde_json::to_vec(&body)?);

    println!("  📤 Sending POST request with JSON body");

    match client.send_raw(request).await {
        Ok(response) => {
            println!("  ✅ Response status: {}", response.status);

            if let Ok(json_value) = serde_json::from_slice::<serde_json::Value>(&response.body) {
                if let Some(headers) = json_value.get("headers")
                    && let Some(auth_header) = headers.get("Authorization")
                {
                    println!("  🔐 Authorization header: {}", auth_header);
                }
                if let Some(json_data) = json_value.get("json") {
                    println!("  📦 Received JSON: {}", json_data);
                }
            }
        }
        Err(e) => {
            println!("  ❌ Request failed: {}", e);
        }
    }

    Ok(())
}

/// Example using HTTP client with middleware stack
async fn http_middleware_example() -> TransportResult<()> {
    let config = HttpConfig::builder("https://httpbin.org")
        .timeout(Duration::from_secs(5))
        .build()?;

    let auth = NoAuth;
    let client = HttpClient::new(config, auth)?;

    println!("  📤 Sending request with middleware (timeout + retry + metrics)");

    // Try to access a slow endpoint
    let request = Request::get("https://httpbin.org/delay/2"); // 2 second delay

    match client.send_raw(request).await {
        Ok(response) => {
            println!("  ✅ Response received: {}", response.status);
        }
        Err(e) => {
            println!("  ⚠️  Request failed (expected with short timeout): {}", e);
        }
    }

    // Show client metrics
    let metrics = client.metrics();
    println!("  📊 Client metrics:");
    println!("    - Success rate: {:.1}%", metrics.success_rate() * 100.0);

    Ok(())
}

// A simple handler for the example
struct EchoHandler;

impl ExchangeHandler for EchoHandler {
    fn get_topic(&self, message: &str) -> Option<String> {
        // For echo server, we just assume everything is "echo" topic for simplicity
        // or try to parse if it's JSON
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(message)
            && let Some(topic) = json.get("topic").and_then(|s| s.as_str())
        {
            return Some(topic.to_string());
        }
        Some("echo".to_string())
    }

    fn build_subscribe_message(&self, topic: &str) -> Message {
        Message::Text(
            json!({ "op": "subscribe", "topic": topic })
                .to_string()
                .into(),
        )
    }

    fn build_unsubscribe_message(&self, topic: &str) -> Message {
        Message::Text(
            json!({ "op": "unsubscribe", "topic": topic })
                .to_string()
                .into(),
        )
    }
}

/// Example using WebSocket client
async fn websocket_example() -> TransportResult<()> {
    // Note: Using a public echo WebSocket for demo
    let config = WebSocketConfig::new("wss://echo.websocket.org");

    let handler = EchoHandler;
    let client = WebSocketClient::new(config, handler).await?;

    println!("  🔌 Connecting to WebSocket...");
    // Connection is automatic in background

    // Subscribe to a topic
    println!("  📤 Subscribing to 'echo' topic...");
    let mut rx = client.subscribe("echo").await?;

    // Send a text message
    println!("  📤 Sending text message");
    // Echo server will bounce this back. Our handler will route it to "echo" topic.
    client
        .send_json(&json!({ "topic": "echo", "msg": "Hello from longtrader!" }))
        .await?;

    // Receive message
    if let Ok(msg) = rx.recv().await {
        println!("  📩 Received: {:?}", msg);
    }

    // Wait a bit and then disconnect
    tokio::time::sleep(Duration::from_millis(500)).await;

    println!("  🔌 Disconnecting...");
    client.close().await?;

    Ok(())
}

struct AuthEchoHandler {
    api_key: String,
    api_secret: String,
}

impl ExchangeHandler for AuthEchoHandler {
    fn get_topic(&self, message: &str) -> Option<String> {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(message)
            && let Some(topic) = json.get("topic").and_then(|s| s.as_str())
        {
            return Some(topic.to_string());
        }
        Some("auth".to_string())
    }

    fn build_subscribe_message(&self, topic: &str) -> Message {
        Message::Text(
            json!({ "op": "subscribe", "topic": topic })
                .to_string()
                .into(),
        )
    }

    fn build_unsubscribe_message(&self, topic: &str) -> Message {
        Message::Text(
            json!({ "op": "unsubscribe", "topic": topic })
                .to_string()
                .into(),
        )
    }

    fn on_open(&self) -> Vec<Message> {
        use hex;
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();

        let payload = format!("{}{}", timestamp, "GET/users/self/verify");

        let mut mac = Hmac::<Sha256>::new_from_slice(self.api_secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(payload.as_bytes());
        let signature = hex::encode(mac.finalize().into_bytes());

        let auth_msg = json!({
            "op": "auth",
            "args": {
                "apiKey": self.api_key,
                "timestamp": timestamp,
                "signature": signature
            }
        });

        println!("  🔑 Sending auth message: {}", auth_msg);
        vec![Message::Text(auth_msg.to_string().into())]
    }
}

/// Example using WebSocket client with Authentication
async fn ws_auth_example() -> TransportResult<()> {
    let config = WebSocketConfig::new("wss://echo.websocket.org");

    let handler = AuthEchoHandler {
        api_key: "my-api-key".to_string(),
        api_secret: "my-secret-key".to_string(),
    };

    let client = WebSocketClient::new(config, handler).await?;

    println!("  🔌 Connecting to WebSocket (Auth)...");

    // Subscribe to a topic
    println!("  📤 Subscribing to 'auth' topic...");
    let mut rx = client.subscribe("auth").await?;

    // Send a text message
    println!("  📤 Sending text message");
    client
        .send_json(&json!({ "topic": "auth", "msg": "Authenticated message" }))
        .await?;

    // Receive message
    if let Ok(msg) = rx.recv().await {
        println!("  📩 Received: {:?}", msg);
    }

    // Wait a bit and then disconnect
    tokio::time::sleep(Duration::from_millis(500)).await;

    println!("  🔌 Disconnecting...");
    client.close().await?;

    Ok(())
}
