# Design: Codebase Quality — Correctness, Security, Test Coverage, Debt Reduction, Architecture

| Metadata | Details |
| :--- | :--- |
| **Status** | Draft |
| **Created** | 2026-06-25 |
| **Mode** | Full |
| **Priority** | P1 |
| **Planned at** | commit `0156ce7`, 2026-06-25 |

## Summary

Consolidated audit findings covering correctness bugs (retry budget leak, redirect panic, UB in compression, scheduler race, queue O(n)), security issues (cookie jar, JSON injection, CDP access control, credential persistence), test coverage gaps (CI misconfiguration, untested codecs), tech debt (dead lints, dead features, dead code), and architecture improvements (error type consolidation, config group deprecation).

## Why this matters

The hpx crate targets HFT crypto exchange workloads where correctness and security are non-negotiable. The retry budget bug silently disables retries for streaming bodies. The redirect panic crashes the client on misconfiguration. The compression UB violates Rust's safety contract. The CI misconfiguration means integration tests and BDD scenarios never run in CI, giving false green signals.

## Approach

Fix independent bugs first (S-effort, zero risk), then fix CI to establish a verification baseline, then tackle debt and architecture in dependency order. All fixes follow the workspace conventions: `thiserror` for library errors, `scc`/`arc-swap` for concurrency, clippy pedantic+nursery enabled.

## Findings

### Finding 1: Retry budget leaked when clone fails (COR-02)

- **Category:** correctness
- **Impact:** HIGH
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-01]:** WHEN `retry()` signals `Retryable` AND `clone_request()` returns `None`, THE retry budget SHALL NOT be decremented.
- **[REQ-02]:** WHEN `retry()` signals `Retryable` AND `clone_request()` returns `Some`, THE retry budget SHALL be decremented by one token.

#### Current state

`crates/hpx/src/client/layer/retry.rs:81` — `budget.withdraw()` is called inside `retry()` before tower calls `clone_request()`. If `clone_request()` returns `None` (body not clonable, max retries hit), the token is consumed but no retry occurs.

#### Approach

Move `budget.withdraw()` from `retry()` into `clone_request()`, after the scope and max-retry checks pass but before returning the cloned request. This ensures the budget is only consumed when a retry actually happens.

#### Architecture Decisions (MADR Format)

- **AD-01:** Move budget withdrawal to `clone_request()`
  - **Context:** Tower's `Retry` middleware calls `retry()` first, then `clone_request()`. The budget is consumed in `retry()` even when `clone_request()` fails.
  - **Decision:** Move `budget.withdraw()` into `clone_request()` after the limit check passes.
  - **Consequences:** Budget is only consumed on actual retries. No API change. No behavior change for successful retries.

---

### Finding 2: expect() in redirect panics on missing config (COR-04)

- **Category:** correctness
- **Impact:** HIGH
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-03]:** WHEN `FollowRedirectPolicy` has no policy set, THE redirect middleware SHALL return an error, not panic.
- **[REQ-04]:** WHEN a redirect is processed normally, THE existing behavior SHALL be preserved.

#### Current state

`crates/hpx/src/redirect.rs:394` — `self.policy.as_ref().expect("[BUG] ...")` panics if `RequestConfig` was not set by a preceding middleware layer.

#### Approach

Replace `expect()` with `.ok_or_else(|| Error::redirect(...))` to return a recoverable error. The error message should indicate the missing policy configuration.

---

### Finding 3: unsafe MaybeUninit cast is UB (COR-05)

- **Category:** correctness
- **Impact:** MEDIUM
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-05]:** THE WebSocket compression buffer allocation SHALL NOT create references to uninitialized memory.
- **[REQ-06]:** THE compressed output SHALL be byte-identical to the current implementation.

#### Current state

`crates/yawc/src/compression.rs:415` — `unsafe { &mut *(uninitbuf as *mut [MaybeUninit<u8>] as *mut [u8]) }` creates a `&mut [u8]` over uninitialized memory, which is UB under stacked borrows.

#### Approach

