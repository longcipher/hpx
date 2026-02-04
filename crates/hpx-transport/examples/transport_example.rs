//! Example demonstrating the new hpx-transport architecture
//!
//! This example shows how to use both REST and WebSocket clients
//! with authentication, and error handling.

use std::time::Duration;

use hpx::ws::message::Message;
use hpx_transport::{
    auth::{ApiKeyAuth, BearerAuth, CompositeAuth, NoAuth},
    error::TransportResult,
    exchange::{ExchangeClient, RestClient, RestConfig},
    rate_limit::RateLimiter,
    websocket::{ExchangeHandler, WebSocketConfig},
};
use serde::Deserialize;
use serde_json::json;

#[tokio::main]
async fn main() -> TransportResult<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    println!("🚀 hpx-transport Examples");

    // Example 1: REST Client with API Key Authentication
    println!("\n📡 Example 1: REST Client with API Key Auth");
    rest_example().await?;

    // Example 2: REST Client with Bearer Token Authentication
    println!("\n🔐 Example 2: REST Client with Bearer Token Auth");
    rest_bearer_example().await?;

    // Example 3: Typed Response Example
    println!("\n📊 Example 3: Typed Response Example");
    typed_response_example().await?;

    // Example 4: Rate Limiting Example
    println!("\n⏱️  Example 4: Rate Limiting Example");
    rate_limiting_example();

    // Example 5: Composite Authentication
    println!("\n🔗 Example 5: Composite Authentication");
    composite_auth_example().await?;

    // Example 6: WebSocket Handler (Conceptual)
    println!("\n🌐 Example 6: WebSocket Handler (Conceptual)");
    websocket_example();

    println!("\n✅ All examples completed successfully!");
    Ok(())
}

/// Example using REST client with API key authentication
async fn rest_example() -> TransportResult<()> {
    let config = RestConfig::new("https://httpbin.org")
        .timeout(Duration::from_secs(10))
        .user_agent("hpx-transport-example/1.0");

    let auth = ApiKeyAuth::header("X-API-Key", "demo-api-key");
    let client = RestClient::new(config, auth)?;

    println!("  📤 Sending GET request to /get");

    match client.get::<serde_json::Value>("/get?symbol=BTCUSDT").await {
        Ok(response) => {
            println!("  ✅ Response status: {}", response.status);
            println!("  ⏱️  Latency: {:?}", response.latency);

            if let Some(args) = response.data.get("args") {
                println!("  🔍 Query parameters: {}", args);
            }
            if let Some(headers) = response.data.get("headers")
                && let Some(api_key) = headers.get("X-Api-Key")
            {
                println!("  🔑 API Key header received: {}", api_key);
            }
        }
        Err(e) => {
            println!("  ❌ Request failed: {}", e);
        }
    }

    Ok(())
}

/// Example using REST client with Bearer token authentication
async fn rest_bearer_example() -> TransportResult<()> {
    let config = RestConfig::new("https://httpbin.org").timeout(Duration::from_secs(10));

    let auth = BearerAuth::new("demo-bearer-token");
    let client = RestClient::new(config, auth)?;

    // Build a POST request with JSON body
    let body = json!({
        "symbol": "ETHUSDT",
        "side": "BUY",
        "quantity": 0.5
    });

    println!("  📤 Sending POST request to /post");

    let response: hpx_transport::typed::TypedResponse<serde_json::Value> =
        client.post("/post", &body).await?;
    println!("  ✅ Response status: {}", response.status);
    println!("  ⏱️  Latency: {:?}", response.latency);

    if let Some(json_data) = response.data.get("json") {
        println!("  📊 Echoed JSON: {}", json_data);
    }

    Ok(())
}

/// Example with typed responses
async fn typed_response_example() -> TransportResult<()> {
    #[derive(Debug, Deserialize)]
    struct HttpBinResponse {
        args: std::collections::HashMap<String, String>,
        url: String,
    }

    let config = RestConfig::new("https://httpbin.org").timeout(Duration::from_secs(10));

    let auth = NoAuth;
    let client = RestClient::new(config, auth)?;

    println!("  📤 Sending typed GET request to /get");

    match client
        .get::<HttpBinResponse>("/get?key=value&foo=bar")
        .await
    {
        Ok(response) => {
            println!("  ✅ Response status: {}", response.status);
            println!("  🔗 URL: {}", response.data.url);
            println!("  📊 Args: {:?}", response.data.args);
        }
        Err(e) => {
            println!("  ❌ Request failed: {}", e);
        }
    }

    Ok(())
}

