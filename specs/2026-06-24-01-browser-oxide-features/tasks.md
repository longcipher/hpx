# Browser Oxide Feature Integration — Implementation Tasks

| Metadata | Details |
| :--- | :--- |
| **Design Doc** | specs/2026-06-24-01-browser-oxide-features/design.md |
| **Owner** | N/A |
| **Start Date** | 2026-06-24 |
| **Target Date** | N/A |
| **Status** | Planning |

## Summary & Phasing

Create `hpx-browser` crate — a full standalone browser engine porting ALL features from browser_oxide. Eight phases: (1) Foundation — crate setup, deps, feature flags; (2) Networking — HttpClient, TLS, headers, cookies, CSP; (3) Stealth — challenge detection, profiles, humanization; (4) DOM + CSS — html5ever DOM, CSS parser, taffy layout; (5) JS Runtime — deno_core, 16 extensions, event loop; (6) Canvas — skia-safe 2D, WebGL stubs, audio; (7) Advanced — Workers, iframes, CDP, pool, parallel; (8) Integration — navigate loop, BDD, benchmarks.

- **Planner Contract Rule:** Emit a contract-complete, build-eligible spec in existing markdown artifacts.
- **State Contract Rule:** `🔴 TODO` -> `🟡 IN PROGRESS` -> `🟢 DONE`; exceptional: `⏭️ SKIPPED`, `🔄 DCR`, `⛔ OBSOLETE`.
- **Property Testing Rule:** Property tests with `proptest` for CSP parser, CSS parser.
- **Fuzzing Rule:** `cargo-fuzz` for CSP parser, CSS parser.
- **Benchmark Rule:** `criterion` for challenge detection, page throughput, TLS handshake.
- **Architecture Decisions Rule:** AD-06 (separate crate for browser engine).

---

## Phase 1: Foundation — Crate Setup, Dependencies, Feature Flags

### Task 1.1: Create hpx-browser Crate with Cargo.toml

> **Context:** Create `crates/hpx-browser/` with Cargo.toml listing all dependencies from browser_oxide: html5ever, deno_core, skia-safe, taffy, quinn, h3, boring2, tokio-boring2, http2, tokio-tungstenite, rustls, flate2, brotli, zstd, sha1, sha2, rand, rand_chacha, rand_distr, png, image, fontdb, rustybuzz, swash, rustfft, serde, serde_json, serde_yaml_ng, thiserror, tracing, base64, chrono, lazy_static, url, percent-encoding, httparse, bytes, http, futures-util, async-trait, tokio (full). Feature flags: `v8`, `canvas`, `webgl`, `quic`, `cdp`, `workers`, `blocker`.
> **Verification:** `cargo check -p hpx-browser` compiles.

- **Priority:** P0
- **Scope:** Cargo.toml configuration
- **Requirement Coverage:** All
- **Scenario Coverage:** N/A (infrastructure)
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior` — new crate, no changes to existing
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Create `crates/hpx-browser/Cargo.toml` with `[package]` metadata, workspace deps
- [x] **Step 2:** Add `v8` feature gating `dep:deno_core`, `dep:deno_error`
- [x] **Step 3:** Add `canvas` feature gating `dep:skia-safe`, `dep:png`, `dep:image`
- [x] **Step 4:** Add `quic` feature gating `dep:quinn`, `dep:h3`, `dep:h3-quinn`
- [x] **Step 5:** Add `cdp` feature gating `dep:tokio-tungstenite`
- [x] **Step 6:** Add `workers` feature (depends on `v8`)
- [x] **Step 7:** Add hpx workspace dep: `hpx = { workspace = true }`
- [x] **Step 8:** Create `crates/hpx-browser/src/lib.rs` with `#![deny(unsafe_code)]` and module stubs
- [x] **Step 9:** Add `[workspace]` member entry in root Cargo.toml
- [x] **BDD Verification:** N/A
- [x] **Verification:** `cargo check -p hpx-browser` and `cargo check -p hpx-browser --features v8,canvas,quic,cdp,workers` both compile
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 1.2: Create Core Type Stubs

