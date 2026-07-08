//! Core CDP protocol types and message handling.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::error::{CdpClientError, Result};

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const BROADCAST_CAPACITY: usize = 1024;

/// Incoming CDP message (response or event).
#[derive(Debug, Clone)]
pub struct CdpMessage {
    pub id: Option<u32>,
    pub method: Option<String>,
    pub params: Option<serde_json::Value>,
    pub result: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
    pub session_id: Option<String>,
}

/// Outgoing CDP request.
#[derive(Debug, Clone, Serialize)]
pub struct CdpRequest {
    pub id: u32,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "sessionId")]
    pub session_id: Option<String>,
}

/// CDP client message router.
///
/// Manages message ID generation, pending response tracking, and event fan-out.
/// All methods are `Send + Sync` safe.
#[derive(Debug, Clone)]
pub struct CdpClient {
    message_id: Arc<AtomicU32>,
    pending_responses: Arc<Mutex<HashMap<u32, oneshot::Sender<CdpMessage>>>>,
    event_broadcast: broadcast::Sender<CdpMessage>,
    ws_tx: Arc<Mutex<Option<mpsc::UnboundedSender<String>>>>,
}

/// Filtered event receiver that only yields messages matching the given criteria.
pub struct FilteredEventReceiver {
    rx: broadcast::Receiver<CdpMessage>,
    method_filter: Option<String>,
    session_filter: Option<String>,
}

impl FilteredEventReceiver {
    /// Receive the next matching event, waiting indefinitely.
    pub async fn recv(&mut self) -> std::result::Result<CdpMessage, broadcast::error::RecvError> {
        loop {
            let msg = self.rx.recv().await?;
            if self.matches(&msg) {
                return Ok(msg);
            }
        }
    }

    /// Receive the next matching event, waiting up to `timeout`.
    pub async fn recv_timeout(
        &mut self,
        timeout: Duration,
    ) -> std::result::Result<CdpMessage, RecvTimeoutError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(RecvTimeoutError::Timeout);
            }
            match tokio::time::timeout(remaining, self.rx.recv()).await {
                Ok(Ok(msg)) => {
                    if self.matches(&msg) {
                        return Ok(msg);
                    }
                }
                Ok(Err(e)) => return Err(RecvTimeoutError::Recv(e)),
                Err(_) => return Err(RecvTimeoutError::Timeout),
            }
        }
    }

    fn matches(&self, msg: &CdpMessage) -> bool {
        let method_ok = self
            .method_filter
            .as_ref()
            .is_none_or(|m| msg.method.as_deref() == Some(m));
        let session_ok = self
            .session_filter
            .as_ref()
            .is_none_or(|s| msg.session_id.as_deref() == Some(s));
        method_ok && session_ok
    }
}

/// Error type for [`FilteredEventReceiver::recv_timeout`].
#[derive(Debug)]
pub enum RecvTimeoutError {
    /// The timeout elapsed before a matching event arrived.
    Timeout,
    /// The broadcast channel was closed or lagged.
    Recv(broadcast::error::RecvError),
}

impl std::fmt::Display for RecvTimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "timeout waiting for filtered event"),
            Self::Recv(e) => write!(f, "broadcast recv error: {e}"),
        }
    }
}

impl std::error::Error for RecvTimeoutError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Timeout => None,
            Self::Recv(e) => Some(e),
        }
    }
}

