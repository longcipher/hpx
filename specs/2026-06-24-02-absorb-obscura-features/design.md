# Design: Absorb Obscura CLI Features & MCP Server

| Metadata | Details |
| :--- | :--- |
| **Author** | pb-plan agent |
| **Status** | Draft |
| **Created** | 2026-06-24 |
| **Reviewers** | N/A |

## 1. Executive Summary

**Problem:** hpx-cli currently supports raw HTTP requests, WebSocket connections, and download management, but lacks the browser-oriented CLI subcommands that obscura-cli provides: `fetch` (render pages with dump formats), `scrape` (parallel scraping via worker processes), `serve` (CDP server), and `mcp` (Model Context Protocol server for AI agents). hpx-browser already has the underlying engine (DOM, CSS, V8, CDP, stealth, challenge detection) but none of it is exposed through the CLI.

**Solution:** Add `fetch`, `scrape`, `serve`, and `mcp` subcommands to hpx-cli, create a new `hpx-mcp` crate for the MCP server, add a `hpx-worker` binary for parallel scraping, and port supporting features (SSRF protection, robots.txt compliance, tracker blocking, HTML-to-Markdown, env var config) from obscura into hpx-browser. All networking uses `hpx` (not `wreq`/`reqwest`).

---

## 2. Source Inputs & Normalization

### 2.1 Source Materials

- Full source of obscura-cli (`/Users/akagi201/tmp/obscura/crates/obscura-cli/`) — main.rs (1500+ lines), worker.rs
- Full source of obscura-mcp (`/Users/akagi201/tmp/obscura/crates/obscura-mcp/`) — MCP JSON-RPC server, 32 tools
- Full source of obscura-cdp (`/Users/akagi201/tmp/obscura/crates/obscura-cdp/`) — CDP server with multi-worker load balancer
- Full source of obscura-net (`/Users/akagi201/tmp/obscura/crates/obscura-net/`) — SSRF guard, robots cache, blocklist
- Full source of obscura-browser (`/Users/akagi201/tmp/obscura/crates/obscura-browser/`) — Page, BrowserContext, lifecycle
- Existing hpx workspace (hpx, hpx-browser, hpx-cli, hpx-emulation, hpx-dl, yawc)
- Existing spec `2026-06-24-01-browser-oxide-features` — covers library-level browser engine features

### 2.2 Normalization Approach

The user's requirement is to "absorb all obscura features" into hpx and make hpx-cli support all obscura-cli functionality. After deep comparison, obscura's features split into two categories:

1. **Library-level** (DOM, CSS, V8, CDP server, stealth, challenge detection, canvas, workers, iframes, QUIC) — already covered by `2026-06-24-01-browser-oxide-features` spec and partially implemented in `hpx-browser`.
2. **CLI-level** (fetch/scrape/serve/mcp subcommands, MCP server, worker subprocess, SSRF, robots.txt, tracker blocking, HTML-to-Markdown, env vars) — **NOT covered by any existing spec**. This is the focus here.

Key constraint: hpx uses `hpx` for HTTP (not `wreq`/`reqwest`). obscura uses `reqwest` + optional `wreq` for stealth. All networking in this plan routes through `hpx::Client`.

### 2.3 Source Requirement Ledger