> **Context:** Create stub modules and types for all major subsystems so the crate compiles with empty implementations.
> **Verification:** `cargo check -p hpx-browser --all-features` compiles.

- **Priority:** P0
- **Scope:** Module structure, type stubs
- **Requirement Coverage:** All
- **Scenario Coverage:** N/A (types only)
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Create `src/page.rs` with `Page` struct and stub methods: `navigate()`, `evaluate()`, `title()`, `content()`, `text_content()`, `text_of()`, `has_element()`, `challenge_verdict()`
- [x] **Step 2:** Create `src/dom/mod.rs` with `Dom` struct, `NodeId`, `NodeType` enum
- [x] **Step 3:** Create `src/css_parser/mod.rs` stub
- [x] **Step 4:** Create `src/css_selectors/mod.rs` stub
- [x] **Step 5:** Create `src/layout/mod.rs` stub
- [x] **Step 6:** Create `src/js_runtime/mod.rs` stub (gated on `v8`)
- [x] **Step 7:** Create `src/event_loop/mod.rs` stub (gated on `v8`)
- [x] **Step 8:** Create `src/canvas/mod.rs` stub (gated on `canvas`)
- [x] **Step 9:** Create `src/net/mod.rs` stub
- [x] **Step 10:** Create `src/stealth/mod.rs` stub
- [x] **Step 11:** Create `src/challenge/mod.rs` stub
- [x] **Step 12:** Create `src/workers/mod.rs` stub (gated on `workers`)
- [x] **Step 13:** Create `src/iframe.rs` stub
- [x] **Step 14:** Create `src/protocol/mod.rs` stub (gated on `cdp`)
- [x] **Step 15:** Create `src/pool.rs` and `src/parallel.rs` stubs
- [x] **Step 16:** Create `src/host/mod.rs` with `EngineHandle` stub
- [x] **BDD Verification:** N/A
- [x] **Verification:** `cargo check -p hpx-browser --all-features` compiles
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

---

## Phase 2: Networking — HttpClient, TLS, Headers, Cookies, CSP

### Task 2.1: Port HttpClient Core

> **Context:** Port browser_oxide's `HttpClient` (net/mod.rs, 1747 lines) into hpx-browser. This includes multi-protocol HTTP (H1/H2/H3), connection pooling, shared session (cookies, DNS, Accept-CH), H1 memory, stale-connection recovery. Uses hpx::Client as the underlying HTTP backend where possible, with browser_oxide's stealth TLS and header logic on top.
> **Verification:** `cargo check -p hpx-browser` compiles with HttpClient.

- **Priority:** P0
- **Scope:** HTTP client core
- **Requirement Coverage:** `R9`, `R10`, `R11`, `R21`
- **Scenario Coverage:** N/A (infrastructure)
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Port `Response` struct (status, headers, set_cookies, body, url, timings)
- [x] **Step 2:** Port `TimingStats` struct (dns/connect/tls/request/response timestamps)
- [x] **Step 3:** Port `SharedSession` (cookies, accept_ch, dns_cache, alt_svc_cache)
- [x] **Step 4:** Port `HttpClient` with `new()`, `shared()`, `get()`, `post()`, `fetch_get()`, `fetch_post_bytes()`, `get_follow()`, etc.
- [x] **Step 5:** Port H1 memory (`h1_only_hosts`) and stale-connection recovery
- [x] **Step 6:** Port `preconnect()` for pre-establishing H2 connections
- [x] **Step 7:** Write unit tests for HttpClient construction, GET/POST, redirect following
- [x] **BDD Verification:** N/A
- [x] **Verification:** `cargo test -p hpx-browser net` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 2.2: Port TLS Fingerprinting

