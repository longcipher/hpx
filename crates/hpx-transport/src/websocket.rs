//! High-performance WebSocket client with automatic reconnection.
//!
//! This module provides a robust, actor-based WebSocket client with support for:
//! - Automatic reconnection with exponential backoff
//! - Subscription management (Pub/Sub)
//! - Exchange-specific message routing via `ExchangeHandler` trait
//! - Thread-safe message sending

use std::{collections::HashMap, time::Duration};

use bytes::Bytes;
use futures_util::StreamExt;
use hpx::ws::{WebSocket, WebSocketWrite, message::Message};
use serde::Serialize;
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    time::sleep,
};
use tracing::{error, info, warn};

use crate::error::{TransportError, TransportResult};

/// Configuration for the WebSocket client.
#[derive(Debug, Clone)]
pub struct WebSocketConfig {
    /// WebSocket URL
    pub url: String,
    /// Initial delay before first reconnection attempt
    pub reconnect_initial_delay: Duration,
    /// Maximum delay between reconnection attempts
    pub reconnect_max_delay: Duration,
    /// Backoff multiplier for reconnection delay
    pub reconnect_backoff_factor: f64,
    /// Interval between ping messages
    pub ping_interval: Duration,
    /// Timeout for pong response
    pub pong_timeout: Duration,
    /// Channel capacity for subscriptions
    pub channel_capacity: usize,
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            reconnect_initial_delay: Duration::from_millis(100),
            reconnect_max_delay: Duration::from_secs(30),
            reconnect_backoff_factor: 2.0,
            ping_interval: Duration::from_secs(30),
            pong_timeout: Duration::from_secs(90),
            channel_capacity: 1024,
        }
    }
}

impl WebSocketConfig {
    /// Create a new configuration with the given URL.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Default::default()
        }
    }

    /// Set reconnection initial delay.
    pub fn reconnect_initial_delay(mut self, delay: Duration) -> Self {
        self.reconnect_initial_delay = delay;
        self
    }

    /// Set reconnection max delay.
    pub fn reconnect_max_delay(mut self, delay: Duration) -> Self {
        self.reconnect_max_delay = delay;
        self
    }

    /// Set ping interval.
    pub fn ping_interval(mut self, interval: Duration) -> Self {
        self.ping_interval = interval;
        self
    }
}

/// Trait for exchange-specific WebSocket logic.
pub trait ExchangeHandler: Send + Sync + 'static {
    /// Extract topic from a text message.
    fn get_topic(&self, message: &str) -> Option<String>;

    /// Build a subscribe message for the given topic.
    fn build_subscribe(&self, topic: &str) -> Message;

    /// Build an unsubscribe message for the given topic.
    fn build_unsubscribe(&self, topic: &str) -> Message;

    /// Called when the connection is established.
    /// Returns a list of messages to send immediately (e.g., authentication).
    fn on_connect(&self) -> Vec<Message> {
        vec![]
    }

    /// Decode binary data to text (e.g., decompress gzip).
    fn decode_binary(&self, data: &[u8]) -> Option<String> {
        String::from_utf8(data.to_vec()).ok()
    }

    /// Process incoming message and return routing info.
    /// Returns (topic, processed_message) or None if message should be ignored.
    fn process_message(&self, message: Message) -> Option<(String, Message)> {
        match message {
            Message::Text(ref text) => self.get_topic(text.as_str()).map(|topic| (topic, message)),
            Message::Binary(ref data) => self.decode_binary(data).and_then(|text| {
                self.get_topic(&text)
                    .map(|topic| (topic, Message::Text(text.into())))
            }),
            _ => None,
        }
    }
}

/// Commands sent from Client to Actor.
enum Command {
    Subscribe {
        topic: String,
        reply_tx: oneshot::Sender<broadcast::Receiver<Message>>,
    },
    Unsubscribe {
        topic: String,
    },
    Send(Message),
    Close,
}

/// The user-facing WebSocket client handle.
#[derive(Debug, Clone)]
pub struct WebSocketHandle {
    cmd_tx: mpsc::UnboundedSender<Command>,
}

