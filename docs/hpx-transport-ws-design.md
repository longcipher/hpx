# hpx-transport WebSocket Architecture Design

> Design for a robust, exchange-agnostic WebSocket client supporting dual interaction patterns (Promise-driven and Event-driven) with automatic reconnection and subscription management.

## 1. Executive Summary

This document describes the design for extending `hpx-transport` with advanced WebSocket capabilities. The design draws inspiration from the bybit-api Node.js library while addressing its limitations and leveraging Rust's type system and async runtime for improved safety and performance.

### Key Features

1. **Dual Interaction Patterns**: Support both request-response (Promise-like) and event-driven patterns
2. **Smart Connection Management**: Automatic reconnection with exponential backoff, heartbeat monitoring, and silent disconnect detection
3. **Subscription Persistence**: Automatic resubscription after reconnection with subscription state tracking
4. **Exchange-Agnostic Design**: Protocol-level abstractions that work with any WebSocket-based API
5. **Type-Safe API**: Leverage Rust's type system for compile-time safety
6. **Zero-Copy Message Handling**: Efficient message routing without unnecessary allocations
7. **Lock-Free Concurrency**: Use `scc::HashMap` for lock-free state management with wait-free reads

## 2. Design Analysis

### 2.1 Reference Design (bybit-api) Strengths

The bybit-api library implements several valuable patterns:

1. **Smart WebSocket Persistence**
   - Heartbeat mechanism with silent disconnect detection
   - Automatic reconnection with authentication replay
   - Topic resubscription after reconnection
   - 24-hour forced disconnect handling

2. **Dual Pattern Interaction**
   - Promise-driven (request-response): Treats WebSocket like HTTP
   - Event-driven: Traditional pub/sub pattern with EventEmitter

3. **Request-Response Tracking**
   - Uses `req_id` for request correlation
   - Stores pending promises in a Map for resolution

### 2.2 Reference Design Limitations

1. **Node.js Single-threaded Model**: Relies on shared mutable state, not suitable for Rust
2. **Exchange-Specific Coupling**: Tightly coupled to Bybit's protocol specifics
3. **No Timeout Handling**: Request-response pattern lacks individual request timeouts
4. **Limited Backpressure**: No mechanism to handle slow consumers
5. **Memory Leaks**: Pending request maps can grow unbounded if responses never arrive

### 2.3 Design Improvements for hpx-transport

1. **Actor Model**: Use channels for thread-safe communication instead of shared state
2. **Lock-Free State Management**: Use `scc::HashMap` for lock-free concurrent access to pending requests and subscriptions
3. **Generic Protocol Traits**: Define abstractions that any exchange can implement
4. **Request Timeouts**: Per-request timeout with automatic cleanup
5. **Backpressure Handling**: Use bounded channels with configurable capacity
6. **Resource Cleanup**: Automatic cleanup of stale pending requests

## 3. Architecture Overview

### 3.1 High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                          User Application                                │
│                                                                          │
│   ┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐   │
│   │  RequestClient  │     │SubscriptionClient│     │  RawClient     │   │
│   │  (Req-Response) │     │ (Event-Driven)   │     │  (Low-level)   │   │
│   └────────┬────────┘     └────────┬─────────┘     └────────┬───────┘   │
└────────────┼───────────────────────┼───────────────────────┼────────────┘
             │                       │                       │
             ▼                       ▼                       ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         WebSocketManager                                 │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │                         Connection Pool                          │    │
