# hpx Concurrency Refactor — Performance-First Design

> Replace correctness bugs and unnecessary contention with a **single-owner WebSocket state machine**, zero-spawn request paths, RAII subscriptions, and targeted lock optimizations. This design does **not** mandate an external actor framework; it favors a minimal, single-task model (proven in ``) with typed commands and explicit lifecycle events.

---

## 0. Design Options and Decision

An earlier revision proposed converting **all** Mutex-protected subsystems (WebSocket, Pool, H2 Ping, TLS Session Cache) to kameo actors. After code-driven analysis and comparing against `::hypercore::ws`, we select a performance-first design with optional hybrid use of kameo only where it adds clear value.

### Options Considered

| Dimension | kameo-Everywhere (Original) | Hybrid (kameo control-plane only) | Performance-First (Selected) |
|-----------|----------------------------|------------------------------------|------------------------------|
| **WebSocket** | kameo Actor with typed Messages | Single-task WS loop, optional kameo supervisor | Single-task state machine + typed Commands |
| **Connection Pool** | kameo PoolActor | Shard by key + `parking_lot::Mutex` | Shard by key + `parking_lot::Mutex` |
| **H2 Ping** | kameo PingActor | Atomic fast path + `parking_lot::Mutex` | Atomic fast path + `parking_lot::Mutex` |
| **TLS Session Cache** | kameo SessionCacheActor | Keep with faster lock | Keep with faster lock |
| **Correctness bugs** | Not explicitly addressed | Fixed | Fixed |
| **New dependency** | `kameo` in hpx + hpx-transport | Optional `kameo` (control-plane only) | `parking_lot` only (no framework) |
| **Estimated effort** | 9-12 days, 5 phases | 6-8 days, 5 phases | 5-7 days, 4 phases |
| **Risk** | Medium-High (framework in hot paths) | Medium (adds supervision complexity) | Low-Medium (targeted fixes) |

**Recommendation**: Performance-first (Selected). Keep a hybrid path open for future supervision or multi-connection orchestration, but do not introduce kameo into the hot paths today.

### Why kameo-Everywhere Was Rejected (And Where kameo Could Still Help)

1. **Pool actor adds latency**: Connection checkout/checkin is a synchronous lookup. Message-passing round-trip (~100ns) is *worse* than an uncontended `parking_lot::Mutex` (~15-25ns). The real fix for contention is **sharding by host key**, not actorization.

2. **H2 ping actor would be slower**: `Recorder::record_data()` fires on *every DATA frame* — this is the hottest path in HTTP/2. A `tell()` via channel is heavier than a `parking_lot::Mutex` lock + two field updates.

3. **TLS session cache is not hot**: Accessed only during TLS handshake (connection establishment), not per-request. Actorization adds overhead and complexity for zero measurable latency benefit.

4. **Correctness bugs dominate**: The current WebSocket actor has a **double-connection bug** (auth/resubscribe lost), **subscription leaks** (no RAII), and **per-request task spawns** (scheduler overhead). Fixing these yields more real-world improvement than any framework change.

5. **kameo still has a role**: If we later need supervision trees, dynamic connection orchestration, or shared control-plane logic across many WebSocket connections, a kameo supervisor layer can wrap the **existing single-task WS loop** without changing the hot path.

### What Was Kept from the Original Design

- **Typed command messages** (enum instead of monolithic `ActorCommand`)
- **Bounded mailbox/channel** for backpressure
- **Lifecycle management** (connect, authenticate, reconnect, close)
- **scc::HashMap stays** for PendingRequestStore, SubscriptionStore, RateLimiter (already optimal)
- **Split API** concept (`ConnectionHandle` + `ConnectionStream`)

### Reference: ::hypercore::ws (Proven Baseline)

`` already uses a single-task connection loop with explicit lifecycle events, auto-reconnect, ping/pong management, and subscription rehydration. Our design intentionally mirrors these proven mechanics while extending them with:

- Typed command channels (control + data).
- Request/response correlation via `PendingRequestStore`.
- RAII subscriptions with drop-triggered unsubscribe.
- Connection epochs for stale request invalidation.

---

## 1. Code-Driven Findings

### [P1] Critical: Double-Connection Bug

**Files**: `crates/hpx-transport/src/websocket/actor.rs`

