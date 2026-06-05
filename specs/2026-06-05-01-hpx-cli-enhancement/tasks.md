# hpx-cli Enhancement — Tasks

| Metadata | Details |
| :--- | :--- |
| **Design Doc** | specs/2026-06-05-01-hpx-cli-enhancement/design.md |
| **Status** | Planning |

## Summary & Timeline

| Phase | Description | Tasks | Estimated Scope |
|-------|-------------|-------|-----------------|
| Phase 1 | Download CLI options & progress | Task 1.1-1.9 | ~3 days |
| Phase 2 | HTTP/WebSocket improvements | Task 2.1-2.5 | ~2 days |
| Phase 3 | Testing & polish | Task 3.1-3.2 | ~2 days |

## Tasks

### Phase 1: Download CLI Options & Progress

### Task 1.1: Real-time download progress display

> **Context:** The `dl add` command currently just prints "Added download {id}" and exits. Subscribe to `DownloadEngine::subscribe()` and render progress to stderr during download execution.
> **Verification:** `hpx dl add <url>` shows a progress line updating in real-time.
> **Requirement Coverage:** R1
> **Scenario Coverage:** dl-progress-display

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** New behavior — previously dl add was fire-and-forget
- **Simplification Focus:** Extract progress rendering into `output.rs` helper
- **Advanced Test Coverage:** Example-based only
- **Status:** 🔴 TODO
- [ ] Step 1: Add `indicatif` dependency via `cargo add -p hpx-cli --workspace`
- [ ] Step 2: Create `src/progress.rs` with `ProgressDisplay` struct wrapping `indicatif::ProgressBar`
- [ ] Step 3: In `main.rs::handle_dl_command`, after `engine.add()`, spawn event listener task that subscribes to `engine.subscribe()` and updates progress bar
- [ ] Step 4: Handle Completed/Failed events to finish progress bar
- [ ] Step 5: Disable progress bar when not a terminal (check `output::is_terminal()`)
- [ ] Step 6: Write unit test for progress event parsing
- [ ] Verification: `cargo run -p hpx-cli -- dl add https://httpbin.org/bytes/1048576` shows progress
- [ ] Verification: `cargo nextest run -p hpx-cli` passes

### Task 1.2: Expose --speed-limit for downloads

> **Context:** `DownloadRequest` already supports `speed_limit: Option<u64>`. Add a CLI flag and parse human-readable speed strings (e.g. "1MB/s", "500KB/s").
> **Verification:** `hpx dl add <url> --speed-limit 1MB/s` downloads at approximately 1MB/s.
> **Requirement Coverage:** R2
> **Scenario Coverage:** dl-speed-limit

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** Additive — new flag, no existing behavior changed
- **Simplification Focus:** Parse speed string in a dedicated helper function
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Add `--speed-limit` flag to `DlCommands::Add` in `cli.rs` (type: `Option<String>`)
- [x] Step 2: Implement `parse_speed_limit(s: &str) -> Result<u64>` helper — parse "1MB/s", "500KB/s", "1024" (raw bytes)
- [x] Step 3: In `main.rs::handle_dl_command`, apply parsed speed limit to `DownloadRequest::builder().speed_limit()`
- [x] Step 4: Write unit tests for `parse_speed_limit` (valid inputs, edge cases, invalid inputs)
- [x] Step 5: Write property test: round-trip parse_speed_limit formatted output
- [x] Verification: `cargo run -p hpx-cli -- dl add https://httpbin.org/bytes/10485760 --speed-limit 1MB/s` throttles
- [x] Verification: `cargo nextest run -p hpx-cli` passes

### Task 1.3: Expose --checksum for downloads

