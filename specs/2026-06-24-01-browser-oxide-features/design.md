# Design: Browser Oxide Feature Integration

| Metadata | Details |
| :--- | :--- |
| **Author** | pb-plan agent |
| **Status** | Revised |
| **Created** | 2026-06-24 |
| **Revised** | 2026-06-24 |
| **Reviewers** | N/A |
| **Related Issues** | N/A |

## 1. Executive Summary

**Problem:** hpx is a high-performance HTTP client with basic browser emulation (UA, Accept, sec-fetch headers + TLS/HTTP2 fingerprinting via hpx-emulation). However, it lacks the ability to render web pages and execute JavaScript locally — users must depend on external headless Chrome/Chromium for any JS-heavy page. It also lacks anti-bot challenge detection, behavioral humanization, advanced cookie management, CSP awareness, and the deep stealth profile system that browser_oxide implements.

**Solution:** Create a new `hpx-browser` crate in the hpx workspace that integrates ALL features from browser_oxide as a standalone Rust browser engine: (1) HTML/DOM parsing via html5ever, (2) CSS parsing and layout via taffy, (3) V8 JavaScript execution via deno_core, (4) Canvas 2D rendering via skia-safe, (5) anti-bot challenge detection + solver trait, (6) enhanced stealth profiles with behavioral humanization, (7) HTTP/2 + HTTP/3 (QUIC) networking with Chrome TLS fingerprint, (8) CSP enforcement, (9) Web Workers, (10) iframes, (11) CDP server for Puppeteer/Playwright interop, and (12) page pooling + parallel navigation. The goal: use hpx to parse, render, and interact with web pages without any dependency on headless Chrome.

---

## 2. Source Inputs & Normalization

### 2.1 Source Materials

- Full source of `browser_oxide` crate at `/Users/akagi201/tmp/browser_oxide/crates/browser_oxide/` (12,000+ lines across net/, stealth/, challenge.rs, classify.rs, page.rs, event_loop/, js_runtime/)
- Full source of hpx workspace (hpx, hpx-emulation, hpx-yawc crates)
- User requirement: "深度分析与参考 browser_oxide 把他的所有特性吸收集成到 hpx crate 里面来"

### 2.2 Normalization Approach

The user's requirement is to "absorb all features" from browser_oxide into hpx, with the explicit goal of eliminating the dependency on headless Chrome for web page rendering and JS execution. After deep analysis, browser_oxide is a full browser engine (12,000+ lines) covering:

1. **Networking layer:** Stealth HTTP client with BoringSSL TLS, JA4 fingerprint, HTTP/2 + HTTP/3 (QUIC), cookie jar, CSP enforcement, proxy support
2. **Browser engine:** HTML parsing (html5ever), DOM tree (arena-allocated), CSS parsing/layout (taffy), V8 JS execution (deno_core), Canvas rendering (skia-safe), Web Workers, iframes
3. **Stealth layer:** Anti-bot challenge detection, behavioral humanization (Sigma-Lognormal mouse/keystroke/scroll), stealth profiles with 50+ fields, browser presets
4. **Interop layer:** CDP server for Puppeteer/Playwright driving

All four layers will be ported into a new `hpx-browser` crate within the hpx workspace. The existing hpx HTTP client crate remains unchanged — `hpx-browser` depends on hpx for networking and adds the browser engine on top.

### 2.3 Source Requirement Ledger

| Requirement ID | Source Summary | Type | Notes |
| :--- | :--- | :--- | :--- |
| `R1` | Anti-bot challenge detection for Cloudflare, DataDome, Akamai, Kasada, PerimeterX, AWS-WAF | Functional | From classify.rs (754 lines) |
| `R2` | Pluggable ChallengeSolver trait for vendor-specific anti-bot bypass | Functional | From challenge.rs (220 lines) |
| `R3` | Enhanced stealth profiles with 50+ fields (GPU, media devices, WebAuthn, behavioral params) | Functional | From stealth/profile.rs + presets.rs |
| `R4` | Behavioral humanization: Sigma-Lognormal mouse trajectories, keystroke dynamics, scroll modeling | Functional | From stealth/behavior.rs (873 lines) |
| `R5` | Region-aware header generation with TLD-based accept-language | Functional | From net/headers.rs (1500 lines) |
| `R6` | Improved cookie jar with domain scoping, RFC 6265 compliance, atomic persistence, collision scrub | Functional | From net/cookies.rs (597 lines) |
| `R7` | CSP3 parser and enforcement (strict-dynamic, nonce/hash sources, fallback chains) | Functional | From net/csp.rs (1137 lines) |
| `R8` | HTTP/2 fingerprint matching (initial_window_size, max_concurrent_streams, pseudo-header order) | Functional | From hpx-emulation fingerprint types |
| `R9` | Critical-CH retry (re-request with high-entropy Client Hints when server demands) | Functional | From net/mod.rs |
| `R10` | H1 memory (remember hosts that negotiated http/1.1 to skip doomed H2 attempts) | Functional | From net/mod.rs |
| `R11` | Stale-connection recovery (evict + retry on GOAWAY) | Functional | From net/mod.rs |
| `R12` | Browser presets: Chrome 148, Firefox 135, Safari 18, with regional variants | Functional | From stealth/presets.rs (1281 lines) |
| `R13` | TLS fingerprint: Chrome 147 cipher suite order, extension permutation, post-quantum curves | Functional | From net/tls.rs (835 lines) |
| `R14` | Challenge-aware navigation loop with cookie-diff retry and budget management | Functional | From page.rs navigate loop |
| `R15` | Page pool and parallel pager for batch scraping | Functional | From pool.rs, parallel.rs |
| `R16` | Full browser engine: HTML/DOM parsing (html5ever), CSS layout (taffy), V8 JS execution (deno_core) | Functional | From dom/, css_parser/, layout/, js_runtime/ |
| `R17` | CDP server for Puppeteer/Playwright driving | Functional | From protocol/ |
| `R18` | Canvas 2D rendering via skia-safe, WebGL stubs, AudioContext fingerprint | Functional | From canvas/ |
| `R19` | Web Workers (each in own V8 isolate) with postMessage/onmessage | Functional | From workers/ |
| `R20` | Iframe support (srcdoc + src) with isolated DOM/V8/CSP | Functional | From iframe.rs |
| `R21` | HTTP/3 (QUIC) via quinn + h3 with connection pooling | Functional | From net/quic.rs, net/h3_request.rs |
| `R22` | Browser event loop: V8 execution, timers, requestAnimationFrame, idle detection | Functional | From event_loop/ |
| `R23` | navigate() loop: fetch → CSP install → build → challenge loop → solver dispatch | Functional | From page.rs navigate_with_init_solvers() |

