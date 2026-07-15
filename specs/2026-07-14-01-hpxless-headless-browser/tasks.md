# hpxless — Standalone Headless Browser — Implementation Tasks

| Metadata | Details |
| :--- | :--- |
| **Design Doc** | specs/2026-07-14-01-hpxless-headless-browser/design.md |
| **Owner** | pb-plan agent |
| **Start Date** | 2026-07-14 |
| **Target Date** | 2026-07-28 |
| **Status** | Planning |

## Summary & Phasing

Build `bin/hpxless` — a standalone headless browser binary that reuses `hpx-browser`'s existing DOM/JS/CDP/stealth infrastructure and adds the missing sub-resource loading pipeline. Four phases: binary scaffolding → resource loading → CDP integration → polish.

- **Planner Contract Rule:** Emit a contract-complete, build-eligible spec in the existing markdown artifacts.
- **State Contract Rule:** `🔴 TODO` → `🟡 IN PROGRESS` → `🟢 DONE`; exceptional: `⏭️ SKIPPED`, `🔄 DCR`, `⛔ OBSOLETE`.

---

## Phase 1: BDD Harness & Binary Scaffolding

### Task 1.1: Create `bin/hpxless` crate with CLI

> **Context:** Create the standalone binary crate with clap CLI. Reuses `hpx-browser` (feature flags `v8` + `cdp` + `stealth`) and `hpx` (for networking). The binary starts a CDP WebSocket server that Puppeteer/Playwright can connect to.
> **Verification:** `cargo build -p hpxless` succeeds; `./target/debug/hpxless --help` prints usage.

- **Priority:** P0
- **Scope:** Binary crate scaffolding, CLI definition
- **Requirement Coverage:** `R1`, `R11`
- **Scenario Coverage:** `hpxless-startup.feature`
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New binary — no existing behavior to preserve
- **Simplification Focus:** N/A — new code
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** Create `bin/hpxless/Cargo.toml` with dependencies: `hpx-browser` (features `v8`, `cdp`), `hpx` (features `stealth`), `clap`, `tokio`, `tracing`, `tracing-subscriber`, `eyre`, `ecdysis`
- [x] **Step 2:** Create `bin/hpxless/src/cli.rs` with `Cli` struct (port, stealth, profile, proxy, block, url, log_level)
- [x] **Step 3:** Create `bin/hpxless/src/main.rs` — parse CLI, init tracing, print startup banner
- [x] **Step 4:** Add `hpxless` to workspace `Cargo.toml` members
- [x] **Step 5:** Add `hpxless` to Justfile targets (build, test, lint)
- [x] **BDD Verification:** N/A — infrastructure task
- [x] **Verification:** `cargo build -p hpxless` succeeds; `./target/debug/hpxless --help` shows all flags
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** `./target/debug/hpxless --port 0 2>&1 | head -5` shows startup log

### Task 1.2: CDP server startup in hpxless

> **Context:** Wire `CdpServer::start` into the hpxless binary. The binary should start a CDP WebSocket server on the configured port, create an initial `Page` (from `--url` or `about:blank`), and accept connections.
> **Verification:** Connect to the CDP endpoint via WebSocket, send `Browser.getVersion`, receive a valid response.

- **Priority:** P0
- **Scope:** CDP server integration, page creation
- **Requirement Coverage:** `R1`, `R5`
- **Scenario Coverage:** `hpxless-startup.feature`
- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** New binary — no existing behavior to preserve
- **Simplification Focus:** N/A — new code
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** In `main.rs`, create `Page::from_html` (or `Page::navigate` if `--url` provided)
- [x] **Step 2:** Start `CdpServer` with the page, port, and stealth flag
- [x] **Step 3:** Add graceful shutdown via `ecdysis` — SIGTERM/SIGINT stops the server
- [x] **Step 4:** Write unit test: start hpxless with `--port 0`, verify port is printed
- [x] **BDD Verification:** Write `features/hpxless-startup.feature` with scenario: "hpxless starts and serves CDP"
- [x] **Verification:** `cargo test -p hpxless` passes; manual WebSocket connect to `ws://127.0.0.1:9222` works
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** Start hpxless, connect with `websocat ws://127.0.0.1:9222`, send `{"id":1,"method":"Browser.getVersion"}`, verify JSON response

### Task 1.3: BDD harness setup for hpxless

