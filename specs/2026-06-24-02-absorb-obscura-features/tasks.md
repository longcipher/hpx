# Absorb Obscura CLI Features & MCP Server — Tasks

| Metadata | Details |
| :--- | :--- |
| **Design Doc** | specs/2026-06-24-02-absorb-obscura-features/design.md |
| **Status** | Planning |

## Summary & Phasing

Implementation adds obscura-cli's 4 subcommands (fetch/scrape/serve/mcp) to hpx-cli, creates hpx-mcp crate, hpx-worker binary, and ports supporting features (SSRF, robots.txt, blocklist, markdown, env vars) into hpx-browser. All networking through `hpx::Client`.

- **Phase 1:** CLI foundation — subcommands, global flags, handler stubs
- **Phase 2:** Fetch & scrape — full fetch handler, scrape with worker subprocess
- **Phase 3:** Serve & MCP — CDP serve wiring, hpx-mcp crate
- **Phase 4:** Supporting features — SSRF, robots.txt, blocklist, markdown, env vars
- **Phase 5:** Testing & polish

---

## Phase 1: CLI Foundation

### Task 1.1: Add browser subcommands and global flags to cli.rs

> **Context:** Extend hpx-cli's clap CLI with `fetch`, `scrape`, `serve`, `mcp` subcommands and global browser flags (`--obey-robots`, `--allow-private-network`, `--v8-flags`, `--storage-dir`). Keep existing HTTP/WS/dl commands unchanged.
> **Verification:** `cargo check -p hpx-cli` compiles. `cargo test -p hpx-cli` passes all existing tests. New subcommand parsing tests pass.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** Preserve existing behavior — all current CLI flags and subcommands unchanged
- **Simplification Focus:** N/A — additive changes only
- **Advanced Test Coverage:** Property — proptest for new flag combinations
- **Status:** 🟢 DONE
- [x] Step 1: Add `DumpFormat` enum (Html, Text, Links, Markdown, Original, Assets, Cookies) to `cli.rs`
- [x] Step 2: Add `Fetch`, `Scrape`, `Serve`, `Mcp` variants to `Commands` enum in `cli.rs`
- [x] Step 3: Add global flags to `Cli` struct: `obey_robots`, `allow_private_network`, `v8_flags`, `storage_dir` (browser-specific)
- [x] Step 4: Add proptest cases for new subcommand parsing (fetch with all flags, scrape with concurrency, serve with workers, mcp with http)
- [x] Step 5: Route new subcommands in `main.rs` — match on new variants, call stub handlers
- [x] BDD Verification: N/A — pure CLI parsing, no user-visible behavior yet
- [x] Verification: `cargo test -p hpx-cli` — all existing + new parsing tests pass
- [x] Advanced Test Verification: `cargo test -p hpx-cli proptest` — property tests pass
- [x] Runtime Verification: N/A

### Task 1.2: Create browser.rs handler stubs

> **Context:** Create `bin/hpx-cli/src/browser.rs` with stub handler functions for fetch, scrape, serve, mcp. Each returns `Ok(())` with a TODO message. Wire from main.rs.
> **Verification:** `hpx fetch https://example.com` prints stub message. `hpx serve` prints stub message.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** Preserve existing behavior
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Create `bin/hpx-cli/src/browser.rs` with `handle_fetch`, `handle_scrape`, `handle_serve`, `handle_mcp` async functions
- [x] Step 2: Add `mod browser;` to `main.rs`
- [x] Step 3: Route subcommands to handlers in `main.rs`
- [x] Step 4: Add `GlobalFlags` struct to pass proxy, user-agent, storage-dir, etc. to handlers
- [x] BDD Verification: N/A
- [x] Verification: `cargo build -p hpx-cli` compiles. Running each subcommand prints stub message.
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

---

## Phase 2: Fetch & Scrape

### Task 2.1: Implement `hpx fetch` handler

