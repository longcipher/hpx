# HPX Actor Refactor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Deliver the single-task WebSocket connection architecture, zero-spawn requests, RAII subscriptions, and targeted lock optimizations with correctness fixes and full test coverage.

**Architecture:** Preserve `WsClient` as a compatibility wrapper over a new `Connection` API, with a single-owner connection task and dual channels for control/data. Use zero-spawn pending requests, RAII subscription guards, and targeted lock optimizations with `parking_lot` and atomics.

**Tech Stack:** Rust, Tokio, hyper, axum (tests), futures, parking_lot.

---

> **Source of truth for requirements:** `docs/hpx-actor-tasks.md` (all task details and acceptance criteria). This plan focuses on *execution sequencing* and TDD steps; refer to the task file for exact requirements.

## Global Execution Rules (Apply to Every Task)

- **TDD**: write failing test → run to confirm failure → implement minimal fix → run test → refactor → re-run.
- **Per-task verification**: `cargo build` and `just lint` must pass before marking task complete.
- **Commits**: one conventional commit per task; concise, impact-oriented.
- **Progress tracking**: update checkboxes in `docs/hpx-actor-tasks.md` after each task.
- **Do not stage `/`.**

---

## Phase 1: Correctness Fixes

### Task 1.1: Fix double-connection bug

**Files:**

- Modify: `crates/hpx-transport/src/websocket/actor.rs`
- Add test: `crates/hpx-transport/tests/websocket_actor_double_connect_test.rs`

**Step 1: Write failing test**

```rust

#[tokio::test]
async fn double_connect_regression() {
    // Arrange: server counts connections; client connects and authenticates
    // Assert: second connection must NOT occur.
}

```

**Step 2: Run test (expect FAIL)**
Run: `cargo test -p hpx-transport --test websocket_actor_double_connect_test`
Expected: FAIL with unexpected second connection.

**Step 3: Minimal implementation**

- Refactor ready-loop to reuse authenticated ws read/write halves.
- Ensure authentication and resubscribe happen on the same socket.

**Step 4: Run test (expect PASS)**
Run: `cargo test -p hpx-transport --test websocket_actor_double_connect_test`

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `fix(transport): prevent websocket double connect`

### Task 1.2: Handle-drop shutdown

**Files:**

- Modify: `crates/hpx-transport/src/websocket/actor.rs`
- Add test: `crates/hpx-transport/tests/websocket_actor_shutdown_on_drop_test.rs`

**Step 1: Write failing test**

```rust

#[tokio::test]
async fn actor_exits_when_handles_dropped() {
    // Arrange: connect, drop all WsClient handles
    // Assert: connection task terminates within 1s
}

```

**Step 2: Run test (expect FAIL)**
Run: `cargo test -p hpx-transport --test websocket_actor_shutdown_on_drop_test`

**Step 3: Minimal implementation**

- Detect `cmd_rx.recv() == None` and initiate graceful shutdown.
- Close websocket, resolve pending with error, exit task.

**Step 4: Run test (expect PASS)**
Run: `cargo test -p hpx-transport --test websocket_actor_shutdown_on_drop_test`

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `fix(transport): shutdown ws actor on last handle drop`

### Task 1.3: Regression tests for Phase 1

**Files:**

- Add test: `crates/hpx-transport/tests/websocket_actor_double_connect_test.rs`
- Add test: `crates/hpx-transport/tests/websocket_actor_shutdown_on_drop_test.rs`

**Step 1: Ensure tests exist and fail on old code**

- Confirm both tests fail without the fixes.

**Step 2: Run tests (expect PASS now)**
Run: `cargo test -p hpx-transport --tests websocket_actor_double_connect_test websocket_actor_shutdown_on_drop_test`

**Step 3: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `test(transport): add websocket actor regression coverage`

---

## Phase 2: New Connection API

> Each task follows the same TDD pattern: add tests for the specific behavior, verify failure, implement, re-run tests, then `cargo build` + `just lint`.

### Task 2.1: Core types

**Files:**

- Create: `crates/hpx-transport/src/websocket/connection.rs`

**Step 1: Add compile-only test (type checks)**

```rust

#[test]
fn connection_types_compile() {
    // Assert types are constructible and derive Debug/Clone where required.
}

```