│  │  ┌────────────────┐  ┌────────────────┐  ┌────────────────┐     │    │
│  │  │ Connection #1  │  │ Connection #2  │  │ Connection #N  │     │    │
│  │  │  (wsKey: pub)  │  │  (wsKey: priv) │  │  (wsKey: ...)  │     │    │
│  │  └────────────────┘  └────────────────┘  └────────────────┘     │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
│  ┌─────────────────────┐  ┌─────────────────────┐                       │
│  │  Pending Requests   │  │  Subscription Store │                       │
│  │  scc::HashMap<ReqId,│  │  scc::HashMap<Topic,│                       │
│  │    OneshotSender>   │  │    BroadcastSender> │                       │
│  └─────────────────────┘  └─────────────────────┘                       │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         Connection Actor                                 │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │  Event Loop (tokio::select!)                                     │    │
│  │                                                                   │    │
│  │  1. Heartbeat Timer ─────> Send Ping, Check Pong Timeout         │    │
│  │  2. WebSocket Read  ─────> Route to Pending/Subscriptions        │    │
│  │  3. Command Channel ─────> Subscribe/Unsubscribe/Send/Close      │    │
│  │  4. Cleanup Timer   ─────> Remove stale pending requests         │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │  Reconnection Logic                                               │    │
│  │  - Exponential backoff with jitter                                │    │
│  │  - Max retry attempts (configurable)                              │    │
│  │  - Authentication replay                                          │    │
│  │  - Subscription restoration                                       │    │
│  └─────────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                              hpx::ws                                     │
│                    (Low-level WebSocket primitives)                      │
└─────────────────────────────────────────────────────────────────────────┘
```

### 3.2 Component Responsibilities

| Component | Responsibility |
|-----------|----------------|
| `WebSocketManager` | High-level API, connection pool management, request routing |
| `ConnectionActor` | Single connection lifecycle, heartbeat, reconnection |
| `RequestClient` | Request-response pattern API (async/await) |
| `SubscriptionClient` | Pub/Sub pattern API (streams/channels) |
| `ProtocolHandler` | Exchange-specific message encoding/decoding |
| `PendingRequestStore` | Track in-flight request-response pairs |
| `SubscriptionStore` | Track active subscriptions for restoration |

## 4. Core Abstractions

### 4.1 Protocol Handler Trait

The `ProtocolHandler` trait abstracts exchange-specific protocol details:

```rust
/// Protocol handler for exchange-specific WebSocket logic.
///
/// This trait defines the contract for handling protocol-level operations
/// that vary between different WebSocket APIs (exchanges, services, etc.)
pub trait ProtocolHandler: Send + Sync + 'static {
    /// The request type sent to the server
    type Request: Serialize + Send;
    
    /// The response type received from the server
    type Response: DeserializeOwned + Send;
    
    /// Error type for protocol-level errors
    type Error: std::error::Error + Send + Sync;

    // === Connection Lifecycle ===
    
    /// Messages to send immediately after connection establishment.
    /// Typically used for authentication.
    fn on_connect(&self) -> Vec<Message> {
        vec![]
    }
    
    /// Called when authentication is required.
    /// Returns the authentication message to send.
    fn build_auth_message(&self) -> Option<Message> {
        None
    }
    
    /// Verify if an authentication response indicates success.
    fn is_auth_success(&self, message: &str) -> bool {
        false
    }

    // === Message Classification ===
    
    /// Determine the message kind for routing purposes.
    fn classify_message(&self, message: &str) -> MessageKind;

    /// Extract request ID from a response message for request-response correlation.
    fn extract_request_id(&self, message: &str) -> Option<RequestId>;
    
    /// Extract topic from a subscription message for pub/sub routing.
    fn extract_topic(&self, message: &str) -> Option<Topic>;

    // === Message Building ===
    
    /// Build a subscribe message for the given topics.
    fn build_subscribe(&self, topics: &[Topic], request_id: RequestId) -> Message;
    
    /// Build an unsubscribe message for the given topics.
    fn build_unsubscribe(&self, topics: &[Topic], request_id: RequestId) -> Message;
    
    /// Build a ping message (protocol-level, not WebSocket ping frame).
    fn build_ping(&self) -> Option<Message> {
        None
    }
    
    /// Build a pong message (protocol-level response to server ping).
    fn build_pong(&self, ping_data: &[u8]) -> Option<Message> {
        None
    }

    // === Message Processing ===
    
    /// Decode binary message (e.g., decompress gzip/deflate).
    fn decode_binary(&self, data: &[u8]) -> Result<String, Self::Error> {
        String::from_utf8(data.to_vec())
            .map_err(|e| /* protocol error */)
    }
    
    /// Check if this is a server ping message requiring a pong response.
    fn is_server_ping(&self, message: &str) -> bool {
        false
    }
    
    /// Check if this is a pong response to our ping.
    fn is_pong_response(&self, message: &str) -> bool {
        false
    }
    
    /// Check if the message indicates a successful subscription.
    fn is_subscription_success(&self, message: &str, topics: &[Topic]) -> bool;
    
    /// Check if the message indicates the connection should be closed and reconnected.
    fn should_reconnect(&self, message: &str) -> bool {
        false
    }
}

/// Classification of incoming messages for routing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageKind {
    /// Response to a specific request (has request_id)
    Response,
    /// Subscription data update (has topic)
    Update,
    /// System message (ping, pong, auth response, etc.)
    System,
    /// Control message (subscription confirmation, error, etc.)
    Control,
    /// Unknown or unparseable message
    Unknown,
}
```

### 4.2 Connection Configuration

```rust
/// WebSocket connection configuration.
#[derive(Debug, Clone)]
pub struct WebSocketConfig {
    /// WebSocket URL
    pub url: String,
    
    // === Reconnection ===
    /// Initial delay before first reconnection attempt
    pub reconnect_initial_delay: Duration,
    /// Maximum delay between reconnection attempts
    pub reconnect_max_delay: Duration,
    /// Backoff multiplier for reconnection delay
    pub reconnect_backoff_factor: f64,
    /// Maximum number of reconnection attempts (None = infinite)
    pub reconnect_max_attempts: Option<u32>,
    /// Add random jitter to reconnection delay (0.0 - 1.0)
    pub reconnect_jitter: f64,
    