> **Context:** Set up cucumber-rs test harness for `bin/hpxless`. The `hpx-dl` crate already has a working cucumber setup — follow the same pattern.
> **Verification:** `cargo test -p hpxless --test cucumber` runs (even if scenarios are pending).

- **Priority:** P1
- **Scope:** BDD test infrastructure
- **Requirement Coverage:** N/A — infrastructure
- **Scenario Coverage:** All BDD scenarios
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** N/A — test infrastructure
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** N/A
- **Status:** 🟢 DONE
- [x] **Step 1:** Create `bin/hpxless/tests/cucumber.rs` following `crates/hpx-dl/tests/cucumber.rs` pattern
- [x] **Step 2:** Create `bin/hpxless/features/` directory
- [x] **Step 3:** Move feature files from `specs/2026-07-14-01-hpxless-headless-browser/features/` to `bin/hpxless/features/`
- [x] **Step 4:** Implement step definitions for `hpxless-startup.feature`
- [x] **BDD Verification:** `cargo test -p hpxless --test cucumber` runs, startup scenario passes
- [x] **Verification:** `cargo test -p hpxless --test cucumber` completes without panic
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

---

## Phase 2: Resource Loading Pipeline

### Task 2.1: Resource URL extraction from DOM

> **Context:** After HTML is parsed into the DOM, we need to walk the tree and extract all resource URLs: `<link rel="stylesheet" href>`, `<script src>`, `<img src>`, `<link rel="font">`. This is the discovery phase — no network requests yet.
> **Verification:** Unit test with HTML containing all resource types → correct URLs extracted, block filtering works.

- **Priority:** P0
- **Scope:** DOM walking, URL extraction
- **Requirement Coverage:** `R2`, `R4`, `R13`
- **Scenario Coverage:** `navigation.feature` (sub-resource discovery)
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New functionality — no existing behavior to preserve
- **Simplification Focus:** N/A — new code
- **Advanced Test Coverage:** Property (proptest: various HTML structures → correct URL extraction)
- **Status:** 🟢 DONE
- [x] **Step 1:** Create `crates/hpx-browser/src/resource_loader.rs` with `ResourceLoader` struct
- [x] **Step 2:** Implement `extract_resource_urls(dom: &Dom) -> Vec<(ResourceType, String)>` — walk DOM tree, match element tag names and attributes
- [x] **Step 3:** Implement block filtering: `filter_by_block_types(resources, block_types)`
- [x] **Step 4:** Write unit tests: HTML with `<link>`, `<script src>`, `<img>`, `<style>` → correct extraction
- [x] **Step 5:** Write proptest: random HTML with resource tags → extracted URLs are valid, block filtering is idempotent
- [x] **BDD Verification:** N/A — unit-level
- [x] **Verification:** `cargo test -p hpx-browser resource_loader` passes
- [x] **Advanced Test Verification:** `cargo test -p hpx-browser --features proptest resource_loader` passes
- [x] **Runtime Verification:** N/A

### Task 2.2: Concurrent sub-resource fetching

> **Context:** Given a list of resource URLs, fetch them concurrently using `HttpClient`. Respect block types. Return `Vec<LoadedResource>` with content and metadata. Cap concurrent requests (default 6 per domain, configurable).
> **Verification:** Unit test: mock server serves CSS/JS/images → `ResourceLoader` fetches all concurrently.

- **Priority:** P0
- **Scope:** Network layer, concurrent fetching
- **Requirement Coverage:** `R2`, `R13`
- **Scenario Coverage:** `navigation.feature` (sub-resource loading)
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New functionality — no existing behavior to preserve
- **Simplification Focus:** N/A — new code
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** Implement `ResourceLoader::fetch_all(&self, urls: Vec<(ResourceType, String)>) -> Vec<LoadedResource>` using `tokio::task::JoinSet` or `futures::stream::FuturesUnordered`
- [x] **Step 2:** Respect `block_types` — skip fetching blocked resource types
- [x] **Step 3:** Set reasonable timeouts (5s per resource, 15s total)
- [x] **Step 4:** Write unit test: start local HTTP server, serve test resources, verify concurrent fetch
- [x] **BDD Verification:** N/A — unit-level
- [x] **Verification:** `cargo test -p hpx-browser resource_loader::fetch` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 2.3: CSS stylesheet injection

> **Context:** After fetching external CSS files, inject them into the DOM as `<style>` tags so blitz/stylo can process them. Also handle inline `<style>` tags that are already in the DOM.
> **Verification:** Navigate to page with `<link rel="stylesheet">` → CSS rules are applied (check computed style via JS).

