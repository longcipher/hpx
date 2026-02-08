use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Instant,
};

use bytes::Bytes;
use futures_util::{SinkExt, Stream, StreamExt};
use hpx::ws::message::Message;
use serde::{Serialize, de::DeserializeOwned};
use tokio::{
    sync::{broadcast, mpsc},
    time::timeout,
};
use tracing::warn;

use super::{
    config::WsConfig,
    pending::PendingRequestStore,
    protocol::{ProtocolHandler, WsMessage},
    subscription::SubscriptionStore,
    types::{MessageKind, RequestId, Topic},
};
use crate::error::{TransportError, TransportResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionEpoch(pub u64);

#[derive(Debug, Clone)]
pub enum ControlCommand {
    Close,
    Reconnect { reason: String },
}

#[derive(Debug, Clone)]
pub enum DataCommand {
    Subscribe {
        topics: Vec<Topic>,
    },
    Unsubscribe {
        topics: Vec<Topic>,
    },
    Send {
        message: WsMessage,
    },
    Request {
        message: WsMessage,
        request_id: RequestId,
    },
}

#[derive(Debug, Clone)]
pub struct IncomingMessage {
    pub raw: WsMessage,
    pub text: Option<String>,
    pub kind: MessageKind,
    pub topic: Option<Topic>,
}

#[derive(Debug, Clone)]
pub enum Event {
    Connected {
        epoch: ConnectionEpoch,
    },
    Disconnected {
        epoch: ConnectionEpoch,
        reason: String,
    },
    Message(IncomingMessage),
}

#[derive(Debug)]
pub struct SubscriptionGuard {
    rx: broadcast::Receiver<WsMessage>,
}

impl SubscriptionGuard {
    fn new(rx: broadcast::Receiver<WsMessage>) -> Self {
        Self { rx }
    }

    pub async fn recv(&mut self) -> Option<WsMessage> {
        self.rx.recv().await.ok()
    }
}

#[derive(Clone)]
pub struct ConnectionHandle {
    ctrl_tx: mpsc::Sender<ControlCommand>,
    cmd_tx: mpsc::Sender<DataCommand>,
    pending: Arc<PendingRequestStore>,
    subs: Arc<SubscriptionStore>,
    config: Arc<WsConfig>,
}

impl ConnectionHandle {
    pub async fn request<R, T>(&self, req: &R) -> TransportResult<T>
    where
        R: Serialize,
        T: DeserializeOwned,
    {
        let json = serde_json::to_string(req)?;
        let response = self.request_raw(WsMessage::text(json)).await?;
        let parsed = serde_json::from_str(&response)?;
        Ok(parsed)
    }

    async fn request_raw(&self, message: WsMessage) -> TransportResult<String> {
        if !self.pending.has_capacity() {
            return Err(TransportError::capacity_exceeded(
                "Too many pending requests",
            ));
        }

        let request_id = RequestId::new();
        let rx = match self.pending.add(request_id.clone(), None) {
            Some(rx) => rx,
            None => {
                return Err(TransportError::capacity_exceeded(
                    "Too many pending requests",
                ));
            }
        };

        self.cmd_tx
            .send(DataCommand::Request {
                message,
                request_id: request_id.clone(),
            })
            .await
            .map_err(|_| {
                self.pending.remove(&request_id);
                TransportError::connection_closed(Some("Connection task shut down".to_string()))
            })?;

        let timeout_duration = self.config.request_timeout;
        match timeout(timeout_duration, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(TransportError::internal("Request channel dropped")),
            Err(_) => {
                self.pending.remove(&request_id);
                Err(TransportError::request_timeout(
                    timeout_duration,
                    request_id.to_string(),
                ))
            }
        }
    }

    pub async fn subscribe(&self, topic: impl Into<Topic>) -> TransportResult<SubscriptionGuard> {
        let topic = topic.into();
        let (rx, is_new) = self.subs.subscribe(topic.clone());

        if is_new {
            self.cmd_tx
                .send(DataCommand::Subscribe {
                    topics: vec![topic.clone()],
                })
                .await
                .map_err(|_| {
                    self.subs.unsubscribe(&topic);
                    TransportError::connection_closed(Some("Connection task shut down".to_string()))
                })?;
        }

        Ok(SubscriptionGuard::new(rx))
    }

    pub async fn unsubscribe(&self, topic: impl Into<Topic>) -> TransportResult<()> {
        let topic = topic.into();
        let was_last = self.subs.unsubscribe(&topic);

        if was_last {
            self.cmd_tx
                .send(DataCommand::Unsubscribe {
                    topics: vec![topic],
                })
                .await
                .map_err(|_| {
                    TransportError::connection_closed(Some("Connection task shut down".to_string()))
                })?;
        }

        Ok(())
    }

    pub async fn send(&self, message: WsMessage) -> TransportResult<()> {
        self.cmd_tx
            .send(DataCommand::Send { message })
            .await
            .map_err(|_| {
                TransportError::connection_closed(Some("Connection task shut down".to_string()))
            })
    }

    pub async fn close(&self) -> TransportResult<()> {
        self.ctrl_tx.send(ControlCommand::Close).await.map_err(|_| {
            TransportError::connection_closed(Some("Connection task shut down".to_string()))
        })
    }

    pub fn is_connected(&self) -> bool {
        !self.ctrl_tx.is_closed()
    }
}

pub struct ConnectionStream {
    rx: mpsc::Receiver<Event>,
}

impl ConnectionStream {
    pub fn new(rx: mpsc::Receiver<Event>) -> Self {
        Self { rx }
    }

    pub async fn next(&mut self) -> Option<Event> {
        self.rx.recv().await
    }
}

impl Stream for ConnectionStream {
    type Item = Event;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        Pin::new(&mut this.rx).poll_recv(cx)
    }
}

