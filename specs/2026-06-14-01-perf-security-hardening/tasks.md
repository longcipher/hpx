# Tasks: Performance & Security Hardening

Planned at commit `fd64fd5`. Execute via `/pb-build 2026-06-14-01-perf-security-hardening`.

---

## Phase 1: Low-risk performance fixes (S effort)

### Task 1.1: Remove redundant file seek in segment download

> **Context:** Every chunk in `download_segment_with_options` calls `file.seek()` before `write_all()`, even though the file position advances correctly after `write_all`. For a 100MB download with 64KB chunks, this is ~1,500 unnecessary syscalls.
> **Verification:** Run existing segment download tests. The downloaded file bytes must be identical.
> **Scenario Coverage:** `features/performance.feature` — "Segment download avoids redundant seeks"

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Preserve existing behavior` — downloaded bytes must be identical
- **Simplification Focus:** Remove the per-chunk `file.seek()` call; seek once before the loop
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx-dl/src/segment.rs:240-265` to confirm the current seek pattern
- [x] Step 2: Move the `file.seek(std::io::SeekFrom::Start(offset)).await?;` to before the `while let` loop (around line 242)
- [x] Step 3: Remove the `file.seek(...)` call inside the loop body (line 250)
- [x] Step 4: Verify `offset` tracking still works for progress reporting (it should — `offset` is only used for the seek and progress, not for write positioning)
- [x] Step 5: Run `cargo nextest run -p hpx-dl --all-features` — all existing tests pass
- [x] Step 6: Run `cargo +nightly clippy -p hpx-dl --all-features -- -D warnings` — no warnings
- [x] BDD Verification: `cargo test -p hpx-dl --test cucumber --all-features` — all pass
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly fmt --all` — no changes

### Task 1.2: Add unit test for seek-before-loop correctness

> **Context:** Verify that the seek-once pattern produces correct byte placement.
> **Verification:** A test that writes chunks via the refactored function and verifies file contents match expected bytes.
> **Scenario Coverage:** `features/performance.feature` — "Segment download avoids redundant seeks" (verification step)

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** N/A — test-only task
- **Status:** 🟢 DONE
- [x] Step 1: Add a test in `crates/hpx-dl/src/segment.rs` under `#[cfg(test)]` that calls the refactored seek logic with a mock file and verifies correct byte placement
- [x] Step 2: Run the test: `cargo test -p hpx-dl --lib -- segment::tests --all-features`
- [x] BDD Verification: N/A — unit test only
- [x] Advanced Test Verification: `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx-dl --all-features -- -D warnings`

### Task 2.1: Cache TLS connector in yawc with OnceLock

> **Context:** `tls_connector()` at `yawc/src/native/mod.rs:1330-1367` builds a new root cert store, crypto provider, and ClientConfig on every call. This is invoked per WSS connection.
> **Verification:** WSS connections work identically; the connector is constructed only once.
> **Scenario Coverage:** `features/performance.feature` — "TLS connector is cached across WSS connections"

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Preserve existing behavior` — WSS connections must work identically
- **Simplification Focus:** Replace per-call construction with `OnceLock`-cached static
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/yawc/src/native/mod.rs:1330-1367` to confirm the `tls_connector()` function
- [x] Step 2: Add `static TLS_CONNECTOR: OnceLock<TlsConnector> = OnceLock::new();` at module level
- [x] Step 3: Change `tls_connector()` to use `TLS_CONNECTOR.get_or_init(|| { ... })` with the existing construction logic
- [x] Step 4: Update the call site at `mod.rs:795` if needed (should work with `|| tls_connector()` since the function now returns `&TlsConnector`)
- [x] Step 5: Run `cargo nextest run -p hpx-yawc --all-features` — all pass
- [x] Step 6: Run `cargo +nightly clippy -p hpx-yawc --all-features -- -D warnings`
- [x] BDD Verification: N/A — no BDD for yawc
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly fmt --all`

### Task 2.2: Add unit test for TLS connector caching

> **Context:** Verify that the cached connector is returned on subsequent calls.
> **Verification:** Calling `tls_connector()` twice returns the same `Arc` pointer.
> **Scenario Coverage:** `features/performance.feature` — "TLS connector is cached across WSS connections" (verification step)

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** N/A — test-only task
- **Status:** 🟢 DONE
- [x] Step 1: Add a test in `crates/yawc/src/native/mod.rs` under `#[cfg(test)]` that calls `tls_connector()` twice and asserts `Arc::ptr_eq` on the inner configs
- [x] Step 2: Run the test: `cargo test -p hpx-yawc --lib -- native::tests --all-features`
- [x] BDD Verification: N/A
- [x] Advanced Test Verification: `cargo nextest run -p hpx-yawc --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx-yawc --all-features -- -D warnings`

