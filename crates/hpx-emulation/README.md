# hpx-emulation

[![crates.io](https://img.shields.io/crates/v/hpx-emulation.svg)](https://crates.io/crates/hpx-emulation)
[![docs.rs](https://docs.rs/hpx-emulation/badge.svg)](https://docs.rs/hpx-emulation)
[![License](https://img.shields.io/crates/l/hpx-emulation.svg)](https://github.com/longcipher/hpx)

Common utilities for the [hpx](https://github.com/longcipher/hpx) HTTP client.

## Features

- **Browser Emulation**: TLS fingerprinting and HTTP/2 settings emulation for Chrome, Firefox, Safari, Opera, and OkHttp
- **Tower Middleware**: Delay and jitter layers for Tower services

## Quick Start

```rust
use hpx_emulation::Emulation;

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

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `emulation` | **Yes** | Browser emulation profiles (Chrome, Firefox, Safari, Opera, OkHttp) |
| `emulation-compression` | No | Compression settings for emulation profiles |
| `emulation-rand` | No | Random emulation profile selection |
| `emulation-serde` | No | Serde serialization for emulation types |
| `tower-delay` | No | Delay/jitter Tower middleware layer |

## Emulation Profiles

The `emulation` feature provides pre-configured browser fingerprint profiles:

- **Chrome**: Multiple versions with accurate TLS/HTTP2 settings
- **Firefox**: Firefox browser emulation
- **Safari**: Safari browser emulation
- **Opera**: Opera browser emulation
- **OkHttp**: OkHttp client emulation

## Tower Middleware

The `tower-delay` feature provides:

- `DelayLayer` — Fixed delay middleware
- `DelayLayerWith` — Configurable delay with jitter

## License

Apache-2.0
