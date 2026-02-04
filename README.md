# hpx

[![DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/longcipher/hpx)
[![Context7](https://img.shields.io/badge/Website-context7.com-blue)](https://context7.com/longcipher/hpx)
[![crates.io](https://img.shields.io/crates/v/hpx.svg)](https://crates.io/crates/hpx)
[![docs.rs](https://docs.rs/hpx/badge.svg)](https://docs.rs/hpx)

![hpx](https://socialify.git.ci/longcipher/hpx/image?font=Source+Code+Pro&language=1&name=1&owner=1&pattern=Circuit+Board&theme=Auto)

This project is a fork of [wreq](https://github.com/0x676e67/wreq) and indirectly [reqwest](https://github.com/seanmonstar/reqwest), designed for the network layer of crypto exchange HFT high-performance applications. The primary goal of this fork is performance optimization.

An ergonomic all-in-one HTTP client for browser emulation with TLS, JA3/JA4, and HTTP/2 fingerprints.

## Features

- **Browser Emulation**: Simulate various browser TLS/HTTP2 fingerprints (JA3/JA4).
- **Content Handling**: Plain bodies, JSON, urlencoded, multipart, streaming.
- **Advanced Features**:
  - Cookies Store
  - Redirect Policy
  - Original Header Preservation
  - Rotating Proxies
  - Certificate Store
  - Tower Middleware Support
  - Request/Response Hooks
  - Retry Configuration
  - Compression (gzip, brotli, deflate, zstd)
  - Character Encoding Support
- **WebSocket**: Upgrade support for WebSockets.
- **TLS Backends**: Support for both BoringSSL (default) and Rustls.

## Architecture

```text
+-----------------------------------------------------------+
|                        User API                           |
| (Client, ClientBuilder, RequestBuilder, Response)         |
+-----------------------------------------------------------+
|                     Tower Middleware Stack                |
| (HooksLayer, RetryLayer, TimeoutLayer, DecompressionLayer)|
+---------------------------+-------------------------------+
|        hpx-core           |      hpx-ws (fastwebsockets)  |
+---------------------------+                               |
|      Connection Pool      |      WebSocket Handshake      |
+-------------+-------------+-------------------------------+
| TLS Backend | (Feature Switched: Rustls / BoringSSL)      |
+-------------+---------------------------------------------+
|                Transport (Tokio TCP Stream)               |
+-----------------------------------------------------------+
```

## Installation

Add `hpx` and related crates to your `Cargo.toml`:

```toml
[dependencies]
hpx = "1.0.0"
hpx-util = "1.0.0"
hpx-transport = "1.0.0"  # Optional: for transport layer access
```

### Feature Flags

`hpx` provides extensive feature flags for customization:

```toml
[dependencies]
hpx = { version = "1.0.0", features = [
    "json",           # JSON request/response support
    "stream",         # Streaming request/response bodies
    "cookies",        # Cookie store support
    "charset",        # Character encoding support
    "gzip",           # Gzip compression
    "brotli",         # Brotli compression
    "zstd",           # Zstandard compression
    "multipart",      # Multipart form data
    "ws",             # WebSocket support
    "socks",          # SOCKS proxy support
    "hickory-dns",    # Alternative DNS resolver
    "tracing",        # Logging support
] }
```

### Performance Optimization

To achieve the best possible performance, consider enabling the following features:

- **`simd-json`**: Replaces `serde_json` with `simd-json` for faster JSON serialization/deserialization (uses SIMD instructions where available).
- **`hickory-dns`**: Enables the high-performance, async-native Hickory (formerly Trust-DNS) resolver, avoiding blocking system calls.
- **`zstd` / `brotli`**: Use modern compression algorithms for better bandwidth efficiency. `zstd` usually offers the best balance of speed and compression ratio.

**Recommended configuration for high-performance applications:**

```toml
[dependencies]
hpx = { version = "1.0.0", features = [
    "simd-json",      # SIMD-accelerated JSON handling
    "hickory-dns",    # Async DNS resolver
    "zstd",           # Fast compression
] }
```

## TLS Backend Configuration

`hpx` supports two TLS backends: **BoringSSL** (default) and **Rustls**.

### BoringSSL (Default)

BoringSSL is the default TLS backend, providing robust support for modern TLS features and extensive browser emulation capabilities. It is recommended for most use cases, especially when browser fingerprinting is required.

To use BoringSSL, no additional configuration is needed if you are using the default features:

```toml
[dependencies]
hpx = "1.0.0"
```

### Rustls

If you prefer a pure Rust TLS implementation, you can switch to Rustls. This might be useful for environments where C dependencies are difficult to manage or if you prefer the safety guarantees of a pure Rust stack.

**Note:** Switching to Rustls may affect the availability or behavior of certain browser emulation features that rely on specific BoringSSL capabilities.

To use Rustls, you must disable the default features and explicitly enable `rustls-tls` along with the HTTP versions you need:

```toml
[dependencies]
hpx = { version = "1.0.0", default-features = false, features = ["rustls-tls", "http1", "http2"] }
```

## Usage Examples

### Making a GET Request

```rust
#[tokio::main]
async fn main() -> hpx::Result<()> {
    let body = hpx::get("https://www.rust-lang.org")
        .send()
        .await?
        .text()
        .await?;

    println!("body = {:?}", body);
    Ok(())
}
```

### JSON Requests

```rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
struct User {
    name: String,
    email: String,
}

#[tokio::main]
async fn main() -> hpx::Result<()> {
    // POST with JSON body
    let user = User {
        name: "John Doe".to_string(),
        email: "john@example.com".to_string(),
    };

    let response = hpx::Client::new()
        .post("https://jsonplaceholder.typicode.com/users")
        .json(&user)
        .send()
        .await?;

    println!("Status: {}", response.status());
    Ok(())
}
```

### Request/Response Hooks

Add lifecycle hooks to monitor and modify requests/responses:

```rust
use hpx::client::{layer::hooks::{Hooks, LoggingHook, RequestIdHook}, Client};

#[tokio::main]
async fn main() -> hpx::Result<()> {
    let client = Client::builder()
        .hooks(Hooks::new()
            .before_request(|req| {
                println!("Sending request to: {}", req.uri());
                async { Ok(()) }
            })
            .after_response(|res| {
                println!("Received response: {}", res.status());
                async { Ok(()) }
            })
        )
        .build()?;

    let _response = client.get("https://httpbin.org/get").send().await?;
    Ok(())
}
```

### Retry Configuration

Configure retry behavior for failed requests:

```rust
use hpx::{client::http::ClientBuilder, retry::RetryConfig};
use std::time::Duration;

#[tokio::main]
async fn main() -> hpx::Result<()> {
    let client = ClientBuilder::new()
        .retry(RetryConfig::new()
            .max_attempts(3)
            .backoff(Duration::from_millis(100))
            .max_backoff(Duration::from_secs(10))
        )
        .build()?;

    let response = client.get("https://httpbin.org/status/500").send().await?;
    println!("Final status: {}", response.status());
    Ok(())
}
```

### Streaming Response Bodies

Efficiently stream large response bodies:

```rust
use tokio::io::AsyncReadExt;

#[tokio::main]
async fn main() -> hpx::Result<()> {
    let mut reader = hpx::get("https://httpbin.org/stream/100")
        .send()
        .await?
        .reader();

    let mut buffer = Vec::new();
    reader.read_to_end(&mut buffer).await?;
    println!("Read {} bytes", buffer.len());
    Ok(())
}
```

### Cookies

Automatic cookie handling:

```rust
use hpx::cookie::Jar;

#[tokio::main]
async fn main() -> hpx::Result<()> {
    let jar = Jar::default();

    let client = hpx::Client::builder()
        .cookie_provider(jar.clone())
        .build()?;

    // Cookies will be automatically stored and sent
    let _resp = client.get("https://httpbin.org/cookies/set/session/123").send().await?;

    // Check stored cookies
    println!("Cookies: {:?}", jar.cookies("https://httpbin.org"));
    Ok(())
}
```

### WebSocket

Upgrade a connection to a WebSocket.

```rust
use futures_util::{SinkExt, StreamExt, TryStreamExt};
use hpx::{header, ws::message::Message};

#[tokio::main]
async fn main() -> hpx::Result<()> {
    let websocket = hpx::websocket("wss://echo.websocket.org")
        .header(header::USER_AGENT, env!("CARGO_PKG_NAME"))
        .send()
        .await?;

    let (mut tx, mut rx) = websocket.into_websocket().await?.split();

    tokio::spawn(async move {
        if let Err(err) = tx.send(Message::text("Hello, World!")).await {
            eprintln!("failed to send message: {err}");
        }
    });

    while let Some(message) = rx.try_next().await? {
        if let Message::Text(text) = message {
            println!("received: {text}");
        }
    }

    Ok(())
}
```

### Browser Emulation

The `emulation` module allows you to simulate specific browser fingerprints.

```rust
use hpx_util::Emulation;

#[tokio::main]
async fn main() -> hpx::Result<()> {
    let resp = hpx::get("https://tls.peet.ws/api/all")
        .emulation(Emulation::Firefox136)
        .send()
        .await?;

    println!("{}", resp.text().await?);
    Ok(())
}
```

### Transport Layer

For advanced use cases, you can work directly with the transport layer:

```rust
use hpx_transport::{HttpClient, typed::TypedRequestBuilder};
use std::time::Duration;

#[tokio::main]
async fn main() -> hpx_transport::Result<()> {
    let client = HttpClient::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let request = TypedRequestBuilder::get("https://httpbin.org/get")
        .header("User-Agent", "hpx-transport")
        .build()?;

    let response = client.send_request(request).await?;
    println!("Status: {}", response.status());
    Ok(())
}
```

## License

Apache-2.0
