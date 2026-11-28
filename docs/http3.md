# HTTP/3 (QUIC) Support

hpx supports HTTP/3 via the `http3` Cargo feature flag. HTTP/3 runs over QUIC (RFC 9000) instead of TCP, providing multiplexed streams without head-of-line blocking, faster connection establishment, and built-in encryption.

## Feature Flag

Enable HTTP/3 in your `Cargo.toml`:

```toml
[dependencies]
hpx = { version = "...", features = ["http3"] }
```

Or via the command line:

```sh
cargo build --features http3
```

## Quick Start

```rust
use hpx::Client;

#[tokio::main]
async fn main() -> hpx::Result<()> {
    let client = Client::builder()
        .http3_only()
        .build()?;

    let resp = client
        .get("https://cloudflare-quic.com/")
        .send()
        .await?;

    println!("Status: {}", resp.status());
    println!("Body: {}", resp.text().await?);
    Ok(())
}
```

## Builder API

The `ClientBuilder` exposes four HTTP/3 configuration methods:

### `http3_only()`

Forces HTTP/3 only — no fallback to HTTP/2 or HTTP/1 over TCP. Sets the version preference to `Http3`. The client will not negotiate TCP ALPN when this is set; h3 is QUIC-only.

```rust
Client::builder().http3_only().build()?;
```

### `http3_prior_knowledge()`

Alias of `http3_only()`. Provided for parity with the reqwest API. HTTP/3 prior knowledge means the client assumes the server supports HTTP/3 without discovering it via Alt-Svc (RFC 7838). See RFC 9114 §3.1.1 for the formal definition.

```rust
Client::builder().http3_prior_knowledge().build()?;
```

### `http3_options(opts: impl Into<Http3Options>)`

Configures HTTP/3 protocol and QUIC transport parameters. Accepts `Http3Options` or any type that converts into it. When not called, `Client::build` falls back to `Http3Options::default()` (Chrome 143 baseline).

```rust
use hpx::http3::Http3Options;

let opts = Http3Options {
    max_idle_timeout: Some(Duration::from_secs(60)),
    ..Default::default()
};

Client::builder()
    .http3_only()
    .http3_options(opts)
    .build()?;
```

### `quic_config(cfg: QuicConfig)`

Low-level escape hatch to override the `quinn::TransportConfig` directly. Takes precedence over the transport config derived from `http3_options`. Use this for fine-grained control over QUIC parameters not exposed via `Http3Options`.

```rust
use hpx::http3::QuicConfig;

let mut quic = QuicConfig::default();
quic.max_idle_timeout(Some(quinn::VarInt::from_u32(60_000)));

Client::builder()
    .http3_only()
    .quic_config(quic)
    .build()?;
```

## `Http3Options` Struct

`Http3Options` defines QUIC transport parameters, HTTP/3 protocol parameters, and fingerprint hooks. All fields are `Option`-typed where `None` means "do not send the parameter" and `Some(v)` means "advertise `v` to the peer". The `Default` implementation matches the Chrome 143 baseline.