### Task 3.1: Replace Vec::contains with HashSet in filter_remaining_segments

> **Context:** `filter_remaining_segments` at `segment.rs:682` uses `completed_indices.contains()` which is O(m) per segment.
> **Verification:** Function returns identical results; existing tests pass.
> **Scenario Coverage:** `features/performance.feature` — "Segment filtering uses constant-time lookup"

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Preserve existing behavior` — filtering results must be identical
- **Simplification Focus:** Collect `completed_indices` into `HashSet<u32>` before filtering
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx-dl/src/segment.rs:675-685` to confirm the current implementation
- [x] Step 2: Add `use std::collections::HashSet;` (or use the workspace `crate::hash::HashSet`)
- [x] Step 3: At the top of `filter_remaining_segments`, collect: `let completed: HashSet<u32> = completed_indices.iter().copied().collect();`
- [x] Step 4: Change the filter to `!completed.contains(&(*i as u32))`
- [x] Step 5: Run `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] Step 6: Run `cargo +nightly clippy -p hpx-dl --all-features -- -D warnings`
- [x] BDD Verification: `cargo test -p hpx-dl --test cucumber --all-features` — all pass
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly fmt --all`

### Task 3.2: Replace Vec::contains with HashSet in build_segment_states

> **Context:** `build_segment_states` at `engine.rs:1091` uses `completed_indices.contains()` which is O(m) per segment.
> **Verification:** Function returns identical results; existing tests pass.
> **Scenario Coverage:** `features/performance.feature` — "build_segment_states uses constant-time lookup"

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** Collect `completed_indices` into `HashSet<u32>` before the mapping
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx-dl/src/engine.rs:1076-1109` to confirm the current implementation
- [x] Step 2: Add `let completed: std::collections::HashSet<u32> = completed_indices.iter().copied().collect();` before the `.map()`
- [x] Step 3: Change `completed_indices.contains(&(index as u32))` to `completed.contains(&(index as u32))`
- [x] Step 4: Run `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] Step 5: Run `cargo +nightly clippy -p hpx-dl --all-features -- -D warnings`
- [x] BDD Verification: `cargo test -p hpx-dl --test cucumber --all-features` — all pass
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly fmt --all`

### Task 4.1: Hoist CSV ReaderBuilder outside map closure

> **Context:** `csv_stream.rs:83-86` constructs a new `csv::ReaderBuilder` inside `.map()` for every CSV row.
> **Verification:** CSV streaming produces identical results; existing tests pass.
> **Scenario Coverage:** `features/performance.feature` — "CSV stream reuses ReaderBuilder across rows"

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Preserve existing behavior` — deserialized CSV values must be identical
- **Simplification Focus:** Create the builder once before `.map()` and clone it inside
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx-streams/src/csv_stream.rs:70-99` to confirm the current implementation
- [x] Step 2: Before the `.map()` closure, create: `let builder = csv::ReaderBuilder::new().delimiter(delimiter).has_headers(false);`
- [x] Step 3: Inside `.map()`, use `builder.clone()` to create per-row readers (or find a way to avoid the clone — `csv::ReaderBuilder` should be `Clone`)
- [x] Step 4: Run `cargo nextest run -p hpx-streams --all-features` — all pass
- [x] Step 5: Run `cargo +nightly clippy -p hpx-streams --all-features` — no warnings
- [x] BDD Verification: N/A — no BDD for hpx-streams
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly fmt --all`

### Task 4.2: Add test for CSV stream builder reuse

> **Context:** Verify that the refactored CSV stream produces identical deserialization results.
> **Verification:** A test that streams CSV data through the refactored path and compares output.
> **Scenario Coverage:** `features/performance.feature` — "CSV stream reuses ReaderBuilder across rows" (verification step)

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** N/A — test-only task
- **Status:** 🟢 DONE
- [x] Step 1: Add a test in `crates/hpx-streams/src/csv_stream.rs` under `#[cfg(test)]` that verifies CSV deserialization with the refactored builder
- [x] Step 2: Run the test: `cargo test -p hpx-streams --lib -- csv_stream::tests --all-features`
- [x] BDD Verification: N/A
- [x] Advanced Test Verification: `cargo nextest run -p hpx-streams --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx-streams --all-features -- -D warnings`

