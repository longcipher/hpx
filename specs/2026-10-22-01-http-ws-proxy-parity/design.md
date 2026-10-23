# Design: HTTP/WebSocket Client Completeness + Proxy Support Parity

| Metadata | Details |
| :--- | :--- |
| **Status** | Complete |
| **Created** | 2026-10-22 |
| **Scope** | Full |
| **Feature Name** | `http-ws-proxy-parity` |
| **Spec Dir** | `specs/2026-10-22-01-http-ws-proxy-parity/` |
| **Reference** | reqwest 0.13 (`Proxy::custom`, `Proxy::from_env`), curl (`NO_PROXY`, `CURLOPT_NOPROXY`), RFC 6455 §4.1 (subprotocol negotiation), RFC 7838 (Alt-Svc), RFC 9112 §6.3 |

## Summary

The `hpx` workspace already shipped a mature HTTP stack (HTTP/1.1, HTTP/2, HTTP/3 over QUIC), three TLS backends (BoringSSL, Rustls, OpenSSL), SSE, cookies, multipart, redirect/retry/timeout/circuit-breaker middleware, and a proxy stack with HTTP CONNECT plus SOCKS4/4a/5/5h.

A full audit of the HTTP client, the WebSocket client, and the proxy subsystem — cross-checked against reqwest 0.13 and curl — found that the remaining gaps were **not** in the protocol stack but in the *policy* layer around it:

1. **Proxy API completeness** — no `Proxy::custom()` (reqwest parity), no public `Proxy::from_env()`, and no way to bypass a client-level proxy for a single request.
2. **Two system-proxy detection bugs** — the Windows `ProxyServer` registry value was used verbatim (so the common `http=…;https=…` form produced garbage), and macOS SOCKS proxy settings were never read.
3. **A security bug** — HTTP/3 (QUIC) has no proxy support, but the Alt-Svc h3 upgrade path and the explicit `Ver::Http3` path both connected *directly*, silently bypassing the configured proxy and exposing the client's IP to the origin.
4. **WebSocket proxy deficiencies** — the yawc backend rejected `socks4://`/`socks4a://` proxies (falling through to an HTTP CONNECT against a SOCKS endpoint), required an explicit proxy port, and never used TLS for `https://` proxy endpoints.
5. **No WebSocket subprotocol negotiation** — RFC 6455 §4.1 subprotocols were rejected outright by the hpx builder and never implemented in yawc, which breaks real protocols (`graphql-transport-ws`, `soap`, …).

This spec closes all five groups.

---

## Requirements

### R1 — Proxy API parity

- **[REQ-01]** `Proxy::custom(f)` shall accept a closure that maps the destination URI to an optional proxy endpoint, and shall apply the proxy's `basic_auth` / `custom_http_auth` / `custom_http_headers` to whichever endpoint the closure selects.
- **[REQ-02]** A custom proxy rule shall be comparable and hashable so that the connection-pool key stays well defined. Two separately created custom rules shall never be equal; clones of the same rule shall be.
- **[REQ-03]** `Proxy::from_env()` shall build a proxy rule from `ALL_PROXY` / `HTTP_PROXY` / `HTTPS_PROXY` (and the lowercase variants) with `NO_PROXY` applied as the exclusion list, and shall return `None` in a CGI context (`REQUEST_METHOD` present).
- **[REQ-04]** `ClientBuilder::proxies_from_env()` shall add the full set of environment rules in precedence order (HTTP, HTTPS, catch-all) so that scheme-specific variables win over `ALL_PROXY`.
- **[REQ-05]** `RequestBuilder::no_proxy()` shall bypass every configured proxy for a single request.
- **[REQ-06]** A per-request proxy rule shall be authoritative: when it does not match the request URI, the request shall be sent directly rather than falling back to the client-level proxy list.

### R2 — System proxy detection

- **[REQ-07]** The Windows `ProxyServer` value shall be parsed per scheme (`host:port`, `http=`, `https=`, `socks=`); unknown protocol keys shall be ignored.
- **[REQ-08]** The Windows `ProxyOverride` value shall be converted to a `NO_PROXY` list, keeping the `<local>` token.
- **[REQ-09]** The `NO_PROXY` matcher shall understand `<local>`: it matches every hostname that does not contain a dot.
- **[REQ-10]** The macOS system SOCKS proxy settings (`kSCPropNetProxiesSOCKSEnable` / `Proxy` / `Port`) shall be detected and applied to the catch-all matcher slot.

