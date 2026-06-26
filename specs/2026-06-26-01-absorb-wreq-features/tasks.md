# Absorb wreq/wreq-Util Features — Implementation Tasks

| Metadata | Details |
| :--- | :--- |
| **Design Doc** | specs/2026-06-26-01-absorb-wreq-features/design.md |
| **Status** | Planning |
| **Mode** | Full |

## Summary & Phasing

Four phases: (1) Foundation fixes that don't depend on new profiles, (2) Browser version coverage expansion, (3) Emulation enhancements (platform, weighted random, cert compression, pool Group), (4) Polish and integration verification.

- **Phase 1:** Foundation — Platform enum, error source chain walking, Response::forbid_recycle()
- **Phase 2:** Browser Profiles — Add ~40 missing browser versions to hpx-emulation
- **Phase 3:** Emulation Enhancements — Weighted random, cert compression, Group pool key
- **Phase 4:** Polish, QA & Docs — Clippy, integration tests, regression verification

---

## Phase 1: Foundation

### Task 1.1: Add Platform Enum to hpx-emulation

> **Context:** wreq-util has `Platform` (Windows/macOS/Linux/Android/iOS) that affects UA strings and `sec-ch-ua-platform` headers. hpx-emulation has `EmulationOS` internally but no user-friendly `Platform` API. This task adds the public `Platform` enum and wires it into `EmulationOption`.
> **Verification:** `cargo test -p hpx-emulation` passes; `Platform::Linux` produces correct `EmulationOS` mapping.

- **Priority:** P0
- **Scope:** hpx-emulation — new enum + EmulationOption field
- **Requirement Coverage:** `R2` / REQ-02
- **Scenario Coverage:** Platform-specific emulation
- **Loop Type:** TDD-only
- **Behavioral Contract:** Preserve existing behavior — `EmulationOption` defaults unchanged; new `platform()` builder method is additive.
- **Simplification Focus:** YAGNI — only 5 platform variants matching real TLS/UA differences. Skip per-platform TLS cipher list overrides for now (ponytail: add if fingerprint testing reveals differences).
- **Status:** 🟢 DONE
- [x] Step 1: Add `Platform` enum to `crates/hpx-emulation/src/emulation.rs` with `Windows`, `MacOS`, `Linux`, `Android`, `IOS` variants, `Default` derive (Windows), and `is_mobile()` method.
- [x] Step 2: Add `platform` field to `EmulationOption` struct (default: `Platform::default()`).
- [x] Step 3: Implement `From<Platform> for EmulationOS` conversion.
- [x] Step 4: Add `platform()` builder method to `EmulationOption`.
- [x] Step 5: Unit tests: `is_mobile()` for each variant, `Platform→EmulationOS` conversion, `EmulationOption::builder().platform(Platform::Linux).build()`.
- [x] **BDD Verification:** `cargo test -p hpx-emulation -- platform` — all platform tests pass
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 1.2: Error Source Chain Walking for Public Error

> **Context:** `hpx::Error::is_timeout()` only checks direct `Kind::Request` + `TimedOut` source. After tower timeout/elapse wrapping, the timeout may be nested deeper. `walk_source_chain()` exists at `error.rs:300` but isn't used by all diagnostic methods. This task refactors `is_timeout()`, `is_connect()`, `is_dns()` to use source chain walking.
> **Verification:** Unit tests with 3-level nested errors pass.

