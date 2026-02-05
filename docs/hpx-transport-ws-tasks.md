# hpx-transport WebSocket Implementation Tasks

> Task breakdown for implementing the advanced WebSocket features described in [hpx-transport-ws-design.md](hpx-transport-ws-design.md)

## Overview

This document breaks down the WebSocket architecture implementation into concrete, actionable tasks organized by phases. Each task includes acceptance criteria and estimated complexity.

---

## Phase 1: Core Abstractions and Types

**Goal**: Define the foundational types and traits that all other components will use.

### Task 1.1: Define Core Type Aliases and Identifiers

**File**: `src/websocket/types.rs`

**Subtasks**:

- [x] Create `RequestId` struct with ULID-based generation
- [x] Create `Topic` struct for subscription routing
- [x] Implement `From<String>`, `From<&str>`, `Display` for both types
- [x] Add unit tests for ID generation uniqueness

**Acceptance Criteria**:

- `RequestId::new()` generates unique IDs
- Both types are `Clone`, `Debug`, `Hash`, `Eq`
- Serialization/deserialization works correctly

**Complexity**: Low

---

### Task 1.2: Define Message Classification Types

**File**: `src/websocket/types.rs`

**Subtasks**:

- [x] Create `MessageKind` enum with variants: `Response`, `Update`, `System`, `Control`, `Unknown`
- [x] Document each variant's purpose
- [x] Add helper methods for common checks

**Acceptance Criteria**:

- All message types are covered
- Enum is non-exhaustive for future extensibility

**Complexity**: Low

---

### Task 1.3: Define ProtocolHandler Trait

**File**: `src/websocket/protocol.rs`

**Subtasks**:

- [x] Define `ProtocolHandler` trait with all methods from design
- [x] Add associated types: `Request`, `Response`, `Error`
- [x] Provide default implementations where possible
- [x] Document each method with examples

**Methods to implement**:

```rust
// Connection lifecycle
fn on_connect(&self) -> Vec<Message>
fn build_auth_message(&self) -> Option<Message>
fn is_auth_success(&self, message: &str) -> bool

// Message classification
fn classify_message(&self, message: &str) -> MessageKind
fn extract_request_id(&self, message: &str) -> Option<RequestId>
fn extract_topic(&self, message: &str) -> Option<Topic>

// Message building
fn build_subscribe(&self, topics: &[Topic], request_id: RequestId) -> Message
fn build_unsubscribe(&self, topics: &[Topic], request_id: RequestId) -> Message
fn build_ping(&self) -> Option<Message>
fn build_pong(&self, ping_data: &[u8]) -> Option<Message>

// Message processing
fn decode_binary(&self, data: &[u8]) -> Result<String, Self::Error>
fn is_server_ping(&self, message: &str) -> bool
fn is_pong_response(&self, message: &str) -> bool
fn is_subscription_success(&self, message: &str, topics: &[Topic]) -> bool
fn should_reconnect(&self, message: &str) -> bool
```

**Acceptance Criteria**:

- Trait is object-safe where possible
- All methods have documentation
- Default implementations are sensible

**Complexity**: Medium

---

### Task 1.4: Create WebSocketConfig Struct

**File**: `src/websocket/config.rs`

**Subtasks**:

- [x] Define `WebSocketConfig` with all fields from design
- [x] Implement `Default` with sensible values
- [x] Add builder pattern methods for easy configuration
- [x] Add validation method to check config consistency
- [x] Document all fields

**Fields**:

```rust
// URL
url: String

// Reconnection
reconnect_initial_delay: Duration
reconnect_max_delay: Duration
reconnect_backoff_factor: f64
reconnect_max_attempts: Option<u32>
reconnect_jitter: f64

// Heartbeat
ping_interval: Duration
pong_timeout: Duration
use_websocket_ping: bool

// Request handling
request_timeout: Duration
max_pending_requests: usize
pending_cleanup_interval: Duration

// Channels
subscription_channel_capacity: usize
command_channel_capacity: usize

// Connection
connect_timeout: Duration
auth_on_connect: bool
max_message_size: usize
```

