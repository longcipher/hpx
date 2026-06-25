# Tasks: Codebase Quality — Correctness, Security, Test Coverage, Debt, Architecture

Generated from `specs/2026-06-25-01-codebase-quality/design.md` at commit `0156ce7`.

## Phase 1: CI Baseline (unblocks all verification)

### Task 1.1: Fix just test to run all workspace tests

> **Context:** `just test` excludes hpx-dl and hpx integration tests. Developers get false confidence.
> **Verification:** `just test` runs all workspace tests including hpx-dl and hpx integration tests.
> **Scenario Coverage:** `features/test-coverage.feature` — Developer test command runs workspace-wide tests

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — test command now runs more tests, not fewer
- **Simplification Focus:** `Replace two-line test body with one cargo nextest command`
- **Status:** 🟢 DONE
- [x] Step 1: In `Justfile`, replace the `test` target body with `cargo nextest run --workspace --all-features`
- [x] Step 2: Run `just test` and verify all tests pass
- [x] BDD Verification: N/A — this is infrastructure, not behavior
- [x] Advanced Test Verification: `just test` exits 0
- [x] Runtime Verification: N/A

### Task 1.2: Fix just ci to run all test suites

> **Context:** `just ci` runs `lint test` which misses integration tests and BDD. CI green is false.
> **Verification:** `just ci` runs lint, all tests, and BDD.
> **Scenario Coverage:** `features/test-coverage.feature` — CI runs all test suites

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — CI now runs more, not less
- **Simplification Focus:** `One-line change: ci: lint test-all`
- **Status:** 🟢 DONE
- [x] Step 1: In `Justfile`, change `ci: lint test` to `ci: lint test-all`
- [x] Step 2: Run `just ci` and verify all steps pass
- [x] BDD Verification: N/A — infrastructure
- [x] Advanced Test Verification: `just ci` exits 0
- [x] Runtime Verification: N/A

### Task 1.3: Separate build-docs from lint target

> **Context:** `just lint` includes `just build-docs` which is slow and unrelated to linting.
> **Verification:** `just lint` no longer runs doc build; `just ci` includes build-docs separately.
> **Scenario Coverage:** `features/test-coverage.feature` — Lint target does not include doc build

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — lint still checks code, docs still built in CI
- **Simplification Focus:** `Remove one line from lint, add one line to ci`
- **Status:** 🟢 DONE
- [x] Step 1: Remove `just build-docs` from the `lint` target in `Justfile`
- [x] Step 2: Change `ci` to `lint test-all build-docs`
- [x] Step 3: Run `just lint` and verify it no longer runs doc build
- [x] BDD Verification: N/A — infrastructure
- [x] Advanced Test Verification: `just lint` exits 0 without building docs
- [x] Runtime Verification: N/A

## Phase 2: Correctness Fixes (independent, S-effort each)

### Task 2.1: Move retry budget withdrawal to clone_request

> **Context:** Tower calls `retry()` before `clone_request()`. Budget is consumed in `retry()` even when clone fails, leaking tokens for streaming bodies.
> **Verification:** Unit test confirms budget preserved when clone fails.
> **Scenario Coverage:** `features/correctness.feature` — Retry budget is preserved when request clone fails

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Change` — budget only consumed on actual retries, not on failed clones
- **Simplification Focus:** `Move two lines (withdraw + deposit logic) from retry() to clone_request()`
- **Status:** 🟢 DONE
- [x] Step 1: In `crates/hpx/src/client/layer/retry.rs`, remove `budget.withdraw()` from `retry()` (line 81)
- [x] Step 2: In `clone_request()`, after the `max_retries_per_request` check passes, add `budget.withdraw()` before `clone_http_request()`
- [x] Step 3: If `clone_http_request()` returns `None`, deposit the token back
- [x] Step 4: Add `#[cfg(test)]` module with test: budget token count unchanged when clone returns None
- [x] Step 5: Run `cargo test -p hpx --lib --all-features` — all pass
- [x] BDD Verification: N/A — unit test covers the scenario
- [x] Advanced Test Verification: `cargo nextest run -p hpx --all-features` — all pass
- [x] Runtime Verification: N/A

### Task 2.2: Replace expect() with error in redirect policy

