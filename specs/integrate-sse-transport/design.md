# Design Document: Integrate SSE Transport

| Metadata | Details |
| :--- | :--- |
| **Author** | pb-plan agent |
| **Status** | Draft |
| **Created** | 2026-02-13 |
| **Reviewers** | — |
| **Related Issues** | N/A |

## 1. Executive Summary

**Problem:** `hpx-transport` currently only supports WebSocket as a streaming transport. Many cryptocurrency exchanges and APIs also expose Server-Sent Events (SSE) endpoints for real-time data (order book updates, trade streams, AI/LLM streaming responses). There is no unified SSE transport in `hpx-transport`, forcing consumers to manually wire up SSE parsing.

**Solution:** Integrate the `sseer` crate's SSE parsing capabilities into `hpx-transport` as a new `sse` module, following the same architectural patterns as the existing `websocket` module (Connection/Handle/Stream split, protocol handler trait, auto-reconnection, authentication integration). The SSE transport will use `hpx` (not `reqwest`) as its HTTP backend and leverage `sseer`'s core parsing primitives (`EventStream`, `Event`, retry policies).

---

## 2. Requirements & Goals

### 2.1 Problem Statement

- `hpx-transport` provides a robust WebSocket transport with connection management, protocol abstraction, and subscription handling, but has no equivalent for SSE.
- The `sseer` crate already lives in the workspace and provides excellent SSE parsing (`EventStream`, `Event`, `Utf8Stream`, `JsonStream`, retry policies), but its only high-level client integration is via `reqwest` (`sseer::reqwest::EventSource`).
- Users need an SSE transport that:
  - Uses `hpx::Client` (not `reqwest`) for HTTP, consistent with the rest of `hpx-transport`.
  - Provides connection management (auto-reconnect, `Last-Event-ID` tracking).
  - Integrates with `hpx-transport`'s authentication (`Authentication` trait).
  - Follows the same ergonomic patterns as the WebSocket module.

### 2.2 Functional Goals

1. **SSE Connection**: Establish and maintain SSE connections using `hpx::Client`, sending `Accept: text/event-stream` and validating `Content-Type: text/event-stream` responses.
2. **Event Parsing**: Reuse `sseer::EventStream` and `sseer::Event` for spec-compliant SSE parsing (BOM handling, multi-line data, comments, retry fields).
3. **Auto-Reconnection**: Implement reconnection with exponential backoff (reusing `sseer::retry` policies), including `Last-Event-ID` header on reconnect.
4. **Authentication Integration**: Support `hpx-transport::auth::Authentication` trait for signing SSE requests (headers, query parameters).
5. **Protocol Handler**: Provide an `SseProtocolHandler` trait for exchange-specific event classification and routing (analogous to `websocket::ProtocolHandler`).
6. **Connection/Handle/Stream Split**: Expose a split API similar to WebSocket — `SseConnection` spawns a background task, returns `SseHandle` (for control commands like close/reconnect) and `SseStream` (for consuming events).
7. **JSON Deserialization**: Optional typed JSON stream via `sseer::JsonStream` for automatic deserialization of SSE event data.
8. **Configuration**: `SseConfig` struct with URL, reconnection settings, timeouts, authentication flag, and custom headers.

### 2.3 Non-Functional Goals

- **Performance:** Zero-copy SSE parsing via `sseer`'s `Bytes`/`BytesMut` pipeline. No unnecessary allocations.
- **Reliability:** Spec-compliant reconnection with `Last-Event-ID`. Graceful handling of non-2xx responses and invalid content types.
- **Observability:** `tracing` spans/events for connection lifecycle, reconnection attempts, and errors.
- **Consistency:** API surface mirrors the WebSocket module where applicable, reducing cognitive load.

### 2.4 Out of Scope

- **Server-side SSE (sending events):** This is client-only.
- **HTTP/2 Server Push:** Out of scope; SSE is HTTP/1.1 chunked transfer or HTTP/2 streaming.
- **WebSocket-to-SSE bridging:** No automatic protocol fallback.
- **Modifying `sseer` core parsing:** Reuse as-is; only add an `hpx`-backend integration layer.

### 2.5 Assumptions

- `hpx::Client` supports streaming HTTP response bodies as `impl Stream<Item = Result<Bytes, hpx::Error>>` (or equivalent via `http_body_util::BodyDataStream`).
- The `sseer` crate's `EventStream<S>` is generic over any `Stream<Item = Result<B, E>> where B: AsRef<[u8]>`, so it can wrap `hpx`'s response body stream directly.
- SSE connections are read-only (server → client); the client only sends the initial HTTP request (with optional auth headers).
- Feature-gated: the SSE transport will be behind an `sse` feature flag in `hpx-transport`.

---

## 3. Architecture Overview

### 3.1 System Context