impl WebSocketHandle {
    /// Create and start a new WebSocket client.
    pub async fn connect<H>(config: WebSocketConfig, handler: H) -> TransportResult<Self>
    where
        H: ExchangeHandler,
    {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let actor = WebSocketActor::new(config, handler, cmd_rx);

        tokio::spawn(actor.run());

        Ok(Self { cmd_tx })
    }

    /// Subscribe to a topic.
    /// Returns a broadcast receiver that receives messages for this topic.
    pub async fn subscribe(
        &self,
        topic: impl Into<String>,
    ) -> TransportResult<broadcast::Receiver<Message>> {
        let topic = topic.into();
        let (reply_tx, reply_rx) = oneshot::channel();

        self.cmd_tx
            .send(Command::Subscribe {
                topic: topic.clone(),
                reply_tx,
            })
            .map_err(|_| TransportError::internal("WebSocket actor is closed"))?;

        reply_rx
            .await
            .map_err(|_| TransportError::internal("Failed to receive subscription channel"))
    }

    /// Unsubscribe from a topic.
    pub async fn unsubscribe(&self, topic: impl Into<String>) -> TransportResult<()> {
        self.cmd_tx
            .send(Command::Unsubscribe {
                topic: topic.into(),
            })
            .map_err(|_| TransportError::internal("WebSocket actor is closed"))?;
        Ok(())
    }

    /// Send a raw message to the server.
    pub async fn send(&self, message: Message) -> TransportResult<()> {
        self.cmd_tx
            .send(Command::Send(message))
            .map_err(|_| TransportError::internal("WebSocket actor is closed"))?;
        Ok(())
    }

    /// Send a JSON message.
    pub async fn send_json<T: Serialize>(&self, payload: &T) -> TransportResult<()> {
        let text = serde_json::to_string(payload)?;
        self.send(Message::Text(text.into())).await
    }

    /// Close the connection.
    pub async fn close(&self) -> TransportResult<()> {
        let _ = self.cmd_tx.send(Command::Close);
        Ok(())
    }
}

/// The background actor managing the WebSocket connection.
struct WebSocketActor<H> {
    config: WebSocketConfig,
    handler: H,
    cmd_rx: mpsc::UnboundedReceiver<Command>,
    subscriptions: HashMap<String, broadcast::Sender<Message>>,
    active_topics: HashMap<String, ()>,
    should_stop: bool,
    write: Option<WebSocketWrite>,
    last_pong: Option<std::time::Instant>,
}

impl<H: ExchangeHandler> WebSocketActor<H> {
    fn new(config: WebSocketConfig, handler: H, cmd_rx: mpsc::UnboundedReceiver<Command>) -> Self {
        Self {
            config,
            handler,
            cmd_rx,
            subscriptions: HashMap::new(),
            active_topics: HashMap::new(),
            should_stop: false,
            write: None,
            last_pong: None,
        }
    }

    async fn connect(url: &str) -> TransportResult<WebSocket> {
        let client = hpx::Client::builder()
            .build()
            .map_err(|e| TransportError::config(format!("Failed to build client: {}", e)))?;

        let req = client.get(url);
        let ws_req = hpx::ws::WebSocketRequestBuilder::new(req);

        let resp = ws_req.send().await.map_err(TransportError::Http)?;

        resp.into_websocket().await.map_err(TransportError::Http)
    }

    async fn run(mut self) {
        let mut reconnect_delay = self.config.reconnect_initial_delay;

        loop {
            info!("Connecting to {}", self.config.url);

            match Self::connect(&self.config.url).await {
                Ok(ws_stream) => {
                    info!("Connected to {}", self.config.url);
                    reconnect_delay = self.config.reconnect_initial_delay;

                    if let Err(e) = self.handle_connection(ws_stream).await {
                        error!("Connection error: {}", e);
                    } else if self.should_stop {
                        info!("Client closed by user.");
                        break;
                    } else {
                        info!("Connection closed by server, reconnecting...");
                    }
                }
                Err(e) => {
                    error!("Failed to connect: {}", e);
                }
            }

            info!("Reconnecting in {:?}...", reconnect_delay);
            sleep(reconnect_delay).await;
            reconnect_delay = (reconnect_delay.mul_f64(self.config.reconnect_backoff_factor))
                .min(self.config.reconnect_max_delay);
        }
    }

