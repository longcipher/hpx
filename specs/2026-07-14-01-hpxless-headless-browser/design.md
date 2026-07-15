# Design Document: hpxless — Standalone Headless Browser

| Metadata | Details |
| :--- | :--- |
| **Author** | pb-plan agent |
| **Status** | Draft |
| **Created** | 2026-07-14 |
| **Scope** | Full |

## 1. Executive Summary

**Problem:** The `hpx-browser` crate already implements a powerful headless browser engine (DOM via blitz, V8 JS via deno_core, CDP server/client, stealth, layout via taffy), but it lacks a standalone binary that can run as a drop-in replacement for headless Chrome. The native navigation path does not load sub-resources (CSS, JS, images) or execute scripts during page load, making it unsuitable for real-world web scraping and automation.

**Solution:** Create `bin/hpxless` — a standalone headless browser binary that exposes a CDP WebSocket server (Puppeteer/Playwright compatible), with full sub-resource loading, script execution during page load, and CSS stylesheet resolution. The binary reuses all existing `hpx-browser` infrastructure and adds the missing resource loading pipeline.

---

## 2. Source Inputs & Normalization

### 2.1 Source Materials

The user provided a detailed architectural analysis (Chinese + English) describing how to build a Lightpanda-like headless browser based on Dioxus Labs' Blitz project. The analysis covers:

- Core feasibility and "surgical omission" strategy
- Five-layer architecture: CDP → JS Runtime → DOM Engine → Layout → Networking
- JS runtime binding design (V8 via rusty_v8 or QuickJS)
- Lazy layout computation (only when geometric APIs are called)
- CDP protocol subset implementation
- High-concurrency async networking with proxy support
- Performance optimizations: V8 startup snapshots, single-process multi-sandbox model
- Four-phase MVP implementation path

### 2.2 Normalization Approach

Three parallel subagents analyzed the live codebase:

1. **hpx-browser analyst** — mapped the complete module structure, feature flags, and all 8,500+ lines
2. **Workspace analyst** — cataloged all 7 workspace crates, the hpx HTTP client, CLI structure, and infrastructure
3. **Spec/BDD analyst** — inventoried 14 existing specs, found 4 browser-related specs already in progress

Key normalization findings:

- The user's proposed architecture is **already 80% implemented** in `hpx-browser`
- V8 JS runtime exists via `deno_core` (not rusty_v8 directly) with 16 extensions and 7 bootstrap scripts
- CDP server exists with ~40 methods; CDP client exists for connecting to real Chrome
- DOM (blitz-dom), layout (taffy), CSS (stylo via blitz), HTML parsing (html5ever via blitz-html) are all integrated
- The **critical gap** is sub-resource loading and script execution during navigation — `Page::navigate()` fetches HTML but does not load CSS/JS/images or run `<script>` tags
- A standalone binary (`bin/hpxless`) does not exist yet

### 2.3 Source Requirement Ledger

| Requirement ID | Source Summary | Type | Notes |
| :--- | :--- | :--- | :--- |
| `R1` | Create standalone headless browser binary `bin/hpxless` | Functional | Drop-in for headless Chrome via CDP |
| `R2` | Sub-resource loading during navigation (CSS, JS, images, fonts) | Functional | `Page.navigate` must fetch linked resources |
| `R3` | Script execution during page load (`<script>` tags) | Functional | Inline and external scripts must execute |
| `R4` | CSS stylesheet loading (`<link>` and `<style>`) | Functional | External CSS must be fetched and applied |
| `R5` | CDP WebSocket server compatible with Puppeteer/Playwright | Functional | Already exists; needs resource loading integration |
| `R6` | Lazy layout computation (skip layout unless geometric API called) | Functional | Performance optimization |
| `R7` | Low memory footprint (~20-64MB per page) | Constraint | Single-process, arena-allocated DOM |
| `R8` | Fast cold startup | Constraint | V8 startup snapshots, minimal dependencies |
| `R9` | Proxy support (HTTP/SOCKS5) | Functional | Already in hpx; wire into hpxless |
| `R10` | Stealth/anti-detection profiles | Functional | Already in hpx-browser; expose via CLI flags |
| `R11` | CLI interface with sensible defaults | Functional | Port, stealth, proxy, profile flags |
| `R12` | Graceful shutdown (SIGTERM/SIGINT) | Functional | Use `ecdysis` per AGENTS.md |
| `R13` | Network request interception (block images/media) | Functional | Save bandwidth and memory |
| `R14` | Single-process multi-page model | Constraint | Each page is a tokio task, not a process |