```rust
#[non_exhaustive]
pub struct Http3Options {
    // === QUIC transport parameters (RFC 9000 §18.2) ===

    /// QUIC `max_idle_timeout` transport parameter.
    /// Default: `Some(Duration::from_secs(30))`
    pub max_idle_timeout: Option<Duration>,

    /// Per-stream flow-control receive window.
    /// Default: `Some(8 * 1024 * 1024)` (8 MiB)
    pub stream_receive_window: Option<u64>,

    /// Connection-level flow-control receive window.
    /// Default: `Some(8 * 1024 * 1024)` (8 MiB)
    pub conn_receive_window: Option<u64>,

    /// Connection-level flow-control send window.
    /// Default: `Some(8 * 1024 * 1024)` (8 MiB)
    pub send_window: Option<u64>,

    /// Whether to use BBR congestion control. `false` selects CUBIC.
    /// Default: `false`
    pub congestion_bbr: bool,

    /// Maximum concurrent bidirectional streams the peer may initiate.
    /// Default: `Some(100)`
    pub max_concurrent_bidi_streams: Option<u64>,

    /// Maximum concurrent unidirectional streams the peer may initiate.
    /// Default: `Some(100)`
    pub max_concurrent_uni_streams: Option<u64>,

    /// Aggregate connection flow-control limit.
    /// Default: `Some(10 * 1024 * 1024)` (10 MiB)
    pub initial_max_data: Option<u64>,

    /// Max stream data for bidirectional local streams.
    /// Default: `Some(8 * 1024 * 1024)` (8 MiB)
    pub initial_max_stream_data_bidi_local: Option<u64>,

    /// Max stream data for bidirectional remote streams.
    /// Default: `Some(8 * 1024 * 1024)` (8 MiB)
    pub initial_max_stream_data_bidi_remote: Option<u64>,

    /// Max stream data for unidirectional streams.
    /// Default: `Some(8 * 1024 * 1024)` (8 MiB)
    pub initial_max_stream_data_uni: Option<u64>,

    /// Active connection ID limit (RFC 9000 §18.2).
    /// Default: `Some(8)`
    pub active_connection_id_limit: Option<u64>,

    /// Whether to enable 0-RTT (early data).
    /// Default: `true`
    pub enable_0rtt: bool,

    // === HTTP/3 protocol parameters ===

    /// `MAX_FIELD_SECTION_SIZE` (RFC 9114 §4.2.2). Bounds the size of header blocks.
    /// Default: `Some(16 * 1024)` (16 KiB)
    pub max_field_section_size: Option<u64>,

    /// Whether to send RFC 8701 GREASE in the SETTINGS frame.
    /// Default: `true`
    pub send_grease: bool,

    /// `QPACK_MAX_TABLE_CAPACITY` (RFC 9204 §3.2.1).
    /// Default: `Some(4096)`
    pub qpack_max_table_capacity: Option<u64>,

    /// `QPACK_BLOCKED_STREAMS` (RFC 9204 §3.2.2).
    /// Default: `Some(100)`
    pub qpack_blocked_streams: Option<u64>,

    // === Fingerprint hooks ===

    /// Arbitrary `(setting_identifier, value)` pairs appended to the h3 SETTINGS frame.
    /// Default: empty
    pub experimental_settings: Vec<(u64, u64)>,

    /// Explicit ordering of h3 SETTINGS entries for fingerprint control.
    /// Default: `[QpackMaxTableCapacity, MaxFieldSectionSize, QpackBlockedStreams]`
    pub settings_order: Vec<H3SettingId>,

    // === Extended CONNECT (RFC 9220) ===

    /// Whether to enable the extended CONNECT protocol for WebSocket-over-h3.
    /// Default: `false` (opt-in; Phase 2 scope)
    pub enable_connect_protocol: bool,
}
```

## `H3Error` Enum

`H3Error` is a typed error enum for HTTP/3 connector errors. Each variant maps to an `hpx::Error` kind and its corresponding `is_*` predicate:

| Variant                        | `Error` Kind | `is_*` Predicate  | Description                                        |
| :---                           | :---         | :---              | :---                                               |
| `Handshake { source }`         | `Connect`    | `is_connect()`    | QUIC handshake failed. Carries `quinn::ConnectionError`. |
| `Framing { source }`           | `Request`    | `is_request()`    | HTTP/3 framing or protocol error from the h3 layer. |
| `StreamReset { code, id }`     | `Body`       | `is_body()`       | A QUIC stream was reset with an application error code. |
| `IdleClose`                    | `Connect`    | `is_connect()`    | The connection was closed while idle.              |
| `ZeroRttRejected`              | `Connect`    | `is_connect()`    | 0-RTT early data was rejected; retry at 1-RTT.     |
| `MigrationFailed`              | `Connect`    | `is_connect()`    | Connection migration failed (e.g., NAT rebinding).  |
| `VersionNegotiationFailed`     | `Connect`    | `is_connect()`    | QUIC version negotiation failed — no common version. |
| `FlowControl`                  | `Body`       | `is_body()`       | HTTP/3 flow-control limit exceeded.                |
| `MaxConcurrentStreamsExceeded` | `Request`    | `is_request()`    | Peer's `SETTINGS_MAX_CONCURRENT_STREAMS` limit exceeded. |
| `GoAwayDrained`                | `Connect`    | `is_connect()`    | The connection was drained after receiving a GOAWAY frame. |
| `AltSvcUnreachable`            | `Connect`    | `is_connect()`    | The Alt-Svc alternative endpoint was unreachable.  |
| `Other(err)`                   | `Request`    | `is_request()`    | Catch-all for DNS, QUIC connect, and other I/O errors. |

