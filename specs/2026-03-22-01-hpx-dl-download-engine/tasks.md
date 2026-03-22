# hpx-dl Download Engine — Tasks

| Metadata | Details |
| :--- | :--- |
| **Design Doc** | specs/2026-03-22-01-hpx-dl-download-engine/design.md |
| **Status** | Planning |

## Summary & Timeline

| Phase | Description | Tasks | Estimated Scope |
|-------|-------------|-------|-----------------|
| Phase 1 | Foundation (crate setup, core types, engine skeleton) | Task 1.1-2.3 | ~3 days |
| Phase 2 | Segmented download (range requests, resume, file assembly) | Task 3.1-3.4 | ~4 days |
| Phase 3 | Queue & rate limiting (priority queue, token bucket) | Task 4.1-4.2 | ~2 days |
| Phase 4 | Checksum, Metalink, proxy | Task 5.1-5.3 | ~3 days |
| Phase 5 | Persistence & events (SQLite, crash recovery, broadcast) | Task 6.1-6.2 | ~2 days |
| Phase 6 | CLI, BDD harness, polish | Task 7.1-7.2 | ~2 days |

## Tasks

### Task 1.1: Create hpx-dl crate skeleton

> **Context:** New crate in the hpx workspace. Must follow AGENTS.md conventions: `cargo add` for dependencies, workspace versions, `thiserror` for library errors, `tracing` for logging.
> **Verification:** `cargo check -p hpx-dl` succeeds.
> **Requirement Coverage:** N1, N2, N3, N4, N5, N6, N7
> **Scenario Coverage:** N/A (infrastructure)

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** N/A (new crate)
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Create `crates/hpx-dl/` directory with `Cargo.toml` using workspace conventions
- [x] Add `lib.rs` with module declarations and public re-exports
- [x] Add `error.rs` with `DownloadError` enum (thiserror)
- [x] Add workspace dependency: `hpx = { path = "../hpx" }` via `cargo add`
- [x] Add core dependencies via `cargo add -p hpx-dl --workspace`: `tokio`, `serde`, `serde_json`, `uuid`, `bytes`, `tracing`, `thiserror`
- [x] Set feature flags: `http` (default), `metalink`, `storage`, `progress`, `cli`
- [x] Configure `Cargo.toml` metadata matching workspace conventions
- [x] Verification: `cargo check -p hpx-dl` compiles without errors
- [x] Verification: `cargo clippy -p hpx-dl -- -D warnings` passes

### Task 1.2: Define core domain types

> **Context:** Core types used throughout the engine: `DownloadId`, `DownloadState`, `DownloadPriority`, `DownloadEvent`, `DownloadRequest`, `DownloadStatus`, `ChecksumSpec`, `HashAlgorithm`.
> **Verification:** All types compile, derive correct traits, unit tests pass.
> **Requirement Coverage:** R4, R5, R10
> **Scenario Coverage:** queue-priority-ordering, checksum-verification

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** N/A (new types)
- **Simplification Focus:** Clear named types with exhaustive derives
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Implement `DownloadId` as a newtype over `uuid::Uuid` with `Display`, `Serialize`, `Deserialize`
- [x] Implement `DownloadState` enum: `Queued`, `Connecting`, `Downloading`, `Paused`, `Completed`, `Failed`
- [x] Implement `DownloadPriority` enum: `Low`, `Normal`, `High`, `Critical` with `Ord` for heap ordering
- [x] Implement `DownloadEvent` enum with all event variants
- [x] Implement `DownloadRequest` struct with builder pattern
- [x] Implement `DownloadStatus` struct (id, url, state, progress, priority)
- [x] Implement `ChecksumSpec` and `HashAlgorithm` types
- [x] Write unit tests for type construction, serialization, priority ordering
- [x] Verification: `cargo nextest run -p hpx-dl` passes

### Task 2.1: Implement EngineBuilder and EngineConfig