- **Priority:** P0
- **Scope:** hpx — error.rs diagnostic methods
- **Requirement Coverage:** `R4` / REQ-04
- **Scenario Coverage:** Timeout/connect/DNS detection through tower layers
- **Loop Type:** TDD-only
- **Behavioral Contract:** Preserve existing behavior — direct matches still return true quickly. Chain walking adds correctness for nested errors, doesn't change existing results.
- **Simplification Focus:** One-liner refactor — reuse existing `walk_source_chain` helper, just call it from `is_timeout()`/`is_connect()`/`is_dns()`.
- **Status:** 🟢 DONE
- [x] Step 1: Refactor `hpx::Error::is_timeout()` to: check direct kind first (fast path), then `walk_source_chain` looking for `TimedOut` or `io::ErrorKind::TimedOut`.
- [x] Step 2: Refactor `hpx::Error::is_connect()` to: check direct kind first, then walk chain looking for connect-related error types.
- [x] Step 3: Add `hpx::Error::is_dns()` method that walks chain looking for DNS resolution errors.
- [x] Step 4: Unit tests: 3-level nested timeout error, nested connect error, nested DNS error, direct match still works, non-matching chain returns false.
- [x] **BDD Verification:** `cargo test -p hpx -- is_timeout` and `cargo test -p hpx -- is_connect` pass
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 1.3: Response::forbid_recycle()

> **Context:** wreq has `Response::forbid_recycle()` which poisons the underlying connection to prevent pool reuse. hpx has `PoisonPill` (AtomicBool) on `Connected`. This task adds the public API to `Response`.
> **Verification:** After calling `forbid_recycle()`, the connection is not returned to the pool.

- **Priority:** P1
- **Scope:** hpx — response.rs
- **Requirement Coverage:** `R6` / REQ-05
- **Scenario Coverage:** Connection poisoning from response side
- **Loop Type:** TDD-only
- **Behavioral Contract:** Preserve existing behavior — new method only, no existing signatures change.
- **Simplification Focus:** Check if response extensions already carry `Connected` info. If so, add `PoisonPill` reference to extensions at response creation time. If not, store it as an optional field.
- **Status:** 🟢 DONE
- [x] Step 1: Investigate how `Response` accesses its connection info (check `Response` extensions, `from_client_response` path).
- [x] Step 2: Ensure `PoisonPill` is accessible from `Response` (add to response extensions if needed).
- [x] Step 3: Add `Response::forbid_recycle(&self)` method that sets the PoisonPill atomic flag.
- [x] Step 4: Unit test: create response, call `forbid_recycle()`, verify PoisonPill is set.
- [x] **BDD Verification:** `cargo test -p hpx -- forbid_recycle` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

---

## Phase 2: Browser Profiles

### Task 2.1: Add Chrome v144-v149 Profiles

> **Context:** hpx-emulation has Chrome up to v143. wreq-util covers up to v149. Each version needs TLS options, HTTP/2 settings, and headers. Use existing `mod_generator!` macro pattern.
> **Verification:** `Emulation::Chrome144` through `Chrome149` variants exist and produce valid `hpx::Emulation`.

- **Priority:** P0
- **Scope:** hpx-emulation — new device/chrome modules
- **Requirement Coverage:** `R1` / REQ-01
- **Scenario Coverage:** All major browser versions available
- **Loop Type:** TDD-only
- **Behavioral Contract:** Preserve existing behavior — new enum variants only.
- **Simplification Focus:** Copy existing Chrome v143 module, adjust version-specific constants (cipher list, HTTP/2 settings, UA string). Use `mod_generator!` macro for boilerplate.
- **Status:** 🟢 DONE
- [x] Step 1: Study existing Chrome version module structure (e.g., `device/chrome/chrome143/`).
- [x] Step 2: Create `chrome144` through `chrome149` modules using `mod_generator!` macro with appropriate TLS/HTTP2/header configs.
- [x] Step 3: Add `Chrome144` through `Chrome149` variants to the `Emulation` enum via `define_enum!`.
- [x] Step 4: Unit test: each variant produces a valid `hpx::Emulation` with non-empty headers.
- [x] **BDD Verification:** `cargo test -p hpx-emulation -- chrome14` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 2.2: Add Firefox v137-v151 Profiles

> **Context:** hpx-emulation has Firefox up to v136. wreq-util covers up to v151. Includes Private Browsing and Android variants.
> **Verification:** `Emulation::Firefox137` through `Firefox151` variants exist.

