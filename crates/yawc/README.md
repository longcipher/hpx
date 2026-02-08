# hpx-yawc

[![crates.io](https://img.shields.io/crates/v/hpx-yawc.svg)](https://crates.io/crates/hpx-yawc)
[![docs.rs](https://docs.rs/hpx-yawc/badge.svg)](https://docs.rs/hpx-yawc)
[![License](https://img.shields.io/crates/l/hpx-yawc.svg)](https://github.com/longcipher/hpx)

A fast, secure WebSocket implementation with RFC 6455 compliance and permessage-deflate compression (RFC 7692). Autobahn compliant. Supports WASM targets.

This crate is part of the [hpx](https://github.com/longcipher/hpx) project. It is a fork of [yawc](https://github.com/infinitefield/yawc) (Yet Another WebSocket Client).

## Features

- Full RFC 6455 compliance — passes the Autobahn test suite
- Permessage-deflate compression (RFC 7692) with configurable window bits
- SIMD-accelerated frame masking (NEON on ARM, AVX2 on x86)
- Optional SIMD-accelerated UTF-8 validation
- WASM target support
- Axum WebSocket extractor integration
- Multiple TLS backends (ring, aws-lc-rs)
- Alternative async runtime support (smol)

## Quick Start

### Client

```rust
use futures::{SinkExt, StreamExt};
use hpx_yawc::{WebSocket, frame::OpCode};

async fn connect() -> hpx_yawc::Result<()> {
    let mut ws = WebSocket::connect("wss://echo.websocket.org".parse()?).await?;

    while let Some(frame) = ws.next().await {
        match frame.opcode() {
            OpCode::Text | OpCode::Binary => ws.send(frame).await?,
            OpCode::Ping => {
                // Pong is sent automatically, but ping is still returned
            }
            _ => {}
        }
    }
    Ok(())
}
```

### Server (with hyper)

```rust
use futures::StreamExt;
use http_body_util::Empty;
use hyper::{
    Request, Response,
    body::{Bytes, Incoming},
};
use hpx_yawc::WebSocket;

async fn upgrade(mut req: Request<Incoming>) -> hpx_yawc::Result<Response<Empty<Bytes>>> {
    let (response, fut) = WebSocket::upgrade(&mut req)?;

    tokio::spawn(async move {
        if let Ok(mut ws) = fut.await {
            while let Some(frame) = ws.next().await {
                // Process frames
            }
        }
    });

    Ok(response)
}
```

## Protocol Handling

hpx-yawc automatically handles WebSocket control frames:

- **Ping frames**: Automatically responded to with pongs, but still returned to your application
- **Pong frames**: Passed through without special handling
- **Close frames**: Automatically acknowledged, then returned before closing the connection

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `rustls-ring` | **Yes** | TLS via rustls with ring crypto backend |
| `rustls-aws-lc-rs` | No | TLS via rustls with AWS LC crypto backend |
| `axum` | No | Axum WebSocket extractor integration |
| `simd` | No | SIMD-accelerated UTF-8 validation |
| `smol` | No | smol async runtime support |
| `zlib` | No | zlib compression backend (enables window bits configuration) |

## Compression

Permessage-deflate compression is always available with the default flate2/miniz_oxide backend (a pure Rust implementation of deflate). The `zlib` feature enables the native zlib backend and advanced [window bits](https://docs.rs/hpx-yawc/latest/hpx_yawc/struct.Options.html#method.with_client_max_window_bits) configuration parameters.

## Runtime Support

hpx-yawc is built on tokio's I/O traits but can work with other async runtimes through simple adapters. Enable the `smol` feature for built-in smol runtime support, or implement trait bridges between your runtime's I/O traits and tokio's `AsyncRead`/`AsyncWrite`.

## License

Apache-2.0