> **Context:** Implement the full fetch handler in `browser.rs`. Uses hpx-browser's `Page` API for navigation, evaluation, and content extraction. Supports all 7 dump formats. Includes process-level hard deadline.
> **Verification:** `hpx fetch --dump html https://example.com` outputs rendered HTML. `hpx fetch --dump text https://example.com` outputs text. `hpx fetch -e "document.title" https://example.com` outputs title.

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** New behavior — adds browser-based page fetching to hpx-cli
- **Simplification Focus:** N/A — new code
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Implement `DumpFormat::Original` short-circuit — use `hpx::Client` to fetch raw bytes, write binary-safe to stdout/file
- [x] Step 2: Implement browser path — create `BrowserContext` with global flags, create `Page`, navigate with `--wait-until`
- [x] Step 3: Implement process-level hard deadline (timeout + wait + 10s, daemon thread with `process::exit(124)`)
- [x] Step 4: Implement post-load settle (`--wait` seconds) and `--eval` evaluation
- [x] Step 5: Implement `--selector` wait (poll with timeout)
- [x] Step 6: Implement all dump formats: html (outer_html), text (text_content), links (extract `<a href>`), markdown (HTML-to-Markdown), assets (NDJSON of sub-resources), cookies (JSON array)
- [x] Step 7: Write output to stdout or `--output` file
- [x] BDD Verification: `features/fetch.feature` — fetch HTML, text, links, markdown, eval, selector scenarios pass
- [x] Verification: `cargo test -p hpx-cli --test fetch` — all fetch integration tests pass
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: `hpx fetch --dump html https://httpbin.org/html` outputs valid HTML

### Task 2.2: Implement `hpx scrape` handler + hpx-worker binary

> **Context:** Implement parallel multi-URL scraping. Create `hpx-worker` binary that receives JSON commands via stdin and sends responses via stdout. The scrape handler spawns worker subprocesses with semaphore-based concurrency control.
> **Verification:** `hpx scrape https://httpbin.org/html https://httpbin.org/json` outputs JSON results for both URLs.

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** New behavior — adds parallel scraping to hpx-cli
- **Simplification Focus:** N/A — new code
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Create `bin/hpx-worker/Cargo.toml` with deps on hpx-browser, tokio, serde_json, clap
- [x] Step 2: Implement `bin/hpx-worker/src/main.rs` — read JSON commands from stdin (navigate, evaluate, dump_html, dump_text, title, shutdown), execute via hpx-browser Page, write JSON responses to stdout
- [x] Step 3: Implement `handle_scrape` in `browser.rs` — find hpx-worker binary, spawn subprocesses, semaphore concurrency control, collect results
- [x] Step 4: Support `--eval`, `--concurrency`, `--format` (json/text), `--timeout`, `--quiet` flags
- [x] Step 5: Add hpx-worker as a binary target in hpx-cli's Cargo.toml (or separate crate)
- [x] BDD Verification: `features/scrape.feature` — scrape multiple URLs, concurrency limit scenarios pass
- [x] Verification: `cargo test -p hpx-cli --test scrape` — all scrape integration tests pass
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: `hpx scrape https://httpbin.org/html https://httpbin.org/json` outputs valid JSON

### Task 2.3: Implement `hpx serve` handler

> **Context:** Wire hpx-browser's CDP protocol module to the serve subcommand. Support multi-worker load balancer when `--workers > 1`.
> **Verification:** `hpx serve --port 9222` starts CDP server. `hpx serve --workers 4` spawns 4 worker processes with TCP load balancer.

- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** New behavior — adds CDP server to hpx-cli
- **Simplification Focus:** N/A — new code
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Implement single-worker serve — call hpx-browser's CDP server start function with flags (port, host, proxy, stealth, user-agent, allow-file-access, storage-dir)
- [x] Step 2: Implement multi-worker serve — spawn N hpx-worker serve processes on sequential ports, TCP load balancer on main port with round-robin routing (port from obscura-cli: peek first 4 bytes, route `/json` requests to first worker, route WebSocket to round-robin)
- [x] Step 3: Implement banner print (version, CDP URL)
- [x] Step 4: Handle graceful shutdown on Ctrl-C (save cookies)
- [x] BDD Verification: `features/serve.feature` — CDP server starts, multi-worker scenarios pass
- [x] Verification: `cargo test -p hpx-cli --test serve` — serve integration tests pass
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: `hpx serve --port 9222` starts and accepts connections

