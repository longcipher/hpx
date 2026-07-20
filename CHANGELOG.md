# Changelog

## Unreleased

### Added

- **HTTP/3 support** (feature-gated behind `http3` feature flag). Enable with `--features http3`.
  - Uses `quinn` for QUIC transport and `h3` crate for HTTP/3 framing.
  - `Client::builder().http3_only()` and `http3_prior_knowledge()` builder methods for HTTP/3 prior knowledge (RFC 9114 §3.1.1).
  - `Http3Options` for configuring QPACK, max field section size, and QUIC transport parameters (Chrome 143 baseline defaults).
  - `QuicConfig` for custom QUIC transport settings (low-level escape hatch over `quinn::TransportConfig`).
  - `H3Error` enum with 12 typed error variants (`Handshake`, `Framing`, `StreamReset`, `IdleClose`, `ZeroRttRejected`, `MigrationFailed`, `VersionNegotiationFailed`, `FlowControl`, `MaxConcurrentStreamsExceeded`, `GoAwayDrained`, `AltSvcUnreachable`, `Other`) exposed at crate root.
  - 13 integration tests covering ALPN negotiation, concurrent requests, streaming body, STOP_SENDING, reconnection, connection failure, and graceful shutdown.
  - Criterion benchmark for h3 vs h2 throughput comparison.
  - Alt-Svc discovery (RFC 7838): `AltSvcCache`, automatic h2→h3 upgrade via Alt-Svc headers, `prefer_http3()` builder method.
  - `H3FailureTracker` circuit breaker with 60s cooldown for QUIC connection failures.
  - 0-RTT resumption support (best-effort optimization, `enable_0rtt` in `Http3Options`).
  - Connection idle timeout with graceful GOAWAY shutdown.
  - WebSocket-over-h3 via Extended CONNECT (RFC 9220) with full WebSocket framing (text, binary, ping, pong, close), `H3WebSocket` transport.
  - Browser emulation macros for HTTP/3: Chrome 96/143, Firefox 88, Safari 14, Edge 96 with `http3_options` and QUIC transport parameter fingerprints.
  - `--http3`, `--prefer-http3`, `--emulation` CLI flags via `hpx-cli` crate.
  - HTTP/1 RFC gap closures: 100-continue (RFC 9110 §10.1.1), smuggling rejection (RFC 9112 §3.3), obs-fold rejected (RFC 9112 §2.3).
  - WebSocket permessage-deflate extension (RFC 7692) with compression/decompression and context takeover support.
  - Known limitations: no Alt-Svc over h1, WebSocket deflate requires `ws` or `ws-fastwebsockets` feature.