# hpx Concurrency Refactor — Task List

> Performance-first refactoring: single-owner WebSocket, zero-spawn requests, RAII subscriptions, targeted lock optimizations.
>
> Reference design: [docs/hpx-actor-design.md](hpx-actor-design.md)
>
> Reference implementation: `::hypercore::ws` (single-task loop + reconnect + resubscribe)

---

## Phase 1: Fix Correctness Bugs (Priority: P0)

> **Goal**: Fix critical bugs in the current WebSocket actor that cause data loss and resource leaks.
> **Risk**: Low — targeted fixes within existing architecture.
> **Effort**: 1-2 days.

### Task 1.1: Fix double-connection bug

**File**: `crates/hpx-transport/src/websocket/actor.rs`

- [x] Locate the second WebSocket connection created in `run_ready_loop_internal()` (around the ready-loop in the file).
- [x] Refactor so that `run_ready_loop_internal()` receives the already-authenticated WebSocket (read/write halves) from `run_connection()` instead of opening a new one.
- [x] Ensure authentication and re-subscription happen on the same connection that the ready-loop reads from.
- [x] Add integration test: connect → authenticate → verify messages arrive on the authenticated connection.

**Acceptance**: Authentication tokens and subscriptions survive the transition into the ready loop.

### Task 1.2: Add handle-drop shutdown

**File**: `crates/hpx-transport/src/websocket/actor.rs`

- [x] Detect when all `WsClient` / command sender clones are dropped (`cmd_rx.recv()` returns `None`).
- [x] Initiate graceful shutdown: close WebSocket, clear pending requests with error, exit task.
- [x] Add test: drop all `WsClient` clones → verify connection task terminates.

**Acceptance**: Connection task exits within 1 second of last handle drop.

### Task 1.3: Add regression tests for the fixed bugs

**Files**: `crates/hpx-transport/tests/*`

- [x] Regression test for double-connection (auth + resubscribe on same socket).
- [x] Regression test for handle-drop shutdown.

**Acceptance**: Both tests fail on old code and pass after Phase 1 fixes.

---

## Phase 2: New Connection API (Priority: P0)

> **Goal**: Introduce `Connection`, `ConnectionHandle`, `ConnectionStream`, `Event` — the single-task connection model.
> **Risk**: Medium — new code path, but existing `WsClient` preserved as wrapper.
> **Effort**: 2-3 days.

### Task 2.1: Define core types

**File**: `crates/hpx-transport/src/websocket/connection.rs` (new)

- [x] Define `ConnectionEpoch(u64)`.
- [x] Define `ControlCommand` enum: `Close`, `Reconnect { reason }`.
- [x] Define `DataCommand` enum: `Subscribe`, `Unsubscribe`, `Send`, `Request`.
- [x] Define `IncomingMessage`:
  - `raw: WsMessage`
  - `text: Option<String>`
  - `kind: MessageKind`
  - `topic: Option<Topic>`
- [x] Define `Event` enum: `Connected { epoch }`, `Disconnected { epoch, reason }`, `Message(IncomingMessage)`.
- [x] All types: derive `Debug`; `Event` and commands: derive `Clone` where appropriate.

**Acceptance**: Types compile; `cargo clippy -p hpx-transport` passes.

### Task 2.2: Implement ConnectionHandle

**File**: `crates/hpx-transport/src/websocket/connection.rs`

- [x] Define `ConnectionHandle` struct:
  - `ctrl_tx: mpsc::Sender<ControlCommand>`
  - `cmd_tx: mpsc::Sender<DataCommand>`
  - `pending: Arc<PendingRequestStore>`
  - `subs: Arc<SubscriptionStore>`
  - `config: Arc<WsConfig>`
- [x] Implement `Clone` for `ConnectionHandle`.
- [x] Implement `request<R, T>(&self, req: &R) -> TransportResult<T>`:
  - Insert into pending store → get receiver.
  - Send `DataCommand::Request` via `cmd_tx`.
  - Await receiver with timeout.
  - On timeout: call `pending.remove(id)`.