**Acceptance Criteria**:

- All fields have reasonable defaults
- Builder methods return `Self` for chaining
- Validation catches invalid configurations

**Complexity**: Low

---

### Task 1.5: Extend TransportError for WebSocket

**File**: `src/error.rs`

**Subtasks**:

- [x] Add WebSocket-specific error variants
- [x] Add helper constructors for each variant
- [x] Implement conversion from underlying error types

**New variants**:

```rust
RequestTimeout { duration: Duration, request_id: String }
SubscriptionFailed { topic: String, message: String }
MaxReconnectAttempts { attempts: u32 }
ProtocolError { message: String }
CapacityExceeded { message: String }
ConnectionClosed { reason: Option<String> }
```

**Acceptance Criteria**:

- Errors are descriptive and actionable
- All errors implement `std::error::Error`
- Preserves source error where applicable

**Complexity**: Low

---

### Task 1.6: Add Lock-Free Dependencies

**File**: `Cargo.toml`

**Subtasks**:

- [x] Add `scc` crate for lock-free concurrent HashMap
- [x] Add `ulid` crate for unique ID generation (RequestId)
- [x] Verify workspace inheritance is correct

**Dependencies to add**:

```toml
[dependencies]
scc = { workspace = true }  # Lock-free concurrent HashMap
ulid = { workspace = true } # Unique Lexicographically Sortable Identifier
```

**Root workspace Cargo.toml**:

```toml
[workspace.dependencies]
scc = "2"      # Lock-free concurrent data structures
ulid = "1"     # Unique ID generation
```

**Why scc?**

- Lock-free HashMap with wait-free reads
- Better performance under high contention than `RwLock<HashMap>`
- Built-in support for `entry`, `scan`, `retain` operations
- No priority inversion or lock contention issues

**Acceptance Criteria**:

- `scc::HashMap` can be used in code
- `ulid::Ulid` can be used for RequestId generation
- `cargo build` succeeds

**Complexity**: Low

---

## Phase 2: State Management Components (Lock-Free)

**Goal**: Implement lock-free stores that track pending requests and subscriptions using `scc::HashMap`.

### Task 2.1: Implement PendingRequestStore (Lock-Free)

**File**: `src/websocket/pending.rs`

**Subtasks**:

- [x] Create `PendingRequest` struct with `response_tx`, `created_at`, `timeout`
- [x] Create `PendingRequestStore` using `scc::HashMap` (lock-free)
- [x] Implement `add()` method - uses lock-free insert
- [x] Implement `resolve()` method - uses lock-free remove
- [x] Implement `cleanup_stale()` method - uses `scc::HashMap::retain`
- [x] Implement `cleanup_stale_with_notify()` - uses scan + remove pattern for timeout notification
- [x] Implement `has_capacity()` method - uses lock-free len()
- [x] Add unit tests for all operations including concurrent access

**API**:

```rust
impl PendingRequestStore {
    pub fn new(config: Arc<WebSocketConfig>) -> Self;
    pub fn add(&self, id: RequestId, timeout: Option<Duration>) 
        -> oneshot::Receiver<Result<String, TransportError>>;
    pub fn resolve(&self, id: &RequestId, response: Result<String, TransportError>) -> bool;
    pub fn cleanup_stale(&self);
    pub async fn cleanup_stale_with_notify(&self);
    pub fn has_capacity(&self) -> bool;
    pub fn len(&self) -> usize;
    pub fn clear(&self);
}
```

**Lock-Free Operations**:

- `insert()` - wait-free under most conditions
- `remove()` - lock-free removal with immediate value return
- `retain()` - lock-free iteration with atomic removal
- `scan()` - wait-free iteration for read-only access
- `len()` - wait-free count

**Acceptance Criteria**:

- Lock-free for concurrent access (no `RwLock` or `Mutex`)
- Uses `scc::HashMap` instead of `std::collections::HashMap`
- Cleanup correctly times out and removes stale requests
- Resolving non-existent ID returns false
- Capacity check works correctly
- Concurrent stress tests pass