The `run_ready_loop_internal()` method creates a **second** WebSocket connection. The first connection — where `run_connection()` performed authentication and re-subscription — is discarded. Result: all authentication tokens and subscriptions are lost on the active connection.

**Root cause**: The ready-loop was designed to manage reconnection independently, but it opens a fresh socket instead of reusing the one that was authenticated.

### [P2] High: Subscription Lifetime Leak

**File**: `crates/hpx-transport/src/websocket/subscription.rs`

Dropping a `broadcast::Receiver` does not decrement the subscription's reference count. Over time, server-side subscriptions accumulate because `unsubscribe` is never sent. This is a resource leak that degrades both client and server.

### [P2] High: Per-Request Task Spawn

**File**: `crates/hpx-transport/src/websocket/actor.rs` (`handle_request` method)

Every `Request` command spawns a new tokio task to forward the response from `PendingRequestStore` to the caller. At high QPS (thousands of requests/sec), this creates significant scheduler overhead (task allocation, waking, deallocation).

### [P2] High: Actor Does Not Exit on Handle Drop

When all `WsClient` clones are dropped, the underlying connection actor continues running indefinitely. There is no shutdown signal tied to the last handle's drop.

### [P3] Medium: Pool Global Lock Contention

**File**: `crates/hpx/src/client/http/client/pool.rs`

A single `Arc<Mutex<PoolInner>>` protects all hosts' connection pools. Under concurrent multi-host requests, lock contention scales linearly with the number of hosts.

### [P3] Medium: H2 Ping Hot-Path Lock

**File**: `crates/hpx/src/client/core/proto/h2/ping.rs`

`Arc<Mutex<Shared>>` is locked on every DATA frame via `Recorder::record_data()`. Under sustained HTTP/2 data transfer, this becomes a throughput bottleneck.

### [P3] Low: TLS Session Cache

**File**: `crates/hpx/src/tls/boring.rs`

`Arc<Mutex<SessionCache>>` is used during TLS handshake only. Not on the per-request hot path. Low priority.

---

## 2. Design Goals

1. **Single-owner connection**: One task owns WebSocket read/write and all state transitions.
2. **No per-request spawns** in hot paths.
3. **Explicit lifecycle events**: Connected / Disconnected observability for upstream consumers.
4. **Bounded backpressure**: Predictable queueing and drop behavior.
5. **Lock minimization**: Shard where possible; avoid global Mutex hot spots.
6. **RAII resource management**: Subscriptions auto-cleanup on drop.
7. **Compatibility path**: Migrate from `WsClient` without breaking all callers at once.

## 3. Non-Goals

- Full rewrite of `ProtocolHandler` trait.
- Mandatory adoption of an actor framework (kameo, actix, etc.).
- Refactoring HTTP client / TLS / H2 beyond targeted optimizations.
- Changing `PendingRequestStore` or `SubscriptionStore` internals (already lock-free).

---

## 4. WebSocket Architecture

### 4.1 Overview

The WebSocket runtime is split into an **outer connection driver** and an **inner connection task**. The driver owns reconnect/backoff, while the task owns the hot-path select loop. This mirrors the proven shape in `::hypercore::ws` while adding typed command channels and request routing.

