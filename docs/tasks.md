# HPX Development Roadmap

> High-Performance HTTP Client Library Development Task List

## Table of Contents

- [Phase 1: Core Performance & Protocols](#phase-1-core-performance--protocols)
- [Phase 2: Developer Experience](#phase-2-developer-experience)
- [Phase 3: Compatibility & Ecosystem](#phase-3-compatibility--ecosystem)
- [Phase 4: Testing & Documentation](#phase-4-testing--documentation)

---

## Phase 1: Core Performance & Protocols

**Goal**: Outperform competitors in throughput and latency by supporting HTTP/3 and optimizing JSON processing.

### Task 1: Implement HTTP/3 (QUIC) Support

**Background**: Currently hpx is based on hyper (HTTP/1 & 2). HTTP/3 is standard for modern clients and effectively solves head-of-line blocking.

#### 1.1 Dependencies & Feature Flag Configuration

- [ ] Add `http3` feature flag in `crates/hpx/Cargo.toml`
- [ ] Add `h3` dependency to workspace (for HTTP/3 protocol handling)
- [ ] Add `h3-quinn` dependency to workspace (for QUIC transport layer)
- [ ] Add `quinn` dependency to workspace (QUIC implementation)
- [ ] Research `boring` (BoringSSL) compatibility with `quinn`
  - Option A: Investigate BoringSSL QUIC API bindings (boring-sys)
  - Option B: Allow switching to `rustls` when `http3` feature is enabled
  - Option C: Implement dual TLS backend support (`boring` for HTTP/1&2, `rustls` for HTTP/3)

#### 1.2 QUIC Connector Implementation

- [ ] Create `quic.rs` module under `crates/hpx/src/client/conn/`
- [ ] Implement `QuicConnector` struct

  ```rust
  pub struct QuicConnector {
      endpoint: quinn::Endpoint,
      config: QuicConfig,
  }
  ```

- [ ] Implement QUIC connection pool management
- [ ] Implement 0-RTT connection resumption support
- [ ] Implement Connection Migration support

#### 1.3 HTTP/3 Protocol Layer Integration

- [ ] Add HTTP/3 branch in `crates/hpx/src/client/conn/http.rs`
- [ ] Implement HTTP/3 request/response encoding/decoding
- [ ] Handle HTTP/3 specific header compression (QPACK)
- [ ] Implement Server Push handling

#### 1.4 ClientBuilder Extension

- [ ] Add `.http3_prior_knowledge()` method to `ClientBuilder`
- [ ] Add `.prefer_http3()` method (try HTTP/3 first, fallback on failure)
- [ ] Add `.quic_config()` method for custom QUIC configuration
- [ ] Implement Alt-Svc header parsing and HTTP/3 upgrade logic

#### 1.5 Fingerprint Emulation Compatibility

- [ ] Research QUIC/HTTP3 fingerprint characteristics (Initial Packet, Transport Parameters)
- [ ] Implement custom configuration for QUIC transport parameters
- [ ] Ensure `Emulation` configuration applies to HTTP/3 as well

### Task 2: Integrate simd-json for Serialization Performance

**Background**: Rust's `simd-json` leverages SIMD instructions, several times faster than `serde_json`.

#### 2.1 Dependency Configuration

- [x] Add `simd-json` dependency to `Cargo.toml` workspace
- [x] Add `simd-json` feature in `crates/hpx/Cargo.toml`

  ```toml
  simd-json = ["dep:simd-json", "json"]
  ```

- [x] Add `simd-json` feature in `crates/hpx-transport/Cargo.toml`

#### 2.2 hpx Crate Integration

- [x] Modify `json()` method in `crates/hpx/src/client/response.rs`

  ```rust
  #[cfg(feature = "simd-json")]
  pub async fn json<T: DeserializeOwned>(self) -> Result<T, Error> {
      let bytes = self.bytes().await?;
      // simd-json requires mutable buffer
      let mut bytes_vec = bytes.to_vec();
      simd_json::from_slice(&mut bytes_vec).map_err(Error::decode)
  }
  ```

- [x] Modify JSON serialization logic in `crates/hpx/src/client/request.rs` (Note: simd-json serialization delegates to serde_json, no changes needed)
- [ ] Add runtime SIMD support detection (optional fallback to serde_json)

#### 2.3 hpx-transport Crate Integration

- [x] Modify JSON handling in `crates/hpx-transport/src/http.rs` (Note: serialization uses serde_json, deserialization uses simd-json when feature enabled)
- [x] Modify serialization logic in `crates/hpx-transport/src/transport.rs`
- [x] Ensure `Request::json()` and `Response::json()` use unified JSON backend

#### 2.4 Performance Optimization & Safety

- [ ] Handle efficient conversion from `Bytes` to mutable buffer
- [ ] Add `json_borrowed()` method for zero-copy deserialization
- [ ] Write benchmarks comparing `serde_json` vs `simd-json` performance

### Task 3: Zero-Copy Body Passthrough

**Background**: Reduce memory copies between network layer and user layer.

#### 3.1 Response Streaming Interface

- [x] Add streaming methods in `crates/hpx/src/client/response.rs`

  ```rust
  pub fn bytes_stream(self) -> impl Stream<Item = Result<Bytes, Error>>
  ```

- [x] Implement `AsyncRead` trait support (via `reader()` method)
- [x] Add `into_body()` method to return raw `Body`

#### 3.2 hpx-transport Zero-Copy Optimization

- [ ] Modify `parse_response` in `crates/hpx-transport/src/http.rs`
- [x] Implement `StreamingResponse` type for streaming passthrough

  ```rust
  pub struct StreamingResponse<S> {
      status: StatusCode,
      headers: HeaderMap,
      body: S,
  }
  ```

- [ ] Optimize decompression middleware for streaming decompression

#### 3.3 Body Type Optimization

- [x] Review memory allocations in `crates/hpx/src/client/body.rs`
- [x] Implement `Body::into_bytes_mut()` to avoid unnecessary clones
- [x] Add `Body::reader()` returning `impl AsyncRead`

---

## Phase 2: Developer Experience

**Goal**: Provide "intuitive" APIs similar to Python httpx and Node.js Ky.

### Task 4: Implement Lifecycle Hooks (Hooks / Interceptors)

**Background**: Tower Middleware is great for library developers but complex for end users. Reference Ky's hooks API.

#### 4.1 Hooks Structure Definition

- [x] Create `hooks.rs` module under `crates/hpx-transport/src/`
- [x] Define `Hooks` struct

  ```rust
  pub struct Hooks {
      before_request: Vec<Arc<dyn BeforeRequestHook>>,
      after_response: Vec<Arc<dyn AfterResponseHook>>,
      before_retry: Vec<Arc<dyn BeforeRetryHook>>,
      before_redirect: Vec<Arc<dyn BeforeRedirectHook>>,
      on_error: Vec<Arc<dyn OnErrorHook>>,
  }

  #[async_trait]
  pub trait BeforeRequestHook: Send + Sync {
      async fn on_request(&self, request: &mut Request) -> Result<(), HookError>;
  }

  #[async_trait]
  pub trait AfterResponseHook: Send + Sync {
      async fn on_response(&self, response: &mut Response) -> Result<(), HookError>;
  }
  ```

#### 4.2 HttpClient Integration

- [x] Modify `HttpClient` in `crates/hpx-transport/src/http.rs`
- [x] Add `with_hooks()` builder method
- [x] Call appropriate hooks before and after request sending
- [x] Handle hook execution errors

#### 4.3 Convenient Hook Implementations

- [x] Implement `SignRequestHook` - request signing hook
- [x] Implement `LoggingHook` - simple logging hook
- [x] Implement `HeaderInjectionHook` - dynamic header injection
- [x] Implement `RequestIdHook` - automatic request ID addition

#### 4.4 hpx Crate Integration

- [x] Add hooks support to `ClientBuilder` in `crates/hpx/src/client/http.rs`
- [x] Add `on_request()`, `on_response()` convenience methods

  ```rust
  client.on_request(|req| {
      req.headers_mut().insert("X-Custom", "value");
      Ok(())
  })
  ```

### Task 5: Smart Retry Strategy with Jitter

**Background**: Current `RetryMiddleware` is basic, needs enhancement to match Node.js Got's capabilities.

#### 5.1 Decorrelated Jitter Implementation

- [x] Modify `RetryMiddleware` in `crates/hpx-transport/src/middleware.rs`
- [x] Implement Decorrelated Jitter algorithm

  ```rust
  fn calculate_delay_with_jitter(&self, attempt: usize, prev_delay: Duration) -> Duration {
      let base = self.initial_delay.as_millis() as f64;
      let cap = self.max_delay.as_millis() as f64;
      // Decorrelated Jitter: sleep = min(cap, random_between(base, sleep * 3))
      let jittered = rand::thread_rng().gen_range(base..=(prev_delay.as_millis() as f64 * 3.0));
      Duration::from_millis(jittered.min(cap) as u64)
  }
  ```

- [x] Add `rand` dependency to workspace

#### 5.2 Retry-After Header Support

- [x] Parse `Retry-After` response header
  - Support seconds format: `Retry-After: 120`
  - Support HTTP Date format: `Retry-After: Wed, 21 Oct 2025 07:28:00 GMT`
- [x] Add `httpdate` dependency for HTTP Date parsing
- [x] Prioritize `Retry-After` value when present

#### 5.3 Enhanced Retry Conditions

- [x] Default support for `429 Too Many Requests` retry
- [x] Add retry for `502 Bad Gateway`, `503 Service Unavailable`, `504 Gateway Timeout`
- [x] Implement network error categorization (DNS failure, connection timeout, connection reset)
- [x] Add idempotency check (only auto-retry idempotent requests)

  ```rust
  fn is_idempotent(method: &Method) -> bool {
      matches!(method, Method::Get | Method::Head | Method::Put | Method::Delete | Method::Options)
  }
  ```

#### 5.4 Retry Strategy Configuration Enhancement

- [x] Add `RetryStrategy` enum

  ```rust
  pub enum RetryStrategy {
      Exponential { base: Duration, max: Duration },
      Linear { delay: Duration },
      Constant { delay: Duration },
      DecorrelatedJitter { base: Duration, max: Duration },
  }
  ```

- [x] Add per-HTTP-method retry configuration
- [x] Add Retry Budget to prevent retry storms

### Task 6: Typed Client Wrapper

**Background**: Similar to Python httpx API style, providing more ergonomic interfaces.

#### 6.1 Generic Request Methods

- [x] Extend `HttpClient` in `crates/hpx-transport/src/http.rs`

  ```rust
  impl HttpClient {
      pub async fn fetch_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
          let response = self.get(path).send().await?;
          self.handle_response(response).await
      }

      pub async fn post_json<T: Serialize, R: DeserializeOwned>(
          &self,
          path: &str,
          body: &T
      ) -> Result<R, ApiError> {
          let response = self.post(path).json(body).send().await?;
          self.handle_response(response).await
      }
  }
  ```

#### 6.2 Automatic Error Mapping

- [x] Define `ApiError` trait

  ```rust
  pub trait ApiError: DeserializeOwned + std::error::Error {
      fn from_status(status: StatusCode, body: Bytes) -> Self;
  }
  ```

- [x] Implement automatic 4xx/5xx response parsing
- [x] Support custom error type mapping

  ```rust
  #[derive(Deserialize, Debug)]
  struct MyApiError {
      code: String,
      message: String,
  }

  impl ApiError for MyApiError { ... }

  let user: User = client.get("/users/1").typed::<MyApiError>().fetch().await?;
  ```

#### 6.3 Typed Response Wrapper

- [x] Implement `TypedResponse<T>` wrapper

  ```rust
  pub struct TypedResponse<T> {
      pub data: T,
      pub status: StatusCode,
      pub headers: HeaderMap,
      pub latency: Duration,
  }
  ```

- [x] Add `fetch_with_meta()` method to return complete metadata

#### 6.4 Builder Pattern Enhancement

- [x] Implement chained call support

  ```rust
  let user: User = client
      .get("/users/1")
      .bearer_auth("token")
      .timeout(Duration::from_secs(5))
      .fetch_json()
      .await?;
  ```

- [x] Add `.retry()` convenience method to override default retry configuration

---

## Phase 3: Compatibility & Ecosystem

**Goal**: Enable the library to run in browsers (WASM) and non-Tokio environments.

### Task 7: Complete WASM Support (Browser Fetch Backend)

**Background**: Compiling Rust to WASM for browser execution is a killer feature, but hpx currently depends heavily on `tokio::net`.

#### 7.1 Conditional Compilation Architecture

- [ ] Create `wasm.rs` module under `crates/hpx/src/client/conn/`
- [ ] Use `cfg_if` or `#[cfg(target_arch = "wasm32")]` conditional compilation
- [ ] Define platform-agnostic `Connector` trait

  ```rust
  #[async_trait(?Send)]
  pub trait Connector {
      async fn connect(&self, uri: &Uri) -> Result<Connection, Error>;
  }
  ```

#### 7.2 Web Fetch Backend

- [ ] Add `wasm-bindgen`, `web-sys`, `js-sys` dependencies
- [ ] Implement `FetchConnector` using browser's `window.fetch`

  ```rust
  #[cfg(target_arch = "wasm32")]
  pub struct FetchConnector {
      // WASM-specific configuration
  }

  #[cfg(target_arch = "wasm32")]
  impl FetchConnector {
      pub async fn send(&self, request: Request) -> Result<Response, Error> {
          let opts = web_sys::RequestInit::new();
          // ... configure fetch options
          let window = web_sys::window().unwrap();
          let resp_promise = window.fetch_with_request(&web_request);
          // ... handle response
      }
  }
  ```

#### 7.3 Feature Isolation

- [ ] Add `wasm` feature in `Cargo.toml`

  ```toml
  [target.'cfg(target_arch = "wasm32")'.dependencies]
  wasm-bindgen = "0.2"
  web-sys = { version = "0.3", features = ["Window", "Request", "Response", "Headers"] }
  js-sys = "0.3"
  wasm-bindgen-futures = "0.4"
  ```

- [ ] Ensure non-WASM features are not compiled for WASM targets
- [ ] Disable WASM-incompatible features (e.g., `boring`, `hickory-dns`, `socks`)

#### 7.4 API Compatibility

- [ ] Ensure upper-level `Client` API remains consistent between WASM and Native
- [ ] Handle async runtime differences in WASM (`wasm-bindgen-futures`)
- [ ] Implement cookie handling in WASM (browser auto-management vs manual management)

#### 7.5 Testing & Examples

- [ ] Add WASM compilation target to CI
- [ ] Create `examples/wasm_demo/` example project
- [ ] Write WASM-specific integration tests

---

## Phase 4: Testing & Documentation

### Task 8: Performance Benchmarks

- [x] Create `benches/` directory
- [x] Implement HTTP/1.1 throughput benchmark (`http_throughput.rs`)
- [ ] Implement HTTP/2 throughput benchmark
- [ ] Implement HTTP/3 throughput benchmark (when completed)
- [x] Implement JSON parsing benchmark (`json_parsing.rs` - `serde_json` vs `simd-json`)
- [x] Implement connection pool performance benchmark (`connection_pool.rs`)
- [ ] Comparison tests with `reqwest`, `ureq`, `hyper`
- [ ] Publish benchmark results to documentation

### Task 11: Integration Test Enhancement

- [ ] Add HTTP/3 integration tests
- [ ] Add WASM integration tests
- [x] Add retry logic integration tests (existing in `crates/hpx/tests/retry.rs`)
- [x] Add hooks system integration tests (`crates/hpx/tests/hooks.rs`)
- [ ] Add fingerprint emulation verification tests (using ja3.io and similar services)

### Task 12: Documentation Improvement

- [ ] Improve README.md (add feature matrix table)
- [ ] Write API documentation (rustdoc)
- [ ] Create user guides under `docs/` directory
  - [ ] Getting Started
  - [ ] HTTP/3 Usage Guide
  - [ ] Fingerprint Emulation Guide
  - [ ] Performance Tuning Guide
  - [ ] WASM Usage Guide
- [ ] Create example project collection
- [ ] Publish to docs.rs

---

## Priority Ranking

### P0 - Core Features (Start Immediately)

1. Task 2: simd-json integration (low risk, high reward)
2. Task 5: Smart retry strategy (improve existing functionality)
3. Task 4: Lifecycle hooks (improve DX)

### P1 - Important Features (Short-term)

4. Task 6: Typed client wrapper
5. Task 3: Zero-copy body passthrough
6. Task 8: Performance benchmarks

### P2 - Strategic Features (Medium-term)

7. Task 1: HTTP/3 support (high complexity)
8. Task 7: WASM support

---

## Technical Decision Records

### TLS Backend Strategy

**Problem**: HTTP/3 requires QUIC, `quinn` defaults to `rustls`, but hpx uses `boring` for fingerprint emulation.

**Decision Options**:

1. Implement BoringSSL QUIC bindings for HTTP/3 (high effort, best compatibility)
2. Use `rustls` for HTTP/3, `boring` for HTTP/1&2 (dual backend, complex configuration)
3. HTTP/3 temporarily without fingerprint emulation (quick implementation, limited functionality)

**Current Status**: Pending

### simd-json Mutable Buffer Handling

**Problem**: `simd-json` requires mutable buffer, but `Bytes` is immutable.

**Solution**:

```rust
// Option 1: Convert to Vec
let mut vec = bytes.to_vec();
simd_json::from_slice(&mut vec)?;

// Option 2: Use BytesMut (if available upstream)
let mut bytes_mut: BytesMut = ...;
simd_json::from_slice(bytes_mut.as_mut())?;
```

**Current Status**: Using Option 1, can optimize later

---

## References

- [HTTP/3 RFC 9114](https://www.rfc-editor.org/rfc/rfc9114)
- [QUIC RFC 9000](https://www.rfc-editor.org/rfc/rfc9000)
- [quinn QUIC implementation](https://github.com/quinn-rs/quinn)
- [simd-json documentation](https://github.com/simd-lite/simd-json)
- [Ky hooks API](https://github.com/sindresorhus/ky#hooks)
- [Got retry options](https://github.com/sindresorhus/got#retry)
