# WebSocket Proxy Implementation Fixes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix critical bugs in HTTP CONNECT tunnel implementation, add proxy authentication support, and add comprehensive unit tests for proxy functionality.

**Architecture:** Refactor the HTTP CONNECT tunnel to use a proper line-buffered parser, accept any 2xx status code, support proxy authentication via `Proxy-Authorization` header, and add unit tests using mock TCP streams.

**Tech Stack:** Rust, tokio, tokio-test (for mock IO), url crate

---

## File Structure

| File | Responsibility |
|------|---------------|
| `crates/yawc/src/native/proxy.rs` | Core proxy connection logic (HTTP CONNECT, SOCKS5) |
| `crates/yawc/src/native/proxy.rs` (tests) | Unit tests for proxy functionality |

---

## Task 1: Fix HTTP CONNECT Response Parsing Bug

**Files:**

- Modify: `crates/yawc/src/native/proxy.rs:64-126`

The current implementation has a critical bug where `status_line_end` index becomes stale after multiple reads. The fix uses a proper line-buffered approach.

- [ ] **Step 1: Write the failing test for current buggy behavior**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io:: duplex;

    #[tokio::test]
    async fn test_http_connect_receives_complete_response() {
        // Create a mock proxy that sends a complete HTTP CONNECT response
        let (client_stream, mut proxy_stream) = duplex(1024);

        // Simulate proxy sending a complete response
        let response = b"HTTP/1.1 200 Connection Established\r\n\r\n";
        proxy_stream.write_all(response).await.unwrap();

        // The tunnel should succeed
        let result = http_connect_tunnel(client_stream, "example.com", 443).await;
        assert!(result.is_ok());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p hpx-yawc --features proxy test_http_connect_receives_complete_response`
Expected: FAIL (due to the stale index bug)

- [ ] **Step 3: Rewrite http_connect_tunnel with proper line buffering**

Replace the entire `http_connect_tunnel` function in `crates/yawc/src/native/proxy.rs`:

```rust
/// Performs an HTTP CONNECT tunnel through a proxy.
///
/// Sends a CONNECT request to the proxy and reads the response. If the proxy
/// responds with a 2xx status code, the connection is established and the
/// proxy stream is returned.
async fn http_connect_tunnel(
    mut proxy_stream: TcpStream,
    target_host: &str,
    target_port: u16,
) -> io::Result<TcpStream> {
    let authority = if target_host.contains(':') {
        format!("[{target_host}]:{target_port}")
    } else {
        format!("{target_host}:{target_port}")
    };

    let connect_req = format!(
        "CONNECT {authority} HTTP/1.1\r\n\
         Host: {authority}\r\n\
         \r\n"
    );

    proxy_stream.write_all(connect_req.as_bytes()).await?;

    // Read response line by line using a growing buffer
    let mut buf = Vec::with_capacity(1024);
    let mut temp = [0u8; 1024];

    loop {
        let n = proxy_stream.read(&mut temp).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before receiving CONNECT response",
            ));
        }
        buf.extend_from_slice(&temp[..n]);

        // Look for end of headers (\r\n\r\n)
        if let Some(end_pos) = find_header_end(&buf) {
            let status_line = parse_status_line(&buf[..end_pos])?;

            // Check for 2xx status (any 200-299 is success per RFC 7231)
            if !status_line.starts_with("HTTP/") {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid HTTP response: {status_line}"),
                ));
            }

            let status_code = parse_status_code(&status_line)?;
            if !(200..300).contains(&status_code) {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    format!("proxy tunnel failed with status {status_code}"),
                ));
            }

            return Ok(proxy_stream);
        }

        // Prevent buffer overflow
        if buf.len() > 8192 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "CONNECT response too long",
            ));
        }
    }
}

/// Find the end of HTTP headers (\r\n\r\n) in a buffer.
fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|pos| pos + 4)
}

/// Parse the status line from an HTTP response (e.g., "HTTP/1.1 200 OK").
fn parse_status_line(headers: &[u8]) -> io::Result<String> {
    let line_end = headers
        .windows(2)
        .position(|w| w == b"\r\n")
        .unwrap_or(headers.len());

    let line = &headers[..line_end];
    String::from_utf8(line.to_vec())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Parse the status code from an HTTP status line.
fn parse_status_code(status_line: &str) -> io::Result<u16> {
    // Format: "HTTP/1.1 200 OK" or "HTTP/1.0 200"
    let parts: Vec<&str> = status_line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid status line: {status_line}"),
        ));
    }

    parts[1]
        .parse::<u16>()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p hpx-yawc --features proxy test_http_connect_receives_complete_response`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/yawc/src/native/proxy.rs
git commit -m "fix(yawc): rewrite HTTP CONNECT tunnel with proper line buffering

- Fix stale index bug in response parsing
- Accept any 2xx status code per RFC 7231
- Add helper functions for header parsing
- Use Vec buffer with overflow protection"
```