    async fn handle_connection(&mut self, ws_stream: WebSocket) -> TransportResult<()> {
        let (write, mut read) = ws_stream.split();
        self.write = Some(write);
        self.last_pong = Some(std::time::Instant::now());

        let mut ping_timer = tokio::time::interval(self.config.ping_interval);
        ping_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Send on_connect messages (e.g., auth)
        for msg in self.handler.on_connect() {
            self.send_message(msg).await?;
        }

        // Resubscribe to active topics
        let topics: Vec<String> = self.active_topics.keys().cloned().collect();
        for topic in topics {
            let msg = self.handler.build_subscribe(&topic);
            self.send_message(msg).await?;
        }

        loop {
            tokio::select! {
                msg = read.next() => {
                    match msg {
                        Some(Ok(msg)) => {
                            self.process_message(msg).await;
                        }
                        Some(Err(e)) => {
                            return Err(TransportError::websocket(e.to_string()));
                        }
                        None => {
                            info!("WebSocket stream ended");
                            return Ok(());
                        }
                    }
                }

                _ = ping_timer.tick() => {
                    if let Err(e) = self.send_message(Message::Ping(Bytes::new())).await {
                        error!("Failed to send ping: {}", e);
                        return Err(e);
                    }

                    if self.last_pong.is_some_and(|last| last.elapsed() > self.config.pong_timeout) {
                        error!("Ping timeout - no pong received");
                        return Err(TransportError::timeout(self.config.pong_timeout));
                    }
                }

                Some(cmd) = self.cmd_rx.recv() => {
                    match cmd {
                        Command::Subscribe { topic, reply_tx } => {
                            self.handle_subscribe(topic, reply_tx).await;
                        }
                        Command::Unsubscribe { topic } => {
                            self.handle_unsubscribe(topic).await;
                        }
                        Command::Send(msg) => {
                            if let Err(e) = self.send_message(msg).await {
                                warn!("Failed to send message: {}", e);
                            }
                        }
                        Command::Close => {
                            self.should_stop = true;
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    async fn process_message(&mut self, message: Message) {
        match &message {
            Message::Pong(_) => {
                self.last_pong = Some(std::time::Instant::now());
                return;
            }
            Message::Ping(data) => {
                let _ = self.send_message(Message::Pong(data.clone())).await;
                return;
            }
            Message::Close(_) => {
                info!("Received close frame");
                return;
            }
            _ => {}
        }

        if let Some((topic, processed_msg)) = self.handler.process_message(message)
            && let Some(tx) = self.subscriptions.get(&topic)
        {
            let _ = tx.send(processed_msg);
        }
    }

    async fn handle_subscribe(
        &mut self,
        topic: String,
        reply_tx: oneshot::Sender<broadcast::Receiver<Message>>,
    ) {
        let rx = if let Some(tx) = self.subscriptions.get(&topic) {
            tx.subscribe()
        } else {
            let (tx, rx) = broadcast::channel(self.config.channel_capacity);
            self.subscriptions.insert(topic.clone(), tx);
            self.active_topics.insert(topic.clone(), ());

            let msg = self.handler.build_subscribe(&topic);
            if let Err(e) = self.send_message(msg).await {
                warn!("Failed to send subscribe message: {}", e);
            }

            rx
        };

        let _ = reply_tx.send(rx);
    }

    async fn handle_unsubscribe(&mut self, topic: String) {
        self.subscriptions.remove(&topic);
        self.active_topics.remove(&topic);

        let msg = self.handler.build_unsubscribe(&topic);
        if let Err(e) = self.send_message(msg).await {
            warn!("Failed to send unsubscribe message: {}", e);
        }
    }

    async fn send_message(&mut self, message: Message) -> TransportResult<()> {
        if let Some(ref mut write) = self.write {
            write
                .send(message)
                .await
                .map_err(|e| TransportError::websocket(e.to_string()))?;
        }
        Ok(())
    }
}