> **Context:** The `DownloadEngine` is the central API surface. It must accept an `hpx::Client` (or build a default one), configuration, and a `Storage` implementation. Follows DIP — engine depends on traits, not concrete types.
> **Verification:** Engine builds with default and custom config.
> **Requirement Coverage:** R10, N1, N7
> **Scenario Coverage:** All (engine is the entry point)

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** N/A (new construction)
- **Simplification Focus:** Builder pattern with sensible defaults, no magic numbers
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Implement `EngineConfig` struct with all fields and defaults (see design.md)
- [x] Implement `EngineBuilder` with fluent API: `client()`, `config()`, `storage()`, `max_concurrent()`, etc.
- [x] Implement `EngineBuilder::build() -> Result<DownloadEngine>`
- [x] Create `DownloadEngine` struct skeleton with owned fields (client, queue, storage, rate_limiter, events, active)
- [x] Write unit tests for builder construction with default and custom configs
- [x] Verification: `cargo nextest run -p hpx-dl` passes

### Task 2.2: Define Storage trait and in-memory implementation

> **Context:** The `Storage` trait abstracts persistence. An in-memory implementation is needed for testing and as a reference implementation.
> **Verification:** In-memory storage passes all trait method tests.
> **Requirement Coverage:** R8, R10
> **Scenario Coverage:** crash-recovery

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** N/A (new trait)
- **Simplification Focus:** Minimal trait surface (5 methods), clear semantics
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Define `Storage` trait in `storage/mod.rs` with async methods: `save`, `load`, `list`, `delete`, `update_progress`
- [x] Define `DownloadRecord` and `SegmentState` types for storage
- [x] Implement `MemoryStorage` (behind `test` feature) using `scc::HashMap`
- [x] Write unit tests covering all `Storage` trait methods via `MemoryStorage`
- [x] Verification: `cargo nextest run -p hpx-dl` passes

### Task 2.3: Implement event broadcast system

> **Context:** `DownloadEngine` emits events via `tokio::sync::broadcast`. Consumers call `engine.subscribe()` to get a `Receiver<DownloadEvent>`.
> **Verification:** Events are received by subscribers in correct order.
> **Requirement Coverage:** R9
> **Scenario Coverage:** progress-reporting

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** N/A (new event system)
- **Simplification Focus:** Simple broadcast channel, no custom event bus
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Implement `EventBroadcaster` wrapping `broadcast::Sender<DownloadEvent>`
- [x] Wire `EventBroadcaster` into `DownloadEngine`
- [x] Implement `engine.subscribe() -> broadcast::Receiver<DownloadEvent>`
- [x] Write unit tests: add subscriber, emit events, verify receipt
- [x] Verification: `cargo nextest run -p hpx-dl` passes

### Task 3.1: Implement HEAD probing and segment calculation

> **Context:** Before downloading, send a HEAD request to determine file size and check `Accept-Ranges` support. Calculate optimal segment count and boundaries.
> **Verification:** Segment boundaries are correct for various file sizes and connection limits.
> **Requirement Coverage:** R1
> **Scenario Coverage:** seg-download-basic

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** N/A (new logic)
- **Simplification Focus:** Named functions for segment calculation, no dense one-liners
- **Advanced Test Coverage:** Property (proptest for segment boundary contiguity and coverage)
- **Status:** 🟢 DONE
- [x] Implement `probe_remote(client, url) -> Result<RemoteInfo>` — sends HEAD, extracts `Content-Length`, `Accept-Ranges`, `ETag`, `Last-Modified`
- [x] Implement `calculate_segments(file_size, max_connections, min_segment_size) -> Vec<SegmentRange>` — returns contiguous non-overlapping byte ranges
- [x] Write property test: for any (file_size, max_connections, min_segment_size), segments are contiguous, non-overlapping, cover [0, file_size)
- [x] Write unit tests for edge cases: 0-byte file, single byte, exact multiple of segment size
- [x] Verification: `cargo nextest run -p hpx-dl` passes

### Task 3.2: Implement single-segment download

