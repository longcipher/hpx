# Design: hpx-dl Download Engine

| Metadata | Details |
| :--- | :--- |
| **Status** | Draft |
| **Created** | 2026-03-22 |
| **Scope** | Full |

## Executive Summary

Build a new crate `hpx-dl` inside the hpx workspace that provides an embeddable, reliable download engine as an alternative to aria2. The engine uses `hpx` (not `reqwest`) as its HTTP backend and replicates aria2's core feature set: multi-connection segmented downloading, download resume with integrity verification, global/per-download speed limiting, download queuing with priority scheduling, Metalink parsing, and proxy support. The design draws on the architecture patterns proven by `gosh-dl` (segmented HTTP, SQLite persistence, priority queue) and `parallel_downloader` (state-file crash recovery, token-bucket rate limiting), adapted to the hpx workspace's conventions.

## Source Inputs & Normalization

The planner consumed:

1. **User requirement**: "参考 <https://github.com/goshitsarch-eng/gosh-dl> <https://crates.io/crates/parallel_downloader> 实现一个 new crate hpx-dl 实现 reliable download, embeddable alternative to aria2, 复刻 aria2 的全部特性. 注意不用 reqwest 而使用 hpx"

2. **Reference repos** (analyzed via web research):
   - `gosh-dl` v0.3.2: Full HTTP + BitTorrent engine with SQLite persistence, segmented downloads, DHT/PEX/LPD, uTP, WebSeeds, priority queue, bandwidth scheduling.
   - `parallel_downloader` v0.3.0: Simpler parallel chunked HTTP downloader with JSON state files, token-bucket rate limiting, daemon mode, TUI.

3. **Live codebase analysis**: hpx workspace (Rust, edition 2024), `hpx` crate provides `Client`, `Request`, `Response` with streaming `bytes_stream()`, configurable timeouts, proxy support, TLS (BoringSSL/Rustls), retry policies, redirect following, and Tower-based middleware stack.

**Ambiguities resolved as assumptions**:

- **A1**: "aria2 的全部特性" (aria2's full features) — interpreted as HTTP/HTTPS multi-connection, resume, speed limiting, queuing, checksum verification, Metalink, proxy. BitTorrent and SFTP are deferred (out of scope for v1) because they require protocol stacks not available in hpx and would massively expand scope.
- **A2**: The crate should be library-first (embeddable), with an optional CLI binary. This matches gosh-dl's design over parallel_downloader's daemon approach.
- **A3**: Persistence should use SQLite (like gosh-dl) rather than JSON state files (like parallel_downloader) for better crash recovery and concurrent access.

## Requirements & Goals

### Functional Requirements

| ID | Requirement |
|----|-------------|
| R1 | Multi-connection segmented HTTP/HTTPS downloads with configurable connection count (1-16) |
| R2 | Download resume via byte-range and ETag/Last-Modified content validation |
| R3 | Per-download and global speed limiting using token-bucket algorithm |
| R4 | Priority-based download queue (Critical, High, Normal, Low) |
| R5 | Checksum verification (MD5, SHA-1, SHA-256) |
| R6 | Metalink v4 (RFC 5854) parsing for multi-source + checksum manifests |
| R7 | HTTP/HTTPS proxy support (leveraging hpx's existing proxy infrastructure) |
| R8 | Download state persistence and crash recovery via SQLite |
| R9 | Event-driven progress reporting via broadcast channels |
| R10 | Embeddable library API (no IPC/RPC required) |
| R11 | Optional CLI binary for standalone use |

### Non-Functional Requirements

| ID | Requirement |
|----|-------------|
| N1 | Use `hpx` crate exclusively for HTTP; `reqwest` forbidden per AGENTS.md |
| N2 | Follow workspace dependency conventions (`cargo add`, workspace versions) |
| N3 | Error handling: `thiserror` for library errors, `eyre` for CLI binary |
| N4 | Logging: `tracing` only |
| N5 | Clippy pedantic compliance, no `unwrap()`/`expect()`/`panic!()` in library code |
| N6 | Concurrency: prefer `scc` for concurrent maps, `tokio::sync` for channels |
| N7 | Feature-gated compilation (core/http/metalink/storage/progress/cli) |

### Out of Scope

| ID | Item | Rationale |
|----|------|-----------|
| O1 | BitTorrent support | Requires full BitTorrent protocol stack (DHT, PEX, trackers, piece management); not available in hpx. Could be added as a future `hpx-bt` crate. |
| O2 | SFTP/FTP support | Requires `libssh2`/`libssh` bindings; hpx is HTTP-only. |
| O3 | RPC/JSON-RPC interface | Library-first design; embeddable means no IPC needed. Users import the crate directly. |
| O4 | TUI dashboard | Out of scope for v1; progress is event-driven, consumers can build their own UI. |
| O5 | uTP transport | Protocol-specific; not relevant for HTTP-only downloads. |

## Requirements Coverage Matrix

| Req ID | design.md Section | Scenarios | Tasks |
|--------|-------------------|-----------|-------|
| R1 | Detailed Design — Segmented Download | seg-download-basic, seg-download-resume | Task 3.1, 3.2 |
| R2 | Detailed Design — Resume & Integrity | seg-download-resume | Task 3.3, 3.4 |
| R3 | Detailed Design — Speed Limiting | speed-limit-enforced | Task 4.1 |
| R4 | Detailed Design — Download Queue | queue-priority-ordering | Task 4.2 |
| R5 | Detailed Design — Checksum Verification | checksum-verification | Task 5.1 |
| R6 | Detailed Design — Metalink | metalink-download | Task 5.2 |
| R7 | Detailed Design — Proxy | proxy-download | Task 5.3 |
| R8 | Detailed Design — Persistence | crash-recovery | Task 6.1 |
| R9 | Detailed Design — Events | progress-reporting | Task 6.2 |
| R10 | Architecture Overview | All | Task 2.1 |
| R11 | Detailed Design — CLI | cli-basic-download | Task 7.1 |
| N1 | Architecture Decisions | — | Task 1.1 |
| N2 | Architecture Decisions | — | Task 1.1 |
| N3 | Architecture Decisions | — | Task 2.1 |
| N4 | Architecture Decisions | — | Task 2.1 |
| N5 | Architecture Decisions | — | Task 2.1 |
| N6 | Architecture Decisions | — | Task 4.1 |
| N7 | Architecture Decisions | — | Task 2.1 |

## Planner Contract Surface

### PlannedSpecContract

This spec is build-eligible for `/pb-build`. The contract surface consists of:

- `design.md` — this file, containing the full architecture, decisions, and detailed design.
- `tasks.md` — phased task breakdown with `Task X.Y` IDs, verification criteria, and BDD/TDD loop types.
- `features/*.feature` — Gherkin scenarios covering user-visible behavior.

### TaskContract

Each task in `tasks.md` includes:

- Unique `Task X.Y` ID for state tracking.
- `Loop Type: BDD+TDD` for user-visible behavior, `TDD-only` for infrastructure.
- `Behavioral Contract` stating whether existing behavior is preserved or intentionally changed.
- `Simplification Focus` to keep implementations readable.
- `Advanced Test Coverage` indicating property/fuzz/benchmark requirements.
- Concrete `Verification` criteria.

### BuildBlockedPacket

If `/pb-build` encounters a blocker, it should emit a `BuildBlockedPacket` with:

- The task ID that is blocked.
- The specific blocker (e.g., missing dependency, test failure).
- Suggested resolution.

### DesignChangeRequestPacket

If implementation reveals a design gap, `/pb-build` or the developer should emit a `DesignChangeRequestPacket` referencing:

- The design section that needs revision.
- The concrete issue discovered.
- Proposed change.

## Architecture Overview

### System Context

```text
┌─────────────────────────────────────────────────────────┐
│                    hpx-dl crate                          │
│                                                          │
│  ┌──────────┐  ┌──────────────┐  ┌──────────────────┐   │
│  │  Engine   │  │  Downloader  │  │    Storage        │   │
│  │  (core)   │──│  (segmented) │──│  (SQLite impl)    │   │
│  └──────────┘  └──────────────┘  └──────────────────┘   │
│       │              │                    │               │
│  ┌──────────┐  ┌──────────────┐  ┌──────────────────┐   │
│  │  Queue    │  │  Rate Limiter│  │   Metalink        │   │
│  │(priority) │  │(token bucket)│  │   Parser          │   │
│  └──────────┘  └──────────────┘  └──────────────────┘   │
│                      │                                   │
│              ┌──────────────┐                            │
│              │  hpx::Client │  (HTTP backend)            │
│              └──────────────┘                            │
└─────────────────────────────────────────────────────────┘
```

### Module Structure

```text
crates/hpx-dl/
├── Cargo.toml
├── src/
│   ├── lib.rs              # Public API re-exports
│   ├── engine.rs           # DownloadEngine: central coordinator
│   ├── error.rs            # Error types (thiserror)
│   ├── queue.rs            # Priority download queue
│   ├── segment.rs          # Segmented download logic
│   ├── speed.rs            # Token-bucket rate limiter
│   ├── metalink.rs         # Metalink v4 parser (winnow)
│   ├── checksum.rs         # Checksum verification
│   ├── storage/
│   │   ├── mod.rs          # Storage trait
│   │   └── sqlite.rs       # SQLite implementation
│   ├── event.rs            # Download events (broadcast)
│   └── cli.rs              # Optional CLI (behind feature flag)
```

## Architecture Decisions

### Inherited Decisions

From `AGENTS.md` and existing hpx workspace conventions:

1. **HTTP Client**: `hpx::Client` is the sole HTTP backend. No `reqwest`.
2. **Error Handling**: `thiserror` for library error types. `eyre` only for CLI binary.
3. **Logging**: `tracing` only. No `log` crate.
4. **Concurrent Maps**: `scc` for any concurrent map needs (e.g., active download tracking).
5. **Parsing**: `winnow` for Metalink XML/binary parsing.
6. **Clippy**: Pedantic lints. No `unwrap()`, `expect()`, or `panic!()` in library code.
7. **Dependency Management**: All deps added via `cargo add -p hpx-dl --workspace`.
8. **Edition**: Rust 2024, resolver 3, matching workspace root.

### New Pattern Selection

| Decision | Pattern | Rationale |
|----------|---------|-----------|
| Engine coordination | **Strategy** | `DownloadEngine` delegates to protocol-specific `Downloader` trait implementations. HTTP is the first implementation; future protocols (BitTorrent, SFTP) can be added without modifying the engine. |
| Rate limiting | **Token Bucket** (via `governor` crate) | Proven in both gosh-dl and parallel_downloader. Simple, efficient, supports per-download and global limits. |
| Download queue | **Priority Queue** (binary heap) | gosh-dl's approach: binary heap with `DownloadPriority` ordering. Straightforward and effective. |
| Storage | **Strategy** (trait + impl) | `Storage` trait abstracts persistence. SQLite implementation is default; in-memory impl for testing. |
| Event notification | **Observer** (broadcast channels) | `tokio::sync::broadcast` for download events. Consumers subscribe to specific download or global events. |
| Segmented download | **Adapter** | `SegmentDownloader` adapts `hpx::Client` for range-based concurrent fetching. |

### SRP / DIP Check

- **SRP**: Each module has a single responsibility — `engine.rs` coordinates, `segment.rs` manages segments, `speed.rs` enforces limits, `storage/` persists state, `queue.rs` prioritizes, `metalink.rs` parses.
- **DIP**: The `Downloader` trait and `Storage` trait define abstractions. The engine depends on trait interfaces, not concrete implementations. `hpx::Client` is injected, not constructed internally, enabling test mocking.

### Code Simplifier Alignment

- The Strategy pattern for `Downloader` reduces coupling between the engine and protocol implementations without adding ceremony — it is a single trait with `download()`, `pause()`, `resume()` methods.
- The `Storage` trait uses only 5 methods (save, load, list, delete, update_progress), keeping the interface minimal.
- Token-bucket via `governor` avoids reinventing rate limiting with complex async semaphore patterns.

## BDD/TDD Strategy

- **Primary Language:** Rust
- **BDD Runner:** `cucumber`
- **BDD Command:** `cargo test --features cli -p hpx-dl --test cucumber`
- **Unit Test Command:** `cargo nextest run -p hpx-dl --all-features`
- **Property Test Tool:** `proptest` (for segment calculation, Metalink parsing, checksum verification)
- **Fuzz Test Tool:** `cargo-fuzz` (conditional — for Metalink binary/XML parsing with untrusted input)
- **Benchmark Tool:** `criterion` (N/A for v1 — no explicit latency SLA; can be added if throughput targets emerge)
- **Feature Files:** `specs/2026-03-22-01-hpx-dl-download-engine/features/*.feature`
- **Outside-in Loop:** Start with `seg-download-basic.feature` scenario (add download, start, complete) → drive `DownloadEngine::add()` and `SegmentDownloader` implementation → then `seg-download-resume.feature` → drive resume logic → then queue/speed/checksum/metalink scenarios.

## Code Simplification Constraints

- **Behavioral Contract:** No existing behavior to preserve — this is a new crate. All behavior is net-new.
- **Repo Standards:** Follow AGENTS.md conventions exactly: `hpx` for HTTP, `scc` for concurrent maps, `tracing` for logging, `thiserror`/`eyre` for errors, `winnow` for parsing, clippy pedantic.
- **Readability Priorities:** Prefer explicit control flow, named constants over magic numbers, clear error types over boxed errors. Avoid nested closures — extract named functions for segment download, checksum computation, etc.
- **Refactor Scope:** N/A (new crate).
- **Clarity Guardrails:** No dense iterator chains that obscure download state transitions. Use match arms for `DownloadState` transitions with explicit logging at each state change.

## BDD Scenario Inventory

| Feature File | Scenario | Business Outcome |
|-------------|----------|-----------------|
| `seg-download.feature` | Basic segmented download | User adds a URL, download completes with multiple connections |
| `seg-download.feature` | Resume interrupted download | User restarts after crash, download resumes from last checkpoint |
| `speed-limit.feature` | Speed limit enforced | Download speed stays within configured limit |
| `queue.feature` | Priority ordering | Critical downloads execute before Low priority ones |
| `checksum.feature` | Checksum verification | Download with checksum succeeds when hash matches, fails when mismatched |
| `metalink.feature` | Metalink download | User provides Metalink file, engine downloads from best source with checksum |
| `proxy.feature` | Proxy download | Download succeeds through configured HTTP proxy |
| `crash-recovery.feature` | Crash recovery | Engine recovers in-progress downloads after unclean shutdown |
| `progress.feature` | Progress reporting | Consumer receives progress events during download |

## Detailed Design

### DownloadEngine (engine.rs)

Central coordinator. Owns the download queue, rate limiter, storage, and event broadcaster. Provides the public API.

```rust
pub struct DownloadEngine {
    client: hpx::Client,
    queue: PriorityQueue,
    storage: Arc<dyn Storage>,
    rate_limiter: GlobalRateLimiter,
    events: broadcast::Sender<DownloadEvent>,
    active: scc::HashMap<DownloadId, JoinHandle<()>>,
}

impl DownloadEngine {
    pub fn builder() -> EngineBuilder { ... }
    pub async fn add(&self, request: DownloadRequest) -> Result<DownloadId> { ... }
    pub async fn pause(&self, id: DownloadId) -> Result<()> { ... }
    pub async fn resume(&self, id: DownloadId) -> Result<()> { ... }
    pub async fn remove(&self, id: DownloadId) -> Result<()> { ... }
    pub async fn status(&self, id: DownloadId) -> Result<DownloadStatus> { ... }
    pub fn subscribe(&self) -> broadcast::Receiver<DownloadEvent> { ... }
    pub async fn list(&self) -> Result<Vec<DownloadStatus>> { ... }
}
```

### DownloadRequest

```rust
pub struct DownloadRequest {
    pub url: Uri,
    pub destination: PathBuf,
    pub priority: DownloadPriority,
    pub checksum: Option<ChecksumSpec>,
    pub headers: HeaderMap,
    pub max_connections: Option<usize>,  // default from engine config
    pub speed_limit: Option<u64>,        // bytes/sec, per-download
    pub mirrors: Vec<Uri>,
}
```

### DownloadState (enum)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadState {
    Queued,
    Connecting,
    Downloading,
    Paused,
    Completed,
    Failed,
}
```

### DownloadEvent

```rust
#[derive(Debug, Clone)]
pub enum DownloadEvent {
    Added { id: DownloadId },
    Started { id: DownloadId },
    Progress { id: DownloadId, downloaded: u64, total: Option<u64> },
    StateChanged { id: DownloadId, state: DownloadState },
    Completed { id: DownloadId },
    Failed { id: DownloadId, error: String },
    Removed { id: DownloadId },
}
```

### DownloadPriority

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DownloadPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}
```

### Segmented Download Logic (segment.rs)

1. Send HEAD request to determine file size and check `Accept-Ranges: bytes` header.
2. If server supports ranges and file is larger than `min_segment_size` (default 1MB):
   - Calculate segment count: `min(max_connections, total_size / min_segment_size).max(1)`
   - Divide file into equal byte ranges.
   - Spawn one Tokio task per segment, each sending a GET with `Range: bytes=start-end` header.
   - Each task writes received bytes to the correct offset in the destination file via `tokio::fs::File::seek`.
   - Progress is aggregated across all segments and emitted as `DownloadEvent::Progress`.
3. If server does not support ranges or file is small: single-connection download.
4. On segment failure: retry with exponential backoff (max 3 attempts, 1s initial, 30s max, ±25% jitter). If all retries fail, mark download as `Failed`.

### Resume Logic

1. On download start, check for existing partial file and storage entry.
2. If ETag/Last-Modified matches server headers: resume from last completed segment.
3. If content has changed (ETag mismatch): delete partial file, restart from beginning.
4. Each segment's state (Pending/Downloading/Completed/Failed) is persisted to SQLite.

### Speed Limiting (speed.rs)

- Global rate limiter: token-bucket via `governor` crate, configured in `EngineConfig`.
- Per-download rate limiter: optional token-bucket, configured per `DownloadRequest`.
- Each segment task checks the rate limiter before reading the next chunk. If tokens are exhausted, the task sleeps until tokens are refilled.

### Priority Queue (queue.rs)

- Binary heap ordered by `DownloadPriority` (higher priority first).
- Within same priority, FIFO ordering.
- Concurrency semaphore limits the number of simultaneously active downloads (default: 5).
- When a download completes or is paused, the next highest-priority queued download is started.

### Metalink Parser (metalink.rs)

Metalink v4 (RFC 5854) is an XML format. Parser responsibilities:

1. Parse `<metalink>` root element.
2. Extract `<file>` elements with `name` attribute.
3. For each file, extract:
   - `<url priority="N">` elements (mirrors with priority).
   - `<hash type="sha-256">` elements (checksums).
   - `<size>` element (file size).
4. Produce a `Vec<DownloadRequest>` with mirrors and checksums pre-populated.

Parser implementation uses `quick-xml` (already a dependency in the workspace for XML parsing) rather than `winnow`, because Metalink is well-structured XML and `quick-xml` is more ergonomic for this use case.

### Checksum Verification (checksum.rs)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HashAlgorithm {
    Md5,
    Sha1,
    Sha256,
}