> **Context:** Port browser_oxide's TLS fingerprint (net/tls.rs, 835 lines) with Chrome 147 cipher suite order, extension permutation, post-quantum curves (MLKEM768, Kyber768Draft00), Safari iOS and Firefox fingerprint configs. Maps to hpx's TlsOptions.
> **Verification:** TLS handshake produces byte-exact Chrome 147 JA3/JA4 fingerprint.

- **Priority:** P0
- **Scope:** TLS fingerprinting
- **Requirement Coverage:** `R13`
- **Scenario Coverage:** TLS fingerprint scenarios
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Port Chrome 147 cipher list (15 ciphers), sigalgs (8), curves (X25519_MLKEM768, X25519, SECP256R1, SECP384R1)
- [x] **Step 2:** Port Safari iOS cipher list (20 ciphers), sigalgs (10), curves, extension permutation
- [x] **Step 3:** Port Firefox cipher list (17 ciphers), sigalgs (11), curves (incl. FFDHE2048/3072)
- [x] **Step 4:** Port Chrome extension permutation (16 extensions, Fisher-Yates shuffled per handshake)
- [x] **Step 5:** Port `chrome_connector()` building SslConnector per device_class
- [x] **Step 6:** Port `configure_connection()` with ECH GREASE, ALPS, SNI
- [x] **Step 7:** Write tests: cipher order matches reference capture, extension permutation works
- [x] **BDD Verification:** N/A
- [x] **Verification:** `cargo test -p hpx-browser tls` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 2.3: Port Headers, Cookies, CSP

> **Context:** Port browser_oxide's header generation (net/headers.rs, 1500 lines), cookie jar (net/cookies.rs, 597 lines), and CSP parser (net/csp.rs, 1137 lines).
> **Verification:** Headers match browser_oxide, cookies scoped by domain, CSP parser handles all directives.