| Requirement ID | Source Summary | Type | Notes |
| :--- | :--- | :--- | :--- |
| `R1` | `fetch` subcommand: render page via hpx-browser, dump in multiple formats (html, text, links, markdown, original, assets, cookies) | Functional | From obscura-cli fetch |
| `R2` | `scrape` subcommand: parallel multi-URL scraping via worker subprocesses | Functional | From obscura-cli scrape |
| `R3` | `serve` subcommand: CDP WebSocket server with multi-worker load balancer | Functional | From obscura-cli serve |
| `R4` | `mcp` subcommand: MCP server (stdio + HTTP transports) for AI agent integration | Functional | From obscura-cli mcp + obscura-mcp |
| `R5` | Worker binary (`hpx-worker`): subprocess for scrape, stdin/stdout JSON-line protocol | Functional | From obscura-cli worker.rs |
| `R6` | SSRF protection: block loopback, RFC1918, link-local, broadcast, IPv4-mapped IPv6 | Functional | From obscura-net SsrfGuardResolver |
| `R7` | robots.txt compliance: fetch, parse, cache, enforce per-host | Functional | From obscura-net RobotsCache |
| `R8` | Tracker blocklist: 3520+ domain PGL list, block requests to known trackers | Functional | From obscura-net blocklist |
| `R9` | HTML-to-Markdown: convert rendered page to readable Markdown | Functional | From obscura-js markdown.rs |
| `R10` | Environment variable configuration: HPX_PROXY, HPX_TIMEOUT, HPX_TIMEZONE, HPX_ALLOW_PRIVATE_NETWORK, etc. | Functional | From obscura env vars |
| `R11` | Process-level hard deadline for CLI fetch: force-exit after timeout+wait+grace | Functional | From obscura-cli fetch |
| `R12` | Dump format `original`: short-circuit browser, stream raw HTTP response bytes | Functional | From obscura-cli DumpFormat::Original |
| `R13` | Dump format `assets`: NDJSON listing every sub-resource URL | Functional | From obscura-cli DumpFormat::Assets |
| `R14` | Dump format `cookies`: dump all cookies including HttpOnly | Functional | From obscura-cli DumpFormat::Cookies |
| `R15` | `--stealth` flag on fetch/serve/mcp: enable anti-detection + tracker blocking | Functional | From obscura-cli |
| `R16` | `--obey-robots` global flag: respect robots.txt | Functional | From obscura-cli |
| `R17` | `--allow-private-network` global flag: allow loopback/RFC1918 fetches | Functional | From obscura-cli |
| `R18` | `--v8-flags` global flag: pass raw flags to V8 | Functional | From obscura-cli |
| `R19` | `--storage-dir` flag: persistent cookie storage directory | Functional | From obscura-cli |
| `R20` | `--wait-until` flag on fetch: load, domcontentloaded, networkidle0 | Functional | From obscura-cli |
| `R21` | `--wait` flag on fetch: post-load settle time | Functional | From obscura-cli |
| `R22` | `--selector` flag on fetch: wait for CSS selector | Functional | From obscura-cli |
| `R23` | `--eval` / `-e` flag on fetch/scrape: evaluate JS expression | Functional | From obscura-cli |
| `R24` | Default subcommand (no subcommand = serve) | Functional | From obscura-cli |

---

## 3. Requirements & Goals

### 3.1 Problem Statement

hpx-cli can make HTTP requests, manage WebSocket connections, and run downloads, but cannot render web pages, execute JavaScript, or serve as a headless browser. Users who want to scrape JS-heavy pages, integrate with AI agents via MCP, or run a local CDP server must use obscura or headless Chrome instead. The hpx-browser crate already has the engine but it's not wired to the CLI.

### 3.2 Functional Goals

1. **`hpx fetch <URL>`**: Render page via hpx-browser, dump output in 7 formats, support `--eval`, `--selector`, `--wait`, `--wait-until`, `--timeout`, `--stealth`, `--output`, `--quiet`
2. **`hpx scrape <URL...>`**: Parallel multi-URL scraping via `hpx-worker` subprocesses, stdin/stdout JSON protocol, `--concurrency`, `--eval`, `--format`, `--timeout`
3. **`hpx serve`**: CDP WebSocket server using hpx-browser protocol module, `--port`, `--host`, `--stealth`, `--workers` (multi-process load balancer), `--proxy`, `--user-agent`, `--allow-file-access`, `--storage-dir`, `--quiet`
4. **`hpx mcp`**: MCP server (32+ tools) using hpx-browser, stdio + HTTP transports, `--http`, `--host`, `--port`, `--proxy`, `--user-agent`, `--stealth`
5. **`hpx-worker` binary**: Separate binary for scrape subprocess communication
6. **SSRF protection**: Block private network fetches by default, `--allow-private-network` override
7. **robots.txt compliance**: `--obey-robots` fetches and enforces robots.txt
8. **Tracker blocking**: `--stealth` enables domain blocklist
9. **HTML-to-Markdown**: `--dump markdown` converts rendered page
10. **Env var config**: `HPX_PROXY`, `HPX_TIMEOUT`, `HPX_TIMEZONE`, `HPX_ALLOW_PRIVATE_NETWORK`
11. **Process hard deadline**: Force-exit backstop for `fetch` command

### 3.3 Non-Functional Goals

- **No wreq/reqwest**: All HTTP through `hpx::Client`
- **Feature-gated**: MCP and worker binary behind Cargo features to avoid bloating default build
- **Existing CLI preserved**: All current hpx-cli functionality (HTTP requests, WebSocket, downloads) unchanged
- **Test coverage**: CLI parsing tests with proptest, integration tests for each subcommand