```text
                                        ┌──────────────────────┐
                                        │   hpx-transport      │
                                        │                      │
┌──────────────┐                        │  ┌────────────────┐  │
│ Exchange API │◄── HTTP GET (SSE) ─────│  │  sse module    │  │
│ (SSE)        │──── text/event-stream ─│  │                │  │
└──────────────┘                        │  │ SseConnection  │  │
                                        │  │ SseHandle      │  │
                                        │  │ SseStream      │  │
                                        │  │ SseConfig      │  │
                                        │  └───────┬────────┘  │
                                        │          │ uses      │
                                        │  ┌───────▼────────┐  │
                                        │  │ auth module    │  │
                                        │  │ error module   │  │
                                        │  │ hpx::Client    │  │
                                        │  └────────────────┘  │
                                        │          │ uses      │
                                        │  ┌───────▼────────┐  │
                                        │  │ sseer (core)   │  │
                                        │  │ EventStream    │  │
                                        │  │ Event, retry   │  │
                                        │  └────────────────┘  │
                                        └──────────────────────┘
```

The SSE module sits alongside the existing `websocket` module inside `hpx-transport`. It depends on:

- `sseer` (workspace crate) for SSE parsing primitives — **without** the `reqwest` feature.
- `hpx` for HTTP client functionality.
- `hpx-transport::auth` for request signing.
- `hpx-transport::error` for unified error types.

### 3.2 Key Design Principles

1. **Reuse `sseer` core, replace `reqwest` with `hpx`:** The `sseer::reqwest::EventSource` is the reference architecture, but we replace `reqwest` with `hpx::Client` for the HTTP layer.
2. **Mirror WebSocket module patterns:** `SseConfig` ↔ `WsConfig`, `SseConnection` ↔ `Connection`, `SseHandle` ↔ `ConnectionHandle`, `SseStream` ↔ `ConnectionStream`.
3. **Feature-gated:** All SSE code behind `sse` feature flag to avoid pulling in `sseer` dependencies for users who don't need SSE.
4. **Protocol handler abstraction:** `SseProtocolHandler` trait for exchange-specific event classification, similar to `websocket::ProtocolHandler` but simpler (SSE is read-only, no subscribe/unsubscribe messages).

### 3.3 Existing Components to Reuse

| Component | Location | How to Reuse |
| :--- | :--- | :--- |
| `sseer::EventStream` | `sseer/src/event_stream.rs` | Core SSE parser — wrap `hpx` response body stream with this |
| `sseer::Event` | `sseer/src/event.rs` | SSE event type — re-export as the canonical event type |
| `sseer::retry::{RetryPolicy, ExponentialBackoff}` | `sseer/src/retry.rs` | Reconnection strategy — use directly for SSE reconnect logic |
| `sseer::json_stream::JsonStream` | `sseer/src/json_stream.rs` | Optional typed JSON deserialization layer |
| `sseer::utf8_stream::Utf8Stream` | `sseer/src/utf8_stream.rs` | UTF-8 validation stream (if needed) |
| `hpx-transport::auth::Authentication` | `crates/hpx-transport/src/auth.rs` | Sign SSE HTTP requests with API keys / HMAC |
| `hpx-transport::error::{TransportError, TransportResult}` | `crates/hpx-transport/src/error.rs` | Unified error type — extend with SSE-specific variants |
| `hpx::Client` | `crates/hpx/` | HTTP client for making SSE GET requests |
| `WsConfig` pattern | `crates/hpx-transport/src/websocket/config.rs` | Builder pattern reference for `SseConfig` |
| `ConnectionState` enum | `crates/hpx-transport/src/websocket/connection.rs` | Reference for SSE connection state machine |

---

## 4. Detailed Design

### 4.1 Module Structure

```text
crates/hpx-transport/src/
├── sse/
│   ├── mod.rs              # Module root, re-exports
│   ├── config.rs           # SseConfig builder
│   ├── connection.rs       # SseConnection, SseHandle, SseStream, background task
│   ├── protocol.rs         # SseProtocolHandler trait
│   ├── handlers/
│   │   ├── mod.rs          # Handler re-exports
│   │   └── generic.rs      # GenericSseHandler (pass-through)
│   └── types.rs            # SseEvent, SseMessageKind, etc.
├── lib.rs                  # Add `pub mod sse;` behind feature gate
└── error.rs                # Add SSE error variants
```

### 4.2 Data Structures & Types

```rust
// sse/types.rs

/// Classification of SSE events (analogous to websocket::MessageKind).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SseMessageKind {
    /// A data event (has event type + data).
    Data,
    /// System/control event (heartbeat, ping comments).
    System,
    /// Retry directive from server.
    Retry,
    /// Unknown/unclassified event.
    Unknown,
}

/// Wrapper around sseer::Event with classification metadata.
#[derive(Clone, Debug)]
pub struct SseEvent {
    /// The raw SSE event from sseer.
    pub raw: sseer::event::Event,
    /// Classified message kind.
    pub kind: SseMessageKind,
}
```