pub struct ChecksumSpec {
    pub algorithm: HashAlgorithm,
    pub expected: String,  // hex-encoded
}

impl ChecksumSpec {
    pub fn verify(&self, file: &Path) -> Result<bool> { ... }
}
```

Verification happens after download completion, before marking state as `Completed`. Uses `md-5`, `sha1`, `sha2` crates (all in workspace dependencies).

### Storage Trait (storage/mod.rs)

```rust
#[async_trait]
pub trait Storage: Send + Sync {
    async fn save(&self, download: &DownloadRecord) -> Result<()>;
    async fn load(&self, id: DownloadId) -> Result<Option<DownloadRecord>>;
    async fn list(&self) -> Result<Vec<DownloadRecord>>;
    async fn delete(&self, id: DownloadId) -> Result<()>;
    async fn update_progress(&self, id: DownloadId, segments: &[SegmentState]) -> Result<()>;
}
```

### SQLite Storage (storage/sqlite.rs)

- Uses `sqlx` with SQLite backend (runtime queries, not compile-time macros, per AGENTS.md).
- Schema:
  - `downloads` table: id, url, destination, state, priority, etag, last_modified, content_length, created_at, updated_at.
  - `segments` table: download_id, index, start, end, state, bytes_downloaded.
- WAL mode for concurrent read access during writes.
- On engine startup, loads all `Downloading` and `Paused` downloads for crash recovery.

### CLI (cli.rs, behind `cli` feature)

- Uses `clap` for argument parsing.
- Commands: `add <url>`, `pause <id>`, `resume <id>`, `remove <id>`, `list`, `status <id>`.
- Creates `DownloadEngine` with default config, runs commands against it.
- Progress display via `indicatif` progress bars (optional `progress` feature).

### Error Types (error.rs)

```rust
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("HTTP error: {0}")]
    Http(#[from] hpx::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("Download not found: {0}")]
    NotFound(DownloadId),

    #[error("Server does not support range requests")]
    NoRangeSupport,

    #[error("Metalink parse error: {0}")]
    MetalinkParse(String),

    #[error("Rate limit exceeded")]
    RateLimited,

    #[error("Download already exists: {0}")]
    AlreadyExists(DownloadId),

    #[error("Download is not in {expected} state, current state: {actual}")]
    InvalidState { expected: String, actual: String },
}
```

### Configuration

```rust
pub struct EngineConfig {
    pub max_concurrent_downloads: usize,      // default: 5
    pub max_connections_per_download: usize,   // default: 5
    pub min_segment_size: u64,                 // default: 1MB
    pub global_speed_limit: Option<u64>,       // bytes/sec, None = unlimited
    pub retry_max_attempts: u32,               // default: 3
    pub retry_initial_delay: Duration,         // default: 1s
    pub retry_max_delay: Duration,             // default: 30s
    pub retry_jitter: f64,                     // default: 0.25
    pub storage_path: PathBuf,                 // default: ./hpx-dl.db
    pub client: Option<hpx::Client>,           // None = create default
}
```

### Proxy Support

Leverages hpx's existing proxy infrastructure. `DownloadRequest` can specify a proxy via `hpx::Proxy` which is passed through to `hpx::ClientBuilder::proxy()`. The engine's client configuration supports:

- HTTP/HTTPS proxies
- SOCKS4/5 proxies (via hpx's `socks` feature)
- System proxy auto-detection (via hpx's `system-proxy` feature)
- Per-download proxy override

## Verification & Testing Strategy

### Unit Tests

Each module has colocated `#[cfg(test)]` tests:

- `segment.rs`: segment calculation logic, range header generation, offset arithmetic.
- `speed.rs`: token-bucket refill and consumption.
- `queue.rs`: priority ordering, FIFO within priority, concurrency semaphore.
- `checksum.rs`: hash computation, hex encoding comparison.
- `metalink.rs`: XML parsing with valid/invalid/malformed input.
- `storage/sqlite.rs`: CRUD operations, schema creation, WAL behavior.

### Property Tests (proptest)

- **Segment calculation**: For any file size and max connections, segment boundaries must be contiguous, non-overlapping, and cover the entire file.
- **Metalink parsing**: Round-trip property — parse a Metalink XML, serialize back, parse again, assert equivalence.
- **Priority queue**: For any sequence of add/pause/resume operations, the next item popped must have the highest priority among active items.

### Fuzz Testing (cargo-fuzz, conditional)

- **Metalink XML parser**: Fuzz the XML input to ensure no panics on malformed input. This is justified because Metalink files are untrusted user-provided data.

### Integration Tests

- `crates/hpx-dl/tests/`: Use a mock HTTP server (via `hpx` test support or `axum` test server) to test:
  - Full download flow: add → start → progress → complete.
  - Resume flow: start → crash (drop engine) → restart → resume → complete.
  - Speed limiting: verify download speed stays within configured bounds.
  - Checksum verification: success and failure cases.

