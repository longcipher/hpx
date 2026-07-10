# Tasks: Security Hardening, DX Quality, and Performance

> **Spec:** `specs/2026-07-10-01-securitydx-quality/`
> **Source of Truth:** `features/*.feature`

## Phase 1: SSRF Prevention (Finding 1: SEC-013)

### Task 1.1: Wire SSRF check into HttpClient::execute_single_request

> **Context:** The SSRF module is fully implemented and tested but never called. This task integrates it at the HttpClient level.
> **Verification:** `cargo test -p hpx-browser` — existing SSRF tests pass; new integration test for HttpClient SSRF rejection passes.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — HttpClient must continue working for public IPs.
- **Simplification Focus:** `Reuse existing ssrf module as-is; no new code in ssrf.rs.`
- **Status:** 🟢 DONE
- [x] Step 1: Add `allow_private_network: bool` field to HttpClient configuration struct in `crates/hpx-browser/src/net/mod.rs`.
- [x] Step 2: In `execute_single_request`, after URL parsing, call `crate::net::ssrf::is_forbidden_host(url.host_str().unwrap_or(""))`. Return an error if forbidden and `allow_private_network` is false.
- [x] Step 3: Write a unit test in `crates/hpx-browser/src/net/mod.rs` `#[cfg(test)]` that verifies HttpClient rejects 127.0.0.1 by default and allows it with `allow_private_network: true`.
- [x] Step 4: Run `cargo test -p hpx-browser` — all tests pass.
- [x] BDD Verification: N/A — SSRF is an internal safety check; verified by unit test.
- [x] Advanced Test Verification: `cargo test -p hpx-browser -- --nocapture` — confirm SSRF rejection error message is user-friendly.
- [x] Runtime Verification: `cargo check --workspace` — no regressions.

### Task 1.2: Wire --allow-private-network CLI flag

> **Context:** The CLI flag exists but is explicitly ignored (`let _ = (...)` in browser.rs).
> **Verification:** `cargo run -- fetch http://127.0.0.1` fails; `cargo run -- fetch http://127.0.0.1 --allow-private-network` succeeds.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` for all other CLI flags.
- **Simplification Focus:** `Remove the let _ = (...) ignore; pass the bool through to HttpClient.`
- **Status:** 🟢 DONE
- [x] Step 1: In `bin/hpx-cli/src/browser.rs`, remove `let _ = (obey_robots, allow_private_network, v8_flags, storage_dir);` and the pony-tail comment.
- [x] Step 2: Pass `allow_private_network` to the HttpClient/Page configuration used in fetch and scrape commands.
- [x] Step 3: Do the same for the `serve` subcommand (CDP server) at line ~402.
- [x] Step 4: Add `--allow-private-network` to the CLI help text: "Allow requests to private/internal IP addresses (disabled by default for security)."
- [x] Step 5: Run `cargo check -p hpx-cli` — compiles.
- [x] BDD Verification: `just bdd` — passes.
- [x] Advanced Test Verification: `cargo test -p hpx-cli` — pass.
- [x] Runtime Verification: Manual test: `cargo run -- fetch http://127.0.0.1 2>&1 | grep -i "forbidden"` exits non-zero; `cargo run -- fetch http://93.184.216.34` succeeds.

### Task 1.3: Add SSRF integration test for hpx-browser