- **Priority:** P0
- **Scope:** hpx-emulation — new device/firefox modules
- **Requirement Coverage:** `R1` / REQ-01
- **Scenario Coverage:** All major browser versions available
- **Loop Type:** TDD-only
- **Behavioral Contract:** Preserve existing behavior.
- **Simplification Focus:** Copy existing Firefox v136 module, adjust. Include Private Browsing and Android variants where wreq-util has them.
- **Status:** 🟢 DONE
- [x] Step 1: Study existing Firefox version module structure.
- [x] Step 2: Create `firefox137` through `firefox151` modules.
- [x] Step 3: Add enum variants via `define_enum!`.
- [x] Step 4: Unit tests.
- [x] **BDD Verification:** `cargo test -p hpx-emulation -- firefox1` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 2.3: Add Safari v19-v26.4 Profiles + iOS/iPad Variants

> **Context:** hpx-emulation has Safari up to v18. wreq-util covers 15.3-26.4 with iOS and iPad variants. Safari fingerprints differ significantly between macOS and iOS.
> **Verification:** `Emulation::Safari19` through `Safari26_4` plus iOS variants exist.

- **Priority:** P0
- **Scope:** hpx-emulation — new device/safari modules
- **Requirement Coverage:** `R1` / REQ-01
- **Scenario Coverage:** All major browser versions available
- **Loop Type:** TDD-only
- **Behavioral Contract:** Preserve existing behavior.
- **Simplification Focus:** Group Safari iOS variants with their macOS counterparts using the same TLS/HTTP2 presets but different UA strings.
- **Status:** 🟢 DONE
- [x] Step 1: Study existing Safari version module structure.
- [x] Step 2: Create `safari19` through `safari26_4` modules, plus iOS variants.
- [x] Step 3: Add enum variants via `define_enum!`.
- [x] Step 4: Unit tests.
- [x] **BDD Verification:** `cargo test -p hpx-emulation -- safari` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 2.4: Add Opera v118-v131 Profiles

> **Context:** hpx-emulation has Opera up to v117. wreq-util covers v116-131.
> **Verification:** `Emulation::Opera118` through `Opera131` variants exist.

- **Priority:** P1
- **Scope:** hpx-emulation — new device/opera modules
- **Requirement Coverage:** `R1` / REQ-01
- **Scenario Coverage:** All major browser versions available
- **Loop Type:** TDD-only
- **Behavioral Contract:** Preserve existing behavior.
- **Simplification Focus:** Opera uses Chromium-based TLS settings; share Chrome TLS presets where applicable.
- **Status:** 🟢 DONE
- [x] Step 1: Study existing Opera version module structure.
- [x] Step 2: Create `opera118` through `opera131` modules.
- [x] Step 3: Add enum variants.
- [x] Step 4: Unit tests.
- [x] **BDD Verification:** `cargo test -p hpx-emulation -- opera` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 2.5: Add Edge v143-v148 Profiles

> **Context:** hpx-emulation has Edge up to v142. wreq-util covers up to v148.
> **Verification:** `Emulation::Edge143` through `Edge148` variants exist.

- **Priority:** P1
- **Scope:** hpx-emulation — new device/edge modules
- **Requirement Coverage:** `R1` / REQ-01
- **Scenario Coverage:** All major browser versions available
- **Loop Type:** TDD-only
- **Behavioral Contract:** Preserve existing behavior.
- **Simplification Focus:** Edge uses Chromium-based TLS settings; share Chrome presets.
- **Status:** 🟢 DONE
- [x] Step 1: Study existing Edge version module structure.
- [x] Step 2: Create `edge143` through `edge148` modules.
- [x] Step 3: Add enum variants.
- [x] Step 4: Unit tests.
- [x] **BDD Verification:** `cargo test -p hpx-emulation -- edge` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 2.6: Browser Variant Count Assertion