**Complexity**: Medium

---

### Task 2.2: Implement SubscriptionStore (Lock-Free)

**File**: `src/websocket/subscription.rs`

**Subtasks**:

- [x] Create `SubscriptionStore` using `scc::HashMap` (lock-free)
- [x] Implement `subscribe()` - uses `scc::HashMap::entry` for atomic upsert
- [x] Implement `add_subscriber()` - uses lock-free get for existing topics
- [x] Implement `unsubscribe()` - uses `update` + `remove` pattern
- [x] Implement `publish()` - uses lock-free `get` operation
- [x] Implement `get_all_topics()` - uses `scc::HashMap::scan` for lock-free iteration
- [x] Implement `subscriber_count()` - uses lock-free get
- [x] Implement `clear()` - uses lock-free clear
- [x] Add unit tests for subscription lifecycle with concurrent access

**API**:

```rust
impl SubscriptionStore {
    pub fn new(config: Arc<WebSocketConfig>) -> Self;
    pub fn subscribe(&self, topic: Topic) -> broadcast::Receiver<Message>;
    pub fn add_subscriber(&self, topic: &Topic) -> Option<broadcast::Receiver<Message>>;
    pub fn unsubscribe(&self, topic: &Topic) -> bool;
    pub fn publish(&self, topic: &Topic, message: Message) -> bool;
    pub fn get_all_topics(&self) -> Vec<Topic>;
    pub fn subscriber_count(&self, topic: &Topic) -> usize;
    pub fn clear(&self);
}
```

**Lock-Free Operations**:

- `entry()` - atomic get-or-insert with `Entry` API
- `get()` - wait-free read access
- `update()` - atomic update-in-place
- `remove()` - lock-free removal
- `scan()` - wait-free iteration

**Acceptance Criteria**:

- Lock-free for concurrent access (no `RwLock` or `Mutex`)
- Uses `scc::HashMap` instead of `std::collections::HashMap`
- Multiple subscribers to same topic share channel
- Last unsubscribe removes topic entirely
- Resubscription returns topics even if subscribers temporarily disconnected
- Concurrent stress tests pass

**Complexity**: Medium

---

## Phase 3: Connection Actor

**Goal**: Implement the background actor that manages the WebSocket connection lifecycle.

### Task 3.1: Define Command Types

**File**: `src/websocket/actor.rs`

**Subtasks**:

- [x] Create `ActorCommand` enum with all command variants
- [x] Create `ConnectionState` enum for state machine
- [x] Document state transitions

**Types**:

```rust
pub enum ActorCommand {
    Subscribe {
        topics: Vec<Topic>,
        reply_tx: oneshot::Sender<TransportResult<()>>,
    },
    Unsubscribe {
        topics: Vec<Topic>,
    },
    Send {
        message: Message,
    },
    Request {
        message: Message,
        request_id: RequestId,
        reply_tx: oneshot::Sender<TransportResult<String>>,
        timeout: Option<Duration>,
    },
    Close,
}

pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Authenticating,
    Ready,
    Reconnecting { attempt: u32 },
    Closing,
    Closed,
}
```

**Acceptance Criteria**:

- All commands have appropriate reply channels where needed
- State enum covers all connection states

**Complexity**: Low

---

### Task 3.2: Implement ConnectionActor Structure

**File**: `src/websocket/actor.rs`

**Subtasks**:

- [x] Create `ConnectionActor<H: ProtocolHandler>` struct
- [x] Implement constructor with all required fields
- [x] Add state transition methods

**Fields**:

```rust
struct ConnectionActor<H: ProtocolHandler> {
    config: Arc<WebSocketConfig>,
    handler: H,
    cmd_rx: mpsc::Receiver<ActorCommand>,
    pending_requests: Arc<PendingRequestStore>,
    subscriptions: Arc<SubscriptionStore>,
    state: ConnectionState,
    write: Option<WebSocketWrite>,
    last_pong: Option<Instant>,
    reconnect_attempt: u32,
}
```

**Acceptance Criteria**:

- Actor can be constructed with all dependencies
- State is properly initialized

**Complexity**: Low

---

### Task 3.3: Implement Connection Management

**File**: `src/websocket/actor.rs`

**Subtasks**:

- [x] Implement `connect()` method using `hpx::ws`
- [x] Implement `disconnect()` method for clean shutdown
- [x] Implement `try_connect()` with timeout handling
- [x] Implement exponential backoff with jitter
- [x] Implement `should_give_up()` for max retries check
- [x] Add connection state change logging

**Methods**:

```rust
impl<H: ProtocolHandler> ConnectionActor<H> {
    async fn connect(&mut self) -> TransportResult<()>;
    async fn disconnect(&mut self);
    async fn try_connect(&mut self) -> bool;
    fn calculate_backoff(&self) -> Duration;
    async fn wait_before_reconnect(&mut self);
    fn should_give_up(&self) -> bool;
    fn initiate_reconnect(&mut self);
}
```

**Acceptance Criteria**:

- Connection uses configured timeout
- Backoff correctly increases with jitter
- Max attempts is respected
- State transitions are logged

**Complexity**: Medium

---

### Task 3.4: Implement Authentication Flow

**File**: `src/websocket/actor.rs`

**Subtasks**:

- [x] Implement `authenticate()` method
- [x] Handle auth success/failure from protocol handler
- [x] Set authenticated state on success
- [x] Retry authentication on failure (configurable)

**Methods**:

```rust
impl<H: ProtocolHandler> ConnectionActor<H> {
    async fn authenticate(&mut self) -> bool;
    fn is_authenticated(&self) -> bool;
}
```

**Acceptance Criteria**:

- Authentication uses protocol handler's auth message
- Success is determined by protocol handler
- Failure triggers reconnect

**Complexity**: Medium

---

### Task 3.5: Implement Heartbeat Management

**File**: `src/websocket/actor.rs`

**Subtasks**:

- [x] Implement ping sending (WebSocket frame or protocol-level)
- [x] Implement pong timeout detection
- [x] Track last pong time
- [x] Trigger reconnect on pong timeout

**Methods**:

```rust
impl<H: ProtocolHandler> ConnectionActor<H> {
    async fn send_ping(&mut self) -> TransportResult<()>;
    fn check_pong_timeout(&self) -> TransportResult<()>;
    fn update_last_pong(&mut self);
}
```

**Acceptance Criteria**:

- Ping sent at configured interval
- Pong timeout triggers reconnect
- Both WebSocket and protocol-level ping supported

**Complexity**: Medium

---

### Task 3.6: Implement Message Handling

**File**: `src/websocket/actor.rs`

**Subtasks**:

- [x] Implement `read_message()` from WebSocket
- [x] Implement `handle_message()` for routing
- [x] Route responses to pending requests
- [x] Route updates to subscriptions
- [x] Handle system messages (ping/pong)
- [x] Handle control messages (subscription confirmations)

**Methods**:

```rust
impl<H: ProtocolHandler> ConnectionActor<H> {
    async fn read_message(&mut self) -> TransportResult<Option<Message>>;
    async fn handle_message(&mut self, message: Message) -> TransportResult<()>;
    fn route_response(&self, message: &str);
    fn route_update(&self, message: Message);
}
```

**Acceptance Criteria**:

- Messages correctly classified using protocol handler
- Responses resolve pending requests
- Updates published to subscriptions
- Unknown messages logged but don't crash

**Complexity**: High

---

### Task 3.7: Implement Command Handling

**File**: `src/websocket/actor.rs`

**Subtasks**:

- [x] Implement handler for `Subscribe` command
- [x] Implement handler for `Unsubscribe` command
- [x] Implement handler for `Send` command
- [x] Implement handler for `Request` command
- [x] Implement handler for `Close` command
- [x] Queue commands during reconnection

**Methods**:

```rust
impl<H: ProtocolHandler> ConnectionActor<H> {
    async fn handle_command(&mut self, cmd: ActorCommand) -> TransportResult<()>;
    async fn handle_subscribe(&mut self, topics: Vec<Topic>, reply_tx: oneshot::Sender<TransportResult<()>>);
    async fn handle_unsubscribe(&mut self, topics: Vec<Topic>);
    async fn handle_send(&mut self, message: Message);
    async fn handle_request(&mut self, message: Message, request_id: RequestId, reply_tx: oneshot::Sender<TransportResult<String>>, timeout: Option<Duration>);
}
```

**Acceptance Criteria**:

- Subscribe sends protocol message and confirms
- Unsubscribe removes from store and sends message
- Request registers in pending store before sending
- Commands during reconnect are handled gracefully

**Complexity**: High

---

### Task 3.8: Implement Resubscription Logic

**File**: `src/websocket/actor.rs`

**Subtasks**:

- [x] Implement `resubscribe_all()` method
- [x] Get all topics from subscription store
- [x] Send subscribe messages in batches
- [x] Handle partial failures

**Methods**:

```rust
impl<H: ProtocolHandler> ConnectionActor<H> {
    async fn resubscribe_all(&mut self) -> TransportResult<()>;
}
```

**Acceptance Criteria**:

- All previously subscribed topics are resubscribed
- Batching respects protocol limits
- Partial failure doesn't break entire resubscription

**Complexity**: Medium

---

### Task 3.9: Implement Main Event Loop

**File**: `src/websocket/actor.rs`

**Subtasks**:

- [x] Implement `run()` as the actor's main entry point
- [x] Implement `run_ready_loop()` for the main event loop
- [x] Use `tokio::select!` for concurrent event handling
- [x] Handle state transitions correctly
- [x] Ensure clean shutdown on close

**Methods**:

```rust
impl<H: ProtocolHandler> ConnectionActor<H> {
    pub async fn run(mut self);
    async fn run_ready_loop(&mut self) -> TransportResult<()>;
}
```

**Acceptance Criteria**:

- Actor runs until closed or max retries exceeded
- All events (ping, messages, commands) handled concurrently
- Clean shutdown on Close command
- Reconnection works after errors

**Complexity**: High

---

## Phase 4: Client API

**Goal**: Implement the user-facing API that applications will use.

### Task 4.1: Implement WebSocketClient Structure

**File**: `src/websocket/client.rs`

**Subtasks**:

- [x] Create `WebSocketClient<H: ProtocolHandler>` struct
- [x] Implement `connect()` constructor that spawns actor
- [x] Implement `Clone` for cheap sharing
- [x] Add `is_connected()` health check

**API**:

```rust
#[derive(Clone)]
pub struct WebSocketClient<H: ProtocolHandler> {
    cmd_tx: mpsc::Sender<ActorCommand>,
    pending_requests: Arc<PendingRequestStore>,
    subscriptions: Arc<SubscriptionStore>,
    config: Arc<WebSocketConfig>,
    _marker: PhantomData<H>,
}

impl<H: ProtocolHandler> WebSocketClient<H> {
    pub async fn connect(config: WebSocketConfig, handler: H) -> TransportResult<Self>;
    pub fn is_connected(&self) -> bool;
}
```

**Acceptance Criteria**:

- Client is cheap to clone
- Actor is spawned on connect
- Connection status is accurate

**Complexity**: Medium

---

### Task 4.2: Implement Request-Response API

**File**: `src/websocket/client.rs`

**Subtasks**:

- [x] Implement `request<R, T>()` for typed request/response
- [x] Implement `request_with_timeout<R, T>()` for custom timeout
- [x] Implement `request_raw()` for raw message exchange
- [x] Add capacity checking before requests
- [x] Handle serialization/deserialization

**API**:

```rust
impl<H: ProtocolHandler> WebSocketClient<H> {
    pub async fn request<R: Serialize, T: DeserializeOwned>(&self, request: &R) -> TransportResult<T>;
    pub async fn request_with_timeout<R: Serialize, T: DeserializeOwned>(&self, request: &R, timeout: Option<Duration>) -> TransportResult<T>;
    pub async fn request_raw(&self, message: Message) -> TransportResult<String>;
}
```

