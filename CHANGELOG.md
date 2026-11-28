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
  - Known limitations: no Alt-Svc over h1, WebSocket deflate requires the `ws` (or `ws-yawc`) feature.
- **Exchange transport governance (`hpx-transport` crate)** — a dependency-free policy layer for HFT adapters.
  - Three independent quota dimensions (`IpWeight`, `Order`, `WebSocketFrame`) with lazy fixed windows, server-header calibration (`X-MBX-USED-WEIGHT-1M`, `X-MBX-ORDER-COUNT-1M`) and `Retry-After` backoff.
  - Per-message dynamic authentication hooks (`MessageSigner`, `NonReusableSigner`, `StaticToken`) for short-lived per-subscription JWTs.
  - Snapshot + buffered-delta order-book alignment (`BookSync`) with sequence-gap detection and resync.
- **FIX SBE streaming** (`hpx-streams/sbe`) — zero-copy length-prefixed envelopes (`SbeCodec`, `SbeMessage`) and `Accept` / `X-MBX-SBE` schema negotiation with `406`/`415` JSON fallback.
- **Borrowed JSON frames** (`hpx-streams/borrowed-json`) — `BorrowedJsonFrame` path lookups over the `simd-json` tape with no per-field allocation.
- **Chrome 150–152 emulation profiles** — ML-DSA `SignatureScheme` codepoints, signature-algorithm GREASE and the `trust_anchors` (`0xCA34`) extension, covered by wire-level ClientHello tests.
- **Emulation fingerprint self-check** (`hpx_emulation::verify`) — independent JA4H, ordered-header and Akamai HTTP/2 signatures with golden-value tests.
- **Low-latency TCP tuning** — `TcpTuning` (`SO_BUSY_POLL`, `SO_INCOMING_CPU`, `SO_PRIORITY`, `TCP_QUICKACK`) applied at connect time, plus `QuickAckStream` / `QuickAckRead` re-arming after reads.
- **Pool prewarming and stale bounds** — `pool_stale_after()` retires half-open idle sockets and backgrounds a replacement in the last tenth of the retirement window.
- **`simd-mask` opt-in for `hpx-yawc`** — AVX2 (runtime-detected, ≥ 64 B) and NEON (≥ 64 KiB on aarch64) XOR masking.
  - The default dispatcher now always uses the 64-bit word path, which measures 12–15% faster than the 32-bit path at 1 KiB and avoids the `align_to_mut` prefix/suffix handling.

### Changed

- `hpx-yawc` now declares and ships the MPL-2.0 license of its upstream fork instead of the workspace Apache-2.0.

### Fixed

- `hpx-browser`'s proxy integration test now compiles with default features; the `v8` and `socks` imports are gated like the tests that use them.
- `hpx-transport`'s `NonReusableSigner` now serializes the interval check and uses `Option<u64>` instead of treating timestamp zero as "never issued".
- SBE decoding uses checked length/offset arithmetic, so a malformed envelope or a caller-supplied offset cannot wrap or panic.
- `TcpTuning::apply` keeps applying the remaining options when one is unsupported by the kernel or container, and reports the first real socket error afterwards.
- The connection-pool prewarm lead is capped at half the retirement window when `stale_after` is much larger than `idle_timeout`.
- `BookSync` bounds the resync buffer (`DEFAULT_MAX_BUFFERED_DELTAS`, configurable with `with_max_buffered`) and reports `BufferLimitReached` instead of growing without limit.

### Documentation

- `docs/transport-runtime-decisions.md` records why the workspace keeps `quinn` and `tokio`, does not ship kernel TLS offload, and does not split WebSocket frames across `writev`.