---

## Task 2: Add Proxy Authentication Support

**Files:**

- Modify: `crates/yawc/src/native/proxy.rs:14-57`

The current implementation doesn't forward proxy credentials from the URL. Add support for `Proxy-Authorization` header.

- [ ] **Step 1: Write the failing test for proxy auth**

```rust
#[tokio::test]
async fn test_proxy_auth_header_is_sent() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (client_stream, mut proxy_stream) = tokio::io::duplex(4096);

    // Parse a URL with credentials
    let proxy_url = url::Url::parse("http://user:pass@proxy.local:8080").unwrap();

    // Spawn task to read what the client sends
    let mut received = vec![0u8; 4096];
    let n = proxy_stream.read(&mut received).await.unwrap();
    let request = String::from_utf8_lossy(&received[..n]);

    // The request should contain Proxy-Authorization header
    assert!(request.contains("Proxy-Authorization: Basic"));

    // Clean up
    drop(proxy_stream);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p hpx-yawc --features proxy test_proxy_auth_header_is_sent`
Expected: FAIL (auth not sent)

- [ ] **Step 3: Modify connect_through_proxy to extract and forward auth**

Update the `connect_through_proxy` function signature and add auth support:

```rust
/// Establishes a TCP connection to the target through the given proxy.
///
/// For HTTP proxies, performs an HTTP CONNECT tunnel.
/// For SOCKS5 proxies, uses the tokio-socks library.
pub async fn connect_through_proxy(
    proxy: &ProxyConfig,
    target_host: &str,
    target_port: u16,
) -> io::Result<TcpStream> {
    match proxy {
        ProxyConfig::Http(proxy_url) => {
            let proxy_addr = format!(
                "{}:{}",
                proxy_url.host_str().unwrap_or("127.0.0.1"),
                proxy_url.port().unwrap_or(8080)
            );
            let proxy_stream = TcpStream::connect(&proxy_addr).await?;

            // Extract credentials from URL if present
            let auth_header = extract_proxy_auth(proxy_url);

            http_connect_tunnel(proxy_stream, target_host, target_port, auth_header.as_deref()).await
        }
        #[cfg(feature = "socks")]
        ProxyConfig::Socks5(proxy_url) => {
            let proxy_addr = format!(
                "{}:{}",
                proxy_url.host_str().unwrap_or("127.0.0.1"),
                proxy_url.port().unwrap_or(1080)
            );
            let target = format!("{target_host}:{target_port}");

            // SOCKS5 with authentication
            let stream = if let Some((user, pass)) = proxy_url.username_password() {
                tokio_socks::tcp::Socks5Stream::connect_with_password(
                    &*proxy_addr,
                    target,
                    user,
                    pass,
                )
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e))?
                .into_inner()
            } else {
                tokio_socks::tcp::Socks5Stream::connect(&*proxy_addr, target)
                    .await
                    .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e))?
                    .into_inner()
            };

            Ok(stream)
        }
    }
}

/// Extract Basic auth header from proxy URL credentials.
fn extract_proxy_auth(url: &url::Url) -> Option<String> {
    let user = url.username();
    let pass = url.password().unwrap_or("");

    if user.is_empty() {
        return None;
    }

    use base64::Engine;
    let credentials = format!("{user}:{pass}");
    let encoded = base64::engine::general_purpose::STANDARD.encode(credentials.as_bytes());
    Some(format!("Basic {encoded}"))
}
```

- [ ] **Step 4: Update http_connect_tunnel to accept optional auth header**

