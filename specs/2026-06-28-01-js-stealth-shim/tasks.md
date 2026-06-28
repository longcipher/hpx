# JS Stealth Shim — Implementation Tasks

| Metadata | Details |
| :--- | :--- |
| **Design Doc** | specs/2026-06-28-01-js-stealth-shim/design.md |
| **Owner** | pb-plan agent |
| **Start Date** | 2026-06-28 |
| **Target Date** | 2026-06-28 |
| **Status** | Planning |

## Summary & Phasing

This spec adds JS-level stealth to hpx by porting obscura's anti-detection shims. The work splits into 4 phases: (1) Foundation — create the bootstrap JS file and Rust-side bridge methods, (2) Core Logic — implement all JS shim sections, (3) Integration — wire --stealth through CLI/CDP to JS runtime, (4) Polish — tests, lint, documentation.

- **Planner Contract Rule:** Contract-complete spec in existing markdown artifacts. No sidecar schema.
- **Packet Contract Rule:** Blocked-build and DCR expectations as markdown-carried sections.
- **State Contract Rule:** `🔴 TODO` → `🟡 IN PROGRESS` → `🟢 DONE`; exceptional: `⏭️ SKIPPED`, `🔄 DCR`, `⛔ OBSOLETE`.
- **Property Testing Rule:** Add `proptest` for GREASE token determinism and canvas noise distribution.
- **Fuzzing Rule:** N/A — JS shim is not parser/protocol code.
- **Benchmark Rule:** N/A — no explicit latency SLA for JS init.
- **Identity Alignment Rule:** No template identity mismatches detected.
- **Architecture Decisions Rule:** Follow AD-01 (Adapter), AD-02 (Strategy), AD-03 (Feature gate) from design.md.
- **Dependency Injection Rule:** StealthProfile flows into BrowserJsRuntime via set_user_agent/set_platform/set_stealth — no direct struct dependency.
- **Behavior Preservation Rule:** All existing Rust-side stealth behavior preserved. JS shim is additive.
- **Simplification Focus:** Distill 7485-line obscura bootstrap.js to ~800 essential lines. Each section self-contained.
- **Clarity Guardrail:** Use `// === Section Name ===` comment headers. No minification. No nested ternaries.

---

## Phase 1: BDD Harness & Scaffolding

### Task 1.1: Create stealth_bootstrap.js scaffold

> **Context:** obscura's bootstrap.js is 7485 lines. We need to distill the essential ~800 lines into hpx's `crates/hpx-browser/src/js_runtime/js/stealth_bootstrap.js`. This file will be loaded via `include_str!` in the stealth extension, following the existing pattern used by console_bootstrap.js, dom_bootstrap.js, etc.
> **Verification:** File exists and contains all section headers. `cargo check -p hpx-browser --features stealth` passes.

- **Priority:** P0
- **Scope:** JS stealth bootstrap file
- **Requirement Coverage:** R1-R9, R13-R16
- **Scenario Coverage:** `stealth-js-shim.feature` (all scenarios)
- **Loop Type:** TDD-only
- **Behavioral Contract:** Preserve existing behavior (additive only)
- **Simplification Focus:** Distill obscura's 7485 lines to ~800 essential lines
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** Create `crates/hpx-browser/src/js_runtime/js/stealth_bootstrap.js` with section headers:
  - `// === Fingerprint Utilities ===`
  - `// === Navigator Object ===`
  - `// === WebDriver Hiding ===`
  - `// === Chrome Object ===`
  - `// === Stub Objects ===`
  - `// === Screen & Viewport ===`
  - `// === sec-ch-ua GREASE ===`
  - `// === userAgentData ===`
  - `// === WebGL Renderer ===`
  - `// === Canvas Noise ===`
  - `// === Audio Spoofing ===`
  - `// === Performance API ===`
  - `// === Page Init ===`