- [x] Implement `subscribe(&self, topic) -> TransportResult<SubscriptionGuard>`.
- [x] Implement `unsubscribe(&self, topics)`.
- [x] Implement `send(&self, msg)` — fire-and-forget via `cmd_tx`.
- [x] Implement `close(&self)` — send `ControlCommand::Close` via `ctrl_tx`.
- [x] Implement `is_connected(&self) -> bool` — `!ctrl_tx.is_closed()`.

**Acceptance**: `ConnectionHandle` compiles with all methods.

### Task 2.3: Implement ConnectionStream

**File**: `crates/hpx-transport/src/websocket/connection.rs`

- [x] Define `ConnectionStream` struct wrapping `mpsc::Receiver<Event>`.
- [x] Implement `futures::Stream<Item = Event>` for `ConnectionStream`.
- [x] Implement `ConnectionStream::next(&mut self) -> Option<Event>` convenience method.

**Acceptance**: `ConnectionStream` implements `Stream`.

### Task 2.4: Implement connection task loop

**File**: `crates/hpx-transport/src/websocket/connection.rs`

- [x] Implement `async fn connection_task(...)` — the single-owner loop:

  ```rust
  loop {
      tokio::select! {
          biased;  // ctrl_rx always checked first
          cmd = ctrl_rx.recv() => { /* handle Close/Reconnect */ },
          cmd = cmd_rx.recv() => { /* handle data commands */ },
          msg = ws_read.next() => { /* handle incoming WS messages */ },
          _ = ping_interval.tick() => { /* send ping, check pong timeout */ },
          _ = cleanup_interval.tick() => { /* cleanup stale pending */ },
      }
  }
  ```

- [x] Handle `ctrl_rx.recv() => None` (all handles dropped) → graceful shutdown.
- [x] Handle `cmd_rx.recv() => None` → same as above.
- [x] Handle `ws_read.next() => None` (connection closed) → initiate reconnect.
- [x] Handle `ws_read.next() => Err(e)` → log error, initiate reconnect.
- [x] Emit `Event::Message` only for `MessageKind::System | Control | Unknown` (avoid duplicating response/update routing).

**Acceptance**: Connection task compiles; handles all select branches.

### Task 2.5: Implement connect + authentication flow

**File**: `crates/hpx-transport/src/websocket/connection.rs`

- [x] Implement `async fn establish_connection(config, handler) -> Result<(WsRead, WsWrite)>`:
  - Create WebSocket connection using existing `connect_websocket()` logic.
  - Split into read/write halves.
- [x] Implement on-connect messages: `for msg in handler.on_connect()` → send.
- [x] Implement auth flow with existing hooks:
  - If `config.auth_on_connect` and `handler.build_auth_message()` is `Some`, send it.
  - Read frames until `is_auth_success` or `is_auth_failure` (respect `request_timeout`).
- [x] Implement re-subscription: iterate `SubscriptionStore` topics, send subscribe messages.
- [x] Increment `ConnectionEpoch` on successful connect.
- [x] Emit `Event::Connected { epoch }`.
- [x] **Critical**: Ensure the **same** read/write halves are used in the select loop (no double-connection).

**Acceptance**: Connection + auth + resubscribe all use the same WebSocket; no double-connection.

### Task 2.6: Implement reconnection with exponential backoff

**File**: `crates/hpx-transport/src/websocket/connection.rs`

- [x] On disconnect/error:
  - Emit `Event::Disconnected { epoch, reason }`.
  - Resolve all pending requests for current epoch with error.
  - Calculate backoff using `WsConfig`: `reconnect_initial_delay`, `reconnect_max_delay`, `reconnect_backoff_factor`, `reconnect_jitter`.
  - Use full-jitter (recommended): `sleep = rand(0..=delay)` where `delay` is the exponential backoff.
  - Sleep for backoff duration.
  - Attempt reconnection.
  - On success: reset attempt counter; re-authenticate; re-subscribe; resume loop.
  - On max attempts exceeded: emit error, shut down.