### BDD Scenarios

Gherkin scenarios in `features/` directory cover all user-visible behaviors listed in the BDD Scenario Inventory.

### Runtime Verification

For tasks that change runtime behavior:

- `tail -n 50` on any engine log output (when tracing is enabled).
- Verify download file exists and has expected size after completion.
- Verify SQLite state matches expected records.

## Implementation Plan

### Phase 1: Foundation (Tasks 1.1-2.3)

Set up the crate, core types, error module, and engine skeleton with dependency injection points.

### Phase 2: Segmented Download (Tasks 3.1-3.4)

Implement the core download loop: HEAD probing, segment calculation, concurrent range fetches, file assembly, resume with content validation.

### Phase 3: Queue & Rate Limiting (Tasks 4.1-4.2)

Add priority queue, concurrency semaphore, global and per-download speed limiting.

### Phase 4: Checksum, Metalink, Proxy (Tasks 5.1-5.3)

Add checksum verification, Metalink v4 parser, and proxy configuration passthrough.

### Phase 5: Persistence & Events (Tasks 6.1-6.2)

Implement SQLite storage, crash recovery, and broadcast event system.

### Phase 6: CLI & Polish (Tasks 7.1-7.2)

Add optional CLI binary with clap, BDD harness with cucumber, final lint/format pass.