---

## 3. Requirements & Goals

### 3.1 Problem Statement

hpx has solid HTTP client foundations and basic browser emulation, but cannot render web pages or execute JavaScript locally. Users scraping JS-heavy sites (SPAs, anti-bot challenges) must depend on headless Chrome/Chromium, Playwright, or Puppeteer — heavy external dependencies that are hard to deploy in constrained environments (serverless, containers, edge). The hpx-emulation crate has 130+ browser version variants but thin profile data (just UA + headers + TLS/HTTP2 settings).

### 3.2 Functional Goals

1. **HTML/DOM parsing:** Port html5ever-based HTML parser with arena-allocated DOM tree, Shadow DOM support, tree mutation operations
2. **CSS parsing and layout:** Port CSS tokenizer, selector matching, cascade, computed style resolution, taffy-based Block/Flexbox/Grid layout
3. **V8 JavaScript execution:** Port deno_core-based JS runtime with 16 DOM extension modules, ES module support, document.readyState lifecycle
4. **Canvas 2D rendering:** Port skia-safe Canvas 2D context, WebGL parameter stubs, AudioContext fingerprint, font shaping
5. **Browser event loop:** Port V8 execution driver with timers, requestAnimationFrame, idle detection, promise resolution
6. **Challenge detection API:** Port `engine_classify()` and `ChallengeVerdict` enum
7. **ChallengeSolver trait:** Port the solver trait for pluggable anti-bot bypass strategies
8. **StealthProfile struct:** Create rich profile type with 50+ fields
9. **Browser presets:** Port Chrome 148, Firefox 135, Safari 18 presets with regional variants
10. **Behavioral humanization:** Port Sigma-Lognormal mouse trajectories, keystroke dynamics, scroll burst
11. **Region-aware headers:** Port TLD-based accept-language dispatch
12. **Cookie jar improvements:** Port domain-scoped jar with RFC 6265 compliance, persistence, collision scrub
13. **CSP parser:** Port CSP3 policy parser and enforcement
14. **HTTP/2 fingerprint:** Extend with structured fingerprint types
15. **HTTP/3 (QUIC):** Port quinn + h3 integration with connection pooling
16. **Critical-CH retry:** Add Accept-CH tracking and high-entropy CH re-request
17. **H1 memory:** Track hosts that negotiated http/1.1
18. **Web Workers:** Port worker-in-isolate with postMessage/onmessage
19. **Iframes:** Port srcdoc/src iframe support with isolated DOM/V8/CSP
20. **CDP server:** Port Chrome DevTools Protocol WebSocket server
21. **Page pool + parallel pager:** Port warm V8 isolate pool and OS-thread worker pool
22. **navigate() loop:** Port the full challenge-aware navigation loop (fetch → CSP → build → challenge → solver)

### 3.3 Non-Functional Goals

- **Performance:** Challenge detection O(n) scan. Behavioral generation deterministic (ChaCha20Rng). Cookie jar uses scc::HashMap. Arena-allocated DOM for O(1) node access. V8 isolate pooling for ~150ms savings per page.
- **Compatibility:** All browser engine features in new `hpx-browser` crate. Existing hpx HTTP client APIs unchanged. Feature flags for optional heavy deps (skia, deno_core).
- **Zero Chromium dependency:** No headless Chrome, no Playwright browser binary, no CDP by default. The entire stack is Rust + V8 (via deno_core).
- **Test coverage:** Property tests for CSP parser, example-based for challenge detection, byte-parity tests for canvas/PNG output.

### 3.4 Out of Scope

- adblock/blocker integration (separate concern, optional feature in browser_oxide)
- Full WebGL rendering via OSMesa (optional feature, stubs sufficient for fingerprinting)
- Mobile-specific optimizations (deferred)

### 3.5 Assumptions

- hpx remains the core HTTP client library. A new `hpx-browser` crate adds the browser engine on top of hpx.
- browser_oxide's BoringSSL config (cipher lists, curves, sigalgs) maps to hpx's existing `TlsOptions` struct.
- deno_core 0.403 is compatible with the hpx workspace's tokio version (1.52+).
- skia-safe 0.97 can be built in the hpx workspace (heavy build dep, gated behind feature flag).
- html5ever, taffy, and other parsing/layout deps are lightweight and can be always-on.
- The `hpx-browser` crate will be the integration target, NOT modifying the existing `hpx` crate with browser engine internals.

### 3.6 Code Simplification Constraints

- **Behavior Preservation Boundary:** All existing hpx APIs remain unchanged. New features are additive only.
- **Repo Standards:** Follow AGENTS.md: `scc` over `dashmap`, `winnow` for parsing, `thiserror` for library errors, `tracing` for logging, `proptest` for property tests, no `unsafe`, clippy::pedantic.
- **Readability Priorities:** Explicit control flow, named constants over magic numbers, no nested ternaries.
- **Refactoring Non-Goals:** Do not restructure existing hpx-emulation device modules unless a task explicitly requires it.
- **Clarity Guardrails:** Challenge detection logic should be a clear if-else chain with named marker constants, not a clever trie or regex engine.

---

## 4. Requirements Coverage Matrix

| Requirement ID | Covered In Design | Scenario Coverage | Task Coverage | Status / Rationale |
| :--- | :--- | :--- | :--- | :--- |
| `R1` | §6.2, §6.4 | Challenge detection scenarios | Task 2.1, 2.2 | Covered |
| `R2` | §6.3 | Solver trait scenarios | Task 2.3 | Covered |
| `R3` | §6.2 | Stealth profile scenarios | Task 3.1, 3.2 | Covered |
| `R4` | §6.2 | Humanization scenarios | Task 3.3, 3.4 | Covered |
| `R5` | §6.4 | Header generation scenarios | Task 4.1 | Covered |
| `R6` | §6.2 | Cookie jar scenarios | Task 4.2 | Covered |
| `R7` | §6.2 | CSP parsing scenarios | Task 4.3 | Covered |
| `R8` | §6.2 | HTTP/2 fingerprint scenarios | Task 5.1 | Covered |
| `R9` | §6.4 | Critical-CH retry scenarios | Task 5.2 | Covered |
| `R10` | §6.4 | H1 memory scenarios | Task 5.3 | Covered |
| `R11` | §6.4 | Stale connection recovery | Task 5.3 | Covered |
| `R12` | §6.2 | Browser preset scenarios | Task 3.2 | Covered |
| `R13` | §6.2 | TLS fingerprint scenarios | Task 5.1 | Covered |
| `R14` | §6.4 | Challenge-aware request | Task 6.1 | Covered |
| `R15` | §6.2 | Page pool / parallel pager | Task 7.1 | Covered |
| `R16` | §6.2 | DOM/CSS/JS engine | Task 8.1-8.5 | Covered |
| `R17` | §6.3 | CDP server | Task 9.1 | Covered |
| `R18` | §6.2 | Canvas/WebGL/Audio | Task 8.6 | Covered |
| `R19` | §6.2 | Web Workers | Task 8.7 | Covered |
| `R20` | §6.2 | Iframes | Task 8.8 | Covered |
| `R21` | §6.2 | HTTP/3 (QUIC) | Task 7.2 | Covered |
| `R22` | §6.2 | Browser event loop | Task 8.3 | Covered |
| `R23` | §6.4 | navigate() loop | Task 6.2 | Covered |