    // === Heartbeat ===
    /// Interval between ping messages
    pub ping_interval: Duration,
    /// Timeout for pong response (triggers reconnect if exceeded)
    pub pong_timeout: Duration,
    /// Use WebSocket ping frames (true) or protocol-level ping (false)
    pub use_websocket_ping: bool,
    
    // === Request Handling ===
    /// Default timeout for request-response operations
    pub request_timeout: Duration,
    /// Maximum number of pending requests
    pub max_pending_requests: usize,
    /// Cleanup interval for stale pending requests
    pub pending_cleanup_interval: Duration,
    
    // === Channels ===
    /// Capacity for subscription broadcast channels
    pub subscription_channel_capacity: usize,
    /// Capacity for command channel
    pub command_channel_capacity: usize,
    
    // === Connection ===
    /// Connection timeout
    pub connect_timeout: Duration,
    /// Whether to authenticate immediately after connection
    pub auth_on_connect: bool,
    /// Maximum message size (for DoS protection)
    pub max_message_size: usize,
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            
            // Reconnection with exponential backoff
            reconnect_initial_delay: Duration::from_millis(100),
            reconnect_max_delay: Duration::from_secs(30),
            reconnect_backoff_factor: 2.0,
            reconnect_max_attempts: None, // Infinite retries by default
            reconnect_jitter: 0.1,
            
            // Heartbeat
            ping_interval: Duration::from_secs(30),
            pong_timeout: Duration::from_secs(10),
            use_websocket_ping: true,
            
            // Request handling
            request_timeout: Duration::from_secs(30),
            max_pending_requests: 1000,
            pending_cleanup_interval: Duration::from_secs(60),
            
            // Channels
            subscription_channel_capacity: 1024,
            command_channel_capacity: 256,
            
            // Connection
            connect_timeout: Duration::from_secs(10),
            auth_on_connect: true,
            max_message_size: 16 * 1024 * 1024, // 16 MB
        }
    }
}
```

### 4.3 Request-Response Types

```rust
/// Unique identifier for request-response correlation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestId(pub String);

impl RequestId {
    pub fn new() -> Self {
        Self(ulid::Ulid::new().to_string())
    }
    