```text
┌──────────────────────────────────────────────────────────────────┐
│                        User Code                                  │
│  let (handle, stream) = Connection::connect(config, handler)?;   │
│  let guard = handle.subscribe("trades.BTC").await?;              │
│  while let Some(event) = stream.next().await { ... }             │
└───────────────────────┬──────────────────────────────────────────┘
                        │
          ┌─────────────┴─────────────┐
          ▼                           ▼
┌──────────────────┐       ┌───────────────────────┐
│ ConnectionHandle │       │ ConnectionStream       │
│                  │       │ impl Stream<Item=Event>│
│ cmd_tx: Sender   │       │ event_rx: Receiver     │
│ pending: Arc<..> │       │                        │
│ subs: Arc<..>    │       │                        │
└────────┬─────────┘       └───────────┬───────────┘
         │                             ▲
         │ Command                     │ Event
         ▼                             │
┌──────────────────────────────────────────────────────────────────┐
│                Connection Driver (outer loop)                     │
│                                                                    │
│  Owns:                                                            │
│    - reconnect attempts + backoff                                 │
│    - ConnectionEpoch (incremented on each connect)                │
│                                                                    │
│  loop {                                                           │
│    establish_connection()  // connect + auth + resubscribe         │
│    run connection_task()    // hot-path select loop                │
│    emit Disconnected + clear pending + backoff                     │
│  }                                                                │
└──────────────────────────────────────────────────────────────────┘
          │
          ▼
┌──────────────────────────────────────────────────────────────────┐
│                 Connection Task (single tokio task)               │
│                                                                    │
│  Owns:                                                            │
│    - WebSocket read/write halves                                  │
│    - ConnectionState state machine                                │
│    - Ping/pong timer                                              │
│                                                                    │
│  select! {                                                        │
│    cmd = ctrl_rx.recv() => handle_control_command(cmd),           │
│    cmd = cmd_rx.recv()  => handle_data_command(cmd),              │
│    msg = ws_read.next() => handle_incoming(msg),                  │
│    _ = ping_interval    => send_ping(),                           │
│    _ = cleanup_interval => cleanup_stale_requests(),              │
│  }                                                                │
└──────────────────────────────────────────────────────────────────┘
         │                      │                       │
         ▼                      ▼                       ▼
┌──────────────┐    ┌────────────────────┐   ┌──────────────────┐
│ WebSocket IO │    │ PendingRequestStore│   │ SubscriptionStore│
│ (split IO)   │    │ (scc::HashMap)     │   │ (scc::HashMap)   │
└──────────────┘    └────────────────────┘   └──────────────────┘
```

### 4.2 Typed Commands (Client → Connection Task)

Replace the monolithic `ActorCommand` enum with a two-channel design:

```rust
/// High-priority control commands (polled first in select!).
enum ControlCommand {
    Close,
    Reconnect { reason: String },
}

/// Normal data commands.
enum DataCommand {
    Subscribe {
        topics: Vec<Topic>,
        reply: oneshot::Sender<TransportResult<()>>,
    },
    Unsubscribe {
        topics: Vec<Topic>,
    },
    Send {
        message: WsMessage,
    },
    Request {
        message: WsMessage,
        request_id: RequestId,
    },
}
```

Two separate `mpsc::channel` instances ensure `Close`/`Reconnect` are never starved by a flood of data commands, since the select! loop polls `ctrl_rx` first (`biased` select).

### 4.3 Events (Connection Task → Client)

```rust
/// Lifecycle events emitted by the connection task.
enum Event {
    /// WebSocket connected and authenticated.
    Connected { epoch: ConnectionEpoch },
    /// WebSocket disconnected.
    Disconnected { epoch: ConnectionEpoch, reason: String },
    /// Optional raw/control message (not routed to pending/subscriptions).
    Message(IncomingMessage),
}

/// Lightweight message view for control/unknown frames.
struct IncomingMessage {
    raw: WsMessage,
    text: Option<String>,
    kind: MessageKind,
    topic: Option<Topic>,
}
```

`Event::Message` is emitted only for **non-routed** messages (e.g., `MessageKind::System`, `Control`, `Unknown`) to avoid duplicating data already delivered through `PendingRequestStore` or `SubscriptionStore`.

**Event backpressure policy**: The event channel is bounded. `Event::Connected` and `Event::Disconnected` must be delivered reliably, while `Event::Message` is best-effort. Recommended implementation: `try_send` for `Event::Message` (drop with a counter/log), and `send().await` for lifecycle events. This prevents a slow consumer from blocking reconnection signaling.

### 4.4 Connection Epochs

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ConnectionEpoch(u64);
```

Incremented on each successful WebSocket connect. Benefits:

- Pending requests from a previous epoch can be resolved as errors immediately on reconnect.
- Stale pong responses are discarded.
- Subscription re-registration can be validated against the current epoch.

### 4.5 Reconnect and Backoff

On disconnect or read/write error, the **connection driver** performs a deterministic sequence:

1. Emit `Event::Disconnected { epoch, reason }`.
2. Resolve all pending requests for the current epoch with error.
3. Compute backoff using `WsConfig`:
   - `delay = min(reconnect_max_delay, reconnect_initial_delay * backoff_factor^attempt)`
   - apply jitter (recommended: **full jitter**, `sleep = rand(0..=delay)`).
4. Sleep for the backoff duration.
5. Attempt reconnect. On success: reset attempt counter, increment epoch, re-auth, re-subscribe.
6. If `reconnect_max_attempts` is exceeded: emit terminal error and shut down.

This matches ’s reconnect loop, but adds explicit epoch accounting and pending-request cleanup.

### 4.6 Zero-Spawn Request Path

**Before** (current, problematic):

```text
WsClient::request()
  → send ActorCommand::Request to actor
  → actor.handle_request() spawns a task
    → task awaits pending_store.wait_for(id)
    → task sends result back via oneshot
  → caller awaits oneshot