- [x] Add tests:
  - Unit test for backoff calculation with `reconnect_jitter = 0.0` (deterministic).
  - Integration test: disconnect → backoff → reconnect → resubscribe → `Event::Connected` emitted.

**Acceptance**: Reconnection works with configured backoff; integration test passes.

### Task 2.7: Implement Connection::connect entry point

**File**: `crates/hpx-transport/src/websocket/connection.rs`

- [x] Implement `Connection::connect(config, handler) -> Result<(ConnectionHandle, ConnectionStream)>`:
  - Validate config.
  - Create channels: `ctrl_tx/ctrl_rx`, `cmd_tx/cmd_rx`, `event_tx/event_rx`.
  - Create `PendingRequestStore` and `SubscriptionStore`.
  - Spawn `connection_driver(...)` via `tokio::spawn` (driver runs `connection_task`).
  - Return `(ConnectionHandle, ConnectionStream)`.
- [x] Add `event_channel_capacity` to `WsConfig` with default `256`.
- [x] Implement event backpressure policy: `Event::Connected/Disconnected` use `send().await`, `Event::Message` uses `try_send` (drop + counter/log).
- [x] Optional: provide `Connection::split()` and/or implement `Stream` for `Connection` to match `::hypercore::ws` ergonomics.

**Acceptance**: `Connection::connect()` returns handle + stream; connection task is running; event channel backpressure does not block lifecycle events.

### Task 2.8: Wire WsClient as compatibility wrapper

**File**: `crates/hpx-transport/src/websocket/ws_client.rs`

- [x] Refactor `WsClient<H>` to hold `ConnectionHandle` internally.
- [x] `WsClient::connect()`:
  - Call `Connection::connect(config, handler)`.
  - Spawn a background task that drains `ConnectionStream` (discard events for compat).
  - Store handle + stream task handle.
- [x] Delegate all public methods (`request`, `subscribe`, `send`, `close`, `is_connected`) to `self.handle`.
- [x] Keep `PhantomData<H>` for type parameter (or remove if no longer needed).
- [x] Maintain exact same public API signatures.

**Acceptance**: All existing `WsClient` consumers compile without changes.

### Task 2.9: Update module exports

**File**: `crates/hpx-transport/src/websocket/mod.rs`

- [x] Add `pub mod connection;`.
- [x] Re-export: `Connection`, `ConnectionHandle`, `ConnectionStream`, `Event`, `ConnectionEpoch`.
- [x] Keep all existing exports (`WsClient`, `WsConfig`, `ProtocolHandler`, `ConnectionState`, etc.).

**Acceptance**: `use hpx_transport::websocket::*` works with both old and new APIs.

### Task 2.10: Remove old ConnectionActor code

**File**: `crates/hpx-transport/src/websocket/actor.rs`

- [x] Remove `ActorCommand` enum.
- [x] Remove `ConnectionActor` struct and all `impl` blocks.
- [x] Remove manual `run()` / `run_connection()` / `run_ready_loop_internal()` methods.
- [x] Clean up imports.
- [x] Either delete `actor.rs` entirely or repurpose for connection task internals.

**Acceptance**: No dead code warnings; `cargo clippy` clean.

### Task 2.11: Run full validation

- [x] `cargo fmt --all -- --check` — no formatting issues.
- [x] `cargo clippy -p hpx-transport -- -D warnings` — zero warnings.
- [x] `cargo test -p hpx-transport --lib --tests` — all tests pass.
- [x] `cargo doc -p hpx-transport --no-deps` — docs build cleanly.

**Acceptance**: All CI commands pass with zero errors.

---

## Phase 3: Zero-Spawn & RAII Subscriptions (Priority: P1)

