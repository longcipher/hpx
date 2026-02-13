# Integrate SSE Transport — Implementation Tasks

| Metadata | Details |
| :--- | :--- |
| **Design Doc** | specs/integrate-sse-transport/design.md |
| **Owner** | — |
| **Start Date** | 2026-02-13 |
| **Target Date** | — |
| **Status** | Planning |

## Summary & Phasing

Integrate `sseer` SSE parsing into `hpx-transport` as a new `sse` module, mirroring the WebSocket module's Connection/Handle/Stream architecture. Uses `hpx::Client` as the HTTP backend instead of `reqwest`.

- **Phase 1: Foundation & Scaffolding** — Workspace deps, feature flag, module structure, config, types, error variants
- **Phase 2: Core Logic** — Connection background task, event parsing pipeline, auto-reconnect
- **Phase 3: Integration & Features** — Protocol handler trait, generic handler, auth integration, Handle/Stream API
- **Phase 4: Polish, QA & Docs** — Tests, examples, documentation, CI

---

## Phase 1: Foundation & Scaffolding

### Task 1.1: Add `sseer` Workspace Dependency and Feature Flag

> **Context:** `sseer` already exists in the workspace but is not declared as a workspace dependency. We need to add it to root `Cargo.toml` `[workspace.dependencies]` and add an `sse` feature to `hpx-transport` that optionally depends on it. Reference: existing `ws-yawc`/`ws-fastwebsockets` feature patterns in `crates/hpx-transport/Cargo.toml`.
> **Verification:** `cargo check -p hpx-transport --features sse` compiles without errors.

- **Priority:** P0
- **Scope:** Cargo configuration
- **Status:** � DONE

- [x] **Step 1:** Add `sseer = { path = "sseer", version = "0.1.7" }` to root `Cargo.toml` `[workspace.dependencies]`.
- [x] **Step 2:** Add `sseer` as a workspace member if not already (check `[workspace] members`).
- [x] **Step 3:** In `crates/hpx-transport/Cargo.toml`, add `sseer = { workspace = true, optional = true }` under `[dependencies]`.
- [x] **Step 4:** Add feature `sse = ["dep:sseer"]` to `[features]` section (do NOT add to `default`).
- [x] **Step 5:** Add `bytes-utils` and `futures-core` as workspace dependencies (needed for SSE types like `Str`), and add them to hpx-transport deps.
- [x] **Verification:** `cargo check -p hpx-transport --features sse` succeeds.

---

### Task 1.2: Create SSE Module Scaffolding

> **Context:** Create the directory structure and empty module files for the SSE transport. Reference the `websocket/` module structure as a template.
> **Verification:** `cargo check -p hpx-transport --features sse` compiles with the new (empty) modules.

- **Priority:** P0
- **Scope:** Module structure
- **Status:** � DONE

- [x] **Step 1:** Create `crates/hpx-transport/src/sse/mod.rs` with submodule declarations (`config`, `connection`, `protocol`, `types`, `handlers`).
- [x] **Step 2:** Create `crates/hpx-transport/src/sse/config.rs` (empty struct placeholder).
- [x] **Step 3:** Create `crates/hpx-transport/src/sse/connection.rs` (empty placeholder).
- [x] **Step 4:** Create `crates/hpx-transport/src/sse/protocol.rs` (empty placeholder).
- [x] **Step 5:** Create `crates/hpx-transport/src/sse/types.rs` (empty placeholder).
- [x] **Step 6:** Create `crates/hpx-transport/src/sse/handlers/mod.rs` and `handlers/generic.rs` (empty placeholders).
- [x] **Step 7:** In `crates/hpx-transport/src/lib.rs`, add `#[cfg(feature = "sse")] pub mod sse;`.
- [x] **Verification:** `cargo check -p hpx-transport --features sse` compiles cleanly.

---

### Task 1.3: Define SSE Types and Config

> **Context:** Define the core types (`SseMessageKind`, `SseEvent`) and configuration struct (`SseConfig`) that will be used throughout the SSE module. Reference `websocket/types.rs` and `websocket/config.rs` for patterns.
> **Verification:** Types compile, `SseConfig` builder methods work in a unit test.

- **Priority:** P0
- **Scope:** Type definitions
- **Status:** � DONE