**Acceptance Criteria**:

- Request-response correlates correctly via request ID
- Timeout is respected
- Capacity exceeded returns appropriate error
- Serialization errors are handled

**Complexity**: High

---

### Task 4.3: Implement Subscription API

**File**: `src/websocket/client.rs`

**Subtasks**:

- [x] Implement `subscribe()` for single topic
- [x] Implement `subscribe_many()` for multiple topics
- [x] Implement `unsubscribe()` for single topic
- [x] Return broadcast receivers for message streams

**API**:

```rust
impl<H: ProtocolHandler> WebSocketClient<H> {
    pub async fn subscribe(&self, topic: impl Into<Topic>) -> TransportResult<broadcast::Receiver<Message>>;
    pub async fn subscribe_many(&self, topics: impl IntoIterator<Item = impl Into<Topic>>) -> TransportResult<Vec<broadcast::Receiver<Message>>>;
    pub async fn unsubscribe(&self, topic: impl Into<Topic>) -> TransportResult<()>;
}
```

**Acceptance Criteria**:

- Subscription waits for confirmation
- Multiple subscribers to same topic work correctly
- Unsubscribe only sends message when last subscriber leaves

**Complexity**: Medium

---

### Task 4.4: Implement Low-Level API

**File**: `src/websocket/client.rs`

**Subtasks**:

- [x] Implement `send()` for raw message sending
- [x] Implement `send_json<T>()` for JSON messages
- [x] Implement `close()` for graceful shutdown

**API**:

```rust
impl<H: ProtocolHandler> WebSocketClient<H> {
    pub async fn send(&self, message: Message) -> TransportResult<()>;
    pub async fn send_json<T: Serialize>(&self, payload: &T) -> TransportResult<()>;
    pub async fn close(&self) -> TransportResult<()>;
}
```

**Acceptance Criteria**:

- Messages are sent without response expectation
- Close gracefully shuts down the connection
- Errors after close are handled gracefully

**Complexity**: Low

---

## Phase 5: Protocol Handler Implementations

**Goal**: Provide reference implementations of the ProtocolHandler trait.

### Task 5.1: Implement GenericJsonHandler

**File**: `src/websocket/handlers/generic_json.rs`

**Subtasks**:

- [x] Create handler for generic JSON-based protocols
- [x] Extract request_id from JSON field (configurable)
- [x] Extract topic from JSON field (configurable)
- [x] Build subscribe/unsubscribe messages

**Configuration**:

```rust
pub struct GenericJsonHandlerConfig {
    pub request_id_field: String,     // e.g., "id", "req_id"
    pub topic_field: String,          // e.g., "topic", "channel"
    pub subscribe_op: String,         // e.g., "subscribe"
    pub unsubscribe_op: String,       // e.g., "unsubscribe"
    pub ping_op: Option<String>,      // e.g., "ping"
    pub pong_op: Option<String>,      // e.g., "pong"
}
```

**Acceptance Criteria**:

- Works with common JSON WebSocket APIs
- Configurable field names
- Optional protocol-level ping/pong

**Complexity**: Medium

---

### Task 5.2: Implement JsonRpcHandler

**File**: `src/websocket/handlers/jsonrpc.rs`

**Subtasks**:

- [x] Implement JSON-RPC 2.0 protocol handler
- [x] Handle `id` field for request-response
- [x] Handle `method` field for notifications
- [x] Build proper JSON-RPC request format

**Acceptance Criteria**:

- Compliant with JSON-RPC 2.0 spec
- Handles both requests and notifications
- Error responses are properly typed

**Complexity**: Medium

---

### Task 5.3: Migrate Existing ExchangeHandler

**File**: `src/websocket/handlers/legacy.rs`

**Subtasks**:

- [x] Create adapter from `ExchangeHandler` to `ProtocolHandler`
- [x] Maintain backward compatibility
- [x] Deprecate old trait in favor of new

**Acceptance Criteria**:

- Existing code continues to work
- Clear migration path documented
- Deprecation warnings where appropriate