- **Priority:** P0
- **Scope:** Headers, cookies, CSP
- **Requirement Coverage:** `R5`, `R6`, `R7`
- **Scenario Coverage:** CSP parsing, cookie jar, header scenarios
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Property` + `Fuzz`
- **Status:** 🟢 DONE
- [x] **Step 1:** Port `nav_headers()`, `chrome_headers()`, `firefox_headers()`, `safari_headers()` with regional accept-language
- [x] **Step 2:** Port `CookieJar` with domain scoping, RFC 6265 compliance, persistence, collision scrub
- [x] **Step 3:** Port CSP3 `Policy::parse()`, `PolicySet::allows()`, strict-dynamic semantics, fallback chains
- [x] **Step 4:** Port Accept-CH upgrade and Critical-CH retry logic
- [x] **Step 5:** Write property tests for CSP parser (arbitrary headers, no panic)
- [x] **Step 6:** Write fuzz target for CSP parser
- [x] **Step 7:** Write tests: regional language matching, cookie domain scoping, CSP strict-dynamic
- [x] **BDD Verification:** `cargo test -p hpx-browser csp` passes
- [x] **Verification:** `cargo test -p hpx-browser net` passes
- [x] **Advanced Test Verification:** `cargo fuzz run csp_parser -- -max_total_time=30` — no crashes
- [x] **Runtime Verification:** N/A

---

## Phase 3: Stealth — Challenge Detection, Profiles, Humanization

### Task 3.1: Port Challenge Detection and Solver

> **Context:** Port classify.rs (754 lines), challenge.rs (220 lines) into hpx-browser. engine_classify(), ChallengeVerdict, ChallengeSolver trait.
> **Verification:** All challenge types detected, solver trait works.

- **Priority:** P0
- **Scope:** Anti-bot detection
- **Requirement Coverage:** `R1`, `R2`
- **Scenario Coverage:** `challenge-detection.feature`
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `Reduce nesting`
- **Advanced Test Coverage:** `Property`
- **Status:** 🟢 DONE
- [x] **Step 1:** Port marker constants and `engine_classify()` from classify.rs
- [x] **Step 2:** Port `ChallengeVerdict`, `ChallengeKind`, `SolveOutcome`, `ChallengeSolver` trait
- [x] **Step 3:** Write 20+ unit tests for classification
- [x] **Step 4:** Write proptest: engine_classify never panics on any input
- [x] **BDD Verification:** `cargo test -p hpx-browser challenge` passes
- [x] **Verification:** `cargo test -p hpx-browser challenge` passes
- [x] **Advanced Test Verification:** `cargo test -p hpx-browser proptest_classify` passes
- [x] **Runtime Verification:** N/A

### Task 3.2: Port Stealth Profiles and Presets

> **Context:** Port StealthProfile (50+ fields), presets (Chrome 148, Firefox 135, Safari 18), validation.
> **Verification:** All presets validate, random_desktop() works.

- **Priority:** P0
- **Scope:** Stealth profiles
- **Requirement Coverage:** `R3`, `R12`
- **Scenario Coverage:** `stealth-profile.feature`
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Port `StealthProfile` struct with all 50+ fields
- [x] **Step 2:** Port `validate()` with all consistency checks
- [x] **Step 3:** Port all presets: chrome_148_{windows,macos,linux}, firefox_135_{macos,windows,linux}, safari_18_macos, regional variants
- [x] **Step 4:** Port `random_desktop()`, `with_locale()`, `chrome_148_macos_sampled()`
- [x] **Step 5:** Write tests: all presets validate, random_desktop diversity
- [x] **BDD Verification:** `cargo test -p hpx-browser stealth` passes
- [x] **Verification:** `cargo test -p hpx-browser stealth` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 3.3: Port Behavioral Humanization

> **Context:** Port Sigma-Lognormal mouse trajectories, keystroke dynamics, scroll burst from behavior.rs (873 lines).
> **Verification:** Deterministic with fixed seed, trajectory smoothness.

- **Priority:** P1
- **Scope:** Humanization
- **Requirement Coverage:** `R4`
- **Scenario Coverage:** `humanization.feature`
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Port `BehaviorProfile`, `Handedness`, `ScrollStyle`, `MousePoint`, `KeystrokeTiming`, `WheelTick`
- [x] **Step 2:** Port `mouse_trajectory()` — Sigma-Lognormal, 2-7 strokes, Fitts's Law, 8ms sampling
- [x] **Step 3:** Port `keystroke_timings()` — LogNormal dwell/flight, bigram modulation
- [x] **Step 4:** Port `wheel_burst()` — Trackpad exponential decay, Wheel discrete notches
- [x] **Step 5:** Write tests: deterministic output, trajectory approaches target
- [x] **BDD Verification:** `cargo test -p hpx-browser humanize` passes
- [x] **Verification:** `cargo test -p hpx-browser behavior` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

---

## Phase 4: DOM + CSS — html5ever DOM, CSS Parser, taffy Layout

### Task 4.1: Port DOM Tree (Arena-Allocated)

> **Context:** Port browser_oxide's DOM (dom/) with arena-allocated tree, NodeId (Copy), Shadow DOM, tree mutations.
> **Verification:** DOM operations work: createElement, appendChild, querySelector.

- **Priority:** P0
- **Scope:** DOM tree
- **Requirement Coverage:** `R16`
- **Scenario Coverage:** `dom-rendering.feature`
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Port `Dom` struct with arena (Vec<NodeType>), parents, children vectors
- [x] **Step 2:** Port `NodeId` (u32 wrapper, Copy), `NodeType` (Document, Element, Text, Comment)
- [x] **Step 3:** Port `ElementData` with tag, attributes, id, classes
- [x] **Step 4:** Port tree mutations: `append_child()`, `insert_before()`, `detach()`, `remove()`, `reparent_children()`
- [x] **Step 5:** Port Shadow DOM support
- [x] **Step 6:** Implement `css_selectors::Element` trait for DOM nodes
- [x] **Step 7:** Port html5ever parser integration: parse HTML string → Dom
- [x] **Step 8:** Write tests: DOM construction, mutations, querySelector
- [x] **BDD Verification:** `cargo test -p hpx-browser dom` passes
- [x] **Verification:** `cargo test -p hpx-browser dom` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 4.2: Port CSS Parser and Selector Matching

> **Context:** Port CSS tokenizer + parser (css_parser/), selector matching (css_selectors/), cascade (css_cascade/), computed values (css_values/).
> **Verification:** CSS rules parsed, selectors match elements, cascade resolves.

- **Priority:** P0
- **Scope:** CSS parsing
- **Requirement Coverage:** `R16`
- **Scenario Coverage:** `dom-rendering.feature`
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Property`
- **Status:** 🟢 DONE
- [x] **Step 1:** Port CSS tokenizer: token types, position tracking
- [x] **Step 2:** Port CSS parser: rule/declaration parsing, @-rules
- [x] **Step 3:** Port selector matching: type, class, id, attribute, pseudo-class, combinators (descendant, child, sibling)
- [x] **Step 4:** Port cascade: specificity, !important, layers, media queries
- [x] **Step 5:** Port computed style resolution
- [x] **Step 6:** Write property tests: parse arbitrary CSS without panic
- [x] **Step 7:** Write tests: selector matching, cascade priority, computed values
- [x] **BDD Verification:** N/A
- [x] **Verification:** `cargo test -p hpx-browser css` passes
- [x] **Advanced Test Verification:** `cargo test -p hpx-browser proptest_css` passes
- [x] **Runtime Verification:** N/A