- [x] **Step 2:** Port fingerprint utilities from obscura: `_fpSeed`, `_fpRand(salt)`, `_fpNoise(x,y,channel)`, `_fp(name)`
- [x] **Step 3:** Port navigator object shim from obscura lines 2796-2906 (userAgent, platform, plugins, mimeTypes, mediaDevices, permissions, battery, geolocation, connection, storage, clipboard, serviceWorker, getGamepads, sendBeacon, javaEnabled)
- [x] **Step 4:** Port webdriver hiding from obscura lines 2908-2918 (prototype chain trick)
- [x] **Step 5:** Port chrome object from obscura lines 2920-2945 (app, runtime, csi, loadTimes)
- [x] **Step 6:** Port stub objects from obscura lines 2948-2955 (Notification, WebGLRenderingContext, WebGL2RenderingContext)
- [x] **Step 7:** Port screen/viewport from obscura lines 2957-2978 (Screen class, devicePixelRatio, innerWidth/Height)
- [x] **Step 8:** Port GREASE derivation from obscura lines 2773-2794 (_uaBrands function)
- [x] **Step 9:** Port userAgentData from obscura lines 2822-2841 (brands, getHighEntropyValues, toJSON)
- [x] **Step 10:** Port WebGL renderer from obscura lines 155-223 (_getFp function, GPU pools by platform)
- [x] **Step 11:** Port canvas noise from obscura lines 5557-5604 (toDataURL/toBlob override with _fpNoise)
- [x] **Step 12:** Port audio spoofing from obscura lines 6067-6068 (AudioContext constructor override)
- [x] **Step 13:** Port performance.memory and performance.timeOrigin from obscura lines 6986-6994
- [x] **Step 14:** Write `__hpx_init()` function (reads globals, configures runtime, deletes itself)
- [x] **Verification:** `cargo check -p hpx-browser --features stealth` passes. File is <1000 lines.
- [x] **Advanced Test Verification:** N/A

### Task 1.2: Add stealth methods to BrowserJsRuntime

> **Context:** `BrowserJsRuntime` (crates/hpx-browser/src/js_runtime/runtime.rs) needs set_user_agent, set_platform, set_stealth, and run_page_init methods. These follow obscura's proven pattern from runtime.rs lines 219-254. The methods inject globals that stealth_bootstrap.js reads at init time.
> **Verification:** `cargo test -p hpx-browser --all-features` passes. New methods are callable.

- **Priority:** P0
- **Scope:** Rust JS runtime bridge
- **Requirement Coverage:** R10, R11
- **Scenario Coverage:** `stealth-js-shim.feature` Scenario: Globals injection
- **Loop Type:** TDD-only
- **Behavioral Contract:** Preserve existing behavior (additive only)
- **Simplification Focus:** Follow obscura's proven pattern exactly
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** Add `set_user_agent(&mut self, ua: &str)` to BrowserJsRuntime — escapes quotes, injects `globalThis.__hpx_ua`
- [x] **Step 2:** Add `set_platform(&mut self, platform: &str, ua_platform: &str, ua_platform_version: &str)` — injects `globalThis.__hpx_platform`, `__hpx_ua_platform`, `__hpx_ua_platform_version`
- [x] **Step 3:** Add `set_stealth(&mut self, enabled: bool)` — injects `globalThis.__hpx_stealth`
- [x] **Step 4:** Add `run_page_init(&mut self)` — executes `globalThis.__hpx_init()`
- [x] **Step 5:** Add unit tests for each method: inject → read back via execute_script
- [x] **Verification:** `cargo test -p hpx-browser --all-features` passes with new tests
- [x] **Advanced Test Verification:** N/A

---

## Phase 2: Scenario Implementation

### Task 2.1: Implement navigator, webdriver, chrome, screen, GREASE, userAgentData shims

> **Context:** The JS bootstrap file needs working implementations for the core anti-detection surfaces. These are the highest-priority shims because they cover the most commonly checked bot-detection signals.
> **Verification:** JS evaluation tests pass for navigator.userAgent, navigator.webdriver, window.chrome, screen.width, navigator.userAgentData.brands.