```

**After** (zero-spawn):

```text
ConnectionHandle::request()
  → pending_store.insert(id) → returns Receiver<Result>
  → send DataCommand::Request { id, msg }
  → caller awaits Receiver directly (no intermediate task)
  → on timeout: caller calls pending_store.remove(id)
```

The connection task's incoming message handler simply does `pending_store.resolve(id, result)`, completing the caller's receiver. This eliminates one tokio task per request.

### 4.7 RAII Subscriptions

```rust
/// Returned by `ConnectionHandle::subscribe()`.
/// On Drop, sends Unsubscribe if this is the last reference.
pub struct SubscriptionGuard {
    topic: Topic,
    subs: Arc<SubscriptionStore>,
    cmd_tx: mpsc::Sender<DataCommand>,
    rx: broadcast::Receiver<WsMessage>,
}

impl Drop for SubscriptionGuard {
    fn drop(&mut self) {
        let count = self.subs.decrement_ref(&self.topic);
        if count == 0 {
            // Last subscriber — send unsubscribe
            let _ = self.cmd_tx.try_send(DataCommand::Unsubscribe {
                topics: vec![self.topic.clone()],
            });
        }
    }
}

impl SubscriptionGuard {
    /// Receive the next message for this subscription.
    pub async fn recv(&mut self) -> Option<WsMessage> {
        self.rx.recv().await.ok()
    }
}
```

This prevents the subscription leak identified in [P2]. When all `SubscriptionGuard` instances for a topic are dropped, the unsubscribe command is automatically sent.

### 4.8 Connection State Machine

```text
Disconnected ──(connect)──→ Connecting ──(ws_open)──→ Connected
     ↑                                                     │
     │                                              (authenticate)
     │                                                     │
     │                                                     ▼
     │              Reconnecting ◄─── (error/disconnect) ─ Ready
     │                   │
     │              (backoff wait)
     │                   │
     │                   ▼
     │              Connecting ──→ ...
     │
     └──── Closing ◄── (Close cmd) ── any state
                │
                ▼
             Closed
```

State transitions are driven by the connection task's `select!` loop. All state is owned by a single task — no locking required.

### 4.9 ConnectionHandle and ConnectionStream (Split API)

```rust
/// Handle for sending commands to the connection.
/// Clone-safe; when all clones are dropped, the connection task shuts down.
#[derive(Clone)]
pub struct ConnectionHandle {
    ctrl_tx: mpsc::Sender<ControlCommand>,
    cmd_tx: mpsc::Sender<DataCommand>,
    pending: Arc<PendingRequestStore>,
    subs: Arc<SubscriptionStore>,
    config: Arc<WsConfig>,
}

/// Stream of lifecycle events and incoming messages.
pub struct ConnectionStream {
    event_rx: mpsc::Receiver<Event>,
}

impl futures::Stream for ConnectionStream {
    type Item = Event;
    fn poll_next(...) -> Poll<Option<Self::Item>> {
        self.event_rx.poll_recv(cx)
    }
}
```

When all `ConnectionHandle` clones are dropped, `cmd_tx` and `ctrl_tx` are closed. The connection task detects this via `recv() → None` and initiates graceful shutdown. This fixes the actor-doesn't-exit-on-handle-drop bug.

### 4.10 WsClient Compatibility Layer

Existing `WsClient` is preserved as a thin wrapper:

```rust
#[derive(Clone)]
pub struct WsClient<H: ProtocolHandler> {
    handle: ConnectionHandle,
    _stream_task: Arc<tokio::task::JoinHandle<()>>,
    _marker: PhantomData<H>,
}