```rust
// sse/config.rs

/// Configuration for SSE connections.
#[derive(Clone, Debug)]
pub struct SseConfig {
    /// SSE endpoint URL.
    pub url: String,
    /// HTTP method (usually GET, some APIs use POST).
    pub method: http::Method,
    /// Additional HTTP headers.
    pub headers: http::HeaderMap,
    /// Optional request body (for POST-based SSE).
    pub body: Option<Vec<u8>>,
    /// Connection timeout.
    pub connect_timeout: Duration,
    /// Initial reconnection delay.
    pub reconnect_initial_delay: Duration,
    /// Maximum reconnection delay.
    pub reconnect_max_delay: Duration,
    /// Backoff factor.
    pub reconnect_backoff_factor: f64,
    /// Maximum reconnection attempts (None = infinite).
    pub reconnect_max_attempts: Option<u32>,
    /// Whether to send auth headers.
    pub auth_on_connect: bool,
    /// Event channel capacity.
    pub event_channel_capacity: usize,
    /// Command channel capacity.
    pub command_channel_capacity: usize,
}
```

```rust
// sse/protocol.rs

/// Trait for exchange-specific SSE event handling.
pub trait SseProtocolHandler: Send + Sync + 'static {
    /// Classify an incoming SSE event.
    fn classify_event(&self, event: &sseer::event::Event) -> SseMessageKind;

    /// Called when connection is established (for logging, metrics, etc.).
    fn on_connect(&self) {}

    /// Called when connection is lost (for logging, metrics, etc.).
    fn on_disconnect(&self) {}

    /// Determine if the connection should be retried after this error.
    fn should_retry(&self, error: &TransportError) -> bool {
        true  // default: always retry
    }
}
```

```rust
// sse/connection.rs

/// SSE connection states.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SseConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting { attempt: u32 },
    Closed,
}

/// Control commands for SSE connection.
#[derive(Debug)]
pub enum SseCommand {
    Close,
    Reconnect { reason: String },
}

/// The SSE connection, analogous to websocket::Connection.
pub struct SseConnection { /* ... */ }

impl SseConnection {
    /// Connect and return a split handle + stream.
    pub async fn connect<H: SseProtocolHandler>(
        config: SseConfig,
        handler: H,
        client: hpx::Client,
    ) -> TransportResult<Self> { /* ... */ }

    /// Split into handle (control) + stream (events).
    pub fn split(self) -> (SseHandle, SseStream) { /* ... */ }
}

/// Handle for controlling the SSE connection (clone-able).
#[derive(Clone)]
pub struct SseHandle { /* ... */ }

impl SseHandle {
    /// Close the connection gracefully.
    pub async fn close(&self) -> TransportResult<()> { /* ... */ }

    /// Request a reconnection.
    pub async fn reconnect(&self, reason: &str) -> TransportResult<()> { /* ... */ }

    /// Get current connection state.
    pub fn state(&self) -> SseConnectionState { /* ... */ }
}

/// Stream of SSE events from the connection.
pub struct SseStream { /* ... */ }

impl Stream for SseStream {
    type Item = SseEvent;
    // ...
}
```

### 4.3 Interface Design

**Public API surface (`hpx_transport::sse`)**:

```rust
// Re-exports from sse/mod.rs
pub use config::SseConfig;
pub use connection::{SseConnection, SseConnectionState, SseHandle, SseStream};
pub use protocol::SseProtocolHandler;
pub use types::{SseEvent, SseMessageKind};
pub mod handlers;

// Re-export sseer core types for convenience
pub use sseer::event::Event as RawSseEvent;
```

**Usage example**:

```rust
use hpx_transport::sse::{SseConfig, SseConnection, SseEvent, handlers::GenericSseHandler};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = SseConfig::new("https://api.exchange.com/v1/stream")
        .connect_timeout(Duration::from_secs(10))
        .reconnect_max_attempts(Some(5));

    let handler = GenericSseHandler::new();
    let client = hpx::Client::builder().build()?;

    let connection = SseConnection::connect(config, handler, client).await?;
    let (handle, mut stream) = connection.split();

    while let Some(event) = stream.next().await {
        println!("Event type: {}, data: {}", event.raw.event, event.raw.data);
    }
    Ok(())
}
```

### 4.4 Logic Flow

**Connection lifecycle:**