- **Priority:** P0
- **Scope:** CSS processing, DOM mutation
- **Requirement Coverage:** `R4`
- **Scenario Coverage:** `navigation.feature` (CSS application)
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New functionality — no existing behavior to preserve
- **Simplification Focus:** N/A — new code
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** Implement `Page::apply_stylesheets(&mut self, styles: &[LoadedResource])` — for each stylesheet, create a `<style>` text node in DOM `<head>`
- [x] **Step 2:** Write unit test: HTML with `<link rel="stylesheet">` → after `apply_stylesheets`, DOM contains `<style>` with CSS content
- [x] **BDD Verification:** Scenario: "Page applies external CSS" — navigate to page with linked stylesheet, check computed color
- [x] **Verification:** `cargo test -p hpx-browser page::apply_stylesheets` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 2.4: Inline script execution during page load

> **Context:** After HTML parsing and before external script execution, execute all inline `<script>` tags (those without `src` attribute) in document order. Each script runs in `BrowserJsRuntime` with the current DOM context.
> **Verification:** Navigate to page with `<script>window.x=42</script>` → `Runtime.evaluate("window.x")` returns `42`.

- **Priority:** P0
- **Scope:** Script execution, V8 integration
- **Requirement Coverage:** `R3`
- **Scenario Coverage:** `navigation.feature` (inline script execution)
- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** New functionality — no existing behavior to preserve
- **Simplification Focus:** N/A — new code
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** Implement `Page::execute_inline_scripts(&mut self) -> Result<(), PageError>` — walk DOM for `<script>` without `src`, execute each in order via `BrowserJsRuntime::execute_script`
- [x] **Step 2:** Handle script errors gracefully — log warning, continue with next script
- [x] **Step 3:** Write unit test: HTML with inline scripts → scripts executed in order, side effects visible in DOM
- [x] **BDD Verification:** Scenario: "Page executes inline scripts during load"
- [x] **Verification:** `cargo test -p hpx-browser --features v8 page::execute_inline_scripts` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 2.5: External script execution

> **Context:** After fetching external `<script src="...">` files, execute them in document order. External scripts are fetched during the resource loading phase (Task 2.2) and stored. Execution happens after inline scripts, respecting the HTML spec ordering.
> **Verification:** Navigate to page with `<script src="app.js">` where `app.js` sets `window.loaded=true` → `Runtime.evaluate("window.loaded")` returns `true`.

- **Priority:** P0
- **Scope:** Script execution, fetch integration
- **Requirement Coverage:** `R3`
- **Scenario Coverage:** `navigation.feature` (external script execution)
- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** New functionality — no existing behavior to preserve
- **Simplification Focus:** N/A — new code
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** Extend `Page::execute_scripts` to also execute fetched external scripts after inline scripts
- [x] **Step 2:** Respect `async` and `defer` attributes: `async` scripts execute as soon as fetched, `defer` scripts execute after parsing, normal scripts block
- [x] **Step 3:** Write unit test: HTML with mixed inline/external scripts → all execute in correct order
- [x] **BDD Verification:** Scenario: "Page executes external scripts"
- [x] **Verification:** `cargo test -p hpx-browser --features v8 page::execute_scripts` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 2.6: Wire resource loading into Page::navigate

> **Context:** Extend `Page::navigate` (and related methods) to call the full resource loading pipeline after HTML fetch: discover resources → fetch → inject CSS → execute scripts. This is the integration point that makes navigation "real".
> **Verification:** `Page::navigate("http://example.com")` results in a page with CSS applied and scripts executed.

- **Priority:** P0
- **Scope:** Pipeline integration
- **Requirement Coverage:** `R2`, `R3`, `R4`
- **Scenario Coverage:** `navigation.feature` (full lifecycle)
- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** Changes `Page::navigate` behavior — now loads sub-resources (was previously HTML-only)
- **Simplification Focus:** N/A — additive change
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** Add `Page::load_subresources(&mut self, block_types: &HashSet<ResourceType>) -> Result<(), PageError>` that orchestrates the full pipeline
- [x] **Step 2:** Call `load_subresources` at the end of `navigate_inner` (after `reload_html` and challenge classification)
- [x] **Step 3:** Add `subresource_block_types: HashSet<ResourceType>` field to `Page` (configurable via builder or constructor)
- [x] **Step 4:** Write integration test: navigate to local test HTML with CSS + JS → verify applied
- [x] **BDD Verification:** Scenario: "Navigate to a page with sub-resources"
- [x] **Verification:** `cargo test -p hpx-browser --features v8 page::navigate` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

