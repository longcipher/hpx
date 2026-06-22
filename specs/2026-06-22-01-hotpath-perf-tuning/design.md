# Design: Hotpath Profiling & Performance Tuning

| Metadata | Details |
| :--- | :--- |
| **Status** | Draft |
| **Created** | 2026-06-22 |
| **Mode** | Full |
| **Priority** | P1 |
| **Planned at** | commit `b9b0830`, 2026-06-22 |

## Summary

> This spec adds `hotpath` instrumentation to the `hpx` and `hpx-dl` crates (which currently have zero profiling), wraps `ProxyPool::select`'s `Matcher` in `Arc` to eliminate per-request deep clones, consolidates double-iteration patterns in the download engine's segment state code, switches segment state `HashSet` to `ahash`, and increases the checksum buffer for large-file throughput.

## Why this matters

> `hotpath` is already a workspace dependency and `yawc` uses it on 6 functions. But the two crates with the most critical hot paths — `hpx` (connection pool, request pipeline, TLS handshake) and `hpx-dl` (segment download, speed limiter, checksum) — have zero instrumentation. Without profiling these paths, performance work is guesswork. The other findings are small but free wins that reduce allocation pressure and syscall count on hot paths.

## Approach

> All changes are additive and localized. `hotpath` integration follows the exact pattern already established in `yawc` (optional dep + feature flag + `#[cfg_attr]`). The `Arc<Matcher>` change is a one-line refactor in `proxy_pool.rs`. The iteration consolidation and ahash switch are mechanical. No architectural changes.

## Findings

### Finding 1 (HOT-01): Add hotpath instrumentation to `hpx` crate

- **Category:** performance
- **Impact:** HIGH — connection pool, request building, and TLS handshake are invisible to profiling. Without instrumentation, perf regressions in the core HTTP client go undetected.
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-01]:** The `hpx` crate shall have an optional `hotpath` feature flag that depends on `dep:hotpath`.
- **[REQ-02]:** Critical hot-path functions shall have `#[cfg_attr(feature = "hotpath", hotpath::measure)]`.
- **[REQ-03]:** All existing tests shall pass with and without the `hotpath` feature enabled.

#### Current state

No `hotpath` in `hpx/Cargo.toml` features or dependencies. No `#[cfg_attr(feature = "hotpath", ...)]` annotations anywhere in `crates/hpx/src/`.

#### Approach

1. Add `hotpath = { workspace = true, optional = true }` to `[dependencies]` in `crates/hpx/Cargo.toml`.
2. Add `hotpath = ["dep:hotpath"]` to `[features]`.
3. Add `#[cfg_attr(feature = "hotpath", hotpath::measure)]` to:
   - `client/http/client.rs` — `HttpClient::call` (the main request path)
   - `client/conn/connector.rs` — `Connector::connect` (connection establishment)
   - `proxy_pool.rs` — `ProxyPool::select` and `ProxyPoolService::call`
   - `hash.rs` — `HashMemo::hash` (hash computation hot path)

#### Architecture Decisions (MADR Format)

- **AD-01:** Use the same `cfg_attr` pattern as `yawc` — zero-cost when feature disabled, no runtime overhead in production builds.

### Finding 2 (HOT-02): Add hotpath instrumentation to `hpx-dl` crate

- **Category:** performance
- **Impact:** HIGH — the download engine is the core product. Segment download, speed limiting, and checksum verification have zero profiling.
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-01]:** The `hpx-dl` crate shall have an optional `hotpath` feature flag.
- **[REQ-02]:** Critical hot-path functions shall have `#[cfg_attr(feature = "hotpath", hotpath::measure)]`.
- **[REQ-03]:** All existing tests shall pass with and without the `hotpath` feature enabled.

#### Current state

No `hotpath` in `hpx-dl/Cargo.toml`. No `#[cfg_attr(feature = "hotpath", ...)]` in any `crates/hpx-dl/src/` file.

#### Approach

1. Add `hotpath = { workspace = true, optional = true }` to `[dependencies]`.
2. Add `hotpath = ["dep:hotpath"]` to `[features]`.
3. Add `#[cfg_attr(feature = "hotpath", hotpath::measure)]` to:
   - `segment.rs` — `download_segment_with_options`
   - `speed.rs` — `SpeedLimiter::refill_tokens`, `SpeedLimiter::try_consume`, `SpeedLimiter::wait_for`
   - `engine.rs` — `build_segment_states`, `DownloadEngine::submit`
   - `checksum.rs` — `compute_checksum`, `hash_file`

#### Architecture Decisions (MADR Format)

- **AD-01:** Same `cfg_attr` pattern as `yawc` and `hpx`. Feature-gated, zero-cost when disabled.

### Finding 3 (PERF-15): Wrap `Matcher` in `Arc` in `ProxyPool`