> **Context:** Ensure the SSRF integration survives refactoring by testing at the HttpClient boundary.
> **Verification:** `cargo test -p hpx-browser` — new test verifies SSRF behavior end-to-end.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Pure additive — no behavior change.`
- **Simplification Focus:** `Use the existing test patterns in ssrf.rs as template.`
- **Status:** 🟢 DONE
- [x] Step 1: Add `ssrf_integration.rs` in `crates/hpx-browser/tests/` that tests HttpClient with a local HTTP server bound to 127.0.0.1 — verify SSRF blocks it by default.
- [x] Step 2: Same test with `allow_private_network: true` — verify it connects.
- [x] Step 3: Run `cargo test -p hpx-browser` — all tests pass.
- [x] BDD Verification: N/A — integration test covers the behavior.
- [x] Advanced Test Verification: `cargo test -p hpx-browser --test ssrf_integration` — pass.
- [x] Runtime Verification: N/A.

## Phase 2: SSLKEYLOGFILE Gate (Finding 2: SEC-014)

### Task 2.1: Add keylog feature flag and gate KeyLog module

> **Context:** SSLKEYLOGFILE is a debugging feature that exposes TLS session keys. Should be opt-in via Cargo feature.
> **Verification:** `cargo check -p hpx --no-default-features` — KeyLog::from_env() is a no-op; `cargo check -p hpx --features keylog` — KeyLog works as before.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior when keylog feature is enabled.`
- **Simplification Focus:** `Use cfg(feature = "keylog") to conditionally compile; provide stub when disabled.`
- **Status:** 🟢 DONE
- [x] Step 1: In `crates/hpx/Cargo.toml`, add `keylog = []` to `[features]` (NOT in default).
- [x] Step 2: In `crates/hpx/src/tls/keylog.rs`, add `#[cfg(feature = "keylog")]` before the existing `mod handle;` and impl block.
- [x] Step 3: Add `#[cfg(not(feature = "keylog"))]` module that defines `KeyLog` as a unit struct with `from_env() -> Self { KeyLog }` and `handle() -> Result<Handle>` returning a no-op Handle.
- [x] Step 4: In TLS backend files (`openssl.rs`, `rustls.rs`), gate `KeyLog::handle()` calls behind `#[cfg(feature = "keylog")]`.
- [x] Step 5: Run `cargo check -p hpx --no-default-features` — compiles without keylog.
- [x] Step 6: Run `cargo check -p hpx --features keylog` — compiles with keylog.
- [x] BDD Verification: N/A — configuration-only change.
- [x] Advanced Test Verification: `cargo test -p hpx --features keylog` — existing tests pass.
- [x] Runtime Verification: `SSLKEYLOGFILE=/tmp/keys.log cargo test -p hpx --features keylog` — key file is created.

### Task 2.2: Document the keylog feature

> **Context:** Users need to know the keylog feature exists and is opt-in for security.
> **Verification:** README and crate docs mention the `keylog` feature.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Documentation only — no code behavior change.`
- **Simplification Focus:** `Add one line to the feature table.`
- **Status:** 🟢 DONE
- [x] Step 1: In README.md, add `keylog` row to the hpx features table under "TLS" section: "TLS key logging to file (for debugging) — NOT for production".
- [x] Step 2: In `crates/hpx/src/tls/keylog.rs`, add `#[cfg(feature = "keylog")]` module doc noting the feature gate requirement.
- [x] BDD Verification: N/A — documentation only.
- [x] Advanced Test Verification: N/A — documentation only.
- [x] Runtime Verification: `cargo doc -p hpx --no-deps` — docs build without errors.

## Phase 3: Path Traversal Prevention (Finding 3: SEC-012)

### Task 3.1: Add download path validation

> **Context:** The download engine accepts user-supplied destination paths without sanitization, enabling path traversal attacks.
> **Verification:** Unit test confirms `../../etc/passwd` is rejected, `./downloads/file.bin` is accepted.

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Reject paths with .. components or absolute paths by default.`
- **Simplification Focus:** `Simple path component check using std::path::Component.`
- **Status:** 🟢 DONE
- [x] RED: Write unit test in `crates/hpx-dl/src/engine.rs` `#[cfg(test)]` that calls `validate_download_path` with `Path::new("../../etc/passwd")` — assert Err.
- [x] RED: Same test with `Path::new("/etc/passwd")` — assert Err.
- [x] RED: Same test with `Path::new("./downloads/file.bin")` — assert Ok.
- [x] GREEN: Implement `fn validate_download_path(path: &Path) -> Result<(), DownloadError>` that iterates components and rejects `ParentDir` and `RootDir`.
- [x] GREEN: Run `cargo test -p hpx-dl` — all tests pass including new ones.
- [x] REFACTOR: Clean up; verify `cargo test -p hpx-dl` still passes.
- [x] BDD Verification: `just bdd` — passes.
- [x] Advanced Test Verification: `cargo test -p hpx-dl -- --nocapture` — verify error messages are clear.
- [x] Runtime Verification: `cargo run -- dl add https://example.com/file.bin -o ../../etc/passwd 2>&1 | grep -i "traversal"` — exits non-zero.

