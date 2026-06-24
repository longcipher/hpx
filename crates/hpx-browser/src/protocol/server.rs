//! WebSocket CDP server — accepts connections and dispatches to CdpSession.
//!
//! `Page` is `!Send` (V8 internals), so the server runs on a dedicated
//! thread with a single-threaded tokio runtime.

use std::{cell::RefCell, rc::Rc};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use crate::protocol::{session::CdpSession, types::*};

/// A running CDP server. Stops when dropped.
pub struct CdpServer {
    port: u16,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl CdpServer {
    /// Start a CDP WebSocket server on `127.0.0.1:{port}`.
    ///
    /// Spawns a dedicated thread with a single-threaded tokio runtime (required
    /// because `Page` is `!Send`). The HTML is used to create a fresh Page on
    /// that thread.
    pub fn start(html: &str, port: u16) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let html = html.to_string();
        let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
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

            local.block_on(&rt, async move {
                let page = match crate::page::Page::from_html(&html, None).await {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!("CdpServer: failed to create page: {}", e);
                        port_tx.send(0).ok();
                        return;
                    }
                };
                let page = Rc::new(RefCell::new(page));

                let listener = match TcpListener::bind(format!("127.0.0.1:{}", port)).await {
                    Ok(l) => l,
                    Err(e) => {
                        tracing::error!("CdpServer: failed to bind CDP port: {}", e);
                        port_tx.send(0).ok();
                        return;
                    }
                };
                let actual_port = listener.local_addr().unwrap().port();
                port_tx.send(actual_port).ok();

                accept_loop(listener, page, shutdown_clone).await;
            });
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
        Self::start(html, 0)
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
    page: Rc<RefCell<crate::page::Page>>,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    loop {
        if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        let accept =
            tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept()).await;

        match accept {
            Ok(Ok((stream, addr))) => {
                let page = page.clone();
                tokio::task::spawn_local(async move {
                    if let Err(e) = handle_connection(stream, page).await {
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

#[allow(
    clippy::await_holding_refcell_ref,
    reason = "single-threaded CDP runtime; RefCell borrow dropped before await"
)]
async fn handle_connection(
    stream: tokio::net::TcpStream,
    page: Rc<RefCell<crate::page::Page>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Peek to check for HTTP vs WebSocket
    let mut buf = [0u8; 512];
    let n = stream.peek(&mut buf).await?;
    let peek = String::from_utf8_lossy(&buf[..n]);

    if peek.starts_with("GET ") && !peek.contains("Upgrade:") && !peek.contains("upgrade:") {
        return handle_http(stream, &page).await;
    }

    let mut ws_stream = tokio_tungstenite::accept_async(stream).await?;
    let mut session = CdpSession::new();

    loop {
        let msg = match ws_stream.next().await {
            Some(Ok(msg)) => msg,
            Some(Err(e)) => return Err(e.into()),
            None => break,
        };
        match msg {
            Message::Text(text) => {
                let req: CdpRequest = match serde_json::from_str(&text) {
                    Ok(r) => r,
                    Err(e) => {
                        let err_msg = format!(
                            r#"{{"error":{{"code":-32700,"message":"Parse error: {}"}}}}"#,
                            e.to_string().replace('"', "'")
                        );
                        ws_stream.send(Message::Text(err_msg.into())).await?;
                        continue;
                    }
                };

                let (response, events) = {
                    let mut page_ref = page.borrow_mut();
                    session.handle_request(&mut page_ref, &req).await
                };

                // Handle pending navigation
                if let Some(_url) = session.pending_navigate.take() {
                    // ponytail: navigation stub — real impl fetches and reloads
                }

                for event in events {
                    ws_stream
                        .send(Message::Text(to_json(&event).into()))
                        .await?;
                }
                ws_stream.send(Message::Text(response.into())).await?;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    Ok(())
}

async fn handle_http(
    stream: tokio::net::TcpStream,
    page: &Rc<RefCell<crate::page::Page>>,
) -> Result<(), Box<dyn std::error::Error>> {
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
            let (title, url) = {
                let page_ref = page.borrow();
                (page_ref.title(), page_ref.url().to_string())
            };
            serde_json::json!([{
                "description": "",
                "devtoolsFrontendUrl": "",
                "id": "page-1",
                "title": title,
                "type": "page",
                "url": url,
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
    use super::*;

    #[test]
    fn server_starts_and_stops() {
        let server = CdpServer::start_ephemeral("<html><body>Hello</body></html>").unwrap();
        assert!(server.port() > 0);
        assert!(server.ws_url().contains("127.0.0.1"));
        drop(server);
    }
}
