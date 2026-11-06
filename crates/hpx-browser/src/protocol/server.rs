//! WebSocket CDP server — accepts connections and dispatches to CdpSession.

use std::{collections::HashSet, sync::Arc};

use futures_util::{SinkExt, StreamExt};
use hpx_yawc::{Frame, frame::OpCode};
use tokio::net::TcpListener;

use crate::{
    protocol::{session::CdpSession, types::*},
    resource_loader::ResourceType,
    stealth::StealthProfile,
};

/// Serializes V8-critical page operations across all connections on the same thread.
///
/// V8's HandleScope stack is thread-local. When multiple isolates are active
/// concurrently on the same thread (as happens with `spawn_local` tasks in the
/// CDP server's `LocalSet`), they corrupt each other's HandleScope state,
/// triggering `Cannot create a handle without a HandleScope` fatal errors.
/// This mutex ensures only one isolate is active at a time.
///
/// The lock is held across `.await` points (inside `handle_request`), so it
/// must be `tokio::sync::Mutex`, not `parking_lot::Mutex`. It is owned per
/// `CdpServer` instance (rather than a process-global `static`) so the lock
/// lifetime is tied to the server and a future per-isolate refinement only needs
/// to swap this single `Arc` for a per-`Page` lock.
type V8Serialize = Arc<tokio::sync::Mutex<()>>;

/// Per-server network / resource-loading options applied to every page.
#[derive(Clone, Default)]
pub struct ServerOptions {
    /// Outbound proxy for navigations and subresource fetches.
    pub proxy: Option<hpx::Proxy>,
    /// Resource types that must not be fetched (images, stylesheets, …).
    pub block_types: HashSet<ResourceType>,
}

/// A running CDP server. Stops when dropped.
pub struct CdpServer {
    port: u16,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    v8_serialize: V8Serialize,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl CdpServer {
    /// Start a CDP WebSocket server on `127.0.0.1:{port}`.
    /// Each WebSocket connection gets its own isolated [`Page`](crate::page::Page).
    pub fn start(
        html: &str,
        port: u16,
        stealth: bool,
        profile: Option<StealthProfile>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::start_with_options(html, port, stealth, profile, ServerOptions::default())
    }

    /// Start a CDP WebSocket server with proxy / resource-block options.
    pub fn start_with_options(
        html: &str,
        port: u16,
        stealth: bool,
        profile: Option<StealthProfile>,
        options: ServerOptions,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let html = html.to_string();
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();
        let v8_serialize = Arc::new(tokio::sync::Mutex::new(()));
        let v8_serialize_clone = v8_serialize.clone();
        let (port_tx, port_rx) = std::sync::mpsc::channel();

        let thread = std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!("CdpServer: failed to build tokio runtime: {}", e);
                    return;
                }
            };