---

## 5. Architecture Overview

### 5.1 System Context

```text
┌──────────────────────────────────────────────────────────────┐
│                    hpx-browser crate (NEW)                     │
│                                                               │
│  ┌─────────────┐  ┌──────────────┐  ┌──────────────────────┐ │
│  │  Page        │  │  JS Runtime  │  │  DOM + CSS Layout    │ │
│  │  (navigate,  │  │  (deno_core  │  │  (html5ever + taffy) │ │
│  │   evaluate)  │  │   + 16 exts) │  │                      │ │
│  └──────┬──────┘  └──────┬───────┘  └──────────┬───────────┘ │
│         │                │                      │             │
│  ┌──────┴──────┐  ┌──────┴───────┐  ┌──────────┴───────────┐ │
│  │  Event Loop │  │  Canvas 2D   │  │  Challenge Detection │ │
│  │  (timers,   │  │  (skia-safe) │  │  + Solver Trait      │ │
│  │   rAF, idle)│  │              │  │                      │ │
│  └──────┬──────┘  └──────────────┘  └──────────────────────┘ │
│         │                                                     │
│  ┌──────┴──────┐  ┌──────────────┐  ┌──────────────────────┐ │
│  │  Workers    │  │  Iframes     │  │  CDP Server          │ │
│  │  (V8 iso)   │  │  (isolated)  │  │  (Puppeteer compat)  │ │
│  └─────────────┘  └──────────────┘  └──────────────────────┘ │
│                                                               │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │  Stealth Layer: Profile + Presets + Humanization + Headers│ │
│  └─────────────────────────────────────────────────────────┘ │
├──────────────────────────────────────────────────────────────┤
│                      hpx crate (existing)                     │
│  ┌───────────┐  ┌───────────┐  ┌───────────┐  ┌───────────┐ │
│  │  Client    │  │  TLS      │  │  Cookie   │  │  Proxy    │ │
│  │  (Tower    │  │  (Boring/ │  │  Jar      │  │  Pool     │ │
│  │   stack)   │  │   Rustls) │  │           │  │           │ │
│  └───────────┘  └───────────┘  └───────────┘  └───────────┘ │
├──────────────────────────────────────────────────────────────┤
│                    hpx-emulation crate                        │
│  ┌───────────┐  ┌───────────┐  ┌──────────────────────────┐ │
│  │  TLS      │  │  HTTP/2   │  │  Browser Presets (130+)  │ │
│  │  Fingerprint│ │  Fingerprint│ │                          │ │
│  └───────────┘  └───────────┘  └──────────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
```

### 5.2 Key Design Principles

- **New crate, not mutation:** Browser engine lives in `hpx-browser`, not inside `hpx`. The HTTP client stays lean.
- **Feature-gated heavy deps:** deno_core (V8), skia-safe (canvas) behind feature flags. html5ever and taffy are lightweight and always-on.
- **Library layer:** New modules use `thiserror` for errors, `tracing` for logging.
- **scc-first:** Concurrent data structures use `scc::HashMap` per AGENTS.md.
- **Winnow for parsing:** CSP parser uses `winnow` per AGENTS.md preference.
- **Deterministic humanization:** All behavioral generation seeded via ChaCha20Rng for reproducibility.
- **Arena DOM:** O(1) node access via NodeId (Copy type), no Rc/RefCell overhead.

### 5.3 Existing Components to Reuse

| Component | Location | How to Reuse |
| :--- | :--- | :--- |
| `hpx::Client` | `crates/hpx/src/client/http.rs` | Use as the HTTP backend for all page.fetch() calls |
| `TlsOptions` | `crates/hpx/src/tls/options.rs` | Extend with curves_list, cipher_list, sigalgs_list, extension_permutation, certificate_compression |
| `Emulation` | `crates/hpx/src/client/emulation.rs` | Extend with StealthProfile integration |
| `BrowserProfile` | `crates/hpx/src/client/emulation.rs` | Map to StealthProfile presets |
| `OrigHeaderMap` | `crates/hpx/src/header.rs` | Use for case-preserving header composition |
| `CookieStore` / `Jar` | `crates/hpx/src/cookie.rs` | Extend with domain-scoped storage, persistence |
| `TlsFingerprint` | `crates/hpx-emulation/src/fingerprint/mod.rs` | Already has structured TLS fingerprint types |
| `Http2Fingerprint` | `crates/hpx-emulation/src/fingerprint/mod.rs` | Already has HTTP/2 fingerprint struct |
| `hpx-yawc` | `crates/yawc/` | Use for WebSocket support in browser engine |
| `scc::HashMap` | workspace dep | Use for concurrent cookie jar, DOM arena |
| `rand_chacha` | workspace dep | Use for deterministic humanization RNG |
| `html5ever` | browser_oxide dep | HTML parsing (always-on in hpx-browser) |
| `deno_core` | browser_oxide dep | V8 JS execution (feature-gated) |
| `skia-safe` | browser_oxide dep | Canvas 2D rendering (feature-gated) |
| `taffy` | browser_oxide dep | CSS Block/Flexbox/Grid layout (always-on) |
| `quinn` + `h3` | browser_oxide dep | HTTP/3 QUIC support (feature-gated) |

### 5.4 Architecture Decisions

