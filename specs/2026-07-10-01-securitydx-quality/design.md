# Design: Security Hardening, DX Quality, and Performance — July 2026 Audit

| Metadata | Details |
| :--- | :--- |
| **Status** | Draft |
| **Created** | 2026-07-10 |
| **Mode** | Full |
| **Priority** | P1 |
| **Planned at** | commit `a29a9ae`, 2026-07-10 |

## Summary

Consolidated audit findings from a deep pb-improve scan. Covers three exploitable security vulnerabilities (SSRF in browser fetch, SSLKEYLOGFILE production exposure, path traversal in downloads), one hot-path performance regression (per-call heap allocation in checksum), two dependency/reproducibility gaps (missing Cargo.lock, AGENTS.md version drift), removal of ~470 lines of deprecated config-group code, and unit test coverage for four critical untested modules (cookie, retry, redirect, proxy). All findings are S-effort (hours each) with LOW risk and HIGH confidence.

## Why this matters

The hpx crate targets crypto exchange HFT workloads. The SSRF vulnerability allows `hpx fetch`/`scrape` to reach internal services — a fully exploitable vector in a tool designed to fetch arbitrary URLs. The SSLKEYLOGFILE feature silently writes TLS session keys to disk in all builds, enabling passive decryption of production traffic. The path traversal in download allows overwriting arbitrary files. These are not theoretical — each has concrete exploit paths. The missing Cargo.lock means builds are not reproducible, which is a compliance gap for any production deployment. The AGENTS.md drift causes AI agents to add wrong dependency versions. The deprecated config groups add 467 lines of dead code to the public API surface. Four critical modules (cookie, retry, redirect, proxy) have zero unit tests despite being on every HTTP request path.

## Approach

All findings are independent and can be implemented in parallel. Security findings take precedence. The SSRF fix reuses existing, tested code (`crates/hpx-browser/src/net/ssrf.rs` — already has comprehensive tests). The SSLKEYLOGFILE fix gates existing functionality behind a new `keylog` feature flag. Path traversal adds a simple sanitization check. The Cargo.lock and AGENTS.md fixes are configuration-only. The deprecated config groups are a pure deletion. Test additions are additive with zero risk.

## Findings

### Finding 1: SSRF protection module never wired into execution path (SEC-013)

- **Category:** security
- **Impact:** HIGH — exploitable SSRF in `hpx fetch`, `hpx scrape`, and CDP server
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-SSRF-01]:** WHEN `hpx fetch <url>` is invoked AND `--allow-private-network` is NOT set, THE SSRF check SHALL reject URLs resolving to loopback, RFC1918, link-local, multicast, broadcast, documentation, unique-local IPv6, and IPv4-mapped IPv6 addresses.
- **[REQ-SSRF-02]:** WHEN `hpx fetch <url>` is invoked AND `--allow-private-network` IS set, THE SSRF check SHALL be bypassed.
- **[REQ-SSRF-03]:** WHEN `hpx scrape <url1> <url2>` is invoked, THE SSRF check SHALL apply independently to each URL.
- **[REQ-SSRF-04]:** WHEN the CDP server receives a navigation request, THE SSRF check SHALL apply (same rules as fetch).

#### Current state

- `crates/hpx-browser/src/net/ssrf.rs` — complete SSRF module with `is_forbidden_ip()` and `is_forbidden_host()`, covering all private/special-use ranges. Has 14 unit tests covering all edge cases. Module is public and fully functional.
- `bin/hpx-cli/src/browser.rs:263` — `let _ = (obey_robots, allow_private_network, v8_flags, storage_dir);` with comment "not yet wired to in-process worker. Ignored for now."
- `bin/hpx-cli/src/browser.rs:402` — same pattern for the `serve` subcommand.
- `crates/hpx-browser/src/net/mod.rs:470` — `execute_single_request` parses the URL and dispatches to the HTTP client. No SSRF check before the request.

#### Approach

1. Add `allow_private_network: bool` field to the HttpClient configuration.
2. In `HttpClient::execute_single_request`, after URL parsing, call `ssrf::is_forbidden_host()` and return an error if forbidden and `allow_private_network` is false.
3. Wire the CLI `--allow-private-network` flag through to the HttpClient.
4. Add the flag to the `serve` subcommand (CDP server) as well.