> **Context:** Download a single byte range from a URL using `hpx::Client`. Write received bytes to a file at the correct offset. This is the building block for multi-segment downloads.
> **Verification:** Single segment download writes correct bytes to correct offset.
> **Requirement Coverage:** R1
> **Scenario Coverage:** seg-download-basic

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** N/A (new logic)
- **Simplification Focus:** Clear loop: read chunk → check rate limit → seek → write → update progress
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Implement `download_segment(client, url, range, file, offset, speed_limiter, event_tx) -> Result<()>` — sends GET with `Range` header, streams `bytes_stream()`, writes to file
- [x] Handle `206 Partial Content` vs `200 OK` (server may not honor range)
- [x] Write unit tests with mock HTTP server returning known byte ranges
- [x] Verification: `cargo nextest run -p hpx-dl` passes

### Task 3.3: Implement multi-segment concurrent download

> **Context:** Spawn multiple `download_segment` tasks, one per segment range. Aggregate progress across all segments. Write to a single destination file with `seek`.
> **Verification:** Complete file is correctly assembled from segments.
> **Requirement Coverage:** R1
> **Scenario Coverage:** seg-download-basic

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** N/A (new logic)
- **Simplification Focus:** Use `tokio::task::JoinSet` for segment tasks, aggregate progress with `AtomicU64`
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Implement `SegmentDownloader` struct: manages concurrent segment tasks
- [x] Implement `SegmentDownloader::download() -> Result<()>` — spawns segment tasks, awaits completion
- [x] Implement progress aggregation: shared `AtomicU64` for total bytes downloaded, emit `DownloadEvent::Progress`
- [x] Handle segment failures: retry with exponential backoff (max 3 attempts, 1s initial, 30s max, ±25% jitter)
- [x] Write integration test with mock server serving a large file, verify complete download
- [x] Verification: `cargo nextest run -p hpx-dl` passes

### Task 3.4: Implement resume with content validation

> **Context:** On restart, check for existing partial file. Send HEAD to get current ETag/Last-Modified. If content unchanged, resume from last completed segment. If changed, restart.
> **Verification:** Download resumes correctly after simulated interruption.
> **Requirement Coverage:** R2
> **Scenario Coverage:** seg-download-resume

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** N/A (new logic)
- **Simplification Focus:** Clear match on content validation: unchanged → resume, changed → restart
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Implement `check_resume(client, url, existing_file, storage) -> Result<ResumeState>` — compares ETag/Last-Modified
- [x] Integrate resume check into `SegmentDownloader::download()` — skip completed segments
- [x] Handle ETag mismatch: delete partial file, emit event, restart
- [x] Write integration test: download 50% → drop engine → recreate → resume → verify complete file
- [x] Verification: `cargo nextest run -p hpx-dl` passes

### Task 4.1: Implement global and per-download speed limiting

> **Context:** Token-bucket rate limiting via `governor` crate. Global limiter shared across all downloads; per-download limiter optional per request.
> **Verification:** Download speed stays within configured bounds.
> **Requirement Coverage:** R3, N6
> **Scenario Coverage:** speed-limit-enforced

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** N/A (new logic)
- **Simplification Focus:** Separate global and per-download limiters, compose with min()
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Add `governor` dependency via `cargo add -p hpx-dl --workspace`
- [x] Implement `GlobalRateLimiter` wrapping `governor::RateLimiter`
- [x] Implement `PerDownloadRateLimiter` (optional, per-request)
- [x] Integrate rate limiting into `download_segment` — check limiter before each chunk read
- [x] Write unit test: download with speed limit, measure elapsed time, verify speed is within 10% of limit
- [x] Verification: `cargo nextest run -p hpx-dl` passes

### Task 4.2: Implement priority download queue

> **Context:** Binary heap ordered by `DownloadPriority`. Concurrency semaphore limits active downloads. When a download completes, next highest-priority item starts.
> **Verification:** Higher priority downloads start before lower priority ones.
> **Requirement Coverage:** R4
> **Scenario Coverage:** queue-priority-ordering

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** N/A (new logic)
- **Simplification Focus:** Standard binary heap with explicit priority comparison, no custom sorting
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Implement `PriorityQueue` with binary heap, ordered by `DownloadPriority` (higher first)
- [x] Implement concurrency semaphore (`tokio::sync::Semaphore`) to limit active downloads
- [x] Implement `engine.add()` — inserts into queue, starts if semaphore permit available
- [x] Implement `engine.pause()` / `engine.resume()` — release/acquire semaphore permit, re-order queue
- [x] Write unit test: add 3 downloads (Low, Critical, Normal), verify execution order is Critical → Normal → Low
- [x] Verification: `cargo nextest run -p hpx-dl` passes

