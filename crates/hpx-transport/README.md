# hpx-transport

[![crates.io](https://img.shields.io/crates/v/hpx-transport.svg)](https://crates.io/crates/hpx-transport)
[![docs.rs](https://docs.rs/hpx-transport/badge.svg)](https://docs.rs/hpx-transport)
[![License](https://img.shields.io/crates/l/hpx-transport.svg)](https://github.com/longcipher/hpx)

Exchange SDK toolkit for cryptocurrency trading with authentication, WebSocket, and rate limiting.

This crate is part of the [hpx](https://github.com/longcipher/hpx) project and builds on the `hpx` HTTP client to provide exchange-specific functionality.

## Features

- **Authentication**: API key, Bearer token, HMAC signing, and composable auth strategies
- **REST Client**: Generic exchange REST client with typed responses
- **WebSocket**: Single-task connection with `Connection`/`Handle`/`Stream` split API and auto-reconnect
- **Rate Limiting**: Token bucket rate limiter using lock-free `scc` containers
- **Typed Responses**: Generic response wrapper with metadata and error handling
- **Metrics**: OpenTelemetry OTLP gRPC metrics integration

## Quick Start

```rust
use hpx_transport::{
    auth::ApiKeyAuth,
    exchange::{RestClient, RestConfig},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = RestConfig::new("https://api.example.com")
        .timeout(std::time::Duration::from_secs(30));

    let auth = ApiKeyAuth::header("X-API-Key", "my-api-key");
    let client = RestClient::new(config, auth)?;

    // Use the client...
    Ok(())
}
```

## Live Proxy Pool Tests

The repository includes an ignored live test that validates proxy pool behavior
(`StickyFailover` and `RandomPerRequest`) against a real proxy list.

Default proxy list path:

```text
docs/webshare_proxy_list.txt
```

You can override the file path with `HPX_PROXY_LIST_PATH`.

```bash
HPX_PROXY_LIST_PATH=/absolute/path/to/proxy_list.txt \
cargo test -p hpx-transport --test proxy_pool_live_webshare -- --ignored --nocapture --test-threads=1
```

## WebSocket (Split API)

```rust,no_run
use hpx_transport::websocket::{Connection, Event, WsConfig};
use hpx_transport::websocket::handlers::GenericJsonHandler;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let config = WsConfig::new("wss://api.exchange.com/ws");
let handler = GenericJsonHandler::new();

let connection = Connection::connect_stream(config, handler).await?;
let (handle, mut stream) = connection.split();

handle.subscribe("trades.BTC").await?;
while let Some(event) = stream.next().await {
    if let Event::Message(msg) = event {
        println!("Control/unknown message: {:?}", msg.kind);
    }
}
# Ok(())
# }
```

## Rate Limiting

```rust
use hpx_transport::rate_limit::RateLimiter;

let limiter = RateLimiter::new();
limiter.add_limit("orders", 10, 1.0).unwrap(); // 10 capacity, 1/sec refill

if limiter.try_acquire("orders") {
    println!("Request allowed");
}
```

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `ws-yawc` | **Yes** | WebSocket backend via hpx-yawc |
| `ws-fastwebsockets` | No | WebSocket backend via fastwebsockets |

## Key Dependencies

- [`hpx`](https://crates.io/crates/hpx) — HTTP client (with `json`, `tracing`, `boring`, `http1`, `http2`)
- [`opentelemetry`](https://crates.io/crates/opentelemetry) — Metrics via OTLP gRPC
- [`scc`](https://crates.io/crates/scc) — Lock-free concurrent containers
- [`tracing`](https://crates.io/crates/tracing) — Structured logging

## License

Apache-2.0
