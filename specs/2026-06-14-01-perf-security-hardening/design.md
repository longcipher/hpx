# Design: Performance & Security Hardening for hpx Workspace

| Metadata | Details |
| :--- | :--- |
| **Status** | Draft |
| **Created** | 2026-06-14 |
| **Mode** | Full |
| **Priority** | P1 |
| **Planned at** | commit `fd64fd5`, 2026-06-14 |

## Summary

> This spec consolidates 11 findings across performance, correctness, and security in the hpx workspace. The highest-leverage changes eliminate unnecessary syscalls on the segment download hot path (redundant seeks), cache the TLS connector for WSS connections, replace O(n×m) linear scans with O(n) HashSet lookups, reduce allocation pressure in the download engine state machine, and harden the WebSocket HTTP CONNECT tunnel against CRLF injection.

## Why this matters

> hpx is designed for crypto exchange HFT where every microsecond counts. The segment download path (seek-per-chunk) runs thousands of unnecessary syscalls per download. The TLS connector rebuild per WSS connection adds hundreds of microseconds of latency on every reconnect. The download engine's clone-on-every-state-transition pattern creates allocation pressure under concurrent downloads. Security findings (CRLF injection, credential logging) are low-effort fixes that prevent real attack vectors.

## Approach

> All changes are localized to specific functions/modules with no cross-cutting architectural changes. Performance fixes focus on hot-path optimization (eliminating allocations, syscalls, and unnecessary work). Security fixes add input validation and log redaction. The persistence correctness fix changes the Drop implementation to drain pending commands. All changes follow existing codebase conventions (lock-free containers, `thiserror` for library errors, clippy pedantic compliance).

## Findings

### Finding 1: Redundant file seek per write chunk

- **Category:** performance
- **Impact:** HIGH — every chunk in a segment download calls `file.seek()` before `write_all()`, even though the file position is already correct after the first chunk. For a 100MB download with 64KB chunks, this is ~1,500 unnecessary syscalls.
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-01]:** The file shall be sought to `range.start` before the write loop begins.
- **[REQ-02]:** Subsequent chunks shall be written without additional seek calls.
- **[REQ-03]:** The final file position shall equal `range.start + total_bytes_written`.

#### Current state

```rust
// crates/hpx-dl/src/segment.rs:243-257
while let Some(chunk_result) = stream.next().await {
    let chunk = chunk_result?;
    let chunk_len = chunk.len();
    limiter.wait_for(u64::try_from(chunk_len).unwrap_or(u64::MAX)).await?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;  // <-- redundant after first iteration
    file.write_all(&chunk).await?;
    offset += u64::try_from(chunk_len).unwrap_or(u64::MAX);
    // ...
}
```

#### Approach

Seek once before the loop, remove the per-chunk seek. The `write_all` calls advance the file position correctly. Track `offset` only for progress reporting, not for seek positions.

#### Architecture Decisions (MADR Format)

- **AD-01:** Use a single `seek` before the loop rather than `pwrite` — the Tokio `File` API uses positioned writes via seek, and the single-seek approach is simpler.

### Finding 2: TLS connector rebuilt per WSS connection

- **Category:** performance
- **Impact:** HIGH — every `wss://` connection constructs a new `RootCertStore`, clones root certs, creates a `CryptoProvider`, builds a `ClientConfig`, and wraps it in `Arc<TlsConnector>`. This is ~100-500μs per connection.
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-01]:** The TLS connector shall be constructed at most once per process lifetime (or per feature-flag configuration).
- **[REQ-02]:** The cached connector shall be thread-safe and shareable across async tasks.

#### Current state

```rust
// crates/yawc/src/native/mod.rs:1330-1367
fn tls_connector() -> TlsConnector {
    let mut root_cert_store = rustls::RootCertStore::empty();
    root_cert_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().map(|ta| TrustAnchor { ... }));
    let maybe_provider = rustls::crypto::CryptoProvider::get_default().cloned();
    // ... builds config, wraps in Arc
    TlsConnector::from(Arc::new(config))
}
```

Called at `mod.rs:795` via `connector.unwrap_or_else(tls_connector)` for every connection.

#### Approach

Use `std::sync::OnceLock<TlsConnector>` (or `LazyLock` on nightly) to cache the connector. The root cert store and config are immutable after construction.

#### Architecture Decisions (MADR Format)

- **AD-01:** Use `OnceLock` rather than `lazy_static` — `OnceLock` is std and requires no external dependency.

### Finding 3: O(n×m) linear scan in segment filtering

