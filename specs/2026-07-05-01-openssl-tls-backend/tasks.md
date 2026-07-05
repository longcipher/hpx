# OpenSSL TLS Backend — Implementation Tasks

| Metadata | Details |
| :--- | :--- |
| **Design Doc** | [specs/2026-07-05-01-openssl-tls-backend/design.md](./design.md) |
| **Owner** | N/A |
| **Start Date** | 2026-07-05 |
| **Target Date** | N/A |
| **Status** | Planning |

## Summary & Phasing

Implementation strategy: Clone the BoringSSL TLS backend module (`tls/boring.rs` + `tls/boring/`) into a parallel OpenSSL module (`tls/openssl.rs` + `tls/openssl/`), perform mechanical crate-name substitutions, gate BoringSSL-specific features as no-ops, and wire the new module into the existing cfg-based backend selection mux. No BDD harness is needed — this is infrastructure-level code verified through unit tests, integration tests, and compilation checks.

- **Planner Contract Rule:** Contract-complete spec in design.md and tasks.md. No sidecar schemas.
- **State Contract Rule:** Status markers: `🔴 TODO` → `🟡 IN PROGRESS` → `🟢 DONE`, with `⏭️ SKIPPED` for explicit skips.
- **Property Testing Rule:** `proptest` for session cache shard routing stability.
- **Benchmark Rule:** N/A — no performance SLA.
- **Behavior Preservation Rule:** All existing BoringSSL and Rustls backend code must remain unchanged.
- **Simplification Rule:** Ponytail ladder — clone existing code (step 4: already-installed pattern), no new abstractions.
- **Phase 1: Foundation** — Feature flags and dependencies
- **Phase 2: Core Logic** — Clone and adapt `tls/boring/` → `tls/openssl/`
- **Phase 3: Integration** — Wire into shared TLS infrastructure and hpx-emulation
- **Phase 4: Polish** — Tests, linting, cross-compilation verification

---

## Phase 1: Foundation

### Task 1.1: Add OpenSSL Feature Flags and Workspace Dependencies

> **Context:** The hpx crate needs new Cargo features to enable the OpenSSL TLS backend. The workspace root must declare the new dependencies following AGENTS.md rules (numeric versions only, `cargo add --workspace`).
> **Verification:** `cargo check -p hpx --no-default-features --features openssl-tls,http1,http2,stream,tracing` compiles (will initially have no code, just validates feature resolution).

- **Priority:** P0
- **Scope:** Cargo.toml configuration
- **Requirement Coverage:** `R2`, `R3`, `R11`
- **Scenario Coverage:** N/A (configuration)
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** Preserve existing behavior — adding optional features must not affect default build
- **Simplification Focus:** N/A (configuration only)
- **Status:** 🟢 DONE
- [x] **Step 1:** Add `openssl = "0.10"` and `tokio-openssl = "0.6"` to workspace root `Cargo.toml` under `[workspace.dependencies]`:

  ```bash
  cargo add openssl --workspace
  cargo add tokio-openssl --workspace
  ```

- [x] **Step 2:** Add optional dependencies to `crates/hpx/Cargo.toml`:

  ```bash
  cargo add openssl -p hpx --workspace --optional
  cargo add tokio-openssl -p hpx --workspace --optional
  ```

- [x] **Step 3:** Add features to `crates/hpx/Cargo.toml` `[features]` section:

  ```toml
  openssl-tls = ["dep:openssl", "dep:tokio-openssl", "dep:brotli", "dep:flate2"]
  openssl-vendored = ["openssl-tls", "openssl/vendored"]
  ```

- [x] **Step 4:** Verify feature resolution:

  ```bash
  cargo check -p hpx --no-default-features --features openssl-tls,http1,http2,stream,tracing
  ```

  (Expected: compiles, no OpenSSL code exists yet — just validates feature graph)
- [x] **Verification:** Feature flags resolve without conflicts; default `boring` feature still works
- [x] **Advanced Test Verification:** N/A (configuration task)
- [x] **Runtime Verification:** N/A (configuration task)