> **Context:** Ensure total variant count meets the ≥120 threshold after all profiles are added.
> **Verification:** `assert!(Emulation::VARIANTS.len() >= 120)` passes.

- **Priority:** P1
- **Scope:** hpx-emulation — test
- **Requirement Coverage:** `R1` / REQ-01
- **Scenario Coverage:** All major browser versions available
- **Loop Type:** TDD-only
- **Behavioral Contract:** Assertion test only.
- **Simplification Focus:** One assert.
- **Status:** 🟢 DONE
- [x] Step 1: Add variant count assertion test to `crates/hpx-emulation/src/emulation.rs #[cfg(test)]`.
- [x] Step 2: Verify it passes after all profile additions.
- [x] **BDD Verification:** `cargo test -p hpx-emulation -- variant_count` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

---

## Phase 3: Emulation Enhancements

### Task 3.1: Platform × Profile Cross-Product in EmulationOption

> **Context:** After Task 1.1 adds `Platform`, wire it so that `EmulationOption::builder().emulation(Emulation::Chrome147).platform(Platform::Linux)` produces Linux-specific Chrome 147 fingerprints (UA string, sec-ch-ua-platform, and any TLS differences).
> **Verification:** Different platforms produce different headers for the same browser version.

- **Priority:** P0
- **Scope:** hpx-emulation — EmulationOption resolution
- **Requirement Coverage:** `R2` / REQ-02
- **Scenario Coverage:** Platform-specific emulation
- **Loop Type:** TDD-only
- **Behavioral Contract:** Default platform produces same output as current behavior.
- **Simplification Focus:** `Platform` maps to `EmulationOS` via `From` impl; the existing `into_emulation()` already handles OS-specific config. Just wire the new field through.
- **Status:** 🟢 DONE
- [x] Step 1: Update `EmulationOption::into_emulation()` to use `self.platform` when resolving OS-specific configs.
- [x] Step 2: Update `build_emulation()` functions to accept platform and adjust UA/sec-ch-ua headers accordingly.
- [x] Step 3: Unit test: Chrome147 + Windows vs Chrome147 + Linux → different User-Agent strings.
- [x] **BDD Verification:** `cargo test -p hpx-emulation -- platform` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 3.2: Platform-Aware UA and sec-ch-ua Headers

> **Context:** Different platforms need different User-Agent suffixes and `sec-ch-ua-platform` values. wreq-util generates these per-platform.
> **Verification:** `Platform::Android` produces UA with "Android" and `sec-ch-ua-platform: "Android"`.

- **Priority:** P0
- **Scope:** hpx-emulation — header generation
- **Requirement Coverage:** `R2` / REQ-02
- **Scenario Coverage:** Platform-specific emulation
- **Loop Type:** TDD-only
- **Behavioral Contract:** Existing profiles without explicit platform use default (unchanged).
- **Simplification Focus:** Add `Platform::user_agent_suffix()` and `Platform::sec_ch_ua_platform()` const methods. Apply in `build_emulation()`.
- **Status:** 🟢 DONE
- [x] Step 1: Implement `Platform::user_agent_suffix()` returning platform-specific UA fragments.
- [x] Step 2: Implement `Platform::sec_ch_ua_platform()` returning the `sec-ch-ua-platform` header value.
- [x] Step 3: Wire into header generation for each browser family.
- [x] Step 4: Unit tests per platform.
- [x] **BDD Verification:** `cargo test -p hpx-emulation -- sec_ch_ua` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 3.3: Weighted Random Emulation Selection

> **Context:** wreq-util provides `Emulation::weighted_random()` using market share data (Chrome 71.4%, Safari iOS 12.4%, Edge 5%, etc.). hpx-emulation only has uniform `random()`.
> **Verification:** Over 10000 samples, Chrome is selected ~71% of the time (within 5%).