### Task 4.3: Port Layout Engine (taffy)

> **Context:** Port taffy-based CSS layout (layout/) with Block, Flexbox, Grid. getBoundingClientRect, getComputedStyle.
> **Verification:** Layout produces correct bounding boxes for test pages.

- **Priority:** P0
- **Scope:** CSS layout
- **Requirement Coverage:** `R16`
- **Scenario Coverage:** `dom-rendering.feature`
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Port taffy integration: build layout tree from DOM + computed styles
- [x] **Step 2:** Port Block layout
- [x] **Step 3:** Port Flexbox layout
- [x] **Step 4:** Port Grid layout
- [x] **Step 5:** Port `getBoundingClientRect()`, `offsetWidth`, `getComputedStyle()` JS bindings
- [x] **Step 6:** Write tests: layout correctness for simple pages
- [x] **BDD Verification:** N/A
- [x] **Verification:** `cargo test -p hpx-browser layout` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

---

## Phase 5: JS Runtime — deno_core, Extensions, Event Loop

### Task 5.1: Port V8 JS Runtime (deno_core)

> **Context:** Port BrowserJsRuntime wrapping deno_core::JsRuntime with 16 extension modules. Gated on `v8` feature.
> **Verification:** JS code executes, DOM bindings work.

- **Priority:** P0
- **Scope:** JS execution engine
- **Requirement Coverage:** `R16`
- **Scenario Coverage:** `js-execution.feature`
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Port `BrowserJsRuntime` wrapping deno_core::JsRuntime
- [x] **Step 2:** Port DOM extension (document.*, createElement, querySelector, innerHTML, textContent)
- [x] **Step 3:** Port fetch extension (fetch() using hpx::Client)
- [x] **Step 4:** Port timer extension (setTimeout, setInterval, clearTimeout, clearInterval)
- [x] **Step 5:** Port console extension (console.log/warn/error)
- [x] **Step 6:** Port crypto extension (crypto.getRandomValues)
- [x] **Step 7:** Port storage extension (localStorage/sessionStorage)
- [x] **Step 8:** Port input extension (click, type, scroll event dispatch)
- [x] **Step 9:** Port layout extension (getBoundingClientRect, getComputedStyle)
- [x] **Step 10:** Port navigation extension (window.location, history)
- [x] **Step 11:** Port stealth extension (override navigator, screen, webdriver, etc.)
- [x] **Step 12:** Port remaining extensions: SSE, WebSocket, performance, workers
- [x] **Step 13:** Write tests: basic JS eval, DOM manipulation via JS, fetch() calls
- [x] **BDD Verification:** `cargo test -p hpx-browser --features v8 js` passes
- [x] **Verification:** `cargo test -p hpx-browser --features v8` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 5.2: Port Browser Event Loop