---

## Phase 2: Core Logic

### Task 2.1: Create `tls/openssl.rs` Main Module (Clone from `tls/boring.rs`)

> **Context:** The core OpenSSL TLS backend module. Clone `crates/hpx/src/tls/boring.rs` as `crates/hpx/src/tls/openssl.rs` and perform systematic substitutions. Remove BoringSSL-specific APIs that have no OpenSSL equivalent.
> **Verification:** The file compiles under `cfg(feature = "openssl-tls")`.

- **Priority:** P0
- **Scope:** Core TLS backend module
- **Requirement Coverage:** `R1`, `R4`, `R6`, `R12`
- **Scenario Coverage:** `openssl-tls-backend-connect`
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New module — no existing behavior to preserve
- **Simplification Focus:** Minimum code via mechanical clone + substitution
- **Status:** 🟢 DONE
- [x] **Step 1:** Copy `crates/hpx/src/tls/boring.rs` to `crates/hpx/src/tls/openssl.rs`
- [x] **Step 2:** Perform substitutions:
  - `use boring::` → `use openssl::`
  - `use tokio_boring::` → `use tokio_openssl::`
  - `boring::error::ErrorStack` → `openssl::error::ErrorStack`
  - `boring::ex_data::Index` → `openssl::ex_data::Index`
  - `boring::ssl::Ssl` → `openssl::ssl::Ssl`
  - `boring::ssl::SslConnector` → `openssl::ssl::SslConnector`
  - `boring::ssl::SslMethod` → `openssl::ssl::SslMethod`
  - `boring::ssl::SslOptions` → `openssl::ssl::SslOptions`
  - `boring::ssl::SslRef` → `openssl::ssl::SslRef`
  - `boring::ssl::SslSessionCacheMode` → `openssl::ssl::SslSessionCacheMode`
  - `tokio_boring::SslStream` → `tokio_openssl::SslStream`
  - Module doc comment: "BoringSSL" → "OpenSSL"
- [x] **Step 3:** Remove BoringSSL-specific API calls that don't exist in `openssl` crate:
  - Remove `cfg.set_enable_ech_grease()` call (ECH is BoringSSL-only)
  - Remove `cfg.set_grease_enabled()` related macro invocations
  - Remove ALPS-related code (`add_application_settings`, `set_alps_use_new_codepoint`)
  - Remove `cfg.set_permute_extensions()` call
  - Remove `cfg.set_extension_permutation()` call
  - Remove `cfg.set_grease_enabled()` call
  - Remove certificate compression calls (`add_certificate_compression_algorithms`)
  - Remove `cert_compression` module reference
- [x] **Step 4:** Remove commented-out BoringSSL-specific code (e.g., `set_aes_hw_override`, `set_preserve_tls13_cipher_list`, `set_delegated_credentials`, `set_record_size_limit`, `set_key_shares_limit`) — these have no OpenSSL equivalent even as comments
- [x] **Step 5:** Add `#[cfg(test)]` module with mTLS test using `openssl::ssl::SslAcceptor` (adapt from boring.rs test)
- [x] **Step 6:** Gate the entire module with `#![cfg(feature = "openssl-tls")]` at the top (or rely on `tls.rs` parent cfg)
- [x] **Verification:** `cargo check -p hpx --no-default-features --features openssl-tls,http1,http2,stream,tracing` compiles
- [x] **Advanced Test Verification:** N/A (module creation, tests in Task 2.4)
- [x] **Runtime Verification:** N/A

### Task 2.2: Create `tls/openssl/cache.rs` — Session Cache (Clone from `tls/boring/cache.rs`)

> **Context:** The sharded TLS session cache for PSK-based session resumption. Clone from `tls/boring/cache.rs` with crate-name substitutions. The `SslSession` type in the `openssl` crate has the same interface.
> **Verification:** Session cache unit tests and proptest pass.