### Task 6.1: Replace std::sync::mpsc with crossbeam-channel

> **Context:** `persistence.rs:1-7` uses `std::sync::mpsc` which is forbidden by AGENTS.md and has worse performance under contention.
> **Verification:** Persistence operations work identically; all tests pass.
> **Scenario Coverage:** `features/performance.feature` — "Persistence channel uses crossbeam-channel"

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Preserve existing behavior` — persistence commands must be processed identically
- **Simplification Focus:** Drop-in replacement of `std::sync::mpsc` with `crossbeam_channel`
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx-dl/src/persistence.rs:1-148` to confirm all `mpsc` usage
- [x] Step 2: Add `crossbeam-channel` to `crates/hpx-dl/Cargo.toml` as a dependency (use workspace version)
- [x] Step 3: Replace `use std::sync::mpsc::{self, Sender};` with `use crossbeam_channel::{Sender, Receiver, bounded};`
- [x] Step 4: Replace `mpsc::channel()` with `bounded(64)` for backpressure
- [x] Step 5: Update `PersistenceCommand` enum — `Sender<Result<...>>` stays the same (crossbeam has compatible `Sender`)
- [x] Step 6: Run `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] Step 7: Run `cargo +nightly clippy -p hpx-dl --all-features -- -D warnings`
- [x] BDD Verification: `cargo test -p hpx-dl --test cucumber --all-features` — all pass
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly fmt --all`

### Task 6.2: Add persistence channel stress test

> **Context:** Verify that the crossbeam-channel replacement handles concurrent sends correctly.
> **Verification:** A test that sends many commands concurrently and verifies all are processed.
> **Scenario Coverage:** `features/performance.feature` — "Persistence channel uses crossbeam-channel" (verification step)

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** N/A — test-only task
- **Status:** 🟢 DONE
- [x] Step 1: Add a test in `crates/hpx-dl/src/persistence.rs` under `#[cfg(test)]` that spawns multiple threads sending Upsert commands and verifies all are processed
- [x] Step 2: Run the test: `cargo test -p hpx-dl --lib -- persistence::tests --all-features`
- [x] BDD Verification: N/A
- [x] Advanced Test Verification: `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx-dl --all-features -- -D warnings`

### Task 7.1: Replace linear scan in mark_segment_completed

> **Context:** `mark_segment_completed` at `engine.rs:418-432` uses `iter_mut().find()` which is O(n) per call.
> **Verification:** Segment completion works identically; existing tests pass.
> **Scenario Coverage:** `features/performance.feature` — "Segment completion uses indexed lookup"

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** Change `segments` to a `Vec` indexed by segment index, or add a lookup map
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx-dl/src/engine.rs:418-436` and `crates/hpx-dl/src/engine.rs:1076-1109` to understand how segments are structured
- [x] Step 2: Determine if segments are always dense (index 0..n-1). If yes, change `segments: Vec<SegmentState>` to index by position.
- [x] Step 3: In `mark_segment_completed`, use `entry.segments[index as usize]` instead of `.iter_mut().find()`
- [x] Step 4: Verify `build_segment_states` produces segments with dense indices (it does — see line 1092)
- [x] Step 5: Run `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] Step 6: Run `cargo +nightly clippy -p hpx-dl --all-features -- -D warnings`
- [x] BDD Verification: `cargo test -p hpx-dl --test cucumber --all-features` — all pass
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly fmt --all`

### Task 7.2: Add test for indexed segment lookup

> **Context:** Verify that segment completion correctly updates the right segment.
> **Verification:** A test that completes segment index 3 and verifies only that segment is updated.
> **Scenario Coverage:** `features/performance.feature` — "Segment completion uses indexed lookup" (verification step)

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** N/A — test-only task
- **Status:** 🟢 DONE
- [x] Step 1: Add a test in `crates/hpx-dl/src/engine.rs` under `#[cfg(test)]` that creates a download entry with 5 segments, completes segment index 2, and verifies only segment 2 is marked completed
- [x] Step 2: Run the test: `cargo test -p hpx-dl --lib -- engine::tests --all-features`
- [x] BDD Verification: N/A
- [x] Advanced Test Verification: `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx-dl --all-features -- -D warnings`

### Task 8.1: Eliminate String allocation in range_header_value