> **Context:** Port BrowserEventLoop (event_loop/) with V8 execution, timers, requestAnimationFrame, idle detection, promise resolution.
> **Verification:** Timers fire, rAF callbacks run, idle detection works.

- **Priority:** P0
- **Scope:** Event loop
- **Requirement Coverage:** `R22`
- **Scenario Coverage:** `js-execution.feature`
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Port `BrowserEventLoop` with js runtime, dom, timer queue, rAF callbacks
- [x] **Step 2:** Port timer queue: add/remove/next-fire-time
- [x] **Step 3:** Port rAF scheduling
- [x] **Step 4:** Port microtask draining
- [x] **Step 5:** Port idle detection (AllWorkDone, Timeout)
- [x] **Step 6:** Port V8 deadline watcher (RAII guard, terminate_execution on budget exceeded)
- [x] **Step 7:** Write tests: timer firing, rAF callback, idle detection
- [x] **BDD Verification:** N/A
- [x] **Verification:** `cargo test -p hpx-browser --features v8 event_loop` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

---

## Phase 6: Canvas + Rendering — skia-safe 2D, WebGL, Audio

### Task 6.1: Port Canvas 2D Rendering

> **Context:** Port Canvas 2D context via skia-safe, WebGL stubs, AudioContext fingerprint, font shaping. Gated on `canvas` feature.
> **Verification:** Canvas draws, PNG byte parity matches browser_oxide.

- **Priority:** P1
- **Scope:** Canvas rendering
- **Requirement Coverage:** `R18`
- **Scenario Coverage:** Canvas rendering scenarios
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Port Canvas 2D context (fillRect, strokeRect, arc, bezierCurveTo, drawImage, etc.)
- [x] **Step 2:** Port WebGL parameter stubs with per-GPU profiles
- [x] **Step 3:** Port AudioContext fingerprint (DynamicsCompressor)
- [x] **Step 4:** Port font shaping (rustybuzz + swash)
- [x] **Step 5:** Write tests: canvas path rendering, PNG byte parity
- [x] **BDD Verification:** N/A
- [x] **Verification:** `cargo test -p hpx-browser --features canvas` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

---

## Phase 7: Advanced — Workers, Iframes, CDP, Pool, Parallel

### Task 7.1: Port Web Workers

> **Context:** Port Web Worker support (workers/) — each worker in own V8 isolate with postMessage/onmessage. Gated on `workers` feature.
> **Verification:** Worker spawns, postMessage works, terminate kills.

- **Priority:** P1
- **Scope:** Web Workers
- **Requirement Coverage:** `R19`
- **Scenario Coverage:** Worker scenarios
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Port Worker struct with own V8 isolate
- [x] **Step 2:** Port postMessage/onmessage bridge
- [x] **Step 3:** Port terminate()
- [x] **Step 4:** Write tests: worker spawn, message passing, termination
- [x] **BDD Verification:** N/A
- [x] **Verification:** `cargo test -p hpx-browser --features workers` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 7.2: Port Iframe Support

> **Context:** Port iframe support (iframe.rs) — srcdoc + src, each iframe gets own DOM/V8/event loop, CSP frame-src enforcement.
> **Verification:** Iframes render, isolated from parent.