**Complexity**: Medium

---

## Phase 6: Testing

**Goal**: Comprehensive testing of all components.

### Task 6.1: Create Mock WebSocket Server

**File**: `tests/mock_server.rs`

**Subtasks**:

- [x] Create `MockWebSocketServer` for testing
- [x] Support configurable response delays
- [x] Support connection drops for reconnect testing
- [x] Support auth simulation

**Acceptance Criteria**:

- Server can simulate various scenarios
- Easy to configure for different test cases
- Proper cleanup after tests

**Complexity**: High

---

### Task 6.2: Implement Unit Tests for Stores

**File**: `tests/stores_test.rs`

**Subtasks**:

- [x] Test `PendingRequestStore` add/resolve/cleanup
- [x] Test `SubscriptionStore` subscribe/unsubscribe/publish
- [x] Test concurrent access patterns
- [x] Test edge cases (empty stores, capacity limits)

**Acceptance Criteria**:

- All store operations tested
- Concurrent access is safe
- Edge cases handled correctly

**Complexity**: Medium

---

### Task 6.3: Implement Integration Tests

**File**: `tests/websocket_integration.rs`

**Subtasks**:

- [x] Test request-response flow
- [x] Test subscription flow
- [x] Test reconnection after disconnect
- [x] Test authentication flow
- [x] Test timeout handling

**Acceptance Criteria**:

- Full flows tested end-to-end
- Reconnection properly restores subscriptions
- Timeouts work correctly

**Complexity**: High

---

### Task 6.4: Implement Stress Tests

**File**: `tests/websocket_stress.rs`

**Subtasks**:

- [x] Test high message throughput
- [x] Test many concurrent subscriptions
- [x] Test many concurrent requests
- [x] Test rapid connect/disconnect cycles

**Acceptance Criteria**:

- No memory leaks under load
- Performance meets requirements
- System remains stable

**Complexity**: Medium

---

## Phase 7: Documentation and Examples

**Goal**: Comprehensive documentation and working examples.

### Task 7.1: Add Module Documentation

**Subtasks**:

- [x] Document `websocket` module overview
- [x] Document each public type and trait
- [x] Add usage examples in doc comments
- [x] Document configuration options

**Acceptance Criteria**:

- All public APIs documented
- Examples compile and run
- Configuration explained clearly

**Complexity**: Medium

---

### Task 7.2: Create Request-Response Example

**File**: `examples/ws_request_response.rs`

**Subtasks**:

- [x] Create example showing request-response pattern
- [x] Show typed request/response usage
- [x] Show timeout handling
- [x] Show error handling

**Acceptance Criteria**:

- Example compiles and runs
- Demonstrates key patterns
- Well commented

**Complexity**: Low

---

### Task 7.3: Create Subscription Example

**File**: `examples/ws_subscription.rs`

**Subtasks**:

- [x] Create example showing subscription pattern
- [x] Show subscribing to multiple topics
- [x] Show handling reconnection
- [x] Show graceful shutdown

**Acceptance Criteria**:

- Example compiles and runs
- Demonstrates subscription lifecycle
- Well commented

**Complexity**: Low

---

### Task 7.4: Create Custom Handler Example

**File**: `examples/ws_custom_handler.rs`

**Subtasks**:

- [x] Create example implementing ProtocolHandler
- [x] Show all required methods
- [x] Demonstrate exchange integration pattern

**Acceptance Criteria**:

- Example compiles and runs
- Shows complete handler implementation
- Well documented

**Complexity**: Medium

---

## Phase 8: Final Integration

**Goal**: Integrate all components and prepare for release.

### Task 8.1: Update lib.rs Exports

**File**: `src/lib.rs`

**Subtasks**:

- [x] Export all new public types
- [x] Organize module structure
- [x] Re-export commonly used types at crate root
- [x] Add feature flags if needed

**Acceptance Criteria**:

- Clean public API surface
- No unnecessary public items
- Feature flags documented

**Complexity**: Low

---

### Task 8.2: Final Code Review and Cleanup