- **Priority:** P0
- **Scope:** Session resumption infrastructure
- **Requirement Coverage:** `R9`
- **Scenario Coverage:** `session-cache`
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New module — same behavior as BoringSSL session cache
- **Simplification Focus:** Direct clone — identical algorithm
- **Status:** 🟢 DONE
- [x] **Step 1:** Copy `crates/hpx/src/tls/boring/cache.rs` to `crates/hpx/src/tls/openssl/cache.rs`
- [x] **Step 2:** Replace `boring::ssl::SslSession` with `openssl::ssl::SslSession`
- [x] **Step 3:** Verify `HashSession` wrapper works — `SslSession` in `openssl` crate must implement the same traits. If `openssl::ssl::SslSession` doesn't implement `Hash`/`Eq` directly, adapt the wrapper.
- [x] **Step 4:** Copy all unit tests from boring cache (shard_count_is_never_zero, shard_routing_is_stable_for_a_key, tls13_session_is_single_use, etc.)
- [x] **Step 5:** Copy proptest cases for shard routing stability
- [x] **Verification:** `cargo nextest run -p hpx --no-default-features --features openssl-tls,http1,http2,stream,tracing -- cache` passes
- [x] **Advanced Test Verification:** `proptest` for shard routing: `cargo nextest run -p hpx --features openssl-tls -- cache::proptests`
- [x] **Runtime Verification:** N/A

### Task 2.3: Create `tls/openssl/ext.rs` — SslConnectorBuilder Extensions

> **Context:** Extension trait for `SslConnectorBuilder` providing cert store configuration and cert verification toggling. Clone from `tls/boring/ext.rs`, removing certificate compression support (not available in OpenSSL).
> **Verification:** Module compiles under `openssl-tls` feature.

- **Priority:** P0
- **Scope:** TLS connector builder extensions
- **Requirement Coverage:** `R4`, `R5`
- **Scenario Coverage:** `fingerprint-degradation`
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New module — cert store and verification work identically; compression is a no-op
- **Simplification Focus:** Remove cert_compression module reference
- **Status:** 🟢 DONE
- [x] **Step 1:** Copy `crates/hpx/src/tls/boring/ext.rs` to `crates/hpx/src/tls/openssl/ext.rs`
- [x] **Step 2:** Replace `boring::` imports with `openssl::` equivalents
- [x] **Step 3:** Remove `add_certificate_compression_algorithms()` method — OpenSSL doesn't support RFC 8879 cert compression. The method signature should be replaced with a no-op that accepts the parameter and returns `Ok(self)`:

  ```rust
  // Certificate compression is not supported by OpenSSL.
  // This method is a no-op for API compatibility.
  fn add_certificate_compression_algorithms(
      self,
      _algorithms: Option<&[CertificateCompressionAlgorithm]>,
  ) -> Result<Self, ErrorStack> {
      if _algorithms.is_some() {
          tracing::debug!("OpenSSL backend does not support certificate compression (RFC 8879)");
      }
      Ok(self)
  }
  ```

- [x] **Step 4:** Verify `configure_cert_store()` and `set_cert_verification()` work with `openssl::x509::X509StoreBuilder` and `openssl::ssl::SslVerifyMode`
- [x] **Verification:** Compiles as part of Task 2.1 verification
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 2.4: Create `tls/openssl/macros.rs` and `tls/openssl/service.rs`

> **Context:** Helper macros and tower::Service implementations. Direct clones with crate-name substitutions.
> **Verification:** Module compiles and tower::Service dispatch works.

- **Priority:** P0
- **Scope:** Utility macros and service layer
- **Requirement Coverage:** `R4`
- **Scenario Coverage:** `openssl-tls-backend-connect`
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New modules — same behavior as BoringSSL equivalents
- **Simplification Focus:** Direct clone
- **Status:** 🟢 DONE
- [x] **Step 1:** Copy `tls/boring/macros.rs` to `tls/openssl/macros.rs` (no changes needed — macros are crate-agnostic)
- [x] **Step 2:** Copy `tls/boring/service.rs` to `tls/openssl/service.rs`
- [x] **Step 3:** In `service.rs`, replace `tokio_boring::connect` with `tokio_openssl::connect` and `tokio_boring::SslStream` with `tokio_openssl::SslStream`
- [x] **Step 4:** Replace `boring::` imports with `openssl::` equivalents
- [x] **Step 5:** Remove any BoringSSL-specific service code (ALPS setup in `setup_ssl2` if present)
- [x] **Verification:** Part of Task 2.1 overall compilation check
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

