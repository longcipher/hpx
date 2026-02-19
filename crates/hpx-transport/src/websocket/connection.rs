use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use hpx::{
    Client,
    ws::{WebSocket, WebSocketRead, WebSocketRequestBuilder, WebSocketWrite, message::Message},
};
use serde::{Serialize, de::DeserializeOwned};
use tokio::{
    sync::{broadcast, mpsc},
    time::{sleep, timeout},
};
use tracing::warn;

use super::{
    config::WsConfig,
    pending::PendingRequestStore,
    protocol::{ProtocolHandler, WsMessage},
    subscription::SubscriptionStore,
    types::{MessageKind, RequestId, Topic},
};
use crate::{
    error::{TransportError, TransportResult},
    reconnect::{BackoffConfig, calculate_backoff},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionEpoch(pub u64);

/// Connection state machine states.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    /// Not connected.
    Disconnected,
    /// Attempting to connect.
    Connecting,
    /// WebSocket connection established.
    Connected,
    /// Performing authentication.
    Authenticating,
    /// Fully connected and authenticated, ready for traffic.
    Ready,
    /// Reconnecting after a disconnect.
    Reconnecting {
        /// Current attempt number.
        attempt: u32,
    },
    /// Gracefully closing.
    Closing,
    /// Fully closed, will not reconnect.
    Closed,
}

impl ConnectionState {
    /// Check if the connection is ready for traffic.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    /// Check if the connection is closed (terminal state).
    pub fn is_closed(&self) -> bool {
        matches!(self, Self::Closed)
    }
}

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

pub struct SubscriptionGuard {
    topic: Topic,
    subs: Arc<SubscriptionStore>,
    cmd_tx: mpsc::Sender<DataCommand>,
    rx: Option<broadcast::Receiver<WsMessage>>,
}

impl SubscriptionGuard {
    fn new(
        topic: Topic,
        subs: Arc<SubscriptionStore>,
        cmd_tx: mpsc::Sender<DataCommand>,
        rx: broadcast::Receiver<WsMessage>,
    ) -> Self {
        Self {
            topic,
            subs,
            cmd_tx,
            rx: Some(rx),
        }
    }

    pub fn into_receiver(mut self) -> broadcast::Receiver<WsMessage> {
        self.rx.take().expect("subscription receiver already taken")
    }

    /// Receive the next message from this subscription.
    pub async fn recv(&mut self) -> Option<WsMessage> {
        let rx = self.rx.as_mut()?;
        rx.recv().await.ok()
    }

    /// Try to receive a message without blocking.
    pub fn try_recv(&mut self) -> Option<WsMessage> {
        let rx = self.rx.as_mut()?;
        rx.try_recv().ok()
    }
}

