# hpx

An ergonomic all-in-one HTTP client for browser emulation with TLS, JA3/JA4, and HTTP/2 fingerprints.

## Features

- **Browser Emulation**: Simulate various browser TLS/HTTP2 fingerprints (JA3/JA4).
- **Content Handling**: Plain bodies, JSON, urlencoded, multipart.
- **Advanced Features**:
  - Cookies Store
  - Redirect Policy
  - Original Header Preservation
  - Rotating Proxies
  - Certificate Store
  - Tower Middleware Support
- **WebSocket**: Upgrade support for WebSockets.
- **TLS Backends**: Support for both BoringSSL (default) and Rustls.

## Architecture

```text
+-----------------------------------------------------------+
|                        User API                           |
| (HpxClient, HpxBuilder, RequestBuilder, WsConnection)     |
+-----------------------------------------------------------+
|                     Tower Middleware Stack                |
| (AuthLayer, RetryLayer, TimeoutLayer, DecompressionLayer) |
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

Add `hpx` to your `Cargo.toml`:

```toml
[dependencies]
hpx = "0.1"
```

## TLS Backend Configuration

`hpx` supports two TLS backends: **BoringSSL** (default) and **Rustls**.

### BoringSSL (Default)

BoringSSL is the default TLS backend, providing robust support for modern TLS features and extensive browser emulation capabilities. It is recommended for most use cases, especially when browser fingerprinting is required.

To use BoringSSL, no additional configuration is needed if you are using the default features:

```toml
[dependencies]
hpx = "0.1"
```

### Rustls

If you prefer a pure Rust TLS implementation, you can switch to Rustls. This might be useful for environments where C dependencies are difficult to manage or if you prefer the safety guarantees of a pure Rust stack.

**Note:** Switching to Rustls may affect the availability or behavior of certain browser emulation features that rely on specific BoringSSL capabilities.

To use Rustls, you must disable the default features and explicitly enable `rustls-tls` along with the HTTP versions you need:

```toml
[dependencies]
hpx = { version = "0.1", default-features = false, features = ["rustls-tls", "http1", "http2"] }
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

## License

Apache-2.0