### Task 5.1: Implement checksum verification

> **Context:** After download completion, verify file checksum against expected value. Support MD5, SHA-1, SHA-256.
> **Verification:** Matching checksum passes, mismatched checksum fails with clear error.
> **Requirement Coverage:** R5
> **Scenario Coverage:** checksum-verification

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** N/A (new logic)
- **Simplification Focus:** Separate compute function per algorithm, shared verify function
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Add `md-5`, `sha1`, `sha2` dependencies via `cargo add -p hpx-dl --workspace`
- [x] Implement `compute_checksum(file, algorithm) -> Result<String>` — reads file in chunks, computes hex-encoded hash
- [x] Implement `ChecksumSpec::verify(file) -> Result<()>` — computes and compares, returns `DownloadError::ChecksumMismatch` on failure
- [x] Integrate verification into download completion flow — verify before marking `Completed`
- [x] Write unit tests: correct hash passes, wrong hash fails, missing file fails
- [x] Write property test: for random data, compute checksum, verify passes with correct spec
- [x] Verification: `cargo nextest run -p hpx-dl` passes

### Task 5.2: Implement Metalink v4 parser

> **Context:** Parse Metalink XML (RFC 5854) to extract file URLs with priorities, checksums, and file sizes. Produce `Vec<DownloadRequest>`.
> **Verification:** Valid Metalink produces correct download requests; malformed input returns parse error.
> **Requirement Coverage:** R6
> **Scenario Coverage:** metalink-download

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** N/A (new parser)
- **Simplification Focus:** Use `quick-xml` for structured XML parsing; no ad-hoc string manipulation
- **Advanced Test Coverage:** Property (round-trip parse), Fuzz (conditional — see below)
- **Status:** 🟢 DONE
- [x] Add `quick-xml` dependency via `cargo add -p hpx-dl --workspace`
- [x] Implement `parse_metalink(xml: &[u8]) -> Result<Vec<MetalinkFile>>` — parses `<metalink>`, `<file>`, `<url>`, `<hash>`, `<size>`
- [x] Implement `MetalinkFile::into_download_requests() -> Vec<DownloadRequest>` — converts to engine-compatible requests with mirrors and checksums
- [x] Write unit tests: valid Metalink v4, Metalink v3 (compat), empty, malformed XML
- [x] Write property test: generate random valid Metalink XML, parse, assert round-trip equivalence
- [x] Conditional: set up `cargo-fuzz` target for `parse_metalink` to ensure no panics on untrusted input
- [x] Verification: `cargo nextest run -p hpx-dl` passes

### Task 5.3: Implement proxy passthrough

> **Context:** Leverage hpx's existing proxy infrastructure. Pass proxy config from `DownloadRequest` through to `hpx::Client`.
> **Verification:** Download succeeds through configured HTTP proxy.
> **Requirement Coverage:** R7
> **Scenario Coverage:** proxy-download

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** N/A (new integration)
- **Simplification Focus:** Delegate to hpx, no custom proxy logic
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Add `proxy` field to `DownloadRequest` (type: `Option<ProxyConfig>`)
- [x] In `SegmentDownloader`, create per-download client with proxy override if specified
- [x] Write unit test: configure mock proxy, download through it, verify request headers
- [x] Verification: `cargo nextest run -p hpx-dl` passes

### Task 6.1: Implement SQLite storage with crash recovery