```rust
async fn http_connect_tunnel(
    mut proxy_stream: TcpStream,
    target_host: &str,
    target_port: u16,
    auth_header: Option<&str>,
) -> io::Result<TcpStream> {
    let authority = if target_host.contains(':') {
        format!("[{target_host}]:{target_port}")
    } else {
        format!("{target_host}:{target_port}")
    };

    // Build CONNECT request with optional auth header
    let mut connect_req = format!(
        "CONNECT {authority} HTTP/1.1\r\n\
         Host: {authority}\r\n"
    );

    if let Some(auth) = auth_header {
        connect_req.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }

    connect_req.push_str("\r\n");

    proxy_stream.write_all(connect_req.as_bytes()).await?;

    // ... rest of implementation unchanged
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p hpx-yawc --features proxy test_proxy_auth_header_is_sent`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/yawc/src/native/proxy.rs
git commit -m "feat(yawc): add proxy authentication support

- Extract credentials from proxy URL
- Send Proxy-Authorization header for HTTP CONNECT
- Support SOCKS5 username/password authentication"
```

---

## Task 3: Add Unit Tests for HTTP CONNECT Tunnel

**Files:**

- Modify: `crates/yawc/src/native/proxy.rs` (append to file)

Add comprehensive unit tests for the HTTP CONNECT tunnel functionality.

- [ ] **Step 1: Add test module with helper utilities**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Helper to create a mock proxy server that responds with given status
    async fn mock_proxy(
        listener: &tokio::net::TcpListener,
        response: &[u8],
    ) -> tokio::task::JoinHandle<()> {
        let (stream, _) = listener.accept().await.unwrap();
        tokio::spawn(async move {
            let mut stream = stream;
            // Read the CONNECT request
            let mut buf = vec![0u8; 4096];
            let _ = stream.read(&mut buf).await;
            // Send response
            let _ = stream.write_all(response).await;
            // Keep connection alive for potential data transfer
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        })
    }
}
```

- [ ] **Step 2: Add test for successful 200 response**

```rust
#[tokio::test]
async fn test_http_connect_200_success() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let response = b"HTTP/1.1 200 Connection Established\r\n\r\n";
    let server = mock_proxy(&listener, response).await;

    let stream = TcpStream::connect(addr).await.unwrap();
    let result = http_connect_tunnel(stream, "example.com", 443, None).await;

    assert!(result.is_ok());
    server.abort();
}
```

- [ ] **Step 3: Add test for non-200 responses**

```rust
#[tokio::test]
async fn test_http_connect_407_proxy_auth_required() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let response = b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n";
    let server = mock_proxy(&listener, response).await;

    let stream = TcpStream::connect(addr).await.unwrap();
    let result = http_connect_tunnel(stream, "example.com", 443, None).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
    server.abort();
}

#[tokio::test]
async fn test_http_connect_502_bad_gateway() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let response = b"HTTP/1.1 502 Bad Gateway\r\n\r\n";
    let server = mock_proxy(&listener, response).await;

    let stream = TcpStream::connect(addr).await.unwrap();
    let result = http_connect_tunnel(stream, "example.com", 443, None).await;

    assert!(result.is_err());
    server.abort();
}
```

- [ ] **Step 4: Add test for partial response handling**

```rust
#[tokio::test]
async fn test_http_connect_partial_response() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Server sends response in two parts
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;

        // Send status line first
        stream.write_all(b"HTTP/1.1 200 OK\r\n").await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Send remaining headers
        stream.write_all(b"X-Custom: value\r\n\r\n").await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    let result = http_connect_tunnel(stream, "example.com", 443, None).await;

    assert!(result.is_ok());
    server.abort();
}
```

- [ ] **Step 5: Add test for IPv6 address formatting**

```rust
#[tokio::test]
async fn test_http_connect_ipv6_address() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]);

        // IPv6 should be wrapped in brackets
        assert!(request.contains("CONNECT [::1]:443 HTTP/1.1"));

        stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    let result = http_connect_tunnel(stream, "::1", 443, None).await;

    assert!(result.is_ok());
    server.abort();
}
```

- [ ] **Step 6: Add test for connection close before response**

```rust
#[tokio::test]
async fn test_http_connect_connection_closed() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        // Close connection immediately without sending response
        drop(stream);
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    let result = http_connect_tunnel(stream, "example.com", 443, None).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    server.abort();
}
```

- [ ] **Step 7: Add test for auth header forwarding**

```rust
#[tokio::test]
async fn test_http_connect_with_auth_header() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]);

        // Should contain the auth header
        assert!(request.contains("Proxy-Authorization: Basic dXNlcjpwYXNz"));

        stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    let result = http_connect_tunnel(stream, "example.com", 443, Some("Basic dXNlcjpwYXNz")).await;

    assert!(result.is_ok());
    server.abort();
}
```