### Task 3.2: Call validate_download_path before ensure_destination_parent

> **Context:** The validation function exists but must be integrated into the download pipeline.
> **Verification:** Download with traversal path fails before any file I/O.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Reject traversal paths early — before dir creation.`
- **Simplification Focus:** `One line inserted before ensure_destination_parent call.`
- **Status:** 🟢 DONE
- [x] Step 1: In `crates/hpx-dl/src/engine.rs`, find the call site of `ensure_destination_parent` and insert `validate_download_path(&destination)?;` before it.
- [x] Step 2: Run `cargo test -p hpx-dl` — all tests pass.
- [x] BDD Verification: `just bdd` — passes.
- [x] Advanced Test Verification: `cargo test -p hpx-dl --test cucumber` — passes.
- [x] Runtime Verification: N/A — covered by unit test in Task 3.1.

## Phase 4: Checksum Buffer Allocation (Finding 4: PERF-17)

### Task 4.1: Replace Vec allocation with thread-local buffer

> **Context:** `compute_checksum` allocates a 64KB Vec on every call. Thread-local avoids allocation.
> **Verification:** The function produces identical hashes before and after, and `cargo test -p hpx-dl` passes.

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Hash output must be byte-identical to current implementation.`
- **Simplification Focus:** `Replace vec![] with thread_local! static; no API change.`
- **Status:** 🟢 DONE
- [x] RED: Add a test that calls `compute_checksum` twice and verifies the hash is identical both times.
- [x] GREEN: Replace `let mut buf = vec![0u8; 65536];` with `thread_local! { static BUF: std::cell::RefCell<Box<[u8; 65536]>> = const { std::cell::RefCell::new(Box::new([0u8; 65536])) }; }` and borrow the buffer.
- [x] GREEN: Run `cargo test -p hpx-dl` — all existing checksum tests pass.
- [x] REFACTOR: Verify `cargo test -p hpx-dl` still passes.
- [x] BDD Verification: `just bdd` — passes.
- [x] Advanced Test Verification: `cargo bench -p hpx-dl --bench checksum` if benchmark exists, else `cargo test -p hpx-dl --release -- --nocapture` — no regression.
- [x] Runtime Verification: N/A.

### Task 4.2: Document the optimization

> **Context:** Performance optimization should be noted.
> **Verification:** Code comment explains why thread-local was chosen.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Documentation only.`
- **Simplification Focus:** `One comment line in checksum.rs.`
- **Status:** 🟢 DONE
- [x] Step 1: Add comment above the thread_local! block: "Thread-local buffer avoids per-call 64 KiB heap allocation in compute_checksum hot path."
- [x] BDD Verification: N/A.
- [x] Advanced Test Verification: N/A.
- [x] Runtime Verification: N/A.

## Phase 5: Cargo.lock (Finding 5: DEPS-05)

### Task 5.1: Generate and commit Cargo.lock

> **Context:** Missing Cargo.lock means non-reproducible builds. This is a compliance requirement.
> **Verification:** `cargo build --workspace` succeeds with the committed Cargo.lock.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Additive — no code change. All existing dependencies locked to current versions.`
- **Simplification Focus:** `One command: cargo update; git add Cargo.lock; git commit.`
- **Status:** 🟢 DONE
- [x] Step 1: Run `cargo update` at workspace root to generate `Cargo.lock`.
- [x] Step 2: Run `cargo check --workspace` — must pass.
- [x] Step 3: Run `cargo nextest run --workspace` — must pass.
- [x] Step 4: `git add Cargo.lock && git commit -m "chore: add Cargo.lock for reproducible builds"`.
- [x] BDD Verification: N/A — configuration only.
- [x] Advanced Test Verification: `cargo build --workspace --locked` — succeeds.
- [x] Runtime Verification: N/A.

## Phase 6: AGENTS.md Version Sync (Finding 6: DEPS-08)

### Task 6.1: Update AGENTS.md to match Cargo.toml

