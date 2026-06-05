# Design: hpx-cli Enhancement

| Metadata | Details |
| :--- | :--- |
| **Status** | Draft |
| **Created** | 2026-06-05 |
| **Scope** | Full |

## Executive Summary

The `hpx-cli` binary currently provides basic HTTP/WebSocket requests and a minimal download management subcommand. However, the underlying `hpx-dl` engine exposes significantly richer functionality (speed limiting, checksum verification, mirror fallback, per-download headers/proxy, event-driven progress) that is not accessible through the CLI. Additionally, the timing waterfall is incomplete, there are no tests, and several ergonomic gaps exist compared to tools like `curl` and `aria2`. This plan addresses these gaps in three phases: exposing existing hpx-dl capabilities, improving the HTTP/WebSocket CLI, and adding missing infrastructure.

## Source Inputs & Normalization

Consumed:

1. Full source audit of `bin/hpx-cli/src/{main.rs, cli.rs, http.rs, ws.rs, output.rs}`
2. Full source audit of `crates/hpx-dl/src/{engine.rs, types.rs, lib.rs}`
3. `AGENTS.md` workspace conventions
4. `README.md` feature documentation

**Ambiguities resolved as assumptions:**

- **A1**: The user asked "analysis还缺少什么功能" — interpreted as a gap analysis between hpx-dl capabilities and hpx-cli exposure, plus general CLI improvements.
- **A2**: "TODO未完成的功能" — the hpx-dl crate has no explicit TODO markers; the plan covers functional gaps rather than code comments.
- **A3**: Scope is limited to hpx-cli improvements; no changes to the `hpx` or `hpx-dl` library crates.

## Requirements & Goals

### Functional Requirements

| ID | Requirement |
|----|-------------|
| R1 | Real-time download progress display (progress bar/percentage) via event subscription |
| R2 | Expose `--speed-limit` for per-download speed limiting |
| R3 | Expose `--checksum` for download integrity verification |
| R4 | Expose `--mirror` for fallback mirror URLs |
| R5 | Expose `--max-connections` for per-download connection count |
| R6 | Expose `--header` for downloads (pass custom headers to download requests) |
| R7 | Expose `--proxy` for downloads (per-download proxy override) |
| R8 | Expose `--retry` configuration for downloads |
| R9 | Expose `--storage-path` and `--max-concurrent` for engine configuration |
| R10 | Complete timing waterfall with DNS/connect/TLS/TTFB/transfer phases |
| R11 | Add tests for hpx-cli (unit tests for CLI parsing, integration tests for HTTP/WS) |
| R12 | WebSocket reconnection support (`--reconnect` flag) |
| R13 | Environment variable support for default options (`HPX_DEFAULT_HEADERS`, etc.) |
| R14 | JSON output mode for programmatic consumption (`--format json` on status/list) |
| R15 | Form file upload support (`--form KEY=@filename`) |

### Non-Functional Requirements

| ID | Requirement |
|----|-------------|
| N1 | Follow AGENTS.md conventions: `hpx` for HTTP, `eyre` for errors, `tracing` for logging |
| N2 | Clippy pedantic compliance |
| N3 | No `unwrap()`/`expect()`/`panic!()` in production code |
| N4 | Maintain backward compatibility with existing CLI flags |

### Out of Scope

| ID | Item | Rationale |
|----|------|-----------|
| O1 | Config file support | Could be added later; current flag-based config is sufficient |
| O2 | TUI progress dashboard | Out of scope; progress display is terminal-based |
| O3 | Metalink CLI support | Metalink is available in hpx-dl but CLI exposure is lower priority |
| O4 | Clipboard support for Windows | Requires platform-specific research |

## Requirements Coverage Matrix