- **Priority:** P0
- **Scope:** JS stealth implementation
- **Requirement Coverage:** R1-R5, R8-R9, R13, R16
- **Scenario Coverage:** `stealth-js-shim.feature` Scenarios: Navigator identity, WebDriver hidden, Chrome object, Screen dimensions, sec-ch-ua tokens, userAgentData, Stub objects, Performance API
- **Loop Type:** BDD+TDD
- **Behavioral Contract:** Preserve existing behavior (additive only)
- **Simplification Focus:** Each shim section independently testable
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** Implement navigator shim — userAgent reads `__hpx_ua`, platform reads `__hpx_platform`, plugins returns 5 PDF plugins, mimeTypes returns 2 PDF types
- [x] **Step 2:** Implement webdriver hiding — prototype chain trick so getOwnPropertyDescriptor returns undefined
- [x] **Step 3:** Implement chrome object — app, runtime, csi() with randomized timing, loadTimes() with realistic timestamps
- [x] **Step 4:** Implement screen/viewport — Screen class reads from globals, default 1920x1080
- [x] **Step 5:** Implement WebGL renderer — _getFp() selects platform-appropriate GPU from pool
- [x] **Step 6:** Implement GREASE derivation — _uaBrands() derives from Chrome major version
- [x] **Step 7:** Implement userAgentData — brands from _uaBrands, getHighEntropyValues returns profile data
- [x] **Step 8:** Implement stub objects — Notification, WebGLRenderingContext, WebGL2RenderingContext
- [x] **Step 9:** Implement performance.memory and performance.timeOrigin
- [x] **Step 10:** Write unit tests for each shim section
- [x] **BDD Verification:** Run `stealth-js-shim.feature` scenarios for navigator, webdriver, chrome, screen, GREASE, userAgentData, stubs, performance
- [x] **Verification:** `cargo test -p hpx-browser --all-features` passes
- [x] **Advanced Test Verification:** N/A

### Task 2.2: Implement canvas noise and audio spoofing

> **Context:** Canvas and audio fingerprinting are secondary detection surfaces. Canvas noise must be per-session deterministic (same seed → same output). Audio spoofing must randomize sample rate and compressor thresholds.
> **Verification:** Canvas toDataURL returns different hashes per session. AudioContext sample rate matches profile.

- **Priority:** P1
- **Scope:** JS stealth implementation
- **Requirement Coverage:** R6, R7
- **Scenario Coverage:** `stealth-js-shim.feature` Scenarios: Canvas noise, Audio context
- **Loop Type:** BDD+TDD
- **Behavioral Contract:** Preserve existing behavior (additive only)
- **Simplification Focus:** Use existing _fpRand for deterministic noise
- **Advanced Test Coverage:** Property (proptest for noise uniqueness)
- **Status:** 🟢 DONE
- [x] **Step 1:** Implement canvas toDataURL/toBlob override with _fpNoise per-pixel noise
- [x] **Step 2:** Implement AudioContext constructor override — randomize sampleRate, baseLatency, compressor thresholds
- [x] **Step 3:** Write unit tests: same seed → same canvas hash; different seeds → different hashes
- [x] **Step 4:** Write proptest: canvas noise is unique per session seed
- [x] **BDD Verification:** Run `stealth-js-shim.feature` scenarios for canvas and audio
- [x] **Verification:** `cargo test -p hpx-browser --all-features` passes
- [x] **Advanced Test Verification:** `cargo test -p hpx-browser -- proptest` passes

### Task 2.3: Wire stealth_bootstrap.js into BrowserJsRuntime

> **Context:** The existing pattern loads bootstrap JS via `include_str!` + `execute_script` in `BrowserJsRuntime::with_base_url()`. stealth_bootstrap.js must be loaded the same way, but only when stealth is enabled.
> **Verification:** When stealth extension is initialized, stealth_bootstrap.js globals are available.