#### Architecture Decisions (MADR Format)

- **AD-01:** Use existing `ssrf` module as-is — it is tested and complete.
  - **Context:** The ssrf module has 14 unit tests covering all edge cases. No new code needed.
  - **Decision:** Call `ssrf::is_forbidden_host()` at the HttpClient level, not at each CLI command.
  - **Consequences:** Single integration point; all code paths (fetch, scrape, CDP) are covered.

---

### Finding 2: SSLKEYLOGFILE exposed in all builds (SEC-014)

- **Category:** security
- **Impact:** HIGH — TLS session key logging in production builds
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-KEYLOG-01]:** WHEN hpx is compiled WITHOUT the `keylog` feature, THE `KeyLog::from_env()` method SHALL return a no-op `KeyLog` regardless of the `SSLKEYLOGFILE` environment variable.
- **[REQ-KEYLOG-02]:** WHEN hpx is compiled WITH the `keylog` feature, THE existing behavior SHALL be preserved (keys written to file).
- **[REQ-KEYLOG-03]:** THE `keylog` feature SHALL NOT be a default feature.

#### Current state

- `crates/hpx/src/tls/keylog.rs:31-38` — `KeyLog::from_env()` reads `SSLKEYLOGFILE` unconditionally.
- `crates/hpx/src/tls/keylog/handle.rs:20-64` — `Handle` writes NSS Key Log format.
- Used by OpenSSL backend (`crates/hpx/src/tls/openssl.rs`) and Rustls backend.

#### Approach

1. Add a new `keylog` feature flag to `crates/hpx/Cargo.toml`.
2. Gate the entire `keylog` module behind `#[cfg(feature = "keylog")]`.
3. Provide a `#[cfg(not(feature = "keylog"))]` stub `KeyLog::from_env()` that always returns `KeyLog(None)`.
4. Gate the backend usage of `KeyLog::handle()` behind the feature flag.

---

### Finding 3: Path traversal via user-controlled download destination (SEC-012)

- **Category:** security
- **Impact:** HIGH — arbitrary file write via path traversal
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-PATH-01]:** WHEN a download destination contains `..` components, THE download SHALL be rejected with a path traversal error.
- **[REQ-PATH-02]:** WHEN a download destination is an absolute path, THE download SHALL be rejected unless a configured download root allows it.
- **[REQ-PATH-03]:** WHEN a download destination is relative and contains no `..` components, THE download SHALL proceed normally.
- **[REQ-PATH-04]:** WHEN a configured download root is set AND the resolved destination is within that root, THE download SHALL be allowed even for absolute paths.

#### Current state

- `crates/hpx-dl/src/engine.rs:1162-1168` — `ensure_destination_parent` calls `create_dir_all` on the raw user-supplied path with no sanitization.
- `bin/hpx-cli/src/main.rs:207-208` — the `--output` flag value is passed directly as `destination` to `DownloadRequest`.

#### Approach

1. Add a `validate_download_path(destination: &Path) -> Result<(), DownloadError>` function.
2. Check for `..` components and absolute paths; reject with a clear error.
3. Optionally add a `download_root` configuration to `EngineConfig` for allowed paths.
4. Call `validate_download_path` before `ensure_destination_parent`.

---

### Finding 4: Checksum buffer allocates 64KB Vec per call (PERF-17)

- **Category:** performance
- **Impact:** MEDIUM — heap allocation on hot verification path
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-CHECKSUM-01]:** WHEN `compute_checksum` is called, THE 64 KiB read buffer SHALL NOT allocate on the heap.
- **[REQ-CHECKSUM-02]:** THE hash output SHALL be identical to the current implementation.
- **[REQ-CHECKSUM-03]:** THE function SHALL remain thread-safe.

#### Current state

- `crates/hpx-dl/src/checksum.rs:38` — `let mut buf = vec![0u8; 65536];` allocates a new Vec per call.

#### Approach

Replace `vec![0u8; 65536]` with a `thread_local!` static `Box<[u8; 65536]>` that is zeroed on first access and reused. Thread-local avoids synchronization overhead and is safe since `compute_checksum` is `async` but runs on a single thread per call.

---

### Finding 5: Cargo.lock missing from repository (DEPS-05)