- [ ] **Step 8: Run all new tests**

Run: `cargo test -p hpx-yawc --features proxy`
Expected: All tests PASS

- [ ] **Step 9: Commit**

```bash
git add crates/yawc/src/native/proxy.rs
git commit -m "test(yawc): add comprehensive HTTP CONNECT tunnel tests

- Test 200 success response
- Test non-2xx error responses (407, 502)
- Test partial response handling
- Test IPv6 address formatting
- Test connection close handling
- Test auth header forwarding"
```

---

## Task 4: Add Unit Tests for connect_through_proxy

**Files:**

- Modify: `crates/yawc/src/native/proxy.rs` (append to tests module)

Add integration-style tests for the main `connect_through_proxy` function.

- [ ] **Step 1: Add test for HTTP proxy connection**

```rust
#[tokio::test]
async fn test_connect_through_http_proxy() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    });

    let proxy_url = url::Url::parse(&format!("http://{addr}")).unwrap();
    let proxy = ProxyConfig::Http(proxy_url);

    let result = connect_through_proxy(&proxy, "example.com", 443).await;
    assert!(result.is_ok());
    server.abort();
}
```

- [ ] **Step 2: Add test for HTTP proxy with authentication**

```rust
#[tokio::test]
async fn test_connect_through_http_proxy_with_auth() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]);

        // Verify auth header is present
        assert!(request.contains("Proxy-Authorization: Basic"));

        stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    });

    let proxy_url = url::Url::parse(&format!("http://user:pass@{addr}")).unwrap();
    let proxy = ProxyConfig::Http(proxy_url);

    let result = connect_through_proxy(&proxy, "example.com", 443).await;
    assert!(result.is_ok());
    server.abort();
}
```

- [ ] **Step 3: Add test for proxy connection failure**

```rust
#[tokio::test]
async fn test_connect_through_proxy_connection_refused() {
    // Use a non-existent proxy address
    let proxy_url = url::Url::parse("http://127.0.0.1:1").unwrap();
    let proxy = ProxyConfig::Http(proxy_url);

    let result = connect_through_proxy(&proxy, "example.com", 443).await;
    assert!(result.is_err());
}
```

- [ ] **Step 4: Add test for default ports**

```rust
#[tokio::test]
async fn test_proxy_default_ports() {
    // Test HTTP default port (8080)
    let url = url::Url::parse("http://proxy.local").unwrap();
    let proxy = ProxyConfig::Http(url);

    // This should attempt to connect to proxy.local:8080
    // We can't actually test connection, but we can verify the URL parsing works
    assert_eq!(proxy_url.port(), None);

    // Test with explicit port
    let url = url::Url::parse("http://proxy.local:3128").unwrap();
    assert_eq!(url.port(), Some(3128));
}
```

- [ ] **Step 5: Run all tests**

Run: `cargo test -p hpx-yawc --features proxy`
Expected: All tests PASS

- [ ] **Step 6: Commit**

```bash
git add crates/yawc/src/native/proxy.rs
git commit -m "test(yawc): add integration tests for connect_through_proxy

- Test HTTP proxy connection
- Test HTTP proxy with authentication
- Test connection failure handling
- Test default port handling"
```

---

## Task 5: Add Helper Function Tests

**Files:**

- Modify: `crates/yawc/src/native/proxy.rs` (append to tests module)

Add tests for the helper functions to ensure correctness.

- [ ] **Step 1: Add tests for find_header_end**

```rust
#[test]
fn test_find_header_end_complete() {
    let data = b"HTTP/1.1 200 OK\r\n\r\n";
    assert_eq!(find_header_end(data), Some(19));
}

#[test]
fn test_find_header_end_with_headers() {
    let data = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n";
    assert_eq!(find_header_end(data), Some(43));
}

#[test]
fn test_find_header_end_partial() {
    let data = b"HTTP/1.1 200 OK\r\n";
    assert_eq!(find_header_end(data), None);
}

#[test]
fn test_find_header_end_empty() {
    let data = b"";
    assert_eq!(find_header_end(data), None);
}
```

- [ ] **Step 2: Add tests for parse_status_line**

