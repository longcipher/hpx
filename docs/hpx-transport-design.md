# hpx-transport Architectural Refactoring Design

> Repositioning hpx-transport from a "duplicate wrapper of hpx" to a "specialized toolkit for building Exchange SDKs"

## 1. Background and Issues

### 1.1 Current Status Analysis

Currently, `hpx-transport` has significant functional overlap with `hpx`:

| Feature | hpx | hpx-transport | Issue |
|---------|-----|---------------|-------|
| HTTP Client | ✅ Complete | ✅ Re-wrapped | Redundant |
| WebSocket | ✅ Complete | ✅ Re-wrapped | Redundant |
| Retry | ✅ Tower Policy | ✅ Custom Middleware | Redundant |
| Hooks | ✅ client/layer/hooks | ✅ hooks.rs | Redundant |
| Request/Response Types | ✅ Native | ✅ Custom | Redundant |

### 1.2 Core Issues

1. **Confused Abstraction Layers**: hpx-transport both re-wraps underlying types and relies on hpx to do the actual work.
2. **Large Volume of Duplicate Code**: Hooks, Retry, and Middleware are implemented in both crates.
3. **Type Conversion Overhead**: Custom Request/Response types need to be converted to hpx types.
4. **High Maintenance Cost**: Two sets of APIs easily lead to inconsistencies.

### 1.3 Retained Value

Some features in hpx-transport are not present in hpx; these provide the real competitive advantage:

- `auth.rs` - Exchange authentication strategies (HMAC, OAuth2, JWT, etc.)
- `websocket.rs` - Actor-based automatic reconnection + subscription management
- `typed.rs` - Typed responses + unified error handling
- `error.rs` - Exchange-level error classification
- `metrics.rs` - OpenTelemetry observability

## 2. New Architecture Design

### 2.1 Layer Positioning

```text
┌─────────────────────────────────────────────────────────────┐
│                    Exchange SDK Layer                        │
│  (binance-sdk, okx-sdk, etc. - User implemented)             │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    hpx-transport                             │
│  Focuses on: Common Exchange Abstractions + Conn Mgmt + Logic │
│  - ExchangeClient trait (Unified REST + WS)                  │
│  - Authentication (Various signing strategies)               │
│  - RateLimiter (Exchange-level rate limiting)                │
│  - ConnectionManager (Auto-reconnect + Sub restoration)      │
│  - TypedResponse (Unified error handling)                    │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                         hpx                                  │
│  Focuses on: Underlying HTTP/WS Capabilities                 │
│  - Client (Conn pooling, TLS, Proxy)                         │
│  - Hooks (Request lifecycle)                                 │
│  - Retry (Retry policies)                                    │
│  - WebSocket (Upgrade, Fragmentation)                        │
│  - Emulation (Browser fingerprinting)                        │
└─────────────────────────────────────────────────────────────┘
```

### 2.2 Module Planning

#### Retained and Refactored Modules

| Module | File | Responsibilities | Changes |
|--------|------|------------------|---------|
| Auth | `auth.rs` | Exchange signing strategies | Enhanced with more preset implementations |
| WebSocket | `websocket.rs` | Actor-based connection management | Streamlined, using hpx types |
| Typed Response | `typed.rs` | Unified response handling | Streamlined, removing duplicate definitions |
| Error | `error.rs` | Error classification | Streamlined, reducing number of types |
| Metrics | `metrics.rs` | Observability | Retained |

#### Deleted Modules

| Module | File | Reason |
|--------|------|--------|
| Middleware | `middleware.rs` | Redundant with hpx Tower layers |
| Hooks | `hooks.rs` | Redundant with hpx hooks |
| Transport | `transport.rs` | Request/Response/Method redundant with hpx |
| HTTP Client | `http.rs` | Mostly redundant with hpx::Client |

#### New Modules

| Module | File | Responsibilities |
|--------|------|------------------|
| Exchange | `exchange.rs` | ExchangeClient trait + RestClient |
| Rate Limit | `rate_limit.rs` | Exchange-level rate limiting |

## 3. Detailed API Design

### 3.1 Core Trait: ExchangeClient