---

## Phase 3: CDP Integration & Features

### Task 3.1: Wire resource loading into CDP Page.navigate

> **Context:** The `CdpSession::handle_request` for `Page.navigate` currently sets `pending_navigate` as a stub. Wire it to actually call `Page::navigate` (which now includes resource loading) and emit proper CDP lifecycle events (`Page.frameNavigated`, `Page.loadEventFired`, `Network.requestWillBeSent`, etc.).
> **Verification:** Puppeteer `page.goto("http://example.com")` works end-to-end.

- **Priority:** P0
- **Scope:** CDP protocol, lifecycle events
- **Requirement Coverage:** `R5`
- **Scenario Coverage:** `navigation.feature` (CDP navigation)
- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** Changes CDP `Page.navigate` from stub to real navigation
- **Simplification Focus:** Remove stub code, replace with real implementation
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** In `CdpSession::handle_request` for `Page.navigate`, call `page.navigate(url).await` instead of just setting `pending_navigate`
- [x] **Step 2:** Emit `Page.frameNavigated` event before navigation starts
- [x] **Step 3:** Emit `Page.domContentEventFired` and `Page.loadEventFired` events after resource loading completes
- [x] **Step 4:** Emit `Network.requestWillBeSent` and `Network.responseReceived` for main document and sub-resources (if Network domain is enabled)
- [x] **Step 5:** Write integration test: send CDP `Page.navigate` → receive lifecycle events
- [x] **BDD Verification:** Scenario: "Puppeteer navigates to page"
- [x] **Verification:** `cargo test -p hpx-browser --features cdp cdp_navigate` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** Start hpxless, connect with Puppeteer, run `page.goto()`, verify success

### Task 3.2: CDP Runtime.evaluate with loaded context

> **Context:** After resource loading, `Runtime.evaluate` should have access to the full page context (CSS applied, scripts executed, DOM styled). Verify this works correctly through the CDP server.
> **Verification:** After navigation, `Runtime.evaluate("getComputedStyle(document.body).color")` returns the correct color.

- **Priority:** P0
- **Scope:** CDP protocol, JS evaluation
- **Requirement Coverage:** `R3`, `R5`
- **Scenario Coverage:** `navigation.feature` (evaluate after load)
- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** Preserves existing `Runtime.evaluate` behavior, but now the page context is richer
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** Verify `Runtime.evaluate` works after resource loading (may require re-initializing JS context after scripts execute)
- [x] **Step 2:** Write test: navigate to page with JS that sets global → `Runtime.evaluate` reads it
- [x] **Step 3:** Write test: navigate to page with CSS → `Runtime.evaluate("getComputedStyle(...)")` returns styled value
- [x] **BDD Verification:** Scenario: "Evaluate JS after page load"
- [x] **Verification:** `cargo test -p hpx-browser --features cdp,v8 cdp_evaluate` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 3.3: Proxy support in hpxless

> **Context:** Wire the `--proxy` CLI flag into the `HttpClient` used for navigation and sub-resource loading. Reuse `hpx::Proxy` for HTTP/HTTPS/SOCKS5 support.
> **Verification:** `hpxless --proxy socks5://127.0.0.1:1080` routes traffic through proxy.

- **Priority:** P1
- **Scope:** Proxy configuration
- **Requirement Coverage:** `R9`
- **Scenario Coverage:** `proxy.feature`
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New functionality — additive
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** N/A
- **Status:** 🟢 DONE
- [x] **Step 1:** Parse `--proxy` flag into `hpx::Proxy` in CLI
- [x] **Step 2:** Pass proxy to `HttpClient::builder().proxy(proxy).build()`
- [x] **Step 3:** Write test: CLI parsing with `--proxy socks5://...` → correct `Proxy` struct
- [x] **BDD Verification:** Scenario: "Navigate through proxy"
- [x] **Verification:** `cargo test -p hpxless cli::proxy` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A — requires proxy server

### Task 3.4: Stealth profile integration

> **Context:** Wire the `--stealth` and `--profile` CLI flags into the page creation. Reuse `StealthProfile` presets (Chrome 148, Firefox 135, etc.).
> **Verification:** `hpxless --stealth` starts with stealth active; `navigator.webdriver` is `false` via CDP.