| Decision ID | Status | Selected Pattern / Principle | Why It Fits Here | Alternatives Rejected | Simplification Impact |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `AD-01` | New | Strategy | ChallengeSolver trait allows pluggable anti-bot bypass without modifying hpx core | Hard-coded solver list would require hpx changes per vendor | Users can implement custom solvers without forking |
| `AD-02` | New | Adapter | StealthProfile adapts to existing Emulation/TlsOptions via `From` impls | Duplicating config would split the API | Single profile drives all fingerprint layers |
| `AD-03` | Inherited | Builder | StealthProfile uses builder pattern matching existing ClientBuilder/EmulationBuilder | Direct construction would require 50+ positional args | Consistent with hpx API style |
| `AD-04` | New | Strategy | Region header dispatch uses trait-based browser family selection | if-else chain on browser name string | Extensible to new browsers |
| `AD-05` | Inherited | SRP-only split | Challenge detection, CSP parsing, cookie jar, DOM, CSS, JS are separate modules | Monolithic module would be 15,000+ lines | Each module testable independently |
| `AD-06` | New | Separate crate | Browser engine in `hpx-browser`, not inside `hpx` | Stuffing DOM/V8/Canvas into hpx would bloat the HTTP client | hpx stays lean; browser engine is opt-in |

- **Architecture Decision Snapshot Inputs:** AGENTS.md mandates `scc` over `dashmap`, `winnow` for parsing, `thiserror` for library errors, `tracing` only, no `unsafe`.
- **SRP Check:** Each new module (challenge, classify, csp, humanize, profile) has one responsibility. The Client gains `send_with_challenge_detection()` but delegates to the challenge module.
- **DIP Check:** ChallengeSolver is a trait — the Client depends on the abstraction, not concrete solvers. CookieStore is already a trait in hpx.
- **Dependency Injection Plan:** Challenge solvers injected via `ClientBuilder::challenge_solvers(Vec<Arc<dyn ChallengeSolver>>)`. CSP enforcement injected via `ClientBuilder::enforce_csp(bool)`.
- **Code Simplifier Alignment:** Named constants for challenge markers (not magic strings). Clear if-else classification chain (not regex). Builder pattern for profiles (not positional constructors).

### 5.5 Project Identity Alignment

No template identity mismatches detected. The hpx workspace uses project-matching crate names.

### 5.6 BDD/TDD Strategy

- **BDD Runner:** `cucumber` (Rust)
- **BDD Command:** `cargo test --test challenge_features` (cucumber-rs colocated tests)
- **Unit Test Command:** `cargo test -p hpx --features challenge,stealth-profile,humanize,csp`
- **Property Test Tool:** `proptest` for CSP parser (large input domain: directive/source combinations)
- **Fuzz Test Tool:** `cargo-fuzz` for CSP parser (untrusted HTTP headers, crash-safety)
- **Benchmark Tool:** `criterion` for challenge detection (hot path on every response)
- **Outer Loop:** Challenge detection and CSP parsing scenarios prove end-to-end behavior
- **Inner Loop:** Unit tests for marker constants, profile validation, humanization determinism
- **Step Definition Location:** `crates/hpx/tests/` (colocated with existing integration tests)

### 5.7 BDD Scenario Inventory

| Feature File | Scenario | Business Outcome | Primary Verification | Supporting TDD Focus |
| :--- | :--- | :--- | :--- | :--- |
| `features/challenge-detection.feature` | Detect Cloudflare challenge | Client identifies CF managed challenge from response body | classify::engine_classify returns correct tag | Marker constant matching |
| `features/challenge-detection.feature` | Detect DataDome challenge | Client identifies DataDome captcha from response body | classify::engine_classify returns correct tag | Edge cases: multilingual |
| `features/challenge-detection.feature` | Pass clean response | Clean page returns Pass verdict | classify::engine_classify returns Pass | False positive regression |
| `features/stealth-profile.feature` | Validate Chrome 148 profile | Profile passes all consistency checks | profile.validate() returns Ok | Cross-field validation |
| `features/stealth-profile.feature` | Random desktop profile selection | random_desktop() returns valid profile | Different profiles on repeated calls | Seed determinism |
| `features/humanization.feature` | Generate mouse trajectory | Trajectory has smooth velocity curve | Points are monotonically approaching target | Endpoint accuracy |
| `features/humanization.feature` | Generate keystroke timings | Timings vary per keystroke | Dwell/flight times are positive, vary | Bigram modulation |
| `features/csp-parsing.feature` | Parse strict-dynamic policy | strict-dynamic ignores host sources | Policy.allows() correct for nonce/hash | Parser property tests |
| `features/cookie-jar.feature` | Domain-scoped cookie storage | Cookies scoped by domain attribute | cookies_for() returns correct set | RFC 6265 compliance |

### 5.8 Simplification Opportunities in Touched Code

| Area | Current Complexity or Smell | Planned Simplification | Why It Preserves or Clarifies Behavior |
| :--- | :--- | :--- | :--- |
| `hpx-emulation` device modules | Macro-heavy per-version profile generation | Keep macros, add StealthProfile adapter layer on top | Preserves existing 130+ variants while adding richer profile data |
| `hpx/src/cookie.rs` | CookieJar uses `Arc<SccHashMap<String, SccHashMap<String, CookieJar>>>` | Add domain-scoped CookieJar2 with persistence behind feature flag | Existing Jar unchanged, new Jar2 opt-in |

---

## 6. Detailed Design

### 6.1 Module Structure