- **Category:** performance
- **Impact:** MEDIUM — `filter_remaining_segments` at `segment.rs:682` and `build_segment_states` at `engine.rs:1091` both use `Vec::contains` for index lookup, resulting in O(n×m) complexity where n = total segments and m = completed indices.
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-01]:** Segment filtering shall complete in O(n) time.
- **[REQ-02]:** The completed indices set shall be constructed once before filtering.

#### Current state

```rust
// crates/hpx-dl/src/segment.rs:679-684
.filter(|(i, _)| !completed_indices.contains(&(*i as u32)))  // O(m) per segment

// crates/hpx-dl/src/engine.rs:1091
let completed = completed_indices.contains(&(index as u32));  // O(m) per segment
```

#### Approach

Collect `completed_indices` into a `HashSet<u32>` before filtering. The `contains` call becomes O(1).

### Finding 4: CSV ReaderBuilder constructed per row

- **Category:** performance
- **Impact:** MEDIUM — `csv::ReaderBuilder` is allocated and configured inside a `.map()` closure for every CSV line streamed.
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-01]:** The CSV reader builder shall be constructed once before the stream processing begins.
- **[REQ-02]:** The builder configuration (delimiter, has_headers) shall be reused for every row.

#### Current state

```rust
// crates/hpx-streams/src/csv_stream.rs:81-86
.map(move |frame_res| match frame_res {
    Ok(frame_str) => {
        let mut csv_reader = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .has_headers(false)
            .from_reader(frame_str.as_bytes());
```

#### Approach

Hoist the `ReaderBuilder` creation outside the `.map()` closure. Clone the builder for each row (builders are cheap to clone) or create a helper function that wraps the per-row logic.

### Finding 5: DownloadEntry clone cascade

- **Category:** performance
- **Impact:** MEDIUM — `update_entry` at `engine.rs:346-356` clones the entire `DownloadEntry` after atomic update via `entry_snapshot`, then `to_record` at `engine.rs:278-295` clones 7+ fields including `Vec<SegmentState>`.
- **Effort:** M

#### Requirements (EARS Notation)

- **[REQ-01]:** The modified entry shall be returned directly from `update_sync` without a redundant snapshot clone.
- **[REQ-02]:** `to_record` shall avoid cloning fields that can be borrowed or moved.

#### Current state

```rust
// crates/hpx-dl/src/engine.rs:346-356
let updated = self.downloads.update_sync(&id, |_, entry| {
    modify(entry);
    entry.touch();
});
// ...
let entry = self.entry_snapshot(id)?;  // clones entire DownloadEntry
let record = entry.to_record(id);      // clones 7+ fields
```

#### Approach

Have `update_sync` return the modified entry (or a snapshot of it) directly. Refactor `to_record` to use `Arc` for shared immutable fields or accept a mutable reference to swap out fields.

### Finding 6: std::sync::mpsc replaced with crossbeam-channel

- **Category:** performance
- **Impact:** LOW-MEDIUM — the persistence channel uses `std::sync::mpsc` which has worse performance under contention than `crossbeam-channel`. Per AGENTS.md, `std::sync::mpsc` is forbidden.
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-01]:** The persistence channel shall use `crossbeam_channel` instead of `std::sync::mpsc`.
- **[REQ-02]:** The API surface of `PersistenceHandle` shall remain unchanged.

#### Current state

```rust
// crates/hpx-dl/src/persistence.rs:1-7
use std::sync::{
    Arc,
    mpsc::{self, Sender},
};
```

#### Approach

Replace `std::sync::mpsc` with `crossbeam_channel`. The `Sender` and `Receiver` types have compatible APIs. Add `crossbeam-channel` to workspace dependencies.

### Finding 7: mark_segment_completed linear scan

- **Category:** performance
- **Impact:** LOW-MEDIUM — `mark_segment_completed` at `engine.rs:418-432` does `entry.segments.iter_mut().find(|s| s.index == index)` which is a linear scan per completion event.
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-01]:** Segment lookup by index shall be O(1).
- **[REQ-02]:** The segments structure shall support indexed access.

#### Current state

```rust
// crates/hpx-dl/src/engine.rs:424-432
self.update_entry(id, |entry| {
    if let Some(segment) = entry.segments.iter_mut().find(|segment| segment.index == index) {
        segment.state = SegmentStatus::Completed;
        segment.bytes_downloaded = bytes_downloaded;
    }
});
```

#### Approach

Either change `segments` to a `Vec` indexed by segment index (since indices are sequential), or add a `HashMap<u32, usize>` index mapping segment index to position. The former is simpler if segments are always dense.

