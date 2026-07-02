# Design: Absorb Blitz Architecture & Refactor hpx-browser

| Metadata | Details |
| :--- | :--- |
| **Author** | pb-plan agent |
| **Status** | Draft |
| **Created** | 2026-07-02 |
| **Reviewers** | — |
| **Related Issues** | — |

## Executive Summary

> Absorb two architectural ideas from DioxusLabs/blitz (Stylo CSS engine, Parley text layout) and perform four targeted code simplifications in hpx-browser: unify the HTTP client API, replace global state with explicit cleanup, harden unsafe DOM accessors, and swap in Stylo as the CSS backbone. The result is a leaner, more spec-compliant browser engine with less custom code and fewer fingerprinting leaks.

## Source Inputs & Normalization

### Source Materials

1. DioxusLabs/blitz repository analysis (README, Cargo.toml, crate structure)
2. hpx-browser codebase exploration via codegraph (lib.rs, net/mod.rs, dom.rs, css_*, layout/*, challenge.rs)
3. Requirement: "接受你的可吸收借鉴的模块建议, 以及全部精简/改进代码建议" (accept all absorbable module suggestions and all simplification suggestions)

### Source Requirement Ledger

| ID | Source Summary | Type | Notes |
| :--- | :--- | :--- | :--- |
| R1 | Unify HttpClient request methods | Simplification | 10+ near-identical methods → single request dispatch |
| R2 | Replace global SharedSession OnceLock with explicit DI | Simplification | Tests need isolation |
| R3 | Replace unreachable!() with expect() in DomElement | Simplification | Safety invariants should be self-documenting |
| R4 | Stylo replaces custom CSS parser/cascade for spec compliance | Absorb | ~4600 lines potentially reducible |
| R5 | Parley for text layout consistency with real browsers | Absorb | Canvas fingerprint hardening |
| R6 | Preserve all existing challenge detection behavior | Constraint | Core capability must not regress |
| R7 | Preserve CDP protocol compatibility | Constraint | Existing CDP callers must not break |
| R8 | Preserve canvas feature behavior | Constraint | Canvas rendering output equivalent |

## Requirements & Goals

### 3.1 Problem Statement

hpx-browser has grown a substantial set of hand-written CSS machinery (~4600 lines across 4 modules) that approaches but cannot match Firefox's exact spec behavior. Meanwhile, DioxusLabs/blitz already solved this by integrating Stylo (Servo's CSS engine), achieving spec compliance with far less code. Additionally, several internal APIs accreted duplication and global state that will be increasingly costly to maintain.

### 3.2 Functional Requirements (EARS)

**Ubiquitous:**

- REQ-01: The system *shall* provide a single `request(method, url, body, redirect_policy)` entry point on HttpClient when the refactor is complete.
- REQ-02: The system *shall* parse and cascade CSS using Stylo when the `css_engine` feature is enabled.
- REQ-03: The system *shall* lay out text via Parley when the `canvas` feature is enabled.
- REQ-04: The system *shall* construct `SharedSession` on the stack and pass it explicitly to components that need it.

**State-driven:**

- REQ-05: While the `css_engine` feature is disabled, the system *shall* continue using the existing custom CSS stack (no behavior change).
- REQ-06: While the redirect policy is `Follow(n)`, the system *shall* follow up to `n` redirects.

**Unwanted:**

- REQ-07: The system *shall not* expose a global mutable `SharedSession` accessible from any thread without explicit Arc.
- REQ-08: The system *shall not* trigger a panic via `unreachable!()` in DomElement accessors.

### 3.3 Non-Functional Goals

- **Performance:** No regression in HTTP request throughput. Challenge detection remains O(n) string scan.
- **Compatibility:** Public API (HttpClient methods, Page construction) preserved or feature-gated.
- **Maintainability:** Remove ~300 lines of duplicated net code, remove ~1 static global, reduce custom CSS surface.

### 3.4 Out of Scope

- Full Vello GPU rendering integration (defer until canvas fingerprinting becomes a concrete requirement)
- anyrender abstraction layer (overkill for single-backend Skia)
- WebSocket transport changes
- Changes to hpx core library

### 3.5 Assumptions

- Stylo version 0.18 is compatible with the workspace's Rust toolchain
- Parley 0.10 text output matches rustybuzz closely enough for canvas matching
- The `css_engine` feature can default to off, allowing gradual rollout

### 3.6 Code Simplification Constraints

Ponytail ladder applied at every decision:

1. Need: Yes — maintenance burden is real
2. Stdlib: N/A (browser engine)
3. Native: N/A
4. Existing dep: Stylo + Parley already exist in ecosystem
5. One line: Several cleanup items are one-liners (expect() fix)
6. Minimum code: unify net API with a single enum + method

**Behavior Preservation Boundary:** Challenge detection output identical byte-for-byte. CDP wire protocol identical. Canvas pixel output equivalent.

## Requirements Coverage Matrix

| ID | Covered In Design | Scenario Coverage | Task Coverage | Status |
| :--- | :--- | :--- | :--- | :--- |
| R1 | §AD-01, §9.1 | net-api-unification | T2.1–T2.3 | Covered |
| R2 | §AD-02, §9.1 | session-explicit-injection | T3.1–T3.2 | Covered |
| R3 | §9.1 | dom-element-safety | T4.1 | Covered |
| R4 | §AD-03, §9.1 | stylo-css-replacement | T5.1–T5.3 | Covered |
| R5 | §AD-04, §9.1 | parley-text-layout | T6.1 | Covered |
| R6 | §3.6 | (all) | (all) | Covered |
| R7 | Out of scope | — | — | Out of scope |
| R8 | §AD-04 | parley-text-layout | T6.1 | Covered |

## Architecture Overview

### Key Design Principles

- **Ponytail-first:** Every simplification justified by the 6-rung ladder
- **Feature-gated rollout:** Stylo and Parley behind `css_engine` and canvas feature flags
- **Explicit > implicit:** Remove static globals and hidden state
- **Minimum custom code:** Replace hand-rolled parsers with battle-tested crates when spec fidelity matters

### Existing Components to Reuse

| Component | Location | How to Reuse |
| :--- | :--- | :--- |
| `hpx::Client` | crates/hpx | Already the HTTP transport; wrapper methods simplified |
| `scc::HashSet` | net/mod.rs | Used for accept_ch origins; keep, move into SharedSession |
| `taffy::TaffyTree` | layout/engine.rs | Stylo feeds into Taffy for layout |
| `Dom`/`NodeId` | dom.rs | Unchanged; DomElement accessors just get expect() fixes |

## Architecture Decisions

### AD-01: Unified HTTP client request method

- **Status:** Accepted
- **Date:** 2026-07-02

**Context:** `HttpClient` has 10+ near-identical public methods differing only in HTTP verb, redirect handling, and body presence. Adding any new behavior (e.g., HTTP/3) requires touching all of them.

**Decision:** Introduce `RedirectPolicy::{Follow(u8), Manual}` enum. Add single method `request(method, url, body, policy) → Result<Response, NetError>`. Keep old methods as thin deprecated wrappers during transition.

**Consequences:**

- Positive: Adding new HTTP method = one match arm; single place for cookie injection
- Positive: Test surface shrinks from 10 methods to 1
- Negative: Slightly more complex method signature (enum parameter)
- Neutral: `#[deprecated]` wrappers maintain backward compatibility for one release cycle

### AD-02: SharedSession as explicit construction

- **Status:** Accepted
- **Date:** 2026-07-02

**Context:** `static SHARED_SESSION: OnceLock<SharedSession>` makes session state implicit and un-testable across different sessions.

**Decision:** Remove the OnceLock. `Page::new()` and `HttpClient::new()` accept `Arc<SharedSession>`. Provide `SharedSession::new()` constructor. Default caller creates session at app root and threads it down.

**Consequences:**

- Positive: Each test gets its own session; no cross-test pollution
- Positive: Shared state is visible in the type system
- Negative: Callers must wire session through (one extra parameter at construction site)

### AD-03: Stylo as optional CSS engine

- **Status:** Accepted (feature-gated)
- **Date:** 2026-07-02

**Context:** Custom css_parser/cascade/css_values/css_selectors total ~4600 lines and can't match spec fidelity. Blaze uses Stylo from Servo.

**Decision:** Add `css_engine` feature that enables Stylo-based path. Without the feature, custom CSS stack remains. With the feature, `stylo_taffy` provides CSS → Taffy layout directly.

**Consequences:**

- Positive: CSS behavior matches Firefox exactly
- Negative: Adds Stylo compilation time and binary size
- Positive: Removes ~4600 lines of hand-written CSS (once feature becomes default)

### AD-04: Parley for canvas text layout

- **Status:** Accepted (canvas feature only)
- **Date:** 2026-07-02

**Context:** Current canvas text goes through rustybuzz + swash. Blitz uses Parley which provides a higher-level API.

**Decision:** Replace direct rustybuzz + swash calls with Parley when `canvas` feature is enabled. Text output should match within 0.5px for Latin scripts.

**Consequences:**

- Positive: Cleaner text layout API
- Negative: New dependency (Parley uses fontations internally)
- Positive: Consistent text metrics reduce fingerprinting surface

### SRP / DIP Check

- **SRP:** HttpClient owns request dispatch; SharedSession owns cookie state; CSS engine handles cascade. Each module has one reason to change.
- **DIP:** Stylo integration goes behind a `CssEngine` trait so both custom and Stylo implementations can coexist during transition.
- **Dependency Injection:** SharedSession is injected via `Arc<SharedSession>` — no service locator, no global state.

## BDD/TDD Strategy

- **BDD Runner:** cucumber (Rust)
- **BDD Command:** `cargo test -p hpx-browser --test cucumber`
- **Unit Test Command:** `cargo test -p hpx-browser`
- **Property Test Tool:** proptest (already in workspace, available via `proptest` feature)
- **Fuzz Test Tool:** N/A (no new parsers in this refactor; Stylo is external)
- **Benchmark Tool:** criterion (already configured in hpx-browser benches)

## BDD Scenario Inventory

| Feature File | Scenario | Business Outcome | Task |
| :--- | :--- | :--- | :--- |
| net-api-unification | Unified request method handles GET | Single code path for all HTTP verbs | T2.1 |
| net-api-unification | Redirect following is policy-based | Redirect behavior is configurable | T2.2 |
| net-api-unification | All existing public methods still work | Backward compatibility preserved | T2.3 |
| session-explicit-injection | Page creates its own SharedSession | No global state leakage | T3.1 |
| session-explicit-injection | Two Pages share session when explicit | Multi-tab scenarios work | T3.2 |
| dom-element-safety | node() uses expect with message | Panic message is informative | T4.1 |
| stylo-css-replacement | Stylo parses real-world stylesheets | Spec compliance | T5.1 |
| parley-text-layout | Parley produces same glyph positions | Canvas output equivalent | T6.1 |

## 12. Cross-Functional Concerns

### Backward Compatibility

- All existing HttpClient public method signatures preserved during transition (deprecated)
- Default features (`canvas`, `cdp`, `v8`) remain unchanged
- Challenge detection Engine::classify signature unchanged

### Testing Strategy

- Property tests for URL redirect resolution (RFC 3986 round-trip)
- Integration tests for Stylo CSS parsing (real-world stylesheets)
- Benchmark regression check for HTTP request throughput