- **Priority:** P0
- **Scope:** Rust-JS integration
- **Requirement Coverage:** R10, R11
- **Scenario Coverage:** `stealth-js-shim.feature` Scenario: Globals injection
- **Loop Type:** TDD-only
- **Behavioral Contract:** Preserve existing behavior (additive only)
- **Simplification Focus:** Follow existing bootstrap loading pattern
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** Update `stealth_ext.rs` to include the bootstrap JS as a deno_core extension that runs `include_str!("js/stealth_bootstrap.js")`
- [x] **Step 2:** Ensure stealth_bootstrap.js is loaded in `BrowserJsRuntime::with_base_url()` after existing bootstraps
- [x] **Step 3:** Write test: create BrowserJsRuntime, call set_user_agent + run_page_init, evaluate `globalThis.__hpx_ua`
- [x] **Verification:** `cargo test -p hpx-browser --all-features` passes
- [x] **Advanced Test Verification:** N/A

---

## Phase 3: Integration & Features

### Task 3.1: Wire --stealth flag through CDP server

> **Context:** `bin/hpx-cli/src/browser.rs:handle_serve` receives `_stealth: bool` but ignores it. The CdpServer constructor needs a `stealth: bool` parameter that propagates to BrowserJsRuntime.
> **Verification:** Start CDP server with `--stealth`, connect via CDP, evaluate `navigator.webdriver === false`.

- **Priority:** P0
- **Scope:** CLI → CDP → JS runtime wiring
- **Requirement Coverage:** R12
- **Scenario Coverage:** `stealth-wiring.feature` Scenario: --stealth flag wired
- **Loop Type:** BDD+TDD
- **Behavioral Contract:** Preserve existing behavior (stealth=false by default)
- **Simplification Focus:** Minimal change — pass bool through constructor chain
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** Add `stealth: bool` parameter to `CdpServer::start()`
- [x] **Step 2:** Pass `stealth` through to `BrowserContext` or page creation
- [x] **Step 3:** In page creation, if stealth is true: extract StealthProfile fields, call set_user_agent/set_platform/set_stealth on BrowserJsRuntime, then run_page_init
- [x] **Step 4:** Update `handle_serve` to pass `_stealth` (rename to `stealth`) to `CdpServer::start()`
- [x] **Step 5:** Add `stealth: bool` to CLI args on `serve` subcommand (already exists as `_stealth`, just wire it)
- [x] **Step 6:** Write integration test: CDP server with stealth=true → evaluate navigator.webdriver
- [x] **BDD Verification:** Run `stealth-wiring.feature` scenario
- [x] **Verification:** `cargo test -p hpx-browser --all-features` and `cargo test -p hpx-cli --all-features` pass
- [x] **Runtime Verification:** Start server with `cargo run -p hpx-cli -- serve --stealth`, connect via CDP, verify navigator.webdriver === false
- [x] **Advanced Test Verification:** N/A

### Task 3.2: Connect StealthProfile to JS globals

> **Context:** When a page is created with a StealthProfile, the profile's fields (ua, platform, os_name, os_version, screen dimensions, GPU info) must be injected as JS globals before __hpx_init() runs.
> **Verification:** Create page with Chrome 148 macOS profile → navigator.userAgent contains macOS UA, screen matches profile dimensions.

- **Priority:** P0
- **Scope:** Profile-to-globals bridge
- **Requirement Coverage:** R10, R11
- **Scenario Coverage:** `stealth-js-shim.feature` Scenario: Navigator identity consistency
- **Loop Type:** BDD+TDD
- **Behavioral Contract:** Preserve existing behavior (additive only)
- **Simplification Focus:** Read profile fields directly, no intermediate mapping
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** In page creation with profile: extract `profile.user_agent()`, `profile.platform()`, `profile.os_name()`, `profile.os_version()`
- [x] **Step 2:** Call `runtime.set_user_agent(ua)`, `runtime.set_platform(platform, os_name, os_version)`
- [x] **Step 3:** Call `runtime.set_stealth(true)`
- [x] **Step 4:** Call `runtime.run_page_init()`
- [x] **Step 5:** Write test: Chrome 148 macOS profile → evaluate navigator.userAgent contains "Macintosh"
- [x] **BDD Verification:** Run `stealth-js-shim.feature` scenarios
- [x] **Verification:** `cargo test -p hpx-browser --all-features` passes
- [x] **Advanced Test Verification:** N/A

