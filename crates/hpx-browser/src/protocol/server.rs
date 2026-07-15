//! WebSocket CDP server — accepts connections and dispatches to CdpSession.

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use hpx_yawc::{Frame, frame::OpCode};
use tokio::net::TcpListener;

use crate::{
    protocol::{session::CdpSession, types::*},
    stealth::StealthProfile,
};

/// Serializes V8-critical page operations across all connections on the same thread.
/// Multiple V8 isolates on a single thread corrupt the HandleScope stack; this mutex
/// ensures only one isolate is active at a time.
static V8_SERIALIZE: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

/// A running CDP server. Stops when dropped.
pub struct CdpServer {
    port: u16,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
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
        let html = html.to_string();
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();
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
                let actual_port = listener
                    .local_addr()
                    .unwrap_or_else(|e| panic!("failed to get local addr: {e}"))
                    .port();
                port_tx.send(actual_port).ok();

                accept_loop(listener, html, stealth, profile, shutdown_clone).await;
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
    shutdown: Arc<std::sync::atomic::AtomicBool>,
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
                tokio::task::spawn_local(async move {
                    if let Err(e) = handle_connection(stream, &html, stealth, profile).await {
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

async fn handle_connection(
    stream: tokio::net::TcpStream,
    html: &str,
    stealth: bool,
    profile: Option<StealthProfile>,
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
            async move {
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

                tokio::task::spawn_local(async move {
                    match upgrade_fut.await {
                        Ok(ws) => {
                            if let Err(e) = handle_ws_connection(ws, &html, stealth, profile).await
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
) -> Result<(), Box<dyn std::error::Error>> {
    let mut page = crate::page::Page::from_html(html, stealth)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;
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
                let text = frame.as_str();
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

                // Serialize V8-critical CDP request handling.
                // Multiple V8 isolates on a single thread corrupt the HandleScope stack.
                let (response, events) = {
                    let _v8_lock = V8_SERIALIZE.lock().await;
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