impl Drop for SubscriptionGuard {
    fn drop(&mut self) {
        if self.rx.is_none() {
            return;
        }

        if let Some(remaining) = self.subs.decrement_ref(&self.topic)
            && remaining == 0
        {
            let cmd_tx = self.cmd_tx.clone();
            let topic = self.topic.clone();
            let unsubscribe = DataCommand::Unsubscribe {
                topics: vec![topic],
            };

            match cmd_tx.try_send(unsubscribe) {
                Ok(()) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    warn!("Connection task closed while dropping SubscriptionGuard");
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(unsubscribe)) => {
                    match tokio::runtime::Handle::try_current() {
                        Ok(handle) => {
                            handle.spawn(async move {
                                if cmd_tx.send(unsubscribe).await.is_err() {
                                    warn!(
                                        "Connection task closed while sending unsubscribe on drop"
                                    );
                                }
                            });
                        }
                        Err(_) => {
                            if cmd_tx.blocking_send(unsubscribe).is_err() {
                                warn!(
                                    "Connection task closed while blocking_send unsubscribe on drop"
                                );
                            }
                        }
                    }
                }
            }
        }
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
        self.request_with_timeout(req, None).await
    }

    pub async fn request_with_timeout<R, T>(
        &self,
        req: &R,
        timeout_override: Option<Duration>,
    ) -> TransportResult<T>
    where
        R: Serialize,
        T: DeserializeOwned,
    {
        let json = serde_json::to_string(req)?;
        let response = self
            .request_raw_with_timeout(WsMessage::text(json), timeout_override)
            .await?;
        let parsed = serde_json::from_str(&response)?;
        Ok(parsed)
    }

    pub async fn request_raw(&self, message: WsMessage) -> TransportResult<String> {
        self.request_raw_with_timeout(message, None).await
    }

    pub async fn request_raw_with_timeout(
        &self,
        message: WsMessage,
        timeout_override: Option<Duration>,
    ) -> TransportResult<String> {
        let timeout_duration = timeout_override.unwrap_or(self.config.request_timeout);

        if !self.pending.has_capacity() {
            return Err(TransportError::capacity_exceeded(
                "Too many pending requests",
            ));
        }

        let request_id = RequestId::new();
        let rx = match self.pending.add(request_id.clone(), timeout_override) {
            Some(rx) => rx,
            None => {
                return Err(TransportError::capacity_exceeded(
                    "Too many pending requests",
                ));
            }
        };

        let started = Instant::now();
        match timeout(
            timeout_duration,
            self.cmd_tx.send(DataCommand::Request {
                message,
                request_id: request_id.clone(),
            }),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                self.pending.remove(&request_id);
                return Err(TransportError::connection_closed(Some(
                    "Connection task shut down".to_string(),
                )));
            }
            Err(_) => {
                self.pending.remove(&request_id);
                return Err(TransportError::request_timeout(
                    timeout_duration,
                    request_id.to_string(),
                ));
            }
        }

        let remaining = timeout_duration.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            self.pending.remove(&request_id);
            return Err(TransportError::request_timeout(
                timeout_duration,
                request_id.to_string(),
            ));
        }

        match timeout(remaining, rx).await {
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

        Ok(SubscriptionGuard::new(
            topic,
            Arc::clone(&self.subs),
            self.cmd_tx.clone(),
            rx,
        ))
    }

    pub async fn subscribe_many(
        &self,
        topics: impl IntoIterator<Item = impl Into<Topic>>,
    ) -> TransportResult<Vec<SubscriptionGuard>> {
        let topics: Vec<Topic> = topics.into_iter().map(Into::into).collect();
        let mut guards = Vec::with_capacity(topics.len());
        let mut new_topics = Vec::new();

        for topic in &topics {
            let (rx, is_new) = self.subs.subscribe(topic.clone());
            guards.push(SubscriptionGuard::new(
                topic.clone(),
                Arc::clone(&self.subs),
                self.cmd_tx.clone(),
                rx,
            ));
            if is_new {
                new_topics.push(topic.clone());
            }
        }

        if !new_topics.is_empty() {
            let topics_to_send = new_topics.clone();
            self.cmd_tx
                .send(DataCommand::Subscribe {
                    topics: topics_to_send,
                })
                .await
                .map_err(|_| {
                    for topic in &new_topics {
                        self.subs.unsubscribe(topic);
                    }
                    TransportError::connection_closed(Some("Connection task shut down".to_string()))
                })?;
        }

        Ok(guards)
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

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn subscription_count(&self) -> usize {
        self.subs.len()
    }

    pub fn subscribed_topics(&self) -> Vec<Topic> {
        self.subs.get_all_topics()
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

pub struct Connection {
    handle: ConnectionHandle,
    stream: ConnectionStream,
}

impl Connection {
    pub async fn connect<H>(
        config: WsConfig,
        handler: H,
    ) -> TransportResult<(ConnectionHandle, ConnectionStream)>
    where
        H: ProtocolHandler,
    {
        let connection = Self::connect_stream(config, handler).await?;
        Ok(connection.split())
    }

    pub async fn connect_stream<H>(config: WsConfig, handler: H) -> TransportResult<Self>
    where
        H: ProtocolHandler,
    {
        config.validate().map_err(TransportError::config)?;

        let config = Arc::new(config);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(config.command_channel_capacity);
        let (cmd_tx, cmd_rx) = mpsc::channel(config.command_channel_capacity);
        let (event_tx, event_rx) = mpsc::channel(config.event_channel_capacity);
        let pending = Arc::new(PendingRequestStore::new(Arc::clone(&config)));
        let subs = Arc::new(SubscriptionStore::new(Arc::clone(&config)));

        let handle = ConnectionHandle {
            ctrl_tx,
            cmd_tx,
            pending: Arc::clone(&pending),
            subs: Arc::clone(&subs),
            config: Arc::clone(&config),
        };

        tokio::spawn(connection_driver(
            Arc::clone(&config),
            handler,
            ctrl_rx,
            cmd_rx,
            event_tx,
            pending,
            subs,
        ));

        let stream = ConnectionStream::new(event_rx);

        Ok(Self { handle, stream })
    }

    pub fn split(self) -> (ConnectionHandle, ConnectionStream) {
        (self.handle, self.stream)
    }

    pub fn handle(&self) -> &ConnectionHandle {
        &self.handle
    }
}

impl Stream for Connection {
    type Item = Event;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        Pin::new(&mut this.stream).poll_next(cx)
    }
}

#[async_trait]
pub(crate) trait WsWriter: Send {
    async fn send_ws(&mut self, message: Message) -> TransportResult<()>;
}

#[async_trait]
impl WsWriter for WebSocketWrite {
    async fn send_ws(&mut self, message: Message) -> TransportResult<()> {
        self.send(message)
            .await
            .map_err(|e| TransportError::websocket(e.to_string()))
    }
}

async fn send_ws_message<W>(ws_write: &mut W, msg: &WsMessage) -> TransportResult<()>
where
    W: WsWriter,
{
    let message = match msg {
        WsMessage::Text(text) => Message::text(text),
        WsMessage::Binary(data) => Message::binary(data.clone()),
    };

    ws_write.send_ws(message).await
}

fn message_to_text<H: ProtocolHandler>(handler: &H, msg: &Message) -> Option<String> {
    match msg {
        Message::Text(text) => Some(text.to_string()),
        Message::Binary(data) => handler.decode_binary(data).ok(),
        _ => None,
    }
}

async fn connect_websocket(config: &WsConfig) -> TransportResult<WebSocket> {
    let client = Client::builder()
        .connect_timeout(config.connect_timeout)
        .build()
        .map_err(|e| TransportError::config(format!("Failed to build client: {e}")))?;

    let req = client.get(&config.url);
    let ws_req = WebSocketRequestBuilder::new(req);
    let resp = timeout(config.connect_timeout, ws_req.send())
        .await
        .map_err(|_| TransportError::timeout(config.connect_timeout))?
        .map_err(TransportError::Http)?;

    timeout(config.connect_timeout, resp.into_websocket())
        .await
        .map_err(|_| TransportError::timeout(config.connect_timeout))?
        .map_err(TransportError::Http)
}

async fn establish_connection<H>(
    config: &WsConfig,
    handler: &H,
    subs: &SubscriptionStore,
    event_tx: &mpsc::Sender<Event>,
    epoch: &mut ConnectionEpoch,
) -> TransportResult<(WebSocketRead, WebSocketWrite)>
where
    H: ProtocolHandler,
{
    let ws = connect_websocket(config).await?;
    let (mut ws_write, mut ws_read) = ws.split();

    for msg in handler.on_connect() {
        send_ws_message(&mut ws_write, &msg).await?;
    }

    if config.auth_on_connect
        && let Some(auth_msg) = handler.build_auth_message()
    {
        send_ws_message(&mut ws_write, &auth_msg).await?;

        let timeout_duration = config.request_timeout;
        let start = Instant::now();

        loop {
            if start.elapsed() >= timeout_duration {
                return Err(TransportError::auth("Authentication timed out"));
            }

            let remaining = timeout_duration.saturating_sub(start.elapsed());
            match timeout(remaining, ws_read.next()).await {
                Ok(Some(Ok(msg))) => {
                    if let Some(text) = message_to_text(handler, &msg) {
                        if handler.is_auth_success(&text) {
                            break;
                        }
                        if handler.is_auth_failure(&text) {
                            return Err(TransportError::auth("Authentication failed"));
                        }
                    }
                }
                Ok(Some(Err(err))) => {
                    return Err(TransportError::websocket(err.to_string()));
                }
                Ok(None) => {
                    return Err(TransportError::connection_closed(None));
                }
                Err(_) => {
                    return Err(TransportError::auth("Authentication timed out"));
                }
            }
        }
    }

    let topics = subs.get_all_topics();
    if !topics.is_empty() {
        let request_id = handler.generate_request_id();
        let msg = handler.build_subscribe(&topics, request_id);
        send_ws_message(&mut ws_write, &msg).await?;
    }

    epoch.0 += 1;
    let connected_epoch = *epoch;
    let _ = event_tx
        .send(Event::Connected {
            epoch: connected_epoch,
        })
        .await;

    Ok((ws_read, ws_write))
}

async fn connection_driver<H>(
    config: Arc<WsConfig>,
    handler: H,
    mut ctrl_rx: mpsc::Receiver<ControlCommand>,
    mut cmd_rx: mpsc::Receiver<DataCommand>,
    event_tx: mpsc::Sender<Event>,
    pending: Arc<PendingRequestStore>,
    subs: Arc<SubscriptionStore>,
) -> TransportResult<()>
where
    H: ProtocolHandler,
{
    let mut attempt: u32 = 0;
    let mut epoch = ConnectionEpoch(0);

    loop {
        let connection =
            establish_connection(&config, &handler, &subs, &event_tx, &mut epoch).await;
        let (ws_read, ws_write) = match connection {
            Ok(parts) => {
                attempt = 0;
                parts
            }
            Err(err) => {
                let reason = err.to_string();
                let _ = event_tx
                    .send(Event::Disconnected {
                        epoch,
                        reason: reason.clone(),
                    })
                    .await;
                pending.clear_with_error(&reason);

                if let Some(max) = config.reconnect_max_attempts
                    && attempt >= max
                {
                    return Err(err);
                }

                let delay = calculate_backoff(
                    BackoffConfig {
                        initial_delay: config.reconnect_initial_delay,
                        max_delay: config.reconnect_max_delay,
                        factor: config.reconnect_backoff_factor,
                        jitter: config.reconnect_jitter,
                    },
                    attempt,
                );
                attempt = attempt.saturating_add(1);
                sleep(delay).await;
                continue;
            }
        };

        let ws_read =
            ws_read.map(|result| result.map_err(|err| TransportError::websocket(err.to_string())));

        match connection_task(
            &config,
            &handler,
            &mut ctrl_rx,
            &mut cmd_rx,
            &event_tx,
            &pending,
            &subs,
            ws_read,
            ws_write,
        )
        .await
        {
            Ok(()) => return Ok(()),
            Err(err) => {
                let reason = err.to_string();
                let _ = event_tx
                    .send(Event::Disconnected {
                        epoch,
                        reason: reason.clone(),
                    })
                    .await;
                pending.clear_with_error(&reason);

                if let Some(max) = config.reconnect_max_attempts
                    && attempt >= max
                {
                    return Err(err);
                }

                let delay = calculate_backoff(
                    BackoffConfig {
                        initial_delay: config.reconnect_initial_delay,
                        max_delay: config.reconnect_max_delay,
                        factor: config.reconnect_backoff_factor,
                        jitter: config.reconnect_jitter,
                    },
                    attempt,
                );
                attempt = attempt.saturating_add(1);
                sleep(delay).await;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn connection_task<H, R, W>(
    config: &WsConfig,
    handler: &H,
    ctrl_rx: &mut mpsc::Receiver<ControlCommand>,
    cmd_rx: &mut mpsc::Receiver<DataCommand>,
    event_tx: &mpsc::Sender<Event>,
    pending: &PendingRequestStore,
    subs: &SubscriptionStore,
    mut ws_read: R,
    mut ws_write: W,
) -> TransportResult<()>
where
    H: ProtocolHandler,
    R: Stream<Item = TransportResult<Message>> + Unpin,
    W: WsWriter,
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
                            return Err(TransportError::websocket(format!(
                                "Failed to send subscribe command: {err}"
                            )));
                        }
                    }
                    Some(DataCommand::Unsubscribe { topics }) => {
                        let request_id = handler.generate_request_id();
                        let msg = handler.build_unsubscribe(&topics, request_id);
                        if let Err(err) = send_ws_message(&mut ws_write, &msg).await {
                            return Err(TransportError::websocket(format!(
                                "Failed to send unsubscribe command: {err}"
                            )));
                        }
                    }
                    Some(DataCommand::Send { message }) => {
                        if let Err(err) = send_ws_message(&mut ws_write, &message).await {
                            return Err(TransportError::websocket(format!(
                                "Failed to send message command: {err}"
                            )));
                        }
                    }
                    Some(DataCommand::Request { message, request_id }) => {
                        let message = handler.inject_request_id(message, &request_id);
                        if let Err(err) = send_ws_message(&mut ws_write, &message).await {
                            pending.resolve(&request_id, Err(TransportError::websocket(err.to_string())));
                            return Err(TransportError::websocket(format!(
                                "Failed to send request command: {err}"
                            )));
                        }
                    }
                    None => return Ok(()),
                }
            }
            msg = ws_read.next() => {
                match msg {
                    Some(Ok(msg)) => {
                        // Enforce max_message_size.
                        let msg_size = match &msg {
                            Message::Text(t) => t.len(),
                            Message::Binary(b) => b.len(),
                            _ => 0,
                        };
                        if config.max_message_size > 0 && msg_size > config.max_message_size {
                            warn!(
                                size = msg_size,
                                max = config.max_message_size,
                                "Dropping oversized WebSocket message"
                            );
                            continue;
                        }

                        match &msg {
                            Message::Pong(_) => {
                                last_pong = Instant::now();
                                continue;
                            }
                            Message::Ping(data) => {
                                let _ = ws_write.send_ws(Message::Pong(data.clone())).await;
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
                                if let Err(err) = event_tx.try_send(Event::Message(incoming)) {
                                    warn!(error = %err, "Dropping event message due to backpressure");
                                }
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
                    let _ = ws_write.send_ws(Message::Ping(Bytes::new())).await;
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
    use std::{
        collections::VecDeque,
        sync::{
            Arc, Mutex,
            atomic::{AtomicU64, Ordering},
        },
        time::Duration,
    };

    use async_trait::async_trait;
    use futures_util::{SinkExt, StreamExt};
    use hpx_yawc::{
        Options, WebSocket,
        frame::{Frame, OpCode},
    };
    use http_body_util::Empty;
    use hyper::{
        Request, Response,
        body::{Bytes, Incoming},
        server::conn::http1,
        service::service_fn,
    };
    use hyper_util::rt::TokioIo;
    use serde_json::json;
    use tokio::{
        net::TcpListener,
        sync::{mpsc, oneshot},
        time::timeout,
    };

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

    struct SinkWriter<W> {
        inner: W,
    }

    impl<W> SinkWriter<W> {
        fn new(inner: W) -> Self {
            Self { inner }
        }
    }

    #[async_trait]
    impl<W> WsWriter for SinkWriter<W>
    where
        W: futures_util::Sink<Message, Error = TransportError> + Unpin + Send,
    {
        async fn send_ws(&mut self, message: Message) -> TransportResult<()> {
            self.inner.send(message).await
        }
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

    #[test]
    fn subscription_guard_drop_without_runtime_still_unsubscribes() {
        let config = test_config();
        let subs = Arc::new(SubscriptionStore::new(config));
        let topic = Topic::from("trades.BTC");
        let (rx, is_new) = subs.subscribe(topic.clone());
        assert!(is_new);

        let (cmd_tx, mut cmd_rx) = mpsc::channel(1);
        {
            let _guard = SubscriptionGuard::new(topic.clone(), Arc::clone(&subs), cmd_tx, rx);
        }

        let cmd = cmd_rx.try_recv().expect("expected unsubscribe command");
        assert!(matches!(
            cmd,
            DataCommand::Unsubscribe { topics } if topics == vec![topic]
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

        let (ctrl_tx, mut ctrl_rx) = mpsc::channel(1);
        let (cmd_tx, mut cmd_rx) = mpsc::channel(1);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let pending = Arc::new(PendingRequestStore::new(Arc::clone(&config)));
        let subs = Arc::new(SubscriptionStore::new(Arc::clone(&config)));

        let ws_read = futures_util::stream::iter(vec![Ok(Message::text("control"))]);
        let ws_write = SinkWriter::new(
            futures_util::sink::drain()
                .sink_map_err(|_| TransportError::internal("drain sink should not error")),
        );

        let handler = TestHandler;
        let task = tokio::spawn(async move {
            connection_task(
                &config,
                &handler,
                &mut ctrl_rx,
                &mut cmd_rx,
                &event_tx,
                &pending,
                &subs,
                ws_read,
                ws_write,
            )
            .await
        });

        let event = event_rx.recv().await.expect("event");
        match event {
            Event::Message(msg) => assert_eq!(msg.kind, MessageKind::Control),
            other => panic!("unexpected event: {other:?}"),
        }

        drop(ctrl_tx);
        drop(cmd_tx);
        let _ = task.await;
    }

    #[tokio::test]
    async fn connection_task_event_backpressure_does_not_block() {
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

        let (ctrl_tx, mut ctrl_rx) = mpsc::channel(1);
        let (cmd_tx, mut cmd_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(1);
        let pending = Arc::new(PendingRequestStore::new(Arc::clone(&config)));
        let subs = Arc::new(SubscriptionStore::new(Arc::clone(&config)));

        event_tx
            .send(Event::Connected {
                epoch: ConnectionEpoch(1),
            })
            .await
            .expect("seed event");

        let ws_read = futures_util::stream::iter(vec![Ok(Message::text("control"))]);
        let ws_write = SinkWriter::new(
            futures_util::sink::drain()
                .sink_map_err(|_| TransportError::internal("drain sink should not error")),
        );

        let handler = TestHandler;
        let task = tokio::spawn(async move {
            connection_task(
                &config,
                &handler,
                &mut ctrl_rx,
                &mut cmd_rx,
                &event_tx,
                &pending,
                &subs,
                ws_read,
                ws_write,
            )
            .await
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        let _ = ctrl_tx.send(ControlCommand::Close).await;

        let task_result = timeout(Duration::from_secs(1), task)
            .await
            .expect("task should not block on full event channel");
        let _ = task_result.expect("task join failed");

        drop(cmd_tx);
    }

    struct AuthServerState {
        conn_count: AtomicU64,
        auth_tx: Mutex<Option<oneshot::Sender<()>>>,
        sub_tx: Mutex<Option<oneshot::Sender<()>>>,
    }

    async fn start_auth_server(state: Arc<AuthServerState>) -> std::io::Result<String> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(_) => return,
                };

                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    state.conn_count.fetch_add(1, Ordering::Relaxed);
                    let io = TokioIo::new(stream);
                    let conn_fut = http1::Builder::new()
                        .serve_connection(
                            io,
                            service_fn(move |req| auth_server_upgrade(req, Arc::clone(&state))),
                        )
                        .with_upgrades();
                    let _ = conn_fut.await;
                });
            }
        });

        Ok(format!("ws://{}", addr))
    }

    async fn auth_server_upgrade(
        mut req: Request<Incoming>,
        state: Arc<AuthServerState>,
    ) -> hpx_yawc::Result<Response<Empty<Bytes>>> {
        let (response, fut) = WebSocket::upgrade_with_options(&mut req, Options::default())?;

        tokio::spawn(async move {
            let mut ws = match fut.await {
                Ok(ws) => ws,
                Err(_) => return,
            };

            while let Some(frame) = ws.next().await {
                if frame.opcode() != OpCode::Text {
                    continue;
                }

                let text = match std::str::from_utf8(frame.payload()) {
                    Ok(text) => text,
                    Err(_) => continue,
                };

                if text == "AUTH" {
                    let _ = ws.send(Frame::text("AUTH_OK")).await;
                    if let Some(tx) = state.auth_tx.lock().unwrap().take() {
                        let _ = tx.send(());
                    }
                    continue;
                }

                if text.starts_with("SUB:") {
                    if let Some(tx) = state.sub_tx.lock().unwrap().take() {
                        let _ = tx.send(());
                    }
                }
            }
        });

        Ok(response)
    }

    struct ReconnectServerState {
        conn_count: AtomicU64,
        sub_txs: Mutex<VecDeque<oneshot::Sender<()>>>,
    }

    async fn start_reconnect_server(state: Arc<ReconnectServerState>) -> std::io::Result<String> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(_) => return,
                };

                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let conn_fut = http1::Builder::new()
                        .serve_connection(
                            io,
                            service_fn(move |req| {
                                reconnect_server_upgrade(req, Arc::clone(&state))
                            }),
                        )
                        .with_upgrades();
                    let _ = conn_fut.await;
                });
            }
        });

        Ok(format!("ws://{}", addr))
    }

    async fn reconnect_server_upgrade(
        mut req: Request<Incoming>,
        state: Arc<ReconnectServerState>,
    ) -> hpx_yawc::Result<Response<Empty<Bytes>>> {
        let (response, fut) = WebSocket::upgrade_with_options(&mut req, Options::default())?;

        tokio::spawn(async move {
            let mut ws = match fut.await {
                Ok(ws) => ws,
                Err(_) => return,
            };

            let conn_index = state.conn_count.fetch_add(1, Ordering::Relaxed);
            let should_close_after_sub = conn_index == 0;

            while let Some(frame) = ws.next().await {
                if frame.opcode() != OpCode::Text {
                    continue;
                }

                let text = match std::str::from_utf8(frame.payload()) {
                    Ok(text) => text,
                    Err(_) => continue,
                };

                if text.starts_with("SUB:") {
                    if let Some(tx) = state.sub_txs.lock().unwrap().pop_front() {
                        let _ = tx.send(());
                    }

                    if should_close_after_sub {
                        return;
                    }
                }
            }
        });

        Ok(response)
    }

    #[tokio::test]
    async fn establish_connection_auth_and_resubscribe() -> TransportResult<()> {
        struct AuthHandler;

        impl ProtocolHandler for AuthHandler {
            fn build_auth_message(&self) -> Option<WsMessage> {
                Some(WsMessage::text("AUTH"))
            }

            fn is_auth_success(&self, message: &str) -> bool {
                message == "AUTH_OK"
            }

            fn is_auth_failure(&self, message: &str) -> bool {
                message == "AUTH_FAIL"
            }

            fn classify_message(&self, _message: &str) -> MessageKind {
                MessageKind::Unknown
            }

            fn extract_request_id(&self, _message: &str) -> Option<RequestId> {
                None
            }

            fn extract_topic(&self, _message: &str) -> Option<Topic> {
                None
            }

            fn build_subscribe(&self, topics: &[Topic], _request_id: RequestId) -> WsMessage {
                let topic = topics.first().map(|t| t.as_str()).unwrap_or("unknown");
                WsMessage::text(format!("SUB:{topic}"))
            }

            fn build_unsubscribe(&self, topics: &[Topic], _request_id: RequestId) -> WsMessage {
                let topic = topics.first().map(|t| t.as_str()).unwrap_or("unknown");
                WsMessage::text(format!("UNSUB:{topic}"))
            }
        }

        let (auth_tx, auth_rx) = oneshot::channel();
        let (sub_tx, sub_rx) = oneshot::channel();
        let state = Arc::new(AuthServerState {
            conn_count: AtomicU64::new(0),
            auth_tx: Mutex::new(Some(auth_tx)),
            sub_tx: Mutex::new(Some(sub_tx)),
        });

        let url = start_auth_server(Arc::clone(&state)).await.expect("server");
        let mut config = WsConfig::new(url).auth_on_connect(true);
        config.request_timeout = Duration::from_secs(1);
        let config = Arc::new(config);

        let subs = SubscriptionStore::new(Arc::clone(&config));
        let _ = subs.subscribe(Topic::from("trades.BTC"));

        let (event_tx, mut event_rx) = mpsc::channel(2);
        let mut epoch = ConnectionEpoch(0);

        let _ = establish_connection(&config, &AuthHandler, &subs, &event_tx, &mut epoch).await?;

        timeout(Duration::from_secs(1), auth_rx)
            .await
            .map_err(|_| TransportError::internal("auth timeout"))?
            .map_err(|_| TransportError::internal("auth channel closed"))?;

        timeout(Duration::from_secs(1), sub_rx)
            .await
            .map_err(|_| TransportError::internal("subscribe timeout"))?
            .map_err(|_| TransportError::internal("subscribe channel closed"))?;

        let event = timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .map_err(|_| TransportError::internal("event timeout"))?
            .ok_or_else(|| TransportError::internal("event channel closed"))?;

        match event {
            Event::Connected {
                epoch: connected_epoch,
            } => {
                assert_eq!(connected_epoch.0, 1);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        assert_eq!(epoch.0, 1);
        assert_eq!(state.conn_count.load(Ordering::Relaxed), 1);

        Ok(())
    }

    #[test]
    fn calculate_backoff_without_jitter_is_deterministic() {
        let config = BackoffConfig {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(1000),
            factor: 2.0,
            jitter: 0.0,
        };

        assert_eq!(calculate_backoff(config, 0), Duration::from_millis(100));
        assert_eq!(calculate_backoff(config, 1), Duration::from_millis(200));
        assert_eq!(calculate_backoff(config, 2), Duration::from_millis(400));
        assert_eq!(calculate_backoff(config, 3), Duration::from_millis(800));
        assert_eq!(calculate_backoff(config, 4), Duration::from_millis(1000));
    }

    #[tokio::test]
    async fn connection_driver_reconnects_and_resubscribes() -> TransportResult<()> {
        struct ReconnectHandler;

        impl ProtocolHandler for ReconnectHandler {
            fn classify_message(&self, _message: &str) -> MessageKind {
                MessageKind::Unknown
            }

            fn extract_request_id(&self, _message: &str) -> Option<RequestId> {
                None
            }

            fn extract_topic(&self, _message: &str) -> Option<Topic> {
                None
            }

            fn build_subscribe(&self, topics: &[Topic], _request_id: RequestId) -> WsMessage {
                let topic = topics.first().map(|t| t.as_str()).unwrap_or("unknown");
                WsMessage::text(format!("SUB:{topic}"))
            }

            fn build_unsubscribe(&self, topics: &[Topic], _request_id: RequestId) -> WsMessage {
                let topic = topics.first().map(|t| t.as_str()).unwrap_or("unknown");
                WsMessage::text(format!("UNSUB:{topic}"))
            }
        }

        let (first_tx, first_rx) = oneshot::channel();
        let (second_tx, second_rx) = oneshot::channel();
        let state = Arc::new(ReconnectServerState {
            conn_count: AtomicU64::new(0),
            sub_txs: Mutex::new(VecDeque::from([first_tx, second_tx])),
        });

        let url = start_reconnect_server(Arc::clone(&state))
            .await
            .expect("server");
        let mut config = WsConfig::new(url);
        config.reconnect_initial_delay = Duration::from_millis(10);
        config.reconnect_max_delay = Duration::from_millis(20);
        config.reconnect_backoff_factor = 2.0;
        config.reconnect_jitter = 0.0;
        config.ping_interval = Duration::from_secs(60);
        config.pending_cleanup_interval = Duration::from_secs(60);
        let config = Arc::new(config);

        let pending = Arc::new(PendingRequestStore::new(Arc::clone(&config)));
        let subs = Arc::new(SubscriptionStore::new(Arc::clone(&config)));
        let _ = subs.subscribe(Topic::from("trades.BTC"));

        let (ctrl_tx, ctrl_rx) = mpsc::channel(1);
        let (cmd_tx, cmd_rx) = mpsc::channel(1);
        let (event_tx, mut event_rx) = mpsc::channel(4);

        let driver = tokio::spawn(connection_driver(
            Arc::clone(&config),
            ReconnectHandler,
            ctrl_rx,
            cmd_rx,
            event_tx,
            Arc::clone(&pending),
            Arc::clone(&subs),
        ));

        timeout(Duration::from_secs(1), first_rx)
            .await
            .map_err(|_| TransportError::internal("first subscribe timeout"))?
            .map_err(|_| TransportError::internal("first subscribe channel closed"))?;

        let mut saw_connected = false;
        let mut saw_disconnected = false;
        for _ in 0..3 {
            if let Ok(Some(event)) = timeout(Duration::from_secs(1), event_rx.recv()).await {
                match event {
                    Event::Connected { .. } => saw_connected = true,
                    Event::Disconnected { .. } => saw_disconnected = true,
                    _ => {}
                }
            }
            if saw_connected && saw_disconnected {
                break;
            }
        }

        timeout(Duration::from_secs(1), second_rx)
            .await
            .map_err(|_| TransportError::internal("second subscribe timeout"))?
            .map_err(|_| TransportError::internal("second subscribe channel closed"))?;

        assert!(saw_connected, "expected at least one Connected event");
        assert!(saw_disconnected, "expected a Disconnected event after drop");
        assert!(state.conn_count.load(Ordering::Relaxed) >= 2);

        let _ = ctrl_tx.send(ControlCommand::Close).await;
        drop(cmd_tx);
        let _ = driver.await;

        Ok(())
    }
}