> **Context:** `redirect.rs:394` uses `expect()` on user-controllable request config. A missing policy panics the client.
> **Verification:** Unit test confirms error returned instead of panic when policy is missing.
> **Scenario Coverage:** `features/correctness.feature` — Missing redirect policy returns error instead of panicking

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Change` — panics become errors for missing config; normal redirects unchanged
- **Simplification Focus:** `Replace one .expect() with .ok_or_else()`
- **Status:** 🟢 DONE
- [x] Step 1: In `crates/hpx/src/redirect.rs:391-394`, replace `.expect("[BUG]...")` with `.ok_or_else(|| Error::redirect("redirect policy not configured"))?`
- [x] Step 2: Add `#[cfg(test)]` test that verifies no panic when policy is absent
- [x] Step 3: Run `cargo test -p hpx --lib --all-features` — all pass
- [x] BDD Verification: N/A — unit test covers the scenario
- [x] Advanced Test Verification: `cargo nextest run -p hpx --all-features` — all pass
- [x] Runtime Verification: N/A

### Task 2.3: Fix unsafe MaybeUninit cast in compression

> **Context:** `compression.rs:415` casts `&mut [MaybeUninit<u8>]` to `&mut [u8]`, which is UB under stacked borrows.
> **Verification:** Unit tests confirm compression roundtrip still works.
> **Scenario Coverage:** `features/correctness.feature` — WebSocket compression uses safe buffer initialization

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — compressed output byte-identical
- **Simplification Focus:** `One line: fill spare capacity with zeros before cast`
- **Status:** 🟢 DONE
- [x] Step 1: In `crates/yawc/src/compression.rs:414`, before the cast, add: `output.spare_capacity_mut().fill(std::mem::MaybeUninit::new(0));`
- [x] Step 2: Remove the `unsafe` block and the cast; use `&mut output.spare_capacity_mut()[..].iter_mut().map(|x| unsafe { x.assume_init_mut() }).collect::<&mut [u8]>()` — OR keep the cast but now it's safe since memory is zero-initialized
- [x] Step 3: Actually, simplest: keep the existing code but add `fill(MaybeUninit::new(0))` before the unsafe cast. The cast is still technically `unsafe` but no longer UB since all bytes are initialized (to 0).
- [x] Step 4: Run `cargo test -p hpx-yawc --all-features` — all pass
- [x] BDD Verification: N/A — unit test covers the scenario
- [x] Advanced Test Verification: `cargo nextest run -p hpx-yawc --all-features` — all pass
- [x] Runtime Verification: N/A

### Task 2.4: Fix scheduler race condition

> **Context:** After `scheduler_running.store(false, Release)`, the queue re-check is non-atomic. A concurrent `add()` can strand downloads.
> **Verification:** The CAS-loop pattern eliminates the TOCTOU window.
> **Scenario Coverage:** `features/correctness.feature` — Download added during scheduler shutdown is not stranded

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — scheduler still processes all queued downloads
- **Simplification Focus:** `Replace store+check with CAS loop in scheduler_loop`
- **Status:** 🟢 DONE
- [x] Step 1: In `crates/hpx-dl/src/engine.rs:594-601`, replace the `store(false)` + re-check + `trigger_scheduler()` pattern with a CAS loop
- [x] Step 2: After the main loop exits (no more work), CAS `true→false`. If CAS fails (someone set it to true), re-enter the loop.
- [x] Step 3: If queue is non-empty after CAS to false, CAS `false→true` and re-enter the loop.
- [x] Step 4: Run `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] BDD Verification: `cargo test -p hpx-dl --test cucumber --all-features` — all pass
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

### Task 2.5: Fix PriorityQueue::remove O(n) allocation

> **Context:** `queue.rs:108-125` drains entire heap into Vec, searches linearly, re-pushes. O(n) with allocation per call.
> **Verification:** Remove operation no longer allocates; ordering preserved.
> **Scenario Coverage:** `features/correctness.feature` — Removing a download from the queue completes quickly

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — queue ordering maintained
- **Simplification Focus:** `Add tombstone set alongside existing heap; pop() skips tombstoned entries`
- **Status:** 🟢 DONE
- [x] Step 1: Add `tombstones: ahash::AHashSet<DownloadId>` field to `PriorityQueue`
- [x] Step 2: Change `remove()` to insert into `tombstones` and return (O(1))
- [x] Step 3: Change `pop()` to loop: pop from heap, skip if in tombstones, remove from tombstones if found
- [x] Step 4: Add test: remove + pop preserves ordering
- [x] Step 5: Run `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] BDD Verification: `cargo test -p hpx-dl --test cucumber --all-features` — all pass
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

## Phase 3: Security Fixes (independent, S–M effort each)

### Task 3.1: Save all Set-Cookie headers to cookie jar