> **Goal**: Eliminate per-request task spawns and make subscriptions auto-cleanup.
> **Risk**: Low — builds on Phase 2 architecture.
> **Effort**: 1-2 days.

### Task 3.1: Implement zero-spawn request path

**Files**: `crates/hpx-transport/src/websocket/connection.rs`, `crates/hpx-transport/src/websocket/pending.rs`

- [x] Note: base zero-spawn path exists in `ConnectionHandle::request`; ensure it is the only path by removing any remaining spawn-based forwarding from legacy code.
- [x] Ensure `PendingRequestStore::add()` returns `oneshot::Receiver<TransportResult<String>>` (already the case).
- [x] Add `PendingRequestStore::remove(id)` to cancel a pending request on timeout.
- [x] `ConnectionHandle::request()`:
  - Call `pending.add(id)` → get receiver.
  - Send `DataCommand::Request { message, request_id }` (no oneshot in the command).
  - `tokio::select!` on receiver + timeout.
  - On timeout: call `pending.remove(id)`.
- [x] Connection task: on incoming response, call `pending.resolve(id, result)` → completes the receiver.
- [x] Remove any `tokio::spawn(...)` used to forward request responses.

**Acceptance**: Requests work end-to-end with zero intermediate tasks; latency improves.

### Task 3.2: Implement SubscriptionGuard

**Files**: `crates/hpx-transport/src/websocket/connection.rs`, `crates/hpx-transport/src/websocket/subscription.rs`

- [x] Define `SubscriptionGuard` in `connection.rs` (needs `DataCommand` + `cmd_tx`).
- [x] Add `SubscriptionStore::decrement_ref(&self, topic) -> Option<usize>` in `subscription.rs`.
- [x] `Drop for SubscriptionGuard`:
  - Decrement ref.
  - If ref hits 0, `try_send(DataCommand::Unsubscribe { topics })`.
- [x] Implement `SubscriptionGuard::recv(&mut self) -> Option<WsMessage>`.
- [x] Implement `Deref<Target = broadcast::Receiver<WsMessage>>` for compatibility.

**Acceptance**: `SubscriptionGuard` compiles; dropping it sends unsubscribe when last.

### Task 3.3: Update ConnectionHandle::subscribe to return SubscriptionGuard

**File**: `crates/hpx-transport/src/websocket/connection.rs`

- [x] `subscribe()` returns `SubscriptionGuard` instead of bare `broadcast::Receiver`.
- [x] Wire `cmd_tx` clone into the guard for unsubscribe-on-drop.
- [x] Update `WsClient::subscribe()` wrapper accordingly.

**Acceptance**: Subscribe returns guard; unsubscribe fires on drop.

### Task 3.4: Add subscription lifecycle tests

**File**: `crates/hpx-transport/tests/subscription_lifecycle.rs` (new)

- [x] Test: subscribe → drop guard → verify unsubscribe command sent.
- [x] Test: subscribe twice to same topic → drop one guard → no unsubscribe sent; drop second → unsubscribe sent.
- [x] Test: subscribe → explicit unsubscribe → drop guard → no double unsubscribe.

**Acceptance**: All subscription lifecycle tests pass.

### Task 3.5: Run full validation

- [x] `cargo fmt --all -- --check` — clean.
- [x] `cargo clippy -p hpx-transport -- -D warnings` — clean.
- [x] `cargo test -p hpx-transport --lib --tests` — all pass.

**Acceptance**: All CI commands pass.

---

## Phase 4: Lock Optimizations (Priority: P2)

> **Goal**: Reduce contention in Pool, H2 Ping, and TLS with targeted optimizations.
> **Risk**: Low-Medium — changes are internal to hpx, public API unchanged.
> **Effort**: 1-2 days.

### Task 4.1: Add parking_lot dependency

**Workspace**: `Cargo.toml`

- [x] `cargo add parking_lot --workspace`
- [x] `cargo add parking_lot -p hpx --workspace`
- [x] Verify workspace builds.

**Acceptance**: `parking_lot` in workspace deps; `cargo build` passes.