```text
crates/hpx-browser/                    # NEW — standalone browser engine crate
├── Cargo.toml                         # deps: hpx, html5ever, deno_core, skia-safe, taffy, etc.
├── src/
│   ├── lib.rs                         # Crate root, re-exports Page, EngineHandle, etc.
│   ├── page.rs                        # Page struct — top-level API (navigate, evaluate, title, content)
│   ├── event_loop/
│   │   ├── mod.rs                     # BrowserEventLoop — V8 execution, timers, rAF, idle detection
│   │   ├── timer.rs                   # setTimeout/setInterval management
│   │   └── raf.rs                     # requestAnimationFrame
│   ├── js_runtime/
│   │   ├── mod.rs                     # BrowserJsRuntime wrapping deno_core::JsRuntime
│   │   ├── dom_ext.rs                 # DOM bindings (document.*, createElement, querySelector, etc.)
│   │   ├── fetch_ext.rs               # fetch() API binding to hpx::Client
│   │   ├── timer_ext.rs              # setTimeout/setInterval/clearTimeout/clearInterval
│   │   ├── console_ext.rs            # console.log/warn/error
│   │   ├── crypto_ext.rs             # crypto.getRandomValues, crypto.subtle
│   │   ├── storage_ext.rs            # localStorage/sessionStorage
│   │   ├── canvas_ext.rs             # Canvas 2D context bindings
│   │   ├── input_ext.rs              # Input event dispatch (click, type, scroll)
│   │   ├── layout_ext.rs             # getBoundingClientRect, getComputedStyle
│   │   ├── navigation_ext.rs         # window.location, history.pushState
│   │   ├── stealth_ext.rs            # Stealth injection (override navigator, screen, etc.)
│   │   ├── sse_ext.rs                # Server-Sent Events
│   │   ├── websocket_ext.rs          # WebSocket bindings
│   │   ├── performance_ext.rs        # performance.now(), PerformanceObserver
│   │   └── worker_ext.rs             # Worker/SharedWorker constructors
│   ├── dom/
│   │   ├── mod.rs                     # Arena-allocated DOM tree, NodeId
│   │   ├── node.rs                    # Node types (Element, Text, Comment, Document)
│   │   ├── element.rs                 # Element with attributes, tag name
│   │   ├── mutation.rs                # appendChild, insertBefore, detach, remove
│   │   └── shadow_dom.rs             # Shadow DOM support
│   ├── css_parser/
│   │   ├── mod.rs                     # CSS tokenizer + parser
│   │   ├── tokenizer.rs              # CSS token types
│   │   └── parser.rs                 # Rule/declaration parsing
│   ├── css_selectors/
│   │   ├── mod.rs                     # Selector matching (combinators, pseudo-classes, nth-child)
│   │   └── matching.rs              # Element trait impl for selector matching
│   ├── css_values/
│   │   ├── mod.rs                     # CSS value types
│   │   └── computed.rs              # Computed style resolution
│   ├── css_cascade/
│   │   ├── mod.rs                     # Cascade + inheritance + layers + media queries
│   │   └── cascade.rs               # Style cascade logic
│   ├── layout/
│   │   ├── mod.rs                     # taffy-based CSS layout engine
│   │   ├── block.rs                  # Block layout
│   │   ├── flex.rs                   # Flexbox layout
│   │   └── grid.rs                   # Grid layout
│   ├── canvas/
│   │   ├── mod.rs                     # Canvas 2D rendering via skia-safe
│   │   ├── context_2d.rs             # CanvasRenderingContext2D
│   │   ├── webgl.rs                  # WebGL parameter stubs with per-GPU profiles
│   │   ├── audio.rs                  # AudioContext fingerprint (DynamicsCompressor)
│   │   └── font.rs                   # Font shaping (rustybuzz + swash)
│   ├── net/
│   │   ├── mod.rs                     # HttpClient (stealth HTTP, multi-protocol)
│   │   ├── tls.rs                    # TLS fingerprinting (Chrome 147 cipher suite order)
│   │   ├── headers.rs                # Browser-aware header generation
│   │   ├── cookies.rs                # Domain-scoped cookie jar
│   │   ├── csp.rs                    # CSP3 parser + enforcement
│   │   ├── h2_client.rs             # HTTP/2 client
│   │   ├── h1_client.rs             # HTTP/1.1 fallback
│   │   ├── h3_request.rs            # HTTP/3 (QUIC) request
│   │   ├── quic.rs                   # Quinn QUIC connection pool
│   │   ├── pool.rs                   # H2 connection pool
│   │   └── proxy.rs                  # Proxy support
│   ├── stealth/
│   │   ├── mod.rs                     # Stealth profiles, presets, behavior
│   │   ├── profile.rs                # StealthProfile (50+ fields)
│   │   ├── presets.rs                # Chrome 148, Firefox 135, Safari 18 presets
│   │   ├── behavior.rs               # Sigma-Lognormal mouse/keystroke/scroll
│   │   ├── gpu.rs                    # GpuProfile
│   │   └── headers.rs               # Region-aware header generation
│   ├── challenge/
│   │   ├── mod.rs                     # ChallengeSolver trait, SolveOutcome
│   │   ├── classify.rs               # engine_classify() — anti-bot detection
│   │   └── verdict.rs                # ChallengeVerdict enum
│   ├── csp/
│   │   ├── mod.rs                     # CSP3 types
│   │   ├── parser.rs                 # winnow-based CSP parser
│   │   └── check.rs                  # PolicySet::allows()
│   ├── workers/
│   │   ├── mod.rs                     # Web Worker management
│   │   └── worker.rs                 # Worker in own V8 isolate
│   ├── iframe.rs                      # Iframe support (srcdoc + src, isolated DOM/V8/CSP)
│   ├── protocol/
│   │   ├── mod.rs                     # CDP server (Chrome DevTools Protocol)
│   │   └── ws.rs                     # WebSocket CDP handler
│   ├── pool.rs                        # PagePool — warm V8 isolate pool
│   ├── parallel.rs                    # ParallelPager — OS-thread worker pool
│   ├── host/
│   │   └── mod.rs                     # EngineHandle — Send+Sync handle over !Send engine
│   └── csp_collector.rs              # Meta-tag CSP collection
```

```text
crates/hpx/src/                       # MODIFIED — minimal changes to existing hpx crate
├── client/
│   └── emulation.rs                   # MODIFIED — extend Emulation with StealthProfile adapter
└── tls/
    └── options.rs                     # MODIFIED — add extension_permutation, cert_compression fields
```

### 6.2 Data Structures & Types