### Finding 8: range_header_value String allocation

- **Category:** performance
- **Impact:** LOW — `range_header_value` at `segment.rs:192-194` allocates a `String` via `format!` for every segment request.
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-01]:** The range header value shall be formatted without heap allocation.

#### Current state

```rust
// crates/hpx-dl/src/segment.rs:192-194
pub fn range_header_value(range: &SegmentRange) -> String {
    format!("bytes={}-{}", range.start, range.end)
}
```

#### Approach

Use `ArrayString` from `arrayvec` or a stack-allocated `[u8; 64]` buffer with `write!`. Alternatively, return a `SmallString` or pre-compute the header when segments are created.

### Finding 9: Silent persistence data loss on shutdown

- **Category:** correctness
- **Impact:** MEDIUM — `PersistenceHandle::Drop` at `persistence.rs:129-138` sends `Shutdown` but does not join the worker thread. Pending `Upsert`/`Delete` commands may be lost.
- **Effort:** M

#### Requirements (EARS Notation)

- **[REQ-01]:** The persistence worker shall process all pending commands before exiting.
- **[REQ-02]:** The drop implementation shall block briefly (bounded timeout) to drain the queue.
- **[REQ-03]:** The worker shall not hang indefinitely if the storage backend is unresponsive.

#### Current state

```rust
// crates/hpx-dl/src/persistence.rs:129-138
impl Drop for PersistenceHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(PersistenceCommand::Shutdown);
        // Do NOT join the worker thread here
    }
}
```

#### Approach

Replace the unbounded `mpsc` channel with a bounded `crossbeam_channel` (capacity 64 or 128). In `Drop`, send `Shutdown` and then drain any remaining commands from the sender side before dropping it. Alternatively, use a shutdown acknowledgment pattern with a short timeout.

### Finding 10: CRLF injection in WS HTTP CONNECT tunnel

- **Category:** security
- **Impact:** MEDIUM — `target_host` is interpolated into the CONNECT request line and Host header at `proxy.rs:94-108` without sanitization. A malicious host containing `\r\n` can inject arbitrary headers.
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-01]:** The target host shall be validated to contain only valid hostname characters before interpolation.
- **[REQ-02]:** Invalid hosts shall return an error, not be silently accepted.

#### Current state

```rust
// crates/yawc/src/native/proxy.rs:94-108
let mut connect_req = format!(
    "CONNECT {authority} HTTP/1.1\r\n\
     Host: {authority}\r\n"
);
```

#### Approach

Add a validation function that checks `target_host` matches `^[a-zA-Z0-9.\-:\[\]]+$`. Reject hosts with control characters. This is a standard input validation pattern.

### Finding 11: LoggingHook leaks Authorization headers

- **Category:** security
- **Impact:** MEDIUM — `LoggingHook::on_request` at `hooks.rs:217-218` logs all header values including `Authorization`, `Cookie`, and `Proxy-Authorization` when TRACE is enabled.
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-01]:** Sensitive header values (Authorization, Cookie, Proxy-Authorization) shall be redacted in log output.
- **[REQ-02]:** The header name shall still be logged for debugging purposes.

#### Current state

```rust
// crates/hpx/src/client/layer/hooks.rs:216-219
if self.log_headers {
    for (name, value) in request.headers() {
        tracing::trace!(header = %name, value = ?value);
    }
}
```

#### Approach

Add a set of sensitive header names (`authorization`, `cookie`, `proxy-authorization`). When logging, check if the header name is in the set and log `"[REDACTED]"` instead of the value.

### Finding 12: Proxy credentials logged in TRACE

- **Category:** security
- **Impact:** MEDIUM — the proxy URI including credentials is logged at TRACE level in the connector.
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-01]:** Proxy URIs logged at TRACE level shall have credentials masked.
- **[REQ-02]:** The proxy host and port shall remain visible.

#### Current state

```rust
// crates/hpx/src/client/conn/connector.rs:376
trace!("connect with maybe proxy: {:?}", proxy);
```

#### Approach

Implement a `redact()` helper on `Uri` or use `url::Url::password()` to strip credentials before logging. Log `scheme://***@host:port` instead of the full URI.

## Architecture Decisions

> Consolidated MADR decisions across all findings.

### AD-01: Use OnceLock for TLS connector caching

- **Decision:** Use `std::sync::OnceLock<TlsConnector>` for the cached TLS connector in yawc.
- **Rationale:** `OnceLock` is std, requires no external dependency, and provides the exact semantics needed (construct once, share immutably).
- **Trade-off:** The connector is per-process-lifetime, which is appropriate for immutable TLS config.