### Task 4.2: Implement sharded connection pool

**File**: `crates/hpx/src/client/http/client/pool.rs`

- [x] Define `ShardedPool<T, K>` struct with `Vec<parking_lot::Mutex<PoolShard<T, K>>>`.
- [x] Implement `shard_for(key) -> &Mutex<PoolShard>` using hash of key.
- [x] Implement `checkout(key) -> Option<T>` — lock only the relevant shard.
- [x] Implement `checkin(key, conn)` — lock only the relevant shard; wake waiters.
- [x] Replace `Pool::inner: Option<Arc<Mutex<PoolInner>>>` with `Option<Arc<ShardedPool>>`.
- [x] Update `Checkout` future to work with sharded pool.
- [x] Update `Pooled<T, K>` Drop impl for sharded pool.
- [x] Default shard count: 16 (or configurable).
- [x] Consider feature flag `sharded-pool` if risk mitigation is needed.

**Acceptance**: Pool checkout/checkin uses per-shard locking; existing pool tests pass.

### Task 4.3: Optimize H2 ping with atomic fast path

**File**: `crates/hpx/src/client/core/proto/h2/ping.rs`

- [x] Split `Shared` into hot-path atomics and cold-path `parking_lot::Mutex`:
  - `bytes: AtomicU64`
  - `last_read_at: AtomicU64`
  - `is_keep_alive_timed_out: AtomicBool`
  - `inner: parking_lot::Mutex<PingInner>` (ping_pong, ping_sent_at, next_bdp_at)
- [x] Update `Recorder::record_data(len)`:
  - `self.shared.bytes.fetch_add(len, Ordering::Relaxed)`
  - `self.shared.last_read_at.store(now_ticks, Ordering::Release)`
- [x] Update `Recorder::record_non_data()`:
  - `self.shared.last_read_at.store(now_ticks, Ordering::Release)`
- [x] Update `Ponger` methods to read atomics for fast checks, lock `inner` only for BDP/ping operations.
- [x] Keep `Ordering::SeqCst` initially if unsure; optimize ordering after benchmarking.
- [x] Consider feature flag `atomic-ping` if risk mitigation is needed.

**Acceptance**: H2 ping works correctly; DATA frame recording is lock-free.

### Task 4.4: Swap TLS session cache to parking_lot

**File**: `crates/hpx/src/tls/boring.rs`