async fn send_ws_message<W>(ws_write: &mut W, msg: &WsMessage) -> TransportResult<()>
where
    W: futures_util::Sink<Message, Error = TransportError> + Unpin,
{
    let message = match msg {
        WsMessage::Text(text) => Message::text(text),
        WsMessage::Binary(data) => Message::binary(data.clone()),
    };

    ws_write.send(message).await
}

#[allow(clippy::too_many_arguments, dead_code)]
pub(crate) async fn connection_task<H, R, W>(
    config: Arc<WsConfig>,
    handler: H,
    mut ctrl_rx: mpsc::Receiver<ControlCommand>,
    mut cmd_rx: mpsc::Receiver<DataCommand>,
    event_tx: mpsc::Sender<Event>,
    pending: Arc<PendingRequestStore>,
    subs: Arc<SubscriptionStore>,
    mut ws_read: R,
    mut ws_write: W,
) -> TransportResult<()>
where
    H: ProtocolHandler,
    R: Stream<Item = TransportResult<Message>> + Unpin,
    W: futures_util::Sink<Message, Error = TransportError> + Unpin,
{
    let mut ping_interval = tokio::time::interval(config.ping_interval);
    let mut cleanup_interval = tokio::time::interval(config.pending_cleanup_interval);
    let mut last_pong = Instant::now();

    loop {
        tokio::select! {
            biased;
            ctrl = ctrl_rx.recv() => {
                match ctrl {
                    Some(ControlCommand::Close) => {
                        return Ok(());
                    }
                    Some(ControlCommand::Reconnect { reason }) => {
                        return Err(TransportError::websocket(reason));
                    }
                    None => return Ok(()),
                }
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(DataCommand::Subscribe { topics }) => {
                        let request_id = handler.generate_request_id();
                        let msg = handler.build_subscribe(&topics, request_id);
                        if let Err(err) = send_ws_message(&mut ws_write, &msg).await {
                            warn!(error = %err, "Failed to send subscribe");
                        }
                    }
                    Some(DataCommand::Unsubscribe { topics }) => {
                        let request_id = handler.generate_request_id();
                        let msg = handler.build_unsubscribe(&topics, request_id);
                        if let Err(err) = send_ws_message(&mut ws_write, &msg).await {
                            warn!(error = %err, "Failed to send unsubscribe");
                        }
                    }
                    Some(DataCommand::Send { message }) => {
                        if let Err(err) = send_ws_message(&mut ws_write, &message).await {
                            warn!(error = %err, "Failed to send message");
                        }
                    }
                    Some(DataCommand::Request { message, request_id }) => {
                        let message = handler.inject_request_id(message, &request_id);
                        if let Err(err) = send_ws_message(&mut ws_write, &message).await {
                            pending.resolve(&request_id, Err(TransportError::websocket(err.to_string())));
                        }
                    }
                    None => return Ok(()),
                }
            }
            msg = ws_read.next() => {
                match msg {
                    Some(Ok(msg)) => {
                        match &msg {
                            Message::Pong(_) => {
                                last_pong = Instant::now();
                                continue;
                            }
                            Message::Ping(data) => {
                                let _ = ws_write.send(Message::Pong(data.clone())).await;
                                continue;
                            }
                            Message::Close(_) => {
                                return Err(TransportError::connection_closed(None));
                            }
                            _ => {}
                        }

                        let (raw, text) = match msg {
                            Message::Text(text) => {
                                let text = text.to_string();
                                (WsMessage::Text(text.clone()), Some(text))
                            }
                            Message::Binary(data) => {
                                let text = handler.decode_binary(&data).ok();
                                (WsMessage::Binary(data.to_vec()), text)
                            }
                            _ => {
                                continue;
                            }
                        };

                        let Some(text) = text else {
                            continue;
                        };

                        if handler.is_server_ping(&text) {
                            if let Some(pong) = handler.build_pong(text.as_bytes()) {
                                let _ = send_ws_message(&mut ws_write, &pong).await;
                            }
                            continue;
                        }

                        if handler.is_pong_response(&text) {
                            last_pong = Instant::now();
                            continue;
                        }

                        if handler.should_reconnect(&text) {
                            return Err(TransportError::websocket("Server requested reconnect"));
                        }

                        let kind = handler.classify_message(&text);
                        match kind {
                            MessageKind::Response => {
                                if let Some(request_id) = handler.extract_request_id(&text) {
                                    pending.resolve(&request_id, Ok(text));
                                }
                            }
                            MessageKind::Update => {
                                if let Some(topic) = handler.extract_topic(&text) {
                                    subs.publish(&topic, WsMessage::Text(text));
                                }
                            }
                            MessageKind::System | MessageKind::Control | MessageKind::Unknown => {
                                let topic = handler.extract_topic(&text);
                                let incoming = IncomingMessage {
                                    raw,
                                    text: Some(text),
                                    kind,
                                    topic,
                                };
                                let _ = event_tx.send(Event::Message(incoming)).await;
                            }
                        }
                    }
                    Some(Err(err)) => {
                        warn!(error = %err, "WebSocket read error");
                        return Err(err);
                    }
                    None => {
                        return Err(TransportError::connection_closed(None));
                    }
                }
            }
            _ = ping_interval.tick() => {
                if config.use_websocket_ping {
                    let _ = ws_write.send(Message::Ping(Bytes::new())).await;
                } else if let Some(ping) = handler.build_ping() {
                    let _ = send_ws_message(&mut ws_write, &ping).await;
                }

                if last_pong.elapsed() > config.pong_timeout {
                    return Err(TransportError::connection_closed(Some("Pong timeout".to_string())));
                }
            }
            _ = cleanup_interval.tick() => {
                pending.cleanup_stale_with_notify();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures_util::SinkExt;
    use serde_json::json;
    use tokio::sync::mpsc;

    use super::*;
    use crate::{error::TransportError, websocket::protocol::ProtocolHandler};

    fn test_config() -> Arc<WsConfig> {
        Arc::new(
            WsConfig::new("wss://test.com")
                .request_timeout(Duration::from_millis(50))
                .max_pending_requests(4),
        )
    }

    fn build_handle() -> (
        ConnectionHandle,
        mpsc::Receiver<DataCommand>,
        Arc<PendingRequestStore>,
    ) {
        let config = test_config();
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(1);
        let (cmd_tx, cmd_rx) = mpsc::channel(4);
        let pending = Arc::new(PendingRequestStore::new(Arc::clone(&config)));
        let subs = Arc::new(SubscriptionStore::new(Arc::clone(&config)));
        let handle = ConnectionHandle {
            ctrl_tx,
            cmd_tx,
            pending: Arc::clone(&pending),
            subs,
            config,
        };
        (handle, cmd_rx, pending)
    }

    #[test]
    fn connection_types_compile() {
        let _epoch = ConnectionEpoch(1);

        let _control = ControlCommand::Close;
        let _data = DataCommand::Subscribe {
            topics: vec![Topic::from("trades.BTC")],
        };

        let message = IncomingMessage {
            raw: WsMessage::text("update"),
            text: Some("update".to_string()),
            kind: MessageKind::Unknown,
            topic: None,
        };

        let _event = Event::Message(message);
    }

    #[tokio::test]
    async fn connection_handle_request_times_out_and_cleans_pending() {
        let (handle, mut cmd_rx, pending) = build_handle();
        let result: TransportResult<serde_json::Value> =
            handle.request(&json!({"op": "ping"})).await;

        assert!(matches!(result, Err(TransportError::RequestTimeout { .. })));
        assert_eq!(pending.len(), 0);

        let cmd = cmd_rx.recv().await.expect("expected request command");
        assert!(matches!(cmd, DataCommand::Request { .. }));
    }

    #[tokio::test]
    async fn connection_handle_subscribe_sends_command() {
        let (handle, mut cmd_rx, _pending) = build_handle();
        let _guard = handle.subscribe("trades.BTC").await.expect("subscribe");

        let cmd = cmd_rx.recv().await.expect("expected subscribe command");
        assert!(matches!(
            cmd,
            DataCommand::Subscribe { topics } if topics == vec![Topic::from("trades.BTC")]
        ));
    }

    #[tokio::test]
    async fn connection_handle_unsubscribe_sends_command_when_last() {
        let (handle, mut cmd_rx, _pending) = build_handle();
        let _guard = handle.subscribe("trades.BTC").await.expect("subscribe");
        let _ = cmd_rx.recv().await.expect("subscribe command");

        handle.unsubscribe("trades.BTC").await.expect("unsubscribe");

        let cmd = cmd_rx.recv().await.expect("expected unsubscribe command");
        assert!(matches!(
            cmd,
            DataCommand::Unsubscribe { topics } if topics == vec![Topic::from("trades.BTC")]
        ));
    }

    #[tokio::test]
    async fn connection_stream_yields_events() {
        let (tx, rx) = mpsc::channel(2);
        let mut stream = ConnectionStream::new(rx);

        tx.send(Event::Connected {
            epoch: ConnectionEpoch(1),
        })
        .await
        .expect("send");

        match stream.next().await {
            Some(Event::Connected { epoch }) => assert_eq!(epoch.0, 1),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn connection_task_emits_control_events() {
        struct TestHandler;

        impl ProtocolHandler for TestHandler {
            fn classify_message(&self, message: &str) -> MessageKind {
                if message == "control" {
                    MessageKind::Control
                } else {
                    MessageKind::Unknown
                }
            }

            fn extract_request_id(&self, _message: &str) -> Option<RequestId> {
                None
            }

            fn extract_topic(&self, _message: &str) -> Option<Topic> {
                None
            }

            fn build_subscribe(&self, _topics: &[Topic], _request_id: RequestId) -> WsMessage {
                WsMessage::text("sub")
            }

            fn build_unsubscribe(&self, _topics: &[Topic], _request_id: RequestId) -> WsMessage {
                WsMessage::text("unsub")
            }
        }

        let mut config = WsConfig::new("wss://test");
        config.ping_interval = Duration::from_secs(60);
        config.pending_cleanup_interval = Duration::from_secs(60);
        let config = Arc::new(config);

        let (ctrl_tx, ctrl_rx) = mpsc::channel(1);
        let (cmd_tx, cmd_rx) = mpsc::channel(1);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let pending = Arc::new(PendingRequestStore::new(Arc::clone(&config)));
        let subs = Arc::new(SubscriptionStore::new(Arc::clone(&config)));

        let ws_read = futures_util::stream::iter(vec![Ok(Message::text("control"))]);
        let ws_write = futures_util::sink::drain()
            .sink_map_err(|_| TransportError::internal("drain sink should not error"));

        let task = tokio::spawn(connection_task(
            config,
            TestHandler,
            ctrl_rx,
            cmd_rx,
            event_tx,
            pending,
            subs,
            ws_read,
            ws_write,
        ));

        let event = event_rx.recv().await.expect("event");
        match event {
            Event::Message(msg) => assert_eq!(msg.kind, MessageKind::Control),
            other => panic!("unexpected event: {other:?}"),
        }

        drop(ctrl_tx);
        drop(cmd_tx);
        let _ = task.await;
    }
}