- **Priority:** P1
- **Scope:** hpx-emulation — rand.rs
- **Requirement Coverage:** `R3` / REQ-03
- **Scenario Coverage:** Weighted random distribution
- **Loop Type:** TDD-only
- **Behavioral Contract:** Existing `Emulation::random()` unchanged. New `weighted_random()` is additive.
- **Simplification Focus:** Hardcoded const weight array. No config file, no trait, just a `(Emulation, f32)` array and cumulative walk. `ponytail:` update weights by editing const when stats change.
- **Status:** 🟢 DONE
- [x] Step 1: Define `const WEIGHTED_PROFILES: &[(fn() -> EmulationOption, f32)]` with market share weights.
- [x] Step 2: Implement `Emulation::weighted_random()` — generate `rand::random::<f32>()`, walk cumulative weights, return matching profile paired with weighted platform.
- [x] Step 3: Property test with proptest: 10000 samples, assert Chrome frequency in [0.66, 0.77].
- [x] **BDD Verification:** `cargo test -p hpx-emulation -- weighted_random` passes
- [x] **Advanced Test Verification:** `cargo test -p hpx-emulation -- weighted_distribution` — proptest distribution validation
- [x] **Runtime Verification:** N/A

### Task 3.4: TLS Certificate Compression Implementations

> **Context:** wreq-util implements `CertificateCompressor` for Brotli/Zlib/Zstd. Chrome uses Brotli only; Firefox/Safari use all three. These affect TLS 1.3 fingerprints.
> **Verification:** `BrotliCompressor` compresses and decompresses data correctly.

- **Priority:** P1
- **Scope:** hpx-emulation — new compress.rs module
- **Requirement Coverage:** `R5` / REQ-06
- **Scenario Coverage:** Browser-appropriate compression applied
- **Loop Type:** TDD-only
- **Behavioral Contract:** New module only, no existing behavior changes.
- **Simplification Focus:** Use `brotli`, `flate2`, `zstd` crates (already transitive deps). Three simple structs implementing the compressor trait. `ponytail:` quality/compression levels hardcoded to match wreq-util defaults.
- **Status:** 🟢 DONE
- [x] Step 1: Add `brotli`, `flate2`, `zstd` as direct deps to hpx-emulation (feature-gated behind `emulation-compression`).
- [x] Step 2: Create `crates/hpx-emulation/src/emulation/compress.rs` with `BrotliCompressor`, `ZlibCompressor`, `ZstdCompressor`.
- [x] Step 3: Implement the `boring::ssl::CertificateCompressor` trait (or equivalent) for each.
- [x] Step 4: Unit tests: round-trip compress/decompress for each compressor.
- [x] Step 5: Wire compressors into Chrome (Brotli only), Firefox (all three), Safari (all three) profiles.
- [x] **BDD Verification:** `cargo test -p hpx-emulation -- compress` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 3.5: Group-Based Connection Pool Key

> **Context:** wreq uses `Group` (BTreeMap-based multi-dimensional identity) for pool keys. Current hpx pool uses `(Scheme, Authority)`. Different emulations to same host should get separate pool entries to avoid fingerprint inconsistency.
> **Verification:** Two requests to same host with different emulations get different pool entries.

- **Priority:** P2
- **Scope:** hpx — pool.rs key computation
- **Requirement Coverage:** `R7` / REQ-07
- **Scenario Coverage:** Different emulations get separate pool entries
- **Loop Type:** TDD-only
- **Behavioral Contract:** Default (no per-request emulation override) uses same host-only key as before. Only when per-request emulation differs from client default does the Group hash change.
- **Simplification Focus:** `ponytail:` add optional emulation hash to existing key struct, don't restructure pool. Only hash emulation if per-request emulation is set.
- **Status:** 🟢 DONE
- [x] Step 1: Extend pool `Key` type to include optional `emulation_hash: Option<u64>`.
- [x] Step 2: When building the request, compute emulation hash from `RequestOptions` transport options if per-request emulation is set.
- [x] Step 3: Unit test: same host + same emulation = same key; same host + different emulation = different key.
- [x] **BDD Verification:** `cargo test -p hpx -- pool_group` passes
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