Replace with `MaybeUninit::slice_assume_init_mut` on the written portion only, or use `Vec<u8>` as an intermediate buffer and copy into `BytesMut`. The `chunk()` function returns `&mut [u8]` which flate2 needs, so the simplest safe fix is to zero-initialize the spare capacity first: `output.spare_capacity_mut().fill(MaybeUninit::new(0))` before the cast. This is a single-line addition with negligible perf cost (the buffer is reserved anyway).

---

### Finding 4: Scheduler race can strand downloads (COR-01)

- **Category:** correctness
- **Impact:** MEDIUM
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-07]:** WHEN a download is added while the scheduler is shutting down, THE scheduler SHALL restart and process the download.
- **[REQ-08]:** THE scheduler SHALL NOT leave downloads permanently queued when permits are available.

#### Current state

`crates/hpx-dl/src/engine.rs:594-601` — After `scheduler_running.store(false, Release)`, the queue re-check is a separate non-atomic operation. A concurrent `add()` in this window can leave the download stranded.

#### Approach

Replace the `store(false)` + re-check pattern with a CAS loop: if queue is non-empty and permits are available, CAS `false→true` and re-enter the loop. This eliminates the TOCTOU window.

---

### Finding 5: PriorityQueue::remove is O(n) with allocation (COR-09)

- **Category:** correctness
- **Impact:** MEDIUM
- **Effort:** M

#### Requirements (EARS Notation)

- **[REQ-09]:** THE queue remove operation SHALL NOT allocate a temporary Vec.
- **[REQ-10]:** THE queue ordering SHALL be preserved after removal.

#### Current state

`crates/hpx-dl/src/queue.rs:108-125` — `remove()` drains the entire heap into a `Vec`, searches linearly, then re-pushes all remaining entries.

#### Approach

Add a tombstone set (`ahash::HashSet<DownloadId>`) alongside the heap. `remove()` inserts the ID into the tombstone set. `pop()` skips tombstoned entries. This makes `remove()` O(1) amortized at the cost of occasional wasted pops.

---

### Finding 6: Cookie jar only saves first Set-Cookie (SEC-002)

- **Category:** security
- **Impact:** MEDIUM
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-11]:** THE cookie jar SHALL save ALL Set-Cookie headers from a response.
- **[REQ-12]:** THE cookie jar SHALL append to existing cookies, not overwrite.

#### Current state

`bin/hpx-cli/src/http.rs:185-188` — `.get("set-cookie")` returns only the first header. Line 209 overwrites the file.

#### Approach

Use `response.headers().get_all("set-cookie")` to collect all values. Append to the file instead of overwriting.

---

### Finding 7: JSON injection in NDJSON output (SEC-003)

- **Category:** security
- **Impact:** MEDIUM
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-13]:** THE NDJSON output SHALL produce valid JSON on each line.
- **[REQ-14]:** THE URL and type values SHALL be properly escaped.

#### Current state

`bin/hpx-cli/src/browser.rs:215-216` — `format!("{{\"url\":\"{url}\",\"type\":\"{asset_type}\"}}\n")` uses raw string interpolation without escaping.

#### Approach

Use `serde_json::to_string()` for each value, or `serde_json::json!()` macro to produce each line. This handles all escaping automatically.

---

### Finding 8: CDP serve has no access control (SEC-009)

- **Category:** security
- **Impact:** HIGH
- **Effort:** M

#### Requirements (EARS Notation)

- **[REQ-15]:** WHEN `hpx serve` binds to a non-loopback address, THE CLI SHALL print a security warning.
- **[REQ-16]:** THE default host SHALL remain 127.0.0.1.

#### Current state

`bin/hpx-cli/src/browser.rs:390-441` — `handle_serve` starts a CDP WebSocket server with no authentication, CORS, or origin checking. The `_host` parameter is unused (binds to port only).

#### Approach

1. Wire the `_host` parameter to the actual bind address.
2. When host is not loopback, print `tracing::warn!` about security implications.
3. Document that CDP gives full browser control and should not be exposed to untrusted networks.

---

### Finding 9: Proxy credentials persisted to SQLite (SEC-011)

- **Category:** security
- **Impact:** HIGH
- **Effort:** M

#### Requirements (EARS Notation)

