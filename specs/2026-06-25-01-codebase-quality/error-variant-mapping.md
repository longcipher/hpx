# Error Type Variant Mapping

Analysis for Task 6.1 — mapping all variants across the three error hierarchies.

## Error Type 1: `hpx::Error` (`crates/hpx/src/error.rs`, ~476 lines)

| Variant | Payload | Notes |
|---------|---------|-------|
| `Kind::Builder` | — | Client builder errors |
| `Kind::Request` | — | Request sending errors |
| `Kind::Tls` | — | TLS handshake/connection errors |
| `Kind::Redirect` | — | Redirect following errors |
| `Kind::Status` | `StatusCode, Option<ReasonPhrase>` | HTTP status code errors |
| `Kind::Body` | — | Body errors |
| `Kind::Decode` | — | Response body decode errors |
| `Kind::Upgrade` | — | Connection upgrade errors |
| `Kind::WebSocket` | — | WebSocket errors (feature-gated `ws-yawc`) |

Sentinel types: `TimedOut`, `BadScheme`, `ProxyConnect(BoxError)`

## Error Type 2: `core::Error` (`crates/hpx/src/client/core/error.rs`, ~438 lines)

| Variant | Payload | Notes |
|---------|---------|-------|
| `Kind::Parse(Parse)` | `Parse` enum | HTTP parsing errors |
| `Kind::User(User)` | `User` enum | User code errors |
| `Kind::IncompleteMessage` | — | EOF before message complete |
| `Kind::UnexpectedMessage` | — | Message when not expected |
| `Kind::Canceled` | — | Request canceled |
| `Kind::ChannelClosed` | — | Channel closed |
| `Kind::Io` | — | I/O errors |
| `Kind::Body` | — | Body read errors |
| `Kind::BodyWrite` | — | Body write errors |
| `Kind::Shutdown` | — | AsyncWrite shutdown errors |
| `Kind::Http2` | — | h2 protocol errors |

Parse sub-variants: `Method`, `Version`, `VersionH2`, `Uri`, `Header(Header)`, `TooLarge`, `Status`, `Internal`

User sub-variants: `Body`, `BodyWriteAborted`, `InvalidConnectWithBody`, `Service`, `NoUpgrade`, `ManualUpgrade`, `DispatchGone`

Sentinel type: `TimedOut`

## Error Type 3: `client::Error` (`crates/hpx/src/client/http/client/error.rs`, ~142 lines)

| Variant | Payload | Notes |
|---------|---------|-------|
| `ErrorKind::Canceled` | — | Request canceled |
| `ErrorKind::ChannelClosed` | — | Channel closed |
| `ErrorKind::Connect` | — | Connection errors |
| `ErrorKind::ProxyConnect` | — | Proxy connection errors |
| `ErrorKind::UserUnsupportedRequestMethod` | — | Bad HTTP method |
| `ErrorKind::UserUnsupportedVersion` | — | Bad HTTP version |
| `ErrorKind::UserAbsoluteUriRequired` | — | Missing absolute URI |
| `ErrorKind::SendRequest` | — | Request send errors |

Also has: `ClientConnectError` enum, `TrySendError<B>` enum

## Overlap Analysis

### Duplicated across types

| Concept | hpx::Error | core::Error | client::Error |
|---------|-----------|-------------|---------------|
| Body error | `Kind::Body` | `Kind::Body` | — |
| Canceled | — | `Kind::Canceled` | `ErrorKind::Canceled` |
| Channel closed | — | `Kind::ChannelClosed` | `ErrorKind::ChannelClosed` |
| TimedOut sentinel | `TimedOut` | `TimedOut` | — |
| BoxError alias | `BoxError` | `BoxError` | (uses hpx's) |

### Unique to hpx::Error

- Builder, Request, Tls, Redirect, Status, Decode, Upgrade, WebSocket
- BadScheme, ProxyConnect sentinels

### Unique to core::Error

- Parse (with sub-enum), User (with sub-enum)
- IncompleteMessage, UnexpectedMessage, Io, BodyWrite, Shutdown, Http2

### Unique to client::Error

- Connect, ProxyConnect (different from hpx's ProxyConnect sentinel)
- UserUnsupportedRequestMethod, UserUnsupportedVersion, UserAbsoluteUriRequired, SendRequest

## Unified `hpx::Error` Design Proposal

```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    // From hpx::Error
    #[error("builder error")]
    Builder,
    #[error("error sending request")]
    Request,
    #[error("tls error")]
    Tls,
    #[error("error following redirect")]
    Redirect,
    #[error("HTTP status error ({0})")]
    Status(StatusCode, Option<ReasonPhrase>),
    #[error("request or response body error")]
    Body,
    #[error("error decoding response body")]
    Decode,
    #[error("error upgrading connection")]
    Upgrade,
    #[cfg(feature = "ws-yawc")]
    #[error("websocket error")]
    WebSocket,

    // From core::Error
    #[error("parse error")]
    Parse(Parse),
    #[error("user error")]
    User(User),
    #[error("incomplete message")]
    IncompleteMessage,
    #[error("unexpected message")]
    UnexpectedMessage,
    #[error("canceled")]
    Canceled,
    #[error("channel closed")]
    ChannelClosed,
    #[error("I/O error")]
    Io(#[source] std::io::Error),
    #[error("body write error")]
    BodyWrite,
    #[error("shutdown error")]
    Shutdown,
    #[error("HTTP/2 error")]
    Http2(#[from] h2::Error),

    // From client::Error
    #[error("connection error")]
    Connect,
    #[error("proxy connection error")]
    ProxyConnect,

    // Shared
    #[error("timed out")]
    TimedOut,
    #[error(transparent)]
    Other(#[from] BoxError),
}
```

### Breaking changes

- Downstream code matching on `core::Error::is_parse()` → need to match on `hpx::Error::Parse(_)`
- Downstream code matching on `client::Error::is_connect()` → need to match on `hpx::Error::Connect`
- `core::Error` and `client::Error` types deprecated, then removed

### Migration path

1. Add `From<core::Error>` and `From<client::Error>` impls for unified `hpx::Error`
2. Deprecate old types
3. Update internal error construction to use `hpx::Error` directly
4. Remove old types in next major version
