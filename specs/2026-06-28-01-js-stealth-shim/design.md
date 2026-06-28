# Design: JS Stealth Shim

| Metadata | Details |
| :--- | :--- |
| **Author** | pb-plan agent |
| **Status** | Draft |
| **Created** | 2026-06-28 |
| **Reviewers** | N/A |
| **Related Issues** | N/A |

## 1. Executive Summary

**Problem:** hpx has extensive network-layer stealth (TLS impersonation, HTTP header ordering, behavioral humanization, challenge detection) but **zero JS-level stealth**. The `stealth_ext.rs` is an empty `deno_core::extension!` stub. Without JS shimming, bot managers that check `navigator.webdriver`, `window.chrome`, `navigator.plugins`, or `navigator.userAgentData` will instantly flag hpx as headless. The CLI `--stealth` flag is accepted but not wired to the CDP server or JS runtime.

**Solution:** Port obscura's JS stealth shim (`bootstrap.js`, 7485 lines) into hpx as a focused `stealth_bootstrap.js` (~800 lines) that covers the critical anti-detection surfaces: navigator identity, webdriver hiding, chrome object, screen/viewport, WebGL renderer spoofing, canvas fingerprint noise, audio context spoofing, sec-ch-ua GREASE tokens, and `navigator.userAgentData`. Wire the existing `StealthProfile` fields through Rust-side globals (`__hpx_ua`, `__hpx_platform`, `__hpx_stealth`) that the JS shim reads at page init time. Wire the `--stealth` flag through the CDP server to `BrowserJsRuntime`.

---

## 2. Source Inputs & Normalization

### 2.1 Source Materials

- Full source of obscura's JS stealth shim: `/Users/akagi201/tmp/obscura/crates/obscura-js/js/bootstrap.js` (7485 lines)
- obscura's Rust-side JS runtime: `/Users/akagi201/tmp/obscura/crates/obscura-js/src/runtime.rs` (set_stealth, set_user_agent, set_platform, run_page_init)
- hpx's existing stealth system: `crates/hpx-browser/src/stealth.rs` (2247 lines) — StealthProfile with 50+ fields, 13 presets, behavioral humanization
- hpx's JS runtime: `crates/hpx-browser/src/js_runtime/runtime.rs` — BrowserJsRuntime with deno_core
- hpx's CLI: `bin/hpx-cli/src/browser.rs` — `handle_serve` with unused `_stealth: bool`
- hpx's existing specs: `2026-06-24-01-browser-oxide-features` (stealth-profile.feature covers Rust-side validation only)

### 2.2 Normalization Approach

The user's requirement is to "review stealth mode support, compare with obscura, and absorb relevant features." After deep comparison:

- **hpx strengths (keep):** StealthProfile (50+ fields, 13 presets), TLS impersonation (Chrome/Firefox/Safari ClientHello), HTTP header ordering, behavioral humanization (mouse/keystroke/scroll), challenge detection, tracker blocklist
- **hpx gaps (fill):** JS API shimming, webdriver hiding, chrome object, screen/viewport, WebGL renderer, canvas noise, audio spoofing, sec-ch-ua GREASE, userAgentData, `--stealth` wiring
- **obscura strengths not to absorb:** wreq-based HTTP client (hpx uses its own HTTP stack), 7485-line bootstrap.js (too large — distill to essential ~800 lines)

Key constraint: hpx uses `hpx` for HTTP (not wreq/reqwest). The JS shim must be self-contained and not depend on obscura's networking.

### 2.3 Source Requirement Ledger