---

## 3. Requirements & Goals

### 3.1 Problem Statement

`hpx-browser` has a powerful engine but cannot autonomously browse real websites because:

1. `Page::navigate()` fetches only the HTML document — it does not load `<link>` CSS, `<script src>` JS, images, or fonts
2. `<script>` tags in the HTML are never executed — JS only runs when explicitly invoked via CDP `Runtime.evaluate`
3. There is no standalone binary — the engine is only usable as a library or through `hpx serve` (which wraps the CDP server with initial HTML)

This means Puppeteer/Playwright connecting to `hpx serve` gets a page where CSS is unstyled, JS has not run, and the DOM reflects raw HTML without any dynamic content.

### 3.2 Functional Goals

1. **`bin/hpxless` binary:** Standalone headless browser that starts a CDP WebSocket server, accepts connections from Puppeteer/Playwright
2. **Full page lifecycle:** Navigation → HTML fetch → parse → sub-resource discovery → parallel resource fetch → CSS apply → script execute → DOM ready
3. **Sub-resource loader:** After HTML parsing, discover `<link>`, `<script src>`, `<img>`, `<style>` and fetch them concurrently
4. **Script executor:** Execute `<script>` tags in document order (inline immediately, external after fetch)
5. **CSS stylesheet loader:** Fetch external CSS via `<link rel="stylesheet">` and inject into `<style>` tags for blitz/stylo processing
6. **Lazy layout:** Only compute layout when CDP or JS calls geometric APIs (`getBoundingClientRect`, `offsetWidth`, etc.)
7. **Resource blocking:** CLI flag to block specific resource types (images, media, fonts) for bandwidth/memory savings
8. **Proxy integration:** Wire hpx proxy support into the navigation pipeline
9. **Stealth integration:** Expose stealth profiles via CLI flags

### 3.3 Non-Functional Goals

- **Performance:** Cold startup < 200ms, navigation to DOMContentLoaded < 2s for typical pages
- **Memory:** < 64MB per page for text-heavy pages (without canvas/screenshot features)
- **Concurrency:** Support 10+ concurrent pages in a single process
- **Compatibility:** Puppeteer `page.goto()` and Playwright `page.goto()` work without modification

### 3.4 Out of Scope

- **Full browser rendering pipeline** (GPU compositing, painting, pixel output) — headless only
- **Service Workers** — not needed for headless automation
- **WebRTC** — not needed for headless automation
- **IndexedDB** — localStorage/sessionStorage only
- **Print/PDF from native mode** — only via CDP client (proxying to real Chrome)
- **Full CSS3/HTML5 spec compliance** — blitz/stylo covers ~90% of common cases
- **Browser extensions** — not applicable
- **Accessibility tree** — future enhancement

### 3.5 Assumptions

- `blitz-dom` and `blitz-html` Git dependencies remain pinned to current rev (`406a57`)
- `deno_core` 0.407.0 remains the V8 binding layer
- The existing `CdpSession::handle_request` method is the integration point for resource loading
- Sub-resource loading uses `hpx::Client` (via `HttpClient`) for all network requests
- Script execution order follows the HTML spec: inline scripts block parsing, external scripts block until fetched

### 3.6 Code Simplification Constraints

- **Behavior Preservation Boundary:** All existing `hpx-browser` public APIs remain unchanged. The `CdpServer::start` signature is preserved. New functionality is additive.
- **Repo Standards:** Follow AGENTS.md — `thiserror` for library errors, `eyre` for application layer, `tracing` for logging, `scc` for concurrent maps, `parking_lot` for locks, `clippy::pedantic` + `clippy::nursery`.
- **Readability Priorities:** Explicit control flow for the resource loading pipeline. No deeply nested futures or combinators. Named functions for each pipeline stage.
- **Refactoring Non-Goals:** Do not refactor existing `hpx-browser` modules unless the resource loading pipeline requires it.
- **Clarity Guardrails:** The resource loading pipeline should be a linear sequence of named stages, not a complex async graph.

---

## 4. Requirements Coverage Matrix