| Req ID | design.md Section | Scenarios | Tasks |
|--------|-------------------|-----------|-------|
| R1 | Detailed Design — Progress Display | dl-progress-display | Task 1.1 |
| R2 | Detailed Design — DL Options | dl-speed-limit | Task 1.2 |
| R3 | Detailed Design — DL Options | dl-checksum | Task 1.3 |
| R4 | Detailed Design — DL Options | dl-mirror | Task 1.4 |
| R5 | Detailed Design — DL Options | dl-max-connections | Task 1.5 |
| R6 | Detailed Design — DL Options | dl-headers | Task 1.6 |
| R7 | Detailed Design — DL Options | dl-proxy | Task 1.7 |
| R8 | Detailed Design — DL Options | dl-retry | Task 1.8 |
| R9 | Detailed Design — Engine Config | dl-engine-config | Task 1.9 |
| R10 | Detailed Design — Timing Waterfall | timing-waterfall | Task 2.1 |
| R11 | Detailed Design — Testing | all | Task 3.1 |
| R12 | Detailed Design — WebSocket | ws-reconnect | Task 2.2 |
| R13 | Detailed Design — Environment | env-config | Task 2.3 |
| R14 | Detailed Design — JSON Output | json-output | Task 2.4 |
| R15 | Detailed Design — Form Upload | form-file-upload | Task 2.5 |

## Architecture Decisions

### Inherited Decisions

1. **HTTP Client**: `hpx::Client` is the sole HTTP backend.
2. **Error Handling**: `eyre` for CLI binary.
3. **Logging**: `tracing` + `tracing_subscriber`.
4. **CLI Framework**: `clap` with derive macros.
5. **Download Engine**: `hpx-dl::DownloadEngine` is the download coordinator.

### New Pattern Selection

| Decision | Pattern | Rationale |
|----------|---------|-----------|
| Progress display | **Observer** | Subscribe to `DownloadEvent` broadcast channel, render progress in CLI loop. Simple, non-blocking. |
| CLI option passthrough | **Adapter** | Map CLI flags to `DownloadRequest` builder methods. Thin mapping layer. |
| Timing phases | **State Machine** | Track request lifecycle phases (DNS → Connect → TLS → TTFB → Transfer) via timestamps. |

### SRP / DIP Check

- **SRP**: `cli.rs` handles argument parsing, `http.rs` handles HTTP execution, `ws.rs` handles WebSocket, `output.rs` handles formatting, `main.rs` orchestrates.
- **DIP**: CLI depends on `hpx::Client` and `hpx_dl::DownloadEngine` abstractions, not concrete implementations.

## BDD/TDD Strategy

- **Primary Language:** Rust
- **BDD Runner:** `cucumber` (existing workspace convention)
- **BDD Command:** `cargo test -p hpx-cli --test cucumber`
- **Unit Test Command:** `cargo nextest run -p hpx-cli`
- **Property Test Tool:** `proptest` (for CLI argument parsing edge cases)
- **Feature Files:** `specs/2026-06-05-01-hpx-cli-enhancement/features/*.feature`

## Code Simplification Constraints

- **Behavioral Contract:** Preserve all existing CLI flags and behavior. New flags are additive.
- **Repo Standards:** Follow AGENTS.md conventions exactly.
- **Readability Priorities:** Extract helper functions for complex flag-to-request mapping. Avoid nested match arms.
- **Refactor Scope:** Limit changes to `bin/hpx-cli/src/` only.
- **Clarity Guardrails:** Use named constants for magic numbers. Document new flags in help text.

## Detailed Design

### 1. Download Progress Display (R1)

Subscribe to `DownloadEngine::subscribe()` and render a progress line to stderr:

```text
[===           ] 23% 1.2/5.0 MB  256 KB/s  ETA 15s
```

When not a terminal, emit one-line status updates per progress event. Use `indicatif` crate (workspace has it available) for terminal progress bars.

### 2. Download CLI Options (R2-R9)

Extend `DlCommands::Add` with new flags:

```rust
DlCommands::Add {
    url: String,
    #[arg(short, long)] output: Option<String>,
    #[arg(short, long, default_value = "normal")] priority: String,
    #[arg(long)] speed_limit: Option<String>,     // e.g. "1MB/s", "500KB/s"
    #[arg(long)] checksum: Option<String>,         // e.g. "sha256:abc123"
    #[arg(long)] mirror: Vec<String>,              // mirror URLs
    #[arg(long)] max_connections: Option<usize>,   // 1-16
    #[arg(short = 'H', long = "header")] headers: Vec<String>,
    #[arg(long)] proxy: Option<String>,
    #[arg(long)] retry: Option<u32>,
}
```

Engine configuration via global flags or environment:

```rust
#[arg(long)] storage_path: Option<String>,
#[arg(long)] max_concurrent: Option<usize>,
```

### 3. Timing Waterfall (R10)

Replace current minimal `TimingWaterfall` with phase-aware tracking:

```rust
struct TimingWaterfall {
    dns_start: Instant,
    dns_done: Option<Instant>,
    connect_done: Option<Instant>,
    tls_done: Option<Instant>,
    ttfb: Option<Instant>,
    body_done: Option<Instant>,
}
```

Extract timing data from hpx response extensions (hpx exposes timing via `Response::extensions()`).

### 4. WebSocket Reconnection (R12)

Add `--reconnect` flag. When connection drops, automatically reconnect with exponential backoff:

```rust
#[arg(long)] reconnect: bool,
#[arg(long, default_value = "3")] reconnect_max: u32,
```

### 5. Environment Variable Support (R13)

Support `HPX_` prefixed environment variables for all flags:

```rust
// In cli.rs, after Cli::parse(), merge with env vars
// e.g., HPX_TIMEOUT=30, HPX_HEADERS="Authorization:Bearer token"
```

Use `std::env::var` with flag precedence over env vars.

### 6. JSON Output (R14)

When `--format json` is used with `dl list` or `dl status`, output structured JSON:

```json
[
  {"id": "uuid", "url": "...", "state": "downloading", "progress": 0.23, "speed": 256000}
]
```

### 7. Form File Upload (R15)

Support `--form KEY=@filename` syntax (like curl):

```rust
// parsed_form_fields() should detect @-prefixed values and read from file
```

### 8. Testing (R11)

- Unit tests for all CLI argument parsing
- Integration tests for HTTP requests against mock server
- Integration tests for WebSocket connections
- Property tests for header/cookie/form parsing

## BDD Scenario Inventory

| Feature File | Scenario | Business Outcome |
|-------------|----------|-----------------|
| dl-progress.feature | Download with progress display | User sees real-time progress bar |
| dl-options.feature | Download with all options | All new flags work correctly |
| timing.feature | Timing waterfall phases | All phases displayed |
| ws-reconnect.feature | WebSocket auto-reconnect | Connection re-established after drop |
| json-output.feature | JSON output format | Programmatic consumers get valid JSON |
| form-upload.feature | Form file upload | File content sent in multipart body |

## Existing Components to Reuse

- `hpx_dl::DownloadEngine` — already supports all download options (speed_limit, checksum, mirrors, max_connections, headers, proxy)
- `hpx_dl::DownloadEvent` — progress events already implemented
- `hpx_dl::DownloadRequest::builder()` — builder pattern already exists
- `hpx::Client` — HTTP client with proxy, timeout, redirect support
- `crate::output::TimingWaterfall` — timing infrastructure exists, needs expansion

## Verification

- `cargo nextest run -p hpx-cli --all-features` passes
- `cargo +nightly clippy -p hpx-cli -- -D warnings` passes
- `cargo build -p hpx-cli` succeeds
- All BDD scenarios pass
- Manual verification: `cargo run -p hpx-cli -- dl add https://example.com/large.bin --speed-limit 1MB/s --checksum sha256:abc123 --progress`
