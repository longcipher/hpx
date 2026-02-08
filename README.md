# hpx

[![DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/longcipher/hpx)
[![Context7](https://img.shields.io/badge/Website-context7.com-blue)](https://context7.com/longcipher/hpx)
[![crates.io](https://img.shields.io/crates/v/hpx.svg)](https://crates.io/crates/hpx)
[![docs.rs](https://docs.rs/hpx/badge.svg)](https://docs.rs/hpx)

![hpx](https://socialify.git.ci/longcipher/hpx/image?font=Source+Code+Pro&language=1&name=1&owner=1&pattern=Circuit+Board&theme=Auto)

This project is a fork of [wreq](https://github.com/0x676e67/wreq) and indirectly [reqwest](https://github.com/seanmonstar/reqwest), designed for the network layer of crypto exchange HFT high-performance applications. The primary goal of this fork is performance optimization.

An ergonomic all-in-one HTTP client for browser emulation with TLS, JA3/JA4, and HTTP/2 fingerprints.

## Version

| Crate | Version | Description |
|-------|---------|-------------|
| [`hpx`](https://crates.io/crates/hpx) | 1.2.0 | High Performance HTTP Client |
| [`hpx-util`](https://crates.io/crates/hpx-util) | 1.2.0 | Browser emulation profiles & Tower middleware |
| [`hpx-transport`](https://crates.io/crates/hpx-transport) | 1.2.0 | Exchange SDK toolkit (auth, WebSocket, rate limiting) |
| [`hpx-yawc`](https://crates.io/crates/hpx-yawc) | 1.2.0 | WebSocket library (RFC 6455 + compression) |
| [`hpx-fastwebsockets`](https://crates.io/crates/hpx-fastwebsockets) | 1.2.0 | Fast minimal WebSocket implementation |

## Features

- **Browser Emulation**: Simulate various browser TLS/HTTP2 fingerprints (JA3/JA4)
- **Content Handling**: Plain bodies, JSON, urlencoded, multipart, streaming
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
- **WebSocket**: Upgrade support with switchable backends (yawc or fastwebsockets)
- **TLS Backends**: BoringSSL (default) and Rustls

## Architecture

```text
+-----------------------------------------------------------+
|                        User API                           |
| (Client, ClientBuilder, RequestBuilder, Response)         |
+-----------------------------------------------------------+
|                     Tower Middleware Stack                 |
| (HooksLayer, RetryLayer, TimeoutLayer, DecompressionLayer)|
+---------------------------+-------------------------------+
|        hpx-core           |   hpx-ws (yawc/fastws)       |
+---------------------------+                               |
|      Connection Pool      |      WebSocket Handshake      |
+-------------+-------------+-------------------------------+
| TLS Backend | (Feature Switched: BoringSSL / Rustls)      |
+-------------+---------------------------------------------+
|                Transport (Tokio TCP Stream)                |
+-----------------------------------------------------------+
```

## Installation

Add `hpx` to your `Cargo.toml`:

```toml
[dependencies]
hpx = "1.2.0"
```

The default features include **BoringSSL** TLS, **HTTP/1.1**, and **HTTP/2** support.

For browser emulation, add the utility crate:

```toml
[dependencies]
hpx = "1.2.0"
hpx-util = "1.2.0"
```

For exchange/trading applications:

```toml
[dependencies]
hpx = "1.2.0"
hpx-transport = "1.2.0"
```

## Feature Flags

### `hpx` Features

The `hpx` crate uses feature flags for fine-grained control. **Default features: `boring`, `http1`, `http2`**.

| Feature | Default | Description |
|---------|---------|-------------|
| **TLS** | | |
| `boring` | **Yes** | BoringSSL TLS backend |
| `rustls-tls` | No | Rustls TLS backend (pure Rust) |
| `webpki-roots` | No | WebPKI root certificates |
| **HTTP** | | |
| `http1` | **Yes** | HTTP/1.1 support |
| `http2` | **Yes** | HTTP/2 support |
| **Content** | | |
| `json` | No | JSON request/response support |
| `simd-json` | No | SIMD-accelerated JSON (enables `json`) |
| `form` | No | x-www-form-urlencoded support |
| `query` | No | URL query string serialization |
| `multipart` | No | Multipart form data |
| `stream` | No | Streaming request/response bodies |
| `charset` | No | Character encoding support |
| **Compression** | | |
| `gzip` | No | Gzip decompression |
| `brotli` | No | Brotli decompression |
| `zstd` | No | Zstandard decompression |
| `deflate` | No | Deflate decompression |
| **WebSocket** | | |
| `ws` | No | WebSocket support (alias for `ws-yawc`) |
| `ws-yawc` | No | WebSocket via hpx-yawc backend |
| `ws-fastwebsockets` | No | WebSocket via fastwebsockets backend |
| **Networking** | | |
| `cookies` | No | Cookie store support |
| `socks` | No | SOCKS proxy support |
| `hickory-dns` | No | Async DNS resolver (Hickory) |
| `system-proxy` | No | System proxy configuration |
| **Observability** | | |
| `tracing` | No | Tracing/logging support |
| **Other** | | |
| `macros` | No | Tokio macros re-export |

### WebSocket Backend Selection

The `ws` feature is an alias for `ws-yawc` (the default WebSocket backend). To use fastwebsockets instead:

```toml
[dependencies]
hpx = { version = "1.2.0", features = ["ws-fastwebsockets"] }
```

When both `ws-yawc` and `ws-fastwebsockets` are enabled, fastwebsockets takes priority.

### Common Feature Combinations

**Minimal HTTP client:**

```toml
hpx = "1.2.0"  # default: boring + http1 + http2
```

**JSON API client:**

```toml
hpx = { version = "1.2.0", features = ["json", "cookies", "gzip"] }
```

**WebSocket client:**

```toml
hpx = { version = "1.2.0", features = ["ws"] }
```

**High-performance trading:**

```toml
hpx = { version = "1.2.0", features = ["simd-json", "hickory-dns", "zstd", "ws"] }
```

**Pure Rust (no C dependencies):**

```toml
hpx = { version = "1.2.0", default-features = false, features = ["rustls-tls", "http1", "http2"] }
```

**Full-featured:**

```toml
hpx = { version = "1.2.0", features = [
    "json", "form", "query", "multipart", "stream",
    "cookies", "charset",
    "gzip", "brotli", "zstd", "deflate",
    "ws", "socks", "hickory-dns",
    "tracing",
] }
```

### `hpx-util` Features

Default: `emulation`.

| Feature | Default | Description |
|---------|---------|-------------|
| `emulation` | **Yes** | Browser emulation profiles (Chrome, Firefox, Safari, Opera, OkHttp) |
| `emulation-compression` | No | Compression settings for emulation profiles |
| `emulation-rand` | No | Random emulation profile selection |
| `emulation-serde` | No | Serde serialization for emulation types |
| `tower-delay` | No | Delay/jitter Tower middleware layer |

### `hpx-transport` Features

Default: `ws-yawc`.

| Feature | Default | Description |
|---------|---------|-------------|
| `ws-yawc` | **Yes** | WebSocket backend via hpx-yawc |
| `ws-fastwebsockets` | No | WebSocket backend via fastwebsockets |

### `hpx-yawc` Features

Default: `rustls-ring`.

| Feature | Default | Description |
|---------|---------|-------------|
| `rustls-ring` | **Yes** | TLS via rustls with ring crypto |
| `rustls-aws-lc-rs` | No | TLS via rustls with AWS LC crypto |
| `axum` | No | Axum WebSocket extractor |
| `simd` | No | SIMD-accelerated UTF-8 validation |
| `smol` | No | smol async runtime support |
| `zlib` | No | Native zlib compression backend |

## TLS Backend Configuration

### BoringSSL (Default)

BoringSSL is the default TLS backend, providing robust support for modern TLS features and extensive browser emulation capabilities. Recommended for most use cases, especially when browser fingerprinting is required.

```toml
[dependencies]
hpx = "1.2.0"
```

### Rustls

A pure Rust TLS implementation. Useful for environments where C dependencies are difficult to manage or when you prefer the safety guarantees of a pure Rust stack.

> **Note:** Switching to Rustls may affect the availability or behavior of certain browser emulation features that rely on specific BoringSSL capabilities.

```toml
[dependencies]
hpx = { version = "1.2.0", default-features = false, features = ["rustls-tls", "http1", "http2"] }
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

    println!("body = {body:?}");
    Ok(())
}
```

### JSON Requests

Requires the `json` feature.

```rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
struct User {
    name: String,
    email: String,
}

#[tokio::main]
async fn main() -> hpx::Result<()> {
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

### Browser Emulation

Requires the `hpx-util` crate with default features.

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

### WebSocket

Requires the `ws` feature.

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

### Cookies

Requires the `cookies` feature.

```rust
use hpx::cookie::Jar;

#[tokio::main]
async fn main() -> hpx::Result<()> {
    let jar = Jar::default();

    let client = hpx::Client::builder()
        .cookie_provider(jar.clone())
        .build()?;

    let _resp = client
        .get("https://httpbin.org/cookies/set/session/123")
        .send()
        .await?;

    println!("Cookies: {:?}", jar.cookies("https://httpbin.org"));
    Ok(())
}
```

### Transport Layer (Exchange SDK)

For cryptocurrency exchange integrations, use the `hpx-transport` crate:

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

## Performance Optimization Tips

- **`simd-json`**: Replaces `serde_json` with SIMD-accelerated JSON parsing
- **`hickory-dns`**: High-performance async DNS resolver, avoids blocking system calls
- **`zstd`**: Fastest compression/decompression ratio for most workloads
- **Lock-free internals**: Uses `scc` concurrent containers and `arc-swap` for hot-path data

## License

Apache-2.0