| Requirement ID | Covered In Design | Scenario Coverage | Task Coverage | Status / Rationale |
| :--- | :--- | :--- | :--- | :--- |
| `R1` | Section 6.1 (Module Structure) | `hpxless starts and serves CDP` | Task 1.1, 1.2, 4.1 | Covered |
| `R2` | Section 6.4 (Resource Loading Pipeline) | `Page loads sub-resources` | Task 2.1, 2.2, 2.3 | Covered |
| `R3` | Section 6.4 (Script Execution) | `Page executes inline scripts` | Task 2.4, 2.5 | Covered |
| `R4` | Section 6.4 (CSS Loading) | `Page applies external CSS` | Task 2.3 | Covered |
| `R5` | Section 5.3 (Reuse CDP server) | `Puppeteer connects to hpxless` | Task 3.1, 3.2 | Covered (existing) |
| `R6` | Section 6.4 (Lazy Layout) | `Layout computed only on demand` | Task 2.6 | Covered |
| `R7` | Section 6.5 (Configuration) | `Memory stays under 64MB` | Task 4.2 | Deferred to benchmark phase |
| `R8` | Section 6.5 (V8 Snapshots) | `Cold startup < 200ms` | Task 4.3 | Deferred to optimization phase |
| `R9` | Section 5.3 (Reuse hpx proxy) | `Navigation through SOCKS5 proxy` | Task 3.3 | Covered |
| `R10` | Section 5.3 (Reuse stealth) | `Stealth profile active` | Task 3.4 | Covered |
| `R11` | Section 6.1 (CLI) | `hpxless --port 9222 --stealth` | Task 1.1, 1.2 | Covered |
| `R12` | Section 6.1 (Graceful shutdown) | `SIGTERM shuts down cleanly` | Task 4.1 | Covered |
| `R13` | Section 6.4 (Resource blocking) | `--block images media` | Task 3.5 | Covered |
| `R14` | Section 6.1 (Multi-page) | `10 concurrent pages` | Task 3.6 | Covered |

---

## 5. Architecture Overview

### 5.1 System Context

```text
                    +-----------------+
                    |   Puppeteer /   |
                    |   Playwright    |
                    +--------+--------+
                             |
                        CDP WebSocket
                             |
                    +--------v--------+
                    |    hpxless      |
                    |  (bin/hpxless)  |
                    +--------+--------+
                             |
              +--------------+--------------+
              |                             |
     +--------v--------+         +---------v---------+
     |  hpx-browser    |         |    hpx client     |
     |  (DOM, JS, CDP) |         |  (HTTP, TLS, WS)  |
     +--------+--------+         +---------+---------+
              |                             |
     +--------v--------+         +---------v---------+
     | blitz-dom/html  |         |   Internet        |
     | taffy, deno_core|         |   (websites)      |
     +-----------------+         +-------------------+
```

### 5.2 Key Design Principles

1. **Reuse over rewrite:** Every module in `hpx-browser` is reused as-is. New code is only written for the missing resource loading pipeline.
2. **Lazy evaluation:** Layout and style computation happen only when geometric APIs are called, not during navigation.
3. **Arena allocation:** DOM nodes live in a per-page arena (`NodeId` integers, not `Rc<RefCell>`). No reference cycles.
4. **Single-process multi-page:** Each page is a `tokio::task`, not a process. Shared V8 platform, isolated contexts.
5. **Pipeline architecture:** Resource loading is a linear pipeline: HTML fetch → parse → discover resources → fetch resources → apply CSS → execute scripts → mark DOM ready.

### 5.3 Existing Components to Reuse