> **Context:** AGENTS.md declares wrong dependency versions. AI agents reading it will add wrong deps.
> **Verification:** Every version in AGENTS.md §"Preferred Dependencies and Versions" matches Cargo.toml.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Documentation only — no code change.`
- **Simplification Focus:** `Update 6 lines in AGENTS.md.`
- **Status:** 🟢 DONE
- [x] Step 1: In `AGENTS.md`, update line 33: `clap = "4.6.1"`.
- [x] Step 2: Update line 38: `tokio = "1.52.1"`.
- [x] Step 3: Update line 44: `sqlx = "0.9.0"` (remove `=` prefix).
- [x] Step 4: Update line 47: `arc-swap = "1.9.2"`.
- [x] Step 5: Update line 49: `scc = "3.8.4"`.
- [x] Step 6: Update line 50: `winnow = "1.0.3"`.
- [x] Step 7: Review lines 40-43, 45-46, 48, 51-52 for any other mismatches and fix them.
- [x] BDD Verification: N/A — documentation only.
- [x] Advanced Test Verification: N/A — documentation only.
- [x] Runtime Verification: N/A.

### Task 6.2: Add CI check for AGENTS.md version drift

> **Context:** Prevent future drift between AGENTS.md and Cargo.toml.
> **Verification:** CI fails when versions don't match.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Additive — CI-only addition.`
- **Simplification Focus:** `N/A — small shell script in Justfile.`
- **Status:** 🟢 DONE
- [x] Step 1: Add `check-agents-md` recipe to `Justfile` that extracts versions from AGENTS.md and Cargo.toml, compares them, and exits non-zero on mismatch.
- [x] Step 2: Add `check-agents-md` to the `lint` or `ci` recipe chain.
- [x] Step 3: Run `just lint` — must pass.
- [x] BDD Verification: N/A — CI infrastructure only.
- [x] Advanced Test Verification: `just check-agents-md` — passes.
- [x] Runtime Verification: N/A.

## Phase 7: Deprecated Config Groups Removal (Finding 7: DEBT-14)

### Task 7.1: Remove config_groups.rs and re-exports

> **Context:** 467 lines of deprecated code that `ClientBuilder` methods already replace.
> **Verification:** `cargo check --workspace` compiles; no references to removed types.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Remove deprecated public API. Breaking change — must be version-gated or announced.`
- **Simplification Focus:** `Pure deletion — remove one file and ~10 lines of re-exports.`
- **Status:** 🟢 DONE
- [x] Step 1: Delete `crates/hpx/src/client/http/config_groups.rs`.
- [x] Step 2: In `crates/hpx/src/lib.rs`, remove the 5 `#[allow(deprecated)]` re-exports at lines 375-379.
- [x] Step 3: In `crates/hpx/src/client/http/builder.rs`, remove `#![allow(deprecated)]` from line 1.
- [x] Step 4: In `crates/hpx/tests/http.rs`, update any tests that instantiate `TransportConfigOptions`, `PoolConfigOptions`, etc. to use `ClientBuilder` methods instead.
- [x] Step 5: Run `cargo check --workspace` — compiles.
- [x] Step 6: Run `cargo nextest run --workspace` — all tests pass.
- [x] BDD Verification: N/A — code removal only.
- [x] Advanced Test Verification: `cargo test -p hpx` — all tests pass.
- [x] Runtime Verification: N/A.

### Task 7.2: Bump crate version for breaking change