### R3 — HTTP/3 must not bypass the proxy

- **[REQ-11]** An Alt-Svc HTTP/3 upgrade shall be skipped when the request would be proxied; the request shall fall back to TCP, which honours the proxy.
- **[REQ-12]** An explicit HTTP/3 request for a proxied destination shall fail closed with a typed error rather than connecting directly.

### R4 — WebSocket proxy completeness

- **[REQ-13]** The yawc backend shall support SOCKS4 and SOCKS4a proxies.
- **[REQ-14]** A proxy URL without a port shall use the scheme default (80 / 443 / 1080).
- **[REQ-15]** A proxy URL with the `https` scheme shall be reached over TLS before the CONNECT tunnel is established.
- **[REQ-16]** The hpx WebSocket bridge shall map `socks4://` and `socks4a://` to the SOCKS4 proxy configuration instead of falling through to an HTTP CONNECT.

### R5 — WebSocket subprotocol negotiation

- **[REQ-17]** The client shall send the offered subprotocols in the `Sec-WebSocket-Protocol` request header, in preference order.
- **[REQ-18]** The server shall select the first server-supported subprotocol that the client also offered, and echo it in the response header.
- **[REQ-19]** A client shall fail the connection when the server selects a subprotocol it did not offer (RFC 6455 §4.1).
- **[REQ-20]** The negotiated subprotocol shall be observable through the hpx API (`WebSocketResponse::protocol`, `WebSocket::protocol`).

---

## Design Decisions

| # | Decision | Rationale |
| :-- | :--- | :--- |
| D1 | `Proxy::custom` stores the closure in a `CustomProxy` compared by the identity of an `Arc<()>` token. | A closure cannot be compared structurally. Identity comparison keeps `Hash`/`Eq` total and keeps the connection-pool key correct: two independently created custom rules never share pooled connections. |
| D2 | The env-driven rules are read through an internal `EnvSource` trait (`ProcessEnv` in production, a fixed map in tests). | Mutating the process environment in a test is `unsafe` and races with every other test in the binary. The trait makes the precedence rules testable without touching global state. |
| D3 | A per-request proxy rule replaces the client-level list instead of falling back to it. | The previous fallback silently used a *different* proxy than the one the caller asked for. An explicit request-level rule is now authoritative. |
| D4 | HTTP/3 through a proxy is not implemented; the client fails closed instead. | QUIC proxying is out of scope for curl and quiche as well. Silently connecting directly would leak the client's IP to the origin, which is a security bug, not a feature. |
| D5 | The Windows per-scheme parsing lives in `proxy/sysproxy.rs`, compiled on every platform under `cfg(test)`. | The parsing rules are the interesting part; testing them only on Windows would leave them untested in CI. |
| D6 | WebSocket subprotocol negotiation is implemented in yawc, the only WebSocket backend hpx ships. | Implementing it in hpx alone would leave the yawc crate — a published crate with its own API — without the feature. |
| D7 | Proxy auth schemes beyond Basic and raw SOCKS5 userinfo (Digest / NTLM / Negotiate) remain out of scope. | They need a challenge/response state machine that the current connect-time model does not have. Preemptive Basic covers the common case. |

---

## Implementation Map