- **[REQ-17]:** THE persistence layer SHALL strip credentials from proxy URLs before storage.
- **[REQ-18]:** THE stripped URL SHALL preserve the host, port, and scheme.

#### Current state

`crates/hpx-dl/src/persistence.rs:60` — `DownloadRecord` (containing `DownloadRequest` with proxy URL) is serialized to SQLite with credentials intact.

#### Approach

Before serializing, parse the proxy URL and strip `userinfo` (username:password@). Use `url::Url` to strip the authority. Store the sanitized URL. On load, the proxy URL will lack credentials — this is acceptable since proxy auth is typically session-scoped.

---

### Finding 10: CI skips integration and BDD tests (TEST-02)

- **Category:** testing
- **Impact:** MEDIUM
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-19]:** THE `just ci` target SHALL run lint, all unit tests, all integration tests, and BDD scenarios.
- **[REQ-20]:** THE `just test` target SHALL run all workspace tests including hpx-dl.

#### Current state

`Justfile:31` — `ci: lint test` where `test` only runs a subset. `test` excludes `hpx-dl` and all integration tests under `crates/hpx/tests/`.

#### Approach

- Change `test` to `cargo nextest run --workspace --all-features`
- Change `ci` to `lint test-all`

---

### Finding 11: hpx-streams near-zero test coverage (TEST-01)

- **Category:** testing
- **Impact:** MEDIUM
- **Effort:** M

#### Requirements (EARS Notation)

- **[REQ-21]:** THE hpx-streams crate SHALL have unit tests for each codec module.
- **[REQ-22]:** THE tests SHALL cover normal parse, empty input, and truncated payload.

#### Current state

`crates/hpx-streams/src/` — 2 tests in ~1134 lines. `json_stream.rs`, `protobuf_stream.rs`, `arrow_ipc_stream.rs`, `json_array_codec.rs`, `protobuf_len_codec.rs`, `arrow_ipc_len_codec.rs` have zero tests.

#### Approach

Add `#[cfg(test)]` modules to each codec file with tests for: normal parse, empty input, truncated frame, and oversized payload.

---

### Finding 12: yawc codec has zero tests (TEST-04)

- **Category:** testing
- **Impact:** MEDIUM
- **Effort:** M

#### Requirements (EARS Notation)

- **[REQ-23]:** THE yawc codec SHALL have roundtrip encode/decode tests.
- **[REQ-24]:** THE tests SHALL cover text, binary, close, ping/pong frames and all payload length variants.

#### Current state

`crates/yawc/src/codec.rs` — no `#[cfg(test)]` module, no tests.

#### Approach

Add tests for: text frame roundtrip, binary frame roundtrip, close frame roundtrip, ping/pong roundtrip, 7-bit/16-bit/64-bit payload lengths, and masking.

---

### Finding 13: hpx-browser blanket-disables 13 lint categories (DEBT-05)

- **Category:** debt
- **Impact:** HIGH
- **Effort:** L

#### Requirements (EARS Notation)

- **[REQ-25]:** THE hpx-browser crate SHALL enforce `clippy::all` lints.
- **[REQ-26]:** THE remaining lint allows SHALL be narrowed to specific items, not blanket.

#### Current state

`crates/hpx-browser/src/lib.rs:1-13` — 13 blanket `#![allow(...)]` directives covering all clippy groups, dead_code, missing_docs, unused_variables, and rustdoc.

#### Approach

Remove blanket allows incrementally: first enable `clippy::all` and fix correctness warnings, then enable pedantic and fix style warnings. For items that are intentionally allowed (e.g., `missing_docs` on internal modules), add targeted `#[allow(...)]` with justification.

---

### Finding 14: `#![allow(dead_code)]` defeats lint in hpx core (DEBT-04)

- **Category:** debt
- **Impact:** MEDIUM
- **Effort:** M

#### Requirements (EARS Notation)

- **[REQ-27]:** THE hpx crate SHALL NOT have blanket `#![allow(dead_code)]`.
- **[REQ-28]:** GENUINELY dead code SHALL be removed; intentional unused items SHALL have targeted allows.

#### Current state

`crates/hpx/src/lib.rs:2` — `#![allow(dead_code)]` alongside `#![deny(unused)]`, partially neutering the deny.