> **Context:** `range_header_value` at `segment.rs:192-194` allocates a `String` via `format!` for every segment request.
> **Verification:** The header value is formatted correctly; existing tests pass.
> **Scenario Coverage:** `features/performance.feature` — "Range header value avoids heap allocation"

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Preserve existing behavior` — header value must be identical
- **Simplification Focus:** Use stack-allocated buffer or `ArrayString`
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx-dl/src/segment.rs:190-194` and check where `range_header_value` is called
- [x] Step 2: Check if `arrayvec` is already a dependency or use `arrayvec::ArrayString<32>` (max range header is ~40 chars: `bytes=18446744073709551615-18446744073709551615`)
- [x] Step 3: If `arrayvec` is not available, use a `[u8; 64]` buffer with `std::fmt::Write`
- [x] Step 4: Update callers to accept the new return type
- [x] Step 5: Run `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] Step 6: Run `cargo +nightly clippy -p hpx-dl --all-features -- -D warnings`
- [x] BDD Verification: `cargo test -p hpx-dl --test cucumber --all-features` — all pass
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly fmt --all`

## Phase 2: Medium-risk refactors (M effort)

### Task 5.1: Return modified entry directly from update_sync

> **Context:** `update_entry` at `engine.rs:346-356` clones the entire `DownloadEntry` via `entry_snapshot` after the atomic update, then `to_record` clones 7+ fields.
> **Verification:** State transitions work identically; persistence records are correct.
> **Scenario Coverage:** `features/performance.feature` — "State transitions avoid unnecessary full-entry clones"

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Preserve existing behavior` — download state must be updated and persisted correctly
- **Simplification Focus:** Return modified entry from `update_sync` closure, eliminating the snapshot clone
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx-dl/src/engine.rs:326-357` and `scc` docs for `update_sync` return type
- [x] Step 2: Check if `scc::HashMap::update_sync` can return a value from the closure. If not, use `read_sync` after update but avoid full clone.
- [x] Step 3: If `update_sync` supports returning a value, modify the closure to return a snapshot or the modified entry
- [x] Step 4: Remove the `entry_snapshot(id)?` call in `update_entry`
- [x] Step 5: Run `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] Step 6: Run `cargo +nightly clippy -p hpx-dl --all-features -- -D warnings`
- [x] BDD Verification: `cargo test -p hpx-dl --test cucumber --all-features` — all pass
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly fmt --all`

### Task 5.2: Optimize to_record to avoid redundant field clones

> **Context:** `to_record` at `engine.rs:278-295` clones `request`, `url`, `destination`, `etag`, `last_modified`, `last_error`, and `segments`.
> **Verification:** The record produced is identical; existing tests pass.
> **Scenario Coverage:** `features/performance.feature` — "State transitions avoid unnecessary full-entry clones" (part 2)

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** Use `Arc` for shared fields or swap fields instead of cloning
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx-dl/src/engine.rs:278-295` and the `DownloadRecord` type
- [x] Step 2: Determine which fields can be wrapped in `Arc` (request, url, destination are likely shared)
- [x] Step 3: If `DownloadRecord` is only used for persistence (written once then discarded), consider making `to_record` take `self` (consuming the entry) in paths where the entry is no longer needed
- [x] Step 4: Run `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] Step 5: Run `cargo +nightly clippy -p hpx-dl --all-features` — -D warnings
- [x] BDD Verification: `cargo test -p hpx-dl --test cucumber --all-features` — all pass
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly fmt --all`

### Task 5.3: Add test for clone-reduced state transitions

> **Context:** Verify that state transitions still produce correct persistence records after the clone reduction.
> **Verification:** A test that transitions a download through multiple states and verifies the persisted records.
> **Scenario Coverage:** `features/performance.feature` — "State transitions avoid unnecessary full-entry clones" (verification step)

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** N/A — test-only task
- **Status:** 🟢 DONE
- [x] Step 1: Add a test in `crates/hpx-dl/src/engine.rs` under `#[cfg(test)]` that creates a download, transitions through Pending→Downloading→Completed, and verifies each persisted record
- [x] Step 2: Run the test: `cargo test -p hpx-dl --lib -- engine::tests --all-features`
- [x] BDD Verification: N/A
- [x] Advanced Test Verification: `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx-dl --all-features -- -D warnings`

## Phase 3: Correctness fix (M effort)

### Task 9.1: Drain pending persistence commands on shutdown