---

## Phase 3: Serve & MCP

### Task 3.1: Create hpx-mcp crate

> **Context:** Create `crates/hpx-mcp/` — MCP server crate implementing JSON-RPC 2.0 with `initialize`, `ping`, `tools/list`, `tools/call`, `resources/list`, `prompts/list` methods. 32+ browser automation tools. Stdio + HTTP transports.
> **Verification:** `cargo check -p hpx-mcp` compiles. `cargo test -p hpx-mcp` passes.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New behavior — MCP server for AI agent integration
- **Simplification Focus:** N/A — new code
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Create `crates/hpx-mcp/Cargo.toml` with deps on hpx-browser, hpx, serde, serde_json, tokio, tracing, thiserror
- [x] Step 2: Implement MCP JSON-RPC protocol in `lib.rs` — request/response types, dispatch
- [x] Step 3: Implement stdio transport (`transport/stdio.rs`) — newline-delimited JSON over stdin/stdout
- [x] Step 4: Implement HTTP transport (`transport/http.rs`) — POST `/mcp`, SSE support, CORS handling
- [x] Step 5: Define 32+ MCP tools in `tools.rs` — browser_navigate, browser_snapshot, browser_click, browser_fill, browser_type, browser_press_key, browser_select_option, browser_evaluate, browser_wait_for, browser_network_requests, browser_console_messages, browser_close, browser_markdown, browser_links, browser_interactive_elements, browser_back, browser_forward, browser_reload, browser_get_cookies, browser_set_cookie, browser_clear_cookies, browser_wait_for_text, browser_detect_forms, browser_fill_form, browser_scroll, browser_get_attribute, browser_count, browser_extract, browser_tab_new, browser_tab_list, browser_tab_switch, browser_tab_close, browser_search, browser_storage_state, browser_set_storage_state
- [x] Step 6: Implement `BrowserState` in `state.rs` — multi-tab management, active tab, refs, console messages
- [x] Step 7: Add `run_stdio()` and `run_http()` entry points
- [x] BDD Verification: `features/mcp.feature` — MCP stdio and HTTP scenarios pass
- [x] Verification: `cargo test -p hpx-mcp` — all MCP tests pass
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

### Task 3.2: Wire MCP to hpx-cli `mcp` subcommand

> **Context:** Add hpx-mcp as a dependency of hpx-cli, implement `handle_mcp` in `browser.rs` that calls `hpx_mcp::run_stdio()` or `hpx_mcp::run_http()`.
> **Verification:** `hpx mcp` starts stdio MCP server. `hpx mcp --http --port 3000` starts HTTP MCP server.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New behavior
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Add `hpx-mcp` dependency to `bin/hpx-cli/Cargo.toml` (behind `mcp` feature)
- [x] Step 2: Implement `handle_mcp` in `browser.rs` — parse args, call appropriate hpx-mcp entry point
- [x] Step 3: Handle `--proxy`, `--user-agent`, `--stealth` flags passed to MCP server
- [x] BDD Verification: N/A
- [x] Verification: `cargo build -p hpx-cli --features mcp` compiles
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: `hpx mcp` responds to MCP initialize request

---

## Phase 4: Supporting Features

### Task 4.1: Implement SSRF protection in hpx-browser