| Area | File | Change |
| :--- | :--- | :--- |
| Custom proxy | `crates/hpx/src/proxy.rs` | `Proxy::custom`, `IntoProxy`, `CustomProxy`, env constructors, `EnvSource` |
| Matcher | `crates/hpx/src/proxy/matcher.rs` | `Custom` slot, `<local>` support, injectable env, shared URI parsing |
| System proxy | `crates/hpx/src/proxy/sysproxy.rs` (new) | Windows `ProxyServer` / `ProxyOverride` parsing, macOS SOCKS assembly |
| System proxy | `crates/hpx/src/proxy/win.rs`, `mac.rs` | Use the new helpers, read the macOS SOCKS keys |
| Request routing | `crates/hpx/src/client/layer/config/options.rs` | `ProxySelection` (Inherit / Direct / Rule) |
| Request routing | `crates/hpx/src/client/conn/connector.rs` | Dispatch on `ProxySelection` |
| Request API | `crates/hpx/src/client/request.rs` | `RequestBuilder::no_proxy()`, authoritative `RequestBuilder::proxy()` |
| Builder API | `crates/hpx/src/client/http/builder.rs` | `ClientBuilder::proxies_from_env()` |
| HTTP/3 | `crates/hpx/src/client/http/client.rs` | `HttpClient::is_proxied`, h3 upgrade skip, fail-closed `Ver::Http3` |
| HTTP/3 | `crates/hpx/src/client/http/client/extra.rs` | `proxy_selection()` accessor |
| WebSocket proxy | `crates/yawc/src/native/proxy.rs` | SOCKS4/4a, default ports, TLS to `https://` proxies |
| WebSocket proxy | `crates/hpx/src/client/ws/backend_yawc.rs` | Map `socks4`/`socks4a`, plumb the protocols |
| Subprotocols | `crates/yawc/src/native/options.rs` | `Options::protocols`, `with_protocols` |
| Subprotocols | `crates/yawc/src/native/mod.rs` | Client offer, server selection, validation, `WebSocket::protocol()` |
| Subprotocols | `crates/yawc/src/native/upgrade.rs` | Server-side selection for the axum integration |
| Subprotocols | `crates/yawc/src/lib.rs` | `InvalidSecWebSocketProtocol` error |
| Subprotocols | `crates/hpx/src/client/ws/backend_yawc.rs` | `WebSocketResponse::protocol()`, `WebSocket::protocol()` |

---

## Verification

| Check | Result |
| :--- | :--- |
| `just format` | pass |
| `just lint` (clippy `-D warnings`, cargo-shear, workspace-inheritance, AGENTS.md version check) | pass |
| `just test` (cargo nextest, workspace, all features) | 2565 tests, 2564 pass, 1 fail |
| The single failure | `hpx::badssl test_badssl_modern` — an external-network test against `badssl.com` that timed out (`Connection timed out`). It is environment dependent, passed in the preceding run of the same tree, and fails identically on the unmodified baseline. |
| `just build-docs` | pass |
| `cargo mutants -p hpx --file crates/hpx/src/proxy.rs --file crates/hpx/src/proxy/matcher.rs --file crates/hpx/src/proxy/sysproxy.rs` | 137 mutants: **99 caught**, 3 missed, 35 unviable — a 97 % kill rate over the testable mutants |
| The 3 remaining misses | `Matcher::builder -> Default::default()` (an equivalent mutant), and `Builder::from_env` / `Builder::from_system` -> `Default::default()` (only observable when the environment actually defines a proxy; the injectable `Builder::from_env_source` equivalent *is* covered) |

---

## Part 2 — Proxy Completion and Remaining Client Gaps

A second pass closed everything that Part 1 deliberately left open.

### Requirements

#### R6 — Proxy authentication

- **[REQ-21]** The client shall answer a `407 Proxy Authentication Required` challenge and retry the request, for both the HTTP `CONNECT` tunnel and a plain HTTP request through a proxy.
- **[REQ-22]** `Proxy::auth(user, pass, method)` shall configure the credentials; `ProxyAuthMethod` shall offer `Basic` (preemptive, RFC 7617), `Digest` (RFC 7616) and `Ntlm` (NTLMv2).
- **[REQ-23]** The Digest response shall follow RFC 7616 §3.4.1 with `qop=auth`, an incrementing `nc`, a per-nonce `cnonce`, `opaque` and the `MD5`, `MD5-sess`, `SHA-256` and `SHA-256-sess` algorithms.
- **[REQ-24]** The NTLM handshake shall implement the three-leg exchange (type-1 negotiate, type-2 challenge, type-3 authenticate) with an NTLMv2 response (MD4 NTLM hash over the UTF-16LE password, HMAC-MD5 proof).

#### R7 — HTTP/3 through a SOCKS5 proxy

- **[REQ-25]** The client shall establish a SOCKS5 `UDP ASSOCIATE` relay and tunnel the QUIC datagrams through it, so HTTP/3 works through a SOCKS5 proxy.
- **[REQ-26]** The QUIC endpoint shall use the relay through quinn's `AsyncUdpSocket` abstraction; the datagrams shall be wrapped and unwrapped with the SOCKS5 UDP header (RFC 1928).

#### R8 — Proxy Auto-Config