> **Context:** `DownloadRequest` supports `checksum: Option<ChecksumSpec>`. Add CLI flag for integrity verification.
> **Verification:** `hpx dl add <url> --checksum sha256:<hash>` verifies after download; mismatch fails with error.
> **Requirement Coverage:** R3
> **Scenario Coverage:** dl-checksum

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** Additive
- **Simplification Focus:** Parse "algorithm:hex_value" format in dedicated helper
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Add `--checksum` flag to `DlCommands::Add` in `cli.rs` (type: `Option<String>`)
- [x] Step 2: Implement `parse_checksum(s: &str) -> Result<ChecksumSpec>` — parse "sha256:abc123", "md5:abc", "sha1:abc"
- [x] Step 3: Apply parsed checksum to `DownloadRequest::builder().checksum()`
- [x] Step 4: Write unit tests for `parse_checksum`
- [x] Verification: `cargo run -p hpx-cli -- dl add <url> --checksum sha256:<correct_hash>` succeeds
- [x] Verification: `cargo nextest run -p hpx-cli` passes

### Task 1.4: Expose --mirror for downloads

> **Context:** `DownloadRequest` supports `mirrors: Vec<String>`. Add CLI flag for mirror URLs.
> **Verification:** `hpx dl add <url> --mirror <mirror1> --mirror <mirror2>` tries mirrors on failure.
> **Requirement Coverage:** R4
> **Scenario Coverage:** dl-mirror

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** Additive
- **Simplification Focus:** Simple Vec<String> passthrough
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Add `--mirror` flag to `DlCommands::Add` in `cli.rs` (type: `Vec<String>`, action: Append)
- [x] Step 2: Apply mirrors to `DownloadRequest::builder().mirrors()`
- [x] Step 3: Write unit test for mirror flag parsing
- [x] Verification: `cargo nextest run -p hpx-cli` passes

### Task 1.5: Expose --max-connections for downloads

> **Context:** `DownloadRequest` supports `max_connections: Option<usize>`. Add CLI flag.
> **Verification:** `hpx dl add <url> --max-connections 8` uses 8 connections.
> **Requirement Coverage:** R5
> **Scenario Coverage:** dl-max-connections

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** Additive
- **Simplification Focus:** Direct passthrough
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Add `--max-connections` flag to `DlCommands::Add` in `cli.rs` (type: `Option<usize>`)
- [x] Step 2: Apply to `DownloadRequest::builder().max_connections()`
- [x] Step 3: Write unit test
- [x] Verification: `cargo nextest run -p hpx-cli` passes

### Task 1.6: Expose --header for downloads

> **Context:** `DownloadRequest` supports `headers: HashMap<String, String>`. The `DlCommands::Add` subcommand currently has no header support.
> **Verification:** `hpx dl add <url> -H "Authorization:Bearer token"` sends the header.
> **Requirement Coverage:** R6
> **Scenario Coverage:** dl-headers

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** Additive
- **Simplification Focus:** Reuse `parsed_headers()` pattern from main Cli
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Add `-H`/`--header` flag to `DlCommands::Add` in `cli.rs`
- [x] Step 2: Parse headers using same logic as `Cli::parsed_headers()`
- [x] Step 3: Apply headers to `DownloadRequest::builder().header()`
- [x] Step 4: Write unit test
- [x] Verification: `cargo nextest run -p hpx-cli` passes

### Task 1.7: Expose --proxy for downloads

> **Context:** `DownloadRequest` supports `proxy: Option<ProxyConfig>`. The `DlCommands::Add` subcommand has no proxy support.
> **Verification:** `hpx dl add <url> --proxy http://proxy:8080` downloads through proxy.
> **Requirement Coverage:** R7
> **Scenario Coverage:** dl-proxy

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** Additive
- **Simplification Focus:** Reuse `ProxyConfig` from hpx-dl types
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Add `--proxy` flag to `DlCommands::Add` in `cli.rs`
- [x] Step 2: Implement `parse_proxy_config(url: &str) -> Result<ProxyConfig>` — detect protocol from URL scheme
- [x] Step 3: Apply proxy to `DownloadRequest::builder().proxy()`
- [x] Step 4: Write unit test
- [x] Verification: `cargo nextest run -p hpx-cli` passes

### Task 1.8: Expose --retry for downloads