impl<H: ProtocolHandler> WsClient<H> {
    pub async fn connect(config: WsConfig, handler: H) -> TransportResult<Self> {
        let (handle, stream) = Connection::connect(config, handler).await?;
        // Spawn a task that drains the event stream (for compatibility)
        let stream_task = tokio::spawn(async move {
            let mut stream = stream;
            while let Some(_event) = stream.next().await {}
        });
        Ok(Self {
            handle,
            _stream_task: Arc::new(stream_task),
            _marker: PhantomData,
        })
    }

    pub async fn request<R: Serialize, T: DeserializeOwned>(
        &self, request: &R,
    ) -> TransportResult<T> {
        self.handle.request(request).await
    }

    pub async fn subscribe(&self, topic: impl Into<Topic>) -> TransportResult<SubscriptionGuard> {
        self.handle.subscribe(topic).await
    }

    // ... delegate all methods to self.handle
}
```

This maintains backward compatibility while allowing advanced users to use the `Connection::connect()` API directly.

### 4.11 WsConfig Additions

Add one field for the event channel capacity and document reconnection tuning:

```rust
/// Capacity for ConnectionStream events.
pub event_channel_capacity: usize,

/// Reconnect tuning (already present in WsConfig)
pub reconnect_initial_delay: Duration,
pub reconnect_max_delay: Duration,
pub reconnect_backoff_factor: f64,
pub reconnect_max_attempts: Option<u32>,
pub reconnect_jitter: f64,
```

Default: `256` (align with subscription channel capacity). This is only for lifecycle/optional messages, so a modest bound is sufficient. Reconnect defaults should be safe and conservative, with jitter set to a small non-zero value to avoid thundering herds.

---

## 5. Pool Optimization (Non-Actor)

### 5.1 Problem

`Arc<Mutex<PoolInner<T, K>>>` is a single lock for all hosts. Under concurrent multi-host requests, this is a contention bottleneck.

### 5.2 Solution: Shard by Key

```rust
use parking_lot::Mutex;

struct ShardedPool<T: Poolable, K: Key> {
    shards: Vec<Mutex<PoolShard<T, K>>>,
    shard_count: usize,
}

struct PoolShard<T: Poolable, K: Key> {
    idle: LruMap<K, Vec<Idle<T>>>,
    waiters: HashMap<K, VecDeque<oneshot::Sender<T>>>,
    connecting: HashSet<K>,
}

impl<T: Poolable, K: Key> ShardedPool<T, K> {
    fn new(shard_count: usize) -> Self {
        let shards = (0..shard_count)
            .map(|_| Mutex::new(PoolShard::default()))
            .collect();
        Self { shards, shard_count }
    }

    fn shard_for(&self, key: &K) -> &Mutex<PoolShard<T, K>> {
        let hash = hash_key(key);
        &self.shards[hash % self.shard_count]
    }

    fn checkout(&self, key: &K) -> Option<T> {
        let mut shard = self.shard_for(key).lock();
        shard.try_take_idle(key)
    }

    fn checkin(&self, key: K, conn: T) {
        let mut shard = self.shard_for(&key).lock();
        shard.put_idle(key, conn);
    }
}
```

Benefits:

- Requests to different hosts never contend.
- `parking_lot::Mutex` is faster than `std::sync::Mutex` (no poisoning, adaptive spinning).
- No architectural change — just a data structure swap inside `Pool`.

### 5.3 Alternative: scc::HashMap

Since scc is already a dependency, another option is `scc::HashMap<K, Mutex<PerHostPool<T>>>`. This provides lock-free key lookup and per-host locking for the idle/waiter lists. Evaluate via benchmarking.

---

## 6. H2 Ping Optimization (Non-Actor)

### 6.1 Problem

`Arc<Mutex<Shared>>` is locked on every DATA frame via `Recorder::record_data()`. It's also locked by `Ponger` for BDP and keep-alive checks.

### 6.2 Solution: parking_lot::Mutex + Atomic Fast Path

```rust
use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use parking_lot::Mutex;

struct PingShared {
    /// Hot path fields (atomics, no lock needed)
    bytes: AtomicU64,
    last_read_at: AtomicU64,  // monotonic ticks
    is_keep_alive_timed_out: AtomicBool,