- **[REQ-27]** `Proxy::pac(script)` shall evaluate a PAC script for every request and honour the `PROXY`, `SOCKS` and `DIRECT` results.
- **[REQ-28]** The standard PAC helper functions shall be available: `isPlainHostName`, `dnsDomainIs`, `localHostOrDomain`, `isInNet`, `shExpMatch`, `weekdayRange`, `dateRange`, `timeRange`, `isResolvable`, `dnsResolve` and `myIpAddress`.

#### R9 — Remaining client gaps

- **[REQ-29]** A bearer token provider shall be awaited before the request, so tokens can be refreshed asynchronously.
- **[REQ-30]** The rustls backend shall populate its handshake configuration from the builder instead of using a default, and shall report that the host name cannot be disabled on its own.
- **[REQ-31]** `ClientBuilder::max_response_header_size` shall be enforced by the HTTP/1 parser and the HTTP/2 header-list limit.
- **[REQ-32]** The HTTP/3 path shall send the request body.

### Additional Design Decisions

| # | Decision | Rationale |
| :-- | :--- | :--- |
| D8 | Proxy credentials are stored next to the preemptive header instead of replacing it. | A challenge-based scheme needs the raw credentials; `Basic` still benefits from being sent preemptively. |
| D9 | The `407` challenge loop lives in the connect layer for `CONNECT` and in the HTTP client for plain HTTP. | Both proxy shapes need it, and the two paths have different retry mechanics (a tunnel can be re-sent on the same connection; a plain request needs a replayable body). |
| D10 | The plain-HTTP retry requires a replayable body; a streaming body is not retried. | A streamed body cannot be produced twice. The `ReplayBody` trait makes the requirement explicit. |
| D11 | NTLMv2 uses the RustCrypto `md4` crate; no DES or RC4 is needed. | NTLMv1 needs DES and RC4, NTLMv2 does not, so the dependency footprint stays at one small crate. |
| D12 | The PAC script is evaluated on a per-thread JavaScript context. | A JavaScript context is not thread safe, but the matcher is shared across threads. |
| D13 | The PAC result uses the first proxy entry. | Trying the entries in order needs a failover loop in the connect layer; the first entry covers the vast majority of PAC files. |
| D14 | The maximum response header size is opt-in (no limit by default). | The HTTP/1 read buffer already bounds the header block, and the existing contract requires a 120 KB single header to be accepted. |
| D15 | The SOCKS5 UDP relay is established lazily, on the first HTTP/3 request. | `ClientBuilder::build` is synchronous, while the relay needs an asynchronous handshake with the proxy. |

### Part 2 Verification

| Check | Result |
| :--- | :--- |
| `just format` / `just lint` / `just build-docs` | pass |
| `cargo nextest run --workspace --all-features` | 2605 tests, all pass |
| Digest reference vector | RFC 7616 §3.5: `response="6629fae49393a05397450978507c4ef1"` |
| NTLM hash reference vector | MS-NLMP 4.2.4: `MD4(UTF-16LE("Password")) = a4f49c406510bdcab6824ee7c30fd852` |
| NTLM type-3 layout | The field descriptors, the offsets they record and the blob layout are asserted against MS-NLMP 2.2.2.3 and 2.2.2.7 |
| HTTP/3 through SOCKS5 | end-to-end test against a local SOCKS5 UDP relay and a local h3 server |
| PAC | end-to-end test: the script selects the proxy for one host and `DIRECT` for another |
| Scoped mutation run over `crates/hpx/src/proxy/auth.rs` | 127 mutants: 85 caught, 31 missed, 11 unviable. The run exposed five real defects in the first NTLM implementation — the type-3 field ordering, the payload offset, the blob layout, the target-info parsing and the `NEGOTIATE_TARGET_INFO` flag value. Every one of them would have made a real proxy reject the handshake; all are fixed and now covered. |

### Not Delivered

- WebSocket over HTTP/3 *is* implemented (`H3WebSocket`). WebSocket over
  HTTP/2 (RFC 8441) was delivered after this spec was written: the H2
  transport lives in yawc (`native/h2.rs`, `h2` feature) as an `H2Stream`
  adapter with the extended-CONNECT handshake, and hpx routes
  `force_http2()` through it via an `InnerWs` enum. See `crates/yawc/tests/h2.rs`
  (5 tests) and `crates/hpx/tests/ws_h2.rs` (8 tests).