| Component | Location | How to Reuse |
| :--- | :--- | :--- |
| `Dom` | `crates/hpx-browser/src/dom.rs` | Full DOM tree operations (create, query, mutate, serialize) |
| `Page` | `crates/hpx-browser/src/page.rs` | Extend with resource loading; reuse navigation, challenge detection |
| `BrowserJsRuntime` | `crates/hpx-browser/src/js_runtime/runtime.rs` | V8 execution environment with all 16 extensions and 7 bootstrap scripts |
| `CdpServer` | `crates/hpx-browser/src/protocol/server.rs` | WebSocket CDP server — reuse as-is, wire into hpxless binary |
| `CdpSession` | `crates/hpx-browser/src/protocol/session.rs` | Extend `Page.navigate` handling to trigger resource loading |
| `HttpClient` | `crates/hpx-browser/src/net/mod.rs` | All HTTP requests (navigation + sub-resources) |
| `StealthProfile` | `crates/hpx-browser/src/stealth.rs` | Anti-detection profiles, behavioral emulation |
| `LayoutEngine` | `crates/hpx-browser/src/layout/engine.rs` | Lazy layout computation, dirty tracking |
| `engine_classify` | `crates/hpx-browser/src/challenge.rs` | Challenge detection during navigation |
| `hpx::Client` | `crates/hpx/src/lib.rs` | Underlying HTTP/1.1 + HTTP/2 transport with TLS |
| `hpx::Proxy` | `crates/hpx/src/proxy.rs` | HTTP/SOCKS5 proxy configuration |
| `ecdysis` | External crate | Graceful restart/reload for the binary |

### 5.4 Architecture Decisions

| Decision ID | Status | Selected Pattern / Principle | Why It Fits Here | Alternatives Rejected | Simplification Impact |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `AD-01` | `New` | **Pipeline** | Resource loading is a linear sequence of stages (fetch HTML → parse → discover → fetch resources → apply → execute). Pipeline pattern makes each stage testable independently. | Graph-based dependency resolver (overkill for HTML resource loading which is mostly sequential with parallel fetches) | Each stage is a simple async function; no complex state machine needed |
| `AD-02` | `Inherited` | **Arena allocation** | `blitz-dom` already uses arena-allocated `NodeId`. JS bindings use `NodeId` integers, not `Rc<RefCell>`. | Reference-counted nodes (would create GC/Rust ownership conflicts) | No memory leaks from reference cycles |
| `AD-03` | `Inherited` | **Feature flags** | `hpx-browser` already uses `v8`, `cdp`, `canvas`, `workers` flags. `hpxless` enables the right subset. | Compile everything (wastes compile time and binary size) | Minimal binary size for headless use case |
| `AD-04` | `New` | **Resource loading as `Page` extension** | Add `load_subresources()` method to `Page` rather than creating a separate `ResourceLoader` struct. The `Page` already owns `Dom` and `HttpClient`. | Separate `ResourceLoader` (would need to borrow `Page` fields, creating lifetime complexity) | All page state in one place; no cross-struct borrowing |
| `AD-05` | `Inherited` | **SRP** | Each module has one responsibility: `Dom` = tree, `Page` = lifecycle, `CdpSession` = protocol, `HttpClient` = network. Resource loading extends `Page` without changing other modules. | Monolithic "browser engine" struct | Changes are localized to touched modules |

- **SRP Check:** `Page` absorbs resource loading because it already owns the DOM and network client. The resource loading logic is a private method chain within `Page`, not a new public concern.
- **DIP Check:** `Page` depends on `HttpClient` (trait-based via hpx) and `Dom` (concrete, blitz-backed). No new dependency inversion needed — the existing seams are sufficient.
- **Dependency Injection Plan:** The `HttpClient` is already injectable via `Page::navigate`'s client parameter. Resource loading reuses the same client.

### 5.5 Project Identity Alignment

No template identity mismatches detected. All crate names (`hpx`, `hpx-browser`, `hpx-cli`, etc.) are project-specific.

### 5.6 BDD/TDD Strategy

- **BDD Runner:** `cucumber` (cucumber-rs, already used in `hpx-dl`)
- **BDD Command:** `cargo test -p hpxless --test cucumber`
- **Unit Test Command:** `cargo test -p hpx-browser`
- **Property Test Tool:** `proptest` (already used in `hpx-browser`)
- **Fuzz Test Tool:** `cargo-fuzz` (conditional — for HTML/CSS parsing robustness)
- **Benchmark Tool:** `criterion` (conditional — for navigation latency)
- **Outer Loop:** CDP-based scenarios: start hpxless → connect via WebSocket → navigate → verify DOM state
- **Inner Loop:** Unit tests for resource discovery, script ordering, CSS injection
- **Step Definition Location:** `crates/hpxless/features/` (BDD) and `crates/hpx-browser/src/` (unit)

### 5.7 BDD Scenario Inventory

