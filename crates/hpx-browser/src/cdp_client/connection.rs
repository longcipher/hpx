//! WebSocket connection lifecycle management for CDP communication.
//!
//! [`Connection`] reads frames from a generic WebSocket stream, parses JSON
//! text frames into [`CdpMessage`]s, and dispatches them through
//! [`CdpClient::dispatch_message`]. On stream termination (close, EOF,
//! or error) it calls [`CdpClient::fail_all_pending`] so that all
//! outstanding command handlers resolve immediately.

use std::sync::Arc;

use futures_util::{Stream, StreamExt};
use serde_json::Value;

use super::cdp::{CdpClient, CdpMessage};

/// A WebSocket connection that reads frames and dispatches CDP messages.
///
/// The stream type `S` must yield `Result<Frame, ConnectionError>` items.
/// This allows the struct to work with both real WebSocket streams
/// (tokio-tungstenite / hpx-yawc) and test mocks.
pub struct Connection<S> {
    cdp: Arc<CdpClient>,
    stream: S,
}

/// Normalized WebSocket frame.
#[derive(Debug)]
pub enum Frame {
    /// UTF-8 text payload.
    Text(String),
    /// Connection closed (with optional close code/reason).
    Close,
    /// Any other frame (binary, ping, pong, etc.) — ignored.
    Other,
}

/// Errors that can occur while reading from the WebSocket stream.
#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    /// I/O error from the underlying transport.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// WebSocket protocol error.
    #[error("WebSocket error: {0}")]
    WebSocket(String),
}