---

## Phase 3: Integration & Features

### Task 3.1: Wire OpenSSL into `tls.rs`, `tls/conn.rs`, and Shared Modules

> **Context:** The TLS module root and connection mux need cfg branches for the OpenSSL backend. Shared types (`TlsVersion`, `AlpnProtocol`, `Certificate`, `Identity`, `CertStore`) need openssl-tls cfg branches.
> **Verification:** All shared TLS types compile correctly under `openssl-tls` feature.

- **Priority:** P0
- **Scope:** TLS module integration points
- **Requirement Coverage:** `R1`, `R6`, `R8`
- **Scenario Coverage:** `openssl-tls-backend-connect`, `openssl-tls-identity`
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** Preserve existing behavior for boring/rustls-tls; add openssl-tls path
- **Simplification Focus:** Follow existing cfg branch patterns exactly
- **Status:** 🟢 DONE
- [x] **Step 1:** Add `pub mod openssl;` to `crates/hpx/src/tls.rs` gated with:

  ```rust
  #[cfg(all(feature = "openssl-tls", not(feature = "boring")))]
  mod openssl;
  ```

- [x] **Step 2:** Update `tls.rs` top-level type definitions:
  - `TlsVersion`: Add `openssl::ssl::SslVersion` branch alongside `boring::ssl::SslVersion`
  - `AlpnProtocol::encode()` / `encode_sequence()`: Add `openssl-tls` branch (same length-prefixed encoding as BoringSSL)
  - Re-exports: `Certificate`, `Identity`, `CertStore` already flow through `tls/x509/` — verify cfg branches cover openssl-tls
- [x] **Step 3:** Update `crates/hpx/src/tls/conn.rs`:
  - Add cfg branch: `#[cfg(all(feature = "openssl-tls", not(feature = "boring")))]` that re-exports from `tls::openssl::*` and `tokio_openssl::SslStream`
  - Update `no_tls` fallback cfg to exclude `openssl-tls`: `#[cfg(not(any(feature = "boring", feature = "openssl-tls", feature = "rustls-tls")))]`
- [x] **Step 4:** Update `crates/hpx/src/tls/x509.rs`:
  - Add `#[cfg(all(feature = "openssl-tls", not(feature = "boring")))]` branch for `Certificate` wrapping `openssl::x509::X509`
  - Add `from_der()`, `from_pem()`, `stack_from_pem()` methods for OpenSSL
- [x] **Step 5:** Update `crates/hpx/src/tls/x509/identity.rs`:
  - Add `#[cfg(all(feature = "openssl-tls", not(feature = "boring")))]` branch for `Identity` with `PKey<Private>`, `X509`, `Vec<X509>` fields
  - Implement `from_pem()`, `from_pkcs8_pem()`, `from_pkcs12_der()` using `openssl::pkey::PKey`, `openssl::x509::X509`, `openssl::pkcs12::Pkcs12`
  - Implement `add_to_tls()` method setting cert, key, chain on `SslConnectorBuilder`
- [x] **Step 6:** Update `crates/hpx/src/tls/x509/store.rs`:
  - Add `openssl-tls` branch for `CertStore` wrapping `Arc<openssl::x509::X509Store>`
  - Add `CertStoreBuilder` methods using `openssl::x509::X509StoreBuilder`
- [x] **Step 7:** Update `crates/hpx/src/tls/x509/parser.rs`:
  - Add `openssl-tls` branch for `parse_certs()` and `parse_certs_with_stack()` using OpenSSL's `X509StoreBuilder`