| Feature File | Scenario | Business Outcome | Primary Verification | Supporting TDD Focus |
| :--- | :--- | :--- | :--- | :--- |
| `features/hpxless-startup.feature` | hpxless starts and serves CDP | Binary starts, CDP endpoint accessible | WebSocket connect + `Browser.getVersion` | CLI argument parsing, port binding |
| `features/navigation.feature` | Navigate to a page with sub-resources | Page loads CSS/JS/images, DOM reflects styled content | CDP `Page.navigate` + `Runtime.evaluate` returns styled element | Resource discovery, parallel fetch |
| `features/navigation.feature` | Execute inline scripts during load | `<script>` tags run in document order | CDP `Runtime.evaluate` checks script side effects | Script parser, execution order |
| `features/navigation.feature` | Apply external CSS | `<link rel="stylesheet">` styles are applied | CDP `Runtime.evaluate` checks computed style | CSS fetch and injection |
| `features/lazy-layout.feature` | Layout computed only on demand | `getBoundingClientRect` triggers layout | CDP `Runtime.evaluate` returns correct rect | Dirty tracking, layout trigger |
| `features/resource-blocking.feature` | Block images during navigation | Images not fetched, page loads faster | Network request log shows no image requests | Resource type filter |
| `features/proxy.feature` | Navigate through proxy | Page loads via SOCKS5 proxy | CDP `Page.navigate` succeeds with proxy flag | Proxy config wiring |
| `features/stealth.feature` | Stealth profile active | `navigator.webdriver` is false | CDP `Runtime.evaluate` checks property | Stealth bootstrap integration |
| `features/multi-page.feature` | Multiple concurrent pages | 10 pages navigate simultaneously | All pages return correct results | Per-page isolation |

### 5.8 Simplification Opportunities in Touched Code

| Area | Current Complexity or Smell | Planned Simplification | Why It Preserves or Clarifies Behavior |
| :--- | :--- | :--- | :--- |
| `CdpSession::handle_request` for `Page.navigate` | `pending_navigate` is set but actual fetch/reload is stubbed | Wire `Page::navigate` directly into the CDP handler, emitting lifecycle events | Removes stub, makes navigation actually work |
| `Page::navigate` | Creates a new `HttpClient` on every call | Accept an optional shared `HttpClient` via parameter | Reuses connection pool, reduces TLS handshakes |

---

## 6. Detailed Design

### 6.1 Module Structure

```text
bin/hpxless/
├── Cargo.toml
└── src/
    ├── main.rs              # Entry point: CLI parsing, server startup, signal handling
    └── cli.rs               # clap CLI definition (port, stealth, proxy, block, profile)

crates/hpx-browser/src/
├── page.rs                  # EXTEND: add load_subresources(), execute_scripts(), load_stylesheets()
├── resource_loader.rs       # NEW: sub-resource discovery and concurrent fetching
└── protocol/session.rs      # EXTEND: wire Page.navigate to resource loading pipeline
```

### 6.2 Data Structures & Types

```rust
// bin/hpxless/src/cli.rs
#[derive(Parser)]
#[command(name = "hpxless", about = "High-performance headless browser")]
struct Cli {
    /// CDP WebSocket server port
    #[arg(long, default_value_t = 9222)]
    port: u16,

    /// Enable stealth anti-detection
    #[arg(long)]
    stealth: bool,

    /// Browser profile (chrome, firefox, safari)
    #[arg(long, value_enum, default_value_t = ProfileArg::Chrome)]
    profile: ProfileArg,

    /// Proxy URL (http/https/socks5)
    #[arg(long)]
    proxy: Option<String>,

    /// Block resource types (images, media, fonts, stylesheets, scripts)
    #[arg(long, value_delimiter = ',')]
    block: Vec<ResourceType>,

    /// Initial URL to load (optional; if omitted, serves about:blank)
    #[arg(long)]
    url: Option<String>,

    /// Log level
    #[arg(long, default_value = "info")]
    log_level: String,
}

// crates/hpx-browser/src/resource_loader.rs
pub struct ResourceLoader {
    client: HttpClient,
    base_url: Url,
    block_types: HashSet<ResourceType>,
}

pub enum ResourceType {
    Stylesheet,
    Script,
    Image,
    Font,
    Media,
}

pub struct LoadedResource {
    pub url: String,
    pub content: String,
    pub resource_type: ResourceType,
    pub mime_type: Option<String>,
}

// Extension to Page
impl Page {
    pub async fn load_subresources(&mut self, block_types: &HashSet<ResourceType>) -> Result<(), PageError> { ... }
    pub fn execute_inline_scripts(&mut self) -> Result<(), PageError> { ... }
    pub fn apply_stylesheets(&mut self, styles: &[String]) { ... }
}
```

