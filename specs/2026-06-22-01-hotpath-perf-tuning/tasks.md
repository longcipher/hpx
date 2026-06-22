# Tasks: Hotpath Profiling & Performance Tuning

Planned at commit `b9b0830`. Execute via `/pb-build 2026-06-22-01-hotpath-perf-tuning`.

---

## Phase 1: Hotpath instrumentation (S effort, HIGH impact)

### Task 1.1: Add hotpath feature flag and dependency to hpx crate

> **Context:** The `hpx` crate has zero profiling instrumentation. `hotpath` is already a workspace dependency. Adding the feature flag follows the exact pattern from `yawc`.
> **Verification:** `cargo check -p hpx --features hotpath` compiles cleanly.
> **Scenario Coverage:** `features/performance.feature` — "hpx crate exposes hotpath feature flag"

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — no functional change, feature-gated
- **Simplification Focus:** Minimal change — one optional dep, one feature line
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx/Cargo.toml` to confirm no existing hotpath usage
- [x] Step 2: Add `hotpath = { workspace = true, optional = true }` to `[dependencies]`
- [x] Step 3: Add `hotpath = ["dep:hotpath"]` to `[features]`
- [x] Step 4: Run `cargo check -p hpx --features hotpath` — compiles
- [x] Step 5: Run `cargo check -p hpx` — still compiles without feature
- [x] BDD Verification: N/A — Cargo.toml change only
- [x] Advanced Test Verification: `cargo nextest run -p hpx --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx --all-features -- -D warnings`

### Task 1.2: Add hotpath::measure to hpx hot-path functions

> **Context:** With the feature flag in place, annotate the critical hot-path functions.
> **Verification:** `cargo bench -p hpx --features hotpath` shows function-level timing in hotpath output.
> **Scenario Coverage:** `features/performance.feature` — "hpx connection pool acquire is instrumented", "hpx request building is instrumented"

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — zero-cost when feature disabled
- **Simplification Focus:** Use `#[cfg_attr(feature = "hotpath", hotpath::measure)]` — same pattern as yawc
- **Status:** 🟢 DONE
- [x] Step 1: Add `#[cfg_attr(feature = "hotpath", hotpath::measure)]` to `ProxyPool::select` in `crates/hpx/src/proxy_pool.rs:171`
- [x] Step 2: Add `#[cfg_attr(feature = "hotpath", hotpath::measure)]` to `ProxyPoolService::call` in `crates/hpx/src/proxy_pool.rs:261`
- [x] Step 3: Add `#[cfg_attr(feature = "hotpath", hotpath::measure)]` to `HashMemo::hash` in `crates/hpx/src/hash.rs:68`
- [x] Step 4: Find and annotate the `HttpClient::call` method in `crates/hpx/src/client/http/client.rs` (or the main request send path)
- [x] Step 5: Find and annotate the `Connector::connect` method in `crates/hpx/src/client/conn/connector.rs`
- [x] Step 6: Run `cargo check -p hpx --features hotpath` — compiles
- [x] Step 7: Run `cargo bench -p hpx --features hotpath --bench http_throughput -- --warm-up-time 1 --measurement-time 2` — hotpath output appears
- [x] BDD Verification: N/A — instrumentation only
- [x] Advanced Test Verification: `cargo nextest run -p hpx --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx --all-features -- -D warnings`

### Task 1.3: Add hotpath feature flag and dependency to hpx-dl crate