/// Example demonstrating rate limiting
fn rate_limiting_example() {
    let limiter = RateLimiter::new();

    // Add rate limits
    limiter.add_limit("orders", 10, 1.0); // 10 tokens, 1 per second refill
    limiter.add_limit("public", 100, 10.0); // 100 tokens, 10 per second refill

    println!("  📊 Rate limiter configured");
    println!("  - orders: 10 capacity, 1/sec refill");
    println!("  - public: 100 capacity, 10/sec refill");

    // Simulate some requests
    let mut order_count = 0;
    for _ in 0..15 {
        if limiter.try_acquire("orders") {
            order_count += 1;
        }
    }
    println!("  ✅ Acquired {} order tokens (expected ~10)", order_count);

    if let Some(tokens) = limiter.available_tokens("orders") {
        println!("  📈 Remaining order tokens: {:.2}", tokens);
    }

    // Reset and try again
    limiter.reset("orders");
    println!("  🔄 Reset orders bucket");

    if let Some(tokens) = limiter.available_tokens("orders") {
        println!("  📈 Remaining order tokens after reset: {:.2}", tokens);
    }
}

/// Example with composite authentication
async fn composite_auth_example() -> TransportResult<()> {
    let config = RestConfig::new("https://httpbin.org").timeout(Duration::from_secs(10));

    // Combine API key and bearer auth
    let api_key = ApiKeyAuth::header("X-API-Key", "my-api-key");
    let bearer = BearerAuth::new("my-bearer-token");
    let auth = CompositeAuth::new(api_key, bearer);

    let client = RestClient::new(config, auth)?;

    println!("  📤 Sending request with composite auth");

    match client.get::<serde_json::Value>("/headers").await {
        Ok(response) => {
            println!("  ✅ Response status: {}", response.status);

            if let Some(headers) = response.data.get("headers") {
                if let Some(api_key) = headers.get("X-Api-Key") {
                    println!("  🔑 API Key header: {}", api_key);
                }
                if let Some(auth) = headers.get("Authorization") {
                    println!("  🔐 Authorization header: {}", auth);
                }
            }
        }
        Err(e) => {
            println!("  ❌ Request failed: {}", e);
        }
    }

    Ok(())
}

/// Example demonstrating WebSocket handler pattern
fn websocket_example() {
    // Define a simple exchange handler for a hypothetical exchange
    struct SimpleExchangeHandler;

    impl ExchangeHandler for SimpleExchangeHandler {
        fn get_topic(&self, message: &str) -> Option<String> {
            // Parse message and extract topic
            // For example, Binance uses: {"e": "trade", "s": "BTCUSDT", ...}
            serde_json::from_str::<serde_json::Value>(message)
                .ok()
                .and_then(|v| v.get("s")?.as_str().map(|s| s.to_string()))
        }

        fn build_subscribe(&self, topic: &str) -> Message {
            // Build subscription message
            let msg = json!({
                "method": "SUBSCRIBE",
                "params": [format!("{}@trade", topic.to_lowercase())],
                "id": 1
            });
            Message::Text(msg.to_string().into())
        }

        fn build_unsubscribe(&self, topic: &str) -> Message {
            // Build unsubscription message
            let msg = json!({
                "method": "UNSUBSCRIBE",
                "params": [format!("{}@trade", topic.to_lowercase())],
                "id": 2
            });
            Message::Text(msg.to_string().into())
        }

        fn on_connect(&self) -> Vec<Message> {
            // Optional: messages to send on connection (e.g., auth)
            vec![]
        }
    }

    println!("  📝 WebSocket handler pattern:");
    println!("     - ExchangeHandler: Implement to customize message routing");
    println!("     - WebSocketConfig: Configure reconnection and timeouts");
    println!("     - WebSocketHandle: Use for subscribe/unsubscribe/send");

    // Show configuration
    let config = WebSocketConfig::new("wss://stream.binance.com:9443/ws")
        .reconnect_initial_delay(Duration::from_millis(100))
        .reconnect_max_delay(Duration::from_secs(30))
        .ping_interval(Duration::from_secs(30));

    println!("  🔧 Configuration:");
    println!("     - URL: {}", config.url);
    println!(
        "     - Reconnect delay: {:?} to {:?}",
        config.reconnect_initial_delay, config.reconnect_max_delay
    );
    println!("     - Ping interval: {:?}", config.ping_interval);

    println!("  ✅ WebSocket example pattern ready");
    println!("     To use, call: WebSocketHandle::connect(config, handler).await");

    // Suppress unused warning for handler demo
    let _ = SimpleExchangeHandler;
}