> **Context:** Removing public API types is a breaking change per semver.
> **Verification:** Version bumped appropriately.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Bump version for breaking change.`
- **Simplification Focus:** `Version bump in Cargo.toml.`
- **Status:** 🟢 DONE
- [x] Step 1: Determine if this should be a major bump (3.0.0) or if a migration window is needed.
- [x] Step 2: Bump `workspace.package.version` in root `Cargo.toml` and all crate `Cargo.toml` files.
- [x] Step 3: Add CHANGELOG entry documenting the removal.
- [x] BDD Verification: N/A.
- [x] Advanced Test Verification: `cargo check --workspace` — compiles.
- [x] Runtime Verification: N/A.

## Phase 8: Cookie Module Unit Tests (Finding 8: TEST-05)

### Task 8.1: Add unit tests for cookie Jar

> **Context:** The cookie store (`crates/hpx/src/cookie.rs`, 20.54 KB) has zero unit tests despite being on every HTTP response path.
> **Verification:** `cargo test -p hpx -- cookie` runs and passes new tests.

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Additive — no behavior change.`
- **Simplification Focus:** `N/A — testing-only task.`
- **Status:** 🟢 DONE
- [x] RED: Add `#[cfg(test)] mod tests` to `crates/hpx/src/cookie.rs` with a scenario: create Jar, call `set_cookies` with a Set-Cookie header, assert `cookies` returns the expected cookie.
- [x] RED: Add scenario: cookie set for `example.com` is NOT returned for `other.com`.
- [x] RED: Add scenario: cookie with `path=/app` is returned for `/app/page` but not `/other`.
- [x] RED: Add scenario: expired cookie (Expires in past) is not returned.
- [x] RED: Add scenario: concurrent insert/read from multiple tokio tasks does not panic.
- [x] GREEN: Run `cargo test -p hpx -- cookie` — all tests pass.
- [x] REFACTOR: Clean up; verify `cargo test -p hpx -- cookie` still passes.
- [x] BDD Verification: N/A — unit tests cover the scenarios.
- [x] Advanced Test Verification: `cargo test -p hpx -- cookie -- --nocapture` — review test output.
- [x] Runtime Verification: N/A.

### Task 8.2: Add unit tests for cookie header parsing edge cases

> **Context:** Cookie header parsing handles malformed input; edge cases must be verified.
> **Verification:** Tests cover empty cookie, malformed attributes, multiple cookies.

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Additive — no behavior change.`
- **Simplification Focus:** `Add test cases using existing parse_set_cookies function.`
- **Status:** 🟢 DONE
- [x] RED: Add test: empty Set-Cookie header returns empty vec.
- [x] RED: Add test: multiple Set-Cookie headers all parsed.
- [x] RED: Add test: cookie with Secure, HttpOnly, SameSite=Strict all parsed correctly.
- [x] GREEN: Run `cargo test -p hpx -- cookie` — all tests pass.
- [x] REFACTOR: Clean up.
- [x] BDD Verification: N/A.
- [x] Advanced Test Verification: `cargo test -p hpx -- cookie` — pass.
- [x] Runtime Verification: N/A.

## Phase 9: Retry Module Unit Tests (Finding 9: TEST-06)

### Task 9.1: Add unit tests for RetryPolicy

> **Context:** `crates/hpx/src/retry.rs` (6.38 KB) has zero unit tests.
> **Verification:** Tests cover scope exclusion, budget arithmetic, classifier chaining.

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Additive — no behavior change.`
- **Simplification Focus:** `N/A — testing-only task.`
- **Status:** 🟢 DONE
- [x] RED: Add `#[cfg(test)] mod tests` to `crates/hpx/src/retry.rs`. Test: default Policy does not retry POST.
- [x] RED: Test: Policy with `scoped(Method::POST)` retries POST on server error.
- [x] RED: Test: budget of 3 is exhausted after 3 retries; 4th transient error does not retry.
- [x] RED: Test: classifier chaining — two classifiers combined produce expected result.
- [x] GREEN: Run `cargo test -p hpx -- retry` — all tests pass.
- [x] REFACTOR: Clean up.
- [x] BDD Verification: N/A.
- [x] Advanced Test Verification: `cargo test -p hpx -- retry` — pass.
- [x] Runtime Verification: N/A.

### Task 9.2: Add proptest for retry budget invariants