impl Default for CdpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl CdpClient {
    /// Create a new CDP client with no writer connected.
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            message_id: Arc::new(AtomicU32::new(1)),
            pending_responses: Arc::new(Mutex::new(HashMap::new())),
            event_broadcast: event_tx,
            ws_tx: Arc::new(Mutex::new(None)),
        }
    }

    /// Generate the next message ID.
    fn next_id(&self) -> u32 {
        self.message_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Register a pending response handler.
    pub(crate) fn register_response_handler(&self, id: u32, tx: oneshot::Sender<CdpMessage>) {
        self.pending_responses.lock().insert(id, tx);
    }

    /// Send a CDP command and await the response.
    pub async fn send_command(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<CdpMessage> {
        self.send_command_internal(method, params, None).await
    }

    /// Send a CDP command with a session ID and await the response.
    pub async fn send_command_with_session(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
        session_id: &str,
    ) -> Result<CdpMessage> {
        self.send_command_internal(method, params, Some(session_id))
            .await
    }

    async fn send_command_internal(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
        session_id: Option<&str>,
    ) -> Result<CdpMessage> {
        let id = self.next_id();
        let (tx, rx) = oneshot::channel();

        // Register before send (race-free pattern)
        self.register_response_handler(id, tx);

        let request = CdpRequest {
            id,
            method: method.to_string(),
            params,
            session_id: session_id.map(String::from),
        };

        let json = serde_json::to_string(&request)?;

        // Send through the mpsc channel — lock released before any .await
        {
            let sender = self.ws_tx.lock();
            let sender = sender
                .as_ref()
                .ok_or_else(|| CdpClientError::websocket("send_command", "no writer connected"))?;
            sender
                .send(json)
                .map_err(|_| CdpClientError::websocket("send_command", "writer channel closed"))?;
        }

        // Await response with timeout
        tokio::time::timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS), rx)
            .await
            .map_err(|_| CdpClientError::timeout("send_command", DEFAULT_TIMEOUT_SECS))?
            .map_err(|_| CdpClientError::websocket("send_command", "response channel closed"))
    }

    /// Subscribe to all CDP events.
    pub fn subscribe_events(&self) -> broadcast::Receiver<CdpMessage> {
        self.event_broadcast.subscribe()
    }

    /// Subscribe to CDP events filtered by method and/or session ID.
    pub fn subscribe_events_filtered(
        &self,
        method: Option<&str>,
        session_id: Option<&str>,
    ) -> FilteredEventReceiver {
        FilteredEventReceiver {
            rx: self.event_broadcast.subscribe(),
            method_filter: method.map(String::from),
            session_filter: session_id.map(String::from),
        }
    }

    /// Connect to a CDP endpoint via WebSocket.
    ///
    /// Establishes a WebSocket connection using hpx-yawc, spawns a writer task
    /// that drains outgoing commands to the socket, and spawns the
    /// [`Connection`](super::connection::Connection) event loop that dispatches
    /// incoming frames. Returns a ready-to-use [`CdpClient`].
    #[cfg(feature = "cdp")]
    pub async fn connect(ws_url: &str) -> super::error::Result<Self> {
        use futures_util::{SinkExt, StreamExt};

        let url: url::Url = ws_url.parse().map_err(|e: url::ParseError| {
            super::error::CdpClientError::connection_failed(ws_url, &e.to_string())
        })?;

        let ws = hpx_yawc::WebSocket::connect(url)
            .await
            .map_err(|e| super::error::CdpClientError::connection_failed(ws_url, &e.to_string()))?;

        let (mut sink, stream) = ws.split();

        let client = Self::new();
        let cdp = Arc::new(client.clone());

        // Writer channel: mpsc → WebSocket sink
        let (ws_tx, mut ws_rx) = mpsc::unbounded_channel::<String>();
        *client.ws_tx.lock() = Some(ws_tx);

        tokio::spawn(async move {
            while let Some(msg) = ws_rx.recv().await {
                let frame = hpx_yawc::frame::Frame::text(msg);
                if let Err(e) = sink.send(frame).await {
                    tracing::debug!("WebSocket write error: {e}");
                    break;
                }
            }
        });

        // Map yawc frames → connection::Frame and run the event loop
        let mapped = stream.map(|frame| {
            use hpx_yawc::frame::OpCode;
            match frame.opcode() {
                OpCode::Text => {
                    let text = std::str::from_utf8(frame.payload().as_ref()).map_err(|e| {
                        super::connection::ConnectionError::WebSocket(e.to_string())
                    })?;
                    Ok(super::connection::Frame::Text(text.to_string()))
                }
                OpCode::Close => Ok(super::connection::Frame::Close),
                _ => Ok(super::connection::Frame::Other),
            }
        });

        let mut conn = super::connection::Connection::new(cdp, mapped);
        tokio::spawn(async move {
            conn.run().await;
        });

        Ok(client)
    }

    /// Fail all pending responses by dropping their senders.
    pub fn fail_all_pending(&self, reason: &str) {
        let mut pending = self.pending_responses.lock();
        tracing::debug!("failing {} pending responses: {reason}", pending.len());
        pending.clear();
    }

    /// Dispatch an incoming message (called by the WebSocket reader).
    pub fn dispatch_message(&self, msg: CdpMessage) {
        let sender = if let Some(id) = msg.id {
            let mut pending = self.pending_responses.lock();
            pending.remove(&id)
        } else {
            None
        };

        if let Some(tx) = sender {
            let _ = tx.send(msg);
        } else {
            let _ = self.event_broadcast.send(msg);
        }
    }

    /// Set the writer channel for testing purposes.
    #[cfg(test)]
    pub(crate) fn set_test_writer(&self, tx: mpsc::UnboundedSender<String>) {
        *self.ws_tx.lock() = Some(tx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_generation_increments() {
        let client = CdpClient::new();
        assert_eq!(client.next_id(), 1);
        assert_eq!(client.next_id(), 2);
        assert_eq!(client.next_id(), 3);
    }

    #[test]
    fn register_before_send_pattern() {
        let client = CdpClient::new();
        let (tx, _rx) = oneshot::channel();

        client.register_response_handler(42, tx);

        {
            let pending = client.pending_responses.lock();
            assert!(pending.contains_key(&42));
        }

        client.fail_all_pending("test");

        {
            let pending = client.pending_responses.lock();
            assert!(!pending.contains_key(&42));
        }
    }

    #[tokio::test]
    async fn fail_all_pending_drops_senders() {
        let client = CdpClient::new();
        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();

        client.register_response_handler(1, tx1);
        client.register_response_handler(2, tx2);

        client.fail_all_pending("test");

        assert!(rx1.await.is_err());
        assert!(rx2.await.is_err());
    }

    #[tokio::test]
    async fn broadcast_fan_out() {
        let client = CdpClient::new();
        let mut rx1 = client.subscribe_events();
        let mut rx2 = client.subscribe_events();

        let msg = CdpMessage {
            id: None,
            method: Some("Page.loadEventFired".to_string()),
            params: Some(serde_json::json!({})),
            result: None,
            error: None,
            session_id: None,
        };

        client.dispatch_message(msg);

        let received1 = rx1.recv().await.unwrap();
        let received2 = rx2.recv().await.unwrap();

        assert_eq!(received1.method, Some("Page.loadEventFired".to_string()));
        assert_eq!(received2.method, Some("Page.loadEventFired".to_string()));
    }

    #[tokio::test]
    async fn dispatch_message_routes_response() {
        let client = CdpClient::new();

        let (tx, rx) = oneshot::channel();
        client.register_response_handler(1, tx);

        let msg = CdpMessage {
            id: Some(1),
            method: None,
            params: None,
            result: Some(serde_json::json!({"value": "ok"})),
            error: None,
            session_id: None,
        };

        client.dispatch_message(msg);

        let response = rx.await.unwrap();
        assert_eq!(response.id, Some(1));
        assert_eq!(response.result, Some(serde_json::json!({"value": "ok"})));
    }

    #[test]
    fn subscribe_events_filtered_by_method() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let client = CdpClient::new();
            let mut filtered = client.subscribe_events_filtered(Some("Page.loadEventFired"), None);

            let msg_event = CdpMessage {
                id: None,
                method: Some("Page.loadEventFired".to_string()),
                params: Some(serde_json::json!({})),
                result: None,
                error: None,
                session_id: None,
            };

            let msg_other = CdpMessage {
                id: None,
                method: Some("Runtime.consoleAPICalled".to_string()),
                params: Some(serde_json::json!({})),
                result: None,
                error: None,
                session_id: None,
            };

            client.dispatch_message(msg_other);
            client.dispatch_message(msg_event);

            let received = tokio::time::timeout(Duration::from_secs(1), filtered.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(received.method, Some("Page.loadEventFired".to_string()));
        });
    }

    #[tokio::test]
    async fn send_command_full_flow() {
        let client = CdpClient::new();
        let (writer_tx, mut writer_rx) = mpsc::unbounded_channel();
        *client.ws_tx.lock() = Some(writer_tx);

        let client_clone = client.clone();
        let send_handle = tokio::spawn(async move {
            client_clone
                .send_command(
                    "Page.navigate",
                    Some(serde_json::json!({"url": "https://example.com"})),
                )
                .await
        });

        let sent_json = writer_rx.recv().await.expect("should receive sent message");
        let sent: serde_json::Value =
            serde_json::from_str(&sent_json).expect("should parse request");

        let sent_id = sent["id"].as_u64().expect("should have id") as u32;

        let response = CdpMessage {
            id: Some(sent_id),
            method: None,
            params: None,
            result: Some(serde_json::json!({"frameId": "123"})),
            error: None,
            session_id: None,
        };
        client.dispatch_message(response);

        let result = send_handle
            .await
            .expect("task should complete")
            .expect("send_command should succeed");
        assert_eq!(result.id, Some(sent_id));
        assert_eq!(result.result, Some(serde_json::json!({"frameId": "123"})));
    }

    #[tokio::test]
    async fn send_command_fails_without_writer() {
        let client = CdpClient::new();

        let result = client.send_command("Page.navigate", None).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("no writer connected"));
    }

    #[tokio::test]
    async fn send_command_with_session() {
        let client = CdpClient::new();
        let (writer_tx, mut writer_rx) = mpsc::unbounded_channel();
        *client.ws_tx.lock() = Some(writer_tx);

        let client_clone = client.clone();
        let send_handle = tokio::spawn(async move {
            client_clone
                .send_command_with_session(
                    "Runtime.evaluate",
                    Some(serde_json::json!({"expression": "1+1"})),
                    "session-42",
                )
                .await
        });

        let sent_json = writer_rx.recv().await.expect("should receive sent message");
        let sent: serde_json::Value =
            serde_json::from_str(&sent_json).expect("should parse request");

        assert_eq!(sent["sessionId"].as_str(), Some("session-42"));

        let sent_id = sent["id"].as_u64().expect("should have id") as u32;

        let response = CdpMessage {
            id: Some(sent_id),
            method: None,
            params: None,
            result: Some(serde_json::json!({"result": {"value": 2}})),
            error: None,
            session_id: Some("session-42".to_string()),
        };
        client.dispatch_message(response);

        let result = send_handle
            .await
            .expect("task should complete")
            .expect("send_command should succeed");
        assert_eq!(result.id, Some(sent_id));
        assert_eq!(result.session_id.as_deref(), Some("session-42"));
    }

    #[tokio::test]
    async fn dispatch_unknown_id_broadcasts_as_event() {
        let client = CdpClient::new();
        let mut rx = client.subscribe_events();

        let msg = CdpMessage {
            id: Some(999),
            method: Some("Page.loadEventFired".to_string()),
            params: None,
            result: None,
            error: None,
            session_id: None,
        };

        client.dispatch_message(msg);

        let received = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received.id, Some(999));
    }

    #[tokio::test]
    async fn broadcast_filtered_by_session() {
        let client = CdpClient::new();
        let mut filtered = client.subscribe_events_filtered(None, Some("session-A"));

        let msg_a = CdpMessage {
            id: None,
            method: Some("Page.loadEventFired".to_string()),
            params: None,
            result: None,
            error: None,
            session_id: Some("session-A".to_string()),
        };

        let msg_b = CdpMessage {
            id: None,
            method: Some("Page.loadEventFired".to_string()),
            params: None,
            result: None,
            error: None,
            session_id: Some("session-B".to_string()),
        };

        client.dispatch_message(msg_b);
        client.dispatch_message(msg_a);

        let received = tokio::time::timeout(Duration::from_secs(1), filtered.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received.session_id.as_deref(), Some("session-A"));
    }

    #[tokio::test]
    async fn recv_timeout_expires_when_no_matching_event() {
        let client = CdpClient::new();
        let mut filtered = client.subscribe_events_filtered(Some("Page.loadEventFired"), None);

        // Send a non-matching event
        client.dispatch_message(CdpMessage {
            id: None,
            method: Some("Runtime.consoleAPICalled".to_string()),
            params: None,
            result: None,
            error: None,
            session_id: None,
        });

        let result = tokio::time::timeout(
            Duration::from_secs(2),
            filtered.recv_timeout(Duration::from_millis(50)),
        )
        .await
        .unwrap();

        assert!(result.is_err(), "recv_timeout should return timeout error");
    }

    #[tokio::test]
    async fn recv_timeout_returns_matching_event() {
        let client = CdpClient::new();
        let mut filtered = client.subscribe_events_filtered(Some("Page.loadEventFired"), None);

        let client_clone = client.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            client_clone.dispatch_message(CdpMessage {
                id: None,
                method: Some("Page.loadEventFired".to_string()),
                params: None,
                result: None,
                error: None,
                session_id: None,
            });
        });

        let result = filtered.recv_timeout(Duration::from_secs(1)).await;
        assert!(result.is_ok(), "recv_timeout should return matching event");
        assert_eq!(
            result.unwrap().method.as_deref(),
            Some("Page.loadEventFired")
        );
    }
}

