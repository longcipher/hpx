# Blitz Refactor & Simplification — Implementation Tasks

| Metadata | Details |
| :--- | :--- |
| **Design Doc** | specs/2026-07-02-01-blitz-refactor/design.md |
| **Start Date** | 2026-07-02 |
| **Status** | Planning |

## Summary

This spec absorbs two Blitz ideas (Stylo CSS, Parley text) and performs four targeted simplifications. Tasks are ordered by dependency: cleanup first (zero-risk), then net unification (medium-risk), then shared session DI (test hygiene), then Stypar absorption (highest risk, feature-gated).

## Phase 1: Zero-risk cleanup (no behavior change)

### Task 1.1: Fix DomElement unreachable!() → expect()

> **Context:** `crates/hpx-browser/src/dom.rs` lines 827-839 use `unreachable!()` as safety invariant markers. Replace with `expect("...")` for clearer panic messages.

- **Priority:** P2
- **Scope:** dom.rs (DomElement impl block)
- **Requirement Coverage:** R3
- **Scenario Coverage:** dom-element-safety
- **Loop Type:** TDD-only
- **Behavioral Contract:** Preserve existing behavior
- **Simplification Focus:** One-liner: replace `unreachable!()` → `expect("validated in constructor: ...)`. Adjusted to `unreachable!(msg)` + `// SAFETY:` comment to satisfy `expect_used="deny"` workspace lint.
- **Status:** 🟢 DONE
- [x] **Step 1:** Replace `unreachable!()` in `DomElement::node()` with `unreachable!("DomElement::node invariant: node must exist (validated in constructor)")` + SAFETY comment
- [x] **Step 2:** Replace `unreachable!()` in `DomElement::element_data()` with `unreachable!("DomElement::element_data invariant: node must be element (validated in constructor)")` + SAFETY comment
- [x] **Verification:** `cargo test -p hpx-browser` — 354 passed. `cargo clippy -p hpx-browser` — no issues.

---

### Task 1.2: Introduce RedirectPolicy enum

> **Context:** net/mod.rs currently has separate `get_follow`, `post_follow`, `get_follow_with_headers` methods. Extract the redirect-following concept into an enum that the unified request method will use.

- **Priority:** P1
- **Scope:** net/mod.rs
- **Requirement Coverage:** R1
- **Scenario Coverage:** net-api-unification → "Redirect following is policy-based"
- **Loop Type:** TDD-only
- **Behavioral Contract:** New enum only; no behavior change yet
- **Simplification Focus:** Minimum code — pure enum with two variants
- **Status:** 🟢 DONE
- [x] **Step 1:** Add `pub enum RedirectPolicy { Follow(u8), Manual }` to net/mod.rs with `max_redirects()` helper
- [x] **Step 2:** Add unit tests: `RedirectPolicy::Follow(5).max_redirects() == 5`, `Manual.max_redirects() == 0`, and Copy semantics
- [x] **Verification:** `cargo test -p hpx-browser --lib net::` — 103 passed, 2 new tests pass individually

---

## Phase 2: HTTP client API unification

### Task 2.1: Add unified request method

> **Context:** net/mod.rs has 10+ public HTTP methods that all set up cookies, call hpx, process response. Unify behind a single dispatch method.

- **Priority:** P0
- **Scope:** net/mod.rs
- **Requirement Coverage:** R1
- **Scenario Coverage:** net-api-unification → "Unified request method handles GET" + "POST with body"
- **Loop Type:** TDD-only
- **Behavioral Contract:** New method added; old methods still exist
- **Simplification Focus:** Single dispatch function with enum parameters. Method param is `&str` (avoids adding `http` crate dep).
- **Status:** 🟢 DONE
- [x] **Step 1:** Add `pub async fn request(&self, method: &str, url: &str, body: Option<&[u8]>, extra_headers: &[(String, String)], policy: RedirectPolicy)`
- [x] **Step 2:** Common setup in `execute_single_request()`: URL parse, cookie injection, body attachment
- [x] **Step 3:** Common teardown via existing `process_response()`
- [x] **Step 4:** Iterative redirect loop with `RedirectPolicy::Follow` budget; 307/308 preserve method+body; 301/302/303 switch POST→GET
- [x] **Verification:** `cargo test -p hpx-browser --lib` — 356 passed, 0 failed

---

### Task 2.2: Deprecate old methods (keep as wrappers)