> **Context:** Replace `MemoryStorage` with SQLite-backed `SqliteStorage`. Schema: `downloads` and `segments` tables. WAL mode. Load in-progress downloads on startup.
> **Verification:** Downloads persist across engine restarts; crash recovery restores state.
> **Requirement Coverage:** R8
> **Scenario Coverage:** crash-recovery

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** N/A (new persistence layer)
- **Simplification Focus:** Runtime queries (`sqlx::query_as`), no compile-time macros
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Add `sqlx` dependency with `sqlite` feature via `cargo add -p hpx-dl --workspace`
- [x] Define schema migration in `storage/sqlite.rs`: `downloads` table, `segments` table
- [x] Implement `SqliteStorage` with all `Storage` trait methods
- [x] Implement `DownloadEngine::recover() -> Result<()>` — loads paused/downloading downloads, restores state
- [x] Write integration test: create engine → add download → drop engine → create new engine → verify download record exists
- [x] Verification: `cargo nextest run -p hpx-dl` passes

### Task 6.2: Implement progress event emission

> **Context:** During download, emit `DownloadEvent::Progress` events at regular intervals. Emit state transitions (`StateChanged`, `Completed`, `Failed`).
> **Verification:** Subscribers receive all expected events during a complete download.
> **Requirement Coverage:** R9
> **Scenario Coverage:** progress-reporting

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** N/A (new event integration)
- **Simplification Focus:** Emit events at natural boundaries (state transitions, progress ticks)
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Wire `EventBroadcaster::emit()` into `SegmentDownloader` — emit `Progress` on each chunk write
- [x] Emit `StateChanged` on every state transition
- [x] Emit `Completed` / `Failed` on download finish
- [x] Implement configurable progress tick interval (default: 500ms)
- [x] Write integration test: subscribe → download → collect events → assert contains Added, Started, Progress (multiple), Completed
- [x] Verification: `cargo nextest run -p hpx-dl` passes

### Task 7.1: Implement CLI binary

> **Context:** Optional CLI behind `cli` feature flag. Uses `clap` for argument parsing. Commands: `add`, `pause`, `resume`, `remove`, `list`, `status`.
> **Verification:** CLI commands execute against the engine.
> **Requirement Coverage:** R11
> **Scenario Coverage:** cli-basic-download

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** N/A (new binary)
- **Simplification Focus:** Thin CLI wrapper around engine API, no business logic in CLI
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Add `clap` dependency via `cargo add -p hpx-dl --workspace`
- [x] Create `src/cli.rs` with `Cli` struct and `Command` enum (clap derive)
- [x] Implement command handlers: each calls corresponding `DownloadEngine` method
- [x] Add binary target in `Cargo.toml` behind `cli` feature
- [x] Write unit tests for CLI argument parsing
- [x] Verification: `cargo build -p hpx-dl --features cli` succeeds
- [x] Runtime Verification: `cargo run -p hpx-dl --features cli -- --help` prints usage

### Task 7.2: Set up BDD harness and write feature files

> **Context:** Set up `cucumber` BDD runner for hpx-dl. Write Gherkin feature files covering all user-visible scenarios.
> **Verification:** BDD scenarios pass against the implemented engine.
> **Requirement Coverage:** All R1-R11
> **Scenario Coverage:** All scenarios

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** N/A (test infrastructure)
- **Simplification Focus:** Thin step definitions that delegate to engine API
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Add `cucumber` dependency via `cargo add -p hpx-dl --workspace`
- [x] Create `tests/cucumber/` directory with step definitions
- [x] Implement step definitions for all feature scenarios
- [x] Set up mock HTTP server for BDD tests (axum or hpx test support)
- [x] Write step definitions: Given (engine setup, server state), When (add/pause/resume), Then (verify state/events/file)
- [x] Verification: `cargo test --features cli -p hpx-dl --test cucumber` passes
- [x] Final verification: `just format && just lint && just test` all pass

## Definition of Done

- [ ] All tasks completed with status 🟢 DONE
- [ ] `cargo nextest run --workspace --all-features` passes
- [ ] `cargo +nightly clippy --all -- -D warnings` passes
- [ ] All BDD scenarios pass
- [ ] `just format` applied
- [ ] `just lint` passes
- [ ] `just test` passes
- [ ] `just build-docs` passes
- [ ] All property tests pass with default proptest config
- [ ] Fuzz target (Metalink parser) runs for 10,000 iterations without panic