> **Context:** The `hpx-dl` crate has zero profiling. Same pattern as hpx.
> **Verification:** `cargo check -p hpx-dl --features hotpath` compiles cleanly.
> **Scenario Coverage:** `features/performance.feature` — "hpx-dl crate exposes hotpath feature flag"

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** Minimal — one optional dep, one feature line
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx-dl/Cargo.toml` to confirm no existing hotpath usage
- [x] Step 2: Add `hotpath = { workspace = true, optional = true }` to `[dependencies]`
- [x] Step 3: Add `hotpath = ["dep:hotpath"]` to `[features]`
- [x] Step 4: Run `cargo check -p hpx-dl --features hotpath` — compiles
- [x] Step 5: Run `cargo check -p hpx-dl` — still compiles without feature
- [x] BDD Verification: N/A — Cargo.toml change only
- [x] Advanced Test Verification: `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx-dl --all-features -- -D warnings`

### Task 1.4: Add hotpath::measure to hpx-dl hot-path functions

> **Context:** With the feature flag in place, annotate download engine hot paths.
> **Verification:** `cargo bench` or `cargo test` with hotpath shows function-level timing.
> **Scenario Coverage:** `features/performance.feature` — "hpx-dl segment download is instrumented", "hpx-dl speed limiter is instrumented", "hpx-dl segment state building is instrumented", "hpx-dl checksum verification is instrumented"

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — zero-cost when feature disabled
- **Simplification Focus:** Use `#[cfg_attr(feature = "hotpath", hotpath::measure)]` — same pattern as yawc
- **Status:** 🟢 DONE
- [x] Step 1: Add `#[cfg_attr(feature = "hotpath", hotpath::measure)]` to `download_segment_with_options` in `crates/hpx-dl/src/segment.rs:222`
- [x] Step 2: Add `#[cfg_attr(feature = "hotpath", hotpath::measure)]` to `SpeedLimiter::refill_tokens` in `crates/hpx-dl/src/speed.rs:92`
- [x] Step 3: Add `#[cfg_attr(feature = "hotpath", hotpath::measure)]` to `SpeedLimiter::try_consume` in `crates/hpx-dl/src/speed.rs:139`
- [x] Step 4: Add `#[cfg_attr(feature = "hotpath", hotpath::measure)]` to `SpeedLimiter::wait_for` in `crates/hpx-dl/src/speed.rs:164`
- [x] Step 5: Add `#[cfg_attr(feature = "hotpath", hotpath::measure)]` to `build_segment_states` in `crates/hpx-dl/src/engine.rs:1068`
- [x] Step 6: Add `#[cfg_attr(feature = "hotpath", hotpath::measure)]` to `compute_checksum` in `crates/hpx-dl/src/checksum.rs:31`
- [x] Step 7: Add `#[cfg_attr(feature = "hotpath", hotpath::measure)]` to `hash_file` in `crates/hpx-dl/src/checksum.rs:12`
- [x] Step 8: Run `cargo check -p hpx-dl --features hotpath` — compiles
- [x] BDD Verification: N/A — instrumentation only
- [x] Advanced Test Verification: `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx-dl --all-features -- -D warnings`

---

## Phase 2: ProxyPool Arc<Matcher> (S effort, MEDIUM impact)

### Task 2.1: Wrap Matcher in Arc in ProxyPool

> **Context:** `ProxyPool::select` clones a `Matcher` (URL parsing state) on every request. `Arc::clone` is O(1).
> **Verification:** All proxy pool tests pass. `select()` returns `Arc<Matcher>`.
> **Scenario Coverage:** `features/performance.feature` — "ProxyPool select avoids deep clone of Matcher"

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — proxy selection logic unchanged
- **Simplification Focus:** Wrap in `Arc` — one-line change to `Inner.matchers` type
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx/src/proxy_pool.rs` to confirm current `Matcher` clone pattern
- [x] Step 2: Change `Inner.matchers` from `Vec<Matcher>` to `Vec<Arc<Matcher>>`
- [x] Step 3: Change `Selection.matcher` from `Matcher` to `Arc<Matcher>`
- [x] Step 4: Change `select()` to use `Arc::clone(&self.inner.matchers[index])`
- [x] Step 5: Update `ProxyPool::with_strategy` to wrap matchers in `Arc::new()`
- [x] Step 6: Update `ProxyPoolService::call` if it uses `selected.matcher` directly
- [x] Step 7: Run `cargo nextest run -p hpx --all-features` — all pass
- [x] BDD Verification: N/A — internal refactor
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx --all-features -- -D warnings`

---

## Phase 3: Segment state iteration consolidation (S effort, LOW impact)

### Task 3.1: Single-pass iteration in resume_state_from_segments