#### Approach

Remove the blanket allow. Run clippy, fix genuine dead code, add narrow `#[allow(dead_code)]` for items that are intentionally kept (e.g., public API items used by downstream crates but not internally).

---

### Finding 15: Dead feature flags in hpx-dl (DEBT-02)

- **Category:** debt
- **Impact:** LOW
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-29]:** THE hpx-dl features section SHALL NOT contain features with zero `cfg` usage.

#### Current state

`crates/hpx-dl/Cargo.toml:29,31` — `progress = []` and `storage = ["sqlite"]` have zero `cfg` guards.

#### Approach

Remove both feature declarations from `Cargo.toml`.

---

### Finding 16: fast_random wrapper is unnecessary indirection (DEBT-08)

- **Category:** debt
- **Impact:** LOW
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-30]:** THE `fast_random` function SHALL be removed and call sites SHALL use `rand::random::<u64>()` directly.

#### Current state

`crates/hpx/src/util.rs:33-35` — `fast_random()` is a 3-line wrapper over `rand::random::<u64>()` used in 3 places.

#### Approach

Replace the 3 call sites with `rand::random::<u64>()` and delete `fast_random`.

---

### Finding 17: Consolidate error type hierarchies (Direction 2)

- **Category:** architecture
- **Impact:** MEDIUM
- **Effort:** L

#### Requirements (EARS Notation)

- **[REQ-31]:** THE hpx crate SHALL have a single primary error type (`hpx::Error`).
- **[REQ-32]:** THE `core::Error` and `client::Error` types SHALL be consolidated into `hpx::Error`.
- **[REQ-33]:** THE duplicate `BoxError`, `TimedOut`, and source-chain walkers SHALL be eliminated.
- **[REQ-34]:** EXISTING error kind variants SHALL be preserved for downstream compatibility.

#### Current state

Three error hierarchies: `crates/hpx/src/error.rs` (473 lines), `crates/hpx/src/client/core/error.rs` (438 lines), `crates/hpx/src/client/http/client/error.rs` (142 lines). Each has its own `BoxError` alias, `TimedOut` sentinel, and source-chain walking logic.

#### Approach

Design spike first: map all error variants across the three types, identify overlaps and unique variants, design a unified enum. Then implement: create unified `hpx::Error` with all variants, add `From` impls for the old types, update all internal error construction, deprecate the old types.

#### Architecture Decisions (MADR Format)

- **AD-02:** Unify into single hpx::Error
  - **Context:** Three error types with ~900 total lines, duplicate BoxError/TimedOut definitions.
  - **Decision:** Merge into `hpx::Error` using thiserror, keeping all unique variants.
  - **Consequences:** Reduced maintenance burden, clearer API for users. Breaking change for downstream code that matches on `core::Error` or `client::Error` specifically.

---

### Finding 18: Deprecate config group structs (Direction 3)

- **Category:** architecture
- **Impact:** MEDIUM
- **Effort:** M

#### Requirements (EARS Notation)

- **[REQ-35]:** THE config group structs SHALL emit deprecation warnings.
- **[REQ-36]:** THE `ClientBuilder` methods SHALL be the single source of truth for configuration.
- **[REQ-37]:** THE config group code SHALL delegate to `ClientBuilder` internally.

#### Current state

`crates/hpx/src/client/http/config_groups.rs` — ~441 lines of `TransportConfigOptions`, `TlsConfigOptions`, `ProtocolConfigOptions`, `ProxyConfigOptions`, `PoolConfigOptions` that duplicate `ClientBuilder` methods 1:1.

#### Approach

Add `#[deprecated]` to each config group struct. Keep the delegation code to avoid breaking existing users. Document `ClientBuilder` as the primary configuration API.

---

## Architecture Decisions

### AD-01: Move budget withdrawal to clone_request()

- **Context:** Tower calls `retry()` before `clone_request()`. Budget is consumed in `retry()` even when clone fails.
- **Decision:** Move `budget.withdraw()` into `clone_request()` after limit checks.
- **Consequences:** Budget only consumed on actual retries. No API change.

### AD-02: Unify into single hpx::Error