- [x] **Step 1:** Implement `SseMessageKind` enum in `sse/types.rs` with variants: `Data`, `System`, `Retry`, `Unknown`.
- [x] **Step 2:** Implement `SseEvent` struct in `sse/types.rs` wrapping `sseer::event::Event` with a `kind: SseMessageKind` field.
- [x] **Step 3:** Implement `SseConfig` struct in `sse/config.rs` with all fields from the design doc (url, method, headers, body, timeouts, reconnection settings, channel capacities).
- [x] **Step 4:** Implement `SseConfig::new()` constructor and builder methods (following `WsConfig` patterns).
- [x] **Step 5:** Add `#[cfg(test)] mod tests` with unit tests for config builder.
- [x] **Verification:** `cargo nextest run -p hpx-transport --features sse` passes config tests.

---

### Task 1.4: Extend Error Types for SSE

> **Context:** Add SSE-specific error variants to `TransportError` behind `#[cfg(feature = "sse")]` or unconditionally (since they're just enum variants). Reference `error.rs` existing patterns.
> **Verification:** SSE error variants compile and display correctly.

- **Priority:** P0
- **Scope:** Error handling
- **Status:** � DONE

- [x] **Step 1:** Add `SseInvalidStatus { status: http::StatusCode }` variant to `TransportError`.
- [x] **Step 2:** Add `SseInvalidContentType { content_type: String }` variant.
- [x] **Step 3:** Add `SseParse { message: String }` variant.
- [x] **Step 4:** Add `SseStreamEnded` variant.
- [x] **Step 5:** Add constructor helpers (`TransportError::sse_invalid_status()`, etc.).
- [x] **Step 6:** Add unit tests for new error variants.
- [x] **Verification:** `cargo nextest run -p hpx-transport --features sse` passes error tests.

---

## Phase 2: Core Logic

### Task 2.1: Implement SSE Connection Background Task

> **Context:** This is the core of the SSE transport. Implement the background task that manages the HTTP connection, pipes the response body through `sseer::EventStream`, and sends parsed events to the `SseStream` consumer via an mpsc channel. Reference `sseer::reqwest::EventSource` for the state machine pattern, and `websocket/connection.rs` for the background task + channel architecture.
> **Verification:** Unit test with a mock bytes stream → EventStream → channel pipeline works correctly.

- **Priority:** P0
- **Scope:** Connection driver
- **Status:** 🔴 TODO

- [ ] **Step 1:** Define `SseConnectionState` enum (Disconnected, Connecting, Connected, Reconnecting, Closed).
- [ ] **Step 2:** Define `SseCommand` enum (Close, Reconnect).
- [ ] **Step 3:** Implement internal `establish_connection()` function:
  - Build `hpx::Client` request with `Accept: text/event-stream`
  - Include `Last-Event-ID` header if set
  - Validate response status (2xx) and Content-Type
  - Return `sseer::EventStream` wrapping the response body stream
- [ ] **Step 4:** Implement the background task loop (`tokio::spawn`):
  - `tokio::select!` over event_stream.next() and command channel
  - On event: classify via protocol handler, send to event channel
  - On close command: break loop
  - On stream end/error: trigger reconnection logic
- [ ] **Step 5:** Implement reconnect logic with exponential backoff:
  - Use `sseer::retry::ExponentialBackoff` (or custom backoff from config)
  - Track `last_event_id` from events, set on reconnect
  - Respect `max_reconnect_attempts` from config
- [ ] **Verification:** Integration test connecting to a mock `hyper` SSE server and receiving events.

---

### Task 2.2: Implement SseConnection / SseHandle / SseStream API

> **Context:** Expose the public API for creating and interacting with SSE connections. `SseConnection` is the entry point, `split()` returns `SseHandle` (clone-able control) and `SseStream` (event consumer). Follows `websocket::Connection` / `ConnectionHandle` / `ConnectionStream` pattern.
> **Verification:** Can create an `SseConnection`, split it, receive events on `SseStream`, and close via `SseHandle`.

- **Priority:** P0
- **Scope:** Public API
- **Status:** 🔴 TODO

- [ ] **Step 1:** Implement `SseConnection` struct holding config, handler, client, and internal state.
- [ ] **Step 2:** Implement `SseConnection::connect()` — establishes initial connection, spawns background task.
- [ ] **Step 3:** Implement `SseConnection::split()` → `(SseHandle, SseStream)`.
- [ ] **Step 4:** Implement `SseHandle` with `close()`, `reconnect()`, and `state()` methods.
- [ ] **Step 5:** Implement `SseStream` as `impl Stream<Item = SseEvent>` using `mpsc::Receiver`.
- [ ] **Step 6:** Wire up re-exports in `sse/mod.rs`.
- [ ] **Verification:** Integration test: connect → split → receive events → close.

---

## Phase 3: Integration & Features

### Task 3.1: Implement SseProtocolHandler Trait and GenericSseHandler

> **Context:** The protocol handler abstraction allows exchange-specific event classification. `GenericSseHandler` is the default pass-through handler. Reference `websocket/protocol.rs` and `websocket/handlers/`.
> **Verification:** `GenericSseHandler` correctly classifies all events as `Data` kind.

- **Priority:** P1
- **Scope:** Protocol abstraction
- **Status:** 🔴 TODO

- [ ] **Step 1:** Define `SseProtocolHandler` trait in `sse/protocol.rs` with methods: `classify_event()`, `on_connect()`, `on_disconnect()`, `should_retry()`.
- [ ] **Step 2:** Implement `GenericSseHandler` in `sse/handlers/generic.rs` that classifies all non-empty events as `Data` and empty events as `System`.
- [ ] **Step 3:** Re-export in `sse/handlers/mod.rs`.
- [ ] **Step 4:** Add unit tests for `GenericSseHandler` classification.
- [ ] **Verification:** `cargo nextest run -p hpx-transport --features sse` passes handler tests.

---

### Task 3.2: Integrate Authentication with SSE Connection

> **Context:** SSE requests need authentication for exchange APIs. Reuse `hpx-transport::auth::Authentication` trait to sign the initial HTTP request (and reconnect requests). The auth trait's `sign()` method modifies headers and optionally adds query params.
> **Verification:** Mock test verifying auth headers are present on SSE requests.

- **Priority:** P1
- **Scope:** Auth integration
- **Status:** 🔴 TODO

- [ ] **Step 1:** Add an optional `Box<dyn Authentication>` field to `SseConnection` internals.
- [ ] **Step 2:** In `establish_connection()`, call `auth.sign()` to modify request headers before sending.
- [ ] **Step 3:** Add `SseConnection::connect_with_auth()` constructor that accepts an `Authentication` impl.
- [ ] **Step 4:** Ensure auth is re-applied on reconnection.
- [ ] **Verification:** Integration test with mock server verifying auth headers are present.

---

### Task 3.3: Add Tracing Instrumentation

> **Context:** The WebSocket module uses `tracing` for observability. Add equivalent spans and events for SSE connection lifecycle. Reference `connection.rs` tracing patterns.
> **Verification:** Running with `RUST_LOG=debug` shows SSE connection lifecycle events.

- **Priority:** P1
- **Scope:** Observability
- **Status:** 🔴 TODO

- [ ] **Step 1:** Add `tracing::info!` for connection established and closed events.
- [ ] **Step 2:** Add `tracing::warn!` for reconnection attempts with attempt number and delay.
- [ ] **Step 3:** Add `tracing::debug!` for individual SSE events received (event type, id).
- [ ] **Step 4:** Add `tracing::error!` for connection errors (invalid status, parse errors).
- [ ] **Step 5:** Add `#[instrument]` on key public methods.
- [ ] **Verification:** Manual verification with tracing-subscriber; log output shows SSE lifecycle.

---

## Phase 4: Polish, QA & Docs

### Task 4.1: Write Integration Tests with Mock SSE Server

> **Context:** Create a mock SSE server using `hyper` (already a dev-dependency) to test the full connection lifecycle. Reference existing WebSocket tests in `tests/` for patterns.
> **Verification:** All integration tests pass.

- **Priority:** P0
- **Scope:** Testing
- **Status:** 🔴 TODO

- [ ] **Step 1:** Create `tests/sse_connection.rs` integration test file.
- [ ] **Step 2:** Implement mock SSE server helper that serves `text/event-stream` responses with configurable events.
- [ ] **Step 3:** Test: basic connection and event reception (TC-01).
- [ ] **Step 4:** Test: non-2xx status code handling (TC-02).
- [ ] **Step 5:** Test: invalid Content-Type handling (TC-03).
- [ ] **Step 6:** Test: auto-reconnection with Last-Event-ID (TC-04).
- [ ] **Step 7:** Test: graceful close via SseHandle (TC-05).
- [ ] **Step 8:** Test: max reconnect attempts exceeded (TC-08).
- [ ] **Verification:** `cargo nextest run -p hpx-transport --features sse` — all tests pass.

---

### Task 4.2: Write Unit Tests

> **Context:** Add unit tests for types, config, error variants, and protocol handlers. Tests should be in-module (`#[cfg(test)] mod tests`).
> **Verification:** All unit tests pass with full feature set.

- **Priority:** P1
- **Scope:** Unit testing
- **Status:** 🔴 TODO

- [ ] **Step 1:** Unit tests for `SseConfig` builder methods and defaults.
- [ ] **Step 2:** Unit tests for `SseMessageKind` and `SseEvent` construction.
- [ ] **Step 3:** Unit tests for SSE error variants (display, construction).
- [ ] **Step 4:** Unit tests for `GenericSseHandler` event classification.
- [ ] **Verification:** `cargo nextest run -p hpx-transport --features sse` — all unit tests pass.

---

### Task 4.3: Add Example

> **Context:** Add an example demonstrating SSE transport usage, similar to `examples/ws_subscription.rs`. Reference existing examples for patterns.
> **Verification:** Example compiles with `cargo build --example sse_stream -p hpx-transport --features sse`.

- **Priority:** P2
- **Scope:** Documentation / Examples
- **Status:** 🔴 TODO

- [ ] **Step 1:** Create `crates/hpx-transport/examples/sse_stream.rs` demonstrating basic SSE connection and event consumption.
- [ ] **Step 2:** Add `[[example]]` entry to `crates/hpx-transport/Cargo.toml` with `required-features = ["sse"]`.
- [ ] **Verification:** `cargo build --example sse_stream -p hpx-transport --features sse` compiles.

---

### Task 4.4: Add Module Documentation and Re-exports

> **Context:** Add comprehensive module-level docs for the SSE module and ensure all public types are properly re-exported from `hpx_transport::sse`. Update `lib.rs` doc comments to mention SSE. Reference `websocket/mod.rs` doc style.
> **Verification:** `RUSTDOCFLAGS="-D warnings" cargo doc -p hpx-transport --features sse --no-deps` succeeds.

- **Priority:** P2
- **Scope:** Documentation
- **Status:** 🔴 TODO

- [ ] **Step 1:** Write module-level docs in `sse/mod.rs` with architecture diagram, quick start example, and module index.
- [ ] **Step 2:** Ensure all public types have doc comments.
- [ ] **Step 3:** Add SSE to `lib.rs` top-level doc comments and re-export list.
- [ ] **Step 4:** Run `RUSTDOCFLAGS="-D warnings" cargo doc -p hpx-transport --features sse --no-deps` and fix warnings.
- [ ] **Verification:** Doc build succeeds with zero warnings.

---

### Task 4.5: Lint, Format, and CI Verification

> **Context:** Ensure all code passes the project's strict lint and formatting requirements. Reference `AGENTS.md` for exact commands.
> **Verification:** Full CI chain passes.

- **Priority:** P0
- **Scope:** Quality assurance
- **Status:** 🔴 TODO

- [ ] **Step 1:** Run `cargo +nightly fmt --all`.
- [ ] **Step 2:** Run `cargo +nightly clippy --all -- -D warnings` and fix any warnings.
- [ ] **Step 3:** Run `cargo nextest run --workspace --all-features` and verify all tests pass.
- [ ] **Step 4:** Run `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --all-features`.
- [ ] **Verification:** All four commands succeed with zero errors/warnings.

---

## Summary & Timeline

| Phase | Tasks | Target Date |
| :--- | :---: | :--- |
| **1. Foundation** | 4 | 02-15 |
| **2. Core Logic** | 2 | 02-19 |
| **3. Integration** | 3 | 02-22 |
| **4. Polish** | 5 | 02-26 |
| **Total** | **14** | |

## Definition of Done

1. [ ] **Linted:** `cargo +nightly clippy --all -- -D warnings` passes with `--features sse`.
2. [ ] **Tested:** Unit tests + integration tests covering connection lifecycle, reconnection, auth, error handling.
3. [ ] **Formatted:** `cargo +nightly fmt --all` applied.
4. [ ] **Documented:** All public types documented, `cargo doc` passes.
5. [ ] **Verified:** Each task's specific Verification criterion met.
6. [ ] **Feature-gated:** SSE code is behind `sse` feature flag; default build unaffected.