- **Category:** dependencies
- **Impact:** MEDIUM — non-reproducible builds
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-LOCK-01]:** A `Cargo.lock` file SHALL exist at the workspace root.
- **[REQ-LOCK-02]:** The lockfile SHALL produce a successful `cargo build` with default features.
- **[REQ-LOCK-03]:** The lockfile SHALL be committed to version control.

#### Current state

- No `Cargo.lock` exists at workspace root. `resolver = "3"` is set in `Cargo.toml`.

#### Approach

Run `cargo update` to generate `Cargo.lock`, verify `cargo check --workspace` passes, commit.

---

### Finding 6: AGENTS.md version drift from Cargo.toml (DEPS-08)

- **Category:** dependencies/documentation
- **Impact:** MEDIUM — AI agents add wrong dependency versions
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-AGENTS-01]:** Every dependency version in `AGENTS.md` §"Preferred Dependencies and Versions" SHALL match the corresponding entry in `Cargo.toml` `[workspace.dependencies]`.
- **[REQ-AGENTS-02]:** A CI check SHALL detect version drift between AGENTS.md and Cargo.toml.

#### Current state

Six known mismatches:

- `clap`: AGENTS.md "4.5.60" vs Cargo.toml "4.6.1"
- `tokio`: AGENTS.md "1.49.0" vs Cargo.toml "1.52.1"
- `sqlx`: AGENTS.md "=0.9.0-alpha.1" vs Cargo.toml "0.9.0"
- `arc-swap`: AGENTS.md "1.8.2" vs Cargo.toml "1.9.2"
- `scc`: AGENTS.md "3.6.5" vs Cargo.toml "3.8.4"
- `winnow`: AGENTS.md "0.7.14" vs Cargo.toml "1.0.3" (major version!)

#### Approach

1. Update AGENTS.md lines 33-52 to match Cargo.toml actual versions.
2. (Optional) Add a `just check-agents-md` recipe that greps both files and diffs.

---

### Finding 7: Deprecated config groups — 467 lines removable (DEBT-14)

- **Category:** tech debt
- **Impact:** LOW — maintenance burden, public API surface bloat
- **Effort:** S

#### Requirements (EARS Notation)

- **[REQ-CONFIG-01]:** `TransportConfigOptions`, `PoolConfigOptions`, `TlsConfigOptions`, `ProtocolConfigOptions`, `ProxyConfigOptions` SHALL be removed from `crates/hpx/src/client/http/config_groups.rs`.
- **[REQ-CONFIG-02]:** The `#![allow(deprecated)]` annotation on `builder.rs:1` SHALL be removed.
- **[REQ-CONFIG-03]:** All five re-exports in `lib.rs` SHALL be removed.
- **[REQ-CONFIG-04]:** All tests referencing deprecated config groups SHALL be updated to use `ClientBuilder` methods.

#### Current state

- `crates/hpx/src/client/http/config_groups.rs` — 467 lines, all marked `#[deprecated(since = "2.5.0")]`.
- `crates/hpx/src/client/http/builder.rs:1` — `#![allow(deprecated)]`.
- `crates/hpx/src/lib.rs:375-379` — `#[allow(deprecated)]` re-exports.

#### Approach

1. Delete `config_groups.rs`.
2. Remove the five re-exports from `lib.rs`.
3. Remove `#![allow(deprecated)]` from `builder.rs:1`.
4. Update any integration tests that instantiate these types directly (4 test sites identified in `tests/http.rs`).

---

### Finding 8: Cookie module zero unit tests (TEST-05)

- **Category:** test coverage
- **Impact:** HIGH — untested critical path (all cookie-enabled HTTP requests)
- **Effort:** S

See `features/testing.feature` for BDD scenarios.

---

### Finding 9: Retry module zero unit tests (TEST-06)

- **Category:** test coverage
- **Impact:** HIGH — retry logic correctness for API reliability
- **Effort:** S

---

### Finding 10: Redirect module zero unit tests (TEST-07)

- **Category:** test coverage
- **Impact:** HIGH — security-sensitive header stripping logic
- **Effort:** S

---

### Finding 11: Proxy module zero unit tests (TEST-08)

- **Category:** test coverage
- **Impact:** MEDIUM — proxy configuration logic
- **Effort:** S

---

## Architecture Decisions