- **Priority:** P1
- **Scope:** Stealth configuration
- **Requirement Coverage:** `R10`
- **Scenario Coverage:** `stealth.feature`
- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** New functionality — additive
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** Map `--profile` CLI arg to `StealthProfile` preset
- [x] **Step 2:** Pass stealth profile to `Page` creation
- [x] **Step 3:** Ensure stealth bootstrap JS runs when `--stealth` is set
- [x] **BDD Verification:** Scenario: "Stealth profile active"
- [x] **Verification:** `cargo test -p hpxless cli::stealth` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** Start hpxless with `--stealth`, connect CDP, evaluate `navigator.webdriver` → `false`

### Task 3.5: Resource blocking

> **Context:** Wire the `--block` CLI flag into `ResourceLoader`. Blocked resource types are not fetched during navigation.
> **Verification:** `hpxless --block images,media` → no image/media requests during navigation.

- **Priority:** P1
- **Scope:** Resource filtering
- **Requirement Coverage:** `R13`
- **Scenario Coverage:** `resource-blocking.feature`
- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** New functionality — additive
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** Parse `--block` flag into `HashSet<ResourceType>` in CLI
- [x] **Step 2:** Pass block types to `Page` and `ResourceLoader`
- [x] **Step 3:** Write test: HTML with `<img>` + block images → no image fetch
- [x] **BDD Verification:** Scenario: "Block images during navigation"
- [x] **Verification:** `cargo test -p hpx-browser resource_loader::block` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 3.6: Multi-page support

> **Context:** The CDP server should support multiple concurrent page connections. Each WebSocket connection gets its own `CdpSession` and `Page`. Pages are isolated — no shared state.
> **Verification:** Open 10 concurrent WebSocket connections, navigate each to a different page, all succeed.

- **Priority:** P1
- **Scope:** Concurrency, page isolation
- **Requirement Coverage:** `R14`
- **Scenario Coverage:** `multi-page.feature`
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** Preserves existing CDP server behavior (already supports multiple connections via `Arc<Mutex<Page>>`)
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** Verify existing `CdpServer` supports multiple concurrent connections (it uses `Arc<Mutex<Page>>`)
- [x] **Step 2:** If needed, change to per-connection `Page` creation (each WS connection gets a fresh `Page`)
- [x] **Step 3:** Write test: 10 concurrent navigations → all complete successfully
- [x] **BDD Verification:** Scenario: "Multiple concurrent pages"
- [x] **Verification:** `cargo test -p hpx-browser --features cdp multi_page` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

---

## Phase 4: Polish, QA & Docs

### Task 4.1: Graceful shutdown with ecdysis

> **Context:** Use `ecdysis` for graceful restart/reload in the hpxless binary. Handle SIGTERM/SIGINT to cleanly shut down the CDP server, close WebSocket connections, and drop pages.
> **Verification:** Send SIGTERM to hpxless → process exits cleanly, no zombie connections.

- **Priority:** P1
- **Scope:** Signal handling, process lifecycle
- **Requirement Coverage:** `R12`
- **Scenario Coverage:** `hpxless-startup.feature` (shutdown)
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New functionality — additive
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** N/A
- **Status:** 🟢 DONE
- [x] **Step 1:** Add `ecdysis` dependency to `hpxless`
- [x] **Step 2:** Register signal handlers for SIGTERM/SIGINT
- [x] **Step 3:** On signal, set shutdown flag, wait for in-flight navigations (with timeout), close server
- [x] **BDD Verification:** N/A — infrastructure
- [x] **Verification:** `cargo build -p hpxless` succeeds; manual SIGTERM test
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** Start hpxless, send `kill -TERM <pid>`, verify clean exit in logs

### Task 4.2: Memory profiling and optimization

> **Context:** Profile hpxless memory usage per page. Target: < 64MB for text-heavy pages. Identify and fix any leaks.
> **Verification:** Navigate to 10 pages sequentially, check RSS stays under 64MB.

- **Priority:** P2
- **Scope:** Performance, memory
- **Requirement Coverage:** `R7`
- **Scenario Coverage:** N/A — performance validation
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** N/A — optimization
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** Benchmark (criterion)
- **Status:** 🟢 DONE
- [x] **Step 1:** Write benchmark: navigate to test page 100 times, measure RSS
- [x] **Step 2:** Profile with `dhat` or `jemalloc` to identify allocation hotspots
- [x] **Step 3:** Optimize: ensure DOM arena is dropped between navigations, V8 context is reused
- [x] **BDD Verification:** N/A
- [x] **Verification:** `cargo bench -p hpx-browser --bench memory` shows < 64MB per page
- [x] **Advanced Test Verification:** `cargo bench -p hpx-browser --bench memory`
- [x] **Runtime Verification:** N/A

