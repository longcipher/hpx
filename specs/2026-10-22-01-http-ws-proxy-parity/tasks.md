# HTTP/WebSocket Client Completeness + Proxy Support Parity — Tasks

| Metadata | Details |
| :--- | :--- |
| **Design Doc** | `specs/2026-10-22-01-http-ws-proxy-parity/design.md` |
| **Status** | Complete |
| **Phases** | 1 Proxy API parity → 2 System proxy detection → 3 HTTP/3 proxy safety + WebSocket proxy → 4 WebSocket subprotocols → 5 Verification |
| **Total Tasks** | 12 |
| **Strict Lints** | `#![deny(unwrap_used, expect_used, panic, todo, unimplemented, dbg_macro, print_stdout, print_stderr, allow_attributes, unused_must_use)]` — use `?` and typed `Result` everywhere. |

## Execution Strategy

> **Outside-In TDD:** each task implements one scenario (or a small cohesive group) and ends with an independently testable deliverable. RED → GREEN → REFACTOR.

---

## Phase 1 — Proxy API parity

### Task 1.1: `Proxy::custom()` (reqwest parity)

- **TaskID:** `T1_1`
- **DependsOn:** `None`
- **Complexity:** `Medium`
- **Requirement Coverage:** REQ-01, REQ-02
- **Interfaces:**
  - **Produces:** `hpx::Proxy::custom`, `hpx::IntoProxy`, `hpx::proxy::CustomProxy` (crate-internal)
- **EvalRule:** a closure may return a `&str`, `String` or `Uri`; the selected endpoint receives the proxy's credentials and headers; two custom rules are never equal, clones are.
- **Status:** ✅ Complete — 7 unit tests in `crates/hpx/src/proxy.rs`.

### Task 1.2: Environment-driven proxy construction

- **TaskID:** `T1_2`
- **DependsOn:** `T1_1`
- **Complexity:** `Medium`
- **Requirement Coverage:** REQ-03, REQ-04
- **Interfaces:**
  - **Produces:** `hpx::Proxy::from_env()`, `hpx::ClientBuilder::proxies_from_env()`
- **EvalRule:** `ALL_PROXY` wins for the single-rule constructor; the scheme-specific variables win over `ALL_PROXY` in the multi-rule constructor; `NO_PROXY` is applied to every rule; a CGI context yields no proxy.
- **Status:** ✅ Complete — 8 unit tests.

### Task 1.3: Per-request proxy selection and bypass

- **TaskID:** `T1_3`
- **DependsOn:** `T1_2`
- **Complexity:** `Medium`
- **Requirement Coverage:** REQ-05, REQ-06
- **Interfaces:**
  - **Produces:** `hpx::RequestBuilder::no_proxy()`, internal `ProxySelection`
- **EvalRule:** a request with `.no_proxy()` never reaches the client-level proxy; a request with `.proxy(p)` uses `p` and never falls back to the client-level list.
- **Status:** ✅ Complete — 4 integration tests in `crates/hpx/tests/proxy.rs`.

---

## Phase 2 — System proxy detection

### Task 2.1: Windows per-scheme `ProxyServer` parsing

- **TaskID:** `T2_1`
- **DependsOn:** `T1_3`
- **Complexity:** `Low`
- **Requirement Coverage:** REQ-07, REQ-08
- **Interfaces:**
  - **Produces:** `crates/hpx/src/proxy/sysproxy.rs` (new), rewritten `crates/hpx/src/proxy/win.rs`
- **EvalRule:** `host:port` applies to HTTP and HTTPS; `http=`/`https=`/`socks=` map to the matching matcher slots; unknown keys are ignored; `ProxyOverride` keeps `<local>` and rewrites `*.host` to `.host`.
- **Status:** ✅ Complete — 10 unit tests, run on every platform.

### Task 2.2: `<local>` support in the `NO_PROXY` matcher

- **TaskID:** `T2_2`
- **DependsOn:** `T2_1`
- **Complexity:** `Low`
- **Requirement Coverage:** REQ-09
- **EvalRule:** `<local>` matches every hostname without a dot and no hostname with a dot.
- **Status:** ✅ Complete — 2 unit tests.