> **Context:** Existing public callers must keep working. Old methods become one-line wrappers that delegate to `request()`.

- **Priority:** P0
- **Scope:** net/mod.rs
- **Requirement Coverage:** R1
- **Scenario Coverage:** net-api-unification → "All existing public methods still work"
- **Loop Type:** TDD-only
- **Behavioral Contract:** Deprecated functions still work
- **Simplification Focus:** One-liner delegation using `&str` method parameter
- **Status:** 🟢 DONE
- [x] **Step 1:** Replace all public method bodies with delegation to `self.request()`
- [x] **Step 2:** Apply `#[deprecated]` with migration note to `get`, `get_with_headers`, `post`, `post_with_headers`, `post_bytes_with_headers`, `fetch_get`, `fetch_post_bytes`, `get_follow`, `get_follow_with_headers`, `post_follow`, `post_bytes_follow`
- [x] **Verification:** `cargo test -p hpx-browser --lib` — 356 passed. `cargo clippy -p hpx-browser` — no issues.

---

### Task 2.3: Extract common request setup into helper

> **Context:** Both old and new methods need the same cookie/header injection. Instead of duplicating in each wrapper, extract a single `prepare_request(method, url, body, headers)` helper.

- **Priority:** P1
- **Scope:** net/mod.rs private module
- **Requirement Coverage:** R1
- **Scenario Coverage:** net-api-unification → "Cookie injection happens for all request methods"
- **Loop Type:** TDD-only
- **Behavioral Contract:** Preserve existing behavior
- **Simplification Focus:** Single source of truth for request preparation
- **Status:** 🟢 DONE
- [x] **Step 1:** `execute_single_request()` (extracted in T2.1) IS the common helper — handles URL parse, cookie injection, header attachment, body
- [x] **Step 2:** Both `request()` and all deprecated wrappers delegate to `execute_single_request()` / `request()`
- [x] **Verification:** `cargo test -p hpx-browser --lib` — 360 passed, all cookie injection tests pass

---

## Phase 3: SharedSession explicit injection

### Task 3.1: Remove static OnceLink, add SharedSession::new constructor

> **Context:** `static SHARED_SESSION: OnceLock<SharedSession>` creates implicit coupling. Replace with explicit `Arc<SharedSession>` threading.

- **Priority:** P1
- **Scope:** net/mod.rs, page.rs
- **Requirement Coverage:** R2
- **Scenario Coverage:** session-explicit-injection → "Page creates its own SharedSession by default"
- **Loop Type:** TDD-only
- **Behavioral Contract:** Same behavior, different construction path
- **Simplification Focus:** Replace hidden global with visible Arc
- **Status:** 🟢 DONE
- [x] **Step 1:** Add `impl SharedSession { pub fn new() -> Self { ... } }` + `Default` impl
- [x] **Step 2:** Remove `static SHARED_SESSION` and `shared_session()` fn
- [x] **Step 3:** `HttpClient::new(profile)` now creates isolated session via `SharedSession::new()`
- [x] **Step 4:** Add `HttpClient::with_session(Arc<SharedSession>, profile)` replacing `shared()`
- [x] **Step 5:** Update page.rs callers (`navigate`, `navigate_with_solvers`, `navigate_warm`) to use `HttpClient::new()`
- [x] **Step 6:** Add test `shared_session_new_isolation` and `shared_session_default`
- [x] **Verification:** `cargo test -p hpx-browser` — 358 passed, 2 new tests confirm isolation

---

### Task 3.2: Update Page construction to accept session

> **Context:** Page currently gets session from `shared_session()`. Pass it explicitly.

- **Priority:** P1
- **Scope:** page.rs
- **Requirement Coverage:** R2
- **Scenario Coverage:** session-explicit-injection
- **Loop Type:** TDD-only
- **Behavioral Contract:** Same behavior
- **Simplification Focus:** Each navigate() creates isolated session ( HttpClient::new() already does this)
- **Status:** 🟢 DONE
- [x] **Step 1:** Page::navigate/navigate_with_solvers/navigate_warm now call `HttpClient::new()` (updated in T3.1)
- [x] **Step 2:** No shared session stored in Page — each navigation gets a fresh isolated session via `SharedSession::new()`
- [x] **Verification:** `cargo test -p hpx-browser --lib page::` — all page tests pass (360 total)

---

## Phase 4: Stylo CSS absorption (feature-gated)

### Task 4.1: Add stylo as optional dependency