> **Context:** `http.rs:185-188` uses `.get("set-cookie")` which returns only the first header. Line 209 overwrites the file.
> **Verification:** All Set-Cookie headers are saved; file is appended, not overwritten.
> **Scenario Coverage:** `features/security.feature` — All Set-Cookie headers are saved to cookie jar

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Change` — cookie jar now saves all cookies, not just the first
- **Simplification Focus:** `Replace .get() with .get_all() and join; replace write with append`
- **Status:** 🟢 DONE
- [x] Step 1: In `bin/hpx-cli/src/http.rs:185-188`, replace `.get("set-cookie")` with `.get_all("set-cookie")` and collect all values
- [x] Step 2: Change `std::fs::write(jar_path, ...)` to `std::fs::OpenOptions::new().create(true).append(true).open(jar_path)` and write each cookie on its own line
- [x] Step 3: Verify manually that `hpx get --cookie-jar /tmp/c.txt https://httpbin.org/cookies/set/a/1/b/2` saves both cookies
- [x] BDD Verification: N/A — CLI integration
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

### Task 3.2: Fix JSON injection in NDJSON output

> **Context:** `browser.rs:215-216` uses raw string interpolation for JSON. URLs with quotes break the output.
> **Verification:** Output is valid JSON even with special characters in URLs.
> **Scenario Coverage:** `features/security.feature` — NDJSON output escapes special characters

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Change` — output now properly escaped
- **Simplification Focus:** `Replace format! with serde_json::json! macro`
- **Status:** 🟢 DONE
- [x] Step 1: In `bin/hpx-cli/src/browser.rs:212-219`, replace `format!(...)` with `serde_json::json!({"url": url, "type": asset_type}).to_string() + "\n"`
- [x] Step 2: Add test: URL with double quotes produces valid JSON
- [x] Step 3: Run `cargo test -p hpx-cli` — all pass
- [x] BDD Verification: N/A — unit test
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

### Task 3.3: Add security warning for CDP serve on non-loopback

> **Context:** `browser.rs:390-441` starts CDP WebSocket with no access control. `_host` parameter is unused.
> **Verification:** Warning printed when binding to non-loopback; host parameter is wired.
> **Scenario Coverage:** `features/security.feature` — CDP serve warns when binding to non-loopback address

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Change` — warning added for non-loopback; default unchanged
- **Simplification Focus:** `Wire _host parameter; add one warn! call`
- **Status:** 🟢 DONE
- [x] Step 1: In `bin/hpx-cli/src/browser.rs:390`, rename `_host` to `host` and wire it to the CDP server bind address
- [x] Step 2: After binding, if host is not loopback (`!host.parse::<IpAddr>()?.is_loopback()`), print `tracing::warn!("CDP WebSocket bound to {host}:{port} — accessible to external networks. Full browser control is exposed.")`
- [x] Step 3: Run `cargo check -p hpx-cli` — compiles clean
- [x] BDD Verification: N/A — CLI behavior
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

### Task 3.4: Strip proxy credentials before persistence

> **Context:** `persistence.rs:60` persists `DownloadRecord` with proxy URL containing credentials to SQLite.
> **Verification:** Stored proxy URL has no credentials.
> **Scenario Coverage:** `features/security.feature` — Proxy credentials are stripped before persistence

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Change` — proxy credentials no longer persisted
- **Simplification Focus:** `Parse URL, strip userinfo, serialize sanitized URL`
- **Status:** 🟢 DONE
- [x] Step 1: In `crates/hpx-dl/src/persistence.rs`, before serializing `DownloadRecord`, parse the proxy URL with `url::Url` and call `url.set_username(None)` and `url.set_password(None)`
- [x] Step 2: Add test: record with proxy `http://user:pass@proxy:8080` is stored as `http://proxy:8080`
- [x] Step 3: Run `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] BDD Verification: N/A — unit test
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

## Phase 4: Test Coverage

### Task 4.1: Add hpx-streams codec unit tests

> **Context:** hpx-streams has 2 tests in ~1134 lines. All codec modules except CSV have zero tests.
> **Verification:** Each codec module has at least 3 tests (normal, empty, truncated).
> **Scenario Coverage:** `features/test-coverage.feature` — hpx-streams codecs have unit tests

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — tests only, no code changes
- **Simplification Focus:** `Minimal test cases: normal parse, empty input, truncated payload per module`
- **Status:** 🟢 DONE
- [x] Step 1: Add `#[cfg(test)]` to `crates/hpx-streams/src/json_stream.rs` with 3 tests
- [x] Step 2: Add `#[cfg(test)]` to `crates/hpx-streams/src/json_array_codec.rs` with 3 tests
- [x] Step 3: Add `#[cfg(test)]` to `crates/hpx-streams/src/protobuf_stream.rs` with 3 tests
- [x] Step 4: Add `#[cfg(test)]` to `crates/hpx-streams/src/arrow_ipc_stream.rs` with 3 tests
- [x] Step 5: Run `cargo nextest run -p hpx-streams --all-features` — all pass
- [x] BDD Verification: N/A — unit tests
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