---

## Phase 4: Polish, QA & Docs

### Task 4.1: Add BDD feature files

> **Context:** The existing `stealth-profile.feature` covers Rust-side validation only. We need `stealth-js-shim.feature` for JS shim behavior and `stealth-wiring.feature` for CLI integration.
> **Verification:** BDD scenarios run and pass.

- **Priority:** P1
- **Scope:** BDD acceptance tests
- **Requirement Coverage:** R1-R16
- **Scenario Coverage:** All scenarios in stealth-js-shim.feature and stealth-wiring.feature
- **Loop Type:** BDD+TDD
- **Behavioral Contract:** Preserve existing behavior
- **Simplification Focus:** Thin BDD steps, route through Rust unit tests
- **Advanced Test Coverage:** Example-based only
- **Status:** 🟢 DONE
- [x] **Step 1:** Create `specs/2026-06-28-01-js-stealth-shim/features/stealth-js-shim.feature` with scenarios
- [x] **Step 2:** Create `specs/2026-06-28-01-js-stealth-shim/features/stealth-wiring.feature` with scenarios
- [x] **Step 3:** Add step definitions in `crates/hpx-browser/tests/` if cucumber-rs is set up
- [x] **Step 4:** Run `cargo test -p hpx-browser --test cucumber --all-features` and verify all pass
- [x] **BDD Verification:** All scenarios pass
- [x] **Verification:** `just bdd` passes
- [x] **Advanced Test Verification:** N/A

### Task 4.2: Lint, format, and final verification

> **Context:** Final quality gate before marking the spec complete.
> **Verification:** All quality checks pass.

- **Priority:** P1
- **Scope:** QA
- **Requirement Coverage:** N/A (quality gate)
- **Scenario Coverage:** N/A
- **Loop Type:** TDD-only
- **Behavioral Contract:** Preserve existing behavior
- **Simplification Focus:** N/A
- **Advanced Test Coverage:** N/A
- **Status:** 🟢 DONE
- [x] **Step 1:** Run `cargo +nightly fmt --all` and format
- [x] **Step 2:** Run `cargo +nightly clippy -p hpx-browser --all -- -D warnings` — fix any warnings
- [x] **Step 3:** Run `cargo test -p hpx-browser --all-features` — all pass
- [x] **Step 4:** Run `cargo test -p hpx-cli --all-features` — all pass
- [x] **Step 5:** Run `just test-all` — all pass
- [x] **Verification:** All lint/format/test commands pass
- [x] **Advanced Test Verification:** N/A

---

## Summary & Timeline

| Phase | Tasks | Target Date |
| :--- | :---: | :--- |
| **1. Foundation** | 2 | 2026-06-28 |
| **2. Core Logic** | 3 | 2026-06-28 |
| **3. Integration** | 2 | 2026-06-28 |
| **4. Polish** | 2 | 2026-06-28 |
| **Total** | **9** | |

## Definition of Done

1. [ ] **Linted:** No lint errors (`cargo +nightly clippy` passes)
2. [ ] **Tested:** Unit tests covering all JS shim sections
3. [ ] **Formatted:** `cargo +nightly fmt --all` applied
4. [ ] **Verified:** All task-specific Verification criteria met
5. [ ] **Advanced-Tested:** proptest for canvas noise and GREASE determinism
6. [ ] **Runtime-Evidenced:** CDP server with --stealth → navigator.webdriver === false
7. [ ] **Behavior-Preserved:** Existing stealth behavior unchanged
8. [ ] **Simplified:** Bootstrap.js <1000 lines, each section self-contained