### AD-02: Use HashSet for segment index lookups

- **Decision:** Convert `completed_indices: &[u32]` to `HashSet<u32>` before filtering/mapping operations.
- **Rationale:** Changes O(n×m) to O(n) with minimal code change. The HashSet is constructed once per call.
- **Trade-off:** Small allocation for the HashSet, but it's dwarfed by the O(n×m) savings.

### AD-03: Use crossbeam-channel for persistence

- **Decision:** Replace `std::sync::mpsc` with `crossbeam_channel` in `PersistenceHandle`.
- **Rationale:** Per AGENTS.md, `std::sync::mpsc` is forbidden. `crossbeam-channel` has better performance and supports bounded channels for backpressure.
- **Trade-off:** Adds a dependency, but it's already used in the workspace.

### AD-04: Validate hostnames for CONNECT tunnel

- **Decision:** Add a hostname validation function that rejects control characters.
- **Rationale:** Standard input validation prevents CRLF injection. The validation is cheap and the set of valid hostname characters is well-defined.
- **Trade-off:** May reject unusual but technically valid IPv6 addresses in brackets — the regex `[a-zA-Z0-9.\-:\[\]]+` covers standard formats.

## BDD/TDD Strategy

- **Primary Language:** Rust
- **BDD Runner:** cucumber-rs (existing in hpx-dl)
- **BDD Command:** `cargo test -p hpx-dl --test cucumber --all-features`
- **Unit Test Command:** `cargo nextest run --workspace --all-features`
- **Feature Files:** `specs/2026-06-14-01-perf-security-hardening/features/*.feature`
- **Outside-in Loop:** Performance scenarios are verified by existing integration tests + new unit tests. Security scenarios require new unit tests for validation functions.

## Code Simplification Constraints

- **Behavioral Contract:** Preserve existing behavior unless a listed scenario explicitly changes it. The segment download produces identical output bytes. The WSS connection works identically. The persistence layer stores the same records.
- **Repo Standards:** Use `thiserror` for library errors, `eyre` for app errors. Follow clippy pedantic + nursery. No `unwrap`/`expect` in non-test code.
- **Readability Priorities:** Prefer explicit control flow, clear names, reduced nesting.
- **Refactor Scope:** Limit cleanup to the touched functions/modules.

## BDD Scenario Inventory

> Complete list of ALL scenarios across ALL findings with task coverage.

- `features/performance.feature` — Segment download avoids redundant seeks → Task 1.1, 1.2
- `features/performance.feature` — TLS connector is cached across WSS connections → Task 2.1, 2.2
- `features/performance.feature` — Segment filtering uses constant-time lookup → Task 3.1, 3.2
- `features/performance.feature` — CSV stream reuses ReaderBuilder across rows → Task 4.1, 4.2
- `features/performance.feature` — State transitions avoid unnecessary full-entry clones → Task 5.1, 5.2, 5.3
- `features/performance.feature` — Persistence channel uses crossbeam-channel → Task 6.1, 6.2
- `features/performance.feature` — Segment completion uses indexed lookup → Task 7.1, 7.2
- `features/performance.feature` — Range header value avoids heap allocation → Task 8.1, 8.2
- `features/correctness.feature` — Persistence worker drains pending commands on shutdown → Task 9.1, 9.2
- `features/security.feature` — HTTP CONNECT tunnel rejects hosts with control characters → Task 10.1, 10.2
- `features/security.feature` — Sensitive headers are redacted in request logging → Task 11.1, 11.2
- `features/security.feature` — Proxy credentials are not logged in trace output → Task 12.1, 12.2

## Existing Components to Reuse

- `ahash::RandomState` / `HashSet` — already used in the workspace for fast hash sets
- `crossbeam-channel` — already a workspace dependency (used in other crates)
- `std::sync::OnceLock` — std, no additional dependency needed
- Existing `#[cfg(test)]` modules in each affected file for colocated tests

## Verification

| Purpose   | Command                           | Expected on success |
|-----------|-----------------------------------|---------------------|
| Format    | `cargo +nightly fmt --all`        | exit 0              |
| Lint      | `cargo +nightly clippy --all -- -D warnings` | exit 0, no errors |
| Tests     | `cargo nextest run --workspace --all-features` | all pass |
| BDD       | `cargo test -p hpx-dl --test cucumber --all-features` | all pass |
| Docs      | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --all-features` | exit 0 |
