# hpx-fastwebsockets

[![crates.io](https://img.shields.io/crates/v/hpx-fastwebsockets.svg)](https://crates.io/crates/hpx-fastwebsockets)
[![docs.rs](https://docs.rs/hpx-fastwebsockets/badge.svg)](https://docs.rs/hpx-fastwebsockets)
[![License](https://img.shields.io/crates/l/hpx-fastwebsockets.svg)](https://github.com/longcipher/hpx)

A fast, minimal RFC 6455 WebSocket implementation.

This crate is part of the [hpx](https://github.com/longcipher/hpx) project. It is a fork of [fastwebsockets](https://github.com/denoland/fastwebsockets) by Divy Srivastava.

Passes the Autobahn test suite and fuzzed with LLVM's libfuzzer. Can be used as a raw WebSocket frame parser or as a full-fledged WebSocket server/client.

## Quick Start

```rust
use tokio::net::TcpStream;
use hpx_fastwebsockets::{WebSocket, OpCode, Role};
use eyre::Result;

async fn handle(mut ws: WebSocket<TcpStream>) -> Result<()> {
    loop {
        let frame = ws.read_frame().await?;
        match frame.opcode {
            OpCode::Close => break,
            OpCode::Text | OpCode::Binary => {
                ws.write_frame(frame).await?;
            }
            _ => {}
        }
    }
    Ok(())
}
```

## HTTP Upgrade (with hyper)

```rust
use hpx_fastwebsockets::upgrade;
use hyper::{Request, Response, body::Incoming};
use http_body_util::Empty;
use hyper::body::Bytes;

async fn server_upgrade(
    mut req: Request<Incoming>,
) -> Result<Response<Empty<Bytes>>, hpx_fastwebsockets::WebSocketError> {
    let (response, fut) = upgrade::upgrade(&mut req)?;

    tokio::spawn(async move {
        let mut ws = fut
            .await
            .expect("Failed to complete WebSocket upgrade");
        // Use ws.read_frame() / ws.write_frame() ...
    });

    Ok(response)
}
```

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `simd` | **Yes** | SIMD-accelerated UTF-8 validation |
| `upgrade` | **Yes** | HTTP upgrade support (requires hyper) |
| `unstable-split` | No | Split WebSocket into read/write halves |
| `with_axum` | No | Axum WebSocket extractor integration |

## Key Types

- `WebSocket<S>` — Main WebSocket struct for reading/writing frames
- `Frame` — A WebSocket frame with opcode and payload
- `OpCode` — Frame opcodes (Text, Binary, Close, Ping, Pong, Continuation)
- `FragmentCollector<S>` — Aggregates fragmented messages into complete frames
- `Role` — Server or Client role (controls masking behavior)

## License

Apache-2.0