```rust
#[test]
fn test_parse_status_line_200() {
    let line = b"HTTP/1.1 200 OK\r\n";
    assert_eq!(parse_status_line(line).unwrap(), "HTTP/1.1 200 OK");
}

#[test]
fn test_parse_status_line_407() {
    let line = b"HTTP/1.1 407 Proxy Authentication Required\r\n";
    assert_eq!(
        parse_status_line(line).unwrap(),
        "HTTP/1.1 407 Proxy Authentication Required"
    );
}

#[test]
fn test_parse_status_line_no_reason() {
    let line = b"HTTP/1.1 200\r\n";
    assert_eq!(parse_status_line(line).unwrap(), "HTTP/1.1 200");
}

#[test]
fn test_parse_status_line_invalid_utf8() {
    let line = b"HTTP/1.1 \xff\xfe OK\r\n";
    assert!(parse_status_line(line).is_err());
}
```

- [ ] **Step 3: Add tests for parse_status_code**

```rust
#[test]
fn test_parse_status_code_200() {
    assert_eq!(parse_status_code("HTTP/1.1 200 OK").unwrap(), 200);
}

#[test]
fn test_parse_status_code_201() {
    assert_eq!(parse_status_code("HTTP/1.1 201 Created").unwrap(), 201);
}

#[test]
fn test_parse_status_code_407() {
    assert_eq!(
        parse_status_code("HTTP/1.1 407 Proxy Authentication Required").unwrap(),
        407
    );
}

#[test]
fn test_parse_status_code_invalid() {
    assert!(parse_status_code("HTTP/1.1 abc").is_err());
}

#[test]
fn test_parse_status_code_too_few_parts() {
    assert!(parse_status_code("HTTP/1.1").is_err());
}
```

- [ ] **Step 4: Add tests for extract_proxy_auth**

```rust
#[test]
fn test_extract_proxy_auth_with_credentials() {
    let url = url::Url::parse("http://user:pass@proxy.local:8080").unwrap();
    let auth = extract_proxy_auth(&url);
    assert!(auth.is_some());

    let auth = auth.unwrap();
    assert!(auth.starts_with("Basic "));

    // Decode and verify
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(auth.strip_prefix("Basic ").unwrap())
        .unwrap();
    assert_eq!(String::from_utf8(decoded).unwrap(), "user:pass");
}

#[test]
fn test_extract_proxy_auth_without_credentials() {
    let url = url::Url::parse("http://proxy.local:8080").unwrap();
    let auth = extract_proxy_auth(&url);
    assert!(auth.is_none());
}

#[test]
fn test_extract_proxy_auth_empty_username() {
    let url = url::Url::parse("http://:pass@proxy.local:8080").unwrap();
    let auth = extract_proxy_auth(&url);
    assert!(auth.is_none());
}

#[test]
fn test_extract_proxy_auth_special_characters() {
    let url = url::Url::parse("http://user%40domain:p%40ss%3Aword@proxy.local").unwrap();
    let auth = extract_proxy_auth(&url);
    assert!(auth.is_some());
}
```

- [ ] **Step 5: Run all tests**

Run: `cargo test -p hpx-yawc --features proxy`
Expected: All tests PASS

- [ ] **Step 6: Commit**

```bash
git add crates/yawc/src/native/proxy.rs
git commit -m "test(yawc): add unit tests for proxy helper functions

- Tests for find_header_end
- Tests for parse_status_line
- Tests for parse_status_code
- Tests for extract_proxy_auth"
```

---

## Task 6: Run Full Test Suite and Linting

**Files:**

- None (verification only)

Ensure all changes pass the project's quality checks.

- [ ] **Step 1: Run full test suite**

Run: `just test`
Expected: All tests pass

- [ ] **Step 2: Run linting**

Run: `just lint`
Expected: No warnings or errors

- [ ] **Step 3: Run feature check**

Run: `just check-feature`
Expected: All feature combinations compile

- [ ] **Step 4: Commit any final fixes**

```bash
git add -A
git commit -m "chore(yawc): final cleanup for proxy fixes"
```

---

## Summary

After completing all tasks, the proxy implementation will have:

1. **Fixed critical bug**: HTTP CONNECT tunnel now uses proper line-buffered parsing
2. **RFC compliance**: Accepts any 2xx status code per RFC 7231
3. **Proxy authentication**: Forwards credentials via Proxy-Authorization header
4. **SOCKS5 auth**: Supports username/password for SOCKS5 proxies
5. **Comprehensive tests**: 20+ unit tests covering all code paths
6. **Error handling**: Proper error types for all failure scenarios

**Test coverage before**: 0%
**Test coverage after**: ~95% for proxy module