    /// Cold path fields (infrequent BDP calculation)
    inner: Mutex<PingInner>,
}

struct PingInner {
    ping_pong: PingPong,
    ping_sent_at: Option<Instant>,
    next_bdp_at: Option<Instant>,
}
```

- `record_data()` uses `bytes.fetch_add()` and `last_read_at.store()` — pure atomic, zero lock.
- `record_non_data()` uses `last_read_at.store()` — pure atomic.
- BDP calculation and ping sending use the `Mutex<PingInner>` cold path (infrequent).

This is strictly faster than any message-passing approach for the hot path.

---

## 7. TLS Session Cache (Minimal Change)

### 7.1 Assessment

TLS session cache is accessed only during TLS handshake — not per-request. Current `std::sync::Mutex` is adequate, but can be swapped to `parking_lot::Mutex` for consistency and minor improvement.

### 7.2 Solution

```rust
// In crates/hpx/src/tls/boring.rs
use parking_lot::Mutex;

// Replace:  use crate::sync::Mutex;
// With:     use parking_lot::Mutex;
// Adjust .lock() calls (parking_lot returns MutexGuard directly, no Result)
```

No structural change required.

---

## 8. API Strategy

### 8.1 New API (Preferred for new code)

For parity with `::hypercore::ws`, we can optionally expose a `Connection` that implements `Stream` and supports `split()`. If we want to keep the API minimal, `Connection::connect()` can still return `(ConnectionHandle, ConnectionStream)` directly; both shapes are compatible and can coexist.

```rust
use hpx_transport::websocket::{Connection, ConnectionHandle, ConnectionStream, Event};

// Option A: explicit split (-style)
let connection = Connection::connect(config, handler).await?;
let (handle, stream) = connection.split();

// Option B: return parts directly
let (handle, stream) = Connection::connect(config, handler).await?;

// Spawn stream consumer
// Note: Event::Message is only emitted for control/unknown messages by default.
tokio::spawn(async move {
    while let Some(event) = stream.next().await {
        match event {
            Event::Connected { epoch } => info!("Connected (epoch {epoch})"),
            Event::Disconnected { epoch, reason } => warn!("Disconnected: {reason}"),
            Event::Message(msg) => debug!("Control message: {:?}", msg.kind),
        }
    }
});