> **Context:** `PersistenceHandle::Drop` at `persistence.rs:129-138` sends `Shutdown` without draining pending commands. This causes silent metadata loss.
> **Verification:** Pending commands are processed before the worker exits.
> **Scenario Coverage:** `features/correctness.feature` — "Persistence worker drains pending commands on shutdown"

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Change behavior` — shutdown now drains pending commands instead of dropping them
- **Simplification Focus:** Use bounded channel + drain in Drop
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx-dl/src/persistence.rs:129-138` to confirm current Drop impl
- [x] Step 2: After switching to crossbeam-channel (Task 6.1), implement bounded channel with capacity 64
- [x] Step 3: In `Drop`, send `Shutdown` then drop the `Sender` — the worker will process all remaining commands in the channel before seeing the disconnect
- [x] Step 4: Verify the worker loop at `persistence.rs:62-78` exits correctly when the channel is disconnected
- [x] Step 5: Run `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] Step 6: Run `cargo +nightly clippy -p hpx-dl --all-features` — -D warnings
- [x] BDD Verification: `cargo test -p hpx-dl --test cucumber --all-features` — all pass
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly fmt --all`

### Task 9.2: Add test for persistence shutdown drain

> **Context:** Verify that pending commands are processed before the worker exits.
> **Verification:** A test that sends commands, drops the handle, and verifies all commands were processed.
> **Scenario Coverage:** `features/correctness.feature` — "Persistence worker drains pending commands on shutdown" (verification step)

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Change behavior` — verify drain behavior
- **Simplification Focus:** N/A — test-only task
- **Status:** 🟢 DONE
- [x] Step 1: Add a test in `crates/hpx-dl/src/persistence.rs` under `#[cfg(test)]` that creates a PersistenceHandle, sends multiple Upsert commands, drops the handle, and verifies all records are persisted
- [x] Step 2: Run the test: `cargo test -p hpx-dl --lib -- persistence::tests --all-features`
- [x] BDD Verification: N/A
- [x] Advanced Test Verification: `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx-dl --all-features -- -D warnings`

## Phase 4: Security fixes (S effort)

### Task 10.1: Validate hostname in HTTP CONNECT tunnel

> **Context:** `target_host` at `proxy.rs:94-108` is interpolated into the CONNECT request without sanitization. A host containing `\r\n` can inject headers.
> **Verification:** Valid hostnames pass; hosts with control characters are rejected.
> **Scenario Coverage:** `features/security.feature` — "HTTP CONNECT tunnel rejects hosts with control characters"

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Change behavior` — invalid hosts now return an error instead of being accepted
- **Simplification Focus:** Add a simple regex or character-check validation function
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/yawc/src/native/proxy.rs:85-111` to confirm the current implementation
- [x] Step 2: Add a `fn validate_host(host: &str) -> io::Result<()>` that checks all characters are in `[a-zA-Z0-9.\-:\[\]]`
- [x] Step 3: Call `validate_host(target_host)?` at the beginning of `http_connect_tunnel`
- [x] Step 4: Return `io::Error::new(io::ErrorKind::InvalidInput, "invalid hostname")` for invalid hosts
- [x] Step 5: Run `cargo nextest run -p hpx-yawc --all-features` — all pass
- [x] Step 6: Run `cargo +nightly clippy -p hpx-yawc --all-features -- -D warnings`
- [x] BDD Verification: N/A — no BDD for yawc
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly fmt --all`

### Task 10.2: Add tests for hostname validation

> **Context:** Verify that valid hostnames pass and malicious ones are rejected.
> **Verification:** Unit tests for the validation function.
> **Scenario Coverage:** `features/security.feature` — "HTTP CONNECT tunnel rejects hosts with control characters" (verification step)

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Change behavior`
- **Simplification Focus:** N/A — test-only task
- **Status:** 🟢 DONE
- [x] Step 1: Add tests in `crates/yawc/src/native/proxy.rs` under `#[cfg(test)]` for `validate_host`: valid cases (example.com, 192.168.1.1, [::1]), invalid cases (host\r\n, host\n, host with spaces)
- [x] Step 2: Run the test: `cargo test -p hpx-yawc --lib -- native::proxy::tests --all-features`
- [x] BDD Verification: N/A
- [x] Advanced Test Verification: `cargo nextest run -p hpx-yawc --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx-yawc --all-features -- -D warnings`

### Task 11.1: Redact sensitive headers in LoggingHook