### 3.4 Out of Scope

- Library-level browser engine features (DOM, CSS, V8, canvas, challenge detection, stealth profiles) — covered by `2026-06-24-01-browser-oxide-features` spec
- Full CDP protocol implementation — already in hpx-browser `protocol` module (feature-gated `cdp`)
- Browser emulation profiles — already in hpx-emulation

### 3.5 Assumptions

- hpx-browser's `protocol` module (CDP server) is functional or will be completed by the browser-oxide spec
- hpx-browser's `page` module provides `navigate()`, `evaluate()`, `title()`, `content()`, `text_content()` APIs
- hpx-browser's `stealth` module provides anti-detection and tracker blocking
- hpx-browser's `challenge` module provides anti-bot detection
- The `hpx` crate's `Client` supports proxy, cookies, user-agent, and all HTTP methods needed

### 3.6 Code Simplification Constraints

- **Behavior Preservation Boundary:** All existing hpx-cli functionality unchanged. New subcommands are additive.
- **Repo Standards:** AGENTS.md mandates: `thiserror` for library errors, `eyre` for CLI, `scc` over `dashmap`, `hpx` over `reqwest`, `winnow` for parsing, `tracing` only, clippy::pedantic.
- **Readability Priorities:** Explicit control flow, named constants, no nested ternaries.
- **Refactoring Non-Goals:** Do not restructure existing hpx-cli modules unless a task requires it.
- **Clarity Guardrails:** Dump format dispatch should be a clear match statement, not a trait object hierarchy.

---

## 4. Requirements Coverage Matrix

| Requirement ID | Covered In Design | Scenario Coverage | Task Coverage | Status / Rationale |
| :--- | :--- | :--- | :--- | :--- |
| `R1` | §6.1, §6.3 | fetch subcommand scenarios | Task 2.1 | Covered |
| `R2` | §6.1, §6.3 | scrape subcommand scenarios | Task 2.2 | Covered |
| `R3` | §6.1, §6.3 | serve subcommand scenarios | Task 2.3 | Covered |
| `R4` | §6.1, §6.3 | mcp subcommand scenarios | Task 3.1 | Covered |
| `R5` | §6.1, §6.3 | worker binary | Task 2.2 | Covered |
| `R6` | §6.4 | SSRF protection | Task 4.1 | Covered |
| `R7` | §6.4 | robots.txt compliance | Task 4.2 | Covered |
| `R8` | §6.4 | tracker blocking | Task 4.3 | Covered |
| `R9` | §6.4 | HTML-to-Markdown | Task 4.4 | Covered |
| `R10` | §6.5 | env var config | Task 4.5 | Covered |
| `R11` | §6.4 | hard deadline | Task 2.1 | Covered |
| `R12` | §6.3 | dump original | Task 2.1 | Covered |
| `R13` | §6.3 | dump assets | Task 2.1 | Covered |
| `R14` | §6.3 | dump cookies | Task 2.1 | Covered |
| `R15` | §6.3 | stealth flag | Task 2.1 | Covered |
| `R16` | §6.3 | obey-robots flag | Task 4.2 | Covered |
| `R17` | §6.3 | allow-private-network | Task 4.1 | Covered |
| `R18` | §6.3 | v8-flags | Task 4.5 | Covered |
| `R19` | §6.3 | storage-dir | Task 4.5 | Covered |
| `R20` | §6.3 | wait-until | Task 2.1 | Covered |
| `R21` | §6.3 | wait flag | Task 2.1 | Covered |
| `R22` | §6.3 | selector flag | Task 2.1 | Covered |
| `R23` | §6.3 | eval flag | Task 2.1 | Covered |
| `R24` | §6.3 | default subcommand | Task 2.3 | Covered |

---

## 5. Architecture Overview

### 5.1 System Context