impl<S> Connection<S>
where
    S: Stream<Item = Result<Frame, ConnectionError>> + Unpin,
{
    /// Create a new connection backed by the given CDP client and stream.
    #[must_use]
    pub fn new(cdp: Arc<CdpClient>, stream: S) -> Self {
        Self { cdp, stream }
    }

    /// Run the connection event loop.
    ///
    /// Reads frames until the stream ends (close, EOF, or error), then
    /// calls `fail_all_pending` on the CDP client.
    pub async fn run(&mut self) {
        while let Some(result) = self.stream.next().await {
            match result {
                Ok(frame) => match frame {
                    Frame::Text(text) => {
                        self.handle_text_frame(&text);
                    }
                    Frame::Close => {
                        tracing::debug!("WebSocket close frame received");
                        break;
                    }
                    Frame::Other => {}
                },
                Err(e) => {
                    tracing::debug!("WebSocket error: {e}");
                    break;
                }
            }
        }
        self.cdp.fail_all_pending("WebSocket connection terminated");
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn handle_text_frame(&self, text: &str) {
        match serde_json::from_str::<Value>(text) {
            Ok(json) => {
                let msg = CdpMessage {
                    id: json
                        .get("id")
                        .and_then(Value::as_u64)
                        .and_then(|v| u32::try_from(v).ok()),
                    method: json.get("method").and_then(Value::as_str).map(String::from),
                    params: json.get("params").cloned(),
                    result: json.get("result").cloned(),
                    error: json.get("error").cloned(),
                    session_id: json
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .map(String::from),
                };
                self.cdp.dispatch_message(msg);
            }
            Err(e) => {
                tracing::debug!("Failed to parse CDP message: {e}");
            }
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use futures_util::stream;

    use super::*;

    // -- Helpers -----------------------------------------------------------

    fn ok(frame: Frame) -> Result<Frame, ConnectionError> {
        Ok(frame)
    }

    fn err(msg: &str) -> Result<Frame, ConnectionError> {
        Err(ConnectionError::WebSocket(msg.to_string()))
    }

    fn text(s: &str) -> Result<Frame, ConnectionError> {
        Ok(Frame::Text(s.to_string()))
    }

    // -- Unit tests --------------------------------------------------------

    #[tokio::test]
    async fn text_frame_parsed_and_dispatched() {
        let cdp = Arc::new(CdpClient::new());
        let mut events = cdp.subscribe_events();

        let frames = vec![ok(Frame::Text(
            r#"{"id":1,"method":"Page.navigate","result":{"frameId":"123"}}"#.to_string(),
        ))];
        let stream = stream::iter(frames);

        let mut conn = Connection::new(cdp.clone(), stream);
        conn.run().await;

        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("timeout")
            .expect("recv error");

        assert_eq!(msg.id, Some(1));
        assert_eq!(msg.method.as_deref(), Some("Page.navigate"));
        assert_eq!(msg.result, Some(serde_json::json!({"frameId": "123"})));
    }

    #[tokio::test]
    async fn non_text_frames_ignored() {
        let cdp = Arc::new(CdpClient::new());
        let mut events = cdp.subscribe_events();

        let frames = vec![
            ok(Frame::Text(
                r#"{"id":1,"method":"Page.navigate"}"#.to_string(),
            )),
            ok(Frame::Close),
        ];
        let stream = stream::iter(frames);

        let mut conn = Connection::new(cdp.clone(), stream);
        conn.run().await;

        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("timeout")
            .expect("recv error");
        assert_eq!(msg.id, Some(1));

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), events.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn binary_frame_ignored() {
        let cdp = Arc::new(CdpClient::new());

        let frames = vec![
            ok(Frame::Other),
            ok(Frame::Text(
                r#"{"id":1,"method":"Page.navigate"}"#.to_string(),
            )),
            ok(Frame::Close),
        ];
        let stream = stream::iter(frames);

        let mut conn = Connection::new(cdp, stream);
        conn.run().await;
    }

    #[tokio::test]
    async fn invalid_json_does_not_panic() {
        let cdp = Arc::new(CdpClient::new());

        let frames = vec![ok(Frame::Text("not json".to_string()))];
        let stream = stream::iter(frames);

        let mut conn = Connection::new(cdp, stream);
        conn.run().await;
    }

    #[tokio::test]
    async fn id_messages_routed_to_pending_handler() {
        let cdp = Arc::new(CdpClient::new());
        let (tx, rx) = tokio::sync::oneshot::channel();
        cdp.register_response_handler(42, tx);

        let mut events = cdp.subscribe_events();

        let frames = vec![ok(Frame::Text(
            r#"{"id":42,"result":{"ok":true}}"#.to_string(),
        ))];
        let stream = stream::iter(frames);

        let mut conn = Connection::new(cdp.clone(), stream);
        conn.run().await;

        let resp = tokio::time::timeout(std::time::Duration::from_secs(1), rx)
            .await
            .expect("timeout")
            .expect("oneshot recv");
        assert_eq!(resp.id, Some(42));

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), events.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn method_only_messages_broadcast_as_events() {
        let cdp = Arc::new(CdpClient::new());
        let mut events = cdp.subscribe_events();

        let frames = vec![ok(Frame::Text(
            r#"{"method":"Page.loadEventFired","params":{}}"#.to_string(),
        ))];
        let stream = stream::iter(frames);

        let mut conn = Connection::new(cdp.clone(), stream);
        conn.run().await;

        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("timeout")
            .expect("recv error");
        assert_eq!(msg.method.as_deref(), Some("Page.loadEventFired"));
        assert!(msg.id.is_none());
    }

    #[tokio::test]
    async fn session_id_extracted_from_json() {
        let cdp = Arc::new(CdpClient::new());
        let mut events = cdp.subscribe_events();

        let frames = vec![ok(Frame::Text(
            r#"{"method":"Runtime.consoleAPICalled","sessionId":"sess-1","params":{}}"#.to_string(),
        ))];
        let stream = stream::iter(frames);

        let mut conn = Connection::new(cdp.clone(), stream);
        conn.run().await;

        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("timeout")
            .expect("recv error");
        assert_eq!(msg.session_id.as_deref(), Some("sess-1"));
    }

    #[tokio::test]
    async fn fail_all_pending_on_empty_stream() {
        let cdp = Arc::new(CdpClient::new());

        let (tx1, rx1) = tokio::sync::oneshot::channel();
        let (tx2, rx2) = tokio::sync::oneshot::channel();
        cdp.register_response_handler(1, tx1);
        cdp.register_response_handler(2, tx2);

        let stream = stream::iter(std::iter::empty::<Result<Frame, ConnectionError>>());
        let mut conn = Connection::new(cdp, stream);
        conn.run().await;

        assert!(rx1.await.is_err(), "rx1 should have failed");
        assert!(rx2.await.is_err(), "rx2 should have failed");
    }

    #[tokio::test]
    async fn close_frame_triggers_fail_all_pending() {
        let cdp = Arc::new(CdpClient::new());

        let (tx, rx) = tokio::sync::oneshot::channel();
        cdp.register_response_handler(7, tx);

        let frames = vec![ok(Frame::Close)];
        let stream = stream::iter(frames);

        let mut conn = Connection::new(cdp, stream);
        conn.run().await;

        assert!(rx.await.is_err(), "rx should have failed after close frame");
    }

    #[tokio::test]
    async fn stream_error_triggers_fail_all_pending() {
        let cdp = Arc::new(CdpClient::new());

        let (tx, rx) = tokio::sync::oneshot::channel();
        cdp.register_response_handler(3, tx);

        let frames = vec![err("connection reset")];
        let stream = stream::iter(frames);

        let mut conn = Connection::new(cdp, stream);
        conn.run().await;

        assert!(
            rx.await.is_err(),
            "rx should have failed after stream error"
        );
    }

    #[tokio::test]
    async fn multiple_text_frames_processed_in_order() {
        let cdp = Arc::new(CdpClient::new());
        let mut events = cdp.subscribe_events();

        let frames = vec![
            ok(Frame::Text(
                r#"{"id":1,"method":"Page.navigate"}"#.to_string(),
            )),
            ok(Frame::Text(
                r#"{"id":2,"method":"Page.reload"}"#.to_string(),
            )),
            ok(Frame::Text(
                r#"{"method":"Page.loadEventFired"}"#.to_string(),
            )),
            ok(Frame::Close),
        ];
        let stream = stream::iter(frames);

        let mut conn = Connection::new(cdp.clone(), stream);
        conn.run().await;

        let msg1 = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("timeout")
            .expect("recv");
        assert_eq!(msg1.id, Some(1));

        let msg2 = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("timeout")
            .expect("recv");
        assert_eq!(msg2.id, Some(2));

        let msg3 = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("timeout")
            .expect("recv");
        assert_eq!(msg3.method.as_deref(), Some("Page.loadEventFired"));
    }

    #[tokio::test]
    async fn close_after_text_frame_still_dispatches() {
        let cdp = Arc::new(CdpClient::new());
        let mut events = cdp.subscribe_events();

        let frames = vec![
            ok(Frame::Text(
                r#"{"id":5,"method":"Page.navigate"}"#.to_string(),
            )),
            ok(Frame::Close),
        ];
        let stream = stream::iter(frames);

        let mut conn = Connection::new(cdp.clone(), stream);
        conn.run().await;

        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("timeout")
            .expect("recv");
        assert_eq!(msg.id, Some(5));
    }

    #[tokio::test]
    async fn partial_json_is_ignored() {
        let cdp = Arc::new(CdpClient::new());
        let mut events = cdp.subscribe_events();

        let frames = vec![
            ok(Frame::Text(r#"{"id":1,"method":"Page"#.to_string())),
            ok(Frame::Close),
        ];
        let stream = stream::iter(frames);

        let mut conn = Connection::new(cdp.clone(), stream);
        conn.run().await;

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), events.recv())
                .await
                .is_err()
        );
    }

    #[test]
    fn connection_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Connection<stream::Iter<std::vec::IntoIter<Result<Frame, ConnectionError>>>>>(
        );
    }
}