// Use handle for requests and subscriptions
let guard = handle.subscribe("trades.BTC").await?;
let resp: AccountInfo = handle.request(&GetAccountRequest { .. }).await?;
```

### 8.2 Compatibility Layer (Existing WsClient)

`WsClient<H>` wraps `ConnectionHandle` and drains the event stream internally. All existing public APIs remain unchanged:

| API | Before | After | Breaking? |
|-----|--------|-------|-----------|
| `WsClient::connect()` | `mpsc::Sender` + spawned actor | `ConnectionHandle` + spawned task | **No** |
| `WsClient::request()` | `ActorCommand::Request` + spawn | `DataCommand::Request` + zero-spawn | **No** |
| `WsClient::subscribe()` | `broadcast::Receiver` | `SubscriptionGuard` (wraps receiver) | **Minor** |
| `WsClient::send()` | `cmd_tx.send()` | `handle.send()` | **No** |
| `WsClient::close()` | `ActorCommand::Close` | `handle.close()` | **No** |
| `WsClient::is_connected()` | `!cmd_tx.is_closed()` | `!ctrl_tx.is_closed()` | **No** |

The only potentially breaking change is `subscribe()` returning `SubscriptionGuard` instead of bare `broadcast::Receiver`. Mitigate with `SubscriptionGuard` implementing `Deref<Target = broadcast::Receiver<WsMessage>>` or providing `.into_receiver()`.

---

## 9. Migration Plan

### Phase 1: Fix Correctness (Highest Priority)

1. Fix double-connection bug — ensure the ready-loop reuses the authenticated connection.
2. Add shutdown detection — connection task exits when all handles are dropped.
3. Add regression tests for auth/resubscribe and handle-drop shutdown.

### Phase 2: New Connection API

1. Introduce `Connection`, `ConnectionHandle`, `ConnectionStream`, `Event`.
2. Implement single-task connection loop with `select!`.
3. Implement two-channel command system (control + data).
4. Implement connection epochs.
5. Wire `WsClient` as compatibility wrapper.

### Phase 3: Zero-Spawn & RAII

1. Refactor request path: caller awaits `PendingRequestStore` receiver directly.
2. Implement `SubscriptionGuard` with RAII drop.
3. Remove per-request task spawn.

### Phase 4: Lock Optimizations

1. Pool: sharding by key + `parking_lot::Mutex`.
2. H2 Ping: atomic fast path + `parking_lot::Mutex` cold path.
3. TLS: swap to `parking_lot::Mutex`.

---

## 10. Benchmark & Validation Plan

### Metrics

| Metric | Tool | Target |
|--------|------|--------|
| WS request/response P99 latency | `criterion` | Lower than baseline |
| WS throughput (msg/sec) | `criterion` | Improve under high concurrency |
| Task spawn count per request | manual audit | 0 (down from 1) |
| Reconnect time-to-recovery | integration test | < 2s average |
| Pool checkout contention | `criterion` (multi-thread) | Linear scaling with hosts |
| H2 ping overhead in data path | `criterion` | Reduced vs Mutex baseline |

### Success Criteria

Before merging each phase:

- P99 latency improvement or no regression on WS request/response.
- Lower scheduler load (fewer spawned tasks).
- No regression in reconnection correctness.
- All existing tests pass.
- Zero clippy warnings.

---

## 11. Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| API churn from `subscribe()` return type | Medium | Low | Compatibility wrapper with `Deref` or `.into_receiver()` |
| Behavioral differences in reconnection | Low | High | Integration tests for reconnect, resubscribe, timeout cleanup |
| Backpressure tuning needed | Medium | Medium | Start with bounded channels, expose capacity in `WsConfig` |
| Event stream drops under pressure | Medium | Low | Best-effort `Event::Message`, reliable lifecycle events, counters/logs |
| Pool sharding uneven distribution | Low | Low | Consistent hashing; benchmark distribution |
| Atomic H2 ping ordering issues | Low | Medium | Use `Ordering::SeqCst` initially, relax after testing |

---

## 12. Dependency Changes

### New Dependencies (Workspace)

```toml
[workspace.dependencies]
parking_lot = "0.12"   # Faster mutex for Pool, H2 Ping, TLS
```

### WsConfig Updates

Add:

```toml
# Capacity for ConnectionStream events
ws.event_channel_capacity = 256
```

### Explicitly NOT Added

- `kameo` — not on hot paths; consider a future **hybrid** supervisor if control-plane needs grow
- `dashmap` — project guidelines prefer `scc`; Pool sharding uses `parking_lot::Mutex` directly

---

## 13. What Stays Unchanged

| Component | Mechanism | Reason |
|-----------|-----------|--------|
| `PendingRequestStore` | `scc::HashMap` | Lock-free, already optimal |
| `SubscriptionStore` | `scc::HashMap` | Lock-free, already optimal |
| `RateLimiter` | `scc::HashMap` | Lock-free, already optimal |
| `ProtocolHandler` trait | Trait + generic | Clean abstraction, no change needed |
| `RestClient` / `ExchangeClient` | Stateless HTTP dispatch | No concurrency concern |
| `WsConfig` | `Arc<WsConfig>` (immutable) | Read-only after creation |

---

## 14. Summary

The optimal design is **not** "actorize everything," but rather:

1. Make WebSocket a **single-owner, single-task state machine** with explicit lifecycle events (Connection/Handle/Stream split API).
2. **Fix critical correctness bugs** first (double-connection, subscription leaks, actor doesn't exit on handle drop).
3. **Remove per-request task spawns** via zero-spawn request path.
4. **RAII subscriptions** to prevent resource leaks.
5. **Connection epochs** to invalidate stale state across reconnections.
6. **Command priorities** via two-channel design (control + data).
7. **Pool**: Shard by key + `parking_lot::Mutex` — not actor.
8. **H2 Ping**: Atomic fast path + `parking_lot::Mutex` cold path — not actor.
9. **TLS**: Swap to `parking_lot::Mutex` — minimal change.
10. **Keep kameo optional**: only for future supervision/control-plane, not in hot paths.

This delivers the largest real-world latency and determinism gains with the lowest complexity and risk, while matching the proven `::hypercore::ws` shape where it matters.