```text
┌─────────────────────────────────────────────────────────┐
│                    hpx-cli (modified)                     │
│                                                          │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌─────────┐ │
│  │  fetch    │  │  scrape  │  │  serve   │  │  mcp    │ │
│  │  subcmd   │  │  subcmd  │  │  subcmd  │  │  subcmd │ │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └────┬────┘ │
│       │              │              │              │      │
│  ┌────┴─────┐  ┌────┴─────┐  ┌────┴─────┐  ┌────┴────┐ │
│  │ hpx-     │  │ hpx-     │  │ hpx-     │  │ hpx-mcp │ │
│  │ browser  │  │ worker   │  │ browser  │  │ crate   │ │
│  │ (Page)   │  │ (binary) │  │ (CDP)    │  │ (tools) │ │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └────┬────┘ │
│       │              │              │              │      │
│  ┌────┴──────────────┴──────────────┴──────────────┴────┐ │
│  │              hpx-browser (existing crate)             │ │
│  │  Page, DOM, CSS, V8, CDP, Stealth, Challenge, Net    │ │
│  └────────────────────────┬──────────────────────────────┘ │
├───────────────────────────┴──────────────────────────────┤
│                    hpx crate (existing)                    │
│  Client, TLS, Cookies, Proxy, WebSocket                  │
└──────────────────────────────────────────────────────────┘
```

### 5.2 Key Design Principles

- **Reuse hpx-browser:** All browser engine features already exist or are planned. CLI is the integration layer.
- **No wreq/reqwest:** obscura uses reqwest+wreq; hpx uses `hpx::Client` exclusively.
- **Feature-gated MCP:** `hpx-mcp` behind a Cargo feature to avoid pulling in MCP deps for HTTP-only users.
- **Worker subprocess pattern:** Same architecture as obscura — spawn `hpx-worker` processes, communicate via stdin/stdout JSON lines.
- **Global flags:** Proxy, user-agent, storage-dir, obey-robots, allow-private-network, v8-flags apply to all browser subcommands.

### 5.3 Existing Components to Reuse

| Component | Location | How to Reuse |
| :--- | :--- | :--- |
| `hpx-browser::page::Page` | `crates/hpx-browser/src/page.rs` | Use for navigate, evaluate, title, content in fetch/scrape |
| `hpx-browser::protocol` | `crates/hpx-browser/src/protocol/` | CDP server for serve subcommand (feature-gated `cdp`) |
| `hpx-browser::stealth` | `crates/hpx-browser/src/stealth/` | Anti-detection, tracker blocking |
| `hpx-browser::challenge` | `crates/hpx-browser/src/challenge/` | Anti-bot detection |
| `hpx-browser::host::EngineHandle` | `crates/hpx-browser/src/host/` | Send+Sync wrapper over !Send engine |
| `hpx::Client` | `crates/hpx/src/client/http.rs` | HTTP backend for all fetch operations |
| `hpx::Proxy` | `crates/hpx/src/proxy/` | Proxy configuration |
| `hpx-yawc` | `crates/yawc/` | WebSocket for CDP/MCP |
| `clap` (derive) | workspace dep | CLI argument parsing |
| `eyre` | workspace dep | CLI error handling |

### 5.4 Architecture Decisions

| Decision ID | Status | Selected Pattern / Principle | Why It Fits Here | Alternatives Rejected | Simplification Impact |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `AD-01` | New | Subcommand pattern (clap) | Each browser subcommand is a self-contained handler | Monolithic command with mode flags | Clear separation, each subcommand testable independently |
| `AD-02` | New | Worker subprocess | Scrape uses spawned hpx-worker processes communicating via stdin/stdout JSON | In-process parallelism with tokio tasks | Process isolation prevents V8 panics from killing main; matches obscura architecture |
| `AD-03` | Inherited | Feature gate | MCP behind `mcp` feature, worker behind `worker` feature | Always-compile everything | Default build stays lean for HTTP-only users |
| `AD-04` | Inherited | Builder pattern | Global CLI flags feed into BrowserContext builder | Direct struct construction | Consistent with hpx API style |

- **SRP Check:** Each subcommand handler has one responsibility. The MCP server is a separate crate. The worker binary is a separate binary target.
- **DIP Check:** CLI handlers depend on hpx-browser's Page/EngineHandle abstractions, not concrete V8 internals.

### 5.5 Project Identity Alignment

No template identity mismatches detected.

### 5.6 BDD/TDD Strategy

- **BDD Runner:** `cucumber` (Rust)
- **BDD Command:** `cargo test -p hpx-cli --test cli_features`
- **Unit Test Command:** `cargo test -p hpx-cli`
- **Property Test Tool:** `proptest` for CLI argument parsing (already used in hpx-cli)
- **Fuzz Test Tool:** N/A — CLI args are parsed by clap, not raw input
- **Benchmark Tool:** N/A — CLI is not a hot path
- **Outer Loop:** CLI subcommand scenarios prove end-to-end behavior
- **Inner Loop:** Unit tests for dump format rendering, worker protocol, MCP tool dispatch
- **Step Definition Location:** `bin/hpx-cli/tests/`