```rust
// === challenge/verdict.rs ===

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChallengeVerdict {
    Pass,
    RenderIncomplete,
    EdgeBlock,
    SensorFail,
    ThinShell,
    ChallengeIncomplete,
}

// === challenge/classify.rs ===

#[derive(Debug, Clone)]
pub struct EngineClass {
    pub tag: &'static str,
    pub verdict: ChallengeVerdict,
    pub len: usize,
}

// === challenge/mod.rs ===

#[derive(Debug, Clone)]
pub struct ChallengeKind {
    pub vendor: &'static str,
    pub sub_kind: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolveOutcome {
    NotApplicable,
    InProgress,
    Solved,
    Unsolvable,
}

#[async_trait(?Send)]
pub trait ChallengeSolver: Send + Sync {
    fn name(&self) -> &'static str;
    async fn observe_response(&self, host: &str, resp: &Response) {}
    fn prepare_request(&self, host: &str, headers: &mut Vec<(String, String)>) {}
    fn detect(&self, body: &str) -> Option<ChallengeKind> { None }
    fn solved_signal(&self, cookies: &str, body: &str) -> bool { false }
}

// === stealth/profile.rs ===

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceClass {
    #[default]
    Desktop,
    MobileAndroid,
    MobileIOS,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StealthProfile {
    // Identity
    pub user_agent: String,
    pub browser_name: String,
    pub browser_version: String,
    pub os_name: String,
    pub os_version: String,
    pub platform: String,
    pub vendor: String,
    // Hardware
    pub screen_width: u32,
    pub screen_height: u32,
    pub device_pixel_ratio: f64,
    pub cpu_cores: u8,
    pub device_memory: u8,
    pub max_touch_points: u8,
    // GPU/WebGL
    pub webgl_vendor: String,
    pub webgl_renderer: String,
    pub gpu_profile: GpuProfile,
    // Locale
    pub language: String,
    pub languages: Vec<String>,
    pub timezone: String,
    // Client Hints
    pub cpu_architecture: String,
    pub cpu_bitness: String,
    pub platform_version: String,
    // Network
    pub device_class: DeviceClass,
    pub connection_effective_type: String,
    // Behavioral
    pub behavior: BehaviorProfile,
    // TLS
    pub tls_impersonate: String,
    // Fingerprint seeds
    pub canvas_seed: u64,
    pub audio_seed: u64,
    pub audio_sample_rate: u32,
    // Media
    pub prefers_color_scheme: String,
    pub pointer_type: String,
    pub hover_capability: String,
    // CSP
    pub enforce_csp: bool,
}

// === stealth/behavior.rs ===

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorProfile {
    pub seed: u64,
    pub handedness: Handedness,
    pub mouse_dpi: u16,
    pub typing_wpm_mean: f32,
    pub typing_wpm_sigma: f32,
    pub scroll_style: ScrollStyle,
    pub fitts_b: f32,
}

#[derive(Debug, Clone)]
pub struct MousePoint {
    pub t_ms: f32,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone)]
pub struct KeystrokeTiming {
    pub ch: char,
    pub dwell_ms: f32,
    pub flight_ms: f32,
}

#[derive(Debug, Clone)]
pub struct WheelTick {
    pub t_ms: f32,
    pub delta_y: f32,
    pub mode: u32,
}

// === csp/types.rs ===

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Directive {
    DefaultSrc, ScriptSrc, ScriptSrcElem, ScriptSrcAttr,
    StyleSrc, StyleSrcElem, StyleSrcAttr, ImgSrc, ConnectSrc,
    FrameSrc, ChildSrc, FontSrc, MediaSrc, ObjectSrc,
    WorkerSrc, ManifestSrc, PrefetchSrc,
}

#[derive(Debug, Clone)]
pub enum Source {
    All, None_, Self_, UnsafeInline, UnsafeEval, UnsafeHashes,
    StrictDynamic, ReportSample, Scheme(String),
    Host(HostSource), Nonce(String), Hash(HashAlgo, String),
}

#[derive(Debug, Clone)]
pub struct Policy {
    pub directives: HashMap<Directive, Vec<Source>>,
    pub report_only: bool,
}

#[derive(Debug, Clone)]
pub struct PolicySet {
    pub policies: Vec<Policy>,
}

// === dom/node.rs ===

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub u32);  // Arena index, Copy type, O(1) access

pub enum NodeType {
    Document,
    Element(ElementData),
    Text(String),
    Comment(String),
}

pub struct ElementData {
    pub tag: Atom,           // interned tag name
    pub attributes: Vec<(Atom, String)>,
    pub id: Option<String>,
    pub classes: Vec<Atom>,
}

// === dom/mod.rs ===

pub struct Dom {
    nodes: Vec<NodeType>,          // arena: index = NodeId.0
    parents: Vec<Option<NodeId>>,
    children: Vec<Vec<NodeId>>,
    shadow_roots: HashMap<NodeId, NodeId>,
}

// === js_runtime/mod.rs ===

pub struct BrowserJsRuntime {
    runtime: deno_core::JsRuntime,
    extensions: Vec<deno_core::Extension>,
}

// === event_loop/mod.rs ===

pub struct BrowserEventLoop {
    js: BrowserJsRuntime,
    dom: Dom,
    timers: TimerQueue,
    raf_callbacks: Vec<JsFunction>,
    idle_reason: IdleReason,
}

pub enum IdleReason {
    AllWorkDone,
    Timeout,
}

// === page.rs ===

pub struct Page {
    children: Vec<ChildIframe>,
    event_loop: BrowserEventLoop,
    url: String,
    solvers: Arc<[Arc<dyn ChallengeSolver>]>,
}
```

### 6.3 Interface Design