```rust
// src/exchange.rs

use async_trait::async_trait;
use hpx::Client;
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    auth::Authentication,
    error::TransportResult,
    typed::TypedResponse,
    websocket::{WebSocketConfig, WebSocketHandle},
};

/// Exchange client trait - Unified REST and WebSocket
#[async_trait]
pub trait ExchangeClient: Send + Sync {
    /// Authenticator type
    type Auth: Authentication;

    /// Get underlying HTTP client
    fn http(&self) -> &Client;

    /// Get authenticator
    fn auth(&self) -> &Self::Auth;

    /// Get base URL
    fn base_url(&self) -> &str;

    /// Send GET request
    async fn get<T: DeserializeOwned + Send>(
        &self,
        path: &str,
    ) -> TransportResult<TypedResponse<T>>;

    /// Send GET request with query parameters
    async fn get_with_query<Q: Serialize + Send + Sync, T: DeserializeOwned + Send>(
        &self,
        path: &str,
        query: &Q,
    ) -> TransportResult<TypedResponse<T>>;

    /// Send POST request
    async fn post<B: Serialize + Send + Sync, T: DeserializeOwned + Send>(
        &self,
        path: &str,
        body: &B,
    ) -> TransportResult<TypedResponse<T>>;

    /// Send PUT request
    async fn put<B: Serialize + Send + Sync, T: DeserializeOwned + Send>(
        &self,
        path: &str,
        body: &B,
    ) -> TransportResult<TypedResponse<T>>;

    /// Send DELETE request
    async fn delete<T: DeserializeOwned + Send>(
        &self,
        path: &str,
    ) -> TransportResult<TypedResponse<T>>;
}

/// REST client configuration
#[derive(Debug, Clone)]
pub struct RestConfig {
    /// Base URL
    pub base_url: String,
    /// Request timeout
    pub timeout: std::time::Duration,
    /// Whether to enable rate limiting
    pub rate_limiting: bool,
}

impl RestConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            timeout: std::time::Duration::from_secs(30),
            rate_limiting: true,
        }
    }
}

/// Generic REST client implementation
pub struct RestClient<A: Authentication> {
    client: Client,
    auth: A,
    config: RestConfig,
}

impl<A: Authentication> RestClient<A> {
    pub fn new(config: RestConfig, auth: A) -> TransportResult<Self> {
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| crate::error::TransportError::Config {
                message: e.to_string(),
            })?;

        Ok(Self { client, auth, config })
    }
}
```

### 3.2 Authentication Module Enhancements

```rust
// src/auth.rs

use async_trait::async_trait;
use hpx::header::HeaderMap;

use crate::error::TransportResult;

/// Authentication trait
#[async_trait]
pub trait Authentication: Send + Sync + Clone {
    /// Sign request - Modifies headers and/or query
    async fn sign(
        &self,
        method: &http::Method,
        path: &str,
        headers: &mut HeaderMap,
        body: Option<&[u8]>,
    ) -> TransportResult<Option<String>>; // Returns optional signature query string

    /// Generate WebSocket authentication message (if needed)
    fn ws_auth_message(&self) -> Option<String> {
        None
    }
}

/// No authentication
#[derive(Debug, Clone, Default)]
pub struct NoAuth;

#[async_trait]
impl Authentication for NoAuth {
    async fn sign(
        &self,
        _method: &http::Method,
        _path: &str,
        _headers: &mut HeaderMap,
        _body: Option<&[u8]>,
    ) -> TransportResult<Option<String>> {
        Ok(None)
    }
}

/// API Key authentication (Header or Query)
#[derive(Debug, Clone)]
pub struct ApiKeyAuth {
    pub key_name: String,
    pub key_value: String,
    pub location: ApiKeyLocation,
}

#[derive(Debug, Clone)]
pub enum ApiKeyLocation {
    Header,
    Query,
}

/// HMAC signature authentication (Common for Binance, OKX, etc.)
#[derive(Clone)]
pub struct HmacAuth {
    pub api_key: String,
    pub secret_key: String,
    pub algorithm: HmacAlgorithm,
    /// Signature parameter name
    pub signature_param: String,
    /// Timestamp parameter name
    pub timestamp_param: String,
    /// API Key header name
    pub api_key_header: String,
}

#[derive(Debug, Clone)]
pub enum HmacAlgorithm {
    Sha256,
    Sha512,
}
```

### 3.3 WebSocket Manager

