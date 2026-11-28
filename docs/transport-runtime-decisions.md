# Transport Runtime Decisions

This document records the three transport-level investigations that were open
for the "four architectural fault lines" work, the evidence behind each
decision, and what would have to change to revisit them.

The short version: all three were investigated to the point where the
remaining step is a change in an *upstream* dependency or a rewrite of the
runtime model, not a change inside hpx. hpx therefore keeps `quinn`, keeps
`tokio`, and does not ship kernel TLS offload.

---

## 1. quiche instead of quinn (unify the TLS backend)

**Question.** hpx links BoringSSL for TLS and `quinn` for QUIC. `quinn`
brings its own TLS stack (`rustls` with `ring`/`aws-lc-rs`, or BoringSSL via
the `rustls-boring` provider). Could `quiche` replace `quinn` so that QUIC and
TCP share one TLS implementation and one fingerprint?

**Answer: no — keep `quinn`.**

### Evidence

- `quiche 0.30.1` and `tokio-quiche 0.23.0` both cap their `libc`
  dependency at `<0.2.190` (an iOS backtrace workaround). hpx requires
  `libc = "0.2.191"`. Cargo unifies semver-compatible `0.2.x` versions, so
  the two requirements are disjoint and **cannot both be satisfied**.
- The published pairs that *are* compatible (`quiche 0.30.0` +
  `tokio-quiche 0.20.0`, both `^0.2.76`) ship a `tokio-quiche` with the High
  severity advisory **CVE-2026-12708** (GHSA-wcm9-jv39-r963), which is only
  fixed in `tokio-quiche 0.23.0` — the version that reintroduces the `libc`
  conflict.

### What would change the answer

- `quiche` dropping the `libc` cap (or hpx relaxing its own `libc` floor),
  **and** `tokio-quiche` shipping a fixed release for every version hpx would
  have to support.

---

## 2. Kernel TLS offload (kTLS)

**Question.** For HFT workloads the syscall and TLS record costs matter.
Could the TLS record layer be pushed into the kernel with `KTLS`?

**Answer: not on this stack — the traffic secrets cannot be exported.**

### Evidence

- BoringSSL 5.6.0 *does* expose the raw TLS 1.3 traffic secrets:
  `SSL_get_traffic_secrets()` is declared in `include/openssl/ssl.h` and
  implemented in `ssl/ssl_lib.cc`.
- The declaration is **C++-only** (`Span<const uint8_t>`), and none of the 15
  generated `boring-sys` binding files expose it, so the function is not
  reachable from Rust through the current bindings.
- `SSL_export_keying_material()` is the RFC 5705 exporter, not the raw
  secret; `SSL_get_dtls_*_traffic_secret()` is DTLS-only.

### Feasible alternative

The secrets *are* obtainable in Rust through a different path:

- **BoringSSL keylog callback.** BoringSSL emits `CLIENT_TRAFFIC_SECRET_0`
  and `SERVER_TRAFFIC_SECRET_0`; hpx already wires `set_keylog_callback`
  (`crates/hpx/src/tls/boring.rs`), so the secrets can be captured from the
  key log and fed to the kernel.
- **rustls `dangerous_extract_secrets()`.** Available on the rustls backend.

That path is Linux-only (kTLS is a Linux feature) and, for the small
order-entry frames this workspace targets, saves little: the record layer is
not the bottleneck. It is therefore not implemented.

---

## 3. thread-per-core / io_uring runtime

**Question.** Could hpx drop `tokio` for a thread-per-core runtime
(`monoio`, `compio`) with an io_uring driver?

**Answer: no — keep `tokio`.**

### Evidence

- The workspace has ~950 `tokio::` references across ~110 files.
- hpx's `Executor`, `Timer` and the shared `Arc`-based client configuration
  and connection pool all require `Send + Sync + 'static`. The `monoio` and
  `compio` connection types are `!Send` and thread-local, so they cannot be
  stored in a pool that is shared between runtimes.
- `HPX_RUNTIME` (`crates/hpx/src/client/core/rt/tokio.rs`) is a `OnceLock`
  holding a four-worker `tokio::runtime::Runtime`. It exists deliberately so
  that one connection pool can be reused across several runtimes; a
  thread-per-core model has no equivalent sharing story.
- Platform support is asymmetric: `monoio` is Linux/kqueue only, while
  `compio` has a macOS-capable `polling` driver. Dropping `tokio` would drop
  the platforms hpx supports today.

### What would change the answer

A runtime whose connection types are `Send + Sync + 'static`, with an
injectable timer, and an io_uring driver that keeps the same executor
abstraction. That is a runtime-level project, not a client-level one.

---

## 4. `writev` frame writes and shared compression pools

**Question.** Should `hpx-yawc` split each frame into a header `IoSlice` and a
payload `IoSlice` and use `poll_write_vectored`, and should permessage-deflate
contexts be shared across connections (or compressed frames cached)?

**Answer: no — the measured cost is not in the syscall the split would save.**

### Evidence

- The encoder already writes the 2–14-byte frame header and the payload into
  one contiguous `BytesMut`. A Hotpath run over a 1 KiB client frame attributes
  ~9 ns to `Frame::write_head` and ~192 ns to the whole `Frame::encode`, with
  the mask application itself below that. Splitting the header out would save
  at most that header copy.
- Client payloads must be masked in place, so a vectored write would have to
  keep the masked payload alive as its own buffer. That is an extra
  allocation/copy, which is larger than the copy the split removes.
- The writers below the codec are TLS. Both rustls and BoringSSL encrypt each
  record into a contiguous output buffer, and `is_write_vectored()` is not
  guaranteed to be true. The kernel still sees one `write` per TLS record, so
  the syscall count does not change.
- Per-connection deflate contexts are what makes context takeover correct.
  Sharing one zlib context across connections changes the compression
  semantics and puts a lock on the frame hot path. A prepared-message cache
  only helps repeated identical frames; client subscription traffic is small
  and mostly unique.

### What would change the answer

A plain-TCP WebSocket path with no TLS record layer, where a vectored write
maps to one syscall, or a workload dominated by repeated identical compressed
frames.

---

## Summary

| Investigation | Decision | Blocker |
| --- | --- | --- |
| `quiche` instead of `quinn` | keep `quinn` | `libc` version conflict + CVE-2026-12708 |
| Kernel TLS offload | not implemented | traffic secrets not reachable from Rust in BoringSSL |
| thread-per-core / io_uring | keep `tokio` | `Send + Sync + 'static` requirements, platform coverage |
| WebSocket `writev` / shared deflate pool | not implemented | TLS record coalescing; masking is in place; per-connection deflate semantics |

None of these blocks the features that were actually missing. The
low-latency work that *is* implemented lives one layer below these
questions: `TCP_QUICKACK` re-arming per read
(`crates/hpx/src/client/conn/tcp_tuning.rs`) and background connection
prewarming (`crates/hpx/src/client/http/client/pool.rs`), plus the opt-in
`simd-mask` XOR path in `crates/yawc/src/mask.rs` (AVX2 on x86_64; the scalar
64-bit path remains faster on aarch64 for the measured sizes).