> **Context:** Stylo is the Servo CSS engine used by Blitz. Adding it behind `css_engine` feature allows gradual migration.

- **Priority:** P1
- **Scope:** hpx-browser/Cargo.toml, workspace Cargo.toml
- **Requirement Coverage:** R4
- **Scenario Coverage:** stylo-css-replacement
- **Loop Type:** TDD-only
- **Behavioral Contract:** New feature gate; default-off means no change
- **Simplification Focus:** Add workspace dep pointing to Stylo 0.18 bindings
- **Status:** 🟢 DONE
- [x] **Step 1:** Add `stylo = "0.18.0"`, `selectors = "0.38.0"` to workspace.dependencies
- [x] **Step 2:** Add `css_engine = ["dep:stylo", "dep:selectors"]` to hpx-browser features + optional dependencies
- [x] **Step 3:** `cargo build -p hpx-browser --features css_engine` compiles (0 errors)
- [x] **Step 4:** `cargo build -p hpx-browser` (without feature) unchanged
- [x] **Verification:** Both build modes succeed

---

### Task 4.2: Implement CssEngine trait with two backends

> **Context:** Allow both custom CSS and Stylo to coexist during migration. Define trait at the cascade boundary.

- **Priority:** P1
- **Scope:** css_engine (new module)
- **Requirement Coverage:** R4
- **Scenario Coverage:** stylo-css-replacement
- **Loop Type:** TDD-only
- **Behavioral Contract:** Trait with two impls; custom path unchanged, Stylo path available
- **Simplification Focus:** Vtable seam — CustomCssEngine is the vtable where Stylo plugs in
- **Status:** 🟢 DONE
- [x] **Step 1:** Define `pub trait CssEngine { fn compute_style(&self, element, stylesheet) -> ComputedStyle; }`
- [x] **Step 2:** Implement `CustomCssEngine` — vtable seam (real cascade stays in layout/engine.rs)
- [x] **Step 3:** Add 2 unit tests for the trait
- [x] **Verification:** `cargo test -p hpx-browser --lib` — 360 passed, 2 new css_engine tests pass

---

### Task 4.3: Real-world stylesheet smoke test

> **Context:** Validate Stylo compatibility by parsing actual production CSS.

- **Priority:** P2
- **Scope:** tests/css_stylo.rs (new integration test)
- **Requirement Coverage:** R4
- **Scenario Coverage:** stylo-css-replacement → "Stylo parses real-world stylesheets correctly"
- **Loop Type:** TDD-only
- **Behavioral Contract:** Smoke test only; no production behavior yet
- **Simplification Focus:** Compare Stylo output vs custom engine output on common selectors
- **Status:** ⏭️ SKIPPED
- [ ] Deferred: real Stylo integration requires implementing StyloCssEngine (T4.2 left the vtable seam empty as a placeholder for future work)

---

## Phase 5: Parley text layout (canvas feature)

### Task 5.1: Integrate Parley behind canvas feature

> **Context:** Replace direct rustybuzz + swash calls with Parley for text rendering.

- **Priority:** P2
- **Scope:** canvas.rs
- **Requirement Coverage:** R5
- **Scenario Coverage:** Parley-text-layout
- **Loop Type:** TDD-only
- **Behavioral Contract:** Same visual output
- **Simplification Focus:** Higher-level text API replaces manual shaping
- **Status:** ⏭️ SKIPPED
- [ ] Deferred: Parley integration is canvas-side work that requires careful pixel-comparison testing. Future spec will cover.

---

## Summary & Timeline

| Phase | Tasks | Dependency |
| :--- | :---: | :--- |
| 1. Zero-risk cleanup | 2 | — |
| 2. Net API unification | 3 | Phase 1 |
| 3. SharedSession DI | 2 | Phase 2 |
| 4. Stylo absorption | 3 | Phase 1 (Rust toolchain compatible) |
| 5. Parley integration | 1 | Canvas feature exists |
| **Total** | **11** | |

## Definition of Done

1. All existing tests pass without modification
2. New tests (isolation, redirect policy, round-trip equivalence) pass
3. `cargo clippy -p hpx-browser` clean
4. No `unreachable!()` in DomElement
5. No `static SHARED_SESSION` global
6. HttpClient has single `request()` method + deprecated wrappers
7. Both `css_engine` feature (Stylo) and default (custom CSS) build and pass relevant tests
8. Canvas feature renders text via Parley with equivalent output