### 5.7 BDD Scenario Inventory

| Feature File | Scenario | Business Outcome | Primary Verification | Supporting TDD Focus |
| :--- | :--- | :--- | :--- | :--- |
| `features/fetch.feature` | Fetch page and dump HTML | User gets rendered HTML from JS-heavy page | `hpx fetch --dump html <url>` outputs HTML | Dump format dispatch |
| `features/fetch.feature` | Fetch page and dump text | User gets readable text content | `hpx fetch --dump text <url>` outputs text | Text extraction |
| `features/fetch.feature` | Fetch page and dump links | User gets all links as TSV | `hpx fetch --dump links <url>` outputs links | Link extraction |
| `features/fetch.feature` | Fetch page and dump markdown | User gets Markdown | `hpx fetch --dump markdown <url>` outputs MD | HTML-to-Markdown |
| `features/fetch.feature` | Fetch with --eval | User evaluates JS and gets result | `hpx fetch -e "document.title" <url>` | JS eval bridge |
| `features/fetch.feature` | Fetch with --selector wait | User waits for element to appear | `hpx fetch --selector "#app" <url>` | Selector polling |
| `features/fetch.feature` | Fetch raw bytes (dump original) | User gets raw HTTP response | `hpx fetch --dump original <url>` | Binary-safe output |
| `features/scrape.feature` | Scrape multiple URLs | User scrapes N URLs in parallel | `hpx scrape <url1> <url2>` outputs JSON | Worker protocol |
| `features/scrape.feature` | Scrape with concurrency limit | User controls parallelism | `hpx scrape --concurrency 3 <urls>` | Semaphore |
| `features/serve.feature` | Start CDP server | Puppeteer can connect | `hpx serve --port 9222` accepts WS | CDP handshake |
| `features/serve.feature` | Multi-worker serve | Load balancer distributes | `hpx serve --workers 4` spawns 4 workers | TCP round-robin |
| `features/mcp.feature` | MCP stdio server | AI agent can list tools | `hpx mcp` responds to tools/list | MCP protocol |
| `features/mcp.feature` | MCP HTTP server | HTTP client can call tools | `hpx mcp --http --port 3000` | HTTP transport |

### 5.8 Simplification Opportunities in Touched Code

| Area | Current Complexity or Smell | Planned Simplification | Why It Preserves or Clarifies Behavior |
| :--- | :--- | :--- | :--- |
| `hpx-cli/src/main.rs` | Flat match on commands | Group browser subcommands into separate module | Keeps main.rs focused on routing, handlers in `browser.rs` |
| Dump format rendering | N/A (new code) | Single `dump_output()` function with match on format | Clear dispatch, easy to add new formats |

---

## 6. Detailed Design

### 6.1 Module Structure

```text
bin/hpx-cli/src/
├── main.rs                    # MODIFIED — add browser subcommands routing
├── cli.rs                     # MODIFIED — add Fetch, Scrape, Serve, Mcp subcommands + global flags
├── browser.rs                 # NEW — fetch, scrape, serve, mcp handler functions
├── http.rs                    # existing
├── ws.rs                      # existing
├── output.rs                  # existing
└── progress.rs                # existing

bin/hpx-worker/
├── Cargo.toml                 # NEW — worker binary for scrape subprocess
└── src/
    └── main.rs                # NEW — stdin/stdout JSON-line protocol handler

crates/hpx-mcp/
├── Cargo.toml                 # NEW — MCP server crate
└── src/
    ├── lib.rs                 # MCP server entry point (stdio + HTTP transports)
    ├── tools.rs               # 32+ MCP tool definitions
    ├── transport/
    │   ├── stdio.rs           # stdio transport (newline-delimited JSON)
    │   └── http.rs            # HTTP transport (POST /mcp, SSE, CORS)
    └── state.rs               # BrowserState (multi-tab, active tab, refs)

crates/hpx-browser/src/
├── net/
│   ├── ssrf.rs                # NEW — SSRF guard (IP blocking)
│   ├── robots.rs              # NEW — robots.txt cache
│   └── blocklist.rs           # NEW — tracker domain blocklist
└── markdown.rs                # NEW — HTML-to-Markdown converter
```

### 6.2 Data Structures & Types

