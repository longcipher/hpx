# hpx

[![crates.io](https://img.shields.io/crates/v/hpx.svg)](https://crates.io/crates/hpx)
[![docs.rs](https://docs.rs/hpx/badge.svg)](https://docs.rs/hpx)
[![License](https://img.shields.io/crates/l/hpx.svg)](https://github.com/longcipher/hpx)

High Performance HTTP Client — an ergonomic all-in-one HTTP client for browser emulation with TLS, JA3/JA4, and HTTP/2 fingerprints.

This is the core crate of the [hpx](https://github.com/longcipher/hpx) project.

## Features

- **Browser Emulation**: Simulate various browser TLS/HTTP2 fingerprints (JA3/JA4)
- **Content Handling**: Plain bodies, JSON, urlencoded, multipart, streaming
- **Cookies Store** with automatic cookie jar management
- **Redirect Policy** with configurable behaviors
- **Rotating Proxies** with SOCKS support
- **Certificate Store** management
- **Mutual TLS** client certificate authentication
- **Tower Middleware** support for extensibility
- **Request/Response Hooks** for lifecycle monitoring
- **Retry Configuration** with backoff strategies
- **Compression**: gzip, brotli, deflate, zstd
- **Character Encoding** support
- **WebSocket Upgrade** with switchable backends (yawc or fastwebsockets)
- **TLS Backends**: BoringSSL (default) and Rustls
- **HTTP/1.1 and HTTP/2** support

## Quick Start

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

## Cloudflare mTLS

`hpx` can attach a client certificate during the TLS handshake, which is required for Cloudflare Access / Zero Trust deployments protected by mutual TLS.

```rust
use hpx::{Client, tls::Identity};
use std::fs;

#[tokio::main]
async fn main() -> hpx::Result<()> {
    let pem = fs::read("cloudflare-client.pem")?;

    let client = Client::builder()
        .identity(Identity::from_pem(&pem)?)
        .build()?;

    let response = client
        .get("https://mtls-protected.example.com")
        .send()
        .await?;

    println!("status = {}", response.status());
    Ok(())
}
```

- Use `Identity::from_pem(...)` when Cloudflare gives you a single PEM bundle containing the certificate chain and private key.
- Use `Identity::from_pkcs8_pem(cert_pem, key_pem)` when the certificate chain and private key are stored in separate PEM files.
- Use `Identity::from_pkcs12_der(...)` for `.p12` / `.pfx` archives when your client certificate is packaged as a PKCS#12 bundle.
- `Identity::from_pem(...)`, `Identity::from_pkcs8_pem(...)`, and `Identity::from_pkcs12_der(...)` work on both the BoringSSL and Rustls backends.

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `boring` | **Yes** | BoringSSL TLS backend |
| `http1` | **Yes** | HTTP/1.1 support |
| `http2` | **Yes** | HTTP/2 support |
| `rustls-tls` | No | Rustls TLS backend (pure Rust) |
| `json` | No | JSON request/response support |
| `simd-json` | No | SIMD-accelerated JSON (requires `json`) |
| `cookies` | No | Cookie store support |
| `charset` | No | Character encoding support |
| `gzip` | No | Gzip decompression |
| `brotli` | No | Brotli decompression |
| `zstd` | No | Zstandard decompression |
| `deflate` | No | Deflate decompression |
| `query` | No | URL query string serialization |
| `form` | No | x-www-form-urlencoded support |
| `multipart` | No | Multipart form data |
| `stream` | No | Streaming request/response bodies |
| `ws` | No | WebSocket support (alias for `ws-yawc`) |
| `ws-yawc` | No | WebSocket via hpx-yawc backend |
| `ws-fastwebsockets` | No | WebSocket via fastwebsockets backend |
| `socks` | No | SOCKS proxy support |
| `hickory-dns` | No | Async DNS resolver (Hickory) |
| `webpki-roots` | No | WebPKI root certificates for TLS |
| `system-proxy` | No | System proxy configuration |
| `tracing` | No | Tracing/logging support |
| `macros` | No | Tokio macros re-export |

## License

Apache-2.0