```rust
// src/websocket.rs

use std::time::Duration;
use futures_util::Stream;
use hpx::ws::message::Message;
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::broadcast;

use crate::error::TransportResult;

/// WebSocket configuration
#[derive(Debug, Clone)]
pub struct WebSocketConfig {
    pub url: String,
    pub reconnect_initial_delay: Duration,
    pub reconnect_max_delay: Duration,
    pub reconnect_backoff_factor: f64,
    pub ping_interval: Duration,
    pub pong_timeout: Duration,
    pub channel_capacity: usize,
}

/// Exchange message handler trait
pub trait ExchangeHandler: Send + Sync + 'static {
    /// Extract topic from message
    fn get_topic(&self, message: &str) -> Option<String>;

    /// Build subscription message
    fn build_subscribe(&self, topic: &str) -> Message;

    /// Build unsubscription message
    fn build_unsubscribe(&self, topic: &str) -> Message;

    /// Messages sent upon connection establishment (e.g., authentication)
    fn on_connect(&self) -> Vec<Message> {
        vec![]
    }

    /// Decode binary message (e.g., gzip compression)
    fn decode_binary(&self, data: &[u8]) -> Option<String> {
        String::from_utf8(data.to_vec()).ok()
    }
}

/// WebSocket connection handle
#[derive(Clone)]
pub struct WebSocketHandle {
    cmd_tx: tokio::sync::mpsc::UnboundedSender<Command>,
}

impl WebSocketHandle {
    /// Subscribe to a topic
    pub async fn subscribe(&self, topic: &str) -> TransportResult<broadcast::Receiver<Message>>;

    /// Unsubscribe from a topic
    pub async fn unsubscribe(&self, topic: &str) -> TransportResult<()>;

    /// Send raw message
    pub async fn send(&self, message: Message) -> TransportResult<()>;

    /// Send JSON message
    pub async fn send_json<T: Serialize>(&self, payload: &T) -> TransportResult<()>;

    /// Close connection
    pub async fn close(&self) -> TransportResult<()>;
}
```

### 3.4 Typed Response

```rust
// src/typed.rs

use std::time::Duration;
use bytes::Bytes;
use http::StatusCode;
use serde::de::DeserializeOwned;

/// Typed response
#[derive(Debug, Clone)]
pub struct TypedResponse<T> {
    /// Parsed data
    pub data: T,
    /// HTTP status code
    pub status: StatusCode,
    /// Request latency
    pub latency: Duration,
    /// Original response body (optionally retained)
    raw_body: Option<Bytes>,
}

impl<T> TypedResponse<T> {
    pub fn new(data: T, status: StatusCode, latency: Duration) -> Self {
        Self {
            data,
            status,
            latency,
            raw_body: None,
        }
    }

    pub fn with_raw_body(mut self, body: Bytes) -> Self {
        self.raw_body = Some(body);
        self
    }

    pub fn raw_body(&self) -> Option<&Bytes> {
        self.raw_body.as_ref()
    }

    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }

    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> TypedResponse<U> {
        TypedResponse {
            data: f(self.data),
            status: self.status,
            latency: self.latency,
            raw_body: self.raw_body,
        }
    }
}

/// API error trait
pub trait ApiError: DeserializeOwned + std::error::Error + Send + Sync {
    /// Construct error from status code and response body
    fn from_response(status: StatusCode, body: &[u8]) -> Self;
}
```

### 3.5 Streamlined Error Types

```rust
// src/error.rs

use thiserror::Error;

pub type TransportResult<T> = Result<T, TransportError>;

#[derive(Error, Debug)]
pub enum TransportError {
    /// HTTP request error
    #[error("HTTP error: {0}")]
    Http(#[from] hpx::Error),

    /// Authentication error
    #[error("Authentication error: {message}")]
    Auth { message: String },

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// API error response
    #[error("API error: status={status}, body={body}")]
    Api { status: http::StatusCode, body: String },

    /// Rate limiting error
    #[error("Rate limited: retry after {retry_after:?}")]
    RateLimit { retry_after: Option<std::time::Duration> },

    /// WebSocket error
    #[error("WebSocket error: {message}")]
    WebSocket { message: String },

    /// Configuration error
    #[error("Configuration error: {message}")]
    Config { message: String },

    /// Internal error
    #[error("Internal error: {message}")]
    Internal { message: String },
}
```

### 3.6 Rate Limit Module (New)

```rust
// src/rate_limit.rs

use std::time::{Duration, Instant};
use parking_lot::Mutex;

/// Token bucket rate limiter
pub struct RateLimiter {
    buckets: Mutex<Vec<TokenBucket>>,
}

struct TokenBucket {
    name: String,
    capacity: u32,
    tokens: f64,
    refill_rate: f64, // tokens per second
    last_update: Instant,
}

impl RateLimiter {
    /// Create new rate limiter
    pub fn new() -> Self {
        Self {
            buckets: Mutex::new(Vec::new()),
        }
    }

    /// Add rate limit rule
    pub fn add_limit(&self, name: &str, capacity: u32, refill_rate: f64) {
        let mut buckets = self.buckets.lock();
        buckets.push(TokenBucket {
            name: name.to_string(),
            capacity,
            tokens: capacity as f64,
            refill_rate,
            last_update: Instant::now(),
        });
    }

    /// Try to acquire token (non-blocking)
    pub fn try_acquire(&self, name: &str) -> bool {
        // Implementation logic...
        true
    }
}
```