```rust
// === cli.rs — new subcommands ===

#[derive(Debug, Subcommand)]
pub(crate) enum Commands {
    #[command(subcommand)]
    Dl(DlCommands),
    /// Fetch and render a single page via the browser engine.
    Fetch {
        url: String,
        #[arg(long, value_enum)]
        dump: Option<DumpFormat>,
        #[arg(long)]
        selector: Option<String>,
        #[arg(long, default_value_t = 5)]
        wait: u64,
        #[arg(long, default_value_t = 30)]
        timeout: u64,
        #[arg(long, default_value = "load")]
        wait_until: String,
        #[arg(long, short)]
        eval: Option<String>,
        #[arg(long, short = 'o')]
        output: Option<std::path::PathBuf>,
        #[arg(long, short)]
        quiet: bool,
    },
    /// Scrape multiple URLs in parallel via worker processes.
    Scrape {
        urls: Vec<String>,
        #[arg(long, short)]
        eval: Option<String>,
        #[arg(long, default_value_t = 10)]
        concurrency: usize,
        #[arg(long, default_value = "json")]
        format: String,
        #[arg(long, default_value_t = 60)]
        timeout: u64,
        #[arg(long, short)]
        quiet: bool,
    },
    /// Start a CDP WebSocket server for Puppeteer/Playwright.
    Serve {
        #[arg(short, long, default_value_t = 9222)]
        port: u16,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long)]
        stealth: bool,
        #[arg(long, default_value_t = 1)]
        workers: u16,
        #[arg(long)]
        allow_file_access: bool,
        #[arg(long)]
        storage_dir: Option<std::path::PathBuf>,
        #[arg(long)]
        quiet: bool,
    },
    /// Start an MCP server for AI agent integration.
    Mcp {
        #[arg(long)]
        http: bool,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 3000)]
        port: u16,
        #[arg(long)]
        stealth: bool,
    },
}

#[derive(Clone, Debug, clap::ValueEnum, PartialEq, Eq)]
pub(crate) enum DumpFormat {
    Html,
    Text,
    Links,
    Markdown,
    Original,
    Assets,
    Cookies,
}

// === Cli struct additions ===
// Global browser flags (apply to fetch/scrape/serve/mcp)

#[arg(long, global = true)]
pub obey_robots: bool,

#[arg(long, global = true)]
pub allow_private_network: bool,

#[arg(long, global = true)]
pub v8_flags: Option<String>,

#[arg(long, global = true)]
pub storage_dir: Option<std::path::PathBuf>,

// === browser.rs — handler signatures ===

pub(crate) async fn handle_fetch(args: FetchArgs, global: GlobalFlags) -> eyre::Result<()>;
pub(crate) async fn handle_scrape(args: ScrapeArgs, global: GlobalFlags) -> eyre::Result<()>;
pub(crate) async fn handle_serve(args: ServeArgs, global: GlobalFlags) -> eyre::Result<()>;
pub(crate) async fn handle_mcp(args: McpArgs, global: GlobalFlags) -> eyre::Result<()>;

// === hpx-mcp — tool definition ===

pub struct McpTool {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: serde_json::Value,
}

// === hpx-mcp — BrowserState ===

pub struct BrowserState {
    tabs: BTreeMap<String, TabState>,
    active_tab: String,
    refs: HashMap<String, String>,  // ref -> CSS selector
    console_messages: Vec<ConsoleMessage>,
}

// === hpx-worker — protocol ===

// stdin commands:
#[derive(Deserialize)]
enum WorkerCommand {
    Navigate { url: String },
    Evaluate { expression: String },
    DumpHtml,
    DumpText,
    Title,
    Shutdown,
}

// stdout responses:
#[derive(Serialize)]
struct WorkerResponse {
    ok: bool,
    result: Option<serde_json::Value>,
    error: Option<String>,
}
```

### 6.3 Interface Design