```text
1. SseConnection::connect(config, handler, client)
   ├── Build HTTP request (GET/POST, headers, Accept: text/event-stream)
   ├── Apply auth if config.auth_on_connect
   ├── Send request via hpx::Client
   ├── Validate response (2xx, Content-Type: text/event-stream)
   └── Wrap response body in sseer::EventStream

2. connection.split() → (SseHandle, SseStream)
   └── Spawns background task (tokio::spawn)

3. Background task loop:
   ├── select! {
   │     event = event_stream.next() => {
   │         classify via handler.classify_event()
   │         send to SseStream channel
   │     }
   │     cmd = command_rx.recv() => {
   │         handle Close / Reconnect
   │     }
   │ }
   └── On disconnect:
       ├── handler.on_disconnect()
       ├── Apply retry policy (ExponentialBackoff)
       ├── Set Last-Event-ID header
       └── Re-establish connection (goto step 1 internally)
```

### 4.5 Configuration

New feature flag in `crates/hpx-transport/Cargo.toml`:

```toml
[features]
sse = ["dep:sseer"]

[dependencies]
sseer = { workspace = true, optional = true }
```

New workspace dependency in root `Cargo.toml`:

```toml
[workspace.dependencies]
sseer = { path = "sseer" }
```

### 4.6 Error Handling

Extend `TransportError` with SSE-specific variants:

```rust
// In error.rs
pub enum TransportError {
    // ... existing variants ...

    /// SSE connection received a non-2xx status code.
    #[error("SSE error: invalid status {status}")]
    SseInvalidStatus { status: http::StatusCode },

    /// SSE connection received invalid Content-Type.
    #[error("SSE error: invalid content-type '{content_type}'")]
    SseInvalidContentType { content_type: String },

    /// SSE event stream parsing error.
    #[error("SSE parse error: {message}")]
    SseParse { message: String },

    /// SSE stream ended unexpectedly.
    #[error("SSE stream ended")]
    SseStreamEnded,
}
```

---

## 5. Verification & Testing Strategy

### 5.1 Unit Testing

- **Config builder:** Verify `SseConfig` builder methods produce correct values.
- **Protocol handler:** Test `GenericSseHandler` classification logic.
- **Event types:** Test `SseEvent` construction and `SseMessageKind` matching.
- **Error variants:** Test SSE-specific error creation and display.

### 5.2 Integration Testing

- **Mock SSE server:** Use `hyper` (already a dev-dependency) to create a local SSE server that emits events. Test:
  - Connection establishment and event parsing.
  - Reconnection with `Last-Event-ID`.
  - Auth header propagation.
  - Non-2xx response handling.
  - Content-Type validation.
- **JSON deserialization:** Test typed JSON stream with mock SSE data.

### 5.3 Validation Rules

| Test Case ID | Action | Expected Outcome | Verification Method |
| :--- | :--- | :--- | :--- |
| **TC-01** | Connect to SSE endpoint | Receives `SseEvent` items via `SseStream` | Integration test with mock server |
| **TC-02** | Server sends non-2xx | `TransportError::SseInvalidStatus` returned | Unit test |
| **TC-03** | Server sends wrong Content-Type | `TransportError::SseInvalidContentType` returned | Unit test |
| **TC-04** | Server disconnects | Auto-reconnect with backoff, `Last-Event-ID` sent | Integration test with mock server |
| **TC-05** | `SseHandle::close()` | Stream terminates cleanly | Integration test |
| **TC-06** | Auth headers applied | Request contains signed headers | Integration test inspecting mock server logs |
| **TC-07** | JSON SSE events | Typed deserialization succeeds | Unit test with `JsonStream` |
| **TC-08** | Max reconnect exceeded | `TransportError::MaxReconnectAttempts` | Unit test |

---

## 6. Implementation Plan

- [ ] **Phase 1: Foundation** — Add `sseer` workspace dep, feature flag, module scaffolding, `SseConfig`, types, error variants
- [ ] **Phase 2: Core Logic** — Implement `SseConnection` background task, event parsing pipeline with `hpx::Client` + `sseer::EventStream`, auto-reconnect logic
- [ ] **Phase 3: Integration** — `SseProtocolHandler` trait, `GenericSseHandler`, `SseHandle`/`SseStream` split API, auth integration
- [ ] **Phase 4: Polish** — Tests (unit + integration with mock server), documentation, examples, CI

---

## 7. Cross-Functional Concerns

- **Backward Compatibility:** The `sse` feature is opt-in. Existing code is unaffected when the feature is not enabled. The `default` feature list does not include `sse`.
- **Dependency Size:** `sseer` is lightweight (`bytes`, `memchr`, `pin-project-lite`, `futures-core`). No heavy transitive dependencies added.
- **Observability:** SSE connection lifecycle events logged via `tracing` at appropriate levels (`info` for connect/disconnect, `warn` for reconnect, `debug` for events).
- **Security:** Auth headers are only sent when `config.auth_on_connect` is true. Sensitive headers are not logged at `debug` level.