> **Context:** `EngineConfig` has `retry_max_attempts` but it's global. `DownloadRequest` doesn't have per-download retry. For now, expose global retry config via CLI flag.
> **Verification:** `hpx dl add <url> --retry 5` sets retry attempts to 5.
> **Requirement Coverage:** R8
> **Scenario Coverage:** dl-retry

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** Additive
- **Simplification Focus:** Store retry value, pass to engine builder
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Add `--retry` flag to `DlCommands::Add` (for future per-download use) and as global flag on `Cli`
- [x] Step 2: In `main.rs::handle_dl_command`, configure engine with retry settings
- [x] Step 3: Write unit test
- [x] Verification: `cargo nextest run -p hpx-cli` passes

### Task 1.9: Expose engine configuration flags

> **Context:** `EngineConfig` has `storage_path`, `max_concurrent_downloads`, `max_connections_per_download`. Expose as global CLI flags.
> **Verification:** `hpx --storage-path /tmp/hpx.db dl add ...` uses custom storage path.
> **Requirement Coverage:** R9
> **Scenario Coverage:** dl-engine-config

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** Additive
- **Simplification Focus:** Global flags on Cli struct, applied to engine builder
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Add `--storage-path` and `--max-concurrent` flags to `Cli` struct
- [x] Step 2: Pass values to `DownloadEngine::builder().storage_path().max_concurrent()`
- [x] Step 3: Write unit test
- [x] Verification: `cargo nextest run -p hpx-cli` passes

### Phase 2: HTTP/WebSocket Improvements

### Task 2.1: Complete timing waterfall with all phases

> **Context:** Current `TimingWaterfall` only tracks DNS start → body done. Add DNS resolution, TCP connect, TLS handshake, TTFB, and transfer phases.
> **Verification:** `hpx -T <url>` shows all timing phases.
> **Requirement Coverage:** R10
> **Scenario Coverage:** timing-waterfall

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** Enhanced — more timing data displayed
- **Simplification Focus:** Extract timing data from hpx response extensions
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Research hpx response extensions for timing data (check `Response::extensions()` types)
- [x] Step 2: Extend `TimingWaterfall` struct with all phase fields
- [x] Step 3: Update `TimingWaterfall::print()` to display all phases
- [x] Step 4: In `http.rs`, populate timing fields from response extensions
- [x] Step 5: Write unit test for timing display
- [x] Verification: `cargo run -p hpx-cli -- -T https://httpbin.org/get` shows DNS/connect/TLS/TTFB/transfer
- [x] Verification: `cargo nextest run -p hpx-cli` passes

### Task 2.2: WebSocket reconnection support

> **Context:** When a WebSocket connection drops, the CLI exits. Add `--reconnect` flag for automatic reconnection with exponential backoff.
> **Verification:** `hpx ws://echo.websocket.org --reconnect` reconnects after connection drop.
> **Requirement Coverage:** R12
> **Scenario Coverage:** ws-reconnect

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** Additive
- **Simplification Focus:** Wrap existing ws::execute in retry loop
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Add `--reconnect` and `--reconnect-max` flags to `Cli` struct
- [x] Step 2: In `ws.rs::execute`, wrap connection in retry loop with exponential backoff
- [x] Step 3: Handle close frames and errors to determine reconnection eligibility
- [x] Step 4: Write unit test for reconnection logic
- [x] Verification: `cargo nextest run -p hpx-cli` passes

### Task 2.3: Environment variable support

> **Context:** Allow `HPX_` prefixed environment variables to set default values for CLI flags.
> **Verification:** `HPX_TIMEOUT=30 hpx httpbin.org/get` uses 30s timeout.
> **Requirement Coverage:** R13
> **Scenario Coverage:** env-config

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** Additive — env vars are fallback, flags take precedence
- **Simplification Focus:** Merge env vars after `Cli::parse()` using clap's `env` attribute
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Add `env = "HPX_TIMEOUT"` etc. to relevant clap `#[arg]` attributes in `cli.rs`
- [x] Step 2: Document supported environment variables in help text
- [x] Step 3: Write unit test for env var parsing
- [x] Verification: `HPX_TIMEOUT=1 cargo run -p hpx-cli -- https://httpbin.org/delay/5` times out at 1s
- [x] Verification: `cargo nextest run -p hpx-cli` passes

### Task 2.4: JSON output mode for dl list/status