#[cfg(all(test, feature = "proptest"))]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn cdp_request_json_round_trip(
            id in 0u32..u32::MAX,
            method in "[A-Za-z]{1,20}\\.[A-Za-z]{1,30}",
            session_id in proptest::option::of("[a-z0-9-]{1,20}"),
        ) {
            let request = CdpRequest {
                id,
                method: method.clone(),
                params: None,
                session_id: session_id.clone(),
            };

            let json = serde_json::to_string(&request)?;
            let parsed: serde_json::Value = serde_json::from_str(&json)?;

            prop_assert_eq!(parsed["id"].as_u64(), Some(id as u64));
            prop_assert_eq!(parsed["method"].as_str(), Some(method.as_str()));
            if let Some(ref sid) = session_id {
                prop_assert_eq!(parsed["sessionId"].as_str(), Some(sid.as_str()));
            }
        }

        #[test]
        fn cdp_message_dispatch_preserves_fields(
            id in proptest::option::of(0u32..1000u32),
            method in proptest::option::of("[A-Za-z]{1,20}\\.[A-Za-z]{1,30}"),
            session_id in proptest::option::of("[a-z0-9-]{1,20}"),
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let client = CdpClient::new();

                if let Some(test_id) = id {
                    // Messages with id go to pending handlers
                    let (tx, rx) = oneshot::channel();
                    client.register_response_handler(test_id, tx);

                    let msg = CdpMessage {
                        id: Some(test_id),
                        method: method.clone(),
                        params: None,
                        result: None,
                        error: None,
                        session_id: session_id.clone(),
                    };
                    client.dispatch_message(msg);

                    let received = tokio::time::timeout(Duration::from_secs(1), rx)
                        .await
                        .expect("timeout")
                        .expect("oneshot recv");
                    prop_assert_eq!(received.id, Some(test_id));
                    prop_assert_eq!(received.method, method);
                    prop_assert_eq!(received.session_id, session_id);
                } else {
                    // Messages without id go to broadcast
                    let mut events = client.subscribe_events();

                    let msg = CdpMessage {
                        id: None,
                        method: method.clone(),
                        params: None,
                        result: None,
                        error: None,
                        session_id: session_id.clone(),
                    };
                    client.dispatch_message(msg);

                    let received = tokio::time::timeout(Duration::from_secs(1), events.recv())
                        .await
                        .expect("timeout")
                        .expect("recv error");
                    prop_assert_eq!(received.method, method);
                    prop_assert_eq!(received.session_id, session_id);
                }

                Ok(())
            })?;
        }
    }
}