- **Category:** performance
- **Impact:** MEDIUM — every HTTP request through a proxy pool clones a `Matcher` (contains URL parsing state). `Arc::clone` is O(1) pointer bump.
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-01]:** `ProxyPool::select` shall return `Arc<Matcher>` instead of `Matcher`.
- **[REQ-02]:** `Selection` shall hold `Arc<Matcher>` instead of `Matcher`.
- **[REQ-03]:** All existing proxy pool tests shall pass.

#### Current state

```rust
// proxy_pool.rs:185
matcher: self.inner.matchers[index].clone(),
```

`Matcher` is cloned on every `select()` call, which happens on every request.

#### Approach

1. Change `Inner.matchers` from `Vec<Matcher>` to `Vec<Arc<Matcher>>`.
2. Change `Selection.matcher` from `Matcher` to `Arc<Matcher>`.
3. `select()` returns `Arc::clone(&self.inner.matchers[index])` — O(1).

### Finding 4 (PERF-13): Single-pass iteration in `resume_state_from_segments`

- **Category:** performance
- **Impact:** LOW — iterates segments slice twice. For 100 segments this is negligible; for 10K segments it's measurable.
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-01]:** `resume_state_from_segments` shall compute `completed_segments` and `bytes_completed` in one iteration.

#### Current state

```rust
// segment.rs:658-671
let completed_segments: Vec<u32> = segments.iter()
    .filter(|s| s.state == SegmentStatus::Completed)
    .map(|s| s.index).collect();
let bytes_completed: u64 = segments.iter()
    .filter(|s| s.state == SegmentStatus::Completed)
    .map(|s| s.bytes_downloaded).sum();
```

#### Approach

Single `fold` or `for` loop collecting both values.

### Finding 5 (PERF-14): Use `ahash::HashSet` for segment state building

- **Category:** performance
- **Impact:** LOW — `std::collections::HashSet` uses SipHash which is slower than `ahash` for integer keys. The project already has `ahash` as a dependency and a pre-seeded `HASHER` in `hash.rs`.
- **Effort:** S

#### Current state

```rust
// engine.rs:1079
let completed: std::collections::HashSet<u32> = completed_indices.iter().copied().collect();
// segment.rs:685
let completed: HashSet<u32> = completed_indices.iter().copied().collect();
```

`segment.rs:685` already uses the project's `HashSet` type alias (which is `ahash`), but `engine.rs:1079` uses `std::collections::HashSet` directly.

#### Approach

Change `engine.rs:1079` to use `crate::hash::HashSet` (or `ahash::HashSet` with the project's `HASHER`).

### Finding 6 (PERF-16): Increase checksum buffer to 64 KiB

- **Category:** performance
- **Impact:** LOW — 8 KiB buffer means more `read()` syscalls for large files. 64 KiB reduces syscall count 8x.
- **Effort:** S

#### Current state

```rust
// checksum.rs:36
let mut buf = [0u8; 8192];
```

#### Approach

Change to `[0u8; 65536]`. The buffer is stack-allocated, so 64 KiB is safe.

## Architecture Decisions

### AD-01: hotpath feature flag pattern

- **Context:** `yawc` already uses `hotpath` with an optional feature flag and `#[cfg_attr(feature = "hotpath", hotpath::measure)]`. The same pattern should be used in `hpx` and `hpx-dl`.
- **Decision:** Use the identical pattern: optional dep + feature flag + `cfg_attr`. Zero-cost when disabled.
- **Consequences:** Adds a feature flag to two more crates. Consistent with existing workspace conventions.

### AD-02: Arc<Matcher> in ProxyPool

- **Context:** `Matcher` is cloned on every request through a proxy pool. The clone is a deep copy of URL parsing state.
- **Decision:** Wrap in `Arc` for O(1) clone. `Matcher` is read-only after pool construction.
- **Consequences:** One extra indirection on access (negligible). No API change for callers.

## BDD/TDD Strategy

- **Primary Language:** Rust
- **BDD Runner:** N/A (no new BDD scenarios — these are internal optimizations)
- **Unit Test Command:** `cargo nextest run --workspace --all-features`
- **Benchmark Command:** `cargo bench --workspace --all-features`
- **Feature Files:** `specs/2026-06-22-01-hotpath-perf-tuning/features/performance.feature`
- **Outside-in Loop:** hotpath feature enables profiling → benchmark → optimize → re-benchmark

## Verification

| Purpose   | Command                           | Expected on success |
|-----------|-----------------------------------|---------------------|
| Install   | `cargo build --workspace --all-features` | exit 0         |
| Lint      | `cargo +nightly clippy --all -- -D warnings` | exit 0, no errors |
| Format    | `cargo +nightly fmt --all -- --check` | exit 0          |
| Tests     | `cargo nextest run --workspace --all-features` | all pass    |
| Bench     | `cargo bench --workspace --all-features` | exit 0          |
| Hotpath   | `cargo bench --package hpx-yawc --features hotpath` | hotpath output shows function timings |