---

## Phase 4: Polish, QA & Docs

### Task 4.1: Clippy and Format Pass

> **Context:** All new code must pass `clippy::pedantic`, `clippy::nursery`, `clippy::cargo` per AGENTS.md.
> **Verification:** `cargo clippy -p hpx -p hpx-emulation -- -D warnings` exits 0.

- **Priority:** P0
- **Scope:** hpx + hpx-emulation
- **Requirement Coverage:** `R10`
- **Scenario Coverage:** N/A (quality gate)
- **Loop Type:** TDD-only
- **Behavioral Contract:** No behavior changes — lint fixes only.
- **Simplification Focus:** Fix clippy warnings, don't suppress them.
- **Status:** 🟢 DONE
- [x] Step 1: Run `cargo clippy -p hpx -p hpx-emulation -- -D warnings`.
- [x] Step 2: Fix all warnings.
- [x] Step 3: Run `cargo fmt -p hpx -p hpx-emulation`.
- [x] **BDD Verification:** Clippy exits 0
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 4.2: Workspace Regression Test

> **Context:** Ensure no regressions across the entire workspace after all changes.
> **Verification:** `cargo test --workspace` passes.

- **Priority:** P0
- **Scope:** Full workspace
- **Requirement Coverage:** `R8` / REQ-08
- **Scenario Coverage:** All existing tests
- **Loop Type:** TDD-only
- **Behavioral Contract:** No existing tests fail.
- **Simplification Focus:** N/A
- **Status:** 🟢 DONE
- [x] Step 1: Run `cargo test --workspace`.
- [x] Step 2: Fix any regressions.
- [x] Step 3: Re-run to confirm.
- [x] **BDD Verification:** `cargo test --workspace` — 0 failures
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 4.3: Integration Smoke Test

> **Context:** Verify end-to-end that emulation profiles actually produce correct HTTP requests to a real server.
> **Verification:** `hpx-cli` GET request with Chrome147 emulation returns 200 with expected headers.

- **Priority:** P1
- **Scope:** Integration
- **Requirement Coverage:** All
- **Scenario Coverage:** End-to-end emulation
- **Loop Type:** TDD-only
- **Behavioral Contract:** Real HTTP request succeeds.
- **Simplification Focus:** One smoke test, not a full test suite.
- **Status:** ⏭️ SKIPPED
- [ ] Step 1: Run `cargo run -p hpx-cli -- get https://httpbin.org/headers -e chrome147` and verify response.
- [ ] Step 2: Verify User-Agent and sec-ch-ua headers in response.
- [ ] **BDD Verification:** HTTP 200 with correct headers
- [ ] **Advanced Test Verification:** N/A
- [ ] **Runtime Verification:** Response headers match Chrome 147 profile

---

## Summary & Timeline

| Phase | Tasks | Target |
| :--- | :---: | :--- |
| **1. Foundation** | 3 | Platform, Error, forbid_recycle |
| **2. Browser Profiles** | 6 | ~40 new versions across 5 browsers |
| **3. Emulation Enhancements** | 5 | Weighted random, cert compression, pool Group |
| **4. Polish** | 3 | Clippy, regression, smoke test |
| **Total** | **17** | |

## Definition of Done

1. [ ] **Linted:** `cargo clippy --workspace -- -D warnings` passes.
2. [ ] **Tested:** `cargo test --workspace` — 0 failures.
3. [ ] **Formatted:** `cargo fmt --workspace` applied.
4. [ ] **Verified:** All task-specific verification criteria met.
5. [ ] **Behavior-Preserved:** No existing public API signatures changed.
6. [ ] **Feature-Gated:** New features behind appropriate feature flags.
7. [ ] **Documented:** New public methods have `/// doc comments` with examples.