**Step 2: Run test (expect FAIL)**
Run: `cargo test -p hpx-transport --lib connection_types_compile`

**Step 3: Implement types**

- Add enums/structs exactly as in `docs/hpx-actor-tasks.md`.

**Step 4: Run test (expect PASS)**

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `feat(transport): add websocket connection core types`

### Task 2.2: ConnectionHandle

**Files:**

- Modify: `crates/hpx-transport/src/websocket/connection.rs`

**Step 1: Failing test for request/subscribe APIs**

```rust

#[tokio::test]
async fn connection_handle_request_times_out() {
    // Use PendingRequestStore to confirm timeout removes pending entry
}

```

**Step 2: Run test (expect FAIL)**
Run: `cargo test -p hpx-transport --test connection_handle_request`

**Step 3: Implement ConnectionHandle methods**

- Use pending store + timeout path per tasks.

**Step 4: Run test (expect PASS)**

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `feat(transport): implement ConnectionHandle API`

### Task 2.3: ConnectionStream

**Files:**

- Modify: `crates/hpx-transport/src/websocket/connection.rs`

**Step 1: Failing test for Stream impl**

```rust

#[tokio::test]
async fn connection_stream_yields_events() {
    // Inject event_rx, ensure next() yields in order
}

```

**Step 2: Run test (expect FAIL)**
Run: `cargo test -p hpx-transport --test connection_stream`

**Step 3: Implement Stream + next()**

**Step 4: Run test (expect PASS)**

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `feat(transport): add ConnectionStream`

### Task 2.4: Connection task loop

**Files:**

- Modify: `crates/hpx-transport/src/websocket/connection.rs`

**Step 1: Failing integration test for select loop**

```rust

#[tokio::test]
async fn connection_task_routes_control_and_data() {
    // Validate control/data paths and event emission behavior
}

```

**Step 2: Run test (expect FAIL)**
Run: `cargo test -p hpx-transport --test connection_task_loop`

**Step 3: Implement loop + select branches**

**Step 4: Run test (expect PASS)**

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `feat(transport): implement websocket connection task`

### Task 2.5: Connect + auth flow

**Files:**

- Modify: `crates/hpx-transport/src/websocket/connection.rs`

**Step 1: Failing integration test for auth/resubscribe**

```rust

#[tokio::test]
async fn connection_auth_and_resubscribe() {
    // Server validates auth + resubscribe on same connection
}

```

**Step 2: Run test (expect FAIL)**
Run: `cargo test -p hpx-transport --test connection_auth`

**Step 3: Implement establish_connection + auth**

**Step 4: Run test (expect PASS)**

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `feat(transport): add connection auth flow`

### Task 2.6: Reconnection backoff

**Files:**

- Modify: `crates/hpx-transport/src/websocket/connection.rs`

**Step 1: Failing test for backoff progression**

```rust

#[tokio::test]
async fn reconnect_uses_backoff() {
    // Validate delay increases and caps
}

```

**Step 2: Run test (expect FAIL)**
Run: `cargo test -p hpx-transport --test connection_reconnect`

**Step 3: Implement backoff logic**

**Step 4: Run test (expect PASS)**

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `feat(transport): add reconnect backoff`

### Task 2.7: Connection::connect

**Files:**

- Modify: `crates/hpx-transport/src/websocket/connection.rs`
- Modify: `crates/hpx-transport/src/websocket/config.rs` (for event channel capacity)

**Step 1: Failing test for connect returns handle+stream**

```rust

#[tokio::test]
async fn connection_connect_returns_handle_stream() {
    // Ensure task spawns and stream receives Connected event
}

```

**Step 2: Run test (expect FAIL)**
Run: `cargo test -p hpx-transport --test connection_connect`

**Step 3: Implement connect**

**Step 4: Run test (expect PASS)**

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `feat(transport): add Connection::connect entrypoint`

### Task 2.8: WsClient wrapper

**Files:**

- Modify: `crates/hpx-transport/src/websocket/ws_client.rs`

**Step 1: Failing test for wrapper behavior**

```rust

#[tokio::test]
async fn wsclient_delegates_to_connection() {
    // Ensure request/subscribe/send/close delegate to handle
}

```

