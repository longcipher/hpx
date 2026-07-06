# hpx

[![DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/longcipher/hpx)
[![Context7](https://img.shields.io/badge/Website-context7.com-blue)](https://context7.com/longcipher/hpx)
[![crates.io](https://img.shields.io/crates/v/hpx.svg)](https://crates.io/crates/hpx)
[![docs.rs](https://docs.rs/hpx/badge.svg)](https://docs.rs/hpx)

![hpx](https://socialify.git.ci/longcipher/hpx/image?font=Source+Code+Pro&language=1&name=1&owner=1&pattern=Circuit+Board&theme=Auto)

A high-performance HTTP client workspace for crypto exchange HFT applications. Forked from [wreq](https://github.com/0x676e67/wreq) / [reqwest](https://github.com/seanmonstar/reqwest), optimized for low-latency network I/O, browser emulation, and automated scraping.

## Workspace Structure

```text
hpx/
├── bin/
│   └── hpx-cli/          # CLI binary — HTTP client, scraper, download manager, CDP server
├── crates/
│   ├── hpx/              # Core HTTP client library (TLS, HTTP/1+2, pooling, middleware)
│   ├── hpx-emulation/    # Browser fingerprint profiles (JA3/JA4, HTTP/2 settings)
│   ├── hpx-browser/      # Headless browser engine (DOM, CSS, layout, JS, challenge detection)
│   ├── hpx-dl/           # Segmented download engine (resume, queue, persistence)
│   ├── hpx-streams/      # Streaming response codecs (JSON/CSV/Protobuf/Arrow)
│   └── yawc/             # WebSocket client/server (RFC 6455, compression)
└── Justfile              # Task runner (fmt, lint, test, ci)
```

## Crates

### [`hpx`](https://crates.io/crates/hpx) — Core HTTP Client

The foundation crate. Everything else builds on top of this.

**Provides:** `Client`, `ClientBuilder`, `RequestBuilder`, `Response`, `Body`, WebSocket upgrade, cookie store, redirect policy, Tower middleware stack, TLS backends (BoringSSL / OpenSSL / Rustls).

**Use when:** You need to make HTTP requests — GET, POST, JSON APIs, file uploads, WebSocket connections, or reverse proxy traffic.

```rust
// Minimal
let body = hpx::get("https://api.example.com").send().await?.text().await?;

// Full control
let client = hpx::Client::builder()
    .emulation(BrowserProfile::Chrome)
    .proxy("socks5://127.0.0.1:1080")
    .cookie_provider(Jar::default())
    .build()?;
```

**Key feature flags:** `json`, `ws`, `cookies`, `gzip`/`brotli`/`zstd`, `socks`, `hickory-dns`, `openssl-tls`/`openssl-vendored`, `hft` (preset), `stealth` (preset).

---

### [`hpx-emulation`](https://crates.io/crates/hpx-emulation) — Browser Fingerprint Profiles

Companion crate for `hpx`. Provides ready-made browser emulation profiles that configure TLS ciphersuites, HTTP/2 settings, and default headers to match real browsers.

**Provides:** `Emulation`, `EmulationOption`, `BrowserProfile` (Chrome, Firefox, Safari, Edge, OkHttp), `EmulationOS`, fingerprint diffing.

**Use when:** You need to impersonate a specific browser for TLS fingerprint checks (JA3/JA4), or when scraping sites that inspect HTTP/2 SETTINGS frames.

```rust
use hpx_emulation::Emulation;

let resp = hpx::get("https://tls.peet.ws/api/all")
    .emulation(Emulation::Firefox136)
    .send()
    .await?;
```

**Depends on:** `hpx` (provides the `EmulationFactory` trait that `hpx::RequestBuilder::emulation()` accepts).

---

### [`hpx-browser`](https://crates.io/crates/hpx-browser) — Headless Browser Engine

A lightweight headless browser built on `hpx`. Parses HTML/CSS, builds a DOM, runs JavaScript (via V8/Deno), detects anti-bot challenges, and renders pages to text/markdown/screenshots.

**Provides:** HTML parser, DOM tree, CSS layout engine (Stylo + Taffy via Blitz), JS runtime (Deno_core), challenge detection (Cloudflare, AWS-WAF, Kasada, PerimeterX, DataDome, hCaptcha, reCAPTCHA), CDP protocol support, parallel scraping, stealth mode, canvas rendering.

**Use when:** You need to render JavaScript-heavy pages, bypass anti-bot protections, scrape SPAs, or run a headless browser programmatically.

**Key modules:**

| Module | Purpose |
|--------|---------|
| `challenge` | Anti-bot challenge classifier (CF, AWS-WAF, Kasada, etc.) |
| `html_parser` | DOM construction (Blitz/html5ever) |
| `dom` / `layout` | DOM tree + CSS layout (Stylo + Taffy via Blitz) |
| `js_runtime` | V8-based JavaScript execution (feature `v8`) |
| `parallel` | Multi-URL concurrent scraping |
| `stealth` | Anti-fingerprinting patches |
| `protocol` | CDP (Chrome DevTools Protocol) server |

**Depends on:** `hpx` (network), `deno_core` (JS), `blitz-html`/`blitz-dom` (HTML/CSS), `parley` (text), `image`/`skia-safe` (canvas, optional).

---

### [`hpx-dl`](https://crates.io/crates/hpx-dl) — Download Engine

Segmented HTTP download engine with resume, priority queue, speed limiting, and SQLite persistence.

**Provides:** `DownloadEngine`, `EngineBuilder`, `SegmentDownloader`, `PriorityQueue`, `SqliteStorage`, checksum verification (SHA-256/384/512, MD5), metalink parsing, event broadcasting.

**Use when:** You need to download large files with pause/resume, parallel segments, integrity verification, or managed download queues.

```rust
use hpx_dl::{DownloadEngine, EngineConfig};

let engine = DownloadEngine::new(EngineConfig::default());
let id = engine.add("https://example.com/large-file.bin").await?;
engine.start(id).await?;
```

**Key features:** `http` (enable HTTP client integration), `sqlite` (persistent storage), `test` (in-memory storage for tests), `metalink` (metalink parsing), `hotpath` (profiling instrumentation).

**Depends on:** `hpx` (HTTP client for segments), `sqlx` (persistence), `ahash` (concurrent data structures).

---

### [`hpx-streams`](https://crates.io/crates/hpx-streams) — Streaming Response Codecs

Extension trait for `hpx::Response` that adds streaming decode for structured response formats.

**Provides:** `JsonStreamResponse`, `CsvStreamResponse`, `ProtobufStreamResponse`, `ArrowIpcStreamResponse` — each adds a `.xxx_stream()` method to `hpx::Response`.

**Use when:** You're consuming large paginated APIs, database dumps, or event streams that return JSON arrays, CSV, Protobuf, or Arrow IPC.

```rust
use hpx_streams::JsonStreamResponse as _;

let mut stream = client
    .get("http://localhost:8080/large-json-array")
    .send()
    .await?
    .json_array_stream::<MyItem>(1024);
```

**Depends on:** `hpx` (the `Response` type), `serde_json`/`csv`/`prost`/`arrow-ipc` (per feature).

**Key features:** `json` (JSON array streaming), `csv` (CSV streaming), `protobuf` (Protobuf streaming), `arrow` (Arrow IPC streaming).

---

### [`hpx-yawc`](https://crates.io/crates/hpx-yawc) (yawc) — WebSocket

RFC 6455 WebSocket implementation with permessage-deflate compression. Can be used standalone or as the WebSocket backend for `hpx`.

**Provides:** `WebSocket` client/server, frame codec, masking, compression (zlib/deflate), Axum integration, SIMD UTF-8 validation.

**Use when:** You need a standalone WebSocket client or server, or when you want the `ws` feature in `hpx` (this is the default backend).

```rust
use futures::{SinkExt, StreamExt};
use hpx_yawc::WebSocket;

let mut ws = WebSocket::connect("wss://echo.websocket.org".parse()?).await?;
ws.send(hpx_yawc::Frame::text("hello")).await?;
```

**Depends on:** `tokio`, `rustls` (TLS), optional `axum` integration.

---

## CLI: `hpx`

The `hpx-cli` binary (`bin/hpx-cli/`) is a multi-tool HTTP client that ties all the crates together.

### Direct HTTP Requests

```bash
# Simple GET
hpx https://api.example.com/data

# POST with JSON body
hpx -X POST https://api.example.com/data -j '{"key":"value"}'

# With headers and auth
hpx -H 'Authorization: Bearer TOKEN' https://api.example.com/protected

# Form data
hpx -X POST https://httpbin.org/post -f 'name=test' -f 'email=test@example.com'

# Save response to file
hpx https://example.com/file.zip -o file.zip

# Follow redirects + timing
hpx -L -T https://httpbin.org/redirect/3

# WebSocket
hpx wss://echo.websocket.org

# Proxy
hpx --proxy socks5://127.0.0.1:1080 https://api.example.com
```

### Browser Commands

```bash
# Fetch and render a page (JS execution, challenge bypass)
hpx fetch https://example.com --dump html

# Dump rendered text
hpx fetch https://example.com --dump text

# Dump all links
hpx fetch https://example.com --dump links

# Wait for a CSS selector
hpx fetch https://example.com --selector '#content' --wait 10

# Evaluate JS on the page
hpx fetch https://example.com -e 'document.title'

# Scrape multiple URLs in parallel
hpx scrape https://a.com https://b.com --concurrency 20

# Start a CDP (Chrome DevTools Protocol) server
hpx serve --port 9222
```

### Download Manager

```bash
# Add a download
hpx dl add https://example.com/large.bin -o ./large.bin

# Add with speed limit, checksum, mirrors
hpx dl add https://example.com/file.bin \
  --speed-limit 1MB/s \
  --checksum sha256:abc123... \
  --mirror https://mirror1.com/file.bin \
  --max-connections 8

# Pause / resume / remove
hpx dl pause <id>
hpx dl resume <id>
hpx dl remove <id>

# List all downloads
hpx dl list

# Check status
hpx dl status <id>
```

### Proxy Testing

```bash
# Run proxy integration smoke tests
hpx proxy-test --proxy http://127.0.0.1:7890
```

## Crate Dependency Graph

```text
hpx-cli (binary)
  ├── hpx            (HTTP client)
  ├── hpx-emulation  (browser profiles)
  ├── hpx-browser    (headless browser, challenge detection)
  ├── hpx-dl         (download engine)
  └── hpx-streams    (streaming codecs)

hpx (core)
  ├── hpx-emulation  (optional, via emulation feature)
  └── hpx-yawc       (optional, via ws feature)

hpx-dl
  └── hpx            (HTTP client for segments)

hpx-streams
  └── hpx            (extends Response with streaming methods)

hpx-browser
  └── hpx            (network layer)
```

**Standalone usage:** Each crate can be used independently. `hpx-yawc` works without `hpx` for raw WebSocket connections. `hpx-dl` can be used with its own `EngineBuilder` without the CLI.

## Feature Flags

### `hpx` Features

Default: `boring`, `http1`, `http2`, `stream`, `tracing`.

| Feature | Default | Description |
|---------|---------|-------------|
| **TLS** | | |
| `boring` | **Yes** | BoringSSL TLS backend |
| `rustls-tls` | No | Rustls TLS backend (pure Rust) |
| `openssl-tls` | No | OpenSSL TLS backend |
| `openssl-vendored` | No | OpenSSL with vendored static linking |
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
| **Networking** | | |
| `cookies` | No | Cookie store support |
| `socks` | No | SOCKS proxy support |
| `hickory-dns` | No | Async DNS resolver (Hickory) |
| `system-proxy` | No | System proxy configuration |
| **Auth** | | |
| `auth` | No | Authentication middleware (bearer, API key, OAuth2) |
| **Observability** | | |
| `tracing` | No | Tracing/logging support |
| `hotpath` | No | Hotpath profiling instrumentation |
| **Streaming** | | |
| `sse` | No | Server-Sent Events support |
| **Presets** | | |
| `hft` | No | Low-latency: auth, BoringSSL, HTTP/1+2, streaming, tracing, Hickory DNS, SIMD JSON, Zstd, yawc WS |
| `stealth` | No | Browser-like: auth, BoringSSL, HTTP/1+2, decompression, cookies, charset, query, streaming, tracing, Hickory DNS, yawc WS |

### Common Feature Combinations

```toml
# Minimal HTTP client
hpx = "2"

# JSON API client
hpx = { version = "2", features = ["json", "cookies", "gzip"] }

# WebSocket client
hpx = { version = "2", features = ["ws"] }

# High-performance trading
hpx = { version = "2", default-features = false, features = ["hft"] }

# Browser-like scraping
hpx = { version = "2", default-features = false, features = ["stealth"] }

# Pure Rust (no C dependencies)
hpx = { version = "2", default-features = false, features = ["rustls-tls", "http1", "http2"] }

# OpenSSL backend
hpx = { version = "2", default-features = false, features = ["openssl-tls", "http1", "http2"] }

# OpenSSL vendored (static linking, no system OpenSSL needed)
hpx = { version = "2", default-features = false, features = ["openssl-vendored", "http1", "http2"] }
```

### `hpx-emulation` Features

Default: `emulation`.

| Feature | Default | Description |
|---------|---------|-------------|
| `emulation` | **Yes** | Browser emulation profiles (Chrome, Firefox, Safari, Opera, OkHttp) |
| `emulation-compression` | No | Compression settings for emulation profiles |
| `emulation-rand` | No | Random emulation profile selection |
| `emulation-serde` | No | Serde serialization for emulation types |

### `hpx-yawc` Features

Default: `rustls-ring`.

| Feature | Default | Description |
|---------|---------|-------------|
| `rustls-ring` | **Yes** | TLS via rustls with ring crypto |
| `rustls-aws-lc-rs` | No | TLS via rustls with AWS LC crypto |
| `axum` | No | Axum WebSocket extractor |
| `proxy` | No | Proxy support |
| `socks` | No | SOCKS proxy support (enables `proxy`) |
| `simd` | No | SIMD-accelerated UTF-8 validation |
| `hotpath` | No | Hotpath profiling instrumentation |
| `smol` | No | smol async runtime support |
| `zlib` | No | Native zlib compression backend |

## TLS Backend Configuration

hpx supports three TLS backends — BoringSSL (default), OpenSSL, and Rustls. Each can be selected via feature flags.

### BoringSSL (Default)

BoringSSL is the default TLS backend, providing robust support for modern TLS features and extensive browser emulation capabilities. Recommended for most use cases, especially when browser fingerprinting is required.

```toml
[dependencies]
hpx = "2"
```

### OpenSSL

A widely-used C library TLS backend. Useful when you need OpenSSL-specific features like hardware acceleration (QAT, AES-NI), PKCS#11 engine support, or when BoringSSL is not available on your platform.

```toml
[dependencies]
hpx = { version = "2", default-features = false, features = ["openssl-tls", "http1", "http2"] }

# Vendored (statically linked) OpenSSL — no system dependency needed
hpx = { version = "2", default-features = false, features = ["openssl-vendored", "http1", "http2"] }
```

### Rustls

A pure Rust TLS implementation. Useful for environments where C dependencies are difficult to manage or when you prefer the safety guarantees of a pure Rust stack.

> **Note:** Switching to Rustls may affect the availability or behavior of certain browser emulation features that rely on specific BoringSSL capabilities.

```toml
[dependencies]
hpx = { version = "2", default-features = false, features = ["rustls-tls", "http1", "http2"] }
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

### Streaming / Proxying / Gateway

Requires the `stream` feature. For reverse proxies and API gateways, `hpx` can forward framework-native request bodies without buffering the full payload.

```rust
use axum::{
    body::Body as AxumBody,
    extract::{Request as AxumRequest, State},
    http::{Response as AxumResponse, StatusCode},
};
use hpx::{Body, Client, Request};

async fn proxy(
    State(client): State<Client>,
    mut req: AxumRequest<AxumBody>,
) -> Result<AxumResponse<Body>, StatusCode> {
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");

    let upstream_uri = format!("https://upstream.internal{path_and_query}")
        .parse()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    *req.uri_mut() = upstream_uri;

    let upstream_req = Request::from_http(req);
    let upstream_res = client
        .execute(upstream_req)
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    Ok(http::Response::<Body>::from(upstream_res))
}
```

### Browser Emulation

Requires the `hpx-emulation` crate with default features.

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

## Performance Optimization Tips

- **`simd-json`**: Replaces `serde_json` with SIMD-accelerated JSON parsing
- **`hickory-dns`**: High-performance async DNS resolver, avoids blocking system calls
- **`zstd`**: Fastest compression/decompression ratio for most workloads
- **Lock-free internals**: Uses `scc` concurrent containers and `arc-swap` for hot-path data

## License

Apache-2.0