> **Context:** `segment.rs:658-671` iterates segments twice — once for `completed_segments` and once for `bytes_completed`.
> **Verification:** All tests pass. Function produces identical results.
> **Scenario Coverage:** `features/performance.feature` — "resume_state_from_segments iterates segments once"

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — same output for same input
- **Simplification Focus:** Replace two iterations with one `for` loop
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx-dl/src/segment.rs:650-677` to confirm the double iteration
- [x] Step 2: Replace with a single `for` loop that collects both `completed_segments` and `bytes_completed`
- [x] Step 3: Run `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] BDD Verification: N/A — internal refactor
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx-dl --all-features -- -D warnings`

### Task 3.2: Use ahash HashSet in build_segment_states

> **Context:** `engine.rs:1079` uses `std::collections::HashSet` instead of the project's `ahash::HashSet`.
> **Verification:** All tests pass. Function uses project hasher.
> **Scenario Coverage:** `features/performance.feature` — "Segment state building uses ahash HashSet"

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** Use project's `HashSet` type alias from `hash.rs`
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx-dl/src/engine.rs:1068-1103` to confirm `std::collections::HashSet` usage
- [x] Step 2: Check if `hpx-dl` has access to `hpx::hash::HashSet` or needs its own `ahash` import
- [x] Step 3: Change `std::collections::HashSet<u32>` to use `ahash::HashSet` with the project's hasher
- [x] Step 4: Run `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] BDD Verification: N/A — internal refactor
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx-dl --all-features -- -D warnings`

### Task 3.3: Increase checksum buffer to 64 KiB

> **Context:** `checksum.rs:36` uses an 8 KiB buffer. For large files, 64 KiB reduces syscall count 8x.
> **Verification:** All checksum tests pass. Buffer is 64 KiB.
> **Scenario Coverage:** `features/performance.feature` — "Checksum buffer is sized for large file throughput"

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — checksums are identical
- **Simplification Focus:** One constant change
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx-dl/src/checksum.rs:36` to confirm 8 KiB buffer
- [x] Step 2: Change `let mut buf = [0u8; 8192]` to `let mut buf = [0u8; 65536]`
- [x] Step 3: Run `cargo nextest run -p hpx-dl --all-features` — all checksum tests pass
- [x] BDD Verification: N/A — constant change
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx-dl --all-features -- -D warnings`

---

## Phase 4: Verification & benchmarking

### Task 4.1: Run full workspace verification

> **Context:** All changes must pass the full CI pipeline.
> **Verification:** `just test-all` passes.
> **Scenario Coverage:** All scenarios — verification step

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — no regressions
- **Simplification Focus:** N/A — verification task
- **Status:** 🟢 DONE
- [x] Step 1: Run `just format` — all files formatted
- [x] Step 2: Run `just lint` — no warnings
- [x] Step 3: Run `just test` — all tests pass
- [x] Step 4: Run `just test-all` — all tests pass including BDD
- [x] BDD Verification: `just bdd` — all pass
- [x] Advanced Test Verification: `cargo bench --workspace --all-features` — benchmarks run
- [x] Runtime Verification: `cargo bench --package hpx-yawc --features hotpath --bench websocket_core -- --warm-up-time 1 --measurement-time 2` — hotpath output shows function timings

### Task 4.2: Validate hotpath output across crates

> **Context:** Verify that hotpath instrumentation produces meaningful profiling output.
> **Verification:** hotpath table shows function-level timings for hpx, hpx-dl, and yawc.
> **Scenario Coverage:** All hotpath scenarios — verification step

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** N/A — profiling validation
- **Simplification Focus:** N/A — validation task
- **Status:** 🟢 DONE
- [x] Step 1: Run `cargo bench --package hpx-yawc --features hotpath --bench websocket_core -- --warm-up-time 1 --measurement-time 2` — verify hotpath output shows `apply_mask`, `apply_mask_fast32`, `apply_mask_fast64`, `set_random_mask_if_not_set`, `write_head`, `Codec::decode`, `Decoder::decode`, `Encoder::encode`
- [x] Step 2: Run `cargo bench --package hpx --features hotpath,json --bench json_parsing -- --warm-up-time 1 --measurement-time 2` — verify hotpath output appears
- [x] Step 3: Run `cargo bench --package hpx --features hotpath --bench http_throughput -- --warm-up-time 1 --measurement-time 2` — verify hotpath output shows hpx functions
- [x] Step 4: Document the hotpath output in a comment or PR description
- [x] BDD Verification: N/A — profiling validation
- [x] Advanced Test Verification: N/A — profiling validation
- [x] Runtime Verification: All benchmarks exit 0