- [x] **Step 8:** Update `crates/hpx/src/tls/options.rs`:
  - For fields currently gated `#[cfg(feature = "boring")]`:
    - `certificate_compression_algorithms`: Keep `boring`-only (OpenSSL doesn't support cert compression)
    - `extension_permutation`: Keep `boring`-only (not available in `openssl` crate)
    - Other fields that OpenSSL supports (curves_list, cipher_list, sigalgs_list) are already backend-agnostic
- [x] **Step 9:** Update `crates/hpx/src/client.rs` if `ConnectIdentity` export is `#[cfg(feature = "boring")]` — add `openssl-tls` to the cfg gate
- [x] **Step 10:** Update `crates/hpx/src/client/conn/tls_info.rs`:
  - Add `TlsInfoFactory` impl for `openssl::ssl::SslStream<TcpStream>` and `openssl::ssl::SslStream<MaybeHttpsStream<...>>`
- [x] **Verification:** `cargo check -p hpx --no-default-features --features openssl-tls,http1,http2,stream,tracing` — full crate compiles
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

### Task 3.2: hpx-emulation Compatibility with OpenSSL Backend

> **Context:** The `hpx-emulation` crate produces `TlsOptions` with BoringSSL-specific fields (GREASE, ECH, ALPS, extension permutation, certificate compression). Under `openssl-tls`, these fields must compile and be silently ignored.
> **Verification:** `cargo check -p hpx-emulation` compiles with hpx's `openssl-tls` feature.

- **Priority:** P1
- **Scope:** Emulation crate compatibility
- **Requirement Coverage:** `R7`, `R5`
- **Scenario Coverage:** `emulation-compat`
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** Preserve existing emulation behavior; under openssl-tls, BoringSSL-specific fields are no-ops
- **Simplification Focus:** Minimal changes — only cfg-gate if compilation fails
- **Status:** 🟢 DONE
- [x] **Step 1:** Run `cargo check -p hpx-emulation --no-default-features --features emulation` with hpx configured for `openssl-tls`. Check if compilation succeeds without changes.
- [x] **Step 2:** If compilation fails due to `TlsOptions` fields that are `#[cfg(feature = "boring")]`, add cfg branches in `hpx-emulation` code that reference those fields:

  ```rust
  #[cfg(feature = "boring")]
  {
      opts.certificate_compression_algorithms = ...;
      opts.extension_permutation = ...;
  }
  ```

- [x] **Step 3:** If the `Emulation` fingerprint types reference BoringSSL-specific types (e.g., `CertificateCompressionAlgorithm`, `ExtensionType` from `boring` crate), add cfg gates or provide stub types under `openssl-tls`.
- [x] **Step 4:** Verify the emulation pipeline produces a valid `TlsOptions` under `openssl-tls` (even if some fields are no-ops).
- [x] **Verification:** `cargo check -p hpx-emulation` and `cargo check --workspace` both compile
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

---

## Phase 4: Polish, QA & Docs

### Task 4.1: Tests — Unit, Integration, and Property Tests

> **Context:** Comprehensive test coverage for the OpenSSL backend. Includes session cache tests, mTLS integration test, identity parsing tests, and connector builder tests.
> **Verification:** `cargo nextest run -p hpx --no-default-features --features openssl-tls,http1,http2,stream,tracing` passes all tests.

- **Priority:** P0
- **Scope:** Test coverage
- **Requirement Coverage:** `R1`–`R9`
- **Scenario Coverage:** All scenarios
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** New tests — no existing test behavior to preserve
- **Simplification Focus:** Reuse existing test fixtures (mTLS certs in `tests/support/mtls/`)
- **Status:** 🟢 DONE
- [x] **Step 1:** Verify session cache unit tests pass (from Task 2.2)
- [x] **Step 2:** Verify mTLS integration test passes (from Task 2.1's `#[cfg(test)]` module):
  - Server uses `openssl::ssl::SslAcceptor` with `mozilla_intermediate` profile
  - Client uses `Identity::from_pem` with combined cert+key PEM
  - Verify response body equals "mtls-ok"
- [x] **Step 3:** Add `Identity::from_pkcs8_pem` and `Identity::from_pkcs12_der` tests using existing test fixtures (if available) or generate test PKCS12 archives
- [x] **Step 4:** Add a test that configures BoringSSL-specific options (GREASE, ECH, ALPS) and verifies no panic under OpenSSL:

  ```rust
  #[test]
  fn boring_specific_options_are_noop_under_openssl() {
      let opts = TlsOptions::builder()
          .enable_ech_grease(true)
          .grease_enabled(true)
          // These should be silently ignored under openssl-tls
          .build();
      let builder = TlsConnector::builder();
      let result = builder.build(&opts);
      assert!(result.is_ok());
  }
  ```

- [x] **Step 5:** Run proptest for session cache shard routing
- [x] **BDD Verification:** N/A (no BDD for this feature)
- [x] **Verification:** `cargo nextest run -p hpx --no-default-features --features openssl-tls,http1,http2,stream,tracing` — all tests pass
- [x] **Advanced Test Verification:** `cargo nextest run -p hpx --no-default-features --features openssl-tls,http1,http2,stream,tracing -- proptest` — property tests pass
- [x] **Runtime Verification:** N/A

### Task 4.2: Lint, Format, and Cross-Compilation Verification

> **Context:** Ensure the OpenSSL backend passes all workspace lint checks and compiles on key targets.
> **Verification:** `just format && just lint` passes; vendored compilation works.

- **Priority:** P1
- **Scope:** Quality assurance
- **Requirement Coverage:** `R10`, `R12`
- **Scenario Coverage:** N/A
- **Loop Type:** `TDD-only`
- **Behavioral Contract:** Preserve existing lint/format compliance
- **Simplification Focus:** N/A
- **Status:** 🟢 DONE
- [x] **Step 1:** Run `just format` — fix any formatting issues
- [x] **Step 2:** Run `cargo clippy -p hpx --no-default-features --features openssl-tls,http1,http2,stream,tracing -- -D warnings` — fix all warnings
- [x] **Step 3:** Run `just lint` — verify full workspace lint passes
- [x] **Step 4:** Verify vendored compilation:

  ```bash
  cargo check -p hpx --no-default-features --features openssl-vendored,http1,http2,stream,tracing
  ```

- [x] **Step 5:** Verify existing backends unchanged:

  ```bash
  cargo check -p hpx --features boring
  cargo check -p hpx --no-default-features --features rustls-tls,http1,http2,stream,tracing
  ```

- [x] **Step 6:** Run full workspace test suite:

  ```bash
  just test
  ```

- [x] **Verification:** All lint, format, and compilation checks pass
- [x] **Advanced Test Verification:** N/A
- [x] **Runtime Verification:** N/A

---

## Summary & Timeline

| Phase | Tasks | Target Date |
| :--- | :---: | :--- |
| **1. Foundation** | 1 | N/A |
| **2. Core Logic** | 4 | N/A |
| **3. Integration** | 2 | N/A |
| **4. Polish** | 2 | N/A |
| **Total** | **9** | |

## Definition of Done

> Every task must meet these criteria before being marked complete.

1. [ ] **Linted:** No clippy warnings with `-D warnings`.
2. [ ] **Tested:** Unit tests and integration tests covering the added logic pass.
3. [ ] **Formatted:** `just format` applied.
4. [ ] **Verified:** The task's specific Verification criterion is met.
5. [ ] **Advanced-Tested (when applicable):** proptest for session cache captured, or `N/A` explicitly justified.
6. [ ] **Runtime-Evidenced (when applicable):** `N/A` — infrastructure code, no runtime server.
7. [ ] **Behavior-Preserved or Documented:** Existing BoringSSL and Rustls backends confirmed unchanged.
8. [ ] **Simplified Responsibly:** Mechanical clone approach — no clever abstractions introduced.
9. [ ] **Performance-Sound (when applicable):** Session cache uses identical sharded algorithm — no performance regression.