```rust
// === Page API usage in fetch handler ===

// Create context with global flags
let context = BrowserContext::builder()
    .proxy(global.proxy.clone())
    .user_agent(global.user_agent.clone())
    .stealth(args.stealth)
    .storage_dir(global.storage_dir.clone())
    .allow_private_network(global.allow_private_network)
    .obey_robots(global.obey_robots)
    .build();

let mut page = Page::new(context);

// Navigate with wait-until
page.navigate_with_wait(&args.url, wait_until).await?;

// Post-load settle
page.settle(args.wait * 1000).await?;

// Eval if requested
if let Some(expr) = &args.eval {
    let result = page.evaluate(expr);
    // Return directly if no dump/selector specified
}

// Wait for selector
if let Some(sel) = &args.selector {
    page.wait_for_selector(sel, Duration::from_secs(args.timeout)).await?;
}

// Dump output
match args.dump {
    DumpFormat::Html => page.outer_html(),
    DumpFormat::Text => page.text_content(),
    DumpFormat::Links => extract_links(&page),
    DumpFormat::Markdown => page.to_markdown(),
    DumpFormat::Original => { /* raw HTTP bytes, skip browser */ },
    DumpFormat::Assets => extract_assets(&page),
    DumpFormat::Cookies => dump_cookies(&page),
}

// === MCP tool list (32 tools) ===
// All tools delegate to hpx-browser Page methods:
// browser_navigate -> page.navigate()
// browser_snapshot -> page.text_content() + page.url()
// browser_click -> page.click(selector)
// browser_fill -> page.fill(selector, value)
// browser_type -> page.type_text(selector, text)
// browser_evaluate -> page.evaluate(expr)
// browser_wait_for -> page.wait_for_selector(sel, timeout)
// browser_network_requests -> page.network_events()
// browser_console_messages -> page.console_messages()
// browser_close -> page.close()
// browser_markdown -> page.to_markdown()
// browser_links -> extract_links(&page)
// ... etc.

// === Worker protocol ===
// Parent spawns hpx-worker, writes JSON commands to stdin,
// reads JSON responses from stdout.
// Commands: navigate, evaluate, dump_html, dump_text, title, shutdown
```

### 6.4 Logic Flow

**`hpx fetch` flow:**

1. Parse CLI args, merge global flags (proxy, user-agent, storage-dir, etc.)
2. If `--dump original` → short-circuit: use `hpx::Client` to fetch raw bytes, write to stdout/file, return
3. Create `BrowserContext` with global flags
4. Create `Page`, navigate with `--wait-until` condition
5. Apply process-level hard deadline (timeout + wait + 10s, daemon thread with `process::exit(124)`)
6. Post-load settle (`--wait` seconds, drive event loop)
7. If `--eval` → evaluate JS, return value directly if no dump/selector specified
8. If `--selector` → poll for CSS selector with timeout
9. Dump output in requested format (default: html)
10. Write to stdout or `--output` file

**`hpx scrape` flow:**

1. Validate at least one URL provided
2. Find `hpx-worker` binary next to `hpx` binary
3. Create semaphore with `--concurrency` limit
4. For each URL, spawn tokio task:
   a. Acquire semaphore permit
   b. Spawn `hpx-worker` subprocess with `OBSCURA_PROXY` env
   c. Write `navigate` command to stdin, read response from stdout
   d. Optionally write `evaluate` command
   e. Write `shutdown` command, wait for exit
   f. Collect result (url, title, eval, time_ms) or error
5. Collect all results, output as JSON or text

**`hpx serve` flow:**

1. If `--workers > 1` → spawn N worker processes on sequential ports, run TCP load balancer on main port with round-robin routing
2. If `--workers == 1` → start CDP server directly using hpx-browser's `protocol` module
3. CDP server listens on `--host:--port`, accepts WebSocket connections
4. Handle `/json/version`, `/json`, `/json/protocol` HTTP endpoints
5. Route CDP messages to hpx-browser page/session handlers

**`hpx mcp` flow:**

1. If `--http` → start HTTP transport (POST `/mcp`, SSE, CORS)
2. Else → start stdio transport (newline-delimited JSON over stdin/stdout)
3. MCP server handles: `initialize`, `ping`, `tools/list`, `tools/call`, `resources/list`, `prompts/list`
4. Each tool call delegates to hpx-browser Page methods
5. BrowserState tracks multi-tab, active tab, interactive refs, console messages

### 6.5 Configuration

New environment variables (matching obscura conventions):

- `HPX_TIMEZONE` — Override process timezone (default: system)
- `HPX_ALLOW_PRIVATE_NETWORK` — Allow loopback/RFC1918 fetches
- `HPX_V8_FLAGS` — Raw V8 flags
- `HPX_CDP_COMMAND_TIMEOUT_MS` — Per-CDP-command timeout (default: 60000)
- `HPX_SCRIPT_DEADLINE_MS` — Script execution deadline (default: 10000)

Existing env vars already supported: `HPX_PROXY`, `HPX_TIMEOUT`, `HPX_METHOD`, `HPX_REDIRECTS`, `HPX_FOLLOW`

### 6.6 Error Handling