| Requirement ID | Source Summary | Type | Notes |
| :--- | :--- | :--- | :--- |
| `R1` | JS stealth extension must shim navigator object (userAgent, platform, plugins, mimeTypes, mediaDevices, permissions, battery, geolocation, connection, storage) | Functional | From obscura bootstrap.js lines 2796-2906 |
| `R2` | navigator.webdriver must return false and be hidden from Object.getOwnPropertyDescriptor | Functional | From obscura bootstrap.js lines 2908-2918 |
| `R3` | window.chrome must exist with app, runtime, csi(), loadTimes() matching real Chrome | Functional | From obscura bootstrap.js lines 2920-2945 |
| `R4` | screen/viewport must report realistic dimensions from StealthProfile | Functional | From obscura bootstrap.js lines 2957-2978 |
| `R5` | WebGL renderer must be spoofed to match platform (ANGLE Direct3D11/Metal/Mesa) | Functional | From obscura bootstrap.js lines 155-223 |
| `R6` | Canvas fingerprint must apply per-session noise via _fpNoise | Functional | From obscura bootstrap.js lines 5557-5604 |
| `R7` | Audio context must be spoofed (sample rate, base latency, compressor thresholds) | Functional | From obscura bootstrap.js lines 6067-6068 |
| `R8` | sec-ch-ua GREASE tokens must be derived deterministically from Chrome major version | Functional | From obscura bootstrap.js lines 2773-2794 |
| `R9` | navigator.userAgentData must return profile-consistent brands and getHighEntropyValues | Functional | From obscura bootstrap.js lines 2822-2841 |
| `R10` | Rust-side globals (__hpx_ua, __hpx_platform, __hpx_stealth) must be injected before JS init | Functional | From obscura runtime.rs set_user_agent/set_platform/set_stealth |
| `R11` | BrowserJsRuntime must call __hpx_init() after setting globals | Functional | From obscura runtime.rs run_page_init |
| `R12` | --stealth CLI flag must be wired through CDP server to BrowserJsRuntime | Functional | From obscura-cli main.rs |
| `R13` | Notification, WebGLRenderingContext, WebGL2RenderingContext stubs must exist | Functional | From obscura bootstrap.js lines 2948-2955 |
| `R14` | Function.prototype.toString must return "function name() { [native code] }" for shimmed functions | Functional | From obscura bootstrap.js |
| `R15` | Internal globals (_*, hpx) must be hidden from enumeration | Non-goal | Deferred — add when detection surface is mapped |
| `R16` | performance.memory, performance.timeOrigin must be spoofed | Functional | From obscura bootstrap.js lines 6986-6994 |

---

## 3. Requirements & Goals

### 3.1 Problem Statement

hpx's stealth system is network-complete but JS-incomplete. Bot managers cross-reference TLS fingerprints against JS-exposed browser properties. Without JS shimming, the TLS impersonation is wasted — a request with Chrome 147's TLS fingerprint but `navigator.webdriver = true` and no `window.chrome` is an instant bot flag.

### 3.2 Functional Goals

1. **JS navigator shim:** Replace navigator properties with profile-consistent values matching real Chrome behavior
2. **WebDriver hiding:** Move webdriver to prototype chain so getOwnPropertyDescriptor returns undefined
3. **Chrome object:** Provide window.chrome with app, runtime, csi(), loadTimes()
4. **Screen/viewport:** Report realistic dimensions from StealthProfile
5. **WebGL spoofing:** Return platform-appropriate GPU renderer strings
6. **Canvas noise:** Apply per-session deterministic noise to canvas operations
7. **Audio spoofing:** Randomize AudioContext parameters per-session
8. **sec-ch-ua GREASE:** Derive brand tokens deterministically from Chrome major version
9. **userAgentData:** Return profile-consistent brands and high-entropy values
10. **Rust-JS bridge:** Inject StealthProfile fields as globals, call __hpx_init()
11. **CLI wiring:** Connect --stealth flag through CDP server to JS runtime
12. **Stub objects:** Notification, WebGLRenderingContext, WebGL2RenderingContext

### 3.3 Non-Functional Goals