```rust
// === Client extension (client/http.rs) ===

impl ClientBuilder {
    /// Register challenge solvers for anti-bot bypass.
    pub fn challenge_solvers(self, solvers: Vec<Arc<dyn ChallengeSolver>>) -> Self;

    /// Enable CSP enforcement on requests.
    pub fn enforce_csp(self, enabled: bool) -> Self;
}

impl Client {
    /// Send request with challenge detection. Returns ChallengeOutcome
    /// indicating whether the response is clean or an anti-bot challenge.
    pub async fn send_with_challenge_detection(
        &self,
        request: Request,
    ) -> Result<ChallengeOutcome>;

    /// Get the stealth profile for this client.
    pub fn stealth_profile(&self) -> Option<&StealthProfile>;
}

#[derive(Debug)]
pub enum ChallengeOutcome {
    Clean(Response),
    Challenge { verdict: ChallengeVerdict, kind: Option<ChallengeKind>, response: Response },
}

// === StealthProfile extension ===

impl StealthProfile {
    pub fn validate(&self) -> Result<(), Vec<String>>;
    pub fn to_emulation(&self) -> Emulation;
    pub fn to_tls_options(&self) -> TlsOptions;
}

// === Humanization functions ===

pub fn mouse_trajectory(from: (f32, f32), to: (f32, f32), target_w: f32, profile: &BehaviorProfile) -> Vec<MousePoint>;
pub fn keystroke_timings(text: &str, profile: &BehaviorProfile) -> Vec<KeystrokeTiming>;
pub fn wheel_burst(target_dy: f32, profile: &BehaviorProfile) -> Vec<WheelTick>;

// === CSP ===

impl Policy {
    pub fn parse(header_value: &str, report_only: bool) -> Self;
}

impl PolicySet {
    pub fn allows(&self, ctx: &CheckCtx) -> AllowDecision;
}

// === Challenge detection ===

pub fn engine_classify(body: &str) -> EngineClass;
pub fn is_managed_challenge_doc(body: &str) -> bool;

// === Page API (hpx-browser crate) ===

impl Page {
    // Construction
    pub async fn navigate(url: &str, profile: &StealthProfile, max_iter: usize) -> Result<Self>;
    pub async fn navigate_with_solvers(url: &str, profile: &StealthProfile, max_iter: usize, solvers: Vec<Arc<dyn ChallengeSolver>>) -> Result<Self>;
    pub fn from_html(html: &str, profile: &StealthProfile) -> Result<Self>;
    pub fn from_html_with_url(html: &str, url: &str, profile: &StealthProfile) -> Result<Self>;

    // JS execution
    pub fn evaluate(&mut self, js: &str) -> Result<String>;
    pub async fn evaluate_async(&mut self, js: &str, timeout: Duration) -> Result<IdleReason>;

    // DOM queries
    pub fn title(&mut self) -> String;
    pub fn content(&mut self) -> String;               // documentElement.outerHTML
    pub fn text_content(&mut self) -> String;           // body.textContent
    pub fn text_of(&mut self, selector: &str) -> Option<String>;
    pub fn has_element(&mut self, selector: &str) -> bool;

    // Human interaction
    pub fn human_click(&mut self, selector: &str) -> Result<String>;
    pub fn human_type(&mut self, selector: &str, text: &str) -> Result<String>;

    // Challenge detection
    pub fn challenge_verdict(&mut self) -> ChallengeVerdict;
    pub fn is_anti_bot_challenge(&mut self) -> bool;

    // Lifecycle
    pub fn reload_html(&mut self, html: &str, url: &str);
    pub fn simulate_tab_switch(&mut self);
    pub fn url(&self) -> &str;

    // Warm reuse
    pub async fn navigate_warm(&mut self, url: &str) -> Result<()>;
}

// === PagePool ===

pub struct PagePool { /* warm V8 isolate pool */ }

impl PagePool {
    pub fn new(max_size: usize) -> Self;
    pub async fn acquire(&self, profile: &StealthProfile) -> Page;
    pub async fn release(&self, page: Page);
    pub async fn navigate(&self, url: &str, profile: &StealthProfile) -> Result<NavigateResult>;
}

// === ParallelPager ===

pub struct ParallelPager { /* N OS-thread workers */ }

impl ParallelPager {
    pub fn new(num_workers: usize) -> Self;
    pub async fn navigate(&self, url: &str, profile: &StealthProfile, max_iter: usize) -> Result<NavigateResult>;
}

// === EngineHandle (Send+Sync over !Send engine) ===

pub struct EngineHandle { /* Send+Sync wrapper */ }

impl EngineHandle {
    pub fn spawn() -> Self;
    pub async fn navigate(&self, url: &str, profile: &StealthProfile, max_iter: usize) -> Result<PageSnapshot>;
    pub async fn evaluate(&self, js: &str) -> Result<String>;
    pub async fn query_text(&self, selector: &str) -> Option<String>;
}
```

### 6.4 Logic Flow

**Page navigate() loop (full browser):**

1. Fetch URL via hpx::Client with stealth headers + TLS fingerprint
2. Parse CSP headers + meta tags → install CSP policy
3. Parse HTML → build DOM tree (html5ever → arena Dom)
4. Collect and execute inline `<script>` tags via V8
5. Fetch and execute external `<script src>` respecting CSP
6. Run event loop: drain microtasks, fire timers, run requestAnimationFrame callbacks
7. Check challenge markers via `engine_classify(body)`
8. If challenge detected → dispatch to registered ChallengeSolvers
9. Cookie-diff retry: if solver changed cookies, re-fetch and rebuild
10. Pending-nav poll: check if solver triggered navigation
11. Budget management: 15s default, extended per-host, 140s for sec-cpt PoW
12. Return Page with rendered DOM, ready for `evaluate()`, `title()`, `text_of()` queries

**Challenge-aware request flow:**

1. Client sends request via normal Tower stack
2. Response received → check `engine_classify(body)`
3. If `ChallengeVerdict::Pass` → return `ChallengeOutcome::Clean(response)`
4. If challenge detected → run solver `detect()` on each registered solver
5. Return `ChallengeOutcome::Challenge { verdict, kind, response }`
6. User can then invoke solver's `solve()` if they have a Page

**Critical-CH retry flow:**

1. Send request, receive response with `Accept-CH` header
2. Store origin in Accept-CH set
3. If response has `Critical-CH` and we didn't send those hints → re-request with high-entropy CH
4. Use profile fields (arch, bitness, platform-version, model, wow64, full-version-list) for CH values

**Cookie persistence flow:**

1. On `CookieJar2::new(path)` → load from JSON file if exists
2. On `set_cookies()` → parse Set-Cookie headers, apply domain/path scoping per RFC 6265
3. On drop or explicit `save()` → atomic write (tempfile + rename)
4. Collision scrub: on `add()`, check if cookie domain collides with known pairs (twitter.com/x.com)

**JS extension loading:**

1. On BrowserJsRuntime creation → register 16 extension modules
2. Each extension injects globals into V8 (document, fetch, setTimeout, crypto, etc.)
3. DOM extension bridges V8 ↔ arena Dom via NodeId references
4. fetch() extension uses hpx::Client for network requests (preserving stealth)

### 6.5 Configuration

New feature flags:

- `challenge` — enables challenge detection module (classify + verdict + trait)
- `stealth-profile` — enables StealthProfile, BehaviorProfile, GpuProfile, presets
- `humanize` — enables mouse/keystroke/scroll generation (depends on `stealth-profile`)
- `csp` — enables CSP3 parser (winnow dep)