> **Context:** `LoggingHook::on_request` at `hooks.rs:217-218` logs all header values including Authorization, Cookie, and Proxy-Authorization.
> **Verification:** Sensitive header values are redacted; other headers are logged normally.
> **Scenario Coverage:** `features/security.feature` — "Sensitive headers are redacted in request logging"

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Change behavior` — sensitive headers now show "[REDACTED]" in logs
- **Simplification Focus:** Add a static set of sensitive header names and check before logging
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx/src/client/layer/hooks.rs:207-228` to confirm the current implementation
- [x] Step 2: Add a constant: `const SENSITIVE_HEADERS: &[HeaderName] = &[header::AUTHORIZATION, header::COOKIE, header::PROXY_AUTHORIZATION];`
- [x] Step 3: In the logging loop, check `if SENSITIVE_HEADERS.contains(name)` and log `"[REDACTED]"` instead of the value
- [x] Step 4: Apply the same pattern to `on_response` at `hooks.rs:238-239`
- [x] Step 5: Run `cargo nextest run -p hpx --all-features` — all pass
- [x] Step 6: Run `cargo +nightly clippy -p hpx --all-features -- -D warnings`
- [x] BDD Verification: N/A — no BDD for hpx core
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly fmt --all`

### Task 11.2: Add test for header redaction

> **Context:** Verify that sensitive headers are redacted in log output.
> **Verification:** A test that creates a LoggingHook, logs a request with Authorization header, and verifies the value is not in the output.
> **Scenario Coverage:** `features/security.feature` — "Sensitive headers are redacted in request logging" (verification step)

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Change behavior`
- **Simplification Focus:** N/A — test-only task
- **Status:** 🟢 DONE
- [x] Step 1: Add a test in `crates/hpx/src/client/layer/hooks.rs` under `#[cfg(test)]` that creates a LoggingHook, constructs a request with `Authorization: Bearer secret-token`, calls `on_request`, and verifies the output contains "[REDACTED]" and not the token
- [x] Step 2: Run the test: `cargo test -p hpx --lib -- client::layer::hooks::tests --all-features`
- [x] BDD Verification: N/A
- [x] Advanced Test Verification: `cargo nextest run -p hpx --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx --all-features -- -D warnings`

### Task 12.1: Mask proxy credentials in trace logs

> **Context:** The proxy URI including credentials is logged at TRACE level in the connector.
> **Verification:** Proxy credentials are not visible in trace output.
> **Scenario Coverage:** `features/security.feature` — "Proxy credentials are not logged in trace output"

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Change behavior` — proxy URIs now show masked credentials in logs
- **Simplification Focus:** Add a `redact_proxy_uri` helper function
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx/src/client/conn/connector.rs:376` to confirm the current logging
- [x] Step 2: Add a helper function `fn redact_proxy_uri(uri: &Uri) -> String` that replaces the password portion with `***`
- [x] Step 3: Use `url::Url` to parse the proxy URI, then reconstruct with masked password
- [x] Step 4: Replace the `trace!` call with the redacted URI
- [x] Step 5: Run `cargo nextest run -p hpx --all-features` — all pass
- [x] Step 6: Run `cargo +nightly clippy -p hpx --all-features -- -D warnings`
- [x] BDD Verification: N/A
- [x] Advanced Test Verification: `cargo nextest run --workspace --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly fmt --all`

### Task 12.2: Add test for proxy credential masking

> **Context:** Verify that proxy credentials are masked in trace output.
> **Verification:** A test that logs a proxy URI with credentials and verifies they are masked.
> **Scenario Coverage:** `features/security.feature` — "Proxy credentials are not logged in trace output" (verification step)

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Change behavior`
- **Simplification Focus:** N/A — test-only task
- **Status:** 🟢 DONE
- [x] Step 1: Add a test in `crates/hpx/src/client/conn/connector.rs` under `#[cfg(test)]` that calls `redact_proxy_uri` with `http://user:pass@proxy:8080` and verifies the output is `http://***@proxy:8080`
- [x] Step 2: Run the test: `cargo test -p hpx --lib -- client::conn::connector::tests --all-features`
- [x] BDD Verification: N/A
- [x] Advanced Test Verification: `cargo nextest run -p hpx --all-features` — all pass
- [x] Runtime Verification: `cargo +nightly clippy -p hpx --all-features -- -D warnings`

---

## Dependency Order

Tasks 1.1-1.2, 2.1-2.2, 3.1-3.2, 4.1-4.2, 7.1-7.2, 8.1, 10.1-10.2, 11.1-11.2, 12.1-12.2 are independent and can run in parallel.

Task 6.1 must complete before Task 9.1 (persistence drain depends on crossbeam-channel).

Task 5.1 should complete before Task 5.2 (clone reduction builds on the update_sync return value).

All tasks should complete before a final `just test-all` verification pass.