- **Priority:** P1
- **Scope:** Iframes
- **Requirement Coverage:** `R20`
- **Scenario Coverage:** Iframe scenarios
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Port `ChildIframe` struct with own DOM, V8 runtime, event loop
- [x] **Step 2:** Port srcdoc and src iframe materialization
- [x] **Step 3:** Port CSP frame-src enforcement
- [x] **Step 4:** Port `rematerialize_iframes()` for script-injected iframes
- [x] **Step 5:** Write tests: iframe isolation, srcdoc rendering
- [x] **BDD Verification:** N/A
- [x] **Verification:** `cargo test -p hpx-browser --features v8 iframe` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 7.3: Port CDP Server

> **Context:** Port Chrome DevTools Protocol server (protocol/) via WebSocket. Enables Puppeteer/Playwright to drive hpx-browser. Gated on `cdp` feature.
> **Verification:** CDP server starts, Puppeteer can connect and navigate.

- **Priority:** P1
- **Scope:** CDP interop
- **Requirement Coverage:** `R17`
- **Scenario Coverage:** CDP scenarios
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Port CDP WebSocket server
- [x] **Step 2:** Port basic CDP commands: Page.navigate, Runtime.evaluate, DOM.getDocument
- [x] **Step 3:** Port CDP event emission: Page.loadEventFired, Network.responseReceived
- [x] **Step 4:** Write tests: CDP server starts, basic command round-trip
- [x] **BDD Verification:** N/A
- [x] **Verification:** `cargo test -p hpx-browser --features cdp` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 7.4: Port PagePool and ParallelPager

> **Context:** Port PagePool (warm V8 isolate pool) and ParallelPager (N OS-thread workers) from pool.rs, parallel.rs. Port EngineHandle (Send+Sync wrapper).
> **Verification:** Pool reuses pages, parallel navigation works.

- **Priority:** P1
- **Scope:** Page pooling and parallel navigation
- **Requirement Coverage:** `R15`
- **Scenario Coverage:** Pool/parallel scenarios
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Port `PagePool` with acquire/release/navigate
- [x] **Step 2:** Port `ParallelPager` with OS-thread worker pool, round-robin scheduling
- [x] **Step 3:** Port `EngineHandle` (Send+Sync over !Send Page)
- [x] **Step 4:** Port `PageSnapshot` for cross-thread result passing
- [x] **Step 5:** Write tests: pool acquire/release, parallel navigate
- [x] **BDD Verification:** N/A
- [x] **Verification:** `cargo test -p hpx-browser --features v8 pool` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

---

## Phase 8: Integration — navigate() Loop, BDD, Benchmarks

### Task 8.1: Implement navigate() Loop

> **Context:** Port the full challenge-aware navigate() loop from page.rs: fetch → CSP install → HTML parse → script execution → event loop drain → challenge detection → solver dispatch → cookie-diff retry → budget management.
> **Verification:** navigate() works end-to-end on test URLs.

- **Priority:** P0
- **Scope:** Page navigation
- **Requirement Coverage:** `R14`, `R23`
- **Scenario Coverage:** `page-navigation.feature`
- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Implement `Page::navigate()` — full loop with challenge detection
- [x] **Step 2:** Implement `Page::navigate_with_solvers()` — with custom solver list
- [x] **Step 3:** Implement `Page::navigate_warm()` — reuse V8 isolate
- [x] **Step 4:** Implement budget management (15s default, per-host overrides, sec-cpt 140s)
- [x] **Step 5:** Implement cookie-diff retry and pending-nav poll
- [x] **Step 6:** Implement geo-country splash detection and follow
- [x] **Step 7:** Write BDD scenarios: navigate to clean page, navigate to challenge page
- [x] **BDD Verification:** `cargo test -p hpx-browser --features v8 navigate` passes
- [x] **Verification:** `cargo test -p hpx-browser --features v8` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 8.2: Write BDD Feature Files

> **Context:** Write Gherkin feature files for page navigation, DOM rendering, JS execution acceptance tests.
> **Verification:** All BDD scenarios defined.