### AD-01: SSRF check at HttpClient level

- **Context:** The `hpx-browser` HttpClient is shared by fetch, scrape, and CDP server. Adding the check at execute_single_request covers all entry points.
- **Decision:** Call `ssrf::is_forbidden_host()` in `HttpClient::execute_single_request()` before the HTTP request.
- **Consequences:** Single integration point. New methods or code paths that call execute_single_request get SSRF protection automatically.

### AD-02: Keylog gated by Cargo feature

- **Context:** TLS key logging is a debugging tool. In production HFT deployments it is a security risk.
- **Decision:** Gate behind `keylog` feature flag (not default), with cfg stubs.
- **Consequences:** Development/debugging requires `--features keylog`. Production builds are safe by default.

### AD-03: Path validation before directory creation

- **Context:** Download destination paths come from user input (CLI `-o` flag or URL path segment).
- **Decision:** Reject `..` components and absolute paths by default. Add optional `download_root` config.
- **Consequences:** May break users who intentionally use absolute paths. Mitigated by config option.

## BDD/TDD Strategy

- **Primary Language:** Rust 2024
- **BDD Runner:** cucumber-rs (`cargo test -p hpx-dl --test cucumber`)
- **Unit Test Command:** `cargo nextest run --workspace`
- **Feature Files:** `specs/2026-07-10-01-securitydx-quality/features/*.feature`
- **Outside-in Loop:** Write Gherkin scenarios → write failing BDD steps → implement → green

## Code Simplification Constraints

**Ponytail Ladder (mandatory at every decision point):**

1. Does this need to exist at all? Speculative need = skip it. (YAGNI)
2. Stdlib does it? Use it.
3. Native platform feature covers it? Use it.
4. Already-installed dependency? Use it.
5. One line? One line.
6. Only then: minimum code that works.

**Never simplify away:** input validation, error handling, security, accessibility, anything explicitly requested.

**Behavioral Contract:** Preserve existing behavior unless a listed scenario or requirement explicitly changes it.

**Repo Standards:** `thiserror` for library errors, `scc`/`arc-swap` for concurrency, clippy pedantic+nursery enabled.

## BDD Scenario Inventory

- `features/security.feature` — SSRF blocks private IPs by default: Exploitable SSRF vector closed → Tasks 1.1-1.4
- `features/security.feature` — SSLKEYLOGFILE ignored without keylog feature: TLS keys not leaked in prod → Tasks 2.1-2.2
- `features/security.feature` — Download rejects parent directory traversal: Arbitrary file write blocked → Tasks 3.1-3.2
- `features/performance.feature` — Checksum reuses pre-allocated buffer: Per-call allocation eliminated → Tasks 4.1-4.2
- `features/dependencies.feature` — Cargo.lock committed for reproducible builds: Lockfile present → Tasks 5.1
- `features/dependencies.feature` — AGENTS.md versions match Cargo.toml: Version drift corrected → Tasks 6.1-6.2
- `features/debt.feature` — Deprecated config groups removed: 467 lines deleted → Tasks 7.1-7.3
- `features/testing.feature` — Cookie jar unit tests: Critical path covered → Tasks 8.1-8.3
- `features/testing.feature` — Retry policy unit tests: Logic verified → Tasks 9.1-9.2
- `features/testing.feature` — Redirect unit tests: Header stripping verified → Tasks 10.1-10.2
- `features/testing.feature` — Proxy unit tests: Pattern matching verified → Tasks 11.1-11.2

## Existing Components to Reuse

- `crates/hpx-browser/src/net/ssrf.rs` — SSRF module with 14 passing tests (reuse as-is)
- `crates/hpx/src/tls/keylog/` — Key log handle infrastructure (gate with feature flag)
- `crates/hpx/src/client/http/builder.rs` — ClientBuilder methods (replace config groups)
- `crates/hpx/tests/` — Existing integration test patterns to follow

## Verification

| Purpose   | Command                          | Expected on success |
|-----------|----------------------------------|---------------------|
| Build     | `cargo check --workspace`        | exit 0              |
| Lint      | `just lint`                      | exit 0, no errors   |
| Tests     | `cargo nextest run --workspace`  | all pass            |
| BDD       | `cargo test -p hpx-dl --test cucumber` | all pass       |