> **Context:** When `--format json` is used with `dl list` or `dl status`, output structured JSON instead of human-readable text.
> **Verification:** `hpx dl list --format json` outputs valid JSON array.
> **Requirement Coverage:** R14
> **Scenario Coverage:** json-output

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** Additive — existing text output preserved when format is not json
- **Simplification Focus:** Use serde_json for serialization
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: In `main.rs::handle_dl_command`, check `cli.format` before printing
- [x] Step 2: For `dl list` and `dl status`, serialize `DownloadStatus` to JSON when format is Json
- [x] Step 3: Write unit test for JSON serialization
- [x] Verification: `cargo run -p hpx-cli -- dl list --format json` outputs valid JSON
- [x] Verification: `cargo nextest run -p hpx-cli` passes

### Task 2.5: Form file upload support

> **Context:** `--form KEY=@filename` should read file content and send as multipart form field (like curl).
> **Verification:** `hpx -X POST url --form file=@/path/to/file` uploads file content.
> **Requirement Coverage:** R15
> **Scenario Coverage:** form-file-upload

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** Enhanced — `@` prefix now triggers file read
- **Simplification Focus:** Reuse existing multipart infrastructure in `http.rs`
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: In `Cli::parsed_form_fields()`, detect `@`-prefixed values and read from file
- [x] Step 2: In `http.rs`, when form fields contain file data, use multipart form instead of urlencoded
- [x] Step 3: Write unit test for `@` prefix detection and file reading
- [x] Step 4: Write integration test with mock server
- [x] Verification: `cargo nextest run -p hpx-cli` passes

### Phase 3: Testing & Polish

### Task 3.1: Add unit tests for CLI argument parsing

> **Context:** hpx-cli currently has no tests. Add unit tests for all CLI argument parsing edge cases.
> **Verification:** All unit tests pass.
> **Requirement Coverage:** R11
> **Scenario Coverage:** all

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** N/A (tests only)
- **Simplification Focus:** Test helpers for common parsing patterns
- **Advanced Test Coverage:** Property (proptest for header/cookie/form parsing)
- **Status:** 🟢 DONE
- [x] Step 1: Create `tests/cli_parsing.rs` with unit tests for `Cli::parse()` with various flag combinations
- [x] Step 2: Test `parsed_headers()`, `parsed_cookies()`, `parsed_form_fields()`, `parsed_multipart_fields()` edge cases
- [x] Step 3: Test `parse_speed_limit()`, `parse_checksum()`, `parse_proxy_config()` helpers
- [x] Step 4: Add proptest for header parsing: arbitrary valid header strings parse correctly
- [x] Step 5: Add proptest for cookie parsing: arbitrary `name=value` pairs parse correctly
- [x] Verification: `cargo nextest run -p hpx-cli` passes
- [x] Verification: `cargo +nightly clippy -p hpx-cli -- -D warnings` passes

### Task 3.2: Add integration tests for HTTP and WebSocket

> **Context:** Add integration tests using mock HTTP server to verify end-to-end CLI behavior.
> **Verification:** Integration tests pass.
> **Requirement Coverage:** R11
> **Scenario Coverage:** all

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** N/A (tests only)
- **Simplification Focus:** Use `axum` test server (already in workspace)
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Add `axum` and `tokio-test` dependencies via `cargo add -p hpx-cli --workspace`
- [x] Step 2: Create `tests/http_integration.rs` with mock server tests for GET, POST, headers, auth
- [x] Step 3: Create `tests/ws_integration.rs` with mock WebSocket server test
- [x] Step 4: Test error cases: invalid URL, timeout, connection refused
- [x] Step 5: Test download integration: add download, verify file created
- [x] Verification: `cargo nextest run -p hpx-cli --all-features` passes
- [x] Verification: `just lint` passes

## Definition of Done

- [ ] All tasks completed with status 🟢 DONE
- [ ] `cargo nextest run -p hpx-cli --all-features` passes
- [ ] `cargo +nightly clippy -p hpx-cli -- -D warnings` passes
- [ ] All BDD scenarios pass
- [ ] `just format` applied
- [ ] `just lint` passes
- [ ] `just test` passes
- [ ] `just build-docs` passes