### Task 2.3: macOS system SOCKS proxy detection

- **TaskID:** `T2_3`
- **DependsOn:** `T2_1`
- **Complexity:** `Low`
- **Requirement Coverage:** REQ-10
- **Interfaces:**
  - **Consumes:** `kSCPropNetProxiesSOCKSEnable` / `Proxy` / `Port`
- **EvalRule:** an enabled SOCKS system proxy populates the catch-all matcher slot as `socks5://host:port` and does not override an existing value.
- **Status:** ✅ Complete — 3 unit tests over the pure assembly helper.

---

## Phase 3 — HTTP/3 proxy safety and WebSocket proxy completeness

### Task 3.1: HTTP/3 must never bypass the proxy

- **TaskID:** `T3_1`
- **DependsOn:** `T2_3`
- **Complexity:** `Medium`
- **Requirement Coverage:** REQ-11, REQ-12
- **Interfaces:**
  - **Consumes:** `HttpClient::is_proxied`, `ProxySelection`
- **EvalRule:** with a proxy configured, `http3_only()` fails with a client error and the proxy is never contacted; an Alt-Svc upgrade for a proxied destination falls back to TCP.
- **Status:** ✅ Complete — 1 integration test in `crates/hpx/tests/http3.rs`.

### Task 3.2: yawc WebSocket proxy completeness

- **TaskID:** `T3_2`
- **DependsOn:** `T3_1`
- **Complexity:** `Medium`
- **Requirement Coverage:** REQ-13, REQ-14, REQ-15, REQ-16
- **Interfaces:**
  - **Produces:** `hpx_yawc::ProxyConfig::Socks4`
- **EvalRule:** a SOCKS4 handshake carries the right version, command, port, address and user id; a SOCKS4a handshake carries the `0.0.0.1` marker and the hostname; a proxy URL without a port resolves to the scheme default; a `https://` proxy is reached over TLS.
- **Status:** ✅ Complete — 4 new unit tests in `crates/yawc/src/native/proxy.rs` plus 1 integration test in `crates/hpx/tests/ws_yawc.rs`.

---

## Phase 4 — WebSocket subprotocol negotiation

### Task 4.1: yawc client/server subprotocol negotiation

- **TaskID:** `T4_1`
- **DependsOn:** `T3_2`
- **Complexity:** `Medium`
- **Requirement Coverage:** REQ-17, REQ-18, REQ-19
- **Interfaces:**
  - **Produces:** `hpx_yawc::Options::with_protocols`, `hpx_yawc::WebSocket::protocol()`, `hpx_yawc::WebSocketError::InvalidSecWebSocketProtocol`
- **EvalRule:** the client sends the offered protocols in order; the server echoes the first mutually supported one; a server that selects an unoffered protocol fails the connection.
- **Status:** ✅ Complete — 3 unit tests for the selection helper plus 2 integration tests.

### Task 4.2: hpx WebSocket subprotocol API

- **TaskID:** `T4_2`
- **DependsOn:** `T4_1`
- **Complexity:** `Low`
- **Requirement Coverage:** REQ-20
- **Interfaces:**
  - **Produces:** `hpx::WebSocketRequestBuilder::protocols()`, `hpx::WebSocketResponse::protocol()`, `hpx::WebSocket::protocol()`
- **EvalRule:** `protocols()` no longer rejects the request; the negotiated value is observable on both the response and the connection.
- **Status:** ✅ Complete — 2 integration tests in `crates/hpx/tests/ws_yawc.rs`.

---

## Phase 5 — Verification and documentation

### Task 5.1: Documentation

- **TaskID:** `T5_1`
- **DependsOn:** `T4_2`
- **Complexity:** `Low`
- **Requirement Coverage:** all
- **EvalRule:** the crate-level proxy documentation reflects the actual default (system proxies are **enabled**), documents the new constructors and the HTTP/3 limitation, and no rustdoc link is broken.
- **Status:** ✅ Complete.