### 6.3 Interface Design

```rust
// Public API for hpxless binary
pub async fn run_hpxless(config: HpxlessConfig) -> Result<(), eyre::Report>;

// Configuration
pub struct HpxlessConfig {
    pub port: u16,
    pub stealth: bool,
    pub profile: BrowserProfile,
    pub proxy: Option<Proxy>,
    pub block_types: HashSet<ResourceType>,
    pub initial_url: Option<String>,
}

// Resource loading (internal to hpx-browser)
impl ResourceLoader {
    pub fn new(client: HttpClient, base_url: Url, block_types: HashSet<ResourceType>) -> Self;
    pub async fn discover_and_load(&self, dom: &Dom) -> Result<Vec<LoadedResource>, NetError>;
    pub fn extract_resource_urls(&self, dom: &Dom) -> Vec<(ResourceType, String)>;
}
```

### 6.4 Logic Flow

**Navigation Pipeline (the core addition):**

```text
1. Page::navigate(url)
   ├── HttpClient::request("GET", url) → HTML response
   ├── Page::reload_html(html, url)  → parse HTML into DOM
   ├── engine_classify(html) → challenge detection
   │
   ├── [NEW] ResourceLoader::extract_resource_urls(dom)
   │   ├── Walk DOM for <link rel="stylesheet" href="...">
   │   ├── Walk DOM for <script src="...">
   │   ├── Walk DOM for <img src="...">
   │   └── Filter by block_types
   │
   ├── [NEW] ResourceLoader::fetch_all(urls) → concurrent HTTP requests
   │   ├── For each stylesheet: inject as <style> tag in DOM
   │   ├── For each script: store for execution
   │   └── For each image: store metadata (dimensions, alt text)
   │
   ├── [NEW] Page::execute_scripts()
   │   ├── Execute inline <script> tags in document order
   │   ├── Execute external scripts (already fetched) in document order
   │   └── Each script runs in BrowserJsRuntime with DOM context
   │
   ├── [NEW] Page::apply_stylesheets()
   │   ├── blitz-dom/stylo processes all <style> tags
   │   └── CSS cascade and specificity computed
   │
   └── Layout remains LAZY (not computed until geometric API called)
```

**Lazy Layout Trigger:**

```text
JS calls getBoundingClientRect() / offsetWidth / etc.
├── LayoutEngine::is_dirty() → true
├── LayoutEngine::resolve_styles() (via stylo)
├── LayoutEngine::resolve_layout() (via taffy)
├── Return computed geometry
└── LayoutEngine::is_dirty() → false (until DOM mutation)
```

### 6.5 Configuration

| Config | Default | Description |
| :--- | :--- | :--- |
| `--port` | `9222` | CDP WebSocket server port |
| `--stealth` | `false` | Enable anti-detection stealth |
| `--profile` | `chrome` | Browser profile for TLS/headers |
| `--proxy` | `None` | Proxy URL (http/https/socks5) |
| `--block` | `[]` | Resource types to block |
| `--url` | `None` | Initial URL to load on startup |
| `--log-level` | `info` | Tracing log level |

### 6.6 Error Handling

- `PageError` already covers navigation failures, network errors, and HTML parse errors
- New variants: `PageError::ResourceLoad(String)` for sub-resource failures (non-fatal — page continues with partial resources)
- Sub-resource failures are logged as warnings, not errors — a page with a failed CSS load is still usable
- Script execution errors are caught and logged, not propagated — a page with a JS error is still usable

### 6.7 Maintainability Notes

- Resource loading is a method chain on `Page`: `navigate` → `load_subresources` → `execute_scripts` → `apply_stylesheets`. Each method is independently testable.
- `ResourceLoader` is a simple struct with `HttpClient` + `base_url` + `block_types`. No traits, no generics, no abstractions.
- The `CdpSession` changes are minimal: replace the `pending_navigate` stub with a real `Page::navigate` call.
- Script execution order follows the HTML spec: scripts are collected during DOM walk, sorted by document position, executed sequentially.