- CLI handlers use `eyre::Result` (existing pattern)
- Worker subprocess errors bubble up as `eyre::Report` with context
- MCP tool errors return JSON-RPC error responses
- SSRF violations return clear error message with blocked IP/CIDR
- Timeout errors include elapsed time and configured limit

### 6.7 Maintainability Notes

- Each subcommand handler is a standalone async function in `browser.rs`
- Dump format rendering is a match statement, not a trait hierarchy
- Worker protocol types are shared between hpx-cli and hpx-worker via a shared module or inline definitions
- MCP tools defined as static data (name, description, schema), dispatch via match

---

## 7. Verification & Testing Strategy

### 7.1 Unit Testing

| Module | What to Test | Scope |
| :--- | :--- | :--- |
| `cli.rs` | New subcommand parsing: fetch, scrape, serve, mcp with all flags | proptest for flag combinations |
| `browser.rs` | Dump format rendering (html, text, links, markdown, assets, cookies) | Example-based |
| `hpx-worker` | Worker protocol serialization/deserialization | Round-trip tests |
| `hpx-mcp` | MCP JSON-RPC request/response parsing | Example-based |
| `hpx-browser::net::ssrf` | IP blocking: loopback, RFC1918, link-local, IPv4-mapped | Example + property |
| `hpx-browser::net::robots` | robots.txt parsing, allow/deny matching | Example-based |
| `hpx-browser::markdown` | HTML-to-Markdown conversion | Snapshot tests |

### 7.2 Property Testing

| Target Behavior | Why Property Testing Helps | Tool / Command | Planned Invariants |
| :--- | :--- | :--- | :--- |
| CLI flag parsing | Many flag combinations | `proptest` in hpx-cli tests | Parse never panics, required fields enforced |
| SSRF IP detection | Arbitrary IP strings | `proptest` | Never panics, private IPs always blocked |

### 7.3 Integration Testing

- `hpx fetch` against a local test server (axum) serving HTML with JS
- `hpx scrape` with multiple URLs against local server
- `hpx serve` with Puppeteer connecting via WebSocket
- `hpx mcp` with MCP client sending tools/list

### 7.4 BDD Acceptance Testing

| Scenario ID | Feature File | Command | Success Criteria |
| :--- | :--- | :--- | :--- |
| **BDD-01** | `features/fetch.feature` | `cargo test -p hpx-cli --test fetch` | All fetch scenarios pass |
| **BDD-02** | `features/scrape.feature` | `cargo test -p hpx-cli --test scrape` | All scrape scenarios pass |
| **BDD-03** | `features/serve.feature` | `cargo test -p hpx-cli --test serve` | All serve scenarios pass |
| **BDD-04** | `features/mcp.feature` | `cargo test -p hpx-mcp` | All MCP scenarios pass |

### 7.5 Robustness & Performance Testing

| Test Type | When It Is Required | Tool / Command | Planned Coverage or Reason Not Needed |
| :--- | :--- | :--- | :--- |
| **Fuzz** | N/A | N/A | CLI args parsed by clap, not raw input |
| **Benchmark** | N/A | N/A | CLI is not a hot path |

### 7.6 Critical Path Verification

| Verification Step | Command | Success Criteria |
| :--- | :--- | :--- |
| **VP-01** | `cargo check -p hpx-cli` | Compiles with default features |
| **VP-02** | `cargo check -p hpx-cli --features mcp,worker` | Compiles with all features |
| **VP-03** | `cargo test -p hpx-cli` | All unit/integration tests pass |
| **VP-04** | `cargo test -p hpx-mcp` | MCP tests pass |
| **VP-05** | `cargo clippy -p hpx-cli -- -D warnings` | No warnings |
| **VP-06** | `cargo clippy -p hpx-mcp -- -D warnings` | No warnings |
| **VP-07** | `just lint && just test && just test-all` | Full workspace passes |

---

## 8. Implementation Plan

- [ ] **Phase 1: CLI Foundation** — Add subcommands and global flags to cli.rs, create browser.rs handler stubs
- [ ] **Phase 2: Fetch & Scrape** — Implement fetch handler (all dump formats), scrape handler (worker subprocess), hpx-worker binary
- [ ] **Phase 3: Serve & MCP** — Wire CDP server to serve subcommand, create hpx-mcp crate, implement MCP tools
- [ ] **Phase 4: Supporting Features** — SSRF guard, robots.txt, tracker blocklist, HTML-to-Markdown, env vars
- [ ] **Phase 5: Testing & Polish** — Integration tests, BDD scenarios, clippy clean, docs