> **Context:** Retry budget must never go negative; behavior must be monotonic.
> **Verification:** proptest verifies invariants across random inputs.

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Additive — no behavior change.`
- **Simplification Focus:** `Use existing proptest patterns from other hpx modules.`
- **Status:** 🟢 DONE
- [x] RED: Add proptest: for any sequence of retry/not-retry decisions with budget N, remaining budget never exceeds N and never goes below 0.
- [x] GREEN: Run `cargo test -p hpx -- retry` — proptest passes.
- [x] REFACTOR: Clean up.
- [x] BDD Verification: N/A.
- [x] Advanced Test Verification: `cargo test -p hpx -- retry -- --nocapture` — proptest report confirms 1000+ cases.
- [x] Runtime Verification: N/A.

## Phase 10: Redirect Module Unit Tests (Finding 10: TEST-07)

### Task 10.1: Add unit tests for redirect header stripping

> **Context:** `crates/hpx/src/redirect.rs` (18.6 KB) has zero unit tests. Cross-origin redirect header stripping is security-critical.
> **Verification:** Tests verify Authorization is stripped on cross-origin but preserved on same-origin.

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Additive — no behavior change.`
- **Simplification Focus:** `N/A — testing-only task.`
- **Status:** 🟢 DONE
- [x] RED: Add `#[cfg(test)] mod tests` to `crates/hpx/src/redirect.rs`. Test: `remove_sensitive_headers` strips Authorization, Cookie, Proxy-Authorization on cross-origin.
- [x] RED: Test: same-origin preserves Authorization.
- [x] RED: Test: REFERER policy variants produce expected headers.
- [x] RED: Test: redirect loop detection stops at max_redirects.
- [x] GREEN: Run `cargo test -p hpx -- redirect` — all tests pass.
- [x] REFACTOR: Clean up.
- [x] BDD Verification: N/A.
- [x] Advanced Test Verification: `cargo test -p hpx -- redirect` — pass.
- [x] Runtime Verification: N/A.

### Task 10.2: Add proptest for redirect attempt invariants

> **Context:** Redirect attempt counter must never exceed configured maximum.
> **Verification:** proptest verifies invariants.

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Additive — no behavior change.`
- **Simplification Focus:** `Use existing proptest patterns.`
- **Status:** 🟢 DONE
- [x] RED: Add proptest: for any redirect sequence, `attempt.used()` never exceeds `attempt.remaining()` + used count.
- [x] GREEN: Run `cargo test -p hpx -- redirect` — proptest passes.
- [x] REFACTOR: Clean up.
- [x] BDD Verification: N/A.
- [x] Advanced Test Verification: `cargo test -p hpx -- redirect -- --nocapture` — pass.
- [x] Runtime Verification: N/A.

## Phase 11: Proxy Module Unit Tests (Finding 11: TEST-08)

### Task 11.1: Add unit tests for proxy rule matching

> **Context:** `crates/hpx/src/proxy.rs` (16.94 KB) has zero unit tests.
> **Verification:** Tests verify URL pattern matching, auth header construction, no_proxy exclusions.

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Additive — no behavior change.`
- **Simplification Focus:** `N/A — testing-only task.`
- **Status:** 🟢 DONE
- [x] RED: Add `#[cfg(test)] mod tests` to `crates/hpx/src/proxy.rs`. Test: proxy with rule `*.example.com` matches `api.example.com` but not `other.com`.
- [x] RED: Test: custom_http_auth produces correct Proxy-Authorization header.
- [x] RED: Test: NO_PROXY exclusion prevents proxy usage for matching URLs.
- [x] RED: Test: proxy URL construction handles username/password.
- [x] GREEN: Run `cargo test -p hpx -- proxy` — all tests pass.
- [x] REFACTOR: Clean up.
- [x] BDD Verification: N/A.
- [x] Advanced Test Verification: `cargo test -p hpx -- proxy` — pass.
- [x] Runtime Verification: N/A.

### Task 11.2: Add integration test for proxy configuration

> **Context:** Verify proxy configuration flows from builder to HTTP request.
> **Verification:** Integration test confirms proxy is applied correctly.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Additive — no behavior change.`
- **Simplification Focus:** `Follow existing hpx/tests/ integration test patterns.`
- **Status:** 🟢 DONE
- [x] Step 1: Add a test in `crates/hpx/tests/proxy.rs` (or extend existing) that configures a proxy and verifies the request targets the proxy.
- [x] Step 2: Run `cargo test -p hpx --test proxy` — pass.
- [x] BDD Verification: N/A.
- [x] Advanced Test Verification: `cargo test -p hpx --test proxy` — pass.
- [x] Runtime Verification: N/A.