---

## 7. Verification & Testing Strategy

### 7.1 Unit Testing

| Test Target | Scope | Tool |
| :--- | :--- | :--- |
| `ResourceLoader::extract_resource_urls` | DOM walking, URL extraction, block filtering | `cargo test -p hpx-browser` |
| `Page::execute_scripts` | Script ordering, inline vs external, error handling | `cargo test -p hpx-browser --features v8` |
| `Page::apply_stylesheets` | CSS injection into DOM | `cargo test -p hpx-browser` |
| CLI argument parsing | All flags, defaults, combinations | `cargo test -p hpxless` |

### 7.2 Property Testing

| Target Behavior | Why Property Testing Helps | Tool / Command | Planned Invariants |
| :--- | :--- | :--- | :--- |
| Resource URL extraction | Various HTML structures produce various URL patterns | `proptest` in `hpx-browser` | URLs are valid, relative URLs resolve correctly, block filtering is idempotent |
| Script ordering | Different script placements in HTML | `proptest` in `hpx-browser` | Execution order matches document order |

### 7.3 Integration Testing

- Start `hpxless` binary → connect via WebSocket → send CDP commands → verify responses
- Use `tokio-tungstenite` client to connect and send raw CDP JSON-RPC
- Verify: navigate to a test HTML page with inline CSS, external JS, and images → check that CSS is applied and JS has executed

### 7.4 BDD Acceptance Testing

| Scenario ID | Feature File | Command | Success Criteria |
| :--- | :--- | :--- | :--- |
| **BDD-01** | `features/hpxless-startup.feature` | `cargo test -p hpxless --test cucumber` | Server starts, CDP endpoint responds |
| **BDD-02** | `features/navigation.feature` | `cargo test -p hpxless --test cucumber` | Page loads with CSS applied and scripts executed |
| **BDD-03** | `features/resource-blocking.feature` | `cargo test -p hpxless --test cucumber` | Blocked resource types not fetched |

### 7.5 Robustness & Performance Testing

| Test Type | When It Is Required | Tool / Command | Planned Coverage or Reason Not Needed |
| :--- | :--- | :--- | :--- |
| **Fuzz** | HTML parsing robustness | `cargo fuzz run parse_html` | Crash-safety for malformed HTML input |
| **Benchmark** | Navigation latency | `criterion` in `hpx-browser` | Navigation to DOMContentLoaded < 2s |

### 7.6 Critical Path Verification (The "Harness")

| Verification Step | Command | Success Criteria |
| :--- | :--- | :--- |
| **VP-01** | `cargo build -p hpxless` | Binary compiles without errors |
| **VP-02** | `./target/debug/hpxless --port 0` | Server starts, prints CDP endpoint URL |
| **VP-03** | Connect via WebSocket, send `{"id":1,"method":"Browser.getVersion"}` | Returns version info |
| **VP-04** | Send `{"id":2,"method":"Page.navigate","params":{"url":"data:text/html,<h1>hello</h1>"}}` | Page navigates, `Page.loadEventFired` event emitted |
| **VP-05** | Send `{"id":3,"method":"Runtime.evaluate","params":{"expression":"document.querySelector('h1').textContent"}}` | Returns `"hello"` |
| **VP-06** | Navigate to page with `<style>h1{color:red}</style>` + `<link rel="stylesheet" href="...">` | CSS applied (check computed style) |
| **VP-07** | Navigate to page with `<script>window.x=42</script>` | `Runtime.evaluate("window.x")` returns `42` |
| **VP-08** | `cargo test -p hpx-browser` | All existing tests still pass |
| **VP-09** | `cargo test -p hpxless` | All new tests pass |
| **VP-10** | `just format && just lint` | No lint errors |

---

## 8. Implementation Plan

- [ ] **Phase 1: Binary scaffolding** — Create `bin/hpxless`, CLI, CDP server startup, signal handling
- [ ] **Phase 2: Resource loading pipeline** — Sub-resource discovery, concurrent fetch, CSS injection, script execution
- [ ] **Phase 3: CDP integration** — Wire resource loading into CDP `Page.navigate`, lifecycle events, resource blocking
- [ ] **Phase 4: Polish** — BDD tests, property tests, benchmarks, documentation, memory profiling