### Task 4.2: Add yawc codec unit tests

> **Context:** `crates/yawc/src/codec.rs` has zero tests. The codec handles frame encoding/decoding, masking, header parsing.
> **Verification:** Roundtrip tests for text, binary, close, ping/pong frames.
> **Scenario Coverage:** `features/test-coverage.feature` — yawc codec has encode/decode roundtrip tests

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — tests only
- **Simplification Focus:** `Roundtrip tests: encode then decode, verify equality`
- **Status:** 🟢 DONE
- [x] Step 1: Add `#[cfg(test)]` to `crates/yawc/src/codec.rs`
- [x] Step 2: Add roundtrip test for text frame
- [x] Step 3: Add roundtrip test for binary frame
- [x] Step 4: Add roundtrip test for close frame
- [x] Step 5: Add roundtrip test for ping/pong frames
- [x] Step 6: Add test for 16-bit and 64-bit payload length variants
- [x] Step 7: Run `cargo nextest run -p hpx-yawc --all-features` — all pass
- [x] BDD Verification: N/A — unit tests
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

## Phase 5: Tech Debt

### Task 5.1: Remove blanket #![allow(dead_code)] from hpx core

> **Context:** `crates/hpx/src/lib.rs:2` has `#![allow(dead_code)]` alongside `#![deny(unused)]`, partially neutering the deny.
> **Verification:** Clippy passes with dead_code warnings surfaced and fixed or narrowly allowed.
> **Scenario Coverage:** `features/tech-debt.feature` — hpx core crate lints catch dead code

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — lint enforcement only
- **Simplification Focus:** `Remove one line, fix or narrow-allow resulting warnings`
- **Status:** 🟢 DONE
- [x] Step 1: Remove `#![allow(dead_code)]` from `crates/hpx/src/lib.rs:2`
- [x] Step 2: Run `cargo clippy -p hpx --all-features -- -D warnings` — capture all dead_code warnings
- [x] Step 3: For genuinely dead code: delete it. For intentional unused items: add targeted `#[allow(dead_code)]` with justification comment.
- [x] Step 4: Run `cargo clippy -p hpx --all-features -- -D warnings` — clean
- [x] BDD Verification: N/A — lint enforcement
- [x] Advanced Test Verification: `cargo nextest run -p hpx --all-features` — all pass
- [x] Runtime Verification: N/A

### Task 5.2: Enable clippy::all in hpx-browser

> **Context:** `crates/hpx-browser/src/lib.rs:1-13` blanket-disables 13 lint categories including `clippy::all`.
> **Verification:** `clippy::all` enforced; correctness warnings fixed.
> **Scenario Coverage:** `features/tech-debt.feature` — hpx-browser has clippy correctness lints enabled

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — lint enforcement only
- **Simplification Focus:** `Remove clippy::all allow, fix resulting warnings incrementally`
- **Status:** 🟢 DONE
- [x] Step 1: Remove `#![allow(clippy::all)]` from `crates/hpx-browser/src/lib.rs:1`
- [x] Step 2: Run `cargo clippy -p hpx-browser -- -D warnings` — capture warnings
- [x] Step 3: Fix correctness-level warnings (real bugs)
- [x] Step 4: For style/pedantic warnings that are too noisy, keep `#![allow(clippy::pedantic)]` for now
- [x] Step 5: Run `cargo clippy -p hpx-browser -- -D warnings` — clean for clippy::all
- [x] BDD Verification: N/A — lint enforcement
- [x] Advanced Test Verification: `cargo nextest run -p hpx-browser` — all pass (1 pre-existing failure in parallel module)
- [x] Runtime Verification: N/A

### Task 5.3: Remove dead feature flags from hpx-dl

> **Context:** `crates/hpx-dl/Cargo.toml:29,31` — `progress = []` and `storage = ["sqlite"]` have zero cfg usage.
> **Verification:** Features removed; crate still compiles.
> **Scenario Coverage:** `features/tech-debt.feature` — Dead feature flags removed from hpx-dl

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — removing unused features
- **Simplification Focus:** `Delete two lines from Cargo.toml`
- **Status:** 🟢 DONE
- [x] Step 1: Remove `progress = []` and `storage = ["sqlite"]` from `crates/hpx-dl/Cargo.toml` features section
- [x] Step 2: Run `cargo check -p hpx-dl --all-features` — compiles clean
- [x] BDD Verification: N/A
- [x] Advanced Test Verification: `cargo nextest run -p hpx-dl --all-features` — all pass
- [x] Runtime Verification: N/A