**Step 2: Run test (expect FAIL)**
Run: `cargo test -p hpx-transport --test wsclient_wrapper`

**Step 3: Implement wrapper**

**Step 4: Run test (expect PASS)**

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `refactor(transport): wrap WsClient around Connection`

### Task 2.9: Module exports

**Files:**

- Modify: `crates/hpx-transport/src/websocket/mod.rs`

**Step 1: Failing test for exports**

```rust

#[test]
fn websocket_exports_compile() {
    // Ensure Connection* types are publicly exported
}

```

**Step 2: Run test (expect FAIL)**
Run: `cargo test -p hpx-transport --lib websocket_exports_compile`

**Step 3: Implement exports**

**Step 4: Run test (expect PASS)**

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `chore(transport): export Connection API`

### Task 2.10: Remove old actor

**Files:**

- Modify/Delete: `crates/hpx-transport/src/websocket/actor.rs`

**Step 1: Failing compile check**

- Remove actor usage in code, ensure no references remain.

**Step 2: Run `cargo check` (expect errors)**
Run: `cargo check -p hpx-transport`

**Step 3: Remove actor code and fix references**

**Step 4: Run `cargo check` (expect PASS)**

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `refactor(transport): remove legacy websocket actor`

### Task 2.11: Phase validation

**Step 1: Run required commands**

- `cargo fmt --all -- --check`
- `cargo clippy -p hpx-transport -- -D warnings`
- `cargo test -p hpx-transport --lib --tests`
- `cargo doc -p hpx-transport --no-deps`

**Step 2: Commit**
Commit: `chore(transport): validate connection refactor phase`

---

## Phase 3: Zero-Spawn & RAII Subscriptions

### Task 3.1: Zero-spawn requests

**Files:**

- Modify: `crates/hpx-transport/src/websocket/connection.rs`
- Modify: `crates/hpx-transport/src/websocket/pending.rs`

**Step 1: Failing test for request resolution without spawn**

```rust

#[tokio::test]
async fn request_resolves_without_spawn() {}

```

**Step 2: Run test (expect FAIL)**
Run: `cargo test -p hpx-transport --test request_zero_spawn`

**Step 3: Implement pending store + request path**

**Step 4: Run test (expect PASS)**

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `feat(transport): zero-spawn request path`

### Task 3.2: SubscriptionGuard

**Files:**

- Modify: `crates/hpx-transport/src/websocket/connection.rs`
- Modify: `crates/hpx-transport/src/websocket/subscription.rs`

**Step 1: Failing test for unsubscribe-on-drop**

```rust

#[tokio::test]
async fn subscription_guard_unsubscribes_on_drop() {}

```

**Step 2: Run test (expect FAIL)**
Run: `cargo test -p hpx-transport --test subscription_guard`

**Step 3: Implement guard + decrement_ref**

**Step 4: Run test (expect PASS)**

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `feat(transport): add SubscriptionGuard`

### Task 3.3: Subscribe returns guard

**Files:**

- Modify: `crates/hpx-transport/src/websocket/connection.rs`
- Modify: `crates/hpx-transport/src/websocket/ws_client.rs`

**Step 1: Failing test for API change**

```rust

#[tokio::test]
async fn subscribe_returns_guard() {}

```

**Step 2: Run test (expect FAIL)**
Run: `cargo test -p hpx-transport --test subscribe_guard`

**Step 3: Implement return type + wiring**

**Step 4: Run test (expect PASS)**

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `refactor(transport): return SubscriptionGuard from subscribe`

### Task 3.4: Subscription lifecycle tests

**Files:**

- Add: `crates/hpx-transport/tests/subscription_lifecycle.rs`

**Step 1: Write failing tests**

```rust

#[tokio::test]
async fn subscription_drop_unsubscribes() {}

```

**Step 2: Run tests (expect FAIL)**
Run: `cargo test -p hpx-transport --test subscription_lifecycle`

**Step 3: Implement missing behavior (if any)**

**Step 4: Run tests (expect PASS)**

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `test(transport): add subscription lifecycle coverage`

### Task 3.5: Phase validation