- **Context:** Three error types with ~900 total lines.
- **Decision:** Merge into `hpx::Error` using thiserror.
- **Consequences:** Breaking change for downstream code matching on specific error types. Reduced maintenance.

### AD-03: Deprecate config groups, consolidate on ClientBuilder

- **Context:** ~441 lines of config group structs duplicate ClientBuilder methods.
- **Decision:** Deprecate config groups, keep ClientBuilder as primary API.
- **Consequences:** Breaking warning for users of config groups. Simplified maintenance.

## BDD/TDD Strategy

- **Primary Language:** Rust
- **BDD Runner:** cucumber-rs (`cargo test -p hpx-dl --test cucumber --all-features`)
- **Unit Test Command:** `cargo nextest run --workspace --all-features`
- **Feature Files:** `specs/2026-06-25-01-codebase-quality/features/*.feature`
- **Outside-in Loop:** Each finding has a RED→GREEN→REFACTOR cycle driven by the feature scenarios

## Code Simplification Constraints

**Ponytail Ladder (mandatory at every decision point):**

1. Does this need to exist at all? Speculative need = skip it. (YAGNI)
2. Stdlib does it? Use it.
3. Native platform feature covers it? Use it.
4. Already-installed dependency? Use it.
5. One line? One line.
6. Only then: minimum code that works.

**Mark deferrals:** Use `ponytail:` comments for deliberate simplifications with known ceilings.

**Never simplify away:** input validation, error handling, security, accessibility, anything explicitly requested.

**Additional constraints:**

- **Behavioral Contract:** Preserve existing behavior unless a listed scenario or requirement explicitly changes it.
- **Repo Standards:** Use only the coding standards established by `AGENTS.md` and the existing codebase.
- **Refactor Scope:** Limit cleanup to touched modules unless the design explicitly justifies broader refactor.

## BDD Scenario Inventory

- `features/correctness.feature` — Retry budget preserved on clone failure → Task 1.1, 1.2
- `features/correctness.feature` — Missing redirect policy returns error → Task 2.1, 2.2
- `features/correctness.feature` — Safe compression buffer → Task 3.1, 3.2
- `features/correctness.feature` — Scheduler race fix → Task 4.1, 4.2
- `features/correctness.feature` — Queue remove performance → Task 5.1, 5.2
- `features/security.feature` — Cookie jar saves all cookies → Task 6.1, 6.2
- `features/security.feature` — NDJSON escaping → Task 7.1, 7.2
- `features/security.feature` — CDP serve access control → Task 8.1, 8.2
- `features/security.feature` — Proxy credential stripping → Task 9.1, 9.2
- `features/test-coverage.feature` — CI runs all tests → Task 10.1, 10.2
- `features/test-coverage.feature` — hpx-streams codec tests → Task 11.1, 11.2
- `features/test-coverage.feature` — yawc codec tests → Task 12.1, 12.2
- `features/test-coverage.feature` — Lint target cleanup → Task 13.1
- `features/tech-debt.feature` — hpx dead_code allow removal → Task 14.1, 14.2
- `features/tech-debt.feature` — hpx-browser lint enables → Task 15.1, 15.2
- `features/tech-debt.feature` — Dead feature flags → Task 16.1
- `features/tech-debt.feature` — fast_random removal → Task 17.1
- `features/architecture.feature` — Error type consolidation → Task 18.1–18.4
- `features/architecture.feature` — Config group deprecation → Task 19.1, 19.2

## Existing Components to Reuse

- `url::Url` for proxy URL credential stripping (already a workspace dependency)
- `serde_json` for NDJSON escaping (already available)
- `ahash` for tombstone set in queue (already a workspace dependency)

## Verification

| Purpose   | Command                           | Expected on success |
|-----------|-----------------------------------|---------------------|
| Format    | `cargo +nightly fmt --all`        | exit 0              |
| Lint      | `cargo +nightly clippy --all -- -D warnings` | exit 0, no errors |
| Tests     | `cargo nextest run --workspace --all-features` | all pass |
| BDD       | `cargo test -p hpx-dl --test cucumber --all-features` | all pass |
| All       | `just test-all`                   | all pass            |