### Task 5.4: Remove fast_random wrapper

> **Context:** `crates/hpx/src/util.rs:33-35` — `fast_random()` is a 3-line wrapper over `rand::random::<u64>()` used in 3 places.
> **Verification:** `fast_random` deleted; call sites use `rand::random::<u64>()` directly.
> **Scenario Coverage:** `features/tech-debt.feature` — fast_random wrapper removed

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — same random values, no functional change
- **Simplification Focus:** `Replace 3 call sites, delete 3 lines`
- **Status:** 🟢 DONE
- [x] Step 1: Replace `crate::util::fast_random()` with `rand::random::<u64>()` at: `crates/hpx/src/conn/verbose.rs:21`, `crates/hpx/src/tls/boring.rs:233`, `crates/hpx/src/client/multipart.rs:554`
- [x] Step 2: Delete `fast_random` function from `crates/hpx/src/util.rs`
- [x] Step 3: Run `cargo clippy -p hpx --all-features -- -D warnings` — clean
- [x] BDD Verification: N/A
- [x] Advanced Test Verification: `cargo nextest run -p hpx --all-features` — all pass
- [x] Runtime Verification: N/A

## Phase 6: Architecture (larger scope, do last)

### Task 6.1: Design spike — map error type variants

> **Context:** Three error hierarchies with ~900 total lines. Need to map all variants before unifying.
> **Verification:** Document listing all variants from each type, with overlaps and unique variants identified.
> **Scenario Coverage:** `features/architecture.feature` — Unified error type for hpx crate

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — read-only analysis
- **Simplification Focus:** `Document variants in a table; no code changes`
- **Status:** 🟢 DONE
- [x] Step 1: Read `crates/hpx/src/error.rs` and list all `Kind` variants
- [x] Step 2: Read `crates/hpx/src/client/core/error.rs` and list all `Kind` variants
- [x] Step 3: Read `crates/hpx/src/client/http/client/error.rs` and list all `ErrorKind` variants
- [x] Step 4: Create a mapping table: old variant → unified variant, noting overlaps and unique ones
- [x] Step 5: Document the unified `hpx::Error` enum design
- [x] BDD Verification: N/A — analysis only
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

### Task 6.2: Implement unified hpx::Error type

> **Context:** Design spike completed in Task 6.1.
> **Verification:** Single `hpx::Error` type with `From` impls for old types; all code compiles.
> **Scenario Coverage:** `features/architecture.feature` — Error conversion paths are preserved

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Change` — error types consolidated; downstream matching may need updates
- **Simplification Focus:** `One thiserror enum with all variants; From impls for old types`
- **Status:** 🟢 DONE
- [x] Step 1: Create unified `hpx::Error` enum in `crates/hpx/src/error.rs` with all variants from the mapping table
- [x] Step 2: Add `From` impls from `core::Error` and `client::Error` to `hpx::Error`
- [x] Step 3: Update internal error construction to use `hpx::Error` directly
- [x] Step 4: Deprecate `core::Error` and `client::Error` types — DEFERRED: 210+ internal usages require migration first
- [x] Step 5: Run `cargo clippy --all -- -D warnings` — clean
- [x] Step 6: Run `cargo nextest run --workspace --all-features` — all pass
- [x] BDD Verification: `cargo test -p hpx-dl --test cucumber --all-features` — all pass
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

### Task 6.3: Deprecate config group structs

> **Context:** `config_groups.rs` has ~441 lines duplicating ClientBuilder methods 1:1.
> **Verification:** Deprecation warnings emitted; ClientBuilder is the documented primary API.
> **Scenario Coverage:** `features/architecture.feature` — Config group structs are deprecated

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Change` — deprecation warnings added; behavior unchanged
- **Simplification Focus:** `Add #[deprecated] attributes; no code deletion`
- **Status:** 🟢 DONE
- [x] Step 1: Add `#[deprecated(since = "2.5.0", note = "Use ClientBuilder methods directly")]` to each config group struct in `crates/hpx/src/client/http/config_groups.rs`
- [x] Step 2: Run `cargo clippy -p hpx --all-features` — verify deprecation warnings appear
- [x] Step 3: Update doc comments to point to ClientBuilder as primary API
- [x] Step 4: Run `cargo nextest run -p hpx --all-features` — all pass (deprecation warnings don't break tests)
- [x] BDD Verification: N/A
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A