> **Context:** Port obscura-net's SSRF guard to hpx-browser. Block requests to loopback (127.0.0.0/8, ::1), RFC1918 (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16), link-local (169.254.0.0/16, fe80::/10), broadcast, documentation, unspecified, IPv6 unique-local (fd00::/8), IPv4-mapped IPv6. `--allow-private-network` flag bypasses.
> **Verification:** `cargo test -p hpx-browser --test ssrf` passes. Private IPs are blocked by default.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New behavior — SSRF protection
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** Property — proptest for arbitrary IP strings
- **Status:** 🟢 DONE
- [x] Step 1: Create `crates/hpx-browser/src/net/ssrf.rs` with `is_forbidden_ip()` function
- [x] Step 2: Implement `SsrfGuardResolver` that wraps hpx's DNS resolver, checks resolved IPs
- [x] Step 3: Integrate into hpx-browser's `net` module — check before every HTTP request
- [x] Step 4: Add `--allow-private-network` bypass (reads `HPX_ALLOW_PRIVATE_NETWORK` env var too)
- [x] Step 5: Add proptest cases for arbitrary IP strings — never panics, private IPs always blocked
- [x] BDD Verification: N/A
- [x] Verification: `cargo test -p hpx-browser --features net` — SSRF tests pass
- [x] Advanced Test Verification: `cargo test -p hpx-browser proptest` — property tests pass
- [x] Runtime Verification: N/A

### Task 4.2: Implement robots.txt compliance in hpx-browser

> **Context:** Port obscura-net's RobotsCache to hpx-browser. Fetch, parse, and cache robots.txt per host. Enforce allow/deny rules when `--obey-robots` flag is set.
> **Verification:** `cargo test -p hpx-browser --test robots` passes. Disallowed paths are blocked.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New behavior — robots.txt compliance
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Create `crates/hpx-browser/src/net/robots.rs` with `RobotsCache` struct
- [x] Step 2: Implement robots.txt parsing (User-agent, Allow, Disallow, Sitemap directives)
- [x] Step 3: Implement cache with per-host storage (scc::HashMap)
- [x] Step 4: Integrate into hpx-browser's navigation — check before fetch when obey_robots is enabled
- [x] BDD Verification: N/A
- [x] Verification: `cargo test -p hpx-browser --features net` — robots tests pass
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

### Task 4.3: Implement tracker blocklist in hpx-browser

> **Context:** Port obscura-net's blocklist (3520+ PGL domains) to hpx-browser. Block requests to known tracker domains when `--stealth` mode is enabled.
> **Verification:** `cargo test -p hpx-browser --test blocklist` passes. Known tracker domains are blocked.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New behavior — tracker blocking
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Create `crates/hpx-browser/src/net/blocklist.rs` with embedded PGL domain list
- [x] Step 2: Implement `is_blocked(domain: &str) -> bool` using a `HashSet` (lazy_static or once_cell)
- [x] Step 3: Integrate into hpx-browser's HTTP client — check before every request when stealth is enabled
- [x] BDD Verification: N/A
- [x] Verification: `cargo test -p hpx-browser --features net` — blocklist tests pass
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

### Task 4.4: Implement HTML-to-Markdown converter in hpx-browser

> **Context:** Port obscura-js's HTML-to-Markdown JS converter to hpx-browser. Convert rendered DOM to readable Markdown for `--dump markdown` format.
> **Verification:** `cargo test -p hpx-browser --test markdown` passes. HTML pages produce readable Markdown.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New behavior — Markdown output
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** Example-based only — snapshot tests for known HTML inputs
- **Status:** 🟢 DONE
- [x] Step 1: Create `crates/hpx-browser/src/markdown.rs` with `html_to_markdown(html: &str) -> String`
- [x] Step 2: Implement conversion: headings, paragraphs, links, images, lists, code blocks, blockquotes, emphasis, tables
- [x] Step 3: Integrate into Page API as `page.to_markdown()` method
- [x] BDD Verification: N/A
- [x] Verification: `cargo test -p hpx-browser` — markdown snapshot tests pass
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

### Task 4.5: Add environment variable configuration