            let local = tokio::task::LocalSet::new();
            rt.block_on(local.run_until(async move {
                let listener = match TcpListener::bind(format!("127.0.0.1:{}", port)).await {
                    Ok(l) => l,
                    Err(e) => {
                        tracing::error!("CdpServer: failed to bind CDP port: {}", e);
                        port_tx.send(0).ok();
                        return;
                    }
                };
                let actual_port = match listener.local_addr() {
                    Ok(addr) => addr.port(),
                    Err(e) => {
                        tracing::error!("CdpServer: failed to read local addr: {}", e);
                        port_tx.send(0).ok();
                        return;
                    }
                };
                port_tx.send(actual_port).ok();

                accept_loop(
                    listener,
                    html,
                    stealth,
                    profile,
                    options,
                    shutdown_clone,
                    v8_serialize_clone,
                )
                .await;
            }));
        });

        let actual_port = port_rx.recv().map_err(|e| {
            Box::new(std::io::Error::other(format!(
                "server thread failed to start: {}",
                e
            ))) as Box<dyn std::error::Error + Send + Sync>
        })?;

        if actual_port == 0 {
            return Err(Box::new(std::io::Error::other(
                "server initialization failed",
            )));
        }

        Ok(Self {
            port: actual_port,
            shutdown,
            v8_serialize,
            thread: Some(thread),
        })
    }

    /// Start on port 0 (OS-assigned) — useful for tests.
    pub fn start_ephemeral(html: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::start(html, 0, false, None)
    }

    /// The port the server is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// WebSocket URL for CDP clients.
    pub fn ws_url(&self) -> String {
        format!("ws://127.0.0.1:{}", self.port)
    }

    /// Signal the server to stop accepting new connections and begin a graceful
    /// shutdown. Already-established WebSocket sessions are allowed to finish; the
    /// server thread joins when the returned guard is dropped (see [`Drop`]).
    pub fn shutdown(&self) {
        self.shutdown
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Drop for CdpServer {
    fn drop(&mut self) {
        self.shutdown
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

async fn accept_loop(
    listener: TcpListener,
    html: String,
    stealth: bool,
    profile: Option<StealthProfile>,
    options: ServerOptions,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    v8_serialize: V8Serialize,
) {
    let html = Arc::new(html);
    loop {
        if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        let accept =
            tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept()).await;

        match accept {
            Ok(Ok((stream, addr))) => {
                let html = html.clone();
                let profile = profile.clone();
                let options = options.clone();
                let v8_serialize = v8_serialize.clone();
                tokio::task::spawn_local(async move {
                    if let Err(e) =
                        handle_connection(stream, &html, stealth, profile, options, v8_serialize)
                            .await
                    {
                        tracing::warn!("CDP connection from {} error: {}", addr, e);
                    }
                });
            }
            Ok(Err(e)) => {
                tracing::warn!("CDP accept error: {}", e);
            }
            Err(_) => {} // timeout — check shutdown again
        }
    }
}

fn bad_request(msg: &str) -> hyper::Response<String> {
    hyper::Response::builder()
        .status(hyper::StatusCode::BAD_REQUEST)
        .body(msg.to_string())
        .unwrap_or_else(|_| hyper::Response::new(String::new()))
}

fn forbidden(msg: &str) -> hyper::Response<String> {
    let mut response = hyper::Response::new(msg.to_string());
    *response.status_mut() = hyper::StatusCode::FORBIDDEN;
    response
}

/// Whether a `Host` header value names the loopback interface: `localhost`,
/// an IPv4 loopback address or a bracketed IPv6 loopback address, each with
/// an optional port.
///
/// The server binds `127.0.0.1`, yet any web page the user visits can still
/// reach it through DNS rebinding: an attacker-controlled name is pointed at
/// `127.0.0.1`, and the browser then sends that name in `Host`.
fn is_loopback_host(host: &str) -> bool {
    // Split off an optional port. A `]` after the last colon means that
    // colon sits inside a bracketed IPv6 literal, not before a port.
    let name = match host.rsplit_once(':') {
        Some((name, port)) if !port.contains(']') => {
            if port.parse::<u16>().is_err() {
                return false;
            }
            name
        }
        _ => host,
    };
    if let Some(ipv6) = name.strip_prefix('[').and_then(|n| n.strip_suffix(']')) {
        return ipv6
            .parse::<std::net::Ipv6Addr>()
            .is_ok_and(|ip| ip.is_loopback());
    }
    name.eq_ignore_ascii_case("localhost")
        || name
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    html: &str,
    stealth: bool,
    profile: Option<StealthProfile>,
    options: ServerOptions,
    v8_serialize: V8Serialize,
) -> Result<(), Box<dyn std::error::Error>> {
    use hyper_util::rt::TokioIo;

    // Peek to check for HTTP vs WebSocket
    let mut buf = [0u8; 512];
    let n = stream.peek(&mut buf).await?;
    let peek = String::from_utf8_lossy(&buf[..n]);

    if peek.starts_with("GET ") && !peek.contains("Upgrade:") && !peek.contains("upgrade:") {
        return handle_http(stream).await;
    }

    // Use hyper to handle the upgrade to WebSocket (HTTP/1 + HTTP/2)
    let io = TokioIo::new(stream);
    let html = html.to_string();

    let service =
        hyper::service::service_fn(move |mut req: hyper::Request<hyper::body::Incoming>| {
            let html = html.clone();
            let profile = profile.clone();
            let options = options.clone();
            // Clone the shared V8 serialize lock per call so the `Fn` service
            // closure stays callable multiple times (each connection gets its own
            // `Arc` handle; the inner `async move` block takes ownership of it).
            let v8_serialize = v8_serialize.clone();
            async move {
                // Refuse handshakes a web page could have sent. Browsers put
                // `Origin` on every WebSocket handshake while CDP clients such
                // as Puppeteer and Playwright send none (Chrome refuses such
                // handshakes too unless `--remote-allow-origins` lists the
                // origin), and a DNS-rebound page arrives with its own `Host`.
                if req.headers().contains_key(hyper::header::ORIGIN) {
                    return Ok::<_, std::convert::Infallible>(forbidden(
                        "WebSocket handshakes with an Origin header are not allowed",
                    ));
                }
                if !req
                    .headers()
                    .get(hyper::header::HOST)
                    .and_then(|host| host.to_str().ok())
                    .is_some_and(is_loopback_host)
                {
                    return Ok(forbidden("Host must name a loopback address"));
                }

                if req
                    .headers()
                    .get(hyper::header::SEC_WEBSOCKET_KEY)
                    .is_none()
                {
                    return Ok::<_, std::convert::Infallible>(bad_request(
                        "missing Sec-WebSocket-Key",
                    ));
                }

                if req
                    .headers()
                    .get(hyper::header::SEC_WEBSOCKET_VERSION)
                    .map(|v| v.as_bytes())
                    != Some(b"13")
                {
                    return Ok(bad_request("invalid Sec-WebSocket-Version"));
                }

                let (response, upgrade_fut) = match hpx_yawc::WebSocket::upgrade(&mut req) {
                    Ok(r) => r,
                    Err(e) => {
                        return Ok(bad_request(&format!("upgrade error: {e}")));
                    }
                };

                let v8_serialize = v8_serialize.clone();
                tokio::task::spawn_local(async move {
                    match upgrade_fut.await {
                        Ok(ws) => {
                            if let Err(e) = handle_ws_connection(
                                ws,
                                &html,
                                stealth,
                                profile,
                                options,
                                v8_serialize,
                            )
                            .await
                            {
                                tracing::warn!("CDP websocket error: {}", e);
                            }
                        }
                        Err(e) => tracing::warn!("CDP upgrade error: {}", e),
                    }
                });

                Ok(response.map(|_| String::new()))
            }
        });

    hyper::server::conn::http1::Builder::new()
        .serve_connection(io, service)
        .with_upgrades()
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    Ok(())
}

async fn handle_ws_connection(
    mut ws: hpx_yawc::WebSocket<hpx_yawc::HttpStream>,
    html: &str,
    stealth: bool,
    profile: Option<StealthProfile>,
    options: ServerOptions,
    v8_serialize: V8Serialize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut page = crate::page::Page::from_html(html, stealth)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    page.set_proxy(options.proxy.clone());
    page.set_subresource_block_types(options.block_types.clone());
    if let Some(prof) = profile {
        #[cfg(feature = "v8")]
        page.set_profile(prof);
        #[cfg(not(feature = "v8"))]
        let _ = prof;
    }
    let mut session = CdpSession::new();

    while let Some(frame) = ws.next().await {
        match frame.opcode() {
            OpCode::Text => {
                let text = frame.as_str()?;
                let req: CdpRequest = match serde_json::from_str(text) {
                    Ok(r) => r,
                    Err(e) => {
                        let err_msg = format!(
                            r#"{{"error":{{"code":-32700,"message":"Parse error: {}"}}}}"#,
                            e.to_string().replace('"', "'")
                        );
                        ws.send(Frame::text(err_msg)).await?;
                        continue;
                    }
                };

                // Serialize V8-critical CDP request handling across all connections.
                // The lock has no borrow on `session`, so the subsequent
                // `&mut session` call to `handle_request` compiles cleanly.
                let (response, events) = {
                    let _v8_lock = v8_serialize.lock().await;
                    session.handle_request(&mut page, &req).await
                };

                for event in events {
                    ws.send(Frame::text(to_json(&event))).await?;
                }
                ws.send(Frame::text(response)).await?;
            }
            OpCode::Close => break,
            _ => {}
        }
    }

    Ok(())
}

async fn handle_http(stream: tokio::net::TcpStream) -> Result<(), Box<dyn std::error::Error>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = stream;
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let path = request
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .unwrap_or("/");

    // `/json/version` and `/json` hand out the WebSocket URL, so a DNS-rebound
    // page must not read them either (see `is_loopback_host`). A missing `Host`
    // is refused as well: HTTP/1.1 requires it and every browser sends it.
    let host = request
        .lines()
        .skip(1)
        .take_while(|line| !line.is_empty())
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("host").then_some(value.trim())
        });
    if !host.is_some_and(is_loopback_host) {
        let resp = "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        stream.write_all(resp.as_bytes()).await?;
        return Ok(());
    }

    let addr = stream.local_addr()?;
    let ws_url = format!("ws://127.0.0.1:{}", addr.port());

    let body = match path {
        "/hello" => {
            let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\nConnection: close\r\n\r\nHello";
            stream.write_all(resp.as_bytes()).await?;
            return Ok(());
        }
        "/json/version" => serde_json::json!({
            "Browser": "hpx-browser/0.1.0",
            "Protocol-Version": "1.3",
            "User-Agent": "hpx-browser/0.1.0",
            "V8-Version": "12.x",
            "WebKit-Version": "0",
            "webSocketDebuggerUrl": ws_url,
        })
        .to_string(),
        "/json" | "/json/list" => {
            // ponytail: per-connection pages, no shared state to list
            serde_json::json!([{
                "description": "",
                "devtoolsFrontendUrl": "",
                "id": "page-1",
                "title": "",
                "type": "page",
                "url": "about:blank",
                "webSocketDebuggerUrl": ws_url,
            }])
            .to_string()
        }
        _ => {
            let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream.write_all(resp.as_bytes()).await?;
            return Ok(());
        }
    };

    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(resp.as_bytes()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[test]
    fn server_starts_and_stops() {
        let server = CdpServer::start_ephemeral("<html><body>Hello</body></html>").unwrap();
        assert!(server.port() > 0);
        assert!(server.ws_url().contains("127.0.0.1"));
        drop(server);
    }

    #[test]
    fn cdp_server_accepts_stealth_param() {
        let server = CdpServer::start("<html></html>", 0, true, None).expect("should start");
        assert!(server.port() > 0);
        drop(server);
    }

    #[test]
    fn loopback_host_accepts_only_loopback_names() {
        for host in [
            "localhost",
            "LocalHost:9222",
            "127.0.0.1",
            "127.0.0.1:9222",
            "127.1.2.3",
            "[::1]",
            "[::1]:9222",
        ] {
            assert!(is_loopback_host(host), "{host} should be accepted");
        }
        for host in [
            "",
            "evil.example",
            "evil.example:9222",
            "localhost.evil.example",
            "127.0.0.1.nip.io",
            "192.168.1.10:9222",
            "::1",
            "[::2]:9222",
            "[::1]x",
            "localhost:",
            "localhost:http",
        ] {
            assert!(!is_loopback_host(host), "{host} should be refused");
        }
    }

    /// Send one raw request and return what the server answers first.
    async fn raw_response(port: u16, request: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }

    fn ws_upgrade_request(host: &str, origin: Option<&str>) -> String {
        let origin = origin.map_or_else(String::new, |o| format!("Origin: {o}\r\n"));
        format!(
            "GET / HTTP/1.1\r\nHost: {host}\r\n{origin}Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
    }

    #[tokio::test]
    async fn ws_upgrade_from_a_web_page_is_forbidden() {
        // Regression: the upgrade never looked at `Origin` or `Host`, so any
        // page the user visited could drive the browser over CDP.
        let server = CdpServer::start_ephemeral("<html></html>").unwrap();
        let port = server.port();

        let cross_origin =
            ws_upgrade_request(&format!("127.0.0.1:{port}"), Some("https://evil.example"));
        let response = raw_response(port, &cross_origin).await;
        assert!(response.starts_with("HTTP/1.1 403"), "{response}");

        let rebound = ws_upgrade_request(&format!("evil.example:{port}"), None);
        let response = raw_response(port, &rebound).await;
        assert!(response.starts_with("HTTP/1.1 403"), "{response}");

        let local = ws_upgrade_request(&format!("localhost:{port}"), None);
        let response = raw_response(port, &local).await;
        assert!(response.starts_with("HTTP/1.1 101"), "{response}");
    }

    #[tokio::test]
    async fn json_endpoints_refuse_a_non_loopback_host() {
        // Regression: a DNS-rebound page could read `webSocketDebuggerUrl`.
        let server = CdpServer::start_ephemeral("<html></html>").unwrap();
        let port = server.port();
        let request = |host: &str| {
            format!("GET /json/version HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
        };

        let response = raw_response(port, &request(&format!("rebind.example:{port}"))).await;
        assert!(response.starts_with("HTTP/1.1 403"), "{response}");
        assert!(!response.contains("webSocketDebuggerUrl"), "{response}");

        let no_host = "GET /json/version HTTP/1.1\r\nConnection: close\r\n\r\n";
        let response = raw_response(port, no_host).await;
        assert!(response.starts_with("HTTP/1.1 403"), "{response}");

        let response = raw_response(port, &request(&format!("[::1]:{port}"))).await;
        assert!(response.contains("webSocketDebuggerUrl"), "{response}");
    }

    async fn ws_handshake(
        stream: &mut tokio::net::TcpStream,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let key_bytes: [u8; 16] = rand::random();
        let key = base64_encode(&key_bytes);

        let request = format!(
            "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n\r\n",
            key
        );
        stream.write_all(request.as_bytes()).await?;

        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await?;
        let response = String::from_utf8_lossy(&buf[..n]);
        assert!(
            response.contains("101"),
            "WebSocket upgrade failed: {response}"
        );

        Ok(())
    }

    fn base64_encode(bytes: &[u8]) -> String {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
            let triple = (b0 << 16) | (b1 << 8) | b2;
            out.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
            out.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
            if chunk.len() > 1 {
                out.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
            } else {
                out.push('=');
            }
            if chunk.len() > 2 {
                out.push(CHARS[(triple & 0x3F) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }

    fn ws_encode_text(payload: &str) -> Vec<u8> {
        let payload_bytes = payload.as_bytes();
        let len = payload_bytes.len();
        let mut frame = Vec::new();
        frame.push(0x81); // FIN + text opcode
        if len < 126 {
            frame.push(len as u8 | 0x80); // masked
        } else if len < 65536 {
            frame.push(126 | 0x80);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(127 | 0x80);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        let mask: [u8; 4] = rand::random();
        frame.extend_from_slice(&mask);
        for (i, &b) in payload_bytes.iter().enumerate() {
            frame.push(b ^ mask[i % 4]);
        }
        frame
    }

    async fn ws_read_text(
        stream: &mut tokio::net::TcpStream,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut header = [0u8; 2];
        stream.read_exact(&mut header).await?;
        let opcode = header[0] & 0x0F;
        let masked = header[1] & 0x80 != 0;
        let mut len = (header[1] & 0x7F) as u64;
        if len == 126 {
            let mut b = [0u8; 2];
            stream.read_exact(&mut b).await?;
            len = u16::from_be_bytes(b) as u64;
        } else if len == 127 {
            let mut b = [0u8; 8];
            stream.read_exact(&mut b).await?;
            len = u64::from_be_bytes(b);
        }
        let mut mask = [0u8; 4];
        if masked {
            stream.read_exact(&mut mask).await?;
        }
        let mut payload = vec![0u8; len as usize];
        stream.read_exact(&mut payload).await?;
        if masked {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
        }
        if opcode == 0x8 {
            return Err("connection closed".into());
        }
        Ok(String::from_utf8(payload)?)
    }

    #[test]
    fn multi_page_concurrent_navigations() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();

        rt.block_on(local.run_until(async {
            let server = CdpServer::start_ephemeral("<html><body>init</body></html>").unwrap();
            let port = server.port();

            let mut handles = Vec::new();
            for i in 0..10 {
                let addr = format!("127.0.0.1:{}", port);
                handles.push(tokio::task::spawn_local(async move {
                    let mut stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
                    ws_handshake(&mut stream).await.unwrap();

                    let navigate_msg = serde_json::json!({
                        "id": 1,
                        "method": "Page.navigate",
                        "params": { "url": format!("https://example.com/page{}", i) }
                    })
                    .to_string();

                    stream
                        .write_all(&ws_encode_text(&navigate_msg))
                        .await
                        .unwrap();

                    let response = ws_read_text(&mut stream).await.unwrap();
                    let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
                    assert!(
                        parsed.get("result").is_some(),
                        "Expected result in response for page {i}: {response}"
                    );
                    assert!(
                        parsed["result"].get("frameId").is_some(),
                        "Missing frameId for page {i}: {response}"
                    );
                    i
                }));
            }

            let mut results = Vec::new();
            for h in handles {
                results.push(h.await.unwrap());
            }
            results.sort();
            assert_eq!(results, (0..10).collect::<Vec<_>>());

            drop(server);
        }));
    }
}