- [x] Replace `use crate::sync::Mutex` with `use parking_lot::Mutex`.
- [x] Update `.lock()` calls (no `.unwrap()` needed — `parking_lot` doesn't poison).
- [x] Verify TLS handshake still works (manual test with HTTPS request).

**Acceptance**: TLS session cache uses `parking_lot::Mutex`; HTTPS works.

### Task 4.5: Add contention benchmarks

**File**: `crates/hpx/benches/contention.rs` (new)

- [x] Benchmark: Pool checkout/checkin with 1/4/16 concurrent tasks across 10 hosts (before/after sharding).
- [x] Benchmark: H2 `record_data()` with 1/4/16 threads (before: Mutex, after: atomic).
- [x] Document results in benchmark output.

**Acceptance**: Benchmarks compile and run; results show improvement under contention.

### Task 4.6: Run full validation

- [x] `cargo fmt --all -- --check` — clean.
- [x] `cargo clippy -p hpx -- -D warnings` — clean.
- [x] `cargo test -p hpx --lib --tests` — all pass.
- [x] `cargo test -p hpx-transport --lib --tests` — all pass (no regression).

**Acceptance**: All CI commands pass with zero errors.

---

## Phase 5: Cleanup & Documentation (Priority: P3)

> **Goal**: Remove dead code, update docs, final polish.
> **Risk**: Low.
> **Effort**: 0.5 day.

### Task 5.1: Remove dead code

**Files**: Various

- [x] Remove unused `ActorCommand` references if any remain.
- [x] Remove old `ConnectionActor` if not already done in Phase 2.
- [x] Remove any `#[allow(dead_code)]` that was added temporarily.
- [x] Run `cargo clippy` — verify zero dead code warnings.

**Acceptance**: No dead code.

### Task 5.2: Update sync.rs documentation

**File**: `crates/hpx/src/sync.rs`

- [x] Add module doc: note that new code should prefer `parking_lot::Mutex` or lock-free structures.
- [x] Keep the non-poisoning wrappers (may be used by downstream code).

**Acceptance**: Documentation updated.

### Task 5.3: Update crate-level documentation

**Files**: `crates/hpx-transport/src/lib.rs`, `crates/hpx-transport/README.md`

- [x] Add documentation for new `Connection` / `ConnectionHandle` / `ConnectionStream` API.
- [x] Add usage example showing the split API.
- [x] Update WebSocket module documentation.

**Acceptance**: `cargo doc -p hpx-transport --no-deps` builds clean; docs include new API.

### Task 5.4: Update design docs

**Files**: `docs/hpx-actor-design.md`, `docs/hpx-actor-tasks.md`, `docs/hpx-transport-ws-design.md`

- [x] Update architecture diagrams to reflect single-task model.
- [x] Document the Connection/Handle/Stream split.
- [x] Document command priority (two-channel design).
- [x] Document reconnect/backoff policy + event backpressure behavior.
- [x] Document  parity and the kameo decision (why not in hot paths).
- [x] Update `docs/hpx-transport-ws-design.md` to reflect the driver loop, backoff, and event policy.

**Acceptance**: Design docs reflect implemented architecture.

### Task 5.5: Final validation

- [x] `cargo fmt --all -- --check` — clean.
- [x] `cargo clippy --workspace -- -D warnings` — clean (or known pre-existing exceptions).
- [x] `cargo test -p hpx -p hpx-transport --lib --tests` — all pass.
- [x] `cargo doc --workspace --no-deps` — no new warnings.

**Acceptance**: Everything green.

### Task 5.6: Fix doc link warnings

**File**: `crates/hpx/src/lib.rs`

- [x] Remove broken `multipart` intra-doc link in the overview list.
- [x] Avoid linking to `ClientBuilder::cookie_store` when the `cookies` feature is off.

**Acceptance**: `cargo doc --workspace --no-deps` has no new link warnings.

### Task 5.7: Clean up example warning

**File**: `crates/yawc/examples/axum.rs`

- [x] Remove unused `Frame` import.

**Acceptance**: `just test` no longer warns about unused imports in yawc examples.

### Task 5.8: Gate network emulation tests

**Files**: `crates/hpx/tests/emulation.rs`, `crates/hpx-util/tests/support/mod.rs`

- [x] Skip network emulation tests unless `HPX_NETWORK_TESTS=1` is set.
- [x] Ensure just test no longer fails on flaky external connections by default.

**Acceptance**: `just test` passes without requiring external network access.

---

## Phase 6: Test Stability & Tooling (Post-Refactor)

### Task 6.1: Stabilize subscription lifecycle tests

**File**: `crates/hpx-transport/tests/subscription_lifecycle.rs`

- [x] Increase timeouts for connection and message expectations to avoid CI flakiness.
- [x] Ensure no unexpected unsubscribe messages during guard drops.

**Acceptance**: `cargo nextest run -p hpx-transport --filter-expr 'binary(/subscription_lifecycle/)'` is stable with retries disabled.

### Task 6.2: Stabilize actor shutdown test

**File**: `crates/hpx-transport/tests/websocket_actor_shutdown_on_drop_test.rs`

- [x] Increase connect/shutdown timeouts to handle loaded test runs.

**Acceptance**: `cargo nextest run -p hpx-transport --filter-expr 'binary(/websocket_actor_shutdown_on_drop_test/)'` is stable with retries disabled.

### Task 6.3: Resolve example output filename collisions

**Files**: `crates/yawc/Cargo.toml`, `crates/yawc/autobahn/client-test.js`, `crates/yawc/benches/run.js`, `crates/yawc/benches/Makefile`, `crates/yawc/examples/custom_dns.rs`

- [x] Rename yawc examples that collide with fastwebsockets (`axum`, `autobahn_client`, `echo_server`).
- [x] Update yawc scripts and docs to reference the new example names.

**Acceptance**: `just test` emits no cargo example filename collision warnings.

---

## Task Dependency Graph

```text
Phase 1: Fix Correctness (can start immediately)
  1.1 (double-conn fix)
  1.2 (handle-drop shutdown)    ─┐
  1.3 (regression tests)        ─┤
                                 │
                                 ▼
Phase 2: New Connection API (depends on Phase 1)
  2.1 (core types)
  2.2 (ConnectionHandle) ─────────┐
  2.3 (ConnectionStream)          │
  2.4 (connection task loop)      │
  2.5 (connect + auth flow)       ├── all converge
  2.6 (reconnection)              │
  2.7 (Connection::connect)  ◄────┘
  2.8 (WsClient wrapper)
  2.9 (module exports)
  2.10 (remove old actor)
  2.11 (validate)
                │
                ▼
Phase 3: Zero-Spawn & RAII (depends on Phase 2)
  3.1 (zero-spawn request)
  3.2 (SubscriptionGuard)
  3.3 (subscribe returns guard)
  3.4 (lifecycle tests)
  3.5 (validate)
                │
                ▼
Phase 4: Lock Optimizations (parallel with Phase 3 for hpx crate)
  4.1 (parking_lot dep)
  4.2 (sharded pool)        ── independent ──┐
  4.3 (atomic H2 ping)      ── independent ──┤
  4.4 (TLS parking_lot)     ── independent ──┤
  4.5 (benchmarks)          ◄────────────────┘
  4.6 (validate)
                │
                ▼
Phase 5: Cleanup (after all phases)
  5.1 → 5.2 → 5.3 → 5.4 → 5.5
```

---

## Priority & Effort Summary

| Phase | Priority | Effort | Risk | Value |
|-------|----------|--------|------|-------|
| Phase 1: Fix Correctness | **P0** | 1-2 days | Low | Critical — fixes data loss bugs |
| Phase 2: New Connection API | **P0** | 2-3 days | Medium | High — new architecture foundation |
| Phase 3: Zero-Spawn & RAII | **P1** | 1-2 days | Low | High — latency + resource leak fix |
| Phase 4: Lock Optimizations | **P2** | 1-2 days | Low-Med | Medium — contention reduction |
| Phase 5: Cleanup | **P3** | 0.5 day | Low | Hygiene |

**Total estimated effort**: 5.5-9.5 days

---

## Key Differences from Original Task List

| Original (kameo-Everywhere) | Updated (Performance-First) |
|-----------------------------|-----------------------------|
| Phase 0: Add kameo dependency | **Removed** — no kameo needed |
| Phase 1: kameo WsConnectionActor (15 tasks) | Phase 2: Single-task Connection (11 tasks) |
| Phase 2: kameo PoolActor (10 tasks) | Phase 4.2: Sharded pool (1 task) |
| Phase 3: kameo PingActor (5 tasks) | Phase 4.3: Atomic fast path (1 task) |
| Phase 4: kameo SessionCacheActor (4 tasks) | Phase 4.4: parking_lot swap (1 task) |
| Phase 5: Deprecate sync.rs for kameo | Phase 5: Document, don't deprecate |
| **No correctness fix phase** | **Phase 1: Fix [P1]/[P2] bugs first** |
| **No RAII subscriptions** | **Phase 3: SubscriptionGuard** |
| **No connection epochs** | **Phase 2: ConnectionEpoch** |
| **No command priorities** | **Phase 2: Two-channel design** |
| Total: 30+ tasks, 9-12 days | Total: ~25 tasks, 5.5-9.5 days |