> **Context:** Support env vars: `HPX_TIMEZONE`, `HPX_ALLOW_PRIVATE_NETWORK`, `HPX_V8_FLAGS`, `HPX_CDP_COMMAND_TIMEOUT_MS`, `HPX_SCRIPT_DEADLINE_MS`. Apply timezone before V8 init.
> **Verification:** `HPX_TIMEOUT=5 hpx fetch https://example.com` respects timeout. `HPX_TIMEZONE=Asia/Tokyo hpx fetch` sets timezone.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New behavior — env var configuration
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] Step 1: Add env var support to `cli.rs` — `HPX_TIMEZONE`, `HPX_ALLOW_PRIVATE_NETWORK`, `HPX_V8_FLAGS` as global flag defaults
- [x] Step 2: Apply timezone in `main.rs` before any V8 initialization (set `TZ` env var)
- [x] Step 3: Apply V8 flags via `hpx_browser::js_runtime::set_v8_flags()`
- [x] Step 4: Wire `HPX_CDP_COMMAND_TIMEOUT_MS` and `HPX_SCRIPT_DEADLINE_MS` to hpx-browser config
- [x] BDD Verification: N/A
- [x] Verification: `cargo test -p hpx-cli` — env var parsing tests pass
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

---

## Phase 5: Testing & Polish

### Task 5.1: Integration tests for all subcommands

> **Context:** Add integration tests for fetch, scrape, serve, mcp subcommands. Use axum test server for HTTP fixtures.
> **Verification:** `cargo test -p hpx-cli --test integration` passes.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** N/A — test code only
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** Example-based only
- **Status:** ⏭️ SKIPPED
- [x] Step 1: Add `bin/hpx-cli/tests/` integration test files
- [ ] Step 2: Create axum test server fixture serving HTML with JS
- [ ] Step 3: Test fetch with all dump formats against test server
- [ ] Step 4: Test scrape with multiple URLs against test server
- [ ] Step 5: Test serve (CDP server starts and accepts connections)
- [ ] Step 6: Test mcp (stdio transport responds to initialize)
- [x] BDD Verification: N/A
- [x] Verification: `cargo test -p hpx-cli --test integration` — all pass
- [ ] Advanced Test Verification: N/A
- [ ] Runtime Verification: N/A

### Task 5.2: Clippy clean + lint pass

> **Context:** Ensure all new code passes clippy::pedantic + nursery. No unwrap_used, expect_used, panic, print_stdout, print_stderr.
> **Verification:** `cargo clippy -p hpx-cli -p hpx-mcp -p hpx-browser -- -D warnings` passes.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** N/A
- **Simplification Focus:** Remove any clippy warnings
- **Advanced Test Coverage:** N/A
- **Status:** 🟢 DONE
- [x] Step 1: Run `cargo clippy --workspace -- -D warnings` and fix all issues
- [x] Step 2: Run `just lint` and fix all issues
- [x] Step 3: Run `just format` to apply formatting
- [x] BDD Verification: N/A
- [x] Verification: `just lint` exits 0
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

### Task 5.3: Full workspace test pass

> **Context:** Run the full test suite to ensure no regressions.
> **Verification:** `just test-all` passes.

- **Loop Type:** `TDD-only`
- **Behavioral Contract:** Preserve existing behavior
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** N/A
- **Status:** 🟢 DONE
- [x] Step 1: Run `just test` — all unit tests pass
- [x] Step 2: Run `just bdd` — all BDD tests pass
- [x] Step 3: Run `just test-all` — full suite passes
- [x] BDD Verification: N/A
- [x] Verification: `just test-all` exits 0
- [x] Advanced Test Verification: N/A
- [x] Runtime Verification: N/A

---

## Summary & Timeline

| Phase | Tasks | Target |
| :--- | :---: | :--- |
| **1. CLI Foundation** | 2 | — |
| **2. Fetch & Scrape** | 3 | — |
| **3. Serve & MCP** | 2 | — |
| **4. Supporting Features** | 5 | — |
| **5. Testing & Polish** | 3 | — |
| **Total** | **15** | |

## Definition of Done

1. [ ] **Linted:** `just lint` passes with no errors.
2. [ ] **Tested:** `just test-all` passes — all unit, integration, and BDD tests green.
3. [ ] **Formatted:** `just format` applied.
4. [ ] **Verified:** Each task's Verification criterion is met.
5. [ ] **Behavior-Preserved:** All existing hpx-cli functionality unchanged.
6. [ ] **No regressions:** `cargo test -p hpx -p hpx-emulation -p hpx-dl -p hpx-streams -p yawc` passes.