- `cargo fmt --all -- --check`
- `cargo clippy -p hpx-transport -- -D warnings`
- `cargo test -p hpx-transport --lib --tests`

Commit: `chore(transport): validate zero-spawn and raii phase`

---

## Phase 4: Lock Optimizations

### Task 4.1: Add parking_lot dependency

**Files:**

- Modify: `Cargo.toml`

**Step 1: Add dependency**
Run: `cargo add parking_lot --workspace`

**Step 2: Build**
Run: `cargo build`

**Step 3: Verify + commit**
Run: `just lint`
Commit: `chore: add parking_lot workspace dependency`

### Task 4.2: Sharded connection pool

**Files:**

- Modify: `crates/hpx/src/client/http/client/pool.rs`

**Step 1: Failing test/bench**

- Add unit test for shard-local locking or extend existing pool tests.

**Step 2: Run test (expect FAIL)**
Run: `cargo test -p hpx --lib pool`

**Step 3: Implement sharded pool**

**Step 4: Run tests (expect PASS)**

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `perf(hpx): shard http connection pool locks`

### Task 4.3: Atomic H2 ping fast path

**Files:**

- Modify: `crates/hpx/src/client/core/proto/h2/ping.rs`

**Step 1: Failing test (if needed)**

- Add unit test to validate bytes/last_read updates.

**Step 2: Run test (expect FAIL)**
Run: `cargo test -p hpx --lib ping`

**Step 3: Implement atomics + mutex split**

**Step 4: Run tests (expect PASS)**

**Step 5: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `perf(hpx): add atomic fast path for h2 ping`

### Task 4.4: TLS session cache to parking_lot

**Files:**

- Modify: `crates/hpx/src/tls/boring.rs`

**Step 1: Failing compile check**

- Update imports and .lock() calls.

**Step 2: Build (expect PASS)**
Run: `cargo build`

**Step 3: Verify + commit**
Run: `just lint`
Commit: `perf(hpx): switch tls cache to parking_lot`

### Task 4.5: Contention benchmarks

**Files:**

- Add: `crates/hpx/benches/contention.rs`

**Step 1: Write benches**

- Add pool and ping benchmarks for 1/4/16 concurrency.

**Step 2: Build benches**
Run: `cargo bench -p hpx --no-run`

**Step 3: Verify + commit**
Run: `cargo build` and `just lint`.
Commit: `bench(hpx): add contention benchmarks`

### Task 4.6: Phase validation

- `cargo fmt --all -- --check`
- `cargo clippy -p hpx -- -D warnings`
- `cargo test -p hpx --lib --tests`
- `cargo test -p hpx-transport --lib --tests`

Commit: `chore: validate lock optimizations phase`

---

## Phase 5: Cleanup & Docs

### Task 5.1: Remove dead code

**Files:**

- Modify: various

**Steps:**

- Remove unused actor code/allow attributes.
- Run `cargo clippy --workspace -- -D warnings`.
- Commit: `chore: remove websocket actor dead code`

### Task 5.2: Update sync.rs docs

**Files:**

- Modify: `crates/hpx/src/sync.rs`

**Steps:**

- Add module doc preferring `parking_lot::Mutex` for new code.
- Run `cargo build` + `just lint`.
- Commit: `docs(hpx): document sync mutex guidance`

### Task 5.3: Update crate-level documentation

**Files:**

- Modify: `crates/hpx-transport/src/lib.rs`, `crates/hpx-transport/README.md`

**Steps:**

- Add Connection API docs and example.
- Run `cargo doc -p hpx-transport --no-deps`.
- Commit: `docs(transport): document Connection API`

### Task 5.4: Update design docs

**Files:**

- Modify: `docs/hpx-actor-design.md`, `docs/hpx-actor-tasks.md`, `docs/hpx-transport-ws-design.md`

**Steps:**

- Update diagrams and descriptions to match implementation.
- Run `cargo build` + `just lint`.
- Commit: `docs: refresh websocket actor design`

### Task 5.5: Final validation

**Steps:**

- `cargo fmt --all -- --check`
- `cargo clippy --workspace -- -D warnings`
- `cargo test -p hpx -p hpx-transport --lib --tests`
- `cargo doc --workspace --no-deps`

Commit: `chore: final validation for actor refactor`