    pub fn from_string(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

/// Topic identifier for pub/sub routing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Topic(pub String);

impl Topic {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

/// A pending request awaiting response.
struct PendingRequest {
    /// Channel to send the response
    response_tx: oneshot::Sender<Result<String, TransportError>>,
    /// When this request was created
    created_at: Instant,
    /// Request-specific timeout (overrides default)
    timeout: Duration,
}

/// Store for pending request-response pairs.
/// 
/// Uses `scc::HashMap` for lock-free concurrent access, providing
/// better performance under high contention compared to `RwLock<HashMap>`.
pub struct PendingRequestStore {
    requests: scc::HashMap<RequestId, PendingRequest>,
    config: Arc<WebSocketConfig>,
}

impl PendingRequestStore {
    /// Create a new pending request store.
    pub fn new(config: Arc<WebSocketConfig>) -> Self {
        Self {
            requests: scc::HashMap::new(),
            config,
        }
    }
    
    /// Add a pending request and return a receiver for the response.
    pub fn add(&self, id: RequestId, timeout: Option<Duration>) 
        -> oneshot::Receiver<Result<String, TransportError>> 
    {
        let (tx, rx) = oneshot::channel();
        let timeout = timeout.unwrap_or(self.config.request_timeout);
        
        let pending = PendingRequest {
            response_tx: tx,
            created_at: Instant::now(),
            timeout,
        };
        
        // scc::HashMap::insert is lock-free
        let _ = self.requests.insert(id, pending);
        rx
    }
    
    /// Resolve a pending request with a response.
    pub fn resolve(&self, id: &RequestId, response: Result<String, TransportError>) -> bool {
        // scc::HashMap::remove is lock-free and returns Option<(K, V)>
        if let Some((_, pending)) = self.requests.remove(id) {
            let _ = pending.response_tx.send(response);
            true
        } else {
            false
        }
    }
    
    /// Clean up timed-out requests.
    /// 
    /// Uses `scc::HashMap::retain` which provides lock-free iteration
    /// with atomic removal of entries that don't satisfy the predicate.
    pub fn cleanup_stale(&self) {
        let now = Instant::now();
        
        self.requests.retain(|_id, pending| {
            if now.duration_since(pending.created_at) > pending.timeout {
                // Note: We cannot send on the channel here because retain
                // gives us a shared reference. Use scan + remove pattern instead.
                false
            } else {
                true
            }
        });
    }
    
    /// Clean up timed-out requests with timeout notification.
    /// 
    /// This variant sends timeout errors to waiting receivers.
    pub async fn cleanup_stale_with_notify(&self) {
        let now = Instant::now();
        let mut timed_out = Vec::new();
        
        // First pass: identify timed-out requests
        self.requests.scan(|id, pending| {
            if now.duration_since(pending.created_at) > pending.timeout {
                timed_out.push(id.clone());
            }
        });
        
        // Second pass: remove and notify
        for id in timed_out {
            if let Some((_, pending)) = self.requests.remove(&id) {
                let _ = pending.response_tx.send(Err(TransportError::Timeout {
                    duration: pending.timeout,
                }));
            }
        }
    }
    
    /// Check if the store has capacity for more requests.
    pub fn has_capacity(&self) -> bool {
        self.requests.len() < self.config.max_pending_requests
    }
}
```

### 4.4 Subscription Store

```rust
/// Store for active subscriptions.
/// 
/// Uses `scc::HashMap` for lock-free concurrent access, providing
/// better performance under high contention compared to `RwLock<HashMap>`.
pub struct SubscriptionStore {
    /// Active subscriptions: Topic -> (BroadcastSender, subscriber_count)
    subscriptions: scc::HashMap<Topic, (broadcast::Sender<Message>, usize)>,
    config: Arc<WebSocketConfig>,
}

impl SubscriptionStore {
    /// Create a new subscription store.
    pub fn new(config: Arc<WebSocketConfig>) -> Self {
        Self {
            subscriptions: scc::HashMap::new(),
            config,
        }
    }
    
    /// Subscribe to a topic. Returns a receiver for messages.
    /// 
    /// Uses `scc::HashMap::entry` for atomic upsert operation.
    pub fn subscribe(&self, topic: Topic) -> broadcast::Receiver<Message> {
        // Use entry API for atomic get-or-insert
        let capacity = self.config.subscription_channel_capacity;
        
        // Try to get existing entry first
        if let Some(entry) = self.subscriptions.get(&topic) {
            let (sender, _) = entry.get();
            return sender.subscribe();
        }
        
        // Create new subscription - use entry for atomic insert
        let (tx, rx) = broadcast::channel(capacity);
        match self.subscriptions.entry(topic) {
            scc::hash_map::Entry::Occupied(entry) => {
                // Another thread inserted, use their sender
                let (sender, _) = entry.get();
                sender.subscribe()
            }
            scc::hash_map::Entry::Vacant(entry) => {
                entry.insert_entry((tx, 1));
                rx
            }
        }
    }
    
    /// Increment subscriber count for existing topic.
    pub fn add_subscriber(&self, topic: &Topic) -> Option<broadcast::Receiver<Message>> {
        self.subscriptions.get(topic).map(|entry| {
            // Note: scc provides interior mutability through entry
            entry.get().0.subscribe()
        })
    }
    
    /// Unsubscribe from a topic. Returns true if this was the last subscriber.
    /// 
    /// Uses lock-free operations for concurrent safety.
    pub fn unsubscribe(&self, topic: &Topic) -> bool {
        // Use update_if to atomically decrement and check
        let mut should_remove = false;
        
        self.subscriptions.update(topic, |_, (sender, count)| {
            if *count > 1 {
                *count -= 1;
                (sender.clone(), *count)
            } else {
                should_remove = true;
                (sender.clone(), 0)
            }
        });
        
        if should_remove {
            self.subscriptions.remove(topic);
            true
        } else {
            false
        }
    }
    
    /// Publish a message to a topic.
    /// 
    /// Lock-free read operation using `scc::HashMap::get`.
    pub fn publish(&self, topic: &Topic, message: Message) -> bool {
        if let Some(entry) = self.subscriptions.get(topic) {
            let (sender, _) = entry.get();
            sender.send(message).is_ok()
        } else {
            false
        }
    }
    
    /// Get all currently subscribed topics (for resubscription after reconnect).
    /// 
    /// Uses `scc::HashMap::scan` for lock-free iteration.
    pub fn get_all_topics(&self) -> Vec<Topic> {
        let mut topics = Vec::new();
        self.subscriptions.scan(|topic, _| {
            topics.push(topic.clone());
        });
        topics
    }
    
    /// Get the current subscriber count for a topic.
    pub fn subscriber_count(&self, topic: &Topic) -> usize {
        self.subscriptions
            .get(topic)
            .map(|entry| entry.get().1)
            .unwrap_or(0)
    }
    
    /// Clear all subscriptions.
    pub fn clear(&self) {
        self.subscriptions.clear();
    }
}
```

## 5. Connection Actor

### 5.1 Actor State Machine

```
┌─────────────────────────────────────────────────────────────────────────┐
│                       Connection State Machine                           │
│                                                                          │
│  ┌────────────┐                                                          │
│  │Disconnected│◄────────────────────────────────────────────┐            │
│  └─────┬──────┘                                             │            │
│        │ connect()                                          │            │
│        ▼                                                    │            │
│  ┌────────────┐     timeout/error     ┌─────────────┐       │            │
│  │ Connecting ├──────────────────────►│ Reconnecting│───────┤            │
│  └─────┬──────┘                       └──────┬──────┘       │            │
│        │ connected                           │ connected    │            │
│        ▼                                     ▼              │            │
│  ┌────────────┐     auth_required     ┌─────────────┐       │            │
│  │ Connected  ├──────────────────────►│Authenticating│      │            │
│  └─────┬──────┘                       └──────┬──────┘       │            │
│        │ !auth_required                      │ auth_success │            │
│        │                                     │              │            │
│        ▼                                     ▼              │            │
│  ┌────────────────────────────────────────────┐            │            │
│  │                   Ready                    │            │            │
│  │  - Heartbeat running                       │            │            │
│  │  - Accepting commands                      │            │            │
│  │  - Processing messages                     │            │            │
│  └──────────────────────┬─────────────────────┘            │            │
│                         │                                   │            │
│         error/timeout/close                                 │            │
│                         │                                   │            │
│                         ▼                                   │            │
│                  ┌─────────────┐                            │            │
│                  │ Reconnecting│────────────────────────────┘            │
│                  └─────────────┘                                         │
│                         │                                                │
│         max_attempts_exceeded / close()                                  │
│                         │                                                │
│                         ▼                                                │
│                  ┌────────────┐                                          │
│                  │   Closed   │                                          │
│                  └────────────┘                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

### 5.2 Actor Implementation

```rust
/// Commands sent to the connection actor.
pub enum ActorCommand {
    /// Subscribe to topics
    Subscribe {
        topics: Vec<Topic>,
        reply_tx: oneshot::Sender<TransportResult<()>>,
    },
    /// Unsubscribe from topics
    Unsubscribe {
        topics: Vec<Topic>,
    },
    /// Send a raw message
    Send {
        message: Message,
    },
    /// Send a request and await response
    Request {
        message: Message,
        request_id: RequestId,
        reply_tx: oneshot::Sender<TransportResult<String>>,
        timeout: Option<Duration>,
    },
    /// Close the connection
    Close,
}

/// Connection state
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// The connection actor managing a single WebSocket connection.
pub struct ConnectionActor<H: ProtocolHandler> {
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

impl<H: ProtocolHandler> ConnectionActor<H> {
    /// Main event loop
    pub async fn run(mut self) {
        loop {
            match self.state {
                ConnectionState::Disconnected | ConnectionState::Reconnecting { .. } => {
                    if !self.try_connect().await {
                        if self.should_give_up() {
                            self.state = ConnectionState::Closed;
                            break;
                        }
                        self.wait_before_reconnect().await;
                    }
                }
                ConnectionState::Connected => {
                    if self.config.auth_on_connect {
                        self.state = ConnectionState::Authenticating;
                        if !self.authenticate().await {
                            self.initiate_reconnect();
                            continue;
                        }
                    }
                    self.state = ConnectionState::Ready;
                    self.resubscribe_all().await;
                }
                ConnectionState::Ready => {
                    if let Err(e) = self.run_ready_loop().await {
                        tracing::error!("Connection error: {}", e);
                        self.initiate_reconnect();
                    }
                }
                ConnectionState::Closing | ConnectionState::Closed => {
                    break;
                }
                _ => {}
            }
        }
    }
    
    async fn run_ready_loop(&mut self) -> TransportResult<()> {
        let mut ping_timer = tokio::time::interval(self.config.ping_interval);
        let mut cleanup_timer = tokio::time::interval(self.config.pending_cleanup_interval);
        
        ping_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        cleanup_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        
        loop {
            tokio::select! {
                // Heartbeat
                _ = ping_timer.tick() => {
                    self.send_ping().await?;
                    self.check_pong_timeout()?;
                }
                
                // Cleanup stale requests
                _ = cleanup_timer.tick() => {
                    self.pending_requests.cleanup_stale();
                }
                
                // Incoming WebSocket message
                msg = self.read_message() => {
                    match msg {
                        Ok(Some(msg)) => self.handle_message(msg).await?,
                        Ok(None) => return Ok(()), // Clean close
                        Err(e) => return Err(e),
                    }
                }
                
                // User commands
                Some(cmd) = self.cmd_rx.recv() => {
                    match cmd {
                        ActorCommand::Close => {
                            self.state = ConnectionState::Closing;
                            return Ok(());
                        }
                        cmd => self.handle_command(cmd).await?,
                    }
                }
            }
        }
    }
    
    async fn handle_message(&mut self, message: Message) -> TransportResult<()> {
        // Handle WebSocket-level pong
        if let Message::Pong(_) = &message {
            self.last_pong = Some(Instant::now());
            return Ok(());
        }
        
        // Handle WebSocket-level ping
        if let Message::Ping(data) = &message {
            self.send_message(Message::Pong(data.clone())).await?;
            return Ok(());
        }
        
        // Handle text/binary messages
        let text = match &message {
            Message::Text(t) => t.to_string(),
            Message::Binary(b) => self.handler.decode_binary(b)?,
            Message::Close(_) => return Ok(()),
            _ => return Ok(()),
        };
        
        // Check for server-level ping
        if self.handler.is_server_ping(&text) {
            if let Some(pong) = self.handler.build_pong(text.as_bytes()) {
                self.send_message(pong).await?;
            }
            return Ok(());
        }
        
        // Check for pong response
        if self.handler.is_pong_response(&text) {
            self.last_pong = Some(Instant::now());
            return Ok(());
        }
        
        // Classify and route the message
        match self.handler.classify_message(&text) {
            MessageKind::Response => {
                if let Some(id) = self.handler.extract_request_id(&text) {
                    self.pending_requests.resolve(&id, Ok(text));
                }
            }
            MessageKind::Update => {
                if let Some(topic) = self.handler.extract_topic(&text) {
                    self.subscriptions.publish(&topic, message);
                }
            }
            MessageKind::Control => {
                // Handle subscription confirmations, errors, etc.
            }
            MessageKind::System => {
                // Handle auth responses, etc.
            }
            MessageKind::Unknown => {
                tracing::debug!("Received unknown message type: {}", text);
            }
        }
        
        Ok(())
    }
}
```

## 6. Client API

### 6.1 WebSocket Handle (Main Entry Point)

```rust
/// The main WebSocket client handle.
///
/// This is the user-facing API for interacting with the WebSocket connection.
/// It is cheap to clone and can be shared across tasks.
#[derive(Clone)]
pub struct WebSocketClient<H: ProtocolHandler> {
    cmd_tx: mpsc::Sender<ActorCommand>,
    pending_requests: Arc<PendingRequestStore>,
    subscriptions: Arc<SubscriptionStore>,
    config: Arc<WebSocketConfig>,
    _marker: PhantomData<H>,
}

impl<H: ProtocolHandler> WebSocketClient<H> {
    /// Create a new WebSocket client and start the connection.
    pub async fn connect(config: WebSocketConfig, handler: H) -> TransportResult<Self> {
        let config = Arc::new(config);
        let (cmd_tx, cmd_rx) = mpsc::channel(config.command_channel_capacity);
        
        let pending_requests = Arc::new(PendingRequestStore::new(config.clone()));
        let subscriptions = Arc::new(SubscriptionStore::new(config.clone()));
        
        let actor = ConnectionActor::new(
            config.clone(),
            handler,
            cmd_rx,
            pending_requests.clone(),
            subscriptions.clone(),
        );
        
        tokio::spawn(actor.run());
        
        Ok(Self {
            cmd_tx,
            pending_requests,
            subscriptions,
            config,
            _marker: PhantomData,
        })
    }
}
```

### 6.2 Request-Response API

```rust
impl<H: ProtocolHandler> WebSocketClient<H> {
    /// Send a request and await the response.
    ///
    /// This method provides a Promise-like async/await experience for
    /// request-response patterns over WebSocket.
    ///
    /// # Example
    ///
    /// ```rust
    /// let response = client.request(order_request).await?;
    /// ```
    pub async fn request<R: Serialize, T: DeserializeOwned>(
        &self,
        request: &R,
    ) -> TransportResult<T> {
        self.request_with_timeout(request, None).await
    }
    
    /// Send a request with a custom timeout.
    pub async fn request_with_timeout<R: Serialize, T: DeserializeOwned>(
        &self,
        request: &R,
        timeout: Option<Duration>,
    ) -> TransportResult<T> {
        // Check capacity
        if !self.pending_requests.has_capacity() {
            return Err(TransportError::internal("Too many pending requests"));
        }
        
        // Generate request ID
        let request_id = RequestId::new();
        
        // Serialize request (implementation would include request_id)
        let message = Message::Text(
            serde_json::to_string(request)?.into()
        );
        
        // Create response channel
        let (reply_tx, reply_rx) = oneshot::channel();
        
        // Send command to actor
        self.cmd_tx.send(ActorCommand::Request {
            message,
            request_id: request_id.clone(),
            reply_tx,
            timeout,
        }).await.map_err(|_| TransportError::internal("Actor closed"))?;
        
        // Await response
        let response = reply_rx.await
            .map_err(|_| TransportError::internal("Response channel closed"))??;
        
        // Deserialize response
        serde_json::from_str(&response).map_err(Into::into)
    }
    
    /// Send a raw request and get raw response.
    pub async fn request_raw(&self, message: Message) -> TransportResult<String> {
        let request_id = RequestId::new();
        let (reply_tx, reply_rx) = oneshot::channel();
        
        self.cmd_tx.send(ActorCommand::Request {
            message,
            request_id,
            reply_tx,
            timeout: None,
        }).await.map_err(|_| TransportError::internal("Actor closed"))?;
        
        reply_rx.await
            .map_err(|_| TransportError::internal("Response channel closed"))?
    }
}
```

### 6.3 Subscription API

```rust
impl<H: ProtocolHandler> WebSocketClient<H> {
    /// Subscribe to a topic and receive updates via a stream.
    ///
    /// # Example
    ///
    /// ```rust
    /// let mut stream = client.subscribe("orderbook.1.BTCUSDT").await?;
    /// while let Some(msg) = stream.recv().await {
    ///     println!("Received: {:?}", msg);
    /// }
    /// ```
    pub async fn subscribe(
        &self,
        topic: impl Into<Topic>,
    ) -> TransportResult<broadcast::Receiver<Message>> {
        let topic = topic.into();
        
        let (reply_tx, reply_rx) = oneshot::channel();
        
        self.cmd_tx.send(ActorCommand::Subscribe {
            topics: vec![topic.clone()],
            reply_tx,
        }).await.map_err(|_| TransportError::internal("Actor closed"))?;
        
        // Wait for subscription confirmation
        reply_rx.await
            .map_err(|_| TransportError::internal("Subscribe channel closed"))??;
        
        // Return the receiver
        Ok(self.subscriptions.subscribe(topic))
    }
    
    /// Subscribe to multiple topics at once.
    pub async fn subscribe_many(
        &self,
        topics: impl IntoIterator<Item = impl Into<Topic>>,
    ) -> TransportResult<Vec<broadcast::Receiver<Message>>> {
        let topics: Vec<Topic> = topics.into_iter().map(Into::into).collect();
        
        let (reply_tx, reply_rx) = oneshot::channel();
        
        self.cmd_tx.send(ActorCommand::Subscribe {
            topics: topics.clone(),
            reply_tx,
        }).await.map_err(|_| TransportError::internal("Actor closed"))?;
        
        reply_rx.await
            .map_err(|_| TransportError::internal("Subscribe channel closed"))??;
        
        Ok(topics.into_iter()
            .map(|t| self.subscriptions.subscribe(t))
            .collect())
    }
    
    /// Unsubscribe from a topic.
    pub async fn unsubscribe(&self, topic: impl Into<Topic>) -> TransportResult<()> {
        let topic = topic.into();
        
        // Remove from local store
        let should_send = self.subscriptions.unsubscribe(&topic);
        
        // Only send unsubscribe if this was the last subscriber
        if should_send {
            self.cmd_tx.send(ActorCommand::Unsubscribe {
                topics: vec![topic],
            }).await.map_err(|_| TransportError::internal("Actor closed"))?;
        }
        
        Ok(())
    }
}
```

### 6.4 Low-Level API

```rust
impl<H: ProtocolHandler> WebSocketClient<H> {
    /// Send a raw message without expecting a response.
    pub async fn send(&self, message: Message) -> TransportResult<()> {
        self.cmd_tx.send(ActorCommand::Send { message })
            .await
            .map_err(|_| TransportError::internal("Actor closed"))
    }
    
    /// Send a JSON message.
    pub async fn send_json<T: Serialize>(&self, payload: &T) -> TransportResult<()> {
        let text = serde_json::to_string(payload)?;
        self.send(Message::Text(text.into())).await
    }
    
    /// Close the connection gracefully.
    pub async fn close(&self) -> TransportResult<()> {
        self.cmd_tx.send(ActorCommand::Close)
            .await
            .map_err(|_| TransportError::internal("Actor already closed"))
    }
    
    /// Check if the connection is ready.
    pub fn is_connected(&self) -> bool {
        !self.cmd_tx.is_closed()
    }
}
```

## 7. Event Emission (Optional Layer)

For users who prefer the event-driven pattern, we provide an optional event emitter layer:

```rust
/// Events emitted by the WebSocket client.
#[derive(Debug, Clone)]
pub enum WebSocketEvent {
    /// Connection established
    Connected,
    /// Connection lost, reconnecting
    Reconnecting { attempt: u32 },
    /// Successfully reconnected
    Reconnected,
    /// Connection closed
    Disconnected { reason: Option<String> },
    /// Authentication successful
    Authenticated,
    /// Received a message (for event-driven pattern)
    Message { topic: Option<Topic>, message: Message },
    /// Error occurred
    Error { error: String },
}

/// Event-driven WebSocket client wrapper.
pub struct EventDrivenClient<H: ProtocolHandler> {
    inner: WebSocketClient<H>,
    event_tx: broadcast::Sender<WebSocketEvent>,
}

impl<H: ProtocolHandler> EventDrivenClient<H> {
    /// Subscribe to client events.
    pub fn events(&self) -> broadcast::Receiver<WebSocketEvent> {
        self.event_tx.subscribe()
    }
}
```

## 8. Error Handling

### 8.1 Error Types

```rust
/// WebSocket-specific errors (extension of TransportError).
#[derive(Error, Debug)]
pub enum WebSocketError {
    /// Connection failed
    #[error("Connection failed: {message}")]
    ConnectionFailed { message: String },
    
    /// Authentication failed
    #[error("Authentication failed: {message}")]
    AuthenticationFailed { message: String },
    
    /// Request timed out
    #[error("Request timed out after {duration:?}")]
    RequestTimeout { duration: Duration },
    
    /// Subscription failed
    #[error("Subscription failed for topic '{topic}': {message}")]
    SubscriptionFailed { topic: String, message: String },
    
    /// Maximum reconnection attempts exceeded
    #[error("Maximum reconnection attempts ({attempts}) exceeded")]
    MaxReconnectAttempts { attempts: u32 },
    
    /// Protocol error
    #[error("Protocol error: {message}")]
    Protocol { message: String },
    
    /// Capacity exceeded
    #[error("Capacity exceeded: {message}")]
    CapacityExceeded { message: String },
}
```

## 9. Implementation Guidelines

### 9.1 Thread Safety and Lock-Free Design

- Use `scc::HashMap` for lock-free concurrent hash maps (preferred over `RwLock<HashMap>`)
- `scc` provides wait-free reads and lock-free writes with better performance under contention
- Use `tokio::sync` for async-aware synchronization (channels, oneshot, broadcast)
- Avoid traditional locks (`Mutex`, `RwLock`) for hot paths
- Use `Arc` for sharing state across tasks

**Why scc over parking_lot/std locks?**
- Lock-free operations eliminate priority inversion and lock contention
- Better scalability on multi-core systems
- Wait-free reads provide consistent low latency
- Built-in support for atomic scan, retain, and entry operations

### 9.2 Performance Considerations

1. **Zero-Copy Message Routing**: Use `Bytes` for binary data to avoid copies
2. **Batch Operations**: Support batching multiple topics in single subscribe/unsubscribe
3. **Connection Pooling**: Support multiple connections per manager (future enhancement)
4. **Efficient Serialization**: Consider `simd-json` for high-throughput scenarios

### 9.3 Testing Strategy

1. **Unit Tests**: Test individual components (stores, protocol handlers)
2. **Integration Tests**: Test full request-response and subscription flows
3. **Mock Server**: Create a mock WebSocket server for testing
4. **Chaos Testing**: Test reconnection under various failure conditions

### 9.4 Observability

1. **Tracing**: Instrument all operations with tracing spans
2. **Metrics**: Track connection state, message counts, latencies
3. **Health Checks**: Expose connection health for monitoring

## 10. Migration Path

### 10.1 From Current `websocket.rs`

The current implementation already uses the actor pattern. Key changes:

1. Add `PendingRequestStore` for request-response pattern
2. Extend `Command` enum with `Request` variant
3. Separate `ProtocolHandler` trait from `ExchangeHandler`
4. Add request-response timeout handling
5. Enhanced reconnection with configurable backoff and jitter

### 10.2 Backward Compatibility

- Existing `ExchangeHandler` trait remains as a simplified interface
- New `ProtocolHandler` extends capabilities for request-response
- Existing subscription API unchanged

## 11. Example Implementations

### 11.1 Generic JSON-RPC Handler

```rust
/// Generic JSON-RPC 2.0 protocol handler.
pub struct JsonRpcHandler {
    // Configuration
}

impl ProtocolHandler for JsonRpcHandler {
    type Request = JsonRpcRequest;
    type Response = JsonRpcResponse;
    type Error = JsonRpcError;
    
    fn classify_message(&self, message: &str) -> MessageKind {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(message) {
            if v.get("id").is_some() && v.get("result").is_some() {
                return MessageKind::Response;
            }
            if v.get("method").is_some() && v.get("params").is_some() {
                return MessageKind::Update;
            }
        }
        MessageKind::Unknown
    }
    
    fn extract_request_id(&self, message: &str) -> Option<RequestId> {
        serde_json::from_str::<serde_json::Value>(message)
            .ok()?
            .get("id")?
            .as_str()
            .map(|s| RequestId::from_string(s))
    }
    
    // ... rest of implementation
}
```

## 12. Future Enhancements

1. **Connection Pooling**: Multiple connections with load balancing
2. **Circuit Breaker**: Prevent cascade failures
3. **Message Compression**: Support permessage-deflate
4. **Binary Protocols**: Support for non-JSON protocols (MessagePack, Protobuf)
5. **Shared Subscriptions**: Multiple clients sharing subscription data
6. **Rate Limiting**: Client-side rate limiting for outgoing messages

## 13. References

- [bybit-api WebSocket Client](https://github.com/tiagosiebler/bybit-api)
- [Tokio Actor Pattern](https://ryhl.io/blog/actors-with-tokio/)
- [WebSocket RFC 6455](https://datatracker.ietf.org/doc/html/rfc6455)
- [JSON-RPC 2.0 Specification](https://www.jsonrpc.org/specification)