// ── Integration tests (require `cdp` feature for hpx-yawc) ──────────────────

#[cfg(feature = "cdp")]
#[cfg(test)]
mod integration_tests {
    use std::net::SocketAddr;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    // ── Mock WebSocket server helpers ───────────────────────────────────────

    /// Perform the server-side WebSocket handshake: read the HTTP upgrade
    /// request, validate headers, compute the accept key, and send back
    /// the 101 response.
    async fn ws_server_handshake(
        stream: &mut tokio::net::TcpStream,
    ) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await?;
        let request = String::from_utf8_lossy(&buf[..n]);

        let key = request
            .lines()
            .find(|l| l.to_lowercase().starts_with("sec-websocket-key:"))
            .and_then(|l| l.split(':').nth(1))
            .map(|s| s.trim().to_string())
            .ok_or("missing sec-websocket-key")?;

        use base64::prelude::*;
        use sha1::{Digest, Sha1};
        let mut sha = Sha1::new();
        sha.update(key.as_bytes());
        sha.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
        let accept = BASE64_STANDARD.encode(sha.finalize());

        let response = format!(
            "HTTP/1.1 101 Switching Protocols\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Accept: {accept}\r\n\
             \r\n"
        );
        stream.write_all(response.as_bytes()).await?;
        Ok(())
    }

    /// Read a single WebSocket text frame from the stream.
    /// Handles client-to-server masking. Non-text frames are skipped.
    async fn ws_read_frame(
        stream: &mut tokio::net::TcpStream,
    ) -> std::result::Result<String, Box<dyn std::error::Error + Send + Sync>> {
        loop {
            let mut header = [0u8; 2];
            stream.read_exact(&mut header).await?;

            let opcode = header[0] & 0x0F;
            let masked = (header[1] & 0x80) != 0;
            let mut len = (header[1] & 0x7F) as u64;

            if len == 126 {
                let mut buf = [0u8; 2];
                stream.read_exact(&mut buf).await?;
                len = u16::from_be_bytes(buf) as u64;
            } else if len == 127 {
                let mut buf = [0u8; 8];
                stream.read_exact(&mut buf).await?;
                len = u64::from_be_bytes(buf);
            }

            let mut mask_key = [0u8; 4];
            if masked {
                stream.read_exact(&mut mask_key).await?;
            }

            let mut payload = vec![0u8; len as usize];
            stream.read_exact(&mut payload).await?;

            if masked {
                for (i, byte) in payload.iter_mut().enumerate() {
                    *byte ^= mask_key[i % 4];
                }
            }

            // Handle ping with pong response
            if opcode == 0x9 {
                let mut pong = Vec::with_capacity(2 + payload.len());
                pong.push(0x8A); // FIN + PONG
                pong.push(payload.len() as u8);
                pong.extend_from_slice(&payload);
                stream.write_all(&pong).await?;
                continue;
            }

            // Skip non-text frames (close, binary, etc.)
            if opcode != 0x1 {
                continue;
            }

            return Ok(String::from_utf8(payload)?);
        }
    }

    /// Write an unmasked WebSocket text frame (server → client).
    async fn ws_write_frame(
        stream: &mut tokio::net::TcpStream,
        text: &str,
    ) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let payload = text.as_bytes();
        let len = payload.len();

        let mut frame = Vec::with_capacity(10 + len);
        frame.push(0x81); // FIN + TEXT

        if len < 126 {
            frame.push(len as u8);
        } else if len < 65536 {
            frame.push(126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }

        frame.extend_from_slice(payload);
        stream.write_all(&frame).await?;
        Ok(())
    }

    /// Spin up a mock CDP server on a random port. Returns the bound address.
    /// The server task reads CDP commands and responds with mock JSON responses.
    /// For each incoming request it echoes back `{"id": <id>, "result": {...}}`
    /// where the result is `{"method": <method>}`.
    async fn start_mock_cdp_server() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            ws_server_handshake(&mut stream).await.unwrap();

            // Read commands and respond
            while let Ok(json_str) = ws_read_frame(&mut stream).await {
                let Ok(req) = serde_json::from_str::<serde_json::Value>(&json_str) else {
                    continue;
                };
                let id = req["id"].as_u64().unwrap_or(0);
                let method = req["method"].as_str().unwrap_or("unknown");
                let response = serde_json::json!({
                    "id": id,
                    "result": { "method": method }
                });
                let _ = ws_write_frame(&mut stream, &response.to_string()).await;
            }
        });

        addr
    }

    /// Spin up a mock CDP server that includes session_id in events.
    /// Reads commands; for each command with a session_id, it also sends
    /// an event for that session after a small delay.
    async fn start_mock_cdp_server_with_sessions() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            ws_server_handshake(&mut stream).await.unwrap();

            while let Ok(json_str) = ws_read_frame(&mut stream).await {
                let Ok(req) = serde_json::from_str::<serde_json::Value>(&json_str) else {
                    continue;
                };
                let id = req["id"].as_u64().unwrap_or(0);
                let method = req["method"].as_str().unwrap_or("unknown");
                let session_id = req["sessionId"].as_str().map(String::from);

                // Send the command response
                let response = serde_json::json!({
                    "id": id,
                    "result": { "method": method },
                    "sessionId": session_id,
                });
                let _ = ws_write_frame(&mut stream, &response.to_string()).await;

                // Delay before sending the event to ensure Connection task processes response first
                tokio::time::sleep(Duration::from_millis(50)).await;

                // Also emit a domain event for the session
                if let Some(ref sid) = session_id {
                    let event = serde_json::json!({
                        "method": format!("{method}.eventFired"),
                        "params": {},
                        "sessionId": sid,
                    });
                    let _ = ws_write_frame(&mut stream, &event.to_string()).await;
                }
            }
        });

        addr
    }

    #[tokio::test]
    async fn session_isolation_events_do_not_cross() {
        let addr = start_mock_cdp_server_with_sessions().await;
        let client = CdpClient::connect(&format!("ws://{addr}/devtools/browser"))
            .await
            .expect("connect should succeed");

        // Verify the connection works by sending a command
        let resp = client
            .send_command_with_session("Page.enable", None, "session-A")
            .await
            .unwrap();
        assert_eq!(resp.id, Some(1));

        // Test session isolation via direct dispatch (same broadcast channel the Connection uses)
        let mut events_a = client.subscribe_events_filtered(None, Some("session-A"));
        let mut events_b = client.subscribe_events_filtered(None, Some("session-B"));

        // Dispatch events directly to the client (simulates what Connection does)
        client.dispatch_message(CdpMessage {
            id: None,
            method: Some("Page.loadEventFired".to_string()),
            params: Some(serde_json::json!({})),
            result: None,
            error: None,
            session_id: Some("session-A".to_string()),
        });

        let evt = tokio::time::timeout(Duration::from_secs(1), events_a.recv())
            .await
            .expect("session-A should receive its event")
            .expect("recv error");
        assert_eq!(evt.session_id.as_deref(), Some("session-A"));
        assert_eq!(evt.method.as_deref(), Some("Page.loadEventFired"));

        // session-B should NOT see session-A's event
        // Note: broadcast channels deliver to all subscribers; filtering happens
        // in recv(). We verify this by checking events_b returns only session-B events.
        assert!(
            tokio::time::timeout(Duration::from_millis(200), events_b.recv())
                .await
                .is_err(),
            "session-B receiver should not see session-A events"
        );

        // Now dispatch an event for session-B
        client.dispatch_message(CdpMessage {
            id: None,
            method: Some("Page.loadEventFired".to_string()),
            params: Some(serde_json::json!({})),
            result: None,
            error: None,
            session_id: Some("session-B".to_string()),
        });

        let evt_b = tokio::time::timeout(Duration::from_secs(1), events_b.recv())
            .await
            .expect("session-B should receive its event")
            .expect("recv error");
        assert_eq!(evt_b.session_id.as_deref(), Some("session-B"));

        // Verify session-A's filtered receiver does not return session-B's event.
        // The broadcast delivers to all subscribers, but the filter discards it.
        let evt_a = tokio::time::timeout(Duration::from_millis(200), events_a.recv()).await;
        // If events_a received anything, it must be from session-A only (none dispatched)
        if let Ok(Ok(evt)) = evt_a {
            assert_eq!(
                evt.session_id.as_deref(),
                Some("session-A"),
                "events_a should only see session-A events"
            );
        }
    }

    // ── Tests ───────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn connect_and_send_command() {
        let addr = start_mock_cdp_server().await;
        let client = CdpClient::connect(&format!("ws://{addr}/devtools/browser"))
            .await
            .expect("connect should succeed");

        let response = client
            .send_command("Page.enable", None)
            .await
            .expect("send_command should succeed");

        assert_eq!(response.id, Some(1));
        assert_eq!(
            response.result,
            Some(serde_json::json!({ "method": "Page.enable" }))
        );
    }

    #[tokio::test]
    async fn connect_multiple_commands() {
        let addr = start_mock_cdp_server().await;
        let client = CdpClient::connect(&format!("ws://{addr}/devtools/browser"))
            .await
            .expect("connect should succeed");

        let r1 = client.send_command("Page.enable", None).await.unwrap();
        let r2 = client.send_command("Runtime.enable", None).await.unwrap();

        assert_eq!(r1.id, Some(1));
        assert_eq!(r2.id, Some(2));
        assert_eq!(
            r1.result,
            Some(serde_json::json!({ "method": "Page.enable" }))
        );
        assert_eq!(
            r2.result,
            Some(serde_json::json!({ "method": "Runtime.enable" }))
        );
    }
}