- **Performance:** JS shim must load once per page init, not per evaluation. Stealth_bootstrap.js should be <1000 lines (distilled from obscura's 7485).
- **Maintainability:** Each shim section must be self-contained and clearly commented. No monolithic blocks.
- **Compatibility:** Must work with deno_core's JsRuntime and the existing extension system.

### 3.4 Out of Scope

- Interactive CAPTCHA solving (Cloudflare Turnstile, hCaptcha)
- IP-based rate limiting (requires proxies — separate concern)
- WebGPU/WebAssembly fingerprint quirks
- Full 7485-line bootstrap.js port — we distill the essential ~800 lines
- wreq integration (hpx uses its own HTTP stack)
- Proxy protocol handling for SOCKS5 (separate concern)

### 3.5 Assumptions

- The deno_core JsRuntime in hpx supports `execute_script` for injecting globals before page JS runs
- The existing `stealth_extension` in `stealth_ext.rs` can be extended with ops or kept as a bootstrap-only extension
- StealthProfile fields (ua, platform, screen dimensions, GPU info) are sufficient to drive all JS shims
- The --stealth flag will be passed through the CdpServer constructor to BrowserJsRuntime

### 3.6 Code Simplification Constraints

- **Behavioral Contract:** Preserve all existing Rust-side stealth behavior. JS shim is additive.
- **Repo Standards:** Follow AGENTS.md — `thiserror` for library errors, `eyre` for app layer, no `unwrap()`/`expect()` in production code.
- **Readability Priorities:** Each JS shim section (navigator, webdriver, chrome, etc.) should be a clearly separated block with a comment header. No nested ternaries.
- **Refactor Scope:** Limited to `crates/hpx-browser/src/js_runtime/` and `bin/hpx-cli/src/browser.rs`. Do not touch existing stealth.rs, tls.rs, or headers.rs.
- **Clarity Guardrails:** The JS bootstrap file should use `// === Section Name ===` comment headers for each shim block. No minification.

---

## 4. Requirements Coverage Matrix

| Requirement ID | Covered In Design | Scenario Coverage | Task Coverage | Status / Rationale |
| :--- | :--- | :--- | :--- | :--- |
| `R1` | Section 6.4 (navigator shim) | `stealth-js-shim.feature` Scenario: Navigator identity | Task 2.1 | Covered |
| `R2` | Section 6.4 (webdriver hiding) | `stealth-js-shim.feature` Scenario: WebDriver hidden | Task 2.1 | Covered |
| `R3` | Section 6.4 (chrome object) | `stealth-js-shim.feature` Scenario: Chrome object exists | Task 2.1 | Covered |
| `R4` | Section 6.4 (screen/viewport) | `stealth-js-shim.feature` Scenario: Screen dimensions | Task 2.1 | Covered |
| `R5` | Section 6.4 (WebGL spoofing) | `stealth-js-shim.feature` Scenario: WebGL renderer | Task 2.1 | Covered |
| `R6` | Section 6.4 (canvas noise) | `stealth-js-shim.feature` Scenario: Canvas noise | Task 2.2 | Covered |
| `R7` | Section 6.4 (audio spoofing) | `stealth-js-shim.feature` Scenario: Audio context | Task 2.2 | Covered |
| `R8` | Section 6.4 (GREASE tokens) | `stealth-js-shim.feature` Scenario: sec-ch-ua tokens | Task 2.1 | Covered |
| `R9` | Section 6.4 (userAgentData) | `stealth-js-shim.feature` Scenario: userAgentData | Task 2.1 | Covered |
| `R10` | Section 6.3 (Rust-JS bridge) | `stealth-js-shim.feature` Scenario: Globals injection | Task 1.2 | Covered |
| `R11` | Section 6.3 (page init) | `stealth-js-shim.feature` Scenario: Page init | Task 1.2 | Covered |
| `R12` | Section 6.5 (CLI wiring) | `stealth-wiring.feature` Scenario: --stealth flag | Task 3.1 | Covered |
| `R13` | Section 6.4 (stubs) | `stealth-js-shim.feature` Scenario: Stub objects | Task 2.1 | Covered |
| `R14` | Section 6.4 (toString patch) | `stealth-js-shim.feature` Scenario: toString native | Task 2.3 | Covered |
| `R15` | Out of Scope | N/A | N/A | Deferred — add when detection surface is mapped |
| `R16` | Section 6.4 (performance) | `stealth-js-shim.feature` Scenario: Performance API | Task 2.1 | Covered |

---

## 5. Architecture Overview

### 5.1 System Context

```text
CLI (--stealth)
  └─> CdpServer (stealth: bool)
        └─> BrowserContext (stealth: bool)
              └─> BrowserJsRuntime
                    ├─ stealth_extension (deno_core extension)
                    ├─ stealth_bootstrap.js (injected at runtime setup)
                    └─ globals injection (__hpx_ua, __hpx_platform, __hpx_stealth)
                          └─ __hpx_init() called after globals set
```

### 5.2 Key Design Principles

- **Distillation over copy:** Port the essential ~800 lines from obscura's 7485-line bootstrap.js, not the whole thing
- **Profile-driven:** All JS shim values derive from StealthProfile fields, not hardcoded constants
- **Layered defense:** JS shim complements existing TLS/header stealth — both must be present for full coverage
- **Feature-gated:** All stealth JS code behind `#[cfg(feature = "stealth")]` to avoid bloating non-stealth builds

### 5.3 Existing Components to Reuse

| Component | Location | How to Reuse |
| :--- | :--- | :--- |
| `StealthProfile` | `crates/hpx-browser/src/stealth.rs:81` | Read fields (ua, platform, screen, gpu) to inject as JS globals |
| `BrowserJsRuntime` | `crates/hpx-browser/src/js_runtime/runtime.rs:17` | Add `set_stealth()`, `set_user_agent()`, `set_platform()`, `run_page_init()` methods |
| `stealth_extension` | `crates/hpx-browser/src/js_runtime/extensions/stealth_ext.rs:1` | Extend with bootstrap JS injection or keep as marker extension |
| `CdpServer::start` | `crates/hpx-browser/src/protocol/server.rs` | Add `stealth: bool` parameter |
| Existing bootstrap pattern | `crates/hpx-browser/src/js_runtime/runtime.rs:59-76` | Follow the `include_str!` + `execute_script` pattern for stealth_bootstrap.js |

### 5.4 Architecture Decisions

| Decision ID | Status | Selected Pattern / Principle | Why It Fits Here | Alternatives Rejected | Simplification Impact |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `AD-01` | New | **Adapter** — BrowserJsRuntime adapts StealthProfile to JS globals | Bridges Rust data model to JS runtime without coupling them | Direct field access (couples JS to Rust struct layout), config object (YAGNI for one-time init) | Keeps Rust and JS sides independent; profile changes don't break JS |
| `AD-02` | New | **Strategy** — bootstrap.js as pluggable init script | Different stealth levels could swap bootstrap files; current impl uses one | Monolithic extension ops (would require many deno_core ops for simple property sets) | Minimal Rust code; all shim logic stays in JS where it's easier to iterate |
| `AD-03` | Inherited | **Feature gate** — `#[cfg(feature = "stealth")]` | Existing pattern in hpx-browser; keeps non-stealth builds small | Runtime flag only (would compile unused JS into non-stealth builds) | Zero cost when stealth is disabled |

- **SRP Check:** BrowserJsRuntime handles JS execution only. StealthProfile handles identity data only. stealth_bootstrap.js handles JS shimming only. No module absorbs multiple concerns.
- **DIP Check:** BrowserJsRuntime depends on an抽象 concept of "stealth config" (the globals it injects), not on StealthProfile directly. The bridge is the globals injection layer.
- **Dependency Injection Plan:** StealthProfile flows into BrowserJsRuntime via `set_user_agent()`, `set_platform()`, `set_stealth()` — no direct struct dependency.

### 5.5 Project Identity Alignment

No template identity mismatches detected. All crate names match the hpx project.

### 5.6 BDD/TDD Strategy

- **BDD Runner:** `cucumber` (Rust, already used in hpx-dl)
- **BDD Command:** `cargo test -p hpx-browser --test cucumber --all-features`
- **Unit Test Command:** `cargo test -p hpx-browser --all-features`
- **Property Test Tool:** `proptest` (already in workspace) — for GREASE token determinism, canvas noise distribution
- **Fuzz Test Tool:** `cargo-fuzz` — N/A (JS shim is not parser/protocol code)
- **Benchmark Tool:** `criterion` — N/A (no explicit latency SLA for JS init)
- **Outer Loop:** `stealth-js-shim.feature` scenarios prove JS stealth works end-to-end
- **Inner Loop:** Unit tests for globals injection, GREASE derivation, profile-to-globals mapping
- **Step Definition Location:** `crates/hpx-browser/tests/` (existing pattern)

### 5.7 BDD Scenario Inventory

| Feature File | Scenario | Business Outcome | Primary Verification | Supporting TDD Focus |
| :--- | :--- | :--- | :--- | :--- |
| `stealth-js-shim.feature` | Navigator identity consistency | navigator.userAgent matches StealthProfile UA | `cargo test -p hpx-browser` | Globals injection from StealthProfile |
| `stealth-js-shim.feature` | WebDriver hidden | navigator.webdriver returns false, getOwnPropertyDescriptor returns undefined | `cargo test -p hpx-browser` | Prototype chain manipulation |
| `stealth-js-shim.feature` | Chrome object exists | window.chrome has app, runtime, csi(), loadTimes() | `cargo test -p hpx-browser` | Chrome object structure |
| `stealth-js-shim.feature` | Screen dimensions from profile | screen.width/height match StealthProfile | `cargo test -p hpx-browser` | Profile-to-screen mapping |
| `stealth-js-shim.feature` | WebGL renderer spoofed | canvas.getContext('webgl') returns platform GPU renderer | `cargo test -p hpx-browser` | GPU pool selection |
| `stealth-js-shim.feature` | sec-ch-ua GREASE tokens | navigator.userAgentData.brands has 3 entries with correct format | `cargo test -p hpx-browser` | GREASE derivation from Chrome version |
| `stealth-js-shim.feature` | userAgentData high entropy | getHighEntropyValues returns profile-consistent data | `cargo test -p hpx-browser` | High entropy values mapping |
| `stealth-js-shim.feature` | Canvas noise applied | toDataURL() returns different hashes per session | `cargo test -p hpx-browser` | Per-session seed noise |
| `stealth-js-shim.feature` | Audio context spoofed | AudioContext sample rate matches profile | `cargo test -p hpx-browser` | Audio parameter randomization |
| `stealth-wiring.feature` | --stealth flag wired | CDP server receives stealth flag, JS runtime gets stealth globals | `cargo test -p hpx-cli` | End-to-end wiring |

### 5.8 Simplification Opportunities in Touched Code

| Area | Current Complexity or Smell | Planned Simplification | Why It Preserves or Clarifies Behavior |
| :--- | :--- | :--- | :--- |
| `stealth_ext.rs` | Empty stub — dead code | Replace with bootstrap JS injection extension | Turns placeholder into functional code |
| `browser.rs:handle_serve` | `_stealth: bool` unused | Wire to CdpServer constructor | Enables existing CLI flag |
| `runtime.rs` | No stealth methods | Add set_user_agent, set_platform, set_stealth, run_page_init | Follows obscura's proven pattern |

---

## 6. Detailed Design

### 6.1 Module Structure

```text
crates/hpx-browser/src/js_runtime/
├── extensions/
│   └── stealth_ext.rs          # Extended: inject stealth_bootstrap.js
├── js/
│   ├── stealth_bootstrap.js    # NEW: ~800-line JS stealth shim
│   ├── console_bootstrap.js    # Existing
│   ├── crypto_bootstrap.js     # Existing
│   ├── dom_bootstrap.js        # Existing
│   ├── fetch_bootstrap.js      # Existing
│   ├── timer_bootstrap.js      # Existing
│   └── storage_bootstrap.js    # Existing
└── runtime.rs                  # Extended: add stealth methods

bin/hpx-cli/src/browser.rs      # Modified: wire --stealth to CDP server
```

### 6.2 Data Structures & Types

No new Rust types needed. The design reuses `StealthProfile` and adds methods to `BrowserJsRuntime`.

```rust
// Added to BrowserJsRuntime (crates/hpx-browser/src/js_runtime/runtime.rs)
impl BrowserJsRuntime {
    pub fn set_user_agent(&mut self, ua: &str) { ... }
    pub fn set_platform(&mut self, platform: &str, ua_platform: &str, ua_platform_version: &str) { ... }
    pub fn set_stealth(&mut self, enabled: bool) { ... }
    pub fn run_page_init(&mut self) { ... }
}
```

### 6.3 Interface Design

```rust
// Runtime.rs — stealth methods
pub fn set_user_agent(&mut self, ua: &str) {
    let escaped = ua.replace('\\', "\\\\").replace('\'', "\\'");
    let _ = self.inner.execute_script(
        "<set-ua>",
        format!("globalThis.__hpx_ua = '{}';", escaped),
    );
}

pub fn set_platform(&mut self, platform: &str, ua_platform: &str, ua_platform_version: &str) {
    let p = platform.replace('\'', "\\'");
    let uap = ua_platform.replace('\'', "\\'");
    let uapv = ua_platform_version.replace('\'', "\\'");
    let _ = self.inner.execute_script(
        "<set-platform>",
        format!(
            "globalThis.__hpx_platform='{}';globalThis.__hpx_ua_platform='{}';globalThis.__hpx_ua_platform_version='{}';",
            p, uap, uapv
        ),
    );
}

pub fn set_stealth(&mut self, enabled: bool) {
    let _ = self.inner.execute_script(
        "<set-stealth>",
        format!("globalThis.__hpx_stealth = {};", enabled),
    );
}

pub fn run_page_init(&mut self) {
    let _ = self.inner.execute_script(
        "<hpx:page-init>",
        "globalThis.__hpx_init();".to_string(),
    );
}
```

### 6.4 Logic Flow — JS Shim (stealth_bootstrap.js)

The bootstrap JS file is structured in clear sections:

```text
1. Fingerprint utilities (_fpSeed, _fpRand, _fpNoise, _fp)
2. navigator object (userAgent, platform, plugins, mimeTypes, mediaDevices,
   permissions, battery, geolocation, connection, storage, clipboard,
   serviceWorker, getGamepads, sendBeacon, javaEnabled)
3. navigator.webdriver hiding (prototype chain trick)
4. window.chrome object (app, runtime, csi, loadTimes)
5. Notification / WebGLRenderingContext / WebGL2RenderingContext stubs
6. Screen class and screen/viewport globals
7. sec-ch-ua GREASE brand derivation (_uaBrands)
8. navigator.userAgentData (brands, getHighEntropyValues, toJSON)
9. WebGL renderer spoofing (_getFp — platform GPU pool selection)
10. Canvas fingerprint noise (toDataURL/toBlob override)
11. Audio context spoofing (AudioContext constructor override)
12. performance.memory and performance.timeOrigin
13. __hpx_init() function — reads globals, configures runtime, deletes itself
```

Each section is preceded by a `// === Section Name ===` comment header.

### 6.5 Configuration

| Config | Source | Default | Notes |
| :--- | :--- | :--- | :--- |
| `--stealth` CLI flag | CLI args | `false` | Passed to CdpServer → BrowserJsRuntime |
| `__hpx_ua` global | StealthProfile.ua | Chrome 148 Windows UA | Set by `set_user_agent()` |
| `__hpx_platform` global | StealthProfile.platform | "Win32" | Set by `set_platform()` |
| `__hpx_ua_platform` global | StealthProfile.os_name | "Windows" | Set by `set_platform()` |
| `__hpx_ua_platform_version` global | StealthProfile.os_version | "15.0.0" | Set by `set_platform()` |
| `__hpx_stealth` global | CLI flag | `false` | Set by `set_stealth()` |

### 6.6 Error Handling

- JS bootstrap injection failure → `expect("stealth bootstrap failed")` (same pattern as existing bootstraps)
- Global injection failure → silent `_ =` (same pattern as obscura — non-critical path)
- Invalid profile fields → handled by existing `StealthProfile::validate()`

### 6.7 Maintainability Notes

- Each JS shim section is independently testable via `execute_script` + property checks
- The bootstrap file uses only ES5-compatible syntax (no arrow functions, no let/const in critical paths) for maximum V8 compatibility
- GPU renderer pools and screen dimension pools are derived from StealthProfile, not hardcoded — profile updates automatically propagate to JS

---

## 7. Verification & Testing Strategy

### 7.1 Unit Testing

- Test globals injection: set_user_agent → execute_script("globalThis.__hpx_ua") returns expected value
- Test GREASE derivation: _uaBrands() returns correct brand count and format
- Test profile-to-globals mapping: StealthProfile fields → correct global values
- Test stealth_extension init: extension loads without errors

### 7.2 Property Testing

| Target Behavior | Why Property Testing Helps | Tool / Command | Planned Invariants |
| :--- | :--- | :--- | :--- |
| GREASE brand derivation | Chrome major version → brand format must be deterministic | `proptest` in `cargo test -p hpx-browser` | Same version → same output; different versions → different GREASE chars |
| Canvas noise | Per-session noise must be unique but consistent within session | `proptest` in `cargo test -p hpx-browser` | Same seed → same noise; different seeds → different noise |

### 7.3 Integration Testing

- End-to-end: `--stealth` flag → CDP server → JS runtime → evaluate `navigator.webdriver === false`
- Profile round-trip: StealthProfile → set_user_agent/set_platform → evaluate navigator.userAgent matches

### 7.4 BDD Acceptance Testing

| Scenario ID | Feature File | Command | Success Criteria |
| :--- | :--- | :--- | :--- |
| **BDD-01** | `stealth-js-shim.feature` | `cargo test -p hpx-browser --test cucumber --all-features` | All scenarios pass |
| **BDD-02** | `stealth-wiring.feature` | `cargo test -p hpx-browser --test cucumber --all-features` | --stealth wiring scenario passes |

### 7.5 Robustness & Performance Testing

| Test Type | When It Is Required | Tool / Command | Planned Coverage or Reason Not Needed |
| :--- | :--- | :--- | :--- |
| **Fuzz** | N/A | N/A | JS shim is not parser/protocol code |
| **Benchmark** | N/A | N/A | No explicit latency SLA for JS init |

### 7.6 Critical Path Verification (The "Harness")

| Verification Step | Command | Success Criteria |
| :--- | :--- | :--- |
| **VP-01** | `cargo test -p hpx-browser --all-features` | All tests pass |
| **VP-02** | `cargo +nightly clippy -p hpx-browser --all -- -D warnings` | No lint errors |
| **VP-03** | `cargo test -p hpx-browser --test cucumber --all-features` | All BDD scenarios pass |
| **VP-04** | Manual: start CDP server with `--stealth`, evaluate `navigator.webdriver` | Returns `false` |

### 5.7 Validation Rules

| Test Case ID | Action | Expected Outcome | Verification Method |
| :--- | :--- | :--- | :--- |
| **TC-01** | Inject Chrome 148 Windows profile globals | navigator.userAgent contains "Chrome/148" | JS evaluation via BrowserJsRuntime |
| **TC-02** | Evaluate navigator.webdriver | Returns false | JS evaluation |
| **TC-03** | Evaluate Object.getOwnPropertyDescriptor(navigator, 'webdriver') | Returns undefined | JS evaluation |
| **TC-04** | Evaluate window.chrome exists | Returns object with app, runtime | JS evaluation |
| **TC-05** | Evaluate navigator.plugins.length | Returns 5 | JS evaluation |
| **TC-06** | Evaluate navigator.userAgentData.brands.length | Returns 3 | JS evaluation |
| **TC-07** | Call __hpx_init(), evaluate screen.width | Returns value from StealthProfile | JS evaluation |

---

## 6. Implementation Plan

- [ ] **Phase 1: Foundation** — Create stealth_bootstrap.js, add runtime methods, extend stealth_ext
- [ ] **Phase 2: Core Logic** — Implement all JS shim sections (navigator, webdriver, chrome, screen, WebGL, canvas, audio, GREASE, userAgentData)
- [ ] **Phase 3: Integration** — Wire --stealth through CDP server, add BDD scenarios, connect StealthProfile to globals
- [ ] **Phase 4: Polish** — Unit tests, property tests, lint clean, documentation

---

## 7. Cross-Functional Concerns

- **Security:** JS shim does not expose any new attack surface. It only sets global properties.
- **Backward Compatibility:** Non-stealth builds are unaffected (feature-gated). Existing stealth behavior (TLS, headers) is preserved.
- **Monitoring:** Add tracing::debug! when stealth globals are injected for debugging.
- **Rollback:** Remove stealth_bootstrap.js and revert stealth_ext.rs to empty stub. No data migration needed.