- **Priority:** P1
- **Scope:** BDD acceptance tests
- **Requirement Coverage:** `R14`, `R16`, `R23`
- **Scenario Coverage:** `page-navigation.feature`, `dom-rendering.feature`, `js-execution.feature`
- **Loop Type:** `BDD+TDD`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Create `features/page-navigation.feature` with scenarios: Navigate to clean page, Navigate with challenge detection, Navigate warm reuse
- [x] **Step 2:** Create `features/dom-rendering.feature` with scenarios: Parse HTML, Query elements, Mutate DOM
- [x] **Step 3:** Create `features/js-execution.feature` with scenarios: Evaluate JS, DOM manipulation via JS, Timer execution
- [x] **Step 4:** Create step definitions
- [x] **BDD Verification:** All feature files parse correctly
- [x] **Verification:** `cargo test -p hpx-browser --features v8` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 8.3: Add Benchmarks

> **Context:** Add criterion benchmarks for page rendering throughput, challenge detection, TLS handshake.
> **Verification:** Benchmarks run, meet regression budgets.

- **Priority:** P2
- **Scope:** Performance benchmarks
- **Requirement Coverage:** `R1`, `R16`
- **Scenario Coverage:** N/A
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Benchmark`
- **Status:** 🟢 DONE
- [x] **Step 1:** Create `benches/page_throughput.rs` — criterion benchmark for page rendering
- [x] **Step 2:** Create `benches/challenge_detection.rs` — engine_classify on various body sizes
- [x] **Step 3:** Create `benches/tls_handshake.rs` — TLS handshake latency
- [x] **BDD Verification:** N/A
- [x] **Verification:** `cargo bench -p hpx-browser --bench page_throughput` runs
- [x] **Advanced Test Verification:** All benchmarks meet regression budgets
- [x] **Runtime Verification:** N/A

### Task 8.4: Full Workspace Verification

> **Context:** Run full lint, test, BDD suite across entire workspace to verify nothing is broken.
> **Verification:** `just lint && just test && just test-all` all pass.

- **Priority:** P0
- **Scope:** Workspace-wide verification
- **Requirement Coverage:** All
- **Scenario Coverage:** All
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** `Preserve existing behavior`
- **Simplification Focus:** `N/A`
- **Advanced Test Coverage:** `Example-based only`
- **Status:** 🟢 DONE
- [x] **Step 1:** Run `just format` — all code formatted
- [x] **Step 2:** Run `just lint` — no warnings
- [x] **Step 3:** Run `just test` — all unit/integration tests pass
- [x] **Step 4:** Run `just bdd` — all BDD scenarios pass
- [x] **Step 5:** Run `just test-all` — full suite passes
- [x] **BDD Verification:** All BDD scenarios pass
- [x] **Verification:** `just lint && just test && just test-all` — all green
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

---

## Summary & Timeline

| Phase | Tasks | Target Date |
| :--- | :---: | :--- |
| **1. Foundation** | 2 | N/A |
| **2. Networking** | 3 | N/A |
| **3. Stealth** | 3 | N/A |
| **4. DOM + CSS** | 3 | N/A |
| **5. JS Runtime** | 2 | N/A |
| **6. Canvas** | 1 | N/A |
| **7. Advanced** | 4 | N/A |
| **8. Integration** | 4 | N/A |
| **Total** | **22** | |

## Definition of Done

1. [ ] **Linted:** `just lint` passes with no warnings.
2. [ ] **Tested:** `just test` — all unit/integration tests pass.
3. [ ] **Formatted:** `just format` applied.
4. [ ] **Verified:** Each task's Verification criterion is met.
5. [ ] **Advanced-Tested:** Property tests for CSP/CSS parser pass. Benchmarks meet budgets. Fuzz targets run without crashes.
6. [ ] **Behavior-Preserved:** All existing hpx workspace tests continue to pass unchanged.
7. [ ] **Standalone:** `hpx-browser` can navigate to a URL, execute JS, and return page content without any external browser dependency.