### Task 4.3: Navigation latency benchmark

> **Context:** Benchmark navigation-to-DOMContentLoaded latency. Target: < 2s for typical pages (HTML + CSS + JS).
> **Verification:** Criterion benchmark shows p50 < 2s for test pages.

- **Priority:** P2
- **Scope:** Performance benchmarking
- **Requirement Coverage:** `R8`
- **Scenario Coverage:** N/A — performance validation
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** N/A — benchmark
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** Benchmark (criterion)
- **Status:** 🟢 DONE
- [x] **Step 1:** Create `crates/hpx-browser/benches/navigation.rs` with criterion benchmark
- [x] **Step 2:** Benchmark: navigate to local test HTML with CSS + JS → measure time
- [x] **Step 3:** Benchmark: navigate to remote page (example.com) → measure time
- [x] **BDD Verification:** N/A
- [x] **Verification:** `cargo bench -p hpx-browser --bench navigation` runs successfully
- [x] **Advanced Test Verification:** `cargo bench -p hpx-browser --bench navigation`
- [x] **Runtime Verification:** N/A

### Task 4.4: BDD scenarios for navigation pipeline

> **Context:** Write comprehensive BDD scenarios covering the full navigation pipeline: sub-resource loading, CSS application, script execution, lazy layout.
> **Verification:** All BDD scenarios pass.

- **Priority:** P1
- **Scope:** Acceptance tests
- **Requirement Coverage:** `R2`, `R3`, `R4`, `R6`
- **Scenario Coverage:** All navigation scenarios
- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** N/A — test coverage
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** N/A
- **Status:** 🟢 DONE
- [x] **Step 1:** Write `features/navigation.feature` with all navigation scenarios
- [x] **Step 2:** Write `features/lazy-layout.feature` for layout-on-demand
- [x] **Step 3:** Write `features/resource-blocking.feature` for block types
- [x] **Step 4:** Write `features/proxy.feature` and `features/stealth.feature`
- [x] **Step 5:** Implement all step definitions
- [x] **BDD Verification:** `cargo test -p hpxless --test cucumber` — all scenarios pass
- [x] **Verification:** `cargo test -p hpxless --test cucumber` passes with 0 failures
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 4.5: Documentation and examples

> **Context:** Write README for `bin/hpxless` with usage examples. Document CLI flags, Puppeteer/Playwright connection instructions, and architecture overview.
> **Verification:** README.md exists with working examples.

- **Priority:** P2
- **Scope:** Documentation
- **Requirement Coverage:** N/A — documentation
- **Scenario Coverage:** N/A
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** N/A — documentation
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** N/A
- **Status:** 🟢 DONE
- [x] **Step 1:** Write `bin/hpxless/README.md` with usage, CLI flags, Puppeteer example, Playwright example
- [x] **Step 2:** Add `hpxless` to workspace-level documentation
- [x] **BDD Verification:** N/A
- [x] **Verification:** `rumdl check bin/hpxless/README.md` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

---

## Summary & Timeline

| Phase | Tasks | Target Date |
| :--- | :---: | :--- |
| **1. BDD Harness & Binary Scaffolding** | 3 | 07-16 |
| **2. Resource Loading Pipeline** | 6 | 07-21 |
| **3. CDP Integration & Features** | 6 | 07-25 |
| **4. Polish, QA & Docs** | 5 | 07-28 |
| **Total** | **20** | |

## Definition of Done

1. [ ] **Linted:** `just lint` passes (clippy pedantic + nursery)
2. [ ] **Tested:** `cargo test -p hpxless` and `cargo test -p hpx-browser` pass
3. [ ] **Formatted:** `just format` applied
4. [ ] **Verified:** All task-specific Verification criteria met
5. [ ] **BDD Passing:** `cargo test -p hpxless --test cucumber` — all scenarios green
6. [ ] **Runtime Working:** `hpxless --port 9222` starts, Puppeteer `page.goto("https://example.com")` succeeds
7. [ ] **Memory Target:** < 64MB RSS per page (text-heavy)
8. [ ] **Performance Target:** Navigation to DOMContentLoaded < 2s for typical pages