**Subtasks**:

- [x] Review all code for consistency
- [x] Run `just format`
- [x] Run `just lint` and fix all warnings
- [x] Run `just test` and ensure all pass
- [x] Check for any TODO comments
- [x] Remove any debug code

**Acceptance Criteria**:

- All checks pass
- No warnings
- Code is production ready

**Complexity**: Medium

---

### Task 8.3: Update README and Changelog

**Subtasks**:

- [x] Update crate README with WebSocket features
- [x] Add changelog entry for new features
- [x] Update crate description if needed

**Acceptance Criteria**:

- Documentation is current
- Changes are documented

**Complexity**: Low

---

## Task Dependencies Graph

```text
Phase 1 (Types & Traits)
    │
    ├── 1.1 Core Types
    ├── 1.2 Message Classification
    ├── 1.3 ProtocolHandler Trait
    ├── 1.4 WebSocketConfig
    └── 1.5 Error Types
            │
            ▼
Phase 2 (State Management)
    │
    ├── 2.1 PendingRequestStore ──────┐
    └── 2.2 SubscriptionStore ────────┼──┐
                                      │  │
                                      ▼  ▼
Phase 3 (Connection Actor) ◄──────────┴──┘
    │
    ├── 3.1 Command Types
    ├── 3.2 Actor Structure
    ├── 3.3 Connection Management
    ├── 3.4 Authentication
    ├── 3.5 Heartbeat
    ├── 3.6 Message Handling
    ├── 3.7 Command Handling
    ├── 3.8 Resubscription
    └── 3.9 Event Loop
            │
            ▼
Phase 4 (Client API)
    │
    ├── 4.1 Client Structure
    ├── 4.2 Request-Response API
    ├── 4.3 Subscription API
    └── 4.4 Low-Level API
            │
            ├─────────────────────────┐
            ▼                         ▼
Phase 5 (Handlers)          Phase 6 (Testing)
    │                              │
    ├── 5.1 GenericJsonHandler     ├── 6.1 Mock Server
    ├── 5.2 JsonRpcHandler         ├── 6.2 Unit Tests
    └── 5.3 Legacy Migration       ├── 6.3 Integration Tests
                                   └── 6.4 Stress Tests
                                          │
                                          ▼
                                   Phase 7 (Documentation)
                                          │
                                          ├── 7.1 Module Docs
                                          ├── 7.2 Request-Response Example
                                          ├── 7.3 Subscription Example
                                          └── 7.4 Custom Handler Example
                                                  │
                                                  ▼
                                           Phase 8 (Integration)
                                                  │
                                                  ├── 8.1 Exports
                                                  ├── 8.2 Final Review
                                                  └── 8.3 README Update
```

## Estimated Timeline

| Phase | Tasks | Estimated Duration |
|-------|-------|-------------------|
| Phase 1 | 5 tasks | 1-2 days |
| Phase 2 | 2 tasks | 1 day |
| Phase 3 | 9 tasks | 3-4 days |
| Phase 4 | 4 tasks | 2 days |
| Phase 5 | 3 tasks | 1-2 days |
| Phase 6 | 4 tasks | 2-3 days |
| Phase 7 | 4 tasks | 1 day |
| Phase 8 | 3 tasks | 1 day |

**Total Estimated Duration**: 12-16 days

## Risk Factors

1. **Complexity of Actor Pattern**: The connection actor has many responsibilities. Consider breaking into sub-actors if complexity grows.

2. **Protocol Handler Abstraction**: The trait must be generic enough for various exchanges while being practical to implement.

3. **Testing Async Code**: Creating reliable tests for async WebSocket behavior is challenging.

4. **Backward Compatibility**: Migrating from existing `ExchangeHandler` must not break existing users.

## Success Criteria

- [x] All unit tests pass
- [x] All integration tests pass
- [x] `just format`, `just lint`, `just test` all pass
- [x] Documentation coverage is complete
- [x] Examples run successfully
- [x] Backward compatibility maintained
- [x] Performance benchmarks meet requirements