Extended `stealth` preset: `stealth = ["challenge", "stealth-profile", "humanize", "csp", ...existing...]

### 6.6 Error Handling

- `ChallengeError` enum (thiserror): `ClassificationFailed`, `SolverError(String)`, `CspParseError(String)`
- CSP parser: `CspParseError` with position info from winnow
- Profile validation: returns `Vec<String>` of validation failures (not a single error, allows reporting all issues)

### 6.7 Maintainability Notes

- Challenge marker constants are `const &str` at module top, not buried in logic
- CSP parser uses winnow combinators per AGENTS.md — no hand-rolled char-by-char parsing
- Humanization functions are pure (take profile, return vec) — easy to test deterministically
- StealthProfile uses serde for YAML/JSON loading — same pattern as browser_oxide's config.rs

---

## 7. Verification & Testing Strategy

### 7.1 Unit Testing

| Module | What to Test | Scope |
| :--- | :--- | :--- |
| `challenge/classify.rs` | All marker constants, edge cases (multilingual, small bodies) | 20+ example-based tests |
| `challenge/verdict.rs` | `is_challenge()`, `as_str()` | Enum correctness |
| `stealth/profile.rs` | `validate()` for valid/invalid profiles | Cross-field consistency |
| `stealth/presets.rs` | All presets validate, `random_desktop()` diversity | Preset correctness |
| `stealth/behavior.rs` | Deterministic output with fixed seed, trajectory smoothness | Pure function tests |
| `csp/parser.rs` | Parse various CSP headers, strict-dynamic, nonce/hash | Property + example |
| `csp/check.rs` | `allows()` for various source/directive combos | Example-based |
| `dom/` | DOM tree operations: appendChild, insertBefore, detach, querySelector | Arena correctness |
| `css_parser/` | CSS tokenizer + parser: selectors, properties, values | Property tests |
| `css_selectors/` | Selector matching: combinators, pseudo-classes, nth-child | Example-based |
| `layout/` | taffy-based layout: block, flex, grid | Visual regression |
| `js_runtime/` | V8 extension loading, DOM bindings, fetch binding | Integration |
| `event_loop/` | Timer firing, rAF callbacks, idle detection | Timing tests |
| `canvas/` | Canvas 2D paths, PNG byte parity | Byte-exact tests |
| `net/tls.rs` | TLS fingerprint: Chrome 147 cipher suite order matches reference | Byte-exact capture |

### 7.2 Property Testing

| Target Behavior | Why Property Testing Helps | Tool / Command | Planned Invariants |
| :--- | :--- | :--- | :--- |
| CSP header parsing | Large combinatorial input space (directives × sources × flags) | `proptest` via `cargo test -p hpx --features csp` | Round-trip: parse → serialize → parse yields same policy. No panic on any UTF-8 input. |
| Challenge marker matching | Body text can be arbitrary | `proptest` | engine_classify never panics on any input. Pass verdict for empty body. |

### 7.3 Integration Testing

- Challenge detection against recorded response bodies from Cloudflare, DataDome, Akamai (test fixtures in `tests/fixtures/challenge/`)
- Cookie jar persistence round-trip: set cookies → save → load → verify
- CSP enforcement: build request to URL with CSP policy → verify blocked/allowed

### 7.4 BDD Acceptance Testing

| Scenario ID | Feature File | Command | Success Criteria |
| :--- | :--- | :--- | :--- |
| **BDD-01** | `features/challenge-detection.feature` | `cargo test -p hpx-browser --features challenge` | All scenarios pass |
| **BDD-02** | `features/stealth-profile.feature` | `cargo test -p hpx-browser --features stealth-profile` | All scenarios pass |
| **BDD-03** | `features/csp-parsing.feature` | `cargo test -p hpx-browser --features csp` | All scenarios pass |
| **BDD-04** | `features/page-navigation.feature` | `cargo test -p hpx-browser` | Navigate, evaluate, title, content all work |
| **BDD-05** | `features/dom-rendering.feature` | `cargo test -p hpx-browser` | HTML parsed, DOM queries return correct results |
| **BDD-06** | `features/js-execution.feature` | `cargo test -p hpx-browser --features v8` | JS executed, DOM manipulations reflected |

### 7.5 Robustness & Performance Testing

| Test Type | When It Is Required | Tool / Command | Planned Coverage or Reason Not Needed |
| :--- | :--- | :--- | :--- |
| **Fuzz** | CSP parser processes untrusted HTTP headers | `cargo fuzz run csp_parser` | Crash-safety for malformed CSP headers |
| **Fuzz** | CSS parser processes untrusted stylesheets | `cargo fuzz run css_parser` | Crash-safety for malformed CSS |
| **Benchmark** | Challenge detection runs on every response body | `criterion` bench for engine_classify | Regression budget: <100us for 50KB body |
| **Benchmark** | Page rendering throughput | `criterion` bench for page_throughput | Matches browser_oxide baseline |
| **Benchmark** | TLS handshake latency | `criterion` bench for tls_handshake | Chrome-matching performance |

### 7.6 Critical Path Verification

| Verification Step | Command | Success Criteria |
| :--- | :--- | :--- |
| **VP-01** | `cargo check -p hpx-browser` | Compiles with default features |
| **VP-02** | `cargo check -p hpx-browser --features v8,canvas,quic,cdp,workers` | Compiles with all features |
| **VP-03** | `cargo test -p hpx-browser` | All unit/integration tests pass |
| **VP-04** | `cargo test -p hpx-browser --features v8` | V8/JS tests pass |
| **VP-05** | `cargo clippy -p hpx-browser -- -D warnings` | No warnings |
| **VP-06** | `cargo test -p hpx` | Existing hpx tests still pass |
| **VP-07** | `cargo test -p hpx-emulation` | Existing emulation tests still pass |
| **VP-08** | `just lint && just test && just test-all` | Full workspace passes |

---

## 8. Implementation Plan

- [ ] **Phase 1: Foundation** — Create hpx-browser crate, add workspace dep, feature flags, Cargo.toml with all deps (html5ever, deno_core, skia-safe, taffy, quinn, h3, etc.)
- [ ] **Phase 2: Networking** — Port HttpClient, TLS fingerprinting, headers, cookies, CSP from browser_oxide net/. Wire hpx::Client as HTTP backend.
- [ ] **Phase 3: Stealth Layer** — Port challenge detection, ChallengeSolver trait, StealthProfile, presets, behavioral humanization, region headers
- [ ] **Phase 4: DOM + CSS** — Port html5ever DOM tree (arena-allocated), CSS parser, selector matching, cascade, taffy layout
- [ ] **Phase 5: JS Runtime** — Port deno_core integration, 16 JS extension modules, BrowserEventLoop (timers, rAF, idle)
- [ ] **Phase 6: Canvas + Rendering** — Port skia-safe Canvas 2D, WebGL stubs, AudioContext fingerprint, font shaping
- [ ] **Phase 7: Advanced Features** — Port Web Workers, iframes, CDP server, PagePool, ParallelPager, EngineHandle
- [ ] **Phase 8: Integration & Polish** — navigate() loop, BDD tests, property tests, benchmarks, fuzz targets, docs

## Revision History

| Date | Change | Reason |
| :--- | :--- | :--- |
| 2026-06-24 | Expanded scope from HTTP-client-only to full standalone browser engine | User feedback: goal is to render web pages and execute JS without headless Chrome dependency |
| 2026-06-24 | Added hpx-browser crate architecture (AD-06) | Browser engine features too heavy for hpx HTTP client crate |
| 2026-06-24 | Converted R15-R17 from non-goal to functional, added R18-R23 | Full browser engine features now in scope |
| 2026-06-24 | Added DOM, CSS, V8, Canvas, Workers, Iframes, CDP, QUIC to module structure | All browser_oxide features being ported |