### Task 5.2: Full verification

- **TaskID:** `T5_2`
- **DependsOn:** `T5_1`
- **Complexity:** `Low`
- **Requirement Coverage:** all
- **EvalRule:** `just format`, `just lint`, `just test` and `just build-docs` all pass; the scoped mutation run over the new proxy code kills at least 90 % of the testable mutants.
- **Status:** ✅ Complete — 2565 tests, 2564 pass, 1 external-network failure (`badssl`, environment dependent, fails on the unmodified baseline too); 99/102 testable proxy mutants caught (97 %).

---

## Phase 6 — Proxy authentication

| Task | Description | Status |
| :--- | :--- | :--- |
| `T6_1` | First-class proxy auth model: `ProxyAuthMethod`, credentials on `Proxy`/`Extra`/`Intercept`, `Proxy-Authenticate` challenge parser. | ✅ Complete — 15 unit tests in `crates/hpx/src/proxy/auth.rs`. |
| `T6_2` | `407` challenge/response loop for the HTTP `CONNECT` tunnel, with response head parsing and body skipping. | ✅ Complete — 2 integration tests in `crates/hpx/tests/proxy.rs`. |
| `T6_3` | HTTP Digest (RFC 7616): MD5 / SHA-256, `qop=auth`, `cnonce`/`nc`, `opaque`, algorithm negotiation. | ✅ Complete — validated against the RFC 7616 §3.5 vector. |
| `T6_4` | NTLMv2: type-1 negotiate, type-2 challenge, type-3 response (MD4 NTLM hash, HMAC-MD5 proof). | ✅ Complete — validated against the MS-NLMP 4.2.4 vector; 3-leg integration test. |
| `T6_5` | `407` retry for plain HTTP through a proxy, with a replayable body (`ReplayBody`). | ✅ Complete — 2 integration tests. |

## Phase 7 — HTTP/3 through SOCKS5

| Task | Description | Status |
| :--- | :--- | :--- |
| `T7_1` | SOCKS5 `UDP ASSOCIATE` client handshake and datagram framing. | ✅ Complete — `crates/hpx/src/client/conn/proxy/socks_udp.rs`, 5 unit tests. |
| `T7_2` | quinn `AsyncUdpSocket` adapter over the SOCKS5 UDP relay, wired into the h3 connector. | ✅ Complete — end-to-end test against a local SOCKS5 UDP relay. |

## Phase 8 — Proxy Auto-Config

| Task | Description | Status |
| :--- | :--- | :--- |
| `T8_1` | Evaluate PAC scripts with a pure-Rust JavaScript engine and parse the `PROXY`/`SOCKS`/`DIRECT` result. | ✅ Complete — `crates/hpx/src/proxy/pac.rs`, 6 unit tests. |
| `T8_2` | Evaluate the script per request inside the proxy matcher, on a per-thread JavaScript context. | ✅ Complete — 2 integration tests. |

## Phase 9 — Remaining client gaps

| Task | Description | Status |
| :--- | :--- | :--- |
| `T9_1` | HTTP/3 request bodies. | ✅ Complete — the body was already sent; the behaviour is now covered by an end-to-end test. |
| `T9_2` | WebSocket over HTTP/2 (RFC 8441). | ✅ Complete — delivered after this spec: `H2Stream` transport in `crates/yawc/src/native/h2.rs` (`h2` feature), `InnerWs` enum in `backend_yawc.rs`, 5 yawc + 8 hpx integration tests, scoped mutation run over `h2.rs`. |
| `T9_3` | Populate the rustls handshake configuration from the builder. | ✅ Complete — the configuration is now built from the builder, and the host-name caveat is reported. |
| `T9_4` | Enforce a maximum response header size. | ✅ Complete — 2 integration tests; the limit is opt-in so the existing large-header contract is preserved. |
| `T9_5` | Asynchronous bearer token refresh. | ✅ Complete — the provider is awaited before the request; `bearer_provider` is no longer deprecated. |