`H3Error` implements `From<H3Error> for hpx::Error`, so it can be propagated with the `?` operator in code that returns `hpx::Result<T>`.

## Architecture Overview

The HTTP/3 stack is composed of three layers:

```text
┌─────────────────────────────┐
│  h3 crate (HTTP/3 framing)  │  ← RFC 9114: SETTINGS, HEADERS, DATA frames
├─────────────────────────────┤
│  h3-quinn (bridge)          │  ← adapts h3 to quinn's QUIC API
├─────────────────────────────┤
│  quinn (QUIC transport)     │  ← RFC 9000: streams, reliability, congestion
└─────────────────────────────┘
```

- **quinn**: Pure-Rust QUIC implementation. Handles connection establishment, stream multiplexing, loss detection, and congestion control.
- **h3-quinn**: Bridge crate that adapts the `h3` crate's connection abstraction to `quinn`'s API.
- **h3**: HTTP/3 framing layer. Manages request/response streams, QPACK header compression, and SETTINGS frame negotiation.

### Connection Flow

1. `ClientBuilder::http3_only()` sets `HttpVersionPref::Http3`.
2. `Client::build()` constructs a `QuicConnector` from `Http3Options` / `QuicConfig`.
3. On each request, `QuicConnector::call()` performs a QUIC handshake with the server.
4. Once connected, `h3::client::Connection` is established over the QUIC connection.
5. HTTP/3 requests are sent as `HEADERS` + `DATA` frames on bidirectional QUIC streams.
6. Responses arrive as `HEADERS` + `DATA` frames, decoded through QPACK.

## TLS Backend

hpx uses a **dual TLS backend** strategy (Option C):

- **HTTP/3 (QUIC)**: Uses `rustls` via `quinn`. `quinn` requires `rustls` for its TLS 1.3 layer — QUIC mandates TLS 1.3.
- **HTTP/1 and HTTP/2 (TCP)**: Uses `BoringSSL` for JA3/JA4 TLS fingerprint fidelity and HTTP/2 frame-level emulation.

This means the `http3` feature pulls in `rustls` and `quinn` as additional dependencies, while the existing `boring` feature continues to serve TCP-based TLS.

## Known Limitations

- **No connection migration**: QUIC connection migration (RFC 9000 §9) is not supported. If the client's network path changes, the connection will fail.
- **QUIC transport parameters**: `Http3Options` and `QuicConfig` expose HTTP/3 SETTINGS ordering, GREASE settings and a `quinn::TransportConfig` escape hatch. The QUIC ClientHello transport parameters, the initial packet number and the DCID are still chosen by `quinn` and are not byte-for-byte matched to a browser profile.
- **0-RTT**: `QuicConnector` calls `into_0rtt()`, tracks `used_0rtt` and surfaces `H3Error::ZeroRttRejected`. Replay safety for non-idempotent requests remains the caller's responsibility.
- **HTTP/1.1 Alt-Svc**: `AltSvcCache` and `prefer_http3()` implement h2-to-h3 discovery; opportunistic upgrade from an h1-only connection is not wired.

Alt-Svc discovery, WebSocket over HTTP/3 (RFC 9220, `H3WebSocket`) and Extended CONNECT are implemented. The vendored `hpx-h3` fork defines WebTransport stream identifiers, but no WebTransport session-establishment API is exposed yet. Earlier revisions of this document listed these as missing.

## References

- [RFC 9114 — HTTP/3](https://www.rfc-editor.org/rfc/rfc9114.html)
- [RFC 9000 — QUIC: A UDP-Based Multiplexed and Secure Transport](https://www.rfc-editor.org/rfc/rfc9000.html)
- [RFC 9001 — Using TLS to Secure QUIC](https://www.rfc-editor.org/rfc/rfc9001.html)
- [RFC 9204 — QPACK: Field Compression for HTTP/3](https://www.rfc-editor.org/rfc/rfc9204.html)
- [RFC 7838 — HTTP Alternative Services (Alt-Svc)](https://www.rfc-editor.org/rfc/rfc7838.html)
- [RFC 9220 — Bootstrapping WebSockets with HTTP/3](https://www.rfc-editor.org/rfc/rfc9220.html)
- [RFC 8701 — Applying Generate Random Extensions And Sustain Extensibility (GREASE) to TLS Extensibility](https://www.rfc-editor.org/rfc/rfc8701.html)
